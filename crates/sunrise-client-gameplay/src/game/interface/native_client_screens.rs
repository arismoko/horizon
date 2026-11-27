//! A client mod's screen control (`ClientHost::close_screen` and
//! `ClientHost::open_inventory`) and the sign it edits
//! (`ClientHost::sign_editor`): the same paths Escape and the inventory key
//! take, so the packets and follow-ups are vanilla's.
use crate::game::ClientGame;
use mod_api::{BlockPos, SignEditView};
use sunrise_gameplay_common::game::Screen;
use winit::keyboard::KeyCode;

impl ClientGame<'_> {
    /// `Minecraft.setScreen` through Escape for a container screen, and the
    /// screen's own close for any other: Escape closes a container with the
    /// menu's `ServerboundContainerClosePacket` first (a bare local close would
    /// leave the server's menu open), while a pause menu closes to the game.
    /// The sign editor and a book are overlays beside the screen: Escape on the sign editor
    /// sends its `ServerboundSignUpdatePacket`, and on a lectern's book the menu's close. A
    /// confirmation, a server's pack prompt or a dialog opened over any of them handles
    /// Escape first. Declining a required pack disconnects rather than returning to the
    /// screen underneath. False when no screen is open, or the foreground screen does not
    /// close with Escape.
    pub fn close_screen(&mut self) -> bool {
        if self.common.player.is_none() {
            return false;
        }
        if let Some(closed) = self.close_foreground() {
            return closed;
        }
        if self.client.sign_edit.is_some() {
            self.close_sign_editor();
            return true;
        }
        // A book or a lectern's book is the same kind of overlay: its screen's onClose (a
        // lectern's closes its menu, sending the container close).
        if self.client.book.is_some() {
            self.close_book();
            return true;
        }
        if self.client.screen == Screen::Playing {
            return false;
        }
        let before = self.client.screen;
        // The Escape path itself, so every screen's own close and follow-ups
        // (and the screen hooks' closing notices) are exactly the key's.
        let _ = self.key_pressed(KeyCode::Escape, KeyCode::Escape);
        self.client.screen != before
    }
    /// Escape on a screen opened in front of the others, in the order `window_event` gives
    /// it the key: the death-exit confirmation answers no after its delay, a command's or
    /// link's confirmation answers no, `CodeOfConductScreen` stays, `PackConfirmScreen`
    /// declines (disconnecting for a required pack), and a dialog runs its own Escape.
    /// Returns whether it closed, or `None` when no such screen is open.
    fn close_foreground(&mut self) -> Option<bool> {
        if self.client.screen == Screen::Dead
            && let Some(shown) = self.client.chat_ui.death_confirm
        {
            if shown.elapsed().as_millis() >= 1000
                && let Err(e) = self.death_confirm_answer(false)
            {
                log::warn!("Sunrise: closing the death-exit confirmation: {e:#}");
            }
            return Some(self.client.chat_ui.death_confirm.is_none());
        }
        if self.client.command_prompt.0.is_some() {
            self.command_prompt_key(KeyCode::Escape);
            return Some(self.client.command_prompt.0.is_none());
        }
        if self.client.link_prompt.is_some() {
            self.answer_link_prompt(false);
            return Some(true);
        }
        if self.client.code_of_conduct.is_some() {
            return Some(false);
        }
        if self.client.server_packs.0.prompt.is_some() {
            if let Err(e) = self.answer_pack_prompt(false) {
                log::warn!("Sunrise: declining the server's resource pack: {e:#}");
            }
            return Some(true);
        }
        if self.dialog_open() {
            let before = self.dialog_screen_key();
            if let Err(e) = self.dialog_key(KeyCode::Escape, None) {
                log::warn!("Sunrise: closing the dialog: {e:#}");
            }
            return Some(!self.dialog_open() || self.dialog_screen_key() != before);
        }
        None
    }
    /// `Minecraft.handleKeybinds` for `key.inventory`: the player's inventory
    /// opens locally and nothing is sent (the player menu is always open on the
    /// server), or, riding a vehicle with its own inventory, the key queues the
    /// open-inventory player command instead (`ServerboundPlayerCommandPacket`,
    /// sent at the player's next tick as the key's press is). False with a
    /// screen already open, for a spectator, in a portal transition, or with no
    /// world.
    pub fn open_inventory(&mut self) -> bool {
        if self.common.player.is_none()
            || self.client.screen != Screen::Playing
            || self.client.sign_edit.is_some()
            || self.common.spectator()
            || self.in_portal_transition()
        {
            return false;
        }
        if self.client.remote.is_some()
            && (self.riding_mount_with_menu() || self.common.riding_chest_boat())
        {
            self.queue_mapping_click("key.inventory");
            return true;
        }
        self.open_inventory_key_action();
        self.client.screen != Screen::Playing
    }
    /// `AbstractSignEditScreen`'s sign: its block position and the side the
    /// editor is on. None when no sign editor is open, or outside a world.
    pub fn sign_editor(&self) -> Option<SignEditView> {
        self.common.player.as_ref()?;
        let edit = self.client.sign_edit.as_ref()?;
        Some(SignEditView {
            pos: BlockPos::new(edit.pos.0, edit.pos.1, edit.pos.2),
            front: edit.front,
        })
    }
}
