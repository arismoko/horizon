//! Mod-supplied repository sources. Discovery owns the metadata and retained
//! readers; selection and resource/data reloads use the ordinary pack paths.
pub use super::{api_values::TryIntoEngine, mods::ModsState};
pub use mod_api::{
    HookId, PackDiscoveryContext, PackFinderError, PackFinders, PackPosition, PackRegistration,
    PackSource, PackType,
};
pub use sunrise_assets::resource_source::{ResourceSource, stays_inside};
use sunrise_level::datapack::DataPack;
pub use sunrise_level::pack_meta::{
    PackInfo, PackMeta, PackSource as ScreenSource, Position, Source, pack_meta_from, read_meta,
};

pub struct DiscoveredPack {
    pub meta: PackMeta,
    pub metadata: serde_json::Value,
}

pub struct Registry {
    kind: PackType,
    mods: ModsState,
    pub packs: Vec<DiscoveredPack>,
}
impl Registry {
    pub fn new(kind: PackType, mods: ModsState) -> Self {
        Self {
            kind,
            mods,
            packs: Vec::new(),
        }
    }

    pub fn register_listener(&mut self, listen: &mut dyn FnMut(&mut dyn PackFinders) -> bool) {
        let checkpoint = self.packs.len();
        if !listen(self) {
            self.packs.truncate(checkpoint);
        }
    }
}
impl PackFinders for Registry {
    fn pack_type(&self) -> PackType {
        self.kind
    }
    fn add_pack_finders(&mut self, entry: PackRegistration) -> Result<(), PackFinderError> {
        if entry.pack_type != self.kind {
            return Ok(());
        }
        let location = &entry.location;
        (|| {
            if !stays_inside(location.path()) {
                return Err(PackFinderError::InvalidPath);
            }
            let id = format!("mod/{location}");
            if self.packs.iter().any(|pack| pack.meta.id == id) {
                return Err(PackFinderError::Duplicate);
            }
            let package = self
                .mods
                .native
                .packages()
                .iter()
                .map(|package| (package.manifest.id.as_str(), &package.path))
                .chain(
                    self.mods
                        .selected
                        .iter()
                        .map(|package| (package.package.manifest.id.as_str(), &package.path)),
                )
                .find(|(id, _)| *id == location.namespace())
                .map(|(_, path)| path)
                .ok_or(PackFinderError::MissingPackage)?;
            let source = if package.is_dir() {
                ResourceSource::directory(package.clone())
            } else {
                ResourceSource::archive(package).map_err(|_| PackFinderError::MissingPackage)?
            };
            let source = source
                .subdirectory(location.path())
                .map_err(|_| PackFinderError::InvalidPath)?;
            let format = match self.kind {
                PackType::ClientResources => sunrise_core::version::RESOURCE_PACK_FORMAT,
                PackType::ServerData => sunrise_core::version::DATA_PACK_FORMAT,
                _ => return Err(PackFinderError::InvalidMetadata),
            };
            let title = entry.title.try_into_engine(&())?;
            let source = Source::Opened(source);
            let metadata = read_meta(&source).ok_or(PackFinderError::InvalidMetadata)?;
            let mut pack = pack_meta_from(id, title, source, format, &metadata);
            pack.selection = Some(PackInfo {
                id: pack.id.clone(),
                title: pack.title.clone(),
                description: pack.raw_description.clone(),
                description_style: pack.description_style.clone(),
                source: match entry.source {
                    PackSource::BuiltIn => ScreenSource::BuiltIn,
                    PackSource::Feature => ScreenSource::Feature,
                    _ => return Err(PackFinderError::InvalidMetadata),
                },
                compat: pack.compatibility,
                icon: None,
                required: entry.always_active,
                position: match entry.position {
                    PackPosition::Top => Position::Top,
                    PackPosition::Bottom => Position::Bottom,
                    _ => return Err(PackFinderError::InvalidMetadata),
                },
                fixed: false,
                hidden: entry.hidden,
            });
            self.packs.push(DiscoveredPack {
                meta: pack,
                metadata,
            });
            Ok(())
        })()
        .inspect_err(|error| {
            let owner = mod_api::content::crash::current_native_mod().unwrap_or("unknown");
            log::warn!("mod {owner} hook add_pack_finders ({location}): {error}");
        })
    }
}

/// A repository may be discovered before Game exists. As with startup reload
/// listeners, this path does not have a game host to lend.
pub struct StartupDiscovery;
impl PackDiscoveryContext for StartupDiscovery {
    fn live_host(&mut self) -> Option<mod_api::HostSide<'_>> {
        None
    }
}
impl mod_api::prelude::TextHost for StartupDiscovery {
    fn text_string(&self, text: &mod_api::Text) -> String {
        sunrise_game_data::lang::scoped(sunrise_game_data::lang::Language::default(), || {
            sunrise_sim::api_text::to_engine_inprocess(text).plain()
        })
        .0
    }

    fn text_from_json(
        &self,
        _: &str,
    ) -> Result<Option<mod_api::Text>, mod_api::text::TextFormError> {
        Err(mod_api::text::TextFormError::Unavailable)
    }

    fn text_to_nbt(&self, _: &mod_api::Text) -> Result<mod_api::Tag, mod_api::text::TextFormError> {
        Err(mod_api::text::TextFormError::Unavailable)
    }

    fn text_from_nbt(
        &self,
        _: &mod_api::Tag,
    ) -> Result<Option<mod_api::Text>, mod_api::text::TextFormError> {
        Err(mod_api::text::TextFormError::Unavailable)
    }

    fn text_from_legacy(&self, _: &str) -> Result<mod_api::Text, mod_api::text::TextFormError> {
        Err(mod_api::text::TextFormError::Unavailable)
    }
}
pub fn discover(kind: PackType, mods: &ModsState) -> Vec<DiscoveredPack> {
    if !mod_api::__private::enabled_listened(HookId::AddPackFinders) {
        return Vec::new();
    }
    let mut registry = Registry::new(kind, mods.clone());
    if matches!(kind, PackType::ClientResources | PackType::ServerData) {
        mod_api::dispatch_add_pack_finders_for(
            &mut StartupDiscovery,
            |listen| registry.register_listener(listen),
            |id| mods.code_owner_enabled(id),
        );
    }
    registry.packs
}

pub fn data_packs(mods: &ModsState) -> Vec<std::sync::Arc<DataPack>> {
    discover(PackType::ServerData, mods)
        .into_iter()
        .map(|pack| data_pack(pack.meta))
        .collect()
}

pub fn data_pack(pack: PackMeta) -> std::sync::Arc<DataPack> {
    let mut data = DataPack::from_source(pack.id, pack.source.open().expect("retained source"))
        .with_filter(pack.filter);
    data.selection = pack.selection;
    std::sync::Arc::new(data)
}

/// The same repository is used by commands, content loading and world generation.
pub fn data_repository(
    world: &std::path::Path,
    resources: &std::path::Path,
    additional: &[std::sync::Arc<DataPack>],
) -> Vec<DataPack> {
    data_repository_in(&world.join("datapacks"), resources, additional)
}

/// [`data_repository`] over a pack folder itself (Create World's `mcworld-` folder).
pub fn data_repository_in(
    folder: &std::path::Path,
    resources: &std::path::Path,
    additional: &[std::sync::Arc<DataPack>],
) -> Vec<DataPack> {
    let mut packs = sunrise_level::datapack::discover_folder_with_features(folder, resources);
    packs.extend(additional.iter().map(|pack| (**pack).clone()));
    packs.sort_by(|a, b| sunrise_level::datapack::compare_ids(&a.id, &b.id));
    packs
}
