//! Non-player entities simulated on the client world. Dropped items follow
//! `ItemEntity.tick`: gravity 0.04, 0.98F drag (block friction on the ground),
//! a ground bounce of -0.5, block landing restitution, piston slime launches,
//! water buoyancy, pickup delay, 6000-tick despawn,
//! `HoneyBlock.entityInside` when falling against a honey block's side, and
//! `BubbleColumnBlock.entityInside` (lift / drag, including the surface jump).
use crate::entity_rules::EntityRules;
use crate::{
    blocks::{BlockFacts, Fluid},
    collisions::{EntityCollisions, SceneCollisions},
    items::{ItemStack, Items},
    motion::block_friction,
    world::World,
};
use sunrise_core::{
    geometry::{Aabb, Direction, Vec3, VoxelShape, border::BorderGeometry},
    position::BlockPos,
    random::{LegacyRandom, RandomSource},
};
use sunrise_world::{
    block_collision::BlockCollisionRegistry,
    level::portal_travel::{BorderBounds, find_free_position},
    movement_properties::BoundMovementProperties,
};

/// `Entity.move`: the position only changes when the collided step is longer
/// than `sqrt(1e-7)` or nothing was cut off; a tiny step against a wall or floor
/// is dropped (its collision effects still run).
pub fn move_applies(wanted: Vec3, moved: Vec3) -> bool {
    let moved_sq = moved.length_squared();
    moved_sq > 1.0e-7 || wanted.length_squared() - moved_sq < 1.0e-7
}

/// `SlimeBlock.stepOn`: a slow walk or slide on slime drags the horizontal
/// speed (`0.4 + |dy| * 0.2`); a fast vertical speed leaves it alone.
pub fn slime_step_on_scale(velocity_y: f64) -> Option<f64> {
    let abs_delta_y = velocity_y.abs();
    (abs_delta_y < 0.1).then_some(0.4 + abs_delta_y * 0.2)
}

pub mod arrows;
mod fluids;
mod item_environment;
mod item_fall;
mod item_merge;
mod item_move;
mod item_no_physics;
pub mod nameplate;
mod obstacles;
pub use fluids::{
    fall_reset_hit, fluid_flow, scan_entity_fluids, swept_fluid_contact, update_entity_fluids,
    water_clip,
};
pub use item_merge::{ItemCells, merge_with_neighbours, merge_with_neighbours_recording};
pub use obstacles::ObstacleGrid;

#[cfg(test)]
mod drop_motion_tests {
    use super::drop_motion;
    use sunrise_core::random::{LegacyRandom, RandomSource};

    /// Vanilla's own `Mth` and `LegacyRandomSource(7)` running
    /// `createItemStackToDrop`'s arithmetic: three throws along the view,
    /// then two scattered drops, bit for bit, then the random's next int.
    #[test]
    fn drops_match_vanillas_motion_bit_for_bit() {
        let mut r = LegacyRandom::new(7);
        let bits = |v: [f64; 3]| v.map(|c| c.to_bits() as i64);
        let thrown = [
            (
                0.0,
                0.0,
                [
                    -4658604453255681268,
                    4595435376155295744,
                    4598847571894135436,
                ],
            ),
            (
                37.5,
                -21.25,
                [
                    -4627867408251048793,
                    4597245257657090048,
                    // Vanilla's `Math.sin` is HotSpot's intrinsic, which rounds this
                    // one a bit lower on AArch64 than on x86-64 (`jmath`).
                    if cfg!(target_arch = "aarch64") {
                        4597448471913462162
                    } else {
                        4597448471913462163
                    },
                ],
            ),
            (
                -123.4,
                60.0,
                [
                    4593476083220988603,
                    -4626282285467959296,
                    -4631965182103987462,
                ],
            ),
        ];
        for (yaw, pitch, want) in thrown {
            assert_eq!(
                bits(drop_motion(&mut r, yaw, pitch, false)),
                want,
                "{yaw} {pitch}"
            );
        }
        let scattered = [
            [
                -4634512480041697280,
                4596373779801702400,
                -4661085178024689664,
            ],
            [
                4600899508711194624,
                4596373779801702400,
                -4629159495333838848,
            ],
        ];
        for want in scattered {
            assert_eq!(bits(drop_motion(&mut r, 0.0, 0.0, true)), want);
        }
        assert_eq!(r.next_int(), 357425922);
    }

    /// A scattered drop takes two floats and rises at 0.2; one along the view
    /// (south, level) takes four and goes about 0.3 along +z.
    #[test]
    fn drops_scatter_or_follow_the_view_in_vanillas_draws() {
        let (mut r, mut after) = (LegacyRandom::new(7), LegacyRandom::new(7));
        let v = drop_motion(&mut r, 90.0, 30.0, true);
        let pow = f64::from(after.next_float() * 0.5);
        after.next_float();
        assert_eq!(v[1], f64::from(0.2f32));
        assert!((v[0].hypot(v[2]) - pow).abs() < 1.0e-3);
        let v = drop_motion(&mut r, 0.0, 0.0, false);
        (0..4).for_each(|_| _ = after.next_float());
        assert!(v[0].abs() <= 0.02 && (v[2] - 0.3).abs() <= 0.02, "{v:?}");
        assert!((v[1] - 0.1).abs() <= 0.1, "{v:?}");
        assert_eq!(r.next_int(), after.next_int());
    }
}

/// `LivingEntity.createItemStackToDrop`'s motion for a stack thrown by an
/// entity facing `yaw` and `pitch` (degrees), from that entity's random:
/// `randomly` scatters it in any direction (a death's drops), else it goes
/// along the view (the drop key).
pub fn drop_motion(
    r: &mut impl sunrise_core::random::RandomSource,
    yaw: f32,
    pitch: f32,
    randomly: bool,
) -> [f64; 3] {
    use sunrise_core::{
        geometry::trig::{DEG_TO_RAD, cos, sin},
        jmath,
    };
    let tau = std::f32::consts::TAU;
    if randomly {
        let pow = r.next_float() * 0.5;
        let dir = f64::from(r.next_float() * tau);
        return [
            f64::from(-sin(dir) * pow),
            f64::from(0.2f32),
            f64::from(cos(dir) * pow),
        ];
    }
    let (x, y) = (f64::from(pitch * DEG_TO_RAD), f64::from(yaw * DEG_TO_RAD));
    let (sin_x, cos_x, sin_y, cos_y) = (sin(x), cos(x), sin(y), cos(y));
    let dir = f64::from(r.next_float() * tau);
    let pow = f64::from(0.02 * r.next_float());
    [
        f64::from(-sin_y * cos_x * 0.3) + jmath::cos(dir) * pow,
        f64::from(-sin_x * 0.3 + 0.1 + (r.next_float() - r.next_float()) * 0.1),
        f64::from(cos_y * cos_x * 0.3) + jmath::sin(dir) * pow,
    ]
}

#[derive(Clone, Debug)]
pub struct ItemEntity {
    /// Entity-owned registration metadata; content clones are detached.
    pub attachment_lifetime: sunrise_mod_runtime::attachments::AttachmentLifetime,
    /// `Entity.yRot` and `xRot` (`/tp`, `/data get ... Rotation`).
    pub rotation: [f32; 2],
    /// When it joined its list ([`crate::join_seq`]); set by the list, not by the spawn site.
    pub join: crate::join_seq::JoinSeq,
    pub health: i32,
    pub fire_ticks: i32,
    pub id: u32,
    pub stack: ItemStack,
    pub thrower: Option<[i32; 4]>,
    /// `ItemEntity.target`: the only player who may pick the item up.
    pub target: Option<[i32; 4]>,
    pub position: Vec3,
    pub previous: Vec3,
    pub velocity: Vec3,
    pub ground: sunrise_world::ground_support::GroundState,
    /// `Entity.fallDistance`: grown by each move down outside water, handed to the block's
    /// `fallOn` on landing ([`ItemStep::landed`]), then reset.
    pub fall_distance: f64,
    pub age: i32,
    /// NeoForge `ItemEntity.lifespan`: maximum age in ticks before expiry
    /// (default [`Self::LIFETIME`], 6000).
    pub lifespan: i32,
    /// `ItemEntity.pickupDelay`: signed tick counter (`32767` = never pick up).
    pub pickup_delay: i32,
    pub bob_offset: f32,
    /// `Entity.tickCount`: ticks since the item entered the level, which
    /// paces `ItemEntity.mergeWithNeighbours`.
    pub ticks: u32,
    /// `ItemEntity.setItem` replaces the stack even when its contents are equal.
    /// Pickup captures this revision to distinguish that replacement from its original.
    pub stack_revision: u64,
    /// `Entity.needsSync` / `syncPosition`: immediate `ServerEntity.sendChanges`
    /// after a bounce or velocity jump (`> 0.1` in `ItemEntity.tick`).
    pub needs_sync: bool,
    /// `Entity.stuckSpeedMultiplier`: set after a move by the cobweb or powder snow
    /// the item is in, scales (and stops) the next one.
    pub stuck: Vec3,
    /// `Entity.wasTouchingWater`: the last fluid push. Entering water after
    /// the first tick is what splashes.
    pub was_touching_water: bool,
    /// `Entity.noGravity` (`NoGravity` in its save): `applyGravity` does nothing.
    pub no_gravity: bool,
    /// `Entity.verticalCollision`: the last move was stopped vertically.
    /// `ServerEntity.isFullPrecisionEncodingRequired` reads it for items, whose
    /// resting position is exact while a 1/4096 delta is not.
    pub vertical_collision: bool,
    /// `Entity.random`, after the constructor's draws: `moveTowardsClosestSpace`
    /// takes an item's escape speed from it.
    pub random: sunrise_core::random::LegacyRandom,
    /// What `entity_size` answered at its last size refresh (`EntityEvent.Size`): its box takes
    /// the answer while it holds for its vanilla size.
    pub size_answer: Option<crate::pose::SizeAnswer>,
}
/// What `Entity.doWaterSplashEffect` asks the level to do for a dropped item.
/// Particles are drawn on the client (`Level.addParticle` is empty on the
/// server). The sound is `Entity.playSound`, which the server broadcasts;
/// `ClientLevel.playSeededSound` ignores it when the except entity is null.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ItemSplash {
    Sound {
        at: [f64; 3],
        volume: f32,
        pitch: f32,
    },
    Particle {
        bubble: bool,
        at: [f64; 3],
        velocity: [f64; 3],
    },
}
/// `Entity.doWaterSplashEffect` of an entity with no rider, `width` wide, at
/// `position` moving at `velocity` (a dropped item, a firework rocket).
pub fn water_splash_effect(
    position: Vec3,
    velocity: Vec3,
    width: f32,
    random: &mut sunrise_core::random::LegacyRandom,
    splash: &mut impl FnMut(ItemSplash),
) {
    let mut water = ItemWater {
        position,
        velocity,
        width,
        random,
        splash,
    };
    let _ = sunrise_world::water_splash::water_splash(&mut water, 1_000_000);
}
struct ItemWater<'a, F> {
    position: Vec3,
    velocity: Vec3,
    width: f32,
    random: &'a mut sunrise_core::random::LegacyRandom,
    splash: &'a mut F,
}
impl<F: FnMut(ItemSplash)> sunrise_world::water_splash::WaterSplashWorld for ItemWater<'_, F> {
    type Error = std::convert::Infallible;
    type Entity = ();
    type Sound = ();
    fn this_entity(&self) {}
    fn controlling_passenger(&mut self) -> Result<Option<()>, Self::Error> {
        Ok(None)
    }
    fn velocity(&mut self, _: ()) -> Result<Vec3, Self::Error> {
        Ok(self.velocity)
    }
    fn splash_sound(&mut self, _: bool) -> Result<(), Self::Error> {
        Ok(())
    }
    fn random_float(&mut self) -> Result<f32, Self::Error> {
        Ok(self.random.next_float())
    }
    fn random_double(&mut self) -> Result<f64, Self::Error> {
        Ok(self.random.next_double())
    }
    fn play_sound(&mut self, _: (), volume: f32, pitch: f32) -> Result<(), Self::Error> {
        let at = [self.position.x, self.position.y, self.position.z];
        (self.splash)(ItemSplash::Sound { at, volume, pitch });
        Ok(())
    }
    fn position(&self) -> Vec3 {
        self.position
    }
    fn width(&self) -> f32 {
        self.width
    }
    fn particle(
        &mut self,
        kind: sunrise_world::water_splash::SplashParticle,
        position: Vec3,
        velocity: Vec3,
    ) -> Result<(), Self::Error> {
        (self.splash)(ItemSplash::Particle {
            bubble: matches!(kind, sunrise_world::water_splash::SplashParticle::Bubble),
            at: [position.x, position.y, position.z],
            velocity: [velocity.x, velocity.y, velocity.z],
        });
        Ok(())
    }
    fn splash_event(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}
/// Environment results needed by the entity owner, including lethal lava damage.
#[derive(Debug)]
pub struct ItemEnvironmentTick {
    pub alive: bool,
    pub lava_hurt: bool,
    /// `ItemStack.onDestroyed` of the damage that emptied the health: the
    /// stacks the destroyed item left, which `ItemUtils.onContainerDestroyed`
    /// adds to the level where it lay.
    pub spilled: Vec<ItemStack>,
    /// The damage type that reduced the item's health to zero, when `!alive`.
    pub destroyed_by: Option<&'static str>,
}
impl ItemEnvironmentTick {
    pub fn should_play_lava_hurt_sound(&self, tick: u32) -> bool {
        self.lava_hurt && (!self.alive || tick.is_multiple_of(10))
    }
}
impl crate::entity_rules::EntityRules for ItemEntity {
    fn is_no_gravity(&self) -> bool {
        self.no_gravity
    }
    /// `ItemEntity.getDefaultGravity`.
    fn get_default_gravity(&self) -> f64 {
        0.04
    }
}
impl crate::entity_rules::EntityRules for ExperienceOrb {
    fn is_no_gravity(&self) -> bool {
        self.no_gravity
    }
    /// `ExperienceOrb.getDefaultGravity`.
    fn get_default_gravity(&self) -> f64 {
        0.03
    }
}
impl crate::entity_rules::EntityRules for ArmorStand {
    fn is_no_gravity(&self) -> bool {
        self.no_gravity
    }
    /// `LivingEntity.getDefaultGravity`.
    fn get_default_gravity(&self) -> f64 {
        0.08
    }
    /// The code's reading of `Entity.isPushedByFluid` for a marker stand: it
    /// takes no fluid push and is never in water.
    fn is_pushed_by_fluid(&self) -> bool {
        !self.marker
    }
}
/// What an item's tick carries from [`ItemEntity::tick_move`] to
/// [`ItemEntity::tick_settle`], across the game's bubble-column push.
#[derive(Clone, Copy)]
pub struct ItemStep {
    old_velocity: Vec3,
    landed: Option<(BlockPos, f64)>,
    movement: Option<Vec3>,
    pub vertical_contact: Option<BlockPos>,
    restitution: Option<(Vec3, Vec3, crate::motion::BodyContact)>,
    /// `Entity.move` with physics: where it started, what it moved, and
    /// whether the move was applied. `None` while idle or without physics,
    /// where `checkFallDamage` does not run.
    pub fall_move: Option<(Vec3, Vec3, bool)>,
    prior_fall: f64,
}
impl ItemStep {
    /// The landing position and accumulated fall distance, before block contact.
    pub fn landed(&self) -> Option<(BlockPos, f64)> {
        self.landed
    }
}
#[cfg(test)]
impl ItemEntity {
    /// A whole `ItemEntity.tick` for the unit tests, in the order the game
    /// runs it: fluid push, move, bubble column, settle, fluid push.
    pub(crate) fn tick(
        &mut self,
        world: &World,
        registry: &sunrise_world::registry::BlockRegistry,
        facts: &BlockFacts,
        collision: &BlockCollisionRegistry<'_>,
        no_physics: bool,
        fluids: &crate::motion::FluidResources,
        entity_boxes: &[Aabb],
        border: Option<&BorderGeometry>,
        blocks: Option<&BoundMovementProperties>,
    ) -> bool {
        let context = crate::test_resources::server_loop_context();
        let before = self.velocity;
        let (water, lava) = self.update_fluids(world, fluids, context);
        self.water_entry(water > 0.0, self.ticks == 0, before, &mut |_| {});
        self.lava_fall(lava > 0.0);
        let step = self.tick_move(
            world,
            facts,
            collision,
            no_physics,
            (water, lava),
            entity_boxes,
            border,
            blocks,
            context,
        );
        crate::motion::bubble_column_push(
            world,
            registry,
            facts,
            collision,
            self.bounds(),
            &mut self.velocity,
        );
        let alive = self.tick_settle(
            world,
            registry,
            facts,
            step,
            context,
            crate::motion::dry_side(),
        );
        let before = self.velocity;
        let (water, _) = self.update_fluids(world, fluids, context);
        self.water_entry(water > 0.0, false, before, &mut |_| {});
        alive
    }
}
impl ItemEntity {
    pub const WIDTH: f64 = 0.25;
    /// `EntityType.ITEM.getDimensions()`: width, height and eye height.
    pub const DIMENSIONS: [f32; 3] = [0.25, 0.25, 0.2125];
    /// `EntityTypes.ITEM` declares a float eye height, widened by `Entity.getEyeY`.
    pub const EYE_HEIGHT: f64 = 0.2125_f32 as f64;
    /// Vanilla `ItemEntity.LIFETIME`: default dropped-item lifespan in ticks.
    pub const LIFETIME: i32 = 6000;
    /// NeoForge `ItemEntity.tick` lifespan cap (`Short.MAX_VALUE - 1`),
    /// preventing `age` (stored as a short) from wrapping into an accidental
    /// infinite lifespan.
    pub const MAX_LIFESPAN: i32 = i16::MAX as i32 - 1;
    /// `ItemEntity.INFINITE_PICKUP_DELAY`: pickup delay that never counts down (`setNeverPickUp`).
    pub const INFINITE_PICKUP_DELAY: i32 = i16::MAX as i32;
    /// `ItemEntity.INFINITE_LIFETIME`: age that never increments (`setUnlimitedLifetime`).
    pub const INFINITE_LIFETIME: i32 = i16::MIN as i32;
    /// `Entity.getLightProbePosition(partialTick)` for a dropped item: the eye
    /// position (`getEyePosition`) over the position drawn this frame
    /// (`getPosition(partialTick)`), not this tick's own.
    pub fn light_probe(&self, partial: f64) -> [f64; 3] {
        [
            self.previous.x + (self.position.x - self.previous.x) * partial,
            self.previous.y + (self.position.y - self.previous.y) * partial + Self::EYE_HEIGHT,
            self.previous.z + (self.position.z - self.previous.z) * partial,
        ]
    }
    /// `EntityRenderer.getPackedLightCoords` for a dropped item: the level's
    /// sky light over the block light of the cell it is lit from, which
    /// `EntityRenderer.getBlockLightLevel` puts at 15 while the item
    /// `isOnFire`. `light` is the level's packed light (sky high, block low);
    /// `environment_tick` clears the fire ticks of a fire-immune stack, as
    /// `ItemEntity.fireImmune` makes it never on fire.
    pub fn render_light(&self, light: u8) -> u32 {
        let block = if self.fire_ticks > 0 { 15 } else { light & 15 };
        (u32::from(light >> 4) * 16) | ((u32::from(block) * 16) << 8)
    }
    pub fn bounds(&self) -> Aabb {
        let h = Self::WIDTH / 2.0;
        let vanilla = Aabb::new(
            Vec3::new(self.position.x - h, self.position.y, self.position.z - h),
            Vec3::new(
                self.position.x + h,
                self.position.y + Self::WIDTH,
                self.position.z + h,
            ),
        );
        crate::pose::SizeAnswer::sized_box(
            self.size_answer,
            Self::DIMENSIONS,
            self.position,
            vanilla,
        )
    }
    /// `ItemEntity.hurtServer`'s health: `amount` off the health, and an item
    /// whose health reaches 0 is destroyed at once (`ItemStack.onDestroyed`,
    /// then `discard`), so what it held leaves it here. None while the item
    /// survives; the emptied stacks otherwise.
    pub fn hurt(&mut self, items: &Items, amount: i32) -> Option<Vec<ItemStack>> {
        self.health -= amount;
        if self.health > 0 {
            return None;
        }
        Some(self.spill_destroyed_contents(items))
    }
    /// Built-in `ItemStack.onDestroyed` spills (`BundleItem.onDestroyed` and
    /// `BlockItem.onDestroyed` for shulker boxes) computed without modifying
    /// `self.stack`, so `ItemPart::item_destroyed` sees the original intact
    /// stack (`ItemEntity.hurtServer` -> `ItemStack.onDestroyed` -> `discard()`).
    pub fn peek_destroyed_contents(&self, items: &Items) -> Vec<ItemStack> {
        let Some(data) = self.stack.data() else {
            return Vec::new();
        };
        if data.bundle_contents.is_empty() && data.container.is_empty() {
            return Vec::new();
        }
        let mut copy = self.stack.clone();
        let mut spilled = crate::bundle::destroyed(items, &mut copy);
        spilled.extend(crate::item_data::destroyed_container(&mut copy));
        spilled
    }
    /// Clears built-in container contents (`bundle_contents`, `container`) in place
    /// and returns them for dropping (`ItemStack.onDestroyed`).
    pub fn spill_destroyed_contents(&mut self, items: &Items) -> Vec<ItemStack> {
        // `ItemStack.onDestroyed` runs the item's own arm, and the component
        // the stack holds is the dispatch: a bundle's (`BundleItem.onDestroyed`)
        // `bundle_contents` and a container item's (`BlockItem.onDestroyed`)
        // `container`, which a mod's own item carries in the same way.
        let mut spilled = crate::bundle::destroyed(items, &mut self.stack);
        spilled.extend(crate::item_data::destroyed_container(&mut self.stack));
        spilled
    }
    /// The ItemEntity snapshot used by both its move and its noCollision check.
    pub fn collision_context(&self) -> sunrise_world::collision_context::CollisionContext<'static> {
        use sunrise_world::collision_context::{
            CollisionContext, CollisionEntity, EntityContextKind,
        };
        CollisionContext::for_entity(
            CollisionEntity {
                identity: sunrise_world::entity_obstruction::EntityIdentity(u64::from(self.id)),
                kind: EntityContextKind::Item,
                y: self.position.y,
                descending: false,
                main_hand: None,
            },
            false,
        )
    }
    /// `Entity.updateFluidInteraction`: water resets the fall, and entering
    /// it after the first tick runs `doWaterSplashEffect` at `velocity`, the
    /// motion before the fluids' currents push it.
    pub fn water_entry<F: FnMut(ItemSplash)>(
        &mut self,
        in_water: bool,
        first_tick: bool,
        velocity: Vec3,
        splash: &mut F,
    ) {
        if in_water {
            self.fall_distance = 0.0;
        }
        if in_water && !self.was_touching_water && !first_tick {
            let mut random = sunrise_core::random::LegacyRandom::new(
                (self.id as i64)
                    .wrapping_mul(0x5DEECE66D)
                    .wrapping_add(self.ticks as i64),
            );
            let width = Self::WIDTH as f32;
            water_splash_effect(self.position, velocity, width, &mut random, splash);
        }
        self.was_touching_water = in_water;
    }
    /// `Entity.moveTowardsClosestSpace` from the middle of the item's box:
    /// towards the nearest side of its block (north, south, west, east or
    /// up) without a full collision cube, at `speed` (`nextFloat() * 0.2 +
    /// 0.1`).
    pub fn move_towards_closest_space(&mut self, world: &World, facts: &BlockFacts, speed: f32) {
        self.velocity = towards_closest_space(
            self.position,
            self.bounds(),
            self.velocity,
            world,
            facts,
            speed,
        );
    }
    /// `ItemEntity.setUnderwaterMovement` / `setUnderLavaMovement`: horizontal
    /// drag is a float (`0.99F` / `0.95F`), and a slow item is lifted by
    /// `5.0E-4F` while slower than `0.06F`.
    fn set_fluid_movement(&mut self, drag: f32) {
        let drag = f64::from(drag);
        let lift = if self.velocity.y < f64::from(0.06_f32) {
            f64::from(5.0e-4_f32)
        } else {
            0.0
        };
        self.velocity = Vec3::new(
            self.velocity.x * drag,
            self.velocity.y + lift,
            self.velocity.z * drag,
        );
    }
    /// The rest of `ItemEntity.tick`, after block effects: movement drag, sync and age. Returns false when the item should be
    /// removed. The game then runs the fluid push again (`ItemEntity.tick`
    /// refreshes immersion after moving).
    pub fn tick_settle(
        &mut self,
        world: &World,
        registry: &sunrise_world::registry::BlockRegistry,
        facts: &BlockFacts,
        step: ItemStep,
        context: &sunrise_world::native_load_context::LoopContext,
        side: &dyn crate::motion::SideLevel,
    ) -> bool {
        if step.movement.is_some() {
            // `float friction = getAirDrag()` (0.98F); on the ground, times
            // the block's friction as a float, then widened for the scale.
            let mut friction = 0.98_f32;
            if self.ground.on_ground {
                let below =
                    BlockPos::containing(self.position.subtract(Vec3::new(0.0, 0.999_999, 0.0)));
                friction *= block_friction(
                    context,
                    side,
                    (self.id, "minecraft:item"),
                    world,
                    registry,
                    facts,
                    below,
                );
            }
            let friction = f64::from(friction);
            let air = f64::from(0.98_f32);
            self.velocity = Vec3::new(
                self.velocity.x * friction,
                self.velocity.y * air,
                self.velocity.z * friction,
            );
            if self.ground.on_ground && self.velocity.y < 0.0 {
                self.velocity.y *= -0.5;
            }
        }
        if self.velocity.subtract(step.old_velocity).length_squared() > 0.01 {
            self.needs_sync = true;
        }
        if self.age != i16::MIN as i32 {
            self.age = self.age.wrapping_add(1);
        }
        self.age < self.lifespan && self.position.y >= f64::from(world.shape.min_y - 64)
    }
    /// `Entity.makeStuckInBlock` as `WebBlock.entityInside` and
    /// `PowderSnowBlock.entityInside` call it for an item (which, unlike a
    /// living entity, is stuck whatever block its feet are in): the speed
    /// factor of the last such block the item's box (deflated by `1.0E-5`) is
    /// in.
    fn stuck_multiplier(&self, world: &World, facts: &BlockFacts) -> Option<Vec3> {
        let b = self.bounds().deflate(Vec3::new(1e-5, 1e-5, 1e-5));
        let mut stuck = None;
        crate::block_contact::box_cells(b, |p| {
            match &*facts.get(world.block(p.x, p.y, p.z)).name {
                "minecraft:cobweb" => {
                    stuck = Some(Vec3::new(0.25, f64::from(0.05_f32), 0.25));
                }
                "minecraft:powder_snow" => {
                    stuck = Some(Vec3::new(f64::from(0.9_f32), 1.5, f64::from(0.9_f32)));
                }
                _ => {}
            }
            true
        });
        stuck
    }
    /// `PistonMovingBlockEntity.causeBounce` for a dropped item. Vanilla then
    /// runs `Entity.move(MoverType.PISTON)`, which leaves the ground on an
    /// upward push; without that, `ItemEntity.tick` skips movement 3 of 4 ticks
    /// while `onGround` with no horizontal speed and gravity eats the bounce.
    pub fn piston_slime_bounce(&mut self, movement: Direction) {
        self.velocity = sunrise_world::piston_push::bounce_velocity(movement, self.velocity);
        self.ground.on_ground = false;
        self.ground.clear_support();
        self.needs_sync = true;
    }
    /// `HoneyBlock.entityInside` for the honey blocks this item's box is in
    /// (`Entity.checkInsideBlocks`, deflated by `1.0E-5`). Slide sounds and
    /// entity event 53 are skipped (`doesEntityDoHoneyBlockSlideEffects` is
    /// false for an item).
    fn apply_honey_slide(&mut self, world: &World, facts: &BlockFacts) {
        let b = self.bounds().deflate(Vec3::new(1e-5, 1e-5, 1e-5));
        crate::block_contact::box_cells(b, |p| {
            if &*facts.get(world.block(p.x, p.y, p.z)).name == "minecraft:honey_block" {
                self.honey_block_inside([p.x, p.y, p.z]);
            }
            true
        });
    }
    /// `HoneyBlock.isSlidingDown` then `doSlideMovement`. `getOldDeltaY` /
    /// `getNewDeltaY` wrap the stored velocity the same way they do for every
    /// entity (`vy / 0.98F + 0.08`).
    pub(crate) fn honey_block_inside(&mut self, pos: [i32; 3]) {
        let old_delta_y = |dy: f64| dy / f64::from(0.98_f32) + 0.08;
        let new_delta_y = |dy: f64| (dy - 0.08) * f64::from(0.98_f32);
        let [x, y, z] = pos.map(f64::from);
        if self.ground.on_ground
            || self.position.y > y + 0.9375 - 1.0e-7
            || old_delta_y(self.velocity.y) >= -0.08
        {
            return;
        }
        let dx = (x + 0.5 - self.position.x).abs();
        let dz = (z + 0.5 - self.position.z).abs();
        let overlap = 0.4375 + Self::WIDTH / 2.0;
        if dx + 1.0e-7 <= overlap && dz + 1.0e-7 <= overlap {
            return;
        }
        let v = self.velocity;
        let old = old_delta_y(v.y);
        self.velocity = if old < -0.13 {
            let factor = -0.05 / old;
            Vec3::new(v.x * factor, new_delta_y(-0.05), v.z * factor)
        } else {
            Vec3::new(v.x, new_delta_y(-0.05), v.z)
        };
    }
    fn has_infinite_pickup_delay(&self) -> bool {
        self.pickup_delay == Self::INFINITE_PICKUP_DELAY
    }
    /// `ItemEntity.isMergable` at `age`: a non-empty stack below its size whose
    /// pickup delay isn't infinite, before its lifetime ends.
    fn mergeable(&self, age: i32, max_stack: u32) -> bool {
        self.stack.count > 0
            && &*self.stack.item != "minecraft:air"
            && !self.has_infinite_pickup_delay()
            && age != Self::INFINITE_LIFETIME
            && age < Self::LIFETIME
            && self.stack.count < max_stack
    }
}

