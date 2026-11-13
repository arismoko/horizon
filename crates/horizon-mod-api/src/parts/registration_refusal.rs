//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

impl RegistrationRefusalRegistrar for PartRegistrar {
    fn registration_refusal(&mut self, reason: String) {
        if self.self_refusal.is_none() {
            let context = crate::RegistrationContext::new();
            context.refuse(&reason);
            self.self_refusal = mod_vocab::__private::take_registration_refusal(&context);
        }
    }
}

impl PartRegistrar {
    pub(crate) fn registration_checkpoint(&self) -> impl FnOnce(&mut Self) + use<> {
        let parts = self.parts.clone();
        let tags = self.tags.clone();
        let keys = self.registry_keys.borrow().clone();
        let menu_recipe_books = self.menu_recipe_books.clone();
        let recipe_book_types = self.recipe_book_types.clone();
        let registered = self.registered;
        let block_entity_transforms = self.block_entity_transforms.clone();
        // Append-only tables retain previous mods' entries without requiring
        // Clone on runtime callback objects or their eventual frozen caches.
        macro_rules! checkpoint {
            ($($field:ident),* $(,)?) => {{
                $(let $field = self.$field.len();)*
                move |registrar: &mut Self| {
                    $(registrar.$field.truncate($field);)*
                    registrar.block_entity_transforms = block_entity_transforms;
                    registrar.parts = parts;
                    registrar.tags = tags;
                    *registrar.registry_keys.borrow_mut() = keys;
                    registrar.menu_recipe_books = menu_recipe_books;
                    registrar.recipe_book_types = recipe_book_types;
                    registrar.registered = registered;
                    registrar.self_refusal = None;
                }
            }};
        }
        checkpoint!(
            game_rule_categories,
            game_rules,
            block_entity_types,
            entity_attributes,
            general_entity_types,
            world_fluids,
            entity_attribute_modifications,
            attributes,
            spawn_placements,
            biome_regions,
            controllers,
            arm_poses,
            structure_types,
            dimensions,
            chunk_generators,
            refused
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::{Hooks, ModEntry, Registrar, RegistrationContext};

    struct Callback;
    impl crate::BlockPart for Callback {
        fn block_subscribed(&self) -> &'static [Option<crate::HookId>] {
            &[]
        }
    }
    impl crate::BlockEntityPart for Callback {
        fn block_entity_subscribed(&self) -> &'static [Option<crate::HookId>] {
            &[]
        }
    }

    fn region() -> crate::BiomeRegion {
        crate::BiomeRegion::new(
            "minecraft:overworld".into(),
            5,
            vec![crate::BiomePoint {
                biome: "minecraft:plains".into(),
                temperature: (0.0, 0.0),
                humidity: (0.0, 0.0),
                continentalness: (0.0, 0.0),
                erosion: (0.0, 0.0),
                depth: (0.0, 0.0),
                weirdness: (0.0, 0.0),
                offset: 0.0,
            }],
        )
    }

    struct Refuses;
    impl Hooks for Refuses {
        fn subscribed(&self) -> &'static [Option<crate::HookId>] {
            &[]
        }
        fn register(&self, r: &mut dyn Registrar, cx: &RegistrationContext) {
            r.declare_fluid("before");
            r.declare_biome_region("rejected", region()).unwrap();
            let kind = r
                .block_entity_type("callback")
                .blocks(&[])
                .transform(|data: &crate::Compound, _: crate::StructureTransform| Ok(data.clone()))
                .build();
            r.block_entity_part(&kind, Callback).unwrap();
            cx.refuse("bootstrap failed: café\nmissing bridge");
            cx.refuse("ignored");
            r.declare_fluid("foreign:after");
        }
    }
    struct Healthy;
    impl Hooks for Healthy {
        fn subscribed(&self) -> &'static [Option<crate::HookId>] {
            &[]
        }
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.declare_fluid("soup");
            r.declare_biome_region("kept", region()).unwrap();
        }
    }

    #[test]
    fn refusal_rolls_back_only_its_owner_and_preserves_the_reason() {
        let mods = [
            ModEntry::new("healthy", Box::new(Healthy)),
            ModEntry::new("broken", Box::new(Refuses)),
            ModEntry::new("later", Box::new(Healthy)),
        ];
        let registrar = super::super::register_all(&mods, |_| false);
        assert_eq!(
            registrar.refused,
            ["mod broken refused registration: bootstrap failed: café\nmissing bridge"]
        );
        assert_eq!(
            registrar
                .biome_regions
                .iter()
                .map(|d| (d.mod_id.as_str(), d.id.as_str(), d.region.weight))
                .collect::<Vec<_>>(),
            [("healthy", "healthy:kept", 5), ("later", "later:kept", 5)]
        );
        assert!(registrar.block_entity_types.is_empty());
        assert!(registrar.block_entity_transforms.is_empty());
        assert!(registrar.parts.block_entity_kinds.is_empty());
        let keys = registrar.registry_keys.borrow();
        assert!(keys.declared_fluids.contains_key("healthy:soup"));
        assert!(keys.declared_fluids.contains_key("later:soup"));
        assert!(!keys.declared_fluids.contains_key("broken:before"));
        assert!(!keys.declared_fluids.contains_key("foreign:after"));
    }
}
