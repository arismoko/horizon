//! Unstable native rendering escape hatch. No API stability is promised.
#![cfg(not(target_arch = "wasm32"))]

/// Raw GPU frame data and the places native mods can inject into.
pub mod render;
