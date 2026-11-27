use super::{commands::Target, dragon::FightPlayer, living_attributes::AttributeWrite};
use crate::game::ServerGame;
use mod_api::{ActiveEffect, EffectApplicable, EffectTickChange, EntityRef, LivingRef, PlayerFood};
use sunrise_core::nbt::Tag;
use sunrise_core::{
    geometry::{Aabb, Direction, Vec3},
    random::{LegacyRandom, RandomSource},
};
use sunrise_gameplay::particle_options::custom_particle_options;
use sunrise_gameplay_common::game::dragon_common::Breath;
use sunrise_gameplay_common::game::potions_common::{
    BodyPotions, CloudParticle, CloudSettings, PotionCloud, PotionState, Victim,
};
use sunrise_sim::health::{Attack, DirectEntity};
use {
    sunrise_game_data::text::Text, sunrise_items::items::ItemStack, sunrise_level::brewing,
    sunrise_level::effects::EffectAction, sunrise_level::effects::EffectInstance,
    sunrise_level::effects::EffectTickHost, sunrise_level::effects::OmenTick,
    sunrise_level::effects::PlayerEffects, sunrise_level::effects::TickingEffect,
    sunrise_level::effects::effect_modifiers, sunrise_level::particle_options::ParticleType,
    sunrise_sim::mobs::Damage, sunrise_sim::mobs::DamageKind, sunrise_sim::mobs::Mob,
    sunrise_sim::mobs::MobEffectAction, sunrise_sim::mobs::ModEffectTick,
    sunrise_sim::mobs::NamedDamage, sunrise_sim::mobs::Who, sunrise_sim::projectiles::Projectile,
    sunrise_sim::projectiles::ProjectileKind,
};

