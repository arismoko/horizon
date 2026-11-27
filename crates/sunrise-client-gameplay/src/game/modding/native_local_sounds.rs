//! Client mods' own sounds (`play_local_sound`) and their `SoundRef` handles: what each is
//! doing, stopping, fading, moving and re-pitching it, and the sounds and devices the client has.
use crate::game::ClientGame;
use mod_api::__private::game::*;
use mod_api::{SoundPlay, SoundRef, SoundState};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use sunrise_client::sound_engine::InstanceId;
use sunrise_core::identifier::Identifier;

/// `Channel.setSelfPosition` sends float coordinates to the mixer.
fn sound_position_readable(position: [f64; 3]) -> bool {
    position.into_iter().all(|axis| (axis as f32).is_finite())
}

/// What a mod asked of one of its sounds, applied at the next audio update.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Change {
    Stop,
    Fade { volume: f32, ticks: u32 },
    Move { position: [f64; 3] },
    Pitch { pitch: f32 },
}
impl Change {
    /// It ends the sound when applied: a stop or an immediate fade to a nonpositive volume.
    fn stops(self) -> bool {
        match self {
            Change::Stop => true,
            Change::Fade { volume, ticks } => ticks == 0 && volume <= 0.0,
            Change::Move { .. } | Change::Pitch { .. } => false,
        }
    }
}

/// Client mods' sounds: those asked for since the last audio update, the changes asked of
/// them, and each started one's engine instance by its handle.
pub struct ModSounds {
    /// The last handle given; handles are never reused.
    last: Cell<u64>,
    /// Sounds asked for (`play_local_sound`), with their handle and the ticks each
    /// waits, until the next audio update plays them (`apply_mod_sounds`).
    pub queued: RefCell<Vec<(u64, SoundPlay, u32)>>,
    /// Stops, fades, moves and pitch changes asked for since the last audio update, in order.
    changes: RefCell<Vec<(u64, Change)>>,
    /// Each started sound's engine instance, until the engine lets it go.
    started: HashMap<u64, InstanceId>,
}
impl Default for ModSounds {
    fn default() -> Self {
        Self {
            last: Cell::new(0),
            queued: RefCell::default(),
            changes: RefCell::default(),
            started: HashMap::new(),
        }
    }
}
impl ModSounds {
    /// Queues `sound` to start at the next audio update, `after` ticks from then.
    pub fn queue(&self, sound: &SoundPlay, after: u32) -> SoundRef {
        let id = self.last.get() + 1;
        self.last.set(id);
        self.queued.borrow_mut().push((id, sound.clone(), after));
        sound_ref(id)
    }
    pub(in crate::game::client_side) fn pending_checkpoint(&self) -> (usize, usize) {
        (self.queued.borrow().len(), self.changes.borrow().len())
    }

    pub(in crate::game::client_side) fn discard_pending_since(
        &self,
        (plays, changes): (usize, usize),
    ) {
        self.queued.borrow_mut().truncate(plays);
        self.changes.borrow_mut().truncate(changes);
        // Do not rewind `last`: a mod might have retained a handle to a discarded play.
    }

    /// Forgets every mod sound: the queued plays, the changes asked of them and the started
    /// ones' instances, as a level change stops every sound. Their handles read `Stopped`;
    /// handles are still never reused.
    pub(in crate::game::client_side) fn clear(&mut self) {
        self.queued.get_mut().clear();
        self.changes.get_mut().clear();
        self.started.clear();
    }
}

