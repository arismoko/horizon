//! Shared helpers for previous-tick entity position, rotation and alive queries (`Entity.xo`,
//! `yo`, `zo`, `yRotO`, `xRotO`, `Entity.isAlive`) used by server and client adapters.
use crate::game::commands_common::CommandsCommonGameExt2 as _;
use crate::game::commands_common::Target;
use crate::game::dragon_common::DragonCommonGameExt1 as _;
use crate::game::entity::Entity;
use crate::game::native_living_body::NativeLivingBodyGameExt1 as _;
use sunrise_sim::join_seq::{JoinList, JoinSeq, Joined};

/// What `Entity.setOldPosAndRot` kept when the entity's tick began: `xo`, `yo`, `zo` and
/// `yRotO`, `xRotO`.
#[derive(Clone, Copy)]
pub struct Old {
    pub position: [f64; 3],
    pub rotation: [f32; 2],
}

/// The old values of the body at `join` in `list`.
pub fn list_old<T: Entity + Joined, const DIRECT: bool>(
    list: &JoinList<T, DIRECT>,
    join: JoinSeq,
) -> Option<Old> {
    let body = list.get(list.locate(join)?)?;
    Some(Old {
        position: body.previous_position().components(),
        rotation: body.previous_rotation(),
    })
}

/// `Entity.isAlive` of the body at `join` in `list`: not removed, and for a living kind
/// (a mob, an armor stand) its health above 0.
pub fn list_alive<T: Entity + Joined, const DIRECT: bool>(
    list: &JoinList<T, DIRECT>,
    join: JoinSeq,
) -> Option<bool> {
    let body = list.get(list.locate(join)?)?;
    Some(!body.is_removed() && body.is_alive())
}

/// Locate a resident body's `JoinSeq` by its network entity id (used by client-local lookup).
pub fn list_join_by_id<T: Entity + Joined, const DIRECT: bool>(
    list: &JoinList<T, DIRECT>,
    id: u32,
) -> Option<JoinSeq> {
    list.iter()
        .find(|body| body.id() == id)
        .map(Joined::join_seq)
}

pub trait NativeEntityPreviousCommonGameExt<'__sg>: crate::game::SideGame<'__sg> {
    /// Previous position and rotation for resident body kinds shared across both sides.
    fn target_old_resident(&self, target: Target, join: Option<JoinSeq>) -> Option<Old> {
        match target {
            Target::Mob(_) => list_old(&self.mobs, join?),
            Target::Item(_) => list_old(&self.dropped, join?),
            Target::Orb(_) => list_old(&self.orbs, join?),
            Target::Projectile(_) => list_old(&self.creatures.projectiles, join?),
            Target::Stand(_) => list_old(&self.armor_stands, join?),
            Target::Boat(_) => list_old(&self.boats, join?),
            Target::Cart(_) => list_old(&self.carts, join?),
            Target::Tnt(_) => list_old(&self.tnts, join?),
            Target::Falling(_) => list_old(&self.falling, join?),
            Target::Rocket(_) => list_old(&self.rockets, join?),
            Target::Trident(_) => list_old(&self.tridents, join?),
            Target::Dragon(_) => {
                let dragon = self.dragon.as_ref()?;
                Some(Old {
                    position: dragon.previous.components(),
                    rotation: [dragon.previous_yaw, dragon.pitch],
                })
            }
            Target::DragonPart(id) => {
                let (dragon, part) = self.resident_dragon_part(id)?;
                Some(Old {
                    position: dragon.parts[part].1.components(),
                    rotation: self.target_rotation(target),
                })
            }
            Target::Crystal(_) => {
                let crystal = self.crystals.get(self.crystals.locate(join?)?)?;
                Some(Old {
                    position: crystal.previous.components(),
                    rotation: crystal.rotation,
                })
            }
            _ => None,
        }
    }

    /// `Entity.isAlive` for a resolved `target` and optional resident `join`.
    fn target_alive_common(&self, target: Target, join: Option<JoinSeq>) -> Option<bool> {
        match target {
            // `LivingEntity.isAlive`: a player's health, synced (`DATA_HEALTH_ID`) on a client.
            Target::Player | Target::Guest(_) => {
                let body = self.native_living_body(target)?;
                Some(crate::game::native_living_body::body_health(body)? > 0.0)
            }
            Target::Dragon(_) => Some(self.dragon.as_ref()?.health > 0.0),
            Target::Mob(_) => list_alive(&self.mobs, join?),
            Target::Item(_) => list_alive(&self.dropped, join?),
            Target::Orb(_) => list_alive(&self.orbs, join?),
            Target::Projectile(_) => list_alive(&self.creatures.projectiles, join?),
            Target::Stand(_) => list_alive(&self.armor_stands, join?),
            Target::Boat(_) => list_alive(&self.boats, join?),
            Target::Cart(_) => list_alive(&self.carts, join?),
            Target::Tnt(_) => list_alive(&self.tnts, join?),
            Target::Falling(_) => list_alive(&self.falling, join?),
            Target::Rocket(_) => list_alive(&self.rockets, join?),
            Target::Trident(_) => list_alive(&self.tridents, join?),
            // A client keeps a row it shows until the server removes it.
            Target::General(_) => join
                .and_then(|join| list_alive(&self.resident_entities.general.rows, join))
                .or(Some(true)),
            // Not living, and a handle names them only while they are in their level.
            Target::Frame(_)
            | Target::Painting(_)
            | Target::Other(_)
            | Target::Crystal(_)
            | Target::DragonPart(_)
            | Target::Cushion(_)
            | Target::Cloud(_)
            | Target::Bolt(_)
            | Target::Hook(_)
            | Target::Fangs(_)
            | Target::Spawner(_)
            | Target::Knot(_) => Some(true),
        }
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeEntityPreviousCommonGameExt<'__sg> for G {}
