//! Sound sources (`SoundSource`), by name.

/// SoundSource names in vanilla order.
pub const SOUND_SOURCES: [&str; 11] = [
    "master", "music", "record", "weather", "block", "hostile", "neutral", "player", "ambient",
    "voice", "ui",
];

/// `SoundSource` indices (vanilla order).
pub const MASTER: usize = 0;
pub const MUSIC: usize = 1;
pub const RECORDS: usize = 2;
pub const WEATHER: usize = 3;
pub const BLOCKS: usize = 4;
pub const NEUTRAL: usize = 6;
pub const PLAYERS: usize = 7;
pub const AMBIENT: usize = 8;
pub const UI: usize = 10;

/// The SoundSource an event plays on. Vanilla callers pick the source; this
/// follows their conventions by event family.
pub fn sound_source_for_event(event: &str) -> usize {
    let e = event.trim_start_matches("minecraft:");
    let index = |name: &str| SOUND_SOURCES.iter().position(|n| *n == name).unwrap_or(0);
    const HOSTILE: [&str; 34] = [
        "zombie",
        "skeleton",
        "creeper",
        "spider",
        "cave_spider",
        "enderman",
        "witch",
        "slime",
        "magma_cube",
        "blaze",
        "ghast",
        "drowned",
        "husk",
        "stray",
        "phantom",
        "pillager",
        "vindicator",
        "evoker",
        "illusioner",
        "ravager",
        "guardian",
        "elder_guardian",
        "silverfish",
        "zombified_piglin",
        "wither_skeleton",
        "bogged",
        "breeze",
        "endermite",
        "parched",
        "warden",
        "zombie_villager",
        "vex",
        "wither",
        // Monster's common hurt/death (e.g. Giant) and other shared sounds.
        "hostile",
    ];
    if e == "block.potent_sulfur.noxious_gas" {
        // PotentSulfurBlock.animateTick, its only caller, plays it as AMBIENT.
        index("ambient")
    } else if e.starts_with("music_disc.") {
        index("record")
    } else if e.starts_with("item.goat_horn.") {
        // InstrumentItem.play: SoundSource.RECORDS.
        index("record")
    } else if e.starts_with("music.") {
        index("music")
    } else if e.starts_with("weather.") || e.starts_with("entity.lightning_bolt") {
        index("weather")
    } else if e.starts_with("block.") {
        index("block")
    } else if e.starts_with("ambient.") {
        index("ambient")
    } else if e.starts_with("ui.") {
        index("ui")
    } else if e.starts_with("entity.player.")
        || e.starts_with("item.")
        || e.starts_with("entity.item.")
        || e.starts_with("entity.experience_orb")
        || e.starts_with("entity.arrow")
    {
        index("player")
    } else if let Some(rest) = e.strip_prefix("entity.") {
        let mob = rest.split('.').next().unwrap_or("");
        if HOSTILE.contains(&mob) {
            index("hostile")
        } else {
            index("neutral")
        }
    } else {
        index("master")
    }
}

/// A portable server-queued sound waiting to be sent over the play protocol.
#[derive(Clone, Debug, PartialEq)]
pub struct ServerSound {
    pub event: String,
    pub source: usize,
    pub position: [f64; 3],
    pub volume: f32,
    pub pitch: f32,
    pub seed: Option<i64>,
    pub entity: Option<u32>,
    pub except_entity: Option<u32>,
}

impl ServerSound {
    pub fn new(event: &str, position: [f64; 3], volume: f32, pitch: f32) -> Self {
        Self {
            source: sound_source_for_event(event),
            event: event.to_owned(),
            position,
            volume,
            pitch,
            seed: None,
            entity: None,
            except_entity: None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SoundEntityState {
    pub position: [f64; 3],
    pub silent: bool,
    pub removed: bool,
}
