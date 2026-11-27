use super::Remote;
use sunrise_level::profile_lookup::default_skin;

impl Remote {
    /// `PlayerInfo.getSkin`: resolved skin or the profile's default, independent of listing.
    pub(in crate::game) fn player_info_skin(&self, uuid: u128) -> &str {
        self.skins
            .get(&uuid)
            .map(|skin| skin.body(uuid).0)
            .unwrap_or_else(|| default_skin(uuid))
    }
}
