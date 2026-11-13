//! The `experience_orb_merge` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0130

pub trait Hooks {
    /// The experience orb `into` is about to take `from` in, after
    /// `experience_orb_can_merge` said yes (`from` is 0 and `from_value` the
    /// points of a new orb that is spawning). Set `canceled` to stop it: both orbs stay, and
    /// a spawning orb tries the next orb in reach, then spawns on its own. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        cancellable = true,
        neoforge = "none (vanilla ExperienceOrb.tryMergeToExisting; NeoForge adds no event or extension for it)"
    )]
    fn experience_orb_merge();
}
