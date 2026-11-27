//! `light_emission` asked of a real part on a content block, through the game's own asks: only a
//! block declaring `dynamic_light` is asked, its answer reads the position, `None` keeps the
//! state's light, an answer above 15 is stored as 15, a bulk replacement and a reload of the
//! chunk keep or ask again as the cells' states say, and `check_light` and a server's light
//! update ask again.
#![cfg(feature = "client")]
use mod_api::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU8, Ordering},
};
#[cfg(test)]
use sunrise_gameplay_common::game::light_emission::LightEmissionGameExt1 as _;
use sunrise_level::chunk::Chunk;
use {
    crate::game::server_side::native_entity_refs::tests::with_game_role_client,
    sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game_role,
    sunrise_gameplay_common::game::execution_role::ExecutionRole,
};
use {
    sunrise_gameplay_common::game::Dim, sunrise_gameplay_common::game::LevelDimension,
    sunrise_gameplay_common::game::SideGame,
};

const LAMP: &str = "lights:lamp";
const PLAIN: &str = "lights:plain";
const FIXED: &str = "lights:fixed";
const PROXY: &str = "lights:proxy";
const SILENT: &str = "lights:silent";

type LightAsk = (LevelDimension, [i32; 3]);

/// Answers by height, as a crystal brighter deeper down would, and records where it is asked;
/// at y 70 it answers what `glow` holds, as an input that changes without a block change.
#[derive(Clone, Default)]
struct Depth {
    asked: Arc<Mutex<Vec<LightAsk>>>,
    glow: Arc<AtomicU8>,
}
impl Depth {
    fn take(&self) -> Vec<[i32; 3]> {
        self.take_asks().into_iter().map(|(_, pos)| pos).collect()
    }

    fn take_asks(&self) -> Vec<LightAsk> {
        std::mem::take(&mut *self.asked.lock().unwrap())
    }
}

#[mod_api::sunrise_part]
impl BlockPart for Depth {
    fn light_emission(&self, _: &dyn Host, event: &mut LightEmissionEvent) -> Option<u8> {
        self.asked
            .lock()
            .unwrap()
            .push((event.dimension, [event.pos.x, event.pos.y, event.pos.z]));
        match event.pos.y {
            y if y < 0 => Some(9),
            64 => Some(12),
            65 => None,
            66 => Some(200),
            70 => Some(self.glow.load(Ordering::Relaxed)),
            _ => Some(255),
        }
    }
}

struct Fixed;

#[mod_api::sunrise_part]
impl BlockPart for Fixed {
    fn light_emission(&self, _: &dyn Host, _: &mut LightEmissionEvent) -> Option<u8> {
        Some(5)
    }
}

struct Proxy(Arc<AtomicU8>);

#[mod_api::sunrise_part]
impl BlockPart for Proxy {
    fn light_emission(&self, host: &dyn Host, event: &mut LightEmissionEvent) -> Option<u8> {
        self.0.fetch_add(1, Ordering::Relaxed);
        let level = host.level(event.dimension.key().id()).unwrap();
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        if event.pos.y == 66 {
            assert_eq!(
                level.get_light_emission(host, [pos[0], pos[1] + 1, pos[2]], event.state),
                Err(LevelReadError::Unavailable)
            );
            None
        } else {
            let lamp = host.block_state_id(LAMP).unwrap();
            level.get_light_emission(host, pos, lamp).ok()
        }
    }
}

/// A dynamic lamp and a static block share a Part; another dynamic block has no Part.
/// The fixed and proxy providers exercise different answers and nested dependencies.
struct Lights(Depth, Arc<AtomicU8>);

#[mod_api::__subscriptions]
impl Hooks for Lights {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        for id in [LAMP, PLAIN] {
            let key = r.block_key(id);
            r.block_part(key, self.0.clone());
        }
        let fixed = r.block_key(FIXED);
        r.block_part(fixed, Fixed);
        let proxy = r.block_key(PROXY);
        r.block_part(proxy, Proxy(self.1.clone()));
    }

    fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
        for id in [LAMP, PLAIN, FIXED, PROXY, SILENT] {
            let mut block = mod_api::Block::new(id, "minecraft:glass");
            block.properties.light = Some(4);
            block.properties.dynamic_light = (id != PLAIN).then_some(true);
            event.register(block).unwrap();
        }
    }
}

