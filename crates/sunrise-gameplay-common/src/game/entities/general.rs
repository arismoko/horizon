//! General entity rows (#792 stage 2): the record codec for mod-declared
//! entity types. The record is the save format's unit: common fields every
//! kind shares, plus the type envelope (namespaced key, payload schema
//! version, payload bytes) and the passenger tree. Validation runs every
//! check before any of it may reach a row; a refusal names the first fault
//! and never panics, clamps or truncates into a valid-looking entity.

/// The design's bounds, re-exported from the one `limits` block the
/// registrar quotes (`mod_api::general_entity_limits`).
pub use mod_api::general_entity_limits as limits;

pub use crate::game::commands_common::Target;
pub use crate::game::entity::Entity;
use crate::game::native_entity_base::{EntityBase, owned};
use std::borrow::Cow;
use sunrise_core::fast_hash::{FxHashMap, FxHashSet};
pub use sunrise_core::geometry::{Aabb, Vec3};
pub use sunrise_core::nbt::Tag;
use sunrise_game_data::text::Text;
use sunrise_sim::entity_rules::EntityRules;
use sunrise_sim::impl_joined;
pub use sunrise_sim::join_seq::{JoinList, JoinSeq};
use sunrise_world::move_gate::MoveGate;

/// One general entity as a record: what save writes and load reads. The
/// engine's live row (`GeneralRow`, stage 3) is built from a validated
/// record; this type never holds runtime ids, slots or pointers.
#[derive(Clone, Debug, PartialEq)]
pub struct GeneralEntityRecord {
    /// The entity's position in blocks.
    pub position: [f64; 3],
    /// The entity's velocity in blocks per tick.
    pub velocity: [f64; 3],
    /// The entity's yaw and pitch in degrees.
    pub rotation: [f32; 2],
    /// The entity's UUID, the identity a reload continues.
    pub uuid: [i32; 4],
    /// The UUID of the entity that fired it (`Owner`, as `TraceableEntity`s and
    /// projectiles save it), when a spawn named one.
    pub owner: Option<[i32; 4]>,
    /// `Entity.getCustomName`, saved as `CustomName`; absent means none.
    pub custom_name: Option<Text>,
    /// `Entity.isCustomNameVisible`, saved as `CustomNameVisible`; absent means false.
    pub custom_name_visible: bool,
    /// The `Entity` base state: `Fire`, `Invulnerable`, `NoGravity`, `Silent`, `Glowing`.
    pub base: EntityBase,
    /// `Entity.onGround`, saved as `OnGround`; absent means false.
    pub on_ground: bool,
    /// `Entity.fallDistance`, saved as `fall_distance`; absent or not a number means zero.
    pub fall_distance: f64,
    /// The type envelope: which registered type owns the payload, at which
    /// schema version, with which bounded payload bytes.
    pub envelope: GeneralTypeEnvelope,
    /// The entities riding this one, stored inside its record.
    pub passengers: Vec<GeneralEntityRecord>,
}

/// The mod-owned half of a record: the registered type's namespaced key, the
/// payload's schema version and the payload bytes themselves.
#[derive(Clone, Debug, PartialEq)]
pub struct GeneralTypeEnvelope {
    /// The type's namespaced id (`mymod:robot`), as declared.
    pub type_key: String,
    /// The payload's schema version, as declared by the type.
    pub payload_version: u16,
    /// The payload bytes, at most [`limits::PAYLOAD_LIMIT`] long.
    pub payload_bytes: Vec<u8>,
}

/// Why a general entity record was refused before any of it could reach a
/// row. Each fault names the record's uuid in its diagnostic string.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum GeneralRecordError {
    /// A mod payload exceeds [`limits::PAYLOAD_LIMIT`]. The bound applies to
    /// mod payloads only, never to a record of a vanilla type.
    PayloadTooLarge { size: usize },
    /// The decoded data of a payload is nested deeper than
    /// [`limits::MAX_NESTING`]. The record's own NBT walk fires this when a
    /// record carries data nested past the bound; the passenger chain is not
    /// counted, only the saved NBT's own nesting limit bounds it.
    NestingTooDeep { depth: u32 },
    /// A passenger's UUID is one of its own ancestors: a cycle.
    PassengerCycle { uuid: [i32; 4] },
    /// The same UUID appears twice in one record tree.
    DuplicateUuid { uuid: [i32; 4] },
    /// A position, velocity or rotation component is not finite.
    NotFinite,
    /// The type envelope is not what the format writes: a field of the wrong
    /// tag kind or shape (`Payload` not a byte array, `Pos` not three
    /// doubles, a passenger list of things that are not compounds). The
    /// record never reaches a row and nothing of it is kept.
    InvalidEnvelope { field: &'static str },
}

impl GeneralRecordError {
    /// The refusal's diagnostic, for the load summary.
    pub fn message(&self) -> String {
        match self {
            GeneralRecordError::PayloadTooLarge { size } => format!(
                "general entity payload {size} exceeds {}",
                limits::PAYLOAD_LIMIT
            ),
            GeneralRecordError::NestingTooDeep { depth } => format!(
                "general entity nesting {depth} exceeds {}",
                limits::MAX_NESTING
            ),
            GeneralRecordError::PassengerCycle { uuid } => {
                format!("general entity passenger cycle at {uuid:?}")
            }
            GeneralRecordError::DuplicateUuid { uuid } => {
                format!("general entity duplicate uuid {uuid:?}")
            }
            GeneralRecordError::NotFinite => {
                "general entity has a non-finite coordinate".to_owned()
            }
            GeneralRecordError::InvalidEnvelope { field } => {
                format!("general entity envelope field {field:?} is malformed")
            }
        }
    }
}

/// The outcome of checking a record against every bound: ready to become a
/// row, or structurally sound but of a type this load cannot resolve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeneralRecordCheck {
    /// Every check passed; the record may reach a row.
    Ready,
    /// Every structural check passed, but the type key's namespace belongs
    /// to neither vanilla nor a loaded mod. Stage 3 keeps such a record
    /// byte for byte as quarantined data; it never reaches a row here.
    UnknownType,
}

/// Check one record tree against every bound, before any of it may reach a
/// row. `is_known_type` answers whether a type key's namespace belongs to
/// vanilla or a loaded mod; the caller supplies the frozen registry's view.
///
/// Structural faults (payload size, cycles,
/// duplicate UUIDs, non-finite coordinates) refuse the tree whatever its
/// type; only a structurally sound record of an unresolvable type reports
/// [`GeneralRecordCheck::UnknownType`], so quarantine never preserves an
/// over-budget or malformed payload. The design's separate data-nesting
/// bound is the payload decode's to check (stage 3), not this walk's. The
/// passenger chain has no bound of its own, as in vanilla: only the saved NBT's
/// nesting limit applies, and the walk is iterative so no chain exhausts the
/// stack. Nothing is written, allocated beyond the walk's own bookkeeping, or
/// panicked on; a refused tree leaves no partial state, and the next valid
/// record checks cleanly.
pub fn check_general_record(
    record: &GeneralEntityRecord,
    is_known_type: impl Fn(&str) -> bool,
) -> Result<GeneralRecordCheck, GeneralRecordError> {
    let mut seen = FxHashSet::default();
    let mut ancestors = FxHashSet::default();
    // Each open record, the next passenger to visit and whether one was unresolvable.
    let mut open: Vec<(&GeneralEntityRecord, usize, bool)> = Vec::new();
    check_record(record, &ancestors, &mut seen)?;
    ancestors.insert(record.uuid);
    open.push((record, 0, false));
    loop {
        let Some(top) = open.last_mut() else {
            unreachable!("the walk returns when its root closes");
        };
        if let Some(passenger) = top.0.passengers.get(top.1) {
            top.1 += 1;
            check_record(passenger, &ancestors, &mut seen)?;
            ancestors.insert(passenger.uuid);
            open.push((passenger, 0, false));
            continue;
        }
        let (closed, _, unknown) = open.pop().expect("a record was open");
        ancestors.remove(&closed.uuid);
        let unknown = unknown || !is_known_type(&closed.envelope.type_key);
        match open.last_mut() {
            Some(parent) => parent.2 |= unknown,
            None if unknown => return Ok(GeneralRecordCheck::UnknownType),
            None => return Ok(GeneralRecordCheck::Ready),
        }
    }
}

