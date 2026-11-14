//! The code that answers a mod's calls on the host, written against the subsystem traits of
//! `horizon-engine-access` and never against the game.
//!
//! - [`bodies`]: the host bodies, generic over the traits they read and write through.
//! - [`goal`]: the goal-tick context and its bodies, over a level the game lends.
//! - [`HostOf`]: the type a listener receives as its `dyn ServerHost`, built around the game's
//!   subsystems by the code above the game.
//!
//! This crate is below `horizon-client`'s users and beside nothing: it does not depend on it.
#![forbid(unsafe_code)]
// The generated impls forward argument lists as they are.
#![allow(clippy::useless_conversion, clippy::unit_arg, clippy::needless_borrow)]

pub mod bodies;
pub mod goal;
mod host_of;

pub use host_of::{HostOf, Legacy};

#[cfg(test)]
mod tests;

#[doc(hidden)]
pub mod __private {
    pub use horizon_client_lib::mobs::{Mob, MobWorld};
    pub use mod_api::{ClipHit, dispatch_entity_tick_pre};
}

/// Defines, for the game type `$game`, the functions the game declares in `game/host/upward.rs`:
/// what the game calls up to. Expanded once by the executable's host crate, and once by the
/// game's own test build (which links no host crate).
#[macro_export]
macro_rules! define_upward {
    ($game:ty) => {
        #[unsafe(no_mangle)]
        pub fn horizon_up_entity_tick_pre(
            game: &mut $game,
            dimension: &str,
            entity: u32,
            kind: &str,
            canceled: &mut bool,
        ) {
            $crate::__private::dispatch_entity_tick_pre(
                &mut $crate::HostOf::new(game),
                dimension,
                entity,
                kind,
                canceled,
            );
        }

        #[unsafe(no_mangle)]
        pub fn horizon_up_goal_pick(
            game: &$game,
            mob: &$crate::__private::Mob,
            w: &$crate::__private::MobWorld<'_>,
            distance: f64,
        ) -> Option<$crate::__private::ClipHit> {
            $crate::goal::GoalCx { m: mob, w, level: game }.pick_own(distance)
        }
    };
}
