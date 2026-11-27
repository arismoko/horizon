use crate::game::{
    GameLevel,
    api_values::{project_stack, stack_read_metadata},
    composter_data::is_composter,
    decorated_pot::DECORATED_POT,
    spill_common as spill,
};
use mod_api::{BlockCapabilitySite, BlockSide, ItemStackView, StorageSlot};
use std::{borrow::Cow, sync::Arc};
use sunrise_core::{geometry::Direction, nbt::Tag, position::BlockPos};
use sunrise_gameplay_common::{
    chests::is_shulker_box,
    hoppers::{Pos, cat_sitting_above, room},
};
use sunrise_items::{
    item_data::cooking_fuel,
    items::{ItemStack, Items},
};
use sunrise_level::{
    containers::{BlockEntity, ModContainer},
    world::World,
};
use sunrise_sim::{mobs::Mob, save};
use sunrise_world::{level::Behaviours, registry::BlockStateId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Generic,
    /// The saved decorated-pot slot: hopper insertion and the comparator signal.
    Pot,
    /// `AbstractFurnaceBlockEntity` (a `WorldlyContainer`).
    Furnace,
    Hopper,
    /// `CrafterBlockEntity`: disabled slots and the smallest-stack-first rule.
    Crafter,
    /// `BrewingStandBlockEntity` (a `WorldlyContainer`).
    Brewing,
    /// `ChiseledBookShelfBlockEntity`: books only, one to a slot.
    Bookshelf,
    /// `JukeboxBlockEntity`'s single slot: a playable disc only.
    Jukebox,
    /// A mod block's block entity (`ModContainer`).
    Mod,
    /// `ShulkerBoxBlockEntity`: a shulker box doesn't go through a face into one.
    Shulker,
    /// `ComposterBlock.getContainer` at this level (a `WorldlyContainerHolder`
    /// without a block entity): input below 7, empty at 7, bone meal at 8.
    Composter(i32),
}

/// A container: one block entity, or a double chest (`CompoundContainer`).
#[derive(Clone, Debug)]
pub struct View {
    pub first: (Pos, usize),
    /// The second half of vanilla's `CompoundContainer`, if this is a double chest.
    pub second: Option<(Pos, usize)>,
    pub kind: Kind,
    /// A mod block's container rules (`Kind::Mod`).
    pub container: Option<Arc<ModContainer>>,
}
impl View {
    pub fn parts(&self) -> impl Iterator<Item = (Pos, usize)> {
        std::iter::once(self.first).chain(self.second)
    }
    pub fn len(&self) -> usize {
        self.parts().map(|(_, size)| size).sum()
    }
    pub fn is_empty(&self) -> bool {
        self.parts().all(|(_, size)| size == 0)
    }

    pub fn locate(&self, mut slot: usize) -> Option<(Pos, usize)> {
        for (pos, size) in self.parts() {
            if slot < size {
                return Some((pos, slot));
            }
            slot -= size;
        }
        None
    }
    /// `WorldlyContainer.getSlotsForFace` or every slot, zero-allocation iterator.
    pub fn slots_for_face_iter(&self, face: Option<Direction>) -> Slots<'_> {
        match (self.kind, face) {
            (Kind::Furnace, Some(Direction::Down)) => Slots::Slice(&[2, 1]),
            (Kind::Furnace, Some(Direction::Up)) => Slots::Slice(&[0]),
            (Kind::Furnace, Some(_)) => Slots::Slice(&[1]),
            (Kind::Brewing, Some(Direction::Up)) => Slots::Slice(&[3]),
            (Kind::Brewing, Some(Direction::Down)) => Slots::Slice(&[0, 1, 2, 3]),
            (Kind::Brewing, Some(_)) => Slots::Slice(&[0, 1, 2, 4]),
            (Kind::Composter(8), Some(Direction::Down)) => Slots::Slice(&[0]),
            (Kind::Composter(level), Some(Direction::Up)) if level < 7 => Slots::Slice(&[0]),
            (Kind::Composter(_), _) => Slots::Slice(&[]),
            (Kind::Mod, Some(face)) => self
                .container
                .as_ref()
                .map_or(Slots::Slice(&[]), |c| Slots::Mod(&c.faces[face as usize])),
            _ => Slots::Range(0..self.len()),
        }
    }
    /// `Container.getMaxStackSize`: what one slot holds of any item, 1 for a chiseled
    /// bookshelf's books and a jukebox's disc, else vanilla's default.
    pub fn slot_limit(&self) -> u32 {
        match self.kind {
            Kind::Bookshelf | Kind::Jukebox | Kind::Composter(_) => 1,
            _ => 99,
        }
    }
    pub fn slots_to_add_iter(&self, face: Option<Direction>) -> Slots<'_> {
        if matches!(self.kind, Kind::Furnace | Kind::Brewing | Kind::Mod) && face.is_some() {
            self.slots_for_face_iter(face)
        } else {
            Slots::Range(0..self.len())
        }
    }
}

