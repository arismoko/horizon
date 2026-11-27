//! Real Game raid creation/wave dispatch with a loaded claimed-bed precondition.
use crate::game::{Options, Progress, Settings, load_resources};
use mod_api::{Authority, LevelReadError, LevelRef, WorldEpoch};
use std::sync::Arc;
use sunrise_core::{geometry::Vec3, position::ChunkPos};
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_level::chunk::Chunk;
use sunrise_sim::{mobs::Mob, raid::Status};

#[test]
#[ignore = "daily"]
fn admitted_raid_loses_removed_loaded_bed_without_erasing_memory() {
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role =
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
    let mut game = sunrise_gameplay::game::ServerGame::new(
        &loaded.resources,
        loaded.pack,
        options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    game.server.world_root =
        std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap()).join("raid-unit-world");
    for x in -5..5 {
        for z in -5..5 {
            game.world
                .chunks
                .insert((x, z), Arc::new(Chunk::empty(x, z)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(x, z))
                .unwrap();
        }
    }
    let registry = game.pack.assets.registry;
    let air = registry.default_state("minecraft:air").unwrap();
    let stone = registry.default_state("minecraft:stone").unwrap();
    for x in -63..=63 {
        for z in -63..=63 {
            game.world.set(x, 199, z, stone).unwrap();
        }
    }
    let foot = registry.default_state("minecraft:white_bed").unwrap();
    let mut head_state = registry.state(foot).unwrap().clone();
    head_state.properties.insert("part".into(), "head".into());
    let head = registry.id(&head_state).unwrap();
    game.world.set(2, 200, 2, foot).unwrap();
    game.world.set(2, 200, 1, head).unwrap();
    game.native_lifecycle
        .begin_open(Some(Authority::IntegratedServer));
    let level = game.level_ref(game.native_lifecycle.epoch().unwrap(), game.dim.key());
    let bed = [2, 200, 1];
    assert_eq!(level.is_village(&game, bed), Ok(false));
    assert_eq!(
        LevelRef::new(WorldEpoch::new(0), game.dim.key()).is_village(&game, bed),
        Err(LevelReadError::Stale)
    );
    let kind = game
        .res
        .mob_types()
        .get("minecraft:villager")
        .unwrap()
        .clone();
    let mut villager = Mob::new(162, kind, Vec3::new(3.5, 200.0, 2.5), 0.0);
    villager.no_ai = true;
    // The full Game/Java fixture proves real AI acquisition; this unit starts
    // from that claimed-bed state and exercises production raid/wave dispatch.
    villager.extra.bed = Some([2, 200, 1]);
    game.mobs.push(villager);
    assert_eq!(level.is_village(&game, bed), Ok(true));
    assert_eq!(level.is_village(&game, [18, 200, 1]), Ok(true));
    assert_eq!(level.is_village(&game, [34, 200, 1]), Ok(false));
    assert!(game.raid_command_create([8, 200, 8], 1));
    let id = game.server.raiding.0.raids.ids()[0];
    for _ in 0..400 {
        game.world.game_time += 1;
        game.tick_raids();
        if game.server.raiding.0.raids.get(id).unwrap().groups_spawned == 1 {
            break;
        }
    }
    let raid = game.server.raiding.0.raids.get(id).unwrap();
    assert!(raid.started);
    assert_eq!(raid.groups_spawned, 1, "real first wave must be admitted");
    assert!(
        game.raid_members(id).next().is_some(),
        "real wave members required"
    );
    for _ in 0..20 {
        game.tick_raids();
    }
    assert_eq!(
        game.server.raiding.0.raids.get(id).unwrap().status,
        Status::Ongoing
    );
    // Unavailable claims remain unknown rather than becoming known air.
    let chunk = game.world.chunks.remove(&(0, 0)).unwrap();
    assert!(sunrise_gameplay::game::ServerGame::is_village(
        &game.village_centers(),
        [2, 200, 1]
    ));
    assert_eq!(
        level.is_village(&game, bed),
        Err(LevelReadError::Unavailable)
    );
    assert!(!game.world.chunks.contains_key(&(0, 0)));
    game.world.chunks.insert((0, 0), chunk);
    game.level_set([2, 200, 1], air).unwrap();
    game.level_set([2, 200, 2], air).unwrap();
    assert_eq!(game.world.block(2, 200, 1), air);
    assert_eq!(game.world.block(2, 200, 2), air);
    game.tick_raids();
    assert_eq!(level.is_village(&game, bed), Ok(false));
    assert_eq!(
        game.server.raiding.0.raids.get(id).unwrap().status,
        Status::Loss
    );
    assert_eq!(
        game.mobs.iter().find(|m| m.id == 162).unwrap().extra.bed,
        Some([2, 200, 1])
    );
}
