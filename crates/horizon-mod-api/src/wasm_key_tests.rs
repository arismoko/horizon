//! Real generated imports against the canonical load-owned registry tables.
use super::*;
use crate::{
    Backend, BackendRef, BlockEntityTypeKey, BlockKey, BoolGameRuleKey, EffectKey, FluidKey, Hooks,
    Host, ItemKey, ModEntry, Registrar,
};

#[test]
fn generated_key_imports_use_canonical_slots_and_refuse_before_the_backend() {
    struct Requests;
    #[crate::__subscriptions]
    impl Hooks for Requests {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            r.game_rule_category("rules").unwrap();
            let rule = r.game_rule_bool("enabled", "boundary:rules", true).unwrap();
            assert_eq!(BoolGameRuleKey::from_slot(rule.0), Ok(rule));
            assert!(BoolGameRuleKey::from_slot(rule.0 + 1).is_err());
            let block = r.block_key("stone");
            let effect = r.effect_key("speed");
            let item = r.item_key("stick");
            assert_eq!(ItemKey::from_slot(item.0), Ok(item));
            assert!(ItemKey::from_slot(item.0 + 1).is_err());
            let fluid = r.declare_fluid("chocolate");
            assert_eq!(FluidKey::from_slot(fluid.0), Ok(fluid));
            assert!(FluidKey::from_slot(fluid.0 + 1).is_err());
            let block_entity = r.block_entity_type_key("test:machine");
            assert_eq!(
                BlockEntityTypeKey::from_slot(block_entity.0),
                Ok(block_entity)
            );
            assert_eq!(BlockKey::from_slot(block.0), Ok(block));
            assert_eq!(EffectKey::from_slot(effect.0), Ok(effect));
            assert!(BlockKey::from_slot(block.0 + 1).is_err());
            assert!(EffectKey::from_slot(effect.0 + 1).is_err());
            // Re-enter a real generated import before the registrar has been installed.
            let table = Table(Cell::new(0));
            let mut frame = Frame::default();
            frame.host(&table);
            let _restore = Restore(CURRENT.replace(&mut frame));
            let mut state = State {
                limits: wasmtime::StoreLimitsBuilder::new().build(),
                buffers: HashMap::new(),
            };
            use glue::horizon::mod_api::host::Host as Reads;
            assert_eq!(
                Reads::block_state_default(&mut state, block.0).unwrap(),
                Some(19)
            );
            assert!(
                Reads::block_state_default(&mut state, block.0 + 1)
                    .unwrap_err()
                    .to_string()
                    .contains("unknown BlockKey slot")
            );
            assert_eq!(
                table.0.get(),
                1,
                "registration validates against pending requests, without executing refused imports"
            );
        }
    }
    crate::install_parts(
        &[ModEntry::new("boundary", Box::new(Requests))],
        &crate::parts::tests::rule_schema(),
    )
    .unwrap();
    crate::registry_keys::resolve_effects(
        horizon_client_core::potions::all_effects()
            .map(|def| horizon_client_core::potions::namespaced(def.id)),
        |id| horizon_client_core::potions::effect(id).map(|def| def.index),
    )
    .unwrap();
    {
        let types = &horizon_client_core::block_entity_types::BlockEntityTypes::new(
            Default::default(),
            vec!["test:machine".into()],
        );
        crate::registry_keys::resolve_block_entity_types(types.names(), |id| types.key(id))
    }
    .unwrap();
    struct Table(Cell<usize>);
    impl mod_api::prelude::KeysBackend for Table {
        fn block_key_id(&self, key: BlockKey) -> String {
            self.0.set(self.0.get() + 1);
            crate::registry_keys::block_id(key).unwrap().to_owned()
        }
        fn block_entity_type_key_id(&self, key: BlockEntityTypeKey) -> String {
            self.0.set(self.0.get() + 1);
            crate::registry_keys::block_entity_type_name(key)
                .unwrap()
                .to_owned()
        }
        fn effect_key_id(&self, key: EffectKey) -> String {
            self.0.set(self.0.get() + 1);
            crate::registry_keys::effect_id(key).unwrap().to_owned()
        }
    }
    impl Backend for Table {}
    mod_api::impl_backend_areas!(Table, except(keys));
    impl mod_api::prelude::TagsHost for Table {
        fn unresolved_fluid_id(&self, fluid: crate::UnresolvedFluid) -> String {
            crate::unresolved_fluid::name(fluid)
        }
    }
    impl mod_api::prelude::BlockStatesHost for Table {
        fn block_state_default(&self, block: BlockKey) -> Option<crate::BlockStateId> {
            assert_eq!(block.0, 0);
            self.0.set(self.0.get() + 1);
            Some(crate::BlockStateId(19))
        }
    }
    impl Host for Table {
        fn backend(&self) -> Option<BackendRef<'_>> {
            Some(BackendRef::of(self))
        }
    }
    mod_api::impl_host_areas!(Table, except(block_states, tags));
    impl mod_api::prelude::LivingBackendMut for Table {
        fn living_add_effect(
            &mut self,
            _: crate::LivingRef,
            effect: &crate::NativeActiveEffect,
        ) -> bool {
            self.0.set(self.0.get() + 1);
            assert_eq!(effect.id, EffectKey::from_slot(0).unwrap());
            assert_eq!(effect.duration, 37);
            true
        }
    }
    impl crate::BackendMut for Table {}
    mod_api::impl_backend_mut_areas!(Table, except(living));
    impl crate::ServerHost for Table {
        fn backend_mut(&mut self) -> Option<crate::BackendMutRef<'_>> {
            Some(crate::BackendMutRef::of(self))
        }
    }
    mod_api::impl_server_host_areas!(Table);
    let mut table = Table(Cell::new(0));
    let mut frame = Frame::default();
    frame.server_host_mut(&mut table);
    struct Restore(*mut Frame);
    impl Drop for Restore {
        fn drop(&mut self) {
            CURRENT.set(self.0);
        }
    }
    let _restore = Restore(CURRENT.replace(&mut frame));
    let mut state = State {
        limits: wasmtime::StoreLimitsBuilder::new().build(),
        buffers: HashMap::new(),
    };
    use glue::horizon::mod_api::backend::Host as Imports;
    assert_eq!(
        Imports::block_key_id(&mut state, 0).unwrap(),
        "minecraft:stone"
    );
    assert_eq!(
        Imports::effect_key_id(&mut state, 0).unwrap(),
        "minecraft:speed"
    );
    assert_eq!(
        Imports::block_entity_type_key_id(&mut state, 0).unwrap(),
        "test:machine"
    );
    assert!(
        Imports::block_entity_type_key_id(&mut state, u32::MAX)
            .unwrap_err()
            .to_string()
            .contains("unknown BlockEntityTypeKey slot")
    );
    // Canonical effect enumeration adds host-owned keys not explicitly requested by a mod.
    let poison = crate::parts::tests::effect_key("poison");
    assert_ne!(poison.0, 0);
    assert_eq!(
        Imports::effect_key_id(&mut state, poison.0).unwrap(),
        "minecraft:poison"
    );
    let handle = crate::unresolved_fluid::intern("absent:wasm").unwrap();
    use glue::horizon::mod_api::host::Host as Reads;
    assert_eq!(
        Reads::unresolved_fluid_id(&mut state, handle.0).unwrap(),
        "absent:wasm"
    );
    assert!(Reads::unresolved_fluid_id(&mut state, u64::MAX).is_err());
    let calls = table.0.get();
    assert_eq!(calls, 4);
    for slot in [1, u32::MAX] {
        assert!(
            Imports::block_key_id(&mut state, slot)
                .unwrap_err()
                .to_string()
                .contains("unknown BlockKey slot")
        );
    }
    let unknown_effect = horizon_client_core::potions::all_effects().count() as u32;
    for slot in [unknown_effect, u32::MAX] {
        assert!(
            Imports::effect_key_id(&mut state, slot)
                .unwrap_err()
                .to_string()
                .contains("unknown EffectKey slot")
        );
    }
    assert_eq!(
        table.0.get(),
        calls,
        "unknown slots never execute the backend"
    );
    // This real imported operation takes a record, so its key must be lifted recursively.
    use glue::horizon::mod_api::{backend_mut::Host as Writes, types};
    let living = || types::EntityRef { epoch: 1, id: 42 };
    let active = |id| types::NativeActiveEffect {
        id,
        duration: 37,
        amplifier: 1,
        ambient: false,
        visible: true,
    };
    assert!(Writes::living_add_effect(&mut state, living(), active(0)).unwrap());
    assert_eq!(table.0.get(), calls + 1);
    assert!(
        Writes::living_add_effect(&mut state, living(), active(unknown_effect))
            .unwrap_err()
            .to_string()
            .contains("unknown EffectKey slot")
    );
    assert_eq!(
        table.0.get(),
        calls + 1,
        "invalid nested keys never execute the write"
    );
    // Native id(host) uses the same ordinary read backend. No guest name cache is involved.
    assert_eq!(
        BlockKey::from_slot(0).unwrap().id(&table),
        "minecraft:stone"
    );
    assert_eq!(poison.id(&table), "minecraft:poison");
    assert!(
        crate::registry_keys::block(BlockKey::from_slot(0).unwrap()).is_none(),
        "slot validity does not require a world mapping"
    );
}
