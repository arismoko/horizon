//! Part state (#781) on real block entities: one value each, kept with its block entity
//! through ticks, transactions, replacement, saving and unloading.
use std::{
    panic::AssertUnwindSafe,
    sync::{Arc, Mutex},
};
use sunrise_gameplay_common::game::GameGameExt1;

use mod_api::{
    AttachmentDescriptor, AttachmentKey, BlockEntityPart, BlockEntityPartState, BlockEntityRef,
    BlockEntityTickEvent, BlockPart, Hooks, HostTransaction, PartStateError, Registrar, ServerHost,
    edit_part_state, read_state,
};
use sunrise_core::nbt::{Compound, Tag};
use sunrise_level::{
    chunk::Chunk,
    containers::{BlockEntity, ModBlockEntity},
};
use sunrise_mod_runtime::attachments::{
    AttachmentCodec, AttachmentError, AttachmentType, Attachments, Holder,
};

use sunrise_gameplay_common::game::Dim;
use sunrise_gameplay_common::world_biomes;

const PROGRESS_CODEC: AttachmentCodec<i32> = AttachmentCodec {
    encode: |n| {
        if *n == i32::MIN {
            return Err(AttachmentError::Encode);
        }
        Ok([("n".into(), Tag::Int(*n))].into())
    },
    decode: |saved: &Compound| match saved.get(&"n".into()) {
        Some(Tag::Int(n)) => Ok(*n),
        _ => Err(AttachmentError::Decode),
    },
};

static PROGRESS: AttachmentType<i32> = AttachmentType {
    namespace: "part_state",
    name: "progress",
    default: || 0,
    codec: Some(PROGRESS_CODEC),
    copy_on_death: false,
    sync_codec: None,
};

/// Counts its block entity's ticks in its state.
#[derive(Clone)]
struct Press {
    progress: AttachmentKey<i32>,
}
#[mod_api::sunrise_part]
impl BlockPart for Press {}
#[mod_api::sunrise_part]
impl BlockEntityPart for Press {
    fn block_entity_tick(&self, event: &mut BlockEntityTickEvent<'_>) {
        let (level, pos, entity) = event
            .subject
            .with_view(|entity| (entity.level(), entity.position(), entity.handle()))
            .unwrap();
        let host = event.subject.host();
        assert_eq!(
            host.block_entity(level, pos),
            Some(entity),
            "resident during its tick"
        );
        self.edit_state(host, entity, |_, _tx, ticks| *ticks += 1)
            .unwrap();
    }
}
impl BlockEntityPartState for Press {
    type Data = i32;
    fn state_key(&self) -> &AttachmentKey<i32> {
        &self.progress
    }
}

#[derive(Clone)]
struct StateKeys {
    press: Press,
    /// Kept for the session only: no codec.
    scratch: AttachmentKey<i32>,
}

struct StateMod {
    keys: Arc<Mutex<Option<StateKeys>>>,
}
#[mod_api::__subscriptions]
impl Hooks for StateMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let progress = r
            .register_attachment_type(
                AttachmentDescriptor::new("part_state:progress", || 0_i32)
                    .serialize(PROGRESS_CODEC),
            )
            .unwrap();
        let scratch = r
            .register_attachment_type(AttachmentDescriptor::new("part_state:scratch", || 0_i32))
            .unwrap();
        let press = Press { progress };
        *self.keys.lock().unwrap() = Some(StateKeys {
            press: press.clone(),
            scratch,
        });
        let kind = r
            .block_entity_type("press")
            .blocks(&["part_state:press"])
            .build();
        r.block_entity_part_with_state(&kind, press).unwrap();
        let press = r.block_key("part_state:press");
        r.block_container(
            &press,
            mod_api::ModContainer {
                slots: 1,
                ..Default::default()
            },
        );
    }
    fn blocks(&self, _: &mod_api::RegistrationContext, e: &mut mod_api::BlocksEvent<'_>) {
        e.register(mod_api::Block::new("part_state:press", "minecraft:barrel").with_item(false))
            .unwrap();
    }
}

/// Run the test `name` with `StateMod` installed, in a fresh process: its content root
/// freezes the process's block entity types, which another test may already have frozen.
fn with_state_mod(
    name: &str,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, &StateKeys),
) {
    if super::native_entity_refs::tests::run_fresh_cases(
        name,
        "SUNRISE_PART_STATE_GAME_CHILD",
        &["valid"],
    ) {
        return;
    }
    let keys = Arc::new(Mutex::new(None));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "part_state",
        Box::new(StateMod { keys: keys.clone() }),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_api::__private::mark_game_thread();
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let root =
            sunrise_gameplay_common::game::mods::content_root(&base, &Default::default()).unwrap();
        super::native_entity_refs::tests::with_game_at(root.prepared().to_owned(), |game| {
            let state_keys = keys.lock().unwrap().clone().expect("StateMod registered");
            game.options.dedicated = Some(Vec::new());
            game.native_level_opened(Dim::Overworld);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            test(game, &state_keys);
        });
    });
}

