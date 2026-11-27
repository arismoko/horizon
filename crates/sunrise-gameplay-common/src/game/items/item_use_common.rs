use crate::game::held_items::OFFHAND_SLOT;
use sunrise_core::use_animation::UseAnimation;
use sunrise_items::items::ItemStack;
use sunrise_sim::first_person_sim::Hand;

thread_local! {
    /// The items whose `use_animation` Part is being asked on this thread, innermost last: one
    /// entry per dispatch, the item's id only. A read of any stack of one of them from inside a
    /// callback (a copy with another count or components included) gets the item's own answer
    /// instead of asking that Part again, so a Part never re-enters itself; another item's Part
    /// is still asked, and nesting stays bounded by the distinct items on the call stack.
    pub(in crate::game) static ASKING_USE_ANIMATION: std::cell::RefCell<Vec<std::sync::Arc<str>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Takes its item off [`ASKING_USE_ANIMATION`] when the Part's answer is in, even if the
/// dispatch unwinds.
pub(in crate::game) struct AskingUseAnimation;

impl Drop for AskingUseAnimation {
    fn drop(&mut self) {
        ASKING_USE_ANIMATION.with_borrow_mut(|asking| asking.pop());
    }
}

#[derive(Clone, Copy)]
pub enum ItemUseState {
    Spear(sunrise_sim::spear_sim::UseTiming),
    Other(UseAnimation),
}

impl ItemUseState {
    pub fn animation(self) -> UseAnimation {
        match self {
            Self::Spear(_) => UseAnimation::Spear,
            Self::Other(animation) => animation,
        }
    }
}

/// The event's replacement counter (a cancel leaves -1), and whether a
/// mod has changed this use's countdown. This flag preserves vanilla's
/// initially zero-duration items; a mod-run-out use skips effects, but
/// still decrements on both sides.
pub fn tick_left(before: i32, remaining: i32, canceled: bool, set: bool) -> (i32, bool) {
    let left = if canceled { -1 } else { remaining };
    (left, set || canceled || remaining != before)
}

/// `useItemRemaining` of a use of `duration` ticks, `ticks` old.
pub fn ticks_left(duration: i32, ticks: u32) -> i32 {
    duration.saturating_sub_unsigned(ticks)
}

/// `LivingEntity.getUsedItemHand` of the use `using` (its slot and ticks so
/// far), none when nothing is in use.
pub fn hand_of(using: Option<(usize, u32)>) -> Option<Hand> {
    using.map(|(slot, _)| {
        if slot == OFFHAND_SLOT {
            Hand::Off
        } else {
            Hand::Main
        }
    })
}

/// `LivingEntity.useItemRemaining` of the use `using` (its slot and ticks so
/// far) of `duration` ticks, 0 when nothing is in use.
pub fn remaining_of(using: Option<(usize, u32)>, duration: i32) -> i32 {
    using.map_or(0, |(_, ticks)| ticks_left(duration, ticks))
}

/// Sets `useItemRemaining` of the use `using` by moving its `duration`, as a
/// mod's tick does; false when nothing is in use.
pub fn set_remaining(using: Option<(usize, u32)>, duration: &mut i32, remaining: i32) -> bool {
    let Some((_, ticks)) = using else {
        return false;
    };
    *duration = duration_leaving(ticks, remaining);
    true
}

/// `getUseDuration(stack) - remainingTime` of a use of `own` ticks with
/// `remaining` left, none below 0.
pub fn held(own: i32, remaining: i32) -> u32 {
    u32::try_from(i64::from(own) - i64::from(remaining)).unwrap_or(0)
}

/// The duration that leaves `remaining` ticks of a use `ticks` old, as a
/// mod's `Tick` set them.
pub fn duration_leaving(ticks: u32, remaining: i32) -> i32 {
    remaining.saturating_add_unsigned(ticks)
}

/// What a `Start` left of a use of `own` ticks: None refused (a cancel, or
/// a time below 0), else whether a
/// mod set the ticks left to none. Such a use runs out on its first tick
/// (`tick_left`), while
/// an item's own 0 (an eye of ender) is held until let go, as vanilla's.
pub fn start_of(own: i32, duration: i32, canceled: bool) -> Option<bool> {
    (!canceled && duration >= 0).then_some(duration == 0 && own != 0)
}

/// `Consumable.startConsuming`: a time of 0 or less is no use, the item is
/// consumed at once (`onConsume`).
pub fn consumes_at_once(consume_ticks: i32) -> bool {
    consume_ticks <= 0
}

/// Captured item and whether Start or Tick has overridden the remaining duration.
#[derive(Default)]
pub struct UseStart {
    pub ticks_set: bool,
    pub item: Option<ItemStack>,
}

impl UseStart {
    pub fn of(item: ItemStack) -> Self {
        Self {
            ticks_set: false,
            item: Some(item),
        }
    }
}

pub use sunrise_items::items::ItemConsumable;

/// `EntityEvent.USE_ITEM_COMPLETE`: the server finished the player's use.
pub const USE_ITEM_COMPLETE_EVENT: i8 = 9;
