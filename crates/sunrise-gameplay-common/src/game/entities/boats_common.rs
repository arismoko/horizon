//! Common boat physics, fluid/status queries, placement clip, and collision helpers shared by server and client.
use super::{collision_source::Mover, entity::Entity};
use std::{f32::consts::PI, io, sync::Arc};
use sunrise_core::nbt::Tag;
use sunrise_core::{
    geometry::{Aabb, ShapeOp, Vec3, VoxelShape},
    position::BlockPos,
    random::{LegacyRandom, RandomSource},
};
use sunrise_items::{blocks::Fluid, items::ItemStack};
use sunrise_level::collisions::{EntityCollisions, SceneCollisions, no_collision_with_context};
use sunrise_sim::mobs::{
    leash::{Holder, Pending},
    riders,
};
use sunrise_sim::{
    entity_rules::EntityRules,
    motion::{BubbleColumn, bubble_column_at},
};
use sunrise_world::{
    collision_context::CollisionContext,
    registry::BlockStateId,
    world_ray::{RayBlockMode, RayFluidMode, RayWorld, clip_world},
};

pub(in crate::game) struct OutlineFluidRay<'a, 'r, S: crate::game::SideGame<'r>>(
    pub(in crate::game) &'a S,
    pub std::marker::PhantomData<fn(&'r ()) -> &'r ()>,
);

impl<'__carrier, S: crate::game::SideGame<'__carrier>> RayWorld
    for OutlineFluidRay<'_, '__carrier, S>
{
    type Error = io::Error;
    type Block = BlockStateId;
    type Fluid = Option<(f32, bool, Fluid)>;
    fn block_state(&mut self, p: BlockPos) -> io::Result<BlockStateId> {
        Ok(self.0.world.block(p.x, p.y, p.z))
    }
    fn fluid_state(&mut self, p: BlockPos) -> io::Result<Self::Fluid> {
        Ok(self
            .0
            .fluid_height_of(p.x, p.y, p.z, None)
            .map(|(h, source)| {
                (
                    h,
                    source,
                    self.0
                        .res
                        .facts()
                        .get(self.0.world.block(p.x, p.y, p.z))
                        .fluid,
                )
            }))
    }
    fn block_shape(
        &mut self,
        state: BlockStateId,
        p: BlockPos,
        _: RayBlockMode,
    ) -> io::Result<VoxelShape> {
        // LiquidBlock.getShape is empty. Waterlogged blocks retain their outline.
        if matches!(
            &*self.0.res.facts().get(state).name,
            "minecraft:water" | "minecraft:lava" | "minecraft:bubble_column"
        ) {
            return Ok(VoxelShape::empty());
        }
        let mut shape = VoxelShape::empty();
        for b in self.0.outline_boxes([p.x, p.y, p.z]) {
            let b: [f64; 6] = std::array::from_fn(|i| b[i] - f64::from([p.x, p.y, p.z][i % 3]));
            let part = VoxelShape::from_box(Aabb::new(
                Vec3::new(b[0], b[1], b[2]),
                Vec3::new(b[3], b[4], b[5]),
            ))
            .map_err(|e| io::Error::other(format!("{e:?}")))?;
            shape = shape
                .join(&part, ShapeOp::OR)
                .map_err(|e| io::Error::other(format!("{e:?}")))?;
        }
        Ok(shape)
    }
    fn interaction_shape(&mut self, _: BlockStateId, _: BlockPos) -> io::Result<VoxelShape> {
        Ok(VoxelShape::empty())
    }
    fn fluid_is_source(&mut self, s: Self::Fluid) -> io::Result<bool> {
        Ok(s.is_some_and(|s| s.1))
    }
    fn fluid_is_empty(&mut self, s: Self::Fluid) -> io::Result<bool> {
        Ok(s.is_none())
    }
    fn fluid_is_water(&mut self, s: Self::Fluid) -> io::Result<bool> {
        Ok(s.is_some_and(|s| s.2 == Fluid::Water))
    }
    fn fluid_shape(&mut self, s: Self::Fluid, _: BlockPos) -> io::Result<VoxelShape> {
        match s {
            None => Ok(VoxelShape::empty()),
            Some((h, _, _)) => {
                VoxelShape::from_box(Aabb::new(Vec3::ZERO, Vec3::new(1.0, f64::from(h), 1.0)))
                    .map_err(|e| io::Error::other(format!("{e:?}")))
            }
        }
    }
}

