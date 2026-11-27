// Included in health.rs to retain its private scope.
#[derive(Clone)]
pub struct HealthResources {
    pub damage: Catalog,
    pub ids: PlayerFloatIds,
}
impl HealthResources {
    pub fn load(root: &Path) -> Result<Self> {
        Self::load_with_conditions(root, &sunrise_world::conditions::LoadConditions::vanilla())
    }
    /// Startup damage resources use the same engine-owned server inventory
    /// as the other tables, before any world pack stack exists.
    pub fn load_with_conditions(
        root: &Path,
        conditions: &sunrise_world::conditions::LoadConditions,
    ) -> Result<Self> {
        let read = |name: &str| -> Result<serde_json::Value> {
            Ok(serde_json::from_slice(&fs::read(root.join(name))?)?)
        };
        let ids = read("player-float-ids.json")?;
        let id = |name: &str| -> Result<u8> {
            Ok(u8::try_from(
                ids[name].as_u64().ok_or("missing player float ID")?,
            )?)
        };
        let damage = Catalog::load(
            root,
            &sunrise_world::loot::PackOverlay {
                conditions,
                ..sunrise_world::loot::PackOverlay::default()
            },
        )?;
        for name in ["minecraft:fall", "minecraft:stalagmite", "minecraft:starve"] {
            if damage.id(name).is_none() {
                return Err(format!("missing damage type {name}").into());
            }
        }
        Ok(Self {
            damage,
            ids: PlayerFloatIds {
                health: id("health")?,
                absorption: id("absorption")?,
            },
        })
    }
    /// A world's frozen damage registry at open, or reloaded tags against
    /// that same registry. Player metadata IDs stay the prepared version's.
    pub fn with_packs(
        &self,
        root: &Path,
        overlay: &sunrise_world::loot::PackOverlay<'_>,
        reload: bool,
    ) -> Result<Self> {
        Ok(Self {
            damage: if reload {
                self.damage.reload_tags(root, overlay)?
            } else {
                Catalog::load(root, overlay)?
            },
            ids: self.ids,
        })
    }
    pub fn landing_source(&self, source: LandingDamage) -> DamageTypeId {
        self.damage
            .id(match source {
                LandingDamage::Fall => "minecraft:fall",
                LandingDamage::Stalagmite => "minecraft:stalagmite",
            })
            .expect("validated landing source")
    }
}
/// `DamageEffects.sound()` by its serialized id: what `Player.getHurtSound`
/// plays for a damage type's `effects` (unknown ids fall back to `hurt`, the codec default).
pub fn hurt_sound(effects: &str) -> &'static str {
    match effects {
        "drowning" => "minecraft:entity.player.hurt_drown",
        "burning" => "minecraft:entity.player.hurt_on_fire",
        "poking" => "minecraft:entity.player.hurt_sweet_berry_bush",
        "freezing" => "minecraft:entity.player.hurt_freeze",
        // HURT and THORNS (both SoundEvents.PLAYER_HURT).
        _ => "minecraft:entity.player.hurt",
    }
}
#[derive(Clone, Copy, Debug)]
pub enum HealthEvent {
    /// `FoodData.tick` calling `hurtServer` with starvation. Only a hit that
    /// lands is recorded (`RecordedDamage`).
    StarvationDamage(f32),
    /// `LivingEntity.aiStep` hurting a fully frozen entity.
    FreezeDamage(f32),
    /// `SweetBerryBushBlock.entityInside` pricking a moving entity.
    SweetBerryBushDamage(f32),
    /// `LivingEntity.baseTick` drowning an entity out of air.
    DrownDamage(f32),
    Updated {
        id: u8,
        value: f32,
        dirty: bool,
    },
    Statistic {
        name: &'static str,
        amount: i32,
    },
    RecordedDamage {
        source: DamageTypeId,
        amount: f32,
    },
    /// `LivingEntity.hurtServer`'s `for (MobEffectInstance effect :
    /// getActiveEffects()) effect.onMobHurt(level, this, source, damage)`: an
    /// accepted hit's effects answer it, with the damage before armor and
    /// absorption, so a hit they take whole still reaches them. The game runs
    /// them (`InfestedMobEffect`).
    MobHurt,
    Phase(&'static str),
    /// `Player.hurtArmor`: a hit that doesn't bypass armor wears the worn
    /// pieces (`LivingEntity.getDamageAfterArmorAbsorb`); the game applies it.
    ArmorHurt {
        source: DamageTypeId,
        causing: i32,
        direct: i32,
        position: Option<[f64; 3]>,
        amount: f32,
    },
    /// `ServerLevel.broadcastDamageEvent`: the client's
    /// `LivingEntity.handleDamageEvent` plays the source's hurt sound.
    DamageBroadcast {
        source: DamageTypeId,
    },
    Criterion {
        /// The hit's damage type, for the responsible player of an accepted hit.
        source: DamageTypeId,
        original: f32,
        amount: f32,
        blocked: bool,
    },
}
impl HealthEvent {
    pub fn trace(self) -> String {
        match self {
            Self::StarvationDamage(amount) => format!("damage starve {:08x}", amount.to_bits()),
            Self::FreezeDamage(amount) => format!("damage freeze {:08x}", amount.to_bits()),
            Self::SweetBerryBushDamage(amount) => {
                format!("damage sweet_berry_bush {:08x}", amount.to_bits())
            }
            Self::DrownDamage(amount) => format!("damage drown {:08x}", amount.to_bits()),
            Self::Updated { id, value, dirty } => {
                format!("updated {id} {:08x} {dirty}", value.to_bits())
            }
            Self::Statistic { name, amount } => format!("health-stat {name} {amount}"),
            Self::RecordedDamage { amount, .. } => {
                format!("record-damage {:08x}", amount.to_bits())
            }
            Self::MobHurt => "mob-hurt".into(),
            Self::Phase(name) => name.into(),
            Self::ArmorHurt { amount, .. } => format!("hurt-armor {:08x}", amount.to_bits()),
            Self::DamageBroadcast { .. } => "broadcast-damage".into(),
            Self::Criterion { .. } => "hurt-criterion".into(),
        }
    }
}
pub(crate) struct Updates<'a>(pub(crate) &'a mut Vec<MotionEvent>);
impl Observer for Updates<'_> {
    type Error = io::Error;
    fn updated(&mut self, data: &mut SyncedFloats, id: u8) -> io::Result<()> {
        self.0.push(MotionEvent::Health(HealthEvent::Updated {
            id,
            value: data.get(id).expect("existing updated field"),
            dirty: data.is_dirty(),
        }));
        Ok(())
    }
}
#[derive(Clone)]
pub struct PlayerHealth {
    pub data: SyncedFloats,
    pub ids: PlayerFloatIds,
    pub food: FoodData,
    pub hurt: State,
    pub accepted_hit: Option<std::sync::Arc<AcceptedHit>>,
    /// `Entity.permanentlyInvulnerable` (#916): the NBT `Invulnerable` flag,
    /// what `Entity.setInvulnerable` writes — a field apart from
    /// `Abilities.invulnerable` (the game mode the client is told about),
    /// which vanilla keeps separately too. Not saved: Sunrise's player save
    /// keeps no such field yet.
    pub permanently_invulnerable: bool,
    /// `Entity.invulnerableTime` (#916): the entity-level invulnerability
    /// window, its own field — never the damage cooldown (`hurt.cooldown`).
    pub invulnerable_time: i32,
    /// Client hurt animation; fully blocked hits only refresh the server's hurt state.
    pub hurt_animation_time: i32,
    /// `Player.hurtDir` on a client: the yaw `ClientboundHurtAnimationPacket`
    /// gave, which `GameRenderer.bobHurt` tilts the camera around.
    pub hurt_dir: f32,
    /// `ServerPlayer.hurtDir` of a hit the player's client hasn't been told
    /// of yet (`indicateDamage`).
    pub indicated_dir: Option<f32>,
    pub dead: bool,
    /// A lethal or protected hit suspended for its acting body's synchronous game continuation.
    pub damage_completion: Option<DamageCompletion>,
    pub sync_velocity: bool,
    pub game_time: i64,
    pub tick_count: i32,
    /// The living target and this player’s tick count at `setLastHurtMob`. Not saved.
    pub last_hurt_mob: Option<(u32, i32)>,
}
impl PlayerHealth {
    pub fn new(resources: &HealthResources, attributes: &mut Attributes) -> Result<Self> {
        let ids = resources.ids;
        let mut data = SyncedFloats::new(&[
            FloatValue {
                id: ids.health,
                value: 1.0,
            },
            FloatValue {
                id: ids.absorption,
                value: 0.0,
            },
        ])?;
        let mut ignored = Vec::new();
        let mut observer = Updates(&mut ignored);
        let maximum = attributes
            .key("minecraft:max_health")
            .ok_or("missing max health")?;
        let maximum = attributes
            .value(
                Some(maximum),
                sunrise_world::attribute_map::ValueKind::Computed,
            )
            .map_err(|e| io::Error::other(format!("health attribute: {e:?}")))?
            as f32;
        PlayerAttributeHealth::new(attributes, &mut data, ids, false, &mut observer)?
            .set_health(maximum)?;
        Ok(Self {
            data,
            ids,
            food: FoodData::default(),
            hurt: HurtState {
                no_action_time: 0,
                cooldown: 0,
                last_hurt: 0.0,
                hurt_duration: 0,
                hurt_time: 0,
                last_source: None,
                last_stamp: 0,
            },
            accepted_hit: None,
            permanently_invulnerable: false,
            invulnerable_time: 0,
            hurt_animation_time: 0,
            hurt_dir: 0.0,
            indicated_dir: None,
            dead: false,
            damage_completion: None,
            sync_velocity: false,
            game_time: 0,
            tick_count: 0,
            last_hurt_mob: None,
        })
    }
    pub fn health(&self) -> f32 {
        self.data
            .get(self.ids.health)
            .expect("validated health field")
    }
    pub fn absorption(&self) -> f32 {
        self.data
            .get(self.ids.absorption)
            .expect("validated absorption field")
    }
    /// The game time the damage clock has counted.
    pub fn game_time(&self) -> i64 {
        self.game_time
    }
    pub fn has_pending_protection(&self) -> bool {
        self.damage_completion
            .as_ref()
            .is_some_and(|hit| hit.protection.is_some())
    }
    pub fn dead_or_dying(&self) -> bool {
        self.health() <= 0.0 || self.dead
    }
    /// Advance all three clock phases for callers without separate level and body ticks.
    pub fn advance_clock(&mut self) {
        self.advance_entity_clock();
        self.advance_damage_cooldown();
        self.advance_body_clock();
    }
    /// `Entity.commonTick`: the entity-level immunity window, not the damage cooldown.
    pub fn advance_entity_clock(&mut self) {
        self.game_time = self.game_time.wrapping_add(1);
        self.tick_count = self.tick_count.wrapping_add(1);
        if self.invulnerable_time > 0 {
            self.invulnerable_time -= 1;
        }
    }
    /// `damageCooldownTime` runs in `ServerPlayer.tick`, not in its `LivingEntity.baseTick`.
    pub fn advance_damage_cooldown(&mut self) {
        if self.hurt.cooldown > 0 {
            self.hurt.cooldown -= 1;
        }
    }
    /// `LivingEntity.baseTick`'s hurt timers.
    pub fn advance_body_clock(&mut self) {
        if self.hurt.hurt_time > 0 {
            self.hurt.hurt_time -= 1;
        }
        if self.hurt_animation_time > 0 {
            self.hurt_animation_time -= 1;
        }
    }
    /// `LivingEntity.animateHurt` / `handleDamageEvent` on a client: the hurt
    /// tilt and red flash restart.
    pub fn animate_hurt(&mut self) {
        self.hurt.hurt_duration = 10;
        self.hurt.hurt_time = 10;
        self.hurt_animation_time = 10;
    }
    /// `LivingEntity.animateHurt(yaw)` on a player: the tilt restarts around
    /// `yaw` (`Player.animateHurt` keeps it as `hurtDir`).
    pub fn animate_hurt_dir(&mut self, yaw: f32) {
        self.animate_hurt();
        self.hurt_dir = yaw;
    }
    /// `ServerPlayer.indicateDamage`: the hit's direction relative to where
    /// the player looks, kept for the packet to its client.
    pub fn indicate_damage(&mut self, xd: f64, zd: f64, yaw: f32) {
        use sunrise_core::geometry::trig::{RAD_TO_DEG, atan2};
        self.indicated_dir = Some((atan2(zd, xd) * f64::from(RAD_TO_DEG) - f64::from(yaw)) as f32);
    }
    /// `LocalPlayer.hurtTo` with `flashOnSetHealth`: a drop below the current
    /// health starts the hurt animation; `damageCooldownTime` (20 on a drop,
    /// 10 on a gain) is what lets `Hud.extractPlayerHealth` blink the hearts.
    pub fn flash_health_change(&mut self, health: f32) {
        let damage = self.health() - health;
        if damage <= 0.0 {
            if damage < 0.0 {
                self.hurt.cooldown = 10;
            }
        } else {
            self.hurt.last_hurt = damage;
            self.hurt.cooldown = 20;
            self.animate_hurt();
        }
    }
}
impl PlayerMotion {
    /// Where the player stands and looks (`getViewVector`), as a hit's [`Victim`].
    pub(crate) fn victim(&self) -> Victim {
        let view = sunrise_world::view_direction::view_vector(self.pitch, self.yaw);
        Victim {
            position: [self.position.x, self.position.y, self.position.z],
            view: [view.x, view.y, view.z],
        }
    }
    /// `hurtServer` of `amount` from `source` in `level`.
    pub fn hurt(
        &mut self,
        level: &mut DamageLevel<'_>,
        resources: &HealthResources,
        rules: HealthRules,
        source: DamageTypeId,
        amount: f32,
    ) -> Result<bool> {
        Ok(self
            .hurt_blocking(level, resources, rules, source, amount, None)?
            .hurt)
    }
    pub fn hurt_blocking(
        &mut self,
        level: &mut DamageLevel<'_>,
        resources: &HealthResources,
        rules: HealthRules,
        source: DamageTypeId,
        amount: f32,
        blocking: Option<Blocking>,
    ) -> Result<BlockOutcome> {
        self.hurt_caused_by(
            level, resources, rules, source, amount, blocking, false, 0, false,
        )
    }
    /// `hurt_blocking` from a source whose causing entity is a living
    /// non-player (`living_cause`), which the `when_caused_by_living_non_player`
    /// damage types scale with the difficulty. `causing` is that entity's id
    /// when it is known (0 otherwise), for the native damage hook.
    pub fn hurt_caused_by(
        &mut self,
        level: &mut DamageLevel<'_>,
        resources: &HealthResources,
        rules: HealthRules,
        source: DamageTypeId,
        amount: f32,
        blocking: Option<Blocking>,
        living_cause: bool,
        causing: i32,
        creative_source: bool,
    ) -> Result<BlockOutcome> {
        let hit = self.start_hurt(
            level,
            resources,
            rules,
            source,
            amount,
            blocking,
            living_cause,
            causing,
            creative_source,
            false,
            true,
            None,
        )?;
        self.finish_hurt(level, resources, rules, hit)
    }
    /// `hurtServer` up to `hurtHelmet`, for a player who wears a `helmet`; the arguments are
    /// `hurt_caused_by`'s. The caller has the player and the level back to wear the helmet
    /// with ([`PausedHurt::helmet_damage`]), and ends the hit with [`Self::finish_hurt`].
    /// `can_harm_player` is the server PvP decision, applied after the invulnerability
    /// observation. Non-PvP damage passes true. The base observation is retained across
    /// all continuations belonging to this hit.
    pub fn start_hurt(
        &mut self,
        level: &mut DamageLevel<'_>,
        resources: &HealthResources,
        rules: HealthRules,
        source: DamageTypeId,
        amount: f32,
        blocking: Option<Blocking>,
        living_cause: bool,
        causing: i32,
        creative_source: bool,
        helmet: bool,
        can_harm_player: bool,
        armor_weapon: Option<ArmorWeapon>,
    ) -> Result<PausedHurt> {
        let mut hit = PausedHurt {
            pending: None,
            invulnerability_checked: None,
            can_harm_player,
            selected: None,
            inner: None,
            source,
            source_name: String::new(),
            amount,
            before: self.health.health() + self.health.absorption(),
            fire_ticks: self.fire_ticks,
            helmet,
            living_cause,
            causing,
            armor_weapon,
            // The invulnerability refusal's inputs (#916), kept as the hit
            // was, for the Environment the pipeline's own rule reads.
            invulnerable: self.invulnerable,
            permanently_invulnerable: self.health.permanently_invulnerable,
            invulnerable_time: self.health.invulnerable_time > 0,
            creative_source,
            blocking,
            blocked: 0.0,
            shield_damage: 0,
            blocking_item_used: false,
            fully_blocked: false,
            block_sound: false,
            knocked: false,
            canceled: false,
            dealt: 0.0,
        };
        if self.client_side {
            return Ok(hit);
        }
        hit.source_name = resources
            .damage
            .definition(source)
            .ok_or("unknown resident damage source")?
            .name
            .clone();
        hit.pending = self.in_combat(level, resources, rules, &mut hit, |combat, state| {
            Ok(combat.hurt_until_helmet(state, amount)?)
        })?;
        if let Some(pending) = &mut hit.pending {
            pending.record_shield_damage(hit.shield_damage as f32);
        }
        Ok(hit)
    }
    /// Select the inner hit before synchronous ordinary equipment wear.
    pub fn select_equipment_hurt(
        &mut self,
        level: &mut DamageLevel<'_>,
        resources: &HealthResources,
        rules: HealthRules,
        hit: &mut PausedHurt,
    ) -> Result<Option<f32>> {
        let Some(pending) = hit.pending.take() else {
            return Ok(None);
        };
        hit.selected = self.in_combat(level, resources, rules, hit, |combat, state| {
            Ok(sunrise_world::living_hurt::select_equipment_hurt(
                state, combat, pending,
            )?)
        })?;
        if let Some(selected) = &hit.selected {
            let damage = selected.damage;
            hit.inner = self.in_combat(level, resources, rules, hit, |combat, _| {
                Ok(combat.prepare_equipment_inner()?)
            })?;
            if hit.inner == Some(false) {
                return Ok(Some(damage));
            }
        }
        Ok(None)
    }
    /// The rest of a hit from `hurtHelmet` on, against the player as the helmet's wear left
    /// them: its armor and protection, and whatever a listener changed.
    pub fn finish_hurt(
        &mut self,
        level: &mut DamageLevel<'_>,
        resources: &HealthResources,
        rules: HealthRules,
        mut hit: PausedHurt,
    ) -> Result<BlockOutcome> {
        if self.client_side {
            return Ok(BlockOutcome::default());
        }
        let hurt = if let Some(selected) = hit.selected.take() {
            let inner = hit.inner;
            self.in_combat(level, resources, rules, &mut hit, |combat, state| {
                Ok(combat.finish_selected_equipment(state, selected, inner)?)
            })?
        } else {
            match hit.pending.take() {
                Some(pending) => {
                    self.in_combat(level, resources, rules, &mut hit, |combat, state| {
                        Ok(combat.hurt_after_helmet(state, pending)?)
                    })?
                }
                None => false,
            }
        };
        let h = &mut self.health;
        let outcome = BlockOutcome {
            hurt,
            blocked: hit.blocked,
            shield_damage: hit.shield_damage,
            blocking_item_used: hit.blocking_item_used,
            fully_blocked: hit.fully_blocked,
            sound: hit.block_sound,
            knockback: hit.knocked,
            canceled: hit.canceled,
            dealt: hit.dealt,
        };
        if outcome.hurt {
            h.accepted_hit = if h.health() <= 0.0 {
                let source = level.attack().player_source(&hit.source_name, hit.causing);
                level.accepted_hit(Hurting::Player(self.entity_id, source))
            } else {
                None
            };
        }
        if outcome.hurt {
            h.hurt_animation_time = h.hurt.hurt_time;
        }
        // `dealDefaultKnockback` ends in `indicateDamage` unless blocked: a
        // hit with no direction of its own (`xd` and `zd` zero) is sent as
        // the opposite of the player's yaw. A hit that has a direction
        // replaces this (`mob_hits_player`).
        if outcome.knockback && outcome.blocked <= 0.0 {
            h.indicate_damage(0.0, 0.0, self.yaw);
        }
        let after = h.health() + h.absorption();
        // `LivingEntity.getDamageAfterArmorAbsorb` is the hit's only reader of
        // these two instances, and a source in `#minecraft:bypasses_armor`
        // makes it return before `getArmorValue` and
        // `getAttributeValue(ARMOR_TOUGHNESS)`. Native `getValue` is what
        // clears an instance's dirty flag, so the record takes the caches the
        // hit left instead of recomputing them: one that does meet the armour
        // has already recomputed both in `after_armor`, and one that bypassed
        // it leaves creation's zero cache untouched.
        let armor = self
            .attributes
            .cached_value_of("minecraft:armor")
            .unwrap_or(0.0) as f32;
        let toughness = self
            .attributes
            .cached_value_of("minecraft:armor_toughness")
            .unwrap_or(0.0) as f32;
        if self.hurt_log.len() >= HURT_LOG {
            self.hurt_log.remove(0);
        }
        self.hurt_log.push(HurtRecord {
            game_time: h.game_time,
            source: hit.source_name,
            amount: hit.amount,
            blocked: outcome.blocked,
            taken: hit.before - after,
            hurt: outcome.hurt,
            armor,
            toughness,
            health: after,
            fire_ticks: hit.fire_ticks,
            knockback: None,
        });
        Ok(outcome)
    }
    /// `run` on the player's combat, the hit's own state lent to it and kept for its next half.
    fn in_combat<R>(
        &mut self,
        level: &mut DamageLevel<'_>,
        resources: &HealthResources,
        rules: HealthRules,
        hit: &mut PausedHurt,
        run: impl FnOnce(
            &mut ServerPlayerCombat<'_, Environment<'_>, Attributes>,
            &mut State,
        ) -> Result<R>,
    ) -> Result<R> {
        let source = resources
            .damage
            .source::<()>(hit.source, None, None, None)
            .ok_or("unknown resident damage source")?;
        let victim = self.victim();
        let h = &mut self.health;
        let mut environment = Environment {
            level: level.reborrow(),
            invulnerability_checked: hit.invulnerability_checked,
            can_harm_player: hit.can_harm_player,
            entity: self.entity_id,
            victim,
            victim_motion: (self.fall_distance, self.fall_flying || self.flying),
            causing: hit.causing,
            invulnerable: hit.invulnerable,
            permanently_invulnerable: hit.permanently_invulnerable,
            invulnerable_time: hit.invulnerable_time,
            fire_immune: self.mod_fire_immune,
            creative_source: hit.creative_source,
            client_loaded: self.client_loaded,
            rules,
            source,
            weapon: hit.armor_weapon.as_ref(),
            random: &mut self.random,
            living_cause: hit.living_cause,
            events: &mut self.events,
            dead: &mut h.dead,
            damage_completion: &mut h.damage_completion,
            sync_velocity: &mut h.sync_velocity,
            game_time: h.game_time,
            status: self.status.clone(),
            helmet: hit.helmet,
            blocking: hit.blocking.take(),
            blocked: hit.blocked,
            shield_damage: hit.shield_damage,
            blocking_item_used: hit.blocking_item_used,
            fully_blocked: hit.fully_blocked,
            block_sound: hit.block_sound,
            knocked: hit.knocked,
            protection: None,
            canceled: hit.canceled,
            dealt: hit.dealt,
        };
        let result = run(
            &mut ServerPlayerCombat::new(
                &mut self.attributes,
                &mut h.data,
                &mut h.food,
                h.ids,
                &mut environment,
            )?,
            &mut h.hurt,
        )?;
        hit.invulnerability_checked = environment.invulnerability_checked;
        hit.blocking = environment.blocking;
        hit.blocked = environment.blocked;
        hit.shield_damage = environment.shield_damage;
        hit.blocking_item_used = environment.blocking_item_used;
        hit.fully_blocked = environment.fully_blocked;
        hit.block_sound = environment.block_sound;
        hit.knocked = environment.knocked;
        hit.canceled = environment.canceled;
        hit.dealt = environment.dealt;
        Ok(result)
    }
    pub fn set_health(&mut self, value: f32, absorption: bool) -> Result<()> {
        let h = &mut self.health;
        let mut observer = Updates(&mut self.events);
        let mut binding = PlayerAttributeHealth::new(
            &mut self.attributes,
            &mut h.data,
            h.ids,
            h.dead,
            &mut observer,
        )?;
        if absorption {
            binding.set_absorption(value)?;
        } else {
            binding.set_health(value)?;
        }
        Ok(())
    }
}
pub(crate) struct Environment<'a> {
    pub(crate) invulnerability_checked: Option<bool>,
    pub(crate) can_harm_player: bool,
    /// The level the hit lands in, for the damage hooks.
    pub(crate) level: DamageLevel<'a>,
    /// The player's entity id, and the causing entity's (0: none or not known).
    pub(crate) entity: i32,
    /// Where the player stands and looks, for the incoming-damage hook.
    pub(crate) victim: Victim,
    pub(crate) victim_motion: (f64, bool),
    pub(crate) causing: i32,
    /// `Abilities.invulnerable`, the game mode (`Player.hurtServer`'s own
    /// gate answers it, which no creative hit passes).
    pub(crate) invulnerable: bool,
    /// `Entity.permanentlyInvulnerable` (#916): the NBT flag.
    pub(crate) permanently_invulnerable: bool,
    /// `Entity.invulnerableTime > 0` (#916): the entity-level window.
    pub(crate) invulnerable_time: bool,
    pub(crate) fire_immune: bool,
    /// The hit is a creative player's (`DamageSource.isCreativePlayer`,
    /// #916): it passes the invulnerability below.
    pub(crate) creative_source: bool,
    /// The player's client has loaded the world since its last respawn or
    /// level change (`ServerGamePacketListenerImpl.hasClientLoaded`).
    pub(crate) client_loaded: bool,
    pub(crate) rules: HealthRules,
    pub(crate) source: DamageSource<'a, ()>,
    /// `DamageSource.getWeaponItem` with the registry of its
    /// `armor_effectiveness` enchantments, for `getDamageAfterArmorAbsorb`.
    pub(crate) weapon: Option<&'a ArmorWeapon>,
    /// The victim's own random for the damage-filtered enchantment effects.
    pub(crate) random: &'a mut sunrise_core::random::EntityRandom,
    /// The causing entity is a living non-player (a mob, or its projectile's owner).
    pub(crate) living_cause: bool,
    pub(crate) events: &'a mut Vec<MotionEvent>,
    pub(crate) dead: &'a mut bool,
    pub(crate) damage_completion: &'a mut Option<DamageCompletion>,
    pub(crate) sync_velocity: &'a mut bool,
    pub(crate) game_time: i64,
    /// Resistance, fire resistance and armor protection (effects.rs).
    pub(crate) status: crate::effects::StatusFlags,
    /// The head slot holds something, for a hit that stops at `hurtHelmet`.
    pub(crate) helmet: bool,
    /// The raised blocking item, if any, and what it blocked.
    pub(crate) blocking: Option<Blocking>,
    pub(crate) blocked: f32,
    /// The durability the blocking item loses.
    pub(crate) shield_damage: i32,
    pub(crate) blocking_item_used: bool,
    /// What it blocked covers the whole hit, as difficulty scaled it.
    pub(crate) fully_blocked: bool,
    pub(crate) block_sound: bool,
    /// `dealDefaultKnockback` ran: a fresh hit, not fully blocked.
    pub(crate) knocked: bool,
    pub(crate) protection: Option<PlayerProtection>,
    /// A native mod canceled the hit (`living_incoming_damage`).
    pub(crate) canceled: bool,
    /// The damage `ENTITY_HURT_PLAYER` is told of.
    pub(crate) dealt: f32,
}
/// A hit on a player stopped at `hurtHelmet` ([`PlayerMotion::start_hurt`]): the hit's own
/// state, which borrows nothing, so the game can run the helmet's wear on its real host.
/// Everything the player and level hold is read again when it resumes.
#[derive(Debug)]
pub struct PausedHurt {
    invulnerability_checked: Option<bool>,
    can_harm_player: bool,
    /// None for a hit refused before `hurtHelmet`.
    selected: Option<sunrise_world::living_hurt::SelectedHurt<()>>,
    inner: Option<bool>,
    pending: Option<PendingHurt<()>>,
    source: DamageTypeId,
    source_name: String,
    amount: f32,
    /// Health and absorption, and the fire ticks, before the hit.
    before: f32,
    fire_ticks: i32,
    helmet: bool,
    living_cause: bool,
    causing: i32,
    /// The attacker's weapon, for the armour fraction's `armor_effectiveness`.
    armor_weapon: Option<ArmorWeapon>,
    /// `Abilities.invulnerable` at the hit's start (#916).
    invulnerable: bool,
    /// `Entity.permanentlyInvulnerable` at the hit's start (#916).
    permanently_invulnerable: bool,
    /// `Entity.invulnerableTime > 0` at the hit's start (#916).
    invulnerable_time: bool,
    /// The hit is a creative player's (`isCreativePlayer`, #916).
    creative_source: bool,
    blocking: Option<Blocking>,
    blocked: f32,
    shield_damage: i32,
    blocking_item_used: bool,
    fully_blocked: bool,
    block_sound: bool,
    knocked: bool,
    canceled: bool,
    dealt: f32,
}
impl PausedHurt {
    /// What `hurtHelmet` is given, if this hit wears the helmet.
    pub fn helmet_damage(&self) -> Option<f32> {
        self.pending.as_ref()?.helmet_damage()
    }
}
/// `LivingEntity.applyItemBlocking` inputs for a raised `BlocksAttacks` item:
/// reductions whose damage-type sets are already matched against the source.
#[derive(Clone, Debug)]
pub struct Blocking {
    pub rules: sunrise_world::blocking_rules::BlockingRules<bool>,
    /// `bypassed_by` contains the source's damage type.
    pub bypassed: bool,
    /// The direct entity is an arrow with piercing.
    pub pierce: bool,
    /// Angle between the head's view and the source position (pi without one).
    pub angle: f64,
}
impl Blocking {
    /// `BlocksAttacks.resolveBlockedDamage`: what the item stops of `damage`,
    /// nothing for a source it bypasses or an arrow that pierces it.
    pub fn blocked(&self, damage: f32) -> f32 {
        if damage <= 0.0 || self.bypassed || self.pierce {
            return 0.0;
        }
        self.rules
            .resolve(&mut MatchedSets, damage, self.angle)
            .unwrap_or(0.0)
    }
}
/// How many hurts `PlayerMotion::hurt_log` keeps.
const HURT_LOG: usize = 64;
/// One hurt as it was taken: what came in, what a raised blocking item
/// stopped, the armour it met and what health it left.
#[derive(Clone, Debug, PartialEq)]
pub struct HurtRecord {
    pub game_time: i64,
    /// The damage type.
    pub source: String,
    /// The damage before blocking, armour and effects.
    pub amount: f32,
    pub blocked: f32,
    /// Health and absorption lost.
    pub taken: f32,
    pub hurt: bool,
    /// `minecraft:armor` and `minecraft:armor_toughness` as the hit left their
    /// instances' caches. `getDamageAfterArmorAbsorb` reads them only for a
    /// source outside `#minecraft:bypasses_armor`, so a hit that bypassed the
    /// armour reports the cache that source never recomputed.
    pub armor: f32,
    pub toughness: f32,
    pub health: f32,
    /// The fire ticks before the hurt.
    pub fire_ticks: i32,
    /// The push the hurt gave, if the caller applied one.
    pub knockback: Option<[f64; 3]>,
}
/// One melee attack on a mob: the damage after the attack strength, the
/// strength (`getAttackStrengthScale`), and the health it took.
#[derive(Clone, Debug, PartialEq)]
pub struct AttackRecord {
    pub game_time: i64,
    pub target: String,
    /// The mob's entity id, and whether the blow killed it.
    pub id: u32,
    pub killed: bool,
    pub amount: f32,
    pub scale: f32,
    pub crit: bool,
    pub landed: bool,
    pub dealt: f32,
}
/// The outcome of a hurt with a raised blocking item.
#[derive(Clone, Copy, Debug, Default)]
pub struct BlockOutcome {
    /// `hurtBlockingItem` ran, including when reductions resolve to zero.
    pub blocking_item_used: bool,
    pub hurt: bool,
    pub blocked: f32,
    /// The durability the blocking item loses (`ItemDamageFunction`, or what a
    /// native mod set).
    pub shield_damage: i32,
    /// `applyItemBlocking`'s `damageBlocked >= damage`, against the damage
    /// `Player.hurtServer` scaled with the difficulty.
    pub fully_blocked: bool,
    /// `BlocksAttacks.onBlocked` ran (the block sound plays).
    pub sound: bool,
    /// The hit knocks the player back (`dealDefaultKnockback`): not one in
    /// the hurt cooldown, nor one the shield took whole.
    pub knockback: bool,
    /// A native mod canceled the hit: its attacker must not be credited.
    pub canceled: bool,
    /// The damage `ENTITY_HURT_PLAYER` is told of: past the block and the helmet.
    pub dealt: f32,
}
