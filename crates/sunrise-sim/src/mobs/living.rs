//! `LivingEntity` movement for mobs: `aiStep` input decay and jumping,
//! `travel` in air, water and lava (friction-scaled acceleration, gravity,
//! fluid slowdown and falling adjustment), fluid immersion heights, fall
//! damage, fire and the walk animation.
use super::{Mob, MobEvent, MobWorld};
use crate::health::DamageLevel;
use crate::items::BlockTagFlags;
use crate::{blocks::Fluid, collisions::SceneCollisions};
use std::ops::ControlFlow;
use sunrise_core::{
    geometry::{Aabb, Axis, Vec3, trig},
    position::BlockPos,
    random::RandomSource,
};
use sunrise_world::native_load_context::LoopContext;
use sunrise_world::{
    block_landing::{
        BlockFall, FallDamageCall, FallDamageTail, FarmlandTrample, LandingEntityKind,
    },
    entity_fluid::{CurrentAccumulator, CurrentTarget},
    entity_move::MoverType,
    fluid::{FluidKind, FluidState},
    fluid_flow::FluidFlowWorld,
    move_gate::MoveGate,
    movement_restitution::{RestitutionInput, RestitutionWorld, restitute_movement},
    registry::BlockStateId,
};
#[path = "climbing.rs"]
mod climbing;
mod travel;

/// How many block particles a landing throws up
/// (`(int)(min(0.2F + power / 15, 2.5) * 150)`), none when the fall is within
/// `safe_fall_distance` (`power = max(floor(fall + 1.0E-6 - safe), 0)`).
fn landing_dust_count(fall_distance: f64, safe_fall_distance: f64) -> Option<i32> {
    let power = f64::from(((fall_distance + 1.0e-6 - safe_fall_distance).floor() as i32).max(0));
    (power > 0.0).then(|| (150.0 * (f64::from(0.2_f32) + power / 15.0).min(2.5)) as i32)
}
/// Where the dust starts: at the mob, or, when its own block is not the one it
/// landed on (`landing`), on that block's face toward the mob.
fn landing_dust_at(position: Vec3, landing: [i32; 3]) -> Vec3 {
    if landing[0] == position.x.floor() as i32 && landing[2] == position.z.floor() as i32 {
        return position;
    }
    let dx = position.x - f64::from(landing[0]) - 0.5;
    let dz = position.z - f64::from(landing[2]) - 0.5;
    let scale = dx.abs().max(dz.abs());
    Vec3::new(
        f64::from(landing[0]) + 0.5 + dx / scale * 0.5,
        position.y,
        f64::from(landing[2]) + 0.5 + dz / scale * 0.5,
    )
}