impl crate::game::CommonGame<'_> {
    pub fn fluid_height(&self, x: i32, y: i32, z: i32) -> Option<(f32, bool)> {
        self.fluid_height_of(x, y, z, Some(Fluid::Water))
    }

    /// `FluidState.getHeight` and whether it is a source, for `fluid` or,
    /// with `None`, any fluid (`ClipContext.Fluid.ANY`).
    pub fn fluid_height_of(
        &self,
        x: i32,
        y: i32,
        z: i32,
        fluid: Option<Fluid>,
    ) -> Option<(f32, bool)> {
        let f = self.res.facts().get(self.world.block(x, y, z));
        if f.fluid == Fluid::None || fluid.is_some_and(|want| f.fluid != want) {
            return None;
        }
        let above = self
            .res
            .facts()
            .get(self.world.block(x, y.wrapping_add(1), z));
        // FluidState.getHeight: 1 under the same fluid, else amount / 9.
        let height = if above.fluid == f.fluid {
            1.0
        } else {
            f32::from(f.fluid_level) / 9.0
        };
        Some((height, f.fluid_level == 8 && !f.fluid_falling))
    }

    /// `getWaterLevelAbove`: the top of the water column the hull is in.
    /// A float, as vanilla's: the block's y plus its water height in float.
    pub(in crate::game) fn water_level_above(&self, b: &Boat) -> f32 {
        let bb = b.bounds();
        let (lo, hi) = (bb.min_position(), bb.max_position());
        let (y0, y1) = (hi.y.floor() as i32, hi.y.ceil() as i32);
        'rows: for y in y0..y1 {
            let mut height = 0.0f32;
            for x in lo.x.floor() as i32..hi.x.ceil() as i32 {
                for z in lo.z.floor() as i32..hi.z.ceil() as i32 {
                    if let Some((h, _)) = self.fluid_height(x, y, z) {
                        height = height.max(h);
                    }
                    if height >= 1.0 {
                        continue 'rows;
                    }
                }
            }
            if height < 1.0 {
                return y as f32 + height;
            }
        }
        (i64::from(y1) + 1) as f32
    }

    /// `Entity.move`'s fall reset for an entity that keeps a fall and moves on
    /// by `moved` from `from`: a move of a block or more casts up to 8 blocks
    /// of it (`ClipContext.Block.FALLDAMAGE_RESETTING`, `ClipContext.Fluid.WATER`)
    /// and a hit resets the fall, before the move's own drop counts.
    pub fn vehicle_fall_reset(&self, fall: f64, from: Vec3, moved: Vec3) -> f64 {
        let length = moved.length_squared();
        if fall == 0.0 || length < 1.0 {
            return fall;
        }
        let reset = &self.res.motion().fall_reset;
        let fluids = &self.res.motion().fluids;
        let to = from.add_vector(moved.normalize().scale(length.sqrt().min(8.0)));
        let hit = sunrise_core::geometry::traverse_blocks(from, to, 1_000_000, |p| {
            // A tagged block clips as a full block, not its collider.
            let name = &self.res.facts().get(self.world.block(p.x, p.y, p.z)).name;
            if reset.contains(&**name) && VoxelShape::block().clip(from, to, p).is_some() {
                return Some(());
            }
            // The water's shape, through the shared fluid shape cache.
            sunrise_sim::entities::water_clip(&self.world, fluids, from, to, p).then_some(())
        });
        if matches!(hit, Ok(Some(()))) {
            0.0
        } else {
            fall
        }
    }

    /// `AbstractBoat.checkFallDamage` after a move that went `moved_y` (with
    /// `b` already where it landed): on the ground the fall resets; a move
    /// down over anything but water (the block under its `blockPosition`)
    /// adds to it.
    pub fn boat_fall(&self, b: &Boat, moved_y: f64) -> f64 {
        if b.ground.on_ground {
            return 0.0;
        }
        if moved_y < 0.0 {
            let below = self.res.facts().get(self.world.block(
                b.position.x.floor() as i32,
                (b.position.y.floor() as i32).wrapping_sub(1),
                b.position.z.floor() as i32,
            ));
            if below.fluid != Fluid::Water {
                return b.fall_distance - f64::from(moved_y as f32);
            }
        }
        b.fall_distance
    }

    /// Refuse a removed tick target before carrying its slot into the clone.
    pub fn store_boat_back(&mut self, index: usize, boat: Boat) -> bool {
        if self
            .persist
            .removals
            .contains(&crate::game::commands_common::Target::Boat(boat.id))
        {
            return false;
        }
        self.boats.store_back(index, boat)
    }

    /// `BubbleColumnBlock.entityInside`: the bubble columns boat `i` touches.
    pub fn boat_bubble_columns(&self, i: usize) -> Vec<BubbleColumn> {
        let b = self.boats[i].bounds().deflate(Vec3::new(
            f64::from(1.0e-5f32),
            f64::from(1.0e-5f32),
            f64::from(1.0e-5f32),
        ));
        let (lo, hi) = (b.min_position(), b.max_position());
        let mut columns = Vec::new();
        for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
            for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
                for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                    if let Some(column) = bubble_column_at(
                        &self.world,
                        self.pack.assets.registry,
                        self.res.facts(),
                        self.collision,
                        BlockPos::new(x, y, z),
                    ) {
                        columns.push(column);
                    }
                }
            }
        }
        columns
    }

    /// `AbstractBoat.onAboveBubbleColumn`: whether the splash comes up, 1 in 100 above water.
    pub fn boat_splash_roll(&mut self, i: usize) -> bool {
        !self.boats[i].under_water() && self.world.random.next_int_bound(100).unwrap_or(1) == 0
    }

    /// `Entity.makeStuckInBlock` from powder snow and cobweb cells a boat reaches into.
    pub fn boat_stuck_in_blocks(&self, b: &mut Boat) {
        let mut cells = Vec::new();
        if b.previous != b.position {
            cells.extend(reached_cells(b.bounds_at(b.previous), |_| true));
        }
        for cell in reached_cells(b.bounds(), |_| true) {
            if !cells.contains(&cell) {
                cells.push(cell);
            }
        }
        for cell in cells {
            let name = &*self
                .res
                .facts()
                .get(self.world.block(cell[0], cell[1], cell[2]))
                .name;
            b.stuck = match name {
                "minecraft:powder_snow" => powder_snow_stuck(),
                "minecraft:cobweb" => sunrise_sim::falling_block::web_multiplier(),
                _ => continue,
            };
            b.fall_distance = 0.0;
        }
    }

    /// Whether the player rides a chest boat (`HasCustomInventoryScreen`).
    pub fn riding_chest_boat(&self) -> bool {
        self.riding
            .and_then(|id| self.boats.iter().find(|b| b.id == id))
            .is_some_and(|b| b.chest)
    }
}

