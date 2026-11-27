//! Common End environment and portal-frame aim helpers shared by server and client games.
use super::{Resources, side_ops::world::SideOps as _};
use sunrise_level::environment::Environment;

pub trait EndGameExt<'__sg>: crate::game::SideGame<'__sg> {
    /// The environment attributes of the current dimension.
    fn dim_environment<'__loan>(&'__loan self) -> &'__loan Environment
    where
        '__sg: '__loan,
    {
        Self::level_environment(
            self.side_state().side_remote_environment(),
            &self.res,
            self.dim.id(),
            &self.dimension_type().id,
        )
    }

    /// A field-level borrow for the client tick, which also mutably borrows its particles.
    fn level_environment<'a>(
        remote_environment: Option<&'a Environment>,
        resources: &'a Resources,
        level: &str,
        dimension_type: &str,
    ) -> &'a Environment {
        // A client of a server takes the type the server sent, built-in levels included.
        remote_environment.unwrap_or_else(|| {
            Environment::for_level(level, resources.environment_of(dimension_type))
        })
    }

    /// EnderEyeItem.use's `getPlayerPOVHitResult`: whether the block aimed
    /// at is an End portal frame (the use passes there).
    fn aims_at_end_portal_frame(&self) -> bool {
        self.pick()
            .is_some_and(|hit| &*self.block_name(hit.pos) == "minecraft:end_portal_frame")
    }
}
impl<'__sg, T: crate::game::SideGame<'__sg>> EndGameExt<'__sg> for T {}

use crate::game::tables::WorldTablesExt as _;

use crate::game::GameGameExt1 as _;

use crate::game::pick_ray::PickRayGameExt1 as _;
