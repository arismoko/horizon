use super::*;

/// `WorldOptions.isOldCustomizedWorld`: the generator settings keep `legacy_custom_options`.
pub fn is_old_customized_world(world: &Path) -> bool {
    world_gen_settings(world).is_some_and(|s| text(s.get("legacy_custom_options")).is_some())
}
/// A dimension's `LevelStem` field read as a resource location (an inline definition is not one).
fn stem_id(tag: Option<&Tag>) -> Option<String> {
    let id = text(tag)?;
    Some(if id.contains(':') {
        id
    } else {
        format!("minecraft:{id}")
    })
}
/// `WorldDimensions.isVanillaLike`: whether a saved dimension is the stable Overworld, Nether or
/// End (`isStableOverworld`, `isStableNether`, `isStableEnd`); any other key never is.
fn dimension_is_vanilla_like(key: &str, dimension: &Tag) -> bool {
    let kind = stem_id(dimension.get("type"));
    let generator = dimension.get("generator");
    let source = generator.and_then(|g| g.get("biome_source"));
    let source_kind = source.and_then(|s| stem_id(s.get("type")));
    // MultiNoiseBiomeSource.stable(preset): a reference to the preset, not its parameters.
    let multi_noise_preset = |preset: &str| {
        source_kind.as_deref() == Some("minecraft:multi_noise")
            && stem_id(source.and_then(|s| s.get("preset"))).as_deref() == Some(preset)
    };
    // NoiseBasedChunkGenerator.stable(settings): the generator uses the settings by key.
    let noise_settings = |settings: &str| {
        generator.is_some_and(|g| stem_id(g.get("type")).as_deref() == Some("minecraft:noise"))
            && stem_id(generator.and_then(|g| g.get("settings"))).as_deref() == Some(settings)
    };
    match key {
        "minecraft:overworld" => {
            matches!(
                kind.as_deref(),
                Some("minecraft:overworld" | "minecraft:overworld_caves")
            ) && (source_kind.as_deref() != Some("minecraft:multi_noise")
                || multi_noise_preset("minecraft:overworld"))
        }
        "minecraft:the_nether" => {
            kind.as_deref() == Some("minecraft:the_nether")
                && noise_settings("minecraft:nether")
                && multi_noise_preset("minecraft:nether")
        }
        "minecraft:the_end" => {
            kind.as_deref() == Some("minecraft:the_end")
                && noise_settings("minecraft:end")
                && source_kind.as_deref() == Some("minecraft:the_end")
        }
        _ => false,
    }
}

/// The merged view of the data packs `world` selects, for a stem check made before the world
/// opens and its load builds its own view (`WorldOpenFlows.openWorldCheckWorldStemCompatibility`).
pub fn world_data_packs(world: &Path) -> crate::datapack::DataPacks {
    let packs = crate::datapack::discover(world);
    let enabled_features = Level::read(world)
        .map(|l| l.enabled_features)
        .unwrap_or_default();
    let enabled = crate::datapack::enabled_ids(world, &packs, &enabled_features);
    let enabled: Vec<&crate::datapack::DataPack> = enabled
        .iter()
        .filter_map(|id| packs.iter().find(|p| &p.id == id))
        .collect();
    crate::datapack::DataPacks::new(&enabled)
}

