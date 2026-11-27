use crate::game::{BlockEntityDraw, MobRender, PlayerRender};
use sunrise_gameplay_common::game::native_hooks::FrameModelEdit;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Before a player is drawn (NeoForge's `RenderPlayerEvent.Pre`): once a frame for each other
    /// player in view and this window's player in third person, before its `render_living_pre`.
    /// `canceled` skips all of the player's drawing this frame and the other three hooks of the
    /// pair. `model` lends the selected body's bind pose and draw-local visibility.
    pub(in crate::game) fn native_render_player_pre(
        &self,
        player: &MobRender<'_>,
        who: &PlayerRender<'_>,
        draw: &mut BlockEntityDraw,
        model: &mut dyn FrameModelEdit,
        canceled: &mut bool,
    ) {
        self.fire_render_player_pre(player, who, draw, model, canceled);
    }
}
