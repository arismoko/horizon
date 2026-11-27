use {
    sunrise_gameplay_common::game::remote::shape_meets,
    sunrise_gameplay_common::game::remote::snap_target,
};

/// The packet riding graph must retain invisible intermediary entities. A custom
/// sit plugin can use an area-effect cloud, whose server position stops updating
/// while it rides a player, with a player or marker stand as its own passenger.
#[test]
#[ignore = "daily"]
fn plugin_player_stack_follows_network_moves_and_local_carriers() {
    use crate::game::load_resources;
    use crate::settings::Settings;
    use sunrise_client_lib::audio::Audio;
    use sunrise_core::geometry::Vec3;
    use sunrise_protocol::packets::play::{self, Clientbound as C};
    use sunrise_sim::{motion::PlayerMotion, sprint::Keys};
    use {
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
        sunrise_gameplay_common::game::execution_role::ExecutionRole,
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
        ExecutionRole::Client,
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
    game.connect_local(client, "SeatTest").unwrap();
    game.client.remote.as_mut().unwrap().entity_id = 7;
    game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
    game.spawn_entity(40, "minecraft:player", [2.0, 64.0, 3.0], [0.0; 3]);
    game.spawn_entity(41, "minecraft:player", [0.0, 64.0, 0.0], [0.0; 3]);
    game.spawn_entity(42, "minecraft:area_effect_cloud", [0.0; 3], [0.0; 3]);
    let set =
        |g: &mut sunrise_client_gameplay::game::ClientGame<'_>, vehicle, passengers: Vec<i32>| {
            g.handle_packet(C::SetPassengers(play::SetPassengers {
                vehicle,
                passengers,
            }))
            .unwrap();
        };
    set(&mut game, 40, vec![42]);
    set(&mut game, 42, vec![7, 41]);
    let close = |at: Vec3, x: f64, y: f64, z: f64| {
        assert!(
            (at.x - x).abs() < 1e-5 && (at.y - y).abs() < 1e-5 && (at.z - z).abs() < 1e-5,
            "position {at:?}, expected [{x}, {y}, {z}]"
        );
    };
    assert!(game.remote_ride(Keys::default()));
    close(game.player.as_ref().unwrap().position, 2.0, 65.7, 3.0);
    close(
        game.mobs.iter().find(|m| m.id == 41).unwrap().position,
        2.0,
        65.7,
        3.0,
    );
    game.handle_packet(C::EntityPositionSync(play::EntityPositionSync {
        id: 40,
        path: play::PositionPath::Linear([12.0, 67.0, -4.0]),
        yaw: 45.0,
        pitch: 0.0,
        on_ground: true,
    }))
    .unwrap();
    for _ in 0..4 {
        game.tick_remote_entities(true);
        game.remote_ride(Keys::default());
    }
    close(game.player.as_ref().unwrap().position, 12.0, 68.7, -4.0);
    close(
        game.mobs.iter().find(|m| m.id == 41).unwrap().position,
        12.0,
        68.7,
        -4.0,
    );
    // The intermediary's packet position must remain unchanged for relative decoding.
    assert_eq!(
        game.client.remote.as_ref().unwrap().tracked(42).unwrap().1,
        [0.0; 3]
    );
    // A remote player riding the local player must follow our own movement too.
    set(&mut game, 42, vec![41]);
    set(&mut game, 7, vec![42]);
    game.player.as_mut().unwrap().position = Vec3::new(20.0, 70.0, 25.0);
    game.position_riders();
    close(
        game.mobs.iter().find(|m| m.id == 41).unwrap().position,
        20.0,
        71.7,
        25.0,
    );
    // A direct stack has no intermediary's 0.5-height offset.
    set(&mut game, 40, vec![7, 41]);
    game.remote_ride(Keys::default());
    close(game.player.as_ref().unwrap().position, 12.0, 68.2, -4.0);
    close(
        game.mobs.iter().find(|m| m.id == 41).unwrap().position,
        12.0,
        68.2,
        -4.0,
    );
    // Removing the carrier detaches passengers even without a final empty packet.
    game.remove_entity(40);
    assert_eq!(game.riding, None);
    assert_eq!(
        game.mobs.iter().find(|m| m.id == 41).unwrap().extra.mount,
        None
    );
}

