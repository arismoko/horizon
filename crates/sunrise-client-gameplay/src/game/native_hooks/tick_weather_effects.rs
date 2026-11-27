#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `ClientLevel.tickWeatherEffects` is about to run in level `dimension` at `game_time`,
    /// the camera at `camera` and the rain at `rain`: set `canceled` to skip it. Client-side.
    pub(in crate::game) fn native_tick_weather_effects(
        &self,
        dimension: &str,
        game_time: i64,
        camera: [f64; 3],
        rain: f32,
        canceled: &mut bool,
    ) {
        self.fire_tick_weather_effects(dimension, game_time, camera, rain, canceled);
    }
}
