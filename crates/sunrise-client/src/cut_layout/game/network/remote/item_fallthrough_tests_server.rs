use super::*;
use crate::game::remote::interaction_event_tests::item_fallthrough_tests::with_packet_game;

pub(crate) fn assert_block_item_before_fallthrough(
    item: &str,
    block: &str,
    expected: &str,
    expected_events: usize,
) {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "use_on_order",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_server_game(|game| {
            game.set_mode_id(0);
            let state = game.pack.assets.registry.default_state(block).unwrap();
            let dirt = game
                .pack
                .assets
                .registry
                .default_state("minecraft:dirt")
                .unwrap();
            game.world.set(0, 63, 0, dirt).unwrap();
            game.world.set(0, 64, 0, state).unwrap();
            game.set_test_forced_hit(Some(hit()));
            game.common.inventory[game.common.selected] = Some(ItemStack::new(item, 2));
            *observer.0.lock().unwrap() = Controls {
                air_answer: Some(Answer::Fail),
                ..Controls::default()
            };
            let used = game.use_hand(Some(&hit())).unwrap();
            assert_eq!(used, expected_events == 0);
            let y = if item == "minecraft:stone" { 65 } else { 64 };
            assert_eq!(&*game.block_name([0, y, 0]), expected);
            if item == "minecraft:bone_meal" || item == "minecraft:stone" {
                assert_eq!(game.held().unwrap().count, 1);
            }
            let state = observer.0.lock().unwrap();
            assert_eq!(
                state
                    .steps
                    .iter()
                    .filter(|s| matches!(s, Step::Air(_, _)))
                    .count(),
                expected_events
            );
        });
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn block_pass_falls_through_but_fail_and_success_stop_the_apple() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "block_fallthrough",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_client_game(|game| {
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(0, 64, 0, stone).unwrap();
            game.set_test_forced_hit(Some(hit()));
            for (answer, expected) in [(Answer::Pass, 1), (Answer::Fail, 0), (Answer::Success, 0)] {
                let selected = game.selected;
                game.inventory[selected] = Some(ItemStack::new("minecraft:apple", 1));
                *observer.0.lock().unwrap() = Controls {
                    hands: Some(Vec::new()),
                    block_answer: Some(Answer::Pass),
                    item_on_answer: Some(answer),
                    air_answer: Some(Answer::Fail),
                    ..Controls::default()
                };
                game.remote_use(mod_api::InteractionAttempt::Press);
                let state = observer.0.lock().unwrap();
                assert_eq!(
                    state
                        .steps
                        .iter()
                        .filter(|step| matches!(step, Step::Air(LogicalSide::Client, Hand::Main)))
                        .count(),
                    expected
                );
                assert!(!state.steps.contains(&Step::ItemPart));
                assert!(game.using.is_none());
            }
        });
    });
}

#[test]
#[ignore = "daily"]
fn server_use_item_packet_fires_once_after_use_item_on_with_block_crosshair() {
    use sunrise_protocol::packets::play::{self, Serverbound as S};
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "packet_fallthrough",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_packet_game(|game| {
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(0, 64, 0, stone).unwrap();
            game.set_test_forced_hit(Some(hit()));
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.inventory[g.selected] = Some(ItemStack::new("minecraft:apple", 1));
            });
            *observer.0.lock().unwrap() = Controls {
                item_on_answer: Some(Answer::Pass),
                air_answer: Some(Answer::Fail),
                ..Controls::default()
            };
            game.lan_guest_packet(
                12,
                S::UseItemOn(play::UseItemOn {
                    hand: 0,
                    pos: [0, 64, 0],
                    face: 1,
                    cursor: [0.5, 1.0, 0.5],
                    inside: false,
                    world_border: false,
                    sequence: 1,
                }),
            );
            assert!(
                observer
                    .0
                    .lock()
                    .unwrap()
                    .steps
                    .iter()
                    .any(|s| matches!(s, Step::Phase(_, LogicalSide::Server)))
            );
            assert!(
                !observer
                    .0
                    .lock()
                    .unwrap()
                    .steps
                    .iter()
                    .any(|s| matches!(s, Step::Air(_, _)))
            );
            game.as_guest(12, |g| assert!(g.pick().is_some())).unwrap();
            game.lan_guest_packet(
                12,
                S::UseItem(play::UseItem {
                    hand: 0,
                    sequence: 2,
                    yaw: 0.0,
                    pitch: 0.0,
                }),
            );
            let state = observer.0.lock().unwrap();
            let hands: Vec<_> = state
                .steps
                .iter()
                .filter_map(|s| match s {
                    Step::Air(LogicalSide::Server, hand) => Some(*hand),
                    _ => None,
                })
                .collect();
            assert_eq!(hands, [Hand::Main]);
        });
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn targeted_item_fallthrough_keeps_hand_and_fail_vetoes_item_action() {
    fn run<'g, S: InteractionSide<'g>>(observer: &Observer, game: &mut S) {
        let side = S::LOGICAL_SIDE;
        game.set_test_forced_hit(Some(hit()));
        for hand in [Hand::Main, Hand::Off] {
            *observer.0.lock().unwrap() = Controls {
                air_answer: Some(Answer::Fail),
                ..Controls::default()
            };
            let selected = game.selected;
            game.inventory[selected] = Some(ItemStack::new("minecraft:apple", 1));
            game.offhand = Some(ItemStack::new("minecraft:apple", 1));
            assert!(!S::interaction_use_air_hand(game, hand));
            assert_eq!(observer.0.lock().unwrap().steps, [Step::Air(side, hand)]);
            assert!(game.using.is_none());
            assert!(!observer.0.lock().unwrap().steps.contains(&Step::ItemPart));
        }
    }

    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "fallthrough_observer",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_server_game(|game| run(&observer, game));
        with_interaction_client_game(|game| run(&observer, game));
    });
}

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
