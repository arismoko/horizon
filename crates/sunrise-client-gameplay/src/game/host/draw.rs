use mod_api::{prelude::*, *};
#[cfg(feature = "client")]
use sunrise_gameplay_common::game::api_values::TryIntoEngine;

#[cfg(feature = "client")]
#[path = "draw/gui_canvas.rs"]
pub(in crate::game) mod gui_canvas;
#[cfg(feature = "client")]
#[path = "draw/text_layout.rs"]
pub mod text_layout;

/// Borrow `minecraft:default` directly or format another font identifier.
pub fn font_id_str(id: &mod_api::Identifier) -> std::borrow::Cow<'static, str> {
    if id.namespace() == "minecraft" && id.path() == "default" {
        std::borrow::Cow::Borrowed(sunrise_client::font::DEFAULT)
    } else {
        std::borrow::Cow::Owned(id.to_string())
    }
}

/// Engine font view lent to [`ClientTooltipComponent`] and `render_tooltip_pre`.
pub struct EngineTooltipFont<'a> {
    pub(in crate::game) font: &'a sunrise_client::font::Font,
    pub id: &'a Identifier,
    pub(in crate::game) name: std::borrow::Cow<'static, str>,
}

/// [`DrawContext`] on the game's `crate::hud::HudBatch`: one per hook call.
#[cfg(feature = "client")]
pub struct GameDrawContext<'a, 'g> {
    pub(crate) game: &'a crate::game::ClientGame<'g>,
    pub surface: &'a mut sunrise_client::hud::HudBatch,
    /// Only a scoped child owns a clip; its parent keeps the enclosing one.
    pub(in crate::game) clip: Option<crate::game::gui_clips::GuiClip>,
    pub(in crate::game) tooltip_font: Option<&'a EngineTooltipFont<'a>>,
}
#[cfg(feature = "client")]
impl DrawContext for GameDrawContext<'_, '_> {
    #[inline]
    fn size(&self) -> [f32; 2] {
        self.game.native_size(&*self.surface)
    }
    #[inline]
    fn item_model(&mut self, item: ItemKey, model: &str, at: [f32; 2], size: f32) -> bool {
        let Some(item) = mod_api::__private::item_key_id(item) else {
            return false;
        };
        self.game
            .native_item_model(&mut *self.surface, item, model, at, size)
    }
    #[inline]
    fn quad(&mut self, points: [[f32; 2]; 4], argb: u32) {
        self.game.native_quad(&mut *self.surface, points, argb)
    }
    #[inline]
    fn fill(&mut self, rect: [f32; 4], argb: u32) {
        self.game.native_fill(&mut *self.surface, rect, argb)
    }
    #[inline]
    fn fill_gradient(&mut self, rect: [f32; 4], top: u32, bottom: u32) {
        self.game
            .native_fill_gradient(&mut *self.surface, rect, top, bottom)
    }
    #[inline]
    fn sprite(&mut self, name: &str, src: [f32; 4], dst: [f32; 4], tint: Option<u32>) -> bool {
        self.game
            .native_sprite(&mut *self.surface, name, src, dst, tint)
    }
    #[inline]
    fn item(&mut self, item: ItemKey, at: [f32; 2], size: f32) -> bool {
        let Some(item) = mod_api::__private::item_key_id(item) else {
            return false;
        };
        self.game.native_item(&mut *self.surface, item, at, size)
    }
    #[inline]
    fn text(&mut self, text: &str, at: [f32; 2], argb: u32, shadow: bool) {
        self.game
            .native_text(&mut *self.surface, text, at, argb, shadow)
    }
    #[inline]
    fn text_with_font(
        &mut self,
        font: &Identifier,
        text: &Text,
        at: [f32; 2],
        argb: u32,
        shadow: bool,
    ) -> Result<f32, TextFormError> {
        let text: sunrise_game_data::text::Text = text.try_into_engine(&())?;
        let [a, r, g, b] = argb.to_be_bytes();
        if a == 0 {
            return Ok(0.0);
        }
        let formatted;
        let name: &str = match self.tooltip_font.filter(|active| active.id == font) {
            Some(active) => active.name(),
            None => {
                formatted = font_id_str(font);
                &formatted
            }
        };
        Ok(self.surface.text_styled_with_font(
            &self.game.client.gui.atlas,
            &text,
            at,
            sunrise_client::hud::rgba(r, g, b, a),
            shadow,
            name,
        ))
    }
    #[inline]
    fn with_tooltip_font_raw(&self, font: &Identifier, f: &mut dyn FnMut(&dyn TooltipFont)) {
        if let Some(active) = self.tooltip_font.filter(|active| active.id == font) {
            f(active);
        } else {
            f(&EngineTooltipFont::new(
                &self.game.client.gui.atlas.font,
                font,
            ));
        }
    }
    #[inline]
    fn text_scaled(&mut self, text: &str, at: [f32; 2], scale: f32, argb: u32, shadow: bool) {
        self.game
            .native_text_scaled(&mut *self.surface, text, at, scale, argb, shadow)
    }
    #[inline]
    fn item_stack(&mut self, stack: &ItemStack, at: [f32; 2], size: f32) -> bool {
        let Ok(stack) = sunrise_gameplay_common::game::api_values::stack_value(stack) else {
            return false;
        };
        self.game
            .native_item_stack(&mut *self.surface, &stack, at, size)
    }
    #[inline]
    fn draw_gui(
        &mut self,
        draw: &mut dyn FnMut(&mut dyn GuiCanvas) -> Result<(), GuiDrawError>,
    ) -> Result<(), GuiDrawError> {
        self.game.draw_gui_canvas(
            self.surface,
            self.clip
                .as_ref()
                .map(crate::game::gui_clips::GuiClip::rect),
            draw,
        )
    }
    #[inline]
    fn gui_nine_slice(&mut self, sprite: &str, rect: [f32; 4], color: u32) {
        self.game
            .native_gui_nine_slice(&mut *self.surface, sprite, rect, color)
    }
    #[inline]
    fn gui_tooltip(
        &mut self,
        lines: &[Text],
        at: [f32; 2],
    ) -> Result<(), mod_api::text::TextFormError> {
        let lines: Vec<_> = lines
            .iter()
            .map(sunrise_sim::api_text::to_engine)
            .collect::<Result<_, _>>()?;
        self.game.native_gui_tooltip(&mut *self.surface, &lines, at);
        Ok(())
    }
    #[inline]
    fn text_width(&self, text: &str) -> f32 {
        self.game.native_text_width(&*self.surface, text)
    }
    #[inline]
    fn fluid(&mut self, fluid: &str, at: [f32; 2], size: f32) {
        self.game.native_fluid(&mut *self.surface, fluid, at, size)
    }
    #[inline]
    fn draw_scene(
        &mut self,
        rect: [f32; 4],
        scene: &mod_api::Scene,
        view: SceneView,
    ) -> Result<(), mod_api::SceneError> {
        let blocks =
            crate::game::client_side::api_values_client::aggregates::scene_blocks(&scene.blocks)?;
        self.game
            .native_draw_scene(&mut *self.surface, rect, &blocks, scene, view)
    }
    #[inline]
    fn model_3d(
        &mut self,
        subject: mod_api::Gui3dSubject<'_>,
        view: mod_api::Gui3dView,
    ) -> Result<(), mod_api::Gui3dError> {
        self.game.native_model_3d(&mut *self.surface, subject, view)
    }
    #[inline]
    fn with_scissor(
        &mut self,
        rect: [f32; 4],
        draw: &mut dyn FnMut(&mut dyn DrawContext),
    ) -> Result<(), GuiClipRefusal> {
        let clip = crate::game::gui_clips::GuiClip::open(self.surface, rect, self.clip.as_ref())?;
        draw(&mut GameDrawContext {
            game: self.game,
            surface: &mut *self.surface,
            clip: Some(clip),
            tooltip_font: self.tooltip_font,
        });
        Ok(())
    }
    #[inline]
    fn entity(&mut self, entity: &GuiEntity, view: GuiEntityView) -> Result<(), GuiEntityError> {
        self.game
            .native_draw_entity(&mut *self.surface, entity, view)
    }
    #[inline]
    fn draw_menu_background(&mut self, rect: [f32; 4], in_world: bool) {
        self.game
            .native_draw_menu_background(&mut *self.surface, rect, in_world)
    }
    #[inline]
    fn tiled_sprite(&mut self, sprite: &str, rect: [f32; 4], size: f32, argb: u32) -> bool {
        self.game
            .native_tiled_sprite(&mut *self.surface, sprite, rect, size, argb)
    }
    #[inline]
    fn gui_draw_item_with(&mut self, stack: &ItemStack, at: [f32; 2], text: Option<&str>) {
        let Ok(stack) = sunrise_gameplay_common::game::api_values::stack_value(stack) else {
            return;
        };
        self.game
            .native_gui_draw_item_with(&mut *self.surface, &stack, at, text)
    }
}
/// [`BlockEntityDrawContext`] on the game's `crate::game::native_block_entity_draw::BlockEntityDraw`.
#[cfg(feature = "client")]
pub(crate) struct GameBlockEntityDrawContext<'a, 'g, 'v> {
    pub(crate) game: &'a crate::game::ClientGame<'g>,
    pub(crate) surface: &'a mut crate::game::native_block_entity_draw::BlockEntityDraw<'v>,
}

