use mod_api::__private::{game::RegistrationContextNew, reload::ReloadStageError};
use mod_api::{
    ConditionError, Hooks, Identifier, LoadCondition, ModEntry, RegisterLoadConditionsEvent,
    RegistrationContext,
};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;
struct Flag(Arc<AtomicUsize>, Arc<AtomicUsize>);
impl LoadCondition for Flag {
    fn test(&self, args: &Value) -> std::result::Result<bool, ConditionError> {
        assert!(args.get("type").is_none());
        self.0.fetch_add(1, Ordering::SeqCst);
        if args["value"] == "content" {
            return Ok(self.1.load(Ordering::SeqCst) == 0);
        }
        if args["value"] == "panic" {
            panic!("fixture panic");
        }
        args["value"].as_bool().ok_or_else(|| ConditionError {
            message: "value must be a boolean".into(),
        })
    }
}
struct Registrant {
    pub count: Arc<AtomicUsize>,
    mode: Arc<AtomicUsize>,
}
#[mod_api::__subscriptions]
impl Hooks for Registrant {
    fn register_load_conditions(
        &self,
        _: &RegistrationContext,
        event: &mut RegisterLoadConditionsEvent<'_>,
    ) {
        event
            .conditions
            .register(
                Identifier::parse("testmod:flag").unwrap(),
                Flag(self.count.clone(), self.mode.clone()),
            )
            .unwrap();
    }
    fn data_pack_files(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::DataPackFilesEvent<'_>,
    ) {
        let yes = match self.mode.load(Ordering::SeqCst) {
            1 => json!({"type":"testmod:nope"}),
            2 => json!({"type":"testmod:flag","value":"bad"}),
            _ => json!({"type":"testmod:flag","value":true}),
        };
        if self.mode.load(Ordering::SeqCst) == 5 {
            event.files.push((
                "advancement/testmod:bad".into(),
                br#"{"neoforge:conditions":[{"type":"testmod:nope"}]}"#.to_vec(),
            ));
        }
        for (name, condition) in [
            ("enabled", yes),
            ("disabled", json!({"type":"testmod:flag","value":false})),
            (
                "not_false",
                json!({"type":"neoforge:not","value":{"type":"testmod:flag","value":false}}),
            ),
            (
                "not_true",
                json!({"type":"neoforge:not","value":{"type":"testmod:flag","value":true}}),
            ),
            (
                "nested",
                json!({"type":"neoforge:and","values":[{"type":"neoforge:always"},{"type":"neoforge:or","values":[{"type":"testmod:flag","value":true}]}]}),
            ),
        ] {
            let mut recipe = json!({
                "type":"minecraft:crafting_shapeless", "ingredients":["minecraft:flint"],
                "result":{"id":"minecraft:stick"}, "neoforge:conditions":[condition]
            });
            if name == "enabled" {
                match self.mode.load(Ordering::SeqCst) {
                    6 => recipe["result"] = Value::Null,
                    7 => recipe["ingredients"] = json!([{"type":"testmod:unregistered"}]),
                    8 => {
                        recipe["type"] = json!("minecraft:stonecutting");
                        recipe["ingredient"] = json!(42);
                    }
                    _ => {}
                }
            }
            event.files.push((
                format!("recipe/testmod:{name}"),
                serde_json::to_vec(&recipe).unwrap(),
            ));
        }
    }
}
#[test]
#[ignore = "daily"]
fn malformed_known_recipe_stops_dedicated_world_startup() {
    let scratch = tempfile::tempdir().unwrap();
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "testmod",
            Box::new(Registrant {
                count: Arc::new(AtomicUsize::new(0)),
                mode: Arc::new(AtomicUsize::new(6)),
            }),
        )]
        .into(),
        || {
            // The world-open consumer used by dedicated startup must stop
            // before publishing any candidate recipes.
            sunrise_gameplay::game::native_entity_refs::tests::with_game_role(
                std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap()),
                sunrise_gameplay_common::game::execution_role::ExecutionRole::DedicatedServer,
                |game| {
                    let mut request = sunrise_gameplay::game::world_loading::WorldRequest::new(
                        "bad-recipes",
                        Some("test"),
                        Some(1),
                        true,
                        None,
                        Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla()),
                    )
                    .unwrap();
                    request.save_root = scratch.path().join("refused-startup");
                    let error = game.load_world(request).err().unwrap().to_string();
                    assert!(
                        error.contains("data/testmod/recipe/enabled.json"),
                        "{error}"
                    );
                    assert!(error.contains("invalid result"), "{error}");
                    assert!(game.recipes().recipe("testmod:enabled").is_none());
                },
            );
        },
    );
}