/// `LavaCauldronBlock.getEntityInsideCollisionShape`: the cauldron's solid
/// shape plus its contents (12 pixels wide, from 4 through 15 pixels high).
/// In particular, the rim and feet count too, but air above the contents does not.
fn lava_cauldron_contacts(
    world: &World,
    registry: &sunrise_world::registry::BlockRegistry,
    bounds: Aabb,
) -> usize {
    let epsilon = f64::from(1e-5_f32);
    let mut contacts = 0;
    let candidates = bounds.deflate(Vec3::new(epsilon, epsilon, epsilon));
    crate::block_contact::box_cells(candidates, |p| {
        let (x, y, z) = (p.x, p.y, p.z);
        if !registry
            .state(world.block(x, y, z))
            .is_some_and(|s| s.name == "minecraft:lava_cauldron")
        {
            return true;
        }
        let shape = crate::block_contact::lava_cauldron_filled_shape();
        let local = bounds.move_by(Vec3::new(-f64::from(x), -f64::from(y), -f64::from(z)));
        // Only block enumeration uses the deflated box. The actual
        // shape contact follows Entity.collidedWithShapeMovingFrom.
        if local.collided_along_vector(Vec3::ZERO, shape.boxes()) {
            contacts += 1;
        }
        true
    });
    contacts
}
/// `ExperienceOrbRenderer.submit`'s billboard before its 0.3 scale, in the
/// order of its four `vertex` calls (bottom left, bottom right, top right, top
/// left). The quad is a whole block tall about the orb's centre and hangs a
/// quarter of it below.
pub const ORB_QUAD: [[f32; 2]; 4] = [[-0.5, -0.25], [0.5, -0.25], [0.5, 0.75], [-0.5, 0.75]];
/// `poseStack.scale(0.3F, 0.3F, 0.3F)`.
pub const ORB_QUAD_SCALE: f32 = 0.3;
/// ExperienceOrb: gravity 0.03, homing towards the player within 8 blocks.
#[derive(Clone, Debug)]
pub struct ExperienceOrb {
    /// Entity-owned registration metadata; content clones are detached.
    pub attachment_lifetime: sunrise_mod_runtime::attachments::AttachmentLifetime,
    /// `Entity.yRot` and `xRot` (`/tp`, `/data get ... Rotation`).
    pub rotation: [f32; 2],
    /// When it joined its list ([`crate::join_seq`]); set by the list, not by the spawn site.
    pub join: crate::join_seq::JoinSeq,
    /// The entity id servers track it by.
    pub id: u32,
    pub position: Vec3,
    pub previous: Vec3,
    pub velocity: Vec3,
    /// `ExperienceOrb.DATA_VALUE`: an int that a save or command can leave at
    /// 0 or below, which a pickup turns into no experience.
    pub value: i32,
    /// `ExperienceOrb.age`: ticks lived, signed (a save can start it below 0);
    /// the orb is discarded at 6000.
    pub age: i32,
    pub ground: sunrise_world::ground_support::GroundState,
    /// `ExperienceOrb.count`: orbs merged into this one; each pickup gives
    /// `value` once and takes one off.
    pub count: u32,
    /// `Entity.tickCount`, which paces `ExperienceOrb.scanForMerges`.
    pub ticks: u32,
    /// `ExperienceOrb.health`: damage wears it down from 5 (`hurtServer`).
    pub health: i32,
    /// `Entity.airSupply`: loaded and saved as is; an orb never breathes.
    pub air: i32,
    /// `Entity.noGravity`: `ExperienceOrb.tick`'s `applyGravity` does nothing.
    pub no_gravity: bool,
    /// `ExperienceOrb.followingPlayer`: the followed player's entity id, kept
    /// until they are gone or further than 8 blocks. Never saved.
    pub following: Option<u32>,
    /// `Entity.remainingFireTicks`: saved as `Fire`, burnt down by
    /// `Entity.baseTick`, which hurts the orb once every 20 ticks.
    pub fire_ticks: i32,
    /// `Entity.fallDistance` (`fall_distance` in its save).
    pub fall_distance: f64,
    /// What `entity_size` answered at its last size refresh (`EntityEvent.Size`): its box takes
    /// the answer while it holds for its vanilla size.
    pub size_answer: Option<crate::pose::SizeAnswer>,
}
/// An orb's `Entity.move` this tick, for the fall bookkeeping its caller
/// finishes (the long-move cast, `checkFallDamage`'s HIT_GROUND).
#[derive(Clone, Copy, Debug)]
pub struct OrbMove {
    pub from: Vec3,
    pub moved: Vec3,
    /// `Entity.isInWater` from this tick's fluid step.
    pub in_water: bool,
}
/// Resume an orb's tick after its authoritative landing and restitution phases.
#[derive(Clone, Copy)]
pub struct OrbTickMove {
    pub step: OrbMove,
    pub movement: crate::motion::BodyMove,
    pub burnt_out: bool,
}

/// Server-side effects of an orb's contact with lava after a move.
pub struct OrbLavaEffects<'a, W> {
    pub next_float: &'a mut dyn FnMut() -> f32,
    pub on_sound: &'a mut dyn FnMut([f64; 3], f32),
    pub invulnerable: &'a mut dyn FnMut(&mut W) -> bool,
}

impl ExperienceOrb {
    /// A fresh orb (`new ExperienceOrb(level, pos, motion, value)`): full
    /// health, no age, fall or followed player, not yet ticked.
    pub fn new(id: u32, position: Vec3, velocity: Vec3, value: i32) -> Self {
        Self {
            size_answer: None,
            attachment_lifetime: Default::default(),
            rotation: [0.0, 0.0],
            join: Default::default(),
            id,
            position,
            previous: position,
            velocity,
            value,
            age: 0,
            ground: Default::default(),
            count: 1,
            ticks: 0,
            health: 5,
            air: 300,
            no_gravity: false,
            following: None,
            fall_distance: 0.0,
            fire_ticks: 0,
        }
    }
    /// `ExperienceOrb.hurtServer`: the damage comes off the health as an
    /// `int`; an orb at 0 or below is discarded. True when it is gone.
    pub fn hurt(&mut self, damage: f32) -> bool {
        self.health = (self.health as f32 - damage) as i32;
        self.health <= 0
    }
    /// The box of an orb whose `position` (the bottom of the box) is `at`.
    pub fn box_at(at: Vec3) -> Aabb {
        let h = 0.25;
        Aabb::new(
            Vec3::new(at.x - h, at.y, at.z - h),
            Vec3::new(at.x + h, at.y + 0.5, at.z + h),
        )
    }
    /// `EntityType.EXPERIENCE_ORB.getDimensions()`: width, height and eye height.
    pub const DIMENSIONS: [f32; 3] = [0.5, 0.5, 0.425];
    pub fn bounds(&self) -> Aabb {
        crate::pose::SizeAnswer::sized_box(
            self.size_answer,
            Self::DIMENSIONS,
            self.position,
            Self::box_at(self.position),
        )
    }
    /// `Entity.unstuckIfPossible`, which `ExperienceOrb`'s constructor runs as
    /// `if (!level.noCollision(box)) unstuckIfPossible(box.getSize())`: the free
    /// spot nearest the box's centre, searched in a cube of the box's size
    /// around it. `None` when the box is already free (the orb then keeps its
    /// exact position) or when nothing free is that close. `shape(p)` is the
    /// block's collision shape at `p`, in world coordinates.
    pub fn unstuck(
        at: Vec3,
        border: &BorderBounds,
        mut shape: impl FnMut(BlockPos) -> Option<VoxelShape>,
    ) -> Option<Vec3> {
        let bounds = Self::box_at(at);
        let size = bounds.average_size();
        let lo = bounds.min_position();
        let cells = |v: f64| (v - 1.0e-7).floor() as i32 - 1..=(v + 1.0e-7).floor() as i32 + 1;
        // noCollision(box): a block shape reaching into it leaves the orb stuck.
        let mut stuck = false;
        'cells: for x in cells(lo.x) {
            for y in cells(lo.y) {
                for z in cells(lo.z) {
                    if shape(BlockPos::new(x, y, z))
                        .is_some_and(|s| s.boxes().iter().any(|b| b.intersects(bounds)))
                    {
                        stuck = true;
                        break 'cells;
                    }
                }
            }
        }
        if !stuck {
            return None;
        }
        // `AABB.ofSize(center, size, size, size)`: where the box's centre may
        // go, a cube of the box's mean size around its centre.
        let allowed = Aabb::of_size(bounds.center(), Vec3::new(size, size, size));
        find_free_position(allowed, bounds.center(), bounds.size(), border, shape)
            .map(|p| p.subtract(Vec3::new(0.0, bounds.size().y / 2.0, 0.0)))
    }
    /// `ExperienceOrb.canMerge(orb, id, value)` for an orb that is not removed:
    /// the same value and ids a multiple of 40 apart.
    pub fn can_merge(&self, id: u32, value: i32) -> bool {
        self.value == value && (self.id as i32).wrapping_sub(id as i32) % 40 == 0
    }
    /// The experience one pickup gives: `playerTouch` gives only a positive
    /// remainder, so an orb of value 0 or below gives none. Mods see this too.
    pub fn points(&self) -> u32 {
        self.value.max(0) as u32
    }
    /// ExperienceOrb.getExperienceValue: the largest orb size not above `value`.
    pub fn split(value: u32) -> u32 {
        [2477, 1237, 617, 307, 149, 73, 37, 17, 7, 3, 1]
            .into_iter()
            .find(|&v| value >= v)
            .unwrap_or(1)
    }
    /// ExperienceOrb.getIcon
    pub fn icon(&self) -> u32 {
        match self.value {
            2477.. => 10,
            1237.. => 9,
            617.. => 8,
            307.. => 7,
            149.. => 6,
            73.. => 5,
            37.. => 4,
            17.. => 3,
            7.. => 2,
            3.. => 1,
            _ => 0,
        }
    }
    /// `Entity.getLightProbePosition`: the block an entity is lit from is the
    /// one its eye is in. An orb's box is half a block, so its eye is
    /// `0.5 * 0.85` above its feet (`EntityDimensions.defaultEyeHeight`).
    pub fn light_probe_y(&self) -> f64 {
        self.position.y + 0.5 * 0.85
    }
    /// `ExperienceOrbRenderer.getBlockLightLevel` over `EntityRenderer`'s
    /// packing: the level's block light raised by 7 and clamped, the sky
    /// light unchanged. `light` is the level's packed light.
    pub fn render_light(&self, light: i32) -> u32 {
        let block = ((light & 15) + 7).clamp(0, 15);
        let sky = light >> 4;
        (u32::try_from(sky).unwrap_or(0) * 16) | ((u32::try_from(block).unwrap_or(15) * 16) << 8)
    }
    /// `ExperienceOrb.setUnderwaterMovement`: buoyancy and drag before moving.
    fn set_underwater_movement(&mut self) {
        // Vanilla widens float constants before applying double arithmetic.
        self.velocity = Vec3::new(
            self.velocity.x * f64::from(0.99_f32),
            (self.velocity.y + f64::from(5.0e-4_f32)).min(f64::from(0.06_f32)),
            self.velocity.z * f64::from(0.99_f32),
        );
    }
    pub fn tick(
        &mut self,
        world: &World,
        facts: &BlockFacts,
        collision: &BlockCollisionRegistry<'_>,
        player: Option<[f64; 3]>,
        server: bool,
        next_float: &mut dyn FnMut() -> f32,
        border: Option<&BorderGeometry>,
        fluids: &crate::motion::FluidResources,
        context: &sunrise_world::native_load_context::LoopContext,
        blocks: Option<&BoundMovementProperties>,
        side: &dyn crate::motion::SideLevel,
        on_sound: &mut dyn FnMut([f64; 3], f32),
    ) -> bool {
        let (stays, _) = self.tick_among(
            world,
            facts,
            collision,
            player,
            server,
            next_float,
            border,
            fluids,
            context,
            blocks,
            side,
            on_sound,
            &[],
        );
        stays
    }
    /// [`Self::tick`], its move also stopped by `entities`: the boxes of the
    /// entities that `canBeCollidedWith` it (`EntityGetter.getEntityCollisions`).
    /// Also gives the move it made, whose fall bookkeeping the caller finishes.
    #[allow(clippy::too_many_arguments)]
    pub fn tick_among(
        &mut self,
        world: &World,
        facts: &BlockFacts,
        collision: &BlockCollisionRegistry<'_>,
        player: Option<[f64; 3]>,
        server: bool,
        next_float: &mut dyn FnMut() -> f32,
        border: Option<&BorderGeometry>,
        fluids: &crate::motion::FluidResources,
        context: &sunrise_world::native_load_context::LoopContext,
        blocks: Option<&BoundMovementProperties>,
        side: &dyn crate::motion::SideLevel,
        on_sound: &mut dyn FnMut([f64; 3], f32),
        entities: &[Aabb],
    ) -> (bool, OrbMove) {
        let motion = crate::motion::BodyMotion {
            world,
            collision,
            facts,
            blocks,
            context,
            side: Some(side),
        };
        let tick =
            self.move_before_fall(motion, player, server, next_float, border, fluids, entities);
        self.velocity = self.move_restitution(motion, tick).velocity;
        motion.apply_block_speed(self.position, &mut self.velocity);
        if !tick.burnt_out {
            motion.step_on(
                (self.id, "minecraft:experience_orb"),
                (self.position, self.bounds()),
                &self.ground,
                &mut self.velocity,
            );
        }
        let stays = self.finish_tick(
            &mut &*world,
            (collision, facts, context, side),
            fluids,
            tick,
            server,
            OrbLavaEffects {
                next_float,
                on_sound,
                invulnerable: &mut |_| false,
            },
        );
        (stays, tick.step)
    }

    /// Move and read the legacy state before the level handles the impact.
    pub fn move_before_fall(
        &mut self,
        motion: crate::motion::BodyMotion<'_>,
        player: Option<[f64; 3]>,
        server: bool,
        next_float: &mut dyn FnMut() -> f32,
        border: Option<&BorderGeometry>,
        fluids: &crate::motion::FluidResources,
        entities: &[Aabb],
    ) -> OrbTickMove {
        let crate::motion::BodyMotion {
            world,
            collision,
            facts,
            context,
            ..
        } = motion;
        self.ticks += 1;
        self.previous = self.position;
        // `Entity.baseTick`'s fluid step, whose `updateFluidOnEyes` answer the
        // water branch below reads: the orb's eye, not its feet, decides.
        let (eye_in_water, in_water, in_lava) = self.update_fluids(world, fluids, context);
        // `Entity.baseTick`'s fire step, right after the fluid step: an orb is
        // not fire immune, so each 20th tick of a burn hurts it for 1 (out of
        // lava; `isInLava` is false on the first tick). The client only
        // `clearFire`s (`min(0, ticks)`). A burn that empties its health discards it.
        let mut burnt_out = false;
        if !server {
            self.fire_ticks = self.fire_ticks.min(0);
        } else {
            if self.fire_ticks > 0 {
                if self.fire_ticks % 20 == 0 && !(in_lava && self.ticks > 1) {
                    burnt_out = self.hurt(1.0);
                }
                self.fire_ticks -= 1;
            }
            // `Entity.move` ends in `applyEffectsFromBlocks`, which puts an
            // orb that is not on fire at `-getFireImmuneTicks()`, 0.
            self.fire_ticks = self.fire_ticks.max(0);
        }
        let block = world.block(
            self.position.x.floor() as i32,
            self.position.y.floor() as i32,
            self.position.z.floor() as i32,
        );
        let bounds = self.bounds();
        let mut scene = SceneCollisions {
            world,
            binding: collision,
            subject: None,
        };
        // `boolean colliding = !level.noCollision(getBoundingBox())`: an orb
        // inside blocks takes no gravity until it is pushed out below.
        let colliding = !crate::collisions::no_collision_empty(&mut scene, bounds);
        // ExperienceOrb.tick: `isEyeInFluid(FluidTags.WATER)` floats the orb,
        // so an orb whose eye dips into the surface floats with its feet
        // still in the air block above it.
        if eye_in_water {
            self.set_underwater_movement();
        } else if !colliding {
            self.velocity.y -= self.get_gravity();
        }
        if facts.get(block).fluid == Fluid::Lava {
            // ExperienceOrb.tick: lava throws the orb back out.
            self.velocity = Vec3::new(
                f64::from((next_float() - next_float()) * 0.2),
                f64::from(0.2f32),
                f64::from((next_float() - next_float()) * 0.2),
            );
        }
        if let Some(p) = player {
            let d = [
                (p[0] - self.position.x) / 8.0,
                (p[1] - self.position.y) / 8.0,
                (p[2] - self.position.z) / 8.0,
            ];
            let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            let power = 1.0 - length;
            if power > 0.0 && length > 1e-6 {
                let power = power * power;
                self.velocity = Vec3::new(
                    self.velocity.x + d[0] / length * power * 0.1,
                    self.velocity.y + d[1] / length * power * 0.1,
                    self.velocity.z + d[2] / length * power * 0.1,
                );
            }
        }
        if player.is_none() && server && colliding {
            // `followingPlayer == null && !level().isClientSide() && colliding`: an orb that would still
            // be stuck at its next position gets
            // `Entity.moveTowardsClosestSpace`, a nudge towards the nearest
            // free side of its block.
            if !crate::collisions::no_collision_empty(&mut scene, bounds.move_by(self.velocity)) {
                let speed = next_float() * 0.2 + 0.1;
                self.move_towards_closest_space(world, facts, speed);
            }
        }
        // `double fallSpeed = getDeltaMovement().y`, taken after the stuck
        // push and before the move.
        let wanted = self.velocity;
        // The orb's Entity.move goes through the move gate (#2414); PR 1a
        // records the movement unchanged.
        let mut gate = crate::move_gate::armed(
            context,
            self.id,
            "minecraft:experience_orb",
            sunrise_world::entity_move::MoverType::SelfMovement,
            wanted,
        );
        let wanted = gate.delta();
        // Entity.collide: the world border's walls and the entities that
        // `canBeCollidedWith` it (a boat, a shulker) stop the orb too.
        let border = border.map(|b| (b, [self.position.x, self.position.z]));
        let moved = crate::move_gate::scene_resolve_in_border_with(
            &mut gate,
            scene,
            bounds,
            wanted,
            0.0,
            self.ground.on_ground,
            border,
            entities,
        )
        .unwrap_or(Vec3::ZERO);
        let from = self.position;
        self.position = Vec3::new(
            self.position.x + moved.x,
            self.position.y + moved.y,
            self.position.z + moved.z,
        );
        if !gate.skipped() {
            motion.record_move(
                crate::motion::BodyCollision {
                    footprint: (self.position, self.bounds()),
                    movement: (wanted, moved),
                    authoritative: server,
                    context: crate::motion::BodyMotion::entity_context(self.id, self.position),
                    subject: None,
                },
                &mut self.ground,
            );
        }
        let effect = motion.legacy_contact(self.position, &self.ground);
        OrbTickMove {
            step: OrbMove {
                from,
                moved,
                in_water,
            },
            movement: crate::motion::BodyMove {
                wanted,
                moved,
                effect,
            },
            burnt_out,
        }
    }

    pub fn move_restitution(
        &self,
        motion: crate::motion::BodyMotion<'_>,
        tick: OrbTickMove,
    ) -> crate::motion::BodyRestitution {
        motion.restitute(
            (self.id, "minecraft:experience_orb"),
            tick.movement,
            self.get_gravity(),
            0.98_f32,
            self.velocity,
            self.fall_distance,
        )
    }

    pub fn finish_tick<W: std::ops::Deref<Target = World>>(
        &mut self,
        world: &mut W,
        (collision, facts, context, side): (
            &BlockCollisionRegistry<'_>,
            &BlockFacts,
            &sunrise_world::native_load_context::LoopContext,
            &dyn crate::motion::SideLevel,
        ),
        fluids: &crate::motion::FluidResources,
        tick: OrbTickMove,
        server: bool,
        lava: OrbLavaEffects<'_, W>,
    ) -> bool {
        let OrbTickMove {
            movement: crate::motion::BodyMove { wanted, .. },
            burnt_out,
            ..
        } = tick;
        // ExperienceOrb.tick applies block effects after moving, before drag.
        let discarded = server
            && !burnt_out
            && self.apply_lava_contact_damage_inner(
                world,
                fluids,
                wanted,
                lava.next_float,
                lava.on_sound,
                lava.invulnerable,
            );
        // `float friction = getAirDrag()` (0.98F), the ground's block
        // friction multiplied in as a float, widened once for the scale.
        let mut friction = 0.98_f32;
        if self.ground.on_ground {
            let below =
                BlockPos::containing(self.position.subtract(Vec3::new(0.0, 0.999_999, 0.0)));
            let kind = "minecraft:experience_orb";
            friction *= block_friction(
                context,
                side,
                (self.id, kind),
                world,
                collision.registry(),
                facts,
                below,
            );
        }
        let friction = f64::from(friction);
        // The drag scales every axis by the same friction, the ground's
        // block friction included.
        self.velocity = Vec3::new(
            self.velocity.x * friction,
            self.velocity.y * friction,
            self.velocity.z * friction,
        );
        // `if (verticalCollisionBelow && fallSpeed < -getGravity())`: the
        // landing bounce is 0.4 of the pre-move speed, not a fixed
        // restitution.
        if self.ground.on_ground && wanted.y < -0.03 {
            self.velocity.y = -wanted.y * 0.4;
        }
        // `++age` on a Java int.
        self.age = self.age.wrapping_add(1);
        !discarded && !burnt_out && self.age < 6000
    }
    /// `LavaFluid.entityInside` on the server: `Entity.lavaIgnite` (300 ticks),
    /// then `Entity.lavaHurt`, four damage through the orb's existing hurt/discard
    /// policy, playing the generic burn sound (`minecraft:entity.generic.burn`,
    /// volume 0.4, pitch 2.0 + random * 0.4) on each hurt contact. This is
    /// separate from the lava impulse at the start of the tick.
    pub fn apply_lava_contact_damage(
        &mut self,
        world: &World,
        fluids: &crate::motion::FluidResources,
        wanted: Vec3,
        next_float: &mut dyn FnMut() -> f32,
        on_sound: &mut dyn FnMut([f64; 3], f32),
    ) -> bool {
        let mut world = world;
        self.apply_lava_contact_damage_inner(
            &mut world,
            fluids,
            wanted,
            next_float,
            on_sound,
            &mut |_| false,
        )
    }

    fn apply_lava_contact_damage_inner<W: std::ops::Deref<Target = World>>(
        &mut self,
        world: &mut W,
        fluids: &crate::motion::FluidResources,
        wanted: Vec3,
        next_float: &mut dyn FnMut() -> f32,
        on_sound: &mut dyn FnMut([f64; 3], f32),
        invulnerable: &mut dyn FnMut(&mut W) -> bool,
    ) -> bool {
        let contacts = fluid_contacts(
            self.previous,
            self.position,
            Some(wanted),
            ExperienceOrb::box_at,
            |p| fluid_height(world, fluids, p, sunrise_world::fluid::FluidKind::Lava),
        );
        if contacts > 0 {
            // LAVA_IGNITE applies before its step's lavaHurt
            // (`InsideBlockEffectApplier.StepBasedCollector.applyAndClear`);
            // `igniteForTicks` keeps a longer fire.
            self.fire_ticks = self.fire_ticks.max(300);
        }
        for _ in 0..contacts {
            let dead = !invulnerable(world) && self.hurt(4.0);
            let pitch = 2.0 + next_float() * 0.4;
            on_sound([self.position.x, self.position.y, self.position.z], pitch);
            if dead {
                return true;
            }
        }
        false
    }
    /// `Entity.moveTowardsClosestSpace` from the middle of the orb's box:
    /// towards the nearest side of its block (north, south, west, east or
    /// up) without a full collision cube, at `speed` (the orb's own
    /// random, `nextFloat() * 0.2 + 0.1`).
    fn move_towards_closest_space(&mut self, world: &World, facts: &BlockFacts, speed: f32) {
        self.velocity = towards_closest_space(
            self.position,
            self.bounds(),
            self.velocity,
            world,
            facts,
            speed,
        );
    }
}

