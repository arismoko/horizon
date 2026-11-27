use crate::game::{BlockEntityDraw, MobRender};
use sunrise_gameplay_common::game::native_hooks::FrameModelEdit;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Before a living entity is drawn (NeoForge's `RenderLivingEvent.Pre`): once a frame for each
    /// mob and player in view, before its pose, model and layers, with its render state and a draw
    /// posed at its feet, turned with its body. `canceled` skips all of the entity's drawing this
    /// frame and its `render_living_post`; `model` lends the selected body's bind pose and
    /// draw-local visibility.
    pub(in crate::game) fn native_render_living_pre(
        &self,
        entity: &MobRender<'_>,
        draw: &mut BlockEntityDraw,
        model: &mut dyn FrameModelEdit,
        canceled: &mut bool,
    ) {
        self.fire_render_living_pre(entity, draw, model, canceled);
    }
}
