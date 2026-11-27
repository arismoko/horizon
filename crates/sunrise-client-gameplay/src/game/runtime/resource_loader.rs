//! What a start loads before the game takes over: the CPU-only bundle a loader generation builds
//! (`GameCpuResources`, `EntityModels`), what the background loader publishes (`Loaded`,
//! `LoaderMsg`), the world's rest loader and a pending world entry, the ordinary launch
//! (`run_loading`) and the smoke run's report (`write_report`).
use super::*;
use sunrise_gameplay_common::game::integrated_link::{
    ProfileResolverFactory, RunComponentHarness, Serve,
};
use sunrise_gameplay_common::painting_variants::PaintingVariants;
use sunrise_sim::lan::SessionVerifier;
use {
    crate::game::client_side::armor_stands_client, crate::game::client_side::boats_client,
    crate::game::client_side::frames_client,
    crate::game::client_side::resource_tables_client::ClientPackResources,
    crate::game::client_side::resource_tables_client::ClientResources,
    crate::game::client_side::resource_tables_client::UiResources,
    sunrise_gameplay_common::game::resource_load::WorldRestLoader,
};

#[test]
fn worker_prepares_owned_models_and_inventory_before_handoff() {
    let root = std::env::temp_dir().join(format!("sunrise-startup-cpu-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("entity-models.json"),
        r#"{
        "minecraft:end_crystal#main": {}, "minecraft:ender_dragon#main": {},
        "minecraft:arrow#main": {}, "minecraft:trident#main": {},
        "minecraft:armor_stand#main": {}, "minecraft:cushion#main": {},
        "minecraft:leash_knot#main": {}, "minecraft:wind_charge#main": {},
        "minecraft:shulker_bullet#main": {}, "minecraft:wither_skull#main": {},
        "minecraft:evoker_fangs#main": {}, "minecraft:standing_banner#main": {},
        "minecraft:standing_banner#flag": {}
    }"#,
    )
    .unwrap();
    std::fs::write(
        root.join("creative-inventory.json"),
        r#"{"schema":4,"item_features":{},"potion_features":{},"tabs":[{
        "id":"minecraft:building_blocks","title":"Building Blocks","type":"CATEGORY","build":[
            {"kind":"stack","visibility":"PARENT_AND_SEARCH_TABS","snbt":"{count:1,id:\"minecraft:stone\"}"}
        ]
    }]}"#,
    )
    .unwrap();
    use std::collections::BTreeMap;
    use sunrise_assets::{
        model::CuboidModel,
        model_loaders::{LoaderProvider, LoaderQuad, LoaderSchema, LoaderTexture},
        resolve::ModelLibrary,
        resource_source::ResourceRoot,
    };
    let models = |sprite: &str| {
        let frame = CuboidModel::from_json_with_loaders(
            &format!(r#"{{"loader":"example:frame_loader","textures":{{"frame":"{sprite}"}}}}"#),
            &|loader| {
                assert_eq!(loader, "example:frame_loader");
                Some(LoaderProvider {
                    schema: LoaderSchema::ModelProperties,
                    bake: Arc::new(|_, _, _| {
                        Ok(vec![LoaderQuad {
                            positions: [
                                [0.75, 0.0, 0.5],
                                [0.25, 0.0, 0.5],
                                [0.25, 1.0, 0.5],
                                [0.75, 1.0, 0.5],
                            ],
                            uvs: [[1.0, 1.0], [0.0, 1.0], [0.0, 0.0], [1.0, 0.0]],
                            texture: LoaderTexture::Slot("frame".into()),
                            cull: None,
                            tint_index: -1,
                            shade: true,
                        }])
                    }),
                })
            },
        )
        .unwrap();
        let item_frame = CuboidModel::from_json(r#"{"parent":"example:block/frame"}"#).unwrap();
        ModelLibrary::with_item_generator(BTreeMap::from([
            ("minecraft:block/item_frame".into(), item_frame),
            ("example:block/frame".into(), frame),
        ]))
        .unwrap()
    };
    let server = GameCpuResources::load(
        &sunrise_assets::resource_source::ResourceRoot::directory(root.clone())
            .with_server_profile(),
        None,
        None,
        false,
    )
    .unwrap();
    assert!(server.crystal_model.is_none());
    assert!(server.armor_stand_assets.main.is_none());
    assert!(server.entity_models.leash_knot.is_none());
    assert_eq!(
        server.creative_screen.catalog().tabs()[0].id,
        "minecraft:building_blocks"
    );
    let worker_root = root.clone();
    let prepared = std::thread::spawn(move || {
        let root = ResourceRoot::directory(worker_root);
        let library = models("example:block/startup_frame");
        GameCpuResources::load(&root, None, Some(&library), true).map_err(|error| error.to_string())
    })
    .join()
    .unwrap()
    .unwrap();
    assert_eq!(prepared.frame_model.len(), 1);
    assert_eq!(prepared.frame_model[0].2, "example:block/startup_frame");
    let reloaded = models("example:block/reloaded_frame");
    let [frame, ..] = crate::game::client_side::frames_client::bake_frames(Some(&reloaded));
    assert_eq!(frame.len(), 1);
    assert_eq!(frame[0].2, "example:block/reloaded_frame");
    std::fs::remove_dir_all(&root).unwrap();
    assert!(prepared.crystal_model.is_some());
    assert!(prepared.cushion_model.is_some());
    let models = &prepared.entity_models;
    assert!(models.leash_knot.is_some() && models.wind_charge.is_some());
    assert!(models.shulker_bullet.is_some() && models.wither_skull.is_some());
    assert!(models.evoker_fangs.is_some() && models.banner.is_some());
    assert!(prepared.dragon_model.is_some());
    assert!(prepared.arrow_model.is_some());
    assert!(prepared.trident_model.is_some());
    assert!(prepared.armor_stand_assets.main.is_some());
    assert!(!prepared.creative_screen.has_contents());
    let contents = prepared.creative_screen.catalog().build(
        sunrise_items::creative_catalog::Parameters {
            operator: false,
            features: &[],
            holders: &Default::default(),
            items: &sunrise_items::items::Items::empty(),
        },
        None,
        |_, _, _| {},
    );
    assert_eq!(contents[0].display[0].item.as_ref(), "minecraft:stone");
    // No documents from the old generation remain available to reload.
    assert!(
        GameCpuResources::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            None,
            None,
            true,
        )
        .is_err()
    );
}

