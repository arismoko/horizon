//! `fluid_util::dispense_container` from a real dispenser: a container's `dispense_item` part
//! that returns it fills and empties a bucket-like container (its empty and full forms
//! different items), picks up before it places, sends the remainder into the dispenser or out
//! of its front, and dispenses an unchanged container where nothing moves.
use mod_api::*;
use std::sync::{Arc, Mutex};
use sunrise_core::{
    geometry::Direction, position::BlockPos, random::RandomSource, ticks::TickTypeId,
};
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::Chunk;
use sunrise_world::{fluid::FluidKind, registry::BlockStateId};
use {
    sunrise_gameplay::game::dispense_fixture,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay_common::game::Dim, sunrise_gameplay_common::game::api_values::test_snapshot,
    sunrise_gameplay_common::game::transactions::After,
};

/// The dispenser, facing east, and the block in front of it.
const DISPENSER: [i32; 3] = [5, 64, 5];
const SLOT_POS: (i32, i32, i32) = (DISPENSER[0], DISPENSER[1], DISPENSER[2]);
const FRONT: [i32; 3] = [6, 64, 5];

/// Answers the dispense of the items it is attached to with `dispense_container`.
struct Container;
#[mod_api::sunrise_part]
impl ItemPart for Container {
    fn dispense_item(
        &self,
        host: &mut dyn ServerHost,
        event: &mut DispenseItemEvent<'_>,
    ) -> Option<Dispensed> {
        fluid_util::dispense_container(host, event)
    }
}

struct DetachedReply;
#[mod_api::sunrise_part]
impl ItemPart for DetachedReply {
    fn dispense_item(
        &self,
        _: &mut dyn ServerHost,
        event: &mut DispenseItemEvent<'_>,
    ) -> Option<Dispensed> {
        let mut stack = event.stack.clone();
        stack.set_count(3).unwrap();
        stack
            .set_components(
                [(
                    "!minecraft:max_stack_size".into(),
                    Tag::Compound(Default::default()),
                )]
                .into(),
            )
            .unwrap();
        Some((Some(stack), 0))
    }
}

#[derive(Debug, PartialEq, Eq)]
struct PickupSnapshot {
    pub second_slot_count: u32,
    next_random: i32,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Replacement {
    #[default]
    None,
    DuringTransfer,
    BeforeNotice,
}

#[derive(Clone, Default)]
struct Notices {
    pub contents: Arc<Mutex<Vec<DimensionKey>>>,
    fluids: Arc<Mutex<Vec<(String, String)>>>,
    pub reopen_on_pickup: bool,
    use_stock_bucket_behavior: bool,
    pub pickup_snapshots: Option<Arc<Mutex<Vec<PickupSnapshot>>>>,
    replacement: Replacement,
}

#[mod_api::sunrise_part]
impl BlockPart for Notices {
    fn contents_changed(&self, _: &mut dyn ServerHost, event: &mut ContentsChangedEvent) {
        self.contents.lock().unwrap().push(event.level.dimension());
    }
}

struct SchedulingLiquidContainer;

#[mod_api::sunrise_part]
impl BlockPart for SchedulingLiquidContainer {
    fn can_place_liquid(&self, _: LevelSide<'_>, _: &mut CanPlaceLiquidEvent) -> Option<bool> {
        Some(true)
    }

    fn place_liquid(
        &self,
        level: &mut dyn LevelWriteHost,
        event: &mut PlaceLiquidEvent,
    ) -> Option<bool> {
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        let filled = level.block_state_id("minecraft:emerald_block").unwrap();
        Some(
            level.set_block(pos, filled, BlockUpdateFlags::CLIENTS)
                && level.schedule_fluid_tick(pos, "minecraft:water", 5),
        )
    }
}

/// Vanilla's buckets, through the game's own `ITEM_FLUIDS`, and paper made a 2000 mB tank in
/// its `custom_data`: a container that can both take more and give.
struct Containers(Notices);
#[mod_api::__subscriptions]
impl Hooks for Containers {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let dispenser = r.block_key("minecraft:dispenser");
        r.block_part(dispenser, self.0.clone());
        let chest = r.block_key("minecraft:chest");
        r.block_part(chest, self.0.clone());
        let dandelion = r.block_key("minecraft:dandelion");
        r.block_part(dandelion, SchedulingLiquidContainer);
        let stick = r.item_key("minecraft:stick");
        r.item_part(stick, DetachedReply);
        for item in [
            "minecraft:bucket",
            "minecraft:water_bucket",
            "minecraft:paper",
            "minecraft:glass_bottle",
            "minecraft:clay_ball",
        ] {
            if item != "minecraft:bucket" || !self.0.use_stock_bucket_behavior {
                let key = r.item_key(item);
                r.item_part(key, Container);
            }
        }
        r.item_capability(
            &ITEM_FLUIDS,
            "minecraft:paper",
            |_: &mut dyn ServerHost, place: &ItemPlace| {
                let data = ComponentId::new("minecraft:custom_data").unwrap();
                Some(
                    Box::new(ItemComponentFluidTank::new(place.clone(), data, 2000))
                        as Box<dyn ResourceHandler<FluidResource>>,
                )
            },
        );
        let replace_during_transfer = self.0.replacement == Replacement::DuringTransfer;
        r.item_capability(
            &ITEM_FLUIDS,
            "minecraft:glass_bottle",
            move |_: &mut dyn ServerHost, place: &ItemPlace| {
                Some(Box::new(Jug {
                    place: place.clone(),
                    replace_during_transfer,
                })
                    as Box<dyn ResourceHandler<FluidResource>>)
            },
        );
        r.item_capability(
            &ITEM_FLUIDS,
            "minecraft:clay_ball",
            |_: &mut dyn ServerHost, place: &ItemPlace| {
                Some(Box::new(MultiStepJug(place.clone()))
                    as Box<dyn ResourceHandler<FluidResource>>)
            },
        );
    }

