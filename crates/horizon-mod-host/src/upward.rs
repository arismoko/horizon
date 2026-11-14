//! What the game calls up to: the host built around the game's subsystems (`horizon-mod-answers`).
//! The game declares these in `game/host/upward.rs`; the linker joins the two.
#![allow(unsafe_code)]
mod_answers::define_upward!(horizon_client::game::Game<'_>);
