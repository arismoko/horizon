//! The calls the game makes up to the code that answers a mod's calls, which lives in crates
//! above this one (`horizon-mod-answers`, built into the executable by `horizon-mod-host`).
//!
//! Each function is declared here and defined above under the same name (`mod_answers::
//! define_upward!`), resolved by the linker: a direct call, with nothing loaded or set at
//! startup. The two sides name the same types, so a mismatched signature is the one thing the
//! compiler cannot catch; the generator emits both sides from one list.
#![allow(unsafe_code)]
use super::super::Game;
use crate::mobs::{Mob, MobWorld};
use mod_api::ClipHit;

unsafe extern "Rust" {
    /// `dispatch_entity_tick_pre` with the game's host built above.
    pub(in crate::game) safe fn horizon_up_entity_tick_pre(
        game: &mut Game<'_>,
        dimension: &str,
        entity: u32,
        kind: &str,
        canceled: &mut bool,
    );
    /// A mod goal's `entity_pick` for the mob in hand, over the level the game lends.
    pub(in crate::game) safe fn horizon_up_goal_pick(
        game: &Game<'_>,
        mob: &Mob,
        w: &MobWorld<'_>,
        distance: f64,
    ) -> Option<ClipHit>;
}

/// The game's own test binary links no host crate, so it defines what the game calls up to.
#[cfg(test)]
mod defined_for_tests {
    mod_answers::define_upward!(crate::game::Game<'_>);
}
