//! The game's side of the `draw` area (`horizon-mod-api/src/host/draw.rs`).
use crate::game::{Game, api_values::TryIntoEngine};
use mod_api::{prelude::*, *};

/// [`DrawContext`] on the game's `crate::hud::HudBatch`.
pub(crate) struct GameDrawContext<'a, 'g> {
    pub(crate) game: &'a Game<'g>,
    pub(crate) surface: &'a mut crate::hud::HudBatch,
}
impl DrawContext for GameDrawContext<'_, '_> {
    fn size(&self) -> [f32; 2] {
        self.game.native_size(&*self.surface)
    }
    fn item_model(&mut self, item: &str, model: &str, at: [f32; 2], size: f32) -> bool {
        self.game
            .native_item_model(&mut *self.surface, item, model, at, size)
    }
    fn quad(&mut self, points: [[f32; 2]; 4], argb: u32) {
        self.game.native_quad(&mut *self.surface, points, argb)
    }
    fn fill(&mut self, rect: [f32; 4], argb: u32) {
        self.game.native_fill(&mut *self.surface, rect, argb)
    }
    fn sprite(&mut self, name: &str, src: [f32; 4], dst: [f32; 4], tint: Option<u32>) -> bool {
        self.game
            .native_sprite(&mut *self.surface, name, src, dst, tint)
    }
    fn item(&mut self, item: &str, at: [f32; 2], size: f32) -> bool {
        self.game.native_item(&mut *self.surface, item, at, size)
    }
    fn text(&mut self, text: &str, at: [f32; 2], argb: u32, shadow: bool) {
        self.game
            .native_text(&mut *self.surface, text, at, argb, shadow)
    }
    fn item_stack(&mut self, stack: &ItemStack, at: [f32; 2], size: f32) -> bool {
        let Ok(stack) = stack.try_into_engine(self.game.items()) else {
            return false;
        };
        self.game
            .native_item_stack(&mut *self.surface, &stack, at, size)
    }
    fn gui_paint(&mut self, draws: &[GuiDraw]) {
        self.game.native_gui_paint(&mut *self.surface, draws)
    }
    fn gui_nine_slice(&mut self, sprite: &str, rect: [f32; 4], color: u32) {
        self.game
            .native_gui_nine_slice(&mut *self.surface, sprite, rect, color)
    }
    fn gui_tooltip(&mut self, lines: &[Text], at: [f32; 2]) {
        self.game.native_gui_tooltip(&mut *self.surface, lines, at)
    }
    fn text_width(&self, text: &str) -> f32 {
        self.game.native_text_width(&*self.surface, text)
    }
    fn fluid(&mut self, fluid: &str, at: [f32; 2], size: f32) {
        self.game.native_fluid(&mut *self.surface, fluid, at, size)
    }
    fn draw_scene(&mut self, rect: [f32; 4], blocks: &[NativeSceneBlock], view: NativeSceneView) {
        self.game
            .native_draw_scene(&mut *self.surface, rect, blocks, view)
    }
    fn draw_menu_background(&mut self, rect: [f32; 4], in_world: bool) {
        self.game
            .native_draw_menu_background(&mut *self.surface, rect, in_world)
    }
    fn tiled_sprite(&mut self, sprite: &str, rect: [f32; 4], size: f32, argb: u32) -> bool {
        self.game
            .native_tiled_sprite(&mut *self.surface, sprite, rect, size, argb)
    }
    fn gui_draw_item_with(&mut self, stack: &ItemStack, at: [f32; 2], text: Option<&str>) {
        let Ok(stack) = stack.try_into_engine(self.game.items()) else {
            return;
        };
        self.game
            .native_gui_draw_item_with(&mut *self.surface, &stack, at, text)
    }
}
/// [`BlockEntityDrawContext`] on the game's `crate::game::native_block_entity_draw::BlockEntityDraw`.
pub(crate) struct GameBlockEntityDrawContext<'a, 'g> {
    pub(crate) game: &'a Game<'g>,
    pub(crate) surface: &'a mut crate::game::native_block_entity_draw::BlockEntityDraw,
}
impl BlockEntityDrawContext for GameBlockEntityDrawContext<'_, '_> {
    fn push(&mut self) {
        self.game.native_draw_push(&mut *self.surface)
    }
    fn pop(&mut self) {
        self.game.native_draw_pop(&mut *self.surface)
    }
    fn translate(&mut self, by: [f32; 3]) {
        self.game.native_draw_translate(&mut *self.surface, by)
    }
    fn rotate_about(&mut self, axis: [f32; 3], degrees: f32, pivot: [f32; 3]) {
        self.game
            .native_draw_rotate_about(&mut *self.surface, axis, degrees, pivot)
    }
    fn scale(&mut self, by: [f32; 3]) {
        self.game.native_draw_scale(&mut *self.surface, by)
    }
    fn keep(&mut self) {
        self.game.native_draw_keep(&mut *self.surface)
    }
    fn partial(&self) -> f32 {
        self.game.native_draw_partial(&*self.surface)
    }
    fn game_time(&self) -> i64 {
        self.game.native_draw_game_time(&*self.surface)
    }
    fn light(&self) -> u32 {
        self.game.native_draw_light(&*self.surface)
    }
    fn set_light(&mut self, light: u32) {
        self.game.native_draw_set_light(&mut *self.surface, light)
    }
    fn quad(&mut self, sprite: &str, corners: [[f32; 3]; 4], uv: [f32; 4], tint: u32) -> bool {
        self.game
            .native_draw_quad(&mut *self.surface, sprite, corners, uv, tint)
    }
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
    fn block(&mut self, block: &str, properties: &[(&str, &str)]) -> bool {
        self.game
            .native_draw_block(&mut *self.surface, block, properties)
    }
    fn item(
        &mut self,
        stack: &ItemStack,
        display: NativeItemDisplayContext,
        owner_yaw: Option<f32>,
        seed: i32,
    ) -> bool {
        let Ok(stack) = stack.try_into_engine(self.game.items()) else {
            return false;
        };
        let Ok(display) = display.try_into_engine(self.game.items()) else {
            return false;
        };
        self.game
            .native_draw_item(&mut *self.surface, &stack, display, owner_yaw, seed)
    }
    fn spin(&mut self, axis: [f32; 3], pivot: [f32; 3], degrees_per_tick: f32, phase: f32) {
        self.game
            .native_draw_spin(&mut *self.surface, axis, pivot, degrees_per_tick, phase)
    }
    fn still(&mut self) {
        self.game.native_draw_still(&mut *self.surface)
    }
    fn model(&mut self, model: ModelRef, tint: u32) -> Result<(), ModelError> {
        self.game.native_draw_model(&mut *self.surface, model, tint)
    }
    fn text(&mut self, text: &Text, at: [f32; 2], color: u32, glowing: bool, outline: u32) -> f32 {
        self.game
            .native_draw_text(&mut *self.surface, text, at, color, glowing, outline)
    }
    fn text_width(&self, text: &str) -> f32 {
        self.game.native_draw_text_width(&*self.surface, text)
    }
    fn model_part(&mut self, part: &ModelPart, sprite: &str, tint: u32) -> bool {
        self.game
            .native_draw_model_part(&mut *self.surface, part, sprite, tint)
    }
}
/// [`WorldDrawContext`] on the game's `crate::world_geometry::WorldGeometry`.
pub(crate) struct GameWorldDrawContext<'a, 'g> {
    pub(crate) game: &'a Game<'g>,
    pub(crate) surface: &'a mut crate::world_geometry::WorldGeometry,
}
impl WorldDrawContext for GameWorldDrawContext<'_, '_> {
    fn lines(&mut self, lines: &[WorldLine]) -> Result<(), DrawError> {
        self.game.native_world_draw_lines(&mut *self.surface, lines)
    }
    fn quads(&mut self, quads: &[WorldQuad]) -> Result<(), DrawError> {
        self.game.native_world_draw_quads(&mut *self.surface, quads)
    }
    fn usage(&self) -> WorldDrawUsage {
        self.game.native_world_draw_usage(&*self.surface)
    }
    fn draw_model_instances(
        &mut self,
        model: ModelRef,
        instances: &[ModelInstance],
    ) -> Result<(), InstanceError> {
        self.game
            .native_world_draw_model_instances(&mut *self.surface, model, instances)
    }
}
