#[test]
fn the_creative_sync_covers_inventory_menu_slots_5_to_45_once_each() {
    // CreativeInventoryListener sends every InventoryMenu slot but the crafting ones: the
    // armour (5..=8), the inventory (9..=44) and the off hand (45).
    let stack = |item: &str| Some(sunrise_items::items::ItemStack::new(item, 1));
    let inventory: Vec<_> = (0..36)
        .map(|i| stack(&format!("minecraft:item_{i}")))
        .collect();
    let offhand = stack("minecraft:shield");
    let armor = [
        stack("minecraft:diamond_helmet"),
        stack("minecraft:diamond_chestplate"),
        stack("minecraft:diamond_leggings"),
        stack("minecraft:diamond_boots"),
    ];
    let stacks =
        sunrise_gameplay_common::game::remote::creative_sync_stacks(&inventory, &offhand, &armor);
    assert_eq!(
        stacks.len(),
        sunrise_gameplay_common::game::remote::CREATIVE_SYNCED
    );
    let mut slots: Vec<i16> = (0..stacks.len())
        .map(sunrise_gameplay_common::game::remote::creative_sync_slot)
        .collect();
    // Each entry goes to the menu slot that holds that stack.
    for (i, &slot) in slots.iter().enumerate() {
        let game_slot =
            sunrise_gameplay_common::game::inventory_screen_slot("inventory", i32::from(slot))
                .unwrap();
        let expected = match game_slot {
            0..36 => &inventory[game_slot],
            504 => &offhand,
            500..504 => &armor[game_slot - 500],
            _ => panic!("entry {i} went to game slot {game_slot}"),
        };
        assert_eq!(&stacks[i], expected, "entry {i}, menu slot {slot}");
    }
    slots.sort();
    assert_eq!(slots, (5..=45).collect::<Vec<i16>>());
}

#[test]
fn a_server_slot_update_leaves_an_unsent_creative_click_to_be_sent() {
    let stack = |item: &str| Some(sunrise_items::items::ItemStack::new(item, 1));
    let none = || vec![None; sunrise_gameplay_common::game::remote::CREATIVE_SYNCED];
    // Synced: nothing. A click puts a helmet on (entry 37, not sent yet), then the server
    // sets hotbar slot 0 (a pickup): only that entry counts as synced.
    let mut synced = none();
    let mut before = none();
    before[37] = stack("minecraft:diamond_helmet");
    let mut now = before.clone();
    now[0] = stack("minecraft:stick");
    sunrise_gameplay_common::game::remote::mark_synced(&mut synced, &before, now.clone());
    assert_eq!(synced[0], now[0]);
    assert_eq!(
        synced[37], None,
        "the helmet still differs, so the next tick sends it"
    );
}

mod give_stack {

    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use mod_api::CreativeGiveError;
    use sunrise_command::args::GameMode;
    use sunrise_core::nbt::{Compound, JavaString, Tag};
    use sunrise_gameplay_common::game::api_values::test_snapshot;
    use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
    use sunrise_protocol::item::{Component, Stack};
    use sunrise_protocol::packets::play::Serverbound as S;
    use {
        crate::game::client_side::native_hotbar_slot::hotbar_slot_tests::logged_in,
        crate::game::client_side::native_hotbar_slot::hotbar_slot_tests::sent,
    };

    fn api(item: &str, count: u32, components: &[(&str, Tag)]) -> mod_api::ItemStack {
        let mut stack =
            mod_api::ItemStack::new(mod_api::__private::item_key(item).unwrap(), count).unwrap();
        if !components.is_empty() {
            stack
                .set_components(
                    components
                        .iter()
                        .map(|(id, tag)| (JavaString::from(*id), tag.clone()))
                        .collect(),
                )
                .unwrap();
        }
        stack
    }

    fn named(item: &str, count: u32, name: &str) -> mod_api::ItemStack {
        api(
            item,
            count,
            &[("minecraft:custom_name", Tag::String(name.into()))],
        )
    }

