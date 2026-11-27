//! `ClientLevel.tickBlockEntities` for spawners, trial spawners, vaults, campfires, beacons,
//! potent sulfur and animated block entities on a server's client. Against vanilla 26.3
//! (`tools/test_block_entity_client_tick_parity.py`), the fixture compares particles, sounds,
//! spins, beacon beams, animation state and the level random after each tick. Separate cases
//! exercise ticker enabling, animation lifetimes and when particles join the engine.
use std::sync::Arc;
use sunrise_client::block_entity_anim::Ticker;
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;
use sunrise_level::chunk::block_entity::{Next, SpawnData, Spawner, TrialSpawner, Vault};
use sunrise_level::containers::BlockEntity;

type Pos = (i32, i32, i32);
use sunrise_protocol::chunk::{BlockEntityInfo, ChunkData, LightData};
use sunrise_world::registry::BlockStateId;

/// The committed cases and the SHA-256 of the log vanilla made of each (`--write` of the harness
/// makes them).
const COMMITTED: &str =
    include_str!("../../../../../../docs/parity/block-entity-client-ticks.jsonl");
/// The packets' block entity type ids: their index here.
const TYPES: [&str; 12] = [
    "minecraft:mob_spawner",
    "minecraft:trial_spawner",
    "minecraft:vault",
    "minecraft:campfire",
    "minecraft:conduit",
    "minecraft:beacon",
    "minecraft:potent_sulfur",
    "minecraft:chest",
    "minecraft:shulker_box",
    "minecraft:bell",
    "minecraft:enchanting_table",
    "minecraft:decorated_pot",
];
const PLAYER: [i32; 4] = [0x5355_4e52, 0x4953_4500, 0, 1];

/// The harness gives its cases (`SUNRISE_CLIENT_TICK_CASES`) and takes the logs
/// (`SUNRISE_CLIENT_TICK_OUT`) to compare them with vanilla's. Without them (the daily run) the
/// committed cases run, and each log must hash to vanilla's.
#[test]
#[ignore = "daily"]
fn client_ticks_match_the_vanilla_cases() {
    run_client_ticks_match_the_vanilla_cases();
}

pub(crate) fn run_client_ticks_match_the_vanilla_cases() {
    if run_in_child("client_ticks_match_the_vanilla_cases") {
        return;
    }
    let harness = std::env::var("SUNRISE_CLIENT_TICK_CASES")
        .ok()
        .zip(std::env::var("SUNRISE_CLIENT_TICK_OUT").ok());
    let committed: Vec<serde_json::Value> = COMMITTED
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let cases: Vec<serde_json::Value> = match &harness {
        Some((cases, _)) => std::fs::read_to_string(cases)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect(),
        None => committed.iter().map(|row| row["case"].clone()).collect(),
    };
    with_remote_game(|game| {
        let mut reordered = false;
        let logs: Vec<String> = cases
            .iter()
            .map(|case| client_tick_case(game, case, &mut reordered).to_string())
            .collect();
        match harness {
            Some((_, out)) => std::fs::write(
                out,
                logs.iter()
                    .map(|log| format!("{log}\n"))
                    .collect::<String>(),
            )
            .unwrap(),
            None => {
                use sha2::Digest;
                let differ: Vec<&str> = committed
                    .iter()
                    .zip(&logs)
                    .filter(|(row, log)| {
                        format!("{:x}", sha2::Sha256::digest(log.as_bytes())) != row["vanilla"]
                    })
                    .map(|(row, _)| row["case"]["name"].as_str().unwrap())
                    .collect();
                assert!(
                    differ.is_empty(),
                    "these cases tick unlike vanilla 26.3 (tools/test_block_entity_client_tick_parity.py \
                     shows where): {differ:?}"
                );
                // A column whose packet lists its block entities in another order than its block
                // entity map walks them: the tickers follow the packet.
                assert!(reordered, "no case sends a column out of its map's order");
            }
        }
    });
}

/// Resource loading publishes registries process-wide. Keep them in the test's own process.
fn run_in_child(test: &str) -> bool {
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        &format!("cut_layout::game::client_side::spawner_client_tick_tests::{test}"),
        "SUNRISE_CLIENT_TICK_CHILD",
        &["remote"],
    )
}

