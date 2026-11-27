//! The sign editing screen (`SignEditScreen` / `HangingSignEditScreen`):
//! opened when a sign is placed, four lines limited to the sign's line
//! width, Up/Down/Enter move between lines, Escape or Done closes.
use crate::game::ClientGame;
use sunrise_client::{
    hud::{HudBatch, rgba},
    ui::EditBox,
};
use sunrise_core::position::BlockPos;
use sunrise_level::containers::BlockEntity;
use sunrise_level::signs_data::editor_sprite;
use sunrise_protocol::packets::play::{self, Serverbound as S};
use sunrise_sim::lan::{
    guest::{native_block_interaction_range, within_block_range},
    playerdata::ints_uuid,
};
use winit::keyboard::KeyCode;

pub struct SignEditor {
    pub pos: (i32, i32, i32),
    pub front: bool,
    pub line: usize,
    /// The line being edited (`AbstractSignEditScreen`'s text field).
    pub(in crate::game) edit: EditBox,
    /// `AbstractSignEditScreen.messages`: the side's lines as edited, sent
    /// when the editor closes, even if the sign is gone by then.
    pub(in crate::game) lines: [String; 4],
}

pub(in crate::game::client_side) fn sign_box() -> EditBox {
    let mut edit = EditBox::new(0.0, 0.0, 90.0, 9.0);
    edit.bordered = false;
    edit.max_length = play::SignLines::MAX_UTF16;
    edit.focused = true;
    edit
}

/// The Done button's text, `CommonComponents.GUI_DONE`, in the selected language.
pub(in crate::game::client_side) fn done_label() -> String {
    super::screen_info::tr("gui.done", &[])
}
/// AbstractSignEditScreen.init's Done button: `width / 2 - 100`,
/// `height / 4 + 144`, int GUI pixels.
pub(in crate::game::client_side) fn done_button(w: f32, h: f32) -> (f32, f32) {
    (
        (w.ceil() / 2.0).floor() - 100.0,
        (h.ceil() / 4.0).floor() + 144.0,
    )
}

