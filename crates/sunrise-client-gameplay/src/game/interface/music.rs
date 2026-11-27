//! `Minecraft.getSituationalMusic` and `getMusicVolume` for the music manager,
//! the local player's ambient sound handlers and the sound engine tick.
use crate::game::{ClientGame, native_hooks::play_sound::HookedSounds};
use sunrise_client::{
    music::{BackgroundMusic, Music, MusicFrequency, Situation},
    options_ui::MusicGroup,
    sound_engine::{SoundInstance, TickContext},
};
use sunrise_items::blocks::Fluid;
use sunrise_level::random_source;

impl ClientGame<'_> {
    /// `Minecraft.tick` -> `MusicManager.tick`, once per client tick.
    pub fn tick_music(&mut self) {
        let (mut music, mut volume) = self.situational_music();
        // A native mod's own choice of track and volume, then its layers.
        self.native_music(&mut music, &mut volume);
        let mut layers = Vec::new();
        self.native_music_layers(&mut layers);
        let mut music_layers = std::mem::take(&mut self.client.music_layers);
        music_layers.tick(&layers, &mut HookedSounds(self));
        self.client.music_layers = music_layers;
        // Options.musicFrequency's listener: MusicManager.setMinutesBetweenSongs.
        let frequency = MusicFrequency::parse(&self.client.settings.file.name("musicFrequency"));
        if frequency != self.client.music.frequency {
            self.client.music.set_frequency(frequency, music.as_ref());
        }
        // The Music page's off tracks leave every pool they play in (the
        // extra off: nothing is filtered, vanilla's pools play as they are).
        let off = if self.client.settings.extras.music_page {
            &self.client.settings.extras.music_off[..]
        } else {
            &[][..]
        };
        self.client.music.set_off(off);
        // LevelLoadingScreen: no new track before the player is in the world.
        // A reconfiguration's ServerReconfigScreen is a plain Screen, not a
        // LevelLoadingScreen: its menu music starts as on any other screen.
        let loading = self.client.title.0.is_none()
            && self.common.player.is_none()
            && self.client.win.is_none()
            && !self
                .client
                .remote
                .as_ref()
                .is_some_and(|r| r.reconfiguring.is_some());
        let mut manager = std::mem::take(&mut self.client.music);
        manager.tick(music.as_ref(), volume, loading, &mut HookedSounds(self));
        self.client.music = manager;
        // Options.musicToast's listener: ToastManager.setMusicToastDisplayState.
        let state = self.client.settings.file.int("musicToast").clamp(0, 2) as u8;
        if state != self.client.music_toast_state {
            self.client.music_toast_state = state;
            self.set_music_toast_state(state);
        }
        // MusicManager's showNowPlayingToast and hideNowPlayingToast calls.
        for show in std::mem::take(&mut self.client.music.toast_requests) {
            if show {
                self.show_music_toast();
            } else if let Some(toast) = &mut self.client.now_playing {
                toast.hide();
            }
        }
    }
    /// The Music page's groups: every track the loaded `sounds.json` can play,
    /// by situation (see `music::track_groups`).
    pub(in crate::game::client_side) fn music_groups(&self) -> Vec<MusicGroup> {
        sunrise_client::music::track_groups(&self.client.audio.0.engine.registry)
    }
    /// A Music page row: play this track of `event` now.
    pub(in crate::game::client_side) fn play_music_track(&mut self, event: &str, track: &str) {
        let mut manager = std::mem::take(&mut self.client.music);
        manager.play_track(event, track, &mut HookedSounds(self));
        self.client.music = manager;
    }
    /// `Options.getFinalSoundSourceVolume(SoundSource.MUSIC)`.
    fn final_music_volume(&self) -> f32 {
        (self.client.settings.master_volume * self.client.settings.volumes[1]) as f32
    }
    /// `ToastManager.setMusicToastDisplayState`: Pause and Pause and Toast make
    /// the toast (the second one shows it at once while music is audible),
    /// Never drops it.
    fn set_music_toast_state(&mut self, state: u8) {
        self.client.now_playing = (state != 0).then(Default::default);
        if state == 2 && self.final_music_volume() > 0.0 {
            self.show_music_toast();
        }
    }
    fn show_music_toast(&mut self) {
        if self.client.now_playing.is_none() {
            return;
        }
        let Some(song) = self.now_playing_song() else {
            return;
        };
        let mut title = song;
        let mut description = None;
        if self.admit_toast(mod_api::ToastKind::Other, &mut title, &mut description)
            && let Some(toast) = &mut self.client.now_playing
        {
            toast.title = title;
            toast.description = description;
            toast.show(self.client.settings.notification_display_time);
        }
    }

    /// `NowPlayingToast.getNowPlayingString` of the song the music manager
    /// plays: its file's short language key, slashes as dots.
    fn now_playing_song(&self) -> Option<sunrise_game_data::text::Text> {
        let id = self.client.music.current_instance()?;
        let location = self
            .client
            .audio
            .0
            .engine
            .instance(id)?
            .sound
            .as_ref()?
            .location
            .as_str();
        let (namespace, path) = location.split_once(':').unwrap_or(("minecraft", location));
        // Identifier.toShortLanguageKey.
        let key = if namespace == "minecraft" {
            path.to_owned()
        } else {
            format!("{namespace}.{path}")
        }
        .replace('/', ".");
        Some(sunrise_game_data::text::Text::literal(
            sunrise_game_data::lang::get_or(&key, &key),
        ))
    }
    /// `ToastManager.update` (it plays no toast sound for this toast) and
    /// `extractRenderState` for the Now Playing toast, which slides in at the
    /// top left over every screen but the pause screen, and
    /// `PauseScreen.extractRenderState` with its `tick`, which draws the song
    /// name on the pause menu under the Pause and Pause and Toast settings
    /// while music is audible. `hidden` (F1) keeps the toast
    /// timing out without drawing it.
    pub(in crate::game::client_side) fn draw_music_toast(
        &mut self,
        batch: &mut sunrise_client::hud::HudBatch,
        hidden: bool,
        clock: sunrise_client::toasts::ToastClock,
    ) {
        let song = self.now_playing_song();
        let Some(toast) = self.client.now_playing.as_mut() else {
            return;
        };
        toast.update(clock, song.is_some(), &mut self.client.music_notes);
        let Some(song) = song else {
            return;
        };
        if self.client.screen == super::Screen::Paused {
            if self.client.pause_menu && self.final_music_volume() > 0.0 {
                self.client.music_notes.tick(clock);
                sunrise_client::toasts::draw_now_playing(
                    &self.client.gui.atlas,
                    batch,
                    0.0,
                    &song,
                    self.client.music_notes.color,
                );
            }
        } else if !hidden
            && self.client.music_toast_state == 2
            && let Some(toast) = &self.client.now_playing
        {
            toast.draw(&self.client.gui.atlas, batch, &self.client.music_notes);
        }
    }
    /// `Minecraft.tick` -> `SoundManager.tick(pause)`, after the music, with
    /// `pauseAllExcept(MUSIC, UI)` when the game pauses and `resume` when it
    /// carries on.
    pub fn tick_sound_engine(&mut self) {
        update_bound_entities(self.client.audio.0.engine.bound_entities(), |id| {
            self.sound_entity_state(id)
        });

        let paused = (self.common.pause, self.client.mod_sounds_paused);
        if paused != self.client.sound_paused {
            self.client.audio.0.resume();
            match sound_pause(paused.0, paused.1) {
                SoundPause::All => self.client.audio.0.pause_all(),
                SoundPause::Game => self.client.audio.0.pause(),
                SoundPause::Playing => {}
            }
            self.client.sound_paused = paused;
        }
        let (vehicle, carts) = self.start_cart_sounds();
        let mobs = self.start_mob_sounds();
        let ridden = self.start_riding_sound();
        let (player_position, player_speed_sqr, player_fall_flying) = self
            .common
            .player
            .as_ref()
            .map_or(([0.0; 3], 0.0, false), |p| {
                let at = p.position;
                (
                    [at.x, at.y, at.z],
                    p.velocity.length_squared(),
                    p.fall_flying,
                )
            });
        // LocalPlayer.onSyncedDataUpdated: starting to glide starts the
        // elytra's flying sound.
        if player_fall_flying && !self.client.was_fall_flying {
            let instance = SoundInstance::elytra(player_position, random_source::create());
            self.play_audio_instance(instance, false);
        }
        self.client.was_fall_flying = player_fall_flying;
        let ctx = TickContext {
            player_underwater: self.client.ambience.was_underwater(),
            player_removed: self.common.player.is_none(),
            vehicle,
            carts,
            mobs,
            frozen: !self.common.clock.runs_normally(),
            player_position,
            player_speed_sqr,
            player_fall_flying,
            ridden,
            camera_position: self.client.audio.0.listener,
        };
        self.client
            .audio
            .0
            .set_listener_reserve(self.client.settings.extras.ui_sound_reserve);
        // Client mods' sounds asked for since the last update start with it, then their stops
        // and fades apply.
        self.apply_mod_sounds();
        self.dispatch_live_play_sounds();
        self.client.audio.0.tick(self.common.pause, &ctx);
        self.dispatch_play_sounds();
    }
    /// `Minecraft.setLevel` -> `updateLevelInEngines`: `SoundManager.stop()`.
    /// The music manager keeps its stopped track and, on its next tick, waits
    /// the level's music delay before the next one.
    pub(in crate::game::client_side) fn stop_sounds_for_new_level(&mut self) {
        self.client.ambience.clear(&mut self.client.audio.0);
        self.client.audio.0.leave_level();
        self.client.jukebox_sounds.clear();
        // Mod plays still queued for the next audio update go too.
        self.client.mod_sounds.clear();
    }
    /// `getSituationalMusic` and `getMusicVolume`.
    pub fn situational_music(&self) -> (Option<Music>, f32) {
        // Login creates the client level before its first player teleport;
        // reconfiguration clears it again. Use the render camera's probe
        // while the level is present, not the local player's eyes.
        let in_world = self.client.title.0.is_none()
            && (self.common.player.is_some()
                || self.client.remote.as_ref().is_some_and(|r| r.has_level()));
        let (background, probe_volume) = if in_world {
            let biome = self.biome_at(self.client.view.position);
            self.client_res().music().at(
                self.common.dim.id(),
                biome,
                world_biomes!(self).pack_file(biome),
            )
        } else {
            (BackgroundMusic::default(), 1.0)
        };
        let situation = Situation {
            // WinScreen.getBackgroundMusic.
            screen_music: self
                .client
                .win
                .is_some()
                .then(mod_api::client::music::Music::credits),
            in_world,
            in_end: self.common.dim == super::Dim::End,
            boss_music: self.boss_music(),
            background: &background,
            probe_volume,
            creative: self.common.creative,
            underwater: self.client.ambience.was_underwater(),
        };
        (situation.music(), situation.volume())
    }
    /// `BiomeManager.getNoiseBiomeAtPosition`: the quart cell holding a position.
    pub fn biome_at(&self, p: [f64; 3]) -> &str {
        let (x, y, z) = (
            p[0].floor() as i32,
            p[1].floor() as i32,
            p[2].floor() as i32,
        );
        self.common
            .world
            .chunks
            .get(&(x >> 4, z >> 4))
            .map_or("minecraft:plains", |c| {
                world_biomes!(self).name(c.biome(x, y, z))
            })
    }
    /// `isEyeInFluid(WATER)`: the eye inside the water of its block column.
    pub(in crate::game::client_side) fn eye_in_water(&self, x: f64, eye_y: f64, z: f64) -> bool {
        let (bx, by, bz) = (x.floor() as i32, eye_y.floor() as i32, z.floor() as i32);
        let f = self
            .common
            .res
            .facts()
            .get(self.common.world.block(bx, by, bz));
        if f.fluid != Fluid::Water {
            return false;
        }
        let above = self
            .common
            .res
            .facts()
            .get(self.common.world.block(bx, by + 1, bz));
        let height = if above.fluid == Fluid::Water {
            1.0
        } else {
            f64::from(f.fluid_level) / 9.0
        };
        eye_y <= f64::from(by) + height
    }
}

