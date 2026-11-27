#![cfg(test)]

#[cfg(test)]
mod tests {
    use super::super::{
        native_entity_refs::tests::run_fresh_cases, native_guest_turn_tests::with_guests,
    };
    use mod_api::LevelRef;
    use mod_api::{__private::particle_named_key, ParticleOptions, WorldEpoch};
    use std::sync::{Arc, mpsc};
    use sunrise_gameplay_common::game::Dim;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::transactions::TransactionsGameExt;
    use sunrise_protocol::{
        buf::Reader,
        entity_data::Particle,
        packets::play::{Clientbound as Packet, LevelEvent, LevelParticles},
    };
    use sunrise_sim::lan::{Out, PlayerSender};

    fn packets(rx: &mpsc::Receiver<Out>) -> Vec<Packet> {
        rx.try_iter()
            .flat_map(|out| match out {
                Out::Packet(body) => vec![body],
                Out::Packets(bodies) => bodies,
                Out::Disconnect(reason) => panic!("unexpected disconnect: {reason:?}"),
            })
            .map(|body| {
                let mut r = Reader::new(&body);
                Packet::decode(r.varint().unwrap(), r.rest()).unwrap()
            })
            .collect()
    }

    #[test]
    #[ignore = "daily"]
    fn particle_batch_is_atomic_and_follows_level_delivery_order() {
        const CASE: &str = "SUNRISE_TEST_PARTICLE_BATCH_2015";
        const A: [f64; 3] = [0.5, 64.5, 0.5];
        const B: [f64; 3] = [63.5, 64.5, 0.5];
        const C: [f64; 3] = [1.5, 64.5, 0.5];
        const E: [f64; 3] = [32.5, 64.5, 0.5];
        const SPREAD: [f64; 3] = [0.25, 0.5, 0.75];
        const SPEED: f64 = 0.125;
        const COUNT: i32 = 3;
        if run_fresh_cases(
            "cut_layout::game::server_side::native_level_effects::tests::particle_batch_is_atomic_and_follows_level_delivery_order",
            CASE,
            &["server"],
        ) {
            return;
        }
        let heart = || ParticleOptions::simple(particle_named_key("minecraft:heart").unwrap());
        let send_at = |game: &mut sunrise_gameplay::game::ServerGame<'_>,
                       level: LevelRef,
                       options: &ParticleOptions,
                       positions: &[[f64; 3]]| {
            level.send_particles_batch(game, options, None, positions, SPREAD, SPEED, COUNT)
        };
        with_guests(|game| {
            let (tx_near, near) = mpsc::channel();
            let (tx_far, far) = mpsc::channel();
            let hub = &mut game.server.lan.as_mut().unwrap().hub;
            hub.update_guest(12, |guest| {
                guest.sender = PlayerSender::new(tx_near, Arc::default());
                guest.position = [0.9, 64.1, 0.9];
            })
            .unwrap();
            hub.update_guest(13, |guest| {
                guest.sender = PlayerSender::new(tx_far, Arc::default());
                guest.position = [63.9, 64.1, 0.9];
            })
            .unwrap();
            let heart_id = hub
                .registry_id("minecraft:particle_type", "minecraft:heart")
                .unwrap();
            let heart = heart();
            let level = game.level("minecraft:overworld").unwrap();
            let send = |game: &mut sunrise_gameplay::game::ServerGame<'_>,
                        positions: &[[f64; 3]]| {
                send_at(game, level, &heart, positions)
            };
            let particle = |pos| {
                Packet::LevelParticles(LevelParticles {
                    particle: Particle {
                        id: heart_id,
                        data: None,
                    },
                    override_limiter: false,
                    always_show: false,
                    pos,
                    dist: [0.25, 0.5, 0.75],
                    max_speed: [0.125; 3],
                    count: 3,
                    randomization: 0,
                })
            };
            // E is less than 32 blocks from the first guest's position, but exactly 32
            // from the centre of that guest's block. Only the second guest receives E.
            assert!(send(game, &[B, A, E, C, A]));
            game.flush_lan_particles();
            assert_eq!(packets(&near), [particle(A), particle(C), particle(A)]);
            assert_eq!(packets(&far), [particle(B), particle(E)]);

            let mut invalid_options = heart.clone();
            invalid_options.scale = f32::NAN;
            assert!(send_at(game, level, &invalid_options, &[]));
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            assert!(!send_at(game, stale, &heart, &[]));
            assert!(!send(game, &[A, [f64::NAN, 64.5, 0.5]]));
            let block = ParticleOptions::simple(particle_named_key("minecraft:block").unwrap());
            assert!(!send_at(game, level, &block, &[A]));
            game.flush_lan_particles();
            assert!(packets(&near).is_empty());
            assert!(packets(&far).is_empty());

            let marker_pos = [-8, 64, 0];
            let marker = |kind| {
                Packet::LevelEvent(LevelEvent {
                    kind,
                    pos: marker_pos,
                    data: 0,
                    global: false,
                })
            };
            game.storage_transaction(|game, _| {
                assert!(level.level_event(game, 1000, marker_pos, 0));
                game.storage_transaction(|game, _| {
                    assert!(send(game, &[A, C, A]));
                    Ok::<(), ()>(())
                })
                .unwrap();
                assert_eq!(
                    game.storage_transaction(|game, _| {
                        assert!(send(game, &[A, B]));
                        Err::<(), _>("nested abort")
                    }),
                    Err("nested abort")
                );
                assert!(level.level_event(game, 1001, marker_pos, 0));
                assert!(packets(&near).is_empty());
                assert!(packets(&far).is_empty());
                Ok::<(), ()>(())
            })
            .unwrap();
            assert_eq!(
                packets(&near),
                [
                    marker(1000),
                    particle(A),
                    particle(C),
                    particle(A),
                    marker(1001)
                ]
            );
            assert!(packets(&far).is_empty());

            assert_eq!(
                game.storage_transaction(|game, _| {
                    game.storage_transaction(|game, _| {
                        assert!(send(game, &[A, B]));
                        Ok::<(), ()>(())
                    })
                    .unwrap();
                    Err::<(), _>("outer abort")
                }),
                Err("outer abort")
            );
            game.flush_lan_particles();
            assert!(packets(&near).is_empty());
            assert!(packets(&far).is_empty());

            // Both guests are now in range, but only one is in the requested dimension.
            game.enter_level(Dim::Nether).unwrap();
            game.enter_level(Dim::Overworld).unwrap();
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |guest| {
                    guest.position = [0.9, 64.1, 0.9];
                    guest.dimension = "minecraft:the_nether".to_owned();
                })
                .unwrap();
            let nether = game.level("minecraft:the_nether").unwrap();
            packets(&near);
            packets(&far);
            game.storage_transaction(|game, _| {
                assert!(send_at(game, nether, &heart, &[A, C]));
                assert!(packets(&near).is_empty());
                assert!(packets(&far).is_empty());
                Ok::<(), ()>(())
            })
            .unwrap();
            assert!(packets(&near).is_empty());
            assert_eq!(packets(&far), [particle(A), particle(C)]);
        });
    }
}
