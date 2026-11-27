#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {
    use sunrise_gameplay_common::game::server_side::native_harvest_queries::tests::{
        Asker, RecursiveAsker,
    };

    use mod_api::__private::game::Fields;
    use sunrise_gameplay_common::game::native_block_tables::BlockTables;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_harvest_queries_common::NativeHarvestQueriesCommonGameExt1;
    use sunrise_gameplay_common::game::native_server::NativeServerGameExt1;

    use super::super::native_guest_turn_tests::with_guests;

    use mod_api::__private::game::player_ref;

    use std::sync::{Arc, Mutex};

    use sunrise_items::items::ItemStack;

    /// A stack is the correct tool by its own rules only; a player is by their selected slot
    /// (never the offhand), and a block that needs no tool is harvested by an empty hand.
    #[test]
    #[ignore = "daily"]
    fn the_correct_tool_is_the_stacks_rules_and_the_players_selected_slot() {
        with_guests(|game| {
            let state = |name: &str| game.pack.assets.registry.default_state(name).unwrap();
            let (stone, dirt) = (state("minecraft:stone"), state("minecraft:dirt"));
            let pickaxe = ItemStack::new("minecraft:iron_pickaxe", 1);
            let shovel = ItemStack::new("minecraft:iron_shovel", 1);
            // The stack alone: dirt needs no tool, yet a pickaxe is not its correct tool.
            assert_eq!(
                BlockTables::of(game).block_state_correct_tool_for_drops(&pickaxe, stone),
                Some(true)
            );
            assert_eq!(
                BlockTables::of(game).block_state_correct_tool_for_drops(&shovel, stone),
                Some(false)
            );
            assert_eq!(
                BlockTables::of(game).block_state_correct_tool_for_drops(&pickaxe, dirt),
                Some(false)
            );
            assert_eq!(
                BlockTables::of(game).block_state_correct_tool_for_drops(
                    &ItemStack::new("minecraft:stick", 1),
                    stone
                ),
                Some(false)
            );
            assert_eq!(
                BlockTables::of(game).block_state_correct_tool_for_drops(
                    &pickaxe,
                    sunrise_world::registry::BlockStateId(u32::MAX)
                ),
                None
            );

            let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
            let level = a.level(&*game).unwrap();
            let hold = |game: &mut sunrise_gameplay::game::ServerGame<'_>,
                        conn,
                        main: Option<ItemStack>,
                        off: Option<ItemStack>| {
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(conn, |guest| {
                        guest.inventory[guest.selected] = main;
                        guest.offhand = off;
                    })
                    .unwrap();
            };
            hold(game, 12, Some(pickaxe.clone()), None);
            hold(game, 13, None, Some(pickaxe));
            let ask = |game: &mut sunrise_gameplay::game::ServerGame<'_>, p, s| {
                game.player_has_correct_tool_for_drops(p, level, [0, 64, 0], s)
            };
            assert_eq!(ask(game, a, stone), Some(true));
            assert_eq!(
                ask(game, b, stone),
                Some(false),
                "the offhand does not count"
            );
            assert_eq!(ask(game, a, dirt), Some(true));
            assert_eq!(ask(game, b, dirt), Some(true), "dirt needs no tool");
            // During another player's turn the displaced player still answers from their own slot.
            let (outer, inner) = game
                .as_guest(13, |game| (ask(game, a, stone), ask(game, b, stone)))
                .unwrap();
            assert_eq!((outer, inner), (Some(true), Some(false)));
            // A handle that names nobody here asks nothing.
            let stale = player_ref(
                mod_api::EntityRef::new(a.fields().entity.epoch(), a.fields().entity.id() + 100),
                a.uuid(),
            );
            assert_eq!(ask(game, stale, stone), None);
        });
    }
    /// A supplied state answers the dig's own math for that state, not the block at the
    /// position, and an unknown state asks nothing.
    #[test]
    #[ignore = "daily"]
    fn the_destroy_progress_query_uses_the_supplied_state() {
        with_guests(|game| {
            let (stone, dirt) = {
                let host = &*game as &dyn mod_api::Host;
                (
                    host.block_state_id("minecraft:stone").unwrap(),
                    host.block_state_id("minecraft:dirt").unwrap(),
                )
            };
            let player = game.player(12).unwrap();
            let level = player.level(&*game).unwrap();
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| {
                    guest.on_ground = true;
                    guest.inventory[guest.selected] =
                        Some(ItemStack::new("minecraft:iron_pickaxe", 1));
                })
                .unwrap();
            let (rock, soil) = (6.0 / 1.5 / 30.0, 1.0 / 0.5 / 30.0);
            assert_eq!(
                player.destroy_progress(game, level, [0, 64, 0], stone),
                Some(rock)
            );
            assert_eq!(
                player.destroy_progress(game, level, [0, 64, 0], dirt),
                Some(soil)
            );
            let unknown = mod_api::__private::block_state_from_ordinal(stone.block(), u16::MAX);
            assert_eq!(
                player.destroy_progress(game, level, [0, 64, 0], unknown),
                None
            );
        })
    }
    /// Two fake players answer from their own selected slots and run the ordinary hooks.
    /// From inside those hooks the query refuses even a different state.
    #[test]
    #[ignore = "daily"]
    fn the_query_from_inside_a_harvest_hook_answers_none() {
        let inner = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "asker",
            Box::new(Asker(inner.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_guests(|game| {
                let level = game.level(game.dim.id()).unwrap();
                let stone = (game as &dyn mod_api::Host)
                    .block_state_id("minecraft:stone")
                    .unwrap();
                game.server.next_entity = 100;
                let fakes = [
                    (1234, "pickaxe", "minecraft:iron_pickaxe", true),
                    (5678, "shovel", "minecraft:iron_shovel", false),
                ]
                .map(|(uuid, name, item, expected)| {
                    let player = game.fake_player(level, uuid, name).unwrap();
                    let hub = &mut game.server.lan.as_mut().unwrap().hub;
                    let index = hub
                        .guests
                        .identity_index(uuid, player.entity().id())
                        .unwrap();
                    hub.guests.update(index, |guest| {
                        guest.selected = 4;
                        guest.inventory[4] = Some(ItemStack::new(item, 1));
                        guest.offhand = Some(ItemStack::new("minecraft:iron_pickaxe", 1));
                    });
                    (player, expected)
                });
                for (player, expected) in fakes {
                    assert_eq!(
                        player.has_correct_tool_for_drops(game, level, [0, 64, 0], stone),
                        Some(expected)
                    );
                    assert_eq!(
                        std::mem::take(&mut *inner.lock().unwrap()),
                        [(player, None)]
                    );
                }
            });
        });
    }
    /// The query refuses from every hook it would otherwise run again: the two harvest hooks
    /// and the break-speed hook.
    #[test]
    #[ignore = "daily"]
    fn the_destroy_progress_query_from_inside_any_of_its_hooks_answers_none() {
        let asker = RecursiveAsker::default();
        let seen = Arc::clone(&asker.0);
        let mods = vec![mod_api::ModEntry::new("recursive", Box::new(asker))].into();
        mod_platform::testing::with_mods(mods, || {
            with_guests(|game| {
                let player = game.player(12).unwrap();
                let level = player.level(&*game).unwrap();
                let stone = (game as &dyn mod_api::Host)
                    .block_state_id("minecraft:stone")
                    .unwrap();
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| {
                        guest.inventory[guest.selected] =
                            Some(ItemStack::new("minecraft:iron_pickaxe", 1));
                    })
                    .unwrap();
                // The outer query runs all three hooks; each inner query answers None.
                assert!(
                    player
                        .destroy_progress(game, level, [0, 64, 0], stone)
                        .is_some()
                );
                assert_eq!(
                    std::mem::take(&mut *seen.lock().unwrap()),
                    [
                        ("can_harvest_block", None),
                        ("player_harvest_check", None),
                        ("player_break_speed", None),
                    ]
                );
            });
        });
    }

    /// A torch that mines as the block below it, through the query, as a block overriding
    /// `getDestroyProgress` does; and dirt that answers a progress of its own.
    struct MinesAsBelow;

    #[mod_api::sunrise_part]
    impl mod_api::BlockPart for MinesAsBelow {
        fn destroy_progress(
            &self,
            host: mod_api::HostSide<'_>,
            event: &mut mod_api::DestroyProgressEvent,
        ) -> Option<f32> {
            let mod_api::HostSide::Server(host) = host else {
                return None;
            };
            let level = event.player.level(host)?;
            let below = [event.pos[0], event.pos[1] - 1, event.pos[2]];
            let pedestal = level.block_state(host, below)?;
            event.player.destroy_progress(host, level, below, pedestal)
        }
    }

    struct Soft;

    #[mod_api::sunrise_part]
    impl mod_api::BlockPart for Soft {
        fn destroy_progress(
            &self,
            _: mod_api::HostSide<'_>,
            _: &mut mod_api::DestroyProgressEvent,
        ) -> Option<f32> {
            Some(0.25)
        }
    }

    struct PartsMod;

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for PartsMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let torch = r.block_key("minecraft:torch");
            r.block_part(torch, MinesAsBelow);
            let dirt = r.block_key("minecraft:dirt");
            r.block_part(dirt, Soft);
        }
    }

    /// A block Part's answer is the dig's progress and the query's: dirt answers its own 0.25,
    /// and a torch mines as the block below it, through the query, which inside the answer is
    /// the game's math and asks no Part: on stone as stone, on dirt as dirt with no Part (not
    /// 0.25), on air as air and on another torch as that torch (hardness 0, at once).
    #[test]
    #[ignore = "daily"]
    fn a_block_part_answers_the_destroy_progress() {
        use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;
        let mods = vec![mod_api::ModEntry::new("parts", Box::new(PartsMod))].into();
        mod_platform::testing::with_mods(mods, || {
            with_guests(|game| {
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                let state = |game: &sunrise_gameplay::game::ServerGame<'_>, name: &str| {
                    game.pack.assets.registry.default_state(name).unwrap()
                };
                for (x, below) in [
                    (0, "minecraft:stone"),
                    (1, "minecraft:dirt"),
                    (2, "minecraft:air"),
                    (3, "minecraft:torch"),
                ] {
                    let below = state(game, below);
                    game.world.set(x, 64, 0, below).unwrap();
                    let torch = state(game, "minecraft:torch");
                    game.world.set(x, 65, 0, torch).unwrap();
                }
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| {
                        guest.on_ground = true;
                        guest.inventory[guest.selected] =
                            Some(ItemStack::new("minecraft:iron_pickaxe", 1));
                    })
                    .unwrap();
                let player = game.player(12).unwrap();
                let level = player.level(&*game).unwrap();
                let torch = (game as &dyn mod_api::Host)
                    .block_state_id("minecraft:torch")
                    .unwrap();
                let dirt = (game as &dyn mod_api::Host)
                    .block_state_id("minecraft:dirt")
                    .unwrap();
                let at = [1, 64, 0];
                assert_eq!(player.destroy_progress(game, level, at, dirt), Some(0.25));
                assert_eq!(
                    game.as_guest(12, |game| game.destroy_progress(at)),
                    Some(0.25)
                );
                // An iron pickaxe on stone (hardness 1.5, the correct tool) and on dirt
                // (hardness 0.5, speed 1, still harvestable); air and a torch have hardness 0.
                let rock = 6.0 / 1.5 / 30.0;
                let soil = 1.0 / 0.5 / 30.0;
                let expected = [rock, soil, f32::INFINITY, f32::INFINITY];
                for (x, expected) in expected.into_iter().enumerate() {
                    let pos = [x as i32, 65, 0];
                    assert_eq!(
                        player.destroy_progress(game, level, pos, torch),
                        Some(expected),
                        "query at {pos:?}"
                    );
                    assert_eq!(
                        game.as_guest(12, |game| game.destroy_progress(pos)),
                        Some(expected),
                        "dig at {pos:?}"
                    );
                }
            });
        });
    }
}