    fn engine(item: &str, count: u32) -> Option<sunrise_items::items::ItemStack> {
        Some(sunrise_items::items::ItemStack::new(item, count))
    }

    /// The `SetCreativeModeSlot` packets among `packets`, as the server decodes them.
    fn creative_slots(packets: &[S]) -> Vec<(i16, Option<Stack>)> {
        packets
            .iter()
            .filter_map(|packet| match packet {
                S::SetCreativeModeSlot(p) => Some((p.slot, p.item.clone())),
                _ => None,
            })
            .collect()
    }

    /// `name`'s network id in `registry`, from the tables the server gave the client.
    fn net_id(
        game: &sunrise_client_gameplay::game::ClientGame<'_>,
        registry: &str,
        name: &str,
    ) -> i32 {
        let r = game.client.remote.as_ref().unwrap();
        r.builtin
            .get(registry)
            .and_then(|ids| ids.id(name))
            .or_else(|| r.registries.id(registry, name))
            .unwrap_or_else(|| panic!("{registry} has no {name}"))
    }

    /// The decoded slot packets as (wire slot, item, count), components left out.
    fn slots_and_counts(packets: &[(i16, Option<Stack>)]) -> Vec<(i16, i32, i32)> {
        let mut out: Vec<_> = packets
            .iter()
            .map(|(slot, stack)| {
                let stack = stack.as_ref().expect("a filled slot");
                (*slot, stack.item, stack.count)
            })
            .collect();
        out.sort();
        out
    }

    /// The one slot packet in `packets`, filled and removing no component.
    fn only_slot(packets: &[(i16, Option<Stack>)]) -> (i16, &Stack) {
        let [(slot, Some(stack))] = packets else {
            panic!("one filled slot packet: {packets:?}");
        };
        assert!(stack.removed.is_empty(), "{stack:?}");
        (*slot, stack)
    }

    /// The tick's sync, with no menu open, sends no creative slot after a give.
    fn tick_sends_nothing(
        game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
        reader: &mut sunrise_protocol::connection::ConnReader,
    ) {
        game.remote_send_state(0);
        let again = creative_slots(&sent(reader));
        assert!(again.is_empty(), "{again:?}");
    }