/// [`CameraDrawContext`] on the game's `crate::screen_effects::CameraQuads`.
#[cfg(feature = "client")]
pub(crate) struct GameCameraDrawContext<'a, 'g> {
    pub(crate) game: &'a crate::game::ClientGame<'g>,
    pub(crate) surface: &'a mut sunrise_client::screen_effects::CameraQuads,
}
/// [`WorldDrawContext`] on the game's `crate::world_geometry::WorldGeometry`.
#[cfg(feature = "client")]
pub(crate) struct GameWorldDrawContext<'a, 'g> {
    pub(crate) game: &'a crate::game::ClientGame<'g>,
    pub(crate) surface: &'a mut sunrise_client::world_geometry::WorldGeometry,
}

/// The only mod surface that appends CPU geometry for named passes.
#[cfg(feature = "client")]
pub(crate) struct GamePrepareRenderBuffersContext<'a, 'g> {
    game: &'a crate::game::ClientGame<'g>,
    surface: &'a mut sunrise_client::world_geometry::WorldGeometry,
}

#[cfg(feature = "client")]
impl<'a, 'g> GamePrepareRenderBuffersContext<'a, 'g> {
    pub(crate) fn new(
        game: &'a crate::game::ClientGame<'g>,
        surface: &'a mut sunrise_client::world_geometry::WorldGeometry,
    ) -> Self {
        Self { game, surface }
    }
}

