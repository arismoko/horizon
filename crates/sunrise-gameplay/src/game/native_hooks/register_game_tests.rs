use mod_api::GameTestFunctions;

impl crate::game::ServerGame<'_> {
    /// Register functions before resolving the world's test definitions.
    pub fn native_register_game_tests(&self, functions: &mut GameTestFunctions) {
        self.fire_register_game_tests(|listen| functions.register_listener(listen));
    }
}
