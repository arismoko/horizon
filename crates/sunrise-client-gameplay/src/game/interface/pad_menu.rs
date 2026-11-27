//! A controller in menus, screens and inventories (Sunrise's own;
//! docs/controllers.md). Each screen is driven one way:
//!
//! - **Focus** (menus): a highlight steps between the widgets, list rows and
//!   fields each screen registers (`widgets::target_as`) with vanilla's
//!   arrow-key navigation; A presses the focused one, the bumpers switch tabs,
//!   the right stick scrolls, A takes hold of a focused slider for left/right to move, A on a text
//!   field opens the on-screen keyboard (`osk`).
//! - **Crosshair** (inventories, containers and the screens whose clickable
//!   parts register no target): a pointer the left stick moves and the D-pad
//!   steps between slots and buttons; the bumpers switch tabs, or the recipe
//!   book's pages with the triggers on its categories.
//!
//! Both feed the screens the same pointer events a mouse sends (the pointer
//! sits on the focused widget, hidden), so every screen keeps its mouse
//! behaviour (drags, sliders, scrollbars, hover highlights) with no pad code
//! of its own.
use super::Screen;
use super::event_loop::EventLoopCtl;
use crate::game::client_side::key_codes::glfw_modifiers;
use crate::game::client_side::key_focus::{self, Act, Nav};
use std::time::Instant;
use sunrise_client::actions::Action;
use sunrise_client::{
    creative::Creative,
    gamepad::{self, Button, Rect},
    hud::HudBatch,
    options_ui::{Event, Page},
    osk::{self, Keyboard},
    screen_narrator::InputType,
    widgets::{self, Kind, Target},
    window::PointerSample,
};
use sunrise_client_input::snap;
use sunrise_game_data::Result;
use winit::{
    event::{DeviceId, ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent},
    keyboard::KeyCode,
    window::WindowId,
};

/// How far the stick must lean to step focus.
const STICK_STEP: f32 = 0.55;
/// Scrolls a step may try to reveal the next row of a list.
const SCROLL_TRIES: u8 = 4;
/// Frames after a scroll that may still have been drawn before it: their rectangles are stale.
const STALE_FRAMES: u64 = 1;

/// Whether the frames drawn since `pad_reveal` scrolled a row, at frame `revealed`, may still
/// show the list where it was.
fn reveal_pending(revealed: Option<u64>, now: u64) -> bool {
    revealed.is_some_and(|at| now <= at + STALE_FRAMES)
}

/// A focus step that ran off the visible part of a list: the list was
/// scrolled and the step runs again once a new frame is drawn. When the
/// targets didn't move (nothing scrolled), the step takes vanilla's vague
/// pass instead.
#[derive(Clone)]
struct Pending {
    button: Button,
    from: Rect,
    generation: u64,
    tries: u8,
    before: Vec<Rect>,
    /// How far the list was scrolled, when it has a scrollbar.
    amount: Option<f32>,
}

/// What the focused widget is called (`widgets::slug`), found again by that when it moves: a list
/// scrolled another row into its place, a filter or a new layout moved it.
#[derive(Clone)]
struct FocusName {
    screen: u64,
    /// The focus it was read from: once the focus is elsewhere, it isn't this one's name.
    rect: Rect,
    name: String,
}

/// The focus the pad left a screen on (`MenuCursor::remembered`).
struct Remembered {
    rect: Rect,
    name: Option<String>,
}

/// The targets at the screen size they were drawn for, to carry the focus or the
/// crosshair over when that changes (a window resize, the GUI scale).
struct Layout {
    screen: u64,
    size: (f32, f32),
    rects: Vec<Rect>,
    /// What the focused widget is called (`widgets::slug`): how many rows fit changes with the
    /// size, so the same place in the draw order can be another widget.
    focus_name: Option<String>,
    /// The size last seen, once it's not `size`, and the frame drawn when it
    /// changed: the new targets come with a later frame.
    resized: Option<((f32, f32), u64)>,
}

/// The targets as the right stick found them, until a new frame shows how
/// far the list moved under the focus.
struct StickScrolled {
    screen: u64,
    generation: u64,
    down: bool,
    before: Vec<Rect>,
    /// How far the focus's list was scrolled, when it has a scrollbar.
    amount: Option<f32>,
}

#[derive(Default)]
pub struct MenuCursor {
    /// Where the pad put the pointer (GUI pixels); `None` while the mouse
    /// has it.
    pub(in crate::game) pos: Option<(f32, f32)>,
    last_poll: Option<Instant>,
    /// The stick moved the cursor last poll (it settles when let go).
    moving: bool,
    /// Gliding onto a slot or control: from, to, since.
    glide: Option<((f32, f32), (f32, f32), Instant)>,
    /// Where the stick's current movement started.
    gesture: Option<(f32, f32)>,
    /// Pointer mode: the open screen's target centres last poll.
    anchors: Option<(Screen, Vec<(f32, f32)>)>,
    layout: Option<Layout>,
    pub(super) wheel: [f32; 2],
    /// The wheel event being handled is the right stick's, on a focus-mode
    /// screen: it scrolls, whatever the pointer is over, and changes no value.
    stick_scroll: bool,
    /// The wheel event being handled is a focus step past a list's edge: it
    /// scrolls what the pointer is over, and changes no value.
    step_scroll: bool,
    stick_scrolled: Option<StickScrolled>,
    /// The held D-pad direction and when it last stepped.
    pub(super) repeat: Option<(Button, Instant, bool)>,
    /// Pointer buttons the pad holds down.
    pub(in crate::game) held: [bool; 2],
    /// The focused button A went down on: letting go presses it, unless the
    /// focus moved or the input was cancelled first.
    pub(in crate::game) armed: Option<Target>,
    /// The frame count when `pad_reveal` scrolled the focused row into view: the frames drawn
    /// before it still show the row where it was, so the focus isn't looked for in them.
    pub(in crate::game) revealed: Option<u64>,
    /// The OS pointer is hidden while the pad's cursor is drawn.
    os_hidden: bool,
    /// Polls since a menu opened (its widgets are drawn after one frame).
    open_polls: u32,
    /// Focus mode: the focused target as last drawn.
    pub focus: Option<Target>,
    /// The left stick's step direction last poll.
    stick: Option<Button>,
    pending: Option<Pending>,
    /// The focused slider being nudged and where the pad put its handle.
    nudge: Option<(Rect, f32)>,
    /// Steps since the held direction went down (sliders speed up).
    repeats: u32,
    /// The on-screen keyboard, open over a text field.
    pub osk: Option<Keyboard>,
    /// The recipe being crafted straight into the inventory (`pad_craft`).
    pub(in crate::game) craft: Option<super::pad_craft::DirectCraft>,
    /// LT held over the crafting book: it changes the category as it comes up, unless A was
    /// pressed meanwhile (LT + A crafts, `pad_craft`).
    pub(in crate::game) trigger: Option<HeldTrigger>,
    /// The keyboard was closed on this screen (it doesn't open by itself again).
    osk_dismissed: bool,
    /// The keyboard drives the focus (`key_focus`: Tab, the arrows, Enter):
    /// focus mode whatever the screen, the OS pointer shown.
    pub(in crate::game) keyboard: bool,
    /// The focused slider's arrows change its value (`canChangeValue`).
    slider_edit: bool,
    /// The slider A took hold of for left and right to move, and the screen it's on.
    pub(in crate::game) slider_hold: Option<(u64, Rect)>,
    /// What the last click landed on: vanilla focuses it, so the next Tab
    /// or arrow goes on from there (`ContainerEventHandler.mouseClicked`).
    pub(in crate::game) origin: Option<Rect>,
    /// A shortcut put the focus here: not where the screen reopens.
    shortcut_focus: Option<Rect>,
    /// A pointer event the pad or the keyboard sends is being handled.
    pub synthetic: bool,
    /// The real mouse's last sample, independent of the keyboard's virtual pointer.
    real_pointer: Option<PointerSample>,
    /// The screen the keyboard's focus is on (`Game::key_screen`), and the
    /// frame drawn when another one opened: the focus starts again on the
    /// new screen's first drawn frame.
    key_screen: u64,
    key_restart: Option<u64>,
    /// The pad's focus on each screen it left (`Game::key_screen`), so coming
    /// back lands where it was, as vanilla keeps a screen's focused widget.
    remembered: std::collections::HashMap<u64, Remembered>,
    /// The frame drawn when the pad last left a screen.
    left_screen: u64,
    /// The focused widget's name, while it has one of its own.
    focus_name: Option<FocusName>,
    /// Which prompt is up (`Game::pad_prompt_key`).
    prompt: Option<u64>,
    /// The frame drawn when one came or went: the focus starts again once it shows.
    prompt_restart: Option<u64>,
    /// The screen and the place the crosshair goes back to when the prompt over it closes.
    prompt_crosshair: Option<(Screen, (f32, f32))>,
}

/// LT held over the crafting book.
#[derive(Clone, Copy)]
pub(in crate::game) struct HeldTrigger {
    /// A was pressed while it was held: it was LT + A, not a category change.
    pressed_a: bool,
}

