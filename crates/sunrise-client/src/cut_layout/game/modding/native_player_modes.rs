#![cfg(test)]

#[cfg(test)]
mod tests {

    use super::super::native_guest_turn_tests::with_guests;
    use mod_api::__private::game::*;
    use mod_api::BlockFace;
    use std::sync::Arc;
    use sunrise_core::nbt::Tag;
    use sunrise_gameplay_common::game::api_values::test_snapshot;
    use sunrise_gameplay_common::game::native_server::NativeServerGameExt1;
    use sunrise_items::items::ItemStack;
    use sunrise_level::chunk::Chunk;

    fn set_mode(game: &mut sunrise_gameplay::game::ServerGame<'_>, uuid: u128, mode: i32) {
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_all(|g| {
                if g.uuid == uuid {
                    g.game_mode = mode;
                    g.abilities.update_for_mode(mode);
                }
            });
    }

    /// A stack that, in adventure mode, places on `block` only.
    fn placing_on(block: &str) -> ItemStack {
        let mut stack = ItemStack::new("minecraft:stone", 1);
        stack.data_mut().set_other(
            "minecraft:can_place_on",
            Some(Tag::compound([("blocks", Tag::String(block.into()))])),
        );
        stack
    }

    /// `Player.mayUseItemAt`: builders always may, adventure and spectator
    /// players only where the stack's `can_place_on` matches the block
    /// clicked, which lies opposite the face from the placed position.
    #[test]
    #[ignore = "daily"]
    fn item_use_follows_the_mode_the_stack_and_the_clicked_block() {
        with_guests(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(5, 63, 5, stone).unwrap();
            set_mode(game, 13, 2);
            let (builder, adventurer) = (game.player(12).unwrap(), game.player(13).unwrap());
            let plain = test_snapshot(&ItemStack::new("minecraft:stone", 1));
            let on_stone = test_snapshot(&placing_on("minecraft:stone"));
            let on_dirt = test_snapshot(&placing_on("minecraft:dirt"));
            let up = |player: mod_api::PlayerRef,
                      game: &sunrise_gameplay::game::ServerGame<'_>,
                      stack| {
                player.may_use_item_at(game, [5, 64, 5], BlockFace::Up, stack)
            };

            // Survival builds with anything.
            assert_eq!(up(builder, game, &plain), Some(true));
            // Adventure needs the component, naming the block clicked (the stone below).
            assert_eq!(up(adventurer, game, &plain), Some(false));
            assert_eq!(up(adventurer, game, &on_dirt), Some(false));
            assert_eq!(up(adventurer, game, &on_stone), Some(true));
            // The same placed position clicked on a side face lies against air.
            assert_eq!(
                adventurer.may_use_item_at(game, [5, 64, 5], BlockFace::North, &on_stone),
                Some(false)
            );
            // The acting player in their own turn answers alike.
            let acting = game
                .as_guest(13, |game| up(adventurer, game, &on_stone))
                .unwrap();
            assert_eq!(acting, Some(true));
            // A spectator may not build either, so the same rule applies.
            set_mode(game, 13, 3);
            assert_eq!(up(adventurer, game, &plain), Some(false));
            assert_eq!(up(adventurer, game, &on_stone), Some(true));
        });
    }

    /// `ServerPlayer.canHarmPlayer`: the `pvp` rule first, then the teams
    /// (allies hurt each other only with friendly fire on).
    #[test]
    #[ignore = "daily"]
    fn one_player_may_harm_another_by_the_pvp_rule_and_their_teams() {
        use sunrise_game_data::gamerules::Value;
        with_guests(|game| {
            let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
            assert_eq!(a.can_harm(&*game, b), Some(true), "no teams, pvp on");

            let board = &mut game.score.board;
            board.add_team("red").allow_friendly_fire = false;
            board.add_to_team("guest12", "red");
            board.add_to_team("guest13", "red");
            assert_eq!(
                a.can_harm(&*game, b),
                Some(false),
                "allies, no friendly fire"
            );
            assert_eq!(b.can_harm(&*game, a), Some(false));

            game.score.board.add_team("red").allow_friendly_fire = true;
            assert_eq!(
                a.can_harm(&*game, b),
                Some(true),
                "allies with friendly fire"
            );

            game.ui.rules.set("pvp", Value::Bool(false));
            assert_eq!(
                a.can_harm(&*game, b),
                Some(false),
                "pvp off beats the teams"
            );

            // During the attacker's own turn the same answer comes from their state in place.
            let acting = game.as_guest(12, |game| a.can_harm(&*game, b)).unwrap();
            assert_eq!(acting, Some(false));
            game.ui.rules.set("pvp", Value::Bool(true));
            let acting = game.as_guest(12, |game| a.can_harm(&*game, b)).unwrap();
            assert_eq!(acting, Some(true));

            // A handle that names nobody here reads nothing.
            let stale = mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(a.fields().entity.epoch(), a.fields().entity.id() + 100),
                a.uuid(),
            );
            assert_eq!(stale.can_harm(&*game, b), None);
            assert_eq!(a.can_harm(&*game, stale), None);
        });
    }
}