/// One record's own faults, before its passengers are visited.
fn check_record(
    record: &GeneralEntityRecord,
    ancestors: &FxHashSet<[i32; 4]>,
    seen: &mut FxHashSet<[i32; 4]>,
) -> Result<(), GeneralRecordError> {
    check_record_fields(record)?;
    if ancestors.contains(&record.uuid) {
        return Err(GeneralRecordError::PassengerCycle { uuid: record.uuid });
    }
    if !seen.insert(record.uuid) {
        return Err(GeneralRecordError::DuplicateUuid { uuid: record.uuid });
    }
    Ok(())
}

pub fn check_record_fields(record: &GeneralEntityRecord) -> Result<(), GeneralRecordError> {
    if !record
        .position
        .iter()
        .chain(record.velocity.iter())
        .all(|component| component.is_finite())
        || !record
            .rotation
            .iter()
            .all(|component| component.is_finite())
    {
        return Err(GeneralRecordError::NotFinite);
    }
    if record.envelope.payload_bytes.len() > limits::PAYLOAD_LIMIT {
        return Err(GeneralRecordError::PayloadTooLarge {
            size: record.envelope.payload_bytes.len(),
        });
    }
    Ok(())
}

/// The save format's common field names, vanilla's own (`Entity.saveWithoutId`)
/// for the fields every kind shares, and the envelope's names for the rest.
pub mod format {
    /// The type's namespaced id (`id`, as every entity compound carries it).
    pub const TYPE: &str = "id";
    /// `Pos`: three doubles.
    pub const POSITION: &str = "Pos";
    /// `Motion`: three doubles.
    pub const VELOCITY: &str = "Motion";
    /// `Rotation`: two floats (yaw, pitch).
    pub const ROTATION: &str = "Rotation";
    /// `UUID`: four ints.
    pub const UUID: &str = "UUID";
    /// `Owner`: the firing entity's UUID, four ints (`Projectile.addAdditionalSaveData`'s key).
    pub const OWNER: &str = "Owner";
    /// `CustomName`: the entity's name as a text component (`Entity.saveWithoutId`).
    pub const CUSTOM_NAME: &str = "CustomName";
    /// `CustomNameVisible`: written only when set (`Entity.saveWithoutId`).
    pub const CUSTOM_NAME_VISIBLE: &str = "CustomNameVisible";
    /// `OnGround`: always written (`Entity.saveWithoutId`).
    pub const ON_GROUND: &str = "OnGround";
    /// `fall_distance`: a double, always written (`Entity.saveWithoutId`).
    pub const FALL_DISTANCE: &str = "fall_distance";
    /// The payload's schema version, a short.
    pub const PAYLOAD_VERSION: &str = "PayloadVersion";
    /// The payload bytes, a byte array.
    pub const PAYLOAD: &str = "Payload";
    /// `Passengers`: the riding records, stored inside their vehicle's.
    pub const PASSENGERS: &str = "Passengers";
}

fn doubles(values: [f64; 3]) -> Tag {
    Tag::List(values.map(|value| Tag::Double(value.to_bits())).to_vec())
}

/// Read one three-double field the format writes (`Pos`, `Motion`), with its
/// vanilla default when the compound omits it.
fn doubles3(tag: &Tag, field: &'static str) -> Result<[f64; 3], GeneralRecordError> {
    let Some(Tag::List(values)) = tag.get(field) else {
        return Ok([0.0; 3]);
    };
    if values.len() != 3 {
        return Err(GeneralRecordError::InvalidEnvelope { field });
    }
    let mut out = [0.0; 3];
    for (slot, value) in values.iter().enumerate() {
        let Tag::Double(bits) = value else {
            return Err(GeneralRecordError::InvalidEnvelope { field });
        };
        out[slot] = f64::from_bits(*bits);
    }
    Ok(out)
}

impl GeneralEntityRecord {
    /// The record as one save compound: the common fields under their vanilla
    /// names, the type envelope (`id`, `PayloadVersion`, `Payload`), and the
    /// passenger tree inside `Passengers`, exactly what save writes and load
    /// reads. Runtime slots, network ids and pointers never appear.
    pub fn to_tag(&self) -> Tag {
        let mut map = sunrise_core::nbt::Compound::new();
        map.insert(
            format::TYPE.into(),
            Tag::String(self.envelope.type_key.as_str().into()),
        );
        map.insert(format::POSITION.into(), doubles(self.position));
        map.insert(format::VELOCITY.into(), doubles(self.velocity));
        map.insert(
            format::ROTATION.into(),
            Tag::List(
                self.rotation
                    .map(|value| Tag::Float(value.to_bits()))
                    .to_vec(),
            ),
        );
        map.insert(format::UUID.into(), Tag::IntArray(self.uuid.to_vec()));
        if let Some(owner) = self.owner {
            map.insert(format::OWNER.into(), Tag::IntArray(owner.to_vec()));
        }
        if let Some(name) = &self.custom_name {
            map.insert(
                format::CUSTOM_NAME.into(),
                sunrise_game_data::scoreboard::component_tag(&name.component()),
            );
        }
        if self.custom_name_visible {
            map.insert(format::CUSTOM_NAME_VISIBLE.into(), Tag::Byte(1));
        }
        self.base.write(&mut map, owned::ALL);
        map.insert(format::ON_GROUND.into(), Tag::Byte(self.on_ground.into()));
        map.insert(
            format::FALL_DISTANCE.into(),
            Tag::Double(self.fall_distance.to_bits()),
        );
        map.insert(
            format::PAYLOAD_VERSION.into(),
            Tag::Short(i16::from_ne_bytes(
                self.envelope.payload_version.to_ne_bytes(),
            )),
        );
        map.insert(
            format::PAYLOAD.into(),
            Tag::ByteArray(self.envelope.payload_bytes.clone()),
        );
        if !self.passengers.is_empty() {
            map.insert(
                format::PASSENGERS.into(),
                Tag::List(self.passengers.iter().map(Self::to_tag).collect()),
            );
        }
        Tag::Compound(map)
    }
}

/// The deepest nesting of one record compound's own data, the passenger
/// wrappers excluded: each riding record is a compound of the same shape as
/// its root, so its depth is checked where it is read; the passenger chain
/// itself is bounded only by the NBT nesting limit the tag was read under.
fn data_nesting(tag: &Tag, passengers_key: &sunrise_core::nbt::JavaString) -> u32 {
    match tag {
        Tag::Compound(map) => {
            let riders = map.get(passengers_key);
            1 + map
                .values()
                .filter(|value| !riders.is_some_and(|riding| std::ptr::eq(*value, riding)))
                .map(|value| data_nesting(value, passengers_key))
                .max()
                .unwrap_or(0)
        }
        Tag::List(values) => {
            1 + values
                .iter()
                .map(|value| data_nesting(value, passengers_key))
                .max()
                .unwrap_or(0)
        }
        _ => 1,
    }
}

/// The encoded NBT size of a tag, as its writer emits it (name payloads
/// included, strings counted as their UTF-8 bytes): the measure the
/// quarantine record bound applies.
pub fn nbt_size(tag: &Tag) -> u64 {
    match tag {
        Tag::End => 1,
        Tag::Byte(_) => 1,
        Tag::Short(_) => 2,
        Tag::Int(_) => 4,
        Tag::Long(_) => 8,
        Tag::Float(_) => 4,
        Tag::Double(_) => 8,
        Tag::ByteArray(bytes) => 4 + bytes.len() as u64,
        Tag::String(text) => 2 + text.to_string_lossy().len() as u64,
        Tag::List(values) => 5 + values.iter().map(nbt_size).sum::<u64>(),
        Tag::Compound(map) => {
            1 + map
                .iter()
                .map(|(field, value)| {
                    1 + 2 + field.to_string_lossy().len() as u64 + nbt_size(value)
                })
                .sum::<u64>()
        }
        Tag::IntArray(ints) => 4 + ints.len() as u64 * 4,
        Tag::LongArray(longs) => 4 + longs.len() as u64 * 8,
    }
}

