//! Client Polytone environment attribute modifiers, expressions, and colormaps; portable
//! dimension/biome environment attribute sampling lives in `sunrise_level::environment`.
use serde_json::Value as Json;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use sunrise_assets::resource_source::ResourceSource;
pub use sunrise_level::environment::{
    AttributeModifiers, Environment, Frame, Sample, Sky, TimelineTable, sky_darken,
    sky_darken_with_clock,
};
use sunrise_level::pack_tags::Tags;
use sunrise_world::environment_attributes::{
    Attribute, AttributeMap, Entry, Kind, Value, attribute,
};

mod noise;

/// Resource packs' Polytone biome and dimension modifiers (#11486): layers they add to, change
/// in or remove from the attribute stack.
#[derive(Clone, Default)]
pub struct PackModifiers {
    biomes: Vec<sunrise_polytone::BiomeModifier>,
    dimensions: Vec<sunrise_polytone::DimensionModifier>,
    /// The packs' config values, for `config(id)` in expressions.
    configs: Arc<sunrise_polytone::pack::Configs>,
    /// The pack overlay root they were read from, where their colormaps are.
    root: Option<ResourceSource>,
    colormaps: ColormapCache,
}

impl PackModifiers {
    /// Every `assets/<namespace>/polytone/{biome,dimension}_modifiers/**.json` under a pack
    /// overlay root. A file that fails to parse is skipped with a warning.
    pub fn read(pack_root: &ResourceSource) -> Self {
        let mut out = Self {
            root: Some(pack_root.clone()),
            ..Self::default()
        };
        for namespace in pack_root.namespaces("assets") {
            for kind in ["biome_modifiers", "dimension_modifiers"] {
                for relative in pack_root.list(&format!("assets/{namespace}/polytone/{kind}/")) {
                    if !relative.ends_with(".json") {
                        continue;
                    }
                    let implicit = sunrise_polytone::implicit_id(&relative, kind);
                    let json = match pack_root
                        .read(&relative)
                        .map_err(|e| e.to_string())
                        .and_then(|bytes| {
                            serde_json::from_slice::<Json>(&bytes).map_err(|e| e.to_string())
                        }) {
                        Ok(json) => json,
                        Err(e) => {
                            log::warn!("{relative}: {e}");
                            continue;
                        }
                    };
                    let parsed = if kind == "biome_modifiers" {
                        sunrise_polytone::BiomeModifier::parse(&json, implicit.as_deref())
                            .map(|m| m.map(|m| out.biomes.push(m)))
                    } else {
                        sunrise_polytone::DimensionModifier::parse(&json, implicit.as_deref())
                            .map(|m| m.map(|m| out.dimensions.push(m)))
                    };
                    if let Err(e) = parsed {
                        log::warn!("{relative}: {e}");
                    }
                }
            }
        }
        out.biomes.sort_by_key(|m| m.priority);
        out.dimensions.sort_by_key(|m| m.priority);
        out
    }

    pub fn is_empty(&self) -> bool {
        self.biomes.is_empty() && self.dimensions.is_empty()
    }

    /// The config values expressions read.
    pub fn with_configs(mut self, configs: sunrise_polytone::pack::Configs) -> Self {
        self.configs = Arc::new(configs);
        self
    }
}

impl AttributeModifiers for PackModifiers {
    fn apply_dimension(
        &self,
        dimension_id: &str,
        tags: &Tags,
        frame: &Arc<RwLock<Frame>>,
        dimension_map: &mut AttributeMap,
        rain: &mut AttributeMap,
        thunder: &mut AttributeMap,
        dimension_post: &mut AttributeMap,
    ) {
        if self.dimensions.is_empty() {
            return;
        }
        let resolver = Resolver {
            frame: frame.clone(),
            configs: self.configs.clone(),
            pack_root: self.root.clone(),
            colormaps: self.colormaps.clone(),
        };
        for m in self.dimensions.iter().filter(|m| {
            m.targets.matches(dimension_id, |tag| {
                tags.contains("dimension_type", tag, dimension_id)
            })
        }) {
            resolver.apply(dimension_map, &m.base);
            resolver.apply(rain, &m.rain);
            resolver.apply(thunder, &m.thunder);
            resolver.apply(dimension_post, &m.post_process);
        }
    }

    fn apply_biome(
        &self,
        biome_id: &str,
        tags: &Tags,
        frame: &Arc<RwLock<Frame>>,
        biome_map: &mut AttributeMap,
        post_map: &mut AttributeMap,
    ) {
        if self.biomes.is_empty() {
            return;
        }
        let resolver = Resolver {
            frame: frame.clone(),
            configs: self.configs.clone(),
            pack_root: self.root.clone(),
            colormaps: self.colormaps.clone(),
        };
        for m in &self.biomes {
            let in_tag = |tag: &str| tags.contains("worldgen/biome", tag, biome_id);
            if m.targets.matches(biome_id, in_tag) {
                resolver.apply(biome_map, &m.base);
                resolver.apply(post_map, &m.post_process);
            }
        }
    }
}

