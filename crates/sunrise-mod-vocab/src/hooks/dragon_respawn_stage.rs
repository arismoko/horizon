//! The `dragon_respawn_stage` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The End's dragon respawn reached `stage` (`EnderDragonFight.setRespawnStage`, and
    /// `respawnDragon`'s start), on the server. `level` is the End. Heard once per stage:
    /// [`DragonRespawnStage::Start`] when the crystals on the exit portal start it,
    /// each later stage as the sequence moves on, and [`DragonRespawnStage::End`] once the
    /// new dragon flies. A respawn aborted by a destroyed portal crystal reaches no stage
    /// (`end_crystal_destroyed` hears it). NeoForge has no such event.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::dragon_fight::ServerGame<'_>::native_dragon_respawn_stage",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no dragon fight event)"
    )]
    fn dragon_respawn_stage(level: LevelRef, stage: mod_api::DragonRespawnStage);
}
