use super::*;

/// The hook points. Each does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    /// `Player.drop`'s item entity for `thrown`, at `at` with motion `v`
    /// and `thrower`: made, heard by `item_toss`, then added to the level
    /// (`ServerLevel.addFreshEntity`). Its id, or None when a mod canceled
    /// the toss or kept it out of the level.
    pub(crate) fn toss_item(
        &mut self,
        thrown: ItemStack,
        at: [f64; 3],
        v: [f64; 3],
        thrower: Option<[i32; 4]>,
    ) -> Option<u32> {
        self.push_item_entity(thrown, at, Some(v), 40, None);
        let id = self.server.next_entity;
        if let Some(item) = self.common.dropped.last_mut() {
            item.thrower = thrower;
        }
        if mod_api::__private::listened(mod_api::HookId::ItemToss)
            && let Some(item) = self.native_item_handle(id)
        {
            let player = self.native_acting_id();
            let mut canceled = false;
            self.common.native_lifecycle.held_joins.push(id);
            self.native_item_toss(player, item, &mut canceled);
            self.common.native_lifecycle.held_joins.pop();
            if canceled {
                // A canceled toss never enters the world; what was thrown is gone.
                if let Some(at) = self.common.dropped.iter().rposition(|d| d.id == id) {
                    self.remove_dropped(at);
                }
                self.forget_entity(id);
                // What the listeners spawned waited behind the toss.
                self.native_joins_flush(false, 0);
                return None;
            }
        }
        let _ = self.join_item_entity(id, at);
        self.common
            .dropped
            .iter()
            .rev()
            .any(|d| d.id == id)
            .then_some(id)
    }
}

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;