/// A fresh press at `pos` in the level the game is in, and its handle.
fn press(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    pos: (i32, i32, i32),
) -> BlockEntityRef {
    let block = game
        .pack
        .assets
        .registry
        .default_state("part_state:press")
        .unwrap();
    game.world.set(pos.0, pos.1, pos.2, block).unwrap();
    let entity = ModBlockEntity::new("part_state:press".into(), Default::default());
    game.world
        .block_entities
        .insert(pos, BlockEntity::Mod(Box::new(entity)));
    let level = game.level(game.dim.id()).unwrap();
    game.block_entity(level, [pos.0, pos.1, pos.2]).unwrap()
}

fn press_with_item_slot(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    pos: (i32, i32, i32),
) -> BlockEntityRef {
    // The save fixture never runs the block-entity tick that binds these declarations.
    let containers = game.native_containers();
    game.world.block_entities.set_mod_containers(containers);
    press(game, pos)
}

fn stored(
    press: &Press,
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    entity: BlockEntityRef,
) -> Option<i32> {
    press
        .state_existing(game, entity, |value| value.cloned())
        .unwrap()
}

#[test]
#[ignore = "daily"]
fn part_state_is_per_block_entity_through_ticks_transactions_and_replacement() {
    const NAME: &str = "cut_layout::game::server_side::block_entity_part_state_tests::part_state_is_per_block_entity_through_ticks_transactions_and_replacement";
    with_state_mod(NAME, |game, keys| {
        let press_part = &keys.press;
        use sunrise_gameplay_common::game::api_values::TryIntoApi;
        use sunrise_items::items::ItemStack;

        let (a, b) = (
            press_with_item_slot(game, (1, 64, 1)),
            press(game, (2, 64, 1)),
        );
        // Reads insert nothing.
        assert_eq!(press_part.state(game, a, Clone::clone), Ok(0));
        assert_eq!(stored(press_part, game, a), None);
        game.with_level(|_, level, _| {
            assert_eq!(press_part.state_in(level, a, Clone::clone), Ok(0));
            assert_eq!(
                press_part.state_existing_in(level, a, |value| value.cloned()),
                Ok(None)
            );
        });
        assert_eq!(
            press_part
                .state_key()
                .has(game, Holder::BlockEntity([1, 64, 1])),
            Ok(false)
        );
        assert_eq!(press_part.edit_state(game, a, |_, _tx, n| *n = 7), Ok(()));
        assert_eq!(
            (stored(press_part, game, a), stored(press_part, game, b)),
            (Some(7), None)
        );
        game.with_level(|_, level, _| {
            assert_eq!(press_part.state_in(level, a, Clone::clone), Ok(7));
            assert_eq!(
                press_part.state_existing_in(level, a, |value| value.cloned()),
                Ok(Some(7))
            );
        });
        let mut chunk = game.world.chunks.remove(&(0, 0)).unwrap();
        game.with_level(|_, level, _| {
            assert_eq!(
                press_part.state_in(level, a, Clone::clone),
                Err(PartStateError::Stale)
            );
            assert_eq!(
                press_part.state_existing_in(level, a, |value| value.cloned()),
                Err(PartStateError::Stale)
            );
        });
        game.install_live_features();
        let rain_pos = [4, 64, 4];
        let plains = world_biomes!(game).id("minecraft:plains");
        let column = Arc::make_mut(&mut chunk);
        let bottom = column.min_y() >> 2;
        // This column is open air with full skylight. Seed every quart cell that
        // BiomeManager.getBiome can select at rain_pos, independent of the zoom seed.
        for x in 0..=1 {
            for y in 15..=16 {
                for z in 0..=1 {
                    column.set_biome_cell(x, y - bottom, z, plains);
                }
            }
        }
        game.world.chunks.insert((0, 0), chunk);
        let level = game.level(game.dim.id()).unwrap();
        game.world.weather.rain_level = (1.0, 1.0);
        let server_rain = game
            .level_is_raining_at(level, rain_pos)
            .expect("server weather read");
        assert!(server_rain, "the fixture position is exposed to rain");
        let dedicated = game.options.dedicated.take();
        let role = game.options.execution_role;
        game.options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
        let client_view = sunrise_gameplay_common::game::native_level_signals::ReadLevel::new(
            game,
            level,
            &game.world,
        );
        assert_eq!(press_part.state_in(&client_view, a, Clone::clone), Ok(0));
        assert_eq!(
            press_part.state_existing_in(&client_view, a, |value| value.cloned()),
            Ok(None)
        );
        assert_eq!(
            mod_api::LevelHost::is_raining_at(&client_view, rain_pos),
            server_rain,
            "a client Part reads weather from its bound world"
        );
        game.options.dedicated = dedicated;
        game.options.execution_role = role;

        // Its own tick reaches it while it is out of the world, under the same handle.
        game.tick_mod_block_entity((1, 64, 1));
        game.tick_mod_block_entity((1, 64, 1));
        assert_eq!(stored(press_part, game, a), Some(9));
        assert_eq!(press_part.state(game, b, Clone::clone), Ok(0));

        // An inner edit commits into the outer transaction, which then aborts: the value and
        // the chunk's unsaved mark go back, so a chunk that was clean is clean again.
        game.world.modified.clear();
        let aborted: Result<(), mod_api::TransactionError<()>> = (game as &mut dyn ServerHost)
            .transaction(|host, _| {
                let stack = ItemStack::new("minecraft:stone", 1).try_into_api().unwrap();
                assert_eq!(
                    press_part.edit_state(host, a, |host, tx, n| {
                        *n = 11;
                        a.set_item_slot(host, tx, 0, Some(stack))
                    }),
                    Ok(Ok(true)),
                );
                assert_eq!(
                    press_part.state(host, a, Clone::clone),
                    Ok(11),
                    "later steps see it"
                );
                assert!(
                    !sunrise_sim::join_seq::visibility::passes_settled(),
                    "no save while it is tentative"
                );
                Err(())
            });
        assert_eq!(aborted, Err(mod_api::TransactionError::Aborted(())));
        assert_eq!(stored(press_part, game, a), Some(9));
        let Some(BlockEntity::Mod(restored)) = game.world.block_entities.get(&(1, 64, 1)) else {
            panic!("the holder must survive the abort");
        };
        assert!(restored.items.iter().all(Option::is_none));
        assert!(
            !game.world.modified.contains(&(0, 0)),
            "clean after the abort"
        );

        // A panic inside an edit leaves no default behind.
        let host: &mut dyn ServerHost = game;
        let panicked = std::panic::catch_unwind(AssertUnwindSafe(|| {
            press_part.edit_state(host, b, |_, _tx, n| {
                *n = 5;
                panic!("edit fails");
            })
        }));
        assert!(panicked.is_err());
        assert!(game.transactions.is_empty());
        assert_eq!(stored(press_part, game, b), None);
        assert_eq!(press_part.edit_state(game, b, |_, _tx, n| *n = 8), Ok(()));
        assert_eq!(stored(press_part, game, b), Some(8));

        // The same state can't be edited from inside its own edit; another key can.
        let inner = press_part.edit_state(game, a, |host, _tx, n| {
            *n = 10;
            (
                press_part.edit_state(host, a, |_, _tx, n| *n = 99),
                edit_part_state(host, a, &keys.scratch, |_, _tx, n| *n = 1),
            )
        });
        assert_eq!(inner, Ok((Err(PartStateError::Reentrant), Ok(()))));
        assert_eq!(stored(press_part, game, a), Some(10));
        assert_eq!(
            read_state(game, a, &keys.scratch, |value| value.cloned()),
            Ok(Some(1))
        );

        // A value replaced while the closure ran refuses the edit and keeps the old one.
        let changed = press_part.edit_state(game, a, |host, _tx, n| {
            *n = 12;
            press_part
                .state_key()
                .set(host, Holder::BlockEntity([1, 64, 1]), 50)
                .unwrap();
        });
        assert_eq!(changed, Err(PartStateError::Changed));
        assert_eq!(stored(press_part, game, a), Some(10));

        // Refusing the draft's encoding keeps its prior bytes and undoes nested writes.
        let saved = game
            .world
            .block_entities
            .attachments((1, 64, 1))
            .unwrap()
            .try_to_nbt()
            .unwrap();
        let refused = press_part.edit_state(game, a, |host, _tx, n| {
            keys.scratch
                .set(host, Holder::BlockEntity([1, 64, 1]), 8)
                .unwrap();
            *n = i32::MIN;
        });
        assert_eq!(
            refused,
            Err(PartStateError::Attachment {
                namespace: "part_state",
                name: "progress",
                error: AttachmentError::Encode,
            })
        );
        assert_eq!(
            read_state(game, a, &keys.scratch, |value| value.copied()),
            Ok(Some(1))
        );
        assert_eq!(
            game.world
                .block_entities
                .attachments((1, 64, 1))
                .unwrap()
                .try_to_nbt()
                .unwrap(),
            saved
        );

        // An undecodable saved value refuses before the closure runs and keeps its bytes.
        let mut bad = Compound::new();
        bad.insert("n".into(), Tag::String("ten".into()));
        let mut saved = Attachments::default();
        PROGRESS.set(&mut saved, Holder::Level, 0).unwrap();
        let mut raw = saved.try_to_nbt().unwrap();
        let Some(Tag::Compound(names)) = raw.get_mut(&"part_state".into()) else {
            unreachable!()
        };
        let Some(Tag::Compound(entry)) = names.get_mut(&"progress".into()) else {
            unreachable!()
        };
        entry.insert("value".into(), Tag::Compound(bad));
        game.world
            .block_entities
            .restore_attachments((2, 64, 1), Attachments::from_nbt(raw.clone()));
        let decode = PartStateError::Attachment {
            namespace: "part_state",
            name: "progress",
            error: AttachmentError::Decode,
        };
        assert_eq!(press_part.state(game, b, Clone::clone), Err(decode.clone()));
        let mut ran = false;
        game.world.modified.clear();
        assert_eq!(
            press_part.edit_state(game, b, |_, _tx, _| ran = true),
            Err(decode)
        );
        assert!(!ran);
        assert!(
            !game.world.modified.contains(&(0, 0)),
            "a refused edit marks nothing"
        );
        assert_eq!(
            game.world
                .block_entities
                .attachments((2, 64, 1))
                .unwrap()
                .try_to_nbt()
                .unwrap(),
            raw
        );

        // A block entity of the same place in another level has its own value, and a
        // handle answers only in its own level.
        game.enter_level(Dim::Nether).unwrap();
        assert_eq!(
            press_part.state(game, a, Clone::clone),
            Err(PartStateError::OtherLevel)
        );
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let nether = press(game, (1, 64, 1));
        assert_eq!(press_part.state(game, nether, Clone::clone), Ok(0));
        assert_eq!(
            press_part.edit_state(game, nether, |_, _tx, n| *n = 3),
            Ok(())
        );
        game.enter_level(Dim::Overworld).unwrap();
        assert_eq!(stored(press_part, game, a), Some(10));

        // A replacement at the same place is a new block entity with no value.
        game.world.block_entities.remove(&(1, 64, 1));
        let replacement = press(game, (1, 64, 1));
        assert_ne!(replacement, a);
        assert_eq!(
            press_part.state(game, a, Clone::clone),
            Err(PartStateError::Stale)
        );
        assert_eq!(
            press_part.edit_state(game, a, |_, _tx, n| *n = 1),
            Err(PartStateError::Stale)
        );
        assert_eq!(stored(press_part, game, replacement), None);
    });
}

