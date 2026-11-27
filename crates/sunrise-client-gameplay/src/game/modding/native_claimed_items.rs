//! Main-hand input captured by an item behavior, and the client host's view raycast.
use super::Screen;
use mod_api::{
    __private::{client_item_use, mod_index_on},
    ClientHost, ClientItemUseBehavior, ItemStack,
    content::crash::{ModCall, enter_native_mod},
};
use std::sync::Arc;
use sunrise_gameplay_common::game::api_values::TryIntoApi;

/// Client input captured by a main-hand item behavior, not a living entity's item use.
pub struct ActiveClientItemUse {
    mod_id: &'static str,
    mod_index: usize,
    behavior: Arc<dyn ClientItemUseBehavior>,
    /// Last known live stack of `item_id` in `slot`, rebound on tick and release like
    /// `LivingEntity.useItem`.
    stack: ItemStack,
    item_id: Arc<str>,
    slot: usize,
    ticks: u32,
}

impl crate::game::ClientGame<'_> {
    /// The first block along the view within `distance`, in loaded chunks only;
    /// `None` on a miss or a nonpositive or non-finite `distance`.
    pub fn raycast(&self, distance: f32) -> Option<mod_api::Hit> {
        let (_, hit) = self.cast_view_ray(Some(f64::from(distance)))?;
        Some(hit)
    }
    /// The first block along the view across loaded chunks only, and its distance; `None` on a miss.
    pub fn view_hit(&self) -> Option<(f32, mod_api::Hit)> {
        self.cast_view_ray(None)
    }
    fn cast_view_ray(&self, max_distance: Option<f64>) -> Option<(f32, mod_api::Hit)> {
        use sunrise_core::geometry::{Axis, Vec3, traverse_blocks_window};
        use sunrise_gameplay_common::game::collider_rays::{ClipData, ColliderRays};
        use sunrise_level::collisions::SceneCollisions;
        use sunrise_world::{
            collision_context::{
                CollisionContext, CollisionEntity, CollisionItem, EntityContextKind,
            },
            entity_obstruction::EntityIdentity,
            world_ray::{RayBlockMode, RayWorld, clip_with_interaction},
        };
        let player = self.common.player.as_ref()?;
        let eye = self.eye_position()?;
        let dir = sunrise_gameplay_common::game::view_vector(self.common.yaw, self.common.pitch);
        let (t_start, t_end) = loaded_ray_segment(
            self.common.world.chunks.keys().copied(),
            self.common.world.bottom_y(),
            self.common.world.top_y(),
            eye,
            dir,
            max_distance,
        )?;
        let clip_from = Vec3::new(eye[0], eye[1], eye[2]);
        let direction = Vec3::new(dir[0], dir[1], dir[2]);
        let ray_at = |t| clip_from.add_vector(direction.scale(t));
        let clip_to = max_distance.map(ray_at);
        let axis = Axis::ALL.into_iter().fold(Axis::X, |best, next| {
            if direction.component(next).abs() > direction.component(best).abs() {
                next
            } else {
                best
            }
        });
        // The walk keeps the whole segment's nudge, stepping state and candidate order; the loaded
        // span only says where its visits begin and end.
        let total = max_distance.unwrap_or(t_end);
        let walk_to = ray_at(total);
        let window = (t_start / total, t_end / total);
        let tables =
            sunrise_gameplay_common::game::native_block_tables::BlockTables::of(&self.common);
        let mut rays = ColliderRays {
            scene: SceneCollisions {
                world: &self.common.world,
                binding: self.common.collision,
                subject: None,
            },
            context: CollisionContext::for_entity(
                CollisionEntity {
                    identity: EntityIdentity(0),
                    kind: EntityContextKind::Player,
                    y: player.position.y,
                    descending: false,
                    main_hand: Some(CollisionItem::EMPTY),
                },
                false,
            ),
            clip_data: Some(ClipData {
                facts: self.common.res.facts(),
                behaviours: self.common.res.behaviours(),
                tables: &tables,
                registry: self.common.pack.assets.registry,
            }),
            report_unloaded: false,
        };
        let (bottom_y, top_y) = (self.common.world.bottom_y(), self.common.world.top_y());
        let (state, hit) = traverse_blocks_window(clip_from, walk_to, window, usize::MAX, |p| {
            if !(bottom_y..top_y).contains(&p.y)
                || !self.common.world.chunks.contains_key(&(p.x >> 4, p.z >> 4))
            {
                return None;
            }
            let state = self.common.world.block(p.x, p.y, p.z);
            if self.common.res.facts().is_air(state) {
                return None;
            }
            match rays
                .block_shape(state, p, RayBlockMode::Outline)
                .and_then(|shape| {
                    let clip_to = if let Some(to) = clip_to {
                        to
                    } else {
                        let Some(bounds) = shape.bounds() else {
                            return Ok(None);
                        };
                        // VoxelShape.clip derives its inside probe from the segment length.
                        // With no caller endpoint, extend past the candidate's voxel and outline
                        // along the dominant axis: AABB.clip excludes the segment endpoint.
                        // This is a bound for the shape clip, not a cap on traversal.
                        let d = direction.component(axis);
                        let block = Vec3::from_block(p).component(axis);
                        let face = if d > 0.0 {
                            bounds.max(axis).max(1.0)
                        } else {
                            bounds.min(axis).min(0.0)
                        };
                        let axial_distance = (block + face - clip_from.component(axis)).abs();
                        ray_at((axial_distance + 1.0) / d.abs())
                    };
                    clip_with_interaction(&mut rays, clip_from, clip_to, p, &shape, state)
                }) {
                Ok(Some(hit)) => Some(Ok((state, hit))),
                Ok(None) => None,
                Err(error) => Some(Err(error)),
            }
        })
        .ok()??
        .ok()?;
        let distance = clip_from.distance_squared(hit.location).sqrt();
        Some((
            distance as f32,
            mod_api::Hit {
                block_x: hit.block.x,
                block_y: hit.block.y,
                block_z: hit.block.z,
                x: hit.location.x,
                y: hit.location.y,
                z: hit.location.z,
                block: mod_api::__private::block_key(&self.common.res.facts().get(state).name)?,
            },
        ))
    }

    fn can_capture_item_use(&self) -> bool {
        self.common.using.is_none()
            && !self.common.spectator()
            && self.client.screen != Screen::Dead
            && self
                .common
                .player
                .as_ref()
                .is_some_and(|p| p.health.health() > 0.0)
            && self
                .client
                .remote
                .as_ref()
                .is_some_and(|r| r.logged_in && r.session.connected())
    }

    /// Can the current client still hold this capture? Do not read the stack's components.
    fn client_item_use_valid(&self, active: &ActiveClientItemUse) -> bool {
        active.slot == self.common.selected
            && self.can_capture_item_use()
            && self
                .common
                .stack_in(active.slot)
                .is_some_and(|stack| stack.item == active.item_id && stack.count != 0)
    }

    /// Read the live hotbar stack at `active.slot` when it still holds the same item kind.
    fn live_client_item_use_stack(&self, active: &ActiveClientItemUse) -> Option<ItemStack> {
        let stack = self.common.stack_in(active.slot)?;
        if stack.item != active.item_id || stack.count == 0 {
            return None;
        }
        stack.try_into_api().ok()
    }

    /// Give the registered main-hand behavior this click instead of using a target or item.
    pub(in crate::game::client_side) fn start_client_item_use(&mut self) -> bool {
        let slot = self.common.selected;
        let Some(stack) = self.common.stack_in(slot) else {
            return false;
        };
        let Some((mod_index, mod_id, behavior)) = client_item_use(&stack.item) else {
            return false;
        };
        if !mod_index_on(mod_index)
            || !self.can_capture_item_use()
            || &*stack.item == "minecraft:air"
        {
            return false;
        }
        let Ok(initial) = stack.try_into_api() else {
            return false;
        };
        let item_id = Arc::clone(&stack.item);
        self.client.client_item_use.0 = Some(ActiveClientItemUse {
            mod_id,
            mod_index,
            behavior: Arc::clone(&behavior),
            stack: initial.clone(),
            item_id,
            slot,
            ticks: 0,
        });
        self.call_client_item_use(mod_id, "use_item", |host| {
            behavior.use_item(host, &initial, slot as u32);
        });
        true
    }

    /// Reset an invalid capture or rebind the live stack and count a held tick. Only input
    /// handling releases it.
    pub(in crate::game) fn tick_client_item_use(&mut self) {
        let Some(active) = self.client.client_item_use.0.as_ref() else {
            return;
        };
        let valid = mod_index_on(active.mod_index) && self.client_item_use_valid(active);
        let live = if valid {
            self.live_client_item_use_stack(active)
        } else {
            None
        };
        let held = self.mapped_key_down("key.use");
        if !valid {
            self.reset_client_item_use();
        } else if let Some(active) = self.client.client_item_use.0.as_mut() {
            if let Some(live) = live {
                active.stack = live;
            }
            if held {
                active.ticks = active.ticks.saturating_add(1);
            }
        }
    }

    pub fn reset_client_item_use(&mut self) {
        self.end_client_item_use(false);
    }

    /// Retire the capture before entering mod code. Only a valid input release calls
    /// `release_using`; every other end is a reset. Return whether a capture owned the input.
    pub(in crate::game::client_side) fn end_client_item_use(
        &mut self,
        release_requested: bool,
    ) -> bool {
        let Some(mut active) = self.client.client_item_use.0.take() else {
            return false;
        };
        if let Some(live) = self.live_client_item_use_stack(&active) {
            active.stack = live;
        }
        if release_requested
            && mod_index_on(active.mod_index)
            && self.client_item_use_valid(&active)
        {
            self.call_client_item_use(active.mod_id, "release_using", |host| {
                active.behavior.release_using(
                    host,
                    &active.stack,
                    active.slot as u32,
                    active.ticks,
                );
            });
        }
        if mod_index_on(active.mod_index) {
            self.call_client_item_use(active.mod_id, "on_stop_using", |host| {
                active.behavior.on_stop_using(
                    host,
                    &active.stack,
                    active.slot as u32,
                    active.ticks,
                );
            });
        }
        true
    }

    fn call_client_item_use(
        &mut self,
        mod_id: &'static str,
        phase: &'static str,
        invoke: impl FnOnce(&mut dyn ClientHost),
    ) {
        let _guard = enter_native_mod(ModCall {
            id: mod_id,
            hook: phase,
            part: Some("ClientItemUseBehavior"),
        });
        invoke(self);
    }
}

