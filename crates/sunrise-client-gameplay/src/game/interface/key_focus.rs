//! Vanilla's keyboard focus navigation in Sunrise's menus
//! (`Screen.keyPressed`, `ContainerEventHandler.nextFocusPath`, 26.3): Tab
//! and Shift+Tab step the focus in tab order, the arrows move it the way they
//! point, Enter, Space and keypad Enter press the focused widget
//! (`AbstractButton.keyPressed`), and a focused slider's left and right move
//! its value once Enter (or Tab) let them (`AbstractSliderButton.keyPressed`).
//! A focused text field is the screen's focus: those keys type into it.
//!
//! The focus lands on what the controller's focus mode reaches: the targets
//! each screen registers (`widgets::target_as`, docs/controllers.md). This
//! module is the pure part: which key does what, the tab order, the slider
//! rules. `pad_menu.rs` applies it in the game (it shares the controller's
//! focus), `startup_menu.rs` on the title shown while resources load.
use sunrise_client::{
    gamepad::{self, Button, Rect},
    hud::{HudAtlas, HudBatch, rgba},
    widgets::{self, Kind, Target},
};
use winit::keyboard::KeyCode;

/// A navigation key, as `Screen.keyPressed` and the widgets read it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::game) enum Nav {
    /// Tab (forward) or Shift+Tab (`FocusNavigationEvent.TabNavigation`).
    Tab { forward: bool },
    /// An arrow (`ArrowNavigation`), as the D-pad direction it steps.
    Arrow(Button),
    /// Enter, Space or keypad Enter (`InputWithModifiers.isSelection`).
    Select,
}

/// The navigation a key press is, if any.
pub(in crate::game) fn nav_of(key: KeyCode, shift: bool) -> Option<Nav> {
    Some(match key {
        KeyCode::Tab => Nav::Tab { forward: !shift },
        KeyCode::ArrowUp => Nav::Arrow(Button::DPadUp),
        KeyCode::ArrowDown => Nav::Arrow(Button::DPadDown),
        KeyCode::ArrowLeft => Nav::Arrow(Button::DPadLeft),
        KeyCode::ArrowRight => Nav::Arrow(Button::DPadRight),
        KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => Nav::Select,
        _ => return None,
    })
}

/// What a navigation key does on the screen as it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::game) enum Act {
    /// The screen takes the key itself: a text field's caret, Enter on a
    /// list whose screen joins its selected row, nothing focused.
    Pass,
    /// Focus `targets[i]` (Tab, or an arrow from no focus).
    Focus(usize),
    /// Move the focus that way (the controller's step, which scrolls lists).
    Step(Button),
    /// Press the focused widget (a click on it).
    Press,
    /// Enter on a focused slider: whether the arrows change its value flips.
    ToggleSlider,
    /// Left (-1) or right (1) on a slider that can change: one step.
    Nudge(f32),
}

/// Where the screen is: what `act` reads besides the focus.
#[derive(Clone, Copy, Debug, Default)]
pub(in crate::game) struct Context {
    /// The focused slider's arrows change its value (`canChangeValue`).
    pub slider_edit: bool,
    /// A text field has the keyboard (a click focused it): with nothing
    /// focused, its caret takes left and right, and Space and Enter belong to
    /// it rather than to the widget the keyboard focus was left on.
    pub typing: bool,
    /// The screen joins its list's selected row on Enter itself
    /// (`WorldListEntry.keyPressed`): a focused row passes Enter on.
    pub rows_take_enter: bool,
    /// The GUI size: an arrow from no focus starts at the screen's edge.
    pub gui: (f32, f32),
}

fn horizontal(button: Button) -> bool {
    matches!(button, Button::DPadLeft | Button::DPadRight)
}

/// What `nav` does, `focus` being the focused target (the one a click
/// focused counts) among `targets`.
pub(in crate::game) fn act(
    nav: Nav,
    focus: Option<Target>,
    targets: &[Target],
    ctx: Context,
) -> Act {
    act_with_tab_groups(nav, focus, targets, ctx, |_| 0)
}

