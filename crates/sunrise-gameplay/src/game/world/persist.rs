//! Saving and loading the game state that lives outside chunk blocks:
//! entities (`entities/*.mca`, per chunk like vanilla's `EntityStorage`) and
//! scheduled block/fluid ticks (chunk `block_ticks`/`fluid_ticks`).
use crate::game::ServerGame;
use sunrise_gameplay_common::game::GameGameExt3;
#[path = "persist/display.rs"]
pub mod display;
#[path = "persist/loaded_pass.rs"]
mod loaded_pass;
pub(in crate::game) use loaded_pass::StagedEntityTree;
#[path = "persist/object_memory.rs"]
mod object_memory;
#[path = "persist/spawn_tree.rs"]
mod spawn_tree;
pub(in crate::game) use spawn_tree::SpawnTree;
#[path = "persist/hurt_by_refs.rs"]
mod hurt_by_refs;
#[path = "persist/unload_sweep.rs"]
mod unload_sweep;

use super::FallingBlock;
use {
    crate::game::server_side::potions_server::potion_cloud_to_nbt,
    sunrise_gameplay_common::game::end_state::SummonedEye,
    sunrise_gameplay_common::game::entity_memory::EntityMemory,
    sunrise_gameplay_common::game::entity_memory::OffLevel,
    sunrise_gameplay_common::game::interactions_common::InteractionState,
    sunrise_gameplay_common::game::interactions_common::is_interaction,
    sunrise_gameplay_common::game::piston_nbt::block_entity_pos,
};

/// Levels of NBT nesting kept clear under the writer's guard for what surrounds a passenger
/// chain: the chunk root, its `Entities` list, the root entity and its own data.
const SAVE_NESTING_MARGIN: usize = 64;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use sunrise_core::{
    geometry::{Direction, Vec3},
    position::{BlockPos, ChunkPos},
    random::RandomSource,
    ticks::ChunkTicks,
};
use sunrise_game_data::{Result, scoreboard::Scoreboard};
use sunrise_level::{chunk::Chunk, random_source};
use sunrise_mod_runtime::attachments::{ATTACHMENTS_KEY, attachment_compound};
use sunrise_sim::{
    entity_nbt,
    mobs::{Mob, Who},
    save::{ChunkExtras, random_uuid, text, uuid_from_text, uuid_text},
};
use {
    sunrise_core::nbt::Tag,
    sunrise_world::{level::MovingPiston, registry::BlockStateId},
};

/// The UUID an entity loads with: the saved one, else the one its constructor made.
/// `Entity.<init>` always draws it from the entity's own random, so the draw is taken
/// whether or not the tag has a UUID (as `load_item_transferred` and `load_mob` do).
fn with_constructor_uuid(saved: Option<[i32; 4]>) -> Option<[i32; 4]> {
    let made = sunrise_sim::save::insecure_uuid(&mut random_source::entity());
    Some(saved.unwrap_or(made))
}
/// `Entity.saveWithoutId`: materialize the mandatory fields of an entity
/// whose simulation state is still represented by its loaded compound.
pub(in crate::game::server_side) fn write_static_entity_fields(tag: &mut Tag) {
    use sunrise_game_data::nbt_path::int_value;
    use sunrise_sim::save::float;

    let int = |key, default| tag.get(key).and_then(int_value).unwrap_or(default);
    let motion = entity_nbt::loaded_motion(tag);
    let fall_distance = float(tag.get("fall_distance")).unwrap_or(0.0);
    let values = [
        (
            "Motion",
            sunrise_sim::save::doubles(&[motion.x, motion.y, motion.z]),
        ),
        (
            "fall_distance",
            Tag::Double(if fall_distance == 0.0 {
                0
            } else {
                fall_distance.to_bits()
            }),
        ),
        ("Fire", Tag::Short(int("Fire", 0) as i16)),
        ("Air", Tag::Short(int("Air", 300) as i16)),
        (
            "OnGround",
            Tag::Byte(i8::from(entity_nbt::flag(tag, "OnGround"))),
        ),
        (
            "Invulnerable",
            Tag::Byte(i8::from(entity_nbt::flag(tag, "Invulnerable"))),
        ),
        ("PortalCooldown", Tag::Int(int("PortalCooldown", 0))),
    ];
    if let Tag::Compound(fields) = tag {
        for (key, value) in values {
            fields.insert(key.into(), value);
        }
    }
}

/// Why the engine is collecting entity tags. Structure capture is not a world save.
#[derive(Clone, Copy)]
pub enum EntitySnapshot {
    Save,
    Unload,
    Structure,
}

/// Runtime ownership paired with the serialized tree out of band. UUIDs are
/// admission identities, not keys for matching these distinct object states.
#[derive(Debug)]
pub struct TransferredVehicleTree {
    source_id: u32,
    kind: VehicleTreeKind,
    pub name: String,
    state: super::entity_initialization::TransferredEntityState,
    passengers: Vec<TransferredVehicleTree>,
}

impl TransferredVehicleTree {
    pub(in crate::game::server_side) fn is_transient(&self) -> bool {
        self.state.transient
    }
}

pub struct RestoredVehicleTree {
    pub root: u32,
    pub by_source: HashMap<u32, u32>,
}

#[derive(Clone, Copy, Debug)]
enum VehicleTreeKind {
    Mob,
    Boat,
    Cart,
    General,
    Stand,
    Other,
}

