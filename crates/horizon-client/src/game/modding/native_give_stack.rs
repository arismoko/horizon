//! Whole stacks for server mods: one given to a player as `/give` gives it
//! (`player_give_stack`), and one read from SNBT as a saved item is (`item_from_snbt`).
use super::Game;
use horizon_client::{items::ItemStack, lan::playerdata};
use mod_api::PlayerRef;

impl Game<'_> {
    /// `GiveCommand.giveItem` for one stack: `Inventory.add`, the rest thrown for the player
    /// alone to pick up, in the player's turn.
    pub(super) fn player_give_stack(&mut self, player: PlayerRef, stack: ItemStack) -> bool {
        if self.remote.is_some() || stack.count == 0 {
            return false;
        }
        let owner = Some(playerdata::uuid_ints(player.uuid()));
        self.native_as_player_handle(player, |game| {
            game.give_acting_stack(stack, owner);
            // containerMenu.broadcastChanges: InventoryChangeTrigger.
            if game.player.is_some() {
                game.check_inventory_trigger();
            }
        })
    }

    /// `ItemStack.CODEC` reading the compound `snbt` stands for (`TagParser`).
    pub(super) fn item_from_snbt(&self, snbt: &str) -> Option<ItemStack> {
        let stack = horizon_items::item_data::parse_stack(snbt).ok()??;
        self.items().get(&stack.item).is_some().then_some(stack)
    }
}

#[cfg(test)]
mod tests {
    use super::super::api_values::{TryIntoApi, TryIntoEngine};
    use super::super::native_guest_turn_tests::with_guests;
    use horizon_client::items::ItemStack;
    use horizon_core::nbt::parse_snbt;
    use horizon_items::item_nbt::item_to_nbt_with_items;

    /// What a mod reads from SNBT and writes back is the compound `ItemStack.CODEC` started
    /// from, through the API's own stack and back.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn an_snbt_item_goes_through_the_api_unchanged() {
        with_guests(|game| {
            for snbt in [
                r#"{id:"minecraft:diamond_sword",count:1,components:{"minecraft:damage":5,"minecraft:enchantments":{"minecraft:sharpness":3},"minecraft:custom_name":"Blade","minecraft:lore":["one","two"],"minecraft:custom_data":{husksync:{v:1b}}}}"#,
                r#"{id:"minecraft:stone",count:64}"#,
                r#"{id:"minecraft:potion",count:1,components:{"minecraft:potion_contents":{potion:"minecraft:strong_healing"}}}"#,
                r#"{id:"minecraft:written_book",count:1,components:{"minecraft:written_book_content":{title:{raw:"Log"},author:"guest13",pages:[{raw:"one"}]}}}"#,
            ] {
                let engine = game.item_from_snbt(snbt).unwrap_or_else(|| panic!("{snbt}"));
                let api = engine.try_into_api().unwrap();
                let back = api.try_into_engine(game.items()).unwrap();
                let (tag, _) = parse_snbt(snbt).unwrap();
                assert_eq!(item_to_nbt_with_items(&back, game.items()), tag, "{snbt}");
                assert_eq!(back, engine, "{snbt}");
            }
            assert_eq!(game.item_from_snbt("stone").map(|s| s.count), None);
            assert_eq!(game.item_from_snbt(r#"{id:"stone"}"#).map(|s| s.count), Some(1));
            assert!(game.item_from_snbt(r#"{id:"minecraft:air",count:1}"#).is_none());
            assert!(game.item_from_snbt(r#"{id:"nomod:thing",count:1}"#).is_none());
        });
    }

    /// A given stack goes into the inventory where it fits, components and all; the rest
    /// is thrown for the player alone, with no pickup delay.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_given_stack_fills_the_inventory_then_drops_for_its_player() {
        with_guests(|game| {
            let b = game.player(13).unwrap();
            let named = game
                .item_from_snbt(r#"{id:"minecraft:stone",count:10,components:{"minecraft:custom_name":"Rock"}}"#)
                .unwrap();
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |g| {
                    let dirt = Some(ItemStack::new("minecraft:dirt", 64));
                    g.inventory.iter_mut().for_each(|s| *s = dirt.clone());
                    g.inventory[5] = Some(ItemStack {
                        count: 60,
                        ..named.clone()
                    });
                })
                .unwrap();
            let items_before = game.dropped.len();
            assert!(game.player_give_stack(b, named.clone()));
            let g = game.lan.as_ref().unwrap().hub.guest(13).unwrap();
            assert_eq!(g.inventory[5].as_ref().map(|s| s.count), Some(64));
            assert_eq!(game.dropped.len(), items_before + 1);
            let dropped = game.dropped.last().unwrap();
            assert_eq!(dropped.stack.count, 6);
            assert_eq!(dropped.stack.extra, named.extra);
            assert_eq!(dropped.pickup_delay, 0, "no pickup delay");
            assert_eq!(
                dropped.target,
                Some(horizon_client::lan::playerdata::uuid_ints(13)),
                "only its player picks it up"
            );
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |g| g.inventory[7] = None)
                .unwrap();
            assert!(game.player_give_stack(b, named.clone()));
            let g = game.lan.as_ref().unwrap().hub.guest(13).unwrap();
            assert_eq!(g.inventory[7], Some(named));
            assert_eq!(game.dropped.len(), items_before + 1);
        });
    }
}
