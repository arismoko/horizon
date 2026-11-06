//! The `fluid_draw` area: host methods the game implements (`game/host/fluid_draw.rs`).
#![allow(unused_variables)]
/// The client host's methods of the `fluid_draw` area, implemented by the game and reached through [`ClientHost`](crate::ClientHost).
pub trait FluidDrawClientHost {
    /// Side: client.
    /// Fluid `fluid`'s name as the lang names it (`fluid_type.<mod>.<name>`, vanilla's by its block),
    /// else its path in words.
    fn fluid_name(&self, fluid: &str) -> String {
        crate::phase::unprovided(
            "ClientHost::fluid_name",
            "fluid_name",
            crate::phase::LoadAccess::ClientSetup,
        )
    }
}
