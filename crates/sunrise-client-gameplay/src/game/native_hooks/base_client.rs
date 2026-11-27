use super::*;
use mod_api::BlockEntityDataView;

#[inline(always)]
pub(in crate::game) fn native_client_resources_reloaded(context: &dyn mod_api::ClientReloadHost) {
    crate::game::native_hooks::fire::fire_client_resources_reloaded(context);
}

pub(in crate::game) fn native_emissive(
    context: &mod_api::RegistrationContext,
    layers: &mut Vec<mod_api::Answered<mod_api::EmissiveSpriteRegistration>>,
) {
    crate::game::native_hooks::fire::fire_emissive(context, layers);
}

/// The hook points. Each does nothing in Sunrise.
impl crate::game::ClientGame<'_> {
    /// `ParticleEngine.clearParticles`: every particle and emitter goes before the next
    /// frame is drawn or the next particle tick runs, whichever comes first. It takes
    /// `&self`, so a screen or render hook can ask, as with `play_local_sound`.
    /// False on a server.
    pub(in crate::game) fn clear_particles(&self) -> bool {
        if !self.common.native_window() {
            return false;
        }
        self.client.mod_clear_particles.set(true);
        true
    }
    /// Where the mouse is, in GUI pixels.
    pub fn cursor(&self) -> [f32; 2] {
        [self.client.cursor.0, self.client.cursor.1]
    }
    /// Creative mode only: `stack` into the local player's inventory as `Inventory.add` does,
    /// each changed slot sent at once (`ServerboundSetCreativeModeSlotPacket`); what did not fit.
    pub fn player_give_creative(
        &mut self,
        player: mod_api::PlayerRef,
        stack: &mod_api::ItemStack,
    ) -> Result<Option<mod_api::ItemStack>, mod_api::CreativeGiveError> {
        use mod_api::CreativeGiveError;
        if self.is_logical_server() || self.player.is_none() || self.res.rest.get().is_none() {
            return Err(CreativeGiveError::Unavailable);
        }
        let local = self.local_player().ok_or(CreativeGiveError::Unavailable)?;
        if local != player {
            return Err(CreativeGiveError::Stale);
        }
        self.creative_give_stack(stack)
    }
    /// The `book_id`s of the recipes the player's recipe book holds now
    /// (the server's book on a server); empty outside a world.
    pub(in crate::game) fn known_recipes(&mut self) -> Arc<[i32]> {
        if self.common.player.is_none() {
            return Arc::from([]);
        }
        self.known_displays()
    }
    /// The mod whose own screen is open, if one is (Escape, or another screen
    /// replacing it, closes it).
    pub fn mod_screen(&self) -> Option<&str> {
        self.client
            .native_mod_screen
            .as_deref()
            .filter(|_| self.client.screen == Screen::Mod)
    }
    /// Closes mod `mod_id`'s own screen, if it is open.
    pub fn mod_screen_close(&mut self, mod_id: &str) {
        if self.mod_screen().is_some_and(|id| id == mod_id) {
            self.close_mod_screen();
        }
    }
    /// Opens a screen of mod `mod_id`'s own through the normal screen-transition
    /// path (`Gui.setScreen`): `screen_opening` and `screen_closing` fire for the
    /// transition and `screen_init_post` runs once opened, so the mod mounts
    /// widgets in `screen_init_post`, draws in `native_screen_render_post`, and
    /// takes its clicks, scroll, keys, and typing there; the mouse is freed, and
    /// Escape closes it. Every mod's own screen has the `ScreenView.id` `mod`, so
    /// a mod's screen hooks check `mod_screen()` is its own before mounting or
    /// drawing. It doesn't pause a single-player game (as the inventory doesn't).
    /// Can replace any open vanilla screen, title screen, overlay (book, sign
    /// editor, dialog, or prompt), or another mod's screen, running the replaced
    /// screen's close cleanup once hooks allow the transition. Returns `true`
    /// when `mod_id`'s screen is open, and `false` when cancelled or redirected
    /// by `screen_opening`, while a world or connection is loading, or without a
    /// client window (a headless/dedicated server, the server thread, or
    /// guest-acting contexts). A gamepad can close it (B, Start) but not click on
    /// it: its clicks don't reach the mouse hooks.
    pub fn mod_screen_open(&mut self, mod_id: &str) -> bool {
        let unavailable = self.client.renderer.0.is_none()
            || (self.common.player.is_none() && self.client.title.0.is_none())
            || self.remote_loading();
        let owner = if self.screen_over_playing() {
            None
        } else {
            self.mod_screen()
        };
        if let Some(open) = mod_screen_opens(self.client.screen, unavailable, owner, mod_id) {
            return open;
        }
        if self.screen_hooks_heard() {
            let decision = {
                let current = if self.screen_over_playing() {
                    self.observed_screen_view()
                        .unwrap_or_else(|| self.screen_change_view(self.client.screen))
                } else if self.client.screen == Screen::Playing
                    && let Some(title) = self.client.title.0.as_ref()
                {
                    let none = self.screen_change_view(Screen::Playing);
                    screen_view_with_id(&none, title.mode.id())
                } else {
                    self.screen_change_view(self.client.screen)
                };
                let new = screen_view_for_mod(&self.screen_change_view(Screen::Mod), mod_id);
                self.decide_screen_for_mod_open(&current, &new, current.id != "playing")
            };
            match decision {
                ScreenDecision::Keep => return false,
                ScreenDecision::Vanilla(target) => {
                    self.close_replaced_screen_for_mod();
                    self.open_screen_target(target);
                    return false;
                }
                ScreenDecision::ModScreen(other_mod) => {
                    self.close_replaced_screen_for_mod();
                    self.apply_screen(Screen::Mod, Some(other_mod));
                    return self.mod_screen() == Some(mod_id);
                }
                ScreenDecision::Open => {}
            }
        }
        self.close_replaced_screen_for_mod();
        self.apply_screen(Screen::Mod, Some(mod_id));
        self.mod_screen() == Some(mod_id)
    }
    /// Change the colours of `biome` (`minecraft:plains`) on this client: the
    /// grass, foliage (leaves, vines), dry foliage (leaf litter) and water
    /// tints, 0xRRGGBB, as its biome file and the colormaps set them. Asked
    /// for every biome when a mod calls `refresh_biome_colors`, which
    /// rebuilds the terrain with them; a season mod answers from its season
    /// and calls that when the season turns. The dark forest's modifier
    /// still applies to the grass set here; the swamp's noise colours
    /// replace it.
    pub(in crate::game) fn native_biome_colors(&self, biome: &str, colors: &mut BiomeColors) {
        self.fire_biome_colors(biome, colors);
    }
    /// Where the mod block entity at `pos` draws, for culling
    /// (NeoForge's `getRenderBoundingBox`): `bounds` is min x, y, z then max
    /// x, y, z, in blocks from the block's lower north-west corner. It starts
    /// as the block with a block of margin around it; widen it when your
    /// `native_block_entity_submit` draws further (a tall shaft, a beam), or
    /// set an infinite value for one never culled. Asked once a frame for each
    /// within its view distance (`block_entity_view_distance`) whose chunk is drawn.
    pub(in crate::game) fn native_block_entity_render_bounding_box(
        &self,
        pos: [i32; 3],
        block: &str,
        state: BlockStateId,
        entity: BlockEntityDataView<'_>,
        bounds: &mut [f32; 6],
    ) {
        self.fire_block_entity_render_bounding_box(pos, block, state, entity, bounds);
    }
    /// Draw the mod block entity at `pos`, of block `block`
    /// (`mymod:machine`), on this client: once a frame while it is in view
    /// (vanilla's `BlockEntityRenderer.submit`: within its view distance, 64 blocks unless
    /// `block_entity_view_distance` says otherwise, its chunk
    /// faded in, and its bounds, `native_block_entity_render_bounding_box`,
    /// in the frustum), when its `id` is yours. `state` is the block's state
    /// (`blockEntity.getBlockState()`: read its `facing` with
    /// `block_state_property`), `entity` holds what the server sent
    /// (`native_block_entity_update_tag`). Add geometry with
    /// `native_draw_quad`, `native_draw_cube`, `native_draw_block` and
    /// `native_draw_item` under `draw`'s pose stack, which starts at the
    /// block's lower north-west corner, in blocks; `draw` also has the partial
    /// tick, the game time and the block's light.
    pub(in crate::game) fn native_block_entity_submit(
        &self,
        pos: [i32; 3],
        block: &str,
        state: BlockStateId,
        entity: BlockEntityDataView<'_>,
        draw: &mut BlockEntityDraw,
    ) {
        self.fire_block_entity_submit(pos, block, state, entity, draw);
    }
    /// Before an eligible targeted block's selection outline is built.
    /// Cancel skips only the default block highlight, after normal HUD and game-mode gates.
    pub(in crate::game) fn native_block_highlight(
        &self,
        pos: [i32; 3],
        state: BlockStateId,
        partial: f32,
        canceled: &mut bool,
    ) {
        self.fire_block_highlight(pos, state, partial, canceled);
    }
    /// Final scaled third-person camera distance, before visual-shape collision clipping.
    /// `entity` includes its living scale/distance and optional mount scale/distance.
    /// Only finite requests in 0..=1024 blocks replace the vanilla distance.
    pub(crate) fn native_camera_distance(
        &self,
        partial: f32,
        mirrored: bool,
        entity: &mod_api::client::camera::CameraEntity,
        distance: &mut f32,
    ) {
        self.fire_camera_distance(partial, mirrored, entity, distance);
    }
    /// The client made a new level for `dimension` (NeoForge's
    /// `LevelEvent.Load` from the `ClientLevel` constructor): on joining a
    /// server, single player's own included, and on changing dimension.
    /// Its chunks come after. When the dimension changes, the old level's
    /// `native_client_level_unload` follows this, as `Minecraft.setLevel`
    /// tells it after the new level is made. Client-side, on the window; the
    /// server's levels have `native_level_load`.
    pub(crate) fn native_client_level_load(&mut self, dimension: &str) {
        self.fire_client_level_load(dimension);
    }
    /// The client dropped its level for `dimension` (`LevelEvent.Unload`
    /// from `Minecraft.setLevel` and `disconnect`): replaced by another
    /// dimension's, or on leaving the server. Once for each
    /// `native_client_level_load`. Not told when the game itself quits.
    /// Client-side.
    pub(crate) fn native_client_level_unload(&mut self, dimension: &str) {
        self.fire_client_level_unload(dimension);
    }
    /// Render-thread notification after a resource-pack reload has applied successfully.
    /// Atlas, models, renderer and client caches already refer to the new resources.
    /// Selection/F3+T and server-pack reloads share this point; initial startup does not
    /// (`client_setup` is the once-only moment after the initial load).
    pub fn native_client_resources_reloaded(&mut self) {
        native_client_resources_reloaded(self);
    }
    /// Computed world fog RGB, after ordinary effects and before upload. Title panoramas skip it.
    /// Only finite RGB components in 0..=1 are accepted; alpha and distances stay unchanged.
    pub(crate) fn native_compute_fog_color(
        &self,
        partial: f32,
        camera: [f64; 3],
        fog_type: mod_api::client::fog::FogType,
        rgb: &mut [f32; 3],
    ) {
        self.fire_compute_fog_color(partial, camera, fog_type, rgb);
    }
    /// World field of view, after normal calculation and zoom and before projection/Nausea.
    /// Called for world rendering and near-plane overlays; hand/HUD FOV uses its own path.
    /// Only finite values in 1..=179 degrees replace the computed FOV.
    pub(crate) fn native_compute_fov(&self, partial: f32, camera: [f64; 3], degrees: &mut f32) {
        self.fire_compute_fov(partial, camera, degrees);
    }
    /// A rectangle in GUI pixels, `argb` (0x78000000 is the extras' box).
    pub(in crate::game) fn native_fill(&self, batch: &mut HudBatch, rect: [f32; 4], argb: u32) {
        let [x, y, w, h] = rect;
        let [a, r, g, b] = argb.to_be_bytes();
        batch.fill(&self.client.gui.atlas, x, y, w, h, rgba(r, g, b, a));
    }
    /// The frame rate this window is about to be held to: Max Framerate after the
    /// inactivity throttle and Battery Mode (260 is unlimited). `focused` is whether the
    /// window has focus, `minimized` whether it is minimised and `in_world` whether a world
    /// is open. A mod may lower `limit`, as Dynamic FPS does for a window in the background:
    /// Sunrise keeps the lower of its own limit and the mod's, and ignores values below 1.
    pub(crate) fn native_framerate_limit(
        &self,
        focused: bool,
        minimized: bool,
        in_world: bool,
        limit: &mut i32,
    ) {
        self.fire_framerate_limit(focused, minimized, in_world, limit);
    }
    /// After the entire installed HUD slot body (`RenderGuiLayerEvent.Post`), once
    /// with the ID from `native_gui_layer_pre`. Does not fire when Pre canceled the slot.
    /// Paint into `draw` to put something above the layer.
    pub(crate) fn native_gui_layer_post(&self, layer: &str, draw: &mut HudBatch) {
        self.fire_gui_layer_post(layer, draw);
    }
    /// Before a vanilla or registered HUD slot runs (`RenderGuiLayerEvent.Pre`), even under F1.
    /// Paint into `draw` below the layer, or set `canceled` to skip the entire installed body
    /// (including any side effects) and `native_gui_layer_post`. A replacement or wrapper
    /// keeps the slot's ID and order, but does not inherit its visibility rules; those stay
    /// in the original body. Focused chat paints separately in its screen without dispatching the HUD slot again.
    pub(crate) fn native_gui_layer_pre(
        &self,
        layer: &str,
        draw: &mut HudBatch,
        canceled: &mut bool,
    ) {
        self.fire_gui_layer_pre(layer, draw, canceled);
    }
    /// Item `item`'s icon (`mymod:machine`) with its top left at `at`,
    /// `size` GUI pixels square (16 in a slot); false for an unknown item.
    pub(in crate::game) fn native_item(
        &self,
        batch: &mut HudBatch,
        item: &str,
        at: [f32; 2],
        size: f32,
    ) -> bool {
        if self.common.res.items().get(item).is_none() {
            return false;
        }
        let start = batch.vertices.len();
        self.draw_item_model(batch, &ItemStack::new(item, 1), at[0], at[1], 0);
        batch.scale_since(start, at[0], at[1], size / 16.0, size / 16.0);
        true
    }
    /// The small bar `GuiGraphicsExtractor.itemDecorations` draws under an
    /// item, wherever the game shows one: `bar` arrives holding vanilla's value
    /// (the damage bar, or a bundle's fullness already applied) and may be
    /// replaced (width `0..=13`, colour `rgb`) or set to `None` to hide it.
    /// Asked each frame a stack is drawn. Client-side.
    pub fn native_item_bar(&self, stack: &ItemStack, bar: &mut Option<(i32, u32)>) {
        self.fire_item_bar(stack, bar);
    }
    /// Append an item's custom decorations after its default bar, cooldown and count.
    /// Coordinates are the icon's top-left in GUI units, as the batch takes them;
    /// no item translation is active.
    /// Runs wherever the shared item-decoration path is used (hotbar, menus, tooltips,
    /// and the other screens that draw item icons: advancements, beacon, stats, trading).
    pub(in crate::game) fn native_item_decorations(
        &self,
        stack: &ItemStack,
        x: f32,
        y: f32,
        draw: &mut dyn mod_api::GuiCanvas,
    ) {
        self.fire_item_decorations(stack, x, y, draw);
    }
    /// Item `item`'s icon drawn with model `model` (`minecraft:paper` as
    /// `mymod:wheel_icon`, an `item_model` of a content mod's), as `native_item`
    /// draws one; false for an unknown item.
    pub(in crate::game) fn native_item_model(
        &self,
        batch: &mut HudBatch,
        item: &str,
        model: &str,
        at: [f32; 2],
        size: f32,
    ) -> bool {
        if self.common.res.items().get(item).is_none() {
            return false;
        }
        let mut stack = ItemStack::new(item, 1);
        let mut data = sunrise_items::item_data::StackData::default();
        data.set_other(
            "minecraft:item_model",
            Some(sunrise_core::nbt::Tag::String(model.into())),
        );
        stack.extra = Some(Box::new(data));
        let start = batch.vertices.len();
        self.draw_item_model(batch, &stack, at[0], at[1], 0);
        batch.scale_since(start, at[0], at[1], size / 16.0, size / 16.0);
        true
    }
    /// Draw a stack and its slot decorations, except the count, at `at`,
    /// `size` GUI pixels square. False when nothing was drawn: an unknown item
    /// or an empty stack (air, or a count of 0, as `ItemStack.isEmpty`).
    pub(in crate::game) fn native_item_stack(
        &self,
        batch: &mut HudBatch,
        stack: &ItemStack,
        at: [f32; 2],
        size: f32,
    ) -> bool {
        // GuiGraphicsExtractor.item and itemDecorations draw nothing for an
        // empty stack.
        if &*stack.item == "minecraft:air" || stack.count == 0 {
            return false;
        }
        if self.common.res.items().get(&stack.item).is_none() {
            return false;
        }
        let start = batch.vertices.len();
        self.draw_item_model(batch, stack, at[0], at[1], 0);
        self.draw_item_decorations(batch, stack, at[0], at[1], Some(""));
        batch.scale_since(start, at[0], at[1], size / 16.0, size / 16.0);
        true
    }
    /// Icons a mod puts on map `map` each time it is drawn: held, in an
    /// item frame or on the cartography table's screen (a map's pixels are
    /// its own; nothing is saved in the map). Push `mod_api::MapDecoration`s
    /// (an atlas sprite, a position in half map pixels, a rotation, and
    /// whether it shows in a frame and on the screen like vanilla's markers
    /// do); they are drawn after the vanilla ones with the same pose, and a
    /// sprite the atlas lacks is left out. `map.center`, `map.scale` and
    /// `map.dimension` say which blocks the map shows, and
    /// `mod_api::MapDecoration::x` has the formula. Vanilla's decorations are
    /// not passed in. Asked every frame a map is drawn, so keep it cheap.
    /// The client half of `MapDecorationType` and `addDecoration`: a type
    /// registered on the server and saved in the map's data is not here.
    pub(crate) fn native_map_decorations(
        &self,
        map: &mod_api::MapInfo<'_>,
        decorations: &mut Vec<mod_api::MapDecoration>,
    ) {
        self.fire_map_decorations(map, decorations);
    }
    /// An accepted raw button event finished dispatch, including a screen handler's early return.
    pub(crate) fn native_mouse_button_post(&mut self, button: i32, pressed: bool, modifiers: u32) {
        self.fire_mouse_button_post(button, pressed, modifiers);
    }
    /// Raw mouse button input on any screen, before dispatch; cancel skips dispatch and Post.
    /// Button/modifiers use the same GLFW encoding as screen hooks. Synthetic clicks skip it.
    pub(crate) fn native_mouse_button_pre(
        &mut self,
        button: i32,
        pressed: bool,
        modifiers: u32,
        canceled: &mut bool,
    ) {
        self.fire_mouse_button_pre(button, pressed, modifiers, canceled);
    }
    /// Gameplay scrolling after accumulation, before hotbar/spectator handling.
    /// `scroll` is scaled x/y, `accumulated` whole x/y steps, `buttons` left/middle/right,
    /// and `cursor` the GUI position. Cancel skips gameplay scrolling.
    pub(crate) fn native_mouse_scrolling(
        &mut self,
        scroll: [f64; 2],
        accumulated: [i32; 2],
        buttons: [bool; 3],
        cursor: [f32; 2],
        modifiers: u32,
        canceled: &mut bool,
    ) {
        self.fire_mouse_scrolling(scroll, accumulated, buttons, cursor, modifiers, canceled);
    }
    /// Choose this client's music, once a client tick: `music` and `volume`
    /// hold what vanilla chose (`Minecraft.getSituationalMusic` and
    /// `getMusicVolume`: the biome's track, the creative or underwater one, a
    /// boss's, the menu's; None for silence). Set `music` to a track of your
    /// own, a sound event of your content mod's `sounds.json` such as
    /// `Music::new("mymod:music.spring", 1200, 6000, false)` (the delays in
    /// ticks between songs, and whether it cuts off a different song playing),
    /// None to keep vanilla's quiet, or change `volume` (0 to 1) to fade it.
    pub(crate) fn native_music(&self, music: &mut Option<Music>, volume: &mut f32) {
        self.fire_music(music, volume);
    }
    /// The tracks to play over (or instead of) the music on this client,
    /// asked once a client tick: push a `MusicLayer` (a sound event, looped,
    /// and its volume, 0 to 1) for each. A layer eases to a new volume over up
    /// to two seconds, and one left out fades out and stops; layers that
    /// first appear in the same tick start together, so list every stem of a
    /// piece from the start, the silent ones at volume 0, and raise them (a
    /// fuller arrangement as a village grows). Their sound events should be
    /// `stream: true` in `sounds.json`.
    pub(crate) fn native_music_layers(&self, layers: &mut Vec<MusicLayer>) {
        self.fire_music_layers(layers);
    }
    /// The player did `event` on mod `mod_id`'s panel (`panel_items`):
    /// act on it when it is yours.
    pub(crate) fn native_panel_event(&mut self, mod_id: &str, event: &PanelEvent) {
        self.fire_panel_event(mod_id, event);
    }
    /// A quad of corners `points` (in order round it) in GUI pixels, `argb`:
    /// a wedge, a slanted bar, what `native_fill`'s rectangles can't draw.
    pub(in crate::game) fn native_quad(
        &self,
        batch: &mut HudBatch,
        points: [[f32; 2]; 4],
        argb: u32,
    ) {
        let [a, r, g, b] = argb.to_be_bytes();
        batch.fill_quad(&self.client.gui.atlas, points, rgba(r, g, b, a));
    }
    /// The server's recipes of `recipe_types` arrived (NeoForge's
    /// `RecipesReceivedEvent`): from now on the recipe helpers
    /// (`native_recipes`, `recipes_of_type`) give the server's recipes
    /// of those types in place of the window's own, so rebuild what you made
    /// of them. It comes after a join, once the window has its player, and
    /// after every `/reload`, on a server whose mods asked for them
    /// (`native_datapack_sync`), single player included; with no types when
    /// the server stopped sending them and the window is back on its own
    /// recipes. A recipe the
    /// window's own recipe book doesn't know has `book_id` -1. Client-side.
    pub(in crate::game) fn native_recipes_received(&mut self, recipe_types: &[String]) {
        self.fire_recipes_received(recipe_types);
    }
    /// The client's own commands (`RegisterClientCommandsEvent`), which
    /// `native_command` runs: push a `mod_api::Command` per usage. Asked when a
    /// connection starts and again when the server sends its own tree, so the
    /// chat box colours, completes and lists them as the game's own; a name
    /// the game already has keeps that command's behaviour. A client command
    /// has no operator level: one with a nonzero level is left out.
    pub(in crate::game) fn native_register_client_commands(
        &mut self,
        commands: &mut Vec<mod_api::Answered<mod_api::ClientCommandRegistration>>,
    ) {
        self.fire_register_client_commands(commands);
    }
    /// Final world fog, after the computed color hook, ordinary calculations and Sunrise's fog
    /// option, before upload. Title panoramas skip it. Invalid color or distance values retain the
    /// original whole fog.
    pub(crate) fn native_render_fog(
        &self,
        partial: f32,
        camera: [f64; 3],
        fog_type: mod_api::client::fog::FogType,
        fog: &mut mod_api::client::fog::FogData,
    ) {
        self.fire_render_fog(partial, camera, fog_type, fog);
    }
    /// A character was typed at the open screen
    /// (`ScreenEvent.CharacterTyped.Pre`, vanilla's `Screen.charTyped`):
    /// `screen` is the screen it is, `c` the character, `modifiers` GLFW's
    /// bits (shift 1, control 2, alt 4, super 8), and `field` a copy of the
    /// focused text box. Set `canceled` and that one character never reaches
    /// a text box.
    ///
    /// `field` is `None` with no box focused, and for the boxes that keep
    /// their own validators: Options' and World Options' fields, the game
    /// rules' and the block editor's. `cursor` and `highlight` count Unicode
    /// scalar characters, and a character sees the box as the one before it
    /// left it. Change `value`, `cursor` or `highlight` and the box takes the
    /// change, cancelled or not, so cancel to replace the character yourself.
    /// New text goes through the box's own filter and length limit, and what
    /// follows the box (chat suggestions, a search's results, the anvil's
    /// name) follows it. `max_length`, `multiline` and whether `field` is
    /// `None` stay the box's: a change to them does nothing.
    ///
    /// It fires once per printable character of a key press's text and once
    /// per character of the input method's committed text, never for a control
    /// character (Enter, Backspace, Tab, Delete, Escape, a Control letter):
    /// those are keys, and the game reads them as keys, not as text. A key's
    /// characters fire after the key has acted, as GLFW's char callback comes
    /// after its key callback (`KeyboardHandler.charTyped`): `screen` is the
    /// screen open then, so a key that closes the screen (the inventory key)
    /// leaves none to hear its character, where vanilla's closed screen hears
    /// nothing, and one that moves the focus types into the box it moved to.
    /// They fire even when `native_screen_key_pressed_pre` cancelled the press
    /// that carried the text, as NeoForge delivers `CharacterTyped.Pre` for a
    /// cancelled `KeyPressed.Pre`: the characters left reach the text box
    /// without the key's own effects. Cancelling one character drops only that
    /// one. The control socket's `type` fires it too, but not for its `\b`
    /// backspace. Same screens as the key press, and it fires with no text box
    /// focused (there is nothing to take the character then).
    pub(in crate::game) fn native_screen_char_typed_pre(
        &self,
        screen: &ScreenView<'_>,
        c: char,
        modifiers: u32,
        field: &mut Option<mod_api::client::widgets::TextEdit>,
        canceled: &mut bool,
    ) {
        self.fire_screen_char_typed_pre(screen, c, modifiers, field, canceled);
    }
    /// A key went down over the open screen (`ScreenEvent.KeyPressed.Pre`,
    /// vanilla's `Screen.keyPressed`): `screen` is the screen it is, `key`
    /// vanilla's name for the key (`key.keyboard.e`) — a key with no name
    /// fires nothing — `repeat` whether it is
    /// the keyboard's auto-repeat of a held press (Sunrise's extra; NeoForge
    /// folds repeats into the event), and `modifiers` GLFW's bits (shift 1,
    /// control 2, alt 4, super 8). Set `canceled` and the press goes no
    /// further: no focus move, no Escape-close, no inventory or hotbar key, no
    /// typing by the key itself.
    ///
    /// It fires for the window's presses and their repeats, and for the
    /// control socket's `key` (a release fires nothing), after the window's
    /// own shortcuts have run — the fullscreen and screenshot keys, the debug
    /// keys and the narrator — and while a screen of the game's or the title's
    /// is open (Options and the pack list opened from the title screen count,
    /// with no player and no `menu`; the title's own). It does
    /// not fire on the connect and loading screens, nor for a server dialog, the server pack,
    /// link and code-of-conduct prompts, a book or a sign editor drawn over
    /// gameplay, nor while a world loads or saves: the key there stays the
    /// game's own. A cancelled press's release still runs, and its characters
    /// are GLFW's separate char callback and still come; see
    /// `native_screen_char_typed_pre`.
    pub fn native_screen_key_pressed_pre(
        &self,
        screen: &ScreenView<'_>,
        key: &str,
        repeat: bool,
        modifiers: u32,
        canceled: &mut bool,
    ) {
        self.fire_screen_key_pressed_pre(screen, key, repeat, modifiers, canceled);
    }
    /// Before an open screen handles a named key's release (same screen gate as key presses).
    /// Cancel bypasses screen handling; physical keys and mapped keyboard hold/toggle state release.
    pub(in crate::game) fn native_screen_key_released_pre(
        &self,
        screen: &ScreenView<'_>,
        key: &str,
        modifiers: u32,
        canceled: &mut bool,
    ) {
        self.fire_screen_key_released_pre(screen, key, modifiers, canceled);
    }
    /// A mouse button went down over the open screen
    /// (`ScreenEvent.MouseButtonPressed.Pre`, vanilla's
    /// `Screen.mouseClicked`): `screen` is the screen it is, `screen.mouse`
    /// where the pointer is (so there is no position of its own), `button` the
    /// button's GLFW index (0 left, 1 right, 2 middle, 3 back, 4 forward; a
    /// button past those is the platform's own number, which differs between
    /// systems), and `modifiers` GLFW's bits (shift 1, control 2,
    /// alt 4, super 8). `double_click` comes from the last handled press on
    /// the same screen instance and button. Cancel to skip the screen and
    /// Post; the press still counts as handled for double-click detection.
    ///
    /// It fires before anything the screen does with the press, on the window
    /// only, and while a screen of the game's or the title's is open (Options
    /// and the pack list opened from the title screen count, with no player
    /// and no `menu`; the title's own screens, their
    /// `screen.id` from the title state machine). It does not fire on the
    /// connect and loading screens, nor for a server
    /// dialog, the server pack, link and code-of-conduct prompts, a book or a
    /// sign editor drawn over gameplay, nor while a world loads or saves: the
    /// click there stays the game's own. A gamepad click or a click the
    /// keyboard's focus sends is not the mouse and fires nothing.
    pub(in crate::game) fn native_screen_mouse_button_pressed_pre(
        &self,
        screen: &ScreenView<'_>,
        button: i32,
        modifiers: u32,
        double_click: bool,
        canceled: &mut bool,
    ) {
        self.fire_screen_mouse_button_pressed_pre(
            screen,
            button,
            modifiers,
            double_click,
            canceled,
        );
    }
    /// A mouse button went up over the open screen
    /// (`ScreenEvent.MouseButtonReleased.Pre`, vanilla's
    /// `Screen.mouseReleased`), with the same `screen`, `button` and
    /// `modifiers` as `native_screen_mouse_button_pressed_pre` and fired at
    /// the same point. Set `canceled` and the release goes no further: a
    /// pressed slot keeps what it took and no drag, slider or scrollbar lets
    /// go.
    ///
    /// Cancel a release only when the press it ends was cancelled too: a
    /// release the screen took ends the drag or the scroll it began, as it
    /// does in vanilla, and no guard here un-cancels it. Same screens as the
    /// press; a press that opened another screen ends on that one.
    pub(in crate::game) fn native_screen_mouse_button_released_pre(
        &self,
        screen: &ScreenView<'_>,
        button: i32,
        modifiers: u32,
        canceled: &mut bool,
    ) {
        self.fire_screen_mouse_button_released_pre(screen, button, modifiers, canceled);
    }

    /// The pointer moved over the open screen (`Screen.mouseMoved`): before the drag hook
    /// of the same move. Cancel keeps the new cursor position and skips the rest of the move.
    pub(in crate::game) fn native_screen_mouse_moved_pre(
        &self,
        screen: &ScreenView<'_>,
        delta: [f32; 2],
        modifiers: u32,
        canceled: &mut bool,
    ) {
        self.fire_screen_mouse_moved_pre(screen, delta, modifiers, canceled);
    }

    /// Files dropped on the window while a screen is open (`Screen.onFilesDrop`, called by
    /// `MouseHandler.onDrop` with the whole drop): `paths` are the operating system's, in its
    /// order. Observe only.
    pub(in crate::game) fn native_screen_files_dropped(
        &self,
        screen: &ScreenView<'_>,
        paths: &[std::path::PathBuf],
    ) {
        self.fire_screen_files_dropped(screen, paths);
    }
    /// Raw device motion while the open mod screen holds the pointer grab.
    pub(in crate::game) fn native_screen_pointer_motion(
        &self,
        screen: &ScreenView<'_>,
        delta: [f64; 2],
    ) {
        self.fire_screen_pointer_motion(screen, delta);
    }
    /// Before a real held-button movement reaches screen drag handlers.
    /// `screen.mouse` is the current GUI position; `delta` is its GUI-space movement.
    /// Cancel keeps the new cursor position but skips all screen drag handlers.
    pub(in crate::game) fn native_screen_mouse_dragged_pre(
        &self,
        screen: &ScreenView<'_>,
        button: i32,
        delta: [f32; 2],
        modifiers: u32,
        canceled: &mut bool,
    ) {
        self.fire_screen_mouse_dragged_pre(screen, button, delta, modifiers, canceled);
    }
    /// The wheel turned over the open screen (`ScreenEvent.MouseScrolled.Pre`,
    /// vanilla's `Screen.mouseScrolled`): `screen` is the screen it is,
    /// `screen.mouse` where the pointer is (so there is no position of its
    /// own), `scroll` the wheel's movement in x and y as the game's own scroll
    /// handler scales it (a line times the wheel sensitivity: with Discrete
    /// Scrolling on, a notch is ±1, its sign only; a touchpad's pixels are
    /// divided by 30), and
    /// `modifiers` GLFW's bits (shift 1, control 2, alt 4, super 8). Set
    /// `canceled` and nothing the screen would scroll moves: not the bundle
    /// under the pointer, nor a list, slider, scrollbar or map. Nothing else
    /// reads the wheel while a screen of the game's is open, so no hotbar slot
    /// changes either.
    ///
    /// It fires before the screen scrolls anything, for the window's wheel
    /// (sunrise-ctl's `scroll-at` sends it through the window too), and
    /// while a screen of the game's or the title's is open (Options and the
    /// pack list opened from the title screen count, with no player and no
    /// `menu`; the title's own). It
    /// does not fire on the connect and loading
    /// screens, nor for a server dialog, the server pack, link and
    /// code-of-conduct prompts, a book or a sign editor drawn over gameplay,
    /// nor while a world loads or saves: the scroll there stays the game's
    /// own. A wheel a gamepad or the keyboard's focus sends is not the mouse
    /// and fires nothing.
    pub(in crate::game) fn native_screen_mouse_scrolled_pre(
        &self,
        screen: &ScreenView<'_>,
        scroll: [f64; 2],
        modifiers: u32,
        canceled: &mut bool,
    ) {
        self.fire_screen_mouse_scrolled_pre(screen, scroll, modifiers, canceled);
    }
    /// The open screen has drawn itself and its tooltips (`Screen.render`,
    /// the point of `ScreenEvent.Render.Post`): draw over it with
    /// `native_fill`, `native_sprite` and `native_text`, `screen.rect` being
    /// its container panel (or the whole window) and `screen.id` the screen
    /// it is. Once a frame on the window, never while playing with no screen;
    /// `&self`, so it reads and draws only. The Win and Options screens call
    /// it on their own path, after their drawing.
    ///
    /// It does not fire for the screens drawn over gameplay after it (a
    /// server dialog, the server pack, link and code-of-conduct prompts, a
    /// book, a sign editor): those keep `Screen::Playing`, which it skips (the
    /// server pack prompt hides an open screen too, and that screen does not
    /// fire under it), and draw over what it drew, while vanilla's
    /// `ScreenEvent.Render.Post` fires for them. The title's own screens
    /// (`title`, `world_select`, `world_create`, `multiplayer`…) fire it too,
    /// their `screen.id` from the title state machine and the whole window
    /// their `rect`, and no dialog or prompt over them.
    pub fn native_screen_render_post(&self, draw: &mut HudBatch, screen: &ScreenView<'_>) {
        self.fire_screen_render_post(draw, screen);
    }
    /// The GUI's size in GUI pixels, width then height.
    pub(in crate::game) fn native_size(&self, draw: &HudBatch) -> [f32; 2] {
        [draw.width, draw.height]
    }
    /// Part `src` (`[x, y, w, h]` in the sprite's own pixels) of sprite
    /// `name` (vanilla's `gui/...`, a content mod's GUI texture `mymod:gui/...`,
    /// or a client mod's dynamic texture bound to an atlas slot,
    /// `create_dynamic_texture`), drawn stretched into `dst` (`[x, y, w, h]` in GUI
    /// pixels), every texel multiplied by `tint` (`0xAARRGGBB`, a fluid's colour)
    /// when there is one. False when there is no such sprite.
    pub fn native_sprite(
        &self,
        batch: &mut HudBatch,
        name: &str,
        src: [f32; 4],
        dst: [f32; 4],
        tint: Option<u32>,
    ) -> bool {
        let Some(mut r) = self.client.gui.atlas.gui_rect(name).copied() else {
            return false;
        };
        let (du, dv) = ((r.u1 - r.u0) / r.w as f32, (r.v1 - r.v0) / r.h as f32);
        r.u0 += src[0] * du;
        r.v0 += src[1] * dv;
        r.u1 = r.u0 + src[2] * du;
        r.v1 = r.v0 + src[3] * dv;
        let [a, red, green, blue] = tint.unwrap_or(u32::MAX).to_be_bytes();
        let start = batch.vertices.len();
        batch.rect_uv(dst[0], dst[1], dst[2], dst[3], r, rgba(red, green, blue, a));
        batch.bind_gui_atlas(start, &self.client.gui.atlas, name);
        true
    }
    /// A line of text at `(x, y)` in GUI pixels, in `argb` as vanilla's `GuiGraphics` takes it
    /// (`0xFFFFFFFF` is opaque white). Fully transparent text draws nothing, as vanilla's.
    pub(in crate::game) fn native_text(
        &self,
        batch: &mut HudBatch,
        text: &str,
        at: [f32; 2],
        argb: u32,
        shadow: bool,
    ) {
        let [a, r, g, b] = argb.to_be_bytes();
        if a == 0 {
            return;
        }
        let [x, y] = at;
        self.client.gui.plain_texts.with(text, |text| {
            batch.text_styled(&self.client.gui.atlas, text, x, y, rgba(r, g, b, a), shadow)
        });
    }
    /// `native_text` scaled by `scale` about `at`, its top-left corner (a GUI pose's scale).
    pub(in crate::game) fn native_text_scaled(
        &self,
        batch: &mut HudBatch,
        text: &str,
        at: [f32; 2],
        scale: f32,
        argb: u32,
        shadow: bool,
    ) {
        let start = batch.vertices.len();
        self.native_text(batch, text, at, argb, shadow);
        batch.scale_since(start, at[0], at[1], scale, scale);
    }
    /// `Item.appendHoverText`: an item's own lines in its tooltip, after its
    /// name and before its components' (enchantments, lore, attributes). Add
    /// lines to `lines`, which arrives empty, each with a colour as `rgba` packs
    /// it (`u32::MAX` keeps the text's own); `Text::legacy` reads `§` codes.
    /// The stack has its components (`stack.data()`). Asked when lines are built for
    /// display or `ItemResource::tooltip`, except during reentrant line queries, when
    /// the stack hides its tooltip or for a crossbow's loaded projectile. `context` shares
    /// the player, flags and vanilla client-level tick rate with component tooltip appenders.
    pub(crate) fn native_tooltip(
        &self,
        stack: &ItemStack,
        context: &mod_api::TooltipAppendContext,
        lines: &mut Vec<(Text, u32)>,
    ) {
        self.fire_tooltip(stack, context, lines);
    }
    /// `Item.getTooltipImage`: optional tooltip component data for this item stack.
    /// Answer `Some(TooltipComponent::custom(data))` (or `Some(TooltipComponent::bundle(stack))`)
    /// to insert a component after the item's name line before
    /// `render_tooltip_gather_components` runs. Answer `None` to leave the vanilla tooltip
    /// component unchanged. Client-side.
    pub(in crate::game) fn native_tooltip_image(
        &self,
        stack: &ItemStack,
        component: &mut Option<mod_api::TooltipComponent>,
    ) {
        self.fire_tooltip_image(stack, component);
    }
    /// The world frame's local visual sources. Push requests; Sunrise validates
    /// and limits them before rendering. Called only while a world is visible.
    pub(crate) fn native_visuals(&self, rims: &mut Vec<BlockRim>, lights: &mut Vec<ColoredLight>) {
        self.fire_visuals(rims, lights);
    }
    /// Shows the panel beside container screens with these items (at most
    /// 8192), as the game's own slots in pages, with a search box; the
    /// player's clicks, searches and keys over items come back as
    /// `native_panel_event`. The panel is the last mod's to show one; empty
    /// items and no recipe page take yours away.
    pub fn panel_items(&mut self, mod_id: &str, items: Vec<mod_api::Stack>) {
        self.client.panel.show_items(mod_id, items);
    }
    /// The particles alive in the client's level (`ParticleEngine.countParticles`): 0 once
    /// a clear is asked, and on a server.
    pub(in crate::game) fn particle_count(&self) -> u32 {
        if !self.common.native_window() || self.client.mod_clear_particles.get() {
            return 0;
        }
        u32::try_from(self.client_particle_count()).unwrap_or(u32::MAX)
    }
    /// Places a recipe in the open screen's grid as a click in the recipe
    /// book does (`ServerboundPlaceRecipePacket`): the inventory's grid or a
    /// crafting table's. `all` places as many as the items allow, as with
    /// Shift. Refused while no such screen is open and for a recipe the
    /// player's book doesn't hold; one missing items shows as a ghost.
    pub(in crate::game) fn place_recipe(&mut self, book_id: i32, all: bool) -> Result<(), String> {
        self.recipe_place(book_id, all)
    }
    /// The open menu's title (the block's name, or the one it was given),
    /// as plain text.
    pub fn screen_title(&self) -> Option<String> {
        self.remote_menu_title().map(|t| t.plain())
    }
    /// Transfer a viewer recipe without learning it.
    pub(in crate::game) fn transfer_recipe(&mut self, book_id: i32) {
        self.transfer_viewer_recipe(book_id);
    }
}