pub(in crate::game) fn act_with_tab_groups(
    nav: Nav,
    focus: Option<Target>,
    targets: &[Target],
    ctx: Context,
    group: impl Fn(&Target) -> i32,
) -> Act {
    match (nav, focus) {
        (Nav::Tab { forward }, _) => {
            grouped_tab_step(targets, focus, forward, group).map_or(Act::Pass, Act::Focus)
        }
        // EditBox.keyPressed takes left and right for its caret.
        (
            Nav::Arrow(b),
            Some(Target {
                kind: Kind::Field, ..
            }),
        ) if horizontal(b) => Act::Pass,
        (Nav::Arrow(b), None) if horizontal(b) && ctx.typing => Act::Pass,
        (
            Nav::Arrow(b),
            Some(Target {
                kind: Kind::Slider { .. },
                ..
            }),
        ) if horizontal(b) && ctx.slider_edit => {
            Act::Nudge(if b == Button::DPadLeft { -1.0 } else { 1.0 })
        }
        (Nav::Arrow(b), Some(_)) => Act::Step(b),
        // No focus: from the screen's edge behind the arrow
        // (`getBorderForArrowNavigation(direction.getOpposite())`).
        (Nav::Arrow(b), None) => {
            let rects: Vec<Rect> = targets.iter().map(|t| t.rect).collect();
            gamepad::focus_step_rect(edge(ctx.gui, b), b, &rects).map_or(Act::Pass, Act::Focus)
        }
        // A focused `EditBox` is the screen's focus (`Screen.setFocused`), so
        // Space and Enter are the box's `charTyped` (a space, say) — never a
        // press of the widget the keyboard's focus was left on from before
        // (`AbstractButton.keyPressed` only hears the focused widget).
        (Nav::Select, _) if ctx.typing => Act::Pass,
        (Nav::Select, None) => Act::Pass,
        (Nav::Select, Some(t)) => match t.kind {
            Kind::Field | Kind::Mounted { .. } => Act::Pass,
            Kind::Row if ctx.rows_take_enter => Act::Pass,
            Kind::Slider { .. } => Act::ToggleSlider,
            _ => Act::Press,
        },
    }
}

/// The strip just outside the screen on the side an arrow comes from.
fn edge((w, h): (f32, f32), button: Button) -> Rect {
    match button {
        Button::DPadUp => [0.0, h, w, 1.0],
        Button::DPadLeft => [w, 0.0, 1.0, h],
        Button::DPadRight => [-1.0, 0.0, 1.0, h],
        _ => [0.0, -1.0, w, 1.0],
    }
}

/// `Screen.keyPressed` changes focus on Tab or an arrow but returns false.
pub(in crate::game) fn moves_focus(act: Act) -> bool {
    matches!(act, Act::Focus(_) | Act::Step(_))
}

/// `AbstractSliderButton.setFocused`: the arrows change a slider's value
/// when the focus came from the mouse or Tab, not from an arrow.
pub(in crate::game) fn slider_edit_on_focus(nav: Nav) -> bool {
    matches!(nav, Nav::Tab { .. })
}

/// Tab or Shift+Tab through children of the default group, in insertion order.
pub(in crate::game) fn tab_step(
    targets: &[Target],
    from: Option<Target>,
    forward: bool,
) -> Option<usize> {
    grouped_tab_step(targets, from, forward, |_| 0)
}

