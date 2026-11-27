#[test]
fn every_inventory_menu_slot_is_a_game_slot_and_45_is_the_off_hand() {
    // InventoryMenu: result, 2x2 grid, armour, 27 + 9 inventory slots, then the off hand.
    assert_eq!(
        crate::game::inventory_screen_slot("inventory", 45),
        Some(504)
    );
    let mut seen = std::collections::HashSet::new();
    for slot in 0..46 {
        let internal = sunrise_gameplay_common::game::inventory_screen_slot("inventory", slot);
        assert!(internal.is_some_and(|i| seen.insert(i)), "slot {slot}");
    }
    assert_eq!(crate::game::inventory_screen_slot("inventory", 46), None);
}

#[test]
fn remote_clicks_find_slots_past_the_old_large_chest_cutoff() {
    for (menu, internal, index) in [
        ("minecraft:generic_9x6", 18, 63),
        ("minecraft:generic_9x6", 19, 64),
        ("minecraft:generic_9x6", 0, 81),
        ("minecraft:generic_9x6", 8, 89),
        ("minecraft:generic_9x6", 353, 53),
        ("minecraft:generic_9x5", 8, 80),
        ("minecraft:generic_9x4", 8, 71),
        ("minecraft:generic_9x3", 8, 62),
        ("inventory", 504, 45),
    ] {
        assert_eq!(
            sunrise_client_gameplay::game::ClientGame::menu_slot_index(menu, internal),
            Some(index),
            "{menu}, game slot {internal}"
        );
    }
    assert_eq!(
        sunrise_client_gameplay::game::ClientGame::menu_slot_index("minecraft:generic_9x6", 354),
        None
    );
    assert_eq!(
        sunrise_client_gameplay::game::ClientGame::menu_slot_index("minecraft:generic_9x6", 504),
        None
    );
    assert_eq!(
        sunrise_client_gameplay::game::ClientGame::menu_slot_index("unknown", 0),
        None
    );
}

#[test]
fn large_mod_menu_backing_slots_never_alias_armor_station_or_mount_slots() {
    use sunrise_level::containers::ModMenuType;
    use {
        sunrise_gameplay_common::game::inventory_slots::MOD_MENU_SLOT_BASE,
        sunrise_gameplay_common::game::inventory_slots::container_slot_index,
    };

    let with_inv = ModMenuType {
        id: "test:large_with_inv",
        owner: "test",
        slots: 32_732,
        player_inventory: true,
        data_slots: 32_768,
        results: Vec::new(),
        inputs: Vec::new(),
        crafting: None,
        links: Default::default(),
        owned_slots: None,
    };
    for s in [0usize, 199, 200, 204, 300, 400, 32_731] {
        let internal = sunrise_gameplay_common::game::inventory_screen_slot_with_type(
            with_inv.id,
            s as i32,
            || Some(&with_inv),
        )
        .expect("backing slot");
        assert_eq!(internal, MOD_MENU_SLOT_BASE + s);
        assert_eq!(container_slot_index(internal), Some(s));
        assert!(internal >= MOD_MENU_SLOT_BASE);
    }
    assert_eq!(
        crate::game::inventory_screen_slot_with_type(with_inv.id, 32_732, || Some(&with_inv)),
        Some(9)
    );
    assert_eq!(
        crate::game::inventory_screen_slot_with_type(with_inv.id, 32_759, || Some(&with_inv)),
        Some(0)
    );
    assert_eq!(
        crate::game::inventory_screen_slot_with_type(with_inv.id, 32_767, || Some(&with_inv)),
        Some(8)
    );
    assert_eq!(
        crate::game::inventory_screen_slot_with_type(with_inv.id, 32_768, || Some(&with_inv)),
        None
    );

    let without_inv = ModMenuType {
        id: "test:large_no_inv",
        owner: "test",
        slots: 32_768,
        player_inventory: false,
        data_slots: 65_536,
        results: Vec::new(),
        inputs: Vec::new(),
        crafting: None,
        links: Default::default(),
        owned_slots: None,
    };
    assert_eq!(
        crate::game::inventory_screen_slot_with_type(without_inv.id, 0, || Some(&without_inv)),
        Some(MOD_MENU_SLOT_BASE)
    );
    assert_eq!(
        crate::game::inventory_screen_slot_with_type(without_inv.id, 32_767, || Some(&without_inv)),
        Some(MOD_MENU_SLOT_BASE + 32_767)
    );
    assert_eq!(
        crate::game::inventory_screen_slot_with_type(without_inv.id, 32_768, || Some(&without_inv)),
        None
    );
}
