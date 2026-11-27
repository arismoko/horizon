// Types that lived in the API root before the domain split.
use crate::item::item_stack::read_stack_view;
use crate::*;
use sunrise_core::profile::AccountProperty;

/// Why an entity type's attribute declaration or addition was refused
/// ([`Registrar::declare_entity_attributes`](EntityAttributesRegistrar::declare_entity_attributes),
/// [`Registrar::modify_entity_attributes`](EntityAttributesRegistrar::modify_entity_attributes)).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum EntityAttributeError {
    /// The entity type's id is malformed or empty.
    InvalidId,
    /// The set names the same attribute twice.
    DuplicateAttribute,
    /// A base value is not finite.
    NotFinite,
    /// Another declaration already covers the entity type.
    Duplicate,
}

/// Why a mod's attribute type declaration was refused
/// ([`Registrar::declare_attribute`](AttributesRegistrar::declare_attribute)).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum AttributeRegistrationError {
    /// Only both-side packages may change the attribute registry.
    WrongSide,
    /// The attribute's id is malformed or empty.
    InvalidId,
    /// Another declaration or built-in entry already covers the attribute.
    Duplicate,
    /// The default or a bound is not finite.
    NotFinite,
    /// `min > max`, or the default is not inside `[min, max]`
    /// (`RangedAttribute`'s own checks).
    InvalidBounds,
}

/// An effect requested once through [`Registrar::effect_key`](EffectsRegistrar::effect_key). The registry is fixed until restart.
// no wire: decoding validates the u32 slot with EffectKey::from_slot on the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its field is private, so only registration creates a key
pub struct EffectKey(pub(crate) u32);

/// A damage type requested once through [`crate::Registrar::damage_type_key`].
/// The key names the same registry ID that [`crate::HurtSource::damage_type`]
/// reports; a level resolves that name in its current damage-type registry.
// exhaustive: the scalar codec validates host slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, sunrise_mod_api_macros::Wire)]
#[wire(via = u32)]
pub struct DamageTypeKey(pub(crate) u32);

impl From<&DamageTypeKey> for u32 {
    fn from(key: &DamageTypeKey) -> Self {
        key.0
    }
}
impl TryFrom<u32> for DamageTypeKey {
    type Error = &'static str;
    fn try_from(slot: u32) -> Result<Self, Self::Error> {
        let known = crate::parts::registration_membership(
            crate::parts::RegistrationKeyKind::DamageType,
            slot,
        )
        .unwrap_or_else(|| {
            crate::parts::requested_keys()
                .damage_types
                .id(slot)
                .is_some()
        });
        known
            .then_some(Self(slot))
            .ok_or("unknown DamageTypeKey slot")
    }
}

/// An entity type (`minecraft:zombie`) in the built-in entity-type registry, in
/// registration order. Returned by `EntityRef::kind`; Copy/Eq comparisons do not
/// allocate. `key.id(host)` pins its namespaced name, if available, without
/// copying the text. A key stays valid until the game exits.
// no wire: the manual scalar codec validates host slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: incoming scalar slots are checked against the host's registry
pub struct EntityTypeKey(pub(crate) u32);

impl EntityTypeKey {
    /// `EntityType.getCategory()` of this registered type, on either side and with no entity
    /// spawned: `minecraft:zombie` is `Monster`, a mod mob its declared category. See
    /// `KeysHost::entity_type_category`; a live entity's is `EntityRef::classification`.
    pub fn category(self, host: &dyn crate::Host) -> Option<crate::MobCategory> {
        crate::backend_of(host)?.entity_type_category(self)
    }
}

impl From<&EntityTypeKey> for u32 {
    fn from(key: &EntityTypeKey) -> Self {
        key.0
    }
}
impl TryFrom<u32> for EntityTypeKey {
    type Error = &'static str;
    fn try_from(slot: u32) -> Result<Self, Self::Error> {
        Self::from_slot(slot)
    }
}

/// A playing sound's channel, for `play_sound_source` and
/// `play_streaming_source`: an id only, to tell sounds apart across the two
/// hooks. It gives no control over the channel: a sound's effects are set through
/// `play_sound` (`SoundPlay::effects`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ChannelRef {
    /// The channel's id, unique among the live ones.
    pub id: u64,
}

/// The client-supplied login destination returned by [`PlayerRef::virtual_host`].
/// This is neither the peer address nor a verified server name.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: the destination consists of exactly a host and a port.
pub struct VirtualHost {
    /// The host string from the handshake, before its first NUL character.
    pub host: String,
    /// The unsigned port from the same handshake.
    pub port: u16,
}

/// An armor stand's six pose rotations (`ArmorStand.DATA_HEAD_POSE` to `DATA_RIGHT_LEG_POSE`,
/// vanilla's `Rotations`): x, y and z in degrees, as `getHeadPose` ... `getRightLegPose` give them.
#[derive(Clone, Copy, Debug, PartialEq, sunrise_mod_api_macros::Wire)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct ArmorStandPose {
    /// `ArmorStand.headPose`: x, y and z rotation in degrees.
    pub head: [f32; 3],
    /// `ArmorStand.bodyPose`: x, y and z rotation in degrees.
    pub body: [f32; 3],
    /// `ArmorStand.leftArmPose`: x, y and z rotation in degrees.
    pub left_arm: [f32; 3],
    /// `ArmorStand.rightArmPose`: x, y and z rotation in degrees.
    pub right_arm: [f32; 3],
    /// `ArmorStand.leftLegPose`: x, y and z rotation in degrees.
    pub left_leg: [f32; 3],
    /// `ArmorStand.rightLegPose`: x, y and z rotation in degrees.
    pub right_leg: [f32; 3],
}

/// A new armor stand's pose (`ArmorStand.DEFAULT_HEAD_POSE` to `DEFAULT_RIGHT_LEG_POSE`).
impl Default for ArmorStandPose {
    fn default() -> Self {
        Self {
            head: [0.0, 0.0, 0.0],
            body: [0.0, 0.0, 0.0],
            left_arm: [-10.0, 0.0, -10.0],
            right_arm: [-15.0, 0.0, 10.0],
            left_leg: [-1.0, 0.0, -1.0],
            right_leg: [1.0, 0.0, 1.0],
        }
    }
}

/// An armor stand's client flags (`ArmorStand.DATA_CLIENT_FLAGS`).
/// The default is vanilla's: no flag set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct ArmorStandFlags {
    /// `ArmorStand.isSmall`.
    pub small: bool,
    /// `ArmorStand.showArms`.
    pub show_arms: bool,
    /// `ArmorStand.showBasePlate`.
    pub show_base_plate: bool,
    /// `ArmorStand.isMarker`.
    pub marker: bool,
}

/// A persistent entity UUID in the same bit order as the value from `EntityRef::uuid`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, sunrise_mod_api_macros::Wire)]
// exhaustive: a UUID is exactly one public 128-bit value
pub struct EntityUuid(pub u128);

/// An entity of the open world, for the `Host` methods that read or change one
/// (`entity.position(host)`). It holds the world's epoch and the entity's id: ids
/// are shared by every server level and never reused while the server world is
/// open. An entity that changes dimension on its own (a lone mob, a general entity)
/// is rebuilt under a new id, as in vanilla, so its old handle goes stale; a vehicle a
/// player rides keeps its id, and its handle still resolves in the new level. Follow
/// the entity's UUID across a change of level.
/// On a client the epoch identifies one client level and changes when that level
/// is replaced. Server reads and writes reject a handle from an earlier epoch,
/// of an entity that is gone, or used on a client. `Host::entity` makes a server
/// handle from an event's entity id; `ClientHost::crosshair_target` supplies a
/// client handle for an entity hit.
///
/// The base state vanilla gives every `Entity` is read and written through one rule for
/// every kind: the remaining fire ticks and `isOnFire`, `fireImmune`, `isSilent`,
/// `hasGlowingTag`, `isNoGravity` and `isInvulnerable`, saved as `Fire`, `Silent`,
/// `Glowing`, `NoGravity` and `Invulnerable` as `Entity.saveWithoutId` writes them. `None`
/// or `false` from them means a stale handle, a non-finite input or the wrong side. The
/// exceptions are a fishing bobber, a leash knot and a dragon's part, which keep no entity
/// data: they read the defaults and refuse writes. Kinds the game keeps as saved data hold
/// `Fire` as the saved short.
#[derive(sunrise_mod_api_macros::Wire, Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct EntityRef {
    pub(crate) epoch: WorldEpoch,
    pub(crate) id: u32,
}

