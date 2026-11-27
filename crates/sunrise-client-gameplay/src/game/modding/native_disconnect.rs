//! A client mod leaving the world, NeoForge's `IClientCommonPacketListenerExtension.disconnect`.
//! The server half is `native_player_disconnect`.
use crate::game::ClientGame;
use mod_api::{DisconnectError, WorldEpoch};
use sunrise_game_data::{Result, text::Text};

/// A client mod's request to leave, carried out at the end of the frame.
pub(in crate::game) struct LeaveRequest {
    /// The world it was made in: a request made while that world closes is dropped.
    epoch: WorldEpoch,
    reason: Option<Text>,
}

#[cfg(feature = "client")]
impl ClientGame<'_> {
    /// Asks to leave the joined server or world at the end of this frame.
    /// Unavailable with no world joined, or while the world is already closing.
    pub(in crate::game) fn disconnect(
        &mut self,
        reason: Option<Text>,
    ) -> std::result::Result<(), DisconnectError> {
        let epoch = self.common.native_lifecycle.epoch();
        let (Some(epoch), Some(_), None) = (
            epoch,
            self.client.remote.as_ref(),
            &self.client.pending_save.0,
        ) else {
            return Err(DisconnectError::Unavailable);
        };
        self.client.native_leave = Some(LeaveRequest { epoch, reason });
        Ok(())
    }

    /// What follows the frame's render: the cursor icon, any deferred client command
    /// submissions, then a mod's `disconnect` (below).
    pub(in crate::game::client_side) fn native_frame_end(&mut self) -> Result<()> {
        sunrise_level::spans::time("pr_cursor", || self.apply_cursor_icon());
        self.drain_pending_client_commands();
        self.native_leave_if_asked()
    }

    /// The end of the frame: a mod's `disconnect` leaves now, outside every hook. With a
    /// reason, the disconnect screen shows it; with none, the pause menu's Disconnect runs. A
    /// request from a world that has closed since is dropped.
    pub(in crate::game::client_side) fn native_leave_if_asked(&mut self) -> Result<()> {
        let Some(request) = self.client.native_leave.take() else {
            return Ok(());
        };
        if self.client.remote.is_none()
            || self.common.native_lifecycle.epoch() != Some(request.epoch)
        {
            return Ok(());
        }
        let left = match request.reason {
            Some(reason) => self.leave_server(Some(reason)),
            None => self.close_world(),
        };
        // A hook the leaving fired (a disconnect or unload listener) may have asked again: that
        // request belongs to the world just closed.
        self.client.native_leave = None;
        left
    }
}