/// CPU-only startup data, owned by one loader generation. No file reads or
/// model baking are needed when the event loop installs this bundle.
pub struct GameCpuResources {
    pub(in crate::game) painting_variants: PaintingVariants,
    pub(in crate::game) frame_model: frames_client::FrameModel,
    pub(in crate::game) glow_frame_model: frames_client::FrameModel,
    pub(in crate::game) map_frame_model: frames_client::FrameModel,
    pub(in crate::game) glow_map_frame_model: frames_client::FrameModel,
    pub(in crate::game) armor_stand_assets: armor_stands_client::ArmorStandAssets,
    pub(in crate::game) waypoint_styles: sunrise_client::locator_bar::WaypointStyles,
    pub(in crate::game) boat_models: boats_client::BoatModels,
    pub(in crate::game) cart_models: crate::game::client_side::minecarts_client::CartModels,
    pub(in crate::game) crystal_model: Option<sunrise_client::mobs::Part>,
    pub(in crate::game) cushion_model: Option<sunrise_client::mobs::Part>,
    pub(in crate::game) dragon_model: Option<sunrise_client::mobs::Part>,
    pub(in crate::game) arrow_model: Option<sunrise_client::mobs::Part>,
    pub(in crate::game) bee_stinger_model: Option<sunrise_client::mobs::Part>,
    pub(in crate::game) trident_model: Option<sunrise_client::mobs::Part>,
    pub(in crate::game) entity_models: EntityModels,
    pub(in crate::game) item_features: sunrise_items::creative_catalog::ItemFeatureRequirements,
    pub(in crate::game) creative_screen: Creative,
}

/// Baked layers of entities that may first show up deep into a session. They come from the
/// document parsed once at startup: parsing entity-models.json (2.4 MB) when the first one
/// appeared stalled that frame by 40 ms of CPU and the file read.
#[derive(Default)]
#[cfg_attr(test, derive(Clone))]
pub(in crate::game) struct EntityModels {
    /// LeashKnotModel.
    pub(in crate::game) leash_knot: Option<sunrise_client::mobs::Part>,
    /// WindChargeModel.
    pub(in crate::game) wind_charge: Option<sunrise_client::mobs::Part>,
    /// ShulkerBulletModel.
    pub(in crate::game) shulker_bullet: Option<sunrise_client::mobs::Part>,
    /// SkullModel of the wither skull.
    pub(in crate::game) wither_skull: Option<sunrise_client::mobs::Part>,
    /// EvokerFangsModel.
    pub(in crate::game) evoker_fangs: Option<Arc<sunrise_client::mobs::Part>>,
    /// BannerModel's pole and flag (a raid captain's banner).
    pub(in crate::game) banner: Option<(sunrise_client::mobs::Part, sunrise_client::mobs::Part)>,
    /// Named model layers indexed from `entity-models.json` for mod render queries.
    pub(in crate::game) layers: sunrise_client::fast_hash::FxHashMap<String, mod_api::ModelPart>,
}

