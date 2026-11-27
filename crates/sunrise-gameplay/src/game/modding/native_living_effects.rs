//! A living entity's active effects for native server mods, through a
//! `LivingRef` (docs/modding/src/how-to/native-mods.md, "Entity and level handles"): vanilla's
//! `activeEffects` read whole, and `addEffect`, `removeEffect`,
//! `removeAllEffects` and `hasEffect` by the game's own paths, so an effect
//! a mod gives lands as a potion's or `/effect`'s does, through
//! `canBeAffected` and the added-effect hooks. Reads return `None`, writes
//! false, for a stale handle or on a client; armor stands and the dragon
//! keep no effects, as `/effect` skips them. `effect` and `effects` also read
//! on a client, for the local player, from the effects the server sent it, and
//! `effect_blend_factor` reads only there, from the fades the client ticks.
use super::commands::Target;
use crate::game::ServerGame;
use mod_api::{ActiveEffect, EffectKey, LivingRef};
use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;
use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;
use sunrise_items::items::ItemStack;
use sunrise_level::effects::EffectInstance;

impl ServerGame<'_> {
    /// Apply one loaded consumable effect without consuming the item.
    pub fn living_apply_consume_effect(
        &mut self,
        living: LivingRef,
        stack: &ItemStack,
        index: usize,
    ) -> Option<bool> {
        let target = self.native_living_target(living)?;
        let consumable = self.consumable(stack)?;
        let effect = consumable.effects.get(index)?;
        match target {
            t @ (Target::Player | Target::Guest(_)) => self
                .for_player(t, |g| {
                    g.apply_consume_effect(stack, effect, None, living.entity())
                })
                .flatten(),
            Target::Mob(_) => {
                self.apply_consume_effect(stack, effect, Some(living), living.entity())
            }
            _ => None,
        }
    }

    /// `LivingEntity.addEffect(instance)`: gives the living entity the
    /// effect by the game's own path. A player's or a guest's goes through
    /// `add_player_effect` (so `MobEffectInstance.update`'s rules, the
    /// attribute modifiers, absorption and the added-effect hooks of a
    /// content mod's effect all apply); a mob's goes past `canBeAffected`
    /// (a spider shrugs off poison) into its effects with the same hooks.
    /// The amplifier is cut to 0..=255 and the duration is -1 (infinite) or
    /// ticks. False before effect resolution, for a duration below -1, an armor
    /// stand, the dragon, a stale handle or a client, or when the active
    /// instance did not change, as in vanilla's `addEffect`.
    pub fn living_add_effect(&mut self, living: LivingRef, effect: &ActiveEffect) -> bool {
        let Some(def) = mod_api::__private::effect_index(effect.id)
            .and_then(sunrise_game_data::potions::effect_by_index)
        else {
            return false;
        };
        if effect.duration < -1 {
            return false;
        }
        let amplifier = effect.amplifier.clamp(0, 255);
        let mut instance = EffectInstance::from_def(def, effect.duration, amplifier);
        instance.ambient = effect.ambient;
        instance.visible = effect.visible;
        instance.show_icon = effect.show_icon;
        match self.native_living_target(living) {
            Some(t @ (Target::Player | Target::Guest(_))) => self
                .for_player(t, |g| g.add_player_effect(instance))
                .unwrap_or(false),
            Some(Target::Mob(id)) => self.add_mob_effect(id, instance, None),
            _ => false,
        }
    }

    /// `LivingEntity.removeEffect(effect)` the way its running out goes
    /// (`LivingEntity.tickEffects` drops it): what the effect gave the
    /// player goes with it — the attribute modifiers recompute
    /// (`apply_effect_status`) and absorption's hearts leave — and a mob's
    /// hidden and ambient lists drop it too, with absorption's hearts
    /// leaving the same way its expiry takes them. False (and nothing
    /// changes) when the entity has the effect not, before effect resolution, or for
    /// an armor stand, the dragon, a stale handle or a client.
    pub(in crate::game) fn remove_mob_effect_by_index(
        &mut self,
        id: u32,
        effect_index: usize,
    ) -> bool {
        let Some(def) = sunrise_game_data::potions::effect_by_index(effect_index) else {
            return false;
        };
        let Some(amplifier) = self
            .common
            .mobs
            .iter()
            .find(|m| m.id == id && !m.removed)
            .and_then(|m| {
                m.extra
                    .effects
                    .iter()
                    .find(|e| e.registry_index == Some(def.index))
                    .map(|e| e.amplifier)
            })
        else {
            return false;
        };
        // `MobEffectEvent.Remove`: refusing the removal keeps it.
        if let Some(living) = self.effect_living_ref(Some(id as i32))
            && let Some(key) = mod_api::__private::effect_key_by_index(def.index)
        {
            let mut cancel = false;
            self.native_mob_effect_remove(living, key, amplifier, &mut cancel);
            if cancel {
                return false;
            }
        }
        self.mutate_mob_effects(id, Some(def.id), |mob| {
            mob.retain_effects(|e| e.registry_index != Some(def.index))
        })
        .unwrap_or(false)
    }

    pub fn living_remove_effect(&mut self, living: LivingRef, effect: EffectKey) -> bool {
        let Some(def) = mod_api::__private::effect_index(effect)
            .and_then(sunrise_game_data::potions::effect_by_index)
        else {
            return false;
        };
        match self.native_living_target(living) {
            Some(t @ (Target::Player | Target::Guest(_))) => self
                .for_player(t, |g| g.remove_player_effects(Some(def.index)))
                .unwrap_or(false),
            Some(Target::Mob(id)) => self.remove_mob_effect_by_index(id, def.index),
            _ => false,
        }
    }

    /// `LivingEntity.removeAllEffects()` the way their running out goes:
    /// every effect's attribute modifiers and absorption hearts leave with
    /// them. False when the entity has none, or for an armor stand, the
    /// dragon, a stale handle or a client.
    pub fn living_remove_all_effects(&mut self, living: LivingRef) -> bool {
        match self.native_living_target(living) {
            Some(t @ (Target::Player | Target::Guest(_))) => self
                .for_player(t, |g| g.remove_player_effects(None))
                .unwrap_or(false),
            Some(Target::Mob(id)) => {
                let held: Vec<(usize, i32)> = self
                    .common
                    .mobs
                    .iter()
                    .find(|m| m.id == id && !m.removed)
                    .map(|m| {
                        m.extra
                            .effects
                            .iter()
                            .filter_map(|e| e.registry_index.map(|index| (index, e.amplifier)))
                            .collect()
                    })
                    .unwrap_or_default();
                // `MobEffectEvent.Remove` per effect: a refused removal
                // keeps that effect.
                let keep: Vec<usize> = held
                    .iter()
                    .filter(|(name, amplifier)| {
                        let mut cancel = false;
                        if let Some(key) = mod_api::__private::effect_key_by_index(*name) {
                            self.native_mob_effect_remove(living, key, *amplifier, &mut cancel);
                        }
                        cancel
                    })
                    .map(|(name, _)| *name)
                    .collect();
                // The refused removals keep their effects; the rest go.
                self.mutate_mob_effects(id, None, |mob| {
                    mob.retain_effects(|e| {
                        e.registry_index.is_some_and(|index| keep.contains(&index))
                    })
                })
                .unwrap_or(false)
            }
            _ => false,
        }
    }
}
