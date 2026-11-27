//! Scoped replacements of the game's answers from the replaceable player-rule seams: the
//! `can_harm_player` answer `Game::player_may_harm` hands back, and the slot `inventory_add`
//! takes from `slot_with_remaining_space` and `free_slot`. The tests that check each site
//! follows a changed answer install one on their thread for one closure ([`replaced`]),
//! never process-wide.
use std::{cell::Cell, thread::LocalKey};
#[cfg(test)]
use sunrise_items::items::ItemStack;

pub(in crate::game) type Harm = fn(i32, i32) -> bool;
#[cfg(test)]
pub(in crate::game) type AddSlot =
    fn(&[Option<ItemStack>], Option<&ItemStack>, usize, &ItemStack, u32) -> Option<usize>;

thread_local! {
    pub static HARM: Cell<Option<Harm>> = const { Cell::new(None) };
    #[cfg(test)]
    pub(in crate::game) static ADD_SLOT: Cell<Option<AddSlot>> = const { Cell::new(None) };
}

/// Runs `body` with `replacement` answering for `seam` on this thread, then puts back what
/// answered before, also when `body` panics.
pub fn replaced<F: Copy + 'static, R>(
    seam: &'static LocalKey<Cell<Option<F>>>,
    replacement: F,
    body: impl FnOnce() -> R,
) -> R {
    struct Restore<F: Copy + 'static>(&'static LocalKey<Cell<Option<F>>>, Option<F>);
    impl<F: Copy + 'static> Drop for Restore<F> {
        fn drop(&mut self) {
            self.0.set(self.1);
        }
    }
    let _restore = Restore(seam, seam.replace(Some(replacement)));
    body()
}

/// The replacement's answer for whether `attacker` may hurt `victim`, when one is installed.
pub fn harm(attacker: i32, victim: i32) -> Option<bool> {
    HARM.get().map(|answer| answer(attacker, victim))
}

/// The replacement's slot for `inventory_add`, when one is installed.
#[cfg(test)]
pub(in crate::game) fn add_slot(
    items: &[Option<ItemStack>],
    offhand: Option<&ItemStack>,
    selected: usize,
    stack: &ItemStack,
    max: u32,
) -> Option<Option<usize>> {
    ADD_SLOT
        .get()
        .map(|answer| answer(items, offhand, selected, stack, max))
}
