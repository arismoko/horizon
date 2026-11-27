//! The `living_travel` hook's contract at its one firing point,
//! `travel_gate::decide` (#2414 PR 2, design #2308): what a listener may
//! decide for one travel invocation, what is refused, and what a panic
//! leaves behind.
use mod_api::{Flow, Hooks, LivingTravelEvent, LoopContext, TravelMode};
use std::sync::{Arc, Mutex};
use sunrise_core::geometry::Vec3;
use sunrise_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs, LoopSide};

/// The server's loop context, the side the hook fires on: no resources, just
/// the side and the origin a tick's own loop context carries.
fn server_context() -> sunrise_world::native_load_context::LoopContext {
    LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        Arc::new(LoadInputs::new(Default::default(), Default::default())),
        [],
    )
    .loop_context(LoopSide::Server, [])
}

/// The same context on the client's side, whose own prediction the hook fires for too.
fn client_context() -> sunrise_world::native_load_context::LoopContext {
    LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        Arc::new(LoadInputs::new(Default::default(), Default::default())),
        [],
    )
    .loop_context(LoopSide::Client, [])
}

/// What the probe listener answers for one travel.
#[derive(Clone, Copy)]
enum Answer {
    Continue,
    Cancel,
    Replace { input: [f64; 3], velocity: [f64; 3] },
    Panic,
}

/// What the probe heard: entity, mode, and the resolved input and velocity.
type Heard = Vec<(u32, TravelMode, [f64; 3], [f64; 3])>;

/// A `side = "server"` mod's `living_travel` listener fires for a zombie and
/// not for a player or a player-driven mount.
#[test]
fn server_only_mod_hears_zombie_and_skips_player_and_driven_mount() {
    let heard = Arc::new(Mutex::new(Vec::<String>::new()));
    struct KindProbe(Arc<Mutex<Vec<String>>>);
    #[mod_api::__subscriptions]
    impl Hooks for KindProbe {
        fn living_travel(&self, _: &dyn LoopContext, event: &mut LivingTravelEvent<'_>) -> Flow {
            self.0.lock().unwrap().push(event.kind.to_owned());
            Flow::Continue
        }
    }
    let mods: mod_api::Mods = vec![mod_api::__private::with_mod_side(
        mod_api::ModEntry::new("server-only", Box::new(KindProbe(Arc::clone(&heard)))),
        mod_api::Side::Server,
    )]
    .into();
    let context = server_context();
    mod_platform::testing::with_mods(mods, || {
        let mut input = Vec3::ZERO;
        let mut velocity = Vec3::ZERO;
        assert!(sunrise_sim::travel_gate::decide(
            &context,
            1,
            "minecraft:zombie",
            || sunrise_world::travel::TravelMode::Air,
            &mut input,
            &mut velocity,
        ));
        assert!(sunrise_sim::travel_gate::decide(
            &context,
            2,
            "minecraft:player",
            || sunrise_world::travel::TravelMode::Air,
            &mut input,
            &mut velocity,
        ));
        assert!(sunrise_sim::travel_gate::decide_controlled(
            &context,
            3,
            "minecraft:horse",
            || sunrise_world::travel::TravelMode::Air,
            &mut input,
            &mut velocity,
            true,
        ));
    });
    assert_eq!(*heard.lock().unwrap(), vec!["minecraft:zombie"]);
}

/// A listener that records the travel it heard and answers `Answer`.
struct Probe {
    answer: Answer,
    heard: Arc<Mutex<Heard>>,
}

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn living_travel(&self, _: &dyn LoopContext, event: &mut LivingTravelEvent<'_>) -> Flow {
        self.heard
            .lock()
            .unwrap()
            .push((event.entity, event.mode, *event.input, *event.velocity));
        match self.answer {
            Answer::Continue => Flow::Continue,
            Answer::Cancel => Flow::Cancel,
            Answer::Replace { input, velocity } => {
                *event.input = input;
                *event.velocity = velocity;
                Flow::Continue
            }
            Answer::Panic => panic!("probe listener unwinds"),
        }
    }
}

/// One asked travel with the probe installed: whether it runs, the values
/// its branch starts from, and what the listener heard. The mode closure
/// counts its own calls, so a no-listener travel can prove it never runs.
fn decided_with(
    answer: Answer,
    context: &sunrise_world::native_load_context::LoopContext,
) -> (bool, Vec3, Vec3, Heard, usize) {
    let (log, mods) = install(answer);
    let mut input = Vec3::new(0.216, 0.0, 0.0);
    let mut velocity = Vec3::new(1.0, -0.078_4, 0.0);
    let mode_calls = Arc::new(Mutex::new(0usize));
    let counted = Arc::clone(&mode_calls);
    mod_platform::testing::with_mods(mods, || {
        let runs = sunrise_sim::travel_gate::decide(
            context,
            7,
            "minecraft:zombie",
            move || {
                *counted.lock().unwrap() += 1;
                sunrise_world::travel::TravelMode::Water
            },
            &mut input,
            &mut velocity,
        );
        assert!(runs, "the travel always runs in these tests' answers");
    });
    (
        true,
        input,
        velocity,
        std::mem::take(&mut *log.lock().unwrap()),
        *mode_calls.lock().unwrap(),
    )
}

