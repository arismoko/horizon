//! The game's side of the `draw` area (`sunrise-mod-api/src/host/draw.rs`).
use crate::game::{
    Game,
    api_values::{TryIntoApi, TryIntoEngine},
};
use mod_api::{prelude::*, *};

#[path = "draw/gui_canvas.rs"]
mod gui_canvas;
#[path = "draw/text_layout.rs"]
pub(in crate::game) mod text_layout;

/// Borrow `minecraft:default` directly or format another font identifier.
pub(in crate::game) fn font_id_str(id: &mod_api::Identifier) -> std::borrow::Cow<'static, str> {
    if id.namespace() == "minecraft" && id.path() == "default" {
        std::borrow::Cow::Borrowed(sunrise_client::font::DEFAULT)
    } else {
        std::borrow::Cow::Owned(id.to_string())
    }
}

/// Engine font view lent to [`ClientTooltipComponent`] and `render_tooltip_pre`.
pub(in crate::game) struct EngineTooltipFont<'a> {
    font: &'a sunrise_client::font::Font,
    id: &'a Identifier,
    name: std::borrow::Cow<'static, str>,
}
impl<'a> EngineTooltipFont<'a> {
    pub(in crate::game) fn new(font: &'a sunrise_client::font::Font, id: &'a Identifier) -> Self {
        Self {
            font,
            id,
            name: font_id_str(id),
        }
    }

    pub(in crate::game) fn name(&self) -> &str {
        &self.name
    }
}

impl TooltipFont for EngineTooltipFont<'_> {
    fn id(&self) -> &Identifier {
        self.id
    }
    fn line_height(&self) -> i32 {
        9
    }
    fn width(&self, text: &Text) -> Result<i32, TextFormError> {
        let text: crate::text::Text = text.try_into_engine(&())?;
        Ok(self.font.width_text_with_font(&text, self.name()) as i32)
    }
    fn split(&self, text: &Text, max_width: i32) -> Result<Vec<Text>, TextFormError> {
        let text: crate::text::Text = text.try_into_engine(&())?;
        self.font
            .split_formatted_with_font(&text, max_width as f32, self.name())
            .into_iter()
            .map(|line| line.try_into_api())
            .collect()
    }
}

impl Game<'_> {
    pub(in crate::game) fn draw_gui_canvas(
        &self,
        batch: &mut crate::hud::HudBatch,
        scissor: Option<[f32; 4]>,
        draw: &mut dyn FnMut(&mut dyn GuiCanvas) -> Result<(), GuiDrawError>,
    ) -> Result<(), GuiDrawError> {
        gui_canvas::GameGuiCanvas::draw(batch, &self.gui.atlas, Some(self), scissor, draw)
    }
}

/// [`DrawContext`] on the game's `crate::hud::HudBatch`: one per hook call.
pub(crate) struct GameDrawContext<'a, 'g> {
    pub(crate) game: &'a Game<'g>,
    pub(crate) surface: &'a mut crate::hud::HudBatch,
    /// Only a scoped child owns a clip; its parent keeps the enclosing one.
    clip: Option<crate::game::gui_clips::GuiClip>,
    tooltip_font: Option<&'a EngineTooltipFont<'a>>,
}
impl<'a, 'g> GameDrawContext<'a, 'g> {
    /// Lend a fresh clip stack and isolate the drawable tail of one listener.
    pub(crate) fn stage(
        game: &Game<'_>,
        surface: &mut crate::hud::HudBatch,
        listen: &mut dyn FnMut(&mut dyn DrawContext) -> bool,
    ) {
        let counts = (
            surface.vertices.len(),
            surface.pictures.len(),
            surface.blends.len(),
            surface.inline_profiles.len(),
        );
        let last_blend_end = surface.blends.last().map(|(range, _)| range.end);
        let accepted = listen(&mut GameDrawContext::new(game, surface));
        if !accepted {
            surface.truncate_vertices(counts.0);
            surface.pictures.truncate(counts.1);
            surface.blends.truncate(counts.2);
            surface.inline_profiles.truncate(counts.3);
            if let Some((last, end)) = surface.blends.last_mut().zip(last_blend_end) {
                last.0.end = end;
            }
            // Keep glyph uploads: font extraction already cached those atlas locations.
            // They do not draw anything without an accepted vertex referencing them.
        }
    }