/// The boundary has paired each modeled NBT node with exactly one captured
/// object state before any destination object or attachment map is installed.
pub struct PreparedVehicleTransfer {
    source_id: u32,
    tag: Tag,
    kind: VehicleTreeKind,
    name: String,
    state: super::entity_initialization::TransferredEntityState,
    passengers: Vec<PreparedVehicleTransfer>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MobObject {
    Resident(usize),
    OffLevel(u32),
}

/// The key that marks a compound queued with the chunk loads as a fresh add
/// (`PersistentEntitySectionManager.addNewEntity`: `/summon`, a portal
/// arrival), not an entity loaded from disk. Taken off before the compound
/// is kept or saved.
const FRESH_ADD: &str = "sunrise:fresh_add";

/// The kinds `load_unlisted` loads, before `load_listed` sees them.
const UNLISTED_KINDS: &[&str] = &[
    "minecraft:area_effect_cloud",
    "minecraft:evoker_fangs",
    "minecraft:ominous_item_spawner",
    "minecraft:ender_dragon",
    "minecraft:end_crystal",
    "minecraft:item_frame",
    "minecraft:glow_item_frame",
    "minecraft:cushion",
    "minecraft:painting",
    "minecraft:lightning_bolt",
];

/// Queue `tag` with the chunk loads as a fresh add.
pub fn mark_fresh(tag: &mut Tag) {
    if let Tag::Compound(c) = tag {
        c.insert(FRESH_ADD.into(), Tag::Byte(1));
    }
}

/// The key that marks a compound queued with the chunk loads as an entity
/// moving within its level (an End gateway: `Entity.teleport` takes
/// `teleportSameDimension`, which moves the entity without removing it), so
/// it doesn't join the level again. Taken off like [`FRESH_ADD`].
const SAME_LEVEL_MOVE: &str = "sunrise:same_level_move";

/// `PrimedTnt.readAdditionalSaveData`: `getShortOr("fuse", 80)` accepts any
/// numeric tag; floating values floor to int before the short narrowing.
pub(in crate::game::server_side) fn tnt_fuse(tag: &Tag) -> i32 {
    i32::from(match tag.get("fuse") {
        Some(Tag::Float(v)) => f32::from_bits(*v).floor() as i32 as i16,
        Some(Tag::Double(v)) => f64::from_bits(*v).floor() as i32 as i16,
        other => sunrise_sim::save::number(other).unwrap_or(80) as i16,
    })
}

/// Whether `tag` was queued as a move within its level; the mark comes off.
fn take_moved(tag: &mut Tag) -> bool {
    match tag {
        Tag::Compound(c) => c.remove(&SAME_LEVEL_MOVE.into()).is_some(),
        _ => false,
    }
}

/// Whether `tag` was queued as a fresh add; the mark comes off.
pub fn take_fresh(tag: &mut Tag) -> bool {
    match tag {
        Tag::Compound(c) => c.remove(&FRESH_ADD.into()).is_some(),
        _ => false,
    }
}

fn anon_key(kind: &str, p: Vec3) -> (String, [u64; 3]) {
    (
        kind.to_owned(),
        [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()],
    )
}
/// The loaded compound underneath the fields Sunrise models: what the entity was
/// loaded with, with every key of `modelled` replacing its loaded value.
pub(in crate::game::server_side) fn with_raw(modelled: Tag, raw: Option<&Tag>) -> Tag {
    with_raw_over(modelled, raw, &[])
}
/// The orb saves ticked Fire and stored (not ticked) Air over the loaded compound.
pub const ORB_LIVE: &[&str] = &["Fire", "Air"];
/// [`with_raw`] for a writer that simulates some of the fields Sunrise
/// otherwise invents: the keys in `live` keep the writer's value.
pub fn with_raw_over(mut modelled: Tag, raw: Option<&Tag>, live: &[&str]) -> Tag {
    if let (Some(Tag::Compound(raw)), Tag::Compound(fields)) = (raw, &modelled) {
        let mut out = raw.clone();
        for (key, value) in fields {
            // Defaults Sunrise invents for fields it does not simulate give way to the save.
            let key_text = key.to_string_lossy();
            let invented = matches!(
                key_text.as_ref(),
                "Fire" | "Air" | "Invulnerable" | "PortalCooldown" | "HasBeenShot"
            ) && !live.contains(&key_text.as_ref());
            if !(invented && out.contains_key(key)) {
                out.insert(key.clone(), value.clone());
            }
        }
        modelled = Tag::Compound(out);
    }
    // The overlay may restore raw bytes for fields whose false value the
    // live writer omitted. Entity.saveWithoutId always writes booleans.
    let on_ground = entity_nbt::flag(&modelled, "OnGround");
    let no_gravity = entity_nbt::flag(&modelled, "NoGravity");
    entity_nbt::write_saved_flags(&mut modelled, on_ground, no_gravity);
    modelled
}

pub fn read_attachments(
    tag: &Tag,
    id: u32,
    maps: &mut HashMap<
        u32,
        sunrise_mod_runtime::attachments::Attachments,
        impl std::hash::BuildHasher,
    >,
    live: bool,
) {
    if let Ok(Some(data)) = attachment_compound(tag.get(ATTACHMENTS_KEY)) {
        match maps.get_mut(&id) {
            // Made by `entity_constructing` listeners just now: saved values win.
            Some(map) if !live => map.read_saved_over(data.clone()),
            Some(_) => {}
            None => {
                maps.insert(
                    id,
                    sunrise_mod_runtime::attachments::Attachments::from_nbt(data.clone()),
                );
            }
        }
    }
}

/// Overlay one modeled object's contents. Raw passengers retain their own payload.
pub fn write_attachments(
    tag: &mut Tag,
    id: u32,
    maps: &sunrise_gameplay_common::game::native_collider_state::EntityMaps,
) -> std::result::Result<(), sunrise_mod_runtime::attachments::AttachmentEncodeError> {
    if let Tag::Compound(root) = tag {
        maps.write_collider(root, id);
        if let Some(map) = maps.get(&id)
            && attachment_compound(root.get(&ATTACHMENTS_KEY.into())).is_ok()
        {
            let data = map.try_to_nbt()?;
            if data.is_empty() {
                root.remove(&ATTACHMENTS_KEY.into());
            } else {
                root.insert(ATTACHMENTS_KEY.into(), Tag::Compound(data));
            }
        }
    }
    Ok(())
}

fn painting_center(p: &sunrise_sim::entities::Painting) -> Vec3 {
    let (lo, hi) = p.bounds();
    Vec3::new(
        (lo[0] + hi[0]) / 2.0,
        (lo[1] + hi[1]) / 2.0,
        (lo[2] + hi[2]) / 2.0,
    )
}
/// `EntitySectionStorage.getExistingSectionsInChunk`: a chunk's sections in
/// packed `SectionPos` order, whose low 20 bits are the section Y: section 0
/// upward, then the negative sections. A record without a position goes last.
fn section_order(tag: &Tag) -> u32 {
    entity_nbt::position(tag).map_or(u32::MAX, |at| {
        ((at.y.floor() as i32 >> 4) as u32) & 0xF_FFFF
    })
}

fn chunk_of(p: Vec3) -> (i32, i32) {
    ((p.x.floor() as i32) >> 4, (p.z.floor() as i32) >> 4)
}

impl crate::game::ServerGame<'_> {
    /// Saved ticks and the save time for a chunk about to be written.
    pub fn attach_save_data(&self, chunk: &mut Chunk) {
        let pos = ChunkPos::new(chunk.x, chunk.z);
        let time = self.common.world.game_time;
        let mut extras: ChunkExtras = chunk.saved.as_deref().cloned().unwrap_or_default();
        extras.block_entity_attachments = self
            .common
            .world
            .block_entities
            .chunk_attachments(chunk.x, chunk.z);
        // ComparatorBlockEntity.saveAdditional: the output belongs to the
        // circuit, not the inventory map. Overlay only that field on the raw
        // compound so components and other saved data survive.
        for (p, output) in &self.common.block_ticks.comparator_outputs {
            if (p.x >> 4, p.z >> 4) != (chunk.x, chunk.z)
                || self
                    .common
                    .res
                    .facts()
                    .get(chunk.block(p.x & 15, p.y, p.z & 15))
                    .name
                    .as_ref()
                    != "minecraft:comparator"
            {
                continue;
            }
            let at = [p.x, p.y, p.z];
            let index = extras
                .block_entities
                .iter()
                .position(|t| block_entity_pos(t) == at);
            let tag = match index {
                Some(i) => &mut extras.block_entities[i],
                None => {
                    extras.block_entities.push(Tag::compound([
                        ("id", Tag::String("minecraft:comparator".into())),
                        ("x", Tag::Int(p.x)),
                        ("y", Tag::Int(p.y)),
                        ("z", Tag::Int(p.z)),
                    ]));
                    extras.block_entities.last_mut().unwrap()
                }
            };
            if text(tag.get("id")).as_deref() != Some("minecraft:comparator") {
                *tag = Tag::compound([
                    ("id", Tag::String("minecraft:comparator".into())),
                    ("x", Tag::Int(p.x)),
                    ("y", Tag::Int(p.y)),
                    ("z", Tag::Int(p.z)),
                ]);
            }
            if let Tag::Compound(c) = tag {
                c.insert("OutputSignal".into(), Tag::Int(*output));
            }
        }
        // HopperBlockEntity.saveAdditional: its inventory is modelled, but its
        // transfer phase lives in Game. The snapshot's source overlay preserves
        // the remaining NBT without changing the live source compound.
        for (p, state) in &self.server.hoppers {
            if (p.0 >> 4, p.2 >> 4) != (chunk.x, chunk.z)
                || self
                    .common
                    .res
                    .facts()
                    .get(chunk.block(p.0 & 15, p.1, p.2 & 15))
                    .name
                    .as_ref()
                    != "minecraft:hopper"
            {
                continue;
            }
            let index = extras.block_entity_sources.iter().position(|(q, _)| q == p);
            let tag = match index {
                Some(i) => &mut extras.block_entity_sources[i].1,
                None => {
                    extras.block_entity_sources.push((
                        *p,
                        Tag::compound([
                            ("id", Tag::String("minecraft:hopper".into())),
                            ("x", Tag::Int(p.0)),
                            ("y", Tag::Int(p.1)),
                            ("z", Tag::Int(p.2)),
                        ]),
                    ));
                    &mut extras.block_entity_sources.last_mut().unwrap().1
                }
            };
            if text(tag.get("id")).as_deref() != Some("minecraft:hopper") {
                *tag = Tag::compound([
                    ("id", Tag::String("minecraft:hopper".into())),
                    ("x", Tag::Int(p.0)),
                    ("y", Tag::Int(p.1)),
                    ("z", Tag::Int(p.2)),
                ]);
            }
            if let Tag::Compound(c) = tag {
                c.insert("TransferCooldown".into(), Tag::Int(state.cooldown));
            }
        }
        // A chunk the scheduler hasn't registered yet (one loaded and
        // unloaded again before the next tick) still holds its saved ticks:
        // `register_chunk_ticks` empties that list once it takes them.
        if let Some(t) = self.common.block_ticks.blocks.chunk(pos) {
            extras.block_ticks = t.pack(time);
        }
        if let Some(t) = self.common.block_ticks.fluids.chunk(pos) {
            extras.fluid_ticks = t.pack(time);
        }
        extras.last_update = time;
        // PistonMovingBlockEntity.saveAdditional: a block caught mid-move is
        // saved with its chunk (#3310).
        for (p, piston) in &self.common.block_ticks.moving_pistons {
            if (p.x >> 4, p.z >> 4) != (chunk.x, chunk.z) {
                continue;
            }
            extras
                .block_entities
                .retain(|tag| block_entity_pos(tag) != [p.x, p.y, p.z]);
            match self.common.saved_moving_piston(*p, piston) {
                Ok(tag) => extras.block_entities.push(tag),
                Err(_) => {
                    if let Some(carried) =
                        self.common.world.block_entities.carried(&(p.x, p.y, p.z))
                        && let Some(attachments) = carried.attachments.as_ref()
                    {
                        extras
                            .block_entity_attachments
                            .push(((p.x, p.y, p.z), attachments.clone()));
                    }
                }
            }
        }
        // TheEndGatewayBlockEntity saves its Age (the server copy's).
        for tag in &mut extras.block_entities {
            let pos = [tag.get("x"), tag.get("y"), tag.get("z")]
                .map(|v| sunrise_sim::save::number(v).unwrap_or(0) as i32);
            if let Some(g) = self.server.be_anims.gateway_servers.get(&pos)
                && let Some(age) = tag.get_mut("Age")
            {
                *age = sunrise_core::nbt::Tag::Long(g.age);
            }
        }
        if let Some(t) = self.common.world.inhabited.get(&(chunk.x, chunk.z)) {
            extras
                .root
                .insert("InhabitedTime".into(), sunrise_core::nbt::Tag::Long(*t));
        }
        chunk.saved = Some(Arc::new(extras));
    }
    /// Register a newly resident chunk with the tick scheduler, with the
    /// block and fluid ticks it was saved with (`LevelChunk.registerTickContainerInLevel`
    /// at FULL). They are unpacked when the chunk starts ticking
    /// (`start_ticking_chunks`).
    pub fn register_chunk_ticks(&mut self, key: (i32, i32)) {
        let pos = ChunkPos::new(key.0, key.1);
        let saved = self
            .common
            .world
            .chunks
            .get(&key)
            .and_then(|c| c.saved.clone());
        // Load before the first block-entity tick, including while ticks are
        // frozen. The scheduler's maps are authoritative from this point on.
        if let Some(saved) = saved.as_deref() {
            for tag in &saved.block_entities {
                let p = block_entity_pos(tag);
                if text(tag.get("id")).as_deref() == Some("minecraft:comparator")
                    && self.common.data_block_name(p) == "minecraft:comparator"
                    && let Some(tag) = sunrise_game_data::block_entity_nbt::canonical(tag)
                    && let Some(Tag::Int(output)) = tag.get("OutputSignal")
                {
                    self.common
                        .block_ticks
                        .comparator_outputs
                        .insert(BlockPos::new(p[0], p[1], p[2]), *output);
                }
            }
            for (p, tag) in &saved.block_entity_sources {
                if text(tag.get("id")).as_deref() == Some("minecraft:hopper")
                    && self.common.data_block_name([p.0, p.1, p.2]) == "minecraft:hopper"
                {
                    self.server
                        .hoppers
                        .insert(*p, super::machines::HopperState::load(tag));
                }
            }
        }
        // ChunkAccess.inhabitedTime as saved.
        if let Some(Tag::Long(t)) = saved
            .as_deref()
            .and_then(|s| s.root.get(&"InhabitedTime".into()))
        {
            self.common.world.inhabited.insert(key, *t);
        }
        let (blocks, fluids) = saved.as_deref().map_or((Vec::new(), Vec::new()), |s| {
            (s.block_ticks.clone(), s.fluid_ticks.clone())
        });
        let loaded = !blocks.is_empty() || !fluids.is_empty();
        if self
            .common
            .block_ticks
            .add_chunk(
                pos,
                ChunkTicks::from_saved(blocks),
                ChunkTicks::from_saved(fluids),
            )
            .is_ok()
            && loaded
        {
            self.server.tick_chunks.wait(key);
        }
        // The saved list is consumed: later saves take the scheduler's ticks.
        if loaded && let Some(c) = self.common.world.chunks.get_mut(&key) {
            let c = Arc::make_mut(c);
            if let Some(s) = c.saved.as_mut() {
                let s = Arc::make_mut(s);
                s.block_ticks.clear();
                s.fluid_ticks.clear();
            }
        }
        // PistonMovingBlockEntity.loadAdditional: blocks saved mid-move carry
        // on moving; later saves take them from the level (#3310).
        let pistons: Vec<_> = saved.as_deref().map_or_else(Vec::new, |s| {
            s.block_entities
                .iter()
                .filter_map(|tag| self.moving_piston_from_nbt(tag))
                .collect()
        });
        if pistons.is_empty() {
            return;
        }
        for &(pos, piston) in &pistons {
            self.common
                .restore_moving_piston(pos, piston, saved.as_deref());
        }
        if let Some(c) = self.common.world.chunks.get_mut(&key) {
            let c = Arc::make_mut(c);
            if let Some(s) = c.saved.as_mut() {
                Arc::make_mut(s).block_entities.retain(|tag| {
                    let at = block_entity_pos(tag);
                    !pistons.iter().any(|(p, _)| [p.x, p.y, p.z] == at)
                });
            }
        }
    }
    /// `PistonMovingBlockEntity.loadAdditional`, for a `minecraft:piston`
    /// entity whose block is still a moving piston.
    fn moving_piston_from_nbt(&self, tag: &Tag) -> Option<(BlockPos, MovingPiston)> {
        use sunrise_sim::save::{float, number, text};
        if text(tag.get("id")).as_deref() != Some("minecraft:piston") {
            return None;
        }
        let [x, y, z] = block_entity_pos(tag);
        let here = self.common.world.block(x, y, z);
        if &*self.common.res.facts().get(here).name != "minecraft:moving_piston" {
            return None;
        }
        let moved_state = tag
            .get("blockState")
            .and_then(|t| self.state_from_nbt(t))
            .or_else(|| {
                self.common
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:air")
            })?;
        let direction = usize::try_from(number(tag.get("facing")).unwrap_or(0))
            .ok()
            .and_then(|i| Direction::ALL.get(i).copied())
            .unwrap_or(Direction::Down);
        let progress = float(tag.get("progress")).unwrap_or(0.0) as f32;
        let flag = |k: &str| number(tag.get(k)).unwrap_or(0) != 0;
        Some((
            BlockPos::new(x, y, z),
            MovingPiston {
                moved_state,
                direction,
                extending: flag("extending"),
                source: flag("source"),
                progress,
                progress_o: progress,
                last_ticked: 0,
                death_ticks: 0,
                id: 0,
            },
        ))
    }
    /// The kinds `entity_lists!` does not hold: `Err` hands the compound back
    /// when `kind` is not one of them.
    pub fn load_unlisted(
        &mut self,
        key: ChunkKey,
        kind: &str,
        uuid: Option<[i32; 4]>,
        tag: Tag,
    ) -> std::result::Result<(), Tag> {
        self.load_unlisted_transferred(key, kind, uuid, tag, &mut None)
    }
    pub(in crate::game::server_side) fn load_unlisted_transferred(
        &mut self,
        key: ChunkKey,
        kind: &str,
        uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<super::entity_initialization::TransferredEntityState>,
    ) -> std::result::Result<(), Tag> {
        if !UNLISTED_KINDS.contains(&kind) {
            return Err(tag);
        }
        match kind {
            "minecraft:area_effect_cloud" => {
                let id = if let Some(state) = transferred.take() {
                    self.load_potion_cloud_transferred(&tag, Some(state))
                } else {
                    self.load_potion_cloud(&tag)
                };
                if let Some(id) = id {
                    let uuid = with_constructor_uuid(uuid);
                    self.remember(id, uuid, tag);
                }
            }
            "minecraft:evoker_fangs" => {
                if let Some(at) = entity_nbt::position(&tag) {
                    self.server.next_entity += 1;
                    let id = self.server.next_entity;
                    let delay = sunrise_sim::save::number(tag.get("Warmup")).unwrap_or(0) as i32;
                    let owner = self.loaded_fangs_owner(&tag);
                    if let Some(state) = transferred.take() {
                        self.adopt_transferred_entity_state(id, state);
                    }
                    self.common.insert_fangs(
                        id,
                        [at.x, at.y, at.z],
                        entity_nbt::loaded_rotation(&tag)[0],
                        delay,
                        owner,
                    );
                    let last = self.common.fangs.list.len() - 1;
                    let motion = entity_nbt::loaded_motion(&tag);
                    let fire = entity_nbt::fire_ticks_from_nbt(&tag);
                    let has_visual_fire = entity_nbt::flag(&tag, "HasVisualFire");
                    self.common.fangs.list.update(last, |f| {
                        f.motion = motion;
                        f.fire_ticks = fire;
                        f.has_visual_fire = has_visual_fire;
                    });
                    let uuid = with_constructor_uuid(uuid);
                    self.remember(id, uuid, tag);
                }
            }
            "minecraft:ominous_item_spawner" => {
                if let Some(state) = transferred.take() {
                    self.load_ominous_item_spawner_transferred(tag, Some(state));
                } else {
                    self.load_ominous_item_spawner(tag);
                }
            }
            "minecraft:ender_dragon" => {
                if let Some(state) = transferred.take() {
                    self.load_dragon_transferred(tag, uuid, Some(state), false);
                } else {
                    self.load_dragon(tag, uuid);
                }
            }
            "minecraft:end_crystal" => {
                if let Some(state) = transferred.take() {
                    self.load_crystal_transferred(tag, uuid, Some(state));
                } else {
                    self.load_crystal(tag, uuid);
                }
            }
            "minecraft:item_frame" | "minecraft:glow_item_frame" => {
                if let Some(state) = transferred.take() {
                    self.load_item_frame_transferred(tag, uuid, Some(state));
                } else {
                    self.load_item_frame(tag, uuid);
                }
            }
            "minecraft:cushion" => {
                if let Some(state) = transferred.take() {
                    self.load_cushion_transferred(tag, uuid, Some(state));
                } else {
                    self.load_cushion(tag, uuid);
                }
            }
            "minecraft:painting" => {
                if let Some(state) = transferred.take() {
                    self.load_painting_transferred(tag, uuid, key, Some(state));
                } else {
                    self.load_painting(tag, uuid, key);
                }
            }
            "minecraft:lightning_bolt" => {
                let _ = self.load_bolt(tag, None, false);
            }
            _ => return Err(tag),
        }
        Ok(())
    }
    /// A saved dropped item.
    pub fn load_item(&mut self, tag: Tag, uuid: Option<[i32; 4]>) {
        self.load_item_transferred(tag, uuid, None);
    }

    /// A saved experience orb (or the orbs it stands for).
    pub fn load_orb(&mut self, tag: Tag, uuid: Option<[i32; 4]>) {
        self.load_orb_transferred(tag, uuid, None);
    }

