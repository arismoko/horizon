#![cfg(test)]
use std::sync::Arc;
use sunrise_core::nbt::Tag;
use sunrise_gameplay::game::commands::execute::ExecState;
use sunrise_gameplay_common::game::commands_common::Target;
use sunrise_level::datapack::Function;
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_gameplay_common::game::tables::GameItems;

    use super::*;

    #[test]
    #[ignore = "daily"]
    fn mob_entity_membership_follows_world_pack_reload() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::server_side::functions::tests::mob_entity_membership_follows_world_pack_reload",
            || {
                sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                    let world = tempfile::tempdir().unwrap();
                    game.server.world_root = world.path().to_owned();
                    let pack = world.path().join("datapacks/snow_walkers");
                    let tags = pack.join("data/minecraft/tags/entity_type");
                    std::fs::create_dir_all(&tags).unwrap();
                    let f = sunrise_core::version::DATA_PACK_FORMAT;
                    std::fs::write(
                        pack.join("pack.mcmeta"),
                        serde_json::json!({"pack": {
                            "description": "Snow walkers", "pack_format": f.0,
                            "min_format": [f.0, f.1], "max_format": [f.0, f.1]
                        }})
                        .to_string(),
                    )
                    .unwrap();
                    let tag_file = tags.join("powder_snow_walkable_mobs.json");
                    std::fs::write(&tag_file, r#"{"replace":true,"values":["minecraft:cow"]}"#)
                        .unwrap();
                    game.reload_datapacks(false, &[]).unwrap();
                    let walkable = "minecraft:powder_snow_walkable_mobs";
                    let world = game.mob_world_base(&[]);
                    assert!(world.entity_has(walkable, "minecraft:cow"));
                    assert!(!world.entity_has(walkable, "minecraft:fox"));

                    std::fs::write(&tag_file, r#"{"replace":true,"values":["minecraft:fox"]}"#)
                        .unwrap();
                    // This unopened-world fixture has no operator player. Run at console authority.
                    assert!(game.as_console(|game| game.run_command("reload")));
                    let world = game.mob_world_base(&[]);
                    assert!(!world.entity_has(walkable, "minecraft:cow"));
                    assert!(world.entity_has(walkable, "minecraft:fox"));
                });
            },
        );
    }
    #[test]
    #[ignore = "daily"]
    fn first_world_open_admits_block_function() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let folder = "first-open-block-function";
            let world = std::env::current_dir().unwrap().join("saves").join(folder);
            let pack = world.join("datapacks/first_open");
            let functions = pack.join("data/first_open/function");
            std::fs::create_dir_all(&functions).unwrap();
            let f = sunrise_core::version::DATA_PACK_FORMAT;
            std::fs::write(
                pack.join("pack.mcmeta"),
                serde_json::json!({"pack":{
                    "description":"First open regression", "pack_format":f.0,
                    "min_format":[f.0,f.1], "max_format":[f.0,f.1]
                }})
                .to_string(),
            )
            .unwrap();
            std::fs::write(
                functions.join("block.mcfunction"),
                "execute if block 0 64 0 minecraft:repeater[delay=2] run return 1\n",
            )
            .unwrap();
            std::fs::write(
                functions.join("invalid.mcfunction"),
                "setblock 0 64 0 first_open:missing_block\n",
            )
            .unwrap();
            let request = sunrise_gameplay::game::world_loading::WorldRequest::new(
                folder,
                None,
                Some(3042),
                true,
                None,
                game.level.rule_registry.clone(),
            )
            .unwrap();
            game.load_world(request).unwrap();
            assert!(
                game.server
                    .fns
                    .0
                    .store
                    .functions
                    .contains_key("first_open:block"),
                "first-open admission errors: {:?}",
                game.server.fns.0.store.errors
            );
            assert!(
                !game
                    .server
                    .fns
                    .0
                    .store
                    .functions
                    .contains_key("first_open:invalid")
            );
            assert!(
                game.server
                    .fns
                    .0
                    .store
                    .errors
                    .iter()
                    .any(|(id, _)| id == "first_open:invalid")
            );
        });
    }
    /// #4449: the mods' commands are in the tree before functions load, from the first world open
    /// on, so a plain line and a macro line naming one both load and run; an unknown command is
    /// refused as before, and a command the mods stop registering is refused after a reload.
    #[test]
    #[ignore = "daily"]
    fn functions_name_mod_commands_from_the_first_world_open() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::server_side::functions::tests::functions_name_mod_commands_from_the_first_world_open",
            || {
                use std::sync::{
                    Arc,
                    atomic::{AtomicBool, AtomicU32, Ordering},
                };
                struct Commands {
                    offered: Arc<AtomicBool>,
                    ran: Arc<AtomicU32>,
                }
                #[mod_api::__subscriptions]
                impl mod_api::Hooks for Commands {
                    fn register_commands(
                        &self,
                        _: &mut dyn mod_api::ServerHost,
                        event: &mut mod_api::RegisterCommandsEvent<'_>,
                    ) {
                        if self.offered.load(Ordering::Relaxed) {
                            let ran = self.ran.clone();
                            event.commands.push(
                                mod_api::Command::new("fnmod [n:integer]").executes(
                                    move |_, _, _| {
                                        ran.fetch_add(1, Ordering::Relaxed);
                                        Ok(1)
                                    },
                                ),
                            );
                        }
                    }
                }
                let offered = Arc::new(AtomicBool::new(true));
                let ran = Arc::new(AtomicU32::new(0));
                let mods = vec![mod_api::ModEntry::new(
                    "fnmod",
                    Box::new(Commands {
                        offered: offered.clone(),
                        ran: ran.clone(),
                    }),
                )];
                mod_platform::testing::with_mods(mods.into(), || {
                    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                        let scratch = tempfile::tempdir().unwrap();
                        let folder = "functions-name-mod-commands";
                        let world = scratch.path().join(folder);
                        let pack = world.join("datapacks/fnmod");
                        let functions = pack.join("data/fnmod/function");
                        std::fs::create_dir_all(&functions).unwrap();
                        let f = sunrise_core::version::DATA_PACK_FORMAT;
                        std::fs::write(
                            pack.join("pack.mcmeta"),
                            serde_json::json!({"pack":{
                                "description":"Mod commands in functions", "pack_format":f.0,
                                "min_format":[f.0,f.1], "max_format":[f.0,f.1]
                            }})
                            .to_string(),
                        )
                        .unwrap();
                        for (name, line) in [
                            ("plain", "fnmod 1"),
                            ("macro", "$fnmod $(n)"),
                            ("unknown", "nosuchmodcommand"),
                        ] {
                            std::fs::write(functions.join(format!("{name}.mcfunction")), line)
                                .unwrap();
                        }
                        let mut request = sunrise_gameplay::game::world_loading::WorldRequest::new(
                            folder,
                            None,
                            Some(3042),
                            true,
                            None,
                            game.level.rule_registry.clone(),
                        )
                        .unwrap();
                        request.save_root = world;
                        game.load_world(request).unwrap();
                        let store = &game.server.fns.0.store;
                        assert!(
                            store.functions.contains_key("fnmod:plain"),
                            "{:?}",
                            store.errors
                        );
                        assert!(
                            store.functions.contains_key("fnmod:macro"),
                            "{:?}",
                            store.errors
                        );
                        assert!(!store.functions.contains_key("fnmod:unknown"));
                        let refusal = |game: &sunrise_gameplay::game::ServerGame<'_>, id: &str| {
                            game.server
                                .fns
                                .0
                                .store
                                .errors
                                .iter()
                                .find(|(f, _)| f == id)
                                .map(|(_, e)| e.clone())
                        };
                        assert!(
                            refusal(game, "fnmod:unknown")
                                .unwrap()
                                .starts_with("Invalid command \"nosuchmodcommand\": ")
                        );
                        game.ui.registries.permission = 4;
                        assert!(game.run_command("function fnmod:plain"));
                        assert!(game.run_command("function fnmod:macro {n:3}"));
                        assert_eq!(ran.load(Ordering::Relaxed), 2);
                        offered.store(false, Ordering::Relaxed);
                        let features = game.level.enabled_features.clone();
                        game.reload_datapacks(false, &features).unwrap();
                        assert!(
                            !game
                                .server
                                .fns
                                .0
                                .store
                                .functions
                                .contains_key("fnmod:plain")
                        );
                        assert!(
                            refusal(game, "fnmod:plain")
                                .unwrap()
                                .starts_with("Invalid command \"fnmod 1\": ")
                        );
                    });
                });
            },
        );
    }
    #[test]
    #[ignore = "daily"]
    fn refused_item_key_reload_keeps_previous_world_generation() {
        use mod_api::{Hooks, ModEntry, Registrar, RegistrationContext};

        struct MissingItem;
        #[mod_api::__subscriptions]
        impl Hooks for MissingItem {
            fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
                registrar.block_key("minecraft:stone");
                registrar.item_key("reload_test:missing");
            }
        }

        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let root =
                std::env::temp_dir().join(format!("reload-atomicity-{}", std::process::id()));
            let pack = root.join("datapacks/candidate");
            let tags = pack.join("data/reload_test/tags/block");
            std::fs::create_dir_all(&tags).unwrap();
            std::fs::write(pack.join("pack.mcmeta"), serde_json::to_vec(&serde_json::json!({
                "pack": {"description": "atomic reload", "min_format": sunrise_core::version::DATA_PACK_FORMAT, "max_format": sunrise_core::version::DATA_PACK_FORMAT}
            })).unwrap()).unwrap();
            std::fs::write(
                tags.join("candidate.json"),
                br#"{"values":["minecraft:stone"]}"#,
            )
            .unwrap();
            let modifiers = pack.join("data/reload_test/loot_modifiers");
            std::fs::create_dir_all(&modifiers).unwrap();
            let modifier_file = modifiers.join("append.json");
            std::fs::write(
                &modifier_file,
                br#"{"type":"neoforge:add_table","table":"minecraft:blocks/stone"}"#,
            )
            .unwrap();
            let modifier_id = mod_api::Identifier::parse("reload_test:append").unwrap();
            game.server.world_root = root.clone();
            game.set_allow_commands(true);
            // Establish the live pack generation before injecting a refused candidate.
            // Mod declarations are load-bound; replacing them cannot be used to test
            // a subsequent successful reload of the same live worker registry.
            game.reload_datapacks(true, &[]).unwrap();
            assert!(game.items().block_tags["reload_test:candidate"].contains("minecraft:stone"));
            let first = game
                .reload_resources()
                .listener(&mod_api::GlobalLootModifierManager::key())
                .unwrap();
            let first_instance = first.get(&modifier_id).unwrap();
            assert_eq!(first.id_of(&first_instance), Some(&modifier_id));
            assert_eq!(first.modifiers().len(), 1);
            let first_generation = Arc::downgrade(
                game.server
                    .world_tables
                    .0
                    .as_ref()
                    .unwrap()
                    .loot
                    .global_modifiers(),
            );
            assert!(game.run_command("datapack disable \"file/candidate\""));
            assert!(
                game.reload_resources()
                    .listener(&mod_api::GlobalLootModifierManager::key())
                    .unwrap()
                    .modifiers()
                    .is_empty()
            );
            // A retained manager keeps the preceding immutable instance generation.
            assert_eq!(first.modifiers().len(), 1);
            assert!(first_generation.upgrade().is_some());
            drop(first);
            assert!(first_generation.upgrade().is_none());
            std::fs::write(&modifier_file, br#"{"type":"neoforge:add_table","table":"minecraft:blocks/stone","priority":1002}"#).unwrap();
            assert!(game.run_command("datapack enable \"file/candidate\""));
            let live_modifiers = game
                .reload_resources()
                .listener(&mod_api::GlobalLootModifierManager::key())
                .unwrap();
            assert_eq!(
                live_modifiers
                    .get(&modifier_id)
                    .unwrap()
                    .priority()
                    .unwrap(),
                1002
            );
            assert_eq!(first_instance.priority().unwrap(), 1000);
            let candidate = mod_api::__private::tag_key(
                &mod_api::Identifier::parse("reload_test:candidate").unwrap(),
            )
            .unwrap()
            .typed::<mod_api::BlockKey>();
            let live_stone = mod_platform::__private::part_tests::block_key("minecraft:stone");
            assert_eq!(
                sunrise_gameplay_common::game::native_tags::block_in_tag(
                    Some(game.items()),
                    live_stone,
                    candidate
                ),
                Some(true)
            );
            sunrise_gameplay_common::game::api_values::install_parts(&[ModEntry::new(
                "reload_test",
                Box::new(MissingItem),
            )])
            .unwrap();
            // Resolve candidate metadata without rebinding the live worker registry.
            let (registry, _) =
                sunrise_world::level::data::load_registry_with_flags(&game.options.resources)
                    .unwrap();
            sunrise_gameplay_common::game::api_values::resolve_block_definitions(&registry)
                .unwrap();
            let stone = mod_platform::__private::part_tests::block_key("minecraft:stone");
            let old_tag = sunrise_gameplay_common::game::native_tags::block_in_tag(
                Some(game.items()),
                stone,
                candidate,
            );
            let old_context = game.loop_context.clone();
            let old_root = game.options.resources.clone();
            let old_data = game.pack.data_source.clone();
            let old_tables = game
                .server
                .world_tables
                .0
                .as_ref()
                .map(|tables| tables as *const _);
            let old_generation = game.damage_tag_generation;
            let old_enabled = game.server.fns.0.enabled.clone();
            std::fs::write(&modifier_file, br#"{"type":"neoforge:add_table","table":"minecraft:blocks/stone","priority":1003}"#).unwrap();
            let error = game.reload_datapacks(true, &[]).err().unwrap();
            assert!(error.to_string().contains("reload_test:missing"));
            let retained = game
                .reload_resources()
                .listener(&mod_api::GlobalLootModifierManager::key())
                .unwrap();
            assert!(Arc::ptr_eq(&retained, &live_modifiers));
            assert_eq!(
                retained.get(&modifier_id).unwrap().priority().unwrap(),
                1002
            );
            assert_eq!(game.options.resources, old_root);
            assert!(game.pack.data_source.same_generation(&old_data));
            assert!(Arc::ptr_eq(&game.loop_context, &old_context));
            assert_eq!(
                old_tables,
                game.server
                    .world_tables
                    .0
                    .as_ref()
                    .map(|tables| tables as *const _)
            );
            assert_eq!(game.damage_tag_generation, old_generation);
            assert_eq!(game.server.fns.0.enabled, old_enabled);
            assert_eq!(
                sunrise_gameplay_common::game::native_tags::block_in_tag(
                    Some(game.items()),
                    stone,
                    candidate
                ),
                old_tag
            );
            game.save_datapack_settings();
            let settings = std::fs::read(root.join("datapacks.json")).unwrap();
            let enabled = game.server.fns.0.enabled.clone();
            let context = game.loop_context.clone();
            let generation = game.damage_tag_generation;
            assert!(!game.run_command("datapack disable \"file/candidate\""));
            assert_eq!(game.server.fns.0.enabled, enabled);
            assert_eq!(
                std::fs::read(root.join("datapacks.json")).unwrap(),
                settings
            );
            assert!(Arc::ptr_eq(&game.loop_context, &context));
            assert_eq!(game.damage_tag_generation, generation);
            assert!(game.items().block_tags["reload_test:candidate"].contains("minecraft:stone"));
            let _ = std::fs::remove_dir_all(root);
        });
    }
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn pack_changes_publish_the_new_zip_generation_for_template_reads() {
        use std::{io::Write, path::PathBuf};
        use {
            sunrise_core::{nbt, nbt::Limits},
            sunrise_world::compression::Compression,
        };
        let root = std::env::temp_dir().join(format!(
            "template-pack-change-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("datapacks")).unwrap();
        struct Scratch(PathBuf);
        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let scratch = Scratch(root);
        let archive = scratch.0.join("datapacks/templates.zip");
        let publish = |marker: &str| {
            let named = nbt::NamedTag {
                name: "".into(),
                tag: Tag::compound([("marker", Tag::String(marker.into()))]),
            };
            let bytes = Compression::Gzip
                .encode(&nbt::write_named(&named, Limits::templates()).unwrap())
                .unwrap();
            let next = archive.with_extension("next");
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&next).unwrap());
            zip.start_file("pack.mcmeta", zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"{}").unwrap();
            zip.start_file(
                "data/generation_test/structure/probe.nbt",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(&bytes).unwrap();
            zip.finish().unwrap();
            std::fs::rename(next, &archive).unwrap();
        };
        publish("old");
        let mut ran = false;
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            ran = true;
            game.server.world_root = scratch.0.clone();
            game.set_allow_commands(true);
            game.reload_datapacks(false, &[]).unwrap();
            let marker = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                game.read_template_tag("generation_test:probe")
                    .and_then(|tag| tag.get("marker").cloned())
            };
            assert_eq!(marker(game), Some(Tag::String("old".into())));
            publish("new");
            assert!(game.run_command("datapack disable \"file/templates.zip\""));
            assert_eq!(marker(game), None);
            assert!(game.run_command("datapack enable \"file/templates.zip\""));
            assert_eq!(marker(game), Some(Tag::String("new".into())));
            assert!(
                !scratch
                    .0
                    .join("data/generation_test/structure/probe.nbt")
                    .exists()
            );
        });
        assert!(ran, "the prepared-resource fixture must run this case");
    }
    #[test]
    #[ignore = "daily"]
    fn function_pack_rejects_invalid_ordinary_lines_before_execution() {
        use std::time::Instant;
        use {
            crate::game::load_resources, crate::settings::Settings,
            sunrise_gameplay_common::game::Options,
        };
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &sunrise_gameplay_common::game::Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        let dir =
            std::env::temp_dir().join(format!("sunrise-function-admission-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pack = dir.join("datapacks/admission");
        let functions = pack.join("data/h144/function");
        let predicates = pack.join("data/h144/predicate");
        std::fs::create_dir_all(&functions).unwrap();
        std::fs::create_dir_all(&predicates).unwrap();
        std::fs::write(pack.join("pack.mcmeta"), r#"{"pack":{"description":"Function admission","pack_format":121,"min_format":[121,0],"max_format":[121,0]}}"#).unwrap();
        std::fs::write(
            predicates.join("ready.json"),
            r#"{"type":"minecraft:all_of","terms":[]}"#,
        )
        .unwrap();
        for (name, text) in [
            (
                "good",
                "execute if predicate h144:ready run scoreboard players add good h144 1\n",
            ),
            (
                "bad",
                "scoreboard players set bad h144 1\nh144_unknown_command\nscoreboard players set bad h144 2\n",
            ),
            (
                "forbidden",
                "scoreboard players set forbidden h144 1\ntick freeze\nscoreboard players set forbidden h144 2\n",
            ),
            (
                "macro",
                "$h144_unknown_$(name)\nscoreboard players set macro h144 1\n",
            ),
            ("macro_bad", "$say $(name)\nh144_unknown_command\n"),
            (
                "sequence",
                "scoreboard players \\\nset sequence h144 17\nreturn 5\n",
            ),
            ("slash", "/say invalid\n"),
        ] {
            std::fs::write(functions.join(format!("{name}.mcfunction")), text).unwrap();
        }
        game.server.world_root = dir.clone();
        let run = |game: &mut sunrise_gameplay::game::ServerGame<'_>, line: &str| {
            game.server.exec.0 = ExecState::default();
            let source = game.player_source();
            game.execute_line(line, source)
        };
        run(&mut game, "scoreboard objectives add h144 dummy").unwrap();
        for name in ["good", "bad", "forbidden", "macro", "sequence"] {
            run(
                &mut game,
                &format!("scoreboard players set {name} h144 -99"),
            )
            .unwrap();
        }
        let clock = game.world.game_time;
        // This is the actual pack loader. Admission must not run any line.
        game.reload_datapacks(false, &game.level.enabled_features.clone())
            .unwrap();
        for name in ["good", "bad", "forbidden", "macro", "sequence"] {
            assert_eq!(game.score.board.score(name, "h144").unwrap().value, -99);
        }
        // Intended old failure: the invalid file previously ran both valid lines.
        assert!(run(&mut game, "function h144:bad").is_err());
        assert_eq!(game.score.board.score("bad", "h144").unwrap().value, -99);
        for name in ["bad", "forbidden", "macro_bad", "slash"] {
            assert!(
                !game
                    .server
                    .fns
                    .0
                    .store
                    .functions
                    .contains_key(&format!("h144:{name}"))
            );
            assert!(
                !game
                    .ui
                    .registries
                    .functions
                    .contains(&format!("h144:{name}"))
            );
            assert!(
                game.server
                    .fns
                    .0
                    .store
                    .errors
                    .iter()
                    .any(|(id, _)| id == &format!("h144:{name}"))
            );
        }
        assert!(run(&mut game, "function h144:forbidden").is_err());
        assert!(game.server.fns.0.store.functions.contains_key("h144:macro"));
        assert!(game.server.fns.0.store.functions["h144:macro"].is_macro());
        run(&mut game, "function h144:good").unwrap();
        assert_eq!(game.score.board.score("good", "h144").unwrap().value, -98);
        run(&mut game, "function h144:sequence").unwrap();
        assert_eq!(
            game.score.board.score("sequence", "h144").unwrap().value,
            17
        );
        assert_eq!(game.ui.registries.permission, 4);
        // A fresh same-pack predicate must be in the compilation snapshot on reload.
        std::fs::write(
            predicates.join("reloaded.json"),
            r#"{"type":"minecraft:all_of","terms":[]}"#,
        )
        .unwrap();
        std::fs::write(
            functions.join("good.mcfunction"),
            "execute if predicate h144:reloaded run scoreboard players add good h144 1\n",
        )
        .unwrap();
        game.reload_datapacks(true, &game.level.enabled_features.clone())
            .unwrap();
        assert!(game.server.fns.0.store.functions.contains_key("h144:good"));
        assert!(
            !game
                .server
                .fns
                .0
                .store
                .functions
                .contains_key("h144:forbidden")
        );
        assert!(
            !game
                .ui
                .registries
                .functions
                .contains(&"h144:forbidden".into())
        );
        assert_eq!(game.score.board.score("good", "h144").unwrap().value, -98);
        run(&mut game, "function h144:good").unwrap();
        assert_eq!(game.score.board.score("good", "h144").unwrap().value, -97);
        assert_eq!(
            game.score.board.score("forbidden", "h144").unwrap().value,
            -99
        );
        assert_eq!(game.world.game_time, clock);
        std::fs::remove_dir_all(dir).unwrap();
    }
    /// `StringTemplate.fromString` fails a `$` line with no variable, so the
    /// file is not registered, and `MacroFunction.MacroEntry.instantiate`
    /// parses a substituted line, throwing `commands.function.error.parse`
    /// before any of the call's lines run.
    #[test]
    #[ignore = "daily"]
    fn macro_lines_need_a_variable_and_parse_after_substitution() {
        use std::time::Instant;
        use {
            crate::game::load_resources, crate::settings::Settings,
            sunrise_gameplay_common::game::Options,
        };
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &sunrise_gameplay_common::game::Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        let dir = std::env::temp_dir().join(format!("sunrise-macro-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let functions = dir.join("datapacks/macros/data/h166/function");
        std::fs::create_dir_all(&functions).unwrap();
        let format = sunrise_core::version::DATA_PACK_FORMAT;
        std::fs::write(
            dir.join("datapacks/macros/pack.mcmeta"),
            serde_json::json!({"pack":{"description":"Macro lines","pack_format":format.0,"min_format":[format.0,format.1],"max_format":[format.0,format.1]}}).to_string(),
        )
        .unwrap();
        for (name, text) in [
            // No `$(…)`: not a macro at all, so the file does not load.
            ("novar", "$say hello\nscoreboard players add #ran h166 1\n"),
            // Substitutes to a line with a trailing argument; the plain lines
            // around it show that none of the call's lines ran.
            (
                "bad",
                "scoreboard players add #ran h166 1\n$scoreboard players set #seen h166 $(v) bogus\nscoreboard players add #ran h166 1\n",
            ),
            (
                "good",
                "$scoreboard players set #seen h166 $(v)\nscoreboard players add #ran h166 1\n",
            ),
        ] {
            std::fs::write(functions.join(format!("{name}.mcfunction")), text).unwrap();
        }
        game.server.world_root = dir.clone();
        let run = |game: &mut sunrise_gameplay::game::ServerGame<'_>, line: &str| {
            game.server.exec.0 = ExecState::default();
            let source = game.player_source();
            game.execute_line(line, source)
        };
        run(&mut game, "scoreboard objectives add h166 dummy").unwrap();
        run(&mut game, "scoreboard players set #ran h166 -99").unwrap();
        game.reload_datapacks(false, &game.level.enabled_features.clone())
            .unwrap();
        assert!(!game.server.fns.0.store.functions.contains_key("h166:novar"));
        assert!(
            game.server
                .fns
                .0
                .store
                .errors
                .iter()
                .any(|(id, e)| id == "h166:novar" && e.contains("No variables in macro"))
        );
        let error = run(&mut game, "function h166:novar").unwrap_err();
        assert_eq!(error.key, "arguments.function.unknown");
        let error = game.check_predicate("h166missing").unwrap_err();
        assert_eq!(error.key, "argument.resource_or_id.no_such_element");
        let text = error.message.plain();
        assert!(text.contains("minecraft:h166missing"), "{text}");
        assert!(text.contains("minecraft:predicate"), "{text}");
        // A substituted line that does not parse fails the call, so no line of
        // it runs, and the reason names the line `MacroEntry.instantiate` got.
        let error = run(&mut game, "function h166:bad {v:1}").unwrap_err();
        assert_eq!(error.key, "commands.function.instantiationFailure");
        let reason = error.message.plain();
        assert!(
            reason.contains("scoreboard players set #seen h166 1 bogus"),
            "{reason}"
        );
        assert_eq!(game.score.board.score("#ran", "h166").unwrap().value, -99);
        assert!(game.score.board.score("#seen", "h166").is_none());
        // A valid substitution still runs every line.
        run(&mut game, "function h166:good {v:7}").unwrap();
        assert_eq!(game.score.board.score("#ran", "h166").unwrap().value, -98);
        assert_eq!(game.score.board.score("#seen", "h166").unwrap().value, 7);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    #[ignore = "daily"]
    fn unless_function_keeps_a_fallthrough_after_running_it_once() {
        use std::time::Instant;
        use sunrise_level::datapack::FunctionLine;
        use {
            crate::game::load_resources, crate::settings::Settings,
            sunrise_gameplay_common::game::Options,
        };
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &sunrise_gameplay_common::game::Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        game.server.fns.0.loaded = true;
        let run = |game: &mut sunrise_gameplay::game::ServerGame<'_>, line: &str| {
            game.server.exec.0 = ExecState::default();
            let source = game.player_source();
            game.execute_line(line, source).unwrap();
        };
        run(&mut game, "scoreboard objectives add h154 dummy");
        let clock = game.world.game_time;
        for (name, tail) in [
            ("counter", None),
            ("zero", Some("return 0")),
            ("five", Some("return 5")),
            ("fail", Some("return fail")),
        ] {
            let id = format!("h154:{name}");
            let mut lines = vec![FunctionLine::Command(
                "scoreboard players add #calls h154 1".into(),
            )];
            if let Some(tail) = tail {
                lines.push(FunctionLine::Command(tail.into()));
            }
            game.server.fns.0.store.functions.insert(
                id.clone(),
                Function {
                    id,
                    lines,
                    parameters: Vec::new(),
                },
            );
        }
        // Explicit returns establish both signs before the old fallthrough
        // failure. The commands run through the actual function/execute route.
        for name in ["zero", "five", "fail", "counter"] {
            for sign in ["if", "unless"] {
                run(&mut game, "scoreboard players set #calls h154 0");
                run(&mut game, "scoreboard players set #out h154 99");
                run(
                    &mut game,
                    &format!(
                        "execute {sign} function h154:{name} run scoreboard players set #out h154 7"
                    ),
                );
                assert_eq!(game.score.board.score("#calls", "h154").unwrap().value, 1);
                let keep = (name == "five") == (sign == "if");
                assert_eq!(
                    game.score.board.score("#out", "h154").unwrap().value,
                    if keep { 7 } else { 99 },
                    "{sign} {name}"
                );
            }
        }
        assert_eq!(game.world.game_time, clock);
    }
}

#[cfg(test)]
mod entity_score_tests {
    use super::*;
    use sunrise_core::nbt::Tag;
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
        sunrise_gameplay_common::game::commands_common::Target,
    };

    #[test]
    #[ignore = "daily"]
    fn named_scores_follow_the_registered_target_holder_and_mutations() {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.build_registries();

        game.set_allow_commands(true);
        game.ensure_datapacks();
        game.server.fns.0.store.predicates.insert(
            "h270:min2".into(),
            serde_json::json!({
                "type": "minecraft:entity_scores", "entity": "this", "scores": {"pts": {"min": 2}}
            }),
        );
        assert!(game.run_command("scoreboard objectives add pts dummy"));
        let mut actors = Vec::new();
        for (index, score) in [Some(0), Some(2), None].into_iter().enumerate() {
            let uuid = [270, 1, 0, index as i32 + 1];
            let tag = Tag::compound([
                ("id", Tag::String("minecraft:pig".into())),
                ("UUID", Tag::IntArray(uuid.to_vec())),
                (
                    "Pos",
                    Tag::List(vec![
                        Tag::Double((3.5 + index as f64).to_bits()),
                        Tag::Double(200.0_f64.to_bits()),
                        Tag::Double(4.5_f64.to_bits()),
                    ]),
                ),
                ("Health", Tag::Float(10.0_f32.to_bits())),
                ("NoAI", Tag::Byte(1)),
                ("NoGravity", Tag::Byte(1)),
            ]);
            let id = game.load_mob(tag, None).unwrap();
            assert_eq!(game.persist.uuids.get(&id), Some(&uuid));
            let holder = game.holder_name(Target::Mob(id)).unwrap();
            assert_eq!(holder, sunrise_sim::save::uuid_text(uuid));
            if let Some(score) = score {
                assert!(game.run_command(&format!("scoreboard players set {holder} pts {score}")));
            }
            assert_eq!(
                game.score.board.score(&holder, "pts").map(|s| s.value),
                score
            );
            actors.push((id, holder));
        }
        let time = game.world.game_time;
        let matches = |game: &mut sunrise_gameplay::game::ServerGame<'_>, id| {
            let mut source = game.player_source();
            source.entity = Some(Target::Mob(id));
            source.pos = game.target_position(Target::Mob(id)).unwrap();
            game.server.command_source = Some(source);
            let result = game.check_predicate("h270:min2").unwrap();
            game.server.command_source = None;
            result
        };
        assert!(!matches(&mut game, actors[0].0));
        assert!(matches(&mut game, actors[1].0));
        assert!(!matches(&mut game, actors[2].0));
        for (index, score, expected) in [(0, 3, true), (1, 1, false)] {
            let (id, holder) = &actors[index];
            assert!(game.run_command(&format!("scoreboard players set {holder} pts {score}")));
            assert_eq!(
                game.score.board.score(holder, "pts").map(|s| s.value),
                Some(score)
            );
            assert_eq!(matches(&mut game, *id), expected);
        }
        assert!(!matches(&mut game, actors[2].0));
        assert!(game.score.board.score(&actors[2].1, "pts").is_none());
        assert_eq!(game.world.game_time, time);
    }
    #[test]
    #[ignore = "daily"]
    fn folder_metadata_rejection_reaches_actual_function_commands() {
        use std::time::Instant;
        use {
            crate::game::load_resources, crate::settings::Settings,
            sunrise_gameplay_common::game::Options,
        };
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &sunrise_gameplay_common::game::Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        let dir = std::env::temp_dir().join(format!(
            "sunrise-game-folder-metadata-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let cases = [
            ("broken", Some("{"), false),
            ("root_empty", Some("{}"), false),
            ("root_array", Some("[]"), false),
            ("pack_empty", Some(r#"{"pack":{}}"#), false),
            ("pack_string", Some(r#"{"pack":"bad"}"#), false),
            ("missing", None, false),
            (
                "full",
                Some(
                    r#"{"pack":{"description":"full","pack_format":121,"min_format":[121,0],"max_format":[121,0]}}"#,
                ),
                true,
            ),
            (
                "description",
                Some(r#"{"pack":{"description":"description only"}}"#),
                true,
            ),
        ];
        for (name, metadata, _) in cases {
            let pack = dir.join("datapacks").join(name);
            std::fs::create_dir_all(pack.join("data/h146/function")).unwrap();
            std::fs::write(
                pack.join(format!("data/h146/function/{name}.mcfunction")),
                format!("scoreboard players set {name} h146 17\n"),
            )
            .unwrap();
            if let Some(metadata) = metadata {
                std::fs::write(pack.join("pack.mcmeta"), metadata).unwrap();
            }
        }
        game.server.world_root = dir.clone();
        let run = |game: &mut sunrise_gameplay::game::ServerGame<'_>, line: &str| {
            game.server.exec.0 = ExecState::default();
            let source = game.player_source();
            game.execute_line(line, source)
        };
        run(&mut game, "scoreboard objectives add h146 dummy").unwrap();
        let clock = game.world.game_time;
        for reload in [false, true] {
            for (name, _, _) in cases {
                run(
                    &mut game,
                    &format!("scoreboard players set {name} h146 -99"),
                )
                .unwrap();
            }
            game.reload_datapacks(reload, &game.level.enabled_features.clone())
                .unwrap();
            for (name, _, valid) in cases {
                assert_eq!(game.score.board.score(name, "h146").unwrap().value, -99);
                let result = run(&mut game, &format!("function h146:{name}"));
                // Intended old failure: a malformed pack's valid function executes.
                assert_eq!(result.is_ok(), valid, "{name}, reload={reload}");
                assert_eq!(
                    game.score.board.score(name, "h146").unwrap().value,
                    if valid { 17 } else { -99 }
                );
                assert_eq!(
                    game.server
                        .fns
                        .0
                        .store
                        .functions
                        .contains_key(&format!("h146:{name}")),
                    valid
                );
                assert_eq!(
                    game.ui
                        .registries
                        .functions
                        .contains(&format!("h146:{name}")),
                    valid
                );
                assert_eq!(
                    game.server
                        .fns
                        .0
                        .packs
                        .iter()
                        .any(|p| p.id == format!("file/{name}")),
                    valid
                );
            }
        }
        assert_eq!(game.world.game_time, clock);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod required_tag_tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn real_pack_required_tag_failures_execute_no_partial_functions() {
        use std::time::{Instant, SystemTime, UNIX_EPOCH};
        use {
            crate::game::load_resources, crate::settings::Settings,
            sunrise_gameplay_common::game::Options,
        };
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &sunrise_gameplay_common::game::Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        let dir = std::env::temp_dir().join(format!(
            "sunrise-required-tags-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let pack = dir.join("datapacks/h145");
        let write = |path: &str, body: &str| {
            let path = pack.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        };
        let (major, minor) = sunrise_core::version::DATA_PACK_FORMAT;
        write(
            "pack.mcmeta",
            &serde_json::json!({"pack": {
                "description": "Required dependency test", "pack_format": major,
                "min_format": [major, minor], "max_format": [major, minor],
            }})
            .to_string(),
        );
        for (i, name) in ["a", "b", "c"].iter().enumerate() {
            write(
                &format!("data/h145/function/{name}.mcfunction"),
                &format!(
                    "scoreboard players add {name} h145 1\nscoreboard players operation order h145 *= ten h145\nscoreboard players add order h145 {}\n",
                    i + 1,
                ),
            );
        }
        let tags = serde_json::json!({
            "empty": [], "ordered": ["h145:a", "h145:b", "h145:a"],
            "nested": ["#h145:empty", "#h145:ordered", "h145:a"],
            "optional_member": ["h145:a", {"id":"h145:missing","required":false}, "h145:b"],
            "optional_child": ["h145:a", {"id":"#h145:missing","required":false}, "h145:b"],
            "missing_first": ["h145:missing", "h145:a"],
            "missing_last": ["h145:a", "h145:missing"],
            "missing_child": ["h145:a", "#h145:missing"],
            "invalid_child": ["h145:c", "h145:missing"],
            "required_invalid_child": ["h145:a", "#h145:invalid_child", "h145:b"],
            "optional_invalid_child": ["h145:a", {"id":"#h145:invalid_child","required":false}, "h145:b"],
        });
        for (name, values) in tags.as_object().unwrap() {
            write(
                &format!("data/h145/tags/function/{name}.json"),
                &serde_json::json!({"values": values}).to_string(),
            );
        }
        game.server.world_root = dir.clone();
        game.build_registries();
        game.set_allow_commands(true);
        let run = |game: &mut sunrise_gameplay::game::ServerGame<'_>, line: &str| {
            game.server.exec.0 = ExecState::default();
            let source = game.player_source();
            if let Err(error) = game.execute_line(line, source) {
                assert!(line.contains("function #h145:"), "{line}: {error:?}");
                assert_eq!(error.key, "commands.function.scheduled.no_functions");
            }
        };
        run(&mut game, "scoreboard objectives add h145 dummy");
        run(&mut game, "scoreboard players set ten h145 10");
        run(&mut game, "scoreboard players set sentinel h145 73");
        let clock = game.world.game_time;
        run(&mut game, "reload");
        assert_eq!(
            game.score.board.score("sentinel", "h145").unwrap().value,
            73
        );
        assert!(game.server.fns.0.enabled.contains(&"file/h145".to_owned()));
        assert!(
            game.server.fns.0.store.errors.is_empty(),
            "{:?}",
            game.server.fns.0.store.errors
        );
        for name in ["a", "b", "c"] {
            assert!(
                game.server
                    .fns
                    .0
                    .store
                    .functions
                    .contains_key(&format!("h145:{name}"))
            );
        }
        for name in tags.as_object().unwrap().keys() {
            assert!(
                game.server
                    .fns
                    .0
                    .store
                    .function_tags
                    .contains_key(&format!("h145:{name}"))
            );
        }
        let score = |game: &sunrise_gameplay::game::ServerGame<'_>, key: &str| {
            game.score.board.score(key, "h145").unwrap().value
        };
        let reset = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
            for key in ["a", "b", "c", "order"] {
                run(game, &format!("scoreboard players set {key} h145 0"));
                assert_eq!(score(game, key), 0);
            }
        };
        // Independent actual functions and valid tag controls run before the defect assertions.
        for (target, expected) in [
            ("h145:a", [1, 0, 0, 1]),
            ("h145:b", [0, 1, 0, 2]),
            ("h145:c", [0, 0, 1, 3]),
            ("#h145:empty", [0, 0, 0, 0]),
            ("#h145:ordered", [1, 1, 0, 12]),
            ("#h145:nested", [1, 1, 0, 12]),
            ("#h145:optional_member", [1, 1, 0, 12]),
            ("#h145:optional_child", [1, 1, 0, 12]),
        ] {
            reset(&mut game);
            run(&mut game, &format!("function {target}"));
            assert_eq!(
                [
                    score(&game, "a"),
                    score(&game, "b"),
                    score(&game, "c"),
                    score(&game, "order")
                ],
                expected,
                "{target}"
            );
        }
        println!(
            "H145 actual pack metadata, functions, empty/nested/dedup, optional-missing and reload controls admitted"
        );
        let mut failures = Vec::new();
        for name in [
            "missing_first",
            "missing_last",
            "missing_child",
            "invalid_child",
            "required_invalid_child",
            "optional_invalid_child",
        ] {
            reset(&mut game);
            run(&mut game, "scoreboard players set result h145 31");
            run(&mut game, "scoreboard players set success h145 17");
            assert_eq!((score(&game, "result"), score(&game, "success")), (31, 17));
            run(
                &mut game,
                &format!(
                    "execute store result score result h145 store success score success h145 run function #h145:{name}"
                ),
            );
            let actual = [
                score(&game, "a"),
                score(&game, "b"),
                score(&game, "c"),
                score(&game, "order"),
            ];
            let expected = if name == "optional_invalid_child" {
                [1, 1, 0, 12]
            } else {
                [0, 0, 0, 0]
            };
            if actual != expected {
                failures.push(format!("{name}: {actual:?} expected {expected:?}"));
            }
            let ids = game
                .server
                .fns
                .0
                .store
                .tag_functions(&format!("h145:{name}"));
            let expected_ids: Vec<String> = if name == "optional_invalid_child" {
                vec!["h145:a".into(), "h145:b".into()]
            } else {
                Vec::new()
            };
            if ids != expected_ids {
                failures.push(format!(
                    "{name}: DataStore {ids:?} expected {expected_ids:?}"
                ));
            }
        }
        assert_eq!(game.world.game_time, clock);
        drop(game);
        std::fs::remove_dir_all(dir).unwrap();
        assert!(
            failures.is_empty(),
            "required tag partial execution: {failures:#?}"
        );
    }

    #[test]
    #[ignore = "daily"]
    fn named_predicates_resolve_loaded_references_for_actual_sources() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            let dir = std::env::temp_dir().join(format!(
                "sunrise-predicate-reference-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            let pack = dir.join("datapacks/references");
            let folder = pack.join("data/h121/predicate");
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(pack.join("pack.mcmeta"), r#"{"pack":{"description":"Predicate references","pack_format":121,"min_format":[121,0],"max_format":[121,0]}}"#).unwrap();
            let write = |name: &str, value: serde_json::Value| {
                std::fs::write(folder.join(format!("{name}.json")), value.to_string()).unwrap();
            };
            write(
                "pig",
                serde_json::json!({"type":"minecraft:entity_properties","entity":"this","predicate":{"minecraft:entity_type":"minecraft:pig"}}),
            );
            write(
                "cow",
                serde_json::json!({"type":"minecraft:entity_properties","entity":"this","predicate":{"minecraft:entity_type":"minecraft:cow"}}),
            );
            write(
                "reference",
                serde_json::json!({"type":"minecraft:all_of","terms":["h121:pig"]}),
            );
            write(
                "inverted",
                serde_json::json!({"type":"minecraft:inverted","term":"h121:pig"}),
            );
            write(
                "conjunction",
                serde_json::json!({"type":"minecraft:all_of","terms":["h121:pig",{"type":"minecraft:inverted","term":"h121:cow"}]}),
            );
            write(
                "both",
                serde_json::json!({"type":"minecraft:all_of","terms":["h121:pig","h121:cow"]}),
            );
            write(
                "either",
                serde_json::json!({"type":"minecraft:any_of","terms":["h121:pig","h121:cow"]}),
            );
            write(
                "repeated",
                serde_json::json!({"type":"minecraft:all_of","terms":["h121:pig","h121:pig"]}),
            );
            for index in 0..33 {
                let next = if index == 32 {
                    "h121:pig".to_owned()
                } else {
                    format!("h121:chain_{}", index + 1)
                };
                write(
                    &format!("chain_{index}"),
                    serde_json::json!({"type":"minecraft:all_of","terms":[next]}),
                );
            }
            game.server.world_root = dir.clone();
            game.reload_datapacks(false, &[]).unwrap();
            assert!(
                game.server
                    .fns
                    .0
                    .store
                    .predicates
                    .contains_key("h121:reference")
            );
            assert!(
                game.ui
                    .registries
                    .predicates
                    .contains(&"h121:reference".into())
            );
            let run = |game: &mut sunrise_gameplay::game::ServerGame<'_>, line: &str| {
                game.server.exec.0 = ExecState::default();
                let source = game.player_source();
                game.execute_line(line, source).unwrap();
            };
            run(game, "scoreboard objectives add h121 dummy");
            let mut actors = Vec::new();
            for (index, kind) in ["pig", "cow"].into_iter().enumerate() {
                let uuid = [121, 20, 30, index as i32 + 1];
                let tag = Tag::compound([
                    (
                        "id",
                        Tag::String(format!("minecraft:{kind}").as_str().into()),
                    ),
                    ("UUID", Tag::IntArray(uuid.to_vec())),
                    (
                        "Pos",
                        Tag::List(vec![
                            Tag::Double((4.5 + index as f64 * 4.0).to_bits()),
                            Tag::Double(200.0_f64.to_bits()),
                            Tag::Double(4.5_f64.to_bits()),
                        ]),
                    ),
                    ("Health", Tag::Float(10.0_f32.to_bits())),
                    ("NoAI", Tag::Byte(1)),
                    ("NoGravity", Tag::Byte(1)),
                ]);
                let id = game.load_mob(tag, None).unwrap();
                assert_eq!(game.persist.uuids.get(&id), Some(&uuid));
                assert_eq!(
                    game.entity_view_of(Target::Mob(id)).kind,
                    format!("minecraft:{kind}")
                );
                actors.push((id, sunrise_sim::save::uuid_text(uuid), kind));
            }
            let clock = game.world.game_time;
            for (_, uuid, kind) in &actors {
                let pig = *kind == "pig";
                for (name, expected) in [
                    ("pig", pig),
                    ("cow", !pig),
                    ("reference", pig),
                    ("inverted", !pig),
                    ("conjunction", pig),
                    ("both", false),
                    ("either", true),
                    ("repeated", pig),
                    ("chain_0", pig),
                ] {
                    for sign in ["if", "unless"] {
                        run(game, "scoreboard players set branch h121 -99");
                        run(
                            game,
                            &format!(
                                "execute as {uuid} {sign} predicate h121:{name} run scoreboard players set branch h121 1"
                            ),
                        );
                        // The intended old failure is the real loaded reference on the pig.
                        assert_eq!(
                            game.score.board.score("branch", "h121").unwrap().value,
                            if expected == (sign == "if") { 1 } else { -99 },
                            "{kind} {sign} {name}"
                        );
                    }
                }
            }
            let previous = game.server.command_source.clone();
            let server = game.server_source();
            assert!(server.entity.is_none());
            game.server.command_source = Some(server);
            assert!(!game.check_predicate("h121:pig").unwrap());
            assert!(!game.check_predicate("h121:reference").unwrap());
            assert!(game.check_predicate("h121:inverted").unwrap());
            assert!(game.check_predicate("h121:absent").is_err());
            game.server.command_source = previous.clone();
            // Defensive evaluator controls only. Java rejects these graphs at
            // world load (a reload refuses a dangling or cyclic reference, so they cannot come from a
            // pack): they are put in the loaded store directly. This does not establish
            // startup/availability parity.
            let mut defensive_graphs = Vec::new();
            let mut defensive = |name: &str, value: serde_json::Value| {
                defensive_graphs.push((name.to_owned(), value));
            };
            defensive(
                "h121:missing",
                serde_json::json!({"type":"minecraft:all_of","terms":["h121:absent"]}),
            );
            defensive(
                "h121:nested_missing",
                serde_json::json!({"type":"minecraft:all_of","terms":["h121:missing"]}),
            );
            defensive(
                "h121:not_missing",
                serde_json::json!({"type":"minecraft:inverted","term":"h121:absent"}),
            );
            defensive(
                "h121:cycle_a",
                serde_json::json!({"type":"minecraft:all_of","terms":["h121:cycle_b"]}),
            );
            defensive(
                "h121:cycle_b",
                serde_json::json!({"type":"minecraft:all_of","terms":["h121:cycle_a"]}),
            );
            defensive(
                "h121:not_cycle",
                serde_json::json!({"type":"minecraft:inverted","term":"h121:cycle_a"}),
            );
            for invalid in ["h121:absent", "h121:cycle_a"] {
                for (label, kind, first) in [
                    ("any_skip", "minecraft:any_of", "h121:pig"),
                    ("any_reach", "minecraft:any_of", "h121:cow"),
                    ("all_skip", "minecraft:all_of", "h121:cow"),
                    ("all_reach", "minecraft:all_of", "h121:pig"),
                ] {
                    defensive(
                        &format!("h121:{label}_{}", invalid.split(':').next_back().unwrap()),
                        serde_json::json!({"type":kind,"terms":[first,invalid]}),
                    );
                }
            }
            // A predicate naming itself by its short id: the same cycle, under its canonical name.
            defensive(
                "minecraft:h121_canonical",
                serde_json::json!({"type":"minecraft:all_of","terms":["h121_canonical"]}),
            );
            game.server.fns.0.store.predicates.extend(defensive_graphs);
            let mut source = game.player_source();
            source.entity = Some(Target::Mob(actors[0].0));
            source.pos = game.target_position(Target::Mob(actors[0].0)).unwrap();
            game.server.command_source = Some(source);
            for name in [
                "missing",
                "nested_missing",
                "not_missing",
                "cycle_a",
                "not_cycle",
                "any_skip_absent",
                "any_reach_absent",
                "all_skip_absent",
                "all_reach_absent",
                "any_skip_cycle_a",
                "any_reach_cycle_a",
                "all_skip_cycle_a",
                "all_reach_cycle_a",
            ] {
                assert!(
                    game.server
                        .fns
                        .0
                        .store
                        .predicates
                        .contains_key(&format!("h121:{name}"))
                );
                assert!(
                    !game.check_predicate(&format!("h121:{name}")).unwrap(),
                    "{name}"
                );
            }
            assert!(!game.check_predicate("minecraft:h121_canonical").unwrap());
            assert!(game.check_predicate("h121:reference").unwrap());
            game.server.command_source = previous;
            assert_eq!(game.world.game_time, clock);
            std::fs::remove_dir_all(dir).unwrap();
        });
    }
}

#[cfg(test)]
#[path = "functions/schedule_reload_tests.rs"]
mod schedule_reload_tests;
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

#[cfg(test)]
include!("functions/typed_state_range_tests.rs");
