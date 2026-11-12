//! The game's slot table, in the one registry every point table shares
//! (`mod_api::__private::points`).
use crate::{POINT_COUNT, PointId};
use mod_api::__private::points::{Place, Table};
use std::sync::atomic::AtomicBool;

pub use mod_api::__private::points::{RegisterError, register, seal, stop};

/// Each place's slot: set once, at the seal, where a mod listens. A point that nothing listens at
/// costs one load of it.
pub static POINTS: [AtomicBool; POINT_COUNT] = [const { AtomicBool::new(false) }; POINT_COUNT];
/// The game's API point table. Only the prebuilt game has the points' sites; any other build's
/// table refuses every listener by mod and target.
pub static TABLE: Table = if crate::STAGED {
    Table::new(&POINTS)
} else {
    Table::without_sites(&POINTS, crate::POINT_TARGETS)
};

impl PointId {
    /// This place in the game's table, for a listener's constructor.
    pub const fn place(self) -> Place {
        Place::new(&TABLE, self as usize)
    }
}
