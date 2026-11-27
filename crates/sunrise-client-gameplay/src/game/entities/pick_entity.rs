//! Unified crosshair entity picking (`LocalPlayer.pick` and `ProjectileUtil.getEntityHitResult`).
#[path = "pick_sections.rs"]
pub(in crate::game) mod pick_sections;

use super::pick_ray::{PickAccumulator, PickCandidate, pick_ray_segment};
use crate::game::{ClientGame, pick_accumulator::PickDispatchExt as _};
use mod_api::{BlockPos, CrosshairTarget, EntityRef};
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_sim::save::uuid_text;

pub(in crate::game) use sunrise_gameplay_common::game::pick_entity_common::{
    AttackRangePickResult, CrosshairDecision, CrosshairEntityHit, aabb_extents, knot_bounds,
};

#[cfg(feature = "client")]
impl ClientGame<'_> {
    pub(in crate::game::client_side) fn client_has_pick_result(&self, id: i32) -> bool {
        let uid = id as u32;
        if self
            .common
            .creatures
            .projectiles
            .iter()
            .any(|p| p.id == uid)
            || self.common.tridents.iter().any(|t| t.id == uid)
            || self.common.rockets.iter().any(|r| r.id == uid)
            || self.common.tnts.iter().any(|t| t.id == uid)
            || self.common.falling.iter().any(|f| f.id == uid)
            || self.resident_dragon_part(uid).is_some()
            || self
                .client
                .remote
                .as_ref()
                .is_some_and(|r| r.general.rows.contains_key(&id))
            || self
                .common
                .resident_entities
                .general
                .rows
                .iter()
                .any(|r| r.id == uid)
        {
            return false;
        }
        true
    }

    pub(in crate::game) fn client_pick_knot_fence(&self, id: u32) -> Option<[i32; 3]> {
        self.client.remote_knots.get(&id).copied()
    }

    pub(in crate::game) fn client_entity_is_taken(&self, _id: u32) -> bool {
        false
    }

    pub(in crate::game) fn crosshair_entity_hit(&self) -> Option<CrosshairEntityHit> {
        if let Some(hit) = self.cached_entity_pick() {
            let (entity, distance, point) = hit?;
            return Some(CrosshairEntityHit {
                id: entity.id() as i32,
                distance,
                point,
                pickable: self.client.client_pick.entity_metadata.0,
                suggest_uuid: self.client.client_pick.entity_metadata.1,
            });
        }
        let hit = self.pick_entity_hit_unified()?;
        let is_synthetic_dragon_part = (sunrise_gameplay_common::game::dragon_common::SPECIAL
            ..sunrise_gameplay_common::game::dragon_common::SPECIAL + 8)
            .contains(&hit.id);
        let id = hit.id as i32;
        let pickable = !self.client.interactions.contains_key(&id)
            && !is_synthetic_dragon_part
            && self.client_has_pick_result(id);
        let suggest_uuid = !is_synthetic_dragon_part && self.resident_dragon_part(hit.id).is_none();
        Some(CrosshairEntityHit {
            id,
            distance: hit.distance_sq.sqrt(),
            point: hit.point,
            pickable,
            suggest_uuid,
        })
    }

    pub fn crosshair_block_target(&self, hit: super::Hit) -> Option<CrosshairTarget> {
        use sunrise_gameplay_common::game::api_values::TryIntoApi;
        Some(CrosshairTarget::Block {
            pos: BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]),
            state: self
                .common
                .world
                .block(hit.pos[0], hit.pos[1], hit.pos[2])
                .try_into_api()
                .ok()?,
            hit: hit.native(),
        })
    }

    /// The stored frame decision on a window; original during dispatch.
    pub fn crosshair_target(&self) -> Option<CrosshairTarget> {
        if !self.common.native_window() || self.client.title.0.is_some() {
            return None;
        }
        self.common.player.as_ref()?;
        if self.uses_stored_pick() {
            return self.stored_client_pick();
        }
        match self.compute_crosshair_decision() {
            CrosshairDecision::Entity(hit) => Some(CrosshairTarget::Entity {
                entity: EntityRef::new(self.common.native_lifecycle.epoch()?, hit.id),
                point: hit.point,
            }),
            CrosshairDecision::Block(hit) => self.crosshair_block_target(hit),
            CrosshairDecision::Miss => None,
        }
    }

    pub fn pick_entity_candidate_within(
        &mut self,
        max_distance: f64,
        max_distance_sq: f64,
    ) -> Option<CrosshairTarget> {
        let subscribed = mod_api::__private::has_scoped_hook(mod_api::HookId::CanRiderInteract);
        let mut pick = if subscribed {
            PickAccumulator::recording(
                max_distance_sq,
                std::mem::take(&mut self.client.client_pick.rider_candidates),
            )
        } else {
            PickAccumulator::new(max_distance_sq)
        };
        self.collect_pick_candidates_within(max_distance, &mut pick);
        if subscribed {
            self.storage_scope(|game, _| {
                game.client.client_pick.rider_candidates = pick.dispatch(game);
                ((), true)
            });
        }
        let hit = pick.hovered?;
        Some(CrosshairTarget::Entity {
            entity: EntityRef::new(self.common.native_lifecycle.epoch()?, hit.id),
            point: hit.point,
        })
    }

    /// Runs the unified `LocalPlayer.raycastHitResult` entity result for unstored consumers.
    pub(in crate::game::client_side) fn pick_entity_hit_unified(&self) -> Option<PickCandidate> {
        match self.compute_crosshair_decision() {
            CrosshairDecision::Entity(hit) => Some(hit),
            _ => None,
        }
    }

    /// Full unstored `LocalPlayer.raycastHitResult`: active `AttackRange` (keeping both entity and
    /// in-range block results), then `LocalPlayer.pick` with nominal miss search bound and strict
    /// `<` reach filters so an out-of-reach entity occluding a block resolves to `Miss`.
    pub(in crate::game) fn compute_crosshair_decision(&self) -> CrosshairDecision {
        if let Some(range) = self.common.active_attack_range() {
            match self.attack_range_pick_result(&range) {
                AttackRangePickResult::Entity(hit) => return CrosshairDecision::Entity(hit),
                AttackRangePickResult::Block(hit) => return CrosshairDecision::Block(hit),
                AttackRangePickResult::Miss => {}
            }
        }
        let Some((eye, dir)) = self.aim() else {
            return CrosshairDecision::Miss;
        };
        let from = Vec3::new(eye[0], eye[1], eye[2]);
        let max_distance = self.common.block_reach.max(self.common.entity_reach);
        let block_hit = if self.client.client_pick.ray.is_some() {
            None
        } else {
            self.pick_unfiltered()
        };
        let (entity_max_distance, search_max_distance_sq, block_distance_sq) = match block_hit {
            Some(block) => {
                let d2 = from.distance_squared(Vec3::new(
                    block.point[0],
                    block.point[1],
                    block.point[2],
                ));
                (d2.sqrt(), d2, d2)
            }
            None => (
                max_distance,
                max_distance * max_distance,
                pick_ray_segment(eye, dir, max_distance)
                    .1
                    .distance_squared(from),
            ),
        };
        let mut pick = PickAccumulator::new(search_max_distance_sq);
        self.collect_pick_candidates_within(entity_max_distance, &mut pick);
        if let Some(hit) = pick.hovered
            && hit.distance_sq < block_distance_sq
        {
            return if hit.distance_sq < self.common.entity_reach * self.common.entity_reach {
                CrosshairDecision::Entity(hit)
            } else {
                CrosshairDecision::Miss
            };
        }
        if let Some(block) = block_hit
            && from.distance_squared(Vec3::new(block.point[0], block.point[1], block.point[2]))
                < self.common.block_reach * self.common.block_reach
        {
            CrosshairDecision::Block(block)
        } else {
            CrosshairDecision::Miss
        }
    }

    /// Single pickable-entity query matching `LocalPlayer.pick` and `Level.getEntities`:
    /// sweeps the camera entity's current bounding box, visits accessible sections in
    /// `EntitySectionStorage` order (filtering by raw box), then appends dragon parts.
    pub(in crate::game::client_side) fn collect_pick_candidates_within(
        &self,
        max_distance: f64,
        pick: &mut PickAccumulator,
    ) {
        let Some((eye, dir)) = self.aim() else {
            return;
        };
        let (from, to) = pick_ray_segment(eye, dir, max_distance);
        let search_box = self
            .pick_camera_bounds()
            .unwrap_or_else(|| Aabb::new(from, from))
            .expand_towards(Vec3::new(
                dir[0] * max_distance,
                dir[1] * max_distance,
                dir[2] * max_distance,
            ))
            .inflate(Vec3::new(1.0, 1.0, 1.0));
        self.for_each_pickable_in_bounds(search_box, |id, raw_bb, radius| {
            let bb = if radius == 0.0 {
                raw_bb
            } else {
                raw_bb.inflate(Vec3::new(radius, radius, radius))
            };
            pick.consider_box(self, id, from, to, bb);
        });
    }

    /// Entity under the crosshair before any block (`Some(None)` when `getPickResult` is empty).
    pub(in crate::game::client_side) fn pick_entity_hit_at(&self) -> Option<(f64, Option<i32>)> {
        self.crosshair_entity_hit()
            .map(|hit| (hit.distance, hit.pickable.then_some(hit.id)))
    }
    pub fn pick_entity_hit(&self) -> Option<Option<i32>> {
        self.pick_entity_hit_at().map(|hit| hit.1)
    }
    /// Returns the wire entity id under the crosshair before any block.
    #[cfg(any(test, feature = "test-support"))]
    pub fn crosshair_entity_id(&self) -> Option<i32> {
        self.crosshair_entity_hit().map(|hit| hit.id)
    }

    /// `ClientSuggestionProvider.getSelectedEntities`: UUID of the crosshair entity hit.
    pub(in crate::game::client_side) fn crosshair_entity_uuid(&self) -> Option<String> {
        self.crosshair_entity_hit()
            .filter(|hit| hit.suggest_uuid)
            .and_then(|hit| self.common.persist.uuids.get(&(hit.id as u32)).copied())
            .map(uuid_text)
    }
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::dragon_common::DragonCommonGameExt1 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;

#[cfg(feature = "client")]
#[cfg(test)]
pub(in crate::game) mod tests {
    use sunrise_gameplay_common::game::commands_common::Target;

    #[test]
    fn tnt_and_falling_blocks_have_no_entity_pick_result() {
        use sunrise_gameplay_common::game::pick_entity_common::has_pick_result;
        assert!(!has_pick_result(Some(Target::Tnt(10))));
        assert!(!has_pick_result(Some(Target::Falling(20))));
        assert!(!has_pick_result(Some(Target::Projectile(30))));
        assert!(!has_pick_result(Some(Target::DragonPart(40))));
        assert!(has_pick_result(Some(Target::Boat(50))));
        assert!(has_pick_result(Some(Target::Mob(60))));
    }
}
