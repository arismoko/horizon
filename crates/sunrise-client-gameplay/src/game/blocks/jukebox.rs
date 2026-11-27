//! Client jukebox song playback, Now Playing overlay, and level-event handlers.
use crate::game::ClientGame;
use sunrise_game_data::text::Text;
#[allow(unused_imports)]
pub(in crate::game) use sunrise_gameplay_common::game::jukebox_common::{
    jukebox_playable, jukebox_signals, song_of_record,
};
use sunrise_items::items::Song;
use sunrise_level::containers::BlockEntity;

#[cfg(feature = "client")]
impl ClientGame<'_> {
    /// Level event 1010: the song from the jukebox, and the "Now Playing" overlay.
    pub(in crate::game::client_side) fn start_song(&mut self, pos: [i32; 3], song: &str) {
        assert!(
            !self.is_logical_server(),
            "start_song called on server role"
        );
        let definition = self.items().songs.get(song).cloned();
        self.play_song(pos, definition);
    }

    fn play_song(&mut self, pos: [i32; 3], definition: Option<Song>) {
        self.stop_sound(pos);
        let Some(s) = definition else {
            return;
        };
        let id = self.play_audio_jukebox_song(&s.sound, pos);
        self.client
            .jukebox_sounds
            .insert((pos[0], pos[1], pos[2]), id);
        let description =
            sunrise_game_data::component::to_text(&s.description, &mut |k| Text::literal(k));
        let message = Text::translate("record.nowPlaying", &[description]);
        self.say_system_now(&message);
        self.client.titles.set_overlay(message, true);
        self.common.notify_record_playing(pos, true);
    }

    /// `LevelEventHandler` 1010 on a client of a server.
    pub(in crate::game) fn jukebox_song_event(&mut self, pos: [i32; 3], data: i32) {
        let Some(registries) = self.client.remote.as_ref().map(|r| r.registries.clone()) else {
            return;
        };
        let Some((name, tag)) = registries.entry("minecraft:jukebox_song", data) else {
            return;
        };
        let Some(tag) = tag else {
            self.start_song(pos, name);
            return;
        };
        let definition = Song::decode(&sunrise_game_data::scoreboard::tag_to_json(tag))
            .inspect_err(|error| log::warn!("Sunrise: jukebox_song {name}: {error}"))
            .ok();
        self.play_song(pos, definition);
    }

    /// `LevelEventHandler` 1011: `stopJukeboxSongAndNotifyNearby`.
    pub(in crate::game) fn jukebox_stop_event(&mut self, pos: [i32; 3]) {
        self.stop_sound(pos);
        self.common.notify_record_playing(pos, false);
    }

    pub(in crate::game) fn stop_sound(&mut self, pos: [i32; 3]) {
        assert!(
            !self.is_logical_server(),
            "stop_sound called on server role"
        );
        if let Some(id) = self.client.jukebox_sounds.remove(&(pos[0], pos[1], pos[2])) {
            self.client.audio.0.stop_and_release(id);
        }
    }

    /// Let go of songs that ended or whose jukebox stopped.
    pub(in crate::game::client_side) fn update_song_volumes(&mut self) {
        assert!(
            !self.is_logical_server(),
            "update_song_volumes called on server role"
        );
        let sounds: Vec<_> = self
            .client
            .jukebox_sounds
            .iter()
            .map(|(k, id)| (*k, *id))
            .collect();
        for (k, id) in sounds {
            let playing = matches!(self.world.block_entities.get(&k), Some(BlockEntity::Jukebox(j)) if j.playing);
            if !keeps_song(
                self.client.audio.0.engine.is_active(id),
                self.client.remote.is_some(),
                playing,
            ) {
                self.client.audio.0.stop_and_release(id);
                self.client.jukebox_sounds.remove(&k);
            }
        }
    }
}

/// Whether a jukebox song keeps playing.
pub(in crate::game::client_side) fn keeps_song(
    active: bool,
    server_client: bool,
    jukebox_playing: bool,
) -> bool {
    active && (server_client || jukebox_playing)
}

use sunrise_gameplay_common::game::tables::GameItems as _;

#[cfg(feature = "client")]
#[cfg(test)]
pub(in crate::game) mod tests {
    use super::keeps_song;
    use crate::game::client_side::native_entity_refs::tests::with_client_game;
    use sunrise_core::nbt::Tag;
    use sunrise_game_data::text::Text;
    use sunrise_gameplay_common::game::tables::GameItems as _;
    use sunrise_protocol::registry::Dynamic;

    #[test]
    fn a_server_clients_jukebox_song_lasts_until_it_ends() {
        // A server's client: on until the song ends (or level event 1011).
        assert!(keeps_song(true, true, false));
        assert!(!keeps_song(false, true, false));
        // A level that plays its own jukebox: also until the jukebox stops.
        assert!(keeps_song(true, false, true));
        assert!(!keeps_song(true, false, false));
    }

    /// A client plays the song the server synchronized, not its own table's.
    #[test]
    #[ignore = "daily"]
    fn level_event_1010_plays_the_servers_synchronized_song() {
        with_client_game(|game| {
            let (client, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(client, "jukebox-song-client").unwrap();
            let definition = Tag::compound([
                (
                    "sound_event",
                    Tag::String("minecraft:music_disc.cat".into()),
                ),
                ("description", Tag::String("Test Disc".into())),
                ("length_in_seconds", Tag::Float(10.0_f32.to_bits())),
                ("comparator_output", Tag::Int(4)),
            ]);
            let mut registries = Dynamic::default();
            registries.registries.insert(
                "minecraft:jukebox_song".into(),
                vec![("test:disc".to_owned(), Some(definition))],
            );
            game.client.remote.as_mut().unwrap().registries = std::sync::Arc::new(registries);
            assert!(!game.items().songs.contains_key("test:disc"));
            game.jukebox_song_event([0, 64, 0], 0);
            assert!(game.client.jukebox_sounds.contains_key(&(0, 64, 0)));
            assert_eq!(
                game.client.titles.overlay,
                Some(Text::translate(
                    "record.nowPlaying",
                    &[Text::literal("Test Disc")]
                ))
            );
            // An id past the registry does nothing (`Registry.get(int)` is empty).
            game.jukebox_song_event([1, 64, 0], 1);
            assert!(!game.client.jukebox_sounds.contains_key(&(1, 64, 0)));
        });
    }

    /// A known-pack entry the server sent without data plays the client's built-in song.
    #[test]
    #[ignore = "daily"]
    fn level_event_1010_plays_a_data_less_entry_from_the_clients_built_in_song() {
        with_client_game(|game| {
            let (client, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(client, "jukebox-built-in-client")
                .unwrap();
            let mut registries = Dynamic::default();
            registries.registries.insert(
                "minecraft:jukebox_song".into(),
                vec![("minecraft:cat".to_owned(), None)],
            );
            game.client.remote.as_mut().unwrap().registries = std::sync::Arc::new(registries);
            assert!(game.items().songs.contains_key("minecraft:cat"));
            game.jukebox_song_event([0, 64, 0], 0);
            assert!(game.client.jukebox_sounds.contains_key(&(0, 64, 0)));
        });
    }
}
