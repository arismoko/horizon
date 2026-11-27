//! Firework rockets (`FireworkRocketItem`, `FireworkRocketEntity`): launching
//! from a block face, boosting a gliding player, the 1.15 horizontal
//! acceleration and 0.04 lift, the flight-duration lifetime, explosion
//! damage with line of sight, and the client explosion (`createFireworks`).
use super::view_vector;
use std::sync::Arc;
use sunrise_core::{
    geometry::{Aabb, Vec3},
    random::RandomSource,
};
use {
    sunrise_items::firework::FireworkExplosion, sunrise_items::items::ItemStack,
    sunrise_level::collisions::SceneCollisions, sunrise_sim::entity_nbt::RocketNbt,
    sunrise_sim::mobs::Who,
};

pub const FIREWORK_EXPLODE_EVENT: i8 = 17;

#[path = "fireworks/impact.rs"]
pub(in crate::game) mod impact;

#[derive(Clone)]
pub struct Rocket {
    /// Entity-owned registration metadata; content clones are detached.
    pub attachment_lifetime: sunrise_mod_runtime::attachments::AttachmentLifetime,
    /// `Entity.yRot` and `xRot` (`/tp`, `/data get ... Rotation`).
    pub rotation: [f32; 2],
    /// `Entity.yRotO` and `xRotO`: the rotation when its tick began (`setOldPosAndRot`), which
    /// `Projectile.updateRotation` eases from and a deflection turns with the rotation.
    pub previous_rotation: [f32; 2],
    /// When it joined its list (`sunrise_client::join_seq`); set by the list.
    pub join: sunrise_sim::join_seq::JoinSeq,
    /// Its entity id: from the entity counter on the server, the server's on
    /// a client.
    pub id: u32,
    pub position: Vec3,
    pub previous: Vec3,
    pub velocity: Vec3,
    pub life: i32,
    pub lifetime: i32,
    pub explosions: Arc<Vec<FireworkExplosion>>,
    /// Boosting the player (attachedToEntity).
    pub attached: bool,
    /// On a server, the guest (by connection) who launched it, or whom it
    /// boosts: `owner`'s `Who::Player` then means them, not the host.
    pub guest: Option<u32>,
    /// `Projectile.getOwner`: the player who launched it, credited with
    /// what its blast hurts.
    pub owner: Option<Who>,
    /// `Projectile.getOwner` when `owner` can't name it: the UUID of an owner
    /// that is neither a player nor a mob.
    pub owner_uuid: Option<[i32; 4]>,
    /// `Projectile.leftOwner`: until then it can't hit its owner.
    pub left_owner: bool,
    /// `Projectile.lastDeflectedBy`: prevents repeated deflection by the same mob.
    pub last_deflected_by: Option<u32>,
    /// `DATA_ATTACHED_TO_TARGET`: the server entity the rocket follows;
    /// boosting happens when it is the local player.
    pub attached_to: Option<u32>,
    /// `DATA_SHOT_AT_ANGLE` (dispensers, crossbows): constant velocity.
    pub angled: bool,
    /// `DATA_ID_FIREWORKS_ITEM`: the stack it was launched from.
    pub item: Arc<ItemStack>,
    /// `Entity.random`: what a mod's `Projectile.deflect` draws from.
    pub random: sunrise_core::random::LegacyRandom,
    /// `Entity.onGround`: whether its last move was stopped going down (or
    /// the one it was loaded with, until it moves).
    pub ground: sunrise_world::ground_support::GroundState,
    /// `Projectile.hasBeenShot`: its first tick posts `projectile_shoot`.
    pub has_been_shot: bool,
    /// `Entity.isSilent`: no launch sound.
    pub silent: bool,
    /// `Entity.fallDistance`.
    pub fall_distance: f64,
    /// `Entity.stuckSpeedMultiplier`: a cobweb or powder snow it went through
    /// slows its next move.
    pub stuck: Vec3,
    /// `Entity.wasTouchingWater`: entering water splashes.
    pub was_touching_water: bool,
    /// `Entity.firstTick`: no splash on the tick it appears in water.
    pub first_tick: bool,
    /// What `entity_size` answered at its last size refresh (`EntityEvent.Size`): its box takes
    /// the answer while it holds for its vanilla size.
    pub size_answer: Option<sunrise_sim::pose::SizeAnswer>,
}
impl Rocket {
    /// Position and motion (the dispenser parity fixture).
    pub fn motion(&self) -> (Vec3, Vec3) {
        (self.position, self.velocity)
    }
    /// `Projectile.updateRotation`: a fifth of the way from the old rotation (yaw,
    /// pitch) toward its motion's direction, the short way round.
    pub fn update_rotation(&mut self) {
        let v = self.velocity;
        let horizontal = (v.x * v.x + v.z * v.z).sqrt();
        let atan2 = sunrise_core::geometry::trig::atan2_degrees;
        let old = self.previous_rotation;
        self.rotation = [
            sunrise_sim::entities::arrows::lerp_rotation(old[0], atan2(v.x, v.z)),
            sunrise_sim::entities::arrows::lerp_rotation(old[1], atan2(v.y, horizontal)),
        ];
    }