/// One record compound as the save format reads it: the common fields (their
/// vanilla defaults when absent), the type envelope and the passenger tree.
/// The record's own data must nest no deeper than [`limits::MAX_NESTING`];
/// shape faults refuse with [`GeneralRecordError::InvalidEnvelope`], and
/// nothing is read from a record that refused.
#[cfg(any(test, feature = "test-support"))]
pub fn record_from_compound(tag: &Tag) -> Result<GeneralEntityRecord, GeneralRecordError> {
    read_general_record(tag, &|_| false)
}

pub fn read_general_record(
    tag: &Tag,
    loads_as_mob: &impl Fn(&Tag) -> bool,
) -> Result<GeneralEntityRecord, GeneralRecordError> {
    let passengers_key = sunrise_core::nbt::JavaString::from(format::PASSENGERS);
    let nesting = data_nesting(tag, &passengers_key);
    if nesting > limits::MAX_NESTING {
        return Err(GeneralRecordError::NestingTooDeep { depth: nesting });
    }
    let Some(Tag::String(type_key)) = tag.get(format::TYPE) else {
        return Err(GeneralRecordError::InvalidEnvelope {
            field: format::TYPE,
        });
    };
    let position = doubles3(tag, format::POSITION)?;
    let velocity = doubles3(tag, format::VELOCITY)?;
    let rotation = match tag.get(format::ROTATION) {
        None => [0.0; 2],
        Some(Tag::List(values)) if values.len() == 2 => {
            let mut out = [0.0f32; 2];
            for (slot, value) in values.iter().enumerate() {
                let Tag::Float(bits) = value else {
                    return Err(GeneralRecordError::InvalidEnvelope {
                        field: format::ROTATION,
                    });
                };
                out[slot] = f32::from_bits(*bits);
            }
            out
        }
        Some(_) => {
            return Err(GeneralRecordError::InvalidEnvelope {
                field: format::ROTATION,
            });
        }
    };
    let uuid = match tag.get(format::UUID) {
        Some(Tag::IntArray(ints)) if ints.len() == 4 => {
            let mut out = [0i32; 4];
            out.copy_from_slice(ints);
            out
        }
        _ => {
            return Err(GeneralRecordError::InvalidEnvelope {
                field: format::UUID,
            });
        }
    };
    let owner = match tag.get(format::OWNER) {
        None => None,
        Some(Tag::IntArray(ints)) if ints.len() == 4 => {
            let mut out = [0i32; 4];
            out.copy_from_slice(ints);
            Some(out)
        }
        Some(_) => {
            return Err(GeneralRecordError::InvalidEnvelope {
                field: format::OWNER,
            });
        }
    };
    // `Entity.load`: a `CustomName` that fails the component codec reads as none.
    let custom_name = tag
        .get(format::CUSTOM_NAME)
        .and_then(sunrise_sim::entity_nbt::decode_custom_name_text);
    let custom_name_visible = sunrise_sim::entity_nbt::flag(tag, format::CUSTOM_NAME_VISIBLE);
    // `Entity.load`: `getBooleanOr("OnGround", false)` and `getDoubleOr("fall_distance", 0.0)`,
    // which read any number and default on a missing key or another tag.
    let (on_ground, fall_distance) = sunrise_sim::entity_nbt::ground_from_nbt(tag);
    let payload_version = match tag.get(format::PAYLOAD_VERSION) {
        None => 0,
        Some(Tag::Short(version)) => u16::from_ne_bytes(version.to_ne_bytes()),
        Some(_) => {
            return Err(GeneralRecordError::InvalidEnvelope {
                field: format::PAYLOAD_VERSION,
            });
        }
    };
    let payload_bytes = match tag.get(format::PAYLOAD) {
        None => Vec::new(),
        Some(Tag::ByteArray(bytes)) => bytes.clone(),
        Some(_) => {
            return Err(GeneralRecordError::InvalidEnvelope {
                field: format::PAYLOAD,
            });
        }
    };
    let passengers = match tag.get(format::PASSENGERS) {
        None => Vec::new(),
        Some(Tag::List(values)) if values.iter().all(|value| matches!(value, Tag::Compound(_))) => {
            values
                .iter()
                .filter(|tag| !loads_as_mob(tag))
                .map(|tag| read_general_record(tag, loads_as_mob))
                .collect::<Result<Vec<_>, _>>()?
        }
        Some(_) => {
            return Err(GeneralRecordError::InvalidEnvelope {
                field: format::PASSENGERS,
            });
        }
    };
    Ok(GeneralEntityRecord {
        position,
        velocity,
        rotation,
        uuid,
        owner,
        custom_name,
        custom_name_visible,
        base: EntityBase::read(Some(tag)),
        on_ground,
        fall_distance,
        envelope: GeneralTypeEnvelope {
            type_key: type_key.to_string_lossy(),
            payload_version,
            payload_bytes,
        },
        passengers,
    })
}

/// A saved general entity compound as a record: it and every compound of
/// its riding tree read under the format's checks, so a refusal names the
/// first fault and leaves nothing read behind (the caller discards the
/// whole tree).
/// The frozen type registry: every declared general entity type, resolved
/// once into dense slots. The registry is data — built from the installed
/// declarations and never hand-kept — and freezes with them: duplicates
/// across mods refuse at freeze, naming the type.
pub struct GeneralTypeCatalog {
    entries: Vec<mod_api::__private::GeneralEntityTypeDeclaration>,
    by_key: FxHashMap<String, usize>,
}

impl GeneralTypeCatalog {
    /// Freeze the installed declarations into slots, sorted by declaring
    /// mod id then type id, so every side running the same mods builds the
    /// same slot numbers. A duplicate type key refuses, naming it.
    pub fn freeze(
        declarations: &[mod_api::__private::GeneralEntityTypeDeclaration],
    ) -> Result<Self, String> {
        let mut sorted = declarations.to_vec();
        sorted.sort_by(|left, right| (&left.mod_id, &left.id).cmp(&(&right.mod_id, &right.id)));
        let mut by_key = FxHashMap::with_capacity_and_hasher(sorted.len(), Default::default());
        for (slot, declaration) in sorted.iter().enumerate() {
            // Enumeration already yields the catalog index without narrowing.
            if by_key.insert(declaration.id.clone(), slot).is_some() {
                return Err(format!(
                    "duplicate general entity type {} (declared by {})",
                    declaration.id, declaration.mod_id
                ));
            }
        }
        Ok(Self {
            entries: sorted,
            by_key,
        })
    }

    /// The type's standing box at `at`, shared by server queries and client culling.
    pub fn bounds(&self, slot: usize, at: Vec3) -> Aabb {
        let (width, height) = self.dimensions(slot);
        let half = width / 2.0;
        Aabb::new(
            Vec3::new(at.x - half, at.y, at.z - half),
            Vec3::new(at.x + half, at.y + height, at.z + half),
        )
    }

    /// The slot a type key resolved to, if the registry holds it.
    pub fn slot(&self, type_key: &str) -> Option<usize> {
        self.by_key.get(type_key).copied()
    }

    /// How many types the registry froze; every slot is below this.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The type key a slot belongs to.
    pub fn key(&self, slot: usize) -> &str {
        &self.entries[slot].id
    }

    /// The range and update interval of the already-resolved type.
    pub fn tracking(&self, slot: usize) -> sunrise_sim::lan::tracking::General {
        (&self.entries[slot]).into()
    }