impl EntityRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(epoch: WorldEpoch, id: u32) -> Self {
        Self { epoch, id }
    }
    /// The entity's id (the network id events carry).
    pub fn id(self) -> u32 {
        self.id
    }
    /// The open of the world the handle belongs to.
    pub fn epoch(self) -> WorldEpoch {
        self.epoch
    }
    /// Where the entity is `partial` of the way through its current tick
    /// (`Entity.getPosition(float)`): on each axis `Mth.lerp(partial, xo, x)`, from
    /// [`Self::previous_position`] to [`Self::position`], the position a renderer draws the
    /// entity at with the frame's partial tick. NeoForge mods call the same vanilla method.
    /// Side: both. None when either read is None.
    pub fn position_at(self, host: &dyn Host, partial: f32) -> Option<[f64; 3]> {
        let old = self.previous_position(host)?;
        let now = self.position(host)?;
        let delta = f64::from(partial);
        Some(std::array::from_fn(|axis| {
            old[axis] + delta * (now[axis] - old[axis])
        }))
    }
    /// Use the entity's live `Entity.getRandom` stream for `draw`. The hit leaves the
    /// draws in the stream in listener order. Returns `None` without running `draw`
    /// if this hit does not lend the entity.
    pub fn with_random<T>(
        self,
        hit: &mut dyn HitHost,
        draw: impl FnOnce(&mut dyn crate::RandomSource) -> T,
    ) -> Option<T> {
        crate::world::random::with_random_draw(draw, |draw| hit.lend_entity_random(self, draw))
    }
}

/// The resolved holder of a mob's or a boat's leash, or its absence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum LeashHolder {
    /// The entity is not leashed.
    Unleashed,
    /// A player, mob or boat holds the leash.
    Entity(EntityRef),
    /// A fence knot holds the leash at this block.
    Knot([i32; 3]),
}

/// A living entity of the open world: a mob, a player, an armor stand or the
/// dragon (vanilla's `LivingEntity`). `host.living(entity)` makes one from an
/// [`EntityRef`] of such an entity, and [`LivingRef::entity`] goes back, so the
/// entity's methods work on it too. Stale handles read as `None`, as an
/// `EntityRef`'s do.
#[derive(sunrise_mod_api_macros::Wire, Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct LivingRef(pub(crate) EntityRef);

impl LivingRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(entity: EntityRef) -> Self {
        Self(entity)
    }
    /// The same entity as an [`EntityRef`].
    pub fn entity(self) -> EntityRef {
        self.0
    }
    /// The entity's id (the network id events carry).
    pub fn id(self) -> u32 {
        self.0.id()
    }

    /// Read the stack in `slot` (`LivingEntity.getItemBySlot`). Players, mobs, armor stands
    /// and the dragon store all eight slots. An armor stand's body and saddle slots are storage
    /// only: vanilla's `canUseSlot` refuses them for interaction. On the client only the local
    /// player's copy is available.
    ///
    /// Returns `Some` of `read`'s result, or `None` without calling it if the slot is empty or
    /// unavailable or the handle is stale. This does not load chunks or change equipment.
    /// The view is valid only during `read`; use [`ItemStackView::project`] to get an owned
    /// copy with components.
    pub fn with_equipment<R>(
        self,
        host: &dyn Host,
        slot: EquipmentSlot,
        read: impl for<'a> FnOnce(ItemStackView<'a>) -> R,
    ) -> Option<R> {
        let backend = crate::backend_of(host)?;
        read_stack_view(read, |lend| backend.living_equipment(self, slot, lend))
    }

    /// Read `hand` (`LivingEntity.getItemInHand`): the selected main-hand slot or the off hand.
    /// The view, return value and availability follow [`Self::with_equipment`].
    pub fn with_hand_item<R>(
        self,
        host: &dyn Host,
        hand: Hand,
        read: impl for<'a> FnOnce(ItemStackView<'a>) -> R,
    ) -> Option<R> {
        let backend = crate::backend_of(host)?;
        read_stack_view(read, |lend| backend.living_hand_item(self, hand, lend))
    }

    /// Read the living attacker's weapon (`DamageSource.getWeaponItem`): the stack kept for a
    /// player's active Riptide spin if present, otherwise the live main hand. Server-side only.
    /// The view and return value follow [`Self::with_equipment`]. For the recorded projectile
    /// weapon of a death, use [`LootDrops::weapon_item`] instead.
    pub fn with_weapon_item<R>(
        self,
        host: &dyn Host,
        read: impl for<'a> FnOnce(ItemStackView<'a>) -> R,
    ) -> Option<R> {
        let backend = crate::backend_of(host)?;
        read_stack_view(read, |lend| backend.living_weapon_item(self, lend))
    }
}

impl From<LivingRef> for EntityRef {
    fn from(living: LivingRef) -> Self {
        living.0
    }
}

/// How an attribute modifier changes its attribute
/// (`AttributeModifier.Operation`). The value is the base plus every
/// `AddValue`, then grows by that sum times each `AddMultipliedBase`, then is
/// multiplied by one plus each `AddMultipliedTotal`.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, sunrise_mod_api_macros::Wire)]
pub enum AttributeOperation {
    /// `add_value`: adds the amount.
    AddValue,
    /// `add_multiplied_base`: adds the amount times the base.
    AddMultipliedBase,
    /// `add_multiplied_total`: multiplies by one plus the amount.
    AddMultipliedTotal,
}

/// An owned attribute modifier (vanilla's `AttributeModifier`) used for writes: its id,
/// which is unique on one attribute of one entity, the amount and the operation.
/// Read modifiers through [`LivingRef::visit_attribute_modifiers`].
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, sunrise_mod_api_macros::Wire)]
pub struct AttributeModifier {
    /// The modifier's id (`mymod:sprint_boost`).
    pub id: Identifier,
    /// The amount the operation applies.
    pub amount: f64,
    /// How it applies.
    pub operation: AttributeOperation,
}

impl AttributeModifier {
    /// A modifier with id `id`.
    pub fn new(id: Identifier, amount: f64, operation: AttributeOperation) -> Self {
        Self {
            id,
            amount,
            operation,
        }
    }
}

/// A modifier lent by [`LivingRef::visit_attribute_modifiers`].
// no wire: the id is borrowed from an in-process attribute map
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AttributeModifierView<'a> {
    /// The canonical modifier id (`mymod:sprint_boost`), borrowed for this visit.
    pub id: &'a str,
    /// The amount the operation applies.
    pub amount: f64,
    /// How it applies.
    pub operation: AttributeOperation,
}

/// Whether an attribute modifier is saved with its entity
/// (`AttributeInstance.addPermanentModifier`) or lasts until the entity
/// unloads (`addTransientModifier`), as the modifiers of worn items and
/// effects do.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, sunrise_mod_api_macros::Wire)]
pub enum ModifierLifetime {
    /// Not saved: gone when the entity is saved and loaded again.
    Transient,
    /// Saved with the entity.
    Permanent,
}

/// A living entity's active effect (vanilla's `MobEffectInstance`), as
/// [`LivingRef::visit_effects`] reads each effect and as `add_effect` gives one: the
/// effect's load-owned [`EffectKey`],
/// the ticks left (-1 is infinite), the amplifier from 0, and the flags a
/// beacon-style effect carries: `ambient` (an ambient one gives no
/// particles), `visible` (whether its particles show) and `show_icon`
/// (whether the HUD and inventory list it).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ActiveEffect {
    /// The effect's load-owned registry key; `id.id(host)` borrows its canonical name if available.
    pub id: EffectKey,
    /// The ticks left; -1 is infinite.
    pub duration: i32,
    /// The amplifier, from 0.
    pub amplifier: i32,
    /// `MobEffectInstance.isAmbient`: an ambient effect shows no particles.
    pub ambient: bool,
    /// `MobEffectInstance.isVisible`: whether its particles show.
    pub visible: bool,
    /// `MobEffectInstance.showIcon`: whether the HUD and the inventory list
    /// it. Vanilla keeps it apart from `visible`.
    pub show_icon: bool,
}

