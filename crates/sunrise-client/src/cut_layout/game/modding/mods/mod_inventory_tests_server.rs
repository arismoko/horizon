use super::*;
use mod_api::ModEntry;
use mod_platform::testing::with_mods;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use sunrise_gameplay::game::mods::tests::inventory::EmptyCode;
use sunrise_gameplay::game::mods::tests::inventory::Guard;
use sunrise_gameplay_common::game::mods::ModsState;
use sunrise_gameplay_common::game::mods::content_overlay;
use sunrise_mod_package::enabled::EnabledMods;

#[test]
#[ignore = "daily"]
fn code_only_reload_publishes_one_new_inventory_after_prepare_and_preserves_old_readers() {
    let fixture = Fixture::new("inventory-reload");
    smod(&fixture.0, "machine_sample");
    let sources = ModsState {
        native: Arc::new(fixture.runtime()),
        ..Default::default()
    };
    let reject = Arc::new(AtomicBool::new(false));
    with_mods(
        vec![
            ModEntry::new("machine_sample", Box::new(EmptyCode)),
            ModEntry::new("reload_guard", Box::new(Guard(reject.clone()))),
        ]
        .into(),
        || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let original = game.options.resources.clone();
                let initial = content_overlay(&original, &sources, &fixture.0.join("jars"))
                    .unwrap()
                    .root;
                game.options.resources = initial.prepared().to_owned();
                game.pack.data_source = initial.data().clone();
                game.pack.data_changes = initial.data_changes().clone();
                game.mods = sources.clone();
                game.server.world_root = fixture.0.join("world");
                let pack = game.server.world_root.join("datapacks/inventory");
                let recipes = pack.join("data/inventorytest/recipe");
                std::fs::create_dir_all(&recipes).unwrap();
                std::fs::write(pack.join("pack.mcmeta"), serde_json::to_vec(&serde_json::json!({
                "pack": {"description":"inventory fixture", "min_format":sunrise_core::version::DATA_PACK_FORMAT, "max_format":sunrise_core::version::DATA_PACK_FORMAT}
            })).unwrap()).unwrap();
                std::fs::write(recipes.join("requires_code.json"), br#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stone"],"result":{"id":"minecraft:stick"},"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"machine_sample"}]}"#).unwrap();
                let tags = pack.join("data/inventorytest/tags/item");
                std::fs::create_dir_all(&tags).unwrap();
                std::fs::write(tags.join("requires_code.json"), br#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"machine_sample"}],"values":["minecraft:diamond"]}"#).unwrap();
                for (kind, mut value) in [
                    ("loot_table", serde_json::json!({"pools":[]})),
                    (
                        "predicate",
                        serde_json::json!({"type":"minecraft:random_chance","chance":1}),
                    ),
                    (
                        "item_modifier",
                        serde_json::json!({"type":"minecraft:set_count","count":1}),
                    ),
                    (
                        "jukebox_song",
                        serde_json::json!({"sound_event":"minecraft:music_disc.cat","length_in_seconds":1,"comparator_output":1,"description":"condition test"}),
                    ),
                    (
                        "instrument",
                        serde_json::json!({"sound_event":"minecraft:item.goat_horn.sound.0","use_duration":7,"range":256,"description":"condition test"}),
                    ),
                    (
                        "enchantment",
                        serde_json::json!({
                            "description":"condition test", "supported_items":"minecraft:stick", "weight":1, "max_level":3,
                            "min_cost":{"base":1,"per_level_above_first":0}, "max_cost":{"base":1,"per_level_above_first":0},
                            "anvil_cost":1, "slots":["mainhand"]
                        }),
                    ),
                ] {
                    value["neoforge:conditions"] = serde_json::json!([{"type":"neoforge:mod_loaded","modid":"machine_sample"}]);
                    let dir = pack.join("data/inventorytest").join(kind);
                    std::fs::create_dir_all(&dir).unwrap();
                    std::fs::write(dir.join("requires_code.json"), value.to_string()).unwrap();
                    std::fs::write(
                        dir.join("unsupported.json"),
                        br#"{"neoforge:conditions":[{"type":"neoforge:never"}]}"#,
                    )
                    .unwrap();
                }
                // This fixture skips world open, which normally loads the built-in sound holders.
                let registries =
                    sunrise_gameplay_common::game::chat_common::command_export(&original, false)
                        .expect("prepared command registries");
                game.ui.registries.fill_from_exported(&registries);
                game.reload_datapacks(true, &[]).unwrap();
                let loaded = |game: &sunrise_gameplay::game::ServerGame<'_>, id: &str| {
                    let items = game.items();
                    (
                        items.has_loot_table(id),
                        items.songs.contains_key(id),
                        items.instruments.contains_key(id),
                        items.enchantments.list.iter().any(|e| &*e.id == id),
                        game.server.fns.0.store.predicates.contains_key(id),
                        game.server.fns.0.store.item_modifiers.contains_key(id),
                    )
                };
                assert_eq!(
                    loaded(game, "inventorytest:requires_code"),
                    (true, true, true, true, true, true)
                );
                assert_eq!(
                    loaded(game, "inventorytest:unsupported"),
                    (false, false, false, false, false, false)
                );
                let old_songs = game.items().songs.clone();
                let read_tags =
                    |game: &sunrise_gameplay::game::ServerGame<'_>| game.items().item_tags.clone();
                let read = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                    sunrise_gameplay::game::server_side::lan::sync::server_sources(
                        &game.options.resources,
                        &game.res.core.item_data,
                        game.items(),
                        &game.pack_files(),
                    )
                    .unwrap()
                };
                let has_recipe = |sources: &sunrise_sim::crafting::ResolvedSources| {
                    sources
                        .recipes
                        .iter()
                        .any(|recipe| recipe.name == "inventorytest:requires_code")
                };
                let old = read(game);
                assert!(has_recipe(&old));
                let old_tags = read_tags(game);
                assert!(old_tags["inventorytest:requires_code"].contains("minecraft:diamond"));
                let old_root = game.options.resources.clone();
                let old_data = game.pack.data_source.clone();
                assert!(mod_api::__private::set_mod_enabled("machine_sample", false));
                reject.store(true, Ordering::Relaxed);
                assert!(game.reload_datapacks(true, &[]).is_err());
                assert_eq!(game.options.resources, old_root);
                assert!(game.pack.data_source.same_generation(&old_data));
                assert!(
                    read_tags(game)["inventorytest:requires_code"].contains("minecraft:diamond")
                );
                assert_eq!(
                    loaded(game, "inventorytest:requires_code"),
                    (true, true, true, true, true, true)
                );
                assert_eq!(
                    EnabledMods::read(&old_root)
                        .unwrap()
                        .server_ids()
                        .collect::<Vec<_>>(),
                    ["machine_sample"]
                );
                reject.store(false, Ordering::Relaxed);
                game.reload_datapacks(true, &[]).unwrap();
                assert_eq!(game.options.resources, original);
                assert!(!has_recipe(&read(game)));
                // RegistryDataLoader.WORLD_REGISTRIES includes songs, instruments and enchantments.
                assert_eq!(
                    loaded(game, "inventorytest:requires_code"),
                    (false, true, true, true, false, false)
                );
                assert!(old_songs.contains_key("inventorytest:requires_code"));
                // Tag files ignore load conditions, including mod_loaded.
                assert!(
                    read_tags(game)["inventorytest:requires_code"].contains("minecraft:diamond")
                );
                assert!(old_tags["inventorytest:requires_code"].contains("minecraft:diamond"));
                assert!(has_recipe(&old));
                assert!(mod_api::__private::set_mod_enabled("machine_sample", true));
                game.reload_datapacks(true, &[]).unwrap();
                assert!(has_recipe(&read(game)));
                assert_eq!(
                    loaded(game, "inventorytest:requires_code"),
                    (true, true, true, true, true, true)
                );
                assert!(
                    read_tags(game)["inventorytest:requires_code"].contains("minecraft:diamond")
                );
                assert_eq!(game.options.resources, old_root);
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn content_mod_dropped_for_foreign_minecraft_remainder_is_absent_from_save_and_join_inventories() {
    use mod_api::Hooks;
    use sunrise_mod_package::selection::SelectedPackage;

    if sunrise_gameplay_common::game::test_fixtures::run_fresh_cases(
        "cut_layout::game::server_side::mods_server::tests::inventory::content_mod_dropped_for_foreign_minecraft_remainder_is_absent_from_save_and_join_inventories",
        "SUNRISE_DROPPED_REMAINDER_INVENTORY_CHILD",
        &["valid"],
    ) {
        return;
    }

    struct NativeMinecraftItem;
    #[mod_api::__subscriptions]
    impl Hooks for NativeMinecraftItem {
        fn items(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::ItemsEvent<'_>) {
            event
                .register(mod_api::Item::new(
                    "minecraft:foreign_bucket",
                    "minecraft:bucket",
                ))
                .unwrap();
        }
    }

    let fixture = Fixture::new("dropped-remainder-inventory");
    let consumer_manifest = "id='consumer'\nname='consumer'\nversion='1'\napi='0.2'\nside='both'\ncapabilities=['registries']\n";
    let consumer_path = fixture.0.join("consumer.smod");
    std::fs::write(
        &consumer_path,
        sunrise_mod_package::Package::write_files(
            consumer_manifest,
            &[(
                "data/consumer/sunrise/item/filled.json",
                br#"{"template":"minecraft:bucket","crafting_remainder":"minecraft:missing_bucket"}"#,
            )],
        )
        .unwrap(),
    )
    .unwrap();
    let consumer = sunrise_mod_package::Package::read(&consumer_path).unwrap();
    let read = sunrise_mod_package::Content::read(&consumer_path, &consumer.manifest).unwrap();
    assert_eq!(read.items.len(), 1);
    assert_eq!(read.items[0].id, "consumer:filled");
    assert_eq!(
        read.items[0].crafting_remainder.as_deref(),
        Some("minecraft:missing_bucket")
    );

    struct DropLog;
    static DROPPED: AtomicBool = AtomicBool::new(false);
    impl log::Log for DropLog {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, record: &log::Record<'_>) {
            if record.level() == log::Level::Warn
                && record.args().to_string().contains(
                    "mod consumer content not loaded: consumer:filled: crafting_remainder \"minecraft:missing_bucket\" is not an item",
                )
            {
                DROPPED.store(true, Ordering::Relaxed);
            }
        }
        fn flush(&self) {}
    }
    log::set_logger(&DropLog).unwrap();
    log::set_max_level(log::LevelFilter::Warn);

    let survivor_manifest = "id='survivor'\nname='survivor'\nversion='1'\napi='0.2'\nside='both'\n";
    let survivor_path = fixture.0.join("survivor.smod");
    std::fs::write(
        &survivor_path,
        sunrise_mod_package::Package::write_files(survivor_manifest, &[]).unwrap(),
    )
    .unwrap();
    let sources = ModsState {
        selected: vec![
            SelectedPackage {
                package: consumer,
                path: consumer_path,
            },
            SelectedPackage {
                package: sunrise_mod_package::Package::read(&survivor_path).unwrap(),
                path: survivor_path,
            },
        ]
        .into(),
        ..Default::default()
    };
    let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    with_mods(
        vec![ModEntry::new("owner_mod", Box::new(NativeMinecraftItem))].into(),
        || {
            let overlay = content_overlay(&base, &sources, &fixture.0.join("jars")).unwrap();
            assert!(overlay.announce);
            assert!(
                DROPPED.load(Ordering::Relaxed),
                "consumer must reach remainder admission"
            );
            assert!(overlay.mods.iter().any(|m| {
                m.id == "owner_mod"
                    && m.content
                        .items
                        .iter()
                        .any(|item| item.id == "minecraft:foreign_bucket")
            }));
            assert!(!overlay.mods.iter().any(|m| m.id == "consumer"));
            assert!(!overlay.content_mods.iter().any(|m| m.id == "consumer"));
            assert!(!overlay.announced.iter().any(|m| m.id == "consumer"));
            assert!(overlay.mods.iter().any(|m| m.id == "survivor"));
            assert!(overlay.content_mods.iter().any(|m| m.id == "survivor"));
            assert!(overlay.announced.iter().any(|m| m.id == "survivor"));
        },
    );
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;