pub trait BoatsCommonGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// `AbstractInterpolationHandler.interpolate` for the boat this client
    /// drives: its own moves since the last step carry the target along (where
    /// the boat fits there), then `LinearInterpolationHandler.doInterpolate`
    /// takes a `1 / steps` share of the way.
    fn interpolate_driven_boat(&self, b: &mut Boat) {
        let Some(mut l) = b.lerp.take() else {
            return;
        };
        let moved = b.position.subtract(l.last.0);
        if moved.length_squared() > f64::from(1.0e-5_f32) {
            let to = l.target.add_vector(moved);
            if self.boat_lerp_target_fits(b, to) {
                l.target = to;
            }
        }
        l.yaw += b.yaw - l.last.1;
        l.pitch += b.pitch - l.last.2;
        let alpha = 1.0 / f64::from(l.steps);
        b.position = b.position.lerp(l.target, alpha);
        let yaw = f64::from(b.yaw) + alpha * wrap_degrees_f64(f64::from(l.yaw) - f64::from(b.yaw));
        let pitch = f64::from(b.pitch) + alpha * (f64::from(l.pitch) - f64::from(b.pitch));
        // Entity.setRot
        b.yaw = yaw as f32 % 360.0;
        b.pitch = pitch as f32 % 360.0;
        l.steps -= 1;
        l.last = (b.position, b.yaw, b.pitch);
        if l.steps > 0 {
            b.lerp = Some(l);
        }
    }

    /// `AbstractInterpolationHandler.adjustInterpolationPosition`: check the
    /// shifted target against blocks, entities and the border at the boat's live pose.
    fn boat_lerp_target_fits(&self, boat: &Boat, target: Vec3) -> bool {
        let bounds = boat.bounds_at(target);
        let scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        let mut collisions = EntityCollisions::with_border(
            scene,
            self.border.geometry(),
            [boat.position.x, boat.position.z],
        );
        let e = sunrise_core::geometry::COLLISION_EPSILON;
        let boxes = self.entity_collisions(Mover::Boat {
            id: boat.id,
            area: bounds.inflate(Vec3::new(e, e, e)),
        });
        collisions.entities = &boxes;
        no_collision_with_context(
            &mut collisions,
            bounds,
            CollisionContext::position(boat.position.y),
        )
    }

    /// `Entity.recordMovement` for a both-side lid push: move an active
    /// interpolation target by the displacement, if the boat fits there.
    fn record_driven_boat_lid_movement(&mut self, i: usize, delta: Vec3) -> bool {
        let boat = &self.boats[i];
        let Some(mut lerp) = boat.lerp else {
            return false;
        };
        let to = lerp.target.add_vector(delta);
        if self.boat_lerp_target_fits(boat, to) {
            lerp.target = to;
        }
        // The next step measures any *other* move since the last pose.
        lerp.last.0 = lerp.last.0.add_vector(delta);
        self.boats.update(i, |boat| boat.lerp = Some(lerp));
        true
    }

    fn tick_boat(&mut self, mut b: Boat, controlled: bool) -> Boat {
        b.previous = b.position;
        b.previous_yaw = b.yaw;
        // Entity.commonTick: the client's interpolation, before the boat ticks.
        if controlled {
            let from = b.position;
            self.interpolate_driven_boat(&mut b);
            // A step that crosses a section is filed where it happens, not folded into
            // the end-of-tick scan with the boat's own move.
            if from != b.position {
                self.entity_section_step(
                    b.id,
                    [from.x, from.y, from.z],
                    [b.position.x, b.position.y, b.position.z],
                );
            }
        }
        // Entity.baseTick ends with Leashable.tickLeash, on a server only
        // (a ridden boat's lead ticks in `tick_boats`).
        if !controlled {
            self.side_tick_boat_leash(&mut b);
        }
        let had_status = std::mem::replace(&mut b.status_known, true);
        b.old_status = b.status;
        let (mut water_level, mut land_friction) = (b.water_level, b.land_friction);
        b.status = self.boat_status(&b, &mut water_level, &mut land_friction);
        (b.water_level, b.land_friction) = (water_level, land_friction);
        b.tick_submersion();
        // `tick_boats_now` wears it off for the boats it ticks, before they burn.
        if controlled {
            b.wear_off_hurt();
        }
        // A remote client gets the controller from the passenger packet, even when
        // this window occupies the second seat. Local worlds derive it here.
        if !self.side_has_remote() {
            b.paddle_controller = controlled;
        }
        if !controlled {
            b.paddling = [false; 2];
            b.input = [false; 4];
        }
        // `Entity.baseTick`: the boat is immersed and pushed by the current it
        // sits in (`isPushedByFluid`), before `floatBoat` scales the velocity.
        // updateFluidInteraction: in water the fall resets.
        if b.push_by_fluid(&self.world, &self.res.motion().fluids, &self.loop_context)
            .0
            > 0.0
        {
            b.fall_distance = 0.0;
        }
        // floatBoat
        // `-getGravity()`: none with NoGravity.
        let mut vspeed = 0.0 - b.get_gravity();
        let mut buoyancy = 0.0;
        let inv_friction: f32;
        let obstacles = self.boat_move_obstacles(&b);
        let mut scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        if had_status
            && b.old_status == Status::InAir
            && b.status != Status::InAir
            && b.status != Status::OnLand
        {
            // getY(1.0): the top of its box.
            b.water_level = b.position.y + HEIGHT;
            // getWaterLevelAbove() - getBbHeight(): a float subtraction.
            let target = f64::from(self.water_level_above(&b) - HEIGHT as f32) + 0.101;
            let moved = Aabb::new(
                Vec3::new(
                    b.bounds().min_position().x,
                    target,
                    b.bounds().min_position().z,
                ),
                Vec3::new(
                    b.bounds().max_position().x,
                    target + HEIGHT,
                    b.bounds().max_position().z,
                ),
            );
            if scene.unobstructed(moved).unwrap_or(false) {
                b.position.y = target;
                b.velocity.y = 0.0;
            }
            b.status = Status::InWater;
        } else {
            match b.status {
                Status::InWater => {
                    buoyancy = (b.water_level - b.position.y) / HEIGHT;
                    inv_friction = 0.9;
                }
                Status::UnderFlowingWater => {
                    vspeed = -7.0e-4;
                    inv_friction = 0.9;
                }
                Status::UnderWater => {
                    buoyancy = f64::from(0.01_f32);
                    inv_friction = 0.45;
                }
                Status::InAir => inv_friction = 0.9,
                Status::OnLand => {
                    inv_friction = b.land_friction;
                    if controlled {
                        b.land_friction /= 2.0;
                    }
                }
            }
            let f = f64::from(inv_friction);
            b.velocity = Vec3::new(b.velocity.x * f, b.velocity.y + vspeed, b.velocity.z * f);
            b.delta_rotation *= inv_friction;
            if buoyancy > 0.0 {
                b.velocity.y = (b.velocity.y + buoyancy * (0.04 / 0.65)) * 0.75;
            }
        }
        // controlBoat
        if controlled {
            let [left, right, up, down] = b.input;
            let mut acceleration = 0.0f32;
            if left {
                b.delta_rotation -= 1.0;
            }
            if right {
                b.delta_rotation += 1.0;
            }
            if right != left && !up && !down {
                acceleration += 0.005;
            }
            b.yaw += b.delta_rotation;
            if up {
                acceleration += 0.04;
            }
            if down {
                acceleration -= 0.005;
            }
            // AbstractBoat.controlBoat: Mth's sine table, at the float angle.
            use sunrise_core::geometry::trig;
            b.velocity.x +=
                f64::from(trig::sin(f64::from(-b.yaw * trig::DEG_TO_RAD)) * acceleration);
            b.velocity.z +=
                f64::from(trig::cos(f64::from(b.yaw * trig::DEG_TO_RAD)) * acceleration);
            b.paddling = [right && !left || up, left && !right || up];
        }
        let mut scene = EntityCollisions {
            blocks: scene,
            entities: &obstacles,
            border: Some((self.border.geometry(), [b.position.x, b.position.z])),
        };
        let movement = b.take_movement();
        let mut gate = sunrise_sim::move_gate::armed_with_controlled(
            &self.loop_context,
            b.id,
            || std::borrow::Cow::Owned(b.entity_type()),
            sunrise_world::entity_move::MoverType::SelfMovement,
            movement,
            controlled,
        );
        let movement = gate.delta();
        if let Ok(moved) = sunrise_sim::move_gate::entities_resolve(
            &mut gate,
            &mut scene,
            b.bounds(),
            movement,
            0.0,
            b.ground.on_ground,
        ) {
            b.fall_distance = self.vehicle_fall_reset(b.fall_distance, b.position, moved);
            b.position = Vec3::new(
                b.position.x + moved.x,
                b.position.y + moved.y,
                b.position.z + moved.z,
            );
            let side = game_side!(self, self.side_game_side_fields());
            let motion = sunrise_sim::motion::BodyMotion {
                world: &self.world,
                collision: self.collision,
                facts: self.res.facts(),
                blocks: Some(&self.motion.blocks),
                context: &self.loop_context,
                side: Some(&side),
            };
            if !gate.skipped() {
                motion.record_move(
                    sunrise_sim::motion::BodyCollision {
                        footprint: (b.position, b.bounds()),
                        movement: (movement, moved),
                        authoritative: true,
                        context: sunrise_sim::motion::BodyMotion::entity_context(b.id, b.position),
                        subject: None,
                    },
                    &mut b.ground,
                );
            }
            let effect = motion.legacy_contact(b.position, &b.ground);
            b.fall_distance = self.boat_fall(&b, moved.y);
            let entity = (b.id, b.entity_type_name());
            let response = motion.restitute(
                entity,
                sunrise_sim::motion::BodyMove {
                    wanted: movement,
                    moved,
                    effect,
                },
                b.get_gravity(),
                1.0_f32,
                b.velocity,
                b.fall_distance,
            );
            b.velocity = self.side_finish_body_restitution(entity, b.position, response);
        }
        self.body_motion(&game_side!(self, self.side_game_side_fields()))
            .apply_block_speed(b.position, &mut b.velocity);
        self.boat_step_on(&mut b);
        self.boat_stuck_in_blocks(&mut b);
        b
    }

    /// `Entity.canSimulateMovement`: for a boat (`AUTHORITATIVE_SIDE`).
    fn boat_can_simulate_movement(&self, id: u32) -> bool {
        self.side_boat_can_simulate_movement(id)
    }

    /// `AbstractBoat.canAddPassenger`: a seat is free and the boat is not under water.
    fn boat_has_room(&self, i: usize) -> bool {
        self.side_boat_has_room(i)
    }

    /// Where the player in boat `i` sits: in the middle alone, in front of a mob.
    fn player_boat_seat(&self, i: usize) -> Vec3 {
        self.side_player_boat_seat(i)
    }

    /// `Entity.handleOnInsideBubbleColumn`.
    fn boat_inside_bubble_column(&mut self, i: usize, column: BubbleColumn) {
        if !self.boat_can_simulate_movement(self.boats[i].id) {
            return;
        }
        self.boats.update(i, |b| column.push(&mut b.velocity));
    }

    /// `AbstractBoat.handleBubbleColumnEffect`: up at 0.6, down by -0.7.
    fn handle_bubble_column_effect(&mut self, i: usize, drag_down: bool) {
        if !self.boat_can_simulate_movement(self.boats[i].id) {
            return;
        }
        self.boats.update(i, |b| {
            if drag_down {
                b.velocity.y += -0.7;
            } else {
                b.velocity.y = 0.6;
            }
        });
    }

    fn boat_step_on(&mut self, boat: &mut Boat) {
        let mut velocity = boat.velocity;
        let writes = self.body_step_on(
            (boat.id, boat.entity_type_name()),
            (boat.position, boat.bounds()),
            &boat.ground,
            &mut velocity,
        );
        let (writes, hurts) = split_boat_contact_hurts(writes);
        boat.apply_step_writes(velocity, writes);
        self.side_queue_boat_contact_hurts(boat.id, hurts);
    }

    /// The second `applyEffectsFromBlocks` of `AbstractBoat.tick` uses the movement left by the first.
    fn boat_second_step(&mut self, i: usize) -> bool {
        let b = &self.boats[i];
        if b.ground.on_ground {
            let mut velocity = b.velocity;
            let (id, kind, footprint, ground) = (
                b.id,
                b.shared_type_name(),
                (b.position, b.bounds()),
                b.ground,
            );
            let writes = self.body_step_on((id, &kind), footprint, &ground, &mut velocity);
            let (writes, hurts) = split_boat_contact_hurts(writes);
            self.boats
                .update(i, |b| b.apply_step_writes(velocity, writes));
            self.side_queue_boat_contact_hurts(id, hurts);
            // The stored row is current here, so the hurt applies at once; a
            // breaking hit stops the rest of the tick.
            return self.side_apply_boat_contact_hurts(id);
        }
        false
    }

    /// The boat `BoatItem.use` would place for `stack` (id = 0; server caller assigns its entity id).
    fn boat_to_place(&self, stack: &ItemStack) -> Option<Boat> {
        let (wood, chest) = registered_boat_item(&stack.item, self.res.items())?;
        let hit = self.boat_item_hit()?;
        let mut boat = Boat::new(
            0,
            Arc::from(wood),
            Vec3::new(hit[0], hit[1], hit[2]),
            self.yaw,
        );
        if chest {
            boat = boat.with_chest();
        }
        let bb = boat.bounds();
        let (lo, hi) = (bb.min_position(), bb.max_position());
        let e = 1.0e-7;
        let inner = Aabb::new(
            Vec3::new(lo.x + e, lo.y + e, lo.z + e),
            Vec3::new(hi.x - e, hi.y - e, hi.z - e),
        );
        let mut scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        if !scene.unobstructed(inner).unwrap_or(false) || self.boat_entity_collision(bb) {
            return None;
        }
        Some(boat)
    }

    /// What boat `b` may run into this tick.
    fn boat_move_obstacles(&self, b: &Boat) -> Vec<Aabb> {
        let reach = b.velocity.length() + 1.0;
        self.entity_collisions(Mover::Boat {
            id: b.id,
            area: b.bounds().inflate(Vec3::new(reach, reach, reach)),
        })
    }

    /// `noEntityCollision(boat, box)` for a new boat (`BoatItem.use`).
    fn boat_entity_collision(&self, bb: Aabb) -> bool {
        // EntityGetter.getEntityCollisions: the box grown by 1e-7.
        let area = bb.inflate(Vec3::new(1.0e-7, 1.0e-7, 1.0e-7));
        !self
            .entity_collisions(Mover::Boat { id: u32::MAX, area })
            .is_empty()
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> BoatsCommonGameExt1<'__sg> for G {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    InWater,
    UnderWater,
    UnderFlowingWater,
    OnLand,
    InAir,
}
#[derive(Clone, Debug)]
pub struct Boat {
    /// `Entity.xRot`: kept for `/tp` and `Rotation`; the kind turns only in yaw.
    pub pitch: f32,
    /// When it joined its list (`sunrise_client::join_seq`); set by the list.
    pub join: sunrise_sim::join_seq::JoinSeq,
    pub attachment_lifetime: sunrise_mod_runtime::attachments::AttachmentLifetime,
    pub id: u32,
    /// Wood type (`oak`, `bamboo` for the raft).
    pub wood: Arc<str>,
    pub(in crate::game) type_names: [Arc<str>; 2],
    pub position: Vec3,
    pub previous: Vec3,
    pub velocity: Vec3,
    pub yaw: f32,
    pub previous_yaw: f32,
    pub delta_rotation: f32,
    pub status: Status,
    pub(in crate::game) old_status: Status,
    /// `AbstractBoat.oldStatus` is null until the first tick has set a status.
    pub(in crate::game) status_known: bool,
    pub(in crate::game) water_level: f64,
    pub(in crate::game) land_friction: f32,
    pub out_of_control: f32,
    /// `AbstractBoat.DATA_ID_BUBBLE_TIME` (synched): the ticks of `BUBBLE_TIME`
    /// left before a bubble column lifts or drags the boat; 0 when it is not
    /// over one.
    pub bubble_time: i32,
    /// `AbstractBoat.isAboveBubbleColumn`, set by `onAboveBubbleColumn` and
    /// cleared by `tickBubbleColumn` (the server's).
    pub above_bubble_column: bool,
    /// `AbstractBoat.bubbleColumnDirectionIsDown`.
    pub bubble_down: bool,
    /// The server asked a rider whose client moves it (a player) to get out:
    /// `guest_vehicle_tick` honors it (`Entity.ejectPassengers`).
    pub eject_riders: bool,
    /// `Entity.tickCount`, counted where the wobble is (a client's tick).
    pub(in crate::game) tick_count: i32,
    /// `AbstractBoat.bubbleMultiplier`, `bubbleAngle` and `bubbleAngleO`
    /// (a client's).
    pub bubble_multiplier: f32,
    pub bubble_angle: f32,
    pub bubble_angle_old: f32,
    pub hurt_time: i32,
    pub hurt_dir: i32,
    pub damage: f32,
    /// `Entity.remainingFireTicks` (`Fire` in its save): burning while
    /// positive, a fire block's cooldown while negative.
    pub fire_ticks: i32,
    /// `Entity.ticksFrozen` (`TicksFrozen` in its save): powder snow builds
    /// it up (`InsideBlockEffectType.FREEZE`); only a living entity melts it.
    pub ticks_frozen: i32,
    /// `Entity.noGravity` (`NoGravity` in its save).
    pub no_gravity: bool,
    pub paddles: [f32; 2],
    pub paddling: [bool; 2],
    /// Whether this boat has a player controlling its paddles.
    pub paddle_controller: bool,
    /// left, right, up, down
    pub input: [bool; 4],
    pub ground: sunrise_world::ground_support::GroundState,
    /// `Entity.fallDistance`: counted by `AbstractBoat.checkFallDamage`, saved
    /// and loaded (`fall_distance`).
    pub fall_distance: f64,
    /// `Entity.stuckSpeedMultiplier`: set where the boat sits in powder snow
    /// (`PowderSnowBlock.entityInside`) or a cobweb (`WebBlock.entityInside`),
    /// spent by the next move.
    pub stuck: Vec3,
    /// `ChestBoat`: a chest with 27 slots in the back seat.
    pub chest: bool,
    pub items: Vec<Option<ItemStack>>,
    /// `AbstractBoat.leashData`: who holds its lead (`boat_leads.rs`).
    pub leash: Option<Holder>,
    /// Entity id of the specific leash knot instance when `leash == Some(Holder::Knot(_))`.
    pub leash_knot_id: Option<u32>,
    /// The player a `Holder::Player` lead is tied to (their UUID).
    pub leash_player: Option<[i32; 4]>,
    /// A holder read from a save that is not in the world yet.
    pub leash_pending: Option<Pending>,
    /// `LeashData.angularMomentum`.
    pub leash_spin: f64,
    /// `Entity.tickCount`, for the 100 ticks a saved holder is waited for.
    pub age: i32,
    pub random: LegacyRandom,
    /// The server teleport the boat this client drives is easing to.
    pub lerp: Option<DrivenLerp>,
    /// What `entity_size` answered at its last size refresh (`EntityEvent.Size`): its box takes
    /// the answer while it holds for its vanilla size.
    pub(in crate::game) size_answer: Option<sunrise_sim::pose::SizeAnswer>,
}
/// `LinearInterpolationHandler` of the boat this client drives: where a server
/// teleport takes it (`interpolationData`), the steps left, and where the boat
/// was after the last step (`lastPositionAndRotation`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrivenLerp {
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub(in crate::game) steps: u32,
    pub(in crate::game) last: (Vec3, f32, f32),
}
/// `LinearInterpolationHandler.DEFAULT_INTERPOLATION_STEPS`, the length
/// `AbstractBoat.createInterpolationHandler` gives it.
pub(in crate::game) const LERP_STEPS: u32 = 3;
/// `Mth.wrapDegrees(double)`.
pub(in crate::game) fn wrap_degrees_f64(angle: f64) -> f64 {
    let mut a = angle % 360.0;
    if a >= 180.0 {
        a -= 360.0;
    }
    if a < -180.0 {
        a += 360.0;
    }
    a
}
/// `ChestBoat.CONTAINER_SIZE`.
pub const CHEST_SLOTS: usize = 27;
/// `AbstractBoat.BUBBLE_TIME`: the ticks over a bubble column before it
/// lifts or drags the boat.
pub const BUBBLE_TIME: i32 = 60;
/// `EntityEvent` 71 and 72, which `AbstractBoat.tickBubbleColumn` broadcasts
/// for a down and an up column.
pub const BUBBLE_DOWN_EVENT: i8 = 71;
pub const BUBBLE_UP_EVENT: i8 = 72;
/// What `PowderSnowBlock.entityInside` passes to `makeStuckInBlock`.
pub fn powder_snow_stuck() -> Vec3 {
    Vec3::new(f64::from(0.9_f32), 1.5, f64::from(0.9_f32))
}
/// The cells a box reaches into (deflated by 1e-5, as `Entity.checkInsideBlocks` does),
/// in x, y, z order.
pub fn reached_cells(bounds: Aabb, is: impl Fn([i32; 3]) -> bool) -> Vec<[i32; 3]> {
    let e = f64::from(1.0e-5_f32);
    let bounds = bounds.deflate(Vec3::new(e, e, e));
    let (lo, hi) = (bounds.min_position(), bounds.max_position());
    let mut cells = Vec::new();
    for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
        for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
            for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                if is([x, y, z]) {
                    cells.push([x, y, z]);
                }
            }
        }
    }
    cells
}
/// `Entity.load`'s `TicksFrozen` (an int; 0 when absent).
pub fn saved_ticks_frozen(tag: &Tag) -> i32 {
    sunrise_sim::save::number(tag.get("TicksFrozen")).unwrap_or(0) as i32
}
/// `Entity.saveWithoutId`: `TicksFrozen` is written only while positive.
pub(in crate::game) fn put_ticks_frozen(map: &mut sunrise_core::nbt::Compound, ticks: i32) {
    if ticks > 0 {
        map.insert("TicksFrozen".into(), Tag::Int(ticks));
    } else {
        map.remove(&"TicksFrozen".into());
    }
}
pub const WIDTH: f64 = 1.375;
pub const HEIGHT: f64 = 0.5625;
impl Boat {
    pub(in crate::game) fn apply_step_writes(
        &mut self,
        velocity: Vec3,
        writes: Vec<mod_api::EntityWrite>,
    ) {
        self.velocity = velocity;
        super::entity::apply_contact_writes(
            writes,
            &mut self.velocity,
            Some(&mut self.fall_distance),
            Some(&mut self.stuck),
            Some(&mut self.fire_ticks),
        );
    }