/// `Entity.moveDist`, `flyDist`, `nextStep` and `Parrot.nextFlap`.
#[derive(Clone, Copy, Debug)]
pub struct Steps {
    pub move_dist: f32,
    pub fly_dist: f32,
    pub next_step: f32,
    pub next_flap: f32,
    /// `AbstractHorse.gallopSoundCounter`.
    pub gallop: i32,
}
impl Default for Steps {
    fn default() -> Self {
        Self {
            move_dist: 0.0,
            fly_dist: 0.0,
            next_step: 1.0,
            next_flap: 1.0,
            gallop: 0,
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Fluids {
    /// The boat whose fluid interaction box last seated this passenger.
    pub boat: Option<(u32, crate::motion::RiddenBoat)>,
    pub water: f64,
    pub lava: f64,
    pub in_water: bool,
    pub in_lava: bool,
    pub eye_in_water: bool,
    pub eye_in_lava: bool,
}
/// `FluidFlowWorld` over the mob's scene: the neighbours a flow vector reads.
struct FlowScene<'a, 'w>(&'a MobWorld<'w>);
impl FluidFlowWorld for FlowScene<'_, '_> {
    type Error = std::convert::Infallible;
    type Block = BlockStateId;
    fn fluid(&mut self, p: BlockPos) -> Result<FluidState, Self::Error> {
        Ok(self.0.fluid_tables.fluid(self.0.world.block(p.x, p.y, p.z)))
    }
    fn block(&mut self, p: BlockPos) -> Result<BlockStateId, Self::Error> {
        Ok(self.0.world.block(p.x, p.y, p.z))
    }
    fn blocks_fluid_flow(&self, s: BlockStateId) -> bool {
        self.0.fluid_tables.blocks_flow(s)
    }
    fn is_ice_class(&self, s: BlockStateId) -> bool {
        self.0.fluid_tables.ice(s)
    }
    fn is_face_sturdy(
        &mut self,
        s: BlockStateId,
        _: BlockPos,
        d: sunrise_core::geometry::Direction,
    ) -> Result<bool, Self::Error> {
        Ok(self.0.fluid_tables.sturdy(s, d))
    }
}
/// The mob's velocity, for `CurrentAccumulator::apply_to`.
struct Pushed<'a>(&'a mut Vec3);
impl CurrentTarget for Pushed<'_> {
    type Error = std::convert::Infallible;
    fn is_player_class(&self) -> bool {
        false
    }
    fn velocity(&mut self) -> Result<Vec3, Self::Error> {
        Ok(*self.0)
    }
    fn add_delta_movement(&mut self, delta: Vec3) -> Result<(), Self::Error> {
        let sum = Vec3::new(self.0.x + delta.x, self.0.y + delta.y, self.0.z + delta.z);
        if delta.is_finite() && sum.is_finite() {
            *self.0 = sum;
        }
        Ok(())
    }
}
/// Dimensions as `EntityDimensions` holds them: the floats `Mob::dims` widened, back.
pub(crate) fn dims_array((w, h, e): (f64, f64, f64)) -> [f32; 3] {
    [w as f32, h as f32, e as f32]
}
pub fn wrap(a: f32) -> f32 {
    let mut a = a % 360.0;
    if a >= 180.0 {
        a -= 360.0;
    }
    if a < -180.0 {
        a += 360.0;
    }
    a
}
/// `LivingEntity.tick` and `tickHeadTurn` for a player, with
/// `Player.getMaxHeadRotationRelativeToBody`'s smaller turn when blocking.
pub fn player_body_yaw(
    mut body: f32,
    yaw: f32,
    dx: f64,
    dz: f64,
    swinging: bool,
    blocking: bool,
) -> f32 {
    use sunrise_core::geometry::trig;

    let mut target = body;
    if (dx * dx + dz * dz) as f32 > 0.002_500_000_2_f32 {
        let walk = trig::atan2(dz, dx) as f32 * trig::RAD_TO_DEG - 90.0;
        let facing = (wrap(yaw) - walk).abs();
        target = if 95.0 < facing && facing < 265.0 {
            walk - 180.0
        } else {
            walk
        };
    }
    if swinging {
        target = yaw;
    }
    body += wrap(target - body) * 0.3;
    let diff = wrap(yaw - body);
    let max_head_turn = if blocking { 15.0 } else { 50.0 };
    if diff.abs() > max_head_turn {
        body += diff - diff.signum() * max_head_turn;
    }
    body
}

/// `BlockState.getCollisionShape(level, pos)` with the empty collision context.
fn block_shape_in(
    world: &crate::world::World,
    collision: &sunrise_world::block_collision::BlockCollisionRegistry,
    x: i32,
    y: i32,
    z: i32,
) -> Option<sunrise_core::geometry::VoxelShape> {
    let mut scene = SceneCollisions {
        world,
        binding: collision,
        subject: None,
    };
    let state = world.block(x, y, z);
    let at = sunrise_core::position::BlockPos::new(x, y, z);
    let context = sunrise_world::collision_context::CollisionContext::empty(false);
    collision
        .collision_shape_with_context(&mut scene, state, at, &context)
        .ok()
}
/// [`block_shape_in`] as boxes in the world.
pub fn collision_boxes_in(
    world: &crate::world::World,
    collision: &sunrise_world::block_collision::BlockCollisionRegistry,
    x: i32,
    y: i32,
    z: i32,
) -> Vec<Aabb> {
    let offset = Vec3::new(f64::from(x), f64::from(y), f64::from(z));
    block_shape_in(world, collision, x, y, z)
        .map(|shape| shape.boxes().iter().map(|b| b.move_by(offset)).collect())
        .unwrap_or_default()
}
impl MobWorld<'_> {
    /// Entity.updateFluidHeightAndDoFluidPushing's currents over the deflated
    /// box: the flow of every water and lava block it touches, summed (a thin
    /// fluid layer scales its flow down), as (water, lava).
    fn fluid_currents(&self, bounds: Aabb) -> (CurrentAccumulator, CurrentAccumulator) {
        let base_y = bounds.min_position().y;
        let b = bounds.deflate(Vec3::new(0.001, 0.001, 0.001));
        let (lo, hi) = (b.min_position(), b.max_position());
        let mut scene = FlowScene(self);
        let (mut water, mut lava) = (CurrentAccumulator::default(), CurrentAccumulator::default());
        for x in lo.x.floor() as i32..=(hi.x.ceil() as i32 - 1) {
            for y in lo.y.floor() as i32..=(hi.y.ceil() as i32 - 1) {
                for z in lo.z.floor() as i32..=(hi.z.ceil() as i32 - 1) {
                    let p = BlockPos::new(x, y, z);
                    let state = self.fluid_tables.fluid(self.world.block(x, y, z));
                    let current = match state.kind() {
                        // A mod's fluid is in neither tag, so it pushes no entity.
                        FluidKind::Empty | FluidKind::Mod(_) | FluidKind::ModFlowing(_) => continue,
                        FluidKind::Water | FluidKind::FlowingWater => &mut water,
                        FluidKind::Lava | FluidKind::FlowingLava => &mut lava,
                    };
                    let surface = f64::from(y) + self.fluid_height(x, y, z).1;
                    if surface < lo.y {
                        continue;
                    }
                    current.height = current.height.max(surface - base_y);
                    let mut flow =
                        crate::motion::pushing_flow(self.loop_context, &mut scene, state, p)
                            .unwrap_or(Vec3::ZERO);
                    if current.height < 0.4 {
                        flow = flow.scale(current.height);
                    }
                    current.accumulate(flow);
                }
            }
        }
        (water, lava)
    }
    /// `Entity.updateFluidInteraction`'s pushes for an entity fluids push
    /// (`isPushedByFluid`) with the box `bounds`: its `velocity` after the
    /// water's and the lava's currents (`EntityFluidInteraction.applyCurrentTo`).
    pub fn pushed_by_currents(&self, bounds: Aabb, mut velocity: Vec3) -> Vec3 {
        let (water, lava) = self.fluid_currents(bounds);
        let lava_scale = if self.dimension_type.fast_lava {
            0.007
        } else {
            0.0023333333333333335
        };
        let mut pushed = Pushed(&mut velocity);
        if water.height > 0.0 {
            let _ = water.apply_to(&mut pushed, 0.014);
        }
        if lava.height > 0.0 {
            let _ = lava.apply_to(&mut pushed, lava_scale);
        }
        velocity
    }
    /// `BlockState.getCollisionShape(level, pos)` (the empty collision
    /// context) as boxes in the world.
    pub fn block_collision_boxes(&self, x: i32, y: i32, z: i32) -> Vec<Aabb> {
        collision_boxes_in(self.world, self.collision, x, y, z)
    }
    pub fn no_collision(&self, b: Aabb) -> bool {
        let mut scene = SceneCollisions {
            world: self.world,
            binding: self.collision,
            subject: None,
        };
        scene.unobstructed(b).unwrap_or(false)
    }
    /// `LevelReader.containsAnyLiquid`: the blocks from the box's minimum
    /// corner to the block holding its maximum corner.
    pub fn contains_any_liquid(&self, b: Aabb) -> bool {
        let (lo, hi) = (b.min_position(), b.max_position());
        let (x0, y0, z0) = (
            lo.x.floor() as i32,
            lo.y.floor() as i32,
            lo.z.floor() as i32,
        );
        let (x1, y1, z1) = (
            hi.x.floor() as i32,
            hi.y.floor() as i32,
            hi.z.floor() as i32,
        );
        (x0..=x1).any(|x| {
            (y0..=y1).any(|y| {
                (z0..=z1).any(|z| self.facts.get(self.world.block(x, y, z)).fluid != Fluid::None)
            })
        })
    }
}
type TravelFinish = fn(&mut Mob, &MobWorld, &mut DamageLevel<'_>, &mut Vec<MobEvent>);

/// Only travel's own scalar inputs survive the move. The paused tick keeps
/// them inline without allocating a continuation.
#[derive(Debug)]
enum TravelTail {
    Fixed(TravelFinish),
    Air {
        friction: f32,
        stop_gliding: bool,
    },
    Fluid {
        falling: bool,
        old_y: f64,
        gravity: f64,
        water_slow: Option<f32>,
    },
    Flying {
        drag: f64,
    },
    FallFlying {
        before: f64,
    },
}

impl TravelTail {
    #[inline(always)]
    fn finish(
        self,
        mob: &mut Mob,
        w: &MobWorld,
        level: &mut DamageLevel<'_>,
        events: &mut Vec<MobEvent>,
        handoff: bool,
    ) -> ControlFlow<TravelPause> {
        match self {
            Self::Fixed(finish) => finish(mob, w, level, events),
            Self::Air {
                friction,
                stop_gliding,
            } => {
                mob.finish_air_travel(w, friction);
                if stop_gliding {
                    mob.extra.fall_flying = false;
                }
            }
            Self::Fluid {
                falling,
                old_y,
                gravity,
                water_slow,
            } => {
                mob.finish_fluid_travel(w, falling, old_y, gravity, water_slow);
            }
            Self::Flying { drag } => {
                mob.set_velocity(Vec3::new(
                    mob.velocity.x * drag,
                    mob.velocity.y * drag,
                    mob.velocity.z * drag,
                ));
            }
            Self::FallFlying { before } => {
                if mob.ground.horizontal_collision {
                    let damage = ((before
                        - (mob.velocity.x * mob.velocity.x + mob.velocity.z * mob.velocity.z)
                            .sqrt())
                        * 10.0
                        - 3.0) as f32;
                    if damage > 0.0 {
                        let monster = super::ai::is_monster(mob.species());
                        let sound = match (monster, damage as i32 > 4) {
                            (true, true) => "entity.hostile.big_fall",
                            (true, false) => "entity.hostile.small_fall",
                            (false, true) => "entity.generic.big_fall",
                            (false, false) => "entity.generic.small_fall",
                        };
                        if !mob.silent {
                            events.push(MobEvent::Sound(
                                sound,
                                [mob.position.x, mob.position.y, mob.position.z],
                                1.0,
                                1.0,
                            ));
                        }
                        mob.damage(
                            level,
                            damage,
                            super::Damage::of(super::DamageKind::FlyIntoWall),
                        );
                        if handoff && mob.hurt_needs_owner() {
                            return ControlFlow::Break(TravelPause(TravelStop::AfterHit));
                        }
                    }
                }
            }
        }
        ControlFlow::Continue(())
    }
}

/// What a travel branch started its own move with: the `living_travel_moved` inputs that
/// the move itself overwrites (#5074).
#[derive(Clone, Copy, Debug)]
struct TravelStart {
    mode: sunrise_world::travel::TravelMode,
    position: Vec3,
    on_ground: bool,
}

/// Travel stopped in `Entity.move` or after `handleFallFlyingCollisions`' hurt.
#[derive(Debug)]
pub struct TravelPause(TravelStop);

#[derive(Debug)]
enum TravelStop {
    Move {
        movement: MovePause,
        tail: TravelTail,
        start: TravelStart,
    },
    AfterHit,
}

/// What the owning level must do before the mob's move can continue.
pub enum MovementAction {
    Trample(FarmlandTrample),
    SettleHit,
    Passengers(PassengerFall),
}

/// The operands already selected for `Entity.propagateFallToPassengers`.
#[derive(Clone, Copy, Debug)]
pub struct PassengerFall {
    pub distance: f64,
    pub modifier: f32,
}

impl TravelPause {
    pub(super) fn action(&self) -> MovementAction {
        match &self.0 {
            TravelStop::Move { movement, .. } => movement.fall.action(),
            TravelStop::AfterHit => MovementAction::SettleHit,
        }
    }

    pub(super) fn resume(
        self,
        mob: &mut Mob,
        w: &MobWorld,
        level: &mut DamageLevel<'_>,
        events: &mut Vec<MobEvent>,
    ) -> ControlFlow<Self> {
        let TravelStop::Move {
            movement,
            tail,
            start,
        } = self.0
        else {
            return ControlFlow::Continue(());
        };
        if let Some(fall) = mob.resume_fall(w, level, events, movement.fall) {
            return ControlFlow::Break(Self(TravelStop::Move {
                movement: MovePause { fall, ..movement },
                tail,
                start,
            }));
        }
        if !mob.removed {
            mob.finish_move(
                w,
                level,
                movement.wanted,
                movement.moved,
                movement.effect,
                events,
            );
            mob.fire_travel_moved(w, start);
            tail.finish(mob, w, level, events, true)?;
        }
        ControlFlow::Continue(())
    }
}

#[derive(Debug)]
struct MovePause {
    fall: FallPause,
    wanted: Vec3,
    moved: Vec3,
    effect: (BlockPos, BlockStateId),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LandingOwner {
    Synchronous,
    Borrowed,
}

#[derive(Debug)]
enum FallPause {
    Trample(FarmlandTrample),
    Hurt { block: FallenOn, stop: FallHurtStop },
}

impl FallPause {
    fn action(&self) -> MovementAction {
        match self {
            Self::Trample(trample) => MovementAction::Trample(*trample),
            Self::Hurt { stop, .. } => stop
                .passengers()
                .map_or(MovementAction::SettleHit, MovementAction::Passengers),
        }
    }
}

/// A mount's `doCheckFallDamage` is waiting for the authoritative Game.
pub struct ClientMovedFall(FallPause);

impl ClientMovedFall {
    pub fn action(&self) -> MovementAction {
        self.0.action()
    }
}

#[derive(Debug)]
struct FallenOn {
    pos: [i32; 3],
    tail: FallDamageTail,
}

#[derive(Clone, Copy, Debug)]
enum FallHurtStop {
    BeforeOwnHurt(PassengerFall, super::DamageKind),
    AfterHurt(Option<PassengerFall>),
    AfterHorsePassengers(PassengerFall),
}

impl FallHurtStop {
    fn passengers(self) -> Option<PassengerFall> {
        match self {
            Self::BeforeOwnHurt(fall, _) | Self::AfterHorsePassengers(fall) => Some(fall),
            Self::AfterHurt(_) => None,
        }
    }
}

/// A passenger's own fall needs either its owning hurt or a further passenger action.
pub struct PassengerFallPause(FallHurtStop);

impl PassengerFallPause {
    /// `None` means the passenger's own hit must settle before resuming.
    pub fn passengers(&self) -> Option<PassengerFall> {
        self.0.passengers()
    }
}

impl Mob {
    /// `Entity.refreshDimensions` after growing from `old_w` by `old_h`:
    /// `fudgePositionAfterSizeChange` moves the mob to the free spot nearest
    /// its old centre where the bigger box fits (`findFreePosition`), or
    /// failing that, one that fits at the old height.
    pub fn fudge_position_after_size_change(&mut self, w: &MobWorld, old_w: f64, old_h: f64) {
        let (new_w, new_h, _) = self.dims();
        let small = new_w <= 4.0 && new_h <= 4.0;
        if self.age == 0 || self.no_physics() || !small || !(new_w > old_w || new_h > old_h) {
            return;
        }
        let center = Vec3::new(
            self.position.x,
            self.position.y + old_h / 2.0,
            self.position.z,
        );
        let dw = (new_w - old_w).max(0.0) + 1.0e-6;
        let dh = (new_h - old_h).max(0.0) + 1.0e-6;
        let at = |allowed: Aabb, size_y: f64| free_position(w, allowed, center, new_w, size_y);
        if let Some(p) = at(Aabb::of_size(center, Vec3::new(dw, dh, dw)), new_h) {
            teleports::set_pos_with(
                self,
                Vec3::new(p.x, p.y - new_h / 2.0, p.z),
                w.section_writer.clone(),
            );
            return;
        }
        if new_w > old_w
            && new_h > old_h
            && let Some(p) = at(Aabb::of_size(center, Vec3::new(dw, 1.0e-6, dw)), old_h)
        {
            teleports::set_pos_with(
                self,
                Vec3::new(p.x, p.y - old_h / 2.0 + 1.0e-6, p.z),
                w.section_writer.clone(),
            );
        }
    }
    fn no_physics(&self) -> bool {
        false
    }
    pub fn eye_y(&self) -> f64 {
        self.position.y + self.eye_height()
    }
    /// `LivingEntity.getScale`: the `minecraft:scale` the server sent on this
    /// living entity (`extra.model_scale`, 0 when none came), through the
    /// species' `sanitizeScale` override. Every box, eye height, seat and
    /// shadow radius takes it.
    pub fn model_scale(&self) -> f32 {
        let scale = if self.extra.model_scale > 0.0 {
            self.extra.model_scale
        } else {
            1.0
        };
        match self.species() {
            // HappyGhast.sanitizeScale: min(scale, 1), it never grows.
            super::Species::HappyGhast => scale.min(1.0),
            // Shulker.sanitizeScale: min(scale, 3).
            super::Species::Shulker => scale.min(3.0),
            _ => scale,
        }
    }
    /// Entity dimensions: the type's, a class's BABY_DIMENSIONS, or the
    /// slime size scaling, in the mob's current pose.
    /// `EntityDimensions` (float width, height and eye height) widened to double.
    /// An `entity_size` answer (`EntityEvent.Size`) replaces them while the mob is still in
    /// the pose and vanilla size it was asked for.
    pub fn dims(&self) -> (f64, f64, f64) {
        let pose = self.pose();
        let vanilla = self.dims_in_pose(pose);
        match self
            .size_answer
            .and_then(|answer| answer.holds(pose, dims_array(vanilla)))
        {
            Some([w, h, e]) => (f64::from(w), f64::from(h), f64::from(e)),
            None => vanilla,
        }
    }
    /// `dims` as vanilla computes them, without an `entity_size` answer: the `newSize`
    /// `EntityEvent.Size` starts from.
    pub fn vanilla_dims(&self) -> [f32; 3] {
        self.vanilla_dims_in(self.pose())
    }
    /// [`Self::vanilla_dims`] in `pose` (`Entity.getDimensions(pose)`).
    pub fn vanilla_dims_in(&self, pose: mod_api::Pose) -> [f32; 3] {
        dims_array(self.dims_in_pose(pose))
    }
    /// The pose this mob is in (`Entity.getPose`): the stance its species
    /// keeps in its own fields — a villager's bed, a camel's sit flag, an
    /// avatar's stance — else the synced pose id.
    pub fn pose(&self) -> mod_api::Pose {
        if super::villager::sleeping(self) {
            return mod_api::Pose::Sleeping;
        }
        if super::camel::is_camel(self.kind.species) && super::camel::sitting(self) {
            return mod_api::Pose::Sitting;
        }
        if matches!(&*self.kind.id, "minecraft:player" | "minecraft:mannequin") {
            return self.extra.avatar_pose.into();
        }
        mod_api::Pose::from_id(i32::from(self.extra.pose))
    }
    /// `dims` without the sleeping box: a sleeping villager's
    /// `getDimensions(Pose.STANDING)`, whose eye height lays it in the bed
    /// (`LivingEntityRenderer.submit`).
    pub fn awake_dims(&self) -> (f64, f64, f64) {
        self.dims_in_pose(match self.pose() {
            mod_api::Pose::Sleeping => mod_api::Pose::Standing,
            pose => pose,
        })
    }
    /// `Entity.getDimensions(Pose)` of the pose asked for: the species' box
    /// (a player's or mannequin's follows the pose, `Avatar.POSES`; a goat
    /// long-jumping and a camel sitting read it from the pose; the sniffer
    /// digging, the warden emerging, the baby, the slime size, the puffer
    /// fish, the shulker's lid, the salmon's variant and the polar bear's
    /// rear-up read the mob's own state, as their `getDefaultDimensions`
    /// do) times its `minecraft:scale` (`getScale`), width, height and eye
    /// height alike — except fixed boxes: sleeping, dying avatars and
    /// digging/emerging wardens (`EntityDimensions.fixed`).
    pub fn dims_in_pose(&self, pose: mod_api::Pose) -> (f64, f64, f64) {
        if pose == mod_api::Pose::Sleeping {
            return (f64::from(0.2_f32), f64::from(0.2_f32), f64::from(0.2_f32));
        }
        let k = &self.kind;
        let scale = self.model_scale();
        if matches!(&*k.id, "minecraft:player" | "minecraft:mannequin") {
            let d = crate::pose::Dimensions::avatar(pose, scale);
            return (
                f64::from(d.width),
                f64::from(d.height),
                f64::from(d.eye_height),
            );
        }
        let (mut w, mut h, mut e) = (k.width as f32, k.height as f32, k.eye_height as f32);
        // Sniffer.DIGGING_DIMENSIONS: 0.4 lower, its eyes at 0.81.
        if k.species == super::Species::Sniffer && super::sniffer::digging(self) {
            (h, e) = (h - 0.4, 0.81);
        }
        if self.is_baby() {
            match super::species::def(k.species).baby {
                super::species::Baby::Abs(bw, bh, be) => {
                    (w, h, e) = (bw as f32, bh as f32, be as f32)
                }
                super::species::Baby::Scale(s, eye) => {
                    let s = s as f32;
                    (w, h, e) = (w * s, h * s, eye.map_or(e * s, |x| x as f32))
                }
                super::species::Baby::None => {}
            }
        }
        // Goat.getDefaultDimensions: a long jump scales the box by 0.7.
        if pose == mod_api::Pose::LongJumping && k.species == super::Species::Goat {
            (w, h, e) = (w * 0.7_f32, h * 0.7_f32, e * 0.7_f32);
        }
        // Camel.getDefaultDimensions: a sitting camel is lower.
        if super::camel::is_camel(k.species) && pose == mod_api::Pose::Sitting {
            (w, h, e) = super::camel::sitting_dims(self.is_baby());
        }
        // Warden.getDefaultDimensions returns fixed dimensions while digging
        // or emerging: EntityDimensions.scale leaves every component alone.
        if k.species == super::Species::Warden && super::warden::digging_or_emerging(self) {
            return (f64::from(w), 1.0, f64::from(0.85_f32));
        }
        if self.kind.species == super::Species::Pufferfish {
            let s = super::aquatic::puff_scale(self.extra.size) as f32;
            (w, h, e) = (w * s, h * s, e * s);
        } else if self.kind.species == super::Species::Phantom {
            // Phantom.getDefaultDimensions: 1 + 0.15 per size.
            let s = 1.0 + 0.15 * self.extra.size as f32;
            (w, h, e) = (w * s, h * s, e * s);
        } else if self.extra.size > 0 {
            let s = self.extra.size as f32;
            (w, h, e) = (w * s, h * s, e * s);
        }
        // Shulker.getDefaultDimensions: the last scale, over everything
        // above, is the lid opening upward on a shell attached to the floor.
        if k.species == super::Species::Shulker
            && let Some(s) = super::shulker::peek_height_scale(self)
        {
            (h, e) = (h * s, e * s);
        }
        // Salmon.getDefaultDimensions: the variant's bounding box scale (small
        // 0.5, medium 1.0, large 1.5) over the whole fish box.
        if k.species == super::Species::Salmon {
            let s = super::aquatic::salmon_scale(self.extra.variant);
            (w, h, e) = (w * s, h * s, e * s);
        }
        // PolarBear.getDefaultDimensions: the rear-up animation scales the box
        // by (1, 1 + clientSideStandAnimation / 6), so a bear at full height is
        // twice as tall, with its eyes twice as high and the same width.
        if k.species == super::Species::PolarBear {
            let f = super::polar_bear::stand_box_scale(self);
            (h, e) = (h * f, e * f);
        }
        // `EntityDimensions.scale(getScale())`: width and height by the scale,
        // the eye height by its height factor (the same one).
        (
            f64::from(w * scale),
            f64::from(h * scale),
            f64::from(e * scale),
        )
    }
    pub fn eye_height(&self) -> f64 {
        self.dims().2
    }
    /// `EntityRenderer.getPackedLightCoords` reads the block
    /// `Entity.getLightProbePosition(partialTick)` is in, and a living entity
    /// leaves that as `getEyePosition`: the eye height over
    /// `getPosition(partialTick)`, the position the frame draws, not this
    /// tick's. A mob walking across a cell boundary is drawn lit by the cell it
    /// is drawn in.
    pub fn light_probe(&self, partial: f64) -> [i32; 3] {
        eye_probe_cell(self.previous, self.position, self.eye_height(), partial)
    }
    pub fn width(&self) -> f32 {
        self.dims().0 as f32
    }
    pub fn height(&self) -> f32 {
        self.dims().1 as f32
    }
    /// `EntityAttachment.NAME_TAG` of the current pose, before
    /// `SubmitNodeCollector.submitNameTag` lifts it 0.5: the box height
    /// (`EntityAttachment.Fallback.AT_HEIGHT`), unless the type attaches the
    /// tag elsewhere, scaled like the box (`EntityDimensions.scale`).
    pub fn name_tag_height(&self) -> f64 {
        if self.kind.species == super::Species::Sniffer {
            // `EntityTypes.SNIFFER`: `.nameTagOffset(2.05F)` over a 1.75 box.
            // `Sniffer.DIGGING_DIMENSIONS` is built without the attachment, so
            // a digging sniffer falls back to its 1.75 - 0.4 box.
            let height = if super::sniffer::digging(self) {
                1.75_f32 - 0.4
            } else {
                2.05_f32
            };
            // `getAgeScale()` (a baby sniffer is `Baby::Scale(0.5)`), then
            // `getScale()`; `EntityAttachments.scale` multiplies the point in
            // `Vec3`'s doubles, so these are double products.
            let age = match super::species::def(self.kind.species).baby {
                super::species::Baby::Scale(s, _) if self.is_baby() => s as f32,
                _ => 1.0,
            };
            f64::from(height) * f64::from(age) * f64::from(self.model_scale())
        } else {
            f64::from(self.height())
        }
    }
    /// `Entity.saveWithoutId`'s `CustomName`: the whole component as
    /// `ComponentSerialization.CODEC` writes it (a name tag's `custom_name`
    /// keeps its colour), else the plain text. `None` without a name.
    pub(crate) fn custom_name_tag(&self) -> Option<sunrise_core::nbt::Tag> {
        let name = self.custom_name.as_ref()?;
        Some(match &self.custom_name_text {
            Some(text) => crate::scoreboard::component_tag(&text.component()),
            None => sunrise_core::nbt::Tag::String(name.as_str().into()),
        })
    }
    /// Whether the save's `CustomName` is not this mob's name, style included:
    /// a rename to the same text with another colour still changes it.
    pub(crate) fn custom_name_changed(&self, saved: Option<&sunrise_core::nbt::Tag>) -> bool {
        match (&self.custom_name_text, saved) {
            (Some(current), Some(saved)) => crate::entity_nbt::name_text(saved) != *current,
            (Some(_), None) => true,
            // A plain name with no component (a mod's) still compares as text.
            (None, _) => saved.and_then(crate::entity_nbt::custom_name) != self.custom_name,
        }
    }
    /// `AgeableMob.isBaby`: `canBeABaby() && getAge() < 0`. The age alone is
    /// not enough — a frog, a parrot, a zombie nautilus, a camel husk, a slime,
    /// a magma cube, a piglin brute, a tadpole, or anything that is not an
    /// `AgeableMob` at all is an adult whatever `Age` says, so its box, voice,
    /// breeding, growth, drops and goals stay an adult's.
    pub fn is_baby(&self) -> bool {
        self.extra.age < 0 && super::species::can_be_a_baby(self.kind.species)
    }
    pub fn fluid_jump_threshold(&self) -> f64 {
        // Breeze.getFluidJumpThreshold: its eye height.
        if self.species() == super::Species::Breeze {
            return self.eye_height();
        }
        if self.eye_height() < 0.4 { 0.0 } else { 0.4 }
    }
    pub(crate) fn is_in_liquid(&self) -> bool {
        self.fluids.in_water || self.fluids.in_lava
    }
    /// `FloatGoal.canUse` and the brains' `Swim`: water deeper than the jump threshold, or lava.
    pub fn floats(&self) -> bool {
        self.fluids.water > self.fluid_jump_threshold() || self.fluids.in_lava
    }
    /// `Entity.isPushedByFluid`: water animals, turtles, axolotls, frogs and
    /// nautiluses keep their own course.
    fn pushed_by_fluid(&self) -> bool {
        use super::Species as S;
        // Drowned.isPushedByFluid: a drowned swimming after its target is not
        // pushed by the current, one walking the bottom is (`extra.flag` holds
        // the `isSwimming` that `Drowned.updateSwimming` sets).
        if self.species() == S::Drowned && self.extra.flag {
            return false;
        }
        !matches!(
            self.species(),
            S::Tadpole
                | S::Cod
                | S::Salmon
                | S::TropicalFish
                | S::Pufferfish
                | S::Squid
                | S::GlowSquid
                | S::Dolphin
                | S::Turtle
                | S::Axolotl
                | S::Frog
                | S::Nautilus
                | S::ZombieNautilus
        )
    }
    fn affected_by_fluids(&self) -> bool {
        true
    }
    /// `Entity.setDeltaMovement`: keep only finite values.
    pub(crate) fn set_velocity(&mut self, velocity: Vec3) {
        if velocity.is_finite() {
            self.velocity = velocity;
        }
    }
    /// Entity.moveRelative: input rotated by yaw and scaled to `speed`.
    pub fn move_relative(&mut self, speed: f32, input: Vec3) {
        let delta = sunrise_world::living_air::input_vector(input, speed, self.yaw);
        self.set_velocity(self.velocity.add_vector(delta));
    }
    /// Entity.move with block collisions and step-up; updates collision flags
    /// and fall distance (Entity.checkFallDamage).
    /// `EntityGetter.getEntityCollisions(this, box.expandTowards(movement))`:
    /// the boxes of the mobs whose `canBeCollidedWith(this)` holds (a living
    /// shulker, a happy ghast held still), but not this mob's vehicle or a
    /// fellow passenger (`Entity.canCollideWith`).
    fn entity_collisions(&self, w: &MobWorld, movement: Vec3) -> Vec<Aabb> {
        let area = self.bounds().expand_towards(movement);
        if area.average_size() < 1.0e-7 {
            return Vec::new();
        }
        let area = area.inflate(Vec3::new(1.0e-7, 1.0e-7, 1.0e-7));
        let mut boxes = Vec::new();
        let solids = crate::entity_solids::Solids::new(
            crate::entity_solids::Mover::Mob {
                id: self.id,
                happy_ghast: self.species() == super::Species::HappyGhast,
                mount: self.extra.mount,
            },
            Some(area),
        );
        match w.others_solid {
            Some(slots) => {
                for &i in slots {
                    let c = w.others[i as usize].solid_candidate();
                    if solids.accepts(&c, || None) {
                        boxes.push(c.bounds);
                    }
                }
            }
            None => {
                for s in w.others.iter().filter(|s| s.may_block_mobs()) {
                    let c = s.solid_candidate();
                    if solids.accepts(&c, || None) {
                        boxes.push(c.bounds);
                    }
                }
            }
        }
        boxes
    }
    pub fn do_move(
        &mut self,
        w: &MobWorld,
        level: &mut DamageLevel<'_>,
        events: &mut Vec<MobEvent>,
        gate: &mut MoveGate,
    ) {
        self.move_until_pause(w, level, events, gate, false);
    }

    fn move_until_pause(
        &mut self,
        w: &MobWorld,
        level: &mut DamageLevel<'_>,
        events: &mut Vec<MobEvent>,
        gate: &mut MoveGate,
        pause: bool,
    ) -> Option<MovePause> {
        crate::move_gate::arm_controlled(
            gate,
            w.loop_context,
            self.id,
            &self.kind.id,
            MoverType::SelfMovement,
            self.velocity,
            super::equine::controlled(self),
        );
        if gate.skipped() {
            return None;
        }
        let mut wanted = gate.delta();
        // Entity.move: with noPhysics the delta is applied through every block,
        // and no collision flag is set.
        if self.no_physics {
            self.position = self.position.add_vector(wanted);
            self.ground.horizontal_collision = false;
            self.vertical_collision_below = false;
            return None;
        }
        // Entity.move: a block the mob is stuck in (sweet berry bush) scales
        // this move once and stops it.
        if self.stuck_multiplier.length_squared() > 1.0e-7 {
            wanted = wanted.multiply(self.stuck_multiplier);
            self.stuck_multiplier = Vec3::ZERO;
            self.set_velocity(Vec3::ZERO);
        }
        let step = self.kind.step_height as f32;
        let from = self.position;
        let scene = SceneCollisions {
            world: w.world,
            binding: w.collision,
            subject: Some(
                self.collision_subject(w.items.map(|items| items.entity_type_tags.as_ref())),
            ),
        };
        // Entity.collide: the world border's walls stop the mob too, and so
        // do the entities it may not pass through.
        let border = w.border.map(|b| (b, [self.position.x, self.position.z]));
        let entities = self.entity_collisions(w, wanted);
        let moved = crate::move_gate::scene_resolve_in_border_with(
            gate,
            scene,
            self.bounds(),
            wanted,
            step,
            self.ground.on_ground,
            border,
            &entities,
        )
        .unwrap_or(Vec3::ZERO);
        let length = moved.x * moved.x + moved.y * moved.y + moved.z * moved.z;
        let wanted_length = wanted.x * wanted.x + wanted.y * wanted.y + wanted.z * wanted.z;
        if length > 1.0e-7 || wanted_length - length < 1.0e-7 {
            if self.fall_distance != 0.0 && length >= 1.0 && fall_reset_hit(w, self.position, moved)
            {
                self.fall_distance = 0.0;
            }
            self.position = Vec3::new(
                self.position.x + moved.x,
                self.position.y + moved.y,
                self.position.z + moved.z,
            );
            super::living::section_writes::record_with(
                self.id,
                self.join,
                from,
                self.position,
                w.section_writer.clone(),
            );
        }
        // !Mth.equal(delta, movement)
        let differs = |a: f64, b: f64| (b - a).abs() >= f64::from(1.0e-5_f32);
        let x_hit = differs(wanted.x, moved.x);
        let z_hit = differs(wanted.z, moved.z);
        self.ground.horizontal_collision = x_hit || z_hit;
        // A server's mobs are authoritative (`isLocalInstanceAuthoritative`):
        // the vertical flags update even without vertical movement.
        let vertical = wanted.y != moved.y;
        self.vertical_collision_below = vertical && wanted.y < 0.0;
        self.ground.on_ground = self.vertical_collision_below;
        // Entity.setOnGroundWithMovement's checkSupportingBlock, before
        // checkFallDamage reads the landing block (getOnPosLegacy).
        self.update_ground_support(w, moved);
        let owner = if pause {
            LandingOwner::Borrowed
        } else {
            LandingOwner::Synchronous
        };
        let effect = self.restitution_contact(w);
        if let Some(fall) = self.species_check_fall_damage(w, level, moved, effect, events, owner) {
            return Some(MovePause {
                fall,
                wanted,
                moved,
                effect,
            });
        }
        self.finish_move(w, level, wanted, moved, effect, events);
        None
    }

    fn finish_move(
        &mut self,
        w: &MobWorld,
        level: &mut DamageLevel<'_>,
        wanted: Vec3,
        moved: Vec3,
        effect: (BlockPos, BlockStateId),
        events: &mut Vec<MobEvent>,
    ) {
        self.restitute_captured(w, level, wanted, moved, effect);
        self.movement_emission(w, moved, events);
        // Entity.move's last step: soul sand, honey and the like slow the
        // horizontal speed of whatever the mob stands in or on.
        let factor = self.block_speed_factor(w);
        self.set_velocity(sunrise_world::entity_move::scale_by_block_speed(
            self.velocity,
            factor,
        ));
    }
    /// Record the normal move's ground for a forced collision.
    pub fn update_pushed_support(&mut self, w: &MobWorld, wanted: Vec3, moved: Vec3) {
        self.ground.on_ground = wanted.y < 0.0 && wanted.y != moved.y;
        self.update_ground_support(w, moved);
    }

    /// `Entity.restituteMovementAfterCollisions`, shared by travel and piston pushes.
    pub fn restitute_movement(
        &mut self,
        w: &MobWorld,
        level: &mut DamageLevel<'_>,
        wanted: Vec3,
        moved: Vec3,
    ) {
        self.restitute_captured(w, level, wanted, moved, self.restitution_contact(w));
    }
    fn restitution_contact(&self, w: &MobWorld) -> (BlockPos, BlockStateId) {
        let [x, y, z] = self.block_on_legacy(w);
        (BlockPos::new(x, y, z), w.world.block(x, y, z))
    }
    fn restitute_captured(
        &mut self,
        w: &MobWorld,
        level: &mut DamageLevel<'_>,
        wanted: Vec3,
        moved: Vec3,
        (effect_position, block): (BlockPos, BlockStateId),
    ) {
        let differs = |a: f64, b: f64| (b - a).abs() >= f64::from(1.0e-5_f32);
        let x_hit = differs(wanted.x, moved.x);
        let z_hit = differs(wanted.z, moved.z);
        let vertical = wanted.y != moved.y;
        // Entity.restituteMovementAfterCollisions: the colliding components
        // bounce back by the entity's bounciness or the block's (a slime
        // block, a bed), whichever is larger. A mob never sneaks, so nothing
        // suppresses its bounce there.
        if wanted.y.abs() > 0.0 && vertical || x_hit || z_hit {
            let input = RestitutionInput {
                block,
                effect_position,
                collided_x: x_hit,
                collided_z: z_hit,
                vertical_collision: vertical,
                collision_below: vertical && wanted.y < 0.0,
                living: true,
                resolved_movement: moved,
                fall_distance: self.fall_distance,
            };
            // The port's phases cannot fail for a mob: a block outside the
            // registry reads as one with no bounce, never as an error.
            let _ = restitute_movement(
                &mut MobRestitution {
                    mob: self,
                    world: w,
                    level,
                },
                input,
            );
        }
    }
    /// `Entity.getBlockSpeedFactor` at the mob's feet.
    fn block_speed_factor(&self, w: &MobWorld) -> f32 {
        crate::motion::block_speed_factor_at(
            w.world,
            w.movement,
            w.loop_context,
            self.position,
            self.ground.supporting_block,
        )
    }
    /// `Entity.applyEffectsFromBlocks` as `LivingEntity.aiStep` runs it, after
    /// `travel` has moved the mob and applied the drag of its fluid (not inside
    /// `Entity.move`): bubble columns push the mob, so the push is not slowed
    /// by that tick's drag, and the server shows the surface of one it is at the
    /// top of.
    pub fn apply_effects_from_blocks(&mut self, w: &MobWorld, events: &mut Vec<MobEvent>) {
        let mut tops = Vec::new();
        if crate::motion::bubble_column_push_over(
            w.world,
            w.registry,
            w.facts,
            w.collision,
            self.bounds(),
            &mut self.velocity,
            |pos| tops.push(pos),
        ) {
            self.fall_distance = 0.0;
        }
        for pos in tops {
            events.push(MobEvent::BubbleColumnParticles {
                pos: [pos.x, pos.y, pos.z],
            });
        }
    }
    /// `checkFallDamage` as the species has it: Strider's lava breaks any fall
    /// (the fluid state waits for the next baseTick); Bat's and Bee's do nothing.
    fn species_check_fall_damage(
        &mut self,
        w: &MobWorld,
        level: &mut DamageLevel<'_>,
        moved: Vec3,
        effect: (BlockPos, BlockStateId),
        events: &mut Vec<MobEvent>,
        owner: LandingOwner,
    ) -> Option<FallPause> {
        if self.kind.flags.lava_walker && self.fluids.in_lava {
            self.fall_distance = 0.0;
        } else if !matches!(self.kind.species, super::Species::Bat | super::Species::Bee) {
            return self.check_fall_damage(w, level, moved, effect, events, owner);
        }
        None
    }
    /// `handleMoveVehicle` for a mount its rider's client moves, after the move:
    /// `setOnGroundWithMovement` with the client's ground (`checkSupportingBlock`),
    /// then `doCheckFallDamage` with the client's move `delta`. The server's
    /// `Entity.move` counted no fall: it isn't the mount's authority.
    pub fn client_moved_fall(
        &mut self,
        w: &MobWorld,
        level: &mut DamageLevel<'_>,
        delta: Vec3,
        on_ground: bool,
        events: &mut Vec<MobEvent>,
    ) -> Option<ClientMovedFall> {
        self.ground.on_ground = on_ground;
        self.update_ground_support(w, delta);
        self.species_check_fall_damage(
            w,
            level,
            delta,
            self.restitution_contact(w),
            events,
            LandingOwner::Borrowed,
        )
        .map(ClientMovedFall)
    }
    /// Refresh fluids before applying the captured landing block.
    fn check_fall_damage(
        &mut self,
        w: &MobWorld,
        level: &mut DamageLevel<'_>,
        moved: Vec3,
        effect: (BlockPos, BlockStateId),
        events: &mut Vec<MobEvent>,
        owner: LandingOwner,
    ) -> Option<FallPause> {
        if !self.fluids.in_water {
            self.update_fluids(w, events);
        }
        // Strider.checkFallDamage: in lava the fall is reset and the base never
        // runs, so a strider that drops into a lava sea lands with no damage and
        // no dust, and walks out of it owing nothing.
        if self.species() == super::Species::Strider && self.fluids.in_lava {
            self.fall_distance = 0.0;
            return None;
        }
        if !self.fluids.in_water && moved.y < 0.0 {
            self.fall_distance -= f64::from(moved.y as f32);
        }
        if self.ground.on_ground {
            if self.fall_distance > 0.0 {
                self.landing_dust(w, effect, events);
                if let Some(fall) = self.fall_on_block(w, level, effect, events, owner) {
                    return Some(fall);
                }
            }
            self.fall_distance = 0.0;
        }
        None
    }
    /// The block particles `LivingEntity.checkFallDamage` sends up from the
    /// block a mob lands on, by the power of the fall.
    fn landing_dust(
        &self,
        w: &MobWorld,
        (position, state): (BlockPos, BlockStateId),
        events: &mut Vec<MobEvent>,
    ) {
        let pos = [position.x, position.y, position.z];
        let Some(count) = landing_dust_count(self.fall_distance, self.safe_fall_distance()) else {
            return;
        };
        if w.facts.is_air(state) {
            return;
        }
        let at = landing_dust_at(self.position, pos);
        events.push(MobEvent::LandingDust {
            state,
            at: [at.x, at.y, at.z],
            count,
            // `sendParticles(..., 0.15)`: the crumbs fly up a little.
            speed: f64::from(0.15_f32),
        });
    }
    /// `Entity.getFluidInteractionBox` and the fluids in it: the box less 0.001,
    /// then the ridden boat's say (`AbstractBoat.modifyPassengerFluidInteractionBox`).
    /// A boat that isn't underwater keeps only what lies above its top, and
    /// all of a passenger it reaches the head of, so a mob sitting in a boat
    /// on a lake is neither in the water (it burns in daylight, isn't pushed
    /// by the flow) nor splashes.
    fn fluid_interaction(&self, w: &MobWorld, boat: Option<crate::motion::RiddenBoat>) -> Fluids {
        let b = self.bounds();
        let Some(boat) = boat.filter(|boat| !boat.under_water) else {
            return w.fluids(b, self.eye_y());
        };
        // `fluids` deflates the box by 0.001 itself: the deflated top against
        // the boat's, and the deflated bottom raised to it, as vanilla does.
        let (lo, hi) = (b.min_position(), b.max_position());
        if boat.max_y >= hi.y - 0.001 {
            return Fluids::default();
        }
        let lo = Vec3::new(lo.x, lo.y.max(boat.max_y - 0.001), lo.z);
        w.fluids_from(Aabb::new(lo, hi), b.min_position().y, self.eye_y())
    }
    /// Entity.updateFluidInteraction: the fluid state, then touching water
    /// resets the fall and entering it (after the first tick) splashes.
    pub(super) fn update_fluids(&mut self, w: &MobWorld, events: &mut Vec<MobEvent>) {
        let was_touching_water = self.fluids.in_water;
        let boat = self
            .fluids
            .boat
            .filter(|(id, _)| self.extra.mount == Some(*id));
        self.fluids = self.fluid_interaction(w, boat.map(|(_, b)| b));
        self.fluids.boat = boat;
        // EntityFluidInteraction.applyCurrentTo: the flow pushes the mob, but
        // `Shulker.setDeltaMovement` drops the push.
        if (self.fluids.in_water || self.fluids.in_lava)
            && self.pushed_by_fluid()
            && self.species() != super::Species::Shulker
        {
            self.velocity = w.pushed_by_currents(self.bounds(), self.velocity);
        }
        if self.fluids.in_water {
            self.fall_distance = 0.0;
            if !was_touching_water && !self.first_tick {
                // The splash is the mob's own: Sunrise's mobs carry no
                // controlling passenger here.
                let _ = sunrise_world::water_splash::water_splash(
                    &mut Splash { mob: self, events },
                    1_000_000,
                );
            }
        }
    }
    /// Entity.applyMovementEmissionAndPlaySound: walked distance, footsteps
    /// and swimming sounds.
    pub fn movement_emission(&mut self, w: &MobWorld, moved: Vec3, events: &mut Vec<MobEvent>) {
        if !super::ai::emits_movement(self) || self.extra.mount.is_some() {
            return;
        }
        let full = (moved.length() * f64::from(0.6_f32)) as f32;
        let horizontal =
            ((moved.x * moved.x + moved.z * moved.z).sqrt() * f64::from(0.6_f32)) as f32;
        let supporting = self.ground.supporting_block;
        let on = self.supported_on_pos(w, supporting, 1.0e-5);
        let state = w.world.block(on[0], on[1], on[2]);
        let climbable = w.facts.get(state).name.as_ref() == "minecraft:powder_snow"
            || w.state_flag(state, BlockTagFlags::CLIMBABLE);
        let steps = &mut self.ai.steps;
        steps.move_dist += if climbable { full } else { horizontal };
        steps.fly_dist += full;
        if steps.move_dist > steps.next_step && !w.facts.is_air(state) {
            let legacy = self.supported_on_pos(w, supporting, 0.2);
            let legacy_state = w.world.block(legacy[0], legacy[1], legacy[2]);
            let produced = !w.facts.is_air(legacy_state)
                && (self.ground.on_ground || climbable)
                && !self.fluids_swimming();
            if produced {
                // `getMovementEmission().emitsSounds()`: the footfall is played
                // only for a type whose emission is `ALL`, while the step game
                // event below is sent by `EVENTS` types too.
                let heard = super::ai::movement_sounds(self);
                if super::equine::hoof_steps(self.species()) {
                    if heard && !self.silent {
                        super::equine::play_step(self, w, legacy, legacy_state, events);
                    }
                } else if let Some((sound, volume, pitch)) =
                    super::entity_sounds::step(self, w, legacy, legacy_state)
                    && heard
                    && !self.silent
                {
                    let at = [self.position.x, self.position.y, self.position.z];
                    events.push(match sound {
                        std::borrow::Cow::Borrowed(name) => {
                            MobEvent::Sound(name, at, volume, pitch)
                        }
                        std::borrow::Cow::Owned(name) => MobEvent::SoundId(name, at, volume, pitch),
                    });
                }
                // Entity.vibrationAndSoundEffectsFromBlock: the step's game
                // event, with the supporting block's state when the effect
                // block is another.
                events.push(MobEvent::GameEvent {
                    event: "minecraft:step",
                    at: [self.position.x, self.position.y, self.position.z],
                    source: self.id,
                    state: if legacy == on { legacy_state } else { state },
                });
                self.ai.steps.next_step = self.next_step();
            } else if self.fluids.in_water {
                self.ai.steps.next_step = self.next_step();
                // Entity.waterSwimSound, and SkeletonHorse.playSwimSound's
                // mapping of the volume it works out.
                let v = self.velocity;
                let speed =
                    (v.x * v.x * f64::from(0.2_f32) + v.y * v.y + v.z * v.z * f64::from(0.2_f32))
                        .sqrt() as f32;
                let volume = super::equine::swim_volume(self, 1.0_f32.min(speed * 0.35));
                if !super::ai::movement_sounds(self) {
                    return;
                }
                let pitch = 1.0 + (self.random.next_float() - self.random.next_float()) * 0.4;
                // Entity.playSwimSound asks getSwimSound for its sound even
                // when the mob is silent, and a skeleton horse counts there.
                let sound = super::ai::swim_sound(self);
                if !self.silent {
                    let at = [self.position.x, self.position.y, self.position.z];
                    events.push(MobEvent::Sound(sound, at, volume, pitch));
                }
            }
        } else if w.facts.is_air(state)
            && self.species() == super::Species::Bee
            && !self.ground.on_ground
            && self.age.is_multiple_of(2)
        {
            // Entity.processFlappingMovement: a flying bee beats its wings on
            // every other tick (Bee.isFlapping, TICKS_PER_FLAP is 2).
            events.push(MobEvent::GameEvent {
                event: "minecraft:flap",
                at: [self.position.x, self.position.y, self.position.z],
                source: self.id,
                state: sunrise_world::registry::BlockStateId(0),
            });
        } else if w.facts.is_air(state) {
            // Entity.processFlappingMovement: onFlap runs before the game
            // event. Silent birds still emit it, independently of sound.
            let flapped = match self.species() {
                super::Species::Chicken => super::farm::chicken_process_flapping(self),
                super::Species::Parrot => super::parrot::process_flapping(self, events),
                _ => false,
            };
            if flapped {
                events.push(MobEvent::GameEvent {
                    event: "minecraft:flap",
                    at: [self.position.x, self.position.y, self.position.z],
                    source: self.id,
                    state: sunrise_world::registry::BlockStateId(0),
                });
            }
        }
    }
    /// `Entity.nextStep`: the next whole metre walked; a warden's steps come
    /// every 0.55, a strider's every 0.6, a turtle's every 0.15.
    fn next_step(&self) -> f32 {
        let walked = self.ai.steps.move_dist;
        if self.kind.species == super::Species::Warden {
            walked + 0.55
        } else if self.kind.species == super::Species::Strider {
            walked + 0.6
        } else if self.kind.species == super::Species::Turtle {
            walked + 0.15
        } else {
            (walked as i32).wrapping_add(1) as f32
        }
    }
    /// Entity.isSwimming (mobs never swim in the player sense).
    fn fluids_swimming(&self) -> bool {
        false
    }
    /// `Entity.checkSupportingBlock` uses the post-move bounds and live collision context.
    fn update_ground_support(&mut self, w: &MobWorld, moved: Vec3) {
        let mut ground = self.ground;
        let (context, subject) =
            self.support_collision(w.items.map(|items| items.entity_type_tags.as_ref()));
        let mut scene = SceneCollisions {
            world: w.world,
            binding: w.collision,
            subject: Some(subject),
        };
        let _ = scene.update_ground_support(
            &mut ground,
            self.position,
            self.bounds(),
            Some(moved),
            context,
        );
        self.ground = ground;
    }
    /// `Entity.getBlockStateOnLegacy`'s position: 0.2 below the feet, or the
    /// supporting block.
    pub(super) fn block_on_legacy(&self, w: &MobWorld) -> [i32; 3] {
        self.supported_on_pos(w, self.ground.supporting_block, 0.2)
    }
    /// `Entity.getOnPos(offset)` with a supporting block: its column (its
    /// own position for fences, walls and gates up to half a block down).
    fn supported_on_pos(
        &self,
        w: &MobWorld,
        supporting: Option<BlockPos>,
        offset: f32,
    ) -> [i32; 3] {
        let Some(p) = supporting else {
            return self.on_pos(offset);
        };
        if offset <= 1.0e-5 {
            return [p.x, p.y, p.z];
        }
        let name = &w.facts.get(w.world.block(p.x, p.y, p.z)).name;
        let tall = [
            "minecraft:fences",
            "minecraft:walls",
            "minecraft:fence_gates",
        ]
        .iter()
        .any(|t| w.has_tag(t, name));
        if offset <= 0.5 && tall {
            [p.x, p.y, p.z]
        } else {
            [
                p.x,
                (self.position.y - f64::from(offset)).floor() as i32,
                p.z,
            ]
        }
    }
    /// Entity.getOnPos(offset) without a supporting block record.
    pub fn on_pos(&self, offset: f32) -> [i32; 3] {
        [
            self.position.x.floor() as i32,
            (self.position.y - f64::from(offset)).floor() as i32,
            self.position.z.floor() as i32,
        ]
    }
    /// `Block.fallOn` of the block under the mob (`Entity.checkFallDamage`):
    /// the block's own rule (hay, honey, slime, powder snow, a bed, a
    /// stalagmite, farmland, a turtle egg) decides how much of the fall hurts.
    fn fall_on_block(
        &mut self,
        w: &MobWorld,
        level: &mut DamageLevel<'_>,
        (position, state): (BlockPos, BlockStateId),
        events: &mut Vec<MobEvent>,
        owner: LandingOwner,
    ) -> Option<FallPause> {
        let pos = [position.x, position.y, position.z];
        let species = self.species();
        let entity = if super::doors::is_zombie(species) {
            LandingEntityKind::Zombie
        } else if species == super::Species::Turtle {
            LandingEntityKind::Turtle
        } else if species == super::Species::Bat {
            LandingEntityKind::Bat
        } else {
            LandingEntityKind::Living
        };
        let distance = self.fall_distance;
        let result = w.landing.begin_fall_on(
            &mut super::landing::MobLanding {
                mob: self,
                w,
                level,
                events,
                pos,
            },
            state,
            position,
            entity,
            distance,
            owner != LandingOwner::Synchronous,
        );
        let call = match result {
            Ok(BlockFall::Done) => return None,
            Ok(BlockFall::Trample(trample)) => return Some(FallPause::Trample(trample)),
            Ok(BlockFall::Damage(call)) => Some(call),
            // A block outside the landing registry falls back to a plain fall.
            Err(_) => None,
        };
        self.start_block_fall(
            w,
            level,
            events,
            pos,
            call,
            distance,
            owner == LandingOwner::Borrowed,
        )
    }

    fn start_block_fall(
        &mut self,
        w: &MobWorld,
        level: &mut DamageLevel<'_>,
        events: &mut Vec<MobEvent>,
        pos: [i32; 3],
        call: Option<FallDamageCall>,
        distance: f64,
        handoff: bool,
    ) -> Option<FallPause> {
        let mut landing = super::landing::MobLanding {
            mob: self,
            w,
            level,
            events,
            pos,
        };
        let (distance, modifier, source) = landing.resolve_fall(call, distance);
        let tail = call.map(FallDamageCall::tail).unwrap_or_default();
        let result = self.begin_fall_hurt(
            w.loop_context,
            level,
            distance,
            modifier,
            events,
            Some((w, pos)),
            source,
            handoff,
        );
        self.finish_block_fall(w, level, events, FallenOn { pos, tail }, result)
    }

    fn finish_block_fall(
        &mut self,
        w: &MobWorld,
        level: &mut DamageLevel<'_>,
        events: &mut Vec<MobEvent>,
        block: FallenOn,
        result: Result<bool, FallHurtStop>,
    ) -> Option<FallPause> {
        match result {
            Err(stop) => Some(FallPause::Hurt { block, stop }),
            Ok(damaged) => {
                let _ = block.tail.after_hurt(
                    &mut super::landing::MobLanding {
                        mob: self,
                        w,
                        level,
                        events,
                        pos: block.pos,
                    },
                    damaged,
                );
                None
            }
        }
    }

    fn resume_fall(
        &mut self,
        w: &MobWorld,
        level: &mut DamageLevel<'_>,
        events: &mut Vec<MobEvent>,
        fall: FallPause,
    ) -> Option<FallPause> {
        let next = match fall {
            FallPause::Trample(trample) => {
                let p = trample.position;
                self.start_block_fall(
                    w,
                    level,
                    events,
                    [p.x, p.y, p.z],
                    Some(trample.damage_call()),
                    trample.distance,
                    true,
                )
            }
            FallPause::Hurt { block, stop } => {
                let contact = Some((w, block.pos));
                let result = self.resume_fall_hurt(level, events, contact, stop, true);
                self.finish_block_fall(w, level, events, block, result)
            }
        };
        if next.is_none() {
            self.fall_distance = 0.0;
        }
        next
    }

    /// Continue the mount's fall after the level handled its pending action.
    pub fn resume_client_moved_fall(
        &mut self,
        w: &MobWorld,
        level: &mut DamageLevel<'_>,
        events: &mut Vec<MobEvent>,
        fall: ClientMovedFall,
    ) -> Option<ClientMovedFall> {
        self.resume_fall(w, level, events, fall.0)
            .map(ClientMovedFall)
    }
    fn mod_block_fall(
        &self,
        level: &mut DamageLevel<'_>,
        contact: Option<(&MobWorld, [i32; 3])>,
        distance: f64,
        multiplier: f32,
    ) -> (f64, f32) {
        contact.map_or((distance, multiplier), |(w, p)| {
            crate::block_contact::fall(
                level,
                crate::block_contact::Contact {
                    entity: self.id as i32,
                    kind: &self.kind.id,
                    state: w.world.block(p[0], p[1], p[2]),
                    position: sunrise_core::position::BlockPos::new(p[0], p[1], p[2]),
                },
                distance,
                multiplier,
            )
        })
    }
    /// `LivingEntity.causeFallDamage`: a fall of `distance` blocks at `modifier`
    /// (its own landing, or a vehicle's passed on by `propagateFallToPassengers`)
    /// brings the species' sounds and reactions, the damage, and the fall
    /// handed to whoever rides it. `AbstractHorse.causeFallDamage` hurts the
    /// horse first and passes the fall on only when it was hurt.
    pub fn cause_fall_damage_from(
        &mut self,
        context: &LoopContext,
        level: &mut DamageLevel<'_>,
        distance: f64,
        modifier: f32,
        events: &mut Vec<MobEvent>,
    ) {
        self.cause_fall_damage_at(
            context,
            level,
            distance,
            modifier,
            events,
            None,
            super::DamageKind::Fall,
        );
    }
    /// The owner-driven `causeFallDamage` of a passenger, including nested passengers.
    pub fn passenger_fall_until_pause(
        &mut self,
        context: &LoopContext,
        level: &mut DamageLevel<'_>,
        events: &mut Vec<MobEvent>,
        fall: PassengerFall,
    ) -> Option<PassengerFallPause> {
        self.begin_fall_hurt(
            context,
            level,
            fall.distance,
            fall.modifier,
            events,
            None,
            super::DamageKind::Fall,
            true,
        )
        .err()
        .map(PassengerFallPause)
    }

    /// Continue a passenger's fall after the owning level carried out the pending action.
    pub fn resume_passenger_fall(
        &mut self,
        level: &mut DamageLevel<'_>,
        events: &mut Vec<MobEvent>,
        pause: PassengerFallPause,
    ) -> Option<PassengerFallPause> {
        self.resume_fall_hurt(level, events, None, pause.0, true)
            .err()
            .map(PassengerFallPause)
    }

    /// `getAttributeValue(SAFE_FALL_DISTANCE)`, which `calculateFallPower`
    /// takes off a fall: Jump Boost's stored modifier among the attribute's
    /// ([`Mob::set_effect_modifiers`]).
    fn safe_fall_distance(&self) -> f64 {
        super::finalize::value(self, super::finalize::Attr::SafeFallDistance)
    }
    /// True when the fall hurt (`causeFallDamage`'s result).
    pub(super) fn cause_fall_damage_at(
        &mut self,
        context: &LoopContext,
        level: &mut DamageLevel<'_>,
        distance: f64,
        modifier: f32,
        events: &mut Vec<MobEvent>,
        contact: Option<(&MobWorld, [i32; 3])>,
        source: super::DamageKind,
    ) -> bool {
        match self.begin_fall_hurt(
            context, level, distance, modifier, events, contact, source, false,
        ) {
            Ok(hurt) => hurt,
            Err(_) => unreachable!("a synchronous fall never yields to an owner"),
        }
    }

    fn begin_fall_hurt(
        &mut self,
        context: &LoopContext,
        level: &mut DamageLevel<'_>,
        distance: f64,
        modifier: f32,
        events: &mut Vec<MobEvent>,
        contact: Option<(&MobWorld, [i32; 3])>,
        source: super::DamageKind,
        handoff: bool,
    ) -> Result<bool, FallHurtStop> {
        // Breeze.causeFallDamage: a landing from higher than three blocks.
        if self.species() == super::Species::Breeze && distance > 3.0 {
            let at = [self.position.x, self.position.y, self.position.z];
            events.push(MobEvent::Sound("entity.breeze.land", at, 1.0, 1.0));
        }
        // Creeper.causeFallDamage: the fall swells the fuse, up to five ticks
        // short of exploding.
        if self.species() == super::Species::Creeper {
            self.swell = (self.swell + (distance * 1.5) as i32).min(self.max_swell - 5);
        }
        // AbstractHorse.causeFallDamage (llamas override it): a landing of more
        // than a block thuds, and the horse's fall makes no generic fall sound.
        let thud =
            super::equine::is_equine(self.species()) && !super::llama::is_llama(self.species());
        if thud && distance > 1.0 && !self.silent {
            let at = [self.position.x, self.position.y, self.position.z];
            let sound = if self.is_baby() {
                "entity.baby_horse.land"
            } else {
                "entity.horse.land"
            };
            events.push(MobEvent::Sound(sound, at, 0.4, 1.0));
        }
        // AbstractHorse.causeFallDamage hurts the horse first and passes the fall
        // on to its riders only when it was hurt.
        let horse = super::equine::is_equine(self.species());
        // `Block.fallOn` is told the fall as it was before the mods' changes.
        let landed = distance;
        let mut distance = distance;
        let mut modifier = modifier;
        if !horse {
            let mut cancel = false;
            crate::landing::native_living_fall(
                context,
                self.id,
                &self.kind.id,
                &mut distance,
                &mut modifier,
                &mut cancel,
            );
            if cancel {
                self.mod_block_fall(level, contact, landed, 0.0);
                return Ok(false);
            }
        }
        // Entity.causeFallDamage: nothing happens to a fall immune entity, nor
        // to those on it.
        if self.kind.flags.fall_immune {
            self.mod_block_fall(level, contact, landed, 0.0);
            return Ok(false);
        }
        let (distance, modifier) = self.mod_block_fall(level, contact, distance, modifier);
        let fall = PassengerFall { distance, modifier };
        // LivingEntity.causeFallDamage lets the riders fall first.
        if !horse && self.fall_to_passengers(fall, events, handoff) {
            return Err(FallHurtStop::BeforeOwnHurt(fall, source));
        }
        self.hurt_from_fall(level, fall, events, contact, source, horse, handoff)
    }

    fn hurt_from_fall(
        &mut self,
        level: &mut DamageLevel<'_>,
        fall: PassengerFall,
        events: &mut Vec<MobEvent>,
        contact: Option<(&MobWorld, [i32; 3])>,
        source: super::DamageKind,
        horse: bool,
        handoff: bool,
    ) -> Result<bool, FallHurtStop> {
        let PassengerFall { distance, modifier } = fall;
        let power = distance + 1.0e-6 - self.safe_fall_distance();
        let mut damage = (power * f64::from(modifier) * self.kind.fall_damage_multiplier).floor();
        // Frog.calculateFallDamage: five points less than the base damage;
        // Goat.calculateFallDamage: ten, so a long jump down a ledge is safe.
        match self.species() {
            super::Species::Frog => damage -= 5.0,
            super::Species::Goat => damage -= 10.0,
            _ => {}
        }

        if damage > 0.0 {
            // Llama.causeFallDamage: shorter damaging falls play the block
            // sound but do not hurt the llama or its riders.
            if super::llama::is_llama(self.species()) && distance < 6.0 {
                self.block_fall_sound(contact, events);
                return Ok(true);
            }
            let at = [self.position.x, self.position.y, self.position.z];
            // Monster.getFallSounds: the hostile pair.
            let monster = super::ai::is_monster(self.species());
            let sound = match (monster, damage > 4.0) {
                (true, true) => "entity.hostile.big_fall",
                (true, false) => "entity.hostile.small_fall",
                (false, true) => "entity.generic.big_fall",
                (false, false) => "entity.generic.small_fall",
            };
            // Horses thud and llamas make no generic fall sound.
            if !horse {
                events.push(MobEvent::Sound(sound, at, 1.0, 1.0));
                self.block_fall_sound(contact, events);
            }
            self.damage(level, damage as f32, super::Damage::of(source));
            if handoff && self.hurt_needs_owner() {
                return Err(FallHurtStop::AfterHurt(horse.then_some(fall)));
            }
            if horse {
                return self.finish_horse_fall(fall, events, contact, handoff);
            }
            return Ok(true);
        }
        Ok(false)
    }

    fn resume_fall_hurt(
        &mut self,
        level: &mut DamageLevel<'_>,
        events: &mut Vec<MobEvent>,
        contact: Option<(&MobWorld, [i32; 3])>,
        stop: FallHurtStop,
        handoff: bool,
    ) -> Result<bool, FallHurtStop> {
        match stop {
            FallHurtStop::BeforeOwnHurt(fall, source) => {
                self.hurt_from_fall(level, fall, events, contact, source, false, handoff)
            }
            FallHurtStop::AfterHurt(Some(fall)) => {
                self.finish_horse_fall(fall, events, contact, handoff)
            }
            FallHurtStop::AfterHurt(None) => Ok(true),
            FallHurtStop::AfterHorsePassengers(_) => {
                self.block_fall_sound(contact, events);
                Ok(true)
            }
        }
    }

    fn finish_horse_fall(
        &self,
        fall: PassengerFall,
        events: &mut Vec<MobEvent>,
        contact: Option<(&MobWorld, [i32; 3])>,
        handoff: bool,
    ) -> Result<bool, FallHurtStop> {
        if self.fall_to_passengers(fall, events, handoff) {
            return Err(FallHurtStop::AfterHorsePassengers(fall));
        }
        self.block_fall_sound(contact, events);
        Ok(true)
    }
    /// `LivingEntity.playBlockFallSound`: the block 0.2 below the feet plays
    /// its fall sound at half its volume and three quarters of its pitch,
    /// unless the mob is silent or the block is air.
    fn block_fall_sound(&self, contact: Option<(&MobWorld, [i32; 3])>, events: &mut Vec<MobEvent>) {
        if self.silent {
            return;
        }
        if let Some((w, _)) = contact {
            let pos = self.on_pos(0.2);
            let state = w.world.block(pos[0], pos[1], pos[2]);
            if !w.facts.is_air(state) {
                let sounds = w.block_sound_type(self, pos, state, mod_api::BlockSoundCause::Fall);
                let at = [self.position.x, self.position.y, self.position.z];
                events.push(MobEvent::SoundId(
                    sounds.fall.as_str().to_owned(),
                    at,
                    sounds.volume * 0.5,
                    sounds.pitch * 0.75,
                ));
            }
        }
    }
    /// `Entity.propagateFallToPassengers`: return the loan at this exact call
    /// if a passenger needs the Game. Synchronous adapters still emit their event.
    fn fall_to_passengers(
        &self,
        fall: PassengerFall,
        events: &mut Vec<MobEvent>,
        handoff: bool,
    ) -> bool {
        if self.rider.is_none() && self.extra.rider.is_none() {
            return false;
        }
        if handoff {
            return true;
        }
        events.push(MobEvent::FallToPassengers {
            mob: self.id,
            distance: fall.distance,
            modifier: fall.modifier,
        });
        false
    }
    /// LivingEntity.getFrictionInfluencedSpeed.
    fn friction_speed(&self, w: &MobWorld, friction: f32) -> f32 {
        if self.ground.on_ground {
            let speed = self.speed(w);
            if friction > 0.6 {
                speed * (0.216_000_02 / (friction * friction * friction))
            } else {
                speed
            }
        } else {
            self.flying_speed(w.loop_context, w.side)
        }
    }
    /// `LivingEntity.getSpeed`, through the `living_speed` hook.
    fn speed(&self, w: &MobWorld) -> f32 {
        let mut speed = self.ai.speed;
        let side = w.side;
        let reads = || side.hit_reads();
        let living = crate::health::LivingLend::mob(self, &reads);
        crate::motion::native_living_speed(w.loop_context, living, &mut speed);
        speed
    }
    /// `LivingEntity.getFlyingSpeed`, through the `living_flying_speed` hook, whose view reads
    /// `side`'s tables.
    fn flying_speed(
        &self,
        context: &sunrise_world::native_load_context::LoopContext,
        side: &dyn crate::motion::SideLevel,
    ) -> f32 {
        let mut speed = if super::equine::controlled(self) {
            self.ai.speed * 0.1
        } else {
            0.02
        };
        let reads = || side.hit_reads();
        let living = crate::health::LivingLend::mob(self, &reads);
        crate::motion::native_living_flying_speed(context, living, &mut speed);
        speed
    }
    /// `LivingEntity.getEffectiveGravity`: the gravity, floored to 0.01 while a
    /// Slow Falling mob is on its way down.
    pub fn effective_gravity(&self) -> f64 {
        let gravity = self.gravity();
        if self.velocity.y <= 0.0 && self.has_effect("minecraft:slow_falling") && !gravity.is_nan()
        {
            gravity.min(0.01)
        } else {
            gravity
        }
    }
    /// Entity.getGravity: none while isNoGravity.
    pub fn gravity(&self) -> f64 {
        if self.no_gravity {
            0.0
        } else {
            self.kind.gravity
        }
    }
    /// LivingEntity.travel.
    pub fn travel(
        &mut self,
        w: &MobWorld,
        level: &mut DamageLevel<'_>,
        input: Vec3,
        events: &mut Vec<MobEvent>,
    ) {
        let _ = self.travel_until_pause(w, level, input, events, false);
    }

    /// `LivingEntity.travelFallFlying`: use the mob's existing authoritative move
    /// and damage routes, with the shared `updateFallFlyingMovement` arithmetic.
    fn travel_fall_flying(
        &mut self,
        w: &MobWorld,
        level: &mut DamageLevel<'_>,
        events: &mut Vec<MobEvent>,
        pause: bool,
    ) -> ControlFlow<TravelPause> {
        let before = (self.velocity.x * self.velocity.x + self.velocity.z * self.velocity.z).sqrt();
        self.set_velocity(crate::motion::fall_flying_movement(
            self.velocity,
            self.head_pitch,
            self.yaw,
            self.effective_gravity(),
        ));
        self.travel_move_with(
            w,
            level,
            events,
            pause,
            sunrise_world::travel::TravelMode::Glide,
            TravelTail::FallFlying { before },
        )
    }
    /// LivingEntity.travelFlying: no gravity, `0.91F` drag in the air,
    /// `0.8F` in water and 0.5 in lava. A happy ghast swims as fast as it
    /// flies (`HappyGhast.travel`, at its `flying_speed` attribute); the others
    /// at 0.02.
    fn travel_flying(
        &mut self,
        w: &MobWorld,
        level: &mut DamageLevel<'_>,
        input: Vec3,
        events: &mut Vec<MobEvent>,
        pause: bool,
    ) -> ControlFlow<TravelPause> {
        let (fluid_speed, air_speed) = if self.species() == super::Species::HappyGhast {
            let speed = super::happy_ghast::travel_speed(self);
            (speed, speed)
        } else {
            (0.02, self.kind.air_speed)
        };
        self.travel_flying_speeds(
            w,
            level,
            input,
            events,
            fluid_speed,
            fluid_speed,
            air_speed,
            pause,
        )
    }
    /// `LivingEntity.travelFlying(input, waterSpeed, lavaSpeed, airSpeed)`:
    /// the speeds vary per mob (an allay flies at the speed its move control
    /// set, `Allay.travel`).
    pub(crate) fn travel_flying_speeds(
        &mut self,
        w: &MobWorld,
        level: &mut DamageLevel<'_>,
        input: Vec3,
        events: &mut Vec<MobEvent>,
        water_speed: f32,
        lava_speed: f32,
        air_speed: f32,
        pause: bool,
    ) -> ControlFlow<TravelPause> {
        let (speed, drag) = if self.fluids.in_water {
            (water_speed, f64::from(0.8_f32))
        } else if self.fluids.in_lava {
            (lava_speed, 0.5)
        } else {
            (air_speed, f64::from(0.91_f32))
        };
        self.move_relative(speed, input);
        self.travel_move_with(
            w,
            level,
            events,
            pause,
            sunrise_world::travel::TravelMode::Flying,
            TravelTail::Flying { drag },
        )
    }

    /// Continue immediately unless movement needs the owning game. Only a
    /// suspended move retains a tail; ordinary movement does not allocate.
    pub(super) fn travel_move(
        &mut self,
        w: &MobWorld,
        level: &mut DamageLevel<'_>,
        events: &mut Vec<MobEvent>,
        pause: bool,
        mode: sunrise_world::travel::TravelMode,
        tail: TravelFinish,
    ) -> ControlFlow<TravelPause> {
        self.travel_move_with(w, level, events, pause, mode, TravelTail::Fixed(tail))
    }

    fn travel_move_with(
        &mut self,
        w: &MobWorld,
        level: &mut DamageLevel<'_>,
        events: &mut Vec<MobEvent>,
        pause: bool,
        mode: sunrise_world::travel::TravelMode,
        tail: TravelTail,
    ) -> ControlFlow<TravelPause> {
        let start = TravelStart {
            mode,
            position: self.position,
            on_ground: self.ground.on_ground,
        };
        if let Some(movement) = self.move_until_pause(w, level, events, &mut MoveGate::new(), pause)
        {
            return ControlFlow::Break(TravelPause(TravelStop::Move {
                movement,
                tail,
                start,
            }));
        }
        self.fire_travel_moved(w, start);
        tail.finish(self, w, level, events, pause)
    }

    /// `living_travel_moved` (movement phase 4, #5074): the branch's own move is done and its
    /// drag and gravity are not. A listener's finite velocity is what they act on.
    fn fire_travel_moved(&mut self, w: &MobWorld, start: TravelStart) {
        if self.removed {
            return;
        }
        let mut velocity = self.velocity;
        crate::travel_gate::moved_controlled(
            w.loop_context,
            self.id,
            &self.kind.id,
            || start.mode,
            crate::travel_gate::Moved {
                moved: self.position.subtract(start.position),
                on_ground_before: start.on_ground,
                on_ground: self.ground.on_ground,
                jumping: self.ai.jumping,
            },
            &mut velocity,
            super::equine::controlled(self),
        );
        if velocity != self.velocity {
            self.set_velocity(velocity);
        }
    }
    /// `Entity.getBlockPosBelowThatAffectsMyMovement` for air travel and restitution.
    fn movement_support_position(&self, w: &MobWorld) -> BlockPos {
        crate::motion::block_support_at(
            w.world,
            Some(w.movement),
            self.position,
            self.ground.supporting_block,
            sunrise_world::movement_support::MOVEMENT_SUPPORT_OFFSET,
        )
    }
    fn block_friction(&self, w: &MobWorld) -> f32 {
        let BlockPos { x, y, z } = self.movement_support_position(w);
        self.friction_at(w, [x, y, z])
    }
    /// The friction of the block at `[x, y, z]` under the mob, as the block's parts answer
    /// (`IBlockExtension.getFriction`).
    pub(super) fn friction_at(&self, w: &MobWorld, [x, y, z]: [i32; 3]) -> f32 {
        crate::motion::block_friction(
            w.loop_context,
            w.side,
            (self.id, &self.kind.id),
            w.world,
            w.registry,
            w.facts,
            BlockPos::new(x, y, z),
        )
    }
    /// `getAttributeValue(BOUNCINESS)`.
    fn entity_bounciness(&self) -> f64 {
        super::sulfur_cube::physics(self).map_or(0.0, |a| f64::from(a.bounciness.clamp(0.0, 1.0)))
    }
    /// `LivingEntity.computeModifiedFriction`.
    fn modified_friction(friction: f32, modifier: f32) -> f32 {
        (1.0 - (1.0 - friction) * modifier).clamp(0.0, 1.0)
    }
    /// `LivingEntity.getAirDrag`: what vertical air drag a bounce applies.
    fn air_drag(&self) -> f32 {
        let base = if self.kind.flags.omnidirectional {
            0.91
        } else {
            0.98
        };
        Self::modified_friction(base, self.air_drag_modifier())
    }
    /// `getAttributeValue(AIR_DRAG_MODIFIER)`.
    fn air_drag_modifier(&self) -> f32 {
        super::sulfur_cube::physics(self).map_or(1.0, |a| a.air_drag_modifier.clamp(0.0, 2048.0))
    }
    /// `LivingEntity.handleOnClimbable` for a mob, which is never a `Player`
    /// that holds still on a ladder: the move across clamped to 0.15 either
    /// way and the fall to 0.15, both the float 0.15F (`Mth.clamp`,
    /// `Math.max`, which keep a NaN).
    fn climbable_velocity(v: Vec3) -> Vec3 {
        let limit = f64::from(0.15f32);
        let y = if v.y.is_nan() { v.y } else { v.y.max(-limit) };
        Vec3::new(v.x.clamp(-limit, limit), y, v.z.clamp(-limit, limit))
    }
    fn travel_in_air(
        &mut self,
        w: &MobWorld,
        level: &mut DamageLevel<'_>,
        input: Vec3,
        events: &mut Vec<MobEvent>,
        pause: bool,
        stop_gliding: bool,
    ) -> ControlFlow<TravelPause> {
        let friction = if self.ground.on_ground {
            // FRICTION_MODIFIER scales the block's slipperiness.
            let modifier = super::sulfur_cube::physics(self)
                .map_or(1.0, |a| a.friction_modifier.clamp(0.0, 2048.0));
            Self::modified_friction(self.block_friction(w), modifier)
        } else {
            1.0
        };
        let speed = self.friction_speed(w, friction);
        self.move_relative(speed, input);
        // LivingEntity.handleRelativeFrictionAndCalculateMovement:
        // handleOnClimbable, the move, then the climb up a wall.
        if self.on_climbable(w) {
            self.fall_distance = 0.0;
            self.set_velocity(Self::climbable_velocity(self.velocity));
        }
        self.travel_move_with(
            w,
            level,
            events,
            pause,
            sunrise_world::travel::TravelMode::Air,
            TravelTail::Air {
                friction,
                stop_gliding,
            },
        )
    }

    fn finish_air_travel(&mut self, w: &MobWorld, friction: f32) {
        let mut movement = self.velocity;
        if (self.ground.horizontal_collision || self.ai.jumping) && self.on_climbable(w) {
            movement.y = 0.2;
        }
        let mut y = movement.y;
        // LivingEntity.travelInAir: levitation lifts by its amplifier.
        if let Some(&super::StoredEffect { amplifier: amp, .. }) = self
            .extra
            .effects
            .iter()
            .find(|e| e.id == "minecraft:levitation")
        {
            y += (0.05 * f64::from(amp + 1) - movement.y) * 0.2;
        } else {
            y -= self.effective_gravity();
        }
        // LivingEntity.shouldDiscardFriction (a breeze's, a frog's or a goat's
        // long jump).
        if super::breeze::discards_friction(self)
            || self.extra.frog.discard_friction
            || self.extra.goat.discard_friction
        {
            self.set_velocity(Vec3::new(movement.x, y, movement.z));
            return;
        }
        let air_drag_modifier = self.air_drag_modifier();
        let drag = Self::modified_friction(0.91, air_drag_modifier);
        let f = friction * drag;
        let vertical = if self.kind.flags.omnidirectional {
            drag
        } else {
            Self::modified_friction(0.98, air_drag_modifier)
        };
        self.set_velocity(Vec3::new(
            movement.x * f64::from(f),
            y * f64::from(vertical),
            movement.z * f64::from(f),
        ));
    }
    fn travel_in_fluid(
        &mut self,
        w: &MobWorld,
        level: &mut DamageLevel<'_>,
        input: Vec3,
        events: &mut Vec<MobEvent>,
        pause: bool,
        water: bool,
    ) -> ControlFlow<TravelPause> {
        let falling = self.velocity.y <= 0.0;
        let old_y = self.position.y;
        let gravity = self.effective_gravity();
        let water_slow = if water {
            let mut slow = if self.extra.sprinting {
                0.9
            } else {
                self.kind.water_slowdown
            };
            let mut speed = 0.02f32;
            let mut efficiency = self.kind.water_efficiency as f32;
            if !self.ground.on_ground {
                efficiency *= 0.5;
            }
            if efficiency > 0.0 {
                slow += (0.546_000_06 - slow) * efficiency;
                speed += (self.speed(w) - speed) * efficiency;
            }
            self.move_relative(speed, input);
            Some(slow)
        } else {
            self.move_relative(0.02, input);
            None
        };
        self.travel_move_with(
            w,
            level,
            events,
            pause,
            if water {
                sunrise_world::travel::TravelMode::Water
            } else {
                sunrise_world::travel::TravelMode::Lava
            },
            TravelTail::Fluid {
                falling,
                old_y,
                gravity,
                water_slow,
            },
        )
    }

    fn finish_fluid_travel(
        &mut self,
        w: &MobWorld,
        falling: bool,
        old_y: f64,
        gravity: f64,
        water_slow: Option<f32>,
    ) {
        if let Some(slow) = water_slow {
            let mut m = self.velocity;
            // LivingEntity.travelInWater: `horizontalCollision && onClimbable()`,
            // a ladder or scaffolding under water as well as a climbing spider.
            if self.ground.horizontal_collision && self.on_climbable(w) {
                m.y = 0.2;
            }
            m = Vec3::new(
                m.x * f64::from(slow),
                m.y * f64::from(0.8_f32),
                m.z * f64::from(slow),
            );
            self.set_velocity(fluid_falling(gravity, falling, self.extra.sprinting, m));
        } else {
            if self.fluids.lava <= self.fluid_jump_threshold() {
                self.set_velocity(Vec3::new(
                    self.velocity.x * 0.5,
                    self.velocity.y * f64::from(0.8_f32),
                    self.velocity.z * 0.5,
                ));
                self.set_velocity(fluid_falling(
                    gravity,
                    falling,
                    self.extra.sprinting,
                    self.velocity,
                ));
            } else {
                self.set_velocity(Vec3::new(
                    self.velocity.x * 0.5,
                    self.velocity.y * 0.5,
                    self.velocity.z * 0.5,
                ));
            }
            if gravity != 0.0 {
                self.set_velocity(
                    self.velocity
                        .with_component(Axis::Y, self.velocity.y - gravity / 4.0),
                );
            }
        }
        // jumpOutOfFluid: Entity.isFree wants the box dry and clear of the
        // entities a mob collides with (canBeCollidedWith: living shulkers).
        let m = self.velocity;
        let ahead = self.bounds().move_by(Vec3::new(
            m.x,
            m.y + f64::from(0.6_f32) - self.position.y + old_y,
            m.z,
        ));
        let wide = ahead.inflate(Vec3::new(1.0e-7, 1.0e-7, 1.0e-7));
        if self.ground.horizontal_collision
            && w.no_collision(ahead)
            && !w.contains_any_liquid(ahead)
            && !w.others_near(wide).any(|s| {
                s.id != self.id
                    && s.alive
                    && s.species == super::Species::Shulker
                    && s.bounds.intersects(wide)
            })
        {
            self.set_velocity(self.velocity.with_component(Axis::Y, f64::from(0.3_f32)));
        }
        // `LivingEntity.travelInFluid` ends with
        // `floatInLiquidWhileRidden(FluidTags.ENTITY_FLOATABLE)`: a mount with
        // a passenger that is in `minecraft:can_float_while_ridden` and deeper
        // in the water than its jump threshold rises 0.04 a tick, which is
        // what keeps a ridden horse and its rider afloat.
        if (self.rider.is_some() || self.extra.rider.is_some())
            && w.entity_tag_bit(
                &self.kind,
                crate::items::EntityTypeTags::CAN_FLOAT_WHILE_RIDDEN,
            )
            && self.fluids.water > self.fluid_jump_threshold()
        {
            self.set_velocity(
                self.velocity
                    .with_component(Axis::Y, self.velocity.y + f64::from(0.04_f32)),
            );
        }
    }
    /// The jump part of LivingEntity.aiStep.
    pub(crate) fn handle_jumping(&mut self, w: &MobWorld) {
        if self.ai.jumping && self.affected_by_fluids() {
            let (fluid_height, in_water_with_height) = if self.fluids.in_lava {
                (self.fluids.lava, false)
            } else {
                (
                    self.fluids.water,
                    self.fluids.in_water && self.fluids.water > 0.0,
                )
            };
            let threshold = self.fluid_jump_threshold();
            if !in_water_with_height || self.ground.on_ground && fluid_height <= threshold {
                if !self.fluids.in_lava || self.ground.on_ground && self.fluids.lava <= threshold {
                    if (self.ground.on_ground || in_water_with_height && fluid_height <= threshold)
                        && self.ai.no_jump_delay == 0
                    {
                        self.jump_from_ground(w);
                        self.ai.no_jump_delay = 10;
                    }
                } else {
                    self.jump_in_liquid(true);
                }
            } else {
                self.jump_in_liquid(false);
            }
        } else {
            self.ai.no_jump_delay = 0;
        }
    }
    /// Mob.jumpInLiquid: floaters rise 0.04 per tick, others 0.3; bees 0.01.
    /// A magma cube in lava rises to `0.22 + size * 0.05` instead
    /// (`MagmaCube.jumpInLiquid`).
    fn jump_in_liquid(&mut self, lava: bool) {
        if lava && self.species() == super::Species::MagmaCube {
            self.velocity.y = f64::from(0.22_f32 + self.extra.size as f32 * 0.05);
            self.needs_sync = true;
            return;
        }
        self.velocity.y += if self.species() == super::Species::Bee {
            0.01
        } else if self.ai.nav.can_float {
            // LivingEntity.jumpInLiquid adds 0.04F.
            f64::from(0.04_f32)
        } else {
            0.3
        };
    }
    pub fn jump_from_ground(&mut self, w: &MobWorld) {
        // LivingEntity.jumpFromGround (and the rabbit's and cubes' own,
        // which call it or do the same): needsSync.
        self.needs_sync = true;
        if self.species() == super::Species::Rabbit {
            super::rabbit::jump(self, w);
            return;
        }
        if matches!(
            self.species(),
            super::Species::Slime | super::Species::MagmaCube | super::Species::SulfurCube
        ) {
            super::cube::jump(self, w);
            self.living_jumped(w.loop_context);
            return;
        }
        let power = self.jump_power(w);
        if power > 1.0e-5 {
            self.velocity.y = f64::from(power).max(self.velocity.y);
            // A sprint jump (a dashing cat or ocelot) adds 0.2 along its yaw.
            if self.extra.sprinting {
                let angle = f64::from(self.yaw * trig::DEG_TO_RAD);
                self.velocity.x += f64::from(-trig::sin(angle)) * 0.2;
                self.velocity.z += f64::from(trig::cos(angle)) * 0.2;
            }
            // LivingEntity.jumpFromGround ends with NeoForge's LivingJumpEvent, before the
            // sniffer's own push below.
            self.living_jumped(w.loop_context);
        }
        // Sniffer.jumpFromGround: a hop the move control asked for, taken from
        // nearly standing still, carries a tenth of a block forward.
        if self.species() == super::Species::Sniffer
            && self.ai.mov.speed_modifier > 0.0
            && self.velocity.horizontal_distance_squared() < 0.01
        {
            self.move_relative(0.1, Vec3::new(0.0, 0.0, 1.0));
        }
    }
    /// The `living_jump` hook for this mob, with its motion after the jump.
    pub(crate) fn living_jumped(&self, context: &LoopContext) {
        let v = self.velocity;
        crate::motion::native_living_jump(context, self.id, &self.kind.id, [v.x, v.y, v.z]);
    }
    /// `LivingEntity.getJumpPower`: the jump power with a multiplier of 1.0.
    pub fn jump_power(&self, w: &MobWorld) -> f32 {
        self.jump_power_with(w, 1.0)
    }
    /// `LivingEntity.getJumpPower(float multiplier)`
    pub(crate) fn jump_power_with(&self, w: &MobWorld, multiplier: f32) -> f32 {
        let mut power = self.attrs.jump_strength as f32 * multiplier * self.block_jump_factor(w)
            + self.jump_boost_power();
        let side = w.side;
        let reads = || side.hit_reads();
        let living = crate::health::LivingLend::mob(self, &reads);
        crate::motion::native_living_jump_power(w.loop_context, living, &mut power);
        power
    }
    /// `LivingEntity.getJumpBoostPower`: amplifier + 1 times 0.1, or 0.0 without the effect.
    pub fn jump_boost_power(&self) -> f32 {
        self.extra
            .effects
            .iter()
            .find(|e| e.id == "minecraft:jump_boost")
            .map_or(0.0, |e| 0.1 * (e.amplifier + 1) as f32)
    }
    /// `Entity.getBlockJumpFactor`: the block the mob stands in, or the one
    /// below that affects movement when the current block has a neutral jump factor of 1.0.
    pub fn block_jump_factor(&self, w: &MobWorld) -> f32 {
        let supporting = self.ground.supporting_block;
        crate::motion::block_jump_factor_at(
            w.world,
            w.movement,
            w.loop_context,
            self.position,
            supporting,
        )
    }
    /// LivingEntity.updateWalkAnimation (client calculateEntityAnimation).
    pub(crate) fn update_walk_animation(&mut self) {
        let dy = if self.kind.flags.omnidirectional {
            self.position.y - self.previous.y
        } else {
            0.0
        };
        let dx = self.position.x - self.previous.x;
        let dz = self.position.z - self.previous.z;
        let distance = (dx * dx + dy * dy + dz * dz).sqrt() as f32;
        self.previous_limb_speed = self.limb_speed;
        if self.alive() {
            let (target, factor) = match self.species() {
                // Creaking.updateWalkAnimation walks the model faster.
                super::Species::Creaking => ((distance * 25.0).min(3.0), 0.4),
                super::Species::Frog => (super::frog::walk_target(self, distance), 0.4),
                s if super::camel::is_camel(s) => super::camel::walk_target(self, distance),
                _ => ((distance * 4.0).min(1.0), 0.4),
            };
            self.limb_speed += (target - self.limb_speed) * factor;
            self.limb_position += self.limb_speed;
        } else {
            self.limb_speed = 0.0;
        }
    }
}
impl crate::entity_rules::EntityRules for Mob {
    fn is_no_gravity(&self) -> bool {
        self.no_gravity
    }
}
/// LivingEntity.getFluidFallingAdjustedMovement.
fn fluid_falling(gravity: f64, falling: bool, sprinting: bool, m: Vec3) -> Vec3 {
    if gravity != 0.0 && !sprinting {
        let y = if falling && (m.y - 0.005).abs() >= 0.003 && (m.y - gravity / 16.0).abs() < 0.003 {
            -0.003
        } else {
            m.y - gravity / 16.0
        };
        Vec3::new(m.x, y, m.z)
    } else {
        m
    }
}

/// `CollisionGetter.findFreePosition`: within `allowed`, the point nearest
/// `preferred` at which a box of the size, centred there, meets no block.
fn free_position(
    w: &MobWorld,
    allowed: Aabb,
    preferred: Vec3,
    size_x: f64,
    size_y: f64,
) -> Option<Vec3> {
    use sunrise_core::geometry::{ShapeOp, VoxelShape};
    let allowed_shape = VoxelShape::from_box(allowed).ok()?;
    if allowed_shape.is_empty() {
        return None;
    }
    let half = Vec3::new(size_x / 2.0, size_y / 2.0, size_x / 2.0);
    let area = allowed.inflate(half);
    let (lo, hi) = (area.min_position(), area.max_position());
    let mut collisions = VoxelShape::empty();
    // getBlockCollisions: the block shapes that reach into the area.
    for x in (lo.x - 1.0e-7).floor() as i32 - 1..=(hi.x + 1.0e-7).floor() as i32 + 1 {
        for y in (lo.y - 1.0e-7).floor() as i32 - 1..=(hi.y + 1.0e-7).floor() as i32 + 1 {
            for z in (lo.z - 1.0e-7).floor() as i32 - 1..=(hi.z + 1.0e-7).floor() as i32 + 1 {
                let boxes = w.block_collision_boxes(x, y, z);
                let touches = boxes.iter().any(|b| {
                    let (a, c) = (b.min_position(), b.max_position());
                    a.x < hi.x && c.x > lo.x && a.y < hi.y && c.y > lo.y && a.z < hi.z && c.z > lo.z
                });
                if !touches {
                    continue;
                }
                for b in boxes {
                    let grown = VoxelShape::from_box(b.inflate(half)).ok()?;
                    collisions = collisions.join(&grown, ShapeOp::OR).ok()?;
                }
            }
        }
    }
    let free = allowed_shape.join(&collisions, ShapeOp::ONLY_FIRST).ok()?;
    free.closest_point(preferred)
}

/// `Entity.doWaterSplashEffect` for a mob: its own random, its splash sound
/// (`getSwimSplashSound`), particles for the client and the SPLASH game event.
struct Splash<'a> {
    mob: &'a mut Mob,
    events: &'a mut Vec<MobEvent>,
}
impl sunrise_world::water_splash::WaterSplashWorld for Splash<'_> {
    type Error = std::convert::Infallible;
    type Entity = ();
    type Sound = &'static str;
    fn this_entity(&self) {}
    fn controlling_passenger(&mut self) -> Result<Option<()>, Self::Error> {
        Ok(None)
    }
    fn velocity(&mut self, _: ()) -> Result<Vec3, Self::Error> {
        Ok(self.mob.velocity)
    }
    fn splash_sound(&mut self, _high_speed: bool) -> Result<&'static str, Self::Error> {
        use super::Species;
        // Entity.getSwimSplashSound and getSwimHighSpeedSplashSound: the
        // overrides are Monster's, the hoglin's, the dolphin's and the axolotl's.
        let species = self.mob.species();
        Ok(match species {
            Species::Dolphin => "entity.dolphin.splash",
            Species::Axolotl => "entity.axolotl.splash",
            _ if species == Species::Hoglin || super::ai::is_monster(species) => {
                "entity.hostile.splash"
            }
            _ => "entity.generic.splash",
        })
    }
    fn random_float(&mut self) -> Result<f32, Self::Error> {
        Ok(self.mob.random.next_float())
    }
    fn random_double(&mut self) -> Result<f64, Self::Error> {
        Ok(self.mob.random.next_double())
    }
    fn play_sound(
        &mut self,
        sound: &'static str,
        volume: f32,
        pitch: f32,
    ) -> Result<(), Self::Error> {
        if !self.mob.silent {
            let at = [
                self.mob.position.x,
                self.mob.position.y,
                self.mob.position.z,
            ];
            self.events.push(MobEvent::Sound(sound, at, volume, pitch));
        }
        Ok(())
    }
    fn position(&self) -> Vec3 {
        self.mob.position
    }
    fn width(&self) -> f32 {
        self.mob.width()
    }
    fn particle(
        &mut self,
        kind: sunrise_world::water_splash::SplashParticle,
        position: Vec3,
        velocity: Vec3,
    ) -> Result<(), Self::Error> {
        self.events.push(MobEvent::ParticleAt {
            kind: match kind {
                sunrise_world::water_splash::SplashParticle::Bubble => "bubble",
                sunrise_world::water_splash::SplashParticle::Splash => "splash",
            },
            at: [position.x, position.y, position.z],
            velocity: [velocity.x, velocity.y, velocity.z],
        });
        Ok(())
    }
    fn splash_event(&mut self) -> Result<(), Self::Error> {
        self.events.push(MobEvent::GameEvent {
            event: "minecraft:splash",
            at: [
                self.mob.position.x,
                self.mob.position.y,
                self.mob.position.z,
            ],
            source: self.mob.id,
            state: sunrise_world::registry::BlockStateId(0),
        });
        Ok(())
    }
}

