use super::*;

impl crate::game::ServerGame<'_> {
    /// `PlayerAdvancements.save`: the existing network player's advancement file.
    pub(in crate::game::server_side) fn save_guest_advancements(
        &self,
        host: &LanHost,
        guest: &sunrise_sim::lan::guest::Guest,
    ) {
        if !guest.joined()
            || self.server.crash_save.excludes(guest.conn)
            || guest.released
            || !guest.participates_in_player_simulation()
            || self.common.adv.tree.list.is_empty()
        {
            return;
        }
        let Some(uuid) = self.records_uuid(host, guest.uuid) else {
            return;
        };
        let path = sunrise_gameplay_common::stats::player_json(
            &self.server.world_root,
            "advancements",
            uuid,
        );
        let json = guest
            .advancements
            .to_json(&self.common.adv.tree, sunrise_world::DATA_VERSION);
        if let Ok(text) = serde_json::to_string_pretty(&json) {
            let _ = sunrise_sim::save::write_durable(&path, text.as_bytes());
        }
    }
    /// `PlayerAdvancements.save` and `ServerStatsCounter.save` of a network player.
    pub fn save_guest_records(&self, host: &LanHost, guest: &sunrise_sim::lan::guest::Guest) {
        if !guest.joined()
            || self.server.crash_save.excludes(guest.conn)
            || guest.released
            || !guest.participates_in_player_simulation()
            || self.common.adv.tree.list.is_empty()
        {
            return;
        }
        let Some(uuid) = self.records_uuid(host, guest.uuid) else {
            return;
        };
        self.save_guest_advancements(host, guest);
        if let Some(play) = host.play.get(&guest.conn) {
            play.stats
                .write(&sunrise_gameplay_common::stats::player_json(
                    &self.server.world_root,
                    "stats",
                    uuid,
                ));
        }
    }
}
