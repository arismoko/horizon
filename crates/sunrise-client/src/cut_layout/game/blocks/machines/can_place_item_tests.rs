use mod_api::ItemResource;
use mod_api::{
    BlockEntityPart, BlockPart, CanPlaceItemEvent, Hooks, ITEMS, ModEntry, Mods, Registrar,
    RegistrationContext, STORAGE,
};
use std::sync::Arc;
use sunrise_core::geometry::Direction;
use sunrise_core::nbt::Tag;
use sunrise_core::position::BlockPos;
use sunrise_core::position::ChunkPos;
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_gameplay_common::game::machine_containers::{PlacementAttempt, PlacementQuery};
use sunrise_gameplay_common::hoppers::Pos;
use sunrise_items::items::ItemStack;
use sunrise_level::containers::BlockEntity;
use sunrise_level::containers::HOPPER_SLOTS;
use sunrise_level::containers::ModContainer;
use sunrise_level::{
    chunk::Chunk,
    containers::{ModBlockEntity, ModBlockEntityFields, ModContainerInsertRule},
    world::World,
};
use sunrise_world::registry::BlockStateId;

const TARGET: Pos = (8, 64, 8);
const ABOVE: Pos = (8, 65, 8);

struct Filter;

#[mod_api::sunrise_part]
impl BlockPart for Filter {}

#[mod_api::sunrise_part]
impl BlockEntityPart for Filter {
    fn can_place_item(&self, event: &mut CanPlaceItemEvent<'_>) -> Option<bool> {
        match event.entity.data.first() {
            Some(1) => return Some(false),
            Some(2) => return None,
            Some(3) => return Some(event.slot == 2 && event.stack.count() == 1),
            Some(4) => return Some(event.slot == 1 && event.stack.count() == 2),
            Some(5) => {
                let barrel = mod_api::__private::block_key("minecraft:barrel").unwrap();
                let metadata =
                    sunrise_gameplay_common::game::api_values::block_metadata_table().unwrap();
                return Some(
                    event.pos == mod_api::BlockPos::new(TARGET.0, TARGET.1, TARGET.2)
                        && event.state.block() == barrel
                        && event.state.property(metadata, "open").unwrap() == Some("true"),
                );
            }
            _ => {}
        }
        match event.slot {
            0 => Some(false),
            1 => Some(true),
            2 => Some(event.entity.items[1].is_none() && event.stack.count() == 1),
            _ => None,
        }
    }
}

#[mod_api::__subscriptions]
impl Hooks for Filter {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let filtered = r.declare_block_entity_type("filtered", Vec::new());
        r.block_entity_part(&filtered, Self).unwrap();
        r.declare_block_entity_type("plain", Vec::new());
    }
}

fn with_filter(test: impl FnOnce(&[ModEntry])) {
    let mods: Mods = vec![ModEntry::new("admission", Box::new(Filter))].into();
    mod_platform::testing::with_mods(mods.clone(), || test(&mods));
}

fn fixture(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    with_filter(|_| {
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                Arc::new({
                    let mut chunk = Chunk::empty(0, 0);
                    chunk.lit = true; // a FULL read of a hand-made column finds it ready
                    chunk
                }),
            );
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            let barrel = game
                .pack
                .assets
                .registry
                .default_state("minecraft:barrel")
                .unwrap();
            game.world
                .set(TARGET.0, TARGET.1, TARGET.2, barrel)
                .unwrap();
            game.world.block_entities.set_mod_containers(
                [(
                    barrel.0,
                    Arc::new(ModContainer {
                        slots: 3,
                        faces: std::array::from_fn(|_| vec![0, 1, 2]),
                        inputs: vec![0, 1, 2],
                        insert_rules: vec![
                            ModContainerInsertRule::Any,
                            ModContainerInsertRule::OnlyWhenEmpty,
                        ],
                        ..Default::default()
                    }),
                )]
                .into_iter()
                .collect(),
            );
            let mut entity = ModBlockEntity::new("admission:filtered".into(), Default::default());
            entity.items = vec![None; 3];
            game.world
                .block_entities
                .insert(TARGET, BlockEntity::Mod(Box::new(entity)));
            test(game);
        });
    });
}

