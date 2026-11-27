#![cfg(test)]
#[cfg(test)]
mod biome_tint_tests {

    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use mod_api::{
        BiomeColorsEvent, ClientHost, ColorResolver, Hooks, ModEntry, prelude::BlockStatesHost,
    };
    use std::sync::Arc;
    use sunrise_level::chunk::Chunk;

    const GRASS: u32 = 0xAD_248B;
    const FOLIAGE: u32 = 0x34_D1EF;
    const WATER: u32 = 0xDA_5227;
    const POS: [i32; 3] = [8, 64, 8]; // The whole blend square samples this plains chunk.

    fn with_stock_client_biomes(
        test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>),
    ) {
        struct Seasons;
        #[mod_api::__subscriptions]
        impl Hooks for Seasons {
            fn biome_colors(&self, _: &dyn ClientHost, event: &mut BiomeColorsEvent<'_>) {
                if event.biome == "minecraft:plains" {
                    event.colors.grass = GRASS;
                    event.colors.foliage = FOLIAGE;
                    event.colors.water = WATER;
                }
            }
        }

        let mods = vec![ModEntry::new("test_seasons", Box::new(Seasons))].into();
        mod_platform::testing::with_mods(mods, || {
            with_client_game(|game| {
                // The replay fixture opens a client level without connecting to a server.
                game.init_remote_for_replay().unwrap();
                game.native_lifecycle.begin_open(None);

                let plains_id = game.res.biomes().id("minecraft:plains");
                let mut chunk = Chunk::empty(0, 0);
                for qy in 0..chunk.sections.len() as i32 * 4 {
                    for qz in 0..4 {
                        for qx in 0..4 {
                            chunk.set_biome_cell(qx, qy, qz, plains_id);
                        }
                    }
                }
                game.world.chunks.insert((0, 0), Arc::new(chunk));
                game.client.streaming.biome_blend = 1;

                // Stock server biome definitions do not install a world-specific table.
                assert!(game.world_biomes.is_none());
                test(game);
                assert!(game.world_biomes.is_none());
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn biome_tint_reads_refreshed_colours_with_stock_server_biomes() {
        with_stock_client_biomes(|game| {
            let host: &mut dyn ClientHost = game;
            assert_ne!(
                host.biome_tint(POS, ColorResolver::Grass),
                Ok(0xFF00_0000 | GRASS)
            );
            host.refresh_biome_colors();
            assert_eq!(
                host.biome_tint(POS, ColorResolver::Grass),
                Ok(0xFF00_0000 | GRASS)
            );
            assert_eq!(
                host.biome_tint(POS, ColorResolver::Foliage),
                Ok(0xFF00_0000 | FOLIAGE)
            );

            let worker = game.worker_context();
            let plains = worker
                .chunk
                .biomes
                .get(worker.chunk.biomes.id("minecraft:plains"));
            assert_eq!((plains.grass, plains.foliage), (GRASS, FOLIAGE));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn face_tints_read_refreshed_colours_with_stock_server_biomes() {
        with_stock_client_biomes(|game| {
            let grass = BlockStatesHost::block_state_id(game, "minecraft:grass_block").unwrap();
            let water = BlockStatesHost::block_state_id(game, "minecraft:water").unwrap();
            let host: &mut dyn ClientHost = game;
            assert_ne!(host.block_tint_color(grass, 0, POS).unwrap(), GRASS);
            assert_ne!(host.block_tint_color(water, 0, POS).unwrap(), WATER);

            host.refresh_biome_colors();

            assert_eq!(host.block_tint_color(grass, 0, POS), Some(GRASS));
            assert_eq!(host.block_tint_color(water, 0, POS), Some(WATER));
        });
    }
}
