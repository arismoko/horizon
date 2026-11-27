//! `model_baking_completed` heard by a trait mod through the game's own hooks: once per
//! successful model load, after both atlases and before `client_setup` or
//! `client_resources_reloaded`, with the new store's standalone models resolvable inside it.
#[cfg(test)]
use super::native_entity_refs::tests::{run_fresh_cases, with_client_game};
use super::packs::ReloadKind;
#[cfg(test)]
use mod_api::{
    __private::game::RegistrationContextNew, ClientHost, ClientReloadHost,
    ClientResourcesReloadedEvent, ClientSetupContext, ClientSetupEvent, Hooks, Identifier,
    ModEntry, ModelBakingCompletedEvent, ModelError, ModelRef, RegistrationContext,
    StandaloneModelsEvent, TextureAtlasStitchedEvent,
};
#[cfg(test)]
use mod_platform::testing::with_mods;
#[cfg(test)]
use std::sync::{Arc, Mutex};

/// The standalone model the mod asked for, installed by the fixture as `standalone_models`
/// would register it.
#[cfg(test)]
const PART: &str = "minecraft:block/grass_block";
/// A model nobody registered.
#[cfg(test)]
const NOBODY: &str = "keeper:block/nobody";

/// What the mod kept: the order of what it heard, and the lookups of each
/// `model_baking_completed`.
#[cfg(test)]
#[derive(Default)]
struct Kept {
    heard: Vec<String>,
    part: Vec<Result<ModelRef, ModelError>>,
    nobody: Vec<Result<ModelRef, ModelError>>,
    baked: Vec<[Result<ModelRef, ModelError>; 3]>,
    during_stitch: Vec<Result<ModelRef, ModelError>>,
}
#[cfg(test)]
struct Keeper(Arc<Mutex<Kept>>);
#[cfg(test)]
#[mod_api::__subscriptions]
impl Hooks for Keeper {
    fn standalone_models(&self, _: &RegistrationContext, event: &mut StandaloneModelsEvent<'_>) {
        const CUBE: &str = "minecraft:block/stone";
        const SLAB: &str = "minecraft:block/stone_slab";
        const MISSING: &str = "keeper:absent_dependency";
        event.register_model(PART);
        event.register("keeper:turned", [CUBE], |bake| {
            bake.append_dependency(CUBE, [0, 1, 0], true)
        });
        event.register("keeper:assembly", [CUBE, SLAB], |bake| {
            bake.append_dependency(CUBE, [0; 3], false)?;
            bake.append_dependency(SLAB, [0; 3], false)
        });
        event.register("keeper:missing", [CUBE, MISSING], |bake| {
            bake.append_dependency(CUBE, [0; 3], false)?;
            bake.append_dependency(MISSING, [0; 3], false)
        });
    }

    fn texture_atlas_stitched(
        &self,
        host: &mut dyn ClientHost,
        event: &mut TextureAtlasStitchedEvent,
    ) {
        let mut kept = self.0.lock().unwrap();
        kept.heard.push(format!("atlas {:?}", event.atlas));
        kept.during_stitch
            .push(host.render_assets().model(&id(PART)));
    }
    fn model_baking_completed(&self, host: &mut dyn ClientHost, _: &mut ModelBakingCompletedEvent) {
        let mut kept = self.0.lock().unwrap();
        kept.heard.push("models".into());
        let assets = host.render_assets();
        kept.part.push(assets.model(&id(PART)));
        kept.nobody.push(assets.model(&id(NOBODY)));
        kept.baked.push([
            assets.model(&id("keeper:turned")),
            assets.model(&id("keeper:assembly")),
            assets.model(&id("keeper:missing")),
        ]);
    }
    fn client_setup(&self, _: &mut dyn ClientSetupContext, _: &mut ClientSetupEvent) {
        self.0.lock().unwrap().heard.push("setup".into());
    }
    fn client_resources_reloaded(
        &self,
        _: &dyn ClientReloadHost,
        _: &mut ClientResourcesReloadedEvent,
    ) {
        self.0.lock().unwrap().heard.push("reloaded".into());
    }
}

