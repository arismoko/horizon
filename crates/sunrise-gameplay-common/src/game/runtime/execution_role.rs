pub use sunrise_gameplay_common::execution_role::ExecutionRole;

use mod_api::HostSide;

/// The hook host of `game`'s side.
pub fn host_side<'a, 'g, G: crate::game::SideGame<'g>>(game: &'a mut G) -> HostSide<'a> {
    game.host_side()
}

impl crate::game::CommonGame<'_> {
    /// This game's side: a server simulates the world, a client does not. The
    /// one source for a helper's side; `remote` says whether a client is
    /// connected, not which side this game is.
    pub fn is_logical_server(&self) -> bool {
        self.options.execution_role.is_logical_server()
    }

    /// Whether this game is a dedicated server, not the integrated server thread (which
    /// keeps `options.dedicated` set too).
    pub fn is_dedicated_server(&self) -> bool {
        self.options.execution_role.is_dedicated_server()
    }
}
