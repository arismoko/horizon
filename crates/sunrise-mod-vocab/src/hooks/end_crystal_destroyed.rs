//! The `end_crystal_destroyed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An End crystal of the End's dragon fight was destroyed, by damage or `/kill`
    /// (`EnderDragonFight.onCrystalDestroyed`), on the server, after the fight answered it.
    /// `level` is the End and `pos` the crystal's block. `respawn_crystal` is true for one
    /// of the crystals driving a respawn: the respawn was aborted, the pillars' crystals
    /// lost their protection and the exit portal is active again. Otherwise the fight
    /// counted the pillars' crystals again and its dragon, when it lives, reacted. NeoForge
    /// has no such event.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::dragon_fight::ServerGame<'_>::native_end_crystal_destroyed",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no dragon fight event)"
    )]
    fn end_crystal_destroyed(level: LevelRef, pos: BlockPos, respawn_crystal: bool);
}
