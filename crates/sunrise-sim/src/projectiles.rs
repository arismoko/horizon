//! Projectiles fired by mobs and thrown by the player: arrows
//! (`AbstractArrow`, gravity 0.05), throwables (`ThrowableProjectile`:
//! snowballs, eggs, ender pearls, gravity 0.03) and fireballs
//! (`AbstractHurtingProjectile`: no gravity, 0.1 acceleration, 0.95 inertia).
//! The game applies the effects of a hit.
#[cfg(test)]
#[path = "projectile_rotation_tests.rs"]
mod projectile_rotation_tests;
use crate::entity_rules::EntityRules;
use crate::mobs::Who;
use sunrise_core::{
    geometry::{Aabb, Direction, Vec3},
    position::BlockPos,
    random::RandomSource,
};

/// `ProjectileUtil.rotateTowardsMovement`'s loops: `old` moved by whole
/// turns of 360 until `new - old` lies in [-180, 180).
///
/// A loaded rotation is kept as saved, and a huge finite angle (1e30) would
/// never move: 360 is below its float spacing, so vanilla's loops would never
/// end. An angle more than 1024 turns away is first brought within one turn
/// with `%` (Java's `frem`). Closer angles take the loops alone, bit for bit.
/// A non-finite one, which `Entity.setYRot` and `setXRot` refuse on load, is
/// taken as 0.
fn shift_by_turns(mut old: f32, new: f32) -> f32 {
    // Written so that a NaN or infinite distance is "far" too.
    let near = (new - old).abs() < 360.0 * 1024.0;
    if !near {
        old = if old.is_finite() { old % 360.0 } else { 0.0 };
    }
    while new - old < -180.0 {
        old -= 360.0;
    }
    while new - old >= 180.0 {
        old += 360.0;
    }
    old
}

/// A new projectile entity's own random: vanilla seeds every entity's random
/// unpredictably, and a projectile's spread (and an arrow's base damage) come
/// from it, never from the shooter's random.
pub fn own_random(_shooter: &crate::mobs::Mob) -> sunrise_core::random::LegacyRandom {
    crate::random_source::entity()
}

/// A projectile's `Entity.random`. Two projectiles are equal whatever their
/// randoms, as with `ArrowState`: the state is not part of what they are.
#[derive(Clone, Debug)]
pub struct OwnRandom(pub sunrise_core::random::LegacyRandom);

impl PartialEq for OwnRandom {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// `RandomSource.create()` for a new entity's random: `seedUniquifier ^
/// nanoTime`, shared with the other entity constructors. Draws nothing from
/// any level random.
pub fn unique_random(_position: Vec3) -> sunrise_core::random::LegacyRandom {
    crate::random_source::entity()
}

/// `ProjectileDeflection`: how a projectile's movement turns.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Turn {
    /// `REVERSE`: one float from the projectile's random, then backwards at `-power / 2`.
    Reverse,
    /// `AIM_DEFLECT` along the deflecting entity's look, or `MOMENTUM_DEFLECT`
    /// along its normalized known speed. None (no deflecting entity) changes
    /// nothing.
    Direction(Option<Vec3>),
}

impl Turn {
    /// The new movement, and the degrees `REVERSE` adds to the yaw. None when
    /// the turn changes nothing. Only `REVERSE` draws from `random`.
    pub fn apply(
        self,
        velocity: Vec3,
        power: [f64; 3],
        random: &mut impl RandomSource,
    ) -> Option<(Vec3, f32)> {
        match self {
            Turn::Reverse => {
                let rotation = 170.0 + random.next_float() * 20.0;
                let v = velocity;
                Some((
                    Vec3::new(
                        v.x * (-power[0] * 0.5),
                        v.y * (-power[1] * 0.5),
                        v.z * (-power[2] * 0.5),
                    ),
                    rotation,
                ))
            }
            Turn::Direction(dir) => dir.map(|d| {
                (
                    Vec3::new(d.x * power[0], d.y * power[1], d.z * power[2]),
                    0.0,
                )
            }),
        }
    }
}

/// A new owner as a projectile stores it: who, UUID and the guest's connection.
pub type OwnerFields = (Option<Who>, Option<[i32; 4]>, Option<u32>);

/// `AbstractArrow.startFalling`'s per-axis random velocity damping.
pub fn start_falling_velocity(
    velocity: Vec3,
    random: &mut sunrise_core::random::LegacyRandom,
) -> Vec3 {
    let x = f64::from(random.next_float() * 0.2);
    let y = f64::from(random.next_float() * 0.2);
    let z = f64::from(random.next_float() * 0.2);
    Vec3::new(velocity.x * x, velocity.y * y, velocity.z * z)
}

