//! `/particle` hands each type's options to the client draw: `ParticleArgument`
//! parses the SNBT and `ParticleTypes.CODEC` (the type's own options codec)
//! turns it into what `ParticleProvider.createParticle` reads — a block
//! crumble's block state, a dust transition's colours and scale, a spell's
//! colour and power, a sculk charge's roll, a vibration's destination.
#[cfg(feature = "client")]
use mod_api::client::particles::PositionSource;
use std::sync::Arc;
use sunrise_core::geometry::Vec3;
#[cfg(feature = "client")]
use sunrise_level::particle_options::ParticleType;
use {
    crate::game::resource_load::load_resources, sunrise_gameplay_common::game::execution_role,
    sunrise_gameplay_common::game::launch_options::Options,
    sunrise_gameplay_common::game::resource_load::Progress,
};
use {
    sunrise_client::settings::Settings, sunrise_level::chunk::Chunk,
    sunrise_sim::motion::PlayerMotion,
};

pub(crate) fn game<'a>(
    resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
) -> sunrise_gameplay::game::ServerGame<'a> {
    let root = std::path::PathBuf::from(
        std::env::var_os("SUNRISE_TEST_RESOURCES")
            .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        // A window's resources: a headless load parses no models, so it has no particle sprites.
        #[cfg(feature = "client")]
        crate::game::execution_role::ExecutionRole::Client,
        #[cfg(not(feature = "client"))]
        crate::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
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
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("fill 0 99 0 15 99 15 stone"));
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 100.0, 0.5)).unwrap());
    game
}

#[cfg(feature = "client")]
#[allow(unused_imports)]
pub(crate) use crate::game::client_side::commands_client::world_cmds::particle_command_tests::drawn;

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn particle_options_carry_their_codec_data() {
    let mut resources = None;
    let mut game = game(&mut resources);
    // BlockParticleOption: `block_crumble` is the crumbs of the named block.
    let crumble = drawn(
        &mut game,
        "particle minecraft:block_crumble{block_state:\"minecraft:stone\"} 0.5 100.5 0.5 0 0 0 0 1",
    );
    assert_eq!(crumble.ty, ParticleType::BlockCrumble);
    let (sprite, _) = crumble.block.expect("the crumbs carry their block");
    assert_eq!(sprite, "block/stone");
    // DustColorTransitionOptions: `from_color`, `to_color` and `scale`.
    let transition = drawn(
        &mut game,
        "particle minecraft:dust_color_transition{from_color:0,to_color:0,scale:2} 0.5 100.5 0.5 0 0 0 0 1",
    );
    assert_eq!(transition.ty, ParticleType::DustColorTransition);
    assert_eq!(transition.color[..3], [0.0; 3]);
    assert_eq!(transition.to_color, [0.0; 3]);
    assert_eq!(transition.scale, 2.0);
    // SpellParticleOption: its colour and power.
    let effect = drawn(
        &mut game,
        "particle minecraft:effect{color:16711680,power:2} 0.5 100.5 0.5 0 0 0 0 1",
    );
    assert_eq!(effect.ty, ParticleType::Effect);
    assert_eq!(effect.color[..3], [1.0, 0.0, 0.0]);
    assert_eq!(effect.scale, 2.0);
    // SculkChargeParticleOptions.roll.
    let charge = drawn(
        &mut game,
        "particle minecraft:sculk_charge{roll:0.5} 0.5 100.5 0.5 0 0 0 0 1",
    );
    assert_eq!(charge.ty, ParticleType::SculkCharge);
    assert_eq!(charge.scale, 0.5);
    // VibrationParticleOption: its block destination and arrival ticks.
    let vibration = drawn(
        &mut game,
        "particle minecraft:vibration{destination:{type:\"minecraft:block\",pos:[0,64,0]},arrival_in_ticks:20} 0.5 100.5 0.5 0 0 0 0 1",
    );
    assert_eq!(vibration.ty, ParticleType::Vibration);
    assert_eq!(
        vibration.vibration,
        Some((PositionSource::Block([0, 64, 0]), [0.5, 64.5, 0.5], 20))
    );
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn typed_particle_commands_suggest_and_execute_the_decoded_value() {
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        "cut_layout::game::server_side::commands::world_cmds::particle_command_tests::typed_particle_commands_suggest_and_execute_the_decoded_value",
        "SUNRISE_TEST_TYPED_PARTICLE_COMMAND_CASE",
        &["typed"],
    ) {
        return;
    }
    let entries = [mod_api::ModEntry::new(
        "test",
        Box::new(sunrise_gameplay::game::lan_particles::tests::MeasuredParticle),
    )];
    mod_platform::__private::part_tests::install_parts_fixture(&entries).unwrap();
    sunrise_gameplay_common::game::mods::resolve_particle_providers(&Default::default()).unwrap();
    let mut resources = None;
    let mut game = game(&mut resources);
    let reg = &game.ui.registries;
    let input = "particle test:meas";
    let dispatcher = sunrise_command::vanilla::dispatcher();
    let (_, suggestions) = dispatcher.suggestions(&dispatcher.parse(input, reg), input.len(), reg);
    assert!(
        suggestions
            .iter()
            .any(|suggestion| suggestion.text == "test:measured")
    );
    for invalid in ["test:measured", "test:measured{measure:\"large\"}"] {
        let error = sunrise_command::args::ArgType::Particle
            .parse(&mut sunrise_command::Reader::new(invalid), reg)
            .unwrap_err();
        assert_eq!(error.key, "particle.invalidOptions");
    }
    let measured = drawn(
        &mut game,
        "particle test:measured{measure:257} 0.5 100.5 0.5 0 0 0 0 1",
    );
    let (value, _) = measured.typed_particle.expect("typed particle requested");
    assert_eq!(
        mod_api::__private::encode_particle_stream(&value, 8).unwrap(),
        [1, 1, 0, 0]
    );
}

