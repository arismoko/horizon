use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use sunrise_gameplay_common::game::Hit;
use sunrise_gameplay_common::game::TryIntoApi;
use sunrise_items::items::ItemStack;
use sunrise_sim::first_person_sim::Hand;

use sunrise_gameplay::game::server_side::advancements::block_use_recording::{
    Recording, SEEN, counts,
};

#[test]
#[ignore = "daily"]
fn consuming_cancellation_has_one_outer_on_local_and_packet_routes() {
    let _recording = Recording::start();
    struct Listener(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Listener {
        fn right_click_block(
            &self,
            host: mod_api::HostSide,
            event: &mut mod_api::RightClickBlockEvent<'_>,
        ) -> mod_api::Flow {
            let mode = self.0.load(Ordering::SeqCst);
            if mode == 0 {
                return mod_api::Flow::Continue;
            }
            if let mod_api::HostSide::Server(host) = host {
                if mode == 1 {
                    let player = host.player(event.click.player).unwrap();
                    player.trigger_block_use(host, event.click.pos).unwrap();
                } else if mode == 4 {
                    let player = host.player(event.click.player).unwrap();
                    player
                        .trigger_item_used_on_block(
                            host,
                            event.click.pos,
                            event.click.stack.unwrap(),
                        )
                        .unwrap();
                }
            }
            *event.cancellation_result = match mode {
                2 => mod_api::InteractionResult::Fail,
                5 => mod_api::InteractionResult::Pass,
                6 => mod_api::InteractionResult::Success,
                7 => mod_api::InteractionResult::SuccessServer,
                _ => mod_api::InteractionResult::Consume,
            };
            mod_api::Flow::Cancel
        }
    }
    let mode = Arc::new(AtomicUsize::new(0));
    mod_platform::__private::install(Arc::from([mod_api::ModEntry::new(
        "count-block-use",
        Box::new(Listener(mode.clone())),
    )]));
    sunrise_gameplay::game::native_guest_turn_tests::with_guests(|g| {
        g.build_registries();
        g.world.chunks.insert(
            (0, 0),
            Arc::new({
                let mut chunk = sunrise_level::chunk::Chunk::empty(0, 0);
                chunk.lit = true; // a FULL read of a hand-made column finds it ready
                chunk
            }),
        );
        let state = g
            .pack
            .assets
            .registry
            .default_state("minecraft:oak_trapdoor")
            .unwrap();
        let hit = Hit {
            pos: [3, 64, 4],
            face: [0, 1, 0],
            point: [3.5, 65.0, 4.5],
            inside: false,
        };
        for packet in [false, true] {
            for (answer, expected) in [
                (0, [1, 0, 1]),
                (1, [1, 0, 1]),
                (2, [0, 0, 0]),
                (3, [0, 0, 1]),
                (4, [0, 1, 1]),
                (5, [0, 0, 0]),
                (6, [0, 0, 1]),
                (7, [0, 0, 1]),
            ] {
                mode.store(answer, Ordering::SeqCst);
                g.world.set(3, 64, 4, state).unwrap();
                g.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| {
                        guest.inventory[0] =
                            (answer == 4).then(|| ItemStack::new("minecraft:stone", 2));
                        guest.offhand = None;
                    })
                    .unwrap();
                SEEN.with_borrow_mut(Vec::clear);
                if packet {
                    let gi = g
                        .server
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guests
                        .iter()
                        .position(|guest| guest.conn == 12)
                        .unwrap();
                    assert!(g.guest_use_item_on(12, gi, hit, Hand::Main, None).is_ok());
                } else {
                    g.as_guest(12, |g| {
                        g.server.forced_hit = Some(hit);
                        g.use_hand(Some(&hit)).unwrap();
                        g.server.forced_hit = None;
                    })
                    .unwrap();
                }
                assert_eq!(counts(), expected, "packet={packet}, mode={answer}");
            }
        }
        // Item-internal triggers and the game-mode item trigger both run.
        mode.store(0, Ordering::SeqCst);
        for (block, item) in [
            ("minecraft:copper_block", "minecraft:honeycomb"),
            ("minecraft:cave_vines", "minecraft:shears"),
        ] {
            let initial = g.pack.assets.registry.default_state(block).unwrap();
            for packet in [false, true] {
                g.world.set(3, 64, 4, initial).unwrap();
                g.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| {
                        guest.inventory[0] = Some(ItemStack::new(item, 1));
                    })
                    .unwrap();
                SEEN.with_borrow_mut(Vec::clear);
                if packet {
                    let gi = g
                        .server
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guests
                        .iter()
                        .position(|guest| guest.conn == 12)
                        .unwrap();
                    assert!(g.guest_use_item_on(12, gi, hit, Hand::Main, None).is_ok());
                } else {
                    g.as_guest(12, |g| {
                        g.server.forced_hit = Some(hit);
                        g.use_hand(Some(&hit)).unwrap();
                        g.server.forced_hit = None;
                    })
                    .unwrap();
                }
                assert_ne!(
                    g.world.block(3, 64, 4),
                    initial,
                    "packet={packet}, item={item}"
                );
                assert_eq!(counts(), [0, 2, 1], "packet={packet}, item={item}");
            }
        }
        // Filling a cauldron replaces the hand but preserves the original water bucket.
        mode.store(0, Ordering::SeqCst);
        let cauldron = g
            .pack
            .assets
            .registry
            .default_state("minecraft:cauldron")
            .unwrap();
        for packet in [false, true] {
            for original_item in ["minecraft:water_bucket", "minecraft:bucket"] {
                if original_item == "minecraft:water_bucket" {
                    g.world.set(3, 64, 4, cauldron).unwrap();
                }
                g.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| {
                        guest.inventory[0] = Some(ItemStack::new(original_item, 1));
                    })
                    .unwrap();
                SEEN.with_borrow_mut(Vec::clear);
                if packet {
                    let gi = g
                        .server
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guests
                        .iter()
                        .position(|guest| guest.conn == 12)
                        .unwrap();
                    assert!(g.guest_use_item_on(12, gi, hit, Hand::Main, None).is_ok());
                } else {
                    g.as_guest(12, |g| {
                        g.server.forced_hit = Some(hit);
                        g.use_hand(Some(&hit)).unwrap();
                        g.server.forced_hit = None;
                    })
                    .unwrap();
                }
                g.as_guest(12, |g| {
                    let final_item = if original_item == "minecraft:bucket" {
                        "minecraft:water_bucket"
                    } else {
                        "minecraft:bucket"
                    };
                    assert_eq!(g.held(), Some(&ItemStack::new(final_item, 1)));
                })
                .unwrap();
                assert_eq!(counts()[2], 1);
                SEEN.with_borrow(|seen| {
                    let stack = &seen
                        .iter()
                        .find(|(name, _)| name == "minecraft:any_block_use")
                        .unwrap()
                        .1;
                    assert_eq!(stack.as_ref(), Some(&ItemStack::new(original_item, 1)));
                });
            }
        }
    });
}

