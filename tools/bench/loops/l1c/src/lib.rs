//! L1C: a `[[native.loop]]` slot of `block_entities.tick` (`src/loops/slot.rs`).
#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl mod_api::Hooks for Mod {}
