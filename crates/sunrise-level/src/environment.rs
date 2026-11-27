//! A dimension's environment attributes (`EnvironmentAttributeSystem.addDefaultLayers`): the
//! dimension type's attributes, the biome layer, the dimension's timelines and, where it can
//! have weather, the rain and thunder layers, sampled into the sky state the renderer draws.
use crate::Result;
use crate::pack_tags::Tags;
use serde_json::Value as Json;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, LazyLock, OnceLock, RwLock};
use sunrise_assets::resource_source::ResourceSource;
use sunrise_world::environment_attributes::{
    AMBIENT_PARTICLES, Attribute, AttributeMap, Value, attribute,
};
use sunrise_world::environment_system::{BiomeBlend, Builder, Conditions, System};
use sunrise_world::environment_timeline::Timeline;

/// Optional load-time attribute patcher implemented by client `PackModifiers`.
/// Invoked only during `Environment` construction; never called per sample.
pub trait AttributeModifiers: Send + Sync {
    fn apply_dimension(
        &self,
        dimension_id: &str,
        tags: &Tags,
        frame: &Arc<RwLock<Frame>>,
        dimension_map: &mut AttributeMap,
        rain: &mut AttributeMap,
        thunder: &mut AttributeMap,
        dimension_post: &mut AttributeMap,
    );

    fn apply_biome(
        &self,
        biome_id: &str,
        tags: &Tags,
        frame: &Arc<RwLock<Frame>>,
        biome_map: &mut AttributeMap,
        post_map: &mut AttributeMap,
    );
}

pub struct Environment {
    system: System,
    /// What the packs' expressions read; the frame sets it before sampling.
    frame: Arc<RwLock<Frame>>,
    /// Biome attribute maps by biome id, where resource packs changed them; the biome layer
    /// reads these instead of the biome's own.
    biomes: HashMap<String, AttributeMap>,
    /// Packs' biome `post_process` maps by biome id: a biome layer after the weather.
    post: HashMap<String, AttributeMap>,
    /// The data source and selected packs used to rebuild a level's environment.
    source: ResourceSource,
    dimension: String,
    modifiers: Option<Arc<dyn AttributeModifiers>>,
}

/// A world's `minecraft:timeline` registry and its bound `minecraft:timeline` tags, supplied
/// either by the server's world tables or by the client's synchronized registries.
#[derive(Clone, Default)]
pub struct TimelineTable {
    pub timelines: BTreeMap<String, Arc<Timeline>>,
    pub tags: BTreeMap<String, Vec<String>>,
}

impl TimelineTable {
    /// Reads the timelines of a resource source (standalone dimension loads and parity probes).
    pub fn from_source(root: &ResourceSource) -> Self {
        let mut timelines = BTreeMap::new();
        for file in root.list_under("data", &["timeline"]) {
            let Some((namespace, path)) = file
                .strip_prefix("data/")
                .and_then(|s| s.split_once("/timeline/"))
                .and_then(|(namespace, s)| Some((namespace, s.strip_suffix(".json")?)))
            else {
                continue;
            };
            let id = format!("{namespace}:{path}");
            match root
                .read(&file)
                .map_err(|e| e.to_string())
                .and_then(|bytes| serde_json::from_slice::<Json>(&bytes).map_err(|e| e.to_string()))
                .and_then(|t| Timeline::parse(&t))
            {
                Ok(timeline) => {
                    timelines.insert(id, Arc::new(timeline));
                }
                Err(e) => log::warn!("timeline {id}: {e}"),
            }
        }
        let tags = crate::pack_tags::bind(root, "timeline", |id| timelines.contains_key(id))
            .into_iter()
            .collect();
        Self { timelines, tags }
    }
}

struct LevelEntry {
    attributes: Json,
    timelines: Arc<TimelineTable>,
    made: OnceLock<Option<Environment>>,
}

/// The dimension types (their `attributes` and `timelines`) and world timeline table of every
/// level in the open world, by level id, and the environment each makes the first time it is asked
/// for (`None` when its attributes are refused, which falls back to the base environment).
type LevelAttributes = HashMap<String, &'static LevelEntry>;
static LEVELS: LazyLock<RwLock<LevelAttributes>> = LazyLock::new(Default::default);

