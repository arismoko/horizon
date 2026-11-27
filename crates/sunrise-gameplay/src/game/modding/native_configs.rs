use mod_api::ConfigEvent;

impl crate::game::ServerGame<'_> {
    /// The server stops: its `Server` configs unload.
    pub fn native_configs_server_stop(&mut self) {
        for (config, snapshot) in self.common.native_configs.drop_server() {
            config.notify_snapshot(ConfigEvent::Unloading(self), &snapshot);
        }
    }
}

#[cfg(test)]
#[path = "native_configs/lifecycle_tests_server.rs"]
mod lifecycle_tests_server;
