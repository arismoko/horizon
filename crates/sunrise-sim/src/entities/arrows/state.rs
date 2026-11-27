// Included in arrows.rs to retain its private scope.
/// `EntityType.ARROW` (and tipped/spectral arrows): 0.5 x 0.5, eyes at 0.13.
pub const WIDTH: f32 = 0.5;
pub const HEIGHT: f32 = 0.5;
const EYE_HEIGHT: f32 = 0.13;

/// `AbstractArrow.Pickup`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pickup {
    #[default]
    Disallowed,
    Allowed,
    CreativeOnly,
}
impl Pickup {
    /// The saved `pickup` byte (`LEGACY_CODEC`: the ordinal, out of range is
    /// DISALLOWED).
    pub fn from_byte(b: i64) -> Self {
        match b {
            1 => Self::Allowed,
            2 => Self::CreativeOnly,
            _ => Self::Disallowed,
        }
    }
    pub fn byte(self) -> i8 {
        self as i8
    }
}
/// `DataComponents.INTANGIBLE_PROJECTILE`.
pub const INTANGIBLE: &str = "minecraft:intangible_projectile";
/// `ProjectileWeaponItem.useAmmo`'s drawn stack: one of the ammo, marked
/// intangible when no ammo was used (creative, Infinity, multishot extras).
pub fn drawn_ammo(ammo: &crate::items::ItemStack, intangible: bool) -> crate::items::ItemStack {
    let mut drawn = crate::items::ItemStack {
        count: 1,
        ..ammo.clone()
    };
    if intangible {
        drawn.data_mut().set_other(
            INTANGIBLE,
            Some(sunrise_core::nbt::Tag::Compound(Default::default())),
        );
    }
    drawn
}
/// What `AbstractArrow.playerTouch` does for a player touching the arrow.
#[derive(Clone, Debug, PartialEq)]
pub enum Touch {
    /// Not stuck yet, still shaking, or not the player's to take.
    Ignore,
    /// ALLOWED: taken if the inventory takes this stack whole.
    Give(crate::items::ItemStack),
    /// CREATIVE_ONLY in creative: taken without an item.
    Take,
}
/// What an arrow's tick asks the game to play or show.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ArrowEvent {
    /// `level().addParticle` (the client's copy of the arrow adds them).
    Particle {
        kind: ArrowParticle,
        position: Vec3,
        velocity: Vec3,
    },
    /// `Level.broadcastEntityEvent(arrow, 0)`: a tipped arrow stuck for 600
    /// ticks. The clients burst its particles (`Arrow.handleEntityEvent`);
    /// the arrow itself is a plain one from now on.
    PotionPuff {
        id: u32,
        color: u32,
    },
    Sound {
        sound: ArrowSound,
        volume: f32,
        pitch: f32,
    },
    /// `Entity.sendBubbleColumnParticles` at this block: two SPLASH (speed 1)
    /// and two BUBBLE (spread 0, 0.01, 0, speed 0.2) particles at the top,
    /// placed by the level random (drawn by the consumer, nothing else draws
    /// it during an arrow's tick).
    BubbleColumnParticles {
        pos: BlockPos,
    },
    /// The block hit reacts (before the arrow sticks).
    Block {
        pos: BlockPos,
        reaction: BlockReaction,
        /// The arrow's owner (`Projectile.getOwner`), who owns a TNT it primes.
        owner: Option<Who>,
    },
    /// The SPLASH and PROJECTILE_SHOOT game events.
    SplashEvent,
    ShootEvent,
    /// `spawnAtLocation(pickupItem, 0.1)`: a turned-back arrow too slow to fly
    /// on drops what picking it up gives.
    Drop,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrowParticle {
    Bubble,
    Crit,
    Splash,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrowSound {
    /// `entity.generic.splash` (an arrow's splash and high-speed splash).
    Splash,
    /// The arrow's `soundEvent` (`entity.arrow.hit`), for blocks and entities.
    HitGround,
    /// An arrow shot from a crossbow: `item.crossbow.hit`.
    CrossbowHit,
    /// `entity.generic.extinguish_fire`.
    Extinguish,
    /// A thrown trident's `getDefaultHitGroundSoundEvent` (`item.trident.hit_ground`).
    TridentHitGround,
    /// `ThrownTrident.onHitEntity`'s `item.trident.hit`.
    TridentHit,
}
/// What a burning arrow does to the block it hits (`onProjectileHit`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockReaction {
    /// `TntBlock.prime` (primed TNT, its sound and PRIME_FUSE), then
    /// `removeBlock(pos, false)`.
    PrimeTnt,
    /// `setBlock(pos, lit=true, 11)`: campfires and candles.
    Light(BlockStateId),
}
/// Remaining segment and entity hits of a piercing arrow's `stepMoveAndHit` tick.
#[derive(Clone, Debug)]
pub struct PierceStep {
    pub end: Vec3,
    pub block: Option<(Vec3, BlockPos, [i32; 3])>,
    pub remaining_hits: Vec<(Who, Vec3)>,
}