impl ClientGame<'_> {
    /// The local player's ambient sound handlers (LocalPlayer.tick).
    pub(in crate::game::client_side) fn tick_ambience(&mut self) {
        let Some(p) = self.common.player.as_ref() else {
            return;
        };
        let feet = [p.position.x, p.position.y, p.position.z];
        let eye_y = feet[1] + f64::from(p.dimensions.eye_height);
        let biome = self.biome_at(feet);
        let sounds = self.client_res().ambient().at(
            self.common.dim.id(),
            biome,
            world_biomes!(self).pack_file(biome),
        );
        let underwater = self.eye_in_water(feet[0], eye_y, feet[2]);
        let world = &self.common.world;
        let light = |b: [i32; 3]| {
            let l = world.light(b[0], b[1], b[2]);
            (l >> 4, l & 15)
        };
        let (half, height) = (
            f64::from(p.dimensions.width / 2.0),
            f64::from(p.dimensions.height),
        );
        let registry = &self.common.pack.assets.registry;
        let facts = self.common.res.facts();
        let bubble_column = sunrise_client::ambience::bubble_column_in(
            [
                feet[0] - half,
                feet[1],
                feet[2] - half,
                feet[0] + half,
                feet[1] + height,
                feet[2] + half,
            ],
            |b| {
                let state = world.block(b[0], b[1], b[2]);
                (&*facts.get(state).name == "minecraft:bubble_column").then(|| {
                    registry
                        .state(state)
                        .and_then(|s| s.properties.get("drag"))
                        .is_some_and(|d| d == "true")
                })
            },
        );
        let ctx = sunrise_client::ambience::AmbientContext {
            sounds: &sounds,
            position: feet,
            eye_y,
            underwater,
            spectator: self.own_player_is_spectator(),
            light: &light,
            bubble_column,
        };
        self.client.ambience.tick(&ctx, &mut self.client.audio.0);
        self.dispatch_play_sounds();
    }
}

