// game-module-visibility: pub
//! Client draw operations through the existing GUI drawing context.

use super::Game;
use horizon_client::{
    hud::{HudBatch, rgba},
    hud_layout::GuiDraw,
};
use horizon_items::items::ItemStack;
use mod_api::Text;

impl Game<'_> {
    /// Paint GUI extraction calls through this window's atlas, including player faces.
    pub(super) fn native_gui_paint(&self, batch: &mut HudBatch, draws: &[GuiDraw]) {
        horizon_client::hud_layout::paint(batch, &self.gui.atlas, draws);
    }
    /// Vanilla's nine-slice widget rendering. `color` is ARGB, as `fill` uses.
    pub(super) fn native_gui_nine_slice(
        &self,
        batch: &mut HudBatch,
        sprite: &str,
        rect: [f32; 4],
        color: u32,
    ) {
        let [a, r, g, b] = color.to_be_bytes();
        horizon_client::ui::nine_slice(
            batch,
            &self.gui.atlas,
            sprite,
            rect[0],
            rect[1],
            rect[2],
            rect[3],
            rgba(r, g, b, a),
        );
    }
    /// A vanilla tooltip at the cursor, in GUI pixels.
    pub(super) fn native_gui_tooltip(&self, batch: &mut HudBatch, lines: &[Text], at: [f32; 2]) {
        horizon_client::ui::tooltip(batch, &self.gui.atlas, lines, (at[0], at[1]));
    }

    /// Draw an item stack with custom count/decoration text (or None) at `at` in GUI units.
    pub(super) fn native_gui_draw_item_with(
        &self,
        batch: &mut HudBatch,
        stack: &ItemStack,
        at: [f32; 2],
        text: Option<&str>,
    ) {
        self.draw_item_model(batch, stack, at[0], at[1]);
        self.draw_item_decorations(batch, stack, at[0], at[1], text.map(str::to_owned));
    }
    /// Whether a specific key is currently held down.
    /// `key` is the vanilla physical key name (`key.keyboard.left.shift`, for example).
    /// Unknown names are not down. Modifier combinations use `ClientHost::modifiers`.
    pub(super) fn is_key_down(&self, key: &str) -> bool {
        horizon_client::keybinds::key_code(key).is_some_and(|key| self.physical_keys.contains(&key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn tooltip_helpers_read_live_keys_and_draw_count_overrides() {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            use winit::keyboard::KeyCode;

            game.physical_keys.insert(KeyCode::ShiftLeft);
            assert!(game.is_key_down("key.keyboard.left.shift"));
            assert!(!game.is_key_down("key.keyboard.right.shift"));
            assert!(!game.is_key_down("not-a-key"));
            game.physical_keys.clear();
            assert!(!game.is_key_down("key.keyboard.left.shift"));

            let stack = ItemStack::new("minecraft:diamond", 5);
            let mut automatic = HudBatch::scratch([320, 240]);
            game.native_gui_draw_item_with(&mut automatic, &stack, [16.0, 32.0], None);
            let mut custom = HudBatch::scratch([320, 240]);
            game.native_gui_draw_item_with(&mut custom, &stack, [16.0, 32.0], Some("7"));
            assert!(!automatic.vertices.is_empty());
            assert_ne!(
                bytemuck::cast_slice::<_, u8>(&automatic.vertices),
                bytemuck::cast_slice::<_, u8>(&custom.vertices),
                "custom count text must reach the actual GUI geometry"
            );
        });
    }
}