fn source(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    block: &str,
    slots: usize,
) -> BlockStateId {
    let state = game
        .pack
        .assets
        .registry
        .resolve(&sunrise_world::chunk::BlockState {
            name: block.to_owned(),
            properties: [("facing".to_owned(), "down".to_owned())]
                .into_iter()
                .collect(),
        })
        .unwrap();
    game.world.set(ABOVE.0, ABOVE.1, ABOVE.2, state).unwrap();
    let mut items = vec![None; slots];
    items[0] = Some(ItemStack::new("minecraft:stone", 2));
    game.world
        .block_entities
        .insert(ABOVE, BlockEntity::Chest(items));
    state
}

fn target<'a>(game: &'a sunrise_gameplay::game::ServerGame<'_>) -> &'a ModBlockEntity {
    let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get(&TARGET) else {
        panic!("expected mod block entity");
    };
    entity
}

#[test]
#[ignore = "daily"]
fn hopper_skips_a_refused_slot_and_preserves_the_stack_if_all_refuse() {
    fixture(|game| {
        source(game, "minecraft:hopper", HOPPER_SLOTS);
        game.tick_hopper(ABOVE);
        assert_eq!(
            target(game).items,
            [None, Some(ItemStack::new("minecraft:stone", 1)), None]
        );
        let BlockEntity::Mod(entity) = game.world.block_entities.get_mut(&TARGET).unwrap() else {
            unreachable!();
        };
        entity.data = vec![1];
        let before = (entity.items.clone(), entity.revision());
        let source_before = game
            .world
            .block_entities
            .get(&ABOVE)
            .unwrap()
            .slots()
            .to_vec();
        game.server.hoppers.get_mut(&ABOVE).unwrap().cooldown = 0;
        game.tick_hopper(ABOVE);
        assert_eq!(
            game.world.block_entities.get(&ABOVE).unwrap().slots(),
            source_before
        );
        assert_eq!(target(game).items, before.0);
        assert_eq!(target(game).revision(), before.1);
    });
}

#[test]
#[ignore = "daily"]
fn shared_admission_part_distinguishes_blocks_and_current_state() {
    fixture(|game| {
        let registry = game.pack.assets.registry;
        let closed = registry.default_state("minecraft:barrel").unwrap();
        let open = registry
            .resolve(&sunrise_world::chunk::BlockState {
                name: "minecraft:barrel".to_owned(),
                properties: [("open".to_owned(), "true".to_owned())]
                    .into_iter()
                    .collect(),
            })
            .unwrap();
        let chest = registry.default_state("minecraft:chest").unwrap();
        let other = (TARGET.0 + 1, TARGET.1, TARGET.2);
        game.world.block_entities.set_mod_containers(
            [closed, open, chest]
                .into_iter()
                .map(|state| {
                    (
                        state.0,
                        Arc::new(ModContainer {
                            slots: 1,
                            inputs: vec![0],
                            ..Default::default()
                        }),
                    )
                })
                .collect(),
        );
        for (pos, state) in [(TARGET, closed), (other, open)] {
            game.world.set(pos.0, pos.1, pos.2, state).unwrap();
            let entity = ModBlockEntity::new(
                "admission:filtered".into(),
                ModBlockEntityFields {
                    items: vec![None],
                    data: vec![5],
                    ..Default::default()
                },
            );
            game.world
                .block_entities
                .insert(pos, BlockEntity::Mod(Box::new(entity)));
        }
        let mut attempt = PlacementAttempt::default();
        let stack = ItemStack::new("minecraft:stone", 1);
        for (pos, state, expected) in [
            (TARGET, closed, false),
            (TARGET, open, true),
            (other, open, false),
            (TARGET, chest, false),
        ] {
            game.world.set(pos.0, pos.1, pos.2, state).unwrap();
            let containers = game.containers();
            let view = containers.view(pos, false).unwrap();
            assert_eq!(
                containers.can_place(&view, 0, &stack, PlacementQuery::Offer(&mut attempt)),
                expected,
                "at {pos:?} with {state:?}",
            );
        }
    });
}

