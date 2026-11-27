use super::entity::{Entity as _, EntityHandle, with_entity};
use mod_api::EntityRef;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::commands_common::Target;
use sunrise_gameplay_common::game::native_entity_motion_values::corners;

impl crate::game::ClientGame<'_> {
    fn client_walk_entity(&self, id: u32) -> Option<EntityHandle> {
        if let Some(i) = self.common.mobs.iter().position(|e| e.id == id) {
            return Some(EntityHandle::Mob(i));
        }
        if let Some(i) = self.common.dropped.iter().position(|e| e.id == id) {
            return Some(EntityHandle::Item(i));
        }
        if let Some(i) = self.common.boats.iter().position(|e| e.id == id) {
            return Some(EntityHandle::Boat(i));
        }
        if let Some(i) = self.common.carts.iter().position(|e| e.id == id) {
            return Some(EntityHandle::Cart(i));
        }
        if let Some(i) = self.common.armor_stands.iter().position(|e| e.id == id) {
            return Some(EntityHandle::ArmorStand(i));
        }
        if let Some(i) = self.common.tnts.iter().position(|e| e.id == id) {
            return Some(EntityHandle::Tnt(i));
        }
        if let Some(i) = self.common.falling.iter().position(|e| e.id == id) {
            return Some(EntityHandle::Falling(i));
        }
        if let Some(i) = self.common.orbs.iter().position(|e| e.id == id) {
            return Some(EntityHandle::Orb(i));
        }
        if let Some(i) = self
            .common
            .creatures
            .projectiles
            .iter()
            .position(|e| e.id == id)
        {
            return Some(EntityHandle::Projectile(i));
        }
        if let Some(i) = self.common.rockets.iter().position(|e| e.id == id) {
            return Some(EntityHandle::Rocket(i));
        }
        if let Some(i) = self.common.tridents.iter().position(|e| e.id == id) {
            return Some(EntityHandle::Trident(i));
        }
        self.common
            .resident_entities
            .general
            .rows
            .iter()
            .position(|e| e.id == id)
            .map(EntityHandle::General)
    }

    pub(in crate::game) fn client_entity_velocity(&self, entity: EntityRef) -> Option<[f64; 3]> {
        let t = self.native_client_entity_target(entity)?;
        let v = match t {
            Target::Player => self.common.player.as_ref()?.velocity,
            _ => match self.client_walk_entity(entity.id()) {
                Some(h) => with_entity!(self, h, e => e.velocity()),
                None => Vec3::ZERO,
            },
        };
        Some([v.x, v.y, v.z])
    }

    pub(in crate::game) fn client_entity_bounding_box(
        &self,
        entity: EntityRef,
    ) -> Option<[[f64; 3]; 2]> {
        if let Some(pos) = self.side_client_display_position(entity) {
            return Some([pos; 2]);
        }
        if let Ok((remote, id)) = self.native_client_entity_context(entity)
            && let Some(row) = remote.general.rows.get(&id)
        {
            return Some(corners(row.bounds()));
        }
        let t = self.native_client_entity_target(entity)?;
        if t == Target::Player && self.client.remote.is_some() {
            return Some(corners(self.common.player.as_ref()?.bounds()));
        }
        if let Some(h) = self.client_walk_entity(entity.id()) {
            return Some(corners(with_entity!(self, h, e => {
                self.client_copy_box(e.id(), e.position(), e.bounds())
            })));
        }
        let (lo, hi) = match t {
            Target::Other(id) => {
                let i = self.client.interactions.get(&(id as i32))?;
                return Some(corners(i.bounds()));
            }
            Target::DragonPart(id) => {
                let dragon = self.common.dragon.as_ref()?;
                let part =
                    sunrise_gameplay_common::game::dragon_common::dragon_part_index(dragon.id, id)?;
                return Some(corners(dragon.part_geometry(part).1));
            }
            Target::Crystal(id) => {
                let c = self.common.crystals.iter().find(|c| c.id == id)?;
                return Some(corners(self.client_copy_box(id, c.position, c.bounds())));
            }
            Target::Cushion(id) => self.common.cushions.iter().find(|c| c.id == id)?.bounds(),
            Target::Frame(id) => self
                .common
                .item_frames
                .iter()
                .find(|f| f.id == id)?
                .bounds(),
            Target::Painting(id) => self.common.paintings.iter().find(|p| p.id == id)?.bounds(),
            _ => return None,
        };
        if lo.iter().chain(&hi).any(|v| !v.is_finite()) {
            return None;
        }
        Some([lo, hi])
    }

    pub(in crate::game) fn client_entity_eye_height(&self, entity: EntityRef) -> Option<f64> {
        if let Some(target) = self.native_client_entity_target(entity) {
            return Some(self.target_eye_height(target));
        }
        let (remote, id) = self.native_client_entity_context(entity).ok()?;
        remote.tracked(id)?;
        self.client
            .interactions
            .get(&id)
            .map(|i| f64::from(i.eye_height()))
    }
}

use sunrise_gameplay_common::game::side_ops::player::PlayerSideGameExt as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
