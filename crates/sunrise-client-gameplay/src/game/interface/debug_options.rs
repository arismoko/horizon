//! `DebugOptionsScreen` (F3+F6): every debug entry with Off / In Overlay /
//! Always buttons, a search box over them, the two built-in profiles and the
//! debug overlay's GUI scale. The statuses live in `debug-profile.json`
//! (`sunrise_client::debug_entries`).
//! https://mcsrc.dev/2/26.3/net/minecraft/client/gui/screens/debug/DebugOptionsScreen
use super::Screen;
use sunrise_client::{
    debug_entries::{Category, Entry, Profile, RegisteredDebugEntry, Status, search},
    hud::HudBatch,
    options_file::V,
    ui::{self, EditBox, Scroll},
};
use sunrise_game_data::text::Text;
use winit::keyboard::KeyCode;

/// `HeaderAndFooterLayout(this, 61, 33)`.
const HEADER: f32 = 61.0;
const FOOTER: f32 = 33.0;
/// `OptionList.getRowWidth` and `ITEM_HEIGHT`.
const ROW_WIDTH: f32 = 350.0;
const ITEM: f32 = 20.0;
/// `OptionEntry.BUTTON_WIDTH`, at the CycleButtons' 16 pixels high.
const BUTTON: f32 = 60.0;
/// The footer: two profile buttons, the GUI scale and Done, 4 pixels apart.
const FOOTER_BUTTONS: [f32; 4] = [120.0, 120.0, 110.0, 60.0];
/// `debug.options.warning`'s colour (-2142128) and the buttons' text colours.
const WARNING: u32 = 0xDF_5050;
const UNSELECTED: u32 = 0xBA_BABA;
const OVERLAY: u32 = 0xFF_FF55;
const NOT_ALLOWED: u32 = 0x80_8080;
/// `EditBox.SEARCH_HINT_STYLE`: `ChatFormatting.GRAY`.
const HINT: u32 = 0xAA_AAAA;

/// The open screen: its search box and the list's scroll.
pub struct DebugOptionsUi {
    pub search: EditBox,
    scroll: Scroll,
}

impl DebugOptionsUi {
    pub(in crate::game::client_side) fn search_focused(&self) -> bool {
        self.search.focused
    }
}

/// One list row (`AbstractOptionEntry`).
pub(in crate::game) enum Row<'a> {
    Category(Category),
    Option(Entry<'a>),
}

/// `OptionList.updateSearch`: a category row before each category's entries.
pub(in crate::game::client_side) fn rows<'a>(
    registered: &'a [RegisteredDebugEntry],
    value: &str,
) -> Vec<Row<'a>> {
    let mut rows = Vec::new();
    let mut current = None;
    for entry in search(registered, value) {
        if current != Some(entry.category) {
            rows.push(Row::Category(entry.category));
            current = Some(entry.category);
        }
        rows.push(Row::Option(entry));
    }
    rows
}

/// `AbstractSelectionList.contentHeight`.
fn content_height(n: usize) -> f32 {
    n as f32 * ITEM + 4.0
}
/// `getRowLeft`: the list spans the screen, its rows centred.
fn row_left(w: f32) -> f32 {
    (w / 2.0).floor() - ROW_WIDTH / 2.0
}
/// The three buttons' left edges (Off, In Overlay, Always) for a row whose
/// content starts at `x` (`extractContent`).
fn button_xs(content_x: f32) -> [f32; 3] {
    let start = content_x + ROW_WIDTH - 4.0 - 3.0 * BUTTON;
    [start, start + BUTTON, start + 2.0 * BUTTON]
}
/// The footer buttons' left edges and their row's top.
pub fn footer(w: f32, h: f32) -> ([f32; 4], f32) {
    let total: f32 = FOOTER_BUTTONS.iter().sum::<f32>() + 3.0 * 4.0;
    let mut x = ((w - total) / 2.0).floor();
    let mut xs = [0.0; 4];
    for (slot, width) in xs.iter_mut().zip(FOOTER_BUTTONS) {
        *slot = x;
        x += width + 4.0;
    }
    (xs, h - FOOTER + ((FOOTER - 20.0) / 2.0).floor())
}
/// The three buttons' statuses and texts, in their order on the row.
const BUTTONS: [(Status, &str, u32); 3] = [
    (Status::Never, "options.off", 0xFF_FFFF),
    (Status::InOverlay, "debug.entry.overlay", OVERLAY),
    (Status::AlwaysOn, "debug.entry.always", WARNING),
];