impl ActiveEffect {
    /// Construct an effect with its duration, amplifier and visual flags, as
    /// vanilla's six-argument `MobEffectInstance` constructor takes them.
    pub fn new(
        id: EffectKey,
        duration: i32,
        amplifier: i32,
        ambient: bool,
        visible: bool,
        show_icon: bool,
    ) -> Self {
        Self {
            id,
            duration,
            amplifier,
            ambient,
            visible,
            show_icon,
        }
    }
}

/// The answer to `mob_effect_applicable`: whether an effect can land on an
/// entity. An unanswered hook defers to the vanilla tag check; `Allow` and
/// `Deny` override it, as NeoForge's `MobEffectEvent.Applicable.Result`
/// does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum EffectApplicable {
    /// The effect lands, whatever the tags say.
    Allow,
    /// The effect is refused, whatever the tags say.
    Deny,
}

/// A vanilla `Mob`, narrowed from an entity with `host.mob(entity)`.
#[derive(sunrise_mod_api_macros::Wire, Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct MobRef(pub(crate) EntityRef);

impl MobRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(entity: EntityRef) -> Self {
        Self(entity)
    }
    /// The same entity as an entity handle.
    pub fn entity(self) -> EntityRef {
        self.0
    }
    /// The same entity as a living handle.
    pub fn living(self) -> LivingRef {
        LivingRef::new(self.0)
    }
}

/// The player, or mobs of the given species: the class filter of a
/// [`GoalSpec::NearestTarget`] and of a [`GoalSpec::AvoidEntity`]. The
/// species ids are namespaced (`minecraft:zombie`); a bare id reads as
/// `minecraft:`'s, like the effects' ids.
#[derive(Clone, Debug)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum MobFilter {
    /// The nearest player.
    Player,
    /// Mobs of these species (`Vec` of namespaced ids).
    Species(Vec<String>),
}

/// A vanilla AI goal, as a mob's `add_goal` and `add_target_goal` give one:
/// only the kinds Sunrise's own species code runs, with the vanilla
/// constructor's parameters (`MeleeAttackGoal(speed, followUnseen)`). Goals
/// with behavior of their own (a `MoveToBlockGoal` subclass's block test)
/// stay out: they take code, not parameters.
#[derive(Clone, Debug)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum GoalSpec {
    /// `FloatGoal`: swim up in water or lava. Vanilla parameters: none.
    Float,
    /// `ClimbOnTopOfPowderSnowGoal`: jump onto powder snow. Parameters: none.
    ClimbOnTopOfPowderSnow,
    /// `PanicGoal(speed)`, or `TurtlePanicGoal(speed)` with `water_range`
    /// set: always head for water this far away, not only when on fire.
    Panic {
        /// Movement speed multiplier while the goal runs; 1.0 is the mob's normal speed.
        speed: f64,
        /// Search distance for water in blocks, as `TurtlePanicGoal`'s.
        water_range: Option<i32>,
    },
    /// `RandomStrollGoal(speed, interval)`; `water_avoiding` set to its
    /// probability makes `WaterAvoidingRandomStrollGoal`, `swim`
    /// `RandomSwimmingGoal`, `flying` `WaterAvoidingRandomFlyingGoal`.
    Stroll {
        /// Movement speed multiplier while the goal runs; 1.0 is the mob's normal speed.
        speed: f64,
        /// Ticks between tries; the goal starts with chance 1 in `interval` each tick.
        interval: i32,
        /// Probability, 0 to 1, of preferring a spot away from water.
        water_avoiding: Option<f32>,
        /// Swim around in water instead of walking.
        swim: bool,
        /// Pick flying destinations.
        flying: bool,
    },
    /// `LookAtPlayerGoal(range, probability, onlyHorizontal)`: the nearest
    /// player, the nearest mob with `mobs`, or only mobs of `only`'s
    /// species.
    LookAtPlayer {
        /// Distance in blocks the goal looks for something to watch.
        range: f32,
        /// Chance, 0 to 1, each tick that the goal starts.
        probability: f32,
        /// Turn the head only sideways, never up or down.
        only_horizontal: bool,
        /// Also watch the nearest mob, not only the nearest player.
        mobs: bool,
        /// A mob type id (`minecraft:cow`) that restricts the mobs watched to that species.
        only: Option<String>,
    },
    /// `RandomLookAroundGoal`. Vanilla parameters: none.
    RandomLookAround,
    /// `MeleeAttackGoal(speed, followUnseen)`.
    MeleeAttack {
        /// Movement speed multiplier while the goal runs; 1.0 is the mob's normal speed.
        speed: f64,
        /// Keep chasing a target the mob can no longer see.
        follow_unseen: bool,
    },
    /// `LeapAtTargetGoal(leapMotion)`.
    LeapAtTarget {
        /// The upward speed of the leap, in blocks per tick.
        yd: f32,
    },
    /// `AvoidEntityGoal(what, maxDist, walk, sprint)`: sprint away from
    /// `what` once it is closer than `max_dist`.
    AvoidEntity {
        /// Which entities the mob avoids.
        what: MobFilter,
        /// Distance in blocks within which the mob flees.
        max_dist: f32,
        /// Speed multiplier while fleeing at a distance.
        walk: f64,
        /// Speed multiplier while fleeing close by.
        sprint: f64,
    },
    /// `TemptGoal(speed, items, canScare)`: `items` are item ids
    /// (`minecraft:wheat`; a bare id reads as `minecraft:`'s).
    Tempt {
        /// Movement speed multiplier while the goal runs; 1.0 is the mob's normal speed.
        speed: f64,
        /// Item ids that tempt the mob.
        items: Vec<String>,
        /// Whether a moving player holding the item scares the mob off.
        can_scare: bool,
    },
    /// `BreedGoal(speed)`. An animal keeps it; other mobs refuse it.
    Breed {
        /// Movement speed multiplier while the goal runs; 1.0 is the mob's normal speed.
        speed: f64,
    },
    /// `FollowParentGoal(speed)`. An animal keeps it; other mobs refuse it.
    FollowParent {
        /// Movement speed multiplier while the goal runs; 1.0 is the mob's normal speed.
        speed: f64,
    },
    /// `FollowMobGoal(speed, stopDistance, areaSize)`.
    FollowMob {
        /// Movement speed multiplier while the goal runs; 1.0 is the mob's normal speed.
        speed: f64,
        /// Distance in blocks at which the mob stops following.
        stop: f32,
        /// Radius in blocks searched for a mob to follow.
        area: f32,
    },
    /// `RestrictSunGoal`. Vanilla parameters: none.
    RestrictSun,
    /// `FleeSunGoal(speed)`.
    FleeSun {
        /// Movement speed multiplier while the goal runs; 1.0 is the mob's normal speed.
        speed: f64,
    },
    /// `EatBlockGoal`. Vanilla parameters: none.
    EatBlock,
    /// `BreathAirGoal`. Vanilla parameters: none.
    BreathAir,
    /// `TryFindWaterGoal`. Vanilla parameters: none.
    TryFindWater,
    /// `NearestAttackableTargetGoal` (the target selector): hunt `prey`,
    /// checking every `interval` ticks, `must_see` (`mustSee`) and
    /// `must_reach` (`mustReach`).
    NearestTarget {
        /// Which entities the mob hunts.
        prey: MobFilter,
        /// Only pick a target the mob can see.
        must_see: bool,
        /// Only pick a target the mob can path to.
        must_reach: bool,
        /// Ticks between tries; the goal starts with chance 1 in `interval` each tick.
        interval: i32,
    },
    /// `HurtByTargetGoal`, `alert` as its `setAlert()`.
    HurtByTarget {
        /// Also alert nearby mobs of the same kind, as `setAlert()`.
        alert: bool,
    },
}