/// The game side of a pack expression in an attribute value: `g`, `p`, `c`, `o` (the camera's
/// position) and `r`, `v` (the value so far) and the pack's `config`.
struct FrameHost<'a> {
    frame: &'a Frame,
    configs: &'a sunrise_polytone::pack::Configs,
    value: f64,
    seed: u64,
}

/// `program`'s value for the frame (a pack's expression outside attribute layers: its `v` is 0).
pub(crate) fn eval_frame(
    program: &sunrise_polytone::expr::Program,
    frame: &Frame,
    configs: &sunrise_polytone::pack::Configs,
) -> Option<sunrise_polytone::expr::Value> {
    let mut host = FrameHost {
        frame,
        configs,
        value: 0.0,
        seed: frame.game_time as u64,
    };
    program.eval(&mut host).ok()
}

impl sunrise_polytone::expr::Host for FrameHost<'_> {
    fn root(&mut self, root: sunrise_polytone::expr::Root) -> sunrise_polytone::expr::Value {
        use sunrise_polytone::expr::{Root, Value as V};
        match root {
            Root::Value => V::Num(self.value),
            other => V::Object(other as u32),
        }
    }
    fn method(
        &mut self,
        target: &sunrise_polytone::expr::Value,
        name: &str,
        _args: &[sunrise_polytone::expr::Value],
    ) -> std::result::Result<sunrise_polytone::expr::Value, String> {
        use sunrise_polytone::expr::{Root, Value as V};
        let f = self.frame;
        let on = |root: Root| *target == V::Object(root as u32);
        let num = |n: f64| Ok(V::Num(n));
        match name {
            "rain" if on(Root::Global) => num(f.global_rain()),
            "time" if on(Root::Global) => num(f.game_time as f64),
            "dayTime" if on(Root::Global) => num(f.day_time as f64),
            "skyType" if on(Root::Global) => num(f64::from(f.sky_type)),
            "dimensionType" if on(Root::Global) => Ok(V::Str(f.dimension.as_str().into())),
            "renderDistance" if on(Root::Global) => num(f64::from(f.render_distance)),
            "moonPhase" if on(Root::Global) => {
                num(f.day_time.div_euclid(24_000).rem_euclid(8) as f64)
            }
            "seaLevel" if on(Root::Global) => num(63.0),
            "viewDistance" if on(Root::Camera) => num(f64::from(f.render_distance * 16)),
            "x" | "y" | "z" if on(Root::Player) || on(Root::Camera) || on(Root::Object) => {
                let at = if on(Root::Player) { f.player } else { f.camera };
                num(at[usize::from(name.as_bytes()[0] - b'x')])
            }
            "inWater" if on(Root::Player) => Ok(V::Bool(f.in_water)),
            "skyLight" if on(Root::Object) || on(Root::Player) => num(f64::from(f.sky_light)),
            "blockLight" if on(Root::Object) || on(Root::Player) => num(f64::from(f.block_light)),
            "temperature" if on(Root::Object) => num(f64::from(f.temperature)),
            "downfall" if on(Root::Object) => num(f64::from(f.downfall)),
            "biome" if on(Root::Object) || on(Root::Player) || on(Root::Camera) => {
                Ok(V::Str(f.biome.as_str().into()))
            }
            "rand" if on(Root::Random) => {
                // A splitmix64 step: the frame's own sequence.
                self.seed = self.seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
                let mut z = self.seed;
                z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                num(((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64)
            }
            _ => Err(format!(
                "{name} is not available in an attribute expression"
            )),
        }
    }
    fn function(
        &mut self,
        name: &str,
        args: &[sunrise_polytone::expr::Value],
    ) -> std::result::Result<sunrise_polytone::expr::Value, String> {
        match (name, args) {
            ("config", [sunrise_polytone::expr::Value::Str(id)]) => self
                .configs
                .get(id)
                .map(sunrise_polytone::pack::ConfigValue::to_value)
                .ok_or_else(|| format!("no config {id}")),
            _ => Err(format!(
                "{name} is not available in an attribute expression"
            )),
        }
    }
}

/// A value as an expression's `v`: floats as they are, colours packed ARGB (`ARGB.as8BitChannel`).
fn as_number(value: &Value) -> f64 {
    let byte = |c: f32| ((c.clamp(0.0, 1.0) * 255.0).floor() as u32) & 255;
    match value {
        Value::Float(f) => f64::from(*f),
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::Rgb([r, g, b]) => {
            f64::from((0xff00_0000 | byte(*r) << 16 | byte(*g) << 8 | byte(*b)) as i32)
        }
        Value::Argb([r, g, b, a]) => {
            f64::from((byte(*a) << 24 | byte(*r) << 16 | byte(*g) << 8 | byte(*b)) as i32)
        }
        Value::AmbientParticles(_) | Value::Other(_) => 0.0,
    }
}

/// A Polytone colormap used as an attribute value (wiki "Colormaps"): an image read at the
/// column and row its two expressions pick, each 0 to 1 with 1 the top-left corner.
pub(crate) struct Colormap {
    x: Axis,
    y: Axis,
    /// Only the lower-left triangle: the row value is scaled by the column value, as vanilla's
    /// grass colormap scales downfall by temperature.
    triangular: bool,
    /// Rounds to the nearest pixel (the default) instead of the one below.
    rounds: bool,
    /// `biome_blend`: blocks take the mean of the colours around them, over the Biome Blend
    /// square, as vanilla's biome tints do. Without the field, maps that sample the biome
    /// (`temperature`, `downfall`, `biome_id`) blend: Polytone 6.8.6's water by `biome_id`
    /// fades over a biome edge.
    pub(crate) biome_blend: bool,
    /// Its axes read nothing that changes up a column of one biome (no `o.y`): a section of
    /// one biome can sample it once per column.
    pub(crate) column: bool,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// A colormap axis: an expression (or a named sampler, wiki "Default Samplers"), or the
/// `biome_id` sampler with its mapper.
enum Axis {
    Expr(sunrise_polytone::expr::Program),
    /// `biome_id`: the value the mapper gives the biome where it samples (0 for the others),
    /// and the expression naming that biome. Unlike the other axes it counts from the left (or
    /// top) edge, in pixel coordinates over the mapper's `texture_size`: the column (or row) is
    /// `floor(value * size)`, so `1` over a `texture_size` of 16 is the second pixel, and the
    /// biomes left out are the first, as the wiki's "start your mapper at 1" advice expects.
    BiomeId(HashMap<String, f64>, sunrise_polytone::expr::Program),
}

impl Axis {
    /// Whether the axis reads only what stays the same up a column of one biome.
    fn column(&self) -> bool {
        use sunrise_polytone::expr::Root;
        let flat = |root: Root, name: &str| match root {
            Root::Object => matches!(name, "x" | "z" | "biome" | "temperature" | "downfall"),
            Root::Random => name == "noise",
            _ => false,
        };
        match self {
            Self::Expr(program) | Self::BiomeId(_, program) => program.reads_only(&flat),
        }
    }
    fn value(&self, host: &mut dyn sunrise_polytone::expr::Host) -> Option<f64> {
        match self {
            Self::Expr(program) => Some(program.eval(host).ok()?.num()),
            Self::BiomeId(mapper, biome) => {
                let biome = match biome.eval(host).ok()? {
                    sunrise_polytone::expr::Value::Str(s) => s,
                    _ => return Some(0.0),
                };
                Some(mapper.get(biome.as_ref()).copied().unwrap_or(0.0))
            }
        }
    }
}

/// A `biome_id_mapper`: inline, or an id of `polytone/biome_id_mappers/<path>.json`. Its values
/// are divided by its `texture_size`, when it has one (pixel coordinates).
fn biome_id_mapper(pack_root: &ResourceSource, json: &Json) -> HashMap<String, f64> {
    let object = match json {
        Json::String(id) => {
            let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
            let file = format!("assets/{namespace}/polytone/biome_id_mappers/{path}.json");
            pack_root
                .read(&file)
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or(Json::Null)
        }
        other => other.clone(),
    };
    let Some(object) = object.as_object() else {
        return HashMap::new();
    };
    let size = object
        .get("texture_size")
        .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()))
        .filter(|s| *s > 0.0)
        .unwrap_or(1.0);
    object
        .iter()
        .filter(|(k, _)| *k != "texture_size")
        .filter_map(|(k, v)| {
            let id = if k.contains(':') {
                k.clone()
            } else {
                format!("minecraft:{k}")
            };
            Some((id, v.as_f64()? / size))
        })
        .collect()
}