    fn vanilla_game_event(
        &self,
        host: &mut dyn ServerHost,
        event: &mut VanillaGameEventEvent<'_>,
    ) -> Flow {
        if matches!(
            event.event,
            "minecraft:fluid_pickup" | "minecraft:fluid_place"
        ) {
            if event.event == "minecraft:fluid_pickup" {
                if self.0.reopen_on_pickup {
                    host.transaction(|_, _| Ok::<_, ()>(())).unwrap();
                }
                if self.0.replacement == Replacement::BeforeNotice {
                    replace_dispenser_with_chest(host, event.dimension);
                }
                if let Some(snapshots) = &self.0.pickup_snapshots {
                    let level = host.level(event.dimension).unwrap();
                    let items = level.capability(host, &ITEMS, DISPENSER, None).unwrap();
                    let second_slot_count = items.amount(host, 1);
                    let next_random = level.with_random(host, |random| random.next_int()).unwrap();
                    snapshots.lock().unwrap().push(PickupSnapshot {
                        second_slot_count,
                        next_random,
                    });
                }
            }
            self.0
                .fluids
                .lock()
                .unwrap()
                .push((event.event.to_owned(), event.dimension.to_owned()));
        }
        Flow::Continue
    }
}

/// An empty glass bottle a bucket of water turns into a potion: a mod's handler that acts on
/// every container its place shows, as one written for a single container may.
struct Jug {
    pub place: ItemPlace,
    replace_during_transfer: bool,
}
impl ResourceHandler<FluidResource> for Jug {
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        1
    }
    fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<FluidResource> {
        None
    }
    fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
        0
    }
    fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: Option<&FluidResource>) -> u32 {
        1000
    }
    fn insert_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        _: usize,
        _: &FluidResource,
        amount: u32,
    ) -> Result<u32, mod_api::TransactionError<mod_api::Overanswered>> {
        if amount < 1000 {
            return Ok(0);
        }
        let potion = mod_api::__private::item_key("minecraft:potion").unwrap();
        let potion = ItemResource::of(&mod_api::ItemStack::new(potion, 1).unwrap());
        let all = self.place.amount(host);
        if self.place.exchange(host, tx, &potion, all)? != all {
            return Ok(0);
        }
        if self.replace_during_transfer {
            replace_dispenser_with_chest(host, "minecraft:overworld");
        }
        Ok(1000)
    }
    fn extract_at(
        &self,
        _: &mut dyn ServerHost,
        _: &TransactionContext,
        _: usize,
        _: &FluidResource,
        _: u32,
    ) -> Result<u32, mod_api::TransactionError<mod_api::Overanswered>> {
        Ok(0)
    }
}

/// A custom container handler that extracts twice and attempts a multi-item insert through the
/// `OneByOne` wrapper, exercising: (1) `OneByOne` capping inserts at 1, (2) `dispense_container`
/// returning the actual final first slot when overflow is present (`count - 2`, not `count - 1`),
/// and (3) `(None, Some(changed))` when the second extract empties slot 0 after slot 1 holds the
/// inserted item.
struct MultiStepJug(ItemPlace);
impl ResourceHandler<FluidResource> for MultiStepJug {
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        1
    }
    fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<FluidResource> {
        None
    }
    fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
        0
    }
    fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: Option<&FluidResource>) -> u32 {
        1000
    }
    fn insert_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        _: usize,
        _: &FluidResource,
        amount: u32,
    ) -> Result<u32, mod_api::TransactionError<mod_api::Overanswered>> {
        if amount < 1000 {
            return Ok(0);
        }
        let Some(current) = self.0.resource(host) else {
            return Ok(0);
        };
        let potion = mod_api::__private::item_key("minecraft:potion").unwrap();
        let potion = ItemResource::of(&mod_api::ItemStack::new(potion, 1).unwrap());
        if self.0.extract(host, tx, &current, 1)? != 1 {
            return Ok(0);
        }
        // Through `OneByOne`, asking to insert 3 caps at 1 (into overflow while slot 0 still holds clay).
        if self.0.insert(host, tx, &potion, 3)? != 1 {
            return Ok(0);
        }
        // Extract a second unit from slot 0 (emptying it when initial count was 2, leaving 2 when 4).
        if self.0.extract(host, tx, &current, 1)? != 1 {
            return Ok(0);
        }
        Ok(1000)
    }
    fn extract_at(
        &self,
        _: &mut dyn ServerHost,
        _: &TransactionContext,
        _: usize,
        _: &FluidResource,
        _: u32,
    ) -> Result<u32, mod_api::TransactionError<mod_api::Overanswered>> {
        Ok(0)
    }
}

fn replace_dispenser_with_chest(host: &mut dyn ServerHost, dimension: &str) {
    let level = host.level(dimension).unwrap();
    let chest = host.block_state_id("minecraft:chest").unwrap();
    assert_eq!(
        level.set_block(host, DISPENSER, chest, BlockUpdateFlags::NONE),
        Ok(true)
    );
}

fn spawn_dispenser_case(test: &str) -> bool {
    run_fresh_cases(
        &format!("cut_layout::game::dispense_container_tests::{test}"),
        "SUNRISE_DISPENSE_CONTAINER_CASE",
        &["dispenser"],
    )
}

/// A dedicated server with a chunk at the origin, a stone floor, an empty dispenser facing east
/// and the containers' parts installed.
fn with_dispenser(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, LevelRef)) {
    with_dispenser_notices(Notices::default(), |game, level, _| test(game, level));
}

fn with_dispenser_notices(
    notices: Notices,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, LevelRef, &Notices),
) {
    let mods: Mods = vec![ModEntry::new(
        "containers",
        Box::new(Containers(notices.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.native_lifecycle
                .begin_open(Some(mod_api::Authority::IntegratedServer));
            let stone = state(game, "minecraft:stone");
            for x in 0..16 {
                for z in 0..16 {
                    game.world.set(x, 63, z, stone).unwrap();
                }
            }
            let level = game.level("minecraft:overworld").unwrap();
            place_dispenser(game, level);
            test(game, level, &notices);
        });
    });
}

fn place_dispenser(game: &mut sunrise_gameplay::game::ServerGame<'_>, level: LevelRef) {
    let base = state(game, "minecraft:dispenser");
    let dispenser = game.with_props(base, &[("facing", "east")]).unwrap();
    assert_eq!(
        game.level_set_block(level, DISPENSER, dispenser, BlockUpdateFlags::NONE),
        Ok(true)
    );
}

fn state(game: &sunrise_gameplay::game::ServerGame<'_>, name: &str) -> BlockStateId {
    game.pack.assets.registry.default_state(name).unwrap()
}

fn front(game: &sunrise_gameplay::game::ServerGame<'_>) -> BlockStateId {
    game.world.block(FRONT[0], FRONT[1], FRONT[2])
}

fn set_front(game: &mut sunrise_gameplay::game::ServerGame<'_>, name: &str) {
    let block = state(game, name);
    game.world.set(FRONT[0], FRONT[1], FRONT[2], block).unwrap();
}

