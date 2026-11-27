//! Sounds to one player and sounds that follow an entity, for native server
//! mods, through a `PlayerRef` and an `EntityRef` (docs/modding/src/how-to/native-mods.md):
//! `StopSoundCommand`'s packet, a `ClientboundSoundPacket` for one player
//! (`PlaySoundCommand`) and `Level.playSound(null, entity, ...)`, which
//! sends a `ClientboundSoundEntityPacket` to the players in range. They
//! return false for an unavailable target or malformed event key. Recipients resolve
//! the event in their sound resources; packet delivery does not imply playback.
use super::{commands::Target, journal_level_effects::PendingWrite};
use mod_api::{EntityRef, PlayerRef, SoundSource};
use sunrise_core::random::RandomSource;
use sunrise_gameplay_common::game::{native_hooks::sound_id, transactions::After};

impl crate::game::ServerGame<'_> {
    /// Where `player` is reached now: the window's own player or a guest.
    pub(in crate::game::server_side) fn native_player_sound_target(
        &self,
        player: PlayerRef,
    ) -> Option<Target> {
        if self.player(player.uuid()) != Some(player) {
            return None;
        }
        match self.native_entity_target(player.entity())? {
            t @ (Target::Player | Target::Guest(_)) => Some(t),
            _ => None,
        }
    }
    /// `StopSoundCommand` for one player: their client stops the playing
    /// sounds that match `sound` (an event id) and `source`; `None` matches
    /// any. True means accepted, not delivered.
    pub fn player_stop_sound(
        &mut self,
        player: PlayerRef,
        sound: Option<&str>,
        source: Option<SoundSource>,
    ) -> bool {
        let Some(target) = self.native_player_sound_target(player) else {
            return false;
        };
        if sound.is_some_and(|s| !sound_id(s)) {
            return false;
        }
        let source = source.map(SoundSource::index);
        if !self.has_level_transaction() {
            self.stop_sound_for(&[target], sound, source);
        } else {
            self.after_commit(After::LevelWrite(Box::new(PendingWrite::PlayerStopSound {
                level: self.common.dim,
                player,
                sound: sound.map(str::to_owned),
                source,
            })));
        }
        true
    }
    /// `PlaySoundCommand` for one player: plays sound event `sound` at the
    /// player, for them only, under `source`'s volume slider. The seed is the
    /// level random's `nextLong`, drawn when the sound publishes. True means
    /// accepted, not delivered.
    pub fn player_play_sound(
        &mut self,
        player: PlayerRef,
        sound: &str,
        source: SoundSource,
        volume: f32,
        pitch: f32,
    ) -> bool {
        let Some(target) = self.native_player_sound_target(player) else {
            return false;
        };
        let Some(at) = self.target_position(target) else {
            return false;
        };
        if !sound_id(sound) {
            return false;
        }
        if !self.has_level_transaction() {
            let seed = self.common.world.random.next_long();
            self.play_sound_to(target, sound, at, volume, pitch, source.index(), seed);
        } else {
            self.after_commit(After::LevelWrite(Box::new(PendingWrite::PlayerSound {
                level: self.common.dim,
                player,
                sound: sound.to_owned(),
                at,
                source: source.index(),
                volume,
                pitch,
            })));
        }
        true
    }
    /// `Level.playSound(null, entity, sound, source, volume, pitch)`: plays
    /// sound event `sound` bound to the entity, so that it follows it, for
    /// the players within range (16 blocks, times the volume above 1) of
    /// where it is now, the entity's own player too. True means queued, not
    /// heard; false for an entity outside the level in place.
    pub fn entity_play_bound_sound(
        &mut self,
        entity: EntityRef,
        sound: &str,
        source: SoundSource,
        volume: f32,
        pitch: f32,
    ) -> bool {
        self.native_bound_sound(entity, sound, source, volume, pitch, None, None)
    }
    /// `ServerLevel.playSeededSound(except, entity, sound, source, volume, pitch, seed)`:
    /// [`Self::entity_play_bound_sound`] leaving out `except`'s client, with the caller's seed.
    pub(in crate::game) fn entity_play_seeded_bound_sound(
        &mut self,
        entity: EntityRef,
        sound: &str,
        source: SoundSource,
        volume: f32,
        pitch: f32,
        except: Option<PlayerRef>,
        seed: i64,
    ) -> bool {
        let Some(except) = self.native_sound_except(except) else {
            return false;
        };
        self.native_bound_sound(entity, sound, source, volume, pitch, except, Some(seed))
    }
    /// A sound bound to `entity` from where it is now, leaving out connection `except`, with
    /// `seed` as [`NativeSoundRequest`](super::native_level_effects::NativeSoundRequest) says.
    fn native_bound_sound(
        &mut self,
        entity: EntityRef,
        sound: &str,
        source: SoundSource,
        volume: f32,
        pitch: f32,
        except: Option<u32>,
        seed: Option<i64>,
    ) -> bool {
        let Some(target) = self.native_entity_target(entity) else {
            return false;
        };
        let (Some(level), Some(at), Ok(id)) = (
            self.entity_level(entity),
            self.target_position(target),
            i32::try_from(entity.id()),
        ) else {
            return false;
        };
        self.native_queue_sound(super::native_level_effects::NativeSoundRequest {
            level,
            sound,
            at,
            source,
            volume,
            pitch,
            except,
            entity: Some(id),
            inline_range: None,
            seed,
        })
    }
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