#[cfg(test)]
fn id(text: &str) -> Identifier {
    Identifier::parse(text).unwrap()
}

#[cfg(test)]
fn keeper() -> (Arc<Mutex<Kept>>, mod_api::Mods) {
    let kept = Arc::new(Mutex::new(Kept::default()));
    let mods = vec![ModEntry::new("keeper", Box::new(Keeper(Arc::clone(&kept))))].into();
    (kept, mods)
}

/// F3+T: the reload stages on a worker and applies here.
pub fn reload(game: &mut crate::game::ClientGame<'_>) {
    game.spawn_pack_reload(ReloadKind::Selection).unwrap();
    let started = std::time::Instant::now();
    while game.client.pack_reload.0.is_some() {
        assert!(
            started.elapsed().as_secs() < 300,
            "the reload never finished"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
        game.drive_pack_reload();
    }
}

/// `ModelEvent.BakingCompleted`: the mod hears it once on the first load, after both atlases and
/// before `client_setup`, and its standalone model resolves inside it while an unregistered name
/// does not; a reload fires it once more before `client_resources_reloaded`, the handle from it
/// resolves and the first load's is stale.
#[cfg(test)]
#[test]
#[ignore = "daily"]
fn a_client_mod_hears_model_baking_completed_once_per_load_and_keeps_its_handles() {
    let _fixture = sunrise_client::models::standalone_fixture(vec![PART.into()]);
    let (kept, mods) = keeper();
    with_mods(mods, || {
        with_client_game(|game| {
            // The first load, as `run` fires them before `client_setup`.
            game.publish_initial_client_models(vec![
                crate::game::client_reload::PreparedClientReload::Vanilla(()),
            ])
            .unwrap();
            let first = {
                let kept = kept.lock().unwrap();
                assert_eq!(
                    kept.heard,
                    ["atlas Blocks", "atlas Gui", "models", "setup"],
                    "once, after both atlases, before client_setup"
                );
                assert_eq!(kept.nobody, [Err(ModelError::UnknownModel)]);
                let [Ok(first)] = kept.part[..] else {
                    panic!("the standalone model resolves in the hook: {:?}", kept.part);
                };
                first
            };
            assert!(
                game.client.client_pack.render.resolve_model(first).is_ok(),
                "the handle is of the store the hook published"
            );
            {
                let mut log = kept.lock().unwrap();
                log.heard.clear();
                log.during_stitch.clear();
            }
            reload(game);
            let kept = kept.lock().unwrap();
            assert_eq!(
                kept.heard,
                ["atlas Blocks", "atlas Gui", "models", "reloaded"],
                "once more, before client_resources_reloaded"
            );
            assert_eq!(
                kept.nobody,
                [Err(ModelError::UnknownModel), Err(ModelError::UnknownModel)]
            );
            let [Ok(_), Ok(second)] = kept.part[..] else {
                panic!(
                    "the standalone model resolves in both hooks: {:?}",
                    kept.part
                );
            };
            assert_eq!(
                kept.during_stitch,
                [Ok(first), Ok(first)],
                "stitch still reads the old model store"
            );
            assert!(game.client.client_pack.render.resolve_model(second).is_ok());
            assert_eq!(
                game.client.client_pack.render.resolve_model(first).err(),
                Some(ModelError::StaleModel),
                "the first load's handle is superseded"
            );
        });
    });
}

/// Real Game/resource-load/model-registry proof, not a GPU render proof.
#[cfg(test)]
#[test]
#[ignore = "daily"]
fn standalone_bakers_resolve_as_one_model_and_republish_after_resource_reload() {
    let (kept, mods) = keeper();
    with_mods(mods, || {
        let mut staged = Vec::new();
        sunrise_gameplay_common::game::native_hooks::native_standalone_models(
            &RegistrationContext::new(),
            &mut staged,
        );
        let _fixture = sunrise_client::models::StandaloneFixture::from_registrations(staged);
        with_client_game(|game| {
            game.publish_initial_client_models(vec![
                crate::game::client_reload::PreparedClientReload::Vanilla(()),
            ])
            .unwrap();
            let [Ok(turned), Ok(assembly), Err(ModelError::UnknownModel)] =
                kept.lock().unwrap().baked[0]
            else {
                panic!("baker results not published atomically");
            };
            assert_eq!(
                game.client
                    .client_pack
                    .render
                    .resolve_model(turned)
                    .unwrap()
                    .faces
                    .len(),
                6
            );
            let model = game
                .client
                .client_pack
                .render
                .resolve_model(assembly)
                .unwrap();
            assert_eq!(model.faces.len(), 12);
            // Both original geometries survive one ModelRef, not two draws or two copies
            // of the first part. The full block and the bottom slab have distinct tops.
            for (faces, height) in [(&model.faces[..6], 1.0), (&model.faces[6..], 0.5)] {
                assert!(
                    faces
                        .iter()
                        .any(|face| face.quad.positions.iter().all(|p| p[1] == height))
                );
            }
            reload(game);
            let kept = kept.lock().unwrap();
            assert_eq!(kept.baked.len(), 2);
            let [
                Ok(second_turned),
                Ok(second_assembly),
                Err(ModelError::UnknownModel),
            ] = kept.baked[1]
            else {
                panic!("baker results not republished");
            };
            assert_eq!(
                game.client.client_pack.render.resolve_model(turned).err(),
                Some(ModelError::StaleModel)
            );
            assert_eq!(
                game.client.client_pack.render.resolve_model(assembly).err(),
                Some(ModelError::StaleModel)
            );
            assert_eq!(
                game.client
                    .client_pack
                    .render
                    .resolve_model(second_turned)
                    .unwrap()
                    .faces
                    .len(),
                6
            );
            assert_eq!(
                game.client
                    .client_pack
                    .render
                    .resolve_model(second_assembly)
                    .unwrap()
                    .faces
                    .len(),
                12
            );
        });
    });
}

/// The production pack build and model publication each occupy the anchor in
/// their respective phase, with neither apply running before all prepares.
#[cfg(test)]
#[test]
#[ignore = "daily"]
fn pack_reload_runs_listeners_on_both_sides_of_model_manager() {
    // Other model loads in this process would also see the installed baker.
    if run_fresh_cases(
        "game::client_side::model_baking_completed_tests::pack_reload_runs_listeners_on_both_sides_of_model_manager",
        "SUNRISE_MODEL_MANAGER_RELOAD_CASE",
        &["reload"],
    ) {
        return;
    }
    use super::{Progress, packs::stage_pack};
    use mod_api::{
        AddClientReloadListenersEvent, ClientReloadContext, ClientReloadListener,
        ClientReloadListenerContext, FileToIdConverter, ProfessionDefinition, ProfessionKey,
        RegisterVillagerProfessionsEvent, ReloadResourceError, VanillaClientListeners,
    };

    struct Listener {
        trace: Arc<Mutex<Vec<&'static str>>>,
        prepare: &'static str,
        apply: &'static str,
    }
    impl ClientReloadListener for Listener {
        fn prepare(
            &self,
            _: &ClientReloadContext,
        ) -> Result<Box<dyn FnOnce() + Send + 'static>, ReloadResourceError> {
            self.trace.lock().unwrap().push(self.prepare);
            let trace = self.trace.clone();
            let apply = self.apply;
            Ok(Box::new(move || trace.lock().unwrap().push(apply)))
        }
    }
    use std::sync::atomic::{AtomicBool, Ordering};

    struct Probe {
        trace: Arc<Mutex<Vec<&'static str>>>,
        refuse: Arc<AtomicBool>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Probe {
        fn add_client_reload_listeners(
            &self,
            _: &dyn ClientReloadListenerContext,
            event: &mut AddClientReloadListenersEvent<'_>,
        ) {
            for (name, prepare, apply) in [
                ("anchor:before", "prepare before", "apply before"),
                ("anchor:after", "prepare after", "apply after"),
            ] {
                event
                    .listeners
                    .add_listener(
                        id(name),
                        FileToIdConverter::json("anchor").unwrap(),
                        Arc::new(Listener {
                            trace: self.trace.clone(),
                            prepare,
                            apply,
                        }),
                    )
                    .unwrap();
            }
            event
                .listeners
                .add_dependency(id("anchor:before"), VanillaClientListeners::model_manager())
                .unwrap();
            event
                .listeners
                .add_dependency(VanillaClientListeners::model_manager(), id("anchor:after"))
                .unwrap();
        }
        fn standalone_models(
            &self,
            _: &RegistrationContext,
            event: &mut StandaloneModelsEvent<'_>,
        ) {
            let trace = self.trace.clone();
            event.register("anchor:stone", ["minecraft:block/stone"], move |bake| {
                trace.lock().unwrap().push("bake model");
                bake.append_dependency("minecraft:block/stone", [0; 3], false)
            });
        }
        fn register_villager_professions(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterVillagerProfessionsEvent<'_>,
        ) {
            if self.refuse.load(Ordering::Relaxed) {
                let definition = ProfessionDefinition {
                    key: ProfessionKey(id("anchor:duplicate")),
                    workstations: vec![],
                    held_poi: vec![],
                    acquirable_poi: vec![],
                    requested_items: vec![],
                    secondary_blocks: vec![],
                    work_sound: None,
                    trades: vec![],
                };
                event.professions.register(definition.clone());
                event.professions.register(definition);
            }
        }
        fn model_baking_completed(
            &self,
            host: &mut dyn ClientHost,
            _: &mut ModelBakingCompletedEvent,
        ) {
            assert!(host.render_assets().model(&id("anchor:stone")).is_ok());
            self.trace.lock().unwrap().push("install model generation");
        }
    }
    let trace = Arc::new(Mutex::new(Vec::new()));
    let refuse = Arc::new(AtomicBool::new(false));
    with_mods(
        vec![ModEntry::new(
            "anchor",
            Box::new(Probe {
                trace: trace.clone(),
                refuse: refuse.clone(),
            }),
        )]
        .into(),
        || {
            let mut registrations = Vec::new();
            sunrise_gameplay_common::game::native_hooks::native_standalone_models(
                &RegistrationContext::new(),
                &mut registrations,
            );
            let _fixture =
                sunrise_client::models::StandaloneFixture::from_registrations(registrations);
            with_client_game(|game| {
                for reject in [true, false] {
                    refuse.store(reject, Ordering::Relaxed);
                    // A clean registration snapshot for each candidate.
                    game.mods.profession_declarations = Default::default();
                    trace.lock().unwrap().clear();
                    let previous = game.client.client_pack.render.clone();
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
                    assert_eq!(
                        *trace.lock().unwrap(),
                        ["prepare before", "bake model", "prepare after"]
                    );
                    if reject {
                        let error = game.apply_staged_pack(staged).unwrap_err().to_string();
                        assert!(
                            error.contains("client villager professions refused"),
                            "{error}"
                        );
                        assert!(error.contains("duplicate"), "{error}");
                        assert!(
                            Arc::ptr_eq(&game.client.client_pack.render, &previous),
                            "old model generation remains installed"
                        );
                        assert_eq!(
                            *trace.lock().unwrap(),
                            ["prepare before", "bake model", "prepare after"]
                        );
                        continue;
                    }
                    game.apply_staged_pack(staged).unwrap();
                    assert!(!Arc::ptr_eq(&game.client.client_pack.render, &previous));
                    assert_eq!(
                        *trace.lock().unwrap(),
                        [
                            "prepare before",
                            "bake model",
                            "prepare after",
                            "apply before",
                            "install model generation",
                            "apply after",
                        ]
                    );
                }
            });
        },
    );
}

/// A reload whose application is refused fires nothing: the mod hears no
/// `model_baking_completed`, and the handle from the last successful load stays right.
#[cfg(test)]
#[test]
#[ignore = "daily"]
fn a_failed_reload_fires_no_model_baking_completed() {
    use super::{Progress, packs::stage_pack};
    let _fixture = sunrise_client::models::standalone_fixture(vec![PART.into()]);
    let (kept, mods) = keeper();
    with_mods(mods, || {
        with_client_game(|game| {
            game.texture_atlases_stitched();
            game.native_model_baking_completed();
            let first = kept.lock().unwrap().part[0].unwrap();
            kept.lock().unwrap().heard.clear();
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
            // An unreadable enabled-mods marker refuses the staged pack's application.
            let root =
                std::env::temp_dir().join(format!("model-baking-refusal-{}", std::process::id()));
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(
                root.join(sunrise_mod_package::enabled::MARKER),
                b"invalid json",
            )
            .unwrap();
            staged.root = root.clone();
            assert!(game.apply_staged_pack(staged).is_err());
            std::fs::remove_dir_all(root).unwrap();
            assert!(kept.lock().unwrap().heard.is_empty(), "nothing fired");
            assert!(
                game.client.client_pack.render.resolve_model(first).is_ok(),
                "the last successful load's handle stays right"
            );
        });
    });
}

#[cfg(test)]
#[test]
#[ignore = "daily"]
fn a_panicking_publication_notice_does_not_stop_later_subscribers_or_recur() {
    struct Observer(Arc<Mutex<Vec<&'static str>>>, bool);
    #[mod_api::__subscriptions]
    impl Hooks for Observer {
        fn texture_atlas_stitched(
            &self,
            _: &mut dyn ClientHost,
            _: &mut TextureAtlasStitchedEvent,
        ) {
            if self.1 {
                panic!("atlas observer");
            }
            self.0.lock().unwrap().push("atlas");
        }
        fn model_baking_completed(
            &self,
            _: &mut dyn ClientHost,
            _: &mut ModelBakingCompletedEvent,
        ) {
            if self.1 {
                panic!("model observer");
            }
            self.0.lock().unwrap().push("models");
        }
    }
    let trace = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![
        ModEntry::new("panic_publication", Box::new(Observer(trace.clone(), true))),
        ModEntry::new(
            "healthy_publication",
            Box::new(Observer(trace.clone(), false)),
        ),
    ]
    .into();
    with_mods(mods, || {
        with_client_game(|game| {
            let journal = Arc::new(Mutex::new(Vec::new()));
            game.transactions.trace = Some(journal.clone());
            let mut reports = Vec::new();
            for reload in 0..2 {
                game.texture_atlases_stitched();
                game.native_model_baking_completed();
                let journal = std::mem::take(&mut *journal.lock().unwrap());
                let mut expected = Vec::new();
                for notification in 0..3 {
                    if reload == 0 && notification != 1 {
                        expected.extend(["open", "abort"]);
                    }
                    expected.extend(["open", "commit"]);
                }
                assert_eq!(
                    journal, expected,
                    "journal closes each subscriber separately"
                );
                reports.extend(
                    mod_api::__private::take_hook_panics()
                        .into_iter()
                        .filter(|report| report.mod_id == "panic_publication")
                        .map(|report| (report.mod_id, report.hook)),
                );
            }
            assert_eq!(
                *trace.lock().unwrap(),
                ["atlas", "atlas", "models", "atlas", "atlas", "models"]
            );
            assert_eq!(
                reports,
                [
                    ("panic_publication", "texture_atlas_stitched"),
                    ("panic_publication", "model_baking_completed")
                ]
            );
        });
    });
}