/// A client of a server with the block entity types of `TYPES`, for `test`.
fn with_remote_game(test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>)) {
    sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
        game.client.settings.render_distance = 32;
        let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
        game.connect_local(pipe, "ClientTick").unwrap();
        let remote = game.client.remote.as_mut().unwrap();
        remote.logged_in = true;
        Arc::make_mut(&mut remote.builtin).registries.insert(
            "minecraft:block_entity_type".into(),
            sunrise_protocol::registry::Ids::from_names(TYPES.map(str::to_owned).to_vec()),
        );
        test(game);
    });
}

fn hex_f64(v: f64) -> String {
    format!("{:x}", v.to_bits())
}

fn hex_f32(v: f32) -> String {
    format!("{:x}", v.to_bits())
}

fn int(v: &serde_json::Value) -> i32 {
    v.as_i64().unwrap() as i32
}

fn key_of(e: &serde_json::Value) -> Pos {
    (int(&e["pos"][0]), int(&e["pos"][1]), int(&e["pos"][2]))
}

fn column(k: Pos) -> (i32, i32) {
    (k.0 >> 4, k.2 >> 4)
}

/// One case's block entity as the server holds it, with its block state and packet type id.
fn client_tick_entity(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    e: &serde_json::Value,
) -> (BlockStateId, i32, BlockEntity) {
    let text = |k: &str| e[k].as_str().unwrap().to_owned();
    let flag = |k: &str| e[k].as_bool().unwrap().to_string();
    let (name, kind, properties, entity) = match e["kind"].as_str().unwrap() {
        "spawner" => {
            let mut mob = sunrise_core::nbt::Compound::new();
            mob.insert("id".into(), Tag::String("minecraft:pig".into()));
            let spawner = Spawner {
                required_player_range: int(&e["range"]),
                spawn_delay: int(&e["delay"]),
                next: Some(Next::Own(SpawnData {
                    entity: mob,
                    ..Default::default()
                })),
                ..Spawner::default()
            };
            (
                "minecraft:spawner",
                0,
                vec![],
                BlockEntity::Spawner(Box::new(spawner)),
            )
        }
        "trial_spawner" => (
            "minecraft:trial_spawner",
            1,
            vec![
                ("trial_spawner_state", text("state")),
                ("ominous", flag("ominous")),
            ],
            BlockEntity::TrialSpawner(Box::new(TrialSpawner {
                next_mob_spawns_at: e["nextMobSpawnsAt"].as_i64().unwrap(),
                ..Default::default()
            })),
        ),
        "campfire" => (
            "minecraft:campfire",
            3,
            vec![
                ("facing", text("facing")),
                ("lit", flag("lit")),
                ("signal_fire", flag("signal")),
            ],
            BlockEntity::Campfire(Box::new(sunrise_level::chunk::block_entity::Campfire {
                items: std::array::from_fn(|i| {
                    e["items"][i]
                        .as_bool()
                        .unwrap()
                        .then(|| sunrise_items::items::ItemStack::new("minecraft:porkchop", 1))
                }),
                ..Default::default()
            })),
        ),
        "conduit" => (
            "minecraft:conduit",
            4,
            vec![("waterlogged", "true".to_owned())],
            BlockEntity::Conduit(None),
        ),
        "beacon" => (
            "minecraft:beacon",
            5,
            vec![],
            BlockEntity::Beacon(Default::default()),
        ),
        "potent_sulfur" => (
            "minecraft:potent_sulfur",
            6,
            vec![("potent_sulfur_state", text("state"))],
            BlockEntity::PotentSulfur(Default::default()),
        ),
        // The animated block entities (`block_entity_anims`): no data the tick reads.
        "chest" => (
            "minecraft:chest",
            7,
            vec![("facing", text("facing"))],
            BlockEntity::Chest(vec![None; 27]),
        ),
        "shulker" => (
            "minecraft:shulker_box",
            8,
            vec![("facing", text("facing"))],
            BlockEntity::Chest(vec![None; 27]),
        ),
        "bell" => (
            "minecraft:bell",
            9,
            vec![
                ("facing", text("facing")),
                ("attachment", "floor".to_owned()),
            ],
            BlockEntity::Bell,
        ),
        "enchanting_table" => (
            "minecraft:enchanting_table",
            10,
            vec![],
            BlockEntity::EnchantingTable,
        ),
        _ => (
            "minecraft:vault",
            2,
            vec![
                ("facing", text("facing")),
                ("ominous", flag("ominous")),
                ("vault_state", text("state")),
            ],
            BlockEntity::Vault(Box::new(Vault {
                display_item: e["display"]
                    .as_bool()
                    .unwrap()
                    .then(|| sunrise_items::items::ItemStack::new("minecraft:diamond", 1)),
                connected_players: if e["connected"].as_bool().unwrap() {
                    vec![PLAYER]
                } else {
                    Vec::new()
                },
                connected_particles_range: e["particlesRange"].as_f64().unwrap(),
                ..Default::default()
            })),
        ),
    };
    let registry = game.pack.assets.registry;
    let mut state = registry
        .state(registry.default_state(name).unwrap())
        .unwrap()
        .clone();
    for (k, v) in properties {
        state.properties.insert(k.into(), v);
    }
    (registry.id(&state).unwrap(), kind, entity)
}

