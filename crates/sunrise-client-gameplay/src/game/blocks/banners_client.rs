use crate::game::ClientGame;
use sunrise_gameplay_common::game::banners::layer_textures;
use sunrise_level::containers::BlockEntity;

impl ClientGame<'_> {
    /// The pattern textures and tints of the banner at `pos`, bottom first.
    pub(in crate::game) fn banner_layers_at(&self, pos: [i32; 3]) -> Vec<(String, u32)> {
        let Some(BlockEntity::Banner(b)) = self
            .common
            .world
            .block_entities
            .get(&(pos[0], pos[1], pos[2]))
        else {
            return Vec::new();
        };
        layer_textures(&self.stations().banners, &b.layers)
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;
