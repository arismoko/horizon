#![cfg(test)]
use mod_api::PlayerRef;
#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use super::*;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use sunrise_command::args::GameMode;
    use sunrise_core::geometry::Vec3;
    use sunrise_items::items::ItemStack;
    use sunrise_sim::{first_person_sim::Hand, motion::PlayerMotion};

    /// A server's game with its own player in survival standing at the
    /// origin chunk, holding `held`: their handle.
    fn with_player(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        held: ItemStack,
    ) -> PlayerRef {
        // The shared fixture does not open a server or register a ticking chunk.
        game.options.dedicated = Some(Vec::new());
        let _ = game
            .block_ticks
            .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0));
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 64.0, 4.5)).unwrap());
        game.server.lan_acting_uuid = Some(123);
        // The host acts, whose entity the hooks name as 0 while no guest acts.
        game.server.lan_acting_entity = Some(0);
        game.server.lan_acting_name = Some("Breaker".into());
        game.server.lan_acting_permission = Some(0);
        game.set_mode(GameMode::Survival);
        game.common.inventory[game.common.selected] = Some(held);
        game.native_acting_player().unwrap()
    }
    fn place(game: &mut sunrise_gameplay::game::ServerGame<'_>, pos: [i32; 3], block: &str) {
        let state = game.pack.assets.registry.default_state(block).unwrap();
        game.world.set(pos[0], pos[1], pos[2], state).unwrap();
    }
    fn block(game: &sunrise_gameplay::game::ServerGame<'_>, pos: [i32; 3]) -> String {
        let state = game.world.block(pos[0], pos[1], pos[2]);
        game.res.facts().get(state).name.to_string()
    }
    fn carried(game: &sunrise_gameplay::game::ServerGame<'_>, item: &str) -> u32 {
        let held = game.inventory.iter().flatten();
        held.filter(|s| &*s.item == item).map(|s| s.count).sum()
    }
    fn popped(game: &sunrise_gameplay::game::ServerGame<'_>, item: &str) -> bool {
        game.dropped.iter().any(|e| &*e.stack.item == item)
    }

    /// Stone broken with a diamond pickaxe drops cobblestone, wears the
    /// pickaxe by one and counts as mined (`Block.playerDestroy`,
    /// `ItemStack.mineBlock`), as a player's own break does.
    #[test]
    #[ignore = "daily"]
    fn a_mod_breaks_stone_as_the_player_with_their_pickaxe() {
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let player = with_player(game, ItemStack::new("minecraft:diamond_pickaxe", 1));
            let pos = [6, 64, 4];
            place(game, pos, "minecraft:stone");
            // A break requested from an offhand interaction still uses the main hand.
            game.offhand = Some(ItemStack::new("minecraft:debug_stick", 1));
            game.hand = Hand::Off;
            assert!(game.player_destroy_block(player, pos, false));
            assert!(matches!(game.hand, Hand::Off));
            assert_eq!(block(game, pos), "minecraft:air");
            assert!(popped(game, "minecraft:cobblestone"));
            assert_eq!(game.inventory[game.selected].as_ref().unwrap().damage, 1);
            assert_eq!(game.stats.get("mined", "minecraft:stone"), 1);
        });
    }

    /// Only the selected slot harvests a block that requires a tool. Iron doors do not
    /// require one in 26.3, so both hand arrangements harvest their lower-half drop.
    #[test]
    #[ignore = "daily"]
    fn harvesting_uses_the_selected_slot_and_iron_doors_need_no_tool() {
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let pickaxe = ItemStack::new("minecraft:iron_pickaxe", 1);
            let player = with_player(game, pickaxe.clone());
            let iron = [8, 64, 4];
            place(game, iron, "minecraft:iron_block");
            assert!(game.player_destroy_block(player, iron, false));
            assert!(
                popped(game, "minecraft:iron_block"),
                "selected pickaxe harvests"
            );
            game.dropped.clear();
            game.common.inventory[game.common.selected] = None;
            game.offhand = Some(pickaxe.clone());
            game.hand = sunrise_sim::first_person_sim::Hand::Off;
            place(game, iron, "minecraft:iron_block");
            assert!(game.player_destroy_block(player, iron, false));
            assert!(
                !popped(game, "minecraft:iron_block"),
                "offhand cannot harvest"
            );
            game.common.inventory[game.common.selected] = Some(pickaxe.clone());
            game.offhand = None;
            game.hand = sunrise_sim::first_person_sim::Hand::Main;
            let (lower, upper) = ([6, 64, 4], [6, 65, 4]);
            let door =
                |game: &mut sunrise_gameplay::game::ServerGame<'_>, half: &str, pos: [i32; 3]| {
                    let state = game
                        .pack
                        .assets
                        .registry
                        .resolve(&sunrise_world::chunk::BlockState {
                            name: "minecraft:iron_door".into(),
                            properties: [
                                ("facing", "north"),
                                ("half", half),
                                ("hinge", "left"),
                                ("open", "false"),
                                ("powered", "false"),
                            ]
                            .into_iter()
                            .map(|(k, v)| (k.into(), v.into()))
                            .collect(),
                        })
                        .unwrap();
                    game.world.set(pos[0], pos[1], pos[2], state).unwrap();
                };
            door(game, "lower", lower);
            door(game, "upper", upper);
            assert!(game.player_destroy_block(player, upper, false));
            assert_eq!(block(game, upper), "minecraft:air");
            // Vanilla takes the lower half out in the same call, by the neighbour update.
            game.server_process_level_events().unwrap();
            assert_eq!(
                block(game, lower),
                "minecraft:air",
                "main hand: correct tool"
            );
            assert!(popped(game, "minecraft:iron_door"));

            game.dropped.clear();
            game.common.inventory[game.common.selected] = None;
            game.offhand = Some(pickaxe);
            game.hand = sunrise_sim::first_person_sim::Hand::Off;
            door(game, "lower", lower);
            door(game, "upper", upper);
            assert_eq!(block(game, lower), "minecraft:iron_door");
            assert!(game.player_destroy_block(player, upper, false));
            assert_eq!(block(game, upper), "minecraft:air");
            assert_eq!(
                block(game, lower),
                "minecraft:air",
                "empty main hand: door needs no tool"
            );
            assert!(popped(game, "minecraft:iron_door"));
        });
    }

    /// Into the inventory: silk touch keeps the stone whole, and a diamond
    /// ore gives its diamond and still its experience (3 to 7 points).
    #[test]
    #[ignore = "daily"]
    fn drops_go_into_the_inventory_rolled_for_the_tools_enchantments() {
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let mut silk = ItemStack::new("minecraft:diamond_pickaxe", 1);
            silk.enchant("minecraft:silk_touch", 1);
            let player = with_player(game, silk);
            let (stone, ore) = ([6, 64, 4], [7, 64, 4]);
            place(game, stone, "minecraft:stone");
            assert!(game.player_destroy_block(player, stone, true));
            assert_eq!(carried(game, "minecraft:stone"), 1);
            assert!(!popped(game, "minecraft:stone"));

            game.common.inventory[game.common.selected] =
                Some(ItemStack::new("minecraft:diamond_pickaxe", 1));
            place(game, ore, "minecraft:diamond_ore");
            assert!(game.player_destroy_block(player, ore, true));
            assert_eq!(carried(game, "minecraft:diamond"), 1);
            assert!(!game.orbs.is_empty(), "the ore's experience");
        });
    }

    /// The direct call still refuses invalid players and absent blocks, and uses
    /// the game mode, main-hand item and game-master permissions.
    #[test]
    #[ignore = "daily"]
    fn the_players_checks_hold() {
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let player = with_player(game, ItemStack::new("minecraft:diamond_pickaxe", 1));
            let (stone, command, air) = ([6, 64, 4], [7, 64, 4], [8, 64, 4]);
            place(game, stone, "minecraft:stone");
            place(game, command, "minecraft:command_block");
            assert!(!game.player_destroy_block(player, air, false));
            assert!(!game.player_destroy_block(player, [16, 64, 4], false));
            let epoch = game.native_lifecycle.epoch().unwrap();
            let stranger =
                mod_api::__private::game::player_ref(mod_api::EntityRef::new(epoch, 21), 124);
            assert!(!game.player_destroy_block(stranger, stone, false));
            game.set_mode(GameMode::Adventure);
            assert!(!game.player_destroy_block(player, stone, false));
            game.set_mode(GameMode::Spectator);
            assert!(!game.player_destroy_block(player, stone, false));
            assert_eq!(game.held().unwrap().damage, 0, "nothing wore the pickaxe");
            game.set_mode(GameMode::Creative);
            assert!(!game.player_destroy_block(player, command, false));
            assert_eq!(block(game, command), "minecraft:command_block");
            game.common.inventory[game.common.selected] =
                Some(ItemStack::new("minecraft:diamond_sword", 1));
            assert!(!game.player_destroy_block(player, stone, false));
            assert_eq!(block(game, stone), "minecraft:stone");
            assert!(game.dropped.is_empty());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_direct_break_ignores_spawn_protection_range_and_mining_progress() {
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let player = with_player(game, ItemStack::new("minecraft:diamond_pickaxe", 1));
            let pos = [13, 64, 13];
            place(game, pos, "minecraft:bedrock");
            game.level.world_spawn = Some(pos);
            game.server.lan_acting_spawn_protection = Some(1);
            assert!(game.spawn_protects(1, game.dim.id(), pos));
            assert!(game.player_destroy_block(player, pos, false));
            assert_eq!(block(game, pos), "minecraft:air");
        });
    }

    /// A vein miner as the issue's contract builds one: the player's break of the vein's
    /// origin starts it, the mod breaks every block of the vein as the player (the origin
    /// too, then cancels the outer break), takes each break's drops and experience in
    /// `block_drops`, and sets them down at the origin merged within stack sizes.
    struct VeinMiner {
        vein: Vec<[i32; 3]>,
        busy: std::sync::atomic::AtomicBool,
        kept: std::sync::Mutex<(Vec<mod_api::ItemStack>, u32)>,
    }

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for VeinMiner {
        fn break_block(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::BreakBlockEvent<'_>,
        ) -> mod_api::Flow {
            use std::sync::atomic::Ordering;
            let origin = self.vein[0];
            if event.pos != origin || self.busy.swap(true, Ordering::Relaxed) {
                return mod_api::Flow::Continue;
            }
            let player = host.players().acting_player().expect("the breaker acts");
            for &pos in &self.vein {
                player.destroy_block(host, pos, false);
            }
            self.busy.store(false, Ordering::Relaxed);
            let (stacks, experience) = std::mem::take(&mut *self.kept.lock().unwrap());
            let mut merged: Vec<mod_api::ItemStack> = Vec::new();
            for stack in stacks {
                let max = host.max_stack_size(stack.item()).unwrap_or(64);
                let mut rest = stack.count();
                for into in merged.iter_mut().filter(|m| m.is_stackable_with(&stack)) {
                    let room = max.saturating_sub(into.count()).min(rest);
                    into.grow(room).unwrap();
                    rest -= room;
                }
                if rest > 0 {
                    let mut left = stack.clone();
                    left.set_count(rest).unwrap();
                    merged.push(left);
                }
            }
            let level = player.level(host).expect("the player's level");
            for stack in merged {
                level.pop_resource(host, origin, stack).unwrap();
            }
            let [x, y, z] = origin.map(|v| f64::from(v) + 0.5);
            level.award_experience(host, x, y, z, experience);
            mod_api::Flow::Cancel
        }
        fn block_drops(
            &self,
            _: &dyn mod_api::LevelHost,
            event: &mut mod_api::BlockDropsEvent<'_>,
        ) -> mod_api::Flow {
            let pos = [event.pos.x, event.pos.y, event.pos.z];
            if self.busy.load(std::sync::atomic::Ordering::Relaxed)
                && event.breaker == Some(123)
                && self.vein.contains(&pos)
            {
                let mut kept = self.kept.lock().unwrap();
                event.drops.retain(|drop| {
                    if let Some(stack) = drop.stack().and_then(|stack| stack.project()) {
                        kept.0.push(stack);
                        false
                    } else {
                        true
                    }
                });
                kept.1 += std::mem::take(&mut event.experience);
            }
            mod_api::Flow::Continue
        }
    }

    /// The issue's acceptance (#2953): a vein miner breaks N blocks as the player and gathers
    /// their drops, merged within stack sizes, and their experience at the origin, with no
    /// drop or orb anywhere else. The pickaxe wears once per block, as the player's own breaks.
    #[test]
    #[ignore = "daily"]
    fn a_vein_miner_sets_a_veins_drops_and_experience_down_at_the_origin() {
        // Five diamond ores in a row, then sixty-six stone: more cobblestone than one stack.
        let mut vein: Vec<[i32; 3]> = (0..5).map(|i| [2 + i, 64, 2]).collect();
        vein.extend((0..66).map(|i| [i % 11 + 2, 65 + i / 11, 2]));
        let ores = 5;
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "vein",
            Box::new(VeinMiner {
                vein: vein.clone(),
                busy: false.into(),
                kept: Default::default(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                with_player(game, ItemStack::new("minecraft:diamond_pickaxe", 1));
                for (i, &pos) in vein.iter().enumerate() {
                    let block = if i < ores {
                        "minecraft:diamond_ore"
                    } else {
                        "minecraft:stone"
                    };
                    place(game, pos, block);
                }
                let origin = vein[0];
                // The player's own break of the origin: the mod broke it, and the vein.
                assert!(!game.player_break_block(origin, None).unwrap());
                assert!(vein.iter().all(|&pos| block(game, pos) == "minecraft:air"));
                assert_eq!(game.held().unwrap().damage, vein.len() as u32);
                assert_eq!(game.stats.get("mined", "minecraft:stone"), 66);

                let near = |at: [f64; 3], by: f64| {
                    (0..3).all(|i| (at[i] - (f64::from(origin[i]) + 0.5)).abs() <= by)
                };
                let mut stacks: Vec<(String, u32)> = Vec::new();
                for item in game.dropped.iter() {
                    let at = [item.position.x, item.position.y, item.position.z];
                    assert!(near(at, 0.5), "every drop pops at the origin: {at:?}");
                    stacks.push((item.stack.item.to_string(), item.stack.count));
                }
                stacks.sort();
                assert_eq!(
                    stacks,
                    [
                        ("minecraft:cobblestone".to_owned(), 2),
                        ("minecraft:cobblestone".to_owned(), 64),
                        ("minecraft:diamond".to_owned(), ores as u32),
                    ]
                );
                // Each diamond ore gives 3 to 7 points, all of them awarded at the origin.
                let points: u32 = game.orbs.iter().map(|o| o.points()).sum();
                assert!(
                    (3 * ores as u32..=7 * ores as u32).contains(&points),
                    "{points}"
                );
                assert!(game.orbs.iter().all(|o| {
                    let p = o.position;
                    near([p.x, p.y, p.z], 0.01)
                }));
            });
        });
    }

    struct ChainedBreaks {
        last: i32,
        vetoed: [i32; 3],
    }

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for ChainedBreaks {
        fn break_block(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::BreakBlockEvent<'_>,
        ) -> mod_api::Flow {
            if event.pos == self.vetoed {
                return mod_api::Flow::Cancel;
            }
            let [x, y, z] = event.pos;
            let (next, collect) = if x < self.last {
                ([x + 1, y, z], false)
            } else {
                (self.vetoed, true)
            };
            let player = host.players().acting_player().expect("the breaker acts");
            assert_eq!(player.destroy_block(host, next, collect), !collect);
            mod_api::Flow::Continue
        }
    }

    #[test]
    #[ignore = "daily"]
    fn chained_breaks_keep_their_own_drops_without_a_depth_limit() {
        let (first, last, vetoed) = (2, 11, [12, 64, 4]);
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "chain",
            Box::new(ChainedBreaks { last, vetoed }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                let player = with_player(game, ItemStack::new("minecraft:diamond_pickaxe", 1));
                for x in first..=last {
                    place(game, [x, 64, 4], "minecraft:stone");
                }
                place(game, vetoed, "minecraft:stone");
                assert!(game.player_destroy_block(player, [first, 64, 4], true));
                assert!((first..=last).all(|x| block(game, [x, 64, 4]) == "minecraft:air"));
                assert_eq!(block(game, vetoed), "minecraft:stone");
                assert_eq!(game.held().unwrap().damage, 10);
                assert_eq!(carried(game, "minecraft:cobblestone"), 1);
                assert_eq!(
                    game.dropped
                        .iter()
                        .filter(|item| &*item.stack.item == "minecraft:cobblestone")
                        .map(|item| item.stack.count)
                        .sum::<u32>(),
                    9
                );
            });
        });
    }

    struct Reenter(AtomicUsize);

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Reenter {
        fn break_block(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::BreakBlockEvent<'_>,
        ) -> mod_api::Flow {
            self.0.fetch_add(1, Ordering::Relaxed);
            // No guard: the break of the subject this callback is outstanding on.
            let player = host.players().acting_player().expect("the breaker acts");
            player.destroy_block(host, event.pos, false);
            mod_api::Flow::Continue
        }
    }

    /// A listener that breaks the block it is told about, with no guard, would call itself without
    /// end. The nested break dispatches nothing again, removes the block, and the outer break then
    /// does nothing more: the callback ran once and the tool wore once. A fresh process, as an
    /// unbounded reentry would end it.
    #[test]
    #[ignore = "daily"]
    fn a_listener_breaking_its_own_subject_is_not_called_again() {
        if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::server_side::native_player_break::tests::a_listener_breaking_its_own_subject_is_not_called_again",
            "SUNRISE_REENTRANT_BREAK_CHILD",
            &["reentry"],
        ) {
            return;
        }
        let calls = Arc::new(Reenter(AtomicUsize::new(0)));
        struct Shared(Arc<Reenter>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Shared {
            fn break_block(
                &self,
                host: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::BreakBlockEvent<'_>,
            ) -> mod_api::Flow {
                mod_api::Hooks::break_block(&*self.0, host, event)
            }
        }
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "reenter",
            Box::new(Shared(calls.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                let player = with_player(game, ItemStack::new("minecraft:diamond_pickaxe", 1));
                let pos = [6, 64, 4];
                place(game, pos, "minecraft:stone");
                assert!(!game.player_destroy_block(player, pos, false));
                assert_eq!(block(game, pos), "minecraft:air");
                assert_eq!(game.held().unwrap().damage, 1);
            });
        });
        assert_eq!(calls.0.load(Ordering::Relaxed), 1);
    }

    struct Counts(Arc<AtomicUsize>);

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Counts {
        fn break_block(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::BreakBlockEvent<'_>,
        ) -> mod_api::Flow {
            assert_eq!(event.pos, [8, 64, 4]);
            self.0.fetch_add(1, Ordering::Relaxed);
            mod_api::Flow::Continue
        }
    }

    /// `ServerPlayerGameMode.destroyBlock` has no air filter: `break_block` fires for air, and the
    /// break removes nothing.
    #[test]
    #[ignore = "daily"]
    fn a_break_of_air_still_dispatches_break_block() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "counts",
            Box::new(Counts(calls.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                let player = with_player(game, ItemStack::new("minecraft:diamond_pickaxe", 1));
                assert!(!game.player_destroy_block(player, [8, 64, 4], false));
                assert_eq!(game.held().unwrap().damage, 0);
            });
        });
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    struct RefuseHarvest;

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for RefuseHarvest {
        fn player_harvest_check(
            &self,
            _: mod_api::HostSide<'_>,
            event: &mut mod_api::PlayerHarvestCheckEvent,
        ) {
            event.can_harvest = false;
        }
    }

    /// The tool wears as `ItemStack.mineBlock` before the removal, whatever the harvest answer: a
    /// mod's refusal costs the drops and the statistic, not the wear.
    #[test]
    #[ignore = "daily"]
    fn a_refused_harvest_still_wears_the_tool_and_drops_nothing() {
        let mods: mod_api::Mods =
            vec![mod_api::ModEntry::new("refuse", Box::new(RefuseHarvest))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                let player = with_player(game, ItemStack::new("minecraft:diamond_pickaxe", 1));
                let pos = [6, 64, 4];
                place(game, pos, "minecraft:stone");
                assert!(game.player_destroy_block(player, pos, false));
                assert_eq!(block(game, pos), "minecraft:air");
                assert_eq!(game.held().unwrap().damage, 1);
                assert!(!popped(game, "minecraft:cobblestone"));
                assert_eq!(game.stats.get("mined", "minecraft:stone"), 0);
            });
        });
    }

    struct FinishFirst(AtomicBool);

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for FinishFirst {
        fn break_block(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::BreakBlockEvent<'_>,
        ) -> mod_api::Flow {
            if !self.0.swap(true, Ordering::Relaxed) {
                let player = host.players().acting_player().expect("the breaker acts");
                assert!(player.destroy_block(host, event.pos, false));
            }
            mod_api::Flow::Continue
        }
    }

    #[test]
    #[ignore = "daily"]
    fn a_break_does_not_disarm_tripwire_already_removed_by_its_listener() {
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "finish-first",
            Box::new(FinishFirst(false.into())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                let player = with_player(game, ItemStack::new("minecraft:shears", 1));
                let pos = [6, 64, 4];
                place(game, pos, "minecraft:tripwire");
                // Disarming the original state in the outer break would write the wire back.
                assert!(!game.player_destroy_block(player, pos, false));
                assert_eq!(block(game, pos), "minecraft:air");
                assert_eq!(game.held().unwrap().damage, 1);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_nested_turtle_egg_break_does_not_repeat_removal_effects() {
        let done = Arc::new(AtomicBool::new(false));
        struct FinishOnce(Arc<AtomicBool>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for FinishOnce {
            fn break_block(
                &self,
                host: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::BreakBlockEvent<'_>,
            ) -> mod_api::Flow {
                if !self.0.swap(true, Ordering::Relaxed) {
                    let player = host.players().acting_player().expect("the breaker acts");
                    assert!(player.destroy_block(host, event.pos, false));
                }
                mod_api::Flow::Continue
            }
        }
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "finish-once",
            Box::new(FinishOnce(done.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                let player = with_player(game, ItemStack::new("minecraft:diamond_pickaxe", 1));
                let registry = game.pack.assets.registry;
                let egg = |eggs: u32| {
                    let base = registry.default_state("minecraft:turtle_egg").unwrap();
                    let mut state = registry.state(base).unwrap().clone();
                    state.properties.insert("eggs".into(), eggs.to_string());
                    registry.resolve(&state).unwrap()
                };
                let nest = [6, 64, 4];
                game.world.set(nest[0], nest[1], nest[2], egg(3)).unwrap();
                assert!(!game.player_destroy_block(player, nest, false));
                assert_eq!(game.world.block(nest[0], nest[1], nest[2]), egg(2));
                assert_eq!(game.held().unwrap().damage, 1);
                assert_eq!(game.stats.get("mined", "minecraft:turtle_egg"), 1);

                // A nested survival break of ice over stone leaves water, not air;
                // the outer break must still skip removal, wear, and statistics.
                done.store(false, Ordering::Relaxed);
                let ice = [7, 64, 4];
                place(game, [7, 63, 4], "minecraft:stone");
                place(game, ice, "minecraft:ice");
                assert!(!game.player_destroy_block(player, ice, false));
                assert_eq!(block(game, ice), "minecraft:water");
                assert_eq!(game.held().unwrap().damage, 2);
                assert_eq!(game.stats.get("mined", "minecraft:ice"), 1);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn restricted_break_attempts_fire_pre_canceled_and_keep_hard_restrictions() {
        let normal_seen = Arc::new(AtomicUsize::new(0));
        let canceled_seen: Arc<Mutex<Vec<([i32; 3], bool)>>> = Arc::new(Mutex::new(Vec::new()));
        let clear_cancel = Arc::new(AtomicBool::new(true));

        struct NormalObserver(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for NormalObserver {
            fn break_block(
                &self,
                _: &mut dyn mod_api::ServerHost,
                _: &mut mod_api::BreakBlockEvent<'_>,
            ) -> mod_api::Flow {
                self.0.fetch_add(1, Ordering::Relaxed);
                mod_api::Flow::Continue
            }
        }

        struct CancelledObserver {
            seen: Arc<Mutex<Vec<([i32; 3], bool)>>>,
            clear_cancel: Arc<AtomicBool>,
        }
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for CancelledObserver {
            #[listener(receive_cancelled)]
            fn break_block(
                &self,
                _: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::BreakBlockEvent<'_>,
            ) -> mod_api::Flow {
                self.seen.lock().unwrap().push((event.pos, event.cancelled));
                if self.clear_cancel.load(Ordering::Relaxed) {
                    event.cancelled = false;
                }
                mod_api::Flow::Continue
            }
        }

        let mods: mod_api::Mods = vec![
            mod_api::ModEntry::new("normal", Box::new(NormalObserver(normal_seen.clone()))),
            mod_api::ModEntry::new(
                "cancelled",
                Box::new(CancelledObserver {
                    seen: canceled_seen.clone(),
                    clear_cancel: clear_cancel.clone(),
                }),
            ),
        ]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                let player = with_player(game, ItemStack::new("minecraft:diamond_pickaxe", 1));
                let (stone, command, air) = ([6, 64, 4], [7, 64, 4], [8, 64, 4]);
                place(game, stone, "minecraft:stone");
                place(game, command, "minecraft:command_block");

                // Loaded air posts break_block uncanceled (ServerPlayerGameMode.destroyBlock has
                // no air filter) and returns false; an unloaded column with no generator refuses
                // without posting.
                assert!(!game.player_destroy_block(player, air, false));
                assert!(!game.player_destroy_block(player, [16, 64, 4], false));
                assert_eq!(normal_seen.swap(0, Ordering::Relaxed), 1);
                assert_eq!(
                    std::mem::take(&mut *canceled_seen.lock().unwrap()),
                    vec![(air, false)]
                );

                // Adventure, spectator, and unauthorized game-master attempts post
                // pre-canceled; clearing the cancel still leaves the block intact.
                game.set_mode(GameMode::Adventure);
                assert!(!game.player_destroy_block(player, stone, false));
                assert_eq!(block(game, stone), "minecraft:stone");

                game.set_mode(GameMode::Spectator);
                assert!(!game.player_destroy_block(player, stone, false));
                assert_eq!(block(game, stone), "minecraft:stone");

                game.set_mode(GameMode::Creative);
                assert!(!game.player_destroy_block(player, command, false));
                assert_eq!(block(game, command), "minecraft:command_block");

                // Creative sword: pre-canceled by the item check; staying canceled
                // keeps the block, while un-canceling lets the break proceed as in
                // NeoForge BreakBlockEvent.
                game.common.inventory[game.common.selected] =
                    Some(ItemStack::new("minecraft:diamond_sword", 1));
                clear_cancel.store(false, Ordering::Relaxed);
                assert!(!game.player_destroy_block(player, stone, false));
                assert_eq!(block(game, stone), "minecraft:stone");

                clear_cancel.store(true, Ordering::Relaxed);
                assert!(game.player_destroy_block(player, stone, false));
                assert_eq!(block(game, stone), "minecraft:air");

                assert_eq!(normal_seen.load(Ordering::Relaxed), 0);
                assert_eq!(
                    *canceled_seen.lock().unwrap(),
                    vec![
                        (stone, true),
                        (stone, true),
                        (command, true),
                        (stone, true),
                        (stone, true),
                    ]
                );
            });
        });
    }

    /// Cancels the break at [`GUARDED`] and the placement of glass, and records what each
    /// placement was told: its position, its placer and whether it went against stone.
    struct Warden;

    /// What `Warden` heard of each placement.
    static WARDEN_HEARD: std::sync::Mutex<
        Vec<(mod_api::BlockPos, Option<mod_api::EntityRef>, bool)>,
    > = std::sync::Mutex::new(Vec::new());

    /// The block `Warden` keeps.
    const GUARDED: [i32; 3] = [7, 64, 4];

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Warden {
        fn break_block(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::BreakBlockEvent<'_>,
        ) -> mod_api::Flow {
            if event.pos == GUARDED {
                mod_api::Flow::Cancel
            } else {
                mod_api::Flow::Continue
            }
        }
        fn entity_place(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityPlaceEvent,
        ) -> mod_api::Flow {
            let stone = host.block_state_id("minecraft:stone");
            let on_stone = stone == Some(event.placed_against);
            WARDEN_HEARD
                .lock()
                .unwrap()
                .push((event.pos, event.placer, on_stone));
            if host.block_state_id("minecraft:glass") == Some(event.state) {
                mod_api::Flow::Cancel
            } else {
                mod_api::Flow::Continue
            }
        }
    }

    /// The veto asks answer as the player's own break and placement would be decided, and
    /// change nothing: a listener's cancel and the player's checks say true, an allowed break
    /// leaves the block and the pickaxe as they were, an allowed placement writes nothing, and
    /// an unloaded block or someone who isn't playing has no answer.
    #[test]
    #[ignore = "daily"]
    fn the_veto_asks_answer_without_changing_the_level() {
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("warden", Box::new(Warden))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                let player = with_player(game, ItemStack::new("minecraft:diamond_pickaxe", 1));
                let stone = [6, 64, 4];
                place(game, stone, "minecraft:stone");
                place(game, GUARDED, "minecraft:stone");
                assert_eq!(game.player_break_canceled(player, stone), Some(false));
                assert_eq!(game.player_break_canceled(player, GUARDED), Some(true));
                assert_eq!(block(game, stone), "minecraft:stone");
                assert_eq!(game.held().unwrap().damage, 0, "nothing wore the pickaxe");
                assert_eq!(game.player_break_canceled(player, [100, 64, 100]), None);
                let epoch = game.native_lifecycle.epoch().unwrap();
                let stranger =
                    mod_api::__private::game::player_ref(mod_api::EntityRef::new(epoch, 21), 124);
                assert_eq!(game.player_break_canceled(stranger, stone), None);

                let registry = game.pack.assets.registry;
                let glass = registry.default_state("minecraft:glass").unwrap();
                let dirt = registry.default_state("minecraft:dirt").unwrap();
                let above = [6, 65, 4];
                let up = mod_api::BlockFace::Up;
                assert_eq!(
                    game.player_place_canceled(player, above, up, glass),
                    Some(true)
                );
                assert_eq!(
                    game.player_place_canceled(player, above, up, dirt),
                    Some(false)
                );
                assert_eq!(block(game, above), "minecraft:air");
                let heard = WARDEN_HEARD.lock().unwrap().clone();
                let at = mod_api::BlockPos::new(6, 65, 4);
                assert_eq!(heard, [(at, Some(player.entity()), true); 2]);
                assert_eq!(game.player_place_canceled(stranger, above, up, dirt), None);

                game.set_mode(GameMode::Adventure);
                assert_eq!(game.player_break_canceled(player, stone), Some(true));
                assert_eq!(block(game, stone), "minecraft:stone");
            });
        });
    }
}
