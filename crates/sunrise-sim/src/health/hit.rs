// Included in health.rs to retain its private scope.
/// What `LivingEntity.hurtServer` is about to hurt, for [`DamageHooks::incoming`], or
/// `LivingEntity.knockback` to push, for [`DamageHooks::knockback`]: the entity and the
/// hit's source.
#[derive(Clone, Copy)]
pub enum Hurting<'a> {
    /// A player, by its entity id.
    Player(i32, HurtSource<'a>),
    /// A mob, and what hurts it.
    Mob(&'a crate::mobs::Mob, &'a crate::mobs::Damage),
    /// The ender dragon, by its entity id.
    Dragon(i32, HurtSource<'a>),
    /// An armor stand, by its entity id, for sweep knockback.
    ArmorStand(i32, HurtSource<'a>),
    /// A non-living entity, by its entity id and registry type.
    Entity(i32, &'a str, HurtSource<'a>),
}
pub use sunrise_world::living_damage::{DamageContainer, Reduction};
/// The native hooks a hit runs, owned by the game and lent to the level the hit lands in
/// ([`DamageLevel`]), so no hook is a global. A table with no entry (`NONE`) runs none.
#[derive(Clone, Copy)]
pub struct DamageHooks {
    /// Override vanilla's per-source entity immunity decision.
    pub invulnerable: Option<fn(DamageLevel<'_>, Hurting<'_>, bool) -> bool>,
    /// `native_living_incoming_damage`: `LivingEntity.hurtServer` once its guards passed. It
    /// may change the container's new damage and post-attack invulnerability ticks; true
    /// cancels the hit.
    pub incoming: Option<fn(DamageLevel<'_>, Hurting<'_>, Victim, &mut DamageContainer) -> bool>,
    /// `native_living_damage_pre`: `LivingEntity.actuallyHurt` once armor and magic took their
    /// share of a hit, before the absorption hearts (`LivingDamageEvent.Pre`). It may change
    /// the new damage.
    pub pre: Option<fn(DamageLevel<'_>, Hurting<'_>, &mut DamageContainer)>,
    /// `native_living_damage_post`: the end of `hurtServer` for a hit that passed the incoming
    /// check and the cooldown, with the health damage it took, 0 included
    /// (`LivingDamageEvent.Post`).
    pub post: Option<fn(DamageLevel<'_>, Hurting<'_>, &DamageContainer)>,
    /// Present when the caller has a game owner to finish death or player protection.
    /// The predicate says whether that continuation also needs the hit values for Post.
    pub owner_post_listened: Option<fn() -> bool>,
    /// `native_living_knockback`: `LivingEntity.knockback`, at its start
    /// (`LivingKnockBackEvent`), on the raw strength and before the knockback resistance and
    /// the motion change. It may change the [`Knock`]; true cancels it.
    pub knockback: Option<fn(DamageLevel<'_>, Hurting<'_>, &mut Knock) -> bool>,
    /// `LivingUseTotemEvent` for one eligible stack. True refuses this hand only.
    pub totem: Option<fn(DamageLevel<'_>, Hurting<'_>, mod_api::Hand, &ItemStack) -> bool>,
    /// `native_living_death_protection`: a protector a mod keeps outside the hands, asked once
    /// neither hand protected. The game takes the stack from nowhere.
    pub protector: Option<fn(DamageLevel<'_>, Hurting<'_>) -> Option<ItemStack>>,
    /// `native_living_shield_block`: `LivingEntity.applyItemBlocking` of a hit with damage,
    /// for what it hurts, the damage and whether vanilla blocks. It may change the
    /// [`ShieldBlock`].
    pub shield: Option<fn(DamageLevel<'_>, Hurting<'_>, f32, bool, &mut ShieldBlock)>,
    /// Retain an accepted source only when a death/loot continuation needs it.
    pub accepted: Option<fn(Hurting<'_>, &Attack) -> Option<AcceptedHit>>,
}
impl DamageHooks {
    /// No hooks: what tests of the vanilla behaviour hit through.
    pub const NONE: Self = Self {
        invulnerable: None,
        incoming: None,
        pre: None,
        post: None,
        owner_post_listened: None,
        knockback: None,
        totem: None,
        protector: None,
        shield: None,
        accepted: None,
    };
    /// The hooks a client asks while it predicts a mob it rides: only the invulnerability
    /// check, which fires on both sides; the others are the server's.
    pub const fn client_side(self) -> Self {
        Self {
            invulnerable: self.invulnerable,
            ..Self::NONE
        }
    }
}
/// A sound a hit's listener queued, with the seed drawn at the call.
pub struct HitSound<'s> {
    pub dimension: &'static str,
    pub sound: &'s str,
    pub at: [f64; 3],
    pub source: i32,
    pub volume: f32,
    pub pitch: f32,
    pub seed: i64,
    /// The entity the sound follows (`Level.playSound(null, entity, ...)`), if any.
    pub emitter: Option<i32>,
}
/// Where a hit's sounds are queued until the host is free: the game's own level sound queue.
pub trait SoundQueue {
    fn queue(&mut self, sound: HitSound<'_>);
}
impl<S: for<'s> From<HitSound<'s>>> SoundQueue for Vec<S> {
    fn queue(&mut self, sound: HitSound<'_>) {
        self.push(sound.into());
    }
}
/// The entity a hit's site lends its listeners: the live borrow of its whole body, for its own
/// random and its reads ([`HitEffects::lent_body`]). The site names who (the attacker it holds
/// a `&mut` to); nothing is looked up.
pub struct HitLend<'a> {
    body: LentBody<'a>,
}
enum LentBody<'a> {
    Mob(&'a mut crate::mobs::Mob),
    /// The player in place, with the effects and stacks the game keeps beside its body.
    Player {
        motion: &'a mut PlayerMotion,
        effects: &'a crate::effects::PlayerEffects,
        equipment: PlayerEquipment<'a>,
    },
    /// A guest not in place, read from its hub record: lent for its reads, not its random.
    Parked(&'a crate::lan::guest::Guest),
}
/// `ServerPlayer.canHarmPlayer`: whether the player `attacker` may hurt the player `victim`
/// (entity ids). Never while the `pvp` game rule is off (`isPvpAllowed`), then
/// `Player.canHarmPlayer`'s team check: allies hurt each other only when their team allows
/// friendly fire. Vanilla asks it both ways round, with the same answer: the victim's
/// `hurtServer` of the attacker, `AbstractArrow.canHitEntity` of the owner. Every
/// player-on-player rule asks it here: a melee blow and its sweep, an arrow, another player's
/// projectile, a trident, a spear's stab, a blast the player caused (a primed TNT they own, an
/// end crystal they broke) and the `PlayerRef::can_harm` read. NeoForge and
/// Fabric have no event at this method, so a Java mod mixes into `ServerPlayer.canHarmPlayer`;
/// a native mod answers here through `[[native.mixin]]` (#4854).
#[sunrise_mod_api_macros::replaceable]
pub fn can_harm_player(
    attacker: i32,
    victim: i32,
    pvp: bool,
    attacker_team: Option<&crate::scoreboard::Team>,
    victim_team: Option<&crate::scoreboard::Team>,
) -> bool {
    let _ = (attacker, victim);
    pvp && crate::scoreboard::teams_allow_damage(victim_team, attacker_team)
}
/// Whether any native mod listens to a hook whose host is a `HitHost`. A site looks up its
/// attacker only then; without one nothing is looked up, borrowed or drawn.
pub fn hit_host_listened() -> bool {
    use mod_api::{__private::listened, HookId};
    [
        HookId::LivingIncomingDamage,
        HookId::LivingDamagePre,
        HookId::LivingDamagePost,
        HookId::LivingShieldBlock,
        HookId::LivingUseTotem,
    ]
    .into_iter()
    .any(listened)
}
impl<'a> HitLend<'a> {
    pub fn mob(mob: &'a mut crate::mobs::Mob) -> Option<Self> {
        Some(Self {
            body: LentBody::Mob(mob),
        })
    }
    /// `player`, if it is the attacker `attacker` (an entity id), with the `effects` and
    /// `equipment` the game keeps beside the body in place. The caller builds it only while a
    /// `HitHost` hook listens ([`hit_host_listened`]).
    pub fn attacking_player(
        player: Option<&'a mut PlayerMotion>,
        attacker: i32,
        effects: &'a crate::effects::PlayerEffects,
        equipment: PlayerEquipment<'a>,
    ) -> Option<Self> {
        let motion = player.filter(|p| p.entity_id == attacker)?;
        Some(Self {
            body: LentBody::Player {
                motion,
                effects,
                equipment,
            },
        })
    }
    /// The guest `conn` of `hub`, which is not in place: its record, for reading only. The caller
    /// names a guest the `LivingRef` reads resolve to its hub record at this moment.
    pub fn parked_guest(hub: Option<&'a crate::lan::hub::Hub>, conn: u32) -> Option<Self> {
        Some(Self {
            body: LentBody::Parked(hub?.guest(conn)?),
        })
    }
    fn id(&self) -> i32 {
        match &self.body {
            LentBody::Mob(mob) => mob.id as i32,
            LentBody::Player { motion, .. } => motion.entity_id,
            LentBody::Parked(guest) => guest.entity_id,
        }
    }
    /// The lent entity's position and own random, for its sounds and draws; none for a parked
    /// guest, whose random the hit does not hold.
    fn position_and_random(
        &mut self,
    ) -> Option<([f64; 3], &mut sunrise_core::random::LegacyRandom)> {
        let (position, random) = match &mut self.body {
            LentBody::Mob(mob) => (mob.position, &mut mob.random),
            LentBody::Player { motion, .. } => (motion.position, &mut *motion.random),
            LentBody::Parked(_) => return None,
        };
        Some(([position.x, position.y, position.z], random))
    }
    fn body(&self) -> LivingBody<'_> {
        match &self.body {
            LentBody::Mob(mob) => LivingBody::Mob(mob),
            LentBody::Player {
                motion,
                effects,
                equipment,
            } => LivingBody::Player(PlayerBody {
                id: motion.entity_id,
                uuid: None,
                health: PlayerHealthRead::Synced(&motion.health.data, motion.health.ids),
                attributes: Some(&motion.attributes),
                effects: Some(effects),
                equipment: *equipment,
                attachments: None,
            }),
            LentBody::Parked(guest) => LivingBody::Player(PlayerBody::of_record(guest)),
        }
    }
    fn reborrow(&mut self) -> HitLend<'_> {
        HitLend {
            body: match &mut self.body {
                LentBody::Mob(mob) => LentBody::Mob(mob),
                LentBody::Player {
                    motion,
                    effects,
                    equipment,
                } => LentBody::Player {
                    motion,
                    effects,
                    equipment: *equipment,
                },
                LentBody::Parked(guest) => LentBody::Parked(guest),
            },
        }
    }
}
/// What a hit lends its listeners beyond the level's blocks and random: the level sound queue
/// and sound-seed stream, and the own random of the entity its site lends ([`HitLend`]). The
/// site builds it from borrows it already holds ([`DamageLevel::with_effects`]); a level built
/// without one refuses every request.
pub struct HitEffects<'a> {
    sounds: &'a mut (dyn SoundQueue + 'a),
    /// The seed stream of `Level.playSound`; no host, no sound can be sent.
    hub: Option<&'a crate::lan::hub::Hub>,
    dimension: &'static str,
    /// The open of the world the lent entity belongs to, to refuse a stale handle.
    epoch: Option<mod_api::WorldEpoch>,
    lent: Option<HitLend<'a>>,
}
impl<'a> HitEffects<'a> {
    pub fn new(
        sounds: &'a mut (dyn SoundQueue + 'a),
        hub: Option<&'a crate::lan::hub::Hub>,
        dimension: &'static str,
        epoch: Option<mod_api::WorldEpoch>,
        lent: Option<HitLend<'a>>,
    ) -> Self {
        Self {
            sounds,
            hub,
            dimension,
            epoch,
            lent,
        }
    }
    fn reborrow(&mut self) -> HitEffects<'_> {
        HitEffects {
            sounds: &mut *self.sounds,
            hub: self.hub,
            dimension: self.dimension,
            epoch: self.epoch,
            lent: self.lent.as_mut().map(HitLend::reborrow),
        }
    }
    /// Queue `sound` (already validated) at `at`, drawing its seed from the level's sound-seed
    /// stream now (`Level.soundSeedGenerator.nextLong`), never from the level's random. False
    /// where no host can send it.
    pub fn play(
        &mut self,
        sound: &str,
        at: [f64; 3],
        source: mod_api::SoundSource,
        volume: f32,
        pitch: f32,
        emitter: Option<i32>,
    ) -> bool {
        let Some(hub) = self.hub else {
            return false;
        };
        self.sounds.queue(HitSound {
            dimension: self.dimension,
            sound,
            at,
            source: source.index() as i32,
            volume,
            pitch,
            seed: hub.next_sound_seed(),
            emitter,
        });
        true
    }
    /// The lent entity `entity` names: its id, position and own random. None for a stale
    /// handle or an entity the hit does not lend.
    pub fn entity(
        &mut self,
        entity: mod_api::EntityRef,
    ) -> Option<(i32, [f64; 3], &mut sunrise_core::random::LegacyRandom)> {
        if self.epoch != Some(entity.epoch()) {
            return None;
        }
        let lent = self.lent.as_mut()?;
        let id = lent.id();
        (i32::try_from(entity.id()).ok()? == id).then_some(())?;
        let (position, random) = lent.position_and_random()?;
        Some((id, position, random))
    }
    /// The body of the entity the hit lends, for its reads; none where it lends none.
    pub fn lent_body(&self) -> Option<LivingBody<'_>> {
        self.lent.as_ref().map(HitLend::body)
    }
}
/// A living entity's body as a read borrows it, whoever holds it: a hit (its victim, its lent
/// causer) or the game's handle reads. The game reads each field of it through one function per
/// field, so a hit's [`LivingView`](mod_api::LivingView) and a [`mod_api::LivingRef`] read agree.
#[derive(Clone, Copy)]
pub enum LivingBody<'a> {
    /// A mob.
    Mob(&'a crate::mobs::Mob),
    /// A player: the one in place, a guest's record, or the body a hit's adapter lends.
    Player(PlayerBody<'a>),
    /// An armor stand.
    Stand(FixedBody<'a>),
    /// The ender dragon.
    Dragon(FixedBody<'a>),
}
/// The fields an armor stand or the dragon keeps of a living entity.
#[derive(Clone, Copy)]
pub struct FixedBody<'a> {
    pub id: u32,
    pub health: &'a f32,
    pub live: &'a crate::living_attributes::LivingAttributes,
    pub equipment: &'a sunrise_items::equipment::Equipment,
}
/// Where a player's health and absorption are read.
#[derive(Clone, Copy)]
pub enum PlayerHealthRead<'a> {
    /// The synced floats of a body in place (`LivingEntity.DATA_HEALTH_ID`,
    /// `Player.DATA_PLAYER_ABSORPTION_ID`).
    Synced(&'a SyncedFloats, PlayerFloatIds),
    /// A parked guest's hub record: its stored health, and the absorption of its body, or the
    /// saved amount while it has none.
    Record {
        health: &'a f32,
        body: Option<&'a PlayerHealth>,
        saved_absorption: &'a Option<f32>,
    },
}
/// A player's body: health, attributes, effects and equipment, each borrowed where it lives.
#[derive(Clone, Copy)]
pub struct PlayerBody<'a> {
    pub id: i32,
    /// The player's UUID, the holder of its attachments, when the borrower knows it.
    pub uuid: Option<u128>,
    pub health: PlayerHealthRead<'a>,
    pub attributes: Option<&'a Attributes>,
    pub effects: Option<&'a crate::effects::PlayerEffects>,
    /// The stacks in vanilla `EquipmentSlot` order.
    pub equipment: PlayerEquipment<'a>,
    /// The player's attachments, where the body keeps them apart from the player in place's
    /// (a parked guest's hub row); none for the player in place.
    pub attachments: Option<&'a sunrise_mod_runtime::attachments::Attachments>,
}
impl<'a> PlayerBody<'a> {
    /// The player in place's body: the floats and attributes its holder lends, and the effects,
    /// stacks and UUID the game keeps beside it (`reads`).
    pub fn in_place(
        id: i32,
        body: PlayerBodyRead<'a, Attributes>,
        reads: Option<&HitReads<'a>>,
    ) -> Self {
        Self {
            id,
            uuid: reads.and_then(|reads| reads.player_uuid),
            health: PlayerHealthRead::Synced(body.data, body.ids),
            attributes: Some(body.attributes),
            effects: reads.and_then(|reads| reads.player_effects),
            equipment: reads.map_or([None; 8], |reads| reads.player_equipment),
            attachments: None,
        }
    }
    /// A guest's body as its hub record keeps it while the guest is not in place: its stored
    /// health, the attributes of its body, its effects, stacks and attachments.
    pub fn of_record(g: &'a crate::lan::guest::Guest) -> Self {
        Self {
            id: g.entity_id,
            uuid: Some(g.uuid),
            health: PlayerHealthRead::Record {
                health: &g.health,
                body: g.motion.as_deref().map(|m| &m.health),
                saved_absorption: &g.saved_absorption,
            },
            attributes: g.motion.as_deref().map(|m| &m.attributes),
            effects: Some(&g.effects),
            equipment: player_equipment(
                &g.inventory,
                g.selected,
                &g.armor,
                &g.offhand,
                &g.equipment,
            ),
            attachments: Some(&g.attachments),
        }
    }
}
/// A player's stacks in vanilla `EquipmentSlot` order, from its inventory, armour (head first)
/// and off hand.
pub fn player_equipment<'a>(
    inventory: &'a [Option<ItemStack>],
    selected: usize,
    armor: &'a [Option<ItemStack>; 4],
    offhand: &'a Option<ItemStack>,
    equipment: &'a sunrise_items::equipment::Equipment,
) -> PlayerEquipment<'a> {
    use sunrise_items::equipment::{BODY, SADDLE};
    [
        inventory.get(selected).and_then(Option::as_ref),
        offhand.as_ref(),
        armor[3].as_ref(),
        armor[2].as_ref(),
        armor[1].as_ref(),
        armor[0].as_ref(),
        equipment.get(BODY),
        equipment.get(SADDLE),
    ]
}
/// The registry keys of the living types that are not mob types (`EntityType.PLAYER`,
/// `ARMOR_STAND`, `ENDER_DRAGON`), resolved with the accepted resources, as a mob type keeps its
/// own ([`crate::mobs::MobType::type_key`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LivingTypeKeys {
    pub player: Option<mod_api::EntityTypeKey>,
    pub armor_stand: Option<mod_api::EntityTypeKey>,
    pub ender_dragon: Option<mod_api::EntityTypeKey>,
}
impl LivingTypeKeys {
    /// The keys of the registry resolved now.
    pub fn resolve() -> Self {
        let key = mod_api::__private::resolved_entity_type_key;
        Self {
            player: key("minecraft:player"),
            armor_stand: key("minecraft:armor_stand"),
            ender_dragon: key("minecraft:ender_dragon"),
        }
    }
}
/// What a hit's listeners need to name and read the living entities it lends: the open of the
/// world, the attribute catalog, the player in place's effects, stacks, UUID and attachments, and the
/// entities' attachment maps, all lent for reading. Built by the game at the site; a level
/// built without it lends no entity.
#[derive(Clone, Copy)]
pub struct HitReads<'a> {
    pub epoch: mod_api::WorldEpoch,
    /// The accepted resources' keys of the player, the armor stand and the dragon.
    pub living_types: LivingTypeKeys,
    pub attributes: &'a std::sync::Arc<sunrise_world::attributes::entity::Catalog>,
    pub player_effects: Option<&'a crate::effects::PlayerEffects>,
    /// The player in place's stacks, in vanilla `EquipmentSlot` order.
    pub player_equipment: PlayerEquipment<'a>,
    /// The acting player's UUID, the holder of its attachments; none outside a player's turn.
    pub player_uuid: Option<u128>,
    pub player_attachments: Option<&'a sunrise_mod_runtime::attachments::Attachments>,
    pub entity_attachments:
        Option<&'a crate::fast_hash::FxHashMap<u32, sunrise_mod_runtime::attachments::Attachments>>,
}
/// The body a heal or movement hook lends: a mob, or the player in place, whose effects, stacks
/// and UUID come from the reads (as a hit's player victim's do, [`PlayerBody::in_place`]).
#[derive(Clone, Copy)]
pub enum LendBody<'a> {
    Mob(&'a crate::mobs::Mob),
    Player {
        id: i32,
        attributes: &'a Attributes,
        data: &'a SyncedFloats,
        ids: PlayerFloatIds,
    },
}
/// A living entity a heal or movement hook lends its listeners for reading
/// ([`LivingView`](mod_api::LivingView)): the body the site holds, and what naming and reading it
/// needs beyond the body, which the game builds only once a listener is subscribed.
#[derive(Clone, Copy)]
pub struct LivingLend<'a> {
    pub body: LendBody<'a>,
    pub reads: &'a dyn Fn() -> Option<HitReads<'a>>,
}
impl<'a> LivingLend<'a> {
    /// A mob's lend.
    pub fn mob(mob: &'a crate::mobs::Mob, reads: &'a dyn Fn() -> Option<HitReads<'a>>) -> Self {
        Self {
            body: LendBody::Mob(mob),
            reads,
        }
    }
    /// The player in place's lend: its floats and attributes as the site holds them.
    pub fn player(
        id: i32,
        attributes: &'a Attributes,
        data: &'a SyncedFloats,
        ids: PlayerFloatIds,
        reads: &'a dyn Fn() -> Option<HitReads<'a>>,
    ) -> Self {
        Self {
            body: LendBody::Player {
                id,
                attributes,
                data,
                ids,
            },
            reads,
        }
    }
    /// The entity's id (`Entity.getId`).
    pub fn id(&self) -> u32 {
        match self.body {
            LendBody::Mob(m) => m.id,
            LendBody::Player { id, .. } => id as u32,
        }
    }
    /// The entity's type name.
    pub fn kind(&self) -> &'a str {
        match self.body {
            LendBody::Mob(m) => &m.kind.id,
            LendBody::Player { .. } => "minecraft:player",
        }
    }
    /// The body its reads read, completed from `reads`.
    pub fn living_body(&self, reads: &HitReads<'a>) -> LivingBody<'a> {
        match self.body {
            LendBody::Mob(m) => LivingBody::Mob(m),
            LendBody::Player {
                id,
                attributes,
                data,
                ids,
            } => LivingBody::Player(PlayerBody::in_place(
                id,
                PlayerBodyRead {
                    attributes,
                    data,
                    ids,
                },
                Some(reads),
            )),
        }
    }
}
/// Inputs to `Entity.isInvulnerableToBase` for a mob or a nonliving body.
pub struct BodyImmunity<T> {
    pub removed: bool,
    pub invulnerable: bool,
    pub creative_source: bool,
    pub fire_immune: bool,
    pub fall_immune: bool,
    pub source_has_tag: T,
}

