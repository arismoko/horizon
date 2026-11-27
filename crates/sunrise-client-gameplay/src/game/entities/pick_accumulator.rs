//! The running state of `ProjectileUtil.getEntityHitResult`.
use crate::game::ClientGame;

pub(in crate::game) use sunrise_gameplay_common::game::pick_ray::{PickAccumulator, PickCandidate};

pub(in crate::game) trait PickDispatchExt: Sized {
    fn recording(max_distance_sq: f64, candidates: Vec<PickCandidate>) -> Self;
    fn dispatch(&mut self, game: &mut ClientGame<'_>) -> Vec<PickCandidate>;
}
impl PickDispatchExt for PickAccumulator {
    /// Reuses the caller's candidate buffer when `CanRiderInteract` is subscribed.
    fn recording(max_distance_sq: f64, mut candidates: Vec<PickCandidate>) -> Self {
        candidates.clear();
        Self::recording_with(max_distance_sq, candidates)
    }

    fn dispatch(&mut self, game: &mut crate::game::ClientGame<'_>) -> Vec<PickCandidate> {
        let candidates = self.take_candidates();
        let camera = game.pick_camera_id();
        for &candidate in &candidates {
            if game.is_pick_camera(candidate.id) {
                continue;
            }
            let same_vehicle = game.pick_shares_vehicle(candidate.id);
            let mut allowed = false;
            if !candidate.inside
                && (candidate.distance_sq < self.nearest_sq() || self.nearest_sq() == 0.0)
                && same_vehicle
                && let Some(epoch) = game.native_lifecycle.epoch()
                && let Some(wire_id) = game.wire_entity_id(candidate.id)
            {
                let entity = mod_api::EntityRef::new(epoch, wire_id as u32);
                game.ask_can_rider_interact(entity, &mut allowed);
                if game.pick_camera_id() != camera {
                    // Camera changed during hook; refuse the pick rather than recasting it.
                    self.hovered = None;
                    break;
                }
            }
            self.accept(candidate, same_vehicle, allowed);
        }
        candidates
    }
}

impl ClientGame<'_> {
    pub fn pick_camera_id(&self) -> Option<u32> {
        if let Some(remote) = self.client.remote.as_ref() {
            if let Some(id) = remote.camera {
                remote.detached_camera.is_none().then_some(id as u32)
            } else {
                Some(remote.entity_id as u32)
            }
        } else {
            self.common
                .player
                .as_ref()
                .map(|_| self.own_entity_id() as u32)
                .filter(|&id| id != 0)
        }
    }

    /// True if `id` (or the wire entity behind a synthetic special-target id) is the camera entity.
    pub(in crate::game::client_side) fn is_pick_camera(&self, id: u32) -> bool {
        let Some(camera) = self.pick_camera_id() else {
            return false;
        };
        id == camera || self.wire_entity_id(id) == Some(camera as i32)
    }

    #[inline(always)]
    pub(in crate::game) fn client_pick_camera_id(&self) -> Option<u32> {
        self.pick_camera_id()
    }

    #[inline(always)]
    pub(in crate::game) fn client_is_pick_camera(&self, id: u32) -> bool {
        self.is_pick_camera(id)
    }

    fn pick_vehicle_step(&self, entity: u32) -> Option<u32> {
        if self.pick_camera_id() == Some(entity) && self.remote_camera_id().is_none() {
            return self.common.riding.or_else(|| {
                self.client
                    .remote
                    .as_ref()
                    .and_then(|r| r.passenger_vehicle(entity))
            });
        }
        if let Some(remote) = &self.client.remote {
            return remote.passenger_vehicle(entity);
        }
        None
    }

    /// Walks the full vehicle chain (including retained off-level entities) with cycle detection.
    fn pick_vehicle_root(&self, id: u32) -> Option<u32> {
        let start = self.wire_entity_id(id).map_or(id, |wire| wire as u32);
        let mut slow = start;
        let mut fast = start;
        loop {
            let Some(next) = self.pick_vehicle_step(fast) else {
                return Some(fast);
            };
            let Some(next2) = self.pick_vehicle_step(next) else {
                return Some(next);
            };
            fast = next2;
            slow = self.pick_vehicle_step(slow)?;
            if slow == fast {
                return None;
            }
        }
    }

    pub(in crate::game::client_side) fn pick_shares_vehicle(&self, id: u32) -> bool {
        let camera_root = self
            .pick_camera_id()
            .and_then(|camera| self.pick_vehicle_root(camera))
            .or_else(|| {
                self.controls_camera()
                    .then(|| self.common.riding.and_then(|v| self.pick_vehicle_root(v)))
                    .flatten()
            });
        camera_root.is_some() && self.pick_vehicle_root(id) == camera_root
    }
}

use sunrise_gameplay_common::game::dragon_common::DragonCommonGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;

use crate::game::client_side::pick_ray_client::PickRecordingExt as _;
