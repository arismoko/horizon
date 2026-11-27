//! The bursts' sound and silence: the breeze's sound for a Wind Charged mob's
//! body (`WindChargedMobEffect.onMobRemoved`) and for a silent source no sound
//! at all (`ServerLevel.explode`'s `playSound`), and vehicles left alone by a
//! mob's blast with mob griefing off (`VehicleEntity.ignoreExplosion`).
use crate::game::load_resources;
use crate::settings::Settings;
use std::sync::Arc;
use sunrise_gameplay_common::game::commands_common::Target;
use sunrise_level::chunk::Chunk;
use sunrise_world::food::Difficulty;
use {sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress};

pub(crate) const BREEZE_BURST: &str = "minecraft:entity.breeze.wind_burst";
#[cfg(feature = "client")]
pub(crate) const WIND_CHARGE_BURST: &str = "minecraft:entity.wind_charge.wind_burst";

/// A windowless integrated-server game on normal difficulty (a level.dat would
/// set it; the default would discard a zombie as Peaceful does) with chunk
/// (0, 0) loaded and a stone floor at y 63, which a dead mob falls onto so
/// that its burst has ground under it. `None` without the prepared resources.
pub(crate) fn with_game<R>(
    f: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>) -> R,
) -> Option<R> {
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES")?);
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
    .ok()?;
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role =
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
    let mut resources = None;
    let res = &*resources.insert(loaded.resources);
    let mut game = sunrise_gameplay::game::ServerGame::new(
        res,
        loaded.pack,
        options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    game.options.dedicated = Some(Vec::new());
    game.server.audio_capture_active = true;
    game.build_registries();
    game.set_allow_commands(true);
    game.world.health_rules.difficulty = Difficulty::Normal;
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.server.streaming.forced.insert((0, 0));
    assert!(game.run_command("fill 0 63 0 15 63 15 stone"));
    Some(f(&mut game))
}

/// How many times `sound` was queued as a server sound.
pub(crate) fn heard(game: &sunrise_gameplay::game::ServerGame<'_>, sound: &str) -> u32 {
    game.server
        .pending_sounds
        .iter()
        .filter(|s| s.event == sound)
        .count() as u32
}

/// An oak boat two blocks east of the burst's centre.
pub(crate) fn boat(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> u32 {
    match game
        .summon_with_nbt("minecraft:oak_boat", [6.5, 64.0, 4.5], "{}")
        .unwrap()
    {
        Some(Target::Boat(id)) => id,
        other => panic!("no boat: {other:?}"),
    }
}

pub(crate) fn boat_speed(game: &sunrise_gameplay::game::ServerGame<'_>, id: u32) -> f64 {
    let v = game.boats.iter().find(|b| b.id == id).unwrap().velocity;
    (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
}

/// A zombie at (4.5, 64, 4.5) with the Wind Charged effect, `Silent` or not,
/// killed and ticked out: `LivingEntity.remove` with KILLED runs its effects
/// after `LivingEntity.tickDeath`'s twenty ticks.
pub(crate) fn a_wind_charged_zombie_dies(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    silent: bool,
) {
    let nbt = if silent { "{Silent:1b}" } else { "{}" };
    game.summon_with_nbt("minecraft:zombie", [4.5, 64.0, 4.5], nbt)
        .unwrap()
        .expect("summoned zombie");
    assert!(
        game.run_command(
            "effect give @e[type=minecraft:zombie,limit=1] minecraft:wind_charged 600"
        ),
        "the effect went on the zombie"
    );
    assert!(game.run_command("kill @e[type=minecraft:zombie,limit=1]"));
    for _ in 0..25 {
        game.tick_creatures().unwrap();
    }
}

/// `VehicleEntity.ignoreExplosion` skips a whole blast for a boat or a
/// minecart when its indirect source is a mob and `mobGriefing` is off, so a
/// Wind Charged mob's death leaves a boat alone. The mob is the source, not a
/// breeze's charge (`ServerExplosion.canTriggerBlocks`), so the blocks that
/// react still do.
#[test]
#[ignore = "daily"]
fn a_mobs_burst_leaves_vehicles_alone_with_mob_griefing_off() {
    let Some(()) = with_game(|game| {
        assert!(game.run_command("gamerule mob_griefing false"));
        assert!(
            !game
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::MOB_GRIEFING)
        );
        // The rule change re-derives the difficulty from the game's
        // (`Game::sync_rules`), which no level.dat set here.
        game.world.health_rules.difficulty = Difficulty::Normal;
        // The block the burst's centre sits in is always one of the exploded
        // positions, so this door is one the blast reaches.
        assert!(game.run_command(
            "setblock 4 64 4 oak_door[half=lower,facing=north,hinge=left,open=false,powered=false]"
        ));
        let door = game.world.block(4, 64, 4);
        let boat = boat(game);
        a_wind_charged_zombie_dies(game, false);
        assert_eq!(boat_speed(game, boat), 0.0, "the boat was left alone");
        assert_ne!(
            game.world.block(4, 64, 4),
            door,
            "a mob's burst still opens doors with mob griefing off"
        );
        assert_eq!(heard(game, BREEZE_BURST), 1);
    }) else {
        return;
    };
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
