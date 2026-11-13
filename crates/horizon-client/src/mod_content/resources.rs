//! Resolve passive resource winners while retaining original singleton layers.
use super::{ContentMod, GENERATED_FILES, Result, load_layers, root_lifetime};
use horizon_assets::resource_source::{Resource, ResourceRoot, ResourceSource};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

/// Open the passive layers for one load. Assets and data retain the same jar
/// readers; only files the table builder generated override the raw data.
pub fn resolved_resources(
    base: &Path,
    prepared: PathBuf,
    jar_store: &Path,
    mods: &[ContentMod],
    cache: PathBuf,
) -> Result<ResourceRoot> {
    let base_source = ResourceSource::directory(base.to_path_buf());
    let loaded = load_layers(jar_store, mods)?;
    let mut data = base_source
        .list("data/")
        .into_iter()
        .map(|name| {
            let resource = Resource::file(base_source.clone(), name.clone());
            (name, resource)
        })
        .collect::<BTreeMap<_, _>>();
    let mut changes = BTreeMap::new();
    let mut data_layers = Vec::new();
    for group in &loaded {
        let declarations = format!("data/{}/horizon/", group.source.id);
        for layer in &group.layers {
            data_layers.push(layer.data.source().clone());
            for name in layer.data.list("data/") {
                if !name.starts_with(&declarations) {
                    changes.insert(
                        name.clone(),
                        Resource::file(layer.data.source().clone(), name),
                    );
                }
            }
        }
    }
    let mut assets_base = base_source.clone();
    if prepared != base {
        let generated: Vec<String> =
            serde_json::from_slice(&fs::read(prepared.join(GENERATED_FILES))?)?;
        let generated_source = ResourceSource::directory(prepared.clone());
        root_lifetime::retain_source(&prepared, &generated_source);
        let mut generated_assets = BTreeMap::new();
        for name in generated {
            if !horizon_assets::resource_source::stays_inside(&name) {
                return Err(format!("invalid generated resource name: {name}").into());
            }
            let resource = Resource::file(generated_source.clone(), name.clone());
            if name.starts_with("data/") {
                changes.insert(name, resource);
            } else if name.starts_with("assets/") {
                generated_assets.insert(name, resource);
            }
        }
        if !generated_assets.is_empty() {
            let mut files = assets_base
                .list("")
                .into_iter()
                .map(|name| {
                    let resource = Resource::file(assets_base.clone(), name.clone());
                    (name, resource)
                })
                .collect::<BTreeMap<_, _>>();
            files.extend(generated_assets);
            assets_base = ResourceSource::indexed(files);
        }
    }
    let layers = loaded
        .into_iter()
        .flat_map(|group| group.layers.into_iter().map(|layer| layer.assets))
        .collect::<Vec<_>>();
    // Selected asset packs stay above client registry resources without changing
    // the server data source or its original passive layer order.
    if !changes.is_empty() {
        let mut files = assets_base
            .list("")
            .into_iter()
            .map(|name| {
                let resource = Resource::file(assets_base.clone(), name.clone());
                (name, resource)
            })
            .collect::<BTreeMap<_, _>>();
        files.extend(changes.clone());
        assets_base = ResourceSource::indexed(files);
    }
    let assets = crate::packs::resources(assets_base, &layers)?;
    data.extend(changes.clone());
    let all_layers = std::iter::once(base_source)
        .chain(data_layers.iter().cloned())
        .collect();
    Ok(ResourceRoot::new(prepared, cache, assets).with_data(
        ResourceSource::indexed_with_layers(data, all_layers),
        ResourceSource::indexed_with_layers(changes, data_layers),
    ))
}
