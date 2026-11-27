//! Client tests for copy-on-write `ChunkSnapshot` capture on a logged-in client world.
use crate::game::native_entity_refs::tests::with_client_game;
use std::sync::Arc;
use sunrise_gameplay_common::game::api_values;
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
use sunrise_level::chunk::Chunk;

#[test]
#[ignore = "daily"]
fn chunk_snapshot_works_on_client_world() {
    with_client_game(|game| {
        let (client, _server) = sunrise_protocol::connection::Pipe::pair();
        game.connect_local(client, "snapshot-client").unwrap();
        game.client.remote.as_mut().unwrap().logged_in = true;

        let stone_key = mod_api::__private::block_key("minecraft:stone").unwrap();
        let stone_id = api_values::default_state(stone_key).unwrap().0;
        let mut column = Chunk::empty(1, 1);
        column.lit = true;
        Arc::make_mut(&mut column.sections[4]).set(0, stone_id);
        game.common.world.chunks.insert((1, 1), Arc::new(column));

        let level = game.level("minecraft:overworld").unwrap();
        let chunk = game
            .chunk(level, [1, 1])
            .expect("client host.chunk resolves loaded chunk");
        let snap = chunk
            .snapshot(game)
            .expect("client ChunkRef::snapshot works");
        assert_eq!(snap.block_state(0, 0, 0).unwrap().block(), stone_key);
        assert!(snap.has_light());
        // Server-only ChunkRef status/loaded checks still refuse client side.
        assert!(!chunk.loaded(game));
        assert_eq!(chunk.status(game), None);
    });
}
