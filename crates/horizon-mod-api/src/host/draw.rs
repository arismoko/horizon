//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The client host's methods of the `draw` area, implemented by the game and reached through [`crate::ClientHost`].
#[horizon_mod_api_macros::host_area]
pub trait DrawClientHost {
    /// Side: client.
    /// Fluid `fluid`'s name as the lang names it (`fluid_type.<mod>.<name>`, vanilla's by its block),
    /// else its path in words.
    #[access(ClientSetup)]
    fn fluid_name(&self, fluid: &str) -> String;
    /// Side: client.
    /// Entity model layer `layer` baked into its `ModelPart` tree, each part
    /// at its pose, as `EntityModelSet.bakeLayer` bakes it: one a native mod
    /// registered (`native_layer_definitions`), or vanilla's, by the name
    /// `entity-models.json` gives it (`minecraft:zombie#main`,
    /// `minecraft:player#main`). The tree is the caller's own copy: bake it
    /// once, as a renderer does, and set its parts' poses each frame. None
    /// for a layer neither has. A vanilla layer no mob has (a player's, a
    /// chest's) is read from the resources each call, so slow. API
    /// @API:model_layers@.
    #[access(ClientSetup)]
    fn bake_layer(&self, layer: &str) -> Option<ModelPart>;
}
/// What the HUD, screen and tooltip hooks draw with, in GUI pixels: one per hook call.
pub trait DrawContext {
    /// The GUI's size in GUI pixels, width then height.
    fn size(&self) -> [f32; 2] {
        unimplemented!("DrawContext::size")
    }
    /// Item `item`'s icon drawn with model `model` (`minecraft:paper` as
    /// `mymod:wheel_icon`, an `item_model` of a content mod's), as `native_item`
    /// draws one; false for an unknown item.
    fn item_model(&mut self, item: &str, model: &str, at: [f32; 2], size: f32) -> bool {
        unimplemented!("DrawContext::item_model")
    }
    /// A quad of corners `points` (in order round it) in GUI pixels, `argb`:
    /// a wedge, a slanted bar, what `native_fill`'s rectangles can't draw.
    fn quad(&mut self, points: [[f32; 2]; 4], argb: u32) {
        unimplemented!("DrawContext::quad")
    }
    /// A rectangle in GUI pixels, `argb` (0x78000000 is the extras' box).
    fn fill(&mut self, rect: [f32; 4], argb: u32) {
        unimplemented!("DrawContext::fill")
    }
    /// Part `src` (`[x, y, w, h]` in the sprite's own pixels) of sprite
    /// `name` (vanilla's `gui/...`, or a content mod's GUI texture
    /// `mymod:gui/...`), drawn stretched into `dst` (`[x, y, w, h]` in GUI
    /// pixels), every texel multiplied by `tint` (`0xAARRGGBB`, a fluid's colour)
    /// when there is one. False when there is no such sprite.
    fn sprite(&mut self, name: &str, src: [f32; 4], dst: [f32; 4], tint: Option<u32>) -> bool {
        unimplemented!("DrawContext::sprite")
    }
    /// Item `item`'s icon (`mymod:machine`) with its top left at `at`,
    /// `size` GUI pixels square (16 in a slot); false for an unknown item.
    fn item(&mut self, item: &str, at: [f32; 2], size: f32) -> bool {
        unimplemented!("DrawContext::item")
    }
    /// A line of text at `(x, y)` in GUI pixels, in `argb` as vanilla's `GuiGraphics` takes it
    /// (`0xFFFFFFFF` is opaque white). Fully transparent text draws nothing, as vanilla's.
    fn text(&mut self, text: &str, at: [f32; 2], argb: u32, shadow: bool) {
        unimplemented!("DrawContext::text")
    }
    /// Draw a stack and its slot decorations, except the count, at `at`,
    /// `size` GUI pixels square. False when nothing was drawn: an unknown item
    /// or an empty stack (air, or a count of 0, as `ItemStack.isEmpty`).
    fn item_stack(&mut self, stack: &ItemStack, at: [f32; 2], size: f32) -> bool {
        unimplemented!("DrawContext::item_stack")
    }
    /// Paint GUI extraction calls through this window's atlas, including player faces.
    fn gui_paint(&mut self, draws: &[GuiDraw]) {
        unimplemented!("DrawContext::gui_paint")
    }
    /// Vanilla's nine-slice widget rendering. `color` is ARGB, as `fill` uses.
    fn gui_nine_slice(&mut self, sprite: &str, rect: [f32; 4], color: u32) {
        unimplemented!("DrawContext::gui_nine_slice")
    }
    /// A vanilla tooltip at the cursor, in GUI pixels.
    fn gui_tooltip(&mut self, lines: &[Text], at: [f32; 2]) {
        unimplemented!("DrawContext::gui_tooltip")
    }
    /// The width of `text` in GUI pixels, as `native_text` draws it on `draw` (`Font.width`).
    fn text_width(&self, text: &str) -> f32 {
        unimplemented!("DrawContext::text_width")
    }
    /// Fluid `fluid` (`minecraft:water`, `mymod:oil`) drawn as a square with its top left at `at`, `size`
    /// GUI pixels wide (16 in a slot): the sprite its look names (`data/<mod>/horizon/fluid/<name>.json`)
    /// tinted with its colour, else a flat square of the colour, which vanilla water and lava have. What a
    /// recipe viewer draws for a fluid stack; the slot and the amount are the mod's.
    fn fluid(&mut self, fluid: &str, at: [f32; 2], size: f32) {
        unimplemented!("DrawContext::fluid")
    }
    /// Draws `blocks`, a scene of block states at positions of its own (not
    /// the world's), into the box `rect` (x0, y0, x1, y1, GUI pixels) of a
    /// screen's `batch`, seen as `view` says: Create's Ponder scenes. Each
    /// block is drawn with its model, full bright and shaded by face, a face
    /// against a solid neighbour in the scene left out; a block with an
    /// `entity` (a mod's) is drawn by `native_block_entity_submit` as in the
    /// world too, its `spin` parts turning with the game time. What lies
    /// outside the box is cut off, and the scene has a depth of its own, so
    /// it never covers the screen's other drawing or is covered by it but in
    /// the order drawn. Draw it from a screen's drawing hook
    /// (`native_screen_draw`). A client's call; nothing is drawn on a server.
    /// API @API:scenes_in_screens@.
    fn draw_scene(&mut self, rect: [f32; 4], blocks: &[NativeSceneBlock], view: NativeSceneView) {
        unimplemented!("DrawContext::draw_scene")
    }
    /// `Screen.renderMenuBackground`: draws the tiled menu background (32x32) across `rect`
    /// (`[x, y, w, h]` in GUI pixels). If `in_world` is true, uses the dark translucent
    /// texture (`gui/inworld_menu_background`); otherwise uses the opaque dirt-like
    /// texture (`gui/menu_background`). API @API:screen_transparent_background@.
    fn draw_menu_background(&mut self, rect: [f32; 4], in_world: bool) {
        unimplemented!("DrawContext::draw_menu_background")
    }
    /// Repeats sprite `sprite` across `rect` (`[x, y, w, h]` in GUI pixels) with each tile
    /// sized `size` GUI pixels square, tinted by `argb`. False when the sprite is not in the HUD atlas.
    /// API @API:screen_transparent_background@.
    fn tiled_sprite(&mut self, sprite: &str, rect: [f32; 4], size: f32, argb: u32) -> bool {
        unimplemented!("DrawContext::tiled_sprite")
    }
    /// Draw an item stack with custom count/decoration text (or None) at `at` in GUI units.
    fn gui_draw_item_with(&mut self, stack: &ItemStack, at: [f32; 2], text: Option<&str>) {
        unimplemented!("DrawContext::gui_draw_item_with")
    }
}
/// What a block entity, mob or player submit hook draws with, in blocks under a pose stack (`PoseStack`): one per hook call.
pub trait BlockEntityDrawContext {
    /// `PoseStack.pushPose`: `pop` comes back to the pose as it is now.
    fn push(&mut self) {
        unimplemented!("BlockEntityDrawContext::draw_push")
    }
    /// `PoseStack.popPose`; the block's own pose is never popped.
    fn pop(&mut self) {
        unimplemented!("BlockEntityDrawContext::draw_pop")
    }
    /// Moves what is drawn next by `by`, in blocks.
    fn translate(&mut self, by: [f32; 3]) {
        unimplemented!("BlockEntityDrawContext::draw_translate")
    }
    /// Turns what is drawn next by `degrees` about `axis` (counterclockwise
    /// seen from the axis' positive end, as `Axis.rotationDegrees`) through
    /// `pivot`, in blocks from the pose's origin: `[0.5; 3]` is the block's centre.
    fn rotate_about(&mut self, axis: [f32; 3], degrees: f32, pivot: [f32; 3]) {
        unimplemented!("BlockEntityDrawContext::draw_rotate_about")
    }
    /// Scales what is drawn next by `by` about the pose's origin.
    fn scale(&mut self, by: [f32; 3]) {
        unimplemented!("BlockEntityDrawContext::draw_scale")
    }
    /// Keeps what this call drew, with neither `native_block_entity_submit` nor
    /// `native_block_entity_render_bounding_box` called until its state, block entity,
    /// light or resource packs change. Parts marked with `spin` keep turning from the
    /// game time; other geometry stays as submitted. Do not bake frame-dependent
    /// values into kept vertices: item glint and time-dependent item models stop
    /// where they were, and neighboring blocks do not invalidate the cache.
    fn keep(&mut self) {
        unimplemented!("BlockEntityDrawContext::draw_keep")
    }
    /// The frame's partial tick, 0 to 1 (`BlockEntityRenderState`).
    fn partial(&self) -> f32 {
        unimplemented!("BlockEntityDrawContext::draw_partial")
    }
    /// The level's game time, for animations.
    fn game_time(&self) -> i64 {
        unimplemented!("BlockEntityDrawContext::draw_game_time")
    }
    /// The light the helpers draw with: the block's own
    /// (`LevelRenderer.getLightCoords`), 16 times sky light in bits 0..8 and
    /// 16 times block light in bits 8..16.
    fn light(&self) -> u32 {
        unimplemented!("BlockEntityDrawContext::draw_light")
    }
    /// Lights what is drawn next with `light` (as `native_draw_light` reads it).
    fn set_light(&mut self, light: u32) {
        unimplemented!("BlockEntityDrawContext::draw_set_light")
    }
    /// Draws a quad with `corners` in blocks under `draw`'s pose, in a model
    /// face's order (top left, bottom left, bottom right, top right as seen
    /// from its front), textured with the `uv` rectangle (u0, v0, u1, v1 in
    /// texels of 16 per side) of `sprite` (`mymod:entity/screen`,
    /// `mymod:block/casing`, `block/stone`) times the 0xRRGGBB `tint`
    /// (0xFFFFFF for none). False for an unknown sprite.
    fn quad(&mut self, sprite: &str, corners: [[f32; 3]; 4], uv: [f32; 4], tint: u32) -> bool {
        unimplemented!("BlockEntityDrawContext::draw_quad")
    }
    /// Draws a box from `from` to `to` (texels of 16 per block, as a block
    /// model's element) under `draw`'s pose, with the faces given, by
    /// `Direction` (down, up, north, south, west, east), baked as a model
    /// element's are (`FaceBakery`), times the 0xRRGGBB `tint`. A turned box
    /// is drawn under a turned pose (`BlockEntityDraw::rotate_about`). False
    /// when a face's sprite is unknown; that face is left out.
    fn cube(
        &mut self,
        from: [f32; 3],
        to: [f32; 3],
        faces: [Option<CubeFace<'_>>; 6],
        tint: u32,
    ) -> bool {
        unimplemented!("BlockEntityDrawContext::draw_cube")
    }
    /// Draws block `block`'s model (`mymod:cog`, any block with a model) in
    /// the state with `properties` (`[("axis", "x")]`, the rest default)
    /// under `draw`'s pose, as vanilla's `submitBlockModel` draws a block in
    /// an entity's place: a block's baked model, with its default tints. A
    /// moving part that is no block is a standalone model
    /// (`native_draw_model`). False for an unknown block or property.
    fn block(&mut self, block: &str, properties: &[(&str, &str)]) -> bool {
        unimplemented!("BlockEntityDrawContext::draw_block")
    }
    /// Draws `stack` under `draw`'s pose as an item is drawn in the world
    /// (`ItemModelResolver.updateForTopItem` and `ItemStackRenderState.submit`), centred on the
    /// pose's origin, in its model's `display` transform: `Fixed` as a
    /// campfire's or an item frame's, `Ground` as a dropped item's. `seed`
    /// picks among random models (a campfire passes its slot plus its
    /// position's hash). `owner_yaw` makes the block entity the item's
    /// owner (`ItemOwner`), at the block's centre and turned that many
    /// degrees (`getVisualRotationYInDegrees`), as a shelf is: a compass on
    /// it points to its target from there. None draws it with no owner, as
    /// a campfire does. False for an empty stack or one with no model. The
    /// client's copy of a block entity has no item slots (`entity.items` is
    /// empty there): send the stacks you draw in
    /// `native_block_entity_update_tag` and read them from `entity.nbt`.
    fn item(
        &mut self,
        stack: &ItemStack,
        display: NativeItemDisplayContext,
        owner_yaw: Option<f32>,
        seed: i32,
    ) -> bool {
        unimplemented!("BlockEntityDrawContext::draw_item")
    }
    /// Begin time-based rotation using the surface's one implementation.
    fn spin(&mut self, axis: [f32; 3], pivot: [f32; 3], degrees_per_tick: f32, phase: f32) {
        unimplemented!("BlockEntityDrawContext::draw_spin")
    }
    /// End time-based rotation for subsequent geometry.
    fn still(&mut self) {
        unimplemented!("BlockEntityDrawContext::draw_still")
    }
    /// Draws the resolved standalone model under `draw`'s pose: unturned, from
    /// the block's lower north-west corner, lit by `draw.light`. Faces with a tint
    /// index are multiplied by `tint` (`0xAARRGGBB`, the same as [`crate::WorldQuad::color`];
    /// `0xFFFFFFFF` for none), alpha included. Faces with no tint index stay opaque.
    /// A stale generation or absent slot returns an error without appending geometry.
    fn model(&mut self, model: ModelRef, tint: u32) -> Result<(), ModelError> {
        unimplemented!("BlockEntityDrawContext::draw_model")
    }
    /// Draws `text` under `draw`'s pose as a sign draws its lines
    /// (`Font.drawInBatch` in a `BlockEntityRenderer`): in the pose's x-y
    /// plane, one unit a font pixel, y down, its left end at `at`, facing
    /// +z. Scale the pose first as a sign does (`draw.scale([1.0 / 64.0,
    /// -1.0 / 64.0, 1.0 / 64.0])` for its letters, 8 pixels high). Each span
    /// is drawn in its own colour, the others in the 0xRRGGBB `color`;
    /// `glowing` text is full bright (a glow ink sac's), the rest lit by
    /// `draw.light`; a nonzero 0xRRGGBB `outline` rings each glyph, as glowing
    /// sign text's. Returns the width drawn, in font pixels: centre a line
    /// by starting it at minus half its width. API @API:draw_text@.
    fn text(&mut self, text: &Text, at: [f32; 2], color: u32, glowing: bool, outline: u32) -> f32 {
        unimplemented!("BlockEntityDrawContext::draw_text")
    }
    /// The width of `text` in font pixels, as `draw_text` draws it, to centre a
    /// line (`Font.width`).
    fn text_width(&self, text: &str) -> f32 {
        unimplemented!("BlockEntityDrawContext::draw_text_width")
    }
    /// Draws model part `part` and the parts under it under `draw`'s pose, as
    /// vanilla's `ModelPart.render` does: each part moved to its place (in
    /// texels, 16 to a block), turned about z, then y, then x, and scaled,
    /// its faces textured from `sprite` (`mymod:entity/golem`, a content
    /// mod's `assets/mymod/textures/entity/golem.png`, or vanilla's
    /// `entity/zombie/zombie`) times the 0xRRGGBB `tint`. A part that isn't
    /// `visible` draws nothing, one that skips drawing (`skip_draw`) only its
    /// children. A mob's model is built in the model's own frame (+y down,
    /// its front towards -z, its feet 24 texels down): in `native_mob_submit`,
    /// `draw.scale([1.0, -1.0, -1.0])` then `draw.translate([0.0, -1.501,
    /// 0.0])` first, as `LivingEntityRenderer` does. False for an unknown
    /// sprite. API @API:model_layers@.
    fn model_part(&mut self, part: &ModelPart, sprite: &str, tint: u32) -> bool {
        unimplemented!("BlockEntityDrawContext::draw_model_part")
    }
}
/// Per-view untextured geometry, cleared even without listeners; `usage` reports fixed setup-time limits shared across listeners and depth modes.
pub trait WorldDrawContext {
    /// Submit unlit ARGB world lines atomically. Invalid coordinates or widths,
    /// or a batch exceeding the view's fixed capacity, leave the batch unchanged.
    fn lines(&mut self, lines: &[WorldLine]) -> Result<(), DrawError> {
        unimplemented!("WorldDrawContext::world_draw_lines")
    }
    /// Submit unlit, alpha-blended world quads atomically, using the white atlas texel.
    fn quads(&mut self, quads: &[WorldQuad]) -> Result<(), DrawError> {
        unimplemented!("WorldDrawContext::world_draw_quads")
    }
    /// Queued counts and fixed capacities, shared by both depth modes and all listeners.
    /// Capacity is allocated during setup, never grown during submission.
    fn usage(&self) -> WorldDrawUsage {
        unimplemented!("WorldDrawContext::world_draw_usage")
    }
    /// Draw every copy in `instances` of the resolved `model`.
    ///
    /// One batch per model. The engine uploads only the instance records on the opaque
    /// and cutout route, and bakes the same prepared geometry on the CPU when a shader
    /// pack, a copy with an [`Overlay`], or another route cannot take instances. A model
    /// with a translucent face is refused with [`InstanceError::Translucent`] and is never
    /// drawn as solid. Tint is `0xAARRGGBB` and light is [`LightCoords`]: both stay in
    /// vanilla's space, and the engine converts them to the frame. A tint's alpha
    /// multiplies only faces that have a tint index, the same as vanilla's model
    /// renderer. The view budget is 16,384 copies. Pass at most that many: a longer
    /// list is refused with [`InstanceError::CapacityExceeded`] before any copy is
    /// examined. On wasm the guest pays for the list it passes, because the runtime
    /// lifts the slice before that check. Within the bound, copies outside the frustum
    /// are omitted and do not count. On `Err`, nothing from this call is drawn.
    fn draw_model_instances(
        &mut self,
        model: ModelRef,
        instances: &[ModelInstance],
    ) -> Result<(), InstanceError> {
        let _ = (model, instances);
        unimplemented!("WorldDrawContext::draw_model_instances")
    }
}
