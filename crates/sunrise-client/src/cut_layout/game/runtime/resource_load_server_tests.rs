use std::{path::Path, sync::Arc, time::Instant};
use sunrise_assets::resource_source::ResourceRoot;
use sunrise_client_lib::settings::Settings;
use sunrise_game_data::{Result, lang};
use sunrise_gameplay::game::server_side::resource_load_server::load_resources_for_role;
use sunrise_gameplay_common::game::{
    PackResources, Resources,
    mods::LoadRole,
    resource_load::{Progress, with_skip_facts_memo},
    test_fixtures::acquire_in_process_load_permit,
};
use sunrise_level::datapack::DataPack;

pub(crate) struct Loaded {
    pub resources: Arc<Resources>,
    pub pack: PackResources,
}

pub(crate) fn load_resources(
    root: ResourceRoot,
    additional_data: Vec<Arc<DataPack>>,
    launch_world: Option<&str>,
    vanilla_root: &Path,
    _settings: &Settings,
    started: Instant,
    menu: bool,
    role: impl Into<LoadRole>,
    progress: &Progress,
    _publish: &mut dyn FnMut((), &Path),
) -> Result<Loaded> {
    let role = role.into();
    assert!(launch_world.is_none() && !menu && !role.execution.is_client());
    let _permit = acquire_in_process_load_permit();
    let (resources, pack) = with_skip_facts_memo(|| {
        load_resources_for_role(
            root,
            additional_data,
            vanilla_root,
            role,
            1,
            started,
            &Default::default(),
            progress,
            true,
        )
    })?;
    Ok(Loaded { resources, pack })
}

pub(crate) use load_resources as load_resources_uncached;

pub(crate) fn load_resources_skip_lang(
    root: ResourceRoot,
    additional_data: Vec<Arc<DataPack>>,
    launch_world: Option<&str>,
    vanilla_root: &Path,
    settings: &Settings,
    started: Instant,
    role: impl Into<LoadRole>,
    progress: &Progress,
    publish: &mut dyn FnMut((), &Path),
) -> Result<Loaded> {
    let _language = lang::isolate_publication(lang::Language::of("en_us", []));
    load_resources(
        root,
        additional_data,
        launch_world,
        vanilla_root,
        settings,
        started,
        false,
        role,
        progress,
        publish,
    )
}
