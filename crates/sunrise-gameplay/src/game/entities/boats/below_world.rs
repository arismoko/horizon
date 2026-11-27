//! `Entity.checkBelowWorld` for boats and minecarts: one more than 64 blocks
//! under its level's floor is discarded (`onBelowWorld`) before it moves.
//! `AbstractBoat.tick` reaches it through `Entity.baseTick`, after the fire;
//! `AbstractMinecart.tick` calls it itself, after its hurt and damage wear off.
//! A discard is a destroying removal: the riders get off where they are and a
//! chest boat or container minecart spills its slots
//! (`AbstractChestBoat.remove`, `AbstractMinecartContainer.remove`), but no
//! ENTITY_DIE and no item of the vehicle itself.
use super::super::{minecarts::cart_at, native_ticks::TickRound};
use crate::game::ServerGame;

/// `Entity.checkBelowWorld`: `getY() < level.getMinY() - 64`.
pub(in crate::game::server_side) fn below_world(y: f64, min_y: i32) -> bool {
    y < f64::from(min_y - 64)
}

impl ServerGame<'_> {
    /// `Entity.checkBelowWorld` for the boat at `i`: true when it fell out of
    /// the world and is gone.
    pub fn boat_below_world(&mut self, i: usize) -> bool {
        if !below_world(
            self.common.boats[i].position.y,
            self.common.world.shape.min_y,
        ) {
            return false;
        }
        self.kill_vehicle_object(crate::game::commands::Target::Boat(self.common.boats[i].id));
        true
    }

    /// The minecart `id` at or after `slot`, unless `AbstractMinecart.tick`'s
    /// `checkBelowWorld` discards it: then its tick is over and the round's
    /// post hook runs for it.
    pub fn cart_in_world(
        &mut self,
        id: u32,
        slot: usize,
        round: &TickRound,
        kind: &str,
    ) -> Option<usize> {
        let i = cart_at(&self.common.carts, id, slot)?;
        if !below_world(
            self.common.carts[i].position.y,
            self.common.world.shape.min_y,
        ) {
            return Some(i);
        }
        self.kill_vehicle_object(crate::game::commands::Target::Cart(id));
        self.native_tick_post_one(round, id, kind);
        None
    }
}

use sunrise_gameplay_common::game::native_ticks::NativeTicksGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::below_world;

    /// The line is 64 under the floor and the vehicle must be below it:
    /// -128 in the overworld (floor -64), -64 in the Nether and the End.
    #[test]
    fn a_vehicle_falls_out_of_the_world_64_blocks_under_the_floor() {
        assert!(!below_world(-128.0, -64));
        assert!(below_world(-128.001, -64));
        assert!(!below_world(-100.0, -64));
        assert!(!below_world(-64.0, 0));
        assert!(below_world(-64.5, 0));
    }
}
