use crate::game::ServerGame;
use crate::game::durability::Wearer;
use sunrise_items::items::ItemStack;

impl ServerGame<'_> {
    /// `damage_stack` for the stack the living entity `id` wears.
    pub(in crate::game) fn damage_living_stack(
        &mut self,
        stack: &ItemStack,
        amount: u32,
        id: u32,
    ) -> (Option<ItemStack>, Option<ItemStack>) {
        let amount = amount.min(i32::MAX as u32) as i32;
        self.damage_stack_worn(stack, amount, Wearer::Living(id))
    }
}
