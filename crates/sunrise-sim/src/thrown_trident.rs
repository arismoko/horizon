//! `ThrownTrident`: the thrown trident's state, its NBT and the flight rules that need no level.
use crate::entities::arrows::Pickup;
use sunrise_core::{
    geometry::{Aabb, Vec3},
    random::RandomSource,
};
use sunrise_items::items::ItemStack;

pub struct ThrownTrident {
    /// Entity-owned registration metadata; content clones are detached.
    pub attachment_lifetime: sunrise_mod_runtime::attachments::AttachmentLifetime,
    /// When it joined its list ([`JoinSeq`](crate::join_seq::JoinSeq)); set by the list.
    pub join: crate::join_seq::JoinSeq,
    /// The entity id a server tracks it by (0 until it is tracked).
    pub id: u32,
    pub stack: ItemStack,
    pub position: Vec3,
    pub previous: Vec3,
    pub velocity: Vec3,
    pub needs_sync: bool,
    pub yaw: f32,
    pub pitch: f32,
    /// `Entity.yRotO` and `xRotO`: the yaw and pitch when its tick began (`setOldPosAndRot`).
    pub previous_yaw: f32,
    pub previous_pitch: f32,
    pub in_ground: bool,
    /// `AbstractArrow.inGroundTime`: the ticks it has been stuck, from 0 on
    /// load; past 4, Loyalty calls the trident back.
    pub in_ground_time: i32,
    /// `AbstractArrow.life`, the saved despawn count: kept apart from
    /// `in_ground_time`, as loaded (negative too).
    pub life: i32,
    /// `AbstractArrow.shakeTime`: seven ticks after sticking, no pickup.
    pub shake: i32,
    pub dealt_damage: bool,
    pub loyalty: u32,
    pub no_physics: bool,
    pub returning: u32,
    pub pickup: Pickup,
    pub left_owner: bool,
    pub has_been_shot: bool,
    pub owner_uuid: Option<[i32; 4]>,
    /// `Entity.isNoGravity`: `AbstractArrow.applyGravity` then skips the pull.
    pub no_gravity: bool,
    /// `Entity.onGround`: moved by `setPos`, it keeps the one it was loaded with.
    pub ground: sunrise_world::ground_support::GroundState,
    /// `Entity.remainingFireTicks`; on clients, the synchronized on-fire flag.
    pub fire_ticks: i32,
    pub random: sunrise_core::random::LegacyRandom,
    /// `Entity.fallDistance`: kept as loaded; water resets it.
    pub fall_distance: f64,
    /// `AbstractArrow.soundEvent`: the trident's own until it first hits a block.
    pub sound_event: String,
    /// Water membership from the preceding `baseTick` (not updated while stuck).
    pub was_touching_water: bool,
    /// `Entity.random`, used for the extinguishing sound and a breeze's turn.
    /// `AbstractArrow.lastState`: the block it stuck in; not saved.
    pub last_state: Option<sunrise_world::registry::BlockStateId>,
    /// `Projectile.lastDeflectedBy`: the breeze that already turned this trident.
    pub deflected_by: Option<u32>,
    /// `Entity.tickCount`: `ProjectileUtil.computeMargin` grows with it.
    pub age: u32,
    /// The entity id a server sent as its owner (the one Loyalty returns it to).
    pub owner: u32,
    /// The guest that threw it (`Projectile.getOwner`); `None` for the host.
    pub guest: Option<u32>,
    /// What `entity_size` answered at its last size refresh (`EntityEvent.Size`): its box takes
    /// the answer while it holds for its vanilla size.
    pub size_answer: Option<crate::pose::SizeAnswer>,
}
impl ThrownTrident {
    /// A trident loaded from its NBT (`AbstractArrow.readAdditionalSaveData`).
    pub fn from_nbt(
        n: crate::entity_nbt::TridentNbt,
        tag: &sunrise_core::nbt::Tag,
        registry: &sunrise_world::registry::BlockRegistry,
    ) -> Self {
        let mut t = Self::remote(0, n.position, n.yaw, n.pitch);
        t.load_nbt(n, tag, registry);
        t
    }
    /// `ThrownTrident.readAdditionalSaveData` on a new or resident entity;
    /// `/data` keeps its runtime identity, random and tick count.
    pub fn load_nbt(
        &mut self,
        n: crate::entity_nbt::TridentNbt,
        tag: &sunrise_core::nbt::Tag,
        registry: &sunrise_world::registry::BlockRegistry,
    ) {
        // `inBlockState` (`BlockState.CODEC`) is `lastState`; absent or
        // unreadable reads as null.
        self.last_state = tag
            .get("inBlockState")
            .and_then(|s| crate::entity_nbt::state_from_nbt(registry, s));
        self.position = n.position;
        self.previous = n.position;
        self.yaw = n.yaw;
        self.pitch = n.pitch;
        // Entity.load ends with setOldPosAndRot.
        self.previous_yaw = n.yaw;
        self.previous_pitch = n.pitch;
        // The server resolves Loyalty against its current enchantment table after this read.
        self.loyalty = 0;
        self.stack = n.item;
        self.velocity = n.motion;
        self.in_ground = n.in_ground;
        self.life = n.life;
        self.in_ground_time = 0;
        self.shake = n.shake;
        self.dealt_damage = n.dealt_damage;
        self.pickup = n.pickup;
        self.left_owner = n.left_owner;
        self.has_been_shot = n.has_been_shot;
        self.owner_uuid = n.owner;
        self.guest = None;
        self.no_gravity = n.no_gravity;
        self.ground.on_ground = n.on_ground;
        self.fire_ticks = n.fire_ticks;
        self.fall_distance = n.fall_distance;
        self.sound_event = n.sound_event;
    }
    /// `AbstractArrow.addAdditionalSaveData` + `DealtDamage`.
    pub fn to_nbt(
        &self,
        uuid: [i32; 4],
        registry: &sunrise_world::registry::BlockRegistry,
        raw: Option<&sunrise_core::nbt::Tag>,
        items: &sunrise_items::items::Items,
    ) -> sunrise_core::nbt::Tag {
        let mut tag = crate::entity_nbt::trident_to_nbt(
            &crate::entity_nbt::TridentSave {
                position: self.position,
                motion: self.velocity,
                yaw: self.yaw,
                pitch: self.pitch,
                in_ground: self.in_ground,
                life: self.life,
                shake: self.shake,
                dealt_damage: self.dealt_damage,
                pickup: self.pickup,
                left_owner: self.left_owner,
                has_been_shot: self.has_been_shot,
                owner: self.owner_uuid,
                no_gravity: self.no_gravity,
                on_ground: self.ground.on_ground,
                fire_ticks: self.fire_ticks,
                fall_distance: self.fall_distance,
                sound_event: &self.sound_event,
                item: &self.stack,
            },
            uuid,
            raw,
            items,
        );
        // `AbstractArrow.addAdditionalSaveData`: `storeNullable("inBlockState")`,
        // never a stale value from the loaded compound.
        if let sunrise_core::nbt::Tag::Compound(map) = &mut tag {
            map.remove(&"inBlockState".into());
            if let Some(state) = self.last_state {
                map.insert(
                    "inBlockState".into(),
                    crate::entity_nbt::state_to_nbt(registry, state),
                );
            }
        }
        tag
    }
    /// A trident a server sent (`handleAddEntity`): placed and moved by it.
    pub fn remote(id: u32, position: Vec3, yaw: f32, pitch: f32) -> Self {
        Self {
            size_answer: None,
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id,
            stack: ItemStack::new("minecraft:trident", 1),
            position,
            previous: position,
            velocity: Vec3::ZERO,
            needs_sync: false,
            yaw,
            pitch,
            previous_yaw: yaw,
            previous_pitch: pitch,
            in_ground: false,
            in_ground_time: 0,
            life: 0,
            shake: 0,
            dealt_damage: false,
            loyalty: 0,
            no_physics: false,
            returning: 0,
            pickup: Pickup::Disallowed,
            left_owner: false,
            has_been_shot: false,
            owner_uuid: None,
            no_gravity: false,
            ground: Default::default(),
            fire_ticks: 0,
            fall_distance: 0.0,
            sound_event: crate::entity_nbt::TRIDENT_HIT_GROUND.to_owned(),
            was_touching_water: false,
            age: 0,
            owner: 0,
            guest: None,
            random: crate::projectiles::unique_random(position),
            deflected_by: None,
            last_state: None,
        }
    }
    /// `Entity.noPhysics`, with the synced flag a client reads
    /// (`AbstractArrow`'s entity-data flags, bit 1): the tracker sends them
    /// again on the next tick.
    pub fn set_no_physics(&mut self, no_physics: bool) {
        self.no_physics = no_physics;
        self.needs_sync = true;
    }
    /// `Entity.move(PISTON)`: the position only; `release_after_piston` follows
    /// (`AbstractArrow.move`).
    pub fn move_by_piston(&mut self, movement: Vec3) {
        self.position = self.position.add_vector(movement);
        self.needs_sync = true;
    }
    pub fn piston_release_bounds(&self) -> Option<Aabb> {
        if !self.in_ground || self.no_physics {
            return None;
        }
        Some(Aabb::new(self.position, self.position).inflate(Vec3::new(0.06, 0.06, 0.06)))
    }
    pub fn release_after_piston(&mut self, no_block_collision: bool) {
        if self.in_ground && !self.no_physics && no_block_collision {
            self.start_falling();
        }
    }
    /// A slime block's bounce: the velocity only; the piston's move after it
    /// releases a stuck trident (`AbstractArrow.move`).
    pub fn piston_slime_bounce(&mut self, movement: sunrise_core::geometry::Direction) {
        self.velocity = sunrise_world::piston_push::bounce_velocity(movement, self.velocity);
        self.needs_sync = true;
    }
    /// `AbstractArrow.onHitBlock`: plays `getHitGroundSoundEvent()`, then
    /// `setSoundEvent(SoundEvents.ARROW_HIT)` for every later hit and the save.
    pub fn hit_ground_sound(&mut self) -> String {
        std::mem::replace(
            &mut self.sound_event,
            "minecraft:entity.arrow.hit".to_owned(),
        )
    }
    /// `AbstractArrow.tick` in flight, before it moves: the rotation turns a
    /// fifth of the way (`Projectile.lerpRotation`) toward the movement the
    /// tick starts with, which faces back while it flies without physics.
    pub fn turn_towards_movement(&mut self) {
        use sunrise_core::geometry::trig::atan2_degrees;
        let m = self.velocity;
        let y_rot = if self.no_physics {
            atan2_degrees(-m.x, -m.z)
        } else {
            atan2_degrees(m.x, m.z)
        };
        let x_rot = atan2_degrees(m.y, m.horizontal_distance());
        self.pitch = crate::entities::arrows::lerp_rotation(self.pitch, x_rot);
        self.yaw = crate::entities::arrows::lerp_rotation(self.yaw, y_rot);
    }
    /// `AbstractArrow.shouldFall`: stuck, and nothing solid within 0.06 of it.
    pub fn should_fall(&self, scene: &crate::entities::arrows::ArrowScene<'_, '_>) -> bool {
        let at = self.position;
        self.in_ground
            && scene.no_block_collision(Aabb::new(at, at).inflate(Vec3::new(0.06, 0.06, 0.06)))
    }
    /// `ThrownTrident.tickDespawn`: a Loyalty trident that can be picked up never
    /// counts; any other adds one to `life` (`AbstractArrow.tickDespawn`) and
    /// is discarded at 1200. True when it is discarded.
    pub fn tick_despawn(&mut self) -> bool {
        if self.loyalty > 0 && self.pickup == Pickup::Allowed {
            return false;
        }
        self.life += 1;
        self.life >= 1200
    }
    /// `AbstractArrow.startFalling`: unstick, scaling the velocity by three
    /// draws of `nextFloat() * 0.2F` in x, y, z order.
    pub fn start_falling(&mut self) {
        self.in_ground = false;
        let v = self.velocity;
        let x = f64::from(self.random.next_float() * 0.2);
        let y = f64::from(self.random.next_float() * 0.2);
        let z = f64::from(self.random.next_float() * 0.2);
        self.velocity = Vec3::new(v.x * x, v.y * y, v.z * z);
        // `this.life = 0`; and a LAN client hears of it.
        self.life = 0;
        self.in_ground_time = 0;
        self.needs_sync = true;
    }
    /// `ThrownTrident.tick`'s Loyalty return: pulled up and towards the
    /// owner's eye.
    pub fn loyalty_pull(&mut self, owner_eye: Vec3) {
        let v = Vec3::new(
            owner_eye.x - self.position.x,
            owner_eye.y - self.position.y,
            owner_eye.z - self.position.z,
        );
        self.position.y += v.y * 0.015 * f64::from(self.loyalty);
        let len = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt().max(1e-9);
        let accel = 0.05 * f64::from(self.loyalty);
        self.velocity = Vec3::new(
            self.velocity.x * 0.95 + v.x / len * accel,
            self.velocity.y * 0.95 + v.y / len * accel,
            self.velocity.z * 0.95 + v.z / len * accel,
        );
    }
}
pub const HALF: f64 = 0.25;

impl crate::entity_rules::EntityRules for ThrownTrident {}
crate::impl_joined!(ThrownTrident);
sunrise_mod_runtime::impl_attachment_lifetime!(ThrownTrident);
