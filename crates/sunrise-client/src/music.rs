//! Situational music: `MusicManager.tick` choosing tracks from
//! `Minecraft.getSituationalMusic`, with the `minecraft:audio/background_music`
//! and `minecraft:audio/music_volume` environment attributes of dimension
//! types and biomes (a biome value overrides its dimension's).
use crate::options_ui::{MusicGroup, MusicTrack};
use crate::sound_engine::{InstanceId, MUSIC, PlayResult, SoundEngine, SoundInstance, Sounds};
use crate::sound_events::{Sentinel, SoundRegistry};
pub use mod_api::client::music::{Music, MusicLayer};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::Path,
    sync::Arc,
};
use sunrise_core::random::{LegacyRandom, RandomSource};
use sunrise_protocol::registry::Builtin;
use sunrise_world::environment_attributes::{
    Attribute, Entry as AttrEntry, Value as AttrValue, attribute as env_attribute,
};

fn music_volume_attribute() -> &'static Attribute {
    env_attribute("audio/music_volume").expect("audio/music_volume attribute is registered")
}

/// Static `SOUND_EVENT` registry, not the resource pack's playable sound pools.
pub(crate) use crate::mod_content::biomes::registered_sound;

/// The Music page's groups: each built-in situation's own tracks first, then
/// every other `music.` event's tracks that no earlier group listed, under
/// its dimension's biomes. A track several pools share is listed once, from
/// the first pool that names it, so "play now" always has a pool that plays
/// it and no track is left without a row.
pub fn track_groups(registry: &SoundRegistry) -> Vec<MusicGroup> {
    let mut groups: Vec<MusicGroup> = Vec::new();
    let mut listed: Vec<String> = Vec::new();
    // Built-in situations the Music settings page lists, in its order.
    let builtin = [
        ("sunrise.pages.music.menu", Music::menu()),
        ("sunrise.pages.music.overworld", Music::game()),
        ("sunrise.pages.music.creative", Music::creative()),
        ("sunrise.pages.music.underwater", Music::under_water()),
        ("sunrise.pages.music.end", Music::end()),
        ("sunrise.pages.music.boss", Music::end_boss()),
        ("sunrise.pages.music.credits", Music::credits()),
    ];
    for (label, music) in &builtin {
        let tracks = registry.tracks(&music.sound);
        if tracks.is_empty() {
            continue;
        }
        listed.extend(tracks.iter().cloned());
        groups.push(MusicGroup {
            label: (*label).to_owned(),
            tracks: tracks
                .into_iter()
                .map(|id| MusicTrack {
                    event: music.sound.clone(),
                    id,
                })
                .collect(),
        });
    }
    // `music_disc.*` (records) do not start with "music.".
    let mut events: Vec<&String> = registry
        .names()
        .filter(|event| event.starts_with("minecraft:music."))
        .collect();
    events.sort();
    // Keyed by the heading's rank, so the dimensions keep their order however
    // the event ids sort.
    let mut rest: BTreeMap<u8, (&'static str, Vec<MusicTrack>)> = BTreeMap::new();
    for event in events {
        if builtin.iter().any(|(_, music)| music.sound == *event) {
            continue;
        }
        let tracks: Vec<MusicTrack> = registry
            .tracks(event)
            .into_iter()
            .filter(|id| !listed.contains(id))
            .map(|id| MusicTrack {
                event: event.clone(),
                id,
            })
            .collect();
        if tracks.is_empty() {
            continue;
        }
        listed.extend(tracks.iter().map(|track| track.id.clone()));
        let (rank, label) = biome_group(event);
        rest.entry(rank)
            .or_insert_with(|| (label, Vec::new()))
            .1
            .extend(tracks);
    }
    groups.extend(rest.into_values().map(|(label, tracks)| MusicGroup {
        label: label.to_owned(),
        tracks,
    }));
    groups
}
/// The heading a biome-only pool's tracks go under: vanilla plays these
/// tracks in several biomes, so no heading names a single one.
fn biome_group(event: &str) -> (u8, &'static str) {
    let path = event.strip_prefix("minecraft:").unwrap_or(event);
    if path.starts_with("music.overworld.") {
        (1, "sunrise.pages.music.overworld_biomes")
    } else if path.starts_with("music.nether.") {
        (2, "sunrise.pages.music.nether_biomes")
    } else if path.starts_with("music.end.") {
        (3, "sunrise.pages.music.end_biomes")
    } else {
        (4, "sunrise.pages.music.other")
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BackgroundMusic {
    pub default: Option<Music>,
    pub creative: Option<Music>,
    pub underwater: Option<Music>,
}
impl BackgroundMusic {
    fn parse(v: &Value, registered: &impl Fn(&str) -> bool) -> Option<Self> {
        let v = v.as_object()?;
        let music = |name: &str| match v.get(name) {
            Some(value) => Music::parse(value, registered).map(Some),
            None => Some(None),
        };
        Some(Self {
            default: music("default")?,
            creative: music("creative")?,
            underwater: music("underwater")?,
        })
    }
    /// `BackgroundMusic.select`.
    pub fn select(&self, creative: bool, underwater: bool) -> Option<&Music> {
        if underwater && self.underwater.is_some() {
            self.underwater.as_ref()
        } else if creative && self.creative.is_some() {
            self.creative.as_ref()
        } else {
            self.default.as_ref()
        }
    }
}
#[derive(Clone, Debug, Default)]
struct Layer {
    music: Option<BackgroundMusic>,
    volume: Option<AttrEntry>,
}
impl Layer {
    fn parse(json: &Value, builtin: &Builtin) -> Option<Self> {
        let Some(attributes) = json.as_object()?.get("attributes") else {
            return Some(Self::default());
        };
        let attributes = attributes.as_object()?;
        Some(Self {
            music: match attributes.get("minecraft:audio/background_music") {
                Some(music) => Some(BackgroundMusic::parse(music, &|sound| {
                    registered_sound(builtin, sound)
                })?),
                None => None,
            },
            volume: match attributes.get("minecraft:audio/music_volume") {
                Some(volume) => Some(AttrEntry::parse(music_volume_attribute(), volume).ok()?),
                None => None,
            },
        })
    }
}

fn resolve_volume(dimension: Option<&Layer>, biome: Option<&Layer>) -> f32 {
    let attr = music_volume_attribute();
    let mut value = attr.default_value().expect("music_volume default");
    if let Some(entry) = dimension.and_then(|l| l.volume.as_ref()) {
        value = entry.apply(&value);
    }
    if let Some(entry) = biome.and_then(|l| l.volume.as_ref()) {
        value = entry.apply(&value);
    }
    match attr.sanitize(value) {
        AttrValue::Float(v) => v,
        _ => 1.0,
    }
}
/// What `Minecraft.getSituationalMusic` and `getMusicVolume` read.
#[derive(Clone, Debug)]
pub struct Situation<'a> {
    /// `Screen.getBackgroundMusic` of the open screen.
    pub screen_music: Option<Music>,
    /// `Minecraft.player != null`.
    pub in_world: bool,
    /// The player's level is the End.
    pub in_end: bool,
    /// `BossHealthOverlay.shouldPlayMusic`: a boss bar asks for music.
    pub boss_music: bool,
    /// `BACKGROUND_MUSIC` at the camera.
    pub background: &'a BackgroundMusic,
    /// `MUSIC_VOLUME` at the camera (1 without a level).
    pub probe_volume: f32,
    /// `abilities.instabuild && abilities.mayfly`.
    pub creative: bool,
    /// `LocalPlayer.isUnderWater`.
    pub underwater: bool,
}
impl Situation<'_> {
    /// `Minecraft.getSituationalMusic`.
    pub fn music(&self) -> Option<Music> {
        if let Some(music) = &self.screen_music {
            return Some(music.clone());
        }
        if !self.in_world {
            return Some(Music::menu());
        }
        if self.in_end && self.boss_music {
            return Some(Music::end_boss());
        }
        self.background
            .select(self.creative, self.underwater)
            .cloned()
    }
    /// `Minecraft.getMusicVolume`.
    pub fn volume(&self) -> f32 {
        if self.screen_music.is_some() {
            1.0
        } else {
            self.probe_volume
        }
    }
}
/// Music attributes by dimension type and biome.
#[derive(Clone, Debug, Default)]
pub struct MusicTables {
    builtin: Arc<Builtin>,
    dimensions: HashMap<String, Layer>,
    biomes: HashMap<String, Layer>,
}
impl MusicTables {
    pub fn load(root: &Path, builtin: Arc<Builtin>) -> Self {
        let read = |dir: &str| -> HashMap<String, Layer> {
            let Ok(entries) = fs::read_dir(root.join("data/minecraft").join(dir)) else {
                return HashMap::new();
            };
            entries
                .flatten()
                .filter_map(|e| {
                    let path = e.path();
                    let name = path.file_stem()?.to_str()?.to_owned();
                    let json: Value = serde_json::from_slice(&fs::read(&path).ok()?).ok()?;
                    Some((format!("minecraft:{name}"), Layer::parse(&json, &builtin)?))
                })
                .collect()
        };
        Self {
            dimensions: read("dimension_type"),
            biomes: read("worldgen/biome"),
            builtin,
        }
    }
    /// `EnvironmentAttributes.BACKGROUND_MUSIC` at a position.
    pub fn background(&self, dimension: &str, biome: &str) -> BackgroundMusic {
        self.biomes
            .get(biome)
            .and_then(|l| l.music.clone())
            .or_else(|| self.dimensions.get(dimension).and_then(|l| l.music.clone()))
            .unwrap_or_default()
    }
    /// `EnvironmentAttributes.MUSIC_VOLUME` at a position (default 1).
    pub fn volume(&self, dimension: &str, biome: &str) -> f32 {
        resolve_volume(self.dimensions.get(dimension), self.biomes.get(biome))
    }
    /// `background` and `volume` in a biome a data pack gave (`pack`, its
    /// file) or the built-in biome `biome`.
    pub fn at(&self, dimension: &str, biome: &str, pack: Option<&Value>) -> (BackgroundMusic, f32) {
        let Some(json) = pack else {
            return (
                self.background(dimension, biome),
                self.volume(dimension, biome),
            );
        };
        let layer = Layer::parse(json, &self.builtin).unwrap_or_default();
        let dimension = self.dimensions.get(dimension);
        let volume = resolve_volume(dimension, Some(&layer));
        (
            layer
                .music
                .or_else(|| dimension.and_then(|l| l.music.clone()))
                .unwrap_or_default(),
            volume,
        )
    }
}
/// `MusicManager.MusicFrequency` (the `musicFrequency` option).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MusicFrequency {
    #[default]
    Default,
    Frequent,
    Constant,
}
impl MusicFrequency {
    pub fn parse(name: &str) -> Self {
        match name {
            "FREQUENT" => Self::Frequent,
            "CONSTANT" => Self::Constant,
            _ => Self::Default,
        }
    }
    fn max_frequency(self) -> i32 {
        match self {
            Self::Default => 20 * 1200,
            Self::Frequent => 10 * 1200,
            Self::Constant => 0,
        }
    }
    fn next_song_delay(self, music: Option<&Music>, random: &mut LegacyRandom) -> i32 {
        let Some(music) = music else {
            return self.max_frequency();
        };
        if self == Self::Constant {
            return 100;
        }
        let min = music.min_delay.min(self.max_frequency());
        let max = music.max_delay.min(self.max_frequency());
        next_int(random, min, max)
    }
}
/// `Mth.nextInt(random, min, max)`: inclusive.
fn next_int(random: &mut LegacyRandom, min: i32, max: i32) -> i32 {
    if min >= max {
        min
    } else {
        random.next_int_bound(max - min + 1).unwrap_or(0) + min
    }
}

