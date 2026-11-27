/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `GatherSkippedAttributeTooltipsEvent`: the attribute modifiers a stack's tooltip leaves
    /// out. Set `skip_all`, or push modifier ids to `skipped_ids` or `EquipmentSlotGroup` names
    /// to `skipped_groups`. Asked once before the attribute lines are built. Display and
    /// resource-query builds skip it when the stack hides the modifiers or the entire tooltip. `player` and
    /// `player_entity_id` are the local player's, or `None` for recipe book search and a stack
    /// nested in another's tooltip (a crossbow's projectile). Asked when lines are built for
    /// display, recipe book search or `ItemResource::tooltip`, except during reentrant line
    /// queries. Client-side, only while a mod listens.
    pub(in crate::game) fn native_gather_skipped_attribute_tooltips(
        &self,
        call: mod_api::__private::GatherSkippedAttributeTooltipsCall<'_>,
    ) {
        self.fire_gather_skipped_attribute_tooltips(call);
    }
}
