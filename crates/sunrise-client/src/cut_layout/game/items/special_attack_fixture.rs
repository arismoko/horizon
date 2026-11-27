#![cfg(test)]

#[cfg(test)]
pub(crate) mod tests {
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    /// Invoke the actual attack methods in an empty scratch Game, without
    /// starting a server, opening a world or entering the game loop.
    #[test]
    #[ignore = "needs prepared resources and SUNRISE_SPECIAL_ATTACK_CASES=INPUT,OUTPUT"]
    fn special_attack_cases_without_world_startup() {
        run_special_attack_cases_without_world_startup();
    }

    pub(crate) fn run_special_attack_cases_without_world_startup() {
        let spec = std::env::var("SUNRISE_SPECIAL_ATTACK_CASES").expect("case paths required");
        let mut resources = None;
        let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
        game.build_registries();
        game.dispense_fixture(&spec).unwrap();
    }
}
