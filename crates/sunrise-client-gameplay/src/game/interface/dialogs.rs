//! A server's dialogs bound to the game (`ClientCommonPacketListenerImpl.
//! showDialog` and `clearDialog`, `DialogScreen`, its `WarningScreen` and the
//! `WaitingForResponseScreen`): the screen over whatever is open (the
//! connecting screen in configuration), its inputs, and the actions its
//! buttons send back.
use super::remote_chat::Behind;
use crate::game::ClientGame;
use std::{sync::Arc, time::Instant};
use sunrise_client::{
    clipboard::EditShortcut,
    confirm_screen::Confirm,
    dialog_screen::{BOX, Layout, Part, WARNING_SPRITES, waiting_button},
    font::Font,
    hud::{HudBatch, rgba},
    multiline_edit::MultiLineEdit,
    ui,
    widgets::{self, Kind as WidgetKind, ScreenInfo, Target},
};
use sunrise_core::nbt::Tag;
use sunrise_game_data::{
    Result,
    text::{ClickEvent, Text},
};
use sunrise_protocol::registry::Dynamic;
use sunrise_sim::dialog::{
    AfterAction, Body, Click, Control, Dialog, InputValue, Kind, text_height,
};
use winit::keyboard::KeyCode;

/// An open `DialogScreen`.
struct Open {
    dialog: Dialog,
    /// A configuration screen and the dialogs opened from it use the same snapshot.
    configuration: Option<Arc<Dynamic>>,
    values: Vec<InputValue>,
    /// The box of each text input.
    fields: Vec<Option<Field>>,
    focus: Option<usize>,
    scroll: ui::Scroll,
    /// The number range input whose slider is held.
    dragging: Option<usize>,
    /// The text input the pointer is selecting in.
    selecting: Option<usize>,
}

/// A dialog a clicked command closed, kept for its confirmation's no.
pub(in crate::game) struct ClosedDialog(Box<Open>);

/// A text input's control: `EditBox`, or `MultiLineEditBox` for a multiline one.
enum Field {
    Line(ui::EditBox),
    Multi(Box<MultiLineEdit>),
}
impl Field {
    fn value(&self) -> String {
        match self {
            Self::Line(edit) => edit.value.clone(),
            Self::Multi(edit) => edit.value(),
        }
    }
    fn insert_text(&mut self, font: &Font, text: &str) {
        match self {
            Self::Line(edit) => edit.insert_text(text),
            Self::Multi(edit) => edit.insert_text(font, text),
        }
    }
    fn key(&mut self, font: &Font, key: KeyCode, ctrl: bool, shift: bool) -> bool {
        match self {
            Self::Line(edit) => edit.key(key, ctrl, shift),
            Self::Multi(edit) => edit.key(font, key, ctrl, shift),
        }
    }
    fn shortcut(
        &mut self,
        font: &Font,
        shortcut: EditShortcut,
        clipboard: &mut dyn sunrise_client::clipboard::Clipboard,
    ) {
        match self {
            Self::Line(edit) => {
                edit.shortcut(shortcut, clipboard);
            }
            Self::Multi(edit) => {
                edit.shortcut(font, shortcut, clipboard);
            }
        }
    }
}

/// The dialog screens a server can open.
#[derive(Default)]
pub(in crate::game) struct DialogState {
    open: Option<Open>,
    revision: u64,
    /// `WaitingForResponseScreen`, since when.
    waiting: Option<Instant>,
    /// `DialogScreen.WarningScreen`, over the dialog.
    warning: bool,
}

impl DialogState {
    /// `Minecraft.pause` over these screens: `DialogScreen.isPauseScreen` is
    /// the dialog's own `pause` (`CommonDialogData.pause`, true by default),
    /// `WaitingForResponseScreen` overrides it false, and the warning above
    /// the dialog is a `ConfirmScreen`, which inherits `Screen`'s true.
    pub(in crate::game::client_side) fn pauses_world(&self) -> bool {
        if self.warning {
            return true;
        }
        if self.waiting.is_some() {
            return false;
        }
        self.open.as_ref().is_some_and(|open| open.dialog.pause)
    }
}

/// `CycleButton.cycleValue`: the next option (`delta` 1) or the one before
/// it, wrapping.
fn cycle(open: &mut Open, input: usize, delta: i32) {
    let Control::SingleOption { options, .. } = &open.dialog.inputs[input].control else {
        return;
    };
    if let Some(InputValue::Option(i)) = open.values.get_mut(input) {
        *i = (*i as i32 + delta).rem_euclid(options.len().max(1) as i32) as usize;
    }
}

