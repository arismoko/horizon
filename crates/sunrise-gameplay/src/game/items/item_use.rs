//! `LivingEntity.startUsingItem` and the use over time of items vanilla's
//! `Item` subclasses don't know: what `Item.getUseDuration` answers, a
//! native mod's own item counted down to its `finishUsingItem`, and the hand
//! pose of a `getUseAnimation` (`native_use_duration`, `native_use_animation`,
//! `native_living_entity_use_item_start`, `native_living_entity_use_item_finish`),
//! and each tick and the release of a use (`native_living_entity_use_item_tick`,
//! `native_on_use_tick`, `native_living_entity_use_item_stop`, `native_release_using`).
use super::held_items::OFFHAND_SLOT;
use sunrise_gameplay_common::game::item_use_common::{hand_of, held, start_of, ticks_left};
use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
use {
    sunrise_items::items::ItemInfo, sunrise_items::items::ItemStack,
    sunrise_sim::first_person_sim::Hand, sunrise_sim::mobs::spear_use,
};

/// Resolve USE_EFFECTS through the stack's component patch.
pub fn interact_vibrations(stack: &ItemStack, default: Option<bool>) -> bool {
    if stack.count == 0 {
        return false;
    }
    let Some(data) = stack.data() else {
        return default.unwrap_or(false);
    };
    if data.other("!minecraft:use_effects").is_some() {
        return false;
    }
    data.other("minecraft:use_effects").map_or_else(
        || default.unwrap_or(false),
        |tag| sunrise_sim::save::number(tag.get("interact_vibrations")).unwrap_or(1) != 0,
    )
}