impl GoalSpec {
    /// The goal's kind id: the same snake_case spelling `goals` and
    /// `target_goals` read and `remove_goal` and `remove_target_goal` take
    /// (`melee_attack`). A species variant of a vanilla goal (a bee's
    /// `BeeHurtByOtherGoal`) keeps its own vanilla name instead.
    pub fn id(&self) -> &'static str {
        match self {
            Self::Float => "float",
            Self::ClimbOnTopOfPowderSnow => "climb_on_top_of_powder_snow",
            Self::Panic { .. } => "panic",
            Self::Stroll { .. } => "stroll",
            Self::LookAtPlayer { .. } => "look_at_player",
            Self::RandomLookAround => "random_look_around",
            Self::MeleeAttack { .. } => "melee_attack",
            Self::LeapAtTarget { .. } => "leap_at_target",
            Self::AvoidEntity { .. } => "avoid_entity",
            Self::Tempt { .. } => "tempt",
            Self::Breed { .. } => "breed",
            Self::FollowParent { .. } => "follow_parent",
            Self::FollowMob { .. } => "follow_mob",
            Self::RestrictSun => "restrict_sun",
            Self::FleeSun { .. } => "flee_sun",
            Self::EatBlock => "eat_block",
            Self::BreathAir => "breath_air",
            Self::TryFindWater => "try_find_water",
            Self::NearestTarget { .. } => "nearest_target",
            Self::HurtByTarget { .. } => "hurt_by_target",
        }
    }
}

/// One goal of a mob's selector, as `goals` and `target_goals` read it.
///
/// This is a process-local observation and does not implement `Wire`.
// no wire: local mob goal inspection entry carrying a static goal id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct MobGoalEntry {
    /// The goal's priority: a lower number preempts a higher one.
    pub priority: i32,
    /// The goal's kind id (see [`GoalSpec::id`]).
    pub id: &'static str,
    /// Whether the goal is running now.
    pub running: bool,
}

/// The ender dragon of the open world, narrowed with `host.dragon(entity)`.
/// It is also a [`MobRef`] and a [`LivingRef`].
#[derive(sunrise_mod_api_macros::Wire, Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct DragonRef(pub(crate) EntityRef);

impl DragonRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(entity: EntityRef) -> Self {
        Self(entity)
    }
    /// The same entity as an [`EntityRef`].
    pub fn entity(self) -> EntityRef {
        self.0
    }
    /// The same entity as a mob handle.
    pub fn mob(self) -> MobRef {
        MobRef::new(self.0)
    }
}

/// An ender dragon phase (`EnderDragonPhase`), in vanilla's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum DragonPhase {
    /// Circling the end island (`DragonPhaseHoldingPattern`).
    HoldingPattern,
    /// Flying at a player and shooting fireballs.
    StrafePlayer,
    /// Flying toward the fountain to land.
    LandingApproach,
    /// Descending onto the fountain.
    Landing,
    /// Lifting off the fountain.
    Takeoff,
    /// Perched, breathing fire.
    SittingFlaming,
    /// Perched, looking for a target.
    SittingScanning,
    /// Perched, about to breathe.
    SittingAttacking,
    /// Charging a player in melee.
    ChargingPlayer,
    /// Dying, flying to the fountain.
    Dying,
    /// Hovering in place.
    Hover,
}

impl DragonPhase {
    const ALL: [Self; 11] = [
        Self::HoldingPattern,
        Self::StrafePlayer,
        Self::LandingApproach,
        Self::Landing,
        Self::Takeoff,
        Self::SittingFlaming,
        Self::SittingScanning,
        Self::SittingAttacking,
        Self::ChargingPlayer,
        Self::Dying,
        Self::Hover,
    ];
    /// The phase's vanilla id, as the `DragonPhase` tag and the synced data
    /// carry it.
    pub fn id(self) -> i32 {
        self as i32
    }
    /// The phase with a vanilla id (`EnderDragonPhase.getById`).
    pub fn from_id(id: i32) -> Option<Self> {
        usize::try_from(id)
            .ok()
            .and_then(|i| Self::ALL.get(i))
            .copied()
    }
}

/// A dropped item entity of the open world, narrowed with `host.item_entity`.
#[derive(sunrise_mod_api_macros::Wire, Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ItemEntityRef(pub(crate) EntityRef);

impl ItemEntityRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(entity: EntityRef) -> Self {
        Self(entity)
    }
    /// The same entity as an [`EntityRef`].
    pub fn entity(self) -> EntityRef {
        self.0
    }
    /// Read the live `ItemEntity.getItem` on the server, without copying the stack.
    /// This sees earlier writes, including those in an open transaction. Aborting that
    /// transaction rolls back its journaled writes.
    ///
    /// Returns `Some` of `read`'s result, or `None` without calling it if the item was
    /// removed, the handle is stale, or the stack is unavailable. Use
    /// [`ItemStackView::project`] to read components or keep an owned stack past the callback.
    pub fn with_stack<R>(
        self,
        host: &dyn Host,
        read: impl for<'a> FnOnce(ItemStackView<'a>) -> R,
    ) -> Option<R> {
        let backend = crate::backend_of(host)?;
        read_stack_view(read, |lend| backend.item_entity_stack(self, lend))
    }
}

/// An experience orb of the open world (vanilla's `ExperienceOrb`), narrowed with
/// `host.experience_orb`.
#[derive(sunrise_mod_api_macros::Wire, Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ExperienceOrbRef(pub(crate) EntityRef);

impl ExperienceOrbRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(entity: EntityRef) -> Self {
        Self(entity)
    }
    /// The same entity as an [`EntityRef`].
    pub fn entity(self) -> EntityRef {
        self.0
    }
}

/// Why a write to an [`ExperienceOrbRef`] was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum ExperienceOrbWriteError {
    /// The handle is stale, removed, or temporarily held out of lookup.
    Stale,
    /// World access is unavailable, or the call ran on a client.
    Unavailable,
    /// The count is 0 or exceeds `i32::MAX` (vanilla `ExperienceOrb.count`, `ExtraCodecs.POSITIVE_INT`).
    InvalidCount,
}

impl crate::NoBackend for ExperienceOrbWriteError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// A display entity of the open world (`block_display`, `item_display` or
/// `text_display`), narrowed with `host.display`.
#[derive(sunrise_mod_api_macros::Wire, Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct DisplayRef(pub(crate) EntityRef);

impl DisplayRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(entity: EntityRef) -> Self {
        Self(entity)
    }
    /// The same entity as an [`EntityRef`].
    pub fn entity(self) -> EntityRef {
        self.0
    }

    /// Lends the current `ItemDisplay.getItemStack` without copying its components.
    /// Use [`ItemStackView::project`] if you need the components or an owned stack
    /// that can outlive this call. Returns `None` without calling `read` when the
    /// stack is empty, this is not a live item display, or the read is unavailable.
    pub fn with_item<R>(
        self,
        host: &dyn Host,
        read: impl for<'a> FnOnce(ItemStackView<'a>) -> R,
    ) -> Option<R> {
        let backend = crate::backend_of(host)?;
        read_stack_view(read, |lend| backend.display_item_stack(self, lend))
    }
}

/// A projectile of the open world (vanilla's `Projectile`), narrowed with
/// `host.projectile`: arrows, tridents (a player's too), firework rockets,
/// thrown items, fireballs, wind charges, skulls, shulker bullets and mods'
/// thrown types.
#[derive(sunrise_mod_api_macros::Wire, Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ProjectileRef(pub(crate) EntityRef);

impl ProjectileRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(entity: EntityRef) -> Self {
        Self(entity)
    }
    /// The same entity as an [`EntityRef`].
    pub fn entity(self) -> EntityRef {
        self.0
    }
}

/// A cast fishing hook of the open world (vanilla's `FishingHook`), narrowed
/// from an entity with `host.fishing_hook(entity)`. It does not narrow to a
/// [`ProjectileRef`]: a hook's owner is always the player who cast it.
#[derive(sunrise_mod_api_macros::Wire, Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct FishingHookRef(pub(crate) EntityRef);

impl FishingHookRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(entity: EntityRef) -> Self {
        Self(entity)
    }
    /// The same entity as an [`EntityRef`].
    pub fn entity(self) -> EntityRef {
        self.0
    }
}

