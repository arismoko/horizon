use super::*;
use crate::game::{native_entity_refs::tests::with_game, world_loading::WorldRequest};
use horizon_client::mod_api::{
    self, AddServerReloadListenersEvent, Hooks, Host, ModEntry, ServerReloadListener,
};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

struct DataListener {
    live: Arc<Mutex<u64>>,
    reject: Arc<AtomicBool>,
}
impl ServerReloadListener for DataListener {
    fn prepare(
        &self,
        context: &ServerReloadContext,
    ) -> Result<ServerReloadApply, ServerReloadError> {
        let id = Identifier::parse("reload_test:value").unwrap();
        assert!(context.list().any(|entry| entry == &id));
        // Arbitrary IDs only perform map lookups, never filesystem reads.
        assert!(
            context
                .read(&Identifier::parse("reload_test:../private").unwrap())
                .is_none()
        );
        let mut value = 0;
        for id in context.list() {
            value += serde_json::from_slice::<u64>(context.read(id).unwrap())
                .map_err(|error| ServerReloadError::new(id.clone(), error))?;
        }
        let live = self.live.clone();
        Ok(ServerReloadApply::new(move || {
            *live.lock().unwrap() = value
        }))
    }
}
struct CheckListener(Arc<AtomicBool>);
impl ServerReloadListener for CheckListener {
    fn prepare(&self, _: &ServerReloadContext) -> Result<ServerReloadApply, ServerReloadError> {
        if self.0.load(Ordering::Relaxed) {
            return Err(ServerReloadError::new(
                Identifier::parse("reload_test:value").unwrap(),
                std::io::Error::other("rejected new machine data"),
            ));
        }
        Ok(ServerReloadApply::new(|| {}))
    }
}
#[mod_api::__subscriptions]
impl Hooks for DataListener {
    fn add_server_reload_listeners(
        &self,
        _host: &dyn Host,
        event: &mut AddServerReloadListenersEvent<'_>,
    ) {
        let id = |text| Identifier::parse(text).unwrap();
        event
            .listeners
            .add_listener(
                id("reload_test:data"),
                FileToIdConverter::json("machines").unwrap(),
                Arc::new(Self {
                    live: self.live.clone(),
                    reject: self.reject.clone(),
                }),
            )
            .unwrap();
        event
            .listeners
            .add_listener(
                id("reload_test:check"),
                FileToIdConverter::json("machines").unwrap(),
                Arc::new(CheckListener(self.reject.clone())),
            )
            .unwrap();
        event
            .listeners
            .add_dependency(id("reload_test:data"), id("reload_test:check"))
            .unwrap();
    }
}
fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}
fn request(root: &Path) -> WorldRequest {
    WorldRequest {
        save_root: root.to_owned(),
        folder: "test".into(),
        name: Some("test".into()),
        seed: Some(1),
        creative: true,
        create: None,
    }
}

