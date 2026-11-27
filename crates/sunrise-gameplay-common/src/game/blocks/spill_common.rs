use super::GameLevel;
use super::api_values::{TryIntoApi, TryIntoEngine};
use std::collections::HashMap;
use sunrise_core::position::BlockPos;
use sunrise_core::random::RandomSource;
use sunrise_game_data::block_entity_nbt;
use sunrise_items::items::{ItemStack, Items};
use sunrise_level::containers::BlockEntity;
use sunrise_sim::{block_entity_item, furnace::FurnaceCooking};
use {
    sunrise_core::nbt::{Compound, Tag},
    sunrise_world::{
        level::{LevelEvent, removal, removal::PreRemove},
        registry::BlockStateId,
    },
};

/// Sources retained before removal; the table alone decides which components copy.
#[derive(Default)]
pub struct Pending {
    sources: HashMap<[i32; 3], std::sync::Arc<sunrise_world::loot::copy::BlockSource>>,
    /// A decorated pot's sherds for its dynamic drop, not component-copy policy.
    pub(in crate::game) pot: HashMap<[i32; 3], [Option<ItemStack>; 4]>,
}

pub fn source_snapshot(
    mut saved: Tag,
    name: Option<Tag>,
    lock: Option<&Tag>,
) -> sunrise_world::loot::copy::BlockSource {
    if let Tag::Compound(fields) = &mut saved {
        if let Some(name) = name {
            fields.insert("CustomName".into(), name);
        }
        if let Some(lock) = lock {
            fields.insert("lock".into(), lock.clone());
        }
    }
    let mut nbt = block_entity_nbt::canonical(&saved).unwrap_or(saved);
    // `saveWithFullMetadata` writes no chunk bookkeeping: `keepPacked` is the chunk's.
    if let Tag::Compound(fields) = &mut nbt {
        fields.remove(&"keepPacked".into());
    }
    let components = block_entity_item::collected_components(&nbt);
    sunrise_world::loot::copy::BlockSource { nbt, components }
}

/// Captures the live, saved or empty block entity for a loot roll.
pub fn world_source(
    world: &sunrise_level::world::World,
    pos: [i32; 3],
    block: &str,
    items: &Items,
    encode: impl FnOnce((i32, i32, i32), &BlockEntity, &str) -> Tag,
) -> Option<std::sync::Arc<sunrise_world::loot::copy::BlockSource>> {
    let key = (pos[0], pos[1], pos[2]);
    let saved = match world.block_entities.get(&key) {
        Some(entity) => {
            live_full_metadata(world, key, entity, block, items, encode(key, entity, block))
        }
        None => saved_entity(world, pos).or_else(|| {
            // Sparse storage can omit an empty block entity, but its type's
            // implicit components still belong to the loot source. Use the
            // factory's entity identity, which need not match the block's id.
            sunrise_game_data::block_entity_types::type_for(block)
                .or_else(|| sunrise_sim::lan::chunks::block_entity_type(block))
                .map(|kind| {
                    Tag::compound([
                        ("id", Tag::String(kind.into())),
                        ("x", Tag::Int(pos[0])),
                        ("y", Tag::Int(pos[1])),
                        ("z", Tag::Int(pos[2])),
                    ])
                })
        })?,
    };
    Some(std::sync::Arc::new(source_snapshot(
        saved,
        world.block_names.get(&key).cloned(),
        world.block_lock(key),
    )))
}

pub fn live_full_metadata(
    world: &sunrise_level::world::World,
    key: (i32, i32, i32),
    entity: &BlockEntity,
    block: &str,
    items: &Items,
    current: Tag,
) -> Tag {
    let original = world
        .chunks
        .get(&(key.0 >> 4, key.2 >> 4))
        .and_then(|chunk| chunk.saved.as_deref())
        .and_then(|saved| saved.block_entity_sources.iter().find(|(p, _)| *p == key))
        .map(|(_, original)| original)
        .filter(|original| block_entity_nbt::id(original) == block_entity_nbt::id(&current));
    match original {
        Some(original) => {
            sunrise_sim::save::overlay_block_entity_source(original, entity, block, &current, items)
        }
        None => current,
    }
}

impl Pending {
    pub fn source(
        &self,
        pos: [i32; 3],
    ) -> Option<std::sync::Arc<sunrise_world::loot::copy::BlockSource>> {
        self.sources.get(&pos).cloned()
    }
    pub fn record_from(&mut self, world: &sunrise_level::world::World, pos: [i32; 3], saved: Tag) {
        let key = (pos[0], pos[1], pos[2]);
        let name = world.block_names.get(&key).cloned();
        self.record_source(pos, source_snapshot(saved, name, world.block_lock(key)));
    }
    pub fn record_saved(&mut self, pos: [i32; 3], tag: &sunrise_core::nbt::Tag) {
        self.record_source(pos, source_snapshot(tag.clone(), None, None));
    }
    pub fn record_source(&mut self, pos: [i32; 3], source: sunrise_world::loot::copy::BlockSource) {
        let source = std::sync::Arc::new(source);
        self.sources.insert(pos, source.clone());
        let tag = &source.nbt;
        if tag.get("id") != Some(&Tag::String("minecraft:decorated_pot".into())) {
            return;
        }
        let sherds = tag.get("sherds");
        let side = |name: &str| match sherds.and_then(|s| s.get(name)) {
            Some(Tag::String(id)) => Some(ItemStack {
                item: std::sync::Arc::from(id.to_string_lossy().as_str()),
                count: 1,
                damage: 0,
                extra: None,
            }),
            Some(t) => sunrise_sim::save::item_from_nbt(t),
            None => None,
        };
        self.pot.insert(
            pos,
            [side("back"), side("left"), side("right"), side("front")],
        );
    }
    /// The block entity's dynamic drops: a decorated pot's sherds, left,
    /// back, front then right (`DecoratedPotBlock.getDrops`).
    pub fn dynamic(&self, pos: [i32; 3]) -> Vec<ItemStack> {
        self.pot
            .get(&pos)
            .map_or_else(Vec::new, |[back, left, right, front]| {
                [left, back, front, right]
                    .into_iter()
                    .flatten()
                    .cloned()
                    .collect()
            })
    }
    pub fn clear(&mut self, pos: [i32; 3]) {
        self.pot.remove(&pos);
        self.sources.remove(&pos);
    }
}

