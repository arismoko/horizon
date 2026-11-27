// Included in health.rs to retain its private scope.
/// The in-flight handoff for a single hurt, taken from the live body by its caller before
/// continuing. Death and protection need this even if nobody listens to Post.
#[derive(Clone, Debug)]
pub struct DamageCompletion {
    pub post: Option<std::sync::Arc<DamagePost>>,
    pub protection: Option<PlayerProtection>,
}

/// The hand and original stack accepted by `checkTotemDeathProtection`.
#[derive(Clone, Debug)]
pub struct PlayerProtection {
    /// The hand it is used up from; None for a mod's protector from outside the hands
    /// (`living_death_protection`), which the game takes from nowhere.
    pub hand: Option<mod_api::Hand>,
    pub stack: ItemStack,
}

/// `InteractionHand.values()`, paired with the corresponding equipment slots.
pub(crate) const PROTECTION_HANDS: [(mod_api::Hand, usize); 2] = [
    (mod_api::Hand::Main, sunrise_items::equipment::MAINHAND),
    (mod_api::Hand::Off, sunrise_items::equipment::OFFHAND),
];

/// The exact accepted hit values, only retained across the handoff for a Post listener.
#[derive(Debug)]
pub struct DamagePost {
    damage_type: String,
    causing: i32,
    direct: i32,
    direct_type: Option<String>,
    position: Option<[f64; 3]>,
    lent_entity: Option<i32>,
    state: DamageState,
}

impl DamagePost {
    /// Lend the hit's original source, final values and attacker identity to Post.
    pub fn view(&self) -> (HurtSource<'_>, &DamageState, Option<i32>) {
        let source = HurtSource {
            damage_type: &self.damage_type,
            causing: self.causing,
            direct: self.direct,
            direct_type: self.direct_type.as_deref(),
            position: self.position,
        };
        (source, &self.state, self.lent_entity)
    }
}

/// A level's random and attachments, taken out of its world while the world is read
/// elsewhere (the mob tick), and put back after.
pub struct LentLevel {
    random: sunrise_core::random::LegacyRandom,
    attachments: sunrise_mod_runtime::attachments::Attachments,
    attachment_syncs: crate::fast_hash::FxHashSet<sunrise_mod_runtime::attachments::Holder>,
}
impl LentLevel {
    /// Take `world`'s random and attachments: until [`Self::give_back`] the world has a
    /// placeholder random and none.
    pub fn take(world: &mut crate::world::World) -> Self {
        Self {
            random: std::mem::replace(
                &mut *world.random,
                sunrise_core::random::LegacyRandom::new(0),
            ),
            attachments: std::mem::take(&mut world.attachments),
            attachment_syncs: std::mem::take(&mut world.attachment_syncs),
        }
    }
    /// Put them back into `world`.
    pub fn give_back(self, world: &mut crate::world::World) {
        *world.random = self.random;
        world.attachments = self.attachments;
        world.attachment_syncs = self.attachment_syncs;
    }
}
/// `Entity.isInvulnerableTo` asked outside a hit (a mod goal's or an entity handle's query):
/// the hook's answer over `vanilla`, or `vanilla` without a listener. The world is only read,
/// and the hook's level host reads no random or attachments, so the level lends empty ones.
pub fn query_invulnerability(
    world: &crate::world::World,
    registry: &sunrise_world::registry::BlockRegistry,
    tags: Option<&crate::items::Items>,
    block_registries: Option<&crate::command::args::Registries>,
    hooks: &DamageHooks,
    hurting: Hurting<'_>,
    vanilla: bool,
) -> bool {
    let Some(hook) = hooks.invulnerable else {
        return vanilla;
    };
    let mut unread = LentLevel {
        random: sunrise_core::random::LegacyRandom::new(0),
        attachments: Default::default(),
        attachment_syncs: Default::default(),
    };
    hook(
        DamageLevel::lent(world, registry, &mut unread, tags, block_registries, hooks),
        hurting,
        vanilla,
    )
}
/// An empty level for tests and probes of hits: an air-only world whose random is seeded
/// with `seed`.
pub struct TestLevel {
    hooks: DamageHooks,
    world: crate::world::World,
    registry: sunrise_world::registry::BlockRegistry,
}
impl TestLevel {
    pub fn new(seed: i64) -> Self {
        Self::with_hooks(seed, DamageHooks::NONE)
    }
    /// [`Self::new`] whose hits run `hooks`.
    pub fn with_hooks(seed: i64, hooks: DamageHooks) -> Self {
        let facts =
            br#"{"defaults":{"minecraft:air":0},"states":[["air",true,false,"","minecraft:air"]]}"#;
        Self {
            hooks,
            world: crate::world::World::new(sunrise_world::registry::BlockStateId(0), seed),
            registry: sunrise_world::registry::BlockRegistry::from_facts_json(&facts[..]).unwrap(),
        }
    }
    pub fn level(&mut self) -> DamageLevel<'_> {
        DamageLevel::of(&mut self.world, &self.registry, None, None, &self.hooks)
    }
    /// The level's random, for a probe that draws from it outside a hit.
    pub fn random(&mut self) -> &mut sunrise_core::random::LegacyRandom {
        &mut self.world.random
    }
}
/// A knockback's strength and direction, as `LivingEntity.knockback` has them
/// before knockback resistance is applied. The entity is pushed away from
/// the direction (`x`, `z`), which vanilla points at the source of the hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Knock {
    pub strength: f64,
    pub x: f64,
    pub z: f64,
}

