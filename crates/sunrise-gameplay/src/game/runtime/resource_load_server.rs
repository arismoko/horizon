use super::*;
use std::path::Path;
use sunrise_assets::resource_source::ResourceRoot;
use sunrise_gameplay_common::game::mods;
use sunrise_gameplay_common::game::{
    resource_load::{
        PreparedCore, Progress, ServerCorePresentation, finish_core, load_core, prepare_core,
    },
    world_rest_common::{self, ServerRestPresentation},
};
use sunrise_level::sim_settings::SimSettings;

pub fn load_resources_server(
    root: ResourceRoot,
    additional_data: Vec<Arc<sunrise_level::datapack::DataPack>>,
    vanilla_root: &Path,
    _settings: &SimSettings,
    threads: usize,
    started: Instant,
    mods: &mods::ModsState,
    progress: &Progress,
    validate_entity_part_state: bool,
) -> Result<(Arc<Resources>, PackResources)> {
    load_resources_for_role(
        root,
        additional_data,
        vanilla_root,
        mods::LoadRole::startup(execution_role::ExecutionRole::DedicatedServer),
        threads,
        started,
        mods,
        progress,
        validate_entity_part_state,
    )
}

#[doc(hidden)]
pub fn load_resources_for_role(
    root: ResourceRoot,
    additional_data: Vec<Arc<sunrise_level::datapack::DataPack>>,
    vanilla_root: &Path,
    role: mods::LoadRole,
    threads: usize,
    started: Instant,
    mods: &mods::ModsState,
    progress: &Progress,
    validate_entity_part_state: bool,
) -> Result<(Arc<Resources>, PackResources)> {
    sunrise_threads::jobs::configure(threads);
    let prepared = root.prepared();
    let inventory = sunrise_mod_package::enabled::EnabledMods::read(prepared)?;
    let conditions =
        sunrise_gameplay_common::game::load_conditions::conditions_for_mods(mods, &inventory)?;
    let captured =
        mods::item_data_load::capture_load_settings(prepared, role.execution, role.owning)?;
    let (core, ()) = startup::time("core", || {
        load_core::<ServerCorePresentation>(
            &root,
            &additional_data,
            &conditions,
            &[],
            role,
            captured,
            threads > 1,
            progress,
            None,
        )
    })?
    .map_err(|error| {
        format!("{error}\nPrepare resources with the launcher, or: sunrise-prepare --prepare")
    })?;
    let (
        PreparedCore {
            mut pack,
            facts,
            biomes,
            item_data,
            item_defaults,
            assets,
            behaviour_json,
            gameplay_projection,
            motion,
            environment,
            collision,
            spawner_types,
            ..
        },
        pack_step,
    ) = prepare_core(
        core,
        prepared,
        &root,
        &additional_data,
        &conditions,
        role.execution,
        None,
        started,
    )?;
    pack.additional_data = additional_data.into();
    drop(pack_step);
    log::info!("load pack tables: {:.2}s", started.elapsed().as_secs_f32());
    progress.set(4);
    sunrise_game_data::lang::set(sunrise_game_data::lang::Language::load_default(
        &vanilla_root.to_path_buf(),
    ));
    progress.set(5);
    let resources = finish_core(
        prepared,
        item_data,
        facts,
        biomes,
        environment,
        collision,
        motion,
        threads,
    )?;
    let (rest, ()) = world_rest_common::load_world_rest(
        &root,
        &conditions,
        &resources,
        spawner_types,
        &assets,
        &behaviour_json,
        &gameplay_projection,
        &item_defaults,
        progress,
        started,
        validate_entity_part_state,
        &ServerRestPresentation,
    )?;
    resources
        .rest
        .set(rest)
        .map_err(|_| "world resources already loaded")?;
    progress.set(9);
    Ok((resources, pack))
}