    /// A saved arrow or spectral arrow.
    pub fn load_arrow(&mut self, tag: Tag, uuid: Option<[i32; 4]>, player_uuid: Option<[i32; 4]>) {
        self.load_arrow_transferred(tag, uuid, player_uuid, None);
    }

    /// A saved llama spit or shulker bullet.
    pub(in crate::game::server_side) fn load_spit_or_bullet(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
    ) {
        self.load_spit_or_bullet_transferred(tag, uuid, player_uuid, None);
    }

    /// `ShulkerBullet.readAdditionalSaveData`'s `Target`, resolved over the UUID
    /// bridge: the `Who` of the loaded player or mob that UUID names, or `None`
    /// for one that is not here. Vanilla keeps the UUID and re-resolves it every
    /// tick; a `Who` can only name an entity in the level, so a target that is
    /// away leaves the bullet with none and it falls (`ShulkerBullet.tick`).
    fn bullet_target(&self, tag: &Tag) -> Option<Who> {
        self.who_by_uuid(entity_nbt::bullet_target(tag)?)
    }
    /// A saved fireball or wind charge.
    pub fn load_fireball(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
    ) {
        self.load_fireball_transferred(tag, uuid, player_uuid, None);
    }

    /// A saved thrown item.
    pub fn load_throwable(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
    ) {
        self.load_throwable_transferred(tag, uuid, player_uuid, None);
    }

    /// A saved falling block.
    pub(in crate::game::server_side) fn load_falling(&mut self, tag: Tag, uuid: Option<[i32; 4]>) {
        self.load_falling_transferred(tag, uuid, None);
    }

    /// A saved thrown trident.
    pub fn load_trident(&mut self, tag: Tag, uuid: Option<[i32; 4]>) {
        self.load_trident_transferred(tag, uuid, None);
    }

    /// A saved firework rocket.
    pub(in crate::game::server_side) fn load_rocket(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        player_uuid: Option<[i32; 4]>,
    ) {
        self.load_rocket_transferred(tag, uuid, player_uuid, None);
    }

    /// `PrimedTnt.readAdditionalSaveData`: its numeric short fuse (80 when absent) and motion.
    // Its Passengers ride it (EntityType.loadEntityRecursive forces them on).
    pub fn load_tnt(&mut self, tag: Tag, uuid: Option<[i32; 4]>) {
        self.load_tnt_transferred(tag, uuid, None);
    }

    /// A saved boat.
    pub fn load_boat(&mut self, tag: Tag, uuid: Option<[i32; 4]>, k: &str) {
        self.load_boat_transferred(tag, uuid, k, None);
    }

    /// A saved minecart.
    pub fn load_cart(&mut self, tag: Tag, uuid: Option<[i32; 4]>, k: &str) {
        self.load_cart_transferred(tag, uuid, k, None);
    }

