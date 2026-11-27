use super::*;
use std::path::Path;
use sunrise_assets::resource_source::ResourceRoot;
use sunrise_sim::mobs::spawner::SpawnerTypeFacts;
use sunrise_world::{attributes::entity::Catalog as Attributes, item_defaults::ItemDefaults};

pub trait RestPresentation {
    type MobTables: Send;
    type PlayModels: Send;
    type ClientTables: Send;
    type Output;

    fn mob_types(
        &self,
        root: &Path,
        defaults: &ItemDefaults,
        catalog: &Arc<Attributes>,
    ) -> Result<MobTypes>;
    fn mob_tables(
        &self,
        root: &ResourceRoot,
        types: &HashMap<String, Arc<MobType>>,
    ) -> Result<Self::MobTables>;
    fn play_models(&self, root: &Path) -> Self::PlayModels;
    fn client_tables<'scope>(
        &'scope self,
        scope: Option<&'scope std::thread::Scope<'scope, '_>>,
        root: &'scope ResourceRoot,
        resources: &'scope Resources,
        assets: &'scope Assets,
    ) -> Self::ClientTables;
    fn finish(
        &self,
        tables: Self::ClientTables,
        mobs: Self::MobTables,
        models: Self::PlayModels,
    ) -> Result<Self::Output>;
}

pub struct ServerRestPresentation;

impl RestPresentation for ServerRestPresentation {
    type MobTables = ();
    type PlayModels = ();
    type ClientTables = ();
    type Output = ();

    fn mob_types(
        &self,
        root: &Path,
        _: &ItemDefaults,
        catalog: &Arc<Attributes>,
    ) -> Result<MobTypes> {
        MobTypes::load_for_server(root, catalog)
    }
    fn mob_tables(&self, _: &ResourceRoot, _: &HashMap<String, Arc<MobType>>) -> Result<()> {
        Ok(())
    }
    fn play_models(&self, _: &Path) {}
    fn client_tables<'scope>(
        &'scope self,
        _: Option<&'scope std::thread::Scope<'scope, '_>>,
        _: &'scope ResourceRoot,
        _: &'scope Resources,
        _: &'scope Assets,
    ) {
    }
    fn finish(&self, (): (), (): (), (): ()) -> Result<()> {
        Ok(())
    }
}

