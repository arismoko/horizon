use super::*;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::native_ticks;

/// `IItemExtension.onEntityItemUpdate`: a dropped item entity is about to
/// tick (`ItemEntity.tick`). Set `canceled` and its
/// movement and merging are skipped for this tick. Server-side.
pub(in crate::game) fn native_on_entity_item_update(
    call: mod_api::__private::OnEntityItemUpdateCall<'_>,
) {
    crate::game::native_hooks::fire::fire_on_entity_item_update(call);
}

pub(in crate::game) struct ResidentDroppedItem<'g, 'w> {
    pub(in crate::game) game: &'g mut crate::game::ServerGame<'w>,
    pub(in crate::game) subject: mod_api::ItemEntityRef,
    pub(in crate::game) index: usize,
}

pub struct ItemEntityTickAccess<'g, 'w> {
    pub game: &'g mut crate::game::ServerGame<'w>,
    pub subject: mod_api::ItemEntityRef,
    pub index: usize,
}

impl mod_api::ItemEntityServerAccess for ItemEntityTickAccess<'_, '_> {
    fn with_view(&mut self, call: &mut dyn FnMut(mod_api::ItemEntitySubjectView<'_>)) {
        let Some(index) = self
            .game
            .native_item_entity_index(self.subject, Some(self.index))
        else {
            return;
        };
        self.index = index;
        let mut data = ResidentDroppedItem {
            game: self.game,
            subject: self.subject,
            index,
        };
        call(mod_api::ItemEntitySubjectView::new(self.subject, &mut data));
    }

    fn host(&mut self) -> &mut dyn mod_api::ServerHost {
        self.game
    }

    fn with_current_transaction_scope(
        &mut self,
        callback: &mut dyn for<'loan> FnMut(
            &'loan mut dyn mod_api::ItemEntityServerAccess,
            &'loan mod_api::TransactionContext,
        ),
    ) -> Result<(), mod_api::TransactionUnavailable> {
        let frame = self
            .game
            .current_storage_frame()
            .ok_or(mod_api::TransactionUnavailable)?;
        frame.lend(|tx| callback(self, tx));
        Ok(())
    }

    fn with_transaction_scope(
        &mut self,
        callback: &mut dyn for<'loan> FnMut(
            &'loan mut dyn mod_api::ItemEntityServerAccess,
            &'loan mod_api::TransactionContext,
        ) -> bool,
    ) -> Result<bool, mod_api::TransactionScopeError> {
        let (subject, index) = (self.subject, self.index);
        self.game
            .scoped_storage(|game, tx| {
                let mut child = ItemEntityTickAccess {
                    game,
                    subject,
                    index,
                };
                callback(&mut child, tx).then_some(())
            })
            .map(|settled| settled.is_some())
    }
}

impl mod_api::ResidentItemEntity for ResidentDroppedItem<'_, '_> {
    fn position(&self) -> [f64; 3] {
        let p = self.game.dropped[self.index].position;
        [p.x, p.y, p.z]
    }
    fn velocity(&self) -> [f64; 3] {
        let v = self.game.dropped[self.index].velocity;
        [v.x, v.y, v.z]
    }
    fn with_stack(&self, call: &mut dyn for<'a> FnMut(mod_api::ItemStackView<'a>)) {
        use sunrise_gameplay_common::game::api_values::{project_stack, stack_read_metadata};
        let stack = &self.game.dropped[self.index].stack;
        let Ok(metadata) = stack_read_metadata(stack) else {
            return;
        };
        let project = |metadata| project_stack(stack, metadata).ok();
        call(mod_api::ItemStackView::resident(metadata, &project));
    }
    fn pickup_delay(&self) -> i32 {
        self.game.dropped[self.index].pickup_delay
    }
    fn age(&self) -> i32 {
        self.game.dropped[self.index].age
    }
    fn target(&self) -> Option<u128> {
        self.game.dropped[self.index]
            .target
            .map(sunrise_sim::lan::playerdata::ints_uuid)
    }
    fn owner(&self) -> Option<mod_api::EntityRef> {
        self.game.item_entity_owner(self.subject)
    }
    fn set_stack(
        &mut self,
        stack: Option<mod_api::ItemStack>,
    ) -> Result<bool, mod_api::StackWriteError> {
        use sunrise_gameplay_common::game::api_values::TryIntoEngine;
        let stack = stack.try_into_engine(self.game.res.items())?;
        self.game.journal_set_item_entity_at(self.index, stack);
        Ok(true)
    }
    fn set_pickup_delay(&mut self, ticks: i32) -> bool {
        self.game.journal_set_item_pickup_delay(self.index, ticks);
        true
    }
    fn set_target(&mut self, target: Option<u128>) -> bool {
        self.game.journal_set_item_target(
            self.index,
            target.map(sunrise_sim::lan::playerdata::uuid_ints),
        );
        true
    }
    fn set_velocity(&mut self, velocity: [f64; 3]) -> bool {
        if !velocity.iter().all(|v| v.is_finite()) {
            return false;
        }
        let (item, kept) = self.game.journal_item_fields(self.index);
        if let Some(kept) = kept {
            kept.velocity.get_or_insert(item.velocity);
        }
        item.velocity = Vec3::new(velocity[0], velocity[1], velocity[2]);
        true
    }
}