/// `ChunkMap.playerLoadedChunk`'s packet for column `at`: its blocks, and the block entities
/// `listed` (indices into the case's) in that order, each with its update tag
/// (`BlockEntity.getUpdateTag`, as a server sends it). Whether that order differs from the order
/// the client's block entity map walks them.
fn send_column(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    entities: &[(Pos, BlockStateId, i32, BlockEntity)],
    at: (i32, i32),
    listed: &[usize],
    fills: &serde_json::Value,
) -> bool {
    let mut chunk = sunrise_level::chunk::Chunk::empty(at.0, at.1);
    // The case's fills over this column, then its block entities' own states.
    for fill in fills.as_array().into_iter().flatten() {
        let corner = |k: &str, i: usize| int(&fill[k][i]);
        let block = game
            .pack
            .assets
            .registry
            .default_state(fill["block"].as_str().unwrap())
            .unwrap();
        for x in corner("from", 0)..=corner("to", 0) {
            for y in corner("from", 1)..=corner("to", 1) {
                for z in corner("from", 2)..=corner("to", 2) {
                    if column((x, y, z)) == at {
                        chunk.set(x & 15, y, z & 15, block.0);
                    }
                }
            }
        }
    }
    let mut infos = Vec::new();
    for &i in listed {
        let (k, state, kind, entity) = &entities[i];
        assert_eq!(column(*k), at, "entity {i} is not in column {at:?}");
        chunk.set(k.0 & 15, k.1, k.2 & 15, state.0);
        // Only a trial spawner's state has this property.
        let active = game
            .pack
            .assets
            .registry
            .state(*state)
            .unwrap()
            .properties
            .get("trial_spawner_state")
            .is_some_and(|v| v.as_str() == "active");
        infos.push(BlockEntityInfo {
            packed_xz: (((k.0 & 15) << 4) | (k.2 & 15)) as u8 as i8,
            y: k.1 as i16,
            kind: *kind,
            tag: sunrise_sim::lan::chunks::entity_tag(entity, active, game.res.items())
                .unwrap_or(Tag::End),
        });
    }
    let packet = sunrise_protocol::packets::play::LevelChunkWithLight {
        x: at.0,
        z: at.1,
        chunk: ChunkData {
            heightmaps: Vec::new(),
            buffer: Vec::new(),
            block_entities: infos,
        },
        light: LightData {
            sky_mask: Vec::new(),
            block_mask: Vec::new(),
            empty_sky_mask: Vec::new(),
            empty_block_mask: Vec::new(),
            sky: Vec::new(),
            block: Vec::new(),
        },
    };
    let format = game.client.remote.as_ref().unwrap().chunk_format.clone();
    game.remote_chunk(&packet, Some((&format, Ok(chunk))));
    let order: Vec<Pos> = listed.iter().map(|&i| entities[i].0).collect();
    sunrise_level::block_entities::chunk_map_order(order.iter().copied()) != order
}

