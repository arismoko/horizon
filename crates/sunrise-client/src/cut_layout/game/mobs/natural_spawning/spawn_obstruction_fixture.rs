use std::fmt::Write;
use sunrise_client_lib::mobs::spawn_selection::NaturalLevel;
use sunrise_core::geometry::Aabb;
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;
use sunrise_core::nbt::parse_snbt;
use sunrise_core::position::BlockPos;
use sunrise_core::random::LegacyRandom;
use sunrise_core::random::RandomSource;
use sunrise_core::random::{InvalidBound, PositionalRandom};
use sunrise_gameplay::game::server_side::natural_spawning::GameLevel;
use sunrise_gameplay::game::server_side::natural_spawning::{PassLevel, SCANNED_QUERIES};
use sunrise_sim::mobs::Species;
#[cfg(test)]
use sunrise_sim::mobs::spawn_selection;
use sunrise_sim::mobs::{
    finalize::FinalizeLevel as _,
    spawner::{Spawner, SpawnerEvent, SpawnerExt, SpawnerLevel, SpawnerSpawn},
};

/// Two attempts at y=64 with a common z and a specified x displacement.
/// No other draws should occur with this configured spawner and fixed delay.
#[derive(Clone)]
struct Attempts {
    doubles: std::array::IntoIter<f64, 8>,
    heights: usize,
    yaws: usize,
}

impl RandomSource for Attempts {
    type Positional = Self;
    fn next_double(&mut self) -> f64 {
        self.doubles.next().expect("spawn position draw")
    }
    fn next_int_bound(&mut self, bound: i32) -> Result<i32, InvalidBound> {
        assert_eq!(bound, 3);
        self.heights += 1;
        Ok(1)
    }
    fn next_float(&mut self) -> f32 {
        self.yaws += 1;
        0.75
    }
    fn set_seed(&mut self, _: i64) {
        unreachable!()
    }
    fn next_int(&mut self) -> i32 {
        unreachable!()
    }
    fn next_long(&mut self) -> i64 {
        unreachable!()
    }
    fn next_boolean(&mut self) -> bool {
        unreachable!()
    }
    fn next_gaussian(&mut self) -> f64 {
        unreachable!()
    }
    fn fork_positional(&mut self) -> Self {
        unreachable!()
    }
    fn fork(&mut self) -> Self {
        unreachable!()
    }
}

impl PositionalRandom for Attempts {
    type Source = Self;
    fn at(&self, _: BlockPos) -> Self {
        unreachable!()
    }
    fn from_seed(&self, _: i64) -> Self {
        unreachable!()
    }
    fn from_hash_of(&self, _: &str) -> Self {
        unreachable!()
    }
    fn from_hash_of_utf16(&self, _: &[u16]) -> Self {
        unreachable!()
    }
}

/// A registered, non-colliding relay with vanilla's item display as the
/// oracle's equivalent. Scope the declarations to this child fixture.
struct DeclareRelay;

#[mod_api::__subscriptions]
impl mod_api::Hooks for DeclareRelay {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        r.declare_general_entity(
            "relay",
            mod_api::GeneralEntityCategory::Misc,
            0.1,
            0.1,
            None,
            mod_api::GeneralSavePolicy::Save,
            false,
            false,
            false,
            None,
            8,
            3,
            1,
            &[],
        )
        .unwrap();
    }
}

fn admission_queries(level: &GameLevel<'_, '_>, z: f64) -> String {
    // Beyond the root zombie's box, but inside a shulker's box. Count all
    // shulkers in the two-attempt area (not the distant resident control).
    let pig = level.game.res.mob_types().get("minecraft:pig").unwrap();
    let probe = spawn_selection::spawn_aabb(
        Species::Pig,
        9.35,
        64.0,
        z,
        pig.width as f32,
        pig.height as f32,
    );
    let nearby = Aabb::new(
        Vec3::new(7.0, 63.0, z - 1.0),
        Vec3::new(11.0, 66.0, z + 1.0),
    );
    let clear = level.no_collision(probe);
    assert_eq!(PassLevel::new(level).no_collision(probe), clear);
    format!(
        "{clear}:{}:{}",
        level.unobstructed(probe),
        level.count_entities("minecraft:shulker", nearby)
    )
}