/// The two blocks' default states, and air.
struct States {
    lamp: sunrise_world::registry::BlockStateId,
    plain: sunrise_world::registry::BlockStateId,
    air: sunrise_world::registry::BlockStateId,
}

/// Runs `test` in a fresh process on a server game whose content root holds the two blocks, with
/// the overworld's chunk (0, 0) empty and no light edits queued.
fn with_lights(
    name: &str,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, LevelRef, States, Depth),
) {
    with_lights_server(name, |game, level, states, depth, _| {
        test(game, level, states, depth);
    });
}

fn prepare_lights_root() -> (std::path::PathBuf, Depth, Arc<AtomicU8>) {
    let depth = Depth::default();
    let proxy_calls = Arc::new(AtomicU8::new(0));
    let mods: Mods = vec![ModEntry::new(
        "lights",
        Box::new(Lights(depth.clone(), proxy_calls.clone())),
    )]
    .into();
    mod_platform::__private::install(mods.clone());
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let root = sunrise_gameplay_common::game::mods::content_root(&base, &Default::default())
        .unwrap()
        .prepared()
        .to_path_buf();
    (root, depth, proxy_calls)
}

fn init_lights_world<'g, G: SideGame<'g>>(game: &mut G) -> States {
    let registry = &game.pack.assets.registry;
    let states = States {
        lamp: registry.default_state(LAMP).unwrap(),
        plain: registry.default_state(PLAIN).unwrap(),
        air: registry.default_state("minecraft:air").unwrap(),
    };
    let light = &game.pack.chunk_context.light;
    assert!(light.is_dynamic(states.lamp.0));
    assert!(
        !light.is_dynamic(states.plain.0),
        "the declaration gates the ask"
    );
    assert_eq!(light.emission(states.lamp.0), 4);
    let mut chunk = Chunk::empty(0, 0);
    chunk.lit = true;
    game.world.chunks.insert((0, 0), Arc::new(chunk));
    game.world.light_edits.clear();
    states
}

fn with_lights_server(
    name: &str,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, LevelRef, States, Depth, &AtomicU8),
) {
    const CHILD: &str = "SUNRISE_DYNAMIC_LIGHT_CHILD";
    if run_fresh_cases(name, CHILD, &["server"]) {
        return;
    }
    let (root, depth, proxy_calls) = prepare_lights_root();
    with_game_role(root, ExecutionRole::IntegratedServer, |game| {
        let states = init_lights_world(game);
        let level = game.level("minecraft:overworld").unwrap();
        test(game, level, states, depth, &proxy_calls);
    });
}

fn with_lights_sides(
    name: &str,
    server_test: impl FnOnce(
        &mut sunrise_gameplay::game::ServerGame<'_>,
        LevelRef,
        States,
        Depth,
        &AtomicU8,
    ),
    client_test: impl FnOnce(
        &mut sunrise_client_gameplay::game::ClientGame<'_>,
        LevelRef,
        States,
        Depth,
        &AtomicU8,
    ),
) {
    const CHILD: &str = "SUNRISE_DYNAMIC_LIGHT_CHILD";
    if run_fresh_cases(name, CHILD, &["server", "client"]) {
        return;
    }
    let client = std::env::var(CHILD).unwrap() == "client";
    let (root, depth, proxy_calls) = prepare_lights_root();
    if client {
        with_game_role_client(root, ExecutionRole::Client, |game| {
            let _server = logged_client(game);
            game.native_lifecycle.begin_open(None);
            // The remote dimension's key, not the client's legacy Dim field, names its level.
            let remote = game.client.remote.as_mut().unwrap();
            remote.level = "lights:remote".into();
            remote.level_key = mod_api::__private::dimension_key(&remote.level);
            let states = init_lights_world(game);
            let level = game.level("lights:remote").unwrap();
            client_test(game, level, states, depth, &proxy_calls);
        });
    } else {
        with_game_role(root, ExecutionRole::IntegratedServer, |game| {
            let states = init_lights_world(game);
            let level = game.level("minecraft:overworld").unwrap();
            server_test(game, level, states, depth, &proxy_calls);
        });
    }
}

