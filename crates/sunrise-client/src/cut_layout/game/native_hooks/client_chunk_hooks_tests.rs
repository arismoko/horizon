//! `client_chunk_load` and `client_chunk_unload` through a loopback client (#3425): a chunk
//! packet adopts a column and tells its load once the blocks are in; every route that frees a
//! column (eviction, a radius shrink, the forget packet, level close) tells its unload, and the
//! view centre moving away hides slots untold.
#![cfg(feature = "client")]
use mod_api::{ClientChunkLoadEvent, ClientChunkUnloadEvent, ClientHost, Hooks};
use std::sync::Mutex;
use sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry;
use sunrise_gameplay_common::game::execution_role::ExecutionRole;
use sunrise_protocol::{
    chunk::{ChunkData, LightData, Paletted, Section, Strategy},
    packets::play::{LevelChunkWithLight, SpawnInfo},
};
use {
    crate::game::server_side::native_entity_refs::tests::with_game_role_client,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
};

/// What each event saw: its name, dimension, chunk and the block id at the chunk's corner
/// `(16 x, 64, 16 z)`, read through the client host inside the listener.
type Call = (&'static str, String, [i32; 2], Option<String>);
static CALLS: Mutex<Vec<Call>> = Mutex::new(Vec::new());

/// The block at `chunk`'s corner at y 64, as a listener reads it.
fn corner(host: &dyn ClientHost, dimension: &str, [x, z]: [i32; 2]) -> Option<String> {
    host.level(dimension)
        .and_then(|level| level.block(host, [x * 16, 64, z * 16]))
        .and_then(mod_api::__private::block_key_id)
        .map(str::to_owned)
}

struct Probe;
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn client_chunk_load(&self, host: &mut dyn ClientHost, event: &mut ClientChunkLoadEvent<'_>) {
        let block = corner(&*host, event.dimension, event.chunk);
        CALLS
            .lock()
            .unwrap()
            .push(("load", event.dimension.to_owned(), event.chunk, block));
    }
    fn client_chunk_unload(
        &self,
        host: &mut dyn ClientHost,
        event: &mut ClientChunkUnloadEvent<'_>,
    ) {
        let block = corner(&*host, event.dimension, event.chunk);
        CALLS
            .lock()
            .unwrap()
            .push(("unload", event.dimension.to_owned(), event.chunk, block));
    }
}

fn take_calls() -> Vec<Call> {
    std::mem::take(&mut *CALLS.lock().unwrap())
}

fn with_client(f: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>)) {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("chunk_probe", Box::new(Probe))].into();
    mod_platform::testing::with_mods(mods, || with_loopback(f));
}

