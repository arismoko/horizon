use crate::game::ClientGame;
use mod_api::{HookId, Identifier, PlayerModel, PlayerSkinOverride};

/// The atlas skin slot the selected dynamic texture is copied into.
pub(in crate::game) struct PlayerSkinSlot {
    texture: Identifier,
    /// The key the slot is held under in `skin_slots`.
    location: String,
    slot: usize,
    /// The slot's sprite name, `sunrise/skin{slot}`.
    name: String,
    /// The slot's texel origin in the atlas ([`HudAtlas::skin_origins`]).
    origin: [u32; 2],
    /// [`HudAtlas::revision`] it was resolved against.
    atlas: u64,
}

impl ClientGame<'_> {
    /// Install the resolved appearance on the render-only avatar and return the original pair.
    pub(crate) fn apply_player_skin_override(
        &mut self,
        panorama: bool,
    ) -> Option<(Option<String>, bool)> {
        if !mod_api::__private::listened(HookId::ResolvePlayerSkin)
            || panorama
            || self.common.player.is_none()
            || self.client.player_model.is_none()
        {
            return None;
        }
        let mut selection = None;
        self.native_resolve_player_skin(&mut selection);
        let PlayerSkinOverride { texture, model, .. } = selection?;
        // The body, the hand and the preview sample one packed atlas, so the 64 x 64 dynamic
        // texture is copied on the GPU into a skin slot of it, every frame it is selected
        // (as `skin_slots` hands slots out for downloaded skins). Anything else keeps vanilla.
        if self.client.dynamic_textures.size(&texture) != Some([64, 64]) {
            return None;
        }
        // The slot, its sprite name and its texel origin are made again only when the selection
        // changes, the slot is given to another skin, or the atlas is rebuilt.
        let held = self.client.player_skin_slot.as_ref().is_some_and(|held| {
            held.texture == texture
                && held.atlas == self.client.gui.atlas.revision
                && self.client.skin_slots.slot(&held.location) == Some(held.slot)
        });
        if !held {
            let location = format!("dynamic:{texture}");
            let (slot, _) = self.client.skin_slots.place(location.clone())?;
            let origin = *self.client.gui.atlas.skin_origins.get(slot)?;
            self.client.player_skin_slot = Some(PlayerSkinSlot {
                texture: texture.clone(),
                location,
                slot,
                name: format!("sunrise/skin{slot}"),
                origin,
                atlas: self.client.gui.atlas.revision,
            });
        }
        let held = self.client.player_skin_slot.as_ref()?;
        let (name, origin) = (held.name.clone(), held.origin);
        self.client.dynamic_textures.copy_to_atlas(&texture, origin);
        let avatar = self.client.player_model.as_mut()?;
        Some((
            avatar.skin.replace(name),
            std::mem::replace(&mut avatar.slim, model == PlayerModel::Slim),
        ))
    }

    #[cfg(any(test, feature = "test-support"))]
    #[inline(always)]
    pub fn test_skin_slot_name(&mut self, texture: &Identifier) -> String {
        let (slot, _) = self
            .client
            .skin_slots
            .place(format!("dynamic:{texture}"))
            .expect("a skin slot");
        format!("sunrise/skin{slot}")
    }
}
