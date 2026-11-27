#![cfg(test)]

#[cfg(test)]
mod tests {
    use mod_api::client::first_person::Hand;
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay::game::native_entity_refs::tests::with_resident_game;
    use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_living_blocking::NativeLivingBlockingGameExt1;
    use sunrise_items::items::ItemStack;
    use sunrise_sim::mobs::Mob;

    /// A raised shield blocks after vanilla's 5-tick delay (`block_delay_seconds` 0.25), in
    /// either hand, for the player as the damage path sees it and for a mob; a bow in use, an
    /// unraised shield and a lowered one don't.
    #[test]
    #[ignore = "daily"]
    fn a_raised_shield_blocks_after_its_delay_for_a_player_and_a_mob() {
        use sunrise_items::equipment::OFFHAND;
        with_resident_game(|game| {
            sunrise_gameplay::game::native_entity_refs::tests::resolve_parts(game).unwrap();
            game.player =
                Some(sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            game.server_set_acting_use(true);
            let player = game.living(game.entity(20).unwrap()).unwrap();
            let shield = ItemStack::new("minecraft:shield", 1);
            let delay = game.blocks_attacks(&shield).unwrap().delay_seconds;
            assert_eq!(delay, 0.25, "vanilla's shield");

            for hand in [Hand::Main, Hand::Off] {
                game.selected = 0;
                (game.inventory[0], game.offhand) = match hand {
                    Hand::Main => (Some(shield.clone()), None),
                    Hand::Off => (None, Some(shield.clone())),
                };
                assert_eq!(game.living_is_blocking(player), Some(false), "not raised");
                assert!(game.living_start_using_item(player, hand));
                for tick in 0..5 {
                    assert_eq!(
                        game.living_is_blocking(player),
                        Some(false),
                        "{hand:?} {tick}"
                    );
                    game.tick_using_server();
                }
                assert_eq!(game.living_is_blocking(player), Some(true), "{hand:?}");
                let hit = game.player_blocking(None, "minecraft:mob_attack", false);
                assert!(hit.is_some(), "the damage path blocks too");
                assert!(game.living_release_using_item(player));
                assert_eq!(game.living_is_blocking(player), Some(false), "lowered");
            }
            game.offhand = Some(ItemStack::new("minecraft:bow", 1));
            assert!(game.living_start_using_item(player, Hand::Off));
            for _ in 0..10 {
                game.tick_using_server();
            }
            assert_eq!(
                game.living_is_blocking(player),
                Some(false),
                "a bow blocks nothing"
            );

            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(12, kind, Vec3::ZERO, 0.0));
            let zombie = game.living(game.entity(12).unwrap()).unwrap();
            let mob = game.mobs.len() - 1;
            game.mobs[mob].extra.set_slot_stack(OFFHAND, Some(shield));
            assert_eq!(game.living_is_blocking(zombie), Some(false));
            assert!(game.living_start_using_item(zombie, Hand::Off));
            game.mobs[mob].extra.using_item = 5;
            assert_eq!(
                game.living_is_blocking(zombie),
                Some(false),
                "4 ticks of use"
            );
            game.mobs[mob].extra.using_item = 6;
            assert_eq!(
                game.living_is_blocking(zombie),
                Some(true),
                "5 ticks of use"
            );
            assert!(game.living_release_using_item(zombie));
            assert_eq!(game.living_is_blocking(zombie), Some(false));
        });
    }
}
