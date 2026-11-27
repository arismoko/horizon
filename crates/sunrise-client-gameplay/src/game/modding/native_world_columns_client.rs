//! Client-side streaming column height reads for native mods.
use crate::game::ClientGame;
use std::ops::ControlFlow;
use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;

impl ClientGame<'_> {
    /// Visit each column's height by `heightmap` in the client's open level, `None` where its
    /// chunk is not loaded. Loads nothing.
    pub(in crate::game) fn level_visit_surface_heights(
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
}
