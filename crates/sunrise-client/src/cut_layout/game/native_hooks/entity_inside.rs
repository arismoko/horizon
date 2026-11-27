#![cfg(test)]
use sunrise_core::position::BlockPos;
#[cfg(test)]
pub(crate) mod entity_inside_tests {
    use crate::game::native_hooks::entity_inside::*;

    /// Landing writes reach the level at the batch end, in the order asked, and only
    /// where the block is still the state landed on.
    #[test]
    #[ignore = "daily"]
    fn landing_writes_apply_at_the_batch_end() {
        if crate::game::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::native_hooks::entity_inside::entity_inside_tests::landing_writes_apply_at_the_batch_end",
            "SUNRISE_INSIDE_LANDING_CHILD",
            &["landing"],
        ) {
            return;
        }
        use sunrise_sim::block_contact::{landed, take_pending};
        use sunrise_world::level::LandingWrite;
        use {
            sunrise_gameplay_common::game::api_values::TryIntoApi,
            sunrise_gameplay_common::game::api_values::TryIntoEngine,
        };
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let state = |name| game.pack.assets.registry.default_state(name).unwrap();
            let (farmland, dirt, stone, air) = (
                state("minecraft:farmland"),
                state("minecraft:dirt"),
                state("minecraft:stone"),
                state("minecraft:air"),
            );
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            game.world.set(1, 64, 1, farmland).unwrap();
            game.world.set(2, 64, 1, farmland).unwrap();
            game.world.set(3, 64, 1, stone).unwrap();
            game.world.set(4, 64, 1, dirt).unwrap();
            game.world.set(5, 64, 1, dirt).unwrap();
            let drops_before = game.dropped.len();
            let at = |x| BlockPos::new(x, 64, 1);
            let requested = vec![
                mod_api::LandingWrite::SetBlock(stone.try_into_api().unwrap()),
                mod_api::LandingWrite::SetBlock(dirt.try_into_api().unwrap()),
            ];
            // Exercise the same complete-batch boundary as generated dispatch before
            // entering the engine queue, then prove its ordered writes reach the world.
            let staged = requested.try_into_engine(&*game).unwrap();
            landed(farmland, at(1), staged);
            landed(dirt, at(4), vec![LandingWrite::DestroyBlock { drop: true }]);
            landed(
                dirt,
                at(5),
                vec![LandingWrite::DestroyBlock { drop: false }],
            );
            // The block changed after the landing: dropped.
            landed(stone, at(2), vec![LandingWrite::SetBlock(dirt)]);
            landed(
                stone,
                at(3),
                vec![LandingWrite::DestroyBlock { drop: false }],
            );
            // Nothing asked: nothing kept.
            landed(farmland, at(1), Vec::new());
            assert_eq!(
                game.world.block(1, 64, 1),
                farmland,
                "nothing writes mid-batch"
            );
            game.world.block_log = Some(Vec::new());
            game.drain_entity_inside();
            assert_eq!(game.world.block(1, 64, 1), dirt);
            assert_eq!(game.world.block(2, 64, 1), farmland);
            assert_eq!(game.world.block(3, 64, 1), air, "the stone is destroyed");
            assert_eq!(game.world.block(4, 64, 1), air);
            assert_eq!(game.world.block(5, 64, 1), air);
            assert_eq!(
                game.dropped.len(),
                drops_before + 1,
                "only drop=true creates dirt resources"
            );
            let dropped = &game.dropped[drops_before];
            assert_eq!(dropped.stack.item.as_ref(), "minecraft:dirt");
            assert_eq!(
                dropped.position.x.floor() as i32,
                4,
                "resources came from drop=true, not the neighboring drop=false block"
            );
            // Both writes reach the clients (`setBlockAndUpdate` and `destroyBlock` are
            // flag 3); the dropped one sends nothing.
            let log = game.world.block_log.take().unwrap();
            assert!(log.contains(&(1, 64, 1)), "the set block is sent: {log:?}");
            assert!(
                log.contains(&(3, 64, 1)),
                "the destroyed block is sent: {log:?}"
            );
            assert!(
                !log.contains(&(2, 64, 1)),
                "the dropped write sends nothing"
            );
            assert!(take_pending().is_empty());
        });
    }
}

