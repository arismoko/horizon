use super::*;

// Stage 2 acceptance row: cursor stone 3, empty slot, ordinary placement.
// Vanilla 26.3 AbstractContainerMenu.doClick (PICKUP) / Slot.safeInsert.
#[test]
fn dfa_baseline_cursor_placement() {
    bare_env(|env| {
        let mut menu = Menu::new(Kind::Chest(3));
        menu.carried = stack("stone", 3);
        menu.click(env, 0, 0, Input::Pickup).unwrap();
        assert_eq!(menu.carried, None);
        assert_eq!(menu.slot(0), stack("stone", 3).as_ref());
    });
}

// Stage 2 acceptance row: quick move stone 10 into room 4 and 6.
// Vanilla 26.3 AbstractContainerMenu.moveItemStackTo fills matching stacks first,
// so the empty slot 2 stays empty when the matching room is enough.
#[test]
fn dfa_baseline_quick_move() {
    bare_env(|env| {
        let mut menu = Menu::new(Kind::Chest(3));
        for slot in 0..27 {
            menu.set_slot(env, slot, stack("dirt", 64));
        }
        menu.set_slot(env, 0, stack("stone", 60));
        menu.set_slot(env, 1, stack("stone", 58));
        menu.set_slot(env, 2, None);
        menu.set_slot(env, 27, stack("stone", 10));
        menu.click(env, 27, 0, Input::QuickMove).unwrap();
        assert_eq!(menu.slot(27), None);
        assert_eq!(menu.slot(0), stack("stone", 64).as_ref());
        assert_eq!(menu.slot(1), stack("stone", 64).as_ref());
        assert_eq!(menu.carried, None);
        assert_eq!(menu.slot(2), None);
        for slot in 3..27 {
            assert_eq!(menu.slot(slot), stack("dirt", 64).as_ref());
        }
    });
}

// Stage 2 acceptance row: recipe needs two stone, source stone 2, output empty.
// Vanilla 26.3 ServerPlaceRecipe.placeRecipe stages one set; ResultSlot.onTake
// consumes one item per occupied grid cell, once, when the result is taken.
#[test]
#[ignore = "daily"]
fn dfa_baseline_recipe_placement() {
    let mut ran = false;
    with_menu(Kind::Crafting, |env, menu| {
        ran = true;
        menu.creative = false;
        menu.set_player_item(0, stack("stone", 2));
        assert_eq!(menu.slot(0), None);
        menu.place_recipe(env, "minecraft:stone_pressure_plate", false, false, None);
        assert_eq!(menu.player_item(0), None);
        assert_eq!(menu.slot(0), stack("stone_pressure_plate", 1).as_ref());
        assert_eq!(
            (1..10)
                .filter_map(|s| menu.slot(s).cloned())
                .collect::<Vec<_>>(),
            vec![stack("stone", 1).unwrap(), stack("stone", 1).unwrap()]
        );
        menu.click(env, 0, 0, Input::Pickup).unwrap();
        assert_eq!(menu.carried, stack("stone_pressure_plate", 1));
        assert!((0..10).all(|s| menu.slot(s).is_none()));
        menu.click(env, 0, 0, Input::Pickup).unwrap();
        assert_eq!(menu.carried, stack("stone_pressure_plate", 1));
        assert_eq!(menu.player_item(0), None);
    });
    assert!(
        ran,
        "requires prepared resources in SUNRISE_TEST_RESOURCES or .local/sunrise-client/26.3"
    );
}