impl crate::game::ServerGame<'_> {
    /// `LivingEntity.updateUsingItem` for a `mod_use` item used `ticks` so far:
    /// `completeUsingItem` once its duration ran out.
    pub fn tick_mod_use(&mut self, slot: usize, ticks: u32, stack: &ItemStack) {
        if !self.item_on_use_tick(stack, ticks) {
            return;
        }
        if let Some(ticks) = count_down(ticks, self.common.use_duration) {
            self.common.using = Some((slot, ticks));
            return;
        }
        let Some(info) = self.items().get(&stack.item).cloned() else {
            self.common.using = None;
            return;
        };
        self.complete_using_item(slot, stack, &info);
    }
    /// `ServerPlayer.completeUsingItem` of `stack` in `slot`: the client is
    /// told, `ItemStack.finishUsingItem` (a consumable is eaten, a potion
    /// drunk, a spyglass lowered, anything else given back) with its use
    /// cooldown, then `LivingEntityUseItemEvent.Finish`.
    pub fn complete_using_item(&mut self, slot: usize, stack: &ItemStack, info: &ItemInfo) {
        let hand = hand_of(Some((slot, 0))).unwrap_or(self.hand);
        // a potion is drunk (no last tick of its own: a mod's tick ran it out).
        if self.finish_drinking(slot, stack) {
            self.use_item_finished(stack);
            self.use_vibration(stack, "minecraft:item_interact_finish");
            self.stopped_using(stack, Some(self.own_entity_id()), hand, 0);
            return;
        }
        self.common.using = None;
        self.send_use_item_complete();
        self.apply_use_cooldown(stack);
        if let Some(consumable) = self.consumable(stack) {
            self.finish_consuming(info, &consumable, stack);
        } else if &*stack.item == "minecraft:spyglass" {
            self.stop_spyglass();
        }
        self.use_item_finished(stack);
        // `LivingEntity.completeUsingItem` ends with `stopUsingItem`.
        self.use_vibration(stack, "minecraft:item_interact_finish");
        self.stopped_using(stack, Some(self.own_entity_id()), hand, 0);
    }
    /// `LivingEntityUseItemEvent.Finish` after `finishUsingItem` of `stack`
    /// (no ticks left): the hand holds what a mod made the result.
    pub(in crate::game::server_side) fn use_item_finished(&mut self, stack: &ItemStack) {
        let mut result = self.common.held().cloned();
        let before = result.clone();
        let hand = self.common.hand;
        self.native_living_entity_use_item_finish(
            stack,
            Some(self.own_entity_id()),
            hand,
            0,
            &mut result,
        );
        if result != before {
            *self.common.held_mut() = result.filter(|s| s.count > 0);
        }
    }
    /// Whether a use in `slot` lost its item: switching hotbar slots only ends
    /// a main-hand use.
    pub(in crate::game::server_side) fn use_hand_changed(&self, slot: usize) -> bool {
        slot != self.common.selected && slot != OFFHAND_SLOT
    }
    /// `LivingEntity.releaseUsingItem` of the acting player, the hand of the
    /// item in use acting: the use is over before any hook runs, so a mod
    /// that releases again from one finds nothing. False when none runs.
    pub fn release_using_item(&mut self) -> bool {
        let Some(hand) = self.common.used_hand() else {
            return false;
        };
        self.with_hand(hand, |game| {
            let Some((slot, ticks)) = game.using.take() else {
                return false;
            };
            let Some(stack) = game.stack_in(slot).cloned() else {
                game.use_start.item = None;
                return true;
            };
            // `releaseUsing` only for the same item; `stopUsingItem` always,
            // from the stack the use began with.
            let (stack, hand_changed) = match game.use_start.item.take() {
                Some(item) if item.item != stack.item => (item, true),
                _ => (stack, game.use_hand_changed(slot)),
            };
            game.release_used_item(&stack, ticks, hand_changed);
            true
        })
    }
    /// The stack in the hand of mob `id`.
    pub(in crate::game::server_side) fn mob_hand_stack(
        &self,
        id: u32,
        off_hand: bool,
    ) -> Option<ItemStack> {
        use sunrise_items::equipment::{MAINHAND, OFFHAND};
        let slot = if off_hand { OFFHAND } else { MAINHAND };
        let mob = self.common.mobs.iter().find(|m| m.id == id)?;
        mob.extra.slot_stack(slot).cloned()
    }
    /// `ItemStack.causeUseVibration` for `stack` used by mob `id`.
    pub(in crate::game::server_side) fn mob_use_vibration(
        &mut self,
        id: u32,
        stack: &ItemStack,
        event: &'static str,
    ) {
        if !self.use_vibrates(stack) {
            return;
        }
        let Some(p) = self
            .common
            .mobs
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.position)
        else {
            return;
        };
        let ctx = self.mob_context(id, None);
        self.post_game_event(event, [p.x, p.y, p.z], ctx);
    }
    /// `LivingEntity.startUsingItem` of mob `id`, `native_living_entity_use_item_start`
    /// as for a player. True when the use started. A mob's use is its
    /// `using_item` counter: its goals run that, and nothing counts a use down
    /// to `finishUsingItem` for it.
    pub(in crate::game::server_side) fn start_mob_using_item(
        &mut self,
        id: u32,
        hand: Hand,
    ) -> bool {
        let off_hand = hand == Hand::Off;
        let free = |m: &sunrise_sim::mobs::Mob| {
            !spear_use::is_using_item(m) && !spear_use::counts_own_timer(m)
        };
        if !self.common.mobs.iter().any(|m| m.id == id && free(m)) {
            return false;
        }
        let Some(stack) = self.mob_hand_stack(id, off_hand) else {
            return false;
        };
        let own = self.use_duration(&stack);
        let (mut duration, mut canceled) = (own, false);
        let entity = Some(id as i32);
        self.native_living_entity_use_item_start(
            &stack,
            entity,
            hand,
            &mut duration,
            &mut canceled,
        );
        if start_of(own, duration, canceled).is_none() {
            return false;
        }
        // The hook may have changed the mob: the same checks again.
        let started = self
            .common
            .mobs
            .iter_mut()
            .find(|m| m.id == id)
            .is_some_and(|m| free(m) && spear_use::start_using_item(m, off_hand));
        if started {
            self.mob_use_vibration(id, &stack, "minecraft:item_interact_start");
        }
        started
    }
    /// `LivingEntity.getUseItemRemainingTicks` of mob `id`: the use's duration
    /// less the ticks its counter has run, 0 when it uses nothing (an admiring
    /// piglin and a drinking witch use none). None for a mob that is gone.
    pub(in crate::game::server_side) fn mob_item_use_remaining(&self, id: u32) -> Option<i32> {
        let mob = self.common.mobs.iter().find(|m| m.id == id)?;
        if !spear_use::is_using_item(mob) {
            return Some(0);
        }
        let stack = self.mob_hand_stack(id, mob.extra.using_off_hand);
        Some(stack.map_or(0, |stack| {
            ticks_left(
                self.use_duration(&stack),
                mob.extra.using_item.unsigned_abs() - 1,
            )
        }))
    }
    /// `LivingEntity.useItemRemaining` of mob `id` set to `remaining`: its use
    /// counter moves to where that many ticks are left. A mob's use never
    /// runs out, so nothing else changes. False when it uses nothing.
    pub(in crate::game::server_side) fn mob_set_item_use_remaining(
        &mut self,
        id: u32,
        remaining: i32,
    ) -> bool {
        let Some(mob) = self.common.mobs.iter().find(|m| m.id == id) else {
            return false;
        };
        if !spear_use::is_using_item(mob) {
            return false;
        }
        let Some(stack) = self.mob_hand_stack(id, mob.extra.using_off_hand) else {
            return false;
        };
        let counter = use_counter_leaving(self.use_duration(&stack), remaining);
        if let Some(mob) = self.common.mobs.iter_mut().find(|m| m.id == id) {
            mob.extra.using_item = counter;
        }
        true
    }
    /// `LivingEntity.releaseUsingItem` of mob `id`: a mod's `Stop` and
    /// `Item.releaseUsing`, the use over before them as for a player. A bow's
    /// and a trident's own release ask for a player, and a use cooldown too
    /// (`UseCooldown.apply`). False when no use runs.
    pub(in crate::game::server_side) fn release_mob_using_item(&mut self, id: u32) -> bool {
        let Some(mob) = self.common.mobs.iter_mut().find(|m| m.id == id) else {
            return false;
        };
        if !spear_use::is_using_item(mob) {
            return false;
        }
        let off_hand = mob.extra.using_off_hand;
        let ticks = mob.extra.using_item.unsigned_abs() - 1;
        spear_use::stop_using_item(mob);
        let Some(stack) = self.mob_hand_stack(id, off_hand) else {
            return true;
        };
        let remaining = ticks_left(self.use_duration(&stack), ticks);
        let hand = if off_hand { Hand::Off } else { Hand::Main };
        let entity = Some(id as i32);
        let mut canceled = false;
        self.native_living_entity_use_item_stop(&stack, entity, hand, remaining, &mut canceled);
        if !canceled {
            self.native_release_using(&stack, entity, remaining, &mut false);
        }
        self.mob_use_vibration(id, &stack, "minecraft:item_interact_finish");
        self.stopped_using(&stack, entity, hand, remaining);
        true
    }
}

