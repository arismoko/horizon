#![cfg(test)]
#[cfg(feature = "client")]
#[cfg(test)]
pub(crate) mod tests {
    use mod_api::{
        AnimateTickEvent, BlockPart, BlockPos, BreakProgressPositionsEvent, ClientHost, Flow,
        Hooks, InteractionKey, InteractionKeyMappingTriggeredEvent, ModEntry, Registrar,
        RegistrationContext,
    };
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_level::chunk::Chunk;

    const PROGRESS_POS: [i32; 3] = [4, 64, 4];
    use {
        crate::game::server_side::native_entity_refs::tests::with_client_game,
        sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client,
        sunrise_gameplay_common::game::Mining,
    };

    struct ExtraPositions {
        calls: Arc<Mutex<Vec<u8>>>,
        invalid: Arc<AtomicBool>,
        reenter: bool,
        refresh_on_animate: Arc<AtomicBool>,
    }
    #[mod_api::sunrise_part]
    impl BlockPart for ExtraPositions {
        fn animate_tick(&self, host: &mut dyn ClientHost, event: &mut AnimateTickEvent<'_>) {
            if self.refresh_on_animate.swap(false, Ordering::Relaxed) {
                assert!(
                    event
                        .level
                        .set_client_break_progress(host, 10, PROGRESS_POS, 5)
                );
            }
        }

        fn break_progress_positions(
            &self,
            host: &mut dyn ClientHost,
            event: &mut BreakProgressPositionsEvent<'_>,
        ) {
            let count = {
                let mut calls = self.calls.lock().unwrap();
                calls.push(event.progress);
                calls.len()
            };
            let p = event.pos;
            // Bound the fixture's recursion so a regression fails the assertion
            // rather than overflowing the test process's stack.
            if self.reenter && count == 1 {
                assert!(event.level.set_client_break_progress(
                    host,
                    -1,
                    [p.x, p.y, p.z],
                    i32::from(event.progress),
                ));
            }
            event.positions.extend([
                BlockPos::new(p.x + 1, p.y, p.z),
                BlockPos::new(p.x, p.y, p.z + 1),
                BlockPos::new(p.x + 1, p.y, p.z + 1),
            ]);
            if self.invalid.load(Ordering::Relaxed) {
                event.positions.extend([
                    p,
                    BlockPos::new(p.x + 1, p.y, p.z),
                    BlockPos::new(p.x + 8, p.y, p.z),
                    BlockPos::new(p.x + 9, p.y, p.z),
                    BlockPos::new(p.x + 20, p.y, p.z),
                    BlockPos::new(p.x + 40, p.y, p.z),
                    BlockPos::new(p.x + 8, p.y + 8, p.z),
                    BlockPos::new(i32::MIN, i32::MAX, i32::MIN),
                ]);
                for x in -7..=7 {
                    for z in -7..=7 {
                        if x * x + z * z <= 64 {
                            event.positions.push(BlockPos::new(p.x + x, p.y, p.z + z));
                        }
                    }
                }
            }
        }
    }
    struct Register {
        calls: Arc<Mutex<Vec<u8>>>,
        invalid: Arc<AtomicBool>,
        reenter: bool,
        refresh_on_animate: Arc<AtomicBool>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Register {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let key = r.block_key("minecraft:stone");
            r.block_part(
                key,
                ExtraPositions {
                    calls: self.calls.clone(),
                    invalid: self.invalid.clone(),
                    reenter: self.reenter,
                    refresh_on_animate: self.refresh_on_animate.clone(),
                },
            );
        }

        fn interaction_key_mapping_triggered(
            &self,
            host: &mut dyn ClientHost,
            event: &mut InteractionKeyMappingTriggeredEvent,
        ) -> Flow {
            if event.key != InteractionKey::Attack {
                return Flow::Continue;
            }
            let level = host.level("minecraft:overworld").unwrap();
            assert!(level.set_client_break_progress(host, 10, PROGRESS_POS, 5));
            event.swing_hand = false;
            Flow::Cancel
        }
    }

