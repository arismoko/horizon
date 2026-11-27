/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `GuiGraphicsExtractor.componentHoverEffect`: the hovered text component's hover, about
    /// to be drawn at the cursor `x`, `y`. Set `cancel` and the game draws nothing for it.
    /// Client-side.
    pub(in crate::game) fn native_component_hover(
        &self,
        call: mod_api::__private::ComponentHoverCall<'_>,
    ) {
        self.fire_component_hover(call);
    }
}