#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Sky {
    pub sky_color: [f32; 3],
    pub fog_color: [f32; 3],
    pub sky_light_factor: f32,
    pub sky_light_color: [f32; 3],
    pub block_light_tint: [f32; 3],
    pub ambient_light_color: [f32; 3],
    pub sun_angle: f32,
    pub moon_angle: f32,
    pub star_angle: f32,
    pub star_brightness: f32,
    pub sunrise_color: [f32; 4],
    pub cloud_color: [f32; 4],
    /// Raw `gameplay/sky_light_level` (0 to 15), before conversion to sky darkening.
    pub sky_light_level: f32,
    /// Environmental fog (`visual/fog_start_distance` / `fog_end_distance`).
    pub fog_start: f32,
    pub fog_end: f32,
    /// `visual/sky_fog_end_distance`.
    #[serde(default)]
    pub sky_fog_end: f32,
    /// `visual/cloud_fog_end_distance`.
    pub cloud_fog_end: f32,
    /// `visual/cloud_height`.
    pub cloud_height: f32,
}

/// Where and when the sky is sampled.
#[derive(Clone, Copy, Default)]
pub struct Sample<'a> {
    /// The day clock's total ticks.
    pub day_time: i64,
    /// `Level.getRainLevel` and `getThunderLevel`, 0 where the dimension has no weather.
    pub rain: f32,
    pub thunder: f32,
    /// The biomes around the camera (`EnvironmentAttributeProbe`'s Gaussian mix).
    pub blend: Option<&'a BiomeBlend<'a>>,
    /// The biome at the camera, for attributes that don't blend.
    pub biome: Option<&'a AttributeMap>,
    /// The same for the packs' biome post-process group.
    pub post_blend: Option<&'a BiomeBlend<'a>>,
    pub post_biome: Option<&'a AttributeMap>,
}

/// What a pack's expression can ask about the frame (wiki "Scripting Expressions": the global,
/// player, camera and positional contexts), set before the frame samples the sky. `clone_from`
/// keeps the destination's strings, so a context copied into every frame allocates nothing once
/// its strings have room for the names.
#[derive(Debug, Default)]
#[cfg_attr(test, derive(PartialEq))]
pub struct Frame {
    pub rain: f32,
    pub thunder: f32,
    pub game_time: i64,
    pub day_time: i64,
    /// The render distance in chunks.
    pub render_distance: i32,
    /// `skyType`: 1 for the overworld's sky, 2 for the End's, 0 for none.
    pub sky_type: i32,
    pub dimension: String,
    pub player: [f64; 3],
    /// The player's motion per tick (`p.speedSq`).
    pub player_velocity: [f64; 3],
    pub player_on_ground: bool,
    pub in_water: bool,
    pub camera: [f64; 3],
    /// The sky and block light at the camera.
    pub sky_light: i32,
    pub block_light: i32,
    /// The camera's biome (`o.biome()`), and its climate (`o.temperature`, `o.downfall`).
    pub biome: String,
    pub temperature: f32,
    pub downfall: f32,
}

impl Clone for Frame {
    fn clone(&self) -> Self {
        let mut frame = Self::default();
        frame.clone_from(self);
        frame
    }

    /// Every field copied, the strings into the destination's own buffers.
    fn clone_from(&mut self, source: &Self) {
        let Self {
            rain,
            thunder,
            game_time,
            day_time,
            render_distance,
            sky_type,
            dimension,
            player,
            player_velocity,
            player_on_ground,
            in_water,
            camera,
            sky_light,
            block_light,
            biome,
            temperature,
            downfall,
        } = source;
        self.dimension.clone_from(dimension);
        self.biome.clone_from(biome);
        (self.rain, self.thunder, self.game_time, self.day_time) =
            (*rain, *thunder, *game_time, *day_time);
        (self.render_distance, self.sky_type) = (*render_distance, *sky_type);
        (self.player, self.player_velocity, self.player_on_ground) =
            (*player, *player_velocity, *player_on_ground);
        (self.in_water, self.camera, self.sky_light, self.block_light) =
            (*in_water, *camera, *sky_light, *block_light);
        (self.temperature, self.downfall) = (*temperature, *downfall);
    }
}

impl Frame {
    /// `g.rain`: the mean of the rain and thunder levels, so plain rain is 0.5 and a
    /// thunderstorm 1 (observed on the real mod: a pack's `g.rain` colormap row).
    pub fn global_rain(&self) -> f64 {
        (f64::from(self.rain) + f64::from(self.thunder)) / 2.0
    }
}

