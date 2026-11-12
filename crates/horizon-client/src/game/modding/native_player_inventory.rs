//! A server player's inventory by vanilla's slot numbers (`Inventory.getItem`/`setItem`), for
//! mods that copy or edit it whole: player data sync, `/invsee`.
use super::{Game, commands::items::Slot, native_server::PlayerRead};
use horizon_client::items::ItemStack;
use horizon_items::equipment::{BODY, Equipment, SADDLE};
use mod_api::PlayerRef;

/// `Inventory`'s slot numbers past the 36 main slots (`EQUIPMENT_SLOT_MAPPING`).
const SLOTS: u32 = 43;

/// The slot the game keeps `Inventory` slot `slot` in.
fn inventory_slot(slot: u32) -> Option<Slot> {
    Some(match slot {
        0..36 => Slot::Inv(slot as usize),
        // `armor` is head first: 36 feet is its last.
        36..40 => Slot::Armor((39 - slot) as usize),
        40 => Slot::Offhand,
        41 => Slot::Equipment(BODY),
        42 => Slot::Equipment(SADDLE),
        _ => return None,
    })
}

/// The non-empty slots, by number, of an inventory laid out as the game keeps it.
fn numbered(
    inventory: &[Option<ItemStack>],
    armor: &[Option<ItemStack>; 4],
    offhand: &Option<ItemStack>,
    equipment: &Equipment,
) -> Vec<(u32, ItemStack)> {
    (0..SLOTS)
        .filter_map(|slot| {
            let stack = match inventory_slot(slot)? {
                Slot::Inv(i) => inventory.get(i).cloned().flatten(),
                Slot::Armor(i) => armor[i].clone(),
                Slot::Offhand => offhand.clone(),
                Slot::Equipment(i) => equipment.get(i).cloned(),
                _ => None,
            };
            Some((slot, stack.filter(|s| s.count > 0)?))
        })
        .collect()
}

impl Game<'_> {
    /// `Inventory.getItem` for every slot that holds something.
    pub(super) fn player_inventory_slots(&self, player: PlayerRef) -> Option<Vec<(u32, ItemStack)>> {
        if self.remote.is_some() {
            return None;
        }
        Some(match self.player_handle_read(player)? {
            PlayerRead::Acting { .. } => {
                numbered(&self.inventory, &self.armor, &self.offhand, &self.equipment)
            }
            PlayerRead::Guest(g) => numbered(&g.inventory, &g.armor, &g.offhand, &g.equipment),
        })
    }

    /// `Inventory.setItem`, in the player's turn: the tick's inventory sync tells the client.
    pub(super) fn player_set_inventory_slot(
        &mut self,
        player: PlayerRef,
        slot: u32,
        stack: Option<ItemStack>,
    ) -> bool {
        let Some(slot) = inventory_slot(slot).filter(|_| self.remote.is_none()) else {
            return false;
        };
        let mut set = false;
        let ran = self.native_as_player_handle(player, |game| {
            set = game.slot_set(&slot, stack.filter(|s| s.count > 0));
            if set && game.player.is_some() {
                game.check_inventory_trigger();
            }
        });
        ran && set
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_guest_turn_tests::with_guests;
    use horizon_client::items::ItemStack;

    fn guest<'a>(game: &'a super::Game<'_>, conn: u32) -> &'a horizon_client::lan::guest::Guest {
        game.lan.as_ref().unwrap().hub.guest(conn).unwrap()
    }

    /// Slots go by `Inventory`'s numbers both ways, out of the player's turn and in it; the
    /// inventory menu's sync then has the changed slots to send, and a stack with components
    /// comes back as it went in.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn inventory_slots_are_read_and_written_by_vanillas_numbers() {
        with_guests(|game| {
            let b = game.player(13).unwrap();
            let named = horizon_items::item_data::parse_stack(
                r#"{id:"minecraft:diamond_sword",count:1,components:{"minecraft:damage":5,"minecraft:custom_name":"Blade","minecraft:custom_data":{husksync:1b}}}"#,
            )
            .unwrap()
            .unwrap();
            let boots = ItemStack::new("minecraft:iron_boots", 1);
            let shield = ItemStack::new("minecraft:shield", 1);
            assert_eq!(game.player_inventory_slots(b), Some(Vec::new()));
            assert!(game.player_set_inventory_slot(b, 8, Some(named.clone())));
            assert!(game.player_set_inventory_slot(b, 36, Some(boots.clone())));
            assert!(game.player_set_inventory_slot(b, 40, Some(shield.clone())));
            assert!(!game.player_set_inventory_slot(b, 43, Some(shield.clone())));
            let g = guest(game, 13);
            assert_eq!(g.inventory[8], Some(named.clone()));
            assert_eq!(g.armor[3], Some(boots.clone()), "36 is the feet");
            assert_eq!(g.offhand, Some(shield.clone()));
            // The inventory menu's hotbar 8 is slot 44, feet 8, off hand 45: unsent yet.
            for menu in [44, 8, 45] {
                assert_ne!(g.menu_slot(menu), g.menu_sent[menu], "menu slot {menu}");
            }
            let read = game.player_inventory_slots(b).unwrap();
            assert_eq!(read, [(8, named.clone()), (36, boots), (40, shield)]);
            assert_eq!(
                horizon_items::item_nbt::item_to_nbt(&read[0].1),
                horizon_items::item_nbt::item_to_nbt(&named)
            );
            let a = game.player(12).unwrap();
            game.as_guest(12, |game| {
                assert!(game.player_set_inventory_slot(a, 39, Some(named.clone())));
                assert_eq!(game.armor[0], Some(named.clone()), "39 is the head");
                assert_eq!(game.player_inventory_slots(a).unwrap(), [(39, named.clone())]);
                assert!(game.player_set_inventory_slot(b, 8, None));
            })
            .unwrap();
            assert_eq!(guest(game, 13).inventory[8], None);
            assert_eq!(guest(game, 12).armor[0], Some(named));
        });
    }
}