#[test]
#[ignore = "daily"]
fn light_query_uses_the_supplied_state_and_its_declared_default() {
    with_lights(
        "cut_layout::game::dynamic_light_emission_tests::light_query_uses_the_supplied_state_and_its_declared_default",
        |game, level, States { plain, .. }, depth| {
            game.world.set(1, 65, 2, plain).unwrap();
            let host: &dyn Host = game;
            let static_state = host.block_state_id(PLAIN).unwrap();
            let dynamic = host.block_state_id(LAMP).unwrap();
            let without_part = host.block_state_id(SILENT).unwrap();
            let air = host.block_state_id("minecraft:air").unwrap();
            assert_eq!(level.get_light_emission(host, [1, 65, 2], dynamic), Ok(4));
            assert_eq!(depth.take(), [[1, 65, 2]]);
            let unloaded = [200, 65, 200];
            assert_eq!(
                level.get_light_emission(host, unloaded, static_state),
                Ok(4)
            );
            assert_eq!(
                level.get_light_emission(host, unloaded, without_part),
                Ok(4)
            );
            assert_eq!(level.get_light_emission(host, unloaded, air), Ok(0));
            let invalid =
                mod_api::__private::block_state_from_ordinal(static_state.block(), u16::MAX);
            assert_eq!(
                level.get_light_emission(host, unloaded, invalid),
                Err(LevelReadError::Unavailable)
            );
            let stale =
                LevelRef::with_generation(level.epoch(), level.dimension(), level.generation() + 1);
            assert_eq!(
                stale.get_light_emission(host, [1, 65, 2], dynamic),
                Err(LevelReadError::Stale)
            );
            assert!(depth.take().is_empty());
            assert_eq!(game.world.chunks.len(), 1);
        },
    );
}

fn assert_live_answers<'g, G: SideGame<'g> + Host>(
    game: &mut G,
    level: LevelRef,
    depth: &Depth,
    expected_dimension: &str,
    ask_edited: impl FnOnce(&mut G),
    before_query: impl FnOnce(&mut G),
    after_query: impl FnOnce(&mut G),
) {
    let fixed = game.pack.assets.registry.default_state(FIXED).unwrap();
    let pos = [1, 70, 2];
    game.world
        .set_with_notification(1, 70, 2, fixed, false)
        .unwrap();
    ask_edited(game);
    game.world.light_edits.clear();
    assert_eq!(answer(game, 70), Some(5));
    assert_eq!(level.brightness(game, LightLayer::Block, pos), Some(0));
    let before = game.world.chunks.get(&(0, 0)).unwrap().clone();
    before_query(game);
    let host: &dyn Host = game;
    let lamp = host.block_state_id(LAMP).unwrap();
    let unloaded = [200, 70, 200];
    for requested in [0, 200] {
        depth.glow.store(requested, Ordering::Relaxed);
        assert_eq!(level.get_light_emission(host, pos, lamp), Ok(requested));
        assert_eq!(
            level.get_light_emission(host, unloaded, lamp),
            Ok(requested)
        );
        let asks = depth.take_asks();
        assert_eq!(asks.len(), 2);
        assert_eq!(asks[0].1, pos);
        assert_eq!(asks[1].1, unloaded);
        assert!(
            asks.iter()
                .all(|(dimension, _)| dimension.key().id() == expected_dimension)
        );
    }
    after_query(game);
    assert_eq!(game.world.chunks.len(), 1);
    assert!(Arc::ptr_eq(
        &before,
        game.world.chunks.get(&(0, 0)).unwrap()
    ));
    assert_eq!(answer(game, 70), Some(5));
    assert!(game.world.light_checks.is_empty());
    assert!(game.world.light_edits.is_empty());
    assert_eq!(level.brightness(game, LightLayer::Block, pos), Some(0));
}