/// Where a fishing bobber outside water floats and fishes, as a mod answers
/// `fishing_medium`: lava, the void under an island, a cloud. In the block of
/// `surface` or below (its y under `floor(surface) + 1`) the bobber floats and
/// fishes as in still water; above it, it falls as anywhere else.
#[derive(Clone, Debug, Default, PartialEq)]
// exhaustive: a mod builds the answer; write it with `..FishingMedium::default()`
#[derive(sunrise_mod_api_macros::Wire)]
pub struct FishingMedium {
    /// The height the bobber floats at (world Y), as water's surface is. A value that is
    /// not finite is no medium.
    pub surface: f64,
    /// The loot table a catch from here rolls (`mymod:gameplay/void_fishing`); None rolls
    /// vanilla's `minecraft:gameplay/fishing`, and so does an id that is malformed or names
    /// no table the server has (with one warning naming the mod and the id). Read on the
    /// server; a client ignores it.
    pub loot_table: Option<String>,
    /// The ticks a fish takes to come, `[min, max]`: the wait is `min + nextInt(max - min +
    /// 1)` less the rod's Lure, with `min` raised to at least 1 and `max` to at least `min`.
    /// None keeps vanilla's 100 to 600. Read on the server; a client ignores it.
    pub lure_ticks: Option<[i32; 2]>,
}

/// A villager or wandering trader (vanilla's `AbstractVillager`, a `Merchant`),
/// narrowed from an entity with `host.merchant(entity)`. If a villager's saved
/// profession is unresolved, offers read empty and merchant mutations are
/// refused until its profession is available again; saved offers are retained.
#[derive(sunrise_mod_api_macros::Wire, Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct MerchantRef(pub(crate) EntityRef);

impl MerchantRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(entity: EntityRef) -> Self {
        Self(entity)
    }
    /// The same entity as an [`EntityRef`].
    pub fn entity(self) -> EntityRef {
        self.0
    }
}

/// Which `Display` subclass an entity is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum DisplayKind {
    /// Block.
    Block,
    /// Item.
    Item,
    /// Text.
    Text,
}

impl DisplayKind {
    /// The entity type id: `minecraft:block_display`, `minecraft:item_display` or `minecraft:text_display`.
    pub fn entity_type(self) -> &'static str {
        match self {
            Self::Block => "minecraft:block_display",
            Self::Item => "minecraft:item_display",
            Self::Text => "minecraft:text_display",
        }
    }
    /// The kind of an entity type id, or None for another entity type.
    pub fn from_entity_type(id: &str) -> Option<Self> {
        [Self::Block, Self::Item, Self::Text]
            .into_iter()
            .find(|kind| kind.entity_type() == id)
    }
}

/// `Display.BillboardConstraints`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum Billboard {
    /// Fixed.
    Fixed,
    /// Vertical.
    Vertical,
    /// Horizontal.
    Horizontal,
    /// Center.
    Center,
}

impl Billboard {
    /// Every constraint, in ordinal order.
    pub const ALL: [Self; 4] = [Self::Fixed, Self::Vertical, Self::Horizontal, Self::Center];
    /// `getSerializedName`: `fixed`, `vertical`, `horizontal` or `center`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Fixed => "fixed",
            Self::Vertical => "vertical",
            Self::Horizontal => "horizontal",
            Self::Center => "center",
        }
    }
    /// The constraint with that name, or None for one vanilla doesn't have.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.name() == name)
    }
}

/// `Display.TextDisplay.Align`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum TextAlignment {
    /// Center.
    Center,
    /// Left.
    Left,
    /// Right.
    Right,
}

impl TextAlignment {
    /// `getSerializedName`: `center`, `left` or `right`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Center => "center",
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

/// A display's `Transformation`: a translation, a left rotation, a scale and a
/// right rotation. Quaternions are `[x, y, z, w]`.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct Transformation {
    /// The offset in blocks.
    pub translation: [f32; 3],
    /// The rotation applied after the scale, as a quaternion (x, y, z, w).
    pub left_rotation: [f32; 4],
    /// The scale on each axis.
    pub scale: [f32; 3],
    /// The rotation applied before the scale, as a quaternion (x, y, z, w).
    pub right_rotation: [f32; 4],
}

impl Transformation {
    /// `Transformation.IDENTITY`.
    pub const IDENTITY: Self = Self {
        translation: [0.0; 3],
        left_rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0; 3],
        right_rotation: [0.0, 0.0, 0.0, 1.0],
    };
}

/// `ProjectileDeflection`: how `ProjectileRef::deflect` turns a projectile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum Deflection {
    /// `REVERSE`: backwards at `-power / 2` of the speed, turned about 180
    /// degrees by a random amount.
    Reverse,
    /// `AIM_DEFLECT`: along the deflecting entity's look.
    AimDeflect,
    /// `MOMENTUM_DEFLECT`: along the deflecting entity's movement.
    MomentumDeflect,
}

/// Why a projectile's pierce level is unavailable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum PierceLevelError {
    /// The host is not a logical server.
    WrongSide,
    /// The projectile handle is stale or unavailable.
    Stale,
    /// The projectile has no arrow state.
    NotArrow,
}

impl NoBackend for PierceLevelError {
    fn no_backend() -> Self {
        Self::Stale
    }

    fn wrong_side() -> Self {
        Self::WrongSide
    }
}

/// Why a player's collision exclusions were not set (`EntityRef::set_collision_exclusions`,
/// `ClientHost::set_collision_exclusions`). The boxes the player had stay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum CollisionExclusionsError {
    /// The handle is stale, or a client has no player of its own.
    Stale,
    /// The entity is not a player: only players keep collision exclusions.
    NotAPlayer,
    /// A coordinate that is NaN or infinite.
    NotFinite,
    /// A box whose minimum is above its maximum on an axis.
    Inverted,
}

impl NoBackend for CollisionExclusionsError {
    fn no_backend() -> Self {
        Self::Stale
    }
}

/// Why a client's hotbar selection was not made (`ClientHost::select_hotbar_slot`). The
/// selection the player had stays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: the three refusals are the contract; a new reason is a breaking change
pub enum HotbarSlotError {
    /// The client shows no world, or has no player of its own.
    NoWorld,
    /// A slot above 8: vanilla's range is 0 to 8, and the server ignores others.
    OutOfRange {
        /// The slot the caller asked for.
        slot: u8,
    },
    /// The number keys do nothing for the local player, as for a spectator (its keys open
    /// the spectator menu instead).
    Refused,
}

impl NoBackend for HotbarSlotError {
    fn no_backend() -> Self {
        Self::NoWorld
    }
}

/// A vanilla `AbstractMinecart`, narrowed from an entity with `host.minecart(entity)`.
#[derive(sunrise_mod_api_macros::Wire, Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct MinecartRef(pub(crate) EntityRef);

impl MinecartRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(entity: EntityRef) -> Self {
        Self(entity)
    }
    /// The same entity as an [`EntityRef`].
    pub fn entity(self) -> EntityRef {
        self.0
    }
}

/// Which minecart an entity is: one per vanilla `AbstractMinecart` subclass.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum MinecartKind {
    /// `Minecart`.
    Rideable,
    /// `MinecartChest`.
    Chest,
    /// `MinecartFurnace`.
    Furnace,
    /// `MinecartTNT`.
    Tnt,
    /// `MinecartHopper`.
    Hopper,
    /// `MinecartCommandBlock`.
    CommandBlock,
    /// `MinecartSpawner`.
    Spawner,
}

/// `ChatVisiblity`: which chat a client shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ChatVisibility {
    /// All chat.
    Full,
    /// Only system messages and commands.
    System,
    /// No chat.
    Hidden,
}

/// `HumanoidArm`: a player's main hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum MainHand {
    /// The left hand.
    Left,
    /// The right hand.
    Right,
}

