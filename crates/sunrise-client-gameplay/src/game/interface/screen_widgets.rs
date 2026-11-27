//! Mounted screen widgets (`ScreenEvent.Init.Post` / `Screen.addRenderableWidget`):
//! engine-owned buttons, local `EditBox` text fields, and custom [`ScreenWidget`]
//! instances with synchronous callback execution via temporary ownership extraction.

#[cfg(feature = "client")]
use crate::game::host::draw::GameDrawContext;
use mod_api::{
    __private::game::CallbackOwner,
    ScreenView, ScreenWidget, ScreenWidgetId, ScreenWidgets,
    client::widgets::{
        Button, ButtonPress, ButtonState, ScreenChild, TextEdit, TextField, TextFieldCallback,
        is_allowed_chat_character,
    },
};
use std::cell::{Cell, RefCell};
use std::time::Instant;
use sunrise_client::{
    clipboard::EditShortcut,
    hud::HudBatch,
    keybinds,
    preedit::PreeditEvent,
    ui::{self, EditBox},
    widgets::{self, Kind, Narratable, Target},
};
use sunrise_game_data::text::{DARK_GRAY, Style, Text};
use sunrise_gameplay_common::game::api_values::TryIntoEngine;
use winit::keyboard::KeyCode;

pub(in crate::game) struct MountedWidgetsInner {
    pub(in crate::game) entries: Vec<Option<MountedWidgetEntry>>,
    pub(in crate::game) removed_pause_actions: Vec<u8>,
    pub(in crate::game) focused: Option<usize>,
    pub(in crate::game) dragging: Option<(usize, i32)>,
}

/// Mounted widgets attached to the currently open screen during `Hooks::screen_init_post`.
pub struct MountedScreenWidgets {
    pub(in crate::game) generation: Cell<u64>,
    pub(in crate::game) next_id: Cell<u64>,
    pub(in crate::game) inner: RefCell<MountedWidgetsInner>,
}

impl Default for MountedScreenWidgets {
    fn default() -> Self {
        Self {
            generation: Cell::new(0),
            next_id: Cell::new(1),
            inner: RefCell::new(MountedWidgetsInner {
                entries: Vec::new(),
                removed_pause_actions: Vec::new(),
                focused: None,
                dragging: None,
            }),
        }
    }
}

impl Drop for MountedScreenWidgets {
    fn drop(&mut self) {
        self.clear();
    }
}

