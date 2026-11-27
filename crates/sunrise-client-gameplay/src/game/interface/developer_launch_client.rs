use super::*;

impl crate::game::ClientGame<'_> {
    /// Join single player's integrated server through memory.
    pub fn connect_local(
        &mut self,
        pipe: sunrise_protocol::connection::Pipe,
        name: &str,
    ) -> Result<()> {
        self.join_server("local", name, Some(pipe), None, false, false, None)
    }
    pub(in crate::game) fn join_server(
        &mut self,
        address: &str,
        name: &str,
        local: Option<sunrise_protocol::connection::Pipe>,
        cookies: Option<sunrise_client::net::Cookies>,
        lan: bool,
        realm: bool,
        list_name: Option<&str>,
    ) -> Result<()> {
        if local.is_none() && self.common.options.developer_single_player {
            return Err(
                sunrise_game_data::lang::get("sunrise.developer_offline.single_player").into(),
            );
        }
        self.connect_remote_session(address, name, local, cookies, lan, realm, list_name)
    }
}