/// A player's client options, as `ServerboundClientInformationPacket` carries them
/// (vanilla's `ClientInformation`). Raw ids map as vanilla's `ByIdMap` does: chat
/// visibility and particle status wrap (`floorMod(id, 3)`), the main hand reads an
/// unknown id as left.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ClientInfo {
    /// The client's language code, such as `en_us`.
    pub language: String,
    /// The client's view distance in chunks.
    pub view_distance: i8,
    /// Which chat the client shows.
    pub chat_visibility: ChatVisibility,
    /// Whether the client shows chat colours.
    pub chat_colors: bool,
    /// Skin layer bits (`PlayerModelPart`).
    pub model_customisation: u8,
    /// The player's main hand.
    pub main_hand: MainHand,
    /// Whether the client asked for filtered text.
    pub text_filtering: bool,
    /// Whether the player shows in the server list sample.
    pub allows_listing: bool,
    /// How many particles the client draws.
    pub particle_status: ParticleStatus,
}

/// Why a mod removes an entity (`Entity.RemovalReason`): the two reasons
/// that destroy it. The unloading and dimension-change reasons belong to the
/// game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum RemovalReason {
    /// `KILLED`: a mob's effects that act on removal run.
    Killed,
    /// `DISCARDED`, as `Entity.discard`.
    Discarded,
}

/// An online player in one open server world, keyed by UUID.
/// This is the player's identity, not a particular body: handles for the same
/// player stay equal across respawn. Respawning does not invalidate a handle.
/// A nested guest turn cannot re-enter a player active in an outer turn;
/// operations requiring that player's swapped state report them unavailable.
#[derive(sunrise_mod_api_macros::Wire, Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct PlayerRef {
    pub(crate) entity: EntityRef,
    pub(crate) uuid: u128,
}

impl PlayerRef {
    /// The same player as an entity handle.
    pub fn entity(self) -> EntityRef {
        self.entity
    }
    /// The player's UUID.
    pub fn uuid(self) -> u128 {
        self.uuid
    }

    /// Read the player's current inventory by vanilla `Inventory.getItem` numbers: 0 to 35
    /// main (0 to 8 hotbar), 36 feet, 37 legs, 38 chest, 39 head, 40 offhand, 41 body, 42 saddle.
    /// The view is read-only and valid only in `read`. Use [`InventoryView::get`] for one slot
    /// or [`InventoryView::iter`] for nonempty slots in ascending order. For just the main
    /// inventory, use `view.iter().take_while(|(slot, _)| *slot < 36)`. Read scalars in `read`;
    /// use [`ItemStackView::project`] to get components or retain an owned stack, and handle
    /// refusal separately from an empty slot.
    ///
    /// Returns `Some` of the callback's result even when the inventory is empty. Returns
    /// `None` without calling it if the player is stale or unavailable. On the client only
    /// the local player's inventory is available, including its local predictions.
    pub fn with_inventory<R>(
        self,
        host: &dyn Host,
        read: impl for<'a> FnOnce(InventoryView<'a>) -> R,
    ) -> Option<R> {
        let mut read = Some(read);
        let mut result = None;
        crate::backend_of(host)?.player_inventory_slots(self, &mut |view| {
            if let Some(read) = read.take() {
                result = Some(read(view));
            }
        })?;
        result
    }

    /// Read the player's ender chest by vanilla slot number, 0 to 26, the `Slot` numbers
    /// in the player's saved `EnderItems`. If the chest is open, this reads the live slots.
    /// The view is read-only and valid only in `read`.
    ///
    /// Returns `Some` of the callback's result even when the chest is empty. Returns
    /// `None` without calling it on a client or if the player is stale or unavailable.
    pub fn with_ender_chest<R>(
        self,
        host: &dyn Host,
        read: impl for<'a> FnOnce(InventoryView<'a>) -> R,
    ) -> Option<R> {
        let mut read = Some(read);
        let mut result = None;
        crate::backend_of(host)?.player_ender_chest(self, &mut |view| {
            if let Some(read) = read.take() {
                result = Some(read(view));
            }
        })?;
        result
    }
}

impl From<PlayerRef> for EntityRef {
    fn from(player: PlayerRef) -> Self {
        player.entity
    }
}

/// The collision flags left by a player's last `Entity.move`.
#[derive(sunrise_mod_api_macros::Wire, Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CollisionFlags {
    /// Movement hit a wall or another horizontal obstruction.
    pub horizontal: bool,
    /// The horizontal collision only slightly changed the movement direction.
    pub minor_horizontal: bool,
    /// Movement was stopped vertically, by a floor or a ceiling.
    pub vertical: bool,
    /// The vertical collision stopped downward movement.
    pub vertical_below: bool,
}

impl CollisionFlags {
    /// Build a snapshot of the collision flags retained by a move.
    pub const fn new(
        horizontal: bool,
        minor_horizontal: bool,
        vertical: bool,
        vertical_below: bool,
    ) -> Self {
        Self {
            horizontal,
            minor_horizontal,
            vertical,
            vertical_below,
        }
    }
}

/// A player's food data.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct PlayerFood {
    /// The food level, 0 to 20.
    pub level: i32,
    /// The saturation, up to the food level.
    pub saturation: f32,
    /// The exhaustion; the game takes a saturation or food point at 4.
    pub exhaustion: f32,
}

// no wire: borrows a profile from the host.
/// A player's borrowed game profile (authlib's `GameProfile`): UUID, name and login
/// properties, such as `textures` (the skin) when available. A custom test host can lend one using
/// `mod_api::testing::GameProfileFixture` with the `testing` feature enabled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct GameProfile<'a> {
    /// The player's UUID.
    pub uuid: u128,
    /// The player's name, borrowed from the host.
    pub name: &'a str,
    properties: &'a [AccountProperty],
}

impl<'a> GameProfile<'a> {
    /// Read the profile's properties in login order without copying the text.
    /// Offline-mode players and profiles from the user cache have no properties.
    pub fn properties(&self) -> impl ExactSizeIterator<Item = ProfileProperty<'a>> + use<'a> {
        self.properties.iter().map(|property| ProfileProperty {
            name: &property.name,
            value: &property.value,
            signature: property.signature.as_deref(),
        })
    }
}

pub(crate) mod profile_adapter {
    use super::{AccountProperty, GameProfile};

    /// Lend the player's login properties from the game record.
    pub fn game_profile<'a>(
        uuid: u128,
        name: &'a str,
        properties: &'a [AccountProperty],
    ) -> GameProfile<'a> {
        GameProfile {
            uuid,
            name,
            properties,
        }
    }
}

// no wire: borrows one login property's strings from the host.
/// One borrowed property of a [`GameProfile`] (authlib's `Property`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProfileProperty<'a> {
    /// The property's name, such as `textures`.
    pub name: &'a str,
    /// Its value, as the session server sent it (base64 for `textures`).
    pub value: &'a str,
    /// The session server's signature, when the value is signed.
    pub signature: Option<&'a str>,
}

// no wire: An in-process result of a native menu open, not a transported payload.
/// Why [`PlayerRef::open_mod_menu`], [`PlayerRef::open_chest_menu`], or
/// [`PlayerRef::open_ender_chest`] opened nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OpenMenuError {
    /// The menu type is not a content mod's (`data/<id>/sunrise/menu/`).
    NotModMenu,
    /// A menu with `owned_slots` also declares a crafting grid.
    OwnedSlotsWithCrafting,
    /// A speculative quick move or merchant action still owns a window.
    /// An ordinary `menu_clicked` callback does not cause this refusal.
    ClickInProgress,
    /// The player is stale, dead, disconnecting, has no supported client, or their turn
    /// is unavailable for the open veto. Also returned by a host without a game.
    PlayerUnavailable,
    /// The extra data exceeds the 1,048,555-byte limit (vanilla's 1 MiB clientbound
    /// custom payload limit minus the 21-byte envelope header).
    TooLarge,
    /// The title could not cross to the game.
    Title(crate::text::TextFormError),
    /// The initial data must contain exactly the menu type's `data_slots` values.
    DataCountMismatch,
    /// The title cannot be encoded as vanilla network NBT.
    TitleNotEncodable,
    /// One of the stacks was refused at the game's item conversion boundary.
    Stack(StackWriteError),
}

impl From<crate::text::TextFormError> for OpenMenuError {
    fn from(error: crate::text::TextFormError) -> Self {
        Self::Title(error)
    }
}

impl From<StackWriteError> for OpenMenuError {
    fn from(error: StackWriteError) -> Self {
        Self::Stack(error)
    }
}

impl NoBackend for OpenMenuError {
    fn no_backend() -> Self {
        Self::PlayerUnavailable
    }
}