impl EntityModels {
    pub(in crate::game::client_side) fn load(
        document: Option<&sunrise_client::mobs::EntityModelDocument>,
    ) -> Self {
        let model = |key| document.and_then(|document| document.model(key));
        let mut layers = sunrise_client::fast_hash::FxHashMap::default();
        if let Some(document) = document {
            for key in document.keys() {
                if let Some(part) = document.model(key) {
                    layers.insert(
                        key.to_owned(),
                        sunrise_client::mobs::model_layers::from_part(&part),
                    );
                }
            }
        }
        Self {
            leash_knot: model("minecraft:leash_knot#main"),
            wind_charge: model("minecraft:wind_charge#main"),
            shulker_bullet: model("minecraft:shulker_bullet#main"),
            wither_skull: model("minecraft:wither_skull#main"),
            evoker_fangs: model("minecraft:evoker_fangs#main").map(Arc::new),
            banner: model("minecraft:standing_banner#main")
                .zip(model("minecraft:standing_banner#flag")),
            layers,
        }
    }
}

impl GameCpuResources {
    #[cfg(test)]
    pub(in crate::game) fn clone_for_tests(&self) -> Self {
        Self {
            painting_variants: self.painting_variants.clone(),
            frame_model: self.frame_model.clone(),
            glow_frame_model: self.glow_frame_model.clone(),
            map_frame_model: self.map_frame_model.clone(),
            glow_map_frame_model: self.glow_map_frame_model.clone(),
            armor_stand_assets: self.armor_stand_assets.clone_for_tests(),
            waypoint_styles: self.waypoint_styles.clone(),
            boat_models: self.boat_models.clone(),
            cart_models: self.cart_models.clone(),
            crystal_model: self.crystal_model.clone(),
            cushion_model: self.cushion_model.clone(),
            dragon_model: self.dragon_model.clone(),
            arrow_model: self.arrow_model.clone(),
            bee_stinger_model: self.bee_stinger_model.clone(),
            trident_model: self.trident_model.clone(),
            entity_models: self.entity_models.clone(),
            item_features: self.item_features.clone(),
            creative_screen: self.creative_screen.clone_for_tests(),
        }
    }

    pub(in crate::game) fn load(
        resources: &sunrise_assets::resource_source::ResourceRoot,
        document: Option<&sunrise_client::mobs::EntityModelDocument>,
        model_library: Option<&sunrise_assets::resolve::ModelLibrary>,
        draws: bool,
    ) -> Result<Self> {
        #[cfg(test)]
        if let Some(cached) = Self::load_cached_for_tests(resources, document, model_library, draws)
        {
            return cached;
        }
        Self::load_uncached(resources, document, model_library, draws)
    }

    #[cfg(test)]
    fn load_cached_for_tests(
        resources: &sunrise_assets::resource_source::ResourceRoot,
        document: Option<&sunrise_client::mobs::EntityModelDocument>,
        model_library: Option<&sunrise_assets::resolve::ModelLibrary>,
        draws: bool,
    ) -> Option<Result<Self>> {
        use std::sync::{Mutex, OnceLock};
        use std::time::SystemTime;
        type Stamp = Option<(u64, Option<SystemTime>)>;
        #[derive(PartialEq, Eq)]
        struct Key {
            root: PathBuf,
            draws: bool,
            model_library: Option<usize>,
            stamps: [Stamp; 3],
        }
        type Slot = Arc<OnceLock<std::result::Result<GameCpuResources, String>>>;
        static CACHE: Mutex<Vec<(Key, Slot)>> = Mutex::new(Vec::new());

        if document.is_some()
            || !resources.mod_layers().is_empty()
            || !resources.data_changes().is_empty()
            || resources.source().as_directory() != Some(resources.prepared())
            || !resources
                .prepared()
                .join("sunrise-resources.json")
                .is_file()
        {
            return None;
        }
        let canon = std::fs::canonicalize(resources.prepared()).ok()?;
        let stat = |name: &str| {
            std::fs::metadata(canon.join(name))
                .ok()
                .map(|m| (m.len(), m.modified().ok()))
        };
        let key = Key {
            root: canon.clone(),
            draws,
            model_library: model_library.map(|m| m as *const _ as usize),
            stamps: [
                stat("entity-models.json"),
                stat("creative-inventory.json"),
                stat("data/minecraft/painting_variant"),
            ],
        };
        let slot = {
            let mut entries = CACHE.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(pos) = entries.iter().position(|(k, _)| *k == key) {
                let entry = entries.remove(pos);
                let slot = entry.1.clone();
                entries.push(entry);
                slot
            } else {
                if entries.len() >= 4 {
                    entries.remove(0);
                }
                let slot = Arc::new(OnceLock::new());
                entries.push((key, slot.clone()));
                slot
            }
        };
        let built = slot.get_or_init(|| {
            Self::load_uncached(resources, None, model_library, draws).map_err(|e| e.to_string())
        });
        Some(match built {
            Ok(cpu) => Ok(cpu.clone_for_tests()),
            Err(e) => Err(e.clone().into()),
        })
    }

