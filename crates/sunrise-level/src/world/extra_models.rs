//! The standalone models a client mod draws as part of a block
//! (`ClientRenderClientHost::set_block_extra_models`), kept in each column
//! (`Chunk::extra_models`) so the meshing workers read them with the blocks and call no mod code.
use super::World;
use mod_api::{BlockExtraModelsError, ModelRef};
use std::sync::Arc;

impl World {
    /// `models` replace the extra models of the block at (`x`, `y`, `z`); an empty list clears
    /// them. When they change, the block's section is meshed again. `NotLoaded` outside the
    /// resident columns or the world's height, with nothing changed. The handles are the
    /// caller's to check.
    pub fn set_extra_models(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        models: &[ModelRef],
    ) -> Result<(), BlockExtraModelsError> {
        let key = (x >> 4, z >> 4);
        let Some(chunk) = self.chunks.get_mut(&key) else {
            return Err(BlockExtraModelsError::NotLoaded);
        };
        let Some(cell) = chunk.dynamic_cell(x, y, z) else {
            return Err(BlockExtraModelsError::NotLoaded);
        };
        let held = chunk
            .extra_models
            .get(&cell)
            .map_or(&[][..], |held| &held[..]);
        if held == models {
            return Ok(());
        }
        let stored = &mut Arc::make_mut(chunk).extra_models;
        if models.is_empty() {
            stored.remove(&cell);
        } else {
            stored.insert(cell, models.into());
        }
        self.dirty_sections
            .insert((key.0, (cell >> 12) as i32, key.1));
        Ok(())
    }
    /// The extra models of the block at (`x`, `y`, `z`), empty for none.
    pub fn extra_models(&self, x: i32, y: i32, z: i32) -> &[ModelRef] {
        self.chunks
            .get(&(x >> 4, z >> 4))
            .and_then(|chunk| chunk.extra_models.get(&chunk.dynamic_cell(x, y, z)?))
            .map_or(&[], |models| &models[..])
    }
    /// Every column's extra models dropped, their sections meshed again: the player's level
    /// changed, and the mods' lists were for the level left.
    pub fn clear_extra_models(&mut self) {
        for chunk in self.chunks.values_mut() {
            if chunk.extra_models.is_empty() {
                continue;
            }
            let key = (chunk.x, chunk.z);
            let chunk = Arc::make_mut(chunk);
            for cell in chunk.extra_models.keys() {
                self.dirty_sections
                    .insert((key.0, (cell >> 12) as i32, key.1));
            }
            chunk.extra_models.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::Chunk;
    use sunrise_world::registry::BlockStateId;

    const STONE: BlockStateId = BlockStateId(1);
    const DIRT: BlockStateId = BlockStateId(2);

    /// Columns (0, 0) and (1, 0), stone at y 70 of x 3, z 5.
    fn world() -> World {
        let mut world = World::new(BlockStateId(0), 0);
        for x in 0..2 {
            world.chunks.insert((x, 0), Arc::new(Chunk::empty(x, 0)));
        }
        world.set(3, 70, 5, STONE).unwrap();
        world.dirty_sections.clear();
        world
    }

    fn model(slot: u32) -> ModelRef {
        mod_api::__private::built::model_ref(1, slot)
    }

    /// The section index of y 70 in a vanilla column (minimum y -64).
    const SECTION: i32 = (70 + 64) >> 4;

    #[test]
    fn a_list_replaces_the_last_and_an_empty_one_clears_it() {
        let mut world = world();
        world
            .set_extra_models(3, 70, 5, &[model(1), model(2)])
            .unwrap();
        assert_eq!(world.extra_models(3, 70, 5), [model(1), model(2)]);
        assert!(world.dirty_sections.contains(&(0, SECTION, 0)));
        assert_eq!(
            world.dirty_sections.len(),
            1,
            "only the block's own section"
        );
        world.dirty_sections.clear();
        world.set_extra_models(3, 70, 5, &[model(3)]).unwrap();
        assert_eq!(world.extra_models(3, 70, 5), [model(3)]);
        assert!(world.dirty_sections.contains(&(0, SECTION, 0)));
        world.dirty_sections.clear();
        // The same list again changes nothing and meshes nothing.
        world.set_extra_models(3, 70, 5, &[model(3)]).unwrap();
        assert!(world.dirty_sections.is_empty());
        assert_eq!(
            world.extra_models(3, 71, 5),
            [],
            "a list is the position's own"
        );
        world.set_extra_models(3, 70, 5, &[]).unwrap();
        assert_eq!(world.extra_models(3, 70, 5), []);
        assert!(world.chunks.get(&(0, 0)).unwrap().extra_models.is_empty());
        assert!(world.dirty_sections.contains(&(0, SECTION, 0)));
        world.dirty_sections.clear();
        world.set_extra_models(3, 70, 5, &[]).unwrap();
        assert!(
            world.dirty_sections.is_empty(),
            "clearing nothing meshes nothing"
        );
    }

    #[test]
    fn outside_the_columns_nothing_changes() {
        let mut world = world();
        assert_eq!(
            world.set_extra_models(40, 70, 5, &[model(1)]),
            Err(BlockExtraModelsError::NotLoaded),
            "no column there"
        );
        assert_eq!(
            world.set_extra_models(3, 320, 5, &[model(1)]),
            Err(BlockExtraModelsError::NotLoaded),
            "above the build height"
        );
        assert!(world.dirty_sections.is_empty());
    }

    #[test]
    fn a_change_of_the_block_drops_its_list() {
        let mut world = world();
        world.set_extra_models(3, 70, 5, &[model(1)]).unwrap();
        world.set_extra_models(4, 70, 5, &[model(2)]).unwrap();
        // The same state again is no change.
        world.set(3, 70, 5, STONE).unwrap();
        assert_eq!(world.extra_models(3, 70, 5), [model(1)]);
        world.set(3, 70, 5, DIRT).unwrap();
        assert_eq!(world.extra_models(3, 70, 5), []);
        assert_eq!(
            world.extra_models(4, 70, 5),
            [model(2)],
            "its neighbour's stays"
        );
        // A bulk edit drops the lists of the cells it changes and only those.
        world.set_extra_models(3, 70, 5, &[model(1)]).unwrap();
        let section = SECTION as usize;
        let chunk = Arc::make_mut(world.chunks.get_mut(&(0, 0)).unwrap());
        let mut values: [u32; 4096] = std::array::from_fn(|i| chunk.sections[section].get(i));
        values[crate::chunk::index(4, 70, 5)] = DIRT.0;
        chunk.replace_blocks(section, &values);
        assert_eq!(world.extra_models(3, 70, 5), [model(1)]);
        assert_eq!(world.extra_models(4, 70, 5), []);
    }

    #[test]
    fn a_list_needs_no_block_entity_and_goes_with_its_column() {
        let mut world = world();
        world.set_extra_models(3, 70, 5, &[model(1)]).unwrap();
        world.set_extra_models(17, 70, 5, &[model(2)]).unwrap();
        world.chunks.remove(&(0, 0));
        assert_eq!(world.extra_models(3, 70, 5), []);
        // The column sent again comes without it.
        world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        assert_eq!(world.extra_models(3, 70, 5), []);
        assert_eq!(world.extra_models(17, 70, 5), [model(2)]);
    }

    #[test]
    fn a_level_change_clears_every_list_and_meshes_its_sections() {
        let mut world = world();
        world.set_extra_models(3, 70, 5, &[model(1)]).unwrap();
        world.set_extra_models(17, 10, 5, &[model(2)]).unwrap();
        world.dirty_sections.clear();
        world.clear_extra_models();
        assert_eq!(world.extra_models(3, 70, 5), []);
        assert_eq!(world.extra_models(17, 10, 5), []);
        assert!(world.dirty_sections.contains(&(0, SECTION, 0)));
        assert!(world.dirty_sections.contains(&(1, (10 + 64) >> 4, 0)));
        assert_eq!(world.dirty_sections.len(), 2);
    }
}