/// Resolve each live entity once per sound tick, even when several sounds name it.
fn update_bound_entities(
    entities: Vec<sunrise_client::sound_engine::SoundEntity>,
    mut resolve: impl FnMut(u32) -> Option<sunrise_game_data::sound_source::SoundEntityState>,
) {
    use sunrise_game_data::sound_source::SoundEntityState;
    let mut states = std::collections::HashMap::new();
    for entity in entities {
        let previous = entity.state();
        if previous.removed {
            continue;
        }
        let state = *states
            .entry(entity.id)
            .or_insert_with(|| resolve(entity.id));
        entity.update(state.unwrap_or(SoundEntityState {
            removed: true,
            ..previous
        }));
    }
}

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

#[cfg(test)]
mod bound_lookup_tests {
    use super::update_bound_entities;
    use sunrise_client::sound_engine::SoundEntity;
    use sunrise_game_data::sound_source::SoundEntityState;

    #[test]
    fn simultaneous_sounds_share_a_lookup_but_follow_each_new_tick() {
        let a = SoundEntity::new(7, [0.0; 3], false);
        let b = SoundEntity::new(7, [1.0; 3], false);
        let mut calls = Vec::new();
        for position in [[24.0, 80.0, -16.0], [25.5, 81.0, -18.0]] {
            update_bound_entities(vec![a.clone(), b.clone()], |id| {
                calls.push(id);
                Some(SoundEntityState {
                    position,
                    silent: true,
                    removed: false,
                })
            });
            assert_eq!(a.state().position, position);
            assert_eq!(b.state().position, position);
            assert!(a.state().silent && b.state().silent);
        }
        assert_eq!(calls, vec![7, 7]);
    }

