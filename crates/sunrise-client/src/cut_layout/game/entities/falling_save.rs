#![cfg(test)]

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    };
    use sunrise_core::position::{BlockPos, ChunkPos};
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_ticks::NativeTicksGameExt1;
    use sunrise_sim::join_seq::JoinSeq;
    use {
        sunrise_gameplay::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood,
        sunrise_gameplay::game::native_entity_refs::tests::with_game,
    };

    #[derive(Default)]
    struct Observed {
        landed_id: AtomicU32,
        detonated: AtomicBool,
    }

    struct Observer(Arc<Observed>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Observer {
        fn entity_tick_pre(
            &self,
            _: mod_api::HostSide<'_>,
            _: &mut mod_api::EntityTickPreEvent<'_>,
        ) -> mod_api::Flow {
            mod_api::Flow::Continue
        }
        fn entity_tick_post(
            &self,
            _: mod_api::HostSide<'_>,
            _: &mut mod_api::EntityTickPostEvent<'_>,
        ) {
        }
        fn explosion_detonate(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::ExplosionDetonateEvent<'_>,
        ) {
            let id = self.0.landed_id.load(Ordering::Relaxed);
            assert_ne!(id, 0);
            assert!(
                host.entity(id).is_none(),
                "the earlier falling entity has landed"
            );
            assert!(event.entities.iter().all(|entity| entity.id() != id));
            self.0.detonated.store(true, Ordering::Relaxed);
        }
    }

    #[test]
    #[ignore = "daily"]
    fn terminal_falling_blocks_balance_residency() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::server_side::falling_save::tests::terminal_falling_blocks_balance_residency",
            || {
                for listening in [false, true] {
                    let observed = Arc::new(Observed::default());
                    let mods: mod_api::Mods = if listening {
                        vec![mod_api::ModEntry::new(
                            "observer",
                            Box::new(Observer(observed.clone())),
                        )]
                    } else {
                        Vec::new()
                    }
                    .into();
                    mod_platform::testing::with_mods(mods, || {
                        with_game(|game| {
                            game.options.dedicated = Some(Vec::new());
                            game.build_registries();
                            game.set_allow_commands(true);
                            prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
                            assert!(game.run_command("forceload add 0 0"));
                            let sand = game
                                .pack
                                .assets
                                .registry
                                .default_state("minecraft:sand")
                                .unwrap();
                            let level = game.level(game.dim.id()).unwrap();
                            for x in 0..6 {
                                assert!(
                                    game.level_spawn_falling_block(
                                        level,
                                        sand,
                                        [2.0 * f64::from(x) + 0.5, 70.0, 0.5],
                                        [0.0; 3],
                                        mod_api::Falling::default(),
                                    )
                                    .unwrap()
                                    .is_some()
                                );
                            }
                            game.reset_entity_residency();
                            // An observer activates the same dispatch without writing entity state.
                            let round = game.native_tick_round();
                            assert_eq!(round.listening(), listening);
                            let joins: Vec<_> =
                                game.falling.iter().map(|block| block.join).collect();
                            // Continuing falls keep every resident's identity and order;
                            // only the selected owner's state advances.
                            let untouched = game.falling[1].position;
                            for _ in 0..3 {
                                game.tick_one_falling(0, &[]).unwrap();
                                assert_eq!(game.falling.len(), 6);
                                assert_eq!(game.falling[1].position, untouched);
                                assert_eq!(
                                    game.falling
                                        .iter()
                                        .map(|block| block.join)
                                        .collect::<Vec<_>>(),
                                    joins
                                );
                                assert_eq!(
                                    game.resident_entities.table.get_mut().end_tick(),
                                    Ok(())
                                );
                            }
                            let id = game.falling[0].id;
                            assert!(game.entity(id).is_some());
                            observed.landed_id.store(id, Ordering::Relaxed);
                            let stone = game
                                .pack
                                .assets
                                .registry
                                .default_state("minecraft:stone")
                                .unwrap();
                            game.world.set(0, 69, 0, stone).unwrap();
                            game.falling.update(0, |block| {
                                block.position.y = 70.0;
                                block.velocity.y = -0.5;
                            });
                            game.add_primed_tnt(BlockPos::new(8, 70, 0), 0.0, 1, None, None);
                            game.server.ordered.announced = JoinSeq::MAX;
                            game.server.tickets.0.update(None);
                            game.begin_ordered_ticks();
                            game.run_ordered_ticks(JoinSeq::MAX, false).unwrap();
                            assert!(game.tnts.is_empty());
                            assert!(!game.falling.iter().any(|block| block.id == id));
                            assert_eq!(game.falling.len(), 5);
                            assert_eq!(game.world.block(0, 70, 0), sand);
                            assert_eq!(observed.detonated.load(Ordering::Relaxed), listening);
                            // Expired blocks permanently leave the resident list.
                            for _ in 0..game.falling.len() {
                                game.falling.update(0, |block| {
                                    block.state = sunrise_world::registry::BlockStateId(0)
                                });
                                game.tick_one_falling(0, &[]).unwrap();
                            }
                            assert!(game.falling.is_empty());
                            assert_eq!(game.resident_entities.table.get_mut().end_tick(), Ok(()));
                        });
                    });
                }
            },
        );
    }
}
