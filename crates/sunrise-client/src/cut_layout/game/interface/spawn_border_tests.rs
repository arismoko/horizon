use sunrise_core::{position::BlockPos, storage::PackedStorage};
use sunrise_protocol::{
    chunk::{ChunkData, LightData, Paletted, Section, Strategy},
    connection::Pipe,
    packets::play::{self, Clientbound as C},
    registry::DimensionType,
};
use sunrise_world::registry::BlockStateId;
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
};

const OVERWORLD: &str = "minecraft:overworld";
const NETHER: &str = "minecraft:the_nether";
const END: &str = "minecraft:the_end";

fn receive(game: &mut sunrise_client_gameplay::game::ClientGame<'_>, packet: C) {
    let bytes = packet.try_encode().unwrap();
    let mut reader = sunrise_protocol::buf::Reader::new(&bytes);
    let id = reader.varint().unwrap();
    game.handle_packet(C::decode_reader(id, reader).unwrap())
        .unwrap();
}

/// Join a local pipe as a client of the three vanilla dimensions.
fn login(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) {
    let (client, _server) = Pipe::pair();
    game.connect_local(client, "spawn-compass").unwrap();
    receive(
        game,
        C::Login(play::Login {
            player_id: 7,
            hardcore: false,
            levels: vec![OVERWORLD.into(), NETHER.into(), END.into()],
            max_players: 1,
            chunk_radius: 2,
            simulation_distance: 2,
            reduced_debug_info: false,
            show_death_screen: true,
            limited_crafting: false,
            spawn: play::SpawnInfo {
                dimension_type: 0,
                dimension: OVERWORLD.into(),
                seed: 0,
                game_type: 1,
                previous_game_type: None,
                debug: false,
                flat: false,
                last_death: None,
                portal_cooldown: 0,
                sea_level: 28,
            },
            online_mode: false,
            enforces_secure_chat: false,
        }),
    );
}

/// Collects the compass's checks over one scene, so a run reports every case that differs.
#[derive(Default)]
struct Compass(Vec<String>);

impl Compass {
    /// `ClientLevel.setRespawnData` for `pos` when given, then the target the client holds.
    fn observe(
        &mut self,
        game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
        label: &str,
        dimension: &'static str,
        pos: Option<[i32; 3]>,
        expected: [i32; 3],
    ) {
        if let Some(pos) = pos {
            receive(
                game,
                C::SetDefaultSpawnPosition(play::SetDefaultSpawnPosition {
                    pos: play::GlobalPos {
                        dimension: String::from(dimension),
                        pos,
                    },
                    yaw: f32::from_bits(0x4321_6000),
                    pitch: -0.0,
                }),
            );
        }
        let target = game.level_state_now(OVERWORLD).respawn.unwrap();
        if (target.dimension.as_str(), target.pos) == (dimension, expected) {
            println!("{label} PASS");
        } else {
            self.0.push(format!(
                "{label}: expected {dimension} at {expected:?}, got {target:?}"
            ));
        }
    }
}

/// The default state ids of stone, water and a torch.
fn states(game: &sunrise_client_gameplay::game::ClientGame<'_>) -> [BlockStateId; 3] {
    let registry = game.pack.assets.registry;
    ["stone", "water", "torch"]
        .map(|id| registry.default_state(&format!("minecraft:{id}")).unwrap())
}

