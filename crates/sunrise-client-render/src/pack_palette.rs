//! The colours a pack replaces where vanilla hardcodes them (Polytone's `colors.json`, read by the
//! platform's pack formats, [`crate::pack_hooks`]): dye, sheep and redstone colours, for drawing
//! only (the pack recolours collars, tropical fish, banners, wool, sign text and redstone wire and
//! dust, while gameplay keeps vanilla's `DyeColor`), mob effect colours, the experience number's,
//! entity shadows, the effect fogs' settings and durability bar colours. Lock-free: the draw paths
//! and the mesher's workers read the dye, sheep and redstone tables per face.
use crate::resource_source::ResourceSource;
use std::sync::{Arc, RwLock};

pub use sunrise_level::dyes::{DYE_TEXT, DYES, diffuse, redstone, sheep_wool, text};

/// The fog settings vanilla hardcodes, as the packs set them (vanilla's by default).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Environment {
    /// The fog colour inside powder snow (RGB).
    pub powder_snow_fog: Option<u32>,
    /// The fog ends of Blindness (5), Darkness (15) and bosses that add world fog (96).
    pub blindness_fog_distance: f32,
    pub darkness_fog_distance: f32,
    pub boss_fog_distance: f32,
    /// How much the underwater fog colour washes out (1 vanilla, 0 none).
    pub water_fog_brightening: f32,
    /// The ticks under water it takes (vanilla's 600).
    pub water_fog_brightening_time: f32,
    /// Below it the sky draws its dark disc (vanilla's 63; flat worlds keep their floor).
    pub horizon_height: f32,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            powder_snow_fog: None,
            blindness_fog_distance: 5.0,
            darkness_fog_distance: 15.0,
            boss_fog_distance: 96.0,
            water_fog_brightening: 1.0,
            water_fog_brightening_time: 600.0,
            horizon_height: 63.0,
        }
    }
}

/// The packs' colours; `None` (and the defaults) where they keep vanilla's.
#[derive(Clone, Debug, PartialEq)]
pub struct PackColors {
    /// By dye id (`DyeColor` order): `getTextureDiffuseColor`, `getTextColor`, the sheep's wool.
    pub dye_diffuse: [Option<u32>; 16],
    pub dye_text: [Option<u32>; 16],
    pub sheep: [Option<u32>; 16],
    /// `RedStoneWireBlock.COLORS` by power.
    pub redstone: [Option<u32>; 16],
    /// Mob effects' colours, by effect id without vanilla's namespace.
    pub effects: Vec<(String, u32)>,
    /// False hides every entity's round shadow, whatever the video option says.
    pub entity_shadows: bool,
    /// The experience level number's colour and outline.
    pub xp_bar: [Option<u32>; 2],
    /// The effect fogs' settings: the fog reads them every frame.
    pub env: Environment,
    /// The fog the packs' fluid modifiers give water and lava.
    pub fluid_fog: FluidFogs,
}

/// A fluid modifier's fog: a flat colour (RGB), and the fog's radius and fade multipliers (1 is
/// vanilla's).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FluidFog {
    pub color: Option<u32>,
    pub radius: f32,
    pub fade: f32,
}

/// The fog the packs' fluid modifiers give water and lava.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FluidFogs {
    pub water: Option<FluidFog>,
    pub lava: Option<FluidFog>,
}

impl Default for PackColors {
    fn default() -> Self {
        Self {
            dye_diffuse: [None; 16],
            dye_text: [None; 16],
            sheep: [None; 16],
            redstone: [None; 16],
            effects: Vec::new(),
            entity_shadows: true,
            xp_bar: [None; 2],
            env: Environment::default(),
            fluid_fog: FluidFogs::default(),
        }
    }
}

impl PackColors {
    /// Whether entities draw their round shadow, as far as the packs say.
    pub fn entity_shadows(&self) -> bool {
        self.entity_shadows
    }
}

/// The colours of the pack overlay at `root`, from the installed pack formats; their dye, sheep
/// and redstone colours go to the draw paths' tables ([`set`]).
pub fn read(root: &ResourceSource) -> PackColors {
    let formats = crate::pack_hooks::formats();
    let colors = formats.colors(root).unwrap_or_default();
    set(&colors);
    *ITEM_BARS.write().unwrap_or_else(|e| e.into_inner()) = formats.item_bars(root);
    colors
}

/// The packs' durability bar colours by item (Polytone's item modifiers' `bar_color`).
pub trait ItemBars: Send + Sync {
    /// The bar colour (RGB) of `item` at this damage (0 to 1), when a pack sets one.
    fn color(&self, item: &str, damage: f32) -> Option<u32>;
}

static ITEM_BARS: RwLock<Option<Arc<dyn ItemBars>>> = RwLock::new(None);

/// A pack's durability bar colour (RGB) for an item at this damage (0 to 1).
pub fn item_bar_color(item: &str, damage: f32) -> Option<u32> {
    let bars = ITEM_BARS.read().unwrap_or_else(|e| e.into_inner());
    bars.as_ref()?.color(item, damage)
}

/// Takes the colours of the packs just loaded (vanilla's where they set none).
pub fn set(colors: &PackColors) {
    sunrise_level::dyes::set_overrides(
        &colors.dye_diffuse,
        &colors.dye_text,
        &colors.sheep,
        &colors.redstone,
    );
}