    /// `AbstractBoat.tick`: the hurt wobble and the damage wear off, before
    /// `Entity.baseTick` can burn the boat.
    pub fn wear_off_hurt(&mut self) {
        self.hurt_time = (self.hurt_time - 1).max(0);
        self.damage = (self.damage - 1.0).max(0.0);
    }
    /// `AbstractBoat.tick`: passengers lose control after 60 consecutive
    /// underwater ticks; any other status resets the counter.
    pub(in crate::game) fn tick_submersion(&mut self) {
        self.out_of_control =
            if matches!(self.status, Status::UnderWater | Status::UnderFlowingWater) {
                self.out_of_control + 1.0
            } else {
                0.0
            };
    }

    pub fn new(id: u32, wood: Arc<str>, position: Vec3, yaw: f32) -> Self {
        let suffix = if &*wood == "bamboo" { "raft" } else { "boat" };
        let type_names = [
            format!("minecraft:{wood}_{suffix}").into(),
            format!("minecraft:{wood}_chest_{suffix}").into(),
        ];
        Self {
            size_answer: None,
            pitch: 0.0,
            join: Default::default(),
            attachment_lifetime: Default::default(),
            id,
            wood,
            type_names,
            position,
            previous: position,
            velocity: Vec3::ZERO,
            yaw,
            previous_yaw: yaw,
            delta_rotation: 0.0,
            status: Status::InAir,
            old_status: Status::InAir,
            status_known: false,
            water_level: 0.0,
            land_friction: 0.0,
            out_of_control: 0.0,
            bubble_time: 0,
            above_bubble_column: false,
            bubble_down: false,
            eject_riders: false,
            tick_count: 0,
            bubble_multiplier: 0.0,
            bubble_angle: 0.0,
            bubble_angle_old: 0.0,
            hurt_time: 0,
            hurt_dir: 1,
            damage: 0.0,
            fire_ticks: 0,
            ticks_frozen: 0,
            no_gravity: false,
            paddles: [0.0; 2],
            paddling: [false; 2],
            paddle_controller: false,
            input: [false; 4],
            ground: Default::default(),
            fall_distance: 0.0,
            stuck: Vec3::ZERO,
            chest: false,
            items: Vec::new(),
            leash: None,
            leash_knot_id: None,
            leash_player: None,
            leash_pending: None,
            leash_spin: 0.0,
            age: 0,
            random: sunrise_level::random_source::entity(),
            lerp: None,
        }
    }
    /// `AbstractInterpolationHandler.interpolateTo`: the boat eases to a server
    /// teleport over `LERP_STEPS` ticks, unless it is easing to that spot already.
    pub fn interpolate_to(&mut self, target: Vec3, yaw: f32, pitch: f32) {
        if self
            .lerp
            .is_some_and(|l| l.yaw == yaw && l.pitch == pitch && l.target == target)
        {
            return;
        }
        self.lerp = Some(DrivenLerp {
            target,
            yaw,
            pitch,
            steps: LERP_STEPS,
            last: (self.position, self.yaw, self.pitch),
        });
    }
    /// The client took over a boat that was easing toward the server's position
    /// (`target` and the rotation, `steps` to go): its LinearInterpolationHandler
    /// goes on, now following the boat's own moves too.
    pub fn go_on_easing(&mut self, target: Vec3, rotation: (f32, f32), steps: u32) {
        if steps == 0 || self.lerp.is_some() {
            return;
        }
        self.lerp = Some(DrivenLerp {
            target,
            yaw: rotation.0,
            pitch: rotation.1,
            steps,
            last: (self.position, self.yaw, self.pitch),
        });
    }
    /// `Entity.getClientPositionAndRotation`: where the boat is easing to, or
    /// where it is when it isn't.
    pub fn client_position(&self) -> (Vec3, f32, f32) {
        self.lerp
            .map_or((self.position, self.yaw, self.pitch), |l| {
                (l.target, l.yaw, l.pitch)
            })
    }
    /// A chest boat or chest raft.
    pub fn with_chest(mut self) -> Self {
        self.chest = true;
        self.items = vec![None; CHEST_SLOTS];
        self
    }
    /// `EntityType.OAK_BOAT & co`'s `getDimensions()`: width, height and eye height.
    pub(in crate::game) const DIMENSIONS: [f32; 3] = [1.375, 0.5625, 0.5625];
    pub fn bounds(&self) -> Aabb {
        self.bounds_at(self.position)
    }
    /// The box of this boat were its position `p`.
    pub fn bounds_at(&self, p: Vec3) -> Aabb {
        let w = WIDTH / 2.0;
        let vanilla = Aabb::new(
            Vec3::new(p.x - w, p.y, p.z - w),
            Vec3::new(p.x + w, p.y + HEIGHT, p.z + w),
        );
        sunrise_sim::pose::SizeAnswer::sized_box(self.size_answer, Self::DIMENSIONS, p, vanilla)
    }
    /// `Entity.saveWithoutId` with `AbstractChestBoat`'s items: the loaded
    /// compound `raw` under the live fields.
    pub fn to_nbt(
        &self,
        uuid: [i32; 4],
        raw: Option<&Tag>,
        items: &sunrise_items::items::Items,
    ) -> Tag {
        let mut tag = sunrise_sim::entity_nbt::boat_to_nbt(
            &self.entity_type(),
            self.position,
            self.velocity,
            [self.yaw, self.pitch],
            (self.ground.on_ground, self.fall_distance),
            uuid,
            raw,
            self.chest.then_some(&self.items[..]),
            items,
        );
        sunrise_sim::entity_nbt::write_saved_flags(
            &mut tag,
            self.ground.on_ground,
            self.no_gravity,
        );
        if let Tag::Compound(map) = &mut tag {
            map.insert("Fire".into(), Tag::Short(self.fire_ticks as i16));
            put_ticks_frozen(map, self.ticks_frozen);
        }
        tag
    }
    /// The start of `Entity.move`: a boat made stuck since its last move goes its
    /// velocity times the multiplier, and its velocity and the multiplier are spent.
    pub fn take_movement(&mut self) -> Vec3 {
        if self.stuck.length_squared() <= 1.0e-7 {
            return self.velocity;
        }
        let movement = self.velocity.multiply(self.stuck);
        self.stuck = Vec3::ZERO;
        self.velocity = Vec3::ZERO;
        movement
    }
    /// `isUnderWater`; it stands in for `canAddPassenger`'s
    /// `isEyeInFluid(WATER)` too.
    pub fn under_water(&self) -> bool {
        matches!(self.status, Status::UnderWater | Status::UnderFlowingWater)
    }
    /// `Entity.isInLiquid`, for `Leashable.angularFriction`.
    pub fn in_liquid(&self) -> bool {
        matches!(
            self.status,
            Status::InWater | Status::UnderWater | Status::UnderFlowingWater
        )
    }
    /// `getMaxPassengers`: two, one beside a chest.
    pub fn max_passengers(&self) -> usize {
        if self.chest { 1 } else { 2 }
    }
    /// `rideHeight`: a third of a boat's height, 8/9 of a raft's.
    pub(in crate::game) fn ride_height(&self) -> f64 {
        let height = HEIGHT as f32;
        f64::from(if &*self.wood == "bamboo" {
            height * 0.888_888_9
        } else {
            height / 3.0
        })
    }
    /// Where the passenger at `index` of `count` stands, at the boat's current
    /// (or, with `previous`, last) place: `getPassengerAttachmentPoint`
    /// turned with the boat, less the passenger's `rider_offset`.
    pub fn seat(
        &self,
        index: usize,
        count: usize,
        animal: bool,
        rider_offset: f64,
        previous: bool,
    ) -> Vec3 {
        let mut z = if self.chest { 0.15f32 } else { 0.0 };
        if count > 1 {
            z = if index == 0 { 0.2 } else { -0.6 };
            if animal {
                z += 0.2;
            }
        }
        let (at, yaw) = if previous {
            (self.previous, self.previous_yaw)
        } else {
            (self.position, self.yaw)
        };
        riders::seat(
            [0.0, self.ride_height(), f64::from(z)],
            rider_offset,
            at,
            yaw,
        )
    }
    /// `AbstractBoat.getBubbleAngle`: the wobble, in degrees, `a` of the way
    /// from the last tick's to this one's.
    pub fn bubble_angle(&self, a: f32) -> f32 {
        self.bubble_angle_old + a * (self.bubble_angle - self.bubble_angle_old)
    }
    /// `AbstractBoat.tickBubbleColumn`: on a client, the wobble grows by 0.05 a
    /// tick while the synched bubble time runs and fades by 0.1 otherwise.
    pub fn tick_bubble_wobble(&mut self) {
        self.tick_count += 1;
        if self.bubble_time > 0 {
            self.bubble_multiplier += 0.05;
        } else {
            self.bubble_multiplier -= 0.1;
        }
        self.bubble_multiplier = self.bubble_multiplier.clamp(0.0, 1.0);
        self.bubble_angle_old = self.bubble_angle;
        self.bubble_angle =
            10.0 * (0.5 * f64::from(self.tick_count)).sin() as f32 * self.bubble_multiplier;
    }
    /// `AbstractBoat.tickBubbleColumn`'s countdown on the server: the bubble
    /// time drops while the boat stays over a column and resets when it leaves.
    /// `Some(down)` on the tick it runs out: the boat is thrown up (`false`) or
    /// pulled down (`true`).
    pub fn tick_bubble_countdown(&mut self) -> Option<bool> {
        if !self.above_bubble_column {
            self.bubble_time = 0;
        }
        if self.bubble_time <= 0 {
            return None;
        }
        self.bubble_time -= 1;
        let diff = BUBBLE_TIME - self.bubble_time - 1;
        let fired = (diff > 0 && self.bubble_time == 0).then_some(self.bubble_down);
        self.above_bubble_column = false;
        fired
    }
    /// `getRowingTime`.
    pub fn paddle_state(&self, side: usize) -> bool {
        self.paddling[side] && self.paddle_controller
    }
    pub fn rowing_time(&self, side: usize, a: f32) -> f32 {
        if self.paddle_state(side) {
            let current = self.paddles[side];
            let previous = current - PI / 8.0;
            previous + a.clamp(0.0, 1.0) * (current - previous)
        } else {
            0.0
        }
    }
}
/// Item id -> (boat entity id, wood).
impl Boat {
    /// The vanilla entity type id (`oak_boat`, `bamboo_chest_raft`), also
    /// the item it drops.
    pub fn entity_type(&self) -> String {
        self.entity_type_name().to_owned()
    }

