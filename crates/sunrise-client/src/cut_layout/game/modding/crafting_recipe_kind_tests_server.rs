use crate::game::crafting_recipe_kind_tests::Probe;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

#[test]
#[ignore = "daily"]
fn unregistered_serializer_failed_reload_preserves_previous_recipes() {
    let bad_recipe = Arc::new(AtomicBool::new(false));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "recipe_probe",
        Box::new(Probe {
            tosses: Arc::new(AtomicUsize::new(0)),
            bad_recipe: bad_recipe.clone(),
        }),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        super::native_entity_refs::tests::with_game(|game| {
            let scratch = tempfile::tempdir().unwrap();
            game.server.world_root = scratch.path().to_path_buf();
            let recipes = sunrise_sim::crafting::Recipes::from_sources(
                &game.options.resources,
                game.items(),
                &sunrise_world::loot::PackOverlay::NONE,
                vec![(
                    "recipe_probe:literal".into(),
                    serde_json::json!({"type":"recipe_probe:literal"}),
                )],
                game.res.core.item_data.context(),
            )
            .unwrap();
            game.reload_datapacks(false, &[]).unwrap();
            game.server.world_tables.0.as_mut().unwrap().recipes = recipes;
            assert!(game.recipes().recipe("recipe_probe:literal").is_some());
            bad_recipe.store(true, Ordering::Relaxed);
            let error = game
                .reload_datapacks(true, &game.level.enabled_features.clone())
                .err()
                .expect("unknown serializer must fail reload");
            let text = format!("{error:?}");
            assert!(
                text.contains("bad.json") && text.contains("missing:serializer"),
                "{text}"
            );
            assert!(game.recipes().recipe("recipe_probe:literal").is_some());
            assert!(game.recipes().recipe("recipe_probe:bad").is_none());
        });
    });
}

use sunrise_gameplay_common::game::tables::GameItems as _;
