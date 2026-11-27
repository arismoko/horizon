#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {

    use mod_api::__private::game::Fields;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_server::NativeServerGameExt1;
    use sunrise_gameplay_common::game::server_side::native_sound_refs::tests::guest;

    use sunrise_gameplay_common::game::server_side::native_sound_refs::tests::packets;

    use sunrise_gameplay_common::game::lan_common::LanSettings;

    use super::super::native_entity_refs::tests::with_game;
    use sunrise_gameplay_common::game::test_fixtures::fixture_resident_column;
    use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

    use mod_api::{SoundSource, WorldEpoch};

    use sunrise_core::geometry::Vec3;

    use sunrise_protocol::packets::play::Clientbound;

    use sunrise_sim::lan::{NetConfig, hub::HubConfig};

    use sunrise_sim::mobs::Mob;

    #[test]
    #[ignore = "daily"]
    fn sounds_reach_the_right_connections() {
        with_game(|game| {
            let net = NetConfig {
                listen: false,
                announce: false,
                lod_distance: 0,
                ..NetConfig::lan(0, String::new(), 3)
            };
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: game.dim.id().to_owned(),
                allow_flight: true,
                seed: 0,
                max_players: 3,
                integrated: true,
                levels: game.server.level_shapes.clone(),
                level_seeds: game.server.levels.0.runtime.seeds(),
            };
            let settings = LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            };
            game.lan_host(settings, net, config).unwrap();
            let (a, rx_a) = guest(21, 21, [1.0, 64.0, 2.0]);
            let (b, rx_b) = guest(22, 22, [3.0, 64.0, 2.0]);
            let (far, rx_far) = guest(23, 23, [900.0, 64.0, 2.0]);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .extend([a, b, far]);
            let pa = game.player(21).unwrap();
            let pb = game.player(22).unwrap();
            let sound = "minecraft:entity.player.burp";

            // stop_sound: only the named player's connection.
            assert!(game.player_stop_sound(pa, Some(sound), Some(SoundSource::Players)));
            assert!(game.player_stop_sound(pa, None, None));
            assert!(!game.player_stop_sound(pa, Some("bad id"), None));
            let got = packets(&rx_a);
            assert_eq!(got.len(), 2);
            let Clientbound::StopSound(first) = &got[0] else {
                panic!("expected stop sound")
            };
            assert_eq!(first.sound.as_deref(), Some(sound));
            assert_eq!(first.source, Some(SoundSource::Players.index() as i32));
            let Clientbound::StopSound(second) = &got[1] else {
                panic!("expected stop sound")
            };
            assert_eq!((second.sound.clone(), second.source), (None, None));
            assert!(packets(&rx_b).is_empty());

            // play_sound: one packet for one player, at their position.
            assert!(game.player_play_sound(pb, sound, SoundSource::Neutral, 0.5, 1.5));
            for key in ["minecraft:no.such.sound", "example:no.such.sound"] {
                assert!(game.player_play_sound(pb, key, SoundSource::Neutral, -1.0, -2.0));
            }
            assert!(!game.player_play_sound(pb, "invalid key", SoundSource::Neutral, 1.0, 1.0));
            assert!(packets(&rx_a).is_empty());
            let got = packets(&rx_b);
            assert_eq!(got.len(), 3);
            let Clientbound::Sound(p) = &got[0] else {
                panic!("expected sound")
            };
            assert_eq!((p.x, p.y, p.z), (24, 512, 16));
            assert_eq!((p.volume, p.pitch), (0.5, 1.5));
            assert_eq!(p.source, SoundSource::Neutral.index() as i32);
            assert!(p.sound.0 >= 0 && p.sound.1.is_none());
            for (packet, key) in got[1..]
                .iter()
                .zip(["minecraft:no.such.sound", "example:no.such.sound"])
            {
                let Clientbound::Sound(p) = packet else {
                    panic!("expected inline sound")
                };
                assert_eq!(p.sound, (-1, Some((key.to_owned(), None))));
                assert_eq!((p.volume, p.pitch), (-1.0, -2.0));
            }

            // A stale handle and an offline player fail.
            let stale = mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(
                    WorldEpoch::new(pa.entity().epoch().get() + 1),
                    pa.entity().id(),
                ),
                pa.fields().uuid,
            );
            assert!(!game.player_stop_sound(stale, None, None));
            assert!(!game.player_play_sound(stale, sound, SoundSource::Players, 1.0, 1.0));
            assert!(game.player(99).is_none());

            // play_bound_sound: players in range get a SoundEntity packet.
            fixture_resident_column(&mut game.world, (0, 0));
            let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            game.mobs
                .push(Mob::new(40, kind, Vec3::new(2.0, 64.0, 2.0), 0.0));
            let cow = game.entity(40).unwrap();
            assert!(game.entity_play_bound_sound(cow, sound, SoundSource::Neutral, 1.0, 1.0));
            for key in ["minecraft:no.such.sound", "example:no.such.sound"] {
                assert!(game.entity_play_bound_sound(cow, key, SoundSource::Neutral, -1.0, -2.0));
            }
            game.flush_native_sounds();
            for rx in [&rx_a, &rx_b] {
                let got = packets(rx);
                assert_eq!(got.len(), 3);
                for (packet, key) in got[1..]
                    .iter()
                    .zip(["minecraft:no.such.sound", "example:no.such.sound"])
                {
                    let Clientbound::SoundEntity(p) = packet else {
                        panic!("expected inline entity sound")
                    };
                    assert_eq!(p.sound, (-1, Some((key.to_owned(), None))));
                    assert_eq!((p.volume, p.pitch), (-1.0, -2.0));
                }
                let Clientbound::SoundEntity(p) = &got[0] else {
                    panic!("expected sound entity")
                };
                assert_eq!(p.id, 40);
                assert_eq!(p.source, SoundSource::Neutral.index() as i32);
            }
            assert!(packets(&rx_far).is_empty());
        });
    }
    /// `Level.playSound` draws its seed at the call, from the level's own
    /// sound-seed generator (`Level.soundSeedGenerator`), not the gameplay
    /// random; and a player exactly at the range edge is outside
    /// `PlayerList.broadcast`.
    #[test]
    #[ignore = "daily"]
    fn queued_sounds_draw_sound_seeds_at_play_and_cut_at_the_range_edge() {
        use sunrise_core::random::RandomSource;

        with_game(|game| {
            let net = NetConfig {
                listen: false,
                announce: false,
                lod_distance: 0,
                ..NetConfig::lan(0, String::new(), 3)
            };
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: game.dim.id().to_owned(),
                allow_flight: true,
                seed: 0,
                max_players: 3,
                integrated: true,
                levels: game.server.level_shapes.clone(),
                level_seeds: game.server.levels.0.runtime.seeds(),
            };
            let settings = LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            };
            game.lan_host(settings, net, config).unwrap();
            let (near, rx_near) = guest(31, 31, [0.5, 64.0, 1.0]);
            let (edge, rx_edge) = guest(32, 32, [0.5, 64.0, 16.0]);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .extend([near, edge]);

            let level = game.level("minecraft:overworld").unwrap();
            let sound = "minecraft:entity.player.burp";
            // The sound source's draws bookend the queue's: each queued sound
            // takes one of its own, distinct from every neighbour's.
            let before = game.server.lan.as_ref().unwrap().hub.next_sound_seed();
            let mut level_random = game.world.random.clone();
            for _ in 0..2 {
                assert!(game.native_queue_sound(
                    sunrise_gameplay::game::server_side::native_level_effects::NativeSoundRequest {
                        level,
                        sound,
                        at: [0.5, 64.0, 0.0],
                        source: SoundSource::Neutral,
                        volume: 1.0,
                        pitch: 1.0,
                        except: None,
                        entity: None,
                        inline_range: None,
                        seed: None,
                    },
                ));
            }
            // The gameplay random is exactly where it was: a mod's sound
            // moves no level draw.
            assert_eq!(
                game.world.random.next_long(),
                level_random.next_long(),
                "a queued sound draws no level-random long"
            );
            game.flush_native_sounds();

            let got = packets(&rx_near);
            assert_eq!(got.len(), 2, "both queued sounds reach the near player");
            let seeds: Vec<i64> = got
                .iter()
                .map(|packet| {
                    let Clientbound::Sound(p) = packet else {
                        panic!("expected sound")
                    };
                    p.seed
                })
                .collect();
            let after = game.server.lan.as_ref().unwrap().hub.next_sound_seed();
            assert_ne!(seeds[0], seeds[1], "each play draws its own seed");
            assert!(
                seeds.iter().all(|seed| *seed != before && *seed != after),
                "the queued sounds take the sound source's next draws"
            );
            assert!(
                packets(&rx_edge).is_empty(),
                "a player exactly at the range edge hears nothing"
            );

            // Commit preserves direct holders even if the key is registered or has no
            // registry ID. The fixed range, not the variable range, excludes the far player.
            use sunrise_core::position::BlockPos;
            use sunrise_items::items::ConsumeSound;
            let ids = [sound, "minecraft:test.inline"];
            game.storage_transaction(|game, _| {
                for id in ids {
                    assert!(
                        game.queue_consume_sound(
                            &ConsumeSound::Direct {
                                id: id.into(),
                                range: Some(0.75),
                            },
                            BlockPos::new(0, 64, 0),
                            SoundSource::Neutral,
                        )
                        .is_some()
                    );
                }
                game.flush_native_sounds();
                assert!(packets(&rx_near).is_empty());
                Ok::<_, ()>(())
            })
            .unwrap();
            let got = packets(&rx_near);
            assert_eq!(got.len(), ids.len());
            for (packet, id) in got.iter().zip(ids) {
                let Clientbound::Sound(packet) = packet else {
                    panic!("the near player must receive the direct sound")
                };
                assert_eq!(packet.sound, (-1, Some((id.into(), Some(0.75)))));
                assert_eq!((packet.x, packet.y, packet.z), (4, 516, 4));
            }
            assert!(packets(&rx_edge).is_empty());
        });
    }
}