/// `Entity.moveTowardsClosestSpace` from the middle of an entity's box
/// (`bounds`, its feet at `position`): the velocity pushed towards the
/// nearest side of its block (north, south, west, east or up) without a full
/// collision cube, at `speed` (`nextFloat() * 0.2 + 0.1`). Items and
/// experience orbs share it.
fn towards_closest_space(
    position: Vec3,
    bounds: Aabb,
    velocity: Vec3,
    world: &World,
    facts: &BlockFacts,
    speed: f32,
) -> Vec3 {
    let at = Vec3::new(
        position.x,
        (bounds.min_position().y + bounds.max_position().y) / 2.0,
        position.z,
    );
    let pos = [
        at.x.floor() as i32,
        at.y.floor() as i32,
        at.z.floor() as i32,
    ];
    let delta = [
        at.x - f64::from(pos[0]),
        at.y - f64::from(pos[1]),
        at.z - f64::from(pos[2]),
    ];
    // (axis, step): NORTH, SOUTH, WEST, EAST, UP.
    let mut closest = (1, 1);
    let mut nearest = f64::MAX;
    for (axis, step) in [(2, -1), (2, 1), (0, -1), (0, 1), (1, 1)] {
        let mut n = pos;
        n[axis] += step;
        if !facts.get(world.block(n[0], n[1], n[2])).full_collision {
            let d = if step > 0 {
                1.0 - delta[axis]
            } else {
                delta[axis]
            };
            if d < nearest {
                nearest = d;
                closest = (axis, step);
            }
        }
    }
    let push = f64::from(closest.1 as f32 * speed);
    let v = velocity.scale(0.75);
    match closest.0 {
        0 => Vec3::new(push, v.y, v.z),
        1 => Vec3::new(v.x, push, v.z),
        _ => Vec3::new(v.x, v.y, push),
    }
}
/// Player experience (Player.giveExperiencePoints / getXpNeededForNextLevel).
#[derive(Clone, Copy, Debug, Default)]
pub struct Experience {
    pub level: i32,
    pub progress: f32,
    pub total: i32,
}
impl Experience {
    /// `Player.getBaseExperienceReward`: keep-inventory and spectators drop none.
    pub fn death_reward(&self, keep: bool) -> u32 {
        if keep {
            0
        } else {
            self.level.wrapping_mul(7).clamp(0, 100) as u32
        }
    }
    /// `ServerPlayer.restoreFrom`: only keep-inventory and spectators retain XP.
    pub fn respawn(&mut self, keep: bool) {
        if !keep {
            *self = Self::default();
        }
    }
    /// `Player.getXpNeededForNextLevel`, wrapping as Java's `int` does for
    /// a level a command or a mod set out of range.
    pub fn needed(level: i32) -> i32 {
        if level >= 30 {
            (level - 30).wrapping_mul(9).wrapping_add(112)
        } else if level >= 15 {
            (level - 15).wrapping_mul(5).wrapping_add(37)
        } else {
            level.wrapping_mul(2).wrapping_add(7)
        }
    }
    /// The first half of `Player.giveExperiencePoints`: the points go to the
    /// bar and the total, and the levels they cross follow one at a time.
    pub fn add_points(&mut self, points: i32) {
        self.progress += points as f32 / Self::needed(self.level) as f32;
        self.total = self.total.wrapping_add(points).max(0);
    }
    /// The change `Player.giveExperienceLevels` makes: below level 0 the level,
    /// the progress and the total are 0.
    pub fn add_levels(&mut self, levels: i32) {
        self.level = self.level.saturating_add(levels);
        if self.level < 0 {
            self.level = 0;
            self.progress = 0.0;
            self.total = 0;
        }
    }
}

/// A player's `Entity.tickCount` and `Player.lastLevelUpTime`. This state belongs
/// to the player entity, not the level or the server's clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct LevelUpSound {
    ticks: i32,
    last: i32,
}

impl LevelUpSound {
    /// `Entity.commonTick`: called once for this player, including spectators.
    pub fn tick(&mut self) {
        self.ticks = self.ticks.wrapping_add(1);
    }

    /// `Player.giveExperienceLevels`: return the sound's volume and start its
    /// cooldown when a positive change lands on a fifth level.
    pub fn give_levels(&mut self, amount: i32, level: i32) -> Option<f32> {
        if amount > 0 && level % 5 == 0 && (self.last as f32) < self.ticks as f32 - 100.0 {
            self.last = self.ticks;
            Some((if level > 30 { 1.0 } else { level as f32 / 30.0 }) * 0.75)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod level_up_sound {
    use super::LevelUpSound;

    #[test]
    fn players_have_independent_ages_and_cooldowns() {
        let mut a = LevelUpSound::default();
        let mut b = LevelUpSound::default();
        for _ in 0..100 {
            a.tick();
            b.tick();
        }
        assert_eq!(a.give_levels(1, 5), None);
        a.tick();
        b.tick();
        assert_eq!(a.give_levels(1, 5), Some(0.125));
        assert_eq!(b.give_levels(1, 10), Some(0.25));
        assert_eq!(a.give_levels(1, 10), None);
        for _ in 0..100 {
            a.tick();
        }
        assert_eq!(a.give_levels(1, 30), None);
        a.tick();
        assert_eq!(a.give_levels(1, 30), Some(0.75));
        let mut respawned = LevelUpSound::default();
        assert_eq!(respawned.give_levels(1, 35), None);
        for _ in 0..101 {
            respawned.tick();
        }
        assert_eq!(respawned.give_levels(-1, 35), None);
        assert_eq!(respawned.give_levels(1, 34), None);
        assert_eq!(respawned.give_levels(1, 35), Some(0.75));
    }
}

/// The `PaintingVariant.DIRECT_CODEC` definition used by the registry and renderer.
#[derive(Clone, Debug)]
pub struct PaintingVariant {
    pub id: std::sync::Arc<str>,
    pub asset: String,
    pub width: i32,
    pub height: i32,
    pub title: Option<serde_json::Value>,
    pub author: Option<serde_json::Value>,
}

impl PaintingVariant {
    pub fn decode(id: &str, value: &serde_json::Value) -> Result<Self, String> {
        let axis = |field: &str| {
            value[field]
                .as_i64()
                .filter(|n| (1..=16).contains(n))
                .map(|n| n as i32)
                .ok_or_else(|| format!("{field} must be an integer in 1..=16"))
        };
        let asset = value["asset_id"].as_str().ok_or("asset_id is required")?;
        let asset = sunrise_core::identifier::Identifier::parse(asset)
            .map_err(|error| format!("asset_id: {error}"))?
            .to_string();
        Ok(Self {
            id: id.into(),
            asset: asset.trim_start_matches("minecraft:").to_owned(),
            width: axis("width")?,
            height: axis("height")?,
            title: value.get("title").cloned(),
            author: value.get("author").cloned(),
        })
    }
}

/// A painting (`Painting`, a `HangingEntity`): the block it hangs in, the
/// 2D data value of the direction it faces, and its variant.
#[derive(Clone, Debug)]
pub struct Painting {
    /// Entity-owned registration metadata; content clones are detached.
    pub attachment_lifetime: sunrise_mod_runtime::attachments::AttachmentLifetime,
    pub join: crate::join_seq::JoinSeq,
    pub id: u32,
    pub pos: [i32; 3],
    /// Direction.get2DDataValue: south 0, west 1, north 2, east 3.
    pub facing: u8,
    pub variant: std::sync::Arc<str>,
    /// Asset path for inline or synced custom variants; `None` looks up `variant`.
    pub asset: Option<std::sync::Arc<str>>,
    pub width: i32,
    pub height: i32,
    /// BlockAttachedEntity.checkInterval.
    pub check_interval: u32,
    /// `Entity.tickCount`: starts at 0, +1 each entity tick, not saved.
    pub ticks: u32,
}
impl Painting {
    pub fn step(&self) -> [i32; 3] {
        [[0, 0, 1], [-1, 0, 0], [0, 0, -1], [1, 0, 0]][usize::from(self.facing & 3)]
    }
    /// Direction.getCounterClockWise (the painting's left as it faces out).
    pub fn left(&self) -> [i32; 3] {
        [[1, 0, 0], [0, 0, 1], [-1, 0, 0], [0, 0, -1]][usize::from(self.facing & 3)]
    }
    /// Painting.calculateBoundingBox: (min, max).
    pub fn bounds(&self) -> ([f64; 3], [f64; 3]) {
        let s = self.step().map(f64::from);
        let l = self.left().map(f64::from);
        let off = |n: i32| if n % 2 == 0 { 0.5 } else { 0.0 };
        let (h, v) = (off(self.width), off(self.height));
        let c = [
            f64::from(self.pos[0]) + 0.5 - s[0] * 0.46875 + l[0] * h,
            f64::from(self.pos[1]) + 0.5 + v,
            f64::from(self.pos[2]) + 0.5 - s[2] * 0.46875 + l[2] * h,
        ];
        let along_x = s[0] != 0.0;
        let size = [
            if along_x {
                0.0625
            } else {
                f64::from(self.width)
            },
            f64::from(self.height),
            if along_x {
                f64::from(self.width)
            } else {
                0.0625
            },
        ];
        (
            std::array::from_fn(|i| c[i] - size[i] / 2.0),
            std::array::from_fn(|i| c[i] + size[i] / 2.0),
        )
    }
}
/// The `DyeColor` ordinal of a colour name (`DyeColor.byName`), white when
/// the name is unknown.
pub fn dye_index(name: &str) -> u8 {
    sunrise_level::dyes::DYE_NAMES
        .iter()
        .position(|n| *n == name)
        .unwrap_or(0) as u8
}
/// A cushion (`Cushion`, a `BlockAttachedEntity`): a 1 x 0.25 block pillow
/// that seats a player. It sits on the top face of a block, at the clicked
/// height, and is coloured by its `DyeColor`.
#[derive(Clone, Debug)]
pub struct Cushion {
    /// Entity-owned registration metadata; content clones are detached.
    pub attachment_lifetime: sunrise_mod_runtime::attachments::AttachmentLifetime,
    pub join: crate::join_seq::JoinSeq,
    pub id: u32,
    /// `Entity.position`: the bottom centre of the cushion.
    pub position: Vec3,
    /// `BlockAttachedEntity.pos` (the saved `block_pos`).
    pub pos: [i32; 3],
    pub yaw: f32,
    /// `DyeColor` ordinal: white 0, orange 1, ..., black 15.
    pub color: u8,
    /// BlockAttachedEntity.ticksSinceLastCheck.
    pub check_interval: u32,
    /// `Entity.getCustomName` (`CustomName`), copied onto the dropped cushion.
    pub custom_name: Option<sunrise_core::nbt::Tag>,
}
impl Cushion {
    /// `Entity.makeBoundingBox`: x and z half a block out, y to y + 0.25.
    pub fn bounds(&self) -> ([f64; 3], [f64; 3]) {
        let p = self.position;
        (
            [p.x - 0.5, p.y, p.z - 0.5],
            [p.x + 0.5, p.y + 0.25, p.z + 0.5],
        )
    }
    /// The `DyeColor` name of the cushion.
    pub fn color_name(&self) -> &'static str {
        sunrise_level::dyes::DYE_NAMES[usize::from(self.color.min(15))]
    }
    /// `Cushion.getCushionItemStackWithData`: the matching coloured item.
    pub fn item(&self) -> String {
        format!("minecraft:{}_cushion", self.color_name())
    }
    /// `Cushion.getCushionItemStackWithData`: the coloured item with
    /// `DataComponents.CUSTOM_NAME` from `getCustomName`, as the item frame
    /// beside it does.
    pub fn item_stack_with_data(&self) -> ItemStack {
        let mut stack = ItemStack::new(&self.item(), 1);
        if let Some(name) = &self.custom_name {
            stack.data_mut().custom_name = Some(name.clone());
        }
        stack
    }
    /// `Cushion.interact`'s seat: the vehicle's PASSENGER attachment
    /// `(0, 0.25, 0)` less the player's `(0, 0.6, 0)`.
    pub fn seat(&self) -> Vec3 {
        Vec3::new(self.position.x, self.position.y - 0.35, self.position.z)
    }
    /// `Entity.getDismountLocationForPassenger`: on top of the cushion.
    pub fn dismount(&self) -> Vec3 {
        Vec3::new(self.position.x, self.position.y + 0.25, self.position.z)
    }
}
/// An item frame or glow item frame (`ItemFrame`).
#[derive(Clone, Debug)]
pub struct ItemFrame {
    /// Entity-owned registration metadata; content clones are detached.
    pub attachment_lifetime: sunrise_mod_runtime::attachments::AttachmentLifetime,
    pub join: crate::join_seq::JoinSeq,
    pub id: u32,
    pub pos: [i32; 3],
    /// Direction.get3DDataValue: down 0, up 1, north 2, south 3, west 4, east 5.
    pub facing: u8,
    pub glow: bool,
    pub item: Option<crate::items::ItemStack>,
    /// Java `%` remainder of ItemRotation, so -7..=7 in 45 degree steps.
    pub rotation: i8,
    pub drop_chance: f32,
    pub fixed: bool,
    pub invisible: bool,
    pub check_interval: u32,
    /// `Entity.tickCount`: starts at 0, +1 each entity tick, not saved.
    pub ticks: u32,
    /// `Entity.getCustomName` (`CustomName`), copied onto the dropped frame.
    pub custom_name: Option<sunrise_core::nbt::Tag>,
    pub random: LegacyRandom,
}
impl ItemFrame {
    /// `ItemFrame.setDirection`: wall frames use the horizontal direction's yaw;
    /// ceiling and floor frames use the vertical direction's pitch.
    pub fn entity_rotation(&self) -> [f32; 2] {
        match self.facing {
            0 => [0.0, 90.0],
            1 => [0.0, -90.0],
            2 => [180.0, 0.0],
            3 => [0.0, 0.0],
            4 => [90.0, 0.0],
            _ => [270.0, 0.0],
        }
    }

    /// `ItemFrame.setItem`: the adopted stack holds one item. The caller owns
    /// the old map association, sounds and neighbour updates.
    pub fn set_item(&mut self, item: Option<ItemStack>) {
        self.item = item.map(|stack| ItemStack { count: 1, ..stack });
    }
    pub fn step(&self) -> [i32; 3] {
        [
            [0, -1, 0],
            [0, 1, 0],
            [0, 0, -1],
            [0, 0, 1],
            [-1, 0, 0],
            [1, 0, 0],
        ][usize::from(self.facing.min(5))]
    }
    pub fn kind(&self) -> &'static str {
        if self.glow {
            "minecraft:glow_item_frame"
        } else {
            "minecraft:item_frame"
        }
    }
    /// ItemFrame.createBoundingBox (without a framed map): (min, max).
    pub fn bounds(&self) -> ([f64; 3], [f64; 3]) {
        let s = self.step().map(f64::from);
        let c: [f64; 3] = std::array::from_fn(|i| f64::from(self.pos[i]) + 0.5 - s[i] * 0.46875);
        let size: [f64; 3] = std::array::from_fn(|i| if s[i] != 0.0 { 0.0625 } else { 0.75 });
        (
            std::array::from_fn(|i| c[i] - size[i] / 2.0),
            std::array::from_fn(|i| c[i] + size[i] / 2.0),
        )
    }
    /// `ItemFrame.getFrameItemStackWithData`: `getFrameItemStack` (a glow
    /// frame's own item), then `DataComponents.CUSTOM_NAME` from
    /// `getCustomName`.
    pub fn item_stack_with_data(&self) -> ItemStack {
        let mut item_stack = ItemStack::new(self.kind(), 1);
        if let Some(name) = &self.custom_name {
            item_stack.data_mut().custom_name = Some(name.clone());
        }
        item_stack
    }
}
/// An armor stand (`ArmorStand`).
#[derive(Clone, Debug)]
pub struct ArmorStand {
    /// Entity-owned registration metadata; content clones are detached.
    pub attachment_lifetime: sunrise_mod_runtime::attachments::AttachmentLifetime,
    /// `Entity.xRot`: kept for `/tp` and `Rotation`; the kind turns only in yaw.
    pub pitch: f32,
    /// `Entity.xRotO`: pitch at the start of the tick for camera interpolation.
    pub previous_pitch: f32,
    /// When it joined its list ([`crate::join_seq`]); set by the list, not by the spawn site.
    pub join: crate::join_seq::JoinSeq,
    pub fire_ticks: i32,
    /// Unsaved fire immunity granted on this logical side.
    pub mod_fire_immune: bool,
    pub health: f32,
    /// `LivingEntity.deathTime`: the ticks a stand at 0 health has been dead.
    pub death_time: i16,
    pub tick_count: i32,
    /// Transient `LivingEntity.lastHurtMob`; an owned arrow or wind charge can set it.
    pub last_hurt_mob: Option<(u32, i32)>,
    pub id: u32,
    /// `Entity.getRandom`, after UUID and living construction draws.
    pub random: LegacyRandom,
    pub position: Vec3,
    /// `Entity.xo`, `yo`, `zo`: the position at the start of the tick, which
    /// the renderer blends toward `position` by the partial tick.
    pub previous: Vec3,
    pub velocity: Vec3,
    /// `LivingEntity.hurtTime`, counted down each tick.
    pub hurt_time: i32,
    /// `Entity.airSupply`: refilled by 4 a tick up to 300 while it can breathe.
    pub air: i32,
    /// Shared flag 7, `FallFlying`: a stand cannot glide, so its first tick clears it.
    pub fall_flying: bool,
    /// `LivingEntity.currentImpulseContextResetGraceTime`.
    pub impulse_grace: i32,
    /// `Entity.yRot`; `ArmorStand.tickHeadTurn` copies it to `yBodyRot`.
    pub yaw: f32,
    pub body_yaw: f32,
    /// `Entity.yRotO`: previous view yaw.
    pub previous_yaw: f32,
    /// `LivingEntity.yBodyRotO`; `tickHeadTurn` syncs it to `yRotO` after admission.
    pub previous_body_yaw: f32,
    pub head_yaw: f32,
    pub previous_head_yaw: f32,
    /// Every `EquipmentSlot`, by ordinal: mainhand, offhand, feet, legs, chest,
    /// head, body, saddle. Vanilla's `canUseSlot` refuses body and saddle only
    /// for interaction; `setItemSlot` and the save keep them.
    pub equipment: sunrise_items::equipment::Equipment,
    pub small: bool,
    pub show_arms: bool,
    pub no_base_plate: bool,
    pub invisible: bool,
    pub marker: bool,
    /// `Entity.isNoGravity` (`NoGravity` in the save). `ArmorStand.hasPhysics`
    /// is false with it, as with `Marker`: `travel` does nothing, so the stand
    /// never moves, and the blocks it stands in do not touch it.
    pub no_gravity: bool,
    /// `Entity.noPhysics`. `None` is `!hasPhysics()` (`Marker` or `NoGravity`),
    /// which `ArmorStand.readAdditionalSaveData` recomputes on every load;
    /// `Some` is a mod's write, which lasts only until the stand is saved and
    /// loaded again.
    pub no_physics: Option<bool>,
    /// `CustomName`: the text of a hologram.
    pub custom_name: Option<crate::text::Text>,
    /// `CustomNameVisible`.
    pub name_visible: bool,
    /// Game time of the last hit (the wobble and the second-hit window).
    pub last_hit: i64,
    /// `DATA_HEAD_POSE` to `DATA_RIGHT_LEG_POSE`: head, body, left arm, right
    /// arm, left leg, right leg, in degrees (`POSE_KEYS` order).
    pub pose: [[f32; 3]; 6],
    /// `Entity.vehicle` on a server's client (`SetPassengers`): the mob it
    /// rides, which seats it every tick (`positionRider`), as NPC plugins hang
    /// a second name over a player's head.
    pub vehicle: Option<u32>,
    /// `ArmorStand.disabledSlots`: bit `filterFlag` locks a slot, bit
    /// `filterFlag + 8` takes from it and bit `filterFlag + 16` puts into it.
    pub disabled_slots: i32,
    /// Server-only `LivingEntity.lastPos` for location-based enchantments.
    pub enchantment_block: Option<BlockPos>,
    /// `Entity.tickCount` for periodic equipment effects.
    pub enchantment_tick: u32,
    /// `Entity.fallDistance`, also used by `LivingEntity.checkFallDamage`'s landing callback.
    /// Each downward movement is narrowed to float before adding to this double.
    pub enchantment_fall_distance: f64,
    /// Last equipment seen by `collectEquipmentChanges`; not saved to NBT.
    pub enchantment_equipment: sunrise_items::equipment::Equipment,
    /// The live attribute map (`AttributeMap`), built when an item with
    /// attribute modifiers is worn or it is read.
    pub live: crate::living_attributes::LivingAttributes,
    /// `Entity.firstTick`: true until its first tick has run.
    pub first_tick: bool,
    pub ground: sunrise_world::ground_support::GroundState,
    /// Its `AttributeMap` holds a `movement_speed` instance, which
    /// `AttributeMap.pack` saves. `LivingEntity.aiStep`'s `removeFrost` makes it
    /// (`getAttribute`) on the stand's first server tick, and a save that lists
    /// it loads it back, so a stand summoned this tick has none yet.
    pub speed_instance: bool,
    /// `NAME_TAG_DISTANCE` and `BELOW_NAME_DISTANCE` from server attribute updates.
    pub nameplate: crate::entities::nameplate::NameplateDistances,
    /// What `entity_size` answered at its last size refresh (`EntityEvent.Size`): its box takes
    /// the answer while it holds for its vanilla size.
    pub size_answer: Option<crate::pose::SizeAnswer>,
    /// Synced client-side effects from `UpdateMobEffect` / `RemoveMobEffect`, as a mob's.
    pub client_effects: crate::effects::PlayerEffects,
    /// `MobEffectInstance.BlendState` (previous, current) for its client-side darkness.
    pub darkness_blend: (f32, f32),
}
/// The fluid a stand travels in (`LivingEntity.travelInFluid`: water first,
/// then lava), from the heights `Entity::push_by_fluid` measured.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum StandMedium {
    Air,
    Water,
    /// `shallow`: `isInShallowFluid(LAVA)`, the lava is no deeper than
    /// `getFluidJumpThreshold` (0.4 for a stand, whose eyes are above 0.4).
    Lava {
        shallow: bool,
    },
}
impl StandMedium {
    pub fn from_heights(water_height: f64, lava_height: f64) -> Self {
        if water_height > 0.0 {
            Self::Water
        } else if lava_height > 0.0 {
            Self::Lava {
                shallow: lava_height <= 0.4,
            }
        } else {
            Self::Air
        }
    }
}
/// Armor stands share LivingEntity's block/tag and trapdoor climb predicate.
struct StandClimbing<'a> {
    world: &'a World,
    position: BlockPos,
}
impl sunrise_world::climbing::ClimbingWorld for StandClimbing<'_> {
    type Error = std::convert::Infallible;
    fn is_spectator(&mut self) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn is_fall_flying(&mut self) -> Result<bool, Self::Error> {
        Ok(false)
    }
    fn block_position(&mut self) -> Result<BlockPos, Self::Error> {
        Ok(self.position)
    }
    fn in_block_state(&mut self) -> Result<sunrise_world::registry::BlockStateId, Self::Error> {
        self.block_state(self.position)
    }
    fn block_state(
        &mut self,
        p: BlockPos,
    ) -> Result<sunrise_world::registry::BlockStateId, Self::Error> {
        Ok(self.world.block(p.x, p.y, p.z))
    }
    fn set_last_climbable_position(&mut self, _: BlockPos) {}
}
/// What a stand's travel carries from [`ArmorStand::tick_move`] to
/// [`ArmorStand::tick_settle`].
#[derive(Clone, Copy)]
pub struct StandStep {
    falling: bool,
    friction: f32,
}
#[cfg(test)]
impl ArmorStand {
    /// A whole travel for the unit tests, with no bubble column.
    pub(crate) fn tick(
        &mut self,
        scene: &mut crate::collisions::SceneCollisions,
        facts: &BlockFacts,
        climbing: &sunrise_world::climbing::ClimbingRegistry<'_>,
        in_water: bool,
        border: Option<&BorderGeometry>,
    ) {
        let context = crate::test_resources::server_loop_context();
        if let Some(step) = self.tick_move(
            scene,
            facts,
            None,
            climbing,
            in_water,
            border,
            &[],
            context,
            crate::motion::dry_side(),
        ) {
            let medium = if in_water {
                StandMedium::Water
            } else {
                StandMedium::Air
            };
            self.tick_settle(medium, step);
        }
        self.apply_honey_slide(scene.world, facts);
    }
}
impl ArmorStand {
    pub const SLOTS: [&'static str; 6] = ["mainhand", "offhand", "feet", "legs", "chest", "head"];
    /// The `Pose` compound's keys.
    pub const POSE_KEYS: [&'static str; 6] =
        ["Head", "Body", "LeftArm", "RightArm", "LeftLeg", "RightLeg"];
    /// `ArmorStand.DEFAULT_*_POSE`.
    pub const DEFAULT_POSE: [[f32; 3]; 6] = [
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0],
        [-10.0, 0.0, -10.0],
        [-15.0, 0.0, 10.0],
        [-1.0, 0.0, -1.0],
        [1.0, 0.0, 1.0],
    ];
    /// Where the renderer draws the stand `alpha` of the way through the tick
    /// (`EntityRenderer`'s `Mth.lerp(partialTick, xo, x)` and so on).
    pub fn render_position(&self, alpha: f64) -> Vec3 {
        self.previous.lerp(self.position, alpha)
    }
    /// `ArmorStand.getDimensionsMarker(false)`: the width, height and eye
    /// height of the box a stand has without the marker flag, which is the box
    /// a marker's light probe walks (`MARKER_DIMENSIONS` itself is empty, and
    /// `isBaby()` is `isSmall()`).
    pub fn probe_dimensions(&self) -> (f64, f64, f64) {
        if self.small {
            (0.25, 0.9875, 0.9875)
        } else {
            (0.5, 1.975, 1.7775)
        }
    }
    /// `ArmorStand.getLightProbePosition` as the cell the renderer reads
    /// (`BlockPos.containing` of the position it returns): a marker walks
    /// `BlockPos.betweenClosed` of its non-marker box at `position()` - it
    /// takes no partial tick - and returns the first cell whose brightness
    /// `max(block, sky)` is 15, else the brightest, ties going to the first
    /// cell walked; every other stand is lit from its eye
    /// (`Entity.getLightProbePosition` is `getEyePosition(partialTick)`).
    /// `light` is the level's packed light.
    pub fn light_probe(
        &self,
        partial: f64,
        mut light: impl FnMut(i32, i32, i32) -> u8,
    ) -> [i32; 3] {
        let (width, height, eye) = self.probe_dimensions();
        if !self.marker {
            let at = self.render_position(partial);
            return [
                at.x.floor() as i32,
                (at.y + eye).floor() as i32,
                at.z.floor() as i32,
            ];
        }
        let half = width / 2.0;
        let (lo_x, hi_x) = (
            (self.position.x - half).floor() as i32,
            (self.position.x + half).floor() as i32,
        );
        let (lo_z, hi_z) = (
            (self.position.z - half).floor() as i32,
            (self.position.z + half).floor() as i32,
        );
        let (lo_y, hi_y) = (
            self.position.y.floor() as i32,
            (self.position.y + height).floor() as i32,
        );
        // `BlockPos.betweenClosed` walks x fastest, then y, then z; the stand's
        // own cell is what it falls back to when the box is empty.
        let mut best = [
            self.position.x.floor() as i32,
            self.position.y.floor() as i32,
            self.position.z.floor() as i32,
        ];
        let mut brightest = i32::MIN;
        for cell_z in lo_z..=hi_z {
            for cell_y in lo_y..=hi_y {
                for cell_x in lo_x..=hi_x {
                    let level = light(cell_x, cell_y, cell_z);
                    let brightness = i32::from((level & 15).max(level >> 4));
                    if brightness == 15 {
                        return [cell_x, cell_y, cell_z];
                    }
                    if brightness > brightest {
                        brightest = brightness;
                        best = [cell_x, cell_y, cell_z];
                    }
                }
            }
        }
        best
    }
    /// `Entity.isOnFire`: an immune stand does not count as burning.
    pub fn is_on_fire(&self) -> bool {
        !self.mod_fire_immune && self.fire_ticks > 0
    }
    /// `EntityRenderer.getPackedLightCoords` for the cell `light_probe` chose:
    /// the level's sky light over the cell's block light, at 15 block light
    /// while the stand `isOnFire`. `light` is the level's packed light.
    pub fn render_light(&self, light: u8) -> u32 {
        let block = if self.is_on_fire() { 15 } else { light & 15 };
        (u32::from(light >> 4) * 16) | ((u32::from(block) * 16) << 8)
    }
    /// The body's yaw `alpha` of the way through the tick
    /// (`LivingEntityRenderer.solveBodyRot`: `Mth.rotLerp(partialTick,
    /// yBodyRotO, yBodyRot)`).
    pub fn render_yaw(&self, alpha: f32) -> f32 {
        self.previous_body_yaw
            + alpha
                * sunrise_level::camera_math::wrap_degrees(self.body_yaw - self.previous_body_yaw)
    }
    /// A server's stand on its client, each tick:
    /// `setOldPosAndRot` (`ClientLevel.tickNonPassenger`), then the step of
    /// its `SteppedInterpolationHandler` (`step`: the position and yaw it is
    /// put at, when one is due). The yaw
    /// turns the short way and stays continuous, so a frame between ticks
    /// never spins it.
    pub fn interpolate(&mut self, step: Option<(Vec3, f32, f32)>) {
        self.previous = self.position;
        self.previous_yaw = self.yaw;
        self.previous_pitch = self.pitch;
        if let Some((at, yaw, pitch)) = step {
            self.position = at;
            self.yaw += sunrise_level::camera_math::wrap_degrees(yaw - self.yaw);
            self.pitch = (pitch % 360.0).clamp(-90.0, 90.0);
        }
        self.previous_body_yaw = self.previous_yaw;
        self.body_yaw = self.yaw;
    }
    /// Snap position and view rotation without changing the body's or head's
    /// yaw samples (`Entity.snapTo`).
    pub fn snap(&mut self, at: Vec3, yaw: f32, pitch: f32) {
        self.position = at;
        self.previous = at;
        self.yaw = yaw;
        self.previous_yaw = yaw;
        self.pitch = (pitch % 360.0).clamp(-90.0, 90.0);
        self.previous_pitch = self.pitch;
    }
    pub fn new(id: u32, position: Vec3, yaw: f32) -> Self {
        Self::new_with_uuid(id, position, yaw).0
    }

    pub fn new_with_uuid(id: u32, position: Vec3, yaw: f32) -> (Self, [i32; 4]) {
        let (mut random, uuid) = crate::mobs::living_random();
        // LivingEntity's initial yaw draw, even when placement or loading replaces it.
        random.next_float();
        (
            Self {
                size_answer: None,
                attachment_lifetime: Default::default(),
                pitch: 0.0,
                previous_pitch: 0.0,
                join: Default::default(),
                fire_ticks: 0,
                mod_fire_immune: false,
                death_time: 0,
                health: 20.0,
                tick_count: 0,
                last_hurt_mob: None,
                id,
                random,
                position,
                hurt_time: 0,
                air: 300,
                fall_flying: false,
                impulse_grace: 0,
                previous: position,
                velocity: Vec3::ZERO,
                yaw,
                body_yaw: yaw,
                previous_yaw: yaw,
                previous_body_yaw: yaw,
                head_yaw: yaw,
                previous_head_yaw: yaw,
                equipment: Default::default(),
                small: false,
                show_arms: false,
                no_base_plate: false,
                invisible: false,
                marker: false,
                no_gravity: false,
                no_physics: None,
                custom_name: None,
                name_visible: false,
                last_hit: i64::MIN / 2,
                pose: Self::DEFAULT_POSE,
                vehicle: None,
                disabled_slots: 0,
                enchantment_block: None,
                enchantment_fall_distance: 0.0,
                enchantment_tick: 0,
                enchantment_equipment: Default::default(),
                live: Default::default(),
                first_tick: true,
                ground: Default::default(),
                speed_instance: false,
                nameplate: Default::default(),
                client_effects: Default::default(),
                darkness_blend: (0.0, 0.0),
            },
            uuid,
        )
    }
    /// `Entity.getOnPos(0.2)` (`getOnPosLegacy`): the supporting block's
    /// column 0.2 below the feet, the supporting block itself when `tall`
    /// says it is a fence, wall or fence gate, else the cell 0.2 below.
    pub fn on_pos_legacy(&self, tall: impl Fn(BlockPos) -> bool) -> BlockPos {
        let y = (self.position.y - f64::from(0.2_f32)).floor() as i32;
        match self.ground.supporting_block {
            Some(p) if tall(p) => p,
            Some(p) => BlockPos::new(p.x, y, p.z),
            None => BlockPos::new(
                self.position.x.floor() as i32,
                y,
                self.position.z.floor() as i32,
            ),
        }
    }
    /// `EquipmentSlot.getFilterFlag` of an `equipment` index (main hand, off
    /// hand, feet, legs, chest, head).
    fn filter_flag(slot: usize) -> u32 {
        [0, 5, 1, 2, 3, 4][slot]
    }
    /// `ArmorStand.isDisabled`: the slot is locked, or it is a hand and the
    /// stand has no arms.
    pub fn is_disabled(&self, slot: usize) -> bool {
        self.disabled_slots & (1 << Self::filter_flag(slot)) != 0 || (slot < 2 && !self.show_arms)
    }
    /// The `swapItem` locks: taking from a filled slot or putting into an
    /// empty one can each be disabled.
    pub fn swap_locked(&self, slot: usize) -> bool {
        let offset = if self.equipment[slot].is_some() {
            8
        } else {
            16
        };
        self.disabled_slots & (1 << (Self::filter_flag(slot) + offset)) != 0
    }
    pub fn environment_tick(
        &mut self,
        mut world: &World,
        facts: &BlockFacts,
        registry: &sunrise_world::registry::BlockRegistry,
    ) -> Option<&'static str> {
        self.environment_tick_inner(&mut world, facts, registry, |_, _| false)
    }
    /// As `environment_tick`, allowing the caller to decide each actual damage attempt.
    pub fn environment_tick_with_hook(
        &mut self,
        mut world: &mut World,
        facts: &BlockFacts,
        registry: &sunrise_world::registry::BlockRegistry,
        mut invulnerable: impl FnMut(&mut World, &str) -> bool,
    ) -> Option<&'static str> {
        self.environment_tick_inner(&mut world, facts, registry, |world, damage| {
            invulnerable(world, damage)
        })
    }
    fn environment_tick_inner<W: std::ops::Deref<Target = World>>(
        &mut self,
        world: &mut W,
        facts: &BlockFacts,
        registry: &sunrise_world::registry::BlockRegistry,
        mut invulnerable: impl FnMut(&mut W, &str) -> bool,
    ) -> Option<&'static str> {
        let (lo, hi) = (self.bounds().min_position(), self.bounds().max_position());
        // The damage type of the last hit that lowered its health.
        let mut hit_by = None;
        let mut water = false;
        let mut lava = false;
        for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
            for y in lo.y.floor() as i32..hi.y.ceil() as i32 {
                for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                    match facts.get(world.block(x, y, z)).fluid {
                        Fluid::Water => water = true,
                        Fluid::Lava => lava = true,
                        _ => {}
                    }
                }
            }
        }
        if water {
            self.fire_ticks = 0;
        } else if self.mod_fire_immune {
            self.fire_ticks = self.fire_ticks.min(0);
        }
        if self.fire_ticks > 0 {
            if self.fire_ticks % 20 == 0
                && !lava
                && !self.invisible
                && !self.marker
                && self.health > 0.5
                && !invulnerable(world, "minecraft:on_fire")
            {
                self.health -= 4.0;
                hit_by = Some("minecraft:on_fire");
            }
            self.fire_ticks -= 1;
        }
        if lava && !self.mod_fire_immune {
            self.fire_ticks = self.fire_ticks.max(300);
        }
        // BaseFireBlock.entityInside: FIRE_IGNITE runs before hurt(inFire), even when invisible.
        if !self.mod_fire_immune
            && !self.marker
            && !self.no_gravity
            && crate::motion::environment::fire_contact(world, registry, self.bounds()) > 0.0
        {
            if self.fire_ticks < 0 {
                self.fire_ticks += 1;
            }
            if self.fire_ticks >= 0 {
                self.fire_ticks = self.fire_ticks.max(160);
            }
            if !self.invisible && !invulnerable(world, "minecraft:in_fire") {
                if self.fire_ticks > 0 {
                    self.health -= 0.15;
                    hit_by = Some("minecraft:in_fire");
                } else {
                    // ArmorStand.hurtServer(#ignites_armor_stands): not on fire yet -> ignite 5s.
                    self.fire_ticks = self.fire_ticks.max(100);
                }
            }
        }
        // `ArmorStand.causeDamage` breaks it only when this tick's damage left
        // it at 0.5 or less: the damage type `brokenByAnything` receives.
        hit_by.filter(|_| self.health <= 0.5)
    }
    /// `ArmorStand.getDefaultDimensions`: MARKER_DIMENSIONS, or BABY_DIMENSIONS (the type's at
    /// half scale, its eyes at 0.9875) for a small one, else the type's.
    pub fn default_dimensions(&self) -> [f32; 3] {
        match (self.marker, self.small) {
            (true, _) => [0.0; 3],
            (_, true) => [0.25, 0.9875, 0.9875],
            _ => [0.5, 1.975, 1.7775],
        }
    }
    /// EntityType.ARMOR_STAND dimensions (0.5 x 1.975F, half for small).
    pub fn bounds(&self) -> Aabb {
        let s = if self.small { 0.5 } else { 1.0 };
        // EntityDimensions.makeBoundingBox adds the float height, widened
        // (halving it for the small stand is exact).
        let (w, h) = (0.25 * s, f64::from(1.975_f32) * s);
        let p = self.position;
        let vanilla = Aabb::new(
            Vec3::new(p.x - w, p.y, p.z - w),
            Vec3::new(p.x + w, p.y + h, p.z + w),
        );
        crate::pose::SizeAnswer::sized_box(self.size_answer, self.default_dimensions(), p, vanilla)
    }
    /// Leather boots on its feet: it walks on powder snow (`PowderSnowBlock.canEntityWalkOnPowderSnow`).
    fn feet_are_leather_boots(&self) -> bool {
        self.equipment[2]
            .as_ref()
            .is_some_and(|s| &*s.item == "minecraft:leather_boots")
    }
    /// The stand's live context for `Entity.collide` and `Level.findSupportingBlock`.
    pub fn support_collision(
        &self,
    ) -> (
        sunrise_world::collision_context::CollisionContext<'_>,
        crate::collisions::PowderSnowSubject<'_>,
    ) {
        use sunrise_world::{
            collision_context::{
                CollisionContext, CollisionEntity, CollisionItem, EntityContextKind,
            },
            entity_obstruction::EntityIdentity,
        };
        let subject = crate::collisions::PowderSnowSubject {
            leather_boots: self.feet_are_leather_boots(),
            fall_distance: self.enchantment_fall_distance,
            main_hand: Some(
                self.equipment[sunrise_items::equipment::MAINHAND]
                    .as_ref()
                    .map_or(CollisionItem::EMPTY, |stack| CollisionItem {
                        item: &stack.item,
                        count: stack.count.min(i32::MAX as u32) as i32,
                    }),
            ),
            ..Default::default()
        };
        let context = CollisionContext::for_entity(
            CollisionEntity {
                identity: EntityIdentity(u64::from(self.id)),
                kind: EntityContextKind::Living,
                y: self.position.y,
                descending: false,
                main_hand: subject.main_hand,
            },
            false,
        );
        (context, subject)
    }
    fn collide(
        &self,
        gate: &mut sunrise_world::move_gate::MoveGate,
        scene: &SceneCollisions<'_, '_>,
        delta: Vec3,
        border: Option<&BorderGeometry>,
        entities: &[Aabb],
    ) -> crate::Result<Vec3> {
        let (context, subject) = self.support_collision();
        let mut collisions = EntityCollisions {
            blocks: SceneCollisions {
                world: scene.world,
                binding: scene.binding,
                subject: Some(subject),
            },
            entities,
            border: border.map(|b| (b, [self.position.x, self.position.z])),
        };
        crate::move_gate::entities_resolve_with_context(
            gate,
            &mut collisions,
            self.bounds(),
            delta,
            0.0,
            self.ground.on_ground,
            context,
        )
    }
    /// LivingEntity.travel (armor stands have no other movement): clamp tiny
    /// velocity components (`LivingEntity.aiStep`), move by the velocity (a
    /// landing stops the fall, `Block.updateEntityMovementAfterFallOn`, and a
    /// wall collision zeros that horizontal axis), let the blocks it is inside
    /// push it (`push`: a bubble column), then in water the 0.8 slowdown and
    /// `getFluidFallingAdjustedMovement`, in air gravity 0.08, ground/air
    /// horizontal drag (`friction * 0.91`), and the 0.98 vertical drag, so a
    /// stand at rest on the ground keeps -0.0784 as vanilla's.
    ///
    /// The game runs the fluid push first (`Entity::push_by_fluid`), then
    /// this up to the blocks' push, then the bubble column
    /// (`Entity::push_by_bubble_column`), then [`ArmorStand::tick_settle`].
    /// `entities` are the boxes `EntityGetter.getEntityCollisions` gives the
    /// stand (a living shulker, a boat, a happy ghast held still), which stop
    /// its move as blocks do. None: a marker or a stand with `NoGravity` does
    /// not travel.
    pub fn tick_move(
        &mut self,
        scene: &SceneCollisions<'_, '_>,
        facts: &BlockFacts,
        movement: Option<&BoundMovementProperties>,
        climbing: &sunrise_world::climbing::ClimbingRegistry<'_>,
        in_water: bool,
        border: Option<&BorderGeometry>,
        entities: &[Aabb],
        context: &sunrise_world::native_load_context::LoopContext,
        side: &dyn crate::motion::SideLevel,
    ) -> Option<StandStep> {
        use crate::entity_rules::EntityRules;
        self.previous = self.position;
        if self.marker {
            return None;
        }
        let v = self.velocity;
        let mut wanted = Vec3::new(
            if v.x.abs() < 0.003 { 0.0 } else { v.x },
            if v.y.abs() < 0.003 { 0.0 } else { v.y },
            if v.z.abs() < 0.003 { 0.0 } else { v.z },
        );
        self.velocity = wanted;
        // ArmorStand.travel: no physics with NoGravity, the velocity stays.
        if self.is_no_gravity() {
            return None;
        }
        // `travelInFluid` reads `isFalling` before the move and any push;
        // `travelInAir` reads ground friction before the move.
        let falling = wanted.y <= 0.0;
        let friction = if !in_water && self.ground.on_ground {
            let below = crate::motion::block_support_at(
                scene.world,
                movement,
                self.position,
                self.ground.supporting_block,
                sunrise_world::movement_support::MOVEMENT_SUPPORT_OFFSET,
            );
            let kind = "minecraft:armor_stand";
            block_friction(
                context,
                side,
                (self.id, kind),
                scene.world,
                scene.binding.registry(),
                facts,
                below,
            )
        } else {
            1.0
        };
        let on_climbable = |position| {
            sunrise_world::climbing::on_climbable(
                &mut StandClimbing {
                    world: scene.world,
                    position: BlockPos::containing(position),
                },
                climbing,
            )
            .unwrap_or(false)
        };
        if !in_water && on_climbable(self.position) {
            self.enchantment_fall_distance = 0.0;
            // LivingEntity.handleOnClimbable; suppressing slide is Player-only.
            let limit = f64::from(0.15f32);
            wanted.x = wanted.x.clamp(-limit, limit);
            wanted.z = wanted.z.clamp(-limit, limit);
            if !wanted.y.is_nan() {
                wanted.y = wanted.y.max(-limit);
            }
        }
        let mut velocity = wanted;
        let mut horizontal_collision = false;
        // The stand's Entity.move goes through the move gate (#2414); PR 1a
        // records the movement unchanged.
        let mut gate = crate::move_gate::armed(
            context,
            self.id,
            "minecraft:armor_stand",
            sunrise_world::entity_move::MoverType::SelfMovement,
            wanted,
        );
        let wanted = gate.delta();
        // Entity.collide: the world border's walls and the entities that
        // `canBeCollidedWith` the stand stop it too.
        if let Ok(moved) = self.collide(&mut gate, scene, wanted, border, entities) {
            self.position = Vec3::new(
                self.position.x + moved.x,
                self.position.y + moved.y,
                self.position.z + moved.z,
            );
            self.ground.on_ground = wanted.y < 0.0 && moved.y != wanted.y;
            if (wanted.x - moved.x).abs() >= f64::from(1.0e-5_f32) {
                horizontal_collision = true;
                velocity.x = 0.0;
            }
            if moved.y != wanted.y {
                velocity.y = 0.0;
            }
            if (wanted.z - moved.z).abs() >= f64::from(1.0e-5_f32) {
                horizontal_collision = true;
                velocity.z = 0.0;
            }
            self.ground.horizontal_collision = horizontal_collision;
            // Entity.setOnGroundWithMovement uses the resolved move on a support miss.
            self.ground_support(scene, moved);
        }
        if !in_water && horizontal_collision && on_climbable(self.position) {
            velocity.y = 0.2;
        }
        self.velocity = velocity;
        Some(StandStep { falling, friction })
    }
    /// `Entity.move(MoverType.SHULKER, delta)`: an opening shulker lid
    /// shoves the stand (one with physics) by `delta`. Blocks, the world
    /// border and `entities` (the boxes `getEntityCollisions` gives the stand)
    /// cut the move short, a move cut to almost nothing is not made, and an
    /// axis that hits something loses its speed
    /// (`restituteMovementAfterCollisions`: a stand does not bounce, so the
    /// speed becomes `-speed * 0`).
    pub fn shove(
        &mut self,
        scene: SceneCollisions,
        delta: Vec3,
        border: Option<&BorderGeometry>,
        entities: &[Aabb],
        authoritative: bool,
        mover: sunrise_world::entity_move::MoverType,
        context: &sunrise_world::native_load_context::LoopContext,
    ) {
        // The lid's `Entity.move(mover, delta)` goes through the
        // move gate (#2414); PR 1a records the movement unchanged.
        let mut gate =
            crate::move_gate::armed(context, self.id, "minecraft:armor_stand", mover, delta);
        let delta = gate.delta();
        let Ok(moved) = self.collide(&mut gate, &scene, delta, border, entities) else {
            return;
        };
        let length = moved.length_squared();
        if length > 1.0e-7 || delta.length_squared() - length < 1.0e-7 {
            self.position = self.position.add_vector(moved);
        }
        // Mth.equal on x and z; y only when the push had a vertical part.
        let x_hit = sunrise_world::entity_move::different(delta.x, moved.x);
        let z_hit = sunrise_world::entity_move::different(delta.z, moved.z);
        self.ground.horizontal_collision = x_hit || z_hit;
        if x_hit {
            self.velocity.x = -self.velocity.x * 0.0;
        }
        if z_hit {
            self.velocity.z = -self.velocity.z * 0.0;
        }
        let vertical = moved.y != delta.y;
        if vertical && delta.y != 0.0 {
            self.velocity.y = -self.velocity.y * 0.0;
        }
        if authoritative || delta.y.abs() > 0.0 {
            self.ground.on_ground = vertical && delta.y < 0.0;
            self.ground_support(&scene, moved);
        }
    }
    /// `Entity.checkSupportingBlock` after a move by `movement`: the block
    /// under the feet while `on_ground`, none otherwise.
    fn ground_support(&mut self, scene: &SceneCollisions, movement: Vec3) {
        let mut ground = self.ground;
        let (context, subject) = self.support_collision();
        let mut scene = SceneCollisions {
            world: scene.world,
            binding: scene.binding,
            subject: Some(subject),
        };
        let _ = scene.update_ground_support(
            &mut ground,
            self.position,
            self.bounds(),
            Some(movement),
            context,
        );
        self.ground = ground;
    }
    /// `HoneyBlock.entityInside`: side-slide movement after the stand's travel.
    /// `ArmorStand.hasPhysics` / `Entity.isAffectedByBlocks` exclude markers
    /// and NoGravity stands. Slide effects are separate from this movement.
    pub fn apply_honey_slide(&mut self, world: &World, facts: &BlockFacts) {
        let old_delta_y = |dy: f64| dy / f64::from(0.98_f32) + 0.08;
        if self.marker
            || self.no_gravity
            || self.ground.on_ground
            || old_delta_y(self.velocity.y) >= -0.08
        {
            return;
        }
        // Entity.checkInsideBlocks: test the deflated body against block cells,
        // not their inset collision shape (the stand slides beside that shape).
        let bounds = self.bounds().deflate(Vec3::new(1e-5, 1e-5, 1e-5));
        let overlap = 0.4375 + if self.small { 0.125 } else { 0.25 };
        crate::block_contact::box_cells(bounds, |p| {
            let (x, y, z) = (p.x, p.y, p.z);
            if &*facts.get(world.block(x, y, z)).name != "minecraft:honey_block"
                || self.position.y > f64::from(y) + 0.9375 - 1.0e-7
            {
                return true;
            }
            let dx = (f64::from(x) + 0.5 - self.position.x).abs();
            let dz = (f64::from(z) + 0.5 - self.position.z).abs();
            if dx + 1.0e-7 <= overlap && dz + 1.0e-7 <= overlap {
                return true;
            }
            // HoneyBlock.doSlideMovement, including its float-widened
            // getOldDeltaY/getNewDeltaY conversion around air travel.
            let old = old_delta_y(self.velocity.y);
            if old < -0.13 {
                let factor = -0.05 / old;
                self.velocity.x *= factor;
                self.velocity.z *= factor;
            }
            self.velocity.y = (-0.05 - 0.08) * f64::from(0.98_f32);
            self.enchantment_fall_distance = 0.0;
            // The new oldDeltaY is -0.05, so another honey cell cannot slide again.
            false
        });
    }
    /// The rest of the stand's travel: the water slowdown, the lava slowdown
    /// and sink, or the air drag and gravity. The blocks' push (a bubble
    /// column) comes after this, as `LivingEntity.aiStep` applies the block
    /// effects after `travel`.
    pub fn tick_settle(&mut self, medium: StandMedium, step: StandStep) {
        use crate::entity_rules::EntityRules;
        let StandStep { falling, friction } = step;
        let velocity = self.velocity;
        let gravity = self.get_gravity();
        self.velocity = if let StandMedium::Lava { shallow } = medium {
            // travelInLava: halve it (a shallow lava also slows the fall by
            // 0.8 with `getFluidFallingAdjustedMovement`), then sink by
            // gravity / 4.
            let (x, y, z) = if shallow {
                let y = velocity.y * f64::from(0.8f32);
                let sink = gravity / 16.0;
                let adjusted = if falling && (y - 0.005).abs() >= 0.003 && (y - sink).abs() < 0.003
                {
                    -0.003
                } else {
                    y - sink
                };
                (velocity.x * 0.5, adjusted, velocity.z * 0.5)
            } else {
                (velocity.x * 0.5, velocity.y * 0.5, velocity.z * 0.5)
            };
            Vec3::new(x, y - gravity / 4.0, z)
        } else if medium == StandMedium::Water {
            // travelInWater: slowDown 0.8, then `getFluidFallingAdjustedMovement`
            // with gravity / 16 (the stand is falling while its fall is not
            // already nearly stopped).
            let slow = f64::from(0.8f32);
            let y = velocity.y * slow;
            let sink = self.get_gravity() / 16.0;
            let adjusted = if falling && (y - 0.005).abs() >= 0.003 && (y - sink).abs() < 0.003 {
                -0.003
            } else {
                y - sink
            };
            Vec3::new(velocity.x * slow, adjusted, velocity.z * slow)
        } else {
            let drag = f64::from(friction * 0.91f32);
            Vec3::new(
                velocity.x * drag,
                (velocity.y - self.get_gravity()) * f64::from(0.98f32),
                velocity.z * drag,
            )
        };
    }
}

