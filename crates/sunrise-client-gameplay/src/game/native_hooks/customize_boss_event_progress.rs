#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// One boss bar is about to be drawn (`CustomizeGuiOverlayEvent.BossEventProgress`): `x`,
    /// `y` are its top-left corner and `increment` the spacing to the next bar, in GUI pixels,
    /// which a mod may change; `cancel` skips drawing it.
    pub(in crate::game) fn native_customize_boss_event_progress(
        &self,
        bar: &mod_api::ClientBossBar,
        x: &mut i32,
        y: &mut i32,
        increment: &mut i32,
        cancel: &mut bool,
    ) {
        self.fire_customize_boss_event_progress(bar, x, y, increment, cancel);
    }
}
