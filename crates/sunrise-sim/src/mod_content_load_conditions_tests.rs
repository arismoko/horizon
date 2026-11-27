use super::*;
use sunrise_world::conditions::LoadConditions;
/// Only 26.3's canonical worldgen feature folder is admitted.
#[test]
fn worldgen_entry_reads_the_feature_folder() {
    assert_eq!(
        worldgen_entry("data/zinc/worldgen/feature/ore_zinc.json"),
        Some(("worldgen/features.json", "zinc:ore_zinc".to_owned()))
    );
    for file in [
        "data/zinc/worldgen/configured_feature/ore_zinc.json",
        "data/zinc/worldgen/nope/ore_zinc.json",
        "data/zinc/data_maps/worldgen/biome/zinc_patch.json",
        "data/zinc/custom/worldgen/feature/ore_zinc.json",
        "data/minecraft/worldgen/biome/zinc_patch.json",
    ] {
        assert_eq!(worldgen_entry(file), None, "{file}");
    }
}

#[test]
#[ignore = "daily"]
fn registered_conditions_select_content_root_tags_and_structure_modifiers() {
    let scratch = tempfile::tempdir().unwrap();
    let Some(base) = prepared() else {
        return;
    };
    let inventory =
        sunrise_mod_package::enabled::EnabledMods::new([("testmod".into(), mod_api::Side::Both)]);
    let package = folder_mod(
        &scratch.path().join("mod"),
        &[
            ("sunrise-mod.toml", &manifest("testmod", "")),
            (
                "data/testmod/tags/block/flag.json",
                r#"{"neoforge:conditions":[{"type":"testmod:flag","value":true}],"values":["minecraft:stone"]}"#,
            ),
            (
                "data/testmod/neoforge/structure_modifier/rabbits.json",
                r#"{"neoforge:conditions":[{"type":"testmod:flag","value":true}],"type":"neoforge:add_spawns","structures":"minecraft:swamp_hut","spawners":{"type":"minecraft:rabbit","weight":3,"minCount":1,"maxCount":1}}"#,
            ),
        ],
    );
    let mut previous: Option<PathBuf> = None;
    for enabled in [true, false] {
        let conditions = LoadConditions::new(["testmod"])
            .with_lookup([("testmod:flag", move |args: &serde_json::Value| {
                Ok(enabled && args["value"].as_bool().unwrap())
            })]);
        let out = super::super::resource_root_with_conditions(
            &base,
            Path::new(""),
            std::slice::from_ref(&package),
            &scratch.path().join("cache"),
            &inventory,
            &conditions,
        )
        .unwrap();
        let tag_file = out.join("data/testmod/tags/block/flag.json");
        assert!(tag_file.exists());
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(tag_file).unwrap()).unwrap();
        assert_eq!(value["values"], serde_json::json!(["minecraft:stone"]));
        assert!(value.get("neoforge:conditions").is_none());
        let spawns = crate::mobs::spawn_selection::StructureSpawns::load(&out);
        let rabbits = spawns
            .get("minecraft:swamp_hut", "creature")
            .is_some_and(|entry| {
                entry
                    .spawns
                    .iter()
                    .any(|spawn| spawn.entity == "minecraft:rabbit")
            });
        assert_eq!(rabbits, enabled);
        assert_eq!(conditions.check_errors(), Ok(()));
        if let Some(old) = &previous {
            assert_ne!(old, &out);
            assert!(
                old.join("data/testmod/tags/block/flag.json").exists(),
                "old usable root was overwritten"
            );
        }
        previous = Some(out);
    }
}

