#![cfg(test)]
use sunrise_sim::mobs::Who;
#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_gameplay::game::game_tests::tests::server_game;

    /// `AbstractThrownPotion.douseFire` -> `CampfireBlock.douse(entity, ...)`:
    /// the campfire's block change is sourced from the potion's owner.
    #[test]
    #[ignore = "daily"]
    fn a_doused_campfire_posts_block_change_from_the_potion_owner() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let pos = [2, 4, 3];
        let campfire = game
            .pack
            .assets
            .registry
            .default_state("minecraft:campfire")
            .unwrap();
        game.world.set(pos[0], pos[1], pos[2], campfire).unwrap();
        // The lit campfire the douse looks for.
        let lit = game.lit_campfire(campfire, "minecraft:campfire");
        assert!(lit.is_some(), "the campfire starts lit");
        sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
            .with(|e| *e.borrow_mut() = Some(Vec::new()));

        game.douse_fire(pos, Some(Who::Mob(7)));

        let after = game.world.block(pos[0], pos[1], pos[2]);
        assert!(
            !game
                .pack
                .assets
                .registry
                .state(after)
                .and_then(|s| s.properties.get("lit"))
                .is_some_and(|v| v == "true"),
            "the campfire is out"
        );
        sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS.with(|e| {
            assert!(
                e.borrow().as_ref().unwrap().contains(&(
                    "minecraft:block_change",
                    [2.5, 4.5, 3.5],
                    Some(Who::Mob(7))
                )),
                "{:?}",
                e.borrow()
            );
        });
    }
}
