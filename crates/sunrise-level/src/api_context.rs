//! Borrowed adapters for load, loop and worldgen hooks shared by client-lib and gameplay.
use std::sync::Arc;
use sunrise_world::native_load_context as engine;

/// Convert the owning load's phase, without reading resources.
pub fn load_origin(origin: engine::DataLoadOrigin) -> mod_api::DataLoadOrigin {
    match origin {
        engine::DataLoadOrigin::ServerDataLoad => mod_api::DataLoadOrigin::ServerDataLoad,
        engine::DataLoadOrigin::ClientPreload => mod_api::DataLoadOrigin::ClientPreload,
        _ => unreachable!("all engine load origins are mapped"),
    }
}

/// Convert the loop's owning side.
pub fn loop_side(side: engine::LoopSide) -> mod_api::LoopSide {
    match side {
        engine::LoopSide::Server => mod_api::LoopSide::Server,
        engine::LoopSide::Client => mod_api::LoopSide::Client,
        _ => unreachable!("all engine loop sides are mapped"),
    }
}

/// Borrows one owning load's frozen metadata, without scanning or copying it.
pub struct LoadReader<'a>(pub &'a engine::LoadContext);
impl LoadReader<'_> {
    /// Build the API view after the hook's subscription guard.
    pub fn context(&self) -> mod_api::OwningLoad<'_> {
        mod_api::OwningLoad::new(load_origin(self.0.origin()), self)
    }
}
impl mod_api::LoadReader for LoadReader<'_> {
    fn registry(&self, name: &str) -> Option<&[String]> {
        self.0.registry(name)
    }
    fn fluid_tags(&self) -> Option<&[(String, Arc<[String]>)]> {
        self.0.fluid_tags()
    }
    fn setting(&self, mod_id: &str, key: &str) -> Option<&str> {
        self.0.setting(mod_id, key)
    }
}

/// Borrows one loop's frozen metadata, without scanning or copying it.
pub struct LoopReader<'a>(pub &'a engine::LoopContext);
impl LoopReader<'_> {
    /// Build the API view after the hook's subscription guard.
    pub fn context(&self) -> mod_api::OwningLoop<'_> {
        mod_api::OwningLoop::new(loop_side(self.0.side()), self, self.0.item_tags())
    }
}
impl mod_api::LoopReader for LoopReader<'_> {
    fn mod_recipe_property_contains(&self, set: &str, item: mod_api::ItemKey) -> bool {
        mod_api::__private::item_key_id(item)
            .is_some_and(|name| self.0.mod_recipe_property_contains(set, name))
    }
    fn registry(&self, name: &str) -> Option<&[String]> {
        self.0.registry(name)
    }
    fn setting(&self, mod_id: &str, key: &str) -> Option<&str> {
        self.0.setting(mod_id, key)
    }
}

/// Borrows a worldgen or disk worker's frozen resource generation.
pub struct WorldgenReader<'a>(pub &'a engine::WorldgenContext);

impl mod_api::__private::WorldgenReader for WorldgenReader<'_> {
    fn registry(&self, name: &str) -> Option<&[String]> {
        self.0.registry(name)
    }
    fn setting(&self, mod_id: &str, key: &str) -> Option<&str> {
        self.0.setting(mod_id, key)
    }
}

