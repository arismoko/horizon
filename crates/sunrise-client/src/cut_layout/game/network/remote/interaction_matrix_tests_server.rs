#[cfg(feature = "client")]
use super::*;
#[cfg(feature = "client")]
use crate::game::remote::interaction_event_tests::interaction_matrix_tests::advancement_tree;
#[cfg(feature = "client")]
use crate::game::remote::interaction_event_tests::interaction_matrix_tests::cases;
#[cfg(feature = "client")]
use crate::game::remote::interaction_event_tests::interaction_matrix_tests::*;

#[cfg(feature = "client")]
pub(crate) fn matrix(mods: mod_api::Mods) {
    fn run<'g, S: InteractionSide<'g>>(game: &mut S) {
        let client = S::IS_CLIENT;
        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let lever = game
            .with_props(
                registry.default_state("minecraft:lever").unwrap(),
                &[("face", "floor")],
            )
            .unwrap();
        game.world.set(0, 63, 0, stone).unwrap();
        // A FULL read of this hand-made column (the advancement's block read) finds it ready.
        std::sync::Arc::make_mut(game.world.chunks.get_mut(&(0, 0)).unwrap()).lit = true;
        game.set_test_forced_hit(Some(hit()));
        for hand in [Hand::Main, Hand::Off] {
            for case in cases(hand) {
                let state = if case.lever { lever } else { stone };
                game.world.set(0, 64, 0, state).unwrap();
                let held = case.held.then(|| ItemStack::new("minecraft:stick", 1));
                let other = case
                    .other_held
                    .then(|| ItemStack::new("minecraft:stick", 1));
                let common = game.common_mut();
                (common.inventory[common.selected], common.offhand) = if hand == Hand::Main {
                    (held, other)
                } else {
                    (other, held)
                };
                // Each case supplies the logical sneak state of the acting player.
                S::interaction_set_sneak(game, case.sneak);
                game.swing = Default::default();
                S::interaction_reset_use_failed(game);
                advancement_tree(game);
                fixture::begin(case.controls);
                let swings = game.swings_started;
                let used = S::interaction_use_block_hand(game, hand);
                assert_eq!(used, case.used, "{client:?} {hand:?} {case:?}");
                assert_eq!(
                    game.swings_started - swings,
                    case.swings[usize::from(client)],
                    "{client:?} {hand:?} {case:?}"
                );
                if game.swings_started != swings {
                    assert_eq!(game.swing.attack_hand(), hand);
                }
                assert_eq!(
                    game.world.block(0, 64, 0) != state,
                    case.default_award,
                    "{client:?} {hand:?} {case:?}"
                );
                for (name, expected) in [
                    ("item_used_on_block", case.item_award),
                    ("default_block_use", case.default_award),
                ] {
                    assert_eq!(
                        game.adv.progress.has(&format!("test:{name}"), "seen"),
                        !client && expected,
                        "{client:?} {hand:?} {name} {case:?}"
                    );
                }
                let expected: Vec<u8> = case
                    .phases
                    .iter()
                    .flat_map(|phase| {
                        [
                            *phase,
                            u8::from(hand == Hand::Off),
                            u8::from(client),
                            u8::from(!case.held),
                        ]
                    })
                    .collect();
                assert_eq!(fixture::finish(), expected, "{client:?} {hand:?} {case:?}");
                assert_eq!(
                    game.with_hand(hand, |g| g.held().map(|s| (s.item.to_string(), s.count))),
                    case.held.then(|| ("minecraft:stick".into(), 1))
                );
            }
        }
    }

    // with_mods registers the mods again for their controllers: install Parts after it, as
    // registration-time declarations (a custom statistic) refuse once the registry is frozen.
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        // The fixture declares a block-entity type of its own: the game stands over the content
        // root its mods make, built once for both of this matrix's games.
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let root = sunrise_gameplay_common::game::mods::content_root(&base, &Default::default())
            .unwrap()
            .prepared()
            .to_owned();
        with_interaction_server_game_at(root.clone(), |game| run(game));
        with_interaction_client_game_at(root, |game| run(game));
    });
}

#[cfg(feature = "client")]
use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

#[cfg(feature = "client")]
use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