    #[test]
    fn missing_entities_stop_all_their_sounds_without_reviving_reused_ids() {
        let a = SoundEntity::new(7, [1.0; 3], false);
        let b = SoundEntity::new(7, [2.0; 3], false);
        let mut calls = 0;
        update_bound_entities(vec![a.clone(), b.clone()], |_| {
            calls += 1;
            None
        });
        assert_eq!(calls, 1);
        assert!(a.state().removed && b.state().removed);
        let replacement = SoundEntity::new(7, [3.0; 3], false);
        update_bound_entities(vec![a.clone(), replacement.clone(), b.clone()], |_| {
            calls += 1;
            Some(SoundEntityState {
                position: [4.0; 3],
                silent: false,
                removed: false,
            })
        });
        assert_eq!(calls, 2);
        assert!(a.state().removed && b.state().removed);
        assert_eq!(a.state().position, [1.0; 3]);
        assert_eq!(replacement.state().position, [4.0; 3]);
        assert!(!replacement.state().removed);
    }
}

/// Which sounds a tick holds paused.
#[derive(Debug, PartialEq)]
enum SoundPause {
    /// None.
    Playing,
    /// The game's pause (`SoundManager.pause`): all but the music and the interface.
    Game,
    /// A mod's `set_sounds_paused`: every sound, whatever the game's pause.
    All,
}

/// A mod's pause holds every sound and outlasts the game's; the game's holds its own set.
fn sound_pause(game: bool, mods: bool) -> SoundPause {
    match (game, mods) {
        (_, true) => SoundPause::All,
        (true, false) => SoundPause::Game,
        (false, false) => SoundPause::Playing,
    }
}

#[cfg(test)]
mod sound_pause_tests {
    use super::*;

    #[test]
    fn a_mods_pause_holds_every_sound_over_the_games() {
        assert_eq!(sound_pause(false, false), SoundPause::Playing);
        assert_eq!(sound_pause(true, false), SoundPause::Game);
        assert_eq!(sound_pause(false, true), SoundPause::All);
        assert_eq!(sound_pause(true, true), SoundPause::All);
    }
}
