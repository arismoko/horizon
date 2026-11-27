//! Static serializer resources adapted to the owning server loot generation.
use mod_api::{
    GlobalLootModifier, GlobalLootModifierSerializer, Identifier, LootModifierError,
    LootModifierSettings,
};
use std::{
    any::{Any, TypeId},
    collections::HashMap,
    path::Path,
    sync::Arc,
};
use sunrise_assets::resource_source::ResourceSource;
use sunrise_world::{
    loot::{
        self,
        global_modifiers::{
            AddTableModifier, DecodedModifier, GlobalLootModifiers, ModifierApply, ModifierFile,
            ModifierRoll,
        },
    },
    native_load_context::LoadContext,
};

pub struct Serializers(HashMap<String, Arc<dyn GlobalLootModifierSerializer>>);
impl Serializers {
    pub fn capture() -> Self {
        Self(
            mod_api::__private::global_loot_modifier_serializer_declarations()
                .into_iter()
                .collect(),
        )
    }
    fn decode(
        &self,
        id: &str,
        kind: &str,
        data: &serde_json::Value,
        load: &LoadContext,
        loot: &loot::LootData,
    ) -> Result<(DecodedModifier, Arc<dyn GlobalLootModifier>), String> {
        let prepare_condition = |condition: Option<serde_json::Value>| {
            condition
                .map(|value| {
                    loot.bindings()
                        .prepare_condition(value, &format!("loot_modifier {id} $.condition"))
                        .map_err(|error| error.to_string())
                })
                .transpose()
        };
        if kind == "neoforge:add_table" {
            let settings = LootModifierSettings::decode(data).map_err(|error| error.to_string())?;
            let table = data
                .get("table")
                .and_then(serde_json::Value::as_str)
                .ok_or("table: expected a loot table id")?;
            let table = Identifier::parse(table).map_err(|error| format!("table: {error}"))?;
            let decoded = DecodedModifier {
                priority: settings.priority,
                condition: prepare_condition(settings.condition.clone())?,
                apply: Arc::new(AddTableModifier {
                    table: table.to_string(),
                }),
            };
            return Ok((decoded, Arc::new(ApiAddTable { settings, table })));
        }
        let serializer = self
            .0
            .get(kind)
            .ok_or_else(|| format!("type: unknown global loot modifier serializer {kind}"))?;
        let reader = sunrise_level::api_context::LoadReader(load);
        let instance = serializer
            .decode(data, &reader.context())
            .map_err(|error| error.to_string())?;
        let condition = instance.condition().map_err(|error| error.to_string())?;
        let decoded = DecodedModifier {
            priority: instance.priority().map_err(|error| error.to_string())?,
            condition: prepare_condition(condition)?,
            apply: Arc::new(Adapter(instance.clone())),
        };
        Ok((decoded, instance))
    }
    pub fn prepare(
        &self,
        files: Vec<ModifierFile>,
        conditions: &sunrise_world::conditions::LoadConditions,
        load: &LoadContext,
        loot: &loot::LootData,
    ) -> Result<
        (
            Arc<GlobalLootModifiers>,
            Arc<mod_api::GlobalLootModifierManager>,
        ),
        String,
    > {
        use mod_api::__private::game::GlobalLootModifierManagerNew;
        let instances = std::cell::RefCell::new(HashMap::new());
        let generation = Arc::new(GlobalLootModifiers::load(
            files,
            conditions,
            |id, kind, data| {
                let (decoded, instance) = self.decode(id, kind, data, load, loot)?;
                instances.borrow_mut().insert(id.to_owned(), instance);
                Ok(decoded)
            },
            |error| log::error!("{error}"),
        ));
        let mut instances = instances.into_inner();
        let entries = generation
            .ids()
            .map(|id| {
                let key = Identifier::parse(id).map_err(|error| error.to_string())?;
                let instance = instances
                    .remove(id)
                    .ok_or_else(|| format!("prepared modifier {id} missing its API resource"))?;
                Ok((key, instance))
            })
            .collect::<Result<_, String>>()?;
        let manager = Arc::new(mod_api::GlobalLootModifierManager::prepared(
            entries,
            generation.clone(),
        ));
        Ok((generation, manager))
    }
}
struct ApiAddTable {
    settings: LootModifierSettings,
    table: Identifier,
}
impl GlobalLootModifier for ApiAddTable {
    fn priority(&self) -> Result<i32, LootModifierError> {
        Ok(self.settings.priority)
    }
    fn condition(&self) -> Result<Option<serde_json::Value>, LootModifierError> {
        Ok(self.settings.condition.clone())
    }
    fn apply(
        &self,
        context: &mut dyn mod_api::LootModifierContext,
        generated: &mut mod_api::LootList<'_>,
    ) -> Result<(), LootModifierError> {
        context.append_raw(&self.table, generated)
    }
}

