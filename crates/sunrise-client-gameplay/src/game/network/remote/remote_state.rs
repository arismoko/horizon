use super::Remote;
use std::sync::Arc;
use sunrise_gameplay_common::game::{api_values::TryIntoApi, remote::client_has_level};

impl Remote {
    pub fn set_menu_title(&mut self, title: Option<sunrise_game_data::text::Text>) {
        self.menu_title_snapshot = title.try_into_api().ok().flatten().map(Arc::new);
        self.menu_title = title;
    }

    pub fn menu_title_snapshot(&self) -> Option<&mod_api::Text> {
        self.menu_title_snapshot.as_deref()
    }

    pub fn shared_menu_title_snapshot(&self) -> Option<Arc<mod_api::Text>> {
        self.menu_title_snapshot.clone()
    }

    pub(in crate::game) fn overworld_clock_time(&self) -> i64 {
        self.clock_time(self.overworld_clock)
    }

    /// The server's synchronized `minecraft:timeline` registry and bound `minecraft:timeline`
    /// tags, decoded for client `Environment` construction.
    pub(in crate::game) fn synced_timelines(&self) -> sunrise_level::environment::TimelineTable {
        let mut timelines = std::collections::BTreeMap::new();
        if let Some(entries) = self.registries.registries.get("minecraft:timeline") {
            for (name, data) in entries {
                let Some(tag) = data else {
                    continue;
                };
                let json = sunrise_game_data::scoreboard::tag_to_json(tag);
                match sunrise_world::environment_timeline::Timeline::parse(&json) {
                    Ok(timeline) => {
                        timelines.insert(name.clone(), Arc::new(timeline));
                    }
                    Err(error) => {
                        log::warn!("synced timeline {name}: {error}");
                    }
                }
            }
        }
        let mut tags = std::collections::BTreeMap::new();
        if let Some(tag_map) = self.registries.tags.get("minecraft:timeline") {
            for (tag, ids) in tag_map {
                let members = ids
                    .iter()
                    .filter_map(|&id| {
                        self.registries
                            .name("minecraft:timeline", id)
                            .map(str::to_owned)
                    })
                    .collect();
                tags.insert(tag.clone(), members);
            }
        }
        sunrise_level::environment::TimelineTable { timelines, tags }
    }

    /// `Minecraft.level != null`: from `handleLogin` until the next
    /// `handleConfigurationStart` (`clearClientLevel`). Minecraft.tick sends
    /// CLIENT_TICK_END only then, so a client waiting for the server's Login
    /// (a busy server takes seconds to place a new player) sends nothing.
    pub(in crate::game) fn has_level(&self) -> bool {
        client_has_level(self.logged_in, self.reconfiguring)
    }

    /// An entity the server added to this client level: its type
    /// (`minecraft:text_display`) and its position as last sent.
    pub fn tracked_entity(&self, id: i32) -> Option<(&str, [f64; 3])> {
        if let Some(row) = self.general.rows.get(&id) {
            return Some((row.kind(), row.position));
        }
        self.entities
            .get(&id)
            .map(|t| (t.kind.as_str(), t.position))
    }

    /// A player entity's profile name: the one it was created with, else its
    /// player-list entry's (an entity the list named after it spawned).
    pub(in crate::game) fn level_player_name(&self, id: i32, uuid: &u128) -> Option<&str> {
        self.level_player_names
            .get(&id)
            .or_else(|| self.players.get(uuid))
            .map(String::as_str)
    }

    /// The local player's profile UUID.
    pub fn profile(&self) -> u128 {
        self.profile
    }

    /// Another player's profile UUID, by their entity id (None for the local player).
    pub(in crate::game) fn player_uuid(&self, id: u32) -> Option<u128> {
        let uuid = *self.level_players.get(&(id as i32))?;
        (uuid != self.profile).then_some(uuid)
    }

    /// The client's synced Entity shared flag 0, including the local player.
    pub(in crate::game) fn entity_on_fire(&self, id: i32) -> Option<bool> {
        if id == self.entity_id {
            Some(self.own_on_fire)
        } else {
            self.entities.get(&id).map(|t| t.on_fire)
        }
    }
    pub(in crate::game) fn tracked_fire_immunity(&self, id: i32) -> Option<bool> {
        Some(self.entities.get(&id)?.mod_fire_immune)
    }
    pub(in crate::game) fn set_tracked_fire_immunity(&mut self, id: i32, immune: bool) -> bool {
        self.entities.get_mut(&id).is_some_and(|entity| {
            entity.mod_fire_immune = immune;
            true
        })
    }
    /// The kind and last sent position of a tracked server entity.
    pub fn tracked(&self, id: i32) -> Option<(&str, [f64; 3])> {
        self.entities.get(&id).map(|t| (&*t.kind, t.position))
    }
}