    fn component<'a>(
        game: &sunrise_client_gameplay::game::ClientGame<'_>,
        stack: &'a Stack,
        name: &str,
    ) -> &'a Component {
        let id = net_id(game, "minecraft:data_component_type", name);
        stack
            .added
            .iter()
            .find_map(|(c, value)| (*c == id).then_some(value))
            .unwrap_or_else(|| panic!("no {name} in {stack:?}"))
    }

    #[test]
    #[ignore = "daily"]
    fn a_refused_give_changes_and_sends_nothing() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::remote::tests::creative_sync_tests::give_stack::a_refused_give_changes_and_sends_nothing",
            || {
                with_client_game(|game| {
                    let stone = api("minecraft:stone", 1, &[]);
                    let before_login = mod_api::__private::game::player_ref(
                        mod_api::EntityRef::new(mod_api::WorldEpoch::new(1), 1),
                        1,
                    );
                    assert_eq!(
                        before_login.give_creative(game, &stone),
                        Err(CreativeGiveError::Unavailable)
                    );
                    let mut reader = logged_in(game);
                    let local = game.local_player().unwrap();
                    let stale = mod_api::__private::game::player_ref(
                        mod_api::EntityRef::new(
                            mod_api::WorldEpoch::new(local.entity().epoch().get().wrapping_add(1)),
                            local.entity().id(),
                        ),
                        local.uuid(),
                    );
                    let inventory = game.inventory.clone();
                    assert_eq!(
                        stale.give_creative(game, &stone),
                        Err(CreativeGiveError::Stale)
                    );
                    assert_eq!(
                        local.give_creative(game, &stone),
                        Err(CreativeGiveError::NotCreative)
                    );
                    game.set_local_mode(GameMode::Creative);
                    assert_eq!(
                        local.give_creative(game, &api("minecraft:stone", 65, &[])),
                        Err(CreativeGiveError::Stack(
                            mod_api::StackWriteError::CountExceeded { maximum: 64 }
                        ))
                    );
                    assert_eq!(
                        local.give_creative(
                            game,
                            &api(
                                "minecraft:iron_sword",
                                1,
                                &[("minecraft:max_stack_size", Tag::Int(16))]
                            )
                        ),
                        Err(CreativeGiveError::Stack(
                            mod_api::StackWriteError::StackableDamageable
                        ))
                    );
                    let synced = game.client.remote.as_ref().unwrap().synced.clone();
                    game.client.remote.as_mut().unwrap().features = Vec::<String>::new().into();
                    assert_eq!(
                        local.give_creative(game, &stone),
                        Err(CreativeGiveError::FeatureDisabled)
                    );
                    assert_eq!(game.client.remote.as_ref().unwrap().synced, synced);
                    game.client.remote.as_mut().unwrap().features =
                        ["minecraft:vanilla".to_owned()].into();
                    game.options.execution_role =
                        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
                    assert_eq!(
                        local.give_creative(game, &stone),
                        Err(CreativeGiveError::Unavailable)
                    );
                    game.options.execution_role =
                        sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
                    assert_eq!(game.inventory, inventory);
                    assert!(creative_slots(&sent(&mut reader)).is_empty());
                });
            },
        );
    }

    /// Components reach the slot and the server unchanged, and the slot goes out at once both
    /// with nothing open and while a server menu is open (whose end-of-tick sync skips it);
    /// the next tick's sync, with no menu open, does not send it again.
    #[test]
    #[ignore = "daily"]
    fn a_give_keeps_components_and_sends_its_slots_at_once() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::remote::tests::creative_sync_tests::give_stack::a_give_keeps_components_and_sends_its_slots_at_once",
            || {
                with_client_game(|game| {
                    let mut reader = logged_in(game);
                    // The enchantment registry a server sends in configuration.
                    std::sync::Arc::make_mut(&mut game.client.remote.as_mut().unwrap().registries)
                        .add(sunrise_protocol::packets::configuration::RegistryData {
                            registry: "minecraft:enchantment".into(),
                            entries: vec![
                                sunrise_protocol::packets::configuration::RegistryEntry {
                                    id: "minecraft:sharpness".into(),
                                    data: None,
                                },
                            ],
                        });
                    game.set_local_mode(GameMode::Creative);
                    game.selected = 0;
                    let local = game.local_player().unwrap();

                    let mut sword = api(
                        "minecraft:iron_sword",
                        1,
                        &[("minecraft:enchantments", {
                            let mut levels = Compound::new();
                            levels.insert("minecraft:sharpness".into(), Tag::Int(3));
                            Tag::Compound(levels)
                        })],
                    );
                    sword.set_damage(11);
                    assert_eq!(local.give_creative(game, &sword), Ok(None));
                    assert_eq!(game.inventory[0].as_ref().map(test_snapshot), Some(sword));
                    let packets = creative_slots(&sent(&mut reader));
                    let (slot, stack) = only_slot(&packets);
                    assert_eq!(
                        (slot, stack.item, stack.count),
                        (
                            36,
                            net_id(game, "minecraft:item", "minecraft:iron_sword"),
                            1
                        )
                    );
                    assert_eq!(stack.added.len(), 2, "{stack:?}");
                    assert_eq!(
                        component(game, stack, "minecraft:damage"),
                        &Component::Int(11)
                    );
                    assert_eq!(
                        component(game, stack, "minecraft:enchantments"),
                        &Component::Enchantments(vec![(
                            net_id(game, "minecraft:enchantment", "minecraft:sharpness"),
                            3
                        )])
                    );
                    tick_sends_nothing(game, &mut reader);

                    let potion = api(
                        "minecraft:potion",
                        1,
                        &[("minecraft:potion_contents", {
                            let mut contents = Compound::new();
                            contents
                                .insert("potion".into(), Tag::String("minecraft:swiftness".into()));
                            Tag::Compound(contents)
                        })],
                    );
                    assert_eq!(local.give_creative(game, &potion), Ok(None));
                    assert_eq!(game.inventory[1].as_ref().map(test_snapshot), Some(potion));
                    let packets = creative_slots(&sent(&mut reader));
                    let (slot, stack) = only_slot(&packets);
                    assert_eq!(
                        (slot, stack.item, stack.count),
                        (37, net_id(game, "minecraft:item", "minecraft:potion"), 1)
                    );
                    assert_eq!(stack.added.len(), 1, "{stack:?}");
                    let swiftness = net_id(game, "minecraft:potion", "minecraft:swiftness");
                    assert!(
                        matches!(
                            component(game, stack, "minecraft:potion_contents"),
                            Component::Potion { potion: Some(p), color: None, .. } if *p == swiftness
                        ),
                        "{stack:?}"
                    );
                    tick_sends_nothing(game, &mut reader);

                    let capped = || {
                        api(
                            "minecraft:stone",
                            16,
                            &[("minecraft:max_stack_size", Tag::Int(16))],
                        )
                    };
                    assert_eq!(local.give_creative(game, &capped()), Ok(None));
                    let packets = creative_slots(&sent(&mut reader));
                    let (slot, stack) = only_slot(&packets);
                    let stone = net_id(game, "minecraft:item", "minecraft:stone");
                    assert_eq!((slot, stack.item, stack.count), (38, stone, 16));
                    assert_eq!(stack.added.len(), 1, "{stack:?}");
                    assert_eq!(
                        component(game, stack, "minecraft:max_stack_size"),
                        &Component::Int(16)
                    );
                    // The capped stack is full at 16: the next one goes to an empty slot.
                    assert_eq!(local.give_creative(game, &capped()), Ok(None));
                    assert_eq!(game.inventory[2].as_ref().map(|s| s.count), Some(16));
                    let packets = creative_slots(&sent(&mut reader));
                    let (slot, stack) = only_slot(&packets);
                    assert_eq!((slot, stack.item, stack.count), (39, stone, 16));
                    tick_sends_nothing(game, &mut reader);

                    game.client
                        .remote
                        .as_mut()
                        .unwrap()
                        .menu
                        .set(Some((1, "minecraft:generic_9x3".into())));
                    let letter = named("minecraft:paper", 2, "letter");
                    assert_eq!(local.give_creative(game, &letter), Ok(None));
                    assert_eq!(game.inventory[4].as_ref().map(test_snapshot), Some(letter));
                    let packets = creative_slots(&sent(&mut reader));
                    let (slot, stack) = only_slot(&packets);
                    assert_eq!(
                        (slot, stack.item, stack.count),
                        (40, net_id(game, "minecraft:item", "minecraft:paper"), 2)
                    );
                    assert_eq!(stack.added.len(), 1, "{stack:?}");
                    assert_eq!(
                        component(game, stack, "minecraft:custom_name"),
                        &Component::Text(Tag::String("letter".into()))
                    );
                    game.client.remote.as_mut().unwrap().menu.set(None);
                    tick_sends_nothing(game, &mut reader);
                });
            },
        );
    }

    /// `Inventory.add`'s order, each give smaller than the room left: the selected slot, the
    /// off hand (wire slot 45), the hotbar, the main inventory, then the first empty slot,
    /// never onto a stack with other components; what does not fit comes back.
    #[test]
    #[ignore = "daily"]
    fn a_give_stacks_in_inventory_order_and_returns_what_did_not_fit() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::remote::tests::creative_sync_tests::give_stack::a_give_stacks_in_inventory_order_and_returns_what_did_not_fit",
            || {
                with_client_game(|game| {
                    let mut reader = logged_in(game);
                    game.set_local_mode(GameMode::Creative);
                    game.selected = 4;
                    game.inventory[4] = engine("minecraft:stone", 60);
                    game.offhand = engine("minecraft:stone", 60);
                    game.inventory[0] = engine("minecraft:stone", 60);
                    let mut renamed = sunrise_items::items::ItemStack::new("minecraft:stone", 1);
                    renamed.data_mut().custom_name = Some(Tag::String("other".into()));
                    game.inventory[1] = Some(renamed.clone());
                    game.inventory[9] = engine("minecraft:stone", 60);
                    game.remote_send_state(0);
                    sent(&mut reader);
                    let stone = net_id(game, "minecraft:item", "minecraft:stone");
                    let local = game.local_player().unwrap();
                    let mut give = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
                                    count| {
                        assert_eq!(
                            local.give_creative(game, &api("minecraft:stone", count, &[])),
                            Ok(None)
                        );
                        let packets = creative_slots(&sent(&mut reader));
                        slots_and_counts(&packets)
                    };

                    // Selected slot (hotbar 4 is wire slot 40) first, and only it.
                    assert_eq!(give(game, 3), [(40, stone, 63)]);
                    // Then the off hand.
                    assert_eq!(give(game, 3), [(40, stone, 64), (45, stone, 62)]);
                    // Then the hotbar (slot 0 is wire slot 36).
                    assert_eq!(give(game, 5), [(36, stone, 63), (45, stone, 64)]);
                    // Then the main inventory (slot 9), past the renamed stone in slot 1.
                    assert_eq!(give(game, 5), [(9, stone, 64), (36, stone, 64)]);
                    // No room on a matching stack: the first empty slot, hotbar 2.
                    assert_eq!(give(game, 2), [(38, stone, 2)]);
                    assert_eq!(game.inventory[1], Some(renamed));

                    let dirt = engine("minecraft:dirt", 64);
                    game.inventory[..36].fill(dirt.clone());
                    game.offhand = dirt;
                    game.remote_send_state(0);
                    sent(&mut reader);
                    let inventory = game.inventory.clone();
                    let gift = named("minecraft:stone", 5, "left over");
                    assert_eq!(local.give_creative(game, &gift), Ok(Some(gift)));
                    assert_eq!(game.inventory, inventory);
                    assert!(creative_slots(&sent(&mut reader)).is_empty());
                });
            },
        );
    }
}

