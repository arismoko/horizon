//! Daily tests for per-state block movement data on `BlockStates`.

use super::native_client_session::tests::logged_client;
use super::native_entity_refs::tests::with_client_game;
use mod_api::{BlockLanding, ClientHost, HostBlockStates, PathType, StateCollision};

#[test]
#[ignore = "daily"]
fn client_block_state_collision_classifies_fixed_contextual_and_positional() {
    mod_platform::testing::with_mods(Default::default(), || {
        with_client_game(|game| {
            let _server = logged_client(game);
            let host = &*game as &dyn ClientHost;
            let states = host.block_states();
            let state = |name: &str| host.block_state_id(name).unwrap();

            assert_eq!(
                states.collision(state("minecraft:stone")),
                Some(StateCollision::Fixed(vec![[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]]))
            );
            assert_eq!(
                states.collision(state("minecraft:air")),
                Some(StateCollision::Fixed(vec![]))
            );
            assert_eq!(
                states.collision(state("minecraft:oak_fence")),
                Some(StateCollision::Fixed(vec![[
                    0.375, 0.0, 0.375, 0.625, 1.5, 0.625
                ]]))
            );
            assert_eq!(
                states.collision(state("minecraft:bamboo")),
                Some(StateCollision::Positional)
            );
            assert_eq!(
                states.collision(state("minecraft:pointed_dripstone")),
                Some(StateCollision::Positional)
            );
            assert_eq!(
                states.collision(state("minecraft:shulker_box")),
                Some(StateCollision::Positional)
            );
            assert!(matches!(
                states.collision(state("minecraft:scaffolding")),
                Some(StateCollision::Contextual(boxes)) if !boxes.is_empty()
            ));
            assert_eq!(
                states.collision(state("minecraft:powder_snow")),
                Some(StateCollision::Contextual(vec![]))
            );
            assert_eq!(
                states.collision(state("minecraft:water")),
                Some(StateCollision::Contextual(vec![]))
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn client_block_state_speed_jump_and_landing_match_vanilla() {
    mod_platform::testing::with_mods(Default::default(), || {
        with_client_game(|game| {
            let _server = logged_client(game);
            let host = &*game as &dyn ClientHost;
            let states = host.block_states();
            let state = |name: &str| host.block_state_id(name).unwrap();

            let soul_sand = state("minecraft:soul_sand");
            assert_eq!(states.speed_factor(soul_sand), Some(0.4));
            assert_eq!(states.jump_factor(soul_sand), Some(1.0));

            let honey = state("minecraft:honey_block");
            assert_eq!(states.speed_factor(honey), Some(0.4));
            assert_eq!(states.jump_factor(honey), Some(0.5));

            let stone = state("minecraft:stone");
            assert_eq!(states.speed_factor(stone), Some(1.0));
            assert_eq!(states.jump_factor(stone), Some(1.0));
            assert_eq!(
                states.landing(stone),
                Some(BlockLanding {
                    bounce_restitution: 0.0,
                    fall_distance_reduction: 0.0,
                })
            );

            // Vanilla 26.3's `Blocks`: beds and the shelf mushroom declare `bounceRestitution(0.75F)`
            // and `fallDistanceReduction(0.5F)`, slime `bounceRestitution(1.0F)` only; hay sets
            // neither (its lighter fall is a `fallOn` multiplier).
            for bouncy in ["minecraft:red_bed", "minecraft:shelf_mushroom"] {
                assert_eq!(
                    states.landing(state(bouncy)),
                    Some(BlockLanding {
                        bounce_restitution: 0.75,
                        fall_distance_reduction: 0.5,
                    }),
                    "{bouncy}"
                );
            }
            assert_eq!(
                states.landing(state("minecraft:slime_block")),
                Some(BlockLanding {
                    bounce_restitution: 1.0,
                    fall_distance_reduction: 0.0,
                })
            );
            assert_eq!(
                states.landing(state("minecraft:hay_block")),
                Some(BlockLanding {
                    bounce_restitution: 0.0,
                    fall_distance_reduction: 0.0,
                })
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn client_block_state_path_type_matches_vanilla() {
    mod_platform::testing::with_mods(Default::default(), || {
        with_client_game(|game| {
            let _server = logged_client(game);
            let host = &*game as &dyn ClientHost;
            let states = host.block_states();
            let state = |name: &str| host.block_state_id(name).unwrap();

            assert_eq!(
                states.path_type(state("minecraft:stone")),
                Some(PathType::Blocked)
            );
            assert_eq!(
                states.path_type(state("minecraft:air")),
                Some(PathType::Open)
            );
            assert_eq!(
                states.path_type(state("minecraft:lava")),
                Some(PathType::Lava)
            );
            assert_eq!(
                states.path_type(state("minecraft:fire")),
                Some(PathType::Fire)
            );
            assert_eq!(
                states.path_type(state("minecraft:cactus")),
                Some(PathType::Damaging)
            );
            assert_eq!(
                states.path_type(state("minecraft:honey_block")),
                Some(PathType::StickyHoney)
            );
            assert_eq!(
                states.path_type(state("minecraft:powder_snow")),
                Some(PathType::PowderSnow)
            );
            assert_eq!(
                states.path_type(state("minecraft:oak_fence")),
                Some(PathType::Fence)
            );
        });
    });
}
