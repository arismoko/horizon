//! The hotbar's `GuiGraphicsExtractor.extractSlot`: the item model followed by decorations.
use crate::game::ClientGame;
use sunrise_client::hud::HudBatch;
use sunrise_client::hud_layout::{GuiItems, HotbarInputs, hotbar, paint_items};
use sunrise_items::equipment::OFFHAND;
use sunrise_items::items::ItemStack;

/// Whose hotbar the slots come from: the local player, a live remote player, or a detached camera player.
#[derive(Clone, Copy)]
pub(crate) enum HotbarSource {
    Own,
    Mob(usize),
    DetachedCamera,
}

impl ClientGame<'_> {
    pub(in crate::game) fn draw_item_hotbar(
        &self,
        batch: &mut HudBatch,
        [w, h]: [i32; 2],
        inputs: &HotbarInputs,
        source: HotbarSource,
    ) {
        let draws = hotbar(w, h, inputs);
        paint_items(
            batch,
            &self.client.gui.atlas,
            &draws,
            &mut HotbarItems { game: self, source },
        );
    }
}

/// A hotbar's slots (0-8) and off hand (40), drawn for `GuiDraw::Item`.
struct HotbarItems<'a, 'g> {
    game: &'a ClientGame<'g>,
    source: HotbarSource,
}
impl HotbarItems<'_, '_> {
    fn stack(&self, slot: u32) -> Option<&ItemStack> {
        match (self.source, slot) {
            (HotbarSource::Own, 40) => self.game.offhand.as_ref(),
            (HotbarSource::Own, 0..=8) => self.game.inventory.get(slot as usize)?.as_ref(),
            (HotbarSource::Mob(i), 40) => self.game.mobs[i].extra.slot_stack(OFFHAND),
            (HotbarSource::Mob(i), 0) => self.game.mobs[i].extra.main_hand_stack(),
            (HotbarSource::DetachedCamera, 40) => self
                .game
                .detached_camera_hud()
                .and_then(|hud| hud.offhand.as_ref()),
            (HotbarSource::DetachedCamera, 0) => self
                .game
                .detached_camera_hud()
                .and_then(|hud| hud.main_hand.as_ref()),
            _ => None,
        }
    }
}
impl GuiItems for HotbarItems<'_, '_> {
    fn item(&mut self, batch: &mut HudBatch, slot: u32, x: i32, y: i32, seed: i32) {
        if let Some(stack) = self.stack(slot) {
            self.game
                .draw_item_model(batch, stack, x as f32, y as f32, seed);
        }
    }
    fn decorations(&mut self, batch: &mut HudBatch, slot: u32, x: i32, y: i32) {
        if let Some(stack) = self.stack(slot) {
            self.game
                .draw_item_decorations(batch, stack, x as f32, y as f32, None);
        }
    }
}
