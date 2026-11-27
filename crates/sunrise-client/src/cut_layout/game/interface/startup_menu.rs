#![cfg(test)]
use std::path::PathBuf;
use std::sync::atomic::AtomicU32;
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;
use sunrise_client::hud::HudBatch;
use sunrise_client::options_ui::Event;
use sunrise_client::options_ui::Page;
use sunrise_client::settings::Settings;
use sunrise_client_gameplay::game::client_side::event_loop::EventLoopCtl;
use sunrise_client_gameplay::game::client_side::loading_screen::Loading;
use sunrise_client_gameplay::game::client_side::resource_loader::LoaderMsg;
use sunrise_client_gameplay::game::client_side::resource_loader::PendingWorldEntry;
use sunrise_client_gameplay::game::client_side::startup_menu::StartupMenu;
use sunrise_gameplay_common::game::launch_options::Options;
use sunrise_gameplay_common::game::resource_load::Progress;
use winit::event::WindowEvent;
#[cfg(test)]
pub(crate) mod tests {

    use std::sync::Arc;
    use sunrise_client::hud::HudAtlas;
    use sunrise_client_gameplay::game::client_side::resource_tables_client::UiResources;

    pub fn gui(root: &std::path::Path) -> UiResources {
        let atlas = if root.exists() {
            HudAtlas::fonts(&sunrise_assets::resource_source::ResourceSource::directory(
                root.to_path_buf(),
            ))
            .unwrap()
        } else {
            HudAtlas::boot()
        };
        UiResources {
            atlas,
            panorama: Arc::new(sunrise_client::gfx::panorama::Faces::colour([
                96, 110, 111, 255,
            ])),
            splashes: Arc::from(""),
            plain_texts: Default::default(),
        }
    }
    pub fn saved_pack_options() -> sunrise_client::options_file::OptionsFile {
        sunrise_client::options_file::OptionsFile::parse(
            &std::fs::read_to_string(crate::game::options_path()).unwrap(),
        )
    }

    use super::*;
    use sunrise_client_gameplay::game::packs;
    use sunrise_gameplay_common::game::mods;