/// Intersect ray `eye + t * dir` (`t >= 0`, optionally bounded by `max_distance`)
/// with the padded bounding box of the loaded chunks.
/// Returns `None` when no chunks are loaded, inputs are non-finite, `max_distance`
/// is nonpositive or non-finite, or the ray misses the loaded extent.
fn loaded_ray_segment(
    chunks: impl IntoIterator<Item = (i32, i32)>,
    bottom_y: i32,
    top_y: i32,
    eye: [f64; 3],
    dir: [f64; 3],
    max_distance: Option<f64>,
) -> Option<(f64, f64)> {
    if !eye.iter().all(|v| v.is_finite()) || !dir.iter().all(|v| v.is_finite()) {
        return None;
    }
    let mut t_min = 0.0_f64;
    let mut t_max = match max_distance {
        Some(limit) if limit.is_finite() && limit > 0.0 => limit,
        Some(_) => return None,
        None => f64::INFINITY,
    };
    let mut iter = chunks.into_iter();
    let (first_cx, first_cz) = iter.next()?;
    let (mut min_cx, mut max_cx) = (first_cx, first_cx);
    let (mut min_cz, mut max_cz) = (first_cz, first_cz);
    for (cx, cz) in iter {
        min_cx = min_cx.min(cx);
        max_cx = max_cx.max(cx);
        min_cz = min_cz.min(cz);
        max_cz = max_cz.max(cz);
    }
    // Pad by one block on each side so a ray entering a boundary chunk's outer face
    // starts outside that block and steps cleanly across the face.
    let bounds = [
        (
            f64::from(min_cx) * 16.0 - 1.0,
            (f64::from(max_cx) + 1.0) * 16.0 + 1.0,
        ),
        (f64::from(bottom_y) - 1.0, f64::from(top_y) + 1.0),
        (
            f64::from(min_cz) * 16.0 - 1.0,
            (f64::from(max_cz) + 1.0) * 16.0 + 1.0,
        ),
    ];
    for axis in 0..3 {
        let (lo, hi) = bounds[axis];
        if dir[axis] == 0.0 {
            if eye[axis] < lo || eye[axis] > hi {
                return None;
            }
            continue;
        }
        let inv = 1.0 / dir[axis];
        let a = (lo - eye[axis]) * inv;
        let b = (hi - eye[axis]) * inv;
        let (near, far) = if a < b { (a, b) } else { (b, a) };
        t_min = t_min.max(near);
        t_max = t_max.min(far);
        if t_min > t_max {
            return None;
        }
    }
    if !t_min.is_finite() || !t_max.is_finite() || t_max <= t_min {
        return None;
    }
    Some((t_min, t_max))
}