#[cfg(test)]
mod experience_tests;

#[cfg(test)]
mod fire_environment_tests {
    use super::*;
    #[test]
    #[ignore = "daily"]
    fn ordinary_items_burn_but_native_netherite_resists_lava() {
        let Some(v) = crate::test_fixtures::vanilla() else {
            return;
        };
        if !v.root.join("items.json").exists() {
            return;
        }
        let (assets, facts) = (&v.assets, &v.facts);
        let defaults = sunrise_world::item_defaults::read(&v.root).unwrap();
        let damage = crate::health::HealthResources::load(&v.root).unwrap();
        let can_hurt = |stack: &ItemStack, name: &str| {
            let default = defaults
                .entries()
                .get(stack.item.as_ref())
                .and_then(|components| components.get("minecraft:damage_resistant"));
            crate::item_damage::stack_can_be_hurt_by(default, &damage.damage, stack, name).unwrap()
        };
        let mut world = World::test_scene_with_placements(assets, &[], 3).unwrap();
        let at = Vec3::new(20.5, 4.0, 20.5);
        world
            .set(
                20,
                4,
                20,
                assets.registry.default_state("minecraft:lava").unwrap(),
            )
            .unwrap();
        let make = |id: &str| ItemEntity {
            size_answer: None,
            random: sunrise_core::random::LegacyRandom::new(0),
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            id: 1,
            thrower: None,
            target: None,
            stack: ItemStack {
                item: std::sync::Arc::from(id),
                count: 1,
                damage: 0,
                extra: None,
            },
            health: 5,
            fire_ticks: 0,
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            ground: Default::default(),
            vertical_collision: false,
            age: 0,
            stack_revision: 0,
            lifespan: ItemEntity::LIFETIME,
            pickup_delay: 0,
            bob_offset: 0.0,
            ticks: 0,
            needs_sync: false,
            stuck: Vec3::ZERO,
            was_touching_water: false,
        };
        let mut protected = make("minecraft:oak_planks");
        let mut heard = Vec::new();
        let empty_items = Items::empty();
        let result = protected.environment_tick_with_hook(
            &mut world,
            facts,
            &empty_items,
            assets.registry,
            &can_hurt,
            |_, damage| {
                heard.push(damage.to_owned());
                damage == "minecraft:lava"
            },
        );
        assert_eq!(heard, ["minecraft:lava"]);
        assert!(result.alive && !result.lava_hurt);
        assert_eq!(protected.health, 5);
        assert!(
            protected.fire_ticks > 0,
            "immunity must not undo lava ignition"
        );
        let mut wood = make("minecraft:oak_planks");
        let mut metal = make("minecraft:netherite_ingot");
        let first = wood.environment_tick(&world, facts, &empty_items, assets.registry, &can_hurt);
        assert!(first.alive && first.lava_hurt);
        assert!(
            !first.should_play_lava_hurt_sound(1),
            "ordinary entry damage must not repeat the burn sound"
        );
        assert!(first.should_play_lava_hurt_sound(10));
        assert!(
            wood.fire_ticks > 0,
            "lava contact must ignite dropped items"
        );
        let last = wood.environment_tick(&world, facts, &empty_items, assets.registry, &can_hurt);
        assert!(
            !last.alive && last.lava_hurt,
            "lethal lava damage still emits burn sound"
        );
        assert!(
            last.should_play_lava_hurt_sound(2),
            "the lethal hit must always play the sound"
        );
        for _ in 0..40 {
            let (result, allocations) = sunrise_threads::allocation_probe::measure(|| {
                metal.environment_tick(&world, facts, &empty_items, assets.registry, &can_hurt)
            });
            assert_eq!(
                allocations.calls, 0,
                "immune environmental tick: {allocations:?}"
            );
            assert!(result.alive && !result.lava_hurt);
            assert!(!result.should_play_lava_hurt_sound(10));
        }
        assert_eq!(metal.health, 5);
        assert_eq!(metal.fire_ticks, 0, "fire-resistant items must not ignite");
        world
            .set(
                20,
                4,
                20,
                assets.registry.default_state("minecraft:fire").unwrap(),
            )
            .unwrap();
        let mut burning = make("minecraft:oak_planks");
        let result =
            burning.environment_tick(&world, facts, &empty_items, assets.registry, &can_hurt);
        assert!(result.alive && !result.lava_hurt);
        assert!(burning.fire_ticks > 0);
        world
            .set(
                20,
                4,
                20,
                assets
                    .registry
                    .default_state("minecraft:soul_fire")
                    .unwrap(),
            )
            .unwrap();
        let mut soul = make("minecraft:oak_planks");
        soul.environment_tick(&world, facts, &empty_items, assets.registry, &can_hurt);
        assert_eq!(soul.health, 3);
        let mut stand = ArmorStand::new(2, at, 0.0);
        world
            .set(
                20,
                4,
                20,
                assets.registry.default_state("minecraft:lava").unwrap(),
            )
            .unwrap();
        for _ in 0..150 {
            assert!(
                stand
                    .environment_tick(&world, facts, assets.registry)
                    .is_none()
            );
        }
        assert_eq!(stand.health, 20.0, "lava does not hurt armor stands");
        // A stand the flames run out of health names the damage that did it.
        world
            .set(
                20,
                4,
                20,
                assets.registry.default_state("minecraft:air").unwrap(),
            )
            .unwrap();
        stand.health = 4.0;
        stand.fire_ticks = 20;
        let mut heard = Vec::new();
        assert_eq!(
            stand.environment_tick_with_hook(&mut world, facts, assets.registry, |_, damage| {
                heard.push(damage.to_owned());
                damage == "minecraft:on_fire"
            }),
            None
        );
        assert_eq!(heard, ["minecraft:on_fire"]);
        assert_eq!(stand.health, 4.0);
        assert_eq!(stand.fire_ticks, 19);
        stand.fire_ticks = 20;
        assert_eq!(
            stand.environment_tick(&world, facts, assets.registry),
            Some("minecraft:on_fire")
        );
    }
}
#[cfg(test)]
mod destroyed_bundle_environment_tests {
    use super::*;

