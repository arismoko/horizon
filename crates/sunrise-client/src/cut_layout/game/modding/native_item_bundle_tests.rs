use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_gameplay_common::game::native_item_bundle::NativeItemBundleGameExt1 as _;
use sunrise_items::items::ItemStack;
use {
    sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan,
    sunrise_gameplay_common::game::server_side::native_item_places::tests::guest,
};

#[test]
#[ignore = "daily"]
fn bundle_handler_weight_front_order_and_refusal() {
    with_game(|game| {
        let empty = ItemStack::new("minecraft:bundle", 1);
        let pearls = ItemStack::new("minecraft:ender_pearl", 16);
        let (moved, full) = game.item_bundle_insert_slot(&empty, 0, &pearls).unwrap();
        assert_eq!(moved, 16);
        assert_eq!(
            game.item_bundle_insert_slot(&full, 0, &ItemStack::new("minecraft:ender_pearl", 1))
                .unwrap(),
            (0, full.clone())
        );
        assert_eq!(game.item_bundle_capacity(&full, 0, &pearls), 0);
        assert_eq!(
            game.item_bundle_insert_slot(&empty, 0, &ItemStack::new("minecraft:shulker_box", 1))
                .unwrap()
                .0,
            0
        );
        let (_, arrows) = game
            .item_bundle_insert_slot(&empty, 0, &ItemStack::new("minecraft:arrow", 2))
            .unwrap();
        let (_, mixed) = game
            .item_bundle_insert_slot(&arrows, 0, &ItemStack::new("minecraft:ender_pearl", 3))
            .unwrap();
        let slots = game.item_bundle_slots(&mixed).unwrap();
        assert_eq!(
            slots,
            vec![
                Some(ItemStack::new("minecraft:ender_pearl", 3)),
                Some(ItemStack::new("minecraft:arrow", 2))
            ]
        );
        let (out, rest) = game.item_bundle_extract(&mixed, 0, 64).unwrap();
        assert_eq!(out, Some(ItemStack::new("minecraft:ender_pearl", 3)));
        assert_eq!(
            game.item_bundle_slots(&rest).unwrap(),
            vec![Some(ItemStack::new("minecraft:arrow", 2))]
        );
        // A UI selection must not redirect automation away from the front.
        let mut selected = mixed.clone();
        selected.data_mut().bundle_selected = sunrise_items::item_data::BundleSelected(1);
        assert_eq!(
            game.item_bundle_extract(&selected, 0, 1).unwrap().0,
            Some(ItemStack::new("minecraft:ender_pearl", 1))
        );
        let mut overfull = empty.clone();
        overfull.data_mut().bundle_contents = vec![
            ItemStack::new("minecraft:ender_pearl", 20),
            ItemStack::new("minecraft:arrow", 64),
        ];
        let (out, rest) = game.item_bundle_extract(&overfull, 0, 1).unwrap();
        assert_eq!(out, Some(ItemStack::new("minecraft:ender_pearl", 1)));
        assert_eq!(
            game.item_bundle_slots(&rest).unwrap(),
            vec![
                Some(ItemStack::new("minecraft:ender_pearl", 19)),
                Some(ItemStack::new("minecraft:arrow", 64))
            ]
        );
        assert!(
            game.item_bundle_slots(&ItemStack::new("minecraft:stone", 1))
                .is_none()
        );
    });
}