    fn loading(receiver: Receiver<LoaderMsg>) -> Loading {
        Loading {
            serve_integrated: sunrise_gameplay::game::server_side::integrated::serve,
            run_component_harness:
                sunrise_gameplay::game::server_side::integrated::run_component_harness_on_server,
            profile_resolver: crate::game::profile_resolver,
            session_verifier: crate::game::session_verifier(),
            window: None,
            renderer: None,
            max_atlas_size: Arc::new(AtomicU32::new(sunrise_client::hud::MAX_ATLAS_SIZE)),
            boot: sunrise_client::hud::HudAtlas::boot(),
            gui: None,
            settings: Settings::default(),
            fullscreen: sunrise_client::window::FullscreenState::new(false),
            video_modes: Default::default(),
            options: None,
            vanilla_root: PathBuf::new(),
            pack_error: None,
            started: Instant::now(),
            panorama_rotation: Default::default(),
            progress: Progress::new(),
            bar: 0.0,
            receiver,
            loaded: None,
            delivered: false,
            handle: None,
            shown: false,
            ui_at: None,
            title_drawn: false,
            menu: StartupMenu::default(),
            error: None,
        }
    }
    #[test]
    #[ignore = "daily"]
    fn startup_menu_creation_carries_installed_mod_rules() {
        use mod_api::{Hooks, Registrar, RegistrationContext};
        use sunrise_client::title::Action;
        use sunrise_game_data::gamerules::Value;
        struct Mod;
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
                registrar.game_rule_category("machines").unwrap();
                registrar
                    .game_rule_int("cap", "example:machines", 8, 0, 64)
                    .unwrap();
            }
        }
        // `set_root` replaces the process's language: keep it in a child process.
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::client_side::startup_menu::tests::startup_menu_creation_carries_installed_mod_rules",
            "SUNRISE_STARTUP_MOD_RULES_CASE",
            &["rules"],
        ) {
            return;
        }
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("example", Box::new(Mod))].into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_client_game(|_| {
                let scratch = tempfile::tempdir().unwrap();
                let mut menu = StartupMenu::default();
                menu.set_root(
                    scratch.path().to_path_buf(),
                    &Settings::default(),
                    Arc::from([]),
                    "",
                );
                let title = menu.title.as_mut().unwrap();
                title.enter_singleplayer();
                let Action::Create { mut options, .. } = title.create_action() else {
                    panic!("creation")
                };
                // The page lists `available_rules`: the mod's integer rule is one of them.
                assert!(options.rules.rule("example:cap").is_some());
                assert_eq!(options.rules.get("example:cap"), Some(Value::Int(8)));
                assert!(options.rules.set("example:cap", Value::Int(12)));
                assert_eq!(options.rules.get("example:cap"), Some(Value::Int(12)));
            })
        });
    }

    #[test]
    #[ignore = "daily"]
    fn queued_creation_is_reconciled_at_handoff() {
        use mod_api::{Hooks, Registrar, RegistrationContext};
        use sunrise_client::title::Action;
        use sunrise_game_data::gamerules::Value;
        struct Mod;
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
                registrar.game_rule_category("machines").unwrap();
                registrar
                    .game_rule_bool("enabled", "example:machines", true)
                    .unwrap();
            }
        }
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("example", Box::new(Mod))].into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_client_game(|game| {
                let saves = std::env::temp_dir()
                    .join(format!("sunrise-startup-rules-{}", std::process::id()));
                let _ = std::fs::remove_dir_all(&saves);
                let mut title = game.new_title(saves);
                title.enter_singleplayer();
                let Action::Create { mut options, .. } = title.create_action() else {
                    panic!("creation")
                };
                options.rules.set("keep_inventory", Value::Bool(true));
                let mut menu = StartupMenu::default();
                menu.title = Some(title);
                menu.pending = Some(PendingWorldEntry::Local {
                    folder: "test".into(),
                    name: None,
                    seed: Some(1),
                    creative: true,
                    create: Some(Box::new(options)),
                });
                menu.handoff(game);
                let Some(PendingWorldEntry::Local {
                    create: Some(options),
                    ..
                }) = &game.client.pending_entry.0
                else {
                    panic!("pending")
                };
                assert_eq!(
                    options.rules.get("example:enabled"),
                    Some(Value::Bool(true))
                );
                assert_eq!(options.rules.get("keep_inventory"), Some(Value::Bool(true)));
                let request =
                    sunrise_gameplay::game::server_side::world_loading::WorldRequest::new(
                        "test",
                        None,
                        None,
                        true,
                        Some(options.as_ref()),
                        options.rules.registry().clone(),
                    )
                    .unwrap();
                assert!(Arc::ptr_eq(
                    &request.rule_registry,
                    options.rules.registry()
                ));
            })
        });
    }

    #[test]
    #[ignore = "daily"]
    fn live_gui_scale_uses_the_current_owner() {
        let (_, receiver) = std::sync::mpsc::channel();
        let mut first = loading(receiver);
        let (_, receiver) = std::sync::mpsc::channel();
        let mut second = loading(receiver);
        first.settings.gui_scale = 3;
        second.settings.gui_scale = 3;
        second.settings.force_unicode = true;
        let view = |owner: &Loading| {
            let batch = HudBatch::new([960, 720], owner.settings.gui_scale_inputs());
            (batch.scale, batch.width, batch.height)
        };
        assert_eq!(view(&first), (3.0, 320.0, 240.0));
        assert_eq!(view(&second), (4.0, 240.0, 180.0));
        super::super::native_entity_refs::tests::with_client_game(|server| {
            server.client.settings.gui_scale = 1;
            server.apply_ui_settings();
            assert_eq!(view(&first), (3.0, 320.0, 240.0));
            assert_eq!(view(&second), (4.0, 240.0, 180.0));
        });
        first.settings.force_unicode = true;
        first.menu.pointer = first
            .menu_pointer_projection()
            .window
            .physical_sample(winit::dpi::PhysicalPosition::new(400.0, 200.0));
        first.apply_menu_settings();
        assert_eq!(view(&first), (4.0, 240.0, 180.0));
        assert_eq!(first.menu.cursor, (100.0, 50.0));
    }
    #[test]
    #[ignore = "daily"]
    fn resource_pack_drops_import_from_startup_and_game_options() {
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::client_side::startup_menu::tests::resource_pack_drops_import_from_startup_and_game_options",
            "SUNRISE_RESOURCE_PACK_DROP_CHILD",
            &["import"],
        ) {
            return;
        }
        // Only this exact test runs in the child. Keep the game directory empty,
        // separate from the isolated homes created by run_fresh_cases.
        let previous_dir = std::env::current_dir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        std::env::set_current_dir(scratch.path()).unwrap();
        struct Loop;
        impl EventLoopCtl for Loop {
            fn exit(&self) {}
            fn set_control_flow(&self, _: winit::event_loop::ControlFlow) {}
        }
        let root =
            PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").expect("prepared resources"));
        let game_dir = std::env::current_dir().unwrap();
        assert!(
            std::fs::read_dir(&game_dir).unwrap().next().is_none(),
            "run the built test executable directly from an empty scratch game directory"
        );
        let source = game_dir.join("drop-fixtures");
        std::fs::create_dir(&source).unwrap();
        for name in ["Startup pack", "Game pack"] {
            let path = source.join(name);
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(
                path.join("pack.mcmeta"),
                br#"{"pack":{"pack_format":1,"description":"Drop test"}}"#,
            )
            .unwrap();
        }
        let (_sender, receiver) = std::sync::mpsc::channel();
        let mut loading = loading(receiver);
        loading.gui = Some(gui(&root));
        loading.vanilla_root = root.clone();
        loading.open_menu_options(Page::Packs);
        loading
            .menu_event(&Loop, WindowEvent::DroppedFile(source.join("Startup pack")))
            .unwrap();
        assert!(
            loading
                .menu
                .options
                .pack_screen
                .as_ref()
                .unwrap()
                .confirming()
        );
        assert!(!packs::packs_dir().join("Startup pack").exists());
        let batch = HudBatch::new(loading.menu_size(), loading.settings.gui_scale_inputs());
        let events = loading.menu.options.click(
            &mut loading.settings,
            &loading.gui.as_ref().unwrap().atlas,
            batch.width / 2.0 - 150.0,
            batch.height / 2.0 + 20.0,
            (batch.width, batch.height),
            [1280, 720],
            false,
            false,
        );
        assert!(events.iter().any(|e| matches!(e, Event::ImportPacks(_))));
        loading.menu_option_events(events);
        assert!(
            packs::packs_dir()
                .join("Startup pack/pack.mcmeta")
                .is_file()
        );
        assert!(
            loading
                .menu
                .options
                .pack_infos
                .iter()
                .any(|p| p.id == "file/Startup pack")
        );
        assert!(loading.settings.resource_packs.is_empty());

        super::super::native_entity_refs::tests::with_client_game(|game| {
            game.client.title.0 = Some(game.new_title(PathBuf::from("saves")));
            game.open_options_at(true, Page::Packs);
            game.handle_window_event_inner(
                &Loop,
                WindowEvent::DroppedFile(source.join("Game pack")),
                None,
                None,
            );
            assert!(
                game.client
                    .chat_ui
                    .options
                    .pack_screen
                    .as_ref()
                    .unwrap()
                    .confirming()
            );
            assert!(!packs::packs_dir().join("Game pack").exists());
            let size = game.gui_size();
            let events = game.client.chat_ui.options.click(
                &mut game.client.settings,
                &game.client.gui.atlas,
                size.0 / 2.0 - 150.0,
                size.1 / 2.0 + 20.0,
                size,
                [1280, 720],
                false,
                false,
            );
            assert!(events.iter().any(|e| matches!(e, Event::ImportPacks(_))));
            game.handle_option_events(events).unwrap();
            assert!(packs::packs_dir().join("Game pack/pack.mcmeta").is_file());
            assert!(
                game.client
                    .chat_ui
                    .options
                    .pack_infos
                    .iter()
                    .any(|p| p.id == "file/Game pack")
            );
            assert!(game.client.settings.resource_packs.is_empty());
        });
        std::env::set_current_dir(previous_dir).unwrap();
    }
    #[test]
    #[ignore = "daily"]
    fn startup_pack_recovery_only_clears_the_failed_selection_revision() {
        use sunrise_client::options_file::V;

        const CASE: &str = "SUNRISE_STARTUP_PACK_RECOVERY_REVISION_CASE";
        const OBSOLETE: &str = "file/old";
        const REPLACEMENT: &str = "file/replacement";
        const ASSET: &str = "assets/fixture/texts/replacement.txt";
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::client_side::startup_menu::tests::startup_pack_recovery_only_clears_the_failed_selection_revision",
            CASE,
            &["current", "superseded"],
        ) {
            return;
        }
        let superseded = std::env::var(CASE).unwrap() == "superseded";
        let (send, recv) = std::sync::mpsc::channel();
        let mut loading = loading(recv);
        loading.settings.resource_packs = vec![OBSOLETE.into()];
        loading
            .settings
            .file
            .set("incompatibleResourcePacks", V::List(vec![OBSOLETE.into()]));
        loading
            .settings
            .save(&sunrise_gameplay_common::game::options_path());
        let revision = loading.menu.pack_selection_revision;
        if superseded {
            // OptionsUi applies the new selection before emitting Event::Packs.
            loading.settings.resource_packs = vec![REPLACEMENT.into()];
            loading.settings.file.set(
                "incompatibleResourcePacks",
                V::List(vec![REPLACEMENT.into()]),
            );
            loading.menu_option_events(vec![Event::Packs(vec![REPLACEMENT.into()])]);
        }
        // Drive the graphical handshake before delivering the worker's later
        // error from the required-only rebuild.
        let (ack, acknowledged) = std::sync::mpsc::channel();
        send.send(LoaderMsg::ClearPackSelection { revision, ack })
            .unwrap();
        loading.poll_menu_resources().unwrap();
        acknowledged.try_recv().unwrap();
        assert!(loading.loaded.is_none());
        let expected: &[&str] = if superseded { &[REPLACEMENT] } else { &[] };
        assert_eq!(loading.settings.resource_packs.as_slice(), expected);
        let options = saved_pack_options();
        assert_eq!(options.list("resourcePacks").as_slice(), expected);
        assert_eq!(
            options.list("incompatibleResourcePacks").as_slice(),
            expected
        );

        send.send(LoaderMsg::done(Err("required pack failed".into())))
            .unwrap();
        if !superseded {
            loading.poll_menu_resources().unwrap();
            assert!(matches!(loading.take_worker_result(), Some(Err(error))
                if error.to_string() == "required pack failed"));
            assert!(saved_pack_options().list("resourcePacks").is_empty());
            return;
        }

        // Run the queued generation with the replacement, in a private prepared
        // folder: the world tail may write its prepared-data cache.
        fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
            std::fs::create_dir_all(to).unwrap();
            for entry in std::fs::read_dir(from).unwrap() {
                let entry = entry.unwrap();
                let target = to.join(entry.file_name());
                if entry.path().is_dir() {
                    copy_tree(&entry.path(), &target);
                } else {
                    std::fs::copy(entry.path(), target).unwrap();
                }
            }
        }
        let vanilla = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let scratch = tempfile::tempdir().unwrap();
        copy_tree(&vanilla, scratch.path());
        let pack = packs::packs_dir().join("replacement");
        std::fs::create_dir_all(pack.join(ASSET).parent().unwrap()).unwrap();
        std::fs::write(
            pack.join("pack.mcmeta"),
            br#"{"pack":{"description":"Replacement","min_format":[97,0],"max_format":[97,1]}}"#,
        )
        .unwrap();
        std::fs::write(pack.join(ASSET), b"replacement was loaded").unwrap();
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = scratch.path().to_path_buf();
        loading.vanilla_root = options.resources.clone();
        loading.options = Some(options);
        loading.poll_menu_resources().unwrap();
        assert!(loading.loaded.is_none(), "discard the failed generation");
        assert!(!loading.menu.reload_packs, "start the queued generation");
        let mut loaded = loop {
            match loading.receiver.recv().unwrap() {
                LoaderMsg::Ui(..) => {}
                LoaderMsg::Done(result) => {
                    break result.map_err(Box::<dyn std::error::Error>::from).unwrap();
                }
                LoaderMsg::ClearPackSelection { .. } => {
                    panic!("replacement unexpectedly failed to load")
                }
            }
        };
        loading.handle.take().unwrap().join().unwrap();
        loaded.rest_loader.take().unwrap().finish().unwrap();
        assert_eq!(
            loaded.pack.source.read(ASSET).unwrap(),
            b"replacement was loaded"
        );
        assert_eq!(saved_pack_options().list("resourcePacks"), [REPLACEMENT]);
    }
    #[test]
    #[ignore = "daily"]
    fn startup_client_pack_recovery_preserves_pending_world_pack_edits() {
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::client_side::startup_menu::tests::startup_client_pack_recovery_preserves_pending_world_pack_edits",
            "SUNRISE_STARTUP_PACK_RECOVERY_WORLD_EDITOR_CASE",
            &["data-packs"],
        ) {
            return;
        }
        let (send, recv) = std::sync::mpsc::channel();
        let mut loading = loading(recv);
        loading.settings.resource_packs = vec!["file/client".into()];
        let dir = tempfile::tempdir().unwrap();
        let data_pack = dir.path().join("example");
        std::fs::create_dir(&data_pack).unwrap();
        std::fs::write(
            data_pack.join("pack.mcmeta"),
            br#"{"pack":{"description":"World pack","min_format":[121,0],"max_format":[121,0]}}"#,
        )
        .unwrap();
        loading.menu.options_open = true;
        let ui = &mut loading.menu.options;
        ui.open_data_packs(
            dir.path().to_path_buf(),
            vec!["vanilla".into()],
            loading.vanilla_root.clone(),
            Vec::new(),
        );
        let mut screen = sunrise_client::pack_screen::PackScreen::new(
            &sunrise_client::screen_draw::Measure(&loading.boot.font),
            sunrise_game_data::text::Text::translate("dataPack.title", &[]),
            480,
            270,
            false,
            ui.pack_infos.clone(),
            &ui.pack_selection,
        );
        let example = screen
            .model
            .unselected
            .iter()
            .position(|pack| pack.id == "file/example")
            .unwrap();
        assert!(screen.model.activate(false, example, true));
        screen.changed = true;
        ui.pack_screen = Some(screen);
        let (ack, acknowledged) = std::sync::mpsc::channel();
        send.send(LoaderMsg::ClearPackSelection {
            revision: loading.menu.pack_selection_revision,
            ack,
        })
        .unwrap();
        loading.poll_menu_resources().unwrap();
        acknowledged.try_recv().unwrap();
        assert!(loading.settings.resource_packs.is_empty());
        assert!(saved_pack_options().list("resourcePacks").is_empty());
        assert_eq!(loading.menu.options.pack_selection, ["vanilla"]);
        assert!(loading.menu.options_open);
        let events = loading.menu.options.back(&mut loading.settings);
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::DataPacks(ids)
            if ids.as_slice() == ["vanilla", "file/example"]))
        );
    }
    #[test]
    fn the_world_renderer_waits_for_the_first_title_frame() {
        let (_send, receiver) = std::sync::mpsc::channel();
        let mut loading = loading(receiver);
        // No menu bundle yet: nothing to wait for.
        assert!(loading.ready_for_handoff());
        // The title artwork is up but no frame of it was presented.
        loading.ui_at = Some(Instant::now());
        assert!(!loading.ready_for_handoff());
        loading.title_drawn = true;
        assert!(loading.ready_for_handoff());
        // A window never asked to redraw doesn't hold the start for long.
        loading.title_drawn = false;
        loading.ui_at = Instant::now().checked_sub(Duration::from_secs(2));
        assert!(loading.ready_for_handoff());
    }
    #[test]
    #[ignore = "daily"]
    fn menu_is_usable_while_the_core_worker_is_blocked() {
        // Publishing the empty root's language must not replace another game's language.
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::client_side::startup_menu::tests::menu_is_usable_while_the_core_worker_is_blocked",
            "SUNRISE_STARTUP_BLOCKED_WORKER_CASE",
            &["blocked"],
        ) {
            return;
        }
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path().to_path_buf();
        let (sender, receiver) = std::sync::mpsc::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let (published, published_rx) = std::sync::mpsc::channel();
        let worker_root = root.clone();
        let worker = std::thread::spawn(move || {
            sender
                .send(LoaderMsg::Ui(
                    gui(&worker_root),
                    worker_root,
                    sunrise_assets::resource_source::ResourceSource::default(),
                ))
                .unwrap();
            published.send(()).unwrap();
            wait.recv().unwrap();
            sender
                .send(LoaderMsg::done(Err("core failure".into())))
                .unwrap();
        });
        published_rx.recv().unwrap();
        let mut loading = loading(receiver);
        loading.poll_menu_resources().unwrap();
        assert!(loading.gui.is_some());
        assert!(loading.loaded.is_none());
        assert!(!worker.is_finished());
        // Navigate and enter a world name without a registry, biomes, pack
        // tables or a Game. Replacing fallback artwork preserves this state.
        let title = loading.menu.title.as_mut().unwrap();
        title.mode = sunrise_client::title::Mode::Create {
            name: String::new(),
            seed: "42".into(),
            creative: true,
            focus: 0,
            options: Default::default(),
        };
        title.type_text("My world");
        loading.set_ui(
            gui(&root),
            root,
            sunrise_assets::resource_source::ResourceSource::default(),
        );
        let action = loading.menu.title.as_ref().unwrap().create_action();
        loading.menu.queue_entry(action, "Player".into());
        assert!(
            matches!(&loading.menu.pending, Some(PendingWorldEntry::Local {
            name: Some(name), seed: Some(42), creative: true, create: Some(_), ..
        }) if name == "My world")
        );
        let mut batch = HudBatch::scratch([1280, 720]);
        loading.draw_menu(&mut batch);
        assert!(
            !batch.vertices.is_empty(),
            "queued entry draws loading progress"
        );
        release.send(()).unwrap();
        worker.join().unwrap();
        loading.poll_menu_resources().unwrap();
        assert!(
            matches!(loading.take_worker_result(), Some(Err(error)) if error.to_string() == "core failure")
        );
    }
    #[test]
    fn a_failed_generation_keeps_its_error_after_the_loader_ends() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            sender
                .send(LoaderMsg::done(Err("missing facts.json".into())))
                .unwrap();
        });
        let mut loading = loading(receiver);
        loading.handle = Some(worker);
        while loading.loaded.is_none() {
            loading.poll_menu_resources().unwrap();
        }
        // App::about_to_wait takes the result to report it; the loop may poll
        // once more before it exits, with the loader gone.
        assert!(
            matches!(loading.take_worker_result(), Some(Err(error)) if error.to_string() == "missing facts.json")
        );
        while !loading.handle.as_ref().is_none_or(JoinHandle::is_finished) {
            std::thread::yield_now();
        }
        loading.poll_menu_resources().unwrap();
        assert!(loading.loaded.is_none());
    }
    #[test]
    fn native_declaration_failures_survive_the_graphical_loader() {
        use mod_api::__private::{DeclarationKind, DeclarationRefusal};
        let report = mods::DeclarationFailures::new(vec![
            DeclarationRefusal::new(
                "maker",
                DeclarationKind::Block,
                mod_api::DeclarationError::InvalidId {
                    id: "maker:Bad Key".into(),
                },
            ),
            DeclarationRefusal::new(
                "porter",
                DeclarationKind::Item,
                mod_api::DeclarationError::Rejected {
                    id: "foreign:wrench".into(),
                    reason: "declared twice".into(),
                },
            ),
        ]);
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || sender.send(LoaderMsg::done(Err(report.into()))).unwrap())
            .join()
            .unwrap();
        let mut loading = loading(receiver);
        loading.poll_menu_resources().unwrap();
        let error = loading.take_worker_result().unwrap().err().unwrap();
        // Entry can log each refusal instead of treating the worker failure as one string.
        let report = error.downcast::<mods::DeclarationFailures>().unwrap();
        let lines = report.lines();
        assert!(matches!(lines.as_slice(), [first, second]
            if first.contains("maker:Bad Key") && second.contains("foreign:wrench")));
    }
    #[test]
    fn a_panicking_loader_reports_its_panic() {
        let (sender, receiver) = std::sync::mpsc::channel::<LoaderMsg>();
        let worker = std::thread::spawn(move || {
            let _sender = sender;
            panic!("atlas has no fonts");
        });
        while !worker.is_finished() {
            std::thread::yield_now();
        }
        let mut loading = loading(receiver);
        loading.handle = Some(worker);
        assert_eq!(
            loading.poll_menu_resources().unwrap_err().to_string(),
            "resource loader stopped before finishing: atlas has no fonts"
        );
    }
    #[test]
    fn disconnected_core_worker_is_reported_instead_of_waiting_forever() {
        let (sender, receiver) = std::sync::mpsc::channel();
        drop(sender);
        let mut loading = loading(receiver);
        assert_eq!(
            loading.poll_menu_resources().unwrap_err().to_string(),
            "resource loader stopped before finishing: no loader thread was running"
        );
    }
}