    fn load_uncached(
        resources: &sunrise_assets::resource_source::ResourceRoot,
        document: Option<&sunrise_client::mobs::EntityModelDocument>,
        model_library: Option<&sunrise_assets::resolve::ModelLibrary>,
        draws: bool,
    ) -> Result<Self> {
        let root = resources.prepared();
        let owned_document;
        let document = if !draws {
            None
        } else if let Some(document) = document {
            Some(document)
        } else {
            owned_document = sunrise_client::mobs::EntityModelDocument::load(root).ok();
            owned_document.as_ref()
        };
        let visuals = draws.then(|| resources.source());
        let model = |key| document.and_then(|document| document.model(key));
        let (
            painting_variants,
            frames,
            armor_stand_assets,
            waypoint_styles,
            boat_models,
            cart_models,
            creative_screen,
        ) = std::thread::scope(|scope| {
            let painting_variants = named_scope(scope, "game_cpu.paintings", move || {
                PaintingVariants::load(root)
            });
            let frames = named_scope(scope, "game_cpu.frames", move || {
                if draws {
                    frames_client::bake_frames(model_library)
                } else {
                    Default::default()
                }
            });
            let armor_stand_assets = named_scope(scope, "game_cpu.armor_stands", move || {
                visuals
                    .map(|source| armor_stands_client::ArmorStandAssets::load(source, document))
                    .unwrap_or_default()
            });
            let waypoint_styles = named_scope(scope, "game_cpu.waypoints", move || {
                visuals
                    .map(sunrise_client::locator_bar::WaypointStyles::load)
                    .unwrap_or_default()
            });
            let boat_models = named_scope(scope, "game_cpu.boats", move || {
                boats_client::BoatModels::load(document)
            });
            let cart_models = named_scope(scope, "game_cpu.carts", move || {
                crate::game::client_side::minecarts_client::CartModels::load(document)
            });
            let creative_screen = named_scope(scope, "game_cpu.creative", move || {
                let mut creative = Creative::load(root).map_err(|error| error.to_string())?;
                if draws {
                    let tab_modifiers =
                        sunrise_client::polytone_creative_tabs::TabModifier::read_all(root);
                    creative.set_pack_modifiers(tab_modifiers);
                }
                Ok::<_, String>(creative)
            });
            (
                painting_variants
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                frames
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                armor_stand_assets
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                waypoint_styles
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                boat_models
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                cart_models
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                creative_screen
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
            )
        });
        let [
            frame_model,
            glow_frame_model,
            map_frame_model,
            glow_map_frame_model,
        ] = frames;
        let creative_screen =
            creative_screen.map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        let item_features = creative_screen.catalog().item_features();
        Ok(Self {
            painting_variants,
            frame_model,
            glow_frame_model,
            map_frame_model,
            glow_map_frame_model,
            armor_stand_assets,
            waypoint_styles,
            boat_models,
            cart_models,
            crystal_model: model("minecraft:end_crystal#main"),
            cushion_model: model("minecraft:cushion#main"),
            dragon_model: model("minecraft:ender_dragon#main"),
            arrow_model: model("minecraft:arrow#main"),
            bee_stinger_model: model("minecraft:bee_stinger#main"),
            trident_model: model("minecraft:trident#main"),
            entity_models: EntityModels::load(document),
            item_features,
            creative_screen,
        })
    }
}

