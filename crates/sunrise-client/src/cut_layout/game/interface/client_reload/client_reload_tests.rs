use mod_api::__private::reload::ReloadStageError;
use mod_api::ClientReloadListenerContext;
use mod_api::HookId;
use mod_api::{
    ClientReloadContext, ClientReloadListener, ClientReloadListeners, FileToIdConverter,
    Identifier, ReloadResourceError,
};
use std::{path::PathBuf, sync::Arc};
use sunrise_client_gameplay::game::client_reload::{
    APPLY_GENERATIONS, PreparedClientReload, register_startup_client_reload_listeners,
    stage_client_reload,
};
use sunrise_client_gameplay::game::native_hooks::game_shutting_down::GameHooks;

fn id(value: &str) -> Identifier {
    Identifier::parse(value).unwrap()
}
fn hats() -> FileToIdConverter {
    FileToIdConverter::json("hats").unwrap()
}

fn apply_client_reload(prepared: Vec<PreparedClientReload>) {
    mod_api::__private::reload::apply_listeners(
        prepared,
        HookId::AddClientReloadListeners,
        "client_reload_apply",
        |()| Ok::<_, std::convert::Infallible>(()),
    )
    .unwrap();
}

/// A listener recording the context it prepared and applying to a shared log.
struct HatListener {
    seen: std::sync::Mutex<Vec<String>>,
    applied: Arc<std::sync::Mutex<Vec<String>>>,
}
impl ClientReloadListener for HatListener {
    fn prepare(
        &self,
        context: &ClientReloadContext,
    ) -> Result<Box<dyn FnOnce() + Send + 'static>, ReloadResourceError> {
        let seen: Vec<String> = context
            .list()
            .filter_map(|id| {
                context
                    .read(id)
                    .map(|bytes| format!("{}={}", id, String::from_utf8_lossy(bytes)))
            })
            .collect();
        self.seen.lock().unwrap().extend(seen);
        let applied = self.applied.clone();
        Ok(Box::new(move || {
            applied.lock().unwrap().push("hats".into());
        }))
    }
}

/// A temp root with one asset under `assets/test_mod/hats/`; each test gets
/// its own, so parallel runs write to different folders.
fn root_with_asset(name: &str, bytes: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sunrise-client-reload-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let asset = dir.join("assets/test_mod/hats/top.json");
    std::fs::create_dir_all(asset.parent().unwrap()).unwrap();
    std::fs::write(asset, bytes).unwrap();
    dir
}

#[test]
#[ignore = "daily"]
fn panicking_client_reload_registration_discards_its_listener_and_edges() {
    use mod_api::{AddClientReloadListenersEvent, Hooks, ModEntry, VanillaClientListeners};
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };
    use {
        crate::game::server_side::native_entity_refs::tests::with_uncached_client_game,
        sunrise_client_gameplay::game::packs::stage_pack, sunrise_gameplay_common::game::Progress,
    };

    struct Listener {
        prepare: &'static str,
        apply: &'static str,
        trace: Arc<Mutex<Vec<&'static str>>>,
    }
    impl ClientReloadListener for Listener {
        fn prepare(
            &self,
            _: &ClientReloadContext,
        ) -> Result<Box<dyn FnOnce() + Send + 'static>, ReloadResourceError> {
            self.trace.lock().unwrap().push(self.prepare);
            let (apply, trace) = (self.apply, self.trace.clone());
            Ok(Box::new(move || trace.lock().unwrap().push(apply)))
        }
    }

    struct Register {
        fails: bool,
        staged_failure: Arc<AtomicBool>,
        trace: Arc<Mutex<Vec<&'static str>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Register {
        fn add_client_reload_listeners(
            &self,
            _: &dyn ClientReloadListenerContext,
            event: &mut AddClientReloadListenersEvent<'_>,
        ) {
            let anchor = VanillaClientListeners::model_manager();
            let after = id("reload_fixture:after");
            event
                .listeners
                .add_listener(
                    after.clone(),
                    hats(),
                    Arc::new(Listener {
                        prepare: if self.fails {
                            "prepare discarded"
                        } else {
                            "prepare after"
                        },
                        apply: if self.fails {
                            "apply discarded"
                        } else {
                            "apply after"
                        },
                        trace: self.trace.clone(),
                    }),
                )
                .unwrap();
            if self.fails {
                event.listeners.add_dependency(after, anchor).unwrap();
                self.staged_failure.store(true, Ordering::Relaxed);
                panic!("client reload registrar failed after adding a listener and an edge");
            }
            // Reuse the failed mod's name and reverse its edge. A leaked row
            // would refuse this registration; a leaked edge would make a cycle.
            event
                .listeners
                .add_dependency(anchor.clone(), after)
                .unwrap();
            let before = id("reload_fixture:before");
            event
                .listeners
                .add_listener(
                    before.clone(),
                    hats(),
                    Arc::new(Listener {
                        prepare: "prepare before",
                        apply: "apply before",
                        trace: self.trace.clone(),
                    }),
                )
                .unwrap();
            event.listeners.add_dependency(before, anchor).unwrap();
        }
    }

    with_uncached_client_game(|game| {
        let staged_failure = Arc::new(AtomicBool::new(false));
        let trace = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![
            ModEntry::new(
                "broken_reload",
                Box::new(Register {
                    fails: true,
                    staged_failure: staged_failure.clone(),
                    trace: trace.clone(),
                }),
            ),
            ModEntry::new(
                "healthy_reload",
                Box::new(Register {
                    fails: false,
                    staged_failure: staged_failure.clone(),
                    trace: trace.clone(),
                }),
            ),
        ]
        .into();
        mod_platform::testing::with_mods(mods, || {
            let old = game.client.client_pack.render.generation;
            let staged = stage_pack(
                &game.vanilla_root(),
                &[],
                &[],
                &game.mods,
                game.register_client_reload_listeners(
                    &game
                        .capture_candidate_settings(&game.options.resources)
                        .unwrap(),
                ),
                &[],
                &game.pack.assets,
                &game.res,
                game.client_res(),
                &game.client.client_pack.context,
                game.client.settings.force_unicode,
                game.client.settings.japanese_glyphs,
                &Progress::new(),
            )
            .unwrap();
            let candidate = staged.client_pack.render.generation;
            assert_ne!(candidate, old);
            assert_eq!(game.client.client_pack.render.generation, old);
            assert!(staged_failure.load(Ordering::Relaxed));
            assert_eq!(*trace.lock().unwrap(), ["prepare before", "prepare after"]);
            APPLY_GENERATIONS.replace(Some(Vec::new()));
            let result = game.apply_staged_pack(staged);
            let generations = APPLY_GENERATIONS.take().unwrap();
            result.unwrap();
            assert_eq!(game.client.client_pack.render.generation, candidate);
            assert_eq!(generations, [old, candidate]);
            assert_eq!(
                *trace.lock().unwrap(),
                [
                    "prepare before",
                    "prepare after",
                    "apply before",
                    "apply after",
                ]
            );
            assert_eq!(
                mod_api::__private::take_hook_panics().as_slice(),
                [mod_api::__private::HookPanicReport {
                    mod_id: "broken_reload",
                    hook: "add_client_reload_listeners",
                },]
            );
        });
    });
}