/// a content mod's thrown item projectile, a copy of `Snowball` that deals
/// `damage` and flies as its `item`.
#[derive(Debug, PartialEq)]
pub struct ThrownType {
    /// `<mod id>:<path>`.
    pub id: &'static str,
    pub item: &'static str,
    pub damage: f32,
    /// its index in [`thrown_types`]: the slot a per-type table keeps it under.
    pub slot: usize,
}
/// the thrown types content mods declare, in registry order.
static THROWN_TYPES: std::sync::OnceLock<Vec<ThrownType>> = std::sync::OnceLock::new();
/// registers the content mods' thrown types, `(id, item, damage)`, a list
/// set once per process (false when it already was).
pub fn register_thrown_types(types: impl IntoIterator<Item = (String, String, f32)>) -> bool {
    if THROWN_TYPES.get().is_some() {
        return false;
    }
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    let types = types
        .into_iter()
        .enumerate()
        .map(|(slot, (id, item, damage))| ThrownType {
            id: leak(id),
            item: leak(item),
            damage,
            slot,
        });
    THROWN_TYPES.set(types.collect()).is_ok()
}
/// the content mods' thrown types, in registry order.
pub fn thrown_types() -> &'static [ThrownType] {
    THROWN_TYPES.get().map_or(&[], Vec::as_slice)
}
/// the lib's tests' mod types, `ruby:bolt` thrown from `ruby:gem` and the
/// tameable `ruby:hound`, `ruby:tabby` and `ruby:macaw` (a wolf, a cat and a
/// parrot): the registries are set once per process, so every test registers
/// the same.
#[cfg(test)]
pub(crate) fn register_test_types() {
    let _ = register_thrown_types([("ruby:bolt".into(), "ruby:gem".into(), 2.0)]);
    let _ = crate::lan::tracking::register_templates([
        ("ruby:bolt".into(), "minecraft:snowball".into()),
        ("ruby:hound".into(), "minecraft:wolf".into()),
        ("ruby:macaw".into(), "minecraft:parrot".into()),
        ("ruby:tabby".into(), "minecraft:cat".into()),
    ]);
}
/// the mod type with entity type `id`; out of line, as vanilla's never are.
#[cold]
#[inline(never)]
pub fn thrown_type(id: &str) -> Option<&'static ThrownType> {
    thrown_types().iter().find(|t| t.id == id)
}
/// the mod type `item` throws (the first in registry order).
#[cold]
#[inline(never)]
pub fn thrown_by_item(item: &str) -> Option<&'static ThrownType> {
    thrown_types().iter().find(|t| t.item == item)
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProjectileKind {
    /// Arrow with its base damage, critical flag and tipped/stray/bogged effect.
    Arrow {
        base_damage: f64,
        critical: bool,
        effect: Option<(&'static str, i32)>,
    },
    /// `ThrownTrident` of a mob (a drowned's): flies and sticks with
    /// `AbstractArrow`'s tick; the trident itself is the arrow state's
    /// `pickup_item`.
    Trident,
    Snowball,
    Egg,
    EnderPearl,
    SmallFireball,
    LargeFireball {
        power: i32,
    },
    /// `DragonFireball`: an `AbstractHurtingProjectile` that bursts into
    /// dragon's breath, does not burn and trails dragon breath.
    DragonFireball,
    LlamaSpit,
    /// A player's `WindCharge` and a breeze's `BreezeWindCharge`: straight
    /// flight that bursts on whatever it meets.
    WindCharge,
    BreezeWindCharge,
    /// `ShulkerBullet`: homing on `target` in axis-aligned steps; one with
    /// no target (a summoned one) falls like any projectile.
    ShulkerBullet {
        target: Option<Who>,
        /// The current step direction (3D data value), if any.
        dir: Option<u8>,
        steps: i32,
        delta: [f64; 3],
        seed: u64,
    },
    /// `WitherSkull`: blue (dangerous) skulls keep their speed better.
    WitherSkull {
        dangerous: bool,
    },
    /// `ThrownSplashPotion` or `ThrownLingeringPotion`: the potion item it
    /// flies as (`ThrowableItemProjectile.getItem`). What it splashes is that
    /// stack's `potion_contents`, so a witch's potion and a summoned one are
    /// the same thing; its entity type is the stack's item.
    ///
    /// The `bool` is whether its entity type is `ThrownLingeringPotion`: the
    /// type is the entity's own and `/summon` can pair either with any `Item`,
    /// so it is kept next to the stack (a thrown potion's follows its item).
    Potion(Box<crate::items::ItemStack>, bool),
    /// `ThrownExperienceBottle`: its own entity type, gravity 0.07.
    ExperienceBottle,
    /// a content mod's type.
    Thrown(&'static ThrownType),
}
impl ProjectileKind {
    /// A thrown potion flying as `stack`: a lingering item makes the lingering
    /// entity type, any other the splash one.
    pub fn potion(stack: crate::items::ItemStack) -> Self {
        let lingering = &*stack.item == "minecraft:lingering_potion";
        Self::Potion(Box::new(stack), lingering)
    }
    /// Wind charges take no push from an explosion (vanilla leaves their motion at 0).
    pub fn ignores_blast(&self) -> bool {
        matches!(self, Self::WindCharge | Self::BreezeWindCharge)
    }
    /// Whether the kind is a `Fireball` proper (`LargeFireball`,
    /// `SmallFireball`): the projectiles that carry an `Item`
    /// (`Fireball.DATA_ITEM_STACK`) and are drawn as it. A `DragonFireball`
    /// and a `WitherSkull` extend the hurting projectile only.
    pub fn is_item_fireball(&self) -> bool {
        matches!(self, Self::SmallFireball | Self::LargeFireball { .. })
    }
    /// `ThrowableItemProjectile.getDefaultItem` for throwable kinds, else `None`.
    pub(crate) fn default_thrown_item(&self) -> Option<&'static str> {
        match self {
            Self::Snowball => Some("minecraft:snowball"),
            Self::Egg => Some("minecraft:egg"),
            Self::EnderPearl => Some("minecraft:ender_pearl"),
            Self::ExperienceBottle => Some("minecraft:experience_bottle"),
            Self::Potion(_, true) => Some("minecraft:lingering_potion"),
            Self::Potion(_, false) => Some("minecraft:splash_potion"),
            Self::Thrown(t) => Some(t.item),
            _ => None,
        }
    }
    /// Whether the kind is a `ThrowableItemProjectile` that owns a saved `Item`.
    pub fn is_throwable_item(&self) -> bool {
        self.default_thrown_item().is_some()
    }
    /// `EntityType.Builder.eyeHeight`: 0.13 for arrows and tridents, 0 for
    /// wind charges, else the default 0.85 of the box height.
    pub fn eye_height(&self) -> f64 {
        match self {
            Self::Arrow { .. } | Self::Trident => f64::from(0.13_f32),
            Self::WindCharge | Self::BreezeWindCharge => 0.0,
            _ => f64::from(self.size() as f32 * 0.85_f32),
        }
    }
    /// The width and height of its entity type's `sized(...)`.
    pub fn size(&self) -> f64 {
        match self {
            Self::Arrow { .. } | Self::Trident => 0.5,
            Self::Snowball
            | Self::Egg
            | Self::EnderPearl
            | Self::LlamaSpit
            | Self::Potion(..)
            | Self::ExperienceBottle
            | Self::Thrown(_) => 0.25,
            Self::SmallFireball
            | Self::WindCharge
            | Self::BreezeWindCharge
            | Self::ShulkerBullet { .. }
            | Self::WitherSkull { .. } => 0.3125,
            Self::LargeFireball { .. } | Self::DragonFireball => 1.0,
        }
    }
    /// Its type's `getDimensions()`: [`Self::size`] square, with the type's eye height.
    pub fn dimensions(&self) -> [f32; 3] {
        let eye = match self {
            Self::Arrow { .. } | Self::Trident => 0.13,
            Self::Snowball
            | Self::Egg
            | Self::EnderPearl
            | Self::LlamaSpit
            | Self::Potion(..)
            | Self::ExperienceBottle
            | Self::Thrown(_) => 0.2125,
            Self::SmallFireball | Self::ShulkerBullet { .. } | Self::WitherSkull { .. } => {
                0.265_625
            }
            Self::WindCharge | Self::BreezeWindCharge => 0.0,
            Self::LargeFireball { .. } | Self::DragonFireball => 0.85,
        };
        let size = self.size() as f32;
        [size, size, eye]
    }
    /// Its entity type.
    pub fn id(&self) -> &'static str {
        match self {
            Self::Arrow { .. } => "minecraft:arrow",
            Self::Trident => "minecraft:trident",
            Self::Snowball => "minecraft:snowball",
            Self::Egg => "minecraft:egg",
            Self::EnderPearl => "minecraft:ender_pearl",
            Self::SmallFireball => "minecraft:small_fireball",
            Self::LargeFireball { .. } => "minecraft:fireball",
            Self::DragonFireball => "minecraft:dragon_fireball",
            Self::LlamaSpit => "minecraft:llama_spit",
            Self::WindCharge => "minecraft:wind_charge",
            Self::BreezeWindCharge => "minecraft:breeze_wind_charge",
            Self::ShulkerBullet { .. } => "minecraft:shulker_bullet",
            Self::WitherSkull { .. } => "minecraft:wither_skull",
            // `ThrownSplashPotion`/`ThrownLingeringPotion`: the item names it.
            Self::Potion(_, true) => "minecraft:lingering_potion",
            Self::Potion(_, false) => "minecraft:splash_potion",
            Self::ExperienceBottle => "minecraft:experience_bottle",
            Self::Thrown(t) => t.id,
        }
    }
}
/// `ThrownItemRenderer`'s sprite for a thrown potion.
fn potion_sprite(item: &str) -> &'static str {
    if item == "minecraft:lingering_potion" {
        "item/lingering_potion"
    } else {
        "item/splash_potion"
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Projectile {
    /// Entity-owned registration metadata; content clones are detached.
    pub attachment_lifetime: sunrise_mod_runtime::attachments::AttachmentLifetime,
    /// When it joined its list ([`crate::join_seq`]); set by the list, not by the spawn site.
    pub join: crate::join_seq::JoinSeq,
    /// The entity id a server tracks it by (0 until it is tracked).
    pub id: u32,
    pub kind: ProjectileKind,
    pub owner: Option<Who>,
    /// A non-living vehicle owner, for collision exclusion only, never hit credit.
    pub owner_vehicle: Option<u32>,
    /// `Projectile.owner`'s UUID, kept while the owner is away (a guest who
    /// left, a mob not loaded): what its save writes as `Owner`.
    pub owner_uuid: Option<[i32; 4]>,
    /// On a server, the guest (by connection) who shot it: `Who::Player`
    /// then means them, not the host.
    pub guest: Option<u32>,
    pub position: Vec3,
    pub previous: Vec3,
    pub velocity: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    /// `Entity.yRotO`/`xRotO`: the rotation the last tick ended with, which
    /// the renderer interpolates from (`setOldPosAndRot`).
    pub previous_yaw: f32,
    pub previous_pitch: f32,
    pub age: u32,
    pub stuck: bool,
    /// `Entity.stuckSpeedMultiplier`: set by a cobweb or powder snow its box
    /// passed through on a block's push (`makeStuckInBlock`); the next push
    /// (`Entity.move`) then stops it dead. Zero: not stuck.
    pub stuck_speed: Vec3,
    /// Whether the projectile has left its owner's box (Projectile.leftOwner).
    pub left_owner: bool,
    /// `Projectile.lastDeflectedBy`: entity id that last deflected this projectile.
    pub last_deflected_by: Option<u32>,
    /// Non-arrow Projectile.hasBeenShot, independent of entity age.
    pub has_been_shot: bool,
    shoot_event: Option<Vec3>,
    /// `ShulkerBullet.destroy` ran in its base tick or block effects (`hurtServer`
    /// from fire or lava): it is discarded before its hit, which never lands.
    pub destroyed: bool,
    /// A shulker bullet hurt in its pre-move `Entity.baseTick` (`(position, is_lava)`),
    /// from `onFire` or `lavaHurt`; left for the game to play and discard.
    bullet_hurt: Option<(Vec3, bool)>,
    /// The shooter's bow enchantments (Power, Punch, Flame, Piercing).
    pub bonus: crate::mobs::gear::ArrowBonus,
    /// Arrows' `AbstractArrow` state (ticked by `arrows::tick`).
    pub arrow: Option<Box<crate::entities::arrows::ArrowState>>,
    /// `AbstractHurtingProjectile.accelerationPower`.
    pub acceleration: f64,
    /// `Entity.onGround` and retained support. Flight sets position directly;
    /// only an explicit ground write or a base move (such as a piston push) updates these.
    pub ground: sunrise_world::ground_support::GroundState,
    /// `Entity.needsSync`: a server tells its clients about the change this
    /// tick instead of at the next update interval.
    pub needs_sync: bool,
    /// `Entity.syncVelocity`, set by `markHurt`: a server sends its motion to
    /// the clients tracking it after its next update, due or not.
    pub sync_velocity: bool,
    /// `WindCharge.noDeflectTicks`: a player's wind charge turns for nothing
    /// (`deflect` refuses) until it has ticked 5 times. Not saved, so a
    /// loaded one starts again at 5.
    pub no_deflect_ticks: u8,
    /// The bubble column tops this tick's `onAboveBubbleColumn` pushes came
    /// from, for their particles (`sendBubbleColumnParticles`); the caller
    /// drains them.
    pub bubble_surfaces: Vec<[i32; 3]>,
    /// Water-entry splash sound and particles from `Entity.doWaterSplashEffect` this tick.
    pub water_splashes: Vec<crate::entities::ItemSplash>,
    /// `Entity.wasTouchingWater`: what the last tick's
    /// `updateFluidInteraction` found around the box after the move, read by
    /// the next tick's inertia (false until the first tick has moved it).
    pub touching_water: bool,
    /// `Entity.firstTick`: true until the first tick completes `Entity.baseTick`,
    /// gating `isInLava()` and water-entry splash effects.
    pub first_tick: bool,
    /// `Entity.noGravity` (`NoGravity` in its save): `getGravity` is 0.
    pub no_gravity: bool,
    /// `Entity.remainingFireTicks`: a thrown projectile that flew through a
    /// fire block burns for 160 ticks, with the flame drawn over it.
    pub fire_ticks: i32,
    /// `Entity.random` of a projectile.
    pub random: OwnRandom,
    pub air_supply: i32,
    pub has_visual_fire: bool,
    /// `Entity.fallDistance` (`fall_distance` in its save): a projectile sets
    /// its position without `Entity.move`, so it keeps what it was given.
    pub fall_distance: f64,
    /// `Entity.DATA_SHARED_FLAGS_ID`'s fire bit, set by the server's base tick.
    pub synced_fire: bool,
    /// `Fireball.DATA_ITEM_STACK`: the item a fireball is drawn as, a fire
    /// charge until [`Projectile::set_item`] gives it another. None for the
    /// kinds that are no `Fireball` (a wither skull, a wind charge).
    pub item: Option<crate::items::ItemStack>,
    /// `ThrowableItemProjectile.DATA_ITEM_STACK` for non-potion throwables (count 1).
    pub thrown_item: Option<Box<crate::items::ItemStack>>,
    /// `ThrownEgg`'s item `CHICKEN_VARIANT` component, as the canonical
    /// `minecraft:chicken_variant` registry entry id (`None` when removed or not an egg).
    pub egg_variant: Option<String>,
    /// The UUID its own random made (`Mth.createInsecureUUID`), for the server
    /// to keep unless a save gives one: an arrow's at construction, another
    /// kind's when it is shot (`shoot_own`), which is the same stream since
    /// nothing else draws from it between.
    pub uuid: Option<[i32; 4]>,
    /// What `entity_size` answered at its last size refresh (`EntityEvent.Size`): its box takes
    /// the answer while it holds for its vanilla size.
    pub size_answer: Option<crate::pose::SizeAnswer>,
}
/// A new projectile's own random (`Entity.random`) and the UUID its first two
/// draws make (`Mth.createInsecureUUID`).
pub(crate) fn new_own_random() -> (sunrise_core::random::LegacyRandom, [i32; 4]) {
    let mut random = crate::random_source::entity();
    let uuid = crate::save::insecure_uuid(&mut random);
    (random, uuid)
}
/// `ThrownEnderpearl.isAllowedToTeleportOwner(owner, level)` for an owner in
/// the pearl's own dimension: vanilla reaches
/// `LivingEntity.isAlive() && !LivingEntity.isSleeping()` (bytecode 26-45) and
/// returns false otherwise, so a pearl whose owner died or went to bed is
/// discarded where it lands: no teleport, no 5 damage, no endermite roll.
pub fn owner_can_take_pearl(alive: bool, sleeping: bool) -> bool {
    alive && !sleeping
}
/// What a projectile struck this tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    None,
    Block {
        pos: [i32; 3],
        at: Vec3,
        face: [i32; 3],
    },
    Entity {
        who: Who,
        at: Vec3,
    },
}
/// The world a projectile flies through: block ray clipping, water, and
/// the boxes of entities it could hit.
/// The speed multiplier `makeStuckInBlock` gives an entity in block `name`
/// (`WebBlock.entityInside`, `PowderSnowBlock.entityInside`), if any.
pub fn stuck_speed_of(name: &str) -> Option<Vec3> {
    match name {
        "minecraft:cobweb" => Some(Vec3::new(0.25, f64::from(0.05_f32), 0.25)),
        "minecraft:powder_snow" => Some(Vec3::new(f64::from(0.9_f32), 1.5, f64::from(0.9_f32))),
        _ => None,
    }
}
/// `ProjectileUtil.computeMargin`: float arithmetic before widening to `f64`.
#[inline]
pub fn compute_margin(tick_count: i32) -> f64 {
    f64::from(((tick_count - 2) as f32 / 20.0_f32).clamp(0.0_f32, 0.3_f32))
}
/// Vanilla `EntitySectionStorage.forEachAccessibleNonEmptySection` encounter key,
/// with dragon parts ordered after all regular entities (`Level.getEntities`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TargetOrder {
    pub is_dragon_part: bool,
    pub section_x: i32,
    pub section_z_key: u32,
    pub section_y_key: u32,
    pub join: u64,
}
impl TargetOrder {
    #[inline]
    pub fn entity(pos: Vec3, join: u64) -> Self {
        let sx = (pos.x.floor() as i32) >> 4;
        let sy = (pos.y.floor() as i32) >> 4;
        let sz = (pos.z.floor() as i32) >> 4;
        Self {
            is_dragon_part: false,
            section_x: sx,
            section_z_key: (sz as u32) & 0x3F_FFFF,
            section_y_key: (sy as u32) & 0xF_FFFF,
            join,
        }
    }
    #[inline]
    pub fn dragon_part(index: u32) -> Self {
        Self {
            is_dragon_part: true,
            section_x: 0,
            section_z_key: 0,
            section_y_key: 0,
            join: u64::from(index),
        }
    }
}
/// One live entity candidate for projectile collision queries.
#[derive(Clone, Copy, Debug)]
pub struct TargetCandidate {
    pub who: Who,
    pub bounds: Aabb,
    /// Actual `Entity.position()` (center for `HangingEntity`, feet otherwise).
    pub position: Vec3,
    pub order: TargetOrder,
}
/// Zero-allocation `ProjectileUtil.getEntityHitResult` accumulator.
#[derive(Clone, Copy, Debug, Default)]
pub struct NearestHit {
    best: Option<(Who, Vec3, f64, TargetOrder)>,
}
impl NearestHit {
    #[inline]
    pub fn new() -> Self {
        Self { best: None }
    }
    /// Broad-phase uninflated `bounds.intersects(search_area)`, exact `Aabb::clip`,
    /// strict nearest (`d < best`) with section encounter order tie-break.
    #[inline]
    pub fn consider(
        &mut self,
        c: TargetCandidate,
        search_area: Aabb,
        from: Vec3,
        to: Vec3,
        margin: f64,
    ) {
        if !c.bounds.intersects(search_area) {
            return;
        }
        let inflated = c.bounds.inflate(Vec3::new(margin, margin, margin));
        let Some(at) = inflated.clip(from, to) else {
            return;
        };
        let d = from.distance_squared(at);
        if self
            .best
            .is_none_or(|(_, _, bd, bo)| d < bd || (d == bd && c.order < bo))
        {
            self.best = Some((c.who, at, d, c.order));
        }
    }
    #[inline]
    pub fn finish(self) -> Option<(Who, Vec3)> {
        self.best.map(|(who, at, _, _)| (who, at))
    }
}
pub trait ProjectileWorld {
    fn clip(&self, from: Vec3, to: Vec3) -> Option<(Vec3, [i32; 3], [i32; 3])>;
    /// `Entity.isInWater` as `updateFluidInteraction` sets it: some water
    /// block's surface reaches the bottom of the box (deflated by 0.001).
    fn in_water(&self, bounds: Aabb) -> bool;
    /// `Entity.fluidHeight(LAVA) > 0.0` as `updateFluidInteraction` sets it.
    fn in_lava_box(&self, _bounds: Aabb) -> bool {
        false
    }
    /// `velocity` after the currents of the water and the lava the box
    /// touches (`EntityFluidInteraction.applyCurrentTo`): a projectile is
    /// pushed by fluids like any entity (`Entity.isPushedByFluid`).
    fn pushed_by_currents(&self, _bounds: Aabb, velocity: Vec3) -> Vec3 {
        velocity
    }
    fn targets(&self) -> &[(Who, Aabb)];
    /// Traverse live entity candidates without allocating; defaults to `targets()` for stubs.
    fn visit_targets(&self, mut visit: impl FnMut(TargetCandidate)) {
        for (idx, &(who, bounds)) in self.targets().iter().enumerate() {
            let c = bounds.center();
            let pos = Vec3::new(c.x, bounds.min_position().y, c.z);
            visit(TargetCandidate {
                who,
                bounds,
                position: pos,
                order: TargetOrder::entity(pos, idx as u64),
            });
        }
    }
    /// Whether `candidate` shares the owner's root-vehicle tree (`isPassengerOfSameVehicle`).
    fn owner_tree_contains(&self, owner: Who, candidate: Who) -> bool {
        candidate == owner
    }
    /// `Projectile.isOutsideOwnerCollisionRange`: true when `swept` touches no pickable
    /// entity in `owner`'s root-vehicle tree.
    fn outside_owner_collision_range(&self, owner: Who, swept: Aabb) -> bool {
        !self
            .targets()
            .iter()
            .any(|&(who, b)| who == owner && b.intersects(swept))
    }
    /// Box of a specific entity for `ShulkerBullet` homing without scanning a snapshot slice.
    fn target_bounds(&self, target: Who) -> Option<Aabb> {
        self.targets()
            .iter()
            .find(|(who, _)| *who == target)
            .map(|(_, b)| *b)
    }
    /// `Level.getMinY() - 64` (`Entity.checkBelowWorld`); the Overworld's
    /// unless the world says otherwise.
    fn below_world_y(&self) -> f64 {
        -128.0
    }
    /// `Level.isEmptyBlock`.
    fn empty(&self, _p: [i32; 3]) -> bool {
        true
    }
    /// `Level.loadedAndEntityCanStandOn(p, projectile)` with the projectile's
    /// feet at `entity_y`: the top face of the block's collision shape is
    /// full (`Block.isFaceFull`), as on a top slab, not only a full cube.
    fn can_stand_on(&self, _p: [i32; 3], _entity_y: f64) -> bool {
        false
    }
    /// `BubbleColumnBlock.entityInside`'s view of the block at `p`: a bubble
    /// column's `drag`, and whether nothing is above it (its surface, where
    /// it throws harder); None for any other block.
    fn bubble_column(&self, _p: [i32; 3]) -> Option<(bool, bool)> {
        None
    }
    /// `entityInside`'s `makeStuckInBlock` for the block at `p`: a cobweb's
    /// or powder snow's speed multiplier, None for any other block.
    fn stuck_speed(&self, _p: [i32; 3]) -> Option<Vec3> {
        None
    }
}
/// `CollisionGetter.clipIncludingBorder` over a block clip's result: a ray
/// from inside the world border whose hit (or end, on a miss) lies outside it
/// stops on the border instead, at `WorldBorder.clampVec3ToBound` of that
/// point, facing `Direction.getApproximateNearest` of the travel. `bounds` is
/// the border's [minX, minZ, maxX, maxZ].
pub fn clip_including_border(
    hit: Option<(Vec3, [i32; 3], [i32; 3])>,
    from: Vec3,
    to: Vec3,
    bounds: [f64; 4],
) -> Option<(Vec3, [i32; 3], [i32; 3])> {
    let [min_x, min_z, max_x, max_z] = bounds;
    // WorldBorder.isWithinBounds(x, z): the far edges are outside.
    let within = |p: Vec3| p.x >= min_x && p.x < max_x && p.z >= min_z && p.z < max_z;
    let end = hit.map_or(to, |h| h.0);
    if !within(from) || within(end) {
        return hit;
    }
    // Mth.clamp, up to 1.0E-5F inside the far edges.
    let clamp = |v: f64, low: f64, high: f64| if v < low { low } else { v.min(high) };
    let inset = f64::from(1.0e-5_f32);
    let at = Vec3::new(
        clamp(end.x, min_x, max_x - inset),
        end.y,
        clamp(end.z, min_z, max_z - inset),
    );
    let face =
        Direction::approximate_nearest(Vec3::new(end.x - from.x, end.y - from.y, end.z - from.z));
    let pos = BlockPos::containing(at);
    Some((at, [pos.x, pos.y, pos.z], face.normal()))
}
/// Direction steps by 3D data value: down, up, north, south, west, east.
pub const STEPS: [[i32; 3]; 6] = [
    [0, -1, 0],
    [0, 1, 0],
    [0, 0, -1],
    [0, 0, 1],
    [-1, 0, 0],
    [1, 0, 0],
];
fn axis_of(d: u8) -> u8 {
    match d {
        0 | 1 => 1,
        2 | 3 => 2,
        _ => 0,
    }
}
impl crate::entity_rules::EntityRules for Projectile {
    fn is_no_gravity(&self) -> bool {
        self.no_gravity
    }
    /// Each projectile class's `getDefaultGravity`.
    fn get_default_gravity(&self) -> f64 {
        match &self.kind {
            ProjectileKind::Arrow { .. } | ProjectileKind::Trident => 0.05,
            ProjectileKind::SmallFireball
            | ProjectileKind::LargeFireball { .. }
            | ProjectileKind::DragonFireball
            | ProjectileKind::WitherSkull { .. } => 0.0,
            ProjectileKind::LlamaSpit => 0.06,
            // `ThrownExperienceBottle.getDefaultGravity` is 0.07.
            ProjectileKind::ExperienceBottle => 0.07,
            ProjectileKind::Potion { .. } => 0.05,
            ProjectileKind::ShulkerBullet { .. } => 0.04,
            _ => 0.03,
        }
    }
}
/// `AbstractHurtingProjectile.applyInertia` / `ThrowableProjectile.applyInertia`:
/// four bubble particles at `position - movement * 0.25`, moving with `movement`.
pub(crate) fn water_bubble_at(position: [f64; 3], movement: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    (
        [
            position[0] - movement[0] * 0.25,
            position[1] - movement[1] * 0.25,
            position[2] - movement[2] * 0.25,
        ],
        movement,
    )
}
/// The chicken variant id a thrown egg stack carries: its explicit
/// `minecraft:chicken/variant` component, else its item's default (egg
/// temperate, blue egg cold, brown egg warm). An explicit removal
/// (`!minecraft:chicken/variant`) returns `None`, leaving the hatchling its
/// default variant.
pub fn egg_chicken_variant(stack: &crate::items::ItemStack) -> Option<String> {
    if let Some(raw) = stack
        .extra
        .as_ref()
        .and_then(|d| d.other("minecraft:chicken/variant"))
        .and_then(|t| match t {
            sunrise_core::nbt::Tag::String(s) => s.to_string().ok(),
            _ => None,
        })
        .and_then(|raw| {
            sunrise_core::identifier::Identifier::parse(&raw)
                .ok()
                .map(|id| id.as_str().to_owned())
        })
    {
        return Some(raw);
    }
    // A removed component resolves to absent, not to the item default.
    if stack
        .extra
        .as_ref()
        .is_some_and(|d| d.other("!minecraft:chicken/variant").is_some())
    {
        return None;
    }
    Some(
        match &*stack.item {
            "minecraft:blue_egg" => "minecraft:cold",
            "minecraft:brown_egg" => "minecraft:warm",
            _ => "minecraft:temperate",
        }
        .to_owned(),
    )
}
impl Projectile {
    /// `Entity.getBoundingBox`: its type's square box standing on its position,
    /// a wind charge's 0.15 lower (`AbstractWindCharge.makeBoundingBox`).
    pub fn bounds(&self) -> Aabb {
        self.bounds_at(self.position)
    }
    /// `Entity.makeBoundingBox(at)`: [`Self::bounds`] with the projectile at `at`.
    pub fn bounds_at(&self, p: Vec3) -> Aabb {
        let half = self.kind.size() / 2.0;
        let height = self.kind.size();
        let bottom = match self.kind {
            ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge => {
                p.y - f64::from(0.15_f32)
            }
            _ => p.y,
        };
        let vanilla = Aabb::new(
            Vec3::new(p.x - half, bottom, p.z - half),
            Vec3::new(p.x + half, bottom + height, p.z + half),
        );
        crate::pose::SizeAnswer::sized_box(self.size_answer, self.kind.dimensions(), p, vanilla)
    }
    /// The collision target of the owner, independent of living damage credit.
    pub(crate) fn collision_owner(&self) -> Option<Who> {
        self.owner_vehicle.map(Who::Mob).or(self.owner)
    }
    pub fn new(kind: ProjectileKind, position: Vec3, owner: Option<Who>) -> Self {
        let kind_is_arrow = matches!(kind, ProjectileKind::Arrow { .. } | ProjectileKind::Trident);
        // `Fireball.defineSynchedData`: DATA_ITEM_STACK starts as a fire charge.
        let item = kind
            .is_item_fireball()
            .then(|| crate::items::ItemStack::new("minecraft:fire_charge", 1));
        // `AbstractWindCharge`'s constructors set it to 0.
        let acceleration = if matches!(
            kind,
            ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge
        ) {
            0.0
        } else {
            0.1
        };
        let no_deflect_ticks = if matches!(kind, ProjectileKind::WindCharge) {
            5
        } else {
            0
        };
        // An arrow keeps its own random (`Entity.random`) for its later draws.
        let (random, uuid) = new_own_random();
        Self {
            size_answer: None,
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 0,
            thrown_item: None,
            egg_variant: None,
            kind,
            owner,
            owner_vehicle: None,
            owner_uuid: None,
            guest: None,
            position,
            previous: position,
            velocity: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            previous_yaw: 0.0,
            previous_pitch: 0.0,
            age: 0,
            stuck: false,
            stuck_speed: Vec3::ZERO,
            left_owner: false,
            last_deflected_by: None,
            has_been_shot: false,
            shoot_event: None,
            destroyed: false,
            bullet_hurt: None,
            bonus: Default::default(),
            acceleration,
            ground: Default::default(),
            needs_sync: false,
            sync_velocity: false,
            no_deflect_ticks,
            bubble_surfaces: Vec::new(),
            water_splashes: Vec::new(),
            touching_water: false,
            first_tick: true,
            no_gravity: false,
            fire_ticks: 0,
            // Entity.random: seeded unpredictably, the position stands in.
            random: OwnRandom(random),
            air_supply: 300,
            has_visual_fire: false,
            fall_distance: 0.0,
            synced_fire: false,
            item,
            arrow: kind_is_arrow.then(|| Box::new(crate::entities::arrows::ArrowState::new())),
            uuid: Some(uuid),
        }
    }
    /// Entity box (`EntityType.sized` cube of `kind.size()`), as `bounds`.
    pub fn aabb(&self) -> Aabb {
        self.bounds()
    }
    /// `Entity.random`.
    pub fn entity_random(&mut self) -> &mut sunrise_core::random::LegacyRandom {
        &mut self.random.0
    }
    /// `AbstractHurtingProjectile`: the kinds with an acceleration power.
    pub(crate) fn is_hurting(&self) -> bool {
        matches!(
            self.kind,
            ProjectileKind::LargeFireball { .. }
                | ProjectileKind::SmallFireball
                | ProjectileKind::DragonFireball
                | ProjectileKind::WitherSkull { .. }
                | ProjectileKind::WindCharge
                | ProjectileKind::BreezeWindCharge
        )
    }
    /// `Projectile.deflect(deflection, by, newOwner, byAttack, power)` on the
    /// server, for every caller: gameplay and a mod's handle. A wind charge
    /// in its first five ticks refuses with no change and no draw
    /// (`WindCharge.deflect`). Otherwise the turn (its draw from this
    /// projectile's random), then the new owner, then `onDeflection`: a
    /// hurting projectile's acceleration power is 0.1 by an attack, else
    /// halved. A turn also sets `needsSync`, and the rotation `REVERSE` adds
    /// goes to `yRotO` too. Returns whether it deflected.
    pub fn deflect(
        &mut self,
        turn: Turn,
        power: [f64; 3],
        owner: OwnerFields,
        by_attack: bool,
    ) -> bool {
        if self.no_deflect_ticks > 0 {
            return false;
        }
        let velocity = self.velocity;
        let turned = turn.apply(velocity, power, self.entity_random());
        if let Some((velocity, rotation)) = turned {
            self.velocity = velocity;
            self.yaw += rotation;
            self.previous_yaw += rotation;
            self.needs_sync = true;
        }
        (self.owner, self.owner_uuid, self.guest) = owner;
        self.owner_vehicle = None;
        if self.is_hurting() {
            self.acceleration = if by_attack {
                0.1
            } else {
                self.acceleration * 0.5
            };
        }
        true
    }
    /// `Projectile.hitTargetOrDeflectSelf` for a deflecting entity: skips repeat
    /// contact with the same deflector; otherwise deflects and remembers `by`.
    pub fn deflect_by(&mut self, by: u32, turn: Turn, power: [f64; 3]) -> bool {
        if self.last_deflected_by == Some(by) {
            return false;
        }
        let owner = (self.owner, self.owner_uuid, self.guest);
        if self.deflect(turn, power, owner, false) {
            self.last_deflected_by = Some(by);
            true
        } else {
            false
        }
    }
    /// The entity random from now on (`Entity.random`): a probe's fixture
    /// seeds it.
    pub fn with_entity_random(mut self, random: sunrise_core::random::LegacyRandom) -> Self {
        self.random.0 = random;
        self
    }
    /// The shooter's bow enchantments; Piercing sets its pierce level. Flame's
    /// `projectile_spawned` ignite runs once the arrow is in the level
    /// (`Projectile.applyOnProjectileSpawned`).
    pub(crate) fn set_bonus(&mut self, bonus: crate::mobs::gear::ArrowBonus) {
        if let Some(a) = self.arrow.as_mut() {
            a.pierce = bonus.pierce.min(127) as u8;
        }
        self.bonus = bonus;
    }
    /// `AbstractArrow.isCritArrow`.
    pub fn critical(&self) -> bool {
        matches!(self.kind, ProjectileKind::Arrow { critical: true, .. })
    }
    pub fn set_critical(&mut self, value: bool) {
        if let ProjectileKind::Arrow { critical, .. } = &mut self.kind {
            *critical = value;
        }
    }
    /// `AbstractArrow.tick`: critical streaks and submerged bubble trails.
    #[cfg(test)]
    pub(crate) fn arrow_trail(
        &self,
        water: bool,
    ) -> Vec<(
        sunrise_level::particle_options::ParticleType,
        [f64; 3],
        [f64; 3],
    )> {
        use sunrise_level::particle_options::ParticleType;
        let mut trail = Vec::new();
        if self.stuck || !self.is_arrow() {
            return trail;
        }
        let (p, v) = (self.position, self.velocity);
        if water {
            let drag = f64::from(0.6f32);
            let speed = [v.x * drag, v.y * drag, v.z * drag];
            let at = [
                p.x - speed[0] * 0.25,
                p.y - speed[1] * 0.25,
                p.z - speed[2] * 0.25,
            ];
            for _ in 0..4 {
                trail.push((ParticleType::Bubble, at, speed));
            }
        }
        if matches!(self.kind, ProjectileKind::Arrow { critical: true, .. }) {
            for i in 0..4 {
                let t = i as f64 / 4.0;
                trail.push((
                    ParticleType::Crit,
                    [p.x + v.x * t, p.y + v.y * t, p.z + v.z * t],
                    [-v.x, -v.y + 0.2, -v.z],
                ));
            }
        }
        trail
    }
    pub fn is_arrow(&self) -> bool {
        matches!(self.kind, ProjectileKind::Arrow { .. })
    }
    /// A `ThrowableProjectile`: snowballs, eggs, ender pearls, potions, a
    /// content mod's thrown item and a llama's spit. They are the projectiles
    /// that apply the effects of the blocks they fly through
    /// (`ThrowableProjectile.tick` calls `applyEffectsFromBlocks`), which an
    /// `AbstractHurtingProjectile` — a fireball, a wind charge, a skull — does
    /// not.
    pub fn is_thrown(&self) -> bool {
        matches!(
            self.kind,
            ProjectileKind::Snowball
                | ProjectileKind::Egg
                | ProjectileKind::EnderPearl
                | ProjectileKind::LlamaSpit
                | ProjectileKind::Potion(..)
                | ProjectileKind::Thrown(_)
        )
    }
    /// `Entity.isOnFire`. `WitherSkull.isOnFire` and `ShulkerBullet.isOnFire` always return false.
    pub fn on_fire(&self) -> bool {
        !matches!(
            self.kind,
            ProjectileKind::WitherSkull { .. } | ProjectileKind::ShulkerBullet { .. }
        ) && (self.fire_ticks > 0 || self.synced_fire)
    }
    /// `Entity.igniteForTicks`: the longer fire wins.
    pub fn ignite_for_ticks(&mut self, ticks: i32) {
        if self.fire_ticks < ticks {
            self.fire_ticks = ticks;
        }
    }
    /// `Entity.clearFire`: it counts from below zero, which `on_fire` reads as out.
    pub fn clear_fire(&mut self) {
        self.fire_ticks = self.fire_ticks.min(0);
    }
    /// `Entity.igniteForSeconds`: an arrow keeps its fire in its
    /// `AbstractArrow` state, every other kind in its own.
    pub fn ignite_for_seconds(&mut self, seconds: f32) {
        match self.arrow.as_mut() {
            Some(a) => a.ignite_for_seconds(seconds),
            None => self.ignite_for_ticks((seconds * 20.0).floor() as i32),
        }
    }
    /// `Entity.isOnFire` as the server counts it: no projectile is
    /// `fireImmune`, and `WitherSkull.isOnFire` and `ShulkerBullet.isOnFire`
    /// are always false.
    pub fn burning(&self) -> bool {
        self.fire_ticks > 0
            && !matches!(
                self.kind,
                ProjectileKind::WitherSkull { .. } | ProjectileKind::ShulkerBullet { .. }
            )
    }
    /// `Entity.baseTick`'s fire: a client clears it (its flame is the synced
    /// flag), the server counts it down and syncs its flag.
    fn base_fire_tick(&mut self, client_side: bool) {
        if client_side {
            self.clear_fire();
            return;
        }
        if self.fire_ticks > 0 {
            self.fire_ticks -= 1;
        }
        self.synced_fire = self.fire_ticks > 0 || self.has_visual_fire;
    }
    /// `AbstractHurtingProjectile.tick`'s `igniteForSeconds(1)` after its base
    /// tick, on both sides, for the two kinds whose `shouldBurn` is true.
    fn renew_fireball_flame(&mut self) {
        if self.kind.is_item_fireball() {
            self.ignite_for_ticks(20);
        }
    }
    /// LlamaSpit and ShulkerBullet run `Projectile.tick` before moving;
    /// ThrowableProjectile and AbstractHurtingProjectile run it afterwards.
    fn base_first(&self) -> bool {
        matches!(
            self.kind,
            ProjectileKind::LlamaSpit | ProjectileKind::ShulkerBullet { .. }
        )
    }
    /// The server's tail of `Entity.applyEffectsFromBlocks`, once the game has
    /// swept this tick's move and flushed its contacts (`fire_before` and
    /// `was_on_fire` read before the sweep): rain puts the fire out, then the
    /// count resets when it is out and no block ignited it this tick. A kind
    /// whose `Projectile.tick` follows its block effects then takes its base
    /// tick's fire and a fireball its renewed flame. The extinguish sound's
    /// pitch (`playEntityOnFireExtinguishedSound`) when the fire went out.
    pub fn finish_block_effects(
        &mut self,
        fire_before: i32,
        was_on_fire: bool,
        in_rain: bool,
    ) -> Option<f32> {
        if in_rain {
            self.clear_fire();
        }
        let pitch = (was_on_fire && !self.burning()).then(|| {
            use sunrise_core::random::RandomSource as _;
            let r = &mut self.random.0;
            1.6 + (r.next_float() - r.next_float()) * 0.4
        });
        if !self.burning() && self.fire_ticks <= fire_before {
            self.fire_ticks = 0;
        }
        if !self.base_first() {
            self.base_fire_tick(false);
            self.renew_fireball_flame();
        }
        pitch
    }
    /// Whether it is a `SpectralArrow` (its own entity type): a spectral
    /// arrow's pickup item, or the glowing effect a dispenser shoots with.
    pub fn is_spectral_arrow(&self) -> bool {
        let ProjectileKind::Arrow { effect, .. } = &self.kind else {
            return false;
        };
        effect.is_some_and(|(id, _)| id == "minecraft:glowing")
            || self
                .arrow
                .as_ref()
                .is_some_and(|a| a.spectral || &*a.pickup_item.item == "minecraft:spectral_arrow")
    }
    /// Its entity type id (`minecraft:spectral_arrow` for a spectral arrow,
    /// else its kind's id).
    pub fn entity_type(&self) -> &'static str {
        if self.is_spectral_arrow() {
            "minecraft:spectral_arrow"
        } else {
            self.kind.id()
        }
    }
    /// `Arrow.ID_EFFECT_COLOR` (`updateColor`): `PotionContents.getColor` of
    /// the contents the arrow carries, from its pickup item, or the effect a
    /// stray, bogged or parched adds; -1 while they are
    /// `PotionContents.EMPTY` (a colour and a custom effect alone make them
    /// nonempty, with no potion id at all).
    pub fn arrow_effect_color(&self) -> i32 {
        let ProjectileKind::Arrow { effect, .. } = &self.kind else {
            return -1;
        };
        if self.is_spectral_arrow() {
            return -1;
        }
        let color = match (effect, &self.arrow) {
            // One added effect is one custom effect: its own colour, opaque.
            (Some((id, _)), _) => crate::brewing::effect(id).map(|e| e.shown_color() | 0xFF00_0000),
            // `PotionContents.getColor` already carries the alpha a
            // `custom_color` was given, as vanilla syncs it. Any arrow item's
            // contents count, as `updateColor` reads the pickup stack.
            (None, Some(a)) => (!crate::brewing::contents_empty(&a.pickup_item))
                .then(|| crate::brewing::contents_color(&a.pickup_item)),
            _ => None,
        };
        color.map_or(-1, |color| color as i32)
    }
    pub fn is_fireball(&self) -> bool {
        matches!(
            self.kind,
            ProjectileKind::SmallFireball
                | ProjectileKind::LargeFireball { .. }
                | ProjectileKind::DragonFireball
                | ProjectileKind::WitherSkull { .. }
        )
    }
    /// `EntityRenderer.getBlockLightLevel` for a projectile: the renderer's
    /// own override wins (`ShulkerBulletRenderer` returns 15, as the fireball
    /// renderers do), and the base class draws anything `isOnFire` at 15
    /// whatever the level says. `None` keeps the level's block light; the sky
    /// light is never overridden.
    pub fn block_light_override(&self) -> Option<u8> {
        (self.is_fireball()
            || self.on_fire()
            || matches!(self.kind, ProjectileKind::ShulkerBullet { .. }))
        .then_some(15)
    }
    /// `EntityRenderer.getPackedLightCoords` lights an entity from the block
    /// its `getLightProbePosition` is in, which is `getEyePosition`: the eye
    /// height of its box over `getPosition(partialTick)`, so the frame's
    /// position rather than this tick's, and not the cell its feet rest in.
    pub fn light_probe_y(&self, partial: f64) -> f64 {
        self.previous.y + (self.position.y - self.previous.y) * partial + self.kind.eye_height()
    }
    pub fn water_bubbles(&self) -> Option<([f64; 3], [f64; 3])> {
        if !self.touching_water {
            return None;
        }
        // ThrowableProjectile.tick applies gravity, then applyInertia reads
        // that movement. AbstractHurtingProjectile.applyInertia reads the
        // movement it already has: fireballs, wither skulls and wind charges
        // have no gravity step before the bubbles.
        let hurting = matches!(
            self.kind,
            ProjectileKind::SmallFireball
                | ProjectileKind::LargeFireball { .. }
                | ProjectileKind::DragonFireball
                | ProjectileKind::WitherSkull { .. }
                | ProjectileKind::WindCharge
                | ProjectileKind::BreezeWindCharge
        );
        let throwable = matches!(
            self.kind,
            ProjectileKind::Snowball
                | ProjectileKind::Egg
                | ProjectileKind::EnderPearl
                | ProjectileKind::Potion(..)
                | ProjectileKind::ExperienceBottle
                | ProjectileKind::Thrown(_)
        );
        let movement = if hurting {
            [self.velocity.x, self.velocity.y, self.velocity.z]
        } else if throwable {
            [
                self.velocity.x,
                self.velocity.y - self.get_gravity(),
                self.velocity.z,
            ]
        } else {
            return None;
        };
        let at = [self.position.x, self.position.y, self.position.z];
        Some(water_bubble_at(at, movement))
    }
    /// Projectile.shoot: normalized direction plus triangular noise, times speed.
    pub fn shoot(
        &mut self,
        random: &mut impl RandomSource,
        dir: [f64; 3],
        speed: f32,
        uncertainty: f32,
    ) {
        let dir = Vec3::new(dir[0], dir[1], dir[2]).normalize();
        let spread = 0.017_227_5 * f64::from(uncertainty);
        let v = [
            (dir.x + random.triangle(0.0, spread)) * f64::from(speed),
            (dir.y + random.triangle(0.0, spread)) * f64::from(speed),
            (dir.z + random.triangle(0.0, spread)) * f64::from(speed),
        ];
        self.velocity = Vec3::new(v[0], v[1], v[2]);
        self.needs_sync = true;
        self.update_rotation();
    }
    /// The UUID a loaded or summoned projectile keeps (`Entity.load`): the
    /// tag's, else the one its own random made. The constructor makes that
    /// random either way, so a kind that makes it at the shot makes it here.
    pub fn own_uuid(&mut self, tag_uuid: Option<[i32; 4]>) -> [i32; 4] {
        let own = self.uuid.take().unwrap_or_else(|| new_own_random().1);
        tag_uuid.unwrap_or(own)
    }
    /// `Projectile.shoot` from the projectile's own random, after its UUID's
    /// draws: an arrow's, or for another kind one made now.
    pub fn shoot_own(&mut self, dir: [f64; 3], speed: f32, uncertainty: f32) {
        let mut random = self.random.0.clone();
        self.shoot(&mut random, dir, speed, uncertainty);
        self.random.0 = random;
    }
    /// Projectile.shootFromRotation for player throws, from the projectile's
    /// own random (`shoot_own`).
    pub fn shoot_from_rotation(
        &mut self,
        pitch: f32,
        yaw: f32,
        offset: f32,
        speed: f32,
        uncertainty: f32,
        source_velocity: Vec3,
        source_on_ground: bool,
    ) {
        use sunrise_core::geometry::trig::{DEG_TO_RAD, cos, sin};
        let (p, y, o) = (
            f64::from(pitch * DEG_TO_RAD),
            f64::from(yaw * DEG_TO_RAD),
            f64::from((pitch + offset) * DEG_TO_RAD),
        );
        let dir = [
            f64::from(-sin(y) * cos(p)),
            f64::from(-sin(o)),
            f64::from(cos(y) * cos(p)),
        ];
        self.shoot_own(dir, speed, uncertainty);
        self.velocity = Vec3::new(
            self.velocity.x + source_velocity.x,
            self.velocity.y
                + if source_on_ground {
                    0.0
                } else {
                    source_velocity.y
                },
            self.velocity.z + source_velocity.z,
        );
    }
    /// AbstractHurtingProjectile: movement along the direction at the acceleration power.
    pub fn fireball(
        kind: ProjectileKind,
        position: Vec3,
        owner: Option<Who>,
        dir: [f64; 3],
    ) -> Self {
        let mut p = Self::new(kind, position, owner);
        let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2])
            .sqrt()
            .max(1.0e-9);
        p.velocity = Vec3::new(dir[0] / len * 0.1, dir[1] / len * 0.1, dir[2] / len * 0.1);
        p
    }
    /// `AbstractHurtingProjectile(EntityType, LivingEntity, Vec3, Level)`'s `setRot(owner.getYRot(), owner.getXRot())`.
    pub(crate) fn facing_shooter(mut self, yaw: f32, pitch: f32) -> Self {
        self.yaw = yaw;
        self.pitch = pitch;
        self
    }
    /// `ThrowableItemProjectile.getItem`: typed stack at count 1 for throwables, else `None`.
    pub(crate) fn thrown_item_stack(&self) -> Option<crate::items::ItemStack> {
        let mut stack = match &self.kind {
            ProjectileKind::Potion(stack, _) => (**stack).clone(),
            _ => {
                let default = self.kind.default_thrown_item()?;
                self.thrown_item
                    .as_deref()
                    .cloned()
                    .unwrap_or_else(|| crate::items::ItemStack::new(default, 1))
            }
        };
        stack.count = 1;
        Some(stack)
    }
    /// `ThrowableItemProjectile.setItem`: keeps a decoded stack at count 1 or restores the default.
    pub fn set_thrown_item(&mut self, stack: Option<crate::items::ItemStack>) {
        let Some(default) = self.kind.default_thrown_item() else {
            return;
        };
        let mut stack = match stack {
            Some(stack) if stack.count > 0 && &*stack.item != "minecraft:air" => stack,
            _ => crate::items::ItemStack::new(default, 1),
        };
        stack.count = 1;
        match &mut self.kind {
            ProjectileKind::Potion(slot, _) => {
                **slot = stack;
            }
            _ => {
                if self.kind == ProjectileKind::Egg {
                    self.egg_variant = egg_chicken_variant(&stack);
                }
                self.thrown_item = Some(Box::new(stack));
            }
        }
    }
    /// `ThrownEgg`'s item: what its hatchlings' variant comes from.
    pub fn set_egg_item(&mut self, stack: &crate::items::ItemStack) {
        if self.kind == ProjectileKind::Egg {
            self.set_thrown_item(Some(stack.clone()));
        }
    }
    /// `Fireball.setItem`: the item a fireball is drawn as. An empty stack
    /// puts the default fire charge back, anything else is kept as a single
    /// one (`copyWithCount(1)`); a kind that is no `Fireball` has no item.
    pub fn set_item(&mut self, stack: crate::items::ItemStack) {
        if !self.kind.is_item_fireball() {
            return;
        }
        let mut stack = if stack.count == 0 || &*stack.item == "minecraft:air" {
            crate::items::ItemStack::new("minecraft:fire_charge", 1)
        } else {
            stack
        };
        stack.count = 1;
        self.item = Some(stack);
    }
    /// Faces the projectile along its velocity, as `Projectile.shoot` ends.
    pub fn update_rotation(&mut self) {
        let v = self.velocity;
        use sunrise_core::geometry::trig::atan2_degrees;
        // `Projectile.shoot`: `(float)(Mth.atan2(..) * 57.2957763671875D)`,
        // one multiply by the folded `180.0F / (float)Math.PI`.
        self.yaw = atan2_degrees(v.x, v.z);
        self.pitch = atan2_degrees(v.y, v.horizontal_distance());
        // `Projectile.shoot` ends with `yRotO = getYRot(); xRotO = getXRot()`:
        // a freshly shot projectile interpolates from the rotation it was
        // aimed along, not from where it was created.
        self.previous_yaw = self.yaw;
        self.previous_pitch = self.pitch;
    }
    /// `Entity.setYRot`: keeps any finite yaw and discards non-finite values.
    pub fn set_yaw(&mut self, yaw: f32) {
        if yaw.is_finite() {
            self.yaw = yaw;
        }
    }
    /// `Entity.setXRot`: wraps finite pitch modulo 360 and clamps to `[-90, 90]`.
    pub fn set_pitch(&mut self, pitch: f32) {
        if pitch.is_finite() {
            self.pitch = (pitch % 360.0).clamp(-90.0, 90.0);
        }
    }
    /// `AbstractHurtingProjectile.tick`'s `rotateTowardsMovement(this, 0.2F)`, before the hit is handled, so on a hit tick too.
    fn turn_hurting(&mut self) {
        if self.is_hurting() {
            self.rotate_towards_movement(0.2);
        }
    }
    /// `ProjectileUtil.rotateTowardsMovement`: eases the rotation from the
    /// previous tick's towards the movement by `t`.
    fn rotate_towards_movement(&mut self, t: f32) {
        let v = self.velocity;
        if v.x * v.x + v.y * v.y + v.z * v.z == 0.0 {
            return;
        }
        let horizontal = (v.x * v.x + v.z * v.z).sqrt();
        let y_rot = sunrise_core::geometry::trig::atan2_degrees(v.z, v.x) + 90.0;
        let x_rot = sunrise_core::geometry::trig::atan2_degrees(horizontal, v.y) - 90.0;
        // A motion whose squared lengths overflow gives no finite target:
        // shifting by whole turns towards it would never end, so the turn is
        // skipped and both rotations stay as they were.
        if !x_rot.is_finite() || !y_rot.is_finite() {
            return;
        }
        self.set_yaw(y_rot);
        self.set_pitch(x_rot);
        // Shift the old angles by whole turns so the lerp takes the shortest
        // way; the shifted old angles are what rendering interpolates from.
        self.previous_pitch = shift_by_turns(self.previous_pitch, self.pitch);
        self.previous_yaw = shift_by_turns(self.previous_yaw, self.yaw);
        self.set_pitch(self.previous_pitch + t * (self.pitch - self.previous_pitch));
        self.set_yaw(self.previous_yaw + t * (self.yaw - self.previous_yaw));
    }
    /// `ShulkerBullet.tick`'s `rotateTowardsMovement(this, 0.5F)`, after the move and any hit, so a bullet a hit leaves alive turns from its new motion.
    pub fn turn_bullet(&mut self) {
        if matches!(self.kind, ProjectileKind::ShulkerBullet { .. }) {
            self.rotate_towards_movement(0.5);
        }
    }
    /// The kinds whose tick turns them with `Projectile.updateRotation`: a
    /// `ThrowableProjectile` (the thrown kinds and the experience bottle) and
    /// llama spit.
    fn turns_with_update_rotation(&self) -> bool {
        self.is_thrown() || self.kind == ProjectileKind::ExperienceBottle
    }
    /// `Projectile.updateRotation`: a fifth of the way (`lerpRotation`) from
    /// the rotation the tick started with toward the motion's direction.
    fn turn_towards_movement(&mut self) {
        use sunrise_core::geometry::trig::atan2_degrees;
        let v = self.velocity;
        let x_rot = atan2_degrees(v.y, v.horizontal_distance());
        let y_rot = atan2_degrees(v.x, v.z);
        self.pitch = crate::entities::arrows::lerp_rotation(self.previous_pitch, x_rot);
        self.yaw = crate::entities::arrows::lerp_rotation(self.previous_yaw, y_rot);
    }
    /// `Entity.setOldPosAndRot`: where the renderer interpolates from.
    pub fn set_old_pos_and_rot(&mut self) {
        self.previous = self.position;
        self.previous_yaw = self.yaw;
        self.previous_pitch = self.pitch;
    }
    /// `Entity.noPhysics` on the arrow part, with the synced flag a client
    /// reads (`AbstractArrow`'s entity-data flags, bit 1): the tracker sends
    /// them again on the next tick. False for a projectile with no arrow part.
    pub fn set_no_physics(&mut self, no_physics: bool) -> bool {
        let Some(arrow) = self.arrow.as_mut() else {
            return false;
        };
        arrow.no_physics = no_physics;
        self.needs_sync = true;
        true
    }
    /// `Entity.move(PISTON)`: the position only; `release_after_piston` follows
    /// (`AbstractArrow.move`).
    pub fn move_by_piston(&mut self, movement: Vec3) {
        self.position = self.position.add_vector(movement);
        self.needs_sync = true;
    }
    pub fn piston_release_bounds(&self) -> Option<Aabb> {
        let arrow = self.arrow.as_ref()?;
        if !arrow.in_ground || arrow.no_physics {
            return None;
        }
        Some(Aabb::new(self.position, self.position).inflate(Vec3::new(0.06, 0.06, 0.06)))
    }
    pub fn release_after_piston(&mut self, no_block_collision: bool) {
        let Some(arrow) = self.arrow.as_mut() else {
            return;
        };
        if arrow.in_ground && !arrow.no_physics && no_block_collision {
            self.velocity = start_falling_velocity(self.velocity, &mut self.random.0);
            arrow.in_ground = false;
            arrow.life = 0;
            self.stuck = false;
            self.needs_sync = true;
        }
    }
    /// `PistonMovingBlockEntity.moveCollidedEntities` on a slime block: the
    /// velocity along the axis. The piston's move after it (`AbstractArrow.move`)
    /// is what takes a stuck arrow out, with `startFalling`'s random damping.
    pub fn piston_slime_bounce(&mut self, movement: Direction) {
        self.velocity = sunrise_world::piston_push::bounce_velocity(movement, self.velocity);
        self.needs_sync = true;
    }
    /// `ShulkerBullet.<init>(level, owner, target, invalidStartAxis)`: a
    /// shulker's bullet at `at`, the centre of its owner's box
    /// (`owner.getBoundingBox().getCenter()`), toward `target`, first leaving
    /// along an axis other than the one it is attached on.
    pub fn shulker_bullet(
        w: &impl ProjectileWorld,
        at: Vec3,
        owner: Who,
        target: Who,
        attach_axis: u8,
        seed: u64,
    ) -> Self {
        let mut p = Self::new(
            ProjectileKind::ShulkerBullet {
                target: Some(target),
                dir: Some(1),
                steps: 0,
                delta: [0.0; 3],
                seed,
            },
            at,
            Some(owner),
        );
        p.select_next_move(w, Some(attach_axis));
        p
    }
    fn bullet_random(&mut self, bound: i32) -> i32 {
        let ProjectileKind::ShulkerBullet { seed, .. } = &mut self.kind else {
            return 0;
        };
        // xorshift64*: the bullet's own random stream.
        *seed ^= *seed >> 12;
        *seed ^= *seed << 25;
        *seed ^= *seed >> 27;
        let v = seed.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33;
        (v % bound.max(1) as u64) as i32
    }
    fn block_pos(&self) -> [i32; 3] {
        [
            self.position.x.floor() as i32,
            self.position.y.floor() as i32,
            self.position.z.floor() as i32,
        ]
    }
    fn target_box(&self, w: &impl ProjectileWorld) -> Option<Aabb> {
        let ProjectileKind::ShulkerBullet {
            target: Some(target),
            ..
        } = self.kind
        else {
            return None;
        };
        w.target_bounds(target)
    }
    /// `ShulkerBullet.selectNextMoveDirection`.
    fn select_next_move(&mut self, w: &impl ProjectileWorld, excluded: Option<u8>) {
        let (target, y_offset) = match self.target_box(w) {
            Some(b) => {
                let (lo, hi) = (b.min_position(), b.max_position());
                let half = (hi.y - lo.y) * 0.5;
                let c = [(lo.x + hi.x) / 2.0, lo.y + half, (lo.z + hi.z) / 2.0];
                (c.map(|v| v.floor() as i32), half)
            }
            None => {
                let p = self.block_pos();
                ([p[0], p[1] - 1, p[2]], 0.5)
            }
        };
        let mut goal = [
            f64::from(target[0]) + 0.5,
            f64::from(target[1]) + y_offset,
            f64::from(target[2]) + 0.5,
        ];
        let mut selection = None;
        let centre = [
            f64::from(target[0]) + 0.5,
            f64::from(target[1]) + 0.5,
            f64::from(target[2]) + 0.5,
        ];
        let d2 = (0..3)
            .map(|i| ([self.position.x, self.position.y, self.position.z][i] - centre[i]).powi(2))
            .sum::<f64>();
        if d2 >= 4.0 {
            let cur = self.block_pos();
            let mut options = Vec::new();
            let step = |d: u8| {
                let s = STEPS[d as usize];
                [cur[0] + s[0], cur[1] + s[1], cur[2] + s[2]]
            };
            for (axis, (lo, hi)) in [(0u8, (4u8, 5u8)), (1, (0, 1)), (2, (2, 3))] {
                if excluded == Some(axis) {
                    continue;
                }
                let i = axis as usize;
                if cur[i] < target[i] && w.empty(step(hi)) {
                    options.push(hi);
                } else if cur[i] > target[i] && w.empty(step(lo)) {
                    options.push(lo);
                }
            }
            let mut pick = self.bullet_random(6) as u8;
            if options.is_empty() {
                let mut i = 5;
                while !w.empty(step(pick)) && i > 0 {
                    pick = self.bullet_random(6) as u8;
                    i -= 1;
                }
            } else {
                pick = options[self.bullet_random(options.len() as i32) as usize];
            }
            let s = STEPS[pick as usize];
            goal = [
                self.position.x + f64::from(s[0]),
                self.position.y + f64::from(s[1]),
                self.position.z + f64::from(s[2]),
            ];
            selection = Some(pick);
        }
        let d = [
            goal[0] - self.position.x,
            goal[1] - self.position.y,
            goal[2] - self.position.z,
        ];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let delta = if len == 0.0 {
            [0.0; 3]
        } else {
            d.map(|v| v / len * 0.15)
        };
        let steps_next = 10 + self.bullet_random(5) * 10;
        if let ProjectileKind::ShulkerBullet {
            dir,
            steps,
            delta: dl,
            ..
        } = &mut self.kind
        {
            *dir = selection;
            *dl = delta;
            *steps = steps_next;
        }
    }
    /// `ShulkerBullet.tick`.
    fn tick_bullet(
        &mut self,
        w: &impl ProjectileWorld,
        impact: bool,
        client_side: bool,
    ) -> (bool, Hit) {
        // Projectile.tick runs before ShulkerBullet's motion and collision sweep.
        if !self.left_owner {
            let swept = self
                .bounds()
                .expand_towards(self.velocity)
                .inflate(Vec3::new(1.0, 1.0, 1.0));
            self.left_owner = self
                .collision_owner()
                .is_none_or(|o| w.outside_owner_collision_range(o, swept));
        }
        // Entity.baseTick: fluid refresh, onFire hurt/countdown, lavaHurt, shared fire bit.
        let from = self.position;
        let pre_bounds = self.bounds();
        let pre_in_water = w.in_water(pre_bounds);
        let pre_in_lava = !self.first_tick && w.in_lava_box(pre_bounds);
        self.touching_water = pre_in_water;
        if pre_in_water {
            self.fall_distance = 0.0;
        }
        self.velocity = w.pushed_by_currents(pre_bounds, self.velocity);
        if !client_side {
            if self.fire_ticks > 0 {
                if self.fire_ticks % 20 == 0 && !pre_in_lava {
                    self.bullet_hurt = Some((from, false));
                    self.destroyed = true;
                }
                self.fire_ticks -= 1;
            }
            if pre_in_lava {
                if self.bullet_hurt.is_none() {
                    self.bullet_hurt = Some((from, true));
                    self.destroyed = true;
                }
                self.fall_distance *= 0.5;
            }
            self.synced_fire = self.fire_ticks > 0 || self.has_visual_fire;
        } else {
            self.clear_fire();
        }
        self.first_tick = false;

        let target = self.target_box(w);
        let ProjectileKind::ShulkerBullet { delta, .. } = &mut self.kind else {
            return (false, Hit::None);
        };
        if target.is_some() {
            for d in delta.iter_mut() {
                *d = (*d * 1.025).clamp(-1.0, 1.0);
            }
            let dl = *delta;
            let v = self.velocity;
            self.velocity = Vec3::new(
                v.x + (dl[0] - v.x) * 0.2,
                v.y + (dl[1] - v.y) * 0.2,
                v.z + (dl[2] - v.z) * 0.2,
            );
        } else {
            // `Entity.applyGravity`: `getGravity` is 0 under NoGravity.
            self.velocity.y -= crate::entity_rules::EntityRules::get_gravity(self);
        }
        let to = from.add_vector(self.velocity);
        let block = w.clip(from, to);
        let end = block.map_or(to, |b| b.0);
        let margin = self.hit_margin();
        let search_area = self
            .bounds()
            .expand_towards(self.velocity)
            .inflate(Vec3::new(1.0, 1.0, 1.0));
        let owner = self.collision_owner();
        let left_owner = self.left_owner;
        let mut nearest = NearestHit::new();
        w.visit_targets(|c| {
            if !left_owner && owner.is_some_and(|o| w.owner_tree_contains(o, c.who)) {
                return;
            }
            nearest.consider(c, search_area, from, end, margin);
        });
        let best = nearest.finish();
        self.position = to;

        // Entity.applyEffectsFromBlocks runs after setPos and before impact, only while alive.
        // Server block contacts (fire, lava, water, cauldrons, powder snow and mod parts) run
        // in `Game::sweep_projectile_inside` before impact settlement.
        if self.bullet_hurt.is_none() {
            self.bubble_columns_along(w, from);
            self.stuck_tick(w, from);
        }

        // Impact is gated on survival after baseTick and swept block effects.
        if impact && self.bullet_hurt.is_none() {
            if let Some((who, at)) = best {
                self.position = at;
                return (false, Hit::Entity { who, at });
            }
            if let Some((at, pos, face)) = block {
                self.position = at;
                return (false, Hit::Block { pos, at, face });
            }
        }
        // Unguarded tick tail: runs even when discarded by fire or lava, including RNG draws.
        self.turn_bullet();
        if let Some(b) = target {
            let (dir, steps) = match self.kind {
                ProjectileKind::ShulkerBullet { dir, steps, .. } => (dir, steps),
                _ => (None, 0),
            };
            if steps > 0 {
                let left = steps - 1;
                if let ProjectileKind::ShulkerBullet { steps, .. } = &mut self.kind {
                    *steps = left;
                }
                if left == 0 {
                    self.select_next_move(w, dir.map(axis_of));
                }
            }
            let dir = match self.kind {
                ProjectileKind::ShulkerBullet { dir, .. } => dir,
                _ => None,
            };
            if let Some(d) = dir {
                let p = self.block_pos();
                let s = STEPS[d as usize];
                let axis = axis_of(d);
                let (lo, hi) = (b.min_position(), b.max_position());
                let t = [
                    ((lo.x + hi.x) / 2.0).floor() as i32,
                    lo.y.floor() as i32,
                    ((lo.z + hi.z) / 2.0).floor() as i32,
                ];
                if w.can_stand_on([p[0] + s[0], p[1] + s[1], p[2] + s[2]], self.position.y)
                    || p[axis as usize] == t[axis as usize]
                {
                    self.select_next_move(w, Some(axis));
                }
            }
        }
        (
            self.bullet_hurt.is_none() && self.position.y >= w.below_world_y(),
            Hit::None,
        )
    }
    /// AbstractHurtingProjectile.applyInertia, on both server and client.
    pub(crate) fn apply_hurting_inertia(&mut self, water: bool) {
        // WitherSkull.getInertia: 0.73 for dangerous skulls.
        // Vanilla returns float inertia and widens it before scaling double motion.
        let inertia = f64::from(match self.kind {
            ProjectileKind::WitherSkull { dangerous: true } => 0.73_f32,
            // AbstractWindCharge.getInertia and getLiquidInertia.
            ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge => 1.0_f32,
            _ if water => 0.8_f32,
            _ => 0.95_f32,
        });
        let v = self.velocity;
        self.velocity = v
            .add_vector(v.normalize().scale(self.acceleration))
            .scale(inertia);
    }

    /// One tick: returns whether to keep the projectile and what it hit.
    pub fn tick(&mut self, w: &impl ProjectileWorld) -> (bool, Hit) {
        sunrise_threads::work::set_entity(sunrise_threads::work::entity_id(self.kind.id()));
        self.tick_with(w, true, false, false)
    }
    /// Server step that lets `LlamaSpit.tick` land an entity hit before resuming its motion.
    pub fn tick_before_hit(&mut self, w: &impl ProjectileWorld) -> (bool, Hit) {
        sunrise_threads::work::set_entity(sunrise_threads::work::entity_id(self.kind.id()));
        self.tick_with(w, true, false, true)
    }
    /// One tick of a server's projectile on its client: [`tick`](Self::tick),
    /// whose bubble columns push it on the client too, so a pearl held in a
    /// stasis chamber hovers between the server's updates instead of sinking
    /// and snapping back.
    pub fn tick_remote(&mut self, w: &impl ProjectileWorld) -> (bool, Hit) {
        self.tick_with(w, true, true, false)
    }
    /// The projectiles whose tick applies the effects of the blocks they moved
    /// through (`applyEffectsFromBlocks`), bubble columns among them:
    /// `ThrowableProjectile`, `AbstractHurtingProjectile` and `ShulkerBullet`.
    /// Arrows push themselves (`arrows::tick`); llama spit applies none.
    pub fn applies_block_effects(&self) -> bool {
        self.is_throwable_projectile()
            || self.is_hurting()
            || matches!(
                self.kind,
                ProjectileKind::ShulkerBullet { .. } | ProjectileKind::ExperienceBottle
            )
    }
    /// A `ThrowableProjectile`: the thrown kinds but llama spit, a plain
    /// `Projectile` with its own tick.
    fn is_throwable_projectile(&self) -> bool {
        self.is_thrown() && self.kind != ProjectileKind::LlamaSpit
    }
    /// `ThrowableProjectile.handleFirstTickBubbleColumn`: a thrown item that
    /// spawns inside a bubble column is pushed by every column block in its box
    /// on its first tick (`findBlocksIn`, the block positions of both corners),
    /// before it has moved through anything.
    fn first_tick_bubble_column(&mut self, w: &impl ProjectileWorld) {
        let b = self.bounds();
        let (lo, hi) = (b.min_position(), b.max_position());
        for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
            for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
                for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                    if let Some((drag, surface)) = w.bubble_column([x, y, z]) {
                        self.bubble_column_push(drag, surface, [x, y, z]);
                    }
                }
            }
        }
    }
    /// `Projectile.onAboveBubbleColumn` / `onInsideBubbleColumn`: unlike
    /// `Entity`'s, a projectile's pushes are not clamped.
    fn bubble_column_push(&mut self, drag: bool, surface: bool, at: [i32; 3]) {
        if surface {
            self.bubble_surfaces.push(at);
        } else {
            // Projectile.onInsideBubbleColumn's resetFallDistance.
            self.fall_distance = 0.0;
            if let Some(arrow) = self.arrow.as_mut() {
                arrow.fall_distance = 0.0;
            }
        }
        self.velocity.y += match (drag, surface) {
            (true, _) => -0.03,
            (false, true) => 0.1,
            (false, false) => 0.06,
        };
    }
    /// `applyEffectsFromBlocks`' bubble columns (`Entity.checkInsideBlocks`):
    /// the blocks the box swept through from `from`, each once, pushing when
    /// the move was long or the final box (deflated by 1e-5F) is inside it.
    pub fn bubble_columns_along(&mut self, w: &impl ProjectileWorld, from: Vec3) {
        self.bubble_columns_since(from, |p| w.bubble_column(p));
    }
    /// [`bubble_columns_along`](Self::bubble_columns_along) with the bubble
    /// columns read by `column`, for a move made by something else: a
    /// piston's push applies the block effects too (`moveEntityByPiston`'s
    /// `applyEffectsFromBlocks`).
    pub fn bubble_columns_since(
        &mut self,
        from: Vec3,
        column: impl Fn([i32; 3]) -> Option<(bool, bool)>,
    ) {
        let to = self.position;
        let eps = f64::from(1.0e-5_f32);
        let deflated = self.bounds().deflate(Vec3::new(eps, eps, eps));
        let far = from.distance_squared(to) > 0.999_990_000_000_252_6f64 * 0.999_990_000_000_252_6;
        let mut cells = Vec::new();
        let mut used = 0;
        let _ = sunrise_core::geometry::sweep_intersected_blocks(
            from,
            to,
            deflated,
            1_000_000,
            |p, step| {
                if step >= 16 {
                    return false;
                }
                used = step;
                cells.push(p);
                true
            },
        );
        // A sweep that used all 16 steps checks the end position once more.
        if used + 1 >= 16 {
            let _ = sunrise_core::geometry::sweep_intersected_blocks(
                to,
                to,
                deflated,
                1_000_000,
                |p, step| {
                    if step >= 1 {
                        return false;
                    }
                    cells.push(p);
                    true
                },
            );
        }
        // Each block once, in the sweep's order: a handful of cells, no set.
        for (i, &p) in cells.iter().enumerate() {
            if cells[..i].contains(&p) {
                continue;
            }
            let Some((drag, surface)) = column([p.x, p.y, p.z]) else {
                continue;
            };
            let cell = Aabb::new(
                Vec3::new(f64::from(p.x), f64::from(p.y), f64::from(p.z)),
                Vec3::new(f64::from(p.x + 1), f64::from(p.y + 1), f64::from(p.z + 1)),
            );
            if far || deflated.intersects(cell) {
                self.bubble_column_push(drag, surface, [p.x, p.y, p.z]);
            }
        }
    }
    /// One tick that meets nothing: what `tick` would have done had its hit been
    /// canceled (`ProjectileImpactEvent`). The projectile goes the whole way of its
    /// motion and flies on, with no hit to act on.
    pub fn tick_through(&mut self, w: &impl ProjectileWorld) -> (bool, Hit) {
        self.tick_with(w, false, false, false)
    }
    fn tick_with(
        &mut self,
        w: &impl ProjectileWorld,
        impact: bool,
        client_side: bool,
        defer_spit_motion: bool,
    ) -> (bool, Hit) {
        let base_first = self.base_first();
        if base_first {
            self.base_fire_tick(client_side);
            self.tick_shoot_event();
        }
        self.bubble_surfaces.clear();
        self.water_splashes.clear();
        // `ThrowableProjectile.tick` begins with `handleFirstTickBubbleColumn`,
        // before its gravity and inertia, so the push takes the tick's drag
        // with it. A server simulates it too, not only a client.
        if self.age == 0 && self.is_throwable_projectile() {
            self.first_tick_bubble_column(w);
        }
        let from = self.position;
        let is_bullet = matches!(self.kind, ProjectileKind::ShulkerBullet { .. });
        let result = self.tick_inner(w, impact, client_side, defer_spit_motion);
        if !is_bullet {
            // A server's block contacts (fire, lava, water, cauldrons, powder snow and
            // parts) are the game's sweep, where the level is: this tick's world view
            // is read-only. Its base tick follows there
            // (`Projectile::finish_block_effects`); a client's comes here.
            if client_side && !base_first {
                self.base_fire_tick(true);
                self.renew_fireball_flame();
            }
            self.stuck_tick(w, from);
        }
        if !base_first {
            self.tick_shoot_event();
        }
        self.first_tick = false;
        // `WindCharge.tick`: counted down after the tick.
        self.no_deflect_ticks = self.no_deflect_ticks.saturating_sub(1);
        result
    }
    /// Completes swept block effects, `checkLeftOwner`, and `Entity.updateFluidInteraction`
    /// after moving to the tick's destination (whether miss, block hit, or entity hit).
    fn finish_post_move(&mut self, w: &impl ProjectileWorld, from: Vec3, owner: Option<Who>) {
        if self.applies_block_effects() {
            self.bubble_columns_along(w, from);
        }
        if !self.left_owner {
            let swept = self
                .bounds()
                .expand_towards(self.velocity)
                .inflate(Vec3::new(1.0, 1.0, 1.0));
            self.left_owner = owner.is_none_or(|o| w.outside_owner_collision_range(o, swept));
        }
        self.update_fluid_interaction(w);
    }
    /// `Entity.updateFluidInteraction`: resets fall distance, runs `doWaterSplashEffect`
    /// (including `Entity.random` draws) on water entry after the first tick, updates
    /// `wasTouchingWater`, then applies fluid currents.
    fn update_fluid_interaction(&mut self, w: &impl ProjectileWorld) {
        let in_water = w.in_water(self.bounds());
        if in_water {
            self.fall_distance = 0.0;
            if !self.touching_water && !self.first_tick {
                let pos = self.position;
                let vel = self.velocity;
                let width = self.kind.size() as f32;
                let mut splashes = Vec::new();
                crate::entities::water_splash_effect(
                    pos,
                    vel,
                    width,
                    &mut self.random.0,
                    &mut |s| {
                        splashes.push(s);
                    },
                );
                self.water_splashes.extend(splashes);
            }
        }
        self.touching_water = in_water;
        self.velocity = w.pushed_by_currents(self.bounds(), self.velocity);
    }
    /// `applyEffectsFromBlocks` after the tick's move, for its cobwebs and
    /// powder snow (`makeStuckInBlock`): the last such block its box (deflated
    /// by 1e-5, `checkInsideBlocks`) met between `from` and where it is now
    /// sets `stuck_speed`, which the next block's push (`Entity.move`) spends.
    fn stuck_tick(&mut self, w: &impl ProjectileWorld, from: Vec3) {
        let end = self.bounds();
        let start = end.move_by(from.subtract(self.position));
        let lo = Vec3::new(
            start.min_position().x.min(end.min_position().x) + 1.0e-5,
            start.min_position().y.min(end.min_position().y) + 1.0e-5,
            start.min_position().z.min(end.min_position().z) + 1.0e-5,
        );
        let hi = Vec3::new(
            start.max_position().x.max(end.max_position().x) - 1.0e-5,
            start.max_position().y.max(end.max_position().y) - 1.0e-5,
            start.max_position().z.max(end.max_position().z) - 1.0e-5,
        );
        for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
            for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
                for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                    if let Some(stuck) = w.stuck_speed([x, y, z]) {
                        self.stuck_speed = stuck;
                    }
                }
            }
        }
    }
    /// The first-tick notification branch of vanilla's base projectile tick.
    /// Arrows own this state in ArrowState and follow their separate tick path.
    fn tick_shoot_event(&mut self) {
        if self.arrow.is_none() && !self.has_been_shot {
            self.has_been_shot = true;
            self.shoot_event = Some(self.position);
        }
    }
    /// Whether fire or lava destroyed this shulker bullet in its pre-move `baseTick`:
    /// `(at, is_lava)`, once.
    pub fn take_bullet_hurt(&mut self) -> Option<(Vec3, bool)> {
        self.bullet_hurt.take()
    }
    /// Drain the first-tick event for dispatch in the owner's game context.
    pub fn take_shoot_event(&mut self) -> Option<Vec3> {
        self.shoot_event.take()
    }
    /// `ProjectileUtil.computeMargin`: how far the boxes of the entities on
    /// its way grow, from nothing for its first two ticks up to 0.3 at its
    /// eighth (`tickCount`, which `age` counts, is raised before the tick).
    fn hit_margin(&self) -> f64 {
        compute_margin(self.age as i32)
    }
    /// `LlamaSpit.tick` resumes with the motion saved before `hitTargetOrDeflectSelf`.
    pub fn finish_spit_motion(&mut self, movement: Vec3) {
        self.turn_towards_movement();
        self.velocity = movement.scale(f64::from(0.99_f32));
        let gravity = self.get_gravity();
        if gravity != 0.0 {
            self.velocity = self.velocity.add_vector(Vec3::new(0.0, -gravity, 0.0));
        }
        self.position = self.position.add_vector(movement);
    }
    fn tick_inner(
        &mut self,
        w: &impl ProjectileWorld,
        impact: bool,
        client_side: bool,
        defer_spit_motion: bool,
    ) -> (bool, Hit) {
        // `Entity.setOldPosAndRot` (the previous position and rotation the
        // renderer interpolates from), kept across the `impact` flag's arrival
        // from #6279.
        self.set_old_pos_and_rot();
        self.age += 1;
        if matches!(self.kind, ProjectileKind::ShulkerBullet { .. }) {
            return self.tick_bullet(w, impact, client_side);
        }
        // ThrowableProjectile and AbstractHurtingProjectile.applyInertia read
        // `isInWater` before their move and `Projectile.tick`'s base tick;
        // LlamaSpit.tick runs that base tick first, at the spit's box here.
        let water = if self.kind == ProjectileKind::LlamaSpit {
            w.in_water(self.bounds())
        } else {
            self.touching_water
        };
        match self.kind {
            ProjectileKind::SmallFireball
            | ProjectileKind::LargeFireball { .. }
            | ProjectileKind::DragonFireball
            | ProjectileKind::WitherSkull { .. }
            | ProjectileKind::WindCharge
            | ProjectileKind::BreezeWindCharge => {
                self.apply_hurting_inertia(water);
            }
            // Arrows and tridents tick with `arrows::tick`.
            ProjectileKind::Arrow { .. } | ProjectileKind::Trident => {}
            // LlamaSpit.tick: gone in water; drag and gravity follow the move.
            ProjectileKind::LlamaSpit => {
                if water {
                    return (false, Hit::None);
                }
            }
            _ => {
                // ThrowableProjectile: gravity, then inertia. Its drag is a
                // float in vanilla (0.8F in water, `getAirDrag` 0.99F), so
                // widens to the double that scales the motion.
                let gravity = self.get_gravity();
                if gravity != 0.0 {
                    self.velocity = self.velocity.add_vector(Vec3::new(0.0, -gravity, 0.0));
                }
                let inertia = f64::from(if water { 0.8_f32 } else { 0.99_f32 });
                self.velocity = Vec3::new(
                    self.velocity.x * inertia,
                    self.velocity.y * inertia,
                    self.velocity.z * inertia,
                );
            }
        }
        // ThrowableProjectile.tick turns before it moves. LlamaSpit.tick
        // turns after its hit, when it resumes its motion.
        if self.turns_with_update_rotation() && self.kind != ProjectileKind::LlamaSpit {
            self.turn_towards_movement();
        }
        // LlamaSpit.tick runs super.tick() (checkLeftOwner) before getHitResultOnMoveVector.
        if self.kind == ProjectileKind::LlamaSpit && !self.left_owner {
            let swept = self
                .bounds()
                .expand_towards(self.velocity)
                .inflate(Vec3::new(1.0, 1.0, 1.0));
            self.left_owner = self
                .collision_owner()
                .is_none_or(|o| w.outside_owner_collision_range(o, swept));
        }
        let from = self.position;
        let to = Vec3::new(
            from.x + self.velocity.x,
            from.y + self.velocity.y,
            from.z + self.velocity.z,
        );
        let block = w.clip(from, to);
        let end = block.map_or(to, |b| b.0);
        // ProjectileUtil.getEntityHitResult: uninflated broad phase, inflated exact clip.
        let margin = self.hit_margin();
        let search_area = self
            .bounds()
            .expand_towards(self.velocity)
            .inflate(Vec3::new(1.0, 1.0, 1.0));
        let owner = self.collision_owner();
        let left_owner = self.left_owner;
        let mut nearest = NearestHit::new();
        w.visit_targets(|c| {
            if !left_owner && owner.is_some_and(|o| w.owner_tree_contains(o, c.who)) {
                return;
            }
            nearest.consider(c, search_area, from, end, margin);
        });
        let best = nearest.finish();
        // DragonFireball.onHit does nothing for its own owner (`ownedBy`): the
        // selected hit is not replaced by what lies behind it, nothing happens,
        // and the fireball keeps its motion. AbstractHurtingProjectile.tick
        // still moves it to the hit's location, as for any hit that is not a
        // MISS, so it stops there this tick.
        if impact
            && self.kind == ProjectileKind::DragonFireball
            && let Some((who, at)) = best
            && self.owner == Some(who)
        {
            self.position = at;
            self.turn_hurting();
            self.finish_post_move(w, from, owner);
            let alive = self.position.y >= w.below_world_y() && self.age < 1200;
            return (alive, Hit::None);
        }
        // LlamaSpit never discards on an entity: `onHitEntity` only hurts, so
        // the spit flies on through what it hit (its block hit discards).
        let spit = self.kind == ProjectileKind::LlamaSpit;
        let spit_hit = best
            .filter(|_| impact && spit)
            .map_or(Hit::None, |(who, at)| Hit::Entity { who, at });
        if impact
            && !spit
            && let Some((who, at)) = best
        {
            self.position = at;
            self.turn_hurting();
            self.finish_post_move(w, from, owner);
            return (false, Hit::Entity { who, at });
        }
        if impact
            && !(spit && best.is_some())
            && let Some((at, pos, face)) = block
        {
            self.position = at;
            self.turn_hurting();
            self.finish_post_move(w, from, owner);
            return (false, Hit::Block { pos, at, face });
        }
        if self.kind == ProjectileKind::LlamaSpit {
            // The 0.25 box touching anything but air discards the spit.
            let h = 0.125;
            let touches = [-h, h].iter().any(|dx| {
                [-h, h].iter().any(|dy| {
                    [-h, h].iter().any(|dz| {
                        !w.empty([
                            (from.x + dx).floor() as i32,
                            (from.y + dy + h).floor() as i32,
                            (from.z + dz).floor() as i32,
                        ])
                    })
                })
            });
            if touches {
                return (false, Hit::None);
            }
            if !defer_spit_motion || spit_hit == Hit::None {
                self.finish_spit_motion(self.velocity);
            }
            return (to.y >= w.below_world_y() && self.age < 6000, spit_hit);
        }
        self.position = to;
        self.turn_hurting();
        self.finish_post_move(w, from, owner);
        let alive = self.position.y >= w.below_world_y()
            && self.age < if self.is_fireball() { 1200 } else { 6000 };
        (alive, Hit::None)
    }
    /// AbstractArrow.onHitEntity damage: ceil(speed * base) plus crit bonus.
    pub fn arrow_damage(&mut self) -> f32 {
        let ProjectileKind::Arrow { base_damage, .. } = self.kind else {
            return 0.0;
        };
        self.arrow_damage_with_base(base_damage)
    }
    /// `AbstractArrow.onHitEntity` pre-pierce damage: `ceil(clamp(pow * base, 0, i32::MAX))`.
    pub fn arrow_damage_rounded(&self, base_damage: f64) -> i32 {
        let v = self.velocity;
        let speed = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt() as f32;
        (f64::from(speed) * base_damage)
            .clamp(0.0, f64::from(i32::MAX))
            .ceil() as i32
    }
    /// `AbstractArrow.onHitEntity` critical bonus draw, run after the pierce budget check.
    pub fn arrow_apply_crit(&mut self, mut damage: i32) -> f32 {
        let ProjectileKind::Arrow { critical, .. } = self.kind else {
            return damage as f32;
        };
        if critical {
            let bonus = self.random.0.next_int_bound(damage / 2 + 2).unwrap_or(0);
            damage = damage.saturating_add(bonus);
        }
        damage as f32
    }
    /// Finish an arrow hit after enchantments transform its base damage, before
    /// speed scaling, rounding and the projectile's critical random draw.
    pub fn arrow_damage_with_base(&mut self, base_damage: f64) -> f32 {
        if !matches!(self.kind, ProjectileKind::Arrow { .. }) {
            return 0.0;
        }
        let damage = self.arrow_damage_rounded(base_damage);
        self.arrow_apply_crit(damage)
    }
    /// the item a thrown potion or a mod's type is drawn as with its model
    /// (`ThrownItemRenderer`): the potion stack itself, or the type's item.
    pub fn drawn_item(&self) -> Option<crate::items::ItemStack> {
        match &self.kind {
            ProjectileKind::Potion(stack, _) => Some((**stack).clone()),
            ProjectileKind::ExperienceBottle => Some(crate::items::ItemStack::new(
                "minecraft:experience_bottle",
                1,
            )),
            ProjectileKind::Thrown(t) => Some(crate::items::ItemStack::new(t.item, 1)),
            _ => None,
        }
    }
    /// The item sprite drawn for thrown projectiles (ThrownItemRenderer).
    pub fn item_sprite(&self) -> Option<&'static str> {
        Some(match &self.kind {
            ProjectileKind::Snowball => "item/snowball",
            ProjectileKind::Egg => "item/egg",
            ProjectileKind::EnderPearl => "item/ender_pearl",
            ProjectileKind::SmallFireball | ProjectileKind::LargeFireball { .. } => {
                "item/fire_charge"
            }
            ProjectileKind::Potion(stack, _) => potion_sprite(&stack.item),
            ProjectileKind::ExperienceBottle => "item/experience_bottle",
            ProjectileKind::LlamaSpit => "entity/llama/llama_spit",
            // Drawn as `WindChargeModel` and `ShulkerBulletModel` by the game,
            // a dragon fireball as `DragonFireballRenderer`'s billboard.
            ProjectileKind::DragonFireball
            | ProjectileKind::WindCharge
            | ProjectileKind::BreezeWindCharge
            | ProjectileKind::ShulkerBullet { .. } => return None,
            ProjectileKind::WitherSkull { dangerous: false } => "entity/wither/wither",
            ProjectileKind::WitherSkull { dangerous: true } => "entity/wither/wither_invulnerable",
            // Drawn as their models by the game.
            ProjectileKind::Arrow { .. } | ProjectileKind::Trident | ProjectileKind::Thrown(_) => {
                return None;
            }
        })
    }
    /// `ThrownItemRenderer`'s scale for the kind: 3 for a large fireball,
    /// 0.75 for a small one, 1 for the rest.
    pub fn sprite_scale(&self) -> f32 {
        match self.kind {
            ProjectileKind::LargeFireball { .. } => 3.0,
            ProjectileKind::SmallFireball => 0.75,
            _ => 1.0,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn egg_variant_honors_explicit_removal() {
        let mut blue = crate::items::ItemStack::new("minecraft:blue_egg", 1);
        assert_eq!(
            egg_chicken_variant(&blue).as_deref(),
            Some("minecraft:cold")
        );
        let mut data = sunrise_items::item_data::StackData::default();
        data.set_other(
            "!minecraft:chicken/variant",
            Some(sunrise_core::nbt::Tag::Compound(Default::default())),
        );
        blue.extra = Some(Box::new(data));
        assert_eq!(egg_chicken_variant(&blue), None);
    }

    struct Empty(Vec<(Who, Aabb)>);
    impl ProjectileWorld for Empty {
        fn clip(&self, _: Vec3, _: Vec3) -> Option<(Vec3, [i32; 3], [i32; 3])> {
            None
        }
        fn in_water(&self, _: Aabb) -> bool {
            false
        }
        fn targets(&self) -> &[(Who, Aabb)] {
            &self.0
        }
    }
    /// Water filled with one kind of bubble column, as in a stasis chamber.
    struct Column {
        drag: bool,
        surface: bool,
    }
    impl ProjectileWorld for Column {
        fn clip(&self, _: Vec3, _: Vec3) -> Option<(Vec3, [i32; 3], [i32; 3])> {
            None
        }
        fn in_water(&self, _: Aabb) -> bool {
            true
        }
        fn targets(&self) -> &[(Who, Aabb)] {
            &[]
        }
        fn bubble_column(&self, _: [i32; 3]) -> Option<(bool, bool)> {
            Some((self.drag, self.surface))
        }
    }
    /// A pearl resting in the water of a column, past its first tick.
    fn resting_pearl() -> Projectile {
        let mut p = Projectile::new(ProjectileKind::EnderPearl, Vec3::new(0.5, 100.2, 0.5), None);
        p.age = 5;
        p.touching_water = true;
        p
    }
    #[test]
    fn a_pearl_in_a_bubble_column_is_pushed_after_it_moves() {
        // ThrowableProjectile.tick: gravity 0.03, the water drag 0.8F, the
        // move, then applyEffectsFromBlocks' BubbleColumnBlock.entityInside:
        // Projectile.onInsideBubbleColumn adds 0.06 (-0.03 dragging down) and
        // onAboveBubbleColumn 0.1 at the surface, with no clamp.
        let drag = f64::from(0.8_f32);
        for (column, push) in [
            (
                Column {
                    drag: false,
                    surface: false,
                },
                0.06,
            ),
            (
                Column {
                    drag: false,
                    surface: true,
                },
                0.1,
            ),
            (
                Column {
                    drag: true,
                    surface: false,
                },
                -0.03,
            ),
            (
                Column {
                    drag: true,
                    surface: true,
                },
                -0.03,
            ),
        ] {
            let mut p = resting_pearl();
            p.tick_remote(&column);
            assert_eq!(p.position.y, 100.2 + -0.03 * drag, "the move comes first");
            assert_eq!(
                p.velocity.y,
                -0.03 * drag + push,
                "drag {} surface {}",
                column.drag,
                column.surface
            );
        }
    }
    #[test]
    fn a_new_pearl_takes_its_first_bubble_column_push_before_gravity() {
        // ThrowableProjectile.handleFirstTickBubbleColumn on the first tick,
        // before gravity, then the air drag 0.99F (not yet touching water),
        // then the push after the move.
        let mut p = Projectile::new(ProjectileKind::EnderPearl, Vec3::new(0.5, 100.2, 0.5), None);
        p.tick_remote(&Column {
            drag: false,
            surface: false,
        });
        let air = f64::from(0.99_f32);
        assert_eq!(p.velocity.y, (0.06 - 0.03) * air + 0.06);
    }
    /// `ThrowableProjectile.handleFirstTickBubbleColumn` on a server: the tick
    /// a snowball is shot in pushes it by the column it spawned in, before its
    /// gravity and the air drag, and only on that first tick. The client path
    /// (`tick_remote`) had it and the host's did not, so a throwable in a bubble
    /// column rose for a guest and sank for the host. Every tick then ends with
    /// `applyEffectsFromBlocks`, whose columns push it again after the move.
    #[test]
    fn a_throwable_spawned_in_a_bubble_column_is_pushed_on_its_first_tick() {
        let air = f64::from(0.99_f32);
        for (column, push) in [
            (
                Column {
                    drag: false,
                    surface: false,
                },
                0.06,
            ),
            (
                Column {
                    drag: false,
                    surface: true,
                },
                0.1,
            ),
            (
                Column {
                    drag: true,
                    surface: false,
                },
                -0.03,
            ),
        ] {
            let mut p = Projectile::new(ProjectileKind::Snowball, Vec3::new(0.5, 100.5, 0.5), None);
            p.tick(&column);
            assert_eq!(
                p.velocity.y,
                (push - 0.03) * air + push,
                "drag {} surface {}",
                column.drag,
                column.surface
            );
            p.velocity = Vec3::ZERO;
            p.tick(&column);
            // The box is in the column's water after the first move, so the
            // drag is `getWaterDrag`'s 0.8F; the first tick's own scan is not
            // run again, only the push of the blocks the move went through.
            assert_eq!(
                p.velocity.y,
                -0.03 * f64::from(0.8_f32) + push,
                "drag {} surface {}",
                column.drag,
                column.surface
            );
        }
        // A kind that is no `ThrowableProjectile` has no first-tick scan: a
        // hurting projectile is pushed only by the blocks its move went through
        // (`AbstractHurtingProjectile.tick`'s `applyEffectsFromBlocks`): its
        // 1-block box spans two column blocks.
        let mut p = Projectile::new(
            ProjectileKind::LargeFireball { power: 1 },
            Vec3::new(0.5, 100.5, 0.5),
            None,
        );
        p.tick(&Column {
            drag: false,
            surface: false,
        });
        assert_eq!(p.velocity.y, 0.12);
    }
    #[test]
    fn a_pearl_outside_any_bubble_column_only_falls() {
        let mut p = resting_pearl();
        p.tick_remote(&Empty(vec![]));
        assert_eq!(p.velocity.y, -0.03 * f64::from(0.8_f32));
    }
    #[test]
    fn piston_move_releases_a_stuck_arrow_with_start_falling_velocity() {
        let mut p = Projectile::new(
            ProjectileKind::Arrow {
                base_damage: 2.0,
                critical: false,
                effect: None,
            },
            Vec3::new(0.5, 100.0, 0.5),
            None,
        );
        p.velocity = Vec3::new(1.0, -2.0, 3.0);
        p.stuck = true;
        p.arrow.as_mut().unwrap().in_ground = true;
        let mut random = p.random.0.clone();
        let expected = start_falling_velocity(p.velocity, &mut random);

        p.move_by_piston(Vec3::new(0.5, 0.0, 0.0));
        assert!(p.piston_release_bounds().is_some());
        p.release_after_piston(false);
        assert!(p.arrow.as_ref().unwrap().in_ground);
        assert_eq!(p.velocity, Vec3::new(1.0, -2.0, 3.0));

        p.release_after_piston(true);
        let arrow = p.arrow.as_ref().unwrap();
        assert!(!arrow.in_ground);
        assert_eq!(p.velocity, expected);
        assert!(!p.stuck);
        assert!(p.needs_sync);
        assert!(p.piston_release_bounds().is_none());
    }

    #[test]
    fn first_tick_shoot_event_uses_base_tick_position_and_saved_flag() {
        let w = Empty(vec![]);
        for kind in [ProjectileKind::Snowball, ProjectileKind::LlamaSpit] {
            let start = Vec3::new(0.0, 100.0, 0.0);
            let mut p = Projectile::new(kind.clone(), start, None);
            p.velocity = Vec3::new(1.0, 0.0, 0.0);
            p.tick(&w);
            let expected = if kind == ProjectileKind::LlamaSpit {
                start
            } else {
                p.position
            };
            assert!(p.has_been_shot);
            assert_eq!(p.take_shoot_event(), Some(expected));
            assert_eq!(p.take_shoot_event(), None);
            p.tick(&w);
            assert_eq!(p.take_shoot_event(), None);
            let mut saved = Projectile::new(kind, start, None);
            saved.has_been_shot = true;
            saved.tick(&w);
            assert_eq!(saved.take_shoot_event(), None);
        }
    }

    #[test]
    fn snowball_falls_with_throwable_gravity_and_drag() {
        let mut p = Projectile::new(ProjectileKind::Snowball, Vec3::new(0.0, 100.0, 0.0), None);
        p.velocity = Vec3::new(1.0, 0.0, 0.0);
        let w = Empty(vec![]);
        p.tick(&w);
        // ThrowableProjectile: gravity before inertia, then move.
        // `getAirDrag` is 0.99F, widened to double: 0.9900000095367432.
        assert_eq!(p.velocity.y.to_bits(), 0xbf9e_69ad_47ae_147b);
        assert_eq!(p.position.x.to_bits(), 0x3fef_ae14_8000_0000);
    }
    /// A server's tick as the game runs it: the move, then the tail of the
    /// block sweep (`Projectile::finish_block_effects`), with no block
    /// touched and no rain.
    fn server_tick(p: &mut Projectile, w: &impl ProjectileWorld) -> Option<f32> {
        let (fire_before, was_on_fire) = (p.fire_ticks, p.burning());
        p.tick(w);
        p.finish_block_effects(fire_before, was_on_fire, false)
    }
    /// A throwable a fire block lit for `BaseFireBlock.fireIgnite`'s 160 ticks
    /// takes `Entity.baseTick`'s countdown after its block effects, the same
    /// tick, and burns down out of the flames.
    #[test]
    fn a_lit_snowball_counts_its_fire_down_after_its_block_effects() {
        let mut p = Projectile::new(ProjectileKind::Snowball, Vec3::new(0.5, 99.5, 0.5), None);
        p.velocity = Vec3::ZERO;
        let air = Empty(vec![]);
        let (fire_before, was_on_fire) = (p.fire_ticks, p.burning());
        p.tick(&air);
        // The sweep's fire block.
        p.ignite_for_ticks(160);
        assert_eq!(
            p.finish_block_effects(fire_before, was_on_fire, false),
            None
        );
        assert_eq!(p.fire_ticks, 159);
        assert!(p.synced_fire);
        assert_eq!(server_tick(&mut p, &air), None);
        assert_eq!(p.fire_ticks, 158);
        for _ in 0..157 {
            server_tick(&mut p, &air);
        }
        assert_eq!(p.fire_ticks, 1);
        // The last tick puts it out in baseTick, after the sound's check.
        assert_eq!(server_tick(&mut p, &air), None);
        assert_eq!(p.fire_ticks, 0, "the fire runs out");
        assert!(!p.on_fire());
    }
    /// `Entity.applyEffectsFromBlocks`: rain clears the fire, which plays
    /// `playEntityOnFireExtinguishedSound`, and the count resets to zero.
    #[test]
    fn rain_puts_a_burning_snowball_out_with_the_extinguish_pitch() {
        let mut p = Projectile::new(ProjectileKind::Snowball, Vec3::new(0.5, 99.5, 0.5), None);
        p.velocity = Vec3::ZERO;
        p.fire_ticks = 100;
        let (fire_before, was_on_fire) = (p.fire_ticks, p.burning());
        p.tick(&Empty(vec![]));
        let pitch = p
            .finish_block_effects(fire_before, was_on_fire, true)
            .expect("it was on fire");
        assert!((1.2..=2.0).contains(&pitch), "{pitch}");
        assert_eq!(p.fire_ticks, 0);
        assert!(!p.synced_fire);
    }
    /// `AbstractHurtingProjectile.tick` renews the fire after the base
    /// countdown, without shortening a longer fire.
    #[test]
    fn a_fireball_renews_its_fire_after_the_base_countdown() {
        let mut p = Projectile::new(
            ProjectileKind::SmallFireball,
            Vec3::new(0.5, 99.5, 0.5),
            None,
        );
        let air = Empty(vec![]);
        server_tick(&mut p, &air);
        assert_eq!(p.fire_ticks, 20);
        assert!(
            !p.synced_fire,
            "the first renewal comes after the shared fire bit"
        );
        server_tick(&mut p, &air);
        assert_eq!(p.fire_ticks, 20);
        assert!(p.synced_fire);
        let (fire_before, was_on_fire) = (p.fire_ticks, p.burning());
        p.tick(&air);
        p.ignite_for_ticks(160);
        p.finish_block_effects(fire_before, was_on_fire, false);
        assert_eq!(p.fire_ticks, 159);
        server_tick(&mut p, &air);
        assert_eq!(p.fire_ticks, 158);
    }

    #[test]
    fn visual_fire_stays_on_when_a_pearls_fire_ticks_expire() {
        let mut server = Projectile::new(ProjectileKind::EnderPearl, Vec3::ZERO, None);
        server.fire_ticks = 1;
        server.has_visual_fire = true;
        let air = Empty(vec![]);
        server_tick(&mut server, &air);
        assert_eq!(server.fire_ticks, 0);
        assert!(server.synced_fire);

        let mut client = Projectile::new(ProjectileKind::EnderPearl, Vec3::ZERO, None);
        client.synced_fire = server.synced_fire;
        client.tick_remote(&air);
        assert!(client.on_fire());
    }

    /// `Entity.baseTick` on a client: `clearFire`. Its flame is the synced
    /// flag the server sends, never a count of its own.
    #[test]
    fn a_clients_projectile_clears_its_own_fire_count() {
        let air = Empty(vec![]);
        for kind in [ProjectileKind::Snowball, ProjectileKind::LlamaSpit] {
            let mut client = Projectile::new(kind, Vec3::new(0.5, 99.5, 0.5), None);
            client.fire_ticks = 100;
            client.tick_remote(&air);
            assert_eq!(client.fire_ticks, 0, "{:?}", client.kind);
            assert!(!client.on_fire(), "{:?}", client.kind);
            client.synced_fire = true;
            client.tick_remote(&air);
            assert!(client.on_fire(), "{:?}", client.kind);
        }
    }

    #[test]
    fn restored_wind_charge_fire_counts_down_and_syncs_visual_fire() {
        for kind in [ProjectileKind::WindCharge, ProjectileKind::BreezeWindCharge] {
            let mut p = Projectile::new(kind, Vec3::ZERO, None);
            p.fire_ticks = 20;
            p.has_visual_fire = true;
            server_tick(&mut p, &Empty(vec![]));
            assert_eq!(p.fire_ticks, 19, "{:?}", p.kind);
            assert!(p.synced_fire, "{:?}", p.kind);
        }
    }

    #[test]
    fn throwables_in_water_leave_bubbles_behind_them() {
        let mut p = Projectile::new(ProjectileKind::Snowball, Vec3::new(0.5, 60.0, 0.5), None);
        p.velocity = Vec3::new(1.0, 0.5, -2.0);
        assert_eq!(p.water_bubbles(), None);
        p.touching_water = true;
        let (at, speed) = p.water_bubbles().expect("a snowball in water");
        let expected_at = [0.25, 60.0 - 0.47 * 0.25, 1.0];
        let expected_speed = [1.0, 0.47, -2.0];
        for i in 0..3 {
            assert!((at[i] - expected_at[i]).abs() < 1e-12, "{at:?}");
            assert!((speed[i] - expected_speed[i]).abs() < 1e-12, "{speed:?}");
        }
        let mut bottle = Projectile::new(
            ProjectileKind::ExperienceBottle,
            Vec3::new(0.5, 60.0, 0.5),
            None,
        );
        bottle.touching_water = true;
        let (_, speed) = bottle.water_bubbles().expect("a bottle in water");
        assert!((speed[1] + 0.07).abs() < 1e-12, "{speed:?}");
        for kind in [
            ProjectileKind::Arrow {
                base_damage: 2.0,
                critical: false,
                effect: None,
            },
            ProjectileKind::LlamaSpit,
        ] {
            let mut other = Projectile::new(kind, Vec3::new(0.5, 60.0, 0.5), None);
            other.touching_water = true;
            assert_eq!(other.water_bubbles(), None);
        }
    }
    /// `AbstractHurtingProjectile.applyInertia`: four bubbles on the motion
    /// the projectile already has, with no gravity taken off first.
    #[test]
    fn hurting_projectiles_in_water_bubble_along_their_motion() {
        let expected_at = [0.25, 59.875, 1.0];
        let expected_speed = [1.0, 0.5, -2.0];
        for kind in [
            ProjectileKind::SmallFireball,
            ProjectileKind::LargeFireball { power: 1 },
            ProjectileKind::WitherSkull { dangerous: true },
            ProjectileKind::WindCharge,
            ProjectileKind::BreezeWindCharge,
        ] {
            let label = format!("{kind:?}");
            let mut p = Projectile::new(kind, Vec3::new(0.5, 60.0, 0.5), None);
            p.velocity = Vec3::new(1.0, 0.5, -2.0);
            assert_eq!(p.water_bubbles(), None, "{label} in air leaves no trail");
            p.touching_water = true;
            let (at, speed) = p.water_bubbles().expect("bubbles in water");
            for i in 0..3 {
                assert!((at[i] - expected_at[i]).abs() < 1e-12, "{label} {at:?}");
                assert!(
                    (speed[i] - expected_speed[i]).abs() < 1e-12,
                    "{label} {speed:?}"
                );
            }
        }
    }
    // Controls added for the DragonFireball owner-hit adapter
    // (`DragonFireball.onHit` ignores its own owner, `ownedBy`): a fireball
    // that has left its owner and flies through the owner's box, with another
    // mob and a wall at x = 1 behind it.
    fn dragon_fireball_through(owner: Who) -> (Projectile, (bool, Hit)) {
        let mut p = Projectile::new(
            ProjectileKind::DragonFireball,
            Vec3::new(0.0, 100.5, 0.5),
            Some(owner),
        );
        p.velocity = Vec3::new(0.5, 0.0, 0.0);
        p.left_owner = true;
        let boxed = |x: f64| Aabb::new(Vec3::new(x, 100.0, 0.0), Vec3::new(x + 0.2, 101.0, 1.0));
        let w = Wall(vec![(Who::Mob(7), boxed(0.4)), (Who::Mob(8), boxed(0.8))]);
        let result = p.tick(&w);
        (p, result)
    }

    #[test]
    fn control_a_dragon_fireball_stops_at_its_owner_and_flies_on() {
        let (p, (keep, hit)) = dragon_fireball_through(Who::Mob(7));
        assert!(keep, "no discard");
        assert_eq!(hit, Hit::None, "no impact");
        // AbstractHurtingProjectile.tick: the position is the owner hit's
        // location (the owner's box, at x = 0.4, less the margin), not the free
        // step of (0.5 + 0.1) × 0.95F; the motion is kept for the next tick.
        assert!(
            p.position.x > 0.0 && p.position.x <= 0.4,
            "{:?}",
            p.position
        );
        assert_eq!((p.position.y, p.position.z), (100.5, 0.5));
        assert!(p.velocity.x > 0.5, "{:?}", p.velocity);
    }

    #[test]
    fn control_a_dragon_fireball_hits_an_entity_that_is_not_its_owner() {
        let (_, (keep, hit)) = dragon_fireball_through(Who::Mob(9));
        assert!(!keep);
        assert!(
            matches!(
                hit,
                Hit::Entity {
                    who: Who::Mob(7),
                    ..
                }
            ),
            "{hit:?}"
        );
    }

    #[test]
    fn control_other_kinds_still_hit_their_owner_once_they_left_it() {
        let mut p = Projectile::new(
            ProjectileKind::LargeFireball { power: 1 },
            Vec3::new(0.0, 100.5, 0.5),
            Some(Who::Mob(7)),
        );
        p.velocity = Vec3::new(0.5, 0.0, 0.0);
        p.left_owner = true;
        let boxed = Aabb::new(Vec3::new(0.4, 100.0, 0.0), Vec3::new(0.6, 101.0, 1.0));
        let (keep, hit) = p.tick(&Wall(vec![(Who::Mob(7), boxed)]));
        assert!(!keep);
        assert!(matches!(
            hit,
            Hit::Entity {
                who: Who::Mob(7),
                ..
            }
        ));
    }

    /// A wall of blocks across x = 1.
    struct Wall(Vec<(Who, Aabb)>);
    impl ProjectileWorld for Wall {
        fn clip(&self, from: Vec3, to: Vec3) -> Option<(Vec3, [i32; 3], [i32; 3])> {
            (from.x < 1.0 && to.x >= 1.0)
                .then(|| (Vec3::new(1.0, from.y, from.z), [1, 100, 0], [-1, 0, 0]))
        }
        fn in_water(&self, _: Aabb) -> bool {
            false
        }
        fn targets(&self) -> &[(Who, Aabb)] {
            &self.0
        }
    }
    /// Open air inside a border from x, z = -4.5 to 5.5 (`/worldborder set
    /// 10` centred on 0.5, 0.5).
    const BORDER: [f64; 4] = [-4.5, -4.5, 5.5, 5.5];
    /// `WorldBorder.clampVec3ToBound`'s far-edge inset, the float 1.0E-5F as
    /// the class file holds it.
    const INSET: f64 = 9.999999747378752E-6;
    struct Bordered(Vec<(Who, Aabb)>);
    impl ProjectileWorld for Bordered {
        fn clip(&self, from: Vec3, to: Vec3) -> Option<(Vec3, [i32; 3], [i32; 3])> {
            clip_including_border(None, from, to, BORDER)
        }
        fn in_water(&self, _: Aabb) -> bool {
            false
        }
        fn targets(&self) -> &[(Who, Aabb)] {
            &self.0
        }
    }
    #[test]
    fn a_pearl_thrown_at_the_world_border_stops_on_it() {
        let mut p = Projectile::new(
            ProjectileKind::EnderPearl,
            Vec3::new(3.5, 100.0, 0.5),
            Some(Who::Player),
        );
        (p.left_owner, p.velocity) = (true, Vec3::new(3.0, 0.0, 0.0));
        let (keep, hit) = p.tick(&Bordered(vec![]));
        // It would end at x = 3.5 + 3 * 0.99F = 6.47, past maxX 5.5: the hit
        // is on the border, 1.0E-5F inside it, facing east (+x), at the
        // height it reached (100 - 0.03 * 0.99F).
        let at = Vec3::new(5.5 - INSET, 100.0 - 0.03 * f64::from(0.99_f32), 0.5);
        assert!(!keep, "ThrownEnderpearl.onHit discards the pearl");
        let Hit::Block {
            pos,
            at: hit_at,
            face,
        } = hit
        else {
            panic!("no border hit: {hit:?}");
        };
        assert!((hit_at.x - at.x).abs() < 1e-12, "{hit_at:?}");
        assert!((hit_at.y - at.y).abs() < 1e-9, "{hit_at:?}");
        assert_eq!(hit_at.z, 0.5);
        assert_eq!((pos, face), ([5, 99, 0], [1, 0, 0]));
        assert_eq!(p.position, hit_at, "the pearl stays where it hit");
        // It teleports its owner to where it started that tick (oldPosition),
        // inside the border.
        assert_eq!(p.previous, Vec3::new(3.5, 100.0, 0.5));
    }
    #[test]
    fn the_border_clamps_each_axis_and_faces_the_travel() {
        // Out across the near edge of z: minZ is kept as is, facing north.
        let from = Vec3::new(0.5, 70.0, -4.0);
        let hit = clip_including_border(None, from, Vec3::new(0.7, 70.0, -6.0), BORDER);
        assert_eq!(
            hit,
            Some((Vec3::new(0.7, 70.0, -4.5), [0, 70, -5], [0, 0, -1]))
        );
        // Ending on maxX itself is outside (isWithinBounds: x < maxX).
        let hit = clip_including_border(None, from, Vec3::new(5.5, 70.0, -4.0), BORDER)
            .expect("a border hit");
        assert_eq!(hit.0.x, 5.5 - INSET);
        assert_eq!(hit.2, [1, 0, 0]);
    }
    #[test]
    fn the_border_leaves_inner_hits_and_rays_from_outside_alone() {
        let inside = Vec3::new(0.5, 70.0, 0.5);
        // A miss that stays inside.
        assert_eq!(
            clip_including_border(None, inside, Vec3::new(5.0, 70.0, 5.0), BORDER),
            None
        );
        // A block struck before the border.
        let block = Some((Vec3::new(2.0, 70.0, 0.5), [2, 70, 0], [-1, 0, 0]));
        assert_eq!(
            clip_including_border(block, inside, Vec3::new(9.0, 70.0, 0.5), BORDER),
            block
        );
        // A ray that starts outside the border is not stopped by it.
        let outside = Vec3::new(7.0, 70.0, 0.5);
        assert_eq!(
            clip_including_border(None, outside, Vec3::new(9.0, 70.0, 0.5), BORDER),
            None
        );
    }
    fn thrown(kind: ProjectileKind) -> Projectile {
        let mut p = Projectile::new(kind, Vec3::new(0.0, 100.0, 0.0), Some(Who::Player));
        (p.left_owner, p.velocity) = (true, Vec3::new(1.5, 0.0, 0.0));
        p
    }
    #[test]
    fn a_tick_through_a_wall_flies_the_whole_way_and_hits_nothing() {
        for kind in [
            ProjectileKind::Snowball,
            ProjectileKind::SmallFireball,
            ProjectileKind::LlamaSpit,
        ] {
            let wall = Wall(Vec::new());
            let mut hit = thrown(kind.clone());
            let (keep, hits) = hit.tick(&wall);
            assert!(
                !keep
                    && matches!(
                        hits,
                        Hit::Block {
                            pos: [1, 100, 0],
                            ..
                        }
                    )
            );
            assert_eq!(hit.position.x, 1.0);
            // The same tick, canceled: on, as in open air.
            let mut through = thrown(kind.clone());
            let mut open = thrown(kind);
            let (kept, none) = through.tick_through(&wall);
            assert_eq!(none, Hit::None);
            let _ = open.tick(&Empty(Vec::new()));
            assert_eq!(
                (through.position, through.velocity),
                (open.position, open.velocity)
            );
            assert!(kept);
        }
    }
    /// Vanilla 26.3: `AbstractHurtingProjectile.tick` turns from pre-impact, pre-fluid motion,
    /// normalizes via `Vec3::normalize()`, runs swept block effects and `updateFluidInteraction`
    /// (currents and post-first-tick water-splash RNG) before hit dispatch, guards repeat Breeze
    /// deflection with `last_deflected_by` while halving dragon-fireball acceleration, and loads
    /// saved `Rotation` via `Entity.load` / `setXRot` semantics.
    #[test]
    #[ignore = "run by tools/test_fireball_hit_turn_parity.py"]
    fn a_fireball_turns_on_the_tick_it_hits() {
        use crate::entity_nbt::{fireball_from_nbt, fireball_to_nbt};
        use crate::save::{doubles, floats};
        use std::collections::{BTreeMap, BTreeSet};
        use sunrise_core::{nbt::Tag, random::LegacyRandom};

        #[derive(serde::Deserialize)]
        struct WitnessDoc {
            game_version: String,
            cases: Vec<WitnessCase>,
        }
        #[derive(serde::Deserialize)]
        struct WitnessCase {
            case: String,
            kind: String,
            scenario: String,
            seed: i64,
            loaded_rot_bits: [String; 2],
            ticks: Vec<WitnessTick>,
        }
        #[derive(serde::Deserialize)]
        struct WitnessTick {
            tick: usize,
            pos_bits: [String; 3],
            vel_bits: [String; 3],
            rot_bits: [String; 2],
            old_rot_bits: [String; 2],
            accel_bits: String,
            alive: bool,
            in_water: bool,
            rng_state: u64,
            sounds: Vec<String>,
        }
        struct Scene {
            targets: Vec<(Who, Aabb)>,
            stone: Option<[i32; 3]>,
            water: Option<(Aabb, Vec3)>,
            bubble: Option<[i32; 3]>,
        }
        impl ProjectileWorld for Scene {
            fn clip(&self, from: Vec3, to: Vec3) -> Option<(Vec3, [i32; 3], [i32; 3])> {
                let p = self.stone?;
                let hit = Aabb::clip_boxes(
                    [Aabb::new(Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0))],
                    from,
                    to,
                    BlockPos::new(p[0], p[1], p[2]),
                )?;
                Some((hit.location, p, hit.direction.normal()))
            }
            fn in_water(&self, bounds: Aabb) -> bool {
                let Some((water, _)) = self.water else {
                    return false;
                };
                let deflated = bounds.deflate(Vec3::new(0.001, 0.001, 0.001));
                deflated.intersects(water)
            }
            fn pushed_by_currents(&self, bounds: Aabb, velocity: Vec3) -> Vec3 {
                match self.water {
                    Some((_, push)) if self.in_water(bounds) => velocity.add_vector(push),
                    _ => velocity,
                }
            }
            fn targets(&self) -> &[(Who, Aabb)] {
                &self.targets
            }
            fn bubble_column(&self, p: [i32; 3]) -> Option<(bool, bool)> {
                (self.bubble == Some(p)).then_some((false, false))
            }
        }
        struct CaseSpec {
            name: &'static str,
            kind: &'static str,
            start: [f64; 3],
            motion: [f64; 3],
            rotation: [f32; 2],
            accel: f64,
            owner: Option<Who>,
            breezes: Vec<u32>,
            scene: Scene,
            overrides: Vec<(usize, Vec3, Vec3)>,
        }

        let path = std::env::var("SUNRISE_FIREBALL_HIT_TURN_WITNESS").expect(
            "run through tools/test_fireball_hit_turn_parity.py, which records the witness",
        );
        let witness = std::fs::read_to_string(&path).unwrap_or_else(|err| {
            panic!(
                "failed to read fireball hit-turn witness at {path} (run via tools/test_fireball_hit_turn_parity.py): {err}"
            )
        });
        let doc: WitnessDoc = serde_json::from_str(&witness).expect("valid 26.3 witness JSON");
        assert_eq!(doc.game_version, "26.3", "witness must be pinned to 26.3");

        let mob_box = |x: f64, y: f64, z: f64, w: f32, h: f32| {
            let half = f64::from(w) / 2.0;
            Aabb::new(
                Vec3::new(x - half, y, z - half),
                Vec3::new(x + half, y + f64::from(h), z + half),
            )
        };
        let pig = |x: f64, y: f64, z: f64| mob_box(x, y, z, 0.9, 0.9);
        let breeze = |x: f64, y: f64, z: f64| mob_box(x, y, z, 0.6, 1.77);
        let flowing_water = (
            Aabb::new(
                Vec3::new(8.0, 66.0, 9.0),
                Vec3::new(9.0, 66.0 + 7.0 / 9.0, 10.0),
            ),
            Vec3::new(0.014, 0.0, 0.0),
        );
        let bubble_water = (
            Aabb::new(Vec3::new(8.0, 66.0, 9.0), Vec3::new(9.0, 68.0, 10.0)),
            Vec3::ZERO,
        );
        let below_cutoff = 1.0e-6_f64;
        let at_cutoff = f64::from(1.0e-5_f32);

        let specs = vec![
            CaseSpec {
                name: "dry_entity_hit_axis",
                kind: "small_fireball",
                start: [8.5, 66.25, 8.25],
                motion: [0.0, 0.0, 0.5],
                rotation: [0.0, 0.0],
                accel: 0.1,
                owner: None,
                breezes: vec![],
                scene: Scene {
                    targets: vec![(Who::Mob(7), pig(8.5, 66.0, 9.0))],
                    stone: None,
                    water: None,
                    bubble: None,
                },
                overrides: vec![],
            },
            CaseSpec {
                name: "dry_entity_hit_diag",
                kind: "fireball",
                start: [8.25, 66.25, 8.25],
                motion: [0.25, 0.125, 0.5],
                rotation: [45.0, -10.0],
                accel: 0.1,
                owner: None,
                breezes: vec![],
                scene: Scene {
                    targets: vec![(Who::Mob(7), pig(8.5, 66.0, 9.0))],
                    stone: None,
                    water: None,
                    bubble: None,
                },
                overrides: vec![],
            },
            CaseSpec {
                name: "block_hit_axis",
                kind: "small_fireball",
                start: [8.5, 66.5, 8.75],
                motion: [0.0, 0.0, 0.5],
                rotation: [0.0, 0.0],
                accel: 0.1,
                owner: None,
                breezes: vec![],
                scene: Scene {
                    targets: vec![],
                    stone: Some([8, 66, 9]),
                    water: None,
                    bubble: None,
                },
                overrides: vec![],
            },
            CaseSpec {
                name: "block_hit_diag",
                kind: "wither_skull",
                start: [8.35, 66.4, 8.75],
                motion: [0.25, 0.125, 0.5],
                rotation: [45.0, -10.0],
                accel: 0.1,
                owner: None,
                breezes: vec![],
                scene: Scene {
                    targets: vec![],
                    stone: Some([8, 66, 9]),
                    water: None,
                    bubble: None,
                },
                overrides: vec![],
            },
            CaseSpec {
                name: "miss_axis",
                kind: "small_fireball",
                start: [8.5, 66.5, 8.25],
                motion: [0.0, 0.0, 0.5],
                rotation: [0.0, 0.0],
                accel: 0.1,
                owner: None,
                breezes: vec![],
                scene: Scene {
                    targets: vec![],
                    stone: None,
                    water: None,
                    bubble: None,
                },
                overrides: vec![],
            },
            CaseSpec {
                name: "miss_diag",
                kind: "fireball",
                start: [8.25, 66.5, 8.25],
                motion: [0.25, 0.125, 0.5],
                rotation: [45.0, -10.0],
                accel: 0.1,
                owner: None,
                breezes: vec![],
                scene: Scene {
                    targets: vec![],
                    stone: None,
                    water: None,
                    bubble: None,
                },
                overrides: vec![],
            },
            CaseSpec {
                name: "dragon_owner_axis",
                kind: "dragon_fireball",
                start: [8.5, 66.25, 8.25],
                motion: [0.0, 0.0, 0.5],
                rotation: [0.0, 0.0],
                accel: 0.1,
                owner: Some(Who::Mob(7)),
                breezes: vec![],
                scene: Scene {
                    targets: vec![(Who::Mob(7), pig(8.5, 66.0, 9.0))],
                    stone: None,
                    water: None,
                    bubble: None,
                },
                overrides: vec![],
            },
            CaseSpec {
                name: "dragon_owner_diag",
                kind: "dragon_fireball",
                start: [8.25, 66.25, 8.25],
                motion: [0.25, 0.125, 0.5],
                rotation: [45.0, -10.0],
                accel: 0.1,
                owner: Some(Who::Mob(7)),
                breezes: vec![],
                scene: Scene {
                    targets: vec![(Who::Mob(7), pig(8.5, 66.0, 9.0))],
                    stone: None,
                    water: None,
                    bubble: None,
                },
                overrides: vec![],
            },
            CaseSpec {
                name: "small_breeze_axis",
                kind: "small_fireball",
                start: [8.5, 66.5, 8.25],
                motion: [0.0, 0.0, 0.5],
                rotation: [0.0, 0.0],
                accel: 0.1,
                owner: None,
                breezes: vec![1],
                scene: Scene {
                    targets: vec![(Who::Mob(1), breeze(8.5, 66.0, 9.0))],
                    stone: None,
                    water: None,
                    bubble: None,
                },
                overrides: vec![],
            },
            CaseSpec {
                name: "fireball_breeze_diag",
                kind: "fireball",
                start: [8.25, 66.5, 8.25],
                motion: [0.25, 0.125, 0.5],
                rotation: [45.0, -10.0],
                accel: 0.1,
                owner: None,
                breezes: vec![1],
                scene: Scene {
                    targets: vec![(Who::Mob(1), breeze(8.5, 66.0, 9.0))],
                    stone: None,
                    water: None,
                    bubble: None,
                },
                overrides: vec![],
            },
            CaseSpec {
                name: "wither_skull_breeze_axis",
                kind: "wither_skull",
                start: [8.5, 66.5, 8.25],
                motion: [0.0, 0.0, 0.5],
                rotation: [0.0, 0.0],
                accel: 0.1,
                owner: None,
                breezes: vec![1],
                scene: Scene {
                    targets: vec![(Who::Mob(1), breeze(8.5, 66.0, 9.0))],
                    stone: None,
                    water: None,
                    bubble: None,
                },
                overrides: vec![],
            },
            CaseSpec {
                name: "dragon_breeze_axis",
                kind: "dragon_fireball",
                start: [8.5, 66.5, 8.25],
                motion: [0.0, 0.0, 0.5],
                rotation: [0.0, 0.0],
                accel: 0.1,
                owner: None,
                breezes: vec![1],
                scene: Scene {
                    targets: vec![(Who::Mob(1), breeze(8.5, 66.0, 9.0))],
                    stone: None,
                    water: None,
                    bubble: None,
                },
                overrides: vec![],
            },
            CaseSpec {
                name: "near_zero_below_cutoff_miss",
                kind: "small_fireball",
                start: [8.5, 66.5, 8.5],
                motion: [0.0, 0.0, below_cutoff],
                rotation: [0.0, 0.0],
                accel: -0.1,
                owner: None,
                breezes: vec![],
                scene: Scene {
                    targets: vec![],
                    stone: None,
                    water: None,
                    bubble: None,
                },
                overrides: vec![],
            },
            CaseSpec {
                name: "near_zero_at_cutoff_miss",
                kind: "small_fireball",
                start: [8.5, 66.5, 8.5],
                motion: [0.0, 0.0, at_cutoff],
                rotation: [0.0, 0.0],
                accel: -0.1,
                owner: None,
                breezes: vec![],
                scene: Scene {
                    targets: vec![],
                    stone: None,
                    water: None,
                    bubble: None,
                },
                overrides: vec![],
            },
            CaseSpec {
                name: "near_zero_below_cutoff_entity_hit",
                kind: "small_fireball",
                start: [8.5, 66.5, 8.5499995],
                motion: [0.0, 0.0, below_cutoff],
                rotation: [0.0, 0.0],
                accel: -0.1,
                owner: None,
                breezes: vec![],
                scene: Scene {
                    targets: vec![(Who::Mob(7), pig(8.5, 66.0, 9.0))],
                    stone: None,
                    water: None,
                    bubble: None,
                },
                overrides: vec![],
            },
            CaseSpec {
                name: "near_zero_at_cutoff_entity_hit",
                kind: "small_fireball",
                start: [8.5, 66.5, 8.5],
                motion: [0.0, 0.0, at_cutoff],
                rotation: [0.0, 0.0],
                accel: -0.1,
                owner: None,
                breezes: vec![],
                scene: Scene {
                    targets: vec![(Who::Mob(7), pig(8.5, 66.0, 8.0))],
                    stone: None,
                    water: None,
                    bubble: None,
                },
                overrides: vec![],
            },
            CaseSpec {
                name: "breeze_wet_entry_after_first_tick",
                kind: "small_fireball",
                start: [8.5, 66.25, 8.05],
                motion: [0.0, 0.0, 0.5],
                rotation: [0.0, 0.0],
                accel: 0.1,
                owner: None,
                breezes: vec![1],
                scene: Scene {
                    targets: vec![(Who::Mob(1), breeze(8.5, 66.0, 9.5))],
                    stone: None,
                    water: Some(flowing_water),
                    bubble: None,
                },
                overrides: vec![],
            },
            CaseSpec {
                name: "breeze_wet_first_tick",
                kind: "small_fireball",
                start: [8.5, 66.25, 8.75],
                motion: [0.0, 0.0, 0.5],
                rotation: [0.0, 0.0],
                accel: 0.1,
                owner: None,
                breezes: vec![1],
                scene: Scene {
                    targets: vec![(Who::Mob(1), breeze(8.5, 66.0, 9.5))],
                    stone: None,
                    water: Some(flowing_water),
                    bubble: None,
                },
                overrides: vec![],
            },
            CaseSpec {
                name: "dragon_owner_water_current",
                kind: "dragon_fireball",
                start: [8.5, 66.1, 8.7],
                motion: [0.0, 0.0, 0.5],
                rotation: [0.0, 0.0],
                accel: 0.1,
                owner: Some(Who::Mob(7)),
                breezes: vec![],
                scene: Scene {
                    targets: vec![(Who::Mob(7), pig(8.5, 66.0, 9.6))],
                    stone: None,
                    water: Some(flowing_water),
                    bubble: None,
                },
                overrides: vec![],
            },
            CaseSpec {
                name: "dragon_owner_bubble_column",
                kind: "dragon_fireball",
                start: [8.5, 66.1, 8.7],
                motion: [0.0, 0.0, 0.5],
                rotation: [0.0, 0.0],
                accel: 0.1,
                owner: Some(Who::Mob(7)),
                breezes: vec![],
                scene: Scene {
                    targets: vec![(Who::Mob(7), pig(8.5, 66.0, 9.6))],
                    stone: None,
                    water: Some(bubble_water),
                    bubble: Some([8, 66, 9]),
                },
                overrides: vec![],
            },
            CaseSpec {
                name: "breeze_bubble_column",
                kind: "small_fireball",
                start: [8.5, 66.25, 8.75],
                motion: [0.0, 0.0, 0.5],
                rotation: [0.0, 0.0],
                accel: 0.1,
                owner: None,
                breezes: vec![1],
                scene: Scene {
                    targets: vec![(Who::Mob(1), breeze(8.5, 66.0, 9.5))],
                    stone: None,
                    water: Some(bubble_water),
                    bubble: Some([8, 66, 9]),
                },
                overrides: vec![],
            },
            CaseSpec {
                name: "breeze_repeat_same",
                kind: "small_fireball",
                start: [8.5, 66.5, 8.8],
                motion: [0.0, 0.0, 0.5],
                rotation: [0.0, 0.0],
                accel: 0.1,
                owner: None,
                breezes: vec![1],
                scene: Scene {
                    targets: vec![(Who::Mob(1), breeze(8.5, 66.0, 9.5))],
                    stone: None,
                    water: None,
                    bubble: None,
                },
                overrides: vec![(2, Vec3::new(8.5, 66.5, 8.8), Vec3::new(0.0, 0.0, 0.5))],
            },
            CaseSpec {
                name: "breeze_repeat_different",
                kind: "small_fireball",
                start: [8.5, 66.5, 9.05],
                motion: [0.0, 0.0, 0.5],
                rotation: [0.0, 0.0],
                accel: 0.1,
                owner: None,
                breezes: vec![1, 2],
                scene: Scene {
                    targets: vec![
                        (Who::Mob(1), breeze(8.5, 66.0, 9.5)),
                        (Who::Mob(2), breeze(8.5, 66.0, 8.7)),
                    ],
                    stone: None,
                    water: None,
                    bubble: None,
                },
                overrides: vec![],
            },
            CaseSpec {
                name: "dragon_breeze_repeat_same_then_different",
                kind: "dragon_fireball",
                start: [8.5, 66.5, 9.05],
                motion: [0.0, 0.0, 0.5],
                rotation: [0.0, 0.0],
                accel: 0.1,
                owner: None,
                breezes: vec![1, 2],
                scene: Scene {
                    targets: vec![
                        (Who::Mob(1), breeze(8.5, 66.0, 9.5)),
                        (Who::Mob(2), breeze(8.5, 66.0, 8.7)),
                    ],
                    stone: None,
                    water: None,
                    bubble: None,
                },
                overrides: vec![
                    (2, Vec3::new(8.5, 66.5, 9.05), Vec3::new(0.0, 0.0, 0.5)),
                    (3, Vec3::new(8.5, 66.5, 9.15), Vec3::new(0.0, 0.0, -0.5)),
                ],
            },
            CaseSpec {
                name: "saved_pitch_180_upward_miss",
                kind: "small_fireball",
                start: [8.5, 66.5, 8.5],
                motion: [0.0, 0.5, 0.0],
                rotation: [0.0, 180.0],
                accel: 0.1,
                owner: None,
                breezes: vec![],
                scene: Scene {
                    targets: vec![],
                    stone: None,
                    water: None,
                    bubble: None,
                },
                overrides: vec![],
            },
            CaseSpec {
                name: "saved_pitch_180_upward_entity_hit",
                kind: "small_fireball",
                start: [8.5, 66.7, 8.5],
                motion: [0.0, 0.5, 0.0],
                rotation: [0.0, 180.0],
                accel: 0.1,
                owner: None,
                breezes: vec![],
                scene: Scene {
                    targets: vec![(Who::Mob(7), pig(8.5, 67.0, 8.5))],
                    stone: None,
                    water: None,
                    bubble: None,
                },
                overrides: vec![],
            },
            CaseSpec {
                name: "saved_pitch_180_upward_breeze",
                kind: "small_fireball",
                start: [8.5, 66.7, 8.5],
                motion: [0.0, 0.5, 0.0],
                rotation: [0.0, 180.0],
                accel: 0.1,
                owner: None,
                breezes: vec![1],
                scene: Scene {
                    targets: vec![(Who::Mob(1), breeze(8.5, 67.0, 8.5))],
                    stone: None,
                    water: None,
                    bubble: None,
                },
                overrides: vec![],
            },
            CaseSpec {
                name: "saved_pitch_neg400_upward_miss",
                kind: "small_fireball",
                start: [8.5, 66.5, 8.5],
                motion: [0.0, 0.5, 0.0],
                rotation: [0.0, -400.0],
                accel: 0.1,
                owner: None,
                breezes: vec![],
                scene: Scene {
                    targets: vec![],
                    stone: None,
                    water: None,
                    bubble: None,
                },
                overrides: vec![],
            },
            CaseSpec {
                name: "saved_huge_rotation_wind_charge",
                kind: "wind_charge",
                start: [8.5, 66.5, 8.5],
                motion: [0.6, 0.0, 0.0],
                rotation: [1.0e30, f32::MAX],
                accel: 0.0,
                owner: None,
                breezes: vec![],
                scene: Scene {
                    targets: vec![],
                    stone: None,
                    water: None,
                    bubble: None,
                },
                overrides: vec![],
            },
        ];

        // Reject duplicate, missing, or extra observations in the pinned 26.3 witness.
        let mut by_case: BTreeMap<&str, &WitnessCase> = BTreeMap::new();
        for entry in &doc.cases {
            assert!(
                by_case.insert(entry.case.as_str(), entry).is_none(),
                "duplicate 26.3 witness row: {}",
                entry.case
            );
        }
        let expected_names: BTreeSet<&str> = specs.iter().map(|s| s.name).collect();
        let actual_names: BTreeSet<&str> = by_case.keys().copied().collect();
        assert_eq!(
            actual_names, expected_names,
            "26.3 witness row set mismatch"
        );

        let u64_hex = |s: &str| u64::from_str_radix(s, 16).expect("witness f64 hex");
        let u32_hex = |s: &str| u32::from_str_radix(s, 16).expect("witness f32 hex");
        // Vanilla's `DoubleTag.valueOf` / `FloatTag.valueOf` store `-0.0` as `ZERO` (`+0.0`) on save.
        let nbt_double_bits = |bits: u64| {
            if bits == (-0.0_f64).to_bits() {
                0
            } else {
                bits
            }
        };
        let nbt_float_bits = |bits: u32| {
            if bits == (-0.0_f32).to_bits() {
                0
            } else {
                bits
            }
        };
        let double_bits = |tag: &Tag, key: &str| -> [u64; 3] {
            let Some(Tag::List(values)) = tag.get(key) else {
                panic!("missing {key} list in saved fireball tag");
            };
            let &[Tag::Double(x), Tag::Double(y), Tag::Double(z)] = values.as_slice() else {
                panic!("unexpected {key} elements in saved fireball tag");
            };
            [x, y, z]
        };
        let float_bits = |tag: &Tag, key: &str| -> [u32; 2] {
            let Some(Tag::List(values)) = tag.get(key) else {
                panic!("missing {key} list in saved fireball tag");
            };
            let &[Tag::Float(yaw), Tag::Float(pitch)] = values.as_slice() else {
                panic!("unexpected {key} elements in saved fireball tag");
            };
            [yaw, pitch]
        };

        for spec in &specs {
            let row = by_case[spec.name];
            assert_eq!(row.kind, spec.kind, "{} kind mismatch", spec.name);
            let entity_id = format!("minecraft:{}", spec.kind);
            let mut fields = vec![
                ("id", Tag::String(entity_id.as_str().into())),
                ("Pos", doubles(&spec.start)),
                ("Motion", doubles(&spec.motion)),
                ("Rotation", floats(&spec.rotation)),
                ("acceleration_power", Tag::Double(spec.accel.to_bits())),
            ];
            if spec.owner.is_some() {
                fields.push(("LeftOwner", Tag::Byte(1)));
            }
            let tag = Tag::compound(fields);
            let mut p = fireball_from_nbt(&tag, None)
                .expect("fireball_from_nbt")
                .with_entity_random(LegacyRandom::new(row.seed));
            p.owner = spec.owner;
            let loaded_rot = [
                u32_hex(&row.loaded_rot_bits[0]),
                u32_hex(&row.loaded_rot_bits[1]),
            ];
            assert_eq!(
                [p.yaw.to_bits(), p.pitch.to_bits()],
                loaded_rot,
                "{} ({}) loaded Rotation",
                spec.name,
                row.scenario
            );

            for (idx, exp) in row.ticks.iter().enumerate() {
                assert_eq!(exp.tick, idx + 1, "{} tick index", spec.name);
                if let Some(&(_, pos, vel)) = spec.overrides.iter().find(|o| o.0 == exp.tick) {
                    p.position = pos;
                    p.velocity = vel;
                }
                let (keep, hit) = p.tick(&spec.scene);
                let mut sounds: Vec<&str> = Vec::new();
                if p.water_splashes
                    .iter()
                    .any(|s| matches!(s, crate::entities::ItemSplash::Sound { .. }))
                {
                    sounds.push("minecraft:entity.generic.splash");
                }
                let alive = match hit {
                    Hit::Entity {
                        who: Who::Mob(id), ..
                    } if spec.breezes.contains(&id) => {
                        assert!(
                            !keep,
                            "{} t={} raw tick stops on Breeze hit",
                            spec.name, exp.tick
                        );
                        if p.deflect_by(id, Turn::Reverse, [1.0; 3]) {
                            sounds.push("minecraft:entity.breeze.deflect");
                        }
                        true
                    }
                    Hit::Entity { .. } => {
                        assert!(!keep, "{} t={} terminal entity hit", spec.name, exp.tick);
                        false
                    }
                    Hit::Block { .. } => {
                        assert!(!keep, "{} t={} terminal block hit", spec.name, exp.tick);
                        false
                    }
                    Hit::None => {
                        assert!(
                            keep,
                            "{} t={} non-hit tick stays alive",
                            spec.name, exp.tick
                        );
                        true
                    }
                };
                assert_eq!(
                    alive, exp.alive,
                    "{} ({}) t={} alive",
                    spec.name, row.scenario, exp.tick
                );
                let exp_pos = [
                    u64_hex(&exp.pos_bits[0]),
                    u64_hex(&exp.pos_bits[1]),
                    u64_hex(&exp.pos_bits[2]),
                ];
                let exp_vel = [
                    u64_hex(&exp.vel_bits[0]),
                    u64_hex(&exp.vel_bits[1]),
                    u64_hex(&exp.vel_bits[2]),
                ];
                let exp_rot = [u32_hex(&exp.rot_bits[0]), u32_hex(&exp.rot_bits[1])];
                let exp_old_rot = [u32_hex(&exp.old_rot_bits[0]), u32_hex(&exp.old_rot_bits[1])];
                let exp_accel = u64_hex(&exp.accel_bits);
                assert_eq!(
                    [
                        p.position.x.to_bits(),
                        p.position.y.to_bits(),
                        p.position.z.to_bits(),
                    ],
                    exp_pos,
                    "{} ({}) t={} pos",
                    spec.name,
                    row.scenario,
                    exp.tick
                );
                assert_eq!(
                    [
                        p.velocity.x.to_bits(),
                        p.velocity.y.to_bits(),
                        p.velocity.z.to_bits(),
                    ],
                    exp_vel,
                    "{} ({}) t={} vel",
                    spec.name,
                    row.scenario,
                    exp.tick
                );
                assert_eq!(
                    [p.yaw.to_bits(), p.pitch.to_bits()],
                    exp_rot,
                    "{} ({}) t={} rot",
                    spec.name,
                    row.scenario,
                    exp.tick
                );
                assert_eq!(
                    [p.previous_yaw.to_bits(), p.previous_pitch.to_bits()],
                    exp_old_rot,
                    "{} ({}) t={} old_rot",
                    spec.name,
                    row.scenario,
                    exp.tick
                );
                assert_eq!(
                    p.acceleration.to_bits(),
                    exp_accel,
                    "{} ({}) t={} accel",
                    spec.name,
                    row.scenario,
                    exp.tick
                );
                assert_eq!(
                    p.touching_water, exp.in_water,
                    "{} ({}) t={} in_water",
                    spec.name, row.scenario, exp.tick
                );
                assert_eq!(
                    p.entity_random().state(),
                    exp.rng_state,
                    "{} ({}) t={} rng_state",
                    spec.name,
                    row.scenario,
                    exp.tick
                );
                let exp_sounds: Vec<&str> = exp.sounds.iter().map(String::as_str).collect();
                assert_eq!(
                    sounds, exp_sounds,
                    "{} ({}) t={} sounds",
                    spec.name, row.scenario, exp.tick
                );
                if alive {
                    let saved =
                        fireball_to_nbt(&p, [0; 4], None, None, &crate::items::Items::default())
                            .expect("fireball_to_nbt");
                    assert_eq!(
                        double_bits(&saved, "Pos"),
                        exp_pos.map(nbt_double_bits),
                        "{} t={} saved Pos",
                        spec.name,
                        exp.tick
                    );
                    assert_eq!(
                        double_bits(&saved, "Motion"),
                        exp_vel.map(nbt_double_bits),
                        "{} t={} saved Motion",
                        spec.name,
                        exp.tick
                    );
                    assert_eq!(
                        float_bits(&saved, "Rotation"),
                        exp_rot.map(nbt_float_bits),
                        "{} t={} saved Rotation",
                        spec.name,
                        exp.tick
                    );
                    let Some(&Tag::Double(saved_accel)) = saved.get("acceleration_power") else {
                        panic!("{} missing acceleration_power", spec.name);
                    };
                    assert_eq!(
                        saved_accel, exp_accel,
                        "{} t={} saved acceleration_power",
                        spec.name, exp.tick
                    );
                }
            }
        }
    }
    #[test]
    fn a_tick_through_an_entity_does_not_hit_it() {
        let pig = Aabb::new(Vec3::new(1.0, 99.5, -0.5), Vec3::new(2.0, 100.5, 0.5));
        let world = Empty(vec![(Who::Mob(7), pig)]);
        let mut hit = thrown(ProjectileKind::EnderPearl);
        assert!(matches!(
            hit.tick(&world),
            (
                false,
                Hit::Entity {
                    who: Who::Mob(7),
                    ..
                }
            )
        ));
        let mut through = thrown(ProjectileKind::EnderPearl);
        let (keep, none) = through.tick_through(&world);
        assert!(keep && none == Hit::None);
        // Gravity and drag of the tick, then the whole of its motion.
        assert!((through.position.x - 1.5 * f64::from(0.99_f32)).abs() < 1e-9);
    }
    #[test]
    fn a_snowball_with_no_gravity_stays_where_it_was_summoned() {
        // Entity.getGravity is 0 with NoGravity: at rest, it never moves.
        let start = Vec3::new(0.5, 100.0, 0.5);
        let mut p = Projectile::new(ProjectileKind::Snowball, start, None);
        p.no_gravity = true;
        let w = Empty(vec![]);
        for _ in 0..20 {
            p.tick(&w);
        }
        assert_eq!((p.position, p.velocity), (start, Vec3::ZERO));
    }
    #[test]
    fn a_mod_thrown_type_flies_tick_for_tick_as_a_snowball() {
        static BOLT: ThrownType = ThrownType {
            id: "ruby:bolt",
            item: "ruby:gem",
            damage: 2.0,
            slot: 0,
        };
        let start = Vec3::new(0.0, 100.0, 0.0);
        let mut snowball = Projectile::new(ProjectileKind::Snowball, start, None);
        let mut bolt = Projectile::new(ProjectileKind::Thrown(&BOLT), start, None);
        for p in [&mut snowball, &mut bolt] {
            p.velocity = Vec3::new(0.3, 0.4, -0.2);
        }
        let air = Empty(Vec::new());
        for _ in 0..40 {
            assert_eq!(snowball.tick(&air), bolt.tick(&air));
            assert_eq!(
                (snowball.position, snowball.velocity, snowball.bounds()),
                (bolt.position, bolt.velocity, bolt.bounds())
            );
        }
        assert_eq!(bolt.kind.id(), "ruby:bolt");
        assert_eq!(bolt.drawn_item().unwrap().item.as_ref(), "ruby:gem");
        // thrown at a mob, it hits it and is gone, as a snowball is.
        let mut bolt = Projectile::new(ProjectileKind::Thrown(&BOLT), start, Some(Who::Player));
        (bolt.left_owner, bolt.velocity) = (true, Vec3::new(1.5, 0.0, 0.0));
        let pig = Aabb::new(Vec3::new(1.0, 99.5, -0.5), Vec3::new(2.0, 100.5, 0.5));
        let (keep, hit) = bolt.tick(&Empty(vec![(Who::Mob(7), pig)]));
        assert!(!keep);
        assert!(matches!(
            hit,
            Hit::Entity {
                who: Who::Mob(7),
                ..
            }
        ));
    }

    /// the registry is set once per process: every test registers the same.
    #[test]
    fn mod_thrown_types_are_found_by_id_and_item_once_registered() {
        register_test_types();
        assert!(!register_thrown_types([(
            "ruby:late".into(),
            "ruby:x".into(),
            1.0
        )]));
        let bolt = thrown_type("ruby:bolt").unwrap();
        assert_eq!((bolt.item, bolt.damage), ("ruby:gem", 2.0));
        assert_eq!(thrown_by_item("ruby:gem"), Some(bolt));
        assert!(
            thrown_type("ruby:late").is_none() && thrown_by_item("minecraft:snowball").is_none()
        );
        // its template's tracking settings, not the builder defaults, as a
        // living type's are (#4031): a wolf's range of 10 chunks, a cat's and
        // a parrot's of 8, not 5; the vanilla types' own stay as they are.
        let settings = crate::lan::tracking::settings;
        assert_eq!(settings("ruby:bolt"), settings("minecraft:snowball"));
        for (vanilla, range, kind) in [
            ("minecraft:wolf", 10, "ruby:hound"),
            ("minecraft:cat", 8, "ruby:tabby"),
            ("minecraft:parrot", 8, "ruby:macaw"),
        ] {
            assert_eq!(settings(vanilla), (range, 3, true), "{vanilla}");
            assert_eq!(settings(kind), (range, 3, true), "{kind}");
        }
        assert_eq!(settings("ruby:late"), (5, 3, true));
    }

    /// `ShulkerBullet.tick` with no target: `applyGravity` (0.04, none under
    /// NoGravity), then the move, no drag. 20 ticks from y 110 fall 8.4.
    #[test]
    fn a_shulker_bullet_without_a_target_falls_unless_no_gravity() {
        let bullet = || {
            Projectile::new(
                ProjectileKind::ShulkerBullet {
                    target: None,
                    dir: None,
                    steps: 0,
                    delta: [0.0; 3],
                    seed: 0,
                },
                Vec3::new(0.5, 110.0, 0.5),
                None,
            )
        };
        let w = Empty(vec![]);
        let mut falling = bullet();
        let mut held = bullet();
        held.no_gravity = true;
        for _ in 0..20 {
            falling.tick(&w);
            held.tick(&w);
        }
        assert!(
            (falling.position.y - 101.6).abs() < 1e-6,
            "{}",
            falling.position.y
        );
        assert_eq!(held.position, Vec3::new(0.5, 110.0, 0.5));
    }

    /// Vanilla 26.3 `ShulkerBullet.tick` rotation witness recorded by `tools/test_shulker_bullet_parity.py`.
    #[test]
    #[ignore = "run by tools/test_shulker_bullet_parity.py"]
    fn a_shulker_bullet_turns_halfway_towards_its_motion_26_3() {
        let path = std::env::var("SUNRISE_SHULKER_BULLET_TURN_WITNESS")
            .expect("run through tools/test_shulker_bullet_parity.py, which records the witness");
        let witness = std::fs::read_to_string(&path).unwrap_or_else(|err| {
            panic!(
                "missing shulker bullet turn witness at {path} ({err}); run tools/test_shulker_bullet_parity.py"
            )
        });
        let hex_f64 = |s: &str| f64::from_bits(u64::from_str_radix(s, 16).unwrap());
        let hex_u32 = |s: &str| u32::from_str_radix(s, 16).unwrap();
        let mut cases = 0usize;
        for line in witness.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let cols: Vec<&str> = line.split('\t').collect();
            assert_eq!(cols.len(), 11, "unexpected witness row: {line}");
            let name = cols[0];
            let mut p = Projectile::new(
                ProjectileKind::ShulkerBullet {
                    target: None,
                    dir: None,
                    steps: 0,
                    delta: [0.0; 3],
                    seed: 0,
                },
                Vec3::new(0.5, 110.0, 0.5),
                None,
            );
            p.no_gravity = cols[1] == "1";
            p.velocity = Vec3::new(hex_f64(cols[2]), hex_f64(cols[3]), hex_f64(cols[4]));
            p.yaw = f32::from_bits(hex_u32(cols[5]));
            p.pitch = f32::from_bits(hex_u32(cols[6]));
            p.set_old_pos_and_rot();
            let mut targets = Vec::new();
            if cols[7] != "-" {
                let seed: i64 = cols[7].parse().unwrap();
                p = p.with_entity_random(sunrise_core::random::LegacyRandom::new(seed));
                let at: Vec<f64> = cols[8].split(',').map(hex_f64).collect();
                assert_eq!(at.len(), 3, "{name}: bad breeze coords");
                let half = f64::from(0.6_f32 / 2.0_f32);
                let height = f64::from(1.77_f32);
                targets.push((
                    Who::Mob(99),
                    Aabb::new(
                        Vec3::new(at[0] - half, at[1], at[2] - half),
                        Vec3::new(at[0] + half, at[1] + height, at[2] + half),
                    ),
                ));
            }
            let w = Empty(targets);
            let ticks: usize = cols[9].parse().unwrap();
            let steps: Vec<&str> = cols[10].split(',').collect();
            assert_eq!(steps.len(), ticks, "{name}: step count mismatch");
            for (tick, step) in steps.iter().enumerate() {
                let (_, hit) = p.tick(&w);
                if matches!(hit, Hit::Entity { .. }) {
                    assert!(p.deflect(Turn::Reverse, [1.0; 3], (None, None, None), false));
                    p.turn_bullet();
                }
                let expected: Vec<u32> = step.split(':').map(hex_u32).collect();
                assert_eq!(expected.len(), 4, "{name} tick {}: bad step", tick + 1);
                assert_eq!(
                    [
                        p.yaw.to_bits(),
                        p.pitch.to_bits(),
                        p.previous_yaw.to_bits(),
                        p.previous_pitch.to_bits(),
                    ],
                    [expected[0], expected[1], expected[2], expected[3]],
                    "{name} tick {}",
                    tick + 1
                );
            }
            cases += 1;
        }
        assert!(cases > 0, "witness file has no cases");
    }
    /// `ShulkerBullet.tick`: a bullet turns off its axis before the block
    /// ahead when it could stand on it (`loadedAndEntityCanStandOn`, asked
    /// at the bullet's own height), and flies on toward one it could not.
    #[test]
    fn a_shulker_bullet_turns_before_a_block_it_could_stand_on() {
        struct Ahead(Vec<(Who, Aabb)>, bool);
        impl ProjectileWorld for Ahead {
            fn clip(&self, _: Vec3, _: Vec3) -> Option<(Vec3, [i32; 3], [i32; 3])> {
                None
            }
            fn in_water(&self, _: Aabb) -> bool {
                false
            }
            fn targets(&self) -> &[(Who, Aabb)] {
                &self.0
            }
            fn can_stand_on(&self, p: [i32; 3], entity_y: f64) -> bool {
                self.1 && p == [1, 64, 0] && (64.0..65.0).contains(&entity_y)
            }
        }
        // East and above: with the X axis ruled out, up is the only way on.
        let target = Aabb::new(Vec3::new(20.0, 70.0, 0.0), Vec3::new(21.0, 71.0, 1.0));
        for standable in [false, true] {
            let mut p = Projectile::new(
                ProjectileKind::ShulkerBullet {
                    target: Some(Who::Mob(2)),
                    dir: Some(5),
                    steps: 30,
                    delta: [0.15, 0.0, 0.0],
                    seed: 1,
                },
                Vec3::new(0.5, 64.5, 0.5),
                None,
            );
            p.tick(&Ahead(vec![(Who::Mob(2), target)], standable));
            let ProjectileKind::ShulkerBullet { dir, .. } = p.kind else {
                unreachable!()
            };
            assert_eq!(dir, Some(if standable { 1 } else { 5 }), "{standable}");
        }
    }
    #[test]
    fn llama_spit_moves_first_then_drags_and_falls() {
        let mut p = Projectile::new(ProjectileKind::LlamaSpit, Vec3::new(0.0, 100.0, 0.0), None);
        p.velocity = Vec3::new(1.0, 0.0, 0.0);
        let w = Empty(vec![]);
        let (keep, _) = p.tick(&w);
        assert!(keep);
        assert!((p.position.x - 1.0).abs() < 1e-9);
        assert_eq!(p.velocity.x.to_bits(), 0x3fef_ae14_8000_0000);
        assert!((p.velocity.y + 0.06).abs() < 1e-9);
    }
    #[test]
    fn arrow_damage_is_speed_times_base() {
        let mut p = Projectile::new(
            ProjectileKind::Arrow {
                base_damage: 2.0,
                critical: false,
                effect: None,
            },
            Vec3::new(0.0, 100.0, 0.0),
            None,
        );
        p.velocity = Vec3::new(3.0, 0.0, 0.0);
        // ceil(3 * 2) = 6 for a non-critical arrow.
        assert_eq!(p.arrow_damage(), 6.0);
    }
    #[test]
    fn a_dragon_fireball_turns_towards_its_flight_a_fifth_per_tick() {
        let mut p = Projectile::new(
            ProjectileKind::DragonFireball,
            Vec3::new(8.5, 231.0, 8.5),
            None,
        );
        p.velocity = Vec3::new(0.1, 0.25, -0.3);
        p.yaw = 0.0;
        p.pitch = 0.0;
        for _ in 0..3 {
            p.tick(&Wall(vec![]));
        }
        assert!((p.yaw - 8.996_265).abs() < 4e-6, "yaw {}", p.yaw);
        assert!((p.pitch + 18.704_494).abs() < 4e-6, "pitch {}", p.pitch);
    }
    #[test]
    fn a_fireball_turns_the_short_way_across_the_wrap_boundary() {
        let mut p = Projectile::new(
            ProjectileKind::DragonFireball,
            Vec3::new(8.5, 231.0, 8.5),
            None,
        );
        // Movement along -z aims at yaw 0 and pitch 0.
        p.velocity = Vec3::new(0.0, 0.0, -1.0);
        p.previous_yaw = 350.0;
        p.previous_pitch = 350.0;
        p.rotate_towards_movement(0.2);
        assert!((p.yaw + 8.0).abs() < 1e-3, "yaw {}", p.yaw);
        assert!((p.pitch + 8.0).abs() < 1e-3, "pitch {}", p.pitch);
        assert!((p.previous_yaw + 10.0).abs() < 1e-3);
        assert!((p.previous_pitch + 10.0).abs() < 1e-3);
    }
    #[test]
    fn fireball_accelerates_along_its_direction() {
        let mut p = Projectile::fireball(
            ProjectileKind::SmallFireball,
            Vec3::new(0.0, 100.0, 0.0),
            None,
            [1.0, 0.0, 0.0],
        );
        let w = Empty(vec![]);
        p.tick(&w);
        // Fresh Java 26.3: (0.1 + 0.1) * (double) 0.95f.
        assert_eq!(p.velocity.x, 0.189_999_997_615_814_2);
    }

    /// A mob's fireball starts at its shooter's rotation and only turns towards its motion on its first tick.
    #[test]
    fn a_shot_fireball_starts_facing_its_shooter() {
        let mut p = Projectile::fireball(
            ProjectileKind::SmallFireball,
            Vec3::new(0.0, 100.0, 0.0),
            Some(Who::Mob(7)),
            [1.0, 0.0, 0.0],
        )
        .facing_shooter(-90.0, 0.0);
        assert_eq!((p.yaw, p.pitch), (-90.0, 0.0));
        p.tick(&Empty(vec![]));
        // rotateTowardsMovement(0.2F): yRotO -90 moves to 270, then 270 + 0.2 * (90 - 270).
        assert_eq!((p.yaw, p.pitch), (234.0, -1.525_878_9e-6));
    }

    /// `Fireball.setItem` with `Fireball.defineSynchedData`'s default and
    /// `getItem`: a fireball is drawn as a fire charge until it is given
    /// another item, and it keeps a single one of it. A kind that is no
    /// `Fireball` (a wither skull, a snowball) has no item to set.
    #[test]
    fn a_fireball_holds_one_of_the_item_it_is_given() {
        let held = |p: &Projectile| {
            p.item
                .as_ref()
                .map(|i| (&*i.item, i.count))
                .map(|(i, n)| (i.to_string(), n))
        };
        for kind in [
            ProjectileKind::LargeFireball { power: 1 },
            ProjectileKind::SmallFireball,
        ] {
            let mut p = Projectile::new(kind, Vec3::new(0.0, 100.0, 0.0), None);
            assert_eq!(
                held(&p),
                Some(("minecraft:fire_charge".to_owned(), 1)),
                "the default"
            );
            p.set_item(crate::items::ItemStack::new("minecraft:diamond", 5));
            assert_eq!(
                held(&p),
                Some(("minecraft:diamond".to_owned(), 1)),
                "copyWithCount(1)"
            );
            // `setItem` refuses an empty stack, either way of being empty, and
            // puts the default fire charge back.
            for empty in [
                crate::items::ItemStack::new("minecraft:air", 1),
                crate::items::ItemStack::new("minecraft:diamond", 0),
            ] {
                p.set_item(empty);
                assert_eq!(held(&p), Some(("minecraft:fire_charge".to_owned(), 1)));
            }
        }
        for kind in [
            ProjectileKind::WitherSkull { dangerous: false },
            ProjectileKind::Snowball,
        ] {
            let mut p = Projectile::new(kind, Vec3::new(0.0, 100.0, 0.0), None);
            assert_eq!(p.item, None);
            p.set_item(crate::items::ItemStack::new("minecraft:diamond", 1));
            assert_eq!(p.item, None, "no Fireball, no item");
        }
    }
    /// `ThrownItemRenderer`'s scale per entity type, which the item model a
    /// fireball is drawn as is scaled by: 3 for a fireball, 0.75 for a small
    /// one, 1 for the other kinds and their own renderers.
    #[test]
    fn a_fireball_draws_at_thrice_or_three_quarters() {
        let scale = |kind: ProjectileKind| {
            Projectile::new(kind, Vec3::new(0.0, 100.0, 0.0), None).sprite_scale()
        };
        assert_eq!(scale(ProjectileKind::LargeFireball { power: 1 }), 3.0);
        assert_eq!(scale(ProjectileKind::SmallFireball), 0.75);
        assert_eq!(scale(ProjectileKind::Snowball), 1.0);
    }
    #[test]
    fn fireball_inertia_preserves_custom_acceleration() {
        let mut p = Projectile::fireball(
            ProjectileKind::SmallFireball,
            Vec3::new(0.0, 100.0, 0.0),
            None,
            [1.0, 0.0, 0.0],
        );
        // A redirected projectile can carry a different acceleration power.
        p.acceleration = 0.2;
        p.tick(&Empty(vec![]));
        assert_eq!(p.velocity.x, 0.284_999_996_423_721_37);
        assert_eq!(p.position.x, 0.284_999_996_423_721_37);
    }

    #[test]
    fn hurting_inertia_widens_water_and_dangerous_skull_floats() {
        for (kind, water, expected) in [
            (ProjectileKind::SmallFireball, true, 0.160_000_002_384_185_8),
            (
                ProjectileKind::WitherSkull { dangerous: true },
                false,
                0.146_000_003_814_697_27,
            ),
        ] {
            let mut p = Projectile::new(kind, Vec3::ZERO, None);
            p.velocity = Vec3::new(0.1, 0.0, 0.0);
            p.acceleration = 0.1;
            p.apply_hurting_inertia(water);
            assert_eq!(p.velocity, Vec3::new(expected, 0.0, 0.0));
        }
    }

    #[test]
    fn a_wind_charge_holds_its_speed_until_a_deflection_gives_it_power() {
        let mut p = Projectile::new(ProjectileKind::WindCharge, Vec3::ZERO, None);
        p.velocity = Vec3::new(0.0, 1.5, 0.0);
        p.apply_hurting_inertia(false);
        assert_eq!(p.velocity, Vec3::new(0.0, 1.5, 0.0));
        // Inertia 1 in water too, and the power adds along the movement.
        p.acceleration = 0.1;
        p.apply_hurting_inertia(true);
        assert!((p.velocity.y - 1.6).abs() < 1e-12, "{p:?}");
    }

    #[test]
    fn ender_pearl_hits_a_wind_charge_box() {
        // Through the charge's box itself: a pearl on its first tick has no
        // margin (`ProjectileUtil.computeMargin`) to reach one it passes near.
        let mut pearl = Projectile::new(
            ProjectileKind::EnderPearl,
            Vec3::new(0.0, 1.1, 0.0),
            Some(Who::Player),
        );
        pearl.left_owner = true;
        pearl.velocity = Vec3::new(2.0, 0.0, 0.0);
        let wind = Projectile::new(
            ProjectileKind::WindCharge,
            Vec3::new(0.6, 1.0, 0.0),
            Some(Who::Player),
        );
        let w = Empty(vec![(Who::Mob(1), wind.aabb())]);
        let (keep, hit) = pearl.tick(&w);
        assert!(!keep, "ThrownEnderpearl.onHit discards the pearl");
        assert!(
            matches!(
                hit,
                Hit::Entity {
                    who: Who::Mob(1),
                    ..
                }
            ),
            "pearl collides with a pickable wind charge"
        );
    }
}