impl crate::game::ClientGame<'_> {
    /// `KeyboardHandler`'s F3+F6: closes the screen if it is open, else opens
    /// it over whatever screen yields (`canInterruptScreen`), closing that
    /// screen first.
    pub(in crate::game::client_side) fn toggle_debug_options(&mut self) {
        if self.client.screen == Screen::DebugOptions {
            self.close_debug_options();
            return;
        }
        if !debug_options_interrupts(self.client.screen, self.screen_over_playing()) {
            return;
        }
        self.close_screen_behind_debug_options();
        let mut search = EditBox::new(0.0, 0.0, (ROW_WIDTH / 3.0).floor(), 20.0);
        search.max_length = 32;
        self.client.debug_options = Some(DebugOptionsUi {
            search,
            scroll: Scroll::default(),
        });
        self.set_screen(Screen::DebugOptions);
        self.client.pause_menu = true;
        self.capture(false);
        self.set_client_mining(None);
    }
    /// The interrupted screen's own close (`Screen.onClose`): the containers
    /// hand their state back and a mod's hooks hear of the screen going. A
    /// screen whose close only swaps the screen (stats, links, the pause menu)
    /// yields by being overwritten instead.
    fn close_screen_behind_debug_options(&mut self) {
        match self.client.screen {
            Screen::Inventory => {
                self.close_container_client();
                self.client.pause_focus = None;
                self.set_screen(Screen::Playing);
                self.client.pause_menu = true;
                self.capture(true);
            }
            Screen::Creative => {
                self.pad_clear_focus();
                self.common.carried = None;
                self.set_screen_with(Screen::Playing, |g| g.close_creative_menu());
                self.capture(true);
            }
            Screen::Mod => self.close_mod_screen(),
            _ => {}
        }
    }
    /// `Screen.onClose`: back to the title when no level is open, otherwise the game.
    pub(in crate::game::client_side) fn close_debug_options(&mut self) {
        self.client.debug_options = None;
        self.set_screen(Screen::Playing);
        self.capture(self.client.title.0.is_none());
    }
    /// The header's title row and warning (`init`'s vertical layout,
    /// centred in the header): the title's x, the row's top, the warning's
    /// lines with their y.
    fn debug_options_header(&self, w: f32) -> (f32, f32, Vec<(Text, f32)>) {
        let font = &self.client.gui.atlas.font;
        let title = Text::translate("debug.options.title", &[]);
        let warning = Text::translate("debug.options.warning", &[]).with_color(WARNING);
        let lines = font.split(&warning, ROW_WIDTH);
        let height = 20.0 + 8.0 + lines.len() as f32 * 9.0;
        let top = ((HEADER - height) / 2.0).floor();
        let third = (ROW_WIDTH / 3.0).floor();
        let tw = font.width_text(&title);
        let row = third + 8.0 + tw + 8.0 + third;
        let title_x = ((w - row) / 2.0).floor() + third + 8.0;
        let lines = lines
            .into_iter()
            .enumerate()
            .map(|(i, line)| (line, top + 28.0 + i as f32 * 9.0))
            .collect();
        (title_x, top, lines)
    }
    /// `debugGuiScale`'s button: `GUI Scale: Unchanged`, `Auto` or the scale.
    fn debug_gui_scale_label(&self) -> Text {
        let value = match self.client.settings.file.int("debugGuiScale") {
            -1 => Text::translate("options.debugGuiScale.unchanged", &[]),
            0 => Text::translate("options.guiScale.auto", &[]),
            n => Text::literal(n.to_string()),
        };
        Text::translate(
            "options.generic_value",
            &[Text::translate("options.guiScale", &[]), value],
        )
    }
    /// `ClampingLazyMaxIntRange(-1, window.calculateScale(0, unicode))` as a
    /// CycleButton: the next value, or the previous one with Shift.
    fn cycle_debug_gui_scale(&mut self, back: bool) {
        let size = self.client.renderer.0.as_ref().map_or([1280, 720], |r| {
            [r.config.width as i32, r.config.height as i32]
        });
        let max =
            sunrise_client::window::calculate_scale(size, 0, self.client.settings.force_unicode);
        let current = self
            .client
            .settings
            .file
            .int("debugGuiScale")
            .clamp(-1, max);
        let next = match (back, current) {
            (false, c) if c >= max => -1,
            (false, c) => c + 1,
            (true, -1) => max,
            (true, c) => c - 1,
        };
        self.client.settings.file.set("debugGuiScale", V::Int(next));
        self.client
            .settings
            .save(&sunrise_gameplay_common::game::options_path());
    }
    pub(in crate::game::client_side) fn draw_debug_options(&mut self, batch: &mut HudBatch) {
        let Some(s) = self.client.debug_options.as_ref() else {
            self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
            self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
            return;
        };
        let (w, h) = (batch.width, batch.height);
        // extractBlurredBackground: the debug overlay goes under the screen.
        let size = self
            .client
            .renderer
            .0
            .as_ref()
            .map_or([w as u32, h as u32], |r| [r.config.width, r.config.height]);
        self.draw_debug_overlay_under(batch, size);
        let atlas = &self.client.gui.atlas;
        let in_world = self.common.player.is_some();
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, in_world);
        let reduced = self.reduced_debug_info();
        let rows = rows(&self.client.registered_debug_entries.0, &s.search.value);
        let (top, bottom) = (HEADER, h - FOOTER);
        let content = content_height(rows.len());
        self.client
            .debug_options
            .as_mut()
            .unwrap()
            .scroll
            .clamp(top, bottom, content);
        // The list's background and separators, then its clipped rows and scrollbar.
        ui::list_background(batch, atlas, top, bottom, in_world);
        ui::separators(batch, atlas, top, bottom, in_world);
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        let (title_x, header_top, lines) = self.debug_options_header(w);
        let gui_scale_label = self.debug_gui_scale_label();
        let s = self.client.debug_options.as_mut().unwrap();
        let first = batch.vertices.len();
        let left = row_left(w);
        let inside = self.client.cursor.1 >= top && self.client.cursor.1 < bottom;
        let mut tooltip = None;
        for (i, row) in rows.iter().enumerate() {
            let y = top + 2.0 - s.scroll.amount.trunc() + i as f32 * ITEM;
            if y + ITEM < top || y > bottom {
                continue;
            }
            let (cx, cy) = (left + 2.0, y + 2.0);
            match row {
                Row::Category(category) => {
                    let label = Text::translate(category.translation_key(), &[])
                        .bold()
                        .underlined();
                    let lw = atlas.font.width_text(&label);
                    let lx = (cx + (ROW_WIDTH - 4.0) / 2.0 - (lw + 8.0) / 2.0).floor() + 4.0;
                    batch.text_styled(atlas, &label, lx, cy + 4.0, u32::MAX, true);
                }
                Row::Option(entry) => {
                    let allowed = entry.reduced || !reduced;
                    let name = if allowed {
                        Text::literal(entry.path)
                    } else {
                        Text::literal(entry.path).italic().with_color(NOT_ALLOWED)
                    };
                    batch.text_styled(atlas, &name, cx, cy + 5.0, u32::MAX, true);
                    let xs = button_xs(cx);
                    let status = self.client.debug_entries.0.status(entry.path);
                    for ((status_of, key, color), x) in BUTTONS.into_iter().zip(xs) {
                        let selected = status == status_of;
                        let label = Text::translate(key, &[]).with_color(if selected {
                            color
                        } else {
                            UNSELECTED
                        });
                        let hover = inside && ui::hovered(self.client.cursor, x, cy, BUTTON, 16.0);
                        ui::button(batch, atlas, x, cy, BUTTON, 16.0, &label, !selected, hover);
                    }
                    if !allowed
                        && inside
                        && ui::hovered(self.client.cursor, cx, y, xs[0] - cx, ITEM)
                    {
                        tooltip = Some(Text::translate("debug.options.notAllowed.tooltip", &[]));
                    }
                }
            }
        }
        batch.clip_since(first, 0.0, top, w, bottom);
        ui::scrollbar(
            batch,
            atlas,
            left + ROW_WIDTH + 8.0,
            top,
            bottom,
            content,
            s.scroll.amount.trunc(),
        );
        // The header: title, search box, warning.
        let title = Text::translate("debug.options.title", &[]);
        batch.text_styled(atlas, &title, title_x, header_top + 5.0, u32::MAX, true);
        s.search.x = title_x + atlas.font.width_text(&title) + 8.0;
        s.search.y = header_top;
        s.search.draw(batch, atlas, None);
        if s.search.value.is_empty() {
            let hint = Text::translate("debug.options.search", &[])
                .with_color(HINT)
                .italic();
            batch.text_styled(
                atlas,
                &hint,
                s.search.text_x(),
                s.search.text_y(),
                u32::MAX,
                true,
            );
        }
        for (line, y) in &lines {
            let lw = atlas.font.width_text(line);
            batch.text_styled(atlas, line, ((w - lw) / 2.0).floor(), *y, u32::MAX, true);
        }
        // The footer.
        let (xs, fy) = footer(w, h);
        let labels = [
            Text::translate(Profile::Default.translation_key(), &[]),
            Text::translate(Profile::Performance.translation_key(), &[]),
            gui_scale_label,
            Text::translate("gui.done", &[]),
        ];
        let active = [
            !self
                .client
                .debug_entries
                .0
                .is_using_profile(Profile::Default),
            !self
                .client
                .debug_entries
                .0
                .is_using_profile(Profile::Performance),
            true,
            true,
        ];
        for (i, label) in labels.iter().enumerate() {
            let hover = ui::hovered(self.client.cursor, xs[i], fy, FOOTER_BUTTONS[i], 20.0);
            if i == 3 {
                sunrise_client::widgets::declare_action(
                    sunrise_client::widgets::Action::Done,
                    [xs[i], fy, FOOTER_BUTTONS[i], 20.0],
                    true,
                );
            }
            ui::button(
                batch,
                atlas,
                xs[i],
                fy,
                FOOTER_BUTTONS[i],
                20.0,
                label,
                active[i],
                hover,
            );
            if i == 2 && hover {
                tooltip = Some(Text::translate("options.debugGuiScale.tooltip", &[]));
            }
        }
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
        if let Some(tooltip) = tooltip {
            let lines = atlas.font.split(&tooltip, 200.0);
            ui::tooltip(batch, atlas, &lines, self.client.cursor);
        }
    }
    /// A left click: a status button, the search box, a footer button.
    pub fn debug_options_click(&mut self) -> bool {
        let Some(mut s) = self.client.debug_options.take() else {
            return false;
        };
        let (w, h) = self.gui_size();
        let (top, bottom) = (HEADER, h - FOOTER);
        let rows = rows(&self.client.registered_debug_entries.0, &s.search.value);
        let left = row_left(w);
        // A press on the scrollbar holds it and clicks nothing else.
        if s.scroll.press(
            self.client.cursor,
            left + ROW_WIDTH + 8.0,
            top,
            bottom,
            content_height(rows.len()),
        ) {
            self.client.debug_options = Some(s);
            return true;
        }
        let shift = self.has_shift_down();
        s.search.focused = ui::hovered(
            self.client.cursor,
            s.search.x,
            s.search.y,
            s.search.width,
            s.search.height,
        );
        if s.search.focused {
            s.search
                .click(&self.client.gui.atlas, self.client.cursor.0, shift, false);
        }
        let inside = self.client.cursor.1 >= top && self.client.cursor.1 < bottom;
        let mut clicked = None;
        if inside {
            for (i, row) in rows.iter().enumerate() {
                let Row::Option(entry) = row else { continue };
                let cy = top + 2.0 - s.scroll.amount.trunc() + i as f32 * ITEM + 2.0;
                let status = self.client.debug_entries.0.status(entry.path);
                for ((status_of, _, _), x) in BUTTONS.into_iter().zip(button_xs(left + 2.0)) {
                    if status != status_of && ui::hovered(self.client.cursor, x, cy, BUTTON, 16.0) {
                        clicked = Some((entry.path.to_owned(), status_of));
                    }
                }
            }
        }
        let search = s.search.focused;
        self.client.debug_options = Some(s);
        if let Some((path, status)) = clicked {
            // OptionEntry.setValue.
            self.play_audio_button_click();
            self.client.debug_entries.0.set_status(&path, status);
            return true;
        }
        let (xs, fy) = footer(w, h);
        let Some(i) =
            (0..4).find(|&i| ui::hovered(self.client.cursor, xs[i], fy, FOOTER_BUTTONS[i], 20.0))
        else {
            return search;
        };
        match i {
            0 | 1 => {
                let profile = if i == 0 {
                    Profile::Default
                } else {
                    Profile::Performance
                };
                if self.client.debug_entries.0.is_using_profile(profile) {
                    return search;
                }
                self.play_audio_button_click();
                self.client.debug_entries.0.load_profile(profile);
            }
            2 => {
                self.play_audio_button_click();
                self.cycle_debug_gui_scale(shift);
            }
            _ => {
                self.play_audio_button_click();
                self.close_debug_options();
            }
        }
        true
    }
    /// The mouse wheel over the list (`scrollRate`: half a row).
    pub(in crate::game::client_side) fn debug_options_scroll(&mut self, delta: f32) -> bool {
        let (_, h) = self.gui_size();
        let scroll = self.pad_stick_scroll()
            || self.pad_step_scroll()
            || (HEADER..h - FOOTER).contains(&self.client.cursor.1);
        if scroll && let Some(s) = &mut self.client.debug_options {
            let content = content_height(
                rows(&self.client.registered_debug_entries.0, &s.search.value).len(),
            );
            s.scroll
                .wheel(delta, ITEM / 2.0, HEADER, h - FOOTER, content);
            return true;
        }
        false
    }
    /// The pointer moved while the list's scrollbar is held.
    pub(in crate::game::client_side) fn debug_options_drag(&mut self) -> bool {
        let (_, h) = self.gui_size();
        let y = self.client.cursor.1;
        if let Some(s) = &mut self.client.debug_options
            && s.scroll.held()
        {
            let content = content_height(
                rows(&self.client.registered_debug_entries.0, &s.search.value).len(),
            );
            s.scroll.drag(y, HEADER, h - FOOTER, content);
            return true;
        }
        false
    }
    /// The button came up: the scrollbar lets go.
    pub(in crate::game::client_side) fn debug_options_release(&mut self) -> bool {
        self.client.debug_options.as_mut().is_some_and(|s| {
            let handled = s.scroll.held();
            s.scroll.release();
            handled
        })
    }
    /// A key the debug keys left: Escape closes, the rest edits the search.
    pub(in crate::game::client_side) fn debug_options_key(
        &mut self,
        key: KeyCode,
        shortcut: Option<sunrise_client::clipboard::EditShortcut>,
    ) -> bool {
        if key == KeyCode::Escape {
            self.close_debug_options();
            return true;
        }
        let ctrl = self.has_control_down_with_quirk();
        let shift = self.has_shift_down();
        let Some(s) = self
            .client
            .debug_options
            .as_mut()
            .filter(|s| s.search.focused)
        else {
            return false;
        };
        if let Some(shortcut) = shortcut {
            s.search.shortcut(shortcut, &mut self.client.clipboard.0);
            true
        } else {
            s.search.key(key, ctrl, shift)
        }
    }
    /// One character off the search box, for the controller's on-screen keyboard.
    pub(in crate::game::client_side) fn debug_options_backspace(&mut self) {
        if let Some(s) = &mut self.client.debug_options
            && s.search.focused
        {
            s.search.key(KeyCode::Backspace, false, false);
        }
    }
    /// Characters typed into the search box (`EditBox.charTyped`).
    pub(in crate::game::client_side) fn debug_options_type(&mut self, typed: &str) {
        let allowed: String = typed
            .chars()
            .filter(|&c| c != '\u{a7}' && c >= ' ' && c != '\u{7f}')
            .collect();
        if let Some(s) = &mut self.client.debug_options
            && s.search.focused
            && !allowed.is_empty()
        {
            s.search.insert_text(&allowed);
        }
    }
}