pub enum Slots<'a> {
    Range(std::ops::Range<usize>),
    Slice(&'static [usize]),
    Mod(&'a [usize]),
}

impl Iterator for Slots<'_> {
    type Item = usize;
    #[inline]
    fn next(&mut self) -> Option<usize> {
        match self {
            Slots::Range(r) => r.next(),
            Slots::Slice(s) => {
                let (&first, rest) = s.split_first()?;
                *s = rest;
                Some(first)
            }
            Slots::Mod(s) => {
                let (&first, rest) = s.split_first()?;
                *s = rest;
                Some(first)
            }
        }
    }
    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = match self {
            Slots::Range(r) => r.len(),
            Slots::Slice(s) => s.len(),
            Slots::Mod(s) => s.len(),
        };
        (len, Some(len))
    }
}
impl ExactSizeIterator for Slots<'_> {}

/// State-level container queries shared by the game and comparator signals.
pub struct Containers<'a> {
    pub world: &'a World,
    pub behaviours: &'a Behaviours,
    pub items: &'a Items,
    pub registry: &'static sunrise_world::registry::BlockRegistry,
    /// Brewing recipes, for what a brewing stand accepts (not needed for signals).
    pub brewing: Option<&'a sunrise_level::brewing::Brewing>,
    /// The level's mobs, for the cat half of `ChestBlock.isChestBlockedAt`.
    pub mobs: &'a [Mob],
}
/// The API snapshots for one offered stack. The offered count may shrink, and an
/// accepted write (or its notification) may change what the next slot sees.
#[derive(Default)]
pub struct PlacementAttempt {
    pub(in crate::game) stack: Option<Option<mod_api::ItemStack>>,
    pub(in crate::game) entity: Option<EntitySnapshot>,
}

/// Server-side `can_take_item_through_face` query passed into portable container rules.
pub trait TakeThroughFace {
    fn ask_take(
        self,
        pos: Pos,
        entity: &sunrise_level::containers::ModBlockEntity,
        slot: usize,
        stack: &ItemStack,
        face: Direction,
    ) -> Option<bool>;
}

pub enum PlacementQuery<'a> {
    Offer(&'a mut PlacementAttempt),
    Planning,
}

/// One mod block entity's API snapshot for a part hook, kept while its position and revision
/// stay.
pub struct EntitySnapshot {
    pos: Pos,
    revision: sunrise_level::chunk::block_entity::ModRevision,
    snapshot: PartSnapshot,
}

pub enum PartSnapshot {
    Unsubscribed,
    Unrepresentable,
    Ready(mod_api::ModBlockEntity),
}

impl PlacementAttempt {
    pub fn ask(
        &mut self,
        world: &World,
        pos: Pos,
        entity: &sunrise_level::containers::ModBlockEntity,
        slot: usize,
        stack: &ItemStack,
    ) -> Option<bool> {
        use super::api_values::TryIntoApi;

        if stack.count == 0 || !mod_api::__private::has_part_hook(mod_api::HookId::CanPlaceItem) {
            return None;
        }
        let snapshot = match EntitySnapshot::current(
            &mut self.entity,
            mod_api::HookId::CanPlaceItem,
            pos,
            entity,
        ) {
            PartSnapshot::Unsubscribed => return None,
            PartSnapshot::Unrepresentable => return Some(false),
            PartSnapshot::Ready(snapshot) => snapshot,
        };
        let Some(offered) = self.stack.get_or_insert_with(|| {
            stack
                .try_into_api()
                .inspect_err(|error| {
                    log::debug!(
                        "Sunrise: can_place_item could not read the offered {}: {error:?}",
                        stack.item
                    );
                })
                .ok()
        }) else {
            return Some(false);
        };
        if offered.count() != stack.count && offered.set_count(stack.count).is_err() {
            return Some(false);
        }
        let (x, y, z) = pos;
        let state = world.block(x, y, z);
        let state = match state.try_into_api() {
            Ok(state) => state,
            Err(error) => {
                log::debug!(
                    "Sunrise: can_place_item could not read block state at {pos:?}: {error:?}"
                );
                return Some(false);
            }
        };
        let mut allowed = None;
        crate::game::native_hooks::native_can_place_item(
            snapshot,
            mod_api::BlockPos::new(x, y, z),
            state,
            slot,
            offered,
            &mut allowed,
        );
        allowed
    }