fn with_loopback(f: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>)) {
    let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let root =
        sunrise_gameplay_common::game::mods::content_root(&base, &Default::default()).unwrap();
    with_game_role_client(
        root.prepared().to_path_buf(),
        ExecutionRole::Client,
        |game| {
            game.native_lifecycle =
                sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycle::default();
            let (client, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(client, "ChunkProbe").unwrap();
            game.enter_remote_dimension(
                &overworld(),
                RemoteDimensionEntry::Login,
                sunrise_client::level_loading::Reason::Other,
            );
            game.client.remote.as_mut().unwrap().logged_in = true;
            take_calls();
            f(game);
        },
    );
}

fn overworld() -> SpawnInfo {
    SpawnInfo {
        dimension_type: 0,
        dimension: "minecraft:overworld".into(),
        seed: 0,
        game_type: 1,
        previous_game_type: None,
        debug: false,
        flat: true,
        last_death: None,
        portal_cooldown: 0,
        sea_level: 63,
    }
}

/// A chunk packet for column `(x, z)` with stone at its local `(0, 64, 0)`.
fn stone_column(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    x: i32,
    z: i32,
) -> LevelChunkWithLight {
    let air = game.res.facts().air.0 as i32;
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap()
        .0 as i32;
    // Section 8 holds y 64..80 in a world from y -64; index 0 is its local (0, 0, 0).
    let sections: Vec<_> = (0..24)
        .map(|i| {
            let mut states = vec![air; 4096];
            if i == 8 {
                states[0] = stone;
            }
            Section {
                non_empty: i16::from(i == 8),
                fluids: 0,
                states: Paletted::from_values(&states, Strategy::Blocks, 16),
                biomes: Paletted::from_values(&[0; 64], Strategy::Biomes, 2),
            }
        })
        .collect();
    LevelChunkWithLight {
        x,
        z,
        chunk: ChunkData {
            heightmaps: vec![],
            buffer: ChunkData::from_sections(&sections),
            block_entities: vec![],
        },
        light: LightData {
            sky_mask: vec![],
            block_mask: vec![],
            empty_sky_mask: vec![],
            empty_block_mask: vec![],
            sky: vec![],
            block: vec![],
        },
    }
}

fn load(chunk: [i32; 2], block: Option<&str>) -> Call {
    (
        "load",
        "minecraft:overworld".into(),
        chunk,
        block.map(str::to_owned),
    )
}

fn unload(chunk: [i32; 2], block: Option<&str>) -> Call {
    (
        "unload",
        "minecraft:overworld".into(),
        chunk,
        block.map(str::to_owned),
    )
}

/// Each call without the block read, for the routes whose columns are empty.
fn without_blocks(calls: Vec<Call>) -> Vec<(&'static str, String, [i32; 2])> {
    calls.into_iter().map(|(n, d, c, _)| (n, d, c)).collect()
}

fn unloaded(chunk: [i32; 2]) -> Vec<(&'static str, String, [i32; 2])> {
    vec![("unload", "minecraft:overworld".into(), chunk)]
}

/// A load once the column is in (its block reads through the host), again for a re-sent
/// column; an unload for a forget with the block still readable, and none for a second forget
/// of the same column; on a dimension change, an unload naming the old dimension.
#[test]
#[ignore = "daily"]
fn a_client_tells_a_chunk_packet_and_its_forget_once() {
    const NAME: &str = "cut_layout::game::native_hooks::client_chunk_hooks_tests::a_client_tells_a_chunk_packet_and_its_forget_once";
    if run_fresh_cases(NAME, "SUNRISE_CLIENT_CHUNK_PACKET", &["game"]) {
        return;
    }
    with_client(|game| {
        let packet = stone_column(game, 3, 4);
        game.remote_chunk(&packet, None);
        assert_eq!(take_calls(), [load([3, 4], Some("minecraft:stone"))]);

        game.remote_chunk(&packet, None);
        assert_eq!(
            take_calls(),
            [load([3, 4], Some("minecraft:stone"))],
            "a re-sent column is told again"
        );

        let mut gone = [(3, 4)].into_iter().collect();
        game.forget_remote_columns(&mut gone, true);
        assert_eq!(take_calls(), [unload([3, 4], Some("minecraft:stone"))]);
        let mut gone = [(3, 4)].into_iter().collect();
        game.forget_remote_columns(&mut gone, true);
        assert!(take_calls().is_empty(), "a column already gone is not told");

        // A respawn into another level drops the old level's columns, told as the old
        // dimension's, before the client level changes.
        game.remote_chunk(&packet, None);
        take_calls();
        game.enter_remote_dimension(
            &SpawnInfo {
                dimension: "minecraft:the_nether".into(),
                ..overworld()
            },
            RemoteDimensionEntry::Login,
            sunrise_client::level_loading::Reason::Other,
        );
        assert_eq!(take_calls(), [unload([3, 4], Some("minecraft:stone"))]);
    });
}

/// SyntheticChunkCache: a centre move only hides slots and shows them again untold, a negative
/// alias replaces exactly its occupant, and a real resize frees out-of-range slots, each with
/// its unload.
#[test]
#[ignore = "daily"]
fn cache_packets_hide_replace_and_resize_slots() {
    use sunrise_protocol::packets::play::{
        Clientbound as C, Distance, ForgetLevelChunk, SetChunkCacheCenter,
    };
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::native_hooks::client_chunk_hooks_tests::cache_packets_hide_replace_and_resize_slots",
        || {
            with_client(|game| {
                let radius = |distance| C::SetChunkCacheRadius(Distance { distance });
                let center = |x| C::SetChunkCacheCenter(SetChunkCacheCenter { x, z: 0 });
                let forget = |x| C::ForgetLevelChunk(ForgetLevelChunk { pos: [x, 0] });
                game.client.settings.render_distance = 8;
                game.apply_packet(radius(2)).unwrap();
                let packet = stone_column(game, 0, 0);
                game.apply_packet(C::LevelChunkWithLight(packet)).unwrap();
                let origin = game.world.chunks[&(0, 0)].clone();
                take_calls();

                game.apply_packet(radius(1)).unwrap();
                assert_eq!(game.effective_render_distance(), 1);
                assert_eq!(game.terrain_distance(), 1);
                assert!(std::sync::Arc::ptr_eq(&origin, &game.world.chunks[&(0, 0)]));
                game.apply_packet(radius(0)).unwrap();
                assert_eq!(game.terrain_distance(), 8);
                game.client.settings.render_distance = 10;
                game.apply_settings();
                assert_eq!(game.client.streaming.radius, 10);
                game.apply_packet(radius(4)).unwrap();
                game.client.settings.render_distance = 11;
                game.apply_settings();
                assert_eq!(game.terrain_distance(), 4);
                assert_eq!(game.client.streaming.radius, 5);
                game.apply_packet(radius(14)).unwrap();
                assert_eq!(game.terrain_distance(), 11);
                game.apply_packet(radius(2)).unwrap();

                game.apply_packet(center(11)).unwrap();
                assert!(!game.world.loaded(0, 0));
                assert_eq!(game.world.chunks.resident_keys().count(), 1);
                game.apply_packet(forget(0)).unwrap();
                game.apply_packet(radius(1)).unwrap();
                game.apply_packet(center(0)).unwrap();
                assert!(std::sync::Arc::ptr_eq(&origin, &game.world.chunks[&(0, 0)]));
                assert!(take_calls().is_empty());

                game.apply_packet(center(-11)).unwrap();
                let packet = stone_column(game, -11, 0);
                game.apply_packet(C::LevelChunkWithLight(packet)).unwrap();
                let alias = game.world.chunks[&(-11, 0)].clone();
                assert_eq!(game.world.chunks.resident_keys().count(), 1);
                assert_eq!(
                    without_blocks(take_calls()),
                    [
                        ("unload", "minecraft:overworld".into(), [0, 0]),
                        ("load", "minecraft:overworld".into(), [-11, 0]),
                    ],
                );
                game.apply_packet(center(0)).unwrap();
                game.apply_packet(forget(0)).unwrap();
                game.apply_packet(center(-11)).unwrap();
                assert!(std::sync::Arc::ptr_eq(
                    &alias,
                    &game.world.chunks[&(-11, 0)]
                ));
                assert!(take_calls().is_empty());

                game.apply_packet(radius(8)).unwrap();
                let packet = stone_column(game, -3, 0);
                game.apply_packet(C::LevelChunkWithLight(packet)).unwrap();
                assert_eq!(game.world.chunks.resident_keys().count(), 2);
                take_calls();
                game.apply_packet(radius(2)).unwrap();
                assert_eq!(game.world.chunks.resident_keys().count(), 1);
                assert!(std::sync::Arc::ptr_eq(
                    &alias,
                    &game.world.chunks[&(-11, 0)]
                ));
                assert_eq!(without_blocks(take_calls()), unloaded([-3, 0]));
                assert_eq!(game.forget_all_remote_columns(), [(-11, 0)]);
                assert_eq!(without_blocks(take_calls()), unloaded([-11, 0]));
            })
        },
    );
}
