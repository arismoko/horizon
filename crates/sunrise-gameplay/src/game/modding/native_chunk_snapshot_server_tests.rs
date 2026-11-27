//! Server tests for copy-on-write `ChunkSnapshot` capture, off-thread reads, and `LevelShape` clipping.
use crate::game::native_entity_refs::tests::with_game;
use mod_api::{Authority, ChunkRef, DimensionKey, Heightmap, LevelRef};
use std::sync::Arc;
use sunrise_gameplay_common::game::api_values;
use sunrise_gameplay_common::game::native_chunk_snapshot::EngineChunkSnapshot;
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
use sunrise_gameplay_common::world_biomes;
use sunrise_level::chunk::{Chunk, LevelShape, Light};

#[test]
#[ignore = "daily"]
fn chunk_snapshot_shares_cow_sections_and_isolates_blocks_biomes_and_light() {
    with_game(|game| {
        let stone_key = mod_api::__private::block_key("minecraft:stone").unwrap();
        let dirt_key = mod_api::__private::block_key("minecraft:dirt").unwrap();
        let leaves_key = mod_api::__private::block_key("minecraft:oak_leaves").unwrap();
        let water_key = mod_api::__private::block_key("minecraft:water").unwrap();
        let stone_id = api_values::default_state(stone_key).unwrap().0;
        let dirt_id = api_values::default_state(dirt_key).unwrap().0;
        let leaves_id = api_values::default_state(leaves_key).unwrap().0;
        let water_id = api_values::default_state(water_key).unwrap().0;

        let biomes = world_biomes!(game).clone();
        let plains_idx = biomes.try_id("minecraft:plains").unwrap();
        let desert_idx = biomes.try_id("minecraft:desert").unwrap();
        assert_ne!(plains_idx, desert_idx);

        let mut column = Chunk::empty(2, -3);
        column.lit = true;
        // Section index 4 -> block Y 0..16 (min_y = -64).
        {
            let sec = Arc::make_mut(&mut column.sections[4]);
            sec.set(0, stone_id); // (0, 0, 0)
            sec.set((3 << 8) | (1 << 4) | 2, water_id); // (2, 3, 1)
            sec.set((5 << 8) | (1 << 4) | 2, leaves_id); // (2, 5, 1)
            sec.set((1 << 8) | (1 << 4) | 3, stone_id); // (3, 1, 1)
            sec.set((3 << 8) | (1 << 4) | 3, water_id); // (3, 3, 1)
            let mut light_arr = Box::new([0xF0u8; 4096]);
            light_arr[0] = (12 << 4) | 7;
            sec.light = Light::packed(light_arr);
        }
        // Section index 5 -> block Y 16..32: stale palette entry (set then cleared back to air).
        {
            let sec = Arc::make_mut(&mut column.sections[5]);
            sec.set(10, dirt_id);
            sec.set(10, 0);
        }
        column.set_biome_cell(0, 16, 0, desert_idx); // quart (0, 0, 0) in section 4

        game.common.world.chunks.insert((2, -3), Arc::new(column));
        let level = game.level("minecraft:overworld").unwrap();
        let chunk_ref = game.chunk(level, [2, -3]).unwrap();

        // Record live section Arc pointer before capture to prove zero-copy COW sharing.
        let pre_sec4_ptr = Arc::as_ptr(&game.common.world.chunks[&(2, -3)].sections[4]);
        let snap = chunk_ref.snapshot(game).expect("snapshot of loaded chunk");
        assert_eq!(snap.pos(), [2, -3]);
        assert_eq!(snap.min_y(), -64);
        assert_eq!(snap.height(), 384);
        assert_eq!(snap.section_count(), 24);
        assert!(snap.has_light());
        // Strong count of section 4 rose from 1 (live chunk) to 2 (live chunk + snapshot).
        assert_eq!(
            Arc::strong_count(&game.common.world.chunks[&(2, -3)].sections[4]),
            2
        );

        // Mutate live chunk blocks, biomes, AND light after snapshot: Arc::make_mut splits
        // only section 4, leaving the snapshot completely unchanged.
        {
            let live = Arc::make_mut(game.common.world.chunks.get_mut(&(2, -3)).unwrap());
            {
                let sec = Arc::make_mut(&mut live.sections[4]);
                sec.set(0, dirt_id);
                sec.light = Light::Uniform(0x00);
            }
            live.set_biome_cell(0, 16, 0, plains_idx);
        }
        let post_sec4_ptr = Arc::as_ptr(&game.common.world.chunks[&(2, -3)].sections[4]);
        assert_ne!(
            pre_sec4_ptr, post_sec4_ptr,
            "live edit to a shared section must copy-on-write"
        );

        // Move snapshot to a background worker thread and verify all reads off-thread.
        let worker_snap = snap.clone();
        let worker = std::thread::spawn(move || {
            assert_eq!(worker_snap.block_state(0, 0, 0).unwrap().block(), stone_key);
            assert_eq!(worker_snap.light(0, 0, 0), Some((12, 7)));

            let desert = mod_api::BiomeKey::parse("minecraft:desert").unwrap();
            assert_eq!(worker_snap.biome(3, 3, 3), Some(&desert));

            let sec4 = worker_snap.section(4).unwrap();
            assert_eq!(sec4.section_y(), 0);
            assert!(!sec4.is_empty());
            assert_eq!(sec4.uniform_block(), None);
            assert_eq!(sec4.uniform_light(), None);
            assert_eq!(sec4.block_state(0, 0, 0).unwrap().block(), stone_key);
            assert_eq!(sec4.light(0, 0, 0), Some((12, 7)));
            assert_eq!(sec4.biome_quart(0, 0, 0), Some(&desert));
            assert_eq!(sec4.biome_quart(4, 0, 0), None);
            assert_eq!(sec4.block_state(16, 0, 0), None);
            assert_eq!(sec4.light(0, 16, 0), None);

            // Section 5 had a stale palette entry cleared back to air: is_empty() must be true.
            let sec5 = worker_snap.section(5).unwrap();
            assert!(sec5.is_empty());
            assert_eq!(sec5.uniform_light(), Some((15, 0)));

            // Section 0 is untouched uniform air.
            let sec0 = worker_snap.section(0).unwrap();
            assert!(sec0.is_empty());
            assert!(sec0.uniform_block().is_some());
            assert!(sec0.uniform_biome().is_some());

            // Heightmaps at (2, 1): water at y=3, oak_leaves at y=5 (leaves block motion so
            // OceanFloor matches leaves; MotionBlockingNoLeaves ignores leaves and stops at water).
            assert_eq!(
                worker_snap.heightmap(Heightmap::WorldSurface, 2, 1),
                Some(6)
            );
            assert_eq!(
                worker_snap.heightmap(Heightmap::MotionBlocking, 2, 1),
                Some(6)
            );
            assert_eq!(
                worker_snap.heightmap(Heightmap::MotionBlockingNoLeaves, 2, 1),
                Some(4)
            );
            assert_eq!(worker_snap.heightmap(Heightmap::OceanFloor, 2, 1), Some(6));
            // Heightmaps at (3, 1): stone at y=1 under water at y=3 (OceanFloor skips fluid).
            assert_eq!(
                worker_snap.heightmap(Heightmap::WorldSurface, 3, 1),
                Some(4)
            );
            assert_eq!(
                worker_snap.heightmap(Heightmap::MotionBlocking, 3, 1),
                Some(4)
            );
            assert_eq!(
                worker_snap.heightmap(Heightmap::MotionBlockingNoLeaves, 3, 1),
                Some(4)
            );
            assert_eq!(worker_snap.heightmap(Heightmap::OceanFloor, 3, 1), Some(2));
            assert_eq!(worker_snap.heightmap(Heightmap::OceanFloor, 0, 0), Some(1));

            // Out-of-bounds checks never panic.
            assert!(worker_snap.section(24).is_none());
            assert_eq!(worker_snap.block_state(16, 0, 0), None);
            assert_eq!(worker_snap.block_state(0, -65, 0), None);
            assert_eq!(worker_snap.block_state(0, 320, 0), None);
            assert_eq!(worker_snap.light(0, 320, 0), None);
            assert_eq!(worker_snap.biome(0, -65, 0), None);
            assert_eq!(worker_snap.heightmap(Heightmap::WorldSurface, 16, 0), None);
        });
        worker.join().expect("worker thread reads cleanly");

        // Non-Overworld LevelShape (Nether 0..256, 16 sections) clips padded ColumnLayout
        // sections so min_y, height, section_count, heightmap, and bounds match level.height.
        let nether_shape = LevelShape::NETHER;
        let mut nether_col = Chunk::with_layout(3, 4, nether_shape.column());
        nether_col.lit = true;
        // Put stone in padded below-0 section (y = -16) and inside Nether (y = 32).
        let padded_sec = ((0 - nether_col.min_y()) >> 4) as usize - 1;
        let in_range_sec = ((32 - nether_col.min_y()) >> 4) as usize;
        Arc::make_mut(&mut nether_col.sections[padded_sec]).set(0, stone_id);
        Arc::make_mut(&mut nether_col.sections[in_range_sec]).set(0, stone_id);
        let nether_snap = EngineChunkSnapshot::from_chunk(
            &nether_col,
            nether_shape,
            api_values::block_state::current().unwrap(),
            biomes.clone(),
            game.common.pack.assets.properties.clone(),
            true,
        );
        assert_eq!(nether_snap.min_y(), 0);
        assert_eq!(nether_snap.height(), 256);
        assert_eq!(nether_snap.section_count(), 16);
        assert_eq!(nether_snap.section(0).unwrap().section_y(), 0);
        assert_eq!(nether_snap.section(15).unwrap().section_y(), 15);
        assert!(nether_snap.section(16).is_none());
        assert_eq!(nether_snap.block_state(0, -16, 0), None);
        assert_eq!(nether_snap.block_state(0, 256, 0), None);
        assert_eq!(
            nether_snap.block_state(0, 32, 0).unwrap().block(),
            stone_key
        );
        assert_eq!(
            nether_snap.heightmap(Heightmap::WorldSurface, 0, 0),
            Some(33)
        );
        assert_eq!(
            nether_snap.heightmap(Heightmap::WorldSurface, 1, 0),
            Some(0)
        );

        // Unloaded chunk & stale level handles return None.
        let unloaded = level.chunk_ref(game, [99, 99]).unwrap();
        assert!(unloaded.snapshot(game).is_none());
        game.common
            .native_lifecycle
            .begin_open(Some(Authority::IntegratedServer));
        assert!(chunk_ref.snapshot(game).is_none());
        let stale = ChunkRef::new(LevelRef::new(level.epoch(), DimensionKey::NETHER), [2, -3]);
        assert!(stale.snapshot(game).is_none());
    });
}
