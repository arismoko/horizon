//! The `visuals` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The world frame's local, world-space rim and colored-light sources.
    /// Push requests; Sunrise validates and limits them before rendering.
    /// Called only while a world is visible.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_visuals",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "none (Sunrise-only; NeoForge has no counterpart)"
    )]
    fn visuals(rims: &mut Vec<BlockRim>, lights: &mut Vec<ColoredLight>);
}
