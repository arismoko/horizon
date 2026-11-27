//! Exercise the actual world-open and command-reload publication coordinator.
use super::*;
use mod_api::{
    DatapackField, DatapackFieldKind, DatapackRegistrySync, DatapackValue, Registrar,
    RegistrationContext,
};
use sunrise_core::nbt::Tag;

struct RegistryMod;
#[mod_api::__subscriptions]
impl Hooks for RegistryMod {
    fn data_pack_files(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::DataPackFilesEvent<'_>,
    ) {
        event.files.extend([
            (
                "predicate/registry_test:native".into(),
                br#"{"type":"minecraft:random_chance","chance":1}"#.to_vec(),
            ),
            (
                "item_modifier/registry_test:native".into(),
                br#"{"type":"minecraft:set_count","count":3}"#.to_vec(),
            ),
        ]);
    }
    fn data_map_types(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::DataMapTypesEvent<'_>,
    ) {
        event.types.push(mod_api::game::DataMapType {
            id: "registry_test:weights".into(),
            registry: "registry_test:spell".into(),
        });
    }
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.declare_datapack_registry(
            "spell",
            DatapackRegistrySync::ServerOnly,
            vec![DatapackField::required(
                "power",
                DatapackFieldKind::Int { min: 0, max: 100 },
            )],
        )
        .unwrap();
    }
}
fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}
#[test]
#[ignore = "daily"]
fn recipe_priority_layer_is_named_and_applied_through_reload() {
    use sunrise_mod_package::content::RECIPE_PRIORITIES_PATH;
    let _publication = sunrise_sim::datapack_net::isolate_server_publication();
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let pack = root.join("datapacks/priority_fixture");
    write(
        &pack.join("pack.mcmeta"),
        r#"{"pack":{"description":"fixture","min_format":94,"max_format":94}}"#,
    );
    write(
        &root.join("datapacks.json"),
        r#"{"enabled":["vanilla","file/priority_fixture"],"disabled":[]}"#,
    );
    for id in ["a", "b"] {
        write(
            &pack.join(format!("data/priority_fixture/recipe/{id}.json")),
            r#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stone"],"result":{"id":"minecraft:stick"}}"#,
        );
    }
    let path = pack.join(RECIPE_PRIORITIES_PATH);
    write(&path, r#"{"entries":{"priority_fixture:b":5}}"#);
    with_game(|game| {
        let order = |game: &sunrise_gameplay::game::ServerGame<'_>| {
            game.recipes().by_type["minecraft:crafting"]
                .iter()
                .filter(|row| row.id.starts_with("priority_fixture:"))
                .map(|row| row.id.clone())
                .collect::<Vec<_>>()
        };
        game.load_world(request(root)).unwrap();
        assert_eq!(order(game), ["priority_fixture:b", "priority_fixture:a"]);
        write(
            &path,
            r#"{"replace":true,"entries":{"priority_fixture:a":10}}"#,
        );
        assert!(game.run_command("reload"));
        assert_eq!(order(game), ["priority_fixture:a", "priority_fixture:b"]);
        let layers = game.pack_files().priority_layers;
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0].pack, "file/priority_fixture");
        assert_eq!(layers[0].path, "data/neoforge/recipe_priorities.json");
        write(&path, "{");
        let error = game
            .reload_datapacks(true, &game.level.enabled_features.clone())
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("file/priority_fixture: data/neoforge/recipe_priorities.json"),
            "{error}"
        );
        assert_eq!(order(game), ["priority_fixture:a", "priority_fixture:b"]);
    });
}