/// Constructor random retained while drop hooks edit a not-yet-joined item.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemRandom {
    pub own: (u64, Option<f64>),
    pub uuid: [i32; 4],
    pub bob: f32,
    pub yaw: f32,
    pub velocity: [f64; 3],
    /// The state a drop hook gave the item entity; the default is a fresh one's.
    pub state: DropState,
}

/// Item-entity state a drop hook can set (`ItemDrop`'s `thrower`, `target`, `no_gravity` and
/// `age`). The default is what `new ItemEntity` starts with.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DropState {
    pub thrower: Option<u128>,
    pub target: Option<u128>,
    pub no_gravity: bool,
    pub age: i32,
}

impl DropState {
    /// The state `drop` asks for.
    pub(in crate::game) fn of(drop: &mod_api::ItemDrop) -> Self {
        Self {
            thrower: drop.thrower,
            target: drop.target,
            no_gravity: drop.no_gravity,
            age: drop.age,
        }
    }
    /// `drop` with this state.
    pub(in crate::game) fn to_api(self, drop: mod_api::ItemDrop) -> mod_api::ItemDrop {
        drop.with_thrower(self.thrower)
            .with_target(self.target)
            .with_no_gravity(self.no_gravity)
            .with_age(self.age)
    }
    /// `item` as a fresh item entity with this state (`setThrower`, `setTarget`,
    /// `setNoGravity` and its age).
    pub fn apply(
        self,
        item: sunrise_sim::entities::ItemEntity,
    ) -> sunrise_sim::entities::ItemEntity {
        let uuid = sunrise_sim::lan::playerdata::uuid_ints;
        sunrise_sim::entities::ItemEntity {
            thrower: self.thrower.map(uuid),
            target: self.target.map(uuid),
            no_gravity: self.no_gravity,
            age: self.age,
            ..item
        }
    }
}

/// The one ItemEntity constructor random sequence, shared by immediate and
/// deferred spawning: UUID, bob offset, yaw, then optional default motion.
pub fn prepare_item_random(velocity: Option<[f64; 3]>) -> ItemRandom {
    let mut own = sunrise_level::random_source::entity();
    let uuid = sunrise_sim::save::insecure_uuid(&mut own);
    let bob = own.next_float() * std::f32::consts::PI * 2.0;
    let yaw = own.next_float() * 360.0;
    let velocity = velocity.unwrap_or_else(|| {
        let x = own.next_double() * 0.2 - 0.1;
        [x, 0.2, own.next_double() * 0.2 - 0.1]
    });
    ItemRandom {
        own: (&own).into(),
        uuid,
        bob,
        yaw,
        velocity,
        state: DropState::default(),
    }
}

/// An item entity to add: where, its motion (None: `new ItemEntity`'s own
/// random motion) and its pickup delay.
pub struct Spawn {
    pub random: Option<ItemRandom>,
    pub stack: ItemStack,
    pub pos: [f64; 3],
    pub motion: Option<[f64; 3]>,
    pub delay: i32,
}

/// The block-entity context bound to a block break at the moment the break begins, so a
/// replacement block entity placed at the same position before drops fire (for example by a
/// `player_destroyed_block` listener) never leaks into `block_drops` or `exp_drop`.
#[derive(Clone, Copy)]
pub enum BlockDropEntity<'a> {
    /// No block entity belongs to this break.
    None,
    /// A block entity that was resident at `pos` with this exact generation when the break
    /// started. Resolves to a live `BlockEntityRef` only while that same generation remains
    /// resident at `pos`.
    Resident(u64, Option<&'a Compound>),
    /// Read-only saved data (including `CustomName`) of a removed or packed block entity.
    /// Never resolves a resident handle at `pos`.
    Saved(&'a Compound),
}

impl<'a> BlockDropEntity<'a> {
    pub fn at(
        world: &sunrise_level::world::World,
        key: (i32, i32, i32),
        data: Option<&'a Compound>,
    ) -> Self {
        if world.block_entities.get(&key).is_some()
            && let Some(generation) = world.block_entities.generation(&key)
        {
            Self::Resident(generation, data)
        } else {
            data.map_or(Self::None, Self::Saved)
        }
    }
}

/// What a broken block spawns (`GameLevel::block_dropped`).
/// Prepared engine item entity; API snapshots exist only while a listener runs.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockItemDrop {
    pub random: Option<ItemRandom>,
    pub stack: ItemStack,
    pub position: [f64; 3],
    pub velocity: [f64; 3],
    pub pickup_delay: i32,
    /// The item-entity state a drop hook set.
    pub state: DropState,
}

impl BlockItemDrop {
    /// The drop as the mods hear and edit it.
    pub(in crate::game) fn from_api(drop: &mod_api::ItemDrop, stack: ItemStack) -> Self {
        Self {
            random: None,
            stack,
            position: drop.position,
            velocity: drop.velocity,
            pickup_delay: drop.pickup_delay,
            state: DropState::of(drop),
        }
    }

    /// The constructor random the item entity joins with, carrying the state a hook set. A drop
    /// with no state set keeps its random as it was (none: drawn when it joins).
    pub fn random_with_state(&self) -> Option<ItemRandom> {
        if self.state == DropState::default() {
            return self.random;
        }
        let mut random = self
            .random
            .unwrap_or_else(|| prepare_item_random(Some(self.velocity)));
        random.state = self.state;
        Some(random)
    }
}

struct EnginePreparedDrops<'a> {
    items: &'a mut Vec<BlockItemDrop>,
    items_registry: &'a Items,
}