#[test]
fn conditional_base_tag_does_not_gate_later_accepted_layers() {
    let base = r#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"code_only"}],"values":["minecraft:stone"]}"#;
    let more = [J::Str("minecraft:dirt".into())];
    for enabled in [false, true] {
        let merged: serde_json::Value = serde_json::from_str(
            &merge_tag(
                Some(base),
                &[TagLayers {
                    values: more.as_slice(),
                    remove: &[],
                }],
                false,
                "conditional",
                &sunrise_world::conditions::LoadConditions::new(if enabled {
                    vec!["code_only"]
                } else {
                    vec![]
                }),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            merged["values"],
            serde_json::json!(["minecraft:stone", "minecraft:dirt"])
        );
        assert!(merged.get("neoforge:conditions").is_none());
    }
}

#[test]
fn tags_merge_without_repeats() {
    let vanilla = r#"{"values": ["minecraft:stone"]}"#;
    let more = [
        J::Str("ruby:ruby_block".into()),
        J::Str("minecraft:stone".into()),
    ];
    let merged: serde_json::Value = serde_json::from_str(
        &merge_tag(
            Some(vanilla),
            &[TagLayers {
                values: more.as_slice(),
                remove: &[],
            }],
            false,
            "t",
            &sunrise_world::conditions::LoadConditions::vanilla(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        merged["values"],
        serde_json::json!(["minecraft:stone", "ruby:ruby_block", "minecraft:stone"])
    );
}

#[test]
#[ignore = "daily"]
fn failed_condition_build_removes_generated_staging() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = tempfile::tempdir().unwrap();
    let package = folder_mod(
        &scratch.path().join("module"),
        &[
            ("sunrise-mod.toml", &manifest("testmod", "")),
            (
                "data/testmod/neoforge/structure_modifier/bad.json",
                r#"{"neoforge:conditions":[{"type":"testmod:flag","value":"bad"}],"values":["minecraft:stone"]}"#,
            ),
        ],
    );
    let conditions = LoadConditions::new([])
        .with_lookup([("testmod:flag", |_: &serde_json::Value| {
            Err("bad value".into())
        })]);
    let out = scratch.path().join("candidate");
    let error = super::super::build_with_conditions(
        &base,
        Path::new(""),
        &[package],
        &out,
        &sunrise_mod_package::enabled::EnabledMods::default(),
        &conditions,
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("data/testmod/neoforge/structure_modifier/bad.json"),
        "{error}"
    );
    assert!(!out.exists());
    assert!(!scratch.path().join("candidate-tables").exists());
}

#[test]
fn removals_follow_the_tag_file_and_a_replace_drops_them() {
    let vanilla =
        r#"{"values": ["minecraft:stone", "minecraft:dirt"], "remove": ["minecraft:sand"]}"#;
    let ids = |ids: &[&str]| -> Vec<J> { ids.iter().map(|i| J::Str((*i).into())).collect() };
    let merged: serde_json::Value = serde_json::from_str(
        &merge_tag(
            Some(vanilla),
            &[TagLayers {
                values: &ids(&["test:ruby"]),
                remove: &ids(&["minecraft:dirt"]),
            }],
            false,
            "t",
            &LoadConditions::vanilla(),
        )
        .unwrap(),
    )
    .unwrap();
    // The vanilla file's removals apply right after its values, then the
    // mods' adds and removals join `values` in load order, each removal
    // carried as `{"id", "remove": true}`.
    assert_eq!(
        merged["values"],
        serde_json::json!(["minecraft:stone", "minecraft:dirt",
            {"id": "minecraft:sand", "remove": true}, "test:ruby",
            {"id": "minecraft:dirt", "remove": true}])
    );
    assert_eq!(merged["remove"], serde_json::Value::Null);
    let file = "data/minecraft/tags/item/things.json";
    let mut merge = TagMerge::default();
    merge.add("a", file, false, true, &ids(&["a:x"]));
    if let Some((_, remove)) = merge.layers.last_mut() {
        remove.extend(ids(&["minecraft:dirt"]));
    }
    merge.add("b", file, true, true, &ids(&["b:y"]));
    assert!(merge.layers.iter().all(|(_, remove)| remove.is_empty()));
}

/// The order the mods' layers keep: a lower mod's removal, then a higher
/// mod's add of the same entry, ends with the entry in the merged file, and
/// the folder walks it back in.
#[test]
fn a_higher_mod_adds_back_what_a_lower_one_removed() {
    let ids = |ids: &[&str]| -> Vec<J> { ids.iter().map(|i| J::Str((*i).into())).collect() };
    let file = "data/minecraft/tags/item/things.json";
    let mut merge = TagMerge::default();
    merge.add("low", file, false, true, &ids(&["a:x"]));
    if let Some((_, remove)) = merge.layers.last_mut() {
        remove.extend(ids(&["minecraft:sand"]));
    }
    merge.add("high", file, false, true, &ids(&["minecraft:sand", "b:y"]));
    merge.add("last", file, false, true, &[]);
    if let Some((_, remove)) = merge.layers.last_mut() {
        remove.extend(ids(&["minecraft:stone"]));
    }
    merge.add("restore", file, false, true, &ids(&["minecraft:stone"]));
    merge.add("remove-again", file, false, true, &[]);
    if let Some((_, remove)) = merge.layers.last_mut() {
        remove.extend(ids(&["minecraft:stone"]));
    }
    let merged: serde_json::Value = serde_json::from_str(
        &merge_tag(
            Some(r#"{"values": ["minecraft:stone", "minecraft:sand"]}"#),
            &merge.layers(),
            false,
            "t",
            &LoadConditions::vanilla(),
        )
        .unwrap(),
    )
    .unwrap();
    // The fold (TagLayers::build) reads the entries in order: x, sand out,
    // sand back in, y.
    let layers = sunrise_world::tag_layers::TagLayers::from_file(merged).unwrap();
    assert_eq!(
        layers.build(|_| true, |_| None).unwrap(),
        ["a:x", "minecraft:sand", "b:y"]
    );
}

#[test]
#[ignore = "daily"]
fn content_root_tag_layers_ignore_failing_top_level_conditions() {
    let Some(base) = prepared() else { return };
    let scratch = tempfile::tempdir().unwrap();
    let file = "data/minecraft/tags/block/conditional_probe.json";
    let packages = [
        ("lower", r#"{"values":["minecraft:stone"]}"#),
        (
            "conditional",
            r#"{"neoforge:conditions":[{"type":"neoforge:never"}],"values":["minecraft:dirt"]}"#,
        ),
    ];
    let mods: Vec<_> = packages
        .into_iter()
        .map(|(id, tag)| {
            folder_mod(
                &scratch.path().join(id),
                &[("sunrise-mod.toml", &manifest(id, "")), (file, tag)],
            )
        })
        .collect();
    let root =
        resource_root_fixture(&base, Path::new(""), &mods, &scratch.path().join("cache")).unwrap();
    let merged: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join(file)).unwrap()).unwrap();
    assert_eq!(
        merged["values"],
        serde_json::json!(["minecraft:stone", "minecraft:dirt"])
    );
}

#[test]
fn unknown_vanilla_tag_references_do_not_break_mod_tags() {
    use sunrise_world::worldgen::tags::TagDefinitions;

    let scratch = tempfile::tempdir().unwrap();
    let base = resources(scratch.path());
    let vanilla = r#"{"values":["minecraft:stone"]}"#;
    for name in TAG_BUNDLES {
        let bundle = BTreeMap::from([("minecraft:fixture/current", vanilla)]);
        fs::write(base.join(name), serde_json::to_vec(&bundle).unwrap()).unwrap();
    }
    for kind in ["block", "item"] {
        let path = |ns: &str, name: &str| format!("data/{ns}/tags/{kind}/{name}.json");
        let current = base.join(path("minecraft", "fixture/current"));
        fs::create_dir_all(current.parent().unwrap()).unwrap();
        fs::write(current, vanilla).unwrap();
        let combined = path("testmod", "combined");
        let m = folder_mod(
            &scratch.path().join(kind),
            &[
                ("sunrise-mod.toml", &manifest("testmod", "")),
                (
                    &combined,
                    r##"{"values":["#minecraft:fixture/removed", "#minecraft:fixture/current", "#testmod:local", "#minecraft:fixture/from_mod", {"id":"#minecraft:fixture/optional", "required":false}]}"##,
                ),
                (&path("testmod", "local"), r#"{"values":[]}"#),
                (&path("minecraft", "fixture/from_mod"), r#"{"values":[]}"#),
            ],
        );
        let out = scratch.path().join(format!("out-{kind}"));
        let warnings = build_fixture(&base, Path::new(""), &[m], &out).unwrap();
        if kind == "block" {
            for name in TAG_BUNDLES {
                let bundle: BTreeMap<String, String> =
                    serde_json::from_slice(&fs::read(out.join(name)).unwrap()).unwrap();
                let mut tags = TagDefinitions::default();
                for (id, text) in bundle {
                    tags.apply_json(&id, name, &text).unwrap();
                }
                let built = tags.build(|id, _| Some(id.to_owned())).unwrap();
                assert!(built.missing.is_empty(), "{name}: {:?}", built.missing);
                assert_eq!(built.tags["testmod:combined"], ["minecraft:stone"]);
            }
        }
        let merged: serde_json::Value =
            serde_json::from_slice(&fs::read(out.join(&combined)).unwrap()).unwrap();
        assert_eq!(
            merged["values"],
            serde_json::json!([
                "#minecraft:fixture/current",
                "#testmod:local",
                "#minecraft:fixture/from_mod"
            ])
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            warnings[0].contains(&combined)
                && warnings[0].contains("testmod:combined")
                && warnings[0].contains("#minecraft:fixture/removed"),
            "{warnings:?}"
        );
    }
}

#[test]
fn no_vanilla_tag_folder_keeps_vanilla_references() {
    let scratch = tempfile::tempdir().unwrap();
    let base = resources(scratch.path());
    let mods = mod_in(
        scratch.path(),
        &tag_files(r##""#minecraft:fixture/unlisted""##),
    );
    let out = scratch.path().join("out");
    let warnings = build_fixture(&base, Path::new(""), &mods, &out).unwrap();
    let merged: serde_json::Value =
        serde_json::from_slice(&fs::read(out.join("data/testmod/tags/item/a.json")).unwrap())
            .unwrap();
    assert_eq!(
        merged["values"],
        serde_json::json!(["#minecraft:fixture/unlisted"])
    );
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn content_mod_worldgen_bundles_and_additions_evaluate_load_conditions() {
    let scratch = tempfile::tempdir().unwrap();
    let base = resources(scratch.path());
    fs::create_dir_all(base.join("worldgen")).unwrap();
    fs::write(base.join("worldgen/features.json"), "{}").unwrap();
    fs::write(
        base.join("worldgen/template_pools.json"),
        serde_json::json!({
            "minecraft:test_pool": r#"{"name":"minecraft:test_pool","fallback":"minecraft:empty","elements":[]}"#
        })
        .to_string(),
    )
    .unwrap();
    let package = folder_mod(
        &scratch.path().join("mod"),
        &[
            ("sunrise-mod.toml", &manifest("boiler", "")),
            (
                "data/boiler/worldgen/feature/gated_ore.json",
                r#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"chemlib"}],"type":"minecraft:no_op","config":{}}"#,
            ),
            (
                "data/boiler/worldgen/template_pool/pool_add.json",
                r#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"chemlib"}],"sunrise:add_to":"minecraft:test_pool","elements":[{"weight":1,"element":{"element_type":"minecraft:empty_pool_element"}}]}"#,
            ),
        ],
    );
    let inventory_on = sunrise_mod_package::enabled::EnabledMods::new([
        ("boiler".into(), mod_api::Side::Both),
        ("chemlib".into(), mod_api::Side::Both),
    ]);
    let out_on = scratch.path().join("out_on");
    super::super::build_with_conditions(
        &base,
        Path::new(""),
        std::slice::from_ref(&package),
        &out_on,
        &inventory_on,
        &LoadConditions::new(["boiler", "chemlib"]),
    )
    .unwrap();
    let features_on: serde_json::Value =
        serde_json::from_slice(&fs::read(out_on.join("worldgen/features.json")).unwrap()).unwrap();
    assert!(features_on.get("boiler:gated_ore").is_some());
    let pools_on: serde_json::Value =
        serde_json::from_slice(&fs::read(out_on.join("worldgen/template_pools.json")).unwrap())
            .unwrap();
    let pool_on: serde_json::Value =
        serde_json::from_str(pools_on["minecraft:test_pool"].as_str().unwrap()).unwrap();
    assert_eq!(pool_on["elements"].as_array().unwrap().len(), 1);

    let inventory_off =
        sunrise_mod_package::enabled::EnabledMods::new([("boiler".into(), mod_api::Side::Both)]);
    let out_off = scratch.path().join("out_off");
    super::super::build_with_conditions(
        &base,
        Path::new(""),
        std::slice::from_ref(&package),
        &out_off,
        &inventory_off,
        &LoadConditions::new(["boiler"]),
    )
    .unwrap();
    let features_off: serde_json::Value =
        serde_json::from_slice(&fs::read(out_off.join("worldgen/features.json")).unwrap()).unwrap();
    assert!(features_off.get("boiler:gated_ore").is_none());
    let pools_off: serde_json::Value =
        serde_json::from_slice(&fs::read(out_off.join("worldgen/template_pools.json")).unwrap())
            .unwrap();
    let pool_off: serde_json::Value =
        serde_json::from_str(pools_off["minecraft:test_pool"].as_str().unwrap()).unwrap();
    assert_eq!(pool_off["elements"].as_array().unwrap().len(), 0);

    // Malformed condition JSON fails atomically.
    let broken = folder_mod(
        &scratch.path().join("broken_mod"),
        &[
            ("sunrise-mod.toml", &manifest("broken", "")),
            (
                "data/broken/worldgen/feature/bad.json",
                r#"{"neoforge:conditions":[{"type":"unknown:bogus"}],"type":"minecraft:no_op","config":{}}"#,
            ),
        ],
    );
    let out_err = scratch.path().join("out_err");
    let err = super::super::build_with_conditions(
        &base,
        Path::new(""),
        std::slice::from_ref(&broken),
        &out_err,
        &sunrise_mod_package::enabled::EnabledMods::new([("broken".into(), mod_api::Side::Both)]),
        &LoadConditions::new(["broken"]),
    );
    assert!(
        err.is_err(),
        "unknown condition in content-mod worldgen must fail atomically"
    );
}
