use std::sync::Arc;
use sunrise_client::level_loading::Reason;
use sunrise_level::world::World;
use sunrise_protocol::{
    buf::set_bit,
    chunk::{ChunkData, LightData, Paletted, Section, Strategy},
    connection::Pipe,
    packets::play::{
        Clientbound as C, Distance, ForgetLevelChunk, LevelChunkWithLight, LightUpdate, SpawnInfo,
    },
};
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry,
    sunrise_gameplay_common::game::in_own_process,
};

fn data(sky: &[(usize, u8)], block: &[(usize, u8)], empty_sky: &[usize]) -> LightData {
    let mask = |bits: &[usize]| {
        let mut mask = Vec::new();
        for &i in bits {
            set_bit(&mut mask, i);
        }
        mask
    };
    let layers = |layers: &[(usize, u8)]| {
        layers
            .iter()
            .map(|&(_, n)| vec![n | n << 4; 2048])
            .collect()
    };
    LightData {
        sky_mask: mask(&sky.iter().map(|&(i, _)| i).collect::<Vec<_>>()),
        block_mask: mask(&block.iter().map(|&(i, _)| i).collect::<Vec<_>>()),
        empty_sky_mask: mask(empty_sky),
        empty_block_mask: vec![],
        sky: layers(sky),
        block: layers(block),
    }
}

fn light(game: &mut sunrise_client_gameplay::game::ClientGame<'_>, x: i32, light: LightData) {
    game.apply_packet(C::LightUpdate(LightUpdate { x, z: 0, light }))
        .unwrap();
}

fn column(game: &mut sunrise_client_gameplay::game::ClientGame<'_>, x: i32, light: LightData) {
    let air = game.world.air.0 as i32;
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap()
        .0 as i32;
    let mut sections: Vec<_> = (0..game.world.layout().sections())
        .map(|_| Section {
            states: Paletted::from_values(&[air; 4096], Strategy::Blocks, 16),
            biomes: Paletted::from_values(&[0; 64], Strategy::Biomes, 2),
            non_empty: 0,
            fluids: 0,
        })
        .collect();
    let mut first = [air; 4096];
    first[0] = stone;
    sections[0].non_empty = 1;
    sections[0].states = Paletted::from_values(&first, Strategy::Blocks, 16);
    game.apply_packet(C::LevelChunkWithLight(LevelChunkWithLight {
        x,
        z: 0,
        chunk: ChunkData {
            heightmaps: vec![],
            buffer: ChunkData::from_sections(&sections),
            block_entities: vec![],
        },
        light,
    }))
    .unwrap();
}

#[test]
fn light_packet_padding_rebuilds_only_adjacent_mesh_rows() {
    let mut world = World::new(sunrise_world::registry::BlockStateId(0), 0);
    let count = world.layout().sections() as i32;
    let layers = data(&[], &[(0, 4), (count as usize + 1, 7)], &[]).layers(count as usize + 2);
    // Padding has no mesh of its own. Its neighbour on the terrain side does,
    // including the diagonals, without touching the next terrain row in.
    world.rebuild_packet_light(-2, 4, &layers);
    assert_eq!(world.dirty_sections.len(), 18);
    for x in -3..=-1 {
        for z in 3..=5 {
            assert!(world.dirty_sections.contains(&(x, 0, z)));
            assert!(world.dirty_sections.contains(&(x, count - 1, z)));
        }
    }
}

