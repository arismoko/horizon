//! `server_level_event`: a server mod cancels a level event before it goes out.
use crate::game::native_entity_refs::tests::with_game;
use mod_api::{Flow, Hooks, ModEntry, ServerHost, ServerLevelEventEvent};
use std::sync::{Arc, Mutex};

/// What each call heard: the level, the event, its position and data, and whether it was
/// global; cancels the events in `cancel`.
#[derive(Clone, Default)]
struct Quiet {
    cancel: Arc<Mutex<Vec<i32>>>,
    heard: Arc<Mutex<Vec<(String, i32, [i32; 3], i32, bool)>>>,
}
#[mod_api::__subscriptions]
impl Hooks for Quiet {
    fn server_level_event(
        &self,
        _: &mut dyn ServerHost,
        event: &mut ServerLevelEventEvent<'_>,
    ) -> Flow {
        self.heard.lock().unwrap().push((
            event.dimension.to_owned(),
            event.id,
            event.pos,
            event.data,
            event.global,
        ));
        if self.cancel.lock().unwrap().contains(&event.id) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

/// A block's break event (2001) goes out when the mod lets it through and not at all when
/// the mod cancels it; a global event (the wither's spawn, 1023) reaches the mod as
/// global, and a canceled one is not queued for the players.
#[test]
#[ignore = "daily"]
fn a_mod_cancels_a_level_event_before_it_goes_out() {
    let quiet = Quiet::default();
    let mods = vec![ModEntry::new("quiet", Box::new(quiet.clone()))];
    mod_platform::testing::with_mods(mods.into(), || {
        with_game(|game| {
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            let data = stone.0 as i32;
            // The level events that went out, as the event fixture records them.
            let sent = |game: &mut crate::game::ServerGame<'_>| {
                crate::game::dispense_fixture::start_recording();
                game.level_event_at(2001, [0, 64, 0], data);
                crate::game::dispense_fixture::stop_recording()
            };
            assert_eq!(
                sent(game),
                [format!("L 2001 0,64,0 {data}")],
                "the break goes out"
            );
            quiet.cancel.lock().unwrap().push(2001);
            assert!(sent(game).is_empty(), "a canceled break event goes nowhere");
            let level = game.dim.id().to_owned();
            assert_eq!(
                quiet.heard.lock().unwrap()[..2],
                [
                    (level.clone(), 2001, [0, 64, 0], stone.0 as i32, false),
                    (level.clone(), 2001, [0, 64, 0], stone.0 as i32, false),
                ]
            );

            quiet.cancel.lock().unwrap().push(1023);
            let queued = game.server.lan_global_events.len();
            game.global_level_event(1023, [5, 70, 5], 0);
            assert_eq!(
                game.server.lan_global_events.len(),
                queued,
                "nothing queued"
            );
            assert_eq!(
                quiet.heard.lock().unwrap().last(),
                Some(&(level, 1023, [5, 70, 5], 0, true))
            );
        });
    });
}