impl mod_api::PreparedBlockDropsAccess for EnginePreparedDrops<'_> {
    #[inline]
    fn len(&self) -> usize {
        self.items.len()
    }

    #[inline]
    fn stack_metadata(
        &self,
        index: usize,
    ) -> Option<(mod_api::ItemKey, std::num::NonZeroU32, Option<u32>)> {
        crate::game::api_values::stack_read_metadata(&self.items.get(index)?.stack).ok()
    }

    #[inline]
    fn project_stack(
        &self,
        index: usize,
        metadata: (mod_api::ItemKey, std::num::NonZeroU32, Option<u32>),
    ) -> Option<mod_api::ItemStack> {
        crate::game::api_values::project_stack(&self.items.get(index)?.stack, metadata).ok()
    }

    #[inline]
    fn position(&self, index: usize) -> Option<[f64; 3]> {
        self.items.get(index).map(|item| item.position)
    }

    #[inline]
    fn velocity(&self, index: usize) -> Option<[f64; 3]> {
        self.items.get(index).map(|item| item.velocity)
    }

    #[inline]
    fn pickup_delay(&self, index: usize) -> Option<i32> {
        self.items.get(index).map(|item| item.pickup_delay)
    }

    #[inline]
    fn thrower(&self, index: usize) -> Option<Option<u128>> {
        self.items.get(index).map(|item| item.state.thrower)
    }

    #[inline]
    fn target(&self, index: usize) -> Option<Option<u128>> {
        self.items.get(index).map(|item| item.state.target)
    }

    #[inline]
    fn no_gravity(&self, index: usize) -> Option<bool> {
        self.items.get(index).map(|item| item.state.no_gravity)
    }

    #[inline]
    fn age(&self, index: usize) -> Option<i32> {
        self.items.get(index).map(|item| item.state.age)
    }

    #[inline]
    fn set_stack(
        &mut self,
        index: usize,
        stack: mod_api::ItemStack,
    ) -> Result<bool, mod_api::StackWriteError> {
        if index >= self.items.len() {
            return Ok(false);
        }
        let converted = stack.try_into_engine(self.items_registry)?;
        self.items[index].stack = converted;
        Ok(true)
    }

    #[inline]
    fn set_position(&mut self, index: usize, position: [f64; 3]) -> bool {
        let Some(item) = self.items.get_mut(index) else {
            return false;
        };
        item.position = position;
        true
    }

    #[inline]
    fn set_velocity(&mut self, index: usize, velocity: [f64; 3]) -> bool {
        let Some(item) = self.items.get_mut(index) else {
            return false;
        };
        item.velocity = velocity;
        true
    }

    #[inline]
    fn set_pickup_delay(&mut self, index: usize, pickup_delay: i32) -> bool {
        let Some(item) = self.items.get_mut(index) else {
            return false;
        };
        item.pickup_delay = pickup_delay;
        true
    }

    #[inline]
    fn set_thrower(&mut self, index: usize, thrower: Option<u128>) -> bool {
        let Some(item) = self.items.get_mut(index) else {
            return false;
        };
        item.state.thrower = thrower;
        true
    }

    #[inline]
    fn set_target(&mut self, index: usize, target: Option<u128>) -> bool {
        let Some(item) = self.items.get_mut(index) else {
            return false;
        };
        item.state.target = target;
        true
    }

    #[inline]
    fn set_no_gravity(&mut self, index: usize, no_gravity: bool) -> bool {
        let Some(item) = self.items.get_mut(index) else {
            return false;
        };
        item.state.no_gravity = no_gravity;
        true
    }

    #[inline]
    fn set_age(&mut self, index: usize, age: i32) -> bool {
        let Some(item) = self.items.get_mut(index) else {
            return false;
        };
        item.state.age = age;
        true
    }

    #[inline]
    fn push(&mut self, drop: mod_api::ItemDrop) -> Result<(), mod_api::StackWriteError> {
        self.items.push(drop.try_into_engine(self.items_registry)?);
        Ok(())
    }

    #[inline]
    fn insert(
        &mut self,
        index: usize,
        drop: mod_api::ItemDrop,
    ) -> Result<bool, mod_api::StackWriteError> {
        if index > self.items.len() {
            return Ok(false);
        }
        self.items
            .insert(index, drop.try_into_engine(self.items_registry)?);
        Ok(true)
    }

    #[inline]
    fn replace(
        &mut self,
        index: usize,
        drop: mod_api::ItemDrop,
    ) -> Result<bool, mod_api::StackWriteError> {
        if index >= self.items.len() {
            return Ok(false);
        }
        self.items[index] = drop.try_into_engine(self.items_registry)?;
        Ok(true)
    }

    #[inline]
    fn remove(&mut self, index: usize) -> bool {
        if index >= self.items.len() {
            return false;
        }
        self.items.remove(index);
        true
    }

    #[inline]
    fn clear(&mut self) {
        self.items.clear();
    }

    #[inline]
    fn truncate(&mut self, len: usize) {
        self.items.truncate(len);
    }

    #[inline]
    fn swap(&mut self, a: usize, b: usize) -> bool {
        if a >= self.items.len() || b >= self.items.len() {
            return false;
        }
        self.items.swap(a, b);
        true
    }
}

pub struct BlockDrops {
    /// The item entities to add, as the mods left them.
    pub items: Vec<BlockItemDrop>,
    /// The experience points to award at the block's centre.
    pub experience: u32,
    /// A mod canceled the break's drops: `spawnAfterBreak` doesn't run.
    pub canceled: bool,
}

#[derive(Default)]
pub struct Spill {
    pub piston_positions: Vec<super::piston_carry_common::PositionChange>,
    pub spawns: Vec<Spawn>,
    /// Experience orbs (`ExperienceOrb.award`): where and how much, each
    /// and the merge id it drew from the level random.
    pub orbs: Vec<([f64; 3], u32, i32)>,
    /// Block entities the behaviour removed, for the game's own follow-ups
    /// (open screens, playing songs).
    pub removed: Vec<([i32; 3], BlockEntity)>,
    /// Runs of `spawns` a player's use popped (a picked berry bush), for the
    /// mods' `native_loot_drops`.
    pub used: Vec<UsedDrops>,
}

/// `spawns[start..end]` came from `table` at the block `pos` in `state`.
pub struct UsedDrops {
    pub start: usize,
    pub end: usize,
    pub pos: [i32; 3],
    pub state: BlockStateId,
    pub table: &'static str,
}

impl Spill {
    pub fn is_empty(&self) -> bool {
        self.piston_positions.is_empty()
            && self.spawns.is_empty()
            && self.orbs.is_empty()
            && self.removed.is_empty()
            && self.used.is_empty()
    }
}

/// `Block.popResource`: a spot around the block's centre from the level random.
pub fn pop_position(
    random: &mut impl sunrise_core::random::RandomSource,
    pos: [i32; 3],
) -> [f64; 3] {
    let mut offset = || -0.25 + random.next_double() * 0.5;
    // EntityTypes.ITEM is 0.25 high.
    let half_height = f64::from(0.25f32) / 2.0;
    [
        f64::from(pos[0]) + 0.5 + offset(),
        f64::from(pos[1]) + 0.5 + offset() - half_height,
        f64::from(pos[2]) + 0.5 + offset(),
    ]
}

/// `Block.popResource` draws the level offsets even when the rule prevents
/// constructing (or capturing) an item entity.
pub fn admitted_pop_position(
    random: &mut impl RandomSource,
    pos: [i32; 3],
    block_drops: bool,
) -> Option<[f64; 3]> {
    let at = pop_position(random, pos);
    block_drops.then_some(at)
}