#[test]
#[ignore = "daily"]
fn part_state_saves_with_its_chunk_and_reloads_under_a_fresh_handle() {
    const NAME: &str = "cut_layout::game::server_side::block_entity_part_state_tests::part_state_saves_with_its_chunk_and_reloads_under_a_fresh_handle";
    with_state_mod(NAME, |game, keys| {
        let press_part = &keys.press;
        // A column far from the player, kept by a forced ticket while it is edited.
        let key = (80, 80);
        let pos = (1283, 100, 1283);
        // The test game starts with streaming paused, which never unloads.
        game.server.streaming.paused = false;
        assert!(game.server.streaming.set_forced(key.0, key.1, true));
        game.world
            .chunks
            .insert(key, Arc::new(Chunk::empty(key.0, key.1)));
        game.side_update_world();
        let entity = press(game, pos);
        game.world.modified.clear();
        assert_eq!(
            press_part.edit_state(game, entity, |_, _tx, n| *n = 7),
            Ok(())
        );
        assert!(
            game.world.modified.contains(&key),
            "an edit marks its chunk for the unload's save"
        );
        assert_eq!(
            edit_part_state(game, entity, &keys.scratch, |_, _tx, n| *n = 4),
            Ok(())
        );

        // The ticket goes and the residency owner (`update_world`) unloads the column: it
        // saves it, then drops its block entities and their live bindings. Its sweep runs every
        // eighth update.
        assert!(game.server.streaming.set_forced(key.0, key.1, false));
        for _ in 0..9 {
            game.side_update_world();
        }
        assert!(!game.world.chunks.contains_key(&key), "unloaded");
        assert!(game.world.block_entities.attachments(pos).is_none());
        assert_eq!(
            press_part.state(game, entity, Clone::clone),
            Err(PartStateError::Stale)
        );

        // Loaded back from the region file it was saved to, as streaming installs a column.
        game.save_level(true).unwrap();
        let mut chunk = game
            .server
            .save
            .0
            .load(key.0, key.1, game.pack.assets.registry, world_biomes!(game))
            .unwrap();
        game.world.install_chunk_block_entities(&mut chunk);
        game.world.chunks.insert(key, Arc::new(chunk));
        let level = game.level(game.dim.id()).unwrap();
        let reloaded = game.block_entity(level, [pos.0, pos.1, pos.2]).unwrap();
        assert_ne!(reloaded, entity);
        assert_eq!(
            press_part.state(game, entity, Clone::clone),
            Err(PartStateError::Stale)
        );
        assert_eq!(stored(press_part, game, reloaded), Some(7));
        assert_eq!(
            read_state(game, reloaded, &keys.scratch, |value| value.cloned()),
            Ok(None),
            "session only"
        );
    });
}

