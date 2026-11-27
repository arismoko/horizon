//! #3720: real Game ticks, Parts storage and transaction close/restore traces.
use mod_api::*;
use std::sync::{Arc, Mutex};
use sunrise_core::{
    geometry::Vec3,
    position::{BlockPos, ChunkPos},
};
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_gameplay_common::game::spill_common as spill;
use sunrise_gameplay_common::hoppers::Pos;
use sunrise_items::items::ItemStack;
use sunrise_level::{
    chunk::Chunk,
    containers::{BlockEntity, ModBlockEntity, ModContainer, ModContainerInsertRule},
};
use {
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

const HOPPER: Pos = (8, 64, 8);
const ABOVE: Pos = (8, 65, 8);
const BELOW: Pos = (8, 63, 8);
const SINK: [i32; 3] = [10, 63, 8];
const OTHER: [i32; 3] = [11, 63, 8];
type Trace = Arc<Mutex<Vec<String>>>;

#[derive(Default)]
struct Rules {
    slots: Option<usize>,
    zero_capacity_slot: Option<usize>,
    accept: Option<&'static str>,
    notify: bool,
    release_first: bool,
    nontransactional: bool,
    refuse_extract: bool,
    write_on_refuse: bool,
    rename_source_on_notice: bool,
    move_source_on_notice: bool,
    replace_source_on_notice: bool,
    forward_to_vanilla: bool,
    external: Vec<Option<mod_api::ItemStack>>,
    source_seen: Vec<(u32, u32)>,
}

#[derive(Clone)]
struct Probe {
    trace: Trace,
    rules: Arc<Mutex<Rules>>,
}

struct Storage {
    probe: Probe,
    at: BlockSide,
}

impl Storage {
    fn entity(&self, host: &dyn Host) -> BlockEntityRef {
        host.block_entity(self.at.level, self.at.pos).unwrap()
    }
    fn stack(&self, host: &dyn Host, slot: usize) -> Option<mod_api::ItemStack> {
        let rules = self.probe.rules.lock().unwrap();
        if rules.nontransactional {
            return rules.external.get(slot).cloned().flatten();
        }
        drop(rules);
        self.entity(host).item_slot(host, slot as u32).unwrap()
    }
}

impl ResourceHandler<ItemResource> for Storage {
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        self.probe.rules.lock().unwrap().slots.unwrap_or(2)
    }
    fn resource(&self, host: &mut dyn ServerHost, slot: usize) -> Option<ItemResource> {
        self.stack(host, slot).as_ref().map(ItemResource::of)
    }
    fn amount(&self, host: &mut dyn ServerHost, slot: usize) -> u32 {
        self.stack(host, slot).map_or(0, |stack| stack.count())
    }
    fn capacity(&self, _: &mut dyn ServerHost, slot: usize, _: Option<&ItemResource>) -> u32 {
        if self.probe.rules.lock().unwrap().zero_capacity_slot == Some(slot) {
            0
        } else {
            64
        }
    }
    fn insert_at(
        &self,
        _: &mut dyn ServerHost,
        _: &TransactionContext,
        _: usize,
        _: &ItemResource,
        _: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        panic!("automation must use the storage's indexless insertion")
    }
    fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        resource: &ItemResource,
        amount: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        assert_eq!(amount, 1);
        assert_eq!(tx.depth(), 0);
        assert!(host.transaction_matches(Some(tx)));
        let item = mod_api::__private::item_key_id(resource.as_stack().item()).unwrap();
        self.probe
            .trace
            .lock()
            .unwrap()
            .push(format!("insert:{item}"));
        let hopper = self
            .at
            .level
            .capability(host, &ITEMS, [8, 64, 8], None)
            .unwrap();
        let source = (hopper.amount(host, 0), hopper.amount(host, 1));
        let (accept, notify, write_on_refuse, forward) = {
            let mut rules = self.probe.rules.lock().unwrap();
            rules.source_seen.push(source);
            (
                rules.accept == Some(item),
                rules.notify,
                rules.write_on_refuse,
                rules.forward_to_vanilla,
            )
        };
        if accept && forward {
            let sink = self.at.level.capability(host, &ITEMS, SINK, None).unwrap();
            return sink.insert(host, tx, resource, amount);
        }
        let entity = self.entity(host);
        if accept {
            entity
                .set_item_slot(host, tx, 0, Some(resource.as_stack().clone()))
                .unwrap();
        } else if write_on_refuse {
            entity.set_data_slot(host, 0, 1).unwrap();
        }
        if notify {
            entity.set_changed(host).unwrap();
        }
        Ok(u32::from(accept))
    }
    fn extract_at(
        &self,
        host: &mut dyn ServerHost,
        tx: &TransactionContext,
        slot: usize,
        resource: &ItemResource,
        amount: u32,
    ) -> Result<u32, TransactionError<Overanswered>> {
        assert_eq!(amount, 1);
        assert_eq!(tx.depth(), 0);
        assert!(host.transaction_matches(Some(tx)));
        self.probe
            .trace
            .lock()
            .unwrap()
            .push(format!("extract:{slot}"));
        let (notify, release_first, nontransactional, refuse) = {
            let rules = self.probe.rules.lock().unwrap();
            (
                rules.notify,
                rules.release_first,
                rules.nontransactional,
                rules.refuse_extract,
            )
        };
        if refuse {
            return Ok(0);
        }
        let Some(stack) = self.stack(host, slot) else {
            return Ok(0);
        };
        assert_eq!(ItemResource::of(&stack), *resource);
        let remaining = stack.shrink(1);
        let entity = self.entity(host);
        if nontransactional {
            self.probe.rules.lock().unwrap().external[slot] = remaining;
        } else {
            entity
                .set_item_slot(host, tx, slot as u32, remaining)
                .unwrap();
        }
        if release_first {
            let hopper = self
                .at
                .level
                .capability(host, &ITEMS, [8, 64, 8], None)
                .unwrap();
            let held = hopper.resource(host, 0).unwrap();
            assert_eq!(hopper.extract_at(host, tx, 0, &held, 63), Ok(63));
        }
        if notify {
            entity.set_changed(host).unwrap();
        }
        Ok(1)
    }
}

#[mod_api::sunrise_part]
impl BlockPart for Probe {
    fn contents_changed(&self, host: &mut dyn ServerHost, event: &mut ContentsChangedEvent) {
        let pos = (event.pos.x, event.pos.y, event.pos.z);
        if pos == HOPPER {
            self.trace
                .lock()
                .unwrap()
                .push(format!("mark:hopper:{:?}", host.transaction_lifecycle()));
        } else if pos == ABOVE || pos == BELOW {
            let entity = host
                .block_entity(event.level, [pos.0, pos.1, pos.2])
                .unwrap();
            let amount: u32 = (0..2)
                .map(|slot| {
                    entity
                        .item_slot(host, slot)
                        .ok()
                        .flatten()
                        .map_or(0, |s| s.count())
                })
                .sum();
            self.trace.lock().unwrap().push(format!(
                "notice:{amount}:{:?}",
                host.transaction_lifecycle()
            ));
            if pos == BELOW && self.rules.lock().unwrap().move_source_on_notice {
                let source = event
                    .level
                    .capability(host, &ITEMS, [8, 64, 8], None)
                    .unwrap();
                let sink = event.level.capability(host, &ITEMS, OTHER, None).unwrap();
                host.transaction(|host, tx| {
                    if let Some(resource) = source.resource(host, 0) {
                        let taken = source.extract_at(host, tx, 0, &resource, 1).unwrap();
                        assert_eq!(sink.insert(host, tx, &resource, taken), Ok(taken));
                    }
                    if self.rules.lock().unwrap().replace_source_on_notice {
                        let stick = ItemResource::of(
                            &mod_api::ItemStack::new(
                                mod_api::__private::item_key("minecraft:stick").unwrap(),
                                1,
                            )
                            .unwrap(),
                        );
                        assert_eq!(sink.extract_at(host, tx, 0, &stick, 1), Ok(1));
                        assert_eq!(source.insert_at(host, tx, 0, &stick, 1), Ok(1));
                    }
                    Ok::<_, ()>(())
                })
                .unwrap();
            }
            if pos == BELOW && self.rules.lock().unwrap().rename_source_on_notice {
                let source = host.block_entity(event.level, [8, 64, 8]).unwrap();
                assert!(
                    source
                        .set_name(host, Some("inside insertion notice"))
                        .unwrap()
                );
            }
        }
    }
    fn tick_block(&self, _: &mut dyn LevelWriteHost, _: &mut TickBlockEvent) -> Flow {
        self.trace.lock().unwrap().push("comparator".into());
        Flow::Continue
    }
}

