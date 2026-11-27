#![cfg(test)]
#[cfg(test)]
mod tests {

    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay_common::game::minecarts::{CartKind, Minecart};

    #[test]
    #[ignore = "daily"]
    fn scoped_cart_commands_keep_tick_and_self_removal_rules() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.world.game_time = 10;
            game.carts
                .push(Minecart::new(41, CartKind::CommandBlock, Vec3::ZERO));
            game.carts
                .push(Minecart::new(42, CartKind::Rideable, Vec3::ZERO));
            game.carts.update(0, |c| c.command.set_command("Searge"));
            assert!(game.perform_cart_command(41));
            assert_eq!(game.carts[0].command.success_count, 1);
            game.carts.update(0, |c| c.command.set_command(""));
            assert!(game.perform_cart_command(41));
            assert_eq!(game.carts[0].command.success_count, 0);
            assert_eq!(game.carts[0].command.last_execution, 10);
            assert!(!game.perform_cart_command(41));
            assert!(!game.perform_cart_command(42));
            assert!(!game.perform_cart_command(99));
            game.world.game_time = 11;
            game.carts.update(0, |c| c.command.set_command("kill @s"));
            assert!(game.perform_cart_command(41));
            assert_eq!(game.carts.len(), 1);
            assert_eq!(game.carts[0].id, 42);
            assert_eq!(game.carts[0].command.success_count, 0);
        });
    }
}
