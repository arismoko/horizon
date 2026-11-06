//! Dimensions a native mod registers: a level with its own build height and
//! generator, as [`register_dimension`](super::Registrar::register_dimension)
//! hands it over once at startup. Data packs make dimensions with
//! `data/<ns>/dimension` and `dimension_type` files; a registered one becomes
//! those same two entries, below every data pack, so a pack can replace it.
use horizon_world::worldgen::overrides::ModDimension;
use serde_json::{Value, json};

/// The generator every level has without a mod's own: vanilla's
/// `NoiseBasedChunkGenerator`.
pub(crate) const NOISE_GENERATOR: &str = "minecraft:noise";

/// What a mod's dimension is made of. The default is the Overworld's shape
/// and terrain over plains; the parts of a dimension type not named here are
/// the Overworld's.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: mods build it themselves as register_dimension's settings
pub struct DimensionSettings {
    /// The lowest block's y: a multiple of 16, from -2032.
    pub min_y: i32,
    /// The build height: a multiple of 16, at least 16, reaching no higher than 2031.
    pub height: i32,
    /// How far up portals and chorus fruit reach, at most `height`.
    pub logical_height: i32,
    /// Whether the sky lights the level.
    pub has_skylight: bool,
    /// Whether the level has a bedrock ceiling, as the Nether does.
    pub has_ceiling: bool,
    /// The chunk generator's id: `minecraft:noise`.
    pub generator: String,
    /// The noise settings' id (`minecraft:overworld`, or a data pack's).
    pub noise_settings: String,
    /// The one biome the level has.
    pub biome: String,
}

impl Default for DimensionSettings {
    fn default() -> Self {
        Self {
            min_y: -64,
            height: 384,
            logical_height: 384,
            has_skylight: true,
            has_ceiling: false,
            generator: NOISE_GENERATOR.into(),
            noise_settings: "minecraft:overworld".into(),
            biome: "minecraft:plains".into(),
        }
    }
}

impl DimensionSettings {
    /// The dimension type's fields these settings set.
    fn dimension_type(&self) -> Value {
        json!({
            "min_y": self.min_y,
            "height": self.height,
            "logical_height": self.logical_height,
            "has_skylight": self.has_skylight,
            "has_ceiling": self.has_ceiling,
        })
    }
    /// Whether the dimension type's codec accepts the build height.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn valid_shape(&self) -> bool {
        horizon_world::worldgen::overrides::LevelHeight::of_json(&self.dimension_type()).is_some()
    }
}

/// A registered dimension: its namespaced id (`modid:pocket`), the mod that
/// registered it and its settings.
pub(crate) struct DimensionInfo {
    pub(crate) mod_id: &'static str,
    pub(crate) id: Box<str>,
    pub(crate) settings: DimensionSettings,
}

impl DimensionInfo {
    /// The level stem and dimension type it stands for.
    fn entries(&self) -> ModDimension {
        let settings = &self.settings;
        let stem = json!({
            "type": &*self.id,
            "generator": {
                "type": settings.generator,
                "settings": settings.noise_settings,
                "biome_source": {"type": "minecraft:fixed", "biome": settings.biome},
            },
        });
        ModDimension {
            id: self.id.to_string(),
            stem: stem.to_string(),
            dimension_type: settings.dimension_type().to_string(),
        }
    }
}

/// The dimensions the mods registered at startup, as the engine reads them.
/// Empty before the parts are installed.
pub fn installed_dimensions() -> Vec<ModDimension> {
    super::parts::installed_dimension_list()
        .iter()
        .map(DimensionInfo::entries)
        .collect()
}
