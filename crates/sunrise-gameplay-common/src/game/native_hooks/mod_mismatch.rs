use super::*;
pub use sunrise_sim::save::level_mods;

pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// Dispatch the saved list at the earliest Game Host boundary on either side.
    /// Each resolution keeps its first resolving listener across dispatch.
    fn native_mod_mismatch(
        &mut self,
        mismatches: &[mod_api::ModMismatch],
        resolved: &mut Vec<mod_api::ModMismatchResolution>,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_mod_mismatch(self, mismatches, resolved);
    }

    /// Before the world in `dir` opens: the saved mods whose content opening it would lose
    /// (`level_mods::missing`) that no `mod_mismatch` listener resolves. Only when one is missing
    /// do the mods hear the differences here, before the world opens; the open then uses this
    /// answer instead of asking again (`check_mod_mismatch`). Reads only: nothing in `dir` changes.
    fn ask_missing_mods(&mut self, dir: &std::path::Path) -> Vec<level_mods::MissingMod> {
        let loaded = level_mods::Loaded::now();
        let missing = level_mods::missing_in(dir, &loaded);
        if missing.is_empty() {
            return missing;
        }
        // Without a saved mod list the hook does not fire (block namespaces alone), but the
        // answer is still kept, for the player's choice to attach to.
        let mismatches = level_mods::mismatches_in(dir, &loaded);
        let mut resolved = Vec::new();
        if let Some(mismatches) = &mismatches {
            self.native_mod_mismatch(mismatches, &mut resolved);
        }
        let mismatches = mismatches.unwrap_or_default();
        let missing = level_mods::unresolved(missing, &resolved);
        level_mods::remember_answer(dir, mismatches, resolved);
        missing
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
