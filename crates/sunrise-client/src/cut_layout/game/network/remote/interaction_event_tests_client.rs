use crate::game::remote::interaction_event_tests::*;
use mod_api::{self, InteractionResult as Answer, LogicalSide, UsePhase};
use std::sync::Arc;
use sunrise_gameplay_common::game::native_hooks::{
    ModGameExt1 as _, SharedGameExt1 as _, StaleBlockClick,
};
use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::Chunk;
use sunrise_sim::first_person_sim::Hand;

#[path = "mapping_press_tests_client.rs"]
pub(crate) mod mapping_press_tests;

impl<'g> InteractionSide<'g> for sunrise_client_gameplay::game::ClientGame<'g> {
    const IS_CLIENT: bool = true;
    const LOGICAL_SIDE: LogicalSide = LogicalSide::Client;
    fn set_interaction_mode(game: &mut Self, mode: sunrise_command::args::GameMode) {
        game.set_local_mode(mode);
    }
    fn interaction_use_block_hand(game: &mut Self, hand: Hand) -> bool {
        game.remote_use_item_on(hand, mod_api::InteractionAttempt::Press, None)
    }
    fn interaction_use_air_hand(game: &mut Self, hand: Hand) -> bool {
        game.remote_use_item(hand)
    }
    fn interaction_use_item_as_player(game: &mut Self) -> bool {
        game.remote_use_item(Hand::Main)
    }
    fn interaction_set_sneak(game: &mut Self, sneak: bool) {
        game.set_mapping_held("key.sneak", sneak);
        if let Some(remote) = game.client.remote.as_mut() {
            if sneak {
                remote.last_input |= sunrise_gameplay_common::game::remote::SHIFT_INPUT;
            } else {
                remote.last_input &= !sunrise_gameplay_common::game::remote::SHIFT_INPUT;
            }
        }
    }
    fn interaction_clear_guest_use(_game: &mut Self) {}
    fn interaction_reset_use_failed(_game: &mut Self) {}
}

#[test]
#[ignore = "daily"]
fn client_block_part_replaces_the_template_for_either_hand() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "block_prediction",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_client_game(|game| {
            let lever = game
                .pack
                .assets
                .registry
                .default_state("minecraft:lever")
                .unwrap();
            game.world.set(0, 64, 0, lever).unwrap();
            game.set_test_forced_hit(Some(hit()));
            game.offhand = Some(ItemStack::new("minecraft:stick", 1));
            let block = Step::Phase(UsePhase::Block, LogicalSide::Client);
            for hand in [Hand::Main, Hand::Off] {
                for (answer, ends, swings) in [
                    (Answer::Success, true, 1),
                    (Answer::SuccessServer, true, 0),
                    (Answer::Consume, true, 0),
                    (Answer::Pass, false, 0),
                    (Answer::Fail, false, 0),
                ] {
                    *observer.0.lock().unwrap() = Controls {
                        expected_hand: Some(hand),
                        block_part_answer: Some(answer),
                        record_empty: true,
                        ..Controls::default()
                    };
                    let before = game.swings_started;
                    assert_eq!(
                        game.remote_use_item_on(hand, mod_api::InteractionAttempt::Press, None),
                        ends,
                        "{hand:?} {answer:?}"
                    );
                    assert_eq!(game.swings_started - before, swings);
                    assert_eq!(game.world.block(0, 64, 0), lever);
                    let state = observer.0.lock().unwrap();
                    assert_eq!(state.use_item_on_packets, 1);
                    assert_eq!(state.use_item_packets, 0);
                    let mut expected = vec![block.clone(), Step::BlockPart];
                    if !ends {
                        expected.push(Step::Phase(UsePhase::ItemAfterBlock, LogicalSide::Client));
                    }
                    assert_eq!(state.steps, expected);
                }
            }
        });
    });
}

