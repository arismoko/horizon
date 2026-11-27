//! A player's inventory by vanilla's slot numbers (`Inventory.getItem`/`setItem`).
//! Reads lend the current slots; writes go through the authoritative server.
use mod_api::{PlayerDataRefusal, PlayerRef};
use sunrise_gameplay_common::game::native_server::{PlayerRead, SLOTS, inventory_slot};
use sunrise_items::items::ItemStack;

/// The authority refusals shared by writes to a server player's own state.
pub(in crate::game) enum PlayerWriteRefusal {
    Stale,
    Client,
    Dead,
}

impl From<PlayerWriteRefusal> for PlayerDataRefusal {
    fn from(error: PlayerWriteRefusal) -> Self {
        match error {
            PlayerWriteRefusal::Stale => Self::Stale,
            PlayerWriteRefusal::Client => Self::Client,
            PlayerWriteRefusal::Dead => Self::Dead,
        }
    }
}

impl crate::game::ServerGame<'_> {
    /// Why a `give_stack` action is refused before it starts: a client, a handle
    /// with no online player, or a dead player.
    pub(in crate::game) fn player_write_gate(
        &self,
        player: PlayerRef,
    ) -> Result<(), PlayerWriteRefusal> {
        if !self.common.is_logical_server() {
            return Err(PlayerWriteRefusal::Client);
        }
        match self.player_handle_read(player) {
            None => Err(PlayerWriteRefusal::Stale),
            Some(PlayerRead::Guest(g)) if g.dead => Err(PlayerWriteRefusal::Dead),
            Some(_) => Ok(()),
        }
    }

    /// `Inventory.setItem`, in the player's turn: the tick's inventory sync tells the client.
    pub(in crate::game) fn player_set_inventory_slot(
        &mut self,
        player: PlayerRef,
        slot: u32,
        stack: Option<ItemStack>,
    ) -> Result<(), PlayerDataRefusal> {
        let turn = self.native_resident_player_turn(player)?;
        let target = inventory_slot(slot).ok_or(PlayerDataRefusal::SlotOutOfRange {
            slot,
            last: SLOTS - 1,
        })?;
        let set = turn.run(self, |game| {
            let set = game.slot_set(&target, stack.filter(|s| s.count > 0));
            if set && game.player.is_some() {
                game.after_commit(
                    sunrise_gameplay_common::game::transactions::After::InventoryTrigger(
                        player.uuid(),
                    ),
                );
            }
            set
        });
        if set == Some(true) {
            Ok(())
        } else {
            Err(PlayerDataRefusal::Stale)
        }
    }
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