#[test]
#[ignore = "daily"]
fn item_slot_writes_save_but_menu_data_is_transient() {
    const NAME: &str = "cut_layout::game::server_side::block_entity_part_state_tests::item_slot_writes_save_but_menu_data_is_transient";
    with_state_mod(NAME, |game, _| {
        use sunrise_gameplay_common::game::api_values::TryIntoApi;
        use sunrise_items::items::ItemStack;

        let entity = press_with_item_slot(game, (1, 64, 1));
        game.save_level(true).unwrap();
        assert!(game.world.modified.is_empty());

        assert_eq!(entity.set_data_slots(game, vec![23]), Ok(true));
        assert_eq!(entity.set_data_slot(game, 0, 42), Ok(true));
        assert_eq!(entity.data_slots(game), Some(vec![42]));
        assert!(
            !game.world.modified.contains(&(0, 0)),
            "transient menu data edits must not mark the chunk unsaved"
        );
        game.save_level(true).unwrap();
        assert!(game.world.modified.is_empty());

        // Both growing a lazy slot list and editing an existing slot must save
        // without an explicit notification, a ticker, or another dirty write.
        for count in [1, 2] {
            let item = ItemStack::new("minecraft:stone", count);
            let stack = item.clone().try_into_api().unwrap();
            assert_eq!(
                game.storage_transaction(|host, tx| entity.set_item_slot(host, tx, 0, Some(stack))),
                Ok(true)
            );
            game.save_level(true).unwrap();
            let chunk = game
                .server
                .save
                .0
                .load(0, 0, game.pack.assets.registry, world_biomes!(game))
                .unwrap();
            let [((1, 64, 1), BlockEntity::Mod(reloaded))] = chunk.block_entities.as_slice() else {
                panic!("mod holder missing from saved chunk")
            };
            assert_eq!(reloaded.items, [Some(item)]);
            assert!(
                reloaded.data.is_empty(),
                "menu data resets even on a real save"
            );
        }
    });
}

