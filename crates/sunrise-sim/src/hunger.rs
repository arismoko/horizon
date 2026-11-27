//! Hunger and regeneration over the resident player's actual health, damage,
//! attribute and exhaustion storage. Starvation borrows the same FoodData that
//! is mid-tick, preserving damage callbacks and the following timer reset.
use crate::{
    Result,
    health::{
        Attributes, DamageLevel, Environment, HealthEvent, HealthResources, HealthRules, HitReads,
        LivingLend, State, Updates,
    },
    motion::{MotionEvent, PlayerMotion},
};
use sunrise_world::native_load_context::LoopContext;
use sunrise_world::{
    food::{Difficulty, FoodData, FoodWorld},
    player_health::{PlayerAttributeHealth, PlayerFloatIds},
    server_player_combat::ServerPlayerCombat,
    synced_floats::SyncedFloats,
};

/// A living entity is about to heal (`LivingHealEvent` at `LivingEntity.heal`). Mods may change
/// the amount or cancel. `healed` lends the entity's body for their reads.
#[inline(never)]
pub fn native_living_heal(
    context: &LoopContext,
    healed: LivingLend<'_>,
    amount: &mut f32,
    cancel: &mut bool,
) {
    crate::native_client_hooks::living_heal(context, healed, amount, cancel);
}

impl PlayerMotion {
    /// Player.causeFoodExhaustion: creative and spectator abilities suppress
    /// exhaustion from movement, attacks and effects.
    pub fn cause_food_exhaustion(&mut self, amount: f32) {
        if !self.invulnerable && !self.spectator {
            self.health.food.add_exhaustion(amount);
        }
    }

