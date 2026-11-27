#![cfg(test)]

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::{
        bind_local_player, with_game, with_resident_game,
    };
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::transactions::TransactionsGameExt;
    use sunrise_sim::mobs::Mob;
    use sunrise_sim::motion::PlayerMotion;

    /// A zombie whose `minecraft:max_absorption` is 4.
    fn absorbing_zombie(game: &sunrise_gameplay::game::ServerGame<'_>) -> Mob {
        let mut kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let kind_mut = Arc::make_mut(&mut kind);
        Arc::make_mut(&mut kind_mut.attributes).insert("minecraft:max_absorption".into(), 4.0);
        Mob::new(12, kind, Vec3::ZERO, 0.0)
    }

    #[test]
    #[ignore = "daily"]
    fn a_players_absorption_is_cut_to_zero_and_max_absorption_and_synced() {
        with_game(|game| {
            let mut player = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            player
                .set_modifier(
                    "minecraft:max_absorption",
                    "test:absorption",
                    10.0,
                    "add_value",
                    true,
                )
                .unwrap();
            game.player = Some(player);
            bind_local_player(game, 20, 123);
            let living = game.living(game.entity(20).unwrap()).unwrap();
            assert_eq!(living.absorption(game), Some(0.0));
            let events = game.player.as_ref().unwrap().events.len();
            assert!(living.set_absorption(game, 8.0));
            assert_eq!(living.absorption(game), Some(8.0));
            assert_eq!(game.player.as_ref().unwrap().health.absorption(), 8.0);
            assert!(
                game.player.as_ref().unwrap().events.len() > events,
                "the new amount is queued for the client and the trackers"
            );
            assert!(living.set_absorption(game, 20.0));
            assert_eq!(living.absorption(game), Some(10.0));
            assert!(living.set_absorption(game, -1.0));
            assert_eq!(living.absorption(game), Some(0.0));
            assert!(living.set_absorption(game, 6.0));
            assert!(!living.set_absorption(game, f32::NAN));
            assert!(!living.set_absorption(game, f32::INFINITY));
            assert_eq!(living.absorption(game), Some(6.0));
            assert_eq!(game.player.as_ref().unwrap().health.absorption(), 6.0);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_mobs_absorption_takes_a_hit_before_its_health() {
        with_resident_game(|game| {
            let zombie = absorbing_zombie(game);
            game.mobs.push(zombie);
            let living = game.living(game.entity(12).unwrap()).unwrap();
            let full = living.health(game).unwrap();
            assert!(living.set_absorption(game, 9.0));
            assert_eq!(living.absorption(game), Some(4.0));
            assert!(living.set_absorption(game, -2.0));
            assert_eq!(living.absorption(game), Some(0.0));
            assert!(!living.set_absorption(game, f32::NAN));
            assert_eq!(living.absorption(game), Some(0.0));
            assert!(living.set_absorption(game, 4.0));
            // Magic damage skips armor: the 3 hearts of the hit come out of the 4.
            let victim = game.entity(12).unwrap();
            assert_eq!(
                victim.hurt(game, "minecraft:magic", 3.0, None, None),
                Some(true)
            );
            assert_eq!(living.absorption(game), Some(1.0));
            assert_eq!(living.health(game), Some(full));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_mobs_set_absorption_survives_a_save_and_reload() {
        with_resident_game(|game| {
            let zombie = absorbing_zombie(game);
            game.mobs.push(zombie);
            let living = game.living(game.entity(12).unwrap()).unwrap();
            assert!(living.set_absorption(game, 3.0));
            let saved = sunrise_sim::entity_nbt::mob_to_nbt(
                &game.mobs[0],
                [1, 0, 0, 1],
                None,
                None,
                |_| None,
                &Default::default(),
            );
            let loaded =
                sunrise_sim::entity_nbt::mob_from_nbt(&saved, game.res.mob_types(), 13, None)
                    .unwrap();
            assert_eq!(loaded.absorption, 3.0);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn an_aborted_transaction_restores_the_absorption_of_a_player_and_a_mob() {
        with_resident_game(|game| {
            let mut player = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            player
                .set_modifier(
                    "minecraft:max_absorption",
                    "test:absorption",
                    10.0,
                    "add_value",
                    true,
                )
                .unwrap();
            game.player = Some(player);
            bind_local_player(game, 20, 123);
            let zombie = absorbing_zombie(game);
            game.mobs.push(zombie);
            game.mobs[0].needs_sync = false;
            let player = game.living(game.entity(20).unwrap()).unwrap();
            let mob = game.living(game.entity(12).unwrap()).unwrap();
            assert!(player.set_absorption(game, 5.0));
            assert!(mob.set_absorption(game, 2.0));
            let events = game.player.as_ref().unwrap().events.len();
            game.mobs[0].needs_sync = false;
            game.storage_scope(|game, _| {
                assert!(player.set_absorption(game, 9.0));
                assert!(mob.set_absorption(game, 4.0));
                ((), false)
            });
            assert_eq!(player.absorption(game), Some(5.0));
            assert_eq!(mob.absorption(game), Some(2.0));
            assert_eq!(game.player.as_ref().unwrap().events.len(), events);
            assert!(!game.mobs[0].needs_sync);
            game.storage_scope(|game, _| {
                assert!(player.set_absorption(game, 9.0));
                assert!(mob.set_absorption(game, 4.0));
                ((), true)
            });
            assert_eq!(player.absorption(game), Some(9.0));
            assert_eq!(mob.absorption(game), Some(4.0));
            assert!(game.mobs[0].needs_sync);
        });
    }
}