/// The dimension's timeline selection, applying ordered tag members in place from the world's
/// bound `minecraft:timeline` tags.
fn timeline_ids(tags: &BTreeMap<String, Vec<String>>, json: &Json) -> Vec<String> {
    let entries = match json {
        Json::Array(items) => items.as_slice(),
        entry => std::slice::from_ref(entry),
    };
    let mut out = Vec::new();
    for entry in entries {
        let Some(id) = sunrise_world::tag_layers::entry_id(entry) else {
            continue;
        };
        let removes = entry["remove"].as_bool() == Some(true);
        let direct;
        let members = if let Some(tag) = id.strip_prefix('#') {
            let Ok(tag) = sunrise_core::identifier::Identifier::parse(tag) else {
                continue;
            };
            let Some(members) = tags.get(tag.to_string().as_str()) else {
                continue;
            };
            members.as_slice()
        } else {
            let Ok(parsed) = sunrise_core::identifier::Identifier::parse(id) else {
                log::warn!("timeline {id}: not an identifier");
                continue;
            };
            direct = parsed.to_string();
            std::slice::from_ref(&direct)
        };
        for member in members {
            if removes {
                out.retain(|kept| kept != member);
            } else if !out.contains(member) {
                out.push(member.clone());
            }
        }
    }
    out
}

impl Environment {
    pub fn load(root: &ResourceSource) -> Result<Self> {
        Self::load_dimension(root, "overworld")
    }

    /// `EnvironmentAttributeSystem.addDefaultLayers` for a dimension type: its attributes, a
    /// biome layer for every attribute some biome sets, its timelines (in tag order), and the
    /// weather where `Level.canHaveWeather` (sky light, no ceiling, not the End).
    pub fn load_dimension(root: &ResourceSource, dimension: &str) -> Result<Self> {
        let timelines = TimelineTable::from_source(root);
        Self::load_dimension_attributes(root, dimension, None, &Json::Null, &timelines)
    }

    /// The same with an optional load-time attribute modifier (e.g. resource packs' Polytone
    /// modifiers): invoked only during construction, never per sample.
    pub fn load_dimension_with_modifiers(
        root: &ResourceSource,
        dimension: &str,
        modifiers: Option<Arc<dyn AttributeModifiers>>,
    ) -> Result<Self> {
        let timelines = TimelineTable::from_source(root);
        Self::load_dimension_attributes(root, dimension, modifiers, &Json::Null, &timelines)
    }

