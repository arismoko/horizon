//! Client adapter for `Entity.xo`, `yo`, `zo`, `yRotO`, `xRotO`, and `Entity.isAlive`.
use crate::game::ClientGame;
use mod_api::EntityRef;
use sunrise_gameplay_common::game::commands_common::{CommandsCommonGameExt2 as _, Target};
use sunrise_gameplay_common::game::native_entity_previous_common::{
    NativeEntityPreviousCommonGameExt as _, Old, list_join_by_id, list_old,
};
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
use sunrise_sim::join_seq::JoinSeq;

impl ClientGame<'_> {
    fn native_client_join(&self, target: Target) -> Option<JoinSeq> {
        match target {
            Target::Mob(id) => list_join_by_id(&self.common.mobs, id),
            Target::Item(id) => list_join_by_id(&self.common.dropped, id),
            Target::Orb(id) => list_join_by_id(&self.common.orbs, id),
            Target::Projectile(id) => list_join_by_id(&self.common.creatures.projectiles, id),
            Target::Stand(id) => list_join_by_id(&self.common.armor_stands, id),
            Target::Boat(id) => list_join_by_id(&self.common.boats, id),
            Target::Cart(id) => list_join_by_id(&self.common.carts, id),
            Target::Tnt(id) => list_join_by_id(&self.common.tnts, id),
            Target::Falling(id) => list_join_by_id(&self.common.falling, id),
            Target::Rocket(id) => list_join_by_id(&self.common.rockets, id),
            Target::Trident(id) => list_join_by_id(&self.common.tridents, id),
            Target::Crystal(id) => self
                .common
                .crystals
                .iter()
                .find(|crystal| crystal.id == id)
                .map(|crystal| crystal.join),
            Target::General(id) => list_join_by_id(&self.common.resident_entities.general.rows, id),
            _ => None,
        }
    }

    fn native_entity_read_row(&self, entity: EntityRef) -> Option<(Target, Option<JoinSeq>)> {
        let target = self.native_client_entity_target(entity)?;
        let join = self.native_client_join(target);
        Some((target, join))
    }

    fn target_old(&self, entity: EntityRef, target: Target, join: Option<JoinSeq>) -> Option<Old> {
        let current = || {
            Some(Old {
                position: self.entity_position(entity)?,
                rotation: self.target_rotation(target),
            })
        };
        match target {
            // The client's own player: where the camera and the model ease from (`LocalPlayer`'s
            // xo, yo, zo), and `yRotO`, `xRotO`, which `Entity.turn` moves with the view.
            Target::Player => Some(Old {
                position: self.common.previous_position.components(),
                rotation: [self.client.rope_rot_old.0, self.client.rope_rot_old.1],
            }),
            Target::General(id) => join
                .and_then(|join| list_old(&self.common.resident_entities.general.rows, join))
                .or_else(|| {
                    // A client's copy of a row its server shows it.
                    let row = self
                        .client
                        .remote
                        .as_ref()?
                        .general
                        .rows
                        .get(&(id as i32))?;
                    Some(Old {
                        position: row.previous,
                        rotation: row.previous_rotation,
                    })
                }),
            Target::Guest(_)
            | Target::Hook(_)
            | Target::Frame(_)
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
        match self.native_entity_read_row(entity) {
            Some((target, join)) => Some(self.target_old(entity, target, join)?.position),
            // An entity a client tracks without a body of its own, as `entity_position` reads it.
            None => {
                let (remote, id) = self.native_client_entity_context(entity).ok()?;
                if let Some(row) = remote.general.rows.get(&id) {
                    return Some(row.previous);
                }
                self.native_client_tracked(entity)?;
                if let Some(display) = self.client.text_displays.0.get(&id) {
                    return Some(display.previous_position().to_array());
                }
                remote.entities.get(&id).map(|tracked| tracked.previous)
            }
        }
    }

    /// `Entity.yRotO`, `xRotO` of the entity a handle names.
    pub fn entity_previous_rotation(&self, entity: EntityRef) -> Option<[f32; 2]> {
        let (target, join) = self.native_entity_read_row(entity)?;
        Some(self.target_old(entity, target, join)?.rotation)
    }

    /// `Entity.isAlive` of the entity a handle names.
    pub fn entity_is_alive(&self, entity: EntityRef) -> Option<bool> {
        let (target, join) = self.native_entity_read_row(entity)?;
        self.target_alive_common(target, join)
    }
}