/// The player's walk step (0.216 blocks) and its landing speed, as a travel
/// resolves them: small values, not just big ones.
fn install(answer: Answer) -> (Arc<Mutex<Heard>>, mod_api::Mods) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "probe",
        Box::new(Probe {
            answer,
            heard: Arc::clone(&log),
        }),
    )]
    .into();
    (log, mods)
}

/// A Continue listener leaves the resolved travel as it was, and hears the
/// entity, its mode, and the resolved input and velocity.
#[test]
fn continue_runs_the_resolved_travel() {
    let (_, input, velocity, heard, mode_calls) = decided_with(Answer::Continue, &server_context());
    assert_eq!(
        heard,
        vec![(
            7,
            TravelMode::Water,
            [0.216, 0.0, 0.0],
            [1.0, -0.078_4, 0.0]
        )],
        "the listener hears the resolved values"
    );
    assert_eq!(mode_calls, 1, "the mode is built for the listener");
    assert_eq!(input, Vec3::new(0.216, 0.0, 0.0));
    assert_eq!(velocity, Vec3::new(1.0, -0.078_4, 0.0));
}

/// A cancelling listener skips the invocation: the travel does not run.
#[test]
fn cancel_skips_the_invocation() {
    let (log, mods) = install(Answer::Cancel);
    let mut input = Vec3::new(0.216, 0.0, 0.0);
    let mut velocity = Vec3::new(1.0, -0.078_4, 0.0);
    mod_platform::testing::with_mods(mods, || {
        let runs = sunrise_sim::travel_gate::decide(
            &server_context(),
            7,
            "minecraft:zombie",
            || sunrise_world::travel::TravelMode::Air,
            &mut input,
            &mut velocity,
        );
        assert!(!runs, "a cancelled travel does not run");
    });
    assert_eq!(
        (input, velocity),
        (Vec3::new(0.216, 0.0, 0.0), Vec3::new(1.0, -0.078_4, 0.0)),
        "a cancelled travel leaves the resolved values untouched"
    );
    assert_eq!(log.lock().unwrap().len(), 1);
}

/// A replacement supplies the branch's starting values, including magnitudes
/// beyond the removed synthetic budgets (input > 1.0, velocity > 64.0).
#[test]
fn a_replacement_supplies_the_starting_values() {
    let (_, input, velocity, _, _) = decided_with(
        Answer::Replace {
            input: [-2.5, 1.5, 0.0],
            velocity: [80.0, -70.0, 0.0],
        },
        &server_context(),
    );
    assert_eq!(input, Vec3::new(-2.5, 1.5, 0.0));
    assert_eq!(velocity, Vec3::new(80.0, -70.0, 0.0));
}

/// Cancellation is independent of any edited input or velocity, even when a
/// listener writes a non-finite component before returning `Flow::Cancel`.
#[test]
fn cancellation_is_independent_of_non_finite_edits() {
    struct CancelWithNaN;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for CancelWithNaN {
        fn living_travel(
            &self,
            _: &dyn mod_api::LoopContext,
            event: &mut mod_api::LivingTravelEvent<'_>,
        ) -> mod_api::Flow {
            *event.input = [f64::NAN, 0.0, 0.0];
            *event.velocity = [80.0, 0.0, 0.0];
            mod_api::Flow::Cancel
        }
    }

    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "cancel-nan",
        Box::new(CancelWithNaN),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        let mut input = Vec3::new(0.216, 0.0, 0.0);
        let mut velocity = Vec3::new(1.0, -0.078_4, 0.0);
        let runs = sunrise_sim::travel_gate::decide(
            &server_context(),
            7,
            "minecraft:zombie",
            || sunrise_world::travel::TravelMode::Air,
            &mut input,
            &mut velocity,
        );
        assert!(!runs, "non-finite edits must not roll back cancellation");
        assert_eq!(
            (input, velocity),
            (Vec3::new(0.216, 0.0, 0.0), Vec3::new(1.0, -0.078_4, 0.0)),
            "a cancelled travel still leaves caller vectors untouched"
        );
    });
}

