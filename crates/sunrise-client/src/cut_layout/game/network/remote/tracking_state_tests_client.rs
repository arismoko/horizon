#[test]
#[ignore = "daily"]
fn tracking_emitters_follow_the_packet_target() {
    use crate::game::load_resources;
    use crate::settings::Settings;
    use sunrise_client_lib::audio::Audio;
    use sunrise_core::geometry::Vec3;
    use sunrise_level::particle_options::ParticleType;
    use sunrise_protocol::{
        entity_data::{Value, serializer},
        packets::play::{self, Clientbound as C},
    };
    use sunrise_sim::{motion::PlayerMotion, pose::Pose};
    use {
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
        sunrise_gameplay_common::game::execution_role::ExecutionRole,
        sunrise_gameplay_common::game::totem_common::TOTEM_EVENT,
    };

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
        sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = ExecutionRole::Client;
    let mut game = sunrise_client_gameplay::game::ClientGame::new(
        &loaded.resources,
        loaded.client_res.unwrap(),
        loaded.pack,
        loaded.client_pack.unwrap(),
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
        sunrise_gameplay::game::server_side::integrated::serve,
        sunrise_gameplay::game::server_side::integrated::run_component_harness_on_server,
        crate::game::session_verifier(),
        crate::game::profile_resolver,
    )
    .unwrap();
    let (client, _server) = sunrise_protocol::connection::Pipe::pair();
    game.connect_local(client, "EmitterTest").unwrap();
    let remote = game.client.remote.as_mut().unwrap();
    remote.entity_id = 7;
    remote.logged_in = true;
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.0, 64.0, 0.0)).unwrap());
    game.client.particles.0.seed_randoms(42, 43, 44);

    let check =
        |game: &sunrise_client_gameplay::game::ClientGame<'_>, ty, feet: [f64; 3], height: f32| {
            let mut seen = false;
            for (options, at, velocity) in game.client.particles.0.requested.as_ref().unwrap() {
                if options.ty != ty {
                    continue;
                }
                seen = true;
                let y = feet[1] + f64::from(height) * (0.5 + (velocity[1] - 0.2) / 4.0);
                assert!((at[0] - feet[0]).abs() <= f64::from(0.6_f32) / 4.0);
                assert!(
                    (at[1] - y).abs() < 1e-10,
                    "{ty:?} at {at:?}, expected y {y}"
                );
                assert!((at[2] - feet[2]).abs() <= f64::from(0.6_f32) / 4.0);
            }
            assert!(seen, "no {ty:?} emissions");
        };
    let types = [
        ParticleType::Crit,
        ParticleType::EnchantedHit,
        ParticleType::TotemOfUndying,
    ];
    let target = 40;
    let start = [5.0, 64.0, 7.0];
    game.spawn_entity(target, "minecraft:player", start, [0.0; 3]);
    game.client.particles.0.requested = Some(Vec::new());
    for action in [1, 2] {
        game.handle_packet(C::Animate(play::Animate { id: target, action }))
            .unwrap();
    }
    game.handle_packet(C::EntityEvent(play::EntityEvent {
        id: target,
        event: TOTEM_EVENT,
    }))
    .unwrap();
    for ty in types {
        check(&game, ty, start, 1.8);
    }

    let moved = [15.0, 65.0, -2.0];
    game.client.particles.0.requested = Some(Vec::new());
    game.handle_packet(C::SetEntityData(play::SetEntityData {
        id: target,
        values: vec![(6, serializer::POSE, Value::Id(Pose::Crouching.id()))],
    }))
    .unwrap();
    game.handle_packet(C::EntityPositionSync(play::EntityPositionSync {
        id: target,
        path: play::PositionPath::Linear(moved),
        yaw: 0.0,
        pitch: 0.0,
        on_ground: true,
    }))
    .unwrap();
    for _ in 0..4 {
        game.tick_remote_entities(true);
    }
    game.tick_particles();
    for ty in types {
        check(&game, ty, moved, 1.5);
    }

    // The old entity moved again before unpairing. Re-pairing its id before
    // the next particle tick keeps emitting at the old entity's final body.
    game.client.particles.0.requested = Some(Vec::new());
    let last = [18.0, 65.0, -2.0];
    game.common
        .mob_index
        .get_mut(&mut game.common.mobs, target as u32)
        .unwrap()
        .position = Vec3::new(last[0], last[1], last[2]);
    game.handle_packet(C::RemoveEntities(play::RemoveEntities {
        ids: vec![target],
    }))
    .unwrap();
    game.spawn_entity(target, "minecraft:player", [28.0, 67.0, 1.0], [0.0; 3]);
    game.tick_particles();
    for ty in types {
        check(&game, ty, last, 1.5);
    }

    game.client.particles.0.clear();
    game.client.particles.0.requested = Some(Vec::new());
    game.handle_packet(C::EntityEvent(play::EntityEvent {
        id: 7,
        event: TOTEM_EVENT,
    }))
    .unwrap();
    check(&game, ParticleType::TotemOfUndying, [0.0, 64.0, 0.0], 1.8);
    game.client.particles.0.requested = Some(Vec::new());
    let player = game.player.as_mut().unwrap();
    player.position = Vec3::new(-4.0, 68.0, 8.0);
    player.set_pose(Pose::Swimming).unwrap();
    game.tick_particles();
    check(&game, ParticleType::TotemOfUndying, [-4.0, 68.0, 8.0], 0.6);
}
