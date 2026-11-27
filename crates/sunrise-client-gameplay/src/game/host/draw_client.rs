use crate::game::host::draw::GameBlockEntityDrawContext;
use crate::game::host::draw::GameCameraDrawContext;
use crate::game::host::draw::GameDrawContext;
use crate::game::host::draw::GameWorldDrawContext;
use crate::game::host::draw::{EngineTooltipFont, font_id_str};
use mod_api::{prelude::*, *};
use sunrise_gameplay_common::game::api_values::{TryIntoApi, TryIntoEngine};

impl crate::game::ClientGame<'_> {
    pub fn draw_gui_canvas(
        &self,
        batch: &mut sunrise_client_render::hud::HudBatch,
        scissor: Option<[f32; 4]>,
        draw: &mut dyn FnMut(&mut dyn GuiCanvas) -> Result<(), GuiDrawError>,
    ) -> Result<(), GuiDrawError> {
        crate::game::host::draw::gui_canvas::GameGuiCanvas::draw(
            batch,
            &self.client.gui.atlas,
            Some(self),
            scissor,
            draw,
        )
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
    fn crumbling_stage(&self) -> Option<u8> {
        self.game.native_draw_crumbling_stage(&*self.surface)
    }
    #[inline]
    fn crumbling(&mut self, on: bool) {
        self.game.native_draw_crumbling(&mut *self.surface, on)
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
        let Ok(stack) = sunrise_gameplay_common::game::api_values::stack_value(stack) else {
            return false;
        };
        let Ok(display) = display.try_into_engine(GameItems::items(self.game)) else {
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
        let text = sunrise_sim::api_text::to_engine(text)?;
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
        let text = sunrise_sim::api_text::to_engine_inprocess(text);
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
    #[inline]
    fn block_entity(&mut self, entity: &SceneBlockEntity) -> Result<(), BlockEntityDrawRefusal> {
        let state = entity.state.try_into_engine(&()).ok();
        self.game
            .native_draw_block_entity(&mut *self.surface, state, entity)
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

impl Drop for GameDrawContext<'_, '_> {
    #[inline]
    fn drop(&mut self) {
        if let Some(clip) = self.clip.take() {
            clip.close(self.surface);
        }
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

impl TooltipFont for EngineTooltipFont<'_> {
    fn id(&self) -> &Identifier {
        self.id
    }
    fn line_height(&self) -> i32 {
        9
    }
    fn width(&self, text: &Text) -> Result<i32, TextFormError> {
        let text: sunrise_game_data::text::Text = text.try_into_engine(&())?;
        Ok(self.font.width_text_with_font(&text, self.name()) as i32)
    }
    fn split(&self, text: &Text, max_width: i32) -> Result<Vec<Text>, TextFormError> {
        let text: sunrise_game_data::text::Text = text.try_into_engine(&())?;
        self.font
            .split_formatted_with_font(&text, max_width as f32, self.name())
            .into_iter()
            .map(|line| line.try_into_api())
            .collect()
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
        let slot = self
            .game
            .client
            .client_pack
            .render
            .mod_pipelines
            .slot(pipeline)?;
        Ok(self.game.pack_override_state(slot as usize))
    }
}

impl<'a, 'g, 'v> GameBlockEntityDrawContext<'a, 'g, 'v> {
    pub(crate) fn new(
        game: &'a crate::game::ClientGame<'g>,
        surface: &'a mut crate::game::native_block_entity_draw::BlockEntityDraw<'v>,
    ) -> Self {
        Self { game, surface }
    }
}

impl<'a, 'g> GameCameraDrawContext<'a, 'g> {
    pub(crate) fn new(
        game: &'a crate::game::ClientGame<'g>,
        surface: &'a mut sunrise_client::screen_effects::CameraQuads,
    ) -> Self {
        Self { game, surface }
    }
}

impl<'a, 'g> GameDrawContext<'a, 'g> {
    pub fn new(
        game: &'a crate::game::ClientGame<'g>,
        surface: &'a mut sunrise_client_render::hud::HudBatch,
    ) -> Self {
        Self {
            game,
            surface,
            clip: None,
            tooltip_font: None,
        }
    }
    /// Lend a fresh clip stack and isolate the drawable tail of one listener.
    pub(crate) fn stage(
        game: &crate::game::ClientGame<'_>,
        surface: &mut sunrise_client_render::hud::HudBatch,
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
    pub(in crate::game) fn with_active_tooltip_font(
        mut self,
        font: &'a EngineTooltipFont<'a>,
    ) -> Self {
        self.tooltip_font = Some(font);
        self
    }
}

impl<'a, 'g> GameWorldDrawContext<'a, 'g> {
    pub(crate) fn new(
        game: &'a crate::game::ClientGame<'g>,
        surface: &'a mut sunrise_client::world_geometry::WorldGeometry,
    ) -> Self {
        Self { game, surface }
    }
}

impl<'a> EngineTooltipFont<'a> {
    pub(in crate::game) fn name(&self) -> &str {
        &self.name
    }
    pub fn new(font: &'a sunrise_client::font::Font, id: &'a Identifier) -> Self {
        Self {
            font,
            id,
            name: font_id_str(id),
        }
    }
}

use sunrise_gameplay_common::game::tables::GameItems;