    #[test]
    #[ignore = "daily"]
    fn crack_positions_are_scoped_stored_bounded_and_cleared_for_both_sources() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let invalid = Arc::new(AtomicBool::new(false));
        let refresh_on_animate = Arc::new(AtomicBool::new(false));
        let mods = vec![ModEntry::new(
            "cracks",
            Box::new(Register {
                calls: calls.clone(),
                invalid: invalid.clone(),
                reenter: false,
                refresh_on_animate: refresh_on_animate.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_client_game(|game| {
                let _server = logged_client(game);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                let dirt = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:dirt")
                    .unwrap();
                let pos = PROGRESS_POS;
                let four = [[4, 64, 4], [4, 64, 5], [5, 64, 4], [5, 64, 5]];
                for p in four {
                    game.world.set(p[0], p[1], p[2], stone).unwrap();
                }
                game.client.settings.improved_transparency = false;
                let camera = glam::DVec3::new(4.5, 65.0, 4.5);
                game.destroy_block_progress(10, pos, 2);
                let rendered = game.cracks(camera);
                assert_eq!(rendered.len(), 4);
                let sprite = &game.client.gui.atlas.sprites["block/destroy_stage_2"];
                assert!(rendered.iter().all(|(_, uv)| *uv == [sprite.u0, sprite.v0]));
                let mut expected = four.map(|p| (p, 2)).to_vec();
                expected.sort_unstable();
                let mut drawn = game.crack_stages();
                drawn.sort_unstable();
                assert_eq!(drawn, expected);
                for _ in 0..3 {
                    assert_eq!(game.crack_stages().len(), 4);
                }
                game.destroy_block_progress(10, pos, 2);
                assert_eq!(
                    *calls.lock().unwrap(),
                    [2],
                    "frames and unchanged stages never redispatch"
                );
                game.destroy_block_progress(10, pos, 7);
                assert!(game.crack_stages().iter().all(|(_, stage)| *stage == 7));
                let rendered = game.cracks(camera);
                assert_eq!(rendered.len(), 4);
                let sprite = &game.client.gui.atlas.sprites["block/destroy_stage_7"];
                assert!(rendered.iter().all(|(_, uv)| *uv == [sprite.u0, sprite.v0]));
                assert_eq!(*calls.lock().unwrap(), [2, 7]);
                for clear in [-1, 10] {
                    game.destroy_block_progress(10, pos, clear);
                    assert!(game.crack_stages().is_empty());
                }
                let mining = |progress| {
                    Some(Mining {
                        pos,
                        progress,
                        per_tick: 0.0,
                        ticks: 1,
                    })
                };
                game.set_client_mining(mining(0.35));
                assert_eq!(
                    game.crack_stages().len(),
                    4,
                    "local prediction includes the extras"
                );
                assert_eq!(game.cracks(camera).len(), 4);
                assert!(game.crack_stages().iter().all(|(_, stage)| *stage == 3));
                game.set_client_mining(mining(0.39));
                assert_eq!(*calls.lock().unwrap(), [2, 7, 3]);
                game.set_client_mining(mining(0.45));
                assert!(game.crack_stages().iter().all(|(_, stage)| *stage == 4));
                game.set_client_mining(None);
                assert!(game.client.mining_crack.is_none());
                assert!(game.crack_stages().is_empty());
                game.world.set(9, 64, 4, dirt).unwrap();
                game.destroy_block_progress(11, [9, 64, 4], 1);
                assert_eq!(game.crack_stages(), [([9, 64, 4], 1)]);
                assert_eq!(
                    *calls.lock().unwrap(),
                    [2, 7, 3, 4],
                    "an unsubscribed state is not dispatched"
                );
                game.destroy_block_progress(11, [9, 64, 4], -1);
                for chunk_x in 1..=2 {
                    game.world
                        .chunks
                        .insert((chunk_x, 0), Arc::new(Chunk::empty(chunk_x, 0)));
                }
                for far in [[24, 64, 4], [44, 64, 4]] {
                    game.world.set(far[0], far[1], far[2], stone).unwrap();
                }
                invalid.store(true, Ordering::Relaxed);
                game.world.game_time = 0;
                game.destroy_block_progress(10, pos, 5);
                let drawn = game.crack_stages();
                assert_eq!(drawn.len(), 65, "base plus at most 64 display extras");
                assert_eq!(
                    drawn
                        .iter()
                        .map(|c| c.0)
                        .collect::<std::collections::HashSet<_>>()
                        .len(),
                    65
                );
                assert!(
                    drawn.contains(&([12, 64, 4], 5)),
                    "eight-block boundary is included"
                );
                for far in [
                    [13, 64, 4],
                    [24, 64, 4],
                    [44, 64, 4],
                    [12, 72, 4],
                    [i32::MIN, i32::MAX, i32::MIN],
                ] {
                    assert!(
                        drawn.contains(&(far, 5)),
                        "an extra may be anywhere in the level: {far:?}"
                    );
                }
                // A mod block entity's `crumbling_stage` reads this: the origin and each extra
                // have the stage, any other position has none.
                let stages = game.crack_stage_map();
                assert_eq!(stages.get(&pos), Some(&5));
                assert_eq!(stages.get(&[24, 64, 4]), Some(&5));
                assert_eq!(stages.get(&[100, 64, 4]), None);
                // Drawing still keeps vanilla's 32-block camera distance: 20 blocks away draws,
                // 40 does not, and the far extra costs nothing.
                let rendered = game.cracks(camera);
                assert_eq!(rendered.len(), 5);
                assert!(rendered.iter().any(|(p, _)| *p == [24, 64, 4]));
                assert!(!rendered.iter().any(|(p, _)| *p == [44, 64, 4]));
                game.world.game_time += 100;
                let calls_before = calls.lock().unwrap().len();
                game.destroy_block_progress(10, pos, 5);
                game.destroy_block_progress(11, [9, 64, 4], 1);
                assert_eq!(calls.lock().unwrap().len(), calls_before);

                game.world.game_time = 499;
                game.tick_remote_level().unwrap();
                assert_eq!(game.world.game_time, 500);
                assert_eq!(game.client.destroying.len(), 2, "400 ticks is still live");
                game.tick_remote_level().unwrap();
                assert_eq!(game.world.game_time, 501);
                assert_eq!(game.crack_stage_map().get(&pos), Some(&5));
                assert!(game.cracks(camera).iter().any(|(p, _)| *p == pos));

                game.client.title.0 = None;
                game.client.screen = sunrise_gameplay_common::game::Screen::Playing;
                game.client.remote.as_mut().unwrap().loaded_sent = true;
                game.player = Some(
                    sunrise_sim::motion::PlayerMotion::new(
                        game.motion,
                        sunrise_core::geometry::Vec3::new(4.5, 65.0, 4.5),
                    )
                    .unwrap(),
                );
                game.set_local_mode(sunrise_command::args::GameMode::Survival);
                game.world.game_time = 519;
                game.vanilla_key_mapping("key.attack")
                    .unwrap()
                    .click(game)
                    .unwrap();
                game.tick_remote_level().unwrap();
                assert_eq!(game.world.game_time, 520);
                assert!(!game.client.destroying.contains_key(&11));
                assert_eq!(game.client.destroying[&10].updated_at, 519);
                assert_eq!(game.crack_stage_map().get(&pos), Some(&5));
                assert_eq!(calls.lock().unwrap().len(), calls_before);

                game.world.game_time = 919;
                game.tick_remote_level().unwrap();
                assert!(
                    game.client.destroying.is_empty(),
                    "expiry drops the stored extras too"
                );
                game.client.remote.as_mut().unwrap().loaded_sent = false;
                game.player.as_mut().unwrap().position =
                    sunrise_core::geometry::Vec3::new(4.5, 65.0, 4.5);
                for x in 0..=8 {
                    for z in 0..=8 {
                        for y in 64..=70 {
                            game.world.set(x, y, z, stone).unwrap();
                        }
                    }
                }
                game.client.particles.0.seed_randoms(0, 0, 0);
                game.world.game_time = 1000;
                game.destroy_block_progress(10, pos, 5);
                let calls_before = calls.lock().unwrap().len();
                refresh_on_animate.store(true, Ordering::Relaxed);
                game.world.game_time = 1419;
                game.tick_remote_level().unwrap();
                assert!(!refresh_on_animate.load(Ordering::Relaxed));
                assert_eq!(game.client.destroying[&10].updated_at, 1420);
                assert_eq!(calls.lock().unwrap().len(), calls_before + 1);
                game.destroy_block_progress(10, pos, 0);
                game.set_client_mining(mining(0.25));
                game.reset_client_world(0);
                assert!(game.client.destroying.is_empty());
                assert!(game.mining.is_none());
                assert!(game.client.mining_crack.is_none());
                assert!(game.crack_stages().is_empty());
            })
        });
    }

    #[test]
    #[ignore = "daily"]
    fn reentrant_same_stage_client_crack_update_dispatches_once() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new(
            "reentrant-cracks",
            Box::new(Register {
                calls: calls.clone(),
                invalid: Arc::new(AtomicBool::new(false)),
                reenter: true,
                refresh_on_animate: Arc::new(AtomicBool::new(false)),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_client_game(|game| {
                let _server = logged_client(game);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                let pos = [4, 64, 4];
                game.world.set(pos[0], pos[1], pos[2], stone).unwrap();
                game.destroy_block_progress(-1, pos, 3);
                assert_eq!(*calls.lock().unwrap(), [3]);
                let crack = &game.client.destroying[&-1];
                assert_eq!((crack.pos, crack.stage), (pos, 3));
                assert_eq!(crack.positions, [[5, 64, 4], [4, 64, 5], [5, 64, 5]]);
            })
        });
    }

    #[test]
    #[ignore = "daily"]
    fn client_cracks_clear_on_world_reset_and_age_only_in_a_level_tick() {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            game.destroy_block_progress(-10, [1, 64, 1], 4);
            game.destroy_block_progress(-11, [2, 64, 1], 9);
            assert_eq!(game.client.destroying.len(), 2);
            game.destroy_block_progress(-10, [1, 64, 1], -1);
            assert!(!game.client.destroying.contains_key(&-10));
            assert!(game.client.destroying.contains_key(&-11));
            game.world.game_time += 420;
            game.tick().unwrap();
            assert_eq!(
                game.ticks, 0,
                "a client without a remote must not tick a server level"
            );
            assert!(game.client.destroying.contains_key(&-11));
            game.reset_client_world(0);
            assert!(game.client.destroying.is_empty());
        });
    }
}
