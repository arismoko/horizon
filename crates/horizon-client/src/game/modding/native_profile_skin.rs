//! A player's skin for server mods (SkinsRestorer): `PlayerRef::set_profile_property` changes
//! their profile, `PlayerRef::refresh` sends them to the other players again.
use super::Game;
use horizon_protocol::packets::login::Property;
use mod_api::PlayerRef;

/// `ExtraCodecs.PROPERTY_MAP`'s limits: 16 properties, names of 64
/// characters, values of 32767 and signatures of 1024.
const MAX_PROPERTIES: usize = 16;
const MAX_PROPERTY_NAME: usize = 64;
const MAX_PROPERTY_VALUE: usize = 32767;
const MAX_PROPERTY_SIGNATURE: usize = 1024;

impl Game<'_> {
    /// Sets property `name` of the player's profile, replacing one of the same name.
    pub(super) fn player_set_profile_property(
        &mut self,
        player: PlayerRef,
        name: &str,
        value: &str,
        signature: Option<&str>,
    ) -> bool {
        if name.is_empty()
            || name.chars().count() > MAX_PROPERTY_NAME
            || value.chars().count() > MAX_PROPERTY_VALUE
            || signature.is_some_and(|s| s.chars().count() > MAX_PROPERTY_SIGNATURE)
        {
            return false;
        }
        let Some(i) = self.connected_guest_index(player) else {
            return false;
        };
        let Some(host) = self.lan.as_mut() else {
            return false;
        };
        let property = Property {
            name: name.to_owned(),
            value: value.to_owned(),
            signature: signature.map(str::to_owned),
        };
        host.hub.guests.update(i, |g| {
            if let Some(old) = g.properties.iter_mut().find(|p| p.name == name) {
                *old = property;
                return true;
            }
            if g.properties.len() >= MAX_PROPERTIES {
                return false;
            }
            g.properties.push(property);
            true
        })
    }

    /// Sends the player to every other player again (`Hub::refresh_player`).
    pub(super) fn player_refresh(&mut self, player: PlayerRef) -> bool {
        let Some(i) = self.connected_guest_index(player) else {
            return false;
        };
        let Some(host) = self.lan.as_mut() else {
            return false;
        };
        let uuid = host.hub.guests[i].uuid;
        host.hub.refresh_player(uuid);
        true
    }
}