#[test]
#[ignore = "daily"]
fn light_query_reads_live_answers_in_the_named_level_without_publication() {
    with_lights_sides(
        "cut_layout::game::dynamic_light_emission_tests::light_query_reads_live_answers_in_the_named_level_without_publication",
        |game, level, _, depth, _| {
            assert_live_answers(
                game,
                level,
                &depth,
                "minecraft:overworld",
                |game| game.test_ask_edited_light_emission(game.dim),
                |game| {
                    assert_eq!(game.enter_command_level(Dim::Nether), Some(Dim::Overworld));
                },
                |game| {
                    assert_eq!(game.enter_command_level(Dim::Overworld), Some(Dim::Nether));
                },
            );
        },
        |game, level, _, depth, _| {
            assert_live_answers(
                game,
                level,
                &depth,
                "lights:remote",
                |game| game.ask_edited_light_emission(),
                |_| {},
                |_| {},
            );
        },
    );
}

#[test]
#[ignore = "daily"]
fn light_providers_can_delegate_but_an_active_cycle_refuses() {
    with_lights_server(
        "cut_layout::game::dynamic_light_emission_tests::light_providers_can_delegate_but_an_active_cycle_refuses",
        |game, level, _, depth, calls| {
            let host: &dyn Host = game;
            let proxy = host.block_state_id(PROXY).unwrap();
            assert_eq!(level.get_light_emission(host, [1, 64, 2], proxy), Ok(12));
            assert_eq!(depth.take(), [[1, 64, 2]]);
            // Asking the same state at another position refuses instead of reading static light.
            assert_eq!(level.get_light_emission(host, [1, 66, 2], proxy), Ok(4));
            assert_eq!(calls.load(Ordering::Relaxed), 2);

            let placed = game.pack.assets.registry.default_state(PROXY).unwrap();
            game.world
                .set_with_notification(1, 66, 2, placed, false)
                .unwrap();
            game.test_ask_edited_light_emission(game.dim);
            assert_eq!(
                calls.load(Ordering::Relaxed),
                3,
                "the engine ask is also guarded"
            );
            assert_eq!(
                answer(game, 66),
                None,
                "the provider declined the outer request"
            );
            assert_eq!(level.get_light_emission(game, [1, 64, 2], proxy), Ok(12));
            assert_eq!(depth.take(), [[1, 64, 2]]);
        },
    );
}

/// The stored answer of the cell at (1, y, 2) of chunk (0, 0).
fn answer(game: &sunrise_gameplay_common::game::CommonGame<'_>, y: i32) -> Option<u8> {
    let chunk = game.world.chunks.get(&(0, 0)).unwrap();
    let cell = chunk.dynamic_cell(1, y, 2).unwrap();
    chunk.dynamic_emission.get(&cell).copied()
}