/// The hook points. Each does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    /// Asks the parts of the item entity at `index` of `dropped` whether its tick
    /// ends here. An item no part is attached to is told nothing: its stack is not
    /// converted and no scope is made.
    pub(crate) fn item_update_halts(&mut self, index: usize) -> bool {
        let item = &self.common.dropped[index];
        if !mod_api::__private::item_listens(mod_api::HookId::OnEntityItemUpdate, &item.stack.item)
        {
            return false;
        }
        let Some(epoch) = self.common.native_lifecycle.epoch() else {
            return false;
        };
        let subject = mod_api::ItemEntityRef::new(mod_api::EntityRef::new(epoch, item.id));
        let Some(index) = self.native_item_entity_index(subject, Some(index)) else {
            return false;
        };
        // Hold the entry's routing identity across callbacks, even if a part changes the stack.
        let item_id = self.common.dropped[index].stack.item.clone();
        let mut canceled = false;
        let mut access = ItemEntityTickAccess {
            game: self,
            subject,
            index,
        };
        native_on_entity_item_update(mod_api::__private::OnEntityItemUpdateCall {
            subject: mod_api::ItemEntityServerScope::new(&mut access, &item_id),
            canceled: &mut canceled,
        });
        canceled
    }
    pub(crate) fn item_update_round(&mut self) -> native_ticks::TickRound {
        let mut round = self.native_tick_round();
        let merging = self.common.is_logical_server();
        // One read says whether any item part listens; with none, no stack is looked at.
        let update_hook =
            merging && mod_api::__private::has_part_hook(mod_api::HookId::OnEntityItemUpdate);
        // When entity tick hooks are subscribed, each item hears its Pre and then its part in its
        // own turn (`tick_dropped_items`). Without tick hooks, run `on_entity_item_update` here.
        if !round.told_listening() && update_hook {
            let ticking = self.entity_ticking_area();
            let mut i = 0;
            while let Some(item) = self.common.dropped.get(i) {
                let (id, x, z) = (
                    item.id,
                    item.position.x.floor() as i32,
                    item.position.z.floor() as i32,
                );
                if !(merging && self.joined_during_entity_pass(id))
                    && ticking(x, z)
                    && self.item_update_halts(i)
                {
                    round.halt(id);
                }
                i += 1;
            }
        }
        round
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod on_entity_item_update_tests {
    use super::*;

    /// One stack of `item` at rest on the floor, a pickup delay of 20 ticks.
    pub fn rest(game: &mut crate::game::ServerGame<'_>, item: &str, x: f64, z: f64) -> u32 {
        game.spawn_item(ItemStack::new(item, 1), [x, 64.0, z], [0.0; 3], 20);
        game.server.ticked_entities_through = game.server.next_entity;
        game.dropped.last().unwrap().id
    }
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_ticks::NativeTicksGameExt1 as _;
