use super::*;
use serde_json::json;

#[test]
fn test_environments_and_instances_decode_validate_and_sync() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("items.json"), b"{}").unwrap();
    let default_env = root.path().join("data/minecraft/test_environment");
    std::fs::create_dir_all(&default_env).unwrap();
    std::fs::write(
        default_env.join("default.json"),
        br#"{"type":"minecraft:all_of","definitions":[]}"#,
    )
    .unwrap();
    let clocks = root.path().join("data/minecraft/world_clock");
    std::fs::create_dir_all(&clocks).unwrap();
    std::fs::write(clocks.join("overworld.json"), b"{}").unwrap();
    let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
    let files = [
        (
            "test_environment/test:storm".into(),
            vec![
                json!({
                    "type": "minecraft:all_of",
                    "definitions": [
                        "minecraft:default",
                        {"type": "minecraft:weather", "weather": "thunder"},
                        {"type": "minecraft:game_rules", "rules": {"minecraft:advance_time": false}},
                        {"type": "minecraft:difficulty", "difficulty": "hard"},
                        {"type": "minecraft:clock_time", "clock": "minecraft:overworld", "time": 18000},
                        {"type": "minecraft:function", "setup": "test:on", "teardown": "test:off"}
                    ]
                })
                .to_string()
                .into_bytes(),
            ],
        ),
        (
            "test_instance/test:smoke".into(),
            vec![
                json!({
                    "type": "minecraft:block_based",
                    "environment": "test:storm",
                    "structure": "minecraft:empty",
                    "max_ticks": 20,
                    "setup_ticks": 1,
                    "required": true,
                    "rotation": "clockwise_90",
                    "padding": 2
                })
                .to_string()
                .into_bytes(),
            ],
        ),
    ];
    let packed = PackRegistryDefinitions::load(
        root.path(),
        &defaults,
        &PackOverlay::new(&files),
        &Default::default(),
        &Default::default(),
        |_| true,
    )
    .unwrap();
    let mut envs = Vec::new();
    packed.append_encoded("minecraft:test_environment", &mut envs);
    assert!(
        envs.iter()
            .any(|((ns, path), _, from_pack)| ns == "test" && path == "storm" && *from_pack),
        "pack test_environment is synced"
    );
    assert!(
        envs.iter()
            .any(|((ns, path), _, from_pack)| ns == "minecraft"
                && path == "default"
                && !*from_pack),
        "prepared default test_environment is kept"
    );
    let mut instances = Vec::new();
    packed.append_encoded("minecraft:test_instance", &mut instances);
    let (_, smoke_tag, from_pack) = instances
        .iter()
        .find(|((ns, path), _, _)| ns == "test" && path == "smoke")
        .expect("pack test_instance is synced");
    assert!(*from_pack);
    assert_eq!(
        smoke_tag.get("environment"),
        Some(&Tag::String("test:storm".into()))
    );
    assert_eq!(smoke_tag.get("max_ticks"), Some(&Tag::Int(20)));
    assert_eq!(smoke_tag.get("padding"), Some(&Tag::Int(2)));

    // Malformed definitions refuse the load and name their files.
    let bad_files = [
        (
            "test_instance/test:bad_ticks".into(),
            vec![
                json!({
                    "type": "minecraft:block_based",
                    "environment": "minecraft:default",
                    "structure": "minecraft:empty",
                    "max_ticks": 0
                })
                .to_string()
                .into_bytes(),
            ],
        ),
        (
            "test_instance/test:missing_env".into(),
            vec![
                json!({
                    "type": "minecraft:block_based",
                    "environment": "test:does_not_exist",
                    "structure": "minecraft:empty",
                    "max_ticks": 10
                })
                .to_string()
                .into_bytes(),
            ],
        ),
        (
            "test_instance/test:missing_fn".into(),
            vec![
                json!({
                    "type": "minecraft:function",
                    "environment": "minecraft:default",
                    "structure": "minecraft:empty",
                    "max_ticks": 10
                })
                .to_string()
                .into_bytes(),
            ],
        ),
        (
            "test_environment/test:bad_rule".into(),
            vec![
                json!({
                    "type": "minecraft:game_rules",
                    "rules": {"minecraft:advance_time": 42}
                })
                .to_string()
                .into_bytes(),
            ],
        ),
        (
            "test_environment/test:bad_time".into(),
            vec![
                json!({
                    "type": "minecraft:clock_time",
                    "clock": "minecraft:overworld",
                    "time": -1
                })
                .to_string()
                .into_bytes(),
            ],
        ),
        (
            "test_environment/test:bad_clock".into(),
            vec![
                json!({
                    "type": "minecraft:clock_time",
                    "clock": "test:missing_clock",
                    "time": 1
                })
                .to_string()
                .into_bytes(),
            ],
        ),
        (
            "test_environment/test:bad_timeline".into(),
            vec![
                json!({
                    "type": "minecraft:timeline_attributes",
                    "timelines": ["test:missing_timeline"]
                })
                .to_string()
                .into_bytes(),
            ],
        ),
    ];
    let error = PackRegistryDefinitions::load(
        root.path(),
        &defaults,
        &PackOverlay::new(&bad_files),
        &Default::default(),
        &Default::default(),
        |_| true,
    )
    .err()
    .expect("malformed definitions must be refused");
    for expected in [
        "data/test/test_instance/bad_ticks.json",
        "data/test/test_instance/missing_env.json",
        "data/test/test_instance/missing_fn.json",
        "data/test/test_environment/bad_rule.json",
        "data/test/test_environment/bad_time.json",
        "data/test/test_environment/bad_clock.json: unknown world_clock test:missing_clock",
        "data/test/test_environment/bad_timeline.json: unknown timeline test:missing_timeline",
    ] {
        assert!(error.contains(expected), "missing {expected} in:\n{error}");
    }
}