/// Walk children sorted stably by tab group. Wrap as `Screen.keyPressed` does.
/// A list's rows are one stop: Tab enters at its first row, Shift+Tab at its last.
fn grouped_tab_step(
    targets: &[Target],
    from: Option<Target>,
    forward: bool,
    group: impl Fn(&Target) -> i32,
) -> Option<usize> {
    let mut order: Vec<_> = (0..targets.len()).collect();
    order.sort_by_key(|&i| group(&targets[i]));
    let mut stops: Vec<Vec<usize>> = Vec::new();
    for i in order {
        let row = targets[i].kind == Kind::Row;
        match stops.last_mut() {
            Some(stop) if row && targets[stop[0]].kind == Kind::Row => stop.push(i),
            _ => stops.push(vec![i]),
        }
    }
    if stops.is_empty() {
        return None;
    }
    let at = from.and_then(|from| {
        stops
            .iter()
            .position(|s| s.iter().any(|&i| targets[i].same_focus_target(from)))
    });
    let n = stops.len();
    let next = match at {
        Some(k) if forward => (k + 1) % n,
        Some(k) => (k + n - 1) % n,
        None if forward => 0,
        None => n - 1,
    };
    let stop = &stops[next];
    Some(if forward {
        stop[0]
    } else {
        stop[stop.len() - 1]
    })
}

/// What the last drawn frame's screen can focus: its enabled buttons and
/// every target it registered (sliders, fields, rows, slots...).
pub(in crate::game) fn drawn_targets() -> Vec<Target> {
    let mut targets: Vec<Target> = widgets::last()
        .iter()
        .filter(|w| w.enabled && !matches!(w.kind, Kind::Mounted { .. }))
        .map(|w| Target {
            rect: [w.x, w.y, w.w, w.h],
            kind: Kind::Button,
        })
        .collect();
    for t in widgets::last_targets() {
        if matches!(t.kind, Kind::Mounted { .. }) {
            continue;
        }
        // A button can be both a labelled button and a target.
        if let Some(same) = targets.iter_mut().find(|o| o.rect == t.rect) {
            same.kind = t.kind;
        } else {
            targets.push(t);
        }
    }
    targets
}

/// The targets inside the GUI (`gui`: its size in GUI pixels).
pub(in crate::game) fn on_screen(mut targets: Vec<Target>, (w, h): (f32, f32)) -> Vec<Target> {
    let (w, h) = (w + 0.5, h + 0.5);
    targets.retain(|t| {
        let [x, y, tw, th] = t.rect;
        x >= -0.5 && y >= -0.5 && x + tw <= w && y + th <= h
    });
    targets
}

/// Which targets the keyboard reaches: vanilla focuses widgets only, never
/// an inventory's slots, and the creative and recipe-book tabs aren't the
/// screen's widgets either (`inventory`).
pub(in crate::game) fn keyboard_targets(targets: Vec<Target>, inventory: bool) -> Vec<Target> {
    targets
        .into_iter()
        .filter(|t| match t.kind {
            Kind::Slot => false,
            Kind::Tab(_) => !inventory,
            _ => true,
        })
        .collect()
}

/// The focus in a newly drawn frame: the same widget, or one the screen
/// moved a little (a list scrolled); else the screen changed and the focus
/// starts again on the first widget (`Screen.setInitialFocus` after a key).
pub(in crate::game) fn follow(focus: Target, targets: &[Target]) -> Option<Target> {
    let rects: Vec<Rect> = targets.iter().map(|t| t.rect).collect();
    gamepad::refind(focus.rect, &rects, 12.0)
        .or_else(|| tab_step(targets, None, true))
        .map(|i| targets[i])
}

/// Where a click focuses a text field with its caret at the end of the
/// text, as Tab leaves it (`EditBox.setFocused` keeps the cursor, which
/// `setValue` put at the end): the box's right end.
pub(in crate::game) fn field_click(rect: Rect) -> (f32, f32) {
    let [x, y, w, h] = rect;
    (x + w - 2.0, y + h / 2.0)
}

/// Vanilla's focus look for what has no highlighted sprite: the white
/// one-pixel outline of a focused list's selection
/// (`AbstractSelectionList`) and of a focused `EditBox`. Buttons, sliders
/// and tabs show their highlighted sprite, drawn under the hidden pointer.
pub(in crate::game) fn draw_outline(atlas: &HudAtlas, batch: &mut HudBatch, focus: Target) {
    if matches!(focus.kind, Kind::Row | Kind::Field) {
        draw_white_outline(atlas, batch, focus.rect);
    }
}

