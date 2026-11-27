#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_client_gameplay::game::client_side::client_attack_targets::tests::finish_login;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;

    use sunrise_client_gameplay::game::client_side::client_attack_targets::tests::sent_attacks;

    #[derive(Default)]
    struct AttackBatch {
        targets: std::sync::Mutex<Vec<mod_api::EntityRef>>,
        cancel: std::sync::atomic::AtomicBool,
    }
    struct Observer(std::sync::Arc<AttackBatch>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Observer {
        fn client_attack_target(
            &self,
            _: &mut dyn mod_api::ClientHost,
            event: &mut mod_api::ClientAttackTargetEvent<'_>,
        ) -> mod_api::Flow {
            *event.targets = self.0.targets.lock().unwrap().clone();
            if self.0.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                mod_api::Flow::Cancel
            } else {
                mod_api::Flow::Continue
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn client_attack_batch_keeps_order_deduplication_and_server_constraints() {
        use sunrise_core::geometry::Vec3;
        use sunrise_level::chunk::Chunk;
        use sunrise_protocol::packets::play::{self, Clientbound as C};
        use sunrise_sim::motion::PlayerMotion;
        use {
            crate::game::server_side::native_entity_refs::tests::with_client_game,
            sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client,
        };
        let state = std::sync::Arc::new(AttackBatch::default());
        let mods = vec![mod_api::ModEntry::new(
            "attack_batch_test",
            Box::new(Observer(state.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_client_game(|game| {
                let mut server =
                    sunrise_protocol::connection::Connection::local(logged_client(game));
                server
                    .reader
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                finish_login(game, &mut server);
                game.native_lifecycle.begin_open(None);
                game.client.title.0 = None;
                game.player =
                    Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 1.0, 0.5)).unwrap());
                game.entity_reach = 3.0;
                game.world
                    .chunks
                    .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
                let kind = game
                    .client
                    .remote
                    .as_ref()
                    .unwrap()
                    .builtin
                    .get("minecraft:entity_type")
                    .unwrap()
                    .id("minecraft:pig")
                    .unwrap();
                for (id, position) in [(31, [2.5, 2.0, 0.5]), (32, [0.5, 2.0, 2.5])] {
                    game.handle_packet(C::AddEntity(play::AddEntity {
                        id,
                        uuid: id as u128,
                        kind,
                        position,
                        velocity: [0.0; 3],
                        pitch: 0,
                        yaw: 0,
                        head_yaw: 0,
                        data: 0,
                    }))
                    .unwrap();
                }
                let first = game.entity(31).unwrap();
                let second = game.entity(32).unwrap();
                *state.targets.lock().unwrap() = vec![first, second, first];
                assert!(game.mod_attack_targets());
                assert_eq!(sent_attacks(game, &mut server), [31, 32]);
                *state.targets.lock().unwrap() = (0..65).flat_map(|_| [second, first]).collect();
                assert!(game.mod_attack_targets());
                assert_eq!(sent_attacks(game, &mut server), [32, 31]);
                state
                    .cancel
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                assert!(game.mod_attack_targets());
                assert!(sent_attacks(game, &mut server).is_empty());
                state
                    .cancel
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                game.entity_reach = 1.0;
                assert!(game.mod_attack_targets());
                assert!(sent_attacks(game, &mut server).is_empty());
                game.entity_reach = 3.0;
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(1, 2, 0, stone).unwrap();
                *state.targets.lock().unwrap() = vec![first];
                assert!(game.mod_attack_targets());
                assert!(
                    sent_attacks(game, &mut server).is_empty(),
                    "explicit extra batch targets still obey occlusion"
                );
                game.native_lifecycle.begin_open(None);
                assert!(game.mod_attack_targets());
                assert!(sent_attacks(game, &mut server).is_empty());
            })
        });
    }
}