impl ClientGame<'_> {
    /// Queues a client-local sound for the next audio update, with a delay in unpaused
    /// sound-engine ticks (`ClientLevel.playLocalSound`, `SoundEngine.playDelayed`).
    pub(in crate::game) fn play_local_sound(
        &self,
        sound: &SoundPlay,
        after: u32,
    ) -> Option<SoundRef> {
        if !self.common.native_window()
            || !sunrise_gameplay_common::game::native_hooks::sound_and_shader_helpers::sound_id(
                &sound.identifier,
            )
            || !sound_position_readable(sound.position)
            || !sound.volume.is_finite()
            || !sound.pitch.is_finite()
            || !sound.effects.valid()
        {
            return None;
        }
        Some(self.client.mod_sounds.queue(sound, after))
    }

    /// Plays the sounds client mods asked for since the last update, then applies their stops
    /// and fades, and forgets the instances the engine let go. A play ended before this update
    /// never reaches the engine. Called from the audio update
    /// (`tick_sound_engine`), before the engine's own tick.
    pub(in crate::game::client_side) fn apply_mod_sounds(&mut self) {
        let stopped: Vec<u64> = self
            .client
            .mod_sounds
            .changes
            .get_mut()
            .iter()
            .filter(|(_, change)| change.stops())
            .map(|&(id, _)| id)
            .collect();
        let queued = std::mem::take(self.client.mod_sounds.queued.get_mut());
        for (id, sound, after) in queued {
            if stopped.contains(&id) {
                continue;
            }
            let instance = self.play_audio_mod_sound(&sound, after);
            self.client.mod_sounds.started.insert(id, instance);
        }
        let sounds = &mut self.client.mod_sounds;
        for (id, change) in sounds.changes.get_mut().drain(..) {
            let Some(&instance) = sounds.started.get(&id) else {
                continue;
            };
            match change {
                Change::Stop => self.client.audio.0.stop_mod_sound(instance),
                Change::Fade { volume, ticks } => {
                    self.client.audio.0.fade_mod_sound(instance, volume, ticks)
                }
                Change::Move { position } => self.client.audio.0.move_mod_sound(instance, position),
                Change::Pitch { pitch } => self.client.audio.0.set_mod_sound_pitch(instance, pitch),
            }
            if change.stops() {
                sounds.started.remove(&id);
            }
        }
        let audio = &self.client.audio.0;
        sounds
            .started
            .retain(|_, instance| audio.mod_sound_state(*instance) != SoundState::Stopped);
    }
    /// For a client mod: where its sound is. A stop or a fade to 0 at once asked since the last
    /// audio update already reads `Stopped`. None on a server.
    pub(in crate::game) fn sound_state(&self, sound: SoundRef) -> Option<SoundState> {
        if !self.common.native_window() {
            return None;
        }
        let id = sound.fields().id;
        let sounds = &self.client.mod_sounds;
        if sounds
            .changes
            .borrow()
            .iter()
            .any(|&(changed, change)| changed == id && change.stops())
        {
            return Some(SoundState::Stopped);
        }
        if sounds
            .queued
            .borrow()
            .iter()
            .any(|(queued, ..)| *queued == id)
        {
            return Some(SoundState::Waiting);
        }
        Some(
            sounds
                .started
                .get(&id)
                .map_or(SoundState::Stopped, |&instance| {
                    self.client.audio.0.mod_sound_state(instance)
                }),
        )
    }
    /// For a client mod: stops its sound for good at the next audio update. A waiting sound
    /// never starts and a looping one never repeats. False, changing nothing, for a sound that
    /// is already over, and on a server.
    pub(in crate::game) fn sound_stop(&self, sound: SoundRef) -> bool {
        self.change_mod_sound(sound, Change::Stop)
    }
    /// Changes this sound's volume over `ticks` unpaused sound-engine ticks; a nonpositive
    /// target stops it at the end. The mixer computes playback gain from the current volume.
    pub(in crate::game) fn sound_fade(&self, sound: SoundRef, volume: f32, ticks: u32) -> bool {
        if !volume.is_finite() {
            return false;
        }
        self.change_mod_sound(sound, Change::Fade { volume, ticks })
    }
    /// Moves the same instance from the next audio update, with no world-border check.
    pub(in crate::game) fn sound_move_to(&self, sound: SoundRef, position: [f64; 3]) -> bool {
        if !sound_position_readable(position) {
            return false;
        }
        self.change_mod_sound(sound, Change::Move { position })
    }
    /// Changes the base pitch of the same instance; the sound engine computes the final pitch.
    pub(in crate::game) fn sound_set_pitch(&self, sound: SoundRef, pitch: f32) -> bool {
        if !pitch.is_finite() {
            return false;
        }
        self.change_mod_sound(sound, Change::Pitch { pitch })
    }
    fn change_mod_sound(&self, sound: SoundRef, change: Change) -> bool {
        match self.sound_state(sound) {
            Some(SoundState::Waiting | SoundState::Playing) => {
                let id = sound.fields().id;
                self.client
                    .mod_sounds
                    .changes
                    .borrow_mut()
                    .push((id, change));
                true
            }
            _ => false,
        }
    }
    /// For a client mod: every sound event the client's resources have
    /// (`SoundManager.getAvailableSounds`), sorted. It builds a new list each call: ask from
    /// setup or a settings screen, not every frame. Empty on a server.
    pub(in crate::game) fn sound_events(&self) -> Vec<Identifier> {
        if !self.common.native_window() {
            return Vec::new();
        }
        self.client
            .audio
            .0
            .event_names()
            .iter()
            .filter_map(|name| Identifier::parse(name).ok())
            .collect()
    }
    /// For a client mod: the output devices sound can play on, by name
    /// (`SoundEngine.getAvailableSoundDevices`). Listing them probes every device (tens of
    /// milliseconds on some systems): ask from a settings screen, not every frame. Empty on a
    /// server and where sound is off.
    pub(in crate::game) fn audio_devices(&self) -> Vec<String> {
        if !self.common.native_window() {
            return Vec::new();
        }
        sunrise_client::audio::output_devices()
    }
    /// For a client mod: the output device the options ask for (`Options.soundDevice`), empty
    /// for the system's default. The sound plays on the default when that one can't be opened.
    /// Changing it keeps every mod sound playing, with its handle. Empty on a server.
    pub(in crate::game) fn audio_device(&self) -> String {
        if !self.common.native_window() {
            return String::new();
        }
        self.client.audio.0.device().to_owned()
    }
}

