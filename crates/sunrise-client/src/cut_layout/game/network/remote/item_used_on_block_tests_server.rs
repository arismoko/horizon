use super::*;
pub(crate) use crate::game::remote::interaction_event_tests::item_used_on_block_tests::install_criteria;

#[test]
#[ignore = "daily"]
fn compass_criterion_sees_two_compasses_before_the_block_takes_one() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "item_used_on_block_observer",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_server_game(|game| {
            install_criteria(game, &[1, 2], "minecraft:compass");
            Arc::make_mut(game.world.chunks.get_mut(&(0, 0)).unwrap()).lit = true;
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:lodestone").unwrap();
            game.world.set(0, 64, 0, stone).unwrap();
            game.set_test_forced_hit(Some(hit()));
            game.common.inventory[game.common.selected] =
                Some(ItemStack::new("minecraft:compass", 2));
            *observer.0.lock().unwrap() = Controls {
                block_answer: Some(Answer::Pass),
                take_in_block: true,

                ..Controls::default()
            };
            assert!(use_block(game));

            assert_eq!(game.held().map(|stack| stack.count), Some(1));
            assert!(game.adv.progress.has("test:sticks2", "seen"));
            assert!(!game.adv.progress.has("test:sticks1", "seen"));
        });
    });
}

#[test]
#[ignore = "daily"]
fn the_item_phase_criterion_sees_the_stack_from_before_the_block_phase() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "item_used_on_block_observer",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_server_game(|game| {
            Arc::make_mut(game.world.chunks.get_mut(&(0, 0)).unwrap()).lit = true;
            install_criteria(game, &[1, 2], "minecraft:stick");
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            game.world.set(0, 64, 0, stone).unwrap();
            game.set_test_forced_hit(Some(hit()));
            game.common.inventory[game.common.selected] =
                Some(ItemStack::new("minecraft:stick", 2));
            // The block's turn takes a stick from the hand and passes; the
            // item's turn then consumes the click.
            *observer.0.lock().unwrap() = Controls {
                block_answer: Some(Answer::Pass),
                take_in_block: true,
                item_on_answer: Some(Answer::Consume),
                ..Controls::default()
            };
            assert!(use_block(game));
            assert_eq!(
                observer.0.lock().unwrap().steps,
                [
                    Step::Phase(UsePhase::Block, LogicalSide::Server),
                    Step::Phase(UsePhase::ItemAfterBlock, LogicalSide::Server),
                ]
            );
            assert_eq!(game.held().map(|stack| stack.count), Some(1));
            assert!(game.adv.progress.has("test:sticks2", "seen"));
            assert!(!game.adv.progress.has("test:sticks1", "seen"));
        });
    });
}

/// `ServerPlayerGameMode.useItemOn` hands the block the stack in hand after
/// the block phase's listeners: one that takes the last fire charge and lets
/// the click go on leaves TNT nothing to light it with.
#[test]
#[ignore = "daily"]
fn tnt_reads_the_hand_the_block_phase_listener_left() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "item_used_on_block_observer",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_server_game(|game| {
            let tnt = game
                .pack
                .assets
                .registry
                .default_state("minecraft:tnt")
                .unwrap();
            game.world.set(0, 64, 0, tnt).unwrap();
            game.set_test_forced_hit(Some(hit()));
            game.common.inventory[game.common.selected] =
                Some(ItemStack::new("minecraft:fire_charge", 1));
            *observer.0.lock().unwrap() = Controls {
                take_in_block: true,
                ..Controls::default()
            };
            use_block(game);
            assert!(game.held().is_none(), "the listener took the fire charge");
            assert!(game.tnts.is_empty(), "nothing primed");
            assert_eq!(game.world.block(0, 64, 0), tnt);
        });
    });
}

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