/// `InstrumentItem.use`: the horn plays the world's instrument, a data pack's new one or its
/// override of a vanilla one, at the sound and range the pack gives it.
#[test]
#[ignore = "daily"]
fn goat_horn_use_plays_the_data_pack_instrument() {
    let _publication = sunrise_client::datapack_net::isolate_server_publication();
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let pack = root.join("datapacks/horns");
    write(
        &pack.join("pack.mcmeta"),
        r#"{"pack":{"description":"fixture","min_format":94,"max_format":94}}"#,
    );
    write(
        &root.join("datapacks.json"),
        r#"{"enabled":["vanilla","file/horns"],"disabled":[]}"#,
    );
    write(
        &pack.join("data/minecraft/instrument/ponder_goat_horn.json"),
        r#"{"sound_event":"minecraft:item.goat_horn.sound.1","use_duration":3,"range":32,"description":"override"}"#,
    );
    write(
        &pack.join("data/test/instrument/horn.json"),
        r#"{"sound_event":"minecraft:item.goat_horn.sound.2","use_duration":5,"range":64,"description":"added"}"#,
    );
    let played = |instrument: Option<&str>| {
        let mut played = Vec::new();
        let mut cooldown = Vec::new();
        with_game(|game| {
            game.load_world(request(root)).unwrap();
            // `finish_open_world` clears `self.player`; `play_player_bound_sound` looks up the
            // player entity's state before emitting the bound sound on the server path.
            game.player = Some(
                sunrise_client::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::new(0.5, 200.0, 0.5),
                )
                .unwrap(),
            );
            let mut horn = sunrise_client::items::ItemStack::new("minecraft:goat_horn", 1);
            if let Some(id) = instrument {
                horn.data_mut()
                    .set_other("minecraft:instrument", Some(Tag::String(id.into())));
            }
            game.selected = 0;
            game.inventory[0] = Some(horn);
            game.offhand = None;
            game.start_audio_capture();
            game.cooldowns = Default::default();
            assert!(game.use_item().unwrap().is_some());
            played = std::mem::take(&mut game.server.pending_sounds)
                .into_iter()
                .map(|sound| (sound.event, sound.volume))
                .collect::<Vec<_>>();
            game.stop_audio_capture();
            cooldown = game
                .cooldowns
                .remaining()
                .map(|(group, ticks)| (group.to_string(), ticks))
                .collect::<Vec<_>>();
        });
        (played, cooldown)
    };
    // The default horn is the overridden ponder one: range 32 is volume 32 / 16, and
    // `use_duration` 3 s is a 60 tick cooldown (`Mth.floor(useDuration * 20)`).
    assert_eq!(
        played(None),
        (
            vec![("minecraft:item.goat_horn.sound.1".to_owned(), 2.0)],
            vec![("minecraft:goat_horn".to_owned(), 60)]
        )
    );
    assert_eq!(
        played(Some("test:horn")),
        (
            vec![("minecraft:item.goat_horn.sound.2".to_owned(), 4.0)],
            vec![("minecraft:goat_horn".to_owned(), 100)]
        )
    );
}

