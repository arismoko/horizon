use mod_api::SoundSource;
use std::sync::{Arc, mpsc};
use sunrise_gameplay::game::native_guest_turn_tests::with_guests;
use sunrise_gameplay::game::server_side::native_level_sound_except_tests::decode_packets;
use sunrise_gameplay_common::game::in_own_process;
use sunrise_protocol::packets::play::Clientbound as C;
use sunrise_sim::lan::PlayerSender;

/// A transaction that emits Nether effects and commits while the Overworld is active replays each
/// effect in its recorded `Dim` (so Nether events/particles/sounds never leak to Overworld guests
/// and `PlayerRef::play_sound` advances Nether's `world.random` rather than the Overworld's) and
/// delivers packets on a single connection in exact call order across all effect kinds (including
/// `GlobalEvent(1038)` followed by `StopSound`, both outside and inside `as_guest`).
#[test]
#[ignore = "daily"]
fn cross_level_transaction_replays_effects_in_request_level_and_preserves_packet_order() {
    in_own_process(
        "cut_layout::game::client_side::native_level_sound_except_tests_client::cross_level_transaction_replays_effects_in_request_level_and_preserves_packet_order",
        || {
            use sunrise_core::random::RandomSource;
            use sunrise_game_data::gamerules::Value;
            use sunrise_gameplay_common::game::Dim;
            use sunrise_level::particle_options::{ParticleOptions, ParticleType};

            with_guests(|game| {
                game.options.dedicated = Some(Vec::new());
                game.start_audio_capture();
                assert!(game.ui.rules.set("global_sound_events", Value::Bool(true)));
                let (tx12, rx12) = mpsc::channel();
                let (tx13, rx13) = mpsc::channel();
                game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                    g.sender = PlayerSender::new(tx12, Arc::default());
                });
                game.server.lan.as_mut().unwrap().hub.guests.update(1, |g| {
                    g.sender = PlayerSender::new(tx13, Arc::default());
                    g.dimension = "minecraft:the_nether".to_owned();
                });
                // Load Nether once so it sits in `levels.parked` when Overworld is active.
                game.enter_level(Dim::Nether).unwrap();
                let mut expected_nether_random = game.world.random.clone();
                game.enter_level(Dim::Overworld).unwrap();
                let mut expected_overworld_random = game.world.random.clone();

                let player13 = game.player(13).unwrap();
                let entity13 = player13.entity();
                let entity13_id = i32::try_from(entity13.id()).unwrap();
                let at = [3.0, 64.0, 5.0];
                let pos = [3, 64, 5];
                let anvil = "minecraft:block.anvil.use";
                let bell = "minecraft:block.bell.use";
                let end_portal_spawn = "minecraft:block.end_portal.spawn";
                let anvil_id = game
                    .server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .registry_id("minecraft:sound_event", anvil)
                    .unwrap();
                let bell_id = game
                    .server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .registry_id("minecraft:sound_event", bell)
                    .unwrap();
                let heart = ParticleOptions::simple(ParticleType::Heart);
                decode_packets(&rx12);
                decode_packets(&rx13);

                // Open an outer transaction in Overworld, switch to Nether and open a Nether
                // transaction to issue Nether effects across all effect kinds, commit the Nether
                // transaction, and switch back to Overworld before closing the outer transaction.
                let committed = game.storage_transaction(|game, _| {
                    game.enter_level(Dim::Nether).unwrap();
                    let nether = game.level("minecraft:the_nether").unwrap();
                    let nether_committed = game.storage_transaction(|game, _| {
                        assert!(game.level_global_level_event(nether, 1038, pos, 0));
                        assert!(game.player_stop_sound(player13, Some(end_portal_spawn), None));
                        assert!(game.player_play_sound(
                            player13,
                            anvil,
                            SoundSource::Neutral,
                            1.0,
                            1.0
                        ));
                        assert!(game.level_level_event(nether, 1500, pos, 0));
                        assert!(game.entity_broadcast_event(entity13, 35));
                        assert!(
                            game.level_send_particles(nether, &heart, None, at, [0.0; 3], 0.0, 2)
                        );
                        assert!(game.level_play_sound(
                            nether,
                            bell,
                            at,
                            SoundSource::Blocks,
                            1.0,
                            1.0,
                            None
                        ));
                        Ok::<(), ()>(())
                    });
                    assert!(nether_committed.is_ok());
                    game.enter_level(Dim::Overworld).unwrap();
                    Ok::<(), ()>(())
                });
                assert!(committed.is_ok());
                assert_eq!(game.dim, Dim::Overworld);

                // Overworld guest 12 receives ONLY the global level event (1038); no Nether-local
                // level event, entity event, particles, or sounds leak into the Overworld.
                let ow_packets = decode_packets(&rx12);
                assert_eq!(
                    ow_packets.len(),
                    1,
                    "only global event 1038 reaches Overworld guest 12"
                );
                assert!(matches!(
                    ow_packets[0],
                    C::LevelEvent(ref e) if e.kind == 1038 && e.global
                ));

                // Nether guest 13 receives every packet on one connection in exact call order, and
                // `PlayerRef::play_sound` drew its seed from Nether's `world.random`.
                let expected_nether_seed = expected_nether_random.next_long();
                let nether_packets = decode_packets(&rx13);
                let tags: Vec<&'static str> = nether_packets
                    .iter()
                    .map(|p| match p {
                        C::LevelEvent(e) if e.kind == 1038 && e.global => "global_1038",
                        C::StopSound(s) if s.sound.as_deref() == Some(end_portal_spawn) => {
                            "stop_portal"
                        }
                        C::Sound(s) if s.sound.0 == anvil_id && s.seed == expected_nether_seed => {
                            "player_anvil"
                        }
                        C::LevelEvent(e) if e.kind == 1500 && !e.global => "level_1500",
                        C::EntityEvent(e) if e.id == entity13_id && e.event == 35 => "entity_35",
                        C::LevelParticles(p) if p.count == 2 => "particles_2",
                        C::Sound(s) if s.sound.0 == bell_id => "level_bell",
                        other => panic!("unexpected Nether packet: {other:?}"),
                    })
                    .collect();
                assert_eq!(
                    tags,
                    vec![
                        "global_1038",
                        "stop_portal",
                        "player_anvil",
                        "level_1500",
                        "entity_35",
                        "particles_2",
                        "level_bell",
                    ]
                );

                // Parked-level effects issued while Overworld is active and an Overworld
                // transaction is open replay into the Nether on Overworld commit in exact order
                // and stay discarded on abort.
                let nether_handle = game.level("minecraft:the_nether").unwrap();
                let cross_abort = game.storage_transaction(|game, _| {
                    assert!(game.level_send_particles(
                        nether_handle,
                        &heart,
                        None,
                        at,
                        [0.0; 3],
                        0.0,
                        2
                    ));
                    assert!(game.level_play_sound(
                        nether_handle,
                        bell,
                        at,
                        SoundSource::Blocks,
                        1.0,
                        1.0,
                        None
                    ));
                    Err::<(), ()>(())
                });
                assert!(cross_abort.is_err());
                assert!(decode_packets(&rx12).is_empty());
                assert!(decode_packets(&rx13).is_empty());

                let cross_commit = game.storage_transaction(|game, _| {
                    assert!(game.level_send_particles(
                        nether_handle,
                        &heart,
                        None,
                        at,
                        [0.0; 3],
                        0.0,
                        2
                    ));
                    assert!(game.level_play_sound(
                        nether_handle,
                        bell,
                        at,
                        SoundSource::Blocks,
                        1.0,
                        1.0,
                        None
                    ));
                    Ok::<(), ()>(())
                });
                assert!(cross_commit.is_ok());
                assert_eq!(game.dim, Dim::Overworld);
                assert!(decode_packets(&rx12).is_empty());
                let cross_tags: Vec<&'static str> = decode_packets(&rx13)
                    .iter()
                    .map(|p| match p {
                        C::LevelParticles(p) if p.count == 2 => "particles_2",
                        C::Sound(s) if s.sound.0 == bell_id => "level_bell",
                        other => panic!("unexpected parked-Nether packet: {other:?}"),
                    })
                    .collect();
                assert_eq!(cross_tags, vec!["particles_2", "level_bell"]);

                // Overworld's `world.random` must remain completely untouched by Nether's `play_sound`.
                assert_eq!(
                    game.world.random.clone().next_long(),
                    expected_overworld_random.next_long(),
                    "Nether PlayerRef::play_sound must not advance Overworld world.random"
                );

                // Packet order also holds inside an `as_guest` turn in BOTH directions
                // (`GlobalEvent(1038)` -> `StopSound` and `StopSound` -> `GlobalEvent(1038)`).
                game.enter_level(Dim::Nether).unwrap();
                let nether = game.level("minecraft:the_nether").unwrap();
                decode_packets(&rx13);
                game.as_guest(13, |game| {
                    let acting = game.player(13).unwrap();
                    let res = game.storage_transaction(|game, _| {
                        assert!(game.player_stop_sound(acting, Some(anvil), None));
                        assert!(game.level_global_level_event(nether, 1038, pos, 0));
                        assert!(game.player_stop_sound(acting, Some(end_portal_spawn), None));
                        Ok::<(), ()>(())
                    });
                    assert!(res.is_ok());
                })
                .expect("guest turn runs");
                let turn_tags: Vec<&'static str> = decode_packets(&rx13)
                    .into_iter()
                    .filter_map(|p| match p {
                        C::StopSound(s) if s.sound.as_deref() == Some(anvil) => Some("stop_anvil"),
                        C::LevelEvent(e) if e.kind == 1038 && e.global => Some("global_1038"),
                        C::StopSound(s) if s.sound.as_deref() == Some(end_portal_spawn) => {
                            Some("stop_portal")
                        }
                        _ => None,
                    })
                    .collect();
                assert_eq!(turn_tags, vec!["stop_anvil", "global_1038", "stop_portal"]);
            });
        },
    );
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;