#[cfg(test)]
mod trail_tests {
    use super::*;
    use sunrise_level::particle_options::ParticleType;
    fn arrow(critical: bool) -> Projectile {
        let mut p = Projectile::new(
            ProjectileKind::Arrow {
                base_damage: 2.0,
                critical,
                effect: None,
            },
            Vec3::new(1.0, 2.0, 3.0),
            None,
        );
        p.velocity = Vec3::new(4.0, 2.0, -1.0);
        p
    }
    #[test]
    fn critical_arrow_emits_four_native_streak_samples() {
        let trail = arrow(true).arrow_trail(false);
        assert_eq!(trail.len(), 4);
        for (i, (ty, pos, speed)) in trail.iter().enumerate() {
            assert_eq!(*ty, ParticleType::Crit);
            assert_eq!(
                *pos,
                [1.0 + i as f64, 2.0 + i as f64 * 0.5, 3.0 - i as f64 * 0.25]
            );
            assert_eq!(*speed, [-4.0, -1.8, 1.0]);
        }
    }
    #[test]
    fn submerged_arrow_emits_four_bubbles_after_water_inertia() {
        let trail = arrow(false).arrow_trail(true);
        assert_eq!(trail.len(), 4);
        let drag = f64::from(0.6f32);
        for (ty, pos, speed) in trail {
            assert_eq!(ty, ParticleType::Bubble);
            assert_eq!(speed, [4.0 * drag, 2.0 * drag, -drag]);
            assert_eq!(pos, [1.0 - drag, 2.0 - drag * 0.5, 3.0 + drag * 0.25]);
        }
    }
    #[test]
    fn ordinary_and_stuck_arrows_do_not_emit_critical_trails() {
        assert!(arrow(false).arrow_trail(false).is_empty());
        let mut p = arrow(true);
        p.stuck = true;
        assert!(p.arrow_trail(true).is_empty());
    }
}
#[cfg(test)]
mod deflect_tests {
    use super::*;
    use sunrise_core::random::{LegacyRandom, RandomSource};

