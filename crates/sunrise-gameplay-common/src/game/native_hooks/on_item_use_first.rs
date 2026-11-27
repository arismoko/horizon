use crate::game::native_hooks::fire::FireHooks;
use mod_api::{BlockClick, InteractionResult, ItemStack};

pub trait GameHooks<'g>: FireHooks<'g> {
    fn native_on_item_use_first(
        &mut self,
        stack: &ItemStack,
        click: &BlockClick<'_>,
        answer: &mut Option<InteractionResult>,
    ) {
        FireHooks::fire_on_item_use_first(self, stack, click, answer);
    }
}

impl<'g, G: FireHooks<'g>> GameHooks<'g> for G {}
