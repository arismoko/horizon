//! Immutable, copy-on-write chunk column snapshots (`ChunkRef::snapshot`, #5233).
//!
//! Each `Section` in `sunrise_level::chunk::Chunk` stores blocks, 3D quart biomes, and sky/block
//! light together behind one `Arc<Section>`, and every live mutation goes through `Arc::make_mut`.
//! Capturing a `ChunkSnapshot` therefore clones only the `Vec<Arc<Section>>` pointers plus shared
//! read-only tables (`Index`, `Biomes`, `Properties`) with zero array copies.
use crate::game::api_values::block_state::Index;
use crate::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
use mod_api::{BlockState, ChunkRef, ChunkSnapshot, Heightmap};
use std::sync::Arc;
use sunrise_level::assets::Properties;
use sunrise_level::biome::Biomes;
use sunrise_level::chunk::{Chunk, LevelShape, Light, Section};
use sunrise_world::heightmap::{HeightmapState, HeightmapType};
use sunrise_world::registry::BlockStateId;
use sunrise_world::worldgen::terrain::TerrainBlockStates;

pub struct EngineChunkSnapshot {
    pos: [i32; 2],
    min_y: i32,
    sections: Arc<[Arc<Section>]>,
    states: Arc<Index>,
    biomes: Arc<Biomes>,
    properties: Properties,
    has_light: bool,
}

impl EngineChunkSnapshot {
    /// Wrap a chunk's copy-on-write `Arc<Section>` list into an immutable [`ChunkSnapshot`],
    /// clipped to the level's actual `LevelShape` build height rather than the widened internal
    /// `ColumnLayout`.
    pub fn from_chunk(
        chunk: &Chunk,
        shape: LevelShape,
        states: Arc<Index>,
        biomes: Arc<Biomes>,
        properties: Properties,
        has_light: bool,
    ) -> ChunkSnapshot {
        let start = ((shape.min_y - chunk.min_y()) >> 4).max(0) as usize;
        let end = start
            .saturating_add(shape.section_count())
            .min(chunk.sections.len());
        let sections: Arc<[Arc<Section>]> = if start <= end {
            chunk.sections[start..end].iter().cloned().collect()
        } else {
            Arc::from([])
        };
        ChunkSnapshot::from_backend(Arc::new(Self {
            pos: [chunk.x, chunk.z],
            min_y: shape.min_y,
            sections,
            states,
            biomes,
            properties,
            has_light,
        }))
    }

    #[inline]
    fn section_and_local_y(&self, y: i32) -> Option<(&Section, usize)> {
        let rel = y.checked_sub(self.min_y)?;
        if rel < 0 {
            return None;
        }
        let si = (rel >> 4) as usize;
        let section = self.sections.get(si)?;
        Some((section, (rel & 15) as usize))
    }

    #[inline]
    fn heightmap_type(kind: Heightmap) -> Option<HeightmapType> {
        match kind {
            Heightmap::WorldSurface => Some(HeightmapType::WorldSurface),
            Heightmap::MotionBlocking => Some(HeightmapType::MotionBlocking),
            Heightmap::MotionBlockingNoLeaves => Some(HeightmapType::MotionBlockingNoLeaves),
            Heightmap::OceanFloor => Some(HeightmapType::OceanFloor),
            _ => None,
        }
    }
}

impl mod_api::__private::ChunkSnapshotBackend for EngineChunkSnapshot {
    #[inline]
    fn pos(&self) -> [i32; 2] {
        self.pos
    }

    #[inline]
    fn min_y(&self) -> i32 {
        self.min_y
    }

    #[inline]
    fn height(&self) -> i32 {
        (self.sections.len() as i32) * 16
    }

    #[inline]
    fn section_count(&self) -> usize {
        self.sections.len()
    }

    #[inline]
    fn has_light(&self) -> bool {
        self.has_light
    }

    #[inline]
    fn heightmap(&self, kind: Heightmap, lx: u8, lz: u8) -> Option<i32> {
        if lx >= 16 || lz >= 16 {
            return None;
        }
        let htype = Self::heightmap_type(kind)?;
        let xz = (usize::from(lz) << 4) | usize::from(lx);
        for (si, section) in self.sections.iter().enumerate().rev() {
            if !section
                .blocks
                .maybe_has(|id| htype.is_opaque(self.properties.heightmap_state(BlockStateId(id))))
            {
                continue;
            }
            let base_y = self.min_y + (si as i32) * 16;
            for ly in (0..16usize).rev() {
                let id = section.blocks.get((ly << 8) | xz);
                if htype.is_opaque(self.properties.heightmap_state(BlockStateId(id))) {
                    return Some(base_y + (ly as i32) + 1);
                }
            }
        }
        Some(self.min_y)
    }

    #[inline]
    fn block_state(&self, lx: u8, y: i32, lz: u8) -> Option<BlockState> {
        if lx >= 16 || lz >= 16 {
            return None;
        }
        let (section, ly) = self.section_and_local_y(y)?;
        let id = section
            .blocks
            .get((ly << 8) | (usize::from(lz) << 4) | usize::from(lx));
        self.states.to_api(BlockStateId(id)).ok()
    }