    pub(in crate::game) fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|entry| entry.id.as_str())
    }

    /// The type's declared width and height, for the row's bounds.
    pub fn dimensions(&self, slot: usize) -> (f64, f64) {
        let entry = &self.entries[slot];
        (entry.width, entry.height)
    }

    /// The type's local passenger point, defaulting to the top center of its box.
    pub(in crate::game) fn passenger_attachment(&self, slot: usize) -> [f64; 3] {
        let entry = &self.entries[slot];
        entry
            .passenger_attachment
            .unwrap_or([0.0, entry.height, 0.0])
    }

    pub fn pickable(&self, slot: usize) -> bool {
        self.entries[slot].pickable
    }

    /// Whether the type was declared fire immune (`EntityType.fireImmune`).
    pub fn fire_immune(&self, slot: usize) -> bool {
        self.entries
            .get(slot)
            .is_some_and(|entry| entry.fire_immune)
    }

    pub(in crate::game) fn piston_push_reaction(
        &self,
        slot: usize,
    ) -> sunrise_world::level::PushReaction {
        super::api_values::push_reaction(&self.entries[slot].piston_push_reaction).1
    }

    /// The type's declared payload schema version, the one a fresh row starts at.
    pub fn payload_version(&self, slot: usize) -> u16 {
        self.entries[slot].payload_version
    }

    /// The type's declared save policy, the writer of a record consults: a
    /// `noSave` type's rows produce zero persisted records.
    pub fn save_policy(&self, slot: usize) -> mod_api::GeneralSavePolicy {
        self.entries[slot].save_policy
    }

    /// The Minecraft feature flags this type requires (`EntityType.requiredFeatures`).
    pub fn required_features(&self, slot: usize) -> &[mod_api::FeatureFlag] {
        &self.entries[slot].required_features
    }

    /// Whether every Minecraft feature flag this type requires is enabled in `enabled_features`
    /// (`EntityType.isEnabled(FeatureFlagSet)`).
    pub fn is_enabled(&self, slot: usize, enabled_features: &mod_api::FeatureFlags) -> bool {
        enabled_features.contains_all(self.required_features(slot))
    }
}

/// The game's frozen registry, built from the installed declarations: the
/// types were declared before any world or connection opened, and the
/// installed set never changes after. The cached catalog is keyed on the
/// declarations' own address, so a process sees one build and a test thread
/// that swaps its installed parts (`TEST_PARTS`) resolves the new set
/// instead of the first one it touched.
pub fn general_type_catalog() -> &'static GeneralTypeCatalog {
    thread_local! {
        static CACHED: std::cell::RefCell<Option<(usize, &'static GeneralTypeCatalog)>> =
            const { std::cell::RefCell::new(None) };
    }
    let declarations = mod_api::__private::general_entity_type_declarations();
    let at = declarations.as_ptr() as usize;
    CACHED.with(|cached| {
        if let Some((frozen_at, catalog)) = *cached.borrow()
            && frozen_at == at
        {
            return catalog;
        }
        let catalog = Box::leak(Box::new(
            GeneralTypeCatalog::freeze(declarations).unwrap_or_else(|duplicate| {
                // Unreachable: a duplicate key was already refused at
                // registration — every mod declares through the one
                // registrar `register_all` runs (`parts/shared.rs`), so the
                // second mod's `declare_general_entity` sees the first's
                // key and returns `Duplicate`, never reaching here.
                unreachable!("duplicate general entity type at freeze: {duplicate}")
            }),
        ));
        *cached.borrow_mut() = Some((at, catalog));
        catalog
    })
}

/// The engine's live general rows (#792): identity, join identity, type
/// slot, common state and the attachment holder. No runtime slot or pointer
/// persists; identity-to-row lookup stays with `EntityTable`
/// (`entity_lookup`, `server_entity_lookup`), never a second index here.
#[derive(Clone, Debug)]
pub struct GeneralRow {
    /// The row's runtime network id, from the game's one counter.
    pub id: u32,
    /// The row's UUID; a reload continues the identity, not old handles.
    pub uuid: [i32; 4],
    /// The row's join identity: its admission into the resident table.
    pub join: JoinSeq,
    /// The frozen registry's slot for the row's type.
    pub type_slot: usize,
    /// The position in blocks.
    pub position: Vec3,
    /// Where the row stood when the last tick began.
    pub previous: Vec3,
    /// The velocity in blocks per tick.
    pub velocity: Vec3,
    /// The yaw and pitch in degrees.
    pub rotation: [f32; 2],
    /// The yaw and pitch when the last tick began (`Entity.yRotO`, `xRotO`).
    pub previous_rotation: [f32; 2],
    /// The UUID of the entity that fired it, kept and saved as `Owner`
    /// (`TraceableEntity.getOwner`); a spawn sets it.
    pub owner: Option<[i32; 4]>,
    /// `Entity.fallDistance`: saved as `fall_distance`, accumulated by a collided move and
    /// reset or halved by the base tick's fluid step.
    pub fall_distance: f64,
    /// `Entity.onGround`, saved as `OnGround`; a collided move writes it.
    pub on_ground: bool,
    /// `Entity.tickCount`: counted by the base tick, wraps as Java's `int` does, not saved.
    pub tick_count: i32,
    /// `Entity.noPhysics`: its moves pass through blocks, and its collision flags stay false.
    /// Not saved, and false unless a mod writes it.
    pub no_physics: bool,
    /// `Entity.horizontalCollision` of the last collided move.
    pub horizontal_collision: bool,
    /// `Entity.verticalCollision` of the last collided move.
    pub vertical_collision: bool,
    /// `Entity.verticalCollisionBelow` of the last collided move.
    pub vertical_collision_below: bool,
    /// The fluids the last base tick found it in (`Entity.updateFluidInteraction`).
    pub fluids: sunrise_sim::mobs::living::Fluids,
    /// `Entity.isUnderWater`: the eye was in water at the start of the base tick, and the
    /// body touches water after it.
    pub under_water: bool,
    /// `Entity.getCustomName`; synced to tracking clients as `Entity.DATA_CUSTOM_NAME`.
    pub custom_name: Option<Text>,
    /// `Entity.isCustomNameVisible`; synced as `Entity.DATA_CUSTOM_NAME_VISIBLE`.
    pub custom_name_visible: bool,
    /// The persisted entity flags and fire ticks, including the no-gravity flag read by the type's tick.
    pub base: EntityBase,
    /// The payload's schema version, as the record carried it and the writer
    /// returns it: no decoder owns it before stage 4.
    pub payload_version: u16,
    /// The record's payload bytes, held as loaded and written back unchanged.
    pub payload_bytes: Vec<u8>,
    /// The entity this row rides (`None` for a root). Riders save inside the
    /// vehicle's `Passengers`, never as their own root records.
    pub mount: Option<u32>,
    /// This row's seat order on its current vehicle.
    pub vehicle_order: i64,
    /// `Entity.boardingCooldown`, set on dismount and counted down by ticks.
    pub boarding_cooldown: i32,
    /// The entity-owned attachment registration lifetime (#796: the sole
    /// Part-data store; there is no parallel entity data map).
    pub attachment_lifetime: sunrise_mod_runtime::attachments::AttachmentLifetime,
}

impl EntityRules for GeneralRow {}
impl_joined!(GeneralRow);

impl GeneralRow {
    /// `Entity.getEyeY`: the feet plus `EntityDimensions.defaultEyeHeight`, `height * 0.85F`.
    fn eye_y(feet_y: f64, height: f64) -> f64 {
        feet_y + f64::from(height as f32 * 0.85_f32)
    }

    /// `Entity.commonTick` and `Entity.baseTick` for one tick of the row, before its type's
    /// tick runs: `tickCount`, the old position and rotation, the boarding cooldown, then
    /// `updateFluidInteraction` (a current pushes the velocity, as `isPushedByFluid` allows, also
    /// a riding row's, which is zeroed first as `rideTick` does; water resets the fall and lava
    /// halves it) and `checkBelowWorld`. `bounds` is the row's box at its position and `height`
    /// the type's declared height, whose `0.85F` is its eye height; `scan` measures the fluids for
    /// the box, the position and the eye's height and may add a current to the velocity it is
    /// given. False when the row fell below the world (`min_y - 64`): the caller discards it.
    /// Gravity, drag and moves are the type's own tick.
    pub fn base_tick(
        &mut self,
        bounds: Aabb,
        height: f64,
        mounted: bool,
        min_y: i32,
        scan: impl FnOnce(Aabb, Vec3, f64, &mut Vec3) -> sunrise_sim::mobs::living::Fluids,
    ) -> bool {
        let first_tick = self.tick_count == 0;
        self.tick_count = self.tick_count.wrapping_add(1);
        self.previous = self.position;
        self.previous_rotation = self.rotation;
        self.boarding_cooldown = (self.boarding_cooldown - 1).max(0);
        if mounted {
            self.velocity = Vec3::ZERO;
        }
        let was_eye_in_water = self.fluids.eye_in_water;
        let (at, eye) = (self.position, Self::eye_y(self.position.y, height));
        // `Entity.isPushedByFluid` is true for a general row, so the scan pushes its velocity.
        let mut fluids = scan(bounds, at, eye, &mut self.velocity);
        // `Entity.isInLava` is false on the first tick (`firstTick`).
        fluids.in_lava &= !first_tick;
        if fluids.in_water {
            self.fall_distance = 0.0;
        }
        if fluids.in_lava {
            self.fall_distance *= 0.5;
        }
        self.under_water = was_eye_in_water && fluids.in_water;
        self.fluids = fluids;
        self.position.y >= f64::from(min_y - 64)
    }