#[cfg(feature = "client")]
impl ClientGame<'_> {
    /// `DialogScreen.WarningScreen` uses the local-server disconnect label.
    pub fn warning_texts(&self) -> (Text, Text, [Text; 2]) {
        let t = |key: &str| Text::translate(key, &[]);
        let disconnect = if self.external_server() {
            "menu.disconnect"
        } else {
            "menu.returnToMenu"
        };
        (
            t("menu.custom_screen_info.title"),
            t("menu.custom_screen_info.contents"),
            [t(disconnect), t("gui.back")],
        )
    }

    /// Whether a dialog screen takes the input.
    pub fn dialog_open(&self) -> bool {
        let d = &self.client.dialogs.0;
        d.open.is_some() || d.waiting.is_some() || d.warning
    }
    /// What tells one dialog screen from another for a controller's focus: a new dialog is a
    /// new screen, and so are the warning and waiting screens.
    pub fn dialog_screen_key(&self) -> (bool, bool, Option<u64>) {
        let d = &self.client.dialogs.0;
        (
            d.warning,
            d.waiting.is_some(),
            d.open.as_ref().map(|_| d.revision),
        )
    }
    /// The warning icon in the dialog's corner, while the dialog itself shows.
    pub fn dialog_warning_icon(&self) -> Option<[f32; 4]> {
        let d = &self.client.dialogs.0;
        if d.warning || d.waiting.is_some() {
            return None;
        }
        self.dialog_layout().map(|layout| layout.warning)
    }
    /// The focused dialog input, including the initial keyboard focus before any click.
    pub(in crate::game::client_side) fn dialog_navigation_focus(&self) -> Option<[f32; 4]> {
        if self.client.dialogs.0.warning || self.client.dialogs.0.waiting.is_some() {
            return None;
        }
        let open = self.client.dialogs.0.open.as_ref()?;
        let focus = open.focus?;
        self.dialog_layout()?
            .placed(open.scroll.amount)
            .find_map(|(part, _)| match part {
                Part::Field { input, rect } | Part::Slider { input, rect } if input == focus => {
                    Some(rect)
                }
                _ => None,
            })
    }

    /// Dialog body controls are added before the footer; the warning button
    /// is added last, but has its own earlier tab group.
    pub(in crate::game::client_side) fn dialog_navigation_order(&self) -> Option<Vec<[f32; 4]>> {
        let warning = self.dialog_warning_icon()?;
        let open = self.client.dialogs.0.open.as_ref()?;
        let layout = self.dialog_layout()?;
        let order = layout
            .placed(open.scroll.amount)
            .filter_map(|(part, _)| match part {
                Part::Field { rect, .. }
                | Part::Check { rect, .. }
                | Part::Slider { rect, .. }
                | Part::Cycle { rect, .. }
                | Part::Button { rect, .. } => Some(rect),
                _ => None,
            });
        Some(order.chain([warning]).collect())
    }

    /// Apply a focus path without clicking the dialog's editor or changing its caret.
    pub(in crate::game::client_side) fn set_dialog_navigation_focus(&mut self, target: Target) {
        let focus = if matches!(target.kind, WidgetKind::Field | WidgetKind::Slider { .. }) {
            self.client.dialogs.0.open.as_ref().and_then(|open| {
                self.dialog_layout()?
                    .placed(open.scroll.amount)
                    .find_map(|(part, _)| match part {
                        Part::Field { input, rect } | Part::Slider { input, rect }
                            if rect == target.rect =>
                        {
                            Some(input)
                        }
                        _ => None,
                    })
            })
        } else {
            None
        };
        if !self.client.dialogs.0.warning
            && self.client.dialogs.0.waiting.is_none()
            && let Some(open) = self.client.dialogs.0.open.as_mut()
        {
            open.focus = focus;
        }
    }

    /// The dialog itself shows, with a text input to start on.
    pub(in crate::game::client_side) fn dialog_starts_typing(&self) -> bool {
        !self.client.dialogs.0.warning
            && self.client.dialogs.0.waiting.is_none()
            && self.dialog_has_text_field()
    }
    /// The dialog's focus is a text input of many lines, where Enter is a new line.
    pub(in crate::game::client_side) fn dialog_typing_lines(&self) -> bool {
        self.client.dialogs.0.open.as_ref().is_some_and(|o| {
            let field = o.focus.and_then(|f| o.fields.get(f));
            matches!(field, Some(Some(Field::Multi(_))))
        })
    }
    /// Whether the dialog's focus is one of its text inputs, which takes the
    /// keys.
    pub fn dialog_typing(&self) -> bool {
        let d = &self.client.dialogs.0;
        !d.warning
            && d.waiting.is_none()
            && d.open.as_ref().is_some_and(|o| {
                o.focus
                    .and_then(|f| o.fields.get(f))
                    .is_some_and(Option::is_some)
            })
    }

    pub fn decode_dialog(&self, tag: &Tag, dynamic: Option<&Dynamic>) -> Option<Dialog> {
        let items = self
            .client
            .remote
            .as_ref()
            .and_then(|r| r.builtin.get("minecraft:item"));
        Dialog::parse_in_registry(
            tag,
            &self.common.ui.registries,
            &|id| {
                items.map_or_else(
                    || self.common.res.item_defaults().entries().contains_key(id),
                    |items| items.id(id).is_some(),
                )
            },
            dynamic,
        )
    }

    pub fn show_dialog(&mut self, tag: &Tag) {
        self.show_dialog_in_phase(tag, None);
    }

    /// `showDialog`: the dialog replaces an open one (and a waiting screen);
    /// under the warning screen, it is what Back returns to.
    pub(in crate::game::client_side) fn show_dialog_in_phase(
        &mut self,
        tag: &Tag,
        configuration: Option<Arc<Dynamic>>,
    ) {
        let dynamic = configuration
            .as_deref()
            .or_else(|| self.client.remote.as_ref().map(|r| r.registries.as_ref()));
        let Some(mut dialog) = self.decode_dialog(tag, dynamic) else {
            log::warn!("Sunrise: failed to show dialog for data {tag:?}");
            return;
        };
        if let Kind::DialogList { dialogs, .. } = &dialog.kind {
            let entries = dynamic.map(|registry| {
                let by_id = |name: &str| {
                    registry
                        .data("minecraft:dialog", registry.id("minecraft:dialog", name)?)
                        .cloned()
                };
                let by_tag = |name: &str| {
                    registry
                        .tag("minecraft:dialog", name)
                        .into_iter()
                        .flatten()
                        .filter_map(|&id| registry.data("minecraft:dialog", id).cloned())
                        .collect()
                };
                Dialog::list_entries(dialogs, &by_tag, &by_id)
            });
            dialog.set_list(&entries.unwrap_or_default(), |tag| {
                self.decode_dialog(tag, dynamic)
            });
        }
        self.open_dialog(dialog, configuration);
    }

    /// The parsed dialog's screen, its inputs at their initial values.
    fn open_dialog(&mut self, dialog: Dialog, configuration: Option<Arc<Dynamic>>) {
        let values = dialog.inputs.iter().map(|i| i.control.initial()).collect();
        let font = &self.client.gui.atlas.font;
        let fields = dialog
            .inputs
            .iter()
            .map(|i| match &i.control {
                Control::Text {
                    width,
                    initial,
                    max_length,
                    multiline: Some((max_lines, height)),
                    ..
                } => {
                    // TextInputHandler.addControl takes an explicit height before computing a default.
                    let height = text_height(Some((*max_lines, *height)));
                    let mut edit = MultiLineEdit::new(font, 0.0, 0.0, *width as f32, height);
                    edit.set_character_limit((*max_length).max(0) as usize);
                    if let Some(lines) = max_lines {
                        edit.set_line_limit((*lines).max(0) as usize);
                    }
                    edit.set_value(font, initial);
                    Some(Field::Multi(Box::new(edit)))
                }
                Control::Text {
                    width,
                    initial,
                    max_length,
                    ..
                } => {
                    let mut edit = ui::EditBox::new(0.0, 0.0, *width as f32, 20.0);
                    edit.max_length = (*max_length).max(1) as usize;
                    edit.set_value(initial);
                    edit.focused = false;
                    Some(Field::Line(edit))
                }
                _ => None,
            })
            .collect();
        self.open_screen_overlay("dialog", |game| {
            game.client.dialogs.0.revision = game.client.dialogs.0.revision.wrapping_add(1);
            game.client.dialogs.0.open = Some(Open {
                dialog,
                configuration,
                values,
                fields,
                focus: None,
                scroll: ui::Scroll::default(),
                dragging: None,
                selecting: None,
            });
            game.client.dialogs.0.waiting = None;
            game.clear_device_keys();
            game.client.physical_keys.clear();
            game.set_mapping_held("key.attack", false);
            game.set_mapping_held("key.use", false);
            game.capture(false);
        });
    }

    /// `clearDialog`: the dialog closes (under the warning screen, Back then
    /// returns to what was open before it).
    pub fn clear_dialog(&mut self) {
        if self.client.dialogs.0.open.take().is_some() && !self.dialog_open() {
            self.dialog_receiver_changed();
        }
    }

    /// Every dialog screen goes (a disconnect, or the level loading screen).
    pub fn close_dialogs(&mut self) {
        self.client.dialogs.0 = DialogState {
            revision: self.client.dialogs.0.revision,
            ..DialogState::default()
        };
    }

    fn dialog_receiver_changed(&mut self) {
        self.refresh_screen_widgets();
        if self.common.player.is_some()
            && self.client.screen == super::Screen::Playing
            && !self.screen_over_playing()
            && self.client.title.0.is_none()
        {
            self.capture(true);
        }
    }

    /// The input values as the controls hold them now.
    fn dialog_values(open: &mut Open) {
        for (value, field) in open.values.iter_mut().zip(&open.fields) {
            if let Some(f) = field {
                *value = InputValue::Text(f.value());
            }
        }
    }

    /// The confirmation's no: the dialog again, with what was typed in it.
    pub(in crate::game::client_side) fn reopen_dialog(&mut self, dialog: ClosedDialog) {
        self.open_screen_overlay("dialog", |game| {
            game.client.dialogs.0.revision = game.client.dialogs.0.revision.wrapping_add(1);
            game.client.dialogs.0.open = Some(*dialog.0);
            game.client.dialogs.0.waiting = None;
        });
    }

    /// `DialogScreen.runAction`: the click event, then the screen the dialog's
    /// `after_action` leaves.
    fn run_dialog_action(&mut self, click: Option<Click>, after: AfterAction) -> Result<()> {
        // defaultHandleClickEvent: an open_url never sets the screen itself, so
        // the dialog stays unless the confirmation takes over (Web Links off,
        // or Prompt on Links off, opens nothing or the browser behind it).
        if let Some(Click::OpenUrl(url)) = &click
            && !(self.client.settings.chat_links && self.client.settings.chat_links_prompt)
        {
            if self.client.settings.chat_links {
                super::packs::open_uri(url);
            }
            return Ok(());
        }
        let before = self.dialog_screen_key();
        let mut wait = after == AfterAction::WaitForResponse;
        let configuration = match click {
            Some(Click::ShowDialog(_)) => self
                .client
                .dialogs
                .0
                .open
                .as_ref()
                .and_then(|o| o.configuration.clone()),
            _ => None,
        };
        let mut closing = None;
        if after == AfterAction::Close {
            let open = self.client.dialogs.0.open.take();
            if matches!(&click, Some(Click::RunCommand(_))) {
                closing = open;
            }
        }
        match click {
            Some(Click::Custom { id, payload }) => {
                if let Some(r) = &self.client.remote {
                    r.session.send_custom_click(&id, payload);
                }
            }
            Some(Click::RunCommand(command)) => {
                // In configuration there is nothing to run it (vanilla warns).
                if self.common.player.is_some() {
                    let sent = self.send_unattended_command(&command);
                    if wait {
                        // openSendConfirmationWindow's `currentScreen`: Back
                        // returns to this dialog, not the screen after the command.
                        let prompted = self.client.command_prompt.0.is_some();
                        if prompted {
                            closing = self.client.dialogs.0.open.take();
                        }
                        wait = sent || prompted;
                    }
                    if let Some(open) = closing {
                        self.command_prompt_returns_to(Behind::Dialog(ClosedDialog(Box::new(
                            open,
                        ))));
                    }
                } else {
                    log::warn!(
                        "Sunrise: commands are not supported in configuration, trying to run '{command}'"
                    );
                    wait = false;
                }
            }
            Some(Click::ShowDialog(dialog)) => {
                // showDialog opens its own receiver, not the waiting screen.
                self.show_dialog_ref_in_phase(&dialog, configuration);
                wait = false;
            }
            Some(Click::CopyToClipboard(value)) => {
                use sunrise_client::clipboard::Clipboard;
                self.client.clipboard.0.set(&value);
            }
            // ConfirmLinkScreen over the screen the dialog leaves. If the
            // confirmation was vetoed, neither it nor its return screen opens.
            Some(Click::OpenUrl(url)) => {
                self.click_url(url);
                wait &= self.client.link_prompt.is_some();
            }
            // defaultHandleClickEvent: `activeScreen.insertText(command, true)`.
            // Only the chat screen (opened under the dialog) takes text.
            Some(Click::SuggestCommand(command)) => {
                if !self.dialog_open() && self.client.chat_ui.chat_screen.is_some() {
                    let m = sunrise_client::screen_draw::Measure(&self.client.gui.atlas.font);
                    let c = sunrise_client::chat::Commands {
                        dispatcher: super::chat::chat_tree(&self.client.remote),
                        registries: &self.common.ui.registries,
                    };
                    if let Some(screen) = self.client.chat_ui.chat_screen.as_mut() {
                        screen.suggest_command(&m, &c, &command);
                    }
                }
            }
            Some(other) => log::error!("Don't know how to handle {other:?}"),
            None => {}
        }
        if wait {
            let start_waiting = |game: &mut Self| {
                game.client.dialogs.0.open = None;
                game.client.dialogs.0.waiting = Some(Instant::now());
            };
            if self.client.command_prompt.0.is_some() || self.client.link_prompt.is_some() {
                // The confirmation already became the receiver. Stage only
                // the screen it can return to; do not mount it under the prompt.
                start_waiting(self);
            } else {
                self.open_screen_overlay("dialog", start_waiting);
            }
        } else if self.dialog_screen_key() != before {
            self.dialog_receiver_changed();
        }
        Ok(())
    }

    /// A `show_dialog` click event's dialog: a registry id or the dialog itself.
    pub(in crate::game::client_side) fn show_dialog_ref(&mut self, dialog: &Tag) {
        self.show_dialog_ref_in_phase(dialog, None);
    }

    fn show_dialog_ref_in_phase(&mut self, dialog: &Tag, configuration: Option<Arc<Dynamic>>) {
        if let Tag::String(id) = dialog {
            let id = id.to_string_lossy();
            let Ok(name) = sunrise_core::identifier::Identifier::parse(&id) else {
                return;
            };
            let dynamic = configuration
                .as_deref()
                .or_else(|| self.client.remote.as_ref().map(|r| r.registries.as_ref()));
            let found = dynamic.and_then(|registry| {
                let id = registry.id("minecraft:dialog", &name.to_string())?;
                registry.data("minecraft:dialog", id).cloned()
            });
            match found {
                Some(tag) => self.show_dialog_in_phase(&tag, configuration),
                None => log::warn!("Sunrise: unknown dialog {id}"),
            }
        } else {
            self.show_dialog_in_phase(dialog, configuration);
        }
    }

    /// `ClientboundShowDialogPacket` in play: a registry entry or the dialog.
    pub(in crate::game) fn show_play_dialog(
        &mut self,
        dialog: &sunrise_protocol::packets::play::DialogHolder,
    ) {
        use sunrise_protocol::packets::play::DialogHolder;
        match dialog {
            DialogHolder::Direct(tag) => self.show_dialog(tag),
            DialogHolder::Registry(id) => {
                let tag = self
                    .client
                    .remote
                    .as_ref()
                    .and_then(|r| r.registries.data("minecraft:dialog", *id).cloned());
                match tag {
                    Some(tag) => self.show_dialog(&tag),
                    None => log::warn!("Sunrise: unknown dialog {id}"),
                }
            }
        }
    }

    fn dialog_layout(&self) -> Option<Layout> {
        let open = self.client.dialogs.0.open.as_ref()?;
        Some(Layout::new(
            &open.dialog,
            &self.client.gui.atlas.font,
            self.gui_size(),
        ))
    }

    pub(in crate::game::client_side) fn draw_dialog(&mut self, batch: &mut HudBatch) {
        if !self.dialog_open() {
            return;
        }
        widgets::cover_frame();
        let (w, h) = (batch.width, batch.height);
        let cursor = self.client.cursor;
        let atlas = &self.client.gui.atlas;
        draw_background(batch, atlas, self.common.player.is_some());
        if self.client.dialogs.0.warning {
            let (title, message, [disconnect, back]) = self.warning_texts();
            // DialogScreen.WarningScreen, a ConfirmScreen: its title and its
            // message.
            widgets::begin_screen(|| {
                ScreenInfo::new(
                    "dialog/warning",
                    widgets::join_for_narration(&[&title.plain(), &message.plain()]),
                )
            });
            let mut confirm = Confirm::new(&atlas.font, (w, h), &title, &message);
            confirm.declare_no();
            confirm.draw(batch, atlas, cursor, [&disconnect, &back]);
            self.draw_overlay_mounted_widgets(batch, "dialog");
            return;
        }
        if let Some(since) = self.client.dialogs.0.waiting {
            // HeaderAndFooterLayout(33, 0) centred: the title, the button 33 below.
            let ticks = (since.elapsed().as_millis() / 50) as u32;
            let (label, visible, active) = waiting_button(ticks);
            let title = Text::translate("gui.waitingForResponse.title", &[]);
            widgets::begin_screen(|| ScreenInfo::new("dialog/waiting", title.plain()));
            // WaitingForResponseScreen.tick: the close button turning active
            // after five seconds is narrated at once.
            if active && ticks < 102 {
                widgets::narrate_now();
            }
            let top = ((h - 53.0) / 2.0).trunc();
            let tw = atlas.font.width_text(&title);
            batch.text_styled(
                atlas,
                &title,
                ((w - tw) / 2.0).trunc(),
                top + 12.0,
                u32::MAX,
                true,
            );
            if visible {
                let (bx, by) = (((w - 200.0) / 2.0).trunc(), top + 33.0);
                let hover = ui::hovered(cursor, bx, by, 200.0, 20.0);
                ui::action_button(
                    batch,
                    atlas,
                    widgets::Action::Back,
                    bx,
                    by,
                    200.0,
                    20.0,
                    &label,
                    active,
                    hover,
                );
            }
            self.draw_overlay_mounted_widgets(batch, "dialog");
            return;
        }
        let Some(layout) = self.dialog_layout() else {
            return;
        };
        // Out of `self` while it draws: an item body draws through `self`.
        let Some(mut open) = self.client.dialogs.0.open.take() else {
            return;
        };
        let atlas = &self.client.gui.atlas;
        let [ax, ay, aw, ah] = layout.area;
        open.scroll.clamp(ay, ay + ah, layout.content);
        // DialogScreen: titled by its dialog; a new dialog is a new screen.
        widgets::begin_screen(|| {
            let title = open.dialog.title.plain();
            ScreenInfo::new(format!("dialog:{title}"), title)
        });
        batch.text_styled(
            atlas,
            &open.dialog.title,
            layout.title.0,
            layout.title.1,
            u32::MAX,
            true,
        );
        let [wx, wy, ww, wh] = layout.warning;
        let warn_hover = ui::hovered(cursor, wx, wy, ww, wh);
        sunrise_client::widgets::record("warning", layout.warning, true);
        // The warning ImageButton: its narration message and tooltip, first
        // in tab order (group -10).
        widgets::narratable(|| {
            let t = |key: &str| Text::translate(key, &[]).plain();
            widgets::Narratable::button(
                t("menu.custom_screen_info.button_narration"),
                layout.warning,
                true,
            )
            .hovered(warn_hover)
            .hint(t("menu.custom_screen_info.tooltip"))
            .order(-10)
        });
        batch.sprite_scaled(
            atlas,
            WARNING_SPRITES[if warn_hover { 2 } else { 0 }],
            wx,
            wy,
            ww,
            wh,
            u32::MAX,
        );
        let buttons = open.dialog.buttons();
        let in_area = |y: f32| y >= ay && y < ay + ah;
        let mut tooltip = None;
        let mut item_tip = None;
        // The hovered text's style, drawn last as componentHoverEffect draws it.
        let mut hovered = None;
        let mut start = None;
        let mut mark = None;
        for (part, clipped) in layout.placed(open.scroll.amount) {
            if clipped && start.is_none() {
                start = Some(batch.vertices.len());
                mark = Some(widgets::frame_mark());
            } else if !clipped && let Some(s) = start.take() {
                batch.clip_since(s, ax, ay, ax + aw, ay + ah);
                if let Some(mark) = mark.take() {
                    widgets::clip_frame(mark, ay, ay + ah);
                }
            }
            match part {
                Part::Line(text, x, y) => {
                    batch.text_styled(atlas, &text, x, y, u32::MAX, true);
                    // AbstractStringWidget: only the styled glyph under the
                    // pointer requests a hand, not the whole message widget.
                    let under = |wanted: fn(&sunrise_game_data::text::Style) -> bool| {
                        sunrise_client::dialog_screen::line_style_at(
                            &atlas.font,
                            &text,
                            x,
                            y,
                            cursor,
                            wanted,
                        )
                    };
                    if (!clipped || in_area(cursor.1))
                        && let Some(style) = under(|_| true)
                    {
                        widgets::widget_cursor(style.click.is_some(), true);
                        hovered = Some(style);
                    }
                }
                Part::Field { input, rect } => {
                    let focused = open.focus == Some(input);
                    // The edit box is named after its input's label.
                    let label = match &open.dialog.inputs[input].control {
                        Control::Text { label, .. } if widgets::narrating() => label.plain(),
                        _ => String::new(),
                    };
                    match open.fields.get_mut(input) {
                        Some(Some(Field::Line(edit))) => {
                            [edit.x, edit.y, edit.width, edit.height] = rect;
                            edit.focused = focused;
                            if widgets::narrating() {
                                edit.message = label;
                            }
                            edit.draw(batch, atlas, None);
                        }
                        Some(Some(Field::Multi(edit))) => {
                            [edit.x, edit.y, edit.width, edit.height] = rect;
                            edit.set_focused(focused);
                            edit.draw(batch, atlas);
                            // MultiLineEditBox.updateWidgetNarration: as an EditBox.
                            widgets::narratable(|| {
                                widgets::Narratable::edit_box(label, edit.value(), rect)
                                    .focused(focused)
                            });
                        }
                        _ => {}
                    }
                }
                Part::Check { input, rect } => {
                    let [x, y, cw, ch] = rect;
                    let hover = in_area(cursor.1) && ui::hovered(cursor, x, y, cw, ch);
                    let on = open.values.get(input) == Some(&InputValue::Bool(true));
                    let sprite = match (on, hover) {
                        (true, true) => "widget/checkbox_selected_highlighted",
                        (true, false) => "widget/checkbox_selected",
                        (false, true) => "widget/checkbox_highlighted",
                        (false, false) => "widget/checkbox",
                    };
                    let Control::Boolean { label, .. } = &open.dialog.inputs[input].control else {
                        continue;
                    };
                    sunrise_client::widgets::record(&label.plain(), rect, true);
                    widgets::narratable(|| {
                        widgets::Narratable::checkbox(label.plain(), on, rect, true).hovered(hover)
                    });
                    batch.sprite_scaled(atlas, sprite, x, y, BOX, BOX, u32::MAX);
                    batch.text_styled(
                        atlas,
                        label,
                        x + BOX + 4.0,
                        y + 4.0,
                        rgba(224, 224, 224, 255),
                        true,
                    );
                }
                Part::Cycle { input, rect } => {
                    let [x, y, cw, ch] = rect;
                    let hover = in_area(cursor.1) && ui::hovered(cursor, x, y, cw, ch);
                    let Some(label) = open
                        .values
                        .get(input)
                        .and_then(|v| open.dialog.inputs[input].control.message(v))
                    else {
                        continue;
                    };
                    ui::button(batch, atlas, x, y, cw, ch, &label, true, hover);
                    // A CycleButton: the option a click moves to.
                    widgets::last_narratable(|n| {
                        let control = &open.dialog.inputs[input].control;
                        let next = match (control, open.values.get(input)) {
                            (
                                Control::SingleOption { options, .. },
                                Some(InputValue::Option(i)),
                            ) if !options.is_empty() => {
                                let (id, display, _) = &options[(i + 1) % options.len()];
                                display.clone().unwrap_or_else(|| Text::literal(id)).plain()
                            }
                            _ => String::new(),
                        };
                        n.what = widgets::What::Cycle {
                            message: label.plain(),
                            next,
                            active: true,
                        };
                    });
                }
                Part::Slider { input, rect } => {
                    let [x, y, sw, sh] = rect;
                    let hover = in_area(cursor.1) && ui::hovered(cursor, x, y, sw, sh);
                    let Some((InputValue::Slider(position), Some(label))) = open
                        .values
                        .get(input)
                        .map(|v| (v.clone(), open.dialog.inputs[input].control.message(v)))
                    else {
                        continue;
                    };
                    let held = open.dragging == Some(input);
                    ui::slider(
                        batch,
                        atlas,
                        x,
                        y,
                        sw,
                        &label,
                        position,
                        hover || held,
                        true,
                        held,
                    );
                }
                Part::Item { body, rect } => {
                    let [x, y, iw, ih] = rect;
                    let Some(Body::Item {
                        item,
                        show_decorations,
                        show_tooltip,
                        ..
                    }) = open.dialog.body.get(body)
                    else {
                        continue;
                    };
                    let Some(stack) = sunrise_items::item_nbt::item_from_nbt(item) else {
                        continue;
                    };
                    // ItemDisplayWidget: the item at its corner, then its decorations.
                    self.draw_item_model(batch, &stack, x, y, 0);
                    if *show_decorations {
                        self.draw_item_decorations(batch, &stack, x, y, None);
                    }
                    if *show_tooltip && in_area(cursor.1) && ui::hovered(cursor, x, y, iw, ih) {
                        item_tip = Some(stack);
                    }
                }
                Part::Button { index, rect } => {
                    let [x, y, bw, bh] = rect;
                    let hover =
                        (!clipped || in_area(cursor.1)) && ui::hovered(cursor, x, y, bw, bh);
                    let b = buttons[index];
                    if open.dialog.escape_button() == Some(index) {
                        let action = widgets::Action::Cancel;
                        ui::action_button(
                            batch, atlas, action, x, y, bw, bh, &b.label, true, hover,
                        );
                    } else {
                        ui::button(batch, atlas, x, y, bw, bh, &b.label, true, hover);
                    }
                    if let Some(t) = &b.tooltip {
                        widgets::last_narratable(|n| n.hint = Some(t.plain()));
                    }
                    if hover && let Some(t) = &b.tooltip {
                        tooltip = Some(t.clone());
                    }
                }
            }
        }
        if let Some(s) = start {
            batch.clip_since(s, ax, ay, ax + aw, ay + ah);
        }
        if let Some(mark) = mark {
            widgets::clip_frame(mark, ay, ay + ah);
        }
        if layout.content > ah {
            // A controller's focus step goes through the body's hidden parts before it leaves.
            widgets::scroll_area(layout.area, open.scroll.amount, layout.content - ah);
            ui::scrollbar(
                batch,
                atlas,
                ax + aw - 6.0,
                ay,
                ay + ah,
                layout.content,
                open.scroll.amount,
            );
        }
        self.client.dialogs.0.open = Some(open);
        self.draw_overlay_mounted_widgets(batch, "dialog");
        if warn_hover {
            tooltip = Some(Text::translate("menu.custom_screen_info.tooltip", &[]));
        }
        if let Some(t) = tooltip {
            let lines: Vec<Text> = atlas
                .font
                .split_lines(&t, 170.0)
                .into_iter()
                .map(|l| l.0)
                .collect();
            ui::tooltip(batch, atlas, &lines, cursor);
        }
        if let Some(stack) = item_tip {
            self.draw_stack_tooltip(batch, &stack, cursor.0, cursor.1);
        }
        if let Some(style) = hovered {
            self.component_hover_effect(batch, &style, cursor);
        }
    }

    /// `DialogScreen.WarningScreen`: Back opens its return dialog if still live.
    fn return_from_dialog_warning(&mut self) {
        if self.client.dialogs.0.open.is_some() || self.client.dialogs.0.waiting.is_some() {
            self.open_screen_overlay("dialog", |game| game.client.dialogs.0.warning = false);
        } else {
            self.client.dialogs.0.warning = false;
            self.dialog_receiver_changed();
        }
    }

    /// A left click on a dialog screen.
    pub fn dialog_click(&mut self) -> Result<bool> {
        let cursor = self.client.cursor;
        if self.client.dialogs.0.warning {
            let (title, message, _) = self.warning_texts();
            let answer = Confirm::new(
                &self.client.gui.atlas.font,
                self.gui_size(),
                &title,
                &message,
            )
            .answer_at(cursor);
            match answer {
                Some(true) => {
                    self.play_audio_button_click();
                    let reason = Text::translate("menu.custom_screen_info.disconnect", &[]);
                    self.leave_server(Some(reason))?;
                }
                Some(false) => {
                    self.play_audio_button_click();
                    self.return_from_dialog_warning();
                }
                None => {}
            }
            return Ok(answer.is_some());
        }
        if let Some(since) = self.client.dialogs.0.waiting {
            let (w, h) = self.gui_size();
            let active = waiting_button((since.elapsed().as_millis() / 50) as u32).2;
            let (bx, by) = (
                ((w - 200.0) / 2.0).trunc(),
                ((h - 53.0) / 2.0).trunc() + 33.0,
            );
            if active && ui::hovered(cursor, bx, by, 200.0, 20.0) {
                self.play_audio_button_click();
                self.client.dialogs.0.waiting = None;
                self.dialog_receiver_changed();
                return Ok(true);
            }
            return Ok(false);
        }
        let Some(layout) = self.dialog_layout() else {
            return Ok(false);
        };
        let [wx, wy, ww, wh] = layout.warning;
        if ui::hovered(cursor, wx, wy, ww, wh) {
            self.play_audio_button_click();
            self.open_screen_overlay("dialog", |game| game.client.dialogs.0.warning = true);
            return Ok(true);
        }
        let shift = self.client.modifiers.shift_key();
        let Some(open) = self.client.dialogs.0.open.as_mut() else {
            return Ok(false);
        };
        let [_, ay, _, ah] = layout.area;
        let mut action = None;
        let mut text_click: Option<ClickEvent> = None;
        open.focus = None;
        for (part, clipped) in layout.placed(open.scroll.amount) {
            if clipped && !(cursor.1 >= ay && cursor.1 < ay + ah) {
                continue;
            }
            match part {
                Part::Field {
                    input,
                    rect: [x, y, w, h],
                } if ui::hovered(
                    cursor,
                    x,
                    y,
                    // A multiline box answers over its scroll bar too.
                    w + if matches!(open.fields.get(input), Some(Some(Field::Multi(_)))) {
                        6.0
                    } else {
                        0.0
                    },
                    h,
                ) =>
                {
                    open.focus = Some(input);
                    match open.fields.get_mut(input) {
                        Some(Some(Field::Line(edit))) => {
                            edit.focused = true;
                            edit.click(&self.client.gui.atlas, cursor.0, false, false);
                            open.selecting = Some(input);
                        }
                        Some(Some(Field::Multi(edit))) => {
                            [edit.x, edit.y, edit.width, edit.height] = [x, y, w, h];
                            edit.set_focused(true);
                            edit.click(&self.client.gui.atlas.font, cursor, shift, false);
                            open.selecting = Some(input);
                        }
                        _ => {}
                    }
                }
                Part::Check {
                    input,
                    rect: [x, y, w, h],
                } if ui::hovered(cursor, x, y, w, h) => {
                    if let Some(InputValue::Bool(b)) = open.values.get_mut(input) {
                        *b = !*b;
                    }
                    action = Some(None);
                }
                Part::Cycle {
                    input,
                    rect: [x, y, w, h],
                } if ui::hovered(cursor, x, y, w, h) => {
                    // CycleButton.onPress: Shift goes back.
                    cycle(open, input, if shift { -1 } else { 1 });
                    action = Some(None);
                }
                Part::Slider {
                    input,
                    rect: [x, y, w, h],
                } if ui::hovered(cursor, x, y, w, h) => {
                    open.focus = Some(input);
                    open.dragging = Some(input);
                    if let Some(v) = open.values.get_mut(input) {
                        *v = InputValue::Slider(ui::slider_value(x, w, cursor.0) as f32);
                    }
                }
                Part::Button {
                    index,
                    rect: [x, y, w, h],
                } if ui::hovered(cursor, x, y, w, h) => {
                    action = Some(Some(index));
                }
                // AbstractStringWidget.setComponentClickHandler: the click event
                // of the styled text under a centred message line.
                Part::Line(line, x, y) if text_click.is_none() => {
                    text_click = sunrise_client::dialog_screen::line_style_at(
                        &self.client.gui.atlas.font,
                        &line,
                        x,
                        y,
                        cursor,
                        |style| style.click.is_some(),
                    )
                    .and_then(|style| style.click);
                }
                _ => {}
            }
        }
        let Some(action) = action else {
            let focused = open.focus.is_some();
            // A text click runs its event (DialogScreen.runAction) with no button sound.
            let click = match text_click {
                Some(ClickEvent::OpenUrl(url)) => Click::OpenUrl(url),
                Some(ClickEvent::ChangePage(page)) => Click::ChangePage(page),
                Some(ClickEvent::SuggestCommand(command)) => Click::SuggestCommand(command),
                Some(ClickEvent::RunCommand(command)) => Click::RunCommand(command),
                Some(ClickEvent::CopyToClipboard(value)) => Click::CopyToClipboard(value),
                Some(ClickEvent::ShowDialog(dialog)) => {
                    Click::ShowDialog(std::sync::Arc::unwrap_or_clone(dialog))
                }
                Some(ClickEvent::Custom { id, payload }) => Click::Custom {
                    id,
                    payload: payload.map(std::sync::Arc::unwrap_or_clone),
                },
                Some(ClickEvent::OpenFile(_) | ClickEvent::Profiler(_)) | None => {
                    return Ok(focused);
                }
            };
            let after = open.dialog.after_action;
            self.run_dialog_action(Some(click), after)?;
            return Ok(true);
        };
        self.play_audio_button_click();
        let Some(index) = action else {
            return Ok(true);
        };
        let Some(open) = self.client.dialogs.0.open.as_mut() else {
            return Ok(false);
        };
        Self::dialog_values(open);
        let d = &open.dialog;
        let click = d.buttons()[index]
            .action
            .as_ref()
            .and_then(|a| a.click(&d.inputs, &open.values));
        let after = d.after_action;
        self.run_dialog_action(click, after)?;
        Ok(true)
    }

    /// The mouse wheel over a dialog: a cycle button under it takes the
    /// step, else the body scrolls 10 a step.
    pub(in crate::game::client_side) fn dialog_scroll(&mut self, delta: f32) -> bool {
        let Some(layout) = self.dialog_layout() else {
            return false;
        };
        let cursor = self.client.cursor;
        let stick_scroll = self.pad_stick_scroll();
        // A focus step past the body's edge scrolls the body, not a box it's leaving.
        let step_scroll = self.pad_step_scroll();
        if let Some(open) = self.client.dialogs.0.open.as_mut() {
            let [_, ay, _, ah] = layout.area;
            if delta != 0.0 && cursor.1 >= ay && cursor.1 < ay + ah {
                let hovered_cycle =
                    layout
                        .placed(open.scroll.amount)
                        .find_map(|(part, _)| match part {
                            Part::Cycle {
                                input,
                                rect: [x, y, w, h],
                            } if ui::hovered(cursor, x, y, w, h) => Some(input),
                            _ => None,
                        });
                if let Some(input) = hovered_cycle.filter(|_| !stick_scroll) {
                    // CycleButton.mouseScrolled: up goes back.
                    cycle(open, input, if delta > 0.0 { -1 } else { 1 });
                    return true;
                }
                // AbstractScrollArea.mouseScrolled: a multiline box takes the wheel.
                let text = layout
                    .placed(open.scroll.amount)
                    .find_map(|(part, _)| match part {
                        Part::Field { input, rect } => match open.fields.get(input) {
                            Some(Some(Field::Multi(_)))
                                if ui::hovered(
                                    cursor,
                                    rect[0],
                                    rect[1],
                                    rect[2] + 6.0,
                                    rect[3],
                                ) =>
                            {
                                Some(input)
                            }
                            _ => None,
                        },
                        _ => None,
                    });
                if let Some(Some(Field::Multi(edit))) = text
                    .filter(|_| !step_scroll)
                    .and_then(|i| open.fields.get_mut(i))
                {
                    edit.wheel(delta);
                    return true;
                }
            }
            let [ax, _, aw, _] = layout.area;
            if stick_scroll || step_scroll || ui::hovered(cursor, ax, ay, aw, ah) {
                open.scroll.wheel(delta, 10.0, ay, ay + ah, layout.content);
                return true;
            }
        }
        false
    }

    /// The pointer moved with a text field or slider held.
    pub(in crate::game::client_side) fn dialog_drag(&mut self) -> bool {
        if self.client.dialogs.0.warning || self.client.dialogs.0.waiting.is_some() {
            return false;
        }
        let (cursor, shift) = (self.client.cursor, self.client.modifiers.shift_key());
        if let Some(open) = self.client.dialogs.0.open.as_mut()
            && let Some(Some(field)) = open.selecting.and_then(|i| open.fields.get_mut(i))
        {
            match field {
                Field::Line(edit) => edit.drag_to(&self.client.gui.atlas, cursor.0),
                Field::Multi(edit) => edit.drag(&self.client.gui.atlas.font, cursor, shift),
            }
            return true;
        }
        let Some(input) = self.client.dialogs.0.open.as_ref().and_then(|o| o.dragging) else {
            return false;
        };
        let Some(layout) = self.dialog_layout() else {
            return false;
        };
        let x = self.client.cursor.0;
        let Some(open) = self.client.dialogs.0.open.as_mut() else {
            return false;
        };
        let slider = layout.body.iter().find_map(|p| match p {
            Part::Slider { input: i, rect } if *i == input => Some(*rect),
            _ => None,
        });
        if let (Some([sx, _, sw, _]), Some(value)) = (slider, open.values.get_mut(input)) {
            *value = InputValue::Slider(ui::slider_value(sx, sw, x) as f32);
            return true;
        }
        false
    }

    /// The left button released: a held slider lets go and clicks.
    pub(in crate::game::client_side) fn dialog_release(&mut self) -> bool {
        if self.client.dialogs.0.warning || self.client.dialogs.0.waiting.is_some() {
            return false;
        }
        if let Some(open) = self.client.dialogs.0.open.as_mut() {
            if let Some(selecting) = open.selecting.take() {
                if let Some(Some(Field::Multi(edit))) = open.fields.get_mut(selecting) {
                    edit.release();
                }
                return true;
            }
            if open.dragging.take().is_some() {
                self.play_audio_button_click();
                return true;
            }
        }
        false
    }

    /// A key pressed on a dialog screen: Escape closes what allows it, the
    /// rest goes to the focused input before the screen's shared navigation
    /// (`shortcut`: the press as an edit shortcut).
    pub fn dialog_key(&mut self, key: KeyCode, shortcut: Option<EditShortcut>) -> Result<bool> {
        let escape = key == KeyCode::Escape;
        if self.client.dialogs.0.warning {
            if escape {
                self.return_from_dialog_warning();
            }
            return Ok(escape);
        }
        if let Some(since) = self.client.dialogs.0.waiting {
            // shouldCloseOnEsc: once Back works.
            if escape && waiting_button((since.elapsed().as_millis() / 50) as u32).2 {
                self.client.dialogs.0.waiting = None;
                self.dialog_receiver_changed();
                return Ok(true);
            }
            return Ok(false);
        }
        let ctrl = self.client.modifiers.control_key() || self.client.modifiers.super_key();
        let shift = self.client.modifiers.shift_key();
        let Some(open) = self.client.dialogs.0.open.as_mut() else {
            return Ok(false);
        };
        if escape {
            if !open.dialog.can_close_with_escape {
                return Ok(false);
            }
            // onClose: the cancel action, and the screen closes.
            Self::dialog_values(open);
            let d = &open.dialog;
            let click = d.on_cancel().and_then(|a| a.click(&d.inputs, &open.values));
            self.run_dialog_action(click, AfterAction::Close)?;
            return Ok(true);
        }
        if let Some(focus) = open.focus
            && let Some(input) = open.dialog.inputs.get(focus)
            && let Control::NumberRange {
                start,
                end,
                step,
                width,
                ..
            } = &input.control
            && let Some(InputValue::Slider(val)) = open.values.get_mut(focus)
            && matches!(key, KeyCode::ArrowLeft | KeyCode::ArrowRight)
        {
            let delta = if let Some(step) = step {
                let range = (end - start).abs();
                if range > 0.0 { *step / range } else { 0.01 }
            } else {
                1.0 / (*width as f32 - 8.0).max(1.0)
            };
            let step_sign = if key == KeyCode::ArrowRight {
                delta
            } else {
                -delta
            };
            *val = (*val + step_sign).clamp(0.0, 1.0);
            return Ok(true);
        }
        let Some(Some(edit)) = open.focus.and_then(|f| open.fields.get_mut(f)) else {
            return Ok(false);
        };
        let font = &self.client.gui.atlas.font;
        if let Some(shortcut) = shortcut {
            edit.shortcut(font, shortcut, &mut self.client.clipboard.0);
            return Ok(true);
        }
        Ok(edit.key(font, key, ctrl, shift))
    }

    /// Whether the open dialog has a text input.
    pub(in crate::game::client_side) fn dialog_has_text_field(&self) -> bool {
        self.client
            .dialogs
            .0
            .open
            .as_ref()
            .is_some_and(|open| open.fields.iter().any(Option::is_some))
    }

    /// `type TEXT` with a dialog open: into the focused text input, or the
    /// first one.
    pub(in crate::game::client_side) fn dialog_type(&mut self, text: &str) -> bool {
        let Some(open) = self.client.dialogs.0.open.as_mut() else {
            return false;
        };
        let focus = open
            .focus
            .or_else(|| open.fields.iter().position(Option::is_some));
        let Some(Some(edit)) = focus.and_then(|f| open.fields.get_mut(f)) else {
            return false;
        };
        open.focus = focus;
        edit.insert_text(&self.client.gui.atlas.font, text);
        true
    }

    /// What `state` reports: the open dialog and its values, or the waiting
    /// and warning screens.
    pub fn dialog_status(&self) -> serde_json::Value {
        let d = &self.client.dialogs.0;
        let screen = if d.warning {
            Some("warning")
        } else if d.waiting.is_some() {
            Some("waiting_for_response")
        } else {
            None
        };
        let open = d.open.as_ref().map(|open| {
            let mut values = open.values.clone();
            for (value, field) in values.iter_mut().zip(&open.fields) {
                if let Some(f) = field {
                    *value = InputValue::Text(f.value());
                }
            }
            open.dialog.json(&values)
        });
        serde_json::json!({ "open": open, "screen": screen })
    }
}