impl Colormap {
    /// One colour wherever it samples (a `biome_colors` flat colour, RGB).
    pub(crate) fn flat(rgb: u32) -> Self {
        let zero = || Axis::Expr(sunrise_polytone::expr::Program::compile("0").expect("a number"));
        let [_, r, g, b] = rgb.to_be_bytes();
        Self {
            x: zero(),
            y: zero(),
            triangular: false,
            rounds: true,
            biome_blend: false,
            column: true,
            width: 1,
            height: 1,
            rgba: vec![r, g, b, 255],
        }
    }

    /// `assets/<namespace>/polytone/colormaps/<path>.png` and its optional `.json` (without
    /// one, the axes are the biome's temperature and downfall).
    pub(crate) fn load(pack_root: &ResourceSource, id: &str) -> Option<Self> {
        let (namespace, path) = id.split_once(':')?;
        let base = format!("assets/{namespace}/polytone/colormaps/{path}");
        let png = format!("{base}.png");
        let (width, height, rgba) = match crate::sprites::decode_rgba(&pack_root.read(&png).ok()?) {
            Ok(image) => image,
            Err(e) => {
                log::warn!("colormap {id}: {e}");
                return None;
            }
        };
        let json: Json = pack_root
            .read(&format!("{base}.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or(Json::Null);
        let compile = sunrise_polytone::expr::Program::compile;
        let mut by_biome = false;
        let mut axis = |key: &str, default: &str| -> Option<Axis> {
            let src = match &json[key] {
                Json::String(s) => s.clone(),
                Json::Number(n) => n.to_string(),
                _ => default.to_owned(),
            };
            by_biome |= matches!(
                src.trim(),
                "biome_id" | "temperature" | "legacy_temperature" | "downfall"
            ) || matches!(src.trim(), "o.temperature" | "o.downfall");
            // The named samplers (wiki "Default Samplers") as the expressions they stand for.
            let src = match src.trim() {
                "biome_id" => {
                    let mapper = biome_id_mapper(pack_root, &json["biome_id_mapper"]);
                    return Some(Axis::BiomeId(mapper, compile("o.biome()").ok()?));
                }
                "temperature" | "legacy_temperature" => "o.temperature".to_owned(),
                "downfall" => "o.downfall".to_owned(),
                "y_level" => "o.y / 128".to_owned(),
                "day_time" => "1 - (g.dayTime % 24000) / 24000".to_owned(),
                _ => src,
            };
            match compile(&src).map(Axis::Expr) {
                Ok(program) => Some(program),
                Err(e) => {
                    log::warn!("colormap {id} {key}: {e}");
                    None
                }
            }
        };
        let (x, y) = (
            axis("x_axis", "o.temperature")?,
            axis("y_axis", "o.downfall")?,
        );
        let column = [&x, &y].iter().all(|axis| axis.column());
        Some(Self {
            x,
            y,
            column,
            biome_blend: json["biome_blend"].as_bool().unwrap_or(by_biome),
            triangular: json["triangular"].as_bool() == Some(true),
            rounds: json["rounds"].as_bool() != Some(false),
            width,
            height,
            rgba,
        })
    }

    /// The colour the axes pick, ARGB; `None` for a transparent pixel (the value stays what
    /// it was, the default `fallback_colormap: original`) or an axis that fails.
    pub(crate) fn sample(&self, host: &mut dyn sunrise_polytone::expr::Host) -> Option<u32> {
        let x = self.x.value(host)?.clamp(0.0, 1.0);
        let mut y = self.y.value(host)?.clamp(0.0, 1.0);
        if self.triangular {
            y *= x;
        }
        let pixel = |axis: &Axis, v: f64, size: u32| {
            let last = size.saturating_sub(1);
            if let Axis::BiomeId(..) = axis {
                return ((v * f64::from(size)).floor() as u32).min(last);
            }
            let p = (1.0 - v) * f64::from(last);
            let p = if self.rounds { p.round() } else { p.floor() };
            (p as u32).min(last)
        };
        let (px, py) = (
            pixel(&self.x, x, self.width),
            pixel(&self.y, y, self.height),
        );
        let i = (py as usize * self.width as usize + px as usize) * 4;
        let [r, g, b, a] = self.rgba.get(i..i + 4)?.try_into().ok()?;
        (a != 0).then(|| u32::from(a) << 24 | u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b))
    }
}

/// The colormaps of the Polytone block and fluid modifiers of the packs merged at `pack_root`:
/// (a fluid modifier, its targets, the colormap's id). Inline colormaps aren't read.
pub(crate) fn polytone_colormap_modifiers(
    pack_root: &ResourceSource,
) -> Vec<(bool, sunrise_polytone::Targets, String)> {
    let mut out = Vec::new();
    for relative in pack_root.list("assets/") {
        for (kind, fluid) in [("block_modifiers", false), ("fluid_modifiers", true)] {
            if !relative.ends_with(".json") || !relative.contains(&format!("/polytone/{kind}/")) {
                continue;
            }
            let Some(json) = pack_root
                .read(&relative)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Json>(&bytes).ok())
            else {
                continue;
            };
            let Some(id) = json.get("colormap").and_then(Json::as_str) else {
                continue;
            };
            let id = if id.contains(':') {
                id.to_owned()
            } else {
                format!("minecraft:{id}")
            };
            let implicit = sunrise_polytone::implicit_id(&relative, kind);
            match sunrise_polytone::Targets::parse(json.get("targets"), implicit.as_deref()) {
                Ok(targets) => out.push((fluid, targets, id)),
                Err(e) => log::warn!("{relative}: {e}"),
            }
        }
    }
    out
}