    pub fn entity_type_name(&self) -> &str {
        &self.type_names[usize::from(self.chest)]
    }

    pub fn shared_type_name(&self) -> Arc<str> {
        Arc::clone(&self.type_names[usize::from(self.chest)])
    }
}
/// The wood of a boat or chest boat item or entity type, and whether it
/// carries a chest.
pub fn any_boat_wood(item: &str) -> Option<(&str, bool)> {
    let name = item.strip_prefix("minecraft:")?;
    if let Some(wood) = name
        .strip_suffix("_chest_boat")
        .or_else(|| name.strip_suffix("_chest_raft"))
    {
        return Some((wood, true));
    }
    boat_wood(item).map(|w| (w, false))
}
pub(in crate::game) fn boat_wood(item: &str) -> Option<&str> {
    let name = item.strip_prefix("minecraft:")?;
    name.strip_suffix("_boat")
        .or_else(|| name.strip_suffix("_raft"))
        .filter(|w| !w.ends_with("_chest"))
}
/// `EntityType.by`: a boat-shaped id still needs a registered entity type.
pub fn registered_boat<'a>(
    kind: &'a str,
    registry: &sunrise_game_data::registries::Registries,
) -> Option<(&'a str, bool)> {
    let boat = any_boat_wood(kind)?;
    registry
        .ids("minecraft:entity_type")
        .binary_search_by(|id| id.as_str().cmp(kind))
        .ok()?;
    Some(boat)
}