#[test]
#[ignore = "daily"]
fn a_stand_position_snap_keeps_the_first_draws_body_yaw() {
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;
    use sunrise_protocol::packets::play::{self, Clientbound as C};

    if run_fresh_cases(
        "cut_layout::game::remote::tests::movement_snap_tests::a_stand_position_snap_keeps_the_first_draws_body_yaw",
        "SUNRISE_STAND_SNAP_TEST",
        &["snap"],
    ) {
        return;
    }
    with_client_game(|game| {
        let (client, _server) = sunrise_protocol::connection::Pipe::pair();
        game.connect_local(client, "StandSnap").unwrap();
        let kind = game
            .client
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:entity_type")
            .unwrap()
            .id("minecraft:armor_stand")
            .unwrap();
        game.handle_packet(C::AddEntity(play::AddEntity {
            id: 40,
            uuid: 1234,
            kind,
            position: [0.5, 64.0, 0.5],
            velocity: [0.0; 3],
            pitch: 0,
            yaw: 32,      // 45°
            head_yaw: 64, // 90°
            data: 0,
        }))
        .unwrap();
        // Far enough for handleEntityPositionSync to snap, before any client tick.
        game.handle_packet(C::EntityPositionSync(play::EntityPositionSync {
            id: 40,
            path: play::PositionPath::Linear([80.5, 64.0, 0.5]),
            yaw: 135.0,
            pitch: 0.0,
            on_ground: false,
        }))
        .unwrap();
        let stand = game
            .common
            .armor_stands
            .iter()
            .find(|a| a.id == 40)
            .unwrap();
        assert_eq!(stand.render_position(0.25), Vec3::new(80.5, 64.0, 0.5));
        assert_eq!([stand.previous_yaw, stand.yaw], [135.0; 2]);
        assert_eq!([stand.render_yaw(0.0), stand.render_yaw(1.0)], [90.0; 2]);
        assert_eq!([stand.previous_head_yaw, stand.head_yaw], [90.0; 2]);
    });
}

/// `Entity.isColliding` and the snap `ClientLevel.syncBlockState` makes: a
/// block the server puts back where the player stands sends it to the
/// position the prediction remembered, and a block that comes back clear of
/// the player, an empty one, or one the player has been teleported since
/// leaves it where it is.
#[test]
fn a_block_the_server_puts_back_snaps_the_player_out_of_it() {
    use sunrise_core::geometry::{Aabb, Vec3, VoxelShape};
    // A player standing in block (0, 64, 0): 0.6 wide, 1.8 tall.
    let player = Aabb::new(Vec3::new(0.2, 64.0, 0.2), Vec3::new(0.8, 65.8, 0.8));
    let cube = VoxelShape::block();
    assert!(
        shape_meets(&cube, [0, 64, 0], player),
        "the block the player is standing in"
    );
    assert!(!shape_meets(&cube, [0, 66, 0], player), "above its head");
    assert!(
        !shape_meets(&cube, [1, 64, 0], player),
        "the block beside it"
    );
    assert!(!shape_meets(&VoxelShape::empty(), [0, 64, 0], player));

    // The snap itself: the remembered position, unless a teleport came after
    // the prediction (`lastTeleportSequence < sequence`).
    let at = [0.5, 64.0, 0.5];
    assert_eq!(snap_target(at, -1, 3, true), Some(at));
    assert_eq!(snap_target(at, -1, 3, false), None, "a block it clears");
    assert_eq!(snap_target(at, 3, 3, true), None, "teleported at 3");
    assert_eq!(
        snap_target(at, 2, 3, true),
        Some(at),
        "teleported before it"
    );
}

/// tooBigToInterpolate: a position sync measures from where the entity
/// is, a teleport from where its interpolation is headed.
#[test]
fn a_far_move_snaps_by_the_packets_own_measure() {
    use {
        sunrise_gameplay_common::game::remote::Path, sunrise_gameplay_common::game::remote::PosRot,
        sunrise_gameplay_common::game::remote::SteppedInterpolation,
        sunrise_gameplay_common::game::remote::movement::MovePacket,
    };
    let at = |x: f64| PosRot {
        position: [x, 0.0, 0.0],
        yaw: 0.0,
        pitch: 0.0,
    };
    let mut handler = SteppedInterpolation::new(3);
    // At x 0, headed for x 50.
    handler.interpolate_to(at(0.0), Some(Path::Linear([50.0, 0.0, 0.0])), None);
    let to = [100.0, 0.0, 0.0];
    assert!(MovePacket::PositionSync.snaps(&handler, at(0.0), to));
    assert!(!MovePacket::Teleport.snaps(&handler, at(0.0), to));
    assert!(!MovePacket::Move.snaps(&handler, at(0.0), to));
    // With nothing in flight, both measure from the entity.
    let idle = SteppedInterpolation::new(3);
    assert!(MovePacket::Teleport.snaps(&idle, at(0.0), to));
    assert!(!MovePacket::Teleport.snaps(&idle, at(0.0), [60.0, 0.0, 0.0]));
}

