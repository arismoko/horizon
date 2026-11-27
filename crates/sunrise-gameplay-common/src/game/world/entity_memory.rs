use crate::game::{boats_common::Boat, commands_common::Target, minecarts::Minecart};
use std::collections::{HashMap, HashSet};
use sunrise_core::{geometry::Vec3, nbt::Tag};
use sunrise_gameplay_common::entity_uuids::EntityUuids;
use sunrise_sim::mobs::Mob;

/// An entity object retained by passenger links but refused level residency.
/// Queries and tracking never visit this store; internal topology and saving do.
/// A boat or minecart is only ever a root here: mobs are the only modeled riders.
pub enum OffLevel {
    Mob(Mob),
    Boat(Boat),
    Cart(Minecart),
}

/// Formerly resident vehicles kept solely by surviving passenger edges.
pub enum RemovedVehicle {
    Mob { position: Vec3, kind: String },
    Boat(Boat),
    Cart(Minecart),
}

impl RemovedVehicle {
    pub fn position(&self) -> Vec3 {
        match self {
            Self::Mob { position, .. } => *position,
            Self::Boat(boat) => boat.position,
            Self::Cart(cart) => cart.position,
        }
    }

    pub fn retire_attachment_registration(&self) {
        match self {
            Self::Mob { .. } => {}
            Self::Boat(boat) => {
                boat.attachment_lifetime.retire();
            }
            Self::Cart(cart) => {
                cart.attachment_lifetime.retire();
            }
        }
    }

    pub fn kind(&self) -> String {
        match self {
            Self::Mob { kind, .. } => kind.clone(),
            Self::Boat(boat) => boat.entity_type(),
            Self::Cart(cart) => cart.kind.id().into(),
        }
    }
}

impl OffLevel {
    pub fn as_mob(&self) -> Option<&Mob> {
        match self {
            Self::Mob(mob) => Some(mob),
            Self::Boat(_) | Self::Cart(_) => None,
        }
    }

    pub fn as_mob_mut(&mut self) -> Option<&mut Mob> {
        match self {
            Self::Mob(mob) => Some(mob),
            Self::Boat(_) | Self::Cart(_) => None,
        }
    }

    pub fn as_boat(&self) -> Option<&Boat> {
        match self {
            Self::Boat(boat) => Some(boat),
            Self::Mob(_) | Self::Cart(_) => None,
        }
    }

    pub fn as_cart(&self) -> Option<&Minecart> {
        match self {
            Self::Cart(cart) => Some(cart),
            Self::Mob(_) | Self::Boat(_) => None,
        }
    }

    /// Only a retained mob can be removed while dependents keep it: a boat or
    /// cart is never selectable, so it leaves only with its passenger graph.
    pub fn is_removed(&self) -> bool {
        match self {
            Self::Mob(mob) => mob.removed,
            Self::Boat(_) | Self::Cart(_) => false,
        }
    }
}