/// What a block colormap's axes read where it samples: the block (`o.x`, `o.biome()`, its
/// biome's climate) and `r.noise`. The world isn't reachable from the mesher, so the rest fails
/// and the block keeps its own tint.
pub(crate) struct BlockHost<'a> {
    at: [i32; 3],
    biome: &'a crate::biome::BiomeInfo,
}

impl<'a> BlockHost<'a> {
    pub(crate) fn new(at: [i32; 3], biome: &'a crate::biome::BiomeInfo) -> Self {
        Self { at, biome }
    }
}

/// The noise `r.noise(x, y)` reads, the same for blocks and particles.
pub(crate) fn polytone_noise() -> &'static noise::PerlinSimplex {
    noise::builtin()
}

impl sunrise_polytone::expr::Host for BlockHost<'_> {
    fn field(
        &mut self,
        root: sunrise_polytone::expr::Root,
        field: sunrise_polytone::expr::Field,
    ) -> Option<sunrise_polytone::expr::Value> {
        use sunrise_polytone::expr::Field as F;
        if root != sunrise_polytone::expr::Root::Object {
            return None;
        }
        Some(sunrise_polytone::expr::Value::Num(match field {
            F::X => f64::from(self.at[0]),
            F::Y => f64::from(self.at[1]),
            F::Z => f64::from(self.at[2]),
            F::Temperature => f64::from(self.biome.temperature),
            F::Downfall => f64::from(self.biome.downfall),
            _ => return None,
        }))
    }

    fn root(&mut self, root: sunrise_polytone::expr::Root) -> sunrise_polytone::expr::Value {
        use sunrise_polytone::expr::Root;
        sunrise_polytone::expr::Value::Object(match root {
            Root::Object => 1,
            Root::Random => 2,
            _ => 0,
        })
    }

    fn method(
        &mut self,
        target: &sunrise_polytone::expr::Value,
        name: &str,
        args: &[sunrise_polytone::expr::Value],
    ) -> std::result::Result<sunrise_polytone::expr::Value, String> {
        use sunrise_polytone::expr::Value as V;
        let num = |n: f64| Ok(V::Num(n));
        match (target, name) {
            (V::Object(1), "x" | "y" | "z") => {
                num(f64::from(self.at[usize::from(name.as_bytes()[0] - b'x')]))
            }
            (V::Object(1), "biome") => Ok(V::Str(self.biome.name.as_str().into())),
            (V::Object(1), "temperature") => num(f64::from(self.biome.temperature)),
            (V::Object(1), "downfall") => num(f64::from(self.biome.downfall)),
            (V::Object(2), "noise") => {
                let mut coords = args.iter().filter(|v| !matches!(v, V::Str(_))).map(V::num);
                let (x, y) = (coords.next().unwrap_or(0.0), coords.next().unwrap_or(0.0));
                num(polytone_noise().get(x, y))
            }
            _ => Err(format!("{name} is not available to block colormaps")),
        }
    }

    fn function(
        &mut self,
        name: &str,
        _: &[sunrise_polytone::expr::Value],
    ) -> std::result::Result<sunrise_polytone::expr::Value, String> {
        Err(format!("{name} is not available to block colormaps"))
    }
}

