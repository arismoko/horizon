use sunrise_gameplay_common::game::remote::synced_local_use;

#[test]
fn the_server_stopping_the_use_lowers_the_shield() {
    // Player.disableShield's stopUsingItem: flags 0 or 2 while the
    // off-hand shield is raised (LocalPlayer.onSyncedDataUpdated →
    // stopUsingItem). stopUsingItem clears bit 0 and leaves bit 1, so a
    // main-hand stop is 0 and an off-hand stop is 2.
    let off = sunrise_gameplay::game::shield::OFFHAND_SLOT;
    assert_eq!(synced_local_use(Some((off, 12)), 0, 3, |_| true), None);
    assert_eq!(synced_local_use(Some((3, 5)), 0, 3, |_| true), None);
    assert_eq!(synced_local_use(Some((off, 12)), 2, 3, |_| true), None);
}

#[test]
fn the_server_starting_a_use_starts_it_in_that_hand() {
    let off = sunrise_gameplay::game::shield::OFFHAND_SLOT;
    assert_eq!(synced_local_use(None, 1, 4, |_| true), Some((4, 0)));
    assert_eq!(synced_local_use(None, 3, 4, |_| true), Some((off, 0)));
    // LivingEntity.startUsingItem needs an item in the hand.
    assert_eq!(synced_local_use(None, 1, 4, |_| false), None);
}

#[test]
fn matching_flags_leave_a_running_use_alone() {
    assert_eq!(synced_local_use(Some((2, 7)), 1, 2, |_| true), Some((2, 7)));
    assert_eq!(synced_local_use(None, 0, 2, |_| true), None);
}
