//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

impl PartRegistrar {
    /// New identifiers only, not attachment targets: Parts may attach to
    /// vanilla or another mod's existing content.
    pub(crate) fn owned_declaration_id(&self, name: &str) -> Result<String, String> {
        owned_declaration_id(self.mod_id, name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;

    fn with_porter(f: impl FnOnce()) {
        crate::load_state::with_test_package_inventory(
            vec![PackageInfo::new(
                "porter".into(),
                "1".into(),
                PackageKind::Code,
                Side::Both,
                vec!["porter".into(), "legacy".into()],
                vec![],
            )],
            f,
        );
    }

    #[test]
    fn manifest_owned_declarations_keep_the_persisted_namespace() {
        with_porter(|| {
            let mut r = PartRegistrar::new("porter");
            r.dimension_shape = |_| true;
            assert_eq!(r.owned_declaration_id("bare").unwrap(), "porter:bare");
            assert_eq!(r.owned_declaration_id("legacy:pot").unwrap(), "legacy:pot");
            let kind = r.declare_block_entity_type("legacy:pot", vec!["legacy:pot".into()]);
            assert_ne!(
                kind,
                mod_vocab::__private::parts_installation::block_entity_type_key(u32::MAX)
            );
            assert_eq!(r.block_entity_types[0].id, "legacy:pot");
            let fluid = r.declare_fluid("legacy:soup");
            assert_ne!(
                fluid,
                mod_vocab::__private::parts_installation::fluid_key(u32::MAX)
            );
            assert!(
                r.registry_keys
                    .borrow()
                    .declared_fluids
                    .contains_key("legacy:soup")
            );
            let world_key = r.declare_world_fluid("legacy:broth", {
                let mut value: WorldFluid = Default::default();
                value.tick_delay = 20;
                value.drop_off = 2;
                value.slope_find_distance = 3;
                value.source_conversion = true;
                value.bucket = false;
                value
            });
            assert_ne!(
                world_key,
                mod_vocab::__private::parts_installation::fluid_key(u32::MAX)
            );
            assert_eq!(world_key, r.fluid_key("legacy:broth"));
            let keys = r.registry_keys.borrow();
            assert!(keys.declared_fluids.contains_key("legacy:broth"));
            assert!(keys.declared_fluids.contains_key("legacy:flowing_broth"));
            assert!(!keys.declared_fluids.contains_key("porter:broth"));
            drop(keys);
            r.declare_attribute("legacy:stamina", 1.0, 0.0, 10.0, true, Sentiment::Neutral)
                .unwrap();
            r.declare_entity_attributes("legacy:cook", "minecraft:zombie", vec![])
                .unwrap();
            r.add_dimension("legacy:kitchen", DimensionSettings::default());
            assert!(r.recipe_book_type("legacy:kitchen").is_ok());
            assert_eq!(r.attributes[0].id, "legacy:stamina");
            assert_eq!(r.entity_attributes[0].id, "legacy:cook");
            assert_eq!(&*r.dimensions[0].id, "legacy:kitchen");
            assert!(r.refused.is_empty(), "{:?}", r.refused);
            assert_eq!(
                r.declare_block_entity_type("unowned:pot", vec![]),
                mod_vocab::__private::parts_installation::block_entity_type_key(u32::MAX)
            );
            let reason = r.refused.last().unwrap();
            assert!(
                reason.contains("porter") && reason.contains("unowned"),
                "{reason}"
            );
            assert!(r.owned_declaration_id("legacy:Bad Name").is_err());
        });
    }

    #[test]
    fn blocks_and_items_are_attributed_before_flattening_mod_outputs() {
        struct Declares;
        #[crate::__subscriptions]
        impl Hooks for Declares {
            fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
                event.blocks.push({
                    let mut value: Block = Block::default();
                    value.id = "legacy:pot".into();
                    value.template = "minecraft:stone".into();
                    value
                });
                event.blocks.push({
                    let mut value: Block = Block::default();
                    value.id = "unowned:pot".into();
                    value.template = "minecraft:stone".into();
                    value
                });
            }
            fn items(&self, _: &RegistrationContext, event: &mut ItemsEvent<'_>) {
                event.items.push({
                    let mut value: Item = Item::default();
                    value.id = "spoon".into();
                    value.template = "minecraft:stick".into();
                    value
                });
            }
        }
        let mods: Mods = vec![ModEntry::new("porter", Box::new(Declares))].into();
        crate::testing::with_mods(mods, || {
            with_porter(|| {
                let context = RegistrationContext::new();
                let mut blocks = Vec::new();
                let mut items = Vec::new();
                crate::dispatch_blocks(&context, &mut blocks);
                crate::dispatch_items(&context, &mut items);
                assert_eq!(blocks[0].id, "legacy:pot");
                assert_eq!(items[0].id, "porter:spoon");
                // Invalid output stays visible for an outer host dispatch to refuse
                // too, rather than being silently lost inside a wasm guest.
                assert_eq!(blocks[1].id, "unowned:pot");
                let errors = take_declaration_refusals(&context);
                assert_eq!(errors.len(), 1);
                assert!(errors[0].contains("porter") && errors[0].contains("unowned"));
            })
        });
    }
}