/// The arrow part of `AbstractArrow` and its `Entity` state.
#[derive(Clone)]
pub struct ArrowState {
    pub in_ground: bool,
    pub canceled_impact_inside: bool,
    pub in_ground_time: i32,
    /// Ticks stuck (`life`): gone at 1200.
    pub life: i32,
    pub shake: i32,
    /// The block it stuck in (`lastState`).
    pub last_state: Option<BlockStateId>,
    pub pickup: Pickup,
    /// Its entity type is `SpectralArrow` (the type is its own, not the pickup item's).
    pub spectral: bool,
    /// `SpectralArrow` saved duration; its constructor default is 200.
    pub spectral_duration: i32,
    /// `pickupItemStack`: what picking the arrow up gives.
    pub pickup_item: crate::items::ItemStack,
    /// `AbstractArrow.firedFromWeapon`: the crossbow or bow it was shot from.
    pub fired_from_weapon: Option<crate::items::ItemStack>,
    /// `AbstractArrow.soundEvent` is `item.crossbow.hit` (`CrossbowItem.createProjectile`)
    /// until the first block hit sets it back to `entity.arrow.hit`.
    pub crossbow_hit_sound: bool,
    pub no_physics: bool,
    /// `Arrow.ID_EFFECT_COLOR` as a client has it from its server (-1: none).
    pub effect_color: i32,
    /// `AbstractArrow.getPierceLevel`.
    pub pierce: u8,
    /// `piercingIgnoreEntityIds`: the entities a piercing arrow has gone
    /// through (cleared when it sticks in a block).
    pub pierced: Vec<Who>,
    /// `piercedAndKilledEntities`: living victims killed by this piercing arrow.
    pub pierced_killed: Vec<crate::advancements::EntityView>,
    /// Pending `stepMoveAndHit` continuation for a piercing arrow this tick.
    pub pierce_step: Option<PierceStep>,
    /// `Entity.remainingFireTicks`.
    pub fire_ticks: i32,
    /// The server's shared fire flag, independent of the client-local fire timer.
    pub synced_fire: bool,
    pub fall_distance: f64,
    pub fluids: EntityFluidInteraction,
    pub was_touching_water: bool,
    pub was_eye_in_water: bool,
    pub first_tick: bool,
    pub has_been_shot: bool,
    /// `Projectile.canBreak`: the decoded top-level projectile predicate,
    /// independent of its pickup item and firing weapon.
    pub can_break: Option<sunrise_core::nbt::Tag>,
    /// `ThrownTrident.dealtDamage`: it has hit an entity (or sat in a block
    /// for over 4 ticks) and hits no more.
    pub dealt_damage: bool,
    pub removed: bool,
    pub events: Vec<ArrowEvent>,
    /// The speed (`deltaMovement.length()`) it hit a block at, before it stuck.
    pub hit_speed: f64,
    /// Spawn position when this fresh arrow joined the level before its first tick.
    pub spawn_pos: Option<Vec3>,
    /// Spawn velocity when this fresh arrow joined the level before its first tick.
    pub spawn_velocity: Option<Vec3>,
    /// Spawn rotation (pitch, yaw) when this fresh arrow joined the level before its first tick.
    pub spawn_rot: Option<(f32, f32)>,
}
impl fmt::Debug for ArrowState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArrowState")
            .field("in_ground", &self.in_ground)
            .field("life", &self.life)
            .field("fire_ticks", &self.fire_ticks)
            .field("synced_fire", &self.synced_fire)
            .finish_non_exhaustive()
    }
}
impl PartialEq for ArrowState {
    fn eq(&self, other: &Self) -> bool {
        self.in_ground == other.in_ground
            && self.life == other.life
            && self.last_state == other.last_state
            && self.fire_ticks == other.fire_ticks
            && self.synced_fire == other.synced_fire
    }
}
impl Default for ArrowState {
    fn default() -> Self {
        Self::new()
    }
}
const ARROW_HIT: &str = "minecraft:entity.arrow.hit";
const CROSSBOW_HIT: &str = "minecraft:item.crossbow.hit";

