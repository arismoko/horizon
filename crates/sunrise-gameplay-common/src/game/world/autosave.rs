//! The world's save on a timer (`MinecraftServer.computeNextAutosaveInterval`,
//! `onTickRateChanged`), the players' autosaves, and the world's metadata (level.dat).
use super::*;

impl crate::game::CommonGame<'_> {
    pub fn admit_entity_save() -> Result<()> {
        if !sunrise_sim::join_seq::visibility::passes_settled() {
            return Err("entity pass is lifted".into());
        }
        Ok(())
    }
}

impl crate::game::CommonGame<'_> {
    pub fn admit_level_save(&self) -> Result<()> {
        // A carrier transfer that broke after `prepared` stopped the
        // session (`carrier_stop_session`): its recovery replays the
        // affected regions from the disk state the transfer itself
        // wrote, so no later save — the autosave a later tick would
        // run, or any exit path's — may write over them.
        if self.world.carrier_fences.is_broken() {
            return Err(
                "a carrier transfer did not finish; the world is left for recovery at reopen"
                    .into(),
            );
        }
        Self::admit_entity_save()?;
        Ok(())
    }
}