#[test]
#[ignore = "daily"]
fn load_conditions_recipe_reload_counts_and_failed_candidates_keep_old_recipes() {
    if run_fresh_cases(
        "cut_layout::game::load_conditions::tests::load_conditions_recipe_reload_counts_and_failed_candidates_keep_old_recipes",
        "SUNRISE_LOAD_CONDITIONS_RELOAD_CHILD",
        &["reload"],
    ) {
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let mode = Arc::new(AtomicUsize::new(0));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "testmod",
            Box::new(Registrant {
                count: count.clone(),
                mode: mode.clone(),
            }),
        )]
        .into(),
        || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let mut request = sunrise_gameplay::game::world_loading::WorldRequest::new(
                    "conditions",
                    Some("test"),
                    Some(1),
                    true,
                    None,
                    Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla()),
                )
                .unwrap();
                request.save_root = scratch.path().join("save");
                let module = scratch.path().join("content");
                std::fs::create_dir_all(module.join("data/testmod/tags/block")).unwrap();
                std::fs::write(module.join("sunrise-mod.toml"), "id='testmod'\nname='Conditions fixture'\nversion='1'\napi='0.2'\nside='both'\ncapabilities=['registries']\n").unwrap();
                std::fs::write(module.join("data/testmod/tags/block/flag.json"), r#"{"neoforge:conditions":[{"type":"testmod:flag","value":"content"}],"values":["minecraft:stone"]}"#).unwrap();
                // Tags deliberately ignore load conditions. A recipe exercises
                // the supported content-file gate alongside the five native files.
                std::fs::create_dir_all(module.join("data/testmod/recipe")).unwrap();
                std::fs::write(module.join("data/testmod/recipe/content.json"), r#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:flint"],"result":{"id":"minecraft:stick"},"neoforge:conditions":[{"type":"testmod:flag","value":"content"}]}"#).unwrap();
                game.mods.selected = vec![sunrise_mod_package::selection::SelectedPackage {
                    package: sunrise_mod_package::Package::read(&module).unwrap(),
                    path: module.clone(),
                }]
                .into();
                game.mods.dev.push(module);
                game.load_world(request).unwrap();
                assert!(game.items().block_tags["testmod:flag"].contains("minecraft:stone"));
                assert!(game.recipes().recipe("testmod:content").is_some());
                assert!(game.recipes().recipe("testmod:enabled").is_some());
                assert!(game.recipes().recipe("testmod:disabled").is_none());
                assert!(game.recipes().recipe("testmod:nested").is_some());
                for _ in 0..2 {
                    count.store(0, Ordering::SeqCst);
                    let stack = game
                        .reload_datapacks(true, &game.level.enabled_features.clone())
                        .unwrap();
                    assert_eq!(count.load(Ordering::SeqCst), 6);
                    assert!(game.recipes().recipe("testmod:content").is_some());
                    assert!(game.recipes().recipe("testmod:not_false").is_some());
                    assert!(game.recipes().recipe("testmod:not_true").is_none());
                    assert!(game.recipes().recipe("testmod:nested").is_some());
                    let conditional_recipe =
                        json!({"neoforge:conditions":[{"type":"testmod:flag","value":"content"}]});
                    mode.store(4, Ordering::SeqCst);
                    game.prepare_pack_condition_reload(&stack).unwrap();
                    let attempt = game.mods.load_condition_candidate.as_ref().unwrap();
                    assert!(
                        !attempt
                            .allows_file(&conditional_recipe, "data/testmod/recipe/content.json")
                    );
                    assert!(!attempt.allows_file(
                        &json!({"neoforge:conditions":[{"type":"testmod:nope"}]}),
                        "failed-pack.json"
                    ));
                    assert!(attempt.check_errors().is_err());
                    mode.store(0, Ordering::SeqCst);
                    game.prepare_pack_condition_reload(&stack).unwrap();
                    let recovered = game.mods.load_condition_candidate.as_ref().unwrap();
                    assert_eq!(recovered.check_errors(), Ok(()));
                    assert!(
                        recovered
                            .allows_file(&conditional_recipe, "data/testmod/recipe/content.json")
                    );
                }
                mode.store(4, Ordering::SeqCst);
                count.store(0, Ordering::SeqCst);
                game.reload_datapacks(true, &game.level.enabled_features.clone())
                    .unwrap();
                assert_eq!(count.load(Ordering::SeqCst), 6);
                assert!(
                    game.items()
                        .block_tags
                        .get("testmod:flag")
                        .is_some_and(|tag| tag.contains("minecraft:stone"))
                );

                assert!(game.recipes().recipe("testmod:content").is_none());
                let old: Vec<_> = game
                    .recipes()
                    .crafting
                    .iter()
                    .map(|(id, _)| id.clone())
                    .collect();
                let old_generation = game.recipes().by_type["minecraft:crafting"].clone();
                let grid = [Some(sunrise_items::items::ItemStack::new(
                    "minecraft:flint",
                    1,
                ))];
                for (bad, expected) in [
                    (1, "testmod:nope"),
                    (2, "value must be a boolean"),
                    (6, "invalid result"),
                    (7, "ingredient kind testmod:unregistered"),
                    (8, "invalid ingredient: 42"),
                ] {
                    mode.store(bad, Ordering::SeqCst);
                    let error = game
                        .reload_datapacks(true, &game.level.enabled_features.clone())
                        .err()
                        .unwrap();
                    let error = match (bad, error) {
                        (8, ReloadStageError::DataSource(message)) => message,
                        (8, other) => panic!("expected a recipe data-source error, got {other:?}"),
                        (_, other) => format!("{other:?}"),
                    };
                    assert!(
                        error.contains("data/testmod/recipe/enabled.json"),
                        "{error}"
                    );
                    assert!(error.contains(expected), "{error}");
                    if bad == 2 {
                        assert!(
                            error.contains("condition testmod:flag: mod testmod"),
                            "{error}"
                        );
                    }
                    assert_eq!(
                        game.recipes()
                            .crafting
                            .iter()
                            .map(|(id, _)| id.clone())
                            .collect::<Vec<_>>(),
                        old
                    );
                    assert!(game.recipes().recipe("testmod:enabled").is_some());
                    assert!(Arc::ptr_eq(
                        &old_generation,
                        &game.recipes().by_type["minecraft:crafting"],
                    ));
                    assert!(game.recipes().recipe_matches(
                        &game.loop_context,
                        "testmod:enabled",
                        &grid,
                        1,
                    ));
                    assert_eq!(
                        game.recipes()
                            .craft(&game.loop_context, &grid, 1)
                            .unwrap()
                            .item
                            .as_ref(),
                        "minecraft:stick",
                    );
                }
                let host = game.server.lan.take();
                sunrise_gameplay::game::lan::sync::forget_sync_data();
                assert!(sunrise_gameplay::game::lan::sync::cached_reload_sync().is_none());
                mode.store(5, Ordering::SeqCst);
                let error = format!(
                    "{:?}",
                    game.reload_datapacks(true, &game.level.enabled_features.clone())
                        .err()
                        .unwrap()
                );
                assert!(
                    error.contains("data/testmod/advancement/bad.json"),
                    "{error}"
                );
                assert_eq!(
                    game.recipes()
                        .crafting
                        .iter()
                        .map(|(id, _)| id.clone())
                        .collect::<Vec<_>>(),
                    old
                );
                game.server.lan = host;
                mode.store(0, Ordering::SeqCst);
                game.reload_datapacks(true, &game.level.enabled_features.clone())
                    .unwrap();
                assert!(game.recipes().recipe("testmod:enabled").is_some());
                assert!(game.recipes().recipe("testmod:content").is_some());
                assert!(!Arc::ptr_eq(
                    &old_generation,
                    &game.recipes().by_type["minecraft:crafting"],
                ));
            });
        },
    );
}
struct NamedCondition(&'static str);
#[mod_api::__subscriptions]
impl Hooks for NamedCondition {
    fn register_load_conditions(
        &self,
        _: &RegistrationContext,
        event: &mut RegisterLoadConditionsEvent<'_>,
    ) {
        let _ = event.conditions.register(
            Identifier::parse(self.0).unwrap(),
            Flag(Arc::default(), Arc::default()),
        );
    }
}
#[test]
fn load_conditions_registration_allows_shared_namespaces() {
    mod_platform::testing::with_mods(
        vec![
            ModEntry::new("a", Box::new(NamedCondition("a:y"))),
            ModEntry::new("b", Box::new(NamedCondition("a:x"))),
        ]
        .into(),
        || {
            let mut registrar = mod_api::LoadConditionDeclarations::new(
                sunrise_world::conditions::is_builtin_condition,
            );
            mod_api::dispatch_register_load_conditions(&RegistrationContext::new(), |listen| {
                registrar.register_listener(listen)
            });
            let entries = registrar.finish().unwrap();
            assert_eq!(entries.len(), 2);
            for (name, owner, value) in [("a:y", "a", true), ("a:x", "b", false)] {
                let (registered_owner, handler) =
                    entries.get(&Identifier::parse(name).unwrap()).unwrap();
                assert_eq!(*registered_owner, owner);
                assert_eq!(handler.test(&json!({"value": value})), Ok(value));
            }
        },
    );
}

#[test]
fn load_conditions_registration_refuses_duplicate_ids_even_if_ignored() {
    mod_platform::testing::with_mods(
        vec![
            ModEntry::new("a", Box::new(NamedCondition("a:x"))),
            ModEntry::new("b", Box::new(NamedCondition("a:x"))),
        ]
        .into(),
        || {
            let mut registrar = mod_api::LoadConditionDeclarations::new(
                sunrise_world::conditions::is_builtin_condition,
            );
            mod_api::dispatch_register_load_conditions(&RegistrationContext::new(), |listen| {
                registrar.register_listener(listen)
            });
            let error = registrar.finish().err().unwrap().message;
            assert!(error.contains("condition a:x"), "{error}");
            assert!(error.contains("both a and b"), "{error}");
        },
    );
}

#[test]
fn a_panicking_load_condition_registrar_leaves_neither_handlers_nor_errors() {
    use std::sync::atomic::AtomicBool;

    struct Failing(Arc<AtomicBool>);
    #[mod_api::__subscriptions]
    impl Hooks for Failing {
        fn register_load_conditions(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterLoadConditionsEvent<'_>,
        ) {
            let handler = || Flag(Arc::default(), Arc::default());
            let shared = Identifier::parse("a:shared").unwrap();
            event
                .conditions
                .register(shared.clone(), handler())
                .unwrap();
            event
                .conditions
                .register(Identifier::parse("a:leaked").unwrap(), handler())
                .unwrap();
            assert_eq!(
                event
                    .conditions
                    .register(shared, handler())
                    .unwrap_err()
                    .message,
                "condition a:shared registered by both a and a",
            );
            self.0.store(true, Ordering::Relaxed);
            panic!("load condition registrar fixture failed");
        }
    }

    let reached = Arc::new(AtomicBool::new(false));
    mod_platform::testing::with_mods(
        vec![
            ModEntry::new("a", Box::new(Failing(reached.clone()))),
            ModEntry::new("b", Box::new(NamedCondition("a:shared"))),
        ]
        .into(),
        || {
            let mut registrar = mod_api::LoadConditionDeclarations::new(
                sunrise_world::conditions::is_builtin_condition,
            );
            sunrise_gameplay_common::game::native_hooks::native_register_load_conditions(
                &RegistrationContext::new(),
                &mut registrar,
            );
            assert!(reached.load(Ordering::Relaxed));
            let entries = registrar.finish().unwrap();
            assert_eq!(entries.len(), 1);
            let (owner, handler) = &entries[&Identifier::parse("a:shared").unwrap()];
            assert_eq!(*owner, "b");
            assert_eq!(handler.test(&json!({"value": false})), Ok(false));
        },
    );
}

#[test]
fn load_conditions_refuse_a_builtin_id_at_startup_even_if_ignored() {
    for name in ["neoforge:always", "neoforge:never", "neoforge:mod_loaded"] {
        mod_platform::testing::with_mods(
            vec![ModEntry::new("compat", Box::new(NamedCondition(name)))].into(),
            || {
                let sources = sunrise_gameplay_common::game::mods::ModsState::default();
                let inventory = sunrise_mod_package::enabled::EnabledMods::default();
                let error = sunrise_gameplay_common::game::load_conditions::conditions_for_mods(
                    &sources, &inventory,
                )
                .unwrap_err();
                assert_eq!(
                    error,
                    format!("mod compat cannot register condition {name}: that ID is built in")
                );
            },
        );
    }
}

#[test]
fn load_conditions_admit_registered_foreign_ids_but_refuse_every_unknown_id() {
    mod_platform::testing::with_mods(
        vec![
            ModEntry::new("a", Box::new(NamedCondition("a:x"))),
            ModEntry::new("b", Box::new(NamedCondition("a:z"))),
            ModEntry::new("compat", Box::new(NamedCondition("c:config"))),
            ModEntry::new("other", Box::new(NamedCondition("neoforge:custom"))),
        ]
        .into(),
        || {
            let sources = sunrise_gameplay_common::game::mods::ModsState::default();
            let inventory = sunrise_mod_package::enabled::EnabledMods::default();
            let conditions = sunrise_gameplay_common::game::load_conditions::conditions_for_mods(
                &sources, &inventory,
            )
            .unwrap();
            let file = |id: &str| json!({"neoforge:conditions":[{"type":id, "value":true}]});
            assert!(conditions.allows_file(&file("c:config"), "data/demo/advancement/known.json"));
            assert!(conditions.allows_file(
                &file("neoforge:custom"),
                "data/demo/advancement/custom.json"
            ));
            for namespace in ["a", "c", "neoforge"] {
                let id = format!("{namespace}:missing");
                let path = format!("data/demo/advancement/{namespace}.json");
                assert!(!conditions.allows_file(&file(&id), &path));
                let error = conditions.check_errors().unwrap_err();
                assert!(
                    error.contains(&format!("{path}: unknown load conditions [{id}]")),
                    "{error}"
                );
            }
        },
    );
}

#[test]
fn load_conditions_disabled_owner_does_not_execute_registered_handler() {
    let count = Arc::new(AtomicUsize::new(0));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "testmod",
            Box::new(Registrant {
                count: count.clone(),
                mode: Arc::default(),
            }),
        )]
        .into(),
        || {
            let sources = sunrise_gameplay_common::game::mods::ModsState::default();
            let inventory = sunrise_mod_package::enabled::EnabledMods::default();
            let file = json!({"neoforge:conditions":[{"type":"testmod:flag","value":true}]});
            assert!(
                sunrise_gameplay_common::game::load_conditions::conditions_for_mods(
                    &sources, &inventory
                )
                .unwrap()
                .allows_file(&file, "flag.json")
            );
            assert!(mod_api::__private::set_mod_enabled("testmod", false));
            let disabled = sunrise_gameplay_common::game::load_conditions::conditions_for_mods(
                &sources, &inventory,
            )
            .unwrap();
            assert!(!disabled.allows_file(&file, "flag.json"));
            assert_eq!(disabled.check_errors(), Ok(()));
            assert_eq!(count.load(Ordering::SeqCst), 1);
            assert!(mod_api::__private::set_mod_enabled("testmod", true));
            assert!(
                sunrise_gameplay_common::game::load_conditions::conditions_for_mods(
                    &sources, &inventory
                )
                .unwrap()
                .allows_file(&file, "flag.json")
            );
            assert_eq!(count.load(Ordering::SeqCst), 2);
        },
    );
}

