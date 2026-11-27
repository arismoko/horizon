#[cfg(feature = "client")]
use crate::game::{BlockEntityDraw, MobRender};
#[cfg(feature = "client")]
use mod_api::FrameModel;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// After a living entity is drawn (NeoForge's `RenderLivingEvent.Post`): once a frame for each
    /// mob and player in view whose pre hook wasn't canceled, after its model and its layers, with
    /// its render state, the final body model if available, and a draw posed at its feet.
    pub(in crate::game) fn native_render_living_post(
        &self,
        entity: &MobRender<'_>,
        model: Option<&dyn FrameModel>,
        draw: &mut BlockEntityDraw,
    ) {
        self.fire_render_living_post(entity, model, draw);
    }
}
