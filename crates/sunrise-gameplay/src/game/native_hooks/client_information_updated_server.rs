use super::*;

/// The hook points. Each does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    /// A placed player's login as the mods hear it: the client information
    /// they joined with (`client_information_updated`, from the defaults),
    /// then `player_logged_in`.
    pub fn announce_login(&mut self, player: u128, name: &str) {
        if mod_api::__private::listened(mod_api::HookId::ClientInformationUpdated)
            && let Some(g) = self
                .server
                .lan
                .as_ref()
                .and_then(|host| host.hub.guests.iter().find(|g| g.uuid == player))
        {
            let entity_id = g.entity_id;
            let new = crate::game::lan::client_info(&g.info);
            // `ClientInformation.createDefault`: no skin parts, not listed.
            let defaults = sunrise_protocol::packets::configuration::ClientInformation {
                model_customisation: 0,
                allows_listing: false,
                ..sunrise_protocol::packets::configuration::ClientInformation::new(2)
            };
            let old = crate::game::lan::client_info(&defaults);
            self.native_client_information_updated(player, entity_id, name, &old, &new);
        }
        self.native_player_logged_in(player, name);
    }
}
