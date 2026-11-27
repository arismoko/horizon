//! Process-wide cache of immutable loaded test resources, keyed by every load-affecting input.
use crate::settings::Settings;
use std::time::Instant;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::SystemTime,
};
use sunrise_client_gameplay::game::client_side::resource_load_client::{
    UiPublishArg, load_resources_for_mods,
};
use sunrise_client_gameplay::game::client_side::resource_loader::Loaded;
use sunrise_client_gameplay::game::client_side::resource_tables_client::{
    ClientPackResources, ClientResources, UiResources,
};
use sunrise_client_gameplay::game::client_side::{packs, shader_packs};
use sunrise_game_data::Result;
#[cfg(feature = "client")]
use sunrise_gameplay_common::game::api_values::TryIntoApi;
use sunrise_gameplay_common::game::{PackResources, Resources};
use sunrise_gameplay_common::game::{api_values, execution_role, mods};
use sunrise_level::assets::Assets;
use {
    crate::game::resource_load::*,
    sunrise_gameplay_common::game::resource_load::{
        Progress, publish_entity_type_defaults, with_skip_facts_memo,
    },
};

type FileStamp = Option<(u64, Option<SystemTime>)>;

const STAMPED_FILES: [&str; 8] = [
    "sunrise-resources.json",
    "facts.json",
    "block-tags.json",
    "block-behaviours.json",
    "gameplay-facts.json",
    "items.json",
    "registries.json",
    "mob-data.json",
];

#[derive(Clone, PartialEq, Eq)]
struct CacheKey {
    root: PathBuf,
    vanilla_root: PathBuf,
    role: execution_role::ExecutionRole,
    pub owning: mods::item_data_load::OwningLoad,
    worker_threads: usize,
    #[cfg(feature = "client")]
    texture_budget_mib: u32,
    #[cfg(feature = "client")]
    labpbr_maps: bool,
    #[cfg(feature = "client")]
    skip_modded_splashes: bool,
    stamps: [FileStamp; STAMPED_FILES.len()],
}

struct IteratedRegistries {
    entity_types: Vec<(String, Option<bool>)>,
    game_events: Option<Vec<String>>,
    menus: Option<Vec<String>>,
}

struct CachedEntry {
    root: PathBuf,
    vanilla_root: PathBuf,
    resources: Arc<Resources>,
    entity_models: Option<Arc<sunrise_client::mobs::EntityModelDocument>>,
    pack: PackResources,
    #[cfg(feature = "client")]
    client_res: Option<Arc<OnceLock<ClientResources>>>,
    #[cfg(feature = "client")]
    client_pack: Option<ClientPackResources>,
    #[cfg(feature = "client")]
    gui: Option<Mutex<UiResources>>,
    iterated: IteratedRegistries,
    mod_namespaces: Vec<String>,
}

type Slot = Arc<OnceLock<std::result::Result<Arc<CachedEntry>, String>>>;
const KEPT: usize = 4;
static CACHE: Mutex<Vec<(CacheKey, Slot)>> = Mutex::new(Vec::new());

pub(crate) use sunrise_gameplay_common::game::test_fixtures::acquire_in_process_load_permit;

fn file_stamp(path: &Path) -> FileStamp {
    std::fs::metadata(path)
        .ok()
        .map(|meta| (meta.len(), meta.modified().ok()))
}

fn root_stamps(root: &Path) -> [FileStamp; STAMPED_FILES.len()] {
    STAMPED_FILES.map(|name| file_stamp(&root.join(name)))
}

fn plain_directory_root(roots: &packs::PackRoots) -> Option<PathBuf> {
    if roots.cleared.is_some() {
        return None;
    }
    let Ok(selected) = &roots.selected else {
        return None;
    };
    if !selected.mod_layers().is_empty()
        || !selected.data_changes().is_empty()
        || selected.source().as_directory() != Some(selected.prepared())
        || selected.data().as_directory() != Some(selected.prepared())
        || roots.base.prepared() != selected.prepared()
    {
        return None;
    }
    Some(selected.prepared().to_path_buf())
}