impl crate::game::ServerGame<'_> {
    /// `InfestedMobEffect.onMobHurt` for the player: from half its height up,
    /// along where it looks.
    pub(in crate::game) fn player_infested(&mut self) {
        if let Some(p) = self.common.player.as_ref() {
            let at = [
                p.position.x,
                p.position.y + f64::from(p.dimensions.height) / 2.0,
                p.position.z,
            ];
            let look = super::view_vector(self.common.yaw, self.common.pitch);
            // The player's draws come from the world's random.
            let velocities =
                sunrise_sim::mobs::infested_silverfish(&mut self.common.world.random, look);
            self.spawn_infested(at, velocities);
        }
    }
    /// `InfestedMobEffect.spawnSilverfish`: a silverfish at `at` (half the
    /// bearer's height up) for each of the velocities the bearer's random drew
    /// (`sunrise_client::mobs::infested_silverfish`).
    pub(in crate::game) fn spawn_infested(&mut self, at: [f64; 3], velocities: Vec<Vec3>) {
        for velocity in velocities {
            if let Some(id) = self.spawn_mob(
                "minecraft:silverfish",
                at,
                false,
                sunrise_gameplay_common::game::creatures::SpawnSite::Plain,
            ) && let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id)
            {
                m.velocity = velocity;
            }
            self.server_play_sound("minecraft:entity.silverfish.hurt", at, 1.0, 1.0);
        }
    }
    /// The local player's effects (`tick_mob_effects`), then the potions
    /// in flight, which a server without a local player still has. A frozen
    /// tick (`/tick freeze`) leaves the potions where they are
    /// (`TickRateManager.isEntityFrozen`); players' effects still run.
    pub(in crate::game) fn tick_effects(&mut self, running: bool) {
        self.tick_mob_effects();
        if running {
            self.tick_thrown_potions();
            self.tick_potion_clouds();
        }
    }
    /// `LivingEntity.tickEffects` (server side) for `self.player`, the local
    /// player or a guest acting (`as_guest`), then its attribute modifiers
    /// and status flags.
    pub fn tick_mob_effects(&mut self) {
        self.clear_dead_player_target();
        let Some(player) = self.common.player.as_ref() else {
            return;
        };
        if player.health.dead_or_dying() {
            // LivingEntity.die → removeAllEffects. Vanilla runs the effects
            // that act on death (`onMobRemoved`) when the dead player is
            // removed, up to 20 ticks later; here it is as the effects go.
            let w = f64::from(player.dimensions.width) / 2.0;
            let bounds = Aabb::new(
                Vec3::new(
                    player.position.x - w,
                    player.position.y,
                    player.position.z - w,
                ),
                Vec3::new(
                    player.position.x + w,
                    player.position.y + f64::from(player.dimensions.height),
                    player.position.z + w,
                ),
            );
            let ids: Vec<&'static str> = self
                .common
                .potions
                .effects
                .list
                .iter()
                .map(|e| e.id)
                .collect();
            let indices: Vec<_> = self
                .common
                .potions
                .effects
                .list
                .iter()
                .map(|e| e.registry_index)
                .collect();
            self.common.potions.effects.clear();
            if !ids.is_empty() {
                self.remove_player_effect_attributes(&indices);
                self.effects_on_death(
                    &ids,
                    bounds,
                    sunrise_sim::save::number(
                        self.common
                            .level
                            .player_extra
                            .as_ref()
                            .and_then(|tag| tag.get("Silent")),
                    )
                    .unwrap_or(0)
                        != 0,
                    None,
                    None,
                );
            }
        }
        // Keep `self.common.potions.effects` live on `ServerGame` during the pass so
        // tick-driven `addEffect` (`BadOmenMobEffect` -> `RaidOmen`) runs
        // `add_player_effect` with the live active-effect map, borrowing effect
        // metadata without cloning `EffectInstance`s and checking
        // `MobEffectEvent.Expired` only when an effect expires and has a listener.
        PlayerEffects::tick_on(self);
        self.turtle_helmet_tick();
        self.apply_effect_status();
    }
    /// `Player.tick` -> `turtleHelmetTick`: with a turtle helmet on the head
    /// and the eyes out of the water, Water Breathing for 200 ticks (no
    /// particles, an icon).
    pub(in crate::game::server_side) fn turtle_helmet_tick(&mut self) {
        let Some(p) = self.common.player.as_ref() else {
            return;
        };
        let eye_y = p.position.y + f64::from(p.dimensions.eye_height);
        let (bx, by, bz) = (
            p.position.x.floor() as i32,
            eye_y.floor() as i32,
            p.position.z.floor() as i32,
        );
        let f = self
            .common
            .res
            .facts()
            .get(self.common.world.block(bx, by, bz));
        let eye_in_water = f.fluid == sunrise_items::blocks::Fluid::Water && {
            let above = self
                .common
                .res
                .facts()
                .get(self.common.world.block(bx, by + 1, bz));
            let height = if above.fluid == sunrise_items::blocks::Fluid::Water {
                1.0
            } else {
                f64::from(f.fluid_level) / 9.0
            };
            eye_y <= f64::from(by) + height
        };
        if !p.health.dead_or_dying()
            && self.common.armor[0]
                .as_ref()
                .is_some_and(|s| &*s.item == "minecraft:turtle_helmet")
            && !eye_in_water
        {
            let mut effect = EffectInstance::new("water_breathing", 200, 0);
            effect.visible = false;
            self.add_player_effect(effect);
        }
    }
    /// Add an active effect to the player and update attributes / absorption.
    /// `LivingEntity.canBeAffected` with the `mob_effect_applicable` hook:
    /// the event fires first, for every effect, and `Allow` or `Deny`
    /// decides; unanswered, the vanilla tag check runs. `entity` is a mob's
    /// `Entity.getId`, None for the player.
    pub(in crate::game) fn can_be_affected(
        &mut self,
        entity: Option<i32>,
        kind: &str,
        effect: &str,
    ) -> bool {
        let key = brewing::effect(effect)
            .and_then(|def| mod_api::__private::effect_key_by_index(def.index));
        self.can_be_affected_resolved(entity, kind, effect, key)
    }

    /// The validated registry path, also used by native operations without parsing a name again.
    pub(in crate::game) fn can_be_affected_resolved(
        &mut self,
        entity: Option<i32>,
        kind: &str,
        effect: &str,
        key: Option<mod_api::EffectKey>,
    ) -> bool {
        let mut answered = None;
        if let Some(living) = self.effect_living_ref(entity)
            && let Some(key) = key
        {
            self.native_mob_effect_applicable(living, key, &mut answered);
        }
        match answered {
            Some(EffectApplicable::Allow) => true,
            Some(EffectApplicable::Deny) => false,
            // An unanswered hook, or a future variant, defers to the
            // vanilla tag check.
            _ => {
                let tags = &self.items().entity_type_tags;
                mob_can_be_affected(tags, kind, tags.slot(kind), effect)
            }
        }
    }
    pub fn add_player_effect(&mut self, effect: EffectInstance) -> bool {
        self.add_player_effect_from(effect, None)
    }
    /// `LivingEntity.addEffect(MobEffectInstance, @Nullable Entity)` for the
    /// acting player (or a guest swapped in via `as_guest`): checks
    /// `canBeAffected` / `mob_effect_applicable`, fires NeoForge's
    /// `MobEffectEvent.Added` before mutating active effects, updates the
    /// active map and status modifiers, runs `MobEffect.onEffectAdded`
    /// (vanilla sound + `MobEffectPart::on_effect_added`) only on first
    /// insertion, and then runs `AbsorptionMobEffect.onEffectStarted`.
    pub fn add_player_effect_from(
        &mut self,
        effect: EffectInstance,
        source: Option<EntityRef>,
    ) -> bool {
        // `LivingEntity.addEffect` refuses what cannot affect the entity;
        // players pass the tag check, but the `mob_effect_applicable` hook
        // is asked for every add.
        let key = effect
            .registry_index
            .and_then(mod_api::__private::effect_key_by_index);
        if !self.can_be_affected_resolved(None, "minecraft:player", effect.id, key) {
            return false;
        }
        let id = effect.id;
        let amp = effect.amplifier;
        // NeoForge's `MobEffectEvent.Added`: fires for every accepted effect
        // (vanilla or mod, new or reapplied, changed or unchanged) before
        // `activeEffects` is mutated.
        if mod_api::__private::listened(mod_api::HookId::MobEffectAdded)
            && let Some(key) = key
            && let Some(living) = self.effect_living_ref(None)
        {
            let old = self
                .common
                .potions
                .effects
                .get(id)
                .and_then(try_active_effect);
            let incoming = ActiveEffect::new(
                key,
                effect.duration,
                effect.amplifier,
                effect.ambient,
                effect.visible,
                effect.show_icon,
            );
            self.native_mob_effect_added(living, old.as_ref(), &incoming, source);
        }
        let new = self.common.potions.effects.get(id).is_none();
        let changed = self.apply_player_effect(effect);
        // `LivingEntity.onEffectAdded` + `MobEffect.onEffectAdded`: only on
        // first insertion (`existing == null`), after insertion and status
        // modifiers and before `onEffectStarted` (absorption).
        if new && changed {
            if let Some(sound) = sound_on_added(id)
                && let Some(p) = self.common.player.as_ref()
            {
                let at = [p.position.x, p.position.y, p.position.z];
                self.server_play_sound(sound, at, 1.0, 1.0);
            }
            if mod_api::__private::has_scoped_hook(mod_api::HookId::OnEffectAdded)
                && let Some(key) = key
                && let Some(living) = self.effect_living_ref(None)
            {
                self.native_on_effect_added(key, living, amp);
            }
        }
        if id == "absorption"
            && let Some(p) = self.common.player.as_mut()
        {
            let target = 4.0 * (amp + 1) as f32;
            let current = p.health.absorption();
            let _ = p.set_health(current.max(target), true);
        }
        changed
    }
    /// `LivingEntity.addEffect(MobEffectInstance, @Nullable Entity)` for a mob
    /// by entity `id`: refuses removed mobs and withers (`WitherBoss.addEffect`
    /// returns false for every effect), checks `canBeAffected` /
    /// `mob_effect_applicable`, fires NeoForge's `MobEffectEvent.Added` before
    /// mutating active effects, updates the mob's active effects and attribute
    /// modifiers (through `mutate_mob_effects` so open transactions journal
    /// the write), and on first insertion plays the vanilla added sound and
    /// dispatches `MobEffectPart::on_effect_added`.
    pub fn add_mob_effect(
        &mut self,
        id: u32,
        instance: EffectInstance,
        source: Option<EntityRef>,
    ) -> bool {
        let Some(name) = instance
            .registry_index
            .and_then(sunrise_game_data::potions::effect_by_index)
            .map(|e| brewing::namespaced(e.id))
            .or_else(|| mob_effect_id(instance.id))
        else {
            return false;
        };
        let Some(kind) = self.common.mobs.iter().find_map(|m| {
            (m.id == id && !m.removed && m.species() != sunrise_sim::mobs::Species::Wither)
                .then(|| m.kind.id.clone())
        }) else {
            return false;
        };
        let key = instance
            .registry_index
            .and_then(mod_api::__private::effect_key_by_index);
        if !self.can_be_affected_resolved(Some(id as i32), &kind, name, key) {
            return false;
        }
        let amplifier = instance.amplifier;
        // NeoForge's `MobEffectEvent.Added`: fires for every accepted effect
        // (vanilla or mod, new or reapplied, changed or unchanged) before
        // `activeEffects` is mutated.
        if mod_api::__private::listened(mod_api::HookId::MobEffectAdded)
            && let Some(key) = key
            && let Some(living) = self.effect_living_ref(Some(id as i32))
        {
            let old = self
                .common
                .mobs
                .iter()
                .find(|m| m.id == id && !m.removed)
                .and_then(|m| {
                    m.extra
                        .effects
                        .iter()
                        .find(|e| e.id == name)
                        .and_then(|e| try_mob_effect(m, e))
                });
            let incoming = ActiveEffect::new(
                key,
                instance.duration,
                instance.amplifier,
                instance.ambient,
                instance.visible,
                instance.show_icon,
            );
            self.native_mob_effect_added(living, old.as_ref(), &incoming, source);
        }
        let Some((new, changed)) = self.mutate_mob_effects(id, Some(name), |mob| {
            let new = !mob.has_effect(name);
            let changed = mob.add_effect_instance(name, instance);
            (new, changed)
        }) else {
            return false;
        };
        if new && changed {
            if let Some(sound) = sound_on_added(name)
                && let Some(m) = self.common.mobs.iter().find(|m| m.id == id && !m.removed)
            {
                let at = [m.position.x, m.position.y, m.position.z];
                self.server_play_sound(sound, at, 1.0, 1.0);
            }
            if mod_api::__private::has_scoped_hook(mod_api::HookId::OnEffectAdded)
                && let Some(key) = key
                && let Some(living) = self.effect_living_ref(Some(id as i32))
            {
                self.native_on_effect_added(key, living, amplifier);
            }
        }
        changed
    }
    /// Store an admitted effect. Only a real update replaces its modifier and
    /// refreshes pending attributes; an add writes the modifier but defers refresh.
    pub fn apply_player_effect(&mut self, effect: EffectInstance) -> bool {
        let id = effect.id;
        let new = self.common.potions.effects.get(id).is_none();
        let changed = self.common.potions.effects.add(effect);
        if changed {
            let active = self.common.potions.effects.get(id).unwrap();
            let (index, amplifier) = (active.registry_index, active.amplifier);
            self.update_player_effect_attributes(index, amplifier, !new);
            self.apply_effect_status();
        }
        changed
    }

    /// Removes one effect, or all effects, and refreshes the player's derived state.
    pub fn remove_player_effects(&mut self, effect: Option<usize>) -> bool {
        let mut removed = Vec::new();
        let mut index = 0;
        while index < self.common.potions.effects.list.len() {
            let current = &self.common.potions.effects.list[index];
            // The one being removed, or all of them when `effect` is None.
            if effect.is_some_and(|index| current.registry_index != Some(index)) {
                index += 1;
                continue;
            }
            let amplifier = current.amplifier;
            // `MobEffectEvent.Remove`: refusing the removal keeps it, and the
            // next one is asked with this one still in place, as in vanilla.
            let mut cancel = false;
            if let Some(living) = self.effect_living_ref(None) {
                self.native_mob_effect_remove(
                    living,
                    mod_api::__private::effect_key_by_index(
                        current
                            .registry_index
                            .expect("active effect registry index"),
                    )
                    .expect("effects resolved at load"),
                    amplifier,
                    &mut cancel,
                );
            }
            if cancel {
                index += 1;
            } else {
                removed.push(
                    self.common
                        .potions
                        .effects
                        .list
                        .remove(index)
                        .registry_index,
                );
            }
        }
        if !removed.is_empty() {
            self.remove_player_effect_attributes(&removed);
            self.apply_effect_status();
        }
        !removed.is_empty()
    }
    /// `MobEffectInstance.tickServer` for a content mod's effect: its
    /// `shouldApplyEffectTickThisTick`, then its `applyEffectTick`, which
    /// may end it. Whether it stays.
    pub(in crate::game::server_side) fn mod_effect_tick(
        &self,
        index: Option<usize>,
        entity: Option<i32>,
        tick_count: i32,
        amplifier: i32,
        holder: LivingRef,
        health: f32,
        max_health: f32,
        food: Option<PlayerFood>,
        active_effects: impl Iterator<Item = usize>,
    ) -> (bool, Vec<EffectTickChange>) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::ShouldApplyEffectTickThisTick) {
            return (true, Vec::new());
        }
        let Some(effect) = index.and_then(mod_api::__private::effect_key_by_index) else {
            static MISSING_EFFECT: std::sync::Once = std::sync::Once::new();
            MISSING_EFFECT.call_once(|| {
                log::debug!("Skipping mod effect tick: missing registry key or declaring mod for index {index:?}");
            });
            return (true, Vec::new());
        };
        let mut keep = true;
        let mut changes = Vec::new();
        let keys = std::cell::OnceCell::new();
        let (keep_out, changes_out) = (&mut keep, &mut changes);
        mod_api::__private::dispatch_mob_effect_tick(
            self,
            tick_count,
            effect,
            amplifier,
            |effect, amplifier| {
                let keys = keys.get_or_init(|| {
                    let mut keys = self.common.potions.tick_keys.borrow_mut();
                    keys.clear();
                    keys.extend(active_effects.filter_map(mod_api::__private::effect_key_by_index));
                    keys
                });
                mod_api::__private::ApplyEffectTickCall {
                    effect,
                    dimension: self.common.dim.id(),
                    entity,
                    amplifier,
                    holder,
                    health,
                    max_health,
                    food,
                    active_effects: keys,
                    changes: changes_out,
                    keep: keep_out,
                }
            },
        );
        (keep, changes)
    }
    /// `LivingEntity.onEffectAdded` and `onEffectUpdated`: only a server
    /// writes the modifiers, vanilla's or a content mod's declared ones; an
    /// attribute the player has no instance of takes none. The client
    /// receives them with its attributes. `index` is the effect's resolved
    /// registry index (`EffectInstance::registry_index`).
    pub(in crate::game) fn update_player_effect_attributes(
        &mut self,
        index: Option<usize>,
        amplifier: i32,
        refresh: bool,
    ) {
        let modifiers = index
            .and_then(sunrise_game_data::potions::effect_by_index)
            .into_iter()
            .flat_map(effect_modifiers);
        for (attribute, id, amount, operation) in modifiers {
            let amount = amount * f64::from(amplifier.wrapping_add(1));
            // MobEffect.addAttributeModifiers replaces by id and stores permanently.
            if let Some(write) = AttributeWrite::add(id, amount, operation, true, true) {
                self.write_attribute(Target::Player, attribute, write);
            }
        }
        if refresh {
            self.refresh_effect_attributes();
        }
    }

    /// `LivingEntity.onEffectsRemoved`: refresh even if there was no modifier
    /// to remove; other writes can still have attributes pending.
    pub(in crate::game) fn remove_player_effect_attributes(&mut self, effects: &[Option<usize>]) {
        for index in effects {
            for (attribute, id, _, _) in index
                .and_then(sunrise_game_data::potions::effect_by_index)
                .into_iter()
                .flat_map(effect_modifiers)
            {
                self.write_attribute(
                    Target::Player,
                    attribute,
                    AttributeWrite::Remove(id.to_owned()),
                );
            }
        }
        self.refresh_effect_attributes();
    }

    pub(in crate::game::server_side) fn refresh_effect_attributes(&mut self) {
        if let Some(player) = self.common.player.as_mut() {
            let _ = player.refresh_attributes();
        }
        self.apply_player_waypoint_updates();
    }
    /// `ThrowableProjectile.tick` for splash/lingering potions and experience bottles in flight:
    /// exact `ProjectileUtil.getHitResultOnMoveVector` block clip + entity sweep, live target
    /// visibility across bottles, vehicle-tree owner check after `setPos`, and Breeze deflection.
    pub(in crate::game::server_side) fn tick_thrown_potions(&mut self) {
        if self.common.potions.thrown.is_empty() {
            return;
        }
        let thrown = std::mem::take(&mut self.common.potions.thrown);
        let mut kept = Vec::with_capacity(thrown.len());
        // ServerLevel.tick: only entities in entity-ticking chunks tick.
        let ticking = self.entity_ticking_area();
        let below_world = self.common.world.below_world_y();
        for mut t in thrown {
            if !ticking(t.position.x.floor() as i32, t.position.z.floor() as i32) {
                t.previous = t.position;
                kept.push(t);
                continue;
            }
            t.age += 1;
            let owner = self.shooter_who(t.thrower, t.guest);
            t.previous = t.position;
            self.common.thrown_potion_motion(&mut t);
            let from = t.position;
            let to = from.add_vector(t.velocity);
            let bounds_at = |p: Vec3| {
                Aabb::new(
                    p.add_vector(Vec3::new(-0.125, 0.0, -0.125)),
                    p.add_vector(Vec3::new(0.125, 0.25, 0.125)),
                )
            };
            let query = sunrise_gameplay_common::game::creatures::TargetQuery {
                include_guests: true,
                ..Default::default()
            };
            let (block_clip, entity_hit) = {
                use sunrise_sim::projectiles::{NearestHit, ProjectileWorld, compute_margin};
                let live = self.live_proj_world(query, &[]);
                let block_clip = live.clip(from, to);
                let entity_end = block_clip.map_or(to, |(at, ..)| at);
                let search_area = bounds_at(from)
                    .expand_towards(t.velocity)
                    .inflate(Vec3::new(1.0, 1.0, 1.0));
                let margin = compute_margin(t.age as i32);
                let mut best = NearestHit::new();
                self.visit_projectile_targets(query, &[], |c| {
                    if !t.left_owner
                        && let Some(o) = owner
                        && self.in_same_vehicle_tree(o, c.who)
                    {
                        return;
                    }
                    best.consider(c, search_area, from, entity_end, margin);
                });
                (block_clip, best.finish())
            };
            let new_pos = entity_hit
                .map(|(_, at)| at)
                .or_else(|| block_clip.map(|(at, ..)| at))
                .unwrap_or(to);
            t.position = new_pos;
            self.entity_powder_snow_particles(
                t.previous,
                t.position,
                bounds_at(t.position),
                t.velocity,
                false,
            );
            // ThrowableProjectile.tick calls super.tick() (`checkLeftOwner`) after `setPos`.
            if !t.left_owner {
                let swept = bounds_at(t.position)
                    .expand_towards(t.velocity)
                    .inflate(Vec3::new(1.0, 1.0, 1.0));
                t.left_owner = owner.is_none_or(|o| self.outside_owner_collision_range(o, swept));
            }
            if let Some((who, hit_at)) = entity_hit {
                // Projectile.hitTargetOrDeflectSelf: Breeze deflects without splashing.
                if let Who::Mob(id) = who
                    && let Some(index) = self.common.mobs.iter().position(|m| {
                        m.id == id && m.alive() && m.species() == sunrise_sim::mobs::Species::Breeze
                    })
                {
                    if t.last_deflected_by != Some(id) {
                        let _ = t.random.next_float();
                        t.velocity = t.velocity.scale(-0.5);
                        t.last_deflected_by = Some(id);
                        self.play_mob_bound_sound(
                            index,
                            "minecraft:entity.breeze.deflect",
                            5,
                            1.0,
                            1.0,
                            false,
                        );
                    }
                    if t.age < 1200 && t.position.y >= below_world {
                        kept.push(t);
                    }
                    continue;
                }
                if let Who::Mob(id) = who {
                    self.redirect_projectile(id, owner, None, t.guest, None);
                }
                let target_pos = match who {
                    Who::Player => self.common.player.as_ref().map(|p| p.position),
                    Who::Mob(id) => self
                        .common
                        .mobs
                        .iter()
                        .find(|m| m.id == id)
                        .map(|m| m.position)
                        .or_else(|| {
                            self.live_target_bounds(who).map(|b| {
                                let c = b.center();
                                Vec3::new(c.x, b.min_position().y, c.z)
                            })
                        }),
                };
                let thrower = t.owner.is_some().then_some(Who::Player);
                let silent = self.entity_silent(t.id);
                let item = t.stack.item.to_string();
                let potion_owner = PotionOwner {
                    player: t.owner.is_some(),
                    name: t.owner,
                    attacker: None,
                    guest: t.guest,
                };
                self.splash(
                    &t.stack,
                    hit_at,
                    target_pos,
                    None,
                    t.age,
                    t.id,
                    &potion_owner,
                    silent,
                    t.velocity,
                );
                self.projectile_landed_event(t.id, thrower, [hit_at.x, hit_at.y, hit_at.z]);
                if item != "minecraft:experience_bottle" {
                    self.native_potion_splash(&item, [hit_at.x, hit_at.y, hit_at.z]);
                }
                continue;
            }
            if let Some((hit_at, pos, face)) = block_clip {
                let dir = Direction::approximate_nearest(Vec3::new(
                    f64::from(face[0]),
                    f64::from(face[1]),
                    f64::from(face[2]),
                ));
                let thrower = t.owner.is_some().then_some(Who::Player);
                let silent = self.entity_silent(t.id);
                let item = t.stack.item.to_string();
                let potion_owner = PotionOwner {
                    player: t.owner.is_some(),
                    name: t.owner,
                    attacker: None,
                    guest: t.guest,
                };
                self.splash(
                    &t.stack,
                    hit_at,
                    None,
                    Some((pos, dir)),
                    t.age,
                    t.id,
                    &potion_owner,
                    silent,
                    t.velocity,
                );
                self.projectile_block_landed_event(t.id, thrower, pos);
                if item != "minecraft:experience_bottle" {
                    self.native_potion_splash(&item, [hit_at.x, hit_at.y, hit_at.z]);
                }
                continue;
            }
            if t.age < 1200 && t.position.y >= below_world {
                kept.push(t);
            }
        }
        self.common.potions.thrown.splice(0..0, kept);
    }
    /// `AreaEffectCloud.readAdditionalSaveData`: loaded clouds join the same
    /// simulation as lingering potions, with their own timing and potion data.
    /// The caller retains their UUID and original compound for command queries.
    pub fn load_potion_cloud(&mut self, tag: &Tag) -> Option<u32> {
        self.load_potion_cloud_transferred(tag, None)
    }
    pub(in crate::game) fn load_potion_cloud_transferred(
        &mut self,
        tag: &Tag,
        transferred: Option<super::entity_initialization::TransferredEntityState>,
    ) -> Option<u32> {
        let (_, mut cloud) = potion_cloud_from_nbt(tag)?;
        let id = self.take_entity_id();
        cloud.id = id;
        if let Some(state) = transferred {
            self.adopt_transferred_entity_state(id, state);
        }
        self.common.breath_clouds.push(cloud);
        Some(id)
    }
    pub fn apply_effect_action(&mut self, action: EffectAction) {
        let action = match action {
            EffectAction::Heal(mut amount) => {
                if let Some(player) = self.common.player.as_ref() {
                    let mut cancel = false;
                    let reads = || crate::game::server_side::entity_server::hit_reads_of!(self);
                    let healed = sunrise_sim::health::LivingLend::player(
                        player.entity_id,
                        &player.attributes,
                        &player.health.data,
                        player.health.ids,
                        &reads,
                    );
                    sunrise_sim::hunger::native_living_heal(
                        &self.common.loop_context,
                        healed,
                        &mut amount,
                        &mut cancel,
                    );
                    if cancel {
                        return;
                    }
                }
                EffectAction::Heal(amount)
            }
            action => action,
        };
        let rules = self.common.world.health_rules;
        // An instant effect can land in a lent attack (a cloud's or a splash
        // potion's indirectMagic): its attacker is the source's causing entity.
        let causing = self.server.combat.0.attack().attacker;
        let Some(p) = self.common.player.as_mut() else {
            self.server.combat.0.forget_noted(false);
            return;
        };
        match action {
            EffectAction::Heal(amount) => {
                let max = p.attribute("minecraft:max_health").unwrap_or(20.0) as f32;
                let h = p.health.health();
                if h > 0.0 {
                    let _ = p.set_health((h + amount).min(max), false);
                }
            }
            EffectAction::Hurt(source, amount) => {
                let mut canceled = false;
                if let Some(id) = self.common.health_resources.damage.id(source)
                    && let Ok(o) = p.hurt_caused_by(
                        crate::game::server_side::entity_server::damage_level_of!(
                            self,
                            self.server.combat.0.attack(),
                            no_push,
                            None,
                            acting_equipment!(self.common)
                        ),
                        &self.common.health_resources,
                        rules,
                        id,
                        amount,
                        None,
                        false,
                        causing,
                        false,
                    )
                {
                    canceled = o.canceled;
                }

                self.server.combat.0.forget_noted(canceled);
                self.finish_guest_damage();
            }
            EffectAction::Poison => {
                if p.health.health() > 1.0
                    && let Some(id) = self.common.health_resources.damage.id("minecraft:magic")
                {
                    let _ = p.hurt(
                        crate::game::server_side::entity_server::damage_level_of!(
                            self,
                            self.server.combat.0.attack(),
                            no_push,
                            None,
                            acting_equipment!(self.common)
                        ),
                        &self.common.health_resources,
                        rules,
                        id,
                        1.0,
                    );
                    self.finish_guest_damage();
                }
            }
            EffectAction::Exhaust(amount) => {
                p.cause_food_exhaustion(amount);
            }
            EffectAction::Eat(nutrition, saturation) => {
                let food = &mut p.health.food;
                food.food = (food.food + nutrition).clamp(0, 20);
                food.saturation = (food.saturation + nutrition as f32 * saturation * 2.0)
                    .clamp(0.0, food.food as f32);
            }
            EffectAction::Update(_, index, amplifier) => {
                self.update_player_effect_attributes(index, amplifier, true)
            }
            EffectAction::Remove(_, index) => self.remove_player_effect_attributes(&[index]),
        }
    }
    /// `HealOrHarmMobEffect.applyInstantaneousEffect` with a source (a
    /// potion or a cloud): indirectMagic, not plain magic, credited to the
    /// mob that threw it when there is one.
    pub(in crate::game::server_side) fn apply_indirect_effect_action(
        &mut self,
        a: EffectAction,
        owner: &PotionOwner,
    ) {
        let a = match a {
            EffectAction::Hurt("minecraft:magic", amount) => {
                match (owner.attacker, &owner.name) {
                    (Some(id), Some(name)) => {
                        self.note_named_attacker("minecraft:indirect_magic", id, name.clone())
                    }
                    _ => {
                        let name = owner.name.clone().unwrap_or_else(|| {
                            Text::translate("entity.minecraft.splash_potion", &[])
                        });
                        self.note_direct_entity("minecraft:indirect_magic", name);
                    }
                }
                EffectAction::Hurt("minecraft:indirect_magic", amount)
            }
            a => a,
        };
        self.apply_effect_action(a);
    }
    /// `Arrow.doPostHurtEffects` and `SpectralArrow.doPostHurtEffects`: the
    /// arrow's item gives its target Glowing (spectral) or its potion contents
    /// at the stack's duration scale (a tipped arrow's default is an eighth);
    /// a mob's arrow without such an item (a stray's) carries its own effect.
    /// Instant effects act at once: harming hurts and healing heals, the
    /// other way round for the undead.
    pub(in crate::game::server_side) fn arrow_post_hurt_effects(
        &mut self,
        p: &Projectile,
        who: Who,
    ) {
        let from_item = p.arrow.as_ref().map(|a| &a.pickup_item);
        let mut effects: Vec<(&'static str, i32, i32)> = match from_item {
            // SpectralArrow.doPostHurtEffects: Glowing for the arrow's own
            // (saved) Duration.
            Some(s) if &*s.item == "minecraft:spectral_arrow" => {
                let ticks = p.arrow.as_ref().map_or(200, |a| a.spectral_duration);
                vec![("glowing", ticks, 0)]
            }
            Some(s) => {
                let scale = s.potion_duration_scale();
                brewing::contents_effects(s)
                    .into_iter()
                    .map(|(id, d, amp)| {
                        (id, sunrise_level::effects::scaled_duration(d, scale), amp)
                    })
                    .collect()
            }
            None => Vec::new(),
        };
        if effects.is_empty()
            && let ProjectileKind::Arrow {
                effect: Some((id, ticks)),
                ..
            } = p.kind
            && let Some(e) = brewing::effect(id)
        {
            effects.push((e.id, ticks, 0));
        }
        if effects.is_empty() {
            return;
        }
        // `Projectile.getEffectSource`: owner when present, otherwise the projectile itself.
        let effect_source = self
            .potion_owner_entity(p.owner, None)
            .or_else(|| self.entity(p.id));
        match who {
            Who::Player => {
                for (id, duration, amp) in effects {
                    if brewing::effect(id).is_some_and(|e| e.instant) {
                        if let Some(a) = sunrise_level::effects::instant(id, amp, 1.0) {
                            self.apply_effect_action(a);
                        }
                    } else {
                        self.add_player_effect_from(
                            EffectInstance::new(id, duration, amp),
                            effect_source,
                        );
                    }
                }
            }
            Who::Mob(id) => {
                let Some(i) = self
                    .common
                    .mobs
                    .iter()
                    .position(|m| m.id == id && m.alive())
                else {
                    return;
                };
                let inverted = self.items().entity_type_tags.has_bit(
                    self.common.mobs[i].kind.flags.entity_tag_slot,
                    sunrise_items::items::EntityTypeTags::INVERTED_HEALING_AND_HARM,
                );
                for (effect, duration, amp) in effects {
                    // A mod's `onEffectAdded` may have changed the mobs.
                    let Some(index) = self.common.mob_index.find(&self.common.mobs, id) else {
                        return;
                    };
                    let m = &mut self.common.mobs[index];
                    match effect {
                        "instant_damage" | "instant_health" => {
                            // `LivingEntity.addEffect` refuses nothing here; the
                            // tick hurts with magic (6 << amp) or heals (4 << amp).
                            if (effect == "instant_damage") != inverted {
                                m.damage(
                                    crate::game::server_side::entity_server::damage_level_of!(
                                        self,
                                        self.server.combat.0.attack(),
                                        no_push,
                                        None
                                    ),
                                    (6 << amp.min(30)) as f32,
                                    Damage::of(DamageKind::Magic),
                                );
                                self.finish_mob_damage_at(index);
                            } else {
                                m.heal(
                                    &self.common.loop_context,
                                    &|| {
                                        crate::game::server_side::entity_server::hit_reads_of!(self)
                                    },
                                    (4 << amp.min(30)) as f32,
                                );
                            }
                        }
                        _ => {
                            self.add_mob_effect(
                                id,
                                EffectInstance::new(effect, duration, amp),
                                effect_source,
                            );
                        }
                    }
                }
            }
        }
    }
    /// The dragon's breath (`AreaEffectCloud` with instant damage at
    /// `amplifier`) on a mob: what a cloud puts on it, `indirectMagic(cloud,
    /// dragon)` with no owner to credit (the dragon is not a mob here).
    pub(in crate::game) fn breath_affect_mob(
        &mut self,
        mob: u32,
        amplifier: i32,
        at: [f64; 3],
        cloud: Option<u32>,
    ) {
        let source = Text::translate("entity.minecraft.area_effect_cloud", &[]);
        let effect = [("instant_damage", 1, amplifier)];
        // DragonFireball.onHit and the flame phase: `setPotionDurationScale(0.25)`.
        self.cloud_affect(
            Victim::Mob(mob),
            &effect,
            (&[], &[], 0.25),
            at,
            &source,
            (None, None),
            cloud,
        );
    }
    /// What a cloud puts on one entity: instant effects at half strength
    /// (`applyInstantaneousEffect` with 0.5), the others at the cloud's duration
    /// scale (`PotionContents.forEachEffect`).
    pub fn cloud_affect(
        &mut self,
        victim: Victim,
        effects: &[(&'static str, i32, i32)],
        flags: (&[&'static str], &[&'static str], f32),
        at: [f64; 3],
        source: &Text,
        owners: (Option<Who>, Option<u32>),
        cloud: Option<u32>,
    ) {
        self.cloud_affect_from(victim, effects, flags, at, source, owners, cloud, None);
    }
    pub(in crate::game::server_side) fn cloud_affect_from(
        &mut self,
        victim: Victim,
        effects: &[(&'static str, i32, i32)],
        (hidden, ambient, duration_scale): (&[&'static str], &[&'static str], f32),
        at: [f64; 3],
        source: &Text,
        (owner, guest): (Option<Who>, Option<u32>),
        cloud: Option<u32>,
        effect_source: Option<EntityRef>,
    ) {
        // `indirectMagic(cloud, owner)` names the cloud in the death message
        // (it gives no kill credit).
        let cloud_owner = PotionOwner {
            name: Some(source.clone()),
            ..Default::default()
        };
        // MobEffectInstance.withScaledDuration: floor(duration * scale), at least 1; an infinite
        // effect stays infinite.
        let scaled = |duration| sunrise_level::effects::scaled_duration(duration, duration_scale);
        match victim {
            Victim::Player(who) => self.as_fight_player(who, |g| {
                let mut attack = g.potion_attack(
                    owner,
                    guest,
                    cloud.map(|id| (id, DirectEntity::Other, "minecraft:area_effect_cloud")),
                );
                attack.position = Some(at);
                for (id, duration, amp) in effects {
                    let Some(e) = brewing::effect(id) else {
                        continue;
                    };
                    if e.instant {
                        if let Some(a) = sunrise_level::effects::instant(id, *amp, 0.5) {
                            g.with_attack(attack.clone(), |g| {
                                g.apply_indirect_effect_action(a, &cloud_owner)
                            });
                        }
                    } else {
                        let mut instance = EffectInstance::new(e.id, scaled(*duration), *amp);
                        instance.visible = !hidden.contains(id);
                        instance.ambient = ambient.contains(id);
                        g.add_player_effect_from(instance, effect_source);
                    }
                }
            }),
            Victim::Mob(mob) => {
                let Some(i) = self.common.mobs.iter().position(|m| m.id == mob) else {
                    return;
                };
                // Undead invert healing and harming.
                let inverted = self.items().entity_type_tags.has_bit(
                    self.common.mobs[i].kind.flags.entity_tag_slot,
                    sunrise_items::items::EntityTypeTags::INVERTED_HEALING_AND_HARM,
                );
                for (id, duration, amp) in effects {
                    // A mod's effect hooks may have changed the mobs.
                    let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == mob) else {
                        return;
                    };
                    match *id {
                        "instant_damage" | "instant_health" => {
                            let amount = |base: i32| {
                                (0.5 * f64::from(base.wrapping_shl(*amp as u32)) + 0.5) as i32
                                    as f32
                            };
                            if (*id == "instant_damage") != inverted {
                                // HealOrHarmMobEffect.applyInstantaneousEffect: indirectMagic(cloud, owner).
                                let source = Damage::indirect_magic(
                                    "minecraft:area_effect_cloud",
                                    owner,
                                    at,
                                );
                                let attack = self.potion_attack(
                                    owner,
                                    guest,
                                    cloud.map(|id| {
                                        (id, DirectEntity::Other, "minecraft:area_effect_cloud")
                                    }),
                                );
                                let plan =
                                    self.prepare_knockback_credit(mob as i32, owner, &attack);
                                let mut credit = plan.lend(&mut self.server.last_hurt_by_player);
                                let Some(index) =
                                    self.common.mob_index.find(&self.common.mobs, mob)
                                else {
                                    return;
                                };
                                self.common.mobs[index].damage_by(
                                    crate::game::server_side::entity_server::damage_level_of!(
                                        self,
                                        &attack,
                                        &mut credit,
                                        None
                                    ),
                                    amount(6),
                                    source,
                                    "minecraft:area_effect_cloud",
                                );
                                self.finish_mob_damage_at(index);
                            } else {
                                m.heal(
                                    &self.common.loop_context,
                                    &|| {
                                        crate::game::server_side::entity_server::hit_reads_of!(self)
                                    },
                                    amount(4),
                                );
                            }
                        }
                        _ => {
                            let mut instance = EffectInstance::new(id, scaled(*duration), *amp);
                            instance.ambient = ambient.contains(id);
                            instance.visible = !hidden.contains(id);
                            self.add_mob_effect(mob, instance, effect_source);
                        }
                    }
                }
            }
        }
    }
    /// `AreaEffectCloud.serverTick`: configured lifetime/radius changes and
    /// five-tick doses, with each victim's configured reapplication delay.
    /// False once the cloud is gone.
    /// `Entity.baseTick`'s fluid steps for a cloud: `updateFluidInteraction`
    /// (the box is `radius * 2` wide and 0.5 high) resets the fall distance
    /// in water and adds the currents to its motion, then lava halves it once
    /// `first_tick` has cleared (`Entity.isInLava` is false on the first tick).
    pub(in crate::game) fn cloud_fluid_step(&self, c: &mut Breath) {
        let (r, [x, y, z]) = (f64::from(c.radius), c.at);
        let bounds = sunrise_core::geometry::Aabb::new(
            Vec3::new(x - r, y, z - r),
            Vec3::new(x + r, y + 0.5, z + r),
        );
        let mut motion = Vec3::new(c.motion[0], c.motion[1], c.motion[2]);
        // EntityDimensions.scalable: the eye at 0.85 of the height.
        let eye = y + f64::from(0.5_f32 * 0.85);
        let (water, lava, _) = sunrise_sim::entities::update_entity_fluids(
            bounds,
            Vec3::new(x, y, z),
            eye,
            &mut motion,
            &self.common.world,
            &self.common.res.motion().fluids,
            &self.common.loop_context,
        );
        c.motion = [motion.x, motion.y, motion.z];
        if water > 0.0 {
            c.fall_distance = 0.0;
        }
        if !c.first_tick && lava > 0.0 {
            c.fall_distance *= 0.5;
        }
        c.first_tick = false;
    }
    /// `LivingEntity.triggerOnDeathMobEffects` (`onMobRemoved` with the reason
    /// KILLED) for what Oozing, Weaving and Wind Charged do when their bearer
    /// dies. `bearer` is the dying mob's id, none when a player dies; `silent`
    /// whether that source is heard, and `random` the dying mob's own
    /// (`mob.getRandom()`; the world's for the player).
    pub(in crate::game) fn effects_on_death(
        &mut self,
        effects: &[&str],
        bounds: Aabb,
        silent: bool,
        bearer: Option<u32>,
        mut random: Option<&mut LegacyRandom>,
    ) {
        let player = bearer.is_none();
        let feet = bounds.bottom_center();
        if effects.contains(&"oozing") {
            // OozingMobEffect.numberOfSlimesToSpawn: two, fewer when the
            // slimes close by already fill the cramming limit.
            let limit = self
                .common
                .ui
                .rules
                .int_at(sunrise_game_data::gamerules::vanilla::MAX_ENTITY_CRAMMING);
            let mut count = 2;
            if limit >= 1 {
                let near = bounds.inflate(Vec3::new(2.0, 2.0, 2.0));
                let nearby = self
                    .common
                    .mobs
                    .iter()
                    .filter(|m| {
                        m.species() == sunrise_sim::mobs::Species::Slime
                            && Some(m.id) != bearer
                            && m.bounds().intersects(near)
                    })
                    .count()
                    .min(limit as usize) as i32;
                count = (limit - nearby).clamp(0, count);
            }
            for _ in 0..count {
                // spawnSlimeOffspring: a size 2 slime half a block up.
                if let Some(id) = self.spawn_mob(
                    "minecraft:slime",
                    [feet.x, feet.y + 0.5, feet.z],
                    false,
                    sunrise_gameplay_common::game::creatures::SpawnSite::Plain,
                ) && let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id)
                {
                    sunrise_sim::mobs::cube::set_size(m, 2);
                }
            }
        }
        // `WeavingMobEffect.onMobRemoved`: `canEntityGrief` for the dying mob.
        if effects.contains(&"weaving") && (player || self.can_entity_grief(bearer)) {
            self.spawn_cobwebs_around(
                [
                    feet.x.floor() as i32,
                    feet.y.floor() as i32,
                    feet.z.floor() as i32,
                ],
                random.as_deref_mut(),
            );
        }
        if effects.contains(&"wind_charged") {
            // A burst centred at half the height, radius 3 to 5.
            let center = [feet.x, feet.y + bounds.size().y / 2.0, feet.z];
            let r = random.unwrap_or(&mut self.common.world.random);
            let radius = 3.0 + r.next_float() * 2.0;
            // WindChargedMobEffect: the bearer bursts, a living source.
            let bearer = if player {
                Some(self.own_entity_id() as u32)
            } else {
                bearer
            };
            let cause = super::wind::BurstCause {
                source: bearer,
                owner: bearer,
            };
            let _ = self.wind_burst(
                center,
                radius,
                1.0,
                super::wind::Source::wind_charged(player, silent).with_cause(cause),
            );
        }
    }
    /// `ItemUtils.createFilledResult`: replace one of the held stack.
    pub(in crate::game) fn fill_held(&mut self, filled: ItemStack) {
        if self.common.creative {
            let _ = self.add_stack_to_inventory(filled);
            return;
        }
        self.consume_held();
        if self.common.held().is_none() {
            *self.common.held_mut() = Some(filled);
        } else if let Some(rest) = self.add_stack_to_inventory(filled)
            && let Some(eye) = self.common.eye_position()
        {
            // `player.drop(stack, false)`.
            self.throw_from_eyes(rest, eye, false, false);
        }
    }
    /// `PotionItem.finishUsingItem` (and `OminousBottleItem`'s) of `stack` in
    /// `slot`, the use complete: false for anything else.
    pub(in crate::game::server_side) fn finish_drinking(
        &mut self,
        slot: usize,
        stack: &ItemStack,
    ) -> bool {
        let ominous = &*stack.item == "minecraft:ominous_bottle";
        if &*stack.item != "minecraft:potion" && !ominous {
            return false;
        }
        let eye = self.common.eye_position().unwrap_or_default();
        self.common.using = None;
        self.send_use_item_complete();
        let consumable = self.consumable(stack);
        if let Some(consumable) = &consumable {
            self.emit_consume(consumable, stack, 16);
        }
        // Consumable.onConsume: the used statistic and the consume_item
        // trigger, with the stack as drunk (before it shrinks).
        self.award_stat("used", &stack.item, 1);
        self.trigger_item("minecraft:consume_item", stack);
        if ominous {
            self.drink_ominous_bottle(stack);
        } else {
            // PotionContents.onConsume: the potion's and the custom effects,
            // at the stack's POTION_DURATION_SCALE.
            let (lasting, now) = PlayerEffects::contents_effects(
                &brewing::contents_effects(stack),
                stack.potion_duration_scale(),
            );
            for e in lasting {
                self.add_player_effect(e);
            }
            for a in now {
                self.apply_effect_action(a);
            }
        }
        if let Some(consumable) = &consumable {
            self.apply_drink_effects(stack, consumable);
        }
        self.post_entity_event("minecraft:drink", sunrise_sim::mobs::Who::Player);
        if !self.common.creative && self.common.stack_in(slot) == Some(stack) {
            self.consume_player_slot(slot, 1);
            if !ominous {
                let bottle = ItemStack::new("minecraft:glass_bottle", 1);
                if self.common.stack_in(slot).is_none() {
                    *self.common.stack_in_mut(slot) = Some(bottle);
                } else if let Some(rest) = self.add_stack_to_inventory(bottle) {
                    self.spawn_item(rest, eye, [0.0, 0.1, 0.0], 40);
                }
            }
        }
        true
    }
    /// Resolve a potion or projectile owner into an `EntityRef` when live.
    pub(in crate::game::server_side) fn potion_owner_entity(
        &self,
        owner: Option<Who>,
        guest: Option<u32>,
    ) -> Option<EntityRef> {
        let id = match owner? {
            Who::Mob(id) => id,
            Who::Player => match guest {
                Some(conn) => self.server.lan.as_ref()?.hub.guest(conn)?.entity_id as u32,
                None => self.own_entity_id() as u32,
            },
        };
        self.entity(id)
    }
    /// The attack of a potion's or cloud's hit: its thrower and what they hold, for
    /// `living_knockback`, and the cloud or potion as the direct entity
    /// (`DamageSources.indirectMagic`), separately from the causing owner.
    pub(in crate::game::server_side) fn potion_attack(
        &self,
        owner: Option<Who>,
        guest: Option<u32>,
        direct: Option<(u32, DirectEntity, &str)>,
    ) -> Attack {
        let mut attack = match owner {
            Some(Who::Mob(id)) => self.attack_of(id as i32),
            Some(Who::Player) => self.attack_of(
                self.player_entity_id_by_connection(guest)
                    .unwrap_or_default(),
            ),
            None => Attack::default(),
        };
        if let Some((id, kind, entity_type)) = direct {
            attack.direct = Some(id as i32);
            attack.direct_kind = Some(kind);
            attack.direct_type = Some(entity_type.to_owned());
        }
        attack
    }
    /// `Projectile.getOwner` of a thrown potion, as [`splash`](Self::splash)
    /// names it: a player's, a mob's (a witch keeps the kill credit), or
    /// nobody's (a dispenser's, a summoned potion's).
    pub fn potion_owner(&self, owner: Option<Who>) -> PotionOwner {
        match owner {
            Some(Who::Player) => PotionOwner {
                name: Some(self.player_display_text()),
                attacker: None,
                player: true,
                guest: self.server.lan_acting,
            },
            Some(Who::Mob(id)) => PotionOwner {
                name: self.mob_display_name(id),
                attacker: Some(id),
                player: false,
                guest: None,
            },
            None => PotionOwner::default(),
        }
    }
    /// Effects saved in level.dat (`Player.active_effects`).
    pub(in crate::game) fn restore_effects(&mut self) {
        self.common.potions = PotionState::default();
        self.common.potions.effects = PlayerEffects::from_saved(&self.common.level.active_effects);
    }
    /// `WeavingMobEffect.spawnCobwebsRandomlyAround`: two or three cobwebs in
    /// free spaces with a solid floor, picked among 15 random blocks within one
    /// block of `pos`.
    pub(in crate::game::server_side) fn spawn_cobwebs_around(
        &mut self,
        pos: [i32; 3],
        mut random: Option<&mut LegacyRandom>,
    ) {
        let r = random
            .as_deref_mut()
            .unwrap_or(&mut self.common.world.random);
        let wanted = 2 + r.next_int_bound(2).unwrap_or(0) as usize;
        let mut chosen: Vec<[i32; 3]> = Vec::new();
        for _ in 0..15 {
            let r = random
                .as_deref_mut()
                .unwrap_or(&mut self.common.world.random);
            let at = [
                pos[0] - 1 + r.next_int_bound(3).unwrap_or(0),
                pos[1] - 1 + r.next_int_bound(3).unwrap_or(0),
                pos[2] - 1 + r.next_int_bound(3).unwrap_or(0),
            ];
            let facts = self.common.res.facts();
            let here = facts.get(self.common.world.block(at[0], at[1], at[2]));
            let below = facts.get(self.common.world.block(at[0], at[1] - 1, at[2]));
            if !chosen.contains(&at)
                && here.replaceable
                && below.sturdy_full & (1 << Direction::Up as u8) != 0
            {
                chosen.push(at);
                if chosen.len() >= wanted {
                    break;
                }
            }
        }
        for at in chosen {
            if self.set_state_named(at, "minecraft:cobweb", &[]).is_ok() {
                self.server_level_event_at(3018, at, 0);
            }
        }
    }
    /// `Creeper.spawnLingeringCloud`: an `AreaEffectCloud` of radius 2.5, wait time 10, duration 300,
    /// shrinking by `-radius / duration` a tick and by 0.5 for each entity it affects, applying
    /// the creeper's effects at 0.25 of their remaining duration.
    pub fn spawn_creeper_cloud(
        &mut self,
        at: [f64; 3],
        effects: Vec<(&'static str, i32, i32)>,
        hidden: Vec<&'static str>,
        ambient: Vec<&'static str>,
        color: u32,
    ) {
        let id = self.take_entity_id();
        self.common.breath_clouds.push(Breath {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id,
            at,
            motion: [0.0; 3],
            fall_distance: 0.0,
            fire_ticks: 0,
            first_tick: true,
            radius: 2.5,
            age: 0,
            duration: 300,
            from_dragon: false,
            victims: Vec::new(),
            potion: Some(PotionCloud {
                settings: CloudSettings::lingering(),
                effects,
                hidden,
                ambient,
                radius_per_tick: -2.5 / 300.0,
                color,
                custom_particle: None,
                victims: Vec::new(),
                source: Text::translate("entity.minecraft.area_effect_cloud", &[]),
                owner: None,
                guest: None,
            }),
        });
    }
    /// `AbstractThrownPotion.onHit` with `ThrownSplashPotion.onHitAsPotion`, or
    /// `ThrownLingeringPotion.onHitAsPotion` for a lingering potion, read from
    /// the stack's `potion_contents`; `mob` is where it hit a mob, `hit` the
    /// block and face it hit, `age` its age, `silent` its Silent flag, `motion` its motion as it hit.
    pub fn splash(
        &mut self,
        stack: &ItemStack,
        at: Vec3,
        mob: Option<Vec3>,
        hit: Option<([i32; 3], Direction)>,
        age: u32,
        potion_id: u32,
        owner: &PotionOwner,
        silent: bool,
        motion: Vec3,
    ) {
        let block = [at.x, at.y, at.z].map(|v| v.floor() as i32);
        if &*stack.item == "minecraft:experience_bottle" {
            // ThrownExperienceBottle.onHit: the water-coloured splash (level
            // event 2002, its sound 1053) and 3 to 11 experience.
            self.server_level_event_at(2002, block, 0xFF38_5DC6u32 as i32);
            if !silent {
                self.server_level_event_at(1053, block, 0);
            }
            let r = &mut self.common.world.random;
            let value = 3 + r.next_int_bound(5).unwrap_or(0) + r.next_int_bound(5).unwrap_or(0);
            // awardWithDirection: away from the face it hit, or back along
            // its own motion when it hit an entity (#11388).
            let roughly = match hit {
                Some((_, face)) => face.normal().map(f64::from),
                None => [-motion.x, -motion.y, -motion.z],
            };
            self.spawn_orbs_toward([at.x, at.y, at.z], roughly, value as u32);
            self.native_exp_bottle([at.x, at.y, at.z], value as u32);
            return;
        }
        // Projectile.onHit calls onHitBlock first, then the potion's
        // affectEntitiesAround; both read the stack's `potion_contents`.
        if let Some((cell, face)) = hit {
            // `AbstractThrownPotion.douseFire` passes `getOwner()` to the
            // campfire's `douse`, which is where the game event's source is.
            let who = if owner.player {
                Some(Who::Player)
            } else {
                owner.attacker.map(Who::Mob)
            };
            self.potion_hit_block(stack, cell, face, who);
        }
        self.affect_entities_around(stack, at, owner.player);
        let lingering = &*stack.item == "minecraft:lingering_potion";
        // AbstractThrownPotion.onHit: level event 2007 and sound 1054 for a
        // potion with instant effects, 2002 and 1053 otherwise.
        let potion = stack.potion().and_then(brewing::potion);
        let instant = potion.is_some_and(|d| {
            d.effects
                .iter()
                .any(|(id, ..)| brewing::effect(id).is_some_and(|e| e.instant))
        });
        // `PotionContents.getColor`: an explicit `custom_color` wins over the
        // amplifier-weighted mix of the potion's and the stack's custom effects.
        let color = brewing::contents_color(stack);
        self.server_level_event_at(if instant { 2007 } else { 2002 }, block, color as i32);
        if !silent {
            self.server_level_event_at(if instant { 1054 } else { 1053 }, block, 0);
        }
        // `PotionContents.hasEffects`: the potion's effects and the stack's
        // custom ones; without either `onHitAsPotion` is not called at all.
        let effects = brewing::contents_effects(stack);
        if effects.is_empty() {
            return;
        }
        if lingering {
            // The cloud sits at the mob it hit (its feet), else the impact;
            // radius 3, wait 10, 600 ticks, shrinking to nothing, with the
            // stack's `potion_contents` and `potion_duration_scale`.
            let at = mob.unwrap_or(at);
            let id = self.take_entity_id();
            self.common.breath_clouds.push(Breath {
                attachment_lifetime: Default::default(),
                join: Default::default(),
                id,
                at: [at.x, at.y, at.z],
                motion: [0.0; 3],
                fall_distance: 0.0,
                fire_ticks: 0,
                first_tick: true,
                radius: 3.0,
                age: 0,
                duration: 600,
                from_dragon: false,
                victims: Vec::new(),
                potion: Some(PotionCloud {
                    settings: CloudSettings {
                        duration_scale: stack.potion_duration_scale(),
                        ..CloudSettings::lingering()
                    },
                    effects,
                    hidden: Vec::new(),
                    ambient: Vec::new(),
                    radius_per_tick: -3.0 / 600.0,
                    color,
                    custom_particle: None,
                    victims: Vec::new(),
                    source: owner.name.clone().unwrap_or_else(|| {
                        Text::translate("entity.minecraft.area_effect_cloud", &[])
                    }),
                    owner: owner.who(),
                    guest: owner.guest,
                }),
            });
            return;
        }
        let duration_scale = f64::from(stack.potion_duration_scale());
        // Every player in range (`ThrownSplashPotion.applySplash` over
        // `LivingEntity`s, the players among them).
        self.for_each_player(|g| {
            g.splash_on_player(&effects, at, age, potion_id, owner, duration_scale)
        });
        // Mobs take the instant effects (undead invert healing and harming).
        let undead = &self.items().entity_type_tags;
        let effect_source = self.potion_owner_entity(owner.who(), owner.guest);
        let mut hits = Vec::new();
        for m in &self.common.mobs {
            if m.alive()
                && let Some(d2) = splash_distance_sqr(at, age, m.bounds())
            {
                let inverted = undead.has_bit(
                    m.kind.flags.entity_tag_slot,
                    sunrise_items::items::EntityTypeTags::INVERTED_HEALING_AND_HARM,
                );
                hits.push((m.id, 1.0 - d2.sqrt() / 4.0, inverted));
            }
        }
        for (mob_id, intensity, inverted) in hits {
            for (id, duration, amp) in &effects {
                let harm = match *id {
                    "instant_damage" => !inverted,
                    "instant_health" => inverted,
                    _ => {
                        // `Potion.applySplash`: timed effects last in
                        // proportion to how close the mob was.
                        let d = (intensity * f64::from(*duration) * duration_scale + 0.5) as i32;
                        if d > 20 {
                            self.add_mob_effect(
                                mob_id,
                                EffectInstance::new(id, d, *amp),
                                effect_source,
                            );
                        }
                        continue;
                    }
                };
                let Some(i) = self
                    .common
                    .mobs
                    .iter()
                    .position(|m| m.id == mob_id && !m.removed)
                else {
                    break;
                };
                let m = &mut self.common.mobs[i];
                if harm && m.species() == sunrise_sim::mobs::Species::Enderman {
                    // `Enderman.hurtServer` for a thrown potion: only water hurts
                    // (`hurtWithCleanWater`), then `repeatedlyTryToTeleport`, which
                    // the AI step runs and which does nothing while it rides.
                    if m.alive() && !m.extra.passenger {
                        m.extra.counter2 = 64;
                    }
                } else if harm {
                    // HealOrHarmMobEffect.applyInstantaneousEffect: indirectMagic(potion, owner).
                    let harm = (intensity * f64::from(6 << amp) + 0.5) as i32 as f32;
                    let source = Damage::indirect_magic(
                        "minecraft:splash_potion",
                        owner.who(),
                        [at.x, at.y, at.z],
                    );
                    let mob = m.id;
                    let mut attack = self.potion_attack(
                        owner.who(),
                        owner.guest,
                        (potion_id != 0).then_some((
                            potion_id,
                            DirectEntity::Projectile,
                            "minecraft:splash_potion",
                        )),
                    );
                    attack.projectile = self.server.combat.0.attack().projectile;
                    attack.projectile_kind = self.server.combat.0.attack().projectile_kind;
                    attack.projectile_type = self.server.combat.0.attack().projectile_type;
                    attack.enchantment_direct =
                        self.server.combat.0.attack().enchantment_direct.clone();
                    let plan = self.prepare_knockback_credit(mob as i32, owner.who(), &attack);
                    let mut credit = plan.lend(&mut self.server.last_hurt_by_player);
                    self.common.mobs[i].damage_by(
                        crate::game::server_side::entity_server::damage_level_of!(
                            self,
                            &attack,
                            &mut credit,
                            None
                        ),
                        harm,
                        source,
                        "minecraft:splash_potion",
                    );
                    self.finish_mob_damage_at(i);
                } else {
                    m.heal(
                        &self.common.loop_context,
                        &|| crate::game::server_side::entity_server::hit_reads_of!(self),
                        (intensity * f64::from(4 << amp) + 0.5) as i32 as f32,
                    );
                }
            }
        }
    }
    /// A splash potion's effects on the player in place, by their distance,
    /// for the stack's `potion_duration_scale`.
    pub(in crate::game::server_side) fn splash_on_player(
        &mut self,
        effects: &[(&'static str, i32, i32)],
        at: Vec3,
        age: u32,
        potion_id: u32,
        owner: &PotionOwner,
        duration_scale: f64,
    ) {
        if let Some(p) = self.common.player.as_ref() {
            let w = f64::from(p.dimensions.width) / 2.0;
            let bounds = Aabb::new(
                Vec3::new(p.position.x - w, p.position.y, p.position.z - w),
                Vec3::new(
                    p.position.x + w,
                    p.position.y + f64::from(p.dimensions.height),
                    p.position.z + w,
                ),
            );
            if let Some(d2) = splash_distance_sqr(at, age, bounds) {
                let intensity = 1.0 - d2.sqrt() / 4.0;
                let mut attack = self.potion_attack(
                    owner.who(),
                    owner.guest,
                    (potion_id != 0).then_some((
                        potion_id,
                        DirectEntity::Projectile,
                        "minecraft:splash_potion",
                    )),
                );
                attack.position = Some([at.x, at.y, at.z]);
                for (id, duration, amp) in effects {
                    let instant = brewing::effect(id).is_some_and(|e| e.instant);
                    if instant {
                        if let Some(a) = sunrise_level::effects::instant(id, *amp, intensity) {
                            self.with_attack(attack.clone(), |g| {
                                g.apply_indirect_effect_action(a, owner)
                            });
                        }
                    } else {
                        let d = (intensity * f64::from(*duration) * duration_scale + 0.5) as i32;
                        if d > 20
                            && let Some(e) = brewing::effect(id)
                        {
                            let effect_source = self.potion_owner_entity(owner.who(), owner.guest);
                            self.add_player_effect_from(
                                EffectInstance::new(e.id, d, *amp),
                                effect_source,
                            );
                        }
                    }
                }
            }
        }
    }
    /// Drinking a potion while right-click is held; true when handled.
    pub(in crate::game::server_side) fn tick_drinking(
        &mut self,
        slot: usize,
        ticks: u32,
        stack: &ItemStack,
    ) -> bool {
        let ominous = &*stack.item == "minecraft:ominous_bottle";
        if &*stack.item != "minecraft:potion" && !ominous {
            return false;
        }
        // the duration the use started with (32, as a mod's Start or Tick left it).
        let duration = self.common.use_duration;
        let remaining = duration.saturating_sub_unsigned(ticks);
        let ticks = ticks + 1;
        // ItemStack.onUseTick runs the consumable's own emission before the item's tick.
        if remaining > 0
            && let Some(consumable) = self.consumable(stack)
            && consumable.should_emit(remaining)
        {
            self.emit_consume(&consumable, stack, 5);
        }
        // ItemStack.onUseTick: the item's own tick after the consumable's.
        if !self.item_on_use_tick(stack, ticks - 1) {
            return true;
        }
        if i64::from(ticks) < i64::from(duration) {
            self.common.using = Some((slot, ticks));
            return true;
        }
        self.finish_drinking(slot, stack);
        self.use_item_finished(stack);
        true
    }
    pub(in crate::game) fn tick_potion_cloud(
        &mut self,
        c: &mut Breath,
        players: &[FightPlayer],
    ) -> bool {
        c.age = c.age.wrapping_add(1);
        let cloud_id = c.id;
        let Some(potion) = c.potion.as_mut() else {
            return true;
        };
        if c.duration != -1 && c.age.wrapping_sub(potion.settings.wait_time) >= c.duration {
            return false;
        }
        if c.age < potion.settings.wait_time {
            return true;
        }
        if potion.radius_per_tick != 0.0 {
            c.radius += potion.radius_per_tick;
            if c.radius < 0.5 {
                return false;
            }
            c.radius = c.radius.clamp(0.0, 32.0);
        }
        if c.age % 5 != 0 {
            return true;
        }
        potion.victims.retain(|(_, until)| c.age < *until);
        if potion.effects.is_empty() {
            potion.victims.clear();
            return true;
        }
        let r = f64::from(c.radius);
        let area = Aabb::new(
            Vec3::new(c.at[0] - r, c.at[1], c.at[2] - r),
            Vec3::new(c.at[0] + r, c.at[1] + 0.5, c.at[2] + r),
        );
        let mut inside = Vec::new();
        for p in players.iter().filter(|p| p.bounds.intersects(area)) {
            inside.push((Victim::Player(p.who), p.position.x, p.position.z));
        }
        let effects = potion.effects.clone();
        // Only mobs that at least one of the cloud's effects can affect are
        // taken (`allEffects.stream().anyMatch(entity::canBeAffected)`), so
        // the others neither become victims nor use up its radius.
        let candidates: Vec<(u32, String, f64, f64)> = self
            .common
            .mobs
            .iter()
            .filter(|m| m.alive() && m.bounds().intersects(area))
            .map(|m| (m.id, m.kind.id.clone(), m.position.x, m.position.z))
            .collect();
        for (mob_id, kind, x, z) in candidates {
            let affected = effects
                .iter()
                .any(|(id, ..)| self.can_be_affected(Some(mob_id as i32), &kind, id));
            if affected {
                inside.push((Victim::Mob(mob_id), x, z));
            }
        }
        for (victim, x, z) in inside {
            let (dx, dz) = (x - c.at[0], z - c.at[2]);
            if potion.victims.iter().any(|(v, _)| *v == victim)
                || dx * dx + dz * dz > f64::from(c.radius * c.radius)
            {
                continue;
            }
            potion.victims.push((
                victim,
                c.age.wrapping_add(potion.settings.reapplication_delay),
            ));
            let cloud_source = (c.id != 0)
                .then(|| {
                    self.common
                        .native_lifecycle
                        .epoch()
                        .map(|epoch| EntityRef::new(epoch, c.id))
                })
                .flatten();
            self.cloud_affect_from(
                victim,
                &effects,
                (
                    &potion.hidden,
                    &potion.ambient,
                    potion.settings.duration_scale,
                ),
                c.at,
                &potion.source,
                (potion.owner, potion.guest),
                Some(cloud_id),
                cloud_source,
            );
            if potion.settings.radius_on_use != 0.0 {
                c.radius += potion.settings.radius_on_use;
                if c.radius < 0.5 {
                    return false;
                }
                c.radius = c.radius.clamp(0.0, 32.0);
            }
            if potion.settings.duration_on_use != 0 && c.duration != -1 {
                c.duration = c.duration.wrapping_add(potion.settings.duration_on_use);
                if c.duration <= 0 {
                    return false;
                }
            }
        }
        true
    }
    /// The authoritative `AreaEffectCloud.serverTick` for lingering potions' clouds.
    /// The logical client draws particles from tracked cloud data.
    pub fn tick_potion_clouds(&mut self) {
        if self.common.breath_clouds.iter().all(|c| c.potion.is_none()) {
            return;
        }
        let players: Vec<FightPlayer> = self
            .fight_players()
            .into_iter()
            .filter(|p| p.alive)
            .collect();
        let mut clouds = self.common.breath_clouds.lift();
        #[cfg(any(test, feature = "test-support"))]
        crate::game::server_side::entity_server::taken_pass_tests::after_take(self);
        let ticking = self.entity_ticking_area();
        let mut expired: Vec<u32> = Vec::new();
        clouds.retain_mut(|c| {
            let keep =
                c.potion.is_none() || !ticking(c.at[0].floor() as i32, c.at[2].floor() as i32) || {
                    self.cloud_fluid_step(c);
                    // Entity.baseTick: `fireImmune()` clears the fire.
                    c.fire_ticks = c.fire_ticks.min(0);
                    self.tick_potion_cloud(c, &players)
                };
            if !keep {
                expired.push(c.id);
            }
            keep
        });
        self.common
            .breath_clouds
            .restore(clouds, sunrise_sim::join_seq::RestoreOrder::SpawnsFirst);
        // A cloud that expired on its own takes its `noPhysics` write with it.
        for id in expired {
            self.forget_entity(id);
        }
    }
    /// Right-click with a potion, splash/lingering potion or glass bottle.
    pub(in crate::game::server_side) fn use_potion_item(
        &mut self,
        stack: &ItemStack,
    ) -> sunrise_game_data::Result<bool> {
        match &*stack.item {
            "minecraft:potion" | "minecraft:ominous_bottle" => {
                self.start_using_item();
                Ok(true)
            }
            "minecraft:splash_potion"
            | "minecraft:lingering_potion"
            | "minecraft:experience_bottle" => {
                let Some(eye) = self.common.eye_position() else {
                    return Ok(true);
                };
                let pitch = 0.4 / (self.common.world.random.next_float() * 0.4 + 0.8);
                let (sound, power) = match &*stack.item {
                    "minecraft:splash_potion" => ("minecraft:entity.splash_potion.throw", 0.5),
                    "minecraft:experience_bottle" => {
                        ("minecraft:entity.experience_bottle.throw", 0.7)
                    }
                    _ => ("minecraft:entity.lingering_potion.throw", 0.5),
                };
                self.server_play_sound(sound, eye, 0.5, pitch);
                // Projectile.shootFromRotation(xRot, yRot, -20, power, 1.0), with
                // what the thrower last moved (getKnownMovement).
                let velocity = self.known_player_movement();
                let on_ground = self
                    .common
                    .player
                    .as_ref()
                    .is_some_and(|p| p.ground.on_ground);
                let one = ItemStack {
                    count: 1,
                    ..stack.clone()
                };
                let mut shot =
                    thrown_projectile(one, Vec3::new(eye[0], eye[1] - 0.1, eye[2]), true);
                shot.shoot_from_rotation(
                    self.common.pitch,
                    self.common.yaw,
                    -20.0,
                    power,
                    1.0,
                    velocity,
                    on_ground,
                );
                shot.guest = self.server.lan_acting;
                self.add_fresh_projectile(shot);
                // ThrowablePotionItem.use / ExperienceBottleItem.use: awardStat(ITEM_USED).
                self.award_stat("used", &stack.item, 1);
                self.swing_arm();
                self.consume_held();
                Ok(true)
            }
            "minecraft:glass_bottle" => {
                // `BottleItem.use`: before the water check, it looks for a living
                // AreaEffectCloud owned by an EnderDragon within the player's box
                // grown by 2. If one is found, it shrinks the cloud's radius by 0.5,
                // plays item.bottle.fill_dragonbreath at the player, emits
                // GameEvent.FLUID_PICKUP, fires PLAYER_INTERACTED_WITH_ENTITY on the
                // cloud, and turns one bottle into dragon_breath.
                if let Some(player) = self.common.player.as_ref()
                    && let Some(cloud_at) =
                        collect_dragon_breath(player.bounds(), &mut self.common.breath_clouds)
                {
                    let player_pos = [player.position.x, player.position.y, player.position.z];
                    self.server_play_sound(
                        "minecraft:item.bottle.fill_dragonbreath",
                        player_pos,
                        1.0,
                        1.0,
                    );
                    self.post_entity_event("minecraft:fluid_pickup", Who::Player);
                    let cloud = sunrise_sim::advancements::EntityView {
                        kind: "minecraft:area_effect_cloud".into(),
                        position: cloud_at,
                        ..Default::default()
                    };
                    self.trigger_item_entity(
                        "minecraft:player_interacted_with_entity",
                        stack,
                        &cloud,
                    );
                    // BottleItem.turnBottleIntoItem.
                    self.award_stat("used", &stack.item, 1);
                    self.fill_held(ItemStack::new("minecraft:dragon_breath", 1));
                    return Ok(true);
                }
                let Some(hit) = self.pick_fluid() else {
                    return Ok(false);
                };
                let state = self.common.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
                if self.common.res.facts().get(state).fluid != sunrise_items::blocks::Fluid::Water {
                    return Ok(false);
                }
                // The sound at the player, the game event at the water block.
                let Some(at) = self
                    .common
                    .player
                    .as_ref()
                    .map(|p| [p.position.x, p.position.y, p.position.z])
                else {
                    return Ok(false);
                };
                self.server_play_sound("minecraft:item.bottle.fill", at, 1.0, 1.0);
                self.post_block_game_event(
                    "minecraft:fluid_pickup",
                    hit.pos,
                    Some(Who::Player),
                    None,
                );
                // BottleItem.turnBottleIntoItem.
                self.award_stat("used", &stack.item, 1);
                let water = ItemStack::new("minecraft:potion", 1).with_potion("minecraft:water");
                self.fill_held(water);
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    /// `PotionItem.useOn`: a water bottle used on a block that turns to mud
    /// (`#convertible_to_mud`), not on its underside, is poured out and the
    /// block becomes mud.
    pub(in crate::game::server_side) fn use_water_bottle_on_block(
        &mut self,
        pos: [i32; 3],
        face: [i32; 3],
        stack: &ItemStack,
    ) -> sunrise_game_data::Result<bool> {
        if &*stack.item != "minecraft:potion"
            || stack.potion() != Some("minecraft:water")
            || face == [0, -1, 0]
            || !self.named_block_in_tag("minecraft:convertible_to_mud", &self.block_name(pos))
        {
            return Ok(false);
        }
        let centre = pos.map(|v| f64::from(v) + 0.5);
        self.server_play_sound("minecraft:entity.generic.splash", centre, 1.0, 1.0);
        self.fill_held(ItemStack::new("minecraft:glass_bottle", 1));
        // PotionItem.useOn: five ServerLevel.sendParticles(SPLASH). The level
        // random places each one (count 1, no spread, speed 1).
        for _ in 0..5 {
            let r = &mut self.common.world.random;
            let at = [
                f64::from(pos[0]) + r.next_double(),
                f64::from(pos[1] + 1),
                f64::from(pos[2]) + r.next_double(),
            ];
            self.server_send_particles(sunrise_sim::lan::server_effects::ServerQueuedParticle {
                dimension: self.common.dim.id(),
                particle: sunrise_sim::lan::server_effects::ServerParticle::simple(
                    ParticleType::Splash,
                ),
                at,
                spread: [0.0; 3],
                speed: [1.0; 3],
                count: 1,
                randomization: 0,
                except: None,
            });
        }
        self.server_play_sound("minecraft:item.bottle.empty", centre, 1.0, 1.0);
        self.post_block_game_event("minecraft:fluid_place", pos, None, None);
        if let Some(mud) = self
            .common
            .pack
            .assets
            .registry
            .default_state("minecraft:mud")
        {
            self.level_set(pos, mud)?;
        }
        self.award_stat("used", &stack.item, 1);
        self.swing_arm();
        Ok(true)
    }
}

/// Convert a player's `EffectInstance` to an `ActiveEffect` when its registry key is resolved.
fn try_active_effect(e: &EffectInstance) -> Option<ActiveEffect> {
    let key = e
        .registry_index
        .and_then(mod_api::__private::effect_key_by_index)?;
    Some(ActiveEffect::new(
        key,
        e.duration,
        e.amplifier,
        e.ambient,
        e.visible,
        e.show_icon,
    ))
}

/// Convert a mob's stored effect to an `ActiveEffect` when its registry key is resolved.
fn try_mob_effect(m: &Mob, e: &sunrise_sim::mobs::StoredEffect) -> Option<ActiveEffect> {
    let key = e
        .registry_index
        .and_then(mod_api::__private::effect_key_by_index)?;
    Some(ActiveEffect::new(
        key,
        e.duration,
        e.amplifier,
        m.extra.ambient_effects.contains(&e.id),
        !m.extra.hidden_effects.contains(&e.id),
        e.show_icon,
    ))
}

/// The live player's effect pass (`PlayerEffects::tick_on`), keeping
/// `self.common.potions.effects` resident on `ServerGame` without cloning
/// `EffectInstance`s and checking `MobEffectEvent.Expired` only when an effect
/// actually expires and a listener is subscribed.
impl EffectTickHost for ServerGame<'_> {
    fn effects(&self) -> &PlayerEffects {
        &self.common.potions.effects
    }
    fn effects_mut(&mut self) -> &mut PlayerEffects {
        &mut self.common.potions.effects
    }
    fn apply_action(&mut self, action: EffectAction) {
        self.apply_effect_action(action);
    }
    fn tick_special(&mut self, e: TickingEffect, t: i32) -> OmenTick {
        match e.id {
            "bad_omen" => self.omen_tick(e),
            "raid_omen" if t == 1 => self.omen_tick(e),
            id if brewing::is_mod_effect(id) => {
                let Some((holder_id, health, max_health, food)) =
                    self.common.player.as_mut().map(|player| {
                        let max_health =
                            player.attribute("minecraft:max_health").unwrap_or(20.0) as f32;
                        let food = &player.health.food;
                        (
                            player.entity_id as u32,
                            player.health.health(),
                            max_health,
                            Some(PlayerFood::from(mod_api::__private::built::PlayerFood {
                                level: food.food,
                                saturation: food.saturation,
                                exhaustion: food.exhaustion,
                            })),
                        )
                    })
                else {
                    return OmenTick::Remove;
                };
                if let Some(epoch) = self.common.native_lifecycle.epoch() {
                    let holder = LivingRef::new(EntityRef::new(epoch, holder_id));
                    let (keep, changes) = self.mod_effect_tick(
                        e.registry_index,
                        None,
                        t,
                        e.amplifier,
                        holder,
                        health,
                        max_health,
                        food,
                        self.common
                            .potions
                            .effects
                            .list
                            .iter()
                            .filter_map(|effect| effect.registry_index),
                    );
                    let actions: Vec<EffectAction> = {
                        let registry = &self.common.health_resources.damage;
                        changes
                            .into_iter()
                            .filter_map(|change| match change {
                                EffectTickChange::Heal(amount) => Some(EffectAction::Heal(amount)),
                                // An unregistered damage type hurts nothing.
                                EffectTickChange::Hurt(source, amount) => {
                                    NamedDamage::registered(registry, &source)
                                        .map(|damage| EffectAction::Hurt(damage.name, amount))
                                }
                                EffectTickChange::AddExhaustion(amount) => {
                                    Some(EffectAction::Exhaust(amount))
                                }
                                EffectTickChange::Eat(nutrition, saturation) => {
                                    Some(EffectAction::Eat(nutrition, saturation))
                                }
                                _ => unreachable!("EffectTickChange: unhandled variant"),
                            })
                            .collect()
                    };
                    for action in actions {
                        self.apply_effect_action(action);
                    }
                    if keep {
                        OmenTick::Continue
                    } else {
                        OmenTick::Remove
                    }
                } else {
                    OmenTick::Continue
                }
            }
            "absorption"
                if self
                    .common
                    .player
                    .as_ref()
                    .is_some_and(|p| p.health.absorption() <= 0.0) =>
            {
                OmenTick::Remove
            }
            _ => OmenTick::Continue,
        }
    }
    fn add_effect(&mut self, effect: EffectInstance) -> bool {
        let target_id = effect.id;
        let had = self.common.potions.effects.get(target_id).is_some();
        self.add_player_effect(effect);
        !had && self.common.potions.effects.get(target_id).is_some()
    }
    fn on_promoted(&mut self, _: &'static str, registry_index: Option<usize>, amplifier: i32) {
        self.update_player_effect_attributes(registry_index, amplifier, true);
    }
    fn on_removed(&mut self, _: &'static str, registry_index: Option<usize>) {
        self.remove_player_effect_attributes(&[registry_index]);
    }
    fn keep_expired(&self, effect: TickingEffect) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::MobEffectExpired) {
            return false;
        }
        let Some(player) = self.effect_living_ref(None) else {
            return false;
        };
        let index = effect.registry_index.expect("active effect registry index");
        let mut cancel = false;
        self.native_mob_effect_expired(
            player,
            mod_api::__private::effect_key_by_index(index).expect("effects resolved at load"),
            effect.amplifier,
            &mut cancel,
        );
        cancel
    }
}

/// A mob's content mod effects tick in its own tick (`ai::tick_effects_hurting`).
impl sunrise_sim::mobs::ModEffectTicks for ServerGame<'_> {
    fn tick_mod_effect(
        &self,
        effect: usize,
        mob: &Mob,
        tick_count: i32,
        amplifier: i32,
    ) -> ModEffectTick {
        let Some(epoch) = self.common.native_lifecycle.epoch() else {
            return ModEffectTick {
                keep: true,
                actions: Vec::new(),
            };
        };
        let holder = LivingRef::new(EntityRef::new(epoch, mob.id));
        let (keep, changes) = self.mod_effect_tick(
            Some(effect),
            Some(mob.id as i32),
            tick_count,
            amplifier,
            holder,
            mob.health,
            mob.attrs.max_health,
            None,
            mob.extra
                .effects
                .iter()
                .filter_map(|effect| effect.registry_index),
        );
        let registry = &self.common.health_resources.damage;
        let actions = changes
            .into_iter()
            .filter_map(|change| match change {
                EffectTickChange::Heal(amount) => Some(MobEffectAction::Heal(amount)),
                // An unregistered damage type hurts nothing.
                EffectTickChange::Hurt(source, amount) => {
                    Self::registered_damage_kind(registry, &source)
                        .map(|kind| MobEffectAction::Hurt(kind, amount))
                }
                // Mobs have no FoodData.
                _ => None,
            })
            .collect();
        ModEffectTick { keep, actions }
    }
    fn effect_expired(&self, mob: u32, effect: usize, amplifier: i32, cancel: &mut bool) {
        if let Some(living) = self.effect_living_ref(Some(mob as i32))
            && let Some(effect) = mod_api::__private::effect_key_by_index(effect)
        {
            self.native_mob_effect_expired(living, effect, amplifier, cancel);
        }
    }
}

/// `Projectile.getOwner` of a thrown potion: what its `indirectMagic` damage
/// is named after and who it counts against. Nothing for a dispenser's or a
/// summoned potion nobody owns, the potion itself then naming the death
/// message.
#[derive(Clone, Debug, Default)]
pub struct PotionOwner {
    /// The thrower's name, when it has one.
    pub name: Option<Text>,
    /// The mob that threw it (a witch): the hurt is its kill credit, as
    /// `indirectMagic(potion, owner)` gives it. A player's throw is the
    /// potion's own and only `name`s the message.
    pub attacker: Option<u32>,
    /// A player threw it: the water damage `indirectMagic` deals is theirs.
    pub player: bool,
    /// On a server, the guest (by connection) who threw it: whose kill the mobs it hurts are
    /// (`LivingEntity.lastHurtByPlayer`).
    pub guest: Option<u32>,
}

impl PotionOwner {
    /// The owner as a damage source's causing entity.
    pub(in crate::game) fn who(&self) -> Option<Who> {
        match self.attacker {
            Some(id) => Some(Who::Mob(id)),
            None => self.player.then_some(Who::Player),
        }
    }
}

/// A mob's effect ids carry the `minecraft:` namespace; a mod's has its own.
pub(in crate::game) fn mob_effect_id(id: &str) -> Option<&'static str> {
    brewing::effect(id).map(|e| brewing::namespaced(e.id))
}

/// `LivingEntity.canBeAffected` with the mob overrides (`WitherBoss` and
/// `WitherSkeleton` shrug off wither, `Spider` and `AbstractNautilus` poison,
/// `Parched` weakness): the immune-to-infested and immune-to-oozing tags
/// decide first, then the tag of mobs that ignore poison and regeneration.
/// `WitherBoss.addEffect` refusing everything is `Mob::add_effect`'s: a cloud
/// still takes the wither as a victim (`AreaEffectCloud` asks `canBeAffected`),
/// so its instant effects land and it shrinks.
pub(in crate::game) fn mob_can_be_affected(
    tags: &sunrise_items::items::EntityTypeTags,
    kind: &str,
    slot: Option<usize>,
    effect: &str,
) -> bool {
    use sunrise_items::items::EntityTypeTags;
    let effect = effect.trim_start_matches("minecraft:");
    let overridden = match kind.trim_start_matches("minecraft:") {
        "wither" | "wither_skeleton" => effect == "wither",
        "spider" | "cave_spider" | "nautilus" | "zombie_nautilus" => effect == "poison",
        "parched" => effect == "weakness",
        _ => false,
    };
    if overridden {
        return false;
    }
    let in_tag = |flag| tags.has_bit(slot, flag);
    if in_tag(EntityTypeTags::IMMUNE_TO_INFESTED) {
        effect != "infested"
    } else if in_tag(EntityTypeTags::IMMUNE_TO_OOZING) {
        effect != "oozing"
    } else {
        !in_tag(EntityTypeTags::IGNORES_POISON_AND_REGEN)
            || (effect != "regeneration" && effect != "poison")
    }
}

/// `ThrownSplashPotion.onHitAsPotion`: the squared distance from the potion's
/// box at the hit point to an entity's box grown by the hit margin
/// (`ProjectileUtil.computeMargin`), for entities that meet the potion's box
/// grown by (4, 2, 4); none when it is 16 or more.
pub(in crate::game) fn splash_distance_sqr(at: Vec3, age: u32, bounds: Aabb) -> Option<f64> {
    let potion = Aabb::new(
        Vec3::new(at.x - 0.125, at.y, at.z - 0.125),
        Vec3::new(at.x + 0.125, at.y + 0.25, at.z + 0.125),
    );
    if !potion.inflate(Vec3::new(4.0, 2.0, 4.0)).intersects(bounds) {
        return None;
    }
    let margin = ((f64::from(age) - 2.0) / 20.0).clamp(0.0, 0.3);
    let d2 = potion.distance_squared_to_box(bounds.inflate(Vec3::new(margin, margin, margin)));
    (d2 < 16.0).then_some(d2)
}

/// A splash potion, lingering potion or experience bottle about to fly (the
/// player's throws and dispensers both make one): `ThrownSplashPotion`,
/// `ThrownLingeringPotion` or `ThrownExperienceBottle`, which
/// `level.addFreshEntity` makes a level entity like a summoned one, so
/// selectors and `/data` find it (#13201).
pub fn thrown_projectile(stack: ItemStack, position: Vec3, by_player: bool) -> Projectile {
    let kind = match &*stack.item {
        "minecraft:experience_bottle" => ProjectileKind::ExperienceBottle,
        item => {
            let lingering = item == "minecraft:lingering_potion";
            ProjectileKind::Potion(Box::new(stack), lingering)
        }
    };
    Projectile::new(kind, position, by_player.then_some(Who::Player))
}

/// `AreaEffectCloud.readAdditionalSaveData`: load the cloud's simulation state and decode
/// its custom particle once. The entity id is the caller's (`load_potion_cloud`).
pub(in crate::game) fn potion_cloud_from_nbt(tag: &Tag) -> Option<(Vec3, Breath)> {
    use sunrise_sim::save::{float, number, text};
    let at = sunrise_sim::entity_nbt::position(tag)?;
    let motion = sunrise_sim::entity_nbt::loaded_motion(tag);
    let int = |key, default| number(tag.get(key)).map_or(default, |v| v as i32);
    let real = |key, default| float(tag.get(key)).map_or(default, |v| v as f32);
    let contents = tag.get("potion_contents");
    let mut effects = contents
        .and_then(|c| text(c.get("potion")))
        .and_then(|id| brewing::potion(&id))
        .map_or_else(Vec::new, |p| p.effects.to_vec());
    let (mut hidden, mut ambient) = (Vec::new(), Vec::new());
    if let Some(Tag::List(custom)) = contents.and_then(|c| c.get("custom_effects")) {
        for effect in custom {
            let Some(def) = text(effect.get("id")).and_then(|id| brewing::effect(&id)) else {
                continue;
            };
            let duration = number(effect.get("duration")).unwrap_or(0) as i32;
            let amp = number(effect.get("amplifier")).unwrap_or(0).clamp(0, 255) as i32;
            effects.push((def.id, duration, amp));
            if number(effect.get("show_particles")) == Some(0) {
                hidden.push(def.id);
            }
            if number(effect.get("ambient")).unwrap_or(0) != 0 {
                ambient.push(def.id);
            }
        }
    }
    let color = contents
        .and_then(|c| number(c.get("custom_color")))
        .map_or_else(
            || {
                sunrise_game_data::potions::effects_color(
                    effects
                        .iter()
                        .filter(|(id, ..)| !hidden.contains(id))
                        .map(|(id, _, amp)| (*id, *amp)),
                )
            },
            |color| color as u32,
        );
    Some((
        at,
        Breath {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 0,
            at: [at.x, at.y, at.z],
            motion: [motion.x, motion.y, motion.z],
            fall_distance: sunrise_sim::entity_nbt::ground_from_nbt(tag).1,
            fire_ticks: sunrise_sim::entity_nbt::fire_ticks_from_nbt(tag),
            first_tick: true,
            radius: real("Radius", 3.0).clamp(0.0, 32.0),
            age: int("Age", 0),
            duration: int("Duration", -1),
            from_dragon: false,
            victims: Vec::new(),
            potion: Some(PotionCloud {
                settings: CloudSettings {
                    wait_time: int("WaitTime", 20),
                    reapplication_delay: int("ReapplicationDelay", 20),
                    radius_on_use: real("RadiusOnUse", 0.0),
                    duration_on_use: int("DurationOnUse", 0),
                    duration_scale: real("potion_duration_scale", 1.0),
                },
                effects,
                hidden,
                ambient,
                radius_per_tick: real("RadiusPerTick", 0.0),
                color,
                custom_particle: tag.get("custom_particle").and_then(|tag| {
                    let options = custom_particle_options(tag)?;
                    let vanilla_tag = options.typed_particle.is_none().then(|| tag.clone());
                    Some(CloudParticle {
                        options,
                        vanilla_tag,
                    })
                }),
                victims: Vec::new(),
                source: Text::translate("entity.minecraft.area_effect_cloud", &[]),
                owner: None,
                guest: None,
            }),
        },
    ))
}

/// `AreaEffectCloud.addAdditionalSaveData`: the lifetime, radius and use
/// settings with the effects the cloud applies, as `custom_effects` with the
/// colour they were drawn in. A cloud from a named potion is written effect by effect;
/// its custom typed particle is encoded by its registered map codec.
pub fn potion_cloud_to_nbt(c: &Breath, uuid: [i32; 4], raw: Option<&Tag>) -> Tag {
    use sunrise_core::nbt::Compound;
    let Some(potion) = &c.potion else {
        return Tag::End;
    };
    let at = Vec3::new(c.at[0], c.at[1], c.at[2]);
    let motion = Vec3::new(c.motion[0], c.motion[1], c.motion[2]);
    // AreaEffectCloud never turns: the rotation it was loaded with.
    let rotation = raw.map_or([0.0; 2], sunrise_sim::entity_nbt::loaded_rotation);
    let mut tag = sunrise_sim::entity_nbt::entity_base_tag(
        "minecraft:area_effect_cloud",
        at,
        motion,
        rotation,
        uuid,
        raw,
    );
    let custom: Vec<Tag> = potion
        .effects
        .iter()
        .map(|(id, duration, amplifier)| {
            let mut effect = Compound::new();
            effect.insert("id".into(), Tag::String(brewing::namespaced(id).into()));
            effect.insert("duration".into(), Tag::Int(*duration));
            effect.insert("amplifier".into(), Tag::Int(*amplifier));
            if potion.hidden.contains(id) {
                effect.insert("show_particles".into(), Tag::Byte(0));
            }
            if potion.ambient.contains(id) {
                effect.insert("ambient".into(), Tag::Byte(1));
            }
            Tag::Compound(effect)
        })
        .collect();
    let mut contents = Compound::new();
    contents.insert("custom_effects".into(), Tag::List(custom));
    contents.insert("custom_color".into(), Tag::Int(potion.color as i32));
    let Tag::Compound(map) = &mut tag else {
        return tag;
    };
    map.insert(
        "fall_distance".into(),
        Tag::Double(if c.fall_distance == 0.0 {
            0
        } else {
            c.fall_distance.to_bits()
        }),
    );
    map.insert("Fire".into(), Tag::Short(c.fire_ticks as i16));
    map.insert("Age".into(), Tag::Int(c.age));
    map.insert("Duration".into(), Tag::Int(c.duration));
    map.insert("Radius".into(), Tag::Float(c.radius.to_bits()));
    map.insert(
        "RadiusPerTick".into(),
        Tag::Float(potion.radius_per_tick.to_bits()),
    );
    // The base tag can carry the original NBT, even if its custom particle did not decode.
    map.remove(&"custom_particle".into());
    if let Some(particle) = &potion.custom_particle {
        let saved = match &particle.options.typed_particle {
            Some((value, _)) => match mod_api::__private::encode_particle_map(value) {
                Ok(fields) => Some(Tag::Compound(fields)),
                Err(error) => {
                    log::warn!(
                        "Cannot save cloud particle {:?}: {error}",
                        particle.options.mod_particle
                    );
                    None
                }
            },
            None => particle.vanilla_tag.clone(),
        };
        if let Some(saved) = saved {
            map.insert("custom_particle".into(), saved);
        }
    }
    map.insert("WaitTime".into(), Tag::Int(potion.settings.wait_time));
    map.insert(
        "ReapplicationDelay".into(),
        Tag::Int(potion.settings.reapplication_delay),
    );
    map.insert(
        "RadiusOnUse".into(),
        Tag::Float(potion.settings.radius_on_use.to_bits()),
    );
    map.insert(
        "DurationOnUse".into(),
        Tag::Int(potion.settings.duration_on_use),
    );
    map.insert(
        "potion_duration_scale".into(),
        Tag::Float(potion.settings.duration_scale.to_bits()),
    );
    // Vanilla writes the key only while the contents differ from
    // PotionContents.EMPTY: no effects and the default water colour.
    if !(potion.effects.is_empty() && potion.color == sunrise_game_data::potions::BASE_POTION_COLOR)
    {
        map.insert("potion_contents".into(), Tag::Compound(contents));
    }
    tag
}

/// `BottleItem.use`: find a living `AreaEffectCloud` owned by an `EnderDragon`
/// within the player's box grown by 2, and shrink its radius by 0.5. Gives
/// the cloud's position when one was found.
pub fn collect_dragon_breath(
    player_box: Aabb,
    clouds: &mut sunrise_sim::join_seq::ResidentList<Breath>,
) -> Option<[f64; 3]> {
    let search_box = player_box.inflate(Vec3::new(2.0, 2.0, 2.0));
    if let Some(i) = clouds.iter().position(|c| {
        if !c.from_dragon
            || c.potion.is_some()
            || sunrise_gameplay_common::game::dragon_common::breath_expired(c.age, c.duration)
        {
            return false;
        }
        let r = f64::from(c.radius);
        let cloud_box = Aabb::new(
            Vec3::new(c.at[0] - r, c.at[1], c.at[2] - r),
            Vec3::new(c.at[0] + r, c.at[1] + 0.5, c.at[2] + r),
        );
        search_box.intersects(cloud_box)
    }) {
        let at = clouds[i].at;
        clouds.update(i, |c| c.radius = (c.radius - 0.5).max(0.0));
        Some(at)
    } else {
        None
    }
}

/// `MobEffect.withSoundOnAdded`: the sound `onEffectAdded` plays when the
/// effect is new on an entity (`MobEffects`: the three omens).
pub(in crate::game) fn sound_on_added(effect: &str) -> Option<&'static str> {
    match effect.strip_prefix("minecraft:").unwrap_or(effect) {
        "bad_omen" => Some("minecraft:event.mob_effect.bad_omen"),
        "trial_omen" => Some("minecraft:event.mob_effect.trial_omen"),
        "raid_omen" => Some("minecraft:event.mob_effect.raid_omen"),
        _ => None,
    }
}

/// Trade the player's own state (effects and what their body was given)
/// with a guest's, around `as_guest`. Potions in flight stay.
pub(in crate::game) fn swap_player(
    potions: &mut PotionState,
    effects: &mut PlayerEffects,
    body: &mut BodyPotions,
) {
    std::mem::swap(&mut potions.effects, effects);
    std::mem::swap(&mut potions.protection_for, &mut body.protection_for);
    std::mem::swap(&mut potions.enchant_modifiers, &mut body.enchant_modifiers);
}

/// Their entity ids, for a server to number them.
pub fn thrown_ids(potions: &mut PotionState) -> impl Iterator<Item = &mut u32> {
    potions.thrown.iter_mut().map(|t| &mut t.id)
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