    pub(crate) fn new(game: &'a Game<'g>, surface: &'a mut crate::hud::HudBatch) -> Self {
        Self {
            game,
            surface,
            clip: None,
            tooltip_font: None,
        }
    }
    pub(in crate::game) fn with_active_tooltip_font(
        mut self,
        font: &'a EngineTooltipFont<'a>,
    ) -> Self {
        self.tooltip_font = Some(font);
        self
    }
}
impl Drop for GameDrawContext<'_, '_> {
    #[inline]
    fn drop(&mut self) {
        if let Some(clip) = self.clip.take() {
            clip.close(self.surface);
        }
    }
}
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
        let text: crate::text::Text = text.try_into_engine(&())?;
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
            &self.game.gui.atlas,
            &text,
            at,
            crate::hud::rgba(r, g, b, a),
            shadow,
            name,
        ))
    }
    #[inline]
    fn with_tooltip_font_raw(&self, font: &Identifier, f: &mut dyn FnMut(&dyn TooltipFont)) {
        if let Some(active) = self.tooltip_font.filter(|active| active.id == font) {
            f(active);
        } else {
            f(&EngineTooltipFont::new(&self.game.gui.atlas.font, font));
        }
    }
    #[inline]
    fn text_scaled(&mut self, text: &str, at: [f32; 2], scale: f32, argb: u32, shadow: bool) {
        self.game
            .native_text_scaled(&mut *self.surface, text, at, scale, argb, shadow)
    }
    #[inline]
    fn item_stack(&mut self, stack: &ItemStack, at: [f32; 2], size: f32) -> bool {
        let Ok(stack) = crate::game::api_values::stack_value(stack) else {
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
            .map(sunrise_client_lib::api_text::to_engine)
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
        let blocks = crate::game::api_values::aggregates::scene_blocks(&scene.blocks)?;
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
        let Ok(stack) = crate::game::api_values::stack_value(stack) else {
            return;
        };
        self.game
            .native_gui_draw_item_with(&mut *self.surface, &stack, at, text)
    }
}
/// [`BlockEntityDrawContext`] on the game's `crate::game::native_block_entity_draw::BlockEntityDraw`.
pub(crate) struct GameBlockEntityDrawContext<'a, 'g, 'v> {
    pub(crate) game: &'a Game<'g>,
    pub(crate) surface: &'a mut crate::game::native_block_entity_draw::BlockEntityDraw<'v>,
}
impl<'a, 'g, 'v> GameBlockEntityDrawContext<'a, 'g, 'v> {
    pub(crate) fn new(
        game: &'a Game<'g>,
        surface: &'a mut crate::game::native_block_entity_draw::BlockEntityDraw<'v>,
    ) -> Self {
        Self { game, surface }
    }
}
impl BlockEntityDrawContext for GameBlockEntityDrawContext<'_, '_, '_> {
    #[inline]
    fn push(&mut self) {
        self.game.native_draw_push(&mut *self.surface)
    }
    #[inline]
    fn pop(&mut self) {
        self.game.native_draw_pop(&mut *self.surface)
    }
    #[inline]
    fn translate(&mut self, by: [f32; 3]) {
        self.game.native_draw_translate(&mut *self.surface, by)
    }
    #[inline]
    fn rotate_about(&mut self, axis: [f32; 3], degrees: f32, pivot: [f32; 3]) {
        self.game
            .native_draw_rotate_about(&mut *self.surface, axis, degrees, pivot)
    }
    #[inline]
    fn scale(&mut self, by: [f32; 3]) {
        self.game.native_draw_scale(&mut *self.surface, by)
    }
    #[inline]
    fn keep(&mut self) {
        self.game.native_draw_keep(&mut *self.surface)
    }
    #[inline]
    fn view(&self) -> &WorldRenderView<'_> {
        self.surface.view()
    }
    #[inline]
    fn partial(&self) -> f32 {
        self.game.native_draw_partial(&*self.surface)
    }
    #[inline]
    fn game_time(&self) -> i64 {
        self.game.native_draw_game_time(&*self.surface)
    }
    #[inline]
    fn animation_time(&self) -> f64 {
        self.game.native_draw_animation_time(&*self.surface)
    }
    #[inline]
    fn light(&self) -> u32 {
        self.game.native_draw_light(&*self.surface)
    }
    #[inline]
    fn set_light(&mut self, light: u32) {
        self.game.native_draw_set_light(&mut *self.surface, light)
    }
    #[inline]
    fn level_light(&mut self) {
        self.game.native_draw_level_light(&mut *self.surface)
    }
    #[inline]
    fn unlit(&mut self, on: bool) {
        self.game.native_draw_unlit(&mut *self.surface, on)
    }
    #[inline]
    fn quad(&mut self, sprite: &str, corners: [[f32; 3]; 4], uv: [f32; 4], tint: u32) -> bool {
        self.game
            .native_draw_quad(&mut *self.surface, sprite, corners, uv, tint)
    }
    #[inline]
    fn textured_quad(
        &mut self,
        render_type: RenderTypeRef,
        corners: [[f32; 3]; 4],
        uv: [[f32; 2]; 4],
        tint: u32,
    ) -> bool {
        self.game
            .native_draw_textured_quad(&mut *self.surface, render_type, corners, uv, tint)
    }
    #[inline]
    fn cube(
        &mut self,
        from: [f32; 3],
        to: [f32; 3],
        faces: [Option<CubeFace<'_>>; 6],
        tint: u32,
    ) -> bool {
        self.game
            .native_draw_cube(&mut *self.surface, from, to, faces, tint)
    }
    #[inline]
    fn block(&mut self, state: BlockState) -> bool {
        let Ok(id) = state.try_into_engine(&()) else {
            return false;
        };
        self.game.native_draw_block(&mut *self.surface, id)
    }
    #[inline]
    fn item(
        &mut self,
        stack: &ItemStack,
        display: ItemDisplayContext,
        owner_yaw: Option<f32>,
        seed: i32,
    ) -> bool {
        let Ok(stack) = crate::game::api_values::stack_value(stack) else {
            return false;
        };
        let Ok(display) = display.try_into_engine(self.game.items()) else {
            return false;
        };
        self.game
            .native_draw_item(&mut *self.surface, &stack, display, owner_yaw, seed)
    }
    #[inline]
    fn spin(&mut self, axis: [f32; 3], pivot: [f32; 3], degrees_per_tick: f32, phase: f32) {
        self.game
            .native_draw_spin(&mut *self.surface, axis, pivot, degrees_per_tick, phase)
    }
    #[inline]
    fn still(&mut self) {
        self.game.native_draw_still(&mut *self.surface)
    }
    #[inline]
    fn model(&mut self, model: ModelRef, tint: u32) -> Result<(), ModelError> {
        self.game.native_draw_model(&mut *self.surface, model, tint)
    }
    #[inline]
    fn text(
        &mut self,
        text: &Text,
        at: [f32; 2],
        color: u32,
        glowing: bool,
        outline: Option<u32>,
    ) -> Result<f32, mod_api::text::TextFormError> {
        let text = sunrise_client_lib::api_text::to_engine(text)?;
        Ok(self
            .game
            .native_draw_text(&mut *self.surface, &text, at, color, glowing, outline))
    }
    #[inline]
    fn terrain_model(
        &mut self,
        model: ModelRef,
        options: TerrainModelOptions<'_>,
    ) -> Result<(), ModelError> {
        self.game
            .native_draw_terrain_model(&mut *self.surface, model, options)
    }
    #[inline]
    fn text_width(&self, text: &Text) -> f32 {
        let text = sunrise_client_lib::api_text::to_engine_inprocess(text);
        self.game.native_draw_text_width(&*self.surface, &text)
    }
    #[inline]
    fn model_part(&mut self, part: &ModelPart, sprite: &str, tint: u32) -> bool {
        self.game
            .native_draw_model_part(&mut *self.surface, part, sprite, tint)
    }
    #[inline]
    fn instance_group(&mut self, group: InstanceGroupRef) -> Result<(), GroupError> {
        self.game
            .native_draw_instance_group(&mut *self.surface, group)
    }
    #[inline]
    fn prepared_item(&mut self, item: PreparedItemRef) -> Result<(), ModelError> {
        self.game
            .native_draw_prepared_item(&mut *self.surface, item)
    }
    #[inline]
    fn fluid_box(&mut self, stack: &FluidStack, bounds: [f32; 6], options: FluidBoxOptions) {
        self.game
            .native_draw_fluid_box(&mut *self.surface, stack, bounds, options)
    }
}
impl HandDrawContext for GameBlockEntityDrawContext<'_, '_, '_> {
    #[inline]
    fn render_arm(&mut self, arm: MainHand) {
        if let Some(normals) = self.surface.normal_pose() {
            self.game.submit_first_person_arm(
                arm == MainHand::Left,
                (self.surface.pose(), normals),
                self.surface.light(),
                self.surface.vertices_mut(),
            );
        }
    }
}

