use super::{Chunk, Section, index};
use crate::assets::{Properties, StateFacts};
use std::{ops::Range, sync::Arc};
use sunrise_world::{
    heightmap::{Heightmap, HeightmapChunk, HeightmapType},
    registry::BlockStateId,
    worldgen::terrain::TerrainBlockStates,
};

impl Chunk {
    /// `ChunkAccess.setHeightmap` for the MOTION_BLOCKING packet entry. An omitted map
    /// leaves the resident one unchanged or starts a fresh chunk at its dimension minimum.
    pub fn adopt_packet_motion_blocking(
        &mut self,
        resident: Option<&Self>,
        [min_y, height]: [i32; 2],
        words: Option<&[i64]>,
        properties: &Properties,
    ) {
        let mut map = resident
            .and_then(|old| old.packet_motion_blocking.clone())
            .unwrap_or_else(|| {
                Arc::new(
                    Heightmap::new(HeightmapType::MotionBlocking, min_y, height)
                        .expect("validated packet dimension shape"),
                )
            });
        if let Some(words) = words {
            let column = HeightmapColumn {
                sections: &self.sections,
                column_min_y: self.min_y,
                range: map.build_height(),
                properties,
            };
            let expected = map.raw_data().len();
            if expected != words.len() {
                log::warn!(
                    "Ignoring heightmap data for chunk [{}, {}], size does not match; expected: {expected}, got: {}",
                    self.x,
                    self.z,
                    words.len(),
                );
            }
            Arc::make_mut(&mut map)
                .set_raw_data(&column, bytemuck::cast_slice(words))
                .expect("map and column view share the same shape");
        }
        self.packet_motion_blocking = Some(map);
    }

    /// `LevelChunk.setBlockState`: update after the changed state is written.
    pub fn update_packet_motion_blocking(&mut self, [x, y, z]: [i32; 3], properties: &Properties) {
        let Some(map) = &mut self.packet_motion_blocking else {
            return;
        };
        let range = map.build_height();
        if !range.contains(&y) {
            return;
        }
        let column = HeightmapColumn {
            sections: &self.sections,
            column_min_y: self.min_y,
            range,
            properties,
        };
        let (x, z) = (x & 15, z & 15);
        if !map.below_update(&column, x, y, z) {
            Arc::make_mut(map)
                .update(&column, x, y, z, column.block_state(x, y, z))
                .expect("map update within the column view");
        }
    }

    pub fn packet_motion_blocking_height(&self, x: i32, z: i32) -> Option<i32> {
        self.packet_motion_blocking.as_ref().map(|map| {
            map.first_available(x & 15, z & 15)
                .expect("column coordinates")
        })
    }
}

/// The level's actual build range within the chunk's padded renderer layout.
struct HeightmapColumn<'a> {
    sections: &'a [Arc<Section>],
    column_min_y: i32,
    range: Range<i32>,
    properties: &'a Properties,
}

impl HeightmapChunk for HeightmapColumn<'_> {
    type State = StateFacts;

    fn min_y(&self) -> i32 {
        self.range.start
    }

    fn height(&self) -> i32 {
        self.range.end - self.range.start
    }

    fn highest_section_y(&self) -> i32 {
        self.sections
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, section)| {
                let y = self.column_min_y + i as i32 * 16;
                (self.range.contains(&y) && !section.is_uniform(0)).then_some(y)
            })
            .unwrap_or(self.range.start)
    }

    /// `LevelChunkSection.getBlockState`: the palette entry in the installed section.
    fn block_state(&self, x: i32, y: i32, z: i32) -> Self::State {
        let i = (y - self.column_min_y).div_euclid(16);
        let state = self
            .sections
            .get(i as usize)
            .map_or(0, |section| section.get(index(x, y, z)));
        self.properties.heightmap_state(BlockStateId(state))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_core::storage::PackedStorage;

    #[test]
    fn packet_motion_blocking_uses_the_dimension_not_the_padded_layout() {
        let properties = Properties::for_tests(&[(true, false, false, false)]);
        let mut chunk = Chunk::empty(0, 0);
        // The renderer still pads this 128-block dimension to the Overworld layout.
        let mut wire = PackedStorage::new(8, 256).unwrap();
        wire.set(2 * 16 + 10, 87).unwrap();
        chunk.adopt_packet_motion_blocking(
            None,
            [0, 128],
            Some(bytemuck::cast_slice(wire.words())),
            &properties,
        );
        assert_eq!(chunk.motion_blocking(10, 2, |_| false), 87);
        assert_eq!(chunk.motion_blocking(2, 10, |_| false), 0);
    }
}