/// `Minecraft.canInterruptScreen` (Gui): whether F3+F6 opens the debug
/// options screen over `screen`, closing it first. `Screen
/// .canInterruptWithAnotherScreen` defaults to `shouldCloseOnEsc`, so every
/// screen Escape closes yields; the death screen and the level teardown keep
/// theirs; the title explicitly allows interruption. The text-box screens
/// (chat, options, sign, book, the search tabs) do not reach it either: a
/// focused box takes every key press first. A playing overlay (a dialog, a
/// prompt) keeps its own Esc. The credits' close is fallible and this path is
/// not, so it keeps its screen too (vanilla interrupts it).
fn debug_options_interrupts(screen: Screen, playing_overlay: bool) -> bool {
    match screen {
        Screen::Playing | Screen::Paused => !playing_overlay,
        Screen::Inventory
        | Screen::Creative
        | Screen::GameModeSwitcher
        | Screen::Stats
        | Screen::Advancements
        | Screen::Friends
        | Screen::Social
        | Screen::ServerLinks
        | Screen::ConfirmLink
        | Screen::DemoIntro
        | Screen::DialogWarning
        | Screen::Mod => true,
        Screen::Chat
        | Screen::Options
        | Screen::WorldOptions
        | Screen::CommandBlock
        | Screen::BlockEditor
        | Screen::Dead
        | Screen::Win
        | Screen::DebugOptions
        | Screen::HudEditor => false,
    }
}

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;

    #[test]
    fn rows_put_a_category_before_each_section() {
        let all = rows(&[], "");
        assert_eq!(all.len(), 48);
        assert!(matches!(all[0], Row::Category(Category::Text)));
        let renderer = all
            .iter()
            .position(|r| matches!(r, Row::Category(Category::Renderer)))
            .unwrap();
        assert!(matches!(all[renderer + 1], Row::Option(e) if e.path == "3d_crosshair"));
        // A search inside one section shows only its heading.
        let hitboxes = rows(&[], "hitbox");
        assert_eq!(hitboxes.len(), 2);
        assert!(matches!(hitboxes[0], Row::Category(Category::Renderer)));
    }

    #[test]
    fn the_layout_matches_vanilla_at_854_by_480() {
        // 854x480 at GUI scale 2 is 427x240.
        assert_eq!(row_left(427.0), 38.0);
        assert_eq!(button_xs(40.0), [206.0, 266.0, 326.0]);
        assert_eq!(footer(427.0, 240.0), ([2.0, 126.0, 250.0, 364.0], 213.0));
    }

    /// F3+F6 interrupts every screen whose Esc would close it
    /// (`Minecraft.canInterruptScreen`), and none whose would not.
    #[test]
    fn the_debug_options_screen_interrupts_what_escape_would_close() {
        assert!(debug_options_interrupts(Screen::Playing, false));
        assert!(debug_options_interrupts(Screen::Paused, false));
        assert!(!debug_options_interrupts(Screen::Playing, true));
        assert!(!debug_options_interrupts(Screen::Paused, true));
        for screen in [
            Screen::Inventory,
            Screen::Creative,
            Screen::GameModeSwitcher,
            Screen::Stats,
            Screen::Advancements,
            Screen::Friends,
            Screen::Social,
            Screen::ServerLinks,
            Screen::ConfirmLink,
            Screen::DialogWarning,
            Screen::Mod,
        ] {
            assert!(
                debug_options_interrupts(screen, false),
                "{} yields",
                screen.id()
            );
        }
        for screen in [
            Screen::Dead,
            Screen::Chat,
            Screen::Options,
            Screen::WorldOptions,
            Screen::CommandBlock,
            Screen::BlockEditor,
            Screen::Win,
        ] {
            assert!(
                !debug_options_interrupts(screen, false),
                "{} keeps its screen",
                screen.id()
            );
        }
    }
}
