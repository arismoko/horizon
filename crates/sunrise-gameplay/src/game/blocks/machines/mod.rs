//! Item-moving blocks: `HopperBlockEntity` (push, pull, item entities,
//! transfer cooldowns), `DispenserBlock` / `DropperBlock.dispenseFrom` with
//! the vanilla dispense behaviours Sunrise has entities for, and container
//! comparator signals (`AbstractContainerMenu.getRedstoneSignalFromContainer`).
pub(in crate::game) mod loot_access;
use crate::game::ServerGame;
use sunrise_core::nbt::Tag;
use sunrise_gameplay_common::game::machine_containers::{
    Containers, EntitySnapshot, Kind, PartSnapshot, PlacementAttempt, PlacementQuery,
    TakeThroughFace, View, bone_meal_stack,
};
use sunrise_gameplay_common::game::spill_common as spill;
#[cfg(test)]
mod can_take_item_through_face_tests;
pub mod cauldron_contents;
pub(in crate::game) mod crafter;
pub(in crate::game) mod native_energy;
pub(in crate::game) mod native_fluids;
pub(in crate::game) mod native_items;
use mod_api::{
    self, BlockSide, Capability, ENTITY_AUTOMATION, EntityRef, ItemResource, ResourceHandler,
};
use std::{borrow::Cow, ops::ControlFlow};
use sunrise_core::{geometry::Direction, position::BlockPos, random::RandomSource};
use sunrise_game_data::Result;
use sunrise_items::items::ItemStack;
#[cfg(test)]
use sunrise_level::containers::ModContainer;
use sunrise_level::containers::{BlockEntity, HOPPER_SLOTS};
use sunrise_sim::save;
use sunrise_world::{level::LevelAccess, registry::BlockStateId};

#[allow(unused_imports)]
pub(in crate::game) use sunrise_gameplay_common::hoppers::{
    HopperState, ItemColumns, StampedItemColumns, cat_sitting_above, dir_from_name, offset, room,
    shelf_signal,
};
#[allow(unused_imports)]
pub(in crate::game) use sunrise_gameplay_common::hoppers::{Pos, hopper_cooldown_in};

/// What item automation reaches among the entities at its target (`ENTITY_AUTOMATION`).
pub(in crate::game) enum AutomationEntity {
    /// A container entity the game answers for, by id: reached as a vanilla container.
    Container(u32),
    /// An entity a mod's provider answers for, with its handler.
    Part(EntityRef, Capability<dyn ResourceHandler<ItemResource>>),
}

/// A block or entity destination, reached through its slots or a mod's handler.
pub(in crate::game) enum ContainerTarget<'a> {
    View(&'a View),
    Part(Capability<dyn ResourceHandler<ItemResource>>),
}

/// Wakes the sleeping hoppers whose suck area (`Hopper.SUCK_AABB`) `b` reaches.
pub(in crate::game::server_side) fn wake_hoppers_reaching(
    entities: &mut sunrise_level::block_entities::BlockEntities,
    b: sunrise_core::geometry::Aabb,
) {
    let (lo, hi) = (b.min_position(), b.max_position());
    for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
        for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
            for y in (lo.y.floor() as i32).saturating_sub(1)..=hi.y.floor() as i32 {
                entities.wake_hopper(&(x, y, z));
            }
        }
    }
}

impl ServerGame<'_> {
    pub fn hopper_cooldown(&self, pos: Pos) -> i32 {
        hopper_cooldown_in(&self.server.hoppers, &self.common.world, pos)
    }
}

#[derive(Clone, Copy)]
pub(in crate::game) enum Insertion {
    Automation(Option<Pos>),
    /// The pull transaction keeps just the receiving hopper's slot.
    Pull,
    /// A capability write may need to keep the whole entity.
    Capability,
}

#[derive(PartialEq, Eq)]
pub(in crate::game) enum MoveIn {
    /// The offer was refused or had no room; a pull can try the next slot.
    Skipped,
    /// A fitting slot failed to take the stack; a pull must abort.
    Blocked,
    Moved,
}

/// The API snapshots for one take transfer (a hopper's pull, one extract): the source
/// entity, read again when a write changes its revision, and its block state, converted
/// again only when the state changes. Each asked slot's own stack is converted per ask.
#[derive(Default)]
pub struct TakeAttempt {
    entity: Option<EntitySnapshot>,
    /// The engine state last converted, and its API form (None when it can't be represented).
    state: Option<(BlockStateId, Option<mod_api::BlockState>)>,
}

/// A take query that may ask a mod container's part: the game hosting the query, and the
/// snapshots of the transfer it belongs to.
pub struct TakeAsk<'q, 'g> {
    pub game: &'q ServerGame<'g>,
    pub attempt: &'q mut TakeAttempt,
}

impl TakeAttempt {
    /// A mod container's `can_take_item_through_face` answer for `slot` of `entity` at
    /// `pos`, when its type listens: `None` keeps the declared output slots, and an entity,
    /// state or stack the API cannot represent refuses the take.
    fn ask(
        &mut self,
        game: &ServerGame<'_>,
        pos: Pos,
        entity: &sunrise_level::containers::ModBlockEntity,
        slot: usize,
        stack: &ItemStack,
        face: Direction,
    ) -> Option<bool> {
        use sunrise_gameplay_common::game::api_values::TryIntoApi;

        let snapshot = match EntitySnapshot::current(
            &mut self.entity,
            mod_api::HookId::CanTakeItemThroughFace,
            pos,
            entity,
        ) {
            PartSnapshot::Unsubscribed => return None,
            PartSnapshot::Unrepresentable => return Some(false),
            PartSnapshot::Ready(snapshot) => snapshot,
        };
        let (x, y, z) = pos;
        let unreadable = |what: &dyn std::fmt::Debug| {
            log::debug!("Sunrise: can_take_item_through_face could not read {what:?} at {pos:?}");
        };
        let block = game.common.world.block(x, y, z);
        let state = match self.state {
            Some((seen, state)) if seen == block => state,
            _ => {
                let state = block.try_into_api().inspect_err(|e| unreadable(e)).ok();
                self.state = Some((block, state));
                state
            }
        };
        let Some(state) = state else {
            return Some(false);
        };
        let Ok(offered) = stack.try_into_api().inspect_err(|e| unreadable(e)) else {
            return Some(false);
        };
        let mut allowed = None;
        game.native_can_take_item_through_face(
            snapshot,
            mod_api::BlockPos::new(x, y, z),
            state,
            slot,
            &offered,
            face,
            &mut allowed,
        );
        allowed
    }
}

impl TakeThroughFace for TakeAsk<'_, '_> {
    fn ask_take(
        self,
        pos: Pos,
        entity: &sunrise_level::containers::ModBlockEntity,
        slot: usize,
        stack: &ItemStack,
        face: Direction,
    ) -> Option<bool> {
        self.attempt.ask(self.game, pos, entity, slot, stack, face)
    }
}

#[cfg(test)]
pub(in crate::game::server_side) trait ViewTestSlots {
    fn slots_to_add(&self, face: Option<Direction>) -> Vec<usize>;
}

#[cfg(test)]
impl ViewTestSlots for View {
    fn slots_to_add(&self, face: Option<Direction>) -> Vec<usize> {
        self.slots_to_add_iter(face).collect()
    }
}