/// `Entity.getEyePosition(partialTick)` as the cell the renderer lights an
/// entity from (`Entity.getLightProbePosition` is that position and
/// `EntityRenderer.getPackedLightCoords` takes `BlockPos.containing` of it):
/// `Mth.lerp(partialTick, xo, x)` on each axis, plus `getEyeHeight()` on y.
pub(crate) fn eye_probe_cell(
    previous: Vec3,
    position: Vec3,
    eye_height: f64,
    partial: f64,
) -> [i32; 3] {
    let at = previous.lerp(position, partial);
    [
        at.x.floor() as i32,
        (at.y + eye_height).floor() as i32,
        at.z.floor() as i32,
    ]
}

/// `Entity.doWaterSplashEffect` for a mob this window is drawing.
/// `Level.addParticle` runs on the client. `ClientLevel.playSeededSound`
/// ignores `Entity.playSound` (`except` is null), and the SPLASH game event
/// stays on the server, so only the bubble and splash particles are kept.
pub fn client_water_splash(mob: &mut Mob, out: &mut Vec<MobEvent>) {
    let mut events = Vec::new();
    let _ = sunrise_world::water_splash::water_splash(
        &mut Splash {
            mob,
            events: &mut events,
        },
        1_000_000,
    );
    out.extend(
        events
            .into_iter()
            .filter(|event| matches!(event, MobEvent::ParticleAt { .. })),
    );
}

