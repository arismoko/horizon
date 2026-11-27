use super::*;

impl crate::game::CommonGame<'_> {
    /// The game's `config/` folder beside `options.txt` (beside
    /// `server.properties` on a dedicated server), shared by every mod as on
    /// Fabric and NeoForge (`FMLPaths.CONFIGDIR`). Startup creates the folder;
    /// lending it performs no I/O.
    pub fn config_dir(&self) -> &Path {
        &self.config_root
    }

    /// No world is open: a client without a player (the title screen). A
    /// server's `Game` (the server thread, a dedicated server, a LAN host)
    /// has no player of its own, and is always in its world.
    pub fn outside_world(&self) -> bool {
        self.player.is_none() && self.options.execution_role.is_client()
    }
}

pub fn mod_data_root() -> PathBuf {
    crate::game::game_dir()
        .join(mod_list::DIR)
        .join(".sunrise-mod-data")
}