#[test]
#[ignore = "daily"]
fn client_empty_block_part_runs_only_for_the_selected_main_hand_fallback() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "empty_prediction",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_client_game(|game| {
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let lever = game
                .with_props(
                    registry.default_state("minecraft:lever").unwrap(),
                    &[("face", "floor")],
                )
                .unwrap();
            game.world.set(0, 63, 0, stone).unwrap();
            game.world.set(0, 64, 0, lever).unwrap();
            game.set_test_forced_hit(Some(hit()));
            game.offhand = Some(ItemStack::new("minecraft:stick", 1));
            for hand in [Hand::Main, Hand::Off] {
                for (event_answer, part_answer) in [
                    (None, None),
                    (None, Some(Answer::TryWithEmptyHand)),
                    (Some(Answer::TryWithEmptyHand), Some(Answer::Fail)),
                ] {
                    *observer.0.lock().unwrap() = Controls {
                        expected_hand: Some(hand),
                        block_answer: event_answer,
                        block_part_answer: part_answer,
                        record_empty: true,
                        empty_part_answer: Some(Answer::SuccessServer),
                        ..Controls::default()
                    };
                    let before = game.swings_started;
                    assert_eq!(
                        game.remote_use_item_on(hand, mod_api::InteractionAttempt::Press, None),
                        hand == Hand::Main
                    );
                    assert_eq!(game.swings_started, before);
                    assert_eq!(game.world.block(0, 64, 0), lever);
                    let state = observer.0.lock().unwrap();
                    assert_eq!(state.use_item_on_packets, 1);
                    let mut expected = vec![Step::Phase(UsePhase::Block, LogicalSide::Client)];
                    if event_answer.is_none() {
                        expected.push(Step::BlockPart);
                    }
                    expected.push(if hand == Hand::Main {
                        Step::EmptyPart(Hand::Main, true)
                    } else {
                        Step::Phase(UsePhase::ItemAfterBlock, LogicalSide::Client)
                    });
                    assert_eq!(state.steps, expected);
                }
            }
            // The empty-hand Part replaces the lever's toggle even when its
            // answer falls through. The selected item phase and Part run next.
            for answer in [Answer::Pass, Answer::Fail, Answer::TryWithEmptyHand] {
                *observer.0.lock().unwrap() = Controls {
                    record_empty: true,
                    empty_part_answer: Some(answer),
                    client_use_on_answer: Some(Answer::Consume),
                    ..Controls::default()
                };
                let before = game.swings_started;
                assert!(
                    game.remote_use_item_on(Hand::Main, mod_api::InteractionAttempt::Press, None),
                    "{answer:?}"
                );
                assert_eq!(game.swings_started, before);
                assert_eq!(game.world.block(0, 64, 0), lever);
                let state = observer.0.lock().unwrap();
                assert_eq!(state.use_item_on_packets, 1);
                assert_eq!(state.use_item_packets, 0);
                assert_eq!(
                    state.steps,
                    [
                        Step::Phase(UsePhase::Block, LogicalSide::Client),
                        Step::BlockPart,
                        Step::EmptyPart(Hand::Main, true),
                        Step::Phase(UsePhase::ItemAfterBlock, LogicalSide::Client),
                        Step::UseOnPart(LogicalSide::Client),
                    ],
                    "{answer:?}",
                );
            }
            // When neither Part answers, the ordinary lever prediction still runs.
            *observer.0.lock().unwrap() = Controls::default();
            let before = game.swings_started;
            assert!(game.remote_use_item_on(Hand::Main, mod_api::InteractionAttempt::Press, None));
            assert_eq!(game.swings_started - before, 1);
            assert_ne!(game.world.block(0, 64, 0), lever);
            assert_eq!(observer.0.lock().unwrap().use_item_on_packets, 1);
        });
    });
}

