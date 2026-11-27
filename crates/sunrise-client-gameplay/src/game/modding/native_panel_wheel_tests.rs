//! The item panel pages on the mouse wheel over it (#4618): one page per
//! notch, down forward and up back, stopping at both ends; a notch off the
//! panel leaves its page alone and goes on to the screen underneath.
use super::native_panel::wheel_turn;

/// A panel 100 px wide right of a window, the screen's height.
const AREA: [f32; 4] = [200.0, 0.0, 100.0, 240.0];
const OVER: (f32, f32) = (250.0, 120.0);
const DOWN: f32 = -1.0;
const UP: f32 = 1.0;

#[test]
fn wheel_notches_over_the_panel_turn_it_and_stop_at_both_ends() {
    let pages = 3;
    let mut page = 0;
    let mut seen = Vec::new();
    for _ in 0..5 {
        page = wheel_turn(OVER, AREA, page, pages, DOWN).expect("the panel takes it");
        seen.push(page);
    }
    assert_eq!(
        seen,
        [1, 2, 2, 2, 2],
        "down pages forward and stops at the last"
    );
    seen.clear();
    for _ in 0..5 {
        page = wheel_turn(OVER, AREA, page, pages, UP).expect("the panel takes it");
        seen.push(page);
    }
    assert_eq!(
        seen,
        [1, 0, 0, 0, 0],
        "up pages back and stops at the first"
    );
    // A smooth wheel's fraction of a notch is still one page.
    assert_eq!(wheel_turn(OVER, AREA, 0, pages, -0.1), Some(1));
    // One page: the notch is the panel's, and nothing turns.
    assert_eq!(wheel_turn(OVER, AREA, 0, 1, DOWN), Some(0));
    assert_eq!(wheel_turn(OVER, AREA, 0, 1, UP), Some(0));
    // A page left past the end (the list shrank) comes back in range.
    assert_eq!(wheel_turn(OVER, AREA, 9, pages, UP), Some(1));
}

#[test]
fn a_notch_off_the_panel_leaves_its_page_and_reaches_the_screen() {
    for cursor in [(199.0, 120.0), (300.0, 120.0), (250.0, 240.0), (50.0, 50.0)] {
        for d in [DOWN, UP] {
            assert_eq!(wheel_turn(cursor, AREA, 1, 3, d), None, "{cursor:?} {d}");
        }
    }
    // The panel's edges are its own.
    assert_eq!(wheel_turn((200.0, 0.0), AREA, 1, 3, DOWN), Some(2));
}