    /// `BundleItem.onDestroyed`: the lava that destroys a dropped bundle
    /// hands its stacks to the item tick (`ItemUtils.onContainerDestroyed`),
    /// and the bundle keeps none of them.
    #[test]
    #[ignore = "daily"]
    fn lava_that_destroys_a_bundle_spills_its_contents() {
        let Some(v) = crate::test_fixtures::vanilla() else {
            return;
        };
        if !v.root.join("items.json").exists() {
            return;
        }
        let (assets, facts) = (&v.assets, &v.facts);
        let defaults = sunrise_world::item_defaults::read(&v.root).unwrap();
        let damage = crate::health::HealthResources::load(&v.root).unwrap();
        let can_hurt = |stack: &ItemStack, name: &str| {
            let default = defaults
                .entries()
                .get(stack.item.as_ref())
                .and_then(|components| components.get("minecraft:damage_resistant"));
            crate::item_damage::stack_can_be_hurt_by(default, &damage.damage, stack, name).unwrap()
        };
        let mut world = World::test_scene_with_placements(assets, &[], 3).unwrap();
        let at = Vec3::new(20.5, 4.0, 20.5);
        world
            .set(
                20,
                4,
                20,
                assets.registry.default_state("minecraft:lava").unwrap(),
            )
            .unwrap();
        let mut stack = ItemStack::new("minecraft:bundle", 1);
        stack.data_mut().bundle_contents = vec![
            ItemStack::new("minecraft:stone", 3),
            ItemStack::new("minecraft:diamond", 1),
        ];
        let mut bundle = ItemEntity {
            size_answer: None,
            random: sunrise_core::random::LegacyRandom::new(0),
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            id: 1,
            thrower: None,
            target: None,
            stack,
            health: 5,
            fire_ticks: 0,
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            ground: Default::default(),
            vertical_collision: false,
            age: 0,
            stack_revision: 0,
            lifespan: ItemEntity::LIFETIME,
            pickup_delay: 0,
            bob_offset: 0.0,
            ticks: 0,
            needs_sync: false,
            stuck: Vec3::ZERO,
            was_touching_water: false,
        };
        // 5 health against 4 a tick: the first tick leaves 1 and nothing out.
        let empty_items = Items::empty();
        let first =
            bundle.environment_tick(&world, facts, &empty_items, assets.registry, &can_hurt);
        assert!(first.alive && first.spilled.is_empty());
        let second =
            bundle.environment_tick(&world, facts, &empty_items, assets.registry, &can_hurt);
        assert!(!second.alive);
        assert_eq!(
            second
                .spilled
                .iter()
                .map(|s| (&*s.item, s.count))
                .collect::<Vec<_>>(),
            [("minecraft:stone", 3), ("minecraft:diamond", 1)]
        );
        assert!(
            bundle
                .stack
                .data()
                .is_none_or(|d| d.bundle_contents.is_empty()),
            "the destroyed bundle keeps nothing inside"
        );
        assert_eq!(second.destroyed_by, Some("minecraft:lava"));
    }

    fn test_item_entity(health: i32, fire_ticks: i32, stack: ItemStack) -> ItemEntity {
        let at = Vec3::new(0.5, 64.5, 0.5);
        ItemEntity {
            size_answer: None,
            random: sunrise_core::random::LegacyRandom::new(1),
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            id: 1,
            thrower: None,
            target: None,
            stack,
            health,
            fire_ticks,
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            ground: Default::default(),
            vertical_collision: false,
            age: 0,
            stack_revision: 0,
            lifespan: ItemEntity::LIFETIME,
            pickup_delay: 0,
            bob_offset: 0.0,
            ticks: 0,
            needs_sync: false,
            stuck: Vec3::ZERO,
            was_touching_water: false,
        }
    }

    fn air_world_and_facts() -> (World, BlockFacts, sunrise_world::registry::BlockRegistry) {
        let registry = sunrise_world::registry::BlockRegistry::from_facts_json(
            br#"{"defaults":{"minecraft:air":0},"states":[["minecraft:air",false,false,"","minecraft:air"]]}"#
                .as_slice(),
        )
        .unwrap();
        let text = |s: &str| -> std::sync::Arc<str> { s.into() };
        let air = sunrise_items::blocks::StateFacts {
            name: text("minecraft:air"),
            emission: 0,
            dynamic_light: false,
            dampening: 0,
            skylight_down: true,
            shape_occlusion: false,
            can_occlude: false,
            solid_render: false,
            hardness: 0.0,
            requires_tool: false,
            replaceable: true,
            random_ticks: false,
            block_entity: false,
            render_shape: sunrise_items::blocks::RenderShape::Invisible,
            spawn_terrain_particles: true,
            destroy_overlay: false,
            full_collision: false,
            friction: 0.6,
            explosion_resistance: 0.0,
            item: text("minecraft:air"),
            pick_item: text("minecraft:air"),
            sounds: mod_api::BlockSoundType {
                identity: mod_api::BlockSoundTypeIdentity::Other,
                volume: 1.0,
                pitch: 1.0,
                break_sound: mod_api::SoundEventKey::parse("minecraft:intentionally_empty")
                    .unwrap(),
                step: mod_api::SoundEventKey::parse("minecraft:intentionally_empty").unwrap(),
                place: mod_api::SoundEventKey::parse("minecraft:intentionally_empty").unwrap(),
                hit: mod_api::SoundEventKey::parse("minecraft:intentionally_empty").unwrap(),
                fall: mod_api::SoundEventKey::parse("minecraft:intentionally_empty").unwrap(),
            },
            fluid: Fluid::None,
            fluid_level: 0,
            fluid_falling: false,
            waterlogged: false,
            random_offset: false,
            offset_limits: None,
            outline_offset: false,
            legacy_solid: false,
            sturdy_full: 0,
            classes: std::sync::Arc::from([text("AirBlock"), text("Block")]),
            outline: std::sync::Arc::from([]),
            outline_edges: std::sync::Arc::from([]),
            map_color: 0,
        };
        let air_id = registry.default_state("minecraft:air").unwrap();
        let facts = BlockFacts {
            states: vec![air],
            air: air_id,
            woods: Default::default(),
            sign_texts: Default::default(),
        };
        let world = World::new(facts.air, 1);
        (world, facts, registry)
    }

    #[test]
    fn environment_tick_records_on_fire_and_pre_killed_damage_types() {
        let mut stack = ItemStack::new("minecraft:bundle", 1);
        stack.data_mut().bundle_contents = vec![ItemStack::new("minecraft:diamond", 2)];
        let (world, facts, registry) = air_world_and_facts();
        let items = Items::empty();
        let can_hurt = |_: &ItemStack, _: &str| true;

        let mut burning = test_item_entity(1, 20, stack.clone());
        let result = burning.environment_tick(&world, &facts, &items, &registry, &can_hurt);
        assert!(!result.alive);
        assert_eq!(result.destroyed_by, Some("minecraft:on_fire"));
        assert_eq!(result.spilled.len(), 1);
        assert_eq!(&*result.spilled[0].item, "minecraft:diamond");

        let mut pre_killed = test_item_entity(0, 0, stack);
        let result = pre_killed.environment_tick(&world, &facts, &items, &registry, &can_hurt);
        assert!(!result.alive);
        assert_eq!(result.destroyed_by, Some("minecraft:generic"));
        assert_eq!(result.spilled.len(), 1);
        assert_eq!(result.spilled[0].count, 2);
    }

    #[test]
    fn environment_tick_deferring_spill_preserves_original_bundle_contents_for_hook() {
        let mut stack = ItemStack::new("minecraft:bundle", 1);
        stack.data_mut().bundle_contents = vec![ItemStack::new("minecraft:diamond", 3)];
        let (mut world, facts, registry) = air_world_and_facts();
        let items = Items::empty();
        let can_hurt = |_: &ItemStack, _: &str| true;

        let mut burning = test_item_entity(1, 20, stack);
        let result = burning.environment_tick_with_hook_deferring_spill(
            &mut world,
            &facts,
            &items,
            &registry,
            &can_hurt,
            |_, _| false,
        );
        assert!(!result.alive);
        assert_eq!(result.destroyed_by, Some("minecraft:on_fire"));
        assert!(result.spilled.is_empty());
        // Original stack contents stay intact until `item_destroyed` finishes.
        let intact = &burning.stack.data().unwrap().bundle_contents;
        assert_eq!(intact.len(), 1);
        assert_eq!(&*intact[0].item, "minecraft:diamond");
        assert_eq!(intact[0].count, 3);

        let peeked = burning.peek_destroyed_contents(&items);
        assert_eq!(peeked.len(), 1);
        assert_eq!(peeked[0].count, 3);
        assert_eq!(burning.stack.data().unwrap().bundle_contents.len(), 1);

        let spilled = burning.spill_destroyed_contents(&items);
        assert_eq!(spilled.len(), 1);
        assert_eq!(spilled[0].count, 3);
        assert!(
            burning
                .stack
                .data()
                .is_none_or(|d| d.bundle_contents.is_empty())
        );
    }
}
#[cfg(test)]
mod armor_stand_render_tests {
    use super::*;

    /// A falling stand is drawn between last tick's position and this one's,
    /// so it slides down every frame instead of jumping once a tick (#4416).
    #[test]
    fn a_falling_stand_is_drawn_between_its_ticks() {
        let mut stand = ArmorStand::new(1, Vec3::new(0.5, 70.0, 0.5), 0.0);
        // A new stand has no move behind it.
        assert_eq!(stand.render_position(0.5), stand.position);
        let fall = |s: &ArmorStand| Some((Vec3::new(0.5, s.position.y - 1.0, 0.5), 0.0, 0.0));
        stand.interpolate(fall(&stand));
        assert_eq!(stand.render_position(0.0).y, 70.0);
        assert_eq!(stand.render_position(0.25).y, 69.75);
        assert_eq!(stand.render_position(1.0).y, 69.0);
        stand.interpolate(fall(&stand));
        assert_eq!(stand.render_position(0.5).y, 68.5);
        // Landed: no step is due, so it stays put, and so does every frame.
        stand.interpolate(None);
        assert_eq!(stand.render_position(0.5).y, 68.0);
    }

    /// A turning stand's body turns between ticks too, the short way round.
    #[test]
    fn a_turning_stand_is_drawn_between_its_ticks() {
        let mut stand = ArmorStand::new(1, Vec3::ZERO, 170.0);
        assert_eq!(stand.render_yaw(0.5), 170.0);
        // To -170 is 20 degrees on through 180, not 340 back through 0.
        stand.interpolate(Some((Vec3::ZERO, -170.0, 0.0)));
        assert_eq!(stand.render_yaw(0.0), 170.0);
        assert_eq!(stand.render_yaw(0.5), 180.0);
        assert_eq!(stand.render_yaw(1.0), 190.0);
        stand.interpolate(None);
        assert_eq!(stand.render_yaw(0.5), 190.0);
    }

    /// A snap leaves no position or view samples to blend from. The body
    /// keeps its own turn between ticks.
    #[test]
    fn a_snapped_stand_is_drawn_where_it_landed() {
        let mut stand = ArmorStand::new(1, Vec3::new(0.5, 70.0, 0.5), 0.0);
        stand.interpolate(Some((Vec3::new(0.5, 69.0, 0.5), 20.0, 0.0)));
        stand.snap(Vec3::new(200.5, 64.0, 0.5), 90.0, 0.0);
        assert_eq!(stand.render_position(0.5), Vec3::new(200.5, 64.0, 0.5));
        assert_eq!([stand.previous_yaw, stand.yaw], [90.0; 2]);
        assert_eq!(stand.render_yaw(0.5), 10.0);
    }
}
#[cfg(test)]
mod armor_stand_light_tests {
    use super::*;

    fn stand(x: f64, y: f64, z: f64) -> ArmorStand {
        ArmorStand::new(1, Vec3::new(x, y, z), 0.0)
    }

    /// `ArmorStand.getLightProbePosition` for a stand that is not a marker is
    /// `Entity.getEyePosition`: `EntityType.ARMOR_STAND` declares
    /// `eyeHeight(1.7775)`, and a small stand (`isBaby()`) is the same type
    /// scaled by 0.5 with `withEyeHeight(0.9875)`. Sunrise read
    /// `floor(base + 0.5)` for every stand, so a stand standing on a lit floor
    /// took the floor's light instead of the light at its head.
    #[test]
    fn a_stand_is_lit_from_the_block_its_eye_is_in() {
        let dark = |_: i32, _: i32, _: i32| 0u8;
        // Full stand on y = 64: eye 65.7775, cell 65 (the old 64.5 was 64).
        assert_eq!(stand(0.5, 64.0, 0.5).light_probe(1.0, dark), [0, 65, 0]);
        assert_eq!((64.0_f64 + 0.5).floor() as i32, 64);
        // Small stand: eye 0.9875 above its base, so y = 64.2 is 65.1875.
        let mut small = stand(0.5, 64.2, 0.5);
        small.small = true;
        assert_eq!(small.light_probe(1.0, dark), [0, 65, 0]);
        assert_eq!((64.2_f64 + 0.5).floor() as i32, 64);
        // A small stand at y = 64.0 stays in its own cell: eye 64.9875.
        let mut low = stand(0.5, 64.0, 0.5);
        low.small = true;
        assert_eq!(low.light_probe(1.0, dark), [0, 64, 0]);
    }

    /// `getEyePosition(partialTick)` interpolates: a stand falling from 70 to
    /// 69 is probed 0.5 down at the middle of the frame, 69.5 + 1.7775 in the
    /// cell at 71, while its feet's cell is 69.
    #[test]
    fn a_stand_probe_follows_the_frame() {
        let dark = |_: i32, _: i32, _: i32| 0u8;
        let mut falling = stand(0.5, 70.0, 0.5);
        falling.interpolate(Some((Vec3::new(0.5, 69.0, 0.5), 0.0, 0.0)));
        assert_eq!(falling.light_probe(0.0, dark), [0, 71, 0]);
        assert_eq!(falling.light_probe(0.5, dark), [0, 71, 0]);
        assert_eq!(falling.light_probe(1.0, dark), [0, 70, 0]);
        assert_eq!(falling.position.y.floor() as i32, 69);
    }

    /// A marker walks `BlockPos.betweenClosed` of its own box - the size it has
    /// without the marker flag, 0.5 by 1.975 - from `position()`, taking the
    /// first cell at brightness 15 and else the brightest. Sunrise used the
    /// stand's own cell, so a marker (a hologram on a dark ceiling) was lit by
    /// its feet's block.
    #[test]
    fn a_marker_scans_the_cells_of_its_own_box() {
        let at = |want: [i32; 3], level: u8| {
            move |x: i32, y: i32, z: i32| {
                if [x, y, z] == want { level } else { 0 }
            }
        };
        let mut marker = stand(0.5, 64.0, 0.5);
        marker.marker = true;
        // The box is x/z 0.25..0.75 - the cell at 0 - and y 64..65.975.
        assert_eq!(marker.light_probe(1.0, at([0, 65, 0], 15)), [0, 65, 0]);
        assert_eq!(marker.light_probe(1.0, at([0, 65, 0], 9)), [0, 65, 0]);
        // Nothing lit: the first cell walked, x fastest then y then z.
        assert_eq!(marker.light_probe(1.0, |_, _, _| 0u8), [0, 64, 0]);
        // The cell above the box is not scanned, so its 15 does not win.
        assert_eq!(marker.light_probe(1.0, at([0, 66, 0], 15)), [0, 64, 0]);
        // 15 wins over a brighter-looking neighbour, and y is walked before z:
        // (0, 65, 0) comes before (0, 64, 1) though both are at 15.
        let mut wide = stand(0.5, 64.0, 0.9);
        wide.marker = true;
        let two = |x: i32, y: i32, z: i32| {
            if (x, y, z) == (0, 65, 0) || (x, y, z) == (0, 64, 1) {
                15
            } else {
                0
            }
        };
        assert_eq!(wide.light_probe(1.0, two), [0, 65, 0]);
        // `getLightProbePosition`'s marker branch reads `position()`, not the
        // partial-tick position: falling does not move the scan.
        let mut moving = stand(0.5, 64.0, 0.5);
        moving.marker = true;
        moving.interpolate(Some((Vec3::new(0.5, 70.0, 0.5), 0.0, 0.0)));
        assert_eq!(moving.light_probe(0.0, |_, _, _| 0u8), [0, 70, 0]);
        assert_eq!(moving.light_probe(0.5, |_, _, _| 0u8), [0, 70, 0]);
    }

    /// `EntityRenderer.getBlockLightLevel`: a stand `isOnFire` is drawn at 15
    /// block light whatever the level says, and the sky light is the probe
    /// cell's own.
    #[test]
    fn a_burning_stand_is_drawn_at_full_block_light() {
        let light = |sky: u8, block: u8| (sky << 4) | block;
        let cold = stand(0.5, 64.0, 0.5);
        assert_eq!(cold.render_light(light(15, 0)), (15 * 16) as u32);
        assert_eq!(cold.render_light(light(0, 9)), ((9 * 16) << 8) as u32);
        let burning = ArmorStand {
            fire_ticks: 100,
            ..stand(0.5, 64.0, 0.5)
        };
        assert_eq!(
            burning.render_light(light(15, 0)),
            ((15 * 16) | ((15 * 16) << 8)) as u32
        );
        assert_eq!(burning.render_light(light(0, 0)), ((15 * 16) << 8) as u32);
    }
}
#[cfg(test)]
mod merge_tests {
    use super::*;

    fn item(id: &str, count: u32, x: f64, age: i32) -> ItemEntity {
        let at = Vec3::new(x, 64.0, 0.5);
        ItemEntity {
            size_answer: None,
            random: sunrise_core::random::LegacyRandom::new(0),
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            id: 0,
            thrower: None,
            target: None,
            stack: ItemStack {
                item: std::sync::Arc::from(id),
                count,
                damage: 0,
                extra: None,
            },
            health: 5,
            fire_ticks: 0,
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            ground: sunrise_world::ground_support::GroundState::from_ground_flag(true),
            vertical_collision: false,
            age,
            stack_revision: 0,
            lifespan: ItemEntity::LIFETIME,
            pickup_delay: 0,
            bob_offset: 0.0,
            ticks: age.max(0) as u32,
            needs_sync: false,
            stuck: Vec3::ZERO,
            was_touching_water: false,
        }
    }
    fn max_stack(stack: &ItemStack) -> u32 {
        if &*stack.item == "minecraft:ender_pearl" {
            16
        } else {
            64
        }
    }
    /// Merges around `items[i]` as its tick would; returns the counts left.
    fn merge(items: &mut [ItemEntity], i: usize) -> Vec<Option<u32>> {
        let mut removed = vec![false; items.len()];
        let mut cells = ItemCells::new(items);
        merge_with_neighbours(items, &mut removed, &mut cells, i, true, &max_stack);
        items
            .iter()
            .zip(removed)
            .map(|(item, gone)| (!gone).then_some(item.stack.count))
            .collect()
    }

    #[test]
    fn merging_a_joined_batch_preserves_survivor_joins_and_order() {
        let mut items: crate::join_seq::JoinList<_> = [
            item("minecraft:sand", 3, 0.5, 41),
            item("minecraft:sand", 5, 1.2, 7),
            item("minecraft:gravel", 1, 8.0, 1),
        ]
        .into_iter()
        .collect();
        let joins: Vec<_> = items.iter().map(|item| item.join).collect();
        let mut removed = [false; 3];
        let mut cells = ItemCells::new(&items);
        merge_with_neighbours(&mut items, &mut removed, &mut cells, 0, true, &max_stack);
        let mut removed = removed.into_iter();
        items.retain(|_| !removed.next().unwrap());
        assert_eq!(items.len(), 2);
        assert_eq!((items[0].stack.count, items[0].age), (8, 7));
        assert_eq!(&*items[1].stack.item, "minecraft:gravel");
        assert_eq!(items[1].stack.count, 1);
        assert_eq!(
            items.iter().map(|item| item.join).collect::<Vec<_>>(),
            joins[1..]
        );
    }

    #[test]
    fn the_smaller_stack_goes_into_the_larger() {
        let mut items = [
            item("minecraft:sand", 3, 0.5, 41),
            item("minecraft:sand", 5, 1.2, 7),
        ];
        assert_eq!(merge(&mut items, 0), [None, Some(8)]);
        // ItemEntity.merge keeps the younger age; the ticking item's is
        // still the one before its tick.
        assert_eq!(items[1].age, 7);
        let mut items = [
            item("minecraft:sand", 5, 0.5, 41),
            item("minecraft:sand", 3, 1.2, 7),
        ];
        assert_eq!(merge(&mut items, 0), [Some(8), None]);
        assert_eq!(items[0].age, 8, "age 7, plus the rest of this tick");
    }

    /// ItemEntity.merge uses signed room, including zero and negative transfers.
    #[test]
    fn overstacked_items_transfer_back_without_losing_merge_metadata() {
        for (counts, expected) in [([80, 10], [64, 26]), ([64, 26], [64, 26])] {
            let mut items = [
                item("minecraft:stone", counts[0], 0.5, 41),
                item("minecraft:stone", counts[1], 0.6, 7),
            ];
            for item in &mut items {
                item.stack.data_mut().set_other(
                    "minecraft:max_stack_size",
                    Some(sunrise_core::nbt::Tag::Int(99)),
                );
            }
            let before = [items[0].stack.clone(), items[1].stack.clone()];
            items[1].pickup_delay = 30;
            let mut removed = [false; 2];
            let mut cells = ItemCells::new(&items);
            merge_with_neighbours(&mut items, &mut removed, &mut cells, 0, true, &|_| 99);
            assert_eq!([items[0].stack.count, items[1].stack.count], expected);
            assert_eq!(removed, [false, false]);
            assert_eq!(items[0].pickup_delay, 30);
            assert_eq!(items[0].age, 8, "younger age 7, then this tick ends");
            for (item, stack) in items.iter().zip(&before) {
                assert!(item.stack.same_item_same_components(stack));
            }
        }
    }

    #[test]
    fn items_merge_only_within_half_a_block_sideways() {
        // Boxes 0.25 wide grown by 0.5: centres closer than 0.75 meet.
        let mut items = [
            item("minecraft:sand", 1, 0.5, 1),
            item("minecraft:sand", 1, 1.24, 1),
        ];
        assert_eq!(merge(&mut items, 0), [None, Some(2)]);
        let mut items = [
            item("minecraft:sand", 1, 0.5, 1),
            item("minecraft:sand", 1, 1.26, 1),
        ];
        assert_eq!(merge(&mut items, 0), [Some(1), Some(1)]);
        let mut items = [
            item("minecraft:sand", 1, 0.5, 1),
            item("minecraft:sand", 1, 0.5, 1),
        ];
        items[1].position.y += 0.3;
        assert_eq!(
            merge(&mut items, 0),
            [Some(1), Some(1)],
            "no vertical growth"
        );
    }

    #[test]
    fn different_or_full_stacks_stay_apart() {
        let mut items = [
            item("minecraft:sand", 1, 0.5, 1),
            item("minecraft:gravel", 1, 0.5, 1),
        ];
        assert_eq!(merge(&mut items, 0), [Some(1), Some(1)]);
        let mut items = [
            item("minecraft:sand", 60, 0.5, 1),
            item("minecraft:sand", 10, 0.5, 1),
        ];
        assert_eq!(
            merge(&mut items, 0),
            [Some(60), Some(10)],
            "70 is over a stack"
        );
        let mut items = [
            item("minecraft:ender_pearl", 10, 0.5, 1),
            item("minecraft:ender_pearl", 6, 0.5, 1),
        ];
        assert_eq!(merge(&mut items, 0), [Some(16), None]);
        let mut items = [
            item("minecraft:ender_pearl", 16, 0.5, 1),
            item("minecraft:ender_pearl", 1, 0.5, 1),
        ];
        assert_eq!(
            merge(&mut items, 0),
            [Some(16), Some(1)],
            "a full stack doesn't merge"
        );
    }

    #[test]
    fn unmergeable_items_are_skipped() {
        let mut items = [
            item("minecraft:sand", 1, 0.5, 1),
            item("minecraft:sand", 1, 0.5, 1),
        ];
        items[1].pickup_delay = ItemEntity::INFINITE_PICKUP_DELAY;
        assert_eq!(merge(&mut items, 0), [Some(1), Some(1)]);
        // An empty item entity does not merge before its next tick discards it.
        let mut items = [
            item("minecraft:sand", 1, 0.5, 1),
            item("minecraft:air", 0, 0.5, 1),
        ];
        assert_eq!(merge(&mut items, 0), [Some(1), Some(0)]);
        // At age 6000 the other item's lifetime is over; this one's tick
        // started at 5999, so it may still merge into a live item.
        let mut items = [
            item("minecraft:sand", 1, 0.5, 6000),
            item("minecraft:sand", 1, 0.5, 6000),
        ];
        assert_eq!(merge(&mut items, 0), [Some(1), Some(1)]);
        items[1].age = 10;
        assert_eq!(merge(&mut items, 0), [None, Some(2)]);
        assert_eq!(items[1].age, 10);
    }

