//! The non-pausing F3+F4 screen (`GameModeSwitcherScreen`).
use super::{ItemStack, Screen};
use sunrise_client::{
    game_mode_switcher::{GameModeSwitcher, ICONS, MODES, slot_origin},
    hud::HudBatch,
    keybinds,
};
use sunrise_game_data::text::Text;

impl crate::game::ClientGame<'_> {
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_open_gamemode_switcher(&mut self) {
        self.open_gamemode_switcher();
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_close_gamemode_switcher(&mut self, apply: bool) {
        self.close_gamemode_switcher(apply);
    }

    pub(in crate::game::client_side) fn open_gamemode_switcher(&mut self) {
        self.client.gamemode_switcher = Some(GameModeSwitcher::new(
            self.common.mode,
            self.common.previous_mode,
        ));
        self.set_screen(Screen::GameModeSwitcher);
        self.clear_device_keys();
        self.set_mapping_held("key.attack", false);
        self.set_mapping_held("key.use", false);
        self.set_client_mining(None);
        self.capture(false);
    }

    pub(in crate::game::client_side) fn close_gamemode_switcher(&mut self, apply: bool) {
        let selected = self.client.gamemode_switcher.take().map(|s| s.selected);
        self.set_screen(Screen::Playing);
        self.capture(true);
        if apply
            && self.common.player.is_some()
            && self.gamemaster()
            && let Some(mode) = selected
            && mode != self.common.mode
        {
            self.change_game_mode(mode);
        }
    }

    /// `GameModeSwitcherScreen.mouseReleased`: releasing the debug
    /// modifier's own mouse button applies the hovered mode and closes the
    /// screen, as the modifier key's release does (`keyReleased`). A modifier
    /// bound to a keyboard key matches no mouse button.
    pub(in crate::game::client_side) fn switcher_release(
        &mut self,
        button: winit::event::MouseButton,
    ) -> bool {
        if sunrise_client::game_mode_switcher::release_applies(
            &keybinds::mouse_name(button),
            self.client.settings.keys.get("key.debug.modifier"),
        ) {
            self.close_gamemode_switcher(true);
            return true;
        }
        false
    }

    pub(in crate::game::client_side) fn draw_gamemode_switcher(&mut self, batch: &mut HudBatch) {
        let (w, h) = (batch.width as i32, batch.height as i32);
        let Some(switcher) = &mut self.client.gamemode_switcher else {
            self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
            self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
            return;
        };
        // GameModeSwitcherScreen: `GameNarrator.NO_TITLE`; its slots are no
        // widgets, so it says how to navigate.
        sunrise_client::widgets::begin_screen(|| {
            sunrise_client::widgets::ScreenInfo::new("gamemode_switcher", "")
        });
        switcher.hover(
            w,
            h,
            (self.client.cursor.0 as i32, self.client.cursor.1 as i32),
        );
        let selected = switcher.selected;
        let atlas = &self.client.gui.atlas;
        batch.sprite_region(
            atlas,
            "gui/container/gamemode_switcher",
            [(w / 2 - 62) as f32, (h / 2 - 58) as f32, 125.0, 75.0],
            128.0,
            128.0,
        );
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        let title = Text::translate(&format!("gameMode.{}", selected.name()), &[]);
        let help = Text::translate(
            "debug.gamemodes.select_next",
            &[
                keybinds::display_name(self.client.settings.keys.get("key.debug.switchGameMode"))
                    .with_color(0x55FFFF),
            ],
        );
        for (text, y) in [(title, h / 2 - 51), (help, h / 2 + 5)] {
            let x = w / 2 - (HudBatch::text_width(atlas, &text.plain()) / 2.0) as i32;
            batch.text_styled(atlas, &text, x as f32, y as f32, u32::MAX, true);
        }
        for (index, mode) in MODES.into_iter().enumerate() {
            let (x, y) = slot_origin(w, h, index);
            batch.sprite(
                &self.client.gui.atlas,
                "gamemode_switcher/slot",
                x as f32,
                y as f32,
            );
            if mode == selected {
                batch.sprite(
                    &self.client.gui.atlas,
                    "gamemode_switcher/selection",
                    x as f32,
                    y as f32,
                );
            }
            self.draw_item_model(
                batch,
                &ItemStack::new(ICONS[index], 1),
                (x + 5) as f32,
                (y + 5) as f32,
                0,
            );
        }
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
    }
}
