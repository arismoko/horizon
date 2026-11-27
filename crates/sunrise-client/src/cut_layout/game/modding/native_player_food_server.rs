#![cfg(test)]
use mod_api::PlayerRef;
use sunrise_gameplay_common::game::native_player_food::NativePlayerFoodGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use super::*;
    use mod_api::{EntityRef, WorldEpoch};
    use sunrise_core::geometry::Vec3;
    use sunrise_items::items::ItemStack;
    use sunrise_sim::motion::PlayerMotion;

    /// The acting player with a body, as a hook's turn finds them.
    fn acting_player(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> PlayerRef {
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
        game.player(123).unwrap()
    }

    /// A mod spends experience on Mending as an orb's pickup does: 2 durability a point, the
    /// rest returned and the player's experience untouched (#5593).
    #[test]
    #[ignore = "daily"]
    fn repairing_with_experience_returns_what_mending_leaves() {
        with_game(|game| {
            let player = acting_player(game);
            let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
            sword.damage = 1000;
            sword.enchant("minecraft:mending", 1);
            game.common.inventory[game.common.selected] = Some(sword);
            assert_eq!(player.repair_items_with_xp(game, 7), Some(0));
            let damage = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                game.inventory[game.selected].as_ref().unwrap().damage
            };
            assert_eq!(damage(game), 986);
            // ExperienceOrb.repairPlayerItems: 4 damage costs 2 of 7 points, and the 5 left find
            // nothing more to repair.
            let selected = game.common.selected;
            game.common.inventory[selected].as_mut().unwrap().damage = 4;
            assert_eq!(player.repair_items_with_xp(game, 7), Some(5));
            assert_eq!(damage(game), 0);
            assert_eq!(player.repair_items_with_xp(game, 7), Some(7));
            let xp = player.experience(game).unwrap();
            assert_eq!((xp.level, xp.total), (0, 0));
            let stale = mod_api::__private::game::player_ref(
                EntityRef::new(WorldEpoch::new(u64::MAX), 7),
                player.uuid(),
            );
            assert_eq!(stale.repair_items_with_xp(game, 7), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn signed_points_bypass_mending() {
        with_game(|game| {
            let player = acting_player(game);
            let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
            sword.damage = 1000;
            sword.enchant("minecraft:mending", 1);
            game.common.inventory[game.common.selected] = Some(sword);
            assert!(player.give_experience_points(game, 7));
            let xp = player.experience(game).unwrap();
            assert_eq!((xp.level, xp.progress, xp.total), (1, 0.0, 7));
            assert!(player.give_experience_points(game, -7));
            let xp = player.experience(game).unwrap();
            assert_eq!((xp.level, xp.progress, xp.total), (0, 0.0, 0));
            assert_eq!(game.inventory[game.selected].as_ref().unwrap().damage, 1000);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn levels_are_added_taken_and_clamped_at_zero() {
        with_game(|game| {
            let player = acting_player(game);
            game.experience.progress = 0.25;
            assert!(game.player_give_levels(player, 3));
            assert_eq!(game.experience.level, 3);
            assert_eq!(
                game.experience.progress, 0.25,
                "a level add keeps the progress"
            );
            game.experience.total = 40;
            assert!(game.player_give_levels(player, -5));
            assert_eq!(game.experience.level, 0, "below zero clamps to zero");
            assert_eq!(game.experience.total, 0, "the clamp clears the total");
            assert_eq!(
                game.experience.progress, 0.0,
                "the clamp clears the progress"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn enchantment_debits_ask_but_absolute_level_writes_only_report() {
        use std::sync::{Arc, Mutex};
        struct Observe(Arc<Mutex<Vec<(&'static str, i32, i32)>>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Observe {
            fn player_xp_level_change(
                &self,
                host: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::PlayerXpLevelChangeEvent<'_>,
            ) -> mod_api::Flow {
                let level = host
                    .players()
                    .acting_player()
                    .unwrap()
                    .experience(host)
                    .unwrap()
                    .level;
                self.0.lock().unwrap().push(("before", level, event.levels));
                if event.levels == -4 {
                    mod_api::Flow::Cancel
                } else {
                    mod_api::Flow::Continue
                }
            }

            fn player_level_change(
                &self,
                host: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::PlayerLevelChangeEvent<'_>,
            ) {
                let level = host
                    .players()
                    .acting_player()
                    .unwrap()
                    .experience(host)
                    .unwrap()
                    .level;
                assert_eq!(event.to, level);
                self.0.lock().unwrap().push(("after", event.from, level));
            }
        }
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "xp",
            Box::new(Observe(heard.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let player = acting_player(game);
                game.server.lan_acting_name = Some("LevelProbe".into());
                game.server.lan_acting_permission = Some(0);
                // A player's turn on the server thread: `acting_player` names the
                // body `bind_local_player` gave the turn.
                game.server.lan_acting = Some(1);
                assert!(player.set_experience_levels(game, 5));
                assert!(player.set_experience_levels(game, 5));
                game.enchantment_performed(4);
                assert_eq!(game.experience.level, 5);
                game.enchantment_performed(2);
                assert_eq!(game.experience.level, 3);
                assert_eq!(
                    *heard.lock().unwrap(),
                    [
                        ("after", 0, 5),
                        ("before", 5, -4),
                        ("before", 5, -2),
                        ("after", 5, 3),
                    ]
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn the_signed_total_is_set_alone() {
        with_game(|game| {
            let player = acting_player(game);
            game.experience.level = 5;
            game.experience.progress = 0.5;
            assert!(game.player_set_total_experience(player, 1234));
            assert_eq!(game.player_experience(player).unwrap().total, 1234);
            assert_eq!(game.experience.level, 5, "the level is kept");
            assert_eq!(game.experience.progress, 0.5, "the progress is kept");
            assert!(game.player_set_total_experience(player, i32::MIN));
            let xp = player.experience(game).unwrap();
            assert_eq!((xp.level, xp.progress, xp.total), (5, 0.5, i32::MIN));
            assert_eq!(game.player_score, 0);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn player_score_uses_the_signed_hook_adjusted_grant() {
        struct TakePoint;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for TakePoint {
            fn player_xp_change(
                &self,
                _: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::PlayerXpChangeEvent<'_>,
            ) -> mod_api::Flow {
                event.amount = -1;
                mod_api::Flow::Continue
            }
        }
        let mods = vec![mod_api::ModEntry::new("take-point", Box::new(TakePoint))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let player = acting_player(game);
                game.server.lan_acting_name = Some("ScoreProbe".into());
                assert!(player.give_experience_points(game, 1));
                assert_eq!(game.player_score, -1);
                assert_eq!(game.experience.total, 0);

                game.player_score = i32::MIN;
                assert!(player.give_experience_points(game, 1));
                assert_eq!(game.player_score, i32::MAX);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_listener_can_rewrite_a_zero_point_grant() {
        struct GiveOne;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for GiveOne {
            fn player_xp_change(
                &self,
                _: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::PlayerXpChangeEvent<'_>,
            ) -> mod_api::Flow {
                event.amount = 1;
                mod_api::Flow::Continue
            }
        }
        let mods = vec![mod_api::ModEntry::new("give-one", Box::new(GiveOne))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let player = acting_player(game);
                game.server.lan_acting_name = Some("ScoreProbe".into());
                game.player_score = 0;
                assert!(player.give_experience_points(game, 0));
                assert_eq!(game.player_score, 1);
                assert_eq!(game.experience.total, 1);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_stale_handle_gives_nothing() {
        with_game(|game| {
            let player = acting_player(game);
            let stale = mod_api::__private::game::player_ref(
                EntityRef::new(WorldEpoch::new(u64::MAX), 7),
                player.uuid(),
            );
            assert!(!game.player_give_experience_points(stale, 10));
            assert!(!game.player_give_levels(stale, 1));
            assert!(!game.player_set_total_experience(stale, 10));
            assert_eq!(game.experience.total, 0);
            assert_eq!(game.experience.level, 0);
        });
    }
}
