//! The question before a world opens that was saved with mods that are not loaded now (#3856):
//! opening it removes their blocks, and what those blocks hold, for good. Don't Open is the
//! default (and Escape); Back Up and Open opens only once the backup exists; Open Without a
//! Backup opens at once. The game asks (`Game::open_world`), since a `mod_mismatch` listener may
//! resolve a mod first; this screen only shows what is left and records the choice.
use super::*;
use crate::save::level_mods::{self, MissingMod};

#[cfg(test)]
#[path = "missing_mods_tests.rs"]
mod tests;

/// The same player-facing entry for visible text and narration.
fn entry(m: &MissingMod) -> String {
    if m.saved_version.is_empty() {
        lang::tr("horizon.missing_mods.entry_unknown", &[&m.id])
    } else {
        lang::tr("horizon.missing_mods.entry", &[&m.id, &m.saved_version])
    }
}

/// The prompt's text: what opening does, then each missing mod.
fn message(missing: &[MissingMod]) -> String {
    let mut message = lang::get("horizon.missing_mods.description");
    for m in missing {
        message.push('\n');
        message.push_str(&entry(m));
    }
    message
}

/// What the narrator reads for the prompt.
pub(super) fn screen_info(missing: &[MissingMod]) -> crate::widgets::ScreenInfo {
    crate::widgets::ScreenInfo::new(
        "missing_mods",
        crate::widgets::join_for_narration(&[
            &lang::get("horizon.missing_mods.title"),
            &message(missing),
        ]),
    )
}

/// Shared viewport geometry for rendering, scrolling and action layout.
pub(super) fn layout(w: f32, h: f32) -> (f32, f32, bool) {
    let narrow = w < 324.0;
    let top = 36.0;
    let bottom = top + ((h - if narrow { 88.0 } else { 64.0 } - top) / 9.0).floor() * 9.0;
    (top, bottom, narrow)
}

impl Title {
    /// Page keys scroll the complete warning regardless of which action has focus.
    /// Arrow keys remain available for ordinary action navigation.
    pub fn page_missing_mods(&mut self, key: winit::keyboard::KeyCode, (w, h): (f32, f32)) -> bool {
        use winit::keyboard::KeyCode;
        if !matches!(self.mode, Mode::MissingMods { .. }) {
            return false;
        }
        let (top, bottom, _) = layout(w, h);
        let content = self.message_lines.get() as f32 * 9.0;
        let mut scroll = self.scroll.get();
        let page = (bottom - top).max(9.0);
        scroll.amount = match key {
            KeyCode::PageUp => scroll.amount - page,
            KeyCode::PageDown => scroll.amount + page,
            KeyCode::Home => 0.0,
            KeyCode::End => content,
            _ => return false,
        };
        scroll.clamp(top, bottom, content);
        self.scroll.set(scroll);
        true
    }

    /// Shows the question for the world in `folder`, whose `missing` mods no listener resolved.
    pub fn ask_missing_mods(&mut self, folder: &str, missing: Vec<MissingMod>) {
        let world = list_worlds(&self.saves)
            .into_iter()
            .find(|w| w.folder == folder)
            .unwrap_or_else(|| {
                WorldEntry::corrupted(folder.to_owned(), std::time::SystemTime::now())
            });
        self.set_mode(Mode::MissingMods { world, missing });
        self.focused_button = Some(101);
        self.scroll.set(Default::default());
    }

    pub(super) fn draw_missing_mods(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        (w, h): (f32, f32),
        missing: &[MissingMod],
    ) {
        crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
        let width = w - 32.0;
        let mut lines = wrapped(atlas, &lang::get("horizon.missing_mods.description"), width);
        let description_lines = lines.len();
        lines.push(String::new());
        for m in missing {
            lines.extend(wrapped(atlas, &entry(m), width));
            lines.push(String::new());
        }
        self.message_lines.set(lines.len());
        let (top, bottom, _) = layout(w, h);
        let mut scroll = self.scroll.get();
        scroll.clamp(top, bottom, lines.len() as f32 * 9.0);
        self.scroll.set(scroll);
        centered(
            atlas,
            batch,
            &lang::get("horizon.missing_mods.title"),
            12.0,
            rgba(255, 85, 85, 255),
        );
        let first = batch.vertices.len();
        for (i, line) in lines.iter().enumerate() {
            centered(
                atlas,
                batch,
                line,
                top + i as f32 * 9.0 - (scroll.amount / 9.0).floor() * 9.0,
                if i < description_lines {
                    u32::MAX
                } else {
                    rgba(170, 170, 170, 255)
                },
            );
        }
        batch.clip_since(first, 0.0, top, w, bottom);
        crate::ui::scrollbar(
            batch,
            atlas,
            w - 10.0,
            top,
            bottom,
            lines.len() as f32 * 9.0,
            scroll.amount,
        );
    }

    /// The question's buttons: Don't Open (101), Back Up and Open (102), Open Without a Backup
    /// (103).
    pub(super) fn answer_missing_mods(&mut self, world: WorldEntry, id: u8) -> Action {
        match id {
            102 => self.back_up_for_missing_mods(world),
            103 => return self.open_with_missing_mods(world),
            _ => self.leave_closed_for_missing_mods(),
        }
        Action::None
    }

    /// Don't Open (and Escape): the world stays closed and as it was, and is asked about again
    /// next time.
    pub(super) fn leave_closed_for_missing_mods(&mut self) {
        level_mods::forget_answer();
        let folder = match &self.mode {
            Mode::MissingMods { world, .. } => Some(world.folder.clone()),
            _ => None,
        };
        self.return_to_list(folder.as_deref());
    }

    /// Back Up and Open: a full copy of the world first; it opens once the copy exists.
    fn back_up_for_missing_mods(&mut self, world: WorldEntry) {
        self.open_after_mods_backup = true;
        self.start_backup(world);
    }

    /// The backup Back Up and Open asked for is done: the world opens, with vanilla's "Backed up"
    /// toast naming where the backup is, or, when it could not be made, stays closed and says why.
    pub(super) fn mods_backup_done(
        &mut self,
        world: WorldEntry,
        result: std::result::Result<(PathBuf, u64), String>,
    ) {
        match result {
            Ok((path, bytes)) => {
                log::info!("Horizon: backed up {} to {}", world.folder, path.display());
                self.return_to_list(Some(&world.folder));
                let folder = path.display().to_string();
                self.pending_toast = Some(WorldToast::Created { folder, bytes });
                if let Action::Play { folder } = self.open_with_missing_mods(world) {
                    self.open_request = Some(folder);
                }
            }
            Err(why) => {
                level_mods::forget_answer();
                self.return_to_list(Some(&world.folder));
                self.failed(lang::tr("horizon.missing_mods.backup_failed", &[&why]));
            }
        }
    }

    /// Open Without a Backup, or the backup made: the player chose to open the world anyway.
    fn open_with_missing_mods(&mut self, world: WorldEntry) -> Action {
        level_mods::confirm(&self.saves.join(&world.folder));
        Action::Play {
            folder: world.folder,
        }
    }
}
