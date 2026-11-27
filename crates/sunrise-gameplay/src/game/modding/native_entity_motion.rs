//! An entity's motion and shape for native server mods, through an
//! `EntityRef` (docs/modding/src/how-to/native-mods.md, "Entity and level handles", #7387):
//! velocity, ground, box, eyes and view. They read the entity a handle names
//! as `native_entity_refs.rs` finds it, and return `None` (or `false`) for a
//! stale handle. On a client (API @API:client_entity_refs@) they read the
//! entities the server shows the window, and nothing moves one.
use super::commands::Target;
use super::entity::{Entity as _, EntityHandle, with_entity, with_entity_mut};
use mod_api::{ClipBlock, ClipFluid, ClipHit, CollisionContext, EntityRef, LevelWriteError};
use std::ops::ControlFlow;
use sunrise_core::geometry::Vec3;
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::game::native_entity_motion_values::{
    corners, on_block_position, turned_view_vector,
};
use sunrise_gameplay_common::game::native_entity_refs_common::ClipRay;
use sunrise_level::collisions::SceneCollisions;
use sunrise_sim::motion::player_context;

/// The end of a ray `distance` blocks from `from` along the unit vector `look`
/// (`from.add(look.scale(distance))`).
pub(in crate::game::server_side) fn ray_end(
    from: [f64; 3],
    look: [f64; 3],
    distance: f64,
) -> [f64; 3] {
    std::array::from_fn(|i| from[i] + look[i] * distance)
}