struct Adapter(Arc<dyn GlobalLootModifier>);
impl ModifierApply for Adapter {
    fn apply(
        &self,
        roll: &mut dyn ModifierRoll,
        generated: &mut dyn loot::LootListAccess,
    ) -> Result<(), String> {
        let active = &*generated as &dyn Any;
        let mut context = RollAdapter {
            roll,
            list_type: Any::type_id(active),
            list_address: std::ptr::from_ref(active).cast::<()>(),
        };
        let mut list = super::api_values::loot_values::loot_list(generated);
        self.0
            .apply(&mut context, &mut list)
            .map_err(|error| error.to_string())
    }
}
struct RollAdapter<'a> {
    roll: &'a mut dyn ModifierRoll,
    list_type: TypeId,
    list_address: *const (),
}
impl mod_api::LootModifierContext for RollAdapter<'_> {
    fn params(&self) -> mod_api::LootParamsView<'_> {
        super::api_values::loot_values::loot_params(self.roll.facts())
    }
    fn random(&mut self) -> &mut dyn mod_api::RandomSource {
        self
    }
    fn roll_raw(
        &mut self,
        table: &Identifier,
        visit: &mut dyn FnMut(mod_api::LootStackView<'_>),
    ) -> Result<(), LootModifierError> {
        self.roll
            .roll_raw(&table.to_string(), &mut |stack| {
                visit(super::api_values::loot_values::loot_stack_view(stack));
            })
            .map_err(|error| LootModifierError::new("roll_raw", error))
    }
    fn append_raw(
        &mut self,
        table: &Identifier,
        generated: &mut mod_api::LootList<'_>,
    ) -> Result<(), LootModifierError> {
        let target = generated.access_mut();
        let backing = &*target as &dyn Any;
        // Check the actual evaluator's loan without calling through a mod's vtable.
        if Any::type_id(backing) != self.list_type
            || !std::ptr::eq(std::ptr::from_ref(backing).cast::<()>(), self.list_address)
        {
            return Err(LootModifierError::new(
                "append_raw",
                "foreign loot output list",
            ));
        }
        mod_api::__private::game::CallbackOwner::call_engine(|| {
            self.roll.append_raw(&table.to_string(), target)
        })
        .map_err(|error| LootModifierError::new("append_raw", error))
    }
}
impl mod_api::RandomSource for RollAdapter<'_> {
    fn next_int(&mut self) -> i32 {
        self.roll.next_int()
    }
    fn next_int_bound(&mut self, bound: i32) -> i32 {
        self.roll.next_int_bound(bound)
    }
    fn next_long(&mut self) -> i64 {
        self.roll.next_long()
    }
    fn next_float(&mut self) -> f32 {
        self.roll.next_float()
    }
    fn next_double(&mut self) -> f64 {
        self.roll.next_double()
    }
    fn next_boolean(&mut self) -> bool {
        self.roll.next_boolean()
    }
    fn next_gaussian(&mut self) -> f64 {
        self.roll.next_gaussian()
    }
}