/// The boat item and its matching vanilla entity type are both registered.
pub fn registered_boat_item<'a>(
    id: &'a str,
    items: &sunrise_items::items::Items,
) -> Option<(&'a str, bool)> {
    let boat = registered_boat(id, &items.component_registry)?;
    items.get(id)?;
    Some(boat)
}

sunrise_sim::impl_joined!(Boat);
sunrise_mod_runtime::impl_attachment_lifetime!(Boat);

impl EntityRules for Boat {
    fn is_no_gravity(&self) -> bool {
        self.no_gravity
    }
    /// `AbstractBoat.getDefaultGravity`.
    fn get_default_gravity(&self) -> f64 {
        0.04
    }
}

/// `Player.DEFAULT_VEHICLE_ATTACHMENT`: how far below the seat a player's
/// feet are.
pub const PLAYER_RIDING_OFFSET: f64 = 0.6;

/// `Entity.push(Entity)`: the push `other` gets from `this` (which takes the
/// opposite one), or None when they stand on nearly the same spot.
pub fn push_impulse(this: Vec3, other: Vec3) -> Option<Vec3> {
    let (mut xa, mut za) = (other.x - this.x, other.z - this.z);
    let mut dd = xa.abs().max(za.abs());
    if dd < f64::from(0.01_f32) {
        return None;
    }
    dd = dd.sqrt();
    xa /= dd;
    za /= dd;
    let pow = (1.0 / dd).min(1.0);
    let k = f64::from(0.05_f32);
    Some(Vec3::new(xa * pow * k, 0.0, za * pow * k))
}
/// `AbstractBoat.push`: another boat is pushed while it is lower than this
/// boat's top, anything else while its feet are no higher than the boat's.
pub fn boat_pushes(boat: Aabb, other: Aabb, other_is_boat: bool) -> bool {
    if other_is_boat {
        other.min_position().y < boat.max_position().y
    } else {
        other.min_position().y <= boat.min_position().y
    }
}