/// The particle and sound requests captured since the tick began, as one stream in the order
/// they were made: a sound sits after the particles requested before it
/// (`CapturedSound::particles_before`).
fn event_stream(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    particles_at_start: u64,
) -> Vec<serde_json::Value> {
    let particles = game
        .client
        .particles
        .0
        .requested
        .replace(Vec::new())
        .unwrap();
    let sounds = game.client.audio.0.capture.replace(Vec::new()).unwrap();
    let mut out = Vec::new();
    let mut particles = particles.into_iter();
    let mut taken = 0;
    let particle_row = |(o, p, v): (
        sunrise_level::particle_options::ParticleOptions,
        [f64; 3],
        [f64; 3],
    )| {
        let mut row = vec![
            serde_json::json!("particle"),
            serde_json::json!(format!("minecraft:{}", o.ty.name())),
        ];
        row.extend(p.iter().chain(&v).map(|d| serde_json::json!(hex_f64(*d))));
        serde_json::Value::Array(row)
    };
    for s in sounds {
        while taken < s.particles_before - particles_at_start {
            out.push(particle_row(particles.next().unwrap()));
            taken += 1;
        }
        let mut row = vec![serde_json::json!("sound"), serde_json::json!(s.event)];
        row.extend(s.position.iter().map(|d| serde_json::json!(hex_f64(*d))));
        row.push(serde_json::json!(hex_f32(s.volume)));
        row.push(serde_json::json!(hex_f32(s.pitch)));
        row.push(serde_json::json!(format!("{:x}", s.seed)));
        out.push(serde_json::Value::Array(row));
    }
    out.extend(particles.map(particle_row));
    out
}

/// A case's start: the level fresh, its player and time set, and its columns sent as chunk
/// packets. Returns its block entities.
fn start_case(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    case: &serde_json::Value,
    reordered: &mut bool,
) -> Vec<(Pos, BlockStateId, i32, BlockEntity)> {
    game.forget_all_remote_columns();
    // The last case's animation state: a level starts with none.
    game.client.be_anims.retain_present(&[]);
    let entities: Vec<_> = case["entities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            let (state, kind, entity) = client_tick_entity(game, e);
            (key_of(e), state, kind, entity)
        })
        .collect();
    let p = &case["player"];
    let at = |i: usize| p["pos"][i].as_f64().unwrap();
    let mut player =
        sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(at(0), at(1), at(2)))
            .unwrap();
    player.health.dead = p["dead"].as_bool().unwrap();
    game.player = Some(player);
    game.mode = if p["spectator"].as_bool().unwrap() {
        sunrise_command::args::GameMode::Spectator
    } else {
        sunrise_command::args::GameMode::Survival
    };
    game.client.remote.as_mut().unwrap().profile = sunrise_sim::lan::playerdata::ints_uuid(PLAYER);
    game.level.player_uuid = Some(PLAYER);
    game.client
        .particles
        .0
        .seed_randoms(0, case["seed"].as_i64().unwrap(), 0);
    // `EnchantingTableBlockEntity.RANDOM`, shared by every table: one stream per case.
    game.client.be_anims.book_random =
        sunrise_core::random::LegacyRandom::new(case["seed"].as_i64().unwrap());
    game.world.game_time = case["gameTime"].as_i64().unwrap();
    // The columns arrive in the order their first block entity is listed, each listing its
    // block entities in the case's order.
    let mut columns: Vec<((i32, i32), Vec<usize>)> = Vec::new();
    for (i, (k, ..)) in entities.iter().enumerate() {
        match columns.iter_mut().find(|(c, _)| *c == column(*k)) {
            Some((_, listed)) => listed.push(i),
            None => columns.push((column(*k), vec![i])),
        }
    }
    for (at, listed) in &columns {
        *reordered |= send_column(game, &entities, *at, listed, &case["fills"]);
    }
    game.client.particles.0.requested = Some(Vec::new());
    game.client.audio.0.capture = Some(Vec::new());
    entities
}