#[test]
#[ignore = "daily"]
fn a_dynamic_light_part_answers_by_position_for_declared_blocks_only() {
    with_lights(
        "cut_layout::game::dynamic_light_emission_tests::a_dynamic_light_part_answers_by_position_for_declared_blocks_only",
        |game, _, States { lamp, plain, air }, depth| {
            for (y, state) in [(-10, lamp), (64, lamp), (65, lamp), (66, lamp), (67, lamp)] {
                game.world
                    .set_with_notification(1, y, 2, state, false)
                    .unwrap();
            }
            game.world
                .set_with_notification(1, 68, 2, plain, false)
                .unwrap();
            game.test_ask_edited_light_emission(game.dim);
            assert_eq!(
                depth.take(),
                [[1, -10, 2], [1, 64, 2], [1, 65, 2], [1, 66, 2], [1, 67, 2]],
                "the undeclared block at y 68 is not asked"
            );
            // Deeper is brighter; None keeps the state's 4 (no entry); 200 and 255 are stored
            // as 15.
            assert_eq!(
                [-10, 64, 65, 66, 67, 68].map(|y| answer(game, y)),
                [Some(9), Some(12), None, Some(15), Some(15), None]
            );

            // A bulk edit of the section holding y 64 to 67 replaces only the cell at y 66.
            let s = game
                .world
                .chunks
                .get(&(0, 0))
                .unwrap()
                .section_index(64)
                .unwrap();
            let chunk = Arc::make_mut(game.world.chunks.get_mut(&(0, 0)).unwrap());
            let mut values: [u32; 4096] = std::array::from_fn(|i| chunk.sections[s].get(i));
            values[2 << 8 | 2 << 4 | 1] = air.0;
            chunk.replace_blocks(s, &values);
            assert_eq!(
                [-10, 64, 66, 67].map(|y| answer(game, y)),
                [Some(9), Some(12), None, Some(15)]
            );

            // The column comes in again, its answers gone: every lamp is asked once more.
            let mut reloaded = (**game.world.chunks.get(&(0, 0)).unwrap()).clone();
            reloaded.dynamic_emission.clear();
            reloaded.dynamic_pending = true;
            game.world.chunks.remove(&(0, 0));
            game.world.chunks.insert((0, 0), Arc::new(reloaded));
            game.ask_chunk_light_emission((0, 0));
            let mut asked = depth.take();
            asked.sort();
            assert_eq!(asked, [[1, -10, 2], [1, 64, 2], [1, 65, 2], [1, 67, 2]]);
            assert_eq!(
                [-10, 64, 65, 66, 67].map(|y| answer(game, y)),
                [Some(9), Some(12), None, None, Some(15)]
            );
            assert!(!game.world.chunks.get(&(0, 0)).unwrap().dynamic_pending);
        },
    );
}

