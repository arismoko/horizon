use super::*;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use sunrise_assets::{item_model::ItemModels, resource_source::ResourceRoot};
use sunrise_world::{
    block_render::FaceVisibility, item_defaults::ItemData, registry::BlockRegistry,
};
use {
    crate::game::client_side::resource_tables_client::ClientPackParts,
    crate::game::client_side::resource_tables_client::ClientPackResources,
    crate::game::client_side::resource_tables_client::ClientResources,
    crate::game::client_side::resource_tables_client::UiResources,
    sunrise_gameplay_common::game::resource_load::CoreLoadError,
    sunrise_gameplay_common::game::resource_load::CorePresentation,
    sunrise_gameplay_common::game::resource_load::PreparedCore,
    sunrise_gameplay_common::game::resource_load::Progress,
    sunrise_gameplay_common::game::resource_load::ServerCorePresentation,
    sunrise_gameplay_common::game::resource_load::Step,
    sunrise_gameplay_common::game::resource_load::WorldRestLoader,
    sunrise_gameplay_common::game::resource_load::finish_core,
    sunrise_gameplay_common::game::resource_load::load_core,
    sunrise_gameplay_common::game::resource_load::prepare_core,
    sunrise_gameplay_common::game::resource_load::write_prepared_files,
};

pub type UiPublishArg = UiResources;
pub(in crate::game) struct ClientCorePresentation;
pub(in crate::game) struct ClientCoreEarly {
    models: Option<sunrise_client::model_library::ParsedModels>,
    entity_models: Option<Arc<sunrise_client::mobs::EntityModelDocument>>,
}
pub(in crate::game) struct ClientCore {
    entity_models: Option<Arc<sunrise_client::mobs::EntityModelDocument>>,
    render: RenderAssets,
    item_renderer: sunrise_client::item_render::ItemRenderer,
}

impl CorePresentation for ClientCorePresentation {
    type Early = ClientCoreEarly;
    type Visual = RenderAssets;
    type Output = ClientCore;

    fn alongside_assets<A>(
        resources: &ResourceRoot,
        parallel: bool,
        load_assets: impl FnOnce() -> A,
    ) -> (A, Self::Early) {
        let root = resources.prepared();
        // The model library reads every file under `assets/` and needs no
        // block registry, so it is parsed beside `Assets::load` rather than
        // after it. It used to sit inside `reload` behind both the registry
        // read and the block facts; on a cold cache that read was most of a
        // second on the critical path, and warm it is still ~30 ms.
        // A server draws nothing and parses no models, so it skips this.
        std::thread::scope(|scope| {
            let models = parallel.then(|| {
                scope.spawn(|| {
                    let _step = startup::step("core.models_parse");
                    sunrise_client::model_library::ParsedModels::load(resources.source())
                })
            });
            let entity_models = Step::spawn(scope, parallel, "entity_models", || {
                startup::time("core.entity_models", || {
                    sunrise_client::mobs::EntityModelDocument::load(root).ok()
                })
            });
            let assets = load_assets();
            let models = models.map(|m| m.join().unwrap_or_else(|e| std::panic::resume_unwind(e)));

            let entity_models = entity_models.join().map(Arc::new);

            (
                assets,
                ClientCoreEarly {
                    models,
                    entity_models,
                },
            )
        })
    }
    fn render(
        resources: &ResourceRoot,
        assets: &Assets,
        early: &mut Self::Early,
        emissive_sprites: &[mod_api::Answered<mod_api::EmissiveSpriteRegistration>],
    ) -> std::result::Result<Self::Visual, String> {
        startup::time("core.block_models_sprites", || {
            if let Some(parsed) = early.models.take() {
                RenderAssets::load_with_models_and_emissive(
                    resources,
                    assets,
                    parsed,
                    emissive_sprites,
                )
            } else {
                RenderAssets::load_with_emissive(resources, assets, emissive_sprites)
            }
        })
        .map_err(|e| e.to_string())
    }
    fn visibility(
        root: &Path,
        registry: &'static BlockRegistry,
    ) -> std::result::Result<FaceVisibility, String> {
        startup::time("core.visibility", || {
            models::load_visibility(root, registry)
        })
        .map_err(|e| e.to_string())
    }
    fn biomes(resources: &ResourceRoot) -> std::result::Result<Biomes, String> {
        Biomes::load(resources).map_err(|e| e.to_string())
    }
    fn finish(
        resources: &ResourceRoot,
        early: Self::Early,
        render: Self::Visual,
        item_data: &Arc<ItemData>,
    ) -> std::result::Result<Self::Output, String> {
        let library = render.model_library.clone();
        let item_renderer = startup::time("core.item_models", || {
            if let Some(document) = early.entity_models.as_deref() {
                sunrise_client::item_render::ItemRenderer::load_with_library_and_entity_models(
                    resources, library, document, item_data,
                )
            } else {
                sunrise_client::item_render::ItemRenderer::load_with_library(
                    resources, library, item_data,
                )
            }
        })
        .map_err(|e| e.to_string())?;

        Ok(ClientCore {
            entity_models: early.entity_models,
            render,
            item_renderer,
        })
    }
    fn item_tables(
        _: &Path,
        _: &Arc<ItemData>,
        output: &Self::Output,
    ) -> std::result::Result<(Arc<ItemModels>, Arc<HashMap<String, i32>>), String> {
        let renderer = &output.item_renderer;
        Ok((
            Arc::clone(&renderer.models),
            Arc::clone(renderer.item_ids()),
        ))
    }
}