use crate::game::side_ops::mobs_rest::MobsRestSideGameExt as _;

use crate::game::side_ops::modapi_hooks::ModapiHooksSideGameExt as _;

use crate::game::GameGameExt1 as _;

use crate::game::collision_source::CollisionSourceGameExt1 as _;

use crate::game::entity_common::EntityCommonGameExt1 as _;

use crate::game::entity_sections::EntitySectionsGameExt1 as _;

pub trait BoatItemHitGameExt<'g>: crate::game::SideGame<'g> {
    /// `Item.getPlayerPOVHitResult`: float view vector, OUTLINE and Fluid.ANY.
    fn boat_item_hit(&self) -> Option<[f64; 3]> {
        let [x, y, z] = self.eye_position()?;
        let start = Vec3::new(x, y, z);
        let dir = sunrise_world::view_direction::view_vector(self.pitch, self.yaw);
        let end = start.add_vector(dir.scale(self.reach()));
        let hit = clip_world(
            &mut OutlineFluidRay(self, std::marker::PhantomData),
            start,
            end,
            RayBlockMode::Outline,
            RayFluidMode::Any,
            256,
        )
        .ok()?;
        (!hit.miss).then_some([hit.hit.location.x, hit.hit.location.y, hit.hit.location.z])
    }
}
impl<'g, G: crate::game::SideGame<'g>> BoatItemHitGameExt<'g> for G {}

