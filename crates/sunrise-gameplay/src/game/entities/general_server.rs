use super::{
    entity_initialization::TransferredEntityState, entity_server::ServerEntity, persist::ChunkKey,
};
use std::collections::HashSet;
use sunrise_core::{geometry::Vec3, nbt::Tag};
use sunrise_gameplay_common::game::{
    commands_common::Target,
    general::{
        GeneralEntityRecord, GeneralRecordCheck, GeneralRow, GeneralTypeCatalog,
        GeneralTypeEnvelope, check_general_record, check_record_fields, format,
        general_type_catalog, limits, nbt_size, quarantined_kind, read_general_record,
    },
};
use sunrise_sim::join_seq::JoinSeq;

impl crate::game::ServerGame<'_> {
    /// Change a general row's seat. The caller owns mount notifications.
    pub(in crate::game::server_side) fn set_general_mount(
        &mut self,
        id: u32,
        mount: Option<(u32, i64)>,
    ) -> bool {
        if self.common.persist.removals.contains(&Target::General(id)) {
            return false;
        }
        let store = &mut self.common.resident_entities.general;
        let Some(row) = store.rows.iter_mut().find(|row| row.id == id) else {
            return false;
        };
        let (vehicle, order) = mount.map_or((None, 0), |(v, place)| (Some(v), place));
        if vehicle.is_none() && row.mount.is_some() {
            row.boarding_cooldown = super::boat_passengers::BOARDING_COOLDOWN;
        }
        row.mount = vehicle;
        row.vehicle_order = order;
        store.mount_writes += 1;
        true
    }

    /// A general row's own record. Passengers are written by the shared tree walker.
    pub fn general_row_record(&self, id: u32, transferring: bool) -> Option<GeneralEntityRecord> {
        let row = self.common.general_row(id)?;
        if !transferring
            && general_type_catalog().save_policy(row.type_slot)
                == mod_api::GeneralSavePolicy::NoSave
        {
            return None;
        }
        Some(GeneralEntityRecord {
            position: [row.position.x, row.position.y, row.position.z],
            velocity: [row.velocity.x, row.velocity.y, row.velocity.z],
            rotation: row.rotation,
            uuid: row.uuid,
            owner: row.owner,
            custom_name: row.custom_name.clone(),
            custom_name_visible: row.custom_name_visible,
            base: row.base,
            on_ground: row.on_ground,
            fall_distance: row.fall_distance,
            envelope: GeneralTypeEnvelope {
                type_key: general_type_catalog().key(row.type_slot).to_owned(),
                payload_version: row.payload_version,
                payload_bytes: row.payload_bytes.clone(),
            },
            passengers: Vec::new(),
        })
    }

    /// A general row and its passengers. A dimension transfer includes live
    /// objects even when their `EntityType` does not serialize to disk.
    pub fn general_tree_nbt(&mut self, id: u32, transferring: bool) -> Option<Tag> {
        let mut tag = self.general_own_nbt(id, transferring)?;
        self.put_passengers_for(&mut tag, id, transferring);
        Some(tag)
    }

    /// A general row's own compound with its attachments, before its passengers.
    pub(in crate::game::server_side) fn general_own_nbt(
        &mut self,
        id: u32,
        transferring: bool,
    ) -> Option<Tag> {
        let mut tag = self.general_row_record(id, transferring)?.to_tag();
        self.write_entity_attachments(&mut tag, id);
        Some(tag)
    }

    /// The unknown-kind gate of the load pass (`spawn_loaded_entity`): may
    /// this record of a kind no listed family claims go to `load_other`,
    /// the existing preserve-byte-for-byte path? An unknown `minecraft:`
    /// kind and a bare vanilla name keep that existing path unchanged. A
    /// namespaced kind of a loaded mod that never declared the type is
    /// skipped with a warning (its own mod would own the record, and a
    /// table that is not its own must not keep it). A kind of a namespace
    /// no loaded mod owns — a missing mod's record — is preserved, within
    /// the quarantine bounds: a record over
    /// [`limits::QUARANTINE_RECORD_LIMIT`], or a chunk already holding
    /// [`limits::QUARANTINE_CHUNK_LIMIT`] of them, is skipped with a
    /// warning instead. What passes is preserved exactly as loaded: the
    /// unknown-kind path holds the compound untouched (`persist.raw`),
    /// never ticks it (an `OtherEntity` has no tick) and writes it back
    /// from that store.
    pub fn general_quarantine_admits(&self, tag: &Tag, kind: &str, at: Vec3) -> bool {
        let Some((namespace, _)) = kind.split_once(':') else {
            return true;
        };
        if namespace == "minecraft" {
            return true;
        }
        // A declared root can have been handed back because a rider's mod
        // is absent. Apply namespace ownership to the unresolved tree types,
        // not to the already-resolved root.
        fn missing_types_owned(record: &GeneralEntityRecord) -> bool {
            let key = &record.envelope.type_key;
            (general_type_catalog().slot(key).is_some()
                || key
                    .split_once(':')
                    .is_some_and(|(ns, _)| ns != "minecraft" && !mod_namespace_loaded(ns)))
                && record.passengers.iter().all(missing_types_owned)
        }
        let missing_tree = general_type_catalog().slot(kind).is_some()
            && read_general_record(tag, &|rider| self.loads_as_mob_passenger(rider))
                .is_ok_and(|record| missing_types_owned(&record));
        if mod_namespace_loaded(namespace) && !missing_tree {
            log::warn!(
                "Sunrise: {kind} names no type of loaded mod {namespace}; skipping its record"
            );
            return false;
        }
        let size = nbt_size(tag);
        if size > limits::QUARANTINE_RECORD_LIMIT {
            log::warn!(
                "Sunrise: quarantined general entity record of {size} bytes exceeds {}; skipping it",
                limits::QUARANTINE_RECORD_LIMIT
            );
            return false;
        }
        let chunk = chunk_of(at);
        let quarantined_here = self
            .server
            .others
            .0
            .iter()
            .filter(|other| quarantined_kind(&other.kind) && chunk_of(other.position) == chunk)
            .count();
        if quarantined_here >= limits::QUARANTINE_CHUNK_LIMIT {
            log::warn!(
                "Sunrise: chunk {:?} already holds {} quarantined general entity records; skipping the next",
                chunk,
                limits::QUARANTINE_CHUNK_LIMIT
            );
            return false;
        }
        true
    }

    /// A retained object in the destination must not cause mixed transfer to
    /// drop a general row after the source tree has already been removed.
    pub(in crate::game::server_side) fn general_transfer_conflicts(
        &self,
        root: u32,
        destination: super::Dim,
    ) -> bool {
        !self.common.resident_entities.general.rows.is_empty()
            && self
                .vehicle_tree_ids(root)
                .into_iter()
                .filter_map(|id| self.common.general_row(id))
                .any(|row| {
                    self.common
                        .resident_entities
                        .table
                        .borrow_mut()
                        .level_uuid_candidate(destination.key(), row.uuid, None)
                        .is_some()
                })
    }

    /// Carry a general entity tree into another level
    /// (`Entity.teleportCrossDimension` for `Target::General`): the whole
    /// tree is validated before anything of it moves — the destination may
    /// not already hold a general row of one of its UUIDs — then each row
    /// leaves the source list and joins the destination's, its identity
    /// (id, UUID, attachments) unchanged but its join identity new, so no
    /// handle of the source level reaches it again. A failure after the
    /// rows left — the destination level cannot open — puts every row back
    /// in the source, at its place and with the join identity it had; a
    /// failure before leaves the tree untouched. The root lands at `at`, and
    /// each rider takes its own vehicle's transition on arrival.
    pub fn teleport_general_tree_across(
        &mut self,
        root_id: u32,
        destination: sunrise_gameplay_common::game::Dim,
        at: [f64; 3],
        rotation: Option<[f32; 2]>,
        turn_riders: bool,
        motion: super::vehicle_teleport::TeleportMotion,
    ) -> Result<bool, String> {
        let source = self.common.dim;
        if source == destination {
            return Ok(false);
        }
        // The whole tree, root first, each row exactly once: a mount link
        // this walk cannot reach from the root is not the tree's.
        let rows = &self.common.resident_entities.general.rows;
        let Some(root) = rows
            .iter()
            .position(|row| row.id == root_id && row.mount.is_none())
        else {
            return Ok(false);
        };
        let mut tree = vec![root];
        let mut cursor = 0;
        while cursor < tree.len() {
            let vehicle = rows[tree[cursor]].id;
            for (index, row) in rows.iter().enumerate() {
                if row.mount == Some(vehicle) && !tree.contains(&index) {
                    tree.push(index);
                }
            }
            cursor += 1;
        }
        // Validate the destination before anything moves: no general row of
        // the tree's UUIDs may already live there.
        let uuids: HashSet<[i32; 4]> = tree.iter().map(|&index| rows[index].uuid).collect();
        if self
            .server
            .levels
            .0
            .parked_general_holds_any_uuid(destination, &uuids)
        {
            return Err(format!(
                "destination {} already holds a general entity of the tree's uuids",
                destination.id()
            ));
        }
        // Extract: highest index first, each row with the place it left.
        tree.sort_unstable();
        let mut extracted = Vec::new();
        for &index in tree.iter().rev() {
            let row = self.common.resident_entities.general.rows.remove(index);
            let transient = self.common.persist.transient.remove(&row.id);
            extracted.push((index, row, transient));
        }
        extracted.reverse();
        // The removals are leaves of the source level
        // (`PersistentEntitySectionManager.stopTracking`): flush them there.
        self.native_leaves_flush();
        // The only failing step: if the destination cannot open, every row
        // returns to the source, at its place and with its join identity.
        if let Err(error) = self.enter_level(destination) {
            for (index, row, transient) in extracted {
                if transient {
                    self.common.persist.transient.insert(row.id);
                }
                self.common
                    .resident_entities
                    .general
                    .rows
                    .insert(index, row);
            }
            return Err(format!("{error}"));
        }
        // The tree joins the destination: identities unchanged, join
        // identity new (a fresh admission — no source handle reaches it).
        let (mut arrivals, transients): (Vec<_>, Vec<_>) = extracted
            .into_iter()
            .map(|(_, row, transient)| (row, transient))
            .unzip();
        super::vehicle_teleport::transfer_general_tree(
            &mut arrivals,
            root_id,
            super::vehicle_teleport::Transition::to_level(
                Vec3::new(at[0], at[1], at[2]),
                motion,
                rotation,
            ),
            turn_riders,
        );
        for (mut row, transient) in arrivals.into_iter().zip(transients) {
            if transient {
                self.common.persist.transient.insert(row.id);
            }
            row.join = JoinSeq::UNJOINED;
            self.common.resident_entities.general.rows.push(row);
        }
        if let Err(error) = self.enter_level(source) {
            log::error!("could not reopen {}: {error}", source.id());
        }
        Ok(true)
    }

    /// `EntityType.loadEntityRecursive`: an entity that cannot be created never
    /// brings its passengers into the candidate tree. Prune before UUID checks
    /// or decoding general payloads, including under vanilla vehicle records.
    pub(in crate::game) fn retain_feature_eligible_entity_tree(&self, tag: &mut Tag) -> bool {
        let catalog = general_type_catalog();
        if catalog.is_empty() {
            return true;
        }
        let features = mod_api::FeatureFlags::new(self.level.enabled_features.as_arc().clone());
        fn retain(
            game: &crate::game::ServerGame<'_>,
            tag: &mut Tag,
            catalog: &GeneralTypeCatalog,
            features: &mod_api::FeatureFlags,
        ) -> bool {
            let Some(kind) = sunrise_sim::entity_nbt::kind(tag) else {
                return true;
            };
            let slot = catalog.slot(&kind);
            // An unavailable mod's raw record is opaque and stays intact for quarantine.
            if slot.is_none()
                && quarantined_kind(&kind)
                && !game.res.mob_types().contains_key(&kind)
            {
                return true;
            }
            if let Some(slot) = slot
                && !catalog.is_enabled(slot, features)
            {
                let missing: Vec<_> = catalog
                    .required_features(slot)
                    .iter()
                    .filter(|&&flag| !features.contains_flag(flag))
                    .map(|flag| flag.id())
                    .collect();
                log::warn!(
                    "Skipping entity with id {kind}: missing feature flags {}",
                    missing.join(", ")
                );
                return false;
            }
            if let Tag::Compound(compound) = tag
                && let Some(Tag::List(riders)) = compound.get_mut(&format::PASSENGERS.into())
            {
                riders.retain_mut(|rider| retain(game, rider, catalog, features));
            }
            true
        }
        retain(self, tag, catalog, &features)
    }

    /// Load one saved general entity tree whose root kind the frozen
    /// registry holds: tag → record → every check, then the whole tree
    /// publishes through the same join boundary a fresh spawn takes. A
    /// structurally sound tree of types this load cannot resolve hands the
    /// tag back (`Err`), so the missing mod's record keeps byte for byte by
    /// the existing unknown-kind path; every other refusal — a malformed
    /// envelope, a structural fault, a UUID a live level entity already
    /// owns — drops the whole tree with a warning and publishes nothing.
    pub(in crate::game::server_side) fn load_general_tree(
        &mut self,
        tag: Tag,
        transferred: Option<TransferredEntityState>,
    ) -> Result<(), Tag> {
        match self.load_general_passenger(&tag, None, transferred) {
            Ok(_) => Ok(()),
            Err(()) => Err(tag),
        }
    }

    pub(in crate::game::server_side) fn load_general_passenger(
        &mut self,
        tag: &Tag,
        mount: Option<u32>,
        transferred: Option<TransferredEntityState>,
    ) -> Result<Option<u32>, ()> {
        let record = match read_general_record(tag, &|rider| self.loads_as_mob_passenger(rider)) {
            Ok(record) => record,
            Err(error) => {
                log::warn!(
                    "Sunrise: refusing general entity record: {}",
                    error.message()
                );
                return Ok(None);
            }
        };
        self.admit_general_record_with_mount(record, tag, mount, transferred)
    }

    /// Admit one general record tree: every check, then its rows inserted
    /// unjoined, the join left to the caller's flush. The root row's runtime
    /// id; `Ok(None)` for a tree refused with a warning (a structural fault,
    /// a UUID a live level entity already owns), `Err` for a sound tree of
    /// types the frozen registry doesn't hold.
    pub(in crate::game::server_side) fn admit_general_record(
        &mut self,
        record: GeneralEntityRecord,
        tag: &Tag,
        transferred: Option<TransferredEntityState>,
    ) -> Result<Option<u32>, ()> {
        self.admit_general_record_with_mount(record, tag, None, transferred)
    }

    fn admit_general_record_with_mount(
        &mut self,
        record: GeneralEntityRecord,
        tag: &Tag,
        mount: Option<u32>,
        transferred: Option<TransferredEntityState>,
    ) -> Result<Option<u32>, ()> {
        let catalog = general_type_catalog();
        match check_general_record(&record, |key| catalog.slot(key).is_some()) {
            Ok(GeneralRecordCheck::Ready) => {}
            Ok(GeneralRecordCheck::UnknownType) => return Err(()),
            Err(error) => {
                log::warn!(
                    "Sunrise: refusing general entity record: {}",
                    error.message()
                );
                return Ok(None);
            }
        }
        // A UUID a live entity of the level already owns refuses the whole
        // tree (`uuid_admission_refused`, the boundary every loaded kind
        // shares), before any row of it exists.
        if general_tree_uuid_admission_refused(self, &record) {
            log::warn!(
                "Sunrise: refusing general entity record: a live entity owns its uuid {:?}",
                record.uuid
            );
            return Ok(None);
        }
        Ok(Some(self.insert_general_record(
            record,
            tag,
            mount,
            transferred,
        )))
    }

    /// Read one spawner passenger without its riders. The spawner loads
    /// those separately and resolves UUID residency in preorder, not with
    /// the saved general tree's structural checks.
    pub(in crate::game::server_side) fn decode_general_spawn_passenger(
        tag: &Tag,
        kind: &str,
    ) -> Option<(usize, GeneralEntityRecord)> {
        let slot = general_type_catalog().slot(kind)?;
        let record = read_general_record(tag, &|_| false)
            .and_then(|record| {
                check_record_fields(&record)?;
                Ok(record)
            })
            .map_err(|error| {
                log::warn!(
                    "Sunrise: refusing general entity record: {}",
                    error.message()
                );
            })
            .ok()?;
        Some((slot, record))
    }

    /// Insert one saved general record and its riders in seat order.
    fn insert_general_record(
        &mut self,
        mut record: GeneralEntityRecord,
        tag: &Tag,
        mount: Option<u32>,
        transferred: Option<TransferredEntityState>,
    ) -> u32 {
        let slot = general_type_catalog()
            .slot(&record.envelope.type_key)
            .expect("check_general_record resolved every type of the tree");
        let riders = std::mem::take(&mut record.passengers);
        let id = self.insert_general_row(slot, record, tag, mount, transferred);
        if let Some(Tag::List(list)) = tag.get(format::PASSENGERS) {
            let mut general = riders.into_iter();
            for rider in list {
                if self.loads_as_mob_passenger(rider) {
                    self.load_mob(rider.clone(), Some(id));
                } else if let Some(record) = general.next() {
                    self.insert_general_record(record, rider, Some(id), None);
                }
            }
        }
        id
    }

    /// Publish one validated record; the caller owns its passenger traversal.
    pub(in crate::game::server_side) fn insert_general_row(
        &mut self,
        slot: usize,
        record: GeneralEntityRecord,
        tag: &Tag,
        mount: Option<u32>,
        transferred: Option<TransferredEntityState>,
    ) -> u32 {
        self.server.next_entity += 1;
        let id = self.server.next_entity;
        // Install before `remember_uuid` (the same ordering a transferred
        // mob's state takes): the destination is the same logical entity.
        if let Some(state) = transferred {
            self.adopt_transferred_entity_state(id, state);
        }
        self.remember_uuid(id, Some(record.uuid), tag);
        let position = Vec3::new(record.position[0], record.position[1], record.position[2]);
        let mut row = GeneralRow {
            id,
            uuid: record.uuid,
            join: JoinSeq::UNJOINED,
            type_slot: slot,
            previous: position,
            position,
            velocity: Vec3::new(record.velocity[0], record.velocity[1], record.velocity[2]),
            rotation: record.rotation,
            previous_rotation: record.rotation,
            owner: record.owner,
            fall_distance: record.fall_distance,
            custom_name: record.custom_name,
            custom_name_visible: record.custom_name_visible,
            base: record.base,
            on_ground: record.on_ground,
            tick_count: 0,
            no_physics: false,
            horizontal_collision: false,
            vertical_collision: false,
            vertical_collision_below: false,
            fluids: Default::default(),
            under_water: false,
            payload_version: record.envelope.payload_version,
            payload_bytes: record.envelope.payload_bytes,
            mount,
            vehicle_order: i64::from(id),
            boarding_cooldown: 0,
            attachment_lifetime: Default::default(),
        };
        self.bind_saved_entity_attachments(id, &mut row.attachment_lifetime);
        self.common.resident_entities.general.rows.push(row);
        id
    }
}

