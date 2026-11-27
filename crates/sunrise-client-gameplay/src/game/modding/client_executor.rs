//! Mod-owned work submitted to the client application's packet/task drain.
use crate::game::ClientGame;
use mod_api::ClientExecutor;
use sunrise_level::crash;

impl ClientGame<'_> {
    pub fn client_executor(&self) -> Option<ClientExecutor> {
        if !self.common.native_window() {
            return None;
        }
        self.client
            .client_tasks
            .0
            .executor(crash::current_native_mod()?)
    }

    pub(in crate::game::client_side) fn close_client_tasks(&mut self) {
        self.client.client_tasks.0.close();
        self.client.pending_client_tasks.0.clear();
    }
}