#[test]
#[ignore = "daily"]
fn bundle_items_place_handler_reads_and_rolls_back() {
    use mod_api::{ITEM_ITEMS, ItemPlace};
    with_game(|game| {
        game.creative = false;
        let mut bundle = ItemStack::new("minecraft:bundle", 1);
        bundle.data_mut().bundle_contents = vec![
            ItemStack::new("minecraft:ender_pearl", 3),
            ItemStack::new("minecraft:arrow", 2),
        ];
        open_to_lan(game);
        let mut player = guest(1);
        player.inventory[0] = Some(bundle.clone());
        game.server.lan.as_mut().unwrap().hub.guests.push(player);
        let place = ItemPlace::PlayerHand {
            player: 123,
            hand: mod_api::Hand::Main,
        };
        let handler = place.capability(game, &ITEM_ITEMS).unwrap();
        assert_eq!(handler.size(game), 2);
        assert_eq!(handler.amount(game, 0), 3);
        assert_eq!(handler.amount(game, 1), 2);
        let pearl = handler.resource(game, 0).unwrap();
        assert_eq!(
            sunrise_gameplay_common::game::api_values::stack_value(pearl.as_stack())
                .unwrap()
                .item
                .as_ref(),
            "minecraft:ender_pearl"
        );
        assert_eq!(
            sunrise_gameplay_common::game::api_values::stack_value(
                handler.resource(game, 1).unwrap().as_stack()
            )
            .unwrap()
            .item
            .as_ref(),
            "minecraft:arrow"
        );
        let result = game.storage_transaction(|host, tx| {
            assert_eq!(handler.extract_at(host, tx, 0, &pearl, 2), Ok(2));
            Err::<(), _>(())
        });
        assert_eq!(result, Err(()));
        assert_eq!(
            game.server.lan.as_ref().unwrap().hub.guests[0].inventory[0],
            Some(bundle)
        );
    });
}

#[test]
#[ignore = "daily"]
fn nested_bundle_weight_limits_arrow_capacity_and_insertion() {
    with_game(|game| {
        let empty = ItemStack::new("minecraft:bundle", 1);
        let arrows = ItemStack::new("minecraft:arrow", 64);
        for (contents, room) in [
            (vec![], 60),
            (vec![ItemStack::new("minecraft:ender_pearl", 1)], 56),
        ] {
            let mut nested = empty.clone();
            nested.data_mut().bundle_contents = contents;
            let mut outer = empty.clone();
            outer.data_mut().bundle_contents = vec![nested.clone()];
            assert_eq!(game.item_bundle_capacity(&outer, 0, &arrows), room);
            let (moved, filled) = game.item_bundle_insert_slot(&outer, 0, &arrows).unwrap();
            assert_eq!(moved, room);
            assert_eq!(
                game.item_bundle_slots(&filled).unwrap(),
                vec![Some(ItemStack::new("minecraft:arrow", room)), Some(nested)]
            );
            assert_eq!(game.item_bundle_capacity(&filled, 0, &arrows), 0);
        }
    });
}

#[test]
#[ignore = "daily"]
fn stacked_bundle_insertion_splits_one_and_restores_inventory_on_abort() {
    use mod_api::{ITEM_ITEMS, ItemPlace};
    use sunrise_gameplay_common::game::api_values::TryIntoApi;
    with_game(|game| {
        game.creative = false;
        open_to_lan(game);
        let mut player = guest(1);
        player.inventory[0] = Some(ItemStack::new("minecraft:bundle", 2));
        let original = player.inventory.clone();
        game.server.lan.as_mut().unwrap().hub.guests.push(player);
        let place = ItemPlace::PlayerHand {
            player: 123,
            hand: mod_api::Hand::Main,
        };
        let handler = place.capability(game, &ITEM_ITEMS).unwrap();
        let arrow = mod_api::ItemResource::of(
            &ItemStack::new("minecraft:arrow", 1).try_into_api().unwrap(),
        );
        assert_eq!(
            game.storage_transaction(|host, tx| {
                assert_eq!(handler.insert_at(host, tx, 0, &arrow, 3), Ok(3));
                Err::<(), _>(())
            }),
            Err(())
        );
        assert_eq!(
            game.server.lan.as_ref().unwrap().hub.guests[0].inventory,
            original
        );
        assert_eq!(
            game.storage_transaction(|host, tx| handler.insert_at(host, tx, 0, &arrow, 3)),
            Ok(3)
        );
        let mut changed = ItemStack::new("minecraft:bundle", 1);
        changed.data_mut().bundle_contents = vec![ItemStack::new("minecraft:arrow", 3)];
        let inventory = &game.server.lan.as_ref().unwrap().hub.guests[0].inventory;
        let stacks: Vec<_> = inventory.iter().flatten().cloned().collect();
        assert_eq!(stacks.len(), 2);
        assert!(stacks.contains(&ItemStack::new("minecraft:bundle", 1)));
        assert!(stacks.contains(&changed));
    });
}