#[cfg(feature = "client")]
impl PrepareRenderBuffersContext for GamePrepareRenderBuffersContext<'_, '_> {
    fn lines(
        &mut self,
        stage: RenderLevelStage,
        lines: &[WorldLine],
    ) -> Result<PreparedWorldDraw, DrawError> {
        if !stage.has_render_pass() {
            return Err(DrawError::NoRenderPass);
        }
        self.surface.prepare_draw(stage, |surface| {
            self.game.native_world_draw_lines(surface, lines)
        })
    }

    fn quads(
        &mut self,
        stage: RenderLevelStage,
        quads: &[WorldQuad],
    ) -> Result<PreparedWorldDraw, DrawError> {
        if !stage.has_render_pass() {
            return Err(DrawError::NoRenderPass);
        }
        self.surface.prepare_draw(stage, |surface| {
            self.game.native_world_draw_quads(surface, quads)
        })
    }

    fn pipeline_quads(
        &mut self,
        stage: RenderLevelStage,
        pipeline: RenderPipelineRef,
        quads: &[PipelineQuad],
    ) -> Result<PreparedWorldDraw, PipelineDrawError> {
        if !stage.has_render_pass() {
            return Err(PipelineDrawError::Pipeline(RenderPipelineError::Invalid));
        }
        let slot = self
            .game
            .client
            .client_pack
            .render
            .mod_pipelines
            .slot(pipeline)
            .map_err(PipelineDrawError::Pipeline)?;
        if stage.oit_stage().is_some()
            && !self
                .game
                .client
                .client_pack
                .render
                .mod_pipelines
                .is_oit_eligible(slot)
        {
            self.game
                .client
                .client_pack
                .render
                .mod_pipelines
                .warn_oit_ineligible(slot);
            return Err(PipelineDrawError::Pipeline(RenderPipelineError::Invalid));
        }
        self.surface.prepare_draw(stage, |surface| {
            self.game
                .native_world_draw_pipeline_quads(surface, pipeline, quads)
        })
    }