// Stage 2 acceptance row, two items per cell: ResultSlot.onTake removes one
// item from each occupied grid cell, not the whole cell. With stone 4,
// placeRecipe with useMaxItems stages two sets (two stone per cell); taking
// the result leaves one stone per cell, and the grid offers the plate again
// (CraftingMenu.slotChangedCraftingGrid).
#[test]
#[ignore = "daily"]
fn dfa_baseline_recipe_take_consumes_one_per_cell() {
    let mut ran = false;
    with_menu(Kind::Crafting, |env, menu| {
        ran = true;
        menu.creative = false;
        menu.set_player_item(0, stack("stone", 4));
        menu.place_recipe(env, "minecraft:stone_pressure_plate", true, false, None);
        assert_eq!(menu.player_item(0), None);
        let grid = |menu: &Menu| {
            (1..10)
                .filter_map(|s| menu.slot(s).cloned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            grid(menu),
            vec![stack("stone", 2).unwrap(), stack("stone", 2).unwrap()]
        );
        assert_eq!(menu.slot(0), stack("stone_pressure_plate", 1).as_ref());
        menu.click(env, 0, 0, Input::Pickup).unwrap();
        assert_eq!(menu.carried, stack("stone_pressure_plate", 1));
        assert_eq!(
            grid(menu),
            vec![stack("stone", 1).unwrap(), stack("stone", 1).unwrap()]
        );
        assert_eq!(menu.slot(0), stack("stone_pressure_plate", 1).as_ref());
    });
    assert!(
        ran,
        "requires prepared resources in SUNRISE_TEST_RESOURCES or .local/sunrise-client/26.3"
    );
}

/// AnvilScreen.slotChanged inside the click that puts an item in (a server's client predicting
/// with the screen open): the name box takes the input's name and onNameChanged gives the menu
/// that name, sent before the click; it is the input's own, so the result stays empty. A plain
/// item's name goes to the menu blank. Without the screen (a server's own menu) nothing renames,
/// and slots set before the screen listens are no change it hears of.
#[test]
fn an_anvil_screen_names_the_input_a_click_puts_in() {
    bare_env(|env| {
        let named = |name: &str| {
            let mut paper = stack("paper", 1).unwrap();
            paper.data_mut().custom_name = Some(sunrise_core::nbt::Tag::String(name.into()));
            paper
        };
        let mut menu = Menu::new(Kind::Anvil);
        menu.anvil.screen = true;
        menu.carried = Some(named("Note"));
        menu.click(env, 0, 0, Input::Pickup).unwrap();
        assert_eq!(menu.anvil.renamed, ["Note"]);
        assert_eq!(menu.anvil.name.as_deref(), Some("Note"));
        assert_eq!(menu.slot(2), None);

        let mut plain = Menu::new(Kind::Anvil);
        plain.anvil.screen = true;
        plain.carried = stack("paper", 1);
        plain.click(env, 0, 0, Input::Pickup).unwrap();
        assert_eq!(plain.anvil.renamed, [""]);

        let mut server = Menu::new(Kind::Anvil);
        server.carried = Some(named("Note"));
        server.click(env, 0, 0, Input::Pickup).unwrap();
        assert!(server.anvil.renamed.is_empty());

        let mut loaded = Menu::new(Kind::Anvil);
        loaded.set_slot(env, 0, Some(named("Note")));
        loaded.anvil.seen_input = loaded.slot(0).cloned();
        loaded.anvil.screen = true;
        loaded.carried = stack("stone", 1);
        loaded.click(env, 1, 0, Input::Pickup).unwrap();
        assert!(loaded.anvil.renamed.is_empty());
    });
}

/// SmithingMenu.createResult looks a recipe up only on a ServerLevel: a server's client's menu
/// shows no result for a template, a diamond sword and a netherite ingot, nor a recipe error.
#[test]
#[ignore = "daily"]
fn a_clients_smithing_table_works_out_no_result() {
    let mut ran = false;
    with_menu(Kind::Smithing, |env, menu| {
        ran = true;
        menu.on_client = true;
        for (slot, item) in [
            (0, "netherite_upgrade_smithing_template"),
            (1, "diamond_sword"),
            (2, "netherite_ingot"),
        ] {
            menu.carried = stack(item, 1);
            menu.click(env, slot, 0, Input::Pickup).unwrap();
            assert_eq!(menu.slot(slot as usize), stack(item, 1).as_ref());
        }
        assert_eq!(menu.slot(3), None);
        assert_eq!(menu.data[0], 0);
    });
    assert!(
        ran,
        "requires prepared resources in SUNRISE_TEST_RESOURCES or .local/sunrise-client/26.3"
    );
}

/// StonecutterMenu.setupResultSlot finds no recipe on a client (its SelectableRecipe entries carry none):
/// a recipe button leaves the result empty, and a shift-click on the result the server sent crafts once,
/// as vanilla's client predicts, not until the input runs out.
#[test]
#[ignore = "daily"]
fn a_clients_stonecutter_crafts_what_the_server_shows_once() {
    let mut ran = false;
    with_menu(Kind::Stonecutter, |env, menu| {
        ran = true;
        menu.on_client = true;
        menu.carried = stack("stone", 8);
        menu.click(env, 0, 0, Input::Pickup).unwrap();
        assert!(menu.button(env, 0));
        assert_eq!(menu.slot(1), None);
        // The server's ContainerSetSlot shows the result.
        menu.set_slot(env, 1, stack("stone_slab", 2));
        menu.click(env, 1, 0, Input::QuickMove).unwrap();
        assert_eq!(menu.slot(0), stack("stone", 7).as_ref());
        assert_eq!(menu.slot(1), None);
    });
    assert!(
        ran,
        "requires prepared resources in SUNRISE_TEST_RESOURCES or .local/sunrise-client/26.3"
    );
}

/// CraftingMenu.slotsChanged works out a result only on a ServerLevel: a server's client's menu
/// leaves the result slot to the server, so two stone placed in the grid show no plate there.
#[test]
#[ignore = "daily"]
fn a_clients_crafting_grid_works_out_no_result() {
    let mut ran = false;
    with_menu(Kind::Crafting, |env, menu| {
        ran = true;
        menu.on_client = true;
        menu.carried = stack("stone", 2);
        menu.click(env, 1, 1, Input::Pickup).unwrap();
        menu.click(env, 2, 1, Input::Pickup).unwrap();
        assert_eq!(menu.slot(1), stack("stone", 1).as_ref());
        assert_eq!(menu.slot(2), stack("stone", 1).as_ref());
        assert_eq!(menu.slot(0), None);
    });
    assert!(
        ran,
        "requires prepared resources in SUNRISE_TEST_RESOURCES or .local/sunrise-client/26.3"
    );
}
