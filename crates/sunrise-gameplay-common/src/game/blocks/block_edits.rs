#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    use crate::game::block_edits_common::column_height;
    use mod_api::{BlockEdit, Heightmap};
    use std::sync::Arc;
    use sunrise_level::world::World;
    use sunrise_world::registry::BlockStateId;

    use mod_api::BlockRun;

    use sunrise_level::chunk::Chunk;

    use sunrise_world::heightmap::HeightmapType;

    pub fn run(block: u32, only: Option<u32>) -> BlockRun {
        BlockRun {
            x: 1,
            z: 2,
            y: 3,
            height: 4,
            surface: None,
            block,
            only,
        }
    }
    pub fn edit(runs: Vec<BlockRun>) -> BlockEdit {
        BlockEdit {
            palette: vec![],
            filters: vec![],
            runs,
        }
    }
    pub fn world() -> World {
        let mut w = World::new(BlockStateId(0), 0);
        w.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        w
    }
    pub fn names(s: u32) -> Arc<str> {
        match s {
            0 => "minecraft:air",
            1 => "minecraft:stone",
            2 => "minecraft:chest",
            // another chest state (a `facing` flip): the name is unchanged
            4 => "minecraft:chest",
            _ => "machines:press",
        }
        .into()
    }
    /// air 0, stone 1, leaves 2, water 3: column (1, 1) stone to 60, water, then leaves to 64; column (2, 2)
    /// stone to 60, leaves, then water to 64; at (3, 3) stone at the top of the world.
    pub fn columns_world() -> World {
        let mut w = world();
        for (x, z, low, high) in [(1, 1, 3, 2), (2, 2, 2, 3)] {
            for y in -64..=64 {
                let s = match y {
                    ..=60 => 1,
                    61..=62 => low,
                    _ => high,
                };
                w.set(x, y, z, BlockStateId(s)).unwrap();
            }
        }
        w.set(3, 319, 3, BlockStateId(1)).unwrap();
        w
    }
    pub fn height(w: &World, h: Heightmap, x: i32, z: i32) -> i32 {
        column_height(
            w,
            h,
            (x, z),
            |s| s.0 == 0,
            |kind, s| match kind {
                HeightmapType::MotionBlockingNoLeaves => matches!(s.0, 1 | 3),
                HeightmapType::OceanFloor => matches!(s.0, 1 | 2),
                _ => s.0 != 0,
            },
        )
    }
}
