use super::*;
use horizon_client::entities::ItemEntity;

impl Game<'_> {
    /// `player` touches `count` of `item` on the ground
    /// (`ItemEntity.playerTouch`), before its pickup delay and target are
    /// checked: it is asked on every tick the two touch, even while the item
    /// can't be taken. Leave `can_pickup` at `Default` for vanilla's rule,
    /// set `True` to take it despite its delay or another player's target
    /// (a full inventory still leaves it), or `False` to leave it there: the
    /// fork sets a vanilla eligibility delay of at least 10 ticks. The hook
    /// still runs on each touch, and a later `True` overrides that delay. The last mod to set it
    /// decides. Server-side.
    pub(in crate::game) fn native_item_picked_up(
        &mut self,
        player: u128,
        item: &str,
        count: u32,
        can_pickup: &mut TriState,
    ) {
        self.fire_item_picked_up(player, item, count, can_pickup);
    }

    /// `player` picked up a whole ground stack (`ItemEntity.playerTouch`,
    /// right after `Inventory.add` took all of it). `original` is a copy of
    /// the stack as it lay before the pickup: changing it does nothing.
    /// `remaining` is what the item entity still holds, so it starts as
    /// `None` (empty). Set a stack there to leave the entity on the ground
    /// holding it: the pickup then counts only `original`'s count less its
    /// count. Neither says how many were picked up; that is `original`'s
    /// count less `remaining`'s. A mod that removes the item entity meanwhile
    /// keeps it removed: no take animation, the stat and criteria still run.
    /// Not called on a partial pickup, where some of the stack went in and
    /// the rest stayed on the ground, nor when
    /// `item_picked_up` refused the pickup or the inventory took nothing; a
    /// forced `True` from `item_picked_up` changes none of this. A creative
    /// player always takes the whole stack. Server-side.
    pub(in crate::game) fn native_item_picked_up_post(
        &mut self,
        player: u128,
        original: &ItemStack,
        remaining: &mut Option<ItemStack>,
    ) {
        self.fire_item_picked_up_post(player, original, remaining);
    }

    /// `Inventory.add` of the ground `stack` the acting player picks up: what
    /// did not fit. A player with infinite materials (creative) takes all of it.
    pub(in crate::game) fn native_pickup_insert(&mut self, stack: &ItemStack) -> Option<ItemStack> {
        let rest = self.add_stack_to_inventory(stack.clone());
        rest.filter(|_| !self.creative)
    }

    /// `item_picked_up_post` for the acting player, who took all of
    /// `original`: the stack a mod left on the item entity, `None` when none
    /// did or no mod listens.
    pub(in crate::game) fn native_pickup_post(
        &mut self,
        original: &ItemStack,
    ) -> Option<ItemStack> {
        let mut remaining = None;
        if mod_api::__private::listened(mod_api::HookId::ItemPickedUpPost) {
            let player = self.native_acting_id();
            self.native_item_picked_up_post(player, original, &mut remaining);
        }
        remaining
    }

    /// `item_picked_up` for the acting player touching the ground `stack`:
    /// the mods' answer, `Default` when no mod listens.
    pub(in crate::game) fn native_pickup_answer(&mut self, stack: &ItemStack) -> TriState {
        let mut can_pickup = TriState::Default;
        if mod_api::__private::listened(mod_api::HookId::ItemPickedUp) {
            let player = self.native_acting_id();
            self.native_item_picked_up(player, &stack.item, stack.count, &mut can_pickup);
        }
        can_pickup
    }

    /// Whether the acting player, `player`, takes the ground `item` they
    /// touch (`ItemEntity.playerTouch`).
    pub(in crate::game) fn native_pickup_allowed(
        &mut self,
        item: &mut ItemEntity,
        player: [i32; 4],
    ) -> bool {
        let answer = self.native_pickup_answer(&item.stack);
        Self::pickup_decision(&mut item.pickup_delay, item.target, player, answer)
    }

    /// `ItemEntity.playerTouch`'s rule for `player`: no pickup delay, and no
    /// target or `player` as the target, unless `answer` forces it either
    /// way. A refusal sets a vanilla eligibility delay of at least 10 ticks;
    /// later touches still dispatch the hook, and `True` overrides the delay.
    pub(in crate::game) fn pickup_decision(
        pickup_delay: &mut i64,
        target: Option<[i32; 4]>,
        player: [i32; 4],
        answer: TriState,
    ) -> bool {
        if answer == TriState::False {
            *pickup_delay = (*pickup_delay).max(10);
            return false;
        }
        answer.or(*pickup_delay == 0 && target.is_none_or(|t| t == player))
    }
}
