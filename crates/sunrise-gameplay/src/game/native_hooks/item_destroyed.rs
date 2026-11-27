use mod_api::ItemEntityRef;
use sunrise_items::items::ItemStack;

impl crate::game::ServerGame<'_> {
    /// `IItemExtension.onDestroyed` (vanilla `ItemStack.onDestroyed`): a dropped item entity
    /// holding `stack` has reached zero health from `damage_type` and is about to be discarded.
    /// `drops` starts with any vanilla container spills and returns the validated stacks to spawn.
    #[inline(never)]
    pub(in crate::game) fn native_item_destroyed(
        &mut self,
        stack: &ItemStack,
        item: ItemEntityRef,
        damage_type: &str,
        drops: &mut Vec<ItemStack>,
    ) {
        self.fire_item_destroyed(stack, item, damage_type, drops);
        drops.retain(|stack| stack.count > 0);
    }

    /// Runs `ItemPart::item_destroyed` for the dropped item at `index` inside a transaction while
    /// the entity is still live with its original intact stack (`ItemEntity.hurtServer` ->
    /// `ItemStack.onDestroyed` -> `discard()`), returning the validated drops to spawn. With no
    /// listening part on this item, spills built-in contents in place with no transaction or
    /// extra clone.
    pub(in crate::game) fn destroy_item_entity_drops(
        &mut self,
        index: usize,
        damage_type: &str,
    ) -> Vec<ItemStack> {
        let Some(entry) = self.common.dropped.get_mut(index) else {
            return Vec::new();
        };
        let items = self.common.res.clone();
        if !mod_api::__private::item_listens(mod_api::HookId::ItemDestroyed, &entry.stack.item) {
            return entry.spill_destroyed_contents(items.items());
        }
        let id = entry.id;
        let vanilla_spilled = entry.peek_destroyed_contents(items.items());
        let stack = entry.stack.clone();
        let Some(handle) = self.entity(id).and_then(|entity| self.item_entity(entity)) else {
            if let Some(entry) = self.common.dropped.get_mut(index).filter(|e| e.id == id) {
                return entry.spill_destroyed_contents(items.items());
            }
            return vanilla_spilled;
        };
        let mut drops = vanilla_spilled.clone();
        // One storage transaction around the dispatch. The hook runtime contains a listener's
        // panic itself (and disposes of its payload), so a failed listener shows here only as
        // a new isolated failure: the transaction then aborts, as it does on a refused commit.
        // Whenever the writes roll back, the listeners' drops are discarded with them and
        // vanilla's container spill is dropped instead.
        let failures = mod_api::runtime::isolated_failures();
        let settled = self.storage_transaction_settled(|game, _| {
            game.native_item_destroyed(&stack, handle, damage_type, &mut drops);
            if mod_api::runtime::isolated_failures() == failures {
                Ok(())
            } else {
                Err(())
            }
        });
        let result = match settled {
            Ok(()) => drops,
            Err(_) => {
                log::warn!(
                    "item_destroyed for {}: a listener failed or the commit was refused; its writes were rolled back",
                    stack.item
                );
                vanilla_spilled
            }
        };
        if let Some(entry) = self.common.dropped.iter_mut().find(|e| e.id == id) {
            let _ = entry.spill_destroyed_contents(items.items());
        }
        result
    }
}
use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_item_entity_refs::NativeItemEntityRefsGameExt1 as _;