/// What `releaseUsingItem` runs of a release: the item's own and a mod's
/// `releaseUsing` (not when a mod's `Stop` canceled, nor when the hand
/// changed item, which stops the use without one), and the spyglass's
/// sound (not when canceled).
pub(in crate::game) fn release_parts(canceled: bool, hand_changed: bool) -> (bool, bool) {
    (!canceled && !hand_changed, !canceled)
}

pub(in crate::game) fn bow_release_power(
    has_ammo: bool,
    charge: i32,
    canceled: bool,
) -> Option<f32> {
    if canceled || charge < 0 || !has_ammo {
        return None;
    }
    let f = charge as f32 / 20.0;
    let power = ((f * f + f * 2.0) / 3.0).min(1.0);
    (power >= 0.1).then_some(power)
}

/// The use counter of a mob (1 on the tick its use starts) that leaves
/// `remaining` ticks of a use of `own`, none past the use's whole duration.
pub(in crate::game) fn use_counter_leaving(own: i32, remaining: i32) -> i32 {
    i32::try_from(held(own, remaining).saturating_add(1)).unwrap_or(i32::MAX)
}

/// `updateUsingItem`'s `--useItemRemaining == 0` for a use `ticks` old of
/// `duration` ticks: the ticks used after this one, none on the tick it runs
/// out. A time of 0 or less, or one a mod cut below what was used, never
/// does: the use lasts until let go.
pub(in crate::game) fn count_down(ticks: u32, duration: i32) -> Option<u32> {
    let ticks = ticks + 1;
    (i64::from(duration) - i64::from(ticks) != 0).then_some(ticks)
}