    fn draw_mesh(
        &mut self,
        stage: RenderLevelStage,
        pipeline: RenderPipelineRef,
        mesh: MeshRef,
        draw: MeshDraw,
        bindings: &DrawBindings<'_>,
    ) -> Result<PreparedWorldDraw, PipelineDrawError> {
        if !stage.has_render_pass() {
            return Err(PipelineDrawError::Pipeline(RenderPipelineError::Invalid));
        }
        let slot = self
            .game
            .client
            .client_pack
            .render
            .mod_pipelines
            .slot(pipeline)
            .map_err(PipelineDrawError::Pipeline)?;
        if stage.oit_stage().is_some()
            && !self
                .game
                .client
                .client_pack
                .render
                .mod_pipelines
                .is_oit_eligible(slot)
        {
            self.game
                .client
                .client_pack
                .render
                .mod_pipelines
                .warn_oit_ineligible(slot);
            return Err(PipelineDrawError::Pipeline(RenderPipelineError::Invalid));
        }
        self.surface.prepare_draw(stage, |surface| {
            self.game
                .native_world_draw_mesh(surface, pipeline, mesh, draw, bindings)
        })
    }

    fn draw_indirect(
        &mut self,
        stage: RenderLevelStage,
        draw: &IndirectDraw<'_>,
    ) -> Result<PreparedWorldDraw, PipelineDrawError> {
        if !stage.has_render_pass() {
            return Err(PipelineDrawError::Pipeline(RenderPipelineError::Invalid));
        }
        let slot = self
            .game
            .client
            .client_pack
            .render
            .mod_pipelines
            .slot(draw.pipeline)
            .map_err(PipelineDrawError::Pipeline)?;
        if stage.oit_stage().is_some()
            && !self
                .game
                .client
                .client_pack
                .render
                .mod_pipelines
                .is_oit_eligible(slot)
        {
            self.game
                .client
                .client_pack
                .render
                .mod_pipelines
                .warn_oit_ineligible(slot);
            return Err(PipelineDrawError::Pipeline(RenderPipelineError::Invalid));
        }
        self.surface.prepare_draw(stage, |surface| {
            self.game.native_world_draw_indirect(surface, draw)
        })
    }

    fn draw_vertices(
        &mut self,
        stage: RenderLevelStage,
        pipeline: RenderPipelineRef,
        vertices: &[&[u8]],
        indices: Option<(IndexFormat, &[u8])>,
        draw: MeshDraw,
        bindings: &DrawBindings<'_>,
    ) -> Result<PreparedWorldDraw, PipelineDrawError> {
        if !stage.has_render_pass() {
            return Err(PipelineDrawError::Pipeline(RenderPipelineError::Invalid));
        }
        let slot = self
            .game
            .client
            .client_pack
            .render
            .mod_pipelines
            .slot(pipeline)
            .map_err(PipelineDrawError::Pipeline)?;
        if stage.oit_stage().is_some()
            && !self
                .game
                .client
                .client_pack
                .render
                .mod_pipelines
                .is_oit_eligible(slot)
        {
            self.game
                .client
                .client_pack
                .render
                .mod_pipelines
                .warn_oit_ineligible(slot);
            return Err(PipelineDrawError::Pipeline(RenderPipelineError::Invalid));
        }
        self.surface.prepare_draw(stage, |surface| {
            self.game
                .native_world_draw_vertices(surface, pipeline, vertices, indices, draw, bindings)
        })
    }
}

/// A stage borrows only its prepared draw command surface, never the upload path.
#[cfg(feature = "client")]
pub(crate) struct GameRenderLevelStageContext<'a, 's> {
    surface: &'a mut sunrise_client::world_geometry::StageSurface<'s>,
}

