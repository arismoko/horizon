#![cfg(test)]
#[cfg(test)]
#[cfg(feature = "client")]
pub(crate) mod tests {

    /// `LightningRodBlock.onLightningStrike` updates the neighbours of the block
    /// in front of the rod whether or not redstone experiments are on (#14371):
    /// a wire beside that block is strongly powered by the struck rod.
    #[test]
    #[ignore = "daily"]
    fn a_struck_rod_updates_the_wire_beside_the_block_behind_it() {
        let mut resources = None;
        let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let wire = registry.default_state("minecraft:redstone_wire").unwrap();
        let mut rod = registry
            .state(registry.default_state("minecraft:lightning_rod").unwrap())
            .cloned()
            .unwrap();
        rod.properties.insert("facing".into(), "up".into());
        let rod = registry.id(&rod).unwrap();
        // The rod on stone at (8, 64, 8); wire on stone beside that block.
        game.world.set(8, 64, 8, stone).unwrap();
        game.world.set(9, 63, 8, stone).unwrap();
        game.world.set(9, 64, 8, wire).unwrap();
        game.world.set(8, 65, 8, rod).unwrap();
        assert!(
            !game
                .level
                .enabled_features
                .iter()
                .any(|feature| feature == "minecraft:redstone_experiments")
        );
        game.strike_rod([8.5, 66.0, 8.5]);
        let power = registry
            .state(game.world.block(9, 64, 8))
            .unwrap()
            .properties
            .get("power")
            .cloned();
        assert_eq!(power.as_deref(), Some("15"));
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
#[path = "tests/mob_conversion_state_tests.rs"]
pub(crate) mod mob_conversion_state_tests;
