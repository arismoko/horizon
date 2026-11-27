use sunrise_sim::save::level_mods;

impl crate::game::ServerGame<'_> {
    /// A server opened a world: its saved code and content mods against the loaded ones.
    /// The mods hear the differences once (before the open, when a mod was missing and the
    /// open asked first: `ask_missing_mods`), and each one nobody resolved is a warning.
    pub fn check_mod_mismatch(&mut self) {
        let (mismatches, resolved) = match level_mods::take_answer(&self.server.save.0.root) {
            Some(answer) => answer,
            None => {
                if self.common.level.saved_mods.is_none() {
                    return;
                }
                let mismatches = self
                    .common
                    .level
                    .mod_mismatches(&level_mods::Loaded::now().mods());
                let mut resolved = Vec::new();
                self.native_mod_mismatch(&mismatches, &mut resolved);
                (mismatches, resolved)
            }
        };
        let shown = |version: &str| -> String {
            if version.is_empty() {
                "an unknown version".to_owned()
            } else {
                version.to_owned()
            }
        };
        let mut ordered: Vec<_> = mismatches.iter().collect();
        ordered.sort_by(|a, b| a.mod_id.cmp(&b.mod_id));
        for mismatch in ordered {
            let current = mismatch
                .current_version
                .as_deref()
                .map_or_else(|| "missing".to_owned(), shown);
            if let Some(resolution) = resolved.iter().find(|r| r.mod_id == mismatch.mod_id) {
                log::debug!(
                    "Sunrise: mod {} was {}, now {}; resolved by {} ({})",
                    mismatch.mod_id,
                    shown(&mismatch.saved_version),
                    current,
                    resolution.resolver,
                    if resolution.resolver == mismatch.mod_id {
                        "self"
                    } else {
                        "other mod"
                    }
                );
            } else {
                log::warn!(
                    "Sunrise: mod {} was {} when the world was saved, now {}",
                    mismatch.mod_id,
                    shown(&mismatch.saved_version),
                    current
                );
            }
        }
        if mismatches
            .iter()
            .any(|m| !resolved.iter().any(|r| r.mod_id == m.mod_id))
        {
            log::warn!("Sunrise: changed mods may cause the world to work incorrectly");
        }
        if mismatches
            .iter()
            .any(|m| resolved.iter().any(|r| r.mod_id == m.mod_id))
        {
            log::debug!("Sunrise: changed mods may cause the world to work incorrectly");
        }
    }
}

use sunrise_gameplay_common::game::native_hooks::mod_mismatch::GameHooks as _;