#[cfg(feature = "client")]
impl<'a, 's> GameRenderLevelStageContext<'a, 's> {
    pub(crate) fn new(
        _: &crate::game::ClientGame<'_>,
        surface: &'a mut sunrise_client::world_geometry::StageSurface<'s>,
    ) -> Self {
        Self { surface }
    }
}

#[cfg(feature = "client")]
impl WorldPassDrawContext for GameRenderLevelStageContext<'_, '_> {
    fn draw(&mut self, batch: PreparedWorldDraw) -> Result<(), PreparedWorldDrawError> {
        self.surface.draw(batch)
    }
}

#[cfg(feature = "client")]
impl WorldOitPassDrawContext for GameRenderLevelStageContext<'_, '_> {
    fn oit_stage(&self) -> OitStage {
        self.surface
            .stage()
            .oit_stage()
            .expect("only lent in the OIT capability")
    }

    fn draw(&mut self, batch: PreparedWorldDraw) -> Result<(), PreparedWorldDrawError> {
        self.surface.draw(batch)
    }
}

#[cfg(feature = "client")]
impl RenderLevelStageContext for GameRenderLevelStageContext<'_, '_> {
    fn pass(&mut self) -> StagePass<'_> {
        match self.surface.stage() {
            RenderLevelStage::AfterSky | RenderLevelStage::AfterLevel => StagePass::None,
            RenderLevelStage::OitTranslucent(stage) => StagePass::Oit(stage, self),
            _ => StagePass::World(self),
        }
    }

    fn has_render_pass(&self) -> bool {
        self.surface.stage().has_render_pass()
    }
}

/// Offscreen pass draw recorder handed to a `FrameGraphContext::add_pass` callback.
#[cfg(feature = "client")]
struct GameOffscreenDrawContext<'a, 'g> {
    game: &'a crate::game::ClientGame<'g>,
    pass: &'a mut sunrise_client_render::gfx::mod_frame_graph::RecordedFramePass,
    transients: &'a [RenderTargetDescriptor],
    camera: [f64; 3],
}

#[cfg(feature = "client")]
impl OffscreenDrawContext for GameOffscreenDrawContext<'_, '_> {
    fn draw_mesh(
        &mut self,
        pipeline: RenderPipelineRef,
        mesh: MeshRef,
        draw: MeshDraw,
        bindings: &DrawBindings<'_>,
    ) -> Result<(), PipelineDrawError> {
        self.game.native_offscreen_draw_mesh(
            self.pass,
            self.transients,
            pipeline,
            mesh,
            draw,
            bindings,
        )
    }

    fn draw_indirect(&mut self, draw: &IndirectDraw<'_>) -> Result<(), PipelineDrawError> {
        self.game
            .native_offscreen_draw_indirect(self.pass, self.transients, draw)
    }

    fn draw_vertices(
        &mut self,
        pipeline: RenderPipelineRef,
        vertices: &[&[u8]],
        indices: Option<(IndexFormat, &[u8])>,
        draw: MeshDraw,
        bindings: &DrawBindings<'_>,
    ) -> Result<(), PipelineDrawError> {
        self.game.native_offscreen_draw_vertices(
            self.pass,
            self.transients,
            pipeline,
            vertices,
            indices,
            draw,
            bindings,
        )
    }

    fn pipeline_quads(
        &mut self,
        pipeline: RenderPipelineRef,
        quads: &[PipelineQuad],
    ) -> Result<(), PipelineDrawError> {
        self.game.native_offscreen_draw_pipeline_quads(
            self.pass,
            self.transients,
            self.camera,
            pipeline,
            quads,
        )
    }
}

/// Dispatch recorder handed to a `FrameGraphContext::add_compute_pass` callback.
#[cfg(feature = "client")]
struct GameComputeDispatchContext<'a, 'g> {
    game: &'a crate::game::ClientGame<'g>,
    pass: &'a mut sunrise_client_render::gfx::mod_frame_graph::RecordedFramePass,
    transients: &'a [RenderTargetDescriptor],
}