#[test]
fn load_condition_panic_disables_only_that_handler_across_generations() {
    use mod_api::__private::{HookPanicReport, take_hook_panics};

    struct Pair(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for Pair {
        fn register_load_conditions(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterLoadConditionsEvent<'_>,
        ) {
            for (id, count) in [
                ("foreign:faulty", self.0.clone()),
                ("foreign:healthy", Arc::default()),
            ] {
                event
                    .conditions
                    .register(Identifier::parse(id).unwrap(), Flag(count, Arc::default()))
                    .unwrap();
            }
        }
    }

    let count = Arc::new(AtomicUsize::new(0));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "condition_pair",
            Box::new(Pair(count.clone())),
        )]
        .into(),
        || {
            let sources = sunrise_gameplay_common::game::mods::ModsState::default();
            let inventory = sunrise_mod_package::enabled::EnabledMods::default();
            let load = sunrise_gameplay_common::game::load_conditions::conditions_for_mods(
                &sources, &inventory,
            )
            .unwrap();
            let faulty = json!({"neoforge:conditions":[{"type":"foreign:faulty","value":"panic"}]});
            let path = "data/foreign/recipe/faulty.json";
            assert!(!load.allows_file(&faulty, path));
            let error = load.check_errors().unwrap_err();
            assert!(
                error.contains("data/foreign/recipe/faulty.json: condition foreign:faulty: mod condition_pair: load condition callback panicked")
                    && error.contains("fixture panic"),
                "{error}"
            );
            assert_eq!(
                take_hook_panics(),
                [HookPanicReport {
                    mod_id: "condition_pair",
                    hook: "test_load_condition",
                }]
            );

            let next = sunrise_gameplay_common::game::load_conditions::conditions_for_mods(
                &sources, &inventory,
            )
            .unwrap();
            let error = next.allows_file_isolated(&faulty, path).unwrap_err();
            assert_eq!(
                error,
                "condition foreign:faulty: mod condition_pair: load condition owner is disabled"
            );
            assert_eq!(count.load(Ordering::SeqCst), 1);
            assert!(take_hook_panics().is_empty());

            let healthy = json!({"neoforge:conditions":[{"type":"foreign:healthy","value":true}]});
            assert_eq!(
                next.allows_file_isolated(&healthy, "healthy.json"),
                Ok(true)
            );
        },
    );
}