    pub fn wrote_entity(&mut self) {
        self.entity = None;
    }
}

impl EntitySnapshot {
    /// The snapshot `cached` keeps of `entity` at `pos` for `hook`, read again when the
    /// position or the entity's revision changed.
    pub fn current<'s>(
        cached: &'s mut Option<Self>,
        hook: mod_api::HookId,
        pos: Pos,
        entity: &sunrise_level::containers::ModBlockEntity,
    ) -> &'s PartSnapshot {
        use super::api_values::TryIntoApi;

        let revision = entity.revision();
        let stale = cached
            .as_ref()
            .is_none_or(|seen| seen.pos != pos || seen.revision != revision);
        let seen = if stale {
            let snapshot = if mod_api::__private::block_entity_type_listens(hook, entity.id()) {
                match entity.try_into_api() {
                    Ok(snapshot) => PartSnapshot::Ready(snapshot),
                    Err(error) => {
                        log::debug!(
                            "Sunrise: a part hook could not read {} at {pos:?}: {error:?}",
                            entity.id()
                        );
                        PartSnapshot::Unrepresentable
                    }
                }
            } else {
                PartSnapshot::Unsubscribed
            };
            cached.insert(EntitySnapshot {
                pos,
                revision,
                snapshot,
            })
        } else {
            cached.as_mut().expect("a kept snapshot")
        };
        &seen.snapshot
    }
}