fn load_inputs_are_vanilla(root: &Path, load_role: mods::LoadRole) -> bool {
    // Generated content-mod roots are one-off temp directories and may have empty `enabled-mods.json`.
    if root.join("sunrise-generated-content-files.json").exists() {
        return false;
    }
    let Ok(inventory) = sunrise_mod_package::enabled::EnabledMods::read(root) else {
        return false;
    };
    if inventory.server_ids().next().is_some() || inventory.client_ids().next().is_some() {
        return false;
    }
    if !mod_api::__private::attribute_declarations().is_empty()
        || !mod_api::__private::attribute_bounds_widenings().is_empty()
        || !mod_api::__private::entity_attribute_declarations().is_empty()
        || !mod_api::__private::entity_attribute_modifications().is_empty()
        || !mod_api::__private::spawn_placement_declarations().is_empty()
        || !mod_api::__private::entity_part_state_declarations().is_empty()
        || !sunrise_world::fluid::mod_fluids().is_empty()
        || !sunrise_gameplay_common::game::general::general_type_catalog().is_empty()
        || mod_api::__private::listened(mod_api::HookId::ModifyDefaultComponents)
        || mod_api::__private::listened(mod_api::HookId::ExtendPoiTypes)
        || mod_api::__private::listened(mod_api::HookId::RegisterCauldronFluidContent)
        || (load_role.owning == mods::item_data_load::OwningLoad::Startup
            && mod_api::__private::RegistrationData::initial_settings_snapshot().is_some())
    {
        return false;
    }
    // Hooks whose resolution mutates interior-mutable caches inside `Resources` keep private loads.
    if mod_api::__private::has_part_hook(mod_api::HookId::CollisionShape)
        || mod_api::__private::has_part_hook(mod_api::HookId::PistonPushReaction)
        || mod_api::__private::has_part_hook(mod_api::HookId::IsPathfindable)
        || mod_api::__private::has_part_hook(mod_api::HookId::SupportsEnchantment)
        || mod_api::__private::has_part_hook(mod_api::HookId::IsPrimaryItemFor)
    {
        return false;
    }
    if load_role.execution.is_client()
        && (mod_api::__private::listened(mod_api::HookId::AddClientReloadListeners)
            || mod_api::__private::listened(mod_api::HookId::RegisterTextureAtlases)
            || sunrise_client_render::mesher::has_custom_block_tint_sources()
            || sunrise_client_render::mesher::has_section_geometry()
            || sunrise_client_render::models::has_standalone_models()
            || sunrise_client_render::mod_sprite_sources::has_registered()
            || sunrise_client_render::mod_pipelines::has_registered()
            || !sunrise_client_render::fluid_models::resolved().is_empty()
            || sunrise_client_render::fluid_models::any_flat_look()
            || sunrise_client::item_render::ItemRenderer::has_registered_providers())
    {
        return false;
    }
    true
}

fn load_iterated_registries(root: &Path) -> std::result::Result<IteratedRegistries, String> {
    use sunrise_protocol::registry::Builtin;
    let builtin = Builtin::load(&root.join("registries.json")).map_err(|e| e.to_string())?;
    let ids = &builtin
        .get("minecraft:entity_type")
        .ok_or_else(|| "missing entity type registry".to_owned())?
        .names;
    let rows = sunrise_gameplay_common::game::entity_type_rows::entity_type_rows(root);
    let entity_types = ids
        .iter()
        .filter(|id| !id.is_empty())
        .map(|id| (id.clone(), rows.contains(id).then(|| rows.is_living(id))))
        .collect();
    let game_events = builtin
        .get("minecraft:game_event")
        .map(|ids| ids.names.clone());
    let menus = builtin.get("minecraft:menu").map(|ids| ids.names.clone());
    Ok(IteratedRegistries {
        entity_types,
        game_events,
        menus,
    })
}