/// Root resources are lowest, then passive content, native contributions and
/// the enabled packs. Readers run only after the loader selects each winning id.
pub fn files(
    root: &Path,
    stack: &super::function_pack_stack::PackStack,
    contributed: Vec<ModifierFile>,
) -> Vec<ModifierFile> {
    let mut files = Vec::new();
    for namespace in sunrise_world::json_folder::namespaces(&root.join("data")) {
        let directory = root.join("data").join(&namespace).join("loot_modifiers");
        let folder = match sunrise_world::json_folder::read(&directory) {
            Ok(folder) => folder,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                log::warn!("loot modifier folder {}: {error}", directory.display());
                continue;
            }
        };
        for (index, file) in folder.files.iter().enumerate() {
            let id = format!("{namespace}:{}", file.name);
            let path = file.path.display().to_string();
            let folder = folder.clone();
            files.push(ModifierFile {
                id,
                pack: "resources".into(),
                path,
                read: Box::new(move || {
                    folder.files[index]
                        .bytes()
                        .map(<[u8]>::to_vec)
                        .map_err(|error| error.to_string())
                }),
            });
        }
    }
    append_pack(&mut files, &stack.content.id, stack.content.source());
    files.extend(contributed);
    for pack in stack.enabled_packs() {
        append_pack(&mut files, &pack.id, pack.source());
    }
    files
}
fn append_pack(files: &mut Vec<ModifierFile>, pack: &str, source: &ResourceSource) {
    for path in source.list_under("data", &["loot_modifiers"]) {
        let Some((namespace, rest)) = path
            .strip_prefix("data/")
            .and_then(|path| path.split_once('/'))
        else {
            continue;
        };
        let Some(name) = rest
            .strip_prefix("loot_modifiers/")
            .and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };
        let id = format!("{namespace}:{name}");
        let source = source.clone();
        let read_path = path.clone();
        files.push(ModifierFile {
            id,
            pack: pack.into(),
            path,
            read: Box::new(move || source.read(&read_path).map_err(|error| error.to_string())),
        });
    }
}
/// Preserve native contributions even though DataStore does not interpret this
/// independently discovered resource folder. Their existing order is priority.
pub fn contributed(files: &[(String, Vec<u8>)]) -> Vec<ModifierFile> {
    files
        .iter()
        .filter_map(|(key, bytes)| {
            let id = key.strip_prefix("loot_modifiers/")?.to_owned();
            let Some((namespace, name)) = id.split_once(':') else {
                log::warn!(
                    "loot modifier {id} (native contributions): expected a namespace:path file id"
                );
                return None;
            };
            let path = format!("data/{namespace}/loot_modifiers/{name}.json");
            let bytes = bytes.clone();
            Some(ModifierFile {
                id,
                pack: "native contributions".into(),
                path,
                read: Box::new(move || Ok(bytes)),
            })
        })
        .collect()
}