    /// The part of the end of `Entity.move` that needs no level, for a collided move that wanted
    /// `wanted` and resolved to `moved`, whatever its cause: the collision flags and the ground,
    /// and `checkFallDamage`'s fall distance. `Game::finish_general_move` runs the rest, in
    /// `Entity.move`'s order: the fall-reset ray before the placement, then
    /// `restituteMovementAfterCollisions` and `getBlockSpeedFactor` after this. A `noPhysics` row
    /// skips all of it.
    pub fn finish_move(&mut self, wanted: Vec3, moved: Vec3) {
        use sunrise_world::entity_move::different;
        if self.no_physics {
            return;
        }
        let blocked_x = different(wanted.x, moved.x);
        let blocked_z = different(wanted.z, moved.z);
        self.horizontal_collision = blocked_x || blocked_z;
        self.vertical_collision = wanted.y != moved.y;
        self.vertical_collision_below = self.vertical_collision && wanted.y < 0.0;
        self.on_ground = self.vertical_collision_below;
        // `Entity.checkFallDamage`: the distance grows by a downward move (as a float) outside
        // water, and landing forgets it.
        if !self.fluids.in_water && moved.y < 0.0 {
            self.fall_distance -= f64::from(moved.y as f32);
        }
        if self.on_ground {
            self.fall_distance = 0.0;
        }
    }
}
sunrise_mod_runtime::impl_attachment_lifetime!(GeneralRow);

/// The general rows' list, one per level, bound to residency like the other
/// families' lists.
pub type GeneralRows = JoinList<GeneralRow, true>;

/// The game's general entity store: one join list per level, wired once
/// into `resident_owners!` and `entity_lists!` as `Kind::General`.
#[derive(Default)]
pub struct GeneralStore {
    /// The level's general rows, in join order.
    pub rows: GeneralRows,
    /// The rows' rider adjacency, reused across the level's entity passes
    /// (`tick.rs` refreshes it).
    pub riders: RiderIndex,
    /// Each mount rewrite retires the rider adjacency, independently of the
    /// join list's structural write stamp.
    pub mount_writes: u64,
}

impl GeneralStore {
    pub fn row(&self, id: u32) -> Option<&GeneralRow> {
        self.rows.iter().find(|row| row.id == id)
    }

    pub fn row_mut(&mut self, id: u32) -> Option<&mut GeneralRow> {
        self.rows.iter_mut().find(|row| row.id == id)
    }
}

/// Root-to-rider adjacency of the general rows, as a first-child and
/// next-sibling index over the rows' places: rebuilt in place, and only
/// when the rows' storage changed or a mount was rewritten, so a stable
/// world's pass reuses its allocations. Sibling chains use board order,
/// which is independent of when the entities themselves joined the level.
#[derive(Default)]
pub struct RiderIndex {
    stamp: Option<sunrise_sim::join_seq::ListStamp>,
    length: usize,
    mounts: u64,
    /// Each vehicle's first general rider. Places at and above `length` are
    /// foreign vehicles, with no corresponding general row.
    pub first_child: Vec<Option<usize>>,
    /// Each rider's next sibling's place on the same vehicle.
    pub next_sibling: Vec<Option<usize>>,
    /// Each vehicle's latest rider of the ongoing rebuild.
    last_child: Vec<Option<usize>>,
    /// A general row or foreign vehicle's place by id.
    places: FxHashMap<u32, usize>,
    boarded: Vec<(usize, u32)>,
    pub ticked: Vec<bool>,
    pub pending: Vec<(usize, Option<u32>)>,
    pub foreign_parents: Vec<(usize, i64, usize)>,
    /// General vehicles a mob rides, from the mob riders' pass of the same tick.
    pub mob_vehicles: sunrise_level::fast_hash::FxHashSet<u32>,
}

impl RiderIndex {
    /// Whether the index describes `rows` and their mounts as they stand now, so a lookup
    /// through it needs no scan of the rows. It is current after each tick pass's refresh and
    /// stale only from the first row insert, removal or mount write after it until the next
    /// refresh: lookups in that window scan instead, so a pass whose listeners change many
    /// mounts pays one scan per lookup, never a wrong answer (a lookup takes `&self`, so it
    /// cannot rebuild the index itself).
    fn is_fresh(&self, rows: &GeneralRows, mounts: u64) -> bool {
        self.stamp == Some(rows.stamp()) && self.length == rows.len() && self.mounts == mounts
    }

    /// Re-derive the adjacency unless nothing changed since the last pass:
    /// one pass over the rows, each edge written once, no allocation while
    /// the row count and the mounts hold still.
    pub fn refresh(&mut self, rows: &GeneralRows, mounts: u64) {
        let stamp = rows.stamp();
        if self.is_fresh(rows, mounts) {
            return;
        }
        self.stamp = Some(stamp);
        self.length = rows.len();
        self.mounts = mounts;
        // The buffers are reused — clear first and resize back, keeping the
        // capacity: a plain resize keeps the surviving slots' old links too,
        // so a row list that shrank leaves links to places that no longer
        // exist, and the next pass would walk off the end.
        self.first_child.clear();
        self.first_child.resize(rows.len(), None);
        self.next_sibling.clear();
        self.next_sibling.resize(rows.len(), None);
        self.last_child.clear();
        self.last_child.resize(rows.len(), None);
        self.places.clear();
        for (place, row) in rows.iter().enumerate() {
            self.places.insert(row.id, place);
        }
        self.boarded.clear();
        self.boarded.extend(
            rows.iter()
                .enumerate()
                .filter_map(|(place, row)| Some((place, row.mount?))),
        );
        self.boarded
            .sort_unstable_by_key(|&(place, _)| (rows[place].vehicle_order, rows[place].id));
        for &(place, vehicle) in &self.boarded {
            let vehicle_place = *self.places.entry(vehicle).or_insert_with(|| {
                let place = self.first_child.len();
                self.first_child.push(None);
                self.last_child.push(None);
                place
            });
            match self.last_child[vehicle_place] {
                // The chain's tail takes the new rider; the head opens it.
                Some(tail) => self.next_sibling[tail] = Some(place),
                None => self.first_child[vehicle_place] = Some(place),
            }
            self.last_child[vehicle_place] = Some(place);
        }
    }
}

impl crate::game::CommonGame<'_> {
    pub fn general_passengers(&self, vehicle: u32) -> impl Iterator<Item = &GeneralRow> {
        let store = &self.resident_entities.general;
        let fresh = store.riders.is_fresh(&store.rows, store.mount_writes);
        // The vehicle's sibling chain while the index is current, else every row.
        let mut next = fresh
            .then(|| store.riders.places.get(&vehicle))
            .flatten()
            .and_then(|&place| store.riders.first_child[place]);
        let mut scan = (!fresh && !store.rows.is_empty()).then(|| store.rows.iter());
        std::iter::from_fn(move || {
            loop {
                let row = match scan.as_mut() {
                    Some(rows) => rows.find(|row| row.mount == Some(vehicle))?,
                    None => {
                        let place = next?;
                        next = store.riders.next_sibling[place];
                        &store.rows[place]
                    }
                };
                if self.persist.removals.is_empty()
                    || !self.persist.removals.contains(&Target::General(row.id))
                {
                    return Some(row);
                }
            }
        })
    }
}

