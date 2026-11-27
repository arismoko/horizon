use super::*;
use crate::game::server_side::remote_server::riptide_tests::client_tick_to_server;
use std::sync::Arc;
use std::sync::mpsc::channel;
use sunrise_client_gameplay::game::remote::riptide_tests::OWNER;
use sunrise_client_gameplay::game::remote::riptide_tests::START;
use sunrise_client_gameplay::game::remote::riptide_tests::arena;
use sunrise_client_gameplay::game::remote::riptide_tests::server_to_client;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::Screen;
use sunrise_items::items::ItemStack;
use sunrise_protocol::packets::play::Serverbound as S;
use sunrise_sim::lan::PlayerSender;
use sunrise_sim::motion::PlayerMotion;
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay::game::native_guest_turn_tests::with_guests,
};

#[test]
#[ignore = "daily"]
fn riptide_ground_release_keeps_launch_velocity() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::client_side::remote_client::riptide_tests::riptide_ground_release_keeps_launch_velocity",
        || {
            with_guests(|server| {
                server.install_live_features();
                with_client_game(|client| {
                    let (mut peer, _writer) = tests::connect_menu_test(client);
                    tests::spawn_menu_test(client);
                    client.client.screen = Screen::Playing;
                    let remote = client.client.remote.as_mut().unwrap();
                    remote.loaded_sent = true;
                    remote.entity_id = OWNER as i32;
                    remote.profile = u128::from(OWNER);
                    remote.level_load.finish();

                    let (tx, rx) = channel();
                    let mut stack = ItemStack::new("minecraft:trident", 1);
                    stack.enchant("minecraft:riptide", 3);
                    for (grounded, roof) in [(true, false), (false, false), (true, true)] {
                        arena(server, roof, |g, c| {
                            g.server.streaming.insert_remote(&mut g.common.world, c);
                        });
                        arena(client, roof, |g, c| {
                            g.client.streaming.insert_remote(&mut g.common.world, c);
                        });
                        let start = if grounded {
                            START
                        } else {
                            Vec3::new(START.x, 65.0, START.z)
                        };
                        let mut body = PlayerMotion::new(client.motion, start).unwrap();
                        body.entity_id = OWNER as i32;
                        body.ground.on_ground = grounded;
                        client.player = Some(body);
                        client.bind_local_player_residency();
                        *client.held_mut() = Some(stack.clone());
                        assert!(
                            client.use_trident(&stack),
                            "the client must see water or rain"
                        );
                        client.using.as_mut().unwrap().1 = 20;

                        let hub = &mut server.server.lan.as_mut().unwrap().hub;
                        hub.singleplayer_owner = Some(u128::from(OWNER));
                        hub.update_guest(OWNER, |g| {
                            g.sender = PlayerSender::new(tx.clone(), Arc::default());
                            g.load_timeout = 0;
                            g.permission = 4;
                            g.position = start.components();
                            g.on_ground = grounded;
                            g.rests_on_ground = grounded;
                            g.motion = None;
                            g.inventory[0] = Some(stack.clone());
                        })
                        .unwrap();
                        server
                            .as_guest(OWNER, |game| {
                                assert!(
                                    game.use_trident(&stack),
                                    "the server must see water or rain"
                                );
                                game.using.as_mut().unwrap().1 = 20;
                            })
                            .unwrap();

                        client.release_remote_use();
                        let packets = client_tick_to_server(client, &mut peer, server);
                        assert!(
                            packets
                                .iter()
                                .any(|p| matches!(p, S::PlayerAction(a) if a.action == 6))
                        );
                        let teleports = server_to_client(&rx, client);
                        let body = client.player.as_ref().unwrap();
                        assert_eq!(body.auto_spin_attack_ticks, 20);
                        assert_eq!(
                            body.velocity,
                            Vec3::new(0.0, 0.0, 3.0),
                            "server cleared the Riptide push: {teleports:?}"
                        );
                        assert!(teleports.is_empty(), "a self-move is not a teleport");
                        let y_after_lift = match (grounded, roof) {
                            (true, false) => 64.0 + f64::from(1.1999999f32),
                            (true, true) => 66.0 - f64::from(1.8f32),
                            (false, _) => start.y,
                        };
                        assert_eq!(body.position.y, y_after_lift);
                        assert!(!body.ground.on_ground);
                        let guest = server
                            .server
                            .lan
                            .as_ref()
                            .unwrap()
                            .hub
                            .guest(OWNER)
                            .unwrap();
                        assert_eq!(guest.position, body.position.components());
                        assert_eq!(guest.motion.as_ref().unwrap().velocity, body.velocity);
                        assert!(!guest.on_ground && !guest.rests_on_ground);

                        for tick in 0..2 {
                            let z_before = client.player.as_ref().unwrap().position.z;
                            let input = client.tick_remote_own_player().unwrap().unwrap();
                            client.remote_send_state(input);
                            let packets = client_tick_to_server(client, &mut peer, server);
                            assert!(packets.iter().any(|p| matches!(
                                p,
                                S::MovePlayerPos(_) | S::MovePlayerPosRot(_)
                            )));
                            assert!(
                                server_to_client(&rx, client).is_empty(),
                                "server corrected launch tick {tick}"
                            );
                            let body = client.player.as_ref().unwrap();
                            assert!(body.position.z > z_before);
                            if tick == 0 && !roof {
                                assert_eq!(body.position.z, START.z + 3.0);
                            }
                            let guest = server
                                .server
                                .lan
                                .as_ref()
                                .unwrap()
                                .hub
                                .guest(OWNER)
                                .unwrap();
                            assert_eq!(guest.position, body.position.components());
                            assert_eq!(guest.on_ground, body.ground.on_ground);
                        }
                    }

                    // Chorus fruit still uses inferred position sync; commands explicitly
                    // mark the teleport. Neither is an ordinary self move.
                    server
                        .as_guest(OWNER, |g| {
                            assert!(g.random_teleport(Vec3::new(4.5, 70.0, 11.5), None));
                        })
                        .unwrap();
                    let teleports = server_to_client(&rx, client);
                    assert_eq!(teleports.len(), 1);
                    assert_eq!(teleports[0].position, [4.5, 64.0, 11.5]);
                    client_tick_to_server(client, &mut peer, server);
                    assert!(server_to_client(&rx, client).is_empty());

                    server
                        .as_guest(OWNER, |g| assert!(g.run_command("tp @s 8.5 72 10.5")))
                        .unwrap();
                    let teleports = server_to_client(&rx, client);
                    assert_eq!(teleports.len(), 1);
                    assert_eq!(teleports[0].position, [8.5, 72.0, 10.5]);
                    assert_eq!(
                        client.player.as_ref().unwrap().position.components(),
                        [8.5, 72.0, 10.5]
                    );
                });
            });
        },
    );
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
