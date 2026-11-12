//! A server player's ender chest (`Player.getEnderChestInventory`) by slot, for mods that copy
//! or edit it whole: player data sync, `/endersee`.
use super::{Game, commands::items::Slot, native_server::PlayerRead};
use horizon_client::items::ItemStack;
use mod_api::PlayerRef;

/// `PlayerEnderChestContainer`'s size.
const SLOTS: usize = 27;

fn numbered(items: &[Option<ItemStack>]) -> Vec<(u32, ItemStack)> {
    items
        .iter()
        .take(SLOTS)
        .enumerate()
        .filter_map(|(i, s)| Some((i as u32, s.clone().filter(|s| s.count > 0)?)))
        .collect()
}

impl Game<'_> {
    /// `PlayerEnderChestContainer.getItem` for every slot that holds something.
    pub(super) fn player_ender_chest(&self, player: PlayerRef) -> Option<Vec<(u32, ItemStack)>> {
        if self.remote.is_some() {
            return None;
        }
        Some(match self.player_handle_read(player)? {
            PlayerRead::Acting { .. } => numbered(&self.ender_items),
            PlayerRead::Guest(g) => numbered(&g.ender_items),
        })
    }

    /// `PlayerEnderChestContainer.setItem`, in the player's turn.
    pub(super) fn player_set_ender_chest_slot(
        &mut self,
        player: PlayerRef,
        slot: u32,
        stack: Option<ItemStack>,
    ) -> bool {
        if self.remote.is_some() || slot as usize >= SLOTS {
            return false;
        }
        let mut set = false;
        let ran = self.native_as_player_handle(player, |game| {
            if game.ender_items.len() < SLOTS {
                game.ender_items.resize(SLOTS, None);
            }
            set = game.slot_set(&Slot::Ender(slot as usize), stack.filter(|s| s.count > 0));
        });
        ran && set
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_guest_turn_tests::with_guests;
    use horizon_client::items::ItemStack;

    /// Ender chest slots are written and read back by number, out of the player's turn and
    /// in it, components and all.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn ender_chest_slots_are_read_and_written() {
        with_guests(|game| {
            let b = game.player(13).unwrap();
            let book = horizon_items::item_data::parse_stack(
                r#"{id:"minecraft:written_book",count:1,components:{"minecraft:written_book_content":{title:{raw:"Log"},author:"guest13",pages:[{raw:"one"}]}}}"#,
            )
            .unwrap()
            .unwrap();
            let pearls = ItemStack::new("minecraft:ender_pearl", 16);
            assert_eq!(game.player_ender_chest(b), Some(Vec::new()));
            assert!(game.player_set_ender_chest_slot(b, 26, Some(book.clone())));
            assert!(!game.player_set_ender_chest_slot(b, 27, Some(book.clone())));
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guest(13).unwrap().ender_items[26],
                Some(book.clone())
            );
            let a = game.player(12).unwrap();
            game.as_guest(12, |game| {
                assert!(game.player_set_ender_chest_slot(a, 0, Some(pearls.clone())));
                assert_eq!(game.player_ender_chest(a).unwrap(), [(0, pearls.clone())]);
                assert_eq!(game.player_ender_chest(b).unwrap(), [(26, book.clone())]);
            })
            .unwrap();
            let read = game.player_ender_chest(b).unwrap();
            assert_eq!(
                horizon_items::item_nbt::item_to_nbt(&read[0].1),
                horizon_items::item_nbt::item_to_nbt(&book)
            );
            assert!(game.player_set_ender_chest_slot(b, 26, None));
            assert_eq!(game.player_ender_chest(b), Some(Vec::new()));
        });
    }
}
