//! Sound playback: vanilla's `SoundEngine` bookkeeping (`sound_engine`) over a
//! small mixer standing in for OpenAL: one channel per source with its gain,
//! pitch, linear distance attenuation (`AL_LINEAR_DISTANCE`, reference 0,
//! rolloff 1), looping, relative flag and source state, Ogg/Vorbis decoding
//! (streams on a worker thread) and OpenAL Soft's mixing: mono sources panned
//! and attenuated, multi-channel buffers (stereo music and records) played
//! direct as OpenAL does.
mod mic;
mod streams;

use crate::Result;
use crate::sound_engine::{
    Attenuation, Backend, Bound, ChannelId, InstanceId, PlayResult, SoundEngine, SoundInstance,
    Sounds, TickContext,
};
use crate::sound_events::{Sound, SoundRegistry};
#[cfg(feature = "audio")]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
#[cfg(any(feature = "audio", test))]
use std::path::Path;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use sunrise_assets::resource_source::{Resource, ResourceSource};
use sunrise_core::random::{LegacyRandom, RandomSource};

mod fx;
mod reverb;

/// An OpenAL source's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Initial,
    Playing,
    Paused,
    Stopped,
}
/// An OpenAL source.
struct Channel {
    samples: Option<(Arc<Vec<f32>>, u32, u16)>,
    position: f64,
    state: State,
    gain: f32,
    pitch: f32,
    /// `AL_LINEAR_DISTANCE` max distance, or no attenuation.
    linear: Option<f32>,
    looping: bool,
    at: [f32; 3],
    relative: bool,
    /// Sound owner: 0 for the primary backend; other ids come from `Audio::share`.
    owner: u8,
    /// Its world (`Mixer::world_id`; 0 none).
    world: u32,
    /// The sound file and start time, used to spot copies from other backends.
    started: Option<(String, std::time::Instant)>,
    /// Its sound event plays clearly through a mod's muffle (`Audio::muffle`).
    unmuffled: bool,
    /// Its own effects (`SoundPlay::effects`) and their filters' state.
    fx: fx::VoiceFx,
}
/// The cutoff of the muffle's low-pass at full strength: the world heard through ringing ears.
const MUFFLED_CUTOFF: f32 = 300.0;
/// The cutoff it sweeps down from as its strength rises.
const OPEN_CUTOFF: f32 = 20_000.0;
/// How long the muffle takes to glide to a new level: a game tick's step (50 ms) would click.
const MUFFLE_GLIDE: f32 = 0.02;
/// A mod's muffle (`Audio::muffle`) in the mixer: the other sounds are mixed apart, low-passed by
/// two one-pole stages and turned down, then added to the sounds it leaves clear. While it is off
/// (never set, or faded out) the sounds mix straight into the output, as without it.
#[derive(Default)]
struct Muffle {
    /// `[amount, lowpass]`, each 0 to 1, as the game's tick last set them.
    target: [f32; 2],
    /// Where the output is now, gliding to `target`.
    current: [f32; 2],
    /// The low-pass stages' state for the left and right output channels.
    stages: [[f32; 2]; 2],
    /// The muffled sounds of one buffer, reused.
    bus: Vec<f32>,
}
impl Muffle {
    fn active(&self) -> bool {
        self.target != [0.0; 2] || self.current.iter().any(|v| *v > 1e-4)
    }
    /// Adds `bus` (the muffled sounds) to `data`, low-passed and turned down.
    fn apply(&mut self, bus: &[f32], data: &mut [f32], channels: usize, rate: u32) {
        let rate = rate as f32;
        let glide = 1.0 - (-1.0 / (MUFFLE_GLIDE * rate)).exp();
        // The filter coefficient only depends on `lowpass`, which glides every frame but sits
        // still once it has reached its target: reuse the last one while it hasn't moved, so a
        // settled muffle pays for `powf` and `exp` once per buffer rather than once per frame.
        // NaN never equals, so the first frame always computes.
        let mut coefficient = (f32::NAN, 0.0f32);
        for (frame, muffled) in data.chunks_mut(channels).zip(bus.chunks(channels)) {
            for (current, target) in self.current.iter_mut().zip(self.target) {
                *current += (target - *current) * glide;
            }
            let [amount, lowpass] = self.current;
            let gain = 1.0 - amount;
            if coefficient.0 != lowpass {
                let cutoff = OPEN_CUTOFF * (MUFFLED_CUTOFF / OPEN_CUTOFF).powf(lowpass);
                coefficient = (
                    lowpass,
                    1.0 - (-std::f32::consts::TAU * cutoff / rate).exp(),
                );
            }
            let a = coefficient.1;
            for ((out, s), stage) in frame.iter_mut().zip(muffled).zip(&mut self.stages) {
                stage[0] += (s - stage[0]) * a;
                stage[1] += (stage[0] - stage[1]) * a;
                // Dry at no strength, so a muffle that only turns sounds down leaves them as they were.
                *out += (s + (stage[1] - s) * lowpass) * gain;
            }
        }
    }
}
/// The most frames one [`mix`] takes. The reverbs' send buffer is reserved for this many when the
/// mixer is created, so the output callback never grows it: a device buffer larger than this (a
/// rare default of 85 ms or more at 192 kHz) is mixed in pieces of this size.
const MAX_MIX_FRAMES: usize = 16_384;
/// The shared reverbs and what the sounds send to them in one buffer.
///
/// Everything it holds is allocated when the mixer is created (`Mixer::default`, on the game
/// thread that opens the sound system), not on the first send and never in the output callback:
/// each bus's delay lines are sized for 192 kHz ([`reverb::Reverb::new`]), 236,071 samples for
/// the three buses together (944,284 bytes), and the send buffer holds [`MAX_MIX_FRAMES`] frames
/// for each bus (196,608 bytes): 1,140,892 bytes (about 1.09 MiB) in all, whatever the output's
/// rate. A slower output uses the start of each line.
struct Reverbs {
    buses: [reverb::Reverb; reverb::BUSES],
    /// One buffer's sends, bus after bus, one mono sample a frame; reused.
    input: Vec<f32>,
    /// `input` is cleared for this buffer.
    ready: bool,
    /// What was sent to each bus this buffer.
    sent: [bool; reverb::BUSES],
}
impl Default for Reverbs {
    fn default() -> Self {
        Self {
            buses: reverb::ROOMS.map(reverb::Reverb::new),
            // Room for the largest piece `mix` takes, so the callback doesn't allocate.
            input: Vec::with_capacity(reverb::BUSES * MAX_MIX_FRAMES),
            ready: false,
            sent: [false; reverb::BUSES],
        }
    }
}
impl Reverbs {
    /// The sends of a buffer of `frames` frames, cleared at the first ask.
    fn input(&mut self, frames: usize) -> &mut [f32] {
        debug_assert!(
            frames <= MAX_MIX_FRAMES,
            "a mix piece is at most MAX_MIX_FRAMES"
        );
        if !self.ready {
            self.input.clear();
            self.input.resize(reverb::BUSES * frames, 0.0);
            self.ready = true;
        }
        &mut self.input
    }
    /// Adds every bus's reverb of this buffer's sends into `out`, then readies the next buffer.
    /// A bus with no sends and no tail left costs nothing.
    fn run(&mut self, out: &mut [f32], channels: usize, rate: u32) {
        let frames = out.len() / channels.max(1);
        if self.ready || self.buses.iter().any(|b| !b.idle()) {
            self.input(frames);
            for ((bus, input), sent) in self
                .buses
                .iter_mut()
                .zip(self.input.chunks(frames.max(1)))
                .zip(self.sent)
            {
                bus.run(input, sent, out, channels, rate);
            }
        }
        self.ready = false;
        self.sent = [false; reverb::BUSES];
    }
}
#[derive(Default)]
struct Mixer {
    channels: HashMap<ChannelId, Channel>,
    listener: [f64; 3],
    listener_orientation: ListenerOrientation,
    /// Additional listeners' cameras by owner. A world sound is heard from the nearest camera,
    /// and panned half as wide while several share the speakers.
    listeners: HashMap<u8, ([f64; 3], ListenerOrientation)>,
    /// Each camera's world by owner, as ids: the callback compares and copies integers.
    worlds: HashMap<u8, u32>,
    /// World names by id - 1 (`world_id`).
    world_names: Vec<String>,
    /// The callback's cameras (position, basis, world), kept to be refilled without allocating.
    cameras: Vec<([f64; 3], Basis, u32)>,
    /// Per-source gain/pan/step cache for one buffer, reused across callbacks.
    voices: Vec<(ChannelId, [f32; 3], f64)>,
    /// Mods' audio streams, separate from vanilla's channels.
    streams: Vec<streams::StreamVoice>,
    /// Each `SoundSource`'s volume for a stream: master times the group, as every sound's.
    stream_gain: [f32; 11],
    /// A mod's muffle of the sounds it does not exempt.
    muffle: Muffle,
    /// The shared reverbs sounds send to (`SoundPlay::effects`).
    reverbs: Reverbs,
    limiter_peak: f32,
    /// The output device the stream was opened on (`Library.currentDeviceName`).
    #[cfg(feature = "audio")]
    device_name: String,
    /// The stream reported its device gone (`Library.isCurrentDeviceDisconnected`).
    #[cfg(feature = "audio")]
    device_lost: bool,
    /// The stream's recent backend errors, which tell a dead stream from a hiccup.
    #[cfg(feature = "audio")]
    stream_errors: StreamErrors,
}

/// A stream's backend errors close together (`StreamErrors::dead`).
#[cfg(any(feature = "audio", test))]
#[derive(Default)]
struct StreamErrors {
    /// When the current run of errors began, and how many it holds.
    since: Option<std::time::Instant>,
    count: u32,
}
#[cfg(any(feature = "audio", test))]
impl StreamErrors {
    /// Errors this close together...
    const WINDOW: std::time::Duration = std::time::Duration::from_secs(2);
    /// ...this many of them: the stream is dead. Under vanilla a device that fails unrecoverably
    /// reads as disconnected (`ALC_CONNECTED`, the `ALC_EXT_disconnect` extension), and
    /// `SoundEngine` opens the output again; cpal reports a dead ALSA stream (`alsa::poll()`
    /// returning POLLERR, over and over) as a backend error like any other, so a run of them
    /// stands for it. One alone is a hiccup the stream recovers from.
    const DEAD_AFTER: u32 = 3;
    /// One more backend error at `now`: whether the stream counts as dead.
    fn dead(&mut self, now: std::time::Instant) -> bool {
        match self.since {
            Some(since) if now.duration_since(since) <= Self::WINDOW => self.count += 1,
            _ => {
                self.since = Some(now);
                self.count = 1;
            }
        }
        self.count >= Self::DEAD_AFTER
    }
}

/// Opening the output again after it failed: a few more tries, a couple of seconds apart, so a
/// device that is gone for good isn't asked forever.
#[cfg(any(feature = "audio", test))]
#[derive(Default, Debug, PartialEq)]
struct Reopen {
    /// The tries left; 0 while the output plays, or after the last try.
    tries: u32,
    /// Ticks since the last try.
    ticks: u32,
}
#[cfg(any(feature = "audio", test))]
impl Reopen {
    const TRIES: u32 = 5;
    /// Two seconds of client ticks.
    const TICKS: u32 = 40;
    /// The output failed to open: whether to keep trying.
    fn failed(&mut self, retrying: bool) -> bool {
        self.ticks = 0;
        self.tries = if retrying {
            self.tries.saturating_sub(1)
        } else {
            Self::TRIES
        };
        self.tries > 0
    }
    /// A tick while tries are left: whether to try now.
    fn due(&mut self) -> bool {
        if self.tries == 0 {
            return false;
        }
        self.ticks += 1;
        self.ticks >= Self::TICKS
    }
}
/// A camera's forward, up and right vectors.
type Basis = ([f64; 3], [f64; 3], [f64; 3]);

#[derive(Clone, Copy, PartialEq)]
struct ListenerOrientation {
    forward: [f32; 3],
    up: [f32; 3],
}
impl Default for ListenerOrientation {
    fn default() -> Self {
        Self {
            forward: [0.0, 0.0, 1.0],
            up: [0.0, 1.0, 0.0],
        }
    }
}
impl ListenerOrientation {
    fn basis(self) -> Basis {
        // `Listener.setTransform` takes the camera's forward and up. OpenAL gets right from
        // their cross product; the up component carries camera roll.
        let forward = glam::DVec3::from_array(self.forward.map(f64::from));
        let up = glam::DVec3::from_array(self.up.map(f64::from));
        (
            forward.to_array(),
            up.to_array(),
            forward.cross(up).to_array(),
        )
    }
}

impl Mixer {
    /// The id of world `name` (1 and up), given once per name.
    fn world_id(&mut self, name: &str) -> u32 {
        let i = match self.world_names.iter().position(|w| w == name) {
            Some(i) => i,
            None => {
                self.world_names.push(name.to_owned());
                self.world_names.len() - 1
            }
        };
        i as u32 + 1
    }
    /// Some sound has effects (`SoundPlay::effects`), or a reverb's tail is still ringing.
    fn effects_heard(&self) -> bool {
        self.channels.values().any(|c| !c.fx.bypassed())
            || self.reverbs.buses.iter().any(|b| !b.idle())
    }
}
/// A sound the player could have heard (`SoundEventListener.onPlaySound`):
/// what the subtitle overlay needs.
#[derive(Clone, Debug, PartialEq)]
pub struct Heard {
    /// The `subtitle` translation key of the sound event.
    pub subtitle: String,
    /// The instance's position (relative sounds report their raw coordinates).
    pub at: Option<[f64; 3]>,
    /// `SoundEngine.getRange`: infinite for relative and non-attenuated sounds.
    pub range: f32,
    pub looping: bool,
}
/// A mod's muffle (`Audio::muffle`): its full strength, and the ticks it fades over and has left.
#[derive(Clone, Copy, Debug, PartialEq)]
struct MuffleFade {
    amount: f32,
    lowpass: f32,
    ticks: u32,
    left: u32,
}
impl MuffleFade {
    /// `[amount, lowpass]` now: full when set, falling in a straight line to none.
    fn level(&self) -> [f32; 2] {
        let left = self.left as f32 / self.ticks as f32;
        [self.amount * left, self.lowpass * left]
    }
}
/// The mixer side of the engine (`Backend`).
pub struct MixerBackend {
    mixer: Arc<Mutex<Mixer>>,
    root: ResourceSource,
    cache: Arc<Mutex<SoundCache>>,
    /// The decoder thread for static sounds, started with the first one.
    decoder: Option<std::sync::mpsc::Sender<Decode>>,
    next: ChannelId,
    live: bool,
    /// Mods' audio streams, which every backend sharing the output shares.
    streams: Arc<streams::StreamPool>,
    /// Whose channels these are (`Channel::owner`).
    owner: u8,
    world: u32,
    /// Number of additional backends sharing the output. With none, there's no duplicate check.
    shares: Arc<std::sync::atomic::AtomicUsize>,
}
/// Dropping a shared backend removes its camera and sounds.
impl Drop for MixerBackend {
    fn drop(&mut self) {
        if self.owner == 0 {
            return;
        }
        self.shares
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        if let Ok(mut m) = self.mixer.lock() {
            m.listeners.remove(&self.owner);
            m.worlds.remove(&self.owner);
            let owner = self.owner;
            m.channels.retain(|_, c| c.owner != owner);
        }
    }
}
/// Each shared backend gets its own block of channel ids. Blocks are never reused, so a pending
/// decode for a dropped backend cannot attach to a new one. The primary backend counts from 0.
static SHARED_CHANNELS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Maximum time between copies of a sound started by different backends.
const SAME_SOUND: std::time::Duration = std::time::Duration::from_millis(150);
/// Decoded interleaved samples, their rate and channel count (`None`: the file did not load).
type Samples = Option<(Arc<Vec<f32>>, u32, u16)>;
/// Decoded static sounds (`SoundBufferLibrary`'s buffers), dropped least
/// recently used first past `CACHE_BYTES`: kept forever, every sound heard in
/// a session stayed (up to 1.4 GiB of samples, villages playing hundreds).
#[derive(Default)]
struct SoundCache {
    entries: HashMap<String, (Samples, u64)>,
    bytes: usize,
    clock: u64,
}
const CACHE_BYTES: usize = 64 << 20;
impl SoundCache {
    fn get(&mut self, key: &str) -> Option<Samples> {
        self.clock += 1;
        let entry = self.entries.get_mut(key)?;
        entry.1 = self.clock;
        Some(entry.0.clone())
    }
    fn insert(&mut self, key: String, samples: Samples) {
        self.clock += 1;
        self.bytes += Self::size(&samples);
        if let Some((old, _)) = self.entries.insert(key, (samples, self.clock)) {
            self.bytes -= Self::size(&old);
        }
        while self.bytes > CACHE_BYTES && self.entries.len() > 1 {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some((gone, _)) = self.entries.remove(&oldest) {
                self.bytes -= Self::size(&gone);
            }
        }
    }
    fn size(samples: &Samples) -> usize {
        samples
            .as_ref()
            .map_or(0, |(s, _, _)| s.len() * std::mem::size_of::<f32>())
    }
}
/// A static sound to decode for a channel waiting on it.
struct Decode {
    key: String,
    file: Resource,
    channel: ChannelId,
    looping: bool,
}
/// Give a channel waiting on its sound the decoded samples and start it, as
/// the buffer future attaching and playing the source. A source stopped or
/// released meanwhile stays so.
fn attach(mixer: &Mutex<Mixer>, channel: ChannelId, samples: Samples, looping: bool) {
    if let Some(samples) = samples
        && let Ok(mut m) = mixer.lock()
        && let Some(c) = m.channels.get_mut(&channel)
        && c.state == State::Initial
    {
        c.samples = Some(samples);
        c.position = 0.0;
        c.state = State::Playing;
        c.looping |= looping;
    }
}
/// The open output stream. A build without the `audio` feature (the server build, which must not
/// need libasound) never has one.
#[cfg(feature = "audio")]
type Stream = cpal::Stream;
#[cfg(not(feature = "audio"))]
type Stream = std::convert::Infallible;
pub use sunrise_level::audio_capture::CapturedSound;

