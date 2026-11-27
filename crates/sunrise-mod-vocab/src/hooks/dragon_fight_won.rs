//! The `dragon_fight_won` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The End's dragon fight was won: its own dragon died, by its death animation or
    /// `/kill` (`EnderDragonFight.setDragonKilled`), on the server. Asked on every win,
    /// after the exit portal was activated and the next gateway placed, before the egg and
    /// before the fight is marked won. `level` is the End, `egg_pos` the top of the
    /// podium's column on the `MOTION_BLOCKING` heightmap (`LevelReader.getHeightmapPos`),
    /// where vanilla puts the egg, and `previously_won` whether a dragon of this fight was
    /// killed before. Vanilla places the egg on the first win only.
    ///
    /// Answer `Some(true)` to place a dragon egg at `egg_pos` (`setBlockAndUpdate`),
    /// `Some(false)` to place none, or `None` to leave it to the next listener and then to
    /// vanilla. A refused block write is not retried, and the win is recorded either way.
    /// The host is read-only. NeoForge has no such event.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::dragon_fight::ServerGame<'_>::native_dragon_fight_won",
        receiver = "read",
        dispatch = "flat",
        side = "server",
        contract = "replace",
        out = "place_egg",
        host = "read",
        neoforge = "none (NeoForge has no dragon fight event)",
        paper = "DragonEggFormEvent"
    )]
    fn dragon_fight_won(
        level: LevelRef,
        egg_pos: BlockPos,
        previously_won: bool,
        place_egg: &mut Option<bool>,
    );
}