/// Column (-1, -1) with the given blocks at x = z = -1: `(y, state, fluid)`.
fn send_column(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    words: Option<Vec<i64>>,
    blocks: &[(i32, BlockStateId, bool)],
) {
    let shape = DimensionType::vanilla(OVERWORLD);
    let sections: Vec<_> = (0..shape.sections())
        .map(|s| {
            let mut states = [0; 4096];
            let mut non_empty = 0;
            let mut fluids = 0;
            for &(y, state, fluid) in blocks {
                let dy = y - shape.min_y;
                if (dy >> 4) as usize == s {
                    states[((dy & 15) * 256 + 255) as usize] = state.0 as i32;
                    non_empty += 1;
                    fluids += i16::from(fluid);
                }
            }
            Section {
                non_empty,
                fluids,
                states: Paletted::from_values(&states, Strategy::Blocks, 16),
                biomes: Paletted::from_values(&[0; 64], Strategy::Biomes, 2),
            }
        })
        .collect();
    receive(
        game,
        C::LevelChunkWithLight(play::LevelChunkWithLight {
            x: -1,
            z: -1,
            chunk: ChunkData {
                heightmaps: Option::into_iter(words).map(|words| (4, words)).collect(),
                buffer: ChunkData::from_sections(&sections),
                block_entities: Vec::new(),
            },
            light: LightData {
                sky_mask: Vec::new(),
                block_mask: Vec::new(),
                empty_sky_mask: Vec::new(),
                empty_block_mask: Vec::new(),
                sky: Vec::new(),
                block: Vec::new(),
            },
        }),
    );
}

/// A server block update at (-1, y, -1).
fn update(game: &mut sunrise_client_gameplay::game::ClientGame<'_>, y: i32, state: i32) {
    receive(
        game,
        C::BlockUpdate(play::BlockUpdate {
            pos: [-1, y, -1],
            state,
        }),
    );
}

/// A world border of size 8 about (-0.25, -0.75).
fn border(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) {
    receive(game, C::SetBorderSize(play::SetBorderSize { size: 8.0 }));
    receive(
        game,
        C::SetBorderCenter(play::SetBorderCenter { x: -0.25, z: -0.75 }),
    );
}

/// A 9-bit motion-blocking heightmap holding 40 above the dimension's block-zero reference (-64).
fn wire() -> Vec<i64> {
    let mut supplied = PackedStorage::new(9, 256).unwrap();
    supplied.set(255, 104).unwrap();
    supplied.words().iter().map(|&word| word as i64).collect()
}

