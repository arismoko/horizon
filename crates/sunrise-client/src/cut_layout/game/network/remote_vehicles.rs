#![cfg(test)]
use sunrise_client_gameplay::game::client_side::remote_vehicles::{
    client_moves, mount_move, riding_jump,
};
use sunrise_core::geometry::Vec3;
use sunrise_protocol::packets::play;
use sunrise_protocol::packets::play::Serverbound as S;
use sunrise_sim::mobs::Mob;
use sunrise_sim::mobs::equine;
use sunrise_sim::sprint::Keys;
use sunrise_world::stepped_interpolation::PosRot;
#[cfg(feature = "client")]
#[cfg(test)]
pub(crate) mod tests {
    #[test]
    fn local_player_on_block_display_and_interaction() {
        for kind in ["minecraft:block_display", "minecraft:interaction"] {
            let at = glam::DVec3::new(10.5, -59.5, 0.5);
            let before = glam::DVec3::new(10.0, -59.0, 0.5);
            let (seat, previous) =
                sunrise_client_gameplay::game::client_side::remote_vehicles::generic_player_seat(
                    at, before, 1.0,
                );
            assert_eq!(
                seat,
                sunrise_core::geometry::Vec3::new(10.5, -60.1, 0.5),
                "{kind}"
            );
            assert_eq!(
                previous,
                sunrise_core::geometry::Vec3::new(10.0, -59.6, 0.5),
                "{kind}"
            );
            let (scaled, _) =
                sunrise_client_gameplay::game::client_side::remote_vehicles::generic_player_seat(
                    at, before, 2.0,
                );
            assert_eq!(scaled.y, -60.7, "{kind}");
        }
    }

    use super::*;
    use sunrise_sim::mobs::MobTypes;