    #[test]
    fn a_stack_takes_its_neighbours_in_spawn_order_until_full() {
        let mut items = [
            item("minecraft:sand", 40, 0.5, 1),
            item("minecraft:sand", 20, 0.7, 1),
            item("minecraft:sand", 10, 0.3, 1),
            item("minecraft:sand", 3, 0.4, 1),
        ];
        // 40 + 20 = 60; 60 + 10 is over 64; then 3 more.
        assert_eq!(merge(&mut items, 0), [Some(63), None, Some(10), None]);
        let mut items = [
            item("minecraft:sand", 1, 0.5, 1),
            item("minecraft:sand", 1, 0.5, 1),
        ];
        items[1].pickup_delay = 30;
        assert_eq!(merge(&mut items, 0), [None, Some(2)]);
        assert_eq!(items[1].pickup_delay, 30, "the longer pickup delay stays");
    }

    #[test]
    fn the_cells_follow_moves() {
        let mut items = [
            item("minecraft:sand", 1, 0.5, 1),
            item("minecraft:sand", 1, 5.5, 1),
        ];
        let mut removed = [false; 2];
        let mut cells = ItemCells::new(&items);
        let from = items[1].position;
        items[1].position.x = 0.9;
        cells.moved(1, from, items[1].position);
        merge_with_neighbours(&mut items, &mut removed, &mut cells, 0, true, &max_stack);
        assert_eq!(removed, [true, false]);
        assert_eq!(items[1].stack.count, 2);
    }
}

#[cfg(test)]
mod world_border_tests {
    use super::*;

    /// `Entity.move`'s last step, `getBlockSpeedFactor`: a dropped item sliding
    /// over soul sand or honey keeps 40% of the horizontal speed it keeps over
    /// stone, in the same tick. Skips quietly without the vanilla data (CI has none).
    #[test]
    #[ignore = "daily"]
    fn a_dropped_item_on_soul_sand_or_honey_is_slowed_by_the_move() {
        let Some(v) = crate::test_fixtures::vanilla() else {
            return;
        };
        let (assets, facts) = (&v.assets, &v.facts);
        let fluids = crate::motion::FluidResources::new(assets, facts).unwrap();
        let (collision, motion) = (&v.collision, &v.motion);
        let mut after = Vec::new();
        // The tops of the blocks: stone 1, soul sand 7/8, honey 15/16.
        for (block, top) in [
            ("minecraft:stone", 4.0),
            ("minecraft:soul_sand", 3.875),
            ("minecraft:honey_block", 3.9375),
        ] {
            let mut world = World::test_scene_with_placements(assets, &[], 3).unwrap();
            let id = assets.registry.default_state(block).unwrap();
            for x in 18..=23 {
                for z in 19..=21 {
                    world.set(x, 3, z, id).unwrap();
                }
            }
            let mut item = super::honey_slide_tests::falling_item(19.5, top, 20.5, 0.2, 0.0, 0.0);
            item.ground.on_ground = true;
            item.tick(
                &world,
                assets.registry,
                facts,
                collision,
                false,
                &fluids,
                &[],
                None,
                Some(&motion.blocks),
            );
            after.push(item.velocity.x);
        }
        let (stone, soul, honey) = (after[0], after[1], after[2]);
        assert!(stone > 0.0);
        assert!(
            (soul - stone * f64::from(0.4_f32)).abs() < 1.0e-9,
            "soul sand {soul} vs stone {stone}"
        );
        assert!(
            (honey - stone * f64::from(0.4_f32)).abs() < 1.0e-9,
            "honey {honey} vs stone {stone}"
        );
    }

    /// `Entity.collectColliders` (#7080): the world border's wall stops an
    /// item and an experience orb as it stops the player. A border of size
    /// 10 around (20, 20) has its east wall at x = 25; each body starts at
    /// x = 24 on the flat ground of the test scene, moving 1.5 east.
    #[test]
    #[ignore = "daily"]
    fn the_world_border_stops_items_and_experience_orbs() {
        let Some(v) = crate::test_fixtures::vanilla() else {
            return;
        };
        let (assets, facts) = (&v.assets, &v.facts);
        let fluids = crate::motion::FluidResources::new(assets, facts).unwrap();
        let collision = &v.collision;
        let world = World::test_scene_with_placements(assets, &[], 3).unwrap();
        let border = BorderGeometry::new([20.0, 20.0], 10.0, 29_999_984).unwrap();
        for bordered in [false, true] {
            let border = bordered.then_some(&border);
            let mut item = super::honey_slide_tests::falling_item(24.0, 4.0, 20.5, 1.5, 0.0, 0.0);
            item.tick(
                &world,
                assets.registry,
                facts,
                collision,
                false,
                &fluids,
                &[],
                border,
                None,
            );
            let mut orb = ExperienceOrb {
                size_answer: None,
                attachment_lifetime: Default::default(),
                fall_distance: 0.0,
                rotation: [0.0, 0.0],
                join: Default::default(),
                no_gravity: false,
                id: 2,
                position: Vec3::new(24.0, 4.0, 20.5),
                previous: Vec3::new(24.0, 4.0, 20.5),
                velocity: Vec3::new(1.5, 0.0, 0.0),
                value: 1,
                age: 0,
                ground: sunrise_world::ground_support::GroundState::from_ground_flag(true),
                count: 1,
                ticks: 0,
                health: 5,
                air: 300,
                following: None,
                fire_ticks: 0,
            };
            orb.tick(
                &world,
                facts,
                collision,
                None,
                true,
                &mut || 0.0,
                border,
                &fluids,
                crate::test_resources::server_loop_context(),
                None,
                crate::motion::dry_side(),
                &mut |_, _| {},
            );
            for (what, east) in [
                ("item", item.bounds().max_position().x),
                ("orb", orb.bounds().max_position().x),
            ] {
                if bordered {
                    assert!(
                        (east - 25.0).abs() < 1.0e-7,
                        "the {what} stops on the wall at x = 25, got {east}"
                    );
                } else {
                    assert!(east > 25.0, "no border: the {what} goes on, got {east}");
                }
            }
        }
    }
}

#[cfg(test)]
mod honey_slide_tests {
    use super::*;

    /// Gravity still stacks on idle ticks; waking every 4th tick zeroes Y on
    /// the ground (`ItemEntity.tick` with `onGround`).
    #[test]
    #[ignore = "daily"]
    fn a_resting_item_skips_move_three_of_four_ticks() {
        let Some(v) = crate::test_fixtures::vanilla() else {
            return;
        };
        let (assets, facts) = (&v.assets, &v.facts);
        let fluids = crate::motion::FluidResources::new(assets, facts).unwrap();
        let collision = &v.collision;
        let mut world = World::test_scene_with_placements(assets, &[], 3).unwrap();
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        for x in 19..=21 {
            for z in 19..=21 {
                world.set(x, 3, z, stone).unwrap();
            }
        }
        let at = Vec3::new(20.5, 4.0, 20.5);
        let mut item = falling_item(at.x, at.y, at.z, 0.0, 0.0, 0.0);
        item.ground.on_ground = true;
        item.age = 0;
        item.ticks = 0;
        item.id = 0;
        let mut ys = Vec::new();
        for _ in 0..8 {
            item.tick(
                &world,
                assets.registry,
                facts,
                collision,
                false,
                &fluids,
                &[],
                None,
                None,
            );
            ys.push(item.velocity.y);
        }
        // Idle ticks keep gravity; the 4th tick's move hits the floor and
        // clears Y (vanilla's sequence for a resting item).
        assert_eq!(ys, vec![-0.04, -0.08, -0.12, 0.0, -0.04, -0.08, -0.12, 0.0]);
        assert_eq!(item.position.y, at.y);
    }

    /// #4558: idle ticks still run `BubbleColumnBlock.entityInside`, so lift
    /// builds and the next wake-up rises off the soul sand.
    #[test]
    #[ignore = "daily"]
    fn a_grounded_item_in_a_soul_sand_column_rises() {
        let Some(v) = crate::test_fixtures::vanilla() else {
            return;
        };
        let (assets, facts) = (&v.assets, &v.facts);
        let fluids = crate::motion::FluidResources::new(assets, facts).unwrap();
        let collision = &v.collision;
        let mut world = World::test_scene_with_placements(assets, &[], 3).unwrap();
        let soul = assets
            .registry
            .default_state("minecraft:soul_sand")
            .unwrap();
        let mut column = assets
            .registry
            .state(
                assets
                    .registry
                    .default_state("minecraft:bubble_column")
                    .unwrap(),
            )
            .unwrap()
            .clone();
        column.properties.insert("drag".into(), "false".into());
        let column = assets.registry.resolve(&column).unwrap();
        for x in 19..=21 {
            for z in 19..=21 {
                world.set(x, 3, z, soul).unwrap();
                for y in 4..=8 {
                    world.set(x, y, z, column).unwrap();
                }
            }
        }
        let at = Vec3::new(20.5, 4.0, 20.5);
        let mut item = falling_item(at.x, at.y, at.z, 0.0, 0.0, 0.0);
        item.ground.on_ground = true;
        item.age = 0;
        item.id = 1;
        let before = item.position.y;
        for _ in 0..16 {
            item.tick(
                &world,
                assets.registry,
                facts,
                collision,
                false,
                &fluids,
                &[],
                None,
                None,
            );
        }
        assert!(
            item.position.y > before + 0.5,
            "expected rise from soul-sand column, y={} (was {})",
            item.position.y,
            before
        );
    }

    /// `Entity.sendBubbleColumnParticles` is for the top of a column only: an item
    /// inside the column shows nothing, one in its surface block shows the block.
    #[test]
    #[ignore = "daily"]
    fn an_item_at_the_top_of_a_bubble_column_is_at_its_surface() {
        let Some(v) = crate::test_fixtures::vanilla() else {
            return;
        };
        let (assets, facts) = (&v.assets, &v.facts);
        let collision = &v.collision;
        let mut world = World::test_scene_with_placements(assets, &[], 3).unwrap();
        let mut column = assets
            .registry
            .state(
                assets
                    .registry
                    .default_state("minecraft:bubble_column")
                    .unwrap(),
            )
            .unwrap()
            .clone();
        column.properties.insert("drag".into(), "false".into());
        let column = assets.registry.resolve(&column).unwrap();
        for y in 4..=8 {
            world.set(20, y, 20, column).unwrap();
        }
        let surfaces = |y: f64| {
            let item = falling_item(20.5, y, 20.5, 0.0, 0.0, 0.0);
            crate::motion::bubble_column_surfaces(
                &world,
                assets.registry,
                facts,
                collision,
                item.bounds(),
            )
        };
        assert_eq!(surfaces(8.2), vec![BlockPos::new(20, 8, 20)]);
        assert_eq!(surfaces(5.0), Vec::new());
        // Straddling the two top blocks, it is still at the top of one column.
        assert_eq!(surfaces(7.9), vec![BlockPos::new(20, 8, 20)]);
        assert_eq!(surfaces(9.0), Vec::new());
    }

    pub(super) fn falling_item(x: f64, y: f64, z: f64, vx: f64, vy: f64, vz: f64) -> ItemEntity {
        let at = Vec3::new(x, y, z);
        ItemEntity {
            size_answer: None,
            random: sunrise_core::random::LegacyRandom::new(0),
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            id: 1,
            thrower: None,
            target: None,
            stack: ItemStack {
                item: std::sync::Arc::from("minecraft:stone"),
                count: 1,
                damage: 0,
                extra: None,
            },
            health: 5,
            fire_ticks: 0,
            position: at,
            previous: at,
            velocity: Vec3::new(vx, vy, vz),
            ground: Default::default(),
            vertical_collision: false,
            age: 10,
            stack_revision: 0,
            lifespan: ItemEntity::LIFETIME,
            pickup_delay: 0,
            bob_offset: 0.0,
            ticks: 10,
            needs_sync: false,
            stuck: Vec3::ZERO,
            was_touching_water: false,
        }
    }

    fn old_delta_y(dy: f64) -> f64 {
        dy / f64::from(0.98_f32) + 0.08
    }
    fn new_delta_y(dy: f64) -> f64 {
        (dy - 0.08) * f64::from(0.98_f32)
    }

    #[test]
    fn a_falling_item_beside_honey_slides_down() {
        // Item width 0.25: overlap 0.5625. Honey at (0, 10, 0); centre 0.5.
        // x = -0.0625 is against the west face (inset 1/16).
        let mut item = falling_item(-0.0625, 10.4, 0.5, 0.12, -0.20, 0.04);
        assert!(old_delta_y(-0.20) < -0.08);
        assert!(old_delta_y(-0.20) >= -0.13);
        item.honey_block_inside([0, 10, 0]);
        assert_eq!(item.velocity.x, 0.12);
        assert_eq!(item.velocity.z, 0.04);
        assert_eq!(item.velocity.y, new_delta_y(-0.05));
    }

    #[test]
    fn a_fast_fall_scales_the_horizontal_speed() {
        let mut item = falling_item(-0.0625, 10.4, 0.5, 0.2, -0.30, -0.1);
        let old = old_delta_y(-0.30);
        assert!(old < -0.13);
        item.honey_block_inside([0, 10, 0]);
        let factor = -0.05 / old;
        assert_eq!(item.velocity.x, 0.2 * factor);
        assert_eq!(item.velocity.z, -0.1 * factor);
        assert_eq!(item.velocity.y, new_delta_y(-0.05));
    }

    #[test]
    fn an_item_on_the_ground_or_on_top_does_not_slide() {
        let mut grounded = falling_item(-0.0625, 10.4, 0.5, 0.0, -0.20, 0.0);
        grounded.ground.on_ground = true;
        grounded.honey_block_inside([0, 10, 0]);
        assert_eq!(grounded.velocity.y, -0.20);

        let mut on_top = falling_item(0.5, 10.9375, 0.5, 0.0, -0.20, 0.0);
        on_top.honey_block_inside([0, 10, 0]);
        assert_eq!(on_top.velocity.y, -0.20, "standing on the honey");
    }

    #[test]
    fn a_centred_or_slow_item_does_not_slide() {
        let mut centred = falling_item(0.5, 10.4, 0.5, 0.0, -0.20, 0.0);
        centred.honey_block_inside([0, 10, 0]);
        assert_eq!(centred.velocity.y, -0.20, "inside, not on a side");

        let mut slow = falling_item(-0.0625, 10.4, 0.5, 0.0, -0.04, 0.0);
        assert!(old_delta_y(-0.04) >= -0.08);
        slow.honey_block_inside([0, 10, 0]);
        assert_eq!(slow.velocity.y, -0.04);
    }
}

#[cfg(test)]
mod slime_bounce_tests {
    use super::*;

    #[test]
    fn slime_step_on_drags_slow_vertical_speed_only() {
        let scale = slime_step_on_scale(0.0862).unwrap();
        assert!((scale - 0.41724).abs() < 1e-5);
        assert_eq!(slime_step_on_scale(0.1), None);
    }

    fn grounded_item() -> ItemEntity {
        let at = Vec3::new(0.5, 2.0, 0.5);
        ItemEntity {
            size_answer: None,
            random: sunrise_core::random::LegacyRandom::new(0),
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            id: 1,
            thrower: None,
            target: None,
            stack: ItemStack {
                item: std::sync::Arc::from("minecraft:stone"),
                count: 1,
                damage: 0,
                extra: None,
            },
            health: 5,
            fire_ticks: 0,
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            ground: sunrise_world::ground_support::GroundState::from_ground_flag(true),
            vertical_collision: false,
            age: 10,
            stack_revision: 0,
            lifespan: ItemEntity::LIFETIME,
            pickup_delay: 0,
            bob_offset: 0.0,
            ticks: 10,
            needs_sync: false,
            stuck: Vec3::ZERO,
            was_touching_water: false,
        }
    }

    #[test]
    fn a_piston_slime_bounce_launches_the_item_off_the_ground() {
        let mut item = grounded_item();
        item.piston_slime_bounce(Direction::Up);
        assert_eq!(item.velocity, Vec3::new(0.0, 1.0, 0.0));
        assert!(!item.ground.on_ground);
        assert!(item.needs_sync);
        item.needs_sync = false;
        item.piston_slime_bounce(Direction::East);
        assert_eq!(item.velocity, Vec3::new(1.0, 1.0, 0.0));
        assert!(!item.ground.on_ground);
        assert!(item.needs_sync);
    }

    #[test]
    fn item_air_drag_widens_get_air_drag() {
        // ItemEntity.tick: getAirDrag 0.98F, not the f64 0.98 (#6814).
        let mut item = super::honey_slide_tests::falling_item(0.5, 64.0, 0.5, 0.1, 0.2, -0.1);
        let world = World::new(sunrise_world::registry::BlockStateId(0), 0);
        let facts = BlockFacts {
            states: Vec::new(),
            air: sunrise_world::registry::BlockStateId(0),
            woods: Default::default(),
            sign_texts: Default::default(),
        };
        item.tick_settle(
            &world,
            crate::test_fixtures::air_registry(),
            &facts,
            ItemStep {
                old_velocity: item.velocity,
                restitution: None,
                vertical_contact: None,
                movement: Some(item.velocity),
                landed: None,
                prior_fall: item.fall_distance,
                fall_move: None,
            },
            crate::test_resources::server_loop_context(),
            crate::motion::dry_side(),
        );
        assert_eq!(item.velocity.x.to_bits(), 0x3fb9_1687_3333_3334);
        assert_eq!(item.velocity.y.to_bits(), 0x3fc9_1687_3333_3334);
        assert_eq!(item.velocity.z.to_bits(), 0xbfb9_1687_3333_3334);
    }

    #[test]
    fn item_fluid_drags_widen_float_constants() {
        // setUnderwaterMovement 0.99F / 5.0E-4F; setUnderLavaMovement 0.95F.
        let mut water = super::honey_slide_tests::falling_item(0.5, 64.0, 0.5, 0.1, 0.01, -0.2);
        water.set_fluid_movement(0.99_f32);
        assert_eq!(water.velocity.x.to_bits(), 0x3fb9_5810_6666_6667);
        assert_eq!(water.velocity.y.to_bits(), 0x3f85_8106_25ae_147b);
        assert_eq!(water.velocity.z.to_bits(), 0xbfc9_5810_6666_6667);
        let mut lava = super::honey_slide_tests::falling_item(0.5, 64.0, 0.5, 0.1, 0.01, -0.2);
        lava.set_fluid_movement(0.95_f32);
        assert_eq!(lava.velocity.x.to_bits(), 0x3fb8_51eb_8000_0000);
        assert_eq!(lava.velocity.y.to_bits(), 0x3f85_8106_25ae_147b);
        assert_eq!(lava.velocity.z.to_bits(), 0xbfc8_51eb_8000_0000);
    }

    #[test]
    #[ignore = "daily"]
    fn a_slime_bounce_marks_the_item_for_immediate_sync() {
        let Some(v) = crate::test_fixtures::vanilla() else {
            return;
        };
        let (assets, facts) = (&v.assets, &v.facts);
        let motion =
            crate::motion::MotionResources::load_with_facts(&v.root, assets, facts).unwrap();
        let collision = &v.collision;
        let mut world = World::test_scene_with_placements(assets, &[], 3).unwrap();
        let slime = assets
            .registry
            .default_state("minecraft:slime_block")
            .unwrap();
        for x in 19..=21 {
            for z in 19..=21 {
                world.set(x, 3, z, slime).unwrap();
            }
        }
        // ItemEntity.tick sets needsSync when getDeltaMovement().subtract(oldMovement).lengthSqr() > 0.01.
        let mut falling = grounded_item();
        falling.position = Vec3::new(20.5, 4.2, 20.5);
        falling.previous = falling.position;
        falling.velocity = Vec3::new(0.0, -0.5, 0.0);
        falling.ground.on_ground = false;
        falling.age = 0;
        falling.id = 1;
        falling.tick(
            &world,
            assets.registry,
            facts,
            collision,
            false,
            &motion.fluids,
            &[],
            None,
            Some(&motion.blocks),
        );
        assert_eq!(falling.position.y, 4.0);
        assert!(falling.velocity.y > 0.0);
        assert!(falling.ground.on_ground);
        assert!(falling.needs_sync);

        // An idle tick where velocity only steps by -0.04 (0.04^2 = 0.0016 <= 0.01)
        // does not set needs_sync.
        falling.velocity = Vec3::ZERO;
        falling.needs_sync = false;
        falling.tick(
            &world,
            assets.registry,
            facts,
            collision,
            false,
            &motion.fluids,
            &[],
            None,
            Some(&motion.blocks),
        );
        assert!(!falling.needs_sync);
    }
}

#[cfg(test)]
mod unstuck_tests {
    use super::*;

    const BORDER: BorderBounds = BorderBounds {
        min_x: -1.0e6,
        min_z: -1.0e6,
        max_x: 1.0e6,
        max_z: 1.0e6,
    };

    /// A level of full blocks at `solid`; every other cell is air.
    fn shapes(solid: &[(i32, i32, i32)]) -> impl FnMut(BlockPos) -> Option<VoxelShape> + '_ {
        move |p| {
            if solid.contains(&(p.x, p.y, p.z)) {
                VoxelShape::from_box(Aabb::block(p)).ok()
            } else {
                None
            }
        }
    }

    /// `level.noCollision(box)` over the same cells the search reads.
    fn free(solid: &[(i32, i32, i32)], b: Aabb) -> bool {
        !solid
            .iter()
            .any(|&(x, y, z)| Aabb::block(BlockPos::new(x, y, z)).intersects(b))
    }

    #[test]
    fn an_orb_spawned_in_a_block_moves_to_a_free_box() {
        // One block at y 0..1 and an orb standing in its upper half, as a mob
        // killed inside one leaves it: its box overlaps the block by 0.1.
        let solid = [(0, 0, 0)];
        let at = Vec3::new(0.5, 0.9, 0.5);
        let moved = ExperienceOrb::unstuck(at, &BORDER, shapes(&solid)).expect("the orb moves");
        // The nearest free box sits on the block's top face.
        assert_eq!(moved, Vec3::new(0.5, 1.0, 0.5));
        assert!(!ExperienceOrb::box_at(moved).intersects(Aabb::block(BlockPos::new(0, 0, 0))));
    }

    #[test]
    fn an_orb_in_open_air_keeps_its_position() {
        let solid = [(0, 0, 0)];
        let at = Vec3::new(0.5, 2.0, 0.5);
        assert!(free(&solid, ExperienceOrb::box_at(at)));
        // Nothing to move for: the spawn sites keep the position as given.
        assert_eq!(ExperienceOrb::unstuck(at, &BORDER, shapes(&solid)), None);
    }
}

#[cfg(test)]
mod entity_collision_tests {
    use super::honey_slide_tests::falling_item;
    use super::*;

    /// `Entity.collide` stops an item at the entities that `canBeCollidedWith` it: dropped
    /// above a boat's box (or a still happy ghast's) it lands on the box's top and rests,
    /// where with no box it falls to the floor of the scene.
    #[test]
    #[ignore = "daily"]
    fn an_item_lands_on_an_entity_box_under_it() {
        let Some(v) = crate::test_fixtures::vanilla() else {
            return;
        };
        let (assets, facts) = (&v.assets, &v.facts);
        let fluids = crate::motion::FluidResources::new(assets, facts).unwrap();
        let collision = &v.collision;
        let mut world = World::test_scene_with_placements(assets, &[], 3).unwrap();
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        world.set(20, 3, 20, stone).unwrap();
        let rest = |boxes: &[Aabb]| {
            let mut item = falling_item(20.5, 12.0, 20.5, 0.0, 0.0, 0.0);
            item.id = 1;
            for _ in 0..80 {
                item.tick(
                    &world,
                    assets.registry,
                    facts,
                    collision,
                    false,
                    &fluids,
                    boxes,
                    None,
                    None,
                );
            }
            item.position.y
        };
        // The stone's top is at 4.0; a box from 5.0 to 9.0 is in the way.
        let platform = Aabb::new(Vec3::new(19.0, 5.0, 19.0), Vec3::new(22.0, 9.0, 22.0));
        assert!((rest(&[]) - 4.0).abs() < 1.0e-6, "on the stone with no box");
        assert!(
            (rest(&[platform]) - 9.0).abs() < 1.0e-6,
            "on top of the box, at its top"
        );
    }

    /// `WebBlock.entityInside` / `PowderSnowBlock.entityInside` make an item stuck: a
    /// fast item sliding into a cobweb is stopped inside it, in powder snow it is slowed.
    #[test]
    #[ignore = "daily"]
    fn a_cobweb_and_powder_snow_slow_an_item() {
        let Some(v) = crate::test_fixtures::vanilla() else {
            return;
        };
        let (assets, facts) = (&v.assets, &v.facts);
        let fluids = crate::motion::FluidResources::new(assets, facts).unwrap();
        let collision = &v.collision;
        let mut world = World::test_scene_with_placements(assets, &[], 3).unwrap();
        let state = |n: &str| assets.registry.default_state(n).unwrap();
        for x in 16..32 {
            world.set(x, 3, 20, state("minecraft:stone")).unwrap();
        }
        let mut slide = |block: Option<&str>| {
            world
                .set(21, 4, 20, state(block.unwrap_or("minecraft:air")))
                .unwrap();
            let mut item = falling_item(20.5, 4.0, 20.5, 0.4, 0.0, 0.0);
            item.id = 1;
            for _ in 0..12 {
                item.tick(
                    &world,
                    assets.registry,
                    facts,
                    collision,
                    false,
                    &fluids,
                    &[],
                    None,
                    None,
                );
            }
            item.position.x
        };
        let free = slide(None);
        let web = slide(Some("minecraft:cobweb"));
        let snow = slide(Some("minecraft:powder_snow"));
        assert!(free > 21.4, "an unobstructed item slides on to {free}");
        assert!(
            web > 20.8 && web < free - 0.3,
            "the web holds it: {web} vs {free}"
        );
        assert!(snow < free, "powder snow slows it: {snow} vs {free}");
    }

