#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {
    use sunrise_level::containers::BlockEntity;

    /// A single-player jigsaw edit is applied by the game that is its own
    /// server, as `handleSetJigsawBlock` does there, and marks the chunk.
    #[test]
    #[ignore = "daily"]
    fn a_local_jigsaw_edit_applies_and_marks_the_chunk() {
        let mut resources = None;
        let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
        let pos = (2, 4, 3);
        let state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:jigsaw")
            .unwrap();
        game.world.set(pos.0, pos.1, pos.2, state).unwrap();
        game.world
            .block_entities
            .insert(pos, BlockEntity::JigsawBlock(Box::default()));
        game.world.modified.clear();
        let edit = sunrise_protocol::packets::play::SetJigsawBlock {
            pos: [2, 4, 3],
            name: "minecraft:test".into(),
            target: "minecraft:test".into(),
            pool: "minecraft:empty".into(),
            final_state: "minecraft:air".into(),
            joint: "aligned".into(),
            selection_priority: 1,
            placement_priority: 2,
        };

        game.apply_set_jigsaw_block(&edit);

        let Some(BlockEntity::JigsawBlock(b)) = game.world.block_entities.get(&pos) else {
            panic!("no jigsaw");
        };
        assert_eq!(&*b.name, "minecraft:test", "the edit landed");
        assert_eq!((b.selection_priority, b.placement_priority), (1, 2));
        assert!(
            game.world.modified.contains(&(0, 0)),
            "the edit left the chunk unmarked"
        );
    }
}
