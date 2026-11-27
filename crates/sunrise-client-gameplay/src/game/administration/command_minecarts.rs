//! `MinecartCommandBlock`: its `BaseCommandBlock` run by a powered activator
//! rail (`performCommand`, `CloseableCommandBlockSource`), the entity data
//! that carries its command and last output to clients, and the edit screen's
//! `ServerboundSetCommandMinecartPacket` with `handleSetCommandMinecart`.
use super::{command_blocks::CommandBlockScreen, minecarts::CartKind};
use sunrise_game_data::text::Text;
use sunrise_level::command_block::Mode;
use sunrise_protocol::packets::play::{self, Serverbound as S};

impl crate::game::ClientGame<'_> {
    /// `LocalPlayer.openMinecartCommandBlock`: the editor over the cart's
    /// command (`MinecartCommandBlockEditScreen`).
    pub(in crate::game::client_side) fn open_cart_command_block(&mut self, id: u32) {
        let Some(block) = self
            .common
            .carts
            .iter()
            .find(|c| c.id == id && c.kind == CartKind::CommandBlock)
            .map(|c| c.command.clone())
        else {
            return;
        };
        self.open_edit_screen((0, 0, 0), Some(id), block, Mode::Redstone, false);
    }
    /// Edit screen's Done for a cart: updates local track flag and sends `SetCommandMinecart`.
    pub(in crate::game::client_side) fn set_command_minecart(
        &mut self,
        id: u32,
        s: &CommandBlockScreen,
    ) {
        debug_assert!(
            !self.is_logical_server(),
            "set_command_minecart called on Server role"
        );
        if !self.common.can_use_game_master_blocks() {
            self.client_system_message(Text::translate("advMode.notAllowed", &[]));
            return;
        }
        let command = s.command.value().to_owned();
        let track_output = s.track_output;
        // The output button set the cart's own flag as it was pressed.
        self.common
            .edit_command_cart(id, |c| c.command.track_output = track_output);
        self.send(S::SetCommandMinecart(play::SetCommandMinecart {
            entity: id as i32,
            command,
            track_output,
        }));
        if !track_output {
            self.common
                .edit_command_cart(id, |c| c.command.last_output = None);
        }
    }
}