#[test]
#[ignore = "daily"]
fn an_unrepresentable_held_stack_defers_to_client_block_prediction() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "refused_prediction",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_client_game(|game| {
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let lever = game
                .with_props(
                    registry.default_state("minecraft:lever").unwrap(),
                    &[("face", "floor")],
                )
                .unwrap();
            game.world.set(0, 63, 0, stone).unwrap();
            game.world.set(0, 64, 0, lever).unwrap();
            game.set_test_forced_hit(Some(hit()));
            let selected = game.selected;
            game.inventory[selected] = Some(ItemStack::new("minecraft:stick", 0));
            *observer.0.lock().unwrap() = Controls {
                record_first: true,
                record_empty: true,
                first_part_answer: Some(Answer::Consume),
                block_part_answer: Some(Answer::Consume),
                empty_part_answer: Some(Answer::Consume),
                ..Controls::default()
            };
            let before = game.swings_started;
            assert!(game.remote_use_item_on(Hand::Main, mod_api::InteractionAttempt::Press, None));
            assert_eq!(game.swings_started - before, 1);
            assert_ne!(game.world.block(0, 64, 0), lever);
            let state = observer.0.lock().unwrap();
            assert!(state.steps.is_empty());
            assert_eq!(state.use_item_on_packets, 1);
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_stale_client_block_click_discards_the_part_decision() {
    with_interaction_client_game(|game| {
        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let lever = registry.default_state("minecraft:lever").unwrap();
        for replace_target in [true, false] {
            game.world.set(0, 64, 0, lever).unwrap();
            let selected = game.selected;
            game.inventory[selected] = Some(ItemStack::new("minecraft:stick", 1));
            let held = game.inventory[selected].clone();
            let click = game.block_click(&hit(), Hand::Main, None);
            let mut calls = 0;
            let answer =
                game.with_block_click_part(&click, lever, held.as_ref(), |game, answer| {
                    calls += 1;
                    if replace_target {
                        game.world.set(0, 64, 0, stone).unwrap();
                    } else {
                        let selected = game.selected;
                        game.inventory[selected].as_mut().unwrap().count += 1;
                    }
                    *answer = Some(Answer::Success);
                });
            assert_eq!(calls, 1);
            assert_eq!(answer, Err(StaleBlockClick));
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_slot_changing_client_part_keeps_the_original_slot_for_the_block_request() {
    use crate::game::client_side::native_hotbar_slot::hotbar_slot_tests::{logged_in, sent};
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use sunrise_protocol::packets::play::{self, Serverbound as S};

    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "slot_changing_part",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            game.set_local_mode(sunrise_command::args::GameMode::Creative);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let lever = game
                .pack
                .assets
                .registry
                .default_state("minecraft:lever")
                .unwrap();
            game.world.set(0, 64, 0, lever).unwrap();
            game.set_test_forced_hit(Some(hit()));
            game.selected = 2;
            game.inventory[2] = Some(ItemStack::new("minecraft:stick", 1));
            game.inventory[4] = Some(ItemStack::new("minecraft:stone", 1));
            game.offhand = Some(ItemStack::new("minecraft:shield", 1));
            *observer.0.lock().unwrap() = Controls {
                block_part_select_slot: Some(4),
                block_part_answer: Some(Answer::Success),
                ..Controls::default()
            };
            let before = game.swings_started;
            game.remote_use(mod_api::InteractionAttempt::Press);

            assert_eq!(game.selected, 4, "the local selection still changes");
            assert_eq!(
                game.swings_started, before,
                "the stale answer must not swing"
            );
            assert_eq!(game.world.block(0, 64, 0), lever);
            let packets = sent(&mut reader);
            assert!(
                matches!(
                    packets.as_slice(),
                    [S::SetCarriedItem(play::SetCarriedItem { slot: 2 }), S::UseItemOn(request)]
                        if request.hand == 0
                            && request.pos == [0, 64, 0]
                            && request.face == 1
                            && request.cursor == [0.5, 1.0, 0.5]
                            && !request.inside
                            && !request.world_border
                            && request.sequence == 1
                ),
                "expected the original slot followed by one unchanged request: {packets:?}",
            );
            let state = observer.0.lock().unwrap();
            assert_eq!(state.click_hands, [Hand::Main]);
            assert_eq!(
                state.steps,
                [
                    Step::Phase(UsePhase::Block, LogicalSide::Client),
                    Step::BlockPart
                ],
                "the stale decision must not retry or fall through",
            );
        });
    });
}