/// The dispenser dispenses `stack` from its slot: what the slot holds then.
fn dispense(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    stack: ItemStack,
) -> Option<ItemStack> {
    let at = (DISPENSER[0], DISPENSER[1], DISPENSER[2]);
    game.dispense_item(at, Direction::East, stack)
        .unwrap()
        .continue_value()
        .unwrap()
}

/// Exercise the fluid helper through its ItemPart without the caller's click and smoke.
fn dispense_part(game: &mut sunrise_gameplay::game::ServerGame<'_>, input: &str) -> Dispensed {
    let mut answer = None;
    game.native_dispense_item(
        game.dim.id(),
        DISPENSER,
        Direction::East,
        &ItemStack::new(input, 1),
        &mut answer,
    );
    answer.unwrap()
}

fn item(stack: &Option<ItemStack>) -> Option<(String, u32)> {
    stack
        .as_ref()
        .map(|stack| (stack.item.to_string(), stack.count))
}

#[test]
#[ignore = "daily"]
fn dispenser_writes_follow_the_parked_level_handle() {
    if run_fresh_cases(
        "cut_layout::game::dispense_container_tests::dispenser_writes_follow_the_parked_level_handle",
        "SUNRISE_DISPENSE_ROUTE_CHILD",
        &["parked"],
    ) {
        return;
    }
    with_dispenser(|game, level| {
        let key = game
            .item_key(&Identifier::parse("minecraft:bucket").unwrap())
            .unwrap();
        let filled = game
            .item_key(&Identifier::parse("minecraft:water_bucket").unwrap())
            .unwrap();
        let stack = mod_api::ItemStack::new(key, 2).unwrap();
        let remainder = mod_api::ItemStack::of(filled);
        let chest_at = [8, 64, 8];
        let chest = state(game, "minecraft:chest");
        assert_eq!(
            game.level_set_block(level, chest_at, chest, BlockUpdateFlags::NONE),
            Ok(true)
        );
        game.enter_command_level(Dim::Nether).unwrap();
        assert_eq!(
            level.dispense_default(game, [32, 64, 32], Direction::East, stack.clone()),
            Err(LevelWriteError::InvalidDispenser)
        );
        assert_eq!(
            level.dispense_with_remainder(
                game,
                [32, 64, 32],
                Direction::East,
                stack.clone(),
                remainder.clone()
            ),
            Err(LevelWriteError::InvalidDispenser)
        );
        assert_eq!(
            level.dispense_with_remainder(
                game,
                chest_at,
                Direction::East,
                stack.clone(),
                remainder.clone()
            ),
            Err(LevelWriteError::InvalidDispenser)
        );
        assert_eq!(
            level.dispense_default(game, chest_at, Direction::East, stack.clone()),
            Err(LevelWriteError::InvalidDispenser)
        );
        let left = level
            .dispense_default(game, DISPENSER, Direction::East, stack.clone())
            .unwrap()
            .unwrap();
        assert_eq!((left.item(), left.count()), (key, 1));
        let left = level
            .dispense_with_remainder(game, DISPENSER, Direction::East, stack, remainder)
            .unwrap()
            .unwrap();
        assert_eq!((left.item(), left.count()), (key, 1));
        assert_eq!(game.dim, Dim::Nether);
        assert!(game.dropped.is_empty());

        game.enter_command_level(Dim::Overworld).unwrap();
        assert_eq!(game.dropped.len(), 1);
        assert_eq!(&*game.dropped[0].stack.item, "minecraft:bucket");
        let dispenser = game
            .world
            .block_entities
            .get(&(DISPENSER[0], DISPENSER[1], DISPENSER[2]))
            .unwrap();
        assert_eq!(
            item(&dispenser.slots()[0]),
            Some(("minecraft:water_bucket".into(), 1))
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_bucket_like_container_fills_from_a_source_and_empties_into_the_air() {
    if spawn_dispenser_case("a_bucket_like_container_fills_from_a_source_and_empties_into_the_air")
    {
        return;
    }
    with_dispenser(|game, _| {
        set_front(game, "minecraft:water");
        let filled = dispense(game, ItemStack::new("minecraft:bucket", 1));
        assert_eq!(item(&filled), Some(("minecraft:water_bucket".into(), 1)));
        assert_eq!(front(game), state(game, "minecraft:air"));
        let emptied = dispense(game, filled.unwrap());
        assert_eq!(item(&emptied), Some(("minecraft:bucket".into(), 1)));
        assert_eq!(front(game), state(game, "minecraft:water"));
        assert!(game.dropped.is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn a_stack_of_three_in_a_full_dispenser_drops_the_new_container_out_of_the_front() {
    if spawn_dispenser_case(
        "a_stack_of_three_in_a_full_dispenser_drops_the_new_container_out_of_the_front",
    ) {
        return;
    }
    with_dispenser(|game, _| {
        let pos = (DISPENSER[0], DISPENSER[1], DISPENSER[2]);
        for cell in game.world.block_entities.get_mut(&pos).unwrap().slots_mut() {
            *cell = Some(ItemStack::new("minecraft:stone", 64));
        }
        set_front(game, "minecraft:water");
        dispense_fixture::start_recording();
        let left = dispense(game, ItemStack::new("minecraft:bucket", 3));
        let lines = dispense_fixture::stop_recording();
        assert_eq!(item(&left), Some(("minecraft:bucket".into(), 2)));
        assert_eq!(front(game), state(game, "minecraft:air"));
        assert_eq!(game.dropped.len(), 1);
        assert_eq!(&*game.dropped[0].stack.item, "minecraft:water_bucket");
        // The fluid's event, then the spill's click and smoke, then the dispense's own.
        let clicks = lines
            .iter()
            .filter(|line| line.starts_with("L 1000 "))
            .count();
        assert_eq!(clicks, 2, "{lines:?}");
        assert!(lines[0].ends_with("minecraft:fluid_pickup"), "{lines:?}");
    });
}

#[test]
#[ignore = "daily"]
fn a_failed_transfer_dispenses_one_unchanged_container_and_changes_nothing() {
    if spawn_dispenser_case(
        "a_failed_transfer_dispenses_one_unchanged_container_and_changes_nothing",
    ) {
        return;
    }
    with_dispenser(|game, _| {
        set_front(game, "minecraft:stone");
        dispense_fixture::start_recording();
        let left = dispense(game, ItemStack::new("minecraft:bucket", 2));
        let lines = dispense_fixture::stop_recording();
        assert_eq!(item(&left), Some(("minecraft:bucket".into(), 1)));
        assert_eq!(game.dropped.len(), 1);
        assert_eq!(&*game.dropped[0].stack.item, "minecraft:bucket");
        assert_eq!(front(game), state(game, "minecraft:stone"));
        assert!(
            !lines.iter().any(|line| line.starts_with("G ")),
            "{lines:?}"
        );
    });
}

#[test]
#[ignore = "daily"]
fn oversized_dispenser_stacks_keep_their_count_until_the_slot_write() {
    if spawn_dispenser_case("oversized_dispenser_stacks_keep_their_count_until_the_slot_write") {
        return;
    }
    with_dispenser(|game, _| {
        let stack = ItemStack::new("minecraft:bucket", 20);
        game.world
            .block_entities
            .get_mut(&SLOT_POS)
            .unwrap()
            .slots_mut()[0] = Some(stack.clone());
        set_front(game, "minecraft:water");
        let left = dispense(game, stack);
        assert_eq!(left, Some(ItemStack::new("minecraft:bucket", 19)));
        assert_eq!(front(game), state(game, "minecraft:air"));
        let slots = game.world.block_entities.get(&SLOT_POS).unwrap().slots();
        assert_eq!(slots[1], Some(ItemStack::new("minecraft:water_bucket", 1)));
        assert!(game.dropped.is_empty(), "no second behaviour may run");

        // Now let dispenseFrom choose and install the active slot. Only that write
        // applies the empty bucket's resident maximum of 16.
        game.world
            .block_entities
            .get_mut(&SLOT_POS)
            .unwrap()
            .slots_mut()[1] = None;
        set_front(game, "minecraft:water");
        let [x, y, z] = DISPENSER;
        game.dispense_from(BlockPos::new(x, y, z), game.world.block(x, y, z))
            .unwrap();
        let slots = game.world.block_entities.get(&SLOT_POS).unwrap().slots();
        assert_eq!(slots[0], Some(ItemStack::new("minecraft:bucket", 16)));
        assert_eq!(slots[1], Some(ItemStack::new("minecraft:water_bucket", 1)));
    });
}

#[test]
#[ignore = "daily"]
fn removed_max_stack_size_limits_dispenser_placement_and_scratch_insertion() {
    if spawn_dispenser_case(
        "removed_max_stack_size_limits_dispenser_placement_and_scratch_insertion",
    ) {
        return;
    }
    with_dispenser(|game, _| {
        game.world
            .block_entities
            .get_mut(&SLOT_POS)
            .unwrap()
            .slots_mut()[0] = Some(ItemStack::new("minecraft:stick", 1));
        let [x, y, z] = DISPENSER;
        game.dispense_from(BlockPos::new(x, y, z), game.world.block(x, y, z))
            .unwrap();
        let slot = &game.world.block_entities.get(&SLOT_POS).unwrap().slots()[0];
        let placed = slot.as_ref().unwrap();
        assert_eq!((&*placed.item, placed.count), ("minecraft:stick", 1));
        assert!(
            placed
                .data()
                .unwrap()
                .other("!minecraft:max_stack_size")
                .is_some()
        );
        assert!(game.dropped.is_empty());

        let stack = test_snapshot(placed);
        let resource = ItemResource::of(&stack);
        let host: &mut dyn ServerHost = game;
        let (inserted, final_stack) = host
            .scratch_stack(stack, |host, tx, slot| {
                assert_eq!(slot.extract(host, tx, &resource, 1), Ok(1));
                slot.insert(host, tx, &resource, 3)
            })
            .unwrap();
        assert_eq!(inserted, 1);
        assert_eq!(final_stack, resource.to_stack(1));
    });
}

#[test]
#[ignore = "daily"]
fn oversized_failed_transfer_runs_the_helpers_default_dispense() {
    if spawn_dispenser_case("oversized_failed_transfer_runs_the_helpers_default_dispense") {
        return;
    }
    with_dispenser(|game, _| {
        set_front(game, "minecraft:stone");
        dispense_fixture::start_recording();
        let left = dispense(game, ItemStack::new("minecraft:glass_bottle", 80));
        let lines = dispense_fixture::stop_recording();
        assert_eq!(left, Some(ItemStack::new("minecraft:glass_bottle", 79)));
        assert_eq!(front(game), state(game, "minecraft:stone"));
        assert_eq!(game.dropped.len(), 1);
        assert_eq!(
            game.dropped[0].stack,
            ItemStack::new("minecraft:glass_bottle", 1)
        );
        // The helper uses default execute (click); the vanilla glass bottle behaviour
        // also drops one here but would emit the failed-dispense sound instead.
        assert_eq!(lines, ["L 1000 5,64,5 0", "L 2000 5,64,5 5"]);
    });
}

#[test]
#[ignore = "daily"]
fn dispenser_remainder_merges_without_contents_changed() {
    if spawn_dispenser_case("dispenser_remainder_merges_without_contents_changed") {
        return;
    }
    with_dispenser_notices(Notices::default(), |game, _, notices| {
        let notices = &notices.contents;
        game.world
            .block_entities
            .get_mut(&SLOT_POS)
            .unwrap()
            .slots_mut()[0] = Some(ItemStack::new("minecraft:stick", 10));
        let merge = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
            let mut offered = ItemStack::new("minecraft:stick", 2);
            offered
                .data_mut()
                .set_other("minecraft:max_stack_size", Some(Tag::Int(64)));
            game.add_to_inventory_or_dispense(SLOT_POS, Direction::East, offered);
            assert_eq!(
                game.world.block_entities.get(&SLOT_POS).unwrap().slots()[0],
                Some(ItemStack::new("minecraft:stick", 12))
            );
        };
        assert_eq!(
            game.storage_transaction(|game, _| {
                merge(game);
                Err::<(), _>(())
            }),
            Err(())
        );
        assert_eq!(
            game.world.block_entities.get(&SLOT_POS).unwrap().slots()[0],
            Some(ItemStack::new("minecraft:stick", 10))
        );
        game.storage_transaction(|game, _| {
            merge(game);
            Ok::<_, ()>(())
        })
        .unwrap();
        assert!(game.dropped.is_empty());
        assert!(
            notices.lock().unwrap().is_empty(),
            "growing a stack is not setItem"
        );
    });
}

#[test]
#[ignore = "daily"]
fn nested_dispense_settles_in_the_command_source_level() {
    if spawn_dispenser_case("nested_dispense_settles_in_the_command_source_level") {
        return;
    }
    with_dispenser_notices(Notices::default(), |game, _, notices| {
        let notices = &notices.contents;
        let marker = ItemStack::new("minecraft:stick", 10);
        game.world
            .block_entities
            .get_mut(&SLOT_POS)
            .unwrap()
            .slots_mut()[0] = Some(marker.clone());
        let mut source = game.server_source();
        source.dimension = Dim::Nether;
        game.with_source(source.clone(), |game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level(Dim::Nether.id()).unwrap();
            place_dispenser(game, level);
            let slots = game
                .world
                .block_entities
                .get_mut(&SLOT_POS)
                .unwrap()
                .slots_mut();
            for slot in &mut slots[1..] {
                *slot = Some(ItemStack::new("minecraft:stone", 64));
            }
        });

        let dispense = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
            game.storage_transaction(|game, _| {
                let nether = game.level(Dim::Nether.id()).unwrap();
                let stick = ItemStack::new("minecraft:stick", 1);
                let stack = test_snapshot(&ItemStack::new("minecraft:bucket", 2));
                for rest in [&stick, &ItemStack::new("minecraft:dirt", 1)] {
                    let left = nether
                        .dispense_with_remainder(
                            game,
                            DISPENSER,
                            Direction::East,
                            stack.clone(),
                            test_snapshot(rest),
                        )
                        .unwrap();
                    assert_eq!(left.unwrap().count(), 1);
                }
                let left = nether
                    .dispense_default(game, DISPENSER, Direction::East, stack)
                    .unwrap();
                assert_eq!(left.unwrap().count(), 1);
                assert_eq!(
                    game.world.block_entities.get(&SLOT_POS).unwrap().slots()[0],
                    Some(stick)
                );
                assert!(game.dropped.is_empty());
                Ok::<_, ()>(())
            })
            .unwrap();
        };

        // A child remains tentative while a transaction in the dispenser's own level is open.
        dispense_fixture::start_recording();
        game.with_source(source.clone(), |game| {
            let outcome = game.storage_transaction(|game, _| {
                dispense(game);
                assert!(notices.lock().unwrap().is_empty());
                assert!(game.dropped.is_empty());
                Err::<(), _>(())
            });
            assert_eq!(outcome, Err(()));
            assert_eq!(
                game.world.block_entities.get(&SLOT_POS).unwrap().slots()[0],
                None
            );
            assert!(game.dropped.is_empty());
        });
        assert!(dispense_fixture::stop_recording().is_empty());

        // No open source-level transaction: its child settles even though the command's
        // outer level is still tentative. A same-position write there rolls back on its own.
        dispense_fixture::start_recording();
        let outcome = game.storage_transaction(|game, _| {
            game.journal_block_entity_slot(SLOT_POS, 0);
            game.world
                .block_entities
                .get_mut(&SLOT_POS)
                .unwrap()
                .slots_mut()[0] = None;
            game.after_commit(After::Changed(SLOT_POS));
            game.with_source(source.clone(), dispense);
            assert_eq!(*notices.lock().unwrap(), [DimensionKey::NETHER]);
            Err::<(), _>(())
        });
        assert_eq!(outcome, Err(()));
        let lines = dispense_fixture::stop_recording();
        assert_eq!(lines, ["L 1000 5,64,5 0", "L 2000 5,64,5 5"]);
        assert_eq!(*notices.lock().unwrap(), [DimensionKey::NETHER]);
        assert_eq!(game.dim, Dim::Overworld);
        assert!(game.dropped.is_empty());
        assert_eq!(
            game.world.block_entities.get(&SLOT_POS).unwrap().slots()[0],
            Some(marker)
        );
        game.with_source(source, |game| {
            assert_eq!(
                game.world.block_entities.get(&SLOT_POS).unwrap().slots()[0],
                Some(ItemStack::new("minecraft:stick", 1))
            );
            let drops: Vec<_> = game.dropped.iter().map(|item| item.stack.clone()).collect();
            assert_eq!(
                drops,
                [
                    ItemStack::new("minecraft:dirt", 1),
                    ItemStack::new("minecraft:bucket", 1)
                ]
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn nested_dispense_container_publishes_fluid_changes_only_in_the_source_level() {
    if spawn_dispenser_case(
        "nested_dispense_container_publishes_fluid_changes_only_in_the_source_level",
    ) {
        return;
    }
    with_dispenser_notices(Notices::default(), |game, _, notices| {
        let overworld = game.server_source();
        let mut nether = overworld.clone();
        nether.dimension = Dim::Nether;
        let [x, y, z] = FRONT;
        let pos = BlockPos::new(x, y, z);
        let water_tick = |game: &sunrise_gameplay::game::ServerGame<'_>| {
            game.block_ticks
                .fluids
                .has_scheduled(pos, TickTypeId(FluidKind::Water.tick_id()))
        };
        game.with_source(nether.clone(), |game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks.add_empty_chunk(pos.chunk()).unwrap();
            set_front(game, "minecraft:netherrack");
            game.world.block_log = Some(Vec::new());
        });
        let slab = state(game, "minecraft:stone_slab");
        let waterlogged = game.with_props(slab, &[("waterlogged", "true")]).unwrap();
        for (input, before, after, output, drop, event, schedules_tick) in [
            (
                "minecraft:bucket",
                state(game, "minecraft:water"),
                state(game, "minecraft:air"),
                "minecraft:water_bucket",
                None,
                "minecraft:fluid_pickup",
                false,
            ),
            // A lava cauldron is not a BucketPickup; the helper drains it through FLUIDS.
            (
                "minecraft:bucket",
                state(game, "minecraft:lava_cauldron"),
                state(game, "minecraft:cauldron"),
                "minecraft:lava_bucket",
                None,
                "minecraft:fluid_pickup",
                false,
            ),
            // The Part, not the vanilla waterlogging path, stages this water tick.
            (
                "minecraft:water_bucket",
                state(game, "minecraft:dandelion"),
                state(game, "minecraft:emerald_block"),
                "minecraft:bucket",
                None,
                "minecraft:fluid_place",
                true,
            ),
            (
                "minecraft:water_bucket",
                slab,
                waterlogged,
                "minecraft:bucket",
                None,
                "minecraft:fluid_place",
                true,
            ),
            (
                "minecraft:water_bucket",
                state(game, "minecraft:poppy"),
                state(game, "minecraft:water"),
                "minecraft:bucket",
                Some("minecraft:poppy"),
                "minecraft:fluid_place",
                false,
            ),
        ] {
            game.world.set(x, y, z, before).unwrap();
            game.world.block_log = Some(Vec::new());
            game.block_ticks.add_empty_chunk(pos.chunk()).unwrap();
            game.with_source(nether.clone(), |game| {
                for commit in [false, true] {
                    dispense_fixture::start_recording();
                    let outcome = game.storage_transaction(|game, _| {
                        game.with_source(overworld.clone(), |game| {
                            let source = game.storage_transaction(|game, _| {
                                let (left, _) = dispense_part(game, input);
                                let expected = test_snapshot(&ItemStack::new(output, 1));
                                assert_eq!(left, Some(expected));
                                assert_eq!(front(game), after, "{input} on {before:?}");
                                assert!(game.world.block_log.as_ref().unwrap().is_empty());
                                assert!(!water_tick(game));
                                assert!(game.dropped.is_empty());
                                assert!(notices.fluids.lock().unwrap().is_empty());
                                commit.then_some(()).ok_or(())
                            });
                            assert_eq!(source, commit.then_some(()).ok_or(()));
                        });
                        assert_eq!(game.dim, Dim::Nether);
                        assert_eq!(notices.fluids.lock().unwrap().len(), usize::from(commit));
                        Err::<(), _>(())
                    });
                    assert_eq!(outcome, Err(()));
                    let lines = dispense_fixture::stop_recording();
                    let heard = std::mem::take(&mut *notices.fluids.lock().unwrap());
                    if commit {
                        assert_eq!(heard, [(event.to_owned(), Dim::Overworld.id().to_owned())]);
                    } else {
                        assert!(heard.is_empty(), "{heard:?}");
                        assert!(lines.is_empty(), "{lines:?}");
                    }
                    assert_eq!(game.dim, Dim::Nether);
                    assert_eq!(front(game), state(game, "minecraft:netherrack"));
                    assert!(game.world.block_log.as_ref().unwrap().is_empty());
                    assert!(!water_tick(game));
                    assert!(game.dropped.is_empty());
                    game.with_source(overworld.clone(), |game| {
                        assert_eq!(front(game), if commit { after } else { before });
                        let log = game.world.block_log.as_ref().unwrap();
                        assert_eq!(
                            log.iter().filter(|p| **p == (x, y, z)).count(),
                            usize::from(commit)
                        );
                        if schedules_tick || !commit {
                            assert_eq!(water_tick(game), commit, "{input} on {before:?}");
                        }
                        let drops: Vec<_> =
                            game.dropped.iter().map(|item| &*item.stack.item).collect();
                        assert_eq!(drops, if commit { drop.as_slice() } else { &[] });
                    });
                }
            });
            let _ = game.block_ticks.remove_chunk(pos.chunk());
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_transaction_in_the_fluid_pickup_listener_does_not_delay_later_attachment_removal() {
    if spawn_dispenser_case(
        "a_transaction_in_the_fluid_pickup_listener_does_not_delay_later_attachment_removal",
    ) {
        return;
    }
    use sunrise_mod_runtime::attachments::{AttachmentType, Holder};

    static VALUE: AttachmentType<i32> = AttachmentType {
        namespace: "containers",
        name: "retirement_witness",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync_codec: None,
    };
    let notices = Notices {
        reopen_on_pickup: true,
        ..Notices::default()
    };
    with_dispenser_notices(notices, |game, level, notices| {
        game.native_level_opened(Dim::Overworld);
        let index = game.attachment_indexes[&Dim::Overworld].clone();
        VALUE
            .set(
                game.world.block_entities.attachments_mut(SLOT_POS).unwrap(),
                Holder::BlockEntity(DISPENSER),
                7,
            )
            .unwrap();
        set_front(game, "minecraft:water");
        let overworld = game.server_source();
        let mut nether = overworld.clone();
        nether.dimension = Dim::Nether;
        game.with_source(nether, |game| {
            game.storage_transaction(|game, _| {
                game.with_source(overworld.clone(), |game| {
                    let (left, _) = dispense_part(game, "minecraft:bucket");
                    assert_eq!(
                        left,
                        Some(test_snapshot(&ItemStack::new("minecraft:water_bucket", 1)))
                    );
                });
                Ok::<_, ()>(())
            })
            .unwrap();
            assert_eq!(game.dim, Dim::Nether);
        });
        assert_eq!(
            *notices.fluids.lock().unwrap(),
            [(
                "minecraft:fluid_pickup".to_owned(),
                Dim::Overworld.id().to_owned()
            )]
        );
        let rows = || {
            index
                .lock()
                .unwrap()
                .snapshot_named(VALUE.namespace, VALUE.name)
        };
        assert_eq!(rows()[0].entry.read(&VALUE), Ok(&7));
        // The pickup reentered an Overworld transaction during that level's own root close.
        // A later ordinary replacement retires this holder without another transaction.
        assert!(!game.has_open_transaction());
        let air = state(game, "minecraft:air");
        assert_eq!(
            game.level_set_block(level, DISPENSER, air, BlockUpdateFlags::NONE),
            Ok(true)
        );
        assert!(game.world.block_entities.get(&SLOT_POS).is_none());
        assert!(rows().is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn dispenser_replacement_during_transfer_does_not_receive_the_remainder() {
    if spawn_dispenser_case("dispenser_replacement_during_transfer_does_not_receive_the_remainder")
    {
        return;
    }
    run_dispenser_replacement(Replacement::DuringTransfer, |game, level| {
        assert!(level.block_tick(game, DISPENSER));
        assert!(
            game.dropped
                .iter()
                .any(|item| item.stack.item.as_ref() == "minecraft:potion")
        );
    });
}

#[test]
#[ignore = "daily"]
fn dispenser_replacement_before_notice_replay_does_not_receive_the_notice() {
    if spawn_dispenser_case(
        "dispenser_replacement_before_notice_replay_does_not_receive_the_notice",
    ) {
        return;
    }
    run_dispenser_replacement(Replacement::BeforeNotice, |game, level| {
        game.storage_transaction(|game, _| {
            assert!(level.block_tick(game, DISPENSER));
            assert_eq!(
                game.world.block_entities.get(&SLOT_POS).unwrap().slots()[1],
                Some(ItemStack::new("minecraft:potion", 1))
            );
            Ok::<_, ()>(())
        })
        .unwrap();
    });
}

fn run_dispenser_replacement(
    replacement: Replacement,
    tick: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, LevelRef),
) {
    let notices = Notices {
        replacement,
        ..Notices::default()
    };
    with_dispenser_notices(notices, |game, level, notices| {
        game.world
            .block_entities
            .get_mut(&SLOT_POS)
            .unwrap()
            .slots_mut()[0] = Some(ItemStack::new("minecraft:glass_bottle", 2));
        let original = game.world.block_entities.identity(SLOT_POS).unwrap();
        set_front(game, "minecraft:water");
        tick(game, level);
        assert_eq!(front(game), state(game, "minecraft:air"));
        let [x, y, z] = DISPENSER;
        assert_eq!(game.world.block(x, y, z), state(game, "minecraft:chest"));
        let holder = game.world.block_entities.identity(SLOT_POS).unwrap();
        assert_ne!(holder, original);
        let contents = game.world.block_entities.get(&SLOT_POS).unwrap().slots();
        assert!(
            contents.iter().all(Option::is_none),
            "replacement contents: {contents:?}"
        );
        assert!(
            notices.contents.lock().unwrap().is_empty(),
            "replacement was notified"
        );
    });
}

#[test]
#[ignore = "daily"]
fn fluid_pickup_publishes_before_remainder_insertion_or_overflow_draws() {
    if spawn_dispenser_case("fluid_pickup_publishes_before_remainder_insertion_or_overflow_draws") {
        return;
    }
    let snapshots = Arc::new(Mutex::new(Vec::new()));
    let notices = Notices {
        pickup_snapshots: Some(snapshots.clone()),
        use_stock_bucket_behavior: true,
        ..Notices::default()
    };
    with_dispenser_notices(notices, |game, _, _| {
        for full in [false, true] {
            let slots = game
                .world
                .block_entities
                .get_mut(&SLOT_POS)
                .unwrap()
                .slots_mut();
            slots[0] = Some(ItemStack::new("minecraft:bucket", 2));
            slots[1..].fill(full.then(|| ItemStack::new("minecraft:stone", 64)));
            set_front(game, "minecraft:water");
            // Call the behaviour directly so the dispenser's slot-selection draw is not involved.
            let next_random = game.world.random.clone().next_int();
            let left = dispense(game, ItemStack::new("minecraft:bucket", 2));
            assert_eq!(left, Some(ItemStack::new("minecraft:bucket", 1)));
            assert_eq!(
                std::mem::take(&mut *snapshots.lock().unwrap()),
                [PickupSnapshot {
                    second_slot_count: if full { 64 } else { 0 },
                    next_random,
                }]
            );
            if full {
                assert_eq!(game.dropped.len(), 1);
                assert_eq!(
                    game.dropped[0].stack,
                    ItemStack::new("minecraft:water_bucket", 1)
                );
            } else {
                let slots = game.world.block_entities.get(&SLOT_POS).unwrap().slots();
                assert_eq!(slots[1], Some(ItemStack::new("minecraft:water_bucket", 1)));
                assert!(game.dropped.is_empty());
            }
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_container_that_could_give_picks_up_first_and_its_event_comes_before_the_click() {
    if spawn_dispenser_case(
        "a_container_that_could_give_picks_up_first_and_its_event_comes_before_the_click",
    ) {
        return;
    }
    with_dispenser(|game, _| {
        sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan(game);
        set_front(game, "minecraft:water");
        let half = dispense(game, ItemStack::new("minecraft:paper", 1));
        assert_eq!(front(game), state(game, "minecraft:air"));
        // Holding 1000 of 2000, it could empty into the air or take more: with a source in
        // front it takes more.
        set_front(game, "minecraft:water");
        game.server.pending_native_sounds.clear();
        dispense_fixture::start_recording();
        let full = dispense(game, half.unwrap());
        let lines = dispense_fixture::stop_recording();
        assert_eq!(front(game), state(game, "minecraft:air"));
        assert_eq!(item(&full), Some(("minecraft:paper".into(), 1)));
        let pickup = lines
            .iter()
            .position(|line| line.ends_with("minecraft:fluid_pickup"));
        let click = lines.iter().position(|line| line.starts_with("L 1000 "));
        assert!(pickup.is_some() && pickup < click, "{lines:?}");
        let sounds: Vec<_> = game
            .server
            .pending_native_sounds
            .iter()
            .map(|s| s.sound.clone())
            .collect();
        assert_eq!(sounds, ["minecraft:item.bucket.fill"]);
        // Full, it can only give: it places what it holds.
        let given = dispense(game, full.unwrap());
        assert_eq!(front(game), state(game, "minecraft:water"));
        assert_eq!(item(&given), Some(("minecraft:paper".into(), 1)));

        // Restore-before-fallback: holding 1500 of 2000 mB facing a 1000 mB water source,
        // `try_pickup_world_fluid` mutates the lent scratch slot to 2000 mB (500 mB accepted)
        // and then aborts because the full bucket did not fit. The abort must restore the lent
        // scratch slot to 1500 mB before `try_place_fluid` runs and extracts 1000 mB, leaving
        // 500 mB (not 1000 mB).
        let mut almost_full = ItemStack::new("minecraft:paper", 1);
        almost_full.data_mut().set_other(
            "minecraft:custom_data",
            Some(Tag::compound([
                ("id", Tag::String("minecraft:water".into())),
                ("amount", Tag::Int(1500)),
            ])),
        );
        game.server.pending_native_sounds.clear();
        dispense_fixture::start_recording();
        let placed = dispense(game, almost_full);
        let lines = dispense_fixture::stop_recording();
        assert_eq!(front(game), state(game, "minecraft:water"));
        assert_eq!(item(&placed), Some(("minecraft:paper".into(), 1)));
        let data = placed
            .as_ref()
            .unwrap()
            .custom_data()
            .expect("restored tank retains 500 mB after fallback placement");
        assert_eq!(
            data.get(&JavaString::from("id")),
            Some(&Tag::String("minecraft:water".into()))
        );
        assert_eq!(data.get(&JavaString::from("amount")), Some(&Tag::Int(500)));
        let place_event = lines
            .iter()
            .position(|line| line.ends_with("minecraft:fluid_place"));
        let click = lines.iter().position(|line| line.starts_with("L 1000 "));
        assert!(place_event.is_some() && place_event < click, "{lines:?}");
        assert!(
            !lines
                .iter()
                .any(|line| line.ends_with("minecraft:fluid_pickup")),
            "{lines:?}"
        );
        let sounds: Vec<_> = game
            .server
            .pending_native_sounds
            .iter()
            .map(|s| s.sound.clone())
            .collect();
        assert_eq!(sounds, ["minecraft:item.bucket.empty"]);
        assert!(game.dropped.is_empty());
        assert_eq!(game.live_scratch_count(), 0);
    });
}

#[test]
#[ignore = "daily"]
fn a_stack_of_custom_containers_gives_their_handler_one_at_a_time() {
    if spawn_dispenser_case("a_stack_of_custom_containers_gives_their_handler_one_at_a_time") {
        return;
    }
    with_dispenser(|game, _| {
        set_front(game, "minecraft:water");
        let left = dispense(game, ItemStack::new("minecraft:glass_bottle", 3));
        // The handler turned every container it saw: one, not the stack.
        assert_eq!(item(&left), Some(("minecraft:glass_bottle".into(), 2)));
        assert_eq!(front(game), state(game, "minecraft:air"));
        let pos = (DISPENSER[0], DISPENSER[1], DISPENSER[2]);
        let held: Vec<_> = game
            .world
            .block_entities
            .get(&pos)
            .unwrap()
            .slots()
            .iter()
            .flatten()
            .map(|stack| (stack.item.to_string(), stack.count))
            .collect();
        assert_eq!(held, [("minecraft:potion".to_owned(), 1)]);
        assert!(game.dropped.is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn dispense_container_preserves_first_slot_caps_one_by_one_insert_and_rolls_back_in_enclosing_tx() {
    if spawn_dispenser_case(
        "dispense_container_preserves_first_slot_caps_one_by_one_insert_and_rolls_back_in_enclosing_tx",
    ) {
        return;
    }
    with_dispenser(|game, level| {
        let pos = (DISPENSER[0], DISPENSER[1], DISPENSER[2]);

        // 1. Actual final first slot (`4 -> 2`, not `stack - 1 = 3`) plus `OneByOne`-capped
        //    insert of 1 potion routed into the dispenser.
        set_front(game, "minecraft:water");
        let left = dispense(game, ItemStack::new("minecraft:clay_ball", 4));
        assert_eq!(item(&left), Some(("minecraft:clay_ball".into(), 2)));
        assert_eq!(front(game), state(game, "minecraft:air"));
        let held: Vec<_> = game
            .world
            .block_entities
            .get(&pos)
            .unwrap()
            .slots()
            .iter()
            .flatten()
            .map(|stack| (stack.item.to_string(), stack.count))
            .collect();
        assert_eq!(held, [("minecraft:potion".to_owned(), 1)]);
        assert!(game.dropped.is_empty());

        // 2. `(None, Some(changed))`: slot 0 emptied after slot 1 took the changed item returns
        //    the changed item directly into the dispenser's active slot without adding a second copy.
        set_front(game, "minecraft:water");
        let replaced = dispense(game, ItemStack::new("minecraft:clay_ball", 2));
        assert_eq!(item(&replaced), Some(("minecraft:potion".into(), 1)));
        assert_eq!(front(game), state(game, "minecraft:air"));
        let held: Vec<_> = game
            .world
            .block_entities
            .get(&pos)
            .unwrap()
            .slots()
            .iter()
            .flatten()
            .map(|stack| (stack.item.to_string(), stack.count))
            .collect();
        assert_eq!(held, [("minecraft:potion".to_owned(), 1)]);

        // 3. Aborting an enclosing transaction around `fluid_util::dispense_container` rolls back
        //    both the world fluid pickup and the dispenser slot write (empty-slot remainder path).
        for cell in game.world.block_entities.get_mut(&pos).unwrap().slots_mut() {
            *cell = None;
        }
        set_front(game, "minecraft:water");
        dispense_fixture::start_recording();
        let aborted_slot = game.storage_transaction(|game, _| {
            let mut answer = None;
            let dim = game.dim.id();
            let stack = ItemStack::new("minecraft:bucket", 3);
            game.native_dispense_item(dim, DISPENSER, Direction::East, &stack, &mut answer);
            assert!(answer.is_some());
            // Tentatively visible inside the enclosing transaction.
            assert_eq!(front(game), state(game, "minecraft:air"));
            let tentative: Vec<_> = game
                .world
                .block_entities
                .get(&pos)
                .unwrap()
                .slots()
                .iter()
                .flatten()
                .map(|s| (s.item.to_string(), s.count))
                .collect();
            assert_eq!(tentative, [("minecraft:water_bucket".to_owned(), 1)]);
            Err::<(), _>(())
        });
        let lines = dispense_fixture::stop_recording();
        assert_eq!(aborted_slot, Err(()));
        assert_eq!(front(game), state(game, "minecraft:water"));
        assert!(
            game.world
                .block_entities
                .get(&pos)
                .unwrap()
                .slots()
                .iter()
                .all(Option::is_none)
        );
        assert!(game.dropped.is_empty());
        assert!(
            lines.is_empty(),
            "aborted tx must publish no events: {lines:?}"
        );

        // 4. Exercise the full tick, including the behaviour reply's own click and smoke.
        //    Every slot holds empty buckets so the new filled one must be dropped regardless
        //    of which slot the dispenser draws.
        for cell in game.world.block_entities.get_mut(&pos).unwrap().slots_mut() {
            *cell = Some(ItemStack::new("minecraft:bucket", 3));
        }
        for commit in [false, true] {
            dispense_fixture::start_recording();
            let outcome = game.storage_transaction(|game, _| {
                assert!(level.block_tick(game, DISPENSER));
                assert_eq!(front(game), state(game, "minecraft:air"));
                assert!(game.dropped.is_empty(), "drop must wait for root commit");
                commit.then_some(()).ok_or(())
            });
            let lines = dispense_fixture::stop_recording();
            assert_eq!(outcome, commit.then_some(()).ok_or(()));
            if commit {
                assert_eq!(front(game), state(game, "minecraft:air"));
                assert_eq!(game.dropped.len(), 1);
                assert_eq!(&*game.dropped[0].stack.item, "minecraft:water_bucket");
                assert_eq!(
                    lines,
                    [
                        "G 6,64,5 minecraft:fluid_pickup",
                        "L 1000 5,64,5 0", // overflow sound
                        "L 2000 5,64,5 5", // overflow smoke
                        "L 1000 5,64,5 0", // reply sound
                        "L 2000 5,64,5 5", // reply smoke
                    ]
                );
            } else {
                assert_eq!(front(game), state(game, "minecraft:water"));
                assert!(game.dropped.is_empty());
                assert!(lines.is_empty(), "abort must be quiet: {lines:?}");
            }
            let count: u32 = game
                .world
                .block_entities
                .get(&pos)
                .unwrap()
                .slots()
                .iter()
                .flatten()
                .map(|stack| stack.count)
                .sum();
            assert_eq!(count, 27 - u32::from(commit));
            assert_eq!(game.live_scratch_count(), 0);
        }
    });
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