/// `Entity.restituteMovementAfterCollisions`' native inputs, bound for a mob:
/// its own bounciness attribute, the block's coefficients and the pose of a
/// mob that never sneaks. The phases themselves are
/// `movement_restitution::restitute_movement`, the same port the player and
/// dropped items use.
struct MobRestitution<'a, 'w, 'l> {
    mob: &'a mut Mob,
    world: &'a MobWorld<'w>,
    level: &'a mut DamageLevel<'l>,
}
impl<'a, 'w, 'l> MobRestitution<'a, 'w, 'l> {
    /// The block's movement data: its coefficients and its bounce tag.
    fn movement(
        &self,
        state: BlockStateId,
    ) -> Result<sunrise_world::movement_properties::MovementBlock, ()> {
        self.world.movement.get(state).map_err(|_| ())
    }
}
impl RestitutionWorld for MobRestitution<'_, '_, '_> {
    // The block data cannot fail here: a state the registry does not know
    // reads as a block with no bounce, never as an error.
    type Error = ();
    /// The entity's own suppression is its sneak pose, and a mob never
    /// sneaks; the block's tag is `block_suppresses_bounce` below.
    fn suppressing_bounce(&mut self) -> Result<bool, ()> {
        Ok(false)
    }
    /// `getAttributeValue(BOUNCINESS)`: a sulfur cube's.
    fn entity_bounciness(&mut self) -> Result<f64, ()> {
        Ok(self.mob.entity_bounciness())
    }
    fn velocity(&mut self) -> Result<Vec3, ()> {
        Ok(self.mob.velocity)
    }
    /// `living_motion::effective_gravity`: the gravity attribute, floored to
    /// 0.01 while a Slow Falling mob is on its way down.
    fn effective_gravity(&mut self) -> Result<f64, ()> {
        Ok(self.mob.effective_gravity())
    }
    fn block_suppresses_bounce(&mut self, state: BlockStateId) -> Result<bool, ()> {
        Ok(self.movement(state)?.suppresses_bounce)
    }
    /// `Entity.getBlockBounciness`: the restitution of the block under the
    /// entity, so a mob landing on a slime block or a bed bounces too, after
    /// the block's parts, on the side that ticks the mob.
    fn block_bounce_restitution(
        &mut self,
        state: BlockStateId,
        position: BlockPos,
    ) -> Result<f32, ()> {
        let own = self.movement(state)?.coefficients.bounce_restitution;
        let contact = crate::block_contact::Contact {
            entity: self.mob.id as i32,
            kind: &self.mob.kind.id,
            state,
            position,
        };
        Ok(crate::block_contact::bounce_restitution(
            crate::motion::FrictionLevel {
                world: self.world.world,
                registry: self.world.collision.registry(),
                context: self.world.loop_context,
                side: self.world.side,
                coefficient: &|id| {
                    self.world
                        .movement
                        .get(id)
                        .ok()
                        .map(|p| p.coefficients.friction)
                },
            },
            contact,
            own,
        ))
    }
    /// `LivingEntity.getAirDrag`.
    fn air_drag(&mut self) -> Result<f32, ()> {
        Ok(self.mob.air_drag())
    }
    fn movement_support_position(&mut self) -> Result<BlockPos, ()> {
        Ok(self.mob.movement_support_position(self.world))
    }
    /// `BlockLandingRegistry.bounce_on`'s only case is the shelf mushroom's
    /// bounce sound, which no path plays yet (`landing.rs` binds it for the
    /// player too and the game's sound code resolves neither).
    fn bounce_on(
        &mut self,
        state: BlockStateId,
        position: BlockPos,
        fall_distance: f64,
        movement: Vec3,
    ) -> Result<Vec3, ()> {
        Ok(crate::block_contact::bounce(
            self.level,
            crate::block_contact::Contact {
                entity: self.mob.id as i32,
                kind: &self.mob.kind.id,
                state,
                position,
            },
            fall_distance,
            movement,
        ))
    }
    fn bounce_event(&mut self) -> Result<(), ()> {
        Ok(())
    }
    /// `Entity.syncPosition`: the tracker keeps where the mob bounced as a
    /// step of its next move (`SteppedInterpolationTracker.updateTracking`).
    fn mark_position_for_sync(&mut self) {
        self.mob.sync_position = true;
    }
    fn set_velocity(&mut self, velocity: Vec3) -> Result<(), ()> {
        self.mob.set_velocity(velocity);
        Ok(())
    }
}