/// Turns Polytone layers into vanilla entries, compiling expressions against the frame.
struct Resolver {
    frame: Arc<RwLock<Frame>>,
    configs: Arc<sunrise_polytone::pack::Configs>,
    /// Where the packs' colormaps are, and the ones loaded so far (`None`: missing).
    pack_root: Option<ResourceSource>,
    colormaps: ColormapCache,
}

/// Colormaps by id, loaded once for all three dimensions' stacks.
type ColormapCache = Arc<std::sync::Mutex<HashMap<String, Option<Arc<Colormap>>>>>;

impl Resolver {
    /// A Polytone layer as a vanilla entry: `Some(None)` removes the attribute's entry, `None`
    /// is a value not supported yet (colormaps) or a broken one.
    fn entry(
        &self,
        attribute: &'static Attribute,
        layer: &sunrise_polytone::Layer,
    ) -> Option<Option<Entry>> {
        use sunrise_polytone::{Argument, Layer, Modifier};
        let (modifier, argument) = match layer {
            Layer::Remove => return Some(None),
            Layer::Apply {
                modifier, argument, ..
            } => (modifier, argument),
        };
        let op = match modifier {
            Modifier::Override => "override",
            Modifier::Add => "add",
            Modifier::Subtract => "subtract",
            Modifier::Multiply => "multiply",
            Modifier::Minimum => "minimum",
            Modifier::Maximum => "maximum",
            Modifier::AlphaBlend => "alpha_blend",
            Modifier::BlendToGray => "blend_to_gray",
            Modifier::And => "and",
            Modifier::Nand => "nand",
            Modifier::Or => "or",
            Modifier::Nor => "nor",
            Modifier::Xor => "xor",
            Modifier::Xnor => "xnor",
        };
        let json = match argument {
            Argument::Number(n) => Json::from(*n),
            Argument::Bool(b) => Json::Bool(*b),
            // Polytone colours may leave out the `#`; vanilla's codec wants it, 6 digits for RGB.
            Argument::Color(c) if attribute.kind == Kind::RgbColor => {
                Json::String(format!("#{:06x}", c & 0x00ff_ffff))
            }
            Argument::Color(c) => Json::String(format!("#{c:08x}")),
            Argument::Object(o) => Json::Object(o.clone()),
            Argument::Expression(src) => return self.expression(attribute, op, src).map(Some),
            Argument::Colormap(id) => return self.colormap(attribute, op, id).map(Some),
        };
        match Entry::with_modifier(attribute, op, &json) {
            Ok(entry) => Some(Some(entry)),
            Err(e) => {
                log::warn!("{}: {e}", attribute.id);
                None
            }
        }
    }

