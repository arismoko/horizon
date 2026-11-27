//! `ClientRenderClientHost::set_section_dirty` and `set_blocks_dirty`: a client mod asks the
//! mesher to rebuild loaded sections (`LevelExtractor.setSectionDirty`,
//! `ClientLevel.setSectionDirtyWithNeighbors`, `LevelExtractor.setBlocksDirty`); and
//! `visit_rendered_chunk_columns`, the columns the mesher has meshed.
use crate::game::ClientGame;

impl ClientGame<'_> {
    /// The meshed columns of the level in place (`Streaming::meshed_columns`), until `visit`
    /// breaks.
    pub(in crate::game) fn visit_rendered_chunk_columns(
        &self,
        visit: &mut dyn FnMut([i32; 2]) -> std::ops::ControlFlow<()>,
    ) -> std::ops::ControlFlow<()> {
        for (x, z) in self.client.streaming.meshed_columns() {
            visit([x, z])?;
        }
        std::ops::ControlFlow::Continue(())
    }

    /// Queues section `(cx, section_y, cz)` (vanilla section coordinates) for remeshing when its
    /// column is loaded and the level holds that height. Whether it was queued.
    fn mark_section_dirty(&mut self, cx: i32, section_y: i32, cz: i32) -> bool {
        let layout = self.common.world.layout();
        let sy = section_y - (layout.min_y() >> 4);
        if sy < 0
            || sy >= layout.sections() as i32
            || !self.common.world.chunks.contains_key(&(cx, cz))
        {
            return false;
        }
        self.common.world.dirty_sections.insert((cx, sy, cz));
        true
    }

    /// `LevelExtractor.setSectionDirty`, with `with_neighbors` as
    /// `ClientLevel.setSectionDirtyWithNeighbors`, which marks each of the 27 sections that is
    /// loaded whether or not the centre is. Whether the centre section was loaded and marked.
    pub(in crate::game) fn set_section_dirty(
        &mut self,
        section: [i32; 3],
        with_neighbors: bool,
    ) -> bool {
        let [x, y, z] = section;
        let centre = self.mark_section_dirty(x, y, z);
        if with_neighbors {
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        // A neighbour past the ends of `i32` is one no level loads.
                        let (Some(nx), Some(ny), Some(nz)) =
                            (x.checked_add(dx), y.checked_add(dy), z.checked_add(dz))
                        else {
                            continue;
                        };
                        self.mark_section_dirty(nx, ny, nz);
                    }
                }
            }
        }
        centre
    }

    /// `LevelExtractor.setBlocksDirty`: every loaded section holding a block of the box widened
    /// by one block on each side, so a box that touches a section's edge also marks the section
    /// across it.
    pub(in crate::game) fn set_blocks_dirty(&mut self, min: [i32; 3], max: [i32; 3]) {
        let layout = self.common.world.layout();
        let lo = layout.min_y() >> 4;
        let hi = lo + layout.sections() as i32 - 1;
        // `min - 1` and `max + 1` in `i64`, so corners at the ends of `i32` cannot overflow; the
        // section index of either still fits an `i32`.
        let span = |axis: usize| {
            let (a, b) = (i64::from(min[axis]), i64::from(max[axis]));
            (((a.min(b) - 1) >> 4) as i32, ((a.max(b) + 1) >> 4) as i32)
        };
        let (x0, x1) = span(0);
        let (y0, y1) = span(1);
        let (y0, y1) = (y0.max(lo), y1.min(hi));
        let (z0, z1) = span(2);
        // The loaded columns, so a box of any size costs what is loaded.
        let columns: Vec<_> = self
            .common
            .world
            .chunks
            .keys()
            .copied()
            .filter(|&(cx, cz)| (x0..=x1).contains(&cx) && (z0..=z1).contains(&cz))
            .collect();
        for (cx, cz) in columns {
            for y in y0..=y1 {
                self.mark_section_dirty(cx, y, cz);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use mod_api::prelude::ClientRenderClientHost;

    /// Only loaded sections inside the level's height are queued, one section or a box of them,
    /// with or without the neighbours. Runs in its own process: loading a game registers the
    /// game's block keys and particle providers process-wide.
    #[test]
    #[ignore = "daily"]
    fn host_marks_only_loaded_sections_dirty() {
        use crate::game::native_entity_refs::tests::with_client_game;
        sunrise_gameplay_common::game::in_own_process(
            "game::client_side::native_section_dirty::tests::host_marks_only_loaded_sections_dirty",
            || {
                with_client_game(|game| {
                    game.common.world.chunks.clear();
                    game.common.world.chunks.insert(
                        (0, 0),
                        std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                    );
                    let min_section = game.common.world.bottom_y() >> 4;
                    let sections = game.common.world.layout().sections() as i32;
                    // The sections of column (0, 0) queued, as vanilla section Ys.
                    let marked = |game: &mut crate::game::ClientGame<'_>| {
                        assert!(!game.common.world.dirty_sections.contains(&(1, 0, 0)));
                        let marked: Vec<i32> = (0..sections)
                            .filter(|&sy| game.common.world.dirty_sections.contains(&(0, sy, 0)))
                            .map(|sy| sy + min_section)
                            .collect();
                        game.common.world.dirty_sections.clear();
                        marked
                    };
                    game.common.world.dirty_sections.clear();

                    assert!(!ClientRenderClientHost::set_section_dirty(
                        game,
                        [1, 4, 0],
                        false
                    ));
                    assert!(game.common.world.dirty_sections.is_empty());
                    // An unloaded centre: its loaded neighbours are still marked, as vanilla marks all 27.
                    assert!(!ClientRenderClientHost::set_section_dirty(
                        game,
                        [1, 4, 0],
                        true
                    ));
                    assert_eq!(marked(game), vec![3, 4, 5]);
                    // A centre below the level: only its neighbour above, the bottom section, is in it.
                    assert!(!ClientRenderClientHost::set_section_dirty(
                        game,
                        [0, min_section - 1, 0],
                        true
                    ));
                    assert_eq!(marked(game), vec![min_section]);

                    assert!(ClientRenderClientHost::set_section_dirty(
                        game,
                        [0, 4, 0],
                        false
                    ));
                    assert_eq!(marked(game), vec![4]);

                    assert!(ClientRenderClientHost::set_section_dirty(
                        game,
                        [0, 4, 0],
                        true
                    ));
                    assert_eq!(marked(game), vec![3, 4, 5]);

                    ClientRenderClientHost::set_blocks_dirty(game, [15, 80, 9], [-20, 60, 3]);
                    assert_eq!(marked(game), vec![3, 4, 5]);
                    // A box on section edges: min X 16 is in column 1, but the block before it is in
                    // column 0; max Y 31 is in section 1, but the block above it is in section 2.
                    ClientRenderClientHost::set_blocks_dirty(game, [16, 20, 0], [20, 31, 0]);
                    assert_eq!(marked(game), vec![1, 2]);
                    // Away from the edges of one section: that section alone.
                    ClientRenderClientHost::set_blocks_dirty(game, [1, 65, 1], [14, 78, 14]);
                    assert_eq!(marked(game), vec![4]);
                    ClientRenderClientHost::set_blocks_dirty(
                        game,
                        [i32::MIN, i32::MIN, i32::MIN],
                        [i32::MAX, i32::MAX, i32::MAX],
                    );
                    assert_eq!(marked(game).len(), sections as usize);

                    ClientRenderClientHost::set_blocks_dirty(
                        game,
                        [-1_000_000, i32::MIN / 2, -1_000_000],
                        [1_000_000, i32::MAX / 2, 1_000_000],
                    );
                    assert_eq!(marked(game).len(), sections as usize);
                });
            },
        );
    }
}
