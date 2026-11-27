//! Weather writes for native server mods. Parameters are shared across levels as in
//! `MinecraftServer.setWeatherParameters`; strengths are immediate interpolants of the
//! addressed level, as in `Level.setRainLevel` and `Level.setThunderLevel`.
use crate::game::ServerGame;
use mod_api::LevelRef;

impl ServerGame<'_> {
    /// Use the resident server level as the origin of the shared parameter write.
    /// Returns false for a negative time, inside `weather_change`, a stale handle or a client.
    pub(in crate::game) fn level_set_weather(
        &mut self,
        level: LevelRef,
        clear_ticks: i32,
        weather_ticks: i32,
        raining: bool,
        thundering: bool,
    ) -> bool {
        if let Err(error) = self.native_level_world_result(level) {
            let mod_id = mod_api::content::crash::current_native_mod().unwrap_or("unknown");
            log::warn!("mod {mod_id}: set_weather refused for {level:?}: {error:?}");
            return false;
        }
        self.set_weather_parameters(
            level.dimension_id(),
            clear_ticks,
            weather_ticks,
            raining,
            thundering,
        )
    }
    /// `Level.setRainLevel(rain)` and `setThunderLevel(thunder)`: both halves of each level
    /// at once, clamped to 0 to 1, so the change shows without fading in. False for a value
    /// that isn't finite, a parked level, a stale handle or a client.
    pub(in crate::game) fn level_set_weather_levels(
        &mut self,
        level: LevelRef,
        rain: f32,
        thunder: f32,
    ) -> bool {
        if rain.is_nan() || thunder.is_nan() {
            let mod_id = mod_api::content::crash::current_native_mod().unwrap_or("unknown");
            log::warn!(
                "mod {mod_id}: set_weather_levels refused NaN (rain {rain}, thunder {thunder})"
            );
            return false;
        }
        let world = if !self.is_logical_server() || !self.level_handle_current(level) {
            None
        } else if level.dimension() == self.dim.key() {
            Some(&mut self.common.world)
        } else {
            self.server
                .levels
                .0
                .parked_world_mut_of(level.dimension_id())
        };
        let Some(world) = world else {
            let mod_id = mod_api::content::crash::current_native_mod().unwrap_or("unknown");
            log::warn!(
                "mod {mod_id}: set_weather_levels refused for {level:?}: no resident server level"
            );
            return false;
        };
        world.weather.set_levels(rain, thunder);
        true
    }
}

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