fn draw_background(batch: &mut HudBatch, atlas: &sunrise_client::hud::HudAtlas, in_world: bool) {
    let (w, h) = (batch.width, batch.height);
    // Screen.extractBackground draws the panorama and blur before this menu overlay.
    // The frame renderer already supplies them while a server is configuring.
    ui::menu_background(batch, atlas, 0.0, 0.0, w, h, in_world);
}

#[cfg(feature = "client")]
#[cfg(test)]
mod background_tests {
    use super::*;
    use sunrise_client::hud::{HudAtlas, Rect};

    #[test]
    fn a_configuration_dialog_keeps_the_panorama_visible() {
        let mut atlas =
            HudAtlas::build(&sunrise_assets::resource_source::ResourceSource::directory(
                std::path::PathBuf::from("/nonexistent-sunrise-dialog-background-test"),
            ))
            .unwrap();
        atlas.sprites.remove("gui/menu_background");
        let mut batch = HudBatch::scratch([640, 480]);
        draw_background(&mut batch, &atlas, false);
        assert_eq!(
            batch.vertices.len(),
            6,
            "only the menu overlay covers the panorama"
        );
        assert!(batch.vertices.iter().all(|v| v.color >> 24 == 160));
    }

    #[test]
    fn dialogs_keep_the_menu_and_in_world_pack_backgrounds() {
        let mut atlas =
            HudAtlas::build(&sunrise_assets::resource_source::ResourceSource::directory(
                std::path::PathBuf::from("/nonexistent-sunrise-dialog-background-test"),
            ))
            .unwrap();
        for (name, u) in [
            ("gui/menu_background", 0.1),
            ("gui/inworld_menu_background", 0.6),
        ] {
            atlas.sprites.insert(
                name.into(),
                Rect {
                    u0: u,
                    v0: 0.2,
                    u1: u + 0.1,
                    v1: 0.3,
                    w: 32,
                    h: 32,
                },
            );
        }
        for (in_world, u) in [(false, 0.1), (true, 0.6)] {
            let mut batch = HudBatch::scratch([640, 480]);
            draw_background(&mut batch, &atlas, in_world);
            assert!(!batch.vertices.is_empty());
            assert!(
                batch
                    .vertices
                    .iter()
                    .all(|v| v.uv[0] >= u && v.uv[0] <= u + 0.1)
            );
            assert!(batch.vertices.iter().all(|v| v.color == u32::MAX));
        }
    }
}
#[cfg(feature = "client")]
#[cfg(test)]
mod pause_tests {
    use super::*;