use crate::game::native_surface::BoatFrictionGameExt as _;

pub trait BoatStatusGameExt<'g>: crate::game::SideGame<'g> {
    fn boat_status(&self, b: &Boat, water_level: &mut f64, land_friction: &mut f32) -> Status {
        let bb = b.bounds();
        let (lo, hi) = (bb.min_position(), bb.max_position());
        // isUnderwater
        let max_y = hi.y + 0.001;
        let mut under = false;
        for x in lo.x.floor() as i32..hi.x.ceil() as i32 {
            for y in hi.y.floor() as i32..max_y.ceil() as i32 {
                for z in lo.z.floor() as i32..hi.z.ceil() as i32 {
                    if let Some((h, source)) = self.fluid_height(x, y, z)
                        && max_y < f64::from(y as f32 + h)
                    {
                        if !source {
                            *water_level = hi.y;
                            return Status::UnderFlowingWater;
                        }
                        under = true;
                    }
                }
            }
        }
        if under {
            *water_level = hi.y;
            return Status::UnderWater;
        }
        // checkInWater
        let mut in_water = false;
        *water_level = -f64::MAX;
        for x in lo.x.floor() as i32..hi.x.ceil() as i32 {
            for y in lo.y.floor() as i32..(lo.y + 0.001).ceil() as i32 {
                for z in lo.z.floor() as i32..hi.z.ceil() as i32 {
                    if let Some((h, _)) = self.fluid_height(x, y, z) {
                        // `float height = y + getHeight`: summed in float.
                        let level = f64::from(y as f32 + h);
                        *water_level = water_level.max(level);
                        in_water |= lo.y < level;
                    }
                }
            }
        }
        if in_water {
            return Status::InWater;
        }

        let friction = self.boat_ground_friction(b, bb);
        if friction > 0.0 {
            *land_friction = friction;
            Status::OnLand
        } else {
            Status::InAir
        }
    }

    /// `AbstractBoat.getGroundFriction`: average friction of the blocks whose
    /// collision shape meets a thin slab just under the hull (lily pads skipped).
    fn boat_ground_friction(&self, b: &Boat, bb: Aabb) -> f32 {
        let (lo, hi) = (bb.min_position(), bb.max_position());
        let ground = Aabb::new(
            Vec3::new(lo.x, lo.y - 0.001, lo.z),
            Vec3::new(hi.x, lo.y, hi.z),
        );
        let (g_lo, g_hi) = (ground.min_position(), ground.max_position());
        let (x0, x1, y0, y1, z0, z1) = (
            (g_lo.x.floor() - 1.0) as i32,
            (g_hi.x.ceil() + 1.0) as i32,
            (g_lo.y.floor() - 1.0) as i32,
            (g_hi.y.ceil() + 1.0) as i32,
            (g_lo.z.floor() - 1.0) as i32,
            (g_hi.z.ceil() + 1.0) as i32,
        );
        let slab = VoxelShape::from_box(ground).unwrap_or_else(|_| VoxelShape::empty());
        let mut scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        let (mut friction, mut count) = (0.0f32, 0);
        for x in x0..x1 {
            for z in z0..z1 {
                let edges = i32::from(x == x0 || x == x1 - 1) + i32::from(z == z0 || z == z1 - 1);
                if edges == 2 {
                    continue;
                }
                for y in y0..y1 {
                    if edges == 1 && (y == y0 || y == y1 - 1) {
                        continue;
                    }
                    let state = self.world.block(x, y, z);
                    let f = self.res.facts().get(state);
                    if f.name.as_ref() == "minecraft:air" || f.has_class("LilyPadBlock") {
                        continue;
                    }
                    let Ok(shape) = self.collision.collision_shape_with_context(
                        &mut scene,
                        state,
                        BlockPos::new(x, y, z),
                        &CollisionContext::empty(false),
                    ) else {
                        continue;
                    };
                    let world_shape =
                        shape.moved(Vec3::new(f64::from(x), f64::from(y), f64::from(z)));
                    if world_shape
                        .join_is_not_empty(&slab, ShapeOp::AND)
                        .unwrap_or(false)
                    {
                        friction +=
                            self.boat_friction(b, BlockPos::new(x, y, z), state, f.friction);
                        count += 1;
                    }
                }
            }
        }
        if count > 0 {
            friction / count as f32
        } else {
            0.0
        }
    }

    /// `AbstractBoat.tick`: `status = getStatus()` for a boat this window does not tick.
    fn refresh_boat_status(&mut self, i: usize) {
        let (mut water_level, mut land_friction) = (0.0, 0.0);
        let status = self.boat_status(&self.boats[i], &mut water_level, &mut land_friction);
        self.boats.update(i, |b| b.status = status);
        self.boats.update(i, |b| b.status_known = true);
        self.boats.update(i, Boat::tick_submersion);
    }
}
impl<'g, G: crate::game::SideGame<'g>> BoatStatusGameExt<'g> for G {}

fn split_boat_contact_hurts(
    writes: Vec<mod_api::EntityWrite>,
) -> (Vec<mod_api::EntityWrite>, Vec<(&'static str, f32)>) {
    let mut rest = Vec::with_capacity(writes.len());
    let mut hurts = Vec::new();
    for write in writes {
        match write {
            mod_api::EntityWrite::Hurt {
                damage_type,
                amount,
            } => {
                if let Some(name) =
                    mod_api::__private::parts_installation::damage_type_name(damage_type)
                {
                    hurts.push((name, amount));
                }
            }
            other => rest.push(other),
        }
    }
    (rest, hurts)
}
