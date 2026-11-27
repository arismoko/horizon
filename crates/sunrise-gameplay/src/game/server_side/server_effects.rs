//! Server-owned sound queue, capture lifecycle, and portable effect helpers.
use crate::game::ServerGame;
use sunrise_game_data::sound_source::ServerSound;

impl ServerGame<'_> {
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_positional_sound_count(&self, event: &str) -> Option<&u32> {
        self.server.test_positional_sound_counts.get(event)
    }

    #[cfg(any(test, feature = "test-support"))]
    #[inline(always)]
    fn record_test_positional_sound(&mut self, event: &str) {
        *self
            .server
            .test_positional_sound_counts
            .entry(event.to_owned())
            .or_default() += 1;
    }

    #[inline(always)]
    pub fn start_audio_capture(&mut self) {
        assert!(
            self.is_logical_server(),
            "start_audio_capture called on Client role"
        );
        self.server.audio_capture_active = true;
        self.server.pending_sounds.clear();
    }

    #[inline(always)]
    pub fn stop_audio_capture(&mut self) {
        self.server.audio_capture_active = false;
        self.server.pending_sounds.clear();
    }

    #[allow(dead_code)]
    #[inline(always)]
    pub(in crate::game) fn audio_capture_active(&self) -> bool {
        self.server.audio_capture_active
    }

    #[allow(dead_code)]
    #[inline(always)]
    pub(in crate::game) fn audio_capture_len(&self) -> usize {
        self.server.pending_sounds.len()
    }

    pub fn server_play_sound(&mut self, event: &str, at: [f64; 3], volume: f32, pitch: f32) {
        assert!(
            self.is_logical_server(),
            "server_play_sound called on Client role"
        );
        #[cfg(any(test, feature = "test-support"))]
        self.record_test_positional_sound(event);
        if self.server.audio_capture_active {
            self.server
                .pending_sounds
                .push(ServerSound::new(event, at, volume, pitch));
        }
    }

    pub(in crate::game) fn server_play_placement_sound(
        &mut self,
        event: &str,
        at: [f64; 3],
        volume: f32,
        pitch: f32,
        except_entity: Option<u32>,
    ) {
        #[cfg(any(test, feature = "test-support"))]
        self.record_test_positional_sound(event);
        if self.server.audio_capture_active {
            self.server.pending_sounds.push(ServerSound {
                source: mod_api::SoundSource::Blocks.index(),
                except_entity,
                ..ServerSound::new(event, at, volume, pitch)
            });
        }
    }

    pub fn server_play_sound_on(
        &mut self,
        source: usize,
        event: &str,
        at: [f64; 3],
        volume: f32,
        pitch: f32,
    ) {
        self.capture_sound_on(source, event, at, volume, pitch, None);
    }

    pub(in crate::game::server_side) fn server_play_sound_on_seeded(
        &mut self,
        source: usize,
        event: &str,
        at: [f64; 3],
        volume: f32,
        pitch: f32,
        seed: i64,
    ) {
        self.capture_sound_on(source, event, at, volume, pitch, Some(seed));
    }

    fn capture_sound_on(
        &mut self,
        source: usize,
        event: &str,
        at: [f64; 3],
        volume: f32,
        pitch: f32,
        seed: Option<i64>,
    ) {
        assert!(
            self.is_logical_server(),
            "server_play_sound_on called on Client role"
        );
        #[cfg(any(test, feature = "test-support"))]
        self.record_test_positional_sound(event);
        if self.server.audio_capture_active {
            self.server.pending_sounds.push(ServerSound {
                event: event.to_owned(),
                source,
                position: at,
                volume,
                pitch,
                seed,
                entity: None,
                except_entity: None,
            });
        }
    }

    pub(in crate::game::server_side) fn server_play_sound_seeded(
        &mut self,
        event: &str,
        at: [f64; 3],
        volume: f32,
        pitch: f32,
        seed: i64,
    ) {
        assert!(
            self.is_logical_server(),
            "server_play_sound_seeded called on Client role"
        );
        #[cfg(any(test, feature = "test-support"))]
        self.record_test_positional_sound(event);
        if self.server.audio_capture_active {
            self.server.pending_sounds.push(ServerSound {
                seed: Some(seed),
                ..ServerSound::new(event, at, volume, pitch)
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::game) fn server_play_bound_sound(
        &mut self,
        event: &str,
        source: usize,
        at: [f64; 3],
        volume: f32,
        pitch: f32,
        emitter: u32,
        silent: bool,
        except: Option<u32>,
    ) {
        assert!(
            self.is_logical_server(),
            "server_play_bound_sound called on Client role"
        );
        if silent {
            return;
        }
        if self.server.audio_capture_active {
            self.server.pending_sounds.push(ServerSound {
                event: event.to_owned(),
                source,
                position: at,
                volume,
                pitch,
                seed: None,
                entity: Some(emitter),
                except_entity: except,
            });
        }
    }

    /// `Level.playSound(except, mob, ...)`: the category and actor exclusion
    /// belong to the call site, not to the event's name.
    pub(in crate::game) fn play_mob_bound_sound(
        &mut self,
        index: usize,
        event: &str,
        source: usize,
        volume: f32,
        pitch: f32,
        except_actor: bool,
    ) {
        let mob = &self.common.mobs[index];
        let (at, id, silent) = (
            [mob.position.x, mob.position.y, mob.position.z],
            mob.id,
            mob.silent,
        );
        let except = except_actor.then(|| self.native_acting_entity());
        self.server_play_bound_sound(event, source, at, volume, pitch, id, silent, except);
    }

    pub(in crate::game) fn server_particle_at(
        &mut self,
        particle: sunrise_sim::lan::server_effects::ServerParticle,
        at: [f64; 3],
        velocity: [f64; 3],
    ) {
        assert!(
            self.is_logical_server(),
            "server_particle_at called on Client role"
        );
        let speed = if velocity == [0.0; 3] {
            [0.0; 3]
        } else {
            [1.0; 3]
        };
        self.server_send_particles(sunrise_sim::lan::server_effects::ServerQueuedParticle {
            dimension: self.common.dim.id(),
            particle,
            at,
            spread: velocity,
            speed,
            count: 0,
            randomization: 0,
            except: None,
        });
    }
}