/// The level stems the world's enabled data packs define, as `WorldOpenFlows.loadWorldStem`'s
/// `datapackDimensions`.
fn datapack_dimensions(data: &crate::datapack::DataPacks) -> Vec<(String, Tag)> {
    crate::datapack::dimension_stems(data)
        .into_iter()
        .map(|(id, json)| (id, json_tag(&json)))
        .collect()
}
/// `data.worldGenSettingsLifecycle() != Lifecycle.stable()` (`WorldDimensions.bake`): the known
/// dimensions (the saved ones and the enabled data packs', a pack's replacing the saved one of
/// its id) lack one of the three builtin ones, or have one that is not the vanilla one.
pub fn world_gen_unstable(world: &Path, data: &crate::datapack::DataPacks) -> bool {
    let saved = world_gen_settings(world)
        .as_ref()
        .and_then(|s| s.get("dimensions"))
        .cloned();
    let stems = datapack_dimensions(data);
    if saved.is_none() && stems.is_empty() {
        return false;
    }
    let mut dimensions: std::collections::BTreeMap<String, Tag> = match saved {
        Some(Tag::Compound(saved)) => saved
            .into_iter()
            .map(|(key, dimension)| (key.to_string_lossy(), dimension))
            .collect(),
        _ => std::collections::BTreeMap::new(),
    };
    // `baseDimensions.getOptional(key).or(saved)`.
    dimensions.extend(stems);
    let builtin = [
        "minecraft:overworld",
        "minecraft:the_nether",
        "minecraft:the_end",
    ];
    builtin.iter().any(|key| !dimensions.contains_key(*key))
        || dimensions
            .iter()
            .any(|(key, dimension)| !dimension_is_vanilla_like(key, dimension))
}

/// The generator settings used for this world session. A replacement from
/// enabled packs is kept in memory until the first save.
#[derive(Clone, Debug)]
pub struct WorldGenerator {
    pub(super) data: Tag,
    pub(super) recovered: bool,
}

impl WorldGenerator {
    pub fn settings(&self) -> &Tag {
        &self.data
    }

    pub(super) fn read(world: &Path, level_data: Option<&Tag>) -> (Option<Self>, i64) {
        use sunrise_world::datafix::{files, schema::Ty, update_saved_data};
        // Before file fixes (world previews), project the old settings into the
        // current layout. During startup the migration has already run.
        let old = level_data
            .and_then(|data| number(data.get("DataVersion")))
            .and_then(|version| i32::try_from(version).ok())
            .is_some_and(files::requires_file_fixing);
        let candidates = [
            old.then(|| files::current_world_gen_settings(world).ok().flatten())
                .flatten(),
            read_dat(&saved_data_path(world, "world_gen_settings")).map(|mut root| {
                update_saved_data(Ty::SavedWorldGenSettings, &mut root);
                root.get("data")
                    .cloned()
                    .unwrap_or(Tag::Compound(Default::default()))
            }),
            level_data.and_then(|data| data.get("WorldGenSettings").cloned()),
        ];
        let usable = |s: &Tag| {
            number(s.get("seed")).is_some()
                && s.get("dimensions").is_some_and(valid_world_gen_dimensions)
        };
        // Old summaries may still display a seed from level.dat even if its
        // settings are unusable. A dedicated world only uses a validated generator.
        let displayed_seed = candidates
            .iter()
            .flatten()
            .find_map(|s| number(s.get("seed")))
            .unwrap_or(0);
        let generator = candidates
            .into_iter()
            .flatten()
            .find(usable)
            .map(|data| Self {
                data,
                recovered: false,
            });
        let seed = generator
            .as_ref()
            .and_then(|g| number(g.data.get("seed")))
            .unwrap_or(displayed_seed);
        (generator, seed)
    }

    /// Read the prepared settings for a newly created world, which does not
    /// have level.dat yet.
    pub fn read_new(world: &Path) -> Option<Self> {
        Self::read(world, None).0
    }
}

/// The world's usable generator settings, for previews outside the world session.
pub fn world_gen_settings(world: &Path) -> Option<Tag> {
    let root = read_level_dat(world);
    WorldGenerator::read(world, root.as_ref().and_then(|r| r.get("Data")))
        .0
        .map(|generator| generator.data)
}