    /// `AbstractBoat.getControllingPassenger` (#6003): only a boat the player rides first is
    /// the boat this client steers; a ridden minecart is not, though the player rides it first.
    #[test]
    #[ignore = "daily"]
    fn only_a_boat_is_the_driven_boat() {
        sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
            use sunrise_gameplay_common::game::boats_common::Boat;
            use sunrise_gameplay_common::game::minecarts::{CartKind, Minecart};
            game.carts.push(Minecart::new(
                5,
                CartKind::Rideable,
                Vec3::new(0.5, 64.0, 0.5),
            ));
            game.boats
                .push(Boat::new(6, "oak".into(), Vec3::new(2.5, 64.0, 0.5), 0.0));
            for (riding, cart, boat) in [
                (None, false, false),
                (Some(5), false, false),
                (Some(6), false, true),
            ] {
                game.riding = riding;
                assert_eq!(game.controls_boat(5), cart, "the cart, riding {riding:?}");
                assert_eq!(game.controls_boat(6), boat, "the boat, riding {riding:?}");
            }
            assert!(game.driven_boat().is_some(), "riding the boat steers it");
            game.riding = Some(5);
            assert!(
                game.driven_boat().is_none(),
                "riding the cart steers no boat"
            );
        });
    }

    /// handleTeleportEntity and handleMoveVehicle for the vehicle this client moves (a saddled pig steered with a
    /// carrot on a stick): a relative teleport counts from where the pig is on this client and eases it there,
    /// reported as the spot it is going to; a server move puts it there and is answered at once with a
    /// ServerboundMoveVehiclePacket from that spot.
    #[test]
    #[ignore = "daily"]
    fn the_driven_vehicle_takes_the_servers_teleports_and_moves() {
        sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
            use sunrise_items::items::ItemStack;
            use sunrise_protocol::packets::play::Clientbound as C;
            use sunrise_sim::motion::PlayerMotion;
            let root =
                std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
            let (mut peer, _writer) = crate::game::remote::tests::connect_menu_test(game);
            peer.set_read_timeout(Some(std::time::Duration::from_millis(50)))
                .unwrap();
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
            let types = MobTypes::load(&root).unwrap();
            let pig = std::sync::Arc::new(types.types["minecraft:pig"].clone());
            let mut m = Mob::new(7, pig, Vec3::new(3.5, 64.0, 2.5), 0.0);
            m.set_saddle(Some(ItemStack::new("minecraft:saddle", 1)));
            game.mobs.clear();
            game.mobs.push(m);
            game.common.inventory[game.common.selected] =
                Some(ItemStack::new("minecraft:carrot_on_a_stick", 1));
            game.riding = Some(7);
            game.client.remote.as_mut().unwrap().first_passenger = true;
            assert!(game.driven_mount().is_some());
            use sunrise_protocol::packets::play::relative as rel;
            game.apply_packet(C::TeleportEntity(play::TeleportEntity {
                id: 7,
                position: [0.0, 0.0, 4.0],
                delta: [0.0; 3],
                yaw: 0.0,
                pitch: 0.0,
                relatives: rel::X | rel::Y | rel::Z | rel::Y_ROT | rel::X_ROT,
                on_ground: false,
            }))
            .unwrap();
            // The pig eases there (its SteppedInterpolationHandler) and reports the spot
            // it is going to.
            assert_eq!(game.mobs[0].position, Vec3::new(3.5, 64.0, 2.5));
            match game.remote_vehicle_packets().first() {
                Some(play::Serverbound::MoveVehicle(p)) => assert_eq!(p.position, [3.5, 64.0, 6.5]),
                other => panic!("{other:?}"),
            }
            while peer.recv().is_ok() {}
            game.apply_packet(C::MoveVehicle(play::MoveVehicle {
                position: [1.5, 64.0, 1.5],
                yaw: 90.0,
                pitch: 0.0,
            }))
            .unwrap();
            assert_eq!(game.mobs[0].position, Vec3::new(1.5, 64.0, 1.5));
            let mut answers = Vec::new();
            while let Ok((id, bytes)) = peer.recv() {
                if let Ok(play::Serverbound::MoveVehicle(p)) = play::Serverbound::decode(id, &bytes)
                {
                    answers.push(p.position);
                }
            }
            assert_eq!(answers, [[1.5, 64.0, 1.5]]);
        });
    }

    #[test]
    #[ignore = "requires prepared resources and the vanilla mount-control reference"]
    fn steering_mount_control_matches_vanilla() {
        run_steering_mount_control_matches_vanilla();
    }

    pub(crate) fn run_steering_mount_control_matches_vanilla() {
        use crate::game::load_resources;
        use crate::settings::Settings;
        use sunrise_client_lib::audio::Audio;
        use sunrise_gameplay_common::game::{Options, Progress, execution_role::ExecutionRole};
        use sunrise_items::items::ItemStack;
        use sunrise_protocol::connection::Pipe;
        use sunrise_sim::motion::PlayerMotion;
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let reference = std::env::var_os("SUNRISE_MOUNT_CONTROL_REFERENCE").unwrap();
        let rows: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(reference).unwrap()).unwrap();
        assert_eq!(rows.len(), 32, "missing vanilla control cases");
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
            &mut |_, _| unreachable!("no menu handoff"),
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
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "SteeringTest").unwrap();
        game.client.remote.as_mut().unwrap().joined = true;
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 300.0, 0.5)).unwrap());
        let types = MobTypes::load(&root).unwrap();
        for row in &rows {
            let kind = row["kind"].as_str().unwrap();
            let t = std::sync::Arc::new(types.types[&format!("minecraft:{kind}")].clone());
            let mut m = Mob::new(7, t, Vec3::new(0.5, 300.0, 0.5), 0.0);
            if row["saddled"].as_bool().unwrap() {
                m.set_saddle(Some(ItemStack::new("minecraft:saddle", 1)));
            }
            let stick = if kind == "pig" {
                "minecraft:carrot_on_a_stick"
            } else {
                "minecraft:warped_fungus_on_a_stick"
            };
            let wrong = if kind == "pig" {
                "minecraft:warped_fungus_on_a_stick"
            } else {
                "minecraft:carrot_on_a_stick"
            };
            let hand = row["hand"].as_str().unwrap();
            game.common.inventory[game.common.selected] = match hand {
                "main" => Some(ItemStack::new(stick, 1)),
                "wrong" => Some(ItemStack::new(wrong, 1)),
                _ => None,
            };
            game.offhand = (hand == "off").then(|| ItemStack::new(stick, 1));
            game.mobs.clear();
            game.mobs.push(m);
            game.riding = Some(7);
            game.client.remote.as_mut().unwrap().first_passenger = row["first"].as_bool().unwrap();
            let controlled = row["controlled"].as_bool().unwrap();
            assert_eq!(game.driven_mount().is_some(), controlled, "{row}");
            let packets = game.remote_vehicle_packets();
            assert_eq!(packets.len(), usize::from(controlled), "{row}");
            if controlled {
                assert!(matches!(packets[0], S::MoveVehicle(_)), "{row}");
                let before = game.mobs[0].position;
                assert!(game.remote_ride(Keys::default()));
                let m = &game.mobs[0];
                assert_eq!(
                    f64::from(m.ai.zza),
                    row["forward"].as_f64().unwrap(),
                    "{row}"
                );
                assert!(m.position.z > before.z, "{row}: mount did not move forward");
                match &game.remote_vehicle_packets()[0] {
                    S::MoveVehicle(p) => assert_eq!(
                        p.position,
                        [m.position.x, m.position.y, m.position.z],
                        "{row}"
                    ),
                    packet => panic!("wrong movement packet: {packet:?}"),
                }
            }
        }
        drop(game);
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!("no menu handoff"),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = ExecutionRole::IntegratedServer;
        let mut server = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        server.player = Some(PlayerMotion::new(server.motion, Vec3::new(0.5, 300.0, 0.5)).unwrap());
        for row in &rows {
            let kind = row["kind"].as_str().unwrap();
            let t = std::sync::Arc::new(types.types[&format!("minecraft:{kind}")].clone());
            let mut m = Mob::new(7, t, Vec3::new(0.5, 300.0, 0.5), 0.0);
            if row["saddled"].as_bool().unwrap() {
                m.set_saddle(Some(ItemStack::new("minecraft:saddle", 1)));
            }
            let stick = if kind == "pig" {
                "minecraft:carrot_on_a_stick"
            } else {
                "minecraft:warped_fungus_on_a_stick"
            };
            let wrong = if kind == "pig" {
                "minecraft:warped_fungus_on_a_stick"
            } else {
                "minecraft:carrot_on_a_stick"
            };
            let hand = row["hand"].as_str().unwrap();
            server.common.inventory[server.common.selected] = match hand {
                "main" => Some(ItemStack::new(stick, 1)),
                "wrong" => Some(ItemStack::new(wrong, 1)),
                _ => None,
            };
            server.offhand = (hand == "off").then(|| ItemStack::new(stick, 1));
            server.mobs.clear();
            server.mobs.push(m);
            server.riding = Some(7);
            let controlled = row["controlled"].as_bool().unwrap();
            // The server reads the same held-item requirement, then waits for
            // this controlling client's MoveVehicle rather than traveling twice.
            if row["first"].as_bool().unwrap() {
                assert!(server.tick_mount_riding(Keys::default()));
                server.mobs[0].rider.as_mut().unwrap().client = true;
            } else {
                server.mobs[0].rider = None;
            }
            assert_eq!(
                equine::moved_by_client(&server.mobs[0]),
                !row["server_simulates"].as_bool().unwrap(),
                "{row}"
            );
            if controlled {
                let mut server_mount = server.mobs[0].clone();
                let before = server_mount.position;
                let mut test_level = sunrise_sim::health::TestLevel::new(0);
                *test_level.random() = (*server.world.random).clone();
                let mut events = Vec::new();
                server_mount.tick(
                    &server.mob_world_base(&[]),
                    &mut test_level.level(),
                    &mut events,
                );
                assert_eq!(
                    server_mount.position, before,
                    "server traveled twice: {row}"
                );
            }
        }
    }

    /// A mob of the prepared resources' type `kind` (None without them).
    fn mob(kind: &str) -> Option<Mob> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        let types = MobTypes::load(&root).ok()?;
        let t = std::sync::Arc::new(types.types.get(kind)?.clone());
        Some(Mob::new(7, t, Vec3::new(1.5, 64.0, -2.5), 0.0))
    }

    /// #2920: on a vanilla server the rider's client moves a saddled horse,
    /// donkey, mule or camel; a pig without its stick, a llama, an unsaddled horse or a second
    /// passenger's ride is the server's.
    #[test]
    fn the_rider_moves_a_saddled_horse_on_its_client() {
        for kind in ["horse", "donkey", "mule", "camel", "pig", "llama"] {
            let Some(mut m) = mob(&format!("minecraft:{kind}")) else {
                return;
            };
            assert!(!client_moves(&m, true, false), "{kind} without a saddle");
            m.set_saddle(Some(sunrise_items::items::ItemStack::new(
                "minecraft:saddle",
                1,
            )));
            let horse = matches!(kind, "horse" | "donkey" | "mule" | "camel");
            assert_eq!(client_moves(&m, true, false), horse, "{kind}");
            assert!(
                !client_moves(&m, false, false),
                "{kind} behind another rider"
            );
        }
    }

    /// #4107: on a vanilla server the rider's client moves a harnessed adult
    /// Happy Ghast (`getControllingPassenger`); an unharnessed one, a baby
    /// or a second passenger's ride is the server's.
    #[test]
    fn the_rider_moves_a_harnessed_ghast_on_its_client() {
        let Some(mut m) = mob("minecraft:happy_ghast") else {
            return;
        };
        assert!(!client_moves(&m, true, false), "ghast without a harness");
        m.extra
            .set_body(Some("minecraft:white_happy_ghast_harness".into()));
        assert!(client_moves(&m, true, false), "harnessed adult");
        assert!(!client_moves(&m, false, false), "behind another rider");
        m.extra.age = -1;
        assert!(!client_moves(&m, true, false), "baby ghast");
    }

    /// The mount ticks before its rider (`ClientLevel.tickNonPassenger`): it
    /// moves on the input the rider's previous tick applied (`zza`, the key
    /// ×0.98 through `LocalPlayer.modifyInput`), so a press reaches it a tick
    /// late and a release a tick late too, as the server's anticheats expect.
    #[test]
    #[ignore = "daily"]
    fn a_driven_horse_moves_on_the_riders_last_tick_input() {
        use crate::game::load_resources;
        use crate::settings::Settings;
        use sunrise_client_lib::audio::Audio;
        use sunrise_gameplay_common::game::{Options, Progress, execution_role::ExecutionRole};
        use sunrise_items::items::ItemStack;
        use sunrise_protocol::connection::Pipe;
        use sunrise_sim::motion::PlayerMotion;
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
            &mut |_, _| unreachable!("no menu handoff"),
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
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "RiderTest").unwrap();
        game.client.remote.as_mut().unwrap().joined = true;
        game.client.remote.as_mut().unwrap().first_passenger = true;
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 300.0, 0.5)).unwrap());
        let types = MobTypes::load(&root).unwrap();
        let t = std::sync::Arc::new(types.types["minecraft:horse"].clone());
        let mut horse = Mob::new(7, t, Vec3::new(0.5, 300.0, 0.5), 0.0);
        horse.set_saddle(Some(ItemStack::new("minecraft:saddle", 1)));
        game.mobs.push(horse);
        game.riding = Some(7);
        let forward = Keys {
            forward: true,
            ..Keys::default()
        };
        let zza = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>, keys| {
            assert!(game.remote_ride(keys));
            game.mobs[0].ai.zza
        };
        assert_eq!(zza(&mut game, forward), 0.0, "the press waits a tick");
        assert_eq!(zza(&mut game, forward), 0.98, "modifyInput's 0.98");
        assert_eq!(
            zza(&mut game, Keys::default()),
            0.98,
            "the release waits a tick"
        );
        assert_eq!(zza(&mut game, Keys::default()), 0.0);
    }

    #[test]
    #[ignore = "daily"]
    fn a_sprinting_rider_boards_a_camel_at_sprint_speed() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::client_side::remote_vehicles::tests::a_sprinting_rider_boards_a_camel_at_sprint_speed",
            || {
                use sunrise_items::items::ItemStack;
                use sunrise_sim::{mobs::camel, motion::PlayerMotion};
                sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(
                    |game| {
                        let (client, _server) = sunrise_protocol::connection::Pipe::pair();
                        game.connect_local(client, "CamelRider").unwrap();
                        let remote = game.client.remote.as_mut().unwrap();
                        remote.joined = true;
                        remote.entity_id = 42;
                        game.world.game_time = 1_000;
                        let at = Vec3::new(4.5, 80.0, 4.5);
                        let mut player = PlayerMotion::new(game.motion, at).unwrap();
                        player.entity_id = 42;
                        player.applied_input = [0.0, 0.98];
                        player.set_sprinting(true).unwrap();
                        game.player = Some(player);
                        let root =
                            sunrise_gameplay::game::native_entity_refs::tests::fixture_root();
                        let types = MobTypes::load(&root).unwrap();
                        let kind = std::sync::Arc::new(types.types["minecraft:camel"].clone());
                        let mut mount = Mob::new(7, kind, at, 0.0);
                        mount.set_saddle(Some(ItemStack::new("minecraft:saddle", 1)));
                        camel::finish_load(&mut mount, game.world.game_time);
                        assert!(!camel::refuse_to_move(&mount));
                        assert_eq!(mount.extra.anim_ticks, 0);
                        let expected = mount.movement_speed() as f32 + 0.1_f32;
                        game.mobs.push(mount);
                        game.apply_remote_passengers(7, &[42]);
                        assert!(game.mobs[0].rider.is_none(), "before the first driven tick");
                        assert!(game.remote_ride(Keys::default()));
                        assert_eq!(game.mobs[0].ai.speed.to_bits(), expected.to_bits());
                        assert_eq!(game.mobs[0].ai.zza, 0.98, "the last walking input");
                        assert!(game.player.as_ref().unwrap().sprinting);
                    },
                );
            },
        );
    }

    /// #2920: the client reports where it moved the mount, and a released
    /// jump with the bar's percentage (`sendRidingJump`).
    #[test]
    fn the_rider_sends_its_mounts_moves_and_jumps() {
        let Some(mut m) = mob("minecraft:horse") else {
            return;
        };
        (m.yaw, m.head_pitch, m.ground.on_ground) = (90.0, 10.0, true);
        let at = PosRot {
            position: [m.position.x, m.position.y, m.position.z],
            yaw: m.yaw,
            pitch: m.head_pitch,
        };
        match mount_move(&m, at) {
            S::MoveVehicle(p) => {
                assert_eq!(p.position, [1.5, 64.0, -2.5]);
                assert_eq!((p.yaw, p.pitch, p.on_ground), (90.0, 10.0, true));
            }
            other => panic!("{other:?}"),
        }
        match riding_jump(42, 0.75) {
            S::PlayerCommand(p) => assert_eq!((p.id, p.action, p.data), (42, 3, 75)),
            other => panic!("{other:?}"),
        }
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
mod plugin_seat_tests {
    #[test]
    #[ignore = "daily"]
    fn controlling_client_copies_sneak_press_and_release_into_rider() {
        use sunrise_core::geometry::Vec3;
        use sunrise_items::items::ItemStack;
        use sunrise_sim::{mobs::Mob, sprint::Keys};
        sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
            let kind = game.res.mob_types().get("minecraft:horse").unwrap().clone();
            let mut horse_mob = Mob::new(7, kind, Vec3::new(8.5, 100.0, 8.5), 0.0);
            horse_mob.set_saddle(Some(ItemStack::new("minecraft:saddle", 1)));
            let horse = horse_mob.id;
            game.mobs.push(horse_mob);
            game.riding = Some(horse);
            for shift in [true, false] {
                let i = game.mobs.iter().position(|m| m.id == horse).unwrap();
                // Exercise client prediction itself, not the server input assignment.
                game.drive_mount(
                    i,
                    Keys {
                        shift,
                        ..Keys::default()
                    },
                    0.0,
                );
                let input = game.mobs[i]
                    .rider
                    .expect("prediction stores next tick's input");
                assert_eq!(input.sneak, shift);
                assert!(!input.client, "the controlling client simulates this mount");
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn local_rider_uses_attachments_and_follows_a_moving_seat_chain() {
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::{mobs::Mob, motion::PlayerMotion, sprint::Keys};
        use {
            crate::game::load_resources, crate::settings::Settings,
            sunrise_client_lib::audio::Audio, sunrise_gameplay_common::game::Options,
            sunrise_gameplay_common::game::Progress,
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
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
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
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        let mut stand = sunrise_sim::entities::ArmorStand::new(42, Vec3::new(2.0, 64.0, 3.0), 0.0);
        stand.marker = true;
        game.armor_stands.push(stand);
        game.riding = Some(42);
        assert!(game.remote_ride(Keys::default()));
        let close = |actual: f64, expected: f64| {
            assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}")
        };
        // EntityAttachments: zero-height marker minus Avatar's 0.6 attachment.
        close(game.player.as_ref().unwrap().position.y, 63.4);
        game.armor_stands[0].marker = false;
        game.armor_stands[0].small = true;
        game.remote_ride(Keys::default());
        close(game.player.as_ref().unwrap().position.y, 64.3875);
        game.armor_stands[0].small = false;
        game.remote_ride(Keys::default());
        close(game.player.as_ref().unwrap().position.y, 65.375);
        let kind = game.client_res().player_type().clone().unwrap();
        game.mobs
            .push(Mob::new(43, kind, Vec3::new(5.0, 70.0, 6.0), 0.0));
        game.armor_stands[0].marker = true;
        game.armor_stands[0].vehicle = Some(43);
        for at in [Vec3::new(5.0, 70.0, 6.0), Vec3::new(12.0, 72.0, -4.0)] {
            game.mobs[0].position = at;
            // No manual position_riders call: local ride must resolve the chain itself.
            game.remote_ride(Keys::default());
            let p = game.player.as_ref().unwrap().position;
            close(p.x, at.x);
            close(p.z, at.z);
            close(p.y, at.y + 1.2);
        }
        // Direct riding and a scaled avatar share the same attachment calculation.
        game.riding = Some(43);
        game.mobs[0].extra.model_scale = 1.5;
        game.remote_ride(Keys::default());
        close(game.player.as_ref().unwrap().position.y, 74.1);
        game.mobs.clear();
        game.armor_stands.clear();
        assert!(!game.remote_ride(Keys::default()));
        assert_eq!(game.riding, None);
    }
}