/// `Block.popResourceFromFace`'s position and motion: an item on the clicked
/// face, moved off that face and jittered across the other two axes.
pub fn pop_position_from_face(
    random: &mut impl sunrise_core::random::RandomSource,
    pos: [i32; 3],
    face: [i32; 3],
) -> ([f64; 3], [f64; 3]) {
    fn jitter<R: sunrise_core::random::RandomSource>(random: &mut R, min: f64, max: f64) -> f64 {
        min + random.next_double() * (max - min)
    }
    // EntityTypes.ITEM is 0.25 wide and 0.25 high.
    let (half_width, half_height) = (0.125, 0.125);
    let x = f64::from(pos[0])
        + 0.5
        + if face[0] == 0 {
            jitter(random, -0.25, 0.25)
        } else {
            f64::from(face[0]) * (0.5 + half_width)
        };
    let y = f64::from(pos[1])
        + 0.5
        + if face[1] == 0 {
            jitter(random, -0.25, 0.25)
        } else {
            f64::from(face[1]) * (0.5 + half_height)
        }
        - half_height;
    let z = f64::from(pos[2])
        + 0.5
        + if face[2] == 0 {
            jitter(random, -0.25, 0.25)
        } else {
            f64::from(face[2]) * (0.5 + half_width)
        };
    let dx = if face[0] == 0 {
        jitter(random, -0.1, 0.1)
    } else {
        f64::from(face[0]) * 0.1
    };
    let dy = if face[1] == 0 {
        jitter(random, 0.0, 0.1)
    } else {
        f64::from(face[1]) * 0.1 + 0.1
    };
    let dz = if face[2] == 0 {
        jitter(random, -0.1, 0.1)
    } else {
        f64::from(face[2]) * 0.1
    };
    ([x, y, z], [dx, dy, dz])
}

/// The block entity at `pos` Sunrise keeps as it was loaded, if any
/// (campfires, decorated pots, chiseled bookshelves, shelves).
pub fn saved_entity(
    world: &sunrise_level::world::World,
    pos: [i32; 3],
) -> Option<sunrise_core::nbt::Tag> {
    use sunrise_core::nbt::Tag;
    let saved = world
        .chunks
        .get(&(pos[0] >> 4, pos[2] >> 4))?
        .saved
        .as_ref()?;
    saved
        .block_entities
        .iter()
        .find(|t| {
            t.get("x") == Some(&Tag::Int(pos[0]))
                && t.get("y") == Some(&Tag::Int(pos[1]))
                && t.get("z") == Some(&Tag::Int(pos[2]))
        })
        .cloned()
}

/// The saved block entity's metadata omits the chunk's `keepPacked`.
pub fn saved_full_metadata(world: &sunrise_level::world::World, pos: [i32; 3]) -> Option<Tag> {
    let mut tag = saved_entity(world, pos)?;
    if let Tag::Compound(fields) = &mut tag {
        fields.remove(&"keepPacked".into());
    }
    Some(tag)
}

/// The read-only payload of an entity kept in packed chunk data, not its loader metadata.
pub fn packed_block_entity_payload(tag: Tag) -> Option<Compound> {
    let Tag::Compound(mut data) = tag else {
        return None;
    };
    data.remove(&"keepPacked".into());
    Some(crate::game::api_values_common::block_entity_payload(data))
}

/// Keep `tag` as the block entity at its position Sunrise does not model,
/// replacing the one there.
pub fn put_saved_entity(
    world: &mut sunrise_level::world::World,
    pos: [i32; 3],
    tag: sunrise_core::nbt::Tag,
) {
    set_saved_entity(world, pos, Some(tag));
}

/// Restore a saved-only block entity, or remove the tag created by a tentative write.
pub fn set_saved_entity(
    world: &mut sunrise_level::world::World,
    pos: [i32; 3],
    tag: Option<sunrise_core::nbt::Tag>,
) {
    use sunrise_core::nbt::Tag;
    let Some(chunk) = world.chunks.get_mut(&(pos[0] >> 4, pos[2] >> 4)) else {
        return;
    };
    let chunk = std::sync::Arc::make_mut(chunk);
    let saved = match tag {
        Some(_) => chunk.saved.get_or_insert_default(),
        None => match &mut chunk.saved {
            Some(saved) => saved,
            None => return,
        },
    };
    let rows = &mut std::sync::Arc::make_mut(saved).block_entities;
    let at = rows.iter().position(|t| {
        t.get("x") == Some(&Tag::Int(pos[0]))
            && t.get("y") == Some(&Tag::Int(pos[1]))
            && t.get("z") == Some(&Tag::Int(pos[2]))
    });
    match (at, tag) {
        (Some(index), Some(tag)) => rows[index] = tag,
        (Some(index), None) => {
            rows.remove(index);
        }
        (None, Some(tag)) => rows.push(tag),
        (None, None) => {}
    }
}

/// How to finish the removal after its pre-removal effects have run.
pub enum RemovingBlockEntity {
    Saved,
    Resident {
        generation: Option<u64>,
        kind: PreRemove,
        neighbor_updates: u8,
    },
    OwnTick,
}

/// Admitted effects from the retiring row's Parts, in the order each phase will apply them.
pub(in crate::game) struct RemovalEffects<'a> {
    items: &'a Items,
    keep_contents: bool,
    drops: Vec<Option<ItemStack>>,
    spawns: Vec<(ItemStack, [f64; 3])>,
    experience: Vec<(u32, [f64; 3])>,
}

impl RemovalEffects<'_> {
    fn checked_stack(
        &self,
        stack: &mod_api::ItemStack,
    ) -> Result<ItemStack, mod_api::BlockRemovalEffectError> {
        let stack = stack
            .try_into_engine(self.items)
            .map_err(mod_api::BlockRemovalEffectError::Stack)?;
        if &*stack.item == "minecraft:air" {
            return Err(mod_api::BlockRemovalEffectError::Stack(
                mod_api::StackWriteError::EmptyEngineStack,
            ));
        }
        Ok(stack)
    }

    /// Roll back only the effects written by the Part that failed.
    pub(in crate::game) fn register_listener(
        &mut self,
        listen: &mut dyn FnMut(&mut dyn mod_api::BlockRemovalEffects) -> bool,
    ) {
        let (keep, drops, spawns, experience) = (
            self.keep_contents,
            self.drops.len(),
            self.spawns.len(),
            self.experience.len(),
        );
        if !listen(self) {
            self.keep_contents = keep;
            self.drops.truncate(drops);
            self.spawns.truncate(spawns);
            self.experience.truncate(experience);
        }
    }
}