    /// An expression value: compiled once, evaluated against the frame each time the layer
    /// applies, its result read as the modifier's argument (a number, a boolean, or a packed
    /// colour for colour attributes).
    fn expression(
        &self,
        attribute: &'static Attribute,
        op: &'static str,
        src: &str,
    ) -> Option<Entry> {
        let program = match sunrise_polytone::expr::Program::compile(src) {
            Ok(program) => program,
            Err(e) => {
                log::warn!("{}: {e}", attribute.id);
                return None;
            }
        };
        let (frame, configs) = (self.frame.clone(), self.configs.clone());
        let f = move |value: &Value| -> Option<Entry> {
            let frame = frame.read().ok()?;
            let mut host = FrameHost {
                frame: &frame,
                configs: &configs,
                value: as_number(value),
                seed: 0,
            };
            let json = match program.eval(&mut host).ok()? {
                sunrise_polytone::expr::Value::Bool(b) => Json::Bool(b),
                result => Json::from(result.num()),
            };
            // A number out of the attribute's range is clamped into it, as the sampled value is
            // (`AttributeRange.sanitize`), not dropped: a pack's fog end that goes negative deep
            // underground is the thickest fog, not the default.
            Entry::with_modifier(attribute, op, &json).ok().or_else(|| {
                let n = json.as_f64()? as f32;
                let Value::Float(clamped) = attribute.sanitize(Value::Float(n)) else {
                    return None;
                };
                Entry::with_modifier(attribute, op, &Json::from(clamped)).ok()
            })
        };
        Some(Entry::Dynamic(
            sunrise_world::environment_attributes::Dynamic(Arc::new(f)),
        ))
    }

    /// A colormap value: sampled whenever its layer applies, its colour the modifier's argument
    /// (`#rrggbb` on RGB attributes, `#aarrggbb` on ARGB ones).
    fn colormap(&self, attribute: &'static Attribute, op: &'static str, id: &str) -> Option<Entry> {
        let root = self.pack_root.as_ref()?;
        let colormap = self
            .colormaps
            .lock()
            .ok()?
            .entry(id.to_owned())
            .or_insert_with(|| Colormap::load(root, id).map(Arc::new))
            .clone()?;
        let (frame, configs) = (self.frame.clone(), self.configs.clone());
        let rgb = attribute.kind == Kind::RgbColor;
        let f = move |value: &Value| -> Option<Entry> {
            let frame = frame.read().ok()?;
            let mut host = FrameHost {
                frame: &frame,
                configs: &configs,
                value: as_number(value),
                seed: 0,
            };
            let color = colormap.sample(&mut host)?;
            let text = if rgb {
                format!("#{:06x}", color & 0x00ff_ffff)
            } else {
                format!("#{color:08x}")
            };
            Entry::with_modifier(attribute, op, &Json::String(text)).ok()
        };
        Some(Entry::Dynamic(
            sunrise_world::environment_attributes::Dynamic(Arc::new(f)),
        ))
    }

