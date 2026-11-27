use crate::game::server_side::native_entity_refs::tests::with_client_game;
use sunrise_level::{chunk::Chunk, containers::BlockEntity};

#[test]
#[ignore = "daily"]
fn forgetting_and_closing_remote_columns_clear_their_data() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::remote::tests::remote_columns_tests::forgetting_and_closing_remote_columns_clear_their_data",
        || {
            with_client_game(|game| {
                for x in 0..2 {
                    game.client
                        .streaming
                        .insert_remote(&mut game.common.world, Chunk::empty(x, 0));
                    game.world
                        .block_entities
                        .insert((x * 16, 64, 0), BlockEntity::Chest(vec![]));
                }
                // A same-position resend replaces the data without removing the column.
                game.client
                    .streaming
                    .replace_remote(&mut game.common.world, Chunk::empty(0, 0));
                assert_eq!(game.world.chunks.len(), 2);
                assert!(game.world.chunks.contains_key(&(0, 0)));
                assert!(game.client.streaming.is_lit(0, 0));
                let mut forgotten = [(0, 0)].into_iter().collect();
                game.forget_remote_columns(&mut forgotten, true);
                assert!(!game.world.chunks.contains_key(&(0, 0)));
                assert!(!game.world.block_entities.contains_key(&(0, 64, 0)));
                assert!(!game.client.streaming.is_lit(0, 0));
                assert!(game.world.chunks.contains_key(&(1, 0)));
                game.forget_remote_columns(&mut [(0, 0)].into_iter().collect(), true);
                game.forget_all_remote_columns();
                assert!(game.world.chunks.is_empty());
                assert!(game.world.block_entities.is_empty());
            })
        },
    );
}

/// An explicit drop returns only the column it removed, and closing the level
/// frees the remaining columns.
#[test]
#[ignore = "daily"]
fn every_free_route_returns_the_columns_it_freed_once() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::remote::tests::remote_columns_tests::every_free_route_returns_the_columns_it_freed_once",
        || {
            with_client_game(|game| {
                for x in [0, 1] {
                    game.client
                        .streaming
                        .insert_remote(&mut game.common.world, Chunk::empty(x, 0));
                }
                let mut gone = [(0, 0)].into_iter().collect();
                assert_eq!(game.forget_remote_columns(&mut gone, true), [(0, 0)]);
                let mut gone = [(0, 0)].into_iter().collect();
                assert!(game.forget_remote_columns(&mut gone, true).is_empty());
                assert_eq!(game.forget_all_remote_columns(), [(1, 0)]);
                assert!(game.world.chunks.is_empty());
                assert!(game.world.block_entities.is_empty());
            })
        },
    );
}

/// The seam sits at the packet level: streaming adopting, replacing or
/// re-sending a column keeps it resident — a re-sent position loads
/// again (`remote_chunk`), never an unload and a load — so a forget
/// returns exactly it.
#[test]
#[ignore = "daily"]
fn streaming_adoption_and_resend_stay_resident_until_the_forget() {
    with_client_game(|game| {
        game.client
            .streaming
            .insert_remote(&mut game.common.world, Chunk::empty(0, 0));
        // A re-send of a resident, lit column takes replace_remote.
        game.client
            .streaming
            .replace_remote(&mut game.common.world, Chunk::empty(0, 0));
        assert!(game.world.chunks.contains_key(&(0, 0)));
        let mut gone = [(0, 0)].into_iter().collect();
        assert_eq!(game.forget_remote_columns(&mut gone, true), [(0, 0)]);
    });
}

#[test]
#[ignore = "daily"]
fn a_forget_packet_keeps_published_light_beside_a_loaded_column() {
    use sunrise_protocol::packets::play::{Clientbound, ForgetLevelChunk};
    with_client_game(|game| {
        use sunrise_level::chunk::SECTIONS;
        for x in 0..2 {
            let mut chunk = Chunk::empty(x, 0);
            chunk.set(0, 0, 0, 1);
            let s = chunk.section_index(0).unwrap();
            chunk.add_light_support(s);
            let mut layers = vec![(None, None); SECTIONS + 2];
            layers[s + 1] = (Some(vec![0x66; 2048]), Some(vec![0x99; 2048]));
            sunrise_level::light::light_from_layers(&mut chunk, &layers, true);
            game.client
                .streaming
                .insert_remote(&mut game.common.world, chunk);
        }
        game.apply_packet(Clientbound::ForgetLevelChunk(ForgetLevelChunk {
            pos: [1, 0],
        }))
        .unwrap();
        assert!(!game.world.loaded(16, 0));
        assert_eq!(game.world.light(16, 0, 0), 0x69);
        game.apply_packet(Clientbound::ForgetLevelChunk(ForgetLevelChunk {
            pos: [0, 0],
        }))
        .unwrap();
        assert_eq!(game.world.light(16, 0, 0), 0xF0);
    });
}
