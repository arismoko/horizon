use super::{damage_post, damage_pre, incoming_damage};
use mod_api::{
    DamageContainerView, Flow, HitHost, Hooks, LivingDamagePostEvent, LivingDamagePreEvent,
    LivingIncomingDamageEvent, ModEntry, Mods, Reduction,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use sunrise_sim::health::{HurtSource, Hurting, TestLevel, Victim};
use sunrise_world::living_damage::{
    DamageContainer, DamageStat, DamageWorld, Reduction as EngineReduction, actually_hurt,
};

struct Target<'a> {
    level: &'a mut TestLevel,
    entity: i32,
    health: f32,
    absorption: f32,
}

impl Target<'_> {
    fn hurting(entity: i32) -> Hurting<'static> {
        Hurting::Player(
            entity,
            HurtSource {
                damage_type: "minecraft:mob_attack",
                causing: 7,
                direct: 7,
                position: Some([1.0, 2.0, 3.0]),
                direct_type: None,
            },
        )
    }
}

impl DamageWorld for Target<'_> {
    type Error = ();
    type Attacker = ();
    fn invulnerable(&mut self) -> Result<bool, ()> {
        Ok(false)
    }
    fn after_armor(&mut self, damage: f32) -> Result<f32, ()> {
        Ok(damage * 0.5)
    }
    fn after_magic(&mut self, container: &mut DamageContainer, damage: f32) -> Result<f32, ()> {
        let after_effects = damage * 0.75;
        let after_effects = container.reduce(
            EngineReduction::MobEffects,
            damage,
            damage - after_effects,
            after_effects,
        );
        let after_enchantments = after_effects * 0.5;
        let after_enchantments = container.reduce(
            EngineReduction::Enchantments,
            after_effects,
            after_effects - after_enchantments,
            after_enchantments,
        );
        Ok(after_enchantments)
    }
    fn absorption(&mut self) -> Result<f32, ()> {
        Ok(self.absorption)
    }
    fn set_absorption(&mut self, value: f32) -> Result<(), ()> {
        self.absorption = value;
        Ok(())
    }
    fn server_player_attacker(&mut self) -> Result<Option<()>, ()> {
        Ok(None)
    }
    fn award_attacker(&mut self, _: (), _: DamageStat, _: i32) -> Result<(), ()> {
        Ok(())
    }
    fn award_self(&mut self, _: DamageStat, _: i32) -> Result<(), ()> {
        Ok(())
    }
    fn source_food_exhaustion(&mut self) -> Result<f32, ()> {
        Ok(0.0)
    }
    fn cause_food_exhaustion(&mut self, _: f32) -> Result<(), ()> {
        Ok(())
    }
    fn record_damage(&mut self, _: f32) -> Result<(), ()> {
        Ok(())
    }
    fn health(&mut self) -> Result<f32, ()> {
        Ok(self.health)
    }
    fn set_health(&mut self, value: f32) -> Result<(), ()> {
        self.health = value;
        Ok(())
    }
    fn damage_event(&mut self) -> Result<(), ()> {
        Ok(())
    }
    fn damage_pre(&mut self, container: &mut DamageContainer) -> Result<(), ()> {
        damage_pre(self.level.level(), Self::hurting(self.entity), container);
        Ok(())
    }
}

fn victim() -> Victim {
    Victim {
        position: [0.0, 64.0, 0.0],
        view: [0.0, 0.0, 1.0],
    }
}

fn run_simple_hit(original: f32, absorption: f32) -> DamageContainer {
    let mut level = TestLevel::new(0);
    let mut container = DamageContainer::new(original);
    let canceled = incoming_damage(
        level.level(),
        Target::hurting(4201),
        victim(),
        &mut container,
    );
    assert!(!canceled);
    let damage = container.new_damage().max(0.0);
    container.record_new_damage(damage);
    container.record_should_cause_side_effects(true);
    let mut target = Target {
        level: &mut level,
        entity: 4201,
        health: 100.0,
        absorption,
    };
    actually_hurt(&mut target, &mut container, damage, true).unwrap();
    damage_post(level.level(), Target::hurting(4201), &container);
    container
}