#[test]
#[ignore = "daily"]
fn entity_sounds_bound_sounds_particles_and_level_events_defer_inside_transactions() {
    in_own_process(
        "cut_layout::game::client_side::native_level_sound_except_tests_client::entity_sounds_bound_sounds_particles_and_level_events_defer_inside_transactions",
        || {
            use sunrise_core::random::{LegacyRandom, RandomSource};
            use sunrise_game_data::gamerules::Value;
            use {
                sunrise_level::particle_options::ParticleOptions,
                sunrise_level::particle_options::ParticleType,
            };

            use std::sync::atomic::{AtomicU32, Ordering};
            use sunrise_gameplay::game::native_guest_turn_tests::add_fixture_guests;

            crate::game::integrated_pair::with_integrated_pair(|pair| {
                let at = [3.0, 64.0, 5.0];
                let client = &mut *pair.client;
                client.player = Some(
                    sunrise_sim::motion::PlayerMotion::new(
                        client.motion,
                        sunrise_core::geometry::Vec3::new(at[0], at[1], at[2]),
                    )
                    .unwrap(),
                );
                client
                    .test_audio_mut()
                    .set_listener(at, [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]);
                let heard = Arc::new(AtomicU32::new(0));
                let observed = heard.clone();
                pair.call_with_client(move |game, fence| {
                add_fixture_guests(game);
                game.options.dedicated = Some(Vec::new());
                game.start_audio_capture();
                let (tx12, rx12) = mpsc::channel();
                let (tx13, rx13) = mpsc::channel();
                let hub = &mut game.server.lan.as_mut().unwrap().hub;
                hub.update_guest(12, |g| {
                    g.sender = PlayerSender::new(tx12, Arc::default());
                }).unwrap();
                hub.update_guest(13, |g| {
                    g.sender = PlayerSender::new(tx13, Arc::default());
                }).unwrap();
                let player13 = game.player(13).unwrap();
                let entity13 = player13.entity();
                let entity13_id = i32::try_from(entity13.id()).unwrap();
                let level = game.level(game.dim.id()).unwrap();
                let pos = [3, 64, 5];
                game.player = Some(
                    sunrise_sim::motion::PlayerMotion::new(
                        game.motion,
                        sunrise_core::geometry::Vec3::new(at[0], at[1], at[2]),
                    )
                    .unwrap(),
                );
                let mut portal_plays = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                    fence(game);
                    heard.load(Ordering::Relaxed)
                };
                let burp = "minecraft:entity.player.burp";
                let bell = "minecraft:block.bell.use";
                let anvil = "minecraft:block.anvil.use";
                let burp_id = game
                    .server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .registry_id("minecraft:sound_event", burp)
                    .unwrap();
                let bell_id = game
                    .server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .registry_id("minecraft:sound_event", bell)
                    .unwrap();
                let anvil_id = game
                    .server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .registry_id("minecraft:sound_event", anvil)
                    .unwrap();
                let heart = ParticleOptions::simple(ParticleType::Heart);
                let flame = ParticleOptions::simple(ParticleType::Flame);

                let clear_queues = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                    game.server.pending_native_sounds.clear();
                    game.server.pending_particles.clear();
                    game.level_events.clear();
                    game.server.lan_events.clear();
                    game.server.lan_global_events.clear();
                    game.server.lan_entity_events.clear();
                    game.server.pending_sounds.clear();
                };
                clear_queues(game);
                decode_packets(&rx12);
                decode_packets(&rx13);

                game.server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .set_sound_seeds(LegacyRandom::new(0x5A17_CAFE));
                let mut expected_sound_seeds = LegacyRandom::new(0x5A17_CAFE);
                let mut expected_level_random = game.world.random.clone();

                let portal_before_abort = portal_plays(game);
                assert!(game.ui.rules.set("global_sound_events", Value::Bool(true)));
                let rolled = game.storage_transaction(|game, _| {
                    assert!(game.entity_play_sound(entity13, burp, 1.0, 1.0));
                    assert!(game.entity_play_bound_sound(
                        entity13,
                        bell,
                        SoundSource::Players,
                        1.0,
                        1.0
                    ));
                    assert!(game.player_play_sound(
                        player13,
                        anvil,
                        SoundSource::Neutral,
                        1.0,
                        1.0
                    ));
                    assert!(game.player_stop_sound(
                        player13,
                        Some(anvil),
                        Some(SoundSource::Neutral)
                    ));
                    assert!(game.entity_broadcast_event(entity13, 35));
                    assert!(game.level_send_particles(level, &heart, None, at, [0.0; 3], 0.0, 2));
                    assert!(game.level_level_event(level, 1500, pos, 0));
                    assert!(game.level_global_level_event(level, 1038, pos, 0));
                    assert!(game.server.pending_native_sounds.is_empty());
                    assert!(game.server.pending_particles.is_empty());
                    assert!(game.server.lan_events.is_empty());
                    assert!(game.server.lan_global_events.is_empty());
                    assert!(game.server.lan_entity_events.is_empty());
                    assert_eq!(portal_plays(game), portal_before_abort);
                    assert!(decode_packets(&rx12).is_empty());
                    assert!(decode_packets(&rx13).is_empty());
                    Err::<(), ()>(())
                });
                assert!(rolled.is_err());
                assert!(game.server.pending_native_sounds.is_empty());
                assert!(game.server.pending_particles.is_empty());
                assert!(game.server.lan_events.is_empty());
                assert!(game.server.lan_global_events.is_empty());
                assert!(game.server.lan_entity_events.is_empty());
                assert_eq!(
                    portal_plays(game),
                    portal_before_abort,
                    "rolled-back global_level_event must not increase local End-portal playback count"
                );
                assert!(decode_packets(&rx12).is_empty());
                assert!(decode_packets(&rx13).is_empty());
                assert_eq!(
                    game.world.random.clone().next_long(),
                    expected_level_random.clone().next_long(),
                    "rolled-back transaction must not advance world.random"
                );

                let portal_before_commit = portal_plays(game);
                let committed = game.storage_transaction(|game, _| {
                    let aborted_child = game.storage_transaction(|game, _| {
                        assert!(game.entity_play_sound(entity13, anvil, 1.0, 1.0));
                        assert!(game.entity_play_bound_sound(
                            entity13,
                            anvil,
                            SoundSource::Players,
                            1.0,
                            1.0
                        ));
                        assert!(game.player_play_sound(
                            player13,
                            burp,
                            SoundSource::Neutral,
                            1.0,
                            1.0
                        ));
                        assert!(game.player_stop_sound(
                            player13,
                            Some(burp),
                            Some(SoundSource::Neutral)
                        ));
                        assert!(game.entity_broadcast_event(entity13, 2));
                        assert!(
                            game.level_send_particles(level, &flame, None, at, [0.0; 3], 0.0, 4)
                        );
                        assert!(game.level_level_event(level, 1000, pos, 0));
                        assert!(game.level_global_level_event(level, 1028, pos, 0));
                        Err::<(), ()>(())
                    });
                    assert!(aborted_child.is_err());

                    let kept_child = game.storage_transaction(|game, _| {
                        assert!(game.entity_play_sound(entity13, burp, 1.0, 1.0));
                        assert!(game.entity_play_bound_sound(
                            entity13,
                            bell,
                            SoundSource::Players,
                            1.0,
                            1.0
                        ));
                        assert!(game.player_play_sound(
                            player13,
                            anvil,
                            SoundSource::Neutral,
                            1.0,
                            1.0
                        ));
                        assert!(game.player_stop_sound(
                            player13,
                            Some(anvil),
                            Some(SoundSource::Neutral)
                        ));
                        assert!(game.entity_broadcast_event(entity13, 35));
                        assert!(
                            game.level_send_particles(level, &heart, None, at, [0.0; 3], 0.0, 3)
                        );
                        assert!(game.level_level_event(level, 1500, pos, 0));
                        assert!(game.level_global_level_event(level, 1038, pos, 0));
                        Ok::<(), ()>(())
                    });
                    assert!(kept_child.is_ok());

                    // Nothing published before outermost commit.
                    assert!(game.server.pending_native_sounds.is_empty());
                    assert!(game.server.pending_particles.is_empty());
                    assert!(game.server.lan_events.is_empty());
                    assert!(game.server.lan_global_events.is_empty());
                    assert!(game.server.lan_entity_events.is_empty());
                    assert_eq!(portal_plays(game), portal_before_commit);
                    assert!(decode_packets(&rx12).is_empty());
                    assert!(decode_packets(&rx13).is_empty());
                    Ok::<(), ()>(())
                });
                assert!(committed.is_ok());

                // Committed effects deliver directly without waiting for `lan_flush_effects`.
                assert!(game.server.lan_events.is_empty());
                assert!(game.server.lan_global_events.is_empty());
                assert!(game.server.lan_entity_events.is_empty());

                assert_eq!(
                    portal_plays(game),
                    portal_before_commit + 1,
                    "global_level_event plays its local global sound once on root commit"
                );
                let packets12 = decode_packets(&rx12);
                let packets13 = decode_packets(&rx13);
                let expected_burp_seed = expected_sound_seeds.next_long();
                let expected_bell_seed = expected_sound_seeds.next_long();
                let expected_anvil_seed = expected_level_random.next_long();

                let sounds12: Vec<_> = packets12
                    .iter()
                    .filter_map(|p| match p {
                        C::Sound(s) => Some((s.sound.0, s.seed)),
                        _ => None,
                    })
                    .collect();
                let sounds13: Vec<_> = packets13
                    .iter()
                    .filter_map(|p| match p {
                        C::Sound(s) => Some((s.sound.0, s.seed)),
                        _ => None,
                    })
                    .collect();
                assert_eq!(
                    sounds12,
                    vec![(burp_id, expected_burp_seed)],
                    "entity_play_sound reaches nearby guest 12 only with the first sound seed"
                );
                assert_eq!(
                    sounds13,
                    vec![(anvil_id, expected_anvil_seed)],
                    "guest 13 receives only player_play_sound with the first world.random seed"
                );
                let stops12: Vec<_> = packets12
                    .iter()
                    .filter(|p| matches!(p, C::StopSound(_)))
                    .collect();
                let stops13: Vec<_> = packets13
                    .iter()
                    .filter_map(|p| match p {
                        C::StopSound(s) => Some(s.sound.as_deref()),
                        _ => None,
                    })
                    .collect();
                assert!(stops12.is_empty());
                assert_eq!(stops13, vec![Some(anvil)]);
                let bound12: Vec<_> = packets12
                    .iter()
                    .filter_map(|p| match p {
                        C::SoundEntity(s) => Some((s.sound.0, s.id, s.seed)),
                        _ => None,
                    })
                    .collect();
                let bound13: Vec<_> = packets13
                    .iter()
                    .filter_map(|p| match p {
                        C::SoundEntity(s) => Some((s.sound.0, s.id, s.seed)),
                        _ => None,
                    })
                    .collect();
                assert_eq!(bound12, vec![(bell_id, entity13_id, expected_bell_seed)]);
                assert_eq!(bound13, vec![(bell_id, entity13_id, expected_bell_seed)]);
                let particles12: Vec<_> = packets12
                    .iter()
                    .filter_map(|p| match p {
                        C::LevelParticles(p) => Some(p.count),
                        _ => None,
                    })
                    .collect();
                let particles13: Vec<_> = packets13
                    .iter()
                    .filter_map(|p| match p {
                        C::LevelParticles(p) => Some(p.count),
                        _ => None,
                    })
                    .collect();
                assert_eq!(particles12, vec![3]);
                assert_eq!(particles13, vec![3]);
                let level_events12: Vec<_> = packets12
                    .iter()
                    .filter_map(|p| match p {
                        C::LevelEvent(e) => Some((e.kind, e.global)),
                        _ => None,
                    })
                    .collect();
                let level_events13: Vec<_> = packets13
                    .iter()
                    .filter_map(|p| match p {
                        C::LevelEvent(e) => Some((e.kind, e.global)),
                        _ => None,
                    })
                    .collect();
                assert_eq!(level_events12, vec![(1500, false), (1038, true)]);
                assert_eq!(level_events13, vec![(1500, false), (1038, true)]);
                let entity_events13: Vec<_> = packets13
                    .iter()
                    .filter_map(|p| match p {
                        C::EntityEvent(e) => Some((e.id, e.event)),
                        _ => None,
                    })
                    .collect();
                assert_eq!(entity_events13, vec![(entity13_id, 35)]);

                assert!(game.ui.rules.set("global_sound_events", Value::Bool(false)));
                clear_queues(game);
                decode_packets(&rx12);
                decode_packets(&rx13);
                let rolled_plain = game.storage_transaction(|game, _| {
                    assert!(game.level_global_level_event(level, 1023, pos, 0));
                    assert!(game.server.lan_events.is_empty());
                    assert!(game.server.lan_global_events.is_empty());
                    assert!(game.server.pending_sounds.is_empty());
                    Err::<(), ()>(())
                });
                assert!(rolled_plain.is_err());
                assert!(game.server.lan_events.is_empty());
                assert!(game.server.lan_global_events.is_empty());
                assert!(game.server.pending_sounds.is_empty());

                let committed_plain = game.storage_transaction(|game, _| {
                    assert!(game.level_global_level_event(level, 1023, pos, 0));
                    assert!(game.server.lan_events.is_empty());
                    Ok::<(), ()>(())
                });
                assert!(committed_plain.is_ok());
                assert!(game.server.lan_global_events.is_empty());
                assert!(game.server.lan_events.is_empty());
                assert!(game.server.pending_sounds.is_empty());
                let plain12: Vec<_> = decode_packets(&rx12)
                    .into_iter()
                    .filter_map(|p| match p {
                        C::LevelEvent(e) => Some((e.kind, e.global)),
                        _ => None,
                    })
                    .collect();
                assert_eq!(plain12, vec![(1023, false)]);
                decode_packets(&rx13);

                // 4. Outside a transaction, every effect path continues queueing/sending immediately.
                assert!(game.ui.rules.set("global_sound_events", Value::Bool(true)));
                clear_queues(game);
                assert!(game.entity_play_sound(entity13, burp, 1.0, 1.0));
                assert!(game.entity_play_bound_sound(
                    entity13,
                    bell,
                    SoundSource::Players,
                    1.0,
                    1.0
                ));
                assert!(game.player_play_sound(player13, anvil, SoundSource::Neutral, 1.0, 1.0));
                assert!(game.player_stop_sound(player13, Some(anvil), Some(SoundSource::Neutral)));
                assert!(game.entity_broadcast_event(entity13, 35));
                assert!(game.level_send_particles(level, &heart, None, at, [0.0; 3], 0.0, 1));
                assert!(game.level_level_event(level, 1500, pos, 0));
                assert!(game.level_global_level_event(level, 1038, pos, 0));
                assert_eq!(game.server.pending_native_sounds.len(), 2);
                assert_eq!(game.server.pending_native_sounds[0].entity, None);
                assert_eq!(
                    game.server.pending_native_sounds[1].entity,
                    Some(entity13_id)
                );
                assert_eq!(game.server.pending_particles.len(), 1);
                assert_eq!(game.server.lan_events.len(), 1);
                assert_eq!(game.server.lan_global_events.len(), 1);
                assert_eq!(game.server.lan_entity_events, vec![(entity13_id, 35)]);
                let direct13 = decode_packets(&rx13);
                assert!(direct13.iter().any(|p| matches!(p, C::Sound(_))));
                assert!(direct13.iter().any(|p| matches!(p, C::StopSound(_))));
                }, |client| {
                    observed.store(
                        client.test_audio_mut().heard.get("minecraft:block.end_portal.spawn").copied().unwrap_or(0),
                        Ordering::Relaxed,
                    );
                });
            });
        },
    );
}