/// `LivingEntity.randomTeleport(x, y, z, true, avoid)` for a mob (#944), the
/// path an enderman's own teleport takes: the target is clamped into the
/// world border, then drops to the first block in
/// `minecraft:entities_can_teleport_to` below it, losing a block of height per
/// step down. The mob lands there (`checkPositionAndTeleport`) unless that
/// ground or a block in its new box is in the `avoid` tag, the box collides or
/// holds a liquid, or the species refuses the spot (`canRandomlyTeleportTo`:
/// an enderman never lands on ground holding water). On landing the old
/// position moves along (`snapTo`), the motion stays, the navigation stops
/// (`PathfinderMob`), and entity event 46 draws the portal particles.
pub fn random_teleport(
    m: &mut Mob,
    w: &MobWorld,
    target: Vec3,
    avoid: Option<&str>,
    events: &mut Vec<MobEvent>,
) -> bool {
    // WorldBorder.clampVec3ToBound keeps the target inside the border
    // (Mth.clamp, 1.0E-5F inside the far edges).
    let (x, z) = match w.border {
        Some(border) => {
            let [min_x, min_z, max_x, max_z] = border.bounds(0.0);
            let clamp = |v: f64, low: f64, high: f64| if v < low { low } else { v.min(high) };
            (
                clamp(target.x, min_x, max_x - f64::from(1.0e-5_f32)),
                clamp(target.z, min_z, max_z - f64::from(1.0e-5_f32)),
            )
        }
        None => (target.x, target.z),
    };
    let (bx, bz) = (x.floor() as i32, z.floor() as i32);
    if !w.world.loaded(bx, bz) {
        return false;
    }
    let refused = |name: &str| avoid.is_some_and(|tag| w.has_tag(tag, name));
    let mut by = target.y.floor() as i32;
    let mut ny = target.y;
    let ground = loop {
        if by <= w.world.bottom_y() {
            return false;
        }
        by -= 1;
        let name = w.block_name(bx, by, bz);
        if w.has_tag("minecraft:entities_can_teleport_to", name) {
            break name;
        }
        ny -= 1.0;
    };
    if refused(ground) {
        return false;
    }
    let half = f64::from(m.width()) / 2.0;
    let b = Aabb::new(
        Vec3::new(x - half, ny, z - half),
        Vec3::new(x + half, ny + f64::from(m.height()), z + half),
    );
    if !w.no_collision(b) {
        return false;
    }
    // LevelReader.containsAnyLiquid and findBlocksIn: the same closed range of
    // cells, from BlockPos.containing(min) to BlockPos.containing(max).
    let (lo, hi) = (b.min_position(), b.max_position());
    for cx in lo.x.floor() as i32..=hi.x.floor() as i32 {
        for cy in lo.y.floor() as i32..=hi.y.floor() as i32 {
            for cz in lo.z.floor() as i32..=hi.z.floor() as i32 {
                if w.fluid(cx, cy, cz) != Fluid::None || refused(w.block_name(cx, cy, cz)) {
                    return false;
                }
            }
        }
    }
    // Enderman.canRandomlyTeleportTo: not onto ground that holds water
    // (a waterlogged slab, leaves or roots).
    if m.species() == super::Species::Enderman && w.fluid(bx, by, bz) == Fluid::Water {
        return false;
    }
    // Entity.teleportTo -> snapTo: the old position moves too, the motion stays.
    teleports::snap_to_with(m, Vec3::new(x, ny, z), w.section_writer.clone());
    m.ai.nav.stop();
    // `randomTeleport(..., true)` broadcasts entity event 46.
    events.push(MobEvent::EntityEvent {
        mob: m.id,
        event: 46,
        at: [m.position.x, m.position.y, m.position.z],
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    /// `AgeableMob.isBaby` is `canBeABaby() && getAge() < 0`, and `Age` alone
    /// says nothing: the six ageable classes that override `canBeABaby` to
    /// false — `Frog`, `Parrot`, `ZombieNautilus`, `CamelHusk`, `Slime` and
    /// `MagmaCube` — stay adults, and so do the `PiglinBrute` (no `isBaby` of
    /// its own), the `Creeper` (outside the ageable tree) and the `Tadpole`,
    /// whose own age runs up from -24000 without it ever being a baby.
    ///
    /// Every one of them keeps its type's box, the plain 1.0 ± 0.2 voice and
    /// `getAgeScale`'s 1.0, where a cow with the same age takes
    /// `Cow.BABY_DIMENSIONS` (0.45 by 0.7, eyes at 0.69) and the 1.5 ± 0.2
    /// that `LivingEntity.getVoicePitch` gives a baby.
    #[test]
    fn a_negative_age_is_a_baby_only_where_can_be_a_baby_allows_it() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists() {
            return;
        }
        let types = super::super::MobTypes::load(&root).unwrap();
        // dims() narrows the type's box to f32 and widens it back.
        let adult = |id: &str| {
            let t = types.types.get(id).unwrap();
            (
                f64::from(t.width as f32),
                f64::from(t.height as f32),
                f64::from(t.eye_height as f32),
            )
        };
        for id in [
            "minecraft:parrot",
            "minecraft:tadpole",
            "minecraft:frog",
            "minecraft:zombie_nautilus",
            "minecraft:camel_husk",
            "minecraft:slime",
            "minecraft:magma_cube",
            "minecraft:piglin_brute",
            "minecraft:creeper",
        ] {
            let mut m = Mob::new(
                1,
                std::sync::Arc::new(types.types.get(id).unwrap().clone()),
                Vec3::ZERO,
                0.0,
            );
            m.extra.age = -24_000;
            assert!(!m.is_baby(), "{id}");
            assert_eq!(m.dims(), adult(id), "{id} keeps its type's box");
            let pitches: Vec<f32> = (0..64).map(|_| m.voice_pitch()).collect();
            assert!(
                pitches.iter().all(|p| (0.8..=1.2).contains(p)),
                "{id}: {pitches:?}"
            );
        }
        // The control: a cow at the same age is a baby in vanilla's box and
        // voice, and an adult again at age 0.
        let mut cow = Mob::new(
            1,
            std::sync::Arc::new(types.types.get("minecraft:cow").unwrap().clone()),
            Vec3::ZERO,
            0.0,
        );
        cow.extra.age = -24_000;
        assert!(cow.is_baby());
        assert_eq!(
            cow.dims(),
            (f64::from(0.45_f32), f64::from(0.7_f32), f64::from(0.69_f32))
        );
        let pitches: Vec<f32> = (0..64).map(|_| cow.voice_pitch()).collect();
        assert!(
            pitches.iter().all(|p| (1.3..=1.7).contains(p)),
            "{pitches:?}"
        );
        cow.extra.age = 0;
        assert!(!cow.is_baby());
        assert_eq!(cow.dims(), adult("minecraft:cow"));
        let pitches: Vec<f32> = (0..64).map(|_| cow.voice_pitch()).collect();
        assert!(
            pitches.iter().all(|p| (0.8..=1.2).contains(p)),
            "{pitches:?}"
        );
    }

    /// `LivingEntity.getJumpPower` multiplies the jump strength by the block jump factor of the
    /// block the mob stands on: honey halves a pig's jump, stone leaves it exactly its strength.
    #[test]
    #[ignore = "daily"]
    fn a_mob_jumps_by_the_jump_factor_of_the_block_under_it() {
        let build = |world: &mut crate::world::World,
                     registry: &sunrise_world::registry::BlockRegistry| {
            world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
            );
            for (x, ground) in [(0, "minecraft:stone"), (2, "minecraft:honey_block")] {
                world
                    .set(x, 0, 0, registry.default_state(ground).unwrap())
                    .unwrap();
            }
        };
        crate::mobs::world::with_built_test_world(&[], build, |w, types| {
            let kind = std::sync::Arc::new(types.types["minecraft:pig"].clone());
            let strength = kind.jump_strength as f32;
            let mut m = Mob::new(1, kind, Vec3::new(0.5, 1.0, 0.5), 0.0);
            assert_eq!(m.jump_power(w).to_bits(), strength.to_bits());
            m.position.x = 2.5;
            assert_eq!(m.jump_power(w), strength * 0.5);
        });
    }

    /// `Entity.getLightProbePosition(partialTick)` for a living entity is
    /// `getEyePosition`: `getEyeHeight()` over `Mth.lerp(partialTick, xo, x)`.
    /// Sunrise probed this tick's own position, so a mob walking into the next
    /// cell was drawn lit by that cell for the whole frame before it got there,
    /// and a falling one for a frame after it left.
    #[test]
    fn a_mob_probe_follows_the_frame() {
        let eye = 1.62;
        // Walking from x = 0.9 to 1.1: the eye is in the cell at 0 until the
        // frame reaches the boundary, and in the cell at 1 from there on.
        let (previous, position) = (Vec3::new(0.9, 64.0, 0.5), Vec3::new(1.1, 64.0, 0.5));
        assert_eq!(eye_probe_cell(previous, position, eye, 0.0), [0, 65, 0]);
        assert_eq!(eye_probe_cell(previous, position, eye, 0.5), [1, 65, 0]);
        assert_eq!(eye_probe_cell(previous, position, eye, 1.0), [1, 65, 0]);
        // The old probe was the tick's own position on every frame.
        assert_eq!(position.x.floor() as i32, 1);
        // Falling a block: the eye is at 71.62 at the start of the frame and
        // 70.62 at its end.
        let (previous, position) = (Vec3::new(0.5, 70.0, 0.5), Vec3::new(0.5, 69.0, 0.5));
        assert_eq!(eye_probe_cell(previous, position, eye, 0.0), [0, 71, 0]);
        assert_eq!(eye_probe_cell(previous, position, eye, 0.5), [0, 71, 0]);
        assert_eq!(eye_probe_cell(previous, position, eye, 1.0), [0, 70, 0]);
    }
    /// A pig dropped hard onto a block bounces by that block's restitution
    /// (`Entity.getBlockBounciness`): landing on a slime block sends it back
    /// up, landing on stone stops it dead. The same 8-block fall then costs the
    /// pig what the block's own `fallOn` allows (`Block.fallOn`): all 5 on
    /// stone, 1 on hay or honey, 0 on slime, 1 on a bed, and a stalagmite kills.
    #[test]
    fn a_mob_bounces_off_a_bouncy_block_and_not_off_stone() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists()
            || !root.join("movement-properties.nbt").exists()
        {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let registry = assets.registry;
        let facts = crate::blocks::BlockFacts::load(&root, registry).unwrap();
        let motion = crate::motion::MotionResources::load(&root, &assets).unwrap();
        let data = super::super::MobData::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            facts.states.len(),
        )
        .unwrap();
        let types = super::super::MobTypes::load(&root).unwrap();
        let biomes = crate::biome::Biomes::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
        )
        .unwrap();
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let items = crate::items::Items::load(
            &root,
            registry,
            &facts,
            &sunrise_world::loot::PackOverlay::NONE,
            &defaults,
        )
        .unwrap();
        let collision = std::sync::Arc::new(crate::collisions::load(&root, registry).unwrap());
        let suffocation =
            crate::mobs::suffocation::bind(collision.clone(), &motion.causes_suffocation).unwrap();
        let lid_openers = std::collections::HashMap::new();
        for (ground, bounces, health) in [
            ("minecraft:slime_block", true, 10.0),
            ("minecraft:stone", false, 5.0),
            ("minecraft:hay_block", false, 9.0),
            ("minecraft:honey_block", false, 9.0),
            ("minecraft:red_bed", true, 9.0),
            ("minecraft:pointed_dripstone", false, 0.0),
        ] {
            let air = registry.default_state("minecraft:air").unwrap();
            let mut world = crate::world::World::new(air, 0);
            world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
            );
            let floor = registry.default_state(ground).unwrap();
            for x in 0..=1 {
                for z in 0..=1 {
                    world.set(x, 0, z, floor).unwrap();
                }
            }
            let w = MobWorld {
                side: crate::motion::dry_side(),
                mob_maps: crate::neoforge_mob_maps::NeoForgeMobMaps::vanilla(),
                goal_host: None,
                live_mobs: None,
                teams: Default::default(),
                professions: crate::mobs::villager::profession_table(),
                loop_context: crate::test_resources::server_loop_context(),
                world: &world,
                registry,
                facts: &facts,
                fluid_tables: &motion.fluids,
                attributes: &motion.attributes,
                movement: &motion.blocks,
                collision: &collision,
                suffocation: &suffocation,
                data: &data,
                player: None,
                player_identity: None,
                player_selection: None,
                controlling_player_movement: None,
                nearest_player: None,
                others: &[],
                dragon: None,
                armor_stands: &[],
                leash_boats: &[],
                who_lookup: None,
                others_grid: None,
                others_index: None,
                passenger_objects: &[],
                others_villagers: None,
                others_solid: None,
                daylight: true,
                monsters_burn: false,
                game_time: 1000,
                day_time: 18000,
                raining: false,
                thundering: false,
                level: sunrise_core::LevelKey::OVERWORLD,
                block_tags: &items.block_tags,
                block_tag_flags: items.block_tag_flags.bits(),
                landing: &motion.landing,
                biomes: &biomes,
                universal_anger: false,
                pvp: false,
                mob_griefing: true,
                mob_drops: true,
                max_entity_cramming: 24,
                difficulty: super::super::spawning::LocalDifficulty::new(2, 18000, 0, 0.0),
                dimension_type: sunrise_world::dimension_type::DimensionFacts::overworld(),
                world_seed: 0,
                sea_level: 63,
                items: Some(&items),
                lid_openers: &lid_openers,
                mod_effects: None,
                mob_hooks: None,
                asks_animal_tame: false,
                asks_living_destroy_block: false,
                asks_baby_spawn: false,
                border: None,
                section_writer: None,
                trading_players: None,
            };
            let kind = std::sync::Arc::new(types.types["minecraft:pig"].clone());
            let mut m = Mob::new(1, kind, Vec3::new(0.5, 1.0, 0.5), 0.0);
            // Falling faster than gravity, so the landing is hard enough.
            m.velocity = Vec3::new(0.0, -1.0, 0.0);
            m.fall_distance = 8.0;
            m.do_move(&w, level, &mut Vec::new(), &mut MoveGate::new());
            assert_eq!(m.health, health, "{ground}: health after an 8 block fall");
            assert!(
                m.vertical_collision_below,
                "{ground}: the pig lands on the block"
            );
            if bounces {
                assert!(
                    m.velocity.y > 0.0,
                    "{ground}: restitution sends the pig back up, got {}",
                    m.velocity.y
                );
            } else {
                assert!(
                    m.velocity.y <= 0.0,
                    "{ground}: no bounce, got {}",
                    m.velocity.y
                );
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn mob_edge_bounce_uses_supporting_column() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let root = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES").expect("prepared resources"),
        );
        let assets = crate::assets::Assets::load(&root).unwrap();
        let registry = assets.registry;
        let facts = crate::blocks::BlockFacts::load(&root, registry).unwrap();
        let motion = crate::motion::MotionResources::load(&root, &assets).unwrap();
        let data = super::super::MobData::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            facts.states.len(),
        )
        .unwrap();
        let types = super::super::MobTypes::load(&root).unwrap();
        let biomes = crate::biome::Biomes::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
        )
        .unwrap();
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let items = crate::items::Items::load(
            &root,
            registry,
            &facts,
            &sunrise_world::loot::PackOverlay::NONE,
            &defaults,
        )
        .unwrap();
        let collision = std::sync::Arc::new(crate::collisions::load(&root, registry).unwrap());
        let suffocation =
            crate::mobs::suffocation::bind(collision.clone(), &motion.causes_suffocation).unwrap();
        let lid_openers = std::collections::HashMap::new();
        for (ground, expected) in [("minecraft:slime_block", 1.0), ("minecraft:stone", 0.0)] {
            let air = registry.default_state("minecraft:air").unwrap();
            let mut world = crate::world::World::new(air, 0);
            world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
            );
            let floor = registry.default_state(ground).unwrap();
            world.set(0, 0, 0, floor).unwrap();
            world
                .set(1, 0, 0, registry.default_state("minecraft:stone").unwrap())
                .unwrap();
            let w = MobWorld {
                side: crate::motion::dry_side(),
                mob_maps: crate::neoforge_mob_maps::NeoForgeMobMaps::vanilla(),
                goal_host: None,
                live_mobs: None,
                teams: Default::default(),
                professions: crate::mobs::villager::profession_table(),
                loop_context: crate::test_resources::server_loop_context(),
                world: &world,
                registry,
                facts: &facts,
                fluid_tables: &motion.fluids,
                attributes: &motion.attributes,
                movement: &motion.blocks,
                collision: &collision,
                suffocation: &suffocation,
                data: &data,
                player: None,
                player_identity: None,
                player_selection: None,
                controlling_player_movement: None,
                nearest_player: None,
                others: &[],
                dragon: None,
                armor_stands: &[],
                leash_boats: &[],
                who_lookup: None,
                others_grid: None,
                others_index: None,
                passenger_objects: &[],
                others_villagers: None,
                others_solid: None,
                daylight: true,
                monsters_burn: false,
                game_time: 1000,
                day_time: 18000,
                raining: false,
                thundering: false,
                level: sunrise_core::LevelKey::OVERWORLD,
                block_tags: &items.block_tags,
                block_tag_flags: items.block_tag_flags.bits(),
                landing: &motion.landing,
                biomes: &biomes,
                universal_anger: false,
                pvp: false,
                mob_griefing: true,
                mob_drops: true,
                max_entity_cramming: 24,
                difficulty: super::super::spawning::LocalDifficulty::new(2, 18000, 0, 0.0),
                dimension_type: sunrise_world::dimension_type::DimensionFacts::overworld(),
                world_seed: 0,
                sea_level: 63,
                items: Some(&items),
                lid_openers: &lid_openers,
                mod_effects: None,
                mob_hooks: None,
                asks_animal_tame: false,
                asks_living_destroy_block: false,
                asks_baby_spawn: false,
                border: None,
                section_writer: None,
                trading_players: None,
            };
            let kind = std::sync::Arc::new(types.types["minecraft:pig"].clone());
            let mut m = Mob::new(1, kind, Vec3::new(0.5, 1.0, 0.5), 0.0);
            m.position = Vec3::new(1.1, 1.0, 0.5);
            m.ground.supporting_block = Some(sunrise_core::position::BlockPos::new(0, 0, 0));
            m.velocity = Vec3::new(0.0, -1.0, 0.0);
            assert_eq!(m.on_pos(0.2), [1, 0, 0]);
            assert_eq!(m.block_on_legacy(&w), [0, 0, 0]);
            m.restitute_movement(&w, level, Vec3::new(0.0, -1.0, 0.0), Vec3::ZERO);
            assert!(
                (m.velocity.y - expected).abs() < 1.0e-6,
                "{ground}: {}",
                m.velocity.y
            );
        }
    }

    /// `LivingEntity.travelInFluid` ends with
    /// `floatInLiquidWhileRidden(FluidTags.ENTITY_FLOATABLE)`: a mount that
    /// carries a passenger, is in `minecraft:can_float_while_ridden` and sits
    /// deeper in the water than its jump threshold gains 0.04 a tick upwards.
    /// That is what keeps a ridden horse and its rider afloat; an empty horse
    /// gets nothing and goes on sinking.
    #[test]
    fn a_ridden_horse_floats_in_deep_water() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists()
            || !root.join("movement-properties.nbt").exists()
        {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let registry = assets.registry;
        let facts = crate::blocks::BlockFacts::load(&root, registry).unwrap();
        let motion = crate::motion::MotionResources::load(&root, &assets).unwrap();
        let data = super::super::MobData::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            facts.states.len(),
        )
        .unwrap();
        let types = super::super::MobTypes::load(&root).unwrap();
        let biomes = crate::biome::Biomes::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
        )
        .unwrap();
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let items = crate::items::Items::load(
            &root,
            registry,
            &facts,
            &sunrise_world::loot::PackOverlay::NONE,
            &defaults,
        )
        .unwrap();
        let collision = std::sync::Arc::new(crate::collisions::load(&root, registry).unwrap());
        let suffocation =
            crate::mobs::suffocation::bind(collision.clone(), &motion.causes_suffocation).unwrap();
        let lid_openers = std::collections::HashMap::new();
        let air = registry.default_state("minecraft:air").unwrap();
        let water = registry.default_state("minecraft:water").unwrap();
        let mut world = crate::world::World::new(air, 0);
        world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
        );
        for x in 4..=12 {
            for z in 4..=12 {
                for y in 63..=66 {
                    world.set(x, y, z, water).unwrap();
                }
            }
        }
        let w = MobWorld {
            side: crate::motion::dry_side(),
            mob_maps: crate::neoforge_mob_maps::NeoForgeMobMaps::vanilla(),
            goal_host: None,
            live_mobs: None,
            teams: Default::default(),
            professions: crate::mobs::villager::profession_table(),
            loop_context: crate::test_resources::server_loop_context(),
            world: &world,
            registry,
            facts: &facts,
            fluid_tables: &motion.fluids,
            attributes: &motion.attributes,
            movement: &motion.blocks,
            collision: &collision,
            suffocation: &suffocation,
            data: &data,
            player: None,
            player_identity: None,
            player_selection: None,
            controlling_player_movement: None,
            nearest_player: None,
            others: &[],
            dragon: None,
            armor_stands: &[],
            leash_boats: &[],
            who_lookup: None,
            others_grid: None,
            others_index: None,
            passenger_objects: &[],
            others_villagers: None,
            others_solid: None,
            daylight: true,
            monsters_burn: false,
            game_time: 1000,
            day_time: 18000,
            raining: false,
            thundering: false,
            level: sunrise_core::LevelKey::OVERWORLD,
            block_tags: &items.block_tags,
            block_tag_flags: items.block_tag_flags.bits(),
            landing: &motion.landing,
            biomes: &biomes,
            universal_anger: false,
            pvp: false,
            mob_griefing: true,
            mob_drops: true,
            max_entity_cramming: 24,
            difficulty: super::super::spawning::LocalDifficulty::new(2, 18000, 0, 0.0),
            dimension_type: sunrise_world::dimension_type::DimensionFacts::overworld(),
            world_seed: 0,
            sea_level: 63,
            items: Some(&items),
            lid_openers: &lid_openers,
            mod_effects: None,
            mob_hooks: None,
            asks_animal_tame: false,
            asks_living_destroy_block: false,
            asks_baby_spawn: false,
            border: None,
            section_writer: None,
            trading_players: None,
        };
        let kind = std::sync::Arc::new(types.types["minecraft:horse"].clone());
        let mut sank = 0.0;
        let mut floated = 0.0;
        for ridden in [false, true] {
            let mut m = Mob::new(1, kind.clone(), Vec3::new(8.5, 64.0, 8.5), 0.0);
            if ridden {
                m.rider = Some(super::super::equine::Rider::default());
            }
            m.fluids.in_water = true;
            m.fluids.water = 1.0;
            m.travel(&w, level, Vec3::ZERO, &mut Vec::new());
            if ridden {
                floated = m.velocity.y;
            } else {
                sank = m.velocity.y;
            }
        }
        assert!(sank < 0.0, "an empty horse sinks, got {sank}");
        let lift = floated - sank;
        assert!(
            (lift - f64::from(0.04_f32)).abs() < 1.0e-9,
            "a ridden horse gains 0.04 a tick, got {lift}"
        );
    }

    /// `Entity.collide` takes `getEntityCollisions` too: a zombie dropping
    /// onto a living shulker or a happy ghast held still (a player on its
    /// back) lands on it; a dead shulker, a happy ghast that is not held, or
    /// a ghastling lets it through (`canBeCollidedWith`).
    #[test]
    fn a_falling_mob_lands_on_a_shulker_or_a_held_happy_ghast() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists()
            || !root.join("movement-properties.nbt").exists()
        {
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let registry = assets.registry;
        let facts = crate::blocks::BlockFacts::load(&root, registry).unwrap();
        let motion = crate::motion::MotionResources::load(&root, &assets).unwrap();
        let data = super::super::MobData::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            facts.states.len(),
        )
        .unwrap();
        let types = super::super::MobTypes::load(&root).unwrap();
        let biomes = crate::biome::Biomes::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        )
        .unwrap();
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let items = crate::items::Items::load(
            &root,
            registry,
            &facts,
            &sunrise_world::loot::PackOverlay::NONE,
            &defaults,
        )
        .unwrap();
        let collision = std::sync::Arc::new(crate::collisions::load(&root, registry).unwrap());
        let suffocation =
            crate::mobs::suffocation::bind(collision.clone(), &motion.causes_suffocation).unwrap();
        let lid_openers = std::collections::HashMap::new();
        let air = registry.default_state("minecraft:air").unwrap();
        let mut world = crate::world::World::new(air, 0);
        world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
        );
        let mut lent = crate::health::LentLevel::take(&mut world);
        let mob = |id: u32, kind: &str, at: Vec3| {
            Mob::new(id, std::sync::Arc::new(types.types[kind].clone()), at, 0.0)
        };
        // Where a zombie starting 3 blocks above `below`'s top ends after 20
        // falling moves, with `below` in the mobs it sees.
        let mut landing = |below: &Mob| -> f64 {
            let others = [below.seen(None, &crate::items::Items::empty())];
            let w = MobWorld {
                side: crate::motion::dry_side(),
                mob_maps: crate::neoforge_mob_maps::NeoForgeMobMaps::vanilla(),
                professions: crate::mobs::villager::profession_table(),
                loop_context: crate::test_resources::server_loop_context(),
                attributes: &motion.attributes,
                player_identity: None,
                player_selection: None,
                mob_hooks: None,
                goal_host: None,
                live_mobs: None,
                asks_animal_tame: false,
                asks_living_destroy_block: false,
                asks_baby_spawn: false,
                teams: Default::default(),
                world: &world,
                registry,
                facts: &facts,
                fluid_tables: &motion.fluids,
                movement: &motion.blocks,
                collision: &collision,
                suffocation: &suffocation,
                data: &data,
                player: None,
                controlling_player_movement: None,
                nearest_player: None,
                others: &others,
                who_lookup: None,
                dragon: None,
                armor_stands: &[],
                leash_boats: &[],
                others_grid: None,
                others_index: None,
                passenger_objects: &[],
                others_villagers: None,
                others_solid: None,
                daylight: true,
                monsters_burn: false,
                game_time: 1000,
                day_time: 6000,
                raining: false,
                thundering: false,
                level: sunrise_core::LevelKey::OVERWORLD,
                block_tags: &items.block_tags,
                block_tag_flags: items.block_tag_flags.bits(),
                landing: &motion.landing,
                biomes: &biomes,
                universal_anger: false,
                pvp: false,
                mob_griefing: true,
                mob_drops: true,
                max_entity_cramming: 24,
                difficulty: super::super::spawning::LocalDifficulty::new(2, 6000, 0, 0.0),
                dimension_type: sunrise_world::dimension_type::DimensionFacts::overworld(),
                world_seed: 0,
                sea_level: 63,
                items: Some(&items),
                lid_openers: &lid_openers,
                mod_effects: None,
                border: None,
                section_writer: None,
                trading_players: None,
            };
            let top = below.bounds().max_position().y;
            let mut zombie = mob(1, "minecraft:zombie", Vec3::new(8.5, top + 3.0, 8.5));
            for _ in 0..20 {
                zombie.velocity = Vec3::new(0.0, -0.5, 0.0);
                zombie.do_move(
                    &w,
                    &mut DamageLevel::lent(
                        &world,
                        registry,
                        &mut lent,
                        Some(&items),
                        None,
                        &crate::health::DamageHooks::NONE,
                    ),
                    &mut Vec::new(),
                    &mut MoveGate::new(),
                );
            }
            zombie.position.y - top
        };
        let mut shulker = mob(2, "minecraft:shulker", Vec3::new(8.5, 64.0, 8.5));
        assert_eq!(landing(&shulker), 0.0, "a living shulker holds it up");
        shulker.health = 0.0;
        assert!(landing(&shulker) < -5.0, "a dead shulker does not");
        let mut ghast = mob(3, "minecraft:happy_ghast", Vec3::new(8.5, 64.0, 8.5));
        assert!(
            landing(&ghast) < -5.0,
            "a happy ghast left alone is not solid"
        );
        ghast.extra.counter = 10;
        assert_eq!(
            landing(&ghast),
            0.0,
            "held still, its back holds the zombie"
        );
        let mut ghastling = mob(4, "minecraft:happy_ghast", Vec3::new(8.5, 64.0, 8.5));
        super::super::finalize::set_baby(&mut ghastling, true);
        ghastling.extra.counter = 10;
        assert!(landing(&ghastling) < -5.0, "a ghastling never is");
    }

    /// `LivingEntity.aiStep` calls `travel` and only then
    /// `applyEffectsFromBlocks`, so the lift of a bubble column is added after
    /// the water's drag and is not scaled by it. A pig 1.0 high in a column of
    /// two blocks of water takes the plain water velocity from `travel`, and
    /// the lift (0.06 for each column block its box touches) on top of it.
    #[test]
    #[ignore = "daily"]
    fn a_bubble_column_lifts_a_mob_after_travel_not_inside_it() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut run = |column: bool| {
            let mut out = None;
            let build =
                |world: &mut crate::world::World,
                 registry: &sunrise_world::registry::BlockRegistry| {
                    world.chunks.insert(
                        (0, 0),
                        std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
                    );
                    let stone = registry.default_state("minecraft:stone").unwrap();
                    let water = registry.default_state("minecraft:water").unwrap();
                    let mut lift = registry
                        .state(registry.default_state("minecraft:bubble_column").unwrap())
                        .unwrap()
                        .clone();
                    lift.properties.insert("drag".into(), "false".into());
                    let lift = registry.resolve(&lift).unwrap();
                    for x in 0..=2 {
                        for z in 0..=2 {
                            world.set(x, 0, z, stone).unwrap();
                            for y in 1..=6 {
                                world.set(x, y, z, water).unwrap();
                            }
                        }
                    }
                    if column {
                        for y in 1..=6 {
                            world.set(1, y, 1, lift).unwrap();
                        }
                    }
                };
            crate::mobs::world::with_built_test_world(&[], build, |w, types| {
                let kind = std::sync::Arc::new(types.types["minecraft:iron_golem"].clone());
                let mut m = Mob::new(1, kind, Vec3::new(1.5, 2.0, 1.5), 0.0);
                m.fluids.in_water = true;
                m.fluids.water = 1.0;
                m.travel(w, level, Vec3::ZERO, &mut Vec::new());
                let after_travel = m.velocity.y;
                m.apply_effects_from_blocks(w, &mut Vec::new());
                out = Some((after_travel, m.velocity.y));
            });
            out
        };
        let (Some((plain, _)), Some((travelled, lifted))) = (run(false), run(true)) else {
            return;
        };
        assert_eq!(
            travelled, plain,
            "travel alone does not take the column's lift"
        );
        // The golem's 2.7 high box touches three column blocks (y 2, 3, 4).
        assert!(
            (lifted - plain - 3.0 * 0.06).abs() < 1.0e-9,
            "three column blocks lift it by 0.18 after the drag, got {}",
            lifted - plain
        );
    }

    /// `Entity.move`'s last step, `getBlockSpeedFactor`: a mob sliding on soul
    /// sand (0.4) or honey (0.4) keeps 40% of its horizontal speed each move,
    /// on stone it keeps all of it, and its vertical speed is untouched.
    #[test]
    #[ignore = "daily"]
    fn a_mob_on_soul_sand_or_honey_is_slowed_by_the_move() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        for (block, factor) in [
            ("minecraft:stone", 1.0),
            ("minecraft:soul_sand", 0.4),
            ("minecraft:honey_block", 0.4),
        ] {
            let build =
                |world: &mut crate::world::World,
                 registry: &sunrise_world::registry::BlockRegistry| {
                    world.chunks.insert(
                        (0, 0),
                        std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
                    );
                    let id = registry.default_state(block).unwrap();
                    for x in 0..=4 {
                        for z in 0..=2 {
                            world.set(x, 0, z, id).unwrap();
                        }
                    }
                };
            crate::mobs::world::with_built_test_world(&[], build, |w, types| {
                let kind = std::sync::Arc::new(types.types["minecraft:zombie"].clone());
                let mut m = Mob::new(1, kind, Vec3::new(0.5, 1.0, 1.5), 0.0);
                m.ground.on_ground = true;
                m.velocity = Vec3::new(0.2, 0.0, 0.0);
                m.do_move(w, level, &mut Vec::new(), &mut MoveGate::new());
                let expected = f64::from(factor as f32) * 0.2;
                assert!(
                    (m.velocity.x - expected).abs() < 1.0e-9,
                    "{block}: velocity.x {} expected {expected}",
                    m.velocity.x
                );
            });
        }
    }

    #[test]
    #[ignore = "daily"]
    fn an_overflowing_block_speed_product_preserves_the_mobs_finite_velocity() {
        use mod_api::{EntityMoveEvent, Flow, Hooks, LivingTravelEvent};
        use sunrise_core::nbt::{Limits, Tag, read_named};
        use sunrise_world::movement_properties::{MovementProperties, MovementTags};

        // Loading the fixture sizes the process-wide chunk palette.
        const CHILD: &str = "SUNRISE_MOB_TRAVEL_OVERFLOW_CHILD";
        if std::env::var_os(CHILD).is_none() {
            crate::fresh_process::run_in_fresh_process(
                "mobs::living::tests::an_overflowing_block_speed_product_preserves_the_mobs_finite_velocity",
                CHILD,
                "1",
            );
            return;
        }

        struct Fast;
        #[mod_api::__subscriptions]
        impl Hooks for Fast {
            fn living_travel(
                &self,
                _: &dyn mod_api::LoopContext,
                event: &mut LivingTravelEvent<'_>,
            ) -> Flow {
                *event.velocity = [f64::MAX, 0.0, 0.0];
                Flow::Continue
            }

            fn entity_move(
                &self,
                _: &dyn mod_api::LoopContext,
                event: &mut EntityMoveEvent<'_>,
            ) -> Flow {
                // Shorten the displacement without changing the stored velocity.
                *event.delta = [0.5, 0.0, 0.0];
                Flow::Continue
            }
        }

        let root = std::env::var_os("SUNRISE_TEST_RESOURCES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/sunrise-client")
                    .join(sunrise_core::VANILLA_VERSION)
            });
        let bytes = std::fs::read(root.join("movement-properties.nbt")).unwrap();
        let (mut movement_table, _) = read_named(&bytes, Limits::tables()).unwrap();
        // Give air a factor of 2.0 in this world only, like a block's part could.
        *movement_table
            .tag
            .get_mut("blocks")
            .unwrap()
            .get_mut("minecraft:air")
            .unwrap()
            .get_mut("speedFactor")
            .unwrap() = Tag::Float(2.0_f32.to_bits());

        let mods = vec![mod_api::ModEntry::new("fast", Box::new(Fast))].into();
        mod_platform::testing::with_mods(mods, || {
            assert!(crate::mobs::world::with_built_test_world(
                &[],
                |world, _| {
                    world.chunks.insert(
                        (0, 0),
                        std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
                    );
                },
                |w, types| {
                    let movement = MovementProperties::from_nbt(&movement_table.tag, w.registry)
                        .unwrap()
                        .bind(&MovementTags::default())
                        .unwrap();
                    let w = MobWorld {
                        movement: &movement,
                        ..w.clone()
                    };
                    let kind = std::sync::Arc::new(types.types["minecraft:zombie"].clone());
                    let mut mob = Mob::new(1, kind, Vec3::new(0.5, 80.0, 0.5), 0.0);
                    let mut test_level = crate::health::TestLevel::new(0);
                    mob.travel(&w, &mut test_level.level(), Vec3::ZERO, &mut Vec::new());
                    assert_eq!(mob.position, Vec3::new(1.0, 80.0, 0.5));
                    // Entity.move refuses the overflowing product as a whole; only the air drag applies.
                    assert_eq!(mob.velocity.x, f64::MAX * f64::from(0.91_f32));
                },
            ));
        });
    }

    /// `Entity.collectColliders`: the world border's wall stops a mob's move
    /// as it stops the player's (#7080). A border of size 10 around (8, 8)
    /// has its east wall at x = 13.
    #[test]
    #[ignore = "daily"]
    fn the_world_border_stops_a_mob() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let border =
            sunrise_core::geometry::border::BorderGeometry::new([8.0, 8.0], 10.0, 29_999_984)
                .unwrap();
        let build = |world: &mut crate::world::World,
                     _: &sunrise_world::registry::BlockRegistry| {
            world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
            );
        };
        crate::mobs::world::with_built_test_world(&[], build, |w, types| {
            let kind = std::sync::Arc::new(types.types["minecraft:pig"].clone());
            let mut east_edge = [0.0; 2];
            for (i, bordered) in [false, true].into_iter().enumerate() {
                let w = MobWorld {
                    side: crate::motion::dry_side(),
                    goal_host: None,
                    live_mobs: None,
                    border: bordered.then_some(&border),
                    ..w.clone()
                };
                // 1.5 blocks east from x = 11.5: past the wall at 13 if nothing stops it.
                let mut m = Mob::new(1, kind.clone(), Vec3::new(11.5, 64.0, 8.5), 0.0);
                m.velocity = Vec3::new(1.5, 0.0, 0.0);
                m.do_move(&w, level, &mut Vec::new(), &mut MoveGate::new());
                east_edge[i] = m.bounds().max_position().x;
                assert_eq!(m.ground.horizontal_collision, bordered);
            }
            assert!(
                east_edge[0] > 13.0,
                "no border: the pig walks on, {east_edge:?}"
            );
            assert!(
                (east_edge[1] - 13.0).abs() < 1.0e-7,
                "the pig's east side stops on the wall at x = 13, {east_edge:?}"
            );
        });
    }

    /// `LivingEntity.checkFallDamage`'s dust: nothing within the safe distance,
    /// `150 * min(0.2 + power / 15, 2.5)` block particles beyond it.
    #[test]
    fn a_landing_throws_up_dust_by_the_power_of_the_fall() {
        // Within the safe distance of 3, or by a hair over it: power 0.
        assert_eq!(landing_dust_count(2.5, 3.0), None);
        assert_eq!(landing_dust_count(3.0, 3.0), None);
        // A fall of 7: power 4, 150 * (0.2 + 4 / 15) = 70.
        assert_eq!(landing_dust_count(7.0, 3.0), Some(70));
        // A long fall is capped at 150 * 2.5.
        assert_eq!(landing_dust_count(60.0, 3.0), Some(375));
        // A mob that takes no fall damage still has a safe distance of its own.
        assert_eq!(landing_dust_count(7.0, 10.0), None);
    }
    /// #1988: both of `checkFallDamage`'s reads of the fall power take the
    /// safe fall distance attribute, a given Jump Boost's stored modifier
    /// included. A zombie landing on stone after 4.5 blocks takes 1 damage
    /// and throws up `150 * (0.2F + 1 / 15)` = 40 grains; with Jump Boost II
    /// (3 + 2 = 5) it takes none and throws up nothing.
    #[test]
    #[ignore = "daily"]
    fn a_jump_boosted_zombies_landing_reads_its_safe_fall_distance() {
        let build = |world: &mut crate::world::World,
                     registry: &sunrise_world::registry::BlockRegistry| {
            world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
            );
            let stone = registry.default_state("minecraft:stone").unwrap();
            world.set(4, 63, 4, stone).unwrap();
        };
        let ran = crate::mobs::world::with_built_test_world(&[], build, |w, types| {
            let kind = std::sync::Arc::new(types.types["minecraft:zombie"].clone());
            let land = |boosted: bool| {
                let mut m = Mob::new(1, kind.clone(), Vec3::new(4.5, 64.0, 4.5), 0.0);
                if boosted {
                    m.add_effect("minecraft:jump_boost", -1, 1);
                }
                m.ground.on_ground = true;
                m.fall_distance = 4.5;
                let mut test_level = crate::health::TestLevel::new(0);
                let mut events = Vec::new();
                m.check_fall_damage(
                    w,
                    &mut test_level.level(),
                    Vec3::ZERO,
                    m.restitution_contact(w),
                    &mut events,
                    LandingOwner::Synchronous,
                );
                let dust: Vec<i32> = events
                    .iter()
                    .filter_map(|e| match e {
                        MobEvent::LandingDust { count, .. } => Some(*count),
                        _ => None,
                    })
                    .collect();
                (m.health, dust)
            };
            assert_eq!(land(false), (19.0, vec![40]));
            assert_eq!(land(true), (20.0, vec![]));
        });
        assert!(ran, "the vanilla test resources must be available");
    }
    /// Over the block it lands on the dust starts at the mob; straddling
    /// into another column it is pulled to that block's near edge.
    #[test]
    fn landing_dust_is_pulled_to_the_block_it_landed_on() {
        let on = Vec3::new(4.3, 65.0, 7.6);
        assert_eq!(landing_dust_at(on, [4, 64, 7]), on);
        // The mob stands over block (4, 7) but its feet block is (5, 7).
        let straddling = landing_dust_at(Vec3::new(5.1, 65.0, 7.5), [4, 64, 7]);
        assert_eq!(straddling, Vec3::new(5.0, 65.0, 7.5));
    }
    /// `LivingEntity.getDimensions` is `getDefaultDimensions(pose).scale(
    /// getScale())` on every living entity, not only on a player or mannequin
    /// whose pose sets the box (`Avatar.POSES`): a cow's `EntityType.COW` box
    /// (0.9 x 1.4, its eyes at 1.3) at `minecraft:scale` 2 is 1.8 x 2.8 with
    /// the eyes at 2.6, and the box `Mob.bounds` hands the hitbox raycast, the
    /// attack range and the leash pins grows with it.
    #[test]
    fn a_scale_attribute_grows_every_living_entitys_box() {
        let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
        let mut cow = Mob::new(1, crate::mobs::sensor_scan_tests::kind(), Vec3::ZERO, 0.0);
        let (w, h, e) = cow.dims();
        assert!(
            close(w, 0.9) && close(h, 1.4) && close(e, 1.3),
            "no scale: {w}x{h}x{e}"
        );
        cow.extra.model_scale = 2.0;
        let (w, h, e) = cow.dims();
        assert!(
            close(w, 1.8) && close(h, 2.8) && close(e, 2.6),
            "scale 2: {w}x{h}x{e}"
        );
        let b = cow.bounds();
        assert!(
            close(b.min_position().x, -0.9)
                && close(b.max_position().x, 0.9)
                && close(b.max_position().y, 2.8),
            "scale 2 box: {b:?}"
        );
        // `RangedAttribute("attribute.name.scale", 1.0, 0.0625, 16.0)`: its
        // floor shrinks the cow to 0.05625 x 0.0875, eyes at 0.08125.
        cow.extra.model_scale = 0.0625;
        let (w, h, e) = cow.dims();
        assert!(
            close(w, 0.05625) && close(h, 0.0875) && close(e, 0.08125),
            "scale 0.0625: {w}x{h}x{e}"
        );
    }
    /// `LivingEntity.getDimensions` answers a sleeping entity with
    /// `SLEEPING_DIMENSIONS` before it ever reaches `getScale`: a sleeper's
    /// 0.2 box and eye height stay 0.2 whatever its `minecraft:scale`.
    #[test]
    fn a_sleeping_mob_keeps_its_bed_box_unscaled() {
        use crate::mobs::Species;
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.id = "minecraft:villager".into();
        kind.species = Species::Villager;
        let mut villager = Mob::new(1, std::sync::Arc::new(kind), Vec3::ZERO, 0.0);
        // Villager.sleeping: the bed's facing code, not 0.
        villager.extra.ints[1] = 1;
        villager.extra.model_scale = 4.0;
        let (w, h, e) = villager.dims();
        let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
        assert!(
            close(w, 0.2) && close(h, 0.2) && close(e, 0.2),
            "asleep: {w}x{h}x{e}"
        );
    }
    /// `LivingEntity.sanitizeScale`: a shulker takes at most 3 and a happy
    /// ghast at most 1, and the box takes the capped value (`EntityType.SHULKER`
    /// is 1 x 1 with its eyes at 0.5).
    #[test]
    fn a_species_caps_the_scale_it_takes() {
        use crate::mobs::Species;
        let mob = |id: &str, species: Species| {
            let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
            kind.id = id.into();
            kind.species = species;
            (kind.width, kind.height, kind.eye_height) = (1.0, 1.0, 0.5);
            Mob::new(1, std::sync::Arc::new(kind), Vec3::ZERO, 0.0)
        };
        let mut shulker = mob("minecraft:shulker", Species::Shulker);
        shulker.extra.model_scale = 4.0;
        assert_eq!(shulker.model_scale(), 3.0);
        let (w, h, e) = shulker.dims();
        let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
        assert!(
            close(w, 3.0) && close(h, 3.0) && close(e, 1.5),
            "shulker: {w}x{h}x{e}"
        );
        let mut ghast = mob("minecraft:happy_ghast", Species::HappyGhast);
        ghast.extra.model_scale = 2.0;
        assert_eq!(ghast.model_scale(), 1.0);
        // Every other species takes the attribute as it came.
        let mut cow = mob("minecraft:cow", Species::Cow);
        cow.extra.model_scale = 4.0;
        assert_eq!(cow.model_scale(), 4.0);
    }
    /// `Sniffer.jumpFromGround`: a hop the move control asked for, taken from
    /// nearly standing still, carries the sniffer a tenth of a block along its
    /// yaw (`moveRelative(0.1F, (0, 0, 1))`). A sniffer that already moves, one
    /// the move control did not ask to move, and every other species keep the
    /// hop they had.
    #[test]
    #[ignore = "daily"]
    fn a_sniffers_hop_from_a_standstill_carries_it_forward() {
        use crate::mobs::Species;
        let sniffer = |speed: f64, sideways: f64| {
            let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
            kind.id = "minecraft:sniffer".into();
            kind.species = Species::Sniffer;
            kind.jump_strength = 0.42;
            let mut m = Mob::new(1, std::sync::Arc::new(kind), Vec3::new(0.5, 64.0, 0.5), 0.0);
            // `MoveControl.getSpeedModifier` and the speed it moves at.
            m.ai.mov.speed_modifier = speed;
            m.velocity = Vec3::new(0.0, 0.0, sideways);
            m
        };
        // Yaw 0 faces south (+z), so the nudge lands on z.
        let hop = |m: &mut Mob| {
            let mut jumped = false;
            if crate::mobs::world::with_test_world(&[], |w, _| m.jump_from_ground(w)) {
                jumped = true;
            }
            jumped
        };
        let mut m = sniffer(1.0, 0.0);
        if !hop(&mut m) {
            return;
        }
        assert!(
            (m.velocity.z - 0.1).abs() < 1.0e-6 && m.velocity.x.abs() < 1.0e-6,
            "the hop goes forward: {:?}",
            m.velocity
        );
        assert!(m.velocity.y > 0.0, "and it is still a hop");
        // Already moving sideways: `horizontalDistanceSqr` is over the 0.01.
        let mut moving = sniffer(1.0, 0.2);
        assert!(hop(&mut moving));
        assert!(
            (moving.velocity.z - 0.2).abs() < 1.0e-6,
            "{:?}",
            moving.velocity
        );
        // The move control wanted nothing this tick.
        let mut still = sniffer(0.0, 0.0);
        assert!(hop(&mut still));
        assert_eq!(still.velocity.z, 0.0);
        // No other species gets the nudge.
        let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
        kind.jump_strength = 0.42;
        let mut cow = Mob::new(1, std::sync::Arc::new(kind), Vec3::new(0.5, 64.0, 0.5), 0.0);
        cow.ai.mov.speed_modifier = 1.0;
        assert!(hop(&mut cow));
        assert_eq!(cow.velocity.z, 0.0);
    }

    /// #8140: `LivingEntity.handleOnClimbable` slows any mob in vines, not
    /// only a spider on a wall. A cow let go at rest moves before gravity
    /// pulls it: 0, then 0.0784, then 0.155232 in open air ((v - 0.08) *
    /// 0.98); in vines each move down from the third on is held to 0.15F,
    /// and the fall distance, reset before each move, is that one move.
    #[test]
    #[ignore = "daily"]
    fn a_mob_in_vines_slides_down_slowly() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        let mut run = |vines: bool| {
            let mut out = None;
            let build =
                |world: &mut crate::world::World,
                 registry: &sunrise_world::registry::BlockRegistry| {
                    let stone = registry.default_state("minecraft:stone").unwrap();
                    let mut vine = registry
                        .state(registry.default_state("minecraft:vine").unwrap())
                        .unwrap()
                        .clone();
                    vine.properties.insert("south".into(), "true".into());
                    let vine = registry.resolve(&vine).unwrap();
                    world.chunks.insert(
                        (0, 0),
                        std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
                    );
                    world.set(1, 0, 1, stone).unwrap();
                    for y in 1..=12 {
                        world.set(1, y, 2, stone).unwrap();
                        if vines {
                            world.set(1, y, 1, vine).unwrap();
                        }
                    }
                };
            crate::mobs::world::with_built_test_world(&[], build, |w, types| {
                let kind = std::sync::Arc::new(types.types["minecraft:cow"].clone());
                let mut m = Mob::new(1, kind, Vec3::new(1.5, 10.0, 1.5), 0.0);
                m.ground.on_ground = false;
                let mut drops = Vec::new();
                for _ in 0..12 {
                    let before = m.position.y;
                    m.travel(w, level, Vec3::ZERO, &mut Vec::new());
                    drops.push(before - m.position.y);
                }
                out = Some((drops, m.fall_distance));
            });
            out
        };
        let (Some((open, open_fall)), Some((vines, vine_fall))) = (run(false), run(true)) else {
            return;
        };
        assert_eq!(open[0], 0.0, "open air, tick 1: {open:?}");
        assert!(
            (open[1] - 0.0784).abs() < 1.0e-6,
            "open air, tick 2: {open:?}"
        );
        assert!(
            (open[2] - 0.155232).abs() < 1.0e-6,
            "open air, tick 3: {open:?}"
        );
        assert!(
            open[11] > 0.5 && open_fall > 3.0,
            "open air keeps speeding up: {open:?}"
        );
        let limit = f64::from(0.15f32);
        assert_eq!(vines[0], 0.0, "vines, tick 1: {vines:?}");
        assert!(
            (vines[1] - 0.0784).abs() < 1.0e-6,
            "vines, tick 2: {vines:?}"
        );
        for (tick, drop) in vines.iter().enumerate().skip(2) {
            assert!(
                (drop - limit).abs() < 1.0e-9,
                "vines, tick {}: {vines:?}",
                tick + 1
            );
        }
        // Reset before the move, then the move's own drop: never more.
        assert!(
            (vine_fall - limit).abs() < 1.0e-9,
            "the vines reset the fall each tick, got {vine_fall}"
        );
    }

    /// `LivingEntity.onClimbable`: `isFallFlying` in a block of
    /// `#minecraft:can_glide_through` (vines) answers false before the
    /// climbable test; a ladder is not in that tag, so a glider still holds
    /// on to it.
    #[test]
    #[ignore = "daily"]
    fn a_gliding_mob_passes_through_vines_but_not_a_ladder() {
        let build = |world: &mut crate::world::World,
                     registry: &sunrise_world::registry::BlockRegistry| {
            world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
            );
            let vine = registry.default_state("minecraft:vine").unwrap();
            let ladder = registry.default_state("minecraft:ladder").unwrap();
            world.set(1, 5, 1, vine).unwrap();
            world.set(3, 5, 1, ladder).unwrap();
        };
        crate::mobs::world::with_built_test_world(&[], build, |w, types| {
            let kind = std::sync::Arc::new(types.types["minecraft:zombie"].clone());
            for (x, block, gliding_holds) in [(1.5, "vine", false), (3.5, "ladder", true)] {
                let mut m = Mob::new(1, kind.clone(), Vec3::new(x, 5.0, 1.5), 0.0);
                assert!(m.on_climbable(w), "{block}: climbable when not gliding");
                m.extra.fall_flying = true;
                assert_eq!(m.on_climbable(w), gliding_holds, "{block}: while gliding");
            }
        });
    }
    #[test]
    #[ignore = "daily"]
    fn damaging_mob_fall_plays_block_fall_sound() {
        let ran = crate::mobs::world::with_built_test_world(
            &[],
            |world, registry| {
                world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
                );
                for (x, block) in [(0, "minecraft:stone"), (1, "minecraft:oak_planks")] {
                    world
                        .set(x, 0, 0, registry.default_state(block).unwrap())
                        .unwrap();
                }
            },
            |w, types| {
                for (x, block_sound) in [
                    (0.5, "minecraft:block.stone.fall"),
                    (1.5, "minecraft:block.wood.fall"),
                ] {
                    let kind = std::sync::Arc::new(types.types["minecraft:zombie"].clone());
                    let mut mob = Mob::new(1, kind, Vec3::new(x, 1.0, 0.5), 0.0);
                    mob.fall_distance = 5.0;
                    let mut events = Vec::new();
                    mob.fall_on_block(
                        w,
                        &mut crate::health::TestLevel::new(0).level(),
                        mob.restitution_contact(w),
                        &mut events,
                        LandingOwner::Synchronous,
                    );
                    assert!(
                        matches!(events.as_slice(), [MobEvent::Sound("entity.hostile.small_fall", _, 1.0, 1.0), MobEvent::SoundId(id, _, 0.5, 0.75), ..] if id == block_sound),
                        "{events:?}"
                    );
                }
            },
        );
        // Without the vanilla data under .local/sunrise-client the world
        // cannot be built: the test has nothing to check.
        assert!(ran, "vanilla data is missing");
    }

    /// `Entity.applyMovementEmissionAndPlaySound` asks `getMovementEmission`:
    /// a squid, a glow squid and a bat answer `EVENTS`, so a beached squid
    /// flopping across stone sends the `minecraft:step` game event and no
    /// sound, where a sheep walking the same block is heard at the block's
    /// step volume times 0.15. A silverfish's own `playStepSound` names
    /// `entity.silverfish.step` and is never heard either, and a shulker, whose
    /// emission is `NONE`, sends nothing at all.
    #[test]
    #[ignore = "daily"]
    fn a_squid_and_a_bat_step_without_a_sound() {
        let build = |world: &mut crate::world::World,
                     registry: &sunrise_world::registry::BlockRegistry| {
            world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
            );
            let id = registry.default_state("minecraft:stone").unwrap();
            for x in 0..=8 {
                for z in 0..=8 {
                    world.set(x, 0, z, id).unwrap();
                }
            }
        };
        crate::mobs::world::with_built_test_world(&[], build, |w, types| {
            // One step's worth of walking: 2 blocks times the 0.6 factor, over
            // the 1.0 the first step waits for.
            let walk = |id: &str| {
                let kind = std::sync::Arc::new(types.types[id].clone());
                let mut m = Mob::new(1, kind, Vec3::new(0.5, 1.0, 0.5), 0.0);
                m.ground.on_ground = true;
                m.ground.supporting_block = Some(sunrise_core::position::BlockPos::new(0, 0, 0));
                let mut events = Vec::new();
                m.movement_emission(w, Vec3::new(2.0, 0.0, 0.0), &mut events);
                events
            };
            let heard = |events: &[MobEvent]| {
                events.iter().find_map(|e| match e {
                    MobEvent::Sound(sound, _, volume, pitch) => Some((*sound, *volume, *pitch)),
                    _ => None,
                })
            };
            let stepped = |events: &[MobEvent]| {
                events
                    .iter()
                    .any(|e| matches!(e, MobEvent::GameEvent { event, .. } if *event == "minecraft:step"))
            };
            for quiet in ["minecraft:squid", "minecraft:glow_squid", "minecraft:bat"] {
                let events = walk(quiet);
                assert_eq!(heard(&events), None, "{quiet} is silent");
                assert!(stepped(&events), "{quiet} still sends the step event");
            }
            // A silverfish asks for its own step sound and its type's emission
            // never plays it.
            let events = walk("minecraft:silverfish");
            assert_eq!(heard(&events), None);
            assert!(stepped(&events));
            // The same walk from a type whose emission is ALL is heard.
            let events = walk("minecraft:sheep");
            assert_eq!(heard(&events), Some(("entity.sheep.step", 0.15, 1.0)));
            // Shulker.getMovementEmission is NONE: not even the event.
            let events = walk("minecraft:shulker");
            assert_eq!(heard(&events), None);
            assert!(!stepped(&events), "a shulker emits nothing");
        });
    }

    /// #9058: `LivingEntity.getJumpPower` calculates
    /// `jump_strength * multiplier * getBlockJumpFactor() + getJumpBoostPower()`.
    /// On honey blocks the jump factor is 0.5, halving the jump impulse.
    /// Jump Boost effects add 0.1 * (amplifier + 1.0).
    #[test]
    #[ignore = "daily"]
    fn living_jump_power_respects_honey_block_and_jump_boost() {
        let build = |world: &mut crate::world::World,
                     registry: &sunrise_world::registry::BlockRegistry| {
            let stone = registry.default_state("minecraft:stone").unwrap();
            let honey = registry.default_state("minecraft:honey_block").unwrap();
            world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
            );
            // Stone column at (0, 0, 0)
            world.set(0, 0, 0, stone).unwrap();
            // Honey column at (2, 0, 0)
            world.set(2, 0, 0, honey).unwrap();
        };
        let ran = crate::mobs::world::with_built_test_world(&[], build, |w, types| {
            let cow_kind = std::sync::Arc::new(types.types["minecraft:cow"].clone());
            let close = |a: f32, b: f32| (a - b).abs() < 1.0e-5;

            // 1. Baseline: On normal block (stone, factor 1.0) with no effects, jump power matches m.attrs.jump_strength.
            let mut on_stone = Mob::new(1, cow_kind.clone(), Vec3::new(0.5, 1.0, 0.5), 0.0);
            assert!((on_stone.attrs.jump_strength - 0.42).abs() < 1.0e-5);
            assert!(close(on_stone.block_jump_factor(w), 1.0));
            assert_eq!(on_stone.jump_boost_power(), 0.0);
            assert!(close(on_stone.jump_power(w), 0.42));
            on_stone.jump_from_ground(w);
            assert!(
                (on_stone.velocity.y - 0.42).abs() < 1.0e-5,
                "stone baseline velocity.y"
            );

            // 2. Blocked negative / Reduction on honey: On honey block (jump factor 0.5), jump power is halved.
            let mut on_honey = Mob::new(2, cow_kind.clone(), Vec3::new(2.5, 1.0, 0.5), 0.0);
            assert!(
                close(on_honey.block_jump_factor(w), 0.5),
                "honey jump factor must be 0.5"
            );
            assert!(
                close(on_honey.jump_power(w), 0.42 * 0.5),
                "honey halving jump power"
            );
            on_honey.jump_from_ground(w);
            assert!(
                (on_honey.velocity.y - 0.21).abs() < 1.0e-5,
                "honey reduced velocity.y"
            );

            // 3. Enabled positive / Jump boost: With minecraft:jump_boost effect (amp 0 and amp 1),
            // jump power increases by +0.1 and +0.2 respectively.
            let mut boosted_0 = Mob::new(3, cow_kind.clone(), Vec3::new(0.5, 1.0, 0.5), 0.0);
            boosted_0.extra.effects.push(crate::mobs::StoredEffect::new(
                "minecraft:jump_boost",
                200,
                0,
            ));
            assert!(close(boosted_0.jump_boost_power(), 0.1));
            assert!(close(boosted_0.jump_power(w), 0.42 + 0.1));
            boosted_0.jump_from_ground(w);
            assert!((boosted_0.velocity.y - 0.52).abs() < 1.0e-5);

            let mut boosted_1 = Mob::new(4, cow_kind.clone(), Vec3::new(0.5, 1.0, 0.5), 0.0);
            boosted_1.extra.effects.push(crate::mobs::StoredEffect::new(
                "minecraft:jump_boost",
                200,
                1,
            ));
            assert!(close(boosted_1.jump_boost_power(), 0.2));
            assert!(close(boosted_1.jump_power(w), 0.42 + 0.2));
            boosted_1.jump_from_ground(w);
            assert!((boosted_1.velocity.y - 0.62).abs() < 1.0e-5);

            // 4. Edge / Special case: Combined honey block + jump boost and custom modified attrs.jump_strength.
            let mut combined = Mob::new(5, cow_kind.clone(), Vec3::new(2.5, 1.0, 0.5), 0.0);
            combined.extra.effects.push(crate::mobs::StoredEffect::new(
                "minecraft:jump_boost",
                200,
                1,
            ));
            // jump_strength * 0.5 + 0.2 = 0.42 * 0.5 + 0.2 = 0.41
            assert!(close(combined.jump_power(w), 0.42 * 0.5 + 0.2));
            combined.jump_from_ground(w);
            assert!((combined.velocity.y - 0.41).abs() < 1.0e-5);

            // Custom modified attribute (e.g. 0.80) on honey with jump boost 0 (0.1)
            let mut custom_attr = Mob::new(6, cow_kind, Vec3::new(2.5, 1.0, 0.5), 0.0);
            custom_attr.attrs.jump_strength = 0.80;
            custom_attr
                .extra
                .effects
                .push(crate::mobs::StoredEffect::new(
                    "minecraft:jump_boost",
                    200,
                    0,
                ));
            // 0.80 * 0.5 + 0.1 = 0.50
            assert!(close(custom_attr.jump_power(w), 0.50));
            custom_attr.jump_from_ground(w);
            assert!((custom_attr.velocity.y - 0.50).abs() < 1.0e-5);

            // 5. Cube jump verification (Slime & MagmaCube on honey vs stone)
            let slime_kind = std::sync::Arc::new(types.types["minecraft:slime"].clone());
            let mut slime_stone = Mob::new(7, slime_kind.clone(), Vec3::new(0.5, 1.0, 0.5), 0.0);
            slime_stone.jump_from_ground(w);
            assert!((slime_stone.velocity.y - 0.42).abs() < 1.0e-5);

            let mut slime_honey = Mob::new(8, slime_kind, Vec3::new(2.5, 1.0, 0.5), 0.0);
            slime_honey.jump_from_ground(w);
            assert!(
                (slime_honey.velocity.y - 0.21).abs() < 1.0e-5,
                "slime jump on honey"
            );

            let magma_kind = std::sync::Arc::new(types.types["minecraft:magma_cube"].clone());
            let mut magma_honey = Mob::new(9, magma_kind, Vec3::new(2.5, 1.0, 0.5), 0.0);
            magma_honey.extra.size = 2; // boost = 2 * 0.1 = 0.2
            magma_honey.jump_from_ground(w);
            // 0.42 * 0.5 + 0.2 = 0.41
            assert!(
                (magma_honey.velocity.y - 0.41).abs() < 1.0e-5,
                "magma cube jump on honey"
            );
        });
        assert!(ran, "built test world ran");
    }

    /// `LivingEntity.jumpFromGround` ends with NeoForge's `LivingJumpEvent`: `living_jump`
    /// hears a cow's jump once, with the motion the jump left it. The rabbit's and the sniffer's
    /// overrides call it first, so their own forward push from standing comes after the event.
    #[test]
    #[ignore = "daily"]
    fn a_mob_jump_fires_living_jump_once_with_its_velocity() {
        use crate::native_client_hooks::test_hooks::{install, jumps, record_jumps};
        install();
        let _recording = record_jumps();
        let ran = crate::mobs::world::with_test_world(&[], |w, types| {
            let mob = |id, kind: &str| {
                let kind = std::sync::Arc::new(types.types[kind].clone());
                Mob::new(id, kind, Vec3::new(0.5, 1.0, 0.5), 0.0)
            };
            let mut cow = mob(7, "minecraft:cow");
            cow.jump_from_ground(w);
            assert_eq!(jumps(), [(7, [0.0, f64::from(0.42_f32), 0.0])]);
            for (id, kind) in [(8, "minecraft:rabbit"), (9, "minecraft:sniffer")] {
                let mut m = mob(id, kind);
                m.ai.mov.speed_modifier = 1.0;
                m.jump_from_ground(w);
                let heard = jumps();
                assert_eq!(heard.len(), 1, "{kind}");
                let (entity, [x, y, z]) = heard[0];
                assert_eq!(
                    (entity, x, z),
                    (id, 0.0, 0.0),
                    "{kind} heard before its push"
                );
                assert!(y > 0.0, "{kind} jumped");
                assert!(m.velocity.z > 0.0, "{kind} pushed forward after the event");
            }
        });
        assert!(ran, "test world ran");
    }

    /// `LivingEntity.getFlyingSpeed`: a horse its player steers (saddled, ridden) moves
    /// through the air at a tenth of its speed; riderless or unsaddled at 0.02.
    #[test]
    fn a_steered_mount_moves_through_the_air_at_a_tenth_of_its_speed() {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        std::sync::Arc::make_mut(&mut kind).species = super::super::Species::Horse;
        let mut m = Mob::new(1, kind, Vec3::ZERO, 0.0);
        // Flying speed needs only the hook context, not prepared vanilla resources.
        let context = crate::test_resources::server_loop_context();
        let side = crate::motion::dry_side();
        m.ai.speed = 0.3;
        assert_eq!(m.flying_speed(context, side), 0.02, "no rider");
        m.rider = Some(super::super::equine::Rider::default());
        assert_eq!(m.flying_speed(context, side), 0.02, "unsaddled");
        m.extra.set_saddled(true);
        assert_eq!(m.flying_speed(context, side), 0.3_f32 * 0.1);
    }
    /// `Drowned.isPushedByFluid` is `!isSwimming()`; water animals, turtles and nautiluses answer false, land mobs true.
    #[test]
    #[ignore = "daily"]
    fn a_swimming_drowned_is_not_pushed_by_the_current() {
        let ran = crate::mobs::world::with_test_world(&[], |_, types| {
            let mob = |id: &str| {
                let kind = std::sync::Arc::new(types.types[id].clone());
                Mob::new(1, kind, Vec3::ZERO, 0.0)
            };
            // `extra.flag` is the drowned's `isSwimming`, as `drowned_update_swimming` sets it.
            let mut drowned = mob("minecraft:drowned");
            drowned.extra.flag = true;
            assert!(!drowned.pushed_by_fluid(), "a swimming drowned");
            drowned.extra.flag = false;
            assert!(drowned.pushed_by_fluid(), "a drowned walking the bottom");
            for id in [
                "minecraft:cod",
                "minecraft:squid",
                "minecraft:turtle",
                "minecraft:nautilus",
            ] {
                assert!(!mob(id).pushed_by_fluid(), "{id}");
            }
            assert!(mob("minecraft:zombie").pushed_by_fluid());
        });
        assert!(ran, "the vanilla test resources must be available");
    }
    /// `Entity.getFluidInteractionBox` through `AbstractBoat.modifyPassengerFluidInteractionBox`, on a drowned over a lake.
    #[test]
    #[ignore = "daily"]
    fn a_mob_in_a_boat_on_water_is_not_in_the_water() {
        let build = |world: &mut crate::world::World,
                     registry: &sunrise_world::registry::BlockRegistry| {
            world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
            );
            let water = registry.default_state("minecraft:water").unwrap();
            for x in 0..=4 {
                for z in 0..=4 {
                    for y in 1..=3 {
                        world.set(x, y, z, water).unwrap();
                    }
                }
            }
        };
        let boat = |under_water, max_y| crate::motion::RiddenBoat { under_water, max_y };
        let ran = crate::mobs::world::with_built_test_world(&[], build, |w, types| {
            let kind = std::sync::Arc::new(types.types["minecraft:drowned"].clone());
            let wet = |mount: Option<u32>, ridden: Option<crate::motion::RiddenBoat>| {
                let mut m = Mob::new(1, kind.clone(), Vec3::new(2.5, 3.5, 2.5), 0.0);
                m.extra.mount = mount;
                m.fluids.boat = ridden.map(|b| (7, b));
                m.update_fluids(w, &mut Vec::new());
                (m.fluids.in_water, m.fluids.water)
            };
            assert!(wet(None, None).0, "no boat");
            assert_eq!(
                wet(Some(7), Some(boat(false, 3.95))),
                (false, 0.0),
                "a boat afloat raises the box's bottom above the surface"
            );
            assert_eq!(
                wet(Some(7), Some(boat(false, 6.0))),
                (false, 0.0),
                "a boat reaching its head leaves no box"
            );
            assert!(wet(Some(7), Some(boat(true, 3.95))).0, "a boat under water");
            assert!(wet(None, Some(boat(false, 3.95))).0, "a boat it left");
        });
        assert!(ran, "the vanilla test resources must be available");
    }
    /// `Strider.checkFallDamage` resets the fall in lava before `LivingEntity`'s runs; a pig in that lava, or a strider on stone, takes it.
    #[test]
    #[ignore = "daily"]
    fn a_strider_landing_in_lava_takes_no_fall_damage() {
        use std::sync::Arc;
        let landed = |id: &str, lava: bool| -> (f32, f64, Vec<i32>) {
            let mut out = None;
            let ran = crate::mobs::world::with_built_test_world(
                &[],
                move |world, registry| {
                    let stone = registry.default_state("minecraft:stone").unwrap();
                    let lava_state = registry.default_state("minecraft:lava").unwrap();
                    for cx in -1..1 {
                        for cz in -1..1 {
                            world
                                .chunks
                                .insert((cx, cz), Arc::new(crate::chunk::Chunk::empty(cx, cz)));
                        }
                    }
                    for x in -4..4 {
                        for z in -4..4 {
                            world.set(x, 1, z, stone).unwrap();
                            if lava {
                                for y in 2..=4 {
                                    world.set(x, y, z, lava_state).unwrap();
                                }
                            }
                        }
                    }
                },
                |w, types| {
                    let kind = Arc::new(types.types[id].clone());
                    let mut m = Mob::new(1, kind, Vec3::new(0.5, 2.0, 0.5), 0.0);
                    let mut events = Vec::new();
                    // `Entity.baseTick` set the fluid state before the move, and ended the first tick.
                    m.update_fluids(w, &mut events);
                    m.first_tick = false;
                    m.ground.on_ground = true;
                    m.update_ground_support(w, Vec3::ZERO);
                    let effect = m.restitution_contact(w);
                    m.fall_distance = 6.0;
                    let start = m.health;
                    let mut test_level = crate::health::TestLevel::new(0);
                    m.species_check_fall_damage(
                        w,
                        &mut test_level.level(),
                        Vec3::ZERO,
                        effect,
                        &mut events,
                        LandingOwner::Synchronous,
                    );
                    let dust = events
                        .iter()
                        .filter_map(|e| match e {
                            MobEvent::LandingDust { count, .. } => Some(*count),
                            _ => None,
                        })
                        .collect();
                    out = Some((start - m.health, m.fall_distance, dust));
                },
            );
            assert!(ran, "the vanilla test resources must be available");
            out.unwrap()
        };
        assert_eq!(
            landed("minecraft:strider", true),
            (0.0, 0.0, vec![]),
            "a strider in lava"
        );
        // `calculateFallDamage`: floor(6 + 1e-6 - 3) = 3; the dust is (int) (150 * (0.2F + 3 / 15.0)) = 60.
        assert_eq!(
            landed("minecraft:pig", true),
            (3.0, 0.0, vec![60]),
            "a pig in the same lava"
        );
        assert_eq!(
            landed("minecraft:strider", false),
            (3.0, 0.0, vec![60]),
            "a strider on stone"
        );
    }
}