#[test]
fn stage_reads_resolved_asset_bytes_without_a_directory() {
    use sunrise_assets::resource_source::{Resource, ResourceSource};
    let source = ResourceSource::indexed(std::collections::BTreeMap::from([(
        "assets/test_mod/hats/top.json".into(),
        Resource::bytes(b"selected".to_vec()),
    )]));
    let applied = Arc::new(std::sync::Mutex::new(Vec::new()));
    let listener = Arc::new(HatListener {
        seen: std::sync::Mutex::new(Vec::new()),
        applied: applied.clone(),
    });
    let mut listeners = <mod_api::ClientListenerRegistry as mod_api::__private::game::ClientReloadListenersOrder>::new();
    listeners
        .add_listener(id("test_mod:hats"), hats(), listener.clone())
        .unwrap();
    let ((), prepared) =
        stage_client_reload(&source, listeners, || Ok::<_, ReloadStageError>(())).unwrap();
    assert_eq!(
        listener.seen.lock().unwrap().as_slice(),
        ["test_mod:top=selected"]
    );
    assert!(applied.lock().unwrap().is_empty());
    apply_client_reload(prepared);
    assert_eq!(applied.lock().unwrap().as_slice(), ["hats"]);
}

#[test]
fn stage_reads_the_merged_assets_and_apply_publishes_after_all_prepares() {
    let root = root_with_asset("merged", b"\"top\"");
    let applied = Arc::new(std::sync::Mutex::new(Vec::new()));
    let listener = Arc::new(HatListener {
        seen: std::sync::Mutex::new(Vec::new()),
        applied: applied.clone(),
    });
    let mut listeners = <mod_api::ClientListenerRegistry as mod_api::__private::game::ClientReloadListenersOrder>::new();
    listeners
        .add_listener(id("test_mod:hats"), hats(), listener.clone())
        .unwrap();
    let ((), prepared) = stage_client_reload(
        &sunrise_assets::resource_source::ResourceSource::directory(root.to_path_buf()),
        listeners,
        || Ok::<_, ReloadStageError>(()),
    )
    .unwrap();
    assert_eq!(
        listener.seen.lock().unwrap().as_slice(),
        ["test_mod:top=\"top\""]
    );
    assert!(applied.lock().unwrap().is_empty(), "nothing published yet");
    apply_client_reload(prepared);
    assert_eq!(applied.lock().unwrap().as_slice(), ["hats"]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_listener_error_retains_its_cause_and_discards_pending_applies() {
    use std::error::Error as _;

    struct Failing;
    #[derive(Debug)]
    struct NotAHat;
    impl std::fmt::Display for NotAHat {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "not a hat")
        }
    }
    impl std::error::Error for NotAHat {}
    impl ClientReloadListener for Failing {
        fn prepare(
            &self,
            _: &ClientReloadContext,
        ) -> Result<Box<dyn FnOnce() + Send + 'static>, ReloadResourceError> {
            Err(ReloadResourceError::new(id("test_mod:broken"), NotAHat))
        }
    }
    let root = root_with_asset("failing", b"{}");
    let applied = Arc::new(std::sync::Mutex::new(Vec::new()));
    let listener = Arc::new(HatListener {
        seen: std::sync::Mutex::new(Vec::new()),
        applied: applied.clone(),
    });
    let mut listeners = <mod_api::ClientListenerRegistry as mod_api::__private::game::ClientReloadListenersOrder>::new();
    listeners
        .add_listener(id("test_mod:pending"), hats(), listener.clone())
        .unwrap();
    listeners
        .add_listener(id("test_mod:failing"), hats(), Arc::new(Failing))
        .unwrap();
    listeners
        .add_dependency(id("test_mod:pending"), id("test_mod:failing"))
        .unwrap();
    let Err(error) = stage_client_reload(
        &sunrise_assets::resource_source::ResourceSource::directory(root.to_path_buf()),
        listeners,
        || Ok::<_, ReloadStageError>(()),
    ) else {
        panic!("a failing listener stops the whole reload")
    };
    assert_eq!(
        error.diagnostic(),
        "reload listener test_mod:failing: test_mod:broken: not a hat"
    );
    let resource_error = error
        .source()
        .unwrap()
        .downcast_ref::<ReloadResourceError>()
        .unwrap();
    assert_eq!(resource_error.resource(), &id("test_mod:broken"));
    assert!(resource_error.source().unwrap().is::<NotAHat>());
    assert_eq!(
        listener.seen.lock().unwrap().as_slice(),
        ["test_mod:top={}"]
    );
    assert!(applied.lock().unwrap().is_empty(), "no partial apply");
    let probe = Arc::downgrade(&applied);
    drop((applied, listener));
    assert!(
        probe.upgrade().is_none(),
        "aborted apply retained its capture"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_converted_file_outside_the_pack_is_absent_not_empty() {
    let root = root_with_asset("absent", b"{}");
    let listener = Arc::new(HatListener {
        seen: std::sync::Mutex::new(Vec::new()),
        applied: Arc::new(std::sync::Mutex::new(Vec::new())),
    });
    let mut listeners = <mod_api::ClientListenerRegistry as mod_api::__private::game::ClientReloadListenersOrder>::new();
    listeners
        .add_listener(id("test_mod:hats"), hats(), listener.clone())
        .unwrap();
    stage_client_reload(
        &sunrise_assets::resource_source::ResourceSource::directory(root.to_path_buf()),
        listeners,
        || Ok::<_, ReloadStageError>(()),
    )
    .unwrap();
    assert_eq!(
        listener.seen.lock().unwrap().as_slice(),
        ["test_mod:top={}"],
        "the staged root holds only the merged asset"
    );
    std::fs::remove_dir_all(root).unwrap();
}

/// First load uses the same model barrier as a reload. The model must be baked
/// during prepare, then published to the real Game between the two applies.
#[test]
#[ignore = "daily"]
fn initial_load_stages_and_applies_client_reload_listeners() {
    // Start a fresh process for the startup path, which installs the standalone model fixture.
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        "cut_layout::game::client_side::client_reload::tests::initial_load_stages_and_applies_client_reload_listeners",
        "SUNRISE_INITIAL_RELOAD_CHILD",
        &["startup"],
    ) {
        return;
    }
    use mod_api::{
        __private::game::RegistrationContextNew, AddClientReloadListenersEvent, ClientHost,
        ClientItemExtensionsRegistrar, ClientReloadListenerContext, Hooks, ModEntry,
        ModelBakingCompletedEvent, RegistrationContext, StandaloneModelsEvent,
        VanillaClientListeners,
    };
    use std::{sync::Mutex, time::Instant};
    use sunrise_assets::resource_source::{ResourceRoot, ResourceSource};
    use {
        crate::settings::Settings,
        sunrise_client_gameplay::game::client_side::loading_window::start_client_game,
        sunrise_client_gameplay::game::client_side::resource_load_client::load_resources_for_mods,
        sunrise_client_lib::audio::Audio,
        sunrise_gameplay::game::native_entity_refs::tests::fixture_root,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    };

    struct Listener {
        trace: Arc<Mutex<Vec<&'static str>>>,
        before: bool,
    }
    impl ClientReloadListener for Listener {
        fn prepare(
            &self,
            context: &ClientReloadContext,
        ) -> Result<Box<dyn FnOnce() + Send + 'static>, ReloadResourceError> {
            assert!(context.list().any(|id| id.namespace() == "minecraft"));
            let (prepare, apply) = if self.before {
                ("prepare before", "apply before")
            } else {
                ("prepare after", "apply after")
            };
            self.trace.lock().unwrap().push(prepare);
            let trace = self.trace.clone();
            Ok(Box::new(move || trace.lock().unwrap().push(apply)))
        }
    }
    struct Broken(Arc<Mutex<Vec<&'static str>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Broken {
        fn add_client_reload_listeners(
            &self,
            _context: &dyn ClientReloadListenerContext,
            event: &mut AddClientReloadListenersEvent<'_>,
        ) {
            let after = id("startup_anchor:after");
            event
                .listeners
                .add_listener(
                    after.clone(),
                    FileToIdConverter::json("post_effect").unwrap(),
                    Arc::new(Listener {
                        trace: self.0.clone(),
                        before: false,
                    }),
                )
                .unwrap();
            event
                .listeners
                .add_dependency(after, VanillaClientListeners::model_manager())
                .unwrap();
            self.0.lock().unwrap().push("discard registration");
            panic!("do not stage this listener or its edge");
        }
    }
    thread_local! {
        static CHECK_UNKNOWN_TEXTURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    struct Probe(Arc<Mutex<Vec<&'static str>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Probe {
        fn add_client_reload_listeners(
            &self,
            _: &dyn ClientReloadListenerContext,
            event: &mut AddClientReloadListenersEvent<'_>,
        ) {
            let anchor = VanillaClientListeners::model_manager();
            let before = id("startup_anchor:before");
            let after = id("startup_anchor:after");
            for (key, before) in [(before.clone(), true), (after.clone(), false)] {
                event
                    .listeners
                    .add_listener(
                        key,
                        FileToIdConverter::json("post_effect").unwrap(),
                        Arc::new(Listener {
                            trace: self.0.clone(),
                            before,
                        }),
                    )
                    .unwrap();
            }
            event
                .listeners
                .add_dependency(before, anchor.clone())
                .unwrap();
            event.listeners.add_dependency(anchor, after).unwrap();
        }

        fn standalone_models(
            &self,
            _: &RegistrationContext,
            event: &mut StandaloneModelsEvent<'_>,
        ) {
            let trace = self.0.clone();
            event.register(
                "startup_anchor:stone",
                ["minecraft:block/stone"],
                move |bake| {
                    trace.lock().unwrap().push("bake model");
                    bake.append_dependency("minecraft:block/stone", [0; 3], false)
                },
            );
        }

        fn register_client_item_extensions(
            &self,
            _: &RegistrationContext,
            event: &mut mod_api::RegisterClientItemExtensionsEvent<'_>,
        ) {
            let extensions: &mut dyn ClientItemExtensionsRegistrar = &mut *event.extensions;
            if CHECK_UNKNOWN_TEXTURE.get() {
                assert_eq!(
                    extensions.register_texture("startup_anchor:missing"),
                    Err("unknown armour texture startup_anchor:missing".into())
                );
                self.0.lock().unwrap().push("refuse unknown texture");
            }
        }

        fn texture_atlas_stitched(
            &self,
            host: &mut dyn ClientHost,
            event: &mut mod_api::TextureAtlasStitchedEvent,
        ) {
            assert_eq!(
                host.render_assets().model(&id("startup_anchor:stone")),
                Err(mod_api::ModelError::UnknownModel)
            );
            if matches!(event.atlas, mod_api::TextureAtlas::Blocks) {
                self.0.lock().unwrap().push("publish atlas without models");
            }
        }

        fn model_baking_completed(
            &self,
            host: &mut dyn ClientHost,
            _: &mut ModelBakingCompletedEvent,
        ) {
            assert!(
                host.render_assets()
                    .model(&id("startup_anchor:stone"))
                    .is_ok()
            );
            self.0.lock().unwrap().push("observe published models");
        }
    }

    std::thread::Builder::new()
        .stack_size(sunrise_threads::GAME_THREAD_STACK_BYTES)
        .spawn(|| {
            let trace = Arc::new(Mutex::new(Vec::new()));
            let mods = vec![
                ModEntry::new("broken_startup_reload", Box::new(Broken(trace.clone()))),
                ModEntry::new("startup_anchor", Box::new(Probe(trace.clone()))),
            ]
            .into();
            mod_platform::testing::with_mods(mods, || {
                let mut registrations = Vec::new();
                sunrise_gameplay_common::game::native_hooks::native_standalone_models(
                    &RegistrationContext::new(),
                    &mut registrations,
                );
                let _models =
                    sunrise_client::models::StandaloneFixture::from_registrations(registrations);
                for (reject_renderer, unknown_texture) in
                    [(false, false), (true, false), (false, true)]
                {
                    CHECK_UNKNOWN_TEXTURE.set(unknown_texture);
                    trace.lock().unwrap().clear();
                    let root = fixture_root();
                    let scratch = tempfile::tempdir().unwrap();
                    let started = Instant::now();
                    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
                    options.resources = root.clone();
                    options.game_dir = Some(scratch.path().to_owned());
                    options.execution_role = sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
                    let settings = Settings::default();

                    let loaded = load_resources_for_mods(
                        ResourceRoot::directory(root.clone()).into(),
                        Vec::new(),
                        None,
                        &root,
                        &settings,
                        started,
                        false,
                        options.execution_role,
                        &Progress::new(),
                        &mut |_, _| unreachable!(),
                        &mut || unreachable!("default fixture has no optional packs"),
                        &options.mods,
                    )
                    .unwrap();
                    if !reject_renderer && !unknown_texture {
                        let reports = mod_api::__private::take_hook_panics();
                        assert_eq!(
                            reports.as_slice(),
                            [mod_api::__private::HookPanicReport {
                                mod_id: "broken_startup_reload",
                                hook: "add_client_reload_listeners",
                            }]
                        );
                        assert_eq!(trace.lock().unwrap().remove(0), "discard registration");
                    }
                    assert_eq!(
                        *trace.lock().unwrap(),
                        ["prepare before", "bake model", "prepare after"]
                    );
                    assert!(loaded.pack_error.is_none());
                    options.resources = loaded.root;
                    let mut game = start_client_game(
                        &loaded.resources,
                        loaded.client_res.unwrap(),
                        loaded.pack,
                        loaded.client_pack.unwrap(),
                        loaded.gui.unwrap(),
                        options,
                        loaded.pack_error,
                        Audio::silent(&ResourceSource::directory(root.clone())),
                        root,
                        started,
                        loaded.rest_loader,
                        settings,
                        loaded.game_cpu,
                        loaded.client_applies,
                        loaded.entity_models,
                        None,
                        sunrise_gameplay::game::server_side::integrated::serve,
                        sunrise_gameplay::game::server_side::integrated::run_component_harness_on_server,
                        crate::game::session_verifier(),
                        crate::game::profile_resolver,
                    )
                    .unwrap()
                    .unwrap();
                    assert_eq!(game.client.client_pack.render.generation, 0);
                    assert_eq!(
                        game.client
                            .client_pack
                            .render
                            .model(&id("startup_anchor:stone")),
                        Err(mod_api::ModelError::UnknownModel)
                    );
                    let pending = game.client.pending_initial_models.0.take().unwrap();
                    let candidate = pending.models.client_pack.render.generation;
                    let renderer = if reject_renderer {
                        Err("renderer validation refused".into())
                    } else {
                        Ok(None)
                    };
                    APPLY_GENERATIONS.replace(Some(Vec::new()));
                    let result = game.apply_pending_client_start(*pending, renderer);
                    let generations = APPLY_GENERATIONS.take().unwrap();
                    if reject_renderer {
                        let error = result.expect_err("startup must refuse a failed renderer");
                        assert!(error.to_string().contains("renderer validation refused"));
                        assert_eq!(game.client.client_pack.render.generation, 0);
                        assert!(generations.is_empty());
                        assert_eq!(
                            *trace.lock().unwrap(),
                            ["prepare before", "bake model", "prepare after"]
                        );
                        continue;
                    }
                    assert!(result.unwrap());
                    assert_eq!(game.client.client_pack.render.generation, candidate);
                    assert_ne!(candidate, 0);
                    assert_eq!(
                        generations,
                        [0, candidate],
                        "generation at the before- and after-model applies"
                    );
                    let mut expected = vec!["prepare before", "bake model", "prepare after"];
                    if unknown_texture {
                        // Returning an extension error rejects just that binding; startup still applies.
                        expected.push("refuse unknown texture");
                    }
                    expected.extend([
                        "publish atlas without models",
                        "apply before",
                        "observe published models",
                        "apply after",
                    ]);
                    assert_eq!(*trace.lock().unwrap(), expected);
                }
            });
        })
        .unwrap()
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

/// A listener registered through the real dispatch, staged against a real
/// merged root (the game's own resources with a dev mod and a resource pack).
#[test]
#[ignore = "daily"]
fn client_reload_reads_real_assets_with_pack_overrides() {
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use mod_api::{self, ModEntry};
    use mod_api::{AddClientReloadListenersEvent, Hooks};
    use mod_platform::testing::with_mods;
    use std::{
        collections::BTreeSet,
        sync::Mutex,
        sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    let dir = std::env::temp_dir().join(format!("client-reload-real-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mod_dir = dir.join("mod");
    let pack_dir = dir.join("packs/override");
    let asset = "assets/client_test/hats/top.json";
    for root in [&mod_dir, &pack_dir] {
        std::fs::create_dir_all(root.join(asset).parent().unwrap()).unwrap();
    }
    std::fs::write(
        mod_dir.join("sunrise-mod.toml"),
        "id = \"client_test\"\nname = \"Client test\"\nversion = \"1\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\n",
    )
    .unwrap();
    std::fs::write(mod_dir.join(asset), b"mod").unwrap();
    std::fs::write(pack_dir.join(asset), b"pack").unwrap();
    std::fs::write(
        pack_dir.join("pack.mcmeta"),
        br#"{"pack":{"description":"test","pack_format":97}}"#,
    )
    .unwrap();

    // Only the archive remains when loading starts.
    {
        use std::io::Write;
        let path = dir.join("packs/override.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        for name in ["pack.mcmeta", asset] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(&std::fs::read(pack_dir.join(name)).unwrap())
                .unwrap();
        }
        zip.finish().unwrap();
        std::fs::remove_dir_all(&pack_dir).unwrap();
    }

    struct HatListener {
        live: Arc<Mutex<Vec<String>>>,
        reject: Arc<AtomicBool>,
        vanilla: Arc<AtomicUsize>,
    }
    impl ClientReloadListener for HatListener {
        fn prepare(
            &self,
            context: &ClientReloadContext,
        ) -> Result<Box<dyn FnOnce() + Send + 'static>, ReloadResourceError> {
            if self.reject.load(Ordering::Relaxed) {
                return Err(ReloadResourceError::new(
                    id("client_test:top"),
                    std::io::Error::other("rejected new hats"),
                ));
            }
            let top = id("client_test:top");
            let value = context
                .read(&top)
                .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
                .unwrap_or_default();
            let live = self.live.clone();
            Ok(Box::new(move || live.lock().unwrap().push(value)))
        }
    }
    #[mod_api::__subscriptions]
    impl Hooks for HatListener {
        fn add_client_reload_listeners(
            &self,
            _context: &dyn ClientReloadListenerContext,
            event: &mut AddClientReloadListenersEvent<'_>,
        ) {
            event
                .listeners
                .add_listener(
                    id("client_test:hats"),
                    hats(),
                    Arc::new(HatListener {
                        live: self.live.clone(),
                        reject: self.reject.clone(),
                        vanilla: self.vanilla.clone(),
                    }),
                )
                .unwrap();
            // The real root's own assets, through a directory vanilla ships.
            event
                .listeners
                .add_listener(
                    id("client_test:vanilla"),
                    FileToIdConverter::json("post_effect").unwrap(),
                    Arc::new(VanillaProbe(self.vanilla.clone())),
                )
                .unwrap();
        }
    }

    /// Counts vanilla's own `post_effect` files in the same merged view.
    struct VanillaProbe(Arc<AtomicUsize>);
    impl ClientReloadListener for VanillaProbe {
        fn prepare(
            &self,
            context: &ClientReloadContext,
        ) -> Result<Box<dyn FnOnce() + Send + 'static>, ReloadResourceError> {
            let count = context
                .list()
                .filter(|id| id.namespace() == "minecraft")
                .count();
            let seen = self.0.clone();
            Ok(Box::new(move || {
                seen.fetch_max(count, Ordering::SeqCst);
            }))
        }
    }

    let live = Arc::new(Mutex::new(Vec::new()));
    let reject = Arc::new(AtomicBool::new(false));
    let vanilla = Arc::new(AtomicUsize::new(0));
    let mods = vec![ModEntry::new(
        "client_test",
        Box::new(HatListener {
            live: live.clone(),
            reject: reject.clone(),
            vanilla: vanilla.clone(),
        }),
    )]
    .into();
    with_mods(mods, || {
        with_client_game(|game| {
            let resources = game.options.resources.clone();
            let enabled = BTreeSet::new();
            let mut sources = sunrise_gameplay_common::game::mods::ModsState {
                dev: vec![mod_dir.clone()],
                ..Default::default()
            };
            sources.select(&std::env::current_dir().unwrap(), &enabled);
            // The merged root without `startup_root`'s process-wide mod
            // registrations, which a test must not freeze for the others
            // (`brewing`'s mod effects among them).
            let merged = |packs: &[sunrise_client::packs::Pack]| {
                let base = sunrise_gameplay_common::game::mods::content_overlay(
                    &resources,
                    &sources,
                    &sunrise_level::logging::game_home().join("jars"),
                )
                .unwrap()
                .root;
                sunrise_client::packs::resources(base.source().clone(), packs).unwrap()
            };
            // The mod's packaged asset over nothing, with the real root's own
            // assets in the same view.
            let root = merged(&[]);
            let ((), prepared) = stage_client_reload(
                &root,
                game.register_client_reload_listeners(
                    &game
                        .capture_candidate_settings(&game.options.resources)
                        .unwrap(),
                ),
                || Ok::<_, ReloadStageError>(()),
            )
            .unwrap();
            assert!(live.lock().unwrap().is_empty(), "nothing published yet");
            apply_client_reload(prepared);
            assert_eq!(live.lock().unwrap().as_slice(), ["mod"]);
            assert!(
                vanilla.load(Ordering::SeqCst) >= 5,
                "the real root's own post_effect assets join the view"
            );
            // A selected resource pack overrides the mod's packaged asset.
            let packs = sunrise_client::packs::folder(
                &dir.join("packs"),
                sunrise_client::packs::RESOURCE_FORMAT,
            );
            assert_eq!(packs.len(), 1);
            let root = merged(&packs);
            let ((), prepared) = stage_client_reload(
                &root,
                game.register_client_reload_listeners(
                    &game
                        .capture_candidate_settings(&game.options.resources)
                        .unwrap(),
                ),
                || Ok::<_, ReloadStageError>(()),
            )
            .unwrap();
            apply_client_reload(prepared);
            assert_eq!(live.lock().unwrap().as_slice(), ["mod", "pack"]);
            // A rejected prepare aborts the reload and keeps the previous state.
            reject.store(true, Ordering::Relaxed);
            let root = merged(&[]);
            let Err(error) = stage_client_reload(
                &root,
                game.register_client_reload_listeners(
                    &game
                        .capture_candidate_settings(&game.options.resources)
                        .unwrap(),
                ),
                || Ok::<_, ReloadStageError>(()),
            ) else {
                panic!("a failing listener stops the whole reload")
            };
            assert!(error.to_string().contains("client_test:hats"));
            assert_eq!(
                live.lock().unwrap().as_slice(),
                ["mod", "pack"],
                "the refused reload publishes nothing"
            );
        });
    });
    std::fs::remove_dir_all(dir).unwrap();
}

/// `client_setup` reaches every listener with the game's client host, one call per
/// firing; `run` fires it once per process, so a listener sees one start.
#[test]
#[ignore = "daily"]
fn client_setup_reaches_each_listener_once_per_firing() {
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use mod_api::{ClientSetupContext, Hooks, ModEntry};
    use mod_platform::testing::with_mods;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Setup(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for Setup {
        fn client_setup(
            &self,
            _context: &mut dyn ClientSetupContext,
            _event: &mut mod_api::ClientSetupEvent,
        ) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let (first, second) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let mods = vec![
        ModEntry::new("setup_a", Box::new(Setup(first.clone()))),
        ModEntry::new("setup_b", Box::new(Setup(second.clone()))),
    ]
    .into();
    with_mods(mods, || {
        with_client_game(|game| {
            game.native_client_setup();
        });
    });
    assert_eq!(
        (first.load(Ordering::SeqCst), second.load(Ordering::SeqCst)),
        (1, 1)
    );
}

/// Only the client role fires client startup hooks.
#[test]
#[ignore = "daily"]
fn only_the_first_local_client_is_the_client_start() {
    use sunrise_client_gameplay::game::client_side::launch_options_client::is_client_start;
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_gameplay_common::game::execution_role::ExecutionRole;

    with_game(|game| {
        game.options.execution_role = ExecutionRole::Client;
        assert!(is_client_start(&game.options));
        game.options.execution_role = ExecutionRole::DedicatedServer;
        assert!(!is_client_start(&game.options));
    });
}

/// Server and client lifecycle moments reach a listener once each and in NeoForge's
/// order: the server's start (`server_about_to_start`, `server_starting`, `server_started`
/// once per open), its stop (`server_stopping`, `server_stopped`, `game_shutting_down`),
/// and the client's own close (`client_stopping`, `client_stopped`, `game_shutting_down`).
#[test]
#[ignore = "daily"]
fn the_lifecycle_moments_fire_once_each_in_order() {
    use mod_api::{ClientHost, Hooks, ModEntry, ServerHost};
    use mod_platform::testing::with_mods;
    use std::sync::Mutex;
    use {
        crate::game::server_side::native_entity_refs::tests::with_client_game,
        sunrise_gameplay::game::native_entity_refs::tests::with_game,
    };

    struct Log(Arc<Mutex<Vec<String>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Log {
        fn server_about_to_start(
            &self,
            _host: &mut dyn ServerHost,
            e: &mut mod_api::ServerAboutToStartEvent,
        ) {
            self.0
                .lock()
                .unwrap()
                .push(format!("about_to_start dedicated={}", e.dedicated));
        }
        fn server_starting(
            &self,
            _host: &mut dyn ServerHost,
            e: &mut mod_api::ServerStartingEvent,
        ) {
            self.0
                .lock()
                .unwrap()
                .push(format!("starting dedicated={}", e.dedicated));
        }
        fn server_stopped(&self, _host: &mut dyn ServerHost, e: &mut mod_api::ServerStoppedEvent) {
            self.0
                .lock()
                .unwrap()
                .push(format!("stopped dedicated={}", e.dedicated));
        }
        fn server_started(&self, _host: &mut dyn ServerHost, e: &mut mod_api::ServerStartedEvent) {
            self.0
                .lock()
                .unwrap()
                .push(format!("started dedicated={}", e.dedicated));
        }
        fn server_stopping(
            &self,
            _host: &mut dyn ServerHost,
            e: &mut mod_api::ServerStoppingEvent,
        ) {
            self.0
                .lock()
                .unwrap()
                .push(format!("stopping dedicated={}", e.dedicated));
        }
        fn client_stopping(
            &self,
            _host: &mut dyn ClientHost,
            _event: &mut mod_api::ClientStoppingEvent,
        ) {
            self.0.lock().unwrap().push("client_stopping".into());
        }
        fn client_stopped(
            &self,
            _host: &mut dyn ClientHost,
            _event: &mut mod_api::ClientStoppedEvent,
        ) {
            self.0.lock().unwrap().push("client_stopped".into());
        }
        fn game_shutting_down(
            &self,
            host: mod_api::HostSide<'_>,
            _event: &mut mod_api::GameShuttingDownEvent,
        ) {
            let side = match host {
                mod_api::HostSide::Client(_) => "client",
                mod_api::HostSide::Server(_) => "server",
            };
            self.0.lock().unwrap().push(format!("shutting_down {side}"));
        }
    }

    let log = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new("log", Box::new(Log(log.clone())))].into();
    with_mods(mods, || {
        with_game(|game| {
            game.options.execution_role =
                sunrise_gameplay_common::game::execution_role::ExecutionRole::DedicatedServer;
            game.native_lifecycle_world_closed();
            log.lock().unwrap().clear();
            game.native_server_about_to_start(true);
            game.native_lifecycle_open_started();
            game.native_server_starting(true);
            game.native_lifecycle_world_ready();
            game.native_lifecycle_world_ready();
            game.native_lifecycle.begin_close();
            game.native_server_stopping(true);
            game.native_lifecycle_world_closed();
            game.native_lifecycle_world_closed();
            game.native_server_stopped(true);
            game.native_game_shutting_down();
        });
        with_client_game(|game| {
            game.native_client_stopping();
            game.native_client_stopped();
            game.native_game_shutting_down();
        });
    });
    assert_eq!(
        *log.lock().unwrap(),
        [
            "about_to_start dedicated=true",
            "starting dedicated=true",
            "started dedicated=true",
            "stopping dedicated=true",
            "stopped dedicated=true",
            "shutting_down server",
            "client_stopping",
            "client_stopped",
            "shutting_down client",
        ]
    );
}

/// `get_arm_pose`'s answer is the pose of that hand's stack; a hand it does not answer for,
/// or no listener at all, leaves the item's own (declared) pose, which a vanilla stick lacks.
#[test]
#[ignore = "daily"]
fn an_arm_pose_answer_replaces_the_pose_the_item_gives() {
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use mod_api::{ArmPose, ClientHost, GetArmPoseEvent, Hand, Hooks, ModEntry};
    use mod_platform::testing::with_mods;

    struct Horn;
    #[mod_api::__subscriptions]
    impl Hooks for Horn {
        fn get_arm_pose(
            &self,
            _: &dyn ClientHost,
            event: &mut GetArmPoseEvent<'_>,
        ) -> Option<ArmPose> {
            (event.hand == Hand::Main).then_some(ArmPose::TootHorn)
        }
    }

    let stick = sunrise_items::items::ItemStack::new("minecraft:stick", 1);
    with_client_game(|game| {
        let mut m = sunrise_sim::mobs::Mob::new(
            1,
            game.client_res().player_type().clone().unwrap(),
            sunrise_core::geometry::Vec3::ZERO,
            0.0,
        );
        for slot in [
            sunrise_items::equipment::MAINHAND,
            sunrise_items::equipment::OFFHAND,
        ] {
            m.extra.set_slot_stack(slot, Some(stick.clone()));
        }
        assert_eq!(game.held_arm_poses(1, &m), [None, None]);
        let mods = vec![ModEntry::new("horn", Box::new(Horn))].into();
        with_mods(mods, || {
            assert_eq!(
                game.held_arm_poses(1, &m),
                [Some(sunrise_core::arm_pose::ArmPose::TootHorn), None]
            );
        });
    });
}

#[test]
fn resolved_ui_resources_do_not_restore_a_filtered_panorama_face() {
    use sunrise_assets::resource_source::{Resource, ResourceSource};
    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let scratch = Scratch(std::env::temp_dir().join(format!(
            "sunrise-ui-filter-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )));
    let folder = scratch
        .0
        .join("assets/minecraft/textures/gui/title/background");
    std::fs::create_dir_all(&folder).unwrap();
    let base = ResourceSource::directory(scratch.0.clone());
    let mut files = std::collections::BTreeMap::new();
    for face in 0..6 {
        let name = format!("assets/minecraft/textures/gui/title/background/panorama_{face}.png");
        let mut png =
            png::Encoder::new(std::fs::File::create(scratch.0.join(&name)).unwrap(), 1, 1);
        png.set_color(png::ColorType::Rgba);
        png.set_depth(png::BitDepth::Eight);
        png.write_header()
            .unwrap()
            .write_image_data(&[200, 30, 40, 255])
            .unwrap();
        // The resolved selection hides face 1 even though the base directory has it.
        if face != 1 {
            files.insert(name.clone(), Resource::file(base.clone(), name));
        }
    }
    assert!(base.contains("assets/minecraft/textures/gui/title/background/panorama_1.png"));
    let gui = sunrise_client_gameplay::game::UiResources::load_with_limit(
        &ResourceSource::indexed(files),
        &scratch.0,
        sunrise_level::pack_image::MAX_ATLAS_SIZE,
    )
    .unwrap();
    assert_eq!(
        gui.panorama.texels().unwrap(),
        [96, 110, 111, 255].repeat(6)
    );
}

#[test]
#[ignore = "daily"]
fn refused_client_context_keeps_pack_and_listeners() {
    use mod_api::__private::reload::PreparedStep;
    use std::sync::atomic::{AtomicBool, Ordering};
    use {
        crate::game::server_side::native_entity_refs::tests::with_uncached_client_game,
        sunrise_client_gameplay::game::packs::stage_pack, sunrise_gameplay_common::game::Progress,
    };

    with_uncached_client_game(|game| {
        let mut staged = stage_pack(
            &game.vanilla_root(),
            &[],
            &[],
            &game.mods,
            game.register_client_reload_listeners(
                &game
                    .capture_candidate_settings(&game.options.resources)
                    .unwrap(),
            ),
            &[],
            &game.pack.assets,
            &game.res,
            game.client_res(),
            &game.client.client_pack.context,
            game.client.settings.force_unicode,
            game.client.settings.japanese_glyphs,
            &Progress::new(),
        )
        .unwrap();
        let root =
            std::env::temp_dir().join(format!("client-reload-refusal-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join(sunrise_mod_package::enabled::MARKER),
            b"invalid json",
        )
        .unwrap();
        staged.root = root.clone();
        let applied = Arc::new(AtomicBool::new(false));
        let flag = applied.clone();
        staged.client_applies.push(PreparedStep::Mod {
            apply: Box::new(move || {
                flag.store(true, Ordering::SeqCst);
            }),
            owner: None,
            id: id("test:listener"),
        });
        let old_root = game.options.resources.clone();
        let old_pack = game.pack.source.clone();
        let old_context = game.loop_context.clone();
        assert!(game.apply_staged_pack(staged).is_err());
        assert_eq!(game.options.resources, old_root);
        assert!(game.pack.source.same_generation(&old_pack));
        assert!(Arc::ptr_eq(&game.loop_context, &old_context));
        assert!(!applied.load(Ordering::SeqCst));
        std::fs::remove_dir_all(root).unwrap();
    });
}

#[test]
fn startup_client_reload_listener_context_reports_known_facts_and_no_published_assets() {
    use mod_api::{
        AddClientReloadListenersEvent, ClientReloadListenerContext, Dist, Hooks, ModEntry, Text,
    };
    use std::sync::Mutex;

    struct StartupObservation {
        dist: Dist,
        data_version: i32,
        math_profile: mod_api::MathProfile,
        enabled_self: bool,
        disabled_other: bool,
        plain: String,
        has_published: bool,
    }
    struct Probe(Arc<Mutex<Option<StartupObservation>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Probe {
        fn add_client_reload_listeners(
            &self,
            context: &dyn ClientReloadListenerContext,
            _: &mut AddClientReloadListenersEvent<'_>,
        ) {
            *self.0.lock().unwrap() = Some(StartupObservation {
                dist: context.dist(),
                data_version: context.server_data_version(),
                math_profile: context.math_profile(),
                enabled_self: context.on("startup_probe"),
                disabled_other: context.on("muted_mod"),
                plain: context.text_string(&Text::literal("hello startup")),
                has_published: context.published().is_some(),
            });
        }
    }
    let observed = Arc::new(Mutex::new(None));
    let mut settings = sunrise_client::settings::Settings::default();
    settings.mods.disabled.insert("muted_mod".into());
    let mods = vec![ModEntry::new(
        "startup_probe",
        Box::new(Probe(observed.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        let capture = sunrise_world::native_load_context::SettingsCapture::new(
            std::path::Path::new(""),
            [],
            [],
        );
        let _ = register_startup_client_reload_listeners(Some(&settings), &capture);
    });
    let seen = observed.lock().unwrap().take().expect("startup hook fired");
    assert_eq!(seen.dist, Dist::Client);
    assert_eq!(seen.data_version, sunrise_core::version::DATA_VERSION);
    assert_eq!(seen.math_profile, crate::game::startup_math_profile());
    assert!(seen.enabled_self);
    assert!(!seen.disabled_other);
    assert_eq!(seen.plain, "hello startup");
    assert!(!seen.has_published);
}

#[test]
#[ignore = "daily"]
fn client_completion_reads_published_resources_without_world_access() {
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use mod_api::{
        AddClientReloadListenersEvent, ClientReloadHost, ClientReloadListenerContext,
        ClientResourcesReloadedEvent, Hooks, ModEntry,
    };
    use std::sync::Mutex;

    struct Probe(Arc<Mutex<Vec<u64>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Probe {
        fn add_client_reload_listeners(
            &self,
            context: &dyn ClientReloadListenerContext,
            _: &mut AddClientReloadListenersEvent<'_>,
        ) {
            let published = context
                .published()
                .expect("later reloads expose published client assets");
            assert!(
                published.block_sprites(&[published.block_state_id("minecraft:stone").unwrap()])[0]
                    .is_some()
            );
            self.0
                .lock()
                .unwrap()
                .push(published.block_sprites_version());
        }
        fn client_resources_reloaded(
            &self,
            context: &dyn ClientReloadHost,
            _: &mut ClientResourcesReloadedEvent,
        ) {
            assert!(
                context.block_sprites(&[context.block_state_id("minecraft:stone").unwrap()])[0]
                    .is_some()
            );
            assert!(context.texture_sprites(&["minecraft:block/stone"])[0].is_some());
            let version = context.block_sprites_version();
            self.0.lock().unwrap().push(version);
        }
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new(
        "published_client",
        Box::new(Probe(seen.clone())),
    )]
    .into();
    // Build the client first: this probe only expects the later registration
    // context, not the startup context that has no published assets yet.
    with_client_game(|game| {
        mod_platform::testing::with_mods(mods, || {
            let version = game.block_sprites_version();
            let _ = game.register_client_reload_listeners(
                &game
                    .capture_candidate_settings(&game.options.resources)
                    .unwrap(),
            );
            assert_eq!(*seen.lock().unwrap(), [version]);
            game.native_client_resources_reloaded();
            game.native_client_resources_reloaded();
            assert_eq!(*seen.lock().unwrap(), [version, version, version]);
        });
    });
}

#[test]
#[ignore = "daily"]
fn client_startup_finishes_configs_before_the_first_runtime_callback() {
    // Startup holds two Game values, which exceed libtest's default thread stack.
    std::thread::Builder::new()
        .name("client-startup-fixture".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            use {sunrise_client_lib::audio::Audio, sunrise_client_gameplay::game::UiResources, crate::game::server_side::native_entity_refs::tests::with_client_game, sunrise_client_gameplay::game::client_side::loading_window::start_client_game};
            use mod_api::{
                ClientHost, ClientSetupContext, ClientSetupEvent, ClientTickPreEvent, ConfigEvent,
                ConfigKind, ConfigListener, ConfigSnapshot, ConfigSpec, Hooks, ModEntry, Mods,
                Registrar, RegistrationContext, ScreenOpening, ScreenOpeningEvent,
            };
            use std::{sync::Mutex, time::Instant};

            struct Probe(Arc<Mutex<Vec<String>>>, std::path::PathBuf);
            #[mod_api::__subscriptions]
            impl Hooks for Probe {
                fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                    for kind in [ConfigKind::Client, ConfigKind::Common] {
                        let seen = self.0.clone();
                        let listener: Arc<ConfigListener> =
                            Arc::new(move |event: ConfigEvent<'_>, config: &ConfigSnapshot| {
                                if let ConfigEvent::Loading(context) = event {
                                    assert!(context.config_dir().ends_with("config"));
                                    seen.lock().unwrap().push(format!("{:?}", config.kind));
                                }
                                Ok(())
                            });
                        r.register_config(
                            ConfigSpec::new(kind, "enabled = true\n")
                                .file(self.1.join(format!("{kind:?}.toml"))),
                            Some(listener),
                        )
                        .unwrap();
                    }
                }
                fn client_setup(&self, _: &mut dyn ClientSetupContext, _: &mut ClientSetupEvent) {
                    self.0.lock().unwrap().push("setup".into());
                }
                fn screen_opening(
                    &self,
                    host: &dyn ClientHost,
                    _: &mut ScreenOpeningEvent<'_>,
                ) -> Option<ScreenOpening> {
                    assert!(host.level("minecraft:overworld").is_none());
                    self.0.lock().unwrap().push("screen".into());
                    None
                }
                fn client_tick_pre(&self, host: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {
                    assert!(host.level("minecraft:overworld").is_none());
                    assert!(host.entity(1).is_none());
                    self.0.lock().unwrap().push("tick".into());
                }
            }
            let seen = Arc::new(Mutex::new(Vec::new()));
            let temp = tempfile::tempdir().unwrap();
            let mods: Mods = vec![ModEntry::new(
                "startup_phase",
                Box::new(Probe(seen.clone(), temp.path().to_owned())),
            )]
            .into();
            mod_platform::testing::with_mods(mods.clone(), || {
                mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
                for pack_error in [None, Some("selected resource pack failed")] {
                    seen.lock().unwrap().clear();
                    with_client_game(|game| {
                        let resources = game.res.clone();
                        let mut options = game.options.clone();
                        options.headless = true;
                        let root = options.resources.clone();
                        let gui = UiResources::fonts_only(
                            &sunrise_assets::resource_source::ResourceSource::directory(
                                root.clone(),
                            ),
                        )
                        .unwrap();
                        let mut started = start_client_game(
                            &resources,
                            std::sync::Arc::clone(&game.client.client_res),
                            game.pack.clone(),
                            game.client.client_pack.clone(),
                            gui,
                            options,
                            pack_error.map(str::to_owned),
                            Audio::silent(
                                &sunrise_assets::resource_source::ResourceSource::directory(
                                    root.clone(),
                                ),
                            ),
                            root,
                            Instant::now(),
                            None,
                            game.client.settings.clone(),
                            None,
                            vec![PreparedClientReload::Vanilla(())],
                            None,
                            None,
                            sunrise_gameplay::game::server_side::integrated::serve,
                            sunrise_gameplay::game::server_side::integrated::run_component_harness_on_server,
                            crate::game::session_verifier(),
                            crate::game::profile_resolver,
                        )
                        .unwrap()
                        .unwrap();
                        // The event loop dispatches this before any per-tick work,
                        // even on the title screen, without a world or renderer.
                        started.native_client_tick_pre();
                        assert_eq!(
                            started.client.chat_ui.options.resource_error.as_deref(),
                            pack_error
                        );
                        let mut expected = vec!["setup", "Client", "Common", "screen"];
                        if pack_error.is_some() {
                            assert!(started.client.chat_ui.options.resource_packs_open());
                            expected.push("screen");
                        }
                        expected.push("tick");
                        assert_eq!(*seen.lock().unwrap(), expected);
                    })
                }
            });
        })
        .unwrap()
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

/// The existing item hook sees borrowed mob extraction data, and its answer reaches
/// the animated model. Other entities, both player paths, and a later no-listener
/// extraction retain their normal poses.
#[test]
#[ignore = "daily"]
fn humanoid_arm_pose_mod_reads_mob_state_and_changes_the_model() {
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use mod_api::{ArmPose, ClientHost, GetArmPoseEvent, Hand, Hooks, ModEntry};
    use mod_platform::testing::with_mods;
    use std::{
        collections::HashMap,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };
    use sunrise_core::{geometry::Vec3, use_animation::UseAnimation};
    use sunrise_items::equipment::{MAINHAND, OFFHAND};
    use sunrise_items::items::ItemStack;
    use sunrise_sim::{
        mobs::{
            Mob,
            anim::{self, Poses},
        },
        motion::PlayerMotion,
    };

    struct MobPose(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for MobPose {
        fn get_arm_pose(
            &self,
            _: &dyn ClientHost,
            event: &mut GetArmPoseEvent<'_>,
        ) -> Option<ArmPose> {
            assert_ne!(
                event.entity_id, 62,
                "a quadruped does not extract humanoid poses"
            );
            if event.entity_id == 7 {
                assert_eq!(event.humanoid.off_hand.unwrap().damage(), 9);
                assert_eq!(event.humanoid.off_hand.unwrap().count(), 3);
                assert_eq!(event.humanoid.main_arm_pose, ArmPose::Item);
                assert_eq!(event.humanoid.off_arm_pose, ArmPose::Item);
            }
            if event.entity_id != 60 {
                return None;
            }
            let state = event.humanoid;
            assert!(state.main_arm_left);
            assert_eq!(state.use_hand, Some(Hand::Off));
            assert_eq!(state.ticks_using_item, 12);
            assert_eq!(state.use_animation, UseAnimation::Brush);
            assert_eq!(state.main_arm_pose, ArmPose::Empty);
            assert_eq!(state.off_arm_pose, ArmPose::Empty);
            let main = state.main_hand.unwrap();
            assert_eq!(
                (main.item(), main.count()),
                (mod_api::__private::item_key("minecraft:stick").unwrap(), 2)
            );
            let off = state.off_hand.unwrap();
            assert_eq!(
                (off.item(), off.count(), off.damage()),
                (
                    mod_api::__private::item_key("minecraft:brush").unwrap(),
                    3,
                    9
                )
            );
            assert!(std::ptr::eq(
                event.stack,
                if event.hand == Hand::Main { main } else { off }
            ));
            self.0.fetch_add(1, Ordering::Relaxed);
            (event.hand == Hand::Main).then_some(ArmPose::ThrowTrident)
        }
    }
    fn arms(m: &Mob, models: &sunrise_client::mobs_render::ClientMobModels) -> [[f32; 9]; 2] {
        let mut poses = Poses::new(models.model(&m.kind.id).unwrap());
        anim::setup(m, &anim::state(m, 0.0), &mut poses, &HashMap::new());
        [*poses.get("right_arm"), *poses.get("left_arm")]
    }
    with_client_game(|game| {
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        game.player.as_mut().unwrap().entity_id = 7;
        game.common.inventory[game.common.selected] = Some(ItemStack::new("minecraft:stick", 1));
        game.offhand = Some(ItemStack::new("minecraft:brush", 3));
        game.offhand.as_mut().unwrap().damage = 9;
        game.tick_player_model();
        let own_before =
            anim::humanoid_arm_poses(game.client.player_model.as_ref().unwrap(), [None; 2], 0.0);
        assert_eq!(own_before, [sunrise_core::arm_pose::ArmPose::Item; 2]);
        for (id, kind) in [
            (60, "minecraft:zombie"),
            (61, "minecraft:skeleton"),
            (62, "minecraft:pig"),
        ] {
            let mut m = Mob::new(
                id,
                game.res.mob_types().get(kind).unwrap().clone(),
                Vec3::ZERO,
                0.0,
            );
            m.extra
                .set_slot_stack(MAINHAND, Some(ItemStack::new("minecraft:stick", 2)));
            m.extra
                .set_slot_stack(OFFHAND, Some(ItemStack::new("minecraft:brush", 3)));
            game.mobs.push(m);
        }
        let mut player = Mob::new(
            8,
            game.client_res().player_type().clone().unwrap(),
            Vec3::ZERO,
            0.0,
        );
        player
            .extra
            .set_slot_stack(MAINHAND, Some(ItemStack::new("minecraft:stick", 1)));
        game.mobs.push(player);
        let m = &mut game.mobs[0];
        let mut brush = ItemStack::new("minecraft:brush", 3);
        brush.damage = 9;
        m.extra.set_slot_stack(OFFHAND, Some(brush));
        m.extra.main_hand_left = true;
        m.extra.using_item = 12;
        m.extra.using_off_hand = true;
        let baseline: Vec<_> = {
            let models = game.client_res().client_mob_models();
            game.mobs.iter().map(|m| arms(m, models)).collect()
        };
        game.extract_remote_arm_poses();
        {
            let models = game.client_res().client_mob_models();
            assert_eq!(
                game.mobs
                    .iter()
                    .map(|m| arms(m, models))
                    .collect::<Vec<_>>(),
                baseline
            );
        }
        let calls = Arc::new(AtomicUsize::new(0));
        with_mods(
            vec![ModEntry::new("mob-pose", Box::new(MobPose(calls.clone())))].into(),
            || {
                game.extract_remote_arm_poses();
                assert_eq!(
                    game.mobs[0].extra.held_arm_pose,
                    [Some(sunrise_core::arm_pose::ArmPose::ThrowTrident), None]
                );
                {
                    let models = game.client_res().client_mob_models();
                    let posed = arms(&game.mobs[0], models);
                    assert!((posed[1][3] + std::f32::consts::PI).abs() < 1e-6);
                    assert_ne!(posed, baseline[0]);
                    for (m, expected) in game.mobs[1..].iter().zip(&baseline[1..]) {
                        assert_eq!(arms(m, models), *expected);
                    }
                }
                let empty = Mob::new(60, game.mobs[0].kind.clone(), Vec3::ZERO, 0.0);
                assert_eq!(game.held_arm_poses(60, &empty), [None; 2]);
                // The local player's extraction also exposes its actual off-hand stack.
                game.extract_player_arm_pose();
                let own = game.client.player_model.as_ref().unwrap();
                assert_eq!(
                    anim::humanoid_arm_poses(own, own.extra.held_arm_pose, 0.0),
                    own_before
                );
            },
        );
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        game.extract_remote_arm_poses();
        let models = game.client_res().client_mob_models();
        assert_eq!(
            game.mobs
                .iter()
                .map(|m| arms(m, models))
                .collect::<Vec<_>>(),
            baseline
        );
    });
}

#[path = "custom_arm_pose_tests.rs"]
mod custom_arm_pose_tests;

use sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycleCommonGameExt1 as _;