    /// A saved armor stand.
    pub fn load_armor_stand(&mut self, tag: Tag, uuid: Option<[i32; 4]>) {
        self.load_armor_stand_transferred(tag, uuid, None)
    }
    pub(in crate::game::server_side) fn load_armor_stand_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        transferred: Option<super::entity_initialization::TransferredEntityState>,
    ) {
        self.server.next_entity += 1;
        if let Some(mut a) = entity_nbt::armor_stand_from_nbt(&tag, self.server.next_entity) {
            let mut tag = tag;
            let riders = self.take_passengers(&mut tag);
            a.live
                .retain_supplier(&self.common.motion.attributes, "minecraft:armor_stand");
            if let Some(state) = transferred {
                self.adopt_transferred_entity_state(a.id, state);
            }
            // Entity.<init>: its own random's first draws, saved UUID or not.
            let uuid = with_constructor_uuid(uuid);
            let id = a.id;
            self.remember(id, uuid, tag);
            self.bind_saved_entity_attachments(id, &mut a.attachment_lifetime);
            self.common.armor_stands.push(a);
            self.load_passengers(id, riders);
        }
    }

    /// The saved ender dragon.
    pub fn load_dragon(&mut self, tag: Tag, uuid: Option<[i32; 4]>) {
        self.load_dragon_transferred(tag, uuid, None, true)
    }
    pub(in crate::game::server_side) fn load_dragon_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        transferred: Option<super::entity_initialization::TransferredEntityState>,
        from_disk: bool,
    ) {
        // The loaded dragon's own random (`Entity.random`): loading draws
        // nothing from the level's.
        let seed = random_source::unique_seed();
        if self.common.dragon.is_none()
            && let Some(mut d) =
                sunrise_gameplay_common::game::dragon_common::Dragon::from_nbt(&tag, seed)
        {
            // EnderDragon's constructor takes its id and its
            // parts' from the entity counter.
            d.live
                .retain_supplier(&self.common.motion.attributes, "minecraft:ender_dragon");
            d.id = self.server.next_entity + 1;
            self.server.next_entity += 9;
            if let Some(state) = transferred {
                self.adopt_transferred_entity_state(d.id, state);
            }
            let uuid = with_constructor_uuid(uuid);
            if from_disk && let Some(saved_uuid) = uuid.or_else(|| entity_nbt::uuid(&tag)) {
                if self.common.persist.dragon_death_canceled.is_none() {
                    let (set, on_disk) = self.server.save.0.load_dragon_death_canceled();
                    self.common.persist.dragon_death_canceled = Some(set);
                    self.common.persist.entity_metadata_on_disk = on_disk;
                }
                if self
                    .common
                    .persist
                    .dragon_death_canceled
                    .as_mut()
                    .is_some_and(|set| set.remove(&saved_uuid))
                {
                    d.death_canceled = true;
                }
            }
            self.remember_tags(d.id, &tag);
            self.remember_uuid(d.id, uuid, &tag);
            d.join = sunrise_sim::join_seq::JoinSeq::next();
            assert!(self.dragon.install(d).is_ok());
        }
    }

    /// A saved end crystal.
    pub(in crate::game::server_side) fn load_crystal(&mut self, tag: Tag, uuid: Option<[i32; 4]>) {
        self.load_crystal_transferred(tag, uuid, None)
    }
    pub(in crate::game::server_side) fn load_crystal_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        transferred: Option<super::entity_initialization::TransferredEntityState>,
    ) {
        // The constructor's draws; the saved UUID replaces the one it drew.
        let (drawn, _) = sunrise_gameplay_common::game::dragon_common::Crystal::own_draws();
        if let Some(mut c) =
            sunrise_gameplay_common::game::dragon_common::Crystal::from_nbt(&tag, drawn)
        {
            c.time = sunrise_gameplay_common::game::dragon_common::Crystal::start_time(
                c.position,
                drawn,
                self.common.clock.still(),
            );
            // EndCrystal's constructor takes an id from the
            // entity counter.
            self.server.next_entity += 1;
            c.id = self.server.next_entity;
            if let Some(state) = transferred {
                self.adopt_transferred_entity_state(c.id, state);
            }
            self.remember(c.id, uuid, tag);
            self.common.crystals.push(c);
        }
    }

    /// A saved item frame.
    pub fn load_item_frame(&mut self, tag: Tag, uuid: Option<[i32; 4]>) {
        self.load_item_frame_transferred(tag, uuid, None)
    }
    pub(in crate::game::server_side) fn load_item_frame_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        transferred: Option<super::entity_initialization::TransferredEntityState>,
    ) {
        self.server.next_entity += 1;
        if let Some(mut f) = entity_nbt::item_frame_from_nbt(&tag, self.server.next_entity) {
            let item = f.item.take();
            f.set_item(item);
            if let Some(state) = transferred {
                self.adopt_transferred_entity_state(f.id, state);
            }
            self.remember(f.id, uuid, tag);
            self.bind_saved_entity_attachments(f.id, &mut f.attachment_lifetime);
            self.common.item_frames.push(f);
        }
    }

    /// A saved cushion.
    pub fn load_cushion(&mut self, tag: Tag, uuid: Option<[i32; 4]>) {
        self.load_cushion_transferred(tag, uuid, None)
    }
    pub(in crate::game::server_side) fn load_cushion_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        transferred: Option<super::entity_initialization::TransferredEntityState>,
    ) {
        self.server.next_entity += 1;
        if let Some(c) = entity_nbt::cushion_from_nbt(&tag, self.server.next_entity) {
            if let Some(state) = transferred {
                self.adopt_transferred_entity_state(c.id, state);
            }
            self.remember(c.id, uuid, tag);
            self.common.cushions.push(c);
        }
    }

    /// A saved painting, or its compound when its block position is unreadable.
    pub fn load_painting(&mut self, tag: Tag, uuid: Option<[i32; 4]>, key: ChunkKey) {
        self.load_painting_transferred(tag, uuid, key, None)
    }
    pub(in crate::game::server_side) fn load_painting_transferred(
        &mut self,
        tag: Tag,
        uuid: Option<[i32; 4]>,
        key: ChunkKey,
        transferred: Option<super::entity_initialization::TransferredEntityState>,
    ) {
        // Painting.readAdditionalSaveData: a `variant` that does not resolve
        // leaves Painting.defineSynchedData's VariantUtils.getAny in place.
        if let Some((pos, facing, variant)) = entity_nbt::painting_from_nbt(&tag)
            && let Some(v) = world_paintings!(self).saved_or_default(variant.as_deref())
        {
            self.server.next_entity += 1;
            let id = self.server.next_entity;
            if let Some(state) = transferred {
                self.adopt_transferred_entity_state(id, state);
            }
            self.remember(id, uuid, tag);
            let mut painting = sunrise_sim::entities::Painting {
                attachment_lifetime: Default::default(),
                join: Default::default(),
                id,
                pos,
                facing,
                variant: v.id,
                asset: None,
                width: v.width,
                height: v.height,
                check_interval: 0,
                ticks: 0,
            };
            self.bind_saved_entity_attachments(id, &mut painting.attachment_lifetime);
            self.common.paintings.push(painting);
        } else {
            self.common
                .persist
                .unknown
                .entry(key)
                .or_default()
                .push(tag);
        }
    }

    /// `Entity.load`: adopt the bounded TAG_LIST_CODEC input as a set.
    /// Keep that adopted Mob state in the retained compound for commands and saves.
    pub(in crate::game::server_side) fn normalize_loaded_mob_tags(tag: &mut Tag) {
        let Tag::Compound(raw) = tag else {
            return;
        };
        let Some(Tag::List(mut tags)) = raw.remove(&"Tags".into()) else {
            return;
        };
        tags.retain(|tag| matches!(tag, Tag::String(_)));
        tags.truncate(1024);
        let mut seen = HashSet::new();
        tags.retain(|tag| seen.insert(tag.clone()));
        if !tags.is_empty() {
            raw.insert("Tags".into(), Tag::List(tags));
        }
    }

    /// A saved mob and, under `Passengers`, the mobs riding it.
    pub fn load_mob(&mut self, mut tag: Tag, mount: Option<u32>) -> Option<u32> {
        if !self.retain_feature_eligible_entity_tree(&mut tag) {
            return None;
        }
        self.load_mob_transferred(tag, mount, None)
    }

    /// A passenger that uses the canonical mob NBT loader.
    pub(in crate::game::server_side) fn loads_as_mob_passenger(&self, passenger: &Tag) -> bool {
        entity_nbt::kind(passenger).is_some_and(|k| self.common.res.mob_types().contains_key(&k))
    }

    fn loads_as_passenger(&self, kind: &str) -> bool {
        self.common.res.mob_types().contains_key(kind)
            || super::general::general_type_catalog().slot(kind).is_some()
            || kind == "minecraft:lightning_bolt"
    }

    /// Install each supported passenger in list order. Raw records for an unavailable
    /// general type stay with the vehicle rather than becoming independent entities.
    fn load_passengers(&mut self, vehicle: u32, riders: Vec<Tag>) {
        let mut unknown = Vec::new();
        for rider in riders {
            if self.loads_as_mob_passenger(&rider) {
                if let Some(id) = self.load_mob_transferred(rider, Some(vehicle), None)
                    && let Some(mob) = self.mob_object_mut(vehicle)
                {
                    mob.extra.rider.get_or_insert(id);
                }
            } else if entity_nbt::kind(&rider).as_deref() == Some("minecraft:lightning_bolt") {
                let _ = self.load_bolt(rider, Some(vehicle), false);
            } else if self
                .load_general_passenger(&rider, Some(vehicle), None)
                .is_err()
                && let Some(kind) = entity_nbt::kind(&rider)
                && entity_nbt::position(&rider)
                    .is_some_and(|at| self.general_quarantine_admits(&rider, &kind, at))
            {
                unknown.push(rider);
            }
        }
        if !unknown.is_empty()
            && let Some(raw) = self.common.persist.raw.get_mut(&vehicle)
        {
            add_passengers(raw, unknown);
        }
    }
    /// The UUIDs `spawn_entity_from_nbt` would add for `tag`, in
    /// `Entity.getSelfAndPassengers` order: the root's, then those of the
    /// riders its loader takes, read as the loaders read them (a malformed
    /// UUID is none). Passengers the loaders keep unloaded or that cannot
    /// attach to a non-serializable vehicle have none here.
    pub(in crate::game::server_side) fn summon_tree_uuids(&self, tag: &Tag) -> Vec<[i32; 4]> {
        let mut uuids: Vec<[i32; 4]> = entity_nbt::entity_uuid(tag).into_iter().collect();
        let kind = entity_nbt::kind(tag).unwrap_or_default();
        let takes_riders = ((!UNLISTED_KINDS.contains(&kind.as_str())
            && self.listed_kind_takes_riders(&kind))
            || kind == "minecraft:armor_stand"
            || super::general::general_type_catalog().slot(&kind).is_some()
            || (!UNLISTED_KINDS.contains(&kind.as_str())
                && !self.listed_kind_takes_riders(&kind)
                && Self::other_takes_passengers(&kind)))
            && sunrise_gameplay_common::game::seats_common::could_accept_passenger(&kind)
            && super::seats::saves_with_chunk(&kind);
        if takes_riders {
            self.rider_uuids(tag, &mut uuids);
        }
        uuids
    }
    fn rider_uuids(&self, tag: &Tag, uuids: &mut Vec<[i32; 4]>) {
        if let Some(Tag::List(list)) = tag.get("Passengers") {
            for p in list
                .iter()
                .filter(|p| entity_nbt::kind(p).is_some_and(|kind| self.loads_as_passenger(&kind)))
            {
                uuids.extend(entity_nbt::entity_uuid(p));
                let p_kind = entity_nbt::kind(p).unwrap_or_default();
                if sunrise_gameplay_common::game::seats_common::could_accept_passenger(&p_kind)
                    && super::seats::saves_with_chunk(&p_kind)
                {
                    self.rider_uuids(p, uuids);
                }
            }
        }
    }

    pub(in crate::game::server_side) fn put_passengers(&mut self, tag: &mut Tag, id: u32) {
        self.put_passengers_for(tag, id, false);
    }

    /// The passengers of `id` under `tag`'s `Passengers`, whole subtrees of either family. Tags are
    /// built top-down and attached bottom-up, so no chain of passengers recurses (vanilla bounds a
    /// saved tree only by the NBT nesting limit, which the region writer applies).
    pub(in crate::game::server_side) fn put_passengers_for(
        &mut self,
        tag: &mut Tag,
        id: u32,
        transferring: bool,
    ) {
        const ROOT: usize = usize::MAX;
        let mut nodes: Vec<(usize, Tag)> = Vec::new();
        let mut pending: Vec<(usize, u32)> = self
            .vehicle_passenger_ids(id)
            .into_iter()
            .rev()
            .map(|rider| (ROOT, rider))
            .collect();
        // Nesting of each node under `tag`: one compound and one list per level.
        let mut levels: Vec<usize> = Vec::new();
        while let Some((parent, rider)) = pending.pop() {
            let level = if parent == ROOT {
                1
            } else {
                levels[parent] + 1
            };
            // The disk writer's own recursion guard: past it no tree can be written, so a chain
            // this deep is cut here and said so, never built (its drop would recurse as deep).
            if level * 2 + SAVE_NESTING_MARGIN > sunrise_core::nbt::DISK_WRITE_NESTING_GUARD {
                log::error!(
                    "entity {rider}: passenger chain deeper than {level} riders is not saved"
                );
                continue;
            }
            let own = if let Some(object) = self.mob_object_slot(rider) {
                Some(self.mob_object_nbt(object))
            } else {
                self.general_own_nbt(rider, transferring)
            };
            let Some(own) = own else { continue };
            let node = nodes.len();
            levels.push(level);
            nodes.push((parent, own));
            pending.extend(
                self.vehicle_passenger_ids(rider)
                    .into_iter()
                    .rev()
                    .map(|child| (node, child)),
            );
        }
        let mut attached: Vec<Vec<Tag>> = (0..=nodes.len()).map(|_| Vec::new()).collect();
        for (node, (parent, mut own)) in nodes.into_iter().enumerate().rev() {
            let mut children = std::mem::take(&mut attached[node]);
            children.reverse();
            add_passengers(&mut own, children);
            let slot = if parent == ROOT {
                attached.len() - 1
            } else {
                parent
            };
            attached[slot].push(own);
        }
        let mut riders = attached.pop().unwrap_or_default();
        riders.reverse();
        add_passengers(tag, riders);
    }

    #[cfg(not(feature = "test-support"))]
    pub(in crate::game::server_side) fn vehicle_passenger_ids(&self, vehicle: u32) -> Vec<u32> {
        self.non_player_passenger_ids(vehicle)
    }

    #[cfg(feature = "test-support")]
    pub fn vehicle_passenger_ids(&self, vehicle: u32) -> Vec<u32> {
        self.non_player_passenger_ids(vehicle)
    }

    /// Non-player passengers in `getPassengers` order, shared by NBT and transfer state.
    fn non_player_passenger_ids(&self, vehicle: u32) -> Vec<u32> {
        self.seated_passengers(vehicle)
            .into_iter()
            .filter_map(|(_, id, player)| (!player).then_some(id))
            .collect()
    }

    pub(in crate::game::server_side) fn vehicle_tree_ids(&self, root: u32) -> Vec<u32> {
        let mounts: Vec<_> = self
            .mob_objects()
            .filter(|m| !m.removed)
            .map(|m| (m.id, m.extra.mount))
            .chain(
                self.common
                    .resident_entities
                    .general
                    .rows
                    .iter()
                    .map(|r| (r.id, r.mount)),
            )
            .collect();
        vehicle_tree(root, &mounts)
    }

    /// Read an unsimulated entity without assigning it a new identity; an eye
    /// of ender's `Item` is normalised in `tag`, which is what it saves.
    pub fn other_from_nbt(
        &self,
        tag: &mut Tag,
        id: u32,
        kind: &str,
    ) -> Option<sunrise_gameplay_common::game::commands_common::OtherEntity> {
        let mut position = entity_nbt::position(tag)?;
        let display = self.read_display_nbt(kind, tag).map(Arc::new);
        let knot_check = (kind == "minecraft:leash_knot").then_some(0);
        if knot_check.is_some() {
            // BlockAttachedEntity.setPos -> LeashFenceKnotEntity.recalculateBoundingBox.
            let block = BlockPos::containing(position);
            position = sunrise_sim::mobs::leash::knot_state([block.x, block.y, block.z]).position;
        }
        let interaction = is_interaction(kind).then(|| InteractionState::from_tag(tag));
        let rotation = entity_nbt::rotation(tag);
        Some(
            sunrise_gameplay_common::game::commands_common::OtherEntity {
                size_answer: None,
                attachment_lifetime: Default::default(),
                join: Default::default(),
                id,
                kind: kind.to_owned(),
                position,
                yaw: rotation[0],
                pitch: rotation[1],
                display,
                interaction,
                knot_check,
                eye: (kind == "minecraft:eye_of_ender").then(|| SummonedEye::load(tag)),
            },
        )
    }

    /// An entity of a type Sunrise does not simulate, kept as its compound.
    pub fn load_other(&mut self, tag: Tag, kind: &str) -> Option<u32> {
        self.load_other_transferred(tag, kind, None)
    }
    pub fn load_other_transferred(
        &mut self,
        mut tag: Tag,
        kind: &str,
        transferred: Option<super::entity_initialization::TransferredEntityState>,
    ) -> Option<u32> {
        // A missing mod's record is opaque, not a vanilla fallback entity to normalize.
        let quarantined = super::general::quarantined_kind(kind).then(|| tag.clone());
        strip_finalize(&mut tag);
        let id = self.server.next_entity + 1;
        let other = self.other_from_nbt(&mut tag, id, kind)?;
        if let Some(display) = &other.display {
            display::strip_display_nbt(display, &mut tag);
        }
        let riders = if quarantined.is_none() && Self::other_takes_passengers(kind) {
            self.take_passengers(&mut tag)
        } else {
            Vec::new()
        };
        self.server.next_entity = id;
        let uuid = with_constructor_uuid(entity_nbt::uuid(&tag));
        if let Some(state) = transferred {
            self.adopt_transferred_entity_state(id, state);
        }
        self.remember(id, uuid, tag);
        if let Some(original) = quarantined {
            self.common.persist.raw.insert(id, original);
        }
        self.server.others.0.push(other);
        self.load_passengers(id, riders);
        Some(id)
    }

    /// A kept entity carries passengers in its save when it accepts and saves (quarantined records stay raw).
    pub(in crate::game::server_side) fn other_takes_passengers(kind: &str) -> bool {
        sunrise_gameplay_common::game::seats_common::could_accept_passenger(kind)
            && super::seats::saves_with_chunk(kind)
            && !super::general::quarantined_kind(kind)
    }

    /// The compound of an entity Sunrise does not simulate, with its current
    /// position, rotation and UUID.
    pub fn other_to_nbt(
        &self,
        o: &sunrise_gameplay_common::game::commands_common::OtherEntity,
        uuid: [i32; 4],
    ) -> Tag {
        self.other_tag(o, uuid, true)
    }

    pub(in crate::game::server_side) fn other_tag(
        &self,
        o: &sunrise_gameplay_common::game::commands_common::OtherEntity,
        uuid: [i32; 4],
        include_id: bool,
    ) -> Tag {
        let mut tag = self
            .common
            .persist
            .raw
            .get(&o.id)
            .cloned()
            .unwrap_or_else(|| Tag::Compound(Default::default()));
        let marker = matches!(o.kind.as_str(), "minecraft:marker" | "marker");
        if marker || o.interaction.is_some() {
            if !marker {
                write_static_entity_fields(&mut tag);
            }
            let base = entity_nbt::Base {
                id: &o.kind,
                uuid,
                position: o.position,
                motion: entity_nbt::motion(&tag),
                rotation: [o.yaw, o.pitch],
                on_ground: entity_nbt::flag(&tag, "OnGround"),
                fall_distance: sunrise_sim::save::float(tag.get("fall_distance")).unwrap_or(0.0),
            };
            let mut out = entity_nbt::EntityTagWriter::from_saved(tag);
            base.write(&mut out, include_id);
            // Marker.addAdditionalSaveData is empty.
            if let Some(state) = &o.interaction {
                state.write(&mut out);
            }
            return out.finish();
        }
        if let Tag::Compound(c) = &mut tag {
            if include_id {
                c.insert("id".into(), Tag::String(o.kind.as_str().into()));
            } else {
                c.remove(&"id".into());
            }
            c.insert(
                "Pos".into(),
                sunrise_sim::save::doubles(&[o.position.x, o.position.y, o.position.z]),
            );
            c.insert(
                "Rotation".into(),
                sunrise_sim::save::floats(&[o.yaw, o.pitch]),
            );
            c.insert("UUID".into(), Tag::IntArray(uuid.to_vec()));
        }
        if o.display.is_some() || is_interaction(&o.kind) {
            write_static_entity_fields(&mut tag);
        }
        if let Some(display) = &o.display {
            self.write_display_nbt(display, &mut tag);
        }
        if let Some(eye) = o.eye
            && let Tag::Compound(c) = &mut tag
        {
            // Entity.saveWithoutId: the live motion and the fire ticks.
            let m = eye.motion;
            c.insert(
                "Motion".into(),
                sunrise_sim::save::doubles(&[m.x, m.y, m.z]),
            );
            c.insert("Fire".into(), Tag::Short(eye.fire_ticks as i16));
        }
        // LeashFenceKnotEntity and EyeOfEnder still use the common fields
        // of Entity.saveWithoutId, which their own saves leave out.
        if o.eye.is_some() || o.kind == "minecraft:leash_knot" {
            entity_nbt::fill_common_defaults(&mut tag);
        }
        with_raw(tag, None) // Entity.saveWithoutId's flags, as booleans.
    }
    /// A known, non-player vehicle's position for choosing the passenger tree's chunk.
    fn saved_vehicle_position(&self, id: u32) -> Option<Vec3> {
        self.common
            .mob_object(id)
            .map(|m| m.position)
            .or_else(|| self.boat_object(id).map(|b| b.position))
            .or_else(|| self.cart_object(id).map(|c| c.position))
            .or_else(|| self.seat(id).map(|seat| seat.position))
    }

    /// `Entity.getVehicle` for the entities whose tree a player can save.
    fn vehicle_of(&self, id: u32) -> Option<u32> {
        let vehicle = self.vehicle_of_any(id)?;
        self.saved_vehicle_position(vehicle).map(|_| vehicle)
    }
    fn riding_root(&self, id: u32) -> Option<u32> {
        let mut root = id;
        for _ in 0..=self.riding_link_limit() {
            match self.vehicle_of(root) {
                Some(parent) => root = parent,
                None => return Some(root),
            }
        }
        None
    }

    pub(in crate::game) fn write_entity_attachments(&mut self, tag: &mut Tag, id: u32) {
        if let Err(error) = write_attachments(tag, id, &self.common.entity_attachments) {
            self.common
                .persist
                .attachment_encode_error
                .get_or_insert(error);
        }
    }

    /// The root vehicle `root` with its passengers (`Entity.save`).
    pub fn vehicle_tree_nbt(&mut self, root: u32) -> Option<Tag> {
        self.vehicle_tree_nbt_for(root, false)
    }

    pub(in crate::game::server_side) fn vehicle_tree_nbt_for(
        &mut self,
        root: u32,
        transferring: bool,
    ) -> Option<Tag> {
        if let Some(object) = self.mob_object_slot(root) {
            return Some(self.mob_object_tree_nbt(object, transferring));
        }
        if self.common.boats.iter().any(|b| b.id == root)
            && self.resident_source_target_visible(
                self.common.dim.key(),
                sunrise_gameplay_common::game::commands_common::Target::Boat(root),
            )
        {
            let uuid = self.uuid_for(root);
            let b = self.boat_object(root)?;
            let mut tag = b.to_nbt(
                uuid,
                self.common.persist.raw.get(&root),
                self.common.res.items(),
            );
            self.put_passengers_for(&mut tag, root, transferring);
            self.write_entity_attachments(&mut tag, root);
            return Some(tag);
        }
        if self.common.general_row(root).is_some() {
            return self.general_tree_nbt(root, transferring);
        }
        let uuid = self.uuid_for(root);
        let mut tag = if self.common.carts.iter().any(|c| c.id == root)
            && self.resident_source_target_visible(
                self.common.dim.key(),
                sunrise_gameplay_common::game::commands_common::Target::Cart(root),
            ) {
            self.cart_object(root)?.to_nbt(
                uuid,
                self.common.persist.raw.get(&root),
                self.common.res.items(),
            )
        } else if let Some(stand) = self.common.armor_stands.iter().find(|a| a.id == root) {
            let mut tag = entity_nbt::armor_stand_to_nbt(
                stand,
                uuid,
                self.common.persist.raw.get(&root),
                self.common.res.items(),
            );
            self.save_portal_cooldown(root, &mut tag);
            tag
        } else {
            let other = self
                .server
                .others
                .0
                .iter()
                .find(|o| o.id == root && Self::other_takes_passengers(&o.kind))?;
            self.other_to_nbt(other, uuid)
        };
        self.put_passengers_for(&mut tag, root, transferring);
        self.write_entity_attachments(&mut tag, root);
        Some(tag)
    }
    /// `ServerPlayer.saveParentVehicle`: a player riding `vehicle` saves
    /// RootVehicle {Attach: that mount's UUID, Entity: the root vehicle with
    /// its passengers}, only while it is the root's one player passenger
    /// (`Entity.hasExactlyOnePlayerPassenger`); `others` are the vehicles the
    /// other players in the level ride. With the root and the mount's UUID.
    pub(in crate::game::server_side) fn parent_vehicle_tag(
        &mut self,
        vehicle: Option<u32>,
        others: &[u32],
    ) -> Option<(Tag, u32, [i32; 4])> {
        let vehicle = vehicle?;
        let root = parent_vehicle_root(vehicle, others, |id| self.vehicle_of(id))?;
        if self.common.persist.transient.contains(&root) {
            return None;
        }
        let entity = self.vehicle_tree_nbt(root)?;
        let attach = self.uuid_for(vehicle);
        Some((root_vehicle_tag(attach, entity), root, attach))
    }

    /// On logout the sole player's tree leaves even if its root opts out of saving.
    pub fn snapshot_parent_vehicle(
        &mut self,
        vehicle: u32,
        others: &[u32],
        remove: bool,
    ) -> Option<(Tag, [i32; 4])> {
        let saved = self.parent_vehicle_tag(Some(vehicle), others);
        if remove {
            let root = saved.as_ref().map(|(_, root, _)| *root).or_else(|| {
                if self.common.persist.transient.is_empty() {
                    return None;
                }
                parent_vehicle_root(vehicle, others, |id| self.vehicle_of(id))
                    .filter(|root| self.common.persist.transient.contains(root))
            });
            if let Some(root) = root {
                self.remove_vehicle_tree(root);
            }
        }
        saved.map(|(tag, _, attach)| (tag, attach))
    }
    /// `PlayerList.remove`: the root vehicle and its passengers leave the
    /// world with the player (`setRemoved(UNLOADED_WITH_PLAYER)`).
    pub fn remove_vehicle_tree(&mut self, root: u32) {
        let gone = self.vehicle_tree_ids(root);
        self.remove_vehicle_rows(&gone);
        for id in gone {
            self.forget_entity(id);
        }
    }

    /// Drop objects after the caller captured the tree, before forgetting or transferring its state.
    pub(in crate::game::server_side) fn remove_vehicle_rows(&mut self, gone: &[u32]) {
        use crate::game::natural_spawning::NaturalAdmission;
        if let Some(NaturalAdmission::Placed { level, id, join }) = &self.server.natural_admission
            && *level == self.common.dim
            && gone.contains(id)
            && let Some(index) = self.common.mobs.locate(*join)
        {
            self.detach_natural_candidate(*id, index);
        }
        self.common.mobs.retain(|m| !gone.contains(&m.id));
        self.common
            .persist
            .off_level
            .retain(|id, _| !gone.contains(id));
        for &id in gone {
            self.common.release_vehicle_container(id);
        }
        self.common.boats.retain(|b| !gone.contains(&b.id));
        self.common.carts.retain(|c| !gone.contains(&c.id));
        self.common.armor_stands.retain(|a| !gone.contains(&a.id));
        self.server.others.0.retain(|o| !gone.contains(&o.id));
        self.common
            .resident_entities
            .general
            .rows
            .retain(|r| !gone.contains(&r.id));
    }
    /// The vehicle with this UUID, among the ones in the world.
    pub fn vehicle_by_uuid(&self, uuid: [i32; 4]) -> Option<u32> {
        self.common.persist.uuids.iter().find_map(|(&id, &found)| {
            (found == uuid
                && (self
                    .common
                    .mobs
                    .iter()
                    .any(|mob| mob.id == id && !mob.removed)
                    || (self.common.boats.iter().any(|boat| boat.id == id)
                        && self.resident_source_target_visible(
                            self.common.dim.key(),
                            sunrise_gameplay_common::game::commands_common::Target::Boat(id),
                        ))
                    || (self.common.carts.iter().any(|cart| cart.id == id)
                        && self.resident_source_target_visible(
                            self.common.dim.key(),
                            sunrise_gameplay_common::game::commands_common::Target::Cart(id),
                        ))
                    || self.seat(id).is_some()))
            .then_some(id)
        })
    }
    /// A vehicle already in the world that was restored from the player's file
    /// during this load (not any entity whose UUID is remembered).
    pub fn restored_vehicle(&self, uuid: [i32; 4]) -> Option<u32> {
        if !self.common.persist.restored.contains(&uuid) {
            return None;
        }
        self.vehicle_by_uuid(uuid)
    }
    /// Vanilla stores the root entity in the player's file, outside chunk entities.
    /// Keep the raw wrapper until attachment succeeds, including unsupported trees.
    pub fn restore_saved_vehicle(&mut self, uuid: [i32; 4], raw: Option<&Tag>) -> Option<u32> {
        if let Some(id) = self.vehicle_by_uuid(uuid) {
            return Some(id);
        }
        let tag = raw?.get("RootVehicle")?.get("Entity")?.clone();
        let root_uuid = entity_nbt::uuid(&tag)?;
        let kind = entity_nbt::kind(&tag)?;
        if !self.common.res.mob_types().contains_key(&kind)
            && sunrise_gameplay_common::game::boats_common::any_boat_wood(&kind).is_none()
            && super::minecarts::CartKind::from_id(&kind).is_none()
            && super::general::general_type_catalog().slot(&kind).is_none()
            && kind != "minecraft:armor_stand"
            && !(Self::other_takes_passengers(&kind) && self.entity_type_exists(&kind))
        {
            return None;
        }
        if self.vehicle_by_uuid(root_uuid).is_none() {
            let at = entity_nbt::position(&tag)?;
            let key = (
                (at.x.floor() as i32).div_euclid(16),
                (at.z.floor() as i32).div_euclid(16),
            );
            self.common.persist.restored.insert(root_uuid);
            self.common.world.queue_entity_nbt(key, vec![tag]);
            self.spawn_loaded_entities();
        }
        self.common.persist.restored.insert(uuid);
        self.vehicle_by_uuid(uuid)
    }
    /// `ServerPlayer.loadAndSpawnParentVehicle`: the ride the player's file
    /// names, once the vehicle is in the world.
    pub(in crate::game::server_side) fn restore_root_vehicle(&mut self) {
        if self.server.serving.is_some()
            || self.common.options.dedicated.is_some()
            || self.common.riding.is_some()
            || self.common.player.is_none()
        {
            return;
        }
        let Some(uuid) = self.server.pending_vehicle else {
            return;
        };
        let raw = self.common.level.player_extra.clone();
        let Some(id) = self.restore_saved_vehicle(uuid, raw.as_ref()) else {
            return;
        };
        if let Some(Tag::Compound(p)) = &mut self.common.level.player_extra {
            p.remove(&"RootVehicle".into());
        }
        self.server.pending_vehicle = None;
        self.common.riding = Some(id);
        // `PlayerList.placeNewPlayer` boards the saved vehicle through `startRiding`, so `addPassenger` places it.
        self.seat_player(id);
    }
    /// A mob's entity tag with the mobs riding it under `Passengers`.
    pub fn mob_tree_nbt(&mut self, i: usize) -> Tag {
        self.mob_object_tree_nbt(MobObject::Resident(i), false)
    }

    pub(in crate::game::server_side) fn mob_tree_tag_without_id(&mut self, i: usize) -> Tag {
        let id = self.common.mobs[i].id;
        let mut tag = self.mob_object_tag(MobObject::Resident(i), false);
        self.put_passengers(&mut tag, id);
        tag
    }
    /// A hive stores just its entering bee, not its riding tree.
    pub(in crate::game::server_side) fn mob_own_nbt(&mut self, i: usize) -> Tag {
        self.mob_object_nbt(MobObject::Resident(i))
    }
    /// Forget the records of an entity that left the world.
    pub fn forget_entity(&mut self, id: u32) {
        self.common.persist.transient.remove(&id);
        self.common.persist.uuids.remove(&id);
        self.server.last_hurt_by_player.remove(&id);
        self.common.persist.score_destroyed.remove(&id);
        self.common
            .forget_entity_initialization(mod_api::Holder::Entity(id));
        self.common.entity_attachments.remove(&id);
        self.common.entity_attachments.remove_collider(id);
        self.common.persist.raw.remove(&id);
        self.common.persist.no_physics.remove(&id);
    }
    /// Detach a travelling entity without discarding its in-memory attachments.
    pub fn detach_entity(&mut self, id: u32) {
        self.common.persist.score_destroyed.remove(&id);
        self.common.persist.uuids.remove(&id);
        self.common.persist.raw.remove(&id);
    }
    /// The compound a crystal or the dragon was loaded with, found again by type
    /// and position.
    pub(in crate::game::server_side) fn persist_anon(&self, kind: &str, p: Vec3) -> Option<&Tag> {
        self.common
            .persist
            .anon
            .get(&anon_key(kind, p))
            .map(|(_, raw)| raw)
    }
    /// `Entity.applyEffectsFromBlocks` on a fire-immune entity that is not
    /// burning (`EndCrystal.tick`): the loaded `Fire` is reset to
    /// `-getFireImmuneTicks()`, which is 0.
    pub fn clear_crystal_fire(&mut self) {
        for crystal in &mut self.common.crystals {
            crystal.fire = 0;
        }
    }

    pub fn crystal_nbt(
        &self,
        crystal: &sunrise_gameplay_common::game::dragon_common::Crystal,
    ) -> Tag {
        let uuid = saved_uuid(&self.common.persist, crystal.id);
        let raw = self
            .common
            .persist
            .raw
            .get(&crystal.id)
            .or_else(|| self.persist_anon("minecraft:end_crystal", crystal.position));
        crystal.to_nbt(uuid, raw)
    }

    /// An entity Sunrise keeps no compound for (orbs, projectiles, the
    /// crystals and the dragon) still keeps its `Tags`, by entity id, so
    /// `@e[tag=]`, `/tag` and `/data get` see them, and the `Item` a
    /// fireball or a thrown item projectile was loaded with.
    pub(in crate::game) fn remember_tags(&mut self, id: u32, tag: &Tag) {
        let mut map = sunrise_core::nbt::Compound::new();
        if let Some(tags @ Tag::List(_)) = tag.get("Tags") {
            map.insert("Tags".into(), tags.clone());
        }
        if let Some(item @ Tag::Compound(_)) = tag.get("Item") {
            map.insert("Item".into(), item.clone());
        }
        // Entity.saveWithoutId's optional fields Sunrise models nowhere for
        // these entities, kept as loaded (written only when set).
        for key in [
            "Silent",
            "Glowing",
            "HasVisualFire",
            "Invulnerable",
            "CustomName",
            "CustomNameVisible",
        ] {
            if let Some(v) = tag.get(key)
                && (key != "CustomName" || entity_nbt::valid_custom_name_tag(v))
            {
                map.insert(key.into(), v.clone());
            }
        }
        if !map.is_empty() {
            self.common.persist.raw.insert(id, Tag::Compound(map));
        }
    }
    /// `Entity.isSilent`: mobs model it directly; other entities retain it
    /// in their saved fields, including command changes to `Silent`.
    pub(in crate::game) fn entity_silent(&self, id: u32) -> bool {
        self.common.mobs.iter().find(|m| m.id == id).map_or_else(
            || {
                self.common
                    .persist
                    .raw
                    .get(&id)
                    .is_some_and(|tag| sunrise_sim::entity_nbt::flag(tag, "Silent"))
            },
            |m| m.silent,
        )
    }
    /// The `Tags` of the projectile `add_projectile` just pushed.
    fn remember_last_projectile_tags(&mut self, tag: &Tag) {
        if let Some(id) = self.common.creatures.projectiles.last().map(|p| p.id) {
            self.remember_tags(id, tag);
        }
    }
    pub(in crate::game::server_side) fn remember(
        &mut self,
        id: u32,
        uuid: Option<[i32; 4]>,
        mut tag: Tag,
    ) {
        self.restore_portal_cooldown(id, &tag);
        self.remember_uuid(id, uuid, &tag);
        // `Team` is read, never written: the team keeps its members itself.
        if let Tag::Compound(c) = &mut tag {
            c.remove(&"Team".into());
        }
        entity_nbt::sanitize_custom_name(&mut tag);
        self.common.persist.raw.insert(id, tag);
    }
    /// The UUID entity `id` was loaded with, and the attachments `tag` saved under it.
    pub fn remember_uuid(&mut self, id: u32, uuid: Option<[i32; 4]>, tag: &Tag) {
        if let Some(u) = uuid {
            self.common.persist.uuids.insert(id, u);
        }
        // `EntityType.create` then `Entity.load`: the constructing listeners set their
        // values first and the saved attachments are read over them. An entity whose map
        // is already live (told before) keeps it, as before.
        let live = self.common.entity_attachments.contains_key(&id);
        if let Some(kind) = text(tag.get("id")) {
            self.initialize_entity(mod_api::Holder::Entity(id), &kind);
        }
        read_attachments(tag, id, &mut self.common.entity_attachments, live);
        self.load_saved_collider(id, tag);
        self.join_saved_team(id, tag);
    }
    pub(in crate::game::server_side) fn join_saved_team(&mut self, id: u32, tag: &Tag) {
        if tag.get("Team").is_some() {
            let name = uuid_text(self.uuid_for(id));
            join_saved_team(&mut self.common.score.board, &name, tag);
        }
    }
    /// The entity a UUID read back from a save names, while it is in the level:
    /// the player, or a mob loaded or saved with that UUID. `None` for nothing
    /// loaded (an entity that is not here, or one `Who` cannot carry) - never
    /// some other entity as a stand-in.
    pub fn who_by_uuid(&self, uuid: [i32; 4]) -> Option<Who> {
        let player = if self.common.player.is_some() {
            self.common.level.player_uuid
        } else {
            None
        };
        entity_nbt::who_by_uuid(&self.common.persist.uuids, &self.common.mobs, player, uuid)
    }
    /// The mob that was loaded or saved with a UUID.
    pub fn mob_by_uuid(&self, uuid: [i32; 4]) -> Option<u32> {
        match self.who_by_uuid(uuid) {
            Some(Who::Mob(id)) => Some(id),
            _ => None,
        }
    }
    /// A mob's UUID, made up for new ones.
    pub fn mob_uuid(&mut self, id: u32) -> [i32; 4] {
        self.uuid_for(id)
    }
    pub fn uuid_for(&mut self, id: u32) -> [i32; 4] {
        if let Some(uuid) = self.known_resident_entity_uuid_at(self.common.dim.key(), id) {
            return uuid;
        }
        self.common
            .persist
            .uuids
            .get_or_insert_with(id, random_uuid)
    }

    pub(in crate::game) fn entity_set_persistent(
        &mut self,
        entity: mod_api::EntityRef,
        persistent: bool,
    ) -> std::result::Result<bool, mod_api::LevelWriteError> {
        use sunrise_gameplay_common::game::commands_common::Target;
        let id = entity.id();
        match self.native_entity_target(entity) {
            None | Some(Target::Player | Target::Guest(_) | Target::DragonPart(_)) => {
                return Ok(false);
            }
            Some(Target::Spawner(_)) => {
                // A freshly spawned ominous spawner already carries its save UUID.
                let Some(spawner) = self.common.ominous_spawners.iter().find(|row| row.id == id)
                else {
                    return Ok(false);
                };
                self.common
                    .persist
                    .uuids
                    .get_or_insert_with(id, || spawner.uuid);
            }
            Some(_) => {
                self.uuid_for(id);
            }
        }
        if persistent {
            self.common.persist.transient.remove(&id);
        } else {
            self.common.persist.transient.insert(id);
        }
        Ok(true)
    }

    /// `BlockState.CODEC`: a block name or `{id, properties}` (or legacy `{Name, Properties}`).
    pub fn state_from_nbt(&self, tag: &Tag) -> Option<BlockStateId> {
        entity_nbt::state_from_nbt(self.common.pack.assets.registry, tag)
    }
    /// World-save snapshots, optionally removing the entities as their chunks unload.
    pub fn entity_chunks(
        &mut self,
        keys: &HashSet<(i32, i32)>,
        remove: bool,
    ) -> Vec<((i32, i32), Vec<Tag>)> {
        let purpose = if remove {
            EntitySnapshot::Unload
        } else {
            EntitySnapshot::Save
        };
        self.snapshot_entity_chunks(keys, purpose).0
    }

    /// The entity NBT in `keys`, plus any unknown entities kept for them.
    /// No snapshot holds a player vehicle: a mob seated on a player gets off first.
    pub fn snapshot_entity_chunks(
        &mut self,
        keys: &HashSet<ChunkKey>,
        purpose: EntitySnapshot,
    ) -> (Vec<(ChunkKey, Vec<Tag>)>, HashSet<ChunkKey>) {
        use sunrise_sim::join_seq::{JoinSeq, Joined};
        self.eject_player_riders(None);
        let include_transient = matches!(purpose, EntitySnapshot::Structure);
        // Each root with its id and join number, ordered once all are in.
        let mut roots: HashMap<ChunkKey, Vec<(u32, JoinSeq, Tag)>> = HashMap::new();
        let mut failed_chunks: HashMap<
            ChunkKey,
            sunrise_mod_runtime::attachments::AttachmentEncodeError,
        > = HashMap::new();
        // Every kind in `entity_lists!` saves itself (`Entity::save_nbt`).
        self.save_every_entity(keys, &mut roots, &mut failed_chunks, include_transient);
        // Owned, since saving a passenger tree borrows `self` mutably.
        let transient = self.common.persist.transient.clone();
        let include = |id, at| {
            keys.contains(&chunk_of(at)) && (include_transient || !transient.contains(&id))
        };
        if let Some(d) = self.common.dragon.as_ref()
            && include(d.id, d.position)
        {
            let raw = anon_raw(&self.common.persist, "minecraft:ender_dragon", d.position);
            let uuid = saved_uuid(&self.common.persist, d.id);
            // The fields kept with it (`remember_tags`: Silent, Tags, a
            // name...) once it has flown from where it was loaded.
            let mut tag = with_raw(
                d.to_nbt(uuid, raw.as_ref(), self.common.res.items()),
                self.common.persist.raw.get(&d.id),
            );
            let key = chunk_of(d.position);
            match write_attachments(&mut tag, d.id, &self.common.entity_attachments) {
                Ok(()) => roots
                    .entry(key)
                    .or_default()
                    .push((d.id, d.join_seq(), tag)),
                Err(error) => {
                    failed_chunks.entry(key).or_insert(error);
                }
            }
        }
        for c in &self.common.crystals {
            if include(c.id, c.position) {
                let mut tag = self.crystal_nbt(c);
                let key = chunk_of(c.position);
                match write_attachments(&mut tag, c.id, &self.common.entity_attachments) {
                    Ok(()) => roots
                        .entry(key)
                        .or_default()
                        .push((c.id, c.join_seq(), tag)),
                    Err(error) => {
                        failed_chunks.entry(key).or_insert(error);
                    }
                }
            }
        }
        for s in &self.common.ominous_spawners {
            let at = Vec3::new(s.position[0], s.position[1], s.position[2]);
            if include(s.id, at) {
                let mut tag = entity_nbt::ominous_item_spawner_to_nbt(
                    &s.item,
                    at,
                    s.spawn_item_after,
                    s.fire_ticks,
                    self.common
                        .persist
                        .uuids
                        .get(&s.id)
                        .copied()
                        .unwrap_or(s.uuid),
                    s.raw.as_deref(),
                    self.common.res.items(),
                );
                let key = chunk_of(at);
                match write_attachments(&mut tag, s.id, &self.common.entity_attachments) {
                    Ok(()) => roots
                        .entry(key)
                        .or_default()
                        .push((s.id, s.join_seq(), tag)),
                    Err(error) => {
                        failed_chunks.entry(key).or_insert(error);
                    }
                }
            }
        }
        for f in &self.common.item_frames {
            let (lo, hi) = f.bounds();
            let at = Vec3::new(
                (lo[0] + hi[0]) / 2.0,
                (lo[1] + hi[1]) / 2.0,
                (lo[2] + hi[2]) / 2.0,
            );
            if include(f.id, at) {
                let uuid = self
                    .common
                    .persist
                    .uuids
                    .get(&f.id)
                    .copied()
                    .unwrap_or_else(random_uuid);
                let mut tag = entity_nbt::item_frame_to_nbt(
                    f,
                    uuid,
                    self.common.persist.raw.get(&f.id),
                    self.common.res.items(),
                );
                let key = chunk_of(at);
                match write_attachments(&mut tag, f.id, &self.common.entity_attachments) {
                    Ok(()) => roots
                        .entry(key)
                        .or_default()
                        .push((f.id, f.join_seq(), tag)),
                    Err(error) => {
                        failed_chunks.entry(key).or_insert(error);
                    }
                }
            }
        }
        for c in &self.common.cushions {
            if include(c.id, c.position) {
                let uuid = self
                    .common
                    .persist
                    .uuids
                    .get(&c.id)
                    .copied()
                    .unwrap_or_else(random_uuid);
                let mut tag =
                    entity_nbt::cushion_to_nbt(c, uuid, self.common.persist.raw.get(&c.id));
                let key = chunk_of(c.position);
                match write_attachments(&mut tag, c.id, &self.common.entity_attachments) {
                    Ok(()) => roots
                        .entry(key)
                        .or_default()
                        .push((c.id, c.join_seq(), tag)),
                    Err(error) => {
                        failed_chunks.entry(key).or_insert(error);
                    }
                }
            }
        }
        for p in &self.common.paintings {
            let at = painting_center(p);
            if include(p.id, at) {
                let uuid = self
                    .common
                    .persist
                    .uuids
                    .get(&p.id)
                    .copied()
                    .unwrap_or_else(random_uuid);
                let mut tag =
                    entity_nbt::painting_to_nbt(p, uuid, self.common.persist.raw.get(&p.id));
                let key = chunk_of(at);
                match write_attachments(&mut tag, p.id, &self.common.entity_attachments) {
                    Ok(()) => roots
                        .entry(key)
                        .or_default()
                        .push((p.id, p.join_seq(), tag)),
                    Err(error) => {
                        failed_chunks.entry(key).or_insert(error);
                    }
                }
            }
        }
        for f in &self.common.fangs.list {
            let at = Vec3::new(f.pos[0], f.pos[1], f.pos[2]);
            if include(f.id, at) {
                let uuid = self
                    .common
                    .persist
                    .uuids
                    .get(&f.id)
                    .copied()
                    .unwrap_or_else(random_uuid);
                let mut tag = self.fangs_to_nbt(f, uuid);
                let key = chunk_of(at);
                match write_attachments(&mut tag, f.id, &self.common.entity_attachments) {
                    Ok(()) => roots
                        .entry(key)
                        .or_default()
                        .push((f.id, f.join_seq(), tag)),
                    Err(error) => {
                        failed_chunks.entry(key).or_insert(error);
                    }
                }
            }
        }
        // `AreaEffectCloud.addAdditionalSaveData`: a lingering potion's cloud is
        // an entity of its chunk. A dragon's own breath carries no potion and
        // is not written.
        for c in &self.common.breath_clouds {
            let at = Vec3::new(c.at[0], c.at[1], c.at[2]);
            if c.potion.is_none() || !include(c.id, at) {
                continue;
            }
            let uuid = self
                .common
                .persist
                .uuids
                .get(&c.id)
                .copied()
                .unwrap_or_else(random_uuid);
            let mut tag = potion_cloud_to_nbt(c, uuid, self.common.persist.raw.get(&c.id));
            let key = chunk_of(at);
            match write_attachments(&mut tag, c.id, &self.common.entity_attachments) {
                Ok(()) => roots
                    .entry(key)
                    .or_default()
                    .push((c.id, c.join_seq(), tag)),
                Err(error) => {
                    failed_chunks.entry(key).or_insert(error);
                }
            }
        }
        for i in 0..self.server.others.0.len() {
            let o = &self.server.others.0[i];
            // EntityType.noSave: Entity.getEncodeId is null, so the chunk never stores it.
            if include(o.id, o.position) && o.kind.as_str() != "minecraft:leash_knot" {
                let uuid = self
                    .common
                    .persist
                    .uuids
                    .get(&o.id)
                    .copied()
                    .unwrap_or_else(random_uuid);
                let (id, position, join) = (o.id, o.position, o.join_seq());
                let key = chunk_of(position);
                self.common.persist.attachment_encode_error = None;
                let tag = if super::general::quarantined_kind(&o.kind)
                    && let Some(raw) = self.common.persist.raw.get(&o.id)
                {
                    raw.clone()
                } else {
                    let mut tag = self.other_to_nbt(o, uuid);
                    self.put_passengers(&mut tag, id);
                    self.write_entity_attachments(&mut tag, id);
                    tag
                };
                if let Some(error) = self.common.persist.attachment_encode_error.take() {
                    failed_chunks.entry(key).or_insert(error);
                } else {
                    roots.entry(key).or_default().push((id, join, tag));
                }
            }
        }
        // `PersistentEntitySectionManager.storeChunkSections`: section by
        // section, each in the order its entities entered it. An entity not
        // yet given an id is not a lookup member: its join stands in.
        let mut out: HashMap<(i32, i32), Vec<Tag>> =
            keys.iter().map(|k| (*k, Vec::new())).collect();
        let refused: HashSet<ChunkKey> = failed_chunks.keys().copied().collect();
        for (key, error) in failed_chunks {
            out.remove(&key);
            if !matches!(purpose, EntitySnapshot::Structure) {
                self.server
                    .save
                    .0
                    .report_entity_record_failure(key, &std::io::Error::other(error));
            }
        }
        {
            // A native discard takes effect before the physical removal sweep.
            let removed: HashSet<_> = self
                .common
                .persist
                .removals
                .iter()
                .filter_map(|&target| {
                    sunrise_gameplay_common::game::commands_common::target_id(target)
                })
                .collect();
            let entry = self
                .common
                .resident_entities
                .table
                .get_mut()
                .section_entry_order();
            for (key, mut tags) in roots {
                tags.retain(|(id, _, _)| !removed.contains(id));
                let Some(dest) = out.get_mut(&key) else {
                    continue;
                };
                tags.sort_by_cached_key(|(id, join, tag)| {
                    (section_order(tag), entry(*id).unwrap_or(*join))
                });
                dest.extend(tags.into_iter().map(|(_, _, tag)| tag));
            }
        }
        for key in keys {
            if let Some(dest) = out.get_mut(key)
                && let Some(list) = self.common.persist.unknown.get(key)
            {
                dest.extend(list.iter().cloned());
            }
        }
        if matches!(purpose, EntitySnapshot::Unload) {
            let unloading = keys.difference(&refused).copied().collect();
            self.remove_unloading_entities(&unloading);
        }
        (out.into_iter().collect(), refused)
    }

    /// Every chunk that currently holds an in-memory savable entity or kept
    /// unknown record (`PersistentEntitySectionManager.getAllChunksToSave`).
    fn occupied_entity_chunks(&self) -> HashSet<ChunkKey> {
        use sunrise_gameplay_common::game::entity::{
            Entity as _, entity_lists_of, for_each_entity,
        };
        let mut keys = HashSet::new();
        let mut include = |id: u32, at: Vec3| {
            if at.x.is_finite() && at.z.is_finite() && !self.common.persist.transient.contains(&id)
            {
                keys.insert(chunk_of(at));
            }
        };
        for_each_entity!(entity_lists_of!(self), e => {
            if !e.is_removed() {
                include(e.id(), e.position());
            }
        });
        if let Some(d) = self.common.dragon.as_ref() {
            include(d.id, d.position);
        }
        for c in &self.common.crystals {
            include(c.id, c.position);
        }
        for s in &self.common.ominous_spawners {
            include(s.id, Vec3::new(s.position[0], s.position[1], s.position[2]));
        }
        for f in &self.common.item_frames {
            let (lo, hi) = f.bounds();
            include(
                f.id,
                Vec3::new(
                    (lo[0] + hi[0]) / 2.0,
                    (lo[1] + hi[1]) / 2.0,
                    (lo[2] + hi[2]) / 2.0,
                ),
            );
        }
        for c in &self.common.cushions {
            include(c.id, c.position);
        }
        for p in &self.common.paintings {
            include(p.id, painting_center(p));
        }
        for f in &self.common.fangs.list {
            include(f.id, Vec3::new(f.pos[0], f.pos[1], f.pos[2]));
        }
        for c in &self.common.breath_clouds {
            if c.potion.is_some() {
                include(c.id, Vec3::new(c.at[0], c.at[1], c.at[2]));
            }
        }
        for o in &self.server.others.0 {
            if o.kind.as_str() != "minecraft:leash_knot" {
                include(o.id, o.position);
            }
        }
        keys.extend(self.common.persist.unknown.keys().copied());
        keys
    }

    /// `PersistentEntitySectionManager.getAllChunksToSave` / `storeChunkSections`:
    /// a fresh section with entities must read and admit the old records first.
    /// Its later saves replace only from the live section, even once empty.
    pub(in crate::game) fn prepare_entity_save(&mut self) -> HashSet<ChunkKey> {
        self.spawn_loaded_entities();
        let mut keys: HashSet<_> = self.common.world.chunks.keys().copied().collect();
        keys.extend(self.common.persist.hidden_entity_sections.iter().copied());
        let mut failed = HashSet::new();
        loop {
            let mut queued = false;
            for key in self.occupied_entity_chunks() {
                if keys.contains(&key) || failed.contains(&key) {
                    continue;
                }
                let Ok(tags) = self.server.save.0.load_entities(key.0, key.1) else {
                    failed.insert(key);
                    continue;
                };
                self.common.persist.hidden_entity_sections.insert(key);
                keys.insert(key);
                if !tags.is_empty() {
                    // Ownership is already set. Queue admission, not a worker
                    // snapshot which would be ignored for an owned section.
                    self.common.world.queue_entity_nbt(key, tags);
                    queued = true;
                }
            }
            if !queued {
                self.common.persist.unreadable_entity_sections = failed.len();
                return keys;
            }
            // Loading entities can itself add or move them to another column.
            self.spawn_loaded_entities();
        }
    }

    /// Save a generated full chunk that finished outside the kept terrain. If the
    /// level already owns its entity section, file the new generation records
    /// there rather than letting the terrain writer replace that section.
    pub(in crate::game) fn save_late_full_chunk(&mut self, mut chunk: Chunk) {
        let key = (chunk.x, chunk.z);
        let additions = if self.common.persist.hidden_entity_sections.contains(&key) {
            std::mem::take(&mut chunk.entities)
        } else {
            Vec::new()
        };
        self.server.streaming.save_finished(chunk);
        if !additions.is_empty() {
            self.common.world.queue_entity_nbt(key, additions);
            if let Err(e) = self.save_entities(false) {
                log::warn!("Sunrise: failed to save entities: {e}");
            }
        }
    }
    #[inline(always)]
    pub(in crate::game) fn save_entity_metadata(&mut self) -> Result<()> {
        if let Some(d) = self.common.dragon.as_ref() {
            if d.death_canceled {
                if self.common.persist.dragon_death_canceled.is_none() {
                    let (set, on_disk) = self.server.save.0.load_dragon_death_canceled();
                    self.common.persist.dragon_death_canceled = Some(set);
                    self.common.persist.entity_metadata_on_disk = on_disk;
                }
                let id = d.id;
                let uuid = self.uuid_for(id);
                self.common
                    .persist
                    .dragon_death_canceled
                    .get_or_insert_with(HashSet::new)
                    .insert(uuid);
            } else if let Some(&uuid) = self.common.persist.uuids.get(&d.id)
                && let Some(set) = self.common.persist.dragon_death_canceled.as_mut()
            {
                set.remove(&uuid);
            }
        }
        if !self.common.persist.entity_metadata_on_disk
            && self
                .common
                .persist
                .dragon_death_canceled
                .as_ref()
                .is_none_or(HashSet::is_empty)
        {
            return Ok(());
        }
        let empty = HashSet::new();
        let set = self
            .common
            .persist
            .dragon_death_canceled
            .as_ref()
            .unwrap_or(&empty);
        self.server.save.0.save_dragon_death_canceled(set)?;
        self.common.persist.entity_metadata_on_disk = !set.is_empty();
        Ok(())
    }

    /// Write the entities of every resident chunk (an autosave or quit).
    /// Without `wait` (the autosave) they are written on the region writer
    /// meanwhile: `PersistentEntitySectionManager.autoSave` stores them
    /// without `saveAll`'s flush.
    pub fn save_entities(&mut self, wait: bool) -> Result<()> {
        let keys = self.prepare_entity_save();
        let chunks = self.entity_chunks(&keys, false);
        self.save_entity_metadata()?;
        self.prune_entity_memory();
        if wait {
            self.server.save.0.save_entities(chunks)?;
        } else {
            self.server.save.0.save_entities_later(chunks)?;
        }
        self.entity_save_complete()
    }
    pub(in crate::game) fn entity_save_complete(&self) -> Result<()> {
        match self.common.persist.unreadable_entity_sections {
            0 => Ok(()),
            n => Err(format!("the old entity records of {n} column(s) could not be read").into()),
        }
    }

    /// Complete a removed vehicle's scoreboard callback at logical removal,
    /// even when physical removal waits for the native-removal sweep.
    pub(in crate::game::server_side) fn score_destroyed_vehicle_once(&mut self, id: u32) {
        if self.common.persist.score_destroyed.insert(id)
            && let Some(uuid) = self.common.persist.uuids.get(&id)
        {
            self.common.score.board.entity_removed(&uuid_text(*uuid));
        }
    }
    /// `ServerLevel.EntityCallbacks.onDestroyed` → `Scoreboard.entityRemoved`
    /// for a non-player entity removed immediately (such as `Entity.kill`),
    /// using its captured UUID so paths that call `forget_entity` still clear
    /// its scores and team.
    pub(in crate::game::server_side) fn score_destroyed_entity_once(
        &mut self,
        id: u32,
        uuid: [i32; 4],
    ) {
        if self.common.persist.score_destroyed.insert(id) {
            self.common.score.board.entity_removed(&uuid_text(uuid));
        }
    }

    /// `ServerLevel.EntityCallbacks.onDestroyed` → `Scoreboard.entityRemoved`
    /// for every kind of entity: a mob's body once it is removed, an item
    /// picked up or killed, an expired cloud, a discarded projectile leaves
    /// the board and its team. Unloaded and
    /// travelling entities leave `persist.uuids` first (`forget_entity`,
    /// `detach_entity`), so they keep their scores, as vanilla's do. Once a
    /// tick, so a command in the same tick still sees them; nothing to do
    /// while no entity holds a score or a team.
    pub fn score_destroyed_entities(&mut self) {
        if !self.common.persist.off_level.is_empty() {
            for (&id, object) in &self.common.persist.off_level {
                if object.is_removed()
                    && self.common.persist.score_destroyed.insert(id)
                    && let Some(uuid) = self.common.persist.uuids.get(&id)
                {
                    self.common.score.board.entity_removed(&uuid_text(*uuid));
                }
            }
        }
        for &id in self.common.persist.removed_vehicles.keys() {
            if self.common.persist.score_destroyed.insert(id)
                && let Some(uuid) = self.common.persist.uuids.get(&id)
            {
                self.common.score.board.entity_removed(&uuid_text(*uuid));
            }
        }
        let board = &self.common.score.board;
        let held: HashSet<[i32; 4]> = board
            .scores
            .keys()
            .map(String::as_str)
            .chain(board.team_members())
            .filter(|holder| holder.len() == 36)
            .filter_map(uuid_from_text)
            .collect();
        if held.is_empty() {
            return;
        }
        let candidates: Vec<(u32, [i32; 4])> = self
            .common
            .persist
            .uuids
            .iter()
            .filter(|(id, uuid)| {
                !self.common.persist.score_destroyed.contains(*id) && held.contains(*uuid)
            })
            .map(|(&id, &uuid)| (id, uuid))
            .collect();
        if candidates.is_empty() {
            return;
        }
        let mut alive = self.live_entity_ids();
        if !self.common.persist.off_level.is_empty() {
            // Query residency is not destruction. Rejected objects still own
            // their UUID state until removed, just like registered objects.
            alive.extend(
                self.common
                    .persist
                    .off_level
                    .iter()
                    .filter_map(|(&id, object)| (!object.is_removed()).then_some(id)),
            );
        }
        for (id, uuid) in candidates {
            if !alive.contains(&id) {
                self.common.score.board.entity_removed(&uuid_text(uuid));
            }
        }
    }
    pub fn prune_entity_memory(&mut self) {
        if !self.common.persist.removed_vehicles.is_empty()
            || self
                .common
                .persist
                .off_level
                .values()
                .any(OffLevel::is_removed)
        {
            // Complete the former resident's destruction before graph GC can
            // forget the UUID retained solely for dismount geometry.
            self.score_destroyed_entities();
        }
        // Forget entities that no longer exist.
        let mut alive = self.live_entity_ids();
        if !self.common.persist.off_level.is_empty()
            || !self.common.persist.removed_vehicles.is_empty()
        {
            self.retain_off_level_objects(&mut alive);
        }
        for id in self.common.persist.uuids.keys() {
            if !alive.contains(id) {
                self.common.entity_attachments.remove(id);
            }
        }
        self.prune_entity_initializations(&alive);
        self.common
            .persist
            .transient
            .retain(|id| alive.contains(id));
        self.common.persist.uuids.retain(|id, _| alive.contains(id));
        self.common.persist.raw.retain(|id, _| alive.contains(id));
        self.common
            .persist
            .score_destroyed
            .retain(|id| alive.contains(id));
    }

    /// A loaded projectile's `Owner`: living hit credit and vehicle collision identity.
    pub(in crate::game::server_side) fn claim_owner(
        &self,
        p: &mut sunrise_sim::projectiles::Projectile,
    ) {
        let Some(uuid) = p.owner_uuid else {
            return;
        };
        let guest = self.server.lan.as_ref().and_then(|host| {
            let ints = sunrise_sim::lan::playerdata::uuid_ints;
            host.hub
                .guests
                .iter()
                .find(|g| ints(g.uuid) == uuid)
                .map(|g| g.conn)
        });
        if guest.is_some() {
            (p.owner, p.guest) = (Some(Who::Player), guest);
        } else if p.owner.is_none() {
            p.owner = self.mob_by_uuid(uuid).map(Who::Mob);
        }
        use sunrise_gameplay_common::game::commands_common::Target;
        p.owner_vehicle = match self.target_by_uuid(sunrise_sim::lan::playerdata::ints_uuid(uuid)) {
            Some(Target::Boat(id) | Target::Cart(id)) => Some(id),
            _ => None,
        };
    }

    /// Capture the entity records before committing a chunk unload
    /// (`PersistentEntitySectionManager.processChunkUnload`), with the chunks
    /// whose record was refused: those stay loaded with their entities.
    pub fn unloading_entity_snapshots(
        &mut self,
        keys: &HashSet<ChunkKey>,
    ) -> (Vec<(ChunkKey, Vec<Tag>)>, HashSet<ChunkKey>) {
        self.spawn_loaded_entities();
        if self
            .common
            .dragon
            .as_ref()
            .is_some_and(|d| d.death_canceled && keys.contains(&chunk_of(d.position)))
        {
            let _ = self.save_entity_metadata();
        }
        self.snapshot_entity_chunks(keys, EntitySnapshot::Save)
    }
    pub fn remove_unloading_entities(&mut self, keys: &HashSet<ChunkKey>) {
        for key in keys {
            self.common.persist.unknown.remove(key);
        }
        self.unload_removed_entities(keys);
    }
}

