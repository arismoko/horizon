use super::*;
use crate::game::commands::Target;
use sunrise_sim::entities::ItemEntity;
use sunrise_sim::join_seq::{JoinList, Joined};

/// Where a touching item entity lives during `ItemEntity.playerTouch`.
///
/// The host player's pickup pass walks a lifted `JoinList` (`items`), while
/// guest pickups walk `self.common.dropped` directly inside `as_guest` turns.
enum PickupSlot<'a> {
    Lifted {
        items: &'a mut JoinList<ItemEntity>,
        index: &'a mut usize,
    },
    Resident {
        conn: u32,
        item_id: u32,
        taken: &'a mut Vec<(u32, u32)>,
    },
}

impl crate::game::ServerGame<'_> {
    /// `item_picked_up` for the acting player touching the item entity
    /// `item`: the mods' answer, `Default` when no mod listens.
    pub(in crate::game) fn native_pickup_answer(&mut self, item: u32) -> TriState {
        let mut can_pickup = TriState::Default;
        if mod_api::__private::listened(mod_api::HookId::ItemPickedUp)
            && let Some(item) = self.native_item_handle(item)
        {
            let player = self.native_acting_id();
            self.native_item_picked_up(player, item, &mut can_pickup);
        }
        can_pickup
    }

    /// `Inventory.add` of the ground `stack` the acting player picks up: what
    /// did not fit. An empty stack inserts nothing (`Inventory.add(EMPTY) == false`).
    /// A player with infinite materials (creative) takes all of a non-empty stack.
    pub(in crate::game) fn native_pickup_insert(&mut self, stack: &ItemStack) -> Option<ItemStack> {
        if stack.count == 0 || &*stack.item == "minecraft:air" {
            return Some(stack.clone());
        }
        let rest = self.add_stack_to_inventory(stack.clone());
        rest.filter(|_| !self.common.creative)
    }

    /// Runs `notice` with the lifted `items[index]` lent back into `self.common.dropped`
    /// so mods can read and mutate the live entity handle. Preserves every
    /// in-place mutation on return, and never resurrects an item removed from
    /// `self.common.dropped`. Returns `(result, still_present_in_items, removed)`.
    fn lend_lifted_item<R>(
        &mut self,
        items: &mut JoinList<ItemEntity>,
        index: usize,
        notice: impl FnOnce(&mut Self) -> R,
    ) -> (R, bool, bool) {
        let (id, join) = (items[index].id, items[index].join_seq());
        let lent = items.extract(join).expect("in its pass");
        self.common
            .dropped
            .put_back(self.common.dropped.len(), lent);
        let result = notice(self);
        let removed = self.common.persist.removals.contains(&Target::Item(id));
        if let Some(back) = self.common.dropped.extract(join) {
            items.put_back(index, back);
            (result, true, removed)
        } else {
            (result, false, true)
        }
    }

    /// `item_picked_up_post` for the acting player, who took all or part of
    /// `original` from the item entity `item`, leaving `remaining` (`None`
    /// when all of it went in): the remainder after listener edits.
    pub(in crate::game) fn native_pickup_post(
        &mut self,
        item: u32,
        original: &ItemStack,
        mut remaining: Option<ItemStack>,
    ) -> Option<ItemStack> {
        if mod_api::__private::listened(mod_api::HookId::ItemPickedUpPost)
            && let Some(item) = self.native_item_handle(item)
        {
            let player = self.native_acting_id();
            self.native_item_picked_up_post(player, item, original, &mut remaining);
        }
        remaining
    }

    /// `ItemEntity.playerTouch` for the host's player, and each of
    /// the lifted `items` whose box meets `reach` (its low and high corners):
    /// `item_picked_up`, `Inventory.add`, `item_picked_up_post`, then the
    /// stat and criterion. An item whose notice a mod hears is lent back to the
    /// level meanwhile, so the mod reaches it, and one that removes it keeps it
    /// removed: the stat and criterion still run.
    pub(crate) fn native_player_pickups(
        &mut self,
        items: &mut JoinList<ItemEntity>,
        (min, max): ([f64; 3], [f64; 3]),
        _feet: [f64; 3],
        own: [i32; 4],
    ) {
        let mut i = 0;
        while i < items.len() {
            let b = items[i].bounds();
            let touching = b.min_position().x < max[0]
                && b.max_position().x > min[0]
                && b.min_position().y < max[1]
                && b.max_position().y > min[1]
                && b.min_position().z < max[2]
                && b.max_position().z > min[2];
            if !touching {
                i += 1;
                continue;
            }
            self.native_touch_item_entity(
                PickupSlot::Lifted {
                    items,
                    index: &mut i,
                },
                own,
            );
        }
    }

    /// `ItemEntity.playerTouch` for guest `conn` touching resident item `item_id`
    /// in `self.common.dropped`: same Pre/insert/Post/stat/criterion sequence as the
    /// host player, appending `(item_id, taken)` to `taken` when a take packet
    /// should be broadcast.
    pub(in crate::game) fn native_guest_pickup(
        &mut self,
        conn: u32,
        item_id: u32,
        player: [i32; 4],
        taken: &mut Vec<(u32, u32)>,
    ) {
        self.native_touch_item_entity(
            PickupSlot::Resident {
                conn,
                item_id,
                taken,
            },
            player,
        );
    }

    /// Unified `ItemEntity.playerTouch` sequence shared by host-player and guest
    /// pickups (matching NeoForge `ItemEntityPickupEvent.Pre` and `Post`).
    fn native_touch_item_entity(&mut self, mut slot: PickupSlot<'_>, player: [i32; 4]) {
        let listens_pre = mod_api::__private::listened(mod_api::HookId::ItemPickedUp);
        let (id, captured, stack_revision) = match &mut slot {
            PickupSlot::Lifted { items, index } => {
                let item = &items[**index];
                let id = item.id;
                if self.common.persist.removals.contains(&Target::Item(id))
                    || (!listens_pre
                        && !Self::pickup_decision(
                            item.pickup_delay,
                            item.target,
                            player,
                            TriState::Default,
                        ))
                {
                    **index += 1;
                    return;
                }
                (id, item.stack.clone(), item.stack_revision)
            }
            PickupSlot::Resident { item_id, .. } => {
                let id = *item_id;
                if self.common.persist.removals.contains(&Target::Item(id)) {
                    return;
                }
                let Some(at) = self.common.dropped.iter().position(|d| d.id == id) else {
                    return;
                };
                let item = &self.common.dropped[at];
                if !listens_pre
                    && !Self::pickup_decision(
                        item.pickup_delay,
                        item.target,
                        player,
                        TriState::Default,
                    )
                {
                    return;
                }
                (id, item.stack.clone(), item.stack_revision)
            }
        };

        let answer = if listens_pre {
            let (answer, present, removed) = match &mut slot {
                PickupSlot::Lifted { items, index } => {
                    self.lend_lifted_item(items, **index, |game| game.native_pickup_answer(id))
                }
                PickupSlot::Resident { conn, .. } => {
                    let Some(answer) = self.as_guest(*conn, |game| game.native_pickup_answer(id))
                    else {
                        return;
                    };
                    let present = self.common.dropped.iter().any(|d| d.id == id);
                    let removed =
                        !present || self.common.persist.removals.contains(&Target::Item(id));
                    (answer, present, removed)
                }
            };
            if removed {
                if let PickupSlot::Lifted { index, .. } = &mut slot
                    && present
                {
                    **index += 1;
                }
                return;
            }
            answer
        } else {
            TriState::Default
        };

        let live = match &slot {
            PickupSlot::Lifted { items, index } => &items[**index],
            PickupSlot::Resident { .. } => {
                let Some(live) = self.common.dropped.iter().find(|item| item.id == id) else {
                    return;
                };
                live
            }
        };
        if !Self::pickup_decision(live.pickup_delay, live.target, player, answer) {
            if let PickupSlot::Lifted { index, .. } = &mut slot {
                **index += 1;
            }
            return;
        }

        let stack_replaced =
            listens_pre && (live.stack_revision != stack_revision || live.stack != captured);
        let thrower = live.thrower;
        let before = captured.count;
        let kind = captured.item.clone();
        let rest = match &slot {
            PickupSlot::Lifted { .. } => self.native_pickup_insert(&captured),
            PickupSlot::Resident { conn, .. } => {
                let Some(rest) = self.as_guest(*conn, |game| game.native_pickup_insert(&captured))
                else {
                    return;
                };
                rest
            }
        };
        if matches!(&rest, Some(r) if r.count >= before) {
            if let PickupSlot::Lifted { index, .. } = &mut slot {
                **index += 1;
            }
            return;
        }

        let listens_post = mod_api::__private::listened(mod_api::HookId::ItemPickedUpPost);
        let mut post_changed = false;
        let remaining = if listens_post {
            if !stack_replaced {
                let during = rest.clone().unwrap_or_else(|| {
                    let mut empty = captured.clone();
                    empty.count = 0;
                    empty
                });
                match &mut slot {
                    PickupSlot::Lifted { items, index } => items[**index].stack = during,
                    PickupSlot::Resident { .. } => {
                        if let Some(at) = self.common.dropped.iter().position(|d| d.id == id) {
                            self.common.dropped[at].stack = during;
                        }
                    }
                }
            }
            let rest_before = rest.clone();
            let (told, present, removed) = match &mut slot {
                PickupSlot::Lifted { items, index } => {
                    self.lend_lifted_item(items, **index, |game| {
                        game.native_pickup_post(id, &captured, rest)
                    })
                }
                PickupSlot::Resident { conn, .. } => {
                    let told = self
                        .as_guest(*conn, |game| game.native_pickup_post(id, &captured, rest))
                        .unwrap_or(None);
                    let present = self.common.dropped.iter().any(|d| d.id == id);
                    let removed =
                        !present || self.common.persist.removals.contains(&Target::Item(id));
                    (told, present, removed)
                }
            };
            if removed {
                let left = told.as_ref().filter(|s| s.count > 0);
                let taken = before.saturating_sub(left.map_or(0, |s| s.count));
                if taken > 0 {
                    self.record_item_pickup(
                        &slot,
                        &kind,
                        taken,
                        left.unwrap_or(&captured),
                        thrower,
                    );
                }
                if let PickupSlot::Lifted { index, .. } = &mut slot
                    && present
                {
                    **index += 1;
                }
                return;
            }
            post_changed = told != rest_before;
            told
        } else {
            rest
        };

        let remaining = remaining.filter(|s| s.count > 0 && &*s.item != "minecraft:air");
        let Some(rem) = remaining else {
            self.record_item_pickup(&slot, &kind, before, &captured, thrower);
            let keeps_replacement = |item: &ItemEntity| {
                stack_replaced
                    && !post_changed
                    && item.stack.count > 0
                    && &*item.stack.item != "minecraft:air"
            };
            match slot {
                PickupSlot::Lifted { items, index } => {
                    if keeps_replacement(&items[*index]) {
                        *index += 1;
                    } else {
                        items.remove(*index);
                    }
                }
                PickupSlot::Resident { taken, .. } => {
                    let mut amount = before;
                    if let Some(at) = self.common.dropped.iter().position(|d| d.id == id) {
                        if keeps_replacement(&self.common.dropped[at]) {
                            amount = 0;
                        } else {
                            self.remove_dropped(at);
                        }
                    }
                    taken.push((id, amount));
                }
            }
            return;
        };

        if (!stack_replaced && !listens_post) || post_changed {
            match &mut slot {
                PickupSlot::Lifted { items, index } => items[**index].stack = rem.clone(),
                PickupSlot::Resident { .. } => {
                    if let Some(at) = self.common.dropped.iter().position(|d| d.id == id) {
                        self.common.dropped[at].stack = rem.clone();
                    }
                }
            }
        }
        let taken_count = before.saturating_sub(rem.count);
        if taken_count > 0 {
            self.record_item_pickup(&slot, &kind, taken_count, &rem, thrower);
            if let PickupSlot::Resident { taken, .. } = &mut slot {
                taken.push((id, taken_count));
            }
        }
        if let PickupSlot::Lifted { index, .. } = &mut slot {
            **index += 1;
        }
    }

    fn record_item_pickup(
        &mut self,
        slot: &PickupSlot<'_>,
        kind: &str,
        count: u32,
        picked: &ItemStack,
        thrower: Option<[i32; 4]>,
    ) {
        match slot {
            PickupSlot::Lifted { .. } => {
                self.award_stat("picked_up", kind, count as i32);
                if let Some(thrower) = thrower {
                    self.trigger_thrown_picked_up_by_player(picked, thrower);
                }
            }
            PickupSlot::Resident { conn, .. } => {
                let _ = self.as_guest(*conn, |game| {
                    game.award_stat("picked_up", kind, count as i32);
                    if let Some(thrower) = thrower {
                        game.trigger_thrown_picked_up_by_player(picked, thrower);
                    }
                });
            }
        }
    }

    /// `ItemEntity.playerTouch`'s rule for `player`: exactly zero pickup delay
    /// and no target or `player` as the target. NeoForge's `Pre` can veto this
    /// touch or override both conditions, without changing the delay.
    pub(in crate::game) fn pickup_decision(
        pickup_delay: i32,
        target: Option<[i32; 4]>,
        player: [i32; 4],
        answer: TriState,
    ) -> bool {
        answer.or(pickup_delay == 0 && target.is_none_or(|t| t == player))
    }
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;