    /// A rocket a server sent (`handleAddEntity`): placed and moved by it.
    pub fn remote(id: u32, position: Vec3, velocity: Vec3) -> Self {
        let item = ItemStack::new("minecraft:firework_rocket", 1);
        let (_, explosions) = fireworks_of(&item);
        Self {
            size_answer: None,
            attachment_lifetime: Default::default(),
            rotation: [0.0, 0.0],
            previous_rotation: [0.0, 0.0],
            join: Default::default(),
            id,
            position,
            previous: position,
            velocity,
            life: 0,
            lifetime: i32::MAX,
            explosions: explosions.into(),
            attached: false,
            guest: None,
            owner: None,
            owner_uuid: None,
            left_owner: false,
            last_deflected_by: None,
            attached_to: None,
            angled: false,
            item: item.into(),
            random: sunrise_sim::projectiles::unique_random(position),
            ground: Default::default(),
            // The server posts the shot.
            has_been_shot: true,
            silent: false,
            fall_distance: 0.0,
            stuck: Vec3::ZERO,
            was_touching_water: false,
            first_tick: true,
        }
    }
    /// `Projectile.setOwner`; the UUID is kept only for an owner `who` can't name.
    pub fn set_owner(&mut self, who: Option<Who>, uuid: Option<[i32; 4]>, guest: Option<u32>) {
        (self.owner, self.guest) = (who, guest);
        self.owner_uuid = uuid.filter(|_| who.is_none());
    }
    /// `FireworkRocketEntity.addAdditionalSaveData`; the owner when it is
    /// `player`, or an entity that is neither a player nor a mob.
    pub fn to_nbt(
        &self,
        uuid: [i32; 4],
        player: Option<[i32; 4]>,
        raw: Option<&sunrise_core::nbt::Tag>,
        items: &sunrise_items::items::Items,
    ) -> sunrise_core::nbt::Tag {
        sunrise_sim::entity_nbt::firework_rocket_to_nbt(
            &sunrise_sim::entity_nbt::RocketSave {
                rotation: self.rotation,
                position: self.position,
                motion: self.velocity,
                life: self.life,
                lifetime: self.lifetime,
                item: &self.item,
                angled: self.angled,
                owner: if matches!(self.owner, Some(Who::Player)) {
                    player
                } else {
                    self.owner_uuid
                },
                left_owner: self.left_owner,
                on_ground: self.ground.on_ground,
                has_been_shot: self.has_been_shot,
                fall_distance: self.fall_distance,
            },
            uuid,
            raw,
            items,
        )
    }
    /// A rocket loaded from its NBT (`/summon`, a saved chunk).
    pub fn from_nbt(r: RocketNbt) -> Self {
        let item = r
            .item
            .unwrap_or_else(|| ItemStack::new("minecraft:firework_rocket", 1));
        let (_, explosions) = fireworks_of(&item);
        Self {
            size_answer: None,
            attachment_lifetime: Default::default(),
            rotation: r.rotation,
            previous_rotation: r.rotation,
            join: Default::default(),
            id: 0,
            position: r.position,
            previous: r.position,
            velocity: r.motion,
            life: r.life,
            lifetime: r.lifetime,
            explosions: explosions.into(),
            attached: false,
            guest: None,
            owner: r.owner,
            owner_uuid: r.owner_uuid.filter(|_| r.owner.is_none()),
            left_owner: r.left_owner,
            last_deflected_by: None,
            attached_to: None,
            angled: r.angled,
            item: item.into(),
            random: sunrise_sim::projectiles::unique_random(r.position),
            ground: sunrise_world::ground_support::GroundState::from_ground_flag(r.on_ground),
            has_been_shot: r.has_been_shot,
            silent: r.silent,
            fall_distance: r.fall_distance,
            stuck: Vec3::ZERO,
            was_touching_water: false,
            first_tick: true,
        }
    }
    /// `Entity.load` over a live rocket (`/data merge`, `/data modify`): what
    /// [`Self::from_nbt`] reads replaces its own; it keeps its id and what it
    /// boosts.
    pub fn load_nbt(&mut self, r: RocketNbt) {
        let loaded = Self::from_nbt(r);
        self.rotation = loaded.rotation;
        self.previous_rotation = loaded.rotation;
        self.has_been_shot = loaded.has_been_shot;
        self.silent = loaded.silent;
        self.position = loaded.position;
        self.previous = loaded.position;
        self.velocity = loaded.velocity;
        self.life = loaded.life;
        self.lifetime = loaded.lifetime;
        self.explosions = loaded.explosions;
        self.item = loaded.item;
        self.angled = loaded.angled;
        self.left_owner = loaded.left_owner;
        self.ground = loaded.ground;
        self.fall_distance = loaded.fall_distance;
    }
    /// One tick of a server's client: the rocket flies on by itself between
    /// the server's updates (a rocket that follows an entity is placed by the
    /// caller).
    pub fn remote_step(&mut self, motion: sunrise_sim::motion::BodyMotion<'_>) {
        self.previous = self.position;
        self.previous_rotation = self.rotation;
        if self.attached_to.is_none() {
            let mut scene = SceneCollisions {
                world: motion.world,
                binding: motion.collision,
                subject: None,
            };
            if let Some((_, wanted, moved, restored)) = self.free_flight(&mut scene, motion.context)
            {
                let movement = self.flight_contact(motion, wanted, moved);
                self.velocity = motion
                    .restitute(
                        (self.id, "minecraft:firework_rocket"),
                        movement,
                        0.0,
                        0.98_f32,
                        self.velocity,
                        self.fall_distance,
                    )
                    .velocity;
                motion.apply_block_speed(self.position, &mut self.velocity);
                motion.step_on(
                    (self.id, "minecraft:firework_rocket"),
                    (self.position, self.bounds()),
                    &self.ground,
                    &mut self.velocity,
                );
                self.velocity = restored;
            }
        }
        self.life += 1;
    }
    /// The end of `Projectile.shoot`: yaw and pitch are set straight from the
    /// shot's motion, and the old rotation (`yRotO`, `xRotO`) to them.
    pub fn turn_to_shot(&mut self) {
        let v = self.velocity;
        let horizontal = (v.x * v.x + v.z * v.z).sqrt();
        let atan2 = sunrise_core::geometry::trig::atan2_degrees;
        self.rotation = [atan2(v.x, v.z), atan2(v.y, horizontal)];
        self.previous_rotation = self.rotation;
    }
    /// The move of a rocket attached to no entity: sped up and rising, or an
    /// angled one's own motion.
    pub fn next_motion(&self) -> Vec3 {
        if self.angled {
            self.velocity
        } else {
            let acceleration = if self.ground.horizontal_collision {
                1.0
            } else {
                1.15
            };
            Vec3::new(
                self.velocity.x * acceleration,
                self.velocity.y + 0.04,
                self.velocity.z * acceleration,
            )
        }
    }
    /// `EntityType.FIREWORK_ROCKET.getDimensions()`: width, height and eye height.
    pub(in crate::game) const DIMENSIONS: [f32; 3] = [0.25, 0.25, 0.2125];
    /// `EntityType.FIREWORK_ROCKET`: 0.25 x 0.25.
    pub fn bounds(&self) -> Aabb {
        let vanilla = Self::bounds_at(self.position);
        sunrise_sim::pose::SizeAnswer::sized_box(
            self.size_answer,
            Self::DIMENSIONS,
            self.position,
            vanilla,
        )
    }
    /// [`Self::bounds`] at `position`.
    pub fn bounds_at(position: Vec3) -> Aabb {
        let h = 0.125;
        Aabb::new(
            Vec3::new(position.x - h, position.y, position.z - h),
            Vec3::new(position.x + h, position.y + 2.0 * h, position.z + h),
        )
    }
    /// `FireworkRocketEntity.tick` for a rocket attached to no entity: speed
    /// up (an angled one keeps its motion) and rise, then move (`Entity.move`:
    /// a cobweb or powder snow scales this move only). Returns where it moved
    /// from, the move it asked for and the one it made.
    pub fn free_flight(
        &mut self,
        scene: &mut SceneCollisions,
        context: &sunrise_world::native_load_context::LoopContext,
    ) -> Option<(Vec3, Vec3, Vec3, Vec3)> {
        let bounds = self.bounds();
        let motion = self.next_motion();
        let mut wanted = motion;
        self.velocity = motion;
        if self.stuck.length_squared() > 1.0e-7 {
            wanted = Vec3::new(
                wanted.x * self.stuck.x,
                wanted.y * self.stuck.y,
                wanted.z * self.stuck.z,
            );
            self.stuck = Vec3::ZERO;
            self.velocity = Vec3::ZERO;
        }
        let from = self.position;
        let mut gate = sunrise_sim::move_gate::armed(
            context,
            self.id,
            "minecraft:firework_rocket",
            sunrise_world::entity_move::MoverType::SelfMovement,
            wanted,
        );
        let wanted = gate.delta();
        let Ok(moved) =
            sunrise_sim::move_gate::scene_resolve(&mut gate, scene, bounds, wanted, 0.0, false)
        else {
            return None;
        };
        self.position = Vec3::new(
            self.position.x + moved.x,
            self.position.y + moved.y,
            self.position.z + moved.z,
        );
        Some((from, wanted, moved, motion))
    }