/// The layers playing: each a looping music-source instance whose volume
/// eases toward the one its mod asks for, by [`MusicLayers::FADE`] a tick.
/// A layer asked for again after it faded out starts from its beginning.
#[derive(Default)]
pub struct MusicLayers {
    playing: Vec<PlayingLayer>,
}
struct PlayingLayer {
    sound: String,
    id: InstanceId,
    gain: f32,
    target: f32,
    /// The engine has played it: once it isn't active any more (a new level
    /// stopped every sound), the layer starts again.
    heard: bool,
}
impl MusicLayers {
    /// The volume a layer moves by in a tick: two seconds from silent to full.
    pub const FADE: f32 = 0.025;
    /// One client tick with the layers `wanted` (the last of a sound counts).
    /// Layers new this tick start together, silent, so stems listed at once
    /// stay in step.
    pub fn tick(&mut self, wanted: &[MusicLayer], sounds: &mut impl Sounds) {
        let target = |sound: &str| {
            wanted
                .iter()
                .rev()
                .find(|w| w.sound == sound)
                .map_or(0.0, |w| {
                    if w.volume.is_finite() {
                        w.volume.clamp(0.0, 1.0)
                    } else {
                        0.0
                    }
                })
        };
        self.playing.retain(|l| {
            let active = sounds.is_active(l.id);
            if l.heard && !active {
                sounds.release(l.id);
                return false;
            }
            true
        });
        for w in wanted {
            if w.sound.is_empty() || self.playing.iter().any(|l| l.sound == w.sound) {
                continue;
            }
            let instance = SoundInstance::for_music_layer(&w.sound, crate::random_source::create());
            let (id, _) = sounds.play_instance(instance, true);
            self.playing.push(PlayingLayer {
                sound: w.sound.clone(),
                id,
                gain: 0.0,
                target: 0.0,
                heard: false,
            });
        }
        self.playing.retain_mut(|l| {
            l.target = target(&l.sound);
            l.gain = if l.gain < l.target {
                (l.gain + Self::FADE).min(l.target)
            } else {
                (l.gain - Self::FADE).max(l.target)
            };
            // Float steps land a hair off the target.
            if (l.gain - l.target).abs() < 1.0E-4 {
                l.gain = l.target;
            }
            l.heard |= sounds.is_active(l.id);
            if l.gain <= 0.0 && l.target <= 0.0 && !wanted.iter().any(|w| w.sound == l.sound) {
                sounds.stop_instance(l.id);
                sounds.release(l.id);
                return false;
            }
            if let Some(instance) = sounds.engine().instance_mut(l.id) {
                instance.volume = l.gain;
            }
            true
        });
    }
    /// The sound events playing and each one's volume now.
    pub fn playing(&self) -> impl Iterator<Item = (&str, f32)> {
        self.playing.iter().map(|l| (l.sound.as_str(), l.gain))
    }
}