use sunrise_gameplay_common::game::native_hooks::base::GameHooks as _;

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::{bow_release_power, count_down, release_parts, use_counter_leaving};
    use sunrise_gameplay_common::game::item_use_common::{
        consumes_at_once, duration_leaving, held, start_of, tick_left, ticks_left,
    };

    /// A mob's use counter is 1 on the tick it starts and a bow's use lasts
    /// 72000 ticks: 71995 left is 5 ticks in, counter 6 (what releasing
    /// reports back as 71995), and more than the whole use is no ticks in.
    #[test]
    fn a_mobs_use_counter_leaves_the_remaining_ticks() {
        for (own, remaining, counter) in [
            (72_000, 71_995, 6),
            (72_000, 72_000, 1),
            (72_000, 1, 72_000),
            (72_000, 0, 72_001),
            (32, 40, 1),
            (32, -3, 36),
            (i32::MAX, i32::MIN, i32::MAX),
            // The extremes a mod may set on a 72000 tick use, which
            // `spear_use`'s tick test runs on.
            (72_000, i32::MIN, i32::MAX),
            (72_000, i32::MAX, 1),
            (72_000, -5, 72_006),
            (i32::MIN, i32::MIN, 1),
            (i32::MIN, i32::MAX, 1),
        ] {
            let got = use_counter_leaving(own, remaining);
            assert_eq!(got, counter, "{own} {remaining}");
            if remaining <= own && got < i32::MAX {
                // Read back the way a use reads its ticks left.
                assert_eq!(ticks_left(own, got.unsigned_abs() - 1), remaining);
            }
            // Reading the ticks left back never overflows either.
            ticks_left(own, got.unsigned_abs() - 1);
        }
    }

    #[test]
    fn a_use_runs_out_on_its_last_tick() {
        // a 32 tick food: ticks 0 to 30 go on, the 32nd tick finishes it.
        assert_eq!(count_down(0, 32), Some(1));
        assert_eq!(count_down(30, 32), Some(31));
        assert_eq!(count_down(31, 32), None);
        // a one tick use finishes on its first tick.
        assert_eq!(count_down(0, 1), None);
        // no time, or one below what was used, waits for the release.
        assert_eq!(count_down(0, 0), Some(1));
        assert_eq!(count_down(10, 4), Some(11));
        assert_eq!(count_down(5, 72_000), Some(6));
    }

    #[test]
    fn a_started_use_finishes_after_its_duration() {
        // start at 0 ticks, count down each tick, finish when it runs out.
        for duration in [1, 16, 32, 40] {
            let (mut ticks, mut ticked) = (0, 0);
            while let Some(next) = count_down(ticks, duration) {
                ticks = next;
                ticked += 1;
            }
            assert_eq!(ticked + 1, duration);
        }
    }

    #[test]
    fn a_start_below_0_refuses_the_use_and_a_mods_0_runs_out() {
        // a cancel or a time below 0 refuses it.
        assert_eq!(start_of(32, 32, true), None);
        assert_eq!(start_of(32, -1, false), None);
        assert_eq!(start_of(0, -1, false), None);
        // a mod's 0 starts it, and its first tick runs it out.
        assert_eq!(start_of(32, 0, false), Some(true));
        let (left, set) = tick_left(0, 0, false, true);
        assert!(!(left > 0 || !set));
        // an item's own 0 (an eye of ender) goes on, as a changed time does.
        assert_eq!(start_of(0, 0, false), Some(false));
        let (left, set) = tick_left(0, 0, false, false);
        assert!(left > 0 || !set);
        assert_eq!(start_of(32, 64, false), Some(false));
    }

    #[test]
    fn a_tick_moves_the_end_of_a_use() {
        // a 32 tick use, 10 ticks in: 22 left.
        assert_eq!(ticks_left(32, 10), 22);
        // a mod leaves 1 tick: the next tick finishes it.
        let duration = duration_leaving(10, 1);
        assert_eq!(count_down(10, duration), None);
        // or 30 more: it goes on to tick 40.
        let duration = duration_leaving(10, 30);
        assert_eq!(duration, 40);
        assert_eq!(count_down(38, duration), Some(39));
        assert_eq!(count_down(39, duration), None);
        // left as it was, nothing moves.
        assert_eq!(duration_leaving(10, ticks_left(32, 10)), 32);
    }

    #[test]
    fn a_tick_of_no_time_left_runs_the_use_out() {
        let goes_on = |(left, set): (i32, bool)| left > 0 || !set;
        // untouched, a use goes on, even one of no time (an eye of ender).
        assert_eq!(tick_left(20, 20, false, false), (20, false));
        assert!(goes_on(tick_left(-3, -3, false, false)));
        // a mod's ticks left go on while above 0.
        assert_eq!(tick_left(20, 5, false, false), (5, true));
        assert!(goes_on(tick_left(20, 5, false, false)));
        // a cancel is -1 left, and it or 0 or less set runs the use out.
        assert_eq!(tick_left(20, 20, true, false), (-1, true));
        assert!(!goes_on(tick_left(20, 0, false, false)));
        assert!(!goes_on(tick_left(20, -1, false, false)));
        // A run-out use keeps skipping effects while its counter goes negative.
        assert!(!goes_on(tick_left(-1, -1, false, true)));
        // and goes on again if a mod gives it ticks back.
        assert!(goes_on(tick_left(-1, 10, false, true)));
    }

    #[test]
    fn a_release_runs_what_vanilla_runs() {
        // let go: the item's release and the spyglass's sound.
        assert_eq!(release_parts(false, false), (true, true));
        // a mod's Stop canceled: neither.
        assert_eq!(release_parts(true, false), (false, false));
        // another item in the hand: no release (Sunrise still lowers a spyglass).
        assert_eq!(release_parts(false, true), (false, true));
    }

    #[test]
    fn bow_release_respects_charge_cancel_and_ammo() {
        assert_eq!(bow_release_power(true, 20, false), Some(1.0));
        assert_eq!(bow_release_power(true, 10, false), Some(5.0 / 12.0));
        assert_eq!(bow_release_power(true, 0, false), None);
        assert_eq!(bow_release_power(true, -1, false), None);
        assert_eq!(bow_release_power(true, 20, true), None);
        assert_eq!(bow_release_power(false, 20, false), None);
    }

    #[test]
    fn a_moved_end_moves_the_charge() {
        // a bow held 10 ticks of 72000: 10, as vanilla's.
        assert_eq!(held(72_000, ticks_left(72_000, 10)), 10);
        // a mod's Tick leaves 71000 at tick 10: it counts as held 1000.
        assert_eq!(
            held(72_000, ticks_left(duration_leaving(10, 71_000), 10)),
            1000
        );
        // more ticks left than the item's own time: not held at all.
        assert_eq!(held(72_000, 80_000), 0);
    }

    #[test]
    fn a_release_tells_how_long_the_item_was_held() {
        // a bow-like item held 25 ticks of 72000: the time left gives the charge back.
        let left = ticks_left(72_000, 25);
        assert_eq!(left, 71_975);
        assert_eq!(72_000 - left, 25);
        // released at once.
        assert_eq!(ticks_left(72_000, 0), 72_000);
    }

    #[test]
    fn a_food_of_no_time_is_eaten_at_once() {
        assert!(consumes_at_once(0));
        assert!(consumes_at_once(-3));
        assert!(!consumes_at_once(1));
        assert!(!consumes_at_once(32));
    }
}