impl Containers<'_> {
    pub fn prop(&self, state: BlockStateId, key: &str) -> Option<&str> {
        self.registry
            .state(state)?
            .properties
            .get(key)
            .map(String::as_str)
    }
    pub(in crate::game) fn name(&self, state: BlockStateId) -> &str {
        self.registry.state(state).map_or("", |s| s.name.as_str())
    }
    /// `ItemInstance.getMaxStackSize`, using the effective component after patching.
    pub fn max_stack(&self, stack: &ItemStack) -> u32 {
        let info = self.items.get(&stack.item);
        match stack.effective_max_stack_size(info) {
            Some(max) => max.clamp(1, 99) as u32,
            // Only unchecked legacy stacks can have a malformed component here.
            None => info.map_or(64, |i| i.max_stack).max(1),
        }
    }
    /// `Container.getMaxStackSize(stack)` of a container: the container's own
    /// limit, at most the item's. `ComposterBlock.InputContainer` (levels 0 to
    /// 6) and `OutputContainer` (level 8, `READY`), `JukeboxBlockEntity` and
    /// `ChiseledBookShelfBlockEntity` override `getMaxStackSize` to 1; every
    /// other container keeps the `Container` default of 99. A composter's
    /// level 7 is `EmptyContainer` (99), which takes nothing through a face.
    pub fn max_stack_in(&self, view: &View, stack: &ItemStack) -> u32 {
        let container = match view.kind {
            Kind::Composter(0..=6) | Kind::Composter(8) | Kind::Jukebox | Kind::Bookshelf => 1,
            _ => 99,
        };
        container.min(self.max_stack(stack))
    }
    /// `HopperBlockEntity.getBlockContainer` (double chests combined, right half
    /// first). `blocked_chests`: `ChestBlock.getContainer(.., false)` returns
    /// nothing for a blocked chest (`isChestBlockedAt`: a redstone conductor
    /// above it or a cat sitting on it).
    pub fn view(&self, pos: Pos, blocked_chests: bool) -> Option<View> {
        let state = self.world.block(pos.0, pos.1, pos.2);
        let name = self.name(state);
        if is_composter(name) {
            let level = self.prop(state, "level").and_then(|l| l.parse().ok())?;
            return Some(View {
                first: (pos, usize::from(level != 7)),
                second: None,
                kind: Kind::Composter(level),
                container: None,
            });
        }
        let entity = self.world.block_entities.get(&pos)?;
        // A mod's block entity is a container when its block says so.
        if matches!(entity, BlockEntity::Mod(_)) {
            let container = Arc::clone(self.world.block_entities.mod_container(state.0)?);
            return Some(View {
                first: (pos, container.slots),
                second: None,
                kind: Kind::Mod,
                container: Some(container),
            });
        }
        // Non-container block entities (signs, banners, beacons, skulls, etc.,
        // or beehives/campfires/brushables) are not `Container`s
        // (HopperBlockEntity.getContainerAt), so a dropper drops in front of
        // them and a hopper leaves them alone.
        let size = entity.slots().len();
        if size == 0 {
            return None;
        }
        let kind = match (entity, name) {
            (BlockEntity::Furnace(_), _) => Kind::Furnace,
            (BlockEntity::Crafter(_), _) => Kind::Crafter,
            (BlockEntity::Brewing(_), _) => Kind::Brewing,
            (BlockEntity::ChiseledBookshelf(_), _) => Kind::Bookshelf,
            (BlockEntity::Jukebox(_), _) => Kind::Jukebox,
            (_, "minecraft:hopper") => Kind::Hopper,
            (_, n) if is_shulker_box(n) => Kind::Shulker,
            _ => Kind::Generic,
        };
        if sunrise_level::containers::is_chest_block(name) {
            let is_blocked = |p: Pos| {
                let above = self.world.block(p.0, p.1 + 1, p.2);
                self.behaviours.facts(above).flags
                    & sunrise_world::level::data::state_flags::CONDUCTOR
                    != 0
                    || cat_sitting_above(self.mobs, p)
            };
            if blocked_chests && is_blocked(pos) {
                return None;
            }
            let half = |s: BlockStateId| {
                Some(sunrise_level::containers::ChestHalf {
                    block: self.name(s),
                    kind: self.prop(s, "type")?,
                    facing: self.prop(s, "facing")?,
                })
            };
            if let Some(own) = half(state)
                && let Some((dx, dz)) = own.connected_step()
            {
                let other = (pos.0 + dx, pos.1, pos.2 + dz);
                let other_state = self.world.block(other.0, other.1, other.2);
                let paired = half(other_state).is_some_and(|o| own.pairs_with(&o))
                    && self
                        .world
                        .block_entities
                        .get(&other)
                        .is_some_and(|e| e.slots().len() == 27);
                if paired {
                    if blocked_chests && is_blocked(other) {
                        return None;
                    }
                    let (first, second) = if own.kind == "right" {
                        ((pos, size), (other, 27))
                    } else {
                        ((other, 27), (pos, size))
                    };
                    return Some(View {
                        first,
                        second: Some(second),
                        kind,
                        container: None,
                    });
                }
            }
        }
        Some(View {
            first: (pos, size),
            second: None,
            kind,
            container: None,
        })
    }
    /// The saved decorated-pot slot as a container: `DecoratedPotBlockEntity`
    /// is a `ContainerSingleItem`, but Sunrise keeps pots out of generic
    /// container discovery (`world.block_entities`), so this is built on demand.
    /// A pot with pending loot is left to its existing unpacking path.
    pub(in crate::game) fn saved_pot_view(&self, pos: Pos) -> Option<View> {
        if self.name(self.world.block(pos.0, pos.1, pos.2)) != "minecraft:decorated_pot"
            || spill::saved_entity(self.world, [pos.0, pos.1, pos.2])
                .is_some_and(|tag| matches!(tag.get("LootTable"), Some(Tag::String(_))))
        {
            return None;
        }
        Some(View {
            first: (pos, 1),
            second: None,
            kind: Kind::Pot,
            container: None,
        })
    }
    /// `HopperBlockEntity.getBlockContainer` at its insertion target only.
    /// Pots keep their authoritative slot in the chunk's saved tag.
    pub fn hopper_insertion_view(&self, pos: Pos) -> Option<View> {
        self.view(pos, false).or_else(|| self.saved_pot_view(pos))
    }
    /// The `ContainerSingleItem` slot for hopper insertion: only a saved pot
    /// needs an owned decode. Existing modeled containers stay borrowed.
    pub fn insertion_item(&self, view: &View, slot: usize) -> Option<Cow<'_, ItemStack>> {
        if view.kind == Kind::Pot {
            let (pos, _) = view.locate(slot)?;
            let tag = spill::saved_entity(self.world, [pos.0, pos.1, pos.2])?;
            return tag
                .get("item")
                .and_then(save::item_from_nbt)
                .filter(|stack| stack.count > 0)
                .map(Cow::Owned);
        }
        self.get(view, slot).map(Cow::Borrowed)
    }
    pub fn get(&self, view: &View, slot: usize) -> Option<&ItemStack> {
        if let Kind::Composter(level) = view.kind {
            // OutputContainer holds one bone meal; InputContainer composts at once.
            return (level == 8 && slot == 0).then_some(bone_meal_stack());
        }
        let (pos, i) = view.locate(slot)?;
        self.world
            .block_entities
            .get(&pos)?
            .slots()
            .get(i)?
            .as_ref()
            // A mod can leave an empty stack in its own slots.
            .filter(|s| s.count > 0)
    }
    pub fn is_empty(&self, view: &View) -> bool {
        (0..view.len()).all(|i| self.get(view, i).is_none())
    }
    /// `Container.canPlaceItem` (+ `canPlaceItemThroughFace` for furnaces) with slot `slot`
    /// empty: the item rule of the slot, whatever it holds now.
    pub(in crate::game) fn accepts(&self, view: &View, slot: usize, stack: &ItemStack) -> bool {
        if let Some(c) = &view.container {
            return c.inputs.contains(&slot);
        }
        match view.kind {
            // ComposterBlock.InputContainer.canPlaceItemThroughFace; output never accepts.
            Kind::Composter(level) => {
                level < 7
                    && slot == 0
                    && self
                        .items
                        .get(&stack.item)
                        .is_some_and(|i| i.compostable.is_some())
            }
            // ShulkerBoxBlockEntity.canPlaceItemThroughFace
            Kind::Shulker => !is_shulker_box(&stack.item),
            Kind::Crafter => match self.world.block_entities.get(&view.first.0) {
                Some(BlockEntity::Crafter(c)) => slot < 9 && !c.disabled[slot],
                _ => false,
            },
            // JukeboxBlockEntity.canPlaceItem: a playable disc.
            Kind::Jukebox => self
                .items
                .get(&stack.item)
                .is_some_and(|i| i.jukebox_song.is_some()),
            // ListBackedContainer.canPlaceItem: `acceptsItemType` (the books).
            Kind::Bookshelf => self
                .items
                .item_tags
                .get("minecraft:bookshelf_books")
                .is_some_and(|m| m.contains(&*stack.item)),
            // BrewingStandBlockEntity.canPlaceItem
            Kind::Brewing => match slot {
                4 => sunrise_level::brewing::brewing_fuel(self.items, stack).is_some(),
                3 => self.brewing.is_some_and(|b| b.is_reagent(stack)),
                _ => {
                    self.brewing.is_some_and(|b| b.is_bottle(stack))
                        || self
                            .items
                            .item_tags
                            .get("minecraft:brewing_potion_inputs")
                            .is_some_and(|m| m.contains(&*stack.item))
                }
            },
            Kind::Furnace => match slot {
                2 => false,
                1 => {
                    &*stack.item == "minecraft:bucket"
                        || cooking_fuel::for_stack(self.items, stack).is_some()
                }
                _ => true,
            },
            _ => true,
        }
    }
    /// `Container.canPlaceItem` (+ `canPlaceItemThroughFace` for furnaces): `accepts`, and what
    /// the slot holds now. The slots of one item (a jukebox's, a chiseled bookshelf's, a brewing
    /// stand's bottles) take one only when empty, a furnace's fuel slot no bucket onto a
    /// bucket, and a crafter fills its smallest stack first. Planning has no offered amount,
    /// so it must not veto a slot that a subscribed part could admit.
    pub fn can_place(
        &self,
        view: &View,
        slot: usize,
        stack: &ItemStack,
        query: PlacementQuery<'_>,
    ) -> bool {
        if !self.accepts(view, slot, stack) {
            return false;
        }
        if let Some(container) = &view.container {
            let Some((pos, index)) = view.locate(slot) else {
                return false;
            };
            let Some(BlockEntity::Mod(entity)) = self.world.block_entities.get(&pos) else {
                return false;
            };
            let answer = match query {
                PlacementQuery::Offer(attempt) => {
                    attempt.ask(self.world, pos, entity, index, stack)
                }
                PlacementQuery::Planning => mod_api::__private::block_entity_type_listens(
                    mod_api::HookId::CanPlaceItem,
                    entity.id(),
                )
                .then_some(true),
            };
            return answer.unwrap_or_else(|| {
                sunrise_level::containers::mod_container_insert_allowed(
                    container
                        .insert_rules
                        .get(slot)
                        .copied()
                        .unwrap_or_default(),
                    self.get(view, slot),
                    stack,
                )
            });
        }
        let held = self.get(view, slot);
        let is_bucket = |s: &ItemStack| &*s.item == "minecraft:bucket";
        match view.kind {
            Kind::Jukebox | Kind::Bookshelf => held.is_none(),
            Kind::Brewing => slot >= 3 || held.is_none(),
            Kind::Furnace => {
                slot != 1
                    || !is_bucket(stack)
                    || !held.is_some_and(is_bucket)
                    || cooking_fuel::for_stack(self.items, stack).is_some()
            }
            Kind::Crafter => match self.world.block_entities.get(&view.first.0) {
                Some(BlockEntity::Crafter(c)) => {
                    // The slot's own stack sets the limit (`slotStack.getMaxStackSize`).
                    let max = c.items.get(slot).and_then(Option::as_ref).unwrap_or(stack);
                    c.can_place(slot, stack, self.max_stack(max))
                }
                _ => false,
            },
            _ => true,
        }
    }
    /// How many of `stack` slot `slot` holds when empty, whatever it holds now: none where
    /// `accepts` refuses it, else the slot's limit (`Container.getMaxStackSize(stack)`: the
    /// container's and the item's, the lower).
    pub fn capacity(&self, view: &View, slot: usize, stack: &ItemStack) -> u32 {
        if !self.accepts(view, slot, stack) {
            return 0;
        }
        self.slot_stack_limit(view, slot, Some(stack))
    }
    pub fn slot_stack_limit(&self, view: &View, slot: usize, stack: Option<&ItemStack>) -> u32 {
        let capacity = stack.map_or(view.slot_limit(), |s| {
            view.slot_limit().min(self.max_stack(s))
        });
        view.locate(slot)
            .and_then(|(pos, _)| self.world.container_stack_limit(pos))
            .map_or(capacity, |limit| capacity.min(limit))
    }
    /// For planning, physical room subject to rules a part cannot override.
    pub fn planning_room(&self, view: &View, slot: usize, stack: &ItemStack) -> u32 {
        if !self.can_place(view, slot, stack, PlacementQuery::Planning) {
            return 0;
        }
        self.insertion_room(view, slot, stack)
    }
    pub fn insertion_room(&self, view: &View, slot: usize, stack: &ItemStack) -> u32 {
        room(
            self.slot_stack_limit(view, slot, Some(stack)),
            self.get(view, slot),
            stack,
        )
    }
    /// `canTakeItemThroughFace` (furnaces and brewing stands restrict it).
    /// `Container.canTakeItem` for a hopper's slot of `source`: the vanilla
    /// shelf rule needs to know where the item is going.
    pub fn can_take_into<A: TakeThroughFace>(
        &self,
        view: &View,
        slot: usize,
        stack: &ItemStack,
        face: Direction,
        target: &View,
        ask: A,
    ) -> bool {
        if view.kind == Kind::Bookshelf {
            // `ChiseledBookShelfBlockEntity.canTakeItem`: the target must have
            // a free slot or a matching stack with room (`hasAnyMatching`).
            return (0..target.len()).any(|i| match self.get(target, i) {
                None => true,
                Some(other) => {
                    other.same_item_same_components(stack)
                        && other.count + stack.count <= self.max_stack(other)
                }
            });
        }
        self.can_take(view, slot, stack, face, ask)
    }
    pub fn can_take<A: TakeThroughFace>(
        &self,
        view: &View,
        slot: usize,
        stack: &ItemStack,
        face: Direction,
        ask: A,
    ) -> bool {
        if let Kind::Composter(level) = view.kind {
            // OutputContainer.canTakeItemThroughFace.
            return level == 8 && face == Direction::Down && &*stack.item == "minecraft:bone_meal";
        }
        if view.kind == Kind::Brewing {
            return slot != 3 || &*stack.item == "minecraft:glass_bottle";
        }
        if let Some(c) = &view.container {
            return self
                .ask_take_through_face(view, slot, stack, face, ask)
                .unwrap_or_else(|| c.outputs.contains(&slot));
        }
        view.kind != Kind::Furnace
            || face != Direction::Down
            || slot != 1
            || matches!(&*stack.item, "minecraft:water_bucket" | "minecraft:bucket")
    }
    /// A mod container's `can_take_item_through_face` answer for `slot` of `view`, asked
    /// only when some part subscribes.
    fn ask_take_through_face<A: TakeThroughFace>(
        &self,
        view: &View,
        slot: usize,
        stack: &ItemStack,
        face: Direction,
        ask: A,
    ) -> Option<bool> {
        if !mod_api::__private::has_part_hook(mod_api::HookId::CanTakeItemThroughFace) {
            return None;
        }
        let (pos, index) = view.locate(slot)?;
        let Some(BlockEntity::Mod(entity)) = self.world.block_entities.get(&pos) else {
            return None;
        };
        ask.ask_take(pos, entity, index, stack, face)
    }
    pub(in crate::game) fn comparator_view(&self, pos: Pos) -> Option<View> {
        self.view(pos, true)
            .filter(|v| !matches!(v.kind, Kind::Composter(_)))
            .or_else(|| self.saved_pot_view(pos))
    }
    /// Comparator signal of stored slots, without materializing deferred loot.
    pub fn stored_container_signal(&self, pos: Pos) -> i32 {
        self.comparator_view(pos)
            .map_or(0, |view| self.signal(&view))
    }
    /// `AbstractContainerMenu.getRedstoneSignalFromContainer`.
    pub(in crate::game) fn signal(&self, view: &View) -> i32 {
        if view.kind == Kind::Crafter {
            // CrafterBlockEntity.getRedstoneSignal: filled or disabled slots.
            return match self.world.block_entities.get(&view.first.0) {
                Some(BlockEntity::Crafter(c)) => c.redstone_signal(),
                _ => 0,
            };
        }
        let mut total = 0.0f32;
        for i in 0..view.len() {
            // `insertion_item` decodes a saved pot's slot; every other kind is `get`.
            if let Some(s) = self.insertion_item(view, i) {
                total += s.count as f32 / self.max_stack(&s) as f32;
            }
        }
        total /= view.len().max(1) as f32;
        // Mth.lerpDiscrete(total, 0, 15). A mod can write a stack over its own
        // limit, so the ratio itself is clamped, as vanilla's invariant does.
        (total.min(1.0) * 14.0).floor() as i32 + i32::from(total > 0.0)
    }
}