#[cfg(test)]
mod recipe_properties_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{DataLoadContext, LoopContext, WorldgenHost};

    #[test]
    fn item_tags_distinguish_unavailable_empty_ordered_and_replaced_generations() {
        struct RequestsItem(Arc<std::sync::OnceLock<mod_api::ItemKey>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for RequestsItem {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                self.0.set(r.item_key("minecraft:emerald")).unwrap();
            }
        }
        let requested = Arc::new(std::sync::OnceLock::new());
        mod_platform::__private::part_tests::install_parts_fixture(&[mod_api::ModEntry::new(
            "test",
            Box::new(RequestsItem(requested.clone())),
        )])
        .unwrap();
        let tag = mod_api::__private::tag_key(&mod_api::Identifier::parse("test:grain").unwrap())
            .unwrap()
            .typed();
        let missing =
            mod_api::__private::tag_key(&mod_api::Identifier::parse("test:missing").unwrap())
                .unwrap()
                .typed();
        let load = engine::LoadContext::new(
            engine::DataLoadOrigin::ServerDataLoad,
            Arc::new(engine::LoadInputs::new(
                Default::default(),
                Default::default(),
            )),
            [],
        );
        let mut generation = load.captured_loop_context(engine::LoopSide::Server);
        let ids = [
            "minecraft:air",
            "minecraft:emerald",
            "minecraft:diamond",
            "minecraft:iron_ingot",
        ];
        let registry = [
            "minecraft:air",
            "minecraft:emerald",
            "minecraft:diamond",
            "test:no_api_key",
        ]
        .into_iter()
        .collect();
        let tags = std::collections::HashMap::from([
            (
                "test:grain".into(),
                vec![
                    "minecraft:emerald".into(),
                    "minecraft:diamond".into(),
                    "minecraft:air".into(),
                ],
            ),
            ("test:empty".into(), vec![]),
            ("test:late".into(), vec!["minecraft:air".into()]),
            (
                "test:no_api_key".into(),
                vec!["minecraft:emerald".into(), "test:no_api_key".into()],
            ),
            (
                "test:outside_generation".into(),
                vec!["minecraft:emerald".into(), "minecraft:iron_ingot".into()],
            ),
        ]);
        let emerald = *requested.get().unwrap();
        generation.set_item_tags(mod_api::__private::BoundItemTags::bind(&tags, &registry));
        let unbound = generation.clone();
        mod_api::__private::resolve_items(ids).unwrap();
        let unbound_reader = LoopReader(&unbound);
        let unbound = unbound_reader.context();
        assert_eq!(unbound.items_in_tag(tag), None);
        assert_eq!(unbound.is_in_tag(emerald, tag), None);

        let diamond = mod_api::__private::item_key("minecraft:diamond").unwrap();
        let air = mod_api::__private::item_key("minecraft:air").unwrap();
        generation.set_item_tags(mod_api::__private::BoundItemTags::bind(&tags, &registry));
        let frozen = generation.clone();
        let old_reader = LoopReader(&frozen);
        let old = old_reader.context();
        let empty = mod_api::__private::tag_key(&mod_api::Identifier::parse("test:empty").unwrap())
            .unwrap()
            .typed();
        // A mod can resolve a populated tag's key after the table is published.
        let late = mod_api::__private::tag_key(&mod_api::Identifier::parse("test:late").unwrap())
            .unwrap()
            .typed();
        let ordered = old.items_in_tag(tag).unwrap();
        assert_eq!(ordered, [emerald, diamond, air]);
        assert_eq!(old.items_in_tag(late), Some(&[air][..]));
        assert_eq!(old.items_in_tag(empty), Some(&[][..]));
        assert_eq!(old.items_in_tag(missing), Some(&[][..]));
        assert_eq!(old.is_in_tag(emerald, tag), Some(true));
        assert_eq!(old.is_in_tag(emerald, missing), Some(false));
        for name in ["test:no_api_key", "test:outside_generation"] {
            let refused = mod_api::__private::tag_key(&mod_api::Identifier::parse(name).unwrap())
                .unwrap()
                .typed();
            assert_eq!(old.items_in_tag(refused), None, "{name}");
            assert_eq!(old.is_in_tag(emerald, refused), None, "{name}");
        }

        generation.set_item_tags(mod_api::__private::BoundItemTags::bind(
            &[("test:grain".into(), vec!["minecraft:diamond".into()])].into(),
            &registry,
        ));
        let new_reader = LoopReader(&generation);
        let new = new_reader.context();
        assert_eq!(new.items_in_tag(tag), Some(&[diamond][..]));
        assert_eq!(new.is_in_tag(emerald, tag), Some(false));
        assert_eq!(ordered, [emerald, diamond, air]);
    }

    #[test]
    fn recipe_fluid_tags_distinguish_absence_from_empty_and_lend_ordered_members() {
        let load = engine::LoadContext::from_settings_snapshot(
            engine::DataLoadOrigin::ServerDataLoad,
            Arc::new(engine::LoadInputs::new(
                Default::default(),
                Default::default(),
            )),
            [],
            Arc::from([]),
        );
        assert_eq!(LoadReader(&load).context().fluid_tags(), None);

        let empty = load.with_fluid_tags(Default::default());
        assert_eq!(LoadReader(&empty).context().fluid_tags(), Some(&[][..]));

        let populated = load.with_fluid_tags(std::collections::HashMap::from([(
            "test:tag".into(),
            vec!["test:still".into(), "test:flowing".into()],
        )]));
        let reader = LoadReader(&populated);
        let view = reader.context();
        let context: &dyn DataLoadContext = &view;
        let tags = context.fluid_tags().unwrap();
        assert!(std::ptr::eq(tags, populated.fluid_tags().unwrap()));
        let [(name, members)] = tags else {
            panic!("expected one fluid tag, got {tags:?}");
        };
        assert_eq!(name, "test:tag");
        assert_eq!(members.as_ref(), ["test:still", "test:flowing"]);

        let next = populated.for_load(engine::DataLoadOrigin::ServerDataLoad, []);
        assert_eq!(LoadReader(&next).context().fluid_tags(), None);
    }

    #[test]
    fn phase_readers_preserve_origins_sides_and_shared_resource_members() {
        let root = std::env::temp_dir().join(format!(
            "phase-reader-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(root.join("active")).unwrap();
        std::fs::write(root.join("active/mode"), " captured ").unwrap();
        let inputs = Arc::new(engine::LoadInputs::new(
            std::collections::BTreeMap::from([(
                "item".into(),
                vec!["test:second".into(), "test:first".into()],
            )]),
            root.clone(),
        ));
        mod_platform::__private::with_test_packages(&["second", "first"], || {
            for (origin, expected) in [
                (
                    engine::DataLoadOrigin::ServerDataLoad,
                    mod_api::DataLoadOrigin::ServerDataLoad,
                ),
                (
                    engine::DataLoadOrigin::ClientPreload,
                    mod_api::DataLoadOrigin::ClientPreload,
                ),
            ] {
                let base = engine::LoadContext::new(origin, inputs.clone(), ["active"]);
                let load = base.with_fluid_tags(std::collections::HashMap::from([(
                    "test:tag".into(),
                    vec!["test:flowing".into(), "test:still".into()],
                )]));
                let reader = LoadReader(&load);
                let context = reader.context();
                assert_eq!(context.origin(), expected);
                assert_eq!(
                    context.registry("item"),
                    Some(&["test:second".into(), "test:first".into()][..])
                );
                assert!(std::ptr::eq(
                    context.registry("item").unwrap(),
                    base.registry("item").unwrap()
                ));
                assert_eq!(context.registry("missing"), None);
                assert_eq!(context.setting("missing", "key"), None);
                assert_eq!(context.setting("active", "mode"), Some("captured"));
                assert_eq!(
                    context.mods().ids().collect::<Vec<_>>(),
                    ["second", "first"]
                );
                let worldgen = load
                    .captured_loop_context(engine::LoopSide::Server)
                    .worldgen_context();
                let reader = WorldgenReader(&worldgen);
                let host = mod_api::OwningWorldgen::new(&reader, &|_, _, _| None, &|_, _| None);
                let context: &dyn WorldgenHost = &host;
                let names = context.registry("item").unwrap();
                assert_eq!(names, ["test:second", "test:first"]);
                assert!(std::ptr::eq(names, load.registry("item").unwrap()));
                assert_eq!(context.setting("active", "mode"), Some("captured"));

                for (side, expected) in [
                    (engine::LoopSide::Client, mod_api::LoopSide::Client),
                    (engine::LoopSide::Server, mod_api::LoopSide::Server),
                ] {
                    let source = load.loop_context(side, ["active"]);
                    let reader = LoopReader(&source);
                    let context = reader.context();
                    assert_eq!(context.side(), expected);
                    assert_eq!(
                        context.registry("item"),
                        Some(&["test:second".into(), "test:first".into()][..])
                    );
                    assert_eq!(context.registry("missing"), None);
                    assert_eq!(context.setting("missing", "key"), None);
                    assert_eq!(context.setting("active", "mode"), Some("captured"));
                    assert_eq!(
                        context.mods().ids().collect::<Vec<_>>(),
                        ["second", "first"]
                    );
                }
            }
        });
        std::fs::remove_dir_all(root).unwrap();
    }
}