impl crate::game::ServerGame<'_> {
    /// `Entity::save_nbt` for a mob: a rider is saved inside its root vehicle,
    /// in the root's chunk; a boat's or cart's passengers inside the vehicle.
    pub fn save_mob(&mut self, i: usize, keys: &HashSet<ChunkKey>) -> Option<(ChunkKey, Tag)> {
        let m = &self.common.mobs[i];
        // `Entity.shouldBeSaved`: a passenger saves only under its root, even
        // when that root is a boat or cart retained off-level and never saved.
        let saves = m
            .extra
            .mount
            .is_none_or(|vehicle| self.saved_vehicle_position(vehicle).is_none())
            && keys.contains(&chunk_of(m.position))
            && !m.removed;
        saves.then(|| (chunk_of(self.common.mobs[i].position), self.mob_tree_nbt(i)))
    }
    pub(in crate::game::server_side) fn save_item(
        &mut self,
        i: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        let at = self.common.dropped[i].position;
        keys.contains(&chunk_of(at)).then(|| {
            let id = self.common.dropped[i].id;
            let uuid = self.uuid_for(id);
            let mut tag = entity_nbt::item_entity_to_nbt(
                &self.common.dropped[i],
                uuid,
                self.common.persist.raw.get(&id),
                self.items(),
            );
            self.save_portal_cooldown(id, &mut tag);
            (chunk_of(at), tag)
        })
    }
    pub(in crate::game::server_side) fn save_orb(
        &mut self,
        i: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        let o = &self.common.orbs[i];
        keys.contains(&chunk_of(o.position)).then(|| {
            let raw = anon_raw(&self.common.persist, "minecraft:experience_orb", o.position);
            let uuid = saved_uuid(&self.common.persist, o.id);
            // Entity.saveWithoutId: the live fire, air and portal cooldown.
            let mut tag = with_raw_over(
                with_raw_over(entity_nbt::orb_to_nbt(o, uuid), raw.as_ref(), ORB_LIVE),
                self.common.persist.raw.get(&o.id),
                ORB_LIVE,
            );
            self.save_portal_cooldown(o.id, &mut tag);
            (chunk_of(o.position), tag)
        })
    }
    pub fn save_projectile(
        &mut self,
        i: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        let p = &self.common.creatures.projectiles[i];
        if !keys.contains(&chunk_of(p.position)) || !entity_nbt::saved_with_chunk(p) {
            return None;
        }
        // `ShulkerBullet`'s target, over the UUID bridge: a mob the level has
        // no UUID for writes no `Target` rather than a made-up one.
        let target = match &p.kind {
            sunrise_sim::projectiles::ProjectileKind::ShulkerBullet {
                target: Some(who), ..
            } => self.save_who_uuid(*who),
            _ => None,
        };
        let owner = p.owner_uuid;
        // Throwables own their typed Item and use id-keyed memory, never a position-keyed entry.
        let raw = if p.kind.is_throwable_item() {
            self.common.persist.raw.get(&p.id).cloned()
        } else {
            anon_raw(&self.common.persist, p.kind.id(), p.position)
        };
        let uuid = saved_uuid(&self.common.persist, p.id);
        let in_block = p
            .arrow
            .as_ref()
            .and_then(|a| a.last_state)
            .map(|s| self.common.state_to_nbt(s));
        let tag = entity_nbt::arrow_to_nbt(
            p,
            uuid,
            owner,
            in_block,
            raw.as_ref(),
            self.common.res.items(),
        )
        .or_else(|| {
            entity_nbt::fireball_to_nbt(p, uuid, owner, raw.as_ref(), self.common.res.items())
        })
        .or_else(|| {
            entity_nbt::throwable_to_nbt(p, uuid, owner, raw.as_ref(), self.common.res.items())
        })
        .or_else(|| entity_nbt::spit_or_bullet_to_nbt(p, uuid, owner, target, raw.as_ref()))?;
        // HasBeenShot and Fire are modeled for these projectiles. The generic raw
        // overlay also serves entity types whose fields are still opaque.
        let shot = tag.get("HasBeenShot").cloned();
        let fire = tag.get("Fire").cloned();
        let mut tag = with_raw(tag, self.common.persist.raw.get(&p.id));
        if let Tag::Compound(map) = &mut tag {
            if let Some(shot) = shot {
                map.insert("HasBeenShot".into(), shot);
            }
            if let Some(fire) = fire {
                map.insert("Fire".into(), fire);
            }
        }
        self.save_projectile_portal_cooldown(p, &mut tag);
        Some((chunk_of(p.position), tag))
    }
    /// The UUID a save writes for a reference to `who`, as `mob_to_nbt`'s
    /// `angry_at` does: the bridge's `who_uuid` over `EntityMemory::uuid_of`,
    /// which answers the UUID the entity was loaded or saved with. A mob the
    /// level has none for writes nothing.
    pub(in crate::game) fn save_who_uuid(&self, who: Who) -> Option<[i32; 4]> {
        entity_nbt::who_uuid(who, &mut |id| self.common.persist.uuid_of(id))
    }
    pub fn save_falling(&mut self, i: usize, keys: &HashSet<ChunkKey>) -> Option<(ChunkKey, Tag)> {
        let (id, pos) = (self.common.falling[i].id, self.common.falling[i].position);
        if !keys.contains(&chunk_of(pos)) {
            return None;
        }
        let uuid = if id != 0 {
            self.uuid_for(id)
        } else {
            random_uuid()
        };
        let f = &self.common.falling[i];
        let raw = match &f.saved {
            Some(saved) => Some(saved.1.clone()),
            None => anon_raw(&self.common.persist, "minecraft:falling_block", f.position),
        };
        let tag = entity_nbt::falling_block_to_nbt(
            self.state_to_nbt(f.state),
            f.position,
            f.velocity,
            f.age,
            f.drop_item,
            f.damage,
            f.fall_distance,
            f.ground.on_ground,
            f.rotation,
            uuid,
        );
        let mut tag = with_raw(tag, raw.as_ref());
        if let Tag::Compound(fields) = &mut tag {
            fields.insert("PortalCooldown".into(), Tag::Int(f.portal.cooldown));
            // Its simulated fire, not a saved copy's (`with_raw` keeps those).
            fields.insert("Fire".into(), Tag::Short(f.fire_ticks as i16));
        }
        Some((chunk_of(pos), tag))
    }
    pub(in crate::game::server_side) fn save_rocket(
        &mut self,
        i: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        let (id, at) = (self.common.rockets[i].id, self.common.rockets[i].position);
        keys.contains(&chunk_of(at)).then(|| {
            let uuid = self.uuid_for(id);
            let tag = self.common.rockets[i].to_nbt(
                uuid,
                self.common.level.player_uuid,
                self.common.persist.raw.get(&id),
                self.common.res.items(),
            );
            (chunk_of(at), tag)
        })
    }
    pub(in crate::game::server_side) fn save_trident(
        &mut self,
        i: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        let (id, at) = (self.common.tridents[i].id, self.common.tridents[i].position);
        keys.contains(&chunk_of(at)).then(|| {
            let (uuid, reg) = (self.uuid_for(id), self.common.pack.assets.registry);
            let tag = self.common.tridents[i].to_nbt(
                uuid,
                reg,
                self.common.persist.raw.get(&id),
                self.common.res.items(),
            );
            (chunk_of(at), tag)
        })
    }
    pub(in crate::game::server_side) fn save_tnt(
        &mut self,
        i: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        let at = self.common.tnts[i].position;
        keys.contains(&chunk_of(at))
            .then(|| (chunk_of(at), self.tnt_nbt(i)))
    }
    pub(in crate::game::server_side) fn save_boat(
        &mut self,
        i: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        let b = &self.common.boats[i];
        let (id, at) = (b.id, b.position);
        if !keys.contains(&chunk_of(at)) {
            return None;
        }
        let uuid = self.saved_uuid_or_random(id);
        let mut tag = b.to_nbt(
            uuid,
            self.common.persist.raw.get(&id),
            self.common.res.items(),
        );
        self.put_passengers(&mut tag, id);
        // AbstractBoat.addAdditionalSaveData: the lead's holder.
        self.write_boat_leash(&mut tag, i);
        self.save_portal_cooldown(id, &mut tag);
        Some((chunk_of(at), tag))
    }
    pub fn save_cart(&mut self, i: usize, keys: &HashSet<ChunkKey>) -> Option<(ChunkKey, Tag)> {
        let (id, at) = (self.common.carts[i].id, self.common.carts[i].position);
        if !keys.contains(&chunk_of(at)) {
            return None;
        }
        let uuid = self.saved_uuid_or_random(id);
        let mut tag = self.common.carts[i].to_nbt(
            uuid,
            self.common.persist.raw.get(&id),
            self.common.res.items(),
        );
        self.put_passengers(&mut tag, id);
        self.save_portal_cooldown(id, &mut tag);
        Some((chunk_of(at), tag))
    }
    pub(in crate::game::server_side) fn save_armor_stand(
        &mut self,
        i: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        let a = &self.common.armor_stands[i];
        let (id, key) = (a.id, chunk_of(a.position));
        if !keys.contains(&key) {
            return None;
        }
        let uuid = self.saved_uuid_or_random(id);
        let mut tag = entity_nbt::armor_stand_to_nbt(
            a,
            uuid,
            self.common.persist.raw.get(&id),
            self.common.res.items(),
        );
        self.put_passengers(&mut tag, id);
        self.save_portal_cooldown(id, &mut tag);
        Some((key, tag))
    }
    /// The UUID kept for `id`, or a new one that is not remembered (what a
    /// saved vehicle or stand gets when it never had one).
    fn saved_uuid_or_random(&self, id: u32) -> [i32; 4] {
        self.common
            .persist
            .uuids
            .get(&id)
            .copied()
            .unwrap_or_else(random_uuid)
    }
}