    /// Applies a group of Polytone layers to `map`: entries set or replaced, `REMOVE` drops
    /// them.
    fn apply(&self, map: &mut AttributeMap, group: &sunrise_polytone::Attributes) {
        for (id, layer) in group {
            let Some(attribute) = attribute(id) else {
                log::warn!("unknown attribute {id}");
                continue;
            };
            match self.entry(attribute, layer) {
                // A computed value that can't decide (a transparent colormap pixel, a failing
                // expression) falls back to the entry it replaces: the value before the pack.
                Some(Some(Entry::Dynamic(f))) => match map.get(attribute).cloned() {
                    Some(original) => {
                        let fallback = move |v: &Value| (f.0)(v).or_else(|| Some(original.clone()));
                        let entry =
                            sunrise_world::environment_attributes::Dynamic(Arc::new(fallback));
                        map.set(attribute, Entry::Dynamic(entry));
                    }
                    None => map.set(attribute, Entry::Dynamic(f)),
                },
                Some(Some(entry)) => map.set(attribute, entry),
                Some(None) => map.remove(attribute),
                None => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A world with one sky-lit dimension and two biomes, one of them in a tag, and a pack with
    /// Polytone modifiers.
    fn with_packs(pack: &[(&str, &str)]) -> Environment {
        let files: Vec<(&str, &[u8])> = pack.iter().map(|(p, t)| (*p, t.as_bytes())).collect();
        with_pack_files(&files)
    }

    fn with_pack_files(pack: &[(&str, &[u8])]) -> Environment {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "sunrise-env-packs-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let write = |path: &str, text: &[u8]| {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(
            "world/data/minecraft/dimension_type/overworld.json",
            r##"{"has_skylight": true, "has_ceiling": false,
                "attributes": {"minecraft:visual/sky_color": "#808080", "minecraft:visual/fog_color": "#000000"}}"##.as_bytes(),
        );
        write(
            "world/data/minecraft/worldgen/biome/plains.json",
            r##"{"attributes": {"minecraft:visual/sky_color": "#404040"}}"##.as_bytes(),
        );
        write(
            "world/data/minecraft/worldgen/biome/desert.json",
            r#"{"attributes": {}}"#.as_bytes(),
        );
        write(
            "world/data/minecraft/tags/worldgen/biome/dry.json",
            r#"{"values": ["minecraft:desert"]}"#.as_bytes(),
        );
        for (path, text) in pack {
            write(&format!("pack/{path}"), text);
        }
        let modifiers = PackModifiers::read(&ResourceSource::directory(dir.join("pack")))
            .with_configs(crate::packs::polytone_configs(&crate::packs::Source::Dir(
                dir.join("pack"),
            )));
        let env = Environment::load_dimension_with(
            &ResourceSource::directory(dir.join("world")),
            "overworld",
            &modifiers,
        )
        .unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        env
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
    fn pack_biome_modifiers_change_their_targets_only() {
        let env = with_packs(&[(
            "assets/mypack/polytone/biome_modifiers/dry_fog.json",
            r##"{"targets": "#minecraft:dry", "attributes_modifiers": {"visual/fog_color": "ff0000"}}"##,
        )]);
        assert_eq!(
            sky_at(&env, "minecraft:desert", 0.0).fog_color,
            [1.0, 0.0, 0.0]
        );
        assert_eq!(sky_at(&env, "minecraft:plains", 0.0).fog_color, [0.0; 3]);
        // Without packs the stack is vanilla's: the biome's own sky.
        let plain = with_packs(&[]);
        assert_eq!(
            sky_at(&plain, "minecraft:plains", 0.0).sky_color,
            [GRAY_40; 3]
        );
    }

    #[test]
    fn dimension_modifiers_remove_weather_layers_and_post_process_last() {
        let raining = |env: &Environment| sky_at(env, "minecraft:plains", 1.0).sky_color;
        // Vanilla's rain grays the sky toward a darker gray.
        assert_ne!(raining(&with_packs(&[])), [GRAY_40; 3]);
        let no_rain = with_packs(&[(
            "assets/minecraft/polytone/dimension_modifiers/overworld.json",
            r#"{"attributes_modifiers": {"rain": {"visual/sky_color": "REMOVE"}}}"#,
        )]);
        assert_eq!(raining(&no_rain), [GRAY_40; 3]);
        let post = with_packs(&[(
            "assets/minecraft/polytone/dimension_modifiers/overworld.json",
            r##"{"attributes_modifiers": {"post_process": {"visual/sky_color": "#0000ff"}}}"##,
        )]);
        assert_eq!(raining(&post), [0.0, 0.0, 1.0]);
    }

    #[test]
    fn biome_post_process_and_priority() {
        let env = with_packs(&[
            (
                "assets/a/polytone/biome_modifiers/wins.json",
                r##"{"targets": "minecraft:plains", "priority": 5,
                    "attributes_modifiers": {"base": {"visual/fog_color": "#00ff00"},
                                             "post_process": {"visual/sky_color": "#ffffff"}}}"##,
            ),
            (
                "assets/b/polytone/biome_modifiers/loses.json",
                r##"{"targets": "minecraft:plains", "priority": -1,
                    "attributes_modifiers": {"visual/fog_color": "#0000ff"}}"##,
            ),
        ]);
        // The higher priority's fog wins.
        assert_eq!(
            sky_at(&env, "minecraft:plains", 0.0).fog_color,
            [0.0, 1.0, 0.0]
        );
        // The post-process sky comes after the weather, so the rain doesn't gray it.
        assert_eq!(
            sky_at(&env, "minecraft:plains", 1.0).sky_color,
            [1.0, 1.0, 1.0]
        );
        assert!(env.biome_post("minecraft:desert").is_none());
    }

    /// A number an expression gives outside the attribute's range is clamped into it, as vanilla
    /// clamps a sampled value: a fog end that a pack takes below zero deep underground is fog
    /// at once, not the default end.
    #[test]
    fn an_expression_value_out_of_range_is_clamped() {
        let env = with_packs(&[(
            "assets/minecraft/polytone/dimension_modifiers/overworld.json",
            r#"{"attributes_modifiers": {"visual/fog_end_distance": "64 - (p.y < 0) * 80"}}"#,
        )]);
        let end_at = |y: f64| {
            env.set_frame(&Frame {
                player: [0.5, y, 0.5],
                ..Frame::default()
            });
            sky_at(&env, "minecraft:plains", 0.0).fog_end
        };
        assert_eq!(end_at(70.0), 64.0);
        assert_eq!(end_at(-2.0), 0.0);
    }