/// [`CameraDrawContext`] on the game's `crate::screen_effects::CameraQuads`.
pub(crate) struct GameCameraDrawContext<'a, 'g> {
    pub(crate) game: &'a Game<'g>,
    pub(crate) surface: &'a mut crate::screen_effects::CameraQuads,
}
impl<'a, 'g> GameCameraDrawContext<'a, 'g> {
    pub(crate) fn new(
        game: &'a Game<'g>,
        surface: &'a mut crate::screen_effects::CameraQuads,
    ) -> Self {
        Self { game, surface }
    }
}
impl CameraDrawContext for GameCameraDrawContext<'_, '_> {
    fn quad(&mut self, corners: [[f32; 3]; 4], argb: u32) -> bool {
        self.game
            .native_camera_quad(&mut *self.surface, corners, argb)
    }
    fn sprite(&mut self, name: &str, corners: [[f32; 3]; 4], uv: [[f32; 2]; 4], argb: u32) -> bool {
        self.game
            .native_camera_sprite(&mut *self.surface, name, corners, uv, argb)
    }
}
/// [`WorldDrawContext`] on the game's `crate::world_geometry::WorldGeometry`.
pub(crate) struct GameWorldDrawContext<'a, 'g> {
    pub(crate) game: &'a Game<'g>,
    pub(crate) surface: &'a mut crate::world_geometry::WorldGeometry,
}
impl<'a, 'g> GameWorldDrawContext<'a, 'g> {
    pub(crate) fn new(
        game: &'a Game<'g>,
        surface: &'a mut crate::world_geometry::WorldGeometry,
    ) -> Self {
        Self { game, surface }
    }
}
impl WorldDrawContext for GameWorldDrawContext<'_, '_> {
    fn draw_instance_group(&mut self, group: InstanceGroupRef) -> Result<(), GroupError> {
        self.game.draw_instance_group(self.surface, group)
    }
    fn lines(&mut self, lines: &[WorldLine]) -> Result<(), DrawError> {
        self.game.native_world_draw_lines(&mut *self.surface, lines)
    }
    fn quads(&mut self, quads: &[WorldQuad]) -> Result<(), DrawError> {
        self.game.native_world_draw_quads(&mut *self.surface, quads)
    }
    fn pipeline_quads(
        &mut self,
        pipeline: RenderPipelineRef,
        quads: &[PipelineQuad],
    ) -> Result<(), PipelineDrawError> {
        self.game
            .native_world_draw_pipeline_quads(&mut *self.surface, pipeline, quads)
    }
    fn draw_mesh(
        &mut self,
        pipeline: RenderPipelineRef,
        mesh: MeshRef,
        draw: MeshDraw,
        bindings: &DrawBindings<'_>,
    ) -> Result<(), PipelineDrawError> {
        self.game
            .native_world_draw_mesh(&mut *self.surface, pipeline, mesh, draw, bindings)
    }
    fn draw_indirect(&mut self, draw: &IndirectDraw<'_>) -> Result<(), PipelineDrawError> {
        self.game
            .native_world_draw_indirect(&mut *self.surface, draw)
    }
    fn draw_vertices(
        &mut self,
        pipeline: RenderPipelineRef,
        vertices: &[&[u8]],
        indices: Option<(IndexFormat, &[u8])>,
        draw: MeshDraw,
        bindings: &DrawBindings<'_>,
    ) -> Result<(), PipelineDrawError> {
        self.game.native_world_draw_vertices(
            &mut *self.surface,
            pipeline,
            vertices,
            indices,
            draw,
            bindings,
        )
    }
    fn name_tag(
        &mut self,
        at: [f64; 3],
        text: &Text,
        options: NameTagOptions,
    ) -> Result<(), DrawError> {
        self.game
            .native_world_name_tag(&mut *self.surface, at, text, options)
    }
    fn draw_model_instances(
        &mut self,
        model: ModelRef,
        instances: &[ModelInstance],
    ) -> Result<(), InstanceError> {
        self.game
            .native_world_draw_model_instances(&mut *self.surface, model, instances)
    }
    fn with_pipeline_modifier(
        &mut self,
        modifier: PipelineModifierRef,
        draw: &mut dyn FnMut(&mut dyn WorldDrawContext),
    ) {
        use mod_api::__private::game::Fields;
        let pushed = self.surface.push_pipeline_modifier(modifier.fields().slot);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            draw(self);
        }));
        if pushed {
            self.surface.pop_pipeline_modifier();
        }
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
    }
    fn set_pack_uniform(&mut self, name: &str, value: PackUniformValue) -> bool {
        let Some((static_name, ty)) = sunrise_client::mod_pipelines::lookup_pack_uniform(name)
        else {
            return false;
        };
        if ty != value.ty() {
            return false;
        }
        self.surface.set_pack_uniform(static_name, value);
        true
    }
    fn pack_override_state(
        &self,
        pipeline: RenderPipelineRef,
    ) -> Result<PackOverrideState, RenderPipelineError> {
        let slot = self.game.pack.render.mod_pipelines.slot(pipeline)?;
        Ok(self.game.pack_override_state(slot as usize))
    }
}
/// The only mod surface that appends CPU geometry for named passes.
pub(crate) struct GamePrepareRenderBuffersContext<'a, 'g> {
    game: &'a Game<'g>,
    surface: &'a mut crate::world_geometry::WorldGeometry,
}