    /// The same with resource packs' Polytone modifiers (wiki "Environment Attributes"): a
    /// dimension modifier's `base` changes the dimension's own entries, `rain` and `thunder`
    /// the weather's, `post_process` adds a layer after all others; a biome modifier's `base`
    /// changes the biome's entries, `post_process` adds a biome layer after the weather.
    /// Modifiers apply lowest priority first, so the highest wins.
    pub fn load_dimension_with<M: AttributeModifiers + Clone + 'static>(
        root: &ResourceSource,
        dimension: &str,
        packs: &M,
    ) -> Result<Self> {
        Self::load_dimension_with_modifiers(root, dimension, Some(Arc::new(packs.clone())))
    }

    /// Builds the data attributes first, then applies any load-time modifiers.
    fn load_dimension_attributes(
        root: &ResourceSource,
        dimension: &str,
        modifiers: Option<Arc<dyn AttributeModifiers>>,
        extra: &Json,
        timelines: &TimelineTable,
    ) -> Result<Self> {
        let mut json: Json = serde_json::from_slice(
            &root.read(&format!("data/minecraft/dimension_type/{dimension}.json"))?,
        )?;
        if let Some(attributes) = extra["attributes"].as_object() {
            if !json["attributes"].is_object() {
                json["attributes"] = Json::Object(Default::default());
            }
            if let Some(own) = json["attributes"].as_object_mut() {
                own.extend(attributes.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
        }
        if !extra["timelines"].is_null() {
            json["timelines"] = extra["timelines"].clone();
        }
        let id = format!("minecraft:{dimension}");
        let tags = modifiers.as_ref().map(|_| Tags::new(root));
        let frame = Arc::new(RwLock::new(Frame::default()));
        let mut dimension_map = AttributeMap::parse(&json["attributes"])?;
        let (mut rain, mut thunder) = sunrise_world::environment_system::weather_maps();
        let mut dimension_post = AttributeMap::default();
        if let (Some(modifiers), Some(tags)) = (modifiers.as_deref(), tags.as_ref()) {
            modifiers.apply_dimension(
                &id,
                tags,
                &frame,
                &mut dimension_map,
                &mut rain,
                &mut thunder,
                &mut dimension_post,
            );
        }
        let mut builder = Builder::default();
        builder.constant_layer(&dimension_map);
        // Without biome data (a bare dimension type), there is no biome layer.
        let mut biomes = HashMap::new();
        let mut post = HashMap::new();
        for file in root.list_under("data", &["worldgen/biome"]) {
            let Some((namespace, name)) = file
                .strip_prefix("data/")
                .and_then(|s| s.split_once("/worldgen/biome/"))
                .and_then(|(namespace, s)| Some((namespace, s.strip_suffix(".json")?)))
            else {
                continue;
            };
            // One unreadable biome file is skipped with a warning, not the whole environment.
            let biome = match root
                .read(&file)
                .map_err(|e| e.to_string())
                .and_then(|bytes| serde_json::from_slice::<Json>(&bytes).map_err(|e| e.to_string()))
            {
                Ok(biome) => biome,
                Err(e) => {
                    log::warn!("biome {file}: {e}");
                    continue;
                }
            };
            let name = format!("{namespace}:{name}");
            let mut map = match AttributeMap::parse(&biome["attributes"]) {
                Ok(map) => map,
                Err(e) => {
                    log::warn!("biome {}: {e}", file);
                    continue;
                }
            };
            let mut post_map = AttributeMap::default();
            if let (Some(modifiers), Some(tags)) = (modifiers.as_deref(), tags.as_ref()) {
                modifiers.apply_biome(&name, tags, &frame, &mut map, &mut post_map);
            }
            if !post_map.is_empty() {
                post.insert(name.clone(), post_map);
            }
            biomes.insert(name, map);
        }
        builder.biome_layer(biomes.values());
        // Ambient emission uses the active level's registry, including synchronized biomes.
        builder
            .biome_attribute(attribute(AMBIENT_PARTICLES).expect("registered ambient particles"));
        for id in timeline_ids(&timelines.tags, &json["timelines"]) {
            if let Some(timeline) = timelines.timelines.get(&id) {
                builder.timeline_layer(timeline);
            } else {
                log::warn!("timeline {id}: not in world timeline table");
            }
        }
        let weather = json["has_skylight"].as_bool() == Some(true)
            && json["has_ceiling"].as_bool() != Some(true)
            && dimension != "the_end";
        if weather {
            builder.weather_layers_with(&rain, &thunder);
        }
        builder.biome_post_layer(post.values());
        builder.constant_layer(&dimension_post);
        Ok(Self {
            system: builder.build(),
            source: root.clone(),
            dimension: dimension.to_owned(),
            modifiers,
            frame,
            biomes,
            post,
        })
    }

    /// Sets what the packs' expressions read for the next samples.
    pub fn set_frame(&self, frame: &Frame) {
        if let Ok(mut f) = self.frame.write() {
            f.clone_from(frame);
        }
    }

    /// A biome's attribute map (by id, `minecraft:plains`), as the packs left it.
    pub fn biome(&self, id: &str) -> Option<&AttributeMap> {
        self.biomes.get(id)
    }

    /// A biome's post-process map from the packs, if any sets one.
    pub fn biome_post(&self, id: &str) -> Option<&AttributeMap> {
        self.post.get(id)
    }

    /// Whether packs added post-process biome layers (the frame then mixes them too).
    pub fn has_biome_post(&self) -> bool {
        !self.post.is_empty()
    }

    /// Replaces every level's dimension type overrides and the world's timeline table, at a
    /// world's load. The table is leaked: one small entry per level and world load.
    pub fn set_level_attributes(levels: Vec<(String, Json)>, timelines: TimelineTable) {
        let timelines = Arc::new(timelines);
        let table = levels
            .into_iter()
            .map(|(id, attributes)| {
                let entry = Box::leak(Box::new(LevelEntry {
                    attributes,
                    timelines: Arc::clone(&timelines),
                    made: OnceLock::new(),
                }));
                (id, &*entry)
            })
            .collect();
        *LEVELS.write().unwrap_or_else(|e| e.into_inner()) = table;
    }

    /// `base` with the attributes and world timelines of `level`, for every level registered at
    /// world open; `base` itself when the level is not registered or its attributes are refused.
    pub fn for_level<'a>(level: &str, base: &'a Environment) -> &'a Environment {
        let entry = LEVELS
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(level)
            .copied();
        let Some(entry) = entry else {
            return base;
        };
        entry
            .made
            .get_or_init(|| {
                Self::load_dimension_attributes(
                    &base.source,
                    &base.dimension,
                    base.modifiers.clone(),
                    &entry.attributes,
                    &entry.timelines,
                )
                .map_err(|e| log::warn!("dimension {level}: attributes refused: {e}"))
                .ok()
            })
            .as_ref()
            .unwrap_or(base)
    }

    /// `base` with the `attributes` and `timelines` of the dimension type `extra` over it, owned,
    /// reading timelines from `timelines` (the server's synced table or the world table): what a
    /// client of a server keeps for the level the server's type describes. A timeline absent from
    /// `timelines` is skipped with a warning. None when the type's attributes are refused.
    pub fn with_type(
        base: &Environment,
        extra: &Json,
        timelines: &TimelineTable,
    ) -> Option<Environment> {
        Self::load_dimension_attributes(
            &base.source,
            &base.dimension,
            base.modifiers.clone(),
            extra,
            timelines,
        )
        .map_err(|e| log::warn!("the server's dimension type: attributes refused: {e}"))
        .ok()
    }

    /// `EnvironmentAttributeSystem.getValue` of one attribute, or its dimension value without
    /// a biome.
    pub fn value(&self, id: &str, at: &Sample) -> Option<Value> {
        self.value_at(attribute(id)?, at, &|_| at.day_time)
    }

    /// Sample a known attribute with the level's clock values.
    pub fn value_at(
        &self,
        attribute: &Attribute,
        at: &Sample,
        clock: &dyn Fn(&str) -> i64,
    ) -> Option<Value> {
        let biome = |_full_resolution: bool| at.biome;
        let post_biome = |_full_resolution: bool| at.post_biome;
        let conditions = Conditions {
            rain: at.rain,
            thunder: at.thunder,
            blend: at.blend,
            biome: &biome,
            clock,
            post_blend: at.post_blend,
            post_biome: Some(&post_biome),
        };
        match (at.blend, at.biome) {
            (None, None) => self.system.dimension_value(attribute, &conditions),
            _ => self.system.value(attribute, &conditions),
        }
    }

    /// The sky state the renderer draws.
    pub fn sample(&self, at: &Sample) -> Sky {
        self.sample_with_clock(at, &|_| at.day_time)
    }

    /// The sky state the renderer draws, reading each timeline's own `WorldClock`.
    pub fn sample_with_clock(&self, at: &Sample, clock: &dyn Fn(&str) -> i64) -> Sky {
        let value = |id: &str| attribute(id).and_then(|attr| self.value_at(attr, at, clock));
        let float = |id: &str| match value(id) {
            Some(Value::Float(f)) => f,
            _ => 0.0,
        };
        let rgb = |id: &str| match value(id) {
            Some(Value::Rgb(c)) => c,
            _ => [0.0; 3],
        };
        let argb = |id: &str| match value(id) {
            Some(Value::Argb(c)) => c,
            _ => [0.0; 4],
        };
        Sky {
            sky_color: rgb("visual/sky_color"),
            fog_color: rgb("visual/fog_color"),
            sky_light_factor: float("visual/sky_light_factor"),
            sky_light_color: rgb("visual/sky_light_color"),
            block_light_tint: rgb("visual/block_light_tint"),
            ambient_light_color: rgb("visual/ambient_light_color"),
            sun_angle: float("visual/sun_angle"),
            moon_angle: float("visual/moon_angle"),
            star_angle: float("visual/star_angle"),
            star_brightness: float("visual/star_brightness"),
            sunrise_color: argb("visual/sunrise_sunset_color"),
            cloud_color: argb("visual/cloud_color"),
            sky_light_level: float("gameplay/sky_light_level"),
            fog_start: float("visual/fog_start_distance"),
            fog_end: float("visual/fog_end_distance"),
            sky_fog_end: float("visual/sky_fog_end_distance"),
            cloud_fog_end: float("visual/cloud_fog_end_distance"),
            cloud_height: float("visual/cloud_height"),
        }
    }
}