#[derive(Clone)]
struct CommitMover(Probe);

#[mod_api::sunrise_part]
impl BlockPart for CommitMover {
    fn contents_changed(&self, host: &mut dyn ServerHost, event: &mut ContentsChangedEvent) {
        if [event.pos.x, event.pos.y, event.pos.z] != SINK {
            return;
        }
        let source = event
            .level
            .capability(host, &ITEMS, [8, 64, 8], None)
            .unwrap();
        let amount = source.amount(host, 0);
        self.0.trace.lock().unwrap().push(format!(
            "sink-source:{amount}:{:?}",
            host.transaction_lifecycle()
        ));
        if let Some(resource) = source.resource(host, 0) {
            let other = event.level.capability(host, &ITEMS, OTHER, None).unwrap();
            host.transaction(|host, tx| {
                assert_eq!(source.extract_at(host, tx, 0, &resource, 1), Ok(1));
                assert_eq!(other.insert(host, tx, &resource, 1), Ok(1));
                Ok::<_, ()>(())
            })
            .unwrap();
        }
    }
}

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        for block in [
            "minecraft:barrel",
            "minecraft:chest",
            "minecraft:composter",
            "minecraft:decorated_pot",
        ] {
            let key = r.block_key(block);
            let probe = self.clone();
            r.block_capability_before_game(&ITEMS, &key, move |_, at, _, _, offer| {
                if at.pos == SINK || at.pos == OTHER {
                    return;
                }
                offer.offer(|| {
                    probe
                        .trace
                        .lock()
                        .unwrap()
                        .push(format!("face:{:?}", at.side.unwrap()));
                    Box::new(Storage {
                        probe: probe.clone(),
                        at,
                    })
                });
            });
        }
        for block in ["minecraft:barrel", "minecraft:hopper"] {
            let key = r.block_key(block);
            r.block_part(key, self.clone());
        }
        let chest = r.block_key("minecraft:chest");
        r.block_part(chest, CommitMover(self.clone()));
        let comparator = r.block_key("minecraft:comparator");
        r.block_part(comparator, self.clone());
        r.entity_capability_before_game(
            &ENTITY_AUTOMATION,
            "minecraft:chest_minecart",
            |_, _, _| panic!("vanilla container entities must not ask their storage providers"),
        );
    }
}

fn fixture(
    name: &str,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, LevelRef, Probe),
) {
    if run_fresh_cases(name, "SUNRISE_STORAGE_ATTEMPTS_CHILD", &["game"]) {
        return;
    }
    let probe = Probe {
        trace: Arc::default(),
        rules: Arc::default(),
    };
    let mods: Mods = vec![ModEntry::new("storage_attempts", Box::new(probe.clone()))].into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            let level = game.level("minecraft:overworld").unwrap();
            place(game, HOPPER, "minecraft:hopper");
            game.world
                .block_entities
                .insert(HOPPER, BlockEntity::Chest(vec![None; 5]));
            let barrel = game
                .pack
                .assets
                .registry
                .default_state("minecraft:barrel")
                .unwrap();
            game.world.block_entities.set_mod_containers(
                [(
                    barrel.0,
                    Arc::new(ModContainer {
                        slots: 2,
                        inputs: vec![0, 1],
                        outputs: vec![0, 1],
                        faces: std::array::from_fn(|_| vec![0, 1]),
                        insert_rules: Vec::new(),
                        menu: None,
                    }),
                )]
                .into_iter()
                .collect(),
            );
            game.transactions.trace = Some(probe.trace.clone());
            probe.trace.lock().unwrap().clear();
            test(game, level, probe);
        })
    });
}

fn default_fixture(name: &str, test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    if run_fresh_cases(name, "SUNRISE_MOD_CONTAINER_RULES_CHILD", &["game"]) {
        return;
    }
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(0, 0))
            .unwrap();
        test(game);
    });
}

fn place<'g>(game: &mut impl sunrise_gameplay_common::game::SideGame<'g>, pos: Pos, block: &str) {
    let state = game.pack.assets.registry.default_state(block).unwrap();
    game.world.set(pos.0, pos.1, pos.2, state).unwrap();
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn item_capability_presence_does_not_open_loot_or_load_chunks() {
    use sunrise_core::nbt::Tag;
    use {
        crate::game::server_side::native_entity_refs::tests::with_game_role_client,
        sunrise_gameplay::game::native_entity_refs::tests::fixture_root,
        sunrise_gameplay::game::native_entity_refs::tests::with_game_role,
        sunrise_gameplay_common::game::execution_role::ExecutionRole,
    };

    if run_fresh_cases(
        "cut_layout::game::server_side::machines::native_items::storage_attempt_tests::item_capability_presence_does_not_open_loot_or_load_chunks",
        "SUNRISE_ITEM_PRESENCE_CHILD",
        &["server", "client"],
    ) {
        return;
    }
    let client = std::env::var("SUNRISE_ITEM_PRESENCE_CHILD").unwrap() == "client";
    macro_rules! check_presence {
        ($game:expr, $is_server:expr) => {{
            let game = &mut *$game;
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let chest = [1, 64, 0];
            let pot = [4, 64, 0];
            let composter = [5, 64, 0];
            place(game, (1, 64, 0), "minecraft:chest");
            game.world
                .block_entities
                .insert((1, 64, 0), BlockEntity::Chest(vec![None; 27]));
            place(game, (1, 65, 0), "minecraft:stone");
            place(game, (3, 64, 0), "minecraft:campfire");
            game.world
                .block_entities
                .insert((3, 64, 0), BlockEntity::Campfire(Box::default()));
            place(game, (4, 64, 0), "minecraft:decorated_pot");
            place(game, (5, 64, 0), "minecraft:composter");
            let pending = Tag::compound([
                ("id", Tag::String("minecraft:decorated_pot".into())),
                ("x", Tag::Int(pot[0])),
                ("y", Tag::Int(pot[1])),
                ("z", Tag::Int(pot[2])),
                (
                    "LootTable",
                    Tag::String("minecraft:chests/simple_dungeon".into()),
                ),
            ]);
            spill::put_saved_entity(&mut game.world, pot, pending.clone());
            game.world.block_log = Some(Vec::new());
            let before = (
                game.world.modified.clone(),
                game.world.random.state(),
                game.world.chunks.len(),
            );
            let cases = [
                (chest, None, Some(true)),
                ([1, 65, 0], None, Some(false)),
                ([3, 64, 0], None, Some(false)),
                (pot, None, Some(true)),
                (composter, None, Some(false)),
                (composter, Some(Direction::North), Some(false)),
                (composter, Some(Direction::Up), Some(true)),
                (composter, Some(Direction::Down), Some(true)),
                ([16, 64, 0], None, None),
                ([0, game.world.top_y(), 0], None, None),
            ];
            for (pos, side, expected) in cases {
                game.with_level(|_, world, ticks| {
                    assert_eq!(
                        world.view(ticks).has_item_capability(pos, side),
                        expected,
                        "{pos:?} {side:?}"
                    );
                });
            }
            (pot, pending, before, cases)
        }};
    }
    if client {
        with_game_role_client(fixture_root(), ExecutionRole::Client, |game| {
            let (pipe, _peer) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(pipe, "StorageContents").unwrap();
            game.client.remote.as_mut().unwrap().logged_in = true;
            let (pot, pending, before, _) = check_presence!(game, false);
            assert_eq!(spill::saved_entity(&game.world, pot), Some(pending));
            assert_eq!(
                (
                    game.world.modified.clone(),
                    game.world.random.state(),
                    game.world.chunks.len()
                ),
                before
            );
            assert!(game.world.block_log.as_ref().unwrap().is_empty());
        });
    } else {
        with_game_role(fixture_root(), ExecutionRole::IntegratedServer, |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.sync_tick_chunks();
            let (pot, pending, before, cases) = check_presence!(game, true);
            let level = game.level(game.dim.id()).unwrap();
            for (pos, side, expected) in cases {
                assert_eq!(
                    level.capability(game, &ITEMS, pos, side).is_some(),
                    expected == Some(true)
                );
            }
            assert_eq!(spill::saved_entity(&game.world, pot), Some(pending));
            assert_eq!(
                (
                    game.world.modified.clone(),
                    game.world.random.state(),
                    game.world.chunks.len()
                ),
                before
            );
            assert!(game.world.block_log.as_ref().unwrap().is_empty());

            // The real handler uses the saved slot; aborting a parent also restores a
            // newly created tag written by its committed child.
            let empty_pot = [6, 64, 0];
            place(game, (6, 64, 0), "minecraft:decorated_pot");
            let cap = level.capability(game, &ITEMS, empty_pot, None).unwrap();
            let stick = ItemResource::of(
                &mod_api::ItemStack::new(
                    game.item_key(&Identifier::parse("minecraft:stick").unwrap())
                        .unwrap(),
                    1,
                )
                .unwrap(),
            );
            let aborted: Result<(), ()> = game.storage_transaction(|game, _| {
                game.storage_transaction(|game, tx| {
                    assert_eq!(cap.insert_at(game, tx, 0, &stick, 3), Ok(3));
                    assert_eq!(cap.amount(game, 0), 3);
                    Ok(())
                })?;
                Err(())
            });
            assert_eq!(aborted, Err(()));
            assert!(spill::saved_entity(&game.world, empty_pot).is_none());
            game.storage_transaction(|game, tx| {
                assert_eq!(cap.insert_at(game, tx, 0, &stick, 3), Ok(3));
                assert_eq!(cap.extract_at(game, tx, 0, &stick, 2), Ok(2));
                Ok::<_, ()>(())
            })
            .unwrap();
            assert_eq!(cap.amount(game, 0), 1);
        });
    }
}

