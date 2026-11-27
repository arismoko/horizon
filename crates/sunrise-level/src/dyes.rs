//! Vanilla `DyeColor` tables and lock-free optional resource-pack colour overrides.
//! Headless servers never call [`set_overrides`] and read vanilla defaults.
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

/// `DyeColor` names and texture diffuse colours, by id.
pub const DYES: [(&str, u32); 16] = [
    ("white", 0xF9FFFE),
    ("orange", 0xF9801D),
    ("magenta", 0xC74EBD),
    ("light_blue", 0x3AB3DA),
    ("yellow", 0xFED83D),
    ("lime", 0x80C71F),
    ("pink", 0xF38BAA),
    ("gray", 0x474F52),
    ("light_gray", 0x9D9D97),
    ("cyan", 0x169C9C),
    ("purple", 0x8932B8),
    ("blue", 0x3C44AA),
    ("brown", 0x835432),
    ("green", 0x5E7C16),
    ("red", 0xB02E26),
    ("black", 0x1D1D21),
];

/// `DyeColor` names and `getTextColor` values by id.
pub const DYE_TEXT: [(&str, u32); 16] = [
    ("white", 0xFFFFFF),
    ("orange", 0xFF681F),
    ("magenta", 0xFF00FF),
    ("light_blue", 0x9AC0CD),
    ("yellow", 0xFFFF00),
    ("lime", 0xBFFF00),
    ("pink", 0xFF69B4),
    ("gray", 0x808080),
    ("light_gray", 0xD3D3D3),
    ("cyan", 0x00FFFF),
    ("purple", 0xA020F0),
    ("blue", 0x0000FF),
    ("brown", 0x8B4513),
    ("green", 0x00FF00),
    ("red", 0xFF0000),
    ("black", 0x000000),
];

/// `DyeColor` names by id.
pub const DYE_NAMES: [&str; 16] = [
    "white",
    "orange",
    "magenta",
    "light_blue",
    "yellow",
    "lime",
    "pink",
    "gray",
    "light_gray",
    "cyan",
    "purple",
    "blue",
    "brown",
    "green",
    "red",
    "black",
];

/// `DyeColor.getTextureDiffuseColor` (RGB) by id.
pub const DYE_DIFFUSE: [u32; 16] = [
    0xF9FFFE, 0xF9801D, 0xC74EBD, 0x3AB3DA, 0xFED83D, 0x80C71F, 0xF38BAA, 0x474F52, 0x9D9D97,
    0x169C9C, 0x8932B8, 0x3C44AA, 0x835432, 0x5E7C16, 0xB02E26, 0x1D1D21,
];

/// Vanilla `DyeColor.getTextureDiffuseColor` (RGB).
pub const fn vanilla_diffuse(dye: usize) -> u32 {
    DYES[dye & 15].1
}

/// Vanilla `DyeColor.getTextColor` (RGB).
pub const fn vanilla_text(dye: usize) -> u32 {
    DYE_TEXT[dye & 15].1
}

/// Vanilla `ColorLerper.Type.SHEEP` wool colour (RGB).
pub fn vanilla_sheep_wool(dye: u8) -> u32 {
    let i = usize::from(dye & 15);
    if i == 0 {
        return 0xE6E6E6;
    }
    let src = DYES[i].1;
    let f = |c: u32| ((c as f32 * 0.75).floor() as u32).min(255);
    f(src >> 16 & 255) << 16 | f(src >> 8 & 255) << 8 | f(src & 255)
}

/// 0 is unset, else `1 << 32 | rgb`.
static DIFFUSE: [AtomicU64; 16] = [const { AtomicU64::new(0) }; 16];
static TEXT: [AtomicU64; 16] = [const { AtomicU64::new(0) }; 16];
static SHEEP: [AtomicU64; 16] = [const { AtomicU64::new(0) }; 16];
static REDSTONE: [AtomicU64; 16] = [const { AtomicU64::new(0) }; 16];

/// Installs resource-pack colour overrides across all four lock-free tables (`None` resets to vanilla).
pub fn set_overrides(
    dye_diffuse: &[Option<u32>; 16],
    dye_text: &[Option<u32>; 16],
    sheep: &[Option<u32>; 16],
    redstone: &[Option<u32>; 16],
) {
    let store = |table: &[AtomicU64; 16], i: usize, color: Option<u32>| {
        table[i].store(
            color.map_or(0, |c| 1 << 32 | u64::from(c & 0xFF_FFFF)),
            Relaxed,
        );
    };
    for i in 0..16 {
        store(&DIFFUSE, i, dye_diffuse[i]);
        store(&TEXT, i, dye_text[i]);
        store(&SHEEP, i, sheep[i]);
        store(&REDSTONE, i, redstone[i]);
    }
}

fn get(table: &[AtomicU64; 16], i: usize) -> Option<u32> {
    let v = table[i & 15].load(Relaxed);
    (v != 0).then_some(v as u32)
}

/// `DyeColor.getTextureDiffuseColor` (RGB) as the packs draw it.
pub fn diffuse(dye: usize) -> u32 {
    get(&DIFFUSE, dye).unwrap_or(DYES[dye & 15].1)
}

/// `DyeColor.getTextColor` (RGB) as the packs draw it.
pub fn text(dye: usize) -> u32 {
    get(&TEXT, dye).unwrap_or(DYE_TEXT[dye & 15].1)
}

/// A pack's `RedStoneWireBlock.COLORS` entry for this power (RGB), when it sets one.
pub fn redstone(power: u8) -> Option<u32> {
    get(&REDSTONE, usize::from(power))
}

/// `ColorLerper.Type.SHEEP` wool colour: a pack's `sheep.<name>`, else white 0xE6E6E6 and the
/// others 75% of vanilla's dye (a pack's `dye.*.diffuse` doesn't reach wool).
pub fn sheep_wool(dye: u8) -> u32 {
    let i = usize::from(dye & 15);
    if let Some(color) = get(&SHEEP, i) {
        return color;
    }
    vanilla_sheep_wool(dye)
}
