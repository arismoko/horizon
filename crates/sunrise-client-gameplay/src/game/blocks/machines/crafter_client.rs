use super::*;
use sunrise_client::hud::{HudBatch, rgba};
use sunrise_protocol::packets::play::{self, Serverbound as S};

pub(in crate::game) const RESULT_SLOT: usize = 309;

impl crate::game::ClientGame<'_> {
    /// The grid's current recipe result (`CrafterMenu.refreshRecipeResult`).
    pub(in crate::game) fn crafter_result(&self) -> Option<ItemStack> {
        let key = self.common.container?;
        let c = self.common.crafter_at(key)?;
        self.recipes().craft(&self.common.loop_context, &c.items, 3)
    }
    /// `CrafterMenu` slot layout: the grid at (26, 17), the result at (134, 35).
    pub(in crate::game) fn crafter_slot_position(&self, i: usize) -> Option<(f32, f32)> {
        if !self.common.crafter_open() {
            return None;
        }
        match i {
            300..309 => Some((
                26.0 + ((i - 300) % 3) as f32 * 18.0,
                17.0 + ((i - 300) / 3) as f32 * 18.0,
            )),
            RESULT_SLOT => Some((134.0, 35.0)),
            _ => None,
        }
    }
    /// `CrafterScreen`'s background, disabled slots, powered arrow, and centred title.
    pub(in crate::game) fn draw_crafter_screen(&self, batch: &mut HudBatch, ox: f32, oy: f32) {
        let Some(c) = self
            .common
            .container
            .and_then(|k| self.common.crafter_at(k))
        else {
            return;
        };
        // CrafterScreen draws the arrow from `menu.isPowered()`: data slot 9, the int, is 1.
        let powered = c.is_triggered();
        let draws =
            sunrise_client::screen_layout::crafter(&sunrise_client::screen_layout::CrafterView {
                width: 176,
                height: 166,
                left: 0,
                top: 0,
                disabled: c.disabled,
                powered,
            });
        // The background goes first, then the labels, then the slot decorations.
        let (background, rest) = draws.split_at(2.min(draws.len()));
        self.paint_screen(batch, ox, oy, background, true);
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        let atlas = &self.client.gui.atlas;
        let grey = rgba(64, 64, 64, 255);
        // A server's crafter menu has the title it was given.
        let title = self.remote_menu_title().unwrap_or_else(|| {
            sunrise_game_data::text::Text::literal(
                self.container_title(self.translate_with_fallback("container.crafter", "Crafter")),
            )
            .resolve_legacy()
        });
        crate::game::screen_info::name_container(|| title.plain());
        let tw = atlas.font.width_text(&title);
        batch.text_styled(
            atlas,
            &title,
            ox + ((176.0 - tw) / 2.0).trunc(),
            oy + 6.0,
            grey,
            false,
        );
        let inventory = self.translate_with_fallback("container.inventory", "Inventory");
        batch.text(atlas, &inventory, ox + 8.0, oy + 72.0, grey, false);
        self.paint_screen(batch, ox, oy, rest, true);
    }
    /// `gui.togglable_slot` over an empty, enabled grid slot.
    pub(in crate::game::client_side) fn crafter_tooltip(&self, slot: usize) -> Option<String> {
        let c = self
            .common
            .container
            .and_then(|k| self.common.crafter_at(k))?;
        let i = slot.checked_sub(300).filter(|i| *i < 9)?;
        (self.common.carried.is_none()
            && c.items[i].is_none()
            && !c.disabled[i]
            && !self.common.spectator())
        .then(|| self.translate_with_fallback("gui.togglable_slot", "Click to disable slot"))
    }
    /// A click in the crafter menu: `CrafterMenu` on the verified menu engine.
    #[allow(dead_code)]
    pub(in crate::game) fn crafter_click(&mut self, slot: usize, shift: bool, right: bool) -> bool {
        if !self.common.crafter_open() {
            return false;
        }
        let input = if shift {
            sunrise_sim::menu::Input::QuickMove
        } else {
            sunrise_sim::menu::Input::Pickup
        };
        self.click_open_inventory(Some(slot), i32::from(right), input);
        true
    }
    /// `CrafterScreen.slotClicked`: an empty grid slot is enabled by a click
    /// (or a number key with a hotbar item) when disabled, and disabled by a
    /// click with nothing carried; the menu then gets the click itself.
    pub(in crate::game) fn crafter_slot_clicked(
        &mut self,
        slot: usize,
        button: i32,
        input: sunrise_sim::menu::Input,
    ) {
        use sunrise_sim::menu::Input;
        if !self.common.crafter_open() || !(300..309).contains(&slot) || self.common.spectator() {
            return;
        }
        let i = slot - 300;
        let Some(c) = self
            .common
            .container
            .and_then(|k| self.common.crafter_at(k))
        else {
            return;
        };
        if c.items[i].is_some() {
            return;
        }
        let disabled = c.disabled[i];
        let enable = match input {
            Input::Pickup if disabled => Some(true),
            Input::Pickup if self.common.carried.is_none() => Some(false),
            Input::Swap if disabled => usize::try_from(button)
                .ok()
                .and_then(|b| {
                    if b == 40 {
                        Some(&self.common.offhand)
                    } else {
                        self.common.inventory.get(b)
                    }
                })
                .is_some_and(Option::is_some)
                .then_some(true),
            _ => None,
        };
        let Some(enabled) = enable else {
            return;
        };
        // ServerboundContainerSlotStateChangedPacket: CrafterBlockEntity.setSlotState.
        self.menu_session(|menu, _, _| {
            menu.set_slot_state(i, enabled);
            Some(true)
        });
        if let Some((container, _)) = self
            .client
            .remote
            .as_ref()
            .and_then(|r| r.menu.as_ref().cloned())
        {
            self.send(S::ContainerSlotStateChanged(
                play::ContainerSlotStateChanged {
                    slot: i as i32,
                    container,
                    enabled,
                },
            ));
        }
        let pitch = if enabled { 1.0 } else { 0.75 };
        self.play_audio("minecraft:ui.button.click", None, 0.4, pitch);
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;
