//! The `experience_orb_can_merge` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Whether the experience orb `into` takes the orb `from` in (vanilla's
    /// `ExperienceOrb.canMerge`): the same value, and ids a multiple of 40 apart. Asked
    /// for each pair whose boxes meet, when an orb spawns (`tryMergeToExisting`: `from` is
    /// 0, the new orb isn't an entity yet, and only `from_value` is known) and every 20
    /// ticks as an orb scans for neighbours (`scanForMerges`). `can_merge` holds vanilla's
    /// answer; change it either way. A merged orb counts as `count` orbs of `into`'s
    /// value, so forcing orbs of different values together drops points unless the mod
    /// raises `into`'s value with `ExperienceOrbRef::set_value` in `experience_orb_merge`.
    /// Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_experience_orb_can_merge",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (vanilla ExperienceOrb.canMerge; NeoForge adds no event or extension for it)"
    )]
    fn experience_orb_can_merge(
        dimension: &str,
        into: u32,
        from: u32,
        into_value: i32,
        from_value: i32,
        can_merge: &mut bool,
    );
}
