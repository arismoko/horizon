use super::*;
use crate::{
    crafting::Recipes,
    enchant::Enchantments,
    items::{ItemStack, Items},
    stations::StationRecipes,
};
use serde_json::json;

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("horizon-priorities-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("data/minecraft/recipe")).unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn matching_shapeless_default_priority_and_replace_empty() {
    let root = Fixture::new("shapeless");
    let items = Items::for_tests(HashMap::new());
    let mut files = vec![];
    for (id, output) in [("a", "minecraft:stick"), ("b", "minecraft:diamond")] {
        files.push((
            format!("recipe/test:{id}"),
            vec![
                serde_json::to_vec(&json!({
                    "type": "minecraft:crafting_shapeless", "ingredients": ["minecraft:stone"],
                    "result": {"id": output}
                }))
                .unwrap(),
            ],
        ));
    }
    let grid = [Some(ItemStack::new("minecraft:stone", 1)), None, None, None];
    let context = crate::test_resources::server_loop_context();
    let load = |files: &[_]| {
        Recipes::load(
            &root.0,
            &items,
            &PackOverlay::new(files),
            &crate::test_resources::server_load(),
        )
        .unwrap()
    };
    let recipes = load(&files);
    let (id, output) = recipes.find(context, &grid, 2).unwrap();
    assert_eq!((id, output.item.as_ref()), ("test:a", "minecraft:stick"));
    files.push((
        "recipe_priorities".into(),
        vec![br#"{"entries":{"test:b":5}}"#.to_vec()],
    ));
    let recipes = load(&files);
    let (id, output) = recipes.find(context, &grid, 2).unwrap();
    assert_eq!((id, output.item.as_ref()), ("test:b", "minecraft:diamond"));
    files
        .last_mut()
        .unwrap()
        .1
        .push(br#"{"replace":true}"#.to_vec());
    let recipes = load(&files);
    let (id, output) = recipes.find(context, &grid, 2).unwrap();
    assert_eq!((id, output.item.as_ref()), ("test:a", "minecraft:stick"));
}

#[test]
fn layers_conditions_unknown_ids_and_integer_boundaries() {
    let root = Fixture::new("layers");
    fs::write(
        root.0.join("data/minecraft/recipe_priorities.json"),
        br#"{"entries":{"test:a":3,"test:absent":17,"test:min":-2147483648}}"#,
    )
    .unwrap();
    let files = vec![("recipe_priorities".into(), vec![
        br#"{"entries":{"test:a":5,"test:b":-2}}"#.to_vec(),
        br#"{"replace":true,"neoforge:conditions":[{"type":"neoforge:not","value":{"type":"neoforge:true"}}]}"#.to_vec(),
        br#"{"entries":{"test:b":2147483647}}"#.to_vec(),
        // A malformed layer cannot clear previously accepted entries.
        br#"{"replace":true,"entries":{"test:a":2147483648}}"#.to_vec(),
    ])];
    let priorities = Priorities::load(&root.0, &PackOverlay::new(&files)).unwrap();
    assert_eq!(priorities.get("test:a"), 5);
    assert_eq!(priorities.get("test:b"), i32::MAX);
    assert_eq!(priorities.get("test:min"), i32::MIN);
    assert_eq!(priorities.0.get("test:absent"), Some(&17));
    assert_eq!(priorities.get("test:unspecified"), 0);
}

#[test]
fn every_projection_uses_the_same_priority_order() {
    let root = Fixture::new("projections");
    let items = Items::for_tests(HashMap::new());
    let mut sources = Vec::new();
    let mut entries = serde_json::Map::new();
    for suffix in ["a", "b", "c"] {
        for kind in [
            "smelting",
            "blasting",
            "smoking",
            "campfire_cooking",
            "smithing_transform",
            "stonecutting",
            "crusher",
        ] {
            let id = format!("test:{kind}_{suffix}");
            let recipe = match kind {
                "smithing_transform" => {
                    json!({"type":"minecraft:smithing_transform", "base":"minecraft:stone", "result":{"id":"minecraft:diamond"}})
                }
                "stonecutting" => {
                    json!({"type":"minecraft:stonecutting", "ingredient":"minecraft:stone", "result":{"id":"minecraft:diamond"}})
                }
                "crusher" => json!({"type":"test:crusher", "output":suffix}),
                _ => {
                    json!({"type":format!("minecraft:{kind}"), "ingredient":"minecraft:stone", "result":{"id":"minecraft:diamond"}})
                }
            };
            entries.insert(id.clone(), json!(if suffix == "a" { -1 } else { 5 }));
            sources.push((id, recipe));
        }
    }
    let files = vec![(
        "recipe_priorities".into(),
        vec![serde_json::to_vec(&json!({"entries":entries})).unwrap()],
    )];
    let overlay = PackOverlay::new(&files);
    let resolved = ResolvedSources::resolve_prioritized(
        &root.0,
        &overlay,
        sources,
        &crate::test_resources::server_load(),
    )
    .unwrap();
    let recipes = Recipes::from_resolved(&root.0, &items, &overlay, &resolved).unwrap();
    let stations = StationRecipes::from_sources(&root.0, &items, &resolved);
    let context = crate::test_resources::server_loop_context();
    let stone = ItemStack::new("minecraft:stone", 1);
    for kind in ["smelting", "blasting", "smoking", "campfire_cooking"] {
        assert_eq!(
            recipes
                .cook(context, &format!("minecraft:{kind}"), &stone)
                .unwrap()
                .id,
            format!("test:{kind}_b")
        );
    }
    assert_eq!(
        stations
            .smithing_recipe(context, &Enchantments::default(), None, Some(&stone), None)
            .unwrap()
            .0,
        "test:smithing_transform_b"
    );
    assert_eq!(
        stations
            .stonecutter(context, &stone)
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        [
            "test:stonecutting_b",
            "test:stonecutting_c",
            "test:stonecutting_a"
        ]
    );
    assert_eq!(
        recipes.by_type["test:crusher"]
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        ["test:crusher_b", "test:crusher_c", "test:crusher_a"]
    );
    for kind in [
        "smelting",
        "blasting",
        "smoking",
        "campfire_cooking",
        "smithing_transform",
        "stonecutting",
    ] {
        let ids: Vec<_> = recipes.by_type[&format!("minecraft:{kind}")]
            .iter()
            .map(|r| r.id.clone())
            .collect();
        assert_eq!(
            ids,
            [
                format!("test:{kind}_b"),
                format!("test:{kind}_c"),
                format!("test:{kind}_a")
            ]
        );
    }
}