impl MountedScreenWidgets {
    pub fn generation(&self) -> u64 {
        self.generation.get()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.borrow().entries.is_empty()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn len(&self) -> usize {
        self.inner.borrow().entries.len()
    }

    pub fn has_focus(&self) -> bool {
        self.inner.borrow().focused.is_some()
    }

    pub fn focused_index(&self) -> Option<usize> {
        self.inner.borrow().focused
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn dragging(&self) -> Option<(usize, i32)> {
        self.inner.borrow().dragging
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn is_disabled(&self, idx: usize) -> bool {
        self.inner
            .borrow()
            .entries
            .get(idx)
            .and_then(Option::as_ref)
            .is_some_and(|e| e.disabled)
    }

    pub(in crate::game) fn index_of(&self, id: ScreenWidgetId) -> Option<usize> {
        self.inner
            .borrow()
            .entries
            .iter()
            .position(|slot| slot.as_ref().is_some_and(|entry| entry.id == id))
    }

    pub fn focused_id(&self) -> Option<ScreenWidgetId> {
        let inner = self.inner.borrow();
        let idx = inner.focused?;
        inner.entries.get(idx)?.as_ref().map(|entry| entry.id)
    }

    /// Bump the screen-widget generation, clear focus/drag state, and dispose all entries.
    pub fn clear(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        let (mut entries, focused) = {
            let mut inner = self.inner.borrow_mut();
            inner.dragging = None;
            inner.removed_pause_actions.clear();
            (std::mem::take(&mut inner.entries), inner.focused.take())
        };
        if let Some(entry) = focused.and_then(|idx| entries.get_mut(idx)?.as_mut()) {
            entry.apply_focus(false);
        }
        for entry in entries.into_iter().flatten() {
            entry.dispose();
        }
    }

    pub(in crate::game) fn clear_links_for(&self, idx: usize) {
        let mut inner = self.inner.borrow_mut();
        if inner.focused == Some(idx) {
            inner.focused = None;
        }
        if inner.dragging.is_some_and(|(drag_idx, _)| drag_idx == idx) {
            inner.dragging = None;
        }
    }
}

/// One widget mounted on the active screen, owned by the engine with per-widget panic isolation.
pub struct MountedWidgetEntry {
    pub(in crate::game) id: ScreenWidgetId,
    pub(in crate::game) owner: CallbackOwner,
    pub(in crate::game) disabled: bool,
    pub kind: MountedWidgetKind,
}

/// Engine representation of each mounted widget variant.
pub enum MountedWidgetKind {
    Button {
        state: ButtonState,
        on_press: Option<Box<ButtonPress>>,
    },
    TextField {
        edit_box: EditBox,
        hint: Option<Text>,
        active: bool,
        visible: bool,
        on_change: Option<Box<TextFieldCallback>>,
        on_submit: Option<Box<TextFieldCallback>>,
    },
    Custom(Box<dyn ScreenWidget>),
}

impl MountedWidgetEntry {
    pub(in crate::game) fn dispose(self) {
        let Self { owner, kind, .. } = self;
        owner.dispose("screen widget", "screen_widget_drop", kind);
    }

    #[cfg(feature = "client")]
    pub(in crate::game) fn press_button(&mut self, game: &mut crate::game::ClientGame<'_>) -> bool {
        let MountedWidgetKind::Button { state, on_press } = &mut self.kind else {
            return false;
        };
        let Some(callback) = on_press.as_mut() else {
            return true;
        };
        let rect = state.rect;
        let answer = call_owner(&self.owner, &mut self.disabled, "button_press", || {
            callback(state, game);
        });
        if !valid_widget_rect(state.rect) {
            state.rect = rect;
        }
        answer.is_some()
    }

    pub(in crate::game) fn can_focus(&mut self) -> bool {
        if self.disabled || !self.owner.is_enabled() {
            self.disabled = true;
            return false;
        }
        match &self.kind {
            MountedWidgetKind::Button { state, .. } => state.visible && state.active,
            MountedWidgetKind::TextField {
                active, visible, ..
            } => *visible && *active,
            MountedWidgetKind::Custom(widget) => call_owner(
                &self.owner,
                &mut self.disabled,
                "screen_widget_focusable",
                || widget.visible() && widget.active() && valid_widget_rect(widget.bounds()),
            )
            .unwrap_or(false),
        }
    }

    pub(in crate::game) fn apply_focus(&mut self, focused: bool) -> bool {
        if self.disabled || !self.owner.is_enabled() {
            self.disabled = true;
            return false;
        }
        match &mut self.kind {
            MountedWidgetKind::Button { .. } => true,
            MountedWidgetKind::TextField { edit_box, .. } => {
                edit_box.focused = focused;
                if focused {
                    edit_box.reset_blink();
                }
                true
            }
            MountedWidgetKind::Custom(widget) => call_owner(
                &self.owner,
                &mut self.disabled,
                "screen_widget_focus",
                || widget.set_focused(focused),
            )
            .is_some(),
        }
    }
}

/// Registrar passed to `Hooks::screen_init_post` (`ScreenEvent.Init.Post`).
pub(in crate::game) struct ScreenWidgetMount {
    pub(in crate::game) next_id: u64,
    pub(in crate::game) entries: Vec<MountedWidgetEntry>,
    pub(in crate::game) children: Vec<ScreenChild>,
    pub(in crate::game) removed: Vec<ScreenWidgetId>,
    pub(in crate::game) initial_focus: Option<ScreenWidgetId>,
}

impl Drop for ScreenWidgetMount {
    fn drop(&mut self) {
        for entry in std::mem::take(&mut self.entries) {
            entry.dispose();
        }
    }
}

impl ScreenWidgetMount {
    pub(in crate::game) fn alloc_id(&mut self) -> ScreenWidgetId {
        let raw = self.next_id;
        self.next_id = self.next_id.saturating_add(1).max(1);
        ScreenWidgetId::from_raw(raw)
    }

    pub(in crate::game) fn stage(&mut self, kind: MountedWidgetKind) -> ScreenWidgetId {
        let id = self.alloc_id();
        let mut entry = MountedWidgetEntry {
            id,
            owner: widget_callback_owner(),
            disabled: false,
            kind,
        };
        let bounds = match &entry.kind {
            MountedWidgetKind::Button { state, .. } => {
                Some((state.rect, Some(state.label.clone())))
            }
            MountedWidgetKind::TextField { edit_box, .. } => Some((
                [edit_box.x, edit_box.y, edit_box.width, edit_box.height],
                None,
            )),
            MountedWidgetKind::Custom(widget) => call_owner(
                &entry.owner,
                &mut entry.disabled,
                "screen_widget_bounds",
                || (widget.bounds(), None),
            ),
        };
        if let Some((rect, label)) = bounds
            && valid_widget_rect(rect)
        {
            self.children.push(ScreenChild::new(id, rect, label));
            self.entries.push(entry);
        } else {
            entry.dispose();
        }
        id
    }
}

impl ScreenWidgets for ScreenWidgetMount {
    fn children(&self) -> &[ScreenChild] {
        &self.children
    }

    fn add_button(&mut self, button: Button) -> ScreenWidgetId {
        let (state, on_press) = button.into_parts();
        self.stage(MountedWidgetKind::Button { state, on_press })
    }

    fn add_text_field(&mut self, field: TextField) -> ScreenWidgetId {
        let parts = field.into_parts();
        let [x, y, w, h] = parts.rect;
        let mut edit_box = EditBox::new(x, y, w, h);
        edit_box.max_length = parts.max_length;
        edit_box.bordered = parts.bordered;
        edit_box.editable = parts.editable;
        edit_box.focused = false;
        edit_box.set_value(&parts.value);
        // `EditBox.setHint`: an unstyled hint takes `DEFAULT_HINT_STYLE` (`DARK_GRAY`).
        let hint = parts.hint.and_then(|hint| {
            let text: Text = hint.try_into_engine(&()).ok()?;
            let unstyled = text.spans.iter().all(|span| span.style == Style::default());
            Some(if unstyled {
                text.with_color(DARK_GRAY)
            } else {
                text
            })
        });
        self.stage(MountedWidgetKind::TextField {
            edit_box,
            hint,
            active: parts.active,
            visible: parts.visible,
            on_change: parts.on_change,
            on_submit: parts.on_submit,
        })
    }

    fn add_widget(&mut self, widget: Box<dyn ScreenWidget>) -> ScreenWidgetId {
        self.stage(MountedWidgetKind::Custom(widget))
    }

    fn set_initial_focus(&mut self, id: ScreenWidgetId) -> bool {
        let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) else {
            return false;
        };
        if !entry.can_focus() {
            return false;
        }
        self.initial_focus = Some(id);
        true
    }

    fn remove(&mut self, id: ScreenWidgetId) -> bool {
        let Some(pos) = self.children.iter().position(|child| child.id == id) else {
            return false;
        };
        self.children.remove(pos);
        if self.initial_focus == Some(id) {
            self.initial_focus = None;
        }
        if let Some(pos) = self.entries.iter().position(|entry| entry.id == id) {
            self.entries.remove(pos).dispose();
        } else {
            self.removed.push(id);
        }
        true
    }
}

pub(in crate::game) fn widget_callback_owner() -> CallbackOwner {
    CallbackOwner::for_mod(mod_api::content::crash::current_native_mod().unwrap_or("sunrise"))
}

pub(in crate::game) fn call_owner<R>(
    owner: &CallbackOwner,
    disabled: &mut bool,
    operation: &'static str,
    invoke: impl FnOnce() -> R,
) -> Option<R> {
    if *disabled || !owner.is_enabled() {
        *disabled = true;
        return None;
    }
    match owner.call("screen widget", operation, invoke) {
        Ok(value) => Some(value),
        Err(_) => {
            if !owner.is_enabled() {
                *disabled = true;
            }
            None
        }
    }
}

pub(in crate::game) fn valid_widget_rect([x, y, w, h]: [f32; 4]) -> bool {
    [x, y, w, h, x + w, y + h].into_iter().all(f32::is_finite)
}

pub(in crate::game) fn point_in_rect(mouse: [f32; 2], rect: [f32; 4]) -> bool {
    let [mx, my] = mouse;
    let [x, y, w, h] = rect;
    w > 0.0 && h > 0.0 && mx >= x && my >= y && mx < x + w && my < y + h
}

impl MountedWidgetEntry {
    pub(in crate::game) fn target(&mut self) -> Option<Target> {
        if self.disabled || !self.owner.is_enabled() {
            self.disabled = true;
            return None;
        }
        match &self.kind {
            MountedWidgetKind::Button { state, .. } => {
                (state.visible && state.active).then_some(Target {
                    rect: state.rect,
                    kind: Kind::Mounted {
                        id: self.id.raw(),
                        field: false,
                    },
                })
            }
            MountedWidgetKind::TextField {
                edit_box,
                active,
                visible,
                ..
            } => (*visible && *active).then_some(Target {
                rect: [edit_box.x, edit_box.y, edit_box.width, edit_box.height],
                kind: Kind::Mounted {
                    id: self.id.raw(),
                    field: true,
                },
            }),
            MountedWidgetKind::Custom(widget) => call_owner(
                &self.owner,
                &mut self.disabled,
                "screen_widget_target",
                || {
                    let rect = widget.bounds();
                    (widget.visible() && widget.active() && valid_widget_rect(rect)).then(|| {
                        Target {
                            rect,
                            kind: Kind::Mounted {
                                id: self.id.raw(),
                                field: widget.captures_input(),
                            },
                        }
                    })
                },
            )
            .flatten(),
        }
    }
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Run `ScreenEvent.Init.Post` to mount widgets on the initialized screen and apply focus.
    pub fn init_screen_widgets(&mut self, view: &ScreenView<'_>) {
        if !mod_api::__private::listened(mod_api::HookId::ScreenInitPost) {
            return;
        }
        let mut mount = ScreenWidgetMount {
            next_id: self.client.screen_state.0.widgets.next_id.get(),
            entries: Vec::new(),
            children: Vec::new(),
            removed: Vec::new(),
            initial_focus: None,
        };
        // Use the initialized layout, not last frame's draw registrations (which may
        // still belong to a different screen, and omit children clipped offscreen).
        let native = match (view.id, self.client.title.0.as_ref()) {
            ("paused", _) => self.pause_init_children(),
            ("title", Some(title)) => {
                let [w, h] = self.gui_frame_now().1.size.map(|axis| axis as f32);
                title
                    .init_children(&self.client.gui.atlas, w, h)
                    .into_iter()
                    .map(|(action, rect, key)| {
                        (
                            action,
                            rect,
                            Some(mod_api::Text::translate(key, Vec::new())),
                        )
                    })
                    .collect()
            }
            _ => Vec::new(),
        };
        let native = native
            .into_iter()
            .map(|(action, rect, label)| {
                let id = mount.alloc_id();
                mount.children.push(ScreenChild::new(id, rect, label));
                (id, action)
            })
            .collect::<Vec<_>>();
        let mut entries: Vec<MountedWidgetEntry> = Vec::new();
        let mut removed_actions = Vec::new();
        let mut initial_focus = None;
        self.native_screen_init_post(view, |listen| {
            let children = mount.children.clone();
            if listen(&mut mount) {
                for id in mount.removed.drain(..) {
                    if initial_focus == Some(id) {
                        initial_focus = None;
                    }
                    if let Some(pos) = entries.iter().position(|entry| entry.id == id) {
                        entries.remove(pos).dispose();
                    } else if let Some(&(_, action)) =
                        native.iter().find(|&&(child, _)| child == id)
                    {
                        removed_actions.push(action);
                    }
                }
                initial_focus = mount.initial_focus.take().or(initial_focus);
                entries.append(&mut mount.entries);
            } else {
                mount.children = children;
                mount.initial_focus = None;
                mount.removed.clear();
                for entry in mount.entries.drain(..) {
                    entry.dispose();
                }
            }
            // IDs issued to a failed listener are never reused.
        });
        self.client
            .screen_state
            .0
            .widgets
            .next_id
            .set(mount.next_id);
        let initial_focus =
            initial_focus.and_then(|id| entries.iter().position(|entry| entry.id == id));
        if view.id == "title"
            && let Some(title) = self.client.title.0.as_mut()
        {
            title.set_removed_children(std::mem::take(&mut removed_actions));
        }
        let mut inner = self.client.screen_state.0.widgets.inner.borrow_mut();
        inner.entries = entries.into_iter().map(Some).collect();
        inner.removed_pause_actions = removed_actions;
        if self
            .client
            .pause_focus
            .is_some_and(|id| inner.removed_pause_actions.contains(&id))
        {
            self.client.pause_focus = None;
        }
        drop(inner);
        if let Some(idx) = initial_focus {
            self.set_mounted_widget_focus(Some(idx));
        }
    }

    /// Whether Init.Post removed this native PauseScreen action from the current build.
    pub fn pause_child_removed(&self, action: u8) -> bool {
        self.client
            .screen_state
            .0
            .widgets
            .inner
            .borrow()
            .removed_pause_actions
            .contains(&action)
    }

    /// Clear and dispose all mounted widgets for the current screen without
    /// discarding `last_click` (same-instance resize rebuilds widgets while
    /// preserving `MouseHandler.LastClick`).
    pub fn clear_screen_widgets(&mut self) {
        self.client.screen_state.0.widgets.clear();
        if let Some(title) = self.client.title.0.as_mut() {
            title.set_removed_children(Vec::new());
        }
        sunrise_client::preedit::reset();
        self.client.ime_area = None;
    }

    /// Remove focus from any currently focused mounted widget.
    pub fn clear_mounted_widget_focus(&mut self) {
        self.set_mounted_widget_focus(None);
    }

    pub(in crate::game) fn set_mounted_widget_focus(&mut self, target: Option<usize>) {
        let previous = self.client.screen_state.0.widgets.focused_index();
        if previous == target {
            return;
        }
        sunrise_client::preedit::reset();
        self.client.ime_area = None;
        self.client
            .screen_state
            .0
            .widgets
            .inner
            .borrow_mut()
            .focused = target;
        if let Some(old_idx) = previous {
            let disabled = {
                let mut inner = self.client.screen_state.0.widgets.inner.borrow_mut();
                inner
                    .entries
                    .get_mut(old_idx)
                    .and_then(Option::as_mut)
                    .is_some_and(|entry| {
                        entry.apply_focus(false);
                        entry.disabled
                    })
            };
            if disabled {
                self.client.screen_state.0.widgets.clear_links_for(old_idx);
            }
        }
        if let Some(new_idx) = target {
            let (ok, widget_target) = {
                let mut inner = self.client.screen_state.0.widgets.inner.borrow_mut();
                match inner.entries.get_mut(new_idx).and_then(Option::as_mut) {
                    Some(entry) => {
                        let ok = entry.apply_focus(true);
                        let t = if ok { entry.target() } else { None };
                        (ok, t)
                    }
                    None => (false, None),
                }
            };
            if !ok {
                self.client.screen_state.0.widgets.clear_links_for(new_idx);
            } else if let Some(t) = widget_target {
                self.clear_screen_click_focus();
                self.client.pad.0.menu.origin = Some(t.rect);
                if self.client.pad.0.menu.keyboard {
                    self.client.pad.0.menu.focus = Some(t);
                }
            }
        }
    }

    /// All visible, active mounted widgets as unified focus/pad targets.
    pub fn mounted_widget_targets(&self) -> Vec<Target> {
        let mut inner = self.client.screen_state.0.widgets.inner.borrow_mut();
        let mut out = Vec::new();
        let mut disabled = Vec::new();
        for (idx, slot) in inner.entries.iter_mut().enumerate() {
            if let Some(entry) = slot.as_mut() {
                if let Some(t) = entry.target() {
                    out.push(t);
                } else if entry.disabled {
                    disabled.push(idx);
                }
            }
        }
        drop(inner);
        for idx in disabled {
            self.client.screen_state.0.widgets.clear_links_for(idx);
        }
        out
    }

    /// Synchronize with the selected child, not whichever widget overlaps its bounds.
    pub fn sync_mounted_focus(&mut self, target: Target) {
        let idx = match target.kind {
            Kind::Mounted { id, .. } => self
                .client
                .screen_state
                .0
                .widgets
                .index_of(ScreenWidgetId::from_raw(id)),
            _ => None,
        };
        self.set_mounted_widget_focus(idx);
    }

    /// Temporarily extract mounted widget `idx` so its callback can receive `&mut dyn ClientHost`
    /// and execute screen transitions synchronously (`Gui.setScreen` / `Button.onPress`).
    /// If the screen's widget generation changed during `f`, the extracted entry is disposed
    /// instead of restored onto the new screen.
    pub fn with_mounted_widget<R>(
        &mut self,
        idx: usize,
        f: impl FnOnce(&mut Self, &mut MountedWidgetEntry) -> R,
    ) -> Option<R> {
        let generation = self.client.screen_state.0.widgets.generation();
        let (mut entry, focused) = {
            let mut inner = self.client.screen_state.0.widgets.inner.borrow_mut();
            (
                inner.entries.get_mut(idx)?.take()?,
                inner.focused == Some(idx),
            )
        };
        if entry.disabled || !entry.owner.is_enabled() {
            entry.disabled = true;
            self.client.screen_state.0.widgets.clear_links_for(idx);
            self.client
                .screen_state
                .0
                .widgets
                .inner
                .borrow_mut()
                .entries[idx] = Some(entry);
            return None;
        }
        let result = f(self, &mut entry);
        if self.client.screen_state.0.widgets.generation() == generation {
            if entry.disabled || !entry.owner.is_enabled() {
                entry.disabled = true;
                self.client.screen_state.0.widgets.clear_links_for(idx);
            }
            let restored = {
                let mut inner = self.client.screen_state.0.widgets.inner.borrow_mut();
                if let Some(slot) = inner.entries.get_mut(idx) {
                    *slot = Some(entry);
                    None
                } else {
                    Some(entry)
                }
            };
            if let Some(entry) = restored {
                entry.dispose();
            }
        } else {
            // Teardown could not blur this entry while it was out of the table.
            if focused {
                entry.apply_focus(false);
            }
            entry.dispose();
        }
        Some(result)
    }

    /// Draw every visible mounted widget inside the receiving screen's
    /// Pre-guarded contents pass, before Foreground and tooltips
    /// (`Screen.extractRenderState`).
    pub fn draw_mounted_screen_widgets(&self, batch: &mut HudBatch) {
        self.draw_mounted_screen_widgets_at(batch, Instant::now());
    }

    pub fn draw_mounted_screen_widgets_at(&self, batch: &mut HudBatch, now: Instant) {
        let len = self
            .client
            .screen_state
            .0
            .widgets
            .inner
            .borrow()
            .entries
            .len();
        if len == 0 {
            return;
        }
        let mouse = self.cursor();
        let partial_tick = self.common.clock.fraction() as f32;
        let focused_idx = self.client.screen_state.0.widgets.focused_index();
        let generation = self.client.screen_state.0.widgets.generation();
        for idx in 0..len {
            if self.client.screen_state.0.widgets.generation() != generation {
                break;
            }
            let Some(mut entry) = self
                .client
                .screen_state
                .0
                .widgets
                .inner
                .borrow_mut()
                .entries
                .get_mut(idx)
                .and_then(Option::take)
            else {
                continue;
            };
            let focused = focused_idx == Some(idx);
            if !entry.disabled && entry.owner.is_enabled() {
                match &mut entry.kind {
                    MountedWidgetKind::Button { state, .. } => {
                        if state.visible {
                            let mark = widgets::frame_mark();
                            let hovered = point_in_rect(mouse, state.rect);
                            let label: Text = state.label.try_into_engine(&()).unwrap_or_default();
                            ui::button_focused_as(
                                batch,
                                &self.client.gui.atlas,
                                state.rect,
                                &label,
                                ui::ButtonVisualState {
                                    active: state.active,
                                    hovered,
                                    focused,
                                },
                                state.sprites.as_ref(),
                                |rect| Narratable::button(label.plain(), rect, state.active),
                            );
                            widgets::mark_mounted(mark, entry.id.raw(), false);
                        }
                    }
                    MountedWidgetKind::TextField {
                        edit_box,
                        hint,
                        active,
                        visible,
                        ..
                    } => {
                        if *visible {
                            edit_box.focused = focused && *active;
                            let mark = widgets::frame_mark();
                            edit_box.draw_at(batch, &self.client.gui.atlas, None, now);
                            widgets::mark_mounted(mark, entry.id.raw(), true);
                            if let Some(hint) = hint
                                && edit_box.value.is_empty()
                                && !edit_box.focused
                            {
                                let color = if edit_box.editable {
                                    edit_box.text_color
                                } else {
                                    edit_box.text_color_uneditable
                                };
                                batch.text_styled(
                                    &self.client.gui.atlas,
                                    hint,
                                    edit_box.text_x(),
                                    edit_box.text_y(),
                                    color,
                                    true,
                                );
                            }
                        }
                    }
                    MountedWidgetKind::Custom(widget) => {
                        let paint = batch.paint_checkpoint();
                        let outcome = call_owner(
                            &entry.owner,
                            &mut entry.disabled,
                            "screen_widget_render",
                            || {
                                if !widget.visible() {
                                    return;
                                }
                                let bounds = widget.bounds();
                                if !valid_widget_rect(bounds) {
                                    return;
                                }
                                if widget.active() {
                                    widgets::target_as(
                                        bounds,
                                        Kind::Mounted {
                                            id: entry.id.raw(),
                                            field: widget.captures_input(),
                                        },
                                    );
                                }
                                let hovered = point_in_rect(mouse, bounds);
                                let host: &crate::game::ClientGame<'_> = self;
                                let mut draw = GameDrawContext::new(host, batch);
                                widget.render(
                                    host,
                                    &mut draw,
                                    mouse,
                                    hovered,
                                    focused,
                                    partial_tick,
                                );
                            },
                        );
                        if outcome.is_none() {
                            paint.discard(batch);
                        }
                    }
                }
            }
            if self.client.screen_state.0.widgets.generation() == generation {
                if entry.disabled || !entry.owner.is_enabled() {
                    entry.disabled = true;
                    self.client.screen_state.0.widgets.clear_links_for(idx);
                }
                let restored = {
                    let mut inner = self.client.screen_state.0.widgets.inner.borrow_mut();
                    if let Some(slot) = inner.entries.get_mut(idx) {
                        *slot = Some(entry);
                        None
                    } else {
                        Some(entry)
                    }
                };
                if let Some(entry) = restored {
                    entry.dispose();
                }
            } else {
                entry.dispose();
                break;
            }
        }
    }

    /// Dispatch a mouse click to the first hit mounted child in insertion order.
    /// A hit consumes the click independently of whether the child takes focus
    /// (`ContainerEventHandler.mouseClicked`). A miss leaves focus alone.
    pub fn mounted_widget_mouse_clicked(
        &mut self,
        mouse: [f32; 2],
        button: i32,
        modifiers: u32,
        double_click: bool,
    ) -> bool {
        self.mounted_widget_click(None, mouse, button, modifiers, double_click)
    }

    /// Controller focus names the child; coincident bounds are only a pointer concern.
    pub(in crate::game::client_side) fn activate_mounted_widget(
        &mut self,
        id: ScreenWidgetId,
        (x, y): (f32, f32),
    ) -> bool {
        let generation = self.client.screen_state.0.widgets.generation();
        let mouse = [x, y];
        let modifiers = crate::game::client_side::key_codes::glfw_modifiers(self.client.modifiers);
        let handled = self.mounted_widget_click(Some(id), mouse, 0, modifiers, false);
        if handled && self.client.screen_state.0.widgets.generation() == generation {
            self.mounted_widget_mouse_released(mouse, 0, modifiers);
        }
        handled
    }

    pub(in crate::game) fn mounted_widget_click(
        &mut self,
        id: Option<ScreenWidgetId>,
        mouse: [f32; 2],
        button: i32,
        modifiers: u32,
        double_click: bool,
    ) -> bool {
        let range = if let Some(id) = id {
            let Some(index) = self.client.screen_state.0.widgets.index_of(id) else {
                return false;
            };
            index..index + 1
        } else {
            let len = self
                .client
                .screen_state
                .0
                .widgets
                .inner
                .borrow()
                .entries
                .len();
            0..len
        };
        let hit_rect = |rect| id.is_some() || point_in_rect(mouse, rect);
        let generation = self.client.screen_state.0.widgets.generation();
        for idx in range {
            if self.client.screen_state.0.widgets.generation() != generation {
                return false;
            }
            enum Hit {
                Miss,
                Button,
                TextField,
                Custom,
            }
            let (hit, disabled) = {
                let mut inner = self.client.screen_state.0.widgets.inner.borrow_mut();
                match inner.entries.get_mut(idx).and_then(Option::as_mut) {
                    Some(entry) if !entry.disabled && entry.owner.is_enabled() => match &entry.kind
                    {
                        MountedWidgetKind::Button { state, .. } => {
                            if state.visible && state.active && hit_rect(state.rect) {
                                (Hit::Button, false)
                            } else {
                                (Hit::Miss, false)
                            }
                        }
                        MountedWidgetKind::TextField {
                            edit_box,
                            active,
                            visible,
                            ..
                        } => {
                            let rect = [edit_box.x, edit_box.y, edit_box.width, edit_box.height];
                            if *visible && *active && hit_rect(rect) {
                                (Hit::TextField, false)
                            } else {
                                (Hit::Miss, false)
                            }
                        }
                        MountedWidgetKind::Custom(widget) => {
                            let over = call_owner(
                                &entry.owner,
                                &mut entry.disabled,
                                "screen_widget_hit_test",
                                || {
                                    let rect = widget.bounds();
                                    widget.visible()
                                        && widget.active()
                                        && valid_widget_rect(rect)
                                        && hit_rect(rect)
                                },
                            )
                            .unwrap_or(false);
                            (if over { Hit::Custom } else { Hit::Miss }, entry.disabled)
                        }
                    },
                    _ => (Hit::Miss, false),
                }
            };
            if disabled {
                self.client.screen_state.0.widgets.clear_links_for(idx);
            }
            match hit {
                Hit::Miss => continue,
                Hit::Button => {
                    if button != 0 {
                        return id.is_none();
                    }
                    self.play_audio_button_click();
                    let accepted =
                        self.with_mounted_widget(idx, |game, entry| entry.press_button(game));
                    if accepted == Some(true)
                        && self.client.screen_state.0.widgets.generation() == generation
                    {
                        self.set_mounted_widget_focus(Some(idx));
                        self.client
                            .screen_state
                            .0
                            .widgets
                            .inner
                            .borrow_mut()
                            .dragging = Some((idx, button));
                    }
                    return true;
                }
                Hit::TextField => {
                    if button != 0 {
                        return id.is_none();
                    }
                    self.set_mounted_widget_focus(Some(idx));
                    let shift = (modifiers & 1) != 0;
                    {
                        let mut inner = self.client.screen_state.0.widgets.inner.borrow_mut();
                        if let Some(Some(entry)) = inner.entries.get_mut(idx)
                            && let MountedWidgetKind::TextField { edit_box, .. } = &mut entry.kind
                        {
                            edit_box.click(&self.client.gui.atlas, mouse[0], shift, double_click);
                        }
                        inner.dragging = Some((idx, button));
                    }
                    return true;
                }
                Hit::Custom => {
                    let handled = self
                        .with_mounted_widget(idx, |game, entry| {
                            if let MountedWidgetKind::Custom(widget) = &mut entry.kind {
                                call_owner(
                                    &entry.owner,
                                    &mut entry.disabled,
                                    "screen_widget_mouse_clicked",
                                    || {
                                        widget.mouse_clicked(
                                            game,
                                            mouse,
                                            button,
                                            modifiers,
                                            double_click,
                                        )
                                    },
                                )
                                .unwrap_or(false)
                            } else {
                                false
                            }
                        })
                        .unwrap_or(false);
                    if handled && self.client.screen_state.0.widgets.generation() == generation {
                        self.set_mounted_widget_focus(Some(idx));
                        if button == 0 {
                            self.client
                                .screen_state
                                .0
                                .widgets
                                .inner
                                .borrow_mut()
                                .dragging = Some((idx, button));
                        }
                    }
                    return id.is_none() || handled;
                }
            }
        }
        false
    }

    /// Dispatch a mouse release to the mounted widget currently being dragged.
    pub fn mounted_widget_mouse_released(
        &mut self,
        mouse: [f32; 2],
        button: i32,
        modifiers: u32,
    ) -> bool {
        let Some((idx, drag_button)) = self.client.screen_state.0.widgets.inner.borrow().dragging
        else {
            return false;
        };
        if drag_button != button {
            return false;
        }
        self.client
            .screen_state
            .0
            .widgets
            .inner
            .borrow_mut()
            .dragging = None;
        if self.client.screen_state.0.widgets.focused_index() != Some(idx) {
            return false;
        }
        self.with_mounted_widget(idx, |game, entry| match &mut entry.kind {
            MountedWidgetKind::TextField {
                active, visible, ..
            } => *active && *visible,
            MountedWidgetKind::Custom(widget) => call_owner(
                &entry.owner,
                &mut entry.disabled,
                "screen_widget_mouse_released",
                || widget.mouse_released(game, mouse, button, modifiers),
            )
            .unwrap_or(false),
            MountedWidgetKind::Button { .. } => true,
        })
        .unwrap_or(false)
    }

    /// Dispatch pointer dragging to the mounted widget currently being dragged.
    pub(in crate::game::client_side) fn mounted_widget_mouse_dragged(
        &mut self,
        mouse: [f32; 2],
        button: i32,
        delta: [f32; 2],
        modifiers: u32,
    ) -> bool {
        let Some((idx, drag_button)) = self.client.screen_state.0.widgets.inner.borrow().dragging
        else {
            return false;
        };
        if drag_button != button || self.client.screen_state.0.widgets.focused_index() != Some(idx)
        {
            return false;
        }
        self.with_mounted_widget(idx, |game, entry| match &mut entry.kind {
            MountedWidgetKind::TextField {
                edit_box,
                active,
                visible,
                ..
            } => {
                if !*active || !*visible {
                    return false;
                }
                edit_box.drag_to(&game.client.gui.atlas, mouse[0]);
                true
            }
            MountedWidgetKind::Custom(widget) => call_owner(
                &entry.owner,
                &mut entry.disabled,
                "screen_widget_mouse_dragged",
                || widget.mouse_dragged(game, mouse, button, delta, modifiers),
            )
            .unwrap_or(false),
            MountedWidgetKind::Button { .. } => true,
        })
        .unwrap_or(false)
    }

    /// Dispatch a mouse wheel scroll to visible mounted custom widgets under `mouse`.
    pub(in crate::game::client_side) fn mounted_widget_mouse_scrolled(
        &mut self,
        mouse: [f32; 2],
        scroll: [f64; 2],
        modifiers: u32,
    ) -> bool {
        let len = self
            .client
            .screen_state
            .0
            .widgets
            .inner
            .borrow()
            .entries
            .len();
        if len == 0 {
            return false;
        }
        let generation = self.client.screen_state.0.widgets.generation();
        for idx in 0..len {
            if self.client.screen_state.0.widgets.generation() != generation {
                return false;
            }
            let is_custom = matches!(
                self.client.screen_state.0
                    .widgets
                    .inner
                    .borrow()
                    .entries
                    .get(idx)
                    .and_then(Option::as_ref),
                Some(entry)
                    if !entry.disabled
                        && entry.owner.is_enabled()
                        && matches!(entry.kind, MountedWidgetKind::Custom(_))
            );
            if !is_custom {
                continue;
            }
            let handled = self
                .with_mounted_widget(idx, |game, entry| {
                    if let MountedWidgetKind::Custom(widget) = &mut entry.kind {
                        call_owner(
                            &entry.owner,
                            &mut entry.disabled,
                            "screen_widget_mouse_scrolled",
                            || {
                                let rect = widget.bounds();
                                if widget.visible()
                                    && widget.active()
                                    && valid_widget_rect(rect)
                                    && point_in_rect(mouse, rect)
                                {
                                    widget.mouse_scrolled(game, mouse, scroll, modifiers)
                                } else {
                                    false
                                }
                            },
                        )
                        .unwrap_or(false)
                    } else {
                        false
                    }
                })
                .unwrap_or(false);
            if self.client.screen_state.0.widgets.generation() != generation {
                return handled;
            }
            if handled {
                return true;
            }
        }
        false
    }

    /// Dispatch a key press to the focused mounted widget.
    /// Tab/Shift+Tab and arrow traversal are handled by the unified screen
    /// focus path (`key_focus_key`), and Escape is never swallowed so vanilla
    /// `Screen.keyPressed`'s `shouldCloseOnEsc` always wins.
    pub fn mounted_widget_key_pressed(
        &mut self,
        key: KeyCode,
        repeat: bool,
        modifiers: u32,
    ) -> bool {
        if self.client.screen_state.0.widgets.is_empty() || key == KeyCode::Escape {
            return false;
        }
        let Some(idx) = self.client.screen_state.0.widgets.focused_index() else {
            return false;
        };
        self.with_mounted_widget(idx, |game, entry| match &mut entry.kind {
            MountedWidgetKind::Button { state, .. } => {
                if !state.visible
                    || !state.active
                    || !matches!(key, KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space)
                {
                    return false;
                }
                game.play_audio_button_click();
                entry.press_button(game);
                true
            }
            MountedWidgetKind::TextField {
                edit_box,
                active,
                visible,
                on_change,
                on_submit,
                ..
            } => {
                if !*visible || !*active {
                    return false;
                }
                if matches!(key, KeyCode::Enter | KeyCode::NumpadEnter) {
                    if let Some(callback) = on_submit.as_mut() {
                        let value = edit_box.value.as_str();
                        call_owner(
                            &entry.owner,
                            &mut entry.disabled,
                            "text_field_submit",
                            || callback(value, game),
                        );
                        return true;
                    }
                    return false;
                }
                let shift = (modifiers & 1) != 0;
                let alt = (modifiers & 4) != 0;
                let ctrl_or_cmd = if cfg!(target_os = "macos") {
                    (modifiers & 8) != 0
                } else {
                    (modifiers & 2) != 0
                };
                if ctrl_or_cmd && !shift && !alt {
                    let shortcut = match key {
                        KeyCode::KeyA => Some(EditShortcut::SelectAll),
                        KeyCode::KeyC => Some(EditShortcut::Copy),
                        KeyCode::KeyV => Some(EditShortcut::Paste),
                        KeyCode::KeyX => Some(EditShortcut::Cut),
                        _ => None,
                    };
                    if let Some(shortcut) = shortcut {
                        let changed = edit_box.shortcut(shortcut, &mut game.client.clipboard.0);
                        if changed && let Some(callback) = on_change.as_mut() {
                            let value = edit_box.value.as_str();
                            call_owner(
                                &entry.owner,
                                &mut entry.disabled,
                                "text_field_change",
                                || callback(value, game),
                            );
                        }
                        return true;
                    }
                }
                if key == KeyCode::KeyA {
                    return false;
                }
                let track_change = matches!(key, KeyCode::Backspace | KeyCode::Delete);
                let before = track_change.then(|| edit_box.value.clone());
                let handled = edit_box.key(key, ctrl_or_cmd, shift);
                if handled
                    && let Some(before) = before
                    && edit_box.value != before
                    && let Some(callback) = on_change.as_mut()
                {
                    let value = edit_box.value.as_str();
                    call_owner(
                        &entry.owner,
                        &mut entry.disabled,
                        "text_field_change",
                        || callback(value, game),
                    );
                }
                handled
            }
            MountedWidgetKind::Custom(widget) => {
                let Some(name) = keybinds::key_name(key) else {
                    return false;
                };
                call_owner(
                    &entry.owner,
                    &mut entry.disabled,
                    "screen_widget_key_pressed",
                    || {
                        if widget.visible() && widget.active() {
                            widget.key_pressed(game, name, repeat, modifiers)
                        } else {
                            false
                        }
                    },
                )
                .unwrap_or(false)
            }
        })
        .unwrap_or(false)
    }

    /// Dispatch a key release to the focused mounted custom widget.
    pub fn mounted_widget_key_released(&mut self, key: KeyCode, modifiers: u32) -> bool {
        let Some(idx) = self.client.screen_state.0.widgets.focused_index() else {
            return false;
        };
        let Some(name) = keybinds::key_name(key) else {
            return false;
        };
        self.with_mounted_widget(idx, |game, entry| match &mut entry.kind {
            MountedWidgetKind::Custom(widget) => call_owner(
                &entry.owner,
                &mut entry.disabled,
                "screen_widget_key_released",
                || {
                    if widget.visible() && widget.active() {
                        widget.key_released(game, name, modifiers)
                    } else {
                        false
                    }
                },
            )
            .unwrap_or(false),
            _ => false,
        })
        .unwrap_or(false)
    }

    /// Dispatch a typed or IME-committed character to the focused mounted widget.
    pub fn mounted_widget_char_typed(&mut self, c: char, modifiers: u32) -> bool {
        let Some(idx) = self.client.screen_state.0.widgets.focused_index() else {
            return false;
        };
        self.with_mounted_widget(idx, |game, entry| match &mut entry.kind {
            MountedWidgetKind::TextField {
                edit_box,
                active,
                visible,
                on_change,
                ..
            } => {
                if !*visible || !*active || !edit_box.editable || !is_allowed_chat_character(c) {
                    return false;
                }
                let mut buf = [0u8; 4];
                let text = c.encode_utf8(&mut buf);
                let before = edit_box.value.clone();
                edit_box.insert_text(text);
                if edit_box.value != before
                    && let Some(callback) = on_change.as_mut()
                {
                    let value = edit_box.value.as_str();
                    call_owner(
                        &entry.owner,
                        &mut entry.disabled,
                        "text_field_change",
                        || callback(value, game),
                    );
                }
                true
            }
            MountedWidgetKind::Custom(widget) => call_owner(
                &entry.owner,
                &mut entry.disabled,
                "screen_widget_char_typed",
                || {
                    if widget.visible() && widget.active() {
                        widget.char_typed(game, c, modifiers)
                    } else {
                        false
                    }
                },
            )
            .unwrap_or(false),
            MountedWidgetKind::Button { .. } => false,
        })
        .unwrap_or(false)
    }

    /// Whether the focused mounted widget handles IME preedit
    /// (`ContainerEventHandler.preeditUpdated` -> `EditBox.preeditUpdated`).
    /// Only an editable, visible, active built-in `TextField` handles preedit;
    /// custom `ScreenWidget`s have no preedit callback and return `false` even
    /// if they return `true` from `captures_input()`.
    pub(in crate::game::client_side) fn mounted_widget_preedit(
        &self,
        _event: Option<&PreeditEvent>,
    ) -> bool {
        let inner = self.client.screen_state.0.widgets.inner.borrow();
        let Some(idx) = inner.focused else {
            return false;
        };
        let Some(Some(entry)) = inner.entries.get(idx) else {
            return false;
        };
        if entry.disabled || !entry.owner.is_enabled() {
            return false;
        }
        matches!(
            &entry.kind,
            MountedWidgetKind::TextField {
                edit_box,
                active,
                visible,
                ..
            } if *active && *visible && edit_box.editable
        )
    }

    /// Read and optionally apply a `TextEdit` rewrite from
    /// `screen_char_typed_pre` for a focused mounted built-in `TextField`.
    /// Custom `ScreenWidget`s own their own state and return `None`.
    pub fn mounted_focused_text_edit(&mut self, edited: Option<&TextEdit>) -> Option<TextEdit> {
        let idx = self.client.screen_state.0.widgets.focused_index()?;
        self.with_mounted_widget(idx, |game, entry| match &mut entry.kind {
            MountedWidgetKind::TextField {
                edit_box,
                active,
                visible,
                on_change,
                ..
            } if *active && *visible && edit_box.editable => {
                if let Some(edited) = edited {
                    let before = edit_box.value.clone();
                    if edit_box.value != edited.value {
                        edit_box.select_all();
                        edit_box.insert_text(&edited.value);
                    }
                    edit_box.set_cursor(edited.highlight, false);
                    edit_box.set_cursor(edited.cursor, true);
                    if edit_box.value != before
                        && let Some(callback) = on_change.as_mut()
                    {
                        let value = edit_box.value.as_str();
                        call_owner(
                            &entry.owner,
                            &mut entry.disabled,
                            "text_field_change",
                            || callback(value, game),
                        );
                    }
                }
                Some(edit_box.edit.clone())
            }
            _ => None,
        })
        .flatten()
    }

    /// Whether the focused mounted widget captures text input (`Screen.isInputCaptured` /
    /// `EditBox.canConsumeInput`).
    pub fn mounted_widget_captures_input(&self) -> bool {
        let inner = self.client.screen_state.0.widgets.inner.borrow();
        let Some(idx) = inner.focused else {
            return false;
        };
        let Some(Some(entry)) = inner.entries.get(idx) else {
            return false;
        };
        if entry.disabled || !entry.owner.is_enabled() {
            return false;
        }
        match &entry.kind {
            MountedWidgetKind::TextField {
                edit_box,
                active,
                visible,
                ..
            } => *active && *visible && edit_box.editable,
            MountedWidgetKind::Custom(widget) => entry
                .owner
                .call("screen widget", "screen_widget_captures_input", || {
                    widget.visible() && widget.active() && widget.captures_input()
                })
                .unwrap_or(false),
            MountedWidgetKind::Button { .. } => false,
        }
    }
}
