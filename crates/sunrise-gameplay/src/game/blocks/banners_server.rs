use super::maps::MapBanner;
use sunrise_gameplay_common::game::banners::map_banner;

impl crate::game::ServerGame<'_> {
    /// The banner at `pos` as a map marker source (`MapBanner.fromWorld`).
    pub(in crate::game::server_side) fn map_banner_at(&self, pos: [i32; 3]) -> Option<MapBanner> {
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        map_banner(
            pos,
            self.common
                .world
                .block_entities
                .get(&(pos[0], pos[1], pos[2])),
            &self.common.res.facts().get(state).name,
        )
    }
}
