/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// One stage of a world view (NeoForge's `RenderLevelStageEvent`), asked for each stage in
    /// frame order with the active stage capability (`StageSurface`).
    pub(in crate::game) fn native_render_level_stage(
        &self,
        stage: mod_api::RenderLevelStage,
        view: &mod_api::WorldRenderView<'_>,
        context: &mut sunrise_client::world_geometry::StageSurface<'_>,
    ) {
        self.fire_render_level_stage(stage, view, context);
    }
}
