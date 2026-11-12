//! A server player's data for mods that carry it between servers: the compound mods keep on the
//! player and the advancement progress, read and replaced whole. (The forced respawn such a mod
//! also needs is `native_player_respawn`.)
use super::{Game, native_server::PlayerRead};
use horizon_core::nbt::{Compound, Tag};
use mod_api::PlayerRef;

/// The key of the mods' compound in a player's saved data (NeoForge's `Entity.PERSISTED_NBT_TAG`).
const PERSISTENT_DATA: &str = "NeoForgeData";

/// The mods' compound in a player's saved data; empty when there is none.
fn persistent_data(saved: Option<&Tag>) -> Compound {
    match saved.and_then(|t| t.get(PERSISTENT_DATA)) {
        Some(Tag::Compound(data)) => data.clone(),
        _ => Compound::new(),
    }
}

/// Puts `data` in a player's saved data as the mods' compound; an empty one takes the key away.
fn set_persistent_data(saved: &mut Option<Tag>, data: Compound) {
    if !matches!(saved, Some(Tag::Compound(_))) {
        *saved = Some(Tag::Compound(Compound::new()));
    }
    let Some(Tag::Compound(root)) = saved else {
        return;
    };
    if data.is_empty() {
        root.remove(&PERSISTENT_DATA.into());
    } else {
        root.insert(PERSISTENT_DATA.into(), Tag::Compound(data));
    }
}

impl Game<'_> {
    /// The mods' compound on a player: the acting player's saved data is the level's for their
    /// turn, another's is their record's.
    pub(super) fn player_persistent_data(&self, player: PlayerRef) -> Option<Compound> {
        if self.remote.is_some() {
            return None;
        }
        Some(match self.player_handle_read(player)? {
            PlayerRead::Acting { .. } => persistent_data(self.level.player_extra.as_ref()),
            PlayerRead::Guest(g) => persistent_data(g.saved.as_ref()),
        })
    }
    /// Replaces the mods' compound on a living player, in their turn so it lands in the record
    /// that is saved.
    pub(super) fn player_set_persistent_data(&mut self, player: PlayerRef, data: Compound) -> bool {
        if self.remote.is_some() {
            return false;
        }
        self.native_as_player_handle(player, |game| {
            set_persistent_data(&mut game.level.player_extra, data);
        })
    }
    /// `PlayerAdvancements.save`'s JSON of a player's progress now.
    pub(super) fn player_advancements(&self, player: PlayerRef) -> Option<String> {
        if self.remote.is_some() {
            return None;
        }
        let progress = match self.player_handle_read(player)? {
            PlayerRead::Acting { .. } => &self.adv.progress,
            PlayerRead::Guest(g) => &g.advancements,
        };
        serde_json::to_string_pretty(&progress.to_json(&self.adv.tree, horizon_world::DATA_VERSION))
            .ok()
    }
    /// `PlayerAdvancements.reload` from `json`, then a full advancement packet: the player's
    /// visible list is forgotten, so the next update resets the client's tree.
    pub(super) fn player_set_advancements(&mut self, player: PlayerRef, json: &str) -> bool {
        if self.remote.is_some() {
            return false;
        }
        let Ok(value @ serde_json::Value::Object(_)) = serde_json::from_str(json) else {
            return false;
        };
        let conn = match self.player_handle_read(player) {
            Some(PlayerRead::Acting { .. }) => self.lan_acting,
            Some(PlayerRead::Guest(g)) => Some(g.conn),
            None => None,
        };
        let Some(conn) = conn else {
            return false;
        };
        let tree = self.advancement_tree();
        let mut progress = horizon_client::advancements::Progress::from_json(&value);
        progress.criteria.retain(|id, _| tree.by_id.contains_key(id));
        let set = self.native_as_player_handle(player, |game| {
            game.adv.progress = progress;
            game.adv.changed.clear();
        });
        if set && let Some(host) = self.lan.as_mut() {
            host.hub.guests.update_connection(conn, |g| {
                g.advancements_loaded = true;
                g.advancements_visible.clear();
            });
        }
        set
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_guest_turn_tests::with_guests;
    use horizon_core::nbt::{Compound, Tag};

    fn saved_data(game: &super::Game<'_>, conn: u32) -> Option<Tag> {
        let guests = &game.lan.as_ref().unwrap().hub.guests;
        let g = &guests[guests.connection_index(conn).unwrap()];
        g.saved.as_ref().and_then(|s| s.get("NeoForgeData")).cloned()
    }

    /// The mods' compound goes into the player's own saved record, in their turn or out of it,
    /// and an empty one takes the key away.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn persistent_data_is_kept_in_the_players_record() {
        with_guests(|game| {
            let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
            assert_eq!(game.player_persistent_data(b), Some(Compound::new()));
            let mut data = Compound::new();
            data.insert("husksync:key".into(), Tag::Int(7));
            assert!(game.player_set_persistent_data(b, data.clone()));
            assert_eq!(saved_data(game, 13), Some(Tag::Compound(data.clone())));
            assert_eq!(saved_data(game, 12), None);
            game.as_guest(12, |game| {
                assert_eq!(game.player_persistent_data(b), Some(data.clone()));
                assert!(game.player_set_persistent_data(a, data.clone()));
                assert_eq!(game.player_persistent_data(a), Some(data.clone()));
            })
            .unwrap();
            assert_eq!(saved_data(game, 12), Some(Tag::Compound(data.clone())));
            assert!(game.player_set_persistent_data(b, Compound::new()));
            assert_eq!(saved_data(game, 13), None);
            assert_eq!(game.player_persistent_data(b), Some(Compound::new()));
        });
    }

    /// Progress read as the save writes it goes back in whole; an advancement this server
    /// lacks is dropped, and the client is sent the whole tree again.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn advancements_are_read_and_replaced_whole() {
        with_guests(|game| {
            game.advancement_tree();
            let b = game.player(13).unwrap();
            let json = r#"{"minecraft:story/root":{"criteria":{"crafting_table":"2026-11-11 10:00:00 +0000"},"done":true},"other:gone":{"criteria":{"x":"2026-11-11 10:00:00 +0000"},"done":true},"DataVersion":4000}"#;
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update_connection(13, |g| g.advancements_visible = vec![true]);
            assert!(game.player_set_advancements(b, json));
            let guests = &game.lan.as_ref().unwrap().hub.guests;
            let g = &guests[guests.connection_index(13).unwrap()];
            assert!(g.advancements.has("minecraft:story/root", "crafting_table"));
            assert!(!g.advancements.criteria.contains_key("other:gone"));
            assert!(g.advancements_visible.is_empty(), "the next update resets the client");
            let read: serde_json::Value =
                serde_json::from_str(&game.player_advancements(b).unwrap()).unwrap();
            assert_eq!(
                read["minecraft:story/root"]["criteria"]["crafting_table"],
                "2026-11-11 10:00:00 +0000"
            );
            assert_eq!(read["minecraft:story/root"]["done"], true);
            assert!(read.get("DataVersion").is_some());
            assert!(!game.player_set_advancements(b, "[1, 2]"));
            assert!(!game.player_set_advancements(b, "not json"));
            assert!(game.player_set_advancements(b, "{}"));
            let guests = &game.lan.as_ref().unwrap().hub.guests;
            let g = &guests[guests.connection_index(13).unwrap()];
            assert!(g.advancements.criteria.is_empty());
        });
    }
}
