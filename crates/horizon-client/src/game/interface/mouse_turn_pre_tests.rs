#![cfg(test)]
use crate::game::{native_entity_refs::tests::with_client_game, *};
use horizon_client::motion::PlayerMotion;
use horizon_core::geometry::Vec3;
use mod_api::{ClientHost, Flow, Hooks, ModEntry, MouseTurnPreEvent};
use std::sync::{Arc, Mutex};

type Deltas = Arc<Mutex<Vec<[f64; 2]>>>;
/// Hears the grabbed mouse's raw movement and cancels the turn when told to.
struct Steer(Deltas, bool);
#[mod_api::__subscriptions]
impl Hooks for Steer {
    fn mouse_turn_pre(&self, _: &mut dyn ClientHost, event: &mut MouseTurnPreEvent) -> Flow {
        self.0.lock().unwrap().push(event.delta);
        if self.1 { Flow::Cancel } else { Flow::Continue }
    }
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_mod_hears_the_raw_mouse_and_a_cancel_keeps_the_look() {
    for cancel in [true, false] {
        let deltas = Deltas::default();
        let mods = vec![ModEntry::new(
            "steer",
            Box::new(Steer(deltas.clone(), cancel)),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_client_game(|game| {
                game.player =
                    Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
                game.screen = Screen::Playing;
                game.title = None;
                game.agent_pointer = true;
                game.smooth_camera = false;
                game.settings.sensitivity = 1.0;
                game.yaw = 10.0;
                game.pitch = 0.0;
                game.mouse.add_motion(100.0, -40.0);
                game.apply_mouse_look();
                if cancel {
                    assert_eq!((game.yaw, game.pitch), (10.0, 0.0));
                } else {
                    // MouseHandler.turnPlayer: (1.0 * 0.6 + 0.2)^3 * 8 = 4.096 per pixel, then
                    // Entity.turn's 0.15 degrees per unit.
                    assert!((game.yaw - 71.44).abs() < 1e-3, "yaw {}", game.yaw);
                    assert!((game.pitch + 24.576).abs() < 1e-3, "pitch {}", game.pitch);
                }
                // A still mouse is not told.
                game.apply_mouse_look();
            });
        });
        assert_eq!(
            *deltas.lock().unwrap(),
            vec![[100.0, -40.0]],
            "cancel {cancel}"
        );
    }
}