#[test]
#[ignore = "daily"]
fn a_committed_pot_write_in_another_level_survives_the_parent_abort() {
    use sunrise_core::nbt::Tag;
    use {
        sunrise_gameplay::game::native_entity_refs::tests::fixture_root,
        sunrise_gameplay::game::native_entity_refs::tests::with_game_role,
        sunrise_gameplay_common::game::Dim,
        sunrise_gameplay_common::game::execution_role::ExecutionRole,
    };

    if run_fresh_cases(
        "cut_layout::game::server_side::machines::native_items::storage_attempt_tests::a_committed_pot_write_in_another_level_survives_the_parent_abort",
        "SUNRISE_CROSS_LEVEL_SAVED_POT_CHILD",
        &["server"],
    ) {
        return;
    }

    let pos = [4, 64, 0];
    with_game_role(fixture_root(), ExecutionRole::IntegratedServer, |game| {
        let mut setup = |dim, item| {
            game.enter_level(dim).unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.sync_tick_chunks();
            place(game, (pos[0], pos[1], pos[2]), "minecraft:decorated_pot");
            let tag = Tag::compound([
                ("id", Tag::String("minecraft:decorated_pot".into())),
                ("x", Tag::Int(pos[0])),
                ("y", Tag::Int(pos[1])),
                ("z", Tag::Int(pos[2])),
                (
                    "item",
                    sunrise_sim::save::item_to_nbt(&ItemStack::new(item, 3), &Default::default()),
                ),
            ]);
            spill::put_saved_entity(&mut game.world, pos, tag.clone());
            tag
        };
        setup(Dim::Nether, "minecraft:stone");
        let overworld_pot = setup(Dim::Overworld, "minecraft:stick");
        let stone = ItemResource::of(
            &mod_api::ItemStack::new(
                game.item_key(&Identifier::parse("minecraft:stone").unwrap())
                    .unwrap(),
                1,
            )
            .unwrap(),
        );

        let aborted: Result<(), ()> = game.storage_transaction(|game, _| {
            game.enter_level(Dim::Nether).unwrap();
            let level = game.level(game.dim.id()).unwrap();
            let pot = level.capability(game, &ITEMS, pos, None).unwrap();
            game.storage_transaction(|game, tx| {
                assert_eq!(pot.extract_at(game, tx, 0, &stone, 1), Ok(1));
                Ok(())
            })?;
            game.enter_level(Dim::Overworld).unwrap();
            Err(())
        });
        assert_eq!(aborted, Err(()));
        assert_eq!(spill::saved_entity(&game.world, pos), Some(overworld_pot));
        game.enter_level(Dim::Nether).unwrap();
        let tag = spill::saved_entity(&game.world, pos).unwrap();
        assert_eq!(
            sunrise_sim::save::item_from_nbt(tag.get("item").unwrap()),
            Some(ItemStack::new("minecraft:stone", 2))
        );
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn block_capabilities_contain_panics_under_the_provider_owner() {
    use mod_api::content::crash::current_native_mod;
    use sunrise_level::crash;
    use {
        sunrise_gameplay::game::native_entity_refs::tests::fixture_root,
        sunrise_gameplay::game::native_entity_refs::tests::with_game_role,
        sunrise_gameplay_common::game::execution_role::ExecutionRole,
    };

    const FLAG: &str = "SUNRISE_CAPABILITY_PROVIDER_OWNER_CHILD";
    const DISCOVERY: [i32; 3] = [1, 64, 0];
    const CONSTRUCTION: [i32; 3] = [2, 64, 0];
    if run_fresh_cases(
        "cut_layout::game::server_side::machines::native_items::storage_attempt_tests::block_capabilities_contain_panics_under_the_provider_owner",
        FLAG,
        &["client", "server"],
    ) {
        return;
    }

    type Calls = Arc<Mutex<Vec<(&'static str, Option<&'static str>)>>>;
    struct Providers {
        fail: bool,
        calls: Calls,
    }

    #[mod_api::__subscriptions]
    impl Hooks for Providers {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            for (name, discovery) in [("minecraft:barrel", true), ("minecraft:chest", false)] {
                let key = r.block_key(name);
                let calls = self.calls.clone();
                let fail = self.fail;
                r.block_capability_before_game(&ITEMS, &key, move |_, at, _, _, offer| {
                    let step = if discovery {
                        "discover barrel"
                    } else {
                        "discover chest"
                    };
                    calls.lock().unwrap().push((step, current_native_mod()));
                    if fail && discovery {
                        panic!("provider discovery panic");
                    }
                    offer.offer(|| {
                        calls
                            .lock()
                            .unwrap()
                            .push(("construct", current_native_mod()));
                        if fail {
                            panic!("provider construction panic");
                        }
                        Box::new(Storage {
                            probe: Probe {
                                trace: Arc::default(),
                                rules: Arc::default(),
                            },
                            at,
                        })
                    });
                });
            }
        }
    }

    let client = std::env::var(FLAG).unwrap() == "client";
    let calls: Calls = Arc::default();
    let mods: Mods = [("bad_provider", true), ("healthy_provider", false)]
        .into_iter()
        .map(|(id, fail)| {
            ModEntry::new(
                id,
                Box::new(Providers {
                    fail,
                    calls: calls.clone(),
                }),
            )
        })
        .collect::<Vec<_>>()
        .into();
    let exercise = move || {
        mod_platform::testing::with_mods(mods, || {
            macro_rules! check_discovery {
                ($game:expr) => {{
                    let game = &mut *$game;
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    place(game, (1, 64, 0), "minecraft:barrel");
                    place(game, (2, 64, 0), "minecraft:chest");

                    let outer = current_native_mod();
                    let query = mod_api::__private::enter("querying_mod", "test");
                    let bad = Some("bad_provider");
                    let healthy = Some("healthy_provider");
                    game.with_level(|_, world, ticks| {
                        let view = world.view(ticks);
                        assert_eq!(view.has_item_capability(DISCOVERY, None), Some(true));
                        assert_eq!(view.has_item_capability(DISCOVERY, None), Some(true));
                        assert_eq!(view.has_item_capability(CONSTRUCTION, None), Some(true));
                    });
                    assert_eq!(current_native_mod(), Some("querying_mod"));
                    assert_eq!(
                        std::mem::take(&mut *calls.lock().unwrap()),
                        [
                            ("discover barrel", bad),
                            ("discover barrel", healthy),
                            ("discover barrel", healthy),
                            ("discover chest", bad),
                        ],
                        "discovery poisons only one registration; presence never constructs",
                    );
                    (outer, query, bad, healthy)
                }};
            }
            if client {
                sunrise_client_gameplay::game::native_entity_refs::tests::with_game_role_client(
                    fixture_root(),
                    ExecutionRole::Client,
                    |game| {
                        let (pipe, _peer) = sunrise_protocol::connection::Pipe::pair();
                        game.connect_local(pipe, "ProviderOwner").unwrap();
                        game.client.remote.as_mut().unwrap().logged_in = true;
                        let (outer, query, _, _) = check_discovery!(game);
                        drop(query);
                        assert_eq!(current_native_mod(), outer);
                    },
                );
            } else {
                with_game_role(fixture_root(), ExecutionRole::IntegratedServer, |game| {
                    game.options.dedicated = Some(Vec::new());
                    let (outer, query, bad, healthy) = check_discovery!(game);
                    let level = game.level(game.dim.id()).unwrap();
                    for pos in [CONSTRUCTION, CONSTRUCTION, DISCOVERY] {
                        let capability = level.capability(game, &ITEMS, pos, None).unwrap();
                        assert_eq!(capability.owner(), "healthy_provider");
                        assert_eq!(current_native_mod(), Some("querying_mod"));
                    }
                    assert_eq!(
                        *calls.lock().unwrap(),
                        [
                            ("discover chest", bad),
                            ("construct", bad),
                            ("discover chest", healthy),
                            ("construct", healthy),
                            ("discover chest", healthy),
                            ("construct", healthy),
                            ("discover barrel", healthy),
                            ("construct", healthy),
                        ],
                    );
                    drop(query);
                    assert_eq!(current_native_mod(), outer);
                });
            }
        })
    };
    // This fixture has a fresh process. An uncontained panic on either game's thread
    // writes a report; on the integrated server it also marks the server fatal.
    let home = tempfile::tempdir().unwrap();
    crash::install(home.path().to_owned(), "client", || None);
    let name = if client {
        "main"
    } else {
        crash::INTEGRATED_SERVER_THREAD
    };
    std::thread::Builder::new()
        .name(name.into())
        .spawn(exercise)
        .unwrap()
        .join()
        .unwrap();
    assert!(!crash::server_crashed());
    assert!(!home.path().join("crash-reports").exists());
}

#[test]
#[ignore = "daily"]
fn aborting_a_pot_insert_restores_its_named_loot_sequence() {
    use sunrise_core::nbt::Tag;
    use sunrise_world::loot::{LootData, PackOverlay};

    default_fixture(
        "cut_layout::game::server_side::machines::native_items::storage_attempt_tests::aborting_a_pot_insert_restores_its_named_loot_sequence",
        |game| {
            const TABLE: &str = "test:pot";
            const SEQUENCE: &str = "test:pot_storage";
            let files = [(
                format!("loot_table/{TABLE}"),
                vec![
                    serde_json::to_vec(&serde_json::json!({
                        "random_sequence": SEQUENCE,
                        "pools": [{
                            "rolls": 1,
                            "entries": [{
                                "type": "minecraft:item",
                                "name": "minecraft:stick",
                                "modifier": {
                                    "type": "minecraft:set_count",
                                    "count": {"type": "minecraft:uniform", "min": 2, "max": 9}
                                }
                            }]
                        }]
                    }))
                    .unwrap(),
                ],
            )];
            let loot = LootData::load(
                &game.options.resources,
                &PackOverlay {
                    files: &files,
                    ..PackOverlay::default()
                },
                game.res.item_defaults(),
            )
            .unwrap();
            assert_eq!(loot.random_sequence(TABLE), Some(SEQUENCE));
            game.server.server_loot.0.set(Some(Arc::new(loot)));

            let pos = [4, 64, 0];
            place(game, (4, 64, 0), "minecraft:decorated_pot");
            let tag = Tag::compound([
                ("id", Tag::String("minecraft:decorated_pot".into())),
                ("x", Tag::Int(pos[0])),
                ("y", Tag::Int(pos[1])),
                ("z", Tag::Int(pos[2])),
                ("LootTable", Tag::String(TABLE.into())),
            ]);
            spill::put_saved_entity(&mut game.world, pos, tag.clone());
            // The named sequence already exists: this tests restoration of its actual
            // position, not merely removal of a newly created sequence.
            game.server.rand_sequences.get(SEQUENCE, game.level.seed);
            let before = game.server.rand_sequences.to_nbt();
            let level_before = game.world.random.state();
            let level = game.level(game.dim.id()).unwrap();
            let items = level.capability(game, &ITEMS, pos, None).unwrap();
            use sunrise_gameplay_common::game::api_values::TryIntoApi;
            let stick =
                ItemResource::of(&ItemStack::new("minecraft:stick", 1).try_into_api().unwrap());
            let insert = |game: &mut sunrise_gameplay::game::ServerGame<'_>,
                          tx: &TransactionContext| {
                assert_eq!(items.insert_at(game, tx, 0, &stick, 1), Ok(1));
                let amount = items.amount(game, 0);
                assert!(
                    (3..=10).contains(&amount),
                    "rolled loot plus insertion: {amount}"
                );
                let materialized = spill::saved_entity(&game.world, pos).unwrap();
                assert!(materialized.get("LootTable").is_none());
                assert_eq!(game.world.random.state(), level_before);
                let after = game.server.rand_sequences.to_nbt();
                assert_ne!(after, before);
                (materialized, after)
            };
            let tentative = game
                .storage_transaction(|game, _| {
                    let tentative = game
                        .storage_transaction(|game, tx| Ok::<_, ()>(insert(game, tx)))
                        .unwrap();
                    Err::<(), _>(tentative)
                })
                .unwrap_err();
            assert_eq!(spill::saved_entity(&game.world, pos), Some(tag));
            assert_eq!(game.server.rand_sequences.to_nbt(), before);
            assert_eq!(game.world.random.state(), level_before);

            assert_eq!(
                game.storage_transaction(|game, tx| Ok::<_, ()>(insert(game, tx))),
                Ok(tentative)
            );
        },
    );
}

#[test]
#[ignore = "daily"]
fn hopper_respects_a_mod_container_only_when_empty_rule() {
    default_fixture(
        "cut_layout::game::server_side::machines::native_items::storage_attempt_tests::hopper_respects_a_mod_container_only_when_empty_rule",
        |game| {
            place(game, HOPPER, "minecraft:hopper");
            hopper(
                game,
                vec![
                    Some(ItemStack::new("minecraft:stone", 1)),
                    None,
                    None,
                    None,
                    None,
                ],
            );
            storage(
                game,
                BELOW,
                vec![Some(ItemStack::new("minecraft:stone", 1)), None],
            );
            let barrel = game
                .pack
                .assets
                .registry
                .default_state("minecraft:barrel")
                .unwrap();
            game.world.block_entities.set_mod_containers(
                [(
                    barrel.0,
                    Arc::new(ModContainer {
                        slots: 2,
                        faces: std::array::from_fn(|_| vec![0, 1]),
                        inputs: vec![0, 1],
                        outputs: vec![0, 1],
                        insert_rules: vec![ModContainerInsertRule::OnlyWhenEmpty],
                        ..Default::default()
                    }),
                )]
                .into_iter()
                .collect(),
            );

            game.world.game_time = 1;
            game.tick_hopper(HOPPER);
            assert_eq!(count(game, BELOW, 0), 1);
            assert_eq!(count(game, BELOW, 1), 1);
            assert_eq!(count(game, HOPPER, 0), 0);
            assert_eq!(
                game.world.block_entities.get(&BELOW).unwrap().slots()[1]
                    .as_ref()
                    .unwrap()
                    .item
                    .as_ref(),
                "minecraft:stone"
            );

            hopper(
                game,
                vec![
                    Some(ItemStack::new("minecraft:gold_ingot", 1)),
                    None,
                    None,
                    None,
                    None,
                ],
            );
            game.world
                .block_entities
                .get_mut(&BELOW)
                .unwrap()
                .slots_mut()[0] = None;
            game.server.hoppers.get_mut(&HOPPER).unwrap().cooldown = 0;
            game.world.game_time = 2;
            game.tick_hopper(HOPPER);
            assert_eq!(count(game, BELOW, 0), 1);
            assert_eq!(
                game.world.block_entities.get(&BELOW).unwrap().slots()[0]
                    .as_ref()
                    .unwrap()
                    .item
                    .as_ref(),
                "minecraft:gold_ingot"
            );
        },
    );
}

fn storage(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    pos: Pos,
    items: Vec<Option<ItemStack>>,
) {
    place(game, pos, "minecraft:barrel");
    let mut entity = ModBlockEntity::new("storage_attempts:machine".into(), Default::default());
    entity.items = items;
    entity.data = vec![0];
    game.world
        .block_entities
        .insert(pos, BlockEntity::Mod(Box::new(entity)));
}
fn hopper(game: &mut sunrise_gameplay::game::ServerGame<'_>, items: Vec<Option<ItemStack>>) {
    game.world
        .block_entities
        .insert(HOPPER, BlockEntity::Chest(items));
}
fn blocked_hopper(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    hopper(
        game,
        vec![
            Some(ItemStack::new("minecraft:stone", 63)),
            Some(ItemStack::new("minecraft:stone", 64)),
            Some(ItemStack::new("minecraft:stone", 64)),
            Some(ItemStack::new("minecraft:stone", 64)),
            Some(ItemStack::new("minecraft:stone", 64)),
        ],
    );
}
fn trace(probe: &Probe) -> Vec<String> {
    std::mem::take(&mut *probe.trace.lock().unwrap())
}
fn count(game: &sunrise_gameplay::game::ServerGame<'_>, pos: Pos, slot: usize) -> u32 {
    game.world.block_entities.get(&pos).unwrap().slots()[slot]
        .as_ref()
        .map_or(0, |s| s.count)
}

#[test]
#[ignore = "daily"]
fn blocked_pull_ticks_extract_abort_restore_without_hopper_mark_or_cooldown() {
    fixture(
        "cut_layout::game::server_side::machines::native_items::storage_attempt_tests::blocked_pull_ticks_extract_abort_restore_without_hopper_mark_or_cooldown",
        |game, _, probe| {
            blocked_hopper(game);
            place(game, BELOW, "minecraft:bedrock");
            storage(
                game,
                ABOVE,
                vec![
                    Some(ItemStack::new("minecraft:diamond", 1)),
                    Some(ItemStack::new("minecraft:gold_ingot", 1)),
                ],
            );
            probe.rules.lock().unwrap().notify = true;
            for time in [1, 2] {
                game.world.game_time = time;
                game.tick_hopper(HOPPER);
                assert_eq!(
                    trace(&probe),
                    [
                        "face:Down",
                        "open",
                        "extract:0",
                        "open",
                        "notice:1:Open",
                        "commit",
                        "abort",
                        "restore:8,65,8:0",
                        "open",
                        "extract:1",
                        "open",
                        "notice:1:Open",
                        "commit",
                        "abort",
                        "restore:8,65,8:1"
                    ]
                );
                assert_eq!(game.hopper_cooldown(HOPPER), 0);
                assert_eq!((count(game, ABOVE, 0), count(game, ABOVE, 1)), (1, 1));
                assert_eq!(count(game, HOPPER, 0), 63);
            }
        },
    );
}

#[test]
#[ignore = "daily"]
fn blocked_push_ticks_share_one_abort_and_restore_refused_source_slots() {
    fixture(
        "cut_layout::game::server_side::machines::native_items::storage_attempt_tests::blocked_push_ticks_share_one_abort_and_restore_refused_source_slots",
        |game, _, probe| {
            hopper(
                game,
                vec![
                    Some(ItemStack::new("minecraft:diamond", 1)),
                    Some(ItemStack::new("minecraft:gold_ingot", 1)),
                    None,
                    None,
                    None,
                ],
            );
            storage(game, BELOW, vec![None; 2]);
            place(game, ABOVE, "minecraft:bedrock");
            probe.rules.lock().unwrap().notify = true;
            for time in [1, 2] {
                game.world.game_time = time;
                game.tick_hopper(HOPPER);
                let mut seen = trace(&probe);
                let mut restored = seen.split_off(11);
                restored.sort();
                assert_eq!(
                    seen,
                    [
                        "face:Up",
                        "open",
                        "insert:minecraft:diamond",
                        "open",
                        "notice:0:Open",
                        "commit",
                        "insert:minecraft:gold_ingot",
                        "open",
                        "notice:0:Open",
                        "commit",
                        "abort"
                    ]
                );
                assert_eq!(restored, ["restore:8,64,8:0", "restore:8,64,8:1"]);
                assert_eq!(game.hopper_cooldown(HOPPER), 0);
                assert_eq!((count(game, HOPPER, 0), count(game, HOPPER, 1)), (1, 1));
            }
            assert_eq!(
                probe.rules.lock().unwrap().source_seen,
                [(0, 1), (1, 0), (0, 1), (1, 0)]
            );
            probe.rules.lock().unwrap().accept = Some("minecraft:gold_ingot");
            game.tick_hopper(HOPPER);
            assert_eq!(
                trace(&probe),
                [
                    "face:Up",
                    "open",
                    "insert:minecraft:diamond",
                    "open",
                    "notice:0:Open",
                    "commit",
                    "insert:minecraft:gold_ingot",
                    "open",
                    "notice:1:Open",
                    "commit",
                    "commit",
                    "open",
                    "mark:hopper:Open",
                    "commit"
                ]
            );
            assert_eq!((count(game, HOPPER, 0), count(game, HOPPER, 1)), (1, 0));
            assert_eq!(count(game, BELOW, 0), 1);
            assert_eq!(game.hopper_cooldown(HOPPER), 8);
        },
    );
}

#[test]
#[ignore = "daily"]
fn push_then_pull_commit_in_one_tick_with_only_the_ordered_hopper_marks() {
    fixture(
        "cut_layout::game::server_side::machines::native_items::storage_attempt_tests::push_then_pull_commit_in_one_tick_with_only_the_ordered_hopper_marks",
        |game, _, probe| {
            hopper(game, vec![Some(ItemStack::new("minecraft:stone", 64)); 5]);
            storage(game, BELOW, vec![None; 2]);
            storage(
                game,
                ABOVE,
                vec![Some(ItemStack::new("minecraft:stone", 1)), None],
            );
            probe.rules.lock().unwrap().accept = Some("minecraft:stone");
            game.tick_hopper(HOPPER);
            assert_eq!(
                trace(&probe),
                [
                    "face:Up",
                    "open",
                    "insert:minecraft:stone",
                    "commit",
                    "face:Down",
                    "open",
                    "extract:0",
                    "open",
                    "mark:hopper:Open",
                    "commit",
                    "commit",
                    "open",
                    "mark:hopper:Open",
                    "commit"
                ]
            );
            assert_eq!(game.hopper_cooldown(HOPPER), 8);
            assert_eq!(
                (
                    count(game, HOPPER, 0),
                    count(game, BELOW, 0),
                    count(game, ABOVE, 0)
                ),
                (64, 1, 0)
            );
            for time in 1..8 {
                game.world.game_time = time;
                game.tick_hopper(HOPPER);
            }
            assert!(trace(&probe).is_empty());
            assert_eq!(game.hopper_cooldown(HOPPER), 1);
            game.tick_hopper(HOPPER);
            assert_eq!(game.hopper_cooldown(HOPPER), 8);
            assert_eq!(
                trace(&probe),
                [
                    "face:Up",
                    "open",
                    "insert:minecraft:stone",
                    "commit",
                    "face:Down",
                    "open",
                    "mark:hopper:Open",
                    "commit"
                ]
            );
        },
    );
}

#[test]
#[ignore = "daily"]
fn pull_chooses_the_first_slot_after_extract_and_not_an_existing_stack() {
    fixture(
        "cut_layout::game::server_side::machines::native_items::storage_attempt_tests::pull_chooses_the_first_slot_after_extract_and_not_an_existing_stack",
        |game, _, probe| {
            blocked_hopper(game);
            place(game, BELOW, "minecraft:bedrock");
            storage(
                game,
                ABOVE,
                vec![Some(ItemStack::new("minecraft:diamond", 1)), None],
            );
            probe.rules.lock().unwrap().release_first = true;
            game.tick_hopper(HOPPER);
            assert_eq!(
                trace(&probe),
                [
                    "face:Down",
                    "open",
                    "extract:0",
                    "open",
                    "mark:hopper:Open",
                    "commit",
                    "commit",
                    "open",
                    "mark:hopper:RootClosing",
                    "commit",
                    "open",
                    "mark:hopper:Open",
                    "commit"
                ]
            );
            assert_eq!(count(game, HOPPER, 0), 1);
            assert_eq!(
                game.world.block_entities.get(&HOPPER).unwrap().slots()[0]
                    .as_ref()
                    .unwrap()
                    .item
                    .as_ref(),
                "minecraft:diamond"
            );
            // A lower empty index wins over a compatible stack at a later index.
            hopper(
                game,
                vec![
                    None,
                    Some(ItemStack::new("minecraft:diamond", 1)),
                    None,
                    None,
                    None,
                ],
            );
            storage(
                game,
                ABOVE,
                vec![Some(ItemStack::new("minecraft:diamond", 1)), None],
            );
            probe.rules.lock().unwrap().release_first = false;
            game.server.hoppers.get_mut(&HOPPER).unwrap().cooldown = 0;
            game.tick_hopper(HOPPER);
            assert_eq!((count(game, HOPPER, 0), count(game, HOPPER, 1)), (1, 1));
        },
    );
}

#[test]
#[ignore = "daily"]
fn full_storage_and_fully_stacked_hopper_are_the_only_capacity_shortcuts() {
    fixture(
        "cut_layout::game::server_side::machines::native_items::storage_attempt_tests::full_storage_and_fully_stacked_hopper_are_the_only_capacity_shortcuts",
        |game, _, probe| {
            hopper(game, vec![Some(ItemStack::new("minecraft:stone", 64)); 5]);
            storage(
                game,
                BELOW,
                vec![Some(ItemStack::new("minecraft:diamond", 64)); 2],
            );
            storage(
                game,
                ABOVE,
                vec![Some(ItemStack::new("minecraft:diamond", 1)), None],
            );
            game.tick_hopper(HOPPER);
            assert_eq!(trace(&probe), ["face:Up"]);
            assert_eq!(game.hopper_cooldown(HOPPER), 0);

            // An empty index with no capacity is full too, so there is still no push.
            probe.rules.lock().unwrap().zero_capacity_slot = Some(1);
            game.world
                .block_entities
                .get_mut(&BELOW)
                .unwrap()
                .slots_mut()[1] = None;
            probe.trace.lock().unwrap().clear();
            game.tick_hopper(HOPPER);
            assert_eq!(trace(&probe), ["face:Up"]);
            assert_eq!(game.hopper_cooldown(HOPPER), 0);
        },
    );
}

#[test]
#[ignore = "daily"]
fn dropper_commits_refused_insertion_and_its_storage_notifications() {
    fixture(
        "cut_layout::game::server_side::machines::native_items::storage_attempt_tests::dropper_commits_refused_insertion_and_its_storage_notifications",
        |game, _, probe| {
            place(game, HOPPER, "minecraft:dropper");
            let state = game.world.block(8, 64, 8);
            let down = game.with_props(state, &[("facing", "down")]).unwrap();
            game.world.set(8, 64, 8, down).unwrap();
            hopper(
                game,
                vec![
                    Some(ItemStack::new("minecraft:diamond", 2)),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                ],
            );
            storage(game, BELOW, vec![None; 2]);
            {
                let mut rules = probe.rules.lock().unwrap();
                rules.notify = true;
                rules.write_on_refuse = true;
            }
            let state = game.world.block(8, 64, 8);
            game.dispense_from(BlockPos::new(8, 64, 8), state).unwrap();
            assert_eq!(
                trace(&probe),
                [
                    "face:Up",
                    "open",
                    "insert:minecraft:diamond",
                    "open",
                    "notice:0:Open",
                    "commit",
                    "commit"
                ]
            );
            assert_eq!((count(game, HOPPER, 0), count(game, BELOW, 0)), (2, 0));
            let level = game.level("minecraft:overworld").unwrap();
            let entity = game.block_entity(level, [8, 63, 8]).unwrap();
            assert_eq!(entity.data_slots(game), Some(vec![1]));
            probe.rules.lock().unwrap().accept = Some("minecraft:diamond");
            game.dispense_from(BlockPos::new(8, 64, 8), state).unwrap();
            assert_eq!(
                trace(&probe),
                [
                    "face:Up",
                    "open",
                    "insert:minecraft:diamond",
                    "open",
                    "notice:1:Open",
                    "commit",
                    "commit"
                ]
            );
            assert_eq!((count(game, HOPPER, 0), count(game, BELOW, 0)), (1, 1));
        },
    );
}

#[test]
#[ignore = "daily"]
fn vanilla_container_provider_and_entity_never_ask_their_storage() {
    fixture(
        "cut_layout::game::server_side::machines::native_items::storage_attempt_tests::vanilla_container_provider_and_entity_never_ask_their_storage",
        |game, _, probe| {
            use {
                sunrise_gameplay_common::game::minecarts::CartKind,
                sunrise_gameplay_common::game::minecarts::Minecart,
            };
            for target in ["minecraft:chest", "minecraft:composter", "minecraft:air"] {
                hopper(game, vec![None; 5]);
                game.server.hoppers.clear();
                place(game, ABOVE, target);
                game.world.block_entities.remove(&ABOVE);
                game.carts.clear();
                match target {
                    "minecraft:chest" => {
                        game.world.block_entities.insert(
                            ABOVE,
                            BlockEntity::Chest(vec![
                                Some(ItemStack::new("minecraft:stone", 1));
                                27
                            ]),
                        );
                    }
                    "minecraft:composter" => {
                        let state = game.world.block(8, 65, 8);
                        let full = game.with_props(state, &[("level", "8")]).unwrap();
                        game.world.set(8, 65, 8, full).unwrap();
                    }
                    _ => {
                        let mut cart =
                            Minecart::new(700, CartKind::Chest, Vec3::new(8.5, 65.0, 8.5));
                        cart.items[0] = Some(ItemStack::new("minecraft:stone", 1));
                        game.carts.push(cart);
                    }
                }
                game.tick_hopper(HOPPER);
                assert_eq!(count(game, HOPPER, 0), 1, "{target}");
                let mut expected = vec!["open", "mark:hopper:Open", "commit"];
                if target == "minecraft:chest" {
                    // The source's notice runs CommitMover without emitting a trace message.
                    expected.extend(["open", "commit"]);
                }
                expected.extend(["open", "mark:hopper:Open", "commit"]);
                assert_eq!(trace(&probe), expected, "{target}");
            }
        },
    );
}

#[test]
#[ignore = "daily"]
fn failed_block_pull_stops_but_minecart_falls_back_to_item_entities() {
    fixture(
        "cut_layout::game::server_side::machines::native_items::storage_attempt_tests::failed_block_pull_stops_but_minecart_falls_back_to_item_entities",
        |game, _, probe| {
            use {
                sunrise_gameplay_common::game::minecarts::CartKind,
                sunrise_gameplay_common::game::minecarts::Minecart,
            };
            storage(
                game,
                ABOVE,
                vec![Some(ItemStack::new("minecraft:diamond", 1)), None],
            );
            probe.rules.lock().unwrap().refuse_extract = true;
            game.spawn_item(
                ItemStack::new("minecraft:stone", 1),
                [8.5, 64.5, 8.5],
                [0.0; 3],
                0,
            );
            game.tick_hopper(HOPPER);
            assert_eq!(game.dropped.len(), 1);
            assert_eq!(count(game, HOPPER, 0), 0);
            assert_eq!(trace(&probe), ["face:Down", "open", "extract:0", "abort"]);
            let mut cart = Minecart::new(701, CartKind::Hopper, Vec3::new(8.5, 64.0, 8.5));
            game.hopper_cart_suck(&mut cart);
            assert!(game.dropped.is_empty());
            assert_eq!(cart.items[0], Some(ItemStack::new("minecraft:stone", 1)));
            assert_eq!(trace(&probe), ["face:Down", "open", "extract:0", "abort"]);
        },
    );
}

#[test]
#[ignore = "daily"]
fn nontransactional_storage_loses_a_blocked_extraction_on_abort() {
    fixture(
        "cut_layout::game::server_side::machines::native_items::storage_attempt_tests::nontransactional_storage_loses_a_blocked_extraction_on_abort",
        |game, _, probe| {
            blocked_hopper(game);
            place(game, BELOW, "minecraft:bedrock");
            storage(game, ABOVE, vec![None; 2]);
            {
                let mut rules = probe.rules.lock().unwrap();
                rules.nontransactional = true;
                rules.external = vec![
                    Some(
                        mod_api::ItemStack::new(
                            mod_api::__private::item_key("minecraft:diamond").unwrap(),
                            1,
                        )
                        .unwrap(),
                    ),
                    None,
                ];
            }
            game.tick_hopper(HOPPER);
            assert_eq!(trace(&probe), ["face:Down", "open", "extract:0", "abort"]);
            assert!(probe.rules.lock().unwrap().external[0].is_none());
            assert_eq!(count(game, HOPPER, 0), 63);
            assert_eq!(game.hopper_cooldown(HOPPER), 0);
        },
    );
}

#[test]
#[ignore = "daily"]
fn extract_notifications_cross_signal_each_tick_and_comparator_ticks_every_second_tick() {
    fixture(
        "cut_layout::game::server_side::machines::native_items::storage_attempt_tests::extract_notifications_cross_signal_each_tick_and_comparator_ticks_every_second_tick",
        |game, _, probe| {
            use sunrise_core::ticks::TickTypeId;
            use sunrise_world::level::LevelAccess as _;
            // Real scheduled ticks require a lit, surrounded, entity-ready block-ticking chunk.
            Arc::make_mut(game.world.chunks.get_mut(&(0, 0)).unwrap()).lit = true;
            for x in -1..=1 {
                for z in -1..=1 {
                    if (x, z) != (0, 0) {
                        game.world
                            .chunks
                            .insert((x, z), Arc::new(Chunk::empty(x, z)));
                    }
                }
            }
            game.server
                .tickets
                .0
                .update(Some(sunrise_sim::ticket_levels::Sources {
                    forced: vec![(0, 0)],
                    ..Default::default()
                }));
            assert!(game.with_level(|_, level, _| level.is_ticking_chunk(ChunkPos::new(0, 0))));
            blocked_hopper(game);
            // A different item keeps the non-full hopper unable to merge the source stone.
            if let Some(stack) = game
                .world
                .block_entities
                .get_mut(&HOPPER)
                .unwrap()
                .slots_mut()[0]
                .as_mut()
            {
                stack.item = "minecraft:dirt".into();
            }
            place(game, BELOW, "minecraft:bedrock");
            storage(
                game,
                ABOVE,
                vec![Some(ItemStack::new("minecraft:stone", 64)), None],
            );
            probe.rules.lock().unwrap().slots = Some(1);
            let above_state = game.world.block(8, 65, 8).0;
            game.world.block_entities.set_mod_containers(
                [(
                    above_state,
                    Arc::new(ModContainer {
                        slots: 1,
                        inputs: vec![0],
                        outputs: vec![0],
                        faces: std::array::from_fn(|_| vec![0]),
                        insert_rules: Vec::new(),
                        menu: None,
                    }),
                )]
                .into_iter()
                .collect(),
            );
            let comparator = BlockPos::new(9, 65, 8);
            place(game, (9, 64, 8), "minecraft:stone");
            place(game, (9, 65, 8), "minecraft:comparator");
            let state = game.world.block(9, 65, 8);
            let state = game.with_props(state, &[("facing", "west")]).unwrap();
            game.world.set(9, 65, 8, state).unwrap();
            game.with_level(|b, level, ticks| b.scheduled_block_tick(level, ticks, comparator));
            assert_eq!(game.block_ticks.comparator_outputs[&comparator], 15);
            let kind = TickTypeId(u32::from(game.res.behaviours().block(state).0));
            probe.rules.lock().unwrap().notify = true;
            trace(&probe);
            let mut comparator_ticks = Vec::new();
            for time in 1..=6 {
                game.world.game_time = time;
                game.with_level(|b, level, ticks| b.tick_scheduled(level, ticks));
                let before = trace(&probe);
                if before.iter().any(|event| event == "comparator") {
                    comparator_ticks.push(time);
                }
                game.tick_hopper(HOPPER);
                assert_eq!(
                    trace(&probe),
                    [
                        "face:Down",
                        "open",
                        "extract:0",
                        "open",
                        "notice:63:Open",
                        "commit",
                        "abort",
                        "restore:8,65,8:0"
                    ]
                );
                assert_eq!(game.hopper_cooldown(HOPPER), 0);
                assert_eq!(count(game, ABOVE, 0), 64);
                assert_eq!(
                    game.res
                        .facts()
                        .get(game.world.block(9, 65, 8))
                        .name
                        .as_ref(),
                    "minecraft:comparator"
                );
                assert!(
                    game.block_ticks.blocks.has_scheduled(comparator, kind),
                    "extract notice keeps real comparator work queued"
                );
            }
            assert_eq!(comparator_ticks, [3, 5]);
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_blocked_pull_slot_aborts_before_the_next_slot_commits() {
    fixture(
        "cut_layout::game::server_side::machines::native_items::storage_attempt_tests::a_blocked_pull_slot_aborts_before_the_next_slot_commits",
        |game, _, probe| {
            blocked_hopper(game);
            place(game, BELOW, "minecraft:bedrock");
            storage(
                game,
                ABOVE,
                vec![
                    Some(ItemStack::new("minecraft:diamond", 1)),
                    Some(ItemStack::new("minecraft:stone", 1)),
                ],
            );
            game.tick_hopper(HOPPER);
            assert_eq!(
                trace(&probe),
                [
                    "face:Down",
                    "open",
                    "extract:0",
                    "abort",
                    "restore:8,65,8:0",
                    "open",
                    "extract:1",
                    "open",
                    "mark:hopper:Open",
                    "commit",
                    "commit",
                    "open",
                    "mark:hopper:Open",
                    "commit"
                ]
            );
            assert_eq!(
                (
                    count(game, ABOVE, 0),
                    count(game, ABOVE, 1),
                    count(game, HOPPER, 0)
                ),
                (1, 0, 64)
            );
            assert_eq!(game.hopper_cooldown(HOPPER), 8);
        },
    );
}

#[test]
#[ignore = "daily"]
fn pushes_use_only_the_opposite_face_for_each_hopper_facing() {
    fixture(
        "cut_layout::game::server_side::machines::native_items::storage_attempt_tests::pushes_use_only_the_opposite_face_for_each_hopper_facing",
        |game, _, probe| {
            place(game, ABOVE, "minecraft:bedrock");
            let base = game.world.block(8, 64, 8);
            for (facing, target, expected) in [
                ("down", BELOW, "face:Up"),
                ("north", (8, 64, 7), "face:South"),
                ("south", (8, 64, 9), "face:North"),
                ("west", (7, 64, 8), "face:East"),
                ("east", (9, 64, 8), "face:West"),
            ] {
                let state = game.with_props(base, &[("facing", facing)]).unwrap();
                game.world.set(8, 64, 8, state).unwrap();
                hopper(
                    game,
                    vec![
                        Some(ItemStack::new("minecraft:stone", 1)),
                        None,
                        None,
                        None,
                        None,
                    ],
                );
                game.server.hoppers.clear();
                storage(game, target, vec![None; 2]);
                probe.rules.lock().unwrap().accept = Some("minecraft:stone");
                game.tick_hopper(HOPPER);
                assert_eq!(
                    trace(&probe),
                    [
                        expected,
                        "open",
                        "insert:minecraft:stone",
                        "commit",
                        "open",
                        "mark:hopper:Open",
                        "commit"
                    ]
                );
                assert_eq!(count(game, target, 0), 1);
            }
        },
    );
}

#[test]
#[ignore = "daily"]
fn refused_push_callback_journals_source_without_losing_the_restored_item() {
    fixture(
        "cut_layout::game::server_side::machines::native_items::storage_attempt_tests::refused_push_callback_journals_source_without_losing_the_restored_item",
        |game, level, probe| {
            hopper(
                game,
                vec![
                    Some(ItemStack::new("minecraft:diamond", 1)),
                    None,
                    None,
                    None,
                    None,
                ],
            );
            storage(game, BELOW, vec![None; 2]);
            place(game, ABOVE, "minecraft:bedrock");
            {
                let mut rules = probe.rules.lock().unwrap();
                rules.notify = true;
                rules.rename_source_on_notice = true;
            }
            game.tick_hopper(HOPPER);
            assert_eq!(
                trace(&probe),
                [
                    "face:Up",
                    "open",
                    "insert:minecraft:diamond",
                    "open",
                    "notice:0:Open",
                    "commit",
                    "abort",
                    "restore:8,64,8:0"
                ]
            );
            assert_eq!(count(game, HOPPER, 0), 1);
            assert_eq!(count(game, BELOW, 0), 0);
            assert_eq!(game.hopper_cooldown(HOPPER), 0);
            assert_eq!(
                game.block_entity(level, [8, 64, 8]).unwrap().name(game),
                None
            );
        },
    );
}

#[test]
#[ignore = "daily"]
fn dropper_removes_accepted_source_before_a_real_root_commit_callback() {
    fixture(
        "cut_layout::game::server_side::machines::native_items::storage_attempt_tests::dropper_removes_accepted_source_before_a_real_root_commit_callback",
        |game, _, probe| {
            place(game, HOPPER, "minecraft:dropper");
            let state = game.world.block(8, 64, 8);
            let state = game.with_props(state, &[("facing", "down")]).unwrap();
            game.world.set(8, 64, 8, state).unwrap();
            hopper(
                game,
                vec![
                    Some(ItemStack::new("minecraft:diamond", 1)),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                ],
            );
            storage(game, BELOW, vec![None; 2]);
            for pos in [SINK, OTHER] {
                let key = (pos[0], pos[1], pos[2]);
                place(game, key, "minecraft:chest");
                game.world
                    .block_entities
                    .insert(key, BlockEntity::Chest(vec![None; 27]));
            }
            {
                let mut rules = probe.rules.lock().unwrap();
                rules.accept = Some("minecraft:diamond");
                rules.forward_to_vanilla = true;
            }
            game.dispense_from(BlockPos::new(8, 64, 8), state).unwrap();
            // The forwarded slot-less insert owns a nested journal. Its commit must not
            // notify the sink before the root closes with the reserved source already gone.
            assert_eq!(
                trace(&probe),
                [
                    "face:Up",
                    "open",
                    "insert:minecraft:diamond",
                    "open",
                    "commit",
                    "commit",
                    "open",
                    "sink-source:0:RootClosing",
                    "commit"
                ]
            );
            assert_eq!(count(game, HOPPER, 0), 0);
            assert_eq!(count(game, (10, 63, 8), 0), 1);
            assert_eq!(count(game, (11, 63, 8), 0), 0);
        },
    );
}

#[test]
#[ignore = "daily"]
fn dropper_reserves_source_across_writable_insertion_notices() {
    fixture(
        "cut_layout::game::server_side::machines::native_items::storage_attempt_tests::dropper_reserves_source_across_writable_insertion_notices",
        |game, _, probe| {
            for replacement in [false, true] {
                place(game, HOPPER, "minecraft:dropper");
                let state = game.world.block(8, 64, 8);
                let state = game.with_props(state, &[("facing", "down")]).unwrap();
                game.world.set(8, 64, 8, state).unwrap();
                let mut slots = vec![None; 9];
                slots[0] = Some(ItemStack::new("minecraft:diamond", 1));
                hopper(game, slots);
                storage(game, BELOW, vec![None; 2]);
                place(game, (11, 63, 8), "minecraft:chest");
                game.world.block_entities.insert(
                    (11, 63, 8),
                    BlockEntity::Chest({
                        let mut slots = vec![None; 27];
                        if replacement {
                            slots[0] = Some(ItemStack::new("minecraft:stick", 1));
                        }
                        slots
                    }),
                );
                {
                    let mut rules = probe.rules.lock().unwrap();
                    rules.accept = Some("minecraft:diamond");
                    rules.notify = true;
                    rules.move_source_on_notice = true;
                    rules.replace_source_on_notice = replacement;
                }
                game.dispense_from(BlockPos::new(8, 64, 8), state).unwrap();
                assert_eq!(count(game, BELOW, 0), 1);
                assert_eq!(count(game, (11, 63, 8), 0), 0);
                assert_eq!(count(game, HOPPER, 0), u32::from(replacement));
                if replacement {
                    let view = game.containers().view(HOPPER, false).unwrap();
                    assert_eq!(
                        game.containers().get(&view, 0).unwrap().item.as_ref(),
                        "minecraft:stick"
                    );
                }
            }
        },
    );
}

#[test]
#[ignore = "daily"]
fn refused_dropper_replacement_rolls_back_without_losing_either_item() {
    fixture(
        "cut_layout::game::server_side::machines::native_items::storage_attempt_tests::refused_dropper_replacement_rolls_back_without_losing_either_item",
        |game, _, probe| {
            place(game, HOPPER, "minecraft:dropper");
            let state = game.world.block(8, 64, 8);
            let state = game.with_props(state, &[("facing", "down")]).unwrap();
            game.world.set(8, 64, 8, state).unwrap();
            let mut slots = vec![None; 9];
            slots[0] = Some(ItemStack::new("minecraft:diamond", 1));
            hopper(game, slots);
            storage(game, BELOW, vec![None; 2]);
            place(game, (11, 63, 8), "minecraft:chest");
            let mut slots = vec![None; 27];
            slots[0] = Some(ItemStack::new("minecraft:stick", 1));
            game.world
                .block_entities
                .insert((11, 63, 8), BlockEntity::Chest(slots));
            {
                let mut rules = probe.rules.lock().unwrap();
                rules.notify = true;
                rules.move_source_on_notice = true;
                rules.replace_source_on_notice = true;
            }
            game.dispense_from(BlockPos::new(8, 64, 8), state).unwrap();
            assert_eq!(count(game, HOPPER, 0), 1);
            assert_eq!(count(game, BELOW, 0), 0);
            assert_eq!(count(game, (11, 63, 8), 0), 1);
            let source = game.containers().view(HOPPER, false).unwrap();
            let replacement = game.containers().view((11, 63, 8), false).unwrap();
            assert_eq!(
                game.containers().get(&source, 0).unwrap().item.as_ref(),
                "minecraft:diamond"
            );
            assert_eq!(
                game.containers()
                    .get(&replacement, 0)
                    .unwrap()
                    .item
                    .as_ref(),
                "minecraft:stick"
            );
            assert!(trace(&probe).iter().any(|entry| entry == "abort"));
        },
    );
}

#[test]
#[ignore = "daily"]
fn dropper_routes_saved_pot_directly_even_with_a_storage_provider() {
    fixture(
        "cut_layout::game::server_side::machines::native_items::storage_attempt_tests::dropper_routes_saved_pot_directly_even_with_a_storage_provider",
        |game, _, probe| {
            use sunrise_core::nbt::Tag;
            let pot = [9, 64, 8];
            place(game, HOPPER, "minecraft:dropper");
            let state = game.world.block(8, 64, 8);
            let state = game.with_props(state, &[("facing", "east")]).unwrap();
            game.world.set(8, 64, 8, state).unwrap();
            hopper(
                game,
                vec![
                    Some(ItemStack::new("minecraft:stone", 2)),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                ],
            );
            place(game, (9, 64, 8), "minecraft:decorated_pot");
            spill::put_saved_entity(
                &mut game.world,
                pot,
                Tag::compound([
                    ("id", Tag::String("minecraft:decorated_pot".into())),
                    ("x", Tag::Int(9)),
                    ("y", Tag::Int(64)),
                    ("z", Tag::Int(8)),
                    (
                        "item",
                        sunrise_sim::save::item_to_nbt(
                            &ItemStack::new("minecraft:stone", 63),
                            &Default::default(),
                        ),
                    ),
                ]),
            );
            let dropped = game.dropped.len();
            game.dispense_from(BlockPos::new(8, 64, 8), state).unwrap();
            let saved = spill::saved_entity(&game.world, pot).unwrap();
            assert_eq!(
                sunrise_sim::save::item_from_nbt(saved.get("item").unwrap())
                    .unwrap()
                    .count,
                64
            );
            assert_eq!(count(game, HOPPER, 0), 1);
            assert_eq!(game.dropped.len(), dropped);
            assert!(
                trace(&probe).is_empty(),
                "the pot's provider was never asked"
            );
            game.dispense_from(BlockPos::new(8, 64, 8), state).unwrap();
            assert_eq!(
                count(game, HOPPER, 0),
                1,
                "a full pot refuses on its direct path"
            );
            assert_eq!(game.dropped.len(), dropped);
            assert!(trace(&probe).is_empty());
        },
    );
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_block_entity_read::NativeBlockEntityReadGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
