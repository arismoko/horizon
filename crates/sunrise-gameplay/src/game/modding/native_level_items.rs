//! Item entities and experience orbs a native mod adds to a level through a
//! `LevelRef` (docs/modding/src/how-to/native-mods.md, "Entity and level handles"): a block's
//! drop (`Block.popResource`), a plain item entity (`new ItemEntity` and
//! `addFreshEntity`) and experience (`ExperienceOrb.award`). They act where
//! the world is simulated, in the level in place: on the server thread, for a
//! server's players, and in-frame on the window's own level. Clients get the
//! new entities through the tracker, as any other.
use crate::game::ServerGame;
use mod_api::{EntityRef, LevelRef};
pub use sunrise_gameplay_common::game::spill_common as spill;
use sunrise_items::items::ItemStack;

impl ServerGame<'_> {
    /// Whether vanilla could make an item entity of `stack`: not empty
    /// (`ItemStack.isEmpty`) and of an item the registry has.
    pub(in crate::game::server_side) fn stack_spawns(&self, stack: &ItemStack) -> bool {
        stack.count > 0
            && &*stack.item != "minecraft:air"
            && self.common.res.items().get(&stack.item).is_some()
    }
    /// `spawn_item` of `stack` and the new item entity as a handle, unless a
    /// mod kept it out (`native_entity_join_level`). It is found by its id,
    /// since the hooks the spawn runs may add entities after it.
    fn spawn_level_item(
        &mut self,
        level: LevelRef,
        stack: ItemStack,
        at: [f64; 3],
        motion: Option<[f64; 3]>,
        pickup_delay: i32,
    ) -> Option<EntityRef> {
        // `spawn_item` takes the next id.
        let id = self.server.next_entity + 1;
        self.add_item_entity(stack, at, motion, pickup_delay);
        self.common
            .dropped
            .iter()
            .rev()
            .any(|d| d.id == id)
            .then(|| EntityRef::new(level.epoch(), id))
    }
    /// Drops `stack` as a broken block drops it (`Block.popResource`): an
    /// item entity at a random point within a quarter block of the middle of
    /// `pos` (three draws from the level random, made for an empty stack and
    /// with `block_drops` off too), with its own random motion and the
    /// default pickup delay of 10 ticks. None, with nothing spawned, for an
    /// empty stack or an unknown item, with the `block_drops` rule off, on a client,
    /// for a stale handle, a level not in place, once the entity ids run out (the level random is
    /// not drawn from), or an item a mod kept out (`native_entity_join_level`).
    pub fn level_pop_resource(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        stack: ItemStack,
    ) -> Option<EntityRef> {
        if !self.level_in_place(level) || self.entity_ids_exhausted() {
            return None;
        }
        let at = spill::pop_position(&mut self.common.world.random, pos);
        // Block.popResource's private overload checks these after the draws.
        if !self.stack_spawns(&stack)
            || !self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::BLOCK_DROPS)
        {
            return None;
        }
        self.spawn_level_item(level, stack, at, None, 10)
    }
    /// Spawns `stack` as an item entity at `x y z` (`new ItemEntity(level, x,
    /// y, z, stack)` and `addFreshEntity`): its own random motion (up 0.2, up
    /// to 0.1 either way across), with `pickup_delay` ticks before it can be
    /// picked up, as `ItemEntity.setPickupDelay` at construction (vanilla
    /// passes 10 for a block's drop, 40 for a player's throw and 32767 for
    /// never). None, with nothing
    /// spawned, for an empty stack or an unknown item, a value that isn't
    /// finite, on a client, for a stale handle, a level not in place, or an
    /// item a mod kept out (`native_entity_join_level`).
    pub fn level_spawn_item(
        &mut self,
        level: LevelRef,
        x: f64,
        y: f64,
        z: f64,
        stack: ItemStack,
        pickup_delay: i32,
    ) -> Option<EntityRef> {
        self.spawn_item_at(level, x, y, z, stack, None, pickup_delay)
    }
    /// `spawn_item` with the item entity's motion set to
    /// `motion` (blocks a tick), as `new ItemEntity(level, x, y, z, stack,
    /// deltaX, deltaY, deltaZ)`, and the same `pickup_delay` ticks before it
    /// can be picked up (`ItemEntity.setPickupDelay`). None in the same
    /// cases, and for a motion that isn't finite.
    pub fn level_spawn_item_with_motion(
        &mut self,
        level: LevelRef,
        x: f64,
        y: f64,
        z: f64,
        stack: ItemStack,
        motion: [f64; 3],
        pickup_delay: i32,
    ) -> Option<EntityRef> {
        self.spawn_item_at(level, x, y, z, stack, Some(motion), pickup_delay)
    }
    fn spawn_item_at(
        &mut self,
        level: LevelRef,
        x: f64,
        y: f64,
        z: f64,
        stack: ItemStack,
        motion: Option<[f64; 3]>,
        pickup_delay: i32,
    ) -> Option<EntityRef> {
        if self.entity_ids_exhausted() {
            return None;
        }
        let finite = [x, y, z]
            .into_iter()
            .chain(motion.into_iter().flatten())
            .all(f64::is_finite);
        if !finite || !self.level_in_place(level) || !self.stack_spawns(&stack) {
            return None;
        }
        self.spawn_level_item(level, stack, [x, y, z], motion, pickup_delay)
    }
    /// Awards `amount` experience at `x y z` as vanilla's `ExperienceOrb.award`
    /// does: orbs of the largest sizes first (2477, 1237, ... 3, 1), each
    /// drawing the id `tryMergeToExisting` compares from the level random and
    /// merging into an orb of the same value whose box meets the 1 block box
    /// around `x y z`, or else a new orb with its own random motion. The
    /// `block_drops` rule doesn't apply (vanilla's blocks check it before
    /// they award). True once the orbs are awarded, even if a mod keeps a new
    /// one out (`native_entity_join_level`). False, with no orbs, for 0, a
    /// value that isn't finite, on a client, for a stale handle or a level
    /// not in place.
    pub fn level_award_experience(
        &mut self,
        level: LevelRef,
        x: f64,
        y: f64,
        z: f64,
        amount: u32,
    ) -> bool {
        if amount == 0 || ![x, y, z].iter().all(|v| v.is_finite()) || !self.level_in_place(level) {
            return false;
        }
        self.spawn_orbs([x, y, z], amount);
        true
    }
}

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