#[test]
#[ignore = "daily"]
fn an_item_write_commits_in_its_own_level_despite_an_unrelated_transaction() {
    const NAME: &str = "cut_layout::game::server_side::block_entity_part_state_tests::an_item_write_commits_in_its_own_level_despite_an_unrelated_transaction";
    with_state_mod(NAME, |game, _| {
        let chunk = (0, 0);
        let pos = (1, 64, 1);
        game.enter_level(Dim::Nether).unwrap();
        game.world
            .chunks
            .insert(chunk, Arc::new(Chunk::empty(chunk.0, chunk.1)));
        let entity = press_with_item_slot(game, pos);
        game.save_level(true).unwrap();
        assert!(!game.world.modified.contains(&chunk));
        game.enter_level(Dim::Overworld).unwrap();
        assert!(!game.world.modified.contains(&chunk));

        game.storage_scope(|game, _| {
            // Like `execute in`: the Nether has no open journal to join.
            game.enter_level(Dim::Nether).unwrap();
            let stone = mod_api::ItemStack::new(
                mod_api::__private::item_key("minecraft:stone").unwrap(),
                1,
            )
            .unwrap();
            assert_eq!(
                game.storage_transaction(|host, child| entity.set_item_slot(
                    host,
                    child,
                    0,
                    Some(stone)
                )),
                Ok(true)
            );
            assert!(
                game.world.modified.contains(&chunk),
                "committed in the Nether"
            );
            game.enter_level(Dim::Overworld).unwrap();
            ((), false)
        });
        assert!(game.transactions.is_empty());
        assert!(!game.world.modified.contains(&chunk));
        assert!(
            game.server
                .levels
                .0
                .parked_world(Dim::Nether)
                .unwrap()
                .modified
                .contains(&chunk)
        );

        game.save_all_levels(true).unwrap();
        game.enter_level(Dim::Nether).unwrap();
        let reloaded = game
            .server
            .save
            .0
            .load(
                chunk.0,
                chunk.1,
                game.pack.assets.registry,
                world_biomes!(game),
            )
            .unwrap();
        let [(_, BlockEntity::Mod(saved))] = reloaded.block_entities.as_slice() else {
            panic!("mod holder missing from saved Nether chunk")
        };
        assert_eq!(
            saved.items,
            [Some(sunrise_items::items::ItemStack::new(
                "minecraft:stone",
                1
            ))]
        );
    });
}

