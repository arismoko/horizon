//! `BlockStates::requires_correct_tool` through the game's block facts:
//! `BlockState.requiresCorrectToolForDrops()` for vanilla's blocks.

use {
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
    sunrise_gameplay_common::game::native_block_tables::BlockTables,
};

/// Stone, ores and obsidian need a correct tool; dirt, logs and air don't.
#[test]
#[ignore = "daily"]
fn requires_correct_tool_is_the_states_own_fact() {
    with_game(|game| {
        let registry = &game.pack.assets.registry;
        let state = |id: &str| registry.default_state(id).unwrap();
        for (id, requires) in [
            ("minecraft:stone", true),
            ("minecraft:iron_ore", true),
            ("minecraft:obsidian", true),
            ("minecraft:dirt", false),
            ("minecraft:oak_log", false),
            ("minecraft:air", false),
        ] {
            assert_eq!(
                BlockTables::of(game).block_state_requires_correct_tool(state(id)),
                Some(requires),
                "{id}"
            );
        }
    });
}
