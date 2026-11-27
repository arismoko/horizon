//! `framerate_limit` (version 2): a mod's answer takes the game's place, raising or lowering
//! the frame rate limit, and 0 skips the frame's render.
use crate::game::native_entity_refs::tests::with_client_game;
use mod_api::{ClientHost, FramerateLimitEvent, Hooks, ModEntry};
use std::sync::{Arc, Mutex};

/// Answers `self.0` (below 0 leaves the game's own).
struct Limit(Arc<Mutex<i32>>);
#[mod_api::__subscriptions]
impl Hooks for Limit {
    fn framerate_limit(&self, _: &dyn ClientHost, event: &mut FramerateLimitEvent) {
        let answer = *self.0.lock().unwrap();
        if answer >= 0 {
            event.limit = answer;
        }
    }
}

/// With Max Framerate at 30, a mod raises the limit to 120 past the game's own, lowers it,
/// caps an answer over 260 at unlimited and skips the frame with 0; with no answer the
/// game's own limit stays. A skipped frame's wait returns.
#[test]
#[ignore = "daily"]
fn a_mods_frame_rate_takes_the_games_place() {
    let answer = Arc::new(Mutex::new(-1));
    let mods = vec![ModEntry::new("limit", Box::new(Limit(answer.clone())))];
    mod_platform::testing::with_mods(mods.into(), || {
        with_client_game(|game| {
            game.client.settings.max_fps = 30;
            let own = game.framerate_limit();
            assert!((1..=30).contains(&own), "the game's own limit: {own}");
            let with = |game: &crate::game::ClientGame<'_>, value| {
                *answer.lock().unwrap() = value;
                game.framerate_limit()
            };
            assert_eq!(with(game, 120), 120, "raised above the game's own");
            assert_eq!(with(game, 5), 5, "lowered");
            assert_eq!(with(game, 1000), 260, "capped at unlimited");
            assert_eq!(with(game, 0), 0, "the frame is skipped");
            assert_eq!(with(game, -1), own, "no answer keeps the game's limit");
            game.limit_frame(0);
        });
    });
}
