#![cfg(test)]
use sunrise_items::items::ItemStack;
use sunrise_sim::mobs::Species;
#[cfg(test)]
pub(crate) mod tests {

    use super::*;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;

    /// `PiglinAi.throwItemsTowardPos` swings the off hand once when it has
    /// something to throw, so barters leave the admiring hand (#12512).
    #[test]
    #[ignore = "daily"]
    fn a_throw_swings_the_off_hand_once() {
        let mut resources = None;
        let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
        game.build_registries();
        game.set_allow_commands(true);
        assert!(game.run_command("summon piglin 8.5 80 8.5 {NoAI:1b,PersistenceRequired:1b}"));
        let i = game
            .mobs
            .iter()
            .position(|m| m.species() == Species::Piglin)
            .expect("the summoned piglin");
        // An empty list has nothing to throw and leaves the body alone.
        game.piglin_throw_items(i, Vec::new());
        assert!(!game.mobs[i].ai.swinging);
        game.piglin_throw_items(i, vec![ItemStack::new("minecraft:gold_ingot", 1)]);
        let m = &game.mobs[i];
        assert!(m.ai.swinging, "the throw starts a swing");
        // The off hand's arm: the right one only when the piglin is left-handed.
        assert_eq!(m.extra.swing_right_arm, Some(m.main_arm_left()));
        assert_eq!(m.swing_duration(), 6, "SwingAnimation.DEFAULT's ticks");
    }
}