/// A listener that unwinds keeps the **resolved** values — its writes die
/// with it — and the poison skips **this hook** of its mod for the session:
/// the next travel runs without it, and neither travel panics out of the
/// gate (#2414 ruling A).
#[test]
fn a_panicking_listener_keeps_the_resolved_travel_and_is_disabled() {
    let (log, mods) = install(Answer::Panic);
    mod_platform::testing::with_mods(mods, || {
        let mut input = Vec3::new(0.216, 0.0, 0.0);
        let mut velocity = Vec3::new(1.0, -0.078_4, 0.0);
        let runs = sunrise_sim::travel_gate::decide(
            &server_context(),
            7,
            "minecraft:zombie",
            || sunrise_world::travel::TravelMode::Air,
            &mut input,
            &mut velocity,
        );
        assert!(runs, "the travel carries on past the panic");
        assert_eq!(
            (input, velocity),
            (Vec3::new(0.216, 0.0, 0.0), Vec3::new(1.0, -0.078_4, 0.0))
        );
        // The poison skips this hook of the mod: this travel hears nobody.
        let mut input = Vec3::new(2.0, 0.0, 0.0);
        let runs = sunrise_sim::travel_gate::decide(
            &server_context(),
            7,
            "minecraft:zombie",
            || sunrise_world::travel::TravelMode::Air,
            &mut input,
            &mut Vec3::new(0.0, 0.0, 0.0),
        );
        assert!(runs);
        assert_eq!(input, Vec3::new(2.0, 0.0, 0.0));
    });
    assert_eq!(
        log.lock().unwrap().len(),
        1,
        "the poisoned listener is not dispatched again"
    );
}

/// A later listener's panic cannot undo an accepted input and velocity.
#[test]
fn a_panic_after_a_replacement_keeps_the_accepted_values() {
    let mods: mod_api::Mods = vec![
        mod_api::ModEntry::new(
            "replacer",
            Box::new(Probe {
                answer: Answer::Replace {
                    input: [0.5, 0.0, 0.0],
                    velocity: [9.0, 0.0, 0.0],
                },
                heard: Arc::new(Mutex::new(Vec::new())),
            }),
        ),
        mod_api::ModEntry::new(
            "panicker",
            Box::new(Probe {
                answer: Answer::Panic,
                heard: Arc::new(Mutex::new(Vec::new())),
            }),
        ),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        let mut input = Vec3::new(0.216, 0.0, 0.0);
        let mut velocity = Vec3::new(1.0, -0.078_4, 0.0);
        let runs = sunrise_sim::travel_gate::decide(
            &server_context(),
            7,
            "minecraft:zombie",
            || sunrise_world::travel::TravelMode::Air,
            &mut input,
            &mut velocity,
        );
        assert!(runs);
        assert_eq!(
            (input, velocity),
            (Vec3::new(0.5, 0.0, 0.0), Vec3::new(9.0, 0.0, 0.0)),
        );
    });
}

/// A client's own prediction asks the same listeners as the server's travel: it hears the same
/// inputs, applies a replacement, and a cancellation skips the client's predicted branch.
#[test]
fn a_client_side_travel_asks_the_listeners_and_applies_their_edits() {
    let (_, input, velocity, heard, mode_calls) = decided_with(
        Answer::Replace {
            input: [0.5, 0.0, 0.0],
            velocity: [2.0, 0.0, 0.0],
        },
        &client_context(),
    );
    assert_eq!(
        heard,
        [(
            7,
            TravelMode::Water,
            [0.216, 0.0, 0.0],
            [1.0, -0.078_4, 0.0]
        )]
    );
    assert_eq!(mode_calls, 1);
    assert_eq!(
        (input, velocity),
        (Vec3::new(0.5, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0))
    );
    let (log, mods) = install(Answer::Cancel);
    let mut input = Vec3::new(0.216, 0.0, 0.0);
    let mut velocity = Vec3::new(1.0, 0.0, 0.0);
    let runs = mod_platform::testing::with_mods(mods, || {
        sunrise_sim::travel_gate::decide(
            &client_context(),
            7,
            "minecraft:player",
            || sunrise_world::travel::TravelMode::Air,
            &mut input,
            &mut velocity,
        )
    });
    assert!(
        !runs,
        "a client's cancelled travel skips its predicted branch"
    );
    assert_eq!(log.lock().unwrap().len(), 1);
}

/// With no listener the mode closure never runs: the resolved values stand
/// and the tick cost is one branch (#2414 ruling: no snapshot, no scan).
#[test]
fn without_listeners_the_mode_is_never_built() {
    let mut input = Vec3::new(0.216, 0.0, 0.0);
    let mut velocity = Vec3::new(1.0, -0.078_4, 0.0);
    let mode_calls = Arc::new(Mutex::new(0usize));
    let counted = Arc::clone(&mode_calls);
    mod_platform::testing::with_mods(
        Vec::<mod_api::ModEntry>::new().into_iter().collect(),
        || {
            let runs = sunrise_sim::travel_gate::decide(
                &server_context(),
                7,
                "minecraft:zombie",
                move || {
                    *counted.lock().unwrap() += 1;
                    sunrise_world::travel::TravelMode::Lava
                },
                &mut input,
                &mut velocity,
            );
            assert!(runs);
        },
    );
    assert_eq!(*mode_calls.lock().unwrap(), 0, "no listener, no mode");
    assert_eq!(
        (input, velocity),
        (Vec3::new(0.216, 0.0, 0.0), Vec3::new(1.0, -0.078_4, 0.0))
    );
}