    struct Empty(Vec<(Who, Aabb)>);
    impl ProjectileWorld for Empty {
        fn clip(&self, _: Vec3, _: Vec3) -> Option<(Vec3, [i32; 3], [i32; 3])> {
            None
        }
        fn in_water(&self, _: Aabb) -> bool {
            false
        }
        fn targets(&self) -> &[(Who, Aabb)] {
            &self.0
        }
    }

    fn none() -> OwnerFields {
        (None, None, None)
    }

    /// `WindCharge.deflect` refuses while `noDeflectTicks` > 0, which counts
    /// down after each of its ticks: five ticks old, it deflects.
    #[test]
    fn a_wind_charge_refuses_a_deflection_for_its_first_five_ticks() {
        let w = Empty(vec![]);
        let mut p = Projectile::new(
            ProjectileKind::WindCharge,
            Vec3::new(0.0, 100.0, 0.0),
            Some(Who::Mob(3)),
        );
        p.velocity = Vec3::new(0.0, 1.0, 0.0);
        assert_eq!(p.no_deflect_ticks, 5);
        for left in (1..=5).rev() {
            let (velocity, random) = (p.velocity, p.random.0.clone());
            assert!(!p.deflect(Turn::Reverse, [1.0; 3], none(), true));
            assert_eq!((p.velocity, p.owner), (velocity, Some(Who::Mob(3))));
            assert_eq!(p.random.0.clone().next_float(), random.clone().next_float());
            assert_eq!(p.no_deflect_ticks, left);
            p.tick(&w);
            p.velocity = velocity;
        }
        assert_eq!(p.no_deflect_ticks, 0);
        assert!(p.deflect(Turn::Reverse, [1.0; 3], none(), true));
        assert_eq!(p.owner, None);
        assert_eq!(p.acceleration, 0.1);
        // A breeze's wind charge, and any other kind, has no such window.
        let breeze = Projectile::new(ProjectileKind::BreezeWindCharge, Vec3::ZERO, None);
        assert_eq!(breeze.no_deflect_ticks, 0);
    }

