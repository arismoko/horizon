//! The `player_render_listened` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0161

pub trait Hooks {
    /// Whether a mod listens to `player_submit` or `mob_layer`: a
    /// mod that hooks either hooks this one too and sets `listened` (HEAD,
    /// `*listened = true`). Without it the game poses no player or mob for
    /// them, each frame. Client-side.
    #[hook(
        side = "client",
        contract = "collect",
        out = "listened",
        cancellable = false,
        neoforge = "none (Horizon-only; NeoForge has no counterpart); an opt-in gate for a hook"
    )]
    fn player_render_listened();
}
