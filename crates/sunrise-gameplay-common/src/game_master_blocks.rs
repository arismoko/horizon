//! Blocks whose class implements `GameMasterBlock`.

/// Blocks whose class implements `GameMasterBlock`: the command blocks, structure
/// block, jigsaw and the test blocks.
pub fn is_game_master_block(name: &str) -> bool {
    sunrise_level::command_block::Mode::of_block(name).is_some()
        || matches!(
            name,
            "minecraft:structure_block"
                | "minecraft:jigsaw"
                | "minecraft:test_block"
                | "minecraft:test_instance_block"
        )
}
