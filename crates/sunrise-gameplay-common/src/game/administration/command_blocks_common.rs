//! Common command-block and command-minecart helpers shared by server and client.
use super::minecarts::{CartKind, Minecart};
use sunrise_gameplay_common::command_blocks::state_of;
use sunrise_level::{command_block::CommandBlock, containers::BlockEntity};
use sunrise_world::chunk::BlockState;

pub type Pos = (i32, i32, i32);

impl crate::game::CommonGame<'_> {
    /// `Player.canUseGameMasterBlocks`: creative with game master commands.
    pub fn can_use_game_master_blocks(&self) -> bool {
        self.creative && self.ui.registries.permission >= 2
    }

    pub fn command_block(&self, pos: Pos) -> Option<&CommandBlock> {
        match self.world.block_entities.get(&pos)? {
            BlockEntity::CommandBlock(c) => Some(c),
            _ => None,
        }
    }

    pub fn block_state_at(&self, pos: Pos) -> Option<&'static BlockState> {
        let registry = self.pack.assets.registry;
        state_of(registry, self.world.block(pos.0, pos.1, pos.2))
    }

    pub fn edit_command_cart<R>(
        &mut self,
        id: u32,
        edit: impl FnOnce(&mut Minecart) -> R,
    ) -> Option<R> {
        let i = self
            .carts
            .iter()
            .position(|c| c.id == id && c.kind == CartKind::CommandBlock)?;
        Some(self.carts.update(i, edit))
    }
}