#[cfg(feature = "client")]
impl ComputeDispatchContext for GameComputeDispatchContext<'_, '_> {
    fn dispatch(
        &mut self,
        pipeline: ComputePipelineRef,
        bindings: &DrawBindings<'_>,
        workgroups: [u32; 3],
    ) -> Result<(), ComputeDispatchError> {
        self.game.native_compute_dispatch(
            self.pass,
            self.transients,
            pipeline,
            bindings,
            Ok(workgroups),
        )
    }

    fn dispatch_indirect(
        &mut self,
        pipeline: ComputePipelineRef,
        bindings: &DrawBindings<'_>,
        indirect: StorageBufferRef,
        offset: u64,
    ) -> Result<(), ComputeDispatchError> {
        self.game.native_compute_dispatch(
            self.pass,
            self.transients,
            pipeline,
            bindings,
            Err((indirect, offset)),
        )
    }
}

/// Frame-graph setup context lent to `Hooks::frame_graph_setup` (`FrameGraphSetupEvent`).
#[cfg(feature = "client")]
pub(crate) struct GameFrameGraphContext<'a, 'g> {
    game: &'a crate::game::ClientGame<'g>,
    surface: &'a mut sunrise_client::world_geometry::FrameGraphSurface,
}

#[cfg(feature = "client")]
impl<'a, 'g> GameFrameGraphContext<'a, 'g> {
    pub(crate) fn new(
        game: &'a crate::game::ClientGame<'g>,
        surface: &'a mut sunrise_client::world_geometry::FrameGraphSurface,
    ) -> Self {
        Self { game, surface }
    }
}

#[cfg(feature = "client")]
impl FrameGraphContext for GameFrameGraphContext<'_, '_> {
    fn create_transient(
        &mut self,
        desc: &RenderTargetDescriptor,
    ) -> Result<TransientTargetId, RenderTargetError> {
        let Some(renderer) = self.game.mod_gpu() else {
            return Err(RenderTargetError::Unavailable);
        };
        desc.validate(renderer.max_texture_dimension_2d())?;
        // Pooled transient targets have a single mip level and no storage access.
        if desc.mip_levels != 1 {
            return Err(RenderTargetError::InvalidMipLevels);
        }
        if desc.storage.is_some() {
            return Err(RenderTargetError::UnsupportedStorage);
        }
        Ok(self.surface.graph.add_transient(desc.clone()))
    }

    fn add_pass(
        &mut self,
        desc: FramePassDescriptor,
        record: &mut dyn FnMut(&mut dyn OffscreenDrawContext),
    ) -> Result<FramePassId, FrameGraphError> {
        let Some(renderer) = self.game.mod_gpu() else {
            return Err(FrameGraphError::Unavailable);
        };
        let owner = mod_api::content::crash::current_native_mod();
        // Validate imported mod render target ownership/liveness, and the mip level the pass
        // renders into, up front.
        if let FrameTarget::Mod(rt) = desc.target {
            renderer
                .check_mod_render_target_level(owner, rt, desc.mip_level)
                .map_err(FrameGraphError::InvalidTarget)?;
        }
        for &t in desc.reads.iter().chain(desc.writes.iter()) {
            if let FrameTarget::Mod(rt) = t {
                renderer
                    .render_target_size(owner, rt)
                    .map_err(FrameGraphError::InvalidTarget)?;
            }
        }
        if let Err(err) = self
            .surface
            .graph
            .validate_new_pass(&desc, self.surface.oit_on)
        {
            if let FrameGraphError::Cycle { ref passes } = err {
                renderer.warn_frame_graph_cycle_once(passes);
            }
            return Err(err);
        }
        if matches!(desc.target, FrameTarget::Transient(_)) && desc.mip_level != 0 {
            return Err(FrameGraphError::InvalidTarget(
                RenderTargetError::InvalidMipLevels,
            ));
        }
        let mut recorded =
            sunrise_client_render::gfx::mod_frame_graph::RecordedFramePass::from_descriptor(
                desc, owner,
            );
        {
            let mut ctx = GameOffscreenDrawContext {
                game: self.game,
                pass: &mut recorded,
                transients: &self.surface.graph.transients,
                camera: self.surface.camera,
            };
            record(&mut ctx);
        }
        Ok(self.surface.graph.push_pass(recorded))
    }

