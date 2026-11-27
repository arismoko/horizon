use super::mod_api_host::world_attachments;
use crate::game::ServerGame;
use mod_api::AttachmentAccess;
use sunrise_mod_runtime::attachments::{ATTACHMENTS_KEY, attachment_compound};

/// The entity families whose saved maps bind an owning lifetime
/// (`bind_saved_entity_attachments`), so a first store publishes index rows for them.
///
/// Every other family stays unindexed on a first store until its saved binding lands: the value
/// is kept and saved, but no query visits it. Cushions, hooks, TNT, fangs, crystals, falling
/// blocks, rockets, tridents, spawners, area-effect clouds, fence knots, the dragon itself and
/// `Other` (markers, displays) are unindexed today (#995).
fn saved_binding_target(target: super::commands::Target) -> bool {
    use super::commands::Target;
    matches!(
        target,
        Target::Mob(_)
            | Target::Item(_)
            | Target::Orb(_)
            | Target::Projectile(_)
            | Target::Stand(_)
            | Target::Frame(_)
            | Target::Painting(_)
            | Target::Boat(_)
            | Target::Cart(_)
            | Target::DragonPart(_)
            | Target::General(_)
    )
}

impl ServerGame<'_> {
    /// Attachment membership uses physical presence (except removed mobs), not the
    /// command resolver's pending-removal masks. Limit resolution to this level.
    pub fn attachment_entity_exists(&self, id: u32) -> bool {
        use sunrise_sim::join_seq::residency::table::EntityLookup;
        matches!(
            self.resident_entities
                .table
                .borrow_mut()
                .entity_lookup(&self.dim.key(), id, |at| {
                    use sunrise_sim::join_seq::residency::Kind;
                    match at.owner.kind {
                        Kind::Hook | Kind::Player => self
                            .locate_specialized_resident_owner(at)
                            .map(|mut source| {
                                // Hooks were counted by physical presence, regardless of pending
                                // removals. Players were not, but must still validate their row.
                                source.value.1 = at.owner.kind == Kind::Hook;
                                source
                            }),
                        _ => self.locate_resident_attachment_owner(at),
                    }
                }),
            EntityLookup::Found {
                value: (_, true),
                ..
            }
        )
    }

    /// Object ownership for persistence cleanup, never a level-query membership set.
    pub(in crate::game::server_side) fn retained_entity_ids(
        &self,
    ) -> std::collections::HashSet<u32> {
        let mut retained = self.live_entity_ids();
        retained.extend(self.common.persist.off_level.keys().copied());
        retained
    }

    /// The attachments of entities that left the level, dropped: each tick
    /// while any entity has some. Only those entities are looked at, and
    /// nothing else of the entities' memory changes.
    pub fn prune_entity_attachments(&mut self) {
        let alive = self.retained_entity_ids();
        let gone: Vec<u32> = self
            .common
            .persist
            .uuids
            .keys()
            .copied()
            .filter(|id| self.common.entity_attachments.contains_key(id) && !alive.contains(id))
            .collect();
        for id in gone {
            self.common
                .forget_entity_initialization(mod_api::Holder::Entity(id));
            self.common.entity_attachments.remove(&id);
        }
    }

    pub(in crate::game) fn native_authoritative(&self) -> bool {
        self.common.is_logical_server()
    }

    /// Access a holder's attachment map, to lend to a mod: a server's, whose
    /// synced changes are sent at the pass's end, or a client's copy of what
    /// its server synced.
    #[allow(dead_code)]
    pub(in crate::game::server_side) fn attachment_map(
        &mut self,
        holder: mod_api::Holder,
        access: AttachmentAccess,
    ) -> Result<&mut mod_api::Attachments, mod_api::AttachmentError> {
        self.side_attachment_map(holder, access)
    }

    pub(in crate::game) fn attachment_map_read(
        &mut self,
        holder: mod_api::Holder,
    ) -> Result<Option<&mod_api::Attachments>, mod_api::AttachmentError> {
        if !self.native_authoritative() {
            return Err(mod_api::AttachmentError::MissingHolder);
        }
        match self.resolve_attachment_slot(holder)? {
            LocalAttachmentSlot::Entity(id) => Ok(self.common.entity_attachments.get(&id)),
            LocalAttachmentSlot::ActingPlayer => Ok(Some(&self.common.player_state.attachments)),
            LocalAttachmentSlot::Guest(conn) => {
                let guests = &self
                    .server
                    .lan
                    .as_ref()
                    .ok_or(mod_api::AttachmentError::MissingHolder)?
                    .hub
                    .guests;
                let index = guests
                    .connection_index(conn)
                    .ok_or(mod_api::AttachmentError::MissingHolder)?;
                Ok(Some(&guests[index].attachments))
            }
            LocalAttachmentSlot::World => {
                super::mod_api_host::world_attachment_ref(&self.common.world, holder)
            }
        }
    }

    /// `attachment_map` without recording the holder for the sync. Inside a transaction, the
    /// map is kept as it was on its first access, so an abort puts it back.
    pub fn attachment_map_quiet(
        &mut self,
        holder: mod_api::Holder,
        access: AttachmentAccess,
    ) -> Result<&mut mod_api::Attachments, mod_api::AttachmentError> {
        if !self.native_authoritative() {
            return Err(mod_api::AttachmentError::MissingHolder);
        }
        self.journal_attachments(holder);
        self.attachment_slot(holder, access)
    }

    /// Bind first-store contents to their real owner, including writes made before
    /// insertion by EntityConstructing listeners. Never invent a temporary owner.
    pub fn bind_resident_entity_attachments(
        &mut self,
        id: u32,
    ) -> Result<(), mod_api::AttachmentError> {
        use mod_api::AttachmentError;
        let id = std::num::NonZeroU32::new(id).ok_or(AttachmentError::MissingHolder)?;
        if matches!(
            self.with_resident_slot(&self.dim.key(), id, |slot| slot.is_bound()),
            super::entity_residency::SlotAccess::Done(true)
        ) {
            return Ok(());
        }
        if !matches!(
            self.server_resident_entity(id.get()),
            sunrise_sim::join_seq::residency::table::EntityLookup::Found {
                value,
                ..
            } if saved_binding_target(value)
        ) {
            return Ok(());
        }
        if !self.common.entity_attachments.contains_key(&id.get()) {
            return Ok(());
        }
        // Cleanup owns per-level UUID metadata, while attachment maps are global.
        // Record the actual part identity so its level alone retires its map.
        if self.level_dragon_part(id.get()).is_some() {
            let uuid = self.uuid_for(id.get());
            self.common.persist.uuids.insert(id.get(), uuid);
        }
        let level = self
            .level(self.common.dim.id())
            .ok_or(AttachmentError::MissingHolder)?;
        let holder =
            mod_api::AttachmentHolder::Entity(mod_api::EntityRef::new(level.epoch(), id.get()));
        let index = self
            .common
            .attachment_indexes
            .entry(self.common.dim)
            .or_default()
            .clone();
        let mut map = self.common.entity_attachments.remove(&id.get()).unwrap();
        self.with_resident_slot(&self.common.dim.key(), id, |slot| {
            if !slot.is_bound() {
                slot.bind(&mut map, &index, holder);
            }
        });
        self.common.entity_attachments.insert(id.get(), map);
        Ok(())
    }

    /// The exact parked player addressed by a holder, including before placement. Shared
    /// with the journal so rollback restores the map used by the attachment operation.
    pub(in crate::game) fn attachment_player_connection(&self, uuid: u128) -> Option<u32> {
        let conn = self
            .common
            .entity_initialization
            .constructing_player_connection(uuid)
            .or_else(|| self.cloning_guest(uuid).map(|guest| guest.conn))
            .or_else(|| {
                let host = self.server.lan.as_ref()?;
                Some(host.hub.holder_guest(uuid, self.common.dim.id())?.conn)
            })?;
        (!self.server.lan_guest_swaps.contains(&conn)).then_some(conn)
    }
}