impl crate::game::ServerGame<'_> {
    /// The entity's velocity in blocks a tick (`Entity.getDeltaMovement`). A
    /// player's is the move its client last reported (`getKnownMovement`); an
    /// entity Sunrise doesn't simulate (a display, a painting) has none, so
    /// zero. On a client, another entity's is the motion the server last
    /// sent it (`Entity.lerpMotion`). API 379.
    pub(in crate::game) fn server_entity_velocity(&self, entity: EntityRef) -> Option<[f64; 3]> {
        let t = self.native_entity_target(entity)?;
        if let Some(g) = self.native_target_guest(t) {
            return Some(std::array::from_fn(|i| {
                g.position[i] - g.previous_position[i]
            }));
        }
        let v = match self.common.native_walk_entity(entity.id()) {
            Some(h) => with_entity!(self, h, e => e.velocity()),
            None => Vec3::ZERO,
        };
        Some([v.x, v.y, v.z])
    }
    /// `Entity.getKnownSpeed`, in blocks a tick: a player's reported move; a
    /// vehicle with a live controlling player, that player's; else what the
    /// entity moved in its last tick (`computeSpeed`: position minus the
    /// previous tick's). Every list the walk keeps has a previous position.
    /// A frame, painting, crystal, spawner, cloud, fangs or marker never
    /// moves, so zero; the dragon uses its own previous position.
    pub fn native_entity_known_speed(&self, entity: EntityRef) -> Option<Vec3> {
        let t = self.native_entity_target(entity)?;
        if let Some(g) = self.native_target_guest(t) {
            let [x, y, z] = g.known_movement;
            return Some(Vec3::new(x, y, z));
        }
        if matches!(t, Target::Player) {
            return Some(self.known_player_movement());
        }
        if let Some(rider) = self.native_controlling_player(t, entity) {
            return self.native_entity_known_speed(rider);
        }
        if let Target::Dragon(_) = t {
            let d = self.common.dragon.as_ref()?;
            return Some(d.position.subtract(d.previous));
        }
        Some(match self.common.native_walk_entity(entity.id()) {
            Some(h) => with_entity!(self, h, e => e.position().subtract(e.previous_position())),
            None => Vec3::ZERO,
        })
    }
    /// Whether `Entity.getControllingPassenger` can be a player: a boat's
    /// first passenger, or a mount's rider as `equine::controlled` decides
    /// (saddled; a pig or strider's rider holding the stick; a harnessed
    /// happy ghast), the rule the mount code steers by.
    pub fn native_steered(&self, t: Target) -> bool {
        match t {
            Target::Boat(_) => true,
            Target::Mob(id) => self
                .common
                .mobs
                .iter()
                .any(|m| m.id == id && sunrise_sim::mobs::equine::controlled(m)),
            _ => false,
        }
    }
    /// `Entity.getControllingPassenger` when it is a player.
    pub(in crate::game::server_side) fn native_controlling_player(
        &self,
        t: Target,
        entity: EntityRef,
    ) -> Option<EntityRef> {
        let steered = self.native_steered(t);
        let mut first = None;
        let _ = self
            .entity_visit_passengers(entity, &mut |rider| {
                first = Some(rider);
                ControlFlow::Break(())
            })
            .ok()?;
        let first = first?;
        let player = matches!(
            self.native_entity_target(first)?,
            Target::Player | Target::Guest(_)
        );
        (steered && player).then_some(first)
    }
    /// Replaces `Entity.deltaMovement`; a player also gets a motion packet.
    pub(in crate::game) fn entity_set_velocity(
        &mut self,
        entity: EntityRef,
        velocity: [f64; 3],
    ) -> bool {
        if !velocity.iter().all(|v| v.is_finite()) {
            return false;
        }
        let Some(target) = self.native_entity_target(entity) else {
            return false;
        };
        let [x, y, z] = velocity;
        let velocity = Vec3::new(x, y, z);
        if matches!(target, Target::Player | Target::Guest(_)) {
            return self.write_player_velocity(target, |_| Some(velocity));
        }
        let Some(h) = self.common.native_walk_entity(entity.id()) else {
            return false;
        };
        with_entity_mut!(self, h, e => *e.velocity_mut() = velocity);
        if let EntityHandle::Mob(i) = h {
            self.common.mobs[i].needs_sync = true;
        }
        true
    }
    /// `Entity.fallDistance` on the server body, including the player's accepted moves.
    pub(in crate::game) fn entity_fall_distance(&self, entity: EntityRef) -> Option<f64> {
        match self.native_entity_target(entity)? {
            Target::Player => Some(self.common.player.as_ref()?.fall_distance),
            Target::Guest(conn) => Some(self.command_guest(conn)?.motion.as_ref()?.fall_distance),
            _ => Some(self.common.native_walk_entity(entity.id()).map_or(
                0.0,
                |handle| with_entity!(self, handle, e => e.fall_distance()),
            )),
        }
    }
    /// `Entity.resetFallDistance`: how far the entity has fallen is forgotten, so its next
    /// landing hurts it as a step does (a fan's or a launcher's push, a mod's soft block).
    /// Players too, the acting one and guests. False for a stale handle or an entity not in
    /// the level in place; true for a kind that keeps no fall distance, which has none to forget.
    pub fn entity_reset_fall_distance(&mut self, entity: EntityRef) -> bool {
        let Some(target) = self.native_entity_target(entity) else {
            return false;
        };
        if matches!(target, Target::Player | Target::Guest(_)) {
            let motion = match target {
                Target::Player => self.common.player.as_mut(),
                Target::Guest(conn) => self
                    .server
                    .lan
                    .as_mut()
                    .and_then(|host| host.hub.guests.motion_mut(conn)),
                _ => None,
            };
            let Some(motion) = motion else {
                return false;
            };
            motion.fall_distance = 0.0;
            return true;
        }
        let Some(h) = self.common.native_walk_entity(entity.id()) else {
            return false;
        };
        with_entity_mut!(self, h, e => e.reset_fall_distance());
        true
    }
    /// `Entity.horizontalCollision` of a general row's last move.
    pub(in crate::game) fn entity_horizontal_collision(&self, entity: EntityRef) -> Option<bool> {
        let Target::General(id) = self.native_entity_target(entity)? else {
            return None;
        };
        Some(self.resident_entities.general.row(id)?.horizontal_collision)
    }
    /// `Entity.verticalCollision` of a general row's last move.
    pub(in crate::game) fn entity_vertical_collision(&self, entity: EntityRef) -> Option<bool> {
        let Target::General(id) = self.native_entity_target(entity)? else {
            return None;
        };
        Some(self.resident_entities.general.row(id)?.vertical_collision)
    }
    /// `Entity.setOnGround` on its live server body, with no movement or fall processing.
    pub(in crate::game) fn entity_set_on_ground(
        &mut self,
        entity: EntityRef,
        on_ground: bool,
    ) -> Result<bool, LevelWriteError> {
        if !self.common.is_logical_server() {
            return Err(LevelWriteError::Unavailable);
        }
        let Some(target) = self.native_entity_target(entity) else {
            return Ok(false);
        };
        match target {
            Target::Player | Target::Guest(_) => {
                self.set_server_player_ground(target, entity.id(), on_ground)
            }
            Target::Hook(id) => self.set_fishing_hook_ground(id, on_ground),
            // A general row keeps the flag itself, with no support block to refresh.
            Target::General(id) => {
                Ok(self
                    .resident_entities
                    .general
                    .row_mut(id)
                    .is_some_and(|row| {
                        row.on_ground = on_ground;
                        true
                    }))
            }
            _ => {
                let Some(handle) = self.common.native_walk_entity(entity.id()) else {
                    return Ok(false);
                };
                with_entity_mut!(self.common, handle, e => {
                    e.update_retained_ground(&self.common.world, self.common.collision, &server_world_items!(self).entity_type_tags, on_ground, None)
                        .map_err(|_| LevelWriteError::Unavailable)
                })
            }
        }
    }

    /// The acting player's record and motion must agree when the guest turn ends. A parked
    /// player's position and equipment belong to its hub row, not to the last simulated motion.
    pub(in crate::game::server_side) fn set_server_player_ground(
        &mut self,
        target: Target,
        entity_id: u32,
        on_ground: bool,
    ) -> Result<bool, LevelWriteError> {
        let Some(body) = self.native_player_body(target) else {
            return Ok(false);
        };
        let (world, position, bounds, descending, held, armor, conn) = match target {
            Target::Player => (
                &self.common.world,
                body.position,
                body.bounds(),
                // PlayerInput updates the guest row before the sneak callback and the body tick.
                self.server
                    .lan_acting
                    .and_then(|conn| self.command_guest(conn))
                    .map_or(body.shift_key_down, |guest| guest.sneaking),
                self.common
                    .inventory
                    .get(self.common.selected)
                    .and_then(Option::as_ref),
                &self.common.armor,
                self.server.lan_acting,
            ),
            Target::Guest(conn) => {
                let Some(guest) = self.command_guest(conn) else {
                    return Ok(false);
                };
                let world = if guest.dimension == self.common.dim.id() {
                    &self.common.world
                } else {
                    self.server
                        .levels
                        .0
                        .parked_world_of(&guest.dimension)
                        .ok_or(LevelWriteError::Unavailable)?
                };
                let [x, y, z] = guest.position;
                (
                    world,
                    Vec3::new(x, y, z),
                    guest.bounds(),
                    guest.sneaking,
                    guest.inventory.get(guest.selected).and_then(Option::as_ref),
                    &guest.armor,
                    Some(conn),
                )
            }
            _ => return Ok(false),
        };
        let mut subject = body.powder_snow_subject();
        subject.leather_boots = super::worn_boots::leather_boots(armor);
        let mut scene = SceneCollisions {
            world,
            binding: self.common.collision,
            subject: Some(subject),
        };
        let mut ground = body.ground;
        ground.on_ground = on_ground;
        let scan = scene.update_ground_support(
            &mut ground,
            position,
            bounds,
            None,
            player_context(
                entity_id,
                position.y,
                descending,
                held.map(|s| s.item.as_ref()),
            ),
        );
        let Some(body) = self.native_player_body_mut(target) else {
            return Ok(false);
        };
        body.ground = ground;
        if let Some((conn, host)) = conn.zip(self.server.lan.as_mut()) {
            host.hub
                .update_guest(conn, |guest| guest.on_ground = on_ground);
        }
        scan.map(|()| true)
            .map_err(|_| LevelWriteError::Unavailable)
    }

    /// The entity's box (`Entity.getBoundingBox`): its lowest and highest
    /// corner. Displays and markers have zero-sized physical boxes. A client display's
    /// corners coincide at its committed tick position, not its rendering box or packet target.
    /// Interactions use their saved dimensions. End crystals have their 2 x 2 box.
    /// None for the dragon; a dragon part's is its own. API 379.
    pub(in crate::game) fn server_entity_bounding_box(
        &self,
        entity: EntityRef,
    ) -> Option<[[f64; 3]; 2]> {
        let t = self.native_entity_target(entity)?;
        if let Some(g) = self.native_target_guest(t) {
            return Some(corners(g.bounds()));
        }
        if let Some(h) = self.common.native_walk_entity(entity.id()) {
            return Some(corners(with_entity!(self, h, e => e.bounds())));
        }
        let (lo, hi) = match t {
            Target::Other(id) => {
                return self
                    .server
                    .others
                    .0
                    .iter()
                    .find(|e| e.id == id)?
                    .display_marker_or_interaction_bounds()
                    .map(corners);
            }
            Target::DragonPart(id) => {
                let (dragon, part) = self.resident_dragon_part(id)?;
                return Some(corners(dragon.part_geometry(part).1));
            }
            Target::Crystal(id) => {
                let c = self.common.crystals.iter().find(|c| c.id == id)?;
                return Some(corners(c.bounds()));
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

    pub(in crate::game) fn server_entity_eye_height(&self, entity: EntityRef) -> Option<f64> {
        let target = self.native_entity_target(entity)?;
        Some(self.server_target_eye_height(target))
    }
    /// The unit vector the entity looks along (`Entity.getViewVector`, from
    /// its pitch and yaw). API 379.
    pub fn entity_view_vector(&self, entity: EntityRef) -> Option<[f64; 3]> {
        let [yaw, pitch] = self.entity_rotation(entity)?;
        Some(turned_view_vector(yaw, pitch, 0.0))
    }
    /// `Entity.mainSupportingBlockPos`, retained by bodies with a shared ground state.
    /// Families that have not moved onto that state still use their existing support read.
    pub(in crate::game::server_side) fn native_supporting_block(
        &self,
        entity: EntityRef,
        t: Target,
    ) -> Option<BlockPos> {
        match t {
            Target::Player => return self.common.player.as_ref()?.ground.supporting_block,
            Target::Guest(_) => {
                return self
                    .native_target_guest(t)?
                    .motion
                    .as_ref()?
                    .ground
                    .supporting_block;
            }
            Target::Hook(id) => return self.fishing_hook_by_id(id)?.0.ground.supporting_block,
            _ => {}
        }
        let handle = self.common.native_walk_entity(entity.id())?;
        if let Some(ground) = with_entity!(self, handle, e => e.ground_state()) {
            return ground.supporting_block;
        }
        if !self.entity_on_ground(entity)? {
            return None;
        }
        let (position, bounds) = with_entity!(self, handle, e => (e.position(), e.bounds()));
        self.supporting_block(position, bounds)
    }
    /// The block the entity stands on (`Entity.getOnPos`). API 379.
    pub fn entity_on_block(&self, entity: EntityRef) -> Option<[i32; 3]> {
        let t = self.native_entity_target(entity)?;
        let [x, y, z] = self.entity_position(entity)?;
        Some(on_block_position(
            self.native_supporting_block(entity, t),
            Vec3::new(x, y, z),
        ))
    }

    fn entity_pick_view_vector(&self, entity: EntityRef) -> Option<[f64; 3]> {
        let target = self.native_entity_read_target(entity)?;
        let [mut yaw, pitch] = self.target_rotation(target);
        // LivingEntity.getViewYRot(1.0F) is head yaw, not body yaw.
        match target {
            Target::Mob(id) => {
                yaw = self.mobs.iter().find(|m| m.id == id)?.head_yaw;
            }
            Target::Player | Target::Guest(_) => {
                if let Some(guest) = self.native_target_guest(target) {
                    yaw = guest.head_yaw;
                }
            }
            _ => {}
        }
        Some(turned_view_vector(yaw, pitch, 0.0))
    }
    /// What the entity looks at within `distance` blocks of its eyes (`Entity.pick`):
    /// block outlines with `ClipFluid::Any` when `include_fluids` is true else `None`.
    pub(in crate::game) fn entity_pick(
        &self,
        entity: EntityRef,
        distance: f64,
        include_fluids: bool,
    ) -> Option<ClipHit> {
        if !distance.is_finite() {
            return None;
        }
        let from = self.entity_eye_position(entity)?;
        let to = ray_end(from, self.entity_pick_view_vector(entity)?, distance);
        let fluids = if include_fluids {
            ClipFluid::Any
        } else {
            ClipFluid::None
        };
        match self.level_clip_ray(
            self.entity_level(entity)?,
            from,
            to,
            ClipBlock::Outline,
            fluids,
            CollisionContext::Empty,
        ) {
            Ok(ClipRay::Reached(hit)) => Some(hit),
            Ok(ClipRay::Unloaded(_)) | Err(_) => None,
        }
    }
    /// The square of the distance from the entity's feet to `pos`
    /// (`Entity.distanceToSqr`). API 379.
    pub(in crate::game) fn entity_distance_to_sqr(
        &self,
        entity: EntityRef,
        pos: [f64; 3],
    ) -> Option<f64> {
        let p = self.entity_position(entity)?;
        let [dx, dy, dz] = std::array::from_fn(|i| p[i] - pos[i]);
        Some(dx * dx + dy * dy + dz * dz)
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::dragon_common::DragonCommonGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_motion_values::NativeEntityMotionValuesGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;

#[cfg(test)]
pub(in crate::game) mod tests {
    use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
    use sunrise_gameplay_common::game::native_entity_motion_values::NativeEntityMotionValuesGameExt1 as _;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
    use sunrise_gameplay_common::game::server_side::native_entity_motion::tests::assert_near;

    use mod_api::EntityRef;
    use sunrise_core::geometry::{Aabb, Vec3};
    use sunrise_gameplay_common::game::entity::Entity as _;

    /// Unit vectors worked out by hand from `Entity.calculateViewVector`
    /// (`x = -sin(yaw) cos(pitch)`, `y = -sin(pitch)`, `z = cos(yaw) cos(pitch)`),
    /// the look turned 90 degrees up for the up vector.
    #[test]
    fn the_up_vector_tilts_with_the_look() {
        let half = std::f64::consts::FRAC_1_SQRT_2;
        // Level and facing south: straight up.
        assert_near(super::turned_view_vector(0.0, 0.0, -90.0), [0.0, 1.0, 0.0]);
        // Looking down 45 degrees, the top of the head leans forward (south).
        assert_near(
            super::turned_view_vector(0.0, 45.0, -90.0),
            [0.0, half, half],
        );
        // The same look facing west (yaw 90): forward is -x.
        assert_near(
            super::turned_view_vector(90.0, 45.0, -90.0),
            [-half, half, 0.0],
        );
        // Looking straight up, the head's top points back (north when facing south).
        assert_near(
            super::turned_view_vector(0.0, -90.0, -90.0),
            [0.0, 0.0, -1.0],
        );
        // No turn is the view vector itself: south, and down 30 degrees.
        assert_near(super::turned_view_vector(0.0, 0.0, 0.0), [0.0, 0.0, 1.0]);
        assert_near(
            super::turned_view_vector(0.0, 30.0, 0.0),
            [0.0, -0.5, 3.0_f64.sqrt() / 2.0],
        );
    }

    #[test]
    fn on_block_is_the_support_else_the_cell_just_under_the_feet() {
        use sunrise_core::position::BlockPos;
        let feet = Vec3::new(0.9, 65.0, -0.2);
        // A support wins even where it is not under the feet's own column.
        assert_eq!(
            super::on_block_position(Some(BlockPos::new(1, 64, -1)), feet),
            [1, 64, -1]
        );
        // None: the cell under the feet; a hair under, so the block of a flush stand.
        assert_eq!(super::on_block_position(None, feet), [0, 64, -1]);
        assert_eq!(
            super::on_block_position(None, Vec3::new(0.5, 65.5, 0.5)),
            [0, 65, 0]
        );
    }

    #[test]
    fn a_pick_ray_ends_a_distance_along_the_look() {
        assert_eq!(
            super::ray_end([1.0, 2.0, 3.0], [0.0, -1.0, 0.0], 4.5),
            [1.0, -2.5, 3.0]
        );
        assert_eq!(
            super::ray_end([1.0, 2.0, 3.0], [0.6, 0.0, 0.8], 0.0),
            [1.0, 2.0, 3.0]
        );
        assert_eq!(
            super::ray_end([1.0, 2.0, 3.0], [0.0, 0.0, 1.0], -2.5),
            [1.0, 2.0, 0.5]
        );
    }

    #[test]
    #[ignore = "daily"]
    fn on_block_picks_the_support_nearest_the_entity_and_pick_clips_blocks() {
        use crate::game::native_entity_refs::tests::with_game;
        use mod_api::BlockFace;
        use std::sync::Arc;
        use sunrise_level::chunk::Chunk;
        use sunrise_sim::mobs::Mob;
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            // A zombie 0.6 wide straddling x = 1: its box is over blocks 0 and 1.
            game.mobs
                .push(Mob::new(40, kind, Vec3::new(0.9, 65.0, 0.5), 0.0));
            let entity = game.entity(40).unwrap();
            // Only block 1 is under the box: it supports, not the block of the feet's own column.
            game.world.set(1, 64, 0, stone).unwrap();
            let _ = game.common.mobs[0].update_retained_ground(
                &game.common.world,
                game.common.collision,
                &world_items!(game).entity_type_tags,
                true,
                None,
            );
            assert_eq!(game.entity_on_block(entity), Some([1, 64, 0]));
            // Adding a nearer block doesn't change the support until the next check.
            game.world.set(0, 64, 0, stone).unwrap();
            assert_eq!(game.entity_on_block(entity), Some([1, 64, 0]));
            let _ = game.common.mobs[0].update_retained_ground(
                &game.common.world,
                game.common.collision,
                &world_items!(game).entity_type_tags,
                true,
                None,
            );
            assert_eq!(game.entity_on_block(entity), Some([0, 64, 0]));
            game.mobs[0].position = Vec3::new(1.1, 65.0, 0.5);
            let _ = game.common.mobs[0].update_retained_ground(
                &game.common.world,
                game.common.collision,
                &world_items!(game).entity_type_tags,
                true,
                None,
            );
            assert_eq!(game.entity_on_block(entity), Some([1, 64, 0]));
            // Off the ground there is no support: the cell under the feet.
            game.mobs[0].position = Vec3::new(0.9, 65.0, 0.5);
            let _ = game.common.mobs[0].update_retained_ground(
                &game.common.world,
                game.common.collision,
                &world_items!(game).entity_type_tags,
                false,
                None,
            );
            assert_eq!(game.entity_on_block(entity), Some([0, 64, 0]));
            let air = game
                .pack
                .assets
                .registry
                .default_state("minecraft:air")
                .unwrap();
            game.world.set(0, 64, 0, air).unwrap();
            let _ = game.common.mobs[0].update_retained_ground(
                &game.common.world,
                game.common.collision,
                &world_items!(game).entity_type_tags,
                true,
                None,
            );
            assert_eq!(game.entity_on_block(entity), Some([1, 64, 0]));

            // Facing south (yaw 0) level, the eyes see a block two cells ahead.
            game.mobs[0].position = Vec3::new(0.5, 65.0, 0.5);
            let eyes = game.entity_eye_position(entity).unwrap();
            let wall = [0, eyes[1].floor() as i32, 2];
            game.world.set(wall[0], wall[1], wall[2], stone).unwrap();
            let hit = game.entity_pick(entity, 3.0, false).unwrap();
            assert!(!hit.miss);
            assert_eq!(hit.block, wall);
            assert_eq!(hit.face, BlockFace::North);
            assert_near(hit.location, [0.5, eyes[1], 2.0]);
            // Too short a ray misses, as does one over an empty column.
            assert!(game.entity_pick(entity, 1.0, false).unwrap().miss);
            game.mobs[0].position = Vec3::new(5.5, 65.0, 0.5);
            assert!(game.entity_pick(entity, 3.0, false).unwrap().miss);
            // A ray into a column that is not loaded has no answer, neither a miss nor a hit.
            game.mobs[0].position = Vec3::new(5.5, 65.0, 14.5);
            assert!(game.entity_pick(entity, 3.0, false).is_none());
            game.mobs[0].position = Vec3::new(5.5, 65.0, 0.5);
            // Looking straight down from the stand hits the top of its support.
            game.mobs[0].position = Vec3::new(1.5, 65.0, 0.5);
            game.mobs[0].head_pitch = 90.0;
            let down = game.entity_pick(entity, 3.0, false).unwrap();
            assert_eq!((down.block, down.face), ([1, 64, 0], BlockFace::Up));
            assert_near(down.location, [1.5, 65.0, 0.5]);
            // Looking down 45 degrees tilts the up vector forward.
            game.mobs[0].head_pitch = 45.0;
            assert_near(
                game.entity_up_vector(entity).unwrap(),
                [
                    0.0,
                    std::f64::consts::FRAC_1_SQRT_2,
                    std::f64::consts::FRAC_1_SQRT_2,
                ],
            );
            // Non-finite distances and stale handles return None.
            assert!(game.entity_pick(entity, f64::NAN, false).is_none());
            assert!(game.entity_pick(entity, f64::INFINITY, true).is_none());
            let stale = EntityRef::new(mod_api::WorldEpoch::new(entity.epoch().get() + 1), 40);
            assert!(game.entity_pick(stale, 3.0, false).is_none());
            assert!(game.entity_pick(stale, 3.0, true).is_none());
            assert!(game.entity_on_block(stale).is_none());
            assert!(game.entity_up_vector(stale).is_none());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn entity_pick_clips_fluids_and_signed_distances() {
        use crate::game::native_entity_refs::tests::with_game;
        use mod_api::BlockFace;
        use std::sync::Arc;
        use sunrise_gameplay_common::game::commands_common::OtherEntity;
        use sunrise_level::chunk::Chunk;
        use sunrise_sim::mobs::Mob;
        with_game(|game| {
            // Fluid clips read the tag tables once the world has built them (`native_tag_items`).
            let root = tempfile::tempdir().unwrap();
            game.server.session_lock.0 =
                Some(sunrise_sim::save::SessionLock::acquire(root.path()).unwrap());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let stone = game.block_state_id("minecraft:stone").unwrap();
            let flowing = game.block_state_id("minecraft:water[level=1]").unwrap();
            let shallow = game.block_state_id("minecraft:water[level=5]").unwrap();
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            let mut mob = Mob::new(41, kind.clone(), Vec3::new(4.5, 65.0, 4.5), 90.0);
            mob.head_yaw = 0.0;
            mob.head_pitch = 0.0;
            game.mobs.push(mob);
            let entity = game.entity(41).unwrap();
            let eye_y = game.entity_eye_position(entity).unwrap()[1];
            let y = eye_y.floor() as i32;
            // Decoy along body yaw (west) must not intercept a head-yaw (south) pick.
            game.world.set(2, y, 4, stone).unwrap();
            // Zombie eyes sit at +0.74 inside cell y: above shallow water (3/9) and below level=1 (7/9).
            game.world.set(4, y, 5, shallow).unwrap();
            game.world.set(4, y, 6, flowing).unwrap();
            game.world.set(4, y, 7, stone).unwrap();
            game.world.set(4, y, 2, stone).unwrap();

            let block_only = entity.pick(game, 5.0, false).unwrap();
            assert!(!block_only.miss);
            assert_eq!(
                (block_only.block, block_only.face),
                ([4, y, 7], BlockFace::North)
            );

            let with_fluids = entity.pick(game, 5.0, true).unwrap();
            assert!(!with_fluids.miss);
            assert_eq!(
                (with_fluids.block, with_fluids.face),
                ([4, y, 6], BlockFace::North)
            );

            // Signed negative distance casts backward along -head view vector.
            let backward = entity.pick(game, -3.0, false).unwrap();
            assert!(!backward.miss);
            assert_eq!(
                (backward.block, backward.face),
                ([4, y, 2], BlockFace::South)
            );

            // GoalCx::r_entity_pick follows head_yaw rather than body yaw too.
            let mut goal_mob = Mob::new(42, kind, Vec3::new(4.5, 65.0, 4.5), 90.0);
            goal_mob.head_yaw = 0.0;
            goal_mob.head_pitch = 0.0;
            let goal_entity = EntityRef::new(entity.epoch(), 42);
            let goal_block = super::super::native_goal_ticks::test_goal_pick(
                game,
                &mut goal_mob,
                goal_entity,
                5.0,
                false,
            )
            .unwrap();
            assert_eq!(
                (goal_block.block, goal_block.face),
                ([4, y, 7], BlockFace::North)
            );
            let goal_fluid = super::super::native_goal_ticks::test_goal_pick(
                game,
                &mut goal_mob,
                goal_entity,
                5.0,
                true,
            )
            .unwrap();
            assert_eq!(
                (goal_fluid.block, goal_fluid.face),
                ([4, y, 6], BlockFace::North)
            );

            // Non-living entities keep entity yaw (west -> decoy stone at x=2).
            game.server.others.0.push(OtherEntity {
                size_answer: None,
                attachment_lifetime: Default::default(),
                knot_check: None,
                eye: None,
                join: Default::default(),
                display: None,
                interaction: None,
                id: 43,
                kind: "minecraft:marker".into(),
                position: Vec3::new(4.5, eye_y, 4.5),
                yaw: 90.0,
                pitch: 0.0,
            });
            let marker = game.entity(43).unwrap();
            let marker_hit = marker.pick(game, 5.0, false).unwrap();
            assert!(!marker_hit.miss);
            assert_eq!(
                (marker_hit.block, marker_hit.face),
                ([2, y, 4], BlockFace::East)
            );

            assert!(entity.pick(game, f64::NAN, true).is_none());
            assert!(entity.pick(game, f64::NEG_INFINITY, false).is_none());
            let stale = EntityRef::new(mod_api::WorldEpoch::new(entity.epoch().get() + 1), 41);
            assert!(stale.pick(game, 5.0, false).is_none());
            assert!(stale.pick(game, 5.0, true).is_none());
            game.server.session_lock.0 = None;
        });
    }

    #[test]
    fn corners_are_lowest_then_highest() {
        let b = Aabb::new(Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0));
        assert_eq!(super::corners(b), [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]]);
    }

    #[test]
    #[ignore = "daily"]
    fn set_on_ground_refreshes_the_live_footprint_without_moving_the_body() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
        use std::sync::Arc;
        use sunrise_core::position::BlockPos;
        use sunrise_items::items::ItemStack;
        use sunrise_level::chunk::Chunk;
        use sunrise_sim::{
            entities::{ArmorStand, ItemEntity},
            projectiles::{Projectile, ProjectileKind},
        };

        if run_fresh_cases(
            "game::server_side::native_entity_motion::tests::set_on_ground_refreshes_the_live_footprint_without_moving_the_body",
            "SUNRISE_GROUND_SETTER_CASE",
            &["support"],
        ) {
            return;
        }
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let snow = registry.default_state("minecraft:powder_snow").unwrap();
            let air = registry.default_state("minecraft:air").unwrap();
            let supported = [1, 64, 0];
            let under_feet = [0, 64, 0];
            game.world.set(1, 64, 0, snow).unwrap();
            // The feet are over air. Only the edge of this stand is over the snow.
            let position = Vec3::new(0.9, 65.0, 0.5);
            let velocity = Vec3::new(0.25, -0.125, 0.5);
            let mut stand = ArmorStand::new(40, position, 25.0);
            stand.velocity = velocity;
            stand.enchantment_fall_distance = 1.5;
            stand.ground.horizontal_collision = true;
            game.armor_stands.push(stand);
            let entity = game.entity(40).unwrap();
            assert_eq!(entity.set_on_ground(game, true), Ok(true));
            assert_eq!(entity.on_ground(game), Some(true));
            assert_eq!(entity.on_block(game), Some(under_feet));
            assert_eq!(entity.set_on_ground(game, false), Ok(true));
            game.armor_stands[0].equipment[2] = Some(ItemStack::new("minecraft:leather_boots", 1));
            assert_eq!(entity.set_on_ground(game, true), Ok(true));
            assert_eq!(entity.on_block(game), Some(supported));
            game.world.set(1, 64, 0, air).unwrap();
            assert_eq!(entity.set_on_ground(game, true), Ok(true));
            assert_eq!(entity.on_block(game), Some(supported));
            assert_eq!(entity.set_on_ground(game, true), Ok(true));
            assert_eq!(entity.on_block(game), Some(under_feet));
            assert_eq!(entity.on_ground(game), Some(true));
            game.world.set(1, 64, 0, snow).unwrap();
            assert_eq!(entity.set_on_ground(game, true), Ok(true));
            assert_eq!(entity.on_block(game), Some(supported));
            assert_eq!(entity.set_on_ground(game, false), Ok(true));
            assert_eq!(entity.on_block(game), Some(under_feet));
            let stand = &game.armor_stands[0];
            assert_eq!(stand.ground.supporting_block, None::<BlockPos>);
            assert!(!stand.ground.on_ground_no_blocks);
            assert!(stand.ground.horizontal_collision);
            assert_eq!(stand.enchantment_fall_distance, 1.5);
            assert_eq!(
                (stand.position, stand.velocity, stand.yaw),
                (position, velocity, 25.0)
            );
            let stale = EntityRef::new(mod_api::WorldEpoch::new(entity.epoch().get() + 1), 40);
            assert_eq!(stale.set_on_ground(game, true), Ok(false));
            assert_eq!(entity.on_ground(game), Some(false));

            // A scan with invalid bounds fails, but vanilla sets the flag before the query.
            game.armor_stands[0].position.y = f64::INFINITY;
            assert_eq!(
                entity.set_on_ground(game, true),
                Err(mod_api::LevelWriteError::Unavailable)
            );
            assert_eq!(entity.on_ground(game), Some(true));

            // A fall over 2.5 blocks selects PowderSnowBlock's 0.9f-high shape.
            // Only the edge of either body overlaps the snow.
            let feet = Vec3::new(0.95, 64.0 + f64::from(0.9_f32), 0.5);
            let mut shot = Projectile::new(
                ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                feet,
                None,
            );
            shot.id = 41;
            shot.arrow.as_mut().unwrap().fall_distance = 3.0;
            game.creatures.projectiles.push(shot);
            let projectile = game.entity(41).unwrap();
            assert_eq!(projectile.on_ground(game), Some(false));
            assert_eq!(projectile.set_on_ground(game, true), Ok(true));
            assert_eq!(projectile.on_ground(game), Some(true));
            assert_eq!(projectile.on_block(game), Some(supported));
            assert_eq!(projectile.set_on_ground(game, false), Ok(true));
            assert_eq!(projectile.on_ground(game), Some(false));
            assert_eq!(projectile.on_block(game), Some(under_feet));

            let mut drop = ItemEntity::new(
                42,
                ItemStack::new("minecraft:stone", 1),
                feet,
                Vec3::ZERO,
                0.0,
            );
            drop.fall_distance = 3.0;
            game.dropped.push(drop);
            let item = game.entity(42).unwrap();
            assert_eq!(item.set_on_ground(game, true), Ok(true));
            assert_eq!(item.on_block(game), Some(supported));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn set_on_ground_follows_the_server_players_parked_and_acting_bodies() {
        use crate::game::native_entity_refs::tests::run_fresh_cases;
        use crate::game::native_guest_turn_tests::with_guests;
        use std::sync::Arc;
        use sunrise_gameplay_common::game::Dim;
        use sunrise_level::chunk::Chunk;

        if run_fresh_cases(
            "game::server_side::native_entity_motion::tests::set_on_ground_follows_the_server_players_parked_and_acting_bodies",
            "SUNRISE_GROUND_SETTER_PLAYER_CASE",
            &["players"],
        ) {
            return;
        }
        with_guests(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let scaffolding = registry.default_state("minecraft:scaffolding").unwrap();
            let entity = game.entity(12).unwrap();
            assert_eq!(entity.set_on_ground(game, true), Ok(false), "no body yet");
            game.as_guest(12, |_| {}).unwrap();
            // The hub has the latest position; the parked motion still sits elsewhere.
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| guest.position = [0.9, 65.0, 0.5])
                .unwrap();
            game.world.set(1, 64, 0, stone).unwrap();
            game.enter_level(Dim::Nether).unwrap();
            // This level has no support at the guest's coordinates. The parked
            // Overworld under their footprint does.
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            assert_eq!(entity.set_on_ground(game, true), Ok(true));
            assert_eq!(entity.on_ground(game), Some(true));
            assert_eq!(entity.on_block(game), Some([1, 64, 0]));
            game.enter_level(Dim::Overworld).unwrap();
            let hook = game
                .as_guest(12, |game| {
                    assert_eq!(entity.on_ground(game), Some(true));
                    assert_eq!(entity.on_block(game), Some([1, 64, 0]));
                    assert_eq!(entity.set_on_ground(game, false), Ok(true));
                    assert_eq!(entity.on_ground(game), Some(false));
                    // The sneak callback runs after assigning the hub flag, before the body tick.
                    game.world.set(1, 64, 0, scaffolding).unwrap();
                    assert!(!game.player.as_ref().unwrap().shift_key_down);
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .update_guest(12, |guest| guest.sneaking = true)
                        .unwrap();
                    assert_eq!(entity.set_on_ground(game, true), Ok(true));
                    assert_eq!(entity.on_block(game), Some([0, 64, 0]));
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .update_guest(12, |guest| guest.sneaking = false)
                        .unwrap();
                    assert_eq!(entity.set_on_ground(game, true), Ok(true));
                    assert_eq!(entity.on_block(game), Some([1, 64, 0]));
                    assert_eq!(entity.set_on_ground(game, false), Ok(true));
                    game.world.set(1, 64, 0, stone).unwrap();

                    // The caster's live hook moves between the acting slot and the hub.
                    game.install_fishing_hook(
                        sunrise_gameplay_common::game::fishing_common::Hook::remote(
                            900,
                            Vec3::new(0.95, 65.0, 0.5),
                            [101, 102, 103, 104],
                        ),
                    );
                    let hook = game.entity(900).unwrap();
                    assert_eq!(hook.set_on_ground(game, true), Ok(true));
                    assert_eq!(hook.on_block(game), Some([1, 64, 0]));
                    hook
                })
                .unwrap();
            assert_eq!(hook.on_ground(game), Some(true));
            assert_eq!(hook.on_block(game), Some([1, 64, 0]));
            assert_eq!(hook.set_on_ground(game, false), Ok(true));
            assert_eq!(hook.on_block(game), Some([0, 64, 0]));
            assert_eq!(entity.on_ground(game), Some(false));
            assert_eq!(entity.on_block(game), Some([0, 64, 0]));
            let guest = game.command_guest(12).unwrap();
            assert!(!guest.on_ground);
            assert!(!guest.motion.as_ref().unwrap().ground.on_ground);
            assert_eq!(guest.motion.as_ref().unwrap().ground.supporting_block, None);
        });
    }
}
