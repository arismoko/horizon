//! Block parts answer the enchanting power an enchanting table counts (`enchant_power_bonus`)
//! and the colour a beacon beam takes (`beacon_color_multiplier`).
use crate::game::{Game, native_entity_refs::tests::with_game};
use horizon_client::chunk::Chunk;
use std::sync::Arc;

const TABLE: [i32; 3] = [8, 64, 8];

fn place(game: &mut Game<'_>, offset: [i32; 3], block: &str) {
    let state = game.pack.assets.registry.default_state(block).unwrap();
    let [x, y, z] = [0, 1, 2].map(|i| TABLE[i] + offset[i]);
    game.world.set(x, y, z, state).unwrap();
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn block_parts_answer_the_enchanting_power_and_the_beam_colour() {
    // The fixture's parts: glass is worth two bookshelves and tints a beam red, and a
    // bookshelf is worth nothing.
    mod_platform::__private::part_tests::enchant_power_and_beam_game_thread();
    with_game(|game| {
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        // A bookshelf whose part answers 0 is not counted.
        place(game, [-2, 0, -2], "minecraft:bookshelf");
        assert_eq!(game.count_bookshelves(TABLE), 0);
        // Glass answering 2 counts as two bookshelves.
        place(game, [-2, 0, -1], "minecraft:glass");
        assert_eq!(game.count_bookshelves(TABLE), 2);
        // Unless the block between it and the table does not transmit.
        place(game, [-1, 0, 0], "minecraft:stone");
        assert_eq!(game.count_bookshelves(TABLE), 0);

        let registry = game.pack.assets.registry;
        let color = |game: &Game<'_>, block: &str| {
            let state = registry.default_state(block).unwrap();
            game.beacon_color(state, block, [8, 65, 8], [8, 64, 8])
        };
        // Glass gives no colour of its own; its part tints the beam red, alpha dropped.
        assert_eq!(color(game, "minecraft:glass"), Some(0xFF_0000));
        // Stained glass without a part keeps its dye colour, and stone none.
        assert_eq!(color(game, "minecraft:red_stained_glass"), Some(0xB0_2E26));
        assert_eq!(color(game, "minecraft:stone"), None);
    });
}
