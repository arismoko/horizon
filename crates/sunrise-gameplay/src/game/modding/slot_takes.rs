//! Server-side `menu_slot_take`: the slot has delivered its stack and the click is paused. A
//! block-backed menu lends sequential live access (`BlockEntityScope`) to its resident backing
//! entity; a blockless owned menu lends a scope with no view, whose `host()` and
//! `transaction(...)` still reach its transient slots through `ItemPlace::MenuSlot`.
use crate::game::server_side::native_hooks_server::block_entity_tick::ResidentBlockEntityAccess;
use sunrise_gameplay_common::game::api_values::TryIntoApi;
use sunrise_gameplay_common::game::transactions::After;
use sunrise_level::block_entities::Pos;
use sunrise_sim::menu::SlotTake;

impl crate::game::ServerGame<'_> {
    /// Deliver a take made by the current click (`Slot.onTake`) to the parts of mod menu type
    /// `menu`. `backing` is the block a block-backed menu is open on, `None` for a blockless
    /// owned menu. Returns `false` when a listener panicked (contained and disabled for
    /// `menu_slot_take` for the rest of the session) so an enclosing click transaction can abort.
    pub(in crate::game::server_side) fn mod_menu_slot_take(
        &mut self,
        backing: Option<Pos>,
        player: u128,
        menu: &str,
        (slot, taken, left): &SlotTake,
    ) -> bool {
        if !mod_api::__private::menu_listens(mod_api::HookId::MenuSlotTake, menu) {
            return true;
        }
        let (Ok(taken), Ok(left)) = (taken.try_into_api(), left.try_into_api()) else {
            return true;
        };
        // A block-backed take whose block entity is already gone has nothing to lend: it is not
        // heard, as before the menu moved off the block entity's part.
        let target = match backing {
            Some(pos) => match self.resident_block_entity_target(pos) {
                Some(target) => Some((pos, target)),
                None => return true,
            },
            None => None,
        };
        let failures = mod_api::runtime::isolated_failures();
        let items_changed = {
            let mut access = match target {
                Some((_, (handle, state))) => {
                    ResidentBlockEntityAccess::for_handle(self, handle, state)
                }
                None => ResidentBlockEntityAccess::blockless(self),
            };
            sunrise_gameplay_common::game::native_hooks::native_menu_slot_take(
                menu,
                mod_api::BlockEntityScope::new(&mut access),
                player,
                *slot,
                &taken,
                left.as_ref(),
            );
            access.items_changed()
        };
        if items_changed
            && let Some((pos, (handle, _))) = target
            && self.native_block_entity_write_key(handle) == Some(pos)
        {
            self.after_commit(After::Changed(pos));
        }
        mod_api::runtime::isolated_failures() == failures
    }
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