/// What the ticks left of each beacon and animated block entity, in the oracle's rows: ints as
/// numbers, floats as raw bits, sorted as `Vec3i.compareTo` does.
fn animation_states(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
) -> Vec<serde_json::Value> {
    use serde_json::json;
    let mut rows: Vec<_> = (&game.world.block_entities)
        .into_iter()
        .filter_map(|(k, e)| {
            let pos = [k.0, k.1, k.2];
            let anims = &game.client.be_anims;
            let colour = |sections: &[(u32, i32)]| -> Vec<_> {
                sections
                    .iter()
                    .map(|(c, h)| json!([c & 0xFF_FFFF, h]))
                    .collect()
            };
            let ticker = anims.ticker(game.world.block(k.0, k.1, k.2).0);
            let [x, y, z] = pos;
            let row = match (e, ticker) {
                (BlockEntity::Beacon(b), _) => json!([
                    x,
                    y,
                    z,
                    "beacon",
                    b.levels,
                    b.last_check_y,
                    colour(&b.sections),
                    colour(&b.checking)
                ]),
                (_, Some(Ticker::Lid)) => {
                    let lid = anims.chests.get(&pos).cloned().unwrap_or_default();
                    json!([
                        x,
                        y,
                        z,
                        "chest",
                        hex_f32(lid.get_openness(1.0)),
                        hex_f32(lid.get_openness(0.0))
                    ])
                }
                (_, Some(Ticker::Shulker { .. })) => {
                    let s = anims.shulkers.get(&pos).cloned().unwrap_or_default();
                    json!([
                        x,
                        y,
                        z,
                        "shulker",
                        hex_f32(s.progress),
                        hex_f32(s.progress_old)
                    ])
                }
                (_, Some(Ticker::Bell)) => {
                    let b = anims.bells.get(&pos).cloned().unwrap_or_default();
                    json!([x, y, z, "bell", b.ticks, i32::from(b.shaking)])
                }
                (_, Some(Ticker::Book)) => {
                    let b = anims.books.get(&pos).cloned().unwrap_or_default();
                    json!([
                        x,
                        y,
                        z,
                        "book",
                        b.time,
                        hex_f32(b.flip),
                        hex_f32(b.o_flip),
                        hex_f32(b.flip_t),
                        hex_f32(b.flip_a),
                        hex_f32(b.open),
                        hex_f32(b.o_open),
                        hex_f32(b.rot),
                        hex_f32(b.o_rot),
                        hex_f32(b.t_rot)
                    ])
                }
                (_, Some(Ticker::Conduit)) => {
                    let c = anims.conduits.get(&pos).cloned().unwrap_or_default();
                    json!([
                        x,
                        y,
                        z,
                        "conduit",
                        c.tick_count,
                        hex_f32(c.active_rotation),
                        i32::from(c.is_active),
                        i32::from(c.is_hunting)
                    ])
                }
                _ => return None,
            };
            Some(((k.1, k.2, k.0), row))
        })
        .collect();
    rows.sort_by_key(|(order, _)| *order);
    rows.into_iter().map(|(_, row)| row).collect()
}

fn client_tick_case(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    case: &serde_json::Value,
    reordered: &mut bool,
) -> serde_json::Value {
    let entities = start_case(game, case, reordered);
    let mut ticks = Vec::new();
    for t in 0..case["ticks"].as_i64().unwrap() {
        let particles_at_start = sunrise_client::particles::requested_count();
        // The packets handled before the tick.
        for op in case["events"][t.to_string()]
            .as_array()
            .into_iter()
            .flatten()
        {
            match op[0].as_str().unwrap() {
                "forget" => {
                    let at = (int(&op[1]), int(&op[2]));
                    game.forget_remote_columns(&mut [at].into_iter().collect(), true);
                }
                "send" => {
                    let listed: Vec<usize> = op[3]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|i| i.as_u64().unwrap() as usize)
                        .collect();
                    *reordered |= send_column(
                        game,
                        &entities,
                        (int(&op[1]), int(&op[2])),
                        &listed,
                        &case["fills"],
                    );
                }
                "event" => {
                    let pos = [int(&op[2][0]), int(&op[2][1]), int(&op[2][2])];
                    game.client_level_event_at(int(&op[1]), pos, int(&op[3]));
                }
                // `ClientboundBlockEventPacket`: the block's `triggerEvent`.
                "blockEvent" => {
                    let pos = [int(&op[1][0]), int(&op[1][1]), int(&op[1][2])];
                    game.block_entity_event(pos, int(&op[2]), int(&op[3]));
                }
                other => panic!("unknown case event {other}"),
            }
        }
        // `path: remote` runs the whole phase, so a pass outside the ticker list shows.
        if case["path"] == "remote" {
            game.tick_remote_block_entities();
            // `Minecraft.tick`: the particle engine follows the block entities.
            game.tick_particle_engine();
        } else {
            game.tick_client_block_entities();
        }
        let events = event_stream(game, particles_at_start);
        // Vec3i.compareTo: y, then z, then x.
        let mut spins: Vec<_> = (&game.world.block_entities)
            .into_iter()
            .filter_map(|(k, e)| {
                let (spin, o_spin) = match e {
                    BlockEntity::Spawner(s) => (hex_f64(s.spin), hex_f64(s.o_spin)),
                    BlockEntity::TrialSpawner(s) => (hex_f64(s.spin), hex_f64(s.o_spin)),
                    BlockEntity::Vault(v) => (hex_f32(v.spin), hex_f32(v.o_spin)),
                    _ => return None,
                };
                Some((
                    (k.1, k.2, k.0),
                    serde_json::json!([k.0, k.1, k.2, spin, o_spin]),
                ))
            })
            .collect();
        spins.sort_by_key(|(order, _)| *order);
        let states = animation_states(game);
        // Keys in their sorted order, as the harness hashes vanilla's log.
        ticks.push(serde_json::json!({
            "events": events,
            "random": format!("{:x}", game.client.particles.0.level_random().state()),
            "states": states,
            "spins": spins.into_iter().map(|(_, s)| s).collect::<Vec<_>>(),
        }));
        game.world.game_time += 1;
    }
    game.client.particles.0.requested = None;
    game.client.audio.0.capture = None;
    serde_json::json!({ "ticks": ticks })
}

