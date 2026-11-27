//! A server's crafting result in the menus a guest predicts with: the client never works the
//! result out itself (`CraftingMenu.slotsChanged` runs on a `ServerLevel` only), so the result
//! slot holds what the server's ContainerSetSlot put there. The wheel down over it takes one
//! craft to the inventory, and a predicted take empties the slot until the server answers.
use crate::game::server_side::native_entity_refs::tests::with_client_game;
use sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client;
use sunrise_items::items::ItemStack;
use sunrise_sim::menu::Input;

/// A guest with one oak log in its 2x2 grid and the server's result, four planks.
fn with_guest_grid(run: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>)) {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_client_game(|game| {
            let _server = logged_client(game);
            game.client.title.0 = None;
            game.craft_size = 2;
            game.craft_grid[0] = Some(ItemStack::new("minecraft:oak_log", 1));
            game.client.remote.as_mut().unwrap().craft_result =
                Some(ItemStack::new("minecraft:oak_planks", 4));
            run(game);
        })
    });
}

#[test]
#[ignore = "daily"]
fn the_wheel_finds_room_for_a_servers_crafting_result() {
    with_guest_grid(|game| {
        assert!(game.tweak_result(200));
        // InventoryMenu.quickMoveStack: the result goes to the end of the hotbar first.
        assert_eq!(game.slot_landing(200), [8]);
    });
}

#[test]
#[ignore = "daily"]
fn a_predicted_take_carries_the_servers_result_and_empties_its_slot() {
    with_guest_grid(|game| {
        assert!(game.slot_click(200, 0, Input::Pickup));
        assert_eq!(
            game.carried,
            Some(ItemStack::new("minecraft:oak_planks", 4))
        );
        assert_eq!(game.craft_result(), None);
        assert_eq!(game.craft_grid[0], None, "ResultSlot.onTake used the log");
    });
}

/// Only the crafting menus take it: a chest's slot 0 is its own first slot, in the menu
/// predicted with and in the chest after the prediction.
#[test]
#[ignore = "daily"]
fn a_chests_first_slot_keeps_its_own_item() {
    use sunrise_level::containers::BlockEntity;
    with_guest_grid(|game| {
        let cobblestone = ItemStack::new("minecraft:cobblestone", 1);
        let mut slots = vec![None; 27];
        slots[0] = Some(cobblestone.clone());
        game.world
            .block_entities
            .insert((0, 64, 0), BlockEntity::Chest(slots));
        game.container = Some((0, 64, 0));
        let mut first = None;
        game.menu_session(|menu, _, _| {
            first = menu.slot(0).cloned();
            Some(true)
        });
        assert_eq!(first, Some(cobblestone.clone()));
        assert_eq!(game.slot_view(300), Some(cobblestone));
    });
}