/// `Level.updateSkyBrightness`'s darkening: from the sky light level, a dimension value.
pub fn sky_darken(environment: &Environment, day_time: i64, rain: f32, thunder: f32) -> i32 {
    sky_darken_with_clock(environment, rain, thunder, &|_| day_time)
}

/// `Level.updateSkyBrightness`'s darkening, reading each timeline's own `WorldClock`.
pub fn sky_darken_with_clock(
    environment: &Environment,
    rain: f32,
    thunder: f32,
    clock: &dyn Fn(&str) -> i64,
) -> i32 {
    let level = match attribute("gameplay/sky_light_level").and_then(|attr| {
        environment.value_at(
            attr,
            &Sample {
                rain,
                thunder,
                ..Sample::default()
            },
            clock,
        )
    }) {
        Some(Value::Float(level)) => level,
        _ => 0.0,
    };
    // Truncate the raw light level without a lossy normalize / denormalize round trip.
    (15.0 - level) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Frame::clone_from` copies every field and writes the strings into the destination's own
    /// buffers (their address and capacity kept), so a context copied each frame allocates
    /// nothing; `clone` gives an equal frame. The environment's held frame, which its attribute
    /// expressions read, is written the same way by `set_frame`, every time it is set.
    #[test]
    fn a_frame_copies_into_its_own_strings() {
        let source = Frame {
            rain: 0.25,
            thunder: 0.5,
            game_time: 7,
            day_time: 8,
            render_distance: 9,
            sky_type: 2,
            dimension: "minecraft:the_end".to_owned(),
            player: [1.0, 2.0, 3.0],
            player_velocity: [4.0, 5.0, 6.0],
            player_on_ground: true,
            in_water: false,
            camera: [7.0, 8.0, 9.0],
            sky_light: 10,
            block_light: 11,
            biome: "minecraft:the_void".to_owned(),
            temperature: 0.75,
            downfall: 0.125,
        };
        // Room for the names, and the booleans the other way round from the source's.
        let roomy = || Frame {
            dimension: String::with_capacity(64),
            biome: String::with_capacity(64),
            player_on_ground: false,
            in_water: true,
            ..Default::default()
        };
        let mut frame = roomy();
        let buffers = |f: &Frame| [&f.dimension, &f.biome].map(|s| (s.as_ptr(), s.capacity()));
        let before = buffers(&frame);
        frame.clone_from(&source);
        assert_eq!(buffers(&frame), before);
        assert_eq!(frame, source);
        assert_eq!(source.clone(), source);
        // The environment's own copy, set twice: the second time over the first's names.
        let files = ResourceSource::indexed(std::collections::BTreeMap::from([(
            "data/minecraft/dimension_type/overworld.json".into(),
            sunrise_assets::resource_source::Resource::bytes(
                r#"{"has_skylight": true, "has_ceiling": false, "attributes": {}}"#
                    .as_bytes()
                    .to_vec(),
            ),
        )]));
        let environment = Environment::load_dimension(&files, "overworld").unwrap();
        *environment.frame.write().unwrap() = roomy();
        let before = buffers(&environment.frame.read().unwrap());
        let overworld = Frame {
            dimension: "minecraft:overworld".to_owned(),
            biome: "minecraft:plains".to_owned(),
            ..Default::default()
        };
        environment.set_frame(&overworld);
        environment.set_frame(&source);
        let held = environment.frame.read().unwrap();
        assert_eq!(buffers(&held), before);
        assert_eq!(*held, source);
    }

    /// The level table is global, so the tests that set it take turns.
    static LEVEL_TABLE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn resolved_dimension_data_and_raw_darkening_need_no_files() {
        use sunrise_assets::resource_source::Resource;
        for (level, expected) in [(14.5, 0), (13.5, 1), (4.0, 11)] {
            let source = ResourceSource::indexed(std::collections::BTreeMap::from([(
                "data/minecraft/dimension_type/overworld.json".into(),
                Resource::bytes(
                    format!(r#"{{"attributes":{{"minecraft:gameplay/sky_light_level":{level}}}}}"#)
                        .into_bytes(),
                ),
            )]));
            let environment = Environment::load(&source).unwrap();
            assert_eq!(
                environment.sample(&Sample::default()).sky_light_level,
                level
            );
            assert_eq!(sky_darken(&environment, 0, 0.0, 0.0), expected);
            assert!(Environment::load_dimension(&source, "missing").is_err());
        }
    }

    fn overworld() -> Option<Environment> {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client/26.3");
        root.join("data/minecraft/timeline/day.json")
            .exists()
            .then(|| Environment::load(&ResourceSource::directory(root)).unwrap())
    }

    #[test]
    fn dark_outside_follows_time_and_weather_levels() {
        let Some(env) = overworld() else {
            return;
        };
        let dark = |time, rain, thunder| sky_darken(&env, time, rain, thunder) >= 4;
        assert!(!dark(6000, 0.0, 0.0), "clear noon");
        assert!(!dark(6000, 1.0, 0.0), "rainy noon");
        assert!(dark(6000, 1.0, 1.0), "thunderstorm at noon");
        assert!(dark(18000, 0.0, 0.0), "clear midnight");
        assert!(!dark(1000, 0.0, 0.0), "clear morning");
    }

    fn sky_at(env: &Environment, biome: &str, rain: f32) -> Sky {
        let map = env.biome(biome).unwrap();
        env.sample(&Sample {
            rain,
            biome: Some(map),
            post_biome: env.biome_post(biome),
            ..Sample::default()
        })
    }

    const GRAY_40: f32 = 64.0 / 255.0;

    #[test]
    fn biomes_load_from_every_namespace_with_their_whole_path() {
        use sunrise_assets::resource_source::Resource;
        let file =
            |path: &str, text: &str| (path.to_owned(), Resource::bytes(text.as_bytes().to_vec()));
        let source = ResourceSource::indexed(std::collections::BTreeMap::from([
            file(
                "data/minecraft/dimension_type/overworld.json",
                r##"{"attributes": {"minecraft:visual/sky_color": "#808080"}}"##,
            ),
            file(
                "data/minecraft/worldgen/biome/plains.json",
                r##"{"attributes": {"minecraft:visual/sky_color": "#404040"}}"##,
            ),
            file(
                "data/example/worldgen/biome/test.json",
                r##"{"attributes": {"minecraft:visual/sky_color": "#ff0000"}}"##,
            ),
            file(
                "data/example/worldgen/biome/region/test.json",
                r##"{"attributes": {"minecraft:visual/sky_color": "#00ff00"}}"##,
            ),
        ]));
        let env = Environment::load(&source).unwrap();
        assert_eq!(
            sky_at(&env, "minecraft:plains", 0.0).sky_color,
            [GRAY_40; 3]
        );
        assert!(
            env.biome("example:test").is_some(),
            "example:test never loaded"
        );
        assert_eq!(sky_at(&env, "example:test", 0.0).sky_color, [1.0, 0.0, 0.0]);
        assert!(
            env.biome("example:region/test").is_some(),
            "example:region/test never loaded"
        );
        assert_eq!(
            sky_at(&env, "example:region/test", 0.0).sky_color,
            [0.0, 1.0, 0.0]
        );
    }

    #[test]
    fn a_malformed_biome_file_is_skipped_and_its_neighbours_still_load() {
        use sunrise_assets::resource_source::Resource;
        let file =
            |path: &str, text: &str| (path.to_owned(), Resource::bytes(text.as_bytes().to_vec()));
        let source = ResourceSource::indexed(std::collections::BTreeMap::from([
            file(
                "data/minecraft/dimension_type/overworld.json",
                r##"{"attributes": {"minecraft:visual/sky_color": "#808080"}}"##,
            ),
            file("data/example/worldgen/biome/broken.json", "{ not json"),
            file(
                "data/example/worldgen/biome/test.json",
                r##"{"attributes": {"minecraft:visual/sky_color": "#ff0000"}}"##,
            ),
        ]));
        let env = Environment::load(&source).unwrap();
        assert!(env.biome("example:broken").is_none());
        assert_eq!(sky_at(&env, "example:test", 0.0).sky_color, [1.0, 0.0, 0.0]);
    }

    /// A level's attributes go over the Overworld's one by one; only a registered level gets its
    /// own environment, and the table clears.
    #[test]
    fn only_a_registered_level_gets_its_own_environment() {
        let _turn = LEVEL_TABLE.lock().unwrap_or_else(|e| e.into_inner());
        use sunrise_assets::resource_source::Resource;
        let source = ResourceSource::indexed(std::collections::BTreeMap::from([(
            "data/minecraft/dimension_type/overworld.json".into(),
            Resource::bytes(
                br#"{"attributes":{"minecraft:gameplay/sky_light_level":15.0}}"#.to_vec(),
            ),
        )]));
        let base = Environment::load(&source).unwrap();
        let id = "test:env_pocket";
        let darken = |e: &Environment| sky_darken(e, 0, 0.0, 0.0);
        assert!(std::ptr::eq(Environment::for_level(id, &base), &base));
        Environment::set_level_attributes(
            vec![(
                id.to_owned(),
                serde_json::json!({"attributes": {"minecraft:gameplay/sky_light_level": 4.0}}),
            )],
            TimelineTable::default(),
        );
        let own = Environment::for_level(id, &base);
        assert!(!std::ptr::eq(own, &base));
        assert_eq!((darken(&base), darken(own)), (0, 11));
        assert!(std::ptr::eq(
            Environment::for_level("test:other", &base),
            &base
        ));
        Environment::set_level_attributes(Vec::new(), TimelineTable::default());
        assert!(std::ptr::eq(Environment::for_level(id, &base), &base));
    }

    /// A client's own environment for a server's type: its attributes over the base's, owned.
    #[test]
    fn a_servers_type_makes_an_owned_environment() {
        use sunrise_assets::resource_source::Resource;
        let source = ResourceSource::indexed(std::collections::BTreeMap::from([(
            "data/minecraft/dimension_type/overworld.json".into(),
            Resource::bytes(
                br#"{"attributes":{"minecraft:gameplay/sky_light_level":15.0}}"#.to_vec(),
            ),
        )]));
        let base = Environment::load(&source).unwrap();
        let darken = |e: &Environment| sky_darken(e, 0, 0.0, 0.0);
        let empty = TimelineTable::default();
        let own = Environment::with_type(
            &base,
            &serde_json::json!({"attributes": {"minecraft:gameplay/sky_light_level": 4.0}}),
            &empty,
        )
        .unwrap();
        assert_eq!((darken(&base), darken(&own)), (0, 11));
        // Timelines the client has no file for are skipped, not refused.
        assert!(
            Environment::with_type(
                &base,
                &serde_json::json!({"timelines": ["test:none"]}),
                &empty
            )
            .is_some()
        );
        let bad = serde_json::json!({"attributes": {"minecraft:visual/not_an_attribute": 1}});
        assert!(Environment::with_type(&base, &bad, &empty).is_none());
    }

    /// Attributes the game refuses leave the level on the Overworld's environment.
    #[test]
    fn refused_attributes_fall_back_to_the_base() {
        let _turn = LEVEL_TABLE.lock().unwrap_or_else(|e| e.into_inner());
        use sunrise_assets::resource_source::Resource;
        let source = ResourceSource::indexed(std::collections::BTreeMap::from([(
            "data/minecraft/dimension_type/overworld.json".into(),
            Resource::bytes(br#"{"attributes":{}}"#.to_vec()),
        )]));
        let base = Environment::load(&source).unwrap();
        Environment::set_level_attributes(
            vec![(
                "test:bad_env".to_owned(),
                serde_json::json!({"attributes": {"minecraft:visual/not_an_attribute": 1}}),
            )],
            TimelineTable::default(),
        );
        assert!(std::ptr::eq(
            Environment::for_level("test:bad_env", &base),
            &base
        ));
        Environment::set_level_attributes(Vec::new(), TimelineTable::default());
    }

    #[test]
    fn a_vanilla_level_reads_world_timelines_through_set_level_attributes() {
        let _turn = LEVEL_TABLE.lock().unwrap_or_else(|e| e.into_inner());
        use sunrise_assets::resource_source::Resource;
        let source = ResourceSource::indexed(std::collections::BTreeMap::from([
            (
                "data/minecraft/dimension_type/overworld.json".into(),
                Resource::bytes(
                    br##"{"timelines":"#minecraft:in_overworld","attributes":{"minecraft:gameplay/sky_light_level":15.0}}"##.to_vec(),
                ),
            ),
            (
                "data/minecraft/tags/timeline/in_overworld.json".into(),
                Resource::bytes(br#"{"values":["minecraft:day"]}"#.to_vec()),
            ),
            (
                "data/minecraft/timeline/day.json".into(),
                Resource::bytes(
                    br#"{"clock":"minecraft:overworld","period_ticks":24000,"tracks":{"minecraft:gameplay/sky_light_level":{"keyframes":[{"ticks":0,"value":15.0}]}}}"#.to_vec(),
                ),
            ),
        ]));
        let base = Environment::load(&source).unwrap();
        assert_eq!(sky_darken(&base, 6000, 0.0, 0.0), 0);

        let overridden = Timeline::parse(&serde_json::json!({
            "clock": "minecraft:overworld",
            "period_ticks": 24000,
            "tracks": {
                "minecraft:gameplay/sky_light_level": {
                    "keyframes": [{"ticks": 0, "value": 5.0}]
                }
            }
        }))
        .unwrap();
        let table = TimelineTable {
            timelines: BTreeMap::from([("minecraft:day".into(), Arc::new(overridden))]),
            tags: BTreeMap::from([(
                "minecraft:in_overworld".into(),
                vec!["minecraft:day".into()],
            )]),
        };
        Environment::set_level_attributes(vec![("minecraft:overworld".into(), Json::Null)], table);
        let world_overworld = Environment::for_level("minecraft:overworld", &base);
        assert_eq!(sky_darken(world_overworld, 6000, 0.0, 0.0), 10);
        Environment::set_level_attributes(Vec::new(), TimelineTable::default());
    }
}