#[test]
#[ignore = "daily"]
fn typed_particle_command_encoder_refusal_does_not_deliver_to_a_guest() {
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        "cut_layout::game::server_side::commands::world_cmds::particle_command_tests::typed_particle_command_encoder_refusal_does_not_deliver_to_a_guest",
        "SUNRISE_TEST_TYPED_PARTICLE_COMMAND_CASE",
        &["refusal"],
    ) {
        return;
    }
    use {
        sunrise_gameplay::game::lan_particles::tests::MeasuredParticle,
        sunrise_gameplay::game::server_side::lan_particles::tests::test_hub,
    };
    let entries = [mod_api::ModEntry::new("test", Box::new(MeasuredParticle))];
    mod_platform::__private::part_tests::install_parts_fixture(&entries).unwrap();
    sunrise_gameplay_common::game::mods::resolve_particle_providers(&Default::default()).unwrap();
    let mut resources = None;
    let mut game = game(&mut resources);
    let (mut hub, receiver, _) = test_hub("CommandParticleOptions", Vec::new(), None);
    hub.guests.update_all(|g| {
        if g.conn == 1 {
            g.brand = "sunrise".into();
            g.position = [0.5, 100.5, 0.5];
        }
    });
    game.server.lan = Some(sunrise_gameplay::game::lan::LanHost::for_test(hub));
    game.server.lan_acting = Some(1);
    assert!(game.run_command("particle test:measured{measure:1} 0.5 100.5 0.5 0 0 0 0 1 force @s"));
    assert!(game.server.guest_packets.iter().any(|p| matches!(
        p,
        sunrise_protocol::packets::play::Clientbound::LevelParticles(_)
    )));
    game.server.guest_packets.clear();
    game.server.pending_particles.clear();
    let (ok, outcome) = game.recording_command_outcome(|game| {
        game.run_command("particle test:measured{measure:-1} 0.5 100.5 0.5 0 0 0 0 1 force @s")
    });
    assert!(!ok);
    assert_eq!(outcome, Some((false, 0)));
    assert!(game.server.guest_packets.iter().all(|p| !matches!(
        p,
        sunrise_protocol::packets::play::Clientbound::LevelParticles(_)
    )));
    assert!(receiver.try_recv().is_err());
    assert!(game.server.pending_particles.is_empty());
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
