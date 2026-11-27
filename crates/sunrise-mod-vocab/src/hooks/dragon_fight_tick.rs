//! The `dragon_fight_tick` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The End's dragon fight is about to tick (`EnderDragonFight.tick`, which `ServerLevel.tick`
    /// runs before the level's entities), on the server, every tick the level ticks.
    /// `level` is the End. Set `canceled` and the fight does nothing this tick: no player scan
    /// for its boss bar, no arena ticket, no state scan, no respawn stage, no dragon found or
    /// made, no crystal count. A mod that runs a fight of its own does it here, with
    /// `LevelRef::dragon_fight_try_respawn`, `LevelRef::dragon_fight_reset_spike_crystals`
    /// and the dragon's own handle. The dragon and the crystals still tick as entities, and
    /// the pillars' crystals are still placed once with the arena, as world generation
    /// places them in vanilla. NeoForge has no such event.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::dragon_fight::ServerGame<'_>::native_dragon_fight_tick",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "none (NeoForge has no dragon fight event)"
    )]
    fn dragon_fight_tick(level: LevelRef, canceled: &mut bool);
}