impl crate::game::CommonGame<'_> {
    /// A general entity row in the active level. Removal hides it before the sweep.
    pub fn general_row(&self, id: u32) -> Option<&GeneralRow> {
        if !self.persist.removals.is_empty() && self.persist.removals.contains(&Target::General(id))
        {
            return None;
        }
        self.general_object(id)
    }

    /// The owning row through its removal callback, before the physical sweep. Through the rider
    /// index's place map while it is current (a miss then is final), else by scan.
    pub fn general_object(&self, id: u32) -> Option<&GeneralRow> {
        let store = &self.resident_entities.general;
        if store.rows.is_empty() {
            return None;
        }
        if store.riders.is_fresh(&store.rows, store.mount_writes) {
            let &place = store.riders.places.get(&id)?;
            return store.rows.get(place);
        }
        store.rows.iter().find(|row| row.id == id)
    }
}

/// Whether a kind is a namespaced non-`minecraft` one — the shape a
/// quarantined (missing-mod) record's kind has.
pub fn quarantined_kind(kind: &str) -> bool {
    kind.split_once(':')
        .is_some_and(|(namespace, _)| namespace != "minecraft")
}

impl Entity for GeneralRow {
    fn id(&self) -> u32 {
        self.id
    }
    fn position(&self) -> Vec3 {
        self.position
    }
    fn position_mut(&mut self) -> &mut Vec3 {
        &mut self.position
    }
    fn previous_position(&self) -> Vec3 {
        self.previous
    }
    fn velocity(&self) -> Vec3 {
        self.velocity
    }
    fn velocity_mut(&mut self) -> &mut Vec3 {
        &mut self.velocity
    }
    fn fall_distance(&self) -> f64 {
        self.fall_distance
    }
    fn reset_fall_distance(&mut self) {
        self.fall_distance = 0.0;
    }

    fn on_ground(&self) -> bool {
        self.on_ground
    }

    fn set_on_ground(&mut self, on_ground: bool) {
        self.on_ground = on_ground;
    }

    fn set_piston_on_ground(&mut self, on_ground: bool) {
        self.on_ground = on_ground;
    }

    fn no_physics(&self, _stored: Option<bool>) -> bool {
        self.no_physics
    }
    /// Places the row by the movement the collision scan resolved (or, with `noPhysics`, the
    /// whole requested one). `Entity.move`'s end, the collision flags and the fall check, is
    /// [`GeneralRow::finish_move`], and its restitution and speed factor are
    /// `Game::finish_general_move`, run by the caller once the scan is known.
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let position = self.position_mut();
        *position = position.add_vector(movement);
        if self.no_physics {
            self.horizontal_collision = false;
            self.vertical_collision = false;
            self.vertical_collision_below = false;
        }
    }
    fn rotation(&self) -> [f32; 2] {
        self.rotation
    }

    #[inline]
    fn previous_rotation(&self) -> [f32; 2] {
        self.previous_rotation
    }
    fn set_rotation(&mut self, rotation: [f32; 2]) {
        self.rotation = rotation;
        self.previous_rotation = rotation;
    }
    fn bounds(&self) -> Aabb {
        general_type_catalog().bounds(self.type_slot, self.position)
    }
    fn is_pickable(&self) -> bool {
        general_type_catalog().pickable(self.type_slot)
    }
    fn fluid_eye_y(&self) -> f64 {
        Self::eye_y(
            self.position.y,
            general_type_catalog().dimensions(self.type_slot).1,
        )
    }
    fn selector_target(&self) -> Option<Target> {
        Some(Target::General(self.id))
    }
    fn get_piston_push_reaction(&self) -> sunrise_world::level::PushReaction {
        general_type_catalog().piston_push_reaction(self.type_slot)
    }
    fn native_type(&self) -> Cow<'_, str> {
        Cow::Borrowed(general_type_catalog().key(self.type_slot))
    }
}

#[cfg(test)]
#[path = "general/physics_tests.rs"]
mod physics_tests;

#[cfg(test)]
pub mod tests {
    use super::*;

    fn record(uuid: [i32; 4], type_key: &str) -> GeneralEntityRecord {
        GeneralEntityRecord {
            position: [0.0, 64.0, 0.0],
            velocity: [0.0, 0.0, 0.0],
            rotation: [0.0, 0.0],
            owner: None,
            custom_name: None,
            custom_name_visible: false,
            base: Default::default(),
            on_ground: false,
            fall_distance: 0.0,
            uuid,
            envelope: GeneralTypeEnvelope {
                type_key: type_key.to_owned(),
                payload_version: 1,
                payload_bytes: Vec::new(),
            },
            passengers: Vec::new(),
        }
    }

    fn known(type_key: &str) -> impl Fn(&str) -> bool {
        let owned = type_key.to_owned();
        move |key| key == owned
    }

    /// A minimal valid tree checks ready, whatever its passengers' order.
    #[test]
    fn valid_record_with_passengers_checks_ready() {
        let mut root = record([1, 2, 3, 4], "mymod:robot");
        let mut rider = record([5, 6, 7, 8], "mymod:robot");
        rider
            .passengers
            .push(record([9, 10, 11, 12], "mymod:robot"));
        root.passengers.push(rider);
        assert_eq!(
            check_general_record(&root, known("mymod:robot")),
            Ok(GeneralRecordCheck::Ready)
        );
    }

    /// A mod payload one byte over the bound refuses with its size; at the
    /// bound it stands. Vanilla records never pass through this codec, so
    /// the bound cannot clip them.
    #[test]
    fn payload_bound_refuses_one_byte_over() {
        let mut over = record([1, 2, 3, 4], "mymod:robot");
        over.envelope.payload_bytes = vec![0; 65_537];
        assert_eq!(
            check_general_record(&over, known("mymod:robot")),
            Err(GeneralRecordError::PayloadTooLarge { size: 65_537 })
        );
        let mut at = record([5, 6, 7, 8], "mymod:robot");
        at.envelope.payload_bytes = vec![0; 65_536];
        assert_eq!(
            check_general_record(&at, known("mymod:robot")),
            Ok(GeneralRecordCheck::Ready)
        );
    }

    /// A passenger chain has no depth bound of its own, as in vanilla: even a
    /// chain far deeper than any NBT could hold checks without exhausting the stack.
    #[test]
    #[ignore = "daily"]
    fn a_deep_passenger_chain_checks_iteratively() {
        let mut root = record([1, 2, 3, 4], "mymod:robot");
        let mut node = &mut root;
        for index in 0..100_000 {
            node.passengers
                .push(record([index + 1, 0, 0, 0], "mymod:robot"));
            node = node.passengers.last_mut().unwrap();
        }
        assert_eq!(
            check_general_record(&root, known("mymod:robot")),
            Ok(GeneralRecordCheck::Ready)
        );
        // Tear down iteratively too: recursive Vec drop is not the validator.
        while let Some(child) = root.passengers.pop() {
            root = child;
        }
    }

    #[test]
    fn non_finite_rotation_refuses() {
        for bad in [f32::NAN, f32::INFINITY] {
            let mut record = record([1, 2, 3, 4], "mymod:robot");
            record.rotation[0] = bad;
            assert_eq!(
                check_general_record(&record, known("mymod:robot")),
                Err(GeneralRecordError::NotFinite)
            );
        }
    }

    /// A passenger carrying its own ancestor's UUID is a cycle, refused
    /// wherever in the tree the repeat hides.
    #[test]
    fn passenger_cycle_refuses() {
        let mut root = record([1, 2, 3, 4], "mymod:robot");
        let mut rider = record([5, 6, 7, 8], "mymod:robot");
        // The rider's passenger is the root again.
        rider.passengers.push(record([1, 2, 3, 4], "mymod:robot"));
        root.passengers.push(rider);
        assert_eq!(
            check_general_record(&root, known("mymod:robot")),
            Err(GeneralRecordError::PassengerCycle { uuid: [1, 2, 3, 4] })
        );
    }

    /// Two records of one UUID in one tree refuse, siblings included.
    #[test]
    fn duplicate_uuid_refuses() {
        let mut root = record([1, 2, 3, 4], "mymod:robot");
        root.passengers.push(record([5, 6, 7, 8], "mymod:robot"));
        root.passengers.push(record([5, 6, 7, 8], "mymod:robot"));
        assert_eq!(
            check_general_record(&root, known("mymod:robot")),
            Err(GeneralRecordError::DuplicateUuid { uuid: [5, 6, 7, 8] })
        );
    }