/// A far sync puts the entity there and drops the steps it was walking;
/// a rotation alone keeps the position it is headed for and turns there
/// over the update interval.
#[test]
fn a_living_entitys_move_snaps_or_steps() {
    use {
        sunrise_gameplay_common::game::remote::Path, sunrise_gameplay_common::game::remote::PosRot,
        sunrise_gameplay_common::game::remote::SteppedInterpolation,
        sunrise_gameplay_common::game::remote::movement::MovePacket,
        sunrise_gameplay_common::game::remote::movement::stepped_move,
    };
    let at = |x: f64, yaw: f32| PosRot {
        position: [x, 0.0, 0.0],
        yaw,
        pitch: 0.0,
    };
    let mut handler = SteppedInterpolation::new(3);
    // Walking to x 5 when the far sync comes.
    handler.interpolate_to(at(0.0, 0.0), Some(Path::Linear([5.0, 0.0, 0.0])), None);
    assert!(handler.active());
    let far = [100.0, 0.0, 0.0];
    let snap = stepped_move(
        &mut handler,
        MovePacket::PositionSync,
        at(0.0, 0.0),
        far,
        Some(Path::Linear(far)),
        Some((90.0, 0.0)),
    );
    assert_eq!(snap, Some(at(100.0, 90.0)));
    assert!(!handler.active());
    assert_eq!(handler.interpolate(), None, "no step back toward x 5");
    // A rotation-only move packet.
    let turn = stepped_move(
        &mut handler,
        MovePacket::Move,
        at(100.0, 90.0),
        far,
        None,
        Some((180.0, 0.0)),
    );
    assert_eq!(turn, None);
    let steps: Vec<_> = std::iter::from_fn(|| handler.interpolate()).collect();
    assert_eq!(
        steps.iter().map(|s| s.yaw).collect::<Vec<_>>(),
        [120.0, 150.0, 180.0]
    );
    assert!(steps.iter().all(|s| s.position == far));
}

#[test]
#[ignore = "daily"]
fn client_entity_move_fires_for_simulated_bodies_and_not_observed() {
    use crate::game::native_entity_refs::tests::with_game_role_client;
    use mod_api::{EntityMoveEvent, Flow, Hooks, LoopContext, LoopSide};
    use std::sync::{Arc, Mutex};
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::execution_role::ExecutionRole;
    use sunrise_gameplay_common::game::test_fixtures::fixture_root;
    use sunrise_items::items::ItemStack;
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::{entities::ItemEntity, motion::PlayerMotion};

    with_game_role_client(fixture_root(), ExecutionRole::Client, |game| {
        let (client, _server) = sunrise_protocol::connection::Pipe::pair();
        game.connect_local(client, "ClientMoveTest").unwrap();
        {
            let remote = game.client.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.loaded_sent = true;
            remote.entity_id = 7;
        }
        game.client
            .streaming
            .insert_remote(&mut game.common.world, Chunk::empty(0, 0));
        let mut player = PlayerMotion::new(game.motion, Vec3::new(3.0, 64.0, 3.0)).unwrap();
        player.entity_id = 7;
        player.velocity = Vec3::new(0.2, 0.0, 0.0);
        game.common.player = Some(player);
        // Client-simulated dropped item (`SERVER_AND_CLIENT`).
        game.common.dropped.push(ItemEntity::new(
            20,
            ItemStack::new("minecraft:stone", 1),
            Vec3::new(5.0, 64.0, 5.0),
            Vec3::new(0.1, 0.0, 0.0),
            0.0,
        ));
        // Observed remote zombie (server-authoritative, not simulated on client).
        game.spawn_entity(30, "minecraft:zombie", [8.0, 64.0, 8.0], [0.0; 3]);

        let heard = Arc::new(Mutex::new(Vec::<(u32, String, LoopSide)>::new()));
        struct Probe(Arc<Mutex<Vec<(u32, String, LoopSide)>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Probe {
            fn entity_move(
                &self,
                context: &dyn LoopContext,
                event: &mut EntityMoveEvent<'_>,
            ) -> Flow {
                self.0
                    .lock()
                    .unwrap()
                    .push((event.entity, event.kind.to_owned(), context.side()));
                Flow::Continue
            }
        }
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "client-move",
            Box::new(Probe(Arc::clone(&heard))),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            game.tick_remote_own_player().unwrap();
            game.tick_remote_entities(true);
        });
        let heard = heard.lock().unwrap();
        assert!(
            heard.iter().any(|(id, kind, side)| {
                *id == 7 && kind == "minecraft:player" && *side == LoopSide::Client
            }),
            "expected client player entity_move, heard {heard:?}"
        );
        assert!(
            heard.iter().any(|(id, kind, side)| {
                *id == 20 && kind == "minecraft:item" && *side == LoopSide::Client
            }),
            "expected client item entity_move, heard {heard:?}"
        );
        assert!(
            !heard.iter().any(|(id, _, _)| *id == 30),
            "observed zombie must not fire client entity_move, heard {heard:?}"
        );
    });
}

