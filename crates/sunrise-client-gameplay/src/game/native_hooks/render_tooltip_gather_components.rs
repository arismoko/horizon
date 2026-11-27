use mod_api::TooltipElement;

/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `RenderTooltipEvent.GatherComponents`: the elements of `stack`'s
    /// tooltip (`ItemStack.getTooltipLines` as text lines, plus any component
    /// supplied by the built-in bundle tooltip or the item's `tooltip_image`
    /// part), once `item_tooltip` has run and before any wrapping, factory
    /// preparation or measuring. Add, change, reorder or remove elements; each
    /// element is then prepared into a `PreparedTooltipComponent` before
    /// `render_tooltip_pre` runs. A positive `max_width` constrains wrapping
    /// in addition to the available screen width. Set `cancel` and nothing is drawn,
    /// and neither `render_tooltip_pre` nor `render_tooltip_texture` is asked. Asked each
    /// frame the tooltip shows, never for one the stack hides. Client-side.
    pub(in crate::game) fn native_render_tooltip_gather_components(
        &self,
        stack: &mod_api::ItemStack,
        screen_width: i32,
        screen_height: i32,
        elements: &mut Vec<TooltipElement>,
        max_width: &mut i32,
        cancel: &mut bool,
    ) {
        self.fire_render_tooltip_gather_components(
            stack,
            screen_width,
            screen_height,
            elements,
            max_width,
            cancel,
        );
    }
}