#[path = "living/fluid_heights.rs"]
mod fluid_heights;
pub mod section_writes;
#[cfg(test)]
#[path = "living/size_answer_tests.rs"]
mod size_answer_tests;
pub mod teleports;

#[cfg(test)]
mod farmland_event_tests {
    use super::*;
    #[test]
    #[ignore = "daily"]
    fn a_mob_trampling_farmland_posts_block_change_from_it() {
        let ran = crate::mobs::world::with_built_test_world(
            &[],
            |world, registry| {
                world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
                );
                let state = registry.default_state("minecraft:farmland").unwrap();
                world.set(0, 0, 0, state).unwrap();
            },
            |w, types| {
                let dirt = w.registry.default_state("minecraft:dirt").unwrap();
                let kind = std::sync::Arc::new(types.types["minecraft:zombie"].clone());
                let mut mob = Mob::new(7, kind, Vec3::new(0.5, 1.0, 0.5), 0.0);
                mob.fall_distance = 5.0;
                let mut events = Vec::new();
                let mut damage = crate::health::TestLevel::new(0);
                mob.fall_on_block(
                    w,
                    &mut damage.level(),
                    mob.restitution_contact(w),
                    &mut events,
                    LandingOwner::Synchronous,
                );
                // The write, then the event the rule posts for it.
                assert!(
                    matches!(
                        events.as_slice(),
                        [
                            MobEvent::SetBlock {
                                pos: [0, 0, 0],
                                state,
                            },
                            MobEvent::LandingGameEvent {
                                event: "minecraft:block_change",
                                at: [0, 0, 0],
                                source: Some(7),
                                state: affected,
                            },
                            ..
                        ] if *state == dirt && *affected == dirt
                    ),
                    "{events:?}"
                );
            },
        );
        // Without the vanilla data under .local/sunrise-client the world
        // cannot be built: the test has nothing to check.
        assert!(ran, "vanilla data is missing");
    }

    #[test]
    fn set_velocity_and_move_relative_follow_vanilla_finiteness_and_normalization() {
        let mut mob = Mob::new(1, crate::mobs::sensor_scan_tests::kind(), Vec3::ZERO, 0.0);
        mob.set_velocity(Vec3::new(80.0, -70.0, 0.0));
        assert_eq!(mob.velocity, Vec3::new(80.0, -70.0, 0.0));

        for bad in [
            Vec3::new(f64::NAN, 0.0, 0.0),
            Vec3::new(0.0, f64::INFINITY, 0.0),
            Vec3::new(0.0, 0.0, f64::NEG_INFINITY),
        ] {
            mob.set_velocity(bad);
            assert_eq!(mob.velocity, Vec3::new(80.0, -70.0, 0.0));
            mob.move_relative(0.1, bad);
            assert_eq!(mob.velocity, Vec3::new(80.0, -70.0, 0.0));
        }

        mob.set_velocity(Vec3::ZERO);
        mob.move_relative(0.1, Vec3::new(2.0, 0.0, 0.0));
        assert_eq!(mob.velocity, Vec3::new(f64::from(0.1_f32), 0.0, 0.0));
    }
}

#[cfg(test)]
#[path = "living_climbable_tests.rs"]
mod climbable_tests;
fn fall_reset_hit(w: &MobWorld, from: Vec3, moved: Vec3) -> bool {
    let reset = w.block_tags.get("minecraft:fall_damage_resetting");
    crate::entities::fall_reset_hit(
        w.world,
        w.fluid_tables,
        |p| {
            let name = &w.facts.get(w.world.block(p.x, p.y, p.z)).name;
            reset.is_some_and(|t| t.contains(&**name))
        },
        from,
        moved,
    )
}

#[cfg(test)]
#[path = "name_tag_nbt_tests.rs"]
mod name_tag_nbt_tests;
