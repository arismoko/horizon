//! Sounds and particles in a level for native server mods, through a
//! `LevelRef` (docs/modding/src/how-to/native-mods.md, "Entity and level handles", #7387).
//! Every player in range hears or sees them, as `ServerLevel.playSound` and
//! `sendParticles` send them. They return false on a client, for a handle of
//! an earlier open, or for arguments the network can't carry.
use mod_api::{LevelRef, PlayerRef, PlayerWorldRefusal, SoundSource};
use sunrise_gameplay_common::game::api_values::TryIntoEngine;
use sunrise_gameplay_common::game::creatures::NativeSound;
use sunrise_gameplay_common::game::journal_level_effects::PendingWrite;
use sunrise_gameplay_common::game::level_sounds_common::LevelSound;
use sunrise_gameplay_common::game::native_hooks::{mod_particles_ok, sound_id};
use sunrise_level::particle_options::ParticleOptions;
use sunrise_sim::lan::server_effects::ServerQueuedParticle;
use sunrise_world::registry::BlockStateId;

/// A level or entity-bound native sound request, before dispatch or queueing.
pub struct NativeSoundRequest<'a> {
    pub level: LevelRef,
    pub sound: &'a str,
    pub at: [f64; 3],
    pub source: SoundSource,
    pub volume: f32,
    pub pitch: f32,
    pub except: Option<u32>,
    pub entity: Option<i32>,
    /// A direct holder, with or without a fixed broadcast radius.
    pub inline_range: Option<Option<f32>>,
    /// The caller's seed (`ServerLevel.playSeededSound`), or `None` to draw the next one from
    /// the level's sound-seed stream when the sound is published (`Level.playSound`).
    pub seed: Option<i64>,
}