impl mod_api::BlockRemovalEffects for RemovalEffects<'_> {
    fn keep_contents(&self) -> bool {
        self.keep_contents
    }

    fn set_keep_contents(&mut self, keep: bool) {
        self.keep_contents = keep;
    }

    fn drop_slot(
        &mut self,
        stack: Option<mod_api::ItemStack>,
    ) -> Result<(), mod_api::BlockRemovalEffectError> {
        let stack = stack
            .as_ref()
            .map(|stack| self.checked_stack(stack))
            .transpose()?;
        self.drops.push(stack);
        Ok(())
    }

    fn spawn_item(
        &mut self,
        stack: mod_api::ItemStack,
        position: [f64; 3],
    ) -> Result<(), mod_api::BlockRemovalEffectError> {
        if !position.into_iter().all(f64::is_finite) {
            return Err(mod_api::BlockRemovalEffectError::NonFinitePosition);
        }
        let stack = self.checked_stack(&stack)?;
        self.spawns.push((stack, position));
        Ok(())
    }

    fn award_experience(
        &mut self,
        amount: u32,
        position: [f64; 3],
    ) -> Result<(), mod_api::BlockRemovalEffectError> {
        if !position.into_iter().all(f64::is_finite) {
            return Err(mod_api::BlockRemovalEffectError::NonFinitePosition);
        }
        if amount != 0 {
            self.experience.push((amount, position));
        }
        Ok(())
    }
}

