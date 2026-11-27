/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// The milliseconds a client tick lasts (`Minecraft.getTickTargetMillis`): `millis` comes
    /// in as the clock's, and a mod may set another.
    pub(in crate::game) fn native_tick_target_millis(&self, millis: &mut f32) {
        self.fire_tick_target_millis(millis);
    }
}