mod destroy_carried {
    use crate::game::client_side::native_hotbar_slot::hotbar_slot_tests::{logged_in, sent};
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use mod_api::CarriedDestroyError;
    use sunrise_client_gameplay::game::ClientGame;
    use sunrise_command::args::GameMode;
    use sunrise_core::nbt::Tag;
    use sunrise_gameplay_common::game::{
        Screen, execution_role::ExecutionRole, in_own_process,
        player_identity::PlayerIdentityGameExt1 as _,
    };
    use sunrise_items::items::ItemStack;
    use sunrise_protocol::packets::play::{self, Clientbound as C, Serverbound as S};

    fn stack(item: &str, count: u32) -> Option<ItemStack> {
        Some(ItemStack::new(item, count))
    }

    /// The packets among `packets` that write a slot or the cursor, as the server decodes them.
    fn slot_writes(packets: Vec<S>) -> Vec<S> {
        packets
            .into_iter()
            .filter(|p| matches!(p, S::SetCreativeModeSlot(_) | S::ContainerClick(_)))
            .collect()
    }

    fn menu(game: &mut ClientGame<'_>) -> mod_api::MenuRef {
        let local = game.local_player().unwrap();
        local.container_menu(game).unwrap()
    }

    /// `menu` refuses with `error` both carrying a stack and carrying nothing, leaving the
    /// cursor and the inventory as they were.
    fn refuses(game: &mut ClientGame<'_>, menu: mod_api::MenuRef, error: CarriedDestroyError) {
        let inventory = game.common.inventory.clone();
        for carried in [stack("minecraft:stone", 5), None] {
            game.common.carried = carried.clone();
            assert_eq!(menu.destroy_carried(game), Err(error), "{carried:?}");
            assert_eq!(game.common.carried, carried);
            assert_eq!(game.common.inventory, inventory);
        }
    }