fn power(game: &sunrise_gameplay::game::ServerGame<'_>) -> DatapackValue {
    let set = game.mods.datapack_registries.as_ref().unwrap();
    set.get("registry_test:spell", "registry_test:fire")
        .unwrap()
        .get("power")
        .unwrap()
        .clone()
}
#[test]
#[ignore = "daily"]
fn datapack_game_open_reload_and_exhaustion_are_atomic() {
    let _publication = sunrise_sim::datapack_net::isolate_server_publication();
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    for (name, value) in [("base", 3), ("override", 7)] {
        let pack = root.join("datapacks").join(name);
        write(
            &pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"fixture","min_format":94,"max_format":94}}"#,
        );
        write(
            &pack.join("data/registry_test/registry_test/spell/fire.json"),
            &format!(r#"{{"power":{value}}}"#),
        );
    }
    write(
        &root.join("datapacks.json"),
        r#"{"enabled":["vanilla","file/base","file/override"],"disabled":[]}"#,
    );
    let winning = root.join("datapacks/override/data/registry_test/registry_test/spell/fire.json");
    let tag = root.join("datapacks/base/data/registry_test/tags/block/publication.json");
    write(&tag, r#"{"values":["minecraft:stone"]}"#);
    let conditioned = "data/registry_test/registry_test/spell/conditioned.json";
    write(
        &root.join("datapacks/base").join(conditioned),
        r#"{"power":3}"#,
    );
    write(
        &root.join("datapacks/override").join(conditioned),
        r#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"missing_condition_fixture"}],"power":"invalid"}"#,
    );
    mod_platform::testing::with_mods(
        vec![ModEntry::new("registry_test", Box::new(RegistryMod))].into(),
        || {
            with_game(|game| {
                let builtin = root.join("builtin");
                for kind in ["predicate", "item_modifier"] {
                    write(
                        &builtin.join(format!("data/registry_test/{kind}/native.json")),
                        r#"{"neoforge:conditions":[{"type":"absent:overridden"}]}"#,
                    );
                }
                let source = sunrise_assets::resource_source::ResourceSource::directory(builtin);
                game.pack.data_source = source.clone();
                game.pack.data_changes = source;
                game.load_world(request(root)).unwrap();
                assert_eq!(
                    game.server.fns.0.store.predicates["registry_test:native"]["chance"],
                    1
                );
                assert_eq!(
                    game.server.fns.0.store.item_modifiers["registry_test:native"]["count"],
                    3
                );
                assert_eq!(power(game), DatapackValue::Int(7));
                assert_eq!(
                    game.mods
                        .datapack_registries
                        .as_ref()
                        .unwrap()
                        .entries("registry_test:spell")
                        .unwrap()
                        .collect::<Vec<_>>(),
                    ["registry_test:fire"]
                );
                let old = game.mods.datapack_registries.as_ref().unwrap().clone();
                write(
                    &winning,
                    r#"{"power":8,"neoforge:conditions":[{"type":"neoforge:or","values":[{"type":"neoforge:always"},{"type":"absent:spell_enabled"}]}]}"#,
                );
                let bad_entries = ["predicate", "advancement", "recipe"].map(|kind| {
                    let path = format!("data/registry_test/{kind}/bad.json");
                    let file = root.join("datapacks/override").join(&path);
                    let id = format!("absent:{kind}");
                    write(
                        &file,
                        &serde_json::json!({"neoforge:conditions":[{"type":id}]}).to_string(),
                    );
                    (path, id, file)
                });
                let referencing_loot =
                    root.join("datapacks/override/data/registry_test/loot_table/reference.json");
                write(
                    &referencing_loot,
                    r#"{"pools":[{"rolls":1,"condition":"registry_test:bad","entries":[{"type":"minecraft:item","name":"minecraft:stick"}]}]}"#,
                );
                let features = game.level.enabled_features.clone();
                let error = game
                    .reload_datapacks(true, &features)
                    .err()
                    .expect("unknown ID refuses the winning registry entry")
                    .to_string();
                assert!(
                    error.contains("data/registry_test/registry_test/spell/fire.json"),
                    "{error}"
                );
                assert!(error.contains("absent:spell_enabled"), "{error}");
                assert!(
                    error.contains("unknown reference") && error.contains("registry_test:bad"),
                    "{error}"
                );
                std::fs::remove_file(referencing_loot).unwrap();
                for (path, id, file) in bad_entries {
                    assert!(error.contains(&path) && error.contains(&id), "{error}");
                    std::fs::remove_file(file).unwrap();
                }
                assert!(std::sync::Arc::ptr_eq(
                    game.mods.datapack_registries.as_ref().unwrap(),
                    &old
                ));
                assert!(std::sync::Arc::ptr_eq(
                    &sunrise_sim::datapack_net::served().unwrap(),
                    &old
                ));
                assert_eq!(power(game), DatapackValue::Int(7));
                let old_files = game
                    .server
                    .world_tables
                    .0
                    .as_ref()
                    .map(|tables| tables.files.clone());
                write(&winning, "{bad");
                write(&tag, r#"{"values":["minecraft:dirt"]}"#);
                assert!(!game.run_command("reload"));
                assert_eq!(power(game), DatapackValue::Int(7));
                assert!(std::sync::Arc::ptr_eq(
                    game.mods.datapack_registries.as_ref().unwrap(),
                    &old
                ));
                assert_eq!(
                    game.server
                        .world_tables
                        .0
                        .as_ref()
                        .map(|tables| tables.files.clone()),
                    old_files
                );
                write(&winning, r#"{"power":9}"#);
                assert!(game.run_command("reload"));
                assert_eq!(power(game), DatapackValue::Int(9));
                assert_eq!(
                    sunrise_sim::datapack_net::served(),
                    game.mods.datapack_registries
                );
                assert!(game.run_command("reload"));
                assert_eq!(power(game), DatapackValue::Int(9));
                let features = game.level.enabled_features.clone();
                game.reload_datapacks(true, &features).unwrap();
                assert_eq!(power(game), DatapackValue::Int(9));
                assert!(game.run_command("datapack disable \"file/override\""));
                assert_eq!(power(game), DatapackValue::Int(3));
                assert!(game.run_command("datapack enable \"file/override\" last"));
                assert_eq!(power(game), DatapackValue::Int(9));
                game.reload_datapacks(false, &features).unwrap();
                assert_eq!(power(game), DatapackValue::Int(9));
                game.finish_close_world_server();
                sunrise_gameplay_common::game::lan_common::sync::forget_sync_data();
                sunrise_gameplay_common::game::lan_common::sync::forget_server_sources();
                game.options.execution_role =
                    sunrise_gameplay_common::game::execution_role::ExecutionRole::DedicatedServer;
                game.load_world(request(root)).unwrap();
                assert_eq!(power(game), DatapackValue::Int(9));
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn world_open_and_wire_use_the_pack_biome_instead_of_the_bundled_file() {
    use sunrise_sim::lan::data::BIOME_REGISTRY;
    use {
        sunrise_gameplay::game::native_entity_refs::tests::fixture_root,
        sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
        sunrise_gameplay::game::native_entity_refs::tests::with_game_at,
    };

    // World open replaces the process-wide wire snapshot for the session.
    if run_fresh_cases(
        "cut_layout::game::server_side::server_reload::tests::datapack_publication_tests::world_open_and_wire_use_the_pack_biome_instead_of_the_bundled_file",
        "SUNRISE_PACK_BIOME_WIRE_CASE",
        &["world-open"],
    ) {
        return;
    }

    fn copy_tree(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    let scratch = tempfile::tempdir().unwrap();
    let resources = scratch.path().join("resources");
    copy_tree(&fixture_root(), &resources);
    let bundle_path = resources.join("worldgen/biomes.json");
    let mut bundle: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&bundle_path).unwrap()).unwrap();
    let mut winner: serde_json::Value =
        serde_json::from_str(bundle["minecraft:plains"].as_str().unwrap()).unwrap();
    let id = "registry_test:replacement";
    let mut bundled = winner.clone();
    bundled["neoforge:conditions"] = serde_json::json!([{"type":"absent:bundled_biome"}]);
    bundle[id] = serde_json::json!(bundled.to_string());
    write(&bundle_path, &bundle.to_string());

    let world = scratch.path().join("world");
    let pack = world.join("datapacks/override");
    write(
        &pack.join("pack.mcmeta"),
        r#"{"pack":{"description":"fixture","min_format":94,"max_format":94}}"#,
    );
    write(
        &world.join("datapacks.json"),
        r#"{"enabled":["vanilla","file/override"],"disabled":[]}"#,
    );
    winner["temperature"] = serde_json::json!(0.125);
    write(
        &pack.join("data/registry_test/worldgen/biome/replacement.json"),
        &winner.to_string(),
    );
    // A packed damage type makes world open decode the wire registries eagerly.
    write(
        &pack.join("data/registry_test/damage_type/fixture.json"),
        r#"{"message_id":"fixture","scaling":"never","exhaustion":0}"#,
    );
    let _publication = sunrise_sim::datapack_net::isolate_server_publication();
    with_game_at(resources, |game| {
        game.load_world(request(&world)).unwrap();
        let sync = sunrise_gameplay_common::game::lan_common::sync::cached_reload_sync().unwrap();
        let biome = sync
            .registry_packets(true, &game.server.level_shapes)
            .into_iter()
            .find(|registry| registry.registry == BIOME_REGISTRY)
            .unwrap()
            .entries
            .into_iter()
            .find(|entry| entry.id == id)
            .unwrap();
        let data = biome.data.expect("missing encoded biome winner");
        assert_eq!(
            data.get("temperature"),
            Some(&Tag::Float(0.125_f32.to_bits()))
        );
        assert!(data.get("carvers").is_none());
        assert!(data.get("features").is_none());
    });
}

/// A reload over a decode limit, or with a bad winning file, is refused naming the pack and
/// file; the old generation stays live and the next valid reload publishes.
#[test]
#[ignore = "daily"]
fn refused_reload_names_the_pack_and_keeps_the_old_generation() {
    let _publication = sunrise_sim::datapack_net::isolate_server_publication();
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let pack = root.join("datapacks/override");
    write(
        &pack.join("pack.mcmeta"),
        r#"{"pack":{"description":"fixture","min_format":94,"max_format":94}}"#,
    );
    let winning = pack.join("data/registry_test/registry_test/spell/fire.json");
    write(&winning, r#"{"power":7}"#);
    write(
        &root.join("datapacks.json"),
        r#"{"enabled":["vanilla","file/override"],"disabled":[]}"#,
    );
    mod_platform::testing::with_mods(
        vec![ModEntry::new("registry_test", Box::new(RegistryMod))].into(),
        || {
            with_game(|game| {
                game.load_world(request(root)).unwrap();
                let features = game.level.enabled_features.clone();
                let initial = game.mods.datapack_registries.as_ref().unwrap().clone();
                assert_eq!(power(game), DatapackValue::Int(7));
                let wide = format!(r#"{{"power":9{}}}"#, " ".repeat(1 << 20));
                write(&winning, &wide);
                game.reload_datapacks(true, &features).unwrap();
                assert_eq!(power(game), DatapackValue::Int(9));
                let published = game.mods.datapack_registries.as_ref().unwrap().clone();
                assert!(!std::sync::Arc::ptr_eq(&published, &initial));
                assert!(std::sync::Arc::ptr_eq(
                    sunrise_sim::datapack_net::served().as_ref().unwrap(),
                    &published
                ));

                write(&winning, "{bad");
                let Err(mod_api::__private::reload::ReloadStageError::DataSource(message)) =
                    game.reload_datapacks(true, &features)
                else {
                    panic!("the reload is refused");
                };
                assert!(
                    message.contains(
                        "pack file/override, file data/registry_test/registry_test/spell/fire.json"
                    ) && message.contains("Malformed"),
                    "{message}"
                );
                assert_eq!(power(game), DatapackValue::Int(9));
                assert!(std::sync::Arc::ptr_eq(
                    game.mods.datapack_registries.as_ref().unwrap(),
                    &published
                ));
                assert!(std::sync::Arc::ptr_eq(
                    sunrise_sim::datapack_net::served().as_ref().unwrap(),
                    &published
                ));

                write(&winning, r#"{"power":11}"#);
                assert!(game.run_command("reload"));
                assert_eq!(power(game), DatapackValue::Int(11));
            });
        },
    );
}
#[path = "datapack_host_read_tests.rs"]
mod datapack_host_read_tests;

#[path = "datapack_game_delivery_tests.rs"]
mod datapack_game_delivery_tests;

// Each case uses real world open/reload, not a preloaded membership predicate.
fn with_membership_fixture(run: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, &Path)) {
    let _publication = sunrise_sim::datapack_net::isolate_server_publication();
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    for name in ["base", "later"] {
        write(
            &root.join(format!("datapacks/{name}/pack.mcmeta")),
            r#"{"pack":{"description":"fixture","min_format":94,"max_format":94}}"#,
        );
    }
    write(
        &root.join("datapacks/base/data/registry_test/registry_test/spell/fire.json"),
        r#"{"power":3}"#,
    );
    write(
        &root.join("datapacks/later/data/registry_test/registry_test/spell/ice.json"),
        r#"{"power":7}"#,
    );
    write(
        &root.join("datapacks/base/data/registry_test/data_maps/registry_test/spell/weights.json"),
        r#"{"values":{"registry_test:fire":11,"registry_test:ice":17,"registry_test:ghost":99}}"#,
    );
    write(
        &root.join("datapacks.json"),
        r#"{"enabled":["vanilla","file/base"],"disabled":["file/later"]}"#,
    );
    mod_platform::testing::with_mods(
        vec![ModEntry::new("registry_test", Box::new(RegistryMod))].into(),
        || {
            with_game(|game| {
                game.load_world(request(root)).unwrap();
                run(game, root);
            })
        },
    );
}
fn weights<'game>(game: &'game sunrise_gameplay::game::ServerGame<'_>) -> &'game [(String, Tag)] {
    game.server
        .world_tables
        .0
        .as_ref()
        .unwrap()
        .data_maps
        .entries("registry_test:spell", "registry_test:weights")
        .unwrap()
}
#[test]
#[ignore = "daily"]
fn data_map_first_open_uses_staged_membership() {
    with_membership_fixture(|game, _| {
        assert_eq!(
            weights(game),
            &[("registry_test:fire".to_owned(), Tag::Int(11))]
        );
    });
}
#[test]
#[ignore = "daily"]
fn data_map_reload_add_uses_staged_membership() {
    with_membership_fixture(|game, root| {
        write(
            &root.join("datapacks/base/data/registry_test/registry_test/spell/ice.json"),
            r#"{"power":5}"#,
        );
        assert!(game.run_command("reload"));
        assert_eq!(
            weights(game),
            &[
                ("registry_test:fire".to_owned(), Tag::Int(11)),
                ("registry_test:ice".to_owned(), Tag::Int(17)),
            ]
        );
    });
}
#[test]
#[ignore = "daily"]
fn data_map_reload_remove_uses_staged_membership() {
    with_membership_fixture(|game, root| {
        std::fs::remove_file(
            root.join("datapacks/base/data/registry_test/registry_test/spell/fire.json"),
        )
        .unwrap();
        assert!(game.run_command("reload"));
        assert!(weights(game).is_empty());
    });
}
#[test]
#[ignore = "daily"]
fn data_map_later_enabled_layer_uses_staged_membership() {
    with_membership_fixture(|game, _| {
        assert!(game.run_command("datapack enable \"file/later\" last"));
        assert_eq!(
            weights(game),
            &[
                ("registry_test:fire".to_owned(), Tag::Int(11)),
                ("registry_test:ice".to_owned(), Tag::Int(17)),
            ]
        );
    });
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;