/// SyntheticClientLight: light packet queue, per-layer chronology, equal resends, and
/// light-only enable/Forget when the block cache rejects a column.
#[test]
#[ignore = "daily"]
fn standalone_light_survives_missing_columns_and_rebuilds_on_equal_resends() {
    in_own_process(
        "cut_layout::game::client_side::remote_chunks::light_tests::standalone_light_survives_missing_columns_and_rebuilds_on_equal_resends",
        || {
            with_client_game(|game| {
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "ChunkLightProbe").unwrap();
                game.enter_remote_dimension(
                    &SpawnInfo {
                        dimension_type: 0,
                        dimension: "minecraft:overworld".into(),
                        seed: 0,
                        game_type: 1,
                        previous_game_type: None,
                        debug: false,
                        flat: false,
                        last_death: None,
                        portal_cooldown: 0,
                        sea_level: 63,
                    },
                    RemoteDimensionEntry::Login,
                    Reason::Other,
                );
                game.apply_packet(C::SetChunkCacheRadius(Distance { distance: 2 }))
                    .unwrap();
                let y = game.world.layout().min_y();
                let count = game.world.layout().sections();
                let top = count + 1;
                let top_y = y + count as i32 * 16;
                let origin = (0, 0);

                light(game, 0, data(&[(1, 8)], &[(0, 2), (1, 4), (top, 6)], &[]));
                assert!(
                    !game
                        .client
                        .remote
                        .as_ref()
                        .unwrap()
                        .chunk_light
                        .columns
                        .contains_key(&origin)
                );
                assert!(game.world.dirty_sections.is_empty());
                game.poll_remote_light();
                assert!(!game.world.loaded(0, 0));
                let layers = game.client.remote.as_ref().unwrap().chunk_light.columns[&origin]
                    .as_ref()
                    .unwrap();
                assert_eq!(layers[1].0.as_ref().unwrap()[0], 0x88);
                assert_eq!(layers[0].1.as_ref().unwrap()[0], 0x22);
                assert_eq!(layers[1].1.as_ref().unwrap()[0], 0x44);
                assert_eq!(layers[top].1.as_ref().unwrap()[0], 0x66);
                assert!(game.world.dirty_sections.contains(&(-1, 0, 1)));
                assert!(
                    game.world
                        .dirty_sections
                        .contains(&(1, count as i32 - 1, -1))
                );

                game.world.dirty_sections.clear();
                light(game, 0, data(&[], &[(1, 9)], &[]));
                light(game, 0, data(&[], &[], &[1]));
                column(game, 0, data(&[], &[(0, 3)], &[]));
                assert!(game.world.loaded(0, 0));
                assert_eq!(game.world.dirty_sections.len(), count);
                game.poll_remote_light();
                assert_eq!(game.world.light(1, y + 1, 1), 0x09);
                assert_eq!(game.world.light(0, y - 1, 0) & 15, 3);
                assert_eq!(game.world.light(0, top_y, 0) & 15, 6);
                assert_eq!(
                    game.world.chunks[&origin].stored_light_layers(1),
                    Some([true, true])
                );
                assert!(
                    game.client.remote.as_ref().unwrap().chunk_light.columns[&origin].is_none()
                );

                let first = game.world.chunks[&origin].sections[0].clone();
                for _ in 0..2 {
                    game.world.dirty_sections.clear();
                    light(game, 0, data(&[], &[(0, 3), (1, 9), (top, 6)], &[1]));
                    assert!(game.world.dirty_sections.is_empty());
                    game.poll_remote_light();
                    assert_eq!(game.world.dirty_sections.len(), 27);
                    assert!(game.world.dirty_sections.contains(&(-1, 1, -1)));
                    assert!(
                        game.world
                            .dirty_sections
                            .contains(&(1, count as i32 - 1, 1))
                    );
                    assert!(Arc::ptr_eq(&first, &game.world.chunks[&origin].sections[0]));
                }
                // An explicitly empty layer with no parallel named mask still requests a rebuild.
                game.world.dirty_sections.clear();
                light(game, 0, data(&[], &[], &[1]));
                game.poll_remote_light();
                assert_eq!(game.world.dirty_sections.len(), 18);
                assert!(Arc::ptr_eq(&first, &game.world.chunks[&origin].sections[0]));

                column(game, 0, data(&[], &[], &[]));
                game.poll_remote_light();
                assert_eq!(game.world.light(1, y + 1, 1), 0x09);
                assert_eq!(game.world.light(0, y - 1, 0) & 15, 3);
                assert_eq!(game.world.light(0, top_y, 0) & 15, 6);

                let far = (50, 0);
                light(game, far.0, data(&[], &[], &[]));
                game.poll_remote_light();
                assert!(game.client.remote.as_ref().unwrap().chunk_light.columns[&far].is_none());
                column(game, far.0, data(&[], &[(1, 7)], &[]));
                game.poll_remote_light();
                assert!(!game.world.loaded(far.0 * 16, 0));
                let layers = game.client.remote.as_ref().unwrap().chunk_light.columns[&far]
                    .as_ref()
                    .unwrap();
                assert_eq!(layers[1].1.as_ref().unwrap()[0], 0x77);
                game.apply_packet(C::ForgetLevelChunk(ForgetLevelChunk {
                    pos: [far.0, far.1],
                }))
                .unwrap();
                assert!(
                    game.client
                        .remote
                        .as_ref()
                        .unwrap()
                        .chunk_light
                        .columns
                        .contains_key(&far)
                );
                game.poll_remote_light();
                assert!(
                    !game
                        .client
                        .remote
                        .as_ref()
                        .unwrap()
                        .chunk_light
                        .columns
                        .contains_key(&far)
                );
            })
        },
    );
}