/// The data the background loader produces for `Game::new`.
pub struct Loaded {
    pub game_cpu: Option<GameCpuResources>,
    /// The one parsed `entity-models.json`, for the handoff `Game::new` does
    /// itself on the synchronous launch.
    pub entity_models: Option<Arc<sunrise_client::mobs::EntityModelDocument>>,
    #[cfg(feature = "client")]
    pub renderer_cpu: Option<sunrise_client::gfx::PreparedTextures>,
    pub root: PathBuf,
    pub pack_error: Option<String>,
    pub resources: Arc<Resources>,
    #[cfg(feature = "client")]
    pub client_res: Option<Arc<std::sync::OnceLock<ClientResources>>>,
    pub rest_loader: Option<WorldRestLoader>,
    /// The client reload listeners' prepared applies, staged by the loader
    /// thread; `start_game` runs them once the game exists.
    #[cfg(feature = "client")]
    pub client_applies: Vec<client_reload::PreparedClientReload>,
    pub pack: PackResources,
    #[cfg(feature = "client")]
    pub client_pack: Option<ClientPackResources>,
    /// The menu bundle, for the synchronous launch; the loading screen receives
    /// it earlier through its own channel.
    #[cfg(feature = "client")]
    pub gui: Option<UiResources>,
}

/// What the background loader publishes: the menu bundle as soon as it is
/// ready, then the core bundle with a handle to the rest worker.
pub enum LoaderMsg {
    #[cfg(feature = "client")]
    Ui(
        UiResources,
        PathBuf,
        sunrise_assets::resource_source::ResourceSource,
    ),
    /// Clear the failed revision on the thread that owns settings, then acknowledge.
    ClearPackSelection {
        revision: u64,
        ack: std::sync::mpsc::Sender<()>,
    },
    Done(std::result::Result<Loaded, LoaderFailure>),
}

impl LoaderMsg {
    pub fn done(result: Result<Loaded>) -> Self {
        Self::Done(result.map_err(
            |error| match error.downcast::<mods::DeclarationFailures>() {
                Ok(refusals) => LoaderFailure::Declarations(*refusals),
                Err(error) => LoaderFailure::Other(error.to_string()),
            },
        ))
    }
}

/// Only the startup report needs its error type preserved across threads.
pub enum LoaderFailure {
    Declarations(mods::DeclarationFailures),
    Other(String),
}

impl From<LoaderFailure> for Box<dyn std::error::Error> {
    fn from(error: LoaderFailure) -> Self {
        match error {
            LoaderFailure::Declarations(refusals) => refusals.into(),
            LoaderFailure::Other(message) => message.into(),
        }
    }
}

pub enum PendingWorldEntry {
    Local {
        folder: String,
        name: Option<String>,
        seed: Option<i64>,
        creative: bool,
        create: Option<Box<sunrise_sim::world_options::WorldOptions>>,
    },
    Server {
        address: String,
        name: String,
        /// The saved server list row's name, when the join came from that row.
        list_name: Option<String>,
        lan: bool,
        realm: bool,
        /// A transfer's carried-over server cookies (`TransferState`).
        cookies: Option<sunrise_client::net::Cookies>,
    },
    StudioReplay(std::path::PathBuf),
}

