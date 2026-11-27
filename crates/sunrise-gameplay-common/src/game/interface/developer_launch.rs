//! Developer single-player startup and network-entry policy.
use super::*;

impl Options {
    pub(in crate::game) fn validate_developer_single_player(&self) -> Result<()> {
        if self.developer_single_player
            && (self.server.is_some()
                || self.session_stdin
                || self.session_file.is_some()
                || !self.execution_role.is_client())
        {
            return Err(
                sunrise_game_data::lang::get("sunrise.developer_offline.single_player").into(),
            );
        }
        Ok(())
    }
}