impl GameLevel<'_> {
    fn pop_resource(&mut self, pos: [i32; 3], stack: ItemStack) {
        if let Some(at) = admitted_pop_position(&mut self.world.random, pos, self.rules.block_drops)
        {
            self.spill.spawns.push(Spawn {
                random: None,
                stack,
                pos: at,
                motion: None,
                delay: 10,
            });
        }
    }

    /// `Block.popResource(level, pos, stack)` with the broken block's copied components.
    /// `Block.popResource(level, pos, new ItemStack(block))`.
    pub(in crate::game) fn pop_block_item(&mut self, pos: BlockPos, state: BlockStateId) {
        let item = self.facts.get(state).item.clone();
        if &*item != "minecraft:air" {
            self.pop_resource([pos.x, pos.y, pos.z], ItemStack::new(&item, 1));
        }
    }
    /// An item `atLowerCornerWithOffset(pos, 0.5, 1.01, 0.5).offsetRandomXZ(random,
    /// 0.7)` with the default pickup delay (`ComposterBlock.extractProduce`).
    pub(in crate::game) fn pop_above(&mut self, pos: BlockPos, item: std::sync::Arc<str>) {
        let at = removal::jukebox_pop_position(&mut self.world.random, pos);
        self.spill.spawns.push(Spawn {
            random: None,
            stack: ItemStack::new(&item, 1),
            pos: at,
            motion: None,
            delay: 10,
        });
    }
    /// `Block.dropFromBlockInteractLootTable` with `Block.popResource` of
    /// every stack.
    pub(in crate::game) fn drop_interact_loot(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        table: &'static str,
    ) {
        let loot = self.loot.clone();
        let matcher = |predicate: &serde_json::Value, entity: &sunrise_world::loot::LootEntity| {
            loot.as_ref()
                .is_some_and(|data| data.entity_matches(predicate, Some(entity)))
        };
        let interacting = self.interacting.as_ref().map(|entity| {
            (
                entity,
                &matcher as &dyn Fn(&serde_json::Value, &sunrise_world::loot::LootEntity) -> bool,
            )
        });
        // `LocationCheck` reads from ORIGIN, the block's centre, through the
        // advancement path's `LocationPredicate` over this level's own state.
        // The roll draws from a copy of the level random, which the location
        // reads of the world leave no hand on.
        let origin = [pos.x, pos.y, pos.z];
        let centre = origin.map(|c| f64::from(c) + 0.5);
        let source = world_source(
            self.world,
            origin,
            &self.facts.get(state).name,
            self.items,
            |key, entity, block| self.encode_live_block_entity(key, entity, block),
        );
        let (block_entity, block_components) = match source {
            Some(source) => (Some(source.nbt.clone()), Some(source.components.clone())),
            None => (None, None),
        };
        let place = sunrise_world::loot::LootPlace {
            origin: Some(centre),
            block_entity,
            block_components,
            storage: Some(self.loot_storage.clone()),
            level: Some(sunrise_world::loot::LootLevel {
                dimension: self.location.dimension.to_owned(),
                game_time: self.world.game_time,
                day_time: self.world.day_time,
                raining: self.raining,
                thundering: self.thundering,
                clocks: self.loot_clocks.clone(),
            }),
        };
        let level = super::location_world::LevelLocation {
            world: &*self.world,
            registry: self.registry,
            facts: self.facts,
            biomes: self.biomes,
            items: self.items,
            collision: self.collision,
            hoppers: &*self.positions.hoppers,
            raining: self.raining,
            thundering: self.thundering,
            refs: self.location,
        };
        let read = |predicate: &serde_json::Value, at: [f64; 3]| {
            sunrise_sim::advancements::location(predicate, at, &level)
        };
        let mut random = self.world.random.clone();
        let drops = sunrise_world::loot::with_native_loot_place(place, || {
            self.items.interact_loot(
                table,
                self.registry,
                state,
                interacting,
                self.interacting_tool.as_ref(),
                Some(sunrise_sim::advancements::loot_location(centre, &read)),
                &mut random,
                self.rand_sequences,
                self.seed,
            )
        });
        self.world.random = random;
        let start = self.spill.spawns.len();
        for stack in drops {
            if &*stack.item != "minecraft:air" && stack.count > 0 {
                self.pop_resource([pos.x, pos.y, pos.z], stack);
            }
        }
        self.spill.used.push(UsedDrops {
            start,
            end: self.spill.spawns.len(),
            pos: [pos.x, pos.y, pos.z],
            state,
            table,
        });
    }
    /// `Block.dropResources(state, level, pos, blockEntity)`: the block's loot
    /// (from its random sequence, else the level random), with what its block
    /// entity copies into it, popped at once. While a mod's machine breaks
    /// the block (`native_machine_destroy_block`), the loot is rolled with its
    /// `drop_tool` (`dropResources(.., entity, tool)`) and, given `drop_into`,
    /// kept there instead of popped.
    pub fn drop_resources_now(&mut self, pos: BlockPos, state: BlockStateId) {
        let key = (pos.x, pos.y, pos.z);
        let at = [pos.x, pos.y, pos.z];
        self.unpack_loot(super::container_loot_level::LootTarget::Block(key), None);
        let packed_data = if let Some(entity) = self.world.block_entities.get(&key) {
            let name = &self.facts.get(state).name;
            let current = self.encode_live_block_entity(key, entity, name);
            let saved = live_full_metadata(self.world, key, entity, name, self.items, current);
            self.pending.record_from(self.world, at, saved);
            None
        } else if let Some(tag) = saved_entity(self.world, at) {
            self.pending.record_saved(at, &tag);
            if self.native_authoritative
                && mod_api::__private::listened(mod_api::HookId::BlockDrops)
            {
                packed_block_entity_payload(tag)
            } else {
                None
            }
        } else {
            None
        };
        let block_entity = BlockDropEntity::at(self.world, key, packed_data.as_ref());
        let dynamic = self.pending.dynamic(at);
        let machine = self
            .drop_tool
            .as_ref()
            .is_some_and(|(target, _)| *target == key);
        let tool = self
            .drop_tool
            .as_ref()
            .filter(|_| machine)
            .and_then(|(_, tool)| tool.clone());
        let roll = sunrise_items::items::BlockLootRoll {
            held: tool.as_ref(),
            dynamic: &dynamic,
            breaker: self.drop_breaker_at == Some(pos),
            ..Default::default()
        };
        let entity = self
            .drop_entity
            .filter(|_| self.drop_breaker_at == Some(pos));
        let drops = self.roll_block_loot(pos, state, entity, roll, None);
        let dropped =
            self.block_dropped(pos, state, None, tool.as_ref(), block_entity, drops, None);
        for item in dropped.items {
            if let Some(kept) = self.drop_into.as_mut().filter(|_| machine) {
                kept.push(item.stack);
            } else {
                self.spill.spawns.push(Spawn {
                    random: item.random_with_state(),
                    stack: item.stack,
                    pos: item.position,
                    motion: Some(item.velocity),
                    delay: item.pickup_delay,
                });
            }
        }
        self.pending.clear(at);
        if dropped.experience > 0 {
            let centre = [
                f64::from(pos.x) + 0.5,
                f64::from(pos.y) + 0.5,
                f64::from(pos.z) + 0.5,
            ];
            self.award_orbs(centre, dropped.experience);
        }
    }
    /// Sample before applying the tool's enchantments, which spawners ignore.
    pub fn block_break_experience(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        breaker: Option<u128>,
        tool: Option<&ItemStack>,
        block_entity: BlockDropEntity<'_>,
    ) -> u32 {
        let name = self.facts.get(state).name.clone();
        let Some(amount) = self
            .part_break_experience(pos, state, breaker, tool, block_entity)
            .or_else(|| removal::break_experience(&mut self.world.random, &name))
        else {
            return 0;
        };
        if &*name == "minecraft:spawner" {
            return amount;
        }
        let ctx = sunrise_items::enchant::EffectContext {
            tool: tool.map(|stack| stack.item.as_ref()),
            items: Some(self.items),
            ..Default::default()
        };
        let experience = self.items.enchantments.modify_value(
            tool,
            "minecraft:block_experience",
            &ctx,
            &mut self.world.random,
            amount as f32,
        );
        (experience as i32).max(0) as u32
    }

    /// What `Block.dropResources` does with the loot it rolled for the block
    /// `state` at `pos`: `Block.popResource` builds an item entity for each
    /// stack (its position drawn from the level random, its motion from the
    /// item's own), then `spawnAfterBreak` samples the block's experience,
    /// in that order. The mods then see the list (`block_drops`, NeoForge's
    /// `BlockDropsEvent`). Nothing is spawned here: the caller adds the
    /// entities and orbs, and a canceled break leaves both empty. The random
    /// draws happen with or without a listener, in vanilla's order.
    pub fn block_dropped(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        breaker: Option<u128>,
        tool: Option<&ItemStack>,
        block_entity: BlockDropEntity<'_>,
        stacks: Vec<ItemStack>,
        exp_to_drop: Option<u32>,
    ) -> BlockDrops {
        let mut items = stacks
            .into_iter()
            .filter(|stack| &*stack.item != "minecraft:air" && stack.count > 0)
            .filter_map(|stack| {
                let position = admitted_pop_position(
                    &mut self.world.random,
                    [pos.x, pos.y, pos.z],
                    self.rules.block_drops,
                )?;
                let random = prepare_item_random(None);
                Some(BlockItemDrop {
                    random: Some(random),
                    stack,
                    position,
                    velocity: random.velocity,
                    pickup_delay: 10,
                    state: DropState::default(),
                })
            })
            .collect();
        // An explicit global override is applied as given.
        let mut experience = exp_to_drop.unwrap_or_else(|| {
            self.block_break_experience(pos, state, breaker, tool, block_entity)
        });
        let mut canceled = false;
        if self.native_authoritative
            && mod_api::__private::listened(mod_api::HookId::BlockDrops)
            && let Ok(api_state) = state.try_into_api()
        {
            let tool_metadata = tool.and_then(|stack| {
                if stack.count == 0 || &*stack.item == "minecraft:air" {
                    return None;
                }
                match crate::game::api_values::stack_read_metadata(stack) {
                    Ok(metadata) => Some(metadata),
                    Err(error) => {
                        log::warn!(
                            "block_drops at {pos:?}: omitting unrepresentable tool '{}' (count {}): {error:?}",
                            stack.item,
                            stack.count,
                        );
                        None
                    }
                }
            });
            let (block_entity_ref, breaker_entity) =
                self.drop_event_refs(pos, breaker, block_entity);
            let block_entity_data = match block_entity {
                BlockDropEntity::Saved(data) => Some(data),
                BlockDropEntity::Resident(_, data) if block_entity_ref.is_none() => data,
                BlockDropEntity::None | BlockDropEntity::Resident(..) => None,
            };
            let project_tool = |metadata| {
                tool.and_then(|stack| crate::game::api_values::project_stack(stack, metadata).ok())
            };
            let tool_view = tool_metadata
                .map(|metadata| mod_api::ItemStackView::resident(metadata, &project_tool));
            let mut drops_access = EnginePreparedDrops {
                items: &mut items,
                items_registry: self.items,
            };
            self.native_block_drops(mod_api::__private::BlockDropsCall {
                dimension: self.dimension,
                pos,
                state: api_state,
                breaker,
                breaker_entity,
                block_entity: block_entity_ref,
                block_entity_data,
                tool: tool_view,
                drops: mod_api::PreparedBlockDrops::new(&mut drops_access),
                experience: &mut experience,
                cancel: &mut canceled,
            });
        }
        if canceled || !self.rules.block_drops {
            items.clear();
            experience = 0;
        }
        BlockDrops {
            items,
            experience,
            canceled,
        }
    }
    /// `ExperienceOrb.award`: orbs of the largest sizes, each drawing the id
    /// `tryMergeToExisting` compares from the level random.
    pub(in crate::game) fn award_orbs(&mut self, at: [f64; 3], mut amount: u32) {
        while amount > 0 {
            let value = removal::orb_value(amount);
            amount -= value;
            let merge_id = self.world.random.next_int_bound(40).unwrap_or(0);
            self.spill.orbs.push((at, value, merge_id));
        }
    }
    /// `BlockEntity.preRemoveSideEffects` and `removeBlockEntity` for the block
    /// entity of `old` at `pos`. A block entity Sunrise never created reads as
    /// vanilla's fresh one: an empty container still draws a spot per slot.
    pub fn remove_entity(&mut self, pos: BlockPos, old: BlockStateId, side_effects: bool) -> u8 {
        let removal = self.pre_remove_entity(pos, old, side_effects);
        self.finish_remove_entity(pos, old, removal)
    }

    pub fn pre_remove_entity(
        &mut self,
        pos: BlockPos,
        old: BlockStateId,
        side_effects: bool,
    ) -> RemovingBlockEntity {
        let key = (pos.x, pos.y, pos.z);
        self.unpack_loot(super::container_loot_level::LootTarget::Block(key), None);
        // `Containers.dropItemStack` splits the original stack before adding the item.
        // A saved-only row has no live slots to drain. Take its backing data here, so a
        // join listener that replaces this cell again cannot decode the same stack twice.
        if let Some(tag) = self.take_saved_entity(pos) {
            self.positions.hoppers.remove(&key);
            if side_effects {
                self.drop_saved_contents(pos, old, tag);
            }
            return RemovingBlockEntity::Saved;
        }
        // Sculk blocks and creaking hearts notice their removal on their own tick.
        if matches!(
            self.world.block_entities.get(&key),
            Some(BlockEntity::Sculk(_) | BlockEntity::CreakingHeart(_))
        ) {
            return RemovingBlockEntity::OwnTick;
        }
        let generation = self.world.block_entities.generation(&key);
        // A mod's block entity answers its parts in place (its slot, so its
        // attachments, stay until `put`), as vanilla's override runs on `this`.
        let mut effects = RemovalEffects {
            items: self.items,
            keep_contents: false,
            drops: Vec::new(),
            spawns: Vec::new(),
            experience: Vec::new(),
        };
        if side_effects
            && matches!(
                self.world.block_entities.get(&key),
                Some(BlockEntity::Mod(_))
            )
            && mod_api::__private::has_part_hook(mod_api::HookId::PreRemoveSideEffects)
            && let Some((lease, BlockEntity::Mod(entity))) = self.world.block_entities.take(&key)
        {
            let data =
                super::native_hooks::block_entity_read::ResidentReadBlockEntity::stored(&entity);
            self.native_pre_remove_side_effects(
                pos,
                old,
                mod_api::BlockEntityDataView::new(&data),
                &mut effects,
            );
            self.world
                .block_entities
                .put(lease, BlockEntity::Mod(entity));
        }
        let entity = self.world.block_entities.get(&key);
        let name = self.facts.get(old).name.clone();
        // BaseContainerBlockEntity.preRemoveSideEffects: Containers.dropContents
        // visits every declared slot, even if its empty storage hasn't been materialized.
        let kind = if let Some(BlockEntity::Mod(m)) = entity
            && let Some(container) = self.world.block_entities.mod_container(old.0)
        {
            PreRemove::Contents {
                slots: container.slots.max(m.items.len()),
                furnace: false,
            }
        } else {
            removal::pre_remove(&name)
        };
        let mut neighbour_updates = 0;
        if side_effects {
            match kind {
                PreRemove::Contents { slots, furnace } => {
                    for slot in 0..if effects.keep_contents { 0 } else { slots } {
                        let stack = self
                            .world
                            .block_entities
                            .get_mut(&key)
                            .and_then(|row| row.slots_mut().get_mut(slot))
                            .and_then(Option::take);
                        self.drop_item_stack(pos, stack);
                    }
                    // AbstractFurnaceBlockEntity.preRemoveSideEffects: then the
                    // experience of what it made, at the block's centre.
                    if furnace
                        && let Some(BlockEntity::Furnace(f)) = self.world.block_entities.get(&key)
                    {
                        let at = [
                            f64::from(pos.x) + 0.5,
                            f64::from(pos.y) + 0.5,
                            f64::from(pos.z) + 0.5,
                        ];
                        for (count, experience) in f.recipe_experience(self.recipes) {
                            let xp = sunrise_level::containers::create_experience(
                                count,
                                experience,
                                &mut self.world.random,
                            );
                            self.award_orbs(at, xp);
                        }
                    }
                }
                PreRemove::Campfire => {
                    let items = match self.world.block_entities.get_mut(&key) {
                        Some(BlockEntity::Campfire(c)) => std::mem::take(&mut c.items),
                        _ => Default::default(),
                    };
                    for stack in items {
                        self.drop_item_stack(pos, stack);
                    }
                }
                PreRemove::Jukebox => {
                    // popOutTheItem: removeTheItem stops a playing song (its
                    // events and a neighbour update), the record pops, and
                    // onSongChanged updates the neighbours again.
                    if let Some(BlockEntity::Jukebox(j)) = self.world.block_entities.get_mut(&key)
                        && let Some(record) = j.items[0].take()
                    {
                        if j.playing {
                            self.jukebox_stopped(pos, old);
                            neighbour_updates += 1;
                        }
                        let at = removal::jukebox_pop_position(&mut self.world.random, pos);
                        self.spawn_removal_item(record, at);
                        neighbour_updates += 1;
                    }
                }
                PreRemove::Lectern => {
                    let props = self.registry.state(old).map(|s| &s.properties);
                    let has_book = props
                        .and_then(|p| p.get("has_book"))
                        .is_some_and(|v| v == "true");
                    let step = match props.and_then(|p| p.get("facing")).map(String::as_str) {
                        Some("north") => [0, -1],
                        Some("south") => [0, 1],
                        Some("west") => [-1, 0],
                        _ => [1, 0],
                    };
                    if has_book
                        && let Some(BlockEntity::Lectern(l)) =
                            self.world.block_entities.get_mut(&key)
                        && let Some(book) = l.book.take()
                    {
                        self.spawn_removal_item(book, removal::lectern_book_position(pos, step));
                    }
                }
                PreRemove::ShulkerBox
                | PreRemove::SculkShrieker
                | PreRemove::CreakingHeart
                | PreRemove::Beacon
                | PreRemove::Nothing => {}
            }
            for stack in effects.drops {
                self.drop_item_stack(pos, stack);
            }
            for (stack, at) in effects.spawns {
                self.spawn_removal_item(stack, at);
            }
            for (amount, at) in effects.experience {
                self.award_orbs(at, amount);
            }
        }
        RemovingBlockEntity::Resident {
            generation,
            kind,
            neighbor_updates: neighbour_updates,
        }
    }

    pub fn finish_remove_entity(
        &mut self,
        pos: BlockPos,
        old: BlockStateId,
        removal: RemovingBlockEntity,
    ) -> u8 {
        let key = (pos.x, pos.y, pos.z);
        // A drop's join listener can replace the row, even with one for the same block state.
        // Don't unregister that row or clear its metadata; only finish the pre-removal updates.
        if let RemovingBlockEntity::Resident {
            generation,
            neighbor_updates,
            ..
        } = removal
            && self.world.block_entities.generation(&key) != generation
        {
            return neighbor_updates;
        }
        // Headless levels cannot rely on a renderer to discard a replaced gateway.
        if &*self.facts.get(old).name == "minecraft:end_gateway" {
            self.positions.gateways.remove(&[pos.x, pos.y, pos.z]);
        }
        let (kind, neighbour_updates) = match removal {
            RemovingBlockEntity::OwnTick | RemovingBlockEntity::Saved => return 0,
            RemovingBlockEntity::Resident {
                kind,
                neighbor_updates,
                ..
            } => (kind, neighbor_updates),
        };
        let entity = self.world.block_entities.remove(&key);
        self.positions.hoppers.remove(&key);
        self.world.block_names.remove(&key);
        self.world.forget_block_entity_source(key);
        // BlockEntity.setRemoved: a jukebox always reports its song stopping,
        // a beacon plays its deactivation.
        match kind {
            PreRemove::Jukebox => self.jukebox_stopped(pos, old),
            PreRemove::Beacon => self.events.push(LevelEvent::Sound {
                pos,
                sound: "minecraft:block.beacon.deactivate".into(),
                source: None,
                volume: 1.0,
                pitch: 1.0,
            }),
            _ => {}
        }
        if let Some(entity) = entity {
            self.spill.removed.push(([pos.x, pos.y, pos.z], entity));
        }
        neighbour_updates
    }
    /// ItemEntity(level, x, y, z, stack), followed by setDefaultPickUpDelay.
    fn spawn_removal_item(&mut self, stack: ItemStack, pos: [f64; 3]) {
        self.spill.spawns.push(Spawn {
            random: None,
            stack,
            pos,
            motion: None,
            delay: 10,
        });
    }
    /// The jukebox's `JUKEBOX_STOP_PLAY` game event and level event 1011.
    fn jukebox_stopped(&mut self, pos: BlockPos, state: BlockStateId) {
        self.events.push(LevelEvent::GameEvent {
            pos,
            event: "minecraft:jukebox_stop_play",
            state: Some(state),
        });
        self.events.push(LevelEvent::LevelEvent {
            id: 1011,
            pos,
            data: 0,
            source: None,
        });
    }
    fn take_saved_entity(&mut self, pos: BlockPos) -> Option<Tag> {
        let chunk = self.world.chunks.get_mut(&(pos.x >> 4, pos.z >> 4))?;
        let index = chunk
            .saved
            .as_ref()?
            .block_entities
            .iter()
            .position(|tag| {
                tag.get("x") == Some(&Tag::Int(pos.x))
                    && tag.get("y") == Some(&Tag::Int(pos.y))
                    && tag.get("z") == Some(&Tag::Int(pos.z))
            })?;
        let saved = std::sync::Arc::make_mut(chunk).saved.as_mut()?;
        Some(std::sync::Arc::make_mut(saved).block_entities.remove(index))
    }

    /// Spill the contents of a block entity still backed by its saved compound.
    fn drop_saved_contents(
        &mut self,
        pos: BlockPos,
        old: BlockStateId,
        tag: sunrise_core::nbt::Tag,
    ) {
        let name = self.facts.get(old).name.clone();
        let items = match removal::pre_remove(&name) {
            PreRemove::Contents { slots, .. } if &*name == "minecraft:decorated_pot" => {
                // Containers.dropContents reads getTheItem, which unpacks the loot table.
                let mut tag = tag;
                super::decorated_pot::unpack_loot(
                    &mut tag,
                    self.loot.as_deref(),
                    &mut self.world.random,
                    self.rand_sequences,
                    self.seed,
                    self.items,
                    sunrise_world::loot::ContainerLootContext::default(),
                );
                let mut items = vec![None; slots];
                items[0] = tag.get("item").and_then(sunrise_sim::save::item_from_nbt);
                items
            }
            PreRemove::Contents { slots, .. } => {
                sunrise_sim::save::decode_items(tag.get("Items"), slots)
            }
            PreRemove::Campfire => sunrise_sim::save::decode_items(tag.get("Items"), 4),
            _ => return,
        };
        for stack in items {
            self.drop_item_stack(pos, stack);
        }
    }
    /// `Containers.dropItemStack` of one slot (an empty one still draws its spot).
    pub fn drop_item_stack(&mut self, pos: BlockPos, stack: Option<ItemStack>) {
        let stack = stack.filter(|s| &*s.item != "minecraft:air");
        let count = stack.as_ref().map_or(0, |s| s.count);
        for piece in removal::drop_item_stack(&mut self.world.random, pos, count) {
            let Some(stack) = &stack else { break };
            self.spill.spawns.push(Spawn {
                random: None,
                stack: ItemStack {
                    count: piece.count,
                    ..stack.clone()
                },
                pos: piece.pos,
                motion: Some(piece.motion),
                delay: 0,
            });
        }
    }
}