    #[test]
    fn obstacle_grid_returns_only_the_boxes_near_an_item() {
        // A boat every 100 blocks along a line, 1000 of them.
        let boat = |x: f64| {
            Aabb::new(
                Vec3::new(x, 64.0, 0.0),
                Vec3::new(x + 1.375, 64.5625, 1.375),
            )
        };
        let mut grid = ObstacleGrid::new((0..1000).map(|i| boat(f64::from(i) * 100.0)).collect());
        let mut out = Vec::new();
        // Around the 7th boat: only it.
        let area = Aabb::new(Vec3::new(699.0, 63.0, -1.0), Vec3::new(701.0, 66.0, 1.0));
        grid.near(area, &mut out);
        assert_eq!(out, vec![boat(700.0)]);
        // Between two boats: none.
        let area = Aabb::new(Vec3::new(750.0, 63.0, 0.0), Vec3::new(751.0, 66.0, 1.0));
        grid.near(area, &mut out);
        assert!(out.is_empty());
        // A box spanning cells is found from any of them, once.
        let wide = Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(20.0, 1.0, 1.0));
        let mut grid = ObstacleGrid::new(vec![wide]);
        let area = Aabb::new(Vec3::new(17.0, 0.0, 0.0), Vec3::new(18.0, 1.0, 1.0));
        grid.near(area, &mut out);
        assert_eq!(out, vec![wide]);
    }

    #[test]
    fn obstacle_grid_leaves_out_a_movers_riders_and_answers_a_huge_area_at_once() {
        let unit = |x: f64| Aabb::new(Vec3::new(x, 64.0, 0.0), Vec3::new(x + 1.0, 65.0, 1.0));
        // The shulker at x = 2 rides stand 7; the one at x = 0 and the boat
        // at x = 4 ride nothing.
        let mut grid = ObstacleGrid::with_roots(
            vec![unit(0.0), unit(2.0), unit(4.0)],
            vec![None, Some(7), None],
        );
        let mut out = Vec::new();
        let area = Aabb::new(Vec3::new(-1.0, 63.0, -1.0), Vec3::new(6.0, 66.0, 2.0));
        grid.near_except(area, Some(7), &mut out);
        assert_eq!(out, vec![unit(0.0), unit(4.0)], "stand 7 ignores its rider");
        grid.near_except(area, Some(8), &mut out);
        assert_eq!(
            out,
            vec![unit(0.0), unit(2.0), unit(4.0)],
            "stand 8 does not"
        );
        // A stand loaded with Motion 1e10 asks for an area 1e10 blocks long
        // (about 1.25e9 cells): the boxes in it come back in order, at once.
        let huge = Aabb::new(Vec3::new(-1.0, 63.0, -1.0), Vec3::new(1.0e10, 66.0, 2.0));
        let start = std::time::Instant::now();
        grid.near(huge, &mut out);
        assert_eq!(out, vec![unit(0.0), unit(2.0), unit(4.0)]);
        let far = Aabb::new(
            Vec3::new(-1.0e300, 63.0, -1.0),
            Vec3::new(1.0e300, 66.0, 2.0),
        );
        grid.near(far, &mut out);
        assert_eq!(out.len(), 3);
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
    }

    /// A farm: 5000 drops and 1000 separated vehicles. Run with `-- --ignored --nocapture`.
    #[test]
    #[ignore = "timing"]
    fn obstacle_grid_beats_scanning_every_box_for_a_big_farm() {
        let boat = |i: u32| {
            let (x, z) = (f64::from(i % 40) * 6.0, f64::from(i / 40) * 6.0);
            Aabb::new(
                Vec3::new(x, 64.0, z),
                Vec3::new(x + 1.375, 64.5625, z + 1.375),
            )
        };
        let boxes: Vec<Aabb> = (0..1000).map(boat).collect();
        let items: Vec<Aabb> = (0..5000)
            .map(|i| {
                let (x, z) = (f64::from(i % 70) * 3.4, f64::from(i / 70) * 3.4);
                Aabb::new(Vec3::new(x, 64.0, z), Vec3::new(x + 0.25, 64.25, z + 0.25))
            })
            .collect();
        let start = std::time::Instant::now();
        let mut hits = 0;
        for item in &items {
            let area = item.inflate(Vec3::new(1.0, 1.0, 1.0));
            hits += boxes.iter().filter(|b| b.intersects(area)).count();
        }
        let scan = start.elapsed();
        let mut grid = ObstacleGrid::new(boxes);
        let start = std::time::Instant::now();
        let (mut out, mut found) = (Vec::new(), 0);
        for item in &items {
            grid.near(item.inflate(Vec3::new(1.0, 1.0, 1.0)), &mut out);
            found += out.len();
        }
        let binned = start.elapsed();
        assert_eq!(hits, found);
        println!("scan {scan:?}, grid {binned:?}");
    }
}