fn spawner_batch(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    z: f64,
    dx: f64,
    id: &str,
    nbt: &str,
    installation: &str,
) -> (String, String) {
    let (Tag::Compound(mut entity), _) = parse_snbt(nbt).unwrap() else {
        panic!("expected a compound");
    };
    entity.insert("id".into(), Tag::String(id.into()));
    let mut spawner = Spawner::load(&parse_snbt(concat!(
        "{Delay:0s,SpawnCount:2s,SpawnRange:1s,MaxNearbyEntities:8s,",
        "MinSpawnDelay:20s,MaxSpawnDelay:20s,SpawnPotentials:[],SpawnData:{custom_spawn_rules:{},entity:{id:\"minecraft:pig\"}}}"
    )).unwrap().0);
    spawner.next_data_mut().unwrap().entity = entity;
    if installation == "admission-resident" {
        assert!(
            game.summon_with_nbt("minecraft:shulker", [2.5, 64.0, z], "{UUID:[I;0,0,0,101]}")
                .unwrap()
                .is_some()
        );
    }
    if installation == "admission-intra" {
        spawner.spawn_count = 1;
    }
    game.player =
        Some(sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(4.5, 64.0, z)).unwrap());
    let level = game.natural_level();
    let mut random = Attempts {
        doubles: [0.0, 0.0, 0.0, 0.0, dx, 0.0, 0.0, 0.0].into_iter(),
        heights: 0,
        yaws: 0,
    };
    let mut own = LegacyRandom::new(0);
    let mut sequences = Default::default();
    let mut planned = Vec::new();
    let mut spawn = |l: &GameLevel, _: &mut Attempts, r: &SpawnerSpawn| {
        assert!(!r.finalize);
        game.plan_spawner_mob(l, r, None, &mut planned, &mut own, &mut sequences)
    };
    let mut events = Vec::new();
    spawner.server_tick(
        &level,
        &mut random,
        [8, 64, z as i32],
        &mut spawn,
        &mut events,
    );
    assert_eq!(random.doubles.len(), 8 - 4 * random.heights);
    if installation == "batch" {
        assert_eq!(random.heights, 2);
        return (planned.len().to_string(), random.yaws.to_string());
    }
    let preview = admission_queries(&level, z);
    let created = level.spawned.take();
    let rides = level.rides.take();
    drop(level);
    let mob_start = game.mobs.len();
    let general_start = game.resident_entities.general.rows.len();
    let roots = game.add_planned(planned, created, rides);
    let mut installed: Vec<_> = game
        .mobs
        .iter()
        .skip(mob_start)
        .map(|mob| (mob.id, mob.kind.id.strip_prefix("minecraft:").unwrap()))
        .chain(
            game.resident_entities
                .general
                .rows
                .iter()
                .skip(general_start)
                .map(|row| (row.id, "relay")),
        )
        .collect();
    installed.sort_unstable_by_key(|&(id, _)| id);
    let installed = if installed.is_empty() {
        "-".into()
    } else {
        installed
            .iter()
            .map(|&(_, kind)| kind)
            .collect::<Vec<_>>()
            .join(",")
    };
    let mut effects = [0; 4];
    for event in events {
        match event {
            SpawnerEvent::LevelEvent(2004) => effects[0] += 1,
            SpawnerEvent::EntityPlace(key, _) => {
                assert!(roots.contains_key(&key));
                effects[1] += 1;
            }
            SpawnerEvent::SpawnAnim(key) => {
                assert!(roots.contains_key(&key));
                effects[2] += 1;
            }
            SpawnerEvent::BlockEvent(1) => effects[3] += 1,
            _ => {}
        }
    }
    let actual = admission_queries(&game.natural_level(), z);
    (
        format!(
            "{}:{}:{}:{}:{}:{}:{}:{}:{installed}",
            roots.len(),
            random.heights,
            random.yaws,
            spawner.spawn_delay,
            effects[0],
            effects[1],
            effects[2],
            effects[3]
        ),
        format!("{preview}/{actual}"),
    )
}

fn with_relay(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    mod_platform::__private::initialize().unwrap();
    let _phase = mod_platform::__private::RegistrationGuard::open();
    let mods = vec![mod_api::ModEntry::new(
        "spawn_fixture",
        Box::new(DeclareRelay),
    )];
    mod_platform::testing::with_mods(mods.into(), || {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(test);
    });
}

