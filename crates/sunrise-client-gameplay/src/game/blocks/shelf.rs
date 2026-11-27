//! Client rendering of the displayed items of `ShelfBlockEntity`.
use super::item_draw::{BlockItemDraw, block_entity_pose};
use sunrise_assets::item_model::DisplayContext;
use sunrise_client::{block_entity_render::vertex_light, camera::rotate_degrees, mesher::Vertex};
use sunrise_client_render::block_entity_reach::Reach;
use sunrise_core::position::BlockPos;
use sunrise_level::containers::BlockEntity;

impl crate::game::ClientGame<'_> {
    /// `ShelfRenderer`: three items face out, aligned to the bottom or centre.
    pub fn shelf_geometry(&self, camera: glam::DVec3, out: &mut Vec<Vertex>) {
        self.shelf_items(camera, |d| {
            self.submit_item_world(d.state, d.pose, d.light, out);
        });
    }

    /// The items [`Self::shelf_geometry`] draws, slot by slot.
    pub fn shelf_items(&self, camera: glam::DVec3, mut draw: impl FnMut(BlockItemDraw)) {
        let _items = sunrise_level::spans::raw_flight_guard("block_entity_items");
        let reach = Reach::Default;
        sunrise_level::spans::note_block_entity_item_visits(|| {
            self.common.world.block_entities.len() as u64
        });
        for (key, entity) in &self.common.world.block_entities {
            let BlockEntity::Shelf(shelf) = entity else {
                continue;
            };
            if !reach.should_render([key.0, key.1, key.2], camera, 0) {
                continue;
            }
            let block = self.common.world.block(key.0, key.1, key.2);
            let facing = self
                .common
                .pack
                .assets
                .registry
                .state(block)
                .and_then(|s| s.properties.get("facing"))
                .map(String::as_str)
                .unwrap_or("north");
            let yaw: f32 = match facing {
                "south" => -0.0,
                "west" => -90.0,
                "north" => -180.0,
                "east" => -270.0,
                _ => 180.0,
            };
            let packed = BlockPos::new(key.0, key.1, key.2).pack() as u64;
            let seed = (packed ^ (packed >> 32)) as i32;
            let light = vertex_light(self.common.world.light(key.0, key.1, key.2));
            let owner = super::item_draw::owner_at(
                [
                    f64::from(key.0) + 0.5,
                    f64::from(key.1) + 0.5,
                    f64::from(key.2) + 0.5,
                ],
                shelf_owner_yaw(facing),
                "",
            );
            for (slot, stack) in shelf.items.iter().enumerate() {
                let Some(stack) = stack else { continue };
                let seed = seed.wrapping_add(slot as i32);
                let item = self.resolve_item(stack, DisplayContext::OnShelf, Some(&owner), seed);
                if item.is_empty() {
                    continue;
                }
                let (lo, hi) = item.bounding_box();
                let (lo, hi) = (f64::from(lo.y), f64::from(hi.y));
                let mut offset_y = -lo;
                if !shelf.align_to_bottom {
                    offset_y += -(hi - lo) / 2.0;
                }
                let mut pose = block_entity_pose(*key, camera);
                pose.translate(0.5, 0.5, 0.5);
                rotate_degrees(&mut pose, 1, yaw);
                pose.translate(
                    (slot as i32 - 1) as f32 * 0.3125,
                    if shelf.align_to_bottom { -0.25 } else { 0.0 },
                    -0.25,
                );
                pose.scale(0.25, 0.25, 0.25);
                pose.translate(0.0, offset_y as f32, 0.0);
                draw(BlockItemDraw {
                    #[cfg(any(test, feature = "test-support"))]
                    stack,
                    #[cfg(any(test, feature = "test-support"))]
                    display: DisplayContext::OnShelf,
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

fn shelf_owner_yaw(facing: &str) -> f32 {
    match facing {
        "south" => 180.0,
        "west" => 270.0,
        "east" => 90.0,
        _ => 0.0,
    }
}
