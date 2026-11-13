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

/// The prompt's text: what opening does, then each missing mod.
fn message(missing: &[MissingMod]) -> String {
    let mut message = lang::get("horizon.missing_mods.description");
    for m in missing {
        message.push('\n');
        message.push_str(&level_mods::describe(m));
    }
    message
}

/// The most lines of text that leave the heading and both rows of buttons on a screen `h` high.
/// `confirm_top` centres a column of 61 plus the text, and the second row of buttons ends 85
/// plus the text below its top, so the text may take `h - 109`, less 8 to spare.
fn max_lines(h: f32) -> usize {
    ((h - 109.0 - 8.0) / 9.0).floor().max(1.0) as usize
}

/// The description, then each mod's lines while they fit in `max` lines; when some don't, the
/// last line says how many mods are left out. The narration reads them all (`screen_info`).
fn fit(description: Vec<String>, mods: Vec<Vec<String>>, max: usize) -> Vec<String> {
    let total = description.len() + mods.iter().map(Vec::len).sum::<usize>();
    let mut lines = description;
    if total <= max {
        lines.extend(mods.into_iter().flatten());
        return lines;
    }
    lines.truncate(max.saturating_sub(1));
    let mut shown = 0;
    for m in &mods {
        if lines.len() + m.len() + 1 > max {
            break;
        }
        lines.extend(m.iter().cloned());
        shown += 1;
    }
    lines.push(lang::tr(
        "horizon.missing_mods.more",
        &[&(mods.len() - shown)],
    ));
    lines
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

impl Title {
    /// Shows the question for the world in `folder`, whose `missing` mods no listener resolved.
    pub fn ask_missing_mods(&mut self, folder: &str, missing: Vec<MissingMod>) {
        let world = list_worlds(&self.saves)
            .into_iter()
            .find(|w| w.folder == folder)
            .unwrap_or_else(|| {
                WorldEntry::corrupted(folder.to_owned(), std::time::SystemTime::now())
            });
        self.set_mode(Mode::MissingMods { world, missing });
    }

    pub(super) fn draw_missing_mods(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        (w, h): (f32, f32),
        missing: &[MissingMod],
    ) {
        crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
        let width = w - 50.0;
        let description = wrapped(atlas, &lang::get("horizon.missing_mods.description"), width);
        let mods = missing
            .iter()
            .map(|m| wrapped(atlas, &level_mods::describe(m), width))
            .collect();
        let budget = max_lines(h - if w < 324.0 { 24.0 } else { 0.0 });
        let lines = fit(description, mods, budget);
        self.message_lines.set(lines.len());
        let top = confirm_top(h, lines.len());
        centered(
            atlas,
            batch,
            &lang::get("horizon.missing_mods.title"),
            top,
            rgba(255, 85, 85, 255),
        );
        for (i, line) in lines.iter().enumerate() {
            centered(atlas, batch, line, top + 17.0 + i as f32 * 9.0, u32::MAX);
        }
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
