//! The tab list's entries as the client holds them, for reads that must not change them.

use super::session_state::Remote;
use sunrise_level::profile_lookup::TabPlayer;

impl Remote {
    /// The listed players (`ClientPacketListener.getListedOnlinePlayers`) in encounter order.
    /// `tab_list::sorted_players` gives the overlay's display order.
    pub(in crate::game) fn listed_tab_player_refs(&self) -> impl Iterator<Item = &TabPlayer> {
        self.listed
            .iter()
            .filter_map(|uuid| self.tab_players.get(uuid))
    }
}

impl Remote {
    /// The listed players' skins as the Tab overlay draws them: each player's resolved skin
    /// texture path and whether it uses the slim model, in no order. A skin still downloading
    /// reads as the profile's default (`ResolvedSkin::default().body`), as the overlay draws it.
    pub(in crate::game) fn listed_player_skins(&self) -> Vec<(u128, String, bool)> {
        listed_player_skins(self.listed.iter(), &self.tab_players, &self.skins)
    }
}

/// `Remote::listed_player_skins`'s body, free so a test can drive it without a connected
/// session: only listed uuids with a tab entry (`listed_tab_player_refs`), each resolved
/// through `ResolvedSkin::body` — the textures' `slim` model, or the UUID's default body
/// and model where the server sent no textures.
pub(in crate::game) fn listed_player_skins<'a>(
    listed: impl Iterator<Item = &'a u128>,
    tab_players: &std::collections::HashMap<u128, TabPlayer>,
    skins: &std::collections::HashMap<u128, sunrise_client::skins::ResolvedSkin>,
) -> Vec<(u128, String, bool)> {
    listed
        .filter_map(|uuid| tab_players.get(uuid))
        .map(|player| {
            let uuid = player.uuid;
            let default = sunrise_client::skins::ResolvedSkin::default();
            let (skin, slim) = skins
                .get(&uuid)
                .map(|skin| skin.body(uuid))
                .unwrap_or_else(|| default.body(uuid));
            (uuid, skin.to_owned(), slim)
        })
        .collect()
}
