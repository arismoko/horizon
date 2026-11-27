//! Portable captured level sound record and sound-category re-exports.
pub use sunrise_game_data::sound_source::{
    AMBIENT, BLOCKS, MASTER, MUSIC, NEUTRAL, PLAYERS, RECORDS, SOUND_SOURCES, ServerSound,
    SoundEntityState, UI, WEATHER, sound_source_for_event,
};

/// A level sound retained for the network broadcaster, in playback order.
#[derive(Clone, Debug)]
pub struct CapturedSound {
    pub event: String,
    pub position: [f64; 3],
    pub volume: f32,
    pub pitch: f32,
    pub source: usize,
    pub entity: Option<u32>,
    pub except_entity: Option<u32>,
    /// The seed the sound's instance takes (`SoundInstance`'s random).
    pub seed: i64,
    /// How many particle requests this thread's particle captures had taken when this sound
    /// was captured (`particles::requested_count`): a test reads the two captures as one
    /// ordered stream.
    pub particles_before: u64,
}