    /// Every refusal, with a stack carried and with none, changes nothing and sends nothing.
    /// A live server menu is refused even when the creative inventory screen is still visible.
    #[test]
    #[ignore = "daily"]
    fn a_refused_destroy_changes_and_sends_nothing() {
        in_own_process(
            "cut_layout::game::remote::tests::creative_sync_tests::destroy_carried::a_refused_destroy_changes_and_sends_nothing",
            || {
                with_client_game(|game| {
                    let before_login = mod_api::__private::game::menu_ref(
                        mod_api::__private::game::player_ref(
                            mod_api::EntityRef::new(mod_api::WorldEpoch::new(1), 1),
                            1,
                        ),
                        0,
                        0,
                    );
                    refuses(game, before_login, CarriedDestroyError::Unavailable);
                    let mut reader = logged_in(game);
                    game.common.inventory[0] = stack("minecraft:dirt", 1);
                    game.remote_send_state(0);
                    sent(&mut reader);
                    let inventory_menu = menu(game);
                    game.client.screen = Screen::Creative;
                    refuses(game, inventory_menu, CarriedDestroyError::NotCreative);
                    game.set_local_mode(GameMode::Creative);
                    game.client.screen = Screen::Inventory;
                    refuses(game, inventory_menu, CarriedDestroyError::ServerOwnedMenu);
                    let packets = sent(&mut reader);
                    assert!(packets.is_empty(), "{packets:?}");

                    game.client.screen = Screen::Creative;
                    let previous_screen = game.client.screen;
                    let chest = game
                        .client
                        .remote
                        .as_ref()
                        .unwrap()
                        .builtin
                        .get("minecraft:menu")
                        .unwrap()
                        .id("minecraft:generic_9x3")
                        .unwrap();
                    game.handle_packet(C::OpenScreen(play::OpenScreen {
                        container: 12,
                        menu: chest,
                        title: Tag::String("Chest".into()),
                    }))
                    .unwrap();
                    assert_eq!(game.client.screen, Screen::Inventory);
                    assert_eq!(game.remote_menu().0, 12);
                    sent(&mut reader);
                    refuses(game, inventory_menu, CarriedDestroyError::Stale);
                    let chest_menu = menu(game);
                    refuses(game, chest_menu, CarriedDestroyError::ServerOwnedMenu);

                    // Keep the previous screen visible without changing the live server menu.
                    // Model that state without depending on how ScreenOpening handles a veto.
                    game.client.screen = previous_screen;
                    refuses(game, chest_menu, CarriedDestroyError::ServerOwnedMenu);
                    game.common.options.execution_role = ExecutionRole::IntegratedServer;
                    refuses(game, chest_menu, CarriedDestroyError::Unavailable);
                    game.common.options.execution_role = ExecutionRole::Client;
                    let packets = sent(&mut reader);
                    assert!(packets.is_empty(), "{packets:?}");
                });
            },
        );
    }

