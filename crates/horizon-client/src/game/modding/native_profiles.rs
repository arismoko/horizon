//! Game profiles for server mods: a player's own (`PlayerRef::profile`) and the ones the
//! server knows by UUID or name (`profile_by_uuid`, `profile_by_name`), never asked of Mojang.
use super::Game;
use horizon_protocol::packets::login::Property;
use mod_api::{PlayerProfile, PlayerRef, ProfileProperty};

fn profile_of(uuid: u128, name: &str, properties: &[Property]) -> PlayerProfile {
    PlayerProfile::new(
        uuid,
        name,
        properties
            .iter()
            .map(|p| ProfileProperty::new(&p.name, &p.value, p.signature.as_deref()))
            .collect(),
    )
}

impl Game<'_> {
    /// The handle's game profile: its UUID, name and properties.
    pub(super) fn player_profile(&self, player: PlayerRef) -> Option<PlayerProfile> {
        let i = self.connected_guest_index(player)?;
        let g = &self.lan.as_ref()?.hub.guests[i];
        Some(profile_of(g.uuid, &g.name, &g.properties))
    }

    /// An online player's profile by UUID, else the user cache's name.
    pub(super) fn profile_by_uuid(&self, uuid: u128) -> Option<PlayerProfile> {
        if let Some(g) = self.lan.as_ref().and_then(|host| {
            host.hub
                .online_players()
                .find(|g| g.uuid == uuid && !g.is_fake())
        }) {
            return Some(profile_of(g.uuid, &g.name, &g.properties));
        }
        if !self.is_dedicated_server() {
            return None;
        }
        let user = super::dedicated::cached_user_by_id(uuid)?;
        Some(PlayerProfile::new(user.id, &user.name, Vec::new()))
    }

    /// An online player's profile by name (any case), else an unexpired user cache entry.
    pub(super) fn profile_by_name(&self, name: &str) -> Option<PlayerProfile> {
        if let Some(g) = self.lan.as_ref().and_then(|host| {
            host.hub
                .online_players()
                .find(|g| g.name.eq_ignore_ascii_case(name) && !g.is_fake())
        }) {
            return Some(profile_of(g.uuid, &g.name, &g.properties));
        }
        if !self.is_dedicated_server() {
            return None;
        }
        let user = super::dedicated::cached_user_by_name(name)?;
        Some(PlayerProfile::new(user.id, &user.name, Vec::new()))
    }
}