    fn add_compute_pass(
        &mut self,
        desc: ComputePassDescriptor,
        record: &mut dyn FnMut(&mut dyn ComputeDispatchContext),
    ) -> Result<FramePassId, FrameGraphError> {
        let Some(renderer) = self.game.mod_gpu() else {
            return Err(FrameGraphError::Unavailable);
        };
        let owner = mod_api::content::crash::current_native_mod();
        for &t in desc.reads.iter().chain(desc.writes.iter()) {
            if let FrameTarget::Mod(rt) = t {
                renderer
                    .render_target_size(owner, rt)
                    .map_err(FrameGraphError::InvalidTarget)?;
            }
        }
        if let Err(err) = self
            .surface
            .graph
            .validate_new_compute_pass(&desc, self.surface.oit_on)
        {
            if let FrameGraphError::Cycle { ref passes } = err {
                renderer.warn_frame_graph_cycle_once(passes);
            }
            return Err(err);
        }
        let mut recorded =
            sunrise_client_render::gfx::mod_frame_graph::RecordedFramePass::from_compute_descriptor(
                desc, owner,
            );
        // Readbacks are accepted only while the node records: they are copies at its place.
        let readbacks = renderer.begin_readback_recording();
        {
            let mut ctx = GameComputeDispatchContext {
                game: self.game,
                pass: &mut recorded,
                transients: &self.surface.graph.transients,
            };
            record(&mut ctx);
        }
        recorded.readbacks = readbacks.finish();
        Ok(self.surface.graph.push_pass(recorded))
    }
}

#[cfg(all(any(test, feature = "test-support"), feature = "client"))]
pub fn test_draw_gui_canvas(
    batch: &mut sunrise_client::hud::HudBatch,
    atlas: &sunrise_client::hud::HudAtlas,
    game: Option<&crate::game::ClientGame<'_>>,
    scissor: Option<[f32; 4]>,
    draw: &mut dyn FnMut(&mut dyn GuiCanvas) -> Result<(), GuiDrawError>,
) -> Result<(), GuiDrawError> {
    gui_canvas::GameGuiCanvas::draw(batch, atlas, game, scissor, draw)
}

#[cfg(all(any(test, feature = "test-support"), feature = "client"))]
pub fn canvas_test_font() -> sunrise_client::font::Font {
    use sunrise_assets::resource_source::{Resource, ResourceSource};
    use sunrise_client::hud::Rect;

    // A 24×8 sheet in a 32×16 bitmap, six fixed-size cells. Ink widths of
    // 3, 2, 2, 2, 2, 3 give advances of 4, 3, 3, 3, 3, 4 respectively.
    let definition = r#"{"providers":[{"type":"space","advances":{" ":2}},{"type":"bitmap","file":"test:font/fixture.png","ascent":7,"chars":["mאבג65"]}]}"#;
    let source = ResourceSource::indexed(
        [(
            "assets/minecraft/font/default.json".into(),
            Resource::bytes(definition.as_bytes().to_vec()),
        )]
        .into(),
    );
    let mut pixels = vec![0; 32 * 16 * 4];
    for (cell, width) in [3, 2, 2, 2, 2, 3].into_iter().enumerate() {
        for y in 0..8 {
            let offset = (y * 32 + cell * 4) * 4;
            pixels[offset..offset + width * 4].fill(255);
        }
    }
    let pixel = Rect {
        u0: 0.0,
        v0: 0.0,
        u1: 1.0 / 32.0,
        v1: 1.0 / 16.0,
        w: 1,
        h: 1,
    };
    let sheet = Rect {
        u0: 0.0,
        v0: 0.0,
        u1: 24.0 / 32.0,
        v1: 8.0 / 16.0,
        w: 24,
        h: 8,
    };
    sunrise_client::font::Font::load(
        &source,
        &[
            ("test:font/fixture".into(), sheet),
            ("sunrise/missing_glyph".into(), pixel),
        ]
        .into(),
        &pixels,
        (32, 16),
        (0, 0),
        pixel,
    )
}
