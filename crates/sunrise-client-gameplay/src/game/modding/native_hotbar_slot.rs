//! A client mod's hotbar selection (`ClientHost::select_hotbar_slot`): the local player picks a
//! slot as a number key does, and the game's own carried-item path tells the server when it does
//! for a key press. Nothing is sent here.
use crate::game::ClientGame;
use mod_api::HotbarSlotError;

#[cfg(feature = "client")]
impl ClientGame<'_> {
    /// `Inventory.setSelectedSlot` for a client mod: `slot` (0 to 8) becomes the hotbar slot the
    /// local player holds. Refused above 8, without a client world, and for a spectator (its
    /// number keys open the spectator menu), with the selection unchanged. The packet follows the
    /// game's key-press path: `MultiPlayerGameMode.ensureHasSentCarriedItem` at the next tick's
    /// start at the latest, and before an attack, a use, an interact, a release, a drop and a
    /// continued block break.
    pub fn select_hotbar_slot(&mut self, slot: u8) -> Result<(), HotbarSlotError> {
        if slot > 8 {
            return Err(HotbarSlotError::OutOfRange { slot });
        }
        if self.common.player.is_none() {
            return Err(HotbarSlotError::NoWorld);
        }
        if self.common.spectator() {
            return Err(HotbarSlotError::Refused);
        }
        self.common.selected = usize::from(slot);
        Ok(())
    }
}
