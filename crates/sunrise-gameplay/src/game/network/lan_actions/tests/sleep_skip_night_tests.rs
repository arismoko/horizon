//! `sleep_skip_night`: a mod keeps the night from being skipped while enough players sleep.
use crate::game::ServerGame;
use mod_api::{Hooks, ModEntry, PlayerRef, ServerHost, SleepSkipNightEvent};
use std::sync::{Arc, Mutex};
use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;

/// Records what each call saw, and cancels the skip.
struct Keep(Arc<Mutex<Vec<(String, i32, i64, i64)>>>);
#[mod_api::__subscriptions]
impl Hooks for Keep {
    fn sleep_skip_night(
        &self,
        _: &mut dyn ServerHost,
        event: &mut SleepSkipNightEvent<'_>,
    ) -> mod_api::Flow {
        self.0.lock().unwrap().push((
            event.dimension.to_owned(),
            event.percentage,
            event.sleeping,
            event.active,
        ));
        mod_api::Flow::Cancel
    }
}

/// Wakes the only deep sleeper's timer and lets the skip go on.
struct Unsettle(PlayerRef);
#[mod_api::__subscriptions]
impl Hooks for Unsettle {
    fn sleep_skip_night(
        &self,
        host: &mut dyn ServerHost,
        _: &mut SleepSkipNightEvent<'_>,
    ) -> mod_api::Flow {
        assert!(self.0.set_sleep_timer(host, 0));
        mod_api::Flow::Continue
    }
}

/// Guest 12 in bed for 100 ticks at night, in the rain, in this level.
pub(super) fn deep_asleep(game: &mut ServerGame<'_>) {
    let level = game.common.dim.id().to_owned();
    game.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update_all(|g| {
            g.dimension = level.clone();
        });
    let player = game.player(12).unwrap();
    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
        g.bed = Some([5, 64, 5]);
    });
    assert!(player.set_sleep_timer(game, 100));
    let clock = game.level_clock_name(game.common.dim);
    game.set_command_clock_time(&clock, 13_000);
    game.common.world.weather.raining = true;
    game.common.world.weather.rain_time = 500;
    game.common.world.weather.rain_level = (1.0, 1.0);
}

#[test]
#[ignore = "daily"]
fn a_canceled_skip_keeps_the_clock_the_sleepers_and_the_rain() {
    crate::game::native_guest_turn_tests::with_guests(|game| {
        assert!(game.common.ui.rules.set(
            "players_sleeping_percentage",
            sunrise_game_data::gamerules::Value::Int(1)
        ));
        let clock = game.level_clock_name(game.common.dim);
        deep_asleep(game);
        assert!(game.enough_slept());

        // The fixture is the server thread with no player of its own: guests 12 and 13
        // count, and only guest 12 is in bed.
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![ModEntry::new("keep", Box::new(Keep(heard.clone())))].into();
        mod_platform::testing::with_mods(mods, || game.tick_sleep());
        assert_eq!(
            *heard.lock().unwrap(),
            [(game.common.dim.id().to_owned(), 1, 1, 2)]
        );
        assert_eq!(game.command_clock_time(&clock), 13_000);
        assert!(game.common.world.weather.raining);
        assert!(
            game.server.lan.as_ref().unwrap().hub.guests[0]
                .bed
                .is_some()
        );

        // With no mod canceling, the same tick skips the night as vanilla does.
        game.tick_sleep();
        assert_eq!(game.command_clock_time(&clock), 24_000);
        assert!(!game.common.world.weather.raining);
        assert!(
            game.server.lan.as_ref().unwrap().hub.guests[0]
                .bed
                .is_none()
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_listener_that_unsettles_the_sleepers_keeps_the_night() {
    crate::game::native_guest_turn_tests::with_guests(|game| {
        assert!(game.common.ui.rules.set(
            "players_sleeping_percentage",
            sunrise_game_data::gamerules::Value::Int(1)
        ));
        let clock = game.level_clock_name(game.common.dim);
        deep_asleep(game);
        assert!(game.enough_slept());

        let guest = game.player(12).unwrap();
        let mods: mod_api::Mods = vec![ModEntry::new("unsettle", Box::new(Unsettle(guest)))].into();
        mod_platform::testing::with_mods(mods, || game.tick_sleep());
        assert_eq!(game.command_clock_time(&clock), 13_000);
        assert!(game.common.world.weather.raining);
        assert!(
            game.server.lan.as_ref().unwrap().hub.guests[0]
                .bed
                .is_some()
        );
    });
}
