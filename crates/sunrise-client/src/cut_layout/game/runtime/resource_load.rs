#![cfg(test)]
use crate::settings::Settings;
use std::sync::Arc;
use std::time::Instant;
use sunrise_client_gameplay::game::client_side::resource_load_client::{
    UiPublishArg, load_resources_for_mods,
};
use sunrise_client_gameplay::game::client_side::resource_loader::Loaded;
use sunrise_client_gameplay::game::packs;
use sunrise_game_data::Result;
use sunrise_gameplay_common::game::mods;
use sunrise_gameplay_common::game::resource_load::{Progress, with_skip_facts_memo};
#[cfg(test)]
#[path = "resource_load/test_cache.rs"]
pub(crate) mod test_cache;

/// Fixture helper for tests without a launch mod state.
#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(crate) fn load_resources(
    roots: impl Into<packs::PackRoots>,
    additional_data: Vec<Arc<sunrise_level::datapack::DataPack>>,
    launch_world: Option<&str>,
    vanilla_root: &std::path::Path,
    settings: &Settings,
    started: Instant,
    menu: bool,
    role: impl Into<mods::LoadRole>,
    progress: &Progress,
    publish: &mut dyn FnMut(UiPublishArg, &std::path::Path),
) -> Result<Loaded> {
    test_cache::load(
        roots.into(),
        additional_data,
        launch_world,
        vanilla_root,
        settings,
        started,
        menu,
        false,
        role.into(),
        progress,
        publish,
    )
}

/// [`load_resources`] that leaves process-wide language untouched, for `isolated_server_game`.
#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(crate) fn load_resources_skip_lang(
    roots: impl Into<packs::PackRoots>,
    additional_data: Vec<Arc<sunrise_level::datapack::DataPack>>,
    launch_world: Option<&str>,
    vanilla_root: &std::path::Path,
    settings: &Settings,
    started: Instant,
    role: impl Into<mods::LoadRole>,
    progress: &Progress,
    publish: &mut dyn FnMut(UiPublishArg, &std::path::Path),
) -> Result<Loaded> {
    test_cache::load(
        roots.into(),
        additional_data,
        launch_world,
        vanilla_root,
        settings,
        started,
        false,
        true,
        role.into(),
        progress,
        publish,
    )
}

/// Private, uncached [`load_resources`] for tests that mutate `Resources` or test distinct loads.
#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(crate) fn load_resources_uncached(
    roots: impl Into<packs::PackRoots>,
    additional_data: Vec<Arc<sunrise_level::datapack::DataPack>>,
    launch_world: Option<&str>,
    vanilla_root: &std::path::Path,
    settings: &Settings,
    started: Instant,
    menu: bool,
    role: impl Into<mods::LoadRole>,
    progress: &Progress,
    publish: &mut dyn FnMut(UiPublishArg, &std::path::Path),
) -> Result<Loaded> {
    let _permit = test_cache::acquire_in_process_load_permit();
    let load_role: mods::LoadRole = role.into();
    let loaded = with_skip_facts_memo(|| {
        load_resources_for_mods(
            roots.into(),
            additional_data,
            launch_world,
            vanilla_root,
            settings,
            started,
            menu,
            load_role,
            progress,
            publish,
            &mut || unreachable!("fixture without pack recovery"),
            &mods::ModsState::default(),
        )
    })?;
    #[cfg(feature = "client")]
    let loaded = {
        let mut loaded = loaded;
        if !menu && load_role.execution.is_client() {
            if let Some(gui) = loaded.gui.as_mut() {
                gui.atlas.pack_pixels_sync();
            }
            if let Some(client_pack) = &loaded.client_pack {
                client_pack.render.pixels.pack_sync();
            }
        }
        loaded
    };
    Ok(loaded)
}