    /// Two projectiles made at one spot do not share a random stream.
    #[test]
    fn projectiles_at_one_spot_get_independent_randoms() {
        let at = Vec3::new(1.0, 64.0, 2.0);
        let mut a = Projectile::new(ProjectileKind::Snowball, at, None);
        let mut b = Projectile::new(ProjectileKind::Snowball, at, None);
        let (fa, fb): (Vec<f32>, Vec<f32>) = (0..6)
            .map(|_| (a.random.0.next_float(), b.random.0.next_float()))
            .unzip();
        assert_ne!(fa, fb);
        let arrow = ProjectileKind::Arrow {
            base_damage: 2.0,
            critical: false,
            effect: None,
        };
        let mut a = Projectile::new(arrow.clone(), at, None);
        let mut b = Projectile::new(arrow, at, None);
        assert_ne!(
            a.entity_random().next_float(),
            b.entity_random().next_float()
        );
        assert_ne!(
            unique_random(at).next_float(),
            unique_random(at).next_float()
        );
    }

    /// `REVERSE` takes exactly one float from the projectile's `Entity.random`.
    #[test]
    fn reverse_draws_one_float_from_the_projectiles_own_random() {
        let mut ball = Projectile::fireball(
            ProjectileKind::LargeFireball { power: 1 },
            Vec3::ZERO,
            None,
            [1.0, 0.0, 0.0],
        );
        ball.random = OwnRandom(LegacyRandom::new(42));
        let mut expect = LegacyRandom::new(42);
        let draw = expect.next_float();
        let yaw = ball.yaw;
        assert!(ball.deflect(Turn::Reverse, [1.0; 3], none(), false));
        assert_eq!(ball.yaw, yaw + (170.0 + draw * 20.0));
        assert_eq!(ball.previous_yaw, yaw + (170.0 + draw * 20.0));
        assert_eq!(
            ball.random.0.clone().next_float(),
            expect.clone().next_float()
        );
        let draw = expect.next_float();
        let yaw = ball.yaw;
        assert!(ball.deflect(Turn::Reverse, [1.0; 3], none(), false));
        assert_eq!(ball.yaw, yaw + (170.0 + draw * 20.0));
        assert_eq!(ball.random.0.clone().next_float(), expect.next_float());
    }
}
#[cfg(test)]
mod pearl_owner_tests {
    use super::*;

