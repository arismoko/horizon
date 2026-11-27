#![cfg(test)]
use mod_api::Authority;

use sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycleCommonGameExt1 as _;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use mod_api::ServerTickPhase;
    use mod_api::{Hooks, ModEntry, ServerHost};
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycle;
    use sunrise_gameplay_common::game::transactions::TransactionsGameExt;

    struct TaskOwner;
    #[mod_api::__subscriptions]
    impl Hooks for TaskOwner {}

    #[test]
    fn client_epochs_never_emit_server_lifecycle_events() {
        let mut lifecycle = NativeLifecycle::default();
        lifecycle.begin_open(None);
        let epoch = lifecycle.epoch().unwrap();
        assert!(lifecycle.is_opening());
        assert!(lifecycle.world_open_ready().is_none());
        assert!(!lifecycle.is_opening());
        assert_eq!(lifecycle.epoch(), Some(epoch));
        assert!(lifecycle.server_tick_start(true).is_none());
        assert!(lifecycle.server_tick_end(false).is_none());
        lifecycle.begin_close();
        assert!(lifecycle.retire());
        assert!(lifecycle.epoch().is_none());

        lifecycle.begin_open(None);
        assert_ne!(lifecycle.epoch(), Some(epoch));
        assert!(lifecycle.retire());
        assert!(lifecycle.epoch().is_none());
    }

    #[test]
    fn frozen_steps_are_observed_without_consuming_them() {
        let mut lifecycle = opening();
        lifecycle.world_open_ready();
        let mut clock = sunrise_sim::clock::FixedStep::default();
        clock.set_frozen(true);
        clock.set_frozen_ticks_to_run(4);
        let mut observed = 0;
        for _ in 0..6 {
            let running = clock.world_runs();
            let (_, start) = lifecycle.server_tick_start(running).unwrap();
            let (_, end) = lifecycle.server_tick_end(false).unwrap();
            assert_eq!(
                start,
                ServerTickPhase::ServerTickStart {
                    world_running: running
                }
            );
            assert_eq!(
                end,
                ServerTickPhase::ServerTickEnd {
                    world_running: running
                }
            );
            observed += usize::from(running);
        }
        assert_eq!(observed, 4);
    }

    /// A task a mod schedules from the tick's start hook (after `server_tick_start` moved "now")
    /// fires on the same tick for delay 0, on the next tick for delay 1, and on the Nth tick
    /// after for delay N. A paused pass moves nothing, and closing the world drops what waits.
    #[test]
    fn tasks_scheduled_from_the_start_hook_fire_on_a_later_tick() {
        let owner = Some("mymod");
        let mut lifecycle = opening();
        lifecycle.world_open_ready();
        let mut fired = Vec::new();
        for tick in 1..=6u64 {
            assert!(lifecycle.server_tick_start(true).is_some());
            // The start hook runs here: it schedules on the first tick only.
            if tick == 1 {
                for delay in [0, 1, 3] {
                    lifecycle
                        .tasks
                        .schedule(owner, delay, Box::new(|_| {}))
                        .unwrap();
                }
            }
            while lifecycle.tasks.pop_due().is_some() {
                fired.push(tick);
            }
            lifecycle.server_tick_end(false);
        }
        assert_eq!(fired, [1, 2, 4]);

        lifecycle
            .tasks
            .schedule(owner, 1, Box::new(|_| {}))
            .unwrap();
        assert!(lifecycle.server_tick_end(true).is_none());
        assert!(lifecycle.tasks.pop_due().is_none());
        lifecycle.begin_close();
        assert!(lifecycle.retire());
        lifecycle.begin_open(Some(Authority::IntegratedServer));
        lifecycle.world_open_ready();
        lifecycle.server_tick_start(true);
        assert!(lifecycle.tasks.pop_due().is_none());
    }

    #[test]
    #[ignore = "daily"]
    fn due_tasks_run_only_for_enabled_owners_under_their_identity() {
        use mod_api::{__private as api, content::crash::current_native_mod};
        use std::{cell::Cell, rc::Rc};

        let mods = vec![
            ModEntry::new("task_owner", Box::new(TaskOwner)),
            ModEntry::new("task_disabled", Box::new(TaskOwner)),
        ];
        mod_platform::testing::with_mods(mods.into(), || {
            with_game(|game| {
                game.native_lifecycle.world_open_ready().unwrap();
                let capture = Rc::new(());
                let retained = Rc::clone(&capture);
                {
                    let host: &mut dyn ServerHost = game;
                    let _owner = api::enter("task_disabled", "schedule fixture");
                    host.schedule_task(
                        1,
                        Box::new(move |_| {
                            drop(retained);
                            panic!("a disabled owner's callback must not run");
                        }),
                    )
                    .unwrap();
                }
                // TaskId is an opaque capability: another caller can cancel a waiting task, and
                // its captures still drop under the scheduling owner's identity.
                struct DropObserver(Rc<Cell<Option<&'static str>>>);
                impl Drop for DropObserver {
                    fn drop(&mut self) {
                        self.0.set(current_native_mod());
                    }
                }
                let dropped_under = Rc::new(Cell::new(None));
                let cancelled_id = {
                    let host: &mut dyn ServerHost = game;
                    let _owner = api::enter("task_disabled", "schedule fixture");
                    let observer = DropObserver(Rc::clone(&dropped_under));
                    host.schedule_task(
                        1,
                        Box::new(move |_| {
                            let _keep = &observer;
                            panic!("a cancelled task must not run");
                        }),
                    )
                    .unwrap()
                };
                let hit = Rc::new(Cell::new(0));
                {
                    let host: &mut dyn ServerHost = game;
                    let _owner = api::enter("task_owner", "schedule fixture");
                    assert!(host.cancel_task(cancelled_id));
                    assert_eq!(
                        dropped_under.get(),
                        Some("task_disabled"),
                        "cancelled captures drop under the scheduling owner's attribution"
                    );
                    assert_eq!(current_native_mod(), Some("task_owner"));
                    assert!(!host.cancel_task(cancelled_id));
                    let hit = Rc::clone(&hit);
                    host.schedule_task(
                        1,
                        Box::new(move |host| {
                            assert_eq!(current_native_mod(), Some("task_owner"));
                            hit.set(1);
                            let same_tick = Rc::clone(&hit);
                            host.schedule_task(
                                0,
                                Box::new(move |_| {
                                    assert_eq!(current_native_mod(), Some("task_owner"));
                                    same_tick.set(2);
                                }),
                            )
                            .unwrap();
                            let next_tick = Rc::clone(&hit);
                            host.schedule_task(
                                1,
                                Box::new(move |_| {
                                    assert_eq!(current_native_mod(), Some("task_owner"));
                                    next_tick.set(3);
                                }),
                            )
                            .unwrap();
                        }),
                    )
                    .unwrap();
                }

                assert!(api::set_mod_enabled("task_disabled", false));
                game.native_lifecycle_server_tick_start(true);
                assert_eq!(
                    hit.get(),
                    2,
                    "a nested delay-0 task completes in the active start drain"
                );
                assert_eq!(Rc::strong_count(&capture), 1, "disabled captures are gone");
                assert_eq!(current_native_mod(), None);

                // A finite delay-0 chain scheduled during the end drain completes without
                // advancing the tick.
                let chain = Rc::new(Cell::new(0));
                {
                    let host: &mut dyn ServerHost = game;
                    let _owner = api::enter("task_owner", "schedule fixture");
                    let chain_a = Rc::clone(&chain);
                    host.schedule_task(
                        0,
                        Box::new(move |host| {
                            assert_eq!(chain_a.get(), 0);
                            chain_a.set(1);
                            let chain_b = Rc::clone(&chain_a);
                            host.schedule_task(
                                0,
                                Box::new(move |host| {
                                    assert_eq!(chain_b.get(), 1);
                                    chain_b.set(2);
                                    let chain_c = Rc::clone(&chain_b);
                                    host.schedule_task(
                                        0,
                                        Box::new(move |_| {
                                            assert_eq!(chain_c.get(), 2);
                                            chain_c.set(3);
                                        }),
                                    )
                                    .unwrap();
                                }),
                            )
                            .unwrap();
                        }),
                    )
                    .unwrap();
                }
                game.native_lifecycle_server_tick_end(false);
                assert_eq!(
                    chain.get(),
                    3,
                    "a finite delay-0 chain scheduled during the end drain completes in the same tick"
                );
                assert_eq!(current_native_mod(), None);

                assert!(api::set_mod_enabled("task_disabled", true));
                game.native_lifecycle_server_tick_start(true);
                assert_eq!(hit.get(), 3, "a delay-1 task runs on the next tick");
                game.native_lifecycle_server_tick_end(false);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn panicking_scheduled_write_rolls_back_before_the_next_task() {
        use mod_api::{ObjectiveRenderType, Text, TransactionLifecycle};
        use std::{cell::Cell, rc::Rc};

        const FAILING: &str = "task_owner";
        const HEALTHY: &str = "other_task_owner";
        let mods = vec![
            ModEntry::new(FAILING, Box::new(TaskOwner)),
            ModEntry::new(HEALTHY, Box::new(TaskOwner)),
        ];
        mod_platform::testing::with_mods(mods.into(), || {
            with_game(|game| {
                game.native_lifecycle.world_open_ready().unwrap();
                let skipped = Rc::new(Cell::new(false));
                let after_reset = Rc::new(Cell::new(false));
                let unrelated = Rc::new(Cell::new(false));
                {
                    let host: &mut dyn ServerHost = game;
                    let _owner = mod_api::__private::enter(FAILING, "schedule fixture");
                    host.schedule_task(
                        1,
                        Box::new(|host| {
                            host.add_objective(
                                "discarded",
                                "dummy",
                                Text::literal("Discarded"),
                                ObjectiveRenderType::Integer,
                            )
                            .unwrap();
                            panic!("scheduled callback panicked");
                        }),
                    )
                    .unwrap();
                    let skipped = Rc::clone(&skipped);
                    host.schedule_task(1, Box::new(move |_| skipped.set(true)))
                        .unwrap();
                }
                {
                    let host: &mut dyn ServerHost = game;
                    let _owner = mod_api::__private::enter(HEALTHY, "schedule fixture");
                    let unrelated = Rc::clone(&unrelated);
                    host.schedule_task(
                        1,
                        Box::new(move |host| {
                            assert!(host.objective("discarded").is_none());
                            host.add_objective(
                                "kept",
                                "dummy",
                                Text::literal("Kept"),
                                ObjectiveRenderType::Integer,
                            )
                            .unwrap();
                            unrelated.set(true);
                        }),
                    )
                    .unwrap();
                }
                game.native_lifecycle_server_tick_start(true);
                game.native_lifecycle_server_tick_end(false);
                assert!(!skipped.get(), "the failed owner's next task ran");
                assert!(unrelated.get(), "the other owner's task did not finish");
                assert!(game.objective("discarded").is_none());
                assert!(game.objective("kept").is_some());
                assert_eq!(game.transaction_lifecycle(), TransactionLifecycle::None);
                // The poison outlives the world's queue without turning off the mod itself.
                let index = mod_api::__private::mod_index(FAILING).unwrap();
                assert!(mod_api::__private::mod_index_on(index));
                game.native_lifecycle
                    .begin_open(Some(Authority::IntegratedServer));
                game.native_lifecycle.world_open_ready().unwrap();
                {
                    let host: &mut dyn ServerHost = game;
                    let _owner = mod_api::__private::enter(FAILING, "schedule fixture");
                    let after_reset = Rc::clone(&after_reset);
                    host.schedule_task(0, Box::new(move |_| after_reset.set(true)))
                        .unwrap();
                }
                game.native_lifecycle_server_tick_start(true);
                game.native_lifecycle_server_tick_end(false);
                assert!(!after_reset.get(), "reopening cleared the session poison");
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_scheduled_transaction_refusal_rolls_back_without_disabling_later_tasks() {
        use mod_api::{
            __private::enter, HostTransaction, ObjectiveError, ObjectiveRenderType, Text,
        };
        use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;

        if run_fresh_cases(
            "cut_layout::game::server_side::native_lifecycle::tests::a_scheduled_transaction_refusal_rolls_back_without_disabling_later_tasks",
            "SUNRISE_SCHEDULED_TRANSACTION_REFUSAL_CHILD",
            &["refused"],
        ) {
            return;
        }

        const OWNER: &str = "task_owner";
        let mods = vec![ModEntry::new(OWNER, Box::new(TaskOwner))];
        mod_platform::testing::with_mods(mods.into(), || {
            with_game(|game| {
                game.native_lifecycle.world_open_ready().unwrap();
                game.native_lifecycle_server_tick_start(true);
                {
                    let host: &mut dyn ServerHost = game;
                    let _owner = enter(OWNER, "schedule fixture");
                    host.schedule_task(
                        0,
                        Box::new(|host| {
                            let result: Result<(), mod_api::TransactionError<ObjectiveError>> =
                                host.transaction(|host, _| {
                                    // The second write refuses the duplicate, rolling back the first.
                                    for _ in 0..2 {
                                        host.add_objective(
                                            "discarded",
                                            "dummy",
                                            Text::literal("Discarded"),
                                            ObjectiveRenderType::Integer,
                                        )?;
                                    }
                                    Ok(())
                                });
                            assert_eq!(
                                result,
                                Err(mod_api::TransactionError::Aborted(
                                    ObjectiveError::NameTaken
                                ))
                            );
                        }),
                    )
                    .unwrap();
                    host.schedule_task(
                        0,
                        Box::new(|host| {
                            assert!(host.objective("discarded").is_none());
                            host.add_objective(
                                "kept",
                                "dummy",
                                Text::literal("Kept"),
                                ObjectiveRenderType::Integer,
                            )
                            .unwrap();
                        }),
                    )
                    .unwrap();
                }
                game.native_lifecycle_server_tick_end(false);
                assert!(game.objective("discarded").is_none());
                assert!(
                    game.objective("kept").is_some(),
                    "the owner's next task did not commit"
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn panicking_scheduled_task_captures_do_not_interrupt_cancellation_or_close() {
        use mod_api::{
            __private::{HookPanicReport, enter, take_hook_panics},
            content::crash::current_native_mod,
        };
        use std::{cell::RefCell, rc::Rc};
        use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;
        use sunrise_sim::{join_seq::residency::table::EntityLookup, mobs::Mob};

        if run_fresh_cases(
            "cut_layout::game::server_side::native_lifecycle::tests::panicking_scheduled_task_captures_do_not_interrupt_cancellation_or_close",
            "SUNRISE_SCHEDULED_TASK_DROP_CHILD",
            &["dispose"],
        ) {
            return;
        }
        let home = std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap());
        sunrise_level::logging::init(&home, "Server thread");
        sunrise_level::crash::install(home, "server", || None);

        const CANCELLED: &str = "cancelled_task_owner";
        const CLOSED: &str = "closed_task_owner";
        const HEALTHY: &str = "healthy_task_owner";
        struct PanickingCapture(&'static str);
        impl Drop for PanickingCapture {
            fn drop(&mut self) {
                panic!("{}", self.0);
            }
        }

        let mods: Vec<_> = [CANCELLED, CLOSED, HEALTHY]
            .into_iter()
            .map(|id| ModEntry::new(id, Box::new(TaskOwner)))
            .collect();
        // The log keeps only its recent lines, and each panic's backtrace fills them, so each
        // panic's attributed line is read before the next panic.
        let assert_logged = |owner: &str, message: &str| {
            assert!(
                sunrise_level::logging::recent_lines()
                    .iter()
                    .any(|line| line.contains(owner) && line.contains(message)),
                "{owner}'s panic is logged with its message"
            );
        };
        mod_platform::testing::with_mods(mods.into(), || {
            with_game(|game| {
                game.native_lifecycle.world_open_ready().unwrap();
                let cancelled = {
                    let _owner = enter(CANCELLED, "schedule fixture");
                    let capture = PanickingCapture("cancelled capture panicked");
                    game.schedule_task(20, Box::new(move |_| drop(capture)))
                        .unwrap()
                };
                {
                    let _caller = enter(HEALTHY, "cancel fixture");
                    assert!(game.cancel_task(cancelled));
                    assert_eq!(current_native_mod(), Some(HEALTHY));
                    assert!(!game.cancel_task(cancelled));
                }
                assert_logged(CANCELLED, "cancelled capture panicked");
                let closed = {
                    let _owner = enter(CLOSED, "schedule fixture");
                    let capture = PanickingCapture("closing capture panicked");
                    game.schedule_task(20, Box::new(move |_| drop(capture)))
                        .unwrap()
                };
                let healthy_capture = Rc::new(());
                let pending = Rc::clone(&healthy_capture);
                let later = game
                    .schedule_task(20, Box::new(move |_| drop(pending)))
                    .unwrap();

                let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                game.mobs.push(Mob::new(410, pig, Default::default(), 0.0));
                assert!(matches!(
                    game.server_resident_entity(410),
                    EntityLookup::Found { .. }
                ));
                game.native_lifecycle.begin_close();
                game.native_lifecycle_world_closed();
                assert!(game.native_lifecycle.epoch().is_none());
                assert!(matches!(
                    game.server_resident_entity(410),
                    EntityLookup::Missing
                ));
                assert_eq!(Rc::strong_count(&healthy_capture), 1);

                assert_eq!(
                    take_hook_panics(),
                    [CANCELLED, CLOSED].map(|mod_id| HookPanicReport {
                        mod_id,
                        hook: "schedule_task_drop",
                    }),
                );
                assert_logged(CLOSED, "closing capture panicked");

                game.native_lifecycle
                    .begin_open(Some(Authority::IntegratedServer));
                game.native_lifecycle.world_open_ready().unwrap();
                let ran = Rc::new(RefCell::new(Vec::new()));
                for owner in [CANCELLED, CLOSED, HEALTHY] {
                    let _owner = enter(owner, "schedule fixture");
                    let ran = Rc::clone(&ran);
                    let fresh = game
                        .schedule_task(0, Box::new(move |_| ran.borrow_mut().push(owner)))
                        .unwrap();
                    assert!(![cancelled, closed, later].contains(&fresh));
                }
                for stale in [cancelled, closed, later] {
                    assert!(!game.cancel_task(stale));
                }
                game.native_lifecycle_server_tick_start(true);
                game.native_lifecycle_server_tick_end(false);
                assert_eq!(*ran.borrow(), [HEALTHY]);
            });
        });
    }

    /// A lifecycle mid-open on the server thread.
    fn opening() -> NativeLifecycle {
        let mut lifecycle = NativeLifecycle::default();
        lifecycle.begin_open(Some(Authority::IntegratedServer));
        lifecycle
    }

    #[test]
    fn server_lifecycle_keeps_the_authority_and_epoch_from_ready_through_close() {
        for authority in [Authority::IntegratedServer, Authority::DedicatedServer] {
            let mut lifecycle = NativeLifecycle::default();
            lifecycle.begin_open(Some(authority));
            let epoch = lifecycle.epoch().unwrap();

            let ready = lifecycle.world_open_ready().expect("the first ready fires");
            assert_eq!(ready.authority, authority);
            assert_eq!(ready.epoch, epoch);
            assert_eq!(ready.tick.get(), 0);
            assert!(lifecycle.world_open_ready().is_none(), "at most once");

            let (start, _) = lifecycle.server_tick_start(true).unwrap();
            let (end, _) = lifecycle.server_tick_end(false).unwrap();
            assert_eq!(start.authority, authority);
            assert_eq!(start.epoch, epoch);
            assert_eq!(start.tick.get(), 1);
            assert_eq!(end, start);

            lifecycle.begin_close();
            assert!(lifecycle.retire());
            assert!(lifecycle.epoch().is_none());
        }
    }

    #[test]
    fn a_failed_open_invalidates_and_a_retry_is_fresh() {
        let mut lifecycle = opening();
        let failed_epoch = lifecycle.epoch().expect("open allocated an epoch");
        assert!(lifecycle.is_opening());
        assert!(lifecycle.retire());
        assert!(!lifecycle.is_opening());
        assert!(lifecycle.world_open_ready().is_none(), "the epoch is gone");
        assert!(lifecycle.server_tick_start(true).is_none());
        lifecycle.begin_open(Some(Authority::IntegratedServer));
        let context = lifecycle.world_open_ready().expect("the retry opens fresh");
        assert!(context.epoch.get() > failed_epoch.get(), "a fresh epoch");
    }

    #[test]
    fn retire_clears_opening_ready_and_closing_and_is_idempotent() {
        let mut lifecycle = opening();
        assert!(lifecycle.is_opening());
        assert!(lifecycle.retire());
        assert!(!lifecycle.retire(), "a second retire is a no-op");
        assert!(!lifecycle.is_opening());
        assert!(lifecycle.epoch().is_none());
        assert!(lifecycle.world_open_ready().is_none());
        assert!(lifecycle.server_tick_start(true).is_none());

        lifecycle.begin_open(Some(Authority::IntegratedServer));
        assert!(lifecycle.is_opening());
        lifecycle.world_open_ready().unwrap();
        assert!(!lifecycle.is_opening());
        assert!(lifecycle.retire());
        assert!(!lifecycle.retire());
        assert!(lifecycle.epoch().is_none());

        lifecycle.begin_open(Some(Authority::IntegratedServer));
        lifecycle.world_open_ready().unwrap();
        lifecycle.begin_close();
        assert!(!lifecycle.is_opening());
        assert!(lifecycle.retire());
        assert!(!lifecycle.retire());
        assert!(lifecycle.epoch().is_none());
        assert!(lifecycle.server_tick_start(true).is_none());
    }
}
