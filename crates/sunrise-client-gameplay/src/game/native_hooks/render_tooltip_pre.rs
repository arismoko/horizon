/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `RenderTooltipEvent.Pre`: a tooltip whose components have been prepared and are about to
    /// be measured, positioned and drawn. `place` is the point it hangs from, before `positioner`
    /// adds its offset and keeps the box on screen; change it to move the tooltip. `components`
    /// are its prepared components, read-only; `font` is the font identifier used for measurement
    /// and drawing without rewrapping; `stack` is the item it describes. Set `cancel` and nothing
    /// is drawn (custom `draw` calls remain visible), and no `render_tooltip_texture` is asked.
    /// Client-side.
    pub(in crate::game) fn native_render_tooltip_pre(
        &self,
        call: mod_api::__private::RenderTooltipPreCall<'_>,
    ) {
        self.fire_render_tooltip_pre(call);
    }
}