    /// A NaN or infinite position, velocity or rotation component refuses
    /// before anything else is consulted.
    #[test]
    fn non_finite_coordinates_refuse() {
        for (slot, bad) in [(0, f64::NAN), (1, f64::NEG_INFINITY), (2, f64::NAN)] {
            let mut record = record([1, 2, 3, 4], "mymod:robot");
            record.position[slot] = bad;
            assert_eq!(
                check_general_record(&record, known("mymod:robot")),
                Err(GeneralRecordError::NotFinite)
            );
        }
        let mut velocity = record([1, 2, 3, 4], "mymod:robot");
        velocity.velocity[1] = f64::INFINITY;
        assert_eq!(
            check_general_record(&velocity, known("mymod:robot")),
            Err(GeneralRecordError::NotFinite)
        );
    }

    /// A structurally sound record of a type whose namespace belongs to
    /// neither vanilla nor a loaded mod reports `UnknownType`, the outcome
    /// stage 3 keeps as quarantined data; a structural fault in the same
    /// tree still refuses, so quarantine never preserves a bad payload.
    #[test]
    fn unknown_type_is_its_own_outcome() {
        let unknown = record([1, 2, 3, 4], "gone:robot");
        assert_eq!(
            check_general_record(&unknown, known("mymod:robot")),
            Ok(GeneralRecordCheck::UnknownType)
        );
        // A passenger of an unknown type poisons the root's readiness too:
        // the tree keeps together.
        let mut mixed = record([5, 6, 7, 8], "mymod:robot");
        mixed.passengers.push(record([9, 10, 11, 12], "gone:robot"));
        assert_eq!(
            check_general_record(&mixed, known("mymod:robot")),
            Ok(GeneralRecordCheck::UnknownType)
        );
        // But a structural fault refuses even for the unknown type.
        let mut over = record([13, 14, 15, 16], "gone:robot");
        over.envelope.payload_bytes = vec![0; 65_537];
        assert_eq!(
            check_general_record(&over, known("mymod:robot")),
            Err(GeneralRecordError::PayloadTooLarge { size: 65_537 })
        );
    }

    fn declaration(mod_id: &str, id: &str) -> mod_api::__private::GeneralEntityTypeDeclaration {
        mod_api::__private::GeneralEntityTypeDeclaration {
            mod_id: mod_id.to_owned(),
            id: id.to_owned(),
            category: mod_api::GeneralEntityCategory::Misc,
            width: 0.8,
            height: 1.9,
            passenger_attachment: None,
            save_policy: mod_api::GeneralSavePolicy::Save,
            summonable: true,
            pickable: false,
            fire_immune: false,
            piston_push_reaction: mod_api::PushReaction::PushPull,
            tracking_chunks: 8,
            update_interval: 3,
            payload_version: 1,
            required_features: Vec::new(),
        }
    }

    /// The frozen registry resolves slots in one stable order — declaring
    /// mod id, then type id — and a duplicate type key refuses at freeze,
    /// naming it, so two mods cannot claim one slot number.
    #[test]
    fn catalog_freezes_slots_in_declaration_order_and_refuses_duplicates() {
        let catalog = GeneralTypeCatalog::freeze(&[
            declaration("zmod", "zmod:zeal"),
            declaration("amod", "amod:alpha"),
            declaration("amod", "amod:beta"),
        ])
        .unwrap();
        assert_eq!(catalog.key(0), "amod:alpha");
        assert_eq!(catalog.key(1), "amod:beta");
        assert_eq!(catalog.key(2), "zmod:zeal");
        assert_eq!(catalog.slot("amod:beta"), Some(1));
        assert_eq!(catalog.dimensions(2), (0.8, 1.9));
        match GeneralTypeCatalog::freeze(&[
            declaration("amod", "amod:same"),
            declaration("bmod", "amod:same"),
        ]) {
            Err(refusal) => assert!(refusal.contains("duplicate general entity type amod:same")),
            Ok(_) => panic!("a duplicate type key froze"),
        }
    }

    #[test]
    fn catalog_enforces_required_minecraft_feature_flags() {
        let mut plain = declaration("amod", "amod:plain");
        plain.required_features = Vec::new();
        let mut redstone = declaration("amod", "amod:redstone");
        redstone.required_features = vec![mod_api::FeatureFlag::RedstoneExperiments];
        let mut both = declaration("amod", "amod:both");
        both.required_features = vec![
            mod_api::FeatureFlag::RedstoneExperiments,
            mod_api::FeatureFlag::MinecartImprovements,
        ];
        let catalog = GeneralTypeCatalog::freeze(&[plain, redstone, both]).unwrap();
        let plain_slot = catalog.slot("amod:plain").unwrap();
        let redstone_slot = catalog.slot("amod:redstone").unwrap();
        let both_slot = catalog.slot("amod:both").unwrap();

        let vanilla_only = mod_api::FeatureFlags::new(["minecraft:vanilla".to_owned()].into());
        assert!(catalog.is_enabled(plain_slot, &vanilla_only));
        assert!(!catalog.is_enabled(redstone_slot, &vanilla_only));
        assert!(!catalog.is_enabled(both_slot, &vanilla_only));

        let with_redstone = mod_api::FeatureFlags::new(
            [
                "minecraft:vanilla".to_owned(),
                "minecraft:redstone_experiments".to_owned(),
            ]
            .into(),
        );
        assert!(catalog.is_enabled(plain_slot, &with_redstone));
        assert!(catalog.is_enabled(redstone_slot, &with_redstone));
        assert!(!catalog.is_enabled(both_slot, &with_redstone));

        let with_both = mod_api::FeatureFlags::new(
            [
                "minecraft:vanilla".to_owned(),
                "minecraft:redstone_experiments".to_owned(),
                "minecraft:minecart_improvements".to_owned(),
            ]
            .into(),
        );
        assert!(catalog.is_enabled(both_slot, &with_both));
    }

    #[test]
    fn catalog_slots_do_not_wrap_at_u16_boundary() {
        let declarations: Vec<_> = (0..65_537)
            .map(|index| declaration("amod", &format!("amod:type_{index:05}")))
            .collect();
        let catalog = GeneralTypeCatalog::freeze(&declarations).unwrap();
        assert_eq!(catalog.slot("amod:type_65535"), Some(65_535));
        assert_eq!(catalog.slot("amod:type_65536"), Some(65_536));
        assert_eq!(catalog.key(65_536), "amod:type_65536");
        assert_eq!(
            catalog.save_policy(65_536),
            mod_api::GeneralSavePolicy::Save
        );
    }

    /// A general row pushed to its list binds residency the way
    /// `resident_owners!` wires `Kind::General` (the default arm's
    /// `bind_residency` with the row's id and join identity), and the entity
    /// table then resolves the row's id to that owner: the one lookup path
    /// every family shares, with no second index.
    #[test]
    fn general_row_binds_and_resolves_through_the_entity_table() {
        use mod_api::DimensionKey;
        use sunrise_sim::join_seq::residency::table::{EntityLookup, EntityTable, SourceEntity};
        use sunrise_sim::join_seq::residency::{Journal, Kind, Owner};
        let journal = Journal::default();
        let mut rows: GeneralRows = Default::default();
        let owner = Owner {
            dimension: DimensionKey::OVERWORLD,
            kind: Kind::General,
        };
        rows.bind_residency(&journal, owner, |row| (row.id, row.join));
        rows.push(GeneralRow {
            id: 42,
            uuid: [1, 2, 3, 4],
            join: sunrise_sim::join_seq::JoinSeq::UNJOINED,
            type_slot: 0,
            position: Vec3::new(0.5, 64.0, 0.5),
            previous: Vec3::new(0.5, 64.0, 0.5),
            velocity: Vec3::ZERO,
            rotation: [0.0, 0.0],
            previous_rotation: [0.0, 0.0],
            owner: None,
            fall_distance: 0.0,
            custom_name: None,
            custom_name_visible: false,
            base: Default::default(),
            on_ground: false,
            tick_count: 0,
            no_physics: false,
            horizontal_collision: false,
            vertical_collision: false,
            vertical_collision_below: false,
            fluids: Default::default(),
            under_water: false,
            payload_version: 1,
            payload_bytes: Vec::new(),
            mount: None,
            vehicle_order: 0,
            boarding_cooldown: 0,
            attachment_lifetime: Default::default(),
        });
        let mut table = EntityTable::new(journal);
        let EntityLookup::Found { location, .. } =
            table.entity_lookup(&DimensionKey::OVERWORLD, 42, |at| {
                rows.locate(at.join).and_then(|index| {
                    rows.get(index).map(|row| SourceEntity {
                        id: row.id,
                        value: index,
                    })
                })
            })
        else {
            panic!("the general row never became resident");
        };
        assert_eq!(location.owner, owner);
        // The list keeps join order: removal is not a swap, and the row's
        // slot moves with it.
        let index = rows.len() - 1;
        assert_eq!(rows[index].id, 42);
        assert_ne!(rows[index].join, sunrise_sim::join_seq::JoinSeq::UNJOINED);
    }

