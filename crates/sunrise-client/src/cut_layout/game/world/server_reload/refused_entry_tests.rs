//! Rejected registry entries fail loads and preserve the current tables on reload.
use super::*;
use mod_api::{DataPackFilesEvent, ServerHost};

const GOOD_TABLE: &str = r#"{"pools":[]}"#;
const UNKNOWN_FUNCTION: &str = r#"{"modifier":{"type":"minecraft:no_such_function"},"pools":[]}"#;
const GOOD_RECIPE: &str = r#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stone"],"result":{"id":"minecraft:stick"}}"#;
const UNKNOWN_RESULT: &str = r#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stone"],"result":{"id":"refused_test:no_such_item"}}"#;
const MISSING_CRITERION: &str =
    r#"{"criteria":{"a":{"trigger":"minecraft:tick"}},"requirements":[["ghost"]]}"#;

struct Files(Arc<AtomicBool>, &'static str, &'static str);
#[mod_api::__subscriptions]
impl Hooks for Files {
    fn data_pack_files(&self, _: &mut dyn ServerHost, event: &mut DataPackFilesEvent<'_>) {
        event.files.push((
            "loot_table/refused_test:good".into(),
            GOOD_TABLE.as_bytes().to_vec(),
        ));
        event.files.push((
            "recipe/refused_test:good".into(),
            GOOD_RECIPE.as_bytes().to_vec(),
        ));
        if self.0.load(Ordering::SeqCst) {
            event
                .files
                .push((self.1.into(), self.2.as_bytes().to_vec()));
        }
    }
}

fn with_bad_file(
    bad: bool,
    path: &'static str,
    json: &'static str,
    run: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, &Arc<AtomicBool>),
) {
    let enabled = Arc::new(AtomicBool::new(bad));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "refused_test",
            Box::new(Files(enabled.clone(), path, json)),
        )]
        .into(),
        || with_game(|game| run(game, &enabled)),
    );
}

#[test]
#[ignore = "daily"]
fn an_unknown_loot_function_refuses_world_open() {
    with_bad_file(
        true,
        "loot_table/refused_test:bad",
        UNKNOWN_FUNCTION,
        |game, _| {
            let scratch = tempfile::tempdir().unwrap();
            assert!(game.load_world(request(scratch.path())).is_err());
        },
    );
}

#[test]
#[ignore = "daily"]
fn an_unknown_loot_function_fails_reload_and_keeps_the_old_tables() {
    with_bad_file(
        false,
        "loot_table/refused_test:bad",
        UNKNOWN_FUNCTION,
        |game, enabled| {
            let scratch = tempfile::tempdir().unwrap();
            game.load_world(request(scratch.path())).unwrap();
            let old = game.server.world_tables.0.as_ref().map(std::ptr::from_ref);
            enabled.store(true, Ordering::SeqCst);
            assert!(!game.run_command("reload"));
            assert_eq!(
                old,
                game.server.world_tables.0.as_ref().map(std::ptr::from_ref)
            );
        },
    );
}

#[test]
#[ignore = "daily"]
fn an_advancement_requiring_a_missing_criterion_refuses_the_load() {
    with_bad_file(
        false,
        "advancement/refused_test:ghost",
        MISSING_CRITERION,
        |game, enabled| {
            let scratch = tempfile::tempdir().unwrap();
            game.load_world(request(scratch.path())).unwrap();
            enabled.store(true, Ordering::SeqCst);
            assert!(!game.run_command("reload"));
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_recipe_with_an_unknown_result_item_refuses_world_open() {
    with_bad_file(
        true,
        "recipe/refused_test:bad",
        UNKNOWN_RESULT,
        |game, _| {
            let scratch = tempfile::tempdir().unwrap();
            assert!(game.load_world(request(scratch.path())).is_err());
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_recipe_with_an_unknown_result_item_fails_reload_and_keeps_the_old_recipes() {
    with_bad_file(
        false,
        "recipe/refused_test:bad",
        UNKNOWN_RESULT,
        |game, enabled| {
            let scratch = tempfile::tempdir().unwrap();
            game.load_world(request(scratch.path())).unwrap();
            let old = game.server.world_tables.0.as_ref().map(std::ptr::from_ref);
            enabled.store(true, Ordering::SeqCst);
            assert!(!game.run_command("reload"));
            assert_eq!(
                old,
                game.server.world_tables.0.as_ref().map(std::ptr::from_ref)
            );
        },
    );
}