pub(in crate::game) fn draw_white_outline(
    atlas: &HudAtlas,
    batch: &mut HudBatch,
    [x, y, w, h]: Rect,
) {
    let white = rgba(255, 255, 255, 255);
    batch.fill(atlas, x, y, w, 1.0, white);
    batch.fill(atlas, x, y + h - 1.0, w, 1.0, white);
    batch.fill(atlas, x, y, 1.0, h, white);
    batch.fill(atlas, x + w - 1.0, y, 1.0, h, white);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn button(x: f32, y: f32) -> Target {
        Target {
            rect: [x, y, 98.0, 20.0],
            kind: Kind::Button,
        }
    }
    fn row(y: f32) -> Target {
        Target {
            rect: [100.0, y, 220.0, 36.0],
            kind: Kind::Row,
        }
    }
    fn slider() -> Target {
        Target {
            rect: [0.0, 0.0, 150.0, 20.0],
            kind: Kind::Slider {
                value: 0.5,
                track: [4.0, 142.0],
            },
        }
    }
    fn field() -> Target {
        Target {
            rect: [0.0, 0.0, 200.0, 20.0],
            kind: Kind::Field,
        }
    }
    const CTX: Context = Context {
        slider_edit: false,
        typing: false,
        rows_take_enter: false,
        gui: (427.0, 240.0),
    };

    #[test]
    fn keys_map_to_vanillas_navigation() {
        assert_eq!(
            nav_of(KeyCode::Tab, false),
            Some(Nav::Tab { forward: true })
        );
        assert_eq!(
            nav_of(KeyCode::Tab, true),
            Some(Nav::Tab { forward: false })
        );
        assert_eq!(
            nav_of(KeyCode::ArrowUp, false),
            Some(Nav::Arrow(Button::DPadUp))
        );
        assert_eq!(
            nav_of(KeyCode::ArrowRight, true),
            Some(Nav::Arrow(Button::DPadRight))
        );
        // isSelection: Enter (40), Space (44), keypad Enter (88).
        for key in [KeyCode::Enter, KeyCode::Space, KeyCode::NumpadEnter] {
            assert_eq!(nav_of(key, false), Some(Nav::Select));
        }
        assert_eq!(nav_of(KeyCode::KeyA, false), None);
        assert_eq!(nav_of(KeyCode::Escape, false), None);
    }

    /// Tab bound to key.swapOffhand in an inventory: the focus moves and the
    /// screen still checks its hotbar keys; a press stays the widget's.
    #[test]
    fn changing_focus_is_distinct_from_activating_the_child() {
        assert!(moves_focus(Act::Focus(0)));
        assert!(moves_focus(Act::Step(Button::DPadDown)));
        assert!(!moves_focus(Act::Press));
        assert!(!moves_focus(Act::ToggleSlider));
        assert!(!moves_focus(Act::Nudge(1.0)));
    }

    #[test]
    fn tab_walks_groups_in_order_and_mounted_children_in_insertion_order() {
        let bottom = Target {
            kind: Kind::Mounted {
                id: 1,
                field: false,
            },
            ..button(10.0, 100.0)
        };
        let top = Target {
            kind: Kind::Mounted {
                id: 2,
                field: false,
            },
            ..button(10.0, 20.0)
        };
        let warning = button(240.0, 120.0);
        let targets = [bottom, warning, top];
        let group = |target: &Target| if *target == warning { -10 } else { 0 };
        let next = |from, forward| grouped_tab_step(&targets, from, forward, group);
        assert_eq!(next(None, true), Some(1));
        assert_eq!(next(Some(warning), true), Some(0));
        assert_eq!(next(Some(bottom), true), Some(2));
        assert_eq!(next(None, false), Some(2));
        assert_eq!(next(Some(top), false), Some(0));
        assert_eq!(next(Some(bottom), false), Some(1));
    }

    #[test]
    fn tab_wraps_both_ways_and_starts_at_either_end() {
        let targets = [button(0.0, 0.0), button(0.0, 30.0), button(0.0, 60.0)];
        assert_eq!(tab_step(&targets, None, true), Some(0));
        assert_eq!(tab_step(&targets, None, false), Some(2));
        assert_eq!(tab_step(&targets, Some(targets[0]), true), Some(1));
        assert_eq!(tab_step(&targets, Some(targets[2]), true), Some(0));
        assert_eq!(tab_step(&targets, Some(targets[0]), false), Some(2));
        // A target no longer there: from the start.
        assert_eq!(tab_step(&targets, Some(button(9.0, 9.0)), true), Some(0));
        assert_eq!(tab_step(&[], None, true), None);
    }

    #[test]
    fn tab_visits_coincident_native_and_mounted_children_independently() {
        let native = button(10.0, 20.0);
        let first = Target {
            kind: Kind::Mounted {
                id: 1,
                field: false,
            },
            ..native
        };
        let second = Target {
            kind: Kind::Mounted {
                id: 2,
                field: false,
            },
            ..native
        };
        let targets = [native, first, second];
        assert_eq!(tab_step(&targets, Some(native), true), Some(1));
        assert_eq!(tab_step(&targets, Some(first), true), Some(2));
        assert_eq!(tab_step(&targets, Some(second), true), Some(0));
        assert_eq!(act(Nav::Select, Some(second), &targets, CTX), Act::Pass);
    }

    #[test]
    fn a_lists_rows_are_one_tab_stop() {
        let search = field();
        let targets = [search, row(40.0), row(76.0), row(112.0), button(0.0, 200.0)];
        // Into the list on its first row, then out of it.
        assert_eq!(tab_step(&targets, Some(search), true), Some(1));
        assert_eq!(tab_step(&targets, Some(targets[2]), true), Some(4));
        // Backwards: in on the last row, out to the search box.
        assert_eq!(tab_step(&targets, Some(targets[4]), false), Some(3));
        assert_eq!(tab_step(&targets, Some(targets[1]), false), Some(0));
    }

    #[test]
    fn select_presses_buttons_and_toggles_sliders() {
        let b = button(0.0, 0.0);
        assert_eq!(act(Nav::Select, Some(b), &[b], CTX), Act::Press);
        let s = slider();
        assert_eq!(act(Nav::Select, Some(s), &[s], CTX), Act::ToggleSlider);
        // Nothing focused: the screen's own Enter (a world list joins).
        assert_eq!(act(Nav::Select, None, &[b], CTX), Act::Pass);
        let f = field();
        assert_eq!(act(Nav::Select, Some(f), &[f], CTX), Act::Pass);
        let r = row(40.0);
        assert_eq!(act(Nav::Select, Some(r), &[r], CTX), Act::Press);
        let joins = Context {
            rows_take_enter: true,
            ..CTX
        };
        assert_eq!(act(Nav::Select, Some(r), &[r], joins), Act::Pass);
    }

    /// A focused text field is the screen's focus: Space types a space into
    /// it, and neither it nor Enter presses the widget the keyboard's focus
    /// was left on (the Create World tab a keyboard player tabbed to, say).
    #[test]
    fn a_focused_text_field_keeps_space_and_enter_for_typing() {
        let b = button(0.0, 40.0);
        let f = field();
        let targets = [f, b];
        let typing = Context {
            typing: true,
            ..CTX
        };
        assert_eq!(act(Nav::Select, Some(b), &targets, typing), Act::Pass);
        assert_eq!(act(Nav::Select, Some(f), &targets, typing), Act::Pass);
        // The same widget still takes them when no field has the focus.
        assert_eq!(act(Nav::Select, Some(b), &targets, CTX), Act::Press);
    }

    /// AbstractSliderButton.keyPressed and setFocused.
    #[test]
    fn slider_arrows_change_the_value_only_once_allowed() {
        let s = slider();
        let left = Nav::Arrow(Button::DPadLeft);
        let right = Nav::Arrow(Button::DPadRight);
        assert_eq!(
            act(left, Some(s), &[s], CTX),
            Act::Step(Button::DPadLeft),
            "focused by an arrow: left moves the focus"
        );
        let edit = Context {
            slider_edit: true,
            ..CTX
        };
        assert_eq!(act(left, Some(s), &[s], edit), Act::Nudge(-1.0));
        assert_eq!(act(right, Some(s), &[s], edit), Act::Nudge(1.0));
        // Up and down always move the focus.
        assert_eq!(
            act(Nav::Arrow(Button::DPadDown), Some(s), &[s], edit),
            Act::Step(Button::DPadDown)
        );
        assert!(slider_edit_on_focus(Nav::Tab { forward: true }));
        assert!(slider_edit_on_focus(Nav::Tab { forward: false }));
        assert!(!slider_edit_on_focus(Nav::Arrow(Button::DPadDown)));
    }

    /// EditBox.keyPressed consumes left and right, not Tab, up or down.
    #[test]
    fn a_text_field_keeps_left_and_right() {
        let f = field();
        let b = button(0.0, 40.0);
        let targets = [f, b];
        for b in [Button::DPadLeft, Button::DPadRight] {
            assert_eq!(act(Nav::Arrow(b), Some(f), &targets, CTX), Act::Pass);
        }
        assert_eq!(
            act(Nav::Arrow(Button::DPadDown), Some(f), &targets, CTX),
            Act::Step(Button::DPadDown)
        );
        assert_eq!(
            act(Nav::Tab { forward: true }, Some(f), &targets, CTX),
            Act::Focus(1)
        );
        // A field a click focused (no keyboard focus) keeps them too.
        let typing = Context {
            typing: true,
            ..CTX
        };
        assert_eq!(
            act(Nav::Arrow(Button::DPadLeft), None, &targets, typing),
            Act::Pass
        );
    }

    #[test]
    fn an_arrow_from_no_focus_starts_at_the_screens_edge() {
        let targets = [
            button(164.0, 60.0),
            button(164.0, 84.0),
            button(10.0, 200.0),
        ];
        let down = act(Nav::Arrow(Button::DPadDown), None, &targets, CTX);
        assert_eq!(down, Act::Focus(0));
        let up = act(Nav::Arrow(Button::DPadUp), None, &targets, CTX);
        assert_eq!(up, Act::Focus(2));
        let right = act(Nav::Arrow(Button::DPadRight), None, &targets, CTX);
        assert_eq!(right, Act::Focus(2));
    }

    #[test]
    fn the_keyboard_skips_slots_and_inventory_tabs() {
        let slot = Target {
            rect: [0.0, 0.0, 16.0, 16.0],
            kind: Kind::Slot,
        };
        let tab = Target {
            rect: [0.0, 0.0, 26.0, 32.0],
            kind: Kind::Tab(false),
        };
        let b = button(0.0, 0.0);
        assert_eq!(keyboard_targets(vec![slot, tab, b], true), vec![b]);
        assert_eq!(keyboard_targets(vec![slot, tab, b], false), vec![tab, b]);
    }

    #[test]
    fn focus_follows_its_widget_or_starts_again() {
        let a = button(0.0, 0.0);
        let moved = button(0.0, 4.0);
        let far = button(200.0, 100.0);
        assert_eq!(follow(a, &[far, moved]), Some(moved));
        // Gone (another screen): the first in tab order (insertion order in default group).
        let first = button(0.0, 50.0);
        assert_eq!(follow(a, &[far, first]), Some(far));
        assert_eq!(follow(a, &[]), None);
    }

    #[test]
    fn a_field_is_clicked_at_its_end() {
        assert_eq!(field_click([10.0, 20.0, 200.0, 20.0]), (208.0, 30.0));
    }
}
