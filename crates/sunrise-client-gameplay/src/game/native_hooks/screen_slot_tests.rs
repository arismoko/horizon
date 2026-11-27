use super::{OpenNativeMenu, OpenNativeScreen, SlotMoves};

/// A data packet for an old window leaves the open mod screen alone.
#[test]
fn mod_screen_data_is_its_windows_only() {
    let mut s = OpenNativeScreen {
        menu: OpenNativeMenu {
            key: (0, 0, 0),
            window: 4,
            generation: 1,
            name: "gtmod:menu".into(),
            title: None,
            block: Some("gtmod:machine".into()),
            pos: None,
            mod_type: None,
            data: Vec::new(),
        },
        slot_moves: Default::default(),
        layout_revision: 0,
        screen: mod_api::__private::OwnedMenuScreen::new(
            "gtmod",
            0,
            mod_api::MenuScreen::new(mod_api::MenuScreenLayout::default()),
        ),
        shown: true,
        detached: false,
    };
    assert!(s.set_data(4, 1, 7));
    assert!(!s.set_data(3, 1, 99));
    assert_eq!(s.menu.data, [0, 7]);
    assert!(s.set_data(4, 300, 11));
    assert_eq!(s.menu.data.len(), 301);
    assert_eq!(s.menu.data[300], 11);
}

/// A slot move changes any backing slot (including one declared `None` at open)
/// to a finite position or hides it, takes effect at the commit, and counts only
/// what changed a position (a repeat of the effective position keeps the layout).
#[test]
fn slot_moves_refuse_what_the_screen_does_not_show() {
    let declared = [
        Some(mod_api::ScreenSlot {
            position: [8.0, 18.0],
            ..Default::default()
        }),
        None,
        Some(mod_api::ScreenSlot {
            position: [26.0, 18.0],
            ..Default::default()
        }),
    ];
    use mod_api::ScreenSlotRefusal::*;
    let at = |slot: usize| declared.get(slot).map(|s| s.as_ref().map(|s| s.position));
    let moves = SlotMoves::default();
    // The current place again changes nothing, not even the count.
    assert_eq!(moves.set(0, at(0), Some([8.0, 18.0])), Ok(()));
    assert_eq!(moves.count.get(), 0);
    assert_eq!(moves.set(2, at(2), Some([100.5, -4.0])), Ok(()));
    assert_eq!(moves.count.get(), 1);
    // Pending until the build commits it; the read-back sees it at once.
    assert_eq!(moves.position(2, Some([26.0, 18.0])), Some([26.0, 18.0]));
    assert_eq!(moves.latest(2, at(2)), Some([100.5, -4.0]));
    moves.commit();
    assert_eq!(moves.position(2, Some([26.0, 18.0])), Some([100.5, -4.0]));
    assert_eq!(moves.position(0, Some([8.0, 18.0])), Some([8.0, 18.0]));
    // Refusals change nothing.
    for (slot, pos, refusal) in [
        (3, Some([0.0, 0.0]), NoSuchSlot),
        (usize::MAX, None, NoSuchSlot),
        (0, Some([f32::NAN, 0.0]), BadPosition),
        (0, Some([0.0, f32::INFINITY]), BadPosition),
    ] {
        assert_eq!(
            moves.set(slot, at(slot), pos),
            Err(refusal),
            "{slot} {pos:?}"
        );
    }
    assert_eq!(moves.count.get(), 1);
    assert_eq!(moves.position(0, Some([8.0, 18.0])), Some([8.0, 18.0]));
    // Hiding a slot inactive at open is already so.
    assert_eq!(moves.set(1, at(1), None), Ok(()));
    assert_eq!(moves.count.get(), 1);
    // Activating a slot declared `None` at open works and can be hidden again (`Slot.isActive()`).
    assert_eq!(moves.set(1, at(1), Some([44.0, 18.0])), Ok(()));
    assert_eq!(moves.count.get(), 2);
    assert_eq!(moves.latest(1, at(1)), Some([44.0, 18.0]));
    assert_eq!(moves.position(1, None), None);
    moves.commit();
    assert_eq!(moves.position(1, None), Some([44.0, 18.0]));
    assert_eq!(moves.set(1, at(1), None), Ok(()));
    assert_eq!(moves.count.get(), 3);
    moves.commit();
    assert_eq!(moves.position(1, None), None);
    // Hidden, then shown again where the mod puts it.
    assert_eq!(moves.set(2, at(2), None), Ok(()));
    assert_eq!(moves.count.get(), 4);
    assert_eq!(moves.set(2, at(2), None), Ok(()));
    assert_eq!(moves.count.get(), 4);
    moves.commit();
    assert_eq!(moves.position(2, Some([26.0, 18.0])), None);
    assert_eq!(moves.set(2, at(2), Some([26.0, 18.0])), Ok(()));
    moves.commit();
    assert_eq!(moves.position(2, Some([26.0, 18.0])), Some([26.0, 18.0]));
    assert_eq!(moves.count.get(), 5);
}
