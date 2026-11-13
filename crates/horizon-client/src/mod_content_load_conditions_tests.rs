use super::*;
use horizon_world::conditions::LoadConditions;
/// Only 26.3's canonical worldgen feature folder is admitted.
#[test]
fn worldgen_entry_reads_the_feature_folder() {
    assert_eq!(
        worldgen_entry("data/zinc/worldgen/feature/ore_zinc.json"),
        Some(("worldgen/features.json", "zinc:ore_zinc".to_owned()))
    );
    for folder in ["configured_feature", "nope"] {
        assert_eq!(
            worldgen_entry(&format!("data/zinc/worldgen/{folder}/ore_zinc.json")),
            None
        );
    }
}

#[test]
#[ignore = "full content root: requires prepared resources"]
fn registered_conditions_select_content_root_tags_and_structure_modifiers() {
    let scratch = tempfile::tempdir().unwrap();
    let Some(base) = prepared() else {
        return;
    };
    let inventory = horizon_mod_package::enabled::EnabledMods::new([(
        "testmod".into(),
        horizon_mod_package::Side::Both,
    )]);
    let package = folder_mod(
        &scratch.path().join("mod"),
        &[
            ("horizon-mod.toml", &manifest("testmod", "")),
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
        let conditions = LoadConditions::new(["testmod"]).with_lookup(
            BTreeMap::from([("testmod".into(), "testmod".into())]),
            move |name, args| {
                (name == "testmod:flag").then(|| Ok(enabled && args["value"].as_bool().unwrap()))
            },
        );
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
        assert_eq!(tag_file.exists(), enabled);
        if enabled {
            let value: serde_json::Value =
                serde_json::from_slice(&fs::read(tag_file).unwrap()).unwrap();
            assert_eq!(value["values"], serde_json::json!(["minecraft:stone"]));
            assert!(value.get("neoforge:conditions").is_none());
        }
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
                &more,
                "conditional",
                &horizon_world::conditions::LoadConditions::new(if enabled {
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
            if enabled {
                serde_json::json!(["minecraft:stone", "minecraft:dirt"])
            } else {
                serde_json::json!(["minecraft:dirt"])
            }
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
            &more,
            "t",
            &horizon_world::conditions::LoadConditions::vanilla(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        merged["values"],
        serde_json::json!(["minecraft:stone", "ruby:ruby_block"])
    );
}

#[test]
#[ignore = "full content root: requires prepared resources"]
fn failed_condition_build_removes_generated_staging() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = tempfile::tempdir().unwrap();
    let package = folder_mod(
        &scratch.path().join("module"),
        &[
            ("horizon-mod.toml", &manifest("testmod", "")),
            (
                "data/testmod/tags/block/bad.json",
                r#"{"neoforge:conditions":[{"type":"testmod:flag","value":"bad"}],"values":["minecraft:stone"]}"#,
            ),
        ],
    );
    let conditions = LoadConditions::new([]).with_lookup(
        BTreeMap::from([("testmod".into(), "testmod".into())]),
        |name, _| (name == "testmod:flag").then(|| Err("bad value".into())),
    );
    let out = scratch.path().join("candidate");
    let error = super::super::build_with_conditions(
        &base,
        Path::new(""),
        &[package],
        &out,
        &horizon_mod_package::enabled::EnabledMods::default(),
        &conditions,
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("data/testmod/tags/block/bad.json"),
        "{error}"
    );
    assert!(!out.exists());
    assert!(!scratch.path().join("candidate-tables").exists());
}