impl<T: FnMut(ImmunityTag) -> bool> BodyImmunity<T> {
    pub fn vanilla(mut self) -> bool {
        match damage_immunity::vanilla_base_immunity(&mut self) {
            Ok(answer) => answer,
            Err(never) => match never {},
        }
    }
}

impl<T: FnMut(ImmunityTag) -> bool> EntityImmunityWorld for BodyImmunity<T> {
    type Error = std::convert::Infallible;

    fn removed(&self) -> bool {
        self.removed
    }
    fn permanently_invulnerable(&mut self) -> std::result::Result<bool, Self::Error> {
        Ok(self.invulnerable)
    }
    fn temporarily_invulnerable(&mut self) -> std::result::Result<bool, Self::Error> {
        Ok(false)
    }
    fn tag(&mut self, tag: ImmunityTag) -> std::result::Result<bool, Self::Error> {
        Ok((self.source_has_tag)(tag))
    }
    fn creative_source(&mut self) -> std::result::Result<bool, Self::Error> {
        Ok(self.creative_source)
    }
    fn fire_immune(&mut self) -> std::result::Result<bool, Self::Error> {
        Ok(self.fire_immune)
    }
    fn fall_immune(&mut self) -> std::result::Result<bool, Self::Error> {
        Ok(self.fall_immune)
    }
}