impl ClientGame<'_> {
    /// `SignBlock.openTextEdit` for the front side after placement.
    pub fn open_sign_editor(&mut self, pos: (i32, i32, i32), front: bool) {
        self.open_screen_overlay("sign_editor", |game| {
            // Minecraft.setScreen: the editor already open is removed first.
            game.close_sign_editor();
            let lines = match game.world.block_entities.get(&pos) {
                Some(BlockEntity::Sign(sign)) if front => sign.front.lines.clone(),
                Some(BlockEntity::Sign(sign)) => sign.back.lines.clone(),
                _ => Default::default(),
            };
            game.client.sign_edit = Some(SignEditor {
                pos,
                front,
                line: 0,
                edit: sign_box(),
                lines,
            });
            game.sign_load();
            game.clear_device_keys();
            game.set_mapping_held("key.use", false);
            game.capture(false);
        });
    }
    /// `AbstractSignEditScreen.tick`: the editor closes (and `removed` sends
    /// the text) once its sign is removed or the player's eyes are further
    /// than 4 blocks beyond the block interaction range
    /// (`SignBlockEntity.playerIsTooFarAwayToEdit`).
    pub(in crate::game::client_side) fn tick_sign_editor(&mut self) {
        let Some(edit) = &self.client.sign_edit else {
            return;
        };
        let (x, y, z) = edit.pos;
        let buffer = 4.0;
        let mut near = self
            .common
            .eye_position()
            .is_some_and(|eye| within_block_range(eye, self.common.reach() + buffer, [x, y, z]));
        if let Some(uuid) = self.common.level.player_uuid {
            native_block_interaction_range(
                &self.common.loop_context,
                ints_uuid(uuid),
                BlockPos::new(x, y, z),
                buffer,
                &mut near,
            );
        }
        let sign = matches!(
            self.world.block_entities.get(&edit.pos),
            Some(BlockEntity::Sign(_))
        );
        if !sign || !near {
            self.close_sign_editor();
        }
    }
    pub fn close_sign_editor(&mut self) {
        if self.remove_sign_editor() {
            self.refresh_screen_widgets();
        }
        self.capture(true);
    }

    /// `AbstractSignEditScreen.removed`: send what the client wrote, or
    /// keep it in the window's own world and send its guests the block update.
    pub fn remove_sign_editor(&mut self) -> bool {
        let Some(edit) = self.client.sign_edit.take() else {
            return false;
        };
        if self.client.remote.is_some() {
            self.send(S::SignUpdate(play::SignUpdate {
                pos: [edit.pos.0, edit.pos.1, edit.pos.2],
                lines: edit.lines,
                front: edit.front,
            }));
        }
        // `SignBlockEntity.updateSignText`'s `setAllowedPlayerEditor(null)`:
        // the editor that closes gives the sign up.
        self.common
            .world
            .block_entities
            .set_sign_editor(edit.pos, None);
        true
    }
    /// `ClientPacketListener.handleOpenSignEditor`: the editor on a sign this
    /// client has.
    pub fn remote_open_sign_editor(&mut self, p: play::OpenSignEditor) {
        let key = (p.pos[0], p.pos[1], p.pos[2]);
        if matches!(
            self.world.block_entities.get(&key),
            Some(BlockEntity::Sign(_))
        ) {
            self.open_sign_editor(key, p.front);
        } else {
            log::warn!(
                "ignoring openTextEdit on a block that is no sign at {:?}",
                p.pos
            );
        }
    }
    pub(in crate::game::client_side) fn sign_edit_key(
        &mut self,
        key: KeyCode,
        shortcut: Option<sunrise_client::clipboard::EditShortcut>,
    ) -> bool {
        if let Some(shortcut) = shortcut {
            self.sign_shortcut(shortcut);
            true
        } else if matches!(
            key,
            KeyCode::Escape
                | KeyCode::Enter
                | KeyCode::NumpadEnter
                | KeyCode::ArrowUp
                | KeyCode::ArrowDown
                | KeyCode::ArrowLeft
                | KeyCode::ArrowRight
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::Backspace
                | KeyCode::Delete
        ) {
            self.sign_input(Some(key), None);
            true
        } else {
            false
        }
    }
    /// A key and the text it typed on the sign being edited (the keyboard,
    /// or a controller's on-screen keyboard).
    pub fn sign_input(&mut self, key: Option<KeyCode>, typed: Option<&str>) {
        if self.client.sign_edit.is_none() {
            return;
        }
        match key {
            Some(KeyCode::Escape) => self.close_sign_editor(),
            Some(KeyCode::ArrowUp) => self.sign_move_line(3),
            Some(KeyCode::ArrowDown | KeyCode::Enter | KeyCode::NumpadEnter) => {
                self.sign_move_line(1)
            }
            Some(
                key @ (KeyCode::ArrowLeft
                | KeyCode::ArrowRight
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::Backspace
                | KeyCode::Delete),
            ) => {
                let (ctrl, shift) = (self.has_control_down(), self.has_shift_down());
                self.sign_change(|edit| {
                    edit.key(key, ctrl, shift);
                });
            }
            _ => {
                let Some(typed) = typed else {
                    return;
                };
                self.sign_change(|edit| edit.insert_text(typed));
            }
        }
    }
    /// Up or down a line: the box writes the line it leaves and loads the next.
    pub(in crate::game::client_side) fn sign_move_line(&mut self, by: usize) {
        self.sign_write();
        if let Some(edit) = self.client.sign_edit.as_mut() {
            edit.line = (edit.line + by) & 3;
        }
        self.sign_load();
    }
    /// The edited line's text.
    pub(in crate::game::client_side) fn sign_current(&self) -> Option<String> {
        let edit = self.client.sign_edit.as_ref()?;
        let BlockEntity::Sign(sign) = self.common.world.block_entities.get(&edit.pos)? else {
            return None;
        };
        let side = if edit.front { &sign.front } else { &sign.back };
        Some(side.lines[edit.line].clone())
    }
    /// `EditBox.setValue` from the line now being edited.
    pub(in crate::game::client_side) fn sign_load(&mut self) {
        let Some(text) = self.sign_current() else {
            return;
        };
        if let Some(edit) = self.client.sign_edit.as_mut() {
            edit.edit.set_value(&text);
        }
    }
    /// The box's text onto the editor's lines and the sign
    /// (`AbstractSignEditScreen.setMessage`), when it still fits the line.
    pub(in crate::game::client_side) fn sign_write(&mut self) {
        let Some(edit) = self.client.sign_edit.as_ref() else {
            return;
        };
        let (pos, front, line, value) = (edit.pos, edit.front, edit.line, edit.edit.value.clone());
        if !self.sign_fits(&value) {
            return;
        }
        if let Some(edit) = self.client.sign_edit.as_mut() {
            edit.lines[line].clone_from(&value);
        }
        let Some(BlockEntity::Sign(sign)) = self.common.world.block_entities.get_mut(&pos) else {
            return;
        };
        let side = if front {
            &mut sign.front
        } else {
            &mut sign.back
        };
        side.lines[line] = value;
    }
    /// TextFieldHelper: `font.width(s) <= maxTextLineWidth`.
    pub(in crate::game::client_side) fn sign_fits(&self, value: &str) -> bool {
        let Some(edit) = self.client.sign_edit.as_ref() else {
            return false;
        };
        let Some(BlockEntity::Sign(sign)) = self.common.world.block_entities.get(&edit.pos) else {
            return false;
        };
        HudBatch::text_width(&self.client.gui.atlas, value) <= sign.max_line_width()
    }
    /// Edit the line, then keep the result only when it still fits.
    pub(in crate::game::client_side) fn sign_change(&mut self, f: impl FnOnce(&mut EditBox)) {
        let before = self
            .client
            .sign_edit
            .as_ref()
            .map(|e| (e.edit.value.clone(), e.edit.cursor, e.edit.highlight));
        if let Some(edit) = self.client.sign_edit.as_mut() {
            f(&mut edit.edit);
            let clean = sunrise_game_data::text::strip_formatting(&edit.edit.value);
            if clean != edit.edit.value {
                let n = sunrise_client::ui::char_len(&clean);
                edit.edit.value = clean;
                edit.edit.cursor = edit.edit.cursor.min(n);
                edit.edit.highlight = edit.edit.highlight.min(n);
            }
        }
        self.sign_keep(before);
    }
    /// Write the box when the line still fits; otherwise put the old text back.
    pub(in crate::game::client_side) fn sign_keep(
        &mut self,
        before: Option<(String, usize, usize)>,
    ) {
        let value = self
            .client
            .sign_edit
            .as_ref()
            .map(|e| e.edit.value.clone())
            .unwrap_or_default();
        if self.sign_fits(&value) {
            self.sign_write();
        } else if let (Some(edit), Some((value, cursor, highlight))) =
            (self.client.sign_edit.as_mut(), before)
        {
            edit.edit.value = value;
            edit.edit.cursor = cursor;
            edit.edit.highlight = highlight;
        }
    }
    /// `TextFieldHelper.keyPressed`'s shortcuts on the edited line. Paste
    /// reads `getClipboardContents` before `insertText`. A paste that would
    /// pass the sign's width is refused whole.
    pub(in crate::game::client_side) fn sign_shortcut(
        &mut self,
        shortcut: sunrise_client::clipboard::EditShortcut,
    ) {
        if shortcut == sunrise_client::clipboard::EditShortcut::Paste {
            let raw = sunrise_client::clipboard::Clipboard::get(&mut self.client.clipboard.0);
            let text = sunrise_client::clipboard::helper_clipboard_contents(&raw);
            self.sign_change(|edit| edit.insert_text(&text));
            return;
        }
        let before = self
            .client
            .sign_edit
            .as_ref()
            .map(|e| (e.edit.value.clone(), e.edit.cursor, e.edit.highlight));
        if let Some(edit) = self.client.sign_edit.as_mut() {
            edit.edit.shortcut(shortcut, &mut self.client.clipboard.0);
            let clean = sunrise_game_data::text::strip_formatting(&edit.edit.value);
            if clean != edit.edit.value {
                let n = sunrise_client::ui::char_len(&clean);
                edit.edit.value = clean;
                edit.edit.cursor = edit.edit.cursor.min(n);
                edit.edit.highlight = edit.edit.highlight.min(n);
            }
        }
        self.sign_keep(before);
    }
    /// Done button (`this.width / 2 - 100, this.height / 4 + 144`, 200x20),
    /// or a click on a line (`EditBox.onClick`).
    pub(in crate::game::client_side) fn sign_edit_click(&mut self, size: [u32; 2]) -> bool {
        let b = HudBatch::new(size, self.client.settings.gui_scale_inputs());
        let (x, y) = done_button(b.width, b.height);
        let (mx, my) = self.client.cursor;
        if mx >= x && mx < x + 200.0 && my >= y && my < y + 20.0 {
            self.play_audio_button_click();
            self.close_sign_editor();
            return true;
        }
        let Some((cx, top, line_height, max_width, lines)) = self.sign_lines(b.width) else {
            return false;
        };
        let shift = self.has_shift_down();
        for i in 0..lines.len() {
            let ly = top + i as f32 * line_height;
            let left = cx - max_width / 2.0;
            if mx < left || mx >= left + max_width || my < ly || my >= ly + line_height {
                continue;
            }
            let current = self.client.sign_edit.as_ref().map(|e| e.line);
            if current != Some(i) {
                self.sign_move_line((i + 4 - current.unwrap_or(0)) & 3);
            }
            let value = self
                .client
                .sign_edit
                .as_ref()
                .map(|e| e.edit.value.clone())
                .unwrap_or_default();
            let lw = HudBatch::text_width(&self.client.gui.atlas, &value);
            let text_x = cx - (lw as i32 / 2) as f32;
            if let Some(edit) = self.client.sign_edit.as_mut() {
                edit.edit.x = text_x;
                edit.edit.y = ly;
                edit.edit.width = max_width;
                edit.edit.click(&self.client.gui.atlas, mx, shift, false);
            }
            self.sign_write();
            return true;
        }
        false
    }
    /// Centre x, the first line's y, the line height, the width limit and the four lines.
    pub(in crate::game::client_side) fn sign_lines(
        &self,
        width: f32,
    ) -> Option<(f32, f32, f32, f32, Vec<String>)> {
        let edit = self.client.sign_edit.as_ref()?;
        let BlockEntity::Sign(sign) = self.common.world.block_entities.get(&edit.pos)? else {
            return None;
        };
        let text = if edit.front { &sign.front } else { &sign.back };
        let line_height = sign.line_height();
        let y = if sign.hanging { 125.0 } else { 90.0 };
        let top = y - (4.0 * line_height / 2.0).floor();
        Some((
            width / 2.0,
            top,
            line_height,
            sign.max_line_width(),
            text.lines.to_vec(),
        ))
    }
    pub(in crate::game::client_side) fn draw_sign_editor(&self, batch: &mut HudBatch) {
        let Some(edit) = &self.client.sign_edit else {
            return;
        };
        let Some(BlockEntity::Sign(sign)) = self.common.world.block_entities.get(&edit.pos) else {
            return;
        };
        self.render_transparent_background("sign_editor", batch);
        let atlas = &self.client.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        let (key, fallback) = if sign.hanging {
            ("hanging_sign.edit", "Edit Hanging Sign Message")
        } else {
            ("sign.edit", "Edit Sign Message")
        };
        let title = self.translate_with_fallback(key, fallback);
        // SignEditScreen / HangingSignEditScreen: their title.
        sunrise_client::widgets::begin_screen(|| {
            sunrise_client::widgets::ScreenInfo::new("sign_edit", title.clone())
        });
        let tw = HudBatch::text_width(atlas, &title);
        batch.text(atlas, &title, (w - tw) / 2.0, 40.0, u32::MAX, true);
        let facts = self.common.res.facts();
        let block = facts.get(self.common.world.block(edit.pos.0, edit.pos.1, edit.pos.2));
        let wood = facts.wood_type(&block.name).unwrap_or("oak");
        let cx = w / 2.0;
        let y = if sign.hanging {
            // HangingSignEditScreen: 16x16 texture scaled 4.5 at (0, -13).
            let s = 4.5;
            batch.sprite_scaled(
                atlas,
                &editor_sprite("hanging_signs", wood),
                cx - 8.0 * s,
                125.0 - 13.0 - 8.0 * s,
                16.0 * s,
                16.0 * s,
                u32::MAX,
            );
            125.0
        } else {
            // SignEditScreen: 24x26 texture scaled 3.9 at (0, 27); wall signs show 12 rows.
            let s = 3.9;
            let wall = block.sign_form().is_some_and(|f| f.wall);
            let rows = if wall { 12.0 } else { 26.0 };
            if let Some(mut r) = atlas.sprites.get(&editor_sprite("signs", wood)).copied() {
                r.v1 = r.v0 + (r.v1 - r.v0) * rows / 26.0;
                batch.rect_uv(
                    cx - 12.0 * s,
                    90.0 + 27.0 - 13.0 * s,
                    24.0 * s,
                    rows * s,
                    r,
                    u32::MAX,
                );
            }
            90.0
        };
        let text = if edit.front { &sign.front } else { &sign.back };
        let color = if text.glowing {
            text.text_color()
        } else {
            text.dark_color()
        };
        let color = rgba((color >> 16) as u8, (color >> 8) as u8, color as u8, 255);
        let line_height = sign.line_height();
        let midpoint = (4.0 * line_height / 2.0).floor();
        for (i, line) in text.lines.iter().enumerate() {
            let shown = if i == edit.line {
                edit.edit.value.as_str()
            } else {
                line.as_str()
            };
            let lw = HudBatch::text_width(atlas, shown);
            let x = cx - (lw as i32 / 2) as f32;
            let ly = y + i as f32 * line_height - midpoint;
            batch.text(atlas, shown, x, ly, color, false);
            if i != edit.line {
                continue;
            }
            let value = &edit.edit.value;
            if edit.edit.cursor != edit.edit.highlight {
                let (a, b) = (
                    edit.edit.cursor.min(edit.edit.highlight),
                    edit.edit.cursor.max(edit.edit.highlight),
                );
                let pre: String = value.chars().take(a).collect();
                let sel: String = value.chars().skip(a).take(b - a).collect();
                batch.text_highlight(
                    atlas,
                    (
                        x + HudBatch::text_width(atlas, &pre),
                        ly,
                        HudBatch::text_width(atlas, &sel),
                        line_height,
                    ),
                    true,
                );
            }
            let pre: String = value.chars().take(edit.edit.cursor).collect();
            let caret_x = x + HudBatch::text_width(atlas, &pre);
            if edit.edit.cursor_visible() {
                batch.fill(atlas, caret_x, ly, 1.0, line_height, color);
            }
            sunrise_client::preedit::report_caret(caret_x, ly, line_height);
        }
        let (bx, by) = done_button(w, h);
        sunrise_client::widgets::declare_action(
            sunrise_client::widgets::Action::Done,
            [bx, by, 200.0, 20.0],
            true,
        );
        sunrise_client::widgets::narratable(|| {
            sunrise_client::widgets::Narratable::button(done_label(), [bx, by, 200.0, 20.0], true)
        });
        batch.sprite_scaled(atlas, "widget/button", bx, by, 200.0, 20.0, u32::MAX);
        let done = done_label();
        let tw = HudBatch::text_width(atlas, &done);
        batch.text(
            atlas,
            &done,
            bx + 100.0 - (tw / 2.0).floor(),
            by + 6.0,
            u32::MAX,
            true,
        );
        self.draw_overlay_mounted_widgets(batch, "sign_editor");
    }
}