/// `ClientLevel.setRespawnData`: the compass uses the adjusted position and the packet's level.
#[test]
#[ignore = "daily"]
fn spawn_packet_stores_the_border_adjusted_compass_target() {
    if run_fresh_cases(
        "cut_layout::game::client_side::item_draw::spawn_border_tests::spawn_packet_stores_the_border_adjusted_compass_target",
        "SUNRISE_SPAWN_BORDER_CASE",
        &["spawn-border"],
    ) {
        return;
    }
    with_client_game(|game| {
        login(game);
        let mut compass = Compass::default();
        // These positions are checked by SyntheticSpawnBorder against 26.3.
        receive(game, C::SetBorderSize(play::SetBorderSize { size: 8.0 }));
        compass.observe(
            game,
            "inside retains all fields",
            OVERWORLD,
            Some([1, 23, -2]),
            [1, 23, -2],
        );
        compass.observe(
            game,
            "minimum is inclusive",
            OVERWORLD,
            Some([-4, 23, -4]),
            [-4, 23, -4],
        );
        compass.observe(
            game,
            "maximum is exclusive and centre chunk absent",
            OVERWORLD,
            Some([4, 23, -4]),
            [0, -64, 0],
        );

        receive(
            game,
            C::SetBorderCenter(play::SetBorderCenter { x: -0.25, z: -0.75 }),
        );
        let [stone, water, torch] = states(game);
        let blocks = [(15, stone, false), (20, water, true), (39, torch, false)];
        let outside = Some([32, 23, 32]);
        send_column(game, Some(vec![0]), &blocks[..2]);
        compass.observe(
            game,
            "fractional centre floors and highest fluid counts",
            NETHER,
            outside,
            [-1, 21, -1],
        );
        send_column(game, Some(wire()), &blocks);
        compass.observe(
            game,
            "encoded motion-blocking height wins",
            END,
            outside,
            [-1, 40, -1],
        );
        receive(
            game,
            C::ForgetLevelChunk(play::ForgetLevelChunk { pos: [-1, -1] }),
        );
        send_column(game, None, &blocks);
        compass.observe(
            game,
            "new packet omitting heightmap leaves the initial minimum",
            END,
            outside,
            [-1, -64, -1],
        );
        send_column(game, Some(wire()), &blocks);
        send_column(game, None, &blocks);
        compass.observe(
            game,
            "resident resend omitting heightmap preserves the old height",
            END,
            outside,
            [-1, 40, -1],
        );
        update(game, 40, torch.0 as i32);
        compass.observe(
            game,
            "nonblocking edit above the top leaves it alone",
            END,
            outside,
            [-1, 40, -1],
        );
        update(game, 24, stone.0 as i32);
        compass.observe(
            game,
            "new opaque block far below wire height leaves it alone",
            END,
            outside,
            [-1, 40, -1],
        );
        update(game, 39, 0);
        compass.observe(
            game,
            "nonopaque replacement at the old top scans down",
            END,
            outside,
            [-1, 25, -1],
        );
        update(game, 41, water.0 as i32);
        compass.observe(
            game,
            "new fluid above old height raises it",
            END,
            outside,
            [-1, 42, -1],
        );
        send_column(game, Some(vec![0]), &blocks);
        compass.observe(
            game,
            "wrong packed length primes from the newly read sections",
            END,
            outside,
            [-1, 21, -1],
        );
        send_column(game, Some(vec![0]), &[]);
        compass.observe(
            game,
            "wrong packed length does not zero previously nonzero empty columns",
            END,
            outside,
            [-1, 21, -1],
        );
        compass.observe(
            game,
            "packet block coordinates are not centres",
            END,
            Some([3, 23, 3]),
            [3, 23, 3],
        );
        receive(
            game,
            C::SetBorderCenter(play::SetBorderCenter {
                x: 30_000_000.25,
                z: -0.75,
            }),
        );
        compass.observe(
            game,
            "border changes do not re-adjust stored spawn",
            END,
            None,
            [3, 23, 3],
        );
        compass.observe(
            game,
            "column outside hard level bounds uses the supplied sea level",
            OVERWORLD,
            outside,
            [30_000_000, 29, -1],
        );
        assert!(compass.0.is_empty(), "{}", compass.0.join("\n"));
    });
}

/// A predicted block takes the one client block write: it raises the compass height, and the
/// acknowledgement restores the server's state and scans the column back down
/// (`LevelChunk.setBlockState` updating the heightmap, `BlockStatePredictionHandler.endPredictionsUpTo`).
#[test]
#[ignore = "daily"]
fn a_predicted_block_moves_the_compass_height_until_acknowledged() {
    if run_fresh_cases(
        "cut_layout::game::client_side::item_draw::spawn_border_tests::a_predicted_block_moves_the_compass_height_until_acknowledged",
        "SUNRISE_SPAWN_BORDER_CASE",
        &["spawn-border-prediction"],
    ) {
        return;
    }
    with_client_game(|game| {
        login(game);
        border(game);
        let [stone, water, torch] = states(game);
        send_column(
            game,
            Some(wire()),
            &[(15, stone, false), (20, water, true), (39, torch, false)],
        );
        let mut compass = Compass::default();
        let outside = Some([32, 23, 32]);
        compass.observe(
            game,
            "encoded motion-blocking height wins",
            END,
            outside,
            [-1, 40, -1],
        );
        game.predict_blocks(&[(BlockPos::new(-1, 45, -1), stone)], 1);
        compass.observe(
            game,
            "a predicted block above the top raises it",
            END,
            outside,
            [-1, 46, -1],
        );
        receive(
            game,
            C::BlockChangedAck(play::BlockChangedAck { sequence: 1 }),
        );
        compass.observe(
            game,
            "the acknowledgement restores the server's air and scans down to the water",
            END,
            outside,
            [-1, 21, -1],
        );
        assert!(compass.0.is_empty(), "{}", compass.0.join("\n"));
    });
}
