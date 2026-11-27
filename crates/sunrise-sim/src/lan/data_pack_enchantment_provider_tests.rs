use super::{PackRegistryDefinitions, REGISTRY_KINDS};
use serde_json::{Value, json};
use sunrise_items::items::Items;
use sunrise_world::loot::PackOverlay;

fn enchantment(description: &str) -> Value {
    json!({
        "description": description, "weight": 1, "max_level": 1, "anvil_cost": 1,
        "min_cost": {"base": 1, "per_level_above_first": 0},
        "max_cost": {"base": 5, "per_level_above_first": 0},
        "supported_items": "minecraft:stick", "slots": ["hand"]
    })
}

#[test]
fn enchantment_provider_is_unsynced_validates_references_and_installs_pack_overrides() {
    assert!(
        REGISTRY_KINDS
            .iter()
            .any(|table| table.key == "minecraft:enchantment_provider"),
        "enchantment_provider row in REGISTRY_KINDS"
    );
    assert!(
        !crate::lan::data::SYNCHRONIZED.contains(&"minecraft:enchantment_provider"),
        "enchantment_provider must not be in SYNCHRONIZED"
    );

    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("items.json"),
        json!({"minecraft:stick": {}, "minecraft:iron_sword": {}}).to_string(),
    )
    .unwrap();
    let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();

    // Valid pack provider referencing an admitted enchantment and tag succeeds.
    let files = [
        (
            "enchantment/minecraft:sharpness".into(),
            vec![enchantment("Sharpness").to_string().into_bytes()],
        ),
        (
            "tags/minecraft:enchantment/test_pool".into(),
            vec![
                json!({"values": ["minecraft:sharpness"]})
                    .to_string()
                    .into_bytes(),
            ],
        ),
        (
            "enchantment_provider/minecraft:mob_spawn_equipment".into(),
            vec![
                json!({
                    "type": "minecraft:single",
                    "enchantment": "minecraft:sharpness",
                    "level": 5
                })
                .to_string()
                .into_bytes(),
            ],
        ),
        (
            "enchantment_provider/test:by_tag".into(),
            vec![
                json!({
                    "type": "minecraft:by_cost",
                    "enchantments": "#minecraft:test_pool",
                    "cost": 10
                })
                .to_string()
                .into_bytes(),
            ],
        ),
    ];
    let overlay = PackOverlay::new(&files);
    let packed = PackRegistryDefinitions::load(
        root.path(),
        &defaults,
        &overlay,
        &Default::default(),
        &Default::default(),
        |_| true,
    )
    .unwrap();
    let mut wire = Vec::new();
    packed.append_encoded("minecraft:enchantment_provider", &mut wire);
    assert!(wire.is_empty(), "enchantment_provider is not synchronized");

    let mut items = Items::tags_only(root.path(), &overlay, &defaults).unwrap();
    packed
        .install_items(&mut items, root.path(), &overlay, &defaults)
        .unwrap();
    assert!(
        items
            .enchantments
            .provider("minecraft:mob_spawn_equipment")
            .is_some()
    );
    assert!(items.enchantments.provider("test:by_tag").is_some());

    // Unbound enchantment holder and unbound enchantment tag refuse the load.
    let bad_files = [
        (
            "enchantment_provider/test:missing_holder".into(),
            vec![
                json!({
                    "type": "minecraft:single",
                    "enchantment": "minecraft:nonexistent",
                    "level": 1
                })
                .to_string()
                .into_bytes(),
            ],
        ),
        (
            "enchantment_provider/test:missing_tag".into(),
            vec![
                json!({
                    "type": "minecraft:by_cost_with_difficulty",
                    "enchantments": "#minecraft:nonexistent_tag",
                    "min_cost": 5,
                    "max_cost_span": 17
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
    .expect("unbound enchantment_provider references must refuse load");
    assert!(error.contains("test:missing_holder"), "{error}");
    assert!(error.contains("test:missing_tag"), "{error}");
}