/// ServerPlayerGameMode.useItemOn copies the hand before RespawnAnchorBlock
/// consumes fuel; handleUseItemOn fires ANY_BLOCK_USE only after that result.
#[test]
#[ignore = "daily"]
fn resource_gated_block_use_order_and_stacks_on_server_and_guest() {
    let _recording = Recording::start();
    sunrise_gameplay::game::native_guest_turn_tests::with_guests(|g| {
        sunrise_gameplay::game::native_guest_turn_tests::place_host(g);
        g.world.chunks.insert(
            (0, 0),
            Arc::new({
                let mut chunk = sunrise_level::chunk::Chunk::empty(0, 0);
                chunk.lit = true; // a FULL read of a hand-made column finds it ready
                chunk
            }),
        );
        let anchor = g
            .pack
            .assets
            .registry
            .default_state("minecraft:respawn_anchor")
            .unwrap();
        let hit = Hit {
            pos: [3, 64, 4],
            face: [0, 1, 0],
            point: [3.5, 65.0, 4.5],
            inside: false,
        };
        for packet in [false, true] {
            g.world.set(3, 64, 4, anchor).unwrap();
            let before = ItemStack::new("minecraft:glowstone", 5);
            let after = ItemStack::new("minecraft:glowstone", 4);
            SEEN.with_borrow_mut(Vec::clear);
            if packet {
                g.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| {
                        guest.inventory[guest.selected] = Some(before.clone());
                        guest.offhand = None;
                    })
                    .unwrap();
                let gi = g
                    .server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guests
                    .iter()
                    .position(|guest| guest.conn == 12)
                    .unwrap();
                assert!(matches!(
                    g.guest_use_item_on(12, gi, hit, Hand::Main, None),
                    Ok(true)
                ));
                g.as_guest(12, |g| assert_eq!(g.held(), Some(&after)))
                    .unwrap();
            } else {
                // Exercise the local server's outer use_target_then_item route,
                // not another call through guest_use_item_on.
                g.set_mode_id(0);
                g.common.inventory[g.common.selected] = Some(before.clone());
                g.offhand = None;
                g.server.forced_hit = Some(hit);
                assert_eq!(g.use_item_as_player().unwrap(), Some(Hand::Main));
                g.server.forced_hit = None;
                assert_eq!(g.held(), Some(&after));
            }
            SEEN.with_borrow(|seen| {
                assert_eq!(
                    seen,
                    &[
                        ("minecraft:item_used_on_block".to_owned(), Some(before)),
                        ("minecraft:any_block_use".to_owned(), Some(after)),
                    ],
                    "packet={packet}"
                );
            });
            let state = g.world.block(3, 64, 4);
            assert_eq!(
                g.pack.assets.registry.state(state).unwrap().properties["charges"],
                "1",
                "packet={packet}"
            );
        }
    });
}