/// Only decoded holders (not file names or the previous load's IDs) count.
#[test]
fn registered_reads_the_candidate_packs_not_the_last_load() {
    use mod_api::{
        DatapackField, DatapackFieldKind, DatapackRegistryDeclaration, DatapackRegistrySync,
    };
    use sunrise_assets::resource_source::ResourceSource;
    use sunrise_gameplay_common::game::load_conditions::{
        RegistryView, admit_datapack_registries_for_test as admit_datapack_registries,
    };
    use sunrise_level::datapack::{DataPack, DataPacks, DataStore, RegistryFiles, registry_files};
    use sunrise_world::{conditions::LoadConditions, loot::PackOverlay};

    let root = tempfile::tempdir().unwrap();
    let keys = [
        "test:a_downstream",
        "test:b_right",
        "test:c_left",
        "test:d_first",
        "test:e_later",
    ];
    let declarations = keys.map(|key| DatapackRegistryDeclaration {
        mod_id: "test".into(),
        key: key.into(),
        fields: vec![DatapackField::required("enabled", DatapackFieldKind::Bool)],
        sync: DatapackRegistrySync::ServerOnly,
    });
    let registered = |registry: &str, value: &str| json!({"type": "neoforge:registered", "registry": registry, "value": value});
    let bound = json!({"type":"neoforge:tag_empty","registry":"test:b_right","tag":"test:bound"});
    let write = |layer: &str, file: &str, json: Value| {
        let path = root.path().join(layer).join("data/test").join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, json.to_string()).unwrap();
    };
    write(
        "base",
        "test/a_downstream/off.json",
        json!({"enabled":true}),
    );
    write(
        "top",
        "test/a_downstream/off.json",
        json!({
            "enabled":true, "neoforge:conditions":[{"type":"neoforge:never"}]
        }),
    );
    write(
        "top",
        "test/a_downstream/answer.json",
        json!({
            "enabled":true, "neoforge:conditions":[
                registered("test:c_left", "test:seed"),
                {"type":"neoforge:not","value":bound.clone()},
                registered("minecraft:item", "minecraft:stone")
            ]
        }),
    );
    write(
        "top",
        "test/c_left/seed.json",
        json!({
            "enabled":true, "neoforge:conditions":[bound]
        }),
    );
    write(
        "top",
        "test/c_left/self.json",
        json!({
            "enabled":true, "neoforge:conditions":[registered("test:c_left", "test:seed")]
        }),
    );
    write(
        "top",
        "test/b_right/edge.json",
        json!({
            "enabled":true, "neoforge:conditions":[registered("test:c_left", "test:seed")]
        }),
    );
    write("top", "test/b_right/holder.json", json!({"enabled":true}));
    write(
        "top",
        "tags/test/b_right/bound.json",
        json!({"values":["test:holder"]}),
    );
    write(
        "top",
        "test/d_first/first.json",
        json!({
            "enabled":true, "neoforge:conditions":[{"type":"neoforge:or", "values":[
                {"type":"neoforge:always"}, registered("test:e_later", "test:later")
            ]}]
        }),
    );
    write(
        "top",
        "test/e_later/later.json",
        json!({
            "enabled":true, "neoforge:conditions":[
                {"type":"test:counter"}, registered("test:d_first", "test:first")
            ]
        }),
    );
    let packs = ["base", "top"].map(|layer| {
        DataPack::from_source(
            format!("file/{layer}"),
            ResourceSource::directory(root.path().join(layer)),
        )
    });
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let base = LoadConditions::new([])
        .with_features(["minecraft:vanilla"])
        .with_lookup([("test:counter", move |_: &Value| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(true)
        })]);
    let tags: Vec<_> = DataStore::load(&DataPacks::new(&[&packs[0], &packs[1]]))
        .files
        .into_iter()
        .collect();
    let overlay = PackOverlay {
        conditions: &base,
        files: &tags,
        ..PackOverlay::default()
    };
    let view = || RegistryView {
        ids: [("minecraft:item".into(), ["minecraft:stone".into()].into())].into(),
        ..RegistryView::default()
    };
    let mut first = view();
    let winners = registry_files(&[&packs[0], &packs[1]], &keys).unwrap();
    let admitted = admit_datapack_registries(
        root.path(),
        &overlay,
        &mut first,
        &declarations,
        &winners,
        &mut mod_api::__private::NoVanillaValueDecoder,
    )
    .unwrap();
    assert_eq!(admitted.entries("test:c_left").unwrap(), ["test:seed"]);
    assert_eq!(admitted.entries("test:b_right").unwrap(), ["test:holder"]);
    assert_eq!(admitted.entries("test:d_first").unwrap(), ["test:first"]);
    assert_eq!(admitted.entries("test:e_later").unwrap(), ["test:later"]);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        admitted.entries("test:a_downstream").unwrap(),
        ["test:answer"]
    );

    let test = |registry, value| json!({"neoforge:conditions":[registered(registry, value)]});
    assert!(
        first
            .conditions(&base)
            .allows(&test("test:a_downstream", "test:answer"))
    );
    let mut next = view();
    admit_datapack_registries(
        root.path(),
        &overlay,
        &mut next,
        &declarations,
        &RegistryFiles::new(),
        &mut mod_api::__private::NoVanillaValueDecoder,
    )
    .unwrap();
    let next = next.conditions(&base);
    assert!(!next.allows(&test("test:a_downstream", "test:answer")));
    assert!(next.allows(&test("minecraft:item", "minecraft:stone")));

    // Changing only the top file to an unknown condition is an owning-loader
    // error, never permission to restore the valid lower file.
    write(
        "top",
        "test/a_downstream/off.json",
        json!({
            "enabled":true, "neoforge:conditions":[{"type":"test:unknown"}]
        }),
    );
    let winners = registry_files(&[&packs[0], &packs[1]], &keys).unwrap();
    let mut refused = view();
    let error = admit_datapack_registries(
        root.path(),
        &overlay,
        &mut refused,
        &declarations,
        &winners,
        &mut mod_api::__private::NoVanillaValueDecoder,
    )
    .unwrap_err();
    assert!(
        error.contains("pack file/top, file data/test/test/a_downstream/off.json"),
        "{error}"
    );
    assert!(error.contains("test:unknown"), "{error}");
    assert!(
        !refused
            .conditions(&base)
            .allows(&test("test:a_downstream", "test:off"))
    );
}