/// The acting player's stacks borrowed in vanilla equipment slot order.
pub type PlayerEquipment<'a> = [Option<&'a ItemStack>; sunrise_items::equipment::SLOT_NAMES.len()];

/// The level a hit lands in, as damage code sees it: its blocks, its random
/// (`Level.random`) and its own attachments, lent to the damage hooks as their level host.
/// Built from the level's world, or, in the mob tick, which reads the world while mobs
/// draw, from a [`LentLevel`] taken out of it for the tick.
pub struct DamageLevel<'a> {
    shape: crate::chunk::LevelShape,
    chunks: &'a crate::chunk_map::ChunkMap,
    air: sunrise_world::registry::BlockStateId,
    game_time: i64,
    paused_mob_hurt: Option<&'a mut Option<Box<crate::mobs::PausedMobHurt>>>,
    registry: &'a sunrise_world::registry::BlockRegistry,
    tags: Option<&'a crate::items::Items>,
    block_registries: Option<&'a crate::command::args::Registries>,
    random: &'a mut sunrise_core::random::LegacyRandom,
    attachments: &'a mut sunrise_mod_runtime::attachments::Attachments,
    attachment_syncs: &'a mut crate::fast_hash::FxHashSet<sunrise_mod_runtime::attachments::Holder>,
    /// The game's hooks; on a client predicting its own mount, only the both-side ones
    /// ([`DamageHooks::client_side`]).
    hooks: &'a DamageHooks,
    /// Who attacks in this hit and what they hold: the facts of the site that built the level.
    attack: &'a Attack,
    /// Canonical UUIDs for attacks first constructed during a borrowed mob tick.
    mob_uuids: Option<&'a std::collections::HashMap<u32, [i32; 4]>>,
    /// Whether this is a client's level, for a hook asked on both sides ([`Self::on_client`]).
    client_side: bool,
    /// Who a push credits: the site's per-victim loan, or [`KnockbackCredit`]'s memory
    /// resolution where the level was built without one.
    credit: Credit<'a>,
    /// Resolves a mob's remembered player where no loan does ([`Self::resolve_player_responsible`]).
    players: Option<(PlayerResolver<'a>, &'a mut dyn PlayerMemory)>,
    /// What the hit's listeners may queue and draw ([`HitEffects`]); none for a level built
    /// outside a game's hit.
    effects: Option<HitEffects<'a>>,
    player_equipment: PlayerEquipment<'a>,
    /// What the hit's listeners need to name and read the entities it lends ([`HitReads`]).
    reads: Option<HitReads<'a>>,
    /// The living body the hit lands on, where the hurt code lends it ([`Self::with_victim`]).
    victim: Option<LivingBody<'a>>,
    /// The entity id the hook's source names as its causing entity (`DamageSource.getEntity`),
    /// 0 for none: the lent attacker is the hit's causer only when it is that entity.
    causing: i32,
    /// The world's frozen mob variant tables, for variant-backed hurt and death sounds
    /// (`Wolf.getHurtSound`/`getDeathSound`, `Cat.getHurtSound`/`getDeathSound`).
    variants: Option<&'a crate::mobs::variants::VariantTables>,
}
impl<'a> DamageLevel<'a> {
    /// The level of `world`, whose blocks `registry` names.
    pub fn of(
        world: &'a mut crate::world::World,
        registry: &'a sunrise_world::registry::BlockRegistry,
        tags: Option<&'a crate::items::Items>,
        block_registries: Option<&'a crate::command::args::Registries>,
        hooks: &'a DamageHooks,
    ) -> Self {
        let crate::world::World {
            chunks,
            shape,
            air,
            game_time,
            random,
            attachments,
            attachment_syncs,
            ..
        } = world;
        Self {
            shape: *shape,
            chunks,
            air: *air,
            game_time: *game_time,
            paused_mob_hurt: None,
            registry,
            tags,
            block_registries,
            random,
            attachments,
            attachment_syncs,
            hooks,
            attack: &NO_ATTACK,
            mob_uuids: None,
            client_side: false,
            credit: Credit::Memory,
            players: None,
            effects: None,
            player_equipment: [None; 8],
            reads: None,
            victim: None,
            causing: 0,
            variants: None,
        }
    }
    /// The level of `world`, with the random and attachments `lent` took out of it.
    pub fn lent(
        world: &'a crate::world::World,
        registry: &'a sunrise_world::registry::BlockRegistry,
        lent: &'a mut LentLevel,
        tags: Option<&'a crate::items::Items>,
        block_registries: Option<&'a crate::command::args::Registries>,
        hooks: &'a DamageHooks,
    ) -> Self {
        Self {
            shape: world.shape,
            chunks: &world.chunks,
            air: world.air,
            game_time: world.game_time,
            paused_mob_hurt: None,
            registry,
            tags,
            block_registries,
            random: &mut lent.random,
            attachments: &mut lent.attachments,
            attachment_syncs: &mut lent.attachment_syncs,
            hooks,
            attack: &NO_ATTACK,
            mob_uuids: None,
            client_side: false,
            credit: Credit::Memory,
            players: None,
            effects: None,
            player_equipment: [None; 8],
            reads: None,
            victim: None,
            causing: 0,
            variants: None,
        }
    }
    /// The same level, whose hit lends `lend` (its causing entity) to its listeners, where the
    /// level carries what a hit lends at all ([`Self::with_effects`]).
    pub fn lending(mut self, lend: Option<HitLend<'a>>) -> Self {
        if let Some(effects) = self.effects.as_mut() {
            effects.lent = lend;
        }
        self
    }
    /// Lend the UUID owner when mobs can start attacks inside this borrow.
    pub fn with_mob_uuids(mut self, uuids: &'a std::collections::HashMap<u32, [i32; 4]>) -> Self {
        self.mob_uuids = Some(uuids);
        self
    }
    pub(crate) fn mob_uuid(&self, id: u32) -> Option<[i32; 4]> {
        self.mob_uuids?.get(&id).copied()
    }
    /// Attach the world's frozen mob variant tables for variant-backed hurt and death sounds.
    pub fn with_variants(mut self, variants: &'a crate::mobs::variants::VariantTables) -> Self {
        self.variants = Some(variants);
        self
    }
    pub(crate) fn variants(&self) -> Option<&crate::mobs::variants::VariantTables> {
        self.variants
    }
    /// The same level, for a shorter borrow.
    pub fn reborrow(&mut self) -> DamageLevel<'_> {
        DamageLevel {
            shape: self.shape,
            chunks: self.chunks,
            air: self.air,
            game_time: self.game_time,
            paused_mob_hurt: self.paused_mob_hurt.as_deref_mut(),
            registry: self.registry,
            tags: self.tags,
            block_registries: self.block_registries,
            random: self.random,
            attachments: self.attachments,
            attachment_syncs: self.attachment_syncs,
            hooks: self.hooks,
            attack: self.attack,
            mob_uuids: self.mob_uuids,
            client_side: self.client_side,
            credit: self.credit.reborrow(),
            players: self
                .players
                .as_mut()
                .map(|(resolver, memory)| (*resolver, &mut **memory as &mut dyn PlayerMemory)),
            effects: self.effects.as_mut().map(HitEffects::reborrow),
            player_equipment: self.player_equipment,
            reads: self.reads,
            victim: self.victim,
            causing: self.causing,
            variants: self.variants,
        }
    }
    /// The owner's slot for a hit that must release its borrows before resuming.
    pub fn with_mob_hurt_handoff(
        mut self,
        paused: &'a mut Option<Box<crate::mobs::PausedMobHurt>>,
    ) -> Self {
        self.paused_mob_hurt = Some(paused);
        self
    }

    pub(crate) fn can_pause_mob_hurt(&self) -> bool {
        self.paused_mob_hurt.is_some()
    }

    pub(crate) fn pause_mob_hurt(&mut self, hit: crate::mobs::PausedMobHurt) {
        let slot = self
            .paused_mob_hurt
            .as_deref_mut()
            .expect("owning hit slot");
        debug_assert!(slot.is_none());
        *slot = Some(Box::new(hit));
    }

    pub(crate) fn paused_mob_hurt(&mut self) -> &mut crate::mobs::PausedMobHurt {
        self.paused_mob_hurt
            .as_deref_mut()
            .and_then(Option::as_deref_mut)
            .expect("paused hit")
    }

    pub(crate) fn lent_attacker(&self) -> Option<i32> {
        self.effects.as_ref()?.lent.as_ref().map(HitLend::id)
    }

    pub(crate) fn wake_mob(&self, mob: &mut crate::mobs::Mob) {
        // Standalone hit simulations have no live bed to update.
        crate::mobs::villager::wake(mob, self.game_time, None, None);
    }

    /// Lend the stacks worn by this hit's player victim for enchantment immunity.
    pub fn with_player_equipment(self, player_equipment: PlayerEquipment<'a>) -> Self {
        Self {
            player_equipment,
            ..self
        }
    }
    /// The same level whose listeners name and read the entities it lends through `reads`.
    pub fn with_reads(self, reads: Option<HitReads<'a>>) -> Self {
        Self { reads, ..self }
    }
    /// What the hit's listeners need to name and read the entities it lends.
    pub fn reads(&self) -> Option<HitReads<'a>> {
        self.reads
    }
    /// The same level, whose hit lands on `victim`'s body: the player, armor stand or dragon
    /// the hurt code holds (a mob's comes with its [`Hurting::Mob`]).
    pub fn with_victim(self, victim: LivingBody<'a>) -> Self {
        Self {
            victim: Some(victim),
            ..self
        }
    }
    /// The body the hit lands on, where the hurt code lends it.
    pub fn victim(&self) -> Option<LivingBody<'a>> {
        self.victim
    }
    /// The same level, for a hook whose source names `causing` (an entity id, 0 for none) as
    /// its causing entity.
    pub fn caused_by(self, causing: i32) -> Self {
        Self { causing, ..self }
    }
    /// The body of the hit's causing entity: the entity the site lends, when it is the one the
    /// hook's source names and it is not removed. None for any other.
    pub fn causing_body(&self) -> Option<LivingBody<'_>> {
        let body = self.effects.as_ref()?.lent_body()?;
        let id = match body {
            LivingBody::Mob(mob) if mob.removed => return None,
            LivingBody::Mob(mob) => mob.id as i32,
            LivingBody::Player(player) => player.id,
            LivingBody::Stand(_) | LivingBody::Dragon(_) => return None,
        };
        (self.causing != 0 && id == self.causing).then_some(body)
    }
    /// The level for a hook that lends no living views (`living_shield_block`,
    /// `living_use_totem`): no victim and no causing entity.
    fn without_views(&mut self) -> DamageLevel<'_> {
        let mut level = self.reborrow();
        level.victim = None;
        level.causing = 0;
        level
    }
    /// The level for a hook on `hurting`: its victim is the body lent with
    /// [`Self::with_victim`], or the mob [`Hurting::Mob`] holds.
    fn for_victim<'s>(&'s mut self, hurting: Hurting<'s>) -> DamageLevel<'s> {
        let level = self.reborrow();
        match (level.victim, hurting) {
            (None, Hurting::Mob(mob, _)) => level.with_victim(LivingBody::Mob(mob)),
            _ => level,
        }
    }
    /// The same level whose hit lends `effects`.
    pub fn with_effects(self, effects: HitEffects<'a>) -> Self {
        Self {
            effects: Some(effects),
            ..self
        }
    }
    /// What this hit lends its listeners, for reading.
    pub fn hit_effects_ref(&self) -> Option<&HitEffects<'a>> {
        self.effects.as_ref()
    }
    /// What this hit lends its listeners: level sounds and an entity's random.
    pub fn hit_effects(&mut self) -> Option<&mut HitEffects<'a>> {
        self.effects.as_mut()
    }
    /// Lend the narrow credit context at a site that can make a living push.
    pub fn with_knockback_credit(mut self, credit: &'a mut dyn KnockbackCredit) -> Self {
        self.credit = Credit::Loan(credit);
        self
    }
    /// Name a site that never makes a living push (a damage type tagged `no_knockback`, or a
    /// check that pushes nothing).
    pub fn no_push(mut self) -> Self {
        self.credit = Credit::NoPush;
        self
    }
    /// Lend the borrowed tick's remembered player references and their live resolver.
    pub fn with_player_resolver(
        mut self,
        players: PlayerResolver<'a>,
        memory: &'a mut dyn PlayerMemory,
    ) -> Self {
        self.players = Some((players, memory));
        self
    }
    /// `LivingEntity.baseTick`: the last tick with a positive clock still keeps the
    /// reference. Clear it at this point only if the clock was already zero.
    pub(crate) fn tick_player_memory(&mut self, mob: &mut crate::mobs::Mob) {
        if mob.ai.last_hurt_by_player > 0 {
            mob.ai.last_hurt_by_player -= 1;
        } else if let Some((_, memory)) = &mut self.players {
            memory.forget(mob.id);
        }
    }
    /// Commit this accepted hit's responsibility at vanilla's pre-push memory point.
    pub fn accepted_mob_hit(&mut self, mob: &mut crate::mobs::Mob, source: &crate::mobs::Damage) {
        // The hit's own source, whatever the credit kind.
        let uuid = match (&self.credit, source.attacker) {
            _ if mob.species() != crate::mobs::Species::Villager => None,
            (Credit::Loan(credit), _) => credit.hurt_by_uuid(source),
            (_, Some(crate::mobs::Who::Player)) => self.attack.player_uuid,
            (_, Some(crate::mobs::Who::Mob(id))) if self.attack.attacker == id as i32 => {
                self.attack.attacker_uuid
            }
            _ => None,
        };
        if let Some(uuid) = uuid {
            crate::mobs::villager::hurt_by(mob, source, uuid);
        }
        if let Credit::Loan(credit) = &mut self.credit {
            credit.accepted_mob_hit(mob);
        }
    }
    /// `LivingEntity.resolvePlayerResponsibleForDamage` for a mob: a player's
    /// hit refreshes the 100-tick timer, a tame wolf's refreshes or clears it
    /// through the credit loan, and any other source keeps the remembered player,
    /// whose reference must still resolve.
    pub fn resolve_player_responsible(
        &mut self,
        mob: &mut crate::mobs::Mob,
        source: &crate::mobs::Damage,
    ) -> bool {
        let by_player = source.attacker == Some(crate::mobs::Who::Player);
        if by_player {
            mob.ai.last_hurt_by_player = 100;
        }
        match &mut self.credit {
            Credit::Loan(credit) => credit.resolve_player_responsible(mob),
            Credit::Memory | Credit::NoPush => {
                by_player
                    || self.players.as_ref().is_some_and(|(resolver, memory)| {
                        memory
                            .remembered(mob.id)
                            .is_some_and(|uuid| resolver.resolve(uuid).is_some())
                    })
            }
        }
    }
    /// Only the game bridge calls this after checking for a knockback subscriber. Splits out
    /// the credit loan so the returned main-hand borrow can outlive moving `self` into the
    /// hook host. Without a site's loan the credit comes from the pushed entity's own hurt
    /// memories and this hit's attack ([`kill_credit::from_memory`]).
    pub fn split_knockback_credit(
        mut self,
        pushed: &Hurting<'_>,
    ) -> (Self, i32, Option<&'a crate::items::ItemStack>) {
        let credit = std::mem::replace(&mut self.credit, Credit::Memory);
        let (id, main) = match credit {
            Credit::Loan(credit) => {
                let credit: &'a dyn KnockbackCredit = &*credit;
                credit.snapshot(pushed, self.attack)
            }
            Credit::Memory => kill_credit::from_memory(pushed, self.attack),
            Credit::NoPush => {
                debug_assert!(false, "living_knockback push from a no_push damage site");
                kill_credit::from_memory(pushed, self.attack)
            }
        };
        (self, id, main)
    }
    /// The same level, on a client when `client_side`: a hook asked on both sides tells its
    /// listener so.
    pub fn on_client(self, client_side: bool) -> Self {
        Self {
            client_side,
            ..self
        }
    }
    /// Whether this is a client's level ([`Self::on_client`]).
    pub fn client_side(&self) -> bool {
        self.client_side
    }
    /// The same level for the hit `attack` describes.
    pub fn attacked_by(self, attack: &'a Attack) -> Self {
        Self { attack, ..self }
    }
    /// Who attacks in this hit and what they hold, for the damage hooks.
    pub fn attack(&self) -> &'a Attack {
        self.attack
    }
    /// The authoritative world's merged tag tables, if available.
    pub fn tags(&self) -> Option<&'a crate::items::Items> {
        self.tags
    }
    /// The cached block parser context, if available.
    pub fn block_registries(&self) -> Option<&crate::command::args::Registries> {
        self.block_registries
    }
    /// `CombatRules.getDamageAfterAbsorb`: resolve the weapon of the actual direct entity.
    pub fn armor_effectiveness(
        &mut self,
        source: HurtSource<'_>,
        victim: &str,
        motion: (f64, bool),
        random: &mut sunrise_core::random::LegacyRandom,
        value: f32,
    ) -> f32 {
        let Some(stack) = self.attack.weapon_of(source.direct) else {
            return value;
        };
        self.enchanted_armor(stack, source, victim, motion, random, value)
    }

    fn enchanted_armor(
        &mut self,
        stack: &ItemStack,
        source: HurtSource<'_>,
        victim: &str,
        motion: (f64, bool),
        random: &mut sunrise_core::random::LegacyRandom,
        value: f32,
    ) -> f32 {
        let Some(items) = self.tags.filter(|_| !self.client_side) else {
            return value;
        };
        let mut ctx = self.attack.enchantment_context(source, victim);
        ctx.this_motion = Some(motion);
        items.enchantments.modify_entity_value(
            &stack.all_enchantments(),
            "minecraft:armor_effectiveness",
            &ctx,
            self.random,
            random,
            value,
        )
    }

    /// The build range of the level whose damage hook is running.
    pub fn shape(&self) -> crate::chunk::LevelShape {
        self.shape
    }
    /// The level's random (`level.getRandom()`).
    pub fn random(&mut self) -> &mut sunrise_core::random::LegacyRandom {
        self.random
    }
    /// The block state at `pos`: air where no chunk is loaded, as `World::block`.
    pub fn block(&self, pos: [i32; 3]) -> sunrise_world::registry::BlockStateId {
        self.chunks
            .get(&(pos[0] >> 4, pos[2] >> 4))
            .map_or(self.air, |chunk| {
                chunk.block(pos[0] & 15, pos[1], pos[2] & 15)
            })
    }
    /// The registry naming the level's block states.
    pub fn registry(&self) -> &'a sunrise_world::registry::BlockRegistry {
        self.registry
    }
    /// The level's own attachments (`Holder::Level`); other holders are not lent to a hit.
    pub fn attachments_read(
        &self,
        holder: sunrise_mod_runtime::attachments::Holder,
    ) -> std::result::Result<
        &sunrise_mod_runtime::attachments::Attachments,
        sunrise_mod_runtime::attachments::AttachmentError,
    > {
        if holder != sunrise_mod_runtime::attachments::Holder::Level {
            return Err(sunrise_mod_runtime::attachments::AttachmentError::MissingHolder);
        }
        Ok(self.attachments)
    }
    /// Mutable attachment storage for an accepted hit edit.
    pub fn attachments(
        &mut self,
        holder: sunrise_mod_runtime::attachments::Holder,
    ) -> std::result::Result<
        &mut sunrise_mod_runtime::attachments::Attachments,
        sunrise_mod_runtime::attachments::AttachmentError,
    > {
        self.attachments_read(holder)?;
        Ok(self.attachments)
    }
    /// Schedule a hit edit or explicit sync for the next send pass.
    pub fn attachment_sync_queued(&mut self, holder: sunrise_mod_runtime::attachments::Holder) {
        self.attachment_syncs.insert(holder);
    }
    /// Return the hook's immunity decision, or vanilla's without a listener.
    pub fn invulnerability_check(&mut self, hurting: Hurting<'_>, vanilla: bool) -> bool {
        match self.hooks.invulnerable {
            Some(hook) => hook(self.reborrow(), hurting, vanilla),
            None => vanilla,
        }
    }
    /// Whether `LivingIncomingDamageEvent` canceled the hit on `hurting`, standing as
    /// `victim` says ([`DamageHooks::incoming`]; never without it).
    pub fn incoming_canceled(
        &mut self,
        hurting: Hurting<'_>,
        victim: Victim,
        container: &mut DamageContainer,
    ) -> bool {
        let hook = self.hooks.incoming;
        hook.is_some_and(|hook| hook(self.for_victim(hurting), hurting, victim, container))
    }
    /// `LivingDamageEvent.Pre` of the hit on `hurting` ([`DamageHooks::pre`]; nothing without it).
    pub fn damage_pre(&mut self, hurting: Hurting<'_>, container: &mut DamageContainer) {
        if let Some(hook) = self.hooks.pre {
            hook(self.for_victim(hurting), hurting, container);
        }
    }
    /// `LivingDamageEvent.Post` of the hit on `hurting` ([`DamageHooks::post`]; nothing without it).
    pub fn damage_post(&mut self, hurting: Hurting<'_>, container: &DamageContainer) {
        if let Some(hook) = self.hooks.post {
            hook(self.for_victim(hurting), hurting, container);
        }
    }

    /// Close an accepted hit, or suspend it just long enough for the caller to give the
    /// owning game back its level and body for death or player protection.
    pub fn finish_damage(
        &mut self,
        hurting: Hurting<'_>,
        container: DamageContainer,
        needs_owner: bool,
    ) -> Option<DamageCompletion> {
        if needs_owner && let Some(listened) = self.hooks.owner_post_listened {
            let post = listened().then(|| {
                let source = match hurting {
                    Hurting::Mob(_, source) => mob_source(source, self.attack),
                    Hurting::Player(_, source)
                    | Hurting::Dragon(_, source)
                    | Hurting::ArmorStand(_, source)
                    | Hurting::Entity(_, _, source) => source,
                };
                std::sync::Arc::new(DamagePost {
                    damage_type: source.damage_type.to_owned(),
                    causing: source.causing,
                    direct: source.direct,
                    direct_type: source.direct_type.map(str::to_owned),
                    position: source.position,
                    lent_entity: self
                        .effects
                        .as_ref()
                        .and_then(|effects| effects.lent.as_ref())
                        .map(HitLend::id),
                    state: container.into_state(),
                })
            });
            return Some(DamageCompletion {
                post,
                protection: None,
            });
        }
        self.damage_post(hurting, &container);
        None
    }
}