/// Whether a namespace belongs to a loaded package: its own id or one of
/// its manifest's namespaces (`owns_namespace`, read from the engine side).
fn mod_namespace_loaded(namespace: &str) -> bool {
    mod_api::__private::loaded_mods().iter().any(|package| {
        package.id() == namespace || package.namespaces().any(|owned| owned == namespace)
    })
}

/// Whether any UUID of the record tree names a live entity of the level
/// (`uuid_admission_refused` over the whole tree, in preorder).
fn general_tree_uuid_admission_refused(
    game: &crate::game::ServerGame<'_>,
    record: &GeneralEntityRecord,
) -> bool {
    fn refused(game: &crate::game::ServerGame<'_>, record: &GeneralEntityRecord) -> bool {
        game.uuid_admission_refused(Some(record.uuid))
            || record.passengers.iter().any(|rider| refused(game, rider))
    }
    refused(game, record)
}

impl ServerEntity for GeneralRow {
    const SERIALIZES_ATTACHMENTS: bool = true;

    fn save_nbt(
        game: &mut crate::game::ServerGame<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        let row = &game.resident_entities.general.rows[index];
        let catalog = general_type_catalog();
        // `noSave`: instances produce zero persisted records.
        if catalog.save_policy(row.type_slot) == mod_api::GeneralSavePolicy::NoSave {
            return None;
        }
        // A rider saves inside its root's record, never as its own.
        if row.mount.is_some() {
            return None;
        }
        let key = chunk_of(row.position);
        if !keys.contains(&key) {
            return None;
        }
        Some((key, game.general_tree_nbt(row.id, false)?))
    }
    fn load_nbt(
        game: &mut crate::game::ServerGame<'_>,
        kind: &str,
        _uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<TransferredEntityState>,
    ) -> Result<(), Tag> {
        // Only a type the frozen registry holds loads here; anything else
        // hands the compound back for the unknown-kind paths.
        if general_type_catalog().slot(kind).is_none() {
            return Err(tag);
        }
        game.load_general_tree(tag, transferred.take())
    }
}

/// The chunk a position lies in, the save walk's own arithmetic.
fn chunk_of(at: Vec3) -> ChunkKey {
    ((at.x.floor() as i32) >> 4, (at.z.floor() as i32) >> 4)
}

#[path = "general/tick.rs"]
mod tick;