#[cfg(test)]
mod ray_extent_tests {
    use super::loaded_ray_segment;

    #[test]
    fn loaded_ray_segment_spans_loaded_chunks_past_1024_and_rejects_invalid_limits() {
        let chunks = [(0, 0), (0, 100)];
        let eye = [0.5, 100.0, 0.5];
        let dir = [0.0, 0.0, 1.0];

        assert_eq!(
            loaded_ray_segment(chunks, -64, 320, eye, dir, None),
            Some((0.0, 1616.5))
        );
        assert_eq!(
            loaded_ray_segment(chunks, -64, 320, eye, dir, Some(2000.0)),
            Some((0.0, 1616.5))
        );
        assert_eq!(
            loaded_ray_segment(chunks, -64, 320, eye, dir, Some(1500.0)),
            Some((0.0, 1500.0))
        );
        // Entering loaded extent from far outside starts at the near face.
        assert_eq!(
            loaded_ray_segment([(0, 100)], -64, 320, eye, dir, None),
            Some((1598.5, 1616.5))
        );
        assert_eq!(
            loaded_ray_segment([(0, 100)], -64, 320, eye, dir, Some(1500.0)),
            None
        );
        // Empty chunk set, misses, and nonpositive or non-finite limits return None.
        assert_eq!(
            loaded_ray_segment(std::iter::empty(), -64, 320, eye, dir, None),
            None
        );
        assert_eq!(
            loaded_ray_segment(chunks, -64, 320, [40.5, 100.0, 0.5], [0.0, -1.0, 0.0], None),
            None
        );
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(
                loaded_ray_segment(chunks, -64, 320, eye, dir, Some(bad)),
                None
            );
        }
    }
}