/// A mod whose `living_travel` listener panicked keeps every other hook:
/// the poison skips this hook alone (#2414 ruling A), so its `entity_move`
/// still hears the same entity on the same session — the two hooks of one
/// travel, the decision above and the move below.
#[test]
fn a_poisoned_travel_listener_keeps_its_move_hook() {
    use mod_api::EntityMoveEvent;
    struct Both {
        travels: Arc<Mutex<usize>>,
        moves: Arc<Mutex<usize>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Both {
        fn living_travel(&self, _: &dyn LoopContext, _: &mut LivingTravelEvent<'_>) -> Flow {
            *self.travels.lock().unwrap() += 1;
            panic!("the travel listener unwinds")
        }
        fn entity_move(&self, _: &dyn LoopContext, _: &mut EntityMoveEvent<'_>) -> Flow {
            *self.moves.lock().unwrap() += 1;
            Flow::Continue
        }
    }
    let travels = Arc::new(Mutex::new(0usize));
    let moves = Arc::new(Mutex::new(0usize));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "both",
        Box::new(Both {
            travels: travels.clone(),
            moves: moves.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        let context = server_context();
        let mut input = Vec3::new(0.216, 0.0, 0.0);
        let mut velocity = Vec3::new(1.0, 0.0, 0.0);
        let runs = sunrise_sim::travel_gate::decide(
            &context,
            7,
            "minecraft:zombie",
            || sunrise_world::travel::TravelMode::Air,
            &mut input,
            &mut velocity,
        );
        assert!(runs);
        // The other hook of the same mod still runs, on the same session.
        let mut gate = sunrise_world::move_gate::MoveGate::new();
        sunrise_sim::move_gate::arm(
            &mut gate,
            &context,
            7,
            "minecraft:zombie",
            sunrise_world::entity_move::MoverType::SelfMovement,
            Vec3::new(1.0, 0.0, 0.0),
        );
        assert_eq!(*moves.lock().unwrap(), 1, "the move hook still runs");
    });
    assert_eq!(*travels.lock().unwrap(), 1);
}

/// Listeners see each earlier listener's edits directly without rollback, and a
/// listener registered with `#[listener(receive_cancelled)]` may clear a
/// cancellation so later listeners and the travel branch still run.
#[test]
fn sequential_edits_stand_and_receive_cancelled_can_clear_cancellation() {
    struct ClearCancel;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for ClearCancel {
        #[listener(receive_cancelled)]
        fn living_travel(
            &self,
            _: &dyn mod_api::LoopContext,
            event: &mut mod_api::LivingTravelEvent<'_>,
        ) -> mod_api::Flow {
            assert!(event.cancelled);
            event.cancelled = false;
            mod_api::Flow::Continue
        }
    }

    let heard = Arc::new(Mutex::new(Vec::new()));
    let probe = |name: &'static str, answer, heard: &Arc<Mutex<Heard>>| {
        mod_api::ModEntry::new(
            name,
            Box::new(Probe {
                answer,
                heard: Arc::clone(heard),
            }),
        )
    };
    let mods: mod_api::Mods = vec![
        probe(
            "edit",
            Answer::Replace {
                input: [2.0, 0.0, 0.0],
                velocity: [70.0, 0.0, 0.0],
            },
            &Arc::default(),
        ),
        probe("cancel", Answer::Cancel, &Arc::default()),
        mod_api::ModEntry::new("clear", Box::new(ClearCancel)),
        probe("later", Answer::Continue, &heard),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        let mut input = Vec3::new(0.216, 0.0, 0.0);
        let mut velocity = Vec3::new(1.0, -0.078_4, 0.0);
        let runs = sunrise_sim::travel_gate::decide(
            &server_context(),
            7,
            "minecraft:zombie",
            || sunrise_world::travel::TravelMode::Air,
            &mut input,
            &mut velocity,
        );
        assert!(runs, "clearing cancellation lets the travel branch run");
        assert_eq!(
            (input, velocity),
            (Vec3::new(2.0, 0.0, 0.0), Vec3::new(70.0, 0.0, 0.0)),
        );
    });
    assert_eq!(
        *heard.lock().unwrap(),
        vec![(7, TravelMode::Air, [2.0, 0.0, 0.0], [70.0, 0.0, 0.0])],
        "the later listener hears the preceding edits once cancellation is cleared"
    );
}
