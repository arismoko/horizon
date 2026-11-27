#![cfg(test)]
#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use mod_api::{
        CustomSpawner, HostTransaction, LevelRef, ObjectiveRenderType, ServerHost, SpawnerKind,
        Text,
    };
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use sunrise_gameplay::game::{
        ServerGame, server_side::bossbars::audience_tests::add_real_and_fake,
    };
    use sunrise_gameplay_common::game::Dim;
    use sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycleCommonGameExt1 as _;

    type Log = Arc<Mutex<Vec<String>>>;

    fn set_spawn_rule(game: &mut ServerGame<'_>, name: &str, value: bool) {
        let rule = game.ui.rules.rule(name).unwrap().clone();
        game.set_game_rule(
            &rule,
            sunrise_game_data::gamerules::Value::Bool(value),
            None,
        );
    }

    struct StatefulSpawner {
        name: &'static str,
        ticks: u32,
        log: Log,
    }
    impl CustomSpawner for StatefulSpawner {
        fn tick(&mut self, host: &mut dyn ServerHost, level: LevelRef, spawn_enemies: bool) {
            self.ticks += 1;
            self.log.lock().unwrap().push(format!(
                "{} tick {} dim={} enemies={}",
                self.name,
                self.ticks,
                level.dimension().id(host).unwrap(),
                spawn_enemies
            ));
        }
    }

    /// A mod that removes the cat spawner and appends its own stateful `CustomSpawner`.
    struct Spawns {
        name: &'static str,
        log: Log,
    }
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Spawns {
        fn modify_custom_spawners(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::ModifyCustomSpawnersEvent<'_>,
        ) {
            self.log
                .lock()
                .unwrap()
                .push(format!("{} modify", self.name));
            event.spawners.remove(SpawnerKind::Cat);
            event.spawners.add(StatefulSpawner {
                name: self.name,
                ticks: 0,
                log: self.log.clone(),
            });
        }
    }

    /// With no listener the list is vanilla's and every spawner ticks: the cat
    /// spawner counts its clock down. With two mods, `modify_custom_spawners`
    /// fires once per level, the cat spawner never ticks, and each mod's owned
    /// spawner ticks directly in list order with its own retained state and the
    /// chunk source's stored `spawn_enemies` flag, not broadcast to other mods.
    #[test]
    #[ignore = "daily"]
    fn custom_spawners_tick_in_list_order_and_a_removed_one_never_ticks() {
        with_game(|game| {
            game.server.cat.next_tick = 5;
            game.tick_spawners();
            assert_eq!(
                game.server.cat.next_tick, 4,
                "no listener: vanilla's cat spawner ticks"
            );
            let builtins: Vec<_> = game.creatures.custom_spawners[&game.dim]
                .builtins()
                .collect();
            assert_eq!(
                builtins,
                [
                    SpawnerKind::WanderingTrader,
                    SpawnerKind::Patrol,
                    SpawnerKind::Cat,
                    SpawnerKind::VillageSiege,
                    SpawnerKind::Phantom,
                ]
            );
        });
        let log = Log::default();
        let mods = ["amod", "bmod"]
            .map(|name| {
                mod_api::ModEntry::new(
                    name,
                    Box::new(Spawns {
                        name,
                        log: log.clone(),
                    }),
                )
            })
            .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.server.cat.next_tick = 5;
                game.set_difficulty_forced(0);
                game.tick_spawners();
                set_spawn_rule(game, "spawn_monsters", false);
                game.tick_spawners();
                set_spawn_rule(game, "spawn_monsters", true);
                game.tick_spawners();
                assert_eq!(
                    game.server.cat.next_tick, 5,
                    "the removed cat spawner never ticks"
                );
            });
        });
        let log = log.lock().unwrap();
        assert_eq!(
            log[..2],
            ["amod modify", "bmod modify"],
            "one modify event per level, in dependency order"
        );
        assert_eq!(log.iter().filter(|l| l.ends_with("modify")).count(), 2);
        let ticks: Vec<&str> = log[2..].iter().map(String::as_str).collect();
        assert_eq!(
            ticks,
            [
                "amod tick 1 dim=minecraft:overworld enemies=true",
                "bmod tick 1 dim=minecraft:overworld enemies=true",
                "amod tick 2 dim=minecraft:overworld enemies=false",
                "bmod tick 2 dim=minecraft:overworld enemies=false",
                "amod tick 3 dim=minecraft:overworld enemies=true",
                "bmod tick 3 dim=minecraft:overworld enemies=true",
            ]
        );
    }

    #[test]
    #[ignore = "daily"]
    fn custom_spawners_are_owned_per_level_empty_outside_overworld_and_dropped_on_unload() {
        let log = Log::default();
        let overworld_drops = Arc::new(AtomicUsize::new(0));
        let nether_drops = Arc::new(AtomicUsize::new(0));

        struct LevelOwnedSpawner {
            dim: String,
            ticks: u32,
            log: Log,
            drops: Arc<AtomicUsize>,
        }
        impl CustomSpawner for LevelOwnedSpawner {
            fn tick(&mut self, host: &mut dyn ServerHost, level: LevelRef, _: bool) {
                self.ticks += 1;
                self.log.lock().unwrap().push(format!(
                    "{}:{}({})",
                    self.dim,
                    self.ticks,
                    level.dimension().id(host).unwrap()
                ));
            }
        }
        impl Drop for LevelOwnedSpawner {
            fn drop(&mut self) {
                self.drops.fetch_add(1, Ordering::Relaxed);
            }
        }

        struct PerLevelMod {
            log: Log,
            overworld_drops: Arc<AtomicUsize>,
            nether_drops: Arc<AtomicUsize>,
        }
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for PerLevelMod {
            fn modify_custom_spawners(
                &self,
                host: &dyn mod_api::Host,
                event: &mut mod_api::ModifyCustomSpawnersEvent<'_>,
            ) {
                let dimension = event.level.dimension().id(host).unwrap();
                let builtin_count = event.spawners.builtins().count();
                self.log
                    .lock()
                    .unwrap()
                    .push(format!("modify:{dimension}={builtin_count}"));
                let drops = if dimension == Dim::Overworld.id() {
                    self.overworld_drops.clone()
                } else {
                    self.nether_drops.clone()
                };
                event.spawners.add(LevelOwnedSpawner {
                    dim: dimension.to_owned(),
                    ticks: 0,
                    log: self.log.clone(),
                    drops,
                });
            }
        }

        let mods = vec![mod_api::ModEntry::new(
            "per_level_spawner_mod",
            Box::new(PerLevelMod {
                log: log.clone(),
                overworld_drops: overworld_drops.clone(),
                nether_drops: nether_drops.clone(),
            }),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_game(|game| {
                game.native_level_opened(Dim::Overworld);
                game.native_level_opened(Dim::Nether);
                // Re-opening an already open level must not fire again.
                game.native_level_opened(Dim::Overworld);

                game.dim = Dim::Overworld;
                game.tick_custom_spawners();
                game.tick_custom_spawners();
                game.dim = Dim::Nether;
                game.tick_custom_spawners();

                // Closing the Nether drops its spawner immediately without touching Overworld.
                game.dim = Dim::Overworld;
                game.native_level_closed(Dim::Nether);
                assert_eq!(nether_drops.load(Ordering::Relaxed), 1);
                assert_eq!(overworld_drops.load(Ordering::Relaxed), 0);

                // Overworld's spawner keeps its state and continues ticking.
                game.tick_custom_spawners();

                // Closing the world drops Overworld's spawner too.
                game.native_lifecycle_world_closed();
                assert_eq!(overworld_drops.load(Ordering::Relaxed), 1);
            });
        });

        let log = log.lock().unwrap();
        assert_eq!(
            log.as_slice(),
            [
                "modify:minecraft:overworld=5",
                "modify:minecraft:the_nether=0",
                "minecraft:overworld:1(minecraft:overworld)",
                "minecraft:overworld:2(minecraft:overworld)",
                "minecraft:the_nether:1(minecraft:the_nether)",
                "minecraft:overworld:3(minecraft:overworld)",
            ]
        );
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn custom_spawners_do_not_run_on_client_side() {
        let calls = Arc::new(AtomicUsize::new(0));
        struct ClientProbe(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for ClientProbe {
            fn modify_custom_spawners(
                &self,
                _: &dyn mod_api::Host,
                _: &mut mod_api::ModifyCustomSpawnersEvent<'_>,
            ) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }

        let mods = vec![mod_api::ModEntry::new(
            "client_spawner_probe",
            Box::new(ClientProbe(calls.clone())),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            crate::game::integrated_pair::with_integrated_pair(|pair| {
                pair.call(|game| {
                    game.common.options.execution_role =
                        sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
                    game.init_custom_spawners(Dim::Overworld);
                    game.tick_spawners();
                    game.tick_custom_spawners();
                    assert!(game.common.creatures.custom_spawners.is_empty());
                });
            });
        });
        assert_eq!(calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    #[ignore = "daily"]
    fn custom_spawners_skip_debug_worlds_and_disabled_spawn_mobs_in_tick_spawners() {
        use sunrise_game_data::gamerules::Value as RuleValue;

        let ticks = Arc::new(AtomicUsize::new(0));
        struct CountingMod(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for CountingMod {
            fn modify_custom_spawners(
                &self,
                _: &dyn mod_api::Host,
                event: &mut mod_api::ModifyCustomSpawnersEvent<'_>,
            ) {
                let counter = self.0.clone();
                event
                    .spawners
                    .add(move |_: &mut dyn ServerHost, _: LevelRef, _: bool| {
                        counter.fetch_add(1, Ordering::Relaxed);
                    });
            }
        }

        let mods = vec![mod_api::ModEntry::new(
            "debug_gate_spawner_mod",
            Box::new(CountingMod(ticks.clone())),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_game(|game| {
                game.server.cat.next_tick = 5;
                // `ServerChunkCache.tickChunks` skips the entire chunk-spawning phase
                // (including `tickCustomSpawners`) in debug worlds, even with `spawn_mobs` on.
                game.server.world_debug = true;
                game.tick_spawners();
                assert_eq!(ticks.load(Ordering::Relaxed), 0);
                assert_eq!(game.server.cat.next_tick, 5);

                // Outside a debug world, `spawn_mobs = false` also skips custom spawners.
                game.server.world_debug = false;
                assert!(game.ui.rules.set("spawn_mobs", RuleValue::Bool(false)));
                game.tick_spawners();
                assert_eq!(ticks.load(Ordering::Relaxed), 0);
                assert_eq!(game.server.cat.next_tick, 5);

                // Eligible tick: both built-in and owned mod spawners run once.
                assert!(game.ui.rules.set("spawn_mobs", RuleValue::Bool(true)));
                game.tick_spawners();
                assert_eq!(ticks.load(Ordering::Relaxed), 1);
                assert_eq!(game.server.cat.next_tick, 4);
            });
        });
    }

    /// A later listener that removes an earlier mod's instance and then panics leaves that
    /// instance in the level: its edits are undone, and the instance keeps ticking.
    #[test]
    #[ignore = "daily"]
    fn an_earlier_instance_survives_a_later_listener_that_removes_it_and_fails() {
        struct Adds(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Adds {
            fn modify_custom_spawners(
                &self,
                _: &dyn mod_api::Host,
                event: &mut mod_api::ModifyCustomSpawnersEvent<'_>,
            ) {
                let ticks = self.0.clone();
                event
                    .spawners
                    .add(move |_: &mut dyn ServerHost, _: LevelRef, _: bool| {
                        ticks.fetch_add(1, Ordering::Relaxed);
                    });
            }
        }
        struct Fails;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Fails {
            fn modify_custom_spawners(
                &self,
                _: &dyn mod_api::Host,
                event: &mut mod_api::ModifyCustomSpawnersEvent<'_>,
            ) {
                let last = event.spawners.len() - 1;
                assert!(event.spawners.remove_at(last));
                assert!(
                    event
                        .spawners
                        .insert(0, |_: &mut dyn ServerHost, _: LevelRef, _: bool| {})
                );
                panic!("listener failed after editing the list");
            }
        }

        let ticks = Arc::new(AtomicUsize::new(0));
        let mods = vec![
            mod_api::ModEntry::new("adds_spawner", Box::new(Adds(ticks.clone()))),
            mod_api::ModEntry::new("fails_after_editing", Box::new(Fails)),
        ];
        mod_platform::testing::with_mods(mods.into(), || {
            with_game(|game| {
                game.init_custom_spawners(game.dim);
                let list = &game.creatures.custom_spawners[&game.dim];
                assert_eq!(
                    list.len(),
                    6,
                    "five built-ins and the earlier mod's instance"
                );
                game.tick_custom_spawners();
                assert_eq!(ticks.load(Ordering::Relaxed), 1);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn appended_custom_spawner_runs_after_phantom_spawner_in_same_pass() {
        struct PhantomOrderMod {
            log: Log,
            remove_phantom: bool,
        }
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for PhantomOrderMod {
            fn modify_custom_spawners(
                &self,
                _: &dyn mod_api::Host,
                event: &mut mod_api::ModifyCustomSpawnersEvent<'_>,
            ) {
                if self.remove_phantom {
                    event.spawners.remove(SpawnerKind::Phantom);
                }
                let log = self.log.clone();
                event.spawners.add(
                    move |host: &mut dyn ServerHost, _: LevelRef, spawn_enemies: bool| {
                        let saw_phantom = host.objective("phantom_pass_ran").is_some();
                        log.lock().unwrap().push(format!(
                            "appended_spawner enemies={spawn_enemies} saw_phantom={saw_phantom}"
                        ));
                    },
                );
            }

            fn player_spawn_phantoms(
                &self,
                host: &mut dyn ServerHost,
                _: &mut mod_api::PlayerSpawnPhantomsEvent<'_>,
            ) -> Option<mod_api::PhantomSpawnResult> {
                self.log.lock().unwrap().push("phantom_spawner".into());
                host.add_objective(
                    "phantom_pass_ran",
                    "dummy",
                    Text::literal("Phantom"),
                    ObjectiveRenderType::Integer,
                )
                .unwrap();
                Some(mod_api::PhantomSpawnResult::Deny)
            }
        }

        for (remove_phantom, expected) in [
            (
                false,
                vec![
                    "appended_spawner enemies=false saw_phantom=false".to_owned(),
                    "phantom_spawner".to_owned(),
                    "appended_spawner enemies=true saw_phantom=true".to_owned(),
                ],
            ),
            (
                true,
                vec!["appended_spawner enemies=true saw_phantom=false".to_owned()],
            ),
        ] {
            let log = Log::default();
            let mods = vec![mod_api::ModEntry::new(
                "phantom_order_mod",
                Box::new(PhantomOrderMod {
                    log: log.clone(),
                    remove_phantom,
                }),
            )];
            mod_platform::testing::with_mods(mods.into(), || {
                with_game(|game| {
                    add_real_and_fake(game);
                    game.world.day_time = 18000;
                    game.stats.set_phantom_tick(0);
                    if !remove_phantom {
                        game.set_difficulty_forced(2);
                        set_spawn_rule(game, "spawn_mobs", false);
                        set_spawn_rule(game, "spawn_monsters", false);
                        set_spawn_rule(game, "spawn_monsters", true);
                        game.tick_mobs().unwrap(); // No custom spawner ticks while spawn_mobs is off.
                        set_spawn_rule(game, "spawn_mobs", true);
                        // The pass is now eligible, but its setting was last refreshed
                        // with spawn_mobs off: Phantom is skipped and the mod sees false.
                        game.tick_mobs().unwrap();
                        // A difficulty update refreshes the setting even in Peaceful.
                        game.set_difficulty_forced(0);
                    }
                    game.tick_mobs().unwrap();
                });
            });
            assert_eq!(*log.lock().unwrap(), expected);
        }
    }

    #[test]
    #[ignore = "daily"]
    fn custom_spawner_panic_and_transaction_refusal_containment() {
        const PANIC_MOD: &str = "spawner_panic_mod";
        const HEALTHY_MOD: &str = "spawner_healthy_mod";

        let panic_ticks = Arc::new(AtomicUsize::new(0));
        let refusal_ticks = Arc::new(AtomicUsize::new(0));
        let healthy_ticks = Arc::new(AtomicUsize::new(0));

        struct PanickingSpawner(Arc<AtomicUsize>);
        impl CustomSpawner for PanickingSpawner {
            fn tick(&mut self, host: &mut dyn ServerHost, _: LevelRef, _: bool) {
                self.0.fetch_add(1, Ordering::Relaxed);
                host.add_objective(
                    "spawner_panic_write",
                    "dummy",
                    Text::literal("Panic"),
                    ObjectiveRenderType::Integer,
                )
                .unwrap();
                panic!("custom spawner test panic");
            }
        }

        struct RefusingOnceSpawner {
            ticks: Arc<AtomicUsize>,
            should_refuse: bool,
        }
        impl CustomSpawner for RefusingOnceSpawner {
            fn tick(&mut self, host: &mut dyn ServerHost, _: LevelRef, _: bool) {
                self.ticks.fetch_add(1, Ordering::Relaxed);
                let refuse = std::mem::take(&mut self.should_refuse);
                let result = host.transaction(|host, _| {
                    host.add_objective(
                        "spawner_refusal_write",
                        "dummy",
                        Text::literal("Refusal"),
                        ObjectiveRenderType::Integer,
                    )
                    .unwrap();
                    if refuse { Err(()) } else { Ok(()) }
                });
                assert_eq!(result.is_err(), refuse);
            }
        }

        struct HealthySpawner(Arc<AtomicUsize>);
        impl CustomSpawner for HealthySpawner {
            fn tick(&mut self, host: &mut dyn ServerHost, _: LevelRef, _: bool) {
                let n = self.0.fetch_add(1, Ordering::Relaxed) + 1;
                if n == 1 {
                    host.add_objective(
                        "spawner_healthy_write",
                        "dummy",
                        Text::literal("Healthy"),
                        ObjectiveRenderType::Integer,
                    )
                    .unwrap();
                }
            }
        }

        struct PanicMod(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for PanicMod {
            fn modify_custom_spawners(
                &self,
                _: &dyn mod_api::Host,
                event: &mut mod_api::ModifyCustomSpawnersEvent<'_>,
            ) {
                event.spawners.add(PanickingSpawner(self.0.clone()));
            }
        }

        struct HealthyMod {
            refusal_ticks: Arc<AtomicUsize>,
            healthy_ticks: Arc<AtomicUsize>,
        }
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for HealthyMod {
            fn modify_custom_spawners(
                &self,
                _: &dyn mod_api::Host,
                event: &mut mod_api::ModifyCustomSpawnersEvent<'_>,
            ) {
                event.spawners.add(RefusingOnceSpawner {
                    ticks: self.refusal_ticks.clone(),
                    should_refuse: true,
                });
                event
                    .spawners
                    .add(HealthySpawner(self.healthy_ticks.clone()));
            }
        }

        let mods = vec![
            mod_api::ModEntry::new(PANIC_MOD, Box::new(PanicMod(panic_ticks.clone()))),
            mod_api::ModEntry::new(
                HEALTHY_MOD,
                Box::new(HealthyMod {
                    refusal_ticks: refusal_ticks.clone(),
                    healthy_ticks: healthy_ticks.clone(),
                }),
            ),
        ];
        mod_platform::testing::with_mods(mods.into(), || {
            with_game(|game| {
                game.tick_custom_spawners();
                assert_eq!(panic_ticks.load(Ordering::Relaxed), 1);
                assert_eq!(refusal_ticks.load(Ordering::Relaxed), 1);
                assert_eq!(healthy_ticks.load(Ordering::Relaxed), 1);
                assert!(
                    game.objective("spawner_panic_write").is_none(),
                    "panicking spawner writes must roll back"
                );
                assert!(
                    game.objective("spawner_refusal_write").is_none(),
                    "refused transaction writes must roll back"
                );
                assert!(
                    game.objective("spawner_healthy_write").is_some(),
                    "unaffected spawner commits normally"
                );

                // Second tick: the panicked spawner stays disabled for the session, while
                // the spawner that refused its transaction was not disabled and succeeds now.
                game.tick_custom_spawners();
                assert_eq!(panic_ticks.load(Ordering::Relaxed), 1);
                assert_eq!(refusal_ticks.load(Ordering::Relaxed), 2);
                assert_eq!(healthy_ticks.load(Ordering::Relaxed), 2);
                assert!(
                    game.objective("spawner_refusal_write").is_some(),
                    "the spawner commits once the transaction is accepted"
                );
            });
            assert_eq!(
                mod_api::__private::take_hook_panics(),
                [mod_api::__private::HookPanicReport {
                    mod_id: PANIC_MOD,
                    hook: "CustomSpawner::tick",
                }],
                "the shared panic queue reports the original spawner operation and mod"
            );
        });
    }
}