impl crate::game::ServerGame<'_> {
    /// The level's dimension id, for a handle of this open on the server.
    fn native_level_dimension(&self, level: LevelRef) -> Option<&'static str> {
        self.native_level_world(level)?;
        Some(level.dimension_id())
    }
    /// Plays sound event `sound` at `at` for the players in this level
    /// within range (16 blocks, times the volume above 1), as
    /// `ServerLevel.playSound(except, ...)` does, under `source`'s volume
    /// slider, leaving out `except`'s client (a fake player has none, so it
    /// leaves out no one; a stale one queues nothing). Registry entries are sent by ID, and
    /// other valid keys by name. True means queued, not heard.
    pub fn level_play_sound(
        &mut self,
        level: LevelRef,
        sound: &str,
        at: [f64; 3],
        source: SoundSource,
        volume: f32,
        pitch: f32,
        except: Option<PlayerRef>,
    ) -> bool {
        self.native_level_sound(level, sound, at, source, volume, pitch, except, None)
    }
    /// [`Self::level_play_sound`] with the caller's seed (`ServerLevel.playSeededSound`): the
    /// level's sound-seed stream does not move.
    pub(in crate::game) fn level_play_seeded_sound(
        &mut self,
        level: LevelRef,
        sound: &str,
        at: [f64; 3],
        source: SoundSource,
        volume: f32,
        pitch: f32,
        except: Option<PlayerRef>,
        seed: i64,
    ) -> bool {
        self.native_level_sound(level, sound, at, source, volume, pitch, except, Some(seed))
    }
    /// The next seed of the stream the level's sounds draw from
    /// (`Level.soundSeedGenerator.nextLong()`), drawn now without playing a sound: the same
    /// draw a published [`Self::level_play_sound`] makes. None where that call would refuse
    /// the level or send nothing.
    pub(in crate::game) fn level_next_sound_seed(&mut self, level: LevelRef) -> Option<i64> {
        self.native_level_dimension(level)?;
        Some(self.server.lan.as_ref()?.hub.next_sound_seed())
    }
    /// The connection a level sound leaves out for `except` (`Level.playSound(except, ...)`):
    /// `Some(None)` for no one, and for a fake player, who has no client to leave out; `None`
    /// refuses the sound for a stale or client-side player.
    pub(in crate::game::server_side) fn native_sound_except(
        &self,
        except: Option<PlayerRef>,
    ) -> Option<Option<u32>> {
        let Some(player) = except else {
            return Some(None);
        };
        match self.player_connection(player) {
            Ok(conn) => Some(Some(conn)),
            Err(PlayerWorldRefusal::NoConnection) => Some(None),
            Err(_) => None,
        }
    }
    /// A positional level sound leaving out `except`, with `seed` as [`NativeSoundRequest`] says.
    fn native_level_sound(
        &mut self,
        level: LevelRef,
        sound: &str,
        at: [f64; 3],
        source: SoundSource,
        volume: f32,
        pitch: f32,
        except: Option<PlayerRef>,
        seed: Option<i64>,
    ) -> bool {
        let Some(except) = self.native_sound_except(except) else {
            return false;
        };
        self.native_queue_sound(NativeSoundRequest {
            level,
            sound,
            at,
            source,
            volume,
            pitch,
            except,
            entity: None,
            inline_range: None,
            seed,
        })
    }
    /// Accept a level or entity-bound sound (`Level.playSound`), leaving out `except` and
    /// optionally binding to `entity`. In a transaction, ask the hooks at the call and defer only
    /// the resolved effect to root commit (drawing its seed on publish unless the request gave
    /// one) and discard on abort; outside one, queue it with its seed.
    pub fn native_queue_sound(&mut self, request: NativeSoundRequest<'_>) -> bool {
        let NativeSoundRequest {
            level,
            sound,
            at,
            source,
            volume,
            pitch,
            except,
            entity,
            inline_range,
            seed,
        } = request;
        let Some(dimension) = self.native_level_dimension(level) else {
            return false;
        };
        if self.server.lan.is_none() || !sound_id(sound) {
            return false;
        }
        self.emit_level_effect(PendingWrite::Sound {
            sound: NativeSound {
                dimension,
                sound: sound.to_owned(),
                inline_range,
                at,
                source: source.index() as i32,
                volume,
                pitch,
                seed: 0,
                except,
                entity,
                skip_hooks: false,
            },
            seed,
        });
        true
    }

    /// Queues a gameplay sound in its own dimension, even when another level is in place.
    pub(in crate::game) fn queue_level_sound(
        &mut self,
        dimension: &'static str,
        sound: LevelSound<'_>,
        except: Option<u32>,
    ) -> bool {
        if self.server.lan.is_none() {
            return false;
        }
        self.emit_level_effect(PendingWrite::Sound {
            sound: NativeSound {
                dimension,
                sound: sound.name.to_owned(),
                inline_range: None,
                at: sound.at,
                source: sound.source.index() as i32,
                volume: sound.volume,
                pitch: sound.pitch,
                seed: 0,
                except,
                entity: sound.emitter.map(|id| id as i32),
                skip_hooks: false,
            },
            seed: None,
        });
        true
    }
    /// Sends `count` particles around `at` to the players in this level
    /// within 32 blocks, as `ServerLevel.sendParticles` does: each offset is
    /// a Gaussian times `spread`, and `speed` scales their motion (a zero
    /// count sends one particle moving along `spread`). Block particle types
    /// need `state`; item particles need a validated stack; trail and vibration aren't supported.
    /// True means queued, not drawn.
    pub fn level_send_particles(
        &mut self,
        level: LevelRef,
        options: &ParticleOptions,
        state: Option<BlockStateId>,
        at: [f64; 3],
        spread: [f64; 3],
        speed: f64,
        count: i32,
    ) -> bool {
        let Some(dimension) = self.native_level_dimension(level) else {
            return false;
        };
        self.queue_level_particles(
            dimension,
            options,
            state,
            std::slice::from_ref(&at),
            spread,
            speed,
            count,
        )
    }

    pub(in crate::game) fn level_send_particles_batch(
        &mut self,
        level: LevelRef,
        options: &mod_api::ParticleOptions,
        state: Option<mod_api::BlockState>,
        positions: &[[f64; 3]],
        spread: [f64; 3],
        speed: f64,
        count: i32,
    ) -> bool {
        let Some(dimension) = self.native_level_dimension(level) else {
            return false;
        };
        if positions.is_empty() {
            return true;
        }
        let items = self.items();
        let (Ok(options), Ok(state)) =
            (options.try_into_engine(items), state.try_into_engine(items))
        else {
            return false;
        };
        self.queue_level_particles(dimension, &options, state, positions, spread, speed, count)
    }

    fn queue_level_particles(
        &mut self,
        dimension: &'static str,
        options: &ParticleOptions,
        state: Option<BlockStateId>,
        positions: &[[f64; 3]],
        spread: [f64; 3],
        speed: f64,
        count: i32,
    ) -> bool {
        let registry = self.common.pack.assets.registry;
        let state_ok = |id| registry.state(id).is_some();
        if !mod_particles_ok(options, state, state_ok, positions, spread, speed) {
            return false;
        }
        let Some(particle) =
            sunrise_gameplay_common::game::lan_particle_data::server_particle_from_options(
                options, state,
            )
        else {
            return false;
        };
        for &at in positions {
            self.emit_level_effect(PendingWrite::Particles(ServerQueuedParticle {
                dimension,
                particle: particle.clone(),
                at,
                spread,
                speed: [speed; 3],
                count,
                randomization: 0,
                except: None,
            }));
        }
        true
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