impl ArrowState {
    /// `AbstractArrow.addAdditionalSaveData`'s `SoundEvent`: the crossbow's
    /// until the first block hit, else the loaded one (a trident's own) or
    /// `entity.arrow.hit`.
    pub fn saved_sound_event(&self, loaded: Option<&Tag>) -> Tag {
        let loaded = match loaded {
            Some(Tag::String(s)) => s.to_string_lossy(),
            _ => ARROW_HIT.into(),
        };
        let sound = match (self.crossbow_hit_sound, loaded.as_ref()) {
            (true, _) => CROSSBOW_HIT,
            (false, CROSSBOW_HIT) => ARROW_HIT,
            (false, other) => other,
        };
        Tag::String(sound.into())
    }
    /// `AbstractArrow.readAdditionalSaveData`'s `weapon` and `SoundEvent`.
    pub fn load_weapon_and_sound(
        &mut self,
        tag: &Tag,
        item: impl FnOnce(&Tag) -> Option<crate::items::ItemStack>,
    ) {
        self.fired_from_weapon = tag.get("weapon").and_then(item);
        let saved = tag.get("SoundEvent");
        self.crossbow_hit_sound =
            matches!(saved, Some(Tag::String(s)) if s.to_string_lossy() == CROSSBOW_HIT);
    }
    pub fn new() -> Self {
        Self {
            in_ground: false,
            canceled_impact_inside: false,
            in_ground_time: 0,
            life: 0,
            shake: 0,
            last_state: None,
            pickup: Pickup::Disallowed,
            spectral: false,
            spectral_duration: 200,
            pickup_item: crate::items::ItemStack::new("minecraft:arrow", 1),
            fired_from_weapon: None,
            crossbow_hit_sound: false,
            no_physics: false,
            effect_color: -1,
            pierce: 0,
            pierced: Vec::new(),
            pierced_killed: Vec::new(),
            pierce_step: None,
            fire_ticks: 0,
            synced_fire: false,
            fall_distance: 0.0,
            fluids: EntityFluidInteraction::new([WATER, LAVA]),
            was_touching_water: false,
            was_eye_in_water: false,
            first_tick: true,
            has_been_shot: false,
            can_break: None,
            dealt_damage: false,
            removed: false,
            events: Vec::new(),
            hit_speed: 0.0,
            spawn_pos: None,
            spawn_velocity: None,
            spawn_rot: None,
        }
    }
    /// `Arrow.getColor` of the server's own arrow: the colour of the potion on
    /// its pickup item, None for a plain arrow (`PotionContents.EMPTY`).
    pub fn potion_color(&self) -> Option<u32> {
        (!crate::brewing::contents_empty(&self.pickup_item))
            .then(|| crate::brewing::contents_color(&self.pickup_item))
    }
    /// The `AbstractArrow` constructor's `pickupItemStack`: the drawn ammo
    /// (one item). Ammo drawn without being used up (`INTANGIBLE_PROJECTILE`)
    /// makes the arrow CREATIVE_ONLY; vanilla takes the component off the
    /// stack it was given, not its copy, so the item keeps it.
    pub fn set_item(&mut self, item: &crate::items::ItemStack) {
        self.pickup_item = item.clone();
        if item.data().and_then(|d| d.other(INTANGIBLE)).is_some() {
            self.pickup = Pickup::CreativeOnly;
        }
    }
    /// `AbstractArrow.setOwner`: a player's arrows may be picked up.
    pub fn set_owner(&mut self, owner: Option<Who>) {
        if owner == Some(Who::Player) && self.pickup == Pickup::Disallowed {
            self.pickup = Pickup::Allowed;
        }
    }
    /// `AbstractArrow.playerTouch` and `tryPickup` (`creative`:
    /// `Player.hasInfiniteMaterials`). Like vanilla it does not look at
    /// removal: players only touch arrows still in the world.
    pub fn player_touch(&self, creative: bool) -> Touch {
        if !(self.in_ground || self.no_physics) || self.shake > 0 {
            return Touch::Ignore;
        }
        match self.pickup {
            Pickup::Disallowed => Touch::Ignore,
            Pickup::Allowed => Touch::Give(self.pickup_item.clone()),
            Pickup::CreativeOnly if creative => Touch::Take,
            Pickup::CreativeOnly => Touch::Ignore,
        }
    }
    /// `Entity.isOnFire` (arrows are not fire immune).
    pub fn on_fire(&self) -> bool {
        self.fire_ticks > 0 || self.synced_fire
    }
    /// `Entity.clearFire`.
    pub fn clear_fire(&mut self) {
        self.fire_ticks = self.fire_ticks.min(0);
    }
    /// `Entity.igniteForTicks` (an arrow never freezes).
    pub fn ignite_for_ticks(&mut self, ticks: i32) {
        if self.fire_ticks < ticks {
            self.fire_ticks = ticks;
        }
    }
    /// `Entity.igniteForSeconds`: Flame's `projectile_spawned` ignite.
    pub fn ignite_for_seconds(&mut self, seconds: f32) {
        self.ignite_for_ticks((seconds * 20.0).floor() as i32);
    }
}
