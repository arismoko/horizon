use mod_api::{Hooks, ModEntry, ServerHost};
use std::sync::Arc;
use std::sync::Mutex;
use std::{fs, path::Path};
use sunrise_game_data::lang::{self, Language};
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_gameplay::game::world_loading::WorldRequest;

fn string(host: &dyn ServerHost, key: &str) -> String {
    host.text_string(&mod_api::Text::translate(key, vec![]))
}

struct Observe(Arc<Mutex<Vec<&'static str>>>);

#[mod_api::__subscriptions]
impl Hooks for Observe {
    fn server_about_to_start(
        &self,
        host: &mut dyn ServerHost,
        _: &mut mod_api::ServerAboutToStartEvent,
    ) {
        assert_eq!(
            string(host, "sunrise.modprofile.started"),
            "Mod profiling started."
        );
        assert_eq!(string(host, "language_test.value"), "language_test.value");
        self.0.lock().unwrap().push("about");
    }

    fn data_pack_files(&self, host: &mut dyn ServerHost, _: &mut mod_api::DataPackFilesEvent<'_>) {
        let mut seen = self.0.lock().unwrap();
        if !seen.contains(&"starting") {
            assert_eq!(string(host, "language_test.value"), "language_test.value");
            seen.push("data");
        } else {
            assert_eq!(string(host, "language_test.value"), "key.jump");
        }
    }

    fn server_starting(&self, host: &mut dyn ServerHost, _: &mut mod_api::ServerStartingEvent) {
        assert_eq!(string(host, "language_test.value"), "key.jump");
        assert_eq!(string(host, "language_test.number"), "1.50");
        assert_eq!(string(host, "language_test.preserved"), "lower value");
        assert_eq!(string(host, "language_test.unread"), "language_test.unread");
        assert_eq!(string(host, "language_test.next"), "language_test.next");
        assert_eq!(string(host, "language_test.off"), "language_test.off");
        assert_eq!(string(host, "language_test.hover"), "lower hover value");
        self.0.lock().unwrap().push("starting");
    }
}

fn write(path: &Path, bytes: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

#[test]
#[ignore = "daily"]
fn dedicated_language_uses_the_opening_selection_once() {
    let _language = lang::isolate_publication(Language::of("en_us", []));
    let _server_data = sunrise_sim::datapack_net::isolate_server_publication();
    let scratch = tempfile::tempdir().unwrap();
    let world = scratch.path();
    let selection = world.join("datapacks.json");
    let (major, minor) = sunrise_core::version::DATA_PACK_FORMAT;
    let meta = format!(
        r#"{{"pack":{{"description":"Language test","min_format":[{major},{minor}],"max_format":[{major},{minor}]}}}}"#
    );
    for (name, contents) in [
        (
            "low",
            r#"{"language_test.value":"lower priority","language_test.preserved":"lower value"}"#,
        ),
        (
            "high",
            r#"{
            # An explicitly typed component uses its declared kind, not `text`.
            'language_test.value': {'type':'keybind','keybind':'key.jump','text':'ignored'},
            'language_test.number': 1.50,
            'language_test.preserved': {'type':'text','text':false},
            'language_test.unread': 'must not be loaded'
        }"#,
        ),
        (
            "later",
            r#"{"language_test.value":"skipped after high","language_test.next":"skipped pack"}"#,
        ),
        (
            "off",
            r#"{"language_test.value":"changed at reload","language_test.off":"disabled initially"}"#,
        ),
    ] {
        let pack = world.join("datapacks").join(name);
        write(&pack.join("pack.mcmeta"), &meta);
        write(&pack.join("assets/language_test/lang/en_us.json"), contents);
        let hover = match name {
            "low" => r#"{"language_test.hover":"lower hover value"}"#,
            "high" => {
                r#"{"language_test.hover":{"text":"bad override","hover_event":{"action":"show_item","id":"not an item id"}}}"#
            }
            _ => continue,
        };
        write(&pack.join("assets/tooltip_ns/lang/en_us.json"), hover);
    }
    write(
        &selection,
        r#"{"enabled":["vanilla","file/low","file/high","file/later"],"disabled":["file/off"]}"#,
    );
    let seen = Arc::new(Mutex::new(Vec::new()));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "language_test",
            Box::new(Observe(seen.clone())),
        )]
        .into(),
        || {
            with_game(|game| {
                lang::set(Language::load_default(&game.ui.vanilla_root));
                let mut request = WorldRequest::new(
                    "language_test",
                    Some("language_test"),
                    Some(1),
                    true,
                    None,
                    Arc::new(sunrise_game_data::gamerules::RuleRegistry::vanilla()),
                )
                .unwrap();
                request.save_root = world.to_owned();
                let prepared = game.prepare_world_and_rules(request).unwrap();
                game.native_server_about_to_start(true);
                let opening_packs = game.finish_open_world(prepared).unwrap();
                assert!(
                    game.server
                        .fns
                        .0
                        .enabled()
                        .iter()
                        .any(|id| id == "file/high")
                );
                assert!(
                    !game
                        .server
                        .fns
                        .0
                        .enabled()
                        .iter()
                        .any(|id| id == "file/off")
                );
                // Publication must use what the world adopted, not a second discovery.
                write(
                    &selection,
                    r#"{"enabled":["vanilla","file/off"],"disabled":["file/low","file/high","file/later"]}"#,
                );
                game.start_loaded_dedicated_world(&opening_packs);
                game.reload_datapacks(true, &game.level.enabled_features.clone())
                    .unwrap();
                assert!(
                    game.server
                        .fns
                        .0
                        .enabled()
                        .iter()
                        .any(|id| id == "file/off")
                );
                assert_eq!(string(game, "language_test.value"), "key.jump");
                assert_eq!(string(game, "language_test.off"), "language_test.off");
            })
        },
    );
    let seen = seen.lock().unwrap();
    assert_eq!(seen.first(), Some(&"about"));
    assert_eq!(seen.last(), Some(&"starting"));
    assert!(seen[1..seen.len() - 1].contains(&"data"));
}
