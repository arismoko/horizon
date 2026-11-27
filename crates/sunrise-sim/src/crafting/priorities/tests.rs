use super::*;
use crate::{
    crafting::Recipes,
    items::{ItemStack, Items},
};
use serde_json::json;
use std::fs;
use sunrise_mod_package::content::RECIPE_PRIORITIES_PATH;

fn layer(pack: &str, bytes: &[u8]) -> RecipePriorityLayer {
    RecipePriorityLayer {
        pack: pack.into(),
        path: RECIPE_PRIORITIES_PATH,
        contents: Ok(bytes.to_vec()),
    }
}

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("sunrise-priorities-{name}-{}", std::process::id()));
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
    let load = |priority_layers: &[_]| {
        Recipes::load(
            &root.0,
            &items,
            &PackOverlay {
                files: &files,
                priority_layers,
                ..PackOverlay::default()
            },
            &crate::test_resources::server_load(),
        )
        .unwrap()
    };
    let recipes = load(&[]);
    let (id, output) = recipes.find(context, &grid, 2).unwrap();
    assert_eq!((id, output.item.as_ref()), ("test:a", "minecraft:stick"));
    let mut priorities = vec![layer("preferred", br#"{"entries":{"test:b":5}}"#)];
    let recipes = load(&priorities);
    let (id, output) = recipes.find(context, &grid, 2).unwrap();
    assert_eq!((id, output.item.as_ref()), ("test:b", "minecraft:diamond"));
    priorities.push(layer("reset", br#"{"replace":true}"#));
    let recipes = load(&priorities);
    let (id, output) = recipes.find(context, &grid, 2).unwrap();
    assert_eq!((id, output.item.as_ref()), ("test:a", "minecraft:stick"));
}

#[test]
fn layers_conditions_unknown_ids_and_integer_boundaries() {
    let root = Fixture::new("layers");
    fs::create_dir_all(root.0.join("data/neoforge")).unwrap();
    fs::write(
        root.0.join(RECIPE_PRIORITIES_PATH),
        br#"{"entries":{"test:a":3,"test:absent":17,"test:min":-2147483648}}"#,
    )
    .unwrap();
    let priority_layers = [
        layer("first", br#"{"entries":{"test:a":5,"test:b":-2}}"#),
        layer(
            "skipped",
            br#"{"replace":true,"neoforge:conditions":[{"type":"neoforge:never"}]}"#,
        ),
        RecipePriorityLayer {
            pack: "unreadable".to_owned(),
            path: RECIPE_PRIORITIES_PATH,
            contents: Err("fixture failed read".to_owned()),
        },
        layer("last", br#"{"entries":{"test:b":2147483647}}"#),
    ];
    let priorities = Priorities::load(
        &root.0,
        &PackOverlay {
            priority_layers: &priority_layers,
            ..PackOverlay::default()
        },
    )
    .unwrap();
    assert_eq!(priorities.get("test:a"), 5);
    assert_eq!(priorities.get("test:b"), i32::MAX);
    assert_eq!(priorities.get("test:min"), i32::MIN);
    assert_eq!(priorities.0.get("test:absent"), Some(&17));
    assert_eq!(priorities.get("test:unspecified"), 0);
}

#[test]
fn an_invalid_layer_is_refused_whole_naming_its_pack() {
    let conditions = LoadConditions::vanilla();
    let mut priorities = Priorities::default();
    priorities
        .layer(&layer("good", br#"{"entries":{"test:a":5}}"#), &conditions)
        .unwrap();
    for json in [
        "[]",
        "5",
        r#"[false,{}]"#,
        r#"{"replace":"yes"}"#,
        r#"{"entries":[]}"#,
        r#"{"replace":true,"entries":{"test:a":2147483648}}"#,
        "{",
    ] {
        let error = priorities
            .layer(&layer("bad_pack", json.as_bytes()), &conditions)
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("bad_pack: data/neoforge/recipe_priorities.json"),
            "{json}: {error}"
        );
        assert_eq!(priorities.get("test:a"), 5);
    }
    // A layer with valid false conditions need not decode its value.
    priorities
        .layer(
            &layer(
                "off",
                br#"{"replace":"yes","neoforge:conditions":[{"type":"neoforge:never"}]}"#,
            ),
            &conditions,
        )
        .unwrap();
    assert_eq!(priorities.get("test:a"), 5);
}

#[test]
fn invalid_load_conditions_refuse_a_replacing_layer_without_clearing_priorities() {
    let conditions = LoadConditions::vanilla();
    let mut priorities = Priorities::default();
    priorities
        .layer(&layer("lower", br#"{"entries":{"test:a":5}}"#), &conditions)
        .unwrap();
    for cases in [
        json!(true),
        json!([false]),
        json!([{"type":"test:unknown"}]),
    ] {
        let bytes = serde_json::to_vec(&json!({
            "replace": true,
            "neoforge:conditions": cases,
            "entries": {"test:b": 12}
        }))
        .unwrap();
        let error = priorities
            .layer(&layer("higher", &bytes), &conditions)
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("higher: data/neoforge/recipe_priorities.json"),
            "{error}"
        );
        assert_eq!(priorities.get("test:a"), 5);
        assert_eq!(priorities.get("test:b"), 0);
    }
}

#[test]
fn recipe_identifiers_are_canonical_and_a_colliding_layer_applies_nothing() {
    let conditions = LoadConditions::vanilla();
    let mut priorities = Priorities::default();
    let apply = |priorities: &mut Priorities, bytes: &[u8]| {
        priorities.layer(&layer("test", bytes), &conditions)
    };
    apply(
        &mut priorities,
        br#"{"entries":{"a":5,"example:missing":9}}"#,
    )
    .unwrap();
    for json in [
        br#"{"replace":true,"entries":{"a":1,"minecraft:a":2}}"#.as_slice(),
        br#"{"replace":true,"entries":{":a":1,"a":2}}"#,
        br#"{"replace":true,"entries":{"a":1,"A":2}}"#,
    ] {
        assert!(apply(&mut priorities, json).is_err());
        assert_eq!(priorities.get("minecraft:a"), 5);
        assert_eq!(priorities.get("example:missing"), 9);
    }
    apply(&mut priorities, br#"{"entries":{":a":12}}"#).unwrap();
    assert_eq!(priorities.get("a"), 12);
    assert_eq!(priorities.get("example:missing"), 9);
}
