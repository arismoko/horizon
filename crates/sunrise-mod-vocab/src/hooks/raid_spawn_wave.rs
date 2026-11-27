//! The `raid_spawn_wave` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A raid wave has been spawned at `pos`; `wave` is its one-based wave
    /// number and `raiders` counts the raiders, not the mobs they ride.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_raid_spawn_wave",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn raid_spawn_wave(raid: i32, pos: [i32; 3], wave: i32, raiders: usize);
}
