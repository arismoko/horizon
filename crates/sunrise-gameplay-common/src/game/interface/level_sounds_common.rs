use mod_api::{HookId, SoundSource};
use std::borrow::Cow;

/// A sound the server's level is about to play, as vanilla asked for it.
pub struct LevelSound<'a> {
    pub name: &'a str,
    pub at: [f64; 3],
    pub volume: f32,
    pub pitch: f32,
    pub source: SoundSource,
    /// The entity vanilla binds the sound to (`playSound(except, sourceEntity, ...)`);
    /// none for a sound at a position, as `Entity.playSound`'s.
    pub emitter: Option<u32>,
}

impl<'a> LevelSound<'a> {
    /// A sound at `at` in the source its event name belongs to.
    pub fn new(name: &'a str, at: [f64; 3], volume: f32, pitch: f32) -> Self {
        let source = SoundSource::ALL
            .get(sunrise_level::sim_settings::sound_source(name))
            .copied()
            .unwrap_or(SoundSource::Master);
        Self {
            name,
            at,
            volume,
            pitch,
            source,
            emitter: None,
        }
    }

    pub fn in_source(self, source: SoundSource) -> Self {
        Self { source, ..self }
    }
}

/// A sound after the mods: what goes in the packet.
pub struct HeardSound<'a> {
    pub name: Cow<'a, str>,
    pub source: SoundSource,
    pub volume: f32,
    pub pitch: f32,
}

/// Whether a mod listens to either level sound hook.
pub fn listened() -> bool {
    mod_api::__private::listened(HookId::PlayLevelSoundAtPosition)
        || mod_api::__private::listened(HookId::PlayLevelSoundAtEntity)
}
