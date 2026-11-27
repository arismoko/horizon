//! Server-side jukebox simulation, record insertion/ejection, ticking, and tests.
use crate::game::ServerGame;
use sunrise_core::{position::BlockPos, random::RandomSource};
#[allow(unused_imports)]
use sunrise_gameplay_common::game::jukebox_common::{
    jukebox_playable, jukebox_signals, song_of_record,
};
use sunrise_world::level::flags as level_flags;
use {
    sunrise_items::items::ItemStack, sunrise_level::containers::BlockEntity,
    sunrise_level::containers::Jukebox, sunrise_level::particle_options::ParticleType,
    sunrise_sim::block_entity_item, sunrise_sim::mobs::Who,
};

impl ServerGame<'_> {
    #[inline(always)]
    pub(in crate::game) fn start_song_server(&mut self, pos: [i32; 3], song: &str) {
        self.common.notify_record_playing(pos, true);
        if self.server.audio_capture_active {
            self.server.lan_songs.push((pos, Some(song.to_owned())));
        }
    }

    /// Level event 1011: the song ends, here and for the guests.
    pub(in crate::game) fn stop_song(&mut self, pos: [i32; 3]) {
        self.common.notify_record_playing(pos, false);
        if self.server.audio_capture_active {
            self.server.lan_songs.push((pos, None));
        }
    }

    pub fn set_has_record(&mut self, pos: [i32; 3], has: bool) -> bool {
        self.set_has_record_notify(pos, has, true)
    }

    /// The state change alone (`level.setBlock` with flag 2); `notify` adds `block_change`.
    pub fn set_has_record_notify(&mut self, pos: [i32; 3], has: bool, notify: bool) -> bool {
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        let registry = self.common.pack.assets.registry;
        let Some(mut s) = registry.state(state).cloned() else {
            return false;
        };
        s.properties.insert(
            "has_record".into(),
            if has { "true" } else { "false" }.into(),
        );
        let Some(new) = registry.id(&s) else {
            return false;
        };
        let p = BlockPos::new(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| {
            b.set_block(level, ticks, p, new, level_flags::UPDATE_CLIENTS)
        });
        if notify {
            let state = self.common.world.block(pos[0], pos[1], pos[2]);
            self.post_block_game_event("minecraft:block_change", pos, None, Some(state));
        }
        true
    }

    /// `JukeboxBlockEntity.notifyItemChangedInJukebox`.
    pub(in crate::game) fn jukebox_record_changed(&mut self, pos: [i32; 3], has: bool) {
        self.set_has_record(pos, has);
    }

    /// `JukeboxBlock.setPlacedBy`.
    pub(in crate::game) fn jukebox_placed(&mut self, pos: [i32; 3], stack: &ItemStack) {
        if block_entity_item::carries_record(stack) {
            self.set_has_record_notify(pos, true, false);
        }
    }

    /// `onSongChanged`: neighbours (the signal) and comparators.
    pub(in crate::game) fn jukebox_changed(&mut self, pos: [i32; 3]) {
        let p = BlockPos::new(pos[0], pos[1], pos[2]);
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| {
            let block = b.block(state);
            b.update_neighbors_at(level, ticks, p, block);
        });
        self.container_changed((pos[0], pos[1], pos[2]));
        self.common
            .world
            .modified
            .insert((pos[0] >> 4, pos[2] >> 4));
    }

    /// `JukeboxBlockEntity.setTheItem` from a container (hopper/dropper).
    pub(in crate::game) fn jukebox_inserted(&mut self, pos: [i32; 3]) {
        let Some(song) = self
            .common
            .jukebox(pos)
            .filter(|j| !j.playing)
            .and_then(|j| j.record())
            .and_then(|record| song_of_record(record, self.items()))
        else {
            return;
        };
        if let Some(BlockEntity::Jukebox(j)) = self
            .common
            .world
            .block_entities
            .get_mut(&(pos[0], pos[1], pos[2]))
        {
            j.playing = true;
            j.song_clock_present = true;
            j.loaded_clock = false;
            j.ticks = 0;
        }
        self.jukebox_record_changed(pos, true);
        self.start_song_server(pos, &song);
        self.jukebox_changed(pos);
    }

    /// `JukeboxBlockEntity.setTheItem(EMPTY)` from a container.
    pub(in crate::game) fn jukebox_taken(&mut self, pos: [i32; 3]) {
        if self
            .common
            .jukebox(pos)
            .is_some_and(|j| j.record().is_some())
        {
            return;
        }
        self.jukebox_stopped(pos);
    }

    /// Stop song and clear `has_record`.
    pub(in crate::game) fn jukebox_stopped(&mut self, pos: [i32; 3]) {
        let mut was_playing = false;
        if let Some(BlockEntity::Jukebox(j)) = self
            .common
            .world
            .block_entities
            .get_mut(&(pos[0], pos[1], pos[2]))
        {
            was_playing = j.playing;
            j.playing = false;
            j.song_clock_present = false;
            j.loaded_clock = false;
            j.ticks = 0;
        }
        self.jukebox_record_changed(pos, false);
        if was_playing {
            self.stop_song_heard(pos);
        } else {
            self.stop_song(pos);
        }
        self.jukebox_changed(pos);
    }

    pub fn jukebox_set_item(&mut self, pos: [i32; 3], stack: Option<ItemStack>) {
        let song = stack.as_ref().and_then(|s| song_of_record(s, self.items()));
        let has_record = stack.is_some();
        let Some(BlockEntity::Jukebox(j)) = self
            .common
            .world
            .block_entities
            .get_mut(&(pos[0], pos[1], pos[2]))
        else {
            return;
        };
        let was_playing = j.playing;
        j.items[0] = stack;
        j.playing = song.is_some();
        j.song_clock_present = song.is_some();
        j.loaded_clock = false;
        j.ticks = 0;
        self.jukebox_record_changed(pos, has_record);
        match song {
            Some(song) => self.start_song_server(pos, &song),
            None if was_playing => self.stop_song_heard(pos),
            None => {}
        }
        self.jukebox_changed(pos);
    }

    /// The jukebox's item or empty-hand method, on the captured block state.
    pub fn use_jukebox(
        &mut self,
        pos: [i32; 3],
        state: sunrise_world::registry::BlockStateId,
        held: Option<&ItemStack>,
        branch: sunrise_world::level::UseBranch,
    ) -> Option<mod_api::InteractionResult> {
        use mod_api::InteractionResult as Answer;
        if &*self.common.res.facts().get(state).name != "minecraft:jukebox" {
            return None;
        }
        let has_record = |state| {
            self.common
                .pack
                .assets
                .registry
                .state(state)
                .and_then(|s| s.properties.get("has_record"))
                .is_some_and(|v| v == "true")
        };
        if matches!(branch, sunrise_world::level::UseBranch::WithoutItem) {
            if has_record(state) {
                self.eject_record(pos);
                return Some(Answer::Success);
            }
            return Some(Answer::Pass);
        }
        if has_record(state) {
            return Some(Answer::TryWithEmptyHand);
        }
        let current = self.common.world.block(pos[0], pos[1], pos[2]);
        if &*self.common.res.facts().get(current).name != "minecraft:jukebox" || has_record(current)
        {
            return Some(Answer::TryWithEmptyHand);
        }
        let Some(stack) = held else {
            return Some(Answer::TryWithEmptyHand);
        };
        let Some(song) = jukebox_playable(stack, self.common.res.items()) else {
            return Some(Answer::TryWithEmptyHand);
        };
        let record = ItemStack {
            count: 1,
            ..stack.clone()
        };
        if !self.common.creative {
            self.consume_held();
        }
        self.common.world.block_entities.insert(
            (pos[0], pos[1], pos[2]),
            BlockEntity::Jukebox(Jukebox {
                items: [Some(record)],
                ticks: 0,
                playing: true,
                song_clock_present: true,
                loaded_clock: false,
            }),
        );
        self.jukebox_record_changed(pos, true);
        if let Some(song) = song {
            self.start_song_server(pos, &song);
        }
        self.jukebox_changed(pos);
        self.post_block_game_event(
            "minecraft:block_change",
            pos,
            Some(Who::Player),
            Some(state),
        );
        self.award_custom_stat("play_record", 1);
        Some(Answer::Success)
    }

    /// `JukeboxSongPlayer.stop` posting `jukebox_stop_play`.
    pub(in crate::game::server_side) fn stop_song_heard(&mut self, pos: [i32; 3]) {
        self.stop_song(pos);
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        self.post_block_game_event("minecraft:jukebox_stop_play", pos, None, Some(state));
    }

    /// `popOutTheItem`: pop disc above the jukebox and stop the song.
    pub(in crate::game::server_side) fn eject_record(&mut self, pos: [i32; 3]) {
        let key = (pos[0], pos[1], pos[2]);
        let (record, was_playing) = match self.common.world.block_entities.get_mut(&key) {
            Some(BlockEntity::Jukebox(j)) => {
                if j.items[0].is_none() {
                    return;
                }
                let was_playing = j.playing;
                j.playing = false;
                j.song_clock_present = false;
                j.loaded_clock = false;
                j.ticks = 0;
                (j.items[0].take(), was_playing)
            }
            _ => return,
        };
        self.jukebox_record_changed(pos, false);
        if was_playing {
            self.stop_song_heard(pos);
        } else {
            self.stop_song(pos);
        }
        if let Some(record) = record {
            let at = sunrise_world::level::removal::jukebox_pop_position(
                &mut self.common.world.random,
                BlockPos::new(pos[0], pos[1], pos[2]),
            );
            self.spawn_item_own(record, at, 10);
        }
        self.jukebox_changed(pos);
    }

    /// A broken jukebox stops its song (its disc drops with the contents).
    pub fn jukebox_removed(&mut self, pos: [i32; 3]) {
        self.stop_song(pos);
    }

    /// `JukeboxSongPlayer.tick` of the jukebox at `key`, while it plays.
    pub fn tick_jukebox(&mut self, key: (i32, i32, i32)) {
        let playing = match self.common.world.block_entities.get(&key) {
            Some(BlockEntity::Jukebox(j)) if j.playing => j.record().and_then(|r| {
                let song = song_of_record(r, self.items())?;
                Some((song, j.ticks, j.loaded_clock))
            }),
            _ => None,
        };
        let Some((song, ticks, loaded)) = playing else {
            return;
        };
        let pos = [key.0, key.1, key.2];
        let Some(s) = self.items().songs.get(&song).cloned() else {
            return;
        };
        if s.finished(ticks) {
            if let Some(BlockEntity::Jukebox(j)) = self.common.world.block_entities.get_mut(&key) {
                j.playing = false;
                j.song_clock_present = false;
                j.loaded_clock = false;
                j.ticks = 0;
            }
            if loaded {
                return;
            }
            self.stop_song_heard(pos);
            self.jukebox_changed(pos);
            return;
        }
        if loaded
            && let Some(BlockEntity::Jukebox(j)) = self.common.world.block_entities.get_mut(&key)
        {
            j.loaded_clock = false;
        }
        if ticks % 20 == 0 {
            let state = self.common.world.block(pos[0], pos[1], pos[2]);
            self.post_block_game_event("minecraft:jukebox_play", pos, None, Some(state));
            let color = f64::from(self.common.world.random.next_int_bound(4).unwrap_or(0)) / 24.0;
            let at = [
                f64::from(pos[0]) + 0.5,
                f64::from(pos[1]) + f64::from(1.2_f32),
                f64::from(pos[2]) + 0.5,
            ];
            self.server_particle_at(
                sunrise_sim::lan::server_effects::ServerParticle::simple(ParticleType::Note),
                at,
                [color, 0.0, 0.0],
            );
        }
        if let Some(BlockEntity::Jukebox(j)) = self.common.world.block_entities.get_mut(&key) {
            j.ticks += 1;
        }
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;