#[test]
#[ignore = "daily"]
fn an_edit_pending_when_its_level_parks_uses_that_levels_dynamic_emission() {
    with_lights(
        "cut_layout::game::dynamic_light_emission_tests::an_edit_pending_when_its_level_parks_uses_that_levels_dynamic_emission",
        |game, overworld, States { lamp, .. }, depth| {
            depth.glow.store(11, Ordering::Relaxed);
            game.world
                .set_with_notification(1, 70, 2, lamp, false)
                .unwrap();
            assert!(depth.take().is_empty());
            game.enter_level(Dim::Nether).unwrap();
            game.server
                .levels
                .0
                .parked_mut(Dim::Overworld)
                .unwrap()
                .streaming
                .paused = false;

            game.update_parked_light(&[Dim::Nether]);

            assert_eq!(game.dim, Dim::Nether);
            assert_eq!(depth.take(), [[1, 70, 2]]);
            assert_eq!(
                game.level_brightness(overworld, LightLayer::Block, [1, 70, 2]),
                Some(11)
            );
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_full_read_prepares_a_saved_columns_dynamic_light() {
    use sunrise_level::{
        chunk_streamer::ChunkStreamer, level_loading::Status, testgen::TestTerrain,
    };
    use sunrise_sim::save::{Persistent, SaveDir};

    with_lights(
        "cut_layout::game::dynamic_light_emission_tests::a_full_read_prepares_a_saved_columns_dynamic_light",
        |game, _, States { lamp, .. }, depth| {
            let scratch = tempfile::tempdir().unwrap();
            let registry = game.pack.assets.registry;
            let biomes = game.res.biomes();
            let save = Arc::new(SaveDir::new(
                scratch.path().to_owned(),
                game.world.shape,
                Some(game.res.shared_items()),
            ));
            let column = (20, 22);
            let [x, y, z] = [column.0 * 16 + 1, 64, column.1 * 16 + 2];
            let mut chunk = Chunk::empty(column.0, column.1);
            chunk.set(1, y, 2, lamp.0);
            assert!(!chunk.lit);
            save.save(&[Arc::new(chunk)], registry, biomes).unwrap();
            let mut streaming = ChunkStreamer::new(
                Arc::new(Persistent {
                    inner: Arc::new(TestTerrain::new(registry, biomes).unwrap()),
                    save,
                    registry: game.pack.assets.clone(),
                    biomes: biomes.clone(),
                }),
                game.pack.chunk_context.clone(),
                1,
            );
            streaming.radius = -1;
            let previous = std::mem::replace(&mut game.server.streaming, streaming);

            assert!(!game.world.chunks.contains_key(&column));
            assert!(game.test_load_full_column(column));
            assert_eq!(
                game.server
                    .streaming
                    .latest_status(&game.world, column.0, column.1),
                Some(Status::Full)
            );
            assert_eq!(game.world.light(x, y, z) & 15, 12);
            assert_eq!(game.world.light(x + 1, y, z) & 15, 11);
            assert_eq!(depth.take(), [[x, y, z]]);
            game.test_ask_loaded_light_emission(0);
            assert!(depth.take().is_empty());
            game.server.streaming = previous;
        },
    );
}

/// `check_light` asks a declared block's part again once at the next update however often it
/// is called, and asks nothing of an undeclared block; it is refused on a client and for another
/// dimension's handle. A server's light update asks a client's part again, for the changed
/// sections only.
#[test]
#[ignore = "daily"]
fn check_light_and_a_light_update_ask_the_part_again() {
    with_lights(
        "cut_layout::game::dynamic_light_emission_tests::check_light_and_a_light_update_ask_the_part_again",
        |game, level, States { lamp, plain, .. }, depth| {
            depth.glow.store(5, Ordering::Relaxed);
            game.world
                .set_with_notification(1, 70, 2, lamp, false)
                .unwrap();
            game.world
                .set_with_notification(1, 71, 2, plain, false)
                .unwrap();
            game.test_ask_edited_light_emission(game.dim);
            assert_eq!(depth.take(), [[1, 70, 2]]);
            assert_eq!(answer(game, 70), Some(5));
            game.world.light_edits.clear();

            // The part's input changes without a block change: the answer stays until a check.
            depth.glow.store(11, Ordering::Relaxed);
            for _ in 0..3 {
                assert!(game.level_check_light(level, [1, 70, 2]));
            }
            assert!(game.level_check_light(level, [1, 71, 2]));
            assert_eq!(
                game.world.light_edits,
                [((1, 70, 2), lamp.0, lamp.0), ((1, 71, 2), plain.0, plain.0)],
                "repeated checks queue one edit"
            );
            assert_eq!(answer(game, 70), Some(5));
            game.test_ask_edited_light_emission(game.dim);
            assert_eq!(
                depth.take(),
                [[1, 70, 2]],
                "asked once, the undeclared block not at all"
            );
            assert_eq!(answer(game, 70), Some(11));
            game.world.light_edits.clear();

            // After the ask a check queues again.
            assert!(game.level_check_light(level, [1, 70, 2]));
            assert_eq!(game.world.light_edits.len(), 1);
            game.world.light_edits.clear();

            let nether = LevelRef::new(level.epoch(), mod_api::DimensionKey::NETHER);
            assert!(
                !game.level_check_light(nether, [1, 70, 2]),
                "another dimension"
            );
            assert!(game.world.light_edits.is_empty());
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_light_update_asks_the_part_again_on_the_client() {
    with_lights_sides(
        "cut_layout::game::dynamic_light_emission_tests::a_light_update_asks_the_part_again_on_the_client",
        |_, _, _, _, _| {},
        |game, _, States { lamp, .. }, depth, _| {
            depth.glow.store(11, Ordering::Relaxed);
            game.world
                .set_with_notification(1, 70, 2, lamp, false)
                .unwrap();
            game.ask_edited_light_emission();
            assert_eq!(depth.take(), [[1, 70, 2]]);
            assert_eq!(answer(game, 70), Some(11));
            game.world.light_edits.clear();

            // A light update for another section asks nothing; one for the lamp's asks it once.
            depth.glow.store(3, Ordering::Relaxed);
            let s = game
                .world
                .chunks
                .get(&(0, 0))
                .unwrap()
                .section_index(70)
                .unwrap();
            game.ask_updated_light_emission((0, 0), &[s + 1]);
            assert!(depth.take().is_empty());
            assert_eq!(answer(game, 70), Some(11));
            game.ask_updated_light_emission((0, 0), &[s]);
            assert_eq!(depth.take(), [[1, 70, 2]]);
            assert_eq!(answer(game, 70), Some(3));
        },
    );
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