/// The world presets of 26.3's `worldgen/world_preset` that the create
/// screen offers, in the order of `WorldPresetTags.NORMAL`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldPreset {
    Normal,
    Flat,
    LargeBiomes,
    Amplified,
    SingleBiomeSurface,
    /// `minecraft:debug_all_block_states`: a grid of every block state
    /// (`DebugLevelSource`). The create screen offers it only with Alt held;
    /// a data pack, `level-type` or a saved world names it too.
    Debug,
}
impl WorldPreset {
    /// The presets, in the order the world-type button cycles through them.
    pub const ALL: [Self; 5] = [
        Self::Normal,
        Self::Flat,
        Self::LargeBiomes,
        Self::Amplified,
        Self::SingleBiomeSurface,
    ];
    /// `WorldPresetTags.EXTENDED`: the list the button cycles while Alt is held.
    pub const EXTENDED: [Self; 6] = [
        Self::Normal,
        Self::Flat,
        Self::LargeBiomes,
        Self::Amplified,
        Self::SingleBiomeSurface,
        Self::Debug,
    ];
    /// `CycleButton.cycleValue`: the preset `step` places on in the Alt
    /// list (`EXTENDED`) or the normal one (`ALL`). The button's position
    /// is the preset's place in the extended list, which the normal list
    /// then wraps (`Mth.positiveModulo`).
    pub fn step(self, step: i32, alt: bool) -> Self {
        let index = Self::EXTENDED.iter().position(|p| *p == self).unwrap_or(0);
        let list: &[Self] = if alt { &Self::EXTENDED } else { &Self::ALL };
        list[(index as i32 + step).rem_euclid(list.len() as i32) as usize]
    }
    /// The preset's registry id (`minecraft:normal`).
    pub const fn id(self) -> &'static str {
        match self {
            Self::Normal => "minecraft:normal",
            Self::Flat => "minecraft:flat",
            Self::LargeBiomes => "minecraft:large_biomes",
            Self::Amplified => "minecraft:amplified",
            Self::SingleBiomeSurface => "minecraft:single_biome_surface",
            Self::Debug => "minecraft:debug_all_block_states",
        }
    }
    /// The preset an id names, if it is one of them.
    pub fn of_id(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .chain([Self::Debug])
            .find(|preset| preset.id() == id)
    }
    /// The language key the world-type button shows
    /// (`Identifier.toLanguageKey("generator")`).
    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "generator.minecraft.normal",
            Self::Flat => "generator.minecraft.flat",
            Self::LargeBiomes => "generator.minecraft.large_biomes",
            Self::Amplified => "generator.minecraft.amplified",
            Self::SingleBiomeSurface => "generator.minecraft.single_biome_surface",
            Self::Debug => "generator.minecraft.debug_all_block_states",
        }
    }
    /// The preset the world-type button cycles to next.
    pub fn next(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|preset| *preset == self)
            .unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }
    /// The preset the world-type button cycles to previously.
    pub fn prev(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|preset| *preset == self)
            .unwrap_or(0);
        Self::ALL[(index + Self::ALL.len() - 1) % Self::ALL.len()]
    }
    /// The preset a saved overworld generator is: the noise settings name
    /// the preset, and the single-biome preset shares the overworld's noise
    /// settings, so its fixed biome source tells the two apart.
    pub fn of_generator(generator: &Tag) -> Self {
        if text(generator.get("type")).as_deref() == Some("minecraft:flat") {
            return Self::Flat;
        }
        if text(generator.get("type")).as_deref() == Some("minecraft:debug") {
            return Self::Debug;
        }
        let source = generator.get("biome_source");
        let fixed = text(source.and_then(|s| s.get("type"))).as_deref() == Some("minecraft:fixed");
        match text(generator.get("settings")).as_deref() {
            Some("minecraft:large_biomes") => Self::LargeBiomes,
            Some("minecraft:amplified") => Self::Amplified,
            Some("minecraft:overworld") if fixed => Self::SingleBiomeSurface,
            _ => Self::Normal,
        }
    }
}
/// World and player state stored in `level.dat`, the saved-data files and
/// the player file.
/// The world options CreateWorldScreen sets (`WorldOptions` and the world
/// preset) that `world_gen_settings` keeps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldGen {
    /// The world preset the world is created with.
    pub preset: WorldPreset,
    pub structures: bool,
    pub bonus_chest: bool,
}
impl WorldGen {
    /// A saved world's choices, from its `world_gen_settings`.
    pub fn read(world: &Path) -> Option<Self> {
        world_gen_settings(world).as_ref().map(Self::from_settings)
    }