/// The reload retains its own generation, never a global live manager. Initial
/// world opening binds the snapshot after its tables exist; reloads bind before
/// any listener apply. A failed preparation simply drops this unbound candidate.
pub fn register_anchor(
    registry: &mut mod_api::ServerListenerRegistry,
) -> Result<Arc<mod_api::GlobalLootModifierManager>, mod_api::ReloadOrderError> {
    use mod_api::ServerReloadListeners;
    let generation = Arc::new(mod_api::GlobalLootModifierManager::default());
    let id = mod_api::GlobalLootModifierManager::key().id().clone();
    registry.add_retained(
        id,
        mod_api::FileToIdConverter::json("loot_modifiers").unwrap(),
        generation.clone(),
        generation.clone(),
    )?;
    Ok(generation)
}
pub fn bind_retained(
    resources: &dyn mod_api::ServerReloadResources,
    generation: &mod_api::GlobalLootModifierManager,
) {
    use mod_api::__private::game::GlobalLootModifierManagerNew;
    if let Some(retained) = resources.listener(&mod_api::GlobalLootModifierManager::key()) {
        retained.bind_from(generation);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::loot_types;
    use mod_api::{
        __private::game::ServerReloadListenersOrder, DataLoadContext, Hooks, LootList,
        LootModifierContext, LootStack, LootStackEditError, ModEntry, Registrar,
        RegistrationContext, ServerListenerRegistry, ServerReloadListeners,
    };
    use serde_json::json;
    use std::{
        collections::BTreeMap,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };
    use sunrise_core::{
        nbt::Tag,
        random::{LegacyRandom, RandomSource as _},
    };
    use sunrise_world::native_load_context::{DataLoadOrigin, LoadInputs};

    #[test]
    fn reload_views_follow_the_registered_loot_anchor() {
        let mut listeners = ServerListenerRegistry::new();
        let functions = Identifier::parse("minecraft:function_library").unwrap();
        let loot = Identifier::parse("neoforge:loot_modifiers").unwrap();
        assert_eq!(listeners.get_last_vanilla_listener(), &functions);

        register_anchor(&mut listeners).unwrap();

        assert_eq!(listeners.get_last_vanilla_listener(), &functions);
        assert!(listeners.get_registry().iter().any(|id| id == &loot));
        assert!(listeners.get_graph().iter().next().is_none());

        let mod_listener = Identifier::parse("test_mod:after").unwrap();
        listeners
            .add_listener(
                mod_listener.clone(),
                mod_api::FileToIdConverter::json("machines").unwrap(),
                Arc::new(mod_api::GlobalLootModifierManager::default()),
            )
            .unwrap();
        let ordered = listeners.finish().unwrap().listeners;
        assert_eq!(
            ordered
                .into_iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            [functions, loot, mod_listener]
        );
    }

    #[derive(Clone, Copy)]
    enum ChainStage {
        First,
        RefuseAfterEdit,
        Last,
    }

    struct ChainMod;
    #[mod_api::__subscriptions]
    impl Hooks for ChainMod {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.item_key("minecraft:stick");
            r.item_key("minecraft:diamond");
            r.item_key("minecraft:gold_ingot");
            r.register_global_loot_modifier_serializer("step", ChainMod);
        }
    }
    impl GlobalLootModifierSerializer for ChainMod {
        fn decode(
            &self,
            data: &serde_json::Value,
            _: &dyn DataLoadContext,
        ) -> Result<Arc<dyn GlobalLootModifier>, LootModifierError> {
            let settings = LootModifierSettings::decode(data)?;
            let stage = match data.get("stage").and_then(serde_json::Value::as_str) {
                Some("first") => ChainStage::First,
                Some("err") => ChainStage::RefuseAfterEdit,
                Some("last") => ChainStage::Last,
                _ => return Err(LootModifierError::new("stage", "unknown stage")),
            };
            Ok(Arc::new(ChainStep { settings, stage }))
        }
    }
    struct ChainStep {
        settings: LootModifierSettings,
        stage: ChainStage,
    }
    impl GlobalLootModifier for ChainStep {
        fn priority(&self) -> Result<i32, LootModifierError> {
            Ok(self.settings.priority)
        }
        fn condition(&self) -> Result<Option<serde_json::Value>, LootModifierError> {
            Ok(self.settings.condition.clone())
        }
        fn apply(
            &self,
            context: &mut dyn LootModifierContext,
            generated: &mut LootList<'_>,
        ) -> Result<(), LootModifierError> {
            match self.stage {
                ChainStage::First => {
                    assert_eq!(context.params().queried_table_id(), "test:root");
                    assert_eq!(
                        generated.iter().map(|s| s.count()).collect::<Vec<_>>(),
                        [64, 64, 2]
                    );
                    assert_eq!(
                        context.random().next_long(),
                        LegacyRandom::new(31).next_long()
                    );
                    let raw_id = Identifier::parse("test:raw").unwrap();
                    let mut inspected = Vec::new();
                    context.roll_raw(&raw_id, &mut |stack| {
                        inspected.push((stack.item_id().to_owned(), stack.count()));
                    })?;
                    assert_eq!(inspected, [("minecraft:gold_ingot".to_owned(), 200)]);
                    {
                        let mut first = generated.get_mut(0).unwrap();
                        first.set_count(-3);
                        first.push_component("minecraft:custom_data", Tag::Int(42))?;
                    }
                    generated.truncate(1);
                    // A second list of the same engine type is still not the
                    // evaluator's active loan, even if its caller supplied it.
                    let mut foreign = loot::LootStacks::default();
                    let mut foreign_list =
                        super::super::api_values::loot_values::loot_list(&mut foreign);
                    let error = context.append_raw(&raw_id, &mut foreign_list).unwrap_err();
                    assert_eq!(error.field, "append_raw");
                    assert_eq!(error.reason, "foreign loot output list");
                    assert!(foreign.0.is_empty());
                    context.append_raw(&raw_id, generated)?;
                    assert!(generated.reorder(1, 0));

                    let stick = mod_api::__private::item_key("minecraft:stick").unwrap();
                    assert_eq!(generated.insert(99, LootStack::new(stick, 1)), Ok(false));
                    assert_eq!(generated.replace(99, LootStack::new(stick, 1)), Ok(false));
                    assert!(!generated.remove(99));
                    assert!(!generated.reorder(0, 99));
                    let mut bad = LootStack::new(stick, 1);
                    bad.components
                        .push(("minecraft:custom_data".into(), Some(Tag::End)));
                    assert_eq!(generated.push(bad), Err(LootStackEditError::InvalidPatch));
                    assert_eq!(generated.insert(1, LootStack::new(stick, 5)), Ok(true));
                    assert_eq!(generated.replace(1, LootStack::new(stick, 7)), Ok(true));
                    assert_eq!(generated.get(1).unwrap().count(), 7);
                    assert!(generated.remove(1));
                    Ok(())
                }
                ChainStage::RefuseAfterEdit => {
                    generated.get_mut(0).unwrap().set_count(205);
                    assert!(generated.reorder(0, 2));
                    context.random().next_long();
                    context.append_raw(&Identifier::parse("test:missing").unwrap(), generated)
                }
                ChainStage::Last => {
                    assert_eq!(
                        generated
                            .iter()
                            .map(|s| (s.item_id(), s.count()))
                            .collect::<Vec<_>>(),
                        [
                            ("minecraft:diamond", -3),
                            ("minecraft:gold_ingot", 200),
                            ("minecraft:gold_ingot", 205),
                        ]
                    );
                    assert_eq!(
                        generated.get(0).unwrap().components().last(),
                        Some(("minecraft:custom_data", Some(&Tag::Int(42))))
                    );
                    let mut expected = LegacyRandom::new(31);
                    expected.next_long();
                    expected.next_long();
                    assert_eq!(context.random().next_long(), expected.next_long());
                    Ok(())
                }
            }
        }
    }

    fn modifier_file(id: &str, value: serde_json::Value) -> ModifierFile {
        let bytes = serde_json::to_vec(&value).unwrap();
        ModifierFile {
            id: id.into(),
            pack: "test".into(),
            path: format!("{id}.json"),
            read: Box::new(move || Ok(bytes)),
        }
    }

    fn load_context() -> LoadContext {
        LoadContext::new(
            DataLoadOrigin::ServerDataLoad,
            Arc::new(LoadInputs::new(BTreeMap::new(), PathBuf::new())),
            std::iter::empty::<&str>(),
        )
    }

    #[test]
    fn scoped_global_modifiers_edit_reorder_append_and_preserve_edits_across_error() {
        let defaults = sunrise_world::item_defaults::ItemDefaults::decode(
            br#"{"minecraft:diamond":{"minecraft:max_stack_size":64},"minecraft:gold_ingot":{"minecraft:max_stack_size":64},"minecraft:stick":{"minecraft:max_stack_size":64}}"#,
        )
        .unwrap();
        let root_table = json!({
            "pools": [{"rolls": 1, "entries": [{"type": "minecraft:item", "name": "minecraft:diamond"}]}],
            "modifier": [
                {"type": "minecraft:set_count", "count": 130},
                {"type": "minecraft:set_components", "components": {"minecraft:max_stack_size": 64}}
            ]
        });
        let raw_table = json!({
            "pools": [
                {"rolls": 1, "entries": [{"type": "minecraft:loot_table", "value": "test:raw"}]},
                {"rolls": 1, "entries": [{"type": "minecraft:item", "name": "minecraft:gold_ingot", "modifier": {"type": "minecraft:set_count", "count": 200}}]}
            ]
        });
        let overlay = [
            (
                "loot_table/test:root".to_owned(),
                vec![serde_json::to_vec(&root_table).unwrap()],
            ),
            (
                "loot_table/test:raw".to_owned(),
                vec![serde_json::to_vec(&raw_table).unwrap()],
            ),
        ];

        mod_platform::testing::with_mods(
            vec![ModEntry::new("chain", Box::new(ChainMod))].into(),
            || {
                mod_platform::__private::part_tests::install_parts_fixture(
                    mod_api::__private::mods(),
                )
                .unwrap();
                mod_api::__private::resolve_items([
                    "minecraft:stick",
                    "minecraft:diamond",
                    "minecraft:gold_ingot",
                ])
                .unwrap();

                let mut data = loot::LootData::load_with_types(
                    Path::new("scoped-glm-overlay"),
                    &loot::PackOverlay::new(&overlay),
                    &defaults,
                    loot_types::capture(),
                )
                .unwrap();
                let serializers = Serializers::capture();
                let files = vec![
                    modifier_file(
                        "chain:first",
                        json!({"type": "chain:step", "stage": "first", "priority": 1002}),
                    ),
                    modifier_file(
                        "chain:add",
                        json!({"type": "neoforge:add_table", "table": "test:raw", "priority": 1001}),
                    ),
                    modifier_file(
                        "chain:err",
                        json!({"type": "chain:step", "stage": "err", "priority": 1000}),
                    ),
                    modifier_file(
                        "chain:last",
                        json!({"type": "chain:step", "stage": "last", "priority": 999}),
                    ),
                ];
                let (generation, manager) = serializers
                    .prepare(
                        files,
                        &sunrise_world::conditions::LoadConditions::vanilla(),
                        &load_context(),
                        &data,
                    )
                    .unwrap();
                data.set_global_modifiers(generation);

                let mut expected_rng = LegacyRandom::new(31);
                expected_rng.next_long();
                expected_rng.next_long();
                expected_rng.next_long();

                let mut world_rng = LegacyRandom::new(31);
                let world_out = data.roll_split("test:root", &mut world_rng);
                assert_eq!(
                    world_out
                        .iter()
                        .map(|s| (s.item.as_str(), s.count))
                        .collect::<Vec<_>>(),
                    [
                        ("minecraft:diamond", -3),
                        ("minecraft:gold_ingot", 200),
                        ("minecraft:gold_ingot", 205),
                    ]
                );
                assert_eq!(
                    world_out[0].components.last(),
                    Some(&("minecraft:custom_data".into(), Some(Tag::Int(42))))
                );
                assert_eq!(
                    <(u64, Option<f64>)>::from(&world_rng),
                    <(u64, Option<f64>)>::from(&expected_rng)
                );

                let mut items = sunrise_items::items::Items::for_tests(HashMap::new());
                items.set_loot_data(&data);
                let mut items_rng = LegacyRandom::new(31);
                let items_out = items.loot_table("test:root", false, None, &mut items_rng);
                assert_eq!(
                    items_out
                        .iter()
                        .map(|s| (&*s.item, s.count))
                        .collect::<Vec<_>>(),
                    [
                        ("minecraft:diamond", 0),
                        ("minecraft:gold_ingot", 200),
                        ("minecraft:gold_ingot", 205),
                    ]
                );
                assert_eq!(
                    <(u64, Option<f64>)>::from(&items_rng),
                    <(u64, Option<f64>)>::from(&expected_rng)
                );

                // The retained ApiAddTable instance uses the same engine append_raw path.
                let api_add = manager
                    .get(&Identifier::parse("chain:add").unwrap())
                    .unwrap();
                let api_generation = Arc::new(GlobalLootModifiers::load(
                    [modifier_file(
                        "chain:api_add",
                        json!({"type": "chain:api_add"}),
                    )],
                    &sunrise_world::conditions::LoadConditions::vanilla(),
                    |_, _, _| {
                        Ok(DecodedModifier {
                            priority: 1000,
                            condition: None,
                            apply: Arc::new(Adapter(api_add.clone())),
                        })
                    },
                    |error| panic!("{error}"),
                ));
                data.set_global_modifiers(api_generation);
                let api_out = data.roll_split("test:root", &mut LegacyRandom::new(31));
                assert_eq!(
                    api_out
                        .iter()
                        .map(|s| (s.item.as_str(), s.count))
                        .collect::<Vec<_>>(),
                    [
                        ("minecraft:diamond", 64),
                        ("minecraft:diamond", 64),
                        ("minecraft:diamond", 2),
                        ("minecraft:gold_ingot", 200),
                    ]
                );
            },
        );
    }

    #[derive(Clone)]
    struct PanicGlmMod {
        decode_calls: Arc<AtomicUsize>,
        apply_calls: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for PanicGlmMod {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.register_global_loot_modifier_serializer("boom", self.clone());
        }
    }
    impl GlobalLootModifierSerializer for PanicGlmMod {
        fn decode(
            &self,
            _: &serde_json::Value,
            _: &dyn DataLoadContext,
        ) -> Result<Arc<dyn GlobalLootModifier>, LootModifierError> {
            self.decode_calls.fetch_add(1, Ordering::Relaxed);
            Ok(Arc::new(self.clone()))
        }
    }
    impl GlobalLootModifier for PanicGlmMod {
        fn apply(
            &self,
            _: &mut dyn LootModifierContext,
            _: &mut LootList<'_>,
        ) -> Result<(), LootModifierError> {
            self.apply_calls.fetch_add(1, Ordering::Relaxed);
            panic!("glm registration panic");
        }
    }

    #[derive(Clone)]
    struct HealthyGlmMod(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for HealthyGlmMod {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.register_global_loot_modifier_serializer("bump", self.clone());
        }
    }
    impl GlobalLootModifierSerializer for HealthyGlmMod {
        fn decode(
            &self,
            _: &serde_json::Value,
            _: &dyn DataLoadContext,
        ) -> Result<Arc<dyn GlobalLootModifier>, LootModifierError> {
            Ok(Arc::new(self.clone()))
        }
    }
    impl GlobalLootModifier for HealthyGlmMod {
        fn apply(
            &self,
            _: &mut dyn LootModifierContext,
            generated: &mut LootList<'_>,
        ) -> Result<(), LootModifierError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            if let Some(mut first) = generated.get_mut(0) {
                let next = first.view().count() + 10;
                first.set_count(next);
            }
            Ok(())
        }
    }

    #[test]
    fn panicking_global_modifier_stays_poisoned_across_reloads_while_others_run() {
        let defaults = sunrise_world::item_defaults::ItemDefaults::decode(
            br#"{"minecraft:stick":{"minecraft:max_stack_size":64}}"#,
        )
        .unwrap();
        let table = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:stick"}]}]});
        let overlay = [(
            "loot_table/test:root".to_owned(),
            vec![serde_json::to_vec(&table).unwrap()],
        )];
        let panic_decodes = Arc::new(AtomicUsize::new(0));
        let panic_applies = Arc::new(AtomicUsize::new(0));
        let healthy_applies = Arc::new(AtomicUsize::new(0));

        mod_platform::testing::with_mods(
            vec![
                ModEntry::new(
                    "panicglm",
                    Box::new(PanicGlmMod {
                        decode_calls: panic_decodes.clone(),
                        apply_calls: panic_applies.clone(),
                    }),
                ),
                ModEntry::new("goodglm", Box::new(HealthyGlmMod(healthy_applies.clone()))),
            ]
            .into(),
            || {
                mod_platform::__private::part_tests::install_parts_fixture(
                    mod_api::__private::mods(),
                )
                .unwrap();
                let mut data = loot::LootData::load_with_types(
                    Path::new("scoped-glm-panic-overlay"),
                    &loot::PackOverlay::new(&overlay),
                    &defaults,
                    loot_types::capture(),
                )
                .unwrap();
                let (generation, _) = Serializers::capture()
                    .prepare(
                        vec![
                            modifier_file(
                                "panicglm:a",
                                json!({"type": "panicglm:boom", "priority": 1002}),
                            ),
                            modifier_file(
                                "panicglm:b",
                                json!({"type": "panicglm:boom", "priority": 1001}),
                            ),
                            modifier_file(
                                "goodglm:add",
                                json!({"type": "goodglm:bump", "priority": 1000}),
                            ),
                        ],
                        &sunrise_world::conditions::LoadConditions::vanilla(),
                        &load_context(),
                        &data,
                    )
                    .unwrap();
                assert_eq!(panic_decodes.load(Ordering::Relaxed), 2);
                data.set_global_modifiers(generation);

                // First roll panics in panicglm:a, poisons the shared registration owner so
                // panicglm:b does not run, while goodglm:add still runs.
                let first = data.roll_split("test:root", &mut LegacyRandom::new(1));
                assert_eq!(first[0].count, 11);
                assert_eq!(panic_applies.load(Ordering::Relaxed), 1);
                assert_eq!(healthy_applies.load(Ordering::Relaxed), 1);

                // Recapturing serializers on reload preserves the poisoned owner:
                // panicglm:boom is skipped without invoking decode, goodglm:add loads and runs.
                let (reloaded, manager) = Serializers::capture()
                    .prepare(
                        vec![
                            modifier_file(
                                "panicglm:a",
                                json!({"type": "panicglm:boom", "priority": 1002}),
                            ),
                            modifier_file(
                                "goodglm:add",
                                json!({"type": "goodglm:bump", "priority": 1000}),
                            ),
                        ],
                        &sunrise_world::conditions::LoadConditions::vanilla(),
                        &load_context(),
                        &data,
                    )
                    .unwrap();
                assert_eq!(panic_decodes.load(Ordering::Relaxed), 2);
                assert_eq!(manager.modifiers().len(), 1);
                data.set_global_modifiers(reloaded);
                let second = data.roll_split("test:root", &mut LegacyRandom::new(1));
                assert_eq!(second[0].count, 11);
                assert_eq!(panic_applies.load(Ordering::Relaxed), 1);
                assert_eq!(healthy_applies.load(Ordering::Relaxed), 2);
            },
        );
    }
}
