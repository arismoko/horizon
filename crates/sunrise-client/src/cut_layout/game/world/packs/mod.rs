#![cfg(test)]
#[cfg(feature = "client")]
use crate::packs;
use std::path::Path;
use std::path::PathBuf;
#[cfg(feature = "client")]
use sunrise_client_gameplay::game::client_side::packs::{
    OVERLAY_HOLD, PackReload, ReloadKind, StageResult, reload_waits, startup_root,
};
#[cfg(feature = "client")]
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::{sync::mpsc, time::Duration};

    #[test]
    #[ignore = "daily"]
    fn fluid_models_reload_recollects_before_baking() {
        // Concurrent resource loads replace the process-wide fluid looks and models.
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::client_side::packs::tests::fluid_models_reload_recollects_before_baking",
            "SUNRISE_FLUID_RELOAD_CHILD",
            &["recollect"],
        ) {
            return;
        }
        use mod_api::{FluidModel, Hooks, ModEntry, RegisterFluidModelsEvent, RegistrationContext};
        use mod_platform::testing::with_mods;
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        #[derive(Default)]
        struct Models(Arc<AtomicUsize>);
        #[mod_api::sunrise_mod]
        impl Hooks for Models {
            fn register_fluid_models(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterFluidModelsEvent<'_>,
            ) {
                if self.0.load(Ordering::SeqCst) == 2 {
                    return;
                }
                let mut model = FluidModel::default();
                model.fluid = "reload_boundary:oil".into();
                model.still = Some(format!(
                    "reload_boundary:block/oil_{}",
                    self.0.load(Ordering::SeqCst)
                ));
                event.register(model);
            }
        }
        let dir = std::env::temp_dir().join(format!("sunrise-fluid-reload-{}", std::process::id()));
        let fluids = dir.join("data/reload_boundary/sunrise/fluid");
        std::fs::create_dir_all(&fluids).unwrap();
        std::fs::write(dir.join("sunrise-mod.toml"),
                "id = \"reload_boundary\"\nname = \"Reload boundary\"\nversion = \"1\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\n").unwrap();
        std::fs::write(fluids.join("oil.json"), br#"{"world":{}}"#).unwrap();
        let vanilla = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let mut sources = sunrise_gameplay_common::game::mods::ModsState {
            dev: vec![dir.clone()],
            ..Default::default()
        };
        let enabled = ["reload_boundary".to_owned()].into();
        sources.select(&std::env::current_dir().unwrap(), &enabled);
        let generation = Arc::new(AtomicUsize::new(0));
        with_mods(
            vec![ModEntry::new(
                "reload_boundary",
                Box::new(Models(generation.clone())),
            )]
            .into(),
            || {
                let root =
                    sunrise_gameplay_common::game::mods::content_root(&vanilla, &sources).unwrap();
                super::super::native_entity_refs::tests::with_game_role_client(
                    root.prepared().to_path_buf(),
                    sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
                    |game| {
                        game.ui.vanilla_root = vanilla;
                        game.mods = sources;
                        game.client.settings.mods.enabled = enabled;
                        // Drive the real entry point and join its CPU staging worker, without
                        // applying the staged textures (which would need a window/GPU).
                        let mut reload = |expected: Option<&str>| {
                            game.spawn_pack_reload(ReloadKind::Selection).unwrap();
                            let mut pending = game.client.pack_reload.0.take().unwrap();
                            let staged = pending.handle.take().unwrap().join().unwrap().unwrap();
                            let layers = &staged.client_pack.render.sprite_layers;
                            for id in ["reload_boundary:block/oil_0", "reload_boundary:block/oil_1"]
                            {
                                assert_eq!(layers.contains_key(id), expected == Some(id), "{id}");
                            }
                        };
                        mod_platform::__private::with_test_packages(&["reload_boundary"], || {
                            reload(Some("reload_boundary:block/oil_0"));
                            generation.store(1, Ordering::SeqCst);
                            reload(Some("reload_boundary:block/oil_1"));
                            // A provider that stops contributing must clear its old models.
                            generation.store(2, Ordering::SeqCst);
                            reload(None);
                        });
                        sunrise_client::fluid_models::register(vec![]);
                        sunrise_client::fluid_models::set_world_looks([]);
                        std::fs::remove_dir_all(dir).unwrap();
                    },
                );
            },
        );
    }

    fn wait(reload: &mut PackReload) -> (ReloadKind, StageResult) {
        for _ in 0..500 {
            if let Some(done) = reload.finished() {
                return done;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the reload worker never finished");
    }

    #[test]
    #[ignore = "daily"]
    fn resource_pack_overrides_mod_and_vanilla() {
        use std::fs;
        let vanilla =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/sunrise-client/26.3");
        assert!(
            vanilla.join("facts.json").is_file(),
            "prepared vanilla data required"
        );
        let dir =
            std::env::temp_dir().join(format!("sunrise-pack-mod-order-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mod_dir = dir.join("mod");
        let pack_dir = dir.join("packs/override");
        let asset = "assets/order_test/textures/block/example.png";
        for root in [&mod_dir, &pack_dir] {
            fs::create_dir_all(root.join(asset).parent().unwrap()).unwrap();
        }
        fs::write(
            mod_dir.join("sunrise-mod.toml"),
            "id = \"order_test\"\nname = \"Order test\"\nversion = \"1\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\n",
        )
        .unwrap();
        fs::write(mod_dir.join(asset), b"mod").unwrap();
        fs::write(pack_dir.join(asset), b"pack").unwrap();
        fs::write(
            pack_dir.join("pack.mcmeta"),
            br#"{"pack":{"description":"test","pack_format":97}}"#,
        )
        .unwrap();
        {
            use std::io::Write;
            let mut archive =
                zip::ZipWriter::new(fs::File::create(dir.join("packs/override.zip")).unwrap());
            for name in ["pack.mcmeta", asset] {
                archive
                    .start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                archive
                    .write_all(&fs::read(pack_dir.join(name)).unwrap())
                    .unwrap();
            }
            archive.finish().unwrap();
            fs::remove_dir_all(&pack_dir).unwrap();
        }
        let mut sources = sunrise_gameplay_common::game::mods::ModsState {
            dev: vec![mod_dir],
            ..Default::default()
        };
        sources.select(&std::env::current_dir().unwrap(), &Default::default());
        let without_pack = startup_root(&vanilla, &[], &[], &sources)
            .unwrap()
            .selected
            .unwrap();
        assert_eq!(without_pack.source().read(asset).unwrap(), b"mod");
        let pack = packs::folder(&dir.join("packs"), packs::RESOURCE_FORMAT);
        assert_eq!(pack.len(), 1);
        let with_pack = startup_root(&vanilla, &[], &pack, &sources)
            .unwrap()
            .selected
            .unwrap();
        assert_eq!(with_pack.source().read(asset).unwrap(), b"pack");
        // The selected archive changes resource lookup without materializing
        // its override beside generated tables.
        assert_eq!(with_pack.prepared(), without_pack.prepared());
        assert_ne!(
            fs::read(with_pack.prepared().join(asset)).ok().as_deref(),
            Some(b"pack".as_slice())
        );
        assert!(!pack_dir.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_reload_stages_on_a_worker_while_the_caller_goes_on() {
        // The stage blocks until the caller lets it go: `spawn` returning at
        // all means the window thread is free to draw the overlay.
        let (release, blocked) = mpsc::channel::<()>();
        let (thread, ran_on) = mpsc::channel();
        let mut reload = PackReload::spawn(ReloadKind::Selection, move |progress| {
            blocked.recv().unwrap();
            progress.set(2);
            thread
                .send(std::thread::current().name().map(str::to_owned))
                .unwrap();
            Err("staged".into())
        })
        .unwrap();
        assert!(reload.finished().is_none(), "still staging");
        assert_eq!(reload.progress.line(), "Preparing resources");
        release.send(()).unwrap();
        let (kind, staged) = wait(&mut reload);
        assert_eq!(kind, ReloadKind::Selection);
        assert_eq!(staged.err().as_deref(), Some("staged"));
        assert_eq!(reload.progress.line(), "Loading pack tables");
        assert_eq!(ran_on.recv().unwrap().as_deref(), Some("pack-reload"));
        assert!(reload.finished().is_none(), "the outcome is taken once");
    }

    #[test]
    fn a_panicking_stage_fails_the_reload() {
        let mut reload = PackReload::spawn(ReloadKind::Server, |_| panic!("bad pack")).unwrap();
        let (kind, staged) = wait(&mut reload);
        assert_eq!(kind, ReloadKind::Server);
        assert_eq!(
            staged.err().as_deref(),
            Some("the pack reload worker panicked")
        );
    }

    /// `ScreensClientHost::reload_overlay_visible` (#5555): false with no reload, true
    /// while a reload stages and while its overlay fades out, false once it has faded.
    #[test]
    #[ignore = "daily"]
    fn a_mod_reads_whether_the_reload_overlay_is_up() {
        sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
            let visible = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
                <sunrise_client_gameplay::game::ClientGame<'_> as mod_api::prelude::ScreensClientHost>::reload_overlay_visible(game)
            };
            assert!(!visible(game));
            game.client.pack_reload.0 = Some(
                PackReload::spawn_holding(ReloadKind::Selection, Duration::ZERO, |_| {
                    Err("staged".into())
                })
                .unwrap(),
            );
            game.client.reload_overlay =
                Some(sunrise_client_gameplay::game::client_side::packs::ReloadOverlay::default());
            assert!(visible(game), "staging");
            game.client.pack_reload.0 = None;
            assert!(visible(game), "fading out after the reload");
            game.client.reload_overlay = None;
            assert!(!visible(game), "faded out");
        });
    }

    /// `LoadingOverlay.isReadyToFadeOut`: a reload that stages at once still
    /// keeps its overlay up - the 1000 ms `fadeInStart` gate - so a small pack
    /// does not flash the Mojang logo for a frame or two.
    #[test]
    fn a_fast_reload_keeps_the_overlay_up_before_it_applies() {
        assert_eq!(OVERLAY_HOLD, Duration::from_millis(1000));
        let mut quick = PackReload::spawn_holding(ReloadKind::Selection, Duration::ZERO, |_| {
            Err("staged".into())
        })
        .unwrap();
        assert_eq!(wait(&mut quick).0, ReloadKind::Selection);
        // A hold no test machine outlasts, however loaded: the worker is done long
        // before the overlay has had its time.
        let hold = Duration::from_secs(3600);
        let mut held =
            PackReload::spawn_holding(ReloadKind::Selection, hold, |_| Err("staged".into()))
                .unwrap();
        for _ in 0..500 {
            if held.handle.as_ref().is_some_and(|h| h.is_finished()) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(held.handle.as_ref().is_some_and(|h| h.is_finished()));
        assert!(held.finished().is_none(), "the overlay is still up");
        // Once the hold has passed, the staged pack is handed over.
        held.hold = Duration::ZERO;
        assert_eq!(wait(&mut held).0, ReloadKind::Selection);
    }

    #[test]
    fn a_reload_waits_for_world_resources_and_the_one_in_flight() {
        assert!(!reload_waits(false, false));
        assert!(reload_waits(true, false));
        assert!(reload_waits(false, true));
        assert!(reload_waits(true, true));
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
#[path = "clouds_reload_tests.rs"]
pub(crate) mod clouds_reload_tests;

#[cfg(all(test, feature = "client"))]
#[path = "core_shaders_reload_tests.rs"]
mod core_shaders_reload_tests;
