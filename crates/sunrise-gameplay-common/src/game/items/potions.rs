#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    use sunrise_core::nbt::Tag;

    pub fn flt(v: f32) -> Tag {
        Tag::Float(v.to_bits())
    }
    #[cfg(any(test, feature = "test-support"))]
    #[derive(Clone)]
    pub struct FoodEffectMod(pub std::sync::Arc<std::sync::atomic::AtomicBool>);
    #[cfg(any(test, feature = "test-support"))]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for FoodEffectMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            for id in ["test:comfort", "test:nourishment"] {
                let key = r.effect_key(id);
                r.mob_effect_part(&key, self.clone()).unwrap();
            }
        }
        fn mob_effect_expired(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::MobEffectExpiredEvent,
        ) -> mod_api::Flow {
            if self.0.load(std::sync::atomic::Ordering::SeqCst)
                && event.effect == mod_api::__private::effect_key("minecraft:regeneration").unwrap()
            {
                mod_api::Flow::Cancel
            } else {
                mod_api::Flow::Continue
            }
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    #[mod_api::sunrise_part]
    impl mod_api::MobEffectPart for FoodEffectMod {
        fn should_apply_effect_tick_this_tick(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::ShouldApplyEffectTickThisTickEvent,
        ) {
            let comfort = mod_api::__private::effect_key("test:comfort").unwrap();
            event.apply = event.effect != comfort || event.tick_count % 80 == 0;
        }
        fn apply_effect_tick(
            &self,
            host: &dyn mod_api::Host,
            event: &mut mod_api::ApplyEffectTickEvent<'_>,
        ) {
            let level = host.level(event.dimension).unwrap();
            let natural = level
                .game_rule(host, "minecraft:natural_health_regeneration")
                .unwrap()
                .unwrap();
            let mod_api::GameRuleValue::Bool(natural) = natural else {
                panic!("boolean game rule")
            };
            if event.effect == mod_api::__private::effect_key("test:comfort").unwrap() {
                if !event
                    .has_effect(mod_api::__private::effect_key("minecraft:regeneration").unwrap())
                    && event.food.is_none_or(|food| food.saturation <= 0.0)
                    && event.health < event.max_health
                {
                    event.heal(1.0);
                }
            } else if let Some(food) = event.food {
                if event.amplifier == 1 {
                    // Pin FoodData.addExhaustion's lack of a lower clamp.
                    event.add_exhaustion(-5.0);
                } else if !(natural && event.health < event.max_health && food.level >= 18)
                    && food.exhaustion > 0.0
                {
                    event.add_exhaustion(-food.exhaustion.min(4.0));
                }
            }
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub struct OtherEffectMod;
    #[cfg(any(test, feature = "test-support"))]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for OtherEffectMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let effect = r.effect_key("other-mod:foreign");
            r.mob_effect_part(&effect, OtherEffectMod).unwrap();
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    #[mod_api::sunrise_part]
    impl mod_api::MobEffectPart for OtherEffectMod {
        fn should_apply_effect_tick_this_tick(
            &self,
            _: &dyn mod_api::Host,
            _: &mut mod_api::ShouldApplyEffectTickThisTickEvent,
        ) {
            panic!("another mod's shouldApply listener must not run");
        }
        fn apply_effect_tick(
            &self,
            _: &dyn mod_api::Host,
            _: &mut mod_api::ApplyEffectTickEvent<'_>,
        ) {
            panic!("another mod's apply listener must not run");
        }
    }
}
