use super::*;
use horizon_client::entities::ItemEntity;
use horizon_client::join_seq::JoinList;
use horizon_core::random::RandomSource;

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
    /// the rest stayed on the ground: `item_picked_up_partial` hears that one,
    /// and a pickup is never told to both. Nor when `item_picked_up` refused
    /// the pickup or the inventory took nothing; a forced `True` from
    /// `item_picked_up` changes none of this. A creative player always takes
    /// the whole stack. Server-side.
    pub(in crate::game) fn native_item_picked_up_post(
        &mut self,
        player: u128,
        original: &ItemStack,
        remaining: &mut Option<ItemStack>,
    ) {
        self.fire_item_picked_up_post(player, original, remaining);
    }

    /// `player` picked up part of a ground stack (`ItemEntity.playerTouch`,
    /// right after `Inventory.add` took some of it and left the rest).
    /// `original` is a copy of the stack as it lay before the pickup:
    /// changing it does nothing. `remaining` is what the item entity still
    /// holds, so it starts as the part that did not fit. Set another stack
    /// there to leave the entity holding it, or `None` to empty it: the
    /// pickup then counts `original`'s count less what is left. The amount
    /// taken is `original`'s count less `remaining`'s. A mod that removes the
    /// item entity meanwhile keeps it removed: no take animation, the stat
    /// and criteria still run. Of the two notices, this one hears a partial
    /// pickup and `item_picked_up_post` a whole one; a pickup is never told
    /// to both. Not called when `item_picked_up` refused the pickup or the
    /// inventory took nothing. A creative player always takes the whole
    /// stack. Server-side; no NeoForge counterpart.
    pub(in crate::game) fn native_item_picked_up_partial(
        &mut self,
        player: u128,
        original: &ItemStack,
        remaining: &mut Option<ItemStack>,
    ) {
        self.fire_item_picked_up_partial(player, original, remaining);
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

    /// `item_picked_up_partial` for the acting player, who took part of
    /// `original` and left `rest`: the stack the item entity then holds,
    /// `None` when a mod emptied it, `rest` when no mod listens.
    pub(in crate::game) fn native_pickup_partial(
        &mut self,
        original: &ItemStack,
        rest: ItemStack,
    ) -> Option<ItemStack> {
        let mut remaining = Some(rest);
        if mod_api::__private::listened(mod_api::HookId::ItemPickedUpPartial) {
            let player = self.native_acting_id();
            self.native_item_picked_up_partial(player, original, &mut remaining);
        }
        remaining
    }

    /// `ItemEntity.playerTouch` for the host's player, at `feet`, and each of
    /// the lifted `items` whose box meets `reach` (its low and high corners):
    /// `item_picked_up`, `Inventory.add`, the post or partial notice, then the
    /// stat, the criterion and the take animation. An item whose notice a mod
    /// hears is lent back to the level meanwhile, so the mod reaches it, and
    /// one that removes it keeps it removed: no take animation, the stat and
    /// criterion still run.
    pub(in crate::game) fn native_player_pickups(
        &mut self,
        items: &mut JoinList<ItemEntity>,
        (min, max): ([f64; 3], [f64; 3]),
        feet: [f64; 3],
        own: [i32; 4],
    ) {
        let listened = mod_api::__private::listened;
        let mut i = 0;
        while i < items.len() {
            let b = items[i].bounds();
            let touching = b.min_position().x < max[0]
                && b.max_position().x > min[0]
                && b.min_position().y < max[1]
                && b.max_position().y > min[1]
                && b.min_position().z < max[2]
                && b.max_position().z > min[2];
            // ItemEntity.playerTouch: `item_picked_up`, then vanilla's delay and target rule.
            if !touching || !self.native_pickup_allowed(&mut items[i], own) {
                i += 1;
                continue;
            }
            let before = items[i].stack.count;
            let kind = items[i].stack.item.clone();
            let thrower = items[i].thrower;
            let rows = items.len();
            // The entity holds the remainder while mods hear it, as on the guest path.
            let (left, removed) = match self.native_pickup_insert(&items[i].stack) {
                Some(rest) if rest.count >= before => (Some(rest), None),
                Some(rest) if listened(mod_api::HookId::ItemPickedUpPartial) => {
                    let during = rest.clone();
                    self.native_pickup_lent(items, i, during, |game, original| {
                        game.native_pickup_partial(original, rest)
                    })
                }
                None if listened(mod_api::HookId::ItemPickedUpPost) => {
                    let mut during = items[i].stack.clone();
                    during.count = 0;
                    self.native_pickup_lent(items, i, during, |game, original| {
                        game.native_pickup_post(original)
                    })
                }
                left => (left, None),
            };
            if let Some(original) = removed {
                self.award_stat("picked_up", &kind, before as i32);
                if let Some(thrower) = thrower {
                    self.trigger_thrown_picked_up_by_player(&original, thrower);
                }
                // Still listed until the level's sweep of departed entities, or already gone.
                i += usize::from(items.len() == rows);
                continue;
            }
            let Some(left) = left else {
                self.award_stat("picked_up", &kind, before as i32);
                if let Some(thrower) = thrower {
                    self.trigger_thrown_picked_up_by_player(&items[i].stack, thrower);
                }
                let item = items.remove(i);
                self.native_take_item(item, feet);
                continue;
            };
            let taken = before.saturating_sub(left.count);
            if taken > 0 {
                self.award_stat("picked_up", &kind, taken as i32);
                if let Some(thrower) = thrower {
                    self.trigger_thrown_picked_up_by_player(&left, thrower);
                }
                self.native_take_item(items[i].clone(), feet);
            }
            items[i].stack = left;
            i += 1;
        }
    }

    /// The take animation of `item` toward the host's player, and its sound at `feet`.
    fn native_take_item(&mut self, item: ItemEntity, feet: [f64; 3]) {
        self.add_pickup(
            crate::game::pickups::Taken::Item(Box::new(item)),
            crate::game::pickups::Collector::Player,
        );
        // handleTakeItemEntity: the random pitch of a full pickup.
        let pitch =
            ((self.world.random.next_float() - self.world.random.next_float()) * 0.7 + 1.0) * 2.0;
        self.audio
            .play("minecraft:entity.item.pickup", Some(feet), 0.2, pitch);
    }

    /// `notice` with the lifted `items[i]` lent back to the level, holding
    /// `during`, where mods reach it: the notice's answer, and the stack the
    /// item held before when a mod removed it meanwhile. An item that is still
    /// there goes back to its place in `items` holding that stack.
    fn native_pickup_lent(
        &mut self,
        items: &mut JoinList<ItemEntity>,
        i: usize,
        during: ItemStack,
        notice: impl FnOnce(&mut Self, &ItemStack) -> Option<ItemStack>,
    ) -> (Option<ItemStack>, Option<ItemStack>) {
        use crate::game::commands::Target;
        use horizon_client::join_seq::Joined;
        let join = items[i].join_seq();
        let mut lent = items.extract(join).expect("in its pass");
        let (id, original) = (lent.id, std::mem::replace(&mut lent.stack, during));
        self.dropped.put_back(self.dropped.len(), lent);
        let answer = notice(self, &original);
        let removed = self.persist.removals.contains(&Target::Item(id));
        if let Some(mut back) = self.dropped.extract(join) {
            back.stack = original.clone();
            items.put_back(i, back);
            if !removed {
                return (answer, None);
            }
        }
        (answer, Some(original))
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
