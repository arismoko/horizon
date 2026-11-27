//! Client draw operations through the existing GUI drawing context.

use crate::game::ClientGame;
use sunrise_client::hud::{HudBatch, rgba};
use sunrise_game_data::text::Text;
use sunrise_items::items::ItemStack as EngineStack;

impl ClientGame<'_> {
    /// `GuiGraphics.fillGradient` for [`mod_api::DrawContext::fill_gradient`]: `rect` is
    /// `[x, y, w, h]` and `top` / `bottom` are ARGB at `y` / `y + h`. Zero area,
    /// one-axis inversion or non-finite input draws nothing; both axes inverted flips the colours.
    pub(in crate::game) fn native_fill_gradient(
        &self,
        batch: &mut HudBatch,
        rect: [f32; 4],
        top: u32,
        bottom: u32,
    ) {
        if rect.into_iter().all(f32::is_finite) {
            sunrise_client::hud_layout::paint_fill_gradient(
                batch,
                &self.client.gui.atlas,
                rect,
                top,
                bottom,
            );
        }
    }
    /// Vanilla's nine-slice widget rendering. `color` is ARGB, as `fill` uses.
    pub(in crate::game) fn native_gui_nine_slice(
        &self,
        batch: &mut HudBatch,
        sprite: &str,
        rect: [f32; 4],
        color: u32,
    ) {
        let [a, r, g, b] = color.to_be_bytes();
        sunrise_client::ui::nine_slice(
            batch,
            &self.client.gui.atlas,
            sprite,
            rect[0],
            rect[1],
            rect[2],
            rect[3],
            rgba(r, g, b, a),
        );
    }
    /// A vanilla tooltip at the cursor, in GUI pixels.
    pub(in crate::game) fn native_gui_tooltip(
        &self,
        batch: &mut HudBatch,
        lines: &[Text],
        at: [f32; 2],
    ) {
        sunrise_client::ui::tooltip(batch, &self.client.gui.atlas, lines, (at[0], at[1]));
    }

    /// Draw an item stack with custom count/decoration text (or None) at `at` in GUI units.
    pub(in crate::game) fn native_gui_draw_item_with(
        &self,
        batch: &mut HudBatch,
        stack: &EngineStack,
        at: [f32; 2],
        text: Option<&str>,
    ) {
        self.draw_item_model(batch, stack, at[0], at[1], 0);
        self.draw_item_decorations(batch, stack, at[0], at[1], text);
    }
    /// Whether a specific key is currently held down.
    /// `key` is the vanilla physical key or mouse button name (`key.keyboard.left.shift`,
    /// `key.mouse.left`, for example). Unknown names are not down. Modifier combinations use
    /// `ClientHost::modifiers`.
    pub fn is_key_down(&self, key: &str) -> bool {
        match sunrise_client::keybinds::key_code(key) {
            Some(key) => self.client.physical_keys.contains(&key),
            None => sunrise_client::keybinds::mouse_button(key)
                .is_some_and(|button| self.client.physical_buttons.contains(&button)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn tooltip_helpers_read_live_keys_and_draw_count_overrides() {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            use winit::keyboard::KeyCode;

            game.client.physical_keys.insert(KeyCode::ShiftLeft);
            assert!(game.is_key_down("key.keyboard.left.shift"));
            assert!(!game.is_key_down("key.keyboard.right.shift"));
            assert!(!game.is_key_down("not-a-key"));
            game.client.physical_keys.clear();
            assert!(!game.is_key_down("key.keyboard.left.shift"));

            let stack = EngineStack::new("minecraft:diamond", 5);
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