#[cfg(feature = "client")]
pub(in crate::game) fn write_report(game: &crate::game::ClientGame<'_>) -> Result<()> {
    if let Some(path) = &game.options.report {
        let report = serde_json::json!({
            "frames": game.client.frames,
            "elapsed_seconds": game.client.started.elapsed().as_secs_f64(),
            "chunks": game.world.chunks.len(),
            "generated": game.client.streaming.generated_total,
            "meshed_sections": game.client.streaming.meshed_total,
            "player": game.player.as_ref().map(|p| [p.position.x, p.position.y, p.position.z]),
            "error": game.error.as_ref().map(|e| e.to_string()),
        });
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    Ok(())
}

/// The ordinary launch: load the resources on a worker while the window shows
/// the loading screen, then swap in the game on the main thread.
#[cfg(feature = "client")]
pub fn run_loading(
    options: Options,
    settings: Settings,
    vanilla_root: PathBuf,
    pack_error: Option<String>,
    started: Instant,
    serve_integrated: Serve,
    run_component_harness: RunComponentHarness,
    session_verifier: SessionVerifier,
    profile_resolver: ProfileResolverFactory,
) -> Result<()> {
    let progress = Progress::new();
    let menu = startup_menu::StartupMenu::new(&options, pack_error.as_deref())?;
    let max_atlas_size = Arc::new(AtomicU32::new(sunrise_client::hud::MAX_ATLAS_SIZE));
    let (receiver, loader) = startup_menu::spawn_loader(
        &options,
        &settings,
        menu.pack_selection_revision,
        mods::LoadRole::startup(options.execution_role),
        &vanilla_root,
        started,
        &progress,
        &max_atlas_size,
    )?;
    let loading = Loading {
        serve_integrated,
        run_component_harness,
        session_verifier,
        profile_resolver,
        window: None,
        renderer: None,
        max_atlas_size,
        boot: sunrise_client::hud::HudAtlas::boot(),
        gui: None,
        settings,
        fullscreen: sunrise_client::window::FullscreenState::new(false),
        video_modes: Default::default(),
        options: Some(options),
        vanilla_root,
        pack_error,
        started,
        panorama_rotation: Default::default(),
        progress,
        bar: 0.0,
        receiver,
        loaded: None,
        delivered: false,
        handle: Some(loader),
        shown: false,
        ui_at: None,
        title_drawn: false,
        menu,
        error: None,
    };
    let mut app = App::Loading(Box::new(loading));
    let result = EventLoop::new()?.run_app(&mut app);
    narrator::stop();
    match &mut app {
        App::Ready(game) => leave_fullscreen_after(&result, &mut game.client.settings),
        App::Loading(loading) => leave_fullscreen_after(&result, &mut loading.settings),
    }
    result?;
    match app {
        App::Ready(mut game) => {
            write_report(&game)?;
            if let Some(e) = game.error.take() {
                return Err(e);
            }
        }
        App::Loading(loading) => {
            if let Some(e) = loading.error {
                return Err(e);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_models_indexes_non_mob_layers_without_keeping_disk_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        std::fs::write(
            path.join("entity-models.json"),
            r#"{
                "minecraft:chest#main": {
                    "pose": [1.0, 2.0, 3.0, 0.25, -0.5, 0.75, 1.0, 2.0, 0.5],
                    "polygons": [
                        [
                            [0.0, 0.0, 0.0, 0.0, 0.0],
                            [14.0, 0.0, 0.0, 0.25, 0.0],
                            [14.0, 10.0, 0.0, 0.25, 0.5],
                            [0.0, 10.0, 0.0, 0.0, 0.5]
                        ]
                    ],
                    "normals": [[0.0, 0.0, 1.0]],
                    "children": {
                        "lid": { "pose": [1.0, 9.0, 1.0] },
                        "lock": { "pose": [7.0, 7.0, 15.0] }
                    }
                }
            }"#,
        )
        .unwrap();

        let document = sunrise_client::mobs::EntityModelDocument::load(&path).unwrap();
        let entity_models = EntityModels::load(Some(&document));
        drop(document);
        drop(dir);
        assert!(!path.exists());

        let mut root_pose = mod_api::PartPose::offset_and_rotation(1.0, 2.0, 3.0, 0.25, -0.5, 0.75);
        root_pose.x_scale = 1.0;
        root_pose.y_scale = 2.0;
        root_pose.z_scale = 0.5;
        let expected = mod_api::ModelPart::new(
            vec![mod_api::ModelPolygon::new(
                [
                    mod_api::ModelVertex::new([0.0, 0.0, 0.0], 0.0, 0.0),
                    mod_api::ModelVertex::new([14.0, 0.0, 0.0], 0.25, 0.0),
                    mod_api::ModelVertex::new([14.0, 10.0, 0.0], 0.25, 0.5),
                    mod_api::ModelVertex::new([0.0, 10.0, 0.0], 0.0, 0.5),
                ],
                [0.0, 0.0, 1.0],
            )],
            vec![
                (
                    "lid".to_owned(),
                    mod_api::ModelPart::new(
                        Vec::new(),
                        Vec::new(),
                        mod_api::PartPose::offset(1.0, 9.0, 1.0),
                    ),
                ),
                (
                    "lock".to_owned(),
                    mod_api::ModelPart::new(
                        Vec::new(),
                        Vec::new(),
                        mod_api::PartPose::offset(7.0, 7.0, 15.0),
                    ),
                ),
            ],
            root_pose,
        );

        assert_eq!(
            entity_models.layers.get("minecraft:chest#main"),
            Some(&expected)
        );
        assert_eq!(entity_models.layers.get("minecraft:unknown#main"), None);
        assert!(EntityModels::load(None).layers.is_empty());
    }
}
