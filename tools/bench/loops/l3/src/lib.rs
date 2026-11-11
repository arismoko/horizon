//! L3: L1's logic as a cancellable HEAD mixin on `tick_hopper` (`src/mixins/head.rs`).
#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl mod_api::Hooks for Mod {}
