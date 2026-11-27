use std::sync::Arc;
use sunrise_level::datapack::{DataPack, DataPacks};

/// The world's pack stack: the built-in pack (unless disabled), the packs
/// of `<world>/datapacks` and the resources root's feature packs, with the
/// enabled ids from `datapacks.json` (`PackRepository.reload`).
pub struct PackStack {
    pub builtin: Option<DataPack>,
    pub content: DataPack,
    pub packs: Vec<Arc<DataPack>>,
    pub enabled: Vec<String>,
    /// The feature flags the stack was selected for: the incoming world's, also on its first
    /// open, before `self.level` holds them (what `neoforge:feature_flags_enabled` reads).
    pub features: Vec<String>,
}
impl PackStack {
    pub fn new(
        builtin_source: &sunrise_assets::resource_source::ResourceSource,
        content_source: &sunrise_assets::resource_source::ResourceSource,
        vanilla_disabled: bool,
        packs: Vec<DataPack>,
        enabled: Vec<String>,
        enabled_features: &[String],
    ) -> Self {
        let builtin = (!vanilla_disabled)
            .then(|| DataPack::from_source("vanilla".into(), builtin_source.clone()));
        Self {
            content: DataPack::from_source("sunrise-content".into(), content_source.clone()),
            builtin,
            packs: packs.into_iter().map(Arc::new).collect(),
            enabled,
            features: enabled_features.to_vec(),
        }
    }

    /// The enabled packs in `enabled` (the selected) order, later overriding
    /// earlier: `PackRepository.openAllSelected` keeps the selected order.
    pub fn enabled_packs(&self) -> impl Iterator<Item = &DataPack> {
        self.enabled
            .iter()
            .filter_map(|id| self.packs.iter().find(|p| &p.id == id).map(Arc::as_ref))
    }

    /// The merged view of this load's data, lowest layer first: the prepared root (unless
    /// disabled), each content mod, what native mods contribute (`native_data_pack_files`, as
    /// the layer `mod_files`), then the selected packs in selection order. A pack's file
    /// overrides a mod's for the same path, and a later contribution overrides an earlier one,
    /// as packs do. `Game` builds it on a world's first open and on `/reload`, so the mods are
    /// asked once per load.
    pub fn data_packs(&self, contributed: Vec<(String, Vec<u8>)>) -> DataPacks {
        let mut layers: Vec<DataPack> = self.builtin.iter().cloned().collect();
        let mods = self.content.source().named_sources();
        if mods.is_empty() {
            layers.push(self.content.clone());
        } else {
            layers.extend(
                mods.iter()
                    .map(|(id, source)| DataPack::from_source(id.clone(), source.clone())),
            );
        }
        let mut files = std::collections::BTreeMap::new();
        for (key, bytes) in contributed {
            if !sunrise_sim::lan::data::is_pack_registry_file(&key)
                && let Err(e) = serde_json::from_slice::<Box<serde_json::value::RawValue>>(&bytes)
            {
                log::warn!(
                    "Sunrise: a native mod's data pack file {key} is not JSON, skipped: {e}"
                );
                continue;
            }
            // The key is `<kind>/<namespace>:<path>` without `.json`.
            let Some((kind_and_namespace, path)) = key.split_once(':') else {
                log::warn!(
                    "Sunrise: a native mod's data pack file {key} has no namespace, skipped"
                );
                continue;
            };
            let Some((kind, namespace)) = kind_and_namespace.rsplit_once('/') else {
                log::warn!("Sunrise: a native mod's data pack file {key} has no kind, skipped");
                continue;
            };
            files.insert(
                format!("data/{namespace}/{kind}/{path}.json"),
                sunrise_assets::resource_source::Resource::bytes(bytes),
            );
        }
        layers.push(DataPack::from_source(
            "mod_files".into(),
            sunrise_assets::resource_source::ResourceSource::indexed(files),
        ));
        layers.extend(self.enabled_packs().cloned());
        DataPacks::new(&layers.iter().collect::<Vec<_>>())
    }
}
