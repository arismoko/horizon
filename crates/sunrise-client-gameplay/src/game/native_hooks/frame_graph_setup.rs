#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Set up data-driven mod offscreen frame-graph passes at the start of level rendering
    /// (NeoForge's `FrameGraphSetupEvent`).
    pub(in crate::game) fn native_frame_graph_setup(
        &self,
        view: &mod_api::WorldRenderView<'_>,
        graph: &mut sunrise_client::world_geometry::FrameGraphSurface,
    ) {
        self.fire_frame_graph_setup(view, graph);
    }
}
