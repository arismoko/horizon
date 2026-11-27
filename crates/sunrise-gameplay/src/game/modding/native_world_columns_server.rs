//! Streaming column height and block-slice queries on server open levels.
use crate::game::ServerGame;
use std::ops::ControlFlow;
use sunrise_gameplay_common::game::{
    block_edits_common::{column_span, fill_column_blocks},
    native_level_refs_common::NativeLevelRefsCommonGameExt1 as _,
};

impl ServerGame<'_> {
    /// Visit each column's height by `heightmap` in an active or parked open level, `None`
    /// where its chunk is not loaded. Loads nothing.
    pub fn level_visit_surface_heights(
        &self,
        level: mod_api::LevelRef,
        columns: &[(i32, i32)],
        heightmap: mod_api::Heightmap,
        visit: &mut dyn FnMut((i32, i32), Option<i32>) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, mod_api::LevelReadError> {
        let world = self.native_level_read_world_result(level)?;
        let started = std::time::Instant::now();
        let mut last: Option<((i32, i32), bool)> = None;
        let mut flow = ControlFlow::Continue(());
        let mut visited = 0usize;
        for &(x, z) in columns {
            let key = (x >> 4, z >> 4);
            let loaded = match last {
                Some((k, l)) if k == key => l,
                _ => {
                    let l = world.chunks.contains_key(&key);
                    last = Some((key, l));
                    l
                }
            };
            let height = loaded.then(|| self.common.world_column_height(world, heightmap, x, z));
            visited += 1;
            if visit((x, z), height).is_break() {
                flow = ControlFlow::Break(());
                break;
            }
        }
        self.common
            .native_read_logged(started, "height query", visited);
        Ok(flow)
    }

    /// Visit each column's height on `heightmap`, server-side biome JSON water colour and
    /// complete block states from `below` under to `above - 1` over that height, bottom first
    /// (air outside the chunk's vertical range; `None` where the column's chunk is not loaded).
    /// Works on active and parked server levels with no column-count or total-block cap.
    pub fn level_visit_columns(
        &self,
        level: mod_api::LevelRef,
        columns: &[(i32, i32)],
        heightmap: mod_api::Heightmap,
        range: (u32, u32),
        visit: &mut dyn for<'a> FnMut(
            (i32, i32),
            Option<mod_api::ColumnSlice<'a>>,
        ) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, mod_api::LevelReadError> {
        let world = self.native_level_world_result(level)?;
        let (below, above) = range;
        let per = column_span(below, above, world.shape.height)
            .ok_or(mod_api::LevelReadError::Unavailable)?;
        let index = super::native_worldgen::prepare(self.common.pack.assets.registry)
            .map_err(|_| mod_api::LevelReadError::Unavailable)?;
        let started = std::time::Instant::now();
        let biomes = world_biomes!(self);
        let mut raw = Vec::with_capacity(per);
        let mut states = Vec::with_capacity(per);
        let mut flow = ControlFlow::Continue(());
        let mut visited = 0usize;
        for &(x, z) in columns {
            let slice = if let Some(chunk) = world.chunks.get(&(x >> 4, z >> 4)) {
                let base = self.common.world_column_height(world, heightmap, x, z);
                raw.clear();
                fill_column_blocks(
                    world.air,
                    chunk,
                    world.shape,
                    x,
                    z,
                    base,
                    below,
                    above,
                    &mut raw,
                );
                states.clear();
                for &state in &raw {
                    let api_state = index
                        .to_api(state)
                        .map_err(|_| mod_api::LevelReadError::Unavailable)?;
                    states.push(api_state);
                }
                let water_color = biomes.get(chunk.biome(x, base - 1, z)).water;
                Some(mod_api::ColumnSlice::new(base, &states, water_color))
            } else {
                None
            };
            visited += 1;
            if visit((x, z), slice).is_break() {
                flow = ControlFlow::Break(());
                break;
            }
        }
        self.common
            .native_read_logged(started, "column read", visited);
        Ok(flow)
    }
}
