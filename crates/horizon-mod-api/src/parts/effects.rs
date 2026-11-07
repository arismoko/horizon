use super::*;
pub(crate) fn resolved_effects() -> &'static OnceLock<horizon_mod_runtime::registry_keys::Effects> {
    static EMPTY: OnceLock<horizon_mod_runtime::registry_keys::Effects> = OnceLock::new();
    installed().map_or(&EMPTY, |installed| &installed.effects)
}

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use super::*;
    #[cfg(test)]
    use crate::*;

    #[test]
    pub(crate) fn registry_keys_resolve_effects_declared_later_and_generated_name_reads() {
        struct Keys(Arc<std::sync::Mutex<Vec<EffectKey>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Keys {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let speed = r.effect_key("speed");
                let later = r.effect_key("key_test:later");
                assert_eq!(EffectKey::from_slot(speed.0), Ok(speed));
                assert_eq!(EffectKey::from_slot(later.0), Ok(later));
                assert!(EffectKey::from_slot(later.0 + 1).is_err());
                self.0.lock().unwrap().extend([speed, later]);
                assert_eq!(speed, r.effect_key("minecraft:speed"));
            }
        }
        let requested = Arc::new(std::sync::Mutex::new(Vec::new()));
        install_parts(
            &[
                ModEntry::new("first", Box::new(Keys(requested.clone()))),
                ModEntry::new("second", Box::new(Keys(requested.clone()))),
            ],
            &crate::parts::tests::rule_schema(),
        )
        .unwrap();
        let speed_key = requested.lock().unwrap()[0];
        assert_eq!(EffectKey::from_slot(speed_key.0), Ok(speed_key));
        assert!(EffectKey::from_slot(2).is_err());
        assert!(
            crate::registry_keys::effect(speed_key).is_none(),
            "startup has no resolved effects"
        );
        assert!(horizon_client_core::potions::register_mod_effects([(
            "key_test:later".to_owned(),
            0x123456,
            horizon_client_core::potions::Category::Neutral
        )]));
        crate::registry_keys::resolve_effects(
            horizon_client_core::potions::all_effects()
                .map(|def| horizon_client_core::potions::namespaced(def.id)),
            |id| horizon_client_core::potions::effect(id).map(|def| def.index),
        )
        .unwrap();
        let keys = requested.lock().unwrap();
        assert_eq!(keys[0], keys[2]);
        assert_eq!(keys[1], keys[3]);
        let speed = crate::registry_keys::effect(keys[0]).unwrap();
        let later = crate::registry_keys::effect(keys[1]).unwrap();
        assert_eq!(speed.id, "speed");
        assert_eq!(later.id, "key_test:later");
        assert_eq!(later.color, 0x123456);
        assert_eq!(crate::registry_keys::effect_key(later.index), Some(keys[1]));
        for def in horizon_client_core::potions::all_effects() {
            let key = crate::registry_keys::effect_key(def.index).unwrap();
            assert_eq!(crate::registry_keys::effect(key).unwrap().id, def.id);
            assert_eq!(
                crate::registry_keys::effect_id(key),
                Some(horizon_client_core::potions::namespaced(def.id))
            );
        }

        assert!(EffectKey::from_slot(u32::MAX).is_err());
        struct Table;
        impl mod_api::prelude::KeysBackend for Table {
            fn effect_key_id(&self, key: EffectKey) -> String {
                crate::registry_keys::effect_id(key)
                    .expect("invalid effect key slot")
                    .to_owned()
            }
        }
        impl Backend for Table {}
        mod_api::impl_backend_areas!(Table, except(keys));
        impl Host for Table {
            fn backend(&self) -> Option<BackendRef<'_>> {
                Some(BackendRef::of(self))
            }
        }
        mod_api::impl_host_areas!(Table);
        assert_eq!(keys[0].id(&Table), "minecraft:speed");
        assert_eq!(keys[1].id(&Table), "key_test:later");
        struct NoGame;
        impl Host for NoGame {}
        mod_api::impl_host_areas!(NoGame);
        assert!(std::panic::catch_unwind(|| keys[0].id(&NoGame)).is_err());
    }
}

/// Registrar operations for effects.
pub trait EffectsRegistrar {
    /// Request an effect by canonical registry id. All mods' declarations are checked at load.
    fn effect_key(&mut self, id: &str) -> crate::EffectKey;
}

impl EffectsRegistrar for PartRegistrar {
    fn effect_key(&mut self, id: &str) -> crate::EffectKey {
        match self
            .registry_keys
            .borrow_mut()
            .effects
            .request(self.mod_id, id)
        {
            Ok(slot) => crate::EffectKey(slot),
            Err(error) => {
                self.refused.push(error);
                crate::EffectKey(u32::MAX)
            }
        }
    }
}