/// MapItem.useOn toggles the marker once; the shared pre-block snapshot must
/// not cause a second ITEM_USED_ON_BLOCK emission in the enclosing use path.
#[test]
#[ignore = "daily"]
fn resource_gated_map_on_banner_fires_once_and_toggles_the_real_marker() {
    let _recording = Recording::start();
    sunrise_gameplay::game::native_guest_turn_tests::with_guests(|g| {
        g.world.chunks.insert(
            (0, 0),
            Arc::new({
                let mut chunk = sunrise_level::chunk::Chunk::empty(0, 0);
                chunk.lit = true; // a FULL read of a hand-made column finds it ready
                chunk
            }),
        );
        let banner = g
            .pack
            .assets
            .registry
            .default_state("minecraft:red_banner")
            .unwrap();
        g.world.set(3, 64, 4, banner).unwrap();
        g.world.block_entities.insert(
            (3, 64, 4),
            sunrise_level::containers::BlockEntity::Banner(sunrise_level::containers::Banner {
                layers: Vec::new(),
                name: None,
                components: Default::default(),
            }),
        );
        g.maps.data.insert(
            42,
            sunrise_gameplay_common::game::maps::MapData::fresh(0.0, 0.0, 0, "minecraft:overworld"),
        );
        let mut stack = ItemStack::new("minecraft:filled_map", 1);
        stack.data_mut().map_id = Some(42);
        g.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(12, |guest| {
                guest.inventory[guest.selected] = Some(stack.clone());
                guest.offhand = None;
            })
            .unwrap();
        let hit = Hit {
            pos: [3, 64, 4],
            face: [0, 1, 0],
            point: [3.5, 65.0, 4.5],
            inside: false,
        };
        for marked in [true, false] {
            SEEN.with_borrow_mut(Vec::clear);
            let gi = g
                .server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guests
                .iter()
                .position(|guest| guest.conn == 12)
                .unwrap();
            assert!(matches!(
                g.guest_use_item_on(12, gi, hit, Hand::Main, None),
                Ok(true)
            ));
            SEEN.with_borrow(|seen| {
                assert_eq!(
                    seen,
                    &[
                        (
                            "minecraft:item_used_on_block".to_owned(),
                            Some(stack.clone())
                        ),
                        ("minecraft:any_block_use".to_owned(), Some(stack.clone())),
                    ]
                );
            });
            let map = &g.maps.data[&42];
            if marked {
                assert_eq!(map.banners.len(), 1);
                assert_eq!(map.banners[0].pos, hit.pos);
                assert_eq!(map.banners[0].color, "red");
            } else {
                assert!(map.banners.is_empty());
            }
            g.as_guest(12, |g| assert_eq!(g.held(), Some(&stack)))
                .unwrap();
        }
    });
}

