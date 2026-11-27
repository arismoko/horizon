//! The game's own item capability (`mod_api::ITEMS`, NeoForge's `Capabilities.Item.BLOCK`):
//! container slots and the composter's state-backed input/output, by the rules hoppers follow.
//! Side-local slots are numbered from zero. It acts where the world is simulated, never on
//! a client's copy, and only on the level in place.
use super::{Insertion, MoveIn, TakeAsk, TakeAttempt};
use mod_api::{
    BlockSide, ItemResource, ItemStackView, LevelReadError, LevelRef, StorageSlot,
    TransactionContext,
};
use sunrise_gameplay_common::game::{
    machine_containers::{Containers, Kind, PlacementAttempt, View},
    spill_common::{put_saved_entity, saved_entity},
    transactions::After,
};
use sunrise_gameplay_common::hoppers::Pos;
use sunrise_items::items::ItemStack;
use sunrise_level::world::World;
use sunrise_sim::{mobs::Mob, save};

impl crate::game::ServerGame<'_> {
    /// Vanilla containers and `WorldlyContainerHolder`s keep their direct automation path.
    /// This owner check borrows storage; it never builds a container view on the tick path.
    pub(in crate::game::server_side) fn vanilla_automation_container(&self, pos: Pos) -> bool {
        use sunrise_level::containers::BlockEntity;
        match self.common.world.block_entities.get(&pos) {
            Some(
                BlockEntity::Mod(_)
                | BlockEntity::Beehive(_)
                | BlockEntity::Campfire(_)
                | BlockEntity::Brushable(_)
                | BlockEntity::EndPortal,
            ) => false,
            Some(entity) if !entity.slots().is_empty() => true,
            _ => matches!(
                self.res
                    .facts()
                    .get(self.world.block(pos.0, pos.1, pos.2))
                    .name
                    .as_ref(),
                "minecraft:composter" | "minecraft:decorated_pot"
            ),
        }
    }
    /// `HopperBlockEntity.inventoryFull`: every slot holds exactly a full item stack.
    pub(in crate::game::server_side) fn hopper_is_stacked(&self, view: &View) -> bool {
        let c = self.containers();
        (0..view.len()).all(|slot| {
            c.get(view, slot)
                .is_some_and(|stack| stack.count == c.max_stack(stack))
        })
    }
    /// `DropperBlock.dispenseFrom`: always commit a storage attempt, including refusal.
    /// Accepted source removal precedes root-commit callbacks that can re-enter the source.
    pub(in crate::game::server_side) fn dropper_into_part(
        &mut self,
        part: &mod_api::Capability<dyn mod_api::ResourceHandler<mod_api::ItemResource>>,
        stack: &ItemStack,
        source: &View,
        slot: usize,
    ) -> bool {
        use sunrise_gameplay_common::game::api_values::TryIntoApi;
        let Ok(snapshot) = stack.try_into_api() else {
            return false;
        };
        let resource = mod_api::ItemResource::of(&snapshot);
        self.storage_transaction(|game, tx| {
            let (pos, index) = source.locate(slot).ok_or(())?;
            game.journal_block_entity_slot(pos, index);
            // Reserve the offered item before writable insertion notices can re-enter.
            let reserved = game.take_one(source, slot).ok_or(())?;
            // A part answering more than it was offered is a broken mod: abort, undoing
            // whatever its insertion moved. A plain refusal still commits.
            let accepted = part.insert(game, tx, &resource, 1).map_err(|_| ())? == 1;
            if !accepted {
                match game.slot_mut_at(source, slot).ok_or(())? {
                    Some(held) if held.same_item_same_components(&reserved) => {
                        held.count = held.count.checked_add(1).ok_or(())?;
                    }
                    empty @ None => *empty = Some(reserved),
                    // A writable notice replaced the slot. Only aborting can restore the
                    // reserved item without deleting that replacement; this exceptional
                    // refusal rolls back the notice's storage writes with the attempt.
                    Some(_) => return Err(()),
                }
            }
            Ok::<_, ()>(accepted)
        })
        .unwrap_or(false)
    }
    /// Whether this game simulates a world, as `native_set_block` asks
    /// (the fluid helpers ask it too).
    pub fn native_simulates(&self) -> bool {
        self.common.is_logical_server()
    }
    /// The level in place, as a handle: where the game's capability lookups (a hopper's) are.
    /// None where no world is simulated.
    pub fn native_level_here(&self) -> Option<LevelRef> {
        let epoch = self.common.native_lifecycle.epoch()?;
        self.native_simulates()
            .then(|| self.level_ref(epoch, self.common.dim.key()))
    }
}

