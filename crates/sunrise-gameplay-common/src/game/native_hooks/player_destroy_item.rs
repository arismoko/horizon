use super::*;

pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// An applied item consumption or durability break, with the complete stack
    /// from before the action and its hand (None for non-hand equipment/ammo).
    /// Each logical side reports its own action, including the local player.
    fn native_player_destroy_item(
        &mut self,
        player: u128,
        player_entity_id: i32,
        original: &ItemStack,
        hand: Option<Hand>,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_player_destroy_item(
            self,
            player,
            player_entity_id,
            original,
            hand,
        );
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
