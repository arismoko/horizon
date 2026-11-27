use mod_api::__private::game::RegistrationContextNew as _;

/// Install the frozen type registries before selected enchantment definitions are loaded.
pub fn register_enchantment_effects() {
    let mut effects = mod_api::__private::EnchantmentEffectRegistry::default();
    crate::game::native_hooks::register_enchantment_effects::native_register_enchantment_effects(
        &mod_api::RegistrationContext::new(),
        &mut effects,
    );
    sunrise_items::enchant::install_effect_types(effects);
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    #[test]
    fn entity_registry_and_spawner_facts_cannot_publish_different_generations() {
        use std::fs;
        use sunrise_gameplay_common::game::mods::register_iterated_registries;

        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        let first = tempfile::tempdir().unwrap();
        let next = tempfile::tempdir().unwrap();
        for (root, entity) in [
            (first.path(), "minecraft:arrow"),
            (next.path(), "minecraft:egg"),
        ] {
            fs::write(
                root.join("registries.json"),
                format!(
                    r#"{{"minecraft:entity_type":{{"entries":{{"{entity}":{{"protocol_id":0}}}}}}}}"#
                ),
            )
            .unwrap();
            fs::write(
                root.join("entity-types.json"),
                format!(r#"{{"{entity}":{{"category":"misc","width":0.25,"height":0.25}}}}"#),
            )
            .unwrap();
        }

        let facts = register_iterated_registries(first.path()).unwrap();
        assert!(facts.contains_key("minecraft:arrow"));
        let arrow = mod_api::__private::resolved_entity_type_key("minecraft:arrow").unwrap();
        // The new reports are internally consistent. They still must not replace the
        // spawner facts while the API continues to expose the old registry's keys.
        let Err(error) = register_iterated_registries(next.path()) else {
            panic!("published a second set of spawner facts against the frozen entity keys");
        };
        assert!(error.to_string().contains("frozen entity-type registry"));
        assert_eq!(
            mod_api::__private::resolved_entity_type_key("minecraft:arrow"),
            Some(arrow)
        );
        assert!(mod_api::__private::resolved_entity_type_key("minecraft:egg").is_none());
    }

    /// A mod's live-switch verdict is read once per version of the mod: a later look finds
    /// it without reading the files again, a `--dev-mod` is always read, and the mod
    /// replaced (a new modification time) is read again.
    #[test]
    fn a_mods_live_switch_verdict_is_read_once_per_version_of_it() {
        let dir = std::env::temp_dir().join(format!("sunrise-verdict-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ruby = dir.join("ruby");
        std::fs::create_dir_all(ruby.join("assets/ruby/lang")).unwrap();
        std::fs::create_dir_all(ruby.join("data/ruby")).unwrap();
        std::fs::write(ruby.join("assets/ruby/lang/en_us.json"), "{}").unwrap();
        let manifest = sunrise_mod_package::Manifest::parse(
            "id = \"ruby\"\nname = \"Ruby\"\nversion = \"0.1.0\"\napi = \"0.2\"\n\
             side = \"both\"\ncapabilities = [\"registries\"]\n",
        )
        .unwrap();
        assert_eq!(verdict(&ruby, &manifest, false), Some(Ok(())));
        // Worldgen added below `data/ruby`: the mod folder's own time stays.
        let worldgen = ruby.join("data/ruby/worldgen/configured_feature");
        std::fs::create_dir_all(&worldgen).unwrap();
        std::fs::write(worldgen.join("ore.json"), "{}").unwrap();
        let switches_on_restart = Some(Err("sunrise.mods.switch.worldgen"));
        assert_eq!(verdict(&ruby, &manifest, true), switches_on_restart);
        assert_eq!(verdict(&ruby, &manifest, false), Some(Ok(())));
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        let mut folder = std::fs::OpenOptions::new();
        folder.read(true);
        // Windows opens a folder only with FILE_FLAG_BACKUP_SEMANTICS, and sets its time only
        // with write access.
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            folder.write(true).custom_flags(0x0200_0000);
        }
        folder.open(&ruby).unwrap().set_modified(later).unwrap();
        assert_eq!(verdict(&ruby, &manifest, false), switches_on_restart);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn native_declaration_diagnostics_report_each_bad_submission() {
        struct DeclaresBadContent;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for DeclaresBadContent {
            fn blocks(&self, _: &RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
                let _ = event.register(mod_api::Block::new("foreign:Bad Lamp", "minecraft:stone"));
            }
            fn items(&self, _: &RegistrationContext, event: &mut mod_api::ItemsEvent<'_>) {
                let _ = event.register(mod_api::Item::new("Bad Key", "minecraft:stick"));
            }
        }
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new(
                "maker",
                Box::new(DeclaresBadContent),
            )]
            .into(),
            || {
                let error = content_overlay(Path::new(""), &ModsState::default(), Path::new(""))
                    .err()
                    .expect("bad declarations refuse the overlay");
                let report = error.downcast_ref::<DeclarationFailures>().unwrap();
                let messages: Vec<_> = report.messages().collect();
                assert_eq!(messages.len(), 2);
                assert_eq!(messages[0].key(), "sunrise.mods.declaration.block");
                assert_eq!(messages[1].key(), "sunrise.mods.declaration.item");
                assert_eq!(
                    report.0,
                    [
                        DeclarationRefusal::new(
                            "maker",
                            DeclarationKind::Block,
                            DeclarationError::InvalidId {
                                id: "foreign:Bad Lamp".into(),
                            }
                        ),
                        DeclarationRefusal::new(
                            "maker",
                            DeclarationKind::Item,
                            DeclarationError::InvalidId {
                                id: "maker:Bad Key".into()
                            }
                        ),
                    ]
                );
            },
        );
    }

    #[test]
    fn native_declaration_local_errors_return_to_the_listener() {
        struct Declares;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Declares {
            fn blocks(&self, _: &RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
                let mut block = mod_api::Block::new("bright", "minecraft:stone");
                block.properties.light = Some(16);
                assert!(matches!(
                    event.register(block),
                    Err(DeclarationError::Rejected { id, reason })
                        if id == "maker:bright" && !reason.is_empty()
                ));
            }
            fn items(&self, _: &RegistrationContext, event: &mut mod_api::ItemsEvent<'_>) {
                let mut item = mod_api::Item::new("spoon", "minecraft:stick");
                item.components = vec![("lore".into(), None), ("minecraft:lore".into(), None)];
                assert!(
                    matches!(event.register(item), Err(DeclarationError::Rejected { id, reason })
                    if id == "maker:spoon" && reason.contains("duplicate component id minecraft:lore"))
                );
            }
        }
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new("maker", Box::new(Declares))].into(),
            || {
                let declared = native_declared();
                assert!(declared.batches.is_empty());
                assert_eq!(
                    declared
                        .refusals
                        .iter()
                        .map(|r| (r.mod_id.as_str(), r.kind, r.error.id()))
                        .collect::<Vec<_>>(),
                    [
                        ("maker", DeclarationKind::Block, "maker:bright"),
                        ("maker", DeclarationKind::Item, "maker:spoon"),
                    ]
                );
            },
        );
    }

    #[test]
    fn native_declaration_reports_collection_collision_and_component_refusals_together() {
        struct Declares;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Declares {
            fn blocks(&self, _: &RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
                event
                    .register(mod_api::Block::new("foreign:tool", "minecraft:stone"))
                    .unwrap();
            }
            fn items(&self, _: &RegistrationContext, event: &mut mod_api::ItemsEvent<'_>) {
                event
                    .register(mod_api::Item::new("foreign:tool", "minecraft:stick"))
                    .unwrap();
                let mut item = mod_api::Item::new("bad_component", "minecraft:stick");
                item.components
                    .push(("maker:unknown".into(), Some(mod_api::Tag::Int(1))));
                event.register(item).unwrap();
            }
        }
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new("maker", Box::new(Declares))].into(),
            || {
                let error = content_overlay(Path::new(""), &ModsState::default(), Path::new(""))
                    .err()
                    .expect("invalid native batch must refuse startup");
                let report = error.downcast_ref::<DeclarationFailures>().unwrap();
                assert_eq!(report.0.len(), 2, "{:?}", report.0);
                for (record, id, detail) in [
                    (&report.0[0], "foreign:tool", "block's item"),
                    (&report.0[1], "maker:bad_component", "maker:unknown"),
                ] {
                    assert_eq!(
                        (&*record.mod_id, record.kind),
                        ("maker", DeclarationKind::Item)
                    );
                    assert!(
                        matches!(&record.error, DeclarationError::Rejected { id: rejected, reason }
                        if rejected == id && reason.contains(detail)),
                        "{record:?}"
                    );
                }
            },
        );
    }

    #[test]
    fn native_component_registration_refuses_a_builtin_key() {
        struct DeclaresDamage;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for DeclaresDamage {
            fn component_types(
                &self,
                _: &RegistrationContext,
                event: &mut mod_api::ComponentTypesEvent<'_>,
            ) {
                event.register("minecraft:damage".into());
            }
        }
        let root =
            std::env::temp_dir().join(format!("sunrise-component-builtin-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("registries.json"),
            r#"{"minecraft:data_component_type":{"entries":{"minecraft:damage":{"protocol_id":0}}}}"#,
        )
        .unwrap();
        let result = mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new("maker", Box::new(DeclaresDamage))].into(),
            || sunrise_gameplay_common::game::mods::register_components(&root),
        );
        std::fs::remove_dir_all(root).unwrap();
        assert_eq!(
            result.unwrap_err().to_string(),
            "mod maker: native component minecraft:damage conflicts with a built-in data component type"
        );
    }

    #[test]
    fn native_component_registration_names_submitting_and_earlier_mods_on_duplicate() {
        struct Declares(&'static str);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Declares {
            fn component_types(
                &self,
                _: &mod_api::RegistrationContext,
                event: &mut mod_api::ComponentTypesEvent<'_>,
            ) {
                event.register(self.0.into());
            }
        }
        let root = std::env::temp_dir().join(format!(
            "sunrise-component-duplicate-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("registries.json"),
            r#"{"minecraft:data_component_type":{"entries":{"minecraft:damage":{"protocol_id":0}}}}"#,
        )
        .unwrap();
        let duplicate = mod_platform::testing::with_mods(
            vec![
                mod_api::ModEntry::new("first", Box::new(Declares("shared:charge"))),
                mod_api::ModEntry::new("second", Box::new(Declares("shared:charge"))),
            ]
            .into(),
            || sunrise_gameplay_common::game::mods::register_components(&root),
        );
        let invalid = mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new("broken", Box::new(Declares("bare")))].into(),
            || sunrise_gameplay_common::game::mods::register_components(&root),
        );
        std::fs::remove_dir_all(root).unwrap();
        assert_eq!(
            duplicate.unwrap_err().to_string(),
            "mod second: duplicate native component shared:charge (already registered by mod first)"
        );
        assert_eq!(
            invalid.unwrap_err().to_string(),
            "mod broken: invalid native component name \"bare\""
        );
    }

    #[test]
    fn panicking_native_declarations_publish_neither_prefixes_nor_diagnostics() {
        struct Declares {
            panics: bool,
        }
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Declares {
            fn blocks(&self, _: &RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
                event
                    .register(mod_api::Block::new("pot", "minecraft:stone"))
                    .unwrap();
                if self.panics {
                    let _ = event.register(mod_api::Block::new("Bad Block", "minecraft:stone"));
                    panic!("failed after an invalid block declaration");
                }
            }
            fn items(&self, _: &RegistrationContext, event: &mut mod_api::ItemsEvent<'_>) {
                event
                    .register(mod_api::Item::new("tool", "minecraft:stick"))
                    .unwrap();
                if self.panics {
                    let _ = event.register(mod_api::Item::new("Bad Item", "minecraft:stick"));
                    panic!("failed after an invalid item declaration");
                }
            }
        }

        mod_platform::testing::with_mods(
            vec![
                mod_api::ModEntry::new("failed", Box::new(Declares { panics: true })),
                mod_api::ModEntry::new("healthy", Box::new(Declares { panics: false })),
            ]
            .into(),
            || {
                let declared = native_declared();
                assert!(declared.refusals.is_empty());
                assert_eq!(
                    declared.batches.keys().copied().collect::<Vec<_>>(),
                    ["healthy"]
                );
                let batch = &declared.batches["healthy"];
                assert_eq!(batch.blocks[0].get().id, "healthy:pot");
                assert_eq!(batch.items[0].get().id, "healthy:tool");
                assert_eq!(mod_api::__private::take_hook_panics().len(), 2);
            },
        );
    }

    #[test]
    fn startup_collects_native_declarations_with_registration_open() {
        struct DeclaresSeat;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for DeclaresSeat {
            fn blocks(&self, _: &RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
                let _ = event.register(mod_api::Block::new("stone_seat", "minecraft:stone"));
            }
            fn items(&self, _: &RegistrationContext, event: &mut mod_api::ItemsEvent<'_>) {
                let _ = event.register(mod_api::Item::new("cushion", "minecraft:stick"));
            }
        }
        let mods = vec![mod_api::ModEntry::new("seat", Box::new(DeclaresSeat))].into();
        mod_platform::testing::with_mods(mods, || {
            // Startup opens nothing around `freeze_declarations`: collecting opens it.
            assert!(!mod_api::registration_open());
            let declared = native_declared();
            let batch = &declared.batches["seat"];
            assert_eq!(batch.blocks[0].get().id, "seat:stone_seat");
            assert_eq!(batch.items[0].get().id, "seat:cushion");
            assert!(!mod_api::registration_open());
        });
    }

    #[test]
    fn native_declarations_follow_the_scoped_mod_fixture() {
        struct DeclaresBlock;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for DeclaresBlock {
            fn blocks(&self, _: &RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
                event
                    .register(mod_api::Block::new("foreign:blastproof", "minecraft:stone"))
                    .unwrap();
            }
        }
        mod_platform::testing::with_mods(Vec::new().into(), || {
            assert!(native_declared().batches.is_empty());
        });
        let mods = vec![mod_api::ModEntry::new("test", Box::new(DeclaresBlock))].into();
        mod_platform::testing::with_mods(mods, || {
            assert!(
                native_declared().batches["test"]
                    .blocks
                    .iter()
                    .any(|b| b.get().id == "foreign:blastproof")
            );
        });
        mod_platform::testing::with_mods(Vec::new().into(), || {
            assert!(native_declared().batches.is_empty());
        });
    }

    #[allow(unused_imports)]
    use mod_api::__private::native::BuiltMod;
    #[allow(unused_imports)]
    use std::{
        collections::{BTreeSet, HashMap},
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
    };
    use sunrise_gameplay_common::game::mods::mod_list;
    #[cfg(test)]
    use sunrise_gameplay_common::game::mods::{
        DeclarationError, DeclarationFailures, DeclarationKind, DeclarationRefusal,
        DisabledServingHost, ExecutionRole, Installed, MissingJar, ModsState, RegistrationContext,
        ServedContentLoadError, ServedContentStage, collect_key_mappings, content_layer_owner,
        content_overlay, content_root, live_switch_of, living_mod_types, mob_effect_entries,
        native_declared, native_mods_detail, not_built, refuse_primary_namespace_claims,
        register_mob_effects, validate_entity_type_families, validate_startup_packages, verdict,
    };
    #[allow(unused_imports)]
    use sunrise_mod_package::runtime::Runtime;

    #[test]
    fn a_package_cannot_claim_another_packages_primary_namespace() {
        let manifest = |id, aliases| {
            sunrise_mod_package::Manifest::parse(&format!(
                "id='{id}'\nname='{id}'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\nnamespaces=[{aliases}]"
            ))
            .unwrap()
        };
        let owner = manifest("owner", "");
        let claimant = manifest("claimant", "'owner'");
        for layers in [[&owner, &claimant], [&claimant, &owner]] {
            assert_eq!(
                refuse_primary_namespace_claims(&layers)
                    .unwrap_err()
                    .to_string(),
                "mod claimant claims namespace owner, which is the primary namespace of mod owner; turn one of them off"
            );
        }
    }

    #[test]
    fn served_by_startup_validation_owners_live_switch_and_fatal_overlay_failures() {
        const HOST_BUILT: &[BuiltMod] = &[BuiltMod {
            id: "horizon_java",
            version: "1.0.0",
            side: mod_api::Side::Both,
        }];
        const SERVER_HOST_BUILT: &[BuiltMod] = &[BuiltMod {
            id: "horizon_srv",
            version: "1.0.0",
            side: mod_api::Side::Server,
        }];

        let f = Fixture::new("served-by");
        let make_native = |dir_name: &str,
                           id: &str,
                           side: &str,
                           serves: bool,
                           built: &'static [BuiltMod]| {
            let compiled = f.0.join(dir_name);
            let pkg = compiled.join(mod_list::NATIVE_DIR).join(id);
            std::fs::create_dir_all(pkg.join("src")).unwrap();
            std::fs::write(
                pkg.join("Cargo.toml"),
                format!("[package]\nname='{id}'\nversion='1.0.0'\n"),
            )
            .unwrap();
            std::fs::write(pkg.join("src/lib.rs"), "pub fn hooks() {}").unwrap();
            std::fs::write(
                pkg.join("sunrise-mod.toml"),
                format!(
                    "id='{id}'\nname='{id}'\nversion='1.0.0'\napi='0.2'\nside='{side}'\ncapabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{}'\nserves_content={serves}\n",
                    "a".repeat(40),
                ),
            )
            .unwrap();
            Runtime::load(&compiled.join("sunrise-engine"), built).unwrap()
        };
        let make_content = |file_name: &str, manifest: &str, files: &[(&str, &[u8])]| {
            let path = f.0.join(file_name);
            std::fs::write(
                &path,
                sunrise_mod_package::Package::write_files(manifest, files).unwrap(),
            )
            .unwrap();
            sunrise_mod_package::selection::SelectedPackage {
                package: sunrise_mod_package::Package::read(&path).unwrap(),
                path,
            }
        };

        let host_runtime = Arc::new(make_native(
            "host-ok",
            "horizon_java",
            "both",
            true,
            HOST_BUILT,
        ));
        let host_unopted = Arc::new(make_native(
            "host-no",
            "horizon_java",
            "both",
            false,
            HOST_BUILT,
        ));
        let runes_ok = make_content(
            "runes.smod",
            "id='runes'\nname='Runes'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\nnamespaces=['runes_extra']\nserved_by='horizon_java'\n",
            &[("assets/runes/lang/en_us.json", b"{}")],
        );
        let plain_ok = make_content(
            "plain.smod",
            "id='plain'\nname='Plain'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n",
            &[("assets/plain/lang/en_us.json", b"{}")],
        );

        // validate_startup_packages accepts valid served_by without clash and refuses unopted/missing hosts.
        let both_selected: Arc<[sunrise_mod_package::selection::SelectedPackage]> =
            vec![runes_ok, plain_ok].into();
        let valid_state = ModsState {
            native: host_runtime.clone(),
            selected: both_selected.clone(),
            ..Default::default()
        };
        validate_startup_packages(&valid_state).unwrap();

        let runes_only: Arc<[sunrise_mod_package::selection::SelectedPackage]> =
            vec![sunrise_mod_package::selection::SelectedPackage {
                package: sunrise_mod_package::Package::read(&both_selected[0].path).unwrap(),
                path: both_selected[0].path.clone(),
            }]
            .into();
        let unopted_state = ModsState {
            native: host_unopted,
            selected: runes_only.clone(),
            ..Default::default()
        };
        let unopted_err = validate_startup_packages(&unopted_state).unwrap_err();
        assert!(
            matches!(
                unopted_err.downcast_ref::<sunrise_mod_package::dependencies::Error>(),
                Some(sunrise_mod_package::dependencies::Error::InvalidServer {
                    owner,
                    target,
                    reason: sunrise_mod_package::dependencies::InvalidServerReason::HostDoesNotServeContent,
                }) if owner == "runes" && target == "horizon_java"
            ),
            "{unopted_err}"
        );

        let missing_host_state = ModsState {
            selected: runes_only,
            ..Default::default()
        };
        let missing_err = validate_startup_packages(&missing_host_state).unwrap_err();
        assert!(
            matches!(
                missing_err.downcast_ref::<sunrise_mod_package::dependencies::Error>(),
                Some(sunrise_mod_package::dependencies::Error::InvalidServer {
                    owner,
                    target,
                    reason: sunrise_mod_package::dependencies::InvalidServerReason::MissingHost,
                }) if owner == "runes" && target == "horizon_java"
            ),
            "{missing_err}"
        );

        // live_switch_of requires restart for served_by content even with assets only.
        let (mut content_ids, mut live_ids) = Default::default();
        live_switch_of(
            &both_selected[0].path,
            &both_selected[0].package,
            false,
            &mut content_ids,
            &mut live_ids,
        );
        live_switch_of(
            &both_selected[1].path,
            &both_selected[1].package,
            false,
            &mut content_ids,
            &mut live_ids,
        );
        assert!(content_ids.contains("runes") && content_ids.contains("plain"));
        assert!(!live_ids.contains("runes"));
        assert!(live_ids.contains("plain"));

        // Installed served content belongs to its host, not to the spelling of a registry key.
        let hosted = content_layer_owner("runes", &valid_state.selected);
        let plain = content_layer_owner("plain", &valid_state.selected);
        assert_eq!(hosted, "horizon_java");
        assert_eq!(plain, "plain");
        struct Quiet;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Quiet {}
        mod_platform::__private::initialize().unwrap();
        mod_platform::testing::with_dispatch(
            vec![mod_api::ModEntry::new("horizon_java", Box::new(Quiet))].into(),
            || {
                assert!(mod_api::__private::set_mod_enabled("horizon_java", false));
                mod_api::__private::install_content_owners(
                    [],
                    [],
                    [
                        ("runes:altar", hosted),
                        ("runes_extra:rune", hosted),
                        ("plain:block", plain),
                    ],
                    [],
                );
                assert!(mod_api::__private::block_entity_owner_disabled(
                    "runes:altar"
                ));
                assert!(mod_api::__private::block_entity_owner_disabled(
                    "runes_extra:rune"
                ));
                assert!(!mod_api::__private::block_entity_owner_disabled(
                    "plain:block"
                ));
            },
        );

        let base = f.0.join("vanilla");
        let store = f.0.join("jars");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("sunrise-resources.json"), "{}").unwrap();

        // Missing jar on a served content package is fatal.
        let runes_missing_jar = make_content(
            "runes-missing-jar.smod",
            &format!(
                "id='runes'\nname='Runes'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\nserved_by='horizon_java'\n[[jar]]\nid='orig'\nversion='1'\nsha512='{}'\n",
                "9".repeat(128),
            ),
            &[],
        );
        let missing_jar_state = ModsState {
            native: host_runtime.clone(),
            selected: vec![runes_missing_jar].into(),
            ..Default::default()
        };
        let err = content_overlay(&base, &missing_jar_state, &store)
            .err()
            .unwrap();
        assert!(err.downcast_ref::<MissingJar>().is_some(), "{err}");

        // Read error on a served content package is fatal.
        let runes_bad_json = make_content(
            "runes-bad-json.smod",
            "id='runes'\nname='Runes'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\nserved_by='horizon_java'\n",
            &[("data/runes/sunrise/item/broken.json", b"{not json")],
        );
        let bad_json_state = ModsState {
            native: host_runtime.clone(),
            selected: vec![runes_bad_json].into(),
            ..Default::default()
        };
        let err = content_overlay(&base, &bad_json_state, &store)
            .err()
            .unwrap();
        let load_err = err
            .downcast_ref::<ServedContentLoadError>()
            .expect("typed ServedContentLoadError");
        assert_eq!(load_err.mod_id, "runes");
        assert_eq!(load_err.stage, ServedContentStage::Read);

        // Remainders supplied by a host are validated after the packages are admitted.
        std::fs::write(base.join("facts.json"), r#"{"states":[],"defaults":{}}"#).unwrap();
        std::fs::write(base.join("gameplay-facts.json"), r#"{"states":[]}"#).unwrap();
        std::fs::write(base.join("items.json"), r#"{"minecraft:mushroom_stew":{}}"#).unwrap();
        let runes_missing_remainder = make_content(
            "runes-missing-rem.smod",
            "id='runes'\nname='Runes'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\nserved_by='horizon_java'\n",
            &[(
                "data/runes/sunrise/item/stew.json",
                br#"{"template":"minecraft:mushroom_stew","crafting_remainder":"runes:host_bowl"}"#,
            )],
        );
        sunrise_mod_package::Content::read(
            &runes_missing_remainder.path,
            &runes_missing_remainder.package.manifest,
        )
        .expect("package-only read allows host-supplied remainder in owned namespace");
        let missing_rem_state = ModsState {
            native: host_runtime.clone(),
            selected: vec![runes_missing_remainder].into(),
            ..Default::default()
        };
        let err = content_overlay(&base, &missing_rem_state, &store)
            .err()
            .unwrap();
        let load_err = err.downcast_ref::<ServedContentLoadError>().unwrap();
        assert_eq!(load_err.mod_id, "runes");
        assert_eq!(load_err.stage, ServedContentStage::Admit);
        assert_eq!(
            load_err.detail,
            r#"runes:stew: crafting_remainder "runes:host_bowl" is not an item"#
        );

        // accept_mod error (unknown component) on a served content package is fatal.
        let runes_bad_component = make_content(
            "runes-bad-component.smod",
            "id='runes'\nname='Runes'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\nserved_by='horizon_java'\n",
            &[(
                "data/runes/sunrise/item/bad.json",
                br#"{"template":"minecraft:stick","components":{"example:unknown_component":1}}"#,
            )],
        );
        let bad_component_state = ModsState {
            native: host_runtime.clone(),
            selected: vec![runes_bad_component].into(),
            ..Default::default()
        };
        let err = content_overlay(&base, &bad_component_state, &store)
            .err()
            .unwrap();
        let load_err = err
            .downcast_ref::<ServedContentLoadError>()
            .expect("typed ServedContentLoadError");
        assert_eq!(load_err.mod_id, "runes");
        assert_eq!(load_err.stage, ServedContentStage::Admit);
        assert!(load_err.detail.contains("unknown_component"), "{load_err}");

        // server_only_declaration catches a side="server" host declaring a block in a served namespace.
        let server_runtime = Arc::new(make_native(
            "host-srv",
            "horizon_srv",
            "server",
            true,
            SERVER_HOST_BUILT,
        ));
        let server_runes = make_content(
            "server-runes.smod",
            "id='srv_runes'\nname='SrvRunes'\nversion='1.0.0'\napi='0.2'\nside='server'\ncapabilities=['registries']\nserved_by='horizon_srv'\n",
            &[],
        );
        struct ServerHostDeclares;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for ServerHostDeclares {
            fn blocks(&self, _: &RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
                event
                    .register(mod_api::Block::new("srv_runes:altar", "minecraft:stone"))
                    .unwrap();
            }
        }
        let server_packages = vec![
            mod_api::PackageInfo::new(
                "horizon_srv".into(),
                "1.0.0".into(),
                mod_api::PackageKind::Code,
                mod_api::Side::Server,
                vec!["horizon_srv".into()],
                vec![],
            ),
            mod_api::PackageInfo::new(
                "srv_runes".into(),
                "1.0.0".into(),
                mod_api::PackageKind::Content,
                mod_api::Side::Server,
                vec!["srv_runes".into()],
                vec![],
            )
            .with_served_by(Some("horizon_srv".into())),
        ];
        let server_state = ModsState {
            native: server_runtime,
            selected: vec![server_runes].into(),
            installation_side: sunrise_mod_package::manifest::Side::Server,
            ..Default::default()
        };
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new(
                "horizon_srv",
                Box::new(ServerHostDeclares),
            )]
            .into(),
            || {
                mod_platform::__private::with_test_package_inventory(server_packages, || {
                    let err = content_overlay(&base, &server_state, &store).err().unwrap();
                    assert_eq!(
                        err.downcast_ref::<sunrise_mod_package::manifest::ServerOnlyDeclaration>(),
                        Some(&sunrise_mod_package::manifest::ServerOnlyDeclaration {
                            declarer: "horizon_srv".into(),
                            supplier: "srv_runes".into(),
                            kind: "block".into(),
                            id: "srv_runes:altar".into(),
                        }),
                        "{err}"
                    );
                });
            },
        );

        // A side="both" host declaring a block or required client-bound payload in a
        // side="server" served package's namespace must also be refused.
        let both_server_runes = make_content(
            "both-server-runes.smod",
            "id='srv_runes'\nname='SrvRunes'\nversion='1.0.0'\napi='0.2'\nside='server'\ncapabilities=['registries']\nserved_by='horizon_java'\n",
            &[],
        );
        let both_server_state = ModsState {
            native: host_runtime,
            selected: vec![both_server_runes].into(),
            installation_side: sunrise_mod_package::manifest::Side::Server,
            ..Default::default()
        };
        let both_server_packages = || {
            vec![
                mod_api::PackageInfo::new(
                    "horizon_java".into(),
                    "1.0.0".into(),
                    mod_api::PackageKind::Code,
                    mod_api::Side::Both,
                    vec!["horizon_java".into()],
                    vec![],
                ),
                mod_api::PackageInfo::new(
                    "srv_runes".into(),
                    "1.0.0".into(),
                    mod_api::PackageKind::Content,
                    mod_api::Side::Server,
                    vec!["srv_runes".into()],
                    vec![],
                )
                .with_served_by(Some("horizon_java".into())),
            ]
        };
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new(
                "horizon_java",
                Box::new(ServerHostDeclares),
            )]
            .into(),
            || {
                mod_platform::__private::with_test_package_inventory(
                    both_server_packages(),
                    || {
                        let err = content_overlay(&base, &both_server_state, &store)
                            .err()
                            .unwrap();
                        assert_eq!(
                        err.downcast_ref::<sunrise_mod_package::manifest::ServerOnlyDeclaration>(),
                        Some(&sunrise_mod_package::manifest::ServerOnlyDeclaration {
                            declarer: "horizon_java".into(),
                            supplier: "srv_runes".into(),
                            kind: "block".into(),
                            id: "srv_runes:altar".into(),
                        }),
                        "{err}"
                    );
                    },
                );
            },
        );

        struct BothHostPayload;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for BothHostPayload {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &RegistrationContext) {
                r.payload_handlers()
                    .registrar("1")
                    .play_to_client(mod_api::PayloadId("srv_runes:sync".into()), None);
            }
        }
        mod_platform::__private::with_test_package_inventory(both_server_packages(), || {
            let payload_mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "horizon_java",
                Box::new(BothHostPayload),
            )]
            .into();
            mod_platform::__private::part_tests::install_parts_fixture(&payload_mods).unwrap();
            mod_platform::testing::with_mods(payload_mods, || {
                let err = content_overlay(&base, &both_server_state, &store)
                    .err()
                    .unwrap();
                assert_eq!(
                    err.downcast_ref::<sunrise_mod_package::manifest::ServerOnlyDeclaration>(),
                    Some(&sunrise_mod_package::manifest::ServerOnlyDeclaration {
                        declarer: "horizon_java".into(),
                        supplier: "srv_runes".into(),
                        kind: "payload".into(),
                        id: "srv_runes:sync".into(),
                    }),
                    "{err}"
                );
            });
        });
    }

    #[test]
    fn served_by_startup_refuses_general_entity_type_in_server_namespace() {
        use std::sync::Mutex;
        const HOST_BUILT: &[BuiltMod] = &[BuiltMod {
            id: "horizon_java",
            version: "1.0.0",
            side: mod_api::Side::Both,
        }];
        let f = Fixture::new("served-by-entity-type-side");
        let compiled = f.0.join("host");
        let pkg = compiled.join(mod_list::NATIVE_DIR).join("horizon_java");
        std::fs::create_dir_all(pkg.join("src")).unwrap();
        std::fs::write(
            pkg.join("Cargo.toml"),
            "[package]\nname='horizon_java'\nversion='1.0.0'\n",
        )
        .unwrap();
        std::fs::write(pkg.join("src/lib.rs"), "pub fn hooks() {}").unwrap();
        std::fs::write(
            pkg.join("sunrise-mod.toml"),
            format!(
                "id='horizon_java'\nname='Horizon'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{}'\nserves_content=true\n",
                "a".repeat(40),
            ),
        )
        .unwrap();
        let host_runtime =
            Arc::new(Runtime::load(&compiled.join("sunrise-engine"), HOST_BUILT).unwrap());
        let make_content = |file_name: &str, id: &str, side: &str| {
            let path = f.0.join(file_name);
            let manifest = format!(
                "id='{id}'\nname='{id}'\nversion='1.0.0'\napi='0.2'\nside='{side}'\ncapabilities=['registries']\nserved_by='horizon_java'\n"
            );
            std::fs::write(
                &path,
                sunrise_mod_package::Package::write_files(&manifest, &[]).unwrap(),
            )
            .unwrap();
            sunrise_mod_package::selection::SelectedPackage {
                package: sunrise_mod_package::Package::read(&path).unwrap(),
                path,
            }
        };
        let srv_runes = make_content("srv-runes.smod", "srv_runes", "server");
        let both_runes = make_content("both-runes.smod", "runes", "both");
        let base = f.0.join("vanilla");
        let store = f.0.join("jars");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("sunrise-resources.json"), "{}").unwrap();
        let server_state = ModsState {
            native: host_runtime.clone(),
            selected: vec![srv_runes].into(),
            installation_side: sunrise_mod_package::manifest::Side::Server,
            ..Default::default()
        };
        let both_state = ModsState {
            native: host_runtime,
            selected: vec![both_runes].into(),
            installation_side: sunrise_mod_package::manifest::Side::Server,
            ..Default::default()
        };
        validate_startup_packages(&server_state).unwrap();
        validate_startup_packages(&both_state).unwrap();

        struct DeclaresEntity {
            target: &'static str,
            outcome: Arc<Mutex<Option<Result<(), mod_api::GeneralEntityRegistrationError>>>>,
        }
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for DeclaresEntity {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &RegistrationContext) {
                let result = r.declare_general_entity(
                    self.target,
                    mod_api::GeneralEntityCategory::Misc,
                    0.8,
                    1.9,
                    None,
                    mod_api::GeneralSavePolicy::Save,
                    true,
                    false,
                    false,
                    None,
                    8,
                    3,
                    1,
                    &[],
                );
                *self.outcome.lock().unwrap() = Some(result.map(|_| ()));
            }
        }

        let inventory = |content_id: &str, content_side: mod_api::Side| {
            vec![
                mod_api::PackageInfo::new(
                    "horizon_java".into(),
                    "1.0.0".into(),
                    mod_api::PackageKind::Code,
                    mod_api::Side::Both,
                    vec!["horizon_java".into()],
                    vec![],
                ),
                mod_api::PackageInfo::new(
                    content_id.into(),
                    "1.0.0".into(),
                    mod_api::PackageKind::Content,
                    content_side,
                    vec![content_id.into()],
                    vec![],
                )
                .with_served_by(Some("horizon_java".into())),
            ]
        };

        // 1. Real startup path refuses at admit_parts with GeneralEntityRegistrationError::WrongSide.
        let outcome = Arc::new(Mutex::new(None));
        let mods = [mod_api::ModEntry::new(
            "horizon_java",
            Box::new(DeclaresEntity {
                target: "srv_runes:drone",
                outcome: outcome.clone(),
            }),
        )];
        mod_platform::__private::with_test_package_inventory(
            inventory("srv_runes", mod_api::Side::Server),
            || {
                assert!(sunrise_gameplay_common::game::part_admission::admit_parts(&mods).is_err());
            },
        );
        assert_eq!(
            *outcome.lock().unwrap(),
            Some(Err(mod_api::GeneralEntityRegistrationError::WrongSide))
        );

        // 2. Defense-in-depth: even if installed without the server-side inventory check,
        // content_overlay refuses with typed ServerOnlyDeclaration.
        let outcome_bypass = Arc::new(Mutex::new(None));
        let mods_bypass: mod_api::Mods = vec![mod_api::ModEntry::new(
            "horizon_java",
            Box::new(DeclaresEntity {
                target: "srv_runes:drone",
                outcome: outcome_bypass.clone(),
            }),
        )]
        .into();
        mod_platform::__private::with_test_package_inventory(
            inventory("srv_runes", mod_api::Side::Both),
            || {
                sunrise_gameplay_common::game::part_admission::admit_parts(&mods_bypass).unwrap();
                assert_eq!(*outcome_bypass.lock().unwrap(), Some(Ok(())));
                mod_platform::testing::with_mods(mods_bypass, || {
                    let err = content_overlay(&base, &server_state, &store).err().unwrap();
                    assert_eq!(
                        err.downcast_ref::<sunrise_mod_package::manifest::ServerOnlyDeclaration>(),
                        Some(&sunrise_mod_package::manifest::ServerOnlyDeclaration {
                            declarer: "horizon_java".into(),
                            supplier: "srv_runes".into(),
                            kind: "entity type".into(),
                            id: "srv_runes:drone".into(),
                        }),
                        "{err}"
                    );
                });
            },
        );

        // 3. Positive control: a side="both" served package passes both admit_parts and content_overlay.
        let outcome_ok = Arc::new(Mutex::new(None));
        let mods_ok: mod_api::Mods = vec![mod_api::ModEntry::new(
            "horizon_java",
            Box::new(DeclaresEntity {
                target: "runes:drone",
                outcome: outcome_ok.clone(),
            }),
        )]
        .into();
        mod_platform::__private::with_test_package_inventory(
            inventory("runes", mod_api::Side::Both),
            || {
                sunrise_gameplay_common::game::part_admission::admit_parts(&mods_ok).unwrap();
                assert_eq!(*outcome_ok.lock().unwrap(), Some(Ok(())));
                mod_platform::testing::with_mods(mods_ok, || {
                    content_overlay(&base, &both_state, &store).unwrap();
                });
            },
        );
    }

    #[test]
    fn served_by_startup_refuses_attribute_and_client_required_declarations_in_server_namespace() {
        use std::sync::Mutex;
        const HOST_BUILT: &[BuiltMod] = &[BuiltMod {
            id: "horizon_java",
            version: "1.0.0",
            side: mod_api::Side::Both,
        }];
        let f = Fixture::new("served-by-attribute-side");
        let compiled = f.0.join("host");
        let pkg = compiled.join(mod_list::NATIVE_DIR).join("horizon_java");
        std::fs::create_dir_all(pkg.join("src")).unwrap();
        std::fs::write(
            pkg.join("Cargo.toml"),
            "[package]\nname='horizon_java'\nversion='1.0.0'\n",
        )
        .unwrap();
        std::fs::write(pkg.join("src/lib.rs"), "pub fn hooks() {}").unwrap();
        std::fs::write(
            pkg.join("sunrise-mod.toml"),
            format!(
                "id='horizon_java'\nname='Horizon'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{}'\nserves_content=true\n",
                "a".repeat(40),
            ),
        )
        .unwrap();
        let host_runtime =
            Arc::new(Runtime::load(&compiled.join("sunrise-engine"), HOST_BUILT).unwrap());
        let make_content = |file_name: &str, id: &str, side: &str| {
            let path = f.0.join(file_name);
            let manifest = format!(
                "id='{id}'\nname='{id}'\nversion='1.0.0'\napi='0.2'\nside='{side}'\ncapabilities=['registries']\nserved_by='horizon_java'\n"
            );
            std::fs::write(
                &path,
                sunrise_mod_package::Package::write_files(&manifest, &[]).unwrap(),
            )
            .unwrap();
            sunrise_mod_package::selection::SelectedPackage {
                package: sunrise_mod_package::Package::read(&path).unwrap(),
                path,
            }
        };
        let srv_runes = make_content("srv-runes.smod", "srv_runes", "server");
        let both_runes = make_content("both-runes.smod", "runes", "both");
        let base = f.0.join("vanilla");
        let store = f.0.join("jars");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("sunrise-resources.json"), "{}").unwrap();
        let server_state = ModsState {
            native: host_runtime.clone(),
            selected: vec![srv_runes].into(),
            installation_side: sunrise_mod_package::manifest::Side::Server,
            ..Default::default()
        };
        let both_state = ModsState {
            native: host_runtime,
            selected: vec![both_runes].into(),
            installation_side: sunrise_mod_package::manifest::Side::Server,
            ..Default::default()
        };
        validate_startup_packages(&server_state).unwrap();
        validate_startup_packages(&both_state).unwrap();

        let inventory = |content_id: &str, content_side: mod_api::Side| {
            vec![
                mod_api::PackageInfo::new(
                    "horizon_java".into(),
                    "1.0.0".into(),
                    mod_api::PackageKind::Code,
                    mod_api::Side::Both,
                    vec!["horizon_java".into()],
                    vec![],
                ),
                mod_api::PackageInfo::new(
                    content_id.into(),
                    "1.0.0".into(),
                    mod_api::PackageKind::Content,
                    content_side,
                    vec![content_id.into()],
                    vec![],
                )
                .with_served_by(Some("horizon_java".into())),
            ]
        };

        struct DeclaresAttribute {
            target: &'static str,
            outcome: Arc<Mutex<Option<Result<(), mod_api::AttributeRegistrationError>>>>,
        }
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for DeclaresAttribute {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &RegistrationContext) {
                let result = r.declare_attribute(
                    self.target,
                    10.0,
                    0.0,
                    100.0,
                    true,
                    mod_api::Sentiment::Positive,
                    Default::default(),
                );
                *self.outcome.lock().unwrap() = Some(result);
            }
        }

        // 1. Real startup path refuses attribute declaration in server served namespace at admit_parts.
        let outcome = Arc::new(Mutex::new(None));
        let attr_mods = [mod_api::ModEntry::new(
            "horizon_java",
            Box::new(DeclaresAttribute {
                target: "srv_runes:mana",
                outcome: outcome.clone(),
            }),
        )];
        mod_platform::__private::with_test_package_inventory(
            inventory("srv_runes", mod_api::Side::Server),
            || {
                assert!(
                    sunrise_gameplay_common::game::part_admission::admit_parts(&attr_mods).is_err()
                );
            },
        );
        assert_eq!(
            *outcome.lock().unwrap(),
            Some(Err(mod_api::AttributeRegistrationError::WrongSide))
        );

        // 2. Defense-in-depth: content_overlay refuses attribute declaration with ServerOnlyDeclaration.
        let outcome_bypass = Arc::new(Mutex::new(None));
        let attr_bypass: mod_api::Mods = vec![mod_api::ModEntry::new(
            "horizon_java",
            Box::new(DeclaresAttribute {
                target: "srv_runes:mana",
                outcome: outcome_bypass.clone(),
            }),
        )]
        .into();
        mod_platform::__private::with_test_package_inventory(
            inventory("srv_runes", mod_api::Side::Both),
            || {
                sunrise_gameplay_common::game::part_admission::admit_parts(&attr_bypass).unwrap();
                assert_eq!(*outcome_bypass.lock().unwrap(), Some(Ok(())));
                mod_platform::testing::with_mods(attr_bypass, || {
                    let err = content_overlay(&base, &server_state, &store).err().unwrap();
                    assert_eq!(
                        err.downcast_ref::<sunrise_mod_package::manifest::ServerOnlyDeclaration>(),
                        Some(&sunrise_mod_package::manifest::ServerOnlyDeclaration {
                            declarer: "horizon_java".into(),
                            supplier: "srv_runes".into(),
                            kind: "attribute".into(),
                            id: "srv_runes:mana".into(),
                        }),
                        "{err}"
                    );
                });
            },
        );

        // 3. Custom stat in a server served namespace passes register and is refused at content_overlay.
        struct DeclaresCustomStat(&'static str);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for DeclaresCustomStat {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &RegistrationContext) {
                r.register_custom_stat(self.0).unwrap();
            }
        }
        let stat_mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "horizon_java",
            Box::new(DeclaresCustomStat("srv_runes:casts")),
        )]
        .into();
        mod_platform::__private::with_test_package_inventory(
            inventory("srv_runes", mod_api::Side::Server),
            || {
                sunrise_gameplay_common::game::part_admission::admit_parts(&stat_mods).unwrap();
                mod_platform::testing::with_mods(stat_mods, || {
                    let err = content_overlay(&base, &server_state, &store).err().unwrap();
                    assert_eq!(
                        err.downcast_ref::<sunrise_mod_package::manifest::ServerOnlyDeclaration>(),
                        Some(&sunrise_mod_package::manifest::ServerOnlyDeclaration {
                            declarer: "horizon_java".into(),
                            supplier: "srv_runes".into(),
                            kind: "custom stat".into(),
                            id: "srv_runes:casts".into(),
                        }),
                        "{err}"
                    );
                });
            },
        );

        // 4. Server-only (unsynced) attachment in a server served namespace is allowed,
        // while synced attachment and datapack registry are refused.
        struct DeclaresUnsynced;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for DeclaresUnsynced {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &RegistrationContext) {
                r.register_attachment_type(mod_api::AttachmentDescriptor::new(
                    "srv_runes:internal",
                    || (),
                ))
                .unwrap();
            }
        }
        let unsynced_mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "horizon_java",
            Box::new(DeclaresUnsynced),
        )]
        .into();
        mod_platform::__private::with_test_package_inventory(
            inventory("srv_runes", mod_api::Side::Server),
            || {
                sunrise_gameplay_common::game::part_admission::admit_parts(&unsynced_mods).unwrap();
                mod_platform::testing::with_mods(unsynced_mods, || {
                    content_overlay(&base, &server_state, &store).unwrap();
                });
            },
        );

        // 5. Positive control: all client-required declarations in a side="both" served namespace pass.
        struct DeclaresAllBoth;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for DeclaresAllBoth {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &RegistrationContext) {
                r.declare_attribute(
                    "runes:mana",
                    10.0,
                    0.0,
                    100.0,
                    true,
                    mod_api::Sentiment::Positive,
                    Default::default(),
                )
                .unwrap();
                r.register_custom_stat("runes:casts").unwrap();
                r.register_attachment_type(
                    mod_api::AttachmentDescriptor::new("runes:charge", || ()).sync(
                        mod_api::AttachmentCodec {
                            encode: |_| Ok(Default::default()),
                            decode: |_| Ok(()),
                        },
                    ),
                )
                .unwrap();
                r.declare_datapack_registry(
                    "runes:spell",
                    mod_api::DatapackRegistrySync::Required,
                    vec![mod_api::DatapackField::required(
                        "power",
                        mod_api::DatapackFieldKind::Int { min: 0, max: 100 },
                    )],
                )
                .unwrap();
            }
        }
        let ok_mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "horizon_java",
            Box::new(DeclaresAllBoth),
        )]
        .into();
        mod_platform::__private::with_test_package_inventory(
            inventory("runes", mod_api::Side::Both),
            || {
                sunrise_gameplay_common::game::part_admission::admit_parts(&ok_mods).unwrap();
                mod_platform::testing::with_mods(ok_mods, || {
                    content_overlay(&base, &both_state, &store).unwrap();
                });
            },
        );
    }

    #[test]
    fn served_by_both_sides_validate_locally_and_agree_on_mod_list() {
        const HOST_BOTH: &[BuiltMod] = &[BuiltMod {
            id: "horizon_java",
            version: "1.0.0",
            side: mod_api::Side::Both,
        }];
        const HOST_CLIENT: &[BuiltMod] = &[BuiltMod {
            id: "horizon_java",
            version: "1.0.0",
            side: mod_api::Side::Client,
        }];
        const HOST_SERVER: &[BuiltMod] = &[BuiltMod {
            id: "horizon_java",
            version: "1.0.0",
            side: mod_api::Side::Server,
        }];
        let f = Fixture::new("served-by-mod-list");
        let make_native = |dir_name: &str, side: &str, built: &'static [BuiltMod]| {
            let compiled = f.0.join(dir_name);
            let pkg = compiled.join(mod_list::NATIVE_DIR).join("horizon_java");
            std::fs::create_dir_all(pkg.join("src")).unwrap();
            std::fs::write(
                pkg.join("Cargo.toml"),
                "[package]\nname='horizon_java'\nversion='1.0.0'\n",
            )
            .unwrap();
            std::fs::write(pkg.join("src/lib.rs"), "pub fn hooks() {}").unwrap();
            std::fs::write(
                pkg.join("sunrise-mod.toml"),
                format!(
                    "id='horizon_java'\nname='Horizon'\nversion='1.0.0'\napi='0.2'\nside='{side}'\ncapabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{}'\nserves_content=true\n",
                    "a".repeat(40),
                ),
            )
            .unwrap();
            Runtime::load(&compiled.join("sunrise-engine"), built).unwrap()
        };
        let path = f.0.join("runes.smod");
        std::fs::write(
            &path,
            sunrise_mod_package::Package::write_files(
                "id='runes'\nname='Runes'\nversion='2.1.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\nserved_by='horizon_java'\n",
                &[],
            )
            .unwrap(),
        )
        .unwrap();
        let selected: Arc<[sunrise_mod_package::selection::SelectedPackage]> =
            vec![sunrise_mod_package::selection::SelectedPackage {
                package: sunrise_mod_package::Package::read(&path).unwrap(),
                path,
            }]
            .into();

        // A client-only or server-only host is refused locally for side="both" served content.
        for (bad_side, expected_host, built) in [
            (
                "client",
                sunrise_mod_package::manifest::Side::Client,
                HOST_CLIENT,
            ),
            (
                "server",
                sunrise_mod_package::manifest::Side::Server,
                HOST_SERVER,
            ),
        ] {
            let bad_state = ModsState {
                native: Arc::new(make_native(&format!("host-{bad_side}"), bad_side, built)),
                selected: selected.clone(),
                ..Default::default()
            };
            let err = validate_startup_packages(&bad_state).unwrap_err();
            assert!(
                matches!(
                    err.downcast_ref::<sunrise_mod_package::dependencies::Error>(),
                    Some(sunrise_mod_package::dependencies::Error::InvalidServer {
                        owner,
                        target,
                        reason: sunrise_mod_package::dependencies::InvalidServerReason::SideNotCovered {
                            host,
                            content: sunrise_mod_package::manifest::Side::Both,
                        },
                    }) if owner == "runes" && target == "horizon_java" && *host == expected_host
                ),
                "{bad_side}: {err}"
            );
        }

        // Valid side="both" host + served content validates on both sides and produces matching
        // sunrise:mod_list payloads directly from content_overlay admission.
        let base = f.0.join("vanilla");
        let store = f.0.join("jars");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("sunrise-resources.json"), "{}").unwrap();
        let both_host = Arc::new(make_native("host-both", "both", HOST_BOTH));
        let server_state = ModsState {
            native: both_host.clone(),
            selected: selected.clone(),
            installation_side: sunrise_mod_package::manifest::Side::Server,
            ..Default::default()
        };
        let client_state = ModsState {
            native: both_host,
            selected: selected.clone(),
            installation_side: sunrise_mod_package::manifest::Side::Client,
            ..Default::default()
        };
        validate_startup_packages(&server_state).unwrap();
        validate_startup_packages(&client_state).unwrap();

        let server_announced = content_overlay(&base, &server_state, &store)
            .unwrap()
            .announced;
        let client_announced = content_overlay(&base, &client_state, &store)
            .unwrap()
            .announced;
        let server_wire = sunrise_protocol::mod_list::decode(&sunrise_protocol::mod_list::encode(
            &server_announced,
        ))
        .unwrap();
        let client_wire = sunrise_protocol::mod_list::decode(&sunrise_protocol::mod_list::encode(
            &client_announced,
        ))
        .unwrap();
        assert!(sunrise_protocol::mod_list::compare(&server_wire, &client_wire).is_empty());

        // A client missing the served package is rejected by sunrise:mod_list comparison.
        let client_without_runes = ModsState {
            native: server_state.native.clone(),
            installation_side: sunrise_mod_package::manifest::Side::Client,
            ..Default::default()
        };
        validate_startup_packages(&client_without_runes).unwrap();
        let partial_announced = content_overlay(&base, &client_without_runes, &store)
            .unwrap()
            .announced;
        let partial_wire = sunrise_protocol::mod_list::decode(&sunrise_protocol::mod_list::encode(
            &partial_announced,
        ))
        .unwrap();
        let diff = sunrise_protocol::mod_list::compare(&server_wire, &partial_wire);
        assert_eq!(diff.missing, ["runes 2.1.0"]);
    }

    #[test]
    fn served_by_host_switch_is_restart_bound_and_withholds_served_content_when_disabled_at_startup()
     {
        const BUILT_HOSTS: &[BuiltMod] = &[
            BuiltMod {
                id: "horizon_java",
                version: "1.0.0",
                side: mod_api::Side::Both,
            },
            BuiltMod {
                id: "plain_native",
                version: "1.0.0",
                side: mod_api::Side::Both,
            },
        ];
        let f = Fixture::new("served-by-host-switch");
        let compiled = f.0.join("compiled");
        for (id, serves) in [("horizon_java", true), ("plain_native", false)] {
            let pkg = compiled.join(mod_list::NATIVE_DIR).join(id);
            std::fs::create_dir_all(pkg.join("src")).unwrap();
            std::fs::write(
                pkg.join("Cargo.toml"),
                format!("[package]\nname='{id}'\nversion='1.0.0'\n"),
            )
            .unwrap();
            std::fs::write(pkg.join("src/lib.rs"), "pub fn hooks() {}").unwrap();
            std::fs::write(
                pkg.join("sunrise-mod.toml"),
                format!(
                    "id='{id}'\nname='{id}'\nversion='1.0.0'\napi='0.2'\nside='both'\n\
                     capabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{}'\n\
                     serves_content={serves}\n",
                    "a".repeat(40),
                ),
            )
            .unwrap();
        }
        let native =
            Arc::new(Runtime::load(&compiled.join("sunrise-engine"), BUILT_HOSTS).unwrap());

        let mods_dir = f.0.join(mod_list::DIR);
        std::fs::create_dir_all(&mods_dir).unwrap();
        std::fs::write(
            mods_dir.join("runes.smod"),
            sunrise_mod_package::Package::write_files(
                "id='runes'\nname='Runes'\nversion='1.0.0'\napi='0.2'\nside='both'\n\
                 capabilities=['registries']\nserved_by='horizon_java'\n",
                &[],
            )
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            mods_dir.join("plain.smod"),
            sunrise_mod_package::Package::write_files(
                "id='plain'\nname='Plain'\nversion='1.0.0'\napi='0.2'\nside='both'\n\
                 capabilities=['registries']\n",
                &[],
            )
            .unwrap(),
        )
        .unwrap();

        let enabled = std::collections::BTreeSet::from(["runes".into(), "plain".into()]);

        // Session 1: host starts enabled; toggling it off is restart-bound while an ordinary
        // native mod switches off live.
        let mut active = ModsState {
            native: native.clone(),
            ..Default::default()
        };
        active.capture_code_switches(&std::collections::BTreeSet::new());
        active.select(&f.0, &enabled);
        validate_startup_packages(&active).unwrap();
        assert_eq!(active.selected.len(), 2);

        let wanted_disabled =
            std::collections::BTreeSet::from(["horizon_java".into(), "plain_native".into()]);
        let (effective, waiting) = active.reconcile_code_switches(&wanted_disabled);
        assert_eq!(waiting, ["horizon_java"]);
        assert!(!effective.contains("horizon_java"));
        assert!(effective.contains("plain_native"));
        assert!(!active.code_disabled(&wanted_disabled, "horizon_java"));
        assert!(active.code_disabled(&wanted_disabled, "plain_native"));

        struct Quiet;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Quiet {}
        mod_platform::__private::initialize().unwrap();
        mod_platform::testing::with_dispatch(
            vec![
                mod_api::ModEntry::new("horizon_java", Box::new(Quiet)),
                mod_api::ModEntry::new("plain_native", Box::new(Quiet)),
            ]
            .into(),
            || {
                mod_api::__private::install_content_owners(
                    [],
                    [],
                    [("runes:altar", "horizon_java")],
                    [],
                );
                active.apply_code_switches(&wanted_disabled);
                assert!(mod_api::__private::mod_index_on(
                    mod_api::__private::mod_index("horizon_java").unwrap()
                ));
                assert!(!mod_api::__private::mod_index_on(
                    mod_api::__private::mod_index("plain_native").unwrap()
                ));
                assert!(!mod_api::__private::block_entity_owner_disabled(
                    "runes:altar"
                ));
            },
        );

        // Session 2 (after restart with host disabled): select() withholds served content,
        // reports the disabled host on its Mods-screen row, passes startup validation, and keeps
        // re-enabling the host restart-bound so both reload together.
        let mut restarted = ModsState {
            native: native.clone(),
            ..Default::default()
        };
        let startup_disabled = std::collections::BTreeSet::from(["horizon_java".into()]);
        restarted.capture_code_switches(&startup_disabled);
        restarted.select(&f.0, &enabled);
        validate_startup_packages(&restarted).unwrap();
        let selected_ids: Vec<_> = restarted
            .selected
            .iter()
            .map(|p| p.package.manifest.id.as_str())
            .collect();
        assert_eq!(selected_ids, ["plain"]);

        let mut rows = vec![Installed {
            id: "runes".into(),
            title: "Runes 1.0.0".into(),
            description: String::new(),
            problem: None,
            dev: false,
            native: None,
        }];
        restarted.report_refusals(&mut rows);
        let problem = rows[0]
            .problem
            .as_deref()
            .expect("withheld served mod shows Mods-screen problem");
        assert!(problem.contains("horizon_java"), "{problem}");

        let reenable = std::collections::BTreeSet::new();
        let (effective_reenable, waiting_reenable) = restarted.reconcile_code_switches(&reenable);
        assert_eq!(waiting_reenable, ["horizon_java"]);
        assert!(effective_reenable.contains("horizon_java"));
        assert!(restarted.code_disabled(&reenable, "horizon_java"));

        // Defend the invariant if a caller bypasses select() with a disabled host still selected.
        let mut bypassed = active.clone();
        bypassed.capture_code_switches(&startup_disabled);
        let err = validate_startup_packages(&bypassed).unwrap_err();
        assert_eq!(
            err.downcast_ref::<DisabledServingHost>(),
            Some(&DisabledServingHost {
                content: "runes".into(),
                host: "horizon_java".into(),
            }),
            "{err}"
        );
    }

    #[test]
    fn served_by_mob_effect_owner_is_serving_host() {
        let f = Fixture::new("served-by-mob-effect");
        let runes_path = f.0.join("runes.smod");
        std::fs::write(
            &runes_path,
            sunrise_mod_package::Package::write_files(
                "id='runes'\nname='Runes'\nversion='1.0.0'\napi='0.2'\nside='both'\n\
                 capabilities=['registries']\nserved_by='horizon_java'\n",
                &[(
                    "data/runes/sunrise/mob_effect/ward.json",
                    br#"{"category":"beneficial","color":65280}"#,
                )],
            )
            .unwrap(),
        )
        .unwrap();
        let plain_path = f.0.join("plain.smod");
        std::fs::write(
            &plain_path,
            sunrise_mod_package::Package::write_files(
                "id='plain'\nname='Plain'\nversion='1.0.0'\napi='0.2'\nside='both'\n\
                 capabilities=['registries']\n",
                &[(
                    "data/plain/sunrise/mob_effect/glow.json",
                    br#"{"category":"neutral","color":16777215}"#,
                )],
            )
            .unwrap(),
        )
        .unwrap();
        let runes_pkg = sunrise_mod_package::Package::read(&runes_path).unwrap();
        let plain_pkg = sunrise_mod_package::Package::read(&plain_path).unwrap();
        let layers = vec![
            sunrise_sim::mod_content::ContentMod {
                id: "runes".into(),
                content: sunrise_mod_package::Content::read(&runes_path, &runes_pkg.manifest)
                    .unwrap(),
                jars: Vec::new(),
            },
            sunrise_sim::mod_content::ContentMod {
                id: "plain".into(),
                content: sunrise_mod_package::Content::read(&plain_path, &plain_pkg.manifest)
                    .unwrap(),
                jars: Vec::new(),
            },
        ];
        let selected = vec![
            sunrise_mod_package::selection::SelectedPackage {
                package: runes_pkg,
                path: runes_path,
            },
            sunrise_mod_package::selection::SelectedPackage {
                package: plain_pkg,
                path: plain_path,
            },
        ];
        let entries = mob_effect_entries(&layers, &selected);
        let owners: Vec<_> = entries
            .iter()
            .map(|(owner, effect)| (effect.id.as_str(), *owner))
            .collect();
        assert_eq!(
            owners,
            [("plain:glow", "plain"), ("runes:ward", "horizon_java")]
        );

        type Trace = Arc<Mutex<Vec<(&'static str, &'static str)>>>;
        #[derive(Clone)]
        struct WardTick<const PERIOD: i32> {
            package: &'static str,
            trace: Trace,
        }
        impl<const PERIOD: i32> WardTick<PERIOD> {
            fn entry(package: &'static str, trace: &Trace) -> mod_api::ModEntry {
                mod_api::ModEntry::new(
                    package,
                    Box::new(Self {
                        package,
                        trace: trace.clone(),
                    }),
                )
            }
        }
        #[mod_api::sunrise_part]
        impl<const PERIOD: i32> mod_api::MobEffectPart for WardTick<PERIOD> {
            fn should_apply_effect_tick_this_tick(
                &self,
                _: &dyn mod_api::Host,
                event: &mut mod_api::ShouldApplyEffectTickThisTickEvent,
            ) {
                self.trace.lock().unwrap().push((self.package, "cadence"));
                event.apply = event.tick_count % PERIOD == 0;
            }

            fn apply_effect_tick(
                &self,
                _: &dyn mod_api::Host,
                event: &mut mod_api::ApplyEffectTickEvent<'_>,
            ) {
                self.trace.lock().unwrap().push((self.package, "action"));
                event.heal(PERIOD as f32);
            }
        }
        #[mod_api::__subscriptions]
        impl<const PERIOD: i32> mod_api::Hooks for WardTick<PERIOD> {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &RegistrationContext) {
                let key = r.effect_key("runes:ward");
                r.mob_effect_part(&key, self.clone()).unwrap();
            }
        }
        let owner_of = |id: &str| {
            owners
                .iter()
                .find_map(|(effect_id, owner)| (*effect_id == id).then_some(*owner))
        };
        let trace = Arc::new(Mutex::new(Vec::new()));
        // Put the dependent first in normal dispatch order; the effect owner must run first.
        let mods: mod_api::Mods = vec![
            WardTick::<3>::entry("dependent", &trace),
            WardTick::<2>::entry("horizon_java", &trace),
        ]
        .into();
        let package = |id: &str, kind, dependencies| {
            mod_api::PackageInfo::new(
                id.into(),
                "1.0.0".into(),
                kind,
                mod_api::Side::Both,
                vec![id.into()],
                dependencies,
            )
        };
        mod_platform::__private::with_test_package_inventory(
            vec![
                package("horizon_java", mod_api::PackageKind::Code, vec![]),
                package("runes", mod_api::PackageKind::Content, vec![])
                    .with_served_by(Some("horizon_java".into())),
                package(
                    "dependent",
                    mod_api::PackageKind::Code,
                    vec![mod_api::PackageDependency::new(
                        "horizon_java".into(),
                        "*".into(),
                        mod_api::DependencyKind::Required,
                        mod_api::DependencyOrder::Before,
                        mod_api::Side::Both,
                        None,
                        None,
                    )],
                ),
            ],
            || {
                mod_platform::testing::with_mods(mods.clone(), || {
                    mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
                    mod_api::__private::resolve_effects(
                        ["runes:ward"],
                        |id| (id == "runes:ward").then_some(0),
                        |_| true,
                    )
                    .unwrap();
                    let effect = mod_api::__private::effect_key("runes:ward").unwrap();
                    mod_api::__private::validate_effect_part_owners(owner_of, |_| true).expect(
                        "the serving host and its dependent may attach to the served effect",
                    );

                    let mut changes = Vec::new();
                    let mut keep = true;
                    let active_effects = [effect];
                    mod_api::__private::dispatch_mob_effect_tick(
                        &mod_platform::testing::FakeHost::new(),
                        6,
                        effect,
                        0,
                        |effect, amplifier| mod_api::__private::ApplyEffectTickCall {
                            effect,
                            dimension: "minecraft:overworld",
                            entity: Some(7),
                            amplifier,
                            holder: mod_api::LivingRef::new(mod_api::EntityRef::new(
                                mod_api::WorldEpoch::new(1),
                                7,
                            )),
                            health: 10.0,
                            max_health: 20.0,
                            food: None,
                            active_effects: &active_effects,
                            changes: &mut changes,
                            keep: &mut keep,
                        },
                    );
                    assert_eq!(
                        *trace.lock().unwrap(),
                        [
                            ("horizon_java", "cadence"),
                            ("horizon_java", "action"),
                            ("dependent", "cadence"),
                            ("dependent", "action"),
                        ]
                    );
                    assert_eq!(
                        changes,
                        [
                            mod_api::EffectTickChange::Heal(2.0),
                            mod_api::EffectTickChange::Heal(3.0)
                        ]
                    );
                });
            },
        );

        let unrelated: mod_api::Mods = vec![WardTick::<2>::entry("third", &trace)].into();
        mod_platform::testing::with_mods(unrelated.clone(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&unrelated).unwrap();
            let refused = mod_api::__private::validate_effect_part_owners(owner_of, |_| true)
                .expect_err("unrelated mod without dependency on serving host must be refused");
            assert_eq!(refused.len(), 1, "{refused:?}");
            assert!(
                refused[0].contains("third") && refused[0].contains("horizon_java"),
                "{refused:?}"
            );
        });
    }

    #[test]
    fn only_clients_collect_registered_keys() {
        let declared = [(
            "sample",
            mod_api::KeyRegistration::new("zoom", "key.keyboard.z"),
        )];
        assert_eq!(
            collect_key_mappings(ExecutionRole::Client, &declared).len(),
            1
        );
        assert!(collect_key_mappings(ExecutionRole::IntegratedServer, &declared).is_empty());
        assert!(collect_key_mappings(ExecutionRole::DedicatedServer, &declared).is_empty());
    }

    pub mod inventory {
        include!("mod_inventory_tests.rs");
    }

    #[cfg(test)]
    pub(in crate::game) mod recipe_priorities {
        include!("recipe_priorities_tests.rs");
    }

    pub(in crate::game) const BUILT: &[BuiltMod] = &[BuiltMod {
        id: "machine_sample",
        version: "0.3.0",
        side: mod_api::Side::Both,
    }];

    pub struct Fixture(pub PathBuf);
    impl Fixture {
        pub fn new(label: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("sunrise-game-mods-{}-{label}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        pub fn runtime(&self) -> Runtime {
            let dir = self.0.join("compiled");
            let package = dir.join(mod_list::NATIVE_DIR).join("machine_sample");
            let file = std::fs::File::open(self.0.join("machine_sample.smod")).unwrap();
            sunrise_assets::zip::ZipArchive::new(file)
                .unwrap()
                .extract(&package)
                .unwrap();
            Runtime::load(&dir.join("sunrise-engine"), BUILT).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    /// Runs `test` again in a child process whose working directory is its own
    /// empty folder: the overlay's `.sunrise-cache` lives under the home, set here to
    /// the working directory, so children sharing one (the temp dir) race on that cache.
    pub fn run_in_child(test: &str, mode: &str) {
        use sunrise_gameplay_common::game::test_fixtures::{
            child_test_name, output_within, ran_one_test,
        };

        let test = child_test_name(test);
        let cwd = std::env::temp_dir().join(format!(
            "sunrise-game-mods-cwd-{}-{mode}",
            std::process::id()
        ));
        std::fs::create_dir_all(&cwd).unwrap();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .current_dir(&cwd)
            .args(["--include-ignored", "--exact", test.as_ref(), "--nocapture"])
            .env("SUNRISE_HOME", &cwd)
            .env(mode, "1");
        let output = output_within(&mut command, &test, mode);
        std::fs::remove_dir_all(&cwd).unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        print!("{stdout}");
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
        assert!(output.status.success() && ran_one_test(&stdout));
    }

    #[test]
    fn a_package_cannot_declare_one_type_in_both_entity_families() {
        let general = sunrise_gameplay_common::game::general::GeneralTypeCatalog::freeze(&[
            mod_api::__private::GeneralEntityTypeDeclaration {
                mod_id: "ruby".into(),
                id: "ruby:robot".into(),
                category: mod_api::GeneralEntityCategory::Misc,
                width: 0.5,
                height: 1.0,
                passenger_attachment: None,
                save_policy: mod_api::GeneralSavePolicy::Save,
                summonable: true,
                pickable: false,
                fire_immune: false,
                piston_push_reaction: mod_api::PushReaction::PushPull,
                tracking_chunks: 8,
                update_interval: 3,
                payload_version: 1,
                required_features: Vec::new(),
            },
        ])
        .unwrap();
        let mut content = sunrise_mod_package::content::Content::declared("ruby");
        let mut entity: sunrise_mod_package::content::EntityType =
            serde_json::from_str(r#"{"template":"minecraft:zombie"}"#).unwrap();
        entity.id = "ruby:robot".into();
        content.entity_types.push(entity);
        let mut package = sunrise_sim::mod_content::ContentMod {
            id: "ruby".into(),
            content,
            jars: Vec::new(),
        };
        assert_eq!(
            validate_entity_type_families(std::slice::from_ref(&package), &general),
            Err("mod ruby declares entity type ruby:robot as both a content and a general entity type".into()),
        );
        package.content.entity_types[0].id = "ruby:zombie".into();
        assert_eq!(validate_entity_type_families(&[package], &general), Ok(()));
    }

    /// A content mod's tameable type (#4031) is its template's mob: a wolf, a
    /// cat or a parrot, tamed and sitting, saves its owner, sitting and its
    /// template's own field (a collar's colour, a parrot's variant) under its
    /// own id and loads back as itself with all of them.
    #[test]
    fn a_mod_tameable_type_saves_and_loads_its_owner() {
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::mobs::{Mob, Species};
        let types = living_mod_types();
        let owner = [1, 2, 3, 4];
        use sunrise_core::nbt::Tag;
        for (id, species, own) in [
            ("ruby:hound", Species::Wolf, ("CollarColor", Tag::Byte(14))),
            ("ruby:tabby", Species::Cat, ("CollarColor", Tag::Byte(3))),
            ("ruby:macaw", Species::Parrot, ("Variant", Tag::Int(2))),
        ] {
            let kind = types[id].clone();
            assert_eq!(kind.species, species, "{id}");
            let mut pet = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
            pet.extra.tame = true;
            pet.extra.owner = Some(owner);
            pet.extra.sitting = true;
            match species {
                Species::Parrot => pet.extra.variant = 2,
                _ => pet.extra.color = if species == Species::Wolf { 14 } else { 3 },
            }
            let tag = sunrise_sim::entity_nbt::mob_to_nbt(
                &pet,
                [0, 0, 0, 1],
                None,
                None,
                |_| None,
                &Default::default(),
            );
            let get = |key: &str| tag.get(key).cloned();
            assert_eq!(
                get("id"),
                Some(sunrise_core::nbt::Tag::String(id.into())),
                "{id}"
            );
            assert_eq!(
                get("Owner"),
                Some(sunrise_core::nbt::Tag::IntArray(owner.to_vec())),
                "{id}"
            );
            assert_eq!(
                get("Sitting"),
                Some(sunrise_core::nbt::Tag::Byte(1)),
                "{id}"
            );
            assert_eq!(get(own.0), Some(own.1), "{id}");
            let loaded = sunrise_sim::entity_nbt::mob_from_nbt(&tag, &types, 2, None)
                .expect("it loads back");
            assert_eq!(
                (
                    loaded.kind.id.as_str(),
                    loaded.species(),
                    loaded.extra.tame,
                    loaded.extra.owner,
                    loaded.extra.sitting,
                    (loaded.extra.color, loaded.extra.variant)
                ),
                (
                    id,
                    species,
                    true,
                    Some(owner),
                    true,
                    (pet.extra.color, pet.extra.variant)
                )
            );
        }
    }

    /// A mod's declared attribute set (#845, `EntityAttributeCreationEvent`)
    /// reaches everything a mob of the type is: the fields it spawns with,
    /// the finalize defaults and the live attribute map's catalog row —
    /// built from the template's, so the reads of #1469 and #1493 answer the
    /// declared values with no second copy.
    #[test]
    #[ignore = "daily"]
    fn a_mods_declared_entity_attributes_reach_spawn_and_the_live_map() {
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::mobs::Mob;
        use sunrise_world::attributes::entity::Catalog;

        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let catalog = Catalog::from_json(
            &sunrise_world::json_file::read(&root.join("attributes.json")).unwrap(),
        )
        .unwrap();
        let declared = [
            ("minecraft:max_health".to_string(), 55.0),
            ("minecraft:movement_speed".to_string(), 0.4),
        ];
        let merged = catalog
            .with_entity_row("ruby:ghoul", "minecraft:zombie", &declared)
            .unwrap();
        let base = |catalog: &Catalog, entity: &str, name: &str| {
            catalog.default_base(entity, catalog.id(name).unwrap())
        };
        // The declared values, and the template's row under them.
        assert_eq!(
            base(&merged, "ruby:ghoul", "minecraft:max_health"),
            Some(55.0)
        );
        assert_eq!(
            base(&merged, "ruby:ghoul", "minecraft:movement_speed"),
            Some(0.4)
        );
        assert_eq!(
            base(&merged, "ruby:ghoul", "minecraft:attack_damage"),
            base(&catalog, "minecraft:zombie", "minecraft:attack_damage")
        );
        // The live map builds for the type and answers the declared values.
        let live = sunrise_sim::living_attributes::LivingAttributes::default();
        assert_eq!(
            live.read(
                &std::sync::Arc::new(merged.clone()),
                "ruby:ghoul",
                "minecraft:max_health",
                sunrise_world::attribute_map::ValueKind::Computed,
                Vec::new
            ),
            Some(55.0)
        );
        // The type's fields spawn with them: the merged row takes the type's
        // attribute map and fields (`take_attribute_row`).
        let mut types = living_mod_types();
        let ghoul = types.get_mut("ruby:ghoul").unwrap();
        Arc::make_mut(ghoul).take_attribute_row(
            Arc::new(merged.entity_row("ruby:ghoul").unwrap()),
            &Arc::new(merged.clone()),
        );
        let ghoul = Mob::new(7, types["ruby:ghoul"].clone(), Vec3::ZERO, 0.0);
        assert_eq!(ghoul.health, 55.0, "the spawn's health");
        assert_eq!(ghoul.attrs.max_health, 55.0);
        assert_eq!(ghoul.attrs.speed, 0.4);
        assert_eq!(
            ghoul.kind.attributes.get("minecraft:max_health"),
            Some(&55.0),
            "the finalize defaults read the declared value"
        );
    }

    /// #812: a mod's declared attribute type joins the catalog before the
    /// rows resolve (a mod entity's set names its own new attribute), and
    /// the live map, an item's modifier, the saves and the sync set carry it
    /// by string id — a guest's catalog built from the same mods decodes the
    /// same network id.
    #[test]
    #[ignore = "daily"]
    fn a_mods_declared_attribute_reaches_rows_the_live_map_saves_and_sync() {
        use sunrise_core::nbt::Tag;
        use sunrise_sim::lan;
        use sunrise_sim::living_attributes::{self, LivingAttributes};
        use sunrise_world::attribute_map::ValueKind;
        use sunrise_world::attributes::{
            Definition, Sentiment,
            entity::{AddedAttribute, Catalog},
        };

        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let catalog = Catalog::from_json(
            &sunrise_world::json_file::read(&root.join("attributes.json")).unwrap(),
        )
        .unwrap();
        let vanilla = catalog.attributes().count();
        // Two mods' attributes, declared in an order that is not the sorted
        // one; one syncable, one not.
        let declared = |mod_id: &str, id: &str, default, syncable, sentiment| AddedAttribute {
            mod_id: mod_id.into(),
            id: id.into(),
            definition: Definition::ranged(default, 0.0, 100.0).unwrap(),
            syncable,
            tooltip: sunrise_world::attributes::Tooltip {
                sentiment,
                ..Default::default()
            },
        };
        let grown = catalog
            .with_attributes(&[
                declared("zeta", "zeta:fury", 15.0, false, Sentiment::default()),
                declared("ruby", "ruby:mana", 10.0, true, Sentiment::Negative),
            ])
            .unwrap();
        // After every vanilla attribute, sorted by mod id: ruby's first
        // though zeta's declared first, and vanilla ids never shift.
        let (mana, fury) = (
            grown.id("ruby:mana").unwrap(),
            grown.id("zeta:fury").unwrap(),
        );
        assert_eq!(mana.registry_id() as usize, vanilla);
        assert_eq!(fury.registry_id() as usize, vanilla + 1);
        assert_eq!(
            grown.id("minecraft:max_health"),
            catalog.id("minecraft:max_health")
        );
        // A mod entity's row names its own new attribute (#845's rows
        // resolve after the attributes join).
        let merged = grown
            .with_entity_row(
                "ruby:ghoul",
                "minecraft:zombie",
                &[("ruby:mana".into(), 42.0), ("zeta:fury".into(), 7.0)],
            )
            .unwrap();
        let live = LivingAttributes::default();
        assert_eq!(
            live.read(
                &std::sync::Arc::new(merged.clone()),
                "ruby:ghoul",
                "ruby:mana",
                ValueKind::Computed,
                Vec::new
            ),
            Some(42.0)
        );
        // An item's `attribute_modifiers` entry naming the mod attribute
        // reaches the live map (the worn-stack path's own write).
        let mut map =
            living_attributes::new_live(&std::sync::Arc::new(merged.clone()), "ruby:ghoul")
                .unwrap();
        assert_eq!(map_value(&mut map, mana), 42.0);
        living_attributes::set_modifier(&mut map, "ruby:mana", "ruby:ring", 8.0, "add_value", true)
            .unwrap();
        assert_eq!(map_value(&mut map, mana), 50.0);
        // The sync set: a syncable attribute joins it, a non-syncable one
        // never does (`AttributeMap.getSyncableAttributes`).
        living_attributes::set_modifier(&mut map, "zeta:fury", "zeta:mark", 1.0, "add_value", true)
            .unwrap();
        let synced: std::collections::HashSet<i32> = lan::attributes::syncable(&map)
            .iter()
            .map(|s| s.attribute)
            .collect();
        assert!(synced.contains(&mana.registry_id()));
        assert!(!synced.contains(&fury.registry_id()));
        // Save → load round trip by string id.
        let saved = Tag::List(living_attributes::pack(&map, true));
        let mut again =
            living_attributes::new_live(&std::sync::Arc::new(merged.clone()), "ruby:ghoul")
                .unwrap();
        living_attributes::apply_saved(&mut again, Some(&saved)).unwrap();
        // The saved base; the ring's transient modifier is equipment, not
        // saved — as vanilla's `AttributeInstance.Packed` keeps permanent
        // modifiers only.
        assert_eq!(map_value(&mut again, mana), 42.0);
        // A save whose attribute the loaded catalog does not know is
        // skipped, as vanilla warns and skips: no panic, the map whole.
        let mut vanilla_map =
            living_attributes::new_live(&std::sync::Arc::new(catalog.clone()), "minecraft:zombie")
                .unwrap();
        living_attributes::apply_saved(&mut vanilla_map, Some(&saved)).unwrap();
        assert!(lan::attributes::syncable(&vanilla_map).is_empty());
        // The network id decodes on a guest's catalog built from the same
        // mods declaring in another order: the snapshot's numbers find the
        // same attributes.
        let guest_catalog = catalog
            .with_attributes(&[
                declared("ruby", "ruby:mana", 10.0, true, Sentiment::Negative),
                declared("zeta", "zeta:fury", 15.0, false, Sentiment::default()),
            ])
            .unwrap()
            .with_entity_row(
                "ruby:ghoul",
                "minecraft:zombie",
                &[("ruby:mana".into(), 42.0), ("zeta:fury".into(), 7.0)],
            )
            .unwrap();
        let snapshots = lan::attributes::syncable(&map);
        let mut guest =
            living_attributes::new_live(&std::sync::Arc::new(guest_catalog), "ruby:ghoul").unwrap();
        lan::attributes::apply(&mut guest, &snapshots);
        assert_eq!(map_value(&mut guest, mana), 50.0);

        fn map_value(
            map: &mut sunrise_sim::living_attributes::Live,
            id: sunrise_world::attributes::entity::AttributeId,
        ) -> f64 {
            // The first read creates the instance, as `getInstance` does.
            let key = map
                .keys()
                .iter()
                .find(|key| key.attribute() == id)
                .cloned()
                .unwrap();
            let Ok(Some(handle)) = map.get_instance(Some(key)) else {
                panic!("the type has the attribute")
            };
            map.instance(&handle).unwrap().value()
        }
    }

    /// A declared set's merged row — the template's with the declared set on
    /// top — is the type's one record: a content row's own value (the
    /// ghoul's 40 health) gives way to the template's, so a mob of the type
    /// spawns with what the live attribute map answers (#845, review).
    #[test]
    #[ignore = "daily"]
    fn a_declared_type_spawns_with_its_merged_row_not_its_content_row() {
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::mobs::Mob;
        use sunrise_world::attributes::entity::Catalog;

        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let catalog = Catalog::from_json(
            &sunrise_world::json_file::read(&root.join("attributes.json")).unwrap(),
        )
        .unwrap();
        // Only the speed is declared: the merged row keeps the template's
        // health (a zombie's 20), not the content row's 40.
        let declared = [("minecraft:movement_speed".to_string(), 0.4)];
        let merged = catalog
            .with_entity_row("ruby:ghoul", "minecraft:zombie", &declared)
            .unwrap();
        let mut types = living_mod_types();
        let ghoul = types.get_mut("ruby:ghoul").unwrap();
        assert_eq!(ghoul.max_health, 40.0, "the content row's own value");
        Arc::make_mut(ghoul).take_attribute_row(
            Arc::new(merged.entity_row("ruby:ghoul").unwrap()),
            &Arc::new(merged.clone()),
        );
        let ghoul = Mob::new(7, types["ruby:ghoul"].clone(), Vec3::ZERO, 0.0);
        assert_eq!(
            ghoul.health, 20.0,
            "the template's health through the merged row"
        );
        assert_eq!(ghoul.attrs.max_health, 20.0);
        assert_eq!(ghoul.attrs.speed, 0.4, "the declared speed");
        let live = sunrise_sim::living_attributes::LivingAttributes::default();
        assert_eq!(
            live.read(
                &std::sync::Arc::new(merged),
                "ruby:ghoul",
                "minecraft:max_health",
                sunrise_world::attribute_map::ValueKind::Computed,
                Vec::new
            ),
            Some(20.0),
            "the live map answers the same row"
        );
    }

    /// A mod that adds an attribute vanilla zombies lack (#846,
    /// `EntityAttributeModificationEvent`) changes the zombie type's default:
    /// a zombie spawned after load answers it through the live attribute map
    /// and carries it in its finalize defaults, while the rest of its row
    /// and its spawn fields stay the zombie's.
    #[test]
    fn a_mods_attribute_addition_reaches_new_vanilla_zombies() {
        use mod_api::{Hooks, ModEntry, Registrar, RegistrationContext};
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::mobs::Mob;
        use sunrise_world::attribute_map::ValueKind;
        use sunrise_world::attributes::entity::Catalog;

        struct Lucky;
        #[mod_api::__subscriptions]
        impl Hooks for Lucky {
            fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
                registrar
                    .modify_entity_attributes(
                        "minecraft:zombie",
                        vec![("minecraft:luck".to_string(), Some(2.5))],
                    )
                    .unwrap();
            }
        }
        sunrise_gameplay_common::game::api_values::install_parts(&[ModEntry::new(
            "lucky",
            Box::new(Lucky),
        )])
        .unwrap();

        let bits = |n: f64| format!("{:016x}", n.to_bits());
        let catalog = Arc::new(
            Catalog::from_json(&serde_json::json!({
                "data_version": sunrise_world::DATA_VERSION,
                "attributes": [
                    {"id": "minecraft:max_health", "default": bits(20.0), "min": bits(1.0), "max": bits(1024.0), "syncable": true},
                    {"id": "minecraft:movement_speed", "default": bits(0.7), "min": bits(0.0), "max": bits(1024.0), "syncable": true},
                    {"id": "minecraft:luck", "default": bits(0.0), "min": bits(-1024.0), "max": bits(1024.0), "syncable": true}
                ],
                "entities": {
                    "minecraft:zombie": {
                        "minecraft:max_health": bits(20.0),
                        "minecraft:movement_speed": bits(0.23000000417232513)
                    }
                }
            }))
            .unwrap(),
        );
        let read = |catalog: &Arc<Catalog>, name: &str| {
            sunrise_sim::living_attributes::LivingAttributes::default().read(
                catalog,
                "minecraft:zombie",
                name,
                ValueKind::Computed,
                Vec::new,
            )
        };
        assert_eq!(
            read(&catalog, "minecraft:luck"),
            None,
            "vanilla zombies lack it"
        );

        // core.motion, then rest.mobs, as the resource load runs them.
        let modifications = mod_api::__private::entity_attribute_modifications();
        assert_eq!(modifications.len(), 1);
        let added = Arc::new(
            sunrise_gameplay_common::game::resource_load::with_mod_attribute_additions(
                &catalog,
                modifications,
            )
            .unwrap(),
        );
        let mut types: std::collections::HashMap<_, _> = living_mod_types()
            .into_iter()
            .map(|(id, kind)| (id, Arc::unwrap_or_clone(kind)))
            .collect();
        sunrise_gameplay_common::game::resource_load::take_modified_attribute_rows(
            &mut types,
            &added,
            modifications,
        );

        let zombie = Mob::new(
            7,
            Arc::new(types["minecraft:zombie"].clone()),
            Vec3::ZERO,
            0.0,
        );
        assert_eq!(read(&added, "minecraft:luck"), Some(2.5), "the live map");
        assert_eq!(
            zombie.kind.attributes.get("minecraft:luck"),
            Some(&2.5),
            "the finalize defaults"
        );
        assert_eq!(read(&added, "minecraft:max_health"), Some(20.0));
        assert_eq!(zombie.health, 20.0, "the spawn's health");
        assert_eq!(zombie.attrs.speed, 0.23000000417232513);
        // Types nobody modified keep their rows.
        assert_eq!(
            types["minecraft:villager"].attributes.get("minecraft:luck"),
            None
        );
    }

    /// An addition the catalog cannot place (an attribute it does not know,
    /// or a type with no attribute row) refuses the resource load, naming
    /// the type.
    #[test]
    fn a_mods_attribute_addition_to_an_unknown_row_refuses_the_load() {
        use mod_api::__private::EntityAttributeModification;
        use sunrise_world::attributes::entity::Catalog;

        let catalog = Catalog::from_json(&serde_json::json!({
            "data_version": sunrise_world::DATA_VERSION,
            "attributes": [
                {"id": "minecraft:luck", "default": "0000000000000000", "min": "c090000000000000", "max": "4090000000000000", "syncable": true}
            ],
            "entities": {"minecraft:zombie": {}}
        }))
        .unwrap();
        let addition = |entity: &str, attribute: &str| EntityAttributeModification {
            entity: entity.into(),
            attributes: vec![(attribute.into(), None)],
        };
        let error = sunrise_gameplay_common::game::resource_load::with_mod_attribute_additions(
            &catalog,
            &[addition("minecraft:zombie", "lucky:magic_resistance")],
        )
        .err()
        .unwrap();
        assert!(error.contains("minecraft:zombie"), "{error}");
        assert!(error.contains("lucky:magic_resistance"), "{error}");
        let error = sunrise_gameplay_common::game::resource_load::with_mod_attribute_additions(
            &catalog,
            &[addition("ruby:ghoul", "minecraft:luck")],
        )
        .err()
        .unwrap();
        assert!(error.contains("ruby:ghoul"), "{error}");
    }

    /// #4473: a content mod's effect may declare modifiers on the attributes
    /// the complete catalog has, vanilla's or a mod's; another refuses the
    /// load, naming the effect and the attribute.
    #[test]
    fn a_declared_effect_modifier_on_an_unknown_attribute_refuses_the_load() {
        use sunrise_level::brewing::{self, EffectDef};
        use sunrise_world::attributes::{Operation, entity::Catalog};

        let catalog = Catalog::from_json(&serde_json::json!({
            "data_version": sunrise_world::DATA_VERSION,
            "attributes": [
                {"id": "minecraft:luck", "default": "0000000000000000", "min": "c090000000000000", "max": "4090000000000000", "syncable": true},
                {"id": "ruby:mana", "default": "0000000000000000", "min": "0000000000000000", "max": "4090000000000000", "syncable": true}
            ],
            "entities": {"minecraft:player": {}}
        }))
        .unwrap();
        let effect = |modifiers| EffectDef {
            id: "ruby:stun",
            modifiers,
            ..*brewing::effect("slowness").unwrap()
        };
        let check = |effects: &[EffectDef]| {
            sunrise_gameplay_common::game::resource_load::check_effect_modifier_attributes(
                &catalog, effects,
            )
        };
        const KNOWN: &[brewing::EffectModifier] = &[
            ("minecraft:luck", "ruby:a", 1.0, Operation::AddValue),
            ("ruby:mana", "ruby:b", 1.0, Operation::AddValue),
        ];
        const UNKNOWN: &[brewing::EffectModifier] =
            &[("ruby:stamina", "ruby:c", 1.0, Operation::AddValue)];
        assert_eq!(check(&[effect(KNOWN), effect(&[])]), Ok(()));
        assert_eq!(
            check(&[effect(KNOWN), effect(UNKNOWN)]),
            Err("mob effect ruby:stun: attribute_modifiers: unknown attribute ruby:stamina".into())
        );
    }

    /// A content mod's type from the wandering trader, the neutral template
    /// whose behaviour is all goals: a trader under its own id, with a
    /// trader's goals, that saves what a trader saves and loads back.
    #[test]
    fn a_mod_trader_type_is_a_wandering_trader_under_its_own_id() {
        use sunrise_core::nbt::Tag;
        use sunrise_sim::mobs::{Mob, Species};
        let types = living_mod_types();
        let merchant = &types["ruby:merchant"];
        assert_eq!(merchant.species, Species::WanderingTrader);
        assert_eq!(merchant.class_id(), "minecraft:wandering_trader");
        let at = sunrise_core::geometry::Vec3::new(0.5, 64.0, 0.5);
        let trader = Mob::new(1, types["minecraft:wandering_trader"].clone(), at, 0.0);
        let merchant = Mob::new(2, merchant.clone(), at, 0.0);
        assert!(merchant.ai.goals.has("Float") && trader.ai.goals.has("Float"));
        let saved = |m: &Mob| {
            sunrise_sim::entity_nbt::mob_to_nbt(
                m,
                [0, 0, 0, 2],
                None,
                None,
                |_| None,
                &Default::default(),
            )
        };
        let keys = |tag: &Tag| -> Vec<String> {
            let Tag::Compound(map) = tag else {
                panic!("a mob saves a compound");
            };
            map.keys().map(|k| k.to_string_lossy()).collect()
        };
        let tag = saved(&merchant);
        assert_eq!(
            keys(&tag),
            keys(&saved(&trader)),
            "it saves what a trader saves"
        );
        let loaded =
            sunrise_sim::entity_nbt::mob_from_nbt(&tag, &types, 3, None).expect("it loads back");
        assert_eq!(
            (loaded.kind.id.as_str(), loaded.species()),
            ("ruby:merchant", Species::WanderingTrader)
        );
    }

    pub fn smod(dir: &std::path::Path, id: &str) {
        let manifest = format!(
            "id = '{id}'\nname = '{id}'\nversion = '0.3.0'\napi = '0.2'\nside = 'both'\ncapabilities = ['registries']\n[native]\ngame = '26.3'\nsunrise = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let files: &[(&str, &[u8])] = &[
            (
                "Cargo.toml",
                b"[package]\nname='different_crate'\nversion='9.9.9'\n",
            ),
            ("src/lib.rs", b"pub fn hooks() {}"),
        ];
        let bytes = sunrise_mod_package::Package::write_files(&manifest, files).unwrap();
        std::fs::write(dir.join(format!("{id}.smod")), bytes).unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn content_overlay_validates_dependencies_and_orders_layers() {
        const MODE: &str = "SUNRISE_CONTENT_DEPENDENCY_TEST";
        if std::env::var_os(MODE).is_none() {
            run_in_child(
                "game::mods::tests::content_overlay_validates_dependencies_and_orders_layers",
                MODE,
            );
            return;
        }
        let f = Fixture::new("content-dependencies");
        let write = |id: &str, version: &str, dependencies: &str| {
            let dir = f.0.join(id);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("sunrise-mod.toml"), format!(
                "id='{id}'\nname='{id}'\nversion='{version}'\napi='0.2'\nside='both'\ncapabilities=['registries']\n{dependencies}"
            )).unwrap();
            dir
        };
        let addon = write("alpha", "1", "[dependencies]\nzbase='^1'");
        let base_mod = write("zbase", "1", "");
        let base = f.0.join("resources");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("sunrise-resources.json"), "{}").unwrap();
        let mut sources = ModsState {
            dev: vec![addon.clone(), base_mod.clone()],
            ..Default::default()
        };
        let enabled = Default::default();
        sources.select(&f.0, &enabled);
        let overlay = content_overlay(&base, &sources, &f.0.join("jars")).unwrap();
        assert_eq!(
            overlay
                .mods
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["zbase", "alpha"]
        );
        sources.dev = vec![addon.clone()];
        sources.select(&f.0, &enabled);
        let error = content_overlay(&base, &sources, &f.0.join("jars"))
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("alpha") && error.contains("zbase"));
        write(
            "alpha",
            "1",
            "[dependencies]\nzbase={version='^1', optional=true}",
        );
        sources.select(&f.0, &enabled);
        assert!(content_overlay(&base, &sources, &f.0.join("jars")).is_ok());
        write("zbase", "2", "");
        sources.dev.push(base_mod.clone());
        sources.select(&f.0, &enabled);
        let error = content_overlay(&base, &sources, &f.0.join("jars"))
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("installed version is 2"));
        write("zbase", "1", "[dependencies]\nalpha='^1'");
        sources.select(&f.0, &enabled);
        assert!(
            content_overlay(&base, &sources, &f.0.join("jars"))
                .err()
                .unwrap()
                .to_string()
                .contains("cycle")
        );
    }

    /// A `side = "server"` content package goes through the real loader: a registry
    /// declaration fails the load by name even when it would not parse, and assets load.
    #[test]
    #[ignore = "daily"]
    fn a_server_package_is_refused_at_load_for_registry_files_and_loads_with_assets() {
        const MODE: &str = "SUNRISE_SERVER_PACKAGE_TEST";
        if std::env::var_os(MODE).is_none() {
            run_in_child(
                "game::mods::tests::a_server_package_is_refused_at_load_for_registry_files_and_loads_with_assets",
                MODE,
            );
            return;
        }
        let f = Fixture::new("server-package");
        let package = f.0.join("srv");
        let write = |file: &str, text: &str| {
            let path = package.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(
            "sunrise-mod.toml",
            "id='srv'\nname='srv'\nversion='1'\napi='0.2'\nside='server'\ncapabilities=['registries']\n",
        );
        write("data/srv/sunrise/item/gem.json", "{ not json");
        // Merging a mod's assets reads the prepared root's block reports, so this is the real one.
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let mut sources = ModsState {
            dev: vec![package.clone()],
            ..Default::default()
        };
        let enabled = Default::default();
        sources.select(&f.0, &enabled);
        let error = content_overlay(&base, &sources, &f.0.join("jars"))
            .err()
            .unwrap()
            .to_string();
        assert_eq!(
            error,
            "srv: a side = \"server\" mod carries data/srv/sunrise/item/gem.json, which every \
             client must have; declare side = \"both\""
        );
        std::fs::remove_dir_all(package.join("data")).unwrap();
        write("assets/srv/lang/en_us.json", "{}");
        sources.select(&f.0, &enabled);
        let overlay = content_overlay(&base, &sources, &f.0.join("jars")).unwrap();
        assert_eq!(
            overlay
                .mods
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["srv"]
        );
    }

    #[test]
    #[ignore = "daily"]
    fn native_content_follows_the_build_and_misplaced_content_follows_its_selection() {
        const MODE: &str = "SUNRISE_MODS_SELECTION_TEST";
        let mode = std::env::var(MODE).ok();
        if mode.is_none() {
            // Inventory and content registries are process-wide, as in a real startup.
            for mode in ["compiled", "vanilla"] {
                let status = std::process::Command::new(std::env::current_exe().unwrap())
                    .current_dir(std::env::temp_dir())
                    .args(["--include-ignored", "--exact", "game::mods::tests::native_content_follows_the_build_and_misplaced_content_follows_its_selection", "--nocapture"])
                    .env(MODE, mode).status().unwrap();
                assert!(status.success());
            }
            return;
        }
        let f = Fixture::new(mode.as_deref().unwrap());
        smod(&f.0, "machine_sample");
        let native = if mode.as_deref() == Some("compiled") {
            f.runtime()
        } else {
            Runtime::default()
        };
        let here = f.0.join("unrelated-game");
        let misplaced = here.join(mod_list::NATIVE_DIR);
        std::fs::create_dir_all(&misplaced).unwrap();
        smod(&misplaced, "machine_sample");
        let manifest = "id='plain'\nname='Plain'\nversion='1.0.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n";
        let plain = sunrise_mod_package::Package::write_files(manifest, &[]).unwrap();
        std::fs::write(misplaced.join("plain.smod"), plain).unwrap();
        let base = f.0.join("resources");
        std::fs::create_dir_all(&base).unwrap();
        // A prepared root's registry report: block entity registration reads it (B1).
        std::fs::write(
            base.join("registries.json"),
            "{\"minecraft:block_entity_type\":{\"entries\":{}}}",
        )
        .unwrap();
        std::fs::write(base.join("sunrise-resources.json"), "{}").unwrap();
        std::fs::write(base.join("block-behaviours.json"), r#"{"fluids":[]}"#).unwrap();
        std::env::set_current_dir(&here).unwrap();
        let mut sources = ModsState {
            native: std::sync::Arc::new(native),
            ..Default::default()
        };
        let enabled = std::collections::BTreeSet::from(["plain".to_owned()]);
        sources.select(&here, &enabled);
        let generated = content_root(&base, &sources).unwrap();
        assert_ne!(generated.prepared(), base);
        assert_eq!(
            sunrise_mod_package::enabled::EnabledMods::read(generated.prepared())
                .unwrap()
                .server_ids()
                .collect::<Vec<_>>(),
            ["plain"],
        );
        let local = sunrise_protocol::mod_list::local();
        let ids: Vec<_> = local.iter().map(|m| m.id.as_str()).collect();
        if mode.as_deref() == Some("compiled") {
            assert_eq!(ids, ["machine_sample", "plain"]);
            assert_eq!(local[0].version, "0.3.0");
        } else {
            assert_eq!(ids, ["plain"]);
        }
        // Out of the fixture: Windows won't remove a process's working directory.
        std::env::set_current_dir(std::env::temp_dir()).unwrap();
    }

    /// A built native package whose own content declares a remainder that is not an item is
    /// refused with the package: its code and Parts never start without the content it declared.
    #[test]
    #[ignore = "daily"]
    fn a_built_native_package_with_a_missing_remainder_is_refused_whole() {
        const MODE: &str = "SUNRISE_NATIVE_REMAINDER_TEST";
        if std::env::var_os(MODE).is_none() {
            run_in_child(
                "game::mods::tests::a_built_native_package_with_a_missing_remainder_is_refused_whole",
                MODE,
            );
            return;
        }
        let f = Fixture::new("native-missing-remainder");
        let text = format!(
            "id='machine_sample'\nname='machine_sample'\nversion='0.3.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{}'\nmixins='{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let code: &[(&str, &[u8])] = &[
            ("Cargo.toml", b"[package]\nname='example'\nversion='1'\n"),
            ("src/lib.rs", b"pub fn hooks() {}"),
            (
                "data/machine_sample/sunrise/item/stew.json",
                br#"{"template":"minecraft:mushroom_stew","crafting_remainder":"machine_sample:missing_bowl"}"#,
            ),
        ];
        let path = f.0.join("machine_sample.smod");
        std::fs::write(
            &path,
            sunrise_mod_package::Package::write_files(&text, code).unwrap(),
        )
        .unwrap();
        let selected: Arc<[sunrise_mod_package::selection::SelectedPackage]> =
            vec![sunrise_mod_package::selection::SelectedPackage {
                package: sunrise_mod_package::Package::read(&path).unwrap(),
                path,
            }]
            .into();
        let sources = ModsState {
            native: Arc::new(f.runtime()),
            selected,
            ..Default::default()
        };
        let base = f.0.join("vanilla");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("sunrise-resources.json"), "{}").unwrap();
        std::fs::write(base.join("facts.json"), r#"{"states":[],"defaults":{}}"#).unwrap();
        std::fs::write(base.join("gameplay-facts.json"), r#"{"states":[]}"#).unwrap();
        std::fs::write(base.join("items.json"), r#"{"minecraft:mushroom_stew":{}}"#).unwrap();
        let error = content_overlay(&base, &sources, &f.0.join("jars"))
            .err()
            .unwrap();
        assert_eq!(
            error.to_string(),
            r#"machine_sample: native content refused at admission: machine_sample:stew: crafting_remainder "machine_sample:missing_bowl" is not an item"#
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_shared_missing_jar_stops_native_load_and_refuses_each_content_mod_row() {
        const MODE: &str = "SUNRISE_JAR_REFUSAL_TEST";
        if std::env::var_os(MODE).is_none() {
            // Native declarations, like the real startup, belong to one process.
            run_in_child(
                "game::mods::tests::a_shared_missing_jar_stops_native_load_and_refuses_each_content_mod_row",
                MODE,
            );
            return;
        }
        let f = Fixture::new("shared-jar-refusal");
        let jar = sunrise_mod_package::manifest::JarRef::new(
            "original".into(),
            "1".into(),
            "9".repeat(128),
        )
        .unwrap();
        for id in ["machine_sample", "alpha", "beta", "gamma"] {
            let native = id == "machine_sample";
            let mut text = format!(
                "id='{id}'\nname='{id}'\nversion='0.3.0'\napi='0.2'\nside='both'\ncapabilities=['registries']\n"
            );
            if native {
                text.push_str(&format!(
                    "[native]\ngame='26.3'\nsunrise='{}'\nmixins='{}'\n",
                    "a".repeat(40),
                    "b".repeat(40)
                ));
            }
            if id != "gamma" {
                text.push_str(&format!(
                    "[[jar]]\nid='original'\nversion='1'\nsha512='{}'\n",
                    jar.sha512()
                ));
            }
            let code: &[(&str, &[u8])] = if native {
                &[
                    ("Cargo.toml", b"[package]\nname='example'\nversion='1'\n"),
                    ("src/lib.rs", b"pub fn hooks() {}"),
                ]
            } else {
                &[]
            };
            std::fs::write(
                f.0.join(format!("{id}.smod")),
                sunrise_mod_package::Package::write_files(&text, code).unwrap(),
            )
            .unwrap();
        }
        let selected: Arc<[sunrise_mod_package::selection::SelectedPackage]> =
            ["alpha", "beta", "gamma"]
                .into_iter()
                .map(|id| {
                    let path = f.0.join(format!("{id}.smod"));
                    sunrise_mod_package::selection::SelectedPackage {
                        package: sunrise_mod_package::Package::read(&path).unwrap(),
                        path,
                    }
                })
                .collect();
        let native = ModsState {
            native: Arc::new(f.runtime()),
            selected: selected.clone(),
            ..Default::default()
        };
        let base = f.0.join("vanilla");
        let store = f.0.join("jars");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("sunrise-resources.json"), "{}").unwrap();
        let error = content_overlay(&base, &native, &store).err().unwrap();
        let missing = error.downcast_ref::<MissingJar>().unwrap();
        assert_eq!(missing.owners, ["machine_sample", "alpha", "beta"]);
        assert_eq!(missing.jar, jar);
        assert_eq!(missing.path, jar.cache_path(&store));
        assert!(!store.exists());

        let content = ModsState {
            selected,
            ..Default::default()
        };
        let overlay = content_overlay(&base, &content, &store).unwrap();
        assert_ne!(overlay.root.prepared(), base);
        assert_eq!(
            sunrise_mod_package::enabled::EnabledMods::read(overlay.root.prepared())
                .unwrap()
                .server_ids()
                .collect::<Vec<_>>(),
            ["gamma"],
        );
        assert_eq!(
            overlay
                .mods
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["gamma"]
        );
        let mut rows: Vec<_> = ["alpha", "beta", "gamma"]
            .into_iter()
            .map(|id| Installed {
                id: id.into(),
                title: id.into(),
                description: String::new(),
                problem: None,
                dev: false,
                native: None,
            })
            .collect();
        content.report_refusals(&mut rows);
        for row in &rows[..2] {
            let problem = row.problem.as_deref().unwrap();
            assert!(problem.contains("alpha, beta: original 1:"));
            assert!(problem.contains(&jar.cache_path(&store).display().to_string()));
        }
        assert!(rows[2].problem.is_none());
    }

    #[test]
    fn the_crash_report_lists_the_embedded_native_mods() {
        let built = [
            BUILT[0],
            BuiltMod {
                id: "pipes",
                version: "2.0",
                side: mod_api::Side::Client,
            },
        ];
        assert_eq!(
            native_mods_detail(&built).as_deref(),
            Some("machine_sample 0.3.0, pipes 2.0")
        );
        assert_eq!(native_mods_detail(&[]), None);
    }

    #[test]
    fn native_mods_installed_but_not_built_get_a_row_saying_so() {
        let f = Fixture::new("not-built");
        for id in ["veinminer", "zoom", "minimap", "machine_sample"] {
            smod(&f.0, id);
        }
        std::fs::write(f.0.join("notes.txt"), "not a mod").unwrap();
        std::fs::write(f.0.join("broken.smod"), "not a zip").unwrap();
        let rows = not_built(&f.0, &[], &[]);
        let ids: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["machine_sample", "minimap", "veinminer", "zoom"]);
        assert_eq!(rows[2].title, "veinminer 0.3.0");
        assert_eq!(rows[2].problem.as_deref(), Some(mod_list::NOT_BUILT));
        assert!(rows.iter().all(|r| r.native.is_none() && !r.dev));
        let listed = Installed {
            id: "minimap".into(),
            ..rows[0].clone()
        };
        let rows = not_built(&f.0, BUILT, &[listed]);
        let ids: Vec<_> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["veinminer", "zoom"]);
        assert!(not_built(&f.0.join("missing"), &[], &[]).is_empty());
    }

    /// A root with mods that declare no mob effects leaves the registry open for the first one
    /// that does.
    #[test]
    fn a_root_without_mob_effects_does_not_freeze_the_registry() {
        use sunrise_level::brewing;
        let _scope = sunrise_game_data::potions::isolate_mod_effects();
        let f = Fixture::new("no-effects");
        register_mob_effects(&[plain_mod(&f, None)], &[]);
        assert!(brewing::register_mod_effects([brewing::ModEffect::new(
            "test:effect",
            0,
            brewing::Category::Neutral,
            "test-potions",
        )]));
    }

    /// #4473: a content mod's declared `attribute_modifiers` register onto its
    /// effect definition in declaration order.
    #[test]
    fn a_declared_effect_registers_its_attribute_modifiers() {
        use sunrise_world::attributes::Operation;
        let _scope = sunrise_game_data::potions::isolate_mod_effects();
        let f = Fixture::new("effect-modifiers");
        let stun = r#"{"category": "harmful", "color": 1, "attribute_modifiers": [
            {"attribute": "minecraft:movement_speed", "id": "plain:stun_speed", "amount": -1.0, "operation": "add_multiplied_total"},
            {"attribute": "minecraft:armor", "id": "plain:stun_armor", "amount": 2.5, "operation": "add_value"},
            {"attribute": "minecraft:scale", "id": "plain:stun_scale", "amount": 0.5, "operation": "add_multiplied_base"}]}"#;
        register_mob_effects(&[plain_mod(&f, Some(stun))], &[]);
        let stun = sunrise_level::brewing::effect("plain:stun").unwrap();
        assert_eq!(
            stun.modifiers,
            [
                (
                    "minecraft:movement_speed",
                    "plain:stun_speed",
                    -1.0,
                    Operation::AddMultipliedTotal
                ),
                (
                    "minecraft:armor",
                    "plain:stun_armor",
                    2.5,
                    Operation::AddValue
                ),
                (
                    "minecraft:scale",
                    "plain:stun_scale",
                    0.5,
                    Operation::AddMultipliedBase
                ),
            ]
        );
    }

    /// A content mod `plain` in `f`, declaring the effect `plain:stun` when given.
    #[cfg(test)]
    fn plain_mod(f: &Fixture, stun: Option<&str>) -> sunrise_sim::mod_content::ContentMod {
        std::fs::write(
            f.0.join("sunrise-mod.toml"),
            "id = 'plain'\nname = 'plain'\nversion = '1.0'\napi = '0.2'\ncapabilities = ['registries']\n",
        )
        .unwrap();
        if let Some(stun) = stun {
            let dir = f.0.join("data/plain/sunrise/mob_effect");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("stun.json"), stun).unwrap();
        }
        let package = sunrise_mod_package::Package::read(&f.0).unwrap();
        sunrise_sim::mod_content::ContentMod {
            id: "plain".into(),
            content: sunrise_mod_package::content::Content::read(&f.0, &package.manifest).unwrap(),
            jars: Vec::new(),
        }
    }
}

pub(in crate::game) mod standalone;