impl crate::game::ServerGame<'_> {
    /// Up to `count` out of slot `slot` of `view`, without `setChanged`.
    fn take_up_to(&mut self, view: &View, slot: usize, count: u32) -> Option<ItemStack> {
        if matches!(view.kind, Kind::Composter(_)) {
            let (x, y, z) = view.first.0;
            return self.composter_capability_extract([x, y, z], count);
        }
        if view.kind == Kind::Pot {
            let pos = view.first.0;
            let at = [pos.0, pos.1, pos.2];
            let mut tag = saved_entity(&self.common.world, at)?;
            let mut item = tag.get("item").and_then(save::item_from_nbt)?;
            let taken = count.min(item.count);
            let out = ItemStack {
                count: taken,
                ..item.clone()
            };
            item.count -= taken;
            Self::set_pot_slot(
                self.common.res.items(),
                &mut tag,
                (item.count != 0).then_some(item),
            );
            put_saved_entity(&mut self.common.world, at, tag);
            return Some(out);
        }
        self.containers().get(view, slot)?;
        let target = self.slot_mut_at(view, slot)?;
        let current = target.as_mut()?;
        let taken = count.min(current.count);
        let out = ItemStack {
            count: taken,
            ..current.clone()
        };
        current.count -= taken;
        if current.count == 0 {
            *target = None;
        }
        Some(out)
    }
    /// The container at `at` the item capability reaches, including the composter's
    /// state-backed ports. Inactive composter ports use its inert level-7 rules, so an input
    /// can never expose bone meal and an output can never accept compost.
    fn capability_view(&self, at: BlockSide) -> Option<View> {
        if !self.level_in_place(at.level) {
            return None;
        }
        self.containers().capability_view(at)
    }
    pub(in crate::game) fn container_set_stack_limit(
        &mut self,
        at: BlockSide,
        limit: u32,
        tx: &TransactionContext,
    ) -> bool {
        if !self.transaction_context_is_current(tx) {
            return false;
        }
        if !(1..=99).contains(&limit) {
            return false;
        }
        let Some(view) = self.capability_view(at) else {
            return false;
        };
        if matches!(view.kind, Kind::Composter(_)) {
            return false;
        }
        for (pos, _) in view.parts() {
            self.journal_block_entity(pos);
            self.common.world.set_container_stack_limit(pos, limit);
        }
        self.view_told(&view);
        true
    }
    /// How many slots `at`'s side reaches, including a composter's one-slot ports.
    /// None without supported storage, in a level not in place, or on a client's copy.
    pub(in crate::game) fn container_size(&self, at: BlockSide) -> Option<usize> {
        self.level_in_place(at.level).then_some(())?;
        self.containers().capability_size(at)
    }

    /// Lend the resident stack in a side-local slot of the level in place.
    pub(in crate::game) fn lend_container_stack(
        &self,
        at: BlockSide,
        slot: usize,
        read: &mut dyn for<'a> FnMut(ItemStackView<'a>),
    ) {
        if self.level_in_place(at.level) {
            self.containers().lend_capability_stack(at, slot, read);
        }
    }

    /// The block entity slot that slot `slot` of those `at`'s side reaches (`container_size`) is:
    /// each slot of a double chest reports the half that owns it, and a composter has none.
    pub(in crate::game) fn container_slot_backing(
        &self,
        at: BlockSide,
        slot: usize,
    ) -> Option<StorageSlot> {
        self.level_in_place(at.level).then_some(())?;
        self.containers().capability_slot_backing(at, slot)
    }
    pub(in crate::game) fn level_containers<'a>(
        &'a self,
        world: &'a World,
        mobs: &'a [Mob],
    ) -> Containers<'a> {
        Containers {
            world,
            behaviours: self.res.behaviours(),
            items: self.items(),
            registry: self.pack.assets.registry,
            brewing: None,
            mobs,
        }
    }

    /// The item capability read used by a level-bound Part, including parked levels.
    pub(in crate::game) fn level_container_size(
        &self,
        world: &World,
        mobs: &[Mob],
        at: BlockSide,
    ) -> Option<usize> {
        self.level_containers(world, mobs).capability_size(at)
    }
    /// Lend a resident stack from a level-bound Part's addressed world, including parked levels.
    pub(in crate::game) fn level_lend_container_stack(
        &self,
        world: &World,
        mobs: &[Mob],
        at: BlockSide,
        slot: usize,
        read: &mut dyn for<'a> FnMut(ItemStackView<'a>),
    ) {
        self.level_containers(world, mobs)
            .lend_capability_stack(at, slot, read);
    }
    /// Keeps what a capability write to slot `slot` of `view` changes: that slot alone where a
    /// write there changes nothing else, else each block entity of the view whole.
    pub(in crate::game::server_side) fn journal_view_slot(&mut self, view: &View, slot: usize) {
        match view.kind {
            Kind::Composter(_) => return,
            Kind::Pot => return self.journal_saved_pot(view.first.0),
            _ => {}
        }
        let slot_only = match view.kind {
            // Plain item slots: no state beside the slot changes.
            Kind::Generic | Kind::Shulker | Kind::Mod => true,
            // State beside the slot: filling an empty input restarts a furnace's cooking; an
            // insert into an empty hopper sets its cooldown; a jukebox's disc starts its song;
            // a bookshelf's books are in its block state and last slot.
            Kind::Furnace | Kind::Hopper | Kind::Jukebox | Kind::Bookshelf => false,
            // Not shown to change only the slot (a crafter's disabled slots, a brewing stand's
            // brew), so kept whole as before.
            Kind::Crafter | Kind::Brewing => false,
            Kind::Pot | Kind::Composter(_) => unreachable!("handled above"),
        };
        if slot_only && let Some((pos, slot)) = view.locate(slot) {
            return self.journal_block_entity_slot(pos, slot);
        }
        for (part, _) in view.parts() {
            self.journal_block_entity(part);
        }
    }
    /// Default capability writes to engine-managed containers notify once at root commit.
    fn view_told(&mut self, view: &View) {
        if matches!(view.kind, Kind::Composter(_)) {
            return;
        }
        for (pos, _) in view.parts() {
            self.after_commit(After::Changed(pos));
        }
    }
    /// Puts as much of `stack` as slot `slot` of those `at`'s side reaches takes, as a hopper
    /// would (`canPlaceItem`, the slot's limit, the same item and components), in transaction
    /// `tx`; what didn't fit.
    pub(in crate::game) fn container_insert(
        &mut self,
        at: BlockSide,
        slot: usize,
        resource: &ItemStack,
        amount: u32,
        tx: &TransactionContext,
    ) -> Option<ItemStack> {
        if !self.transaction_context_is_current(tx) {
            return Some(ItemStack {
                count: amount,
                ..resource.clone()
            });
        }
        let stack = ItemStack {
            count: amount,
            ..resource.clone()
        };
        let Some(view) = self.capability_view(at) else {
            return Some(stack);
        };
        let Some(slot) = view.capability_slots(at.side).nth(slot) else {
            return Some(stack);
        };
        if matches!(view.kind, Kind::Composter(_)) {
            if amount == 0
                || self.containers().planning_room(&view, slot, &stack) == 0
                || !self.composter_capability_insert(at.pos, &stack)
            {
                return Some(stack);
            }
            return (amount > 1).then(|| ItemStack {
                count: amount - 1,
                ..stack
            });
        }
        let (rest, result) = self.move_in_item(
            Insertion::Capability,
            &view,
            stack,
            slot,
            &mut PlacementAttempt::default(),
        );
        // A jukebox's disc starts at the root commit (`JukeboxBlockEntity.setTheItem`).
        if result == MoveIn::Moved {
            self.view_told(&view);
        }
        rest
    }
    /// How much of `resource` the slots `at`'s side reaches hold in all (`STORAGE`'s amount),
    /// in one pass over one view of the container.
    /// None where the total does not fit in 64 bits.
    pub(in crate::game) fn container_kind_amount(
        &self,
        at: BlockSide,
        resource: &ItemStack,
    ) -> Option<u64> {
        let Some(view) = self.capability_view(at) else {
            return Some(0);
        };
        let c = self.containers();
        view.capability_slots(at.side)
            .filter_map(|slot| c.insertion_item(&view, slot))
            .filter(|held| held.same_item_same_components(resource))
            .try_fold(0u64, |total, held| total.checked_add(u64::from(held.count)))
    }
    /// Physical room through `at`'s side, subject to rules a part cannot override.
    /// This is a bound for planning, not an admission decision on an offered count.
    /// None where the total does not fit in 64 bits.
    pub(in crate::game) fn container_kind_space(
        &self,
        at: BlockSide,
        resource: &ItemStack,
    ) -> Option<u64> {
        let Some(view) = self.capability_view(at) else {
            return Some(0);
        };
        let c = self.containers();
        view.capability_slots(at.side)
            .try_fold(0u64, |total, slot| {
                total.checked_add(u64::from(c.planning_room(&view, slot, resource)))
            })
    }
    /// Visits the occupied slots `at`'s side reaches, in order, converting only reached stacks.
    pub(in crate::game) fn container_visit_entries(
        &self,
        at: BlockSide,
        visitor: &mut dyn FnMut(&ItemResource, u64) -> std::ops::ControlFlow<()>,
    ) -> Result<(), LevelReadError> {
        use sunrise_gameplay_common::game::api_values::TryIntoApi;
        let Some(view) = self.capability_view(at) else {
            return Ok(());
        };
        let c = self.containers();
        for stack in view
            .capability_slots(at.side)
            .filter_map(|slot| c.insertion_item(&view, slot))
        {
            let stack = stack
                .as_ref()
                .try_into_api()
                .map_err(|_| LevelReadError::Unavailable)?;
            if visitor(&ItemResource::of(&stack), u64::from(stack.count())).is_break() {
                break;
            }
        }
        Ok(())
    }
    /// Puts up to `amount` of `resource` into the slots `at`'s side reaches, those holding it
    /// first and then the empty ones, in transaction `tx`, on one view of the container: what
    /// went in. Each slot is kept and filled as `container_insert` keeps and fills it.
    pub(in crate::game) fn container_insert_kind(
        &mut self,
        at: BlockSide,
        resource: &ItemStack,
        amount: u64,
        tx: &TransactionContext,
    ) -> u64 {
        if !self.transaction_context_is_current(tx) {
            return 0;
        }
        if amount == 0 {
            return 0;
        }
        let Some(view) = self.capability_view(at) else {
            return 0;
        };
        if matches!(view.kind, Kind::Composter(_)) {
            return u64::from(
                self.containers().capacity(&view, 0, resource) > 0
                    && self.composter_capability_insert(at.pos, resource),
            );
        }
        let (mut left, mut told) = (amount, false);
        let mut attempt = PlacementAttempt::default();
        for holding in [true, false] {
            for slot in view.capability_slots(at.side) {
                if left == 0 {
                    break;
                }
                let pick = match self.containers().insertion_item(&view, slot) {
                    Some(held) => holding && held.same_item_same_components(resource),
                    None => !holding,
                };
                if !pick {
                    continue;
                }
                let stack = ItemStack {
                    count: left.min(u64::from(u32::MAX)) as u32,
                    ..resource.clone()
                };
                let offered = stack.count;
                let (rest, result) =
                    self.move_in_item(Insertion::Capability, &view, stack, slot, &mut attempt);
                if result == MoveIn::Moved {
                    left -= u64::from(offered - rest.map_or(0, |stack| stack.count));
                    told = true;
                }
            }
        }
        if told {
            self.view_told(&view);
        }
        amount - left
    }
    /// Takes up to `amount` of `resource` out of the slots `at`'s side reaches and may take
    /// from, in transaction `tx`, on one view of the container: what came out.
    pub(in crate::game) fn container_extract_kind(
        &mut self,
        at: BlockSide,
        resource: &ItemStack,
        amount: u64,
        tx: &TransactionContext,
    ) -> u64 {
        if !self.transaction_context_is_current(tx) {
            return 0;
        }
        if amount == 0 {
            return 0;
        }
        let Some(view) = self.capability_view(at) else {
            return 0;
        };
        let (mut left, mut told) = (amount, false);
        let mut attempt = TakeAttempt::default();
        for slot in view.capability_slots(at.side) {
            if left == 0 {
                break;
            }
            let c = self.containers();
            let Some(held) = c.insertion_item(&view, slot) else {
                continue;
            };
            if !held.same_item_same_components(resource)
                || at.side.is_some_and(|face| {
                    let ask = TakeAsk {
                        game: self,
                        attempt: &mut attempt,
                    };
                    !c.can_take(&view, slot, &held, face, ask)
                })
            {
                continue;
            }
            self.journal_view_slot(&view, slot);
            let ask = left.min(u64::from(u32::MAX)) as u32;
            if let Some(out) = self.take_up_to(&view, slot, ask) {
                left -= u64::from(out.count);
                told = true;
            }
        }
        if told {
            self.view_told(&view);
        }
        amount - left
    }
    /// How many of `stack` slot `slot` of those `at`'s side reaches holds when empty, as
    /// NeoForge's `isValid` and `getCapacityAsLong` answer: none where it may not go in (a
    /// furnace's output, `canPlaceItem`), else the slot's limit for it. `None` asks for the
    /// general slot limit before item-specific rules.
    pub(in crate::game) fn container_capacity(
        &self,
        at: BlockSide,
        slot: usize,
        stack: Option<&ItemStack>,
    ) -> u32 {
        let Some(view) = self.capability_view(at) else {
            return 0;
        };
        view.capability_slots(at.side).nth(slot).map_or(0, |slot| {
            let containers = self.containers();
            match stack {
                Some(stack) => containers.capacity(&view, slot, stack),
                None => containers.slot_stack_limit(&view, slot, None),
            }
        })
    }
    /// Takes up to `max` out of slot `slot` of those `at`'s side reaches, in transaction `tx`;
    /// through a side, only what that side may take (`canTakeItemThroughFace`). What came out.
    pub(in crate::game) fn container_extract(
        &mut self,
        at: BlockSide,
        slot: usize,
        max: u32,
        tx: &TransactionContext,
    ) -> Option<ItemStack> {
        if !self.transaction_context_is_current(tx) {
            return None;
        }
        let view = self.capability_view(at)?;
        let slot = view.capability_slots(at.side).nth(slot)?;
        let c = self.containers();
        let stack = c.insertion_item(&view, slot)?;
        if max == 0
            || at.side.is_some_and(|face| {
                let ask = TakeAsk {
                    game: self,
                    attempt: &mut TakeAttempt::default(),
                };
                !c.can_take(&view, slot, &stack, face, ask)
            })
        {
            return None;
        }
        self.journal_view_slot(&view, slot);
        let out = self.take_up_to(&view, slot, max)?;
        // A jukebox's song stops at the root commit (`JukeboxBlockEntity.removeItem`).
        self.view_told(&view);
        Some(out)
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;
use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::super::ViewTestSlots as _;
    use super::*;
    use std::sync::Arc;
    use sunrise_core::geometry::Direction;
    use sunrise_level::containers::ModContainer;

    fn view(kind: Kind, slots: usize) -> View {
        View {
            first: ((0, 0, 0), slots),
            second: None,
            kind,
            container: (kind == Kind::Mod).then(|| {
                Arc::new(ModContainer {
                    slots,
                    ..Default::default()
                })
            }),
        }
    }

    #[test]
    fn a_face_only_narrows_the_slots_of_a_worldly_container() {
        assert_eq!(
            view(Kind::Furnace, 3).slots_to_add(Some(Direction::Up)),
            [0]
        );
        assert_eq!(
            view(Kind::Generic, 2).slots_to_add(Some(Direction::Up)),
            [0, 1]
        );
        assert_eq!(
            view(Kind::Shulker, 2).slots_to_add(Some(Direction::Up)),
            [0, 1]
        );
        assert_eq!(view(Kind::Furnace, 3).slots_to_add(None), [0, 1, 2]);
    }

    fn level() -> LevelRef {
        LevelRef::new(
            mod_api::WorldEpoch::new(1),
            mod_api::DimensionKey::OVERWORLD,
        )
    }

    /// The own slots `view`'s `face` reports, by slot, at position `(0, 0, 0)` unless it says.
    fn reported(view: &View, face: Option<Direction>) -> Vec<Option<(Pos, usize)>> {
        let at = BlockSide {
            level: level(),
            pos: [0, 0, 0],
            side: face,
        };
        let count = view.capability_slots(face).count();
        (0..=count)
            .map(|slot| {
                view.slot_backing(at, slot).map(|backing| {
                    assert_eq!(backing.level, level());
                    (
                        (backing.pos[0], backing.pos[1], backing.pos[2]),
                        backing.slot,
                    )
                })
            })
            .collect()
    }

    fn own(slots: &[usize]) -> Vec<Option<(Pos, usize)>> {
        slots
            .iter()
            .map(|&slot| Some(((0, 0, 0), slot)))
            .chain([None])
            .collect()
    }

    #[test]
    fn a_furnaces_faces_report_the_slots_getslotsforface_picks() {
        let furnace = view(Kind::Furnace, 3);
        assert_eq!(reported(&furnace, Some(Direction::Up)), own(&[0]));
        for face in [
            Direction::North,
            Direction::East,
            Direction::South,
            Direction::West,
        ] {
            assert_eq!(reported(&furnace, Some(face)), own(&[1]));
        }
        let down = reported(&furnace, Some(Direction::Down));
        assert_eq!(down, own(&[2, 1]));
        assert_eq!(down[1], reported(&furnace, Some(Direction::North))[0]);
    }

    #[test]
    fn a_brewing_stands_faces_report_the_slots_getslotsforface_picks() {
        let stand = view(Kind::Brewing, 5);
        assert_eq!(reported(&stand, Some(Direction::Up)), own(&[3]));
        assert_eq!(reported(&stand, Some(Direction::East)), own(&[0, 1, 2, 4]));
        assert_eq!(reported(&stand, Some(Direction::Down)), own(&[0, 1, 2, 3]));
    }

    #[test]
    fn a_composter_has_no_backing_on_any_face() {
        for fill in [0, 7, 8] {
            let composter = view(Kind::Composter(fill), 1);
            for face in [
                None,
                Some(Direction::Up),
                Some(Direction::Down),
                Some(Direction::North),
            ] {
                assert_eq!(
                    composter.slot_backing(
                        BlockSide {
                            level: level(),
                            pos: [0, 0, 0],
                            side: face
                        },
                        0
                    ),
                    None
                );
            }
        }
    }

    #[test]
    fn a_mod_container_reports_its_owning_block_and_declared_face_slots() {
        let mut mod_view = view(Kind::Mod, 4);
        mod_view.first = ((5, 6, 7), 4);
        let mut container = ModContainer {
            slots: 4,
            ..Default::default()
        };
        container.faces[Direction::Up as usize] = vec![3, 1];
        mod_view.container = Some(Arc::new(container));
        let at = BlockSide {
            level: level(),
            pos: [5, 6, 7],
            side: Some(Direction::Up),
        };
        let slot = |slot| {
            Some(StorageSlot {
                level: level(),
                pos: [5, 6, 7],
                slot,
            })
        };
        assert_eq!(mod_view.slot_backing(at, 0), slot(3));
        assert_eq!(mod_view.slot_backing(at, 1), slot(1));
        assert_eq!(mod_view.slot_backing(at, 2), None);
    }

    #[test]
    fn a_double_chest_reports_each_slot_with_the_half_that_owns_it() {
        let double = View {
            first: ((0, 64, 0), 27),
            second: Some(((1, 64, 0), 27)),
            kind: Kind::Generic,
            container: None,
        };
        let at = BlockSide {
            level: level(),
            pos: [0, 64, 0],
            side: None,
        };
        for slot in 0..54 {
            let half = [[0, 64, 0], [1, 64, 0]][slot / 27];
            assert_eq!(
                double.slot_backing(at, slot),
                Some(StorageSlot {
                    level: level(),
                    pos: half,
                    slot: slot % 27
                })
            );
        }
        assert_eq!(double.slot_backing(at, 54), None);
    }
}

#[cfg(test)]
#[path = "storage_slot_backing_tests.rs"]
mod storage_slot_backing_tests;
