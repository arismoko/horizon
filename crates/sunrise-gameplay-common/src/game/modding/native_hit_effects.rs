//! The level sound queue (`Vec<NativeSound>`, a `SoundQueue`) as the sink of a hit's sounds (`HitHost::play_sound_at`,
//! `play_sound_from`): the same `lan_sounds` queue, sound-seed stream and validation as a
//! `LevelRef`'s sounds, reached through the hit's [`HitEffects`](sunrise_client::health::HitEffects).
use super::creatures::NativeSound;
use sunrise_sim::health::HitSound;

impl From<HitSound<'_>> for NativeSound {
    fn from(sound: HitSound<'_>) -> Self {
        Self {
            dimension: sound.dimension,
            sound: sound.sound.to_owned(),
            inline_range: None,
            at: sound.at,
            source: sound.source,
            volume: sound.volume,
            pitch: sound.pitch,
            seed: sound.seed,
            except: None,
            entity: sound.emitter,
            skip_hooks: false,
        }
    }
}
