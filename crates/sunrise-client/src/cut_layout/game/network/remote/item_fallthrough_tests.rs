use super::*;

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn entity_pass_falls_through_but_success_stops_the_apple() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "entity_fallthrough",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        for (entity, expected) in [("minecraft:zombie", 1), ("minecraft:villager", 0)] {
            with_interaction_client_game(|game| {
                game.common.inventory[game.common.selected] =
                    Some(ItemStack::new("minecraft:apple", 1));
                game.offhand = None;
                game.spawn_entity(42, entity, [0.5, 64.0, 5.5], [0.0; 3]);
                game.refresh_client_pick(1.0);
                assert!(game.remote_use_targets_entity());
                // Zombie interaction passes; the awake adult villager succeeds.
                // Entity CONSUME prediction belongs to #2931 and is deliberately excluded.
                *observer.0.lock().unwrap() = Controls {
                    air_answer: Some(Answer::Fail),
                    ..Controls::default()
                };
                game.remote_use(mod_api::InteractionAttempt::Press);
                let state = observer.0.lock().unwrap();
                let hands: Vec<_> = state
                    .steps
                    .iter()
                    .filter_map(|s| match s {
                        Step::Air(LogicalSide::Client, hand) => Some(*hand),
                        _ => None,
                    })
                    .collect();
                assert_eq!(
                    hands,
                    if expected == 1 {
                        vec![Hand::Main]
                    } else {
                        vec![]
                    },
                    "{entity}"
                );
            });
        }
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn air_use_fires_once_and_passing_hands_fire_in_main_off_order() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "air_fallthrough",
        Box::new(observer.clone()),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_client_game(|game| {
            game.common.inventory[game.common.selected] =
                Some(ItemStack::new("minecraft:apple", 1));
            assert!(!game.remote_use_targets_entity());
            assert!(game.pick().is_none());
            for (offhand, expected) in [
                (false, vec![Hand::Main]),
                (true, vec![Hand::Main, Hand::Off]),
            ] {
                game.offhand = offhand.then(|| ItemStack::new("minecraft:apple", 1));
                *observer.0.lock().unwrap() = Controls {
                    air_answer: Some(Answer::Pass),
                    ..Controls::default()
                };
                game.remote_use(mod_api::InteractionAttempt::Press);
                let state = observer.0.lock().unwrap();
                let hands: Vec<_> = state
                    .steps
                    .iter()
                    .filter_map(|s| match s {
                        Step::Air(LogicalSide::Client, hand) => Some(*hand),
                        _ => None,
                    })
                    .collect();
                assert_eq!(hands, expected);
            }
        });
    });
}

pub(crate) fn with_packet_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let _receiver = setup_interaction_guest(game);
        test(game);
    });
}

#[allow(unused_imports)]
pub(crate) use crate::game::server_side::remote_server::interaction_event_tests::item_fallthrough_tests::assert_block_item_before_fallthrough;

#[test]
#[ignore = "daily"]
fn bone_meal_grows_grass_before_item_fallthrough() {
    assert_block_item_before_fallthrough(
        "minecraft:bone_meal",
        "minecraft:short_grass",
        "minecraft:tall_grass",
        0,
    );
}

#[test]
#[ignore = "daily"]
fn axe_strips_oak_log_before_item_fallthrough() {
    assert_block_item_before_fallthrough(
        "minecraft:iron_axe",
        "minecraft:oak_log",
        "minecraft:stripped_oak_log",
        0,
    );
}

#[test]
#[ignore = "daily"]
fn stone_places_before_item_fallthrough() {
    assert_block_item_before_fallthrough(
        "minecraft:stone",
        "minecraft:stone",
        "minecraft:stone",
        0,
    );
}

#[test]
#[ignore = "daily"]
fn apple_on_stone_passes_to_item_fallthrough_once() {
    assert_block_item_before_fallthrough(
        "minecraft:apple",
        "minecraft:stone",
        "minecraft:stone",
        1,
    );
}

#[cfg(feature = "client")]
use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