    fn dialog(pause: bool) -> Dialog {
        let s = |value: &str| Tag::String(value.into());
        Dialog::parse(
            &Tag::compound([
                ("type", s("notice")),
                ("title", s("Hi")),
                ("pause", Tag::Byte(i8::from(pause))),
            ]),
            &Default::default(),
            &|_| false,
            &|_| false,
        )
        .unwrap()
    }

    fn state(pause: bool) -> DialogState {
        DialogState {
            open: Some(Open {
                dialog: dialog(pause),
                configuration: None,
                values: Vec::new(),
                fields: Vec::new(),
                focus: None,
                scroll: ui::Scroll::default(),
                dragging: None,
                selecting: None,
            }),
            revision: 1,
            waiting: None,
            warning: false,
        }
    }

    /// `DialogScreen.isPauseScreen` is `dialog.common().pause()`, true by
    /// default, so a server's dialog stops a single-player world unless it
    /// asks otherwise. The waiting screen and the warning override it.
    #[test]
    fn a_dialog_pauses_the_world_only_while_its_pause_says_so() {
        assert!(state(true).pauses_world());
        assert!(!state(false).pauses_world());
        // No dialog screen at all.
        assert!(!DialogState::default().pauses_world());
        // DialogScreen.WarningScreen extends ConfirmScreen: true.
        let mut warning = state(false);
        warning.warning = true;
        assert!(warning.pauses_world());
        // WaitingForResponseScreen.isPauseScreen is false.
        let mut waiting = state(true);
        waiting.waiting = Some(Instant::now());
        assert!(!waiting.pauses_world());
    }
}