#[derive(Debug, PartialEq)]
struct ObservedStage {
    who: &'static str,
    original: f32,
    new_damage: f32,
    invuln: f32,
    armor: f32,
    mob_effects: f32,
    enchantments: f32,
    blocked: f32,
    shield_damage: f32,
    post_ticks: i32,
    side_effects: bool,
    preceding: f32,
}

#[derive(Debug, PartialEq)]
struct ObservedPost {
    inflicted: f32,
    health_damage: f32,
    absorption: f32,
    side_effects: bool,
    blocked: f32,
    shield_damage: f32,
    post_ticks: i32,
}

#[test]
fn two_mods_reduction_order_live_views_pre_absorption_and_post() {
    let stages = Arc::new(Mutex::new(Vec::<ObservedStage>::new()));
    let post = Arc::new(Mutex::new(Vec::<ObservedPost>::new()));

    struct ModA(Arc<Mutex<Vec<ObservedStage>>>);
    #[mod_api::__subscriptions]
    impl Hooks for ModA {
        fn living_incoming_damage(
            &self,
            _: &mut dyn HitHost,
            event: &mut LivingIncomingDamageEvent<'_>,
        ) -> Flow {
            let log = Arc::clone(&self.0);
            event
                .container
                .add_reduction_modifier(Reduction::Armor, move |view, amount| {
                    log.lock().unwrap().push(ObservedStage {
                        who: "mod_a:armor",
                        original: view.original(),
                        new_damage: view.new_damage(),
                        invuln: view.reduction(Reduction::Invulnerability),
                        armor: view.reduction(Reduction::Armor),
                        mob_effects: view.reduction(Reduction::MobEffects),
                        enchantments: view.reduction(Reduction::Enchantments),
                        blocked: view.blocked(),
                        shield_damage: view.shield_damage(),
                        post_ticks: view.post_attack_invulnerability_ticks(),
                        side_effects: view.should_cause_side_effects(),
                        preceding: amount,
                    });
                    amount * 0.5
                });
            Flow::Continue
        }

        fn living_damage_pre(&self, _: &mut dyn HitHost, event: &mut LivingDamagePreEvent<'_>) {
            let log = Arc::clone(&self.0);
            event
                .container
                .add_reduction_modifier(Reduction::Absorption, move |view, amount| {
                    log.lock().unwrap().push(ObservedStage {
                        who: "mod_a:absorption",
                        original: view.original(),
                        new_damage: view.new_damage(),
                        invuln: view.reduction(Reduction::Invulnerability),
                        armor: view.reduction(Reduction::Armor),
                        mob_effects: view.reduction(Reduction::MobEffects),
                        enchantments: view.reduction(Reduction::Enchantments),
                        blocked: view.blocked(),
                        shield_damage: view.shield_damage(),
                        post_ticks: view.post_attack_invulnerability_ticks(),
                        side_effects: view.should_cause_side_effects(),
                        preceding: amount,
                    });
                    amount * 0.5
                });
        }
    }

    struct ModB {
        stages: Arc<Mutex<Vec<ObservedStage>>>,
        post: Arc<Mutex<Vec<ObservedPost>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for ModB {
        fn living_incoming_damage(
            &self,
            _: &mut dyn HitHost,
            event: &mut LivingIncomingDamageEvent<'_>,
        ) -> Flow {
            let log = Arc::clone(&self.stages);
            event
                .container
                .add_reduction_modifier(Reduction::Armor, move |view, amount| {
                    log.lock().unwrap().push(ObservedStage {
                        who: "mod_b:armor",
                        original: view.original(),
                        new_damage: view.new_damage(),
                        invuln: view.reduction(Reduction::Invulnerability),
                        armor: view.reduction(Reduction::Armor),
                        mob_effects: view.reduction(Reduction::MobEffects),
                        enchantments: view.reduction(Reduction::Enchantments),
                        blocked: view.blocked(),
                        shield_damage: view.shield_damage(),
                        post_ticks: view.post_attack_invulnerability_ticks(),
                        side_effects: view.should_cause_side_effects(),
                        preceding: amount,
                    });
                    amount + 2.0
                });
            Flow::Continue
        }

        fn living_damage_post(&self, _: &mut dyn HitHost, event: &mut LivingDamagePostEvent<'_>) {
            let view: DamageContainerView<'_> = event.container;
            self.post.lock().unwrap().push(ObservedPost {
                inflicted: view.inflicted(),
                health_damage: view.health_damage(),
                absorption: view.reduction(Reduction::Absorption),
                side_effects: view.should_cause_side_effects(),
                blocked: view.blocked(),
                shield_damage: view.shield_damage(),
                post_ticks: view.post_attack_invulnerability_ticks(),
            });
        }
    }

    let mods: Mods = vec![
        ModEntry::new("mod_a", Box::new(ModA(Arc::clone(&stages)))),
        ModEntry::new(
            "mod_b",
            Box::new(ModB {
                stages: Arc::clone(&stages),
                post: Arc::clone(&post),
            }),
        ),
    ]
    .into();

    mod_platform::testing::with_mods(mods, || {
        let mut level = TestLevel::new(0);
        let mut container = DamageContainer::new(24.0);
        assert!(!incoming_damage(
            level.level(),
            Target::hurting(4200),
            victim(),
            &mut container,
        ));
        // Simulate shield block and cooldown stage updates before `actually_hurt`.
        container.record_block(2.0);
        container.record_shield_damage(3.0);
        let mut damage = container.new_damage() - 2.0;
        container.record_new_damage(damage);
        container.record_should_cause_side_effects(false);
        container.record_post_attack_invulnerability_ticks(15);
        damage = container.reduce(EngineReduction::Invulnerability, damage, 2.0, damage - 2.0);
        assert_eq!(damage, 20.0);

        let mut target = Target {
            level: &mut level,
            entity: 4200,
            health: 100.0,
            absorption: 4.0,
        };
        actually_hurt(&mut target, &mut container, damage, true).unwrap();
        let (health, absorption) = (target.health, target.absorption);
        damage_post(level.level(), Target::hurting(4200), &container);

        assert_eq!(container.reduction(EngineReduction::Armor), 7.0);
        assert_eq!(container.reduction(EngineReduction::MobEffects), 3.25);
        assert_eq!(container.reduction(EngineReduction::Enchantments), 4.875);
        assert_eq!(container.reduction(EngineReduction::Absorption), 2.0);
        assert_eq!(health, 97.125);
        assert_eq!(absorption, 2.0);
    });

    assert_eq!(
        *stages.lock().unwrap(),
        vec![
            ObservedStage {
                who: "mod_a:armor",
                original: 24.0,
                new_damage: 20.0,
                invuln: 2.0,
                armor: 0.0,
                mob_effects: 0.0,
                enchantments: 0.0,
                blocked: 2.0,
                shield_damage: 3.0,
                post_ticks: 15,
                side_effects: false,
                preceding: 10.0,
            },
            ObservedStage {
                who: "mod_b:armor",
                original: 24.0,
                new_damage: 20.0,
                invuln: 2.0,
                armor: 0.0,
                mob_effects: 0.0,
                enchantments: 0.0,
                blocked: 2.0,
                shield_damage: 3.0,
                post_ticks: 15,
                side_effects: false,
                preceding: 5.0,
            },
            ObservedStage {
                who: "mod_a:absorption",
                original: 24.0,
                new_damage: 4.875,
                invuln: 2.0,
                armor: 7.0,
                mob_effects: 3.25,
                enchantments: 4.875,
                blocked: 2.0,
                shield_damage: 3.0,
                post_ticks: 15,
                side_effects: false,
                preceding: 4.0,
            },
        ]
    );
    assert_eq!(
        *post.lock().unwrap(),
        vec![ObservedPost {
            inflicted: 4.875,
            health_damage: 2.875,
            absorption: 2.0,
            side_effects: false,
            blocked: 2.0,
            shield_damage: 3.0,
            post_ticks: 15,
        }]
    );
}

#[test]
fn panicking_reduction_callback_is_isolated_disabled_and_reported() {
    let bad_incoming_runs = Arc::new(AtomicUsize::new(0));
    let bad_reduction_runs = Arc::new(AtomicUsize::new(0));
    let good_reduction_runs = Arc::new(AtomicUsize::new(0));

    struct BadMod {
        incoming: Arc<AtomicUsize>,
        reduction: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for BadMod {
        fn living_incoming_damage(
            &self,
            _: &mut dyn HitHost,
            event: &mut LivingIncomingDamageEvent<'_>,
        ) -> Flow {
            self.incoming.fetch_add(1, Ordering::Relaxed);
            let runs = Arc::clone(&self.reduction);
            event
                .container
                .add_reduction_modifier(Reduction::Armor, move |_, _| {
                    runs.fetch_add(1, Ordering::Relaxed);
                    panic!("boom in armor reduction");
                });
            Flow::Continue
        }
    }

    struct GoodMod(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for GoodMod {
        fn living_incoming_damage(
            &self,
            _: &mut dyn HitHost,
            event: &mut LivingIncomingDamageEvent<'_>,
        ) -> Flow {
            let runs = Arc::clone(&self.0);
            event
                .container
                .add_reduction_modifier(Reduction::Armor, move |_, amount| {
                    runs.fetch_add(1, Ordering::Relaxed);
                    amount + 2.0
                });
            Flow::Continue
        }
    }

    let mods: Mods = vec![
        ModEntry::new(
            "bad_mod",
            Box::new(BadMod {
                incoming: Arc::clone(&bad_incoming_runs),
                reduction: Arc::clone(&bad_reduction_runs),
            }),
        ),
        ModEntry::new(
            "good_mod",
            Box::new(GoodMod(Arc::clone(&good_reduction_runs))),
        ),
    ]
    .into();

    mod_platform::testing::with_mods(mods, || {
        // Hit 1: bad_mod panics, leaves 10.0 unchanged, good_mod adds 2.0 -> 12.0.
        let first = run_simple_hit(20.0, 0.0);
        assert_eq!(first.reduction(EngineReduction::Armor), 12.0);
        assert_eq!(bad_incoming_runs.load(Ordering::Relaxed), 1);
        assert_eq!(bad_reduction_runs.load(Ordering::Relaxed), 1);
        assert_eq!(good_reduction_runs.load(Ordering::Relaxed), 1);
        assert_eq!(
            mod_api::__private::take_hook_panics(),
            vec![mod_api::__private::HookPanicReport {
                mod_id: "bad_mod",
                hook: "living_damage_reduction",
            }]
        );

        // Hit 2: bad_mod's living_incoming_damage still fires, its reduction callback is disabled,
        // and good_mod continues running cleanly.
        let second = run_simple_hit(20.0, 0.0);
        assert_eq!(second.reduction(EngineReduction::Armor), 12.0);
        assert_eq!(bad_incoming_runs.load(Ordering::Relaxed), 2);
        assert_eq!(bad_reduction_runs.load(Ordering::Relaxed), 1);
        assert_eq!(good_reduction_runs.load(Ordering::Relaxed), 2);
        assert!(mod_api::__private::take_hook_panics().is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn production_crash_hook_contains_reduction_panics_on_server_thread() {
    const CHILD: &str = "SUNRISE_DAMAGE_REDUCTION_CRASH_CHILD";
    let home = tempfile::tempdir().expect("tempdir for crash reports");
    if std::env::var_os(CHILD).is_none() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "game::native_hooks::live_damage_tests::production_crash_hook_contains_reduction_panics_on_server_thread",
            ])
            .env(CHILD, "1")
            .current_dir(home.path())
            .output()
            .expect("run crash-hook witness in its own process");
        assert!(
            result.status.success() && String::from_utf8_lossy(&result.stdout).contains("1 passed"),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr),
        );
        return;
    }
    sunrise_level::crash::install(home.path().to_path_buf(), "client", || None);

    let panic_runs = Arc::new(AtomicUsize::new(0));
    let payload_drops = Arc::new(AtomicUsize::new(0));
    let panic_runs_thread = Arc::clone(&panic_runs);
    let payload_drops_thread = Arc::clone(&payload_drops);
    let reports_dir = home.path().join("crash-reports");

    let handle = std::thread::Builder::new()
        .name(sunrise_level::crash::INTEGRATED_SERVER_THREAD.into())
        .spawn(move || {
            struct PanicPayload(Arc<AtomicUsize>);
            impl Drop for PanicPayload {
                fn drop(&mut self) {
                    self.0.fetch_add(1, Ordering::Relaxed);
                    panic!("reduction panic payload destructor");
                }
            }
            struct ServerPanicMod {
                runs: Arc<AtomicUsize>,
                payload_drops: Arc<AtomicUsize>,
            }
            #[mod_api::__subscriptions]
            impl Hooks for ServerPanicMod {
                fn living_incoming_damage(
                    &self,
                    _: &mut dyn HitHost,
                    event: &mut LivingIncomingDamageEvent<'_>,
                ) -> Flow {
                    let runs = Arc::clone(&self.runs);
                    let payload_drops = Arc::clone(&self.payload_drops);
                    event
                        .container
                        .add_reduction_modifier(Reduction::Armor, move |_, _| {
                            runs.fetch_add(1, Ordering::Relaxed);
                            std::panic::panic_any(PanicPayload(Arc::clone(&payload_drops)));
                        });
                    Flow::Continue
                }
            }

            struct ServerGoodMod;
            #[mod_api::__subscriptions]
            impl Hooks for ServerGoodMod {
                fn living_incoming_damage(
                    &self,
                    _: &mut dyn HitHost,
                    event: &mut LivingIncomingDamageEvent<'_>,
                ) -> Flow {
                    event
                        .container
                        .add_reduction_modifier(Reduction::Armor, |_, amount| amount + 2.0);
                    Flow::Continue
                }
            }

            struct PanicOnDrop;
            impl Drop for PanicOnDrop {
                fn drop(&mut self) {
                    panic!("reduction capture destructor");
                }
            }
            struct ServerDropMod;
            #[mod_api::__subscriptions]
            impl Hooks for ServerDropMod {
                fn living_incoming_damage(
                    &self,
                    _: &mut dyn HitHost,
                    event: &mut LivingIncomingDamageEvent<'_>,
                ) -> Flow {
                    if event.entity == 4202 {
                        let capture = PanicOnDrop;
                        event
                            .container
                            .add_reduction_modifier(Reduction::Armor, move |_, _| {
                                let _capture = &capture;
                                panic!("a cancelled hit must not run armor modifiers");
                            });
                        Flow::Cancel
                    } else {
                        event
                            .container
                            .add_reduction_modifier(Reduction::Armor, |_, amount| amount + 8.0);
                        Flow::Continue
                    }
                }
            }

            let mods: Mods = vec![
                ModEntry::new("server_drop_mod", Box::new(ServerDropMod)),
                ModEntry::new(
                    "server_panic_mod",
                    Box::new(ServerPanicMod {
                        runs: panic_runs_thread,
                        payload_drops: payload_drops_thread,
                    }),
                ),
                ModEntry::new("server_good_mod", Box::new(ServerGoodMod)),
            ]
            .into();

            mod_platform::testing::with_mods(mods, || {
                let mut level = TestLevel::new(0);
                let mut cancelled = DamageContainer::new(20.0);
                assert!(incoming_damage(
                    level.level(),
                    Target::hurting(4202),
                    victim(),
                    &mut cancelled
                ));
                drop(cancelled);
                let faults = mod_api::__private::take_hook_panics();
                assert_eq!(faults.len(), 1);
                assert_eq!(faults[0].mod_id, "server_drop_mod");
                assert_eq!(faults[0].hook, "living_damage_reduction");
                assert!(!sunrise_level::crash::server_crashed());
                assert!(!reports_dir.exists());

                // A reduction panic does not trip `server_crashed()` or write a crash report.
                let first = run_simple_hit(20.0, 0.0);
                assert_eq!(first.reduction(EngineReduction::Armor), 12.0);
                assert!(!sunrise_level::crash::server_crashed());
                assert!(!reports_dir.exists());

                let faults = mod_api::__private::take_hook_panics();
                assert_eq!(faults.len(), 1);
                assert_eq!(faults[0].mod_id, "server_panic_mod");
                assert_eq!(faults[0].hook, "living_damage_reduction");

                // The second hit still gets server_good_mod's adjustment.
                let second = run_simple_hit(20.0, 0.0);
                assert_eq!(second.reduction(EngineReduction::Armor), 12.0);
                assert!(!sunrise_level::crash::server_crashed());
                assert!(!reports_dir.exists());
                assert!(mod_api::__private::take_hook_panics().is_empty());
            });
        })
        .expect("spawn integrated server test thread");

    handle.join().expect("server thread must not panic");
    assert_eq!(payload_drops.load(Ordering::Relaxed), 1);
    assert_eq!(panic_runs.load(Ordering::Relaxed), 1);
}
