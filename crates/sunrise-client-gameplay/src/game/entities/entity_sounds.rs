//! Client-side entity-bound level sound playback and entity sound state lookup.
use crate::game::ClientGame;
#[cfg(feature = "client")]
use sunrise_game_data::sound_source::SoundEntityState;
use sunrise_gameplay_common::game::{
    entity_sounds_common::saved_silent, level_sounds_common::LevelSound,
};

#[cfg(feature = "client")]
impl ClientGame<'_> {
    /// `Entity.isSilent` for the player this window plays.
    #[allow(dead_code)]
    pub(in crate::game::client_side) fn player_silent(&self) -> bool {
        saved_silent(self.common.level.player_extra.as_ref())
    }

    pub(in crate::game) fn client_play_player_bound_sound(
        &mut self,
        event: &str,
        source: usize,
        volume: f32,
        pitch: f32,
        _except_actor: bool,
    ) {
        let id = self
            .client
            .remote
            .as_ref()
            .map_or(0, |r| r.entity_id as u32);
        let mut sound = LevelSound::new(event, [0.0; 3], volume, pitch)
            .in_source(mod_api::SoundSource::ALL[source]);
        sound.emitter = Some(id);
        let seed = self.client.audio.0.next_sound_seed();
        self.play_client_level_sound(sound, seed);
    }

    pub(in crate::game::client_side) fn sound_entity_state(
        &mut self,
        id: u32,
    ) -> Option<SoundEntityState> {
        if self.client.remote.is_some() {
            self.remote_sound_entity_state(id as i32)
        } else {
            None
        }
    }
}