/// Why [`PlayerRef::send_ping`] did not queue a play ping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum PlayerPingRefusal {
    /// The handle no longer identifies a player on this server.
    Stale,
    /// The player has no open play connection of their own.
    NoConnection,
    /// The ping could not be encoded or admitted to the outbound queue or transaction batch.
    SendFailed,
}

impl NoBackend for PlayerPingRefusal {
    fn no_backend() -> Self {
        Self::Stale
    }
}

/// Why [`PlayerRef::respawn`] respawned nobody. Nothing changed when one comes back; the checks
/// run in the order the variants are listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum RespawnRefusal {
    /// The call was made on a client; the respawn is the server's.
    Client,
    /// A player's state is swapped in (the call came from inside a player's hook, such as a
    /// command or a packet handler); call it from `server_tick`.
    InPlayerHook,
    /// The handle no longer identifies a player on this server: it is stale, or they left.
    Stale,
    /// The player has no connection of their own to press Respawn on: a fake player, or the
    /// world's own player in a single-player world.
    NoConnection,
    /// The player has health above 0 and has not won the game: the case the Respawn button
    /// ignores. A player who has won the game (the End poem is showing) is respawned even
    /// alive, as the button does.
    Alive,
}

impl NoBackend for RespawnRefusal {
    fn no_backend() -> Self {
        Self::Stale
    }

    fn wrong_side() -> Self {
        Self::Client
    }
}

/// Where a player would respawn now, as [`PlayerRef::respawn_target`] finds it: the transition
/// `ServerPlayer.findRespawnPositionAndUseSpawnBlock(false, DO_NOTHING)` gives.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct RespawnTarget {
    /// The level: the respawn point's, or the world spawn's (`MinecraftServer.findRespawnDimension`).
    pub level: LevelRef,
    /// The feet position in that level.
    pub position: [f64; 3],
    /// The yaw the player faces: toward the bed or anchor, else the world spawn's.
    pub yaw: f32,
    /// The pitch the player faces: level at a bed or anchor, else the world spawn's.
    pub pitch: f32,
    /// The player had a respawn point whose bed or anchor no longer gives a position
    /// (`TeleportTransition.missingRespawnBlock`): the target is the world spawn, and a respawn
    /// there would tell them `block.minecraft.spawn.not_valid`.
    pub missing_respawn_block: bool,
}

/// Why [`PlayerRef::respawn_target`] found no target now. The checks run in the order the
/// variants are listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum RespawnTargetUnavailable {
    /// The handle no longer identifies a player on this server: it is stale, or they left.
    Stale,
    /// A player's state is swapped in (the call came from inside a player's hook, such as a
    /// command or a packet handler) and the target is in a level not in place, which the call
    /// would have to put in place to read; call it from `server_tick`.
    InPlayerHook,
    /// The world spawn's column search (`PlayerSpawnFinder`) waits for a chunk to load, as the
    /// End exit's search does; the call asked for it. Ask again on a later tick.
    Loading,
}

impl NoBackend for RespawnTargetUnavailable {
    fn no_backend() -> Self {
        Self::Stale
    }
}

/// Why [`MenuRef::set_items`] replaced nothing. Nothing is sent when one comes back.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum MenuItemsRefusal {
    /// The call was made on a client; menus are the server's.
    Client,
    /// The menu's player is not online and placed.
    NoPlayer,
    /// The handle does not name the player's open [`PlayerRef::open_chest_menu`] or
    /// [`PlayerRef::open_mod_menu`] window: a stale handle, a menu the game opened (a chest, a
    /// trade), or the inventory menu.
    NotOpen,
    /// One of the stacks was refused at the game's item conversion boundary: an item the world
    /// lacks, a count or damage the item does not allow.
    Stack(StackWriteError),
    /// The window owns its slots. Write them through [`ItemPlace::MenuSlot`](crate::ItemPlace::MenuSlot).
    OwnedSlots,
    /// The call is not available in this phase or host state.
    Unavailable,
}

impl From<StackWriteError> for MenuItemsRefusal {
    fn from(error: StackWriteError) -> Self {
        Self::Stack(error)
    }
}

impl NoBackend for MenuItemsRefusal {
    fn no_backend() -> Self {
        Self::Unavailable
    }

    fn wrong_side() -> Self {
        Self::Client
    }
}

/// A player's experience (`Player.experienceLevel`, `experienceProgress`,
/// `totalExperience`).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct PlayerExperience {
    /// The experience level.
    pub level: i32,
    /// Progress toward the next level, from 0 to 1.
    pub progress: f32,
    /// The total experience points.
    pub total: i32,
}

/// A change requested by an effect tick, applied after its hooks return.
// hidden: a game-side type, not part of the mod surface
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum EffectTickChange {
    Heal(f32),
    Hurt(String, f32),
    AddExhaustion(f32),
    Eat(i32, f32),
}

impl ApplyEffectTickEvent<'_> {
    /// `LivingEntity.hasEffect`: whether the holder currently has `effect`.
    pub fn has_effect(&self, effect: crate::EffectKey) -> bool {
        self.active_effects.contains(&effect)
    }
    /// Queue `LivingEntity.heal` on the holder.
    pub fn heal(&mut self, amount: f32) {
        self.changes.push(EffectTickChange::Heal(amount));
    }

    /// Queue damage of a registered damage type on the holder: a vanilla
    /// one (`minecraft:magic`, `minecraft:wither`) or a datapack's
    /// (`demo:heat`). It reaches the holder's hurt hooks with that type; an
    /// unregistered type hurts nothing.
    pub fn hurt(&mut self, damage_type: &str, amount: f32) {
        self.changes
            .push(EffectTickChange::Hurt(damage_type.to_owned(), amount));
    }

    /// Queue food exhaustion; a non-player holder ignores it.
    pub fn add_exhaustion(&mut self, amount: f32) {
        self.changes.push(EffectTickChange::AddExhaustion(amount));
    }

    /// Queue `FoodData.eat`; a non-player holder ignores it.
    pub fn eat(&mut self, nutrition: i32, saturation_modifier: f32) {
        self.changes
            .push(EffectTickChange::Eat(nutrition, saturation_modifier));
    }
}

/// A player's server-side abilities (`Abilities`).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct PlayerAbilities {
    /// Takes no damage.
    pub invulnerable: bool,
    /// Is flying now.
    pub flying: bool,
    /// May start flying.
    pub may_fly: bool,
    /// Breaks blocks at once and uses no items (creative).
    pub instabuild: bool,
    /// May break and place blocks.
    pub may_build: bool,
    /// The flying speed.
    pub fly_speed: f32,
    /// Walking speed (`Abilities.walkingSpeed`, vanilla's 0.1).
    pub walk_speed: f32,
}

/// What a fake player does in `PlayerRef::perform_action`: a player's own action, run through
/// the game's path for it, as a client's packet is. Its answer is an [`ActionOutcome`], or an
/// [`ActionRefused`] when none of it ran.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: mods build one; the game runs every kind, so a new kind must be handled there
#[derive(sunrise_mod_api_macros::Wire)]
pub enum PlayerAction {
    /// Use the item in `hand` on the block at `pos`, clicked at `hit`
    /// (`ServerPlayerGameMode.useItemOn`): the block's use, then the item's `useOn`, which
    /// places a block item.
    UseOn {
        /// The hand whose item is used.
        hand: Hand,
        /// The block clicked (`BlockHitResult.getBlockPos`).
        pos: [i32; 3],
        /// Where on it.
        hit: BlockHit,
    },
    /// Use the item in `hand` in the air (`ServerPlayerGameMode.useItem`).
    UseItem {
        /// The hand whose item is used.
        hand: Hand,
    },
    /// Interact with `target` using one hand (`Player.interactOn`), without falling back to
    /// using the item in the air when the entity passes the click.
    InteractOn {
        /// The hand whose item is used.
        hand: Hand,
        /// The entity clicked, in the fake's level.
        target: EntityRef,
        /// The hit relative to the entity's origin, as in an entity interaction packet.
        location: [f64; 3],
    },
    /// Attack `target` with the main hand (`ServerGamePacketListenerImpl.handleInteract`'s
    /// ATTACK, then `Player.attack`): a player, a mob or another entity, or a fireball, a wind
    /// charge or a shulker bullet turned back. Mobs don't take the fake as the player to answer.
    Attack {
        /// What is hit, in the fake's level.
        target: EntityRef,
    },
    /// Break the block at `pos` at once (`ServerPlayerGameMode.destroyBlock`), with the game
    /// mode's and the held tool's rules, the tool's wear and the drops, and no digging time:
    /// a mod that digs over time counts the progress itself. Outside creative, a block no
    /// digging breaks (destroy time -1, such as bedrock) stays.
    BreakBlock {
        /// The block broken.
        pos: [i32; 3],
    },
}