/// What Sunrise remembers about loaded entities to save them faithfully.
#[derive(Default)]
pub struct EntityMemory {
    /// Runtime-only opt-outs from saving the entity as a root.
    pub transient: HashSet<u32>,
    /// UUIDs of entities by Sunrise entity ID: the one each was loaded with,
    /// or made up once something needed it (a save, a native mod's attachment).
    pub uuids: EntityUuids,
    /// The compound each mob or item entity was loaded with.
    pub raw: HashMap<u32, Tag>,
    /// Full rejected passenger objects, not resident entities or frozen NBT copies.
    /// Kept with the level while any live object graph still refers to them.
    pub off_level: HashMap<u32, OffLevel>,
    /// Formerly resident vehicles kept for vetoed dismounts, never for ticking.
    pub removed_vehicles: HashMap<u32, RemovedVehicle>,
    /// Destruction callbacks already consumed for geometry retained after removal.
    pub score_destroyed: HashSet<u32>,
    /// Mount notifications currently on the callback stack, keyed by object IDs.
    pub mount_notifications: HashSet<(u32, u32, bool)>,
    /// Deferred native removals already notified their passenger edges.
    pub notified_vehicle_removals: HashSet<u32>,
    /// Pre-removal effects in progress. The vehicle still resolves during these effects.
    pub preparing_vehicle_removals: HashSet<Target>,
    /// Vehicle destruction is logically visible before its passenger callbacks.
    pub destroying_vehicles: HashSet<Target>,
    /// Bodies that set `Entity.syncPosition` this tick, cleared after tracking.
    pub sync_body_position: HashSet<u32>,
    /// Synched entity-data entries dirtied by forced writes since the last sync.
    pub forced_meta: HashMap<u32, [u64; 4]>,
    /// `Entity.noPhysics` a native mod wrote for a kind whose own struct keeps
    /// no field for it (the dragon, an area-effect cloud, an ominous item
    /// spawner, a shulker bullet): the value replaces the kind's constructor
    /// default and is never saved.
    pub no_physics: HashMap<u32, bool>,
    /// Passengers that ride a vehicle, in load order: live entity IDs or unspawned tags.
    /// Entities of types Sunrise does not simulate, kept per chunk and
    /// written back unchanged.
    pub unknown: HashMap<(i32, i32), Vec<Tag>>,
    /// Entity sections loaded before their terrain for a save. They stay owned
    /// until unload, even when empty, so the next save can clear a departed row.
    pub hidden_entity_sections: HashSet<(i32, i32)>,
    /// The columns whose old entity record the last `prepare_entity_save` could not read.
    /// Their records stay untouched and the save as a whole is incomplete.
    pub unreadable_entity_sections: usize,
    /// The compound of orbs, arrows, falling blocks, tridents, crystals and the
    /// dragon as they were loaded, found again by type and position (one that
    /// has moved since loading is saved afresh); their UUIDs are in `uuids`.
    pub anon: HashMap<(String, [u64; 3]), (Vec3, Tag)>,
    /// UUIDs of the vehicles `restore_saved_vehicle` brought in from the
    /// player's file: their chunk copies must not spawn a second one. Cleared
    /// with the rest when a world closes.
    pub restored: HashSet<[i32; 4]>,
    /// Entities a native mod kept out of the level (`native_entity_join_level`),
    /// counted so `/summon` can fail as vanilla's refused add does.
    pub joins_canceled: u32,
    /// The latest insertion considered for `native_entity_join_level`.
    pub joined_seq: sunrise_sim::join_seq::JoinSeq,
    /// Joins admitted synchronously (`admit_fresh_join`) whose `JoinSeq` is
    /// still ahead of `joined_seq`: kept independently of `live` so a later
    /// flush never re-announces an entity that already joined and left.
    pub admitted_joins: Vec<(sunrise_sim::join_seq::JoinSeq, u32)>,
    /// The entities mods were told of and that have not left since: id (in
    /// increasing order), type, and whether the last leave sweep saw it.
    /// Only filled while a mod is loaded.
    pub live: Vec<(u32, String, bool)>,
    /// Entities a native mod removed (`entity_discard`): gone for every
    /// lookup from the call on, taken out of their lists at the next sweep of
    /// departed entities. Mobs go at once and never wait here.
    pub removals: Vec<Target>,
    /// First attachment persistence error raised while building the current entity record.
    pub attachment_encode_error: Option<sunrise_mod_runtime::attachments::AttachmentEncodeError>,
    pub dragon_death_canceled: Option<HashSet<[i32; 4]>>,
    pub entity_metadata_on_disk: bool,
}

impl EntityMemory {
    /// Move object-owned save and dirty state, not either level's residency ledger.
    /// The source still emits its leave and the destination its join.
    pub fn move_object_to(&mut self, id: u32, destination: &mut Self) {
        if let Some(uuid) = self.uuids.remove(&id) {
            destination.uuids.insert(id, uuid);
        }
        if let Some(raw) = self.raw.remove(&id) {
            destination.raw.insert(id, raw);
        }
        if self.transient.remove(&id) {
            destination.transient.insert(id);
        }
        if let Some(forced) = self.forced_meta.remove(&id) {
            destination.forced_meta.insert(id, forced);
        }
        if self.score_destroyed.remove(&id) {
            destination.score_destroyed.insert(id);
        }
    }
}

impl EntityMemory {
    pub fn force_meta(&mut self, id: u32, index: u8) {
        self.forced_meta.entry(id).or_default()[usize::from(index) / 64] |= 1 << (index % 64);
    }
}
impl EntityMemory {
    /// The UUID entity `id` was loaded or saved with: what a save writes for a
    /// field naming that entity (`entity_nbt::mob_to_nbt`'s `uuid_of`). `None`
    /// for one that has none yet, so no reference is written rather than a
    /// made-up UUID nothing would answer for.
    pub fn uuid_of(&self, id: u32) -> Option<[i32; 4]> {
        self.uuids.get(&id).copied()
    }
}

impl EntityMemory {
    /// Puts back the raw record `kept` holds, or takes out the one a spawn added where none was:
    /// the abort of a transaction whose spawn wrote over it (`journal_anon`).
    pub fn put_back_anon(&mut self, (key, was): ((String, [u64; 3]), Option<(Vec3, Tag)>)) {
        match was {
            Some(record) => self.anon.insert(key, record),
            None => self.anon.remove(&key),
        };
    }
}

impl crate::game::CommonGame<'_> {
    /// The compound an entity was loaded with (and commands have edited).
    pub fn persist_raw(&self, id: u32) -> Option<&Tag> {
        self.persist.raw.get(&id)
    }
}
impl crate::game::CommonGame<'_> {
    /// Internal object lookup, including a passenger refused level residency.
    /// This is deliberately separate from `level_entity` and native handle queries.
    pub fn mob_object(&self, id: u32) -> Option<&Mob> {
        self.mobs
            .iter()
            .find(|mob| mob.id == id)
            .or_else(|| self.persist.off_level.get(&id).and_then(OffLevel::as_mob))
    }
}