/// What the attachment index answers for `entity`'s `progress`, without the handle check.
fn indexed(
    game: &sunrise_gameplay::game::ServerGame<'_>,
    entity: BlockEntityRef,
    progress: &AttachmentKey<i32>,
) -> Option<i32> {
    use mod_api::AttachmentQueries;
    let host: &dyn mod_api::Host = game;
    host.query(progress)
        .unwrap()
        .get(
            game.attachment_frame_borrow(),
            mod_api::AttachmentHolder::BlockEntity(entity),
            |value| value.cloned(),
        )
        .ok()
        .flatten()
}

#[test]
#[ignore = "daily"]
fn a_removal_retires_its_state_only_at_the_root_commit() {
    const NAME: &str = "cut_layout::game::server_side::block_entity_part_state_tests::a_removal_retires_its_state_only_at_the_root_commit";
    with_state_mod(NAME, |game, keys| {
        let press_part = &keys.press;
        let progress = press_part.state_key();
        let pos = (1, 64, 1);
        let a = press(game, pos);
        assert_eq!(press_part.edit_state(game, a, |_, _tx, n| *n = 7), Ok(()));

        // Removed and replaced inside a transaction that then aborts: the handle is stale
        // while it is open, but the index keeps the map bound; the abort brings the very
        // binding back, and the tentative block entity's handle stays stale.
        let tentative = game.storage_scope(|game, _| {
            game.journal_remove_block_entity(pos);
            assert_eq!(
                press_part.state(game, a, Clone::clone),
                Err(PartStateError::Stale)
            );
            assert_eq!(
                indexed(game, a, progress),
                Some(7),
                "still bound until the root commits"
            );
            let tentative = press(game, pos);
            assert_eq!(
                press_part.edit_state(game, tentative, |_, _tx, n| *n = 5),
                Ok(())
            );
            (tentative, false)
        });
        assert_eq!(press_part.state(game, a, Clone::clone), Ok(7));
        assert_eq!(indexed(game, a, progress), Some(7));
        assert_eq!(
            press_part.state(game, tentative, Clone::clone),
            Err(PartStateError::Stale)
        );
        assert_eq!(indexed(game, tentative, progress), None);

        // A nested removal committed into its parent is still not published.
        game.storage_scope(|game, _| {
            game.storage_scope(|game, _| {
                game.journal_remove_block_entity(pos);
                ((), true)
            });
            assert_eq!(indexed(game, a, progress), Some(7));
            // The root commit retires it.
            ((), true)
        });
        assert_eq!(
            press_part.state(game, a, Clone::clone),
            Err(PartStateError::Stale)
        );
        assert_eq!(indexed(game, a, progress), None);
        assert!(game.world.block_entities.attachments(pos).is_none());
    });
}