fn build_cached_entry(
    key: &CacheKey,
    settings: Settings,
) -> std::result::Result<Arc<CachedEntry>, String> {
    let _permit = acquire_in_process_load_permit();
    let key = key.clone();
    // Neutral thread avoids caller's `TEST_MODS`/`TEST_PARTS` so the cached registry stays vanilla.
    std::thread::Builder::new()
        .stack_size(sunrise_threads::GAME_THREAD_STACK_BYTES)
        .name("test-res-cache".into())
        .spawn(move || {
            // Install an empty Parts fixture first so entity and block keys share one installation.
            mod_platform::__private::part_tests::install_parts_fixture(&[])
                .map_err(|errors| errors.join("\n"))?;
            let load_role = mods::LoadRole {
                execution: key.role,
                owning: key.owning,
            };
            let load = || {
                with_skip_facts_memo(|| {
                    load_resources_for_mods(
                        sunrise_assets::resource_source::ResourceRoot::directory(key.root.clone())
                            .into(),
                        Vec::new(),
                        None,
                        &key.vanilla_root,
                        &settings,
                        Instant::now(),
                        false,
                        load_role,
                        &Progress::new(),
                        &mut |_, _| unreachable!("cached build runs with menu=false"),
                        &mut || unreachable!("fixture without pack recovery"),
                        &mods::ModsState::default(),
                    )
                })
            };
            let loaded = load().map_err(|e| e.to_string())?;
            #[cfg(feature = "client")]
            let gui = if key.role.is_client() {
                let mut gui = loaded.gui.ok_or("cached build produced no GUI bundle")?;
                let client_pack = loaded
                    .client_pack
                    .as_ref()
                    .ok_or("cached build produced no client pack")?;
                gui.atlas.pack_pixels_sync();
                client_pack.render.pixels.pack_sync();
                Some(Mutex::new(gui))
            } else {
                None
            };
            let iterated = load_iterated_registries(&key.root)?;
            let registry = loaded.pack.assets.registry;
            let names = (0..registry.len() as u32)
                .filter_map(|id| registry.state(sunrise_world::registry::BlockStateId(id)))
                .map(|state| state.name.as_str());
            let mod_namespaces = sunrise_sim::save::level_mods::mod_namespaces(names);
            Ok(Arc::new(CachedEntry {
                root: key.root,
                vanilla_root: key.vanilla_root,
                resources: loaded.resources,
                entity_models: loaded.entity_models,
                pack: loaded.pack,
                #[cfg(feature = "client")]
                client_pack: loaded.client_pack,
                #[cfg(feature = "client")]
                client_res: loaded.client_res,
                #[cfg(feature = "client")]
                gui,
                iterated,
                mod_namespaces,
            }))
        })
        .map_err(|e| e.to_string())?
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

fn cached_language(
    root: &Path,
    vanilla_root: &Path,
    code: &str,
    dedicated: bool,
) -> Arc<sunrise_game_data::lang::Language> {
    #[derive(PartialEq, Eq)]
    struct LangKey {
        root: PathBuf,
        code: String,
        dedicated: bool,
        stamp: FileStamp,
    }
    type LangSlot = Arc<OnceLock<Arc<sunrise_game_data::lang::Language>>>;
    static LANGS: Mutex<Vec<(LangKey, LangSlot)>> = Mutex::new(Vec::new());

    let dir = if dedicated { vanilla_root } else { root };
    let lang_code = if dedicated {
        sunrise_game_data::lang::DEFAULT.to_owned()
    } else {
        code.to_owned()
    };
    let key = LangKey {
        root: dir.to_path_buf(),
        stamp: file_stamp(&dir.join(format!("assets/minecraft/lang/{lang_code}.json"))),
        code: lang_code.clone(),
        dedicated,
    };
    let slot = {
        let mut entries = LANGS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(pos) = entries.iter().position(|(k, _)| *k == key) {
            let entry = entries.remove(pos);
            let slot = entry.1.clone();
            entries.push(entry);
            slot
        } else {
            if entries.len() >= 8 {
                entries.remove(0);
            }
            let slot = Arc::new(OnceLock::new());
            entries.push((key, slot.clone()));
            slot
        }
    };
    slot.get_or_init(|| {
        let language = if dedicated {
            sunrise_game_data::lang::Language::load_default(&vanilla_root.to_path_buf())
        } else {
            let source =
                sunrise_assets::resource_source::ResourceSource::directory(root.to_path_buf());
            sunrise_game_data::lang::Language::load(&[&source], code)
        };
        Arc::new(language)
    })
    .clone()
}

fn replay_iterated_registries(iterated: &IteratedRegistries) -> std::result::Result<(), String> {
    let mut living = std::collections::HashMap::with_capacity(iterated.entity_types.len());
    for (id, is_living) in &iterated.entity_types {
        living.insert(id.as_str(), *is_living);
    }
    mod_api::__private::resolve_entity_types(
        iterated.entity_types.iter().map(|(id, _)| id.as_str()),
        |id| living.get(id).copied().flatten(),
    )
    .map_err(|errors| errors.join("\n"))?;
    if let Some(events) = &iterated.game_events {
        mod_api::__private::resolve_game_events(events.iter().map(String::as_str))
            .map_err(|errors| errors.join("\n"))?;
    }
    if let Some(menus) = &iterated.menus {
        mod_api::__private::resolve_menus(menus.iter().map(String::as_str))
            .map_err(|errors| errors.join("\n"))?;
    }
    Ok(())
}

fn hydrate_hit(
    cached: &CachedEntry,
    key: &CacheKey,
    settings: &Settings,
    skip_lang: bool,
    progress: &Progress,
) -> std::result::Result<Option<Loaded>, String> {
    #[cfg(feature = "client")]
    let mut client_pack = if key.role.is_client() {
        let mut client_pack = cached
            .client_pack
            .as_ref()
            .ok_or("missing cached client pack")?
            .clone();
        let Some(item_renderer) = client_pack.item_renderer.clone_for_tests() else {
            return Ok(None);
        };
        client_pack.item_renderer = Arc::new(item_renderer);
        Some(client_pack)
    } else {
        None
    };
    sunrise_threads::jobs::configure(key.worker_threads);
    #[cfg(feature = "client")]
    if key.role.is_client() {
        sunrise_client::sprites::set_texture_budget(key.texture_budget_mib);
        sunrise_client::sprites::set_labpbr_maps(key.labpbr_maps);
    }
    sunrise_level::chunk::set_block_states(cached.pack.assets.registry.len());
    if !mod_api::__private::parts_installation::parts_installed() {
        mod_platform::__private::part_tests::install_parts_fixture(&[])
            .map_err(|errors| errors.join("\n"))?;
    }
    replay_iterated_registries(&cached.iterated)?;
    // Mods that requested custom entity-type keys change `type_key` slots inside `Resources`.
    let entity_keys_match = sunrise_sim::health::LivingTypeKeys::resolve()
        == cached.resources.living_type_keys()
        && cached
            .resources
            .mob_types()
            .values()
            .all(|mob| mob.type_key == mod_api::__private::resolved_entity_type_key(&mob.id));
    if !entity_keys_match {
        return Ok(None);
    }
    publish_entity_type_defaults(&cached.resources.core.motion.attributes);
    sunrise_sim::save::level_mods::set_block_namespaces(Some(cached.mod_namespaces.clone()));
    let registry =
        crate::game::api_values::resolve_cached_block_definitions(cached.pack.assets.registry)?;
    let _ = sunrise_world::poi::extend(Vec::new());
    sunrise_gameplay_common::cauldron_contents::install(
        sunrise_gameplay_common::cauldron_contents::CauldronContents::vanilla(),
    );
    #[cfg(feature = "client")]
    if let Some(client_pack) = &client_pack {
        sunrise_client::pack_palette::set(&client_pack.polytone_colors);
        sunrise_level::brewing::set_pack_colors(client_pack.polytone_colors.effects.clone());
    }
    if !skip_lang {
        sunrise_game_data::lang::set_shared(cached_language(
            &cached.root,
            &cached.vanilla_root,
            &settings.language,
            key.role.is_dedicated_server(),
        ));
    }
    api_values::resolve_item_registry(cached.resources.items())?;

    let mut pack = cached.pack.clone();
    if !std::ptr::eq(registry, cached.pack.assets.registry) {
        let assets = Arc::new(Assets {
            registry,
            properties: cached.pack.assets.properties.clone(),
            material_states: cached.pack.assets.material_states,
            blocks_fluid_flow: cached.pack.assets.blocks_fluid_flow.clone(),
        });
        let old = &cached.pack.chunk_context;
        pack.chunk_context = Arc::new(sunrise_level::chunk_streamer::ChunkWorkerContext {
            assets: assets.clone(),
            biomes: old.biomes.clone(),
            light: old.light.clone(),
        });
        pack.assets = assets;
        #[cfg(feature = "client")]
        if let Some(client_pack) = &mut client_pack {
            client_pack.context = Arc::new(sunrise_client_render::streaming::WorkerContext {
                chunk: (*pack.chunk_context).clone(),
                mesh: client_pack.context.mesh.clone(),
            });
            client_pack.water_overlay_state = registry
                .default_state("minecraft:water")
                .ok_or_else(|| {
                    "block registry is missing the default minecraft:water state".to_owned()
                })?
                .try_into_api()
                .map_err(|e| format!("cannot bind the default minecraft:water state: {e:?}"))?;
        }
    }
    #[cfg(feature = "client")]
    let (gui, client_res) = if key.role.is_client() {
        let gui = cached
            .gui
            .as_ref()
            .ok_or("missing cached GUI bundle")?
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone_for_tests();
        let client_res = cached
            .client_res
            .as_ref()
            .ok_or("missing cached client resource slot")?;
        (Some(gui), Some(client_res.clone()))
    } else {
        (None, None)
    };
    progress.set(8);
    Ok(Some(Loaded {
        game_cpu: None,
        entity_models: cached.entity_models.clone(),
        #[cfg(feature = "client")]
        renderer_cpu: None,
        root: cached.root.clone(),
        pack_error: None,
        resources: cached.resources.clone(),
        rest_loader: None,
        #[cfg(feature = "client")]
        client_applies: Vec::new(),
        pack,
        #[cfg(feature = "client")]
        client_pack,
        #[cfg(feature = "client")]
        client_res,
        #[cfg(feature = "client")]
        gui,
    }))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn load(
    roots: packs::PackRoots,
    additional_data: Vec<Arc<sunrise_level::datapack::DataPack>>,
    launch_world: Option<&str>,
    vanilla_root: &Path,
    settings: &Settings,
    started: Instant,
    menu: bool,
    skip_lang: bool,
    load_role: mods::LoadRole,
    progress: &Progress,
    publish: &mut dyn FnMut(UiPublishArg, &Path),
) -> Result<Loaded> {
    if menu || !additional_data.is_empty() || launch_world.is_some() {
        return load_resources_uncached(
            roots,
            additional_data,
            launch_world,
            vanilla_root,
            settings,
            started,
            menu,
            load_role,
            progress,
            publish,
        );
    }
    let Some(raw_root) = plain_directory_root(&roots) else {
        return load_resources_uncached(
            roots,
            additional_data,
            launch_world,
            vanilla_root,
            settings,
            started,
            menu,
            load_role,
            progress,
            publish,
        );
    };
    if !load_inputs_are_vanilla(&raw_root, load_role) {
        return load_resources_uncached(
            roots,
            additional_data,
            launch_world,
            vanilla_root,
            settings,
            started,
            menu,
            load_role,
            progress,
            publish,
        );
    }
    let (Ok(canon_root), Ok(canon_vanilla)) = (
        std::fs::canonicalize(&raw_root),
        std::fs::canonicalize(vanilla_root),
    ) else {
        return load_resources_uncached(
            roots,
            additional_data,
            launch_world,
            vanilla_root,
            settings,
            started,
            menu,
            load_role,
            progress,
            publish,
        );
    };
    let key = CacheKey {
        stamps: root_stamps(&canon_root),
        root: canon_root,
        vanilla_root: canon_vanilla,
        role: load_role.execution,
        owning: load_role.owning,
        worker_threads: settings.sunrise.worker_threads(),
        #[cfg(feature = "client")]
        texture_budget_mib: settings.sunrise.texture_budget_mib,
        #[cfg(feature = "client")]
        labpbr_maps: load_role.execution.is_client() && shader_packs::picks_material_pack(settings),
        #[cfg(feature = "client")]
        skip_modded_splashes: settings.extras.skip_modded_splashes_if_vanilla_modified,
    };
    let slot = {
        let mut entries = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(pos) = entries.iter().position(|(k, _)| *k == key) {
            let entry = entries.remove(pos);
            let slot = entry.1.clone();
            entries.push(entry);
            slot
        } else {
            if entries.len() >= KEPT {
                entries.remove(0);
            }
            let slot = Arc::new(OnceLock::new());
            entries.push((key.clone(), slot.clone()));
            slot
        }
    };
    let built = slot.get_or_init(|| build_cached_entry(&key, settings.clone()));
    let cached = match built {
        Ok(cached) => cached,
        Err(error) => return Err(error.clone().into()),
    };
    match hydrate_hit(cached, &key, settings, skip_lang, progress)? {
        Some(loaded) => Ok(loaded),
        None => load_resources_uncached(
            roots,
            additional_data,
            launch_world,
            vanilla_root,
            settings,
            started,
            menu,
            load_role,
            progress,
            publish,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_for(role: execution_role::ExecutionRole) -> Loaded {
        let root = PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES")
                .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
        );
        let settings = Settings::default();
        load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            role,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap()
    }

    #[test]
    #[ignore = "daily"]
    fn repeated_and_concurrent_loads_share_resources_while_uncached_and_mutating_hooks_stay_private()
     {
        let first = load_for(execution_role::ExecutionRole::IntegratedServer);
        let second = load_for(execution_role::ExecutionRole::IntegratedServer);
        assert!(first.resources.living_type_keys().player.is_some());
        assert!(Arc::ptr_eq(&first.resources, &second.resources));
        assert!(std::ptr::eq(
            first.pack.assets.registry,
            second.pack.assets.registry
        ));
        assert!(Arc::ptr_eq(
            &first.pack.item_models,
            &second.pack.item_models
        ));
        assert!(Arc::ptr_eq(&first.pack.item_ids, &second.pack.item_ids));
        #[cfg(feature = "client")]
        {
            assert!(first.client_pack.is_none());
            assert!(first.client_res.is_none());
            assert!(first.gui.is_none());
            let c1 = load_for(execution_role::ExecutionRole::Client);
            let mut c2 = load_for(execution_role::ExecutionRole::Client);
            assert!(Arc::ptr_eq(&c1.pack.item_models, &c2.pack.item_models));
            assert!(Arc::ptr_eq(&c1.pack.item_ids, &c2.pack.item_ids));
            let c1_pack = c1.client_pack.as_ref().unwrap();
            let c2_pack = c2.client_pack.as_mut().unwrap();
            assert!(!Arc::ptr_eq(&c1_pack.item_renderer, &c2_pack.item_renderer));
            assert!(Arc::get_mut(&mut c2_pack.item_renderer).is_some());
        }

        // Concurrent calls on separate threads all share the exact same `Arc<Resources>`.
        let others: Vec<_> = std::thread::scope(|scope| {
            (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        load_for(execution_role::ExecutionRole::IntegratedServer).resources
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|h| h.join().unwrap())
                .collect()
        });
        for res in others {
            assert!(Arc::ptr_eq(&first.resources, &res));
        }

        // `load_resources_uncached` always returns a private `Arc<Resources>`.
        let root = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let uncached = load_resources_uncached(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        assert!(!Arc::ptr_eq(&first.resources, &uncached.resources));

        // A custom block key still shares `Resources`/`BlockRegistry::Data` with private worker slots.
        struct CustomBlockKey;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for CustomBlockKey {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                r.block_key("minecraft:oak_sign");
            }
        }
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "cache_custom_key",
            Box::new(CustomBlockKey),
        )]
        .into();
        mod_platform::testing::with_mods(mods.clone(), || {
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            let modded = load_for(execution_role::ExecutionRole::IntegratedServer);
            assert!(Arc::ptr_eq(&first.resources, &modded.resources));
            assert!(!std::ptr::eq(
                first.pack.assets.registry,
                modded.pack.assets.registry
            ));
            assert_eq!(
                first.pack.assets.registry.payload_identity(),
                modded.pack.assets.registry.payload_identity()
            );
        });

        // A mod subscribing to a `Resources`-mutating hook (`CollisionShape`) bypasses cache.
        struct MutatingPart;
        #[mod_api::sunrise_part]
        impl mod_api::BlockPart for MutatingPart {
            fn collision_shape(
                &self,
                _: &dyn mod_api::LoopContext,
                _: &mut mod_api::CollisionShapeEvent<'_>,
            ) {
            }
        }
        struct MutatingMod;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for MutatingMod {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                let key = r.block_key("minecraft:stone");
                r.block_part(key, MutatingPart);
            }
        }
        let mutating: mod_api::Mods = vec![mod_api::ModEntry::new(
            "cache_mutating",
            Box::new(MutatingMod),
        )]
        .into();
        mod_platform::testing::with_mods(mutating.clone(), || {
            sunrise_gameplay_common::game::api_values::install_parts(&mutating).unwrap();
            let private = load_for(execution_role::ExecutionRole::IntegratedServer);
            assert!(!Arc::ptr_eq(&first.resources, &private.resources));
        });
    }

    #[test]
    fn root_stamps_change_when_prepared_files_change_and_overlaid_roots_bypass_cache() {
        let dir = tempfile::tempdir().unwrap();
        let before = root_stamps(dir.path());
        std::fs::write(dir.path().join("facts.json"), b"{}").unwrap();
        let after = root_stamps(dir.path());
        assert_ne!(before, after);

        let plain: packs::PackRoots =
            sunrise_assets::resource_source::ResourceRoot::directory(dir.path().to_path_buf())
                .into();
        assert_eq!(plain_directory_root(&plain), Some(dir.path().to_path_buf()));

        let overlaid: packs::PackRoots =
            sunrise_assets::resource_source::ResourceRoot::directory(dir.path().to_path_buf())
                .with_mod_layers(Arc::from([
                    sunrise_assets::resource_source::ResourceSource::default(),
                ]))
                .into();
        assert_eq!(plain_directory_root(&overlaid), None);
    }
}