#[test]
fn unsynced_registry_admits_conditional_payloads_and_tags_without_worldgen_limit() {
    use mod_api::{
        DatapackField, DatapackFieldKind, DatapackRegistryDeclaration, DatapackRegistrySet,
        DatapackRegistrySync, DatapackValue,
    };
    use sunrise_gameplay_common::game::load_conditions::{
        RegistryView, admit_datapack_registries_for_test as admit_datapack_registries,
    };
    use sunrise_world::loot::PackOverlay;
    let root = tempfile::tempdir().unwrap();
    let declaration = DatapackRegistryDeclaration {
        mod_id: "test".into(),
        key: "test:spell".into(),
        sync: DatapackRegistrySync::ServerOnly,
        fields: vec![
            DatapackField::required("enabled", DatapackFieldKind::Bool),
            DatapackField::with_default(
                "neoforge:conditions",
                DatapackFieldKind::Bool,
                DatapackValue::Bool(true),
            ),
        ],
    };
    assert!(
        declaration
            .decode_entry(r#"{"neoforge:value":{"enabled":true}}"#)
            .is_err()
    );
    let winners = [
        (
            "inline",
            br#"{"enabled":true,"neoforge:conditions":[{"type":"neoforge:always"}]}"#.as_slice(),
        ),
        (
            "wrapped",
            br#"{"neoforge:value":{"enabled":false,"neoforge:conditions":false},"neoforge:conditions":[]}"#
                .as_slice(),
        ),
    ]
    .into_iter()
    .map(|(id, bytes)| {
        (
            format!("data/test/test/spell/{id}.json"),
            ("top".to_owned(), bytes.to_vec()),
        )
    })
    .collect();
    let tagged = (0..8193)
        .map(|index| {
            (
                format!("tags/test:test/spell/t{index}"),
                vec![br#"{"values":["test:inline"]}"#.to_vec()],
            )
        })
        .collect::<Vec<_>>();
    let mut view = RegistryView::default();
    let admitted = admit_datapack_registries(
        root.path(),
        &PackOverlay::new(&tagged),
        &mut view,
        &[declaration],
        &winners,
        &mut mod_api::__private::NoVanillaValueDecoder,
    )
    .unwrap();
    let set = DatapackRegistrySet::from_candidate(admitted);
    assert_eq!(
        set.get("test:spell", "test:inline")
            .unwrap()
            .get("neoforge:conditions"),
        Some(&DatapackValue::Bool(true))
    );
    assert_eq!(
        set.get("test:spell", "test:wrapped").unwrap().values,
        [
            ("enabled".into(), DatapackValue::Bool(false)),
            ("neoforge:conditions".into(), DatapackValue::Bool(false)),
        ]
    );
    assert_eq!(view.tags["test:spell"].len(), 8193);
    assert_eq!(view.tags["test:spell"].get("test:t8192"), Some(&true));
}

#[test]
#[ignore = "daily"]
fn first_open_conditions_see_static_tags_and_admitted_biomes() {
    use std::fs;
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_sim::lan::data::BIOME_REGISTRY;

    // World opening installs part/entity/particle registries and the LAN sync cache.
    if run_fresh_cases(
        "cut_layout::game::load_conditions::tests::first_open_conditions_see_static_tags_and_admitted_biomes",
        "SUNRISE_BIOME_ADMISSION_CHILD",
        &["world"],
    ) {
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    let pack = scratch.path().join("datapacks/conditions");
    fs::create_dir_all(&pack).unwrap();
    fs::write(
        pack.join("pack.mcmeta"),
        r#"{"pack":{"description":"conditions","min_format":[121,0],"max_format":[121,0]}}"#,
    )
    .unwrap();
    fs::write(
        scratch.path().join("datapacks.json"),
        r#"{"enabled":["vanilla","file/conditions"],"disabled":[]}"#,
    )
    .unwrap();
    let write = |file: &str, value: Value| {
        let path = pack.join("data/demo").join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, value.to_string()).unwrap();
    };
    let registered =
        |id| json!({"type":"neoforge:registered","registry":"minecraft:worldgen/biome","value":id});
    let registered_in =
        |registry, id| json!({"type":"neoforge:registered","registry":registry,"value":id});
    let tag_empty =
        |registry, tag| json!({"type":"neoforge:tag_empty","registry":registry,"tag":tag});
    let not = |condition| json!({"type":"neoforge:not","value":condition});
    with_game(|game| {
        write(
            "worldgen/biome/disabled.json",
            json!({
                "neoforge:conditions":[{"type":"neoforge:never"}],
                "neoforge:value":{}
            }),
        );
        let mut enabled = game
            .common
            .res
            .biomes()
            .builtin_file("minecraft:plains")
            .unwrap()
            .clone();
        enabled["effects"]["water_color"] = json!([1.0, 0.5, 0.0]);
        enabled["attributes"]["minecraft:visual/cloud_height"] = json!(123.25);
        enabled["attributes"]["minecraft:gameplay/increased_fire_burnout"] = json!(true);
        write(
            "worldgen/biome/enabled.json",
            json!({
                "neoforge:conditions":[not(registered("minecraft:plains"))],
                "neoforge:value":enabled
            }),
        );
        write(
            "tags/worldgen/biome/disabled.json",
            json!({"values":["demo:disabled"]}),
        );
        // A pack-added placed feature and its tag, read by downstream conditions.
        write(
            "worldgen/placed_feature/added.json",
            json!({"feature":{"type":"minecraft:no_op","config":{}},"placement":[]}),
        );
        write(
            "tags/worldgen/placed_feature/added_tag.json",
            json!({"values":["demo:added"]}),
        );
        for (id, condition) in [
            (
                "placed_feature",
                registered_in("minecraft:worldgen/placed_feature", "demo:added"),
            ),
            (
                "placed_feature_tag",
                not(tag_empty(
                    "minecraft:worldgen/placed_feature",
                    "demo:added_tag",
                )),
            ),
            (
                "static_tag",
                not(tag_empty(
                    "minecraft:point_of_interest_type",
                    "minecraft:acquirable_job_site",
                )),
            ),
            ("known_biome", registered("demo:enabled")),
            ("unknown_biome", registered("demo:disabled")),
            (
                "empty_biome_tag",
                tag_empty("minecraft:worldgen/biome", "demo:disabled"),
            ),
        ] {
            write(
                &format!("recipe/{id}.json"),
                json!({
                    "type":"minecraft:crafting_shapeless", "ingredients":["minecraft:flint"],
                    "result":{"id":"minecraft:stick"}, "neoforge:conditions":[condition]
                }),
            );
        }
        let mut request = sunrise_gameplay::game::world_loading::WorldRequest::new(
            "condition-winners",
            Some("test"),
            Some(1),
            true,
            None,
            Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla()),
        )
        .unwrap();
        request.save_root = scratch.path().to_owned();
        game.load_world(request).unwrap();
        let biomes = game.common.world_biomes_base.as_ref().unwrap();
        assert!(biomes.contains("demo:enabled"));
        assert_eq!(biomes.get(biomes.id("demo:enabled")).water, 0xff7f00);
        let local = biomes.pack_file("demo:enabled").unwrap();
        assert!(local["features"].is_array());
        assert_eq!(
            local["attributes"]["minecraft:gameplay/increased_fire_burnout"],
            true
        );
        assert!(!biomes.contains("demo:disabled"));
        for id in [
            "demo:static_tag",
            "demo:known_biome",
            "demo:empty_biome_tag",
            "demo:placed_feature",
            "demo:placed_feature_tag",
        ] {
            assert!(
                game.recipes().recipe(id).is_some(),
                "missing admitted recipe {id}"
            );
        }
        assert!(game.recipes().recipe("demo:unknown_biome").is_none());
        let sync = sunrise_gameplay_common::game::lan_common::sync::sync_data(
            &game.common.options.resources,
            &game.common.res.core.item_data,
            game.items(),
            &sunrise_gameplay_common::game::pack_files(&game.server.world_tables.0),
        )
        .unwrap();
        let packet = sync
            .registry_packets(true, &game.server.level_shapes)
            .into_iter()
            .find(|packet| packet.registry == BIOME_REGISTRY)
            .unwrap();
        assert!(
            packet
                .entries
                .iter()
                .all(|entry| entry.id != "demo:disabled")
        );
        let wire = packet
            .entries
            .into_iter()
            .find(|entry| entry.id == "demo:enabled")
            .unwrap()
            .data
            .unwrap();
        let wire = sunrise_game_data::scoreboard::tag_to_json(&wire);
        assert_eq!(wire["effects"]["water_color"], "#ff7f00");
        assert!(wire.get("neoforge:value").is_none());
        assert!(wire.get("carvers").is_none());
        assert!(wire.get("features").is_none());
        assert_eq!(wire["attributes"]["minecraft:visual/cloud_height"], 123.25);
        assert!(
            wire["attributes"]
                .get("minecraft:gameplay/increased_fire_burnout")
                .is_none()
        );
    });
}

#[test]
fn worldgen_load_conditions_binds_static_layer_tags_and_vanilla_features() {
    use sunrise_assets::resource_source::ResourceSource;
    use sunrise_level::datapack::{DataPack, worldgen_overrides};

    let scratch = tempfile::tempdir().unwrap();
    let resources = scratch.path().join("resources");
    std::fs::create_dir_all(resources.join("data/minecraft/tags/block")).unwrap();
    std::fs::write(
        resources.join("registries.json"),
        json!({
            "minecraft:block": {
                "entries": {
                    "minecraft:stone": {"protocol_id": 0},
                    "minecraft:dirt": {"protocol_id": 1}
                }
            }
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        resources.join("data/minecraft/tags/block/base_rock.json"),
        json!({"values": ["minecraft:stone"]}).to_string(),
    )
    .unwrap();

    let pack_dir = scratch.path().join("pack");
    std::fs::create_dir_all(pack_dir.join("data/demo/tags/block")).unwrap();
    std::fs::create_dir_all(pack_dir.join("data/demo/worldgen/feature")).unwrap();
    std::fs::write(
        pack_dir.join("data/demo/tags/block/pack_soil.json"),
        json!({"values": ["minecraft:dirt"]}).to_string(),
    )
    .unwrap();
    std::fs::write(
        pack_dir.join("data/demo/worldgen/feature/needs_soil.json"),
        json!({
            "neoforge:conditions": [{
                "type": "neoforge:not",
                "value": {
                    "type": "neoforge:tag_empty",
                    "registry": "minecraft:block",
                    "tag": "demo:pack_soil"
                }
            }],
            "type": "minecraft:no_op",
            "config": {}
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        pack_dir.join("data/demo/worldgen/feature/when_absent_empty.json"),
        json!({
            "neoforge:conditions": [{
                "type": "neoforge:tag_empty",
                "registry": "minecraft:block",
                "tag": "demo:nonexistent"
            }],
            "type": "minecraft:no_op",
            "config": {}
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        pack_dir.join("data/demo/worldgen/feature/fails_non_empty.json"),
        json!({
            "neoforge:conditions": [{
                "type": "neoforge:tag_empty",
                "registry": "minecraft:block",
                "tag": "minecraft:base_rock"
            }],
            "type": "minecraft:no_op",
            "config": {}
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        pack_dir.join("data/demo/worldgen/feature/vanilla_flag.json"),
        json!({
            "neoforge:conditions": [{
                "type": "neoforge:feature_flags_enabled",
                "flags": ["minecraft:vanilla"]
            }],
            "type": "minecraft:no_op",
            "config": {}
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        pack_dir.join("data/demo/worldgen/feature/experimental_flag.json"),
        json!({
            "neoforge:conditions": [{
                "type": "neoforge:feature_flags_enabled",
                "flags": ["minecraft:trade_rebalance"]
            }],
            "type": "minecraft:no_op",
            "config": {}
        })
        .to_string(),
    )
    .unwrap();

    let pack = DataPack::from_source("file/demo".into(), ResourceSource::directory(pack_dir));
    let conditions = sunrise_gameplay_common::game::load_conditions::worldgen_load_conditions(
        &sunrise_world::conditions::LoadConditions::vanilla(),
        &resources,
        &[&pack],
    );
    let (overrides, _) = worldgen_overrides(
        &sunrise_level::datapack::DataPacks::new(&[&pack]),
        &conditions,
    );
    let features: Vec<&str> = overrides.entries("features").map(|(id, _)| id).collect();
    assert!(features.contains(&"demo:needs_soil"), "{features:?}");
    assert!(features.contains(&"demo:when_absent_empty"), "{features:?}");
    assert!(!features.contains(&"demo:fails_non_empty"), "{features:?}");
    assert!(features.contains(&"demo:vanilla_flag"), "{features:?}");
    assert!(
        !features.contains(&"demo:experimental_flag"),
        "{features:?}"
    );
}

#[test]
fn slot_source_and_extended_registry_tag_empty_conditions() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let dir = root.join("data/boiler/slot_source");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("when_damage_tag_present.json"),
        json!({
            "neoforge:conditions": [{
                "type": "neoforge:not",
                "value": {
                    "type": "neoforge:tag_empty",
                    "registry": "minecraft:damage_type",
                    "tag": "minecraft:bypasses_armor"
                }
            }],
            "type": "minecraft:slot_range",
            "slots": "container.*"
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        dir.join("when_biome_tag_present.json"),
        json!({
            "neoforge:conditions": [{
                "type": "neoforge:not",
                "value": {
                    "type": "neoforge:tag_empty",
                    "registry": "minecraft:worldgen/biome",
                    "tag": "minecraft:is_forest"
                }
            }],
            "type": "minecraft:slot_range",
            "slots": "container.*"
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        dir.join("when_mod_absent.json"),
        json!({
            "neoforge:conditions": [{
                "type": "neoforge:mod_loaded",
                "modid": "absent"
            }],
            "type": "minecraft:slot_range",
            "slots": "container.*"
        })
        .to_string(),
    )
    .unwrap();

    let conditions = sunrise_world::conditions::LoadConditions::vanilla().with_tags([
        (
            "minecraft:damage_type",
            [("minecraft:bypasses_armor", true)],
        ),
        ("minecraft:worldgen/biome", [("minecraft:is_forest", true)]),
    ]);
    let overlay = sunrise_world::loot::PackOverlay {
        conditions: &conditions,
        ..sunrise_world::loot::PackOverlay::default()
    };
    std::fs::write(root.join("items.json"), "{}").unwrap();
    let defaults = sunrise_world::item_defaults::read(root).unwrap();
    let loot = sunrise_world::loot::LootData::load(root, &overlay, &defaults).unwrap();
    assert!(
        loot.slot_sources()
            .contains_key("boiler:when_damage_tag_present")
    );
    assert!(
        loot.slot_sources()
            .contains_key("boiler:when_biome_tag_present")
    );
    assert!(!loot.slot_sources().contains_key("boiler:when_mod_absent"));
}

#[test]
fn trade_files_with_unknown_load_condition_report_errors() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let trade_dir = root.join("data/boiler/villager_trade");
    let set_dir = root.join("data/boiler/trade_set");
    std::fs::create_dir_all(&trade_dir).unwrap();
    std::fs::create_dir_all(&set_dir).unwrap();
    std::fs::write(
        trade_dir.join("bad_trade.json"),
        json!({
            "neoforge:conditions": [{"type": "unknown:bogus_trade"}],
            "wants": {"id": "minecraft:emerald", "count": 1},
            "gives": {"id": "minecraft:stone", "count": 1}
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        set_dir.join("bad_set.json"),
        json!({
            "neoforge:conditions": [{"type": "unknown:bogus_set"}],
            "trades": "#boiler:empty"
        })
        .to_string(),
    )
    .unwrap();

    let conditions = sunrise_world::conditions::LoadConditions::new(["chemlib"]);
    let overlay = sunrise_world::loot::PackOverlay {
        conditions: &conditions,
        ..sunrise_world::loot::PackOverlay::default()
    };
    let loaded = sunrise_sim::merchant::Trades::load(root, &overlay).unwrap();
    assert!(!loaded.trades.contains_key("boiler:bad_trade"));
    assert!(!loaded.sets.contains_key("boiler:bad_set"));
    let err = conditions
        .check_errors()
        .expect_err("unknown condition in trade files must be reported");
    assert!(
        err.contains("data/boiler/villager_trade/bad_trade.json")
            && err.contains("unknown:bogus_trade"),
        "expected trade file path and unknown condition id, got: {err}"
    );
    assert!(
        err.contains("data/boiler/trade_set/bad_set.json") && err.contains("unknown:bogus_set"),
        "expected trade_set file path and unknown condition id, got: {err}"
    );
}

use sunrise_gameplay_common::game::tables::GameItems as _;