#[test]
#[ignore = "needs prepared resources"]
fn server_reload_delivers_mod_files_overrides_and_preserves_all_data_on_failure() {
    let root = std::env::current_dir()
        .unwrap()
        .join("target")
        .join(format!("server-reload-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let manifest = "id = \"reload_test\"\nname = \"Reload Test\"\nversion = \"0.1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n";
    let package = root.join("reload_test.hmod");
    write(
        &package,
        horizon_mods::Package::write_files(
            manifest,
            &[("data/reload_test/machines/value.json", b"1")],
        )
        .unwrap(),
    );
    let package_manifest = horizon_mods::Package::read(&package).unwrap().manifest;
    let content = horizon_mods::Content::read(&package, &package_manifest).unwrap();
    let resources = root.join("resources");
    let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    horizon_client::mod_content::build(
        &base,
        &[horizon_client::mod_content::ContentMod {
            id: "reload_test".into(),
            content,
        }],
        &resources,
    )
    .unwrap();
    let world = root.join("world");
    let pack = world.join("datapacks/test");
    write(&pack.join("pack.mcmeta"), "{}");
    let data = pack.join("data/reload_test/machines/value.json");
    write(&data, "2");
    // Other namespaces and subdirectories join the same directory listener.
    write(&pack.join("data/other/machines/heavy/extra.json"), "5");
    write(&pack.join("data/other/machines/ignored.txt"), "not json");
    let function = pack.join("data/reload_test/function/keep.mcfunction");
    write(&function, "say old");
    write(
        &world.join("datapacks.json"),
        r#"{"enabled":["file/test"],"disabled":[]}"#,
    );
    let live = Arc::new(Mutex::new(0));
    let reject = Arc::new(AtomicBool::new(false));
    let mods = vec![ModEntry::new(
        "reload_test",
        Box::new(DataListener {
            live: live.clone(),
            reject: reject.clone(),
        }),
    )]
    .into();
    mod_api::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.resources = resources;
            game.load_world(request(&world)).unwrap();
            assert_eq!(
                *live.lock().unwrap(),
                7,
                "override 2 plus other namespace 5"
            );
            assert!(game.run_command("datapack disable \"file/test\""));
            assert_eq!(
                *live.lock().unwrap(),
                1,
                "the mod's packaged file is the fallback"
            );
            assert!(game.run_command("datapack enable \"file/test\""));
            assert_eq!(*live.lock().unwrap(), 7);
            write(&data, "3");
            assert!(game.run_command("reload"));
            assert_eq!(*live.lock().unwrap(), 8);
            let old_function = game.fns.store.functions["reload_test:keep"].clone();
            let old_selection = std::fs::read(world.join("datapacks.json")).unwrap();
            let old_enabled = game.fns.enabled.clone();
            let old_vanilla_disabled = game.cmds.vanilla_disabled;
            write(&data, "4");
            write(&function, "say new");
            reject.store(true, Ordering::Relaxed);
            assert!(!game.run_command("reload"));
            assert_eq!(
                *live.lock().unwrap(),
                8,
                "earlier successful prepare must not publish"
            );
            assert_eq!(game.fns.store.functions["reload_test:keep"], old_function);
            let error = game
                .reload_datapacks(true, &game.level.enabled_features.clone())
                .err()
                .unwrap()
                .to_string();
            assert!(
                error.contains("reload listener reload_test:check: reload_test:value"),
                "{error}"
            );
            assert!(!game.run_command("datapack disable \"file/test\""));
            assert_eq!(
                std::fs::read(world.join("datapacks.json")).unwrap(),
                old_selection
            );
            assert_eq!(
                game.fns.enabled, old_enabled,
                "the refused change publishes nothing"
            );
            assert_eq!(
                game.cmds.vanilla_disabled, old_vanilla_disabled,
                "the refused change flips no command switch"
            );
            assert!(game.fns.store.functions.contains_key("reload_test:keep"));
            assert_eq!(*live.lock().unwrap(), 8);
            reject.store(false, Ordering::Relaxed);
            write(&data, "not JSON");
            let error = game
                .reload_datapacks(true, &game.level.enabled_features.clone())
                .err()
                .unwrap()
                .to_string();
            assert!(
                error.contains("reload listener reload_test:data: reload_test:value"),
                "{error}"
            );
            assert_eq!(*live.lock().unwrap(), 8);
            assert_eq!(game.fns.store.functions["reload_test:keep"], old_function);
        })
    });
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "needs prepared resources"]
fn server_reload_skips_unparsable_files_and_delivers_the_rest() {
    // Vanilla's `SimpleJsonResourceReloadListener.scanDirectory` logs and skips one bad
    // file; only a listener's own error aborts the reload. This listener follows it.
    struct ParsingListener(Arc<Mutex<Vec<(String, String)>>>);
    impl ServerReloadListener for ParsingListener {
        fn prepare(
            &self,
            context: &ServerReloadContext,
        ) -> Result<ServerReloadApply, ServerReloadError> {
            let mut parsed = Vec::new();
            for id in context.list() {
                match serde_json::from_slice::<String>(context.read(id).unwrap()) {
                    Ok(value) => parsed.push((id.to_string(), value)),
                    Err(error) => log::warn!("Couldn't parse data file '{id}': {error}"),
                }
            }
            let live = self.0.clone();
            Ok(ServerReloadApply::new(move || {
                *live.lock().unwrap() = parsed
            }))
        }
    }
    struct SkipHooks(Arc<Mutex<Vec<(String, String)>>>);
    #[mod_api::__subscriptions]
    impl Hooks for SkipHooks {
        fn add_server_reload_listeners(
            &self,
            _host: &dyn Host,
            event: &mut AddServerReloadListenersEvent<'_>,
        ) {
            event
                .listeners
                .add_listener(
                    Identifier::parse("reload_test:data").unwrap(),
                    FileToIdConverter::json("machines").unwrap(),
                    Arc::new(ParsingListener(self.0.clone())),
                )
                .unwrap();
        }
    }
    let root = std::env::current_dir()
        .unwrap()
        .join("target")
        .join(format!("server-reload-skip-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let manifest = "id = \"reload_test\"\nname = \"Reload Test\"\nversion = \"0.1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n";
    let package = root.join("reload_test.hmod");
    write(
        &package,
        horizon_mods::Package::write_files(
            manifest,
            &[("data/reload_test/machines/bar.json", b"\"good\"")],
        )
        .unwrap(),
    );
    let package_manifest = horizon_mods::Package::read(&package).unwrap().manifest;
    let content = horizon_mods::Content::read(&package, &package_manifest).unwrap();
    let resources = root.join("resources");
    let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    horizon_client::mod_content::build(
        &base,
        &[horizon_client::mod_content::ContentMod {
            id: "reload_test".into(),
            content,
        }],
        &resources,
    )
    .unwrap();
    let world = root.join("world");
    let pack = world.join("datapacks/test");
    write(&pack.join("pack.mcmeta"), "{}");
    let malformed = pack.join("data/reload_test/machines/foo.json");
    write(&malformed, "{bad json");
    write(
        &pack.join("data/reload_test/machines/bar.json"),
        "\"better\"",
    );
    write(
        &world.join("datapacks.json"),
        r#"{"enabled":["file/test"],"disabled":[]}"#,
    );
    let live = Arc::new(Mutex::new(Vec::new()));
    mod_api::testing::with_mods(
        vec![ModEntry::new(
            "reload_test",
            Box::new(SkipHooks(live.clone())),
        )]
        .into(),
        || {
            with_game(|game| {
                game.options.resources = resources;
                game.load_world(request(&world)).unwrap();
                assert_eq!(
                    *live.lock().unwrap(),
                    vec![("reload_test:bar".into(), "better".into())],
                    "the malformed file is skipped and the override wins"
                );
                write(&malformed, "\"fixed\"");
                assert!(game.run_command("reload"));
                assert_eq!(
                    *live.lock().unwrap(),
                    vec![
                        ("reload_test:bar".into(), "better".into()),
                        ("reload_test:foo".into(), "fixed".into()),
                    ],
                    "a repaired file joins the next reload"
                );
                write(&malformed, "still not json");
                assert!(
                    game.run_command("reload"),
                    "a skipped file never fails the reload"
                );
                assert_eq!(
                    *live.lock().unwrap(),
                    vec![("reload_test:bar".into(), "better".into())],
                    "the bad file is skipped again"
                );
            })
        },
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "needs prepared resources"]
fn server_reload_registration_failure_stops_world_open_and_quiet_mods_never_listen() {
    struct Quiet;
    #[mod_api::__subscriptions]
    impl Hooks for Quiet {}
    struct Broken;
    #[mod_api::__subscriptions]
    impl Hooks for Broken {
        fn add_server_reload_listeners(
            &self,
            _host: &dyn Host,
            event: &mut AddServerReloadListenersEvent<'_>,
        ) {
            let _ = event.listeners.add_dependency(
                Identifier::parse("mistyped:listener").unwrap(),
                VanillaServerListeners::functions(),
            );
        }
    }
    let root = std::env::current_dir()
        .unwrap()
        .join("target")
        .join(format!("server-reload-open-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    mod_api::testing::with_mods(vec![ModEntry::new("quiet", Box::new(Quiet))].into(), || {
        with_game(|game| {
            // A mod that doesn't implement the hook is never subscribed, so
            // world open and `/reload` skip the registration graph.
            assert!(!mod_api::listened(
                mod_api::HookId::AddServerReloadListeners
            ));
            game.load_world(request(&root)).unwrap();
            assert!(!mod_api::listened(
                mod_api::HookId::AddServerReloadListeners
            ));
            assert!(game.run_command("reload"));
            assert!(!mod_api::listened(
                mod_api::HookId::AddServerReloadListeners
            ));
        })
    });
    std::fs::remove_dir_all(&root).unwrap();
    mod_api::testing::with_mods(
        vec![ModEntry::new("broken_mod", Box::new(Broken))].into(),
        || {
            with_game(|game| {
                let error = game.load_world(request(&root)).unwrap_err().to_string();
                assert!(
                    error.contains("unknown reload listener mistyped:listener"),
                    "{error}"
                );
                assert!(game.fns.store.functions.is_empty());
            })
        },
    );
    std::fs::remove_dir_all(root).unwrap();
}
