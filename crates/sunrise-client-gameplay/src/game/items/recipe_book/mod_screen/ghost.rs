//! Custom ghost slots share the vanilla painter, but never its slot-coordinate fallback.
use super::*;
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub(super) fn draw_mod_book_ghosts(&mut self, batch: &mut HudBatch) {
        let (ox, oy) = self.inventory_origin(batch);
        let ghosts: Vec<GhostView> = self
            .client
            .recipe_ui
            .0
            .ghost
            .iter()
            .filter_map(|ghost| {
                let slot = sunrise_gameplay_common::game::inventory_slots::container_slot_index(
                    ghost.slot,
                )?;
                let (x, y) = self.native_screen_slot(slot)?;
                let (x, y) = (ox + x, oy + y);
                // Clip in floating-point screen space before the shared integer
                // painter adds its padding. Offscreen mod coordinates never overflow it.
                if x + 16.0 < 0.0 || y + 16.0 < 0.0 || x > batch.width || y > batch.height {
                    return None;
                }
                Some(GhostView {
                    x: x as i32,
                    y: y as i32,
                    items: ghost.items.clone(),
                    result: ghost.result,
                })
            })
            .collect();
        let big_result = self.native_screen_big_result();
        let calls = screen::ghost(self.swap_index(), 0, 0, big_result, &ghosts);
        self.paint_book(batch, &calls);
    }
}
