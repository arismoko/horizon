//! The Quick Actions dialog (`key.quickActions`, default G), read the way
//! `Minecraft.getQuickActionsDialog` reads it from the server's synchronized
//! `minecraft:dialog` registry and `minecraft:quick_actions` tag:
//!
//! - an empty tag opens nothing,
//! - one dialog opens itself,
//! - several open the `minecraft:quick_actions` dialog (a `dialog_list` over
//!   the same tag).
//!
//! `handleKeybinds` then shows it with `showDialog(holder, gui.screen())`; the
//! data comes from the pack stack, so a world's data packs supply it.
use crate::game::ClientGame;
use sunrise_core::nbt::Tag;

impl ClientGame<'_> {
    /// `Minecraft.getQuickActionsDialog`: the dialogs the tag names, from
    /// `RegistryAccess.lookupOrThrow(Registries.DIALOG)`.
    fn quick_actions_dialog(&self) -> Option<Tag> {
        let registries = &self.client.remote.as_ref()?.registries;
        let dialogs = registries.registries.get("minecraft:dialog")?;
        let tag = registries.tag("minecraft:dialog", "minecraft:quick_actions")?;
        match tag.len() {
            0 => None,
            1 => {
                let i = usize::try_from(tag[0]).ok()?;
                dialogs.get(i)?.1.clone()
            }
            // `Dialogs.QUICK_ACTIONS`: the dialog list over the same tag.
            _ => Some(
                dialogs
                    .iter()
                    .find(|(name, _)| name == "minecraft:quick_actions")?
                    .1
                    .clone()?,
            ),
        }
    }

    /// `key.quickActions`: only open if the tag names a dialog.
    pub(in crate::game::client_side) fn has_quick_actions(&self) -> bool {
        self.quick_actions_dialog().is_some()
    }

    /// `getQuickActionsDialog().ifPresent(showDialog(holder, gui.screen()))`.
    /// Returns whether a dialog screen actually opened.
    pub(in crate::game::client_side) fn open_quick_actions(&mut self) -> bool {
        let Some(dialog) = self.quick_actions_dialog() else {
            return false;
        };
        let before_rev = self.dialog_screen_key();
        let before_screen = self.client.screen;
        self.show_dialog(&dialog);
        self.dialog_screen_key() != before_rev || self.client.screen != before_screen
    }
}