    /// Options of the generator selected by the world loader.
    pub fn from_settings(settings: &Tag) -> Self {
        let flag = |key: &str, default: bool| number(settings.get(key)).map_or(default, |v| v != 0);
        let generator = settings
            .get("dimensions")
            .and_then(|d| d.get("minecraft:overworld"))
            .and_then(|o| o.get("generator"));
        Self {
            preset: generator.map_or(WorldPreset::Normal, WorldPreset::of_generator),
            structures: flag("generate_structures", true),
            bonus_chest: flag("bonus_chest", false),
        }
    }
    /// The bundled world preset a saved world's overworld was generated
    /// with, from its generator: `minecraft:amplified` and
    /// `minecraft:large_biomes` by their noise settings, and
    /// `minecraft:single_biome_surface` by a fixed biome source. Anything
    /// else (and any world without settings) is `minecraft:normal`.
    pub fn noise_preset(settings: Option<&Tag>) -> &'static str {
        let generator = settings
            .and_then(|s| s.get("dimensions"))
            .and_then(|d| d.get("minecraft:overworld"))
            .and_then(|o| o.get("generator"));
        let noise = text(generator.and_then(|g| g.get("settings")));
        let fixed_biome = generator
            .and_then(|g| g.get("biome_source"))
            .and_then(|b| text(b.get("type")))
            .is_some_and(|t| t == "minecraft:fixed");
        match noise.as_deref() {
            Some("minecraft:amplified") => "minecraft:amplified",
            Some("minecraft:large_biomes") => "minecraft:large_biomes",
            Some("minecraft:overworld") if fixed_biome => "minecraft:single_biome_surface",
            _ => "minecraft:normal",
        }
    }
}
impl Default for WorldGen {
    fn default() -> Self {
        Self {
            preset: WorldPreset::Normal,
            structures: true,
            bonus_chest: false,
        }
    }
}
/// Writes a new world's `world_gen_settings` (seed, options and dimensions).
pub fn write_world_gen_settings(dir: &Path, seed: i64, options: WorldGen) -> Result<()> {
    write_world_gen_settings_with(dir, seed, options, None, None, None)
}

pub(super) const VANILLA_FEATURE_FLAGS: [&str; 1] = ["minecraft:vanilla"];
/// The effective feature flags stored by a level. Fresh and defaulted saves have vanilla's set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeatureFlagIds(Arc<[String]>);

impl Default for FeatureFlagIds {
    fn default() -> Self {
        static VANILLA: std::sync::LazyLock<Arc<[String]>> =
            std::sync::LazyLock::new(|| VANILLA_FEATURE_FLAGS.map(str::to_owned).into());
        Self(Arc::clone(&VANILLA))
    }
}

impl FeatureFlagIds {
    /// Share the flag ids with views of this level without copying them.
    pub fn as_arc(&self) -> &Arc<[String]> {
        &self.0
    }
}

impl std::ops::Deref for FeatureFlagIds {
    type Target = [String];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: Into<Arc<[String]>>> From<T> for FeatureFlagIds {
    fn from(ids: T) -> Self {
        Self(ids.into())
    }
}

impl FromIterator<mod_api::FeatureFlag> for FeatureFlagIds {
    fn from_iter<T: IntoIterator<Item = mod_api::FeatureFlag>>(flags: T) -> Self {
        let mut ids = Vec::new();
        for flag in flags {
            let id = flag.id();
            if !ids.iter().any(|present| present == id) {
                ids.push(id.to_owned());
            }
        }
        Self(ids.into())
    }
}
