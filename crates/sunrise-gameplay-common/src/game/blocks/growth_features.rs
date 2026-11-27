use std::sync::Arc;
use sunrise_sim::vanillagen;
use sunrise_world::worldgen::live::LiveFeatures;

/// The feature registry over `overrides`: the world's data packs' and the
/// mods' worldgen entries, which a sapling's or a mod's feature may be one of.
/// `Ok(None)` when the bundle has no features.
pub fn build_live_features(
    resources: &std::path::Path,
    registry: &'static sunrise_world::registry::BlockRegistry,
    source: &sunrise_assets::resource_source::ResourceSource,
    overrides: &sunrise_world::worldgen::overrides::WorldgenOverrides,
) -> std::result::Result<Option<LiveFeatures>, String> {
    let data =
        vanillagen::shared_data_from(&resources.join("worldgen"), overrides, registry, source)
            .map_err(|e| e.to_string())?;
    LiveFeatures::new(&data).map_err(|e| e.to_string())
}

/// The last bundle built, so a world opened with the same data packs (the
/// usual re-open) reuses it: the resources folder, the overrides it was built
/// from, and the features.
pub(in crate::game) type Built = (
    std::path::PathBuf,
    sunrise_world::worldgen::overrides::WorldgenOverrides,
    Arc<LiveFeatures>,
);
pub(in crate::game) static LAST_BUILT: std::sync::Mutex<Option<Built>> =
    std::sync::Mutex::new(None);

/// The feature registry for a world with `overrides`: the cached one when the
/// last build had the same ones, else a fresh build (which is then cached).
pub(in crate::game) fn live_features_for(
    options: &super::Options,
    pack: &super::PackResources,
    overrides: &sunrise_world::worldgen::overrides::WorldgenOverrides,
) -> std::result::Result<Option<Arc<LiveFeatures>>, String> {
    let mut last = LAST_BUILT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((dir, built_from, features)) = last.as_ref()
        && *dir == options.resources
        && built_from == overrides
    {
        return Ok(Some(features.clone()));
    }
    let built = build_live_features(
        &options.resources,
        pack.assets.registry,
        &pack.data_changes,
        overrides,
    )?
    .map(Arc::new);
    if let Some(features) = &built {
        *last = Some((
            options.resources.clone(),
            overrides.clone(),
            features.clone(),
        ));
    }
    Ok(built)
}

/// Install the registry for the world being opened, replacing the previous
/// world's. `overrides` are the world's: with the plain bundle only, a data
/// pack's configured feature (a mod sapling's tree) is unknown here and its
/// growth is dropped. Shared by world opening and fixture setup.
pub fn install_live_features(
    res: &super::Resources,
    pack: &super::PackResources,
    options: &super::Options,
    overrides: &sunrise_world::worldgen::overrides::WorldgenOverrides,
) {
    let started = std::time::Instant::now();
    match live_features_for(options, pack, overrides) {
        Ok(Some(features)) => {
            res.behaviours().set_live_features(features);
            log::info!("Sunrise: live features ready in {:?}", started.elapsed());
        }
        Ok(None) => {
            res.behaviours().clear_live_features();
            log::warn!("Sunrise: the worldgen bundle has no features; saplings will not grow")
        }
        Err(e) => {
            res.behaviours().clear_live_features();
            log::warn!("Sunrise: live features unavailable: {e}")
        }
    }
}
