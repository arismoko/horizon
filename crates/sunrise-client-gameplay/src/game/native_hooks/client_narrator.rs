//! `ClientNarratorClientHost::narrate`: a client mod's line for the player's
//! narrator, through the game's own `GameNarrator` (vanilla's
//! `Minecraft.getNarrator()`).
use mod_api::{NarrateError, Narration};
use sunrise_narrator::GameNarrator;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn narrate(
        &mut self,
        text: &sunrise_game_data::text::Text,
        when: Narration,
    ) -> Result<(), NarrateError> {
        debug_assert!(self.options.execution_role.is_client());
        let line = text.plain();
        sunrise_narrator::with_narrator(|narrator| say_for_mod(narrator, &line, when))
    }
}

/// `saySystemNow` or `saySystemQueued` for a mod, refusing by name what
/// vanilla would drop silently: an empty line, an option that does not
/// narrate system messages, no speech. A refused line is not logged.
pub(in crate::game) fn say_for_mod(
    narrator: &mut GameNarrator,
    line: &str,
    when: Narration,
) -> Result<(), NarrateError> {
    if line.is_empty() {
        return Err(NarrateError::Empty);
    }
    if !narrator.status().should_narrate_system() {
        return Err(NarrateError::Off);
    }
    if !narrator.is_active() {
        return Err(NarrateError::Unavailable);
    }
    match when {
        Narration::Now => narrator.say_system_now(line),
        // `Queued`, and any later mode: never interrupts.
        _ => narrator.say_system_queued(line),
    }
    Ok(())
}