    /// `ThrownEnderpearl.isAllowedToTeleportOwner` in the pearl's own
    /// dimension: only a living, awake owner takes the pearl. Vanilla's
    /// `onHit` discards the pearl (and skips the 5 damage) otherwise.
    #[test]
    fn a_dead_or_sleeping_owner_cannot_take_its_pearl() {
        assert!(owner_can_take_pearl(true, false), "alive and awake");
        assert!(!owner_can_take_pearl(false, false), "a dead owner");
        assert!(!owner_can_take_pearl(true, true), "a sleeping owner");
        assert!(!owner_can_take_pearl(false, true), "dead in bed");
    }
}
#[cfg(test)]
mod render_light_tests {
    use super::*;

    fn shot(kind: ProjectileKind) -> Projectile {
        Projectile::new(kind, Vec3::new(0.5, 64.0, 0.5), None)
    }

    /// `EntityRenderer.getBlockLightLevel` and the two overrides: a shulker
    /// bullet and a fireball are 15 wherever they fly, anything on fire is 15
    /// by the base class, and an ordinary arrow keeps the level's light.
    #[test]
    fn only_the_full_bright_projectiles_override_the_block_light() {
        let bullet = shot(ProjectileKind::ShulkerBullet {
            target: None,
            dir: None,
            steps: 0,
            delta: [0.0; 3],
            seed: 0,
        });
        assert_eq!(bullet.block_light_override(), Some(15));
        assert_eq!(
            shot(ProjectileKind::SmallFireball).block_light_override(),
            Some(15)
        );
        assert_eq!(
            shot(ProjectileKind::WitherSkull { dangerous: false }).block_light_override(),
            Some(15)
        );
        let mut arrow = shot(ProjectileKind::Arrow {
            base_damage: 2.0,
            critical: false,
            effect: None,
        });
        assert_eq!(arrow.block_light_override(), None);
        // `Entity.isOnFire` on any projectile is 15, whatever its kind.
        arrow.ignite_for_ticks(100);
        assert_eq!(arrow.block_light_override(), Some(15));
        let mut snowball = shot(ProjectileKind::Snowball);
        assert_eq!(snowball.block_light_override(), None);
        snowball.ignite_for_ticks(1);
        assert_eq!(snowball.block_light_override(), Some(15));
    }