impl ServerGame<'_> {
    pub fn containers(&self) -> Containers<'_> {
        Containers {
            world: &self.common.world,
            behaviours: self.common.res.behaviours(),
            items: self.items(),
            registry: self.common.pack.assets.registry,
            brewing: Some(&self.stations().brewing),
            mobs: &self.common.mobs,
        }
    }
    pub(in crate::game) fn stored_container_signal(&self, pos: Pos) -> i32 {
        // Match a GameLevel view; the level's block entities own the slots.
        Containers {
            brewing: None,
            ..self.containers()
        }
        .stored_container_signal(pos)
    }
    pub(in crate::game::server_side) fn block_can_place(
        &self,
        pos: [i32; 3],
        slot: usize,
        stack: &ItemStack,
    ) -> bool {
        let containers = self.containers();
        let Some(view) = containers.view((pos[0], pos[1], pos[2]), false) else {
            return false;
        };
        view.kind == Kind::Shulker
            || containers.can_place(
                &view,
                slot,
                stack,
                PlacementQuery::Offer(&mut PlacementAttempt::default()),
            )
    }
    pub fn slot_mut_at(&mut self, view: &View, slot: usize) -> Option<&mut Option<ItemStack>> {
        let (pos, i) = view.locate(slot)?;
        let entity = self.common.world.block_entities.get_mut(&pos)?;
        entity.slot_mut(i)
    }
    /// `BlockEntity.setChanged` for every block entity of a container.
    pub fn view_changed(&mut self, view: &View) {
        // A composter's containers change its block state instead.
        if let Kind::Composter(_) = view.kind {
            return;
        }
        for (pos, _) in view.parts() {
            self.container_changed(pos);
            // `ShelfBlockEntity.setChanged()` (the no-argument override posts
            // `BLOCK_ACTIVATE`): a container writing a shelf's slot is heard.
            if matches!(
                self.world.block_entities.get(&pos),
                Some(BlockEntity::Shelf(_))
            ) {
                let state = self.common.world.block(pos.0, pos.1, pos.2);
                self.post_block_game_event(
                    "minecraft:block_activate",
                    [pos.0, pos.1, pos.2],
                    None,
                    Some(state),
                );
            }
        }
    }
    /// `BlockEntity.setChanged` at `pos`: comparators re-read the container.
    pub fn container_changed(&mut self, pos: Pos) {
        if pos.1 == super::minecarts::CART_KEY_Y {
            return;
        }
        if mod_api::__private::has_scoped_hook(mod_api::HookId::ContentsChanged)
            && let Some(level) = self.native_level_here()
        {
            let at = BlockPos::new(pos.0, pos.1, pos.2);
            self.native_contents_changed(
                level,
                at,
                self.common.world.block(pos.0, pos.1, pos.2),
                None,
            );
        }
        self.container_changed_without_hook(pos);
    }

    #[allow(dead_code)]
    pub(in crate::game) fn server_container_changed(&mut self, pos: Pos) {
        self.container_changed(pos);
    }

    /// `BlockEntity.setChanged` updates without dispatching `contents_changed` again.
    pub(in crate::game) fn container_changed_without_hook(&mut self, pos: Pos) {
        if pos.1 == super::minecarts::CART_KEY_Y {
            return;
        }
        self.chiseled_bookshelf_synced(pos);
        if matches!(
            self.world.block_entities.get(&pos),
            Some(BlockEntity::Shelf(_))
        ) {
            self.mark_block_entity_updated(pos);
        }
        // A double chest's other half changes with it.
        let other = self.common.chest_other_half(pos);
        for pos in std::iter::once(pos).chain(other) {
            // A hopper under this container reads it: a change here is one for it.
            self.common.world.block_entities.wake_below(&pos);
            self.common.world.modified.insert((pos.0 >> 4, pos.2 >> 4));
            self.with_level(|b, level, ticks| {
                b.container_changed(level, ticks, BlockPos::new(pos.0, pos.1, pos.2))
            });
        }
        // A write outside any transaction (a hopper, `/item replace`): committed writes are
        // reported once at their root commit instead.
        if self.transaction_lifecycle() == mod_api::TransactionLifecycle::None {
            self.notify_block_menu_slot_changed(pos, |_| None);
        }
    }
    /// `HopperBlockEntity.tryMoveInItem`.
    pub(in crate::game::server_side) fn try_move_in_item(
        &mut self,
        insertion: Insertion,
        view: &View,
        stack: ItemStack,
        slot: usize,
        attempt: &mut PlacementAttempt,
    ) -> (Option<ItemStack>, MoveIn) {
        let (rest, result) = self.move_in_item(insertion, view, stack, slot, attempt);
        if result == MoveIn::Moved {
            self.view_changed(view);
            // JukeboxBlockEntity.setTheItem through a hopper's or a dropper's
            // transfer: the disc it lands on starts playing.
            if view.kind == Kind::Jukebox {
                let (x, y, z) = view.first.0;
                self.jukebox_inserted([x, y, z]);
            }
        }
        (rest, result)
    }
    /// `tryMoveInItem` before `setChanged`: ask about the full offer, then keep and
    /// write the admitted slot. A skipped slot is never journaled.
    pub(in crate::game::server_side) fn move_in_item(
        &mut self,
        insertion: Insertion,
        view: &View,
        mut stack: ItemStack,
        slot: usize,
        attempt: &mut PlacementAttempt,
    ) -> (Option<ItemStack>, MoveIn) {
        let c = self.containers();
        if !c.can_place(view, slot, &stack, PlacementQuery::Offer(attempt)) {
            return (Some(stack), MoveIn::Skipped);
        }
        // Capability operations use the same saved slot as hoppers, but may first
        // access a pot that still has loot. Discovery never reaches this write path.
        if view.kind == Kind::Pot && matches!(insertion, Insertion::Capability) {
            let pos = view.first.0;
            let at = [pos.0, pos.1, pos.2];
            if spill::saved_entity(&self.common.world, at)
                .is_some_and(|tag| tag.get("LootTable").is_some())
            {
                self.journal_saved_pot(pos);
                self.journal_block_loot_random();
                if let Some(tag) = self.pot_tag(at) {
                    spill::put_saved_entity(&mut self.common.world, at, tag);
                    self.after_commit(sunrise_gameplay_common::game::transactions::After::Changed(
                        pos,
                    ));
                }
            }
        }
        let c = self.containers();
        let (from_hopper, count) = match insertion {
            Insertion::Automation(from_hopper) => (from_hopper, stack.count),
            Insertion::Pull | Insertion::Capability => {
                let count = c.insertion_room(view, slot, &stack).min(stack.count);
                if count == 0 {
                    return (Some(stack), MoveIn::Skipped);
                }
                if matches!(insertion, Insertion::Pull) {
                    let Some((pos, index)) = view.locate(slot) else {
                        return (Some(stack), MoveIn::Blocked);
                    };
                    self.journal_block_entity_slot(pos, index);
                } else {
                    self.journal_view_slot(view, slot);
                }
                (None, count)
            }
        };
        self.unpack_inventory_slot(view, slot);
        let was_empty = self.inventory_is_empty(view);
        let c = self.containers();
        let vanilla_max = c.max_stack(&stack);
        let override_limit = view
            .locate(slot)
            .and_then(|(pos, _)| c.world.container_stack_limit(pos));
        let restricted = override_limit.is_some_and(|limit| limit < vanilla_max);
        let max = override_limit.map_or(vanilla_max, |limit| vanilla_max.min(limit));
        if restricted && max == 0 {
            return (Some(stack), MoveIn::Blocked);
        }

        let mut success = false;
        let mut filled_empty = false;
        let mut pot = if view.kind == Kind::Pot {
            let pos = view.first.0;
            let Some(tag) = spill::saved_entity(&self.common.world, [pos.0, pos.1, pos.2])
                .or_else(|| self.common.pot_data([pos.0, pos.1, pos.2]))
            else {
                return (Some(stack), MoveIn::Blocked);
            };
            let item = tag.get("item").and_then(save::item_from_nbt);
            Some((tag, item))
        } else {
            None
        };
        let target = if let Some((_, item)) = &mut pot {
            item
        } else {
            let Some(target) = self.slot_mut_at(view, slot) else {
                return (Some(stack), MoveIn::Blocked);
            };
            target
        };
        let beyond_slot = stack.count - count;
        stack.count = count;
        // An empty stack a mod left in its slot: the slot is free (`isEmpty`).
        if target.as_ref().is_some_and(|s| s.count == 0) {
            *target = None;
        }
        match target {
            None => {
                // HopperBlockEntity.setItem limits the accepted stack, then
                // tryMoveInItem consumes the input even when it exceeded that limit.
                *target = Some(ItemStack {
                    count: stack.count.min(max),
                    ..stack.clone()
                });
                stack.count = if restricted { stack.count.saturating_sub(max) } else { 0 };
                success = true;
                filled_empty = true;
            }
            Some(current)
                // HopperBlockEntity.canMergeItems
                if current.count <= max && current.same_item_same_components(&stack) =>
            {
                let space = max.saturating_sub(current.count);
                let count = stack.count.min(space);
                stack.count -= count;
                current.count += count;
                success = count > 0;
            }
            Some(_) => {}
        }
        if success && let Some((mut tag, item)) = pot {
            let pos = view.first.0;
            Self::set_pot_slot(self.common.res.items(), &mut tag, item);
            spill::put_saved_entity(&mut self.common.world, [pos.0, pos.1, pos.2], tag);
            if !matches!(insertion, Insertion::Capability) {
                self.mark_block_entity_updated(pos);
            }
        }
        // tryMoveInItem's setItem on an empty slot: AbstractFurnaceBlockEntity.setItem restarts
        // the cooking for a new input (a merge only grows the stack, and keeps it).
        let recipes = server_world_recipes!(self);
        if filled_empty
            && let Some((pos, 0)) = view.locate(slot)
            && let Some(BlockEntity::Furnace(furnace)) =
                self.common.world.block_entities.get_mut(&pos)
        {
            use sunrise_sim::furnace::FurnaceCooking;
            let input = furnace.items[0].take();
            furnace.set_input(&self.common.loop_context, recipes, input);
        }
        if success && was_empty && view.kind == Kind::Hopper {
            let dest = view.first.0;
            let from_ticked =
                from_hopper.and_then(|p| self.server.hoppers.get(&p).map(|h| h.ticked));
            let state = self.server.hoppers.entry(dest).or_default();
            if state.cooldown <= 8 {
                let skip = i32::from(from_ticked.is_some_and(|t| state.ticked >= t));
                state.cooldown = 8 - skip;
            }
        }
        let result = if success {
            attempt.wrote_entity();
            MoveIn::Moved
        } else {
            MoveIn::Blocked
        };
        stack.count += beyond_slot;
        ((stack.count > 0).then_some(stack), result)
    }
    /// `HopperBlockEntity.addItem(from, container, stack, direction)`; the remainder.
    pub fn add_item(
        &mut self,
        from_hopper: Option<Pos>,
        view: &View,
        stack: ItemStack,
        face: Option<Direction>,
    ) -> Option<ItemStack> {
        if let Kind::Composter(_) = view.kind {
            let (x, y, z) = view.first.0;
            return self.composter_container_insert([x, y, z], stack, face == Some(Direction::Up));
        }
        let mut attempt = PlacementAttempt::default();
        let mut rest = Some(stack);
        for slot in view.slots_to_add_iter(face) {
            let Some(stack) = rest.take() else { break };
            rest = self
                .try_move_in_item(
                    Insertion::Automation(from_hopper),
                    view,
                    stack,
                    slot,
                    &mut attempt,
                )
                .0;
        }
        rest
    }
    /// Remove one item from a slot (`removeItem(slot, 1)`).
    pub(in crate::game::server_side) fn take_one(
        &mut self,
        view: &View,
        slot: usize,
    ) -> Option<ItemStack> {
        if view.kind == Kind::Pot {
            return self.pot_take_one(view.first.0);
        }
        self.containers().get(view, slot)?;
        let target = self.slot_mut_at(view, slot)?;
        let current = target.as_mut()?;
        let one = ItemStack {
            count: 1,
            ..current.clone()
        };
        current.count -= 1;
        if current.count == 0 {
            *target = None;
        }
        Some(one)
    }
    /// `ContainerSingleItem.removeItem(0, 1)` on a decorated pot's saved slot.
    pub(in crate::game::server_side) fn pot_take_one(&mut self, pos: Pos) -> Option<ItemStack> {
        let mut tag = spill::saved_entity(&self.common.world, [pos.0, pos.1, pos.2])?;
        let mut item = tag.get("item").and_then(save::item_from_nbt)?;
        if item.count == 0 {
            return None;
        }
        let one = ItemStack {
            count: 1,
            ..item.clone()
        };
        item.count -= 1;
        self.pot_store(pos, &mut tag, (item.count > 0).then_some(item));
        Some(one)
    }
    /// Undo `pot_take_one` after a failed move.
    pub(in crate::game::server_side) fn pot_put_back(&mut self, pos: Pos, one: ItemStack) {
        let Some(mut tag) = spill::saved_entity(&self.common.world, [pos.0, pos.1, pos.2]) else {
            return;
        };
        let item = match tag.get("item").and_then(save::item_from_nbt) {
            Some(mut current) => {
                current.count += one.count;
                current
            }
            None => one,
        };
        self.pot_store(pos, &mut tag, Some(item));
    }
    /// Write a pot's saved item slot, keeping the tag's other metadata.
    pub(in crate::game::server_side) fn pot_store(
        &mut self,
        pos: Pos,
        tag: &mut Tag,
        item: Option<ItemStack>,
    ) {
        Self::set_pot_slot(self.common.res.items(), tag, item);
        spill::put_saved_entity(&mut self.common.world, [pos.0, pos.1, pos.2], tag.clone());
        self.mark_block_entity_updated(pos);
    }
    pub(in crate::game::server_side) fn set_pot_slot(
        items: &sunrise_items::items::Items,
        tag: &mut Tag,
        item: Option<ItemStack>,
    ) {
        if let Tag::Compound(map) = tag {
            match item {
                Some(item) => {
                    map.insert("item".into(), save::item_to_nbt(&item, items));
                }
                None => {
                    map.remove(&"item".into());
                }
            }
        }
    }
    /// Undo `take_one` after a failed move.
    pub(in crate::game::server_side) fn put_back(
        &mut self,
        view: &View,
        slot: usize,
        one: ItemStack,
    ) {
        if view.kind == Kind::Pot {
            self.pot_put_back(view.first.0, one);
            return;
        }
        if let Some(target) = self.slot_mut_at(view, slot) {
            match target {
                Some(current) => current.count += one.count,
                None => *target = Some(one),
            }
        }
    }
    /// `HopperBlockEntity.pushItemsTick` of the hopper at `pos`, if its block
    /// is still a hopper (`BlockEntityType.isValid`).
    pub fn tick_hopper(&mut self, pos: Pos) {
        let time = self.common.world.game_time;
        // Every hopper keeps its cooldown and the tick it last ticked
        // (`HopperBlockEntity.tickedGameTime`), idle ones too: a hopper that
        // receives an item reads it for its cooldown (8, or 7 once it ticked).
        let state = self.server.hoppers.entry(pos).or_default();
        state.cooldown = state.cooldown.wrapping_sub(1);
        state.ticked = time;
        if state.cooldown > 0 {
            return;
        }
        state.cooldown = 0;
        // Most ticks only advance the cooldown: the block is checked when the
        // hopper is about to move, not on every tick it waits.
        let hopper = sunrise_game_data::block_entity_types::factory_block(
            &self
                .common
                .res
                .facts()
                .get(self.common.world.block(pos.0, pos.1, pos.2))
                .name,
        ) == "minecraft:hopper";
        if !hopper {
            self.server.hoppers.remove(&pos);
            return;
        }
        let state = self.common.world.block(pos.0, pos.1, pos.2);
        // A hopper a redstone signal turned off has nothing to try until the
        // signal changes, which wakes it through `state_changed`.
        if self.containers().prop(state, "enabled") == Some("false") {
            self.common.world.block_entities.sleep_hopper(pos);
            return;
        }
        let moved = self.hopper_try_move_items_with_state(pos, state);
        // An empty hopper with no source above has nothing to take or push:
        // its inventory, the block above and the items or vehicles reaching
        // it are what wake it.
        if !moved && self.hopper_has_no_source(pos) {
            self.common.world.block_entities.sleep_hopper(pos);
        }
    }
    /// Keeps a sleeping hopper's transfer state current without its transfer
    /// attempt: vanilla decrements the cooldown and stamps `tickedGameTime` on
    /// every tick, and a feeder that runs later in the same tick reads the
    /// stamp to choose 7 ticks over 8.
    pub(in crate::game) fn tick_sleeping_hopper(&mut self, pos: Pos) {
        let state = self.server.hoppers.entry(pos).or_default();
        state.cooldown = state.cooldown.wrapping_sub(1).max(0);
        state.ticked = self.common.world.game_time;
    }
    /// Drops the cooldowns of hoppers whose block entity is gone.
    pub(in crate::game) fn forget_gone_hoppers(&mut self) {
        let entities = &self.common.world.block_entities;
        self.server.hoppers.retain(|p, _| {
            matches!(entities.get(p), Some(BlockEntity::Chest(items)) if items.len() == HOPPER_SLOTS)
        });
    }
    /// `HopperBlockEntity.tryMoveItems` with `suckInItems`.
    pub fn hopper_try_move_items(&mut self, pos: Pos) {
        let state = self.common.world.block(pos.0, pos.1, pos.2);
        self.hopper_try_move_items_with_state(pos, state);
    }
    /// Whether the hopper moved an item this tick.
    pub(in crate::game::server_side) fn hopper_try_move_items_with_state(
        &mut self,
        pos: Pos,
        state: BlockStateId,
    ) -> bool {
        let c = self.containers();
        let enabled = c.prop(state, "enabled") != Some("false");
        let Some(view) = c.view(pos, false) else {
            return false;
        };
        if self
            .server
            .hoppers
            .get(&pos)
            .is_some_and(|h| h.cooldown > 0)
            || !enabled
        {
            return false;
        }
        self.native_prepare_hopper_transfer([pos.0, pos.1, pos.2]);
        self.unpack_loot(
            sunrise_gameplay_common::game::container_loot_level::LootTarget::Block(pos),
            None,
        );
        let c = self.containers();
        let mut changed = false;
        if !c.is_empty(&view) {
            changed = self.hopper_eject(pos, &view);
        }
        if !self.hopper_is_stacked(&view) {
            changed |= self.hopper_suck(pos, &view);
        }
        if changed {
            self.server.hoppers.entry(pos).or_default().cooldown = 8;
            self.container_changed(pos);
        }
        changed
    }
    /// Whether an empty hopper has no source to take from: no container above
    /// it (block, container entity or mod container) and nothing in its slots.
    /// Dropped items and container vehicles are left to the wake scans, which
    /// run before the hoppers tick.
    pub(in crate::game::server_side) fn hopper_has_no_source(&self, pos: Pos) -> bool {
        // A mod's transfer listener and a mod entity's item handler keep their
        // every-eligible-tick contract: an idle hopper that would call them
        // stays awake.
        if mod_api::__private::listened(mod_api::HookId::PrepareHopperTransfer)
            || mod_api::has_entity_providers(&ENTITY_AUTOMATION)
        {
            return false;
        }
        let empty = matches!(
            self.world.block_entities.get(&pos),
            Some(BlockEntity::Chest(items)) if items.iter().all(Option::is_none)
        );
        if !empty {
            return false;
        }
        let above = (pos.0, pos.1 + 1, pos.2);
        if self.containers().hopper_insertion_view(above).is_some()
            || matches!(
                self.world.block_entities.get(&above),
                Some(BlockEntity::Mod(_))
            )
        {
            return false;
        }
        // A mod block without a container block entity can still answer with an
        // item handler: only its own provider table says, so a provider that
        // later offers an item is never slept over.
        let Some(level) = self.native_level_here() else {
            return false;
        };
        let provider_at = |pos: Pos| mod_api::BlockSide {
            level,
            pos: [pos.0, pos.1, pos.2],
            side: Some(Direction::Down),
        };
        if mod_api::block_providers(&mod_api::ITEMS, self, provider_at(above)).is_some()
            || mod_api::block_providers(&mod_api::ITEMS, self, provider_at(pos)).is_some()
        {
            return false;
        }
        let at = [
            f64::from(pos.0) + 0.5,
            f64::from(pos.1) + 1.5,
            f64::from(pos.2) + 0.5,
        ];
        self.container_entities_at(at).is_empty()
    }
    /// Wakes the sleeping hoppers whose suck area (`Hopper.SUCK_AABB`) a dropped
    /// item's box reaches, once a tick before the hoppers tick.
    pub(in crate::game) fn wake_hoppers_near_items(&mut self) {
        if !self.common.world.block_entities.any_hopper_asleep() {
            return;
        }
        let entities = &mut self.common.world.block_entities;
        for item in &self.common.dropped {
            wake_hoppers_reaching(entities, item.bounds());
        }
    }
    /// Wakes the sleeping hoppers an item dropped now reaches. A drop made
    /// during the block entity pass (a loop slot's, another block entity's)
    /// comes after the pass's scan, so a later hopper of the same pass would
    /// otherwise stay asleep and miss it.
    pub(in crate::game::server_side) fn wake_hoppers_near_dropped_item(&mut self, id: u32) {
        if !self.common.world.block_entities.any_hopper_asleep() {
            return;
        }
        // `push_item_entity` just pushed it; a join listener runs later.
        let Some(item) = self.common.dropped.last().filter(|item| item.id == id) else {
            return;
        };
        wake_hoppers_reaching(&mut self.common.world.block_entities, item.bounds());
    }
    /// Wakes the sleeping hoppers a minecart or boat could be taken from.
    pub(in crate::game) fn wake_hoppers_near_vehicles(&mut self) {
        if !self.common.world.block_entities.any_hopper_asleep() {
            return;
        }
        let entities = &mut self.common.world.block_entities;
        for cart in &self.common.carts {
            wake_hoppers_reaching(entities, cart.bounds());
        }
        for boat in &self.common.boats {
            wake_hoppers_reaching(entities, boat.bounds());
        }
    }
    /// `HopperBlockEntity.entityInside` for the item entities that moved this
    /// tick: an item whose box meets the hopper's bowl (`Hopper.INSIDE`) and
    /// its suck area runs `tryMoveItems` with that item as the pickup, in the
    /// entity phase, before the hopper's own tick counts its cooldown down.
    pub(in crate::game::server_side) fn hoppers_catch_items(&mut self) {
        let mut i = 0;
        while i < self.common.dropped.len() {
            let item = &self.common.dropped[i];
            // `ItemEntity.tick` moves 1 of 4 ticks while resting;
            // `checkInsideBlocks` only follows a move.
            let moved = item.position != item.previous
                || item.ticks.wrapping_add(item.id).is_multiple_of(4);
            if !moved || item.stack.count == 0 {
                i += 1;
                continue;
            }
            let b = item.bounds();
            let (lo, hi) = (b.min_position(), b.max_position());
            let mut found = None;
            'blocks: for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
                for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
                    for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                        let state = self.common.world.block(x, y, z);
                        if sunrise_game_data::block_entity_types::factory_block(
                            &self.common.res.facts().get(state).name,
                        ) != "minecraft:hopper"
                        {
                            continue;
                        }
                        // Hopper.INSIDE: the bowl, 2..14 wide and from 11/16 up.
                        let (fx, fy, fz) = (f64::from(x), f64::from(y), f64::from(z));
                        if lo.x < fx + 0.875
                            && hi.x > fx + 0.125
                            && lo.y < fy + 1.0
                            && hi.y > fy + 11.0 / 16.0
                            && lo.z < fz + 0.875
                            && hi.z > fz + 0.125
                        {
                            found = Some((x, y, z));
                            break 'blocks;
                        }
                    }
                }
            }
            let Some(pos) = found else {
                i += 1;
                continue;
            };
            if self.hopper_take_item(pos, i) {
                continue; // the item left the list; `i` is the next one
            }
            i += 1;
        }
    }
    /// `tryMoveItems(…, () -> addItem(hopper, itemEntity))`; true when the
    /// item entity was taken whole.
    pub(in crate::game::server_side) fn hopper_take_item(&mut self, pos: Pos, i: usize) -> bool {
        if self
            .server
            .hoppers
            .get(&pos)
            .is_some_and(|h| h.cooldown > 0)
        {
            return false;
        }
        let state = self.common.world.block(pos.0, pos.1, pos.2);
        let c = self.containers();
        if c.prop(state, "enabled") == Some("false") {
            return false;
        }
        let Some(view) = c.view(pos, false) else {
            return false;
        };
        self.unpack_loot(
            sunrise_gameplay_common::game::container_loot_level::LootTarget::Block(pos),
            None,
        );
        let c = self.containers();
        let mut changed = false;
        if !c.is_empty(&view) {
            changed = self.hopper_eject(pos, &view);
        }
        let mut taken = false;
        if !self.hopper_is_stacked(&view) {
            let stack = self.common.dropped[i].stack.clone();
            let allowed = self.with_level(|_, level, _| {
                level.allow_hopper_pickup_item(
                    BlockPos::new(pos.0, pos.1, pos.2),
                    &stack.item,
                    stack.count,
                )
            });
            match allowed
                .then(|| self.add_item(None, &view, stack, None))
                .flatten()
            {
                None if !allowed => {}
                None => {
                    self.remove_dropped(i);
                    taken = true;
                    changed = true;
                }
                Some(rest) => self.common.dropped[i].stack = rest,
            }
        }
        if changed {
            self.server.hoppers.entry(pos).or_default().cooldown = 8;
            self.container_changed(pos);
        }
        taken
    }
    /// A mod's item handler through `side`. Vanilla containers and container providers keep
    /// their direct automation path even when a capability overrides their public storage.
    pub(in crate::game::server_side) fn item_part_at(
        &mut self,
        pos: Pos,
        side: Direction,
    ) -> Option<Capability<dyn ResourceHandler<ItemResource>>> {
        let at = BlockSide {
            level: self.native_level_here()?,
            pos: [pos.0, pos.1, pos.2],
            side: Some(side),
        };
        let providers = mod_api::block_providers(&mod_api::ITEMS, self, at)?;
        if self.vanilla_automation_container(pos) {
            return None;
        }
        let game = self.container_size(at).is_some();
        providers.ask(self, game)
    }
    /// Offers `stack` to a capability part and, in the same transaction, `take`s what it took
    /// from where it came, after the part's own writes, so a part handing it on back into the
    /// source loses nothing: how many went. A part taking none, a part answering more than it
    /// was offered (a mod bug) and a failed `take` undo the move (none went).
    pub(in crate::game::server_side) fn put_into_part(
        &mut self,
        part: &Capability<dyn ResourceHandler<ItemResource>>,
        stack: &ItemStack,
        take: impl FnOnce(&mut Self, u32) -> Option<()>,
    ) -> u32 {
        use sunrise_gameplay_common::game::api_values::TryIntoApi;
        let Ok(snapshot) = stack.try_into_api() else {
            return 0;
        };
        let resource = ItemResource::of(&snapshot);
        self.storage_transaction(|game, tx| {
            let host: &mut dyn mod_api::ServerHost = game;
            let went = part
                .insert(host, tx, &resource, stack.count)
                .map_err(|_| ())?;
            if went == 0 {
                return Err(());
            }
            take(game, went).map(|()| went).ok_or(())
        })
        .unwrap_or(0)
    }
    /// `InventoryMoveItemEvent` for `item` going from `source` to `destination`, where one end
    /// is a capability part: whether a mod canceled the move.
    pub(in crate::game::server_side) fn part_move_canceled(
        &mut self,
        source: [i32; 3],
        destination: [i32; 3],
        item: &str,
    ) -> bool {
        let mut cancel = false;
        self.native_inventory_move_item(source, destination, item, &mut cancel);
        cancel
    }
    /// `HopperBlockEntity.ejectItems`: one root attempt across source slots, restoring each
    /// refused slot directly. Only the caller's final cooldown mark changes the hopper.
    pub(in crate::game::server_side) fn eject_into_part(
        &mut self,
        hopper: &View,
        part: &Capability<dyn ResourceHandler<ItemResource>>,
        at: [i32; 3],
    ) -> bool {
        if mod_api::resource_util::is_full(self, part) {
            return false;
        }
        let from = self.container_world_pos(hopper);
        self.storage_transaction(|game, tx| {
            use sunrise_gameplay_common::game::api_values::TryIntoApi;
            for slot in 0..hopper.len() {
                let Some(snapshot) = game
                    .containers()
                    .get(hopper, slot)
                    .and_then(|stack| stack.try_into_api().ok())
                else {
                    continue;
                };
                let resource = ItemResource::of(&snapshot);
                let item = mod_api::__private::item_key_id(snapshot.item()).unwrap();
                if game.part_move_canceled(from, at, item) {
                    continue;
                }
                let (source, index) = hopper.locate(slot).ok_or(())?;
                game.journal_block_entity_slot(source, index);
                let Some(mut one) = game.take_one(hopper, slot) else {
                    continue;
                };
                let went = part.insert(game, tx, &resource, 1).map_err(|_| ())?;
                if went == 1 {
                    return Ok(());
                }
                one.count = snapshot.count();
                *game.slot_mut_at(hopper, slot).ok_or(())? = Some(one);
            }
            Err(())
        })
        .is_ok()
    }
    /// `HopperBlockEntity.suckInItems`: extract before choosing the first receptive hopper
    /// slot. Each nonempty storage index gets its own commit or abort.
    pub(in crate::game::server_side) fn suck_from_part(
        &mut self,
        part: &Capability<dyn ResourceHandler<ItemResource>>,
        at: [i32; 3],
        hopper: &View,
    ) -> bool {
        if self.hopper_is_stacked(hopper) {
            return false;
        }
        let into = self.container_world_pos(hopper);
        for slot in 0..part.size(self) {
            let Some(resource) = part.resource(self, slot) else {
                continue;
            };
            use sunrise_gameplay_common::game::api_values::TryIntoEngine;
            let Ok(one) = resource.as_stack().try_into_engine(self.items()) else {
                continue;
            };
            if part.amount(self, slot) == 0 || self.part_move_canceled(at, into, &one.item) {
                continue;
            }
            let moved = self.storage_transaction(|game, tx| {
                let host: &mut dyn mod_api::ServerHost = game;
                let came = part
                    .extract_at(host, tx, slot, &resource, 1)
                    .map_err(|_| ())?;
                if came != 1 {
                    return Err(());
                }
                let mut attempt = PlacementAttempt::default();
                let mut rest = one;
                for target in 0..hopper.len() {
                    match game.try_move_in_item(Insertion::Pull, hopper, rest, target, &mut attempt)
                    {
                        (Some(remainder), MoveIn::Skipped) => rest = remainder,
                        (None, MoveIn::Moved) => return Ok(()),
                        _ => return Err(()),
                    }
                }
                Err(())
            });
            if moved.is_ok() {
                return true;
            }
        }
        false
    }
    /// `HopperBlockEntity.ejectItems`: into the block container in front, else
    /// a container entity there (`getContainerAt`). A capability part in front takes the
    /// container's place.
    pub(in crate::game::server_side) fn hopper_eject(&mut self, pos: Pos, hopper: &View) -> bool {
        let state = self.common.world.block(pos.0, pos.1, pos.2);
        let Some(facing) = self
            .containers()
            .prop(state, "facing")
            .and_then(dir_from_name)
        else {
            return false;
        };
        let front = offset(pos, facing);
        let face = facing.opposite();
        if let Some(part) = self.item_part_at(front, face) {
            let at = [front.0, front.1, front.2];
            self.native_hopper_inventory_search([pos.0, pos.1, pos.2], Some(at), None);
            return self.eject_into_part(hopper, &part, at);
        }
        if let Some(target) = self.containers().hopper_insertion_view(front) {
            self.native_hopper_inventory_search(
                [pos.0, pos.1, pos.2],
                Some([front.0, front.1, front.2]),
                None,
            );
            return self.eject_into(pos, hopper, &target, face);
        }
        let Some(found) = self.automation_entity_at(
            [
                f64::from(front.0) + 0.5,
                f64::from(front.1) + 0.5,
                f64::from(front.2) + 0.5,
            ],
            face,
        ) else {
            return false;
        };
        match found {
            AutomationEntity::Container(id) => {
                self.native_hopper_inventory_search([pos.0, pos.1, pos.2], None, Some(id));
                self.with_entity_container(id, |g, target| g.eject_into(pos, hopper, target, face))
                    .unwrap_or(false)
            }
            AutomationEntity::Part(entity, part) => {
                self.native_hopper_inventory_search([pos.0, pos.1, pos.2], None, Some(entity.id()));
                self.eject_into_part(hopper, &part, [front.0, front.1, front.2])
            }
        }
    }
    /// Where a container is in the world for the mods: its block, or the block a chest
    /// boat or minecart is in (its view sits under a placeholder key).
    pub(in crate::game::server_side) fn container_world_pos(&self, view: &View) -> [i32; 3] {
        let (x, y, z) = view.first.0;
        if x == i32::MAX - 15 && y == super::minecarts::CART_KEY_Y {
            let id = z as u32;
            let at = self
                .common
                .carts
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.position)
                .or_else(|| {
                    self.common
                        .boats
                        .iter()
                        .find(|b| b.id == id)
                        .map(|b| b.position)
                });
            if let Some(p) = at {
                return [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32];
            }
        }
        [x, y, z]
    }
    /// `InventoryMoveItemEvent` for the item in `slot` of `holder` going from `source` to
    /// `destination`: whether a mod canceled the move. Nothing is looked up with no mod loaded.
    pub(in crate::game::server_side) fn hopper_move_canceled(
        &self,
        holder: &View,
        slot: usize,
        source: &View,
        destination: &View,
    ) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::InventoryMoveItem) {
            return false;
        }
        let containers = self.containers();
        let Some(stack) = containers.get(holder, slot) else {
            return false;
        };
        let mut cancel = false;
        self.native_inventory_move_item(
            self.container_world_pos(source),
            self.container_world_pos(destination),
            &stack.item,
            &mut cancel,
        );
        cancel
    }
    /// The rest of `HopperBlockEntity.ejectItems`: one item into `target`.
    pub fn eject_into(&mut self, pos: Pos, hopper: &View, target: &View, face: Direction) -> bool {
        self.unpack_inventory_slot(hopper, 0);
        if self.inventory_is_full(target, face) {
            return false;
        }
        for slot in 0..hopper.len() {
            if self.hopper_move_canceled(hopper, slot, hopper, target) {
                continue;
            }
            let Some(one) = self.take_one(hopper, slot) else {
                continue;
            };
            match self.add_item(Some(pos), target, one, Some(face)) {
                None => {
                    self.view_changed(target);
                    return true;
                }
                Some(rest) => self.put_back(hopper, slot, rest),
            }
        }
        false
    }
    /// `HopperBlockEntity.suckInItems`: from the container above, or item entities.
    pub(in crate::game::server_side) fn hopper_suck(&mut self, pos: Pos, hopper: &View) -> bool {
        let above = (pos.0, pos.1 + 1, pos.2);
        let at = [
            f64::from(pos.0) + 0.5,
            f64::from(pos.1) + 1.5,
            f64::from(pos.2) + 0.5,
        ];
        if let Some(took) = self.suck_from_container(above, at, hopper) {
            return took;
        }
        let above_state = self.common.world.block(above.0, above.1, above.2);
        let blocked = self.common.res.facts().get(above_state).full_collision
            && !self
                .common
                .res
                .behaviours()
                .tag("minecraft:does_not_block_hoppers")
                .is_some_and(|t| t.contains(self.common.res.behaviours().block(above_state)));
        if blocked {
            return false;
        }
        if self.common.dropped.is_empty() {
            return false;
        }
        // Hopper.SUCK_AABB: the hopper bowl and the block above.
        let (min, max) = (
            [
                f64::from(pos.0),
                f64::from(pos.1) + 11.0 / 16.0,
                f64::from(pos.2),
            ],
            [
                f64::from(pos.0) + 1.0,
                f64::from(pos.1) + 2.0,
                f64::from(pos.2) + 1.0,
            ],
        );
        if let Some(columns) = self.server.item_columns.as_mut()
            && !columns.may_reach(&self.common.dropped, pos.0, pos.2)
        {
            return false;
        }
        self.suck_item_entities(Some(pos), hopper, min, max)
    }
    /// The container half of `HopperBlockEntity.suckInItems`: one item from the
    /// container at `above`, through its bottom face; with no block container
    /// there, from a container entity whose box meets the one around `at`
    /// (`getSourceContainerOrHandler`). `None` when there is no container. A capability part
    /// above takes the container's place.
    pub fn suck_from_container(&mut self, above: Pos, at: [f64; 3], hopper: &View) -> Option<bool> {
        let (x, y, z) = hopper.first.0;
        if let Some(part) = self.item_part_at(above, Direction::Down) {
            let above = [above.0, above.1, above.2];
            self.native_hopper_inventory_search([x, y, z], Some(above), None);
            return Some(self.suck_from_part(&part, above, hopper));
        }
        let c = self.containers();
        let Some(source) = c.hopper_insertion_view(above) else {
            let found = self.automation_entity_at(at, Direction::Down)?;
            return match found {
                AutomationEntity::Container(id) => {
                    self.native_hopper_inventory_search([x, y, z], None, Some(id));
                    self.with_entity_container(id, |g, source| {
                        g.suck_from_slots(None, source, hopper)
                    })
                }
                AutomationEntity::Part(entity, part) => {
                    self.native_hopper_inventory_search([x, y, z], None, Some(entity.id()));
                    Some(self.suck_from_part(&part, [above.0, above.1, above.2], hopper))
                }
            };
        };
        self.native_hopper_inventory_search([x, y, z], Some([above.0, above.1, above.2]), None);
        if let Kind::Composter(level) = source.kind {
            if level != 8 {
                return Some(false);
            }
            // tryTakeInItemFromSlot on the OutputContainer: removeItem calls
            // its setChanged, which empties the composter, and so does the
            // second setChanged after the move or, when the hopper has no
            // room, the setItem putting the bone meal back into the detached
            // container (whose bone meal is lost with it).
            let bone_meal = bone_meal_stack().clone();
            if mod_api::__private::listened(mod_api::HookId::InventoryMoveItem) {
                let mut cancel = false;
                self.native_inventory_move_item(
                    [above.0, above.1, above.2],
                    self.container_world_pos(hopper),
                    &bone_meal.item,
                    &mut cancel,
                );
                if cancel {
                    return Some(false);
                }
            }
            let at = BlockPos::new(above.0, above.1, above.2);
            self.with_level(|b, level, ticks| b.composter_empty(level, ticks, at));
            let moved = self.add_item(None, hopper, bone_meal, None).is_none();
            self.with_level(|b, level, ticks| b.composter_empty(level, ticks, at));
            self.common
                .world
                .modified
                .insert((above.0 >> 4, above.2 >> 4));
            return Some(moved);
        }
        let source_hopper = (source.kind == Kind::Hopper).then_some(above);
        Some(self.suck_from_slots(source_hopper, &source, hopper))
    }
    /// `HopperBlockEntity.tryTakeInItemFromSlot` over the `DOWN` face of `source`.
    pub(in crate::game::server_side) fn suck_from_slots(
        &mut self,
        source_hopper: Option<Pos>,
        source: &View,
        hopper: &View,
    ) -> bool {
        let mut prepared = None;
        let mut attempt = TakeAttempt::default();
        for slot in source.slots_for_face_iter(Some(Direction::Down)) {
            self.unpack_next_inventory_part(source, slot, &mut prepared);
            {
                let c = self.containers();
                let Some(stack) = c.insertion_item(source, slot).map(Cow::into_owned) else {
                    continue;
                };
                let ask = TakeAsk {
                    game: self,
                    attempt: &mut attempt,
                };
                if !c.can_take_into(source, slot, &stack, Direction::Down, hopper, ask) {
                    continue;
                }
            }
            if self.hopper_move_canceled(source, slot, source, hopper) {
                continue;
            }
            let Some(one) = self.take_one(source, slot) else {
                continue;
            };
            match self.add_item(source_hopper, hopper, one, None) {
                None => {
                    self.view_changed(source);
                    // JukeboxBlockEntity.removeItem -> setTheItem(EMPTY): a
                    // disc a hopper takes out stops the song.
                    if source.kind == Kind::Jukebox {
                        let (x, y, z) = source.first.0;
                        self.jukebox_taken([x, y, z]);
                    }
                    return true;
                }
                Some(rest) => {
                    self.put_back(source, slot, rest);
                }
            }
        }
        false
    }
    /// The alive container entities (`EntitySelector.CONTAINER_ENTITY_SELECTOR`: chest and
    /// hopper minecarts, chest boats) whose box meets the 1x1x1 box around `at`.
    pub(in crate::game::server_side) fn container_entities_at(&self, at: [f64; 3]) -> Vec<u32> {
        let meets = |b: sunrise_core::geometry::Aabb| {
            let (lo, hi) = (b.min_position(), b.max_position());
            lo.x < at[0] + 0.5
                && hi.x > at[0] - 0.5
                && lo.y < at[1] + 0.5
                && hi.y > at[1] - 0.5
                && lo.z < at[2] + 0.5
                && hi.z > at[2] - 0.5
        };
        let mut found: Vec<u32> = self
            .common
            .carts
            .iter()
            .filter(|c| {
                matches!(
                    c.kind,
                    super::minecarts::CartKind::Chest | super::minecarts::CartKind::Hopper
                ) && meets(c.bounds())
            })
            .map(|c| c.id)
            .collect();
        found.extend(
            self.common
                .boats
                .iter()
                .filter(|b| b.chest && meets(b.bounds()))
                .map(|b| b.id),
        );
        found
    }
    /// The index `RandomSource.nextInt(len)` picks of `len` candidates: the level's one draw,
    /// none for no candidate.
    pub(in crate::game::server_side) fn pick_index(&mut self, len: usize) -> Option<usize> {
        if len == 0 {
            return None;
        }
        let pick = self
            .common
            .world
            .random
            .next_int_bound(len as i32)
            .unwrap_or(0);
        Some(pick as usize)
    }
    /// The entity item automation reaches around `at` from `side`, through `ENTITY_AUTOMATION`: one of
    /// the container entities the game has there, and, once a mod registered a provider for it,
    /// the entities whose mod provider answers it, picked at random as `getEntityContainer` picks, the one draw.
    /// Vanilla container entities always keep their direct path, even with a provider.
    pub(in crate::game::server_side) fn automation_entity_at(
        &mut self,
        at: [f64; 3],
        side: Direction,
    ) -> Option<AutomationEntity> {
        let vanilla = self.container_entities_at(at);
        if !mod_api::has_entity_providers(&ENTITY_AUTOMATION) {
            let pick = self.pick_index(vanilla.len())?;
            return Some(AutomationEntity::Container(vanilla[pick]));
        }
        let mut found: Vec<AutomationEntity> = Vec::new();
        let context = Some(side);
        let ask = |game: &mut Self, entity: EntityRef, has_game_answer: bool| {
            let providers = mod_api::entity_providers(&ENTITY_AUTOMATION, &*game, entity, context)?;
            providers.ask(game, has_game_answer)
        };
        found.extend(vanilla.iter().copied().map(AutomationEntity::Container));
        if let Some(level) = self.native_level_here() {
            let min = [at[0] - 0.5, at[1] - 0.5, at[2] - 0.5];
            let max = [at[0] + 0.5, at[1] + 0.5, at[2] + 0.5];
            for entity in self.level_entities_in(level, min, max).unwrap_or_default() {
                if !vanilla.contains(&entity.id())
                    && let Some(part) = ask(self, entity, false)
                {
                    found.push(AutomationEntity::Part(entity, part));
                }
            }
        }
        let pick = self.pick_index(found.len())?;
        Some(found.swap_remove(pick))
    }
    /// Runs `f` on the block container at `pos`, or one entity there, with providers taking
    /// the game's slots' place when registered ahead of them.
    pub(in crate::game::server_side) fn with_container_at<R>(
        &mut self,
        pos: Pos,
        side: Direction,
        f: impl FnOnce(&mut Self, ContainerTarget<'_>) -> R,
    ) -> Option<R> {
        if let Some(part) = self.item_part_at(pos, side) {
            return Some(f(self, ContainerTarget::Part(part)));
        }
        if let Some(view) = self.containers().hopper_insertion_view(pos) {
            return Some(f(self, ContainerTarget::View(&view)));
        }
        let centre = [
            f64::from(pos.0) + 0.5,
            f64::from(pos.1) + 0.5,
            f64::from(pos.2) + 0.5,
        ];
        match self.automation_entity_at(centre, side)? {
            AutomationEntity::Container(id) => {
                self.with_entity_container(id, |game, view| f(game, ContainerTarget::View(view)))
            }
            AutomationEntity::Part(_, part) => Some(f(self, ContainerTarget::Part(part))),
        }
    }
    /// Runs `f` on a container entity's items as a container, under the
    /// placeholder key its menu uses (the open menu's copy when it is open),
    /// then hands the items back to the entity.
    pub fn with_entity_container<R>(
        &mut self,
        id: u32,
        f: impl FnOnce(&mut Self, &View) -> R,
    ) -> Option<R> {
        let mut items = if let Some(i) = self.common.carts.iter().position(|c| c.id == id) {
            self.unpack_loot(
                sunrise_gameplay_common::game::container_loot_level::LootTarget::Cart(id),
                None,
            );
            self.common
                .carts
                .update(i, |c| std::mem::take(&mut c.items))
        } else {
            let i = self.common.boats.iter().position(|b| b.id == id)?;
            self.unpack_boat_loot(i, false);
            self.common
                .boats
                .update(i, |b| std::mem::take(&mut b.items))
        };
        let key = (i32::MAX - 15, super::minecarts::CART_KEY_Y, id as i32);
        let (view, open) = self.lend_items(key, &mut items);
        let result = f(self, &view);
        self.reclaim_items(key, open, &mut items);
        if let Some(i) = self.common.carts.iter().position(|c| c.id == id) {
            self.common.carts.update(i, |c| c.items = items);
        } else if let Some(i) = self.common.boats.iter().position(|b| b.id == id) {
            self.common.boats.update(i, |b| b.items = items);
        }
        Some(result)
    }
    /// A vehicle's slots as a container under its placeholder `key`. When its
    /// menu is open the placeholder is the live copy and stays as it is.
    /// Returns the view and whether the menu was open.
    pub(in crate::game::server_side) fn lend_items(
        &mut self,
        key: Pos,
        items: &mut Vec<Option<ItemStack>>,
    ) -> (View, bool) {
        let open = self.common.world.block_entities.contains_key(&key);
        if !open {
            let slots = BlockEntity::Chest(std::mem::take(items));
            self.common.world.block_entities.insert(key, slots);
        }
        let size = self
            .common
            .world
            .block_entities
            .get(&key)
            .map_or(0, |e| e.slots().len());
        let view = View {
            first: (key, size),
            second: None,
            kind: Kind::Generic,
            container: None,
        };
        (view, open)
    }
    /// Gives `lend_items`' slots back to the vehicle, so its own items stay
    /// the one source of truth: a menu copy that is still around (open, or
    /// left by a guest's close) gets the changed items copied to `items`.
    pub(in crate::game::server_side) fn reclaim_items(
        &mut self,
        key: Pos,
        open: bool,
        items: &mut Vec<Option<ItemStack>>,
    ) {
        let slots = if open {
            self.common.world.block_entities.get(&key).cloned()
        } else {
            self.common.world.block_entities.remove(&key)
        };
        if let Some(BlockEntity::Chest(slots)) = slots {
            *items = slots;
        }
    }
    /// The item half of `HopperBlockEntity.suckInItems` (`getItemsAtAndAbove`):
    /// adds the item entities that meet the box, until one is taken whole.
    pub(in crate::game::server_side) fn suck_item_entities(
        &mut self,
        hopper_pos: Option<Pos>,
        hopper: &View,
        min: [f64; 3],
        max: [f64; 3],
    ) -> bool {
        let mut i = 0;
        while i < self.common.dropped.len() {
            let b = self.common.dropped[i].bounds();
            let (lo, hi) = (b.min_position(), b.max_position());
            let inside = lo.x < max[0]
                && hi.x > min[0]
                && lo.y < max[1]
                && hi.y > min[1]
                && lo.z < max[2]
                && hi.z > min[2];
            if inside {
                let stack = self.common.dropped[i].stack.clone();
                let allowed = hopper_pos.is_none_or(|pos| {
                    self.with_level(|_, level, _| {
                        level.allow_hopper_pickup_item(
                            BlockPos::new(pos.0, pos.1, pos.2),
                            &stack.item,
                            stack.count,
                        )
                    })
                });
                if !allowed {
                    i += 1;
                    continue;
                }
                match self.add_item(None, hopper, stack, None) {
                    None => {
                        self.remove_dropped(i);
                        return true;
                    }
                    Some(rest) => self.common.dropped[i].stack = rest,
                }
            }
            i += 1;
        }
        false
    }
    /// `suck_item_entities` for a hopper minecart, which first asks the item
    /// columns whether any item's box reaches the columns of the box: an item
    /// that meets it reaches one of them. The columns follow the item list's
    /// stamp, so any write since they were built (an item's move, an
    /// explosion's push, a command, a pickup) makes them scan, then rebuild.
    pub(in crate::game::server_side) fn cart_suck_items(
        &mut self,
        hopper: &View,
        min: [f64; 3],
        max: [f64; 3],
    ) -> bool {
        if !self
            .server
            .pickup_columns
            .get_or_insert_with(Default::default)
            .may_reach_area(
                &self.common.dropped,
                (min[0].floor() as i32, max[0].floor() as i32),
                (min[2].floor() as i32, max[2].floor() as i32),
            )
        {
            return false;
        }
        self.suck_item_entities(None, hopper, min, max)
    }
    /// `MinecartHopper.tick`'s `tryConsumeItems` for whichever path ticks the
    /// cart. `ServerLevel.tickNonPassenger` ticks the vehicle, then its
    /// passengers, so a ridden cart pulls too; Sunrise ticks that one in its
    /// rider's turn (`tick_cart_riding`). `alive` is `isAlive` after the move.
    pub(in crate::game) fn hopper_cart_tick(
        &mut self,
        cart: &mut super::minecarts::Minecart,
        alive: bool,
    ) {
        if alive && cart.kind == super::minecarts::CartKind::Hopper && cart.enabled {
            self.hopper_cart_suck(cart);
        }
    }
    /// `MinecartHopper.tick`'s `tryConsumeItems` (`suckInItems`) for an enabled
    /// hopper minecart: one item from the container above, else the item
    /// entities over it or touching it, each tick without a cooldown.
    /// `consumedItemThisFrame` in 26.3 caps a tick at one item: the tick tries
    /// once, and `makeStepAlongTrack` may try again, but only one succeeds.
    pub fn hopper_cart_suck(&mut self, cart: &mut super::minecarts::Minecart) {
        if !self.unpack_ticked_cart_loot(cart) {
            return;
        }
        // The cart's slots are edited as a container under the placeholder key
        // its menu uses; when that menu is open the placeholder is the live copy.
        let key = (i32::MAX - 15, super::minecarts::CART_KEY_Y, cart.id as i32);
        let (view, open) = self.lend_items(key, &mut cart.items);
        let p = cart.position;
        // getLevelY is the cart's y + 0.5, so the container is the block at
        // y + 1.5, and Hopper.SUCK_AABB (moved by getLevel - 0.5) starts 11/16
        // above the cart.
        let above = (
            p.x.floor() as i32,
            (p.y + 1.5).floor() as i32,
            p.z.floor() as i32,
        );
        let at = [p.x, p.y + 1.5, p.z];
        let took = match self.suck_from_container(above, at, &view) {
            Some(took) => took,
            None => self.cart_suck_items(
                &view,
                [p.x - 0.5, p.y + 11.0 / 16.0, p.z - 0.5],
                [p.x + 0.5, p.y + 2.0, p.z + 0.5],
            ),
        };
        if !took {
            // The cart's own box, inflated by 0.25 sideways.
            let b = cart.bounds();
            let (lo, hi) = (b.min_position(), b.max_position());
            self.cart_suck_items(
                &view,
                [lo.x - 0.25, lo.y, lo.z - 0.25],
                [hi.x + 0.25, hi.y, hi.z + 0.25],
            );
        }
        self.reclaim_items(key, open, &mut cart.items);
    }

    // ---- dispensers and droppers ----

    /// `DispenserBlock.dispenseFrom` / `DropperBlock.dispenseFrom`.
    pub fn dispense_from(&mut self, pos: BlockPos, state: BlockStateId) -> Result<()> {
        let key = (pos.x, pos.y, pos.z);
        let name = self.common.res.facts().get(state).name.clone();
        if &*name == "minecraft:crafter" {
            return self.crafter_dispense(pos, state);
        }
        let dropper = &*name == "minecraft:dropper";
        if !dropper && &*name != "minecraft:dispenser" {
            return Ok(());
        }
        if !dropper && self.dispenser_source(key).is_none() {
            log::warn!("ignoring dispense from {key:?}: no matching dispenser holder");
            return Ok(());
        }
        let Some(view) = self.containers().view(key, false) else {
            return Ok(());
        };
        self.unpack_loot(
            sunrise_gameplay_common::game::container_loot_level::LootTarget::Block(key),
            None,
        );
        // DispenserBlockEntity.getRandomSlot
        let mut slot = None;
        let mut odds = 1;
        for i in 0..view.len() {
            if self.containers().get(&view, i).is_some() {
                if self.common.world.random.next_int_bound(odds).unwrap_or(1) == 0 {
                    slot = Some(i);
                }
                odds += 1;
            }
        }
        let Some(slot) = slot else {
            self.dispenser_event(1001, key, 0);
            if !dropper {
                self.post_block_game_event(
                    "minecraft:block_activate",
                    [pos.x, pos.y, pos.z],
                    None,
                    Some(state),
                );
            }
            return Ok(());
        };
        let Some(facing) = self
            .containers()
            .prop(state, "facing")
            .and_then(dir_from_name)
        else {
            return Ok(());
        };
        let Some(stack) = self.containers().get(&view, slot).cloned() else {
            return Ok(());
        };
        let holder = self.common.world.block_entities.identity(key);
        let target = offset(key, facing);
        let mut remaining = if dropper {
            let side = facing.opposite();
            let inserted = self.with_container_at(target, side, |game, into| match into {
                ContainerTarget::View(into) => {
                    let one = ItemStack {
                        count: 1,
                        ..stack.clone()
                    };
                    let inserted = game.add_item(None, into, one, Some(side)).is_none();
                    if inserted {
                        game.view_changed(into);
                    }
                    ControlFlow::Continue(inserted)
                }
                ContainerTarget::Part(part) => {
                    game.dropper_into_part(&part, &stack, &view, slot);
                    ControlFlow::Break(())
                }
            });
            match inserted {
                Some(ControlFlow::Break(())) => {
                    self.container_changed(key);
                    return Ok(());
                }
                Some(ControlFlow::Continue(inserted)) => Some(ItemStack {
                    count: stack.count - u32::from(inserted),
                    ..stack
                })
                .filter(|s| s.count > 0),
                None => self.dispense_default_in_level(key, facing, stack),
            }
        } else {
            let ControlFlow::Continue(remaining) = self.dispense_item(key, facing, stack)? else {
                return Ok(());
            };
            remaining
        };
        // DispenserBlock.dispenseFrom calls BaseContainerBlockEntity.setItem after the
        // behaviour returns. Only this resident write limits the detached remainder's count.
        if !dropper && let Some(stack) = &mut remaining {
            stack.count = stack
                .count
                .min(self.containers().max_stack_in(&view, stack));
        }
        // A listener can replace the block entity while the behaviour runs. Like the
        // vanilla BlockSource, the action owns the original row, not its replacement.
        if let Some(holder) = holder
            && self.common.world.block_entities.identity(key) == Some(holder)
        {
            self.journal_block_entity_slot(key, slot);
            if let Some(entry) = self.slot_mut_at(&view, slot) {
                *entry = remaining;
                if dropper {
                    self.after_commit(sunrise_gameplay_common::game::transactions::After::Changed(
                        key,
                    ));
                } else {
                    self.after_holder_changed(holder);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
pub(in crate::game) mod item_column_tests {
    use super::ItemColumns;
    use sunrise_core::geometry::Vec3;
    use sunrise_items::items::ItemStack;
    use sunrise_sim::entities::ItemEntity;

    fn item(id: u32, x: f64, y: f64, z: f64) -> ItemEntity {
        let at = Vec3::new(x, y, z);
        ItemEntity {
            size_answer: None,
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            random: sunrise_core::random::LegacyRandom::new(0),
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            health: 5,
            fire_ticks: 0,
            id,
            stack: ItemStack::new("minecraft:stone", 1),
            thrower: None,
            target: None,
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            ground: sunrise_world::ground_support::GroundState::from_ground_flag(true),
            vertical_collision: false,
            age: 0,
            stack_revision: 0,
            lifespan: ItemEntity::LIFETIME,
            pickup_delay: 0,
            bob_offset: 0.0,
            ticks: 0,
            needs_sync: false,
            stuck: Vec3::ZERO,
            was_touching_water: false,
        }
    }
    /// `hopper_suck`'s test: some item box meets the hopper's SUCK_AABB.
    fn scan(items: &[ItemEntity], (x, y, z): (i32, i32, i32)) -> bool {
        let (min, max) = (
            Vec3::new(f64::from(x), f64::from(y) + 11.0 / 16.0, f64::from(z)),
            Vec3::new(f64::from(x) + 1.0, f64::from(y) + 2.0, f64::from(z) + 1.0),
        );
        items.iter().any(|i| {
            let b = i.bounds();
            let (lo, hi) = (b.min_position(), b.max_position());
            lo.x < max.x
                && hi.x > min.x
                && lo.y < max.y
                && hi.y > min.y
                && lo.z < max.z
                && hi.z > min.z
        })
    }

    /// An item moved in place (a `/tp`, an explosion's push) keeps the list's
    /// length and last id; the stamped columns still see where it went.
    #[test]
    fn a_moved_item_is_seen_where_it_went() {
        let mut items = sunrise_sim::join_seq::JoinList::<_, true>::from_iter([
            item(1, 0.5, 64.0, 0.5),
            item(2, 40.5, 64.0, 40.5),
        ]);
        let mut columns = super::StampedItemColumns::default();
        let far = ((100, 100), (100, 100));
        // The first question for a list scans; the second builds the columns.
        assert!(columns.may_reach_area(&items, far.0, far.1));
        assert!(!columns.may_reach_area(&items, far.0, far.1));
        items.update(0, |item| {
            item.position = sunrise_core::geometry::Vec3::new(100.5, 64.0, 100.5)
        });
        assert!(columns.may_reach_area(&items, far.0, far.1));
        assert!(columns.may_reach_area(&items, far.0, far.1));
        assert!(!columns.may_reach_area(&items, (0, 0), (0, 0)));
    }

    #[test]
    fn no_player_with_an_item_in_reach_is_skipped() {
        // guest_pickups' horizontal test, on every item.
        let touches = |items: &[ItemEntity], p: [f64; 3]| {
            items
                .iter()
                .any(|d| (d.position.x - p[0]).abs() < 1.5 && (d.position.z - p[2]).abs() < 1.5)
        };
        let mut seed = 0x0bad_5eed_u64;
        let mut next = |range: f64| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (seed >> 11) as f64 / (1u64 << 53) as f64 * range
        };
        let mut items: Vec<ItemEntity> = Vec::new();
        let mut columns = ItemColumns::default();
        let (mut skipped, mut found) = (0, 0);
        for round in 0..4000u32 {
            if round % 25 == 0 {
                // On column edges as often as not.
                let mut at = || {
                    let v = next(40.0) - 20.0;
                    if next(1.0) < 0.5 { v.round() } else { v }
                };
                items.push(item(round, at(), 64.0, at()));
            } else if round % 101 == 0 && !items.is_empty() {
                items.remove(next(items.len() as f64) as usize);
            }
            let p = [next(44.0) - 22.0, 64.0, next(44.0) - 22.0];
            let reach = columns.may_reach_area(
                &items,
                ((p[0] - 1.5).floor() as i32, (p[0] + 1.5).floor() as i32),
                ((p[2] - 1.5).floor() as i32, (p[2] + 1.5).floor() as i32),
            );
            let touching = touches(&items, p);
            assert!(
                reach || !touching,
                "round {round}: skipped a player touching an item"
            );
            skipped += usize::from(!reach);
            found += usize::from(touching);
        }
        assert!(
            skipped > 1000 && found > 100,
            "skipped {skipped}, found {found}"
        );
    }

    #[test]
    fn no_hopper_with_an_item_over_it_is_skipped() {
        let mut seed = 0x1234_5678_9abc_def1u64;
        let mut next = |range: f64| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (seed >> 11) as f64 / (1u64 << 53) as f64 * range
        };
        let mut items: Vec<ItemEntity> = Vec::new();
        let mut columns = ItemColumns::default();
        let mut next_id = 0;
        let mut skipped = 0;
        for round in 0..3000 {
            // Items come and go as hoppers take them and crafters drop them.
            match round % 5 {
                0 | 1 => {
                    next_id += 1;
                    // On edges too: a quarter of them sit on a block line.
                    let snap = |v: f64, edge: bool| if edge { v.floor() + 0.125 } else { v };
                    let edge = round % 4 == 0;
                    items.push(item(
                        next_id,
                        snap(next(24.0) - 12.0, edge),
                        64.0,
                        snap(next(24.0) - 12.0, edge),
                    ));
                }
                2 if !items.is_empty() => {
                    let i = next(items.len() as f64) as usize;
                    items.remove(i);
                }
                _ => {}
            }
            let hopper = (next(26.0) as i32 - 13, 63, next(26.0) as i32 - 13);
            let reach = columns.may_reach(&items, hopper.0, hopper.2);
            let found = scan(&items, hopper);
            assert!(
                reach || !found,
                "round {round}: skipped a hopper with an item over it"
            );
            skipped += usize::from(!reach);
        }
        assert!(skipped > 1000, "skipped only {skipped}");
    }
}

#[cfg(test)]
pub(in crate::game) mod mod_container_tests {
    use super::{Kind, ModContainer, View, room};
    use std::sync::Arc;
    use sunrise_core::geometry::Direction;
    use sunrise_items::items::ItemStack;

    /// A mod's container reaches the slots its block names for each face;
    /// without a face, every slot.
    #[test]
    fn mod_container_slots_by_face() {
        let mut faces: [Vec<usize>; 6] = Default::default();
        faces[Direction::Up as usize] = vec![0];
        faces[Direction::Down as usize] = vec![2];
        faces[Direction::North as usize] = vec![1];
        let view = View {
            first: ((0, 0, 0), 3),
            second: None,
            kind: Kind::Mod,
            container: Some(Arc::new(ModContainer {
                slots: 3,
                faces,
                inputs: vec![0, 1],
                outputs: vec![2],
                menu: None,
                insert_rules: Vec::new(),
            })),
        };
        assert_eq!(
            view.slots_for_face_iter(Some(Direction::Up))
                .collect::<Vec<_>>(),
            [0]
        );
        assert_eq!(
            view.slots_for_face_iter(Some(Direction::Down))
                .collect::<Vec<_>>(),
            [2]
        );
        assert_eq!(
            view.slots_for_face_iter(Some(Direction::North))
                .collect::<Vec<_>>(),
            [1]
        );
        assert!(
            view.slots_for_face_iter(Some(Direction::East))
                .next()
                .is_none()
        );
        assert_eq!(
            view.slots_for_face_iter(None).collect::<Vec<_>>(),
            [0, 1, 2]
        );
    }

    /// A slot takes no more than its container's limit and the item's: one book into a chiseled
    /// bookshelf slot of 64 offered, a chest slot 64 and then the rest of 64.
    #[test]
    fn a_slot_takes_up_to_its_containers_limit() {
        let books = |count| ItemStack {
            item: "minecraft:book".into(),
            count,
            damage: 0,
            extra: None,
        };
        let limit = |kind| {
            let view = View {
                first: ((0, 0, 0), 6),
                second: None,
                kind,
                container: None,
            };
            view.slot_limit().min(64)
        };
        assert_eq!(room(limit(Kind::Bookshelf), None, &books(64)), 1);
        assert_eq!(room(limit(Kind::Jukebox), None, &books(64)), 1);
        assert_eq!(room(limit(Kind::Generic), None, &books(64)), 64);
        assert_eq!(room(limit(Kind::Generic), Some(&books(60)), &books(64)), 4);
    }
}

#[cfg(test)]
pub(in crate::game) mod capacity_tests {
    use super::{Containers, Kind, View};
    use std::collections::HashSet;
    use sunrise_items::items::ItemStack;
    use sunrise_level::{brewing::Brewing, containers::BlockEntity};

    /// A one-item slot answers its capacity as if empty (NeoForge's `getCapacityAsLong`), so a
    /// write may swap what it holds, while it takes nothing more now: a jukebox's disc, a
    /// chiseled bookshelf's book and a brewing stand's bottle.
    #[test]
    fn a_full_one_item_slot_still_has_room_for_a_replacement() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("block-behaviours.json").exists() {
            return;
        }
        let assets = sunrise_level::assets::Assets::load(&root).unwrap();
        let registry = assets.registry;
        let facts = sunrise_items::blocks::BlockFacts::load(&root, registry).unwrap();
        let overlay = &sunrise_world::loot::PackOverlay::NONE;
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let items =
            sunrise_items::items::Items::load(&root, registry, &facts, overlay, &defaults).unwrap();
        let behaviours = sunrise_world::level::Behaviours::load(registry, &root).unwrap();
        let brewing = Brewing {
            recipes: Vec::new(),
            reagents: HashSet::new(),
            inputs: HashSet::from(["minecraft:potion".to_owned()]),
        };
        let air = registry.default_state("minecraft:air").unwrap();
        let mut world = sunrise_level::world::World::new(air, 0);
        let cases = [
            (
                "minecraft:jukebox",
                Kind::Jukebox,
                1,
                "minecraft:music_disc_cat",
                "minecraft:music_disc_13",
            ),
            (
                "minecraft:chiseled_bookshelf",
                Kind::Bookshelf,
                6,
                "minecraft:book",
                "minecraft:enchanted_book",
            ),
            (
                "minecraft:brewing_stand",
                Kind::Brewing,
                5,
                "minecraft:potion",
                "minecraft:potion",
            ),
        ];
        for (x, &(block, _, _, held, _)) in (0..).zip(&cases) {
            let mut entity = BlockEntity::for_block(block).unwrap();
            entity.slots_mut()[0] = Some(ItemStack::new(held, 1));
            world.block_entities.insert((x, 64, 0), entity);
        }
        let c = Containers {
            mobs: &[],
            world: &world,
            behaviours: &behaviours,
            items: &items,
            registry,
            brewing: Some(&brewing),
        };
        for (x, &(block, kind, len, _, offered)) in (0..).zip(&cases) {
            let view = View {
                first: ((x, 64, 0), len),
                second: None,
                kind,
                container: None,
            };
            let offered = ItemStack::new(offered, 1);
            assert_eq!(c.capacity(&view, 0, &offered), 1, "{block}");
            assert_eq!(c.planning_room(&view, 0, &offered), 0, "{block}");
            assert_eq!(
                c.capacity(&view, 0, &ItemStack::new("minecraft:stick", 1)),
                0,
                "{block}"
            );
        }
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;