// The ticker list's order and the animated block entities' lifetimes, case by case (not vanilla
// logs: each states the vanilla rule it follows).

fn who(at: [f64; 3]) -> serde_json::Value {
    serde_json::json!({ "pos": at, "dead": false, "spectator": false })
}

fn case_of(
    time: i64,
    at: [f64; 3],
    entities: serde_json::Value,
    fills: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "name": "own", "seed": 7, "gameTime": time, "ticks": 0, "player": who(at),
        "entities": entities, "fills": fills,
    })
}

fn campfire_json(pos: [i32; 3], lit: bool) -> serde_json::Value {
    serde_json::json!({ "kind": "campfire", "pos": pos, "items": [true, true, true, true],
        "lit": lit, "signal": false, "facing": "north" })
}

fn spawner_json(pos: [i32; 3]) -> serde_json::Value {
    serde_json::json!({ "kind": "spawner", "pos": pos, "range": -1, "delay": 20 })
}

/// One tick of a client's whole block entity phase (`tick_remote_block_entities`), then the
/// particle engine as `tick_remote_player` runs it: what it requested, and the game time moves on.
fn tick_phase(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) -> Vec<serde_json::Value> {
    let start = sunrise_client::particles::requested_count();
    game.tick_remote_block_entities();
    game.tick_particle_engine();
    let events = event_stream(game, start);
    game.world.game_time += 1;
    events
}

fn x_of(event: &serde_json::Value) -> f64 {
    f64::from_bits(u64::from_str_radix(event[2].as_str().unwrap(), 16).unwrap())
}

/// `LevelChunk.setBlockState` on an unlit campfire made before the first ticker pass: lit, its
/// ticker (`CampfireBlock.getTicker`) is added at that moment, behind the spawner that arrived
/// after it, and not ahead of it where the campfire arrived.
#[test]
#[ignore = "daily"]
fn a_campfire_lit_before_the_first_ticker_pass_ticks_behind_the_spawner() {
    if run_in_child("a_campfire_lit_before_the_first_ticker_pass_ticks_behind_the_spawner") {
        return;
    }
    with_remote_game(|game| {
        let campfire = campfire_json([1, 64, 1], false);
        let case = case_of(
            1000,
            [6.5, 64.0, 2.5],
            serde_json::json!([campfire, spawner_json([5, 64, 1])]),
            serde_json::json!([]),
        );
        start_case(game, &case, &mut false);
        let lit = client_tick_entity(game, &campfire_json([1, 64, 1], true)).0;
        game.world.set(1, 64, 1, lit).unwrap();
        let (mut spawner_seen, mut campfire_seen) = (0, 0);
        for _ in 0..60 {
            let events = tick_phase(game);
            let first_campfire = events.iter().position(|e| x_of(e) < 2.0);
            let last_spawner = events.iter().rposition(|e| x_of(e) > 5.0);
            campfire_seen += usize::from(first_campfire.is_some());
            spawner_seen += usize::from(last_spawner.is_some());
            if let (Some(c), Some(s)) = (first_campfire, last_spawner) {
                assert!(
                    s < c,
                    "the lit campfire ticked ahead of the spawner: {events:?}"
                );
            }
        }
        assert!(
            spawner_seen > 0 && campfire_seen > 0,
            "{spawner_seen} {campfire_seen}"
        );
    });
}