    /// `Entity.getLightProbePosition` is the eye position, so an arrow lying
    /// in the top eighth of a block is lit by the block above it.
    #[test]
    fn a_projectile_is_lit_from_the_block_its_eye_is_in() {
        let mut arrow = shot(ProjectileKind::Arrow {
            base_damage: 2.0,
            critical: false,
            effect: None,
        });
        arrow.position = Vec3::new(0.5, 64.9, 0.5);
        assert_eq!(arrow.light_probe_y(1.0).floor(), 65.0);
        assert_eq!(arrow.position.y.floor(), 64.0);
        // A wind charge's entity type declares no eye height, so it keeps its feet.
        let mut wind = shot(ProjectileKind::WindCharge);
        wind.position = Vec3::new(0.5, 64.9, 0.5);
        assert_eq!(wind.light_probe_y(1.0), 64.9);
        // A snowball's box is a quarter block: the eye is 0.2125 up.
        let mut ball = shot(ProjectileKind::Snowball);
        ball.position = Vec3::new(0.5, 64.8, 0.5);
        assert!((ball.light_probe_y(1.0) - 64.8 - 0.85 * 0.25).abs() < 1e-6);
    }

    /// `getEyePosition(partialTick)` reads `getPosition(partialTick)`: an arrow
    /// falling from 64 to 65 is probed at 64.63 for the first half of the frame
    /// (the cell at 64) and only reaches the cell at 65 when the frame ends.
    /// Sunrise read the tick's own position, so the light popped a frame early.
    #[test]
    fn a_projectile_probe_follows_the_frame() {
        let mut arrow = shot(ProjectileKind::Arrow {
            base_damage: 2.0,
            critical: false,
            effect: None,
        });
        arrow.previous = Vec3::new(0.5, 64.0, 0.5);
        arrow.position = Vec3::new(0.5, 65.0, 0.5);
        assert!((arrow.light_probe_y(0.0) - 64.13).abs() < 1e-6);
        assert_eq!(arrow.light_probe_y(0.0).floor(), 64.0);
        assert_eq!(arrow.light_probe_y(0.5).floor(), 64.0);
        assert!((arrow.light_probe_y(0.5) - 64.63).abs() < 1e-6);
        assert_eq!(arrow.light_probe_y(1.0).floor(), 65.0);
        // The old probe, this tick's position, was 65.0 for every frame.
        assert_eq!((arrow.position.y + arrow.kind.eye_height()).floor(), 65.0);
    }