impl Knock {
    /// The movement write shared by the living knockback paths, after the hook returns.
    pub fn apply(
        self,
        velocity: &mut sunrise_core::geometry::Vec3,
        resistance: f64,
        on_ground: bool,
        random: &mut impl sunrise_core::random::RandomSource,
    ) -> bool {
        use sunrise_core::geometry::Vec3;

        let Self {
            strength,
            mut x,
            mut z,
        } = self;
        let power = strength * (1.0 - resistance);
        if power <= 0.0 {
            return false;
        }
        while x * x + z * z < f64::from(1.0e-5_f32) {
            x = (random.next_double() - random.next_double()) * 0.01;
            z = (random.next_double() - random.next_double()) * 0.01;
        }
        let length = (x * x + z * z).sqrt();
        let next = Vec3::new(
            velocity.x / 2.0 - x / length * power,
            if on_ground {
                (velocity.y / 2.0 + power).min(0.4)
            } else {
                velocity.y
            },
            velocity.z / 2.0 - z / length * power,
        );
        // Entity.setDeltaMovement refuses a result that isn't finite.
        if !next.is_finite() {
            return false;
        }
        *velocity = next;
        true
    }
}

/// `CombatRules.getDamageAfterAbsorb`'s weapon: the stack
/// `DamageSource.getWeaponItem` names, the registry its `armor_effectiveness`
/// enchantments (Breach) are read from.
pub struct ArmorWeapon {
    pub stack: crate::items::ItemStack,
    pub enchantments: std::sync::Arc<crate::enchant::Enchantments>,
}
impl std::fmt::Debug for ArmorWeapon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArmorWeapon")
            .field("stack", &self.stack)
            .finish_non_exhaustive()
    }
}
impl<'a> DamageLevel<'a> {
    /// What `applyItemBlocking` stops of `damage` (above 0) once [`DamageHooks::shield`] had
    /// its say on `vanilla`, the block vanilla resolved: none when the hook unblocks it.
    pub fn shield_block(
        &mut self,
        hurting: Hurting<'_>,
        damage: f32,
        vanilla: ShieldBlock,
    ) -> Option<ShieldBlock> {
        let mut block = vanilla;
        if let Some(hook) = self.hooks.shield {
            hook(
                self.without_views(),
                hurting,
                damage,
                vanilla.blocked,
                &mut block,
            );
        }
        if !block.blocked {
            return None;
        }
        // A value a mod leaves that is not a number stops nothing.
        block.blocked_damage = if block.blocked_damage.is_nan() {
            0.0
        } else {
            block.blocked_damage.clamp(0.0, damage)
        };
        Some(block)
    }
    /// The live component of an eligible hand stack, including its prototype defaults.
    pub(crate) fn death_protection<'s>(
        &self,
        stack: &'s ItemStack,
    ) -> Option<&'s [crate::items::ConsumeEffect]>
    where
        'a: 's,
    {
        if stack.count == 0 {
            return None;
        }
        stack.death_protection(self.tags.and_then(|items| items.get(&stack.item)))
    }

    /// Whether [`DamageHooks::totem`] refused this particular hand stack.
    pub(crate) fn totem_canceled(
        &mut self,
        hurting: Hurting<'_>,
        hand: mod_api::Hand,
        stack: &ItemStack,
    ) -> bool {
        self.hooks
            .totem
            .is_some_and(|hook| hook(self.without_views(), hurting, hand, stack))
    }
    /// [`DamageHooks::protector`]'s answer once neither hand protected `hurting`: a stack with
    /// `death_protection`, else nothing.
    pub(crate) fn mod_protector(&mut self, hurting: Hurting<'_>) -> Option<ItemStack> {
        let stack = self
            .hooks
            .protector
            .and_then(|hook| hook(self.without_views(), hurting))?;
        self.death_protection(&stack).is_some().then_some(stack)
    }
    /// Whether this level belongs to a logical server `Game` whose hooks
    /// apply mob death-protection and effect additions through the owning loop.
    pub fn defers_death_protection_effects(&self) -> bool {
        !self.client_side && self.hooks.totem.is_some()
    }
}
/// The block an entity with no raised item is asked about: none, and one a mod forces
/// stops the whole hit.
pub fn no_block(damage: f32) -> ShieldBlock {
    ShieldBlock {
        blocked: false,
        blocked_damage: damage,
        shield_damage: 0,
    }
}

