use super::{
    entity::Entity,
    pick_entity::pick_sections::PickMember,
    pick_entity_common::{PickKind, aabb_extents},
};
use core::ops::ControlFlow;
use mod_api::{EntityRef, LevelReadError, LevelRef};
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_sim::join_seq::residency::table::SectionSearch;

impl crate::game::ClientGame<'_> {
    /// `LevelRenderer.destroyBlockProgress` for this client's own view only.
    pub(in crate::game) fn level_set_client_break_progress(
        &mut self,
        level: LevelRef,
        breaker: i32,
        pos: [i32; 3],
        progress: i32,
    ) -> bool {
        if self.native_client_level() != Some(level) {
            return false;
        }
        self.destroy_block_progress(breaker, pos, progress);
        true
    }

    /// Client-side `Entity.getType`: `minecraft:zombie`.
    pub(in crate::game) fn client_entity_kind(
        &self,
        entity: EntityRef,
    ) -> Option<mod_api::EntityTypeKey> {
        match self.native_entity_read_target(entity) {
            Some(target) => self.native_entity_kind_key(target),
            None => {
                mod_api::__private::resolved_entity_type_key(self.native_client_tracked(entity)?.0)
            }
        }
    }

    /// The level the entity is in on the client (`Entity.level`).
    pub(in crate::game) fn client_entity_level(&self, entity: EntityRef) -> Option<LevelRef> {
        (self.native_client_entity_target(entity).is_some()
            || self.native_client_tracked(entity).is_some())
        .then(|| LevelRef::new(entity.epoch(), self.native_level_key()))
    }

    pub(in crate::game) fn client_item_entity(
        &self,
        _entity: EntityRef,
    ) -> Option<mod_api::ItemEntityRef> {
        None
    }

    /// Visit intersecting, readable entities in `ClientLevel.getEntities` query order.
    pub(in crate::game) fn client_level_visit_entities_in(
        &self,
        level: LevelRef,
        min: [f64; 3],
        max: [f64; 3],
        visit: &mut dyn FnMut(EntityRef) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, LevelReadError> {
        if !self.client_level_handle_current(level) {
            return Err(LevelReadError::Stale);
        }
        if !min.iter().chain(&max).all(|v| v.is_finite())
            || self.native_client_level() != Some(level)
        {
            return Err(LevelReadError::Unavailable);
        }
        let bounds = Aabb::new(
            Vec3::new(min[0], min[1], min[2]),
            Vec3::new(max[0], max[1], max[2]),
        );
        let search = SectionSearch::new(bounds);
        for member in self.client_section_candidates(search) {
            let entity = EntityRef::new(level.epoch(), member.id);
            if self
                .client_query_body(entity, member)
                .is_some_and(|body| bounds.intersects(body))
                && visit(entity).is_break()
            {
                return Ok(ControlFlow::Break(()));
            }
        }
        if let Some(dragon) = self.common.dragon.as_ref().filter(|d| d.id != 0) {
            let mut cursor = None;
            while let Some(id) = self.common.dragon.next_part(&mut cursor) {
                if let Some(part) =
                    sunrise_gameplay_common::game::dragon_common::dragon_part_index(dragon.id, id)
                    && bounds.intersects(dragon.part_geometry(part).1)
                {
                    let entity = EntityRef::new(level.epoch(), id);
                    if self.native_client_entity_target(entity).is_some()
                        && visit(entity).is_break()
                    {
                        return Ok(ControlFlow::Break(()));
                    }
                }
            }
        }
        Ok(ControlFlow::Continue(()))
    }

    /// Resolve a client section member's box without consulting server residency.
    pub(in crate::game::client_side) fn client_query_body(
        &self,
        entity: EntityRef,
        member: PickMember,
    ) -> Option<Aabb> {
        match member.kind {
            PickKind::RemoteDisplay => {
                let [x, y, z] = self.side_client_display_position(entity)?;
                let p = Vec3::new(x, y, z);
                Some(Aabb::new(p, p))
            }
            // Indexed only for picking; not readable box-query bodies.
            PickKind::Cushion
            | PickKind::Interaction
            | PickKind::RemoteGeneral
            | PickKind::RemoteKnot
            | PickKind::LocalKnot
            | PickKind::Crystal => None,
            PickKind::LocalPlayer if self.common.mode_id() != 3 => {
                self.common.player.as_ref().map(|p| p.bounds())
            }
            PickKind::LocalPlayer => None,
            _ => {
                let remote = self.client.remote.as_ref()?;
                remote.tracked(member.id as i32)?;
                let id = member.id;
                if let Some(dragon) = self.common.dragon.as_ref().filter(|d| d.id == id) {
                    return Some(dragon.body_bounds());
                }
                if let Some(frame) = self.common.item_frames.iter().find(|f| f.id == id) {
                    return Some(aabb_extents(frame.bounds()));
                }
                if let Some(painting) = self.common.paintings.iter().find(|p| p.id == id) {
                    return Some(aabb_extents(painting.bounds()));
                }
                fn list_bounds<T: Entity>(items: &[T], id: u32) -> Option<Aabb> {
                    items
                        .iter()
                        .find(|e| e.id() == id && !e.is_removed())
                        .map(Entity::bounds)
                }
                list_bounds(&self.common.mobs, id)
                    .or_else(|| list_bounds(&self.common.dropped, id))
                    .or_else(|| list_bounds(&self.common.orbs, id))
                    .or_else(|| list_bounds(&self.common.boats, id))
                    .or_else(|| list_bounds(&self.common.carts, id))
                    .or_else(|| list_bounds(&self.common.armor_stands, id))
                    .or_else(|| list_bounds(&self.common.tnts, id))
                    .or_else(|| list_bounds(&self.common.falling, id))
                    .or_else(|| list_bounds(&self.common.creatures.projectiles, id))
                    .or_else(|| list_bounds(&self.common.rockets, id))
                    .or_else(|| list_bounds(&self.common.tridents, id))
                    .or_else(|| list_bounds(&self.common.resident_entities.general.rows, id))
            }
        }
    }
}

use sunrise_gameplay_common::game::side_ops::player::PlayerSideGameExt as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt4 as _;
