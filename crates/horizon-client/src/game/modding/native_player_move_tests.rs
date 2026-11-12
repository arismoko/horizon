//! `player_move`: heard once a tick past Paper's thresholds; a cancel teleports back.
use super::native_guest_turn_tests::with_guests;
use mod_api::{Flow, Hooks, PlayerMoveEvent, ServerHost};
use std::sync::{Arc, Mutex};

/// Records each move and cancels one that goes past x = 10, as a world wall does.
struct Wall(Arc<Mutex<Vec<(u128, [f64; 3], [f64; 3], [f32; 2])>>>);

#[mod_api::__subscriptions]
impl Hooks for Wall {
    fn player_move(&self, _: &mut dyn ServerHost, event: &mut PlayerMoveEvent) -> Flow {
        self.0
            .lock()
            .unwrap()
            .push((event.player, event.from, event.to, event.to_rotation));
        if event.to[0] > 10.0 {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_cancelled_move_teleports_the_player_back() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new("wall", Box::new(Wall(heard.clone())))].into();
    mod_platform::testing::with_mods(mods, || {
        with_guests(|game| {
            let i = game.guest_index(13).unwrap();
            let set = |game: &mut super::Game<'_>, position: [f64; 3], yaw: f32| {
                game.lan.as_mut().unwrap().hub.guests.update(i, |g| {
                    g.position = position;
                    g.yaw = yaw;
                });
            };
            let take = || std::mem::take(&mut *heard.lock().unwrap());
            // The first tick only records where everyone is.
            game.tick_player_moves();
            assert!(take().is_empty());
            // Under 1/16 of a block and 10 degrees: not heard.
            set(game, [3.05, 64.0, 5.0], 5.0);
            game.tick_player_moves();
            assert!(take().is_empty());
            set(game, [5.0, 64.0, 5.0], 5.0);
            game.tick_player_moves();
            assert_eq!(take(), [(13, [3.0, 64.0, 5.0], [5.0, 64.0, 5.0], [5.0, 0.0])]);
            // A turn alone is heard too.
            set(game, [5.0, 64.0, 5.0], 40.0);
            game.tick_player_moves();
            assert_eq!(take(), [(13, [5.0, 64.0, 5.0], [5.0, 64.0, 5.0], [40.0, 0.0])]);
            // Past the wall: cancelled, and the player is sent back facing as before.
            set(game, [20.0, 64.0, 5.0], 90.0);
            game.tick_player_moves();
            assert_eq!(take().len(), 1);
            let g = &game.lan.as_ref().unwrap().hub.guests[i];
            assert_eq!(g.position, [5.0, 64.0, 5.0]);
            assert_eq!(g.yaw, 40.0);
            assert!(g.awaiting_teleport.is_some());
            // The teleport back is not heard as a move.
            game.tick_player_moves();
            assert!(take().is_empty());
        });
    });
}
