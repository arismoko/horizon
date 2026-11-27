//! The `living_travel_moved` hook at its one firing point, `travel_gate::moved_controlled`
//! (#5074, movement phase 4): who hears it and what a listener may replace.
use mod_api::{Hooks, LivingTravelMovedEvent, LoopContext};
use std::sync::{Arc, Mutex};
use sunrise_core::geometry::Vec3;
use sunrise_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs, LoopSide};

fn context(side: LoopSide) -> sunrise_world::native_load_context::LoopContext {
    LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        Arc::new(LoadInputs::new(Default::default(), Default::default())),
        [],
    )
    .loop_context(side, [])
}

fn after() -> sunrise_sim::travel_gate::Moved {
    sunrise_sim::travel_gate::Moved {
        moved: Vec3::ZERO,
        on_ground_before: false,
        on_ground: true,
        jumping: false,
    }
}

/// A listener that records the kind it hears and proposes `velocity`.
struct Probe {
    velocity: [f64; 3],
    heard: Arc<Mutex<Vec<String>>>,
}

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn living_travel_moved(&self, _: &dyn LoopContext, event: &mut LivingTravelMovedEvent<'_>) {
        self.heard.lock().unwrap().push(event.kind.to_owned());
        *event.velocity = self.velocity;
    }
}

/// A `side = "server"` mod's `living_travel_moved` listener fires for a zombie and not for a
/// player or a player-driven mount.
#[test]
fn server_only_mod_hears_zombie_and_skips_player_and_driven_mount() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::__private::with_mod_side(
        mod_api::ModEntry::new(
            "server-only",
            Box::new(Probe {
                velocity: [7.0, 0.0, 0.0],
                heard: Arc::clone(&heard),
            }),
        ),
        mod_api::Side::Server,
    )]
    .into();
    let context = context(LoopSide::Server);
    mod_platform::testing::with_mods(mods, || {
        for (kind, controlled, edited) in [
            ("minecraft:zombie", false, true),
            ("minecraft:player", false, false),
            ("minecraft:horse", true, false),
        ] {
            let mut velocity = Vec3::new(1.0, 0.0, 0.0);
            sunrise_sim::travel_gate::moved_controlled(
                &context,
                1,
                kind,
                || sunrise_world::travel::TravelMode::Air,
                after(),
                &mut velocity,
                controlled,
            );
            assert_eq!(velocity.x == 7.0, edited, "{kind}");
        }
    });
    assert_eq!(*heard.lock().unwrap(), vec!["minecraft:zombie"]);
}

/// A non-finite proposal leaves the velocity as it was, and with no listener the mode is never
/// built.
#[test]
fn a_non_finite_proposal_is_dropped_and_no_listener_builds_nothing() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "nan",
        Box::new(Probe {
            velocity: [f64::NAN, 0.0, 0.0],
            heard: Arc::default(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        let mut velocity = Vec3::new(1.0, 2.0, 3.0);
        sunrise_sim::travel_gate::moved_controlled(
            &context(LoopSide::Client),
            1,
            "minecraft:player",
            || sunrise_world::travel::TravelMode::Lava,
            after(),
            &mut velocity,
            true,
        );
        assert_eq!(velocity, Vec3::new(1.0, 2.0, 3.0));
    });
    mod_platform::testing::with_mods(Vec::<mod_api::ModEntry>::new().into(), || {
        let mut velocity = Vec3::new(1.0, 2.0, 3.0);
        sunrise_sim::travel_gate::moved_controlled(
            &context(LoopSide::Server),
            1,
            "minecraft:zombie",
            || unreachable!("no listener, no mode"),
            after(),
            &mut velocity,
            false,
        );
        assert_eq!(velocity, Vec3::new(1.0, 2.0, 3.0));
    });
}