/// `PotentSulfurBlockEntity.setLevel` stamps the eruption clock as the block entity joins the
/// level, so a geyser received at 1000 whose first ticker pass is at 1005 plumes at 1020
/// (`eruptionTime % 20`), not at 1005.
#[test]
#[ignore = "daily"]
fn a_geyser_counts_its_eruption_from_the_tick_it_joined_the_level() {
    if run_in_child("a_geyser_counts_its_eruption_from_the_tick_it_joined_the_level") {
        return;
    }
    with_remote_game(|game| {
        let sulfur = serde_json::json!({ "kind": "potent_sulfur", "pos": [13, 64, 13],
            "state": "erupting" });
        let water = serde_json::json!([{ "from": [13, 65, 13], "to": [13, 67, 13],
            "block": "minecraft:water" }]);
        let case = case_of(1000, [6.5, 64.0, 2.5], serde_json::json!([sulfur]), water);
        start_case(game, &case, &mut false);
        game.world.game_time = 1005;
        let mut plumes = Vec::new();
        for time in 1005..1030 {
            if tick_phase(game).iter().any(|e| e[1] == "minecraft:geyser") {
                plumes.push(time);
            }
        }
        assert_eq!(plumes, [1020]);
    });
}

/// `DecoratedPotBlockEntity.triggerEvent`: wobble survives ticks, state changes and update
/// tags, but a new pot (`LevelChunk.setBlockState` or `replaceWithPacketData`) cannot inherit it.
#[test]
#[ignore = "daily"]
fn a_decorated_pots_wobble_survives_the_client_ticks() {
    if run_in_child("a_decorated_pots_wobble_survives_the_client_ticks") {
        return;
    }
    use sunrise_protocol::packets::play::{self, Clientbound as C};

    with_remote_game(|game| {
        let pos = [8, 64, 8];
        let fills = serde_json::json!([{
            "from": pos, "to": pos, "block": "minecraft:decorated_pot",
        }]);
        let case = case_of(1000, [6.5, 64.0, 2.5], serde_json::json!([]), fills);
        start_case(game, &case, &mut false);
        send_column(game, &[], (0, 0), &[], &case["fills"]);
        let remote = game.client.remote.as_mut().unwrap();
        Arc::make_mut(&mut remote.builtin).registries.insert(
            "minecraft:block".into(),
            sunrise_protocol::registry::Ids::from_names(vec!["minecraft:decorated_pot".into()]),
        );
        let event = || {
            C::BlockEvent(play::BlockEvent {
                pos,
                b0: 1,
                b1: 0,
                block: 0,
            })
        };
        game.apply_packet(event()).unwrap();
        for _ in 0..3 {
            tick_phase(game);
        }
        let wobble = game.block_anim(pos, 0.5).wobble;
        assert!(
            wobble.is_some_and(|(_, progress)| progress > 0.0),
            "{wobble:?}"
        );

        let registry = game.pack.assets.registry;
        let pot = registry.default_state("minecraft:decorated_pot").unwrap();
        let mut wet = registry.state(pot).unwrap().clone();
        wet.properties.insert("waterlogged".into(), "true".into());
        for state in [registry.id(&wet).unwrap(), pot] {
            game.apply_packet(C::BlockUpdate(play::BlockUpdate {
                pos,
                state: state.0 as i32,
            }))
            .unwrap();
            assert_eq!(game.block_anim(pos, 0.5).wobble, wobble);
        }
        game.apply_packet(C::BlockEntityData(play::BlockEntityData {
            pos,
            kind: TYPES
                .iter()
                .position(|&t| t == "minecraft:decorated_pot")
                .unwrap() as i32,
            tag: Tag::Compound(
                [(
                    "sherds".into(),
                    Tag::Compound(
                        [(
                            "back".into(),
                            Tag::String("minecraft:angler_pottery_sherd".into()),
                        )]
                        .into_iter()
                        .collect(),
                    ),
                )]
                .into_iter()
                .collect(),
            ),
        }))
        .unwrap();
        let anim = game.block_anim(pos, 0.5);
        assert_eq!(
            anim.sherds[0].as_deref(),
            Some("minecraft:angler_pottery_sherd")
        );
        assert_eq!(anim.wobble, wobble);

        for state in [game.world.air, pot] {
            game.apply_packet(C::BlockUpdate(play::BlockUpdate {
                pos,
                state: state.0 as i32,
            }))
            .unwrap();
        }
        assert_eq!(game.block_anim(pos, 0.5).wobble, None);
        for forget in [false, true] {
            game.apply_packet(event()).unwrap();
            assert!(game.block_anim(pos, 0.5).wobble.is_some());
            if forget {
                game.apply_packet(C::ForgetLevelChunk(play::ForgetLevelChunk { pos: [0, 0] }))
                    .unwrap();
            }
            send_column(game, &[], (0, 0), &[], &case["fills"]);
            assert_eq!(game.block_anim(pos, 0.5).wobble, None);
        }
    });
}

