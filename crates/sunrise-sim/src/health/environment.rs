// Included in health.rs to retain its private scope.
struct MatchedSets;
impl sunrise_world::blocking_rules::TypeMatchWorld for MatchedSets {
    type Error = io::Error;
    type Set = bool;
    fn source_in_set(&mut self, set: bool) -> io::Result<bool> {
        Ok(set)
    }
}
impl Environment<'_> {
    fn event(&mut self, event: HealthEvent) {
        self.events.push(MotionEvent::Health(event));
    }
    fn phase(&mut self, name: &'static str) -> io::Result<()> {
        self.event(HealthEvent::Phase(name));
        Ok(())
    }
    fn stat(&mut self, name: &'static str, amount: i32) -> io::Result<()> {
        self.event(HealthEvent::Statistic { name, amount });
        Ok(())
    }
}
impl Observer for Environment<'_> {
    type Error = io::Error;
    #[inline]
    fn updated(&mut self, data: &mut SyncedFloats, id: u8) -> io::Result<()> {
        Updates(self.events).updated(data, id)
    }
}
impl<'a> ArmorWorld for Environment<'a> {
    type Error = io::Error;
    type Weapon = &'a ArmorWeapon;
    type ServerLevel = ();
    #[inline]
    fn weapon(&mut self) -> io::Result<Option<&'a ArmorWeapon>> {
        Ok(self.weapon)
    }
    #[inline]
    fn server_level(&mut self) -> io::Result<Option<()>> {
        Ok(Some(()))
    }
    /// `EnchantmentHelper.modifyArmorEffectiveness`: the weapon's
    /// `armor_effectiveness` effects over the fraction the armour, toughness
    /// and damage left.
    #[inline]
    fn modify_armor_effectiveness(
        &mut self,
        _: (),
        weapon: &'a ArmorWeapon,
        value: f32,
    ) -> io::Result<f32> {
        let attack = self.level.attack();
        let source = attack.player_source(&self.source.definition().name, self.causing);
        let mut ctx = attack.enchantment_context(source, "minecraft:player");
        ctx.this_motion = Some(self.victim_motion);
        Ok(weapon.enchantments.modify_entity_value(
            &weapon.stack.all_enchantments(),
            "minecraft:armor_effectiveness",
            &ctx,
            self.level.random(),
            self.random,
            value,
        ))
    }
}
impl ServerDamageEnvironment<Attributes> for Environment<'_> {
    type Attacker = ();
    #[inline]
    fn invulnerable(&mut self) -> io::Result<bool> {
        damage_immunity::server_player_immunity(self)
    }
    #[inline]
    fn bypasses(&mut self, kind: Bypass) -> io::Result<bool> {
        Ok(self.source.bypasses(kind))
    }
    #[inline]
    fn hurt_armor(&mut self, _: &mut Attributes, damage: f32) -> io::Result<()> {
        // The worn armor lives in the game, which wears it from this event.
        let hit = self
            .level
            .attack()
            .player_source(&self.source.definition().name, self.causing);
        self.event(HealthEvent::ArmorHurt {
            source: self.source.type_id(),
            causing: hit.causing,
            direct: hit.direct,
            position: hit.position,
            amount: damage,
        });
        Ok(())
    }
    #[inline]
    fn has_resistance(&mut self) -> io::Result<bool> {
        Ok(self.status.resistance.is_some())
    }
    #[inline]
    fn resistance_amplifier(&mut self) -> io::Result<i32> {
        self.status
            .resistance
            .ok_or_else(|| io::Error::other("no resistance effect"))
    }
    #[inline]
    fn award_resisted(&mut self, value: i32) -> io::Result<()> {
        self.stat("resisted", value)
    }
    #[inline]
    fn attacker(&mut self) -> io::Result<Option<()>> {
        Ok(None)
    }
    #[inline]
    fn award_dealt_resisted(&mut self, _: i32) -> io::Result<()> {
        Err(io::Error::other("no damage attacker"))
    }
    #[inline]
    fn damage_protection(&mut self, _: ()) -> io::Result<f32> {
        let Some(items) = self.level.tags() else {
            return Ok(self.status.protection(&self.source.definition().name));
        };
        let attack = self.level.attack();
        let source = attack.player_source(&self.source.definition().name, self.causing);
        let mut ctx = attack.enchantment_context(source, "minecraft:player");
        ctx.this_motion = Some(self.victim_motion);
        let mut protection = 0.0;
        for (stack, slot) in self
            .level
            .player_equipment
            .into_iter()
            .zip(sunrise_items::equipment::SLOT_NAMES)
        {
            let Some(stack) = stack else { continue };
            protection = items.enchantments.equipment_protection(
                stack,
                slot,
                &ctx,
                self.level.random(),
                self.random,
                protection,
            );
        }
        Ok(protection)
    }
    #[inline]
    fn award_self(&mut self, stat: DamageStat, value: i32) -> io::Result<()> {
        self.stat(
            match stat {
                DamageStat::Absorbed => "absorbed",
                DamageStat::Taken => "taken",
                DamageStat::DealtAbsorbed => return Err(io::Error::other("attacker statistic")),
            },
            value,
        )
    }
    #[inline]
    fn award_attacker(&mut self, _: (), _: DamageStat, _: i32) -> io::Result<()> {
        Err(io::Error::other("no damage attacker"))
    }
    #[inline]
    fn source_food_exhaustion(&mut self) -> io::Result<f32> {
        Ok(self.source.definition().exhaustion)
    }
    #[inline]
    fn ability_invulnerable(&self) -> bool {
        self.invulnerable
    }
    #[inline]
    fn level_is_client(&mut self) -> io::Result<bool> {
        Ok(false)
    }
    #[inline]
    fn record_damage(&mut self, amount: f32) -> io::Result<()> {
        self.event(HealthEvent::RecordedDamage {
            source: self.source.type_id(),
            amount,
        });
        Ok(())
    }
    #[inline]
    fn damage_event(&mut self) -> io::Result<()> {
        self.phase("entity-damage")
    }
    #[inline]
    fn damage_pre(&mut self, container: &mut DamageContainer) -> io::Result<()> {
        self.pre(container, None)
    }
    #[inline]
    fn damage_pre_with(
        &mut self,
        container: &mut DamageContainer,
        body: PlayerBodyRead<'_, Attributes>,
    ) -> io::Result<()> {
        self.pre(container, Some(body))
    }
}
impl CombatEnvironment<Attributes> for Environment<'_> {
    #[inline]
    fn dead_flag(&self) -> bool {
        *self.dead
    }
    #[inline]
    fn hurt_start_with(
        &mut self,
        _: &mut State,
        container: &mut DamageContainer,
        body: PlayerBodyRead<'_, Attributes>,
    ) -> io::Result<bool> {
        self.incoming(container, Some(body))
    }
    #[inline]
    fn damage_post_with(
        &mut self,
        _: &mut State,
        container: DamageContainer,
        body: PlayerBodyRead<'_, Attributes>,
    ) -> io::Result<()> {
        self.post(container, Some(body))
    }
}
/// The hit's level for a hook on player `entity`, with its body when the combat lends it: the
/// floats and attributes the combat holds, the effects and stacks the game keeps beside it.
fn player_hook_level<'b>(
    level: &'b mut DamageLevel<'_>,
    entity: i32,
    body: Option<PlayerBodyRead<'b, Attributes>>,
) -> DamageLevel<'b> {
    let reads = level.reads();
    let victim =
        body.map(|body| LivingBody::Player(PlayerBody::in_place(entity, body, reads.as_ref())));
    let level = level.reborrow();
    match victim {
        Some(victim) => level.with_victim(victim),
        None => level,
    }
}
impl Environment<'_> {
    fn incoming(
        &mut self,
        container: &mut DamageContainer,
        body: Option<PlayerBodyRead<'_, Attributes>>,
    ) -> io::Result<bool> {
        let source = self
            .level
            .attack()
            .player_source(&self.source.definition().name, self.causing);
        let (entity, victim) = (self.entity, self.victim);
        let canceled = player_hook_level(&mut self.level, self.entity, body).incoming_canceled(
            Hurting::Player(entity, source),
            victim,
            container,
        );
        self.canceled |= canceled;
        Ok(canceled)
    }
    fn pre(
        &mut self,
        container: &mut DamageContainer,
        body: Option<PlayerBodyRead<'_, Attributes>>,
    ) -> io::Result<()> {
        let source = self
            .level
            .attack()
            .player_source(&self.source.definition().name, self.causing);
        let entity = self.entity;
        player_hook_level(&mut self.level, self.entity, body)
            .damage_pre(Hurting::Player(entity, source), container);
        Ok(())
    }
    fn post(
        &mut self,
        container: DamageContainer,
        body: Option<PlayerBodyRead<'_, Attributes>>,
    ) -> io::Result<()> {
        let source = self
            .level
            .attack()
            .player_source(&self.source.definition().name, self.causing);
        let (entity, needs_owner) = (self.entity, *self.dead || self.protection.is_some());
        let completion = player_hook_level(&mut self.level, self.entity, body).finish_damage(
            Hurting::Player(entity, source),
            container,
            needs_owner,
        );
        *self.damage_completion = completion;
        if let Some(hit) = self.damage_completion.as_mut() {
            hit.protection = self.protection.take();
        }
        Ok(())
    }
}
impl EntityImmunityWorld for Environment<'_> {
    type Error = io::Error;
    #[inline]
    fn removed(&self) -> bool {
        false
    }
    #[inline]
    fn permanently_invulnerable(&mut self) -> io::Result<bool> {
        Ok(self.permanently_invulnerable)
    }
    #[inline]
    fn temporarily_invulnerable(&mut self) -> io::Result<bool> {
        Ok(self.invulnerable_time)
    }
    #[inline]
    fn tag(&mut self, tag: ImmunityTag) -> io::Result<bool> {
        Ok(self.source.immunity_tag(tag))
    }
    #[inline]
    fn creative_source(&mut self) -> io::Result<bool> {
        Ok(self.creative_source)
    }
    #[inline]
    fn fire_immune(&mut self) -> io::Result<bool> {
        Ok(self.fire_immune)
    }
    #[inline]
    fn fall_immune(&mut self) -> io::Result<bool> {
        Ok(self.level.tags().is_some_and(|items| {
            items.entity_type_tags.has_flag(
                "minecraft:player",
                crate::items::EntityTypeTags::FALL_DAMAGE_IMMUNE,
            )
        }))
    }
    #[inline]
    fn invulnerability_check(&mut self, vanilla: bool) -> io::Result<bool> {
        Ok(self.observe_hit_invulnerability(vanilla))
    }
}
impl ImmunityWorld for Environment<'_> {
    #[inline]
    fn enchantment_immunity(&mut self) -> io::Result<bool> {
        let Some(items) = self.level.tags() else {
            return Ok(false);
        };
        let equipment = self
            .level
            .player_equipment
            .into_iter()
            .map(|stack| stack.map_or(&[][..], ItemStack::enchantments))
            .zip(sunrise_items::equipment::SLOT_NAMES);
        let source = self
            .level
            .attack()
            .player_source(&self.source.definition().name, self.causing);
        let context = self
            .level
            .attack()
            .enchantment_context(source, "minecraft:player");
        Ok(items.enchantments.is_damage_immune(
            items.enchantments.damage_immunities(equipment),
            &context,
            self.level.random(),
        ))
    }
    #[inline]
    fn damage_rule(&mut self, rule: DamageRule) -> io::Result<bool> {
        Ok(match rule {
            DamageRule::Drowning => self.rules.drowning_damage,
            DamageRule::Fall => self.rules.fall_damage,
            DamageRule::Fire => self.rules.fire_damage,
            DamageRule::Freeze => self.rules.freeze_damage,
        })
    }
    #[inline]
    fn changing_dimension(&mut self) -> io::Result<bool> {
        Ok(false)
    }
    #[inline]
    fn ender_pearl(&mut self) -> io::Result<bool> {
        Ok(self.source.is_type("minecraft:ender_pearl"))
    }
    #[inline]
    fn client_loaded(&mut self) -> io::Result<bool> {
        Ok(self.client_loaded)
    }
}
impl HurtWorld for Environment<'_> {
    type Error = io::Error;
    type Source = DamageTypeId;
    type UseItem = ();
    type Blocking = ();
    type Sound = ();
    #[inline]
    fn source(&self) -> DamageTypeId {
        self.source.type_id()
    }
    #[inline]
    fn is_server_player(&self) -> bool {
        true
    }
    #[inline]
    fn invulnerable(&mut self, _: &mut State) -> io::Result<bool> {
        damage_immunity::server_player_immunity(self)
    }
    #[inline]
    fn dead_or_dying(&mut self, _: &mut State) -> io::Result<bool> {
        Err(io::Error::other("combat binding supplies health predicate"))
    }
    #[inline]
    fn tag(&mut self, _: &mut State, tag: HurtTag) -> io::Result<bool> {
        Ok(self.source.hurt_tag(tag))
    }
    #[inline]
    fn fire_resistance(&mut self, _: &mut State) -> io::Result<bool> {
        Ok(self.status.fire_resistance)
    }
    #[inline]
    fn hurt_start(&mut self, _: &mut State, container: &mut DamageContainer) -> io::Result<bool> {
        self.incoming(container, None)
    }
    #[inline]
    fn sleeping(&mut self, _: &mut State) -> io::Result<bool> {
        Ok(false)
    }
    #[inline]
    fn stop_sleeping(&mut self, _: &mut State) -> io::Result<()> {
        Ok(())
    }
    #[inline]
    fn use_item(&mut self, _: &mut State) -> io::Result<()> {
        Ok(())
    }
    #[inline]
    fn apply_blocking(&mut self, _: &mut State, damage: f32) -> io::Result<f32> {
        if damage <= 0.0 {
            return Ok(0.0);
        }
        // Nothing is stopped without a raised item, or when its `bypassed_by` or a
        // piercing arrow skips it; a native mod may still force a block.
        let vanilla = match &self.blocking {
            Some(b) if !b.bypassed && !b.pierce => {
                b.rules.resolve(&mut MatchedSets, damage, b.angle)?
            }
            _ => 0.0,
        };
        let shield_damage = self
            .blocking
            .as_ref()
            .map_or(0, |b| b.rules.item_damage.apply(vanilla));
        let block = if vanilla > 0.0 {
            ShieldBlock {
                blocked: true,
                blocked_damage: vanilla,
                shield_damage,
            }
        } else {
            ShieldBlock {
                shield_damage,
                ..no_block(damage)
            }
        };
        let source = self
            .level
            .attack()
            .player_source(&self.source.definition().name, self.causing);
        // `applyItemBlocking` calls `hurtBlockingItem` with what the item stopped,
        // nothing included (a hit from behind): the item is used all the same.
        // Only a block a mod's hook turned off leaves it unused.
        let reached =
            vanilla <= 0.0 && matches!(&self.blocking, Some(b) if !b.bypassed && !b.pierce);
        let Some(block) =
            self.level
                .shield_block(Hurting::Player(self.entity, source), damage, block)
        else {
            if reached {
                self.blocking_item_used = true;
                self.shield_damage = shield_damage;
            }
            return Ok(0.0);
        };
        self.blocking_item_used = true;
        self.blocked = block.blocked_damage;
        // `setShieldDamage` is what the item loses if `blocked` holds, whatever
        // `blocked_damage` says; below 0, the item's own `itemDamage` of what is stopped.
        self.shield_damage = if !block.blocked {
            0
        } else if block.shield_damage < 0 {
            self.blocking
                .as_ref()
                .map_or(0, |b| b.rules.item_damage.apply(block.blocked_damage))
        } else {
            block.shield_damage
        };
        self.fully_blocked = self.blocked >= damage;
        Ok(self.blocked)
    }
    #[inline]
    fn freeze_hurts_extra(&mut self, _: &mut State) -> io::Result<bool> {
        Ok(false)
    }
    #[inline]
    fn helmet_empty(&mut self, _: &mut State) -> io::Result<bool> {
        Ok(!self.helmet)
    }
    /// The game wears it between the hit's halves ([`PlayerMotion::start_hurt`]).
    #[inline]
    fn hurt_helmet(&mut self, _: &mut State, _: f32) -> io::Result<()> {
        Ok(())
    }
    #[inline]
    fn actually_hurt(&mut self, _: &mut State, _: &mut DamageContainer, _: f32) -> io::Result<()> {
        Err(io::Error::other("combat binding supplies inner damage"))
    }
    #[inline]
    fn resolve_mob(&mut self, _: &mut State) -> io::Result<()> {
        Ok(())
    }
    #[inline]
    fn resolve_player(&mut self, _: &mut State) -> io::Result<()> {
        Ok(())
    }
    #[inline]
    fn blocking_component(&mut self, _: ()) -> Option<()> {
        self.blocking.as_ref().map(|_| ())
    }
    #[inline]
    fn on_blocked(&mut self, _: &mut State, _: ()) -> io::Result<()> {
        self.block_sound = true;
        Ok(())
    }
    #[inline]
    fn broadcast_damage(&mut self, _: &mut State) -> io::Result<()> {
        self.event(HealthEvent::DamageBroadcast {
            source: self.source.type_id(),
        });
        Ok(())
    }
    #[inline]
    fn mark_hurt(&mut self, _: &mut State) -> io::Result<()> {
        *self.sync_velocity = true;
        Ok(())
    }
    /// The caller applies it: these sources carry no entity or position.
    #[inline]
    fn knockback(&mut self, _: &mut State, _: f32, _: bool) -> io::Result<()> {
        self.knocked = true;
        Ok(())
    }
    #[inline]
    fn death_protection(&mut self, _: &mut State) -> io::Result<bool> {
        if self
            .source
            .immunity_tag(ImmunityTag::BypassesInvulnerability)
        {
            return Ok(false);
        }
        let source = self
            .level
            .attack()
            .player_source(&self.source.definition().name, self.causing);
        for (hand, slot) in PROTECTION_HANDS {
            let Some(stack) = self.level.player_equipment[slot] else {
                continue;
            };
            if self.level.death_protection(stack).is_some()
                && !self
                    .level
                    .totem_canceled(Hurting::Player(self.entity, source), hand, stack)
            {
                self.protection = Some(PlayerProtection {
                    hand: Some(hand),
                    stack: stack.clone(),
                });
                return Ok(true);
            }
        }
        let protector = self
            .level
            .mod_protector(Hurting::Player(self.entity, source));
        self.protection = protector.map(|stack| PlayerProtection { hand: None, stack });
        Ok(self.protection.is_some())
    }
    #[inline]
    fn death_sound(&mut self, _: &mut State) -> io::Result<()> {
        Ok(())
    }
    #[inline]
    fn make_sound(&mut self, _: &mut State, _: ()) -> io::Result<()> {
        self.phase("death-sound")
    }
    #[inline]
    fn secondary_sound(&mut self, _: &mut State) -> io::Result<()> {
        if self.source.definition().effects == "thorns" {
            self.phase("thorns-sound")?;
        }
        Ok(())
    }
    #[inline]
    fn die(&mut self, _: &mut State) -> io::Result<()> {
        *self.dead = true;
        self.phase("death-request")
    }
    /// `Player.playSound` leaves the player itself out: others hear this one.
    #[inline]
    fn hurt_sound(&mut self, _: &mut State) -> io::Result<()> {
        if !self.block_sound {
            let sound = hurt_sound(&self.source.definition().effects);
            self.events.push(MotionEvent::HurtSound(sound));
        }
        self.phase("hurt-sound")
    }
    #[inline]
    fn game_time(&mut self, _: &mut State) -> io::Result<i64> {
        Ok(self.game_time)
    }
    #[inline]
    fn effects_on_hurt(&mut self, _: &mut State, _: f32) -> io::Result<()> {
        // `for (MobEffectInstance effect : getActiveEffects()) effect.onMobHurt(..)`:
        // Infested is the only effect that answers a hit, and its roll draws
        // from the level's random, so the game runs it.
        if self.status.infested {
            self.event(HealthEvent::MobHurt);
        }
        Ok(())
    }
    #[inline]
    fn player_hurt_criterion(
        &mut self,
        _: &mut State,
        original: f32,
        amount: f32,
        blocked: bool,
    ) -> io::Result<()> {
        self.dealt = amount;
        self.event(HealthEvent::Criterion {
            source: self.source.type_id(),
            original,
            amount,
            blocked,
        });
        Ok(())
    }
    #[inline]
    fn award_blocked(&mut self, _: &mut State, amount: i32) -> io::Result<()> {
        self.stat("blocked", amount)
    }
    #[inline]
    fn attacker_criterion(&mut self, _: &mut State, _: f32, _: f32, _: bool) -> io::Result<()> {
        Ok(())
    }
    #[inline]
    fn damage_post(&mut self, _: &mut State, container: DamageContainer) -> io::Result<()> {
        self.post(container, None)
    }
}
impl PlayerHurtWorld for Environment<'_> {
    #[inline]
    fn ability_invulnerable(&self) -> bool {
        self.invulnerable
    }
    #[inline]
    fn bypasses_invulnerability(&mut self, _: &mut State) -> io::Result<bool> {
        Ok(self.source.in_tag("minecraft:bypasses_invulnerability"))
    }
    #[inline]
    fn remove_shoulders(&mut self, _: &mut State) -> io::Result<()> {
        Ok(())
    }
    #[inline]
    fn scales_with_difficulty(&mut self, _: &mut State) -> io::Result<bool> {
        // DamageSource.scalesWithDifficulty.
        Ok(match self.source.definition().scaling {
            Scaling::Never => false,
            Scaling::Always => true,
            Scaling::LivingNonPlayer => self.living_cause,
        })
    }
    #[inline]
    fn difficulty(&mut self, _: &mut State) -> io::Result<Difficulty> {
        Ok(self.rules.difficulty)
    }
}
impl ServerPlayerHurtWorld for Environment<'_> {
    type Player = ();
    type Arrow = ();
    #[inline]
    fn causing_entity(&mut self, _: &mut State) -> io::Result<CausingEntity<(), ()>> {
        Ok(if self.can_harm_player {
            CausingEntity::Other
        } else {
            CausingEntity::Player(())
        })
    }
    #[inline]
    fn arrow_player_owner(&mut self, _: &mut State, _: ()) -> io::Result<Option<()>> {
        Ok(None)
    }
    #[inline]
    fn can_harm_player(&mut self, _: &mut State, _: ()) -> io::Result<bool> {
        Ok(self.can_harm_player)
    }
}
