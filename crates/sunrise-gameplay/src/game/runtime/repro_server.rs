use crate::game::ServerGame;
use std::path::PathBuf;

impl ServerGame<'_> {
    #[inline(always)]
    #[allow(dead_code)]
    pub(in crate::game) fn start_recording_world_root_server(&self) -> Option<PathBuf> {
        if !self.server.world_root.as_os_str().is_empty() {
            Some(self.server.world_root.clone())
        } else {
            None
        }
    }
}