    #[test]
    fn projectile_entity_type_parity() {
        let arrow = shot(ProjectileKind::Arrow {
            base_damage: 2.0,
            critical: false,
            effect: None,
        });
        assert_eq!(arrow.entity_type(), "minecraft:arrow");
        let spectral = shot(ProjectileKind::Arrow {
            base_damage: 2.0,
            critical: false,
            effect: Some(("minecraft:glowing", 200)),
        });
        assert_eq!(spectral.entity_type(), "minecraft:spectral_arrow");
        assert_eq!(
            shot(ProjectileKind::SmallFireball).entity_type(),
            "minecraft:small_fireball"
        );
    }
    /// `ThrownEgg.onHit`: the hatchlings take the variant of the item's
    /// `CHICKEN_VARIANT` component (egg temperate, blue cold, brown warm).
    #[test]
    fn thrown_egg_carries_its_chickens_variant() {
        use crate::items::ItemStack;
        let variant = |item: &str| {
            let mut egg = Projectile::new(ProjectileKind::Egg, Vec3::ZERO, None);
            egg.set_egg_item(&ItemStack::new(item, 1));
            egg.egg_variant
        };
        assert_eq!(
            variant("minecraft:egg").as_deref(),
            Some("minecraft:temperate")
        );
        assert_eq!(
            variant("minecraft:brown_egg").as_deref(),
            Some("minecraft:warm")
        );
        assert_eq!(
            variant("minecraft:blue_egg").as_deref(),
            Some("minecraft:cold")
        );
        let mut snowball = Projectile::new(ProjectileKind::Snowball, Vec3::ZERO, None);
        snowball.set_egg_item(&ItemStack::new("minecraft:blue_egg", 1));
        assert_eq!(snowball.egg_variant, None);
    }
}

#[cfg(test)]
mod arrow_color_tests {
    use super::*;
    use crate::items::ItemStack;
    use sunrise_core::nbt::Tag;

    /// `Arrow.updateColor`: `ID_EFFECT_COLOR` is -1 only while the pickup
    /// item's contents are `PotionContents.EMPTY`, and otherwise
    /// `PotionContents.getColor` — so a tipped arrow whose contents have a
    /// `custom_color` or only custom effects, with no potion id at all, still
    /// syncs a colour.
    #[test]
    fn a_tipped_arrows_synced_colour_is_its_whole_contents() {
        let color = |stack: &ItemStack| {
            let kind = ProjectileKind::Arrow {
                base_damage: 2.0,
                critical: false,
                effect: None,
            };
            let mut p = Projectile::new(kind, Vec3::new(0.5, 64.0, 0.5), None);
            let mut arrow = crate::entities::arrows::ArrowState::new();
            arrow.set_item(stack);
            p.arrow = Some(Box::new(arrow));
            p.arrow_effect_color()
        };
        let tipped = || ItemStack::new("minecraft:tipped_arrow", 1);
        assert_eq!(color(&tipped()), -1, "PotionContents.EMPTY has no colour");
        // `PotionContents.getColor`: an explicit `custom_color` wins, as given.
        let mut custom_color = tipped();
        custom_color
            .data_mut()
            .potion_extra
            .push(("custom_color".into(), Tag::Int(0x8012_3456u32 as i32)));
        assert_eq!(color(&custom_color), 0x8012_3456u32 as i32);
        // Else the mix of the visible effects: poison's own colour (0x87A363).
        let mut custom_effect = tipped();
        custom_effect.data_mut().potion_extra.push((
            "custom_effects".into(),
            Tag::List(vec![Tag::compound([
                ("id", Tag::String("minecraft:poison".into())),
                ("duration", Tag::Int(900)),
            ])]),
        ));
        assert_eq!(color(&custom_effect), 0xFF87_A363u32 as i32);
        // `updateColor` reads the pickup stack, whatever arrow item it is.
        let mut plain = ItemStack::new("minecraft:arrow", 1);
        plain
            .data_mut()
            .potion_extra
            .push(("custom_color".into(), Tag::Int(0x8012_3456u32 as i32)));
        assert_eq!(color(&plain), 0x8012_3456u32 as i32);
    }
}
crate::impl_joined!(Projectile);
sunrise_mod_runtime::impl_attachment_lifetime!(Projectile);

#[cfg(test)]
mod enchanted_arrow_order_tests {
    use super::*;
    #[test]
    fn enchanted_base_is_scaled_before_rounding_and_critical_draw() {
        let mut arrow = Projectile::new(
            ProjectileKind::Arrow {
                base_damage: 2.0,
                critical: false,
                effect: None,
            },
            Vec3::ZERO,
            None,
        );
        arrow.velocity = Vec3::new(3.0, 0.0, 0.0);
        // Power I adds 1 to base damage, not to the scaled hit.
        assert_eq!(arrow.arrow_damage_with_base(3.0), 9.0);
        if let ProjectileKind::Arrow { critical, .. } = &mut arrow.kind {
            *critical = true;
        }
        arrow.random.0 = sunrise_core::random::LegacyRandom::new(42);
        // java.util.Random(42): nextInt(6) = 2.
        assert_eq!(arrow.arrow_damage_with_base(3.0), 11.0);
        assert_eq!(arrow.random.0.next_int(), 234785527);
    }
}
