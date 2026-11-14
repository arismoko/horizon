//! The interfaces the engine's subsystems offer to code that answers a mod's calls.
//!
//! Each trait is what one subsystem owns (entities, levels), not what one host body happens to
//! call. The game implements them over its private state; the crates that answer the mod API are
//! written against them and never name the game. This crate depends on the mod vocabulary's value
//! types and nothing above it.
#![forbid(unsafe_code)]

mod entities;
mod levels;

pub use entities::{EntityIdentity, EntityLeash, EntityRead, EntityView, UuidState};
pub use levels::{LevelRead, LevelWrite};

/// The subsystems a host answers from, as one bound. The game implements each; a fake in a test
/// implements the ones its body uses.
pub trait Subsystems: EntityRead + EntityIdentity + EntityLeash + LevelRead + LevelWrite {}
impl<T: EntityRead + EntityIdentity + EntityLeash + LevelRead + LevelWrite> Subsystems for T {}
