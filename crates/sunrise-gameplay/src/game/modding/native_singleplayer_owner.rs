//! `Players::singleplayer_profile` and `PlayerRef::is_singleplayer_owner`
//! (`MinecraftServer.getSingleplayerProfile`, `isSingleplayerOwner`): read from the profile
//! the integrated server is created with, before its startup hooks and apart from its hub.
use crate::game::ServerGame;
use mod_api::{GameProfile, PlayerRef};
use sunrise_core::profile::AccountProperty;
use sunrise_sim::lan::OwnerProfile;

/// `IntegratedServer`'s `singleplayerProfile`: the window's player as the server was created
/// with it, properties and all. Fixed once made: every read returns this snapshot.
pub struct SingleplayerProfile {
    pub name: String,
    pub uuid: u128,
    properties: Vec<AccountProperty>,
}

impl SingleplayerProfile {
    /// `setSingleplayerProfile(minecraft.getGameProfile())`: the signed-in account's
    /// properties, waiting for the fetch that started with the world's open as
    /// `getGameProfile` joins its `profileFuture`. An offline player has none.
    pub fn launched(name: String, uuid: u128, account: Option<&OwnerProfile>) -> Self {
        let properties = account.map(OwnerProfile::wait).unwrap_or_default().to_vec();
        Self {
            name,
            uuid,
            properties,
        }
    }
}

impl ServerGame<'_> {
    /// `IntegratedServer.getSingleplayerProfile`. None for a dedicated server, which is
    /// created with no such profile, and for a client.
    pub(in crate::game) fn native_singleplayer_profile(&self) -> Option<GameProfile<'_>> {
        let profile = self.server.singleplayer_profile.as_ref()?;
        Some(mod_api::__private::game::game_profile(
            profile.uuid,
            &profile.name,
            &profile.properties,
        ))
    }

    /// `IntegratedServer.isSingleplayerOwner`: the connected player's name against the
    /// profile's, ignoring case; the UUID, the properties and the permission level play no part.
    pub(in crate::game) fn player_is_singleplayer_owner(&self, player: PlayerRef) -> bool {
        let Some(owner) = &self.server.singleplayer_profile else {
            return false;
        };
        self.online_player_record(player)
            .is_some_and(|guest| owner.name.eq_ignore_ascii_case(&guest.name))
    }
}
