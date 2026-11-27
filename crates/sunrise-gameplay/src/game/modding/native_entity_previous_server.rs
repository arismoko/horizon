//! Server adapter for `Entity.xo`, `yo`, `zo`, `yRotO`, `xRotO`, and `Entity.isAlive`.
use crate::game::ServerGame;
use mod_api::EntityRef;
use sunrise_gameplay_common::game::commands_common::{CommandsCommonGameExt2 as _, Target};
use sunrise_gameplay_common::game::native_entity_previous_common::{
    NativeEntityPreviousCommonGameExt as _, Old, list_old,
};
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;
use sunrise_sim::join_seq::JoinSeq;

impl ServerGame<'_> {
    fn native_entity_read_row(&self, entity: EntityRef) -> Option<(Target, Option<JoinSeq>)> {
        let (target, location) = self.native_entity_target_with_location(entity)?;
        Some((target, location.map(|location| location.join)))
    }

    fn target_old(&self, target: Target, join: Option<JoinSeq>) -> Option<Old> {
        let current = || {
            Some(Old {
                position: self.target_position(target)?,
                rotation: self.target_rotation(target),
            })
        };
        match target {
            // A server's player snaps every move it accepts, so at every hook moment the old
            // values match its current position and rotation.
            Target::Player | Target::Guest(_) => current(),
            Target::General(_) => list_old(&self.common.resident_entities.general.rows, join?),
            Target::Hook(id) => {
                let (hook, _) = self.fishing_hook_by_id(id)?;
                Some(Old {
                    position: hook.previous.components(),
                    rotation: [hook.previous_yaw, hook.previous_pitch],
                })
            }
            Target::Frame(_)
            | Target::Painting(_)
            | Target::Other(_)
            | Target::Cushion(_)
            | Target::Cloud(_)
            | Target::Bolt(_)
            | Target::Fangs(_)
            | Target::Spawner(_)
            | Target::Knot(_) => current(),
            _ => self.target_old_resident(target, join),
        }
    }

    /// `Entity.xo`, `yo`, `zo` of the entity a handle names.
    pub fn entity_previous_position(&self, entity: EntityRef) -> Option<[f64; 3]> {
        let (target, join) = self.native_entity_read_row(entity)?;
        Some(self.target_old(target, join)?.position)
    }

    /// `Entity.yRotO`, `xRotO` of the entity a handle names.
    pub fn entity_previous_rotation(&self, entity: EntityRef) -> Option<[f32; 2]> {
        let (target, join) = self.native_entity_read_row(entity)?;
        Some(self.target_old(target, join)?.rotation)
    }

    /// `Entity.isAlive` of the entity a handle names.
    pub fn entity_is_alive(&self, entity: EntityRef) -> Option<bool> {
        let (target, join) = self.native_entity_read_row(entity)?;
        self.target_alive_common(target, join)
    }
}
