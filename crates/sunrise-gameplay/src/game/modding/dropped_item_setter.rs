use crate::game::ServerGame;
use sunrise_items::items::ItemStack;

impl ServerGame<'_> {
    /// Sets the stack in a validated item-entity slot, as a hopper's `addItem`
    /// leaves what it couldn't take (`ItemEntity.setItem`). An empty stack
    /// (count 0) discards the entity. Players see it on their next tracker update.
    pub(in crate::game) fn native_set_item_entity(&mut self, i: usize, stack: ItemStack) -> bool {
        if self.server.serving.is_none() && self.common.options.dedicated.is_none() {
            return false;
        }
        if stack.count == 0 {
            self.remove_dropped(i);
        } else {
            self.common.dropped[i].stack = stack;
        }
        true
    }
}