    #[inline]
    fn light(&self, lx: u8, y: i32, lz: u8) -> Option<(u8, u8)> {
        if lx >= 16 || lz >= 16 {
            return None;
        }
        let (section, ly) = self.section_and_local_y(y)?;
        let packed = section
            .light
            .get((ly << 8) | (usize::from(lz) << 4) | usize::from(lx));
        Some((packed >> 4, packed & 0x0F))
    }

    #[inline]
    fn biome(&self, lx: u8, y: i32, lz: u8) -> Option<u32> {
        if lx >= 16 || lz >= 16 {
            return None;
        }
        let (section, ly) = self.section_and_local_y(y)?;
        let qx = usize::from(lx >> 2);
        let qy = ly >> 2;
        let qz = usize::from(lz >> 2);
        Some(section.biomes.get((qy << 4) | (qz << 2) | qx))
    }

    #[inline]
    fn biome_key(&self, biome_index: u32) -> Option<&mod_api::BiomeKey> {
        self.biomes
            .registered_keys()
            .get(usize::try_from(biome_index).ok()?)
    }

    #[inline]
    fn section_y(&self, section_index: usize) -> i32 {
        (self.min_y >> 4) + (section_index as i32)
    }

    #[inline]
    fn section_is_empty(&self, section_index: usize) -> bool {
        let Some(section) = self.sections.get(section_index) else {
            return true;
        };
        if !section
            .blocks
            .maybe_has(|id| !self.properties.heightmap_state(BlockStateId(id)).is_air())
        {
            return true;
        }
        if let Some(id) = section.blocks.uniform() {
            return self.properties.heightmap_state(BlockStateId(id)).is_air();
        }
        (0..4096).all(|i| {
            self.properties
                .heightmap_state(BlockStateId(section.blocks.get(i)))
                .is_air()
        })
    }

    #[inline]
    fn section_uniform_block(&self, section_index: usize) -> Option<BlockState> {
        let id = self.sections.get(section_index)?.blocks.uniform()?;
        self.states.to_api(BlockStateId(id)).ok()
    }

    #[inline]
    fn section_uniform_light(&self, section_index: usize) -> Option<(u8, u8)> {
        match self.sections.get(section_index)?.light {
            Light::Uniform(packed) => Some((packed >> 4, packed & 0x0F)),
            _ => None,
        }
    }

    #[inline]
    fn section_uniform_biome(&self, section_index: usize) -> Option<u32> {
        self.sections.get(section_index)?.biomes.uniform()
    }

    #[inline]
    fn section_block_state(
        &self,
        section_index: usize,
        lx: u8,
        ly: u8,
        lz: u8,
    ) -> Option<BlockState> {
        if lx >= 16 || ly >= 16 || lz >= 16 {
            return None;
        }
        let section = self.sections.get(section_index)?;
        let id = section
            .blocks
            .get((usize::from(ly) << 8) | (usize::from(lz) << 4) | usize::from(lx));
        self.states.to_api(BlockStateId(id)).ok()
    }

    #[inline]
    fn section_light(&self, section_index: usize, lx: u8, ly: u8, lz: u8) -> Option<(u8, u8)> {
        if lx >= 16 || ly >= 16 || lz >= 16 {
            return None;
        }
        let section = self.sections.get(section_index)?;
        let packed = section
            .light
            .get((usize::from(ly) << 8) | (usize::from(lz) << 4) | usize::from(lx));
        Some((packed >> 4, packed & 0x0F))
    }

    #[inline]
    fn section_biome_quart(&self, section_index: usize, qx: u8, qy: u8, qz: u8) -> Option<u32> {
        if qx >= 4 || qy >= 4 || qz >= 4 {
            return None;
        }
        let section = self.sections.get(section_index)?;
        Some(
            section
                .biomes
                .get((usize::from(qy) << 4) | (usize::from(qz) << 2) | usize::from(qx)),
        )
    }
}

pub trait NativeChunkSnapshotGameExt<'__sg>: crate::game::SideGame<'__sg> {
    /// Capture an immutable, copy-on-write snapshot of a resident chunk on either side (server
    /// active/parked levels, or a logged-in client's loaded level).
    fn chunk_snapshot(&self, chunk: ChunkRef) -> Option<ChunkSnapshot> {
        let [x, z] = chunk.pos();
        let world = self.native_level_view_world(chunk.level())?;
        let column = world.chunks.get(&(x, z))?;
        let states = crate::game::api_values::block_state::current().ok()?;
        let biomes = crate::world_biomes!(self).clone();
        let properties = self.pack.assets.properties.clone();
        let has_light = column.lit || column.light_loaded;
        Some(EngineChunkSnapshot::from_chunk(
            column,
            world.shape,
            states,
            biomes,
            properties,
            has_light,
        ))
    }
}

impl<'__sg, G: crate::game::SideGame<'__sg>> NativeChunkSnapshotGameExt<'__sg> for G {}