/// What an attacker holds: the main hand and the off hand.
pub type Held = [Option<crate::items::ItemStack>; 2];

/// The vanilla class family of a hit's direct entity (`DamageSource.getDirectEntity`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DirectEntity {
    /// No direct entity, or an entity that is neither `LivingEntity` nor `Projectile`
    /// (such as primed TNT, an area-effect cloud, evoker fangs, or a vehicle).
    #[default]
    Other,
    /// `LivingEntity` (players, mobs, armor stands, the ender dragon).
    Living,
    /// `Projectile` other than `AbstractArrow` or `LargeFireball`.
    Projectile,
    /// `AbstractArrow` (arrows and thrown tridents; also a `Projectile`).
    Arrow,
    /// `LargeFireball` (ghast fireballs; also a `Projectile`).
    LargeFireball,
}
impl DirectEntity {
    pub fn is_living(self) -> bool {
        self == Self::Living
    }
    pub fn is_projectile(self) -> bool {
        matches!(self, Self::Projectile | Self::Arrow | Self::LargeFireball)
    }
    pub fn is_arrow(self) -> bool {
        self == Self::Arrow
    }
    pub fn is_large_fireball(self) -> bool {
        self == Self::LargeFireball
    }
}
/// Who attacks in a hit and what they hold, as `living_knockback` and the weapon of
/// `living_incoming_damage` name them. The site that starts the attack builds it (the game's
/// `attack_of`) and lends it with the level the hit lands in ([`DamageLevel::attacked_by`]), so
/// a nested attack has its own and nothing is left behind when it returns.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Attack {
    /// The attacking entity's id, 0 for none.
    pub attacker: i32,
    /// The projectile delivering this hit, if any (`DamageSource.getDirectEntity`).
    pub projectile: Option<i32>,
    /// The class family of [`Self::projectile`] (`AbstractArrow`, `LargeFireball`, or another `Projectile`).
    pub projectile_kind: DirectEntity,
    /// The projectile's entity type, independent of its damage source and class family.
    pub projectile_type: Option<&'static str>,
    /// An explicitly supplied direct entity, including 0 for no direct entity.
    /// None uses the normal melee/projectile factory rules.
    pub direct: Option<i32>,
    /// The class family of an explicitly supplied [`Self::direct`] entity (`Some(id)` with `id != 0`).
    /// Ignored when [`Self::direct`] is `None` or `Some(0)`.
    pub direct_kind: Option<DirectEntity>,
    /// The entity type of an explicitly supplied direct entity.
    pub direct_type: Option<String>,
    /// `DamageSource.getSourcePosition`, as supplied by the hit's factory.
    pub position: Option<[f64; 3]>,
    /// An arrow's fired-from weapon or a thrown trident's complete stack.
    pub projectile_weapon: Option<crate::items::ItemStack>,
    /// The verified causing player's UUID, when the attacker is a player.
    pub player_uuid: Option<[i32; 4]>,
    /// The causing mob's UUID for `Villager.setLastHurtByMob`'s gossip.
    pub attacker_uuid: Option<[i32; 4]>,
    /// Type and movement of the causing entity, bound before the hit borrows its victim.
    pub enchantment_attacker: Option<EnchantmentActor>,
    /// The explicit direct entity or projectile, distinct from the causing entity.
    pub enchantment_direct: Option<EnchantmentActor>,
    /// What it holds, or none when it is not known (no native mod listens: the game only
    /// records it then).
    pub held: Held,
}
/// The identity and predicate view of one entity named by a damage source.
#[derive(Clone, Debug, PartialEq)]
pub struct EnchantmentActor {
    pub id: i32,
    pub kind: std::borrow::Cow<'static, str>,
    pub motion: Option<(f64, bool)>,
}