    /// ServerPlayer.tickRegeneration, dispatched from Player.aiStep before
    /// inventory and living movement. The separately invoked food tick follows
    /// movement in ServerPlayer.doTick.
    /// `reads` is what the heal hook's view of the player needs beyond its body (see
    /// [`LivingLend`]).
    pub fn peaceful_regeneration<'r>(
        &mut self,
        context: &LoopContext,
        reads: &dyn Fn() -> Option<HitReads<'r>>,
        rules: HealthRules,
    ) -> Result<()> {
        if rules.difficulty != Difficulty::Peaceful || !rules.natural_regeneration {
            return Ok(());
        }
        if self.health.tick_count % 20 == 0 {
            let health = self.health.health();
            let maximum = self.attribute("minecraft:max_health")? as f32;
            if health < maximum {
                let mut amount = 1.0;
                let mut cancel = false;
                let lent = || reads();
                let healed = LivingLend::player(
                    self.entity_id,
                    &self.attributes,
                    &self.health.data,
                    self.health.ids,
                    &lent,
                );
                native_living_heal(context, healed, &mut amount, &mut cancel);
                if !cancel {
                    let h = &mut self.health;
                    let mut observer = Updates(&mut self.events);
                    PlayerAttributeHealth::new(
                        &mut self.attributes,
                        &mut h.data,
                        h.ids,
                        h.dead,
                        &mut observer,
                    )?
                    .heal(amount)?;
                }
            }
            let saturation = self.health.food.saturation;
            if saturation < 20.0 {
                self.health.food.saturation = saturation + 1.0;
            }
        }
        if self.health.tick_count % 10 == 0 && self.health.food.needs_food() {
            self.health.food.food = self.health.food.food.wrapping_add(1);
        }
        Ok(())
    }
    /// `FoodData.tick` in `level`, where starvation hurts.
    pub fn food_tick(
        &mut self,
        context: &LoopContext,
        level: &mut DamageLevel<'_>,
        resources: &HealthResources,
        rules: HealthRules,
    ) -> Result<()> {
        if self.client_side {
            return Ok(());
        }
        let victim = self.victim();
        let h = &mut self.health;
        let mut world = TickWorld {
            context,
            level: level.reborrow(),
            entity: self.entity_id,
            victim,
            victim_motion: (self.fall_distance, self.fall_flying || self.flying),
            resources,
            rules,
            invulnerable: self.invulnerable || self.spectator,
            fire_immune: self.mod_fire_immune,
            client_loaded: self.client_loaded,
            attributes: &mut self.attributes,
            data: &mut h.data,
            ids: h.ids,
            hurt: &mut h.hurt,
            hurt_animation_time: &mut h.hurt_animation_time,
            dead: &mut h.dead,
            damage_completion: &mut h.damage_completion,
            sync_velocity: &mut h.sync_velocity,
            game_time: h.game_time,
            events: &mut self.events,
            status: self.status.clone(),
            random: &mut self.random,
        };
        h.food.tick(&mut world)
    }
}
struct TickWorld<'a> {
    context: &'a LoopContext,
    level: DamageLevel<'a>,
    entity: i32,
    victim: crate::health::Victim,
    victim_motion: (f64, bool),
    resources: &'a HealthResources,
    rules: HealthRules,
    invulnerable: bool,
    fire_immune: bool,
    client_loaded: bool,
    attributes: &'a mut Attributes,
    data: &'a mut SyncedFloats,
    ids: PlayerFloatIds,
    hurt: &'a mut State,
    hurt_animation_time: &'a mut i32,
    dead: &'a mut bool,
    damage_completion: &'a mut Option<crate::health::DamageCompletion>,
    sync_velocity: &'a mut bool,
    game_time: i64,
    events: &'a mut Vec<MotionEvent>,
    /// Resistance, fire resistance and armor protection (effects.rs): the
    /// starvation hurt is a `LivingEntity.hurtServer` like any other, so its
    /// `onMobHurt` (Infested) and magic absorb see them.
    status: crate::effects::StatusFlags,
    random: &'a mut sunrise_core::random::EntityRandom,
}
impl FoodWorld for TickWorld<'_> {
    type Error = Box<dyn std::error::Error>;
    type Level = ();
    fn level(&mut self, _: &mut FoodData) -> Result<()> {
        Ok(())
    }
    fn difficulty(&mut self, _: (), _: &mut FoodData) -> Result<Difficulty> {
        Ok(self.rules.difficulty)
    }
    fn natural_regeneration(&mut self, _: (), _: &mut FoodData) -> Result<bool> {
        Ok(self.rules.natural_regeneration)
    }
    fn is_hurt(&mut self, _: &mut FoodData) -> Result<bool> {
        let mut observer = Updates(self.events);
        Ok(PlayerAttributeHealth::new(
            self.attributes,
            self.data,
            self.ids,
            *self.dead,
            &mut observer,
        )?
        .is_hurt()?)
    }
    fn heal(&mut self, _: &mut FoodData, mut amount: f32) -> Result<()> {
        let mut cancel = false;
        let level = &self.level;
        let reads = || level.reads();
        let healed = LivingLend::player(self.entity, self.attributes, self.data, self.ids, &reads);
        native_living_heal(self.context, healed, &mut amount, &mut cancel);
        if cancel {
            return Ok(());
        }
        let mut observer = Updates(self.events);
        Ok(PlayerAttributeHealth::new(
            self.attributes,
            self.data,
            self.ids,
            *self.dead,
            &mut observer,
        )?
        .heal(amount)?)
    }
    fn health(&mut self, _: &mut FoodData) -> Result<f32> {
        Ok(self
            .data
            .get(self.ids.health)
            .expect("validated health field"))
    }
    fn hurt_starvation(&mut self, _: (), food: &mut FoodData, amount: f32) -> Result<()> {
        let id = self
            .resources
            .damage
            .id("minecraft:starve")
            .expect("validated starvation source");
        let mut environment = Environment {
            level: self.level.reborrow(),
            invulnerability_checked: None,
            can_harm_player: true,
            entity: self.entity,
            victim: self.victim,
            victim_motion: self.victim_motion,
            causing: 0,
            invulnerable: self.invulnerable,
            permanently_invulnerable: false,
            invulnerable_time: false,
            fire_immune: self.fire_immune,
            creative_source: false,
            client_loaded: self.client_loaded,
            rules: self.rules,
            source: self
                .resources
                .damage
                .source::<()>(id, None, None, None)
                .expect("validated starvation source"),
            weapon: None,
            random: self.random,
            living_cause: false,
            events: self.events,
            dead: self.dead,
            damage_completion: self.damage_completion,
            sync_velocity: self.sync_velocity,
            game_time: self.game_time,
            status: self.status.clone(),
            helmet: false,
            blocking: None,
            blocked: 0.0,
            shield_damage: 0,
            blocking_item_used: false,
            fully_blocked: false,
            block_sound: false,
            knocked: false,
            protection: None,
            canceled: false,
            dealt: 0.0,
        };
        if ServerPlayerCombat::new(self.attributes, self.data, food, self.ids, &mut environment)?
            .hurt(self.hurt, amount)?
        {
            environment
                .events
                .push(MotionEvent::Health(HealthEvent::StarvationDamage(amount)));
            *self.hurt_animation_time = self.hurt.hurt_time;
        }
        Ok(())
    }
}