enum LocalAttachmentSlot {
    Entity(u32),
    ActingPlayer,
    Guest(u32),
    World,
}

impl crate::game::ServerGame<'_> {
    /// Where `holder`'s attachment map lives now: an entity's by its object ID, a player's where
    /// their state is (swapped in for a guest's turn, or kept beside), the world's others.
    pub fn attachment_slot(
        &mut self,
        holder: mod_api::Holder,
        access: AttachmentAccess,
    ) -> Result<&mut mod_api::Attachments, mod_api::AttachmentError> {
        use mod_api::AttachmentError;
        match self.resolve_attachment_slot(holder)? {
            LocalAttachmentSlot::Entity(id) => {
                if access == AttachmentAccess::Store {
                    if self
                        .common
                        .persist
                        .removals
                        .contains(&super::commands::Target::Painting(id))
                    {
                        return Err(AttachmentError::MissingHolder);
                    }
                    if !self.common.entity_initialization.is_constructing(holder) {
                        self.common.entity_attachments.entry(id).or_default();
                        self.bind_resident_entity_attachments(id)?;
                    }
                }
                Ok(self.common.entity_attachments.entry(id).or_default())
            }
            LocalAttachmentSlot::ActingPlayer => {
                if access == AttachmentAccess::Store
                    && let Some(conn) = self.server.lan_acting
                {
                    self.bind_player_attachments(conn, true);
                }
                Ok(&mut self.common.player_state.attachments)
            }
            LocalAttachmentSlot::Guest(conn) => {
                if access == AttachmentAccess::Store {
                    self.bind_player_attachments(conn, true);
                }
                self.server
                    .lan
                    .as_mut()
                    .and_then(|host| host.hub.guests.attachments_mut(conn))
                    .map(|attachments| &mut **attachments)
                    .ok_or(AttachmentError::MissingHolder)
            }
            LocalAttachmentSlot::World => world_attachments(&mut self.common.world, holder),
        }
    }

    fn resolve_attachment_slot(
        &mut self,
        holder: mod_api::Holder,
    ) -> Result<LocalAttachmentSlot, mod_api::AttachmentError> {
        use mod_api::AttachmentError;
        if let mod_api::Holder::Entity(id) = holder {
            if !self.common.entity_initialization.is_constructing(holder) {
                use super::entity_residency::SlotAccess;
                use sunrise_sim::join_seq::residency::Kind;
                let id = std::num::NonZeroU32::new(id).ok_or(AttachmentError::MissingHolder)?;
                match self.with_resident_slot(&self.common.dim.key(), id, |_| ()) {
                    SlotAccess::Done(()) => {}
                    SlotAccess::Taken => return Err(AttachmentError::HolderBusy),
                    SlotAccess::Missing => return Err(AttachmentError::MissingHolder),
                    SlotAccess::UnsupportedKind(kind) => {
                        let name = match kind {
                            Kind::Mob => "mob",
                            Kind::Item => "item",
                            Kind::Orb => "orb",
                            Kind::Projectile => "projectile",
                            Kind::Stand => "armorstand",
                            Kind::Frame => "itemframe",
                            Kind::Painting => "painting",
                            Kind::Boat => "boat",
                            Kind::Cart => "minecart",
                            Kind::Other => "other",
                            Kind::Fangs => "fangs",
                            Kind::Crystal => "crystal",
                            Kind::Dragon => "dragon",
                            Kind::Tnt => "tnt",
                            Kind::Falling => "falling",
                            Kind::Rocket => "rocket",
                            Kind::Trident => "trident",
                            Kind::Spawner => "spawner",
                            Kind::Cloud => "cloud",
                            Kind::Bolt => "bolt",
                            Kind::Cushion => "cushion",
                            Kind::Knot => "knot",
                            Kind::Player => "player",
                            Kind::Hook => "hook",
                            Kind::General => "general",
                            Kind::DragonPart => "dragonpart",
                        };
                        return Err(AttachmentError::UnsupportedHolder(name));
                    }
                }
            }
            if !self.common.entity_initialization.is_constructing(holder)
                && !self.attachment_entity_exists(id)
            {
                return Err(AttachmentError::MissingHolder);
            }
            self.initialize_entity_on_access(holder);
            attachment_compound(
                self.common
                    .persist
                    .raw
                    .get(&id)
                    .and_then(|tag| tag.get(ATTACHMENTS_KEY)),
            )?;
            return Ok(LocalAttachmentSlot::Entity(id));
        }
        if let mod_api::Holder::Player(uuid) = holder {
            if self.server.lan_acting_uuid == Some(uuid) {
                return Ok(LocalAttachmentSlot::ActingPlayer);
            }
            let conn = self
                .attachment_player_connection(uuid)
                .ok_or(AttachmentError::MissingHolder)?;
            if self.server.lan_guest_swaps.contains(&conn) {
                return Err(AttachmentError::MissingHolder);
            }
            return Ok(LocalAttachmentSlot::Guest(conn));
        }
        Ok(LocalAttachmentSlot::World)
    }
}

use sunrise_gameplay_common::game::side_ops::modapi::ModapiSideGameExt as _;

use sunrise_gameplay_common::game::entity_initialization_common::EntityInitializationCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
