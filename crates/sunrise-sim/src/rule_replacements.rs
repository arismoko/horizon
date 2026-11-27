//! Scoped replacements of the replaceable player-rule seams (`desired_pose`, `free_slot`,
//! `slot_with_remaining_space`, `suitable_hotbar_slot`), for the tests that check each site
//! follows a seam's answer once a mod's `[[native.mixin]]` row changes it. A replacement holds
//! on the installing thread for one closure ([`replaced`]), never process-wide.
use crate::{
    items::ItemStack,
    pose::{Pose, PoseFlags},
};
use std::{cell::Cell, thread::LocalKey};

pub(crate) type DesiredPose = fn(i32, PoseFlags) -> Pose;
pub(crate) type FreeSlot = fn(&[Option<ItemStack>]) -> Option<usize>;
pub(crate) type RemainingSpace =
    fn(&[Option<ItemStack>], Option<&ItemStack>, usize, &ItemStack, u32) -> Option<usize>;
pub(crate) type HotbarSlot = fn(&[Option<ItemStack>], usize) -> usize;

thread_local! {
    pub(crate) static DESIRED_POSE: Cell<Option<DesiredPose>> = const { Cell::new(None) };
    pub(crate) static FREE_SLOT: Cell<Option<FreeSlot>> = const { Cell::new(None) };
    pub(crate) static REMAINING_SPACE: Cell<Option<RemainingSpace>> = const { Cell::new(None) };
    pub(crate) static HOTBAR_SLOT: Cell<Option<HotbarSlot>> = const { Cell::new(None) };
}

/// Runs `body` with `replacement` answering for `seam` on this thread, then puts back what
/// answered before, also when `body` panics.
pub(crate) fn replaced<F: Copy + 'static, R>(
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

/// The replacement's answer for `desired_pose`, when one is installed.
pub(crate) fn desired_pose(player: i32, flags: PoseFlags) -> Option<Pose> {
    DESIRED_POSE.get().map(|answer| answer(player, flags))
}

/// The replacement's answer for `free_slot`, when one is installed.
pub(crate) fn free_slot(items: &[Option<ItemStack>]) -> Option<Option<usize>> {
    FREE_SLOT.get().map(|answer| answer(items))
}

/// The replacement's answer for `slot_with_remaining_space`, when one is installed.
pub(crate) fn slot_with_remaining_space(
    items: &[Option<ItemStack>],
    offhand: Option<&ItemStack>,
    selected: usize,
    stack: &ItemStack,
    max: u32,
) -> Option<Option<usize>> {
    REMAINING_SPACE
        .get()
        .map(|answer| answer(items, offhand, selected, stack, max))
}

/// The replacement's answer for `suitable_hotbar_slot`, when one is installed.
pub(crate) fn suitable_hotbar_slot(items: &[Option<ItemStack>], selected: usize) -> Option<usize> {
    HOTBAR_SLOT.get().map(|answer| answer(items, selected))
}
