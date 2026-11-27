use crate::game::ClientGame;
use sunrise_client::sound_engine::{
    InstanceId, PlayResult, PreparedPlay, SoundEngine, SoundInstance, Sounds,
};

/// The game's one way to play a sound (vanilla's `SoundEngine.play`, where NeoForge's.
macro_rules! play_audio_forwards {
    ($($(#[$doc:meta])* fn $name:ident => $audio:ident($($arg:ident: $ty:ty),*) $(-> $ret:ty)?;)*) => {
        impl ClientGame<'_> {
            $(
                $(#[$doc])*
                pub(in crate::game) fn $name(&mut self, $($arg: $ty),*) $(-> $ret)? {
                    let played = self.client.audio.0.$audio($($arg),*);
                    self.dispatch_play_sounds();
                    played
                }
            )*
        }
    };
}

play_audio_forwards! {
    /// `Audio::play`.
    fn play_audio => play(event: &str, at: Option<[f64; 3]>, volume: f32, pitch: f32);
    /// `Audio::play_distance_delayed`.
    fn play_audio_distance_delayed => play_distance_delayed(event: &str, at: [f64; 3], volume: f32, pitch: f32, camera: [f64; 3]);
    /// `Audio::play_unshared`.
    fn play_audio_unshared => play_unshared(event: &str, at: Option<[f64; 3]>, volume: f32, pitch: f32);
    /// `Audio::play_seeded`.
    fn play_audio_seeded => play_seeded(event: &str, at: Option<[f64; 3]>, volume: f32, pitch: f32, seed: i64);
    /// `Audio::play_on`.
    fn play_audio_on => play_on(source: usize, event: &str, at: Option<[f64; 3]>, volume: f32, pitch: f32);
    /// `Audio::play_on_seeded`.
    fn play_audio_on_seeded => play_on_seeded(source: usize, event: &str, at: Option<[f64; 3]>, volume: f32, pitch: f32, seed: i64);
    /// `Audio::play_entity_seeded`.
    fn play_audio_entity_seeded => play_entity_seeded(event: &str, source: usize, volume: f32, pitch: f32, seed: i64, entity: sunrise_client::sound_engine::SoundEntity);
    /// `Audio::play_button_click`.
    fn play_audio_button_click => play_button_click();
    /// `Audio::play_mod_sound`.
    fn play_audio_mod_sound => play_mod_sound(sound: &mod_api::SoundPlay, after: u32) -> InstanceId;
    /// `Audio::play_jukebox_song`.
    fn play_audio_jukebox_song => play_jukebox_song(event: &str, pos: [i32; 3]) -> InstanceId;
    /// `Audio::play_instance_after`: a delayed start, fired when it comes due.
    fn play_audio_instance_after => play_instance_after(instance: SoundInstance, ticks: i32);
}

impl ClientGame<'_> {
    /// `PlaySoundEvent` (vanilla's `SoundEngine.play`): fired with the game's `ClientHost`.
    pub(in crate::game) fn native_play_sound(&mut self, call: super::fire::PlaySoundCall<'_>) {
        self.fire_play_sound(call);
    }

    /// `SoundManager.play` of a new instance: the hook fires, then the answer plays at once.
    pub(in crate::game) fn play_audio_instance(
        &mut self,
        instance: SoundInstance,
        retain: bool,
    ) -> (InstanceId, PlayResult) {
        let id = self.client.audio.0.engine.add(instance, retain);
        (id, self.start_sound(id))
    }

    /// `SoundManager.play` of an instance the caller holds.
    pub(in crate::game) fn start_sound(&mut self, id: InstanceId) -> PlayResult {
        match self.client.audio.0.begin_play(id) {
            Some(prepared) => self.fire_start(prepared),
            None => self.client.audio.0.play_id(id),
        }
    }

    fn fire_start(&mut self, prepared: PreparedPlay) -> PlayResult {
        let mut sound = Some(prepared.original.clone());
        self.native_play_sound(super::fire::PlaySoundCall {
            instance: prepared.instance,
            starting: true,
            original: &prepared.original,
            sound: &mut sound,
        });
        self.client
            .audio
            .0
            .finish_prepared(prepared.instance, sound)
    }

    /// Fires `play_sound` for every start the audio made without the game (a helper the game.
    pub(in crate::game) fn dispatch_play_sounds(&mut self) {
        while let Some(prepared) = self.client.audio.0.next_deferred() {
            self.fire_start(prepared);
        }
    }

    /// Fires `play_sound` for each live refresh of a playing sound, once per audio tick: a.
    pub(in crate::game) fn dispatch_live_play_sounds(&mut self) {
        for prepared in self.client.audio.0.live_requests() {
            let mut sound = Some(prepared.original.clone());
            self.native_play_sound(super::fire::PlaySoundCall {
                instance: prepared.instance,
                starting: false,
                original: &prepared.original,
                sound: &mut sound,
            });
            self.client
                .audio
                .0
                .set_live_answer(prepared.instance, sound);
        }
    }
}

/// The sound manager the music, its layers and the ambience play through: `play_sound` fires.
pub(in crate::game) struct HookedSounds<'g, 'a>(pub(in crate::game) &'g mut ClientGame<'a>);

impl Sounds for HookedSounds<'_, '_> {
    fn engine(&mut self) -> &mut SoundEngine {
        &mut self.0.client.audio.0.engine
    }
    fn play_instance(&mut self, instance: SoundInstance, retain: bool) -> (InstanceId, PlayResult) {
        self.0.play_audio_instance(instance, retain)
    }
    fn play_id(&mut self, id: InstanceId) -> PlayResult {
        self.0.start_sound(id)
    }
    fn stop_instance(&mut self, id: InstanceId) {
        self.0.client.audio.0.stop_instance(id);
    }
    fn update_category_volume(&mut self, source: usize, gain: f32) {
        self.0.client.audio.0.update_category_volume(source, gain);
    }
}