pub struct Audio {
    _stream: Option<Stream>,
    device_name: String,
    pub engine: SoundEngine,
    backend: MixerBackend,
    mics: Arc<mic::MicPool>,
    /// Record `Heard` sounds (the subtitle overlay is listening).
    pub record_heard: bool,
    pub listener: [f64; 3],
    listener_orientation: ListenerOrientation,
    pub master: f32,
    /// Seeds of positional plays (the client level random's `nextLong`).
    random: LegacyRandom,
    /// A `SoundSource` index the next sound plays on (`/playsound`).
    source_override: Option<usize>,
    /// Positional sounds played since the last drain, kept while the world
    /// is open to the network so guests hear them too.
    pub capture: Option<Vec<CapturedSound>>,
    /// How many times each positional sound played (the control socket's `status`),
    /// with or without an audio device.
    pub heard: std::collections::BTreeMap<String, u32>,
    /// How many times each selection was stopped ("record *", "* minecraft:…", `*` for
    /// any), with or without an audio device, as `heard`.
    pub stopped: std::collections::BTreeMap<String, u32>,
    /// The volume sliders' `SoundPreviewHandler`.
    preview: crate::sound_preview::SoundPreview,
    /// A mod's muffle fading out (`muffle`), advanced by `tick`.
    muffle: Option<MuffleFade>,
    /// The `stream_gain` the mixer was last given: it is locked again only when this changes.
    stream_gain_sent: [f32; 11],
    /// `SoundEngine.lastSeenDevices`: the system default output last seen.
    #[cfg(feature = "audio")]
    default_seen: Option<String>,
    /// The ALSA cards (Linux) or Core Audio devices (macOS) last seen: listing
    /// the output devices probes every one, so it waits for these to change
    /// (`card_fingerprint`).
    #[cfg(feature = "audio")]
    cards_seen: Option<String>,
    /// The other half of `lastSeenDevices`: every output device last seen.
    #[cfg(feature = "audio")]
    devices_seen: Vec<String>,
    /// Ticks since the output devices were last looked at (`DeviceTracker.tick`).
    #[cfg(feature = "audio")]
    device_ticks: u32,
    /// Opening the output again after it failed to (`Reopen`).
    #[cfg(feature = "audio")]
    reopen: Reopen,
    /// The look at the output devices running on its own thread, until a tick
    /// takes its answer: listing them can take 100 ms (#11490).
    #[cfg(feature = "audio")]
    device_probe: Option<PendingProbe>,
}
/// What one look at the output devices found: the system default, the
/// fingerprint, and the device list when those say it may have changed.
#[cfg(feature = "audio")]
struct DeviceProbe {
    default: Option<String>,
    cards: Option<String>,
    devices: Option<Vec<String>>,
}
#[cfg(feature = "audio")]
type PendingProbe = Arc<Mutex<Option<DeviceProbe>>>;
/// Looks at the output devices as `should_change_device` did on the render
/// thread: the device list only when the default or the fingerprint moved.
#[cfg(feature = "audio")]
fn probe_devices(default_seen: Option<String>, cards_seen: Option<String>) -> DeviceProbe {
    let default = default_output_name();
    let cards = card_fingerprint();
    // Listing the devices probes every one (on Linux every built-in PCM,
    // printing ALSA's errors for the plugins this system lacks; on macOS
    // every Core Audio device, about 90 ms with twenty of them; on Windows
    // every endpoint, with no fingerprint to skip it); only the fingerprint's
    // devices can change the list, and it is cheap.
    let devices = (default != default_seen
        || cards_changed(cards_seen.as_deref(), cards.as_deref()))
    .then(output_devices);
    DeviceProbe {
        default,
        cards,
        devices,
    }
}
fn decode(resource: &Resource) -> Option<(Vec<f32>, u32, u16)> {
    use symphonia::core::{
        audio::SampleBuffer, codecs::DecoderOptions, formats::FormatOptions, io::MediaSourceStream,
        meta::MetadataOptions, probe::Hint,
    };
    let file = std::io::Cursor::new(resource.read().ok()?);
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("ogg");
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .ok()?;
    let mut format = probed.format;
    let track = format.default_track()?;
    let id = track.id;
    let rate = track.codec_params.sample_rate?;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .ok()?;
    let mut samples = Vec::new();
    let mut channels = 0u16;
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != id {
            continue;
        }
        let Ok(buffer) = decoder.decode(&packet) else {
            continue;
        };
        let spec = *buffer.spec();
        let frame_channels = spec.channels.count().max(1);
        let mut decoded = SampleBuffer::<f32>::new(buffer.capacity() as u64, spec);
        decoded.copy_interleaved_ref(buffer);
        if channels == 0 {
            channels = frame_channels.min(2) as u16;
        }
        if frame_channels <= 2 {
            samples.extend_from_slice(decoded.samples());
        } else {
            // OpenAL plays multi-channel buffers direct; a stereo output hears the odd/even split.
            for frame in decoded.samples().chunks(frame_channels) {
                let (mut left, mut right) = (0.0, 0.0);
                let (mut lefts, mut rights) = (0.0f32, 0.0f32);
                for (i, sample) in frame.iter().enumerate() {
                    if i % 2 == 0 {
                        left += sample;
                        lefts += 1.0;
                    } else {
                        right += sample;
                        rights += 1.0;
                    }
                }
                samples.push(left / lefts.max(1.0));
                samples.push(right / rights.max(1.0));
            }
        }
    }
    (channels != 0).then_some((samples, rate, channels))
}
/// A sound file without its variant number (`mob/creeper/say2.ogg` is `mob/creeper/say`):
/// each client can pick its own variant of the same event.
// ponytail: variants named by a trailing number, as vanilla's are; the event name if a pack breaks it.
fn variantless(key: &str) -> &str {
    key.trim_end_matches(".ogg")
        .trim_end_matches(|c: char| c.is_ascii_digit())
}
impl MixerBackend {
    /// Another backend sharing this output started the same sound here just now: this copy
    /// stays stopped, and the first is heard from the nearest camera.
    ///
    /// A copy is recognised only by its file (without the variant), world, place and time: the
    /// clients pick variants on their own and play many sounds locally (steps, digging), so
    /// nothing marks two copies of one event. Two different events alike in all four (two
    /// footsteps on grass, side by side, within 150 ms) are heard once. At that distance and
    /// timing they sound like one; a doubled sound is the louder fault.
    fn duplicate(&self, channel: ChannelId, (namespace, path): (&str, &str)) -> bool {
        // Alone: nobody else's copy, and no lock taken for it.
        if self.shares.load(std::sync::atomic::Ordering::Relaxed) == 0 {
            return false;
        }
        let Ok(mut m) = self.mixer.lock() else {
            return false;
        };
        // Relative sounds (clicks, music) belong only to their listener.
        let Some(c) = m.channels.get(&channel).filter(|c| !c.relative) else {
            return false;
        };
        if m.listeners.is_empty() {
            return false;
        }
        let key = format!("{namespace}:{path}");
        let key = variantless(&key);
        let at = c.at;
        let world = c.world;
        let twin = m.channels.values().any(|o| {
            o.owner != self.owner
                && o.world == world
                && !o.relative
                && o.started
                    .as_ref()
                    .is_some_and(|(k, t)| k == key && t.elapsed() < SAME_SOUND)
                && (0..3).map(|i| (o.at[i] - at[i]).powi(2)).sum::<f32>() < 4.0
        });
        if let Some(c) = m.channels.get_mut(&channel) {
            if twin {
                c.state = State::Stopped;
            } else {
                c.started = Some((key.to_owned(), std::time::Instant::now()));
            }
        }
        twin
    }
    fn with(&self, channel: ChannelId, f: impl FnOnce(&mut Channel)) {
        if let Ok(mut m) = self.mixer.lock()
            && let Some(c) = m.channels.get_mut(&channel)
        {
            f(c);
        }
    }
}
impl Backend for MixerBackend {
    fn create(&mut self, _streaming: bool) -> Option<ChannelId> {
        self.next += 1;
        let id = self.next;
        self.mixer.lock().ok()?.channels.insert(
            id,
            Channel {
                samples: None,
                position: 0.0,
                state: State::Initial,
                gain: 1.0,
                pitch: 1.0,
                linear: None,
                looping: false,
                at: [0.0; 3],
                relative: false,
                owner: self.owner,
                world: self.world,
                started: None,
                unmuffled: false,
                fx: fx::VoiceFx::default(),
            },
        );
        Some(id)
    }
    fn destroy(&mut self, channel: ChannelId) {
        if let Ok(mut m) = self.mixer.lock() {
            m.channels.remove(&channel);
        }
    }
    fn set_pitch(&mut self, channel: ChannelId, pitch: f32) {
        self.with(channel, |c| c.pitch = pitch);
    }
    fn set_volume(&mut self, channel: ChannelId, volume: f32) {
        self.with(channel, |c| c.gain = volume);
    }
    fn set_attenuation(&mut self, channel: ChannelId, linear: Option<f32>) {
        self.with(channel, |c| c.linear = linear);
    }
    fn set_looping(&mut self, channel: ChannelId, looping: bool) {
        self.with(channel, |c| c.looping = looping);
    }
    fn set_position(&mut self, channel: ChannelId, p: [f64; 3]) {
        self.with(channel, |c| c.at = [p[0] as f32, p[1] as f32, p[2] as f32]);
    }
    fn set_relative(&mut self, channel: ChannelId, relative: bool) {
        self.with(channel, |c| c.relative = relative);
    }
    fn set_unmuffled(&mut self, channel: ChannelId, unmuffled: bool) {
        self.with(channel, |c| c.unmuffled = unmuffled);
    }
    fn set_effects(&mut self, channel: ChannelId, effects: mod_api::SoundEffects) {
        self.with(channel, |c| c.fx.target = effects);
    }
    fn update_channel(
        &mut self,
        channel: ChannelId,
        volume: f32,
        pitch: f32,
        position: [f64; 3],
        effects: mod_api::SoundEffects,
    ) {
        if let Ok(mut m) = self.mixer.lock()
            && let Some(c) = m.channels.get_mut(&channel)
        {
            c.gain = volume;
            c.pitch = pitch;
            c.at = [position[0] as f32, position[1] as f32, position[2] as f32];
            c.fx.target = effects;
        }
    }
    fn filter_stopped(&mut self, live: &[(u64, ChannelId)], stopped: &mut Vec<u64>) {
        if let Ok(m) = self.mixer.lock() {
            for &(h, c) in live {
                if m.channels
                    .get(&c)
                    .is_none_or(|ch| ch.state == State::Stopped)
                {
                    stopped.push(h);
                }
            }
        } else {
            stopped.extend(live.iter().map(|&(h, _)| h));
        }
    }
    /// `getCompleteBuffer` (cached, else decoded on the decoder thread) or
    /// `getStream` (decoded on a thread of its own), then attach and
    /// `alSourcePlay`. A sound that cannot load leaves its source initial,
    /// as a failed buffer future does in vanilla.
    fn start(&mut self, channel: ChannelId, sound: &Sound, looping: bool) {
        if !self.live {
            return;
        }
        let (namespace, path) = sound.path();
        let file = Resource::file(self.root.clone(), format!("assets/{namespace}/{path}"));
        // A looping sound follows its entity in one client's tracking only: each keeps its own.
        if !looping && self.duplicate(channel, (namespace, &path)) {
            return;
        }
        if sound.stream {
            // A streamed sound loops by restarting its stream.
            let mixer = self.mixer.clone();
            std::thread::spawn(move || {
                let samples = decode(&file).map(|(s, r, c)| (Arc::new(s), r, c));
                attach(&mixer, channel, samples, looping);
            });
            return;
        }
        let key = format!("{namespace}:{path}");
        let cached = self.cache.lock().ok().and_then(|mut c| c.get(&key));
        if let Some(samples) = cached {
            attach(&self.mixer, channel, samples, looping);
            return;
        }
        let decoder = self.decoder.get_or_insert_with(|| {
            let (send, receive) = std::sync::mpsc::channel::<Decode>();
            let (mixer, cache) = (self.mixer.clone(), self.cache.clone());
            std::thread::Builder::new()
                .name("sound decoder".into())
                .spawn(move || {
                    for job in receive {
                        // Queued twice before the first decode finished.
                        let cached = cache.lock().ok().and_then(|mut c| c.get(&job.key));
                        let samples = cached.unwrap_or_else(|| {
                            let samples = decode(&job.file).map(|(s, r, c)| (Arc::new(s), r, c));
                            if let Ok(mut c) = cache.lock() {
                                c.insert(job.key, samples.clone());
                            }
                            samples
                        });
                        attach(&mixer, job.channel, samples, job.looping);
                    }
                })
                .expect("sound decoder thread");
            send
        });
        let _ = decoder.send(Decode {
            key,
            file,
            channel,
            looping,
        });
    }
    /// `alSourceStop`: any state becomes stopped, an initial source too (a
    /// sound still decoding then never starts).
    fn stop(&mut self, channel: ChannelId) {
        self.with(channel, |c| c.state = State::Stopped);
    }
    fn pause(&mut self, channel: ChannelId) {
        self.with(channel, |c| {
            if c.state == State::Playing {
                c.state = State::Paused;
            }
        });
    }
    fn unpause(&mut self, channel: ChannelId) {
        self.with(channel, |c| {
            if c.state == State::Paused {
                c.state = State::Playing;
            }
        });
    }
    fn stopped(&mut self, channel: ChannelId) -> bool {
        self.mixer
            .lock()
            .map(|m| {
                m.channels
                    .get(&channel)
                    .is_none_or(|c| c.state == State::Stopped)
            })
            .unwrap_or(true)
    }
}
/// OpenAL Soft's stereo gains `[left, right]` for a mono source in a
/// direction given in listener space (right, up, forward; any length).
/// Vanilla's context (`Library.createAttributes`: HRTF off) on a stereo
/// output renders pairwise: front azimuths are stretched by 1.5
/// (`ScaleAzimuthFront`, so 60 degrees off-centre pans fully) and the
/// direction is decoded by the first-order stereo decoder, 0.5 W, 0.5 Y and
/// 0.0957 X in this scaling. A centred source gets 0.596 on each side, one
/// behind 0.404, one above or below 0.5, one at the source's own position
/// is panned to the front. Matches OpenAL Soft 1.25.2 (LWJGL 3.4.3, the
/// pinned client's) rendered through `ALC_SOFT_loopback` within 1e-5.
#[cfg(test)]
fn stereo_gains(right: f32, up: f32, forward: f32) -> [f32; 2] {
    let length = (right * right + up * up + forward * forward).sqrt();
    stereo_gains_with_length([right, up, forward].map(f64::from), f64::from(length)).unwrap()
}
/// `stereo_gains` for a direction of known `length`, or `None` for unreadable geometry.
#[inline]
fn stereo_gains_with_length(direction: [f64; 3], length: f64) -> Option<[f32; 2]> {
    let [right, up, forward] = direction;
    let narrow = [right as f32, up as f32, forward as f32, length as f32];
    let (u, mut azimuth) = if narrow.into_iter().all(f32::is_finite) {
        let [right, up, forward, length] = narrow;
        if length <= f32::EPSILON {
            (0.0, 0.0)
        } else {
            (up / length, right.atan2(forward))
        }
    } else if length.is_finite() && direction.into_iter().all(f64::is_finite) {
        // Finite channel coordinates can have projections or a length outside f32.
        // Compute the direction before narrowing; ordinary pans retain f32 rounding.
        ((up / length) as f32, right.atan2(forward) as f32)
    } else {
        return None;
    };
    let u = u.clamp(-1.0, 1.0);
    let cos_elev = (1.0 - u * u).max(0.0).sqrt();
    if azimuth.abs() < std::f32::consts::FRAC_PI_2 {
        azimuth = (azimuth.abs() * 1.5)
            .min(std::f32::consts::FRAC_PI_2)
            .copysign(azimuth);
    }
    let (sin_az, cos_az) = azimuth.sin_cos();
    let side = sin_az * cos_elev;
    let front = cos_az * cos_elev * 0.095_660_8;
    Some([0.5 - 0.5 * side + front, 0.5 + 0.5 * side + front])
}
/// The gain OpenAL applies for distance (`AL_LINEAR_DISTANCE` with reference
/// distance 0 and rolloff 1: the distance clamps to the max distance).
fn distance_gain(linear: Option<f32>, distance: f64) -> f32 {
    match linear {
        None => 1.0,
        Some(max) if max > 0.0 => {
            let distance = distance.min(f64::from(max));
            let narrow = distance as f32;
            // An unbounded reach must not turn a finite distance into infinity / infinity.
            let fraction = if narrow.is_finite() {
                narrow / max
            } else {
                (distance / f64::from(max)) as f32
            };
            (1.0 - fraction).clamp(0.0, 1.0)
        }
        Some(_) => 0.0,
    }
}
/// One voice's buffer through its effects ([`fx::VoiceFx`]): read as the plain path reads it
/// (linear interpolation, looping or stopping at the end), its direct sound filtered and panned
/// by `[left, right, gain]` into `out`, and its sends, at the sound's distance gain, into the
/// shared reverbs' input.
fn mix_with_effects(
    c: &mut Channel,
    (samples, source_channels, frames, step): (&[f32], usize, usize, f64),
    [left, right, gain]: [f32; 3],
    out: &mut [f32],
    channels: usize,
    rate: u32,
    reverbs: &mut Reverbs,
) {
    let buffer_frames = out.len() / channels;
    let stereo = source_channels > 1;
    let mut glide = c.fx.begin(buffer_frames, rate);
    let mut input: &mut [f32] = &mut [];
    if glide.sends {
        for (sent, to) in reverbs.sent.iter_mut().zip(glide.sent) {
            *sent |= to;
        }
        input = reverbs.input(buffer_frames);
    }
    for (f, frame) in out.chunks_mut(channels).enumerate() {
        let mut i = c.position as usize;
        if i + 1 >= frames {
            if c.looping && frames > 1 {
                c.position = 0.0;
                i = 0;
            } else {
                c.state = State::Stopped;
                break;
            }
        }
        let t = (c.position - i as f64) as f32;
        let mut x = if stereo {
            [
                samples[i * 2] + (samples[(i + 1) * 2] - samples[i * 2]) * t,
                samples[i * 2 + 1] + (samples[(i + 1) * 2 + 1] - samples[i * 2 + 1]) * t,
            ]
        } else {
            [samples[i] + (samples[i + 1] - samples[i]) * t, 0.0]
        };
        let send = c.fx.frame(&mut glide, &mut x, stereo);
        let [l, r] = if stereo { x } else { [x[0], x[0]] };
        if channels >= 2 {
            frame[0] += l * left;
            frame[1] += r * right;
        } else {
            frame[0] += (l + r) * 0.5 * gain;
        }
        if glide.sends {
            for (bus, level) in send.into_iter().enumerate() {
                input[bus * buffer_frames + f] += level * gain;
            }
        }
        c.position += step;
    }
}
/// [`mix`] over `data` in pieces of at most [`MAX_MIX_FRAMES`] frames: the reverbs' send buffer
/// was sized for that when the mixer was made, so mixing never allocates, however large a buffer
/// the device asks for. One piece, the whole buffer, for every usual buffer size, and whenever no
/// sound has effects and no reverb is ringing: then nothing uses the send buffer, and the output
/// is the one `mix` it was before effects existed.
fn mix_pieces(mixer: &mut Mixer, data: &mut [f32], channels: usize, rate: u32) {
    let most = MAX_MIX_FRAMES * channels.max(1);
    if data.len() <= most || !mixer.effects_heard() {
        mix(mixer, data, channels, rate);
    } else {
        for piece in data.chunks_mut(most) {
            mix(mixer, piece, channels, rate);
        }
    }
    streams::request_pull(&mixer.streams, data.len() / channels, rate);
}
// Only the output stream mixes; a build without one (no `audio` feature) never does.
#[cfg_attr(not(feature = "audio"), allow(dead_code))]
fn mix(mixer: &mut Mixer, data: &mut [f32], channels: usize, rate: u32) {
    data.fill(0.0);
    // Every active listener's camera, in the mixer's scratch buffer.
    mixer.cameras.clear();
    mixer.cameras.extend(
        std::iter::once((0, (mixer.listener, mixer.listener_orientation)))
            .chain(
                mixer
                    .listeners
                    .iter()
                    .map(|(&owner, &camera)| (owner, camera)),
            )
            .map(|(owner, (at, orientation))| {
                (
                    at,
                    orientation.basis(),
                    mixer.worlds.get(&owner).copied().unwrap_or_default(),
                )
            }),
    );
    let shared = mixer.cameras.len() > 1;
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    // Gains and pans per source for this buffer.
    let mut voices = std::mem::take(&mut mixer.voices);
    voices.clear();
    for (id, c) in &mut mixer.channels {
        let Some((_, source_rate, source_channels)) = &c.samples else {
            continue;
        };
        // OpenAL plays a multi-channel buffer direct: no panning and no distance attenuation
        // (`CalcVoiceParams`' non-spatialized path, vanilla's HRTF-off stereo context). Vanilla's
        // music and records are stereo. A mono buffer is the positional one.
        let direct = *source_channels > 1;
        if c.state != State::Playing {
            continue;
        }

        let at = [f64::from(c.at[0]), f64::from(c.at[1]), f64::from(c.at[2])];
        let away = |listener: [f64; 3]| {
            [
                at[0] - listener[0],
                at[1] - listener[1],
                at[2] - listener[2],
            ]
        };
        let length_sq = |d: [f64; 3]| d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
        let camera = if !shared {
            let cam = &mixer.cameras[0];
            if c.relative || cam.2 == c.world {
                Some((cam.0, cam.1))
            } else {
                None
            }
        } else {
            mixer
                .cameras
                .iter()
                .filter(|camera| c.relative || camera.2 == c.world)
                .min_by(|a, b| length_sq(away(a.0)).total_cmp(&length_sq(away(b.0))))
                .map(|camera| (camera.0, camera.1))
        };
        let (listener, (forward, up, right)) =
            camera.unwrap_or_else(|| ([0.0; 3], ListenerOrientation::default().basis()));
        let d = if c.relative { at } else { away(listener) };
        let d_sqr = length_sq(d);

        let (gain, [left, right]) = if camera.is_none() {
            (0.0, [0.0, 0.0])
        } else if direct {
            let gain = c.gain.clamp(0.0, 1.0);
            (gain, [gain, gain])
        } else {
            let mut zero = false;
            if let Some(max) = c.linear {
                let max_d = max as f64;
                if max <= 0.0 || (!c.relative && d_sqr >= max_d * max_d) {
                    zero = true;
                }
            }
            if zero {
                (0.0, [0.0, 0.0])
            } else {
                let distance = d_sqr.sqrt();
                let direction = if c.relative {
                    [d[0], d[1], -d[2]]
                } else {
                    [dot(d, right), dot(d, up), dot(d, forward)]
                };
                if let Some([mut left, mut right]) = stereo_gains_with_length(direction, distance) {
                    let gain = c.gain.clamp(0.0, 1.0) * distance_gain(c.linear, distance);
                    if shared {
                        let middle = (left + right) * 0.5;
                        left = middle + (left - middle) * 0.5;
                        right = middle + (right - middle) * 0.5;
                    }
                    (gain, [left * gain, right * gain])
                } else {
                    (0.0, [0.0, 0.0])
                }
            }
        };
        // Keep an unreadable voice out of the shared output and reverb buses,
        // but keep advancing its playhead.
        let gains = [left, right, gain];
        let gains = if gains.into_iter().all(f32::is_finite) {
            gains
        } else {
            [0.0; 3]
        };
        let step = f64::from(*source_rate) / f64::from(rate) * f64::from(c.pitch.max(0.0));
        voices.push((*id, gains, step));
    }
    // A mod's muffle: the sounds it doesn't exempt mix into a bus of their own first.
    let muffling = mixer.muffle.active();
    let mut bus = std::mem::take(&mut mixer.muffle.bus);
    if muffling {
        bus.clear();
        bus.resize(data.len(), 0.0);
    }
    for (id, [left, right, gain], step) in voices.iter().copied() {
        let Some(c) = mixer.channels.get_mut(&id) else {
            continue;
        };
        let Some((samples, _, source_channels)) = c.samples.clone() else {
            continue;
        };
        let source_channels = source_channels as usize;
        let frames = samples.len() / source_channels;
        let inaudible = if channels >= 2 {
            left == 0.0 && right == 0.0
        } else {
            gain == 0.0
        };
        if inaudible {
            let buffer_frames = data.len() / channels;
            // As the loop below: a sound of one frame or none stops, looping or not.
            if !c.looping || frames <= 1 {
                // `frames` may be 0: no `frames - 1` in usize. A step of 0 never
                // reaches the end, as in the loop below.
                let needed = frames as f64 - 1.0 - c.position;
                if needed <= 0.0 {
                    c.state = State::Stopped;
                } else if step > 0.0 {
                    let steps_to_stop = needed / step;
                    if (buffer_frames as f64) >= steps_to_stop {
                        c.state = State::Stopped;
                    } else {
                        c.position += step * (buffer_frames as f64);
                    }
                }
            } else {
                for _ in 0..buffer_frames {
                    let i = c.position as usize;
                    if i + 1 >= frames {
                        c.position = 0.0;
                    }
                    c.position += step;
                }
            }
            continue;
        }

        let out: &mut [f32] = if muffling && !c.unmuffled {
            &mut bus
        } else {
            &mut *data
        };
        if !c.fx.bypassed() {
            mix_with_effects(
                c,
                (samples.as_slice(), source_channels, frames, step),
                [left, right, gain],
                out,
                channels,
                rate,
                &mut mixer.reverbs,
            );
            continue;
        }
        c.fx.played(rate);
        for frame in out.chunks_mut(channels) {
            let mut i = c.position as usize;
            if i + 1 >= frames {
                if c.looping && frames > 1 {
                    c.position = 0.0;
                    i = 0;
                } else {
                    c.state = State::Stopped;
                    break;
                }
            }
            let t = (c.position - i as f64) as f32;
            if source_channels > 1 {
                let l = samples[i * 2] + (samples[(i + 1) * 2] - samples[i * 2]) * t;
                let r = samples[i * 2 + 1] + (samples[(i + 1) * 2 + 1] - samples[i * 2 + 1]) * t;
                if channels >= 2 {
                    frame[0] += l * left;
                    frame[1] += r * right;
                } else {
                    frame[0] += (l + r) * 0.5 * gain;
                }
            } else {
                let s = samples[i] + (samples[i + 1] - samples[i]) * t;
                if channels >= 2 {
                    frame[0] += s * left;
                    frame[1] += s * right;
                } else {
                    // A mono output has no panning: the source's own gain.
                    frame[0] += s * gain;
                }
            }
            c.position += step;
        }
    }
    // Mods' audio streams join the sounds the muffle takes, as any sound does.
    let out: &mut [f32] = if muffling { &mut bus } else { &mut *data };
    streams::mix(
        &mut mixer.streams,
        &mixer.cameras,
        &mixer.stream_gain,
        out,
        channels,
        rate,
    );
    mixer.voices = voices;
    // The reverbs' tails, muffled with the sounds while a mod's muffle is on.
    mixer
        .reverbs
        .run(if muffling { &mut bus } else { &mut *data }, channels, rate);
    if muffling {
        mixer.muffle.apply(&bus, data, channels, rate);
    } else {
        // Off: the next muffle starts from silence, not from the last one's filter.
        mixer.muffle.current = [0.0; 2];
        mixer.muffle.stages = [[0.0; 2]; 2];
    }
    mixer.muffle.bus = bus;
    // Limit the whole buffer together, preserving waveform and stereo balance
    // instead of flattening each overloaded sample; release over 200 ms
    // (OpenAL Soft's output limiter, `ALC_OUTPUT_LIMITER_SOFT`, releases
    // over 0.2 s).
    let peak = data.iter().fold(1.0f32, |peak, s| peak.max(s.abs()));
    mixer.limiter_peak = mixer.limiter_peak.max(peak);
    let release = (-1.0 / (0.2 * rate as f32)).exp();
    for frame in data.chunks_mut(channels) {
        let gain = mixer.limiter_peak.recip();
        for s in frame {
            *s *= gain;
        }
        mixer.limiter_peak = (mixer.limiter_peak * release).max(peak);
    }
}
/// Why the game should not open an output, if it should not:
/// `SUNRISE_NO_AUDIO`, or a virtual screen (`vscreen`), where agents and
/// harnesses run the game and must stay off the user's speakers.
/// `SUNRISE_AUDIO=1` opens the output anyway (testing into a null sink).
#[cfg(any(feature = "audio", test))]
fn output_disabled(var: impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<String> {
    if var("SUNRISE_NO_AUDIO").is_some() {
        return Some("audio off: SUNRISE_NO_AUDIO is set".into());
    }
    if var("SUNRISE_AUDIO").is_some_and(|v| v == "1") {
        return None;
    }
    let display = var("WAYLAND_DISPLAY")?;
    Path::new(&display)
        .components()
        .any(|c| c.as_os_str() == "vscreen")
        .then(|| "audio off on a virtual screen (vscreen); SUNRISE_AUDIO=1 opens it".into())
}
/// Whether microphone capture is suppressed by an explicit opt-out (`SUNRISE_NO_AUDIO`).
/// Unlike [`output_disabled`], capture does not inherit the `WAYLAND_DISPLAY` `vscreen`
/// speaker-suppression heuristic.
#[cfg(any(feature = "audio", test))]
fn capture_disabled(var: impl Fn(&str) -> Option<std::ffi::OsString>) -> bool {
    var("SUNRISE_NO_AUDIO").is_some()
}
/// An output stream on `device` in its default configuration, whatever its
/// sample format: the mixer works in f32 and converts.
#[cfg(feature = "audio")]
fn open_device(
    device: &cpal::Device,
    mixer: Arc<Mutex<Mixer>>,
) -> Result<(cpal::Stream, cpal::SupportedStreamConfig)> {
    use cpal::SampleFormat as F;
    let config = device.default_output_config()?;
    let build = |stream_config: &cpal::StreamConfig| match config.sample_format() {
        F::F32 => build_stream::<f32>(device, &config, stream_config, mixer.clone()),
        F::F64 => build_stream::<f64>(device, &config, stream_config, mixer.clone()),
        F::I16 => build_stream::<i16>(device, &config, stream_config, mixer.clone()),
        F::I32 => build_stream::<i32>(device, &config, stream_config, mixer.clone()),
        F::I8 => build_stream::<i8>(device, &config, stream_config, mixer.clone()),
        F::U16 => build_stream::<u16>(device, &config, stream_config, mixer.clone()),
        F::U32 => build_stream::<u32>(device, &config, stream_config, mixer.clone()),
        F::U8 => build_stream::<u8>(device, &config, stream_config, mixer.clone()),
        other => Err(format!("unsupported sample format {other}").into()),
    };
    let short = audio_stream_config(&config);
    let stream = match build(&short) {
        Ok(stream) => stream,
        // A device that refuses the short fixed buffer still plays with cpal's default one.
        Err(e) if short != config.config() => {
            log::info!("Sunrise: audio output refused a short buffer ({e}); using the default");
            build(&config.config())?
        }
        Err(e) => return Err(e),
    };
    stream.play()?;
    if let Ok(mut m) = mixer.lock() {
        m.device_name = device.name().unwrap_or_default();
        m.device_lost = false;
        m.stream_errors = StreamErrors::default();
    }
    Ok((stream, config))
}
/// Try one output device for `open`: skip it when it was already tried, log it
/// when it opens, and keep its error when it does not.
#[cfg(feature = "audio")]
fn try_open_device(
    device: &cpal::Device,
    mixer: Arc<Mutex<Mixer>>,
    tried: &mut Vec<String>,
    errors: &mut Vec<String>,
) -> Option<cpal::Stream> {
    let name = device.name().unwrap_or_else(|_| "unnamed device".into());
    if tried.contains(&name) {
        return None;
    }
    tried.push(name.clone());
    match open_device(device, mixer) {
        Ok((stream, config)) => {
            log::info!(
                "Sunrise: audio output {name} ({} Hz, {} channels, {})",
                config.sample_rate().0,
                config.channels(),
                config.sample_format()
            );
            Some(stream)
        }
        Err(e) => {
            errors.push(format!("{name}: {e}"));
            None
        }
    }
}
/// The name of the system's default output device.
#[cfg(feature = "audio")]
fn default_output_name() -> Option<String> {
    cpal::default_host()
        .default_output_device()
        .and_then(|d| d.name().ok())
}
/// A cheap fingerprint of what `output_devices` can list. On Linux, the ALSA
/// cards: the rest are `cpal`'s fixed names. On macOS, every Core Audio
/// device's name, inputs included: telling which of them play opens each one
/// (about 90 ms a second with twenty devices), naming them takes 2 or 3 ms.
/// `None` elsewhere, where the list is always looked at.
#[cfg(feature = "audio")]
fn card_fingerprint() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/asound/cards").ok()
    }
    #[cfg(target_os = "macos")]
    {
        let devices = cpal::default_host().devices().ok()?;
        Some(
            devices
                .filter_map(|d| d.name().ok())
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}
/// Whether the output device list should be looked at again: only the ALSA
/// cards can add or remove an entry, and `None` means there is no fingerprint,
/// so the list has to be checked as before.
#[cfg(any(feature = "audio", test))]
fn cards_changed(seen: Option<&str>, cards: Option<&str>) -> bool {
    match (seen, cards) {
        (Some(seen), Some(cards)) => seen != cards,
        _ => true,
    }
}

/// The buffer size to request for `open_device`. On Linux the ALSA backend's
/// `BufferSize::Default` means a ~100 ms buffer, so sound effects come out
/// about 100 ms late: request a short fixed buffer (1024 frames) by default.
/// An environment override `SUNRISE_AUDIO_BUFFER` can specify `default` or a
/// custom frame count (e.g. 2048) when the default short buffer underruns
/// (e.g. PipeWire min-quantum >= 1024).
#[cfg(feature = "audio")]
fn target_buffer_size(
    range: (u32, u32),
    var: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Option<cpal::BufferSize> {
    if let Some(val) = var("SUNRISE_AUDIO_BUFFER").and_then(|v| v.into_string().ok()) {
        if val == "default" {
            return Some(cpal::BufferSize::Default);
        }
        if let Ok(frames) = val.parse::<u32>() {
            return Some(cpal::BufferSize::Fixed(frames.clamp(range.0, range.1)));
        }
    }
    #[cfg(target_os = "linux")]
    {
        Some(cpal::BufferSize::Fixed(1024u32.clamp(range.0, range.1)))
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// The cpal `StreamConfig` to open an output stream with. On Linux the ALSA
/// backend's `BufferSize::Default` means a ~100 ms buffer, so sound effects
/// come out about 100 ms late: request a short fixed buffer there. Other
/// platforms keep cpal's default behaviour.
#[cfg(feature = "audio")]
fn audio_stream_config(config: &cpal::SupportedStreamConfig) -> cpal::StreamConfig {
    let mut sc = config.config();
    let range = match config.buffer_size() {
        cpal::SupportedBufferSize::Range { min, max } => (*min, *max),
        cpal::SupportedBufferSize::Unknown => (256, 4096),
    };
    if let Some(bs) = target_buffer_size(range, |name| std::env::var_os(name)) {
        sc.buffer_size = bs;
    }
    sc
}

#[cfg(feature = "audio")]
fn build_stream<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    stream_config: &cpal::StreamConfig,
    mixer: Arc<Mutex<Mixer>>,
) -> Result<cpal::Stream> {
    let rate = config.sample_rate().0;
    let channels = config.channels() as usize;
    let mut scratch = Vec::new();
    let on_error = mixer.clone();
    let stream = device.build_output_stream(
        stream_config,
        move |data: &mut [T], _| {
            scratch.resize(data.len(), 0.0f32);
            if let Ok(mut m) = mixer.lock() {
                mix_pieces(&mut m, &mut scratch, channels, rate);
            } else {
                scratch.fill(0.0);
            }
            for (out, s) in data.iter_mut().zip(&scratch) {
                *out = T::from_sample(*s);
            }
        },
        move |e| {
            log::warn!("Sunrise: audio stream error: {e}");
            if let Ok(mut m) = on_error.lock() {
                let dead = match e {
                    cpal::StreamError::DeviceNotAvailable => true,
                    cpal::StreamError::BackendSpecific { .. } => {
                        m.stream_errors.dead(std::time::Instant::now())
                    }
                };
                if dead && !m.device_lost {
                    if !matches!(e, cpal::StreamError::DeviceNotAvailable) {
                        log::warn!("Sunrise: the audio stream stopped; opening the output again");
                    }
                    m.device_lost = true;
                }
            }
        },
        None,
    )?;
    Ok(stream)
}
/// Available audio output device names.
#[cfg(feature = "audio")]
pub fn output_devices() -> Vec<String> {
    if output_disabled(|name| std::env::var_os(name)).is_some() {
        return Vec::new();
    }
    let host = cpal::default_host();
    let mut names = Vec::new();
    if let Ok(devices) = host.output_devices() {
        for device in devices {
            if let Ok(name) = device.name()
                && !names.contains(&name)
            {
                names.push(name);
            }
        }
    }
    names
}

#[cfg(not(feature = "audio"))]
pub fn output_devices() -> Vec<String> {
    Vec::new()
}

impl Audio {
    pub fn new(root: &ResourceSource) -> Self {
        Self::with_device(root, "")
    }
    pub fn with_device(root: &ResourceSource, device_name: &str) -> Self {
        Self::with_output(root, SoundRegistry::load(root), true, device_name)
    }
    /// Keep the client's sound registry but open no output device.
    pub fn silent(root: &ResourceSource) -> Self {
        Self::with_output(root, SoundRegistry::load(root), false, "")
    }
    /// Dedicated servers send sound events to clients without opening their sound files.
    pub fn for_server() -> Self {
        Self::with_output(
            &ResourceSource::default(),
            SoundRegistry::default(),
            false,
            "",
        )
    }
    /// The output device asked for (`soundDevice`), empty for the system's default; the
    /// default plays when that one can't be opened, as in vanilla.
    pub fn device(&self) -> &str {
        &self.device_name
    }
    /// The loop context of this client's current resource generation.
    pub fn loop_context(&self) -> Option<&sunrise_world::native_load_context::LoopContext> {
        self.engine.loop_context()
    }
    /// Replace the loop context when a resource generation starts or reloads, and tell
    /// the sound hooks the engine is loaded with it.
    /// Prepares the start of `id` for the game's `play_sound` (see `SoundEngine::begin_play`).
    pub fn begin_play(&mut self, id: InstanceId) -> Option<crate::sound_engine::PreparedPlay> {
        self.engine.begin_play(id)
    }

    /// The next start made without the game, for it to fire `play_sound` for.
    pub fn next_deferred(&mut self) -> Option<crate::sound_engine::PreparedPlay> {
        self.engine.next_deferred()
    }

    /// Apply the game's `play_sound` answer to a prepared start, and start it at once.
    pub fn finish_prepared(
        &mut self,
        id: InstanceId,
        answer: Option<mod_api::SoundPlay>,
    ) -> PlayResult {
        self.sync_volumes();
        self.engine.finish_prepared(id, answer, &mut self.backend)
    }

    /// The live refreshes the game fires `play_sound` for this tick.
    pub fn live_requests(&mut self) -> Vec<crate::sound_engine::PreparedPlay> {
        self.engine.live_requests()
    }

    /// The game's `play_sound` answer to a live refresh request.
    pub fn set_live_answer(&mut self, id: InstanceId, answer: Option<mod_api::SoundPlay>) {
        self.engine.set_live_answer(id, answer);
    }

    pub fn set_loop_context(&mut self, context: sunrise_world::native_load_context::LoopContext) {
        self.engine.set_loop_context(context);
        self.engine.notify_loaded();
    }
    fn with_output(
        root: &ResourceSource,
        registry: SoundRegistry,
        output: bool,
        target_device: &str,
    ) -> Self {
        if output && registry.is_empty() {
            // The game then plays nothing on a working output (#1035, #1076).
            log::warn!(
                "Sunrise: no sounds in the active resources; prepare the game again in the \
                 launcher (or run sunrise-prepare sounds)"
            );
        }
        let mixer = Arc::new(Mutex::new(Mixer {
            // Room for every stream's voice, so opening one never allocates under the lock.
            streams: Vec::with_capacity(mod_api::AUDIO_STREAMS_TOTAL as usize),
            ..Mixer::default()
        }));
        let opened = if output {
            Self::open_named(mixer.clone(), target_device)
        } else {
            Err("no output on a dedicated server".into())
        };
        // Without `audio` a stream can't exist (`Infallible`), so the `Ok` arm never runs.
        #[cfg_attr(not(feature = "audio"), allow(unreachable_code))]
        let (stream, live) = match opened {
            Ok(v) => (Some(v), true),
            Err(e) => {
                if output {
                    log::warn!("Sunrise: no sound: {e}");
                }
                (None, false)
            }
        };
        let mut engine = SoundEngine::new(registry);
        engine.loaded = live;
        #[cfg(feature = "audio")]
        let cards = if live { card_fingerprint() } else { None };
        // Where the cards gate the list (Linux, macOS), it starts empty: opening the
        // output does not need it, and listing it probes every device; the first
        // change fills it. Elsewhere it starts with every device listed, as before.
        #[cfg(feature = "audio")]
        let devices = if live && cards.is_none() {
            output_devices()
        } else {
            Vec::new()
        };
        #[cfg(feature = "audio")]
        let mics_enabled = output && !capture_disabled(|k| std::env::var_os(k));
        #[cfg(not(feature = "audio"))]
        let mics_enabled = false;
        Self {
            _stream: stream,
            device_name: target_device.to_owned(),
            engine,
            backend: MixerBackend {
                mixer,
                root: root.to_owned(),
                cache: Arc::default(),
                decoder: None,
                next: 0,
                live,
                streams: Arc::default(),
                owner: 0,
                world: 0,
                shares: Arc::default(),
            },
            mics: Arc::new(mic::MicPool::new(mics_enabled)),
            record_heard: false,
            listener: [0.0; 3],
            listener_orientation: ListenerOrientation::default(),
            master: 1.0,
            random: crate::random_source::create(),
            source_override: None,
            capture: None,
            heard: Default::default(),
            stopped: Default::default(),
            preview: Default::default(),
            muffle: None,
            stream_gain_sent: [f32::NAN; 11],
            #[cfg(feature = "audio")]
            default_seen: if live { default_output_name() } else { None },
            #[cfg(feature = "audio")]
            cards_seen: cards,
            #[cfg(feature = "audio")]
            devices_seen: devices,
            #[cfg(feature = "audio")]
            device_ticks: 0,
            #[cfg(feature = "audio")]
            reopen: Reopen::default(),
            #[cfg(feature = "audio")]
            device_probe: None,
        }
    }
    #[cfg(not(feature = "audio"))]
    fn open_named(_: Arc<Mutex<Mixer>>, _: &str) -> Result<Stream> {
        Err("this build has no sound output (built without the `audio` feature)".into())
    }
    #[cfg(feature = "audio")]
    fn open_named(mixer: Arc<Mutex<Mixer>>, target: &str) -> Result<Stream> {
        if let Some(reason) = output_disabled(|name| std::env::var_os(name)) {
            return Err(reason.into());
        }
        let host = cpal::default_host();
        if !target.is_empty()
            && let Ok(devices) = host.output_devices()
        {
            for device in devices {
                let name = device.name().unwrap_or_else(|_| "unnamed device".into());
                if name == target {
                    match open_device(&device, mixer.clone()) {
                        Ok((stream, config)) => {
                            log::info!(
                                "Sunrise: audio output {name} ({} Hz, {} channels, {})",
                                config.sample_rate().0,
                                config.channels(),
                                config.sample_format()
                            );
                            return Ok(stream);
                        }
                        Err(e) => {
                            log::warn!(
                                "Sunrise: could not open audio device '{target}': {e}; falling back to default"
                            );
                        }
                    }
                    break;
                }
            }
        }
        Self::open(mixer)
    }
    #[cfg(feature = "audio")]
    pub fn set_device(&mut self, name: &str) -> Result<()> {
        if !self.backend.live && self._stream.is_none() && !self.engine.loaded {
            self.device_name = name.to_owned();
            return Ok(());
        }
        self._stream = Some(Self::open_named(self.backend.mixer.clone(), name)?);
        self.device_name = name.to_owned();
        self.backend.live = true;
        self.backend.streams.output_lost(false);
        self.engine.loaded = true;
        self.engine.notify_loaded();
        Ok(())
    }
    #[cfg(not(feature = "audio"))]
    pub fn set_device(&mut self, name: &str) -> Result<()> {
        self.device_name = name.to_owned();
        Ok(())
    }
    #[cfg(feature = "audio")]
    fn open(mixer: Arc<Mutex<Mixer>>) -> Result<Stream> {
        if let Some(reason) = output_disabled(|name| std::env::var_os(name)) {
            return Err(reason.into());
        }
        let host = cpal::default_host();
        let mut errors = Vec::new();
        // The default device first: it is what plays, and listing the others
        // probes every PCM, which is slow and prints ALSA's errors for the
        // plugins this system lacks.
        let mut tried = Vec::new();
        if let Some(device) = host.default_output_device()
            && let Some(stream) = try_open_device(&device, mixer.clone(), &mut tried, &mut errors)
        {
            return Ok(stream);
        }
        // A default that cannot open (busy, or a stale ALSA default) should not
        // leave the game silent when another device works.
        for device in host.output_devices().into_iter().flatten() {
            if let Some(stream) = try_open_device(&device, mixer.clone(), &mut tried, &mut errors) {
                return Ok(stream);
            }
        }
        if errors.is_empty() {
            return Err(format!("no audio output device on the {} host", host.id().name()).into());
        }
        Err(format!("no audio output device opened ({})", errors.join("; ")).into())
    }
    fn bound(&mut self) -> Bound<'_, MixerBackend> {
        Bound {
            engine: &mut self.engine,
            backend: &mut self.backend,
        }
    }
    /// World identity for this player's sources and camera. Existing channels keep their world.
    pub fn set_world(&mut self, world: &str) {
        let mut changed = false;
        if let Ok(mut mixer) = self.backend.mixer.lock() {
            let id = mixer.world_id(world);
            changed = self.backend.world != id;
            self.backend.world = id;
            mixer.worlds.insert(self.backend.owner, id);
        }
        // The primary listener changed dimension: stop all mod streams (matching vanilla's
        // `updateLevelInEngines` -> `SoundEngine.stopAll`).
        // After the mixer's lock is released: the pool is locked before the mixer elsewhere.
        if changed && self.backend.owner == 0 {
            self.backend.streams.stop_all();
        }
    }

    /// `SoundEngine.updateSource`: the effective camera position and its forward and up vectors.
    pub fn set_listener(&mut self, position: [f64; 3], forward: [f32; 3], up: [f32; 3]) {
        let orientation = ListenerOrientation { forward, up };
        if self.listener == position && self.listener_orientation == orientation {
            return;
        }
        self.listener = position;
        self.listener_orientation = orientation;
        if let Ok(mut m) = self.backend.mixer.lock() {
            if self.backend.owner == 0 {
                m.listener = position;
                m.listener_orientation = orientation;
            } else {
                m.listeners
                    .insert(self.backend.owner, (position, orientation));
            }
        }
    }
    /// Share this output with another listener and its independent sound engine.
    /// Give it a unique nonzero `owner`. Dropping it removes its camera and sounds.
    pub fn share(&self, owner: u8) -> Self {
        assert_ne!(owner, 0, "shared audio owners must be nonzero");
        self.backend
            .shares
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut engine = SoundEngine::new(self.engine.registry.clone());
        engine.loaded = self.backend.live;
        Self {
            _stream: None,
            device_name: self.device_name.clone(),
            engine,
            backend: MixerBackend {
                mixer: self.backend.mixer.clone(),
                root: self.backend.root.clone(),
                cache: self.backend.cache.clone(),
                decoder: None,
                next: (SHARED_CHANNELS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1)
                    << 40,
                live: self.backend.live,
                streams: self.backend.streams.clone(),
                owner,
                world: self.backend.world,
                shares: self.backend.shares.clone(),
            },
            mics: self.mics.clone(),
            record_heard: false,
            listener: [0.0; 3],
            listener_orientation: ListenerOrientation::default(),
            master: self.master,
            random: crate::random_source::create(),
            source_override: None,
            capture: None,
            heard: Default::default(),
            stopped: Default::default(),
            preview: Default::default(),
            muffle: None,
            stream_gain_sent: [f32::NAN; 11],
            // The primary backend owns and maintains the output device.
            #[cfg(feature = "audio")]
            default_seen: self.default_seen.clone(),
            #[cfg(feature = "audio")]
            cards_seen: self.cards_seen.clone(),
            #[cfg(feature = "audio")]
            devices_seen: self.devices_seen.clone(),
            #[cfg(feature = "audio")]
            device_ticks: 0,
            #[cfg(feature = "audio")]
            reopen: Reopen::default(),
            #[cfg(feature = "audio")]
            device_probe: None,
        }
    }
    /// Sync the volume options, refreshing the categories that changed
    /// (the option sliders' `refreshCategoryVolume`).
    fn sync_volumes(&mut self) {
        let mut volumes = [self.master; 11];
        for (i, v) in volumes.iter_mut().enumerate().skip(1) {
            *v = crate::settings::source_volume(i);
        }
        for source in 0..11 {
            if self.engine.source_volumes[source] != volumes[source] {
                self.engine.source_volumes[source] = volumes[source];
                self.engine
                    .refresh_category_volume(source, &mut self.backend);
            }
        }
        self.engine.listening = self.record_heard;
    }
    /// `SoundEngine.shouldChangeDevice`: the open device was lost, the system
    /// default changed (with no device chosen in the options), or the chosen
    /// device came back; the output is then opened again. The devices are looked
    /// at once a second, on a thread of their own: a later tick reads what it
    /// found.
    #[cfg(feature = "audio")]
    fn should_change_device(&mut self) -> bool {
        // Only the primary backend manages the shared output device and its lost flag.
        if self.backend.owner != 0 {
            return false;
        }
        let (current, lost) = match self.backend.mixer.lock() {
            Ok(mut m) => (m.device_name.clone(), std::mem::take(&mut m.device_lost)),
            Err(_) => return false,
        };
        if lost {
            log::info!("Sunrise: audio device was lost");
            self.device_probe = None;
            self.default_seen = default_output_name();
            self.cards_seen = card_fingerprint();
            self.devices_seen = output_devices();
            return true;
        }
        if let Some(pending) = &self.device_probe {
            let Some(probe) = pending.lock().ok().and_then(|mut found| found.take()) else {
                return false;
            };
            self.device_probe = None;
            return self.devices_changed(probe, current);
        }
        self.device_ticks += 1;
        if self._stream.is_none() || self.device_ticks < 20 {
            return false;
        }
        self.device_ticks = 0;
        let pending = PendingProbe::default();
        let out = Arc::clone(&pending);
        let seen = (self.default_seen.clone(), self.cards_seen.clone());
        let started = std::thread::Builder::new()
            .name("audio-devices".into())
            .spawn(move || {
                let probe = probe_devices(seen.0, seen.1);
                if let Ok(mut found) = out.lock() {
                    *found = Some(probe);
                }
            });
        match started {
            Ok(_) => self.device_probe = Some(pending),
            Err(e) => log::warn!("Sunrise: could not look at the audio devices: {e}"),
        }
        false
    }
    /// The rest of `shouldChangeDevice` with what a look at the devices found,
    /// `current` being the device playing now.
    #[cfg(feature = "audio")]
    fn devices_changed(&mut self, probe: DeviceProbe, current: String) -> bool {
        let DeviceProbe {
            default,
            cards,
            devices,
        } = probe;
        let Some(devices) = devices else {
            return false;
        };
        self.cards_seen = cards;
        // Only a change in the device lists is looked at, as in vanilla: a
        // preferred device that is listed but cannot open is not retried.
        if default == self.default_seen && devices == self.devices_seen {
            return false;
        }
        self.default_seen = default.clone();
        let wanted = self.device_name.clone();
        let available = devices.contains(&wanted);
        self.devices_seen = devices;
        if wanted.is_empty() {
            let changed = default.as_deref().is_some_and(|d| d != current);
            if changed {
                log::info!("Sunrise: system default audio device has changed");
            }
            return changed;
        }
        let changed = current != wanted && available;
        if changed {
            log::info!("Sunrise: preferred audio device has become available");
        }
        changed
    }
    /// Opens the output on the chosen device again; `retrying` after an earlier try failed.
    #[cfg(feature = "audio")]
    fn reopen_output(&mut self, retrying: bool) {
        // The old stream lets go of the device first: a dead one may still hold it.
        self._stream = None;
        let device = self.device_name.clone();
        match self.set_device(&device) {
            Ok(()) => {
                if retrying {
                    log::info!("Sunrise: the audio output is back");
                }
                self.reopen = Reopen::default();
            }
            Err(e) => {
                // No output now: nothing starts on it, and mods' streams stop.
                self.backend.live = false;
                self.backend.streams.output_lost(true);
                if self.reopen.failed(retrying) {
                    log::warn!(
                        "Sunrise: could not reopen the audio output ({e}); trying again soon"
                    );
                } else {
                    log::warn!("Sunrise: could not reopen the audio output ({e}); it stays off");
                }
            }
        }
    }
    /// Opens a mod stream on the separate mod stream pool. The shared worker reads `source`
    /// on demand.
    pub fn open_stream(
        &self,
        owner: &'static str,
        spec: &mod_api::StreamSpec,
        entity_pos: Option<[f64; 3]>,
        source: Box<dyn mod_api::AudioStream>,
    ) -> std::result::Result<Arc<dyn mod_api::AudioStreamSource>, mod_api::AudioRefusal> {
        if !self.backend.live {
            mod_api::__private::game::CallbackOwner::for_mod(owner).dispose(
                "audio stream",
                "AudioStream::drop",
                source,
            );
            return Err(mod_api::AudioRefusal::NoOutput);
        }
        self.backend
            .streams
            .open(&self.backend.mixer, owner, spec, entity_pos, source)
    }
    /// Lists available microphone input devices on this client.
    pub fn microphone_devices(
        &self,
    ) -> std::result::Result<Vec<mod_api::MicDevice>, mod_api::MicRefusal> {
        self.mics.devices()
    }
    /// Opens a live microphone capture stream on this client.
    pub fn open_microphone(
        &self,
        spec: &mod_api::MicSpec,
    ) -> std::result::Result<Arc<dyn mod_api::MicSource>, mod_api::MicRefusal> {
        self.mics.open(spec)
    }
    /// Places each entity-placed stream at its entity's position as `resolve` finds it now, or
    /// stops it where it finds none, even while mod audio gain is zero (matching vanilla
    /// `SoundEngine.tickInGameSound`, which ticks and checks `isStopped()` before applying gain).
    /// Called once a frame; it does nothing, and locks nothing the output callback takes, with no
    /// stream open.
    pub fn follow_stream_entities(
        &self,
        resolve: impl FnMut(mod_api::EntityRef) -> Option<[f64; 3]>,
    ) {
        self.backend.streams.follow_entities(resolve);
    }
    /// The listener left its level (`updateLevelInEngines(null)`): every vanilla sound and mod
    /// stream stops. The one thing every way out of a level calls, so none can forget the streams.
    pub fn leave_level(&mut self) {
        self.stop_all();
    }
    /// `SoundManager.tick(paused)`, once per client tick.
    pub fn tick(&mut self, paused: bool, ctx: &TickContext) {
        #[cfg(feature = "audio")]
        {
            if self.should_change_device() {
                // SoundEngine.reload: stop every sound, open the output again.
                self.stop_all();
                self.reopen_output(false);
            } else if self.backend.owner == 0 && self.reopen.due() {
                self.reopen_output(true);
            }
        }
        self.sync_volumes();
        self.backend.streams.reap(&self.backend.mixer);
        self.sync_stream_gain(sunrise_hooks::sound::engine::mod_stream_gain());
        self.engine.tick(paused, ctx, &mut self.backend);
        // A paused game holds a mod's muffle where it is, as it holds the sounds.
        if !paused && let Some(fade) = &mut self.muffle {
            fade.left = fade.left.saturating_sub(1);
            let level = fade.level();
            if fade.left == 0 {
                self.muffle = None;
                self.engine
                    .set_unmuffled(Default::default(), &mut self.backend);
            }
            self.set_muffle_level(level);
        }
    }
    /// Gives the mixer the volume each stream plays at: the sound's own times `mod_gain`. The
    /// mixer is locked only when the volumes changed.
    fn sync_stream_gain(&mut self, mod_gain: f32) {
        let gain = std::array::from_fn(|source| {
            self.engine.final_source_volume(source).clamp(0.0, 1.0)
                * self.engine.source_scale(source)
                * mod_gain
        });
        if gain != self.stream_gain_sent
            && let Ok(mut m) = self.backend.mixer.lock()
        {
            m.stream_gain = gain;
            self.stream_gain_sent = gain;
        }
    }
    /// A mod's muffle (`native_muffle_sounds`): `amount` (0 to 1) of every sound's volume taken
    /// away and `lowpass` (0 to 1) dulling it, from untouched to a 300 Hz low-pass, fading in a
    /// straight line to none over `ticks` client ticks; the sound events in `exempt` (namespaced,
    /// `mymod:ear.ringing`) play clearly through it, those already playing too. It replaces any
    /// muffle in place; `ticks` 0 or no strength ends it. It affects the shared output.
    /// False for an amount or strength outside 0 to 1.
    pub fn muffle(&mut self, amount: f32, lowpass: f32, ticks: u32, exempt: &[&str]) -> bool {
        if !(0.0..=1.0).contains(&amount) || !(0.0..=1.0).contains(&lowpass) {
            return false;
        }
        let on = ticks > 0 && (amount > 0.0 || lowpass > 0.0);
        let exempt = if on {
            exempt
                .iter()
                .map(|e| crate::sound_events::normalize(e))
                .collect()
        } else {
            Default::default()
        };
        self.engine.set_unmuffled(exempt, &mut self.backend);
        self.muffle = on.then_some(MuffleFade {
            amount,
            lowpass,
            ticks,
            left: ticks,
        });
        self.set_muffle_level(self.muffle.map_or([0.0; 2], |fade| fade.level()));
        true
    }
    fn set_muffle_level(&mut self, level: [f32; 2]) {
        if let Ok(mut m) = self.backend.mixer.lock() {
            m.muffle.target = level;
        }
    }
    /// The `uiSoundReserve` extra: positional sounds leave
    /// `sound_engine::HEARD_RESERVE` static channels free (off: vanilla's pool).
    pub fn set_listener_reserve(&mut self, on: bool) {
        self.engine.listener_reserve = if on {
            crate::sound_engine::HEARD_RESERVE
        } else {
            0
        };
    }
    /// `SoundManager.pauseAllExcept(MUSIC, UI)`.
    pub fn pause(&mut self) {
        self.engine.pause_all_except(
            &[crate::sound_engine::MUSIC, crate::sound_engine::UI],
            &mut self.backend,
        );
    }
    /// Every playing sound paused, music and the interface's included (a client mod's pause).
    pub fn pause_all(&mut self) {
        self.engine.pause_all_except(&[], &mut self.backend);
    }
    /// A client mod's scale over `source`'s volume, applied to the sounds playing now.
    pub fn set_source_scale(&mut self, source: usize, scale: f32) {
        self.engine
            .set_source_scale(source, scale, &mut self.backend);
    }
    /// `SoundManager.resume`.
    pub fn resume(&mut self) {
        self.engine.resume(&mut self.backend);
    }
    /// `SoundManager.stop()`: every sound.
    pub fn stop_all(&mut self) {
        self.engine.stop_all(&mut self.backend);
        if self.backend.owner == 0 {
            self.backend.streams.stop_all();
        }
    }
    /// `SoundManager.reload` (resource packs changed): stop every sound, then
    /// read sound events and sound files from the new resource root.
    pub fn reload(&mut self, root: &ResourceSource) {
        self.reload_with(root, SoundRegistry::load(root));
    }
    /// [`Self::reload`] with the root's sound events already read
    /// (`SoundRegistry::load`, which a pack reload runs off the render thread).
    pub fn reload_with(&mut self, root: &ResourceSource, registry: SoundRegistry) {
        self.stop_all();
        self.engine.registry = registry;
        self.backend.root = root.to_owned();
        // A queued decode retains its old resource generation and cache. Dropping
        // the sender lets it finish without populating the new generation's cache.
        self.backend.decoder = None;
        self.backend.cache = Arc::new(Mutex::new(SoundCache::default()));
    }
    /// Reserve the seed of a local level request before its cancellable hooks run.
    pub fn next_sound_seed(&mut self) -> i64 {
        self.random.next_long()
    }
    /// Play a sound event at a world position, as `ClientLevel.playLocalSound`
    /// (a `SimpleSoundInstance` seeded from the level random), or relative to
    /// the listener without attenuation (`None`).
    pub fn play(&mut self, event: &str, at: Option<[f64; 3]>, volume: f32, pitch: f32) {
        let seed = self.random.next_long();
        self.play_seeded(event, at, volume, pitch, seed);
    }
    /// `ClientLevel.playLocalSound` with `distanceDelay`: a sound more than 10
    /// blocks from the camera starts `sqrt(distance) / 40` seconds later.
    pub fn play_distance_delayed(
        &mut self,
        event: &str,
        at: [f64; 3],
        volume: f32,
        pitch: f32,
        camera: [f64; 3],
    ) {
        let distance_sqr: f64 = (0..3).map(|i| (at[i] - camera[i]).powi(2)).sum();
        if distance_sqr <= 100.0 {
            self.play(event, Some(at), volume, pitch);
            return;
        }
        self.play_after(
            event,
            at,
            volume,
            pitch,
            (distance_sqr.sqrt() / 40.0 * 20.0) as i32,
        );
    }
    /// `play` at a world position, starting `ticks` sound-engine ticks (20 a
    /// second) from now: `SoundEngine.playDelayed`.
    pub fn play_after(&mut self, event: &str, at: [f64; 3], volume: f32, pitch: f32, ticks: i32) {
        let seed = self.random.next_long();
        if let Some(capture) = self.capture.as_mut() {
            capture.push(CapturedSound {
                event: event.to_owned(),
                position: at,
                volume,
                pitch,
                source: self.source_override.unwrap_or_else(|| {
                    crate::settings::sound_source(&crate::sound_events::normalize(event))
                }),
                entity: None,
                except_entity: None,
                seed,
                particles_before: crate::particles::requested_count(),
            });
        }
        *self.heard.entry(event.to_owned()).or_default() += 1;
        self.sync_volumes();
        let event = crate::sound_events::normalize(event);
        let source = self
            .source_override
            .unwrap_or_else(|| crate::settings::sound_source(&event));
        let instance =
            SoundInstance::at(&event, source, volume, pitch, LegacyRandom::new(seed), at);
        let id = self.engine.add(instance, false);
        self.engine.play_delayed(id, i64::from(ticks));
    }
    /// A client mod's sound (`ClientHost::play_local_sound`), as its `SoundPlay` gives it,
    /// starting `after` sound-engine ticks from now (`SoundEngine.playDelayed`) or at once.
    pub fn play_mod_sound(&mut self, sound: &mod_api::SoundPlay, after: u32) -> InstanceId {
        let seed = self.random.next_long();
        if let Some(capture) = self.capture.as_mut().filter(|_| !sound.relative) {
            capture.push(CapturedSound {
                event: sound.identifier.clone(),
                position: sound.position,
                volume: sound.volume,
                pitch: sound.pitch,
                source: sound.source.index(),
                entity: None,
                except_entity: None,
                seed,
                particles_before: crate::particles::requested_count(),
            });
        }
        *self.heard.entry(sound.identifier.clone()).or_default() += 1;
        let mut instance = SoundInstance::from_play(sound, LegacyRandom::new(seed));
        instance.identifier = crate::sound_events::normalize(&sound.identifier);
        // Tickable, so the engine applies a fade's volume each tick it plays; and it may start
        // silent (`canStartSilent`, as vanilla's tickable loops), so a play at volume 0 is
        // still there when a fade raises it.
        instance.ticking = crate::sound_engine::Ticking::Mod { fade: None };
        instance.can_start_silent = true;
        if after == 0 {
            self.play_instance(instance, false).0
        } else {
            self.sync_volumes();
            let id = self.engine.add(instance, false);
            self.engine.play_delayed(id, i64::from(after));
            id
        }
    }
    /// Where a client mod's sound (`SoundRef`) is.
    pub fn mod_sound_state(&self, id: InstanceId) -> mod_api::SoundState {
        self.engine.mod_sound_state(id)
    }
    /// A client mod's `SoundRef::stop`.
    pub fn stop_mod_sound(&mut self, id: InstanceId) {
        self.engine.stop_for_good(id, &mut self.backend);
    }
    /// A client mod's `SoundRef::fade`; an immediate nonpositive target stops the sound.
    pub fn fade_mod_sound(&mut self, id: InstanceId, volume: f32, ticks: u32) {
        if ticks == 0 && volume <= 0.0 {
            self.stop_mod_sound(id);
        } else if let Some(instance) = self.engine.instance_mut(id) {
            instance.fade_to(volume, ticks);
        }
    }
    /// A client mod's `SoundRef::move_to`: the engine's next tick plays the instance there.
    pub fn move_mod_sound(&mut self, id: InstanceId, position: [f64; 3]) {
        if let Some(instance) = self.engine.instance_mut(id) {
            instance.position = position;
        }
    }
    /// A client mod's `SoundRef::set_pitch`, applied by the engine's next tick.
    pub fn set_mod_sound_pitch(&mut self, id: InstanceId, pitch: f32) {
        if let Some(instance) = self.engine.instance_mut(id) {
            instance.pitch = pitch;
        }
    }
    /// `SoundManager.playDelayed` of an instance the caller made: it starts
    /// `ticks` sound-engine ticks from now.
    pub fn play_instance_after(&mut self, instance: SoundInstance, ticks: i32) {
        *self.heard.entry(instance.identifier.clone()).or_default() += 1;
        self.sync_volumes();
        let id = self.engine.add(instance, false);
        self.engine.play_delayed(id, i64::from(ticks));
    }
    /// `play` for this client only: a sound a packet's handler plays
    /// (`playLocalSound`), which the guests play from that packet themselves.
    pub fn play_unshared(&mut self, event: &str, at: Option<[f64; 3]>, volume: f32, pitch: f32) {
        let capture = self.capture.take();
        self.play(event, at, volume, pitch);
        self.capture = capture;
    }
    /// `AbstractWidget.playButtonClickSound`: an unseeded, relative UI sound.
    pub fn play_button_click(&mut self) {
        self.play_instance(
            SoundInstance::for_ui(
                "minecraft:ui.button.click",
                1.0,
                0.25,
                crate::random_source::create(),
            ),
            false,
        );
    }
    /// `SoundPreviewHandler.preview`: a category volume slider moved to
    /// `volume` on the title screen's options.
    pub fn preview(&mut self, source: usize, volume: f32) {
        let mut preview = std::mem::take(&mut self.preview);
        preview.preview(self, source, volume);
        self.preview = preview;
    }
    /// An incoming entity-sound packet: retain its seed and category without rebroadcasting it.
    pub fn play_entity_seeded(
        &mut self,
        event: &str,
        source: usize,
        volume: f32,
        pitch: f32,
        seed: i64,
        entity: crate::sound_engine::SoundEntity,
    ) {
        *self.heard.entry(event.to_owned()).or_default() += 1;
        let event = crate::sound_events::normalize(event);
        let instance = SoundInstance::entity_bound(
            &event,
            source,
            volume,
            pitch,
            LegacyRandom::new(seed),
            entity,
        );
        self.play_instance(instance, false);
    }
    /// A level's entity overload: preserve the emitter and excluded player for
    /// the server broadcaster while the local sound follows the same object.
    pub fn play_bound(
        &mut self,
        event: &str,
        source: usize,
        volume: f32,
        pitch: f32,
        entity: crate::sound_engine::SoundEntity,
        except_entity: Option<u32>,
    ) {
        let seed = self.random.next_long();
        if let Some(capture) = self.capture.as_mut() {
            capture.push(CapturedSound {
                event: event.to_owned(),
                position: entity.state().position,
                volume,
                pitch,
                source,
                entity: Some(entity.id),
                except_entity,
                seed,
                particles_before: crate::particles::requested_count(),
            });
        }
        self.play_entity_seeded(event, source, volume, pitch, seed, entity);
    }
    /// Removal belongs to the old object even if its numeric id is immediately reused.
    pub fn remove_sound_entity(&mut self, id: u32) {
        for entity in self
            .engine
            .bound_entities()
            .into_iter()
            .filter(|e| e.id == id)
        {
            let mut state = entity.state();
            state.removed = true;
            entity.update(state);
        }
    }
    /// `play` with the seed a sound packet carries.
    pub fn play_seeded(
        &mut self,
        event: &str,
        at: Option<[f64; 3]>,
        volume: f32,
        pitch: f32,
        seed: i64,
    ) {
        if let (Some(capture), Some(at)) = (self.capture.as_mut(), at) {
            capture.push(CapturedSound {
                event: event.to_owned(),
                position: at,
                volume,
                pitch,
                source: self.source_override.unwrap_or_else(|| {
                    crate::settings::sound_source(&crate::sound_events::normalize(event))
                }),
                entity: None,
                except_entity: None,
                seed,
                particles_before: crate::particles::requested_count(),
            });
        }
        if at.is_some() {
            *self.heard.entry(event.to_owned()).or_default() += 1;
        }
        self.sync_volumes();
        let event = crate::sound_events::normalize(event);
        let source = self
            .source_override
            .unwrap_or_else(|| crate::settings::sound_source(&event));
        let random = LegacyRandom::new(seed);
        let instance = match at {
            Some(at) => SoundInstance::at(&event, source, volume, pitch, random, at),
            None => SoundInstance::new(
                &event,
                source,
                volume,
                pitch,
                random,
                false,
                0,
                Attenuation::None,
                [0.0; 3],
                true,
            ),
        };
        self.play_instance(instance, false);
    }
    /// Play a sound on a given `SoundSource` index (`/playsound`).
    pub fn play_on(
        &mut self,
        source: usize,
        event: &str,
        at: Option<[f64; 3]>,
        volume: f32,
        pitch: f32,
    ) {
        let seed = self.random.next_long();
        self.play_on_seeded(source, event, at, volume, pitch, seed);
    }
    /// A level sound with an explicit category and seed, as received from the server.
    pub fn play_on_seeded(
        &mut self,
        source: usize,
        event: &str,
        at: Option<[f64; 3]>,
        volume: f32,
        pitch: f32,
        seed: i64,
    ) {
        self.source_override = Some(source);
        self.play_seeded(event, at, volume, pitch, seed);
        self.source_override = None;
    }
    /// `SimpleSoundInstance.forJukeboxSong` at a jukebox; the handle stays
    /// until `release`.
    pub fn play_jukebox_song(&mut self, event: &str, pos: [i32; 3]) -> InstanceId {
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        let event = crate::sound_events::normalize(event);
        let instance = SoundInstance::for_jukebox_song(&event, at, crate::random_source::create());
        self.play_instance(instance, true).0
    }
    /// Stop and drop a held instance.
    pub fn stop_and_release(&mut self, id: InstanceId) {
        self.stop_instance(id);
        self.release(id);
    }
    /// `SoundBufferLibrary.enumerate`: the decoded buffers kept and their bytes.
    pub fn cache_stats(&self) -> (usize, usize) {
        self.backend
            .cache
            .lock()
            .map_or((0, 0), |c| (c.entries.len(), c.bytes))
    }
    /// Every sound event id (for command completion).
    /// The engine's table sizes and the mixer's channels (`status`).
    pub fn census(&self) -> serde_json::Value {
        let mut census = self.engine.census();
        census["channels"] = self
            .backend
            .mixer
            .lock()
            .map_or(0, |m| m.channels.len())
            .into();
        if let Ok(cache) = self.backend.cache.lock() {
            census["decoded"] = cache.entries.len().into();
            census["decoded_mib"] = (cache.bytes >> 20).into();
        }
        census
    }
    pub fn event_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.engine.registry.names().cloned().collect();
        v.sort();
        v
    }
    /// `ClientboundStopSoundPacket`: stop the playing sounds of an event
    /// and/or a source.
    pub fn stop_sounds(&mut self, event: Option<&str>, source: Option<usize>) {
        let event = event.map(crate::sound_events::normalize);
        let selection = format!(
            "{} {}",
            source
                .and_then(|s| crate::settings::SOUND_SOURCES.get(s))
                .map_or("*", |s| s),
            event.as_deref().unwrap_or("*")
        );
        *self.stopped.entry(selection).or_default() += 1;
        self.engine
            .stop_matching(event.as_deref(), source, &mut self.backend);
    }
    /// The sounds played since the last call, for the subtitle overlay.
    pub fn take_heard(&mut self) -> Vec<Heard> {
        std::mem::take(&mut self.engine.notifications)
            .into_iter()
            .filter_map(|n| {
                Some(Heard {
                    subtitle: n.subtitle?,
                    at: Some(n.position),
                    range: n.range,
                    looping: n.looping,
                })
            })
            .collect()
    }
    /// Mixes one buffer of audio into `data` (channels: 1 or 2, rate in Hz).
    pub fn mix_buffer(&mut self, data: &mut [f32], channels: usize, rate: u32) {
        if let Ok(mut m) = self.backend.mixer.lock() {
            mix_pieces(&mut m, data, channels, rate);
        }
    }
    #[doc(hidden)]
    pub fn insert_cached_sound(&mut self, key: &str, samples: Vec<f32>, rate: u32, channels: u16) {
        if let Ok(mut c) = self.backend.cache.lock() {
            c.insert(
                key.to_owned(),
                Some((std::sync::Arc::new(samples), rate, channels)),
            );
        }
    }
    #[doc(hidden)]
    pub fn backend_live_for_bench(&mut self) {
        self.backend.live = true;
    }
}
impl Sounds for Audio {
    fn engine(&mut self) -> &mut SoundEngine {
        &mut self.engine
    }
    fn play_instance(&mut self, instance: SoundInstance, retain: bool) -> (InstanceId, PlayResult) {
        self.sync_volumes();
        self.bound().play_instance(instance, retain)
    }
    fn play_id(&mut self, id: InstanceId) -> PlayResult {
        self.sync_volumes();
        self.bound().play_id(id)
    }
    fn stop_instance(&mut self, id: InstanceId) {
        self.bound().stop_instance(id);
    }
    fn update_category_volume(&mut self, source: usize, gain: f32) {
        self.bound().update_category_volume(source, gain);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run of backend errors close together is a dead stream; one or two, or errors far apart,
    /// are hiccups.
    #[test]
    fn a_run_of_stream_errors_is_a_dead_stream() {
        use std::time::{Duration, Instant};
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut errors = StreamErrors::default();
        assert!(!errors.dead(at(0)));
        assert!(!errors.dead(at(500)));
        assert!(errors.dead(at(900)));
        // Spread out, they never add up.
        let mut apart = StreamErrors::default();
        for k in 0..10 {
            assert!(!apart.dead(at(k * 2500)), "error {k}");
        }
    }

    /// A failed reopening is tried again every two seconds, five more times, then given up.
    #[test]
    fn a_failed_reopening_retries_a_few_times_then_stops() {
        let mut reopen = Reopen::default();
        assert!(!reopen.due(), "nothing to retry while the output plays");
        assert!(reopen.failed(false));
        let mut tries = 0;
        for _ in 0..1000 {
            if reopen.due() {
                tries += 1;
                if !reopen.failed(true) {
                    break;
                }
            }
        }
        assert_eq!(tries, Reopen::TRIES);
        assert_eq!(reopen, Reopen::default());
        // The next lost device starts a fresh run.
        assert!(reopen.failed(false));
        assert_eq!(reopen.tries, Reopen::TRIES);
    }

    #[test]
    fn reload_isolates_pending_decodes_from_the_new_resource_cache() {
        let source = ResourceSource::indexed(Default::default());
        let mut audio = Audio::silent(&source);
        let old = audio.backend.cache.clone();
        old.lock().unwrap().insert("old".into(), None);
        audio.reload(&source);
        assert!(!Arc::ptr_eq(&old, &audio.backend.cache));
        old.lock().unwrap().insert("late".into(), None);
        assert!(audio.backend.cache.lock().unwrap().entries.is_empty());
    }

    #[test]
    #[ignore = "daily"]
    fn archive_sound_decodes_like_the_original_without_unpacking() {
        use std::io::Write;
        let prepared = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES").expect("resources"),
        );
        let source = ResourceSource::directory(prepared);
        let name = source
            .list("assets/minecraft/sounds/")
            .into_iter()
            .find(|name| name.ends_with(".ogg"))
            .expect("imported ogg");
        let bytes = source.read(&name).unwrap();
        let original = decode(&Resource::bytes(bytes.clone())).expect("vanilla ogg decodes");
        assert!(!original.0.is_empty());
        let folder = std::env::temp_dir().join(format!(
            "sunrise-archive-sound-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&folder).unwrap();
        let jar = folder.join("sound.jar");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&jar).unwrap());
        writer
            .start_file(&name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(&bytes).unwrap();
        writer.finish().unwrap();
        let resource = Resource::file(ResourceSource::archive(&jar).unwrap(), name);
        std::fs::remove_file(&jar).unwrap();
        let archived = decode(&resource).expect("retained archive ogg decodes");
        assert_eq!(archived, original);
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);
        std::fs::remove_dir(&folder).unwrap();
    }

    #[test]
    fn explicit_category_seed_does_not_advance_local_sound_random() {
        let source = ResourceSource::indexed(Default::default());
        let mut audio = Audio::silent(&source);
        let mut expected = audio.random.clone();
        audio.capture = Some(Vec::new());
        audio.play_on_seeded(
            8,
            "minecraft:entity.cow.ambient",
            Some([1.0; 3]),
            0.5,
            1.5,
            -17,
        );
        assert_eq!(audio.random.next_long(), expected.next_long());
        let capture = audio.capture.as_ref().unwrap();
        assert_eq!(capture.len(), 1);
        assert_eq!(capture[0].source, 8);
        assert_eq!(capture[0].position, [1.0; 3]);
        assert_eq!((capture[0].volume, capture[0].pitch), (0.5, 1.5));
        assert!(audio.source_override.is_none());
        // A local request reserves one draw before dispatch, and playback reuses it.
        let seed = audio.next_sound_seed();
        assert_eq!(seed, expected.next_long());
        audio.play_on_seeded(
            7,
            "minecraft:entity.player.swim",
            Some([0.0; 3]),
            1.0,
            1.0,
            seed,
        );
        assert_eq!(audio.random.next_long(), expected.next_long());
    }

    #[test]
    fn level_capture_keeps_order_category_entity_and_actor_exclusion() {
        let mut audio = Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            (Path::new("/nonexistent/sunrise-entity-sound-test")).to_path_buf(),
        ));
        audio.capture = Some(Vec::new());
        let mut expected = audio.random.clone();
        audio.play_on(
            7,
            "minecraft:block.amethyst_block.chime",
            Some([1.0; 3]),
            0.5,
            1.0,
        );
        let entity = crate::sound_engine::SoundEntity::new(37, [2.123456789, 64.0, -3.0], false);
        audio.play_bound(
            "minecraft:block.amethyst_block.chime",
            6,
            2.0,
            1.2,
            entity,
            Some(17),
        );
        let sounds = audio.capture.as_ref().unwrap();
        assert_eq!(sounds.len(), 2);
        assert_eq!(
            (sounds[0].source, sounds[0].entity, sounds[0].except_entity),
            (7, None, None)
        );
        let bound = &sounds[1];
        assert_eq!(
            (bound.source, bound.entity, bound.except_entity),
            (6, Some(37), Some(17))
        );
        assert_eq!(bound.position, [2.123456789, 64.0, -3.0]);
        assert_eq!((bound.volume, bound.pitch), (2.0, 1.2));
        expected.next_long();
        expected.next_long();
        assert_eq!(audio.random.next_long(), expected.next_long());
    }

    #[test]
    fn entity_sound_packets_neither_rebroadcast_nor_advance_the_local_seed_generator() {
        let mut audio = Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            (Path::new("/nonexistent/sunrise-entity-sound-test")).to_path_buf(),
        ));
        audio.capture = Some(Vec::new());
        let mut expected = audio.random.clone();
        let entity = crate::sound_engine::SoundEntity::new(7, [1.0, 2.0, 3.0], false);
        audio.play_entity_seeded("minecraft:fixture.bound", 6, 0.7, 1.2, 42, entity);
        assert!(audio.capture.as_ref().unwrap().is_empty());
        assert_eq!(audio.random.next_long(), expected.next_long());
    }
    #[test]
    fn removal_marks_only_existing_references_to_that_entity() {
        use crate::sound_engine::SoundEntity;
        let mut audio = Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            (Path::new("/nonexistent/sunrise-entity-sound-test")).to_path_buf(),
        ));
        let old = SoundEntity::new(7, [0.0; 3], false);
        let other = SoundEntity::new(8, [0.0; 3], false);
        for entity in [old.clone(), other.clone()] {
            audio.engine.add(
                SoundInstance::entity_bound(
                    "minecraft:fixture.bound",
                    6,
                    1.0,
                    1.0,
                    LegacyRandom::new(42),
                    entity,
                ),
                true,
            );
        }
        audio.remove_sound_entity(7);
        let replacement = SoundEntity::new(7, [10.0; 3], false);
        assert!(old.state().removed);
        assert!(!other.state().removed);
        assert!(!replacement.state().removed);
    }

    #[test]
    fn button_click_uses_vanilla_ui_parameters_without_advancing_world_random() {
        let mut audio = Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            (Path::new("/nonexistent/sunrise-test-assets")).to_path_buf(),
        ));
        audio.engine.registry.add_file(
            "minecraft",
            br#"{"ui.button.click":{"sounds":["gui/button"]}}"#,
            &|_, _| true,
        );
        // Exercise the mixer without opening an output device. The missing
        // sample is harmless: this test checks the source the engine creates.
        audio.engine.loaded = true;
        audio.backend.live = true;
        let mut random = audio.random.clone();
        audio.play_button_click();
        let instance = audio.engine.instance(1).expect("click reached the engine");
        assert_eq!(instance.identifier, "minecraft:ui.button.click");
        assert_eq!(instance.source, crate::sound_engine::UI);
        assert_eq!(instance.volume, 0.25);
        assert_eq!(instance.pitch, 1.0);
        assert_eq!(instance.attenuation, Attenuation::None);
        assert_eq!(instance.position, [0.0; 3]);
        assert!(instance.relative);
        assert!(!instance.looping);
        assert_eq!(audio.random.next_long(), random.next_long());
        audio.pause();
        assert_eq!(
            audio.backend.mixer.lock().unwrap().channels[&1].state,
            State::Initial
        );
        audio.stop_all();
    }

    /// A client mod's `SoundPlay` reaches the engine as it says: an interface sound at once,
    /// at the listener; a positioned, looping one on its own source, queued `after` ticks.
    #[test]
    fn a_mod_sound_plays_as_its_sound_play_says() {
        use mod_api::{SoundPlay, SoundSource};
        let mut audio = Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            (Path::new("/nonexistent/sunrise-test-assets")).to_path_buf(),
        ));
        audio.engine.registry.add_file(
            "minecraft",
            br#"{"ui.button.click":{"sounds":["gui/button"]}}"#,
            &|_, _| true,
        );
        audio.engine.registry.add_file(
            "mymod",
            br#"{"boom":{"sounds":["mymod:boom"]}}"#,
            &|_, _| true,
        );
        audio.engine.loaded = true;
        audio.backend.live = true;

        audio.play_mod_sound(
            &SoundPlay::ui("minecraft:ui.button.click").with_volume(0.5),
            0,
        );
        let click = audio
            .engine
            .instance(1)
            .expect("the click reached the engine");
        assert_eq!(click.source, crate::sound_engine::UI);
        assert_eq!((click.volume, click.pitch), (0.5, 1.0));
        assert_eq!(click.attenuation, Attenuation::None);
        assert!(click.relative && !click.looping);
        assert_eq!(audio.engine.census()["queued"], 0);

        let boom = SoundPlay::at("mymod:boom", SoundSource::Hostile, [1.0, 64.0, 2.0])
            .with_pitch(1.5)
            .looping(20);
        audio.play_mod_sound(&boom, 40);
        let queued = audio
            .engine
            .instance(2)
            .expect("the boom is held for later");
        assert_eq!(queued.source, SoundSource::Hostile as usize);
        assert_eq!(queued.position, [1.0, 64.0, 2.0]);
        assert_eq!((queued.pitch, queued.delay), (1.5, 20));
        assert_eq!(queued.attenuation, Attenuation::Linear);
        assert!(queued.looping && !queued.relative);
        assert_eq!(audio.engine.census()["queued"], 1, "it waits its 40 ticks");
        audio.stop_all();
    }

    fn channel(samples: Vec<f32>, linear: Option<f32>, at: [f32; 3], looping: bool) -> Channel {
        Channel {
            samples: Some((Arc::new(samples), 48_000, 1)),
            position: 0.0,
            state: State::Playing,
            gain: 1.0,
            pitch: 1.0,
            linear,
            looping,
            at,
            relative: false,
            owner: 0,
            world: 0,
            started: None,
            unmuffled: false,
            fx: fx::VoiceFx::default(),
        }
    }

    #[test]
    fn an_unreadable_pan_does_not_silence_other_voices() {
        let clear = || stereo_channel(vec![0.25; 6], None, [0.0; 3], false);
        let mut mixer = Mixer::default();
        mixer.channels.insert(1, clear());
        let mut plain = [0.0; 4];
        mix(&mut mixer, &mut plain, 2, 48_000);
        assert_eq!(plain, [0.25; 4]);

        mixer.channels.insert(1, clear());
        mixer
            .channels
            .insert(2, channel(vec![0.5; 4], None, [f32::NAN, 0.0, 0.0], false));
        let mut mixed = [0.0; 4];
        mix(&mut mixer, &mut mixed, 2, 48_000);
        assert_eq!(mixed, plain);
        assert_eq!(mixer.channels[&2].position, 2.0);
    }

    #[test]
    fn a_readable_extreme_position_pans_alongside_an_ordinary_voice() {
        let max = f64::from(f32::MAX);
        let axis = f32::MAX * 0.75;
        let poses = [
            // Nearly level with the pitched listener: 2 MAX left, 0.75 sqrt(2) MAX forward.
            // This exceeds the stereo decoder's 60-degree full-left angle.
            (
                [-max, 0.0, 0.0],
                45.0,
                [f32::MAX, -axis, axis],
                false,
                [0.75, 0.25],
            ),
            // Directly left at 60 degrees elevation: 1 MAX left and sqrt(3) MAX up.
            // The elevation halves the left-right spread, so the gains are 0.75 and 0.25.
            (
                [0.0, (1.0 - 3.0_f64.sqrt()) * max, 0.0],
                0.0,
                [f32::MAX, f32::MAX, 0.0],
                false,
                [0.625, 0.375],
            ),
            // Listener-relative right and ahead (Z negated), ignoring the listener pose:
            // length sqrt(1.25) MAX > f32::MAX, azimuth atan2(1, 0.5) > 60 degrees -> full right.
            (
                [max, 0.0, 0.0],
                45.0,
                [f32::MAX, 0.0, -(f32::MAX * 0.5)],
                true,
                [0.25, 0.75],
            ),
        ];
        for (listener, listener_pitch, at, relative, [left, right]) in poses {
            for (linear, expected) in [
                (None, [left, right, left, right]),
                (Some(f32::MAX), [0.25; 4]),
                (Some(f32::INFINITY), [left, right, left, right]),
            ] {
                let mut mixer = Mixer {
                    listener,
                    listener_orientation: camera_orientation(0.0, listener_pitch, 0.0),
                    ..Mixer::default()
                };
                mixer
                    .channels
                    .insert(1, stereo_channel(vec![0.25; 6], None, [0.0; 3], false));
                let mut voice = channel(vec![0.5; 4], linear, at, false);
                voice.relative = relative;
                mixer.channels.insert(2, voice);
                let mut mixed = [0.0; 4];
                mix(&mut mixer, &mut mixed, 2, 48_000);
                assert_eq!(mixed, expected);
            }
        }
    }

    /// A channel whose buffer is interleaved stereo.
    fn stereo_channel(
        samples: Vec<f32>,
        linear: Option<f32>,
        at: [f32; 3],
        looping: bool,
    ) -> Channel {
        let mut c = channel(Vec::new(), linear, at, looping);
        c.samples = Some((Arc::new(samples), 48_000, 2));
        c
    }

    #[test]
    fn decoded_sounds_stay_within_the_budget_least_recently_used_out() {
        let mut cache = SoundCache::default();
        let quarter = CACHE_BYTES / 4 / std::mem::size_of::<f32>();
        let sound = || Some((Arc::new(vec![0.0f32; quarter]), 44100, 1));
        for key in ["a", "b", "c", "d"] {
            cache.insert(key.into(), sound());
        }
        assert!(cache.get("a").is_some());
        cache.insert("e".into(), sound());
        assert!(cache.bytes <= CACHE_BYTES);
        assert!(
            cache.get("b").is_none(),
            "the least recently used goes first"
        );
        assert!(cache.get("a").is_some() && cache.get("e").is_some());
        // A failed load is remembered without counting.
        cache.insert("missing".into(), None);
        assert_eq!(cache.get("missing"), Some(None));
    }
    #[test]
    fn output_stays_off_on_a_virtual_screen_unless_asked() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| std::ffi::OsString::from(*v))
            }
        };
        assert!(output_disabled(env(&[("WAYLAND_DISPLAY", "wayland-1")])).is_none());
        assert!(output_disabled(env(&[])).is_none());
        const VSCREEN: &str = "/run/user/1000/vscreen/main/wayland-1";
        assert!(output_disabled(env(&[("WAYLAND_DISPLAY", VSCREEN)])).is_some());
        assert!(
            output_disabled(env(&[("WAYLAND_DISPLAY", VSCREEN), ("SUNRISE_AUDIO", "1")])).is_none()
        );
        assert!(
            output_disabled(env(&[("SUNRISE_NO_AUDIO", "1"), ("SUNRISE_AUDIO", "1")])).is_some()
        );
    }
    #[test]
    fn capture_ignores_virtual_screen_heuristic_but_honors_no_audio_override() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| std::ffi::OsString::from(*v))
            }
        };
        const VSCREEN: &str = "/run/user/1000/vscreen/main/wayland-1";
        assert!(!capture_disabled(env(&[])));
        assert!(!capture_disabled(env(&[("WAYLAND_DISPLAY", "wayland-1")])));
        assert!(!capture_disabled(env(&[("WAYLAND_DISPLAY", VSCREEN)])));
        assert!(capture_disabled(env(&[("SUNRISE_NO_AUDIO", "1")])));
        assert!(capture_disabled(env(&[
            ("SUNRISE_NO_AUDIO", "1"),
            ("SUNRISE_AUDIO", "1"),
        ])));
    }
    #[test]
    #[cfg(feature = "audio")]
    fn buffer_size_override_and_defaults() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| std::ffi::OsString::from(*v))
            }
        };
        let range = (256, 4096);
        assert_eq!(
            target_buffer_size(range, env(&[("SUNRISE_AUDIO_BUFFER", "default")])),
            Some(cpal::BufferSize::Default)
        );
        assert_eq!(
            target_buffer_size(range, env(&[("SUNRISE_AUDIO_BUFFER", "2048")])),
            Some(cpal::BufferSize::Fixed(2048))
        );
        assert_eq!(
            target_buffer_size(range, env(&[("SUNRISE_AUDIO_BUFFER", "100")])),
            Some(cpal::BufferSize::Fixed(256))
        );
        assert_eq!(
            target_buffer_size(range, env(&[("SUNRISE_AUDIO_BUFFER", "10000")])),
            Some(cpal::BufferSize::Fixed(4096))
        );
        #[cfg(target_os = "linux")]
        assert_eq!(
            target_buffer_size(range, env(&[])),
            Some(cpal::BufferSize::Fixed(1024))
        );
        #[cfg(not(target_os = "linux"))]
        assert_eq!(target_buffer_size(range, env(&[])), None);
    }
    #[test]
    fn linear_attenuation_reaches_zero_at_the_max_distance() {
        assert_eq!(distance_gain(None, 100.0), 1.0);
        assert_eq!(distance_gain(Some(16.0), 0.0), 1.0);
        assert_eq!(distance_gain(Some(16.0), 8.0), 0.5);
        assert_eq!(distance_gain(Some(16.0), 16.0), 0.0);
        assert_eq!(distance_gain(Some(16.0), 40.0), 0.0);
    }

    #[test]
    fn overlapping_voices_are_limited_without_flattening_or_changing_stereo_balance() {
        let mut m = Mixer::default();
        let wave = vec![0.25, 0.5, 0.75, 1.0, -1.0, -0.75, -0.5, -0.25, 0.0];
        for id in 1..=4 {
            m.channels
                .insert(id, channel(wave.clone(), None, [-1.0, 0.0, 0.0], false));
        }
        let mut data = [0.0; 16];
        mix(&mut m, &mut data, 2, 48_000);
        // West of a listener facing south: fully on the right.
        for (frame, sample) in data.as_chunks::<2>().0.iter().zip(wave) {
            assert!(frame[0].abs() < 1e-6, "{frame:?}");
            assert!((frame[1] - sample).abs() < 1e-6, "{frame:?}");
        }
        assert!(data.iter().all(|s| s.abs() <= 1.0));
    }

    #[test]
    fn limiter_leaves_quiet_audio_alone_and_recovers_across_callbacks() {
        let mut m = Mixer::default();
        m.channels
            .insert(1, channel(vec![0.25; 2], None, [0.0; 3], true));
        let quiet = 0.25 * stereo_gains(0.0, 0.0, 1.0)[0];
        let mut data = [0.0; 16];
        mix(&mut m, &mut data, 2, 48_000);
        assert_eq!(data, [quiet; 16]);

        for id in 2..=5 {
            m.channels
                .insert(id, channel(vec![1.0; 2], None, [0.0; 3], true));
        }
        mix(&mut m, &mut data, 2, 48_000);
        assert!(data.iter().all(|s| s.abs() <= 1.0));
        m.channels.retain(|&id, _| id == 1);
        let mut recovery = vec![0.0; 2 * 48_000];
        mix(&mut m, &mut recovery, 2, 48_000);
        assert!(recovery[0] > 0.0 && recovery[0] < 0.1);
        assert!(recovery.windows(2).all(|w| w[1] >= w[0]));
        assert_eq!(*recovery.last().unwrap(), quiet);
    }

    #[test]
    fn sources_play_attenuated_then_stop_or_loop() {
        let mut m = Mixer::default();
        m.channels
            .insert(1, channel(vec![1.0; 4], Some(16.0), [8.0, 0.0, 0.0], false));
        m.channels
            .insert(2, channel(vec![1.0; 4], None, [0.0; 3], true));
        let mut data = [0.0f32; 16];
        mix(&mut m, &mut data, 2, 48_000);
        assert_eq!(m.channels[&1].state, State::Stopped);
        assert_eq!(m.channels[&2].state, State::Playing);
        // Mono sum of the two at the first frame: the far one is half as loud.
        let first = data[0] + data[1];
        assert!(first > 1.4 && first < 3.1, "{first}");
    }

    #[test]
    fn stereo_panning_matches_openal_soft() {
        // OpenAL Soft 1.25.2 (LWJGL 3.4.3's macOS build), stereo loopback
        // device with vanilla's attributes, 1 kHz sine 4 blocks away.
        let cases = [
            ((0.0, 0.0, 1.0), [0.5957, 0.5957]),
            ((-1.0, 0.0, 0.0), [1.0, 0.0]),
            ((1.0, 0.0, 0.0), [0.0, 1.0]),
            ((0.0, 0.0, -1.0), [0.4043, 0.4043]),
            ((0.0, 1.0, 0.0), [0.5, 0.5]),
            ((0.0, -1.0, 0.0), [0.5, 0.5]),
            ((-1.0, 0.0, 1.0), [0.9985, 0.0747]),
            ((-1.0, 0.0, -1.0), [0.7859, 0.0788]),
            ((0.0, 0.0, 0.0), [0.5957, 0.5957]),
        ];
        for ((right, up, forward), expected) in cases {
            let gains = stereo_gains(right, up, forward);
            for (g, e) in gains.into_iter().zip(expected) {
                assert!((g - e).abs() < 1e-4, "{right} {up} {forward}: {gains:?}");
            }
        }
    }

    fn camera_orientation(yaw: f32, pitch: f32, roll: f32) -> ListenerOrientation {
        let rotation = crate::camera::rotation_with_roll(yaw, pitch, roll);
        ListenerOrientation {
            forward: rotation.transform([0.0, 0.0, -1.0]),
            up: rotation.transform([0.0, 1.0, 0.0]),
        }
    }

    #[test]
    fn stereo_sounds_play_direct_without_panning_or_distance() {
        // OpenAL Soft plays a multi-channel buffer direct (its non-spatialized path), so vanilla's
        // stereo music and records keep their own left/right mix at full volume whichever way the
        // camera faces and however far it is. Only mono buffers are panned and attenuated.
        let wave = [0.1, -0.2, 0.3, -0.4, 0.5, -0.6]; // three stereo frames
        let expected = [wave[0], wave[1], wave[2], wave[3]];
        for (yaw, at, linear, listener) in [
            (0.0, [100.0, 0.0, 0.0], Some(16.0), [0.0; 3]),
            (180.0, [0.0, 0.0, -100.0], Some(16.0), [0.0; 3]),
            (90.0, [2.0, 1.0, -3.0], Some(16.0), [5.0, 60.0, 5.0]),
            (0.0, [0.0, 0.0, 0.0], None, [0.0; 3]),
        ] {
            let mut m = Mixer {
                listener,
                listener_orientation: camera_orientation(yaw, 0.0, 0.0),
                ..Mixer::default()
            };
            m.channels
                .insert(1, stereo_channel(wave.to_vec(), linear, at, false));
            let mut data = [0.0; 4];
            mix(&mut m, &mut data, 2, 48_000);
            assert_eq!(data, expected, "yaw {yaw}, source {at:?}");
        }
    }

    #[test]
    fn panning_follows_the_camera_rotation() {
        let gains = |yaw: f32, pitch: f32, at: [f32; 3]| {
            let mut m = Mixer {
                listener_orientation: camera_orientation(yaw, pitch, 0.0),
                ..Mixer::default()
            };
            m.channels.insert(1, channel(vec![1.0; 4], None, at, true));
            let mut data = [0.0; 2];
            mix(&mut m, &mut data, 2, 48_000);
            data
        };
        let close =
            |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4;
        // Facing south (yaw 0), east is on the left; facing north, on the right.
        assert!(close(gains(0.0, 0.0, [1.0, 0.0, 0.0]), [1.0, 0.0]));
        assert!(close(gains(180.0, 0.0, [1.0, 0.0, 0.0]), [0.0, 1.0]));
        // Looking straight down, a source below is ahead and one south is above.
        assert!(close(gains(0.0, 90.0, [0.0, -1.0, 0.0]), [0.5957, 0.5957]));
        assert!(close(gains(0.0, 90.0, [0.0, 0.0, 1.0]), [0.5, 0.5]));
    }

    #[test]
    fn changing_only_camera_roll_changes_the_stereo_image() {
        let mut audio = Audio::silent(&ResourceSource::indexed(Default::default()));
        let mut gains = |roll| {
            let orientation = camera_orientation(180.0, 0.0, roll);
            audio.set_listener([0.0; 3], orientation.forward, orientation.up);
            let mut mixer = audio.backend.mixer.lock().unwrap();
            mixer
                .channels
                .insert(1, channel(vec![1.0; 4], None, [0.0, 1.0, 0.0], true));
            let mut stereo = [0.0; 2];
            mix(&mut mixer, &mut stereo, 2, 48_000);
            stereo
        };
        let [left, right] = gains(0.0);
        assert_eq!(left, right, "source directly above an upright camera");
        let [left, right] = gains(90.0);
        assert!(
            left > right,
            "the same source is to the rolled camera's left"
        );
    }

    #[test]
    fn shared_sounds_are_heard_from_the_nearest_camera() {
        // The primary listener is far away; the nearer listener hears full, centred gain.
        let mut m = Mixer {
            listener: [100.0, 0.0, 0.0],
            ..Mixer::default()
        };
        m.listeners
            .insert(1, ([0.0, 0.0, 1.0], ListenerOrientation::default()));
        m.channels
            .insert(1, channel(vec![1.0; 4], Some(16.0), [0.0, 0.0, 2.0], true));
        let mut data = [0.0; 2];
        mix(&mut m, &mut data, 2, 48_000);
        assert!(
            data[0] > 0.5 && (data[0] - data[1]).abs() < 1e-4,
            "{data:?}"
        );
        // The primary listener alone is beyond the 16-block range.
        m.listeners.clear();
        let mut data = [0.0; 2];
        mix(&mut m, &mut data, 2, 48_000);
        assert_eq!(data, [0.0; 2]);
    }

    #[test]
    fn a_shared_listener_registers_even_in_the_hosts_world() {
        let mut host = Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            (Path::new("/nonexistent/sunrise-test-audio")).to_path_buf(),
        ));
        host.set_world("minecraft:overworld");
        let mut guest = host.share(1);
        guest.set_world("minecraft:overworld");
        guest.set_listener([1.0, 2.0, 3.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]);
        let id = guest.backend.create(false).unwrap();
        let mixer = host.backend.mixer.lock().unwrap();
        let overworld = mixer
            .world_names
            .iter()
            .position(|w| w == "minecraft:overworld");
        let overworld = overworld.map(|i| i as u32 + 1);
        assert_eq!(mixer.worlds.get(&1).copied(), overworld);
        assert_eq!(Some(mixer.channels[&id].world), overworld);
    }

    #[test]
    fn cameras_in_other_dimensions_cannot_hear_a_source() {
        let mut m = Mixer {
            listener: [100.0, 0.0, 0.0],
            ..Default::default()
        };
        m.listeners
            .insert(1, ([0.0, 0.0, 1.0], ListenerOrientation::default()));
        let nether = m.world_id("nether");
        m.worlds.insert(1, nether);
        m.channels
            .insert(1, channel(vec![1.0; 4], Some(16.0), [0.0, 0.0, 2.0], true));
        let mut data = [0.0; 2];
        mix(&mut m, &mut data, 2, 48_000);
        assert_eq!(
            data, [0.0; 2],
            "nearby Nether camera must not hear Overworld source"
        );
        m.channels.get_mut(&1).unwrap().world = nether;
        mix(&mut m, &mut data, 2, 48_000);
        assert!(data[0] > 0.5, "same-world camera must hear source");
        let end = m.world_id("end");
        m.worlds.insert(1, end);
        data.fill(0.0);
        mix(&mut m, &mut data, 2, 48_000);
        assert_eq!(data, [0.0; 2], "no listener remains in source's world");
    }

    #[test]
    fn sounds_in_a_world_without_listeners_finish_while_inaudible() {
        let mut mixer = Mixer::default();
        let mut source = channel(vec![1.0; 4], None, [0.0; 3], false);
        let nether = mixer.world_id("nether");
        source.world = nether;
        mixer.channels.insert(1, source);
        let mut data = [0.0; 4];
        mix(&mut mixer, &mut data, 2, 48_000);
        assert_eq!(data, [0.0; 4]);
        assert!(mixer.channels[&1].position > 0.0);
        mix(&mut mixer, &mut data, 2, 48_000);
        assert_eq!(mixer.channels[&1].state, State::Stopped);
        mixer.worlds.insert(0, nether);
        mix(&mut mixer, &mut data, 2, 48_000);
        assert_eq!(data, [0.0; 4], "returning must not resume the old one-shot");
    }

    /// A sound of one frame or none, heard or not, looping or not, stops at its first
    /// buffer as the mixing loop stops it: none stays playing (and holds its channel).
    #[test]
    fn sounds_too_short_to_mix_stop_even_unheard() {
        for (samples, looping) in [(vec![], false), (vec![1.0], false), (vec![1.0], true)] {
            let mut mixer = Mixer::default();
            let mut source = channel(samples.clone(), None, [0.0; 3], looping);
            source.gain = 0.0;
            mixer.channels.insert(1, source);
            let mut data = [0.0; 4];
            mix(&mut mixer, &mut data, 2, 48_000);
            assert_eq!(
                mixer.channels[&1].state,
                State::Stopped,
                "{samples:?} {looping}"
            );
        }
    }

    #[test]
    fn variants_of_one_sound_are_the_same_sound() {
        assert_eq!(
            variantless("minecraft:mob/creeper/say1.ogg"),
            "minecraft:mob/creeper/say"
        );
        assert_eq!(
            variantless("minecraft:step/grass6.ogg"),
            "minecraft:step/grass"
        );
        assert_eq!(
            variantless("minecraft:random/click.ogg"),
            "minecraft:random/click"
        );
    }

    #[test]
    fn a_shared_backends_copy_of_a_sound_is_dropped() {
        let mixer = Arc::new(Mutex::new(Mixer::default()));
        // One additional backend shares the output.
        let shares = Arc::new(std::sync::atomic::AtomicUsize::new(1));
        let backend = |owner| MixerBackend {
            mixer: mixer.clone(),
            root: ResourceSource::indexed(Default::default()),
            cache: Arc::default(),
            decoder: None,
            next: (u64::from(owner)) << 40,
            live: true,
            streams: Arc::default(),
            owner,
            world: 0,
            shares: shares.clone(),
        };
        let (mut primary, mut shared) = (backend(0), backend(1));
        // The second listener is active, so the backends share the speakers.
        mixer
            .lock()
            .unwrap()
            .listeners
            .insert(1, ([0.0; 3], ListenerOrientation::default()));
        let started = |b: &mut MixerBackend, at: [f64; 3]| {
            let id = b.create(false).unwrap();
            b.set_position(id, at);
            !b.duplicate(id, ("minecraft", "mob/creeper/say1.ogg"))
        };
        assert!(started(&mut primary, [5.0, 64.0, 5.0]));
        // The shared backend got the same sound at the same place: one copy plays.
        assert!(!started(&mut shared, [5.2, 64.0, 5.0]));
        // A sound elsewhere, or repeated by the same backend, still plays.
        assert!(started(&mut shared, [40.0, 64.0, 5.0]));
        assert!(started(&mut primary, [5.0, 64.0, 5.0]));
        // Equal coordinates in another dimension describe a different sound.
        shared.world = mixer.lock().unwrap().world_id("nether");
        assert!(started(&mut shared, [5.0, 64.0, 5.0]));
        // Dropping the shared backend also removes its sounds.
        drop(shared);
        assert!(
            mixer
                .lock()
                .unwrap()
                .channels
                .values()
                .all(|c| c.owner == 0)
        );
    }

    #[test]
    fn reusing_a_seat_does_not_reuse_its_channels_or_leave_its_camera() {
        let host = Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            (Path::new("/nonexistent/sunrise-test-audio")).to_path_buf(),
        ));
        let mut guest = host.share(1);
        guest.set_world("overworld");
        guest.set_listener([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]);
        let old = guest.backend.create(false).unwrap();
        drop(guest);
        {
            let mixer = host.backend.mixer.lock().unwrap();
            assert!(!mixer.channels.contains_key(&old));
            assert!(!mixer.listeners.contains_key(&1));
            assert!(!mixer.worlds.contains_key(&1));
        }
        let mut replacement = host.share(1);
        let new = replacement.backend.create(false).unwrap();
        assert_ne!(old, new);
        assert!(
            replacement
                .backend
                .mixer
                .lock()
                .unwrap()
                .channels
                .contains_key(&new)
        );
    }

    #[test]
    fn a_mono_output_plays_sources_unpanned() {
        let mut m = Mixer::default();
        m.channels
            .insert(1, channel(vec![0.5; 4], None, [0.0, 0.0, -3.0], true));
        let mut data = [0.0; 2];
        mix(&mut m, &mut data, 1, 48_000);
        assert_eq!(data, [0.5; 2]);
    }

    /// One second of a `hz` sine at 48 kHz, amplitude 0.3 (looped, it repeats seamlessly).
    fn sine(hz: f32) -> Vec<f32> {
        (0..48_000)
            .map(|i| 0.3 * (std::f32::consts::TAU * hz * i as f32 / 48_000.0).sin())
            .collect()
    }

    fn rms(data: &[f32]) -> f32 {
        (data.iter().map(|s| s * s).sum::<f32>() / data.len() as f32).sqrt()
    }

    /// `seconds` of the mixer's stereo output, in the 10 ms buffers an output stream asks for.
    fn listen(m: &mut Mixer, seconds: f32) -> Vec<f32> {
        let mut out = Vec::new();
        let mut buffer = [0.0; 960];
        for _ in 0..(seconds * 100.0).round() as usize {
            mix(m, &mut buffer, 2, 48_000);
            out.extend_from_slice(&buffer);
        }
        out
    }

    /// A looping tone 2 blocks ahead, heard for `seconds` once `effects` have settled.
    fn heard_with(hz: f32, effects: mod_api::SoundEffects, seconds: f32) -> Vec<f32> {
        let mut m = Mixer::default();
        let mut c = channel(sine(hz), None, [0.0, 0.0, 2.0], true);
        c.fx.target = effects;
        m.channels.insert(1, c);
        listen(&mut m, 0.2);
        listen(&mut m, seconds)
    }

    #[test]
    fn a_sounds_low_pass_dulls_it_and_its_gain_turns_it_down() {
        let none = mod_api::SoundEffects::NONE;
        let plain = |hz: f32| rms(&heard_with(hz, none, 0.2));
        let mut dull = none;
        dull.lowpass = 500.0;
        // Three octaves and more above the cutoff, two poles: a 6 kHz whine all but goes.
        assert!(rms(&heard_with(6000.0, dull, 0.2)) < plain(6000.0) * 0.02);
        assert!(rms(&heard_with(100.0, dull, 0.2)) > plain(100.0) * 0.95);
        let mut thin = none;
        thin.highpass = 2000.0;
        assert!(rms(&heard_with(100.0, thin, 0.2)) < plain(100.0) * 0.01);
        assert!(rms(&heard_with(8000.0, thin, 0.2)) > plain(8000.0) * 0.95);
        let mut quiet = none;
        quiet.gain = 0.5;
        quiet.direct = 0.5;
        let ratio = rms(&heard_with(1000.0, quiet, 0.2)) / plain(1000.0);
        assert!((ratio - 0.25).abs() < 1e-3, "{ratio}");
    }

    #[test]
    fn effects_back_to_none_play_the_sound_exactly_as_without_them() {
        let mut m = Mixer::default();
        let mut c = channel(sine(440.0), None, [1.0, 0.0, 2.0], true);
        let mut effects = mod_api::SoundEffects::NONE;
        effects.lowpass = 800.0;
        effects.room = 1.0;
        c.fx.target = effects;
        m.channels.insert(1, c);
        listen(&mut m, 0.1);
        m.channels.get_mut(&1).unwrap().fx.target = mod_api::SoundEffects::NONE;
        // One buffer glides back, the next is the plain path again.
        listen(&mut m, 0.01);
        assert!(m.channels[&1].fx.bypassed());
        // Once the room's tail has died away, the output is a plain channel's, sample for sample.
        listen(&mut m, 2.0);
        let mut plain = Mixer::default();
        let mut c = channel(sine(440.0), None, [1.0, 0.0, 2.0], true);
        c.position = m.channels[&1].position;
        plain.channels.insert(1, c);
        assert_eq!(listen(&mut m, 0.1), listen(&mut plain, 0.1));
    }

    #[test]
    fn a_reverb_send_rings_on_after_the_sound_ends() {
        let tail = |send: f32| {
            let mut m = Mixer::default();
            // 50 ms of noise-like tone, not looping.
            let mut c = channel(sine(1234.0)[..2400].to_vec(), None, [0.0, 0.0, 2.0], false);
            let mut effects = mod_api::SoundEffects::NONE;
            effects.cavern = send;
            c.fx.target = effects;
            m.channels.insert(1, c);
            listen(&mut m, 0.1);
            assert_eq!(m.channels[&1].state, State::Stopped);
            rms(&listen(&mut m, 0.5))
        };
        assert_eq!(tail(0.0), 0.0);
        assert!(tail(1.0) > 1e-3, "{}", tail(1.0));
    }

    #[test]
    fn a_huge_buffer_with_reverb_sends_never_grows_the_send_buffer() {
        // 0.5 s at 192 kHz in one device buffer: 96,000 frames, mixed in pieces.
        let mut m = Mixer::default();
        let mut c = channel(sine(440.0), None, [0.0, 0.0, 2.0], true);
        let mut effects = mod_api::SoundEffects::NONE;
        effects.hall = 1.0;
        c.fx.target = effects;
        m.channels.insert(1, c);
        let (pointer, capacity) = (m.reverbs.input.as_ptr(), m.reverbs.input.capacity());
        assert_eq!(capacity, reverb::BUSES * MAX_MIX_FRAMES);
        let mut data = vec![0.0; 2 * 96_000];
        mix_pieces(&mut m, &mut data, 2, 192_000);
        assert!(data.iter().any(|s| *s != 0.0));
        assert_eq!(
            (m.reverbs.input.as_ptr(), m.reverbs.input.capacity()),
            (pointer, capacity)
        );
    }

    #[test]
    fn a_sound_started_with_effects_has_them_from_its_first_frame() {
        let mut m = Mixer::default();
        let mut c = channel(sine(440.0), None, [0.0, 0.0, 2.0], true);
        let mut silent = mod_api::SoundEffects::NONE;
        silent.gain = 0.0;
        silent.room = 1.0;
        c.fx.target = silent;
        m.channels.insert(1, c);
        assert!(listen(&mut m, 0.1).iter().all(|s| *s == 0.0));
        assert!(m.reverbs.buses.iter().all(reverb::Reverb::idle));
    }

    #[test]
    fn a_send_to_one_reverb_leaves_the_others_idle() {
        let mut m = Mixer::default();
        let mut c = channel(sine(440.0), None, [0.0, 0.0, 2.0], true);
        let mut effects = mod_api::SoundEffects::NONE;
        effects.room = 1.0;
        c.fx.target = effects;
        m.channels.insert(1, c);
        listen(&mut m, 0.1);
        let idle = m.reverbs.buses.each_ref().map(reverb::Reverb::idle);
        assert_eq!(idle, [false, true, true]);
    }

    #[test]
    fn a_huge_buffer_without_effects_mixes_as_one_buffer() {
        // 20,000 frames in one device buffer, too loud for the limiter only near the end: with no
        // effects it is one `mix` limited as a whole, bit for bit, not two pieces limited apart.
        let loud = || {
            let mut m = Mixer::default();
            let samples = sine(440.0)
                .iter()
                .enumerate()
                .map(|(i, s)| if i < 18_000 { *s } else { s * 6.0 })
                .collect();
            m.channels
                .insert(1, channel(samples, None, [0.0, 0.0, 2.0], true));
            m
        };
        let mut pieces = vec![0.0; 2 * 20_000];
        mix_pieces(&mut loud(), &mut pieces, 2, 48_000);
        let mut whole = vec![0.0; 2 * 20_000];
        mix(&mut loud(), &mut whole, 2, 48_000);
        assert!(whole.iter().any(|s| *s != 0.0));
        assert_eq!(
            pieces.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
            whole.iter().map(|s| s.to_bits()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_mono_output_plays_a_sound_with_effects_unpanned() {
        let mut m = Mixer::default();
        let mut c = channel(vec![0.5; 4], None, [0.0, 0.0, -3.0], true);
        let mut effects = mod_api::SoundEffects::NONE;
        effects.gain = 0.5;
        c.fx.target = effects;
        m.channels.insert(1, c);
        let mut data = [0.0; 2];
        mix(&mut m, &mut data, 1, 48_000);
        assert_eq!(data, [0.25; 2]);
    }

    #[test]
    fn changing_a_sounds_effects_glides_without_a_click() {
        // A steady tone whose gain and cutoff jump between buffers: the output never moves more
        // in one frame than the tone itself can (its largest step is 0.3 * TAU * 440 / 48000).
        let mut m = Mixer::default();
        let c = channel(sine(440.0), None, [0.0, 0.0, 2.0], true);
        m.channels.insert(1, c);
        let mut out = listen(&mut m, 0.05);
        for (gain, lowpass) in [
            (0.0, 20_000.0),
            (1.0, 300.0),
            (0.2, 20_000.0),
            (1.0, 20_000.0),
        ] {
            let mut e = mod_api::SoundEffects::NONE;
            e.gain = gain;
            e.lowpass = lowpass;
            m.channels.get_mut(&1).unwrap().fx.target = e;
            out.extend(listen(&mut m, 0.05));
        }
        let largest = 0.3 * std::f32::consts::TAU * 440.0 / 48_000.0;
        let jump = out
            .chunks(2)
            .zip(out.chunks(2).skip(1))
            .map(|(a, b)| (b[0] - a[0]).abs().max((b[1] - a[1]).abs()))
            .fold(0.0f32, f32::max);
        assert!(jump <= largest * 1.05, "{jump} > {largest}");
    }

    #[test]
    fn a_muffle_dulls_and_quietens_every_sound_but_the_exempt_ones() {
        // A looping tone heard through a muffle of `[amount, lowpass]`, once it has settled.
        let heard = |hz: f32, muffle: [f32; 2], unmuffled: bool| {
            let mut m = Mixer::default();
            let mut c = channel(sine(hz), None, [0.0, 0.0, 2.0], true);
            c.unmuffled = unmuffled;
            m.channels.insert(1, c);
            m.muffle.target = muffle;
            listen(&mut m, 0.2);
            listen(&mut m, 0.2)
        };
        let plain = |hz: f32| rms(&heard(hz, [0.0; 2], false));
        // Fully low-passed, a 6 kHz whine all but goes and a 100 Hz hum keeps most of its level.
        assert!(rms(&heard(6000.0, [0.0, 1.0], false)) < plain(6000.0) * 0.05);
        assert!(rms(&heard(100.0, [0.0, 1.0], false)) > plain(100.0) * 0.8);
        // Half the volume taken and no low-pass: half the level, the tone itself untouched.
        let ducked = rms(&heard(6000.0, [0.5, 0.0], false)) / plain(6000.0);
        assert!((ducked - 0.5).abs() < 1e-3, "{ducked}");
        // An exempt sound plays through as if there were no muffle, sample for sample.
        assert_eq!(
            heard(6000.0, [0.5, 1.0], true),
            heard(6000.0, [0.0; 2], false)
        );
    }

    #[test]
    fn a_muffles_low_pass_matches_a_per_frame_reference() {
        // The coefficient is reused while the glide hasn't moved `lowpass`; the output must
        // still equal recomputing it every frame, through the glide and once settled.
        let rate = 48_000u32;
        let input: Vec<f32> = (0..2 * 4800)
            .map(|i| ((i / 2) as f32 * 0.37).sin() * 0.5)
            .collect();
        let mut muffle = Muffle {
            target: [0.3, 0.8],
            ..Default::default()
        };
        let glide = 1.0 - (-1.0 / (MUFFLE_GLIDE * rate as f32)).exp();
        let (mut current, mut stages) = ([0.0f32; 2], [[0.0f32; 2]; 2]);
        for _ in 0..3 {
            let mut got = vec![0.0f32; input.len()];
            muffle.apply(&input, &mut got, 2, rate);
            let mut want = vec![0.0f32; input.len()];
            for (frame, heard) in want.chunks_mut(2).zip(input.chunks(2)) {
                for (c, t) in current.iter_mut().zip([0.3f32, 0.8]) {
                    *c += (t - *c) * glide;
                }
                let cutoff = 20_000.0f32 * (300.0f32 / 20_000.0).powf(current[1]);
                let a = 1.0 - (-std::f32::consts::TAU * cutoff / rate as f32).exp();
                for ((out, s), stage) in frame.iter_mut().zip(heard).zip(&mut stages) {
                    stage[0] += (s - stage[0]) * a;
                    stage[1] += (stage[0] - stage[1]) * a;
                    *out += (s + (stage[1] - s) * current[1]) * (1.0 - current[0]);
                }
            }
            assert_eq!(got, want);
        }
    }

    #[test]
    fn a_faded_muffle_leaves_the_sound_as_it_was() {
        let mixer = || {
            let mut m = Mixer::default();
            m.channels
                .insert(1, channel(sine(3000.0), None, [2.0, 0.0, 1.0], true));
            m
        };
        let (mut muffled, mut plain) = (mixer(), mixer());
        muffled.muffle.target = [0.7, 1.0];
        assert_ne!(listen(&mut muffled, 0.1), listen(&mut plain, 0.1));
        muffled.muffle.target = [0.0; 2];
        listen(&mut muffled, 0.5);
        listen(&mut plain, 0.5);
        assert_eq!(listen(&mut muffled, 0.1), listen(&mut plain, 0.1));
    }

    #[test]
    fn a_mods_muffle_fades_with_the_game_ticks_and_spares_its_exempt_sounds() {
        let mut audio = Audio::silent(&ResourceSource::directory(
            "/nonexistent/sunrise-test-assets".into(),
        ));
        audio.engine.registry.add_file(
            "minecraft",
            br#"{"ambient.cave":{"sounds":["ambient/cave/cave1"]}}"#,
            &|_, _| true,
        );
        audio.engine.registry.add_file(
            "hztest",
            br#"{"ear.ringing":{"sounds":["ear/ringing"]}}"#,
            &|_, _| true,
        );
        // The engine makes its channels; with no output, none loads a file.
        audio.engine.loaded = true;
        let marks = |audio: &Audio| {
            let m = audio.backend.mixer.lock().unwrap();
            let mut marks: Vec<_> = m
                .channels
                .iter()
                .map(|(&id, c)| (id, c.unmuffled))
                .collect();
            marks.sort();
            marks
        };
        let level = |audio: &Audio| audio.backend.mixer.lock().unwrap().muffle.target;
        let close =
            |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6;
        // Two sounds already playing, and one exempt sound started under the muffle.
        audio.play("ambient.cave", None, 1.0, 1.0);
        audio.play("hztest:ear.ringing", None, 1.0, 1.0);
        let exempt: Vec<_> = (0..65)
            .map(|n| format!("hztest:other.{n}"))
            .chain(std::iter::once("hztest:ear.ringing".to_owned()))
            .collect();
        let exempt: Vec<_> = exempt.iter().map(String::as_str).collect();
        assert!(audio.muffle(0.8, 1.0, 4, &exempt));
        audio.play("hztest:ear.ringing", None, 1.0, 1.0);
        assert_eq!(marks(&audio), [(1, false), (2, true), (3, true)]);
        assert!(close(level(&audio), [0.8, 1.0]));
        let ticks = TickContext::default();
        audio.tick(false, &ticks);
        assert!(close(level(&audio), [0.6, 0.75]), "{:?}", level(&audio));
        // A paused game holds it.
        audio.tick(true, &ticks);
        assert!(close(level(&audio), [0.6, 0.75]), "{:?}", level(&audio));
        for _ in 0..3 {
            audio.tick(false, &ticks);
        }
        assert_eq!(level(&audio), [0.0; 2]);
        // Over: its exemptions go with it.
        assert_eq!(marks(&audio), [(1, false), (2, false), (3, false)]);
        // Out of range changes nothing.
        assert!(!audio.muffle(1.5, 0.0, 10, &[]));
        assert_eq!(level(&audio), [0.0; 2]);
    }

    #[test]
    fn output_devices_are_looked_at_again_only_when_the_cards_change() {
        // No fingerprint (not Linux, or the cards could not be read): the list
        // has to be looked at as before.
        assert!(cards_changed(None, None));
        assert!(cards_changed(Some("0 [PCH ]"), None));
        // The same cards list the same devices; a card added or removed does not.
        assert!(!cards_changed(Some("0 [PCH ]"), Some("0 [PCH ]")));
        assert!(cards_changed(Some("0 [PCH ]"), Some("0 [PCH ]\n 1 [USB ]")));
    }

    /// The mixer live without an output device, and the button click's event: a sound
    /// holds a channel, and a stopped channel is let go at the next update.
    fn live_audio() -> Audio {
        let mut audio = Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            (Path::new("/nonexistent/sunrise-test-assets")).to_path_buf(),
        ));
        audio.engine.registry.add_file(
            "minecraft",
            br#"{"ui.button.click":{"sounds":["gui/button"]}}"#,
            &|_, _| true,
        );
        audio.engine.loaded = true;
        audio.backend.live = true;
        audio
    }

    #[test]
    fn a_mod_sound_waits_out_its_delay_plays_and_stops_for_good() {
        use mod_api::SoundState::*;
        let mut audio = live_audio();
        let ctx = TickContext::default();
        let click = mod_api::SoundPlay::ui("minecraft:ui.button.click");
        let sound = audio.play_mod_sound(&click, 2);
        let long = audio.play_mod_sound(&click, u32::MAX);
        assert_eq!(audio.mod_sound_state(sound), Waiting);
        audio.tick(false, &ctx);
        assert_eq!(
            audio.mod_sound_state(long),
            Waiting,
            "the long delay did not wrap"
        );
        assert_eq!(audio.mod_sound_state(sound), Waiting, "one tick of two");
        audio.tick(false, &ctx);
        assert_eq!(audio.mod_sound_state(sound), Playing);
        audio.stop_mod_sound(sound);
        audio.tick(false, &ctx);
        audio.tick(false, &ctx);
        assert_eq!(audio.mod_sound_state(sound), Stopped);
        // Stopped while it waits, it never starts; a looping one never repeats.
        let never = audio.play_mod_sound(&click.clone().looping(0), 3);
        audio.stop_mod_sound(never);
        assert_eq!(audio.mod_sound_state(never), Stopped);
        for _ in 0..5 {
            audio.tick(false, &ctx);
            assert_eq!(audio.mod_sound_state(never), Stopped);
        }
    }

    #[test]
    fn a_mod_sound_moves_and_changes_pitch_without_restarting() {
        use mod_api::SoundState::*;
        let mut audio = live_audio();
        let ctx = TickContext::default();
        let play = mod_api::SoundPlay::at(
            "minecraft:ui.button.click",
            mod_api::SoundSource::Blocks,
            [31_000_000.0, 2.0, 3.0],
        )
        .with_volume(-1.0)
        .with_pitch(3.0)
        .looping(-1);
        let sound = audio.play_mod_sound(&play, 0);
        let channel = audio.engine.channel(sound).unwrap();
        {
            let mixer = audio.backend.mixer.lock().unwrap();
            let voice = &mixer.channels[&channel];
            assert_eq!(voice.at, [31_000_000.0, 2.0, 3.0]);
            assert_eq!((voice.gain, voice.pitch), (0.0, 2.0));
            assert!(voice.looping);
        }
        audio.move_mod_sound(sound, [10.0, 32_000_000.0, 30.0]);
        audio.set_mod_sound_pitch(sound, -1.0);
        audio.tick(false, &ctx);
        assert_eq!(audio.engine.channel(sound), Some(channel), "not restarted");
        let mixer = audio.backend.mixer.lock().unwrap();
        let voice = &mixer.channels[&channel];
        assert_eq!(voice.at, [10.0, 32_000_000.0, 30.0]);
        assert_eq!(voice.pitch, 0.5);
        assert_eq!(audio.mod_sound_state(sound), Playing);
    }

    #[test]
    fn a_mod_sound_fades_to_its_volume_and_a_fade_to_zero_stops_it() {
        use mod_api::SoundState::*;
        let mut audio = live_audio();
        let ctx = TickContext::default();
        let sound = audio.play_mod_sound(&mod_api::SoundPlay::ui("minecraft:ui.button.click"), 0);
        assert_eq!(audio.mod_sound_state(sound), Playing);
        let volume = |audio: &Audio| audio.engine.instance(sound).map(|i| i.volume);
        audio.fade_mod_sound(sound, 0.5, 2);
        let mut volumes = Vec::new();
        for _ in 0..3 {
            audio.tick(false, &ctx);
            volumes.push(volume(&audio));
        }
        assert_eq!(
            volumes,
            [Some(0.75), Some(0.5), Some(0.5)],
            "two equal steps, then held"
        );
        // Paused, a fade waits.
        audio.fade_mod_sound(sound, 0.0, 2);
        audio.tick(true, &ctx);
        assert_eq!(volume(&audio), Some(0.5));
        audio.tick(false, &ctx);
        assert_eq!(volume(&audio), Some(0.25));
        assert_eq!(audio.mod_sound_state(sound), Playing);
        for _ in 0..4 {
            audio.tick(false, &ctx);
        }
        assert_eq!(audio.mod_sound_state(sound), Stopped, "faded out");
        // A fade at once: to a volume sets it, to 0 stops it.
        let other = audio.play_mod_sound(&mod_api::SoundPlay::ui("minecraft:ui.button.click"), 0);
        audio.fade_mod_sound(other, 0.3, 0);
        assert_eq!(volume_of(&audio, other), Some(0.3));
        audio.fade_mod_sound(other, -1.0, 0);
        for _ in 0..2 {
            audio.tick(false, &ctx);
        }
        assert_eq!(audio.mod_sound_state(other), Stopped);
    }

    /// A sound that repeats after a gap (`looping` with a delay) keeps its handle across the
    /// gap: once its sample ends it waits for the repeat, plays again, and still stops.
    #[test]
    fn a_repeating_mod_sound_waits_between_its_repeats_and_plays_again() {
        use mod_api::SoundState::*;
        let mut audio = live_audio();
        let ctx = TickContext::default();
        let click = mod_api::SoundPlay::ui("minecraft:ui.button.click").looping(2);
        let sound = audio.play_mod_sound(&click, 0);
        assert_eq!(audio.mod_sound_state(sound), Playing);
        // The sample ends on its own, as the mixer stops a channel that runs out.
        for channel in audio.backend.mixer.lock().unwrap().channels.values_mut() {
            channel.state = State::Stopped;
        }
        let mut states = Vec::new();
        for _ in 0..30 {
            audio.tick(false, &ctx);
            states.push(audio.mod_sound_state(sound));
        }
        assert_eq!(states[0], Waiting, "{states:?}");
        assert!(!states.contains(&Stopped), "{states:?}");
        assert_eq!(states.last(), Some(&Playing), "repeated: {states:?}");
        audio.stop_mod_sound(sound);
        audio.tick(false, &ctx);
        assert_eq!(audio.mod_sound_state(sound), Stopped);
    }

    #[test]
    fn a_fade_between_extreme_finite_volumes_stays_finite() {
        let mut audio = live_audio();
        let ctx = TickContext::default();
        let play = mod_api::SoundPlay::ui("minecraft:ui.button.click").with_volume(-f32::MAX);
        let sound = audio.play_mod_sound(&play, 0);
        audio.fade_mod_sound(sound, f32::MAX, 2);
        audio.tick(false, &ctx);
        assert_eq!(volume_of(&audio, sound), Some(0.0));
        audio.tick(false, &ctx);
        assert_eq!(volume_of(&audio, sound), Some(f32::MAX));
    }

    /// A mod sound at volume 0 starts silent and keeps its handle, so a fade raises it.
    #[test]
    fn a_silent_mod_sound_starts_and_a_fade_raises_it() {
        use mod_api::SoundState::*;
        let mut audio = live_audio();
        let ctx = TickContext::default();
        let silent = mod_api::SoundPlay::ui("minecraft:ui.button.click").with_volume(0.0);
        let sound = audio.play_mod_sound(&silent, 0);
        assert_eq!(audio.mod_sound_state(sound), Playing, "started silently");
        audio.fade_mod_sound(sound, 1.0, 2);
        audio.tick(false, &ctx);
        assert_eq!(volume_of(&audio, sound), Some(0.5));
        audio.tick(false, &ctx);
        assert_eq!(volume_of(&audio, sound), Some(1.0));
        assert_eq!(audio.mod_sound_state(sound), Playing);
    }

    fn volume_of(audio: &Audio, id: InstanceId) -> Option<f32> {
        audio.engine.instance(id).map(|i| i.volume)
    }
}
