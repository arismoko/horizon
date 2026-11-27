/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// The hook `screen_exclusion` asks: a mod adds the rectangles it
    /// draws over `screen` (a side panel, a button column) to `areas`, in GUI
    /// pixels, `[x, y, width, height]`. NeoForge has no counterpart in the
    /// game; recipe viewers ask for it through their own API
    /// (`EmiExclusionArea`, JEI's `IGuiContainerHandler.getGuiExtraAreas`).
    /// Client-side.
    pub(in crate::game) fn native_screen_exclusion_areas(
        &self,
        screen: &ScreenView<'_>,
        areas: &mut Vec<[f32; 4]>,
    ) {
        self.fire_screen_exclusion_areas(screen, areas);
    }
}
