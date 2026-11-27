use super::item_draw::{BlockItemDraw, block_entity_pose};
use sunrise_assets::item_model::DisplayContext;
use sunrise_client::{block_entity_render::vertex_light, camera::rotate_degrees, mesher::Vertex};
use sunrise_client_render::block_entity_reach::Reach;
use sunrise_core::position::BlockPos;
use sunrise_level::containers::BlockEntity;

impl crate::game::ClientGame<'_> {
    /// `CampfireRenderer`: each cooking item lies on the fire, slot `i`
    /// turned to `Direction.from2DDataValue((i + facing) % 4)`, laid flat
    /// (90° about X), at a corner and three eighths of its size, drawn as a
    /// FIXED item seeded by the block's position plus the slot.
    pub fn campfire_geometry(&self, camera: glam::DVec3, out: &mut Vec<Vertex>) {
        self.campfire_items(camera, |d| {
            self.submit_item_world(d.state, d.pose, d.light, out);
        });
    }
    /// The items [`Self::campfire_geometry`] draws, in `CampfireRenderer.submit`'s order.
    pub fn campfire_items(&self, camera: glam::DVec3, mut draw: impl FnMut(BlockItemDraw)) {
        let _items = sunrise_level::spans::raw_flight_guard("block_entity_items");
        let reach = Reach::Default;
        sunrise_level::spans::note_block_entity_item_visits(|| {
            self.common.world.block_entities.len() as u64
        });
        for (key, entity) in &self.common.world.block_entities {
            let BlockEntity::Campfire(c) = entity else {
                continue;
            };
            if c.items.iter().all(Option::is_none) {
                continue;
            }
            if !reach.should_render([key.0, key.1, key.2], camera, 0) {
                continue;
            }
            // Direction.get2DDataValue: south 0, west 1, north 2, east 3.
            let state = self.common.world.block(key.0, key.1, key.2);
            let facing = match self
                .common
                .pack
                .assets
                .registry
                .state(state)
                .and_then(|s| s.properties.get("facing"))
                .map(String::as_str)
            {
                Some("west") => 1,
                Some("north") => 2,
                Some("east") => 3,
                _ => 0,
            };
            let light = vertex_light(self.common.world.light(key.0, key.1, key.2));
            let packed = BlockPos::new(key.0, key.1, key.2).pack();
            for (i, stack) in c.items.iter().enumerate() {
                let Some(stack) = stack else {
                    continue;
                };
                let seed = (packed as i32).wrapping_add(i as i32);
                let item = self.resolve_item(stack, DisplayContext::Fixed, None, seed);
                if item.is_empty() {
                    continue;
                }
                let mut pose = block_entity_pose(*key, camera);
                pose.translate(0.5, 0.449_218_75, 0.5);
                // Direction.toYRot: 90° per 2D step from south.
                rotate_degrees(&mut pose, 1, -(((i + facing) % 4) as f32 * 90.0));
                rotate_degrees(&mut pose, 0, 90.0);
                pose.translate(-0.3125, -0.3125, 0.0);
                pose.scale(0.375, 0.375, 0.375);
                draw(BlockItemDraw {
                    #[cfg(any(test, feature = "test-support"))]
                    stack,
                    #[cfg(any(test, feature = "test-support"))]
                    display: DisplayContext::Fixed,
                    #[cfg(any(test, feature = "test-support"))]
                    seed,
                    state: &item,
                    pose: pose.to_glam(),
                    light,
                });
            }
        }
    }
}
