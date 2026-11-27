//! Player status effects (`MobEffectInstance` / `LivingEntity.activeEffects`):
//! add/update with hidden effects, per-tick application of the periodic
//! effects (regeneration, poison, wither, hunger, saturation, instant
//! effects), and the derived flags the movement and damage code read.
use crate::particle_options::{ParticleOptions, ParticleType};
use sunrise_game_data::potions;
use sunrise_protocol::entity_data::Value;
use sunrise_world::attributes::Operation;

#[derive(Clone, Debug, PartialEq)]
pub struct EffectInstance {
    /// Effect id: vanilla's without namespace (`speed`, `poison`, ...), a
    /// content mod's `<ns>:<path>`.
    pub id: &'static str,
    /// The registry index, resolved at the instance boundary; unknown internal fixtures have none.
    pub registry_index: Option<usize>,
    /// Remaining ticks; -1 is infinite.
    pub duration: i32,
    pub amplifier: i32,
    pub ambient: bool,
    pub visible: bool,
    /// `MobEffectInstance.showIcon`: whether the HUD and inventory list it.
    pub show_icon: bool,
    pub hidden: Option<Box<EffectInstance>>,
    /// Previous and current client blend factors for effects with no blend
    /// duration. Darkness and nausea use their existing longer fades instead.
    pub client_blend: (bool, bool),
}
impl EffectInstance {
    pub fn new(id: &'static str, duration: i32, amplifier: i32) -> Self {
        Self::resolved(
            id,
            potions::effect(id).map(|def| def.index),
            duration,
            amplifier,
        )
    }
    /// Construct from an already resolved registry entry without another name lookup.
    pub fn from_def(def: &potions::EffectDef, duration: i32, amplifier: i32) -> Self {
        Self::resolved(def.id, Some(def.index), duration, amplifier)
    }
    fn resolved(
        id: &'static str,
        registry_index: Option<usize>,
        duration: i32,
        amplifier: i32,
    ) -> Self {
        Self {
            id,
            registry_index,
            duration,
            amplifier: amplifier.clamp(0, 255),
            ambient: false,
            visible: true,
            show_icon: true,
            hidden: None,
            client_blend: (false, false),
        }
    }
    /// `new MobEffectInstance(effect, duration, amplifier, ambient, visible)`:
    /// the icon shows exactly when the particles do.
    pub fn with_flags(mut self, ambient: bool, visible: bool) -> Self {
        self.ambient = ambient;
        self.visible = visible;
        self.show_icon = visible;
        self
    }
    pub fn infinite(&self) -> bool {
        self.duration == -1
    }
    /// This effect as its entity's swirls are computed from.
    pub fn shown(&self) -> ShownEffect {
        ShownEffect {
            effect: self.id,
            amplifier: self.amplifier,
            visible: self.visible,
            ambient: self.ambient,
        }
    }
    /// `MobEffectInstance.endsWithin`.
    pub fn ends_within(&self, ticks: i32) -> bool {
        !self.infinite() && self.duration <= ticks
    }
    fn shorter_than(&self, other: &Self) -> bool {
        !self.infinite() && (self.duration < other.duration || other.infinite())
    }
    /// `MobEffectInstance.update`.
    pub fn update(&mut self, take: &Self) -> bool {
        let mut changed = false;
        if take.amplifier > self.amplifier {
            if take.shorter_than(self) {
                let previous = self.hidden.take();
                let mut copy = self.clone();
                copy.hidden = previous;
                self.hidden = Some(Box::new(copy));
            }
            self.amplifier = take.amplifier;
            self.duration = take.duration;
            changed = true;
        } else if self.shorter_than(take) {
            if take.amplifier == self.amplifier {
                self.duration = take.duration;
                changed = true;
            } else if let Some(h) = self.hidden.as_mut() {
                h.update(take);
            } else {
                self.hidden = Some(Box::new(EffectInstance {
                    hidden: None,
                    ..take.clone()
                }));
            }
        }
        if !take.ambient && self.ambient || changed {
            self.ambient = take.ambient;
            changed = true;
        }
        if take.visible != self.visible {
            self.visible = take.visible;
            changed = true;
        }
        if take.show_icon != self.show_icon {
            self.show_icon = take.show_icon;
            changed = true;
        }
        changed
    }
    /// `MobEffectInstance.tickClient`: a client counts its effects down
    /// (the server removes them), then advances the zero-duration blend.
    pub fn tick_client(&mut self) {
        if self.infinite() || self.duration > 0 {
            self.tick_down();
            self.downgrade();
        }
        self.client_blend = (self.client_blend.1, !self.ends_within(0));
    }
    /// `MobEffectInstance.tickDownDuration` and `mapDuration`: tick the hidden
    /// chain too, preserving both zero and the infinite sentinel.
    pub fn tick_down(&mut self) {
        if let Some(h) = self.hidden.as_mut() {
            h.tick_down();
        }
        if self.duration != 0 && !self.infinite() {
            self.duration = self.duration.wrapping_sub(1);
        }
    }
    pub fn downgrade(&mut self) -> bool {
        if self.duration == 0
            && let Some(h) = self.hidden.take()
        {
            let h = *h;
            self.duration = h.duration;
            self.amplifier = h.amplifier;
            self.ambient = h.ambient;
            self.visible = h.visible;
            self.show_icon = h.show_icon;
            self.hidden = h.hidden;
            true
        } else {
            false
        }
    }
}
/// What an effect tick asks the game to do to the player.
#[derive(Clone, Debug, PartialEq)]
pub enum EffectAction {
    Heal(f32),
    /// Damage with a damage type id (`minecraft:magic`, `minecraft:wither`).
    Hurt(&'static str, f32),
    /// PoisonMobEffect: one magic damage while above one health.
    Poison,
    Exhaust(f32),
    /// FoodData.eat(nutrition, saturation modifier).
    Eat(i32, f32),
    /// The active instance promoted its hidden effect, or updated in place:
    /// its id, registry index ([`EffectInstance::registry_index`]) and
    /// amplifier.
    Update(&'static str, Option<usize>, i32),
    /// The active instance expired or its tick returned false: its id and
    /// registry index.
    Remove(&'static str, Option<usize>),
}
/// `shouldApplyEffectTickThisTick` + `applyEffectTick` for the periodic effects.
pub fn periodic(id: &str, tick: i32, amp: i32) -> Option<EffectAction> {
    // Java's `>>` and `<<` take the shift distance modulo 32 (`wrapping_*`).
    let every = |base: i32| {
        let interval = base.wrapping_shr(amp as u32);
        interval <= 0 || tick % interval == 0
    };
    match id {
        "regeneration" if every(50) => Some(EffectAction::Heal(1.0)),
        "poison" if every(25) => Some(EffectAction::Poison),
        "wither" if every(40) => Some(EffectAction::Hurt("minecraft:wither", 1.0)),
        "hunger" => Some(EffectAction::Exhaust(0.005 * (amp + 1) as f32)),
        "saturation" if tick >= 1 => Some(EffectAction::Eat(amp + 1, 1.0)),
        "instant_health" if tick >= 1 => Some(EffectAction::Heal(
            4i32.wrapping_shl(amp as u32).max(0) as f32,
        )),
        "instant_damage" if tick >= 1 => Some(EffectAction::Hurt(
            "minecraft:magic",
            6i32.wrapping_shl(amp as u32) as f32,
        )),
        _ => None,
    }
}
/// `HealOrHarmMobEffect.applyInstantaneousEffect` (potions apply these directly;
/// the amount rounds to the nearest whole point).
pub fn instant(id: &str, amp: i32, scale: f64) -> Option<EffectAction> {
    match id {
        "instant_health" => Some(EffectAction::Heal(
            (scale * f64::from(4i32.wrapping_shl(amp as u32)) + 0.5) as i32 as f32,
        )),
        "instant_damage" => Some(EffectAction::Hurt(
            "minecraft:magic",
            (scale * f64::from(6i32.wrapping_shl(amp as u32)) + 0.5) as i32 as f32,
        )),
        "saturation" => Some(EffectAction::Eat(amp + 1, 1.0)),
        _ => None,
    }
}
/// `MobEffectInstance.withScaledDuration`: max(floor(d * scale), 1), leaving
/// an infinite (-1) or zero duration alone.
pub fn scaled_duration(duration: i32, scale: f32) -> i32 {
    if duration == -1 || duration == 0 {
        duration
    } else {
        ((duration as f32 * scale).floor() as i32).max(1)
    }
}
/// `MobEffect.particleFactory`: the omens and the four effects `MobEffects`
/// registers with a particle of their own (Wind Charged `SMALL_GUST`,
/// Weaving `ITEM_COBWEB`, Oozing `ITEM_SLIME`, Infested `INFESTED`) show
/// that one; any other effect an entity effect in its colour, faint when
/// ambient.
pub fn effect_particle(effect: &str, ambient: bool) -> Option<ParticleOptions> {
    Some(match effect.trim_start_matches("minecraft:") {
        "trial_omen" => ParticleOptions::simple(ParticleType::TrialOmen),
        "raid_omen" => ParticleOptions::simple(ParticleType::RaidOmen),
        "wind_charged" => ParticleOptions::simple(ParticleType::SmallGust),
        "weaving" => ParticleOptions::simple(ParticleType::ItemCobweb),
        "oozing" => ParticleOptions::simple(ParticleType::ItemSlime),
        "infested" => ParticleOptions::simple(ParticleType::Infested),
        other => {
            let color = potions::effect(other)?.color;
            let channel = |shift: u32| ((color >> shift) & 255) as f32 / 255.0;
            // MobEffect.AMBIENT_ALPHA is 38 of 255.
            let alpha = if ambient { 38.0 / 255.0 } else { 1.0 };
            ParticleOptions::colored(
                ParticleType::EntityEffect,
                [channel(16), channel(8), channel(0), alpha],
            )
        }
    })
}
/// An active effect as `LivingEntity.updateSynchronizedMobEffectParticles`
/// sees it; vanilla recomputes the particles when one of these changes
/// (`onEffectAdded`, `onEffectUpdated`, `onEffectsRemoved`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShownEffect {
    pub effect: &'static str,
    pub amplifier: i32,
    pub visible: bool,
    pub ambient: bool,
}
/// `LivingEntity.updateSynchronizedMobEffectParticles` for an entity with
/// `effects`: the particle of each visible one (`effect_particle`) and
/// `areAllEffectsAmbient` (true when none is visible).
pub fn effect_particles(
    effects: impl IntoIterator<Item = ShownEffect>,
) -> (Vec<ParticleOptions>, bool) {
    visible_effect_particles(
        (effects.into_iter())
            .filter(|e| e.visible)
            .map(|e| (e.ambient, effect_particle(e.effect, e.ambient))),
    )
}
/// The swirls of an entity's `visible` effects, each as whether it is ambient
/// and its particle, if it has one: the particles and
/// `areAllEffectsAmbient` (true when none is visible).
pub fn visible_effect_particles(
    visible: impl IntoIterator<Item = (bool, Option<ParticleOptions>)>,
) -> (Vec<ParticleOptions>, bool) {
    let mut all_ambient = true;
    let particles = (visible.into_iter())
        .filter_map(|(ambient, particle)| {
            all_ambient &= ambient;
            particle
        })
        .collect();
    (particles, all_ambient)
}
/// A living entity's `DATA_EFFECT_PARTICLES` and `DATA_EFFECT_AMBIENCE_ID`
/// as the native mods left them (`effect_particle_modification`), with the
/// effects they were computed for: sent until those effects change and the
/// mods are asked again, as vanilla recomputes them only then.
#[derive(Clone, Debug, PartialEq)]
pub struct ModifiedEffectParticles {
    effects: Vec<ShownEffect>,
    /// The entries sent, each left out at its default.
    pub meta: Vec<(u8, i32, Value)>,
}
impl ModifiedEffectParticles {
    pub fn new(
        effects: impl IntoIterator<Item = ShownEffect>,
        meta: Vec<(u8, i32, Value)>,
    ) -> Self {
        Self {
            effects: effects.into_iter().collect(),
            meta,
        }
    }
    /// Whether these were computed for `effects`.
    pub fn computed_for(&self, effects: impl IntoIterator<Item = ShownEffect>) -> bool {
        self.effects.iter().copied().eq(effects)
    }
}
/// What an omen's `applyEffectTick` decided.
#[derive(Clone, Debug, PartialEq)]
pub enum OmenTick {
    /// It returned true: the effect counts down as usual.
    Continue,
    /// It returned false: the effect goes.
    Remove,
    /// It added this effect, then returned false.
    Add(EffectInstance),
}
/// A saved player's effect: `MobEffectInstance.Details` under its `id`.
#[derive(Clone, Debug, PartialEq)]
pub struct SavedEffect {
    pub id: String,
    pub duration: i32,
    pub amplifier: i32,
    pub ambient: bool,
    /// `show_particles`.
    pub particles: bool,
    /// `show_icon`.
    pub icon: bool,
    /// `hidden_effect`: the weaker, longer instance of the same effect that
    /// comes back when this one ends.
    pub hidden: Option<Box<SavedEffect>>,
}
impl SavedEffect {
    /// Ambient off, particles and icon shown, nothing hidden: the codec's
    /// defaults.
    pub fn new(id: impl Into<String>, duration: i32, amplifier: i32) -> Self {
        Self {
            id: id.into(),
            duration,
            amplifier,
            ambient: false,
            particles: true,
            icon: true,
            hidden: None,
        }
    }
    fn of(e: &EffectInstance) -> Self {
        Self {
            id: potions::namespaced(e.id).to_owned(),
            duration: e.duration,
            amplifier: e.amplifier,
            ambient: e.ambient,
            particles: e.visible,
            icon: e.show_icon,
            hidden: e.hidden.as_deref().map(|h| Box::new(Self::of(h))),
        }
    }
    /// `new MobEffectInstance(effect, details)`: the hidden chain comes back
    /// with it.
    fn instance(&self, id: &'static str) -> EffectInstance {
        let mut e = EffectInstance::new(id, self.duration, self.amplifier);
        e.ambient = self.ambient;
        e.visible = self.particles;
        e.show_icon = self.icon;
        e.hidden = self.hidden.as_deref().map(|h| Box::new(h.instance(id)));
        e
    }
}
#[derive(Clone, Debug, Default)]
pub struct PlayerEffects {
    pub list: Vec<EffectInstance>,
    pub tick_count: i32,
    /// How many times `add` added or updated an effect: each is one
    /// `onEffectAdded` or `onEffectUpdated` (`EffectsChangedTrigger`).
    pub changes: u32,
}
/// Copyable metadata of the active effect currently being ticked, so the pass
/// never clones `EffectInstance` (or its boxed hidden-effect chain) while
/// callbacks run against the live host.
#[derive(Clone, Copy, Debug)]
pub struct TickingEffect {
    pub id: &'static str,
    pub registry_index: Option<usize>,
    pub duration: i32,
    pub amplifier: i32,
}

impl TickingEffect {
    pub fn infinite(self) -> bool {
        self.duration == -1
    }

    fn from_instance(e: &EffectInstance) -> Self {
        Self {
            id: e.id,
            registry_index: e.registry_index,
            duration: e.duration,
            amplifier: e.amplifier,
        }
    }
}

/// Host callbacks for [`PlayerEffects::tick_on`], allowing the single
/// `LivingEntity.tickEffects` pass to keep the active-effect map live on its
/// owner without cloning `EffectInstance`s.
pub trait EffectTickHost {
    fn effects(&self) -> &PlayerEffects;
    fn effects_mut(&mut self) -> &mut PlayerEffects;
    fn apply_action(&mut self, action: EffectAction);
    fn tick_special(&mut self, effect: TickingEffect, tick_count: i32) -> OmenTick;
    /// Add `effect` returned by [`OmenTick::Add`], returning `true` when a new
    /// entry was inserted into `effects().list` (vanilla's CME stop condition).
    fn add_effect(&mut self, effect: EffectInstance) -> bool;
    fn on_promoted(&mut self, id: &'static str, registry_index: Option<usize>, amplifier: i32);
    fn on_removed(&mut self, id: &'static str, registry_index: Option<usize>);
    /// Ask `MobEffectEvent.Expired` when an effect has no remaining duration.
    /// Returning `true` keeps the effect at 0 ticks for the next pass.
    fn keep_expired(&self, effect: TickingEffect) -> bool;
}

fn locate_effect(list: &[EffectInstance], hint: usize, id: &'static str) -> Option<usize> {
    if hint < list.len() && list[hint].id == id {
        Some(hint)
    } else {
        list.iter().position(|cur| cur.id == id)
    }
}

impl PlayerEffects {
    pub fn get(&self, id: &str) -> Option<&EffectInstance> {
        self.list.iter().find(|e| e.id == id)
    }
    /// Read by the registry identity already resolved at creation, without comparing names.
    pub fn get_registered(&self, index: usize) -> Option<&EffectInstance> {
        self.list
            .iter()
            .find(|effect| effect.registry_index == Some(index))
    }

    pub fn amplifier(&self, id: &str) -> Option<i32> {
        self.get(id).map(|e| e.amplifier)
    }
    /// `LivingEntity.addEffect`; returns whether anything changed.
    pub fn add(&mut self, effect: EffectInstance) -> bool {
        let changed = match self.list.iter_mut().find(|e| e.id == effect.id) {
            Some(existing) => existing.update(&effect),
            None => {
                self.list.push(effect);
                true
            }
        };
        if changed {
            self.changes = self.changes.wrapping_add(1);
        }
        changed
    }
    pub fn clear(&mut self) {
        self.list.clear();
    }
    /// The effects of a saved player (`Player.active_effects`), each with
    /// the instances it hides.
    pub fn from_saved(saved: &[SavedEffect]) -> Self {
        let mut effects = Self::default();
        for s in saved {
            if let Some(def) = potions::effect(&s.id) {
                effects.add(s.instance(def.id));
            }
        }
        effects
    }
    /// `Player.active_effects` for the save.
    pub fn saved(&self) -> Vec<SavedEffect> {
        self.list.iter().map(SavedEffect::of).collect()
    }
    /// ClientboundUpdateMobEffectPacket's flags without blending: ambient,
    /// particles visible and the icon shown.
    pub fn flags(e: &EffectInstance) -> i8 {
        i8::from(e.ambient) | i8::from(e.visible) << 1 | i8::from(e.show_icon) << 2
    }
    /// `LivingEntity.tickEffects` (server part) running directly against a host
    /// that owns [`PlayerEffects`], keeping the active-effect list live during
    /// callbacks without cloning any [`EffectInstance`].
    ///
    /// Bad Omen turning into a new Raid Omen adds an entry to the map being
    /// iterated: vanilla's iterator then throws a
    /// `ConcurrentModificationException`, which `tickEffects` swallows. Bad
    /// Omen stays (not counted down) and the effects after it are not
    /// ticked; the next tick refreshes Raid Omen and drops Bad Omen.
    pub fn tick_on<H: EffectTickHost>(host: &mut H) {
        let tick_count = {
            let fx = host.effects_mut();
            fx.tick_count = fx.tick_count.wrapping_add(1);
            fx.tick_count
        };
        let mut i = 0;
        while i < host.effects().list.len() {
            let eff = TickingEffect::from_instance(&host.effects().list[i]);
            if !(eff.infinite() || eff.duration > 0) {
                if host.keep_expired(eff) {
                    i += 1;
                } else {
                    let removed = host.effects_mut().list.remove(i);
                    host.on_removed(removed.id, removed.registry_index);
                }
                continue;
            }
            let t = if eff.infinite() {
                tick_count
            } else {
                eff.duration
            };
            if let Some(a) = periodic(eff.id, t, eff.amplifier) {
                host.apply_action(a);
            }
            let answer = match eff.id {
                "bad_omen" | "absorption" => host.tick_special(eff, t),
                "raid_omen" if t == 1 => host.tick_special(eff, t),
                id if potions::is_mod_effect(id) => host.tick_special(eff, t),
                _ => OmenTick::Continue,
            };
            match answer {
                OmenTick::Continue => {}
                OmenTick::Remove => {
                    if let Some(pos) = locate_effect(&host.effects().list, i, eff.id) {
                        let removed = host.effects_mut().list.remove(pos);
                        host.on_removed(removed.id, removed.registry_index);
                        if pos < i {
                            i -= 1;
                        }
                    }
                    continue;
                }
                OmenTick::Add(effect) => {
                    // `LivingEntity.tickEffects` + `BadOmenMobEffect.applyEffectTick`:
                    // inserting a new entry structurally modifies `activeEffects`, so
                    // `iterator.remove()` throws `ConcurrentModificationException`,
                    // leaving `bad_omen` for the next tick and ending this pass.
                    // Updating an existing entry (or a denied add) does not structurally
                    // modify the map, so `bad_omen` is removed and the pass continues.
                    if host.add_effect(effect) {
                        break;
                    }
                    if let Some(pos) = locate_effect(&host.effects().list, i, eff.id) {
                        let removed = host.effects_mut().list.remove(pos);
                        host.on_removed(removed.id, removed.registry_index);
                        if pos < i {
                            i -= 1;
                        }
                    }
                    continue;
                }
            }
            let Some(pos) = locate_effect(&host.effects().list, i, eff.id) else {
                continue;
            };
            let (promoted, expired) = {
                let cur = &mut host.effects_mut().list[pos];
                cur.tick_down();
                let promoted =
                    cur.downgrade()
                        .then_some((cur.id, cur.registry_index, cur.amplifier));
                let expired = (!(cur.infinite() || cur.duration > 0))
                    .then(|| TickingEffect::from_instance(cur));
                (promoted, expired)
            };
            if let Some((id, registry_index, amplifier)) = promoted {
                host.on_promoted(id, registry_index, amplifier);
            }
            if let Some(expired_eff) = expired
                && !host.keep_expired(expired_eff)
            {
                let removed = host.effects_mut().list.remove(pos);
                host.on_removed(removed.id, removed.registry_index);
                i = pos;
            } else {
                i = pos + 1;
            }
        }
    }
    /// `LivingEntity.tickEffects` (server part): actions to apply, in order.
    pub fn tick(&mut self) -> Vec<EffectAction> {
        self.tick_count = self.tick_count.wrapping_add(1);
        let mut actions = Vec::new();
        let tick_count = self.tick_count;
        self.list.retain_mut(|e| {
            if !(e.infinite() || e.duration > 0) {
                return false;
            }
            let t = if e.infinite() { tick_count } else { e.duration };
            if let Some(a) = periodic(e.id, t, e.amplifier) {
                actions.push(a);
            }
            e.tick_down();
            e.downgrade();
            e.infinite() || e.duration > 0
        });
        actions
    }
    /// Effects applied by drinking a potion (`PotionContents.applyToLivingEntity`).
    pub fn potion_effects(
        potion: Option<&str>,
        scale: f32,
    ) -> (Vec<EffectInstance>, Vec<EffectAction>) {
        let effects = potion
            .and_then(potions::potion)
            .map_or(&[][..], |d| d.effects);
        Self::contents_effects(effects, scale)
    }
    /// `PotionContents.forEachEffect`'s effects, as `potion_effects`.
    pub fn contents_effects(
        effects: &[(&'static str, i32, i32)],
        scale: f32,
    ) -> (Vec<EffectInstance>, Vec<EffectAction>) {
        let mut lasting = Vec::new();
        let mut now = Vec::new();
        for (id, duration, amp) in effects {
            let id = potions::effect(id).map_or("speed", |e| e.id);
            if potions::effect(id).is_some_and(|e| e.instant) {
                now.extend(instant(id, *amp, 1.0));
            } else {
                lasting.push(EffectInstance::new(
                    id,
                    scaled_duration(*duration, scale),
                    *amp,
                ));
            }
        }
        (lasting, now)
    }
    /// Flags for movement and damage code.
    pub fn status(&self) -> StatusFlags {
        StatusFlags {
            jump_boost: self.amplifier("jump_boost"),
            slow_falling: self.get("slow_falling").is_some(),
            levitation: self.amplifier("levitation"),
            resistance: self.amplifier("resistance"),
            fire_resistance: self.get("fire_resistance").is_some(),
            dolphins_grace: self.get("dolphins_grace").is_some(),
            weaving: self.get("weaving").is_some(),
            infested: self.get("infested").is_some(),
            water_breathing: self.get("water_breathing").is_some()
                || self.get("conduit_power").is_some(),
            breath_of_the_nautilus: self.get("breath_of_the_nautilus").is_some(),
            protection: Vec::new(),
        }
    }
    /// Attribute modifiers of active effects: (attribute, id, amount, operation).
    pub fn modifiers(&self) -> Vec<EffectModifier> {
        let mut out = Vec::new();
        for e in &self.list {
            let def = e
                .registry_index
                .and_then(potions::effect_by_index)
                .or_else(|| potions::effect(e.id));
            for (a, id, amount, op) in def.into_iter().flat_map(effect_modifiers) {
                out.push((a, id, amount * f64::from(e.amplifier.wrapping_add(1)), op));
            }
        }
        out
    }
    /// `Player.getDestroySpeed` effect factors (haste/conduit, mining fatigue).
    pub fn mining_speed_factor(&self) -> f32 {
        let mut f = 1.0;
        let haste = self.amplifier("haste").max(self.amplifier("conduit_power"));
        if let Some(a) = haste {
            f *= 1.0 + (a + 1) as f32 * 0.2;
        }
        if let Some(a) = self.amplifier("mining_fatigue") {
            f *= sunrise_core::jmath::pow(0.3, f64::from(a + 1)) as f32;
        }
        f
    }
}
/// Effect and equipment facts the movement and damage kernels query.
#[derive(Clone, Debug, Default)]
pub struct StatusFlags {
    pub jump_boost: Option<i32>,
    pub slow_falling: bool,
    pub levitation: Option<i32>,
    pub resistance: Option<i32>,
    pub fire_resistance: bool,
    pub dolphins_grace: bool,
    /// Weaving: cobwebs hold half as much (`WebBlock.entityInside`).
    pub weaving: bool,
    /// Infested: a hit that is not blocked rolls `InfestedMobEffect.onMobHurt`.
    pub infested: bool,
    pub water_breathing: bool,
    pub breath_of_the_nautilus: bool,
    /// Armor enchantment protection per damage type id (`EnchantmentHelper.getDamageProtection`).
    pub protection: Vec<(String, f32)>,
}
impl StatusFlags {
    pub fn protection(&self, damage_type: &str) -> f32 {
        self.protection
            .iter()
            .find(|(t, _)| *t == damage_type)
            .map_or(0.0, |(_, p)| *p)
    }
}

pub use potions::EffectModifier;

/// The attribute modifiers an effect adds at amplifier 0
/// (`MobEffect.addAttributeModifier`): (attribute, id, amount, operation),
/// vanilla's or those a content mod declared for its effect. Each level
/// multiplies the amount (`amplifier + 1`).
pub fn effect_modifiers(effect: &potions::EffectDef) -> impl Iterator<Item = EffectModifier> {
    effect_modifier(effect.id)
        .into_iter()
        .chain(effect.modifiers.iter().copied())
}

/// The attribute modifier a vanilla effect adds at amplifier 0. Vanilla's
/// amounts are `float` literals widened to the modifier's `double`, so
/// `f64::from` the `f32` to keep the low digits.
fn effect_modifier(effect: &str) -> Option<EffectModifier> {
    match effect {
        "speed" => Some((
            "minecraft:movement_speed",
            "minecraft:effect.speed",
            f64::from(0.2f32),
            Operation::AddMultipliedTotal,
        )),
        "slowness" => Some((
            "minecraft:movement_speed",
            "minecraft:effect.slowness",
            f64::from(-0.15f32),
            Operation::AddMultipliedTotal,
        )),
        "haste" => Some((
            "minecraft:attack_speed",
            "minecraft:effect.haste",
            f64::from(0.1f32),
            Operation::AddMultipliedTotal,
        )),
        "mining_fatigue" => Some((
            "minecraft:attack_speed",
            "minecraft:effect.mining_fatigue",
            f64::from(-0.1f32),
            Operation::AddMultipliedTotal,
        )),
        "strength" => Some((
            "minecraft:attack_damage",
            "minecraft:effect.strength",
            3.0,
            Operation::AddValue,
        )),
        "weakness" => Some((
            "minecraft:attack_damage",
            "minecraft:effect.weakness",
            -4.0,
            Operation::AddValue,
        )),
        "jump_boost" => Some((
            "minecraft:safe_fall_distance",
            "minecraft:effect.jump_boost",
            1.0,
            Operation::AddValue,
        )),
        "health_boost" => Some((
            "minecraft:max_health",
            "minecraft:effect.health_boost",
            4.0,
            Operation::AddValue,
        )),
        "absorption" => Some((
            "minecraft:max_absorption",
            "minecraft:effect.absorption",
            4.0,
            Operation::AddValue,
        )),
        "invisibility" => Some((
            "minecraft:waypoint_transmit_range",
            "minecraft:effect.waypoint_transmit_range_hide",
            -1.0,
            Operation::AddMultipliedTotal,
        )),
        "luck" => Some((
            "minecraft:luck",
            "minecraft:effect.luck",
            1.0,
            Operation::AddValue,
        )),
        "unluck" => Some((
            "minecraft:luck",
            "minecraft:effect.unluck",
            -1.0,
            Operation::AddValue,
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// `HealOrHarmMobEffect`: Java's `<<` shifts by the amplifier modulo
    /// 32 and wraps, so amplifier 32 is amplifier 0 again; a ticked heal is
    /// at least 0 (#11386).
    #[test]
    fn heal_and_harm_shift_like_java() {
        let magic = "minecraft:magic";
        assert_eq!(
            instant("instant_health", 32, 1.0),
            Some(EffectAction::Heal(4.0))
        );
        assert_eq!(
            instant("instant_damage", 33, 1.0),
            Some(EffectAction::Hurt(magic, 12.0))
        );
        assert_eq!(
            instant("instant_health", 1, 0.5),
            Some(EffectAction::Heal(4.0))
        );
        assert_eq!(
            periodic("instant_health", 1, 32),
            Some(EffectAction::Heal(4.0))
        );
        // 4 << 29 is i32::MIN: the heal stops at 0. 6 << 30 wraps to i32::MIN.
        assert_eq!(
            periodic("instant_health", 1, 29),
            Some(EffectAction::Heal(0.0))
        );
        assert_eq!(
            periodic("instant_damage", 1, 30),
            Some(EffectAction::Hurt(magic, i32::MIN as f32))
        );
        // Regeneration: 50 >> 32 is 50 again, not 0 (which would heal each tick).
        assert_eq!(periodic("regeneration", 49, 32), None);
        assert_eq!(
            periodic("regeneration", 50, 32),
            Some(EffectAction::Heal(1.0))
        );
    }

    #[test]
    fn adding_counts_a_change_only_when_the_effect_changed() {
        let mut fx = PlayerEffects::default();
        fx.add(EffectInstance::new("speed", 100, 0));
        assert_eq!(fx.changes, 1);
        // A longer duration updates the effect (`onEffectUpdated`).
        fx.add(EffectInstance::new("speed", 200, 0));
        assert_eq!(fx.changes, 2);
        // A shorter one with the same amplifier is hidden by nothing: no change.
        fx.add(EffectInstance::new("speed", 50, 0));
        assert_eq!(fx.changes, 2);
    }

    struct TestTickHost<'a, F> {
        fx: &'a mut PlayerEffects,
        special: F,
        actions: Vec<EffectAction>,
        added: bool,
    }

    impl<F> EffectTickHost for TestTickHost<'_, F>
    where
        F: FnMut(TickingEffect, i32, &mut Vec<EffectAction>) -> OmenTick,
    {
        fn effects(&self) -> &PlayerEffects {
            self.fx
        }
        fn effects_mut(&mut self) -> &mut PlayerEffects {
            self.fx
        }
        fn apply_action(&mut self, action: EffectAction) {
            self.actions.push(action);
        }
        fn tick_special(&mut self, effect: TickingEffect, tick_count: i32) -> OmenTick {
            (self.special)(effect, tick_count, &mut self.actions)
        }
        fn add_effect(&mut self, effect: EffectInstance) -> bool {
            if let Some(existing) = self.fx.list.iter_mut().find(|x| x.id == effect.id) {
                if existing.update(&effect) {
                    self.actions.push(EffectAction::Update(
                        existing.id,
                        existing.registry_index,
                        existing.amplifier,
                    ));
                }
                return false;
            }
            self.fx.list.push(effect);
            self.added = true;
            true
        }
        fn on_promoted(&mut self, id: &'static str, registry_index: Option<usize>, amplifier: i32) {
            self.actions
                .push(EffectAction::Update(id, registry_index, amplifier));
        }
        fn on_removed(&mut self, id: &'static str, registry_index: Option<usize>) {
            self.actions.push(EffectAction::Remove(id, registry_index));
        }
        fn keep_expired(&self, _: TickingEffect) -> bool {
            false
        }
    }

    fn run_tick_on(
        fx: &mut PlayerEffects,
        special: impl FnMut(TickingEffect, i32, &mut Vec<EffectAction>) -> OmenTick,
    ) -> (Vec<EffectAction>, bool) {
        let mut host = TestTickHost {
            fx,
            special,
            actions: Vec::new(),
            added: false,
        };
        PlayerEffects::tick_on(&mut host);
        (host.actions, host.added)
    }

    #[test]
    fn bad_omen_in_a_village_becomes_raid_omen_over_two_ticks() {
        let mut fx = PlayerEffects::default();
        fx.add(EffectInstance::new("bad_omen", 100, 1));
        fx.add(EffectInstance::new("speed", 50, 0));
        let mut convert = |e: TickingEffect, _: i32, _: &mut Vec<EffectAction>| match e.id {
            "bad_omen" => OmenTick::Add(EffectInstance::new("raid_omen", 600, e.amplifier)),
            _ => OmenTick::Continue,
        };
        // The first tick adds Raid Omen and stops there (the iterator's
        // ConcurrentModificationException): Bad Omen and Speed stay as they were.
        let (_, added) = run_tick_on(&mut fx, &mut convert);
        assert!(added);
        assert_eq!(fx.get("bad_omen").unwrap().duration, 100);
        assert_eq!(fx.get("speed").unwrap().duration, 50);
        assert_eq!(fx.get("raid_omen").unwrap().duration, 600);
        // The second refreshes Raid Omen and drops Bad Omen.
        let (_, added) = run_tick_on(&mut fx, &mut convert);
        assert!(!added);
        assert!(fx.get("bad_omen").is_none());
        assert_eq!(fx.get("speed").unwrap().duration, 49);
        assert_eq!(fx.get("raid_omen").unwrap().amplifier, 1);
    }
    #[test]
    fn a_reloaded_effect_falls_back_to_the_one_it_hid() {
        // /effect give poison 6 s level II over poison 30 s level I, saved
        // and loaded: when level II runs out, level I resumes with what is
        // left of its 30 s.
        let mut fx = PlayerEffects::default();
        fx.add(EffectInstance::new("poison", 600, 0));
        fx.add(EffectInstance::new("poison", 120, 1));
        let mut loaded = PlayerEffects::from_saved(&fx.saved());
        for _ in 0..120 {
            loaded.tick();
        }
        let poison = loaded.get("poison").expect("level I resumed");
        assert_eq!((poison.amplifier, poison.duration), (0, 480));
    }
    /// `ruby:glow`, registered in this thread's own scope until the guard drops.
    fn register_ruby_glow() -> impl Drop {
        let scope = potions::isolate_mod_effects();
        assert!(potions::register_mod_effects([potions::ModEffect {
            modifiers: vec![(
                "minecraft:movement_speed".into(),
                "ruby:effect/glow".into(),
                -0.25,
                Operation::AddMultipliedTotal,
            )],
            ..potions::ModEffect::new("ruby:glow", 0xFF0000, potions::Category::Neutral, "ruby")
        }]));
        scope
    }
    #[test]
    fn a_mod_effect_saves_under_its_namespace() {
        let _scope = register_ruby_glow();
        let mut fx = PlayerEffects::default();
        fx.add(EffectInstance::new(
            potions::effect("ruby:glow").unwrap().id,
            600,
            1,
        ));
        let saved = fx.saved();
        assert_eq!(saved, [SavedEffect::new("ruby:glow", 600, 1)]);
        assert_eq!(PlayerEffects::from_saved(&saved).list, fx.list);
        let mut ordered = PlayerEffects::default();
        ordered.add(EffectInstance::new("regeneration", 50, 0));
        ordered.add(EffectInstance::new("ruby:glow", 10, 0));
        ordered.add(EffectInstance::new("wither", 40, 0));
        let (actions, _) = run_tick_on(&mut ordered, |_, _, actions| {
            actions.push(EffectAction::Eat(2, 0.5));
            OmenTick::Continue
        });
        assert_eq!(
            actions,
            [
                EffectAction::Heal(1.0),
                EffectAction::Eat(2, 0.5),
                EffectAction::Hurt("minecraft:wither", 1.0)
            ]
        );
        // Without its mod the saved effect is dropped, as an unknown id is.
        assert!(
            PlayerEffects::from_saved(&[SavedEffect::new("gone:glow", 600, 1)])
                .list
                .is_empty()
        );
    }
    /// A content mod's effect answers each tick before it counts down (an
    /// infinite one with the tick count), and goes at once on Remove.
    #[test]
    fn a_mod_effect_answers_before_it_counts_down() {
        let mut fx = PlayerEffects::default();
        fx.add(EffectInstance::new("ruby:glow", 3, 1));
        fx.add(EffectInstance::new("ruby:bleed", -1, 0));
        let mut seen = Vec::new();
        let mut answer = |e: TickingEffect, t: i32, _: &mut Vec<EffectAction>| {
            seen.push((e.id, t));
            match (e.id, t) {
                ("ruby:glow", 2) => OmenTick::Remove,
                _ => OmenTick::Continue,
            }
        };
        run_tick_on(&mut fx, &mut answer);
        run_tick_on(&mut fx, &mut answer);
        let ticks = [
            ("ruby:glow", 3),
            ("ruby:bleed", 1),
            ("ruby:glow", 2),
            ("ruby:bleed", 2),
        ];
        assert_eq!(seen, ticks);
        assert!(fx.get("ruby:glow").is_none());
        assert_eq!(fx.get("ruby:bleed").unwrap().duration, -1);
    }
    #[test]
    fn raid_omen_acts_on_its_last_tick() {
        let mut fx = PlayerEffects::default();
        fx.add(EffectInstance::new("raid_omen", 2, 0));
        let mut seen = Vec::new();
        let mut watch = |e: TickingEffect, _: i32, _: &mut Vec<EffectAction>| {
            seen.push(e.duration);
            OmenTick::Remove
        };
        run_tick_on(&mut fx, &mut watch);
        assert!(fx.get("raid_omen").is_some());
        run_tick_on(&mut fx, &mut watch);
        assert!(fx.get("raid_omen").is_none());
        assert_eq!(seen, vec![1]);
    }

    #[test]
    fn update_keeps_stronger_and_hides_longer() {
        let mut fx = PlayerEffects::default();
        fx.add(EffectInstance::new("speed", 9600, 0));
        // A stronger but shorter effect takes over and hides the long one.
        fx.add(EffectInstance::new("speed", 1800, 1));
        let e = fx.get("speed").unwrap();
        assert_eq!((e.amplifier, e.duration), (1, 1800));
        assert_eq!(
            e.hidden.as_ref().map(|h| (h.amplifier, h.duration)),
            Some((0, 9600))
        );
        for _ in 0..1800 {
            fx.tick();
        }
        let e = fx.get("speed").unwrap();
        assert_eq!((e.amplifier, e.duration), (0, 7800));
        // Weaker and shorter: ignored.
        assert!(!fx.add(EffectInstance::new("speed", 100, 0)));
    }

    #[test]
    fn regeneration_and_poison_intervals() {
        let mut fx = PlayerEffects::default();
        fx.add(EffectInstance::new("regeneration", 900, 0));
        let heals = (0..900)
            .flat_map(|_| fx.tick())
            .filter(|a| *a == EffectAction::Heal(1.0))
            .count();
        // Duration 900..1 divisible by 50: 18 heals.
        assert_eq!(heals, 18);
        assert!(fx.get("regeneration").is_none());
        let mut fx = PlayerEffects::default();
        fx.add(EffectInstance::new("poison", 432, 1));
        let hurts = (0..432)
            .flat_map(|_| fx.tick())
            .filter(|a| matches!(a, EffectAction::Poison))
            .count();
        // Interval 25 >> 1 = 12: 432/12 = 36 hits.
        assert_eq!(hurts, 36);
    }

    #[test]
    fn potions_split_instant_and_lasting() {
        let (lasting, now) = PlayerEffects::potion_effects(Some("minecraft:strong_healing"), 1.0);
        assert!(lasting.is_empty());
        assert_eq!(now, vec![EffectAction::Heal(8.0)]);
        let (lasting, _) = PlayerEffects::potion_effects(Some("minecraft:long_swiftness"), 0.25);
        assert_eq!((lasting[0].id, lasting[0].duration), ("speed", 2400));
        let (lasting, _) = PlayerEffects::potion_effects(Some("minecraft:turtle_master"), 1.0);
        assert_eq!(lasting.len(), 2);
        let mut fx = PlayerEffects::default();
        fx.add(EffectInstance::new("haste", 100, 1));
        assert!((fx.mining_speed_factor() - 1.4).abs() < 1e-6);
        assert_eq!(fx.modifiers().len(), 1);
    }

    /// #4473: a content mod's effect adds the modifiers its declaration
    /// lists, each amount times the amplifier plus one, as vanilla's do.
    #[test]
    fn a_mod_effects_declared_modifier_scales_with_amplifier() {
        let _scope = register_ruby_glow();
        let mut fx = PlayerEffects::default();
        fx.add(EffectInstance::new("ruby:glow", 2400, 0));
        fx.add(EffectInstance::new("speed", 2400, 0));
        let glow = |amount| {
            (
                "minecraft:movement_speed",
                "ruby:effect/glow",
                amount,
                Operation::AddMultipliedTotal,
            )
        };
        let speed = (
            "minecraft:movement_speed",
            "minecraft:effect.speed",
            f64::from(0.2f32),
            Operation::AddMultipliedTotal,
        );
        assert_eq!(fx.modifiers(), [glow(-0.25), speed]);
        fx.add(EffectInstance::new("ruby:glow", 2400, 2));
        assert_eq!(fx.modifiers(), [glow(-0.75), speed]);
    }

    #[test]
    fn absorption_modifier_scales_with_amplifier() {
        let mut fx = PlayerEffects::default();
        fx.add(EffectInstance::new("absorption", 2400, 0));
        assert_eq!(
            fx.modifiers(),
            vec![(
                "minecraft:max_absorption",
                "minecraft:effect.absorption",
                4.0,
                Operation::AddValue,
            )]
        );
        fx.add(EffectInstance::new("absorption", 2400, 3));
        assert_eq!(
            fx.modifiers(),
            vec![(
                "minecraft:max_absorption",
                "minecraft:effect.absorption",
                16.0,
                Operation::AddValue,
            )]
        );
    }
}