impl MenuCursor {
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_set_pos(&mut self, pos: Option<(f32, f32)>) {
        self.pos = pos;
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_armed_mut(&mut self) -> &mut Option<Target> {
        &mut self.armed
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_wheel(&self) -> [f32; 2] {
        self.wheel
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_repeat(&self) -> Option<(Button, Instant, bool)> {
        self.repeat
    }

    /// A running direct craft ends, as cleanly as the server still lets it (`DirectCraft::stop`).
    pub(in crate::game::client_side) fn stop_craft(&mut self) {
        if let Some(craft) = &mut self.craft {
            craft.stop();
        }
    }

    /// `Screen.clearFocus`: nothing is focused any more, and no click focus
    /// is kept for the next Tab (`ComponentPath.applyFocus(false)`).
    fn clear_focus(&mut self) {
        self.focus = None;
        self.origin = None;
        self.slider_edit = false;
        self.pending = None;
        self.armed = None;
    }
}

/// The target of the one widget called `name` (`widgets::slug`). None when several are: the
/// rectangles have to tell them apart.
fn named_target(name: &str, labelled: &[widgets::Widget], rects: &[Rect]) -> Option<usize> {
    let mut same = labelled.iter().filter(|w| widgets::slug(&w.label) == name);
    let w = same.next().filter(|_| same.next().is_none())?;
    let rect = [w.x, w.y, w.w, w.h];
    rects
        .iter()
        .position(|r| r.iter().zip(&rect).all(|(a, b)| (a - b).abs() < 0.5))
}

/// How far the list holding `rect` is scrolled.
fn list_amount(areas: &[gamepad::ScrollArea], rect: &Rect) -> Option<f32> {
    areas.iter().find(|a| a.holds(rect)).map(|a| a.amount)
}

/// How far the rows of a list moved, by what its scrollbar says: `None` when it
/// has none to ask, `Some(None)` when they didn't.
fn list_shift(was: Option<f32>, now: Option<f32>) -> Option<Option<f32>> {
    let dy = was?.trunc() - now?.trunc();
    Some((dy.abs() >= 0.5).then_some(dy))
}

/// The target nearest `rect` on its line, for a focus that lost it.
fn beside(rect: Rect, targets: &[Rect]) -> Option<usize> {
    let middle = rect[1] + rect[3] / 2.0;
    let centre = |r: &Rect| r[0] + r[2] / 2.0;
    targets
        .iter()
        .enumerate()
        .filter(|(_, r)| r[1] <= middle && middle < r[1] + r[3])
        .min_by(|(_, a), (_, b)| {
            let from = centre(&rect);
            (centre(a) - from)
                .abs()
                .total_cmp(&(centre(b) - from).abs())
        })
        .map(|(i, _)| i)
}

/// The controller's initial focus uses layout, independently of keyboard Tab order.
fn first_controller_target(targets: impl Iterator<Item = Target>) -> Option<Target> {
    targets.min_by_key(|t| (t.rect[1].round() as i32, t.rect[0].round() as i32))
}

/// Where letting go of A clicks: the armed button, if the focus is still on it. A row scrolled
/// into view under A (`pad_reveal`) is found again a fraction of a pixel off.
fn armed_click(armed: Option<Target>, focus: Option<Target>) -> Option<Target> {
    focus.filter(|focus| armed.is_some_and(|old| old.same_focus_target(*focus)))
}

impl crate::game::ClientGame<'_> {
    /// A menu, a screen, the title, a sign or a book is open: the pad drives
    /// it instead of the player.
    pub(in crate::game::client_side) fn pad_in_menu(&self) -> bool {
        self.key_focus_screen() || self.prompt_open()
    }

    /// A screen the keyboard's focus can drive is open. A prompt over the world isn't one: its
    /// keys are its own.
    fn key_focus_screen(&self) -> bool {
        self.client.title.0.is_some()
            || self.client.screen != Screen::Playing
            || self.client.sign_edit.is_some()
            || self.client.book.is_some()
            || self.observed_overlay_now() == Some("dialog")
            || (self.prompt_open() && !self.client.screen_state.0.widgets.is_empty())
    }

    /// Which prompt is up: another one starts the controller's focus again.
    fn pad_prompt_key(&self) -> Option<u64> {
        use std::hash::{Hash, Hasher};
        if !self.prompt_open() {
            return None;
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (
            self.client.command_prompt.0.is_some(),
            self.client.link_prompt.is_some(),
            self.client.code_of_conduct.is_some(),
            self.client.server_packs.0.prompt.is_some(),
        )
            .hash(&mut hasher);
        self.dialog_screen_key().hash(&mut hasher);
        Some(hasher.finish())
    }

    /// B or Start on a prompt: as Escape, the top one first. The code of conduct has no answer
    /// by Escape (`shouldCloseOnEsc`), and a dialog only when it allows it.
    pub(in crate::game::client_side) fn pad_prompt_back(&mut self) -> Result<()> {
        if self.client.command_prompt.0.is_some() {
            self.answer_command_prompt(false);
        } else if self.client.link_prompt.is_some() {
            self.answer_link_prompt(false);
        } else if self.client.code_of_conduct.is_some() {
        } else if self.client.server_packs.0.prompt.is_some() {
            self.answer_pack_prompt(false)?;
        } else {
            self.dialog_key(KeyCode::Escape, None)?;
        }
        Ok(())
    }

    /// Where the focus starts on a prompt: on what answers no, when it has that, so a press meant
    /// for the game can't accept anything. A dialog with text to fill in starts on its first
    /// input; its warning icon comes last.
    fn pad_prompt_first(&self, targets: &[Target]) -> Option<Target> {
        use widgets::Action;
        if !self.dialog_starts_typing() {
            let declared = widgets::last_actions(self.key_screen());
            let no = declared
                .iter()
                .filter(|d| d.enabled && matches!(d.action, Action::Cancel | Action::Back))
                .find_map(|d| targets.iter().find(|t| t.rect == d.rect));
            if let Some(no) = no {
                return Some(*no);
            }
        }
        let warning = self.dialog_warning_icon();
        first_controller_target(targets.iter().copied().filter(|t| Some(t.rect) != warning))
    }

    pub fn gui_batch(&self) -> Option<HudBatch> {
        self.client.renderer.0.as_ref().map(|r| {
            HudBatch::new(
                [r.config.width, r.config.height],
                self.client.settings.gui_scale_inputs(),
            )
        })
    }

    /// Focus drives the open screen: every menu but inventories and
    /// containers, and the screens whose clickable parts register no focus
    /// target (the advancements map, the statistics lists, the pack rows, a
    /// book's page text), where the pointer does (the keyboard always focuses).
    pub(in crate::game::client_side) fn pad_navigation_mode(&self) -> bool {
        self.prompt_open() || !self.pad_pointer_screen()
    }

    /// The open screen is one the crosshair drives, whatever is over it.
    fn pad_pointer_screen(&self) -> bool {
        matches!(
            self.client.screen,
            Screen::HudEditor
                | Screen::Inventory
                | Screen::Creative
                | Screen::Stats
                | Screen::Advancements
        ) || self.client.book.is_some()
            || (self.client.screen == Screen::Options
                && self.client.chat_ui.options.page() == Page::Packs)
    }

    pub(in crate::game::client_side) fn pad_focus_mode(&self) -> bool {
        self.client.pad.0.menu.keyboard || self.pad_navigation_mode()
    }

    /// Where focus can land: buttons drawn last frame, every registered
    /// target (sliders, fields, rows, keyboard keys), and the slots
    /// and tabs of inventories (GUI pixels), on screen. The keyboard's
    /// focus skips what vanilla's keyboard can't reach (`key_targets`).
    pub fn pad_targets(&self) -> Vec<Target> {
        if self.client.pad.0.menu.keyboard {
            self.key_targets()
        } else {
            self.pad_visible_targets()
        }
    }

    /// The targets the keyboard reaches: no slots, no inventory tabs.
    fn key_targets(&self) -> Vec<Target> {
        // Advancements' tabs aren't widgets either: vanilla clicks them by hand. Statistics'
        // stay out too until a parity check settles where its tab bar sits in the order.
        let inventory = self.client.title.0.is_none()
            && matches!(
                self.client.screen,
                Screen::Inventory | Screen::Creative | Screen::Advancements | Screen::Stats
            );
        key_focus::keyboard_targets(self.pad_visible_targets(), inventory)
    }

    fn pad_visible_targets(&self) -> Vec<Target> {
        let targets = self.pad_all_targets();
        match self.gui_batch() {
            Some(batch) => key_focus::on_screen(targets, (batch.width, batch.height)),
            None => targets,
        }
    }

    /// The inventory screen's slots as pad targets, at the positions the
    /// frame draws them (a mod screen's moved slots included).
    pub fn pad_slot_targets(&self, batch: &HudBatch) -> Vec<Target> {
        let (ox, oy) = self.inventory_origin(batch);
        self.slots()
            .into_iter()
            .map(|i| {
                let (x, y) = self.slot_position(i);
                Target {
                    rect: [ox + x, oy + y, 16.0, 16.0],
                    kind: Kind::Slot,
                }
            })
            .collect()
    }

    fn pad_all_targets(&self) -> Vec<Target> {
        let mut targets = key_focus::drawn_targets();
        targets.extend(self.mounted_widget_targets());
        if self.client.title.0.is_some()
            || self.client.sign_edit.is_some()
            || self.client.book.is_some()
            || self.prompt_open()
        {
            return targets;
        }
        let Some(batch) = self.gui_batch() else {
            return targets;
        };
        let slot = |x: f32, y: f32| Target {
            rect: [x, y, 16.0, 16.0],
            kind: Kind::Slot,
        };
        match self.client.screen {
            Screen::Inventory => {
                let (ox, oy) = self.inventory_origin(&batch);
                targets.extend(self.pad_slot_targets(&batch));
                targets.extend(
                    self.station_targets()
                        .into_iter()
                        .map(|[x, y, w, h]| Target {
                            rect: [ox + x, oy + y, w, h],
                            kind: Kind::Button,
                        }),
                );
                targets.extend(self.recipe_book_targets());
            }
            Screen::Creative => {
                let (ox, oy) = Self::creative_origin(&batch);
                if self.creative_inventory_open() {
                    targets.extend(self.slots().into_iter().map(|i| {
                        let (x, y) = self.slot_position(i);
                        slot(ox + x, oy + y)
                    }));
                } else {
                    for r in 0..5 {
                        for c in 0..9 {
                            targets.push(slot(
                                ox + 9.0 + c as f32 * 18.0,
                                oy + 18.0 + r as f32 * 18.0,
                            ));
                        }
                    }
                    for c in 0..9 {
                        targets.push(slot(ox + 9.0 + c as f32 * 18.0, oy + 112.0));
                    }
                }
                for (i, t) in self.client.creative_screen.0.tabs.iter().enumerate() {
                    // A tab of another page of mods' tabs isn't drawn: its place is another's.
                    if !self.client.creative_screen.0.on_page(t) {
                        continue;
                    }
                    let (x, y) = Creative::tab_position(t);
                    targets.push(Target {
                        rect: [ox + x, oy + y, 26.0, 32.0],
                        kind: Kind::Tab(i == self.client.creative_screen.0.selected_index()),
                    });
                }
            }
            _ => {}
        }
        targets
    }

    /// Where focus starts when a menu opens: the held hotbar slot in an
    /// inventory, else the first button, else the first target that isn't
    /// a tab.
    fn pad_first_target(&self, targets: &[Target]) -> Option<Target> {
        if self.client.pad.0.menu.keyboard {
            // Screen.setInitialFocus: a forward Tab from nothing.
            return key_focus::tab_step(targets, None, true).map(|i| targets[i]);
        }
        if self.prompt_open() {
            return self.pad_prompt_first(targets).or(targets.first().copied());
        }
        let rects: Vec<Rect> = targets.iter().map(|t| t.rect).collect();
        // Back on a screen the pad left: where it was, as vanilla keeps a
        // screen's focused widget (Options, after its Done). The same widget
        // first: the rows under it can have moved.
        if let Some(i) = self
            .client
            .pad
            .0
            .menu
            .remembered
            .get(&self.key_screen())
            .and_then(|r| {
                r.name
                    .as_ref()
                    .and_then(|n| named_target(n, &widgets::last_labelled(), &rects))
                    .or_else(|| gamepad::refind(r.rect, &rects, 2.0))
            })
        {
            return Some(targets[i]);
        }
        let batch = self.gui_batch()?;
        let at = |x: f32, y: f32| {
            targets
                .iter()
                .find(|t| t.rect[0] == x && t.rect[1] == y)
                .copied()
        };
        if self.client.title.0.is_none() {
            match self.client.screen {
                Screen::Inventory => {
                    let (ox, oy) = self.inventory_origin(&batch);
                    let (x, y) = self.slot_position(self.common.selected);
                    if let Some(t) = at(ox + x, oy + y) {
                        return Some(t);
                    }
                }
                Screen::Creative if !self.creative_inventory_open() => {
                    let (ox, oy) = Self::creative_origin(&batch);
                    if let Some(t) = at(ox + 9.0 + self.common.selected as f32 * 18.0, oy + 112.0) {
                        return Some(t);
                    }
                }
                _ => {}
            }
        }
        first_controller_target(targets.iter().copied()).or(targets.first().copied())
    }

    /// The pointer is over a slot or control.
    fn pad_over_target(&self) -> bool {
        let (x, y) = self.client.cursor;
        self.pad_targets().iter().any(|t| {
            let [tx, ty, w, h] = t.rect;
            (tx..tx + w).contains(&x) && (ty..ty + h).contains(&y)
        })
    }

    /// Start the cursor's glide onto `to` (GUI pixels), from wherever it is.
    fn pad_glide_to(&mut self, to: (f32, f32)) {
        self.client.pad.0.menu.glide = Some((self.client.cursor, to, Instant::now()));
    }

    /// Send the window a pointer move to `at` (GUI pixels), as the mouse would.
    fn pad_pointer_to(&mut self, event_loop: &dyn EventLoopCtl, at: (f32, f32)) {
        let (Some(window), Some(projection)) = (
            self.client.window.0.as_ref(),
            self.renderer_pointer_projection(),
        ) else {
            return;
        };
        let id = window.id();
        let at = (
            at.0.clamp(0.0, projection.gui.size[0] as f32 - 1.0),
            at.1.clamp(0.0, projection.gui.size[1] as f32 - 1.0),
        );
        let position = projection.window_pointer(at);
        self.pad_synthetic_event(
            event_loop,
            id,
            WindowEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position,
            },
            None,
        );
        // Read back what the window made of it, so a real mouse move shows.
        self.client.pad.0.menu.pos = Some(self.client.cursor);
    }

    fn pad_pointer_button(&mut self, event_loop: &dyn EventLoopCtl, right: bool, pressed: bool) {
        let Some(id) = self.client.window.0.as_ref().map(|w| w.id()) else {
            return;
        };
        self.client.pad.0.menu.held[usize::from(right)] = pressed;
        let button = if right {
            MouseButton::Right
        } else {
            MouseButton::Left
        };
        let state = if pressed {
            ElementState::Pressed
        } else {
            ElementState::Released
        };
        self.pad_synthetic_event(
            event_loop,
            id,
            WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state,
                button,
            },
            None,
        );
    }

    /// A click (press and release) at a point, the pointer staying there.
    /// A pointer event of the pad's or the keyboard's, as the window would
    /// send it (marked, so it isn't taken for the real mouse).
    fn pad_synthetic_event(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        id: WindowId,
        event: WindowEvent,
        retained_sample: Option<PointerSample>,
    ) {
        let was = std::mem::replace(&mut self.client.pad.0.menu.synthetic, true);
        self.handle_window_event_with_sample(
            event_loop,
            id,
            event,
            super::input_trace::InputOrigin::Game,
            retained_sample,
        );
        self.client.pad.0.menu.synthetic = was;
    }

    /// The event being handled is one the pad or the keyboard's focus made
    /// up (`pad_synthetic_event`), not the window's. A controller's click or
    /// a focused widget's is not the mouse, so the native mods' mouse hooks
    /// skip it.
    pub fn pad_synthetic_input(&self) -> bool {
        self.client.pad.0.menu.synthetic
    }

    fn pad_click_at(&mut self, event_loop: &dyn EventLoopCtl, at: (f32, f32), right: bool) {
        self.pad_pointer_to(event_loop, at);
        self.pad_pointer_button(event_loop, right, true);
        self.pad_pointer_button(event_loop, right, false);
    }

    /// The wheel event being handled only scrolls (`MenuCursor::stick_scroll`).
    pub(in crate::game::client_side) fn pad_stick_scroll(&self) -> bool {
        self.client.pad.0.menu.stick_scroll || self.client.pad.0.menu.step_scroll
    }

    /// It came from a focus step, so the pointer still decides what scrolls.
    pub(in crate::game::client_side) fn pad_step_scroll(&self) -> bool {
        self.client.pad.0.menu.step_scroll
    }

    /// The right stick's wheel: in focus mode a list scrolls and the focus
    /// goes with it (`pad_follow_focus`, once the next frame is drawn).
    fn pad_stick_wheel(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        focus_mode: bool,
        targets: &[Target],
        [x, y]: [f32; 2],
    ) {
        if focus_mode && y != 0.0 && self.client.pad.0.menu.stick_scrolled.is_none() {
            let amount = self
                .client
                .pad
                .0
                .menu
                .focus
                .and_then(|f| list_amount(&widgets::last_scroll_areas(), &f.rect));
            self.client.pad.0.menu.stick_scrolled = Some(StickScrolled {
                amount,
                screen: self.key_screen(),
                generation: widgets::generation(),
                down: y < 0.0,
                before: targets.iter().map(|t| t.rect).collect(),
            });
        }
        self.client.pad.0.menu.stick_scroll = focus_mode;
        self.pad_wheel(event_loop, x, y);
        self.client.pad.0.menu.stick_scroll = false;
    }

    fn pad_step_wheel(&mut self, event_loop: &dyn EventLoopCtl, button: Button) {
        let y = if button == Button::DPadUp { 1.0 } else { -1.0 };
        self.client.pad.0.menu.step_scroll = true;
        self.pad_wheel(event_loop, 0.0, y);
        self.client.pad.0.menu.step_scroll = false;
    }

    fn pad_wheel(&mut self, event_loop: &dyn EventLoopCtl, x: f32, y: f32) {
        // Another row can scroll under the armed button's rect.
        self.client.pad.0.menu.armed = None;
        let Some(id) = self.client.window.0.as_ref().map(|w| w.id()) else {
            return;
        };
        self.pad_synthetic_event(
            event_loop,
            id,
            WindowEvent::MouseWheel {
                device_id: DeviceId::dummy(),
                delta: MouseScrollDelta::LineDelta(x, y),
                phase: TouchPhase::Moved,
            },
            None,
        );
    }

    /// Put the pointer where the pad last left it, or on the first target
    /// (focus mode: focus lands there too).
    fn pad_take_pointer(&mut self, event_loop: &dyn EventLoopCtl) {
        if self.client.pad.0.menu.pos.is_some() {
            return;
        }
        let targets = self.pad_targets();
        let first = self.pad_first_target(&targets);
        if self.pad_focus_mode() && first.is_some() {
            self.client.pad.0.menu.focus = first;
            if let Some(target) = first {
                self.pad_focus_selects_row(target);
            }
        }
        let at = first.map_or(self.client.cursor, |t| t.center());
        self.pad_pointer_to(event_loop, at);
    }

    /// Run `f` with `key` held as a keyboard holds it, in the modifier word
    /// screens read as well as the physical keys, then put both back. The
    /// pad's only way to send Shift or Ctrl, so it can't fall out of step
    /// with what the keyboard sends.
    fn pad_with_modifier<T>(&mut self, key: KeyCode, f: impl FnOnce(&mut Self) -> T) -> T {
        let held = sunrise_client_input::modifiers::hold(
            &mut self.client.modifiers,
            &mut self.client.physical_keys,
            key,
        );
        let result = f(self);
        sunrise_client_input::modifiers::restore(
            &mut self.client.modifiers,
            &mut self.client.physical_keys,
            held,
        );
        result
    }

    /// Focus mode with something to focus on this screen.
    fn pad_focusing(&self) -> bool {
        self.pad_focus_mode() && self.client.pad.0.menu.focus.is_some()
    }

    /// Clear transient pad menu holds and armed targets when a context menu takes focus.
    #[inline(always)]
    pub(super) fn settle_pad_for_context_menu(&mut self) {
        self.direct_craft_stop();
        self.client
            .screen_state
            .0
            .context_menu_input
            .take_pad_repeat_owned();
        self.client
            .screen_state
            .0
            .context_menu_input
            .take_pad_wheel_owned();
        let menu = &mut self.client.pad.0.menu;
        menu.armed = None;
        menu.trigger = None;
        menu.repeat = None;
        menu.wheel = [0.0, 0.0];
        menu.slider_hold = None;
        menu.nudge = None;
        menu.glide = None;
        menu.pending = None;
        menu.stick_scrolled = None;
    }

    /// Clear any pad direction repeat or wheel accumulation started by the context menu on close.
    #[inline(always)]
    pub(super) fn clear_context_menu_pad_state(&mut self) {
        if self
            .client
            .screen_state
            .0
            .context_menu_input
            .take_pad_repeat_owned()
        {
            self.client.pad.0.menu.repeat = None;
        }
        if self
            .client
            .screen_state
            .0
            .context_menu_input
            .take_pad_wheel_owned()
        {
            self.client.pad.0.menu.wheel = [0.0, 0.0];
        }
    }

    /// A button in a menu.
    pub fn pad_menu_button(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        button: Button,
        down: bool,
    ) -> Result<()> {
        if !down {
            if self
                .client
                .screen_state
                .0
                .context_menu_input
                .take_pad(button)
            {
                if self.client.pad.0.menu.repeat.is_some_and(|r| r.0 == button) {
                    self.client.pad.0.menu.repeat = None;
                    self.client
                        .screen_state
                        .0
                        .context_menu_input
                        .take_pad_repeat_owned();
                }
                return Ok(());
            }
            if self.context_menu_open() {
                self.client.pad.0.menu.armed = None;
                self.client.pad.0.menu.trigger = None;
                self.client.pad.0.menu.slider_hold = None;
                if self.client.pad.0.menu.repeat.is_some_and(|r| r.0 == button) {
                    self.client.pad.0.menu.repeat = None;
                }
                match button {
                    Button::South if self.client.pad.0.menu.held[0] => {
                        self.pad_pointer_button(event_loop, false, false);
                    }
                    Button::West if self.client.pad.0.menu.held[1] => {
                        self.pad_pointer_button(event_loop, true, false);
                    }
                    _ => {}
                }
                return Ok(());
            }
            match button {
                Button::LeftTrigger if self.client.pad.0.menu.trigger.is_some() => {
                    let pressed_a = self
                        .client
                        .pad
                        .0
                        .menu
                        .trigger
                        .take()
                        .is_some_and(|t| t.pressed_a);
                    if !pressed_a && self.pad_recipe_book_showing() {
                        self.pad_tab(event_loop, false);
                    }
                }
                Button::South if self.client.pad.0.menu.armed.is_some() => {
                    let armed = self.client.pad.0.menu.armed.take();
                    if let Some(target) = armed_click(armed, self.client.pad.0.menu.focus) {
                        if let Kind::Mounted { id, .. } = target.kind {
                            self.activate_mounted_widget(
                                mod_api::ScreenWidgetId::from_raw(id),
                                target.center(),
                            );
                        } else {
                            self.pad_click_at(event_loop, target.center(), false);
                        }
                    }
                }
                Button::South if self.client.pad.0.menu.held[0] => {
                    self.pad_pointer_button(event_loop, false, false)
                }
                Button::West if self.client.pad.0.menu.held[1] => {
                    self.pad_pointer_button(event_loop, true, false)
                }
                b if self.client.pad.0.menu.repeat.is_some_and(|r| r.0 == b) => {
                    self.client.pad.0.menu.repeat = None
                }
                _ => {}
            }
            return Ok(());
        }
        // The pad takes the focus back from the keyboard.
        self.client.pad.0.menu.keyboard = false;
        if self.context_menu_open() {
            self.settle_pad_for_context_menu();
            self.client
                .screen_state
                .0
                .context_menu_input
                .own_pad(button);
            match button {
                Button::DPadUp => {
                    self.client.pad.0.menu.repeats = 0;
                    self.context_menu_key(KeyCode::ArrowUp);
                    self.client
                        .screen_state
                        .0
                        .context_menu_input
                        .own_pad_repeat();
                    self.client.pad.0.menu.repeat = Some((button, Instant::now(), false));
                }
                Button::DPadDown => {
                    self.client.pad.0.menu.repeats = 0;
                    self.context_menu_key(KeyCode::ArrowDown);
                    self.client
                        .screen_state
                        .0
                        .context_menu_input
                        .own_pad_repeat();
                    self.client.pad.0.menu.repeat = Some((button, Instant::now(), false));
                }
                Button::DPadLeft => {
                    self.client.pad.0.menu.repeats = 0;
                    self.context_menu_key(KeyCode::ArrowLeft);
                    self.client
                        .screen_state
                        .0
                        .context_menu_input
                        .own_pad_repeat();
                    self.client.pad.0.menu.repeat = Some((button, Instant::now(), false));
                }
                Button::DPadRight => {
                    self.client.pad.0.menu.repeats = 0;
                    self.context_menu_key(KeyCode::ArrowRight);
                    self.client
                        .screen_state
                        .0
                        .context_menu_input
                        .own_pad_repeat();
                    self.client.pad.0.menu.repeat = Some((button, Instant::now(), false));
                }
                Button::South => {
                    self.context_menu_key(KeyCode::Enter);
                }
                Button::East | Button::Start => {
                    self.context_menu_key(KeyCode::Escape);
                }
                _ => {}
            }
            return Ok(());
        }
        // A with LT held over the crafting book is LT + A: LT coming up changes no category.
        if button == Button::South
            && let Some(trigger) = &mut self.client.pad.0.menu.trigger
        {
            trigger.pressed_a = true;
        }
        // Any press ends a running craft but another LT + A on its recipe, which asks for one
        // more further down: quick taps while one is in flight each make a craft.
        if !self.direct_craft_takes_press(button) {
            self.direct_craft_stop();
        }
        // A prompt over the screen answers first; no press reaches both.
        let prompt = self.prompt_open();
        // Bindings, or the radial menu's opener row, waiting for a button: this one (Start
        // cancels).
        if !prompt
            && self.client.screen == Screen::Options
            && let Some(action) = self.client.chat_ui.options.selected_pad_action.take()
        {
            // Start cancels, and so does a button the action can't take (the radial menu's
            // opener under the selecting thumb or on B, `gamepad::opener_refused`): the binding
            // stays.
            if button != Button::Start && self.client.settings.controller.can_bind(action, button) {
                self.client.settings.controller.bind(action, Some(button));
            }
            return self.handle_option_events(vec![Event::Changed]);
        }
        // A key row waiting for its key takes a mouse button: anything that clicks would bind it.
        // B or Start is Escape, which unbinds (`pad_back`).
        if self.pad_binding() && !matches!(button, Button::East | Button::Start) {
            return Ok(());
        }
        // The screen under the keyboard went away (dying with chat open): so does the keyboard.
        if self.client.pad.0.menu.osk.is_some() && !self.pad_has_typing_screen() {
            self.client.pad.0.menu.osk = None;
        }
        // The on-screen keyboard takes every button while it is open.
        let suggests = self.client.pad.0.menu.osk.is_some() && self.pad_chat_owns_keyboard();
        if let Some(keyboard) = self.client.pad.0.menu.osk.as_mut() {
            if let Some(out) = keyboard.button(button, suggests) {
                self.pad_osk_output(out);
            }
            if matches!(
                button,
                Button::DPadUp | Button::DPadDown | Button::DPadLeft | Button::DPadRight
            ) {
                self.client.pad.0.menu.repeat = Some((button, Instant::now(), false));
            }
            return Ok(());
        }
        // B lets go of a slider, before it means Back or a screen's button.
        if button == Button::East && self.pad_holding_slider() {
            self.client.pad.0.menu.slider_hold = None;
            return Ok(());
        }
        if let Some((action, target)) = self.pad_action_target(button) {
            // The button may change a slider's value, which the pad's own position doesn't know.
            self.client.pad.0.menu.nudge = None;
            if action == widgets::Action::Exit {
                self.pad_focus_on(event_loop, target);
                self.client.pad.0.menu.shortcut_focus = Some(target.rect);
            } else {
                self.pad_click_at(event_loop, target.center(), false);
            }
            return Ok(());
        }
        // A prompt has its focus and B: the screen behind it has none of its buttons now.
        if prompt
            && !matches!(
                button,
                Button::South
                    | Button::East
                    | Button::Start
                    | Button::DPadUp
                    | Button::DPadDown
                    | Button::DPadLeft
                    | Button::DPadRight
            )
        {
            return Ok(());
        }
        // A with nothing focused lands the focus, as the D-pad does: nothing
        // fires on a widget the player hasn't seen highlighted.
        if button == Button::South
            && self.pad_focus_mode()
            && self.client.pad.0.menu.focus.is_none()
        {
            let targets = self.pad_targets();
            if let Some(first) = self.pad_first_target(&targets) {
                self.pad_focus_on(event_loop, first);
                return Ok(());
            }
        }
        // LT + A on a recipe crafts it into the inventory.
        if button == Button::South
            && !prompt
            && self.client.pad.0.edges.is_held(Button::LeftTrigger)
            && self.client.screen == Screen::Inventory
            && self.client.title.0.is_none()
        {
            self.pad_take_pointer(event_loop);
            // Refused, the press is plain A's.
            if let Some(entry) = self.recipe_under_pointer()
                && self.direct_craft_start(entry)
            {
                return Ok(());
            }
        }
        // The book's controls take A alone; X, Y and the throws are for slots and recipes.
        let book_control = !prompt
            && self.client.screen == Screen::Inventory
            && self.client.title.0.is_none()
            && self.recipe_book_under_pointer();
        let on_recipe = book_control && self.recipe_under_pointer().is_some();
        match button {
            Button::West | Button::North if book_control && !on_recipe => {}
            Button::Select | Button::LeftStick if book_control => {}
            Button::East | Button::Start => return self.pad_back(),
            Button::South if self.pad_focusing() => {
                let focus = self.pad_reveal(self.client.pad.0.menu.focus.expect("focusing"));
                self.client.pad.0.menu.focus = Some(focus);
                match focus.kind {
                    // A click would jump the handle: A takes hold of it, or lets it go.
                    Kind::Slider { .. } => {
                        let hold =
                            (!self.pad_holding_slider()).then(|| (self.key_screen(), focus.rect));
                        self.client.pad.0.menu.slider_hold = hold;
                        self.client.pad.0.menu.nudge = None;
                    }
                    Kind::Mounted { id, field: true } => {
                        if self.activate_mounted_widget(
                            mod_api::ScreenWidgetId::from_raw(id),
                            focus.center(),
                        ) && self.mounted_widget_captures_input()
                        {
                            self.pad_open_keyboard();
                        }
                    }
                    Kind::Field => {
                        self.pad_click_at(event_loop, focus.center(), false);
                        self.pad_open_keyboard();
                    }
                    Kind::Button | Kind::Tab(_) | Kind::Mounted { field: false, .. } => {
                        self.pad_pointer_to(event_loop, focus.center());
                        self.client.pad.0.menu.armed = Some(focus);
                    }
                    _ => {
                        self.pad_pointer_to(event_loop, focus.center());
                        self.pad_pointer_button(event_loop, false, true);
                    }
                }
            }
            // A sign has no field to press, so A opens its keyboard from anywhere on it.
            Button::South if !prompt && self.client.sign_edit.is_some() => self.pad_open_keyboard(),
            Button::South | Button::West => {
                self.pad_take_pointer(event_loop);
                self.pad_pointer_button(event_loop, button == Button::West, true);
                // A on a text field in pointer mode opens the keyboard, as
                // in focus mode (not the right click).
                if button == Button::South
                    && !self.pad_focusing()
                    && field_at(&self.pad_targets(), self.client.cursor)
                {
                    self.pad_open_keyboard();
                }
            }
            // Y in an inventory: a shift-click (quick move).
            Button::North if matches!(self.client.screen, Screen::Inventory | Screen::Creative) => {
                self.pad_take_pointer(event_loop);
                let in_keys = self.held_mapping(Action::Sneak);
                self.insert_device_key(KeyCode::ShiftLeft);
                self.pad_with_modifier(KeyCode::ShiftLeft, |game| {
                    game.pad_pointer_button(event_loop, false, true);
                    game.pad_pointer_button(event_loop, false, false);
                });
                if !in_keys {
                    self.remove_device_key(&KeyCode::ShiftLeft);
                }
            }
            // View / left stick in an inventory: throw one / the stack from
            // the slot under focus (Q / Ctrl+Q).
            Button::Select | Button::LeftStick
                if matches!(self.client.screen, Screen::Inventory | Screen::Creative)
                    && self.client.title.0.is_none() =>
            {
                self.pad_take_pointer(event_loop);
                if self.client.screen != Screen::Creative
                    || !self.client.creative_screen.0.tab().search
                {
                    if button == Button::LeftStick {
                        self.pad_with_modifier(KeyCode::ControlLeft, |game| {
                            game.inventory_hotkey(KeyCode::KeyQ);
                        });
                    } else {
                        self.inventory_hotkey(KeyCode::KeyQ);
                    }
                }
            }
            Button::DPadUp | Button::DPadDown | Button::DPadLeft | Button::DPadRight => {
                self.client.pad.0.menu.repeats = 0;
                self.pad_step(event_loop, button);
                self.client.pad.0.menu.repeat = Some((button, Instant::now(), false));
            }
            // The recipe book open: bumpers turn its pages, triggers change its category.
            Button::LeftBumper | Button::RightBumper if self.pad_recipe_book_showing() => {
                self.pad_recipe_page(button == Button::RightBumper);
            }
            // On the crafting book LT waits for its release, as LT + A crafts there; RT, and
            // either on a furnace's book, change the category as they go down.
            Button::LeftTrigger
                if self.pad_recipe_book_showing()
                    && self.active_book() == Some(sunrise_sim::recipe_book::BookType::Crafting) =>
            {
                self.client.pad.0.menu.trigger = Some(HeldTrigger { pressed_a: false });
            }
            Button::LeftTrigger | Button::RightTrigger if self.pad_recipe_book_showing() => {
                self.pad_tab(event_loop, button == Button::RightTrigger);
            }
            // Bumpers switch tabs (creative, Create World, the graphics sidebar, ...).
            Button::LeftBumper | Button::RightBumper => {
                self.pad_tab(event_loop, button == Button::RightBumper);
            }
            _ => {}
        }
        Ok(())
    }

    /// One D-pad (or stick) step in either mode.
    fn pad_step(&mut self, event_loop: &dyn EventLoopCtl, button: Button) {
        if self.context_menu_open() {
            let key = match button {
                Button::DPadUp => KeyCode::ArrowUp,
                Button::DPadDown => KeyCode::ArrowDown,
                Button::DPadLeft => KeyCode::ArrowLeft,
                Button::DPadRight => KeyCode::ArrowRight,
                _ => return,
            };
            self.context_menu_key(key);
            return;
        }
        if let Some(keyboard) = self.client.pad.0.menu.osk.as_mut() {
            keyboard.step(button);
            return;
        }
        // A slider's step is a click, which a waiting key row would bind.
        if self.pad_binding() {
            return;
        }
        if self.pad_focus_mode() {
            let targets = self.pad_targets();
            if !targets.is_empty() {
                self.pad_focus_step(event_loop, button, &targets);
                return;
            }
        }
        self.pad_pointer_step(event_loop, button);
    }

    /// Pointer mode: hop the cursor to the nearest target that way.
    fn pad_pointer_step(&mut self, event_loop: &dyn EventLoopCtl, button: Button) {
        if self.client.pad.0.menu.pos.is_none() {
            self.pad_take_pointer(event_loop);
            return;
        }
        // With A held over slots, the drag goes on to the next slot (a stack
        // spreads); on anything else (a slider grabbed), the D-pad nudges.
        if self.client.pad.0.menu.held[0] {
            let slots: Vec<Target> = self
                .pad_targets()
                .into_iter()
                .filter(|t| matches!(t.kind, Kind::Slot))
                .collect();
            let centres: Vec<(f32, f32)> = slots.iter().map(Target::center).collect();
            if snap::nearest(self.client.cursor, &centres, 0.5).is_some() {
                if let Some(i) = gamepad::focus_step(self.client.cursor, button, &centres)
                    && (self.client.pad.0.menu.repeats == 0
                        || !gamepad::repeat_crosses_slots(self.client.cursor, centres[i], true))
                {
                    self.pad_pointer_to(event_loop, centres[i]);
                }
                return;
            }
            let (dx, dy) = match button {
                Button::DPadLeft => (-2.0, 0.0),
                Button::DPadRight => (2.0, 0.0),
                Button::DPadUp => (0.0, -2.0),
                _ => (0.0, 2.0),
            };
            let at = self.client.cursor;
            self.pad_pointer_to(event_loop, (at.0 + dx, at.1 + dy));
            return;
        }
        if matches!(button, Button::DPadUp | Button::DPadDown)
            && self.client.screen == Screen::Inventory
            && self.station_list_step(button == Button::DPadUp)
        {
            return;
        }
        // A step during a glide goes on from where the glide ends.
        let from = self
            .client
            .pad
            .0
            .menu
            .glide
            .map_or(self.client.cursor, |(_, to, _)| to);
        let targets = self.pad_targets();
        let centres: Vec<(f32, f32)> = targets.iter().map(Target::center).collect();
        let Some(i) = gamepad::focus_step(from, button, &centres) else {
            return;
        };
        // A held direction stops at the edge of the slots it is on; a fresh press may cross.
        let on_slot = targets.iter().any(|t| {
            t.kind == Kind::Slot
                && (t.center().0 - from.0).abs() <= 0.5
                && (t.center().1 - from.1).abs() <= 0.5
        });
        if self.client.pad.0.menu.repeats > 0
            && on_slot
            && gamepad::repeat_crosses_slots(from, centres[i], targets[i].kind == Kind::Slot)
        {
            return;
        }
        self.pad_glide_to(centres[i]);
    }

    /// Focus mode: move the highlight (or a focused slider's value).
    fn pad_focus_step(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        button: Button,
        targets: &[Target],
    ) {
        let Some(focus) = self.client.pad.0.menu.focus else {
            self.client.pad.0.menu.pos = None;
            self.pad_take_pointer(event_loop);
            return;
        };
        if self.pad_holding_slider() && matches!(button, Button::DPadLeft | Button::DPadRight) {
            let step = gamepad::slider_step(self.client.pad.0.menu.repeats);
            let sign = if button == Button::DPadLeft {
                -1.0
            } else {
                1.0
            };
            self.pad_nudge(event_loop, focus, sign * step);
            return;
        }
        if self.pad_step_value(event_loop, button, focus) {
            return;
        }
        self.pad_focus_move(event_loop, button, focus, targets);
    }

    /// Left or right on a focused value the screen steps with them (`widgets::steps_value`: the
    /// settings shell's choices): Right presses it, Left presses it with Shift, the screen's own
    /// step back. Whether it did; anything else moves the focus.
    fn pad_step_value(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        button: Button,
        focus: Target,
    ) -> bool {
        if !matches!(button, Button::DPadLeft | Button::DPadRight)
            || !widgets::last_steps_value(focus.rect)
        {
            return false;
        }
        let focus = self.pad_reveal(focus);
        self.client.pad.0.menu.focus = Some(focus);
        // A step, not a press: a radial slot's press opens its list.
        self.client.chat_ui.options.pad_clicks_step(true);
        if button == Button::DPadLeft {
            self.pad_with_modifier(KeyCode::ShiftLeft, |game| {
                game.pad_click_at(event_loop, focus.center(), false)
            });
        } else {
            self.pad_click_at(event_loop, focus.center(), false);
        }
        self.client.chat_ui.options.pad_clicks_step(false);
        true
    }

    /// Left and right are the focused slider's, after A took hold of it. Until then they move the
    /// focus, so passing over a slider doesn't change it.
    fn pad_holding_slider(&self) -> bool {
        let menu = &self.client.pad.0.menu;
        !menu.keyboard
            && self.pad_focus_mode()
            && menu.focus.is_some_and(|focus| {
                matches!(focus.kind, Kind::Slider { .. })
                    && menu.slider_hold == Some((self.key_screen(), focus.rect))
            })
    }

    /// Move a focused slider's handle by `by` GUI pixels (one pixel is
    /// vanilla's arrow-key step, `value + 1 / (width - 8)`), as a click there.
    fn pad_nudge(&mut self, event_loop: &dyn EventLoopCtl, focus: Target, by: f32) {
        let Kind::Slider { value, track } = focus.kind else {
            return;
        };
        let from = match self.client.pad.0.menu.nudge {
            Some((rect, x)) if rect == focus.rect => x,
            _ => gamepad::slider_x(track, value),
        };
        let x = (from + by).clamp(track[0], track[0] + track[1]);
        self.client.pad.0.menu.nudge = Some((focus.rect, x));
        self.pad_click_at(event_loop, (x, focus.center().1), false);
    }

    /// Step the focus that way: vanilla's arrow-key navigation; a step out of a
    /// list that can still scroll that way (`gamepad::list_step`) scrolls it
    /// and goes on once the next frame is drawn.
    fn pad_focus_move(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        button: Button,
        focus: Target,
        targets: &[Target],
    ) {
        // The focus goes on from where the scroll leaves it (a held direction's
        // repeat can come first).
        if self.client.pad.0.menu.pending.is_some() {
            return;
        }
        // Nor while a row scrolled into view: the targets still show it where it was, and a step
        // from them could land on its neighbour.
        if reveal_pending(self.client.pad.0.menu.revealed, widgets::generation()) {
            return;
        }
        let rects: Vec<Rect> = targets.iter().map(|t| t.rect).collect();
        let vertical = matches!(button, Button::DPadUp | Button::DPadDown);
        // A step the screen declared comes first (`widgets::focus_link`).
        let linked = vertical
            .then(|| widgets::linked_step(focus.rect, button == Button::DPadDown))
            .flatten()
            .and_then(|to| gamepad::refind(to, &rects, 0.0));
        if let Some(i) = linked {
            self.pad_focus_on(event_loop, targets[i]);
            return;
        }
        let areas = widgets::last_scroll_areas();
        let step = if vertical {
            gamepad::list_step(focus.rect, button, &rects, &areas)
        } else {
            gamepad::focus_step_rect(focus.rect, button, &rects)
        };
        // Outside the list a screen has, a step scrolls nothing (Back, Done).
        let outside = !areas.is_empty() && !areas.iter().any(|a| a.holds(&focus.rect));
        match step {
            Some(i) => self.pad_focus_on(event_loop, targets[i]),
            None if vertical && outside => {
                if let Some(i) = gamepad::focus_step_vague(focus.rect, button, &rects) {
                    self.pad_focus_on(event_loop, targets[i]);
                }
            }
            None if vertical => {
                // Off the visible rows of a list: scroll it and try again on
                // the next frame.
                self.pad_step_wheel(event_loop, button);
                self.client.pad.0.menu.pending = Some(Pending {
                    button,
                    from: focus.rect,
                    generation: widgets::generation(),
                    tries: 1,
                    before: rects,
                    amount: list_amount(&areas, &focus.rect),
                });
            }
            None => {}
        }
    }

    /// `AbstractSelectionList.setFocused`, on the lists where selecting does no more.
    fn pad_focus_selects_row(&mut self, target: Target) {
        if target.kind != Kind::Row {
            return;
        }
        let size = self.gui_size();
        if let Some(title) = self
            .client
            .title
            .0
            .as_mut()
            .filter(|t| t.focus_selects_row())
        {
            title.focus_world_row(target.center(), size);
        }
    }

    /// `AbstractSelectionList.setSelected`: a settings row the focus lands on, or presses, while
    /// its list hides part of it scrolls all into view first. Where it is now.
    fn pad_reveal(&mut self, target: Target) -> Target {
        if self.client.screen != Screen::Options || matches!(target.kind, Kind::Mounted { .. }) {
            return target;
        }
        let size = self.gui_size();
        let mut moved = target;
        if let Some(dy) =
            self.client
                .chat_ui
                .options
                .reveal(&self.client.settings, size, target.rect)
        {
            moved.rect[1] += dy;
            self.client.pad.0.menu.revealed = Some(widgets::generation());
        }
        moved
    }

    fn pad_focus_on(&mut self, event_loop: &dyn EventLoopCtl, target: Target) {
        let target = self.pad_reveal(target);
        self.client.pad.0.menu.shortcut_focus = None;
        let left = self.client.pad.0.menu.focus.replace(target);
        if left.is_some_and(|l| !l.same_focus_target(target)) {
            self.clear_screen_click_focus();
            self.client.pad.0.menu.armed = None;
        }
        self.sync_mounted_focus(target);
        self.client.pad.0.menu.pending = None;
        self.client.pad.0.menu.nudge = None;
        if !self.client.pad.0.menu.held[0] {
            self.pad_pointer_to(event_loop, target.center());
        }
        if self.client.pad.0.menu.keyboard {
            // The key itself was narrated (`narrate_input`).
            self.key_focus_landed(event_loop, left, target);
            return;
        }
        if left.is_none_or(|l| !l.same_focus_target(target)) {
            self.pad_focus_selects_row(target);
        }
        // The screen's narration hears it as an arrow key moving the focus.
        self.client.narration.set_input(InputType::KeyboardArrow);
        self.client.narration.keyboard_action();
    }

    /// The pad leaves screen `screen`: remember its focus there for when it
    /// comes back. Called before the new screen's targets are read, so the
    /// focus is still the left screen's.
    pub(in crate::game::client_side) fn pad_remember_focus(&mut self, screen: u64) {
        self.client.pad.0.menu.left_screen = widgets::generation();
        let Some(focus) = self.client.pad.0.menu.focus else {
            return;
        };
        if self.client.pad.0.menu.shortcut_focus.take() == Some(focus.rect) {
            return;
        }
        let name = self.pad_focus_name();
        let remembered = &mut self.client.pad.0.menu.remembered;
        if remembered.len() > 32 && !remembered.contains_key(&screen) {
            remembered.clear();
        }
        let rect = focus.rect;
        remembered.insert(screen, Remembered { rect, name });
    }

    /// Vanilla's arrow navigation moves a screen's focus off the widget a
    /// click focused: the screens that draw that click focus themselves drop
    /// it, so only the pad's focus shows. The screen it clears need not be
    /// the one in front (a screen that just opened leaves the old focus
    /// behind it), so every menu's click focus goes.
    pub(in crate::game::client_side) fn clear_screen_click_focus(&mut self) {
        if let Some(title) = self.client.title.0.as_mut()
            && !matches!(title.mode, sunrise_client::title::Mode::MissingMods { .. })
        {
            title.focused_button = None;
        }
        self.client.pause_focus = None;
        self.client.death_focus = None;
        self.client.chat_ui.options.clear_click_focus();
        if let Some(options) = self.client.world_options.0.as_mut() {
            options.clear_click_focus();
        }
    }

    /// `Gui.setScreen` clears the focus of the screen it leaves
    /// (`Screen.clearFocus`): the screen that opens starts with none, so the
    /// button a click focused is not still highlighted
    /// (`AbstractWidget.isHoveredOrFocused`) when the player comes back to it.
    /// The frame after another screen is front, `key_screen` names a new one.
    pub(in crate::game::client_side) fn drop_left_screen_focus(&mut self) {
        let screen = self.key_screen();
        if another_screen_is_front(&mut self.client.focus_screen, screen) {
            self.clear_screen_click_focus();
        }
    }

    /// The controller's focus ring in focus mode, for the screen's narration:
    /// the focused widget.
    pub(in crate::game::client_side) fn pad_focus_rect(&self) -> Option<[f32; 4]> {
        let focus = self
            .client
            .pad
            .0
            .menu
            .focus
            .filter(|_| self.pad_in_menu() && self.pad_focus_mode());
        focus.map(|t| t.rect)
    }

    /// A bumper: the next or previous tab, wrapping (creative keeps its own
    /// order; other screens register their tabs).
    fn pad_tab(&mut self, event_loop: &dyn EventLoopCtl, next: bool) {
        // The server list has no tabs: the bumpers move the selected server.
        if self
            .client
            .title
            .0
            .as_mut()
            .is_some_and(|title| title.multiplayer_move_selected(next))
        {
            return;
        }
        if self.client.screen == Screen::Creative && self.client.title.0.is_none() {
            self.pad_creative_tab(next);
            return;
        }
        // A held A is on a widget: a tab click would land in the middle of its press.
        if self.client.pad.0.menu.held[0] || self.client.pad.0.menu.armed.is_some() {
            return;
        }
        // A settings screen whose page list scrolls: every page in list order, those out of view
        // too, which have no tab on the screen.
        if self.client.screen == Screen::Options {
            let size = self.gui_size();
            if let Some(rect) =
                self.client
                    .chat_ui
                    .options
                    .step_page(&self.client.settings, size, next)
            {
                self.play_audio_button_click();
                if self.pad_focus_mode() {
                    self.client.pad.0.menu.focus = Some(Target {
                        rect,
                        kind: Kind::Tab(true),
                    });
                }
                return;
            }
        }
        let tabs: Vec<(Rect, bool)> = self
            .pad_targets()
            .into_iter()
            .filter_map(|t| match t.kind {
                Kind::Tab(selected) => Some((t.rect, selected)),
                _ => None,
            })
            .collect();
        let Some(i) = gamepad::tab_step(&tabs, next) else {
            return;
        };
        let rect = tabs[i].0;
        let back = self.client.pad.0.menu.pos;
        self.pad_click_at(
            event_loop,
            (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0),
            false,
        );
        if self.pad_focus_mode() {
            // The page changed under the old focus: start again on the new one.
            self.client.pad.0.menu.focus = Some(Target {
                rect,
                kind: Kind::Tab(true),
            });
        } else if let Some(at) = back {
            self.pad_pointer_to(event_loop, at);
        }
    }

    /// An inventory with its recipe book open and nothing over it: the book's tabs are targets
    /// only then (`recipe_book_targets`).
    pub(in crate::game::client_side) fn pad_recipe_book_showing(&self) -> bool {
        self.client.title.0.is_none()
            && self.client.screen == Screen::Inventory
            && self
                .recipe_book_targets()
                .iter()
                .any(|t| matches!(t.kind, Kind::Tab(_)))
    }

    /// Select the next or previous creative tab (wrapping), as clicking it does. A tab on
    /// another page of mods' tabs shows its page: clicking where it would be would hit the
    /// tab drawn there on this page.
    fn pad_creative_tab(&mut self, next: bool) {
        let count = self.client.creative_screen.0.tabs.len();
        if count == 0 {
            return;
        }
        let selected = self.client.creative_screen.0.selected_index();
        let tab = if next {
            (selected + 1) % count
        } else {
            (selected + count - 1) % count
        };
        super::select_creative_tab(
            &mut self.client.creative_screen.0,
            &mut self.client.inv_clicks,
            tab,
        );
        self.build_screen();
        self.refresh_saved_hotbars();
        self.client.inv_drag = None;
        self.client.last_inv_click = None;
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_pad_menu_frame(&mut self, event_loop: &dyn EventLoopCtl) {
        self.pad_menu_frame(event_loop);
    }

    /// Once per poll in a menu: the stick moves the cursor (pointer) or
    /// steps focus, the right stick scrolls, a held direction repeats, focus
    /// follows its widget from frame to frame, and the OS pointer hides
    /// while the pad drives.
    pub(in crate::game::client_side) fn pad_menu_frame(&mut self, event_loop: &dyn EventLoopCtl) {
        let now = Instant::now();
        let seconds = self
            .client
            .pad
            .0
            .menu
            .last_poll
            .map_or(0.0, |t| now.duration_since(t).as_secs_f32().min(0.05));
        self.client.pad.0.menu.last_poll = Some(now);
        self.key_focus_frame();
        let in_menu = self.pad_in_menu()
            && (self.client.settings.controller.enabled || self.client.pad.0.menu.keyboard);
        if !in_menu {
            let menu = &mut self.client.pad.0.menu;
            menu.keyboard = false;
            menu.slider_edit = false;
            menu.open_polls = 0;
            menu.pos = None;
            menu.repeat = None;
            menu.held = [false; 2];
            menu.focus = None;
            menu.pending = None;
            menu.stick_scrolled = None;
            menu.trigger = None;
            menu.anchors = None;
            menu.layout = None;
            menu.nudge = None;
            menu.osk = None;
            menu.osk_dismissed = false;
            menu.craft = None;
            menu.focus_name = None;
            menu.prompt = None;
            menu.prompt_restart = None;
            menu.prompt_crosshair = None;
            if self
                .client
                .screen_state
                .0
                .context_menu_input
                .has_pad_stick_or_repeat()
            {
                self.client
                    .screen_state
                    .0
                    .context_menu_input
                    .clear_pad_stick_and_repeat();
            }
            self.pad_show_os_pointer(true);
            return;
        }
        self.client.pad.0.menu.open_polls = self.client.pad.0.menu.open_polls.saturating_add(1);
        self.direct_craft_poll();
        // Once the focus or the screen moved on from a slider it's let go for good, or coming back
        // to it would find it still held.
        if !self.pad_holding_slider() {
            self.client.pad.0.menu.slider_hold = None;
        }
        // A real mouse move hands the pointer back. The keyboard's focus
        // stays where the next Tab or arrow goes on from.
        if let Some(at) = self.client.pad.0.menu.pos
            && ((at.0 - self.client.cursor.0).abs() > 0.5
                || (at.1 - self.client.cursor.1).abs() > 0.5)
        {
            self.client.pad.0.menu.pos = None;
            let focus = self.client.pad.0.menu.focus.take();
            if std::mem::take(&mut self.client.pad.0.menu.keyboard) {
                self.client.pad.0.menu.origin = focus.map(|t| t.rect);
            }
            self.pad_yield_input(event_loop);
        }
        // Focus lands on the menu as it opens when the pad was in use.
        if self.client.pad.0.in_use
            && self.client.pad.0.menu.pos.is_none()
            && self.client.pad.0.menu.open_polls == 3
            && !self.context_menu_open()
        {
            self.pad_take_pointer(event_loop);
        }
        if !self.client.pad.0.in_use && !self.client.pad.0.menu.keyboard {
            self.pad_show_os_pointer(true);
            return;
        }
        let focus_mode = self.pad_focus_mode();
        let targets = if focus_mode {
            self.pad_targets()
        } else {
            Vec::new()
        };
        if !self.context_menu_pad_active() {
            self.pad_menu_frame_as::<false>(event_loop, now, seconds, focus_mode, &targets);
        } else {
            self.pad_menu_frame_as::<true>(event_loop, now, seconds, focus_mode, &targets);
        }
        // Vanilla's keyboard focus leaves the mouse pointer alone.
        self.pad_show_os_pointer(
            self.client.pad.0.menu.pos.is_none() || self.client.pad.0.menu.keyboard,
        );
    }

    /// The inactive path checks for a newly opened context menu only after input callbacks.
    #[inline(always)]
    fn pad_menu_frame_as<const MENU: bool>(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        now: Instant,
        seconds: f32,
        focus_mode: bool,
        targets: &[Target],
    ) {
        let context_menu = if MENU {
            let open = self.context_menu_open();
            if !open {
                self.clear_context_menu_pad_state();
            }
            open
        } else {
            false
        };
        let focusing = focus_mode && !targets.is_empty();
        let resizing = self.pad_follow_resize(event_loop);
        let restarting = self.pad_prompt_frame(targets);
        if focusing
            && !context_menu
            && !resizing
            && !restarting
            && !self.key_focus_restart(event_loop, targets)
        {
            self.pad_follow_focus(event_loop, targets);
        }
        let (move_deadzone, look_deadzone) = (
            self.client.settings.controller.move_deadzone,
            self.client.settings.controller.look_deadzone,
        );
        let state = self.client.pad.0.state;
        let stick_suppressed = MENU
            && !context_menu
            && self.client.screen_state.0.context_menu_input.stick_owned()
            && {
                let left_radial = gamepad::radial(state.left, move_deadzone);
                if left_radial == [0.0, 0.0] {
                    self.client
                        .screen_state
                        .0
                        .context_menu_input
                        .release_stick();
                    self.client.pad.0.menu.stick = None;
                    false
                } else {
                    self.client.pad.0.menu.stick = gamepad::stick_direction_held(
                        left_radial,
                        STICK_STEP,
                        self.client.pad.0.menu.stick,
                    );
                    true
                }
            };
        if stick_suppressed {
            // Held left stick from the closed context menu stays suppressed until neutral.
        } else if focusing || self.client.pad.0.menu.osk.is_some() || context_menu {
            // The stick steps like the D-pad.
            let dir = gamepad::stick_direction_held(
                gamepad::radial(state.left, move_deadzone),
                STICK_STEP,
                self.client.pad.0.menu.stick,
            );
            if dir != self.client.pad.0.menu.stick {
                if let Some((b, ..)) = self.client.pad.0.menu.repeat
                    && Some(b) == self.client.pad.0.menu.stick
                {
                    self.client.pad.0.menu.repeat = None;
                    if MENU && context_menu {
                        self.client
                            .screen_state
                            .0
                            .context_menu_input
                            .take_pad_repeat_owned();
                    }
                }
                self.client.pad.0.menu.stick = dir;
                if let Some(button) = dir {
                    if self.client.pad.0.menu.pos.is_none()
                        && self.client.pad.0.menu.osk.is_none()
                        && !context_menu
                    {
                        self.pad_take_pointer(event_loop);
                    } else {
                        self.client.pad.0.menu.repeats = 0;
                        self.pad_step(event_loop, button);
                        if !MENU && self.client.screen_state.0.context_menu.is_some() {
                            return;
                        }
                    }
                    if MENU && context_menu {
                        self.client
                            .screen_state
                            .0
                            .context_menu_input
                            .own_pad_repeat();
                        self.client.screen_state.0.context_menu_input.own_stick();
                    }
                    self.client.pad.0.menu.repeat = Some((button, now, false));
                } else if MENU && context_menu {
                    self.client
                        .screen_state
                        .0
                        .context_menu_input
                        .release_stick();
                }
            }
        } else {
            self.client.pad.0.menu.stick = None;
            let tuning = &snap::TUNING;
            let mut speed = seconds * self.client.settings.controller.pointer_speed;
            if tuning.over_target_speed != 1.0 && self.pad_over_target() {
                speed *= tuning.over_target_speed;
            }
            let (dx, dy) = gamepad::cursor_delta(state.left, move_deadzone, speed);
            if dx != 0.0 || dy != 0.0 {
                self.client.pad.0.menu.glide = None;
                self.pad_take_pointer(event_loop);
                let at = self.client.cursor;
                self.client.pad.0.menu.gesture.get_or_insert(at);
                self.pad_pointer_to(event_loop, (at.0 + dx, at.1 + dy));
                self.client.pad.0.menu.moving = true;
            } else if self.client.pad.0.menu.moving {
                // Let go: the snapping engine picks where it settles.
                self.client.pad.0.menu.moving = false;
                let start = self
                    .client
                    .pad
                    .0
                    .menu
                    .gesture
                    .take()
                    .unwrap_or(self.client.cursor);
                let centres: Vec<(f32, f32)> =
                    self.pad_targets().iter().map(Target::center).collect();
                if let Some(i) = snap::settle(start, self.client.cursor, &centres, tuning) {
                    self.pad_glide_to(centres[i]);
                }
            } else if let Some((from, to, since)) = self.client.pad.0.menu.glide {
                let elapsed = now.duration_since(since).as_secs_f32();
                self.pad_pointer_to(event_loop, snap::glide(from, to, elapsed, tuning));
                if elapsed >= tuning.glide_seconds {
                    self.client.pad.0.menu.glide = None;
                }
            }
            if !resizing {
                self.pad_follow_targets();
            }
        }
        // An open context menu takes the right stick ahead of the on-screen keyboard.
        let scroll = if MENU && context_menu {
            let right = gamepad::radial(state.right, look_deadzone);
            if right != [0.0, 0.0]
                || (self.client.pad.0.menu.osk.is_some() && state.right[1].abs() >= 0.3)
            {
                self.client
                    .screen_state
                    .0
                    .context_menu_input
                    .own_right_stick();
                if right != [0.0, 0.0] {
                    self.client
                        .screen_state
                        .0
                        .context_menu_input
                        .own_pad_wheel();
                }
            } else if self
                .client
                .screen_state
                .0
                .context_menu_input
                .right_stick_owned()
                && state.right[1].abs() < 0.3
            {
                self.client
                    .screen_state
                    .0
                    .context_menu_input
                    .release_right_stick();
            }
            right
        } else if MENU
            && self
                .client
                .screen_state
                .0
                .context_menu_input
                .right_stick_owned()
        {
            if gamepad::radial(state.right, look_deadzone) == [0.0, 0.0]
                && state.right[1].abs() < 0.3
            {
                self.client
                    .screen_state
                    .0
                    .context_menu_input
                    .release_right_stick();
            }
            [0.0, 0.0]
        } else {
            let suggests = self.client.pad.0.menu.osk.is_some() && self.pad_chat_owns_keyboard();
            if let Some(keyboard) = self.client.pad.0.menu.osk.as_mut() {
                if let Some(out) = keyboard.stick(state.right[1], suggests) {
                    self.pad_osk_output(out);
                    if !MENU && self.client.screen_state.0.context_menu.is_some() {
                        return;
                    }
                }
                [0.0, 0.0]
            } else {
                gamepad::radial(state.right, look_deadzone)
            }
        };
        let wheel = &mut self.client.pad.0.menu.wheel;
        wheel[0] += scroll[0] * self.client.settings.controller.scroll_speed * seconds;
        wheel[1] += scroll[1] * self.client.settings.controller.scroll_speed * seconds;
        if scroll == [0.0, 0.0] {
            *wheel = [0.0, 0.0];
        }
        let lines = [wheel[0].trunc(), wheel[1].trunc()];
        if lines != [0.0, 0.0] {
            wheel[0] -= lines[0];
            wheel[1] -= lines[1];
            if MENU && context_menu {
                self.context_menu_wheel(lines[1]);
            } else {
                // Stick right scrolls right: the wheel's x runs the other way.
                self.pad_stick_wheel(event_loop, focus_mode, targets, [-lines[0], lines[1]]);
                if !MENU && self.client.screen_state.0.context_menu.is_some() {
                    return;
                }
            }
        }
        if let Some((button, since, repeating)) = self.client.pad.0.menu.repeat {
            if MENU
                && !context_menu
                && (self
                    .client
                    .screen_state
                    .0
                    .context_menu_input
                    .owns_pad(button)
                    || self.client.screen_state.0.context_menu_input.stick_owned())
            {
                self.client.pad.0.menu.repeat = None;
            } else {
                let wait = if repeating {
                    self.client.settings.controller.repeat_interval
                } else {
                    self.client.settings.controller.repeat_delay
                };
                if now.duration_since(since).as_secs_f32() >= wait {
                    self.client.pad.0.menu.repeat = Some((button, now, true));
                    self.client.pad.0.menu.repeats =
                        self.client.pad.0.menu.repeats.saturating_add(1);
                    self.pad_step(event_loop, button);
                    if !MENU && self.client.screen_state.0.context_menu.is_some() {
                        return;
                    }
                }
            }
        }
        if !context_menu {
            self.pad_auto_keyboard();
        }
    }

    /// Each poll: a prompt came up or went away. The keyboard was typing into what was open, and
    /// the focus starts again once the new frame shows: nothing pressed on one screen lands on
    /// the other. A keyboard a prompt took from chat or a sign opens again after it
    /// (`pad_auto_keyboard`), and the crosshair goes back to where it was. Whether the focus is
    /// waiting for that frame.
    fn pad_prompt_frame(&mut self, targets: &[Target]) -> bool {
        let prompt = self.pad_prompt_key();
        if prompt != self.client.pad.0.menu.prompt {
            let pointer_screen = self.pad_pointer_screen();
            let (screen, cursor) = (self.client.screen, self.client.cursor);
            let menu = &mut self.client.pad.0.menu;
            let opened = menu.prompt.is_none();
            menu.prompt = prompt;
            if menu.osk.take().is_some() && opened {
                menu.osk_dismissed = false;
            }
            if menu.keyboard {
                menu.prompt_crosshair = None;
            } else {
                if opened && pointer_screen && menu.pos.is_some() && menu.prompt_crosshair.is_none()
                {
                    let at = menu.glide.map_or(cursor, |(_, to, _)| to);
                    menu.prompt_crosshair = Some((screen, at));
                }
                menu.focus = None;
                menu.armed = None;
                menu.pending = None;
                menu.nudge = None;
                menu.stick_scrolled = None;
                menu.prompt_restart = Some(widgets::generation());
            }
        }
        let Some(changed_at) = self.client.pad.0.menu.prompt_restart else {
            return false;
        };
        // Two frames on: the first may have been drawn before it changed.
        if widgets::generation() <= changed_at + 1 {
            return true;
        }
        self.client.pad.0.menu.prompt_restart = None;
        if self.client.pad.0.menu.pos.is_some() && !targets.is_empty() {
            self.client.pad.0.menu.focus = self.pad_first_target(targets);
        }
        if self.client.pad.0.menu.prompt.is_none()
            && let Some((screen, at)) = self.client.pad.0.menu.prompt_crosshair.take()
            && screen == self.client.screen
            && self.client.pad.0.menu.pos.is_some()
            && !self.pad_focus_mode()
        {
            self.pad_glide_to(at);
        }
        false
    }

    /// Each poll: the screen's size changed (a window resize, the GUI scale), so
    /// once its new layout is drawn, the focus and the crosshair go on from the
    /// same widget. Too far for `pad_follow_focus` and `snap::follow`, which it
    /// holds off (true) until then.
    fn pad_follow_resize(&mut self, event_loop: &dyn EventLoopCtl) -> bool {
        let Some(batch) = self.gui_batch() else {
            return false;
        };
        let (screen, size) = (self.key_screen(), (batch.width, batch.height));
        let Some(mut layout) = self
            .client
            .pad
            .0
            .menu
            .layout
            .take()
            .filter(|l| l.screen == screen)
        else {
            self.client.pad.0.menu.layout = Some(Layout {
                screen,
                size,
                rects: self.pad_targets().iter().map(|t| t.rect).collect(),
                focus_name: self.pad_focus_name(),
                resized: None,
            });
            return false;
        };
        let generation = widgets::generation();
        if size == layout.size {
            layout.resized = None;
            layout.rects = self.pad_targets().iter().map(|t| t.rect).collect();
            layout.focus_name = self.pad_focus_name();
            self.client.pad.0.menu.layout = Some(layout);
            return false;
        }
        if layout.resized.is_none_or(|(seen, _)| seen != size) {
            layout.resized = Some((size, generation));
        }
        // Two frames on: the first may have been drawn before the size changed.
        if layout.resized.is_some_and(|(_, at)| generation <= at + 1) {
            self.client.pad.0.menu.layout = Some(layout);
            return true;
        }
        let targets = self.pad_targets();
        let rects: Vec<Rect> = targets.iter().map(|t| t.rect).collect();
        let (before, old) = (std::mem::take(&mut layout.rects), layout.size);
        let found = |from: Rect| gamepad::refind_resized(from, &before, &rects, old, size);
        self.client.pad.0.menu.pending = None;
        self.client.pad.0.menu.nudge = None;
        self.client.pad.0.menu.anchors = None;
        if self.pad_focus_mode() {
            let named = layout
                .focus_name
                .take()
                .and_then(|name| named_target(&name, &widgets::last_labelled(), &rects));
            self.client.pad.0.menu.focus = named
                .or_else(|| {
                    self.client
                        .pad
                        .0
                        .menu
                        .focus
                        .and_then(|focus| found(focus.rect))
                })
                .map(|i| targets[i]);
        } else if self.client.pad.0.menu.pos.is_some() && self.client.pad.0.menu.held == [false; 2]
        {
            let at = self.client.cursor;
            let on = before.iter().find(|r| {
                (r[0] + r[2] / 2.0 - at.0).abs() <= 0.5 && (r[1] + r[3] / 2.0 - at.1).abs() <= 0.5
            });
            let to = match on.and_then(|r| found(*r)) {
                Some(i) => targets[i].center(),
                None => gamepad::rescaled(at, old, size),
            };
            self.client.pad.0.menu.glide = None;
            self.client.pad.0.menu.gesture = None;
            self.pad_pointer_to(event_loop, to);
        }
        self.client.pad.0.menu.layout = Some(Layout {
            screen,
            size,
            rects,
            focus_name: None,
            resized: None,
        });
        false
    }

    /// What the focused widget is called, when it has a name of its own on this screen.
    fn pad_focus_name(&self) -> Option<String> {
        let [x, y, w, h] = self.client.pad.0.menu.focus?.rect;
        widgets::last_labelled()
            .iter()
            .find(|l| {
                (l.x - x).abs() < 0.5
                    && (l.y - y).abs() < 0.5
                    && (l.w - w).abs() < 0.5
                    && (l.h - h).abs() < 0.5
            })
            .map(|l| widgets::slug(&l.label))
            .filter(|name| !name.is_empty())
    }

    /// The focused widget where its name says it is now, on the screen and from the focus the
    /// name was read from. The keyboard's focus keeps to the rectangles it always followed.
    fn pad_named_focus(&self, focus: Target, rects: &[Rect]) -> Option<usize> {
        let named = self.client.pad.0.menu.focus_name.as_ref().filter(|n| {
            !self.client.pad.0.menu.keyboard
                && n.rect == focus.rect
                && n.screen == self.key_screen()
        })?;
        named_target(&named.name, &widgets::last_labelled(), rects)
    }

    /// Pointer mode, each poll: the targets moved out from under a resting
    /// cursor (a recipe book opened, another creative tab): it goes with them.
    fn pad_follow_targets(&mut self) {
        // A press can move them (the recipe book opens on the way down): the
        // cursor follows once it's let go, from the layout it pressed on.
        if self.client.pad.0.menu.held != [false; 2] {
            return;
        }
        let centres: Vec<(f32, f32)> = self.pad_targets().iter().map(Target::center).collect();
        let before = self
            .client
            .pad
            .0
            .menu
            .anchors
            .replace((self.client.screen, centres.clone()))
            .filter(|(screen, _)| *screen == self.client.screen);
        let menu = &self.client.pad.0.menu;
        if menu.pos.is_none() || menu.moving || menu.glide.is_some() {
            return;
        }
        if let Some((_, before)) = before
            && let Some(i) = snap::follow(self.client.cursor, &before, &centres)
        {
            self.pad_glide_to(centres[i]);
        }
    }

    /// Focus mode, each poll: find the focused widget in the new frame (a
    /// list scrolled, a page changed), finish a step that scrolled a list,
    /// and keep the hidden pointer on the focused widget.
    fn pad_follow_focus(&mut self, event_loop: &dyn EventLoopCtl, targets: &[Target]) {
        let rects: Vec<Rect> = targets.iter().map(|t| t.rect).collect();
        let screen = self.key_screen();
        let stick_scrolled = self
            .client
            .pad
            .0
            .menu
            .stick_scrolled
            .take_if(|s| widgets::generation() > s.generation)
            .filter(|s| s.screen == screen);
        let ready = self
            .client
            .pad
            .0
            .menu
            .pending
            .as_ref()
            // Two frames on: the first may have been drawn before the scroll.
            .map(|p| widgets::generation() > p.generation + 1);
        if ready == Some(true)
            && let Some(p) = self.client.pad.0.menu.pending.take()
        {
            // A step scrolled a list: carry on from where the focused row moved
            // (still leaving a list with more to show, it scrolls again), or
            // take the step it would have taken when nothing scrolled.
            let areas = widgets::last_scroll_areas();
            let shift = list_shift(p.amount, list_amount(&areas, &p.from)).unwrap_or_else(|| {
                gamepad::scroll_shift(&p.before, &rects, p.button == Button::DPadDown)
            });
            let mut from = p.from;
            let step = if let Some(dy) = shift {
                from[1] += dy;
                self.client.pad.0.menu.focus =
                    gamepad::refind(from, &rects, 2.0).map(|i| targets[i]);
                gamepad::list_step(from, p.button, &rects, &areas)
            } else {
                gamepad::focus_step_rect(p.from, p.button, &rects)
            };
            match step {
                Some(i) => self.pad_focus_on(event_loop, targets[i]),
                None if shift.is_some() && p.tries < SCROLL_TRIES => {
                    self.pad_step_wheel(event_loop, p.button);
                    self.client.pad.0.menu.pending = Some(Pending {
                        from,
                        generation: widgets::generation(),
                        tries: p.tries + 1,
                        before: rects.clone(),
                        amount: list_amount(&areas, &from),
                        ..p
                    });
                }
                None => {}
            }
        } else if ready.is_none()
            // Two frames on, as after a step that scrolled: the first may predate the scroll.
            && !reveal_pending(self.client.pad.0.menu.revealed, widgets::generation())
        {
            // A settings page that says where its focus opens (a radial slot's list on the slot's
            // action, the slot again after it), once its row is drawn.
            let opened = (self.client.screen == Screen::Options)
                .then(|| self.client.chat_ui.options.shell_opened_focus())
                .flatten()
                .and_then(|rect| gamepad::refind(rect, &rects, 2.0));
            if let Some(i) = opened {
                self.client.chat_ui.options.clear_shell_opened_focus();
                self.client.pad.0.menu.focus = Some(targets[i]);
            } else if let Some(focus) = self.client.pad.0.menu.focus {
                let kind = std::mem::discriminant(&focus.kind);
                let named = self.pad_named_focus(focus, &rects);
                let followed = stick_scrolled.and_then(|s| {
                    // Its scrollbar knows how far, when the rows land on each other's places.
                    let areas = widgets::last_scroll_areas();
                    if let Some(Some(dy)) = list_shift(s.amount, list_amount(&areas, &focus.rect)) {
                        let mut moved = focus.rect;
                        moved[1] += dy;
                        if let Some(i) = gamepad::refind(moved, &rects, 0.5) {
                            return Some(i);
                        }
                    }
                    let after: Vec<(Rect, bool)> = targets
                        .iter()
                        .map(|t| (t.rect, std::mem::discriminant(&t.kind) == kind))
                        .collect();
                    gamepad::follow_scroll(focus.rect, s.down, &s.before, &after)
                });
                // The widget itself, wherever it went; else where its rectangle says.
                self.client.pad.0.menu.focus = if matches!(focus.kind, Kind::Mounted { .. }) {
                    targets.iter().copied().find(|t| t.same_focus_target(focus))
                } else {
                    named
                        .or(followed)
                        .filter(|&i| !matches!(targets[i].kind, Kind::Mounted { .. }))
                        .map(|i| targets[i])
                        .or_else(|| targets.iter().copied().find(|t| t.same_focus_target(focus)))
                        .or_else(|| {
                            gamepad::refind(focus.rect, &rects, 12.0)
                                .map(|i| targets[i])
                                .filter(|t| !matches!(t.kind, Kind::Mounted { .. }))
                        })
                };
                if self.client.pad.0.menu.focus.is_none() && self.client.pad.0.menu.pos.is_some() {
                    // The widget went away on this screen (a Reset or an Undo took itself away
                    // when pressed): what's beside it. Two frames on, as the first may have been
                    // drawn before the screen changed.
                    let next_to = (!self.client.pad.0.menu.keyboard
                        && widgets::generation() > self.client.pad.0.menu.left_screen + 1)
                        .then(|| beside(focus.rect, &rects))
                        .flatten();
                    // Else a page changed: start again.
                    let first = match next_to {
                        Some(i) => Some(targets[i]),
                        None => self.pad_first_target(targets),
                    };
                    if self.client.pad.0.menu.keyboard
                        && let Some(first) = first
                    {
                        // Screen.setInitialFocus after a keyboard press.
                        self.pad_focus_on(event_loop, first);
                    } else {
                        self.client.pad.0.menu.focus = first;
                    }
                }
            } else if self.client.pad.0.menu.pos.is_some() {
                // The pad drives but nothing is focused (switched from the pointer).
                let centres: Vec<(f32, f32)> = targets.iter().map(Target::center).collect();
                self.client.pad.0.menu.focus = if self.prompt_open() {
                    self.pad_first_target(targets)
                } else {
                    gamepad::nearest(self.client.cursor, &centres, f32::INFINITY)
                        .map(|i| targets[i])
                };
            }
        }
        self.client.pad.0.menu.focus_name = self.client.pad.0.menu.focus.and_then(|focus| {
            let name = self.pad_focus_name()?;
            let (screen, rect) = (self.key_screen(), focus.rect);
            Some(FocusName { screen, rect, name })
        });
        let Some(focus) = self.client.pad.0.menu.focus else {
            return;
        };
        // Includes focus restored or followed directly above, not only explicit
        // pad_focus_on landings. The pad now owns the screen's focus mark.
        if !self.client.pad.0.menu.keyboard
            && let Some(title) = self.client.title.0.as_mut()
        {
            title.missing_mods_pad_focus();
        }
        if self.client.pad.0.menu.pos.is_none() || self.client.pad.0.menu.held.iter().any(|h| *h) {
            return;
        }
        let at = match self.client.pad.0.menu.nudge {
            Some((rect, x)) if rect == focus.rect => (x, focus.center().1),
            _ => focus.center(),
        };
        // A text field the keyboard focused keeps the pointer where its
        // click put the caret.
        let [x, y, w, h] = focus.rect;
        let inside = (x..x + w).contains(&self.client.cursor.0)
            && (y..y + h).contains(&self.client.cursor.1);
        if self.client.pad.0.menu.keyboard
            && matches!(focus.kind, Kind::Field | Kind::Mounted { field: true, .. })
            && inside
        {
            return;
        }
        if (at.0 - self.client.cursor.0).abs() > 0.5 || (at.1 - self.client.cursor.1).abs() > 0.5 {
            self.pad_pointer_to(event_loop, at);
        }
    }

    fn pad_show_os_pointer(&mut self, show: bool) {
        // While the game's grab is internal the system pointer stays as it is; the first poll
        // after shows or hides it.
        if self.client.pad.0.menu.os_hidden != show || self.internal_grab() {
            return;
        }
        self.client.pad.0.menu.os_hidden = !show;
        if let Some(window) = &self.client.window.0 {
            window.set_cursor_visible(show && !self.client.captured);
        }
    }

    /// `pad status`: the menu mode, the focused target and every target.
    pub(in crate::game::client_side) fn pad_menu_status(&self) -> serde_json::Value {
        let kind = |k: Kind| match k {
            Kind::Button | Kind::Mounted { field: false, .. } => "button",
            Kind::Slider { .. } => "slider",
            Kind::Field | Kind::Mounted { field: true, .. } => "field",
            Kind::Row => "row",
            Kind::Slot => "slot",
            Kind::Tab(true) => "tab*",
            Kind::Tab(false) => "tab",
        };
        let target = |t: &Target| serde_json::json!({ "rect": t.rect, "kind": kind(t.kind) });
        serde_json::json!({
            "mode": if self.pad_focus_mode() { "focus" } else { "pointer" },
            "focus": self.client.pad.0.menu.focus.as_ref().map(target),
            "holding_slider": self.pad_holding_slider(),
            "pointer": self.client.pad.0.menu.pos.map(|_| self.client.cursor),
            "keyboard": self.client.pad.0.menu.osk.is_some(),
            "crafting": self.client.pad.0.menu.craft.is_some(),
            "craft_hint": self.direct_craft_usable(),
            "targets": self.pad_targets().iter().map(target).collect::<Vec<_>>(),
            "actions": self.pad_actions_status(),
            "glyphs": self.pad_glyphs_status(),
            "undeclared": self.pad_undeclared_buttons(),
            "screen_key": format!("{:016x}", self.key_screen()),
            "remembered": self
                .client.pad.0
                .menu
                .remembered
                .iter()
                .map(|(k, r)| (format!("{k:016x}"), serde_json::json!(r.rect)))
                .collect::<serde_json::Map<_, _>>(),
        })
    }

    // ---- the keyboard: vanilla's focus navigation (`key_focus`) ----

    pub(in crate::game::client_side) fn pad_cancel_menu(&mut self, event_loop: &dyn EventLoopCtl) {
        for right in [false, true] {
            if self.client.pad.0.menu.held[usize::from(right)] {
                self.pad_pointer_button(event_loop, right, false);
            }
        }
        self.client.pad.0.menu.repeat = None;
        self.client.pad.0.menu.stick = None;
        self.client.pad.0.menu.wheel = [0.0; 2];
        self.client.pad.0.menu.moving = false;
        self.client.pad.0.menu.trigger = None;
        self.client.pad.0.menu.glide = None;
        self.client.pad.0.menu.gesture = None;
        self.client.pad.0.menu.pending = None;
        self.client.pad.0.menu.nudge = None;
    }

    /// The keyboard drives the open menu's focus.
    pub(in crate::game::client_side) fn key_focus_driving(&self) -> bool {
        self.client.pad.0.menu.keyboard && self.key_focus_screen()
    }

    /// A pointer event the keyboard sends (a press, the hidden pointer on
    /// the focused widget) is being handled: the screen's narration doesn't
    /// take it for the mouse (`InputType` stays the keyboard's).
    pub(in crate::game::client_side) fn key_focus_quiet(&self) -> bool {
        self.client.pad.0.menu.synthetic && self.client.pad.0.menu.keyboard
    }

    /// The screen keeps the navigation keys for itself: typing in chat (Tab
    /// completes a command), a sign, a book, the creative, recipe-book and
    /// mods searches, the anvil's name, a command block, a key or
    /// button being bound, the credits, the on-screen keyboard.
    fn key_focus_blocked(&self) -> bool {
        if self.pad_keyboard_captures() {
            return true;
        }
        if self.observed_overlay_now() == Some("dialog") {
            return false;
        }
        let playing_screen = |screen| self.client.title.0.is_none() && self.client.screen == screen;
        playing_screen(Screen::Chat)
            || self.client.sign_edit.is_some()
            || matches!(
                self.client.book,
                Some(super::books::BookScreen::Edit { .. })
            )
            || (playing_screen(Screen::Creative) && self.client.creative_screen.0.tab().search)
            || self.recipe_search_focused()
            || self.mod_browser_typing()
            || self.station_typing()
            || self.client.screen == Screen::CommandBlock
            || self.client.screen == Screen::Win
            || (self.client.screen == Screen::Options
                && (self.options_waiting_for_key()
                    || self.client.chat_ui.options.selected_pad_action.is_some()))
    }

    /// `Screen.clearFocus`, which `Gui.setScreen` runs on the screen it takes
    /// down: the keyboard's focus belongs to that screen. A key bound to the
    /// container's own close key can move the focus and close the screen in
    /// one press (Tab bound to the inventory), and vanilla clears the focus
    /// before the next frame, so nothing stays highlighted.
    pub fn pad_clear_focus(&mut self) {
        self.client.pad.0.menu.clear_focus();
    }

    /// A key pressed in a menu, before the screen takes it
    /// (`Screen.keyPressed`): Tab, Shift+Tab and the arrows move the focus,
    /// Enter, Space and keypad Enter press the focused widget; the focused
    /// widget's own keys come first (a text field's caret, a slider's
    /// value). Whether the key was used; when not, the screen has it (a
    /// container also has Tab and the arrows once they moved the focus).
    pub fn key_focus_key(&mut self, event_loop: &dyn EventLoopCtl, key: KeyCode) -> Option<bool> {
        if self
            .client
            .title
            .0
            .as_ref()
            .is_some_and(|t| matches!(t.mode, sunrise_client::title::Mode::MissingMods { .. }))
        {
            return None;
        }
        let nav = key_focus::nav_of(key, self.has_shift_down())?;
        if !self.key_focus_screen()
            || (self.key_focus_blocked() && !self.client.screen_state.0.widgets.has_focus())
        {
            return None;
        }
        let dialog = self.observed_overlay_now() == Some("dialog");
        let mut targets = self.key_targets();
        if targets.is_empty() {
            return None;
        }
        if dialog
            && matches!(nav, Nav::Tab { .. })
            && let Some(order) = self.dialog_navigation_order()
        {
            targets.sort_by_key(|target| {
                if matches!(target.kind, Kind::Mounted { .. }) {
                    order.len() + 1
                } else {
                    order
                        .iter()
                        .position(|rect| *rect == target.rect)
                        .unwrap_or(order.len())
                }
            });
        }
        let mounted = self
            .client
            .screen_state
            .0
            .widgets
            .focused_id()
            .map(|id| id.raw());
        // The focus: the keyboard's or the controller's, else what a click
        // focused (a slider a click focused changes with the arrows).
        let mut slider_edit = self.client.pad.0.menu.slider_edit;
        let mut focus = if let Some(id) = mounted {
            targets
                .iter()
                .copied()
                .find(|t| matches!(t.kind, Kind::Mounted { id: candidate, .. } if candidate == id))
        } else {
            self.client
                .pad
                .0
                .menu
                .focus
                .filter(|_| self.pad_focus_mode())
                .and_then(|focus| targets.iter().copied().find(|t| t.same_focus_target(focus)))
        };
        if focus.is_none()
            && mounted.is_none()
            && dialog
            && let Some(rect) = self.dialog_navigation_focus()
        {
            focus = targets
                .iter()
                .copied()
                .find(|target| !matches!(target.kind, Kind::Mounted { .. }) && target.rect == rect);
        }
        if focus.is_none()
            && let Some(origin) = self.client.pad.0.menu.origin
            && let Some(clicked) = targets.iter().copied().find(|t| {
                !matches!(t.kind, Kind::Mounted { .. })
                    && t.rect.iter().zip(origin).all(|(a, b)| (a - b).abs() < 0.5)
            })
        {
            focus = Some(clicked);
            slider_edit = true;
        }
        let ctx = key_focus::Context {
            slider_edit,
            typing: self.edit_box_focused(),
            rows_take_enter: self
                .client
                .title
                .0
                .as_ref()
                .is_some_and(|t| t.list_focused()),
            gui: self.gui_size(),
        };
        let act = if dialog && matches!(nav, Nav::Tab { .. }) {
            let warning = self.dialog_warning_icon();
            key_focus::act_with_tab_groups(nav, focus, &targets, ctx, |target| {
                if !matches!(target.kind, Kind::Mounted { .. }) && Some(target.rect) == warning {
                    -10
                } else {
                    0
                }
            })
        } else {
            key_focus::act(nav, focus, &targets, ctx)
        };
        if act == Act::Pass {
            return None;
        }
        if !self.client.pad.0.menu.keyboard {
            self.client.pad.0.menu.key_screen = self.key_screen();
            self.client.pad.0.menu.key_restart = None;
        }
        let menu = &mut self.client.pad.0.menu;
        menu.keyboard = true;
        menu.origin = None;
        menu.focus = focus;
        menu.slider_edit = slider_edit;
        if menu.pos.is_none() {
            menu.pos = Some(self.client.cursor);
        }
        let mut handled = !key_focus::moves_focus(act);
        match (act, focus) {
            (Act::Focus(i), _) => {
                self.pad_focus_on(event_loop, targets[i]);
                self.client.pad.0.menu.slider_edit = key_focus::slider_edit_on_focus(nav);
            }
            (Act::Step(button), Some(focus)) => {
                handled = !matches!(focus.kind, Kind::Mounted { .. })
                    && self.pad_step_value(event_loop, button, focus);
                if !handled {
                    self.pad_focus_move(event_loop, button, focus, &targets);
                }
            }
            (Act::Press, Some(focus)) => {
                let focus = self.pad_reveal(focus);
                self.client.pad.0.menu.focus = Some(focus);
                // AbstractButton.keyPressed: onPress, with its click sound.
                self.pad_click_at(event_loop, focus.center(), false);
            }
            (Act::ToggleSlider, _) => self.client.pad.0.menu.slider_edit = !slider_edit,
            (Act::Nudge(sign), Some(focus)) => self.pad_nudge(event_loop, focus, sign),
            _ => {}
        }
        // The narration reads a focused slider's `canChangeValue` from the
        // input type (Tab: it can, an arrow: it can't); keep them in step
        // after Enter flipped it or a click let it.
        if let Some(Target {
            kind: Kind::Slider { .. },
            ..
        }) = self.client.pad.0.menu.focus
        {
            self.client
                .narration
                .set_input(if self.client.pad.0.menu.slider_edit {
                    InputType::KeyboardTab
                } else {
                    InputType::KeyboardArrow
                });
        }
        Some(handled)
    }

    /// Which screen is open, as far as the keyboard's focus cares.
    pub fn key_screen(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::mem::discriminant(&self.client.screen).hash(&mut hasher);
        self.remote_menu_id().hash(&mut hasher);
        if self.client.screen == Screen::Options {
            std::mem::discriminant(&self.client.chat_ui.options.page()).hash(&mut hasher);
        }
        self.client
            .title
            .0
            .as_ref()
            .map(|t| t.screen_key())
            .hash(&mut hasher);
        (self.client.sign_edit.is_some(), self.client.book.is_some()).hash(&mut hasher);
        self.pad_prompt_key().hash(&mut hasher);
        hasher.finish()
    }

    /// Each poll while the keyboard has the focus: another screen opened
    /// (a pressed button opened it), so once it is drawn the focus starts on
    /// its first widget (`Screen.setInitialFocus` after a key). Whether the
    /// focus is waiting for that or just started again.
    fn key_focus_restart(&mut self, event_loop: &dyn EventLoopCtl, targets: &[Target]) -> bool {
        if !self.client.pad.0.menu.keyboard {
            return false;
        }
        let screen = self.key_screen();
        if screen != self.client.pad.0.menu.key_screen {
            self.client.pad.0.menu.key_screen = screen;
            self.client.pad.0.menu.key_restart = Some(widgets::generation());
            self.client.pad.0.menu.pending = None;
        }
        let Some(changed_at) = self.client.pad.0.menu.key_restart else {
            return false;
        };
        // Two frames on: the first may have been drawn before the click.
        if widgets::generation() > changed_at + 1 {
            self.client.pad.0.menu.key_restart = None;
            self.client.pad.0.menu.focus = None;
            if let Some(first) = self.pad_first_target(targets) {
                self.pad_focus_on(event_loop, first);
            }
        }
        true
    }

    /// The keyboard's focus moved from `left` to `target`
    /// (`ComponentPath.applyFocus`): a slider's arrows don't change it yet,
    /// a text field takes the typing (a click at its end, where Tab leaves
    /// the caret) and the one left behind lets go of it, and a world row is
    /// selected (`AbstractSelectionList.setFocused`).
    fn key_focus_landed(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        left: Option<Target>,
        target: Target,
    ) {
        self.client.pad.0.menu.slider_edit = false;
        if left.is_some_and(|l| l.same_focus_target(target)) {
            return;
        }
        if self.observed_overlay_now() == Some("dialog") {
            self.set_dialog_navigation_focus(target);
            return;
        }
        if left.is_some_and(|l| l.kind == Kind::Field)
            && target.kind != Kind::Field
            && let Some(title) = self.client.title.0.as_mut()
        {
            title.blur_field();
        }
        match target.kind {
            Kind::Field => {
                self.pad_click_at(event_loop, key_focus::field_click(target.rect), false);
            }
            Kind::Row => {
                let size = self.gui_size();
                if let Some(title) = self.client.title.0.as_mut() {
                    title.focus_world_row(target.center(), size);
                }
            }
            _ => {}
        }
    }

    /// The real mouse, before the screen takes it: where it is, and a press
    /// or a scroll hands it the pointer back from the keyboard, at the
    /// place the mouse left it (`setLastInputType(MOUSE)`).
    pub(in crate::game::client_side) fn key_focus_mouse(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        event: &WindowEvent,
        sample: Option<PointerSample>,
    ) {
        if self.client.pad.0.menu.synthetic {
            return;
        }
        let takeover = match event {
            // Not while the pad holds the radial menu open: as for raw mouse motion in play
            // (`device_event`), a cursor that moved is a bumped mouse.
            WindowEvent::CursorMoved { .. } => {
                self.client
                    .pad
                    .0
                    .menu
                    .real_pointer
                    .map(PointerSample::screen_position)
                    != sample.map(PointerSample::screen_position)
                    && !self.pad_radial_captures()
            }
            WindowEvent::KeyboardInput { event, .. } => event.state == ElementState::Pressed,
            WindowEvent::MouseInput { state, .. } => *state == ElementState::Pressed,
            WindowEvent::MouseWheel { .. } => true,
            _ => false,
        };
        if takeover {
            self.pad_yield_input(event_loop);
        }
        match event {
            WindowEvent::CursorMoved { .. } => self.client.pad.0.menu.real_pointer = sample,
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            } => self.key_focus_to_mouse(event_loop),
            WindowEvent::MouseWheel { .. } => self.key_focus_to_mouse(event_loop),
            _ => {}
        }
    }

    /// The first built-in child under the pointer. A previous screen's frame
    /// must not keep its bounds in the new screen's input order.
    pub(in crate::game::client_side) fn native_pointer_child(&self) -> Option<Target> {
        if !widgets::last_input_screen_is(self.key_screen()) {
            return None;
        }
        let inventory = self.observed_overlay_now().is_none()
            && matches!(self.client.screen, Screen::Inventory | Screen::Creative);
        // The built-in children a mod removed at `ScreenEvent.Init.Post`: last frame may still
        // have drawn them, and a replacement at their bounds takes their presses.
        let removed: Vec<[f32; 4]> = match self.client.title.0.as_ref() {
            _ if self.observed_overlay_now().is_some() => Vec::new(),
            _ if self.client.screen == Screen::Paused => self
                .pause_init_children()
                .into_iter()
                .filter(|(action, _, _)| self.pause_child_removed(*action))
                .map(|(_, rect, _)| rect)
                .collect(),
            Some(title)
                if self.client.screen == Screen::Playing && title.has_removed_children() =>
            {
                let [w, h] = self.gui_frame_now().1.size.map(|axis| axis as f32);
                title
                    .init_children(&self.client.gui.atlas, w, h)
                    .into_iter()
                    .filter(|(id, _, _)| title.child_removed(*id))
                    .map(|(_, rect, _)| rect)
                    .collect()
            }
            _ => Vec::new(),
        };
        let (cx, cy) = self.client.cursor;
        key_focus::keyboard_targets(key_focus::drawn_targets(), inventory)
            .into_iter()
            .find(|t| {
                let [x, y, w, h] = t.rect;
                cx >= x
                    && cy >= y
                    && cx < x + w
                    && cy < y + h
                    && !removed
                        .iter()
                        .any(|r| r.iter().zip(t.rect).all(|(a, b)| (a - b).abs() < 0.5))
            })
    }

    pub(in crate::game::client_side) fn focus_native_pointer_child(&mut self, child: Target) {
        if !self.client.pad.0.menu.synthetic && self.key_focus_screen() {
            self.clear_mounted_widget_focus();
            self.client.pad.0.menu.origin = Some(child.rect);
        }
    }

    /// The keyboard lets go of the pointer: it goes back to the real mouse.
    /// Widget hover uses the real mouse while keyboard navigation moves the virtual pointer.
    pub(in crate::game::client_side) fn widget_tooltip_cursor(&self) -> (f32, f32) {
        if !self.key_focus_driving() {
            return self.client.cursor;
        }
        self.client
            .pad
            .0
            .menu
            .real_pointer
            .zip(self.renderer_pointer_projection())
            .map_or((0.0, 0.0), |(sample, projection)| {
                projection.project_pointer(sample).subpixel()
            })
    }

    fn key_focus_to_mouse(&mut self, event_loop: &dyn EventLoopCtl) {
        if !std::mem::take(&mut self.client.pad.0.menu.keyboard) {
            return;
        }
        let menu = &mut self.client.pad.0.menu;
        menu.origin = menu.focus.take().map(|t| t.rect);
        menu.pos = None;
        menu.pending = None;
        menu.nudge = None;
        let real = menu.real_pointer;
        if let (Some(sample), Some(id)) = (real, self.client.window.0.as_ref().map(|w| w.id())) {
            self.pad_synthetic_event(
                event_loop,
                id,
                WindowEvent::CursorMoved {
                    device_id: DeviceId::dummy(),
                    position: self.pointer_space([1, 1]).physical_position(sample),
                },
                Some(sample),
            );
        }
    }

    /// Each poll: what a click focused is forgotten once it is gone (the
    /// screen changed), and out of menus.
    fn key_focus_frame(&mut self) {
        let Some(origin) = self.client.pad.0.menu.origin else {
            return;
        };
        let there = self.key_focus_screen()
            && self
                .key_targets()
                .iter()
                .any(|t| t.rect.iter().zip(origin).all(|(a, b)| (a - b).abs() < 0.5));
        if !there {
            self.client.pad.0.menu.origin = None;
        }
    }

    /// The keyboard focus the title had while resources loaded
    /// (`StartupMenu`), carried into the game's.
    pub(in crate::game::client_side) fn key_focus_adopt(
        &mut self,
        focus: Target,
        slider_edit: bool,
    ) {
        let screen = self.key_screen();
        let menu = &mut self.client.pad.0.menu;
        menu.key_screen = screen;
        menu.key_restart = None;
        menu.keyboard = true;
        menu.focus = Some(focus);
        menu.slider_edit = slider_edit;
        menu.pos = Some(self.client.cursor);
    }

    // ---- text fields ----

    /// Whether the screen under the keyboard can still be typed in.
    fn pad_has_typing_screen(&self) -> bool {
        self.text_input_focused()
            || self.title_input_active()
            || matches!(self.client.screen, Screen::Options | Screen::WorldOptions)
    }

    /// Chat's suggestion controls belong to its editor, not an overlay or a mounted field.
    fn pad_chat_owns_keyboard(&self) -> bool {
        self.client.screen == Screen::Chat
            && self.client.title.0.is_none()
            && self.client.chat_ui.chat_screen.is_some()
            && !self.screen_over_playing()
            && !self.mounted_widget_captures_input()
    }

    /// Enter on these goes on to a new line: a dialog's, a sign's or a book's page.
    fn pad_enter_keeps_typing(&self) -> bool {
        !self.mounted_widget_captures_input()
            && (self.dialog_typing_lines()
                || self.client.sign_edit.is_some()
                || matches!(
                    self.client.book,
                    Some(super::books::BookScreen::Edit { signing: false, .. })
                ))
    }

    /// Open the on-screen keyboard over the focused field: Sunrise's, or
    /// Steam's on a Steam Deck (`steam://open/keyboard`, which types into
    /// the window like a real keyboard).
    fn pad_open_keyboard(&mut self) {
        // Without xdg-open (macOS, Windows), Sunrise's keyboard instead.
        if gamepad::steam_keyboard(self.client.settings.controller.keyboard)
            && std::process::Command::new("xdg-open")
                .arg("steam://open/keyboard")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .is_ok()
        {
            return;
        }
        self.client.pad.0.menu.osk = Some(Keyboard::new(self.pad_enter_keeps_typing()));
    }

    /// Chat or a sign opened by the pad (either menu mode): the keyboard
    /// opens with it (once per screen; B closes it for good). Not the chat a
    /// sleeper gets (`InBedChatScreen`): the game opens that one, not the
    /// player, so the keyboard waits for A on its chat box.
    fn pad_auto_keyboard(&mut self) {
        let menu = &self.client.pad.0.menu;
        if osk_auto_opens(
            menu.osk.is_some(),
            menu.osk_dismissed,
            self.client.pad.0.in_use,
            menu.open_polls,
            !self.prompt_open()
                && ((self.client.screen == Screen::Chat && !self.client.chat_ui.in_bed_chat)
                    || self.client.sign_edit.is_some()),
        ) {
            self.client.pad.0.menu.osk_dismissed = true;
            self.pad_open_keyboard();
        }
    }

    /// What a key of the on-screen keyboard does: the same as a typed key in
    /// the field that has focus.
    pub fn pad_osk_output(&mut self, out: osk::Output) {
        if self.context_menu_open() {
            return;
        }
        use osk::Output;
        let (key, text) = match &out {
            Output::Text(t) => (None, Some(t.as_str())),
            Output::Backspace => (Some(KeyCode::Backspace), None),
            Output::Enter => (Some(KeyCode::Enter), None),
            Output::Done if self.pad_enter_keeps_typing() => {
                self.pad_osk_done_lines();
                return;
            }
            Output::Done => (Some(KeyCode::Enter), None),
            // Chat's suggestions (CommandSuggestions.keyPressed): Tab, Up and Down.
            Output::Complete | Output::Previous | Output::Next => {
                if self.pad_chat_owns_keyboard() {
                    let key = match out {
                        Output::Complete => KeyCode::Tab,
                        Output::Previous => KeyCode::ArrowUp,
                        _ => KeyCode::ArrowDown,
                    };
                    self.chat_key_code(key, None);
                }
                return;
            }
            Output::Close => {
                self.client.pad.0.menu.osk = None;
                self.client.pad.0.menu.osk_dismissed = true;
                return;
            }
        };
        let backspace = key == Some(KeyCode::Backspace);
        if self.mounted_widget_captures_input() {
            // These are edits from the controller keyboard, not physical keys or chars.
            let modifiers = glfw_modifiers(self.client.modifiers);
            if let Some(text) = text {
                let receiver = self.client.screen_state.0.widgets.generation();
                for c in text.chars() {
                    self.mounted_widget_char_typed(c, modifiers);
                    if self.client.screen_state.0.widgets.generation() != receiver {
                        break;
                    }
                }
            } else if let Some(key) = key {
                self.mounted_widget_key_pressed(key, false, modifiers);
            }
        } else if self.dialog_open() {
            // Over whatever else is open. Enter only ends the typing, but for a new line.
            let result = match text {
                Some(t) => {
                    self.dialog_type(t);
                    Ok(())
                }
                None if backspace || self.dialog_typing_lines() => {
                    self.dialog_key(key.expect("a key"), None).map(|_| ())
                }
                None => Ok(()),
            };
            if let Err(e) = result {
                log::warn!("Sunrise: on-screen keyboard: {e}");
            }
        } else if self.client.screen == Screen::Chat && self.client.title.0.is_none() {
            match text {
                Some(t) => self.chat_type(t),
                None => {
                    self.chat_key_code(key.expect("a key"), None);
                }
            }
        } else if self.client.sign_edit.is_some() {
            self.sign_input(key, text);
        } else if self.client.book.is_some() {
            self.book_input(key, text);
        } else if self.client.screen == Screen::Creative
            && self.client.creative_screen.0.tab().search
        {
            if backspace {
                self.creative_search_backspace(false);
            } else if key != Some(KeyCode::Enter) {
                self.creative_search_edit(text);
            }
        } else if self.mod_browser_typing() {
            if key != Some(KeyCode::Enter) {
                self.mod_browser_text(text, backspace);
            }
        } else if self.recipe_search_focused() {
            if key != Some(KeyCode::Enter) {
                self.recipe_text(text, backspace);
            }
        } else if self.station_typing() {
            if key != Some(KeyCode::Enter) {
                self.station_text(text, backspace);
            }
        } else if self.client.screen == Screen::WorldOptions {
            if key != Some(KeyCode::Enter) {
                self.world_options_text(text);
            }
        } else if self.client.screen == Screen::CommandBlock {
            // Enter is its Done, as on the keyboard.
            match text {
                Some(t) => self.command_block_type(t),
                None => {
                    self.command_block_key_code(key.expect("a key"), None, None);
                }
            }
        } else if self.client.screen == Screen::BlockEditor {
            match text {
                Some(t) => self.block_editor_type(t),
                None => {
                    self.block_editor_key_code(key.expect("a key"), None, None);
                }
            }
        } else if self.client.screen == Screen::Social {
            match text {
                Some(t) => self.social_type(t),
                None if backspace => self.social_backspace(),
                None => {}
            }
        } else if self.client.screen == Screen::DebugOptions {
            match text {
                Some(t) => self.debug_options_type(t),
                None if backspace => self.debug_options_backspace(),
                None => {}
            }
        } else if self.client.screen == Screen::Options {
            let result = match text {
                Some(t) => {
                    self.client
                        .chat_ui
                        .options
                        .type_text(&self.client.gui.atlas, t);
                    Ok(())
                }
                None => self.options_key(key.expect("a key"), None).map(|_| ()),
            };
            if let Err(e) = result {
                log::warn!("Sunrise: on-screen keyboard: {e}");
            }
        } else if self.title_input_active() {
            match key {
                Some(KeyCode::Enter) => {
                    if let Some(t) = self.client.title.0.as_mut() {
                        t.confirm_edit();
                    }
                }
                Some(KeyCode::Backspace) => self.title_edit(None, true),
                _ => self.title_edit(text, false),
            }
        }
        // Enter ends the typing, but for a new line; a field that closed with it closes the
        // keyboard.
        if (key == Some(KeyCode::Enter) && !self.pad_enter_keeps_typing())
            || !self.pad_has_typing_screen()
        {
            self.client.pad.0.menu.osk = None;
        }
    }

    /// Done over a field of many lines: what the screen's own Done button does (a book and
    /// quill saves and closes, a sign closes and sends its text); a dialog's input has none, so
    /// the keyboard closes. Never a new line: that's the New Line key.
    fn pad_osk_done_lines(&mut self) {
        self.client.pad.0.menu.osk = None;
        self.client.pad.0.menu.osk_dismissed = true;
        if self.dialog_typing_lines() {
            return;
        }
        if self.client.sign_edit.is_some() {
            self.close_sign_editor();
        } else if self.client.book.is_some()
            && let Err(e) = self.book_control(Some("done"))
        {
            log::warn!("Sunrise: on-screen keyboard: {e}");
        }
    }

    // ---- drawing ----

    /// The pad's cursor (pointer mode) or focus highlight (focus mode), and
    /// the on-screen keyboard, over everything else.
    pub(in crate::game::client_side) fn draw_pad_cursor(&self, batch: &mut HudBatch) {
        widgets::set_action_glyphs(self.pad_action_glyphs_on());
        widgets::set_action_screen(self.key_screen());
        if !self.pad_in_menu() {
            return;
        }
        if let Some(keyboard) = &self.client.pad.0.menu.osk {
            let at_top = self
                .client
                .pad
                .0
                .menu
                .focus
                .is_some_and(|f| f.center().1 > batch.height / 2.0)
                || self.client.screen == Screen::Chat;
            let mut ui = sunrise_client::ui_theme::Ui::new(
                &self.client.gui.atlas,
                batch,
                self.client.cursor,
            );
            let options = &self.client.chat_ui.options;
            keyboard.draw(
                &mut ui,
                at_top,
                options.pad_glyphs,
                self.pad_chat_owns_keyboard(),
            );
            ui.finish();
            return;
        }
        if !self.client.pad.0.in_use && !self.client.pad.0.menu.keyboard {
            return;
        }
        let Some((x, y)) = self.client.pad.0.menu.pos else {
            return;
        };
        if self.pad_focus_mode()
            && let Some(focus) = self.client.pad.0.menu.focus
        {
            self.draw_pad_focus(batch, focus);
            self.draw_pad_action_glyphs(batch);
            return;
        }
        self.draw_pad_action_glyphs(batch);
        // A crosshair centred on the point it clicks: inventories, and a
        // screen with nothing to focus.
        use sunrise_client::pad_cursor;
        let (left, top) = pad_cursor::origin((x, y));
        batch.sprite_scaled(
            &self.client.gui.atlas,
            pad_cursor::SPRITE,
            left,
            top,
            pad_cursor::DRAWN,
            pad_cursor::DRAWN,
            u32::MAX,
        );
    }

    /// The focus highlight, as vanilla's keyboard focus looks: buttons,
    /// sliders and slots already draw their highlighted sprite under the
    /// (hidden) pointer, and list rows and fields get vanilla's white
    /// one-pixel outline (`AbstractSelectionList`'s focused selection,
    /// `EditBox`'s highlighted border).
    fn draw_pad_focus(&self, batch: &mut HudBatch, focus: Target) {
        key_focus::draw_outline(&self.client.gui.atlas, batch, focus);
        // A slider in hand shows it: left and right do something else on one that isn't.
        if self.pad_holding_slider() {
            key_focus::draw_white_outline(&self.client.gui.atlas, batch, focus.rect);
        }
    }
}

/// Whether the keyboard opens by itself over chat or a sign: the pad is in
/// use and the screen has been open for a few polls, and it is neither
/// open nor dismissed already. Independent of the menu mode.
fn osk_auto_opens(
    osk_open: bool,
    dismissed: bool,
    pad_in_use: bool,
    polls: u32,
    typing_screen: bool,
) -> bool {
    !osk_open && !dismissed && pad_in_use && polls >= 3 && typing_screen
}

/// Whether a text field in `targets` contains the point `at` (GUI pixels).
fn field_at(targets: &[Target], at: (f32, f32)) -> bool {
    targets.iter().any(|t| {
        let [x, y, w, h] = t.rect;
        matches!(t.kind, Kind::Field | Kind::Mounted { field: true, .. })
            && at.0 >= x
            && at.0 < x + w
            && at.1 >= y
            && at.1 < y + h
    })
}

/// Whether `front` is another screen than the one `shown` names yet, and
/// records it: the menu focus belongs to the screen that showed it
/// (`Gui.setScreen` clears the focus of the screen it leaves).
pub(in crate::game::client_side) fn another_screen_is_front(shown: &mut u64, front: u64) -> bool {
    std::mem::replace(shown, front) != front
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_focus_that_lost_its_widget_goes_beside_it() {
        let reset = [200.0, 50.0, 40.0, 20.0];
        let targets = [
            [0.0, 0.0, 100.0, 16.0],
            [100.0, 50.0, 90.0, 20.0],
            [0.0, 50.0, 40.0, 20.0],
            [200.0, 80.0, 40.0, 20.0],
        ];
        assert_eq!(beside(reset, &targets), Some(1));
        assert_eq!(beside(reset, &targets[3..]), None);
    }

    #[test]
    fn keyboard_auto_opens_for_pad_on_typing_screen() {
        assert!(osk_auto_opens(false, false, true, 3, true));
        assert!(!osk_auto_opens(false, false, true, 2, true));
        assert!(!osk_auto_opens(false, false, false, 5, true));
        assert!(!osk_auto_opens(false, true, true, 5, true));
        assert!(!osk_auto_opens(true, false, true, 5, true));
        assert!(!osk_auto_opens(false, false, true, 5, false));
    }

    fn labelled(label: &str, rect: Rect) -> widgets::Widget {
        let [x, y, w, h] = rect;
        widgets::Widget {
            label: label.to_owned(),
            x,
            y,
            w,
            h,
            enabled: true,
            kind: Kind::Button,
        }
    }

    #[test]
    fn a_named_widget_is_found_wherever_it_moved() {
        let (first, second) = ([10.0, 40.0, 100.0, 20.0], [10.0, 64.0, 100.0, 20.0]);
        let labels = [labelled("FOV: 80", second), labelled("Done", first)];
        // Its value changed its label and another row took its place: the name still finds it.
        assert_eq!(named_target("fov", &labels, &[first, second]), Some(1));
        // Clipped away: no target where the label says, so the rectangles decide.
        assert_eq!(named_target("fov", &labels, &[first]), None);
        assert_eq!(named_target("music", &labels, &[first, second]), None);
    }

    #[test]
    fn two_widgets_of_one_name_leave_it_to_the_rectangles() {
        let (first, second) = ([10.0, 40.0, 40.0, 20.0], [10.0, 64.0, 40.0, 20.0]);
        let labels = [labelled("Reset", first), labelled("Reset", second)];
        assert_eq!(named_target("reset", &labels, &[first, second]), None);
    }

    #[test]
    fn a_new_screen_is_a_new_place_for_the_focus() {
        let mut shown = 7;
        assert!(!another_screen_is_front(&mut shown, 7), "the same screen");
        assert!(another_screen_is_front(&mut shown, 8), "another screen");
        assert!(!another_screen_is_front(&mut shown, 8), "still that one");
        assert_eq!(shown, 8);
    }

    #[test]
    fn a_scrollbar_tells_how_far_rows_moved_when_rectangles_cannot() {
        // A whole row a notch: the same rectangles, so only the amount shows it.
        assert_eq!(list_shift(Some(0.0), Some(24.0)), Some(Some(-24.0)));
        assert_eq!(list_shift(Some(24.0), Some(0.0)), Some(Some(24.0)));
        assert_eq!(list_shift(Some(24.0), Some(24.0)), Some(None));
        // No scrollbar on either frame: nothing to ask.
        assert_eq!(list_shift(None, Some(24.0)), None);
        assert_eq!(list_shift(Some(24.0), None), None);
    }

    #[test]
    fn armed_button_clicks_only_while_still_focused() {
        let rect = [10.0, 20.0, 100.0, 20.0];
        let kind = Kind::Button;
        let focus = Some(Target { rect, kind });
        assert_eq!(
            armed_click(focus, focus).as_ref().map(Target::center),
            Some((60.0, 30.0))
        );
        let refound = Some(Target {
            rect: [10.0, 20.000_002, 100.0, 20.0],
            kind,
        });
        assert!(armed_click(focus, refound).is_some());
        // A row scrolled into view at frame 10 is looked for again from frame 12.
        assert!(!reveal_pending(None, 10));
        assert!(reveal_pending(Some(10), 10) && reveal_pending(Some(10), 11));
        assert!(!reveal_pending(Some(10), 12));
        let moved = Some(Target {
            rect: [10.0, 44.0, 100.0, 20.0],
            kind,
        });
        assert_eq!(armed_click(focus, moved), None);
        assert_eq!(armed_click(focus, None), None);
        assert_eq!(armed_click(None, focus), None);
    }

    #[test]
    fn clear_focus_leaves_nothing_focused() {
        let mut menu = MenuCursor {
            focus: Some(Target {
                rect: [229.0, 98.0, 20.0, 18.0],
                kind: Kind::Button,
            }),
            origin: Some([229.0, 98.0, 20.0, 18.0]),
            slider_edit: true,
            ..MenuCursor::default()
        };
        menu.clear_focus();
        assert!(menu.focus.is_none());
        assert!(menu.origin.is_none());
        assert!(!menu.slider_edit);
    }

    #[test]
    fn field_hit_test() {
        let t = |rect, kind| Target { rect, kind };
        let targets = [
            t([0.0, 0.0, 50.0, 20.0], Kind::Button),
            t([10.0, 30.0, 100.0, 20.0], Kind::Field),
        ];
        assert!(field_at(&targets, (10.0, 30.0)));
        assert!(field_at(&targets, (109.0, 49.0)));
        assert!(!field_at(&targets, (110.0, 40.0)));
        assert!(!field_at(&targets, (50.0, 29.0)));
        assert!(!field_at(&targets, (5.0, 5.0)), "a button is not a field");
        assert!(!field_at(&[], (5.0, 5.0)));
    }
}
