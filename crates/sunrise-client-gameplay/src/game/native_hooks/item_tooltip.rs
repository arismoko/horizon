/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `ItemTooltipEvent`: the whole tooltip of `stack` (`ItemStack.getTooltipLines`),
    /// once its name, the item's own lines, the components' lines, the
    /// advanced lines and the operator warning are in `lines`, each with a
    /// colour as `rgba` packs it (`u32::MAX` keeps the text's own). Add,
    /// change, reorder or remove lines. `player` and `player_entity_id` are
    /// the local player's, `display` the stack's `minecraft:tooltip_display`,
    /// `advanced` (F3+H) and `creative` (a creative tab's own slot) its
    /// `TooltipFlag`. Asked for display and mod line requests, never for a hidden
    /// tooltip, a crossbow's loaded projectile or a reentrant request. Client-side.
    pub(in crate::game) fn native_item_tooltip(
        &self,
        call: mod_api::__private::ItemTooltipCall<'_>,
    ) {
        self.fire_item_tooltip(call);
    }
}
