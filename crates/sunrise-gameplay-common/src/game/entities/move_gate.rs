//! The game layer's door of the move gate (#2414): who is moving, and the one
//! `Game::entity_move` the pushers and family ticks arm a move through. PR 1a
//! carries no hook — every decision is `Run` and the tick cost is one branch;
//! PR 1b fires the `entity_move` hook inside the client `move_gate::arm` this
//! door calls, with a revision recheck before the kind's physics runs.
use super::entity::{Entity, EntityHandle, with_entity};
use std::borrow::Cow;
use sunrise_core::geometry::Vec3;
use sunrise_world::entity_move::MoverType;
use sunrise_world::move_gate::MoveGate;

/// Which body a move is for: the local player window, a LAN guest, or a listed
/// entity. The piston and shulker-box pushes and the family ticks name their
/// mover with it.
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub enum MoveWho {
    Player,
    Guest(u32),
    Entity(EntityHandle),
}

impl crate::game::CommonGame<'_> {
    /// The kind name (`minecraft:zombie`) a move is for, built on demand.
    fn move_who_kind(&self, who: MoveWho) -> Cow<'_, str> {
        match who {
            MoveWho::Player | MoveWho::Guest(_) => Cow::Borrowed("minecraft:player"),
            MoveWho::Entity(handle) => with_entity!(self, handle, entity => entity.native_type()),
        }
    }
}

pub trait MoveGateGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// Arm one physical `Entity.move` invocation (#2414) for `who` with `cause`
    /// and hand back its gate: the caller then runs the kind's physics with
    /// the movement the gate decided (`delta`), and skips it when the gate
    /// says so. A pusher arms before its own restrictions — the piston limit
    /// included — so a cancelled move spends none of them.
    fn entity_move(&mut self, who: MoveWho, cause: MoverType, delta: Vec3) -> MoveGate {
        let entity = self.move_who_id(who);
        let client_controlled = match who {
            MoveWho::Player | MoveWho::Guest(_) => true,
            MoveWho::Entity(EntityHandle::Mob(i)) => {
                sunrise_sim::mobs::equine::controlled(&self.mobs[i])
            }
            MoveWho::Entity(EntityHandle::Boat(i)) => {
                self.side_boat_client_controlled(self.boats[i].id)
            }
            MoveWho::Entity(_) => false,
        };
        // The type name is built only if a listener asks for it (#2414: a
        // move nobody listens to costs one branch and no string).
        sunrise_sim::move_gate::armed_with_controlled(
            &self.loop_context,
            entity,
            || self.move_who_kind(who),
            cause,
            delta,
            client_controlled,
        )
    }

    /// The entity id a move is for; the player window's own id for it.
    fn move_who_id(&self, who: MoveWho) -> u32 {
        match who {
            MoveWho::Player => self.player.as_ref().map_or(0, |p| p.entity_id as u32),
            MoveWho::Guest(conn) => self.side_move_guest_entity_id(conn).unwrap_or(0),
            MoveWho::Entity(handle) => with_entity!(self, handle, entity => entity.id()),
        }
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> MoveGateGameExt1<'__sg> for G {}

use crate::game::side_ops::mobs_rest::MobsRestSideGameExt as _;