pub fn bone_meal_stack() -> &'static ItemStack {
    static STACK: std::sync::LazyLock<ItemStack> = std::sync::LazyLock::new(|| ItemStack {
        item: "minecraft:bone_meal".into(),
        count: 1,
        damage: 0,
        extra: None,
    });
    &STACK
}

/// Comparator input from a container block entity (0 when there is none):
/// `ChestBlock.getContainer(.., false)`, whose blocked check sees the level's
/// cats (`isCatSittingOnChest`).
pub(in crate::game) fn container_signal(level: &mut GameLevel<'_>, pos: BlockPos) -> i32 {
    let view = level.containers().comparator_view((pos.x, pos.y, pos.z));
    if let Some(view) = &view {
        for (key, _) in view.parts() {
            level.unpack_loot(
                crate::game::container_loot_level::LootTarget::Block(key),
                None,
            );
        }
    }
    let c = level.containers();
    view.map_or(0, |v| c.signal(&v))
}

impl GameLevel<'_> {
    pub(in crate::game) fn containers(&self) -> Containers<'_> {
        Containers {
            world: self.world,
            behaviours: self.behaviours,
            items: self.items,
            registry: self.registry,
            brewing: None,
            mobs: self.entities().mobs,
        }
    }
}

impl View {
    /// Composter capability ports keep one slot as their level changes; hopper views retain
    /// vanilla's level-dependent slot enumeration.
    pub fn capability_slots(&self, face: Option<Direction>) -> Slots<'_> {
        if matches!(self.kind, Kind::Composter(_)) {
            Slots::Range(0..1)
        } else {
            self.slots_for_face_iter(face)
        }
    }

    /// [`Containers::capability_slot_backing`] over a built view.
    pub fn slot_backing(&self, at: BlockSide, slot: usize) -> Option<StorageSlot> {
        if matches!(self.kind, Kind::Composter(_)) {
            return None;
        }
        let backing_index = self.capability_slots(at.side).nth(slot)?;
        let (pos, own_slot) = self.locate(backing_index)?;
        Some(StorageSlot {
            level: at.level,
            pos: [pos.0, pos.1, pos.2],
            slot: own_slot,
        })
    }
}