/// The prepared resource location, optional world to preload, and launch start time.
pub struct StartupLoadContext<'a> {
    pub vanilla_root: &'a std::path::Path,
    pub launch_world: Option<&'a str>,
    pub started: Instant,
}

/// A launch without an interactive title owns its settings on the loading thread.
pub fn load_resources_sync(
    roots: packs::PackRoots,
    additional_data: Vec<Arc<sunrise_level::datapack::DataPack>>,
    context: StartupLoadContext<'_>,
    settings: &mut Settings,
    role: impl Into<mods::LoadRole>,
    progress: &Progress,
    mod_sources: &mods::ModsState,
) -> Result<Loaded> {
    let snapshot = settings.clone();
    load_resources_for_mods(
        roots,
        additional_data,
        context.launch_world,
        context.vanilla_root,
        &snapshot,
        context.started,
        false,
        role,
        progress,
        &mut |_, _| {},
        &mut || {
            packs::clear_selection(settings);
            Ok(())
        },
        mod_sources,
    )
}

/// A synchronous launch loads both phases; an ordinary launch returns the
/// core bundle while the rest worker keeps loading. `launch_world` names the
/// world to open, whose data is preloaded during startup.
///
/// The owning mod state supplies the same cached condition table used to build
/// content roots and reload data. Recovery clears the settings owner's selection
/// before reopening the required resource packs over the accepted content root.
#[allow(clippy::too_many_arguments)]
pub fn load_resources_for_mods(
    roots: packs::PackRoots,
    additional_data: Vec<Arc<sunrise_level::datapack::DataPack>>,
    launch_world: Option<&str>,
    vanilla_root: &std::path::Path,
    settings: &Settings,
    started: Instant,
    menu: bool,
    role: impl Into<mods::LoadRole>,
    progress: &Progress,
    publish: &mut dyn FnMut(UiPublishArg, &std::path::Path),
    clear_pack_selection: &mut dyn FnMut() -> Result<()>,
    mod_sources: &mods::ModsState,
) -> Result<Loaded> {
    let default_limit = Arc::new(AtomicU32::new(sunrise_client::hud::MAX_ATLAS_SIZE));
    load_resources_for_mods_with_limit(
        roots,
        additional_data,
        launch_world,
        vanilla_root,
        settings,
        started,
        menu,
        role,
        progress,
        &default_limit,
        &mut |ui, root, _| publish(ui, root),
        clear_pack_selection,
        mod_sources,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn load_resources_for_mods_with_limit(
    roots: packs::PackRoots,
    additional_data: Vec<Arc<sunrise_level::datapack::DataPack>>,
    launch_world: Option<&str>,
    vanilla_root: &std::path::Path,
    settings: &Settings,
    started: Instant,
    menu: bool,
    role: impl Into<mods::LoadRole>,
    progress: &Progress,
    max_atlas_size: &Arc<AtomicU32>,
    publish: &mut dyn FnMut(
        UiPublishArg,
        &std::path::Path,
        &sunrise_assets::resource_source::ResourceSource,
    ),
    clear_pack_selection: &mut dyn FnMut() -> Result<()>,
    mod_sources: &mods::ModsState,
) -> Result<Loaded> {
    let load_role: mods::LoadRole = role.into();
    let role = load_role.execution;
    let packs::PackRoots {
        base,
        selected,
        cleared,
    } = roots;
    let mut recover = |failure: &str, required: &[sunrise_client::packs::Pack]| -> Result<_> {
        log::warn!("Sunrise: resource packs failed to load: {failure}");
        clear_pack_selection()?;
        packs::layer_client_packs(&base, required).map_err(Into::into)
    };
    let (resolved, mut rollback, mut pack_error) = match (selected, cleared) {
        (Ok(selected), cleared) => (selected, cleared, None),
        (Err(error), Some(required)) => {
            let failure = error.to_string();
            (recover(&failure, &required)?, None, Some(failure))
        }
        (Err(error), None) => return Err(error.into()),
    };
    let root = base.prepared().to_path_buf();
    let is_client = role.is_client();

    let (texture_atlases, emissive_sprites) = if is_client {
        (
            client_reload::collect_texture_atlases(),
            client_reload::collect_emissive_sprites(),
        )
    } else {
        (Vec::new(), Vec::new())
    };
    let stage = |root| {
        if is_client {
            client_reload::stage_texture_atlases(root, &texture_atlases)
        } else {
            root
        }
    };

    let mut resolved = stage(resolved);
    let threads = settings.sunrise.worker_threads();
    // One pool of workers for every level's streaming, the client's and the
    // server thread's (sunrise_threads::jobs), sized before any world opens.
    sunrise_threads::jobs::configure(threads);
    if is_client {
        sunrise_client::sprites::set_texture_budget(settings.sunrise.texture_budget_mib);
        sunrise_client::sprites::set_labpbr_maps(shader_packs::picks_material_pack(settings));
    }
    // Stage A: the menu bundle (atlas and panorama). With a loading screen it is
    // published at once so the title screen can appear while the world bundle
    // loads; the synchronous launch instead carries it in `Loaded`.
    let load_ui = |source: &sunrise_assets::resource_source::ResourceSource| {
        let _step = startup::step("ui.load");
        let limit = max_atlas_size.load(Ordering::Relaxed);
        UiResources::load_with_limit(source, vanilla_root, limit).map(|ui| {
            ui.with_splashes(
                source,
                vanilla_root,
                settings.extras.skip_modded_splashes_if_vanilla_modified,
            )
        })
    };
    // The menu bundle loads on this thread while workers build the world
    // bundle's core tables from the same root (`load_core`). A pack failure in
    // either clears the client selection, not required mod resources (NeoForge's
    // `Minecraft.rollbackResourcePacks`).
    let parallel = threads > 1;
    let mut offer = |ui: UiResources,
                     root: &std::path::Path,
                     source: &sunrise_assets::resource_source::ResourceSource| {
        if menu {
            log::info!(
                "Sunrise: UI bundle ready after {} ms",
                started.elapsed().as_millis()
            );
            publish(ui, root, source);
            None
        } else {
            Some(ui)
        }
    };
    let inventory = sunrise_mod_package::enabled::EnabledMods::read(&root)?;
    let conditions = sunrise_gameplay_common::game::load_conditions::conditions_for_mods(
        mod_sources,
        &inventory,
    )?;
    // The generation's settings, captured once before its reload listeners register: they and
    // the item data read this capture, never the files again.
    let captured = mods::item_data_load::capture_load_settings(&root, role, load_role.owning)?;
    let ((core, client), resolved, client_load) = loop {
        let listeners = is_client.then(|| {
            client_reload::register_startup_client_reload_listeners(Some(settings), &captured)
        });
        let (client_ui, core) = std::thread::scope(|scope| {
            let core = |_unit: &()| {
                // A failed pack starts core again from the game's own
                // resources, which the world's preload must read: with packs
                // it waits for the core that loaded.
                let early = launch_world.filter(|_| root == vanilla_root);
                let build = || {
                    startup::time("core", || {
                        if is_client {
                            load_core::<ClientCorePresentation>(
                                &resolved,
                                &additional_data,
                                &conditions,
                                &emissive_sprites,
                                load_role,
                                captured.clone(),
                                parallel,
                                progress,
                                early,
                            )
                            .map(|result| result.map(|(core, client)| (core, Some(client))))
                        } else {
                            load_core::<ServerCorePresentation>(
                                &resolved,
                                &additional_data,
                                &conditions,
                                &emissive_sprites,
                                load_role,
                                captured.clone(),
                                parallel,
                                progress,
                                early,
                            )
                            .map(|result| result.map(|(core, ())| (core, None)))
                        }
                    })
                    .map_err(CoreLoadError::Fatal)?
                    .map_err(CoreLoadError::Pack)
                };
                {
                    if let Some(listeners) = listeners {
                        client_reload::stage_client_reload(resolved.source(), listeners, build)
                    } else {
                        build().map(|core| (core, Vec::new()))
                    }
                }
            };
            #[cfg(any(test, feature = "test-support"))]
            let core = mod_api::testing::inherit_mods(core);
            let core = Step::spawn(scope, parallel, "core", move || core(&()));
            let ui = if is_client {
                load_ui(resolved.source()).map(|ui| offer(ui, &root, resolved.source()))
            } else {
                Ok(None)
            };

            (ui, core.join())
        });
        // A cloud texture vanilla would read past fails its reload (`CloudRenderer.prepare`),
        // and so this load: checked here, where the packs can still be dropped, before the
        // pack tables read it again. A server draws no clouds.
        let clouds = if is_client {
            sunrise_client::clouds::load(resolved.source()).map(drop)
        } else {
            Ok(())
        };
        // Core data is provisional: a selected world pack may replace an entry.
        let (failure, required) = match (client_ui, core, clouds, rollback.take()) {
            (Ok(gui), Ok((core, applies)), Ok(()), _) => {
                break (core, resolved, (gui, applies));
            }
            (_, Err(CoreLoadError::Fatal(e)), _, _) => return Err(e.into()),
            (Err(e), _, _, Some(required)) => (e.to_string(), required),
            (_, Err(CoreLoadError::Pack(e)), _, Some(required)) => (e, required),
            (_, _, Err(e), Some(required)) => (e.to_string(), required),
            (_, Err(CoreLoadError::Pack(e)), _, None) => {
                return Err(format!(
                    "{e}\nPrepare resources with the launcher, or: sunrise-prepare --prepare"
                )
                .into());
            }
            (Err(e), Ok(_), _, None) => return Err(e),
            (Ok(_), Ok(_), Err(e), None) => return Err(e.into()),
        };
        resolved = stage(recover(&failure, &required)?);
        pack_error = Some(failure);
    };
    let (loaded_gui, client_applies) = client_load;

    let (
        PreparedCore {
            mut pack,
            facts,
            biomes,
            item_data,
            item_defaults,
            assets,
            visibility,
            behaviour_json,
            gameplay_projection,
            motion,
            environment,
            collision,
            spawner_types,
        },
        pack_step,
    ) = prepare_core(
        core,
        &root,
        &resolved,
        &additional_data,
        &conditions,
        role,
        launch_world,
        started,
    )?;
    let (client_pack, entity_models) = if let Some(ClientCore {
        render,
        item_renderer,
        entity_models,
    }) = client
    {
        let render = Arc::new(render);
        let tables = Arc::new(startup::time("pack.mesh_tables", || {
            MeshTables::new(
                &assets,
                &render,
                &facts,
                visibility,
                crate::game::client_side::api_values_client::mod_tint_values(assets.registry)?,
            )
            .map(|tables| tables.with_polytone_colormaps(resolved.source(), assets.registry))
        })?);
        (
            Some(ClientPackResources::assemble(
                &resolved,
                &pack,
                ClientPackParts {
                    render,
                    tables,
                    item_renderer,
                },
                true,
            )?),
            entity_models,
        )
    } else {
        (None, None)
    };

    pack.additional_data = additional_data.into();
    drop(pack_step);
    log::info!("load pack tables: {:.2}s", started.elapsed().as_secs_f32());
    progress.set(4);
    if !menu {
        use sunrise_game_data::lang::{self, Language};
        // An integrated server shares the client's loaded resources and
        // language; it doesn't run this loader in production.
        lang::set(if role.is_dedicated_server() {
            Language::load_default(&vanilla_root.to_path_buf())
        } else {
            Language::load(&[&pack.source], &settings.language)
        });
    }
    progress.set(5);
    let resources = finish_core(
        &root,
        item_data,
        facts,
        biomes,
        environment,
        collision,
        motion,
        threads,
    )?;
    if menu {
        log::info!(
            "Sunrise: core-ready after {} ms",
            started.elapsed().as_millis()
        );
    }
    // The block entity renderer's face visibility: the meshes' own, whose
    // shapes already hold their faces, not a second load of the same tables.
    let visibility = client_pack
        .as_ref()
        .map_or_else(sunrise_world::block_render::FaceVisibility::empty, |cp| {
            cp.context.mesh.tables.visibility.clone()
        });

    let client_res_slot = is_client.then(|| Arc::new(std::sync::OnceLock::new()));
    let rest_loader = if menu {
        let resources = resources.clone();
        let resource_root = resolved.clone();
        let vanilla_root = vanilla_root.to_owned();
        let entity_models = entity_models.clone();
        let worker_client_res = client_res_slot.clone();
        let rest_progress = progress.clone();
        let progress = progress.clone();
        let handle = std::thread::Builder::new()
            .stack_size(sunrise_threads::GAME_THREAD_STACK_BYTES)
            .name("world-rest-loader".into())
            .spawn(move || {
                let _step = startup::step("rest");
                let rest = crate::game::client_side::world_rest::load_world_rest(
                    &resource_root,
                    &conditions,
                    &resources,
                    spawner_types,
                    &assets,
                    &behaviour_json,
                    &gameplay_projection,
                    entity_models.as_deref(),
                    item_defaults.as_ref(),
                    visibility,
                    &progress,
                    started,
                    is_client,
                )
                .map_err(|error| error.to_string())?;
                publish_world_rest(&resources, rest, worker_client_res.as_deref())
                    .map_err(str::to_owned)?;
                progress.set(9);
                log::info!(
                    "Sunrise: rest-ready after {} ms",
                    started.elapsed().as_millis()
                );
                write_prepared_files(vanilla_root);
                Ok(())
            })?;
        Some(WorldRestLoader {
            progress: rest_progress,
            handle,
        })
    } else {
        let rest = crate::game::client_side::world_rest::load_world_rest(
            &resolved,
            &conditions,
            &resources,
            spawner_types,
            &assets,
            &behaviour_json,
            &gameplay_projection,
            entity_models.as_deref(),
            item_defaults.as_ref(),
            visibility,
            progress,
            started,
            is_client,
        )?;
        publish_world_rest(&resources, rest, client_res_slot.as_deref())?;
        progress.set(9);
        None
    };
    // The world tables load (above) while the handoff's CPU work is done: the
    // world renderer's textures beside the game's own tables. A failure here
    // waits for the tail, so no tail worker outlives its pack-loader generation.
    let prepared_at = Instant::now();
    let (game_cpu, textures_cpu) = if menu {
        let (game_cpu, textures_cpu) = std::thread::scope(|_scope| {
            // A server read no sprite pixels (`reload_for_server`), so it has no window textures to mip.
            let textures = client_pack.as_ref().map(|cp| {
                Step::spawn(_scope, parallel, "textures", || {
                    startup::time("handoff.renderer_textures", || {
                        prepare_renderer_textures(cp)
                    })
                })
            });
            let library = client_pack
                .as_ref()
                .and_then(|cp| cp.render.model_library.as_deref().ok());

            let game_cpu = startup::time("handoff.game_cpu", || {
                GameCpuResources::load(
                    &resolved,
                    entity_models.as_deref(),
                    library,
                    cfg!(feature = "client") && is_client,
                )
            });
            let textures_cpu = textures.map(Step::join);

            (game_cpu, textures_cpu)
        });
        let game_cpu = match game_cpu {
            Ok(game_cpu) => game_cpu,
            Err(error) => {
                if let Some(tail) = rest_loader {
                    let _ = tail.handle.join();
                }
                return Err(error);
            }
        };
        log::info!(
            "Sunrise: handoff CPU resources prepared on worker in {} ms",
            prepared_at.elapsed().as_millis()
        );
        (Some(game_cpu), textures_cpu)
    } else {
        (None, Default::default())
    };

    Ok(Loaded {
        game_cpu,
        entity_models,
        renderer_cpu: textures_cpu,
        root,
        pack_error,
        resources,
        client_res: client_res_slot,
        rest_loader,
        client_applies,
        pack,
        client_pack,
        gui: loaded_gui,
    })
}

fn publish_world_rest(
    resources: &Resources,
    rest: crate::game::client_side::world_rest::LoadedWorldRest,
    #[cfg(feature = "client")] client_slot: Option<&std::sync::OnceLock<ClientResources>>,
) -> std::result::Result<(), &'static str> {
    if resources.rest.get().is_some() {
        return Err("world resources already loaded");
    }
    let (rest, client_rest) = rest;
    match (client_slot, &client_rest) {
        (Some(slot), Some(_)) if slot.get().is_some() => {
            return Err("client resources already loaded");
        }
        (Some(_), Some(_)) | (None, None) => {}
        (Some(_), None) => return Err("client resources missing for client load"),
        (None, Some(_)) => return Err("unexpected client resources for server load"),
    }
    let _ = resources.rest.set(rest);
    if let (Some(slot), Some(client_rest)) = (client_slot, client_rest) {
        let _ = slot.set(client_rest);
    }
    Ok(())
}