#[test]
#[ignore = "daily"]
fn entity_interpolate_to_steps_observed_boat_and_refuses_simulated_bodies() {
    use crate::game::native_entity_refs::tests::with_game_role_client;
    use mod_api::{ClientHost, EntityRef, WorldEpoch};
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::execution_role::ExecutionRole;
    use sunrise_gameplay_common::game::test_fixtures::fixture_root;
    use sunrise_level::chunk::Chunk;
    use sunrise_protocol::packets::play::{self, Clientbound as C};
    use sunrise_sim::motion::PlayerMotion;

    with_game_role_client(fixture_root(), ExecutionRole::Client, |game| {
        let (client, _server) = sunrise_protocol::connection::Pipe::pair();
        game.connect_local(client, "InterpTest").unwrap();
        {
            let remote = game.client.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.loaded_sent = true;
            remote.entity_id = 7;
        }
        game.client
            .streaming
            .insert_remote(&mut game.common.world, Chunk::empty(0, 0));
        let mut player = PlayerMotion::new(game.motion, Vec3::new(1.0, 64.0, 1.0)).unwrap();
        player.entity_id = 7;
        game.common.player = Some(player);
        let boat_kind = game
            .client
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:entity_type")
            .unwrap()
            .id("minecraft:oak_boat")
            .unwrap();
        game.handle_packet(C::AddEntity(play::AddEntity {
            id: 50,
            uuid: 50,
            kind: boat_kind,
            position: [10.0, 64.0, 5.0],
            velocity: [0.0; 3],
            pitch: 0,
            yaw: 0,
            head_yaw: 0,
            data: 0,
        }))
        .unwrap();

        let player_ref = game.entity(7).unwrap();
        let boat_ref = game.entity(50).unwrap();
        let stale_ref = EntityRef::new(WorldEpoch::new(0), 50);

        {
            let host: &mut dyn ClientHost = game;
            // Refused for local player, stale handle, and non-finite inputs.
            assert!(!player_ref.interpolate_to(host, [2.0, 64.0, 2.0], None));
            assert!(!stale_ref.interpolate_to(host, [13.0, 64.0, 5.0], None));
            assert!(!boat_ref.interpolate_to(host, [f64::NAN, 64.0, 5.0], None));
            assert!(!boat_ref.interpolate_to(host, [13.0, 64.0, 5.0], Some([f32::INFINITY, 0.0])));
            // Accepted for observed boat: eases over 3 ticks.
            assert!(boat_ref.interpolate_to(host, [13.0, 64.0, 5.0], Some([90.0, 0.0])));
        }
        assert_eq!(game.common.boats[0].position, Vec3::new(10.0, 64.0, 5.0));
        for _ in 0..3 {
            game.tick_remote_entities(true);
        }
        let at = game.common.boats[0].position;
        assert!(
            (at.x - 13.0).abs() < 1e-6 && (at.y - 64.0).abs() < 1e-6 && (at.z - 5.0).abs() < 1e-6,
            "boat should reach interpolation target after 3 steps, got {at:?}"
        );

        // Once the local player drives the boat, the client simulates it and interpolate_to refuses.
        game.handle_packet(C::SetPassengers(play::SetPassengers {
            vehicle: 50,
            passengers: vec![7],
        }))
        .unwrap();
        let before = game.common.boats[0].position;
        {
            let host: &mut dyn ClientHost = game;
            assert!(!boat_ref.interpolate_to(host, [20.0, 64.0, 5.0], None));
        }
        assert_eq!(game.common.boats[0].position, before);
    });
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
use sunrise_gameplay_common::game::passengers_common::PassengersCommonGameExt1 as _;
