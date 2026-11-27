use sunrise_core::identifier::Identifier;
use sunrise_items::items::ItemStack;

/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `RenderTooltipEvent.Texture`: the look of a tooltip box about to be drawn. Set `texture`
    /// to a tooltip style and the box draws its `tooltip/<name>_background` and `_frame` GUI
    /// sprites; left `None`, it is the solid fills in `background` and the `border_top` to
    /// `border_bottom` gradient, as ARGB. Stack is None for tooltips not associated with an item.
    pub(in crate::game) fn native_render_tooltip_texture(
        &self,
        stack: Option<&ItemStack>,
        texture: &mut Option<Identifier>,
        background: &mut u32,
        border_top: &mut u32,
        border_bottom: &mut u32,
    ) {
        self.fire_render_tooltip_texture(stack, texture, background, border_top, border_bottom);
    }
}
