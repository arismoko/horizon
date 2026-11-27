use super::Dim;

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn tick_level_weather(&mut self, dim: Dim) {
        let parameters = self.common.world.weather;
        let advance = self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::ADVANCE_WEATHER);
        let world = if dim == self.common.dim {
            &mut self.common.world
        } else if let Some(parked) = self.server.levels.0.parked_mut(dim) {
            &mut parked.world
        } else {
            return;
        };
        if !world.shape.can_have_weather(dim == Dim::End) {
            return;
        }
        if dim != self.common.dim {
            world.weather.copy_parameters_from(parameters);
        }
        if advance {
            world.weather.tick_timers(&mut world.random);
        }
        world.weather.tick_levels();
        if dim != self.common.dim {
            let parameters = world.weather;
            self.common.world.weather.copy_parameters_from(parameters);
        }
    }

    pub(in crate::game) fn set_weather_parameters(
        &mut self,
        origin_dimension: &str,
        clear_ticks: i32,
        weather_ticks: i32,
        raining: bool,
        thundering: bool,
    ) -> bool {
        let mut cancel = false;
        self.native_weather_change(
            origin_dimension,
            clear_ticks,
            weather_ticks,
            raining,
            thundering,
            &mut cancel,
        );
        if cancel {
            return false;
        }

        let weather = &mut self.common.world.weather;
        weather.clear_time = clear_ticks;
        weather.rain_time = weather_ticks;
        weather.thunder_time = weather_ticks;
        weather.raining = raining;
        weather.thundering = thundering;
        true
    }

    pub(in crate::game) fn tick_weather_effects(&mut self) {
        if self.common.flash > 0 {
            self.common.flash -= 1;
        }
        // The rain sounds come with the rain splashes (`tick_particles`).
    }
}
