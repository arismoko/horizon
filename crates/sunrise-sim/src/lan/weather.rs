//! Weather packets shared by the player's level and viewed levels.

use super::hub::game_event;
use crate::world::{World, weather_flags};
use sunrise_protocol::packets::play::{self, Clientbound as C};

pub(super) type State = (bool, f32, f32);

/// `Level.isRaining` and the level's current raw interpolants. The server-wide
/// WeatherData flags only drive the interpolants; they aren't the level's state.
pub(super) fn state(world: &World, dimension: &str) -> State {
    let weather = &world.weather;
    let raining = world
        .shape
        .can_have_weather(dimension == "minecraft:the_end")
        && weather_flags(weather.levels(1.0)).0;
    (raining, weather.rain_level.1, weather.thunder_level.1)
}

/// `ServerLevel.advanceWeatherCycle` sends changed strengths first. A rain
/// transition then resets the client's rain level, so it must resend both
/// strengths even if one has not changed. NeoForge scopes the whole sequence
/// to that level. With no previous state this initializes a new client world.
pub(super) fn send_updates(
    previous: Option<State>,
    (raining, rain, thunder): State,
    mut send: impl FnMut(&C),
) {
    let mut event = |event, param| send(&C::GameEvent(play::GameEvent { event, param }));
    let (was_raining, last_rain, last_thunder) = previous.unwrap_or_default();
    let transition = raining != was_raining;
    if previous.is_some() || !transition {
        if rain != last_rain {
            event(game_event::RAIN_LEVEL, rain);
        }
        if thunder != last_thunder {
            event(game_event::THUNDER_LEVEL, thunder);
        }
    }
    if transition {
        event(
            if raining {
                game_event::START_RAINING
            } else {
                game_event::STOP_RAINING
            },
            0.0,
        );
        event(game_event::RAIN_LEVEL, rain);
        event(game_event::THUNDER_LEVEL, thunder);
    }
}
