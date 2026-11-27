//! Titles, the action bar and the selected item name: the state and layout
//! live in `hud_overlays` (`Titles`, `ToolHighlight`); this feeds and draws them.
use crate::game::ClientGame;
#[allow(unused_imports)]
pub(in crate::game) use sunrise_client::hud_overlays::Titles;
use sunrise_client::{hud::HudBatch, hud_layout, hud_overlays::HeldName};
use sunrise_game_data::text::Text;

impl ClientGame<'_> {
    #[inline(always)]
    pub(in crate::game::client_side) fn set_hud_overlay(&mut self, message: Text) {
        self.client.titles.set_overlay(message, false);
    }
    /// `Hud.setOverlayMessage(line, animate_color)` on this window's HUD, outside a borrowed
    /// server guest's turn: no chat, narration or packet, as the direct setter does none.
    pub(in crate::game) fn set_overlay_message(&mut self, line: Text, animate_color: bool) {
        if self.common.native_window() {
            self.client.titles.set_overlay(line, animate_color);
        }
    }

    /// `Hud.extractOverlayMessage`: the action bar.
    pub(in crate::game::client_side) fn draw_overlay_message(&self, batch: &mut HudBatch) {
        let (w, h) = (batch.width as i32, batch.height as i32);
        let partial = self.common.clock.fraction() as f32;
        let background = self.client.settings.text_background(0.0);
        let font = &self.client.gui.atlas.font;
        let draws = self
            .client
            .titles
            .overlay_message(w, h, partial, font, background);
        hud_layout::paint(batch, &self.client.gui.atlas, &draws);
    }
    /// `Hud.extractTitle`.
    pub(in crate::game::client_side) fn draw_title(&self, batch: &mut HudBatch) {
        let (w, h) = (batch.width as i32, batch.height as i32);
        let partial = self.common.clock.fraction() as f32;
        let background = self.client.settings.text_background(0.0);
        let font = &self.client.gui.atlas.font;
        let draws = self.client.titles.title(w, h, partial, font, background);
        hud_layout::paint(batch, &self.client.gui.atlas, &draws);
    }
    /// `Hud.extractSavingIndicator`, under the screens: the option, then the
    /// fading "Saving world" text.
    pub(in crate::game::client_side) fn draw_saving_indicator(&self, batch: &mut HudBatch) {
        if !self.client.settings.autosave_indicator {
            return;
        }
        let draws = self.client.saving_indicator.draw(
            batch.width as i32,
            batch.height as i32,
            self.common.clock.fraction() as f32,
            &self.client.gui.atlas.font,
            Text::literal(self.translate_with_fallback("menu.savingLevel", "Saving world")),
            self.client.settings.text_background(0.0),
        );
        hud_layout::paint(batch, &self.client.gui.atlas, &draws);
    }
    /// The selected stack as `Hud.tick` compares it for the name highlight.
    pub(in crate::game::client_side) fn held_name(&self) -> Option<HeldName> {
        let stack = self.common.inventory.get(self.common.selected)?.as_ref()?;
        let custom = stack.data().is_some_and(|data| data.custom_name.is_some());
        let previous = self
            .client
            .tool_highlight
            .last
            .as_ref()
            .map(|last| &last.hover_name);
        Some(HeldName {
            item: stack.item.to_string(),
            hover_name: sunrise_sim::stations::hover_name_in_reusing(
                &self.common.loop_context,
                &self.common.pack.names,
                stack,
                previous,
            ),
            rarity_color: self.stack_rarity_color(stack),
            custom_name: custom,
        })
    }
    /// The title, action bar and selected-item parts of `Hud.tick`.
    pub fn tick_hud(&mut self) {
        self.client.titles.tick();
        // Hud.tick: the vignette after the titles, before the held item's name. (A local world,
        // which has no `Gui.tick` here, eases it in its level tick, `tick_particles`.)
        self.tick_vignette();
        self.tick_in_bed_chat();
        // `MinecraftServer.isCurrentlySaving` of single player's server.
        let saving = self
            .client
            .integrated
            .as_ref()
            .map_or(self.common.autosaving, |server| server.is_saving());
        self.client.saving_indicator.tick(saving);
        let held = self.held_name();
        self.client
            .tool_highlight
            .tick(held, self.client.settings.notification_display_time);
        self.client.tool_highlight.tip = self.highlight_tip();
    }
    /// `IItemExtension.getHighlightTip` of the selected stack, while its name
    /// shows and a part of its item listens.
    pub(in crate::game::client_side) fn highlight_tip(&self) -> Option<Text> {
        let last = self.client.tool_highlight.last.as_ref()?;
        if self.client.tool_highlight.timer <= 0 {
            return None;
        }
        let stack = self.common.inventory.get(self.common.selected)?.as_ref()?;
        if !mod_api::__private::item_listens(mod_api::HookId::HighlightTip, &stack.item) {
            return None;
        }
        let mut name = last.styled();
        self.native_highlight_tip(stack, &mut name);
        Some(name)
    }
    /// `Hud.extractSelectedItemName`.
    pub(in crate::game::client_side) fn draw_selected_name(
        &self,
        batch: &mut HudBatch,
        can_hurt: bool,
    ) {
        let draws = self.client.tool_highlight.draw(
            batch.width as i32,
            batch.height as i32,
            can_hurt,
            &self.client.gui.atlas.font,
            self.client.settings.text_background(0.0),
        );
        hud_layout::paint(batch, &self.client.gui.atlas, &draws);
    }
}

use sunrise_gameplay_common::game::stack_names::StackNamesGameExt1 as _;
