//! `light_emission`, the light a `dynamic_light` block emits by where it is
//! (`IBlockExtension.getLightEmission`): asked of its parts on the thread that owns the
//! level as a chunk holding one comes in, as an edit or `level_check_light` reaches one, and
//! on a client as the server's light update reaches its section, the answers kept in the
//! chunk (`Chunk::dynamic_emission`) for the light engine, which never asks on its own.
use super::*;
use sunrise_gameplay_common::game::light_emission::{
    LightEmissionGameExt1 as _, apply_chunk_light_emission, store_light_emission,
};
use sunrise_level::fast_hash::FxHashMap;

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn load_full_column(&mut self, column: (i32, i32)) -> bool {
        self.load_full_chunk(None, column).is_ok()
    }

    pub(in crate::game) fn ask_loaded_light_emission(&mut self, first: usize) {
        if !self.common.pack.chunk_context.light.any_dynamic() {
            return;
        }
        for i in first..self.common.world.loaded_chunks.len() {
            let (x, z, _) = self.common.world.loaded_chunks[i];
            let key = (x, z);
            if self
                .common
                .world
                .chunks
                .get(&key)
                .is_some_and(|chunk| chunk.dynamic_pending)
            {
                self.ask_chunk_light_emission(key);
            }
        }
    }

    pub(in crate::game) fn ask_edited_light_emission(&mut self, dim: Dim) {
        let Some(world) = self.light_edit_world(dim) else {
            return;
        };
        // The checks queued so far are asked now: a later one at the same cell queues again.
        world.light_checks.clear();
        let count = world.light_edits.len();
        if count == 0 || !self.common.pack.chunk_context.light.any_dynamic() {
            return;
        }
        let dimension = if dim == self.common.dim {
            self.current_light_emission_dimension()
        } else {
            dim.level()
        };
        for i in 0..count {
            let Some(world) = self.light_edit_world(dim) else {
                return;
            };
            let ((x, y, z), _, _) = world.light_edits[i];
            let state = world.block(x, y, z).0;
            if !self.common.pack.chunk_context.light.is_dynamic(state) {
                continue;
            }
            let emission = self.ask_light_emission(dimension, x, y, z, state);
            let key = (x >> 4, z >> 4);
            if let Some(world) = self.light_edit_world(dim)
                && let Some(cell) = world.chunks.get(&key).and_then(|c| c.dynamic_cell(x, y, z))
            {
                store_light_emission(world, key, cell, emission);
            }
        }
    }

    fn light_edit_world(&mut self, dim: Dim) -> Option<&mut World> {
        if dim == self.common.dim {
            Some(&mut self.common.world)
        } else {
            self.server.levels.0.parked_mut(dim).map(|p| &mut p.world)
        }
    }

    /// Query dynamic light emission in the active (`None`) or parked (`Some`) level.
    pub(in crate::game::server_side) fn ask_level_chunk_light_emission(
        &mut self,
        parked: Option<&str>,
        dimension: LevelDimension,
        key: (i32, i32),
    ) {
        let world = match parked {
            None => &self.common.world,
            Some(id) => match self.server.levels.0.parked_world_of(id) {
                Some(world) => world,
                None => return,
            },
        };
        let Some(chunk) = world.chunks.get(&key) else {
            return;
        };
        let pending = chunk.dynamic_pending;
        let cells = self
            .common
            .pack
            .chunk_context
            .light
            .dynamic_cells(chunk, |_| true);
        if cells.is_empty() && !pending {
            return;
        }
        let mut answers = FxHashMap::default();
        for (cell, [x, y, z], state) in cells {
            if let Some(emission) = self.ask_light_emission(dimension, x, y, z, state) {
                answers.insert(cell, emission);
            }
        }
        let world = match parked {
            None => &mut self.common.world,
            Some(id) => match self.server.levels.0.parked_world_mut_of(id) {
                Some(world) => world,
                None => return,
            },
        };
        apply_chunk_light_emission(world, key, pending, answers);
    }
}

#[cfg(any(test, feature = "test-support"))]
impl crate::game::ServerGame<'_> {
    pub fn test_load_full_column(&mut self, column: (i32, i32)) -> bool {
        self.load_full_column(column)
    }

    pub fn test_ask_loaded_light_emission(&mut self, first: usize) {
        self.ask_loaded_light_emission(first);
    }

    pub fn test_ask_edited_light_emission(&mut self, dim: Dim) {
        self.ask_edited_light_emission(dim);
    }
}