    /// A refusal leaves no partial state: the very next valid record checks
    /// cleanly through the same entry point.
    #[test]
    fn refused_record_does_not_poison_the_next_one() {
        let mut over = record([1, 2, 3, 4], "mymod:robot");
        over.envelope.payload_bytes = vec![0; 65_537];
        assert!(check_general_record(&over, known("mymod:robot")).is_err());
        let good = record([5, 6, 7, 8], "mymod:robot");
        assert_eq!(
            check_general_record(&good, known("mymod:robot")),
            Ok(GeneralRecordCheck::Ready)
        );
    }

    /// The record codec round-trips a tree through the save format: what
    /// `to_tag` writes is what `record_from_tag` reads back, passengers and
    /// payload bytes included.
    #[test]
    fn record_round_trips_through_the_save_tag() {
        let mut root = record([1, 2, 3, 4], "mymod:robot");
        root.position = [0.5, 64.0, -12.25];
        root.velocity = [0.0, -0.08, 0.25];
        root.rotation = [90.0, -45.0];
        root.envelope.payload_version = 7;
        root.envelope.payload_bytes = vec![9, 8, 7, 6];
        let mut rider = record([5, 6, 7, 8], "mymod:drone");
        rider.rotation = [-180.0, 30.0];
        rider.envelope.payload_bytes = vec![1];
        root.passengers.push(rider);
        let tag = root.to_tag();
        assert_eq!(record_from_compound(&tag), Ok(root));
    }

    /// The writer names the fields the format fixes: the common fields under
    /// their vanilla names, the envelope under its own.
    #[test]
    fn record_to_tag_uses_the_formats_field_names() {
        let tag = record([1, 2, 3, 4], "mymod:robot").to_tag();
        let Tag::Compound(map) = &tag else {
            panic!("a record writes a compound");
        };
        for field in [
            format::TYPE,
            format::POSITION,
            format::VELOCITY,
            format::ROTATION,
            format::UUID,
            format::PAYLOAD_VERSION,
            format::PAYLOAD,
        ] {
            let key = sunrise_core::nbt::JavaString::from(field);
            assert!(map.contains_key(&key), "{field} missing");
        }
        // A lone record writes no passenger list, as vanilla's save does not.
        assert!(!map.contains_key(&sunrise_core::nbt::JavaString::from(format::PASSENGERS)));
        assert_eq!(
            tag.get(format::TYPE),
            Some(&Tag::String("mymod:robot".into()))
        );
    }

    /// A record whose own data nests one level past the bound refuses with
    /// the nesting fault, and one level inside it stands; the passenger
    /// wrappers themselves never count toward this bound, so a tree at the
    /// long passenger chain still parses.
    #[test]
    fn record_refuses_data_nested_past_the_bound() {
        let deeply = |levels: u32| {
            let mut tag = record([1, 2, 3, 4], "mymod:robot").to_tag();
            let mut nested = Tag::Compound(Default::default());
            for _ in 1..levels {
                let mut next = Tag::Compound(Default::default());
                if let Tag::Compound(map) = &mut next {
                    map.insert("inner".into(), nested);
                }
                nested = next;
            }
            if let Tag::Compound(map) = &mut tag {
                map.insert("Extra".into(), nested);
            }
            tag
        };
        // The root compound is the first level: an `Extra` field nested to
        // the bound makes the record's data one level past it.
        assert_eq!(
            record_from_compound(&deeply(limits::MAX_NESTING)),
            Err(GeneralRecordError::NestingTooDeep {
                depth: limits::MAX_NESTING + 1
            })
        );
        assert!(record_from_compound(&deeply(limits::MAX_NESTING - 1)).is_ok());
        // A passenger chain is not data nesting.
        let mut root = record([1, 2, 3, 4], "mymod:robot");
        let mut node = &mut root;
        for index in 0..16 {
            node.passengers
                .push(record([index + 5, 0, 0, 0], "mymod:drone"));
            node = node.passengers.last_mut().unwrap();
        }
        assert_eq!(
            record_from_compound(&root.to_tag())
                .unwrap()
                .passengers
                .len(),
            1
        );
    }

    /// A field of the wrong shape refuses the whole record as a malformed
    /// envelope, naming the field: no defaulting, no partial read.
    #[test]
    fn record_refuses_malformed_envelope_fields() {
        let base = || record([1, 2, 3, 4], "mymod:robot").to_tag();
        let malformed = |field: &'static str, value: Tag| {
            let mut tag = base();
            if let Tag::Compound(map) = &mut tag {
                map.insert(field.into(), value);
            }
            tag
        };
        for (field, value, expected) in [
            (
                format::PAYLOAD,
                Tag::Int(1),
                GeneralRecordError::InvalidEnvelope {
                    field: format::PAYLOAD,
                },
            ),
            (
                format::PAYLOAD_VERSION,
                Tag::Int(1),
                GeneralRecordError::InvalidEnvelope {
                    field: format::PAYLOAD_VERSION,
                },
            ),
            (
                format::UUID,
                Tag::IntArray(vec![1, 2, 3]),
                GeneralRecordError::InvalidEnvelope {
                    field: format::UUID,
                },
            ),
            (
                format::ROTATION,
                Tag::List(vec![Tag::Float(0.0f32.to_bits())]),
                GeneralRecordError::InvalidEnvelope {
                    field: format::ROTATION,
                },
            ),
            (
                format::POSITION,
                Tag::List(vec![
                    Tag::Double(0.0f64.to_bits()),
                    Tag::Float(64.0f32.to_bits()),
                    Tag::Double(0.0f64.to_bits()),
                ]),
                GeneralRecordError::InvalidEnvelope {
                    field: format::POSITION,
                },
            ),
            (
                format::PASSENGERS,
                Tag::List(vec![Tag::String("robot".into())]),
                GeneralRecordError::InvalidEnvelope {
                    field: format::PASSENGERS,
                },
            ),
            (
                format::TYPE,
                Tag::Int(1),
                GeneralRecordError::InvalidEnvelope {
                    field: format::TYPE,
                },
            ),
        ] {
            assert_eq!(
                record_from_compound(&malformed(field, value)),
                Err(expected),
                "{field}"
            );
        }
    }

    /// The encoded size the quarantine bound reads: a payload's bytes count
    /// into it one for one, so the gate can tell a record at the bound from
    /// one byte past it.
    #[test]
    fn nbt_size_counts_payload_bytes_one_for_one() {
        let at = GeneralEntityRecord {
            envelope: GeneralTypeEnvelope {
                payload_bytes: vec![0; 1024],
                ..record([1, 2, 3, 4], "mymod:robot").envelope
            },
            ..record([1, 2, 3, 4], "mymod:robot")
        };
        let over = GeneralEntityRecord {
            envelope: GeneralTypeEnvelope {
                payload_bytes: vec![0; 1025],
                ..at.envelope.clone()
            },
            ..at.clone()
        };
        assert_eq!(
            nbt_size(&over.to_tag()) - nbt_size(&at.to_tag()),
            1,
            "one payload byte must add one encoded byte"
        );
    }
}