/// The mob layer's [`sunrise_client::mobs::WhoLookup`]: a warden's anger keeps
/// the suspects that left by UUID, and resolves the ones that are back through
/// the same map a save writes them from (`mob_to_nbt`'s `uuid_of`).
impl sunrise_sim::mobs::WhoLookup for ServerGame<'_> {
    fn who_by_uuid(&self, uuid: [i32; 4]) -> Option<Who> {
        crate::game::ServerGame::who_by_uuid(self, uuid)
    }

    fn mob_uuid(&self, id: u32) -> Option<[i32; 4]> {
        self.common.persist.uuids.get(&id).copied()
    }
}

/// A chunk's key, as `chunk_of` gives it.
pub type ChunkKey = (i32, i32);

/// The raw compound kept for an entity loaded at `at`, found by kind and position.
fn anon_raw(persist: &EntityMemory, kind: &str, at: Vec3) -> Option<Tag> {
    persist
        .anon
        .get(&anon_key(kind, at))
        .map(|(_, raw)| raw.clone())
}
/// The UUID entity `id` was loaded with or given, else a new one for this save.
fn saved_uuid(persist: &EntityMemory, id: u32) -> [i32; 4] {
    persist.uuids.get(&id).copied().unwrap_or_else(random_uuid)
}

/// Only a mob is finalized; another entity drops the request.
fn strip_finalize(tag: &mut Tag) {
    if let Tag::Compound(c) = tag {
        c.remove(&"sunrise:finalize".into());
    }
}
/// `riders` under `tag`'s `Passengers`, after the passengers it kept from
/// loading (`Entity.saveWithoutId`).
pub fn add_passengers(tag: &mut Tag, riders: Vec<Tag>) {
    if let Tag::Compound(c) = tag {
        let mut list = match c.get(&"Passengers".into()) {
            Some(Tag::List(l)) => l.clone(),
            _ => Vec::new(),
        };
        list.extend(riders);
        if list.is_empty() {
            c.remove(&"Passengers".into());
        } else {
            c.insert("Passengers".into(), Tag::List(list));
        }
    }
}
/// `Entity.getRootVehicle`: up the vehicles `vehicle_of` gives, from `id`.
fn root_vehicle(id: u32, vehicle_of: impl Fn(u32) -> Option<u32>) -> u32 {
    let mut root = id;
    // A bound against a loop of mounts.
    for _ in 0..16 {
        match vehicle_of(root) {
            Some(v) if v != root => root = v,
            _ => break,
        }
    }
    root
}
/// `ServerPlayer.saveParentVehicle`'s root: that of `vehicle`, while no
/// other player rides its tree (`Entity.hasExactlyOnePlayerPassenger`);
/// `others` are the vehicles the other players ride.
pub fn parent_vehicle_root(
    vehicle: u32,
    others: &[u32],
    vehicle_of: impl Fn(u32) -> Option<u32>,
) -> Option<u32> {
    let root = root_vehicle(vehicle, &vehicle_of);
    let shared = others.iter().any(|&o| root_vehicle(o, &vehicle_of) == root);
    (!shared).then_some(root)
}
/// RootVehicle {Attach: the mount's UUID, Entity: the root's tree}.
pub fn root_vehicle_tag(attach: [i32; 4], entity: Tag) -> Tag {
    Tag::compound([
        ("Attach", Tag::IntArray(attach.to_vec())),
        ("Entity", entity),
    ])
}
/// `Entity.getPassengersAndSelf` of `root`, from each rider's mount.
pub fn vehicle_tree(root: u32, mounts: &[(u32, Option<u32>)]) -> Vec<u32> {
    let mut tree = vec![root];
    let mut k = 0;
    while k < tree.len() {
        let v = tree[k];
        let riders: Vec<u32> = mounts
            .iter()
            .filter(|(id, m)| *m == Some(v) && !tree.contains(id))
            .map(|(id, _)| *id)
            .collect();
        tree.extend(riders);
        k += 1;
    }
    tree
}

/// Only a Mob or an AbstractBoat is Leashable: any other kind never reads
/// `leash`, so a summoned or loaded tag must not be kept and saved.
pub fn drop_unleashable_leash(tag: &mut Tag, kind: &str, is_mob: bool) {
    if !is_mob
        && sunrise_gameplay_common::game::boats_common::any_boat_wood(kind).is_none()
        && let Tag::Compound(c) = tag
    {
        c.remove(&"leash".into());
    }
}
/// `Entity.load`: a string `Team` adds the entity (`name`, its UUID) to that
/// team when the scoreboard has it; else vanilla only logs a warning.
pub fn join_saved_team(board: &mut Scoreboard, name: &str, tag: &Tag) -> bool {
    let Some(team) = text(tag.get("Team")) else {
        return false;
    };
    if board.team(&team).is_none() {
        log::warn!("Unable to add mob to team \"{team}\" (that team probably doesn't exist)");
        return false;
    }
    board.add_to_team(name, &team);
    true
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::entity_initialization_common::EntityInitializationCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;