    /// On the creative screen the cursor is the client's alone: the destroy slot clears it
    /// and sends nothing, and an empty cursor is no destroy.
    #[test]
    #[ignore = "daily"]
    fn the_creative_screen_destroys_without_a_packet() {
        in_own_process(
            "cut_layout::game::remote::tests::creative_sync_tests::destroy_carried::the_creative_screen_destroys_without_a_packet",
            || {
                with_client_game(|game| {
                    let mut reader = logged_in(game);
                    game.set_local_mode(GameMode::Creative);
                    game.client.screen = Screen::Creative;
                    game.common.inventory[0] = stack("minecraft:dirt", 1);
                    game.remote_send_state(0);
                    sent(&mut reader);
                    let inventory = game.common.inventory.clone();
                    let menu = menu(game);
                    game.common.carried = stack("minecraft:diamond_sword", 1);
                    assert_eq!(menu.destroy_carried(game), Ok(true));
                    assert_eq!(game.common.carried, None);
                    assert_eq!(menu.destroy_carried(game), Ok(false));
                    assert_eq!(game.common.inventory, inventory);
                    let writes = slot_writes(sent(&mut reader));
                    assert!(writes.is_empty(), "{writes:?}");
                    // The tick's sync after it sends nothing either.
                    game.remote_send_state(0);
                    let writes = slot_writes(sent(&mut reader));
                    assert!(writes.is_empty(), "{writes:?}");
                });
            },
        );
    }
}