/// The attack of a hit nobody started: environment damage, falls, a level built outside a hit.
pub static NO_ATTACK: Attack = Attack {
    attacker: 0,
    projectile: None,
    projectile_kind: DirectEntity::Other,
    projectile_type: None,
    direct: None,
    direct_kind: None,
    direct_type: None,
    position: None,
    projectile_weapon: None,
    player_uuid: None,
    attacker_uuid: None,
    enchantment_attacker: None,
    enchantment_direct: None,
    held: [None, None],
};
/// Whether vanilla's factory for `damage_type` passes the projectile as
/// `DamageSource.getDirectEntity` (`arrow`, `trident`, `thrown`, `spit`,
/// `fireball`, `unattributed_fireball`, `wither_skull`, `wind_charge`,
/// `fireworks`, `mob_projectile`, `indirect_magic`). `magic()` does not, and
/// blast damage types (`explosion`, `player_explosion`, `bad_respawn_point`) do
/// not imply a projectile either, though a blast can still supply a projectile
/// explicitly via [`Attack::direct`] / [`Attack::direct_kind`] (for example, a
/// `LargeFireball` or `WitherSkull` explosion).
pub fn names_its_projectile(damage_type: &str) -> bool {
    matches!(
        damage_type,
        "minecraft:arrow"
            | "minecraft:trident"
            | "minecraft:fireball"
            | "minecraft:unattributed_fireball"
            | "minecraft:wither_skull"
            | "minecraft:mob_projectile"
            | "minecraft:thrown"
            | "minecraft:wind_charge"
            | "minecraft:fireworks"
            | "minecraft:spit"
            | "minecraft:indirect_magic"
    )
}
impl DamageLevel<'_> {
    /// Whether [`DamageHooks::knockback`] canceled the knockback (never without it). A
    /// value it leaves that is not a finite number is put back as it was.
    pub fn knockback_canceled(&mut self, pushed: Hurting<'_>, knock: &mut Knock) -> bool {
        let Some(hook) = self.hooks.knockback else {
            return false;
        };
        let before = *knock;
        let canceled = hook(self.reborrow(), pushed, knock);
        for (now, was) in [
            (&mut knock.strength, before.strength),
            (&mut knock.x, before.x),
            (&mut knock.z, before.z),
        ] {
            if !now.is_finite() {
                *now = was;
            }
        }
        canceled
    }
}
/// Map an explicit damage type id to an engine [`crate::mobs::DamageKind`] only when
/// the kind always resolves back to that id. In particular, the engine's explosion
/// factory chooses the type based on the attacker; an explicit registry type must not.
/// Other registered types in `catalog` keep their id and tags as
/// [`crate::mobs::DamageKind::Named`]; unknown or unregistered types return `None`.
pub fn resolve_damage_kind(catalog: Option<&Catalog>, ty: &str) -> Option<crate::mobs::DamageKind> {
    use crate::mobs::{DamageKind, NamedDamage};
    Some(match ty.strip_prefix("minecraft:").unwrap_or(ty) {
        "player_attack" => DamageKind::PlayerAttack,
        "mace_smash" => DamageKind::MaceSmash,
        "spear" => DamageKind::Spear,
        "mob_attack" => DamageKind::MobAttack,
        "mob_attack_no_aggro" => DamageKind::MobAttackNoAggro,
        "arrow" => DamageKind::Arrow,
        "thrown" => DamageKind::Thrown,
        "fireball" => DamageKind::Fireball,
        "wither_skull" => DamageKind::WitherSkull,
        "fall" => DamageKind::Fall,
        "on_fire" => DamageKind::OnFire,
        "in_fire" => DamageKind::InFire,
        "hot_floor" => DamageKind::HotFloor,
        "campfire" => DamageKind::Campfire,
        "lava" => DamageKind::Lava,
        "drown" => DamageKind::Drown,
        "dry_out" => DamageKind::DryOut,
        "magic" => DamageKind::Magic,
        "sting" => DamageKind::Sting,
        "cactus" => DamageKind::Cactus,
        "sweet_berry_bush" => DamageKind::SweetBerryBush,
        "freeze" => DamageKind::Freeze,
        "in_wall" => DamageKind::InWall,
        "thorns" => DamageKind::Thorns,
        "cramming" => DamageKind::Cramming,
        "falling_anvil" => DamageKind::FallingAnvil,
        "falling_block" => DamageKind::FallingBlock,
        "fireworks" => DamageKind::Fireworks,
        "generic_kill" => DamageKind::GenericKill,
        "lightning_bolt" => DamageKind::LightningBolt,
        "out_of_world" => DamageKind::OutOfWorld,
        "outside_border" => DamageKind::OutsideBorder,
        "sonic_boom" => DamageKind::SonicBoom,
        "spit" => DamageKind::Spit,
        "stalagmite" => DamageKind::Stalagmite,
        "starve" => DamageKind::Starve,
        "sulfur_cube_hot" => DamageKind::SulfurCubeHot,
        "wind_charge" => DamageKind::WindCharge,
        "wither" => DamageKind::Wither,
        "generic" => DamageKind::Generic,
        _ => {
            return catalog
                .and_then(|c| NamedDamage::registered(c, ty))
                .map(DamageKind::Named);
        }
    })
}