fn any_block_use_stacks() -> Vec<ItemStack> {
    SEEN.with_borrow(|seen| {
        seen.iter()
            .filter(|(name, _)| name == "minecraft:any_block_use")
            .map(|(_, stack)| stack.clone().unwrap())
            .collect()
    })
}

#[test]
#[ignore = "daily"]
fn block_use_original_shrink_and_damage_values() {
    let _recording = Recording::start();
    sunrise_gameplay::game::native_guest_turn_tests::with_guests(|g| {
        g.set_mode_id(0);
        g.common.inventory[g.common.selected] = Some(ItemStack::new("minecraft:glowstone", 5));
        g.with_block_use_original(Hand::Main, |g| {
            g.consume_held();
            g.finish_block_use([3, 64, 4], Hand::Main);
        });
        assert_eq!(
            any_block_use_stacks(),
            [ItemStack::new("minecraft:glowstone", 4)]
        );
        SEEN.with_borrow_mut(Vec::clear);
        g.common.inventory[g.common.selected] = Some(ItemStack::new("minecraft:iron_shovel", 1));
        g.with_block_use_original(Hand::Main, |g| {
            g.damage_held(1);
            g.finish_block_use([3, 64, 4], Hand::Main);
        });
        let mut expected = ItemStack::new("minecraft:iron_shovel", 1);
        expected.damage = 1;
        assert_eq!(any_block_use_stacks(), [expected]);
    });
}

#[test]
#[ignore = "daily"]
fn block_use_listener_replacements_keep_original_on_both_routes() {
    sunrise_gameplay::game::native_entity_refs::tests::run_with_game_stack(|| {
        struct Replace(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Replace {
            fn right_click_block(
                &self,
                host: mod_api::HostSide,
                event: &mut mod_api::RightClickBlockEvent<'_>,
            ) -> mod_api::Flow {
                let mode = self.0.load(Ordering::SeqCst);
                if let mod_api::HostSide::Server(host) = host {
                    let item = if mode == 0 {
                        "minecraft:diamond"
                    } else {
                        "minecraft:glowstone"
                    };
                    let player = host.player(event.click.player).unwrap();
                    let living = host.living(player.entity()).unwrap();
                    living
                        .set_hand_item(
                            host,
                            Hand::Main,
                            Some(ItemStack::new(item, 19).try_into_api().unwrap()),
                        )
                        .unwrap();
                }
                if mode == 4 {
                    return mod_api::Flow::Continue;
                }
                *event.cancellation_result = mod_api::InteractionResult::Consume;
                mod_api::Flow::Cancel
            }
        }
        let mode = Arc::new(AtomicUsize::new(0));
        mod_platform::__private::install(Arc::from([mod_api::ModEntry::new(
            "replace-original",
            Box::new(Replace(mode.clone())),
        )]));
        let _recording = Recording::start();
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|g| {
            g.build_registries();
            g.world.chunks.insert(
                (0, 0),
                Arc::new({
                    let mut chunk = sunrise_level::chunk::Chunk::empty(0, 0);
                    chunk.lit = true; // a FULL read of a hand-made column finds it ready
                    chunk
                }),
            );
            let hit = Hit {
                pos: [3, 64, 4],
                face: [0, 1, 0],
                point: [3.5, 65.0, 4.5],
                inside: false,
            };
            for replacement in [0, 1, 4] {
                mode.store(replacement, Ordering::SeqCst);
                for packet in [false, true] {
                    let anchor = g
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:respawn_anchor")
                        .unwrap();
                    g.world.set(3, 64, 4, anchor).unwrap();
                    g.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .update_guest(12, |guest| {
                            guest.inventory[guest.selected] =
                                Some(ItemStack::new("minecraft:glowstone", 5));
                            guest.offhand = None;
                        })
                        .unwrap();
                    SEEN.with_borrow_mut(Vec::clear);
                    if packet {
                        let gi = g
                            .server
                            .lan
                            .as_ref()
                            .unwrap()
                            .hub
                            .guests
                            .iter()
                            .position(|guest| guest.conn == 12)
                            .unwrap();
                        assert!(g.guest_use_item_on(12, gi, hit, Hand::Main, None).is_ok());
                    } else {
                        g.as_guest(12, |g| {
                            g.server.forced_hit = Some(hit);
                            g.use_hand(Some(&hit)).unwrap();
                            g.server.forced_hit = None;
                        })
                        .unwrap();
                    }
                    g.as_guest(12, |g| {
                        let expected_item = if replacement == 0 {
                            "minecraft:diamond"
                        } else {
                            "minecraft:glowstone"
                        };
                        assert_eq!(
                            g.held(),
                            Some(&ItemStack::new(
                                expected_item,
                                if replacement >= 4 { 18 } else { 19 }
                            ))
                        );
                    })
                    .unwrap();
                    assert_eq!(
                        counts(),
                        if replacement >= 4 {
                            [0, 1, 1]
                        } else {
                            [0, 0, 1]
                        }
                    );
                    assert_eq!(
                        any_block_use_stacks(),
                        [ItemStack::new(
                            "minecraft:glowstone",
                            if replacement >= 4 { 4 } else { 5 }
                        )],
                        "replacement={replacement}, packet={packet}"
                    );
                }
            }
        });
    });
}