struct BundleMod;
#[mod_api::__subscriptions]
impl mod_api::Hooks for BundleMod {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        r.item_capability(
            &mod_api::ITEM_ITEMS,
            "minecraft:paper",
            |_: &mut dyn mod_api::ServerHost, place: &mod_api::ItemPlace| {
                Some(Box::new(mod_api::BundleItems::new(place.clone()))
                    as Box<dyn mod_api::ResourceHandler<mod_api::ItemResource>>)
            },
        );
    }
}

#[test]
#[ignore = "daily"]
fn mod_registered_bundle_items_uses_weight_and_front_extraction() {
    use mod_api::{ITEM_ITEMS, ItemPlace};
    let mods: mod_api::Mods =
        vec![mod_api::ModEntry::new("bundle_test", Box::new(BundleMod))].into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.creative = false;
            let mut bundle = ItemStack::new("minecraft:paper", 1);
            bundle.data_mut().bundle_contents = vec![ItemStack::new("minecraft:ender_pearl", 16)];
            open_to_lan(game);
            let mut player = guest(1);
            player.inventory[0] = Some(bundle.clone());
            game.server.lan.as_mut().unwrap().hub.guests.push(player);
            let place = ItemPlace::PlayerHand {
                player: 123,
                hand: mod_api::Hand::Main,
            };
            let handler = place.capability(game, &ITEM_ITEMS).unwrap();
            let pearl = handler.resource(game, 0).unwrap();
            assert_eq!(
                game.storage_transaction(|host, tx| handler.insert_at(host, tx, 0, &pearl, 1)),
                Ok(0)
            );
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[0].inventory[0],
                Some(bundle)
            );
            assert_eq!(
                game.storage_transaction(|host, tx| handler.extract_at(host, tx, 0, &pearl, 3)),
                Ok(3)
            );
            assert_eq!(handler.amount(game, 0), 13);
            use sunrise_gameplay_common::game::api_values::TryIntoApi;
            let arrow = mod_api::ItemResource::of(
                &ItemStack::new("minecraft:arrow", 1).try_into_api().unwrap(),
            );
            assert_eq!(
                game.storage_transaction(|host, tx| handler.insert_at(host, tx, 0, &arrow, 2)),
                Ok(2)
            );
            assert_eq!(
                game.storage_transaction(|host, tx| handler.insert_at(host, tx, 0, &pearl, 1)),
                Ok(1)
            );
            assert_eq!(handler.resource(game, 0), Some(pearl.clone()));
            assert_eq!(handler.amount(game, 0), 14);
            assert_eq!(
                game.storage_transaction(|host, tx| handler.extract_at(host, tx, 0, &pearl, 64)),
                Ok(14)
            );
            assert_eq!(handler.resource(game, 0), Some(arrow));
            assert_eq!(handler.amount(game, 0), 2);
        })
    });
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

#[cfg(test)]
mod custom_bundle_integration_tests {
    use super::*;
    use mod_api::{
        BundleConfig, BundleItems, ComponentTypesEvent, Flow, Hooks, Host, HostSide, ITEM_ITEMS,
        ItemDestroyedEvent, ItemPart, ItemPlace, ItemResource, ItemStackedOnOtherEvent, ModEntry,
        Mods, Registrar, RegistrationContext, StackedOnOtherDirection, item::menu::ClickAction,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use sunrise_core::nbt::Tag;
    use sunrise_gameplay::game::native_guest_turn_tests::with_guests;
    use sunrise_gameplay_common::game::api_values::TryIntoApi;
    use sunrise_gameplay_common::game::tables::GameItems;
    use sunrise_protocol::packets::play::{ContainerClick, Serverbound};

    const POUCH_COMPONENT: &str = "pouch:contents";

    fn empty_pouch(count: u32) -> ItemStack {
        let mut stack = ItemStack::new("minecraft:paper", count);
        stack
            .data_mut()
            .set_other(POUCH_COMPONENT, Some(Tag::List(Vec::new())));
        stack
    }

    struct FilteredPouchMod {
        pub config: BundleConfig,
    }
    #[mod_api::__subscriptions]
    impl Hooks for FilteredPouchMod {
        fn component_types(&self, _: &RegistrationContext, event: &mut ComponentTypesEvent<'_>) {
            event.register(POUCH_COMPONENT.into());
        }
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let config = self.config.clone();
            r.item_capability(&ITEM_ITEMS, "minecraft:paper", move |_, place| {
                Some(
                    Box::new(BundleItems::with_config(place.clone(), config.clone()))
                        as Box<dyn mod_api::ResourceHandler<ItemResource>>,
                )
            });
        }
    }