    #[test]
    fn expression_values_read_the_frame_and_the_value_so_far() {
        let env = with_packs(&[
            (
                "assets/minecraft/polytone/dimension_modifiers/overworld.json",
                r#"{"attributes_modifiers": {
                    "visual/fog_end_distance": "14 * (c.viewDistance / 16) + min(g.rain * 2, 1) * 100",
                    "visual/fog_start_distance": {"argument": "v + config('mypack:shift')", "modifier": "override"},
                    "visual/cloud_height": "v * 0.5"}}"#,
            ),
            (
                "assets/mypack/polytone/config_entries/shift.json",
                r#"{"default_value": 7}"#,
            ),
        ]);
        let at = |rain: f32, thunder: f32, chunks: i32| {
            env.set_frame(&Frame {
                rain,
                thunder,
                render_distance: chunks,
                ..Frame::default()
            });
            sky_at(&env, "minecraft:plains", 0.0)
        };
        // `c.viewDistance` is in blocks: 16 chunks are 256.
        assert_eq!(at(0.0, 0.0, 16).fog_end, 224.0);
        // `g.rain` is the mean of rain and thunder: half rain is 0.25 (448 + 50), plain rain
        // 0.5 and a thunderstorm 1, both past the `min` (224 + 100).
        assert_eq!(at(0.5, 0.0, 32).fog_end, 498.0);
        assert_eq!(at(1.0, 0.0, 16).fog_end, 324.0);
        assert_eq!(at(1.0, 1.0, 16).fog_end, 324.0);
        // `v` is the value the layers before gave: the default start 0, and 192.33 for clouds.
        assert_eq!(at(0.0, 0.0, 16).fog_start, 7.0);
        assert_eq!(at(0.0, 0.0, 16).cloud_height, 192.33 * 0.5);
    }

    /// A PNG of RGBA pixels in one row.
    fn png_row(pixels: &[[u8; 4]]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, pixels.len() as u32, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&pixels.concat()).unwrap();
        writer.finish().unwrap();
        out
    }

    #[test]
    fn colormap_values_read_the_pixel_their_axes_pick() {
        // Left to right: red, green, transparent, blue; x 1 is the left edge.
        let png = png_row(&[
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [9, 9, 9, 0],
            [0, 0, 255, 255],
        ]);
        let env = with_pack_files(&[
            (
                "assets/minecraft/polytone/dimension_modifiers/overworld.json",
                br#"{"attributes_modifiers": {"visual/sky_color": "mypack:sky"}}"#,
            ),
            (
                "assets/mypack/polytone/colormaps/sky.json",
                br#"{"x_axis": "1 - g.rain", "y_axis": 0}"#,
            ),
            ("assets/mypack/polytone/colormaps/sky.png", &png),
        ]);
        let sky = |rain: f32, thunder: f32| {
            env.set_frame(&Frame {
                rain,
                thunder,
                ..Frame::default()
            });
            env.sample(&Sample::default()).sky_color
        };
        // `g.rain` 0, 0.3 (rain 0.6), 1 (a thunderstorm).
        assert_eq!(sky(0.0, 0.0), [1.0, 0.0, 0.0]);
        assert_eq!(sky(0.6, 0.0), [0.0, 1.0, 0.0]);
        assert_eq!(sky(1.0, 1.0), [0.0, 0.0, 1.0]);
        // A transparent pixel keeps the colour the layers before gave (the dimension's #808080):
        // `g.rain` 0.7.
        assert_eq!(sky(1.0, 0.4), [128.0 / 255.0; 3]);
    }

    #[test]
    fn biome_id_colormaps_read_the_column_their_mapper_gives_the_biome() {
        // Left to right: red, green, transparent, blue.
        let png = png_row(&[
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [9, 9, 9, 0],
            [0, 0, 255, 255],
        ]);
        let env = with_pack_files(&[
            (
                "assets/minecraft/polytone/dimension_modifiers/overworld.json",
                br#"{"attributes_modifiers": {"visual/sky_color": "mypack:sky"}}"#,
            ),
            (
                "assets/mypack/polytone/colormaps/sky.json",
                br#"{"x_axis": "biome_id", "y_axis": 0, "biome_id_mapper": "mypack:biomes"}"#,
            ),
            ("assets/mypack/polytone/colormaps/sky.png", &png),
            (
                // Pixel columns from the left, over a texture_size of the image's width.
                "assets/mypack/polytone/biome_id_mappers/biomes.json",
                br#"{"minecraft:desert": 3, "swamp": 1, "minecraft:plains": 2, "texture_size": 4}"#,
            ),
        ]);
        let sky = |biome: &str| {
            env.set_frame(&Frame {
                biome: biome.to_owned(),
                ..Frame::default()
            });
            env.sample(&Sample::default()).sky_color
        };
        assert_eq!(sky("minecraft:desert"), [0.0, 0.0, 1.0]);
        assert_eq!(sky("minecraft:swamp"), [0.0, 1.0, 0.0]);
        // A biome the mapper doesn't name is 0: the first column.
        assert_eq!(sky("minecraft:forest"), [1.0, 0.0, 0.0]);
        // Plains land on the transparent pixel: the colour before stays.
        assert_eq!(sky("minecraft:plains"), [128.0 / 255.0; 3]);
    }
}
