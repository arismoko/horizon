use crate::game::client_side::native_hotbar_slot::hotbar_slot_tests::logged_in;
use crate::game::server_side::native_entity_refs::tests::with_client_game;
use mod_api::{ClientHost, InteractionAttempt};
use std::sync::Arc;
use sunrise_client_gameplay::game::remote::gamemode_actions_tests::add_entity;
use sunrise_client_gameplay::game::remote::gamemode_actions_tests::outgoing;
use sunrise_client_gameplay::game::remote::gamemode_actions_tests::ranged_stick;
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::Chunk;
use sunrise_protocol::packets::play::{self, Clientbound as C, Serverbound as S};
use sunrise_sim::first_person_sim::Hand;

#[test]
#[ignore = "daily"]
fn ordinary_entity_attacks_check_both_range_edges_and_set_only_instabuild_delay() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::client_side::remote_client::gamemode_actions_tests::ordinary_entity_attacks_check_both_range_edges_and_set_only_instabuild_delay",
        || {
            with_client_game(|game| {
                let mut reader = logged_in(game);
                game.common.inventory[game.common.selected] = Some(ranged_stick(1.5, 2.5, 0.125));
                let min = 1.375_f64;
                let max = 2.625_f64;
                for (id, kind) in [
                    (101, "minecraft:oak_boat"),
                    (102, "minecraft:armor_stand"),
                    (103, "minecraft:end_crystal"),
                ] {
                    add_entity(game, id, kind, [0.5, 65.0, 2.5]);
                    assert!(game.remote_attacked_object(id).is_some(), "fixture: {kind}");
                    // Deliberately set only the ability, independent of the client game mode.
                    for instabuild in [false, true] {
                        game.handle_packet(C::PlayerAbilities(play::PlayerAbilities {
                            flags: if instabuild { 8 } else { 0 },
                            flying_speed: 0.05,
                            walking_speed: 0.1,
                        }))
                        .unwrap();
                        for (distance, allowed, delay_before) in [
                            (min.next_down(), false, 0),
                            (min, true, 0),
                            (max, true, 0),
                            (max.next_up(), false, 3),
                        ] {
                            game.client.hand_strength.ticker = 100;
                            game.client.hand_strength.swap_ticker = 100;
                            game.client.remote.as_mut().unwrap().destroy_delay = delay_before;
                            let case = (kind, instabuild, distance.to_bits());
                            assert_eq!(
                                game.remote_attack_entity(id, distance),
                                allowed,
                                "{case:?}"
                            );
                            let attacks: Vec<_> = outgoing(game, &mut reader)
                                .into_iter()
                                .filter_map(|packet| match packet {
                                    S::Attack(attack) => Some(attack.id),
                                    _ => None,
                                })
                                .collect();
                            assert!(
                                attacks.iter().copied().eq(allowed.then_some(id)),
                                "{case:?}: {attacks:?}"
                            );
                            let strength = if allowed { 0 } else { 100 };
                            assert_eq!(game.client.hand_strength.ticker, strength, "{case:?}");
                            assert_eq!(game.client.hand_strength.swap_ticker, strength, "{case:?}");
                            assert_eq!(
                                game.client.remote.as_ref().unwrap().destroy_delay,
                                if instabuild && allowed {
                                    5
                                } else {
                                    delay_before
                                },
                                "{case:?}"
                            );
                        }
                    }
                }
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn out_of_range_entity_click_still_swings_without_attacking_or_resetting() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::client_side::remote_client::gamemode_actions_tests::out_of_range_entity_click_still_swings_without_attacking_or_resetting",
        || {
            with_client_game(|game| {
                let mut reader = logged_in(game);
                game.client.title.0 = None;
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                add_entity(game, 101, "minecraft:armor_stand", [0.5, 65.0, 2.0]);
                game.common.inventory[game.common.selected] = Some(ranged_stick(2.0, 3.0, 0.0));
                game.refresh_client_pick(1.0);
                assert_eq!(game.crosshair_entity_id(), Some(101));
                game.client.hand_strength.ticker = 100;
                let swings = game.swings_started;
                game.remote_start_attack();
                let packets = outgoing(game, &mut reader);
                assert_eq!(
                    packets.iter().filter(|p| matches!(p, S::Punch(_))).count(),
                    1
                );
                assert!(!packets.iter().any(|p| matches!(p, S::Attack(_))));
                assert_eq!(game.swings_started, swings + 1);
                assert_eq!(game.client.hand_strength.ticker, 100);
                assert_eq!(game.clicks.miss_time, 0);
                assert_eq!(game.client.remote.as_ref().unwrap().destroy_delay, 0);

                // The stored hit point has not moved, but the player is now far enough away.
                // The original pick-ray distance must not be used for the attack check.
                game.player.as_mut().unwrap().position.z = -0.5;
                game.remote_start_attack();
                assert!(
                    outgoing(game, &mut reader)
                        .iter()
                        .any(|p| matches!(p, S::Attack(attack) if attack.id == 101))
                );
                assert_eq!(game.client.hand_strength.ticker, 0);
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn spectator_entity_use_sends_both_hands_without_feeding_or_swinging() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::client_side::remote_client::gamemode_actions_tests::spectator_entity_use_sends_both_hands_without_feeding_or_swinging",
        || {
            with_client_game(|game| {
                let mut reader = logged_in(game);
                game.client.title.0 = None;
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                add_entity(game, 101, "minecraft:horse", [0.5, 66.1, 2.5]);
                game.mobs[0].extra.age = -24000;
                let wheat = Some(ItemStack::new("minecraft:wheat", 3));
                game.common.inventory[game.common.selected] = wheat.clone();
                game.offhand = wheat.clone();
                game.client.particles.0.requested = Some(Vec::new());
                game.refresh_client_pick(1.0);
                assert_eq!(game.crosshair_entity_id(), Some(101));
                // Positive control: this is a feed that predicts a particle in survival.
                assert!(game.remote_use_entity(Hand::Main));
                let requested = game.client.particles.0.requested.as_mut().unwrap();
                assert_eq!(requested.len(), 1);
                requested.clear();
                outgoing(game, &mut reader);

                game.handle_packet(C::GameEvent(play::GameEvent {
                    event: 3,
                    param: 3.0,
                }))
                .unwrap();
                for id in [101, 102, 103] {
                    if id == 102 {
                        // A boat that normally consumes the main-hand click is also PASS.
                        add_entity(game, id, "minecraft:oak_boat", [0.5, 66.5, 1.5]);
                    } else if id == 103 {
                        // Remove the closer boat so the crosshair picks the remote leash knot;
                        // knots have no API entity body, and spectator must still send both hands
                        // with hit locations relative to the knot's origin.
                        game.handle_packet(C::RemoveEntities(play::RemoveEntities {
                            ids: vec![102],
                        }))
                        .unwrap();
                        add_entity(game, id, "minecraft:leash_knot", [0.0, 66.0, 1.0]);
                    }
                    game.refresh_client_pick(1.0);
                    assert_eq!(game.crosshair_entity_id(), Some(id));
                    let expected_loc = if id == 103 {
                        let Some(mod_api::CrosshairTarget::Entity { point, .. }) =
                            game.crosshair_target()
                        else {
                            panic!("expected crosshair knot target");
                        };
                        let knot_origin = sunrise_sim::mobs::leash::knot_state([0, 66, 1]).position;
                        let raw = [
                            point[0] - knot_origin.x,
                            point[1] - knot_origin.y,
                            point[2] - knot_origin.z,
                        ];
                        // Round-trip through LpVec3 so the assertion checks the exact wire value.
                        let encoded = S::Interact(play::Interact {
                            id,
                            hand: 0,
                            location: raw,
                            secondary: false,
                        })
                        .encode();
                        let mut r = sunrise_protocol::buf::Reader::new(&encoded);
                        let pkt_id = r.varint().unwrap();
                        let S::Interact(decoded) = S::decode(pkt_id, r.rest()).unwrap() else {
                            panic!("expected Interact");
                        };
                        Some(decoded.location)
                    } else {
                        None
                    };
                    let swings = game.swings_started;
                    game.remote_use(InteractionAttempt::Press);
                    let packets = outgoing(game, &mut reader);
                    let interactions: Vec<_> = packets
                        .iter()
                        .filter_map(|packet| match packet {
                            S::Interact(interact) => {
                                Some((interact.id, interact.hand, interact.location))
                            }
                            _ => None,
                        })
                        .collect();
                    assert_eq!(
                        interactions
                            .iter()
                            .map(|(e, h, _)| (*e, *h))
                            .collect::<Vec<_>>(),
                        [(id, 0), (id, 1)]
                    );
                    if let Some(expected) = expected_loc {
                        for (_, _, loc) in &interactions {
                            assert_eq!(*loc, expected);
                        }
                    }
                    assert!(
                        !packets
                            .iter()
                            .any(|p| matches!(p, S::UseItem(_) | S::UseItemOn(_) | S::Punch(_)))
                    );
                    assert!(
                        game.client
                            .particles
                            .0
                            .requested
                            .as_ref()
                            .unwrap()
                            .is_empty()
                    );
                    assert_eq!(game.swings_started, swings);
                    assert_eq!(game.common.inventory[game.common.selected], wheat);
                    assert_eq!(game.offhand, wheat);
                    assert!(game.using.is_none());
                    if let Some(expected) = expected_loc {
                        // Ordinary survival clicks on a remote knot spawned at fence block
                        // coordinates must also subtract the canonical knot origin, not the
                        // raw AddEntity block corner.
                        game.handle_packet(C::GameEvent(play::GameEvent {
                            event: 3,
                            param: 0.0,
                        }))
                        .unwrap();
                        game.refresh_client_pick(1.0);
                        assert_eq!(game.crosshair_entity_id(), Some(103));
                        game.remote_use(InteractionAttempt::Press);
                        let survival_packets = outgoing(game, &mut reader);
                        let survival_interactions: Vec<_> = survival_packets
                            .iter()
                            .filter_map(|packet| match packet {
                                S::Interact(interact) => Some((interact.id, interact.location)),
                                _ => None,
                            })
                            .collect();
                        assert!(
                            !survival_interactions.is_empty(),
                            "expected survival Interact on knot 103, got {survival_packets:?}"
                        );
                        for (target_id, loc) in survival_interactions {
                            assert_eq!(target_id, 103);
                            assert_eq!(loc, expected);
                        }
                    }
                }

                // Minecraft.startUseItem: an entity whose blockPosition() is outside the world
                // border aborts the entire click with no Interact, UseItem, or off-hand turn,
                // in both spectator and survival modes.
                game.handle_packet(C::SetBorderCenter(play::SetBorderCenter { x: 0.0, z: 0.0 }))
                    .unwrap();
                game.handle_packet(C::SetBorderSize(play::SetBorderSize { size: 2.0 }))
                    .unwrap();
                // Border spans [-1.0, 1.0); entity 103's blockPosition is (0, 66, 1) -> outside.
                let snowball = Some(ItemStack::new("minecraft:snowball", 2));
                game.common.inventory[game.common.selected] = snowball.clone();
                game.offhand = snowball;
                for mode in [3.0_f32, 0.0_f32] {
                    game.handle_packet(C::GameEvent(play::GameEvent {
                        event: 3,
                        param: mode,
                    }))
                    .unwrap();
                    game.refresh_client_pick(1.0);
                    assert_eq!(game.crosshair_entity_id(), Some(103));
                    let swings = game.swings_started;
                    game.remote_use(InteractionAttempt::Press);
                    let packets = outgoing(game, &mut reader);
                    assert!(
                        !packets.iter().any(|p| matches!(
                            p,
                            S::Interact(_) | S::UseItem(_) | S::UseItemOn(_) | S::Punch(_)
                        )),
                        "mode {mode}: expected click abort outside world border, got {packets:?}"
                    );
                    assert_eq!(game.swings_started, swings);
                }
            });
        },
    );
}

/// `MultiPlayerGameMode.destroyDelay` through the mod host: 0 outside a world, 5 after a
/// creative attack, one less for each continued-destroy tick.
#[test]
#[ignore = "daily"]
fn mining_destroy_delay_reads_five_after_a_creative_attack_then_counts_down() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::client_side::remote_client::gamemode_actions_tests::mining_destroy_delay_reads_five_after_a_creative_attack_then_counts_down",
        || {
            with_client_game(|game| {
                let read = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
                    let host: &dyn ClientHost = game;
                    host.mining_destroy_delay()
                };
                assert_eq!(read(game), 0);
                let _reader = logged_in(game);
                game.common.inventory[game.common.selected] = Some(ranged_stick(1.5, 2.5, 0.125));
                add_entity(game, 101, "minecraft:armor_stand", [0.5, 65.0, 2.5]);
                game.handle_packet(C::PlayerAbilities(play::PlayerAbilities {
                    flags: 8,
                    flying_speed: 0.05,
                    walking_speed: 0.1,
                }))
                .unwrap();
                assert_eq!(read(game), 0);
                game.client.hand_strength.ticker = 100;
                assert!(game.remote_attack_entity(101, 2.0));
                assert_eq!(read(game), 5);
                assert!(game.remote_mining(false));
                assert_eq!(read(game), 4);
            });
        },
    );
}