/// `MusicManager`.
pub struct MusicManager {
    random: LegacyRandom,
    /// `currentMusic`: the instance the manager holds.
    current: Option<InstanceId>,
    current_gain: f32,
    next_song_delay: i32,
    toast_shown: bool,
    /// The built-in tracks the Music page turned off (sound ids): every pool
    /// picks around them.
    off: Vec<String>,
    /// `current` was started by the Music page's "play now", not by the
    /// situation: its own music waits for it to end.
    forced: bool,
    pub frequency: MusicFrequency,
    /// `ToastManager.showNowPlayingToast` (true) / `hideNowPlayingToast`
    /// (false) calls since the last drain.
    pub toast_requests: Vec<bool>,
}
impl Default for MusicManager {
    fn default() -> Self {
        // RandomSource.create(): seeded from the clock, not the world.
        Self::new(crate::random_source::unique_seed())
    }
}
impl MusicManager {
    pub fn new(seed: i64) -> Self {
        Self {
            random: LegacyRandom::new(seed),
            current: None,
            current_gain: 1.0,
            next_song_delay: 100,
            toast_shown: false,
            off: Vec::new(),
            forced: false,
            frequency: MusicFrequency::Default,
            toast_requests: Vec::new(),
        }
    }
    /// The sound event of the current track.
    pub fn current<'a>(&self, sounds: &'a SoundEngine) -> Option<&'a str> {
        self.current
            .and_then(|id| sounds.instance(id))
            .map(|i| i.identifier.as_str())
    }
    pub fn current_instance(&self) -> Option<InstanceId> {
        self.current
    }
    pub fn next_song_delay(&self) -> i32 {
        self.next_song_delay
    }
    pub fn current_gain(&self) -> f32 {
        self.current_gain
    }
    pub fn toast_shown(&self) -> bool {
        self.toast_shown
    }
    fn set_current(&mut self, id: Option<InstanceId>, sounds: &mut impl Sounds) {
        if let Some(old) = self.current
            && Some(old) != id
        {
            sounds.release(old);
        }
        self.current = id;
        if id.is_none() {
            self.forced = false;
        }
    }
    /// The tracks the Music page turned off: each pool picks around them, so a
    /// situation whose tracks are all off plays nothing.
    pub fn set_off(&mut self, tracks: &[String]) {
        if self.off != tracks {
            self.off = tracks.to_vec();
        }
    }
    /// `MusicManager.tick` with `Minecraft.getMusicVolume` and
    /// `getSituationalMusic`; `loading` is a `LevelLoadingScreen`.
    pub fn tick(
        &mut self,
        music: Option<&Music>,
        volume: f32,
        loading: bool,
        sounds: &mut impl Sounds,
    ) {
        if self.current.is_some()
            && self.current_gain != volume
            && !self.fade(volume, music, sounds)
        {
            return;
        }
        let Some(music) = music else {
            self.next_song_delay = self.next_song_delay.max(100);
            return;
        };
        if let Some(current) = self.current {
            // canReplace
            let playing = sounds
                .engine()
                .instance(current)
                .map(|i| i.identifier.clone());
            if music.replace_current_music
                && !self.forced
                && playing.as_deref() != Some(music.sound.as_str())
            {
                sounds.stop_instance(current);
                self.next_song_delay = next_int(&mut self.random, 0, music.min_delay / 2);
            }
            if !sounds.is_active(current) {
                self.set_current(None, sounds);
                let delay = self
                    .frequency
                    .next_song_delay(Some(music), &mut self.random);
                self.next_song_delay = self.next_song_delay.min(delay);
            }
        }
        self.next_song_delay = self.next_song_delay.min(music.max_delay);
        if self.current.is_none() && !loading {
            self.next_song_delay -= 1;
            if self.next_song_delay <= 0 {
                self.start_playing(music, sounds);
            }
        }
    }
    /// `MusicManager.startPlaying`, with the Music page's off tracks left out
    /// of the pool.
    pub fn start_playing(&mut self, music: &Music, sounds: &mut impl Sounds) {
        self.start_playing_with(music, None, sounds);
    }
    /// Play one track of `event` now (a Music page row): it takes the
    /// manager's slot, and the situation's own music waits until it ends.
    /// False when the event has no such track.
    pub fn play_track(&mut self, event: &str, track: &str, sounds: &mut impl Sounds) -> bool {
        let music = Music::new(event, 0, 0, true);
        self.start_playing_with(&music, Some(track), sounds)
    }
    /// `MusicManager.startPlaying`, choosing the concrete sound with `only`
    /// (that track alone) or else `off` left out.
    fn start_playing_with(
        &mut self,
        music: &Music,
        only: Option<&str>,
        sounds: &mut impl Sounds,
    ) -> bool {
        let mut instance = SoundInstance::for_music(&music.sound, crate::random_source::create());
        // No track off and no row to play: vanilla's startPlaying as it is.
        // `resolve` picks the track, and an event with nothing to play (or no
        // event) is still the current music, with the delay at its maximum.
        if only.is_none() && self.off.is_empty() {
            return self.play_current(instance, only, sounds);
        }
        // The event's own random picks the track, as `resolve` would.
        let mut random = instance.random.clone();
        let sound = sounds
            .engine()
            .registry
            .sound_except(&music.sound, &mut random, &|id| match only {
                Some(track) => id != track,
                // A row's On/Off is per track, everywhere it plays.
                None => self.off.iter().any(|off| off == id),
            });
        instance.random = random;
        let Some(sound) = sound.filter(|s| s.sentinel == Sentinel::None) else {
            // Nothing left to play (every track of the situation is off, or
            // the event is unknown): retry after a normal delay, so turning
            // a track back on is heard without reloading the music.
            if only.is_none() {
                self.next_song_delay = self
                    .frequency
                    .next_song_delay(Some(music), &mut self.random);
            }
            return false;
        };
        // The manager resolved the sound itself; `resolve` keeps it.
        instance.sound = Some(sound);
        instance.event = Some(None);
        instance.chosen = true;
        self.play_current(instance, only, sounds)
    }
    /// The rest of `MusicManager.startPlaying`: play `instance` as the current
    /// music (a Music page row replaces the track already playing).
    fn play_current(
        &mut self,
        instance: SoundInstance,
        only: Option<&str>,
        sounds: &mut impl Sounds,
    ) -> bool {
        // "Play now" takes the manager's slot: the track already playing stops
        // (a situation's own track is replaced by `set_current` below).
        if only.is_some()
            && let Some(current) = self.current
        {
            sounds.stop_instance(current);
            self.set_current(None, sounds);
        }
        let (id, result) = sounds.play_instance(instance, true);
        self.set_current(Some(id), sounds);
        self.forced = only.is_some();
        match result {
            PlayResult::Started => {
                self.toast_requests.push(true);
                self.toast_shown = true;
            }
            PlayResult::StartedSilently => self.toast_shown = false,
            PlayResult::NotStarted => {}
        }
        self.next_song_delay = i32::MAX;
        true
    }
    /// `MusicManager.showNowPlayingToastIfNeeded`.
    pub fn show_now_playing_toast_if_needed(&mut self) {
        if !self.toast_shown {
            self.toast_requests.push(true);
            self.toast_shown = true;
        }
    }
    /// `MusicManager.stopPlaying(Music)`.
    pub fn stop_playing_music(
        &mut self,
        music: &Music,
        situational: Option<&Music>,
        sounds: &mut impl Sounds,
    ) {
        if self.is_playing_music(music, sounds.engine()) {
            self.stop_playing(situational, sounds);
        }
    }
    /// `MusicManager.stopPlaying()`.
    pub fn stop_playing(&mut self, situational: Option<&Music>, sounds: &mut impl Sounds) {
        if let Some(current) = self.current {
            sounds.stop_instance(current);
            self.set_current(None, sounds);
            self.toast_requests.push(false);
        }
        self.next_song_delay = self
            .frequency
            .next_song_delay(situational, &mut self.random)
            + 100;
    }
    /// `MusicManager.isPlayingMusic`.
    pub fn is_playing_music(&self, music: &Music, sounds: &SoundEngine) -> bool {
        self.current(sounds) == Some(music.sound.as_str())
    }
    /// `MusicManager.setMinutesBetweenSongs`.
    pub fn set_frequency(&mut self, frequency: MusicFrequency, situational: Option<&Music>) {
        self.frequency = frequency;
        self.next_song_delay = self
            .frequency
            .next_song_delay(situational, &mut self.random);
    }
    /// `MusicManager.fadePlaying`: false once the track faded out and stopped.
    fn fade(&mut self, volume: f32, situational: Option<&Music>, sounds: &mut impl Sounds) -> bool {
        if self.current.is_none() {
            return false;
        }
        if self.current_gain == volume {
            return true;
        }
        if self.current_gain < volume {
            self.current_gain += self.current_gain.clamp(5.0E-4, 0.005);
            if self.current_gain > volume {
                self.current_gain = volume;
            }
        } else {
            self.current_gain = 0.03 * volume + 0.97 * self.current_gain;
            if (self.current_gain - volume).abs() < 1.0E-4 || self.current_gain < volume {
                self.current_gain = volume;
            }
        }
        self.current_gain = self.current_gain.clamp(0.0, 1.0);
        if self.current_gain <= 1.0E-4 {
            self.stop_playing(situational, sounds);
            false
        } else {
            sounds.update_category_volume(MUSIC, self.current_gain);
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sound_engine::{Bound, test_backend::Null};
    use crate::sound_events::SoundRegistry;

    fn engine() -> SoundEngine {
        let mut registry = SoundRegistry::default();
        registry.add_file(
            "minecraft",
            br#"{"music.menu": {"sounds": [{"name": "m", "stream": true}]},
                 "music.game": {"sounds": [{"name": "g", "stream": true}]},
                 "music.end": {"sounds": [{"name": "e", "stream": true}]}}"#,
            &|_, _| true,
        );
        SoundEngine::new(registry)
    }

    /// A data pack's biome file replaces the built-in biome's attributes:
    /// what it leaves out comes from the dimension, not the built-in biome.
    #[test]
    fn a_pack_biome_reads_its_own_file_over_the_dimension() {
        use serde_json::json;
        fn layer_of(sound: &str, volume: serde_json::Value) -> serde_json::Value {
            json!({"attributes": {
                "minecraft:audio/background_music": {"default": {
                    "sound": {"sound_id": sound}, "min_delay": 1, "max_delay": 2
                }},
                "minecraft:audio/music_volume": volume
            }})
        }
        let builtin = Arc::new(Builtin::default());
        let tables = MusicTables {
            dimensions: HashMap::from([(
                "minecraft:overworld".to_owned(),
                Layer::parse(&layer_of("test:game", 0.8.into()), &builtin).unwrap(),
            )]),
            biomes: HashMap::from([(
                "minecraft:plains".to_owned(),
                Layer::parse(&layer_of("test:meadow", 0.3.into()), &builtin).unwrap(),
            )]),
            builtin,
        };
        let sound =
            |(music, volume): (BackgroundMusic, f32)| (music.default.map(|m| m.sound), volume);
        let builtin = tables.at("minecraft:overworld", "minecraft:plains", None);
        assert_eq!(sound(builtin), (Some("test:meadow".to_owned()), 0.3));
        let mut pack = layer_of("test:dunes", 0.7.into());
        let packed = tables.at("minecraft:overworld", "minecraft:plains", Some(&pack));
        assert_eq!(sound(packed), (Some("test:dunes".to_owned()), 0.7));
        for bad in [json!(null), json!({"min_delay": 0, "max_delay": 5})] {
            pack["attributes"]["minecraft:audio/background_music"]["creative"] = bad;
            let packed = tables.at("minecraft:overworld", "minecraft:plains", Some(&pack));
            assert_eq!(sound(packed), (Some("test:game".to_owned()), 0.8));
        }
        let bare = json!({"temperature": 0.8});
        let packed = tables.at("minecraft:overworld", "minecraft:plains", Some(&bare));
        assert_eq!(sound(packed), (Some("test:game".to_owned()), 0.8));

        // Valid float modifiers apply over the inherited dimension volume and
        // sanitize the final result; out-of-range raw overrides are rejected.
        let modified = layer_of(
            "test:dunes",
            json!({"modifier": "multiply", "argument": 0.5}),
        );
        let packed = tables.at("minecraft:overworld", "minecraft:plains", Some(&modified));
        assert_eq!(sound(packed), (Some("test:dunes".to_owned()), 0.4));

        let saturated = layer_of("test:dunes", json!({"modifier": "add", "argument": 0.5}));
        let packed = tables.at("minecraft:overworld", "minecraft:plains", Some(&saturated));
        assert_eq!(sound(packed), (Some("test:dunes".to_owned()), 1.0));

        for raw_bad in [json!(1.5), json!(-0.1)] {
            let bad = layer_of("test:dunes", raw_bad);
            assert!(Layer::parse(&bad, &tables.builtin).is_none());
            let packed = tables.at("minecraft:overworld", "minecraft:plains", Some(&bad));
            assert_eq!(sound(packed), (Some("test:game".to_owned()), 0.8));
        }
    }

    #[test]
    fn audio_music_codec_requires_valid_delays_and_sound_holders() {
        use mod_api::client::music::parse_sound_event;
        use serde_json::json;

        let registered = |sound: &str| sound == "minecraft:music.game";
        let base = json!({"sound":"music.game", "min_delay":0, "max_delay":0});
        assert_eq!(
            Music::parse(&base, &registered),
            Some(Music::new("minecraft:music.game", 0, 0, false))
        );
        let mut inline = json!({
            "sound": {"sound_id":"test:track", "range":32.0},
            "min_delay":10, "max_delay":20, "replace_current_music":true
        });
        assert_eq!(
            parse_sound_event(&inline["sound"], &registered),
            Some(("test:track".to_owned(), Some(32.0)))
        );
        assert_eq!(
            Music::parse(&inline, &registered),
            Some(Music::new("test:track", 10, 20, true))
        );
        inline["sound"]["range"] = json!("not a number");
        assert_eq!(
            parse_sound_event(&inline["sound"], &registered),
            Some(("test:track".to_owned(), None))
        );
        assert_eq!(
            Music::parse(&inline, &registered),
            Some(Music::new("test:track", 10, 20, true))
        );
        for (field, bad) in [
            ("min_delay", json!(-1)),
            ("max_delay", json!(-1)),
            ("max_delay", json!(2147483648_i64)),
            ("min_delay", json!("0")),
            ("sound", json!("test:unknown")),
            ("sound", json!({"range": 8})),
            ("replace_current_music", json!(null)),
        ] {
            let mut input = base.clone();
            input[field] = bad;
            assert!(Music::parse(&input, &registered).is_none(), "{input}");
        }
        for field in ["min_delay", "max_delay", "sound"] {
            let mut input = base.clone();
            input.as_object_mut().unwrap().remove(field);
            assert!(Music::parse(&input, &registered).is_none(), "{input}");
        }
        assert_eq!(
            BackgroundMusic::parse(&json!({}), &registered),
            Some(BackgroundMusic::default())
        );
        assert!(
            BackgroundMusic::parse(&json!({"default":base, "creative":null}), &registered)
                .is_none()
        );
    }

    fn layer(sound: &str, volume: f32) -> MusicLayer {
        MusicLayer {
            sound: sound.to_owned(),
            volume,
        }
    }

    #[test]
    fn layers_fade_in_together_follow_their_volume_and_fade_out() {
        let (mut engine, mut backend) = (engine(), Null::default());
        let mut layers = MusicLayers::default();
        let mut tick = |layers: &mut MusicLayers, wanted: &[MusicLayer]| {
            layers.tick(
                wanted,
                &mut Bound {
                    engine: &mut engine,
                    backend: &mut backend,
                },
            );
        };
        let playing = |layers: &MusicLayers| -> Vec<(String, f32)> {
            layers
                .playing()
                .map(|(s, g)| (s.to_owned(), (g * 1000.0).round() / 1000.0))
                .collect()
        };
        // Two stems, one silent: both start in the same tick.
        let both = [
            layer("minecraft:music.game", 1.0),
            layer("minecraft:music.end", 0.0),
        ];
        tick(&mut layers, &both);
        assert_eq!(
            playing(&layers),
            [
                ("minecraft:music.game".into(), 0.025),
                ("minecraft:music.end".into(), 0.0)
            ]
        );
        for _ in 0..50 {
            tick(&mut layers, &both);
        }
        assert_eq!(playing(&layers)[0].1, 1.0);
        // The second stem rises toward its new volume, by FADE a tick.
        let fuller = [
            layer("minecraft:music.game", 1.0),
            layer("minecraft:music.end", 0.5),
        ];
        for _ in 0..10 {
            tick(&mut layers, &fuller);
        }
        assert_eq!(playing(&layers)[1].1, 0.25);
        // A layer left out fades out, then stops.
        let one = [layer("minecraft:music.end", 0.5)];
        for _ in 0..39 {
            tick(&mut layers, &one);
        }
        assert_eq!(playing(&layers).len(), 2);
        tick(&mut layers, &one);
        assert_eq!(playing(&layers), [("minecraft:music.end".into(), 0.5)]);
    }

    #[test]
    fn layer_volumes_out_of_range_are_clamped() {
        let (mut engine, mut backend) = (engine(), Null::default());
        let mut layers = MusicLayers::default();
        for volume in [7.0, f32::NAN] {
            for _ in 0..60 {
                layers.tick(
                    &[layer("minecraft:music.game", volume)],
                    &mut Bound {
                        engine: &mut engine,
                        backend: &mut backend,
                    },
                );
            }
            let gain = layers.playing().next().map(|(_, g)| g);
            assert_eq!(gain, Some(if volume.is_nan() { 0.0 } else { 1.0 }));
        }
    }

    #[test]
    fn menu_music_starts_within_its_delay_and_game_music_replaces_nothing() {
        let (mut engine, mut backend) = (engine(), Null::default());
        let mut m = MusicManager::new(1);
        let menu = Music::menu();
        let tick =
            |m: &mut MusicManager, music: &Music, engine: &mut SoundEngine, backend: &mut Null| {
                m.tick(Some(music), 1.0, false, &mut Bound { engine, backend });
            };
        // nextSongDelay starts at 100, capped to MENU's max delay 600: starts on tick 100.
        for _ in 0..99 {
            tick(&mut m, &menu, &mut engine, &mut backend);
        }
        assert!(m.current(&engine).is_none());
        tick(&mut m, &menu, &mut engine, &mut backend);
        assert_eq!(m.current(&engine), Some("minecraft:music.menu"));
        // Game music does not replace the current track.
        let game = Music::new("minecraft:music.game", 12000, 24000, false);
        tick(&mut m, &game, &mut engine, &mut backend);
        assert_eq!(m.current(&engine), Some("minecraft:music.menu"));
    }

    #[test]
    fn a_new_level_stops_menu_music_and_waits_for_game_music() {
        let (mut engine, mut backend) = (engine(), Null::default());
        let mut m = MusicManager::new(2);
        let menu = Music::menu();
        m.start_playing(
            &menu,
            &mut Bound {
                engine: &mut engine,
                backend: &mut backend,
            },
        );
        // Minecraft.setLevel -> SoundManager.stop(); the next tick has a player.
        engine.stop_all(&mut backend);
        let game = Music::game();
        m.tick(
            Some(&game),
            1.0,
            true,
            &mut Bound {
                engine: &mut engine,
                backend: &mut backend,
            },
        );
        assert!(m.current_instance().is_none());
        assert!((12000..=24000).contains(&m.next_song_delay()));
    }

    #[test]
    fn zero_music_volume_fades_out_and_stops() {
        let (mut engine, mut backend) = (engine(), Null::default());
        let mut m = MusicManager::new(3);
        let game = Music::new("minecraft:music.game", 12000, 24000, false);
        m.start_playing(
            &game,
            &mut Bound {
                engine: &mut engine,
                backend: &mut backend,
            },
        );
        let mut ticks = 0;
        while m.current_instance().is_some() && ticks < 1000 {
            m.tick(
                Some(&game),
                0.0,
                false,
                &mut Bound {
                    engine: &mut engine,
                    backend: &mut backend,
                },
            );
            ticks += 1;
        }
        // 0.97^n < 1e-4 after ~303 ticks.
        assert!((290..=310).contains(&ticks), "{ticks}");
    }

    #[test]
    fn select_prefers_underwater_then_creative() {
        let bg = BackgroundMusic {
            default: Some(Music::new("a", 0, 0, false)),
            creative: Some(Music::new("b", 0, 0, false)),
            underwater: Some(Music::new("c", 0, 0, false)),
        };
        assert_eq!(bg.select(true, true).unwrap().sound, "c");
        assert_eq!(bg.select(true, false).unwrap().sound, "b");
        assert_eq!(bg.select(false, false).unwrap().sound, "a");
    }

    #[test]
    fn the_music_page_lists_every_track_once_from_a_pool_that_plays_it() {
        let mut registry = SoundRegistry::default();
        registry.add_file(
            "minecraft",
            br#"{
                "music.game": {"sounds": ["music/game/shared", "music/game/second"]},
                "music.menu": {"sounds": ["music/menu/one"]},
                "music.overworld.forest": {"sounds": ["music/game/shared", "music/game/bromeliad"]},
                "music.overworld.swamp": {"sounds": ["music/game/bromeliad", "music/game/aerie"]},
                "music.nether.warped_forest": {"sounds": ["music/game/rubedo"]},
                "music_disc.cat": {"sounds": ["records/cat"]},
                "note": {"sounds": ["block/note/harp"]}
            }"#,
            &|_, _| true,
        );
        let groups = track_groups(&registry);
        let summary: Vec<(&str, Vec<&str>)> = groups
            .iter()
            .map(|group| {
                (
                    group.label.as_str(),
                    group
                        .tracks
                        .iter()
                        .map(|track| track.id.rsplit('/').next().unwrap())
                        .collect(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("sunrise.pages.music.menu", vec!["one"]),
                ("sunrise.pages.music.overworld", vec!["shared", "second"]),
                // A biome pool's own track, from the pool that plays it.
                (
                    "sunrise.pages.music.overworld_biomes",
                    vec!["bromeliad", "aerie"]
                ),
                ("sunrise.pages.music.nether_biomes", vec!["rubedo"]),
            ]
        );
        // Each row plays from a pool that has the track; no track repeats.
        let mut seen = Vec::new();
        for group in &groups {
            for track in &group.tracks {
                assert!(!seen.contains(&track.id), "{} listed twice", track.id);
                seen.push(track.id.clone());
                assert!(
                    registry.tracks(&track.event).contains(&track.id),
                    "{} does not play {}",
                    track.event,
                    track.id
                );
            }
        }
        // Records and other events are not music.
        assert!(seen.iter().all(|id| !id.contains("records/")));
    }

    #[test]
    fn track_groups_orders_all_builtin_situations() {
        let mut registry = SoundRegistry::default();
        registry.add_file(
            "minecraft",
            br#"{
                "music.credits": {"sounds": ["music/game/end/credits"]},
                "music.dragon": {"sounds": ["music/game/end/boss"]},
                "music.end": {"sounds": ["music/game/end/end"]},
                "music.under_water": {"sounds": ["music/game/water/axolotl"]},
                "music.creative": {"sounds": ["music/game/creative/creative1"]},
                "music.game": {"sounds": ["music/game/calm1"]},
                "music.menu": {"sounds": ["music/menu/menu1"]}
            }"#,
            &|_, _| true,
        );
        let groups = track_groups(&registry);
        let actual: Vec<(&str, &str)> = groups
            .iter()
            .map(|group| (group.label.as_str(), group.tracks[0].event.as_str()))
            .collect();
        assert_eq!(
            actual,
            [
                ("sunrise.pages.music.menu", "minecraft:music.menu"),
                ("sunrise.pages.music.overworld", "minecraft:music.game"),
                ("sunrise.pages.music.creative", "minecraft:music.creative"),
                (
                    "sunrise.pages.music.underwater",
                    "minecraft:music.under_water"
                ),
                ("sunrise.pages.music.end", "minecraft:music.end"),
                ("sunrise.pages.music.boss", "minecraft:music.dragon"),
                ("sunrise.pages.music.credits", "minecraft:music.credits"),
            ]
        );
    }

    /// The overworld pool, two tracks, and a menu track.
    fn two_tracks() -> SoundEngine {
        let mut registry = SoundRegistry::default();
        registry.add_file(
            "minecraft",
            br#"{"music.game": {"sounds": [
                    {"name": "music/game/one", "stream": true},
                    {"name": "music/game/two", "stream": true}]},
                 "music.menu": {"sounds": [{"name": "music/menu/one", "stream": true}]}}"#,
            &|_, _| true,
        );
        SoundEngine::new(registry)
    }
    fn played(m: &MusicManager, engine: &SoundEngine) -> Option<String> {
        let id = m.current_instance()?;
        let sound = engine.instance(id)?.sound.clone()?;
        Some(sound.location)
    }

    #[test]
    fn a_track_turned_off_leaves_the_pool() {
        let (mut engine, mut backend) = (two_tracks(), Null::default());
        let mut m = MusicManager::new(11);
        m.set_off(&["minecraft:music/game/one".to_owned()]);
        m.start_playing(
            &Music::game(),
            &mut Bound {
                engine: &mut engine,
                backend: &mut backend,
            },
        );
        assert_eq!(
            played(&m, &engine).as_deref(),
            Some("minecraft:music/game/two")
        );
        // `current` is the event, as `isPlayingMusic` reads it.
        assert_eq!(m.current(&engine), Some("minecraft:music.game"));
    }

    #[test]
    fn a_situation_with_every_track_off_plays_nothing_and_tries_later() {
        let (mut engine, mut backend) = (two_tracks(), Null::default());
        let mut m = MusicManager::new(12);
        m.set_off(&[
            "minecraft:music/game/one".to_owned(),
            "minecraft:music/game/two".to_owned(),
        ]);
        m.start_playing(
            &Music::game(),
            &mut Bound {
                engine: &mut engine,
                backend: &mut backend,
            },
        );
        assert!(m.current_instance().is_none());
        // A retry after the usual delay, so turning one back on is heard.
        assert!((12000..=24000).contains(&m.next_song_delay()));
    }

    #[test]
    fn play_now_plays_that_track_and_the_situation_waits() {
        let (mut engine, mut backend) = (two_tracks(), Null::default());
        let mut m = MusicManager::new(13);
        m.start_playing(
            &Music::menu(),
            &mut Bound {
                engine: &mut engine,
                backend: &mut backend,
            },
        );
        let menu = m.current_instance().expect("menu music started");
        // Even a track turned off plays when asked to, by hand, and it takes
        // the slot from the track already playing.
        m.set_off(&["minecraft:music/game/two".to_owned()]);
        assert!(m.play_track(
            "minecraft:music.game",
            "minecraft:music/game/two",
            &mut Bound {
                engine: &mut engine,
                backend: &mut backend,
            },
        ));
        // The slot's old track is told to stop as soon as it is replaced.
        assert_ne!(m.current_instance(), Some(menu), "the slot changed");
        assert!(
            backend.stopped.values().any(|stopped| *stopped),
            "the old channel stopped"
        );
        assert_eq!(
            played(&m, &engine).as_deref(),
            Some("minecraft:music/game/two")
        );
        // Menu music replaces what plays; a "play now" track keeps its slot.
        for _ in 0..3 {
            m.tick(
                Some(&Music::menu()),
                1.0,
                false,
                &mut Bound {
                    engine: &mut engine,
                    backend: &mut backend,
                },
            );
        }
        assert_eq!(
            played(&m, &engine).as_deref(),
            Some("minecraft:music/game/two")
        );
        // Once it ends, the situation's own music takes over again.
        engine.stop_all(&mut backend);
        m.tick(
            Some(&Music::menu()),
            1.0,
            false,
            &mut Bound {
                engine: &mut engine,
                backend: &mut backend,
            },
        );
        assert!(m.current_instance().is_none());
    }

    #[test]
    fn play_now_of_a_track_the_event_lacks_plays_nothing() {
        let (mut engine, mut backend) = (two_tracks(), Null::default());
        let mut m = MusicManager::new(14);
        assert!(!m.play_track(
            "minecraft:music.game",
            "minecraft:music/game/absent",
            &mut Bound {
                engine: &mut engine,
                backend: &mut backend,
            },
        ));
        assert!(m.current_instance().is_none());
    }
}
