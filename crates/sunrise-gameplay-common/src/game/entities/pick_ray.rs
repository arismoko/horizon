//! Shared raycast pick core and role-dispatched crosshair targeting wrappers.
use super::{Hit, SideGame};
use sunrise_core::geometry::{Aabb, Vec3};

#[derive(Clone, Copy, Debug)]
pub struct PickCandidate {
    pub id: u32,
    pub point: [f64; 3],
    pub distance_sq: f64,
    pub inside: bool,
}

/// Ray segment `(from, to)` matching `LocalPlayer.pick`.
pub fn pick_ray_segment(eye: [f64; 3], dir: [f64; 3], reach: f64) -> (Vec3, Vec3) {
    let from = Vec3::new(eye[0], eye[1], eye[2]);
    let to = Vec3::new(
        from.x + dir[0] * reach,
        from.y + dir[1] * reach,
        from.z + dir[2] * reach,
    );
    (from, to)
}

pub struct PickAccumulator {
    pub hovered: Option<PickCandidate>,
    pub nearest_sq: f64,
    pub candidates: Option<Vec<PickCandidate>>,
}

impl PickAccumulator {
    pub fn new(max_distance_sq: f64) -> Self {
        Self {
            hovered: None,
            nearest_sq: max_distance_sq,
            candidates: None,
        }
    }

    pub fn consider_candidate<'g, G: SideGame<'g>>(
        &mut self,
        game: &G,
        candidate: PickCandidate,
    ) -> bool {
        if let Some(candidates) = &mut self.candidates {
            candidates.push(candidate);
            return false;
        }
        self.accept(
            candidate,
            game.side_pick_shares_vehicle(candidate.id),
            false,
        )
    }

    pub fn consider_box<'g, G: SideGame<'g>>(
        &mut self,
        game: &G,
        id: u32,
        from: Vec3,
        to: Vec3,
        bb: Aabb,
    ) -> bool {
        let Some(candidate) = game.test_pick_box(id, from, to, bb) else {
            return false;
        };
        self.consider_candidate(game, candidate)
    }

    pub fn accept(&mut self, candidate: PickCandidate, same_vehicle: bool, allowed: bool) -> bool {
        if candidate.inside {
            if self.nearest_sq < 0.0 {
                return false;
            }
            self.nearest_sq = 0.0;
        } else if candidate.distance_sq < self.nearest_sq || self.nearest_sq == 0.0 {
            if same_vehicle && !allowed {
                if self.nearest_sq != 0.0 {
                    return false;
                }
            } else {
                self.nearest_sq = candidate.distance_sq;
            }
        } else {
            return false;
        }
        self.hovered = Some(candidate);
        true
    }
}

impl crate::game::CommonGame<'_> {
    /// `Entity.canBePickedFromInside`, false only for a sulfur cube holding a body item.
    pub(in crate::game) fn pickable_from_inside(&self, id: u32) -> bool {
        !self.mobs.iter().any(|m| {
            m.id == id
                && m.species() == sunrise_sim::mobs::Species::SulfurCube
                && sunrise_sim::mobs::sulfur_cube::has_body(m)
        })
    }
}

pub trait PickRayGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// Vanilla `ProjectileUtil.getEntityHitResult` per-box test (`AABB.contains` and `AABB.clip`).
    fn test_pick_box(&self, id: u32, from: Vec3, to: Vec3, bb: Aabb) -> Option<PickCandidate> {
        if self.side_is_pick_camera(id) {
            return None;
        }
        let clip_point = bb.clip(from, to);
        if bb.contains(from) {
            if !self.pickable_from_inside(id) {
                return None;
            }
            let at = clip_point.unwrap_or(from);
            Some(PickCandidate {
                id,
                point: [at.x, at.y, at.z],
                distance_sq: 0.0,
                inside: true,
            })
        } else {
            let at = clip_point?;
            Some(PickCandidate {
                id,
                point: [at.x, at.y, at.z],
                distance_sq: from.distance_squared(at),
                inside: false,
            })
        }
    }

    fn aim(&self) -> Option<([f64; 3], [f64; 3])> {
        self.side_aim()
    }

    fn pick(&self) -> Option<Hit> {
        self.side_pick()
    }

    fn pick_unfiltered(&self) -> Option<Hit> {
        self.side_pick_unfiltered()
    }

    fn mob_hit(&self) -> Option<(usize, f64, [f64; 3])> {
        self.side_mob_hit()
    }

    /// Nearer than the targeted block?
    fn before_block(&self, distance: f64) -> bool {
        self.side_block_distance()
            .is_none_or(|block| block > distance)
    }

    /// The frame under the crosshair within reach and its distance.
    fn frame_hit(&self) -> Option<(usize, f64)> {
        self.side_frame_hit()
    }

    fn boat_hit(&self) -> Option<(usize, f64)> {
        self.side_boat_hit()
    }

    fn cart_hit(&self) -> Option<(usize, f64)> {
        self.side_cart_hit()
    }

    /// The armor stand under the crosshair, its distance and the hit point.
    fn armor_stand_hit(&self) -> Option<(usize, f64, [f64; 3])> {
        self.side_armor_stand_hit()
    }

    fn painting_hit(&self) -> Option<(usize, f64)> {
        self.side_painting_hit()
    }

    fn dragon_hit(&self) -> Option<(u32, f64)> {
        self.side_dragon_hit()
    }

    #[cfg(any(test, feature = "test-support"))]
    fn set_test_forced_hit(&mut self, hit: Option<Hit>) {
        self.side_set_test_forced_hit(hit);
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> PickRayGameExt1<'__sg> for G {}

use crate::game::side_ops::player::PlayerSideGameExt as _;

use crate::game::side_ops::presentation::PresentationSideGameExt as _;