#[test]
#[ignore = "daily"]
fn spawn_entity_obstruction() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::natural_spawning::spawn_obstruction_fixture::spawn_entity_obstruction",
        || {
            with_relay(|game| {
                let cases = include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/spawn_obstruction.tsv"
                ));
                let mut output = String::new();
                let mut failures = Vec::new();
                for (i, line) in cases
                    .lines()
                    .filter(|line| !line.starts_with('#'))
                    .enumerate()
                {
                    let [name, installation, id, dx, data, clear, unobstructed] = line
                        .split_ascii_whitespace()
                        .collect::<Vec<_>>()
                        .try_into()
                        .unwrap();
                    let z = 8.5 + 16.0 * i as f64;
                    let dx = dx.parse::<f64>().unwrap();
                    let at = [8.5 + dx, 64.0, z];
                    game.world.chunks.insert(
                        (0, i as i32),
                        std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, i as i32)),
                    );
                    if installation == "batch" || installation.starts_with("admission-") {
                        let (first, second) = spawner_batch(game, z, dx, id, data, installation);
                        let mode = if installation == "batch" {
                            "batch"
                        } else {
                            "admission"
                        };
                        writeln!(output, "{name} {mode} {first} {second}").unwrap();
                        if (first.as_str(), second.as_str()) != (clear, unobstructed) {
                            failures.push(name);
                        }
                        continue;
                    }
                    if installation == "falling" {
                        let state = game
                            .pack
                            .assets
                            .registry
                            .default_state("minecraft:sand")
                            .unwrap();
                        game.level_events
                            .push(sunrise_world::level::LevelEvent::FallingBlock {
                                pos: BlockPos::new(8, 64, z as i32),
                                state,
                                hurt_per_distance: None,
                            });
                        game.server_process_level_events().unwrap();
                    } else if id == "minecraft:ender_dragon" {
                        // Game stores one dragon; install each independent case anew.
                        game.discard_resident_dragon();
                        game.spawn_dragon_facing(
                            at,
                            sunrise_gameplay_common::game::creatures::SpawnSite::Raw,
                            Some(0.0),
                        )
                        .expect(name);
                        if installation == "dragon-part" {
                            game.dragon.as_mut().unwrap().parts[0].0 = Vec3::new(8.5, 64.0, z);
                        }
                    } else if matches!(installation, "live" | "removed") {
                        let summoned = game.summon_with_nbt(id, at, data);
                        assert!(matches!(summoned, Ok(Some(_))), "failed to summon {name}");
                        if installation == "removed" {
                            let mob = game.mobs.last_mut().unwrap();
                            assert_eq!(mob.kind.id, id);
                            mob.removed = true;
                        }
                    }
                    let level = game.natural_level();
                    if matches!(installation, "pending" | "pending-riders") {
                        let (Tag::Compound(mut entity), _) = parse_snbt(data).unwrap() else {
                            panic!("{name}: expected a compound");
                        };
                        entity.insert("id".into(), Tag::String(id.into()));
                        let request = SpawnerSpawn {
                            entity: &entity,
                            id,
                            position: at,
                            yaw: 0.0,
                            finalize: false,
                            equipment: None,
                            trial: false,
                        };
                        game.plan_spawner_mob(
                            &level,
                            &request,
                            None,
                            &mut Vec::new(),
                            &mut LegacyRandom::new(0),
                            &mut Default::default(),
                        )
                        .expect(name);
                    }
                    let pig = game.res.mob_types().get("minecraft:pig").unwrap();
                    // At this offset a pig meets the shulker, but not either zombie.
                    let x = if installation == "pending-riders" {
                        8.625
                    } else {
                        8.5
                    };
                    let bounds = spawn_selection::spawn_aabb(
                        Species::Pig,
                        x,
                        64.0,
                        z,
                        pig.width as f32,
                        pig.height as f32,
                    );
                    let collision = level.no_collision(bounds);
                    let obstruction = spawn_selection::spawn_obstruction(
                        Species::Pig,
                        &level,
                        bounds,
                        true,
                        8,
                        64,
                        z as i32,
                    );
                    let finalize = PassLevel::new(&level).no_collision(bounds);
                    writeln!(output, "{name} obstruction {collision} {obstruction}").unwrap();
                    if collision != clear.parse::<bool>().unwrap()
                        || obstruction != unobstructed.parse::<bool>().unwrap()
                        || finalize != collision
                    {
                        failures.push(name);
                    }
                    // The lazy grid must give the same answers as the initial scan.
                    for _ in 0..SCANNED_QUERIES {
                        assert_eq!(level.no_collision(bounds), collision, "{name}");
                        assert_eq!(level.unobstructed(bounds), obstruction, "{name}");
                    }
                }
                if let Some(path) = std::env::var_os("SUNRISE_SPAWN_OBSTRUCTION_RECORDING") {
                    std::fs::write(path, output).unwrap();
                }
                assert!(
                    failures.is_empty(),
                    "different spawn decisions: {failures:?}"
                );
            });
        },
    );
}
