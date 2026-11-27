use super::*;

impl crate::game::ServerGame<'_> {
    #[inline(always)]
    #[allow(dead_code)]
    pub(in crate::game) fn init_remote_for_replay_server(&mut self) {
        self.server.save.0 = Arc::new(SaveDir::new(
            std::env::temp_dir().join("sunrise-replay-unused"),
            sunrise_sim::save::LevelShape::OVERWORLD,
            None,
        ));
    }
    #[inline(always)]
    #[allow(dead_code)]
    pub(in crate::game) fn install_remote_session_server(&mut self) {
        self.server.save.0 = Arc::new(SaveDir::new(
            std::env::temp_dir().join("sunrise-remote-unused"),
            sunrise_sim::save::LevelShape::OVERWORLD,
            None,
        ));
    }
}