#[test]
#[ignore = "daily"]
fn a_save_or_unload_during_a_transaction_writes_only_committed_state() {
    const NAME: &str = "cut_layout::game::server_side::block_entity_part_state_tests::a_save_or_unload_during_a_transaction_writes_only_committed_state";
    with_state_mod(NAME, |game, keys| {
        let press_part = &keys.press;
        let (key, pos) = ((80, 80), (1283, 100, 1283));
        game.server.streaming.paused = false;
        assert!(game.server.streaming.set_forced(key.0, key.1, true));
        game.world
            .chunks
            .insert(key, Arc::new(Chunk::empty(key.0, key.1)));
        game.side_update_world();
        let entity = press(game, pos);
        assert_eq!(
            press_part.edit_state(game, entity, |_, _tx, n| *n = 7),
            Ok(())
        );
        let reload = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
            let chunk = game
                .server
                .save
                .0
                .load(key.0, key.1, game.pack.assets.registry, world_biomes!(game))
                .unwrap();
            let saved = chunk.saved.as_ref().unwrap();
            let (_, map) = saved
                .block_entity_attachments
                .iter()
                .find(|(at, _)| *at == pos)
                .unwrap();
            PROGRESS.get_existing(&mut map.clone(), Holder::Level)
        };

        // A save asked for while an edit is tentative is refused; once it aborts, the save
        // writes the committed value.
        game.storage_scope(|game, _| {
            assert_eq!(
                press_part.edit_state(game, entity, |_, _tx, n| *n = 9),
                Ok(())
            );
            assert!(game.save_level(true).is_err(), "refused while tentative");
            ((), false)
        });
        game.save_level(true).unwrap();
        assert_eq!(reload(game), Ok(Some(7)));

        // An unload due while a transaction is open waits for it to close, then saves the
        // committed value.
        game.storage_scope(|game, _| {
            assert_eq!(
                press_part.edit_state(game, entity, |_, _tx, n| *n = 11),
                Ok(())
            );
            assert!(game.server.streaming.set_forced(key.0, key.1, false));
            for _ in 0..9 {
                game.side_update_world();
            }
            assert!(game.world.chunks.contains_key(&key), "kept while tentative");
            assert_eq!(press_part.state(game, entity, Clone::clone), Ok(11));
            ((), false)
        });
        game.side_update_world();
        assert!(!game.world.chunks.contains_key(&key), "unloaded");
        game.save_level(true).unwrap();
        assert_eq!(reload(game), Ok(Some(7)));
    });
}

#[test]
#[ignore = "daily"]
fn one_position_replaced_twice_in_an_aborted_transaction_gets_its_original_back() {
    const NAME: &str = "cut_layout::game::server_side::block_entity_part_state_tests::one_position_replaced_twice_in_an_aborted_transaction_gets_its_original_back";
    with_state_mod(NAME, |game, keys| {
        let press_part = &keys.press;
        let progress = press_part.state_key();
        let pos = (1, 64, 1);
        let original = press(game, pos);
        assert_eq!(
            press_part.edit_state(game, original, |_, _tx, n| *n = 7),
            Ok(())
        );
        let tentative = game.storage_scope(|game, _| {
            let mut tentative = Vec::new();
            for value in [20, 30] {
                game.journal_remove_block_entity(pos);
                let replacement = press(game, pos);
                assert_eq!(
                    press_part.edit_state(game, replacement, |_, _tx, n| *n = value),
                    Ok(())
                );
                tentative.push(replacement);
            }
            assert_eq!(
                indexed(game, original, progress),
                Some(7),
                "bound until the root commits"
            );
            (tentative, false)
        });
        assert_eq!(press_part.state(game, original, Clone::clone), Ok(7));
        assert_eq!(indexed(game, original, progress), Some(7));
        for handle in tentative {
            assert_eq!(
                press_part.state(game, handle, Clone::clone),
                Err(PartStateError::Stale)
            );
            assert_eq!(indexed(game, handle, progress), None);
        }
    });
}

#[path = "block_entity_part_state_tests/fixture.rs"]
mod fixture;

struct NativeFixture;
#[mod_api::__subscriptions]
impl Hooks for NativeFixture {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        fixture::register(r);
    }
    fn blocks(&self, _: &mod_api::RegistrationContext, e: &mut mod_api::BlocksEvent<'_>) {
        fixture::blocks(e);
    }
}

/// The fixture's value at `pos`, read from its saved form.
fn saved_value(
    game: &sunrise_gameplay::game::ServerGame<'_>,
    pos: (i32, i32, i32),
    name: &str,
) -> Option<i32> {
    let saved = game
        .world
        .block_entities
        .attachments(pos)?
        .try_to_nbt()
        .unwrap();
    let Some(Tag::Compound(names)) = saved.get(&"part_state_fixture".into()) else {
        return None;
    };
    let Some(Tag::Compound(entry)) = names.get(&name.into()) else {
        return None;
    };
    match entry.get(&"value".into()) {
        Some(Tag::Compound(value)) => match value.get(&"n".into()) {
            Some(Tag::Int(n)) => Some(*n),
            _ => None,
        },
        _ => None,
    }
}

/// Ticks the fixture's block entity at `pos` once in `mode`, its only data slot (data slots are
/// not saved, so a reloaded one has none).
fn tick_in(game: &mut sunrise_gameplay::game::ServerGame<'_>, pos: (i32, i32, i32), mode: i32) {
    let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get_mut(&pos) else {
        panic!("no fixture block entity at {pos:?}")
    };
    entity.data = vec![mode];
    game.tick_mod_block_entity(pos);
    assert!(game.transactions.is_empty());
}

