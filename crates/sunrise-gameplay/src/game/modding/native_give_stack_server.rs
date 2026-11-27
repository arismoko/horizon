//! Server-side stack giving for native mods (`player_give_stack`).
use crate::game::ServerGame;
use mod_api::{PlayerDataRefusal, PlayerRef};
use sunrise_items::items::ItemStack;
use sunrise_sim::lan::playerdata;

impl ServerGame<'_> {
    /// `GiveCommand.giveItem` for one stack: `Inventory.add`, the rest thrown for the player
    /// alone to pick up in their level, even when the hook is running in another one.
    pub(in crate::game) fn player_give_stack(
        &mut self,
        player: PlayerRef,
        stack: ItemStack,
    ) -> Result<Option<ItemStack>, PlayerDataRefusal> {
        self.player_write_gate(player)?;
        let owner = Some(playerdata::uuid_ints(player.uuid()));
        let mut rest = None;
        let ran = self.native_as_player_handle(player, |game| {
            rest = game.give_acting_stack(stack, owner);
            // containerMenu.broadcastChanges: InventoryChangeTrigger.
            if game.player.is_some() {
                game.after_commit(
                    sunrise_gameplay_common::game::transactions::After::InventoryTrigger(
                        player.uuid(),
                    ),
                );
            }
        });
        if ran {
            Ok(rest)
        } else {
            Err(PlayerDataRefusal::Stale)
        }
    }
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