#[test]
#[ignore = "daily"]
fn dropper_skips_a_refused_slot() {
    fixture(|game| {
        let state = source(game, "minecraft:dropper", 9);
        game.dispense_from(BlockPos::new(ABOVE.0, ABOVE.1, ABOVE.2), state)
            .unwrap();
        assert_eq!(
            target(game).items,
            [None, Some(ItemStack::new("minecraft:stone", 1)), None]
        );
        assert_eq!(
            game.world.block_entities.get(&ABOVE).unwrap().slots()[0],
            Some(ItemStack::new("minecraft:stone", 1))
        );
        assert!(game.dropped.is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn default_items_insert_checks_admission_and_overrides_the_declared_rule() {
    fixture(|game| {
        let level = game.level("minecraft:overworld").unwrap();
        let at = [TARGET.0, TARGET.1, TARGET.2];
        let handler = level
            .capability(game, &ITEMS, at, Some(Direction::Up))
            .unwrap();
        let stack = ItemStack::new("minecraft:stone", 1);
        use sunrise_gameplay_common::game::api_values::TryIntoApi;
        let resource = ItemResource::of(&stack.try_into_api().unwrap());
        assert_eq!(
            game.storage_transaction(|host, tx| {
                let first = handler.insert(host, tx, &resource, 2).map_err(|_| ())?;
                // Slot 1's declared OnlyWhenEmpty would refuse this subsequent insert.
                let second = handler
                    .insert_at(host, tx, 1, &resource, 1)
                    .map_err(|_| ())?;
                Ok::<_, ()>((first, second))
            }),
            Ok((2, 1))
        );
        assert_eq!(
            target(game).items,
            [None, Some(ItemStack::new("minecraft:stone", 3)), None]
        );
    });
}

#[test]
#[ignore = "daily"]
fn refusing_a_capability_insert_does_not_journal_the_slot() {
    fixture(|game| {
        let level = game.level("minecraft:overworld").unwrap();
        let at = [TARGET.0, TARGET.1, TARGET.2];
        let handler = level
            .capability(game, &ITEMS, at, Some(Direction::Up))
            .unwrap();
        use sunrise_gameplay_common::game::api_values::TryIntoApi;
        let resource =
            ItemResource::of(&ItemStack::new("minecraft:stone", 1).try_into_api().unwrap());
        let before = (target(game).revision(), game.transactions.writes());
        let aborted = game.storage_transaction(|game, tx| {
            assert_eq!(handler.insert_at(game, tx, 0, &resource, 1), Ok(0));
            assert_eq!(
                sunrise_gameplay_common::game::transactions::TransactionsGameExt::journal_kept(
                    game
                ),
                (0, 0)
            );
            Err::<(), ()>(())
        });
        assert_eq!(aborted, Err(()));
        assert_eq!(target(game).items, [None, None, None]);
        assert_eq!(
            (target(game).revision(), game.transactions.writes()),
            before
        );
    });
}

fn prepare_counted_admission(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    mode: i32,
    counts: [u32; 3],
) -> ItemResource {
    let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get_mut(&TARGET) else {
        unreachable!();
    };
    entity.data = vec![mode];
    entity.items = counts
        .map(|count| Some(ItemStack::new("minecraft:stone", count)))
        .into();
    use sunrise_gameplay_common::game::api_values::TryIntoApi;
    ItemResource::of(&ItemStack::new("minecraft:stone", 1).try_into_api().unwrap())
}

#[test]
#[ignore = "daily"]
fn admission_checks_the_offer_before_clamping_to_room() {
    fixture(|game| {
        let resource = prepare_counted_admission(game, 3, [64, 64, 63]);
        let level = game.level("minecraft:overworld").unwrap();
        let at = [TARGET.0, TARGET.1, TARGET.2];
        let items = level
            .capability(game, &ITEMS, at, Some(Direction::Up))
            .unwrap();
        let storage = level
            .capability(game, &STORAGE, at, Some(Direction::Up))
            .unwrap();
        assert_eq!(
            game.storage_transaction(|game, tx| {
                let indexed = items.insert_at(game, tx, 2, &resource, 2).map_err(|_| ())?;
                let by_kind = storage.insert(game, tx, &resource, 2).map_err(|_| ())?;
                Ok::<_, ()>((indexed, by_kind))
            }),
            Ok((0, 0))
        );
        assert_eq!(
            target(game).items[2],
            Some(ItemStack::new("minecraft:stone", 63))
        );
    });
}

fn transfer_to_filtered_container(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    resource: &ItemResource,
    amount: u64,
) {
    source(game, "minecraft:hopper", HOPPER_SLOTS);
    let level = game.level("minecraft:overworld").unwrap();
    let from = level
        .capability(game, &STORAGE, [ABOVE.0, ABOVE.1, ABOVE.2], None)
        .unwrap();
    let to = level
        .capability(
            game,
            &STORAGE,
            [TARGET.0, TARGET.1, TARGET.2],
            Some(Direction::Up),
        )
        .unwrap();
    assert_eq!(to.space_for(game, resource), Ok(amount));
    assert_eq!(
        game.storage_transaction(|game, tx| {
            mod_api::transfer(game, &from, &to, resource, amount, tx)
        }),
        Ok(amount)
    );
    assert_eq!(from.amount(game, resource), Ok(2 - amount));
    assert_eq!(
        target(game).items,
        vec![Some(ItemStack::new("minecraft:stone", 64)); 3]
    );
}

#[test]
#[ignore = "daily"]
fn storage_transfer_can_insert_into_a_slot_that_only_admits_one() {
    fixture(|game| {
        let resource = prepare_counted_admission(game, 3, [64, 64, 63]);
        transfer_to_filtered_container(game, &resource, 1);
    });
}

#[test]
#[ignore = "daily"]
fn storage_transfer_can_insert_two_despite_a_declared_rule_the_part_replaces() {
    fixture(|game| {
        // The only slot with room declares OnlyWhenEmpty, but the part admits exactly two.
        let resource = prepare_counted_admission(game, 4, [64, 62, 64]);
        transfer_to_filtered_container(game, &resource, 2);
    });
}

#[test]
#[ignore = "daily"]
fn unrepresentable_hook_inputs_refuse_insertion_instead_of_using_any() {
    fixture(|game| {
        let stone = ItemStack::new("minecraft:stone", 1);
        let entity = target(game);
        let mut attempt = PlacementAttempt::default();
        let unloaded = World::new(BlockStateId(u32::MAX), 0);
        assert_eq!(
            attempt.ask(&game.world, TARGET, entity, 1, &stone),
            Some(true)
        );
        assert_eq!(
            attempt.ask(&unloaded, TARGET, entity, 1, &stone),
            Some(false)
        );

        let view = game.containers().view(TARGET, false).unwrap();
        let unknown = ItemStack::new("admission:not_registered", 1);
        assert_eq!(
            game.add_item(None, &view, unknown.clone(), None),
            Some(unknown.clone())
        );
        assert_eq!(target(game).items, [None, None, None]);
        let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get_mut(&TARGET) else {
            unreachable!();
        };
        entity.items[2] = Some(unknown.clone());
        assert_eq!(game.add_item(None, &view, stone.clone(), None), Some(stone));
        assert_eq!(target(game).items, [None, None, Some(unknown)]);
    });
}

#[test]
#[ignore = "daily"]
fn storage_rechecks_the_entity_after_an_accepted_write_in_the_same_attempt() {
    fixture(|game| {
        let level = game.level("minecraft:overworld").unwrap();
        let at = [TARGET.0, TARGET.1, TARGET.2];
        let handler = level
            .capability(game, &STORAGE, at, Some(Direction::Up))
            .unwrap();
        use sunrise_gameplay_common::game::api_values::TryIntoApi;
        let resource =
            ItemResource::of(&ItemStack::new("minecraft:stone", 1).try_into_api().unwrap());
        assert_eq!(
            game.storage_transaction(|host, tx| handler.insert(host, tx, &resource, 65)),
            Ok(64)
        );
        // Slot 2 would take the remainder if the query still saw slot 1 empty.
        assert_eq!(
            target(game).items,
            [None, Some(ItemStack::new("minecraft:stone", 64)), None]
        );
    });
}

#[test]
#[ignore = "daily"]
fn loot_replace_block_uses_the_container_rule_for_each_slot() {
    fixture(|game| {
        game.build_registries();
        let result = |game: &mut sunrise_gameplay::game::ServerGame<'_>, slot| {
            game.as_console(|game| game.run_command(&format!(
            r#"loot replace block 8 64 8 container.{slot} 1 loot {{"pools":[{{"rolls":1,"entries":[{{"type":"minecraft:item","name":"minecraft:stone"}}]}}]}}"#
        )))
        };
        result(game, 0);
        assert!(target(game).items.iter().all(Option::is_none));
        assert!(result(game, 1));
        assert_eq!(
            target(game).items,
            [None, Some(ItemStack::new("minecraft:stone", 1)), None]
        );
    });
}

#[test]
#[ignore = "daily"]
fn no_answer_falls_back_to_the_declared_insert_rule() {
    fixture(|game| {
        let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get_mut(&TARGET) else {
            unreachable!();
        };
        entity.data = vec![2];
        let c = game.containers();
        let view = c.view(TARGET, false).unwrap();
        let offered = ItemStack::new("minecraft:stone", 1);
        assert!(c.can_place(
            &view,
            1,
            &offered,
            PlacementQuery::Offer(&mut PlacementAttempt::default())
        ));
        *game.slot_mut_at(&view, 1).unwrap() = Some(offered.clone());
        assert!(!game.containers().can_place(
            &view,
            1,
            &offered,
            PlacementQuery::Offer(&mut PlacementAttempt::default()),
        ));
    });
}

#[test]
#[cfg(not(any(feature = "system-allocator", feature = "alloc-census")))]
fn native_admission_allocates_only_when_preparing_subscribed_snapshots() {
    use std::hint::black_box;
    use sunrise_threads::allocation_count::measure;

    with_filter(|mods| {
        sunrise_gameplay_common::game::api_values::install_parts(mods).unwrap();
        mod_api::__private::resolve_items(["minecraft:stone"]).unwrap();
        let facts =
            r#"{"defaults":{"minecraft:air":0},"states":[["a",true,false,"","minecraft:air"]]}"#;
        let registry =
            sunrise_world::registry::BlockRegistry::from_facts_json(facts.as_bytes()).unwrap();
        sunrise_gameplay_common::game::api_values::resolve_block_definitions(&registry).unwrap();
        let world = World::new(registry.default_state("minecraft:air").unwrap(), 0);
        let mut stack = ItemStack::new("minecraft:stone", 2);
        stack
            .data_mut()
            .set_other("minecraft:max_stack_size", Some(Tag::Int(16)));
        let make_entity = |id: &str| {
            let mut entity = ModBlockEntity::new(id.into(), Default::default());
            entity.items = vec![None; 3];
            entity
        };
        // Even when there is a subscriber for another type, the ordinary container
        // doesn't convert the component-bearing offer or the entity's saved fields.
        let plain = make_entity("admission:plain");
        let mut attempt = PlacementAttempt::default();
        let (plain_answers, plain_allocs) = measure(|| {
            [0, 1].map(|slot| black_box(attempt.ask(&world, (0, 64, 0), &plain, slot, &stack)))
        });
        assert_eq!(plain_answers, [None, None]);
        assert_eq!(plain_allocs, 0);

        let mut filtered = make_entity("admission:filtered");
        let mut attempt = PlacementAttempt::default();
        let (first, setup_allocs) =
            measure(|| black_box(attempt.ask(&world, TARGET, &filtered, 0, &stack)));
        assert_eq!(first, Some(false));
        assert!(
            setup_allocs > 0,
            "allocation counter must observe snapshot creation"
        );
        stack.count = 1;
        let (next, reuse_allocs) =
            measure(|| black_box(attempt.ask(&world, TARGET, &filtered, 2, &stack)));
        assert_eq!(next, Some(true));
        assert_eq!(reuse_allocs, 0);

        *filtered.item_slot_mut(1) = Some(ItemStack::new("minecraft:stone", 1));
        attempt.wrote_entity();
        let (refreshed, refresh_allocs) =
            measure(|| black_box(attempt.ask(&world, TARGET, &filtered, 2, &stack)));
        assert_eq!(refreshed, Some(false));
        assert!(refresh_allocs > 0);
        println!(
            "allocations: unsubscribed pair={plain_allocs}, subscribed setup={setup_allocs}, next slot={reuse_allocs}, changed entity={refresh_allocs}"
        );
    });
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
