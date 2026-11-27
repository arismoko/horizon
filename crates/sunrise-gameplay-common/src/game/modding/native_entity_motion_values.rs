use crate::game::{
    commands_common::Target,
    entity::{Entity as _, EntityHandle, with_entity},
};
use mod_api::EntityRef;
use sunrise_core::{
    geometry::{Aabb, Axis, Vec3},
    position::BlockPos,
};

/// `Entity.calculateViewVector` for a look turned `pitch_offset` degrees down: the unit
/// vector along `yaw` and `pitch + pitch_offset`.
pub fn turned_view_vector(yaw: f32, pitch: f32, pitch_offset: f32) -> [f64; 3] {
    let v = sunrise_world::view_direction::view_vector(pitch + pitch_offset, yaw);
    [v.x, v.y, v.z]
}

/// `Entity.getOnPos()`: the supporting block when there is one, else the block `1.0E-5F`
/// under the feet at `position`.
pub fn on_block_position(support: Option<BlockPos>, position: Vec3) -> [i32; 3] {
    match support {
        Some(p) => [p.x, p.y, p.z],
        None => [
            position.x.floor() as i32,
            (position.y - f64::from(1.0e-5_f32)).floor() as i32,
            position.z.floor() as i32,
        ],
    }
}

pub fn corners(b: Aabb) -> [[f64; 3]; 2] {
    let axes = [Axis::X, Axis::Y, Axis::Z];
    [axes.map(|a| b.min(a)), axes.map(|a| b.max(a))]
}

impl crate::game::CommonGame<'_> {
    /// The simulated entity (one of the shared entity walk's lists) with `id`.
    pub fn native_walk_entity(&self, id: u32) -> Option<EntityHandle> {
        self.entity_handles()
            .find(|h| with_entity!(self, *h, e => e.id() == id))
    }
}

pub trait NativeEntityMotionValuesGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// The server's copy of the player a target names.
    fn native_target_guest<'a>(&'a self, t: Target) -> Option<&'a sunrise_sim::lan::guest::Guest>
    where
        '__sg: 'a,
    {
        match t {
            Target::Player => self.command_guest(crate::game::Side::lan_acting(self.side_state())?),
            Target::Guest(conn) => self.command_guest(conn),
            _ => None,
        }
    }

    /// Whether the entity stands on a block (`Entity.onGround`). API 379.
    fn entity_on_ground(&self, entity: EntityRef) -> Option<bool> {
        let t = self.native_entity_read_target(entity)?;
        if t == Target::Player
            && let Some(p) = self.player.as_ref()
        {
            return Some(p.ground.on_ground);
        }
        if let Some(g) = self.native_target_guest(t) {
            return Some(g.on_ground);
        }
        if let Target::Hook(id) = t
            && let Some(on_ground) = self.side_fishing_hook_on_ground(id)
        {
            return Some(on_ground);
        }
        Some(
            self.native_walk_entity(entity.id())
                .is_some_and(|h| with_entity!(self, h, e => e.on_ground())),
        )
    }

    /// The unit vector that points up out of the entity's head
    /// (`Entity.getUpVector`): its view vector turned a quarter of a turn
    /// toward the zenith.
    fn entity_up_vector(&self, entity: EntityRef) -> Option<[f64; 3]> {
        let [yaw, pitch] = self.entity_rotation(entity)?;
        Some(turned_view_vector(yaw, pitch, -90.0))
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeEntityMotionValuesGameExt1<'__sg> for G {}

use crate::game::side_ops::items::ItemsSideGameExt as _;

use crate::game::commands_common::CommandsCommonGameExt2 as _;

use crate::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use crate::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;
