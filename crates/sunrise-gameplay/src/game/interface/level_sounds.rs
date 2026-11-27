//! Level sound requests on each logical side: the mods' `PlayLevelSoundEvent`,
//! then server broadcast or client playback.

use std::borrow::Cow;

use mod_api::{Identifier, SoundSource};
use sunrise_gameplay_common::game::level_sounds_common::{HeardSound, LevelSound, listened};

use crate::game::ServerGame;
use sunrise_gameplay_common::game::native_hooks::level_sounds::GameHooks as _;

/// A sound bound to an entity (`level.playSound(except, entity, ...)`),
/// waiting for the end of the tick to be asked of the mods and sent.
pub struct BoundSound {
    pub name: String,
    pub at: [f64; 3],
    pub volume: f32,
    pub pitch: f32,
    pub source: SoundSource,
    pub emitter: u32,
    /// The connection of the player vanilla leaves out, who played it already.
    pub except: Option<u32>,
}

impl ServerGame<'_> {
    /// `ServerLevel.playSeededSound`'s entity overload, `playSound(except,
    /// emitter, ...)`: `sound` plays here and, while the level's sounds are
    /// captured for others, joins the tick's bound sounds. `lan_broadcast_effects` asks
    /// them of the mods (`play_level_sound_at_entity`) after the tick's
    /// other sounds, and sends them to the players around but the one on
    /// connection `except`.
    pub(in crate::game) fn play_bound_sound(
        &mut self,
        sound: LevelSound<'_>,
        emitter: u32,
        except: Option<u32>,
    ) {
        // Kept while the level's sounds are captured for others, as
        // `Audio.play` keeps the rest.
        if self.server.audio_capture_active {
            self.server.bound_sounds.push(BoundSound {
                name: sound.name.to_owned(),
                at: sound.at,
                volume: sound.volume,
                pitch: sound.pitch,
                source: sound.source,
                emitter,
                except,
            });
        }
    }

    /// The tick's bound sounds, in the order they played, for the players.
    pub(in crate::game::server_side) fn send_bound_sounds(&mut self) {
        for s in std::mem::take(&mut self.server.bound_sounds) {
            let sound = LevelSound {
                name: &s.name,
                at: s.at,
                volume: s.volume,
                pitch: s.pitch,
                source: s.source,
                emitter: Some(s.emitter),
            };
            self.send_level_sound(sound, s.except);
        }
    }

    /// `Level.playSound`'s packet for the players near the sound's position in
    /// the level in place, but the player on connection `except`, once the
    /// mods allowed it.
    pub(in crate::game) fn send_level_sound(&mut self, sound: LevelSound<'_>, except: Option<u32>) {
        let at = sound.at;
        let dimension = self.common.dim.id();
        let Some(heard) = self.ask_level_sound(dimension, sound) else {
            return;
        };
        if let Some(host) = self.server.lan.as_ref() {
            host.hub.sound(
                &heard.name,
                at,
                heard.volume,
                heard.pitch,
                heard.source as i32,
                except,
            );
        }
    }

    /// A synchronous level sound, not captured again at the end of the tick.
    pub(in crate::game::server_side) fn play_immediate_level_sound(
        &mut self,
        sound: LevelSound<'_>,
    ) {
        self.send_level_sound(sound, None);
    }

    /// `ServerLevel.playSeededSound`'s `EventHooks.onPlaySoundAtEntity` or
    /// `onPlaySoundAtPosition`: `play_level_sound_at_entity` when the sound's
    /// emitter is an entity a mod can name, `play_level_sound_at_position`
    /// otherwise. None when a mod cancels it or leaves it unplayable (a volume
    /// or pitch that isn't finite and at least 0). With no listener, the sound
    /// as asked, borrowed.
    pub(in crate::game) fn ask_level_sound<'a>(
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
        // The mods see a namespaced id (`minecraft:entity.pig.ambient`).
        let Ok(mut id) = Identifier::parse(name) else {
            return Some(asked);
        };
        let (mut source_now, mut volume_now, mut pitch_now, mut cancel) =
            (source, volume, pitch, false);
        let entity = emitter.and_then(|id| self.entity(id));
        // A missing client emitter must not become an AtPosition request.
        if emitter.is_some() && entity.is_none() && !self.common.is_logical_server() {
            return None;
        }
        struct AskGuard<'g, 'a> {
            game: &'g mut crate::game::ServerGame<'a>,
            queued_before: usize,
        }
        impl Drop for AskGuard<'_, '_> {
            fn drop(&mut self) {
                self.game.creatures.asking_level_sound = false;
                if let Some(added) = self
                    .game
                    .server
                    .pending_native_sounds
                    .get_mut(self.queued_before..)
                {
                    for sound in added {
                        sound.skip_hooks = true;
                    }
                }
            }
        }
        let queued_before = self.server.pending_native_sounds.len();
        self.common.creatures.asking_level_sound = true;
        let guard = AskGuard {
            game: self,
            queued_before,
        };
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
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