/// What the game offers without looking at any stack contents or walking a double chest.
enum ItemTarget {
    Composter(i32),
    Pot,
    Container,
}

/// `Level.isInValidBounds` and the resident-only `getChunkNow` / `getChunk(..., false)` query.
fn valid_resident_position(world: &sunrise_level::world::World, [x, y, z]: [i32; 3]) -> bool {
    sunrise_level::chunk_streamer::chunk_pos_is_valid((x >> 4, z >> 4))
        && world.is_loaded_at(x, y, z)
}

impl Containers<'_> {
    fn item_target(
        &self,
        state: BlockStateId,
        side: Option<Direction>,
        entity: Option<&BlockEntity>,
    ) -> Option<ItemTarget> {
        match self.name(state) {
            // NeoForge `ComposterWrapper.get`: no unsided or horizontal port. Inactive
            // ports use the level-7 container's transfer rules but still have a handler.
            "minecraft:composter" => {
                let level = self.prop(state, "level")?.parse().ok()?;
                match side {
                    Some(Direction::Up) if level < 7 => Some(ItemTarget::Composter(level)),
                    Some(Direction::Down) if level == 8 => Some(ItemTarget::Composter(8)),
                    Some(Direction::Up | Direction::Down) => Some(ItemTarget::Composter(7)),
                    _ => None,
                }
            }
            // The pot's implicit entity and saved slot use the block's resident state, not
            // `world.block_entities`. A new empty pot need not have a saved tag yet.
            DECORATED_POT => Some(ItemTarget::Pot),
            _ => match entity? {
                BlockEntity::Mod(_) => self
                    .world
                    .block_entities
                    .mod_container(state.0)
                    .map(|_| ItemTarget::Container),
                entity => (!entity.slots().is_empty()).then_some(ItemTarget::Container),
            },
        }
    }

    pub fn capability_site(
        &self,
        at: BlockSide,
        state: BlockStateId,
        entity_removed: bool,
    ) -> Option<BlockCapabilitySite> {
        use crate::game::{api_values::TryIntoApi, native_block_entity_read::block_entity_handle};
        if !valid_resident_position(self.world, at.pos) {
            return None;
        }
        let [x, y, z] = at.pos;
        let entity = if entity_removed {
            None
        } else {
            self.world.block_entities.get(&(x, y, z))
        };
        let resident = entity.and_then(|entity| {
            let handle = block_entity_handle(self.world, at.level, at.pos)?;
            let kind = mod_api::__private::block_entity_type_key(save::block_entity_id(
                entity,
                self.name(state),
            ));
            Some((handle, kind))
        });
        Some(BlockCapabilitySite::new(
            at,
            state.try_into_api().ok()?,
            resident,
            self.item_target(state, at.side, entity).is_some(),
        ))
    }

    pub fn capability_view(&self, at: BlockSide) -> Option<View> {
        if !valid_resident_position(self.world, at.pos) {
            return None;
        }
        let [x, y, z] = at.pos;
        let pos = (x, y, z);
        let state = self.world.block(x, y, z);
        let entity = self.world.block_entities.get(&pos);
        match self.item_target(state, at.side, entity)? {
            ItemTarget::Composter(level) => Some(View {
                first: (pos, 1),
                second: None,
                kind: Kind::Composter(level),
                container: None,
            }),
            ItemTarget::Pot => Some(View {
                first: (pos, 1),
                second: None,
                kind: Kind::Pot,
                container: None,
            }),
            ItemTarget::Container => self.view(pos, false),
        }
    }
    pub fn capability_size(&self, at: BlockSide) -> Option<usize> {
        Some(self.capability_view(at)?.capability_slots(at.side).count())
    }
    pub fn lend_capability_stack(
        &self,
        at: BlockSide,
        slot: usize,
        read: &mut dyn for<'a> FnMut(ItemStackView<'a>),
    ) {
        let Some(view) = self.capability_view(at) else {
            return;
        };
        let Some(slot) = view.capability_slots(at.side).nth(slot) else {
            return;
        };
        let Some(stack) = self.insertion_item(&view, slot) else {
            return;
        };
        let Ok(metadata) = stack_read_metadata(&stack) else {
            return;
        };
        let project = |metadata| project_stack(&stack, metadata).ok();
        read(ItemStackView::resident(metadata, &project));
    }
    /// The block entity slot that slot `slot` of `at`'s side reaches (`WorldlyContainer.
    /// getSlotsForFace`), owned by the half that holds it in a double chest (`ChestBlock.
    /// getContainer`, `CompoundContainer`). A composter has no block entity: vanilla builds its
    /// container per access (`ComposterBlock.getContainer`), so it has no backing.
    pub fn capability_slot_backing(&self, at: BlockSide, slot: usize) -> Option<StorageSlot> {
        let view = self.capability_view(at)?;
        view.slot_backing(at, slot)
    }
}