    pub fn flight_contact(
        &mut self,
        motion: sunrise_sim::motion::BodyMotion<'_>,
        wanted: Vec3,
        moved: Vec3,
    ) -> sunrise_sim::motion::BodyMove {
        let authoritative =
            motion.context.side() == sunrise_world::native_load_context::LoopSide::Server;
        motion.record_move(
            sunrise_sim::motion::BodyCollision {
                footprint: (self.position, self.bounds()),
                movement: (wanted, moved),
                authoritative,
                context: sunrise_sim::motion::BodyMotion::entity_context(self.id, self.position),
                subject: None,
            },
            &mut self.ground,
        );
        let effect = motion.legacy_contact(self.position, &self.ground);
        sunrise_sim::motion::BodyMove {
            wanted,
            moved,
            effect,
        }
    }
}
/// `damageAmount * (float)Math.sqrt((5.0 - this.distanceTo(target)) / 5.0)`
/// from the rocket at `from` to a target at `to`; `Entity.distanceTo` sums
/// the squared offsets in floats.
pub fn blast_damage(damage: f32, from: Vec3, to: Vec3) -> f32 {
    let (x, y, z) = (
        (from.x - to.x) as f32,
        (from.y - to.y) as f32,
        (from.z - to.z) as f32,
    );
    let distance = f64::from(x * x + y * y + z * z).sqrt() as f32;
    damage * ((5.0 - f64::from(distance)) / 5.0).sqrt() as f32
}
pub fn fireworks_of(stack: &ItemStack) -> (u8, Vec<FireworkExplosion>) {
    // The item's default component: flight 1, no explosions.
    stack
        .data()
        .and_then(|d| d.fireworks.clone())
        .unwrap_or((1, Vec::new()))
}
/// `FireworkRocketEntity(Level, double, double, double, ItemStack)`, after the UUID:
/// the motion's two triangles, then the lifetime's two `nextInt`s, all from `random`.
pub fn rocket_draws(random: &mut sunrise_core::random::LegacyRandom, flight: u8) -> (Vec3, i32) {
    let tri = |r: &mut sunrise_core::random::LegacyRandom| r.triangle(0.0, 0.002297);
    let velocity = Vec3::new(tri(random), 0.05, tri(random));
    let lifetime = 10 * (1 + i32::from(flight))
        + random.next_int_bound(6).unwrap_or(0)
        + random.next_int_bound(7).unwrap_or(0);
    (velocity, lifetime)
}
pub trait FireworksGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// A rocket built from `own`, the rocket's own random (`Entity.random`): the
    /// UUID, the motion and the lifetime draw from it in that order, and a rocket
    /// that is then shot (`Projectile.shoot`) goes on drawing from the same one.
    /// `Entity.getHandHoldingItemAngle(Items.FIREWORK_ROCKET)` for the local
    /// player: half a block out towards the hand holding the rocket while
    /// gliding, zero otherwise. A rocket only in the offhand is in the arm
    /// opposite the main one.
    fn rocket_hand_angle(&self) -> [f64; 3] {
        if !self.player.as_ref().is_some_and(|p| p.fall_flying) {
            return [0.0; 3];
        }
        let is_rocket =
            |s: Option<&ItemStack>| s.is_some_and(|s| &*s.item == "minecraft:firework_rocket");
        let main = self.stack_in(self.selected);
        let offhand_only = is_rocket(self.offhand.as_ref()) && !is_rocket(main);
        let right = offhand_only == crate::game::Side::main_hand_left(self.side_state());
        let arm = if right { 80 } else { -80 };
        let v = view_vector(self.yaw + arm as f32, 0.0);
        [v[0] * 0.5, v[1] * 0.5, v[2] * 0.5]
    }
    /// `FireworkRocketEntity.tick` for a rocket attached to the local player:
    /// a glide is boosted towards the look, and the rocket sits at the rocket
    /// hand with the player's motion.
    fn follow_own_player(&mut self, r: &mut Rocket) {
        let hand = self.rocket_hand_angle();
        // getLookAngle reads Mth's float sine table, as the server does; the
        // f64 sin/cos of super::view_vector drift the boost off the server's
        // path and trip the movement check while gliding under a rocket.
        let look = sunrise_world::view_direction::view_vector(self.pitch, self.yaw);
        let Some(p) = self.player.as_mut() else {
            return;
        };
        if p.fall_flying {
            let m = p.velocity;
            p.velocity = Vec3::new(
                m.x + look.x * 0.1 + (look.x * 1.5 - m.x) * 0.5,
                m.y + look.y * 0.1 + (look.y * 1.5 - m.y) * 0.5,
                m.z + look.z * 0.1 + (look.z * 1.5 - m.z) * 0.5,
            );
        }
        r.position = p.position.add_vector(Vec3::new(hand[0], hand[1], hand[2]));
        r.velocity = p.velocity;
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> FireworksGameExt1<'__sg> for G {}

/// `FireworkRocketItem.useOn` returns PASS while the player is fall flying,
/// leaving the boost to `use` (a guest's UseItemOn must not launch a rocket).
pub fn use_on_passes(fall_flying: bool) -> bool {
    fall_flying
}
#[cfg(any(test, feature = "test-support"))]
#[path = "fireworks/hit_attempt_tests.rs"]
pub mod hit_attempt_tests;

sunrise_sim::impl_joined!(Rocket);
sunrise_mod_runtime::impl_attachment_lifetime!(Rocket);

impl sunrise_sim::entity_rules::EntityRules for Rocket {}
