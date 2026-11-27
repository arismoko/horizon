use crate::game::ClientGame;
use mod_api::HookId;

impl ClientGame<'_> {
    /// Only a subscribed client lays out substituted images. Atlas, dynamic sprite geometry and
    /// mod toggles can each invalidate layout, but updating a texture's pixels does not.
    pub(crate) fn inline_images_revision(&self) -> Option<[u64; 3]> {
        (self.common.native_window()
            && mod_api::__private::enabled_listened(HookId::SubstituteInlineImages))
        .then(|| {
            [
                self.client.gui.atlas.revision,
                self.client.dynamic_textures.geometry_revision(),
                mod_api::__private::mods_switched(),
            ]
        })
    }
}
