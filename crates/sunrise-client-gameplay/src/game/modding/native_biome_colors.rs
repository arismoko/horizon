//! Biome colours a client mod changes (`native_biome_colors`), for seasons:
//! the grass, foliage, dry foliage and water tints the mesher blends, as a
//! resource pack's colormaps and biome effects set them, changed while the
//! game runs.

use crate::game::ClientGame;
use std::sync::Arc;

// Native mods call it; nothing in Sunrise does.
impl ClientGame<'_> {
    /// Asks `native_biome_colors` again for every biome and, when the colours
    /// differ from those in use, rebuilds every loaded section and the far
    /// terrain with them (`LevelRenderer.allChanged`, as changing Biome Blend
    /// does). Call it when your colours change (a new season, a new day of a
    /// slow fade) and once when the player joins, not every frame: a rebuild
    /// takes the workers a few seconds.
    pub(in crate::game) fn refresh_biome_colors(&mut self) {
        // A world's own table is asked from the biome files' colours each
        // time, so a mod's colours do not stack; a plain world's is the pack's.
        if let (Some(base), Some(current)) =
            (&self.common.world_biomes_base, &self.common.world_biomes)
        {
            let Some(biomes) = base.recolored_from(current, |biome, colors| {
                self.native_biome_colors(biome, colors)
            }) else {
                return;
            };
            self.common.world_biomes = Some(Arc::new(biomes));
        } else {
            let base = self.common.res.biomes().clone();
            let biomes = base
                .with_colors(|biome, colors| self.native_biome_colors(biome, colors))
                .map_or_else(|| base.clone(), Arc::new);
            if Arc::ptr_eq(&biomes, &self.client.client_pack.context.chunk.biomes) {
                return;
            }
            self.client.client_pack.context = self.client.client_pack.context_with(biomes);
        }
        let context = self.worker_context();
        self.side_clear_lod();
        self.client
            .streaming
            .reload_resources(&mut self.common.world, context);
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;