impl<'a, 'g> GamePrepareRenderBuffersContext<'a, 'g> {
    pub(crate) fn new(
        game: &'a Game<'g>,
        surface: &'a mut crate::world_geometry::WorldGeometry,
    ) -> Self {
        Self { game, surface }
    }
}

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
            .pack
            .render
            .mod_pipelines
            .slot(pipeline)
            .map_err(PipelineDrawError::Pipeline)?;
        if stage.oit_stage().is_some() && !self.game.pack.render.mod_pipelines.is_oit_eligible(slot)
        {
            self.game
                .pack
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
            .pack
            .render
            .mod_pipelines
            .slot(pipeline)
            .map_err(PipelineDrawError::Pipeline)?;
        if stage.oit_stage().is_some() && !self.game.pack.render.mod_pipelines.is_oit_eligible(slot)
        {
            self.game
                .pack
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
            .pack
            .render
            .mod_pipelines
            .slot(draw.pipeline)
            .map_err(PipelineDrawError::Pipeline)?;
        if stage.oit_stage().is_some() && !self.game.pack.render.mod_pipelines.is_oit_eligible(slot)
        {
            self.game
                .pack
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
            .pack
            .render
            .mod_pipelines
            .slot(pipeline)
            .map_err(PipelineDrawError::Pipeline)?;
        if stage.oit_stage().is_some() && !self.game.pack.render.mod_pipelines.is_oit_eligible(slot)
        {
            self.game
                .pack
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
pub(crate) struct GameRenderLevelStageContext<'a, 's> {
    surface: &'a mut crate::world_geometry::StageSurface<'s>,
}

impl<'a, 's> GameRenderLevelStageContext<'a, 's> {
    pub(crate) fn new(
        _: &Game<'_>,
        surface: &'a mut crate::world_geometry::StageSurface<'s>,
    ) -> Self {
        Self { surface }
    }
}

impl WorldPassDrawContext for GameRenderLevelStageContext<'_, '_> {
    fn draw(&mut self, batch: PreparedWorldDraw) -> Result<(), PreparedWorldDrawError> {
        self.surface.draw(batch)
    }
}

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
struct GameOffscreenDrawContext<'a, 'g> {
    game: &'a Game<'g>,
    pass: &'a mut sunrise_client::gfx::mod_frame_graph::RecordedFramePass,
    transients: &'a [RenderTargetDescriptor],
    camera: [f64; 3],
}

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
struct GameComputeDispatchContext<'a, 'g> {
    game: &'a Game<'g>,
    pass: &'a mut sunrise_client::gfx::mod_frame_graph::RecordedFramePass,
    transients: &'a [RenderTargetDescriptor],
}

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
pub(crate) struct GameFrameGraphContext<'a, 'g> {
    game: &'a Game<'g>,
    surface: &'a mut crate::world_geometry::FrameGraphSurface,
}

impl<'a, 'g> GameFrameGraphContext<'a, 'g> {
    pub(crate) fn new(
        game: &'a Game<'g>,
        surface: &'a mut crate::world_geometry::FrameGraphSurface,
    ) -> Self {
        Self { game, surface }
    }
}

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
        // Validate imported mod render target ownership/liveness up front.
        if let FrameTarget::Mod(rt) = desc.target {
            renderer
                .render_target_size(owner, rt)
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
        let mut recorded =
            sunrise_client::gfx::mod_frame_graph::RecordedFramePass::from_descriptor(desc, owner);
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
            sunrise_client::gfx::mod_frame_graph::RecordedFramePass::from_compute_descriptor(
                desc, owner,
            );
        {
            let mut ctx = GameComputeDispatchContext {
                game: self.game,
                pass: &mut recorded,
                transients: &self.surface.graph.transients,
            };
            record(&mut ctx);
        }
        Ok(self.surface.graph.push_pass(recorded))
    }
}

#[cfg(test)]
pub(in crate::game) fn canvas_test_font() -> crate::font::Font {
    use crate::{
        hud::Rect,
        resource_source::{Resource, ResourceSource},
    };

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
    crate::font::Font::load(
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// A font whose printable characters are all `advance` GUI pixels wide: the vanilla provider
    /// list with a `space` provider, so a layout test measures exact advances.
    fn fake_font(advance: f32) -> crate::font::Font {
        let advances: Vec<String> = (32u8..=126)
            .map(char::from)
            // A JSON object key: the quote and the backslash are the two that need escaping.
            .filter(|&c| !matches!(c, '"' | '\\'))
            .map(|c| format!("\"{c}\":{advance}"))
            .collect();
        let json = format!(
            r#"{{"providers":[{{"type":"space","advances":{{{}}}}}]}}"#,
            advances.join(",")
        );
        let source = crate::resource_source::ResourceSource::indexed(BTreeMap::from([(
            "assets/minecraft/font/default.json".into(),
            crate::resource_source::Resource::bytes(json.into_bytes()),
        )]));
        crate::font::Font::metrics(&source)
    }

    /// A line's component as the game draws it.
    fn engine(text: &mod_api::Text) -> crate::text::Text {
        text.try_into_engine(&()).unwrap()
    }

    #[derive(Debug)]
    struct MeasuredLine {
        text: Text,
        at: [f32; 2],
        width: f32,
    }

    #[derive(Debug)]
    struct Measurement {
        lines: Vec<MeasuredLine>,
        size: TextLayoutSize,
    }

    fn measured(
        font: &crate::font::Font,
        text: &Text,
        max_width: f32,
        align: TextAlign,
        scale: f32,
    ) -> Result<Measurement, TextLayoutError> {
        let layout = text_layout::LabelLayout::try_from(TextLayoutSpec {
            max_width,
            align,
            scale,
        })?;
        let mut lines = Vec::new();
        let size = layout.visit_components(font, text, &mut |line| {
            lines.push(MeasuredLine {
                text: line.text.clone(),
                at: line.at,
                width: line.width,
            })
        })?;
        Ok(Measurement { lines, size })
    }

    fn plain(lines: &Measurement) -> Vec<String> {
        lines
            .lines
            .iter()
            .map(|line| engine(&line.text).plain())
            .collect()
    }

    fn layout(text: &str, max_width: f32, align: TextAlign, scale: f32) -> Measurement {
        measured(
            &fake_font(6.0),
            &Text::literal(text),
            max_width,
            align,
            scale,
        )
        .unwrap()
    }

    #[test]
    fn a_label_wraps_at_spaces_and_keeps_its_words() {
        // 48 pixels is eight characters: "a quick" fits, "brown" does not fit after it, and
        // "brown fox" is one character too wide for the second line.
        let lines = layout("a quick brown fox", 48.0, TextAlign::Left, 1.0);
        assert_eq!(plain(&lines), ["a quick", "brown", "fox"]);
        assert_eq!(lines.size.width, 42.0);
        assert!(
            lines.lines.iter().all(|line| line.width <= 48.0),
            "{:?}",
            lines.lines.iter().map(|l| l.width).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_word_longer_than_the_line_is_broken_at_a_character() {
        let lines = layout("abcdefghij", 24.0, TextAlign::Left, 1.0);
        assert_eq!(plain(&lines), ["abcd", "efgh", "ij"]);
        assert_eq!(plain(&lines).concat(), "abcdefghij");
    }

    #[test]
    fn an_explicit_newline_starts_a_line_and_an_empty_component_has_no_line() {
        assert_eq!(
            plain(&layout("AA\nBB", 100.0, TextAlign::Left, 1.0)),
            ["AA", "BB"]
        );
        let empty = layout("", 100.0, TextAlign::Left, 1.0);
        assert!(empty.lines.is_empty());
        assert_eq!((empty.size.width, empty.size.height), (0.0, 0.0));
    }

    #[test]
    fn a_trailing_newline_leaves_empty_last_lines() {
        // One `\n` ends the line it closes and the label still holds the empty line after it;
        // two leave two, the second from the split's own trailing newline.
        let one = layout("AA\n", 100.0, TextAlign::Left, 1.0);
        assert_eq!(plain(&one), ["AA", ""]);
        assert_eq!((one.size.width, one.size.height), (12.0, 18.0));
        let two = layout("AA\n\n", 100.0, TextAlign::Left, 1.0);
        assert_eq!(plain(&two), ["AA", "", ""]);
        assert_eq!((two.size.width, two.size.height), (12.0, 27.0));
    }

    #[test]
    fn leading_spaces_and_runs_of_spaces_stay_in_the_line() {
        // Nothing trims a label: the spaces at its start are characters like any other.
        let lead = layout("  AA", 100.0, TextAlign::Left, 1.0);
        assert_eq!(plain(&lead), ["  AA"]);
        assert_eq!(lead.size.width, 24.0);
        // A run between words is kept when it fits, and the space a wrap breaks at is dropped.
        assert_eq!(
            plain(&layout("AA   BB", 100.0, TextAlign::Left, 1.0)),
            ["AA   BB"]
        );
        assert_eq!(
            plain(&layout("  AA BB", 24.0, TextAlign::Left, 1.0)),
            ["  AA", "BB"]
        );
    }

    #[test]
    fn spaces_at_a_break_leave_empty_lines() {
        // A leading space that does not fit is the break: an empty first line, then one character
        // a line. The break at a run of spaces drops only the space it breaks at.
        assert_eq!(
            plain(&layout(" AA", 6.0, TextAlign::Left, 1.0)),
            ["", "A", "A"]
        );
        assert_eq!(
            plain(&layout("AA    BBBB", 24.0, TextAlign::Left, 1.0)),
            ["AA  ", "", "BBBB"]
        );
    }

    #[test]
    fn each_line_is_placed_by_the_alignment() {
        // 24 and 12 pixels wide, in a box of 100: left leaves them at 0, centre at 38 and 44,
        // right at 76 and 88.
        for (align, expected) in [
            (TextAlign::Left, [0.0, 0.0]),
            (TextAlign::Centre, [38.0, 44.0]),
            (TextAlign::Right, [76.0, 88.0]),
        ] {
            let lines = layout("AAAA\nBB", 100.0, align, 1.0);
            let placed: Vec<f32> = lines.lines.iter().map(|line| line.at[0]).collect();
            assert_eq!(placed, expected, "{align:?}");
        }
    }

    #[test]
    fn a_centred_line_rounds_the_half_line_width_down() {
        // A 15-pixel line in a 40-pixel box: `TextAlignment.CENTER.calculateLeft` floors the half
        // line width, so it starts at 13, not at the exact 12.5 an unrounded centre would give.
        let lines = measured(
            &fake_font(5.0),
            &Text::literal("AAA"),
            40.0,
            TextAlign::Centre,
            1.0,
        )
        .unwrap();
        assert_eq!(lines.lines[0].at[0], 13.0);
    }

    #[test]
    fn the_height_is_the_line_count_times_the_line_height() {
        let lines = layout("A\nB\nC", 100.0, TextAlign::Left, 1.0);
        let down: Vec<f32> = lines.lines.iter().map(|line| line.at[1]).collect();
        assert_eq!(down, [0.0, 9.0, 18.0]);
        assert_eq!(lines.size.height, 3.0 * 9.0);
        // A viewport sizes itself from the last line rather than the count.
        assert_eq!(lines.size.height, down.last().unwrap() + 9.0);
    }

    #[test]
    fn styles_survive_wrapping() {
        let mut text = mod_api::Text::literal("AAAA BBBB");
        text.style.bold = Some(true);
        let lines = measured(&fake_font(6.0), &text, 30.0, TextAlign::Left, 1.0).unwrap();
        assert_eq!(plain(&lines), ["AAAA", "BBBB"]);
        for line in &lines.lines {
            let line = engine(&line.text);
            assert!(
                line.chars().all(|(c, style)| c == ' ' || style.is_bold()),
                "{:?}",
                line.plain()
            );
        }
    }

    #[test]
    fn laid_out_objects_keep_their_sprites_profiles_and_fallbacks_on_redraw() {
        use mod_api::text::{TextContent, TextObject, TextProfile};
        use sunrise_client::text::InlineObject;

        let atlas = Text::new(TextContent::Object(TextObject::Atlas {
            atlas: "minecraft:gui".into(),
            sprite: "minecraft:heart/full".into(),
            fallback: Some(Box::new(Text::literal("[heart]"))),
        }));
        let mut profile = TextProfile::default();
        profile.name = Some("Alex".into());
        let mut head = Text::new(TextContent::Object(TextObject::Player {
            profile,
            hat: false,
            fallback: Some(Box::new(Text::literal("[Alex]"))),
        }));
        head.style.shadow_color = Some(0xFF00_0011);
        let input = atlas.append(head);
        let font = fake_font(6.0);
        let lines = measured(&font, &input, 150.0, TextAlign::Left, 1.0).unwrap();
        let redrawn = engine(&lines.lines[0].text);
        assert_eq!(redrawn.plain(), "[heart][Alex]");
        assert_eq!(font.width_text(&redrawn), lines.lines[0].width);
        let [atlas, head] = redrawn.spans.as_slice() else {
            panic!("expected two object spans")
        };
        assert!(matches!(&atlas.style.inline_object,
            Some(InlineObject::Atlas { atlas, sprite, .. })
            if atlas == "minecraft:gui" && sprite == "minecraft:heart/full"));
        assert!(matches!(&head.style.inline_object,
            Some(InlineObject::Player { profile, hat: false, .. })
            if profile.get("name").and_then(serde_json::Value::as_str) == Some("Alex")));
        assert_eq!(head.style.shadow_color, Some(0xFF00_0011));
    }

    #[test]
    fn layout_does_not_read_style_indices_from_mod_metadata() {
        use mod_api::{Tag, text::TextClick};
        let mut styled = Text::literal("A");
        styled.style.click = Some(TextClick::CopyToClipboard("not for B".into()));
        let mut plain = Text::literal("B");
        plain.metadata = Tag::compound([("__sunrise_local_style_id", Tag::Int(0))]);
        let input = Text::literal("").append(styled).append(plain);
        let lines = measured(&fake_font(6.0), &input, 150.0, TextAlign::Left, 1.0).unwrap();
        let [styled, plain] = lines.lines[0].text.children.as_slice() else {
            panic!("expected two styles")
        };
        assert_eq!(
            styled.style.click,
            Some(TextClick::CopyToClipboard("not for B".into()))
        );
        assert_eq!(
            plain.content,
            mod_api::text::TextContent::Literal("B".into())
        );
        assert_eq!(plain.style.click, None);
    }

    #[test]
    fn layout_copies_inherited_deep_hover_and_opaque_metadata_without_recursion() {
        use mod_api::{
            Tag,
            text::{TextContent, TextHover},
        };
        const DEPTH: usize = 4096;
        let mut hover = Text::literal("hover leaf");
        let mut metadata = Tag::Int(73);
        for _ in 0..DEPTH {
            let mut parent = Text::literal("");
            parent.style.hover = Some(TextHover::Text(Box::new(hover)));
            hover = parent;
            metadata = Tag::List(vec![metadata]);
        }
        let mut root = Text::literal("");
        root.style.hover = Some(TextHover::Text(Box::new(hover)));
        root.style.metadata = Tag::compound([("deep", metadata), ("override", Tag::Int(1))]);
        let mut child = Text::literal("A");
        child.style.metadata = Tag::compound([("override", Tag::Int(2))]);
        root.children.push(child);
        let lines = measured(&fake_font(6.0), &root, 150.0, TextAlign::Left, 1.0).unwrap();
        let style = &lines.lines[0].text.style;
        let Some(TextHover::Text(hover)) = &style.hover else {
            panic!("inherited hover")
        };
        let mut hover = hover.as_ref();
        let Tag::Compound(metadata) = &style.metadata else {
            panic!("inherited metadata")
        };
        assert_eq!(
            metadata[&sunrise_core::nbt::JavaString::from("override")],
            Tag::Int(2)
        );
        let mut deep = &metadata[&sunrise_core::nbt::JavaString::from("deep")];
        for _ in 0..DEPTH {
            let Tag::List(list) = deep else {
                panic!("nested metadata")
            };
            deep = &list[0];
        }
        assert_eq!(deep, &Tag::Int(73));
        for _ in 0..DEPTH {
            let Some(TextHover::Text(child)) = &hover.style.hover else {
                panic!("nested hover")
            };
            hover = child;
        }
        assert_eq!(hover.content, TextContent::Literal("hover leaf".into()));
    }

    #[test]
    fn a_reset_after_a_break_restores_the_style_the_tail_carried() {
        // `§lAA §rB` in a 14-pixel box: two bold characters fill the first line, the space is
        // where it breaks, and the second line starts in the bold style the split carried over.
        // Its `§r` goes back to that carried style, so `B` stays bold, and no line keeps a code.
        let lines = layout("§lAA §rB", 14.0, TextAlign::Left, 1.0);
        assert_eq!(plain(&lines), ["AA", "B"]);
        let bold: Vec<bool> = engine(&lines.lines[1].text)
            .chars()
            .map(|(_, style)| style.is_bold())
            .collect();
        assert_eq!(bold, [true], "the reset restores the carried bold style");
        for line in plain(&lines) {
            assert!(!line.contains('§'), "a line carries no code: {line:?}");
        }
    }

    #[test]
    fn scale_wraps_at_the_box_over_it_and_scales_every_answer() {
        // 36 pixels at scale 2 wrap in 24: two characters a line, 12 pixels measured and 24
        // answered, the lines 18 apart and the block 54 high.
        let lines = layout("AAAAAA", 24.0, TextAlign::Left, 2.0);
        assert_eq!(plain(&lines), ["AA", "AA", "AA"]);
        let widths: Vec<f32> = lines.lines.iter().map(|line| line.width).collect();
        assert_eq!(widths, [24.0, 24.0, 24.0]);
        let down: Vec<f32> = lines.lines.iter().map(|line| line.at[1]).collect();
        assert_eq!(down, [0.0, 18.0, 36.0]);
        assert_eq!((lines.size.width, lines.size.height), (24.0, 54.0));
        // Positions scale too: a 6-pixel line centred in 12 starts at 3, answered at 6.
        let one = layout("A", 24.0, TextAlign::Centre, 2.0);
        assert_eq!(one.lines[0].at, [6.0, 0.0]);
        assert_eq!(one.lines[0].width, 12.0);
    }

    #[test]
    fn a_width_below_one_still_wraps() {
        // `Font.split`'s one-pixel minimum: a box of no width, or one narrower than a pixel, gives
        // one character a line rather than an unwrapped line of any length.
        for max_width in [0.0, 0.5, -4.0] {
            let lines = layout("AAA", max_width, TextAlign::Left, 1.0);
            assert_eq!(plain(&lines), ["A", "A", "A"], "{max_width}");
            assert_eq!(lines.size.height, 27.0, "{max_width}");
        }
    }

    #[test]
    fn a_glyph_wider_than_the_box_gets_a_line_of_its_own() {
        // A 20-pixel character in a 6-pixel box: neither fits, and neither is dropped.
        let lines = measured(
            &fake_font(20.0),
            &Text::literal("AB"),
            6.0,
            TextAlign::Left,
            1.0,
        )
        .unwrap();
        assert_eq!(plain(&lines), ["A", "B"]);
        assert_eq!(lines.lines[0].width, 20.0);
        assert_eq!(lines.size.width, 20.0);
    }

    #[test]
    fn a_layout_refuses_a_width_or_a_scale_it_cannot_place() {
        for max_width in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(
                measured(
                    &fake_font(6.0),
                    &Text::literal("AA"),
                    max_width,
                    TextAlign::Left,
                    1.0
                )
                .unwrap_err(),
                TextLayoutError::WidthNotFinite,
                "{max_width}"
            );
        }
        for scale in [f32::NAN, f32::INFINITY, 0.0, -1.0] {
            assert_eq!(
                measured(
                    &fake_font(6.0),
                    &Text::literal("AA"),
                    100.0,
                    TextAlign::Left,
                    scale
                )
                .unwrap_err(),
                TextLayoutError::ScaleInvalid,
                "{scale}"
            );
        }
        // A finite width `scale` makes overflow is no box at all, not a box of no width.
        assert_eq!(
            measured(
                &fake_font(6.0),
                &Text::literal("AA"),
                f32::MAX,
                TextAlign::Left,
                f32::MIN_POSITIVE
            )
            .unwrap_err(),
            TextLayoutError::WidthNotFinite
        );
    }

    #[test]
    fn a_layout_visits_every_line_and_character_without_transport_limits() {
        let font = fake_font(6.0);
        let layout = text_layout::LabelLayout::try_from(TextLayoutSpec {
            max_width: 1e9,
            align: TextAlign::Left,
            scale: 1.0,
        })
        .unwrap();
        let text = Text::literal("abcdefghijklmnop\n".repeat(600));
        let mut lines = 0;
        let mut characters = 0;
        let size = layout
            .visit_components(&font, &text, &mut |line| {
                lines += 1;
                characters += engine(line.text).plain().len();
            })
            .unwrap();
        assert_eq!(lines, 601);
        assert_eq!(characters, 9600);
        assert_eq!((size.width, size.height), (96.0, 5409.0));
    }

    #[test]
    fn warmed_wrapped_paint_lends_lines_without_allocating() {
        let font = fake_font(6.0);
        let text = crate::text::Text::literal("abcd ef  ghi\njkl");
        let layout = text_layout::LabelLayout::try_from(TextLayoutSpec {
            max_width: 24.0,
            align: TextAlign::Left,
            scale: 1.0,
        })
        .unwrap();
        crate::lang::scoped(crate::lang::Language::of("en_us", []), || {
            let spec = [("abcd", 0.0), ("ef ", 9.0), ("ghi", 18.0), ("jkl", 27.0)];
            let paint = || {
                let mut lines = 0;
                let size = layout
                    .visit_visual(&font, &text, |line, at| {
                        let (expected, y) = spec[lines];
                        assert!(line.chars().map(|(c, _)| c).eq(expected.chars()));
                        assert_eq!(at, [0.0, y]);
                        lines += 1;
                    })
                    .unwrap();
                assert_eq!(lines, spec.len());
                assert_eq!((size.width, size.height), (24.0, 36.0));
            };
            paint();
            let (_, allocations) = sunrise_threads::allocation_count::measure(paint);
            assert_eq!(allocations, 0);
        });
    }

    #[test]
    fn warm_item_count_and_empty_decorations_need_no_replay_allocations() {
        use crate::hud_overlays::{ItemDecorations, paint_item_decorations};
        let mut atlas = crate::hud::HudAtlas::boot();
        atlas.font = canvas_test_font();
        let mut batch = crate::hud::HudBatch::scratch([320, 240]);
        for (count, vertices) in [(1, 0), (65, 24)] {
            let decorations = ItemDecorations {
                count,
                count_text: None,
                bar: None,
                cooldown: 0.0,
            };
            paint_item_decorations(&mut batch, &atlas, [0.0; 2], &decorations);
            batch.vertices.clear();
            let (_, allocations) = sunrise_threads::allocation_count::measure(|| {
                paint_item_decorations(&mut batch, &atlas, [0.0; 2], &decorations);
            });
            assert_eq!(batch.vertices.len(), vertices);
            assert_eq!(allocations, 0, "stack count {count}");
        }
    }

    #[test]
    #[ignore = "daily"]
    fn scaled_text_is_the_text_scaled_about_its_corner() {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            let at = [10.0, 20.0];
            let mut plain = crate::hud::HudBatch::scratch([320, 240]);
            GameDrawContext::new(game, &mut plain).text("Jump", at, 0xFFFF_FFFF, true);
            let mut scaled = crate::hud::HudBatch::scratch([320, 240]);
            GameDrawContext::new(game, &mut scaled).text_scaled("Jump", at, 0.5, 0xFFFF_FFFF, true);
            assert!(!plain.vertices.is_empty(), "the text draws glyphs");
            assert_eq!(plain.vertices.len(), scaled.vertices.len());
            for (p, s) in plain.vertices.iter().zip(&scaled.vertices) {
                let half = [
                    at[0] + (p.pos[0] - at[0]) * 0.5,
                    at[1] + (p.pos[1] - at[1]) * 0.5,
                ];
                assert_eq!(s.pos, half, "every corner halfway to `at`");
                assert_eq!(
                    (s.uv, s.color),
                    (p.uv, p.color),
                    "the same glyphs and colours"
                );
            }
        });
    }

    #[test]
    fn a_wrapped_label_measures_and_draws_its_own_height() {
        let mut atlas = crate::hud::HudAtlas::boot();
        atlas.font = canvas_test_font();
        let spec = TextLayoutSpec {
            max_width: 32.0,
            align: TextAlign::Left,
            scale: 2.0,
        };
        let text = Text::literal("§cmmmm §aאבג §bmmmm");
        let mut actual = crate::hud::HudBatch::scratch([320, 240]);
        gui_canvas::GameGuiCanvas::draw(&mut actual, &atlas, None, None, &mut |canvas| {
            canvas.scope(
                &[mod_api::client::hud_layout::PoseOp::Translate(4.0, 6.0)],
                Some([5.0, 10.0, 34.0, 53.0]),
                &mut |scoped| {
                    let size =
                        scoped.text_with_word_wrap(&text, [10.0, 20.0], spec, u32::MAX, false)?;
                    assert_eq!(
                        size,
                        TextLayoutSize {
                            width: 32.0,
                            height: 54.0
                        }
                    );
                    Ok(())
                },
            )
        })
        .unwrap();

        let runs: Vec<_> = actual.scissors_in(0..66).collect();
        assert_eq!(runs, [(0..66, Some([9.0, 16.0, 43.0, 69.0]))]);
        let clip = runs[0].1.unwrap();
        // The final column and row are partly clipped, but the glyph UVs still
        // address the original cells. The Hebrew line's first glyph is gimel.
        let glyphs = [
            ([14.0, 26.0, 22.0, 42.0], [22.0, 42.0], 0.0, 0xFF55_55FF),
            ([22.0, 26.0, 30.0, 42.0], [30.0, 42.0], 0.0, 0xFF55_55FF),
            ([30.0, 26.0, 38.0, 42.0], [38.0, 42.0], 0.0, 0xFF55_55FF),
            ([38.0, 26.0, 46.0, 42.0], [43.0, 42.0], 0.0, 0xFF55_55FF),
            ([14.0, 44.0, 22.0, 60.0], [22.0, 60.0], 12.0, 0xFF55_FF55),
            ([20.0, 44.0, 28.0, 60.0], [28.0, 60.0], 8.0, 0xFF55_FF55),
            ([26.0, 44.0, 34.0, 60.0], [34.0, 60.0], 4.0, 0xFF55_FF55),
            ([14.0, 62.0, 22.0, 78.0], [22.0, 69.0], 0.0, 0xFFFF_FF55),
            ([22.0, 62.0, 30.0, 78.0], [30.0, 69.0], 0.0, 0xFFFF_FF55),
            ([30.0, 62.0, 38.0, 78.0], [38.0, 69.0], 0.0, 0xFFFF_FF55),
            ([38.0, 62.0, 46.0, 78.0], [43.0, 69.0], 0.0, 0xFFFF_FF55),
        ];
        assert_eq!(actual.vertices.len(), glyphs.len() * 6);
        for (quad, ([l, t, r, b], [visible_r, visible_b], u, color)) in
            actual.vertices.as_chunks::<6>().0.iter().zip(glyphs)
        {
            let corners = [[l, t], [r, t], [r, b], [l, b]];
            let (min_u, max_u) = ((u + 0.01) / 32.0, (u + 4.0 - 0.01) / 32.0);
            let (min_v, max_v) = (0.01 / 16.0, 7.99 / 16.0);
            let uvs = [
                [min_u, min_v],
                [max_u, min_v],
                [max_u, max_v],
                [min_u, max_v],
            ];
            for (v, corner) in quad.iter().zip([0, 1, 2, 0, 2, 3]) {
                assert_eq!(
                    (v.pos, v.uv, v.color, v.text),
                    (corners[corner], uvs[corner], color, 1)
                );
            }
            assert_eq!(
                [
                    quad[0].pos[0].max(clip[0]),
                    quad[0].pos[1].max(clip[1]),
                    quad[2].pos[0].min(clip[2]),
                    quad[2].pos[1].min(clip[3])
                ],
                [l, t, visible_r, visible_b],
            );
        }
    }

    #[test]
    #[ignore = "daily"]
    fn local_tooltip_draws_deep_text_but_refuses_unrepresentable_metadata() {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            let mut deep = mod_api::Text::literal("Tooltip");
            for _ in 0..mod_api::text::MAX_TEXT_DEPTH {
                deep = mod_api::Text::literal("").append(deep);
            }
            let mut invalid = mod_api::Text::literal("Unrepresentable");
            invalid.metadata = Tag::compound([("test:nonfinite", Tag::Double(f64::NAN.to_bits()))]);

            let mut batch = crate::hud::HudBatch::scratch([400, 200]);
            let mut draw = GameDrawContext::new(game, &mut batch);
            assert_eq!(
                draw.gui_tooltip(&[deep.clone(), invalid], [16.0, 16.0]),
                Err(TextFormError::NonFinite)
            );
            assert!(draw.surface.vertices.is_empty());
            draw.gui_tooltip(&[deep], [16.0, 16.0]).unwrap();
            assert!(!draw.surface.vertices.is_empty());
        });
    }

    #[test]
    fn local_label_layout_handles_deep_trees_and_preserves_opaque_styles() {
        let mut opaque_dialog = Tag::ByteArray(vec![7, 8]);
        for _ in 0..600 {
            let mut map = std::collections::BTreeMap::new();
            map.insert("n".into(), opaque_dialog);
            opaque_dialog = Tag::Compound(map.into_iter().collect());
        }
        let mut style = mod_api::text::TextStyle::default();
        style.bold = Some(true);
        style.click = Some(mod_api::text::TextClick::ShowDialog(opaque_dialog.clone()));
        let mut deep = mod_api::Text::literal("AAAA BBBB").with_style(style);
        for _ in 0..mod_api::text::MAX_TEXT_DEPTH {
            let mut parent = mod_api::Text::literal("");
            parent.children.push(deep);
            deep = parent;
        }
        let lines = measured(&fake_font(6.0), &deep, 30.0, TextAlign::Left, 1.0).unwrap();
        assert_eq!(lines.lines.len(), 2);
        for line in &lines.lines {
            assert_eq!(line.text.style.bold, Some(true));
            assert_eq!(
                line.text.style.click,
                Some(mod_api::text::TextClick::ShowDialog(opaque_dialog.clone()))
            );
        }
    }

    #[test]
    fn engine_tooltip_font_measures_and_splits_deep_local_text() {
        let default_json = r#"{"providers":[{"type":"space","advances":{" ":2,"a":3}}]}"#;
        let alt_json = r#"{"providers":[{"type":"space","advances":{" ":4,"a":8}}]}"#;
        let source = crate::resource_source::ResourceSource::indexed(BTreeMap::from([
            (
                "assets/minecraft/font/default.json".into(),
                crate::resource_source::Resource::bytes(default_json.as_bytes().to_vec()),
            ),
            (
                "assets/minecraft/font/alt.json".into(),
                crate::resource_source::Resource::bytes(alt_json.as_bytes().to_vec()),
            ),
        ]));
        let font = crate::font::Font::metrics(&source);
        let default_id = Identifier::parse("minecraft:default").unwrap();
        let alt_id = Identifier::parse("minecraft:alt").unwrap();

        let default_tf = EngineTooltipFont::new(&font, &default_id);
        let alt_tf = EngineTooltipFont::new(&font, &alt_id);
        assert_eq!(default_tf.id(), &default_id);
        assert_eq!(alt_tf.id(), &alt_id);
        assert_eq!(default_tf.line_height(), 9);

        let sample = mod_api::Text::literal("aa aa");
        assert_eq!(default_tf.width(&sample).unwrap(), 14);
        assert_eq!(alt_tf.width(&sample).unwrap(), 36);
        assert_eq!(default_tf.split(&sample, 16).unwrap().len(), 1);
        assert_eq!(alt_tf.split(&sample, 16).unwrap().len(), 2);

        let mut deep = mod_api::Text::literal("x");
        for _ in 0..600 {
            deep = mod_api::Text::literal("").append(deep);
        }
        assert_eq!(
            default_tf.width(&deep),
            default_tf.width(&mod_api::Text::literal("x"))
        );
        assert_eq!(default_tf.split(&deep, 100).unwrap().len(), 1);
    }
}