#[cfg(test)]
pub(crate) mod sweep_tests {
    fn fresh(name: &str) -> bool {
        let module = module_path!().split_once("::").unwrap().1;
        sunrise_gameplay_common::game::test_fixtures::run_fresh_cases(
            &format!("{module}::{name}"),
            "SUNRISE_INSIDE_SWEEP_CHILD",
            &["sweep"],
        )
    }

    use sunrise_gameplay_common::game::native_hooks::BLOCK_CONTACT_HOOKS;

    use sunrise_gameplay_common::game::native_hooks::entity_inside::sweep_tests::InsidePart;

    use sunrise_gameplay_common::game::native_hooks::entity_inside::sweep_tests::arrow_shot;

    use sunrise_gameplay_common::game::native_hooks::entity_inside::sweep_tests::install_inside_mod;

    use sunrise_gameplay::game::native_hooks::entity_inside::sweep_tests::item_at_blocks;

    use sunrise_gameplay::game::native_hooks::entity_inside::sweep_tests::sweep_game_player;

    use sunrise_gameplay::game::native_hooks::entity_inside::sweep_tests::sweep_game_player_live;

    /// A listening part is told mid-walk, in walk order, and its typed writes reach
    /// the player in this tick: both pushes add, and the wither effect event lands
    /// for the server's `give_player_effect`, which runs before this tick's effect
    /// tick.
    #[test]
    #[ignore = "daily"]
    fn a_part_pushes_the_player_and_its_effect_lands() {
        if fresh("a_part_pushes_the_player_and_its_effect_lands") {
            return;
        }
        let part = InsidePart::default();
        let mods = install_inside_mod("inside-push", &part);
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let _ = sunrise_sim::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
                let (before, after, dead) = sweep_game_player(game);
                assert!(!dead);
                assert_eq!(
                    *part.contacts.lock().unwrap(),
                    vec![[1, 100, 0], [2, 100, 0]],
                    "both contacts, in walk order"
                );
                assert!(
                    (after - before - 1.0).abs() < 1e-9,
                    "both pushes reached the player: {after}"
                );
                let effect = game
                    .potions
                    .effects
                    .get("wither")
                    .expect("the wither effect applied synchronously to the player");
                assert_eq!(effect.duration, 40);
                assert!(sunrise_sim::block_contact::take_pending().is_empty());
            });
        });
    }
    /// A part that kills the player with its first write: the walk's later contacts
    /// still happen, but the flush stops at the death, so the later writes and
    /// effects never apply.
    #[test]
    #[ignore = "daily"]
    fn a_killing_write_stops_the_flush() {
        if fresh("a_killing_write_stops_the_flush") {
            return;
        }
        let part = InsidePart {
            hurt_after: Some((0, 1000.0)),
            ..InsidePart::default()
        };
        let mods = install_inside_mod("inside-kill", &part);
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let _ = sunrise_sim::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
                let (before, after, dead) = sweep_game_player(game);
                assert_eq!(
                    part.contacts.lock().unwrap().len(),
                    2,
                    "the walk itself reaches both contacts"
                );
                assert!(dead, "the killing write applied");
                assert_eq!(
                    after.to_bits(),
                    before.to_bits(),
                    "the writes after the death never apply"
                );
            });
        });
    }
    /// A part that panics on its first contact is disabled for this hook. The walk
    /// and the player's tick go on without applying the failed callback's writes.
    #[test]
    #[ignore = "daily"]
    fn a_panicking_part_leaves_the_players_tick_restored() {
        if fresh("a_panicking_part_leaves_the_players_tick_restored") {
            return;
        }
        let part = InsidePart {
            panic_after: Some(0),
            ..InsidePart::default()
        };
        let mods = install_inside_mod("inside-panic", &part);
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let _ = sunrise_sim::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
                let (before, after, dead) = sweep_game_player(game);
                assert!(!dead);
                assert_eq!(
                    part.contacts.lock().unwrap().len(),
                    1,
                    "contact #1 panicked and disabled this mod's entity_inside callback for the session"
                );
                assert_eq!(
                    mod_api::__private::take_hook_panics(),
                    [mod_api::__private::HookPanicReport {
                        mod_id: "inside-panic",
                        hook: "entity_inside",
                    }]
                );
                assert_eq!(after, before, "the panicking part never pushed");
                assert!(
                    game.player.as_ref().unwrap().fire_ticks <= 0,
                    "the failed part's staged lava ignition never applied"
                );
                assert!(
                    game.player
                        .as_ref()
                        .unwrap()
                        .events
                        .iter()
                        .any(|event| matches!(
                            event,
                            sunrise_sim::motion::MotionEvent::EnterBlock { .. }
                        )),
                    "the vanilla moments still ran"
                );
            });
        });
    }
    /// Without a listening part the walk runs its vanilla moments and leaves the
    /// player's writes and the pending queue alone: no part is installed, so the
    /// fire's gate answers false and its DamageLevel is never built.
    #[test]
    #[ignore = "daily"]
    fn without_a_part_the_walk_costs_no_mod_work() {
        if fresh("without_a_part_the_walk_costs_no_mod_work") {
            return;
        }
        // The production binding, as every sweep test sets it: nothing is installed
        // on this thread, so its gate answers false for every state and the fire's
        // DamageLevel is never built. (A dummy binding here would race the
        // process-wide `block_contact::HOOKS` OnceLock — first setter wins —
        // against the other tests' real one.)
        let _ = sunrise_sim::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let (before, after, dead) = sweep_game_player(game);
            assert!(!dead);
            assert!((after - before).abs() < 1e-9, "no writes without a part");
            assert!(
                game.player
                    .as_ref()
                    .unwrap()
                    .events
                    .iter()
                    .any(|event| matches!(
                        event,
                        sunrise_sim::motion::MotionEvent::EnterBlock { .. }
                    )),
                "the vanilla moments still ran"
            );
            assert!(sunrise_sim::block_contact::take_pending().is_empty());
        });
    }
    /// A part's `AddEffect` keeps its collector position across the event channel:
    /// one write asked after FREEZE and one after EXTINGUISH land in that order,
    /// `applyAndClear`'s own, ready for the server's `give_player_effect` before
    /// this tick's effect tick.
    #[test]
    #[ignore = "daily"]
    fn an_added_effect_keeps_its_collector_position() {
        if fresh("an_added_effect_keeps_its_collector_position") {
            return;
        }
        let part = InsidePart::default();
        let mods = install_inside_mod("inside-order", &part);
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let _ = sunrise_sim::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
                let (_, _, dead) = sweep_game_player(game);
                assert!(!dead);
                assert_eq!(
                    *part.applied_order.lock().unwrap(),
                    vec![("minecraft:wither", 40), ("minecraft:speed", 20),],
                    "each contact's callbacks flush in InsideEffect order: the FREEZE slot before the EXTINGUISH slot (second identical add_effect returns false)"
                );
                assert_eq!(game.potions.effects.get("wither").unwrap().duration, 40);
                assert_eq!(game.potions.effects.get("speed").unwrap().duration, 20);
            });
        });
    }
    /// A server's player is told once a tick: the early `ServerStuck` pass keeps
    /// the moves for `server_block_damage`, and only the damage pass tells the
    /// part — two blocks, two contacts, not four, as vanilla's `entityInside`
    /// runs once a move.
    #[test]
    #[ignore = "daily"]
    fn a_servers_player_is_told_once_a_tick() {
        if fresh("a_servers_player_is_told_once_a_tick") {
            return;
        }
        let part = InsidePart::default();
        let mods = install_inside_mod("inside-once", &part);
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                let _ = sunrise_sim::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                for x in [1, 2] {
                    game.world.set(x, 100, 0, stone).unwrap();
                }
                if game.player.is_none() {
                    game.player = Some(
                        sunrise_sim::motion::PlayerMotion::new(
                            game.motion,
                            sunrise_core::geometry::Vec3::new(0.5, 100.0, 0.5),
                        )
                        .unwrap(),
                    );
                }
                game.player.as_mut().unwrap().history.add(
                    sunrise_world::entity_move::MovementRecord {
                        from: sunrise_core::geometry::Vec3::new(0.5, 100.0, 0.5),
                        to: sunrise_core::geometry::Vec3::new(2.5, 100.0, 0.5),
                        original: None,
                    },
                );
                sweep_game_player_live(
                    game,
                    sunrise_core::geometry::Vec3::new(0.5, 100.0, 0.5),
                    true,
                );
                assert_eq!(
                    *part.contacts.lock().unwrap(),
                    vec![[1, 100, 0], [2, 100, 0]],
                    "one contact a block, the damage pass's, not the stuck pass's too"
                );
            });
        });
    }
    /// Without a part the two server passes leave the player as vanilla has it:
    /// the walk still reports its blocks and nothing is queued.
    #[test]
    #[ignore = "daily"]
    fn the_server_passes_without_a_part_keep_vanilla_behavior() {
        if fresh("the_server_passes_without_a_part_keep_vanilla_behavior") {
            return;
        }
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            for x in [1, 2] {
                game.world.set(x, 100, 0, stone).unwrap();
            }
            if game.player.is_none() {
                game.player = Some(
                    sunrise_sim::motion::PlayerMotion::new(
                        game.motion,
                        sunrise_core::geometry::Vec3::new(0.5, 100.0, 0.5),
                    )
                    .unwrap(),
                );
            }
            game.player
                .as_mut()
                .unwrap()
                .history
                .add(sunrise_world::entity_move::MovementRecord {
                    from: sunrise_core::geometry::Vec3::new(0.5, 100.0, 0.5),
                    to: sunrise_core::geometry::Vec3::new(2.5, 100.0, 0.5),
                    original: None,
                });
            let _ = sunrise_sim::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
            let velocity = game.player.as_ref().unwrap().velocity.x;
            sweep_game_player_live(
                game,
                sunrise_core::geometry::Vec3::new(0.5, 100.0, 0.5),
                true,
            );
            assert_eq!(
                game.player.as_ref().unwrap().velocity.x,
                velocity,
                "no writes without a part"
            );
            let entered = game
                .player
                .as_ref()
                .unwrap()
                .events
                .iter()
                .filter(|event| {
                    matches!(event, sunrise_sim::motion::MotionEvent::EnterBlock { .. })
                })
                .count();
            assert_eq!(entered, 2, "each block still reported, vanilla's moments");
            assert!(sunrise_sim::block_contact::take_pending().is_empty());
        });
    }
    /// A part's pushes reach the item in its own tick, one contact a block.
    #[test]
    #[ignore = "daily"]
    fn a_part_pushes_an_item_once_a_block() {
        if fresh("a_part_pushes_an_item_once_a_block") {
            return;
        }
        let part = InsidePart::default();
        let mods = install_inside_mod("inside-item", &part);
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let _ = sunrise_sim::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
                let mut item = item_at_blocks(game);
                let before = item.velocity.x;
                sunrise_sim::block_contact::in_level(
                    sunrise_sim::parallel::TickThread::mint(),
                    Some("minecraft:overworld"),
                    || {
                        game.sweep_item_inside(&mut item, None);
                    },
                );
                assert_eq!(
                    *part.contacts.lock().unwrap(),
                    vec![[1, 100, 0], [2, 100, 0]],
                    "one contact a block, in walk order"
                );
                assert!(
                    (item.velocity.x - before - 1.0).abs() < 1e-9,
                    "both pushes reached the item"
                );
                assert!(sunrise_sim::block_contact::take_pending().is_empty());
            });
        });
    }
    /// A panicking part inside the item batch: the staged fire contains it, the
    /// lent level goes back, and the lifted row the batch holds returns to the
    /// level's list (#2194's and #2213's carrier discipline).
    #[test]
    #[ignore = "daily"]
    fn a_panicking_part_leaves_the_item_batch_restored() {
        if fresh("a_panicking_part_leaves_the_item_batch_restored") {
            return;
        }
        let part = InsidePart {
            panic_after: Some(0),
            ..InsidePart::default()
        };
        let mods = install_inside_mod("inside-item-panic", &part);
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let _ = sunrise_sim::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
                let item = item_at_blocks(game);
                game.dropped.push(item);
                let rows = game.dropped.len();
                let mut items = game.dropped.lift();
                let before = items[0].velocity.x;
                sunrise_sim::block_contact::in_level(
                    sunrise_sim::parallel::TickThread::mint(),
                    Some("minecraft:overworld"),
                    || {
                        game.sweep_item_inside(&mut items[0], None);
                    },
                );
                game.put_back_dropped(items);
                assert_eq!(
                    part.contacts.lock().unwrap().len(),
                    1,
                    "contact #1 panicked and disabled this mod's entity_inside callback for the session"
                );
                assert_eq!(
                    mod_api::__private::take_hook_panics(),
                    [mod_api::__private::HookPanicReport {
                        mod_id: "inside-item-panic",
                        hook: "entity_inside",
                    }]
                );
                assert_eq!(game.dropped.len(), rows, "the lifted row returned");
                let item = game.dropped.first().unwrap();
                assert_eq!(
                    item.velocity.x, before,
                    "the failed part's staged writes never applied"
                );
                // The level went back: its blocks read as they were.
                assert!(
                    game.world.block(1, 100, 0).0 != 0,
                    "the level reads as it was"
                );
                assert!(sunrise_sim::block_contact::take_pending().is_empty());
            });
        });
    }
    /// Without a part the item's sweep costs one gate test: nothing is written,
    /// nothing queued, exactly what main's recorded route did with no listener.
    #[test]
    #[ignore = "daily"]
    fn an_item_without_a_part_is_left_alone() {
        if fresh("an_item_without_a_part_is_left_alone") {
            return;
        }
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let _ = sunrise_sim::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
            let mut item = item_at_blocks(game);
            let before = (item.velocity.x, item.health, item.fire_ticks);
            sunrise_sim::block_contact::in_level(
                sunrise_sim::parallel::TickThread::mint(),
                Some("minecraft:overworld"),
                || {
                    game.sweep_item_inside(&mut item, None);
                },
            );
            assert_eq!(
                (item.velocity.x, item.health, item.fire_ticks),
                before,
                "no writes without a part"
            );
            assert!(sunrise_sim::block_contact::take_pending().is_empty());
        });
    }
    /// The projectile's batch sweep: a part's push reaches it, one contact a
    /// block, and a removed projectile's flush stops.
    #[test]
    #[ignore = "daily"]
    fn a_part_pushes_a_projectile_and_removal_stops_the_flush() {
        if fresh("a_part_pushes_a_projectile_and_removal_stops_the_flush") {
            return;
        }
        let part = InsidePart::default();
        let mods = install_inside_mod("inside-shot", &part);
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let _ = sunrise_sim::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                for x in [1, 2] {
                    game.world.set(x, 100, 0, stone).unwrap();
                }
                let mut shot = sunrise_sim::projectiles::Projectile::new(
                    sunrise_sim::projectiles::ProjectileKind::Snowball,
                    sunrise_core::geometry::Vec3::new(2.5, 100.0, 0.5),
                    None,
                );
                shot.id = 9;
                shot.previous = sunrise_core::geometry::Vec3::new(0.5, 100.0, 0.5);
                let before = shot.velocity.x;
                sunrise_sim::block_contact::in_level(
                    sunrise_sim::parallel::TickThread::mint(),
                    Some("minecraft:overworld"),
                    || {
                        game.sweep_projectile_inside(&mut shot, None);
                    },
                );
                assert_eq!(
                    *part.contacts.lock().unwrap(),
                    vec![[1, 100, 0], [2, 100, 0]],
                    "one contact a block, in walk order"
                );
                assert!(
                    (shot.velocity.x - before - 1.0).abs() < 1e-9,
                    "both pushes reached the projectile"
                );
                // A removed arrow's flush stops: nothing applies past its death.
                part.contacts.lock().unwrap().clear();
                let mut arrow = sunrise_sim::projectiles::Projectile::new(
                    sunrise_sim::projectiles::ProjectileKind::Arrow {
                        base_damage: 1.0,
                        critical: false,
                        effect: None,
                    },
                    sunrise_core::geometry::Vec3::new(2.5, 100.0, 0.5),
                    None,
                );
                arrow.id = 10;
                arrow.previous = sunrise_core::geometry::Vec3::new(0.5, 100.0, 0.5);
                arrow.velocity = sunrise_core::geometry::Vec3::ZERO;
                let arrow_state = &mut arrow.arrow;
                if let Some(a) = arrow_state.as_mut() {
                    a.removed = true;
                }
                let before = arrow.velocity.x;
                sunrise_sim::block_contact::in_level(
                    sunrise_sim::parallel::TickThread::mint(),
                    Some("minecraft:overworld"),
                    || {
                        game.sweep_projectile_inside(&mut arrow, None);
                    },
                );
                assert!(
                    part.contacts.lock().unwrap().is_empty(),
                    "a removed projectile is not walked"
                );
                assert!((arrow.velocity.x - before).abs() < 1e-9);
            });
        });
    }
    /// Without a part the projectile's sweep costs one gate test and nothing else,
    /// exactly what main's recorded route did with no listener.
    #[test]
    #[ignore = "daily"]
    fn a_projectile_without_a_part_is_left_alone() {
        if fresh("a_projectile_without_a_part_is_left_alone") {
            return;
        }
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let _ = sunrise_sim::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            game.world.set(1, 100, 0, stone).unwrap();
            let mut shot = sunrise_sim::projectiles::Projectile::new(
                sunrise_sim::projectiles::ProjectileKind::LlamaSpit,
                sunrise_core::geometry::Vec3::new(1.5, 100.0, 0.5),
                None,
            );
            shot.previous = sunrise_core::geometry::Vec3::new(0.5, 100.0, 0.5);
            let before = (shot.velocity.x, shot.fire_ticks, shot.fall_distance);
            sunrise_sim::block_contact::in_level(
                sunrise_sim::parallel::TickThread::mint(),
                Some("minecraft:overworld"),
                || {
                    game.sweep_projectile_inside(&mut shot, None);
                },
            );
            assert_eq!(
                (shot.velocity.x, shot.fire_ticks, shot.fall_distance),
                before,
                "no writes without a part"
            );
            assert!(sunrise_sim::block_contact::take_pending().is_empty());
        });
    }
    /// An arrow's contacts, on the same batch sweep the other projectiles':
    /// a flying arrow meets its blocks in walk order, once each, and a stuck
    /// arrow keeps the contact of the block it sits in.
    #[test]
    #[ignore = "daily"]
    fn an_arrow_is_told_once_a_block_stuck_and_flying() {
        if fresh("an_arrow_is_told_once_a_block_stuck_and_flying") {
            return;
        }
        let part = InsidePart::default();
        let mods = install_inside_mod("inside-arrow", &part);
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let _ = sunrise_sim::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                for x in [1, 2] {
                    game.world.set(x, 100, 0, stone).unwrap();
                }
                let flying = arrow_shot(9, (0.5, 100.0, 0.5), (2.5, 100.0, 0.5));
                let before = flying.velocity.x;
                let mut flying = flying;
                sunrise_sim::block_contact::in_level(
                    sunrise_sim::parallel::TickThread::mint(),
                    Some("minecraft:overworld"),
                    || {
                        game.sweep_projectile_inside(&mut flying, None);
                    },
                );
                assert_eq!(
                    *part.contacts.lock().unwrap(),
                    vec![[1, 100, 0], [2, 100, 0]],
                    "the flying arrow meets its blocks in walk order, once each"
                );
                assert!(
                    (flying.velocity.x - before - 1.0).abs() < 1e-9,
                    "the part's pushes applied to the arrow"
                );
                part.contacts.lock().unwrap().clear();
                // A stuck arrow: no move, the block it sits in still tells the part.
                let mut stuck = arrow_shot(10, (1.5, 100.0, 0.5), (1.5, 100.0, 0.5));
                stuck.velocity = sunrise_core::geometry::Vec3::ZERO;
                sunrise_sim::block_contact::in_level(
                    sunrise_sim::parallel::TickThread::mint(),
                    Some("minecraft:overworld"),
                    || {
                        game.sweep_projectile_inside(&mut stuck, None);
                    },
                );
                assert_eq!(
                    *part.contacts.lock().unwrap(),
                    vec![[1, 100, 0]],
                    "the stuck arrow keeps the contact of its block"
                );
                assert!(sunrise_sim::block_contact::take_pending().is_empty());
            });
        });
    }
    /// A panicking part inside the arrow batch: the listener's panic is contained
    /// in the staged fire, the lifted projectile row returns to the level's list
    /// exactly once, and the arrow keeps its state.
    #[test]
    #[ignore = "daily"]
    fn a_panicking_part_leaves_the_arrow_list_whole() {
        if fresh("a_panicking_part_leaves_the_arrow_list_whole") {
            return;
        }
        let part = InsidePart {
            panic_after: Some(0),
            ..InsidePart::default()
        };
        let mods = install_inside_mod("inside-arrow-panic", &part);
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let _ = sunrise_sim::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                for x in [1, 2] {
                    game.world.set(x, 100, 0, stone).unwrap();
                }
                // The arrow joins the level's own list: the batch's lifted row.
                let shot = arrow_shot(11, (0.5, 100.0, 0.5), (2.5, 100.0, 0.5));
                let velocity = shot.velocity.x;
                let id = shot.id;
                game.creatures.projectiles.push(shot);
                let rows = game.creatures.projectiles.len();
                let mut projectiles = game.creatures.projectiles.lift();
                sunrise_sim::block_contact::in_level(
                    sunrise_sim::parallel::TickThread::mint(),
                    Some("minecraft:overworld"),
                    || {
                        projectiles.update(0, |p| game.sweep_projectile_inside(p, None));
                    },
                );
                game.creatures.projectiles.restore(
                    projectiles,
                    sunrise_sim::join_seq::RestoreOrder::SpawnsFirst,
                );
                assert_eq!(
                    part.contacts.lock().unwrap().len(),
                    1,
                    "contact #1 panicked and disabled this mod's entity_inside callback for the session"
                );
                assert_eq!(
                    mod_api::__private::take_hook_panics(),
                    [mod_api::__private::HookPanicReport {
                        mod_id: "inside-arrow-panic",
                        hook: "entity_inside",
                    }]
                );
                assert_eq!(
                    game.creatures.projectiles.len(),
                    rows,
                    "the lifted row returned, exactly once"
                );
                let arrow = game
                    .creatures
                    .projectiles
                    .iter()
                    .next()
                    .expect("the arrow is still in the list");
                assert_eq!(arrow.id, id, "the same arrow, not lost nor duplicated");
                assert!(
                    (arrow.velocity.x - velocity).abs() < 1e-9,
                    "the panicking callback's staged writes never applied"
                );
                assert!(sunrise_sim::block_contact::take_pending().is_empty());
            });
        });
    }
}