#[test]
#[ignore = "daily"]
fn block_use_nested_originals_and_replacements_are_isolated() {
    let _recording = Recording::start();
    sunrise_gameplay::game::native_guest_turn_tests::with_guests(|g| {
        g.set_mode_id(0);
        g.common.inventory[g.common.selected] = Some(ItemStack::new("minecraft:glowstone", 5));
        g.with_block_use_original(Hand::Main, |g| {
            g.consume_held();
            *g.held_mut() = Some(ItemStack::new("minecraft:bucket", 1));
            g.with_block_use_original(Hand::Main, |g| {
                *g.held_mut() = Some(ItemStack::new("minecraft:water_bucket", 1));
                g.finish_block_use([3, 64, 4], Hand::Main);
            });
            g.finish_block_use([3, 64, 4], Hand::Main);
        });
        assert_eq!(
            any_block_use_stacks(),
            [
                ItemStack::new("minecraft:bucket", 1),
                ItemStack::new("minecraft:glowstone", 4)
            ]
        );
        assert!(g.server.block_use_original.is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn block_use_registry_guard_final_hand_matches_original() {
    let _recording = Recording::start();
    let scan_registry = |g: &mut sunrise_gameplay::game::ServerGame<'_>| {
        g.build_registries();
        g.set_mode_id(0);
        g.world.chunks.insert(
            (0, 0),
            Arc::new({
                let mut chunk = sunrise_level::chunk::Chunk::empty(0, 0);
                chunk.lit = true; // a FULL read of a hand-made column finds it ready
                chunk
            }),
        );
        let stone = g
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let air = g
            .pack
            .assets
            .registry
            .default_state("minecraft:air")
            .unwrap();
        let hit = Hit {
            pos: [3, 64, 4],
            face: [0, 1, 0],
            point: [3.5, 65.0, 4.5],
            inside: false,
        };
        // Item bindings describe block placement; no exhaustive useOn marker exists.
        // Iterate the entire item registry, a superset including non-block tools.
        let items = g.items().order.clone();
        let mut reports = 0;
        for item in items {
            g.world.set(3, 64, 4, stone).unwrap();
            g.world.set(3, 65, 4, air).unwrap();
            g.common.inventory[g.common.selected] = Some(ItemStack::new(&item, 5));
            g.offhand = None;
            SEEN.with_borrow_mut(Vec::clear);
            g.server.forced_hit = Some(hit);
            g.use_hand(Some(&hit)).unwrap();
            g.server.forced_hit = None;
            for reported in any_block_use_stacks() {
                reports += 1;
                let final_stack = g
                    .held()
                    .cloned()
                    .unwrap_or_else(|| ItemStack::new("minecraft:air", 0));
                assert!(
                    reported == final_stack || final_stack.item != item,
                    "untracked in-place block use for {item}: reported={reported:?}, final={final_stack:?}"
                );
            }
        }
        assert!(
            reports > 0,
            "registry guard exercised no consuming block uses"
        );
    };
    mod_platform::testing::with_mods(Arc::from([]), || {
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(scan_registry);
    });
}

fn run_block_use_regression(
    g: &mut sunrise_gameplay::game::ServerGame<'_>,
    packet: bool,
    stack: ItemStack,
) {
    let hit = Hit {
        pos: [3, 64, 4],
        face: [0, 1, 0],
        point: [3.5, 65.0, 4.5],
        inside: false,
    };
    g.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .update_guest(12, |guest| {
            guest.inventory[guest.selected] = Some(stack);
            guest.offhand = None;
        })
        .unwrap();
    SEEN.with_borrow_mut(Vec::clear);
    if packet {
        let gi = g
            .server
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guests
            .iter()
            .position(|guest| guest.conn == 12)
            .unwrap();
        assert!(g.guest_use_item_on(12, gi, hit, Hand::Main, None).is_ok());
    } else {
        g.as_guest(12, |g| {
            g.set_mode_id(0);
            g.server.forced_hit = Some(hit);
            assert!(g.use_hand(Some(&hit)).unwrap());
            g.server.forced_hit = None;
        })
        .unwrap();
    }
}

#[test]
#[ignore = "daily"]
fn block_use_hive_bottles_report_shrink_and_exhaustion_on_both_routes() {
    mod_platform::__private::install(Arc::from([]));
    let _recording = Recording::start();
    sunrise_gameplay::game::native_guest_turn_tests::with_guests(|g| {
        g.build_registries();
        g.world.chunks.insert(
            (0, 0),
            Arc::new({
                let mut chunk = sunrise_level::chunk::Chunk::empty(0, 0);
                chunk.lit = true; // a FULL read of a hand-made column finds it ready
                chunk
            }),
        );
        let registry = g.pack.assets.registry;
        let hive = registry.default_state("minecraft:beehive").unwrap();
        let mut full = registry.state(hive).unwrap().clone();
        full.properties.insert("honey_level".into(), "5".into());
        let full = registry.id(&full).unwrap();
        for packet in [false, true] {
            for (count, reported, final_hand) in [
                (
                    5,
                    ItemStack::new("minecraft:glass_bottle", 4),
                    ItemStack::new("minecraft:glass_bottle", 4),
                ),
                (
                    1,
                    ItemStack::new("minecraft:air", 0),
                    ItemStack::new("minecraft:honey_bottle", 1),
                ),
            ] {
                g.world.set(3, 64, 4, full).unwrap();
                run_block_use_regression(
                    g,
                    packet,
                    ItemStack::new("minecraft:glass_bottle", count),
                );
                assert_eq!(
                    any_block_use_stacks(),
                    [reported],
                    "packet={packet}, bottles={count}"
                );
                g.as_guest(12, |g| assert_eq!(g.held(), Some(&final_hand)))
                    .unwrap();
                let state = registry.state(g.world.block(3, 64, 4)).unwrap();
                assert_eq!(state.properties["honey_level"], "0");
            }
        }
    });
}

#[test]
#[ignore = "daily"]
fn block_use_lodestone_reports_component_edit_on_both_routes() {
    use sunrise_core::nbt::Tag;
    mod_platform::__private::install(Arc::from([]));
    let _recording = Recording::start();
    sunrise_gameplay::game::native_guest_turn_tests::with_guests(|g| {
        g.build_registries();
        g.world.chunks.insert(
            (0, 0),
            Arc::new({
                let mut chunk = sunrise_level::chunk::Chunk::empty(0, 0);
                chunk.lit = true; // a FULL read of a hand-made column finds it ready
                chunk
            }),
        );
        let lodestone = g
            .pack
            .assets
            .registry
            .default_state("minecraft:lodestone")
            .unwrap();
        let mut expected = ItemStack::new("minecraft:compass", 1);
        expected.data_mut().set_other(
            "minecraft:lodestone_tracker",
            Some(Tag::compound([
                (
                    "target",
                    Tag::compound([
                        ("dimension", Tag::String("minecraft:overworld".into())),
                        ("pos", Tag::IntArray(vec![3, 64, 4])),
                    ]),
                ),
                ("tracked", Tag::Byte(1)),
            ])),
        );
        for packet in [false, true] {
            g.world.set(3, 64, 4, lodestone).unwrap();
            run_block_use_regression(g, packet, ItemStack::new("minecraft:compass", 1));
            assert_eq!(
                any_block_use_stacks(),
                [expected.clone()],
                "packet={packet}"
            );
            g.as_guest(12, |g| assert_eq!(g.held(), Some(&expected)))
                .unwrap();
        }
    });
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;
