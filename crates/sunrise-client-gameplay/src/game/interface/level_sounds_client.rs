use crate::game::ClientGame;
use mod_api::{Identifier, SoundSource};
use std::borrow::Cow;
use sunrise_gameplay_common::game::level_sounds_common::{HeardSound, LevelSound, listened};
use sunrise_gameplay_common::game::native_hooks::level_sounds::GameHooks as _;

impl ClientGame<'_> {
    /// `ClientLevel.playSeededSound`: ask this client's mods before playback.
    /// A packet reaches this independently of the server's level event.
    pub(in crate::game::client_side) fn play_client_level_sound(
        &mut self,
        mut sound: LevelSound<'_>,
        seed: i64,
    ) {
        assert!(
            !self.is_logical_server(),
            "play_client_level_sound called on Server role"
        );
        let entity = match sound.emitter {
            Some(id) => {
                let Some(state) = self.remote_sound_entity_state(id as i32) else {
                    return;
                };
                sound.at = state.position;
                let entity = sunrise_client::sound_engine::SoundEntity::new(
                    id,
                    state.position,
                    state.silent,
                );
                entity.update(state);
                Some(entity)
            }
            None => None,
        };
        let at = sound.at;
        let Some(heard) = self.client_ask_level_sound(self.common.dim.id(), sound) else {
            return;
        };
        match entity {
            Some(entity) => self.play_audio_entity_seeded(
                &heard.name,
                heard.source.index(),
                heard.volume,
                heard.pitch,
                seed,
                entity,
            ),
            None => self.play_audio_on_seeded(
                heard.source.index(),
                &heard.name,
                Some(at),
                heard.volume,
                heard.pitch,
                seed,
            ),
        }
    }
    /// A client-local level request without a packet seed. LocalPlayer.playSound
    /// asks AtEntity but still creates a positional, not entity-bound, instance.
    pub(in crate::game) fn play_client_local_sound(&mut self, sound: LevelSound<'_>) {
        assert!(
            !self.is_logical_server(),
            "play_client_local_sound called on Server role"
        );
        let seed = self.client.audio.0.next_sound_seed();
        let at = sound.at;
        if let Some(heard) = self.client_ask_level_sound(self.common.dim.id(), sound) {
            self.play_audio_on_seeded(
                heard.source.index(),
                &heard.name,
                Some(at),
                heard.volume,
                heard.pitch,
                seed,
            );
        }
    }
    /// Client-side `PlayLevelSoundEvent` check (no server `pending_native_sounds`).
    pub(in crate::game::client_side) fn client_ask_level_sound<'a>(
        &mut self,
        dimension: &str,
        sound: LevelSound<'a>,
    ) -> Option<HeardSound<'a>> {
        let LevelSound {
            name,
            at,
            volume,
            pitch,
            source,
            emitter,
        } = sound;
        let asked = HeardSound {
            name: Cow::Borrowed(name),
            source,
            volume,
            pitch,
        };
        if !listened() || self.common.creatures.asking_level_sound {
            return Some(asked);
        }
        let Ok(mut id) = Identifier::parse(name) else {
            return Some(asked);
        };
        let (mut source_now, mut volume_now, mut pitch_now, mut cancel) =
            (source, volume, pitch, false);
        let entity = emitter.and_then(|id| self.entity(id));
        // A missing client emitter must not become an AtPosition request.
        if emitter.is_some() && entity.is_none() {
            return None;
        }
        struct AskGuard<'g, 'a> {
            game: &'g mut crate::game::ClientGame<'a>,
        }
        impl Drop for AskGuard<'_, '_> {
            fn drop(&mut self) {
                self.game.creatures.asking_level_sound = false;
            }
        }
        self.common.creatures.asking_level_sound = true;
        let guard = AskGuard { game: self };
        match entity {
            Some(entity) => guard.game.native_play_level_sound_at_entity(
                mod_api::__private::PlayLevelSoundAtEntityCall {
                    entity,
                    sound: &mut id,
                    source: &mut source_now,
                    volume: &mut volume_now,
                    pitch: &mut pitch_now,
                    original_volume: volume,
                    original_pitch: pitch,
                    cancel: &mut cancel,
                },
            ),
            None => {
                let level = guard.game.level(dimension)?;
                guard.game.native_play_level_sound_at_position(
                    mod_api::__private::PlayLevelSoundAtPositionCall {
                        level,
                        position: at,
                        sound: &mut id,
                        source: &mut source_now,
                        volume: &mut volume_now,
                        pitch: &mut pitch_now,
                        original_volume: volume,
                        original_pitch: pitch,
                        cancel: &mut cancel,
                    },
                );
            }
        }
        drop(guard);
        if cancel {
            return None;
        }
        Some(HeardSound {
            name: Cow::Owned(id.to_string()),
            source: source_now,
            volume: volume_now,
            pitch: pitch_now,
        })
    }
    /// `LocalPlayer.playSound`, including inherited step, fall and consume sounds.
    pub(in crate::game) fn play_client_player_sound(
        &mut self,
        name: &str,
        volume: f32,
        pitch: f32,
    ) {
        let Some(player) = self.common.player.as_ref() else {
            return;
        };
        let at = [player.position.x, player.position.y, player.position.z];
        let Some(remote) = self.client.remote.as_ref() else {
            return;
        };
        let mut sound = LevelSound::new(name, at, volume, pitch).in_source(SoundSource::Players);
        sound.emitter = Some(remote.entity_id as u32);
        self.play_client_local_sound(sound);
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