    #[test]
    #[ignore = "daily"]
    fn custom_bundle_component_multipliers_tag_filter_and_nested_occupancy() {
        let arrow_pouch =
            BundleConfig::custom(POUCH_COMPONENT, 4).with_filter_tag("minecraft:arrows");
        let mods: Mods = vec![ModEntry::new(
            "custom_pouch",
            Box::new(FilteredPouchMod {
                config: arrow_pouch.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods.clone(), || {
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            with_game(|game| {
                game.creative = false;
                let empty = empty_pouch(1);
                let arrow = ItemStack::new("minecraft:arrow", 64);
                let stone = ItemStack::new("minecraft:stone", 64);

                // Tag filter accepts minecraft:arrows and refuses minecraft:stone.
                assert_eq!(
                    game.bundle_capacity(&empty, 0, &arrow, &arrow_pouch)
                        .unwrap(),
                    256
                );
                assert_eq!(
                    game.bundle_capacity(&empty, 0, &stone, &arrow_pouch)
                        .unwrap(),
                    0
                );
                assert_eq!(
                    game.bundle_insert_slot(&empty, 0, &stone, &arrow_pouch)
                        .unwrap()
                        .unwrap()
                        .0,
                    0
                );

                // Multiplier 4 holds 4 * 64 = 256 arrows split across 4 stacks <= 64.
                let mut current = empty.clone();
                for _ in 0..4 {
                    let (moved, next) = game
                        .bundle_insert_slot(&current, 0, &arrow, &arrow_pouch)
                        .unwrap()
                        .unwrap();
                    assert_eq!(moved, 64);
                    current = next;
                }
                assert_eq!(
                    game.bundle_capacity(&current, 0, &arrow, &arrow_pouch)
                        .unwrap(),
                    0
                );
                let metrics = game
                    .bundle_metrics(&current, &arrow_pouch)
                    .unwrap()
                    .unwrap();
                assert!(metrics.is_full);
                assert_eq!(metrics.fullness, 1.0);
                assert_eq!(metrics.bar, Some((13, 0xFF5454)));
                assert_eq!(metrics.stack_count, 4);

                // Multiplier 8 and 12 scale capacity cleanly over the same stored component.
                let m8 = BundleConfig::custom(POUCH_COMPONENT, 8);
                let m12 = BundleConfig::custom(POUCH_COMPONENT, 12);
                assert_eq!(game.bundle_capacity(&current, 0, &arrow, &m8).unwrap(), 256);
                assert_eq!(
                    game.bundle_capacity(&current, 0, &arrow, &m12).unwrap(),
                    512
                );
                let half_metrics = game.bundle_metrics(&current, &m8).unwrap().unwrap();
                assert!(!half_metrics.is_full);
                assert_eq!(half_metrics.fullness, 0.5);
                assert_eq!(half_metrics.bar, Some((7, 0x7087FF)));

                // Nested occupancy: vanilla bundle + nested custom pouch inside an unfiltered pouch.
                let unfiltered = BundleConfig::custom(POUCH_COMPONENT, 4);
                let mut nested_vanilla = ItemStack::new("minecraft:bundle", 1);
                nested_vanilla.data_mut().bundle_contents =
                    vec![ItemStack::new("minecraft:ender_pearl", 4)];
                let (_, nested_custom) = game
                    .bundle_insert_slot(
                        &empty,
                        0,
                        &ItemStack::new("minecraft:ender_pearl", 4),
                        &unfiltered,
                    )
                    .unwrap()
                    .unwrap();
                let (_, with_vanilla) = game
                    .bundle_insert_slot(&empty, 0, &nested_vanilla, &unfiltered)
                    .unwrap()
                    .unwrap();
                let (_, with_both) = game
                    .bundle_insert_slot(&with_vanilla, 0, &nested_custom, &unfiltered)
                    .unwrap()
                    .unwrap();
                // Each inner container weighs 4/16 + 1/16 = 5/16 -> total 10/16.
                // Remaining in M=4 pouch = 4 - 10/16 = 54/16 = 216 arrows.
                assert_eq!(
                    game.bundle_capacity(&with_both, 0, &arrow, &unfiltered)
                        .unwrap(),
                    216
                );

                // Transactional ITEM_ITEMS via BundleItems::with_config splits stacked pouches and
                // rolls back cleanly on abort.
                open_to_lan(game);
                let mut player = guest(1);
                player.inventory[0] = Some(empty_pouch(2));
                let original = player.inventory.clone();
                game.server.lan.as_mut().unwrap().hub.guests.push(player);
                let place = ItemPlace::PlayerHand {
                    player: 123,
                    hand: mod_api::Hand::Main,
                };
                let handler = place.capability(game, &ITEM_ITEMS).unwrap();
                let arrow_res =
                    ItemResource::of(&ItemStack::new("minecraft:arrow", 1).try_into_api().unwrap());
                let stone_res =
                    ItemResource::of(&ItemStack::new("minecraft:stone", 1).try_into_api().unwrap());
                assert_eq!(
                    game.storage_transaction(|host, tx| {
                        assert_eq!(handler.insert_at(host, tx, 0, &stone_res, 5), Ok(0));
                        assert_eq!(handler.insert_at(host, tx, 0, &arrow_res, 20), Ok(20));
                        Err::<(), _>(())
                    }),
                    Err(())
                );
                assert_eq!(
                    game.server.lan.as_ref().unwrap().hub.guests[0].inventory,
                    original
                );

                assert_eq!(
                    game.storage_transaction(
                        |host, tx| handler.insert_at(host, tx, 0, &arrow_res, 20)
                    ),
                    Ok(20)
                );
                let inventory = &game.server.lan.as_ref().unwrap().hub.guests[0].inventory;
                let stacks: Vec<_> = inventory.iter().flatten().cloned().collect();
                assert_eq!(stacks.len(), 2);
                assert!(stacks.contains(&empty_pouch(1)));
                let filled = stacks
                    .iter()
                    .find(|stack| *stack != &empty_pouch(1))
                    .expect("split filled pouch");
                assert_eq!(filled.count, 1);
                assert_eq!(
                    game.bundle_slots(filled, &arrow_pouch).unwrap().unwrap(),
                    vec![Some(ItemStack::new("minecraft:arrow", 20))]
                );

                // Unsupported non-list custom payload (e.g. a compound without "items") returns
                // Err(BundleReadError::UnsupportedFormat) across all 5 queries and is left
                // completely untouched by BundleItems.
                let mut compound_pouch = ItemStack::new("minecraft:paper", 1);
                let bad_payload =
                    Tag::compound([("entries", Tag::List(Vec::new())), ("tier", Tag::Int(2))]);
                compound_pouch
                    .data_mut()
                    .set_other(POUCH_COMPONENT, Some(bad_payload.clone()));
                assert_eq!(
                    game.bundle_slots(&compound_pouch, &arrow_pouch),
                    Err(mod_api::BundleReadError::UnsupportedFormat)
                );
                assert_eq!(
                    game.bundle_capacity(&compound_pouch, 0, &arrow, &arrow_pouch),
                    Err(mod_api::BundleReadError::UnsupportedFormat)
                );
                assert_eq!(
                    game.bundle_insert_slot(&compound_pouch, 0, &arrow, &arrow_pouch),
                    Err(mod_api::BundleReadError::UnsupportedFormat)
                );
                assert_eq!(
                    game.bundle_extract(&compound_pouch, 0, 1, &arrow_pouch),
                    Err(mod_api::BundleReadError::UnsupportedFormat)
                );
                assert_eq!(
                    game.bundle_metrics(&compound_pouch, &arrow_pouch),
                    Err(mod_api::BundleReadError::UnsupportedFormat)
                );
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.inventory[0] = Some(compound_pouch.clone());
                    });
                let bad_handler = place.capability(game, &ITEM_ITEMS).unwrap();
                assert_eq!(bad_handler.size(game), 0);
                assert_eq!(bad_handler.capacity(game, 0, Some(&arrow_res)), 0);
                assert_eq!(
                    game.storage_transaction(
                        |host, tx| bad_handler.insert_at(host, tx, 0, &arrow_res, 5)
                    ),
                    Ok(0)
                );
                assert_eq!(
                    game.server.lan.as_ref().unwrap().hub.guests[0].inventory[0]
                        .as_ref()
                        .and_then(|s| s.data())
                        .and_then(|d| d.other(POUCH_COMPONENT)),
                    Some(&bad_payload)
                );

                // Malformed list entry ([Int(42), valid arrow]) and nested malformed custom pouch
                // (both inside outer and offered to a valid outer) also refuse with
                // BundleReadError::UnsupportedFormat across all 5 bundle_* operations without
                // overwriting the stack.
                let bad_list_payload = Tag::List(vec![
                    Tag::Int(42),
                    Tag::compound([
                        ("id", Tag::String("minecraft:arrow".into())),
                        ("count", Tag::Int(4)),
                    ]),
                ]);
                let mut bad_list_pouch = ItemStack::new("minecraft:paper", 1);
                bad_list_pouch
                    .data_mut()
                    .set_other(POUCH_COMPONENT, Some(bad_list_payload.clone()));
                let mut outer_with_nested_bad = ItemStack::new("minecraft:paper", 1);
                outer_with_nested_bad.data_mut().set_other(
                    POUCH_COMPONENT,
                    Some(Tag::List(vec![sunrise_items::item_nbt::item_to_nbt(
                        &bad_list_pouch,
                        game.items(),
                    )])),
                );
                let mut unknown_item_pouch = ItemStack::new("minecraft:paper", 1);
                unknown_item_pouch.data_mut().set_other(
                    POUCH_COMPONENT,
                    Some(Tag::List(vec![
                        Tag::compound([
                            ("id", Tag::String("mod:removed_item".into())),
                            ("count", Tag::Int(1)),
                        ]),
                        Tag::compound([
                            ("id", Tag::String("minecraft:arrow".into())),
                            ("count", Tag::Int(4)),
                        ]),
                    ])),
                );
                for malformed in [&bad_list_pouch, &outer_with_nested_bad, &unknown_item_pouch] {
                    assert_eq!(
                        game.bundle_slots(malformed, &unfiltered),
                        Err(mod_api::BundleReadError::UnsupportedFormat)
                    );
                    assert_eq!(
                        game.bundle_capacity(malformed, 0, &arrow, &unfiltered),
                        Err(mod_api::BundleReadError::UnsupportedFormat)
                    );
                    assert_eq!(
                        game.bundle_insert_slot(malformed, 0, &arrow, &unfiltered),
                        Err(mod_api::BundleReadError::UnsupportedFormat)
                    );
                    assert_eq!(
                        game.bundle_extract(malformed, 0, 1, &unfiltered),
                        Err(mod_api::BundleReadError::UnsupportedFormat)
                    );
                    assert_eq!(
                        game.bundle_metrics(malformed, &unfiltered),
                        Err(mod_api::BundleReadError::UnsupportedFormat)
                    );
                }
                // Offering a malformed nested custom pouch into a valid outer pouch fails cleanly.
                assert_eq!(
                    game.bundle_capacity(&empty, 0, &bad_list_pouch, &unfiltered),
                    Err(mod_api::BundleReadError::UnsupportedFormat)
                );
                assert_eq!(
                    game.bundle_insert_slot(&empty, 0, &bad_list_pouch, &unfiltered),
                    Err(mod_api::BundleReadError::UnsupportedFormat)
                );

                // Weights are vanilla's int `Fraction`: five arrows stacking to the coprime 97,
                // 89, 83, 79 and 73 need a denominator past i32::MAX ("Excessive total bundle
                // weight"), and a multiplier past i32::MAX has no capacity. Every operation
                // refuses with ExcessiveWeight and nothing is written.
                let arrow_stacking_to = |max: i32| {
                    Tag::compound([
                        ("id", Tag::String("minecraft:arrow".into())),
                        ("count", Tag::Int(1)),
                        (
                            "components",
                            Tag::compound([("minecraft:max_stack_size", Tag::Int(max))]),
                        ),
                    ])
                };
                let mut excessive = ItemStack::new("minecraft:paper", 1);
                excessive.data_mut().set_other(
                    POUCH_COMPONENT,
                    Some(Tag::List(
                        [97, 89, 83, 79, 73].map(arrow_stacking_to).to_vec(),
                    )),
                );
                let huge = BundleConfig::custom(POUCH_COMPONENT, u32::MAX);
                for (stack, config) in [(&excessive, &unfiltered), (&empty, &huge)] {
                    let refused = Some(mod_api::BundleReadError::ExcessiveWeight);
                    assert_eq!(game.bundle_slots(stack, config).err(), refused);
                    assert_eq!(
                        game.bundle_capacity(stack, 0, &arrow, config).err(),
                        refused
                    );
                    assert_eq!(
                        game.bundle_insert_slot(stack, 0, &arrow, config).err(),
                        refused
                    );
                    assert_eq!(game.bundle_extract(stack, 0, 1, config).err(), refused);
                    assert_eq!(game.bundle_metrics(stack, config).err(), refused);
                }
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.inventory[0] = Some(excessive.clone());
                    });
                let excessive_handler = place.capability(game, &ITEM_ITEMS).unwrap();
                assert_eq!(excessive_handler.size(game), 0);
                assert_eq!(
                    game.storage_transaction(
                        |host, tx| excessive_handler.insert_at(host, tx, 0, &arrow_res, 5)
                    ),
                    Ok(0)
                );
                assert_eq!(
                    game.server.lan.as_ref().unwrap().hub.guests[0].inventory[0].as_ref(),
                    Some(&excessive)
                );
            });
        });
    }

    struct PouchPart {
        pub config: BundleConfig,
        panic_stack: Arc<AtomicBool>,
    }
    #[mod_api::sunrise_part]
    impl ItemPart for PouchPart {
        fn item_stacked_on_other(
            &self,
            host: HostSide<'_>,
            event: &mut ItemStackedOnOtherEvent<'_, '_>,
        ) -> Flow {
            if self.panic_stack.load(Ordering::Relaxed) {
                panic!("pouch stacking failure");
            }
            let host_ref: &dyn Host = match &host {
                HostSide::Server(h) => &**h,
                HostSide::Client(h) => &**h,
            };
            let primary = event.click.action == ClickAction::Primary;
            match event.click.direction {
                StackedOnOtherDirection::CarriedOnOther => {
                    let Some(carried) = event.click.carried().cloned() else {
                        return Flow::Continue;
                    };
                    if primary
                        && let Some(clicked) = event.click.clicked().cloned()
                        && let Ok(Some((moved, updated))) =
                            self.config.try_insert(host_ref, &carried, &clicked)
                        && moved > 0
                    {
                        event.click.set_carried(Some(updated));
                        event.click.set_clicked(clicked.shrink(moved));
                        event.click.handled = true;
                        return Flow::Cancel;
                    }
                    if !primary
                        && event.click.clicked().is_none()
                        && let Ok(Some((Some(out), updated))) =
                            self.config.extract(host_ref, &carried, u32::MAX)
                    {
                        event.click.set_carried(Some(updated));
                        event.click.set_clicked(Some(out));
                        event.click.handled = true;
                        return Flow::Cancel;
                    }
                }
                StackedOnOtherDirection::OtherStackedOnMe => {
                    if primary
                        && let Some(clicked) = event.click.clicked().cloned()
                        && let Some(carried) = event.click.carried().cloned()
                        && let Ok(Some((moved, updated))) =
                            self.config.try_insert(host_ref, &clicked, &carried)
                        && moved > 0
                    {
                        event.click.set_clicked(Some(updated));
                        event.click.set_carried(carried.shrink(moved));
                        event.click.handled = true;
                        return Flow::Cancel;
                    }
                }
                StackedOnOtherDirection::Event => {}
            }
            Flow::Continue
        }

        fn item_destroyed(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut ItemDestroyedEvent<'_, '_>,
        ) {
            if let Ok(Some(inside)) = self.config.slots(host, event.stack) {
                for stack in inside {
                    event.drops.push(stack).unwrap();
                }
            }
        }
    }

    struct FullPouchMod {
        pub config: BundleConfig,
        panic_stack: Arc<AtomicBool>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for FullPouchMod {
        fn component_types(&self, _: &RegistrationContext, event: &mut ComponentTypesEvent<'_>) {
            event.register(POUCH_COMPONENT.into());
        }
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let key = r.item_key("minecraft:paper");
            r.item_part(
                key,
                PouchPart {
                    config: self.config.clone(),
                    panic_stack: self.panic_stack.clone(),
                },
            );
        }
    }

    #[test]
    #[ignore = "daily"]
    fn custom_bundle_transactional_stacking_lan_and_destruction_drops() {
        let config = BundleConfig::custom(POUCH_COMPONENT, 4).with_filter_tag("minecraft:arrows");
        let panic_stack = Arc::new(AtomicBool::new(false));
        let mods: Mods = vec![ModEntry::new(
            "full_pouch",
            Box::new(FullPouchMod {
                config: config.clone(),
                panic_stack: panic_stack.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods.clone(), || {
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            with_guests(|game| {
                // 1. Server inventory click: carried pouch inserts clicked arrows (`CarriedOnOther`).
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.inventory = vec![None; guest.inventory.len()];
                        guest.carried = Some(empty_pouch(1));
                        guest.inventory[9] = Some(ItemStack::new("minecraft:arrow", 12));
                    });
                game.lan_guest_packet(
                    12,
                    Serverbound::ContainerClick(ContainerClick {
                        container: 0,
                        state: 0,
                        slot: 9,
                        button: 0,
                        input: 0,
                        changed: Vec::new(),
                        carried: None,
                    }),
                );
                let pouch = {
                    let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
                    assert!(guest.inventory[9].is_none());
                    guest.carried.clone().unwrap()
                };
                let slots = game.bundle_slots(&pouch, &config).unwrap().unwrap();
                assert_eq!(slots, vec![Some(ItemStack::new("minecraft:arrow", 12))]);

                // 2. LAN guest click: carried arrows into clicked pouch (`OtherStackedOnMe`).
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.inventory = vec![None; guest.inventory.len()];
                        guest.inventory[9] = Some(pouch.clone());
                        guest.carried = Some(ItemStack::new("minecraft:arrow", 8));
                    });
                game.lan_guest_packet(
                    12,
                    Serverbound::ContainerClick(ContainerClick {
                        container: 0,
                        state: 0,
                        slot: 9,
                        button: 0,
                        input: 0,
                        changed: Vec::new(),
                        carried: None,
                    }),
                );
                let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
                assert!(guest.carried.is_none());
                let updated = guest.inventory[9].clone().unwrap();
                assert_eq!(
                    game.bundle_slots(&updated, &config).unwrap().unwrap(),
                    vec![Some(ItemStack::new("minecraft:arrow", 20))]
                );

                // 3. Rollback on panic preserves both cursor and clicked slot (run after the
                // successful clicks because a panicking callback is retired for the session).
                panic_stack.store(true, Ordering::Relaxed);
                let before_carried = Some(pouch);
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.carried = before_carried.clone();
                        guest.inventory[10] = Some(ItemStack::new("minecraft:arrow", 5));
                    });
                game.lan_guest_packet(
                    12,
                    Serverbound::ContainerClick(ContainerClick {
                        container: 0,
                        state: 0,
                        slot: 10,
                        button: 0,
                        input: 0,
                        changed: Vec::new(),
                        carried: None,
                    }),
                );
                let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
                assert_eq!(guest.carried, before_carried);
                assert_eq!(
                    guest.inventory[10].as_ref().map(|s| (&*s.item, s.count)),
                    Some(("minecraft:arrow", 5))
                );
                panic_stack.store(false, Ordering::Relaxed);

                // 4. Dropped custom pouch destroyed by damage spills its stored arrows.
                game.world = sunrise_level::world::World::new(game.res.facts().air, 1);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                game.dropped.clear();
                game.spawn_item(updated, [4.5, 64.0, 4.5], [0.0; 3], 0);
                let id = game.dropped.last().unwrap().id;
                assert!(game.hurt_item_entity(id, 10.0, "minecraft:on_fire", None, None, None));
                assert!(game.dropped.iter().all(|e| e.id != id));
                assert_eq!(game.dropped.len(), 1);
                assert_eq!(&*game.dropped[0].stack.item, "minecraft:arrow");
                assert_eq!(game.dropped[0].stack.count, 20);
            });
        });
    }
}