#[cfg(test)]
pub(in crate::game) mod tests {
    #[test]
    fn done_button_sits_on_whole_gui_pixels() {
        // `width / 2 - 100, height / 4 + 144` in ints: 334 high is 83 + 144.
        assert_eq!(super::done_button(427.0, 334.0), (113.0, 227.0));
        assert_eq!(super::done_button(320.0, 240.0), (60.0, 204.0));
    }

    /// The Done button said "Done" in every language (#1927).
    #[test]
    fn done_button_reads_the_selected_language() {
        use sunrise_game_data::lang::{Language, ResourceSource, scoped};
        struct Vanilla;
        impl ResourceSource for Vanilla {
            fn read(&self, path: &str) -> Option<Vec<u8>> {
                match path {
                    "assets/minecraft/lang/en_us.json" => Some(br#"{"gui.done": "Done"}"#.to_vec()),
                    "assets/minecraft/lang/de_de.json" => {
                        Some(br#"{"gui.done": "Fertig"}"#.to_vec())
                    }
                    _ => None,
                }
            }
        }
        for (code, want) in [("de_de", "Fertig"), ("en_us", "Done")] {
            let (label, missed) = scoped(Language::load(&[&Vanilla], code), super::done_label);
            assert_eq!(label, want, "{code}");
            assert!(missed.is_empty(), "{code}: {missed:?}");
        }
    }
}