#[cfg(test)]
mod armor_stand_motion_tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn honey_slide_runs_after_stand_travel_and_preserves_no_physics_controls() {
        let Some(v) = crate::test_fixtures::vanilla() else {
            return;
        };
        let (assets, facts) = (&v.assets, &v.facts);
        let (collision, motion) = (&v.collision, &v.motion);
        let mut world = World::new(assets.registry.default_state("minecraft:air").unwrap(), 3);
        world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
        );
        for (z, name) in [(4, "minecraft:honey_block"), (10, "minecraft:stone")] {
            for y in 100..=120 {
                world
                    .set(6, y, z, assets.registry.default_state(name).unwrap())
                    .unwrap();
            }
        }
        // Independently captured Java values after five actual ticks.
        for (z, no_gravity, marker, y, vy, grounded) in [
            (
                4.5,
                false,
                false,
                117.51156798899842,
                -0.12740000247955321,
                false,
            ),
            (10.5, false, false, 118.0, -0.0784000015258789, true),
            (4.5, true, false, 118.0, 0.0, false),
            (4.5, false, true, 118.0, 0.0, false),
        ] {
            let mut stand = ArmorStand::new(1, Vec3::new(5.78, 118.0, z), 0.0);
            // Match the freshly summoned entity rather than the placement constructor.
            stand.ground.on_ground = false;
            stand.no_gravity = no_gravity;
            stand.marker = marker;
            let mut scene = SceneCollisions {
                world: &world,
                binding: collision,
                subject: None,
            };
            for _ in 0..5 {
                stand.tick(&mut scene, facts, &motion.climbing, false, None);
            }
            assert_eq!(
                stand.position,
                Vec3::new(5.78, y, z),
                "NoGravity={no_gravity}, Marker={marker}"
            );
            assert_eq!(stand.velocity, Vec3::new(0.0, vy, 0.0));
            assert_eq!(stand.ground.on_ground, grounded);
        }
    }

    #[test]
    fn a_name_tag_names_a_live_stand_and_unnamed_tags_pass() {
        let mut stand = ArmorStand::new(1, Vec3::ZERO, 0.0);
        let mut tag = ItemStack::new("minecraft:name_tag", 4);
        assert!(!stand.name_with_tag(&tag));
        assert!(stand.custom_name.is_none());
        let name = sunrise_core::nbt::Tag::compound([
            ("text", sunrise_core::nbt::Tag::String("Bob".into())),
            ("color", sunrise_core::nbt::Tag::String("gold".into())),
        ]);
        tag.data_mut().custom_name = Some(name.clone());
        assert!(stand.name_with_tag(&tag));
        assert_eq!(stand.custom_name, Some(crate::component::name_text(&name)));
        assert_eq!(stand.health, 20.0);
        assert!(!stand.name_with_tag(&ItemStack::new("minecraft:name_tag", 3)));
        stand.health = 0.0;
        tag.data_mut().custom_name = Some(sunrise_core::nbt::Tag::String("Other".into()));
        assert!(
            stand.name_with_tag(&tag),
            "a dead target still handles the named tag"
        );
        assert_eq!(stand.custom_name, Some(crate::component::name_text(&name)));
    }

    #[test]
    fn a_new_stand_starts_with_matching_previous() {
        let at = Vec3::new(1.0, 2.0, 3.0);
        let stand = ArmorStand::new(1, at, 90.0);
        assert_eq!(stand.previous, at);
        assert_eq!(stand.position, at);
    }

    fn tick_stand(
        stand: &mut ArmorStand,
        world: &World,
        facts: &BlockFacts,
        collision: &BlockCollisionRegistry<'_>,
    ) {
        let mut scene = SceneCollisions {
            world,
            binding: collision,
            subject: None,
        };
        let empty = Default::default();
        let climbing = sunrise_world::climbing::ClimbingRegistry::new(
            collision.registry(),
            &empty,
            &empty,
            &empty,
        )
        .unwrap();
        stand.tick(&mut scene, facts, &climbing, false, None);
    }

    #[test]
    #[ignore = "daily"]
    fn climbing_uses_actual_blocks_and_post_collision_position() {
        let Some(v) = crate::test_fixtures::vanilla() else {
            return;
        };
        let (assets, facts) = (&v.assets, &v.facts);
        let (collision, motion) = (&v.collision, &v.motion);
        let registry = assets.registry;
        let state = |name, properties: &[(&str, &str)]| {
            let mut state = registry
                .state(registry.default_state(name).unwrap())
                .unwrap()
                .clone();
            for (key, value) in properties {
                state.properties.insert((*key).into(), (*value).into());
            }
            registry.id(&state).unwrap()
        };
        let new_world = || {
            let mut world = World::new(registry.default_state("minecraft:air").unwrap(), 3);
            world.chunks.insert(
                (1, 1),
                std::sync::Arc::new(crate::chunk::Chunk::empty(1, 1)),
            );
            world
        };
        for (name, properties, ladder_below, climbable) in [
            ("minecraft:air", vec![], false, false),
            ("minecraft:ladder", vec![("facing", "west")], false, true),
            ("minecraft:vine", vec![("east", "true")], false, true),
            ("minecraft:twisting_vines_plant", vec![], false, true),
            ("minecraft:weeping_vines_plant", vec![], false, true),
            (
                "minecraft:oak_trapdoor",
                vec![("open", "true"), ("facing", "west")],
                true,
                true,
            ),
            (
                "minecraft:oak_trapdoor",
                vec![("open", "true"), ("facing", "east")],
                true,
                false,
            ),
        ] {
            let mut world = new_world();
            world.set(20, 10, 20, state(name, &properties)).unwrap();
            if ladder_below {
                world
                    .set(20, 9, 20, state("minecraft:ladder", &[("facing", "west")]))
                    .unwrap();
            }
            let mut stand = ArmorStand::new(1, Vec3::new(20.5, 10.0, 20.5), 0.0);
            stand.velocity.y = -0.5;
            let mut scene = SceneCollisions {
                world: &world,
                binding: collision,
                subject: None,
            };
            stand.tick(&mut scene, facts, &motion.climbing, false, None);
            let fall = if climbable { f64::from(0.15f32) } else { 0.5 };
            assert!(
                (stand.position.y - (10.0 - fall)).abs() < 1e-9,
                "{name} {properties:?}"
            );
        }
        // A wall collision inside a ladder lifts before gravity and drag.
        let mut world = new_world();
        world
            .set(20, 10, 20, state("minecraft:ladder", &[("facing", "west")]))
            .unwrap();
        for y in 10..13 {
            world.set(21, y, 20, state("minecraft:stone", &[])).unwrap();
        }
        let mut stand = ArmorStand::new(1, Vec3::new(20.74, 10.0, 20.5), 0.0);
        stand.velocity = Vec3::new(1.0, 0.0, 0.0);
        let mut scene = SceneCollisions {
            world: &world,
            binding: collision,
            subject: None,
        };
        stand.tick(&mut scene, facts, &motion.climbing, false, None);
        assert!((stand.velocity.y - (0.2 - 0.08) * f64::from(0.98f32)).abs() < 1e-9);
    }

    /// `Entity.collectColliders` (#7080): the world border's wall stops an
    /// armor stand as it stops the player. A border of size 10 around
    /// (20, 20) has its east wall at x = 25; the stand starts at x = 24 on the
    /// test scene's flat ground, moving 1.5 east.
    #[test]
    #[ignore = "daily"]
    fn the_world_border_stops_an_armor_stand() {
        let Some(v) = crate::test_fixtures::vanilla() else {
            return;
        };
        let (assets, facts) = (&v.assets, &v.facts);
        let collision = &v.collision;
        let world = World::test_scene_with_placements(assets, &[], 3).unwrap();
        let border = BorderGeometry::new([20.0, 20.0], 10.0, 29_999_984).unwrap();
        for bordered in [false, true] {
            let mut stand = ArmorStand::new(1, Vec3::new(24.0, 4.0, 20.5), 0.0);
            stand.ground.on_ground = true;
            stand.velocity = Vec3::new(1.5, 0.0, 0.0);
            let mut scene = SceneCollisions {
                world: &world,
                binding: collision,
                subject: None,
            };
            let empty = Default::default();
            let climbing = sunrise_world::climbing::ClimbingRegistry::new(
                collision.registry(),
                &empty,
                &empty,
                &empty,
            )
            .unwrap();
            stand.tick(
                &mut scene,
                facts,
                &climbing,
                false,
                bordered.then_some(&border),
            );
            let east = stand.bounds().max_position().x;
            if bordered {
                assert!(
                    (east - 25.0).abs() < 1.0e-7,
                    "stops on the wall, got {east}"
                );
            } else {
                assert!(east > 25.0, "no border: goes on, got {east}");
            }
        }
    }

    /// `Entity.collide` (#11877): the boxes `getEntityCollisions` gives a
    /// stand (a shulker's here, one block on the test scene's ground, whose
    /// top is at y = 4) stop it as blocks do. Dropped from y = 8 it rests on
    /// the box's top at y = 5; sliding east at 2 blocks a tick from x = 18 it
    /// stops with its east side on the box's west face, x = 20. With no boxes
    /// it falls to the ground and slides on.
    #[test]
    #[ignore = "daily"]
    fn a_stand_lands_on_and_stops_at_the_entity_boxes_it_is_given() {
        let Some(v) = crate::test_fixtures::vanilla() else {
            return;
        };
        let (assets, facts) = (&v.assets, &v.facts);
        let collision = &v.collision;
        let world = World::test_scene_with_placements(assets, &[], 3).unwrap();
        let empty = Default::default();
        let climbing = sunrise_world::climbing::ClimbingRegistry::new(
            collision.registry(),
            &empty,
            &empty,
            &empty,
        )
        .unwrap();
        let shulker = Aabb::new(Vec3::new(20.0, 4.0, 20.0), Vec3::new(21.0, 5.0, 21.0));
        let context = crate::test_resources::server_loop_context();
        let run = |stand: &mut ArmorStand, boxes: &[Aabb], ticks: usize| {
            for _ in 0..ticks {
                let scene = SceneCollisions {
                    world: &world,
                    binding: collision,
                    subject: None,
                };
                if let Some(step) = stand.tick_move(
                    &scene,
                    facts,
                    Some(&v.motion.blocks),
                    &climbing,
                    false,
                    None,
                    boxes,
                    context,
                    crate::motion::dry_side(),
                ) {
                    stand.tick_settle(StandMedium::Air, step);
                }
            }
        };
        for (boxes, floor) in [(&[shulker][..], 5.0), (&[][..], 4.0)] {
            let mut stand = ArmorStand::new(1, Vec3::new(20.5, 8.0, 20.5), 0.0);
            run(&mut stand, boxes, 20);
            let y = stand.position.y;
            assert!((y - floor).abs() < 1.0e-9, "rests at {floor}, got {y}");
            assert!(stand.ground.on_ground, "on the ground at {floor}");
        }
        for (boxes, blocked) in [(&[shulker][..], true), (&[][..], false)] {
            let mut stand = ArmorStand::new(1, Vec3::new(18.0, 4.0, 20.5), 0.0);
            stand.ground.on_ground = true;
            stand.velocity = Vec3::new(2.0, 0.0, 0.0);
            run(&mut stand, boxes, 1);
            let east = stand.bounds().max_position().x;
            if blocked {
                assert!((east - 20.0).abs() < 1.0e-7, "stops at the box, got {east}");
                assert_eq!(stand.velocity.x, 0.0, "the wall takes its speed");
            } else {
                assert!((east - 20.25).abs() < 1.0e-7, "goes on, got {east}");
            }
        }
    }

    #[test]
    fn a_stand_in_lava_halves_its_motion_and_sinks_by_a_quarter_of_gravity() {
        // LivingEntity.travelInLava, measured on the real server: a stand at
        // rest in deep lava has Motion.y -0.02, then -0.03, then -0.035.
        let step = StandStep {
            falling: true,
            friction: 1.0,
        };
        let mut stand = ArmorStand::new(1, Vec3::new(0.5, 100.0, 0.5), 0.0);
        let mut seen = Vec::new();
        for _ in 0..3 {
            stand.tick_settle(StandMedium::Lava { shallow: false }, step);
            seen.push(stand.velocity.y);
        }
        for (got, want) in seen.iter().zip([-0.02, -0.03, -0.035]) {
            assert!((got - want).abs() < 1e-12, "got {got}, want {want}");
        }
        // Shallow lava also applies the 0.8 and the gravity / 16 sink.
        let mut shallow = ArmorStand::new(1, Vec3::new(0.5, 100.0, 0.5), 0.0);
        shallow.velocity = Vec3::new(0.2, -0.1, 0.0);
        shallow.tick_settle(StandMedium::Lava { shallow: true }, step);
        assert!((shallow.velocity.x - 0.1).abs() < 1e-12);
        let want = -0.1 * f64::from(0.8f32) - 0.08 / 16.0 - 0.08 / 4.0;
        assert!((shallow.velocity.y - want).abs() < 1e-12);
        // Air is unchanged: gravity, then the 0.98 drag.
        let mut air = ArmorStand::new(1, Vec3::new(0.5, 100.0, 0.5), 0.0);
        air.tick_settle(StandMedium::Air, step);
        assert!((air.velocity.y + 0.0784).abs() < 1e-6);
    }

    #[test]
    fn lava_depth_picks_the_shallow_or_deep_travel() {
        assert_eq!(StandMedium::from_heights(0.0, 0.0), StandMedium::Air);
        assert_eq!(StandMedium::from_heights(0.3, 1.0), StandMedium::Water);
        assert_eq!(
            StandMedium::from_heights(0.0, 0.4),
            StandMedium::Lava { shallow: true }
        );
        assert_eq!(
            StandMedium::from_heights(0.0, 1.975),
            StandMedium::Lava { shallow: false }
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_tick_keeps_the_previous_position_for_interpolation() {
        let Some(v) = crate::test_fixtures::vanilla() else {
            return;
        };
        let (assets, collision, facts) = (&v.assets, &v.collision, &v.facts);
        let world = World::test_scene_with_placements(assets, &[], 3).unwrap();
        let start = Vec3::new(20.5, 20.0, 20.5);
        let mut stand = ArmorStand::new(1, start, 0.0);
        stand.ground.on_ground = false;
        // A summoned stand has no motion yet: the first tick only builds it
        // (vanilla moves by the velocity, then applies gravity), so it falls
        // from the second tick on (measured on the real server: drop 0, 0, 0.0784 blocks).
        tick_stand(&mut stand, &world, facts, collision);
        assert_eq!(stand.position, start);
        assert!((stand.velocity.y + 0.0784).abs() < 1e-6);
        tick_stand(&mut stand, &world, facts, collision);
        assert_eq!(stand.previous, start);
        assert!(
            stand.position.y < start.y,
            "an airborne stand must fall, y={}",
            stand.position.y
        );
        let mid = stand.previous.y + (stand.position.y - stand.previous.y) * 0.5;
        assert!(
            mid < start.y && mid > stand.position.y,
            "alpha 0.5 must sit between previous and current"
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_no_gravity_stand_stays_put() {
        let Some(v) = crate::test_fixtures::vanilla() else {
            return;
        };
        let (assets, collision, facts) = (&v.assets, &v.collision, &v.facts);
        let world = World::test_scene_with_placements(assets, &[], 3).unwrap();
        let start = Vec3::new(20.5, 20.0, 20.5);
        let mut stand = ArmorStand::new(1, start, 0.0);
        stand.ground.on_ground = false;
        stand.no_gravity = true;
        // ArmorStand.travel does nothing without physics: measured on the real
        // server, Pos and Motion stay exactly as summoned for every tick.
        for _ in 0..5 {
            tick_stand(&mut stand, &world, facts, collision);
            assert_eq!(stand.position, start);
            assert_eq!(stand.velocity, Vec3::ZERO);
        }
        stand.no_gravity = false;
        tick_stand(&mut stand, &world, facts, collision);
        tick_stand(&mut stand, &world, facts, collision);
        assert!(stand.position.y < start.y, "gravity again once cleared");
    }

    #[test]
    #[ignore = "daily"]
    fn leather_boots_walk_on_powder_snow() {
        let Some(v) = crate::test_fixtures::vanilla() else {
            return;
        };
        let (assets, collision, facts) = (&v.assets, &v.collision, &v.facts);
        let mut world = World::test_scene_with_placements(assets, &[], 3).unwrap();
        let snow = assets
            .registry
            .default_state("minecraft:powder_snow")
            .unwrap();
        for x in 19..=21 {
            for z in 19..=21 {
                world.set(x, 10, z, snow).unwrap();
            }
        }
        let start = Vec3::new(20.5, 11.0, 20.5);
        let mut bare = ArmorStand::new(1, start, 0.0);
        bare.ground.on_ground = true;
        for _ in 0..8 {
            tick_stand(&mut bare, &world, facts, collision);
        }
        assert!(
            bare.position.y < start.y - 0.2,
            "bare feet must sink into powder snow, y={}",
            bare.position.y
        );

        let mut booted = ArmorStand::new(2, start, 0.0);
        booted.ground.on_ground = true;
        booted.equipment[2] = Some(ItemStack::new("minecraft:leather_boots", 1));
        for _ in 0..8 {
            tick_stand(&mut booted, &world, facts, collision);
        }
        assert!(
            (booted.position.y - start.y).abs() < 1.0e-6,
            "leather boots must walk on powder snow, y={}",
            booted.position.y
        );
        assert_eq!(
            booted.ground.supporting_block,
            Some(BlockPos::new(20, 10, 20))
        );

        // Even without boots, an existing fall beyond 2.5 meets powder snow's
        // 0.9F-high falling collision shape on both collided move paths.
        for shoved in [false, true] {
            let mut falling = ArmorStand::new(3, Vec3::new(20.5, 11.125, 20.5), 0.0);
            let delta = Vec3::new(0.0, -0.5, 0.0);
            falling.enchantment_fall_distance = 3.0;
            if shoved {
                falling.shove(
                    SceneCollisions {
                        world: &world,
                        binding: collision,
                        subject: None,
                    },
                    delta,
                    None,
                    &[],
                    true,
                    sunrise_world::entity_move::MoverType::ShulkerBox,
                    crate::test_resources::server_loop_context(),
                );
            } else {
                falling.velocity = delta;
                tick_stand(&mut falling, &world, facts, collision);
            }
            assert_eq!(falling.position.y, 10.0 + f64::from(0.9_f32));
            assert!(falling.ground.on_ground);
            assert_eq!(
                falling.ground.supporting_block,
                Some(BlockPos::new(20, 10, 20))
            );
        }
    }
}

#[cfg(test)]
mod item_destroyed_tests {
    use super::*;

    fn item(id: &str, contents: &[(&str, u32)]) -> ItemEntity {
        let mut stack = ItemStack::new(id, 1);
        stack.data_mut().bundle_contents = contents
            .iter()
            .map(|(id, count)| ItemStack::new(id, *count))
            .collect();
        ItemEntity {
            size_answer: None,
            random: sunrise_core::random::LegacyRandom::new(0),
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            id: 1,
            thrower: None,
            target: None,
            stack,
            health: 5,
            fire_ticks: 0,
            position: Vec3::new(8.5, 64.0, 8.5),
            previous: Vec3::new(8.5, 64.0, 8.5),
            velocity: Vec3::ZERO,
            ground: Default::default(),
            vertical_collision: false,
            age: 0,
            stack_revision: 0,
            lifespan: ItemEntity::LIFETIME,
            pickup_delay: 0,
            bob_offset: 0.0,
            ticks: 0,
            needs_sync: false,
            stuck: Vec3::ZERO,
            was_touching_water: false,
        }
    }

    /// `BundleItem.onDestroyed` (`ItemStack.onDestroyed` → `dropContents`):
    /// the damage that empties an item's health (`ItemEntity.hurtServer`,
    /// which `environment_tick` calls for fire, lava and cactus) hands back
    /// the bundle's stacks, and the bundle keeps none of them.
    #[test]
    fn a_destroyed_bundle_spills_its_contents() {
        let mut bundle = item(
            "minecraft:bundle",
            &[("minecraft:stone", 3), ("minecraft:diamond", 1)],
        );
        let items = Items::empty();
        // Above zero after the hit: nothing leaves the bundle.
        assert_eq!(bundle.hurt(&items, 1), None);
        assert_eq!(bundle.health, 4);
        let spent = bundle.hurt(&items, 4).expect("the hit that destroys it");
        assert_eq!(
            spent
                .iter()
                .map(|s| (&*s.item, s.count))
                .collect::<Vec<_>>(),
            [("minecraft:stone", 3), ("minecraft:diamond", 1)]
        );
        assert!(
            bundle
                .stack
                .data()
                .is_none_or(|d| d.bundle_contents.is_empty()),
            "the destroyed bundle keeps nothing inside"
        );
        // A second hit has nothing left to spill, and an item that holds
        // nothing never reports any.
        assert_eq!(bundle.hurt(&items, 1), Some(Vec::new()));
        assert_eq!(
            item("minecraft:stone", &[]).hurt(&Items::empty(), 5),
            Some(Vec::new())
        );
    }

    /// `BlockItem.onDestroyed`: the same death path spills a container item's
    /// `container` — a shulker box's stacks — in slot order, and the box keeps
    /// no slot filled.
    #[test]
    fn a_destroyed_shulker_box_spills_its_container() {
        let mut box_item = item("minecraft:shulker_box", &[]);
        box_item.stack.data_mut().container = vec![
            (20, ItemStack::new("minecraft:diamond", 4)),
            (0, ItemStack::new("minecraft:oak_log", 8)),
            (3, ItemStack::new("minecraft:cobblestone", 64)),
        ];
        let spent = box_item
            .hurt(&Items::empty(), 5)
            .expect("the hit that destroys it");
        assert_eq!(
            spent
                .iter()
                .map(|s| (&*s.item, s.count))
                .collect::<Vec<_>>(),
            [
                ("minecraft:oak_log", 8),
                ("minecraft:cobblestone", 64),
                ("minecraft:diamond", 4),
            ]
        );
        assert!(
            box_item.stack.data().is_none_or(|d| d.container.is_empty()),
            "the destroyed box keeps no slot filled"
        );
    }
}

#[cfg(test)]
mod experience_cost_tests {
    use super::Experience;

    /// Java's `int` arithmetic wraps; a huge or very negative level must not
    /// panic where overflow checks are on.
    #[test]
    fn needed_wraps_for_levels_out_of_range() {
        assert_eq!(Experience::needed(0), 7);
        assert_eq!(Experience::needed(15), 37);
        assert_eq!(Experience::needed(30), 112);
        assert_eq!(
            Experience::needed(i32::MAX),
            (i32::MAX - 30).wrapping_mul(9).wrapping_add(112)
        );
        assert_eq!(
            Experience::needed(i32::MIN),
            i32::MIN.wrapping_mul(2).wrapping_add(7)
        );
    }
}
#[cfg(test)]
mod orb_underwater_tests {
    use super::*;

    #[test]
    fn matches_the_executed_java_private_method() {
        // Actual 26.3 setUnderwaterMovement calls on server-level orbs,
        // captured by test_orb_water_parity.py (orb-water-reference.json).
        // Cover both horizontal signs, falling/rising, and the vertical cap.
        let cases: [([f64; 3], [u64; 3]); 6] = [
            (
                [0.0, 0.0, 0.0],
                [0x0000000000000000, 0x3f40624de0000000, 0x0000000000000000],
            ),
            (
                [0.1, 0.01, -0.2],
                [0x3fb9581066666667, 0x3f85810625ae147b, 0xbfc9581066666667],
            ),
            (
                [-0.25, -0.03, 0.125],
                [0xbfcfae1480000000, 0xbf9e353f7c851eb8, 0x3fbfae1480000000],
            ),
            (
                [0.0, 0.059, 0.0],
                [0x0000000000000000, 0x3fae76c8b46d9168, 0x0000000000000000],
            ),
            (
                [0.5, 0.0595, -0.5],
                [0x3fdfae1480000000, 0x3faeb851e0000000, 0xbfdfae1480000000],
            ),
            (
                [-1.0, 0.2, 1.0],
                [0xbfefae1480000000, 0x3faeb851e0000000, 0x3fefae1480000000],
            ),
        ];
        let actual: Vec<_> = cases
            .iter()
            .map(|(v, _)| {
                let mut orb = ExperienceOrb {
                    size_answer: None,
                    attachment_lifetime: Default::default(),
                    fall_distance: 0.0,
                    rotation: [0.0, 0.0],
                    join: Default::default(),
                    no_gravity: false,
                    id: 1,
                    position: Vec3::ZERO,
                    previous: Vec3::ZERO,
                    velocity: Vec3::new(v[0], v[1], v[2]),
                    value: 1,
                    age: 0,
                    ground: Default::default(),
                    count: 1,
                    ticks: 0,
                    health: 5,
                    air: 300,
                    following: None,
                    fire_ticks: 0,
                };
                orb.set_underwater_movement();
                [
                    orb.velocity.x.to_bits(),
                    orb.velocity.y.to_bits(),
                    orb.velocity.z.to_bits(),
                ]
            })
            .collect();
        let expected: Vec<_> = cases.into_iter().map(|(_, bits)| bits).collect();
        assert_eq!(actual, expected);
    }
}

#[cfg(test)]
mod render_light_tests {
    use super::*;

    fn orb(y: f64) -> ExperienceOrb {
        ExperienceOrb {
            size_answer: None,
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            id: 1,
            position: Vec3::new(0.5, y, 0.5),
            previous: Vec3::new(0.5, y, 0.5),
            velocity: Vec3::new(0.0, 0.0, 0.0),
            value: 3,
            age: 0,
            ground: Default::default(),
            count: 1,
            ticks: 0,
            health: 5,
            air: 300,
            following: None,
            fire_ticks: 0,
        }
    }

    /// `ExperienceOrbRenderer.getBlockLightLevel`: the level's block light
    /// plus 7, clamped to 15; the sky light is the level's own. The orb used
    /// to be drawn at 10 in both nibbles, so an orb in a dark cave was washed
    /// out and one in daylight at block light 5 was too dim.
    #[test]
    fn an_orb_is_seven_brighter_than_the_block_it_sits_in() {
        let light = |sky: i32, block: i32| (sky << 4) | block;
        // Sky 15, block 0 (daylight): sky stays 15, block goes to 7.
        assert_eq!(
            orb(64.0).render_light(light(15, 0)),
            (15 * 16) | ((7 * 16) << 8)
        );
        // A dark cave: sky 0, block 0 -> block 7, and no invented sky light.
        assert_eq!(orb(20.0).render_light(light(0, 0)), (7 * 16) << 8);
        // Torchlight 9 -> 15, the clamp, not 16.
        assert_eq!(orb(20.0).render_light(light(0, 9)), (15 * 16) << 8);
        assert_eq!(
            orb(20.0).render_light(light(15, 15)),
            (15 * 16) | ((15 * 16) << 8)
        );
    }

    /// `Entity.getLightProbePosition` is the eye position: an orb's box is
    /// half a block, so it reads the light of the cell 0.425 above its feet.
    #[test]
    fn an_orb_is_lit_from_the_block_its_eye_is_in() {
        assert_eq!(orb(64.6).light_probe_y().floor(), 65.0);
        assert_eq!(orb(64.6).position.y.floor(), 64.0);
        assert!((orb(64.0).light_probe_y() - 64.425).abs() < 1e-9);
    }

    /// `ExperienceOrbRenderer.submit`'s quad: a block tall about the orb's
    /// centre, scaled by 0.3, so it spans -0.075 to +0.225 above the 0.1 the
    /// pose is translated by. Sunrise drew a symmetric 0.3 square, a quarter
    /// of the sprite too low.
    #[test]
    fn the_orb_sprite_hangs_a_quarter_below_its_centre() {
        let off = ORB_QUAD.map(|[x, y]| [x * ORB_QUAD_SCALE, 0.1 + y * ORB_QUAD_SCALE]);
        let want = [
            [-0.15, 0.1 - 0.075],
            [0.15, 0.1 - 0.075],
            [0.15, 0.1 + 0.225],
            [-0.15, 0.1 + 0.225],
        ];
        for (got, want) in off.into_iter().zip(want) {
            assert!((got[0] - want[0]).abs() < 1e-6, "{got:?} vs {want:?}");
            assert!((got[1] - want[1]).abs() < 1e-6, "{got:?} vs {want:?}");
        }
    }
}

#[cfg(test)]
mod dropped_item_light_tests {
    use super::*;

    fn item(previous_y: f64, y: f64) -> ItemEntity {
        ItemEntity {
            size_answer: None,
            random: sunrise_core::random::LegacyRandom::new(0),
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            id: 1,
            thrower: None,
            target: None,
            stack: ItemStack {
                item: std::sync::Arc::from("minecraft:stone"),
                count: 1,
                damage: 0,
                extra: None,
            },
            health: 5,
            fire_ticks: 0,
            position: Vec3::new(0.5, y, 0.5),
            previous: Vec3::new(0.5, previous_y, 0.5),
            velocity: Vec3::ZERO,
            ground: Default::default(),
            vertical_collision: false,
            age: 0,
            stack_revision: 0,
            lifespan: ItemEntity::LIFETIME,
            pickup_delay: 0,
            bob_offset: 0.0,
            ticks: 0,
            needs_sync: false,
            stuck: Vec3::ZERO,
            was_touching_water: false,
        }
    }

    /// `Entity.getLightProbePosition` reads the block at the item's eye, so
    /// a falling item just below y = 65 reads the cell at 65, not the floor.
    #[test]
    fn a_dropped_item_is_lit_from_the_block_its_eye_is_in() {
        let eye_height = f64::from(0.2125_f32);
        let falling = item(64.85, 64.85);
        let probe = falling.light_probe(1.0);
        assert_eq!(probe[1], 64.85 + eye_height);
        assert_eq!(probe[1].floor(), 65.0);
        let resting = item(64.0, 64.0);
        let probe = resting.light_probe(1.0);
        assert_eq!(probe[1], 64.0 + eye_height);
        assert_eq!(probe[1].floor(), 64.0);
    }

    /// `Entity.getLightProbePosition(partialTick)` follows `getPosition(partialTick)`:
    /// an item falling a block in one tick is probed halfway between the two
    /// positions in the middle of the frame. The old code read the tick's own
    /// position, so the light jumped a frame early.
    #[test]
    fn an_item_probe_follows_the_frame() {
        let falling = item(64.0, 65.0);
        let mid = falling.light_probe(0.5);
        assert_eq!(mid[1], 64.5 + f64::from(0.2125_f32));
        assert_eq!(mid[1].floor(), 64.0);
        assert_eq!(falling.light_probe(1.0)[1].floor(), 65.0);
        // Across x and z too, not just down.
        let sliding = ItemEntity {
            random: sunrise_core::random::LegacyRandom::new(0),
            position: Vec3::new(1.5, 64.0, 0.5),
            previous: Vec3::new(0.5, 64.0, 0.5),
            ..item(64.0, 64.0)
        };
        assert_eq!(sliding.light_probe(0.5)[0], 1.0);
    }

    /// `EntityRenderer.getBlockLightLevel` is `isOnFire() ? 15 : the level's
    /// block light`, and the sky light is never overridden. A dropped item was
    /// drawn at the level's block light while it burned.
    #[test]
    fn a_burning_item_is_drawn_at_full_block_light() {
        let light = |sky: u8, block: u8| (sky << 4) | block;
        let cold = item(64.0, 64.0);
        assert_eq!(cold.render_light(light(15, 0)), (15 * 16) as u32);
        assert_eq!(cold.render_light(light(0, 9)), ((9 * 16) << 8) as u32);
        let burning = ItemEntity {
            random: sunrise_core::random::LegacyRandom::new(0),
            fire_ticks: 160,
            ..item(64.0, 64.0)
        };
        // Daylight, dark block: the block light goes to 15, the sky stays.
        assert_eq!(
            burning.render_light(light(15, 0)),
            ((15 * 16) | ((15 * 16) << 8)) as u32
        );
        // A dark cave: 15 block light, still no invented sky light.
        assert_eq!(burning.render_light(light(0, 0)), ((15 * 16) << 8) as u32);
    }
}
crate::impl_joined!(
    ItemEntity,
    ExperienceOrb,
    ArmorStand,
    Painting,
    Cushion,
    ItemFrame
);
sunrise_mod_runtime::impl_attachment_lifetime!(
    ItemEntity,
    ExperienceOrb,
    ArmorStand,
    Painting,
    Cushion,
    ItemFrame
);

#[cfg(test)]
#[path = "entities/attachment_lifetime_tests.rs"]
mod attachment_lifetime_tests;

#[cfg(test)]
mod orb_lava_tests {
    use super::*;

    fn with_fixture(
        mut f: impl FnMut(
            &mut World,
            &BlockFacts,
            &BlockCollisionRegistry<'_>,
            &crate::motion::FluidResources,
            &crate::assets::Assets,
        ),
    ) {
        let Some(v) = crate::test_fixtures::vanilla() else {
            eprintln!("orb lava fixture unavailable: prepared resources missing");
            return;
        };
        let (assets, facts) = (&v.assets, &v.facts);
        let fluids = crate::motion::FluidResources::new(assets, facts).unwrap();
        let collision = &v.collision;
        let mut world = World::test_scene_with_placements(assets, &[], 3).unwrap();
        f(&mut world, facts, collision, &fluids, assets);
    }

    fn sample_orb(at: Vec3, health: i32) -> ExperienceOrb {
        ExperienceOrb {
            size_answer: None,
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            id: 1,
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            value: 1,
            age: 0,
            ground: Default::default(),
            count: 1,
            ticks: 0,
            health,
            air: 300,
            following: None,
            fire_ticks: 0,
        }
    }

    /// `Entity.baseTick`: a server orb with `remainingFireTicks > 0` is hurt for
    /// 1 on every tick where the counter is a multiple of 20, and the counter
    /// drops by 1 each tick. `Fire: 100` meets that at 100, 80, 60, 40 and 20,
    /// so the five health are gone on the 81st tick, with 19 fire ticks left.
    /// A client copy only clears the fire; a fire-free orb stays at 5 health,
    /// and a saved `Fire:-5s` is back to 0 after one server tick.
    #[test]
    #[ignore = "daily"]
    fn a_burning_server_orb_is_hurt_every_twenty_ticks_and_burns_out() {
        with_fixture(|world, facts, collision, fluids, _| {
            let at = Vec3::new(20.5, 6.0, 20.5);
            let tick = |orb: &mut ExperienceOrb, server: bool| {
                orb.tick(
                    world,
                    facts,
                    collision,
                    None,
                    server,
                    &mut || 0.0,
                    None,
                    fluids,
                    crate::test_resources::server_loop_context(),
                    None,
                    crate::motion::dry_side(),
                    &mut |_, _| {},
                )
            };
            let mut orb = sample_orb(at, 5);
            orb.fire_ticks = 100;
            let mut health = Vec::new();
            for n in 1..=81 {
                let stays = tick(&mut orb, true);
                if n == 1 || n == 21 || n == 41 || n == 61 {
                    health.push((n, orb.fire_ticks, orb.health, stays));
                }
                assert_eq!(stays, n < 81, "tick {n}");
            }
            assert_eq!(
                health,
                [
                    (1, 99, 4, true),
                    (21, 79, 3, true),
                    (41, 59, 2, true),
                    (61, 39, 1, true)
                ]
            );
            assert_eq!((orb.health, orb.fire_ticks), (0, 19));
            let mut client = sample_orb(at, 5);
            client.fire_ticks = 100;
            assert!(tick(&mut client, false));
            assert_eq!((client.health, client.fire_ticks), (5, 0));
            let mut cool = sample_orb(at, 5);
            cool.fire_ticks = -5;
            for _ in 0..40 {
                assert!(tick(&mut cool, true));
            }
            assert_eq!((cool.health, cool.fire_ticks), (5, 0));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_server_orb_takes_lava_damage_after_moving_and_discards_at_zero() {
        with_fixture(|world, facts, collision, fluids, assets| {
            let lava = assets.registry.default_state("minecraft:lava").unwrap();
            world.set(20, 4, 20, lava).unwrap();
            let at = Vec3::new(20.5, 4.1, 20.5);
            let mut orb = sample_orb(at, 5);
            let mut client = orb.clone();
            for expected_age in 1..=2 {
                assert!(client.tick(
                    world,
                    facts,
                    collision,
                    None,
                    false,
                    &mut || 0.0,
                    None,
                    fluids,
                    crate::test_resources::server_loop_context(),
                    None,
                    crate::motion::dry_side(),
                    &mut |_, _| {}
                ));
                assert_eq!((client.health, client.age), (5, expected_age));
            }
            let mut sounds = Vec::new();
            assert!(orb.tick(
                world,
                facts,
                collision,
                None,
                true,
                &mut || 0.0,
                None,
                fluids,
                crate::test_resources::server_loop_context(),
                None,
                crate::motion::dry_side(),
                &mut |pos, pitch| sounds.push((pos, pitch)),
            ));
            assert_eq!((orb.health, orb.age), (1, 1));
            assert_eq!(sounds.len(), 1);
            sounds.clear();
            assert!(!orb.tick(
                world,
                facts,
                collision,
                None,
                true,
                &mut || 0.0,
                None,
                fluids,
                crate::test_resources::server_loop_context(),
                None,
                crate::motion::dry_side(),
                &mut |pos, pitch| sounds.push((pos, pitch)),
            ));
            assert_eq!((orb.health, orb.age), (-3, 2));
            assert_eq!(sounds.len(), 1);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn an_invulnerable_orb_keeps_its_health_but_still_emits_lava_sound() {
        with_fixture(|world, facts, collision, fluids, assets| {
            let lava = assets.registry.default_state("minecraft:lava").unwrap();
            world.set(20, 4, 20, lava).unwrap();
            let mut orb = sample_orb(Vec3::new(20.5, 4.1, 20.5), 5);
            let mut sounds = 0;
            let mut calls = 0;
            let context = crate::test_resources::server_loop_context();
            let side = crate::motion::dry_side();
            let motion = crate::motion::BodyMotion {
                world,
                collision,
                facts,
                blocks: None,
                context,
                side: Some(side),
            };
            let tick = orb.move_before_fall(motion, None, true, &mut || 0.0, None, fluids, &[]);
            orb.velocity = orb.move_restitution(motion, tick).velocity;
            assert!(orb.finish_tick(
                &mut &mut *world,
                (collision, facts, context, side),
                fluids,
                tick,
                true,
                OrbLavaEffects {
                    next_float: &mut || 0.0,
                    on_sound: &mut |_, _| sounds += 1,
                    invulnerable: &mut |_| {
                        calls += 1;
                        true
                    },
                },
            ));
            assert_eq!(orb.health, 5);
            assert_eq!(calls, 1);
            assert_eq!(sounds, 1);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn legacy_orb_in_air_or_water_or_non_lava_does_not_play_sound_or_consume_random() {
        with_fixture(|world, facts, collision, fluids, assets| {
            // Air scenario
            {
                let mut orb_air = sample_orb(Vec3::new(20.5, 6.0, 20.5), 5);
                let mut draws = 0;
                let mut next_float = || {
                    draws += 1;
                    0.5
                };
                let mut sounds = Vec::new();
                assert!(orb_air.tick(
                    world,
                    facts,
                    collision,
                    None,
                    true,
                    &mut next_float,
                    None,
                    fluids,
                    crate::test_resources::server_loop_context(),
                    None,
                    crate::motion::dry_side(),
                    &mut |pos, pitch| sounds.push((pos, pitch)),
                ));
                assert!(sounds.is_empty(), "orb in air must not play burn sound");
                assert_eq!(
                    draws, 0,
                    "orb in air must not consume random float for lava"
                );
                assert_eq!(orb_air.health, 5);
            }

            // Water scenario
            {
                let water = assets.registry.default_state("minecraft:water").unwrap();
                world.set(20, 4, 20, water).unwrap();
                let mut orb_water = sample_orb(Vec3::new(20.5, 4.1, 20.5), 5);
                let mut draws = 0;
                let mut next_float = || {
                    draws += 1;
                    0.5
                };
                let mut sounds = Vec::new();
                assert!(orb_water.tick(
                    world,
                    facts,
                    collision,
                    None,
                    true,
                    &mut next_float,
                    None,
                    fluids,
                    crate::test_resources::server_loop_context(),
                    None,
                    crate::motion::dry_side(),
                    &mut |pos, pitch| sounds.push((pos, pitch)),
                ));
                assert!(sounds.is_empty(), "orb in water must not play burn sound");
                assert_eq!(
                    draws, 0,
                    "orb in water must not consume random float for lava"
                );
                assert_eq!(orb_water.health, 5);
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn negative_case_non_contact_or_client_orb_does_not_play_burn_sound() {
        with_fixture(|world, facts, collision, fluids, assets| {
            let lava = assets.registry.default_state("minecraft:lava").unwrap();
            world.set(20, 4, 20, lava).unwrap();

            // Distant orb (no contact)
            let mut orb_away = sample_orb(Vec3::new(22.5, 4.1, 22.5), 5);
            let mut draws = 0;
            let mut next_float = || {
                draws += 1;
                0.5
            };
            let mut sounds = Vec::new();
            assert!(orb_away.tick(
                world,
                facts,
                collision,
                None,
                true,
                &mut next_float,
                None,
                fluids,
                crate::test_resources::server_loop_context(),
                None,
                crate::motion::dry_side(),
                &mut |pos, pitch| sounds.push((pos, pitch)),
            ));
            assert!(
                sounds.is_empty(),
                "non-contact orb must not play burn sound"
            );
            assert_eq!(draws, 0, "non-contact orb must not consume random float");
            assert_eq!(orb_away.health, 5);

            // Client orb in lava (server = false)
            let mut client_orb = sample_orb(Vec3::new(20.5, 4.1, 20.5), 5);
            sounds.clear();
            assert!(client_orb.tick(
                world,
                facts,
                collision,
                None,
                false,
                &mut || 0.5,
                None,
                fluids,
                crate::test_resources::server_loop_context(),
                None,
                crate::motion::dry_side(),
                &mut |pos, pitch| sounds.push((pos, pitch)),
            ));
            assert!(
                sounds.is_empty(),
                "client orb in lava must not play burn sound"
            );
            assert_eq!(client_orb.health, 5);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn nonpositive_health_orbs_age_in_air_and_take_damage_in_lava() {
        with_fixture(|world, facts, collision, fluids, assets| {
            let lava = assets.registry.default_state("minecraft:lava").unwrap();
            world.set(20, 4, 20, lava).unwrap();
            for health in [0, -3] {
                for (y, fire_ticks, ticks, damage, burn_sounds) in
                    [(6.0, 0, 20, 0, 0), (4.1, 0, 1, 4, 1), (4.1, 20, 1, 1, 0)]
                {
                    let mut orb = sample_orb(Vec3::new(20.5, y, 20.5), health);
                    orb.no_gravity = true;
                    orb.fire_ticks = fire_ticks;
                    let mut sounds = 0;
                    for _ in 0..ticks {
                        assert_eq!(
                            orb.tick(
                                world,
                                facts,
                                collision,
                                None,
                                true,
                                &mut || 0.5,
                                None,
                                fluids,
                                crate::test_resources::server_loop_context(),
                                None,
                                crate::motion::dry_side(),
                                &mut |_, _| sounds += 1,
                            ),
                            damage == 0,
                            "Health {health}, Y {y}, Fire {fire_ticks}"
                        );
                    }
                    assert_eq!(orb.age, ticks);
                    assert_eq!(orb.health, health - damage);
                    assert_eq!(sounds, burn_sounds);
                }
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn positive_case_server_orb_in_lava_plays_generic_burn_sound() {
        with_fixture(|world, facts, collision, fluids, assets| {
            let lava = assets.registry.default_state("minecraft:lava").unwrap();
            world.set(20, 4, 20, lava).unwrap();

            let at = Vec3::new(20.5, 4.1, 20.5);
            let mut orb = sample_orb(at, 5);
            let mut sounds = Vec::new();
            let float_val = 0.35_f32;
            let stays = orb.tick(
                world,
                facts,
                collision,
                None,
                true,
                &mut || float_val,
                None,
                fluids,
                crate::test_resources::server_loop_context(),
                None,
                crate::motion::dry_side(),
                &mut |pos, pitch| sounds.push((pos, pitch)),
            );
            assert!(stays, "orb survives first contact with 1 HP");
            assert_eq!(orb.health, 1);
            // Entity.lavaIgnite sets it alight for 300 ticks (#2689).
            assert_eq!(orb.fire_ticks, 300);
            assert_eq!(sounds.len(), 1, "exactly one burn sound played");
            let expected_pitch = 2.0_f32 + float_val * 0.4_f32;
            assert!((sounds[0].1 - expected_pitch).abs() < 1e-6);
            assert_eq!(
                sounds[0].0,
                [orb.position.x, orb.position.y, orb.position.z]
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn boundary_case_surviving_and_discarding_and_multi_contact_in_one_tick() {
        with_fixture(|world, facts, collision, fluids, assets| {
            let lava = assets.registry.default_state("minecraft:lava").unwrap();
            world.set(20, 4, 20, lava).unwrap();

            // 1. Surviving at 1 HP vs discarding at <= 0 HP
            let mut orb = sample_orb(Vec3::new(20.5, 4.1, 20.5), 5);
            let mut sounds = Vec::new();

            // Tick 1: 5 HP -> 1 HP (survives)
            assert!(orb.tick(
                world,
                facts,
                collision,
                None,
                true,
                &mut || 0.1,
                None,
                fluids,
                crate::test_resources::server_loop_context(),
                None,
                crate::motion::dry_side(),
                &mut |pos, pitch| sounds.push((pos, pitch)),
            ));
            assert_eq!(orb.health, 1);
            assert_eq!(sounds.len(), 1);
            assert!((sounds[0].1 - (2.0 + 0.1 * 0.4)).abs() < 1e-6);

            // Tick 2: 1 HP -> -3 HP (discarded)
            sounds.clear();
            assert!(!orb.tick(
                world,
                facts,
                collision,
                None,
                true,
                &mut || 0.9,
                None,
                fluids,
                crate::test_resources::server_loop_context(),
                None,
                crate::motion::dry_side(),
                &mut |pos, pitch| sounds.push((pos, pitch)),
            ));
            assert_eq!(orb.health, -3);
            assert_eq!(sounds.len(), 1);
            assert!((sounds[0].1 - (2.0 + 0.9 * 0.4)).abs() < 1e-6);

            // 2. Multiple lava contacts in a single tick
            world.set(21, 4, 20, lava).unwrap();
            let mut multi_orb = sample_orb(Vec3::new(21.5, 4.1, 20.5), 5);
            multi_orb.previous = Vec3::new(19.8, 4.1, 20.5);
            let wanted = Vec3::new(1.7, 0.0, 0.0);
            let mut float_draws = vec![0.2_f32, 0.8_f32];
            let mut multi_sounds = Vec::new();
            let discarded = multi_orb.apply_lava_contact_damage(
                world,
                fluids,
                wanted,
                &mut || float_draws.remove(0),
                &mut |pos, pitch| multi_sounds.push((pos, pitch)),
            );
            assert!(discarded, "orb is discarded after two contacts");
            assert_eq!(multi_orb.health, -3);
            assert_eq!(multi_sounds.len(), 2, "both contacts triggered burn sound");
            assert!((multi_sounds[0].1 - (2.0 + 0.2 * 0.4)).abs() < 1e-6);
            assert!((multi_sounds[1].1 - (2.0 + 0.8 * 0.4)).abs() < 1e-6);
        });
    }
}

/// `FluidState.getHeight` of the fluid of `family` at `p` (a full block under
/// more of the same fluid, else its own height); None where there is none.
pub fn fluid_height(
    world: &World,
    fluids: &crate::motion::FluidResources,
    p: BlockPos,
    family: sunrise_world::fluid::FluidKind,
) -> Option<f64> {
    let fluid = fluids.fluid(world.block(p.x, p.y, p.z));
    if !fluid.kind().same_family(family) {
        return None;
    }
    let above = fluids.fluid(world.block(p.x, p.y + 1, p.z));
    Some(if fluid.kind().same_family(above.kind()) {
        1.0
    } else {
        f64::from(fluid.own_height())
    })
}

/// `Entity.checkInsideBlocks` and `collidedWithFluid`: the cells whose fluid box
/// (`height`'s, None where there is none) the entity's box (`bounds_at`) touches
/// over its move, each visited once.
/// `wanted` is `Entity.move`'s uncollided movement, whose axis order the path
/// takes; None for a move set by position (`setPos`), swept in one segment.
pub fn fluid_contacts(
    from: Vec3,
    to: Vec3,
    wanted: Option<Vec3>,
    bounds_at: impl Fn(Vec3) -> Aabb,
    mut height: impl FnMut(BlockPos) -> Option<f64>,
) -> usize {
    let delta = to.subtract(from);
    let mut visited = std::collections::HashSet::new();
    let mut contacts = 0;
    let mut remaining = 16;
    let mut segment = |start: Vec3, end: Vec3, budget: i32| {
        let mut iterations = 0;
        let epsilon = f64::from(1.0e-5_f32);
        let target = bounds_at(end).deflate(Vec3::new(epsilon, epsilon, epsilon));
        let _ = sunrise_core::geometry::sweep_intersected_blocks(
            start,
            end,
            target,
            1_000_000,
            |p, step| {
                if step >= budget {
                    return false;
                }
                iterations = step;
                if visited.contains(&p) {
                    return true;
                }
                if let Some(height) = height(p) {
                    let min = Vec3::new(f64::from(p.x), f64::from(p.y), f64::from(p.z));
                    let fluid = Aabb::new(min, min.add_vector(Vec3::new(1.0, height, 1.0)));
                    if bounds_at(start).collided_along_vector(end.subtract(start), &[fluid]) {
                        visited.insert(p);
                        contacts += 1;
                    }
                }
                true
            },
        );
        iterations + 1
    };
    if let Some(wanted) = wanted.filter(|_| delta.length_squared() > 0.0) {
        // Direction.axisStepOrder uses the original, uncollided movement.
        let order = if wanted.x.abs() < wanted.z.abs() {
            [1, 2, 0]
        } else {
            [1, 0, 2]
        };
        let components = delta.components();
        let mut start = from;
        for axis in order {
            if components[axis] != 0.0 {
                let mut end = start.components();
                end[axis] += components[axis];
                let end = Vec3::new(end[0], end[1], end[2]);
                remaining -= segment(start, end, remaining);
                start = end;
            }
        }
    } else {
        remaining -= segment(from, to, remaining);
    }
    if remaining <= 0 {
        segment(to, to, 1);
    }
    contacts
}

#[cfg(test)]
mod orb_lava_shape_tests {
    use super::*;

    fn contacts(from: Vec3, to: Vec3) -> usize {
        fluid_contacts(
            from,
            to,
            Some(to.subtract(from)),
            ExperienceOrb::box_at,
            |p| (p == BlockPos::new(0, 0, 0)).then_some(f64::from(8.0_f32 / 9.0)),
        )
    }

    #[test]
    fn lava_uses_the_swept_box_and_fluid_height_not_the_feet_block() {
        let side = Vec3::new(-0.2, 0.1, 0.5);
        assert_eq!(contacts(side, side), 1);
        let above = Vec3::new(0.5, 0.95, 0.5);
        assert_eq!(contacts(above, above), 0);
        let through = Vec3::new(-1.0, 0.1, 0.5);
        let end = Vec3::new(2.0, 0.1, 0.5);
        assert_eq!(contacts(through, end), 1);
        let away = Vec3::new(-1.0, 0.1, -1.0);
        assert_eq!(contacts(away, away), 0);
    }

    #[test]
    fn axis_segments_do_not_damage_twice_for_the_same_lava_cell() {
        let from = Vec3::new(0.1, 0.1, 0.1);
        let to = Vec3::new(0.7, 0.2, 0.7);
        assert_eq!(contacts(from, to), 1);
        assert_eq!(
            fluid_contacts(
                from,
                to,
                Some(Vec3::new(0.1, 0.1, 0.9)),
                ExperienceOrb::box_at,
                |p| { (p == BlockPos::new(0, 0, 0)).then_some(1.0) }
            ),
            1
        );
    }
}
