use sunrise_items::items::ItemStack;
use sunrise_sim::first_person_sim::Hand;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn native_held_item_tint(
        &self,
        entity_id: u32,
        hand: Hand,
        stack: &ItemStack,
        partial_tick: f32,
        tint: &mut Option<u32>,
    ) {
        self.fire_held_item_tint(entity_id, hand, stack, partial_tick, tint);
    }
}