/// Temporary state for [`PlayerRef::perform_action_scoped`]. The existing fake acts in
/// survival with a fresh standing body, holding only `stack` in its main hand; its persistent
/// body, items, look, secondary-use state and action timers are restored afterwards.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct PlayerActionSetup {
    /// The main-hand stack, or `None` for an empty hand.
    pub stack: Option<ItemStack>,
    /// Eye position in world coordinates, using the standing eye height (1.62).
    pub eye: [f64; 3],
    /// Vanilla yaw in degrees.
    pub yaw: f32,
    /// Vanilla pitch in degrees.
    pub pitch: f32,
    /// Secondary use, as when holding sneak; this does not change the standing eye height.
    pub secondary_use: bool,
    /// Start with a full attack charge: with the stack in hand, the attack-strength ticker is
    /// set so that `getAttackStrengthScale(0.5)` is 1.0, and the fake's own ticker comes back
    /// afterwards with its other action timers. Only [`PlayerAction::Attack`] reads it. A stack
    /// that leaves no attack speed never charges, as in vanilla.
    /// `false` by default; set it by assignment: `setup.full_attack_charge = true;`.
    pub full_attack_charge: bool,
}

impl PlayerActionSetup {
    /// A scoped survival setup with the supplied main-hand stack and viewpoint.
    pub fn new(
        stack: Option<ItemStack>,
        eye: [f64; 3],
        yaw: f32,
        pitch: f32,
        secondary_use: bool,
    ) -> Self {
        Self {
            stack,
            eye,
            yaw,
            pitch,
            secondary_use,
            full_attack_charge: false,
        }
    }
}

/// Tooltip color semantics for an attribute change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// exhaustive: vanilla's `Attribute.Sentiment` has these three values, and the game matches them all
#[derive(sunrise_mod_api_macros::Wire)]
pub enum Sentiment {
    /// Increasing values are beneficial.
    #[default]
    Positive,
    /// Increasing values are harmful.
    Negative,
    /// Changes are neutral.
    Neutral,
}

/// Why an entity physically moves (`Entity.move`'s `MoverType`, the
/// `entity_move` hook of #2308/#2414): itself, the player behind a position
/// packet, a piston, a shulker box lid, a shulker mob's opening lid. Boats
/// and minecarts pass `SelfMovement`, as vanilla's own kinds do; `Player` is
/// the server packet sites only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum MoveCause {
    /// `MoverType.SELF`: the body's own tick, travel or packet follow-up.
    SelfMovement,
    /// `MoverType.PLAYER`: the server applying a player's position or vehicle
    /// packet.
    Player,
    /// `MoverType.PISTON`: a moving piston block pushes the body.
    Piston,
    /// `MoverType.SHULKER_BOX`: a shulker box's opening lid pushes it away.
    ShulkerBox,
    /// `MoverType.SHULKER`: a shulker mob's opening lid pushes it away.
    Shulker,
    /// Sunrise's own: a moving collider (an entity's `set_collider`) carries a body standing on
    /// it or shoves a body in the swept path of its translation and rotation, after the level's
    /// entities tick. Vanilla has no such mover.
    Carried,
}

impl MoveCause {
    /// Every cause, in ordinal order.
    pub const ALL: [Self; 6] = [
        Self::SelfMovement,
        Self::Player,
        Self::Piston,
        Self::ShulkerBox,
        Self::Shulker,
        Self::Carried,
    ];
}

/// The `LivingEntity.travel` branch a living body is about to run. Replacing
/// the travel input or velocity does not change the branch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum TravelMode {
    /// `LivingEntity.travelInAir`: the ordinary ground-and-air branch.
    Air,
    /// `LivingEntity.travelInFluid` with water: swimming.
    Water,
    /// `LivingEntity.travelInFluid` with lava.
    Lava,
    /// `LivingEntity.travelFallFlying`: gliding.
    Glide,
    /// The flying branch (`Player` abilities flying or a mob's flight).
    Flying,
    /// Move using the existing velocity alone, as squid do in and out of water.
    VelocityOnly,
}

pub(crate) fn refused_move_output(
    accepted: &[f64; 3],
    delta: &[f64; 3],
) -> Result<(), &'static str> {
    if accepted.map(f64::to_bits) != delta.map(f64::to_bits)
        && delta.iter().any(|component| !component.is_finite())
    {
        return Err("delta");
    }
    Ok(())
}

/// A `living_travel_moved` listener's proposed velocity: any finite vector is accepted
/// (`Entity.setDeltaMovement`), a changed non-finite one is refused.
pub(crate) fn refused_velocity_output(
    accepted: &[f64; 3],
    velocity: &[f64; 3],
) -> Result<(), &'static str> {
    if accepted.map(f64::to_bits) != velocity.map(f64::to_bits)
        && velocity.iter().any(|component| !component.is_finite())
    {
        return Err("velocity");
    }
    Ok(())
}

#[test]
fn travel_velocity_proposals_accept_finite_vectors_and_refuse_a_changed_non_finite_one() {
    let accepted = [65.0, -66.0, 67.0];
    assert_eq!(refused_velocity_output(&[1.0; 3], &accepted), Ok(()));
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut proposed = accepted;
        proposed[1] = value;
        assert_eq!(
            refused_velocity_output(&accepted, &proposed),
            Err("velocity")
        );
    }
}

#[test]
fn move_replacements_accept_finite_components_without_an_axis_limit() {
    let accepted = [65.0, -66.0, 67.0];
    assert_eq!(refused_move_output(&[1.0; 3], &accepted), Ok(()));
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for axis in 0..3 {
            let mut proposed = accepted;
            proposed[axis] = value;
            assert_eq!(refused_move_output(&accepted, &proposed), Err("delta"));
        }
    }
}

/// How a declared attribute's modifier lines read on item tooltips: NeoForge's
/// `RangedAttribute`, `PercentageAttribute` and `BooleanAttribute`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// exhaustive: NeoForge's three display kinds, and the tooltip matches them all
#[derive(sunrise_mod_api_macros::Wire)]
pub enum AttributeDisplay {
    /// Added values as numbers, multiplied ones as percentages: vanilla's.
    #[default]
    Ranged,
    /// Added values as percentages too: `+0.05` reads `+5%`.
    Percentage,
    /// On or off: an added value above zero reads enabled, any other disabled.
    Boolean,
}

/// How a declared attribute's tooltip lines read beyond their colour (NeoForge's
/// `IAttributeExtension`). The default reads as vanilla's attributes do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
// exhaustive: mods build it with `..Default::default()`, and the game reads every field
#[derive(sunrise_mod_api_macros::Wire)]
pub struct AttributeTooltip {
    /// How its modifier amounts read.
    pub display: AttributeDisplay,
    /// The namespaced id of the attribute's base modifier (`getBaseId`): a modifier with this
    /// id reads as the green total of the player's base value and its amount, as
    /// `minecraft:base_attack_damage` does for attack damage. Without a player (recipe search)
    /// it reads as a plain line.
    pub base_modifier: Option<String>,
}
#[cfg(test)]
#[path = "entity_type_key_tests.rs"]
mod entity_type_key_tests;

/// Why a move-control wish was refused. The previous wish stays unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum WantedPositionError {
    /// The host has no game backend (for example, a mod test double).
    Unavailable,
    /// A coordinate is NaN or infinite.
    NonFinitePosition,
    /// The speed is NaN or infinite. Finite negative speeds are accepted.
    NonFiniteSpeed,
}
impl NoBackend for WantedPositionError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}