pub fn load_world_rest<P: RestPresentation + Sync>(
    resource_root: &sunrise_assets::resource_source::ResourceRoot,
    conditions: &sunrise_world::conditions::LoadConditions,
    resources: &Resources,
    spawner_types: HashMap<String, SpawnerTypeFacts>,
    assets: &Assets,
    behaviour_json: &serde_json::Value,
    gameplay_projection: &[sunrise_items::blocks::GameplayProjection],
    item_defaults: &sunrise_world::item_defaults::ItemDefaults,
    progress: &Progress,
    started: Instant,
    validate_entity_part_state: bool,
    presentation: &P,
) -> Result<(WorldRest, P::Output)> {
    let root = resource_root.prepared();
    let facts = resources.facts();
    let biomes = resources.biomes();
    let threads = resources.threads();
    let registry = assets.registry;
    let content = sunrise_level::datapack::DataPack::from_source(
        "sunrise-content".into(),
        resource_root.data_changes().clone(),
    );
    let store =
        sunrise_level::datapack::DataStore::load(&sunrise_level::datapack::DataPacks::new(&[
            &content,
        ]));
    let files = store.files.into_iter().collect::<Vec<_>>();
    let overlay = &sunrise_world::loot::PackOverlay {
        conditions,
        files: &files,
        priority_layers: &store.priority_layers,
    };
    progress.set(6);
    // Independent tables load together, within the worker budget the settings
    // allow. Only the recipes and the stations read another table: the items.
    let load_items = || -> Result<_> {
        let mut items = startup::time("rest.items", || {
            Items::load(root, registry, facts, overlay, resources.item_defaults())
        })?;
        items.bind_component_damage_catalog(&resources.motion().health.damage);
        crate::game::world_entry::bind_world_registry_tags(
            &mut items, root, overlay, overlay, biomes,
        )?;
        api_values::resolve_item_registry(&items)?;
        log::info!("load items: {:.2}s", started.elapsed().as_secs_f32());
        progress.set(7);
        // The recipe tables, the stations and the recipe book read one folder:
        // hold its files so it is read once.
        let conditions = overlay.conditions.with_tags(items.bound_tags());
        let overlay = &sunrise_world::loot::PackOverlay {
            conditions: &conditions,
            ..*overlay
        };
        let _recipe_files = sunrise_world::json_folder::read(&root.join("data/minecraft/recipe"));
        let sources = startup::time("rest.recipe_sources", || {
            crate::game::lan_common::sync::initial_sources(
                root,
                &resources.core.item_data,
                &items,
                overlay,
            )
        })?;
        let recipes = startup::time("rest.recipes", || {
            Recipes::from_resolved(root, &items, overlay, &sources)
        })?;
        let stations = startup::time("rest.stations", || {
            sunrise_sim::stations::StationData::from_sources(
                root,
                &items,
                overlay,
                sunrise_sim::banner::Banners::load(root, resources.item_defaults()),
                (&sources, &recipes),
            )
        })?;
        Ok((items, recipes, stations))
    };
    let load_behaviours = || {
        startup::time("rest.behaviours", || {
            Behaviours::load_with_gameplay(registry, root, behaviour_json, gameplay_projection)
        })
    };
    let load_mobs = || -> Result<_> {
        let _step = startup::step("rest.mobs");
        let catalog = &resources.motion().attributes;
        let mut types = presentation.mob_types(root, item_defaults, catalog)?;
        // #845: the declared sets take their types' rows — the merged rows
        // the live attribute map's catalog took in core (the template's with
        // the declared set on top) — as their attribute map and spawn
        // fields, so a mob of the type spawns with what the map answers.
        for declaration in mod_api::__private::entity_attribute_declarations() {
            let row = catalog.entity_row(&declaration.id).map(std::sync::Arc::new);
            match (types.types.get_mut(&declaration.id), row) {
                (Some(kind), Some(row)) => kind.take_attribute_row(row, catalog),
                (Some(_), None) => log::warn!(
                    "Sunrise: mod entity attributes of {} never reached the catalog; ignored",
                    declaration.id
                ),
                (None, _) => log::warn!(
                    "Sunrise: mod entity attributes of {} name no loaded entity type; ignored",
                    declaration.id
                ),
            }
        }
        // #846: the types mods added attributes to take their grown rows the
        // same way, so a new mob of the type spawns with them.
        resource_load::take_modified_attribute_rows(
            &mut types.types,
            catalog,
            mod_api::__private::entity_attribute_modifications(),
        );
        let mob_types: HashMap<_, _> = types
            .types
            .into_iter()
            .map(|(k, mut v)| {
                // The entity-type registry is complete before the rest loads
                // (`register_iterated_registries`): the type keeps its key from it.
                v.type_key = mod_api::__private::resolved_entity_type_key(&v.id);
                (k, Arc::new(v))
            })
            .collect();
        let mut mob_data = sunrise_sim::mobs::MobData::load(resource_root, facts.states.len())?;
        // NeoForge keys placements by EntityType, not by the subset backed by MobTypes.
        // Use exactly the registry the spawners will consult, with facts already validated.
        mob_data
            .install_spawn_placements(mod_api::__private::spawn_placement_declarations(), |id| {
                spawner_types.contains_key(id)
            })?;
        if validate_entity_part_state {
            crate::game::entity_part_state_common::refuse_entity_part_state_kinds(|kind| {
                mob_types.contains_key(kind)
            })?;
        }
        let client_mobs = presentation.mob_tables(resource_root, &mob_types)?;
        Ok((mob_types, mob_data, client_mobs))
    };
    let load_player_models = || -> Result<_> {
        let _step = startup::step("rest.player_models");
        Ok(presentation.play_models(root))
    };
    // The errors cross threads as text.
    fn text<T, E: std::fmt::Display>(
        result: std::result::Result<T, E>,
    ) -> std::result::Result<T, String> {
        result.map_err(|e| e.to_string())
    }
    let (
        items,
        behaviours,
        mobs,
        player_models,
        providers,
        nether_environment,
        end_environment,
        client_tables,
    ) = if threads > 1 {
        std::thread::scope(|scope| {
            let items = named_scope(scope, "items", || text(load_items()));
            let behaviours = named_scope(scope, "behaviours", || text(load_behaviours()));
            let mobs = named_scope(scope, "mobs", || text(load_mobs()));
            let player_models = named_scope(scope, "player_models", || text(load_player_models()));
            let providers = named_scope(scope, "providers", || {
                startup::time("rest.providers", || Providers::load(root))
            });
            let nether_environment = named_scope(scope, "nether_environment", || {
                startup::time("rest.nether_environment", || {
                    text(Environment::load_dimension(
                        &sunrise_assets::resource_source::ResourceSource::directory(
                            root.to_path_buf(),
                        ),
                        "the_nether",
                    ))
                })
            });
            let end_environment = named_scope(scope, "end_environment", || {
                startup::time("rest.end_environment", || {
                    text(Environment::load_dimension(
                        &sunrise_assets::resource_source::ResourceSource::directory(
                            root.to_path_buf(),
                        ),
                        "the_end",
                    ))
                })
            });
            let client_tables =
                presentation.client_tables(Some(scope), resource_root, resources, assets);
            (
                items.join().unwrap(),
                behaviours.join().unwrap(),
                mobs.join().unwrap(),
                player_models.join().unwrap(),
                providers.join().unwrap(),
                nether_environment.join().unwrap(),
                end_environment.join().unwrap(),
                client_tables,
            )
        })
    } else {
        (
            text(load_items()),
            text(load_behaviours()),
            text(load_mobs()),
            text(load_player_models()),
            Providers::load(root),
            startup::time("rest.nether_environment", || {
                text(Environment::load_dimension(
                    &sunrise_assets::resource_source::ResourceSource::directory(root.to_path_buf()),
                    "the_nether",
                ))
            }),
            startup::time("rest.end_environment", || {
                text(Environment::load_dimension(
                    &sunrise_assets::resource_source::ResourceSource::directory(root.to_path_buf()),
                    "the_end",
                ))
            }),
            presentation.client_tables(None, resource_root, resources, assets),
        )
    };
    // World staging checks conditions after the selected packs have supplied their winners.
    let (items, recipes, stations) = items?;
    let (mob_types, mob_data, client_mobs) = mobs?;
    let models = player_models?;
    let nether_environment = nether_environment?;
    let end_environment = end_environment?;
    let prepared_tests = sunrise_sim::lan::data::PackRegistryDefinitions::load_prepared_game_tests(
        root,
        item_defaults,
    );
    let (test_environments, test_instances) =
        crate::game::test_registry_tables::build_game_test_tables(
            &prepared_tests,
            &mod_api::GameTestFunctions::default(),
        );
    log::info!(
        "Sunrise: resources loaded in {:.1}s; {} block states, {} items, {} biomes; {} worker threads",
        started.elapsed().as_secs_f32(),
        facts.states.len(),
        items.items.len(),
        biomes.list.len(),
        threads
    );
    progress.set(8);
    let world_rest = WorldRest {
        stations,
        items: Arc::new(items),
        recipes,
        transformers: Default::default(),
        providers,
        nether_environment,
        end_environment,
        behaviours: Arc::new(behaviours?),
        mob_types,
        living_type_keys: sunrise_sim::health::LivingTypeKeys::resolve(),
        mob_data,
        spawner_types,
        test_environments,
        test_instances,
    };
    Ok((
        world_rest,
        presentation.finish(client_tables, client_mobs, models)?,
    ))
}
