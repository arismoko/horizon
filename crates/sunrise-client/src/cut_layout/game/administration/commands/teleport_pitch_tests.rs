use crate::game::load_resources;
use crate::settings::Settings;
use sunrise_gameplay_common::game::commands_common::Target;
use sunrise_gameplay_common::game::{Options, Progress};

#[test]
#[ignore = "daily"]
fn teleport_and_rotate_resolve_pitch_at_their_own_boundary() {
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
    let kind = game
        .res
        .mob_types()
        .get("minecraft:zombie")
        .unwrap()
        .clone();
    game.mobs.push(sunrise_sim::mobs::Mob::new(
        42,
        kind,
        sunrise_core::geometry::Vec3::new(0.0, 0.0, 0.0),
        0.0,
    ));
    for (rotation, expected) in [
        ([40.0, f32::NAN], [40.0, 20.0]),
        ([f32::INFINITY, 30.0], [10.0, 30.0]),
        ([40.0, f32::INFINITY], [40.0, 90.0]),
        ([40.0, f32::NEG_INFINITY], [40.0, -90.0]),
    ] {
        game.yaw = 10.0;
        game.pitch = 20.0;
        game.teleport_to_in_level(Target::Player, [0.0; 3], Some(rotation), [false; 3], true);
        assert_eq!([game.yaw, game.pitch], expected);
        game.mobs[0].yaw = 10.0;
        game.mobs[0].head_pitch = 20.0;
        game.teleport_to_in_level(Target::Mob(42), [0.0; 3], Some(rotation), [false; 3], true);
        assert_eq!([game.mobs[0].yaw, game.mobs[0].head_pitch], expected);
    }
    game.build_registries();
    game.set_allow_commands(true);
    for (command, expected) in [
        ("rotate @e[type=minecraft:zombie,limit=1] 0 360", 90.0),
        ("tp @e[type=minecraft:zombie,limit=1] 0 0 0 0 360", 0.0),
        ("tp @e[type=minecraft:zombie,limit=1] 0 0 0 0 270", -90.0),
    ] {
        assert!(game.run_command(command), "{command}");
        assert_eq!(game.mobs[0].head_pitch, expected, "{command}");
    }
    for (source, target, angle, expected) in [
        (90.0, 90.0, "~100", 90.0),
        (-90.0, -90.0, "~-100", -90.0),
        (0.0, -90.0, "~100", -90.0),
        (0.0, 90.0, "~-100", 90.0),
    ] {
        game.pitch = source;
        game.mobs[0].head_pitch = target;
        assert!(game.run_command(&format!(
            "tp @e[type=minecraft:zombie,limit=1] 0 0 0 0 {angle}"
        )));
        assert_eq!(game.mobs[0].head_pitch, expected);
    }
    game.yaw = 190.0;
    game.mobs[0].yaw = 170.0;
    assert!(game.run_command("tp @e[type=minecraft:zombie,limit=1] 0 0 0 ~ ~"));
    assert_eq!(game.mobs[0].yaw, 190.0, "relative yaw wraps the delta");

    game.mobs[0].head_pitch = 360.0;
    assert!(
        game.run_command("tp @e[type=minecraft:zombie,limit=1] @e[type=minecraft:zombie,limit=1]")
    );
    assert_eq!(
        game.mobs[0].head_pitch, 0.0,
        "destination rotation wraps too"
    );

    let overflow = "9".repeat(400);
    for (pitch, expected) in [(overflow.clone(), 90.0), (format!("-{overflow}"), -90.0)] {
        assert!(game.run_command(&format!(
            "rotate @e[type=minecraft:zombie,limit=1] 0 {pitch}"
        )));
        assert_eq!(game.mobs[0].head_pitch, expected);
    }
    for (yaw, pitch, expected) in [
        ("40", overflow.as_str(), [40.0, 20.0]),
        (overflow.as_str(), "30", [10.0, 30.0]),
    ] {
        game.mobs[0].yaw = 10.0;
        game.mobs[0].head_pitch = 20.0;
        assert!(game.run_command(&format!(
            "tp @e[type=minecraft:zombie,limit=1] 0 0 0 {yaw} {pitch}"
        )));
        assert_eq!([game.mobs[0].yaw, game.mobs[0].head_pitch], expected);
    }
}

#[test]
#[ignore = "daily"]
fn teleport_to_the_current_position_marks_the_player_for_sync() {
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_sim::motion::PlayerMotion;

    with_game(|game| {
        let at = Vec3::new(0.5, 64.0, 0.5);
        game.player = Some(PlayerMotion::new(game.motion, at).unwrap());

        game.teleport_to_in_level(Target::Player, [at.x, at.y, at.z], None, [false; 3], false);

        assert!(game.player.as_ref().is_some_and(|p| p.teleported));
    });
}

#[test]
fn pitch_wraps_before_clamping() {
    assert_eq!(
        sunrise_gameplay_common::game::commands_common::entity_pitch(360.0),
        0.0
    );
    assert_eq!(
        sunrise_gameplay_common::game::commands_common::entity_pitch(450.0),
        90.0
    );
    assert_eq!(
        sunrise_gameplay_common::game::commands_common::entity_pitch(-360.0),
        0.0
    );
    assert_eq!(
        sunrise_gameplay_common::game::commands_common::entity_pitch(-450.0),
        -90.0
    );
    assert_eq!(
        sunrise_gameplay_common::game::commands_common::entity_pitch(
            sunrise_gameplay_common::game::commands_common::wrap_degrees(270.0)
        ),
        -90.0
    );
    assert_eq!(
        sunrise_gameplay_common::game::commands_common::entity_pitch(
            sunrise_gameplay_common::game::commands_common::wrap_degrees(-270.0)
        ),
        90.0
    );
    assert_eq!(
        sunrise_gameplay_common::game::commands_common::entity_pitch(
            sunrise_gameplay_common::game::commands_common::wrap_degrees(360.0)
        ),
        0.0
    );
    assert_eq!(
        sunrise_gameplay_common::game::commands_common::entity_pitch(
            sunrise_gameplay_common::game::commands_common::wrap_degrees(450.0)
        ),
        90.0
    );
    assert_eq!(
        sunrise_gameplay_common::game::commands_common::wrap_degrees(190.0),
        -170.0
    );
    assert_eq!(
        sunrise_gameplay_common::game::commands_common::wrap_degrees(540.0),
        -180.0
    );
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