#[cfg(test)]
mod tests {
    use mod_api::__private::game::{Fields, sound_ref};
    use mod_api::{
        ClientHost, Flow, Hooks, ModEntry, RenderGuiPreEvent, SoundPlay, SoundRef, SoundSource,
        SoundState,
    };
    use mod_platform::testing::with_mods;
    use std::sync::{Arc, Mutex};
    use sunrise_core::identifier::Identifier;

    /// From a screen hook's read-only host: queues sounds to stop, fade and keep waiting.
    struct Player(Arc<Mutex<Vec<SoundRef>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Player {
        fn render_gui_pre(&self, host: &dyn ClientHost, _: &mut RenderGuiPreEvent) -> Flow {
            let click = SoundPlay::ui("minecraft:ui.button.click");
            let stopped = host.play_local_sound(&click, 40).unwrap();
            let faded = host.play_local_sound(&click.clone().looping(0), 0).unwrap();
            let far = SoundPlay::at(
                &click.identifier,
                SoundSource::Blocks,
                [31_000_000.0, 64.0, 0.0],
            )
            .with_volume(-0.5)
            .with_pitch(-1.0)
            .looping(-1);
            let delayed = host.play_local_sound(&far, u32::MAX).unwrap();
            assert_eq!(delayed.state(host), Some(SoundState::Waiting));
            assert_eq!(stopped.state(host), Some(SoundState::Waiting), "queued");
            assert!(stopped.stop(host));
            assert_eq!(
                stopped.state(host),
                Some(SoundState::Stopped),
                "a stop reads at once"
            );
            assert!(!stopped.stop(host), "already over");
            assert!(!stopped.fade(host, 0.5, 10), "already over");
            assert!(!stopped.move_to(host, [0.0; 3]), "already over");
            assert!(!stopped.set_pitch(host, 1.0), "already over");
            assert!(!faded.move_to(host, [f64::NAN, 0.0, 0.0]));
            assert!(!delayed.move_to(host, [0.0, f64::MAX, 0.0]));
            assert!(delayed.move_to(host, [0.0, 31_000_000.0, 0.0]));
            assert!(!faded.set_pitch(host, f32::NAN));
            assert!(faded.set_pitch(host, -1.0));
            assert!(!faded.fade(host, f32::NAN, 10));
            assert!(faded.fade(host, -1.0, 20));
            assert_eq!(
                faded.state(host),
                Some(SoundState::Waiting),
                "fading out, not over"
            );
            assert!(
                host.play_local_sound(&SoundPlay::ui("not an id"), 0)
                    .is_none()
            );
            assert!(
                host.play_local_sound(&click.clone().with_pitch(f32::NAN), 0)
                    .is_none()
            );
            let mut unreadable = far;
            unreadable.position[0] = f64::MAX;
            assert!(host.play_local_sound(&unreadable, 0).is_none());
            unreadable.position[0] = 0.0;
            unreadable.effects.gain = f32::NAN;
            assert!(host.play_local_sound(&unreadable, 0).is_none());
            let _ = (
                host.sound_events(),
                host.audio_devices(),
                host.audio_device(),
            );
            self.0.lock().unwrap().extend([stopped, faded, delayed]);
            Flow::Continue
        }
    }

    /// `SoundRef`: a handle per play, its state, stop and fade asked from any client hook and
    /// applied at the next audio update; a stopped play never starts. Unknown handles and a
    /// server read as over or none.
    #[test]
    #[ignore = "daily"]
    fn a_client_mod_follows_stops_and_fades_its_own_sounds() {
        let kept = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods =
            vec![ModEntry::new("player", Box::new(Player(Arc::clone(&kept))))].into();
        with_mods(mods, || {
            super::super::native_entity_refs::tests::with_client_game(|game| {
                game.native_render_gui_pre(&mut false);
                let [stopped, faded, delayed] = kept.lock().unwrap()[..] else {
                    panic!("three handles");
                };
                assert_ne!(stopped, faded, "a handle per play");
                game.tick_sound_engine();
                assert!(game.client.mod_sounds.queued.borrow().is_empty());
                assert_eq!(game.sound_state(stopped), Some(SoundState::Stopped));
                assert!(
                    !game
                        .client
                        .mod_sounds
                        .started
                        .contains_key(&stopped.fields().id),
                    "a stop lets the play go"
                );
                // Without an output device (a test machine) the engine starts nothing, so the
                // faded loop is over here; with one it plays until its fade ends.
                assert_ne!(game.sound_state(faded), Some(SoundState::Waiting));
                assert_eq!(game.sound_state(delayed), Some(SoundState::Waiting));
                // Every event the engine has, none dropped (prepared test resources may have
                // no sounds at all).
                let names = game.client.audio.0.event_names();
                let events = game.sound_events();
                assert_eq!(events.len(), names.len());
                assert!(
                    names
                        .iter()
                        .all(|name| events.contains(&Identifier::parse(name).unwrap()))
                );
                // A handle no play has, and a server.
                let unknown = sound_ref(999_999);
                assert_eq!(game.sound_state(unknown), Some(SoundState::Stopped));
                assert!(!game.sound_stop(unknown));
                game.options.execution_role =
                    super::super::execution_role::ExecutionRole::DedicatedServer;
                assert_eq!(game.sound_state(faded), None);
                assert!(
                    game.play_local_sound(&SoundPlay::ui("minecraft:ui.button.click"), 0)
                        .is_none()
                );
                assert!(game.sound_events().is_empty());
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_client_muffle_accepts_more_than_sixty_four_exempt_events() {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let exempt: Vec<_> = (0..65).map(|n| format!("test:sound.{n}")).collect();
            let exempt: Vec<_> = exempt.iter().map(String::as_str).collect();
            assert!(game.muffle_sounds(0.5, 0.5, 4, &exempt));
            assert!(!game.muffle_sounds(0.5, 0.5, 4, &["not an id"]));
        });
    }

    /// A play stopped before the audio update never reaches the engine, and a level change
    /// drops the plays still queued: their handles read `Stopped`, nothing plays later, and
    /// handles are still never reused.
    #[test]
    #[ignore = "daily"]
    fn stopped_and_level_cleared_plays_never_reach_the_engine() {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            game.client.audio.0.heard.clear();
            let harp = "minecraft:block.note_block.harp";
            let bell = "minecraft:block.note_block.bell";
            let stopped = game.play_local_sound(&SoundPlay::ui(harp), 0).unwrap();
            assert!(game.sound_stop(stopped));
            game.tick_sound_engine();
            assert!(
                !game.client.audio.0.heard.contains_key(harp),
                "stopped before the update"
            );
            assert_eq!(game.sound_state(stopped), Some(SoundState::Stopped));

            let queued = game.play_local_sound(&SoundPlay::ui(bell), 20).unwrap();
            assert_eq!(game.sound_state(queued), Some(SoundState::Waiting));
            game.stop_sounds_for_new_level();
            assert_eq!(game.sound_state(queued), Some(SoundState::Stopped));
            assert!(!game.sound_stop(queued), "already over");
            game.tick_sound_engine();
            assert!(
                !game.client.audio.0.heard.contains_key(bell),
                "cleared with the level"
            );

            let next = game.play_local_sound(&SoundPlay::ui(harp), 0).unwrap();
            assert!(next != stopped && next != queued, "a new handle");
        });
    }
}