/// The Part counts ticks across an aborted edit, a refused recursive edit and a chunk
/// unload/reload. An undecodable value refuses the edit without changing its saved bytes.
#[test]
#[ignore = "daily"]
fn part_state_counts_ticks_across_chunk_reload() {
    const NAME: &str = "cut_layout::game::server_side::block_entity_part_state_tests::part_state_counts_ticks_across_chunk_reload";
    if super::native_entity_refs::tests::run_fresh_cases(
        NAME,
        "SUNRISE_PART_STATE_CHILD",
        &["game"],
    ) {
        return;
    }
    sunrise_gameplay::game::native_entity_refs::tests::run_with_game_stack(|| {
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "part_state_fixture",
            Box::new(NativeFixture),
        )]
        .into();
        mod_platform::testing::with_mods(mods.clone(), || {
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            mod_api::__private::mark_game_thread();
            let base =
                std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
            let root =
                sunrise_gameplay_common::game::mods::content_root(&base, &Default::default())
                    .unwrap();
            super::native_entity_refs::tests::with_game_at(root.prepared().to_owned(), |game| {
                game.options.dedicated = Some(Vec::new());
                let (key, pos) = ((80, 80), (1283, 100, 1283));
                game.server.streaming.paused = false;
                assert!(game.server.streaming.set_forced(key.0, key.1, true));
                game.world
                    .chunks
                    .insert(key, Arc::new(Chunk::empty(key.0, key.1)));
                game.side_update_world();
                let block = game
                    .pack
                    .assets
                    .registry
                    .default_state("part_state_fixture:part_state")
                    .unwrap();
                game.world.set(pos.0, pos.1, pos.2, block).unwrap();
                let entity = ModBlockEntity::new(
                    "part_state_fixture:part_state".into(),
                    sunrise_level::containers::ModBlockEntityFields {
                        data: vec![0],
                        ..Default::default()
                    },
                );
                game.world
                    .block_entities
                    .insert(pos, BlockEntity::Mod(Box::new(entity)));
                for _ in 0..3 {
                    tick_in(game, pos, 0);
                }
                assert_eq!(saved_value(game, pos, "part_state_count"), Some(3));

                // Unloaded by the owner, then loaded back from the region file.
                assert!(game.server.streaming.set_forced(key.0, key.1, false));
                for _ in 0..9 {
                    game.side_update_world();
                }
                assert!(!game.world.chunks.contains_key(&key), "unloaded");
                assert!(game.world.block_entities.attachments(pos).is_none());
                game.save_level(true).unwrap();
                let mut chunk = game
                    .server
                    .save
                    .0
                    .load(key.0, key.1, game.pack.assets.registry, world_biomes!(game))
                    .unwrap();
                game.world.install_chunk_block_entities(&mut chunk);
                game.world.chunks.insert(key, Arc::new(chunk));
                tick_in(game, pos, 0);
                assert_eq!(
                    saved_value(game, pos, "part_state_count"),
                    Some(4),
                    "reloaded"
                );

                // An undecodable saved count: refused, bytes kept, and the Part continues.
                let mut raw = game
                    .world
                    .block_entities
                    .attachments(pos)
                    .unwrap()
                    .try_to_nbt()
                    .unwrap();
                let Some(Tag::Compound(names)) = raw.get_mut(&"part_state_fixture".into()) else {
                    unreachable!()
                };
                let Some(Tag::Compound(count)) = names.get_mut(&"part_state_count".into()) else {
                    unreachable!()
                };
                count.insert(
                    "value".into(),
                    Tag::Compound([("n".into(), Tag::String("five".into()))].into()),
                );
                game.world
                    .block_entities
                    .restore_attachments(pos, Attachments::from_nbt(raw.clone()));
                tick_in(game, pos, 1);
                let after = game
                    .world
                    .block_entities
                    .attachments(pos)
                    .unwrap()
                    .try_to_nbt()
                    .unwrap();
                let Some(Tag::Compound(names)) = after.get(&"part_state_fixture".into()) else {
                    unreachable!()
                };
                let Some(Tag::Compound(before)) = raw.get(&"part_state_fixture".into()) else {
                    unreachable!()
                };
                assert_eq!(
                    names.get(&"part_state_count".into()),
                    before.get(&"part_state_count".into()),
                    "its bytes are kept"
                );
                assert_eq!(saved_value(game, pos, "part_state_mark"), Some(1));
            });
        });
    });
}

use sunrise_gameplay_common::game::side_ops::events::EventsSideGameExt as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_block_entity_read::NativeBlockEntityReadGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
