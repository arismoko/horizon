#[cfg(feature = "client")]
#[inline(always)]
pub(in crate::game) fn native_client_setup(context: &mut dyn mod_api::ClientSetupContext) {
    crate::game::native_hooks::fire::fire_client_setup(context);
}

/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// NeoForge's `FMLClientSetupEvent`: the client started. Fires once per game process,
    /// after the first client load has applied and before the title screen's first frame;
    /// never again on a reload (`client_resources_reloaded`) or a new world.
    pub fn native_client_setup(&mut self) {
        native_client_setup(self);
    }
}