/// A conduit forgotten and sent again between two ticks is a new object (`ClientChunkCache.drop`,
/// `replaceWithPacketData`): it starts its animation from nothing.
#[test]
#[ignore = "daily"]
fn a_conduit_sent_again_between_ticks_starts_fresh() {
    if run_in_child("a_conduit_sent_again_between_ticks_starts_fresh") {
        return;
    }
    with_remote_game(|game| {
        let conduit = serde_json::json!({ "kind": "conduit", "pos": [8, 64, 8] });
        let case = case_of(
            1000,
            [6.5, 64.0, 2.5],
            serde_json::json!([conduit]),
            serde_json::json!([]),
        );
        let entities = start_case(game, &case, &mut false);
        for _ in 0..3 {
            tick_phase(game);
        }
        let pos = [8, 64, 8];
        assert_eq!(game.block_anim(pos, 0.5).conduit.0, 3);
        game.apply_packet(
            sunrise_protocol::packets::play::Clientbound::ForgetLevelChunk(
                sunrise_protocol::packets::play::ForgetLevelChunk { pos: [0, 0] },
            ),
        )
        .unwrap();
        send_column(game, &entities, (0, 0), &[0], &case["fills"]);
        assert_eq!(game.block_anim(pos, 0.5).conduit, (0, false, false, 0.0));
        tick_phase(game);
        assert_eq!(game.block_anim(pos, 0.5).conduit.0, 1);
    });
}

/// `ShulkerBoxBlockEntity.tick` pushes entities (`moveCollidedEntities`) in its own turn of
/// `Level.tickBlockEntities`: a table after it reads the player where that push left it
/// (`EnchantingTableBlockEntity.bookAnimationTick`, `getNearestPlayer`).
#[test]
#[ignore = "daily"]
fn a_book_after_a_shulker_lid_reads_the_pushed_player() {
    if run_in_child("a_book_after_a_shulker_lid_reads_the_pushed_player") {
        return;
    }
    with_remote_game(|game| {
        let shulker = serde_json::json!({ "kind": "shulker", "pos": [8, 64, 8], "facing": "east" });
        let table = serde_json::json!({ "kind": "enchanting_table", "pos": [9, 64, 10] });
        let case = case_of(
            1000,
            [9.3, 64.0, 9.15],
            serde_json::json!([shulker, table]),
            serde_json::json!([]),
        );
        start_case(game, &case, &mut false);
        game.block_entity_event([8, 64, 8], 1, 1);
        tick_phase(game);
        // VanillaBlockEntityClientTickOracle, "shulker pushes before spawner ticks".
        assert_eq!(
            game.block_anim([9, 64, 10], 0.5).book.t_rot.to_bits(),
            0xbfd1_9528
        );
    });
}

/// `Minecraft.tick` ticks the block entities, then the particle engine, whose tick ends by
/// adding the particles requested so far: the smoke a campfire made is in the engine at the end
/// of that same tick, not a tick later.
#[test]
#[ignore = "daily"]
fn a_campfires_smoke_joins_the_engine_in_the_tick_that_made_it() {
    if run_in_child("a_campfires_smoke_joins_the_engine_in_the_tick_that_made_it") {
        return;
    }
    with_remote_game(|game| {
        let case = case_of(
            1000,
            [6.5, 64.0, 8.5],
            serde_json::json!([campfire_json([8, 64, 8], true)]),
            serde_json::json!([]),
        );
        start_case(game, &case, &mut false);
        // The engine makes the particles, not the harness's log.
        game.client.particles.0.requested = None;
        game.client.remote.as_mut().unwrap().loaded_sent = true;
        for _ in 0..40 {
            game.tick_remote().unwrap();
            assert_eq!(
                game.client.particles.0.pending_len(),
                0,
                "particles wait a tick to join the engine"
            );
        }
        assert!(
            !game.client.particles.0.is_empty(),
            "the campfire made no smoke"
        );
    });
}
