use crate::game::client_side::key_codes::{glfw_button, glfw_modifiers};
use crate::game::client_side::{context_menu, input_trace};
use crate::game::event_loop::EventLoopCtl;
use crate::game::loading_window::{build_renderer, loading_frame};
use crate::game::motion_ids::motion_block_entity;
use crate::game::subtitles::{Subtitles, in_game_ui, subtitles};
use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant},
};
use sunrise_client::held::Source;
use sunrise_client::{
    block_entity_render::{
        self, CacheOptions, Scene as BlockEntityScene, StaticCacheFrameStats, StaticSectionSnapshot,
    },
    gfx::{Frame, Renderer},
    graphics::EntityCullingMode,
    hud::HudBatch,
    hud_layout, hud_overlays,
    mesher::Vertex,
    title::Action,
};
use sunrise_client_gameplay::{frame_autotune, hud_gates::debug_crosshair_drawn};
use sunrise_client_render::block_entity_reach::Reach;
use sunrise_core::{geometry::Aabb, position::BlockPos};
use sunrise_core::{geometry::Vec3, random::RandomSource};
use sunrise_game_data::Result;
use sunrise_items::blocks::Fluid;
use sunrise_level::spans::startup;
use sunrise_level::{containers::BlockEntity, spans};
use sunrise_world::{level::LevelEvent, registry::BlockStateId};
use winit::{
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    keyboard::{KeyCode, PhysicalKey},
};
use {crate::game::*, sunrise_gameplay_common::game::*};

/// Window key parts used by IME and preset editor dispatch.
pub struct WindowKeyInput<'a> {
    pub physical: PhysicalKey,
    pub state: ElementState,
    pub repeat: bool,
    pub text: Option<&'a str>,
    pub shortcut: Option<sunrise_client::clipboard::EditShortcut>,
}

impl crate::game::ClientGame<'_> {
    /// Client-side water splash particles for a dropped item (`side_apply_item_splash`).
    pub(in crate::game) fn client_apply_item_splash(
        &mut self,
        splash: sunrise_sim::entities::ItemSplash,
    ) {
        use sunrise_sim::entities::ItemSplash;
        if let ItemSplash::Particle {
            bubble,
            at,
            velocity,
        } = splash
        {
            use sunrise_level::particle_options::{ParticleOptions, ParticleType};
            let kind = if bubble {
                ParticleType::Bubble
            } else {
                ParticleType::Splash
            };
            self.client
                .particles
                .0
                .add(&ParticleOptions::simple(kind), at, velocity);
        }
    }
    /// Client-side block entity NBT for adventure predicates (`side_block_entity_nbt`).
    pub(in crate::game) fn client_block_entity_nbt(
        &self,
        pos: [i32; 3],
    ) -> Option<sunrise_core::nbt::Tag> {
        if let Some(tag) = self.common.pot_data(pos) {
            return Some(tag);
        }
        let key = (pos[0], pos[1], pos[2]);
        let entity = self.common.world.block_entities.get(&key)?;
        let block = self.common.data_block_name(pos);
        Some(self.client_encode_live_block_entity(key, entity, &block))
    }
    /// Send a control-socket command through the client chat path (`side_send_chat_command`).
    pub(in crate::game) fn client_send_chat_command(&mut self, command: &str) {
        self.send_signed_chat_input(command);
    }
    /// Clear shared entity lists, client particles, remote mirrors, and render caches.
    pub(in crate::game) fn client_clear_entities(&mut self) {
        self.common.clear_shared_entities();
        self.clear_client_particles();
        self.clear_remote_objects();
        self.clear_remote_projectiles();
        self.client_clear_render_caches();
    }
    /// How far the block entity of a block state is drawn (its `BlockEntityRenderer`'s reach), and
    /// whether entity culling may hide it (the end portal and gateway never are), by state id,
    /// resolved once: every block entity draw and the frame's block entity selection ask.
    pub(in crate::game) fn block_entity_reach_entry(&self, state: u32) -> (Reach, bool) {
        let table = self.client.block_entity_reach.get_or_init(|| {
            self.common
                .res
                .facts()
                .states
                .iter()
                .map(|state| {
                    let unculled = matches!(
                        &*state.name,
                        "minecraft:end_gateway" | "minecraft:end_portal"
                    );
                    (Reach::of(&state.name), unculled)
                })
                .collect()
        });
        // A state the registry lacks has air's facts (`BlockFacts::get`).
        table
            .get(state as usize)
            .copied()
            .unwrap_or((Reach::Default, false))
    }
    /// `Camera.setRotation`: the camera's orientation, turned round in
    /// third-person front view.
    pub(crate) fn camera_orientation(&self) -> glam::Quat {
        use sunrise_level::camera_math::CameraType::FirstPerson;
        match self.client.posed_rotation {
            Some((yaw, pitch, roll)) => camera_rotation_rolled(FirstPerson, yaw, pitch, roll),
            None if self.client.studio.flying() => camera_rotation_rolled(
                FirstPerson,
                self.client.studio.freecam.yaw,
                self.client.studio.freecam.pitch,
                -self.client.studio.freecam.roll,
            ),
            None => camera_rotation(self.client.camera_type, self.common.yaw, self.common.pitch),
        }
    }
    /// `AbstractCandleBlock.extinguish`: the smoke from each wick, at the
    /// block's `getParticleOffsets` (sixteenths of a block), rising 0.1.
    pub(in crate::game) fn candle_smoke(&mut self, pos: [i32; 3], state: BlockStateId) {
        use sunrise_level::particle_options::{ParticleOptions, ParticleType};
        let candles = self
            .common
            .pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("candles")?.parse::<usize>().ok());
        let offsets = sunrise_gameplay_common::game::CommonGame::candle_offsets(
            &self.common.res.facts().get(state).name,
            candles,
        );
        for o in offsets {
            self.client.particles.0.add(
                &ParticleOptions::simple(ParticleType::Smoke),
                [
                    f64::from(pos[0]) + f64::from(o[0]) / 16.0,
                    f64::from(pos[1]) + f64::from(o[1]) / 16.0,
                    f64::from(pos[2]) + f64::from(o[2]) / 16.0,
                ],
                // AbstractCandleBlock.extinguish: ySpeed 0.1F, widened.
                [0.0, f64::from(0.1_f32), 0.0],
            );
        }
    }
    #[inline(always)]
    pub(in crate::game) fn clear_entities_client_render_caches(&mut self) {
        self.client.entity_vertices.clear();
        self.client.see_through_vertices.clear();
        self.client.glowing_vertices.get_mut().clear();
        self.client.shadow_only_vertices.get_mut().clear();
        self.client.entity_spans.borrow_mut().clear();
        self.client.entity_motion.borrow_mut().clear();
        self.client.entity_motion_posed.borrow_mut().clear();
        self.client
            .skin_models
            .0
            .borrow_mut()
            .restart(&mut self.client.skin_frame.borrow_mut());
        self.client.skin_frame.borrow_mut().clear_draws();
    }
    #[inline(always)]
    pub(in crate::game) fn clear_entities_client_transient_blocks(&mut self) {
        self.client.transient_blocks.clear();
    }
    #[inline(always)]
    pub(in crate::game) fn client_clear_render_caches(&mut self) {
        self.clear_entities_client_transient_blocks();
        self.clear_entities_client_render_caches();
    }
    /// Reject obsolete mesh entries even if an old asynchronous upload arrives after a move.
    pub(in crate::game) fn live_block_entity_render(&self, pos: [i32; 3], state: u32) -> bool {
        self.common.world.block(pos[0], pos[1], pos[2]).0 == state
            && self.client_res().block_entity_render().renders(state)
    }
    /// Route a winit key event and its text independently (`KeyboardHandler.charTyped`).
    pub fn dispatch_window_keyboard(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        key: Option<KeyCode>,
        pressed: bool,
        repeat: bool,
        shortcut: Option<sunrise_client::clipboard::EditShortcut>,
        text: Option<&str>,
    ) -> Result<()> {
        if !self.context_menu_key_active() {
            self.dispatch_window_keyboard_as::<false>(
                event_loop, key, pressed, repeat, shortcut, text,
            )
        } else {
            self.dispatch_window_keyboard_as::<true>(
                event_loop, key, pressed, repeat, shortcut, text,
            )
        }
    }

    #[inline(always)]
    fn dispatch_window_keyboard_as<const MENU: bool>(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        key: Option<KeyCode>,
        pressed: bool,
        repeat: bool,
        shortcut: Option<sunrise_client::clipboard::EditShortcut>,
        text: Option<&str>,
    ) -> Result<()> {
        let has_screen = self.observed_screen_view().is_some();
        let menu_blocking = MENU
            && (self.context_menu_open()
                || key.is_some_and(|k| self.context_menu_owns_key(Source::Keyboard, k)));
        if let Some(key) = key
            && !self.dispatch_screen_key_as::<MENU>(
                event_loop,
                key,
                pressed,
                repeat,
                shortcut,
                Source::Keyboard,
            )?
            && !self.key_press_canceled(key, pressed, repeat)
        {
            self.native_panel_key_event(key, pressed, repeat);
            self.mapped_key_event(key, pressed, repeat)?;
        }
        // A key callback can open a menu before GLFW's character event.
        if pressed && has_screen && !menu_blocking && !self.context_menu_open() {
            self.settle_title_mode();
            self.screen_chars_typed(text);
        }
        Ok(())
    }
    /// `FallingBlockRenderer` for the falling blocks of `run`, drawn with their baked model.
    pub(in crate::game) fn falling_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        setup: &FallingSetup<'_>,
        run: std::ops::Range<usize>,
        out: &mut Vec<Vertex>,
    ) {
        let FallingSetup {
            view: falling_view,
            forward: falling_forward,
        } = *setup;
        let tables = &self.client.client_pack.context.mesh.tables;
        for f in &self.common.falling[run] {
            if self.hides_as_camera(f.id) {
                continue;
            }
            let p = f.model_origin(alpha, |state, x, z| tables.offset(state, x, z)) - camera;
            let o = p.as_vec3();
            // Only rear-origin models pay for the envelope/depth check before full frustum culling.
            if falling_forward.dot(o) < 0.0
                && falling_view.is_some_and(|view| {
                    self.client
                        .client_pack
                        .render
                        .model_catalog
                        .render_bounds(f.state)
                        .is_some_and(|[lo, hi]| {
                            let lo = glam::Vec3::from_array(lo) + o;
                            let hi = glam::Vec3::from_array(hi) + o;
                            let magnitude = o
                                .abs()
                                .max_element()
                                .max(lo.abs().max_element())
                                .max(hi.abs().max_element());
                            let padding =
                                glam::Vec3::splat(1.0e-4 + magnitude * f32::EPSILON * 4.0);
                            !view.intersects_depth(lo - padding, hi + padding)
                        })
                })
            {
                continue;
            }
            // `FallingBlockEntity.blockPosition()`: the cell its feet are in.
            let (x, y, z) = (
                f.position.x.floor() as i32,
                f.position.y.floor() as i32,
                f.position.z.floor() as i32,
            );
            if !sunrise_gameplay_common::game::falling_block_visible(
                f.state,
                self.common.world.block(x, y, z),
            ) {
                continue;
            }
            let Some((_, tint)) = self.falling_block_face_sprite(f.state) else {
                continue;
            };
            let light = self.common.world.light(x, y + 1, z);
            let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            let start = out.len();
            self.mark_motion(
                start,
                sunrise_gameplay_common::game::MOTION_OBJECT | u64::from(f.id),
            );
            self.falling_model_vertices(f.state, f.start_pos, o, packed, out);
            let tint = sunrise_client::mobs::render::abgr(tint);
            for vertex in &mut out[start..] {
                vertex.color = sunrise_client::hud_layout::multiply(vertex.color, tint);
            }
        }
    }
    pub fn pointer_projection(
        &self,
        framebuffer: [u32; 2],
    ) -> sunrise_client::window::PointerProjection {
        use sunrise_client::window::{GuiSize, PointerProjection};
        let inputs = self.client.settings.gui_scale_inputs();
        PointerProjection {
            gui: GuiSize::new(framebuffer, inputs.requested, inputs.unicode),
            window: self.pointer_space(framebuffer),
        }
    }
    pub(in crate::game) fn render_frame(&mut self) -> Result<bool> {
        self.render_with_hooks(|renderer, frame, hooks| {
            sunrise_level::spans::time("renderer", || renderer.render(frame, hooks))
        })
    }
    /// Extract every particle group after the frame's entity, screen and hand callbacks.
    /// Item model callbacks can clear during extraction too: roll back only this tail of
    /// geometry (including GPU jobs and outlines), never the frame's ordinary entities.
    pub fn client_particle_geometry(
        &mut self,
        camera: glam::DVec3,
        alpha: f64,
        fov_degrees: f32,
        providers: Option<(
            &sunrise_client::particles::ParticleCullView,
            &mut sunrise_client::particles::ParticleQuads,
        )>,
        out: &mut Vec<Vertex>,
    ) {
        self.apply_mod_particle_clear();
        // The packs' GPU particle records draw here too (a type without a shader the
        // renderer draws), and aren't in the native count.
        if self.client_particle_count() == 0 && self.client.particles.0.polytone_gpu_records() == 0
        {
            return;
        }
        let first = out.len();
        let skin = self.client.skin_frame.borrow().mark();
        let outline_jobs = self.client.skin_frame.borrow().outline_jobs.len();
        let outline_vertices = self.client.skin_frame.borrow().outline_vertices;
        let spans = self.client.entity_spans.borrow().len();
        let motion = self.client.entity_motion.borrow().len();
        let glowing = self.client.glowing_vertices.borrow().len();
        self.mark_motion(out.len(), 0);
        // Particles (ParticleEngine.extract): camera-facing quads.
        self.mark_stage(out.len(), "PARTICLES");
        let world = &self.common.world;
        let particle_mode = self.client.settings.sunrise.particle_culling;
        let mut providers = providers;
        if particle_mode == sunrise_client::graphics::ParticleCullingMode::Off {
            self.client.particles.0.render_world(
                camera,
                self.particle_rotation(),
                alpha as f32,
                providers
                    .as_mut()
                    .map(|(view, quads)| (*view, &mut **quads)),
                None,
                &|b| world.light(b[0], b[1], b[2]),
                out,
            );
        } else {
            let frustum = self.client.view_frustum;
            let pixel_focal_length = self.client.renderer.0.as_ref().map_or(0.0, |renderer| {
                let tangent = (f64::from(fov_degrees).to_radians() * 0.5).tan();
                if tangent.is_finite() && tangent > 0.0 {
                    (f64::from(renderer.config.height.max(1)) / (2.0 * tangent)) as f32
                } else {
                    0.0
                }
            });
            {
                let mut particle_culling = self.particle_occlusion_context(camera);
                self.client.particles.0.render_world(
                    camera,
                    self.particle_rotation(),
                    alpha as f32,
                    providers
                        .as_mut()
                        .map(|(view, quads)| (*view, &mut **quads)),
                    Some(sunrise_client::particles::ParticleWorldCull {
                        pixel_focal_length,
                        frustum: frustum.as_ref(),
                        occluded: &mut |min, max| particle_culling.hidden(min, max),
                    }),
                    &|b| world.light(b[0], b[1], b[2]),
                    out,
                );
            }
            self.finish_async_particle_culling();
        }
        self.mark_entity(out.len(), "minecraft:item");
        self.pickup_item_geometry(camera, alpha, out);
        self.mark_motion(out.len(), 0);
        self.pickup_arrow_geometry(camera, alpha, out);
        self.mark_entity(out.len(), "minecraft:trident");
        self.pickup_trident_geometry(camera, alpha, out);
        self.mark_entity(out.len(), "minecraft:experience_orb");
        // ItemPickupParticle: the render state extracted at partial tick 1.
        let taken = self
            .client
            .pickups
            .0
            .live
            .iter()
            .filter_map(|p| match &p.taken {
                pickups::Taken::Orb(o) => Some((&**o, pickups::position(p, alpha), 1.0)),
                _ => None,
            });
        let frame = self.orb_frame();
        self.orb_geometry(camera, &frame, taken, None, out);
        self.elder_apparition_geometry(alpha, out);
        if self.client.mod_clear_particles.get() {
            self.apply_mod_particle_clear();
            out.truncate(first);
            if let Some((_, quads)) = providers {
                quads.clear();
            }
            {
                let mut frame = self.client.skin_frame.borrow_mut();
                frame.truncate(skin);
                frame.outline_jobs.truncate(outline_jobs);
                frame.outline_vertices = outline_vertices;
            }
            self.client.entity_spans.borrow_mut().truncate(spans);
            self.client.entity_motion.borrow_mut().truncate(motion);
            self.client.glowing_vertices.borrow_mut().truncate(glowing);
            return;
        }
        self.mark_motion(out.len(), 0);
        self.mark_stage(out.len(), "NONE");
    }
    pub fn close_container_client(&mut self) {
        assert!(
            !self.is_logical_server(),
            "close_container_client called on Server role"
        );
        self.pad_clear_focus();
        self.mod_browser_closed();
        self.reset_inventory_clicks();
        self.close_station_client();
        self.mount_closed();
        self.client.recipe_ui.0.reset_screen();
        self.remote_close_container();
    }
    /// Shared window/socket ScreenEvent Pre → the captured screen → cancellable Post.
    /// A declined event is never offered to a replacement screen or its bindings.
    pub fn dispatch_screen_key(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        key: KeyCode,
        pressed: bool,
        repeat: bool,
        shortcut: Option<sunrise_client::clipboard::EditShortcut>,
        source: Source,
    ) -> Result<bool> {
        if !self.context_menu_key_active() {
            self.dispatch_screen_key_as::<false>(event_loop, key, pressed, repeat, shortcut, source)
        } else {
            self.dispatch_screen_key_as::<true>(event_loop, key, pressed, repeat, shortcut, source)
        }
    }

    #[inline(always)]
    fn dispatch_screen_key_as<const MENU: bool>(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        key: KeyCode,
        pressed: bool,
        repeat: bool,
        shortcut: Option<sunrise_client::clipboard::EditShortcut>,
        source: Source,
    ) -> Result<bool> {
        if MENU && self.context_menu_key_event(source, key, pressed, repeat) {
            return Ok(true);
        }
        let instance = self.client.screen_state.0.screen_instance;
        let receiver = self.observed_input_screen();
        let canceled = if pressed {
            self.screen_key_pressed_on(receiver.as_ref(), key, repeat)
        } else {
            self.screen_key_released_on(receiver.as_ref(), key)
        };
        if canceled {
            if !pressed && source == Source::Keyboard {
                // Cleanup a physical release without dispatching it to another screen.
                self.remove_device_key(&key);
                self.mapped_key_event(key, false, false)?;
            }
            return Ok(true);
        }
        if self.client.screen_state.0.screen_instance == instance
            && self.screen_key_response(event_loop, key, pressed, repeat, shortcut)?
        {
            return Ok(true);
        }
        let post_canceled = self.screen_key_post_on(receiver.as_ref(), key, pressed, repeat);
        Ok(post_canceled
            || self.client.screen_state.0.screen_instance != instance
            || (pressed
                && ((receiver.is_some() && self.context_menu_open())
                    || self.mounted_widget_captures_input()
                    || self.observed_overlay_now().is_some()
                    || self.title_input_active()
                    || matches!(
                        self.client.screen,
                        Screen::Chat
                            | Screen::Options
                            | Screen::Social
                            | Screen::WorldOptions
                            | Screen::CommandBlock
                            | Screen::BlockEditor
                            | Screen::DebugOptions
                            | Screen::DemoIntro
                            | Screen::Win
                    ))))
    }
    /// Dispatch a window key through IME, the window's shortcuts and overlays, then any bound
    /// preset editor. `true` means no further screen or gameplay handling is needed.
    pub fn dispatch_window_key(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        input: WindowKeyInput<'_>,
        before_screen: impl FnOnce(&mut Self) -> bool,
    ) -> bool {
        let pressed = input.state == ElementState::Pressed;
        if pressed && !input.repeat && !self.client.pad.0.menu.synthetic {
            self.pad_radial_cancel();
        }
        if let PhysicalKey::Code(key) = input.physical {
            if pressed {
                // Only a press that finds no field focused can open one.
                let text = input
                    .text
                    .filter(|_| self.preset_editor_title().is_none() && !self.text_field_focused());
                self.client.held_key_text.pressed(key, text);
            } else {
                self.client.held_key_text.released(key);
            }
        }
        // The OS consumes composing keys before they can reach a screen or a shortcut.
        if (pressed && self.ime_suppresses_key()) || before_screen(self) {
            return true;
        }
        let key = match input.physical {
            PhysicalKey::Code(key) => Some(key),
            PhysicalKey::Unidentified(_) => None,
        };
        if let Err(error) = self.dispatch_window_keyboard(
            event_loop,
            key,
            pressed,
            input.repeat,
            input.shortcut,
            input.text,
        ) {
            self.common.error = Some(error);
            event_loop.exit();
        }
        true
    }
    /// ItemEntityRenderer: items bob and spin; blocks render as small cubes.
    pub fn entity_geometry(
        &mut self,
        camera: glam::DVec3,
        alpha: f64,
        billboard_frustum: Option<sunrise_client::gfx::JoinedFrusta>,
        mut out: Vec<Vertex>,
        additive: &mut Vec<Vertex>,
    ) -> (
        Vec<Vertex>,
        Arc<[StaticSectionSnapshot]>,
        Vec<[i32; 3]>,
        StaticCacheFrameStats,
    ) {
        // Its steps as spans (SUNRISE_SPIKES): each lap names the step that just ended.
        let mut laps = sunrise_level::spans::laps();
        let mut static_block_entities = std::sync::Arc::from([]);
        let mut visible_static_block_entity_sections = Vec::new();
        let mut static_block_entity_stats = StaticCacheFrameStats::default();
        self.client.entity_spans.borrow_mut().clear();
        self.client.entity_motion.borrow_mut().clear();
        self.client.entity_motion_posed.borrow_mut().clear();
        self.client.map_slots.next_frame();
        // LevelExtractor.extract: the transient blocks queued since the last frame.
        self.drain_transient_block_queue();
        self.prepare_entity_culling();
        self.prepare_occlusion_culling(camera);
        self.prepare_trims();
        self.begin_skinning();
        self.mark_entity(out.len(), "minecraft:entity_flame");
        // One scan for both passes that draw them (block entities below and the
        // overlay near the end of this function).
        let cracks = self.cracks(camera);
        let fire_yaw = self
            .client
            .posed_rotation
            .map_or(self.client.view.y_rot, |(y, _, _)| y);
        // Packs may move vertices or draw from another camera. The guarded
        // billboard frustum covers TAA's jitter without changing model culling.
        let fire_view = billboard_frustum.as_ref().filter(|_| {
            self.client.pack_ids.is_none() && self.client.native_shaders.picked.is_empty()
        });
        let fire_visible = |position, width, height| {
            fire_view.is_none_or(|view| {
                sunrise_client::fire_render::in_view(view, camera, position, width, height)
            })
        };
        laps.lap("eg_prep");
        for item in &self.common.dropped {
            if self.hides_as_camera(item.id) {
                continue;
            }
            if item.fire_ticks > 0
                && self
                    .stack_can_be_hurt_by(&item.stack, "minecraft:in_fire")
                    .unwrap_or(true)
            {
                let at = item.previous.lerp(item.position, alpha);
                if !fire_visible([at.x, at.y, at.z], 0.25, 0.25) {
                    continue;
                }
                sunrise_client::fire_render::flames(
                    [at.x, at.y, at.z],
                    0.25,
                    0.25,
                    camera,
                    fire_yaw,
                    &self.client.gui.atlas.sprites,
                    self.common.world.light(
                        at.x.floor() as i32,
                        at.y.floor() as i32,
                        at.z.floor() as i32,
                    ),
                    &mut out,
                );
            }
        }
        laps.lap("eg_fire_items");
        for stand in &self.common.armor_stands {
            if self.hides_as_camera(stand.id) {
                continue;
            }
            if stand.is_on_fire() && !stand.marker {
                let scale = if stand.small { 0.5 } else { 1.0 };
                let at = stand.render_position(alpha);
                if !fire_visible([at.x, at.y, at.z], 0.5 * scale, 1.975 * scale) {
                    continue;
                }
                sunrise_client::fire_render::flames(
                    [at.x, at.y, at.z],
                    0.5 * scale,
                    1.975 * scale,
                    camera,
                    fire_yaw,
                    &self.client.gui.atlas.sprites,
                    self.common.world.light(
                        at.x.floor() as i32,
                        at.y.floor() as i32,
                        at.z.floor() as i32,
                    ),
                    &mut out,
                );
            }
        }
        laps.lap("eg_fire_stands");
        // `LevelRenderer.submitFeatures`: `submitEntities` walks the entities in the order they
        // joined the level (`EntityLookup.byId`, as `LevelExtractor.extractVisibleEntities`
        // lists them), whatever the family, before `submitBlockEntities`. Each run of one
        // family is drawn by that family's own geometry.
        let orb_view = billboard_frustum.as_ref().filter(|_| {
            self.client.pack_ids.is_none() && self.client.native_shaders.picked.is_empty()
        });
        // What a family sets up once a frame, not once per run: the first run of the family
        // builds it.
        let tnt_sprites = std::cell::OnceCell::new();
        let falling_setup = std::cell::OnceCell::new();
        let orb_frame = std::cell::OnceCell::new();
        let mut runs = std::mem::take(&mut self.client.entity_runs);
        let general_frame = self.begin_general_geometry(camera, alpha);
        let crystal_frame = self.begin_crystal_geometry();
        self.ordered_draw_runs(general_frame.is_some(), crystal_frame.is_some(), &mut runs);
        // Sets the CEM frame even if there are no mobs; later passes also use it.
        let mob_frame = self.begin_mob_geometry(camera, alpha);
        for run in &runs {
            use crate::game::client_side::entity_client::DrawFamily as F;
            let range = run.range.clone();
            match run.family {
                F::Mob => self.mob_run_geometry(&mob_frame, range, &mut out),
                F::LocalPlayer => self.local_player_geometry(camera, &mut out),
                F::RemoteGeneral => {
                    if let Some(frame) = &general_frame {
                        self.general_run_geometry(frame, range, &mut out);
                    }
                }
                F::Dragon => {
                    // A dragon summoned outside the End draws at its join slot too.
                    if let Some(dragon) = self
                        .common
                        .dragon
                        .as_ref()
                        .filter(|d| !self.hides_as_camera(d.id))
                    {
                        self.dragon_geometry(dragon, camera, alpha, &mut out, additive);
                    }
                }
                F::Crystal => {
                    if let Some(frame) = &crystal_frame {
                        for crystal in &self.common.crystals[range] {
                            self.end_crystal_geometry(crystal, frame, camera, &mut out);
                        }
                    }
                }
                F::Item => {
                    // `ItemEntityRenderer`.
                    self.mark_entity(out.len(), "minecraft:item");
                    self.dropped_item_geometry(camera, alpha, &mut out, range);
                }
                F::Orb => {
                    self.mark_entity(out.len(), "minecraft:experience_orb");
                    let live = self.common.orbs[range]
                        .iter()
                        .filter(|o| !self.hides_as_camera(o.id))
                        .map(|o| {
                            let at = [
                                o.previous.x + (o.position.x - o.previous.x) * alpha,
                                o.previous.y + (o.position.y - o.previous.y) * alpha,
                                o.previous.z + (o.position.z - o.previous.z) * alpha,
                            ];
                            (o, at, alpha as f32)
                        });
                    let frame = orb_frame.get_or_init(|| self.orb_frame());
                    self.orb_geometry(camera, frame, live, orb_view, &mut out);
                }
                F::Falling => {
                    self.mark_entity(out.len(), "minecraft:falling_block");
                    let setup = falling_setup.get_or_init(|| self.falling_setup());
                    self.falling_geometry(camera, alpha, setup, range, &mut out);
                }
                F::Tnt => {
                    self.mark_entity(out.len(), "minecraft:tnt");
                    let sprites = *tnt_sprites.get_or_init(|| self.tnt_face_sprites());
                    self.tnt_geometry(camera, alpha, &mut out, range, sprites);
                }
                F::ArmorStand => {
                    self.mark_entity(out.len(), "minecraft:armor_stand");
                    self.armor_stand_geometry(camera, alpha, &mut out, range);
                }
                F::Boat => {
                    self.mark_entity(out.len(), "minecraft:oak_boat");
                    self.boat_geometry(camera, alpha, &mut out, range);
                }
                F::Cart => {
                    self.mark_entity(out.len(), "minecraft:minecart");
                    self.cart_geometry(camera, alpha, &mut out, range);
                }
                F::Trident => {
                    self.mark_entity(out.len(), "minecraft:trident");
                    self.trident_geometry(camera, alpha, &mut out, range);
                }
                F::Rocket => {
                    self.mark_entity(out.len(), "minecraft:firework_rocket");
                    self.rocket_geometry(camera, alpha, &mut out, range);
                }
            }
            self.mark_motion(out.len(), 0);
        }
        self.client.entity_runs = runs;
        laps.lap("eg_ordered_entities");
        // `VaultRenderer`.
        self.mark_entity(out.len(), "");
        self.vault_geometry(camera, alpha, &mut out);
        // `CampfireRenderer`.
        self.mark_entity(out.len(), "");
        self.campfire_geometry(camera, &mut out);
        self.brushable_geometry(camera, &mut out);
        self.shelf_geometry(camera, &mut out);
        self.mark_entity(out.len(), "minecraft:ominous_item_spawner");
        self.ominous_item_spawner_geometry(camera, alpha, &mut out);
        laps.lap("eg_items_misc");
        // Spawners and projectiles still have their own pass.
        self.other_creature_geometry(camera, alpha, &mut out);
        self.mark_motion(out.len(), 0);
        laps.lap("eg_other_creatures");
        // These billboards stay in their later pass, outside the ordered body runs.
        for m in &self.common.mobs {
            if self.hides_as_camera(m.id) {
                continue;
            }
            // Blaze.isOnFire: a charged blaze burns.
            let blazing = m.species() == sunrise_sim::mobs::Species::Blaze && m.extra.flag;
            if (m.is_on_fire() || blazing) && !m.removed {
                let at = m.previous.lerp(m.position, alpha);
                if !fire_visible([at.x, at.y, at.z], m.width(), m.height()) {
                    continue;
                }
                self.mark_entity(out.len(), &m.kind.id);
                sunrise_client::fire_render::flames(
                    [at.x, at.y, at.z],
                    m.width(),
                    m.height(),
                    camera,
                    fire_yaw,
                    &self.client.gui.atlas.sprites,
                    self.common.world.light(
                        at.x.floor() as i32,
                        at.y.floor() as i32,
                        at.z.floor() as i32,
                    ),
                    &mut out,
                );
            }
        }
        if self.own_body_in_view()
            && let Some(p) = &self.common.player
            && p.is_on_fire()
            && !p.spectator
        {
            let feet = self
                .common
                .previous_position
                .lerp(p.position, self.common.clock.camera_fraction());
            if fire_visible(
                [feet.x, feet.y, feet.z],
                p.dimensions.width,
                p.dimensions.height,
            ) {
                self.mark_player(out.len(), false);
                self.mark_motion(out.len(), Self::MOTION_OWN_PLAYER_FIRE);
                sunrise_client::fire_render::flames(
                    [feet.x, feet.y, feet.z],
                    p.dimensions.width,
                    p.dimensions.height,
                    camera,
                    fire_yaw,
                    &self.client.gui.atlas.sprites,
                    self.common.world.light(
                        feet.x.floor() as i32,
                        feet.y.floor() as i32,
                        feet.z.floor() as i32,
                    ),
                    &mut out,
                );
                self.mark_motion(out.len(), 0);
            }
        }
        laps.lap("eg_fire_living");
        self.mark_entity(out.len(), "minecraft:evoker_fangs");
        self.fangs_geometry(camera, alpha, &mut out);
        self.block_item_display_geometry(camera, alpha, &mut out);
        laps.lap("eg_displays");
        let mut see_through = std::mem::take(&mut self.client.see_through_vertices);
        see_through.clear();
        let mut st_collector = std::mem::take(&mut self.client.see_through_collector);
        st_collector.clear();
        self.name_tag_geometry(camera, alpha, &mut out, &mut st_collector);
        self.test_instance_marker_geometry(camera, &mut out, &mut st_collector);
        self.debug_renderer_geometry(camera, &mut out);
        self.text_display_geometry(camera, alpha, &mut out, &mut st_collector);
        st_collector.flush(&mut see_through);
        self.client.see_through_collector = st_collector;
        self.client.see_through_vertices = see_through;
        laps.lap("eg_name_tags");
        // Entity shadows (EntityRenderer.extractShadow); a native pack's sun shadows replace them.
        self.mark_translucent_entity(out.len(), "minecraft:entity_shadow");
        let pack_shadows = self
            .client
            .renderer
            .0
            .as_ref()
            .is_some_and(|r| r.pack_entity_shadows());
        let shadows = self.client.settings.entity_shadows
            && self.client.client_pack.polytone_colors.entity_shadows();
        let shadow_sprite = (shadows && !pack_shadows)
            .then(|| self.client.gui.atlas.sprites.get("misc/shadow").copied())
            .flatten();
        if let Some(sprite) = shadow_sprite {
            let ambient = self.dimension_type().ambient_light;
            let darken = self.sky_darken();
            let a = alpha;
            let lerp = |p: Vec3, q: Vec3| {
                glam::DVec3::new(
                    p.x + (q.x - p.x) * a,
                    p.y + (q.y - p.y) * a,
                    p.z + (q.z - p.z) * a,
                )
            };
            // Each caster carries (interpolated placement pos, current distance pos, radius, strength).
            let mut casters: Vec<(glam::DVec3, glam::DVec3, f32, f32)> =
                sunrise_client::shadows::mob_casters(
                    self.common
                        .mobs
                        .iter()
                        .filter(|m| !self.hides_as_camera(m.id)),
                    self.client_res().client_mob_models(),
                    a,
                )
                .map(|(pos, r, s)| (pos, pos, r, s))
                .collect();
            let item = sunrise_client::shadows::shadow_params("minecraft:item");
            casters.extend(
                self.common
                    .dropped
                    .iter()
                    .filter(|i| !self.hides_as_camera(i.id))
                    .map(|i| {
                        let p = lerp(i.previous, i.position);
                        (p, p, item.0, item.1)
                    }),
            );
            let orb = sunrise_client::shadows::shadow_params("minecraft:experience_orb");
            casters.extend(
                self.common
                    .orbs
                    .iter()
                    .filter(|o| !self.hides_as_camera(o.id))
                    .map(|o| {
                        let p = lerp(o.previous, o.position);
                        (p, p, orb.0, orb.1)
                    }),
            );
            // Text, block and item displays: same visibility gate, per-entity partial tick,
            // and current-position distance fade as `EntityRenderer.extractShadow`.
            let display_scale = self.view_scale();
            let display_now = std::time::Instant::now();
            casters.extend(self.client.text_displays.0.iter().filter_map(|(&id, d)| {
                let entity_partial = self.display_partial(id, a);
                if !self.display_visible(
                    id,
                    d,
                    camera,
                    entity_partial as f32,
                    display_scale,
                    display_now,
                ) {
                    return None;
                }
                let (pos, r, s) = d.shadow(entity_partial)?;
                Some((pos, d.position(), r, s))
            }));
            // Packs may move vertices or draw from another view.
            let shadow_view = self.client.view_frustum.as_ref().filter(|_| {
                self.client.pack_ids.is_none() && self.client.native_shaders.picked.is_empty()
            });
            for (pos, dist_pos, radius, strength) in casters {
                if let Some(view) = shadow_view
                    && !sunrise_client::shadows::in_view_from(
                        view, camera, pos, dist_pos, radius, strength,
                    )
                {
                    continue;
                }
                sunrise_client::shadows::shadow_from(
                    &self.common.world,
                    self.common.res.facts(),
                    &|state| {
                        self.common
                            .res
                            .collision()
                            .cached_shapes()
                            .cached_shape(state)
                            .ok()
                            .flatten()
                            .and_then(|shape| shape.bounds())
                    },
                    sprite,
                    camera,
                    pos,
                    dist_pos,
                    radius,
                    strength,
                    ambient,
                    darken,
                    &mut out,
                );
            }
        }
        laps.lap("eg_shadows");
        // Block entities (BlockEntityRenderers): chests, shulker boxes, banners, skulls...
        if let Some(r) = &self.client.renderer.0 {
            // Block entities outside the view frustum are skipped (with a
            // block of margin for models larger than their block).
            let view = self.client.view_frustum;
            // Beams and structure outlines reach far outside their block
            // (BlockEntityRenderer.shouldRenderOffScreen): never culled.
            let off_screen = |state: u32| self.block_entity_reach(state).off_screen();
            // Sections past 64 blocks are kept for the End gateway's 256.
            let long_range = |state: u32| self.block_entity_reach(state) == Reach::EndGateway;
            let in_view = |p: &[i32; 3]| {
                view.is_none_or(|v| {
                    let o = (glam::DVec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]))
                        - camera)
                        .as_vec3();
                    v.intersects(o - glam::Vec3::ONE, o + glam::Vec3::splat(2.0))
                })
            };
            // One walk of the uploaded sections' block entities feeds both lists below
            // (`Renderer::block_entities_in_range` prunes by range alone): the camera's list,
            // and the sections the GPU cache warms.
            let mut camera_list: Vec<&([i32; 3], u32)> = Vec::new();
            let mut cache_candidates: Vec<&([i32; 3], u32)> = Vec::new();
            // A shader pack keeps the cache off (`render_with_cache_options_and_callbacks`),
            // so nothing warms it.
            let warm_cache = self.client.pack_ids.is_none();
            for entity in r.block_entities_in_range(
                self.client.settings.chunk_fade as f32,
                camera,
                view,
                self.effective_render_distance(),
                long_range,
                off_screen,
            ) {
                let (pos, state) = *entity.block;
                if !self.live_block_entity_render(pos, state) {
                    continue;
                }
                if off_screen(state) || entity.section_inside || in_view(&pos) {
                    camera_list.push(entity.block);
                }
                if warm_cache && entity.near {
                    cache_candidates.push(entity.block);
                }
            }
            // A shader pack's shadow pass draws the block entities of every section it draws,
            // in the view or not (`Renderer::shadow_pass_block_entities`, #8464): those the
            // view leaves out go in hidden from the camera.
            let shadow_only: Vec<&([i32; 3], u32)> =
                if self.client.pack_ids.is_some() && self.client.occlusion.0.shadow_casters.0 {
                    let listed: HashSet<[i32; 3]> = camera_list.iter().map(|b| b.0).collect();
                    r.shadow_pass_block_entities(camera)
                        .filter(|(pos, state)| {
                            !listed.contains(pos) && self.live_block_entity_render(*pos, *state)
                        })
                        .collect()
                } else {
                    Vec::new()
                };
            let shadow_only_at: HashSet<[i32; 3]> = shadow_only.iter().map(|b| b.0).collect();
            let dynamic_visible = |pos: [i32; 3], state: u32, lo: glam::DVec3, hi: glam::DVec3| {
                if off_screen(state) || shadow_only_at.contains(&pos) {
                    return true;
                }
                if let Some(view) = view
                    && !view.intersects(
                        (lo - camera - glam::DVec3::splat(0.5)).as_vec3(),
                        (hi - camera + glam::DVec3::splat(0.5)).as_vec3(),
                    )
                {
                    return false;
                }
                if self.client.settings.sunrise.entity_culling == EntityCullingMode::Off
                    || self.client.occlusion.0.shadow_casters.0
                    || self.block_entity_reach_entry(state).1
                    || hi.y - lo.y > 2.0
                {
                    return true;
                }
                // Culler::aabb_bounds adds 0.05 on each side. Keep its final
                // query just inside the conservative rendered-model envelope.
                let bounds = Aabb::new(
                    Vec3::new(lo.x + 0.06, lo.y + 0.06, lo.z + 0.06),
                    Vec3::new(hi.x - 0.06, hi.y - 0.06, hi.z - 0.06),
                );
                !self.block_entity_occluded(bounds)
            };
            self.with_block_entity_scene(camera, alpha as f32, &dynamic_visible, |scene| {
                let visible = camera_list.into_iter().chain(shadow_only.iter().copied());
                let block_entity_cache = CacheOptions {
                    shulker_boxes: self.client.settings.sunrise.cache_shulker_boxes,
                    signs: self.client.settings.sunrise.cache_signs,
                    decorated_pots: self.client.settings.sunrise.cache_decorated_pots,
                    bells: self.client.settings.sunrise.cache_bells,
                    chests: self.client.settings.sunrise.cache_chests,
                    banners: self.client.settings.sunrise.cache_banners,
                    copper_golem_statues: self.client.settings.sunrise.cache_copper_golem_statues,
                };
                // Entity occlusion is for ordinary entity meshes. Block entities
                // already have section and per-block frustum culling; routing them
                // through the per-vertex occlusion callback both repeats that work
                // and disables their per-section GPU cache. Shader packs still need
                // per-entity span marks, so they keep the callback path.
                if self.client.pack_ids.is_some() {
                    let section_revision = |pos| r.block_entity_section_revision(pos);
                    static_block_entity_stats = self
                        .client_res()
                        .block_entity_render()
                        .render_with_cache_options_and_callbacks(
                            scene,
                            visible,
                            block_entity_cache,
                            r.block_entity_revision(),
                            Some(&section_revision),
                            &mut out,
                            |pos, state, start| {
                                self.mark_block_entity(start, state);
                                self.mark_motion(start, motion_block_entity(pos));
                            },
                            |pos, _, start, out| {
                                if shadow_only_at.contains(&pos) {
                                    self.hide_spans_from(start, out.len());
                                }
                            },
                        );
                } else {
                    (
                        static_block_entities,
                        visible_static_block_entity_sections,
                        static_block_entity_stats,
                    ) = self
                        .client_res()
                        .block_entity_render()
                        .render_with_gpu_cache_options(
                            scene,
                            visible,
                            cache_candidates.into_iter(),
                            block_entity_cache,
                            r.block_entity_revision(),
                            |pos| r.block_entity_section_revision(pos),
                            &mut out,
                        );
                }
                // Mod block entities: their mods draw them (`native_block_entity_submit`),
                // culled by the eye-visible section filter (ordinary renderers) and the bounds
                // they give. A world without any (`has_mods`) has none to find among its other
                // unrendered block entities (furnaces, hoppers...).
                let radius = self.effective_render_distance();
                let untemplated = self.untemplated_mod_block_entities(camera);
                let mods = self
                    .common
                    .world
                    .block_entities
                    .has_mods()
                    .then(|| {
                        r.visible_block_entities(
                            self.client.settings.chunk_fade as f32,
                            camera,
                            view,
                            radius,
                        )
                        .filter(|((_, state), _)| {
                            !self.client_res().block_entity_render().renders(*state)
                        })
                    })
                    .into_iter()
                    .flatten();
                let untemplated_eye = untemplated.iter().map(|block| {
                    (
                        block,
                        r.block_entity_section_in_eye_view(block.0, camera, view.as_ref(), radius),
                    )
                });
                let mod_cracks = self.mod_block_entity_geometry(
                    mods.chain(untemplated_eye),
                    camera,
                    alpha as f32,
                    &mut out,
                );
                self.mark_stage(out.len(), "DESTROY");
                out.extend(mod_cracks);
                for &(pos, stage) in &cracks {
                    let state = self.common.world.block(pos[0], pos[1], pos[2]);
                    self.client_res()
                        .block_entity_render()
                        .render_breaking(scene, pos, state.0, stage, &mut out);
                }
            });
        }
        laps.lap("eg_block_entities");
        self.mark_motion(out.len(), 0);
        self.mark_entity(out.len(), "");
        self.moving_piston_geometry(camera, alpha, &mut out);
        // LevelRenderer.submitTransientBlocks.
        self.transient_block_geometry(camera, &mut out);
        self.mark_entity(out.len(), "minecraft:painting");
        self.painting_geometry(camera, alpha, &mut out);
        self.mark_motion(out.len(), 0);
        self.mark_entity(out.len(), "minecraft:item_frame");
        self.frame_geometry(camera, alpha, &mut out);
        self.mark_motion(out.len(), 0);
        self.cushion_geometry(camera, &mut out);
        // Beams: `gbuffers_beaconbeam`, not a block entity's program (5.4 has no stage for them).
        self.mark_stage(out.len(), "NONE");
        self.beacon_geometry(camera, alpha, &mut out);
        self.mark_entity(out.len(), "minecraft:fishing_bobber");
        self.fishing_geometry(camera, alpha, &mut out);
        self.mark_entity(out.len(), "minecraft:leash_knot");
        self.leash_geometry(camera, alpha, &mut out);
        self.mark_entity(out.len(), "minecraft:splash_potion");
        self.thrown_potion_geometry(camera, alpha, &mut out);
        laps.lap("eg_objects");
        // Breaking progress overlays (destroy_stage_0..9): ours and the other players'.
        self.mark_stage(out.len(), "DESTROY");
        for &(pos, stage) in &cracks {
            let o = (glam::DVec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]))
                - camera)
                .as_vec3();
            let state = self.common.world.block(pos[0], pos[1], pos[2]);
            if self.common.res.facts().get(state).render_shape
                == sunrise_items::blocks::RenderShape::Model
                && let Ok(models) = self
                    .client
                    .client_pack
                    .render
                    .model_catalog
                    .collect(state, BlockPos::new(pos[0], pos[1], pos[2]))
            {
                let offset = self
                    .client
                    .client_pack
                    .context
                    .mesh
                    .tables
                    .offset(state, pos[0], pos[2])
                    .unwrap_or([0.0; 3]);
                let origin = o + glam::DVec3::from(offset).as_vec3();
                let translucent = models
                    .iter()
                    .any(|(_, model)| model.faces.iter().any(|f| f.translucent));
                for (_, model) in models {
                    sunrise_client::breaking::geometry(model, origin, translucent, stage, &mut out);
                }
            }
        }
        laps.lap("eg_cracks");
        self.finish_async_occlusion_culling();
        laps.lap("eg_culling_finish");
        (
            out,
            static_block_entities,
            visible_static_block_entity_sections,
            static_block_entity_stats,
        )
    }
    /// Aligns `locator_camera` with this frame's render camera before HUD extraction
    /// (`GameRenderer.renderLevel` sets `mainCamera` before `Gui.render`).
    pub fn extract_frame_hud(
        &mut self,
        size: [u32; 2],
        reload_overlay: Option<(f32, sunrise_client::level_loading::OverlayFade)>,
        title: bool,
        camera: [f64; 3],
        view_yaw: f32,
        view_pitch: f32,
        fov_degrees: f32,
    ) -> HudBatch {
        self.client.locator_camera =
            (!title).then_some((camera, view_yaw, view_pitch, fov_degrees));
        self.hud_with_reload(size, reload_overlay)
    }
    pub(in crate::game::client_side) fn falling_setup(&self) -> FallingSetup<'_> {
        FallingSetup {
            // Packs can move vertices or draw another view; otherwise only depth
            // planes are used, which TAA's X/Y projection jitter cannot change.
            view: self.client.view_frustum.as_ref().filter(|_| {
                self.client.pack_ids.is_none() && self.client.native_shaders.picked.is_empty()
            }),
            forward: self.camera_orientation() * -glam::Vec3::Z,
        }
    }
    pub(in crate::game) fn finish_close_world(&mut self) -> Result<()> {
        self.client.ambience.clear(&mut self.client.audio.0);
        // updateLevelInEngines(null): every sound stops; the music manager
        // lets go of its track.
        self.client
            .music
            .stop_playing(None, &mut self.client.audio.0);
        self.client.audio.0.leave_level();
        self.client.jukebox_sounds.clear();
        if self.client.remote.is_some() {
            self.client.world_icon = None;
            return self.leave_server(None);
        }
        // The save screen has finished every write before releasing storage.
        self.common.ui.dispatcher = Default::default();
        self.reset_creative_contents();
        self.common.server_items = None;
        self.common.recipe_tables.clear_session();
        self.reset_client_recipe_book();
        self.refresh_recipe_property_context();
        self.common.world_biomes = None;
        self.common.world_biomes_base = None;
        self.client.world_icon = None;
        sunrise_level::crash::clear_detail("World");
        self.client.debug_charts.reset();
        self.reset_server_debug_lines();
        self.client.boss_overlay.reset();
        // GameRenderer.resetData (Minecraft.clearClientLevel).
        self.client.item_activation.reset();
        self.client.kept_block_entities.get_mut().clear();
        self.client.mod_view_distances.take();
        self.client.streaming.paused = true;
        self.common.world =
            sunrise_gameplay_common::game::world_for_mods(self.common.res.facts().air, 0);
        self.common.player = None;
        // `LocalPlayer`'s constructor: `KeyMapping.resetToggleKeys`, so a sneak or sprint toggled
        // in this world is not still on in the next one.
        self.reset_toggle_keys();
        self.common.sleeping = None;
        self.common.sleep_timer = 0;
        self.client_clear_entities();
        self.set_screen(Screen::Playing);
        self.clear_lod();
        self.client.pending_uploads.0.clear();
        if let Some(r) = self.client.renderer.0.as_mut() {
            r.clear_sections(self.common.world.layout());
        }
        // Clear sync data (registries, dialog tags) so dialogs don't leak
        // between worlds.
        sunrise_gameplay_common::game::lan_common::sync::forget_sync_data();
        sunrise_gameplay_common::game::lan_common::sync::forget_server_sources();
        let mut title = self
            .new_title(std::env::current_dir().unwrap_or_default().join("saves"))
            .with_user(|| self.common.options.player_name());
        title.last_server = self.client.settings.file.string("lastServer");
        title.skip_multiplayer_warning = self.client.settings.file.bool("skipMultiplayerWarning");
        self.show_title(title);
        self.capture(false);
        Ok(())
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn handle_window_event_inner(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        event: WindowEvent,
        cursor_sample: Option<sunrise_client::window::PointerSample>,
        mouse_press_millis: Option<u64>,
    ) -> bool {
        self.handle_window_event_inner_as(
            event_loop,
            event,
            cursor_sample,
            mouse_press_millis,
            input_trace::InputOrigin::User,
        )
    }

    /// One window event, in the order vanilla's `KeyboardHandler` and `MouseHandler` see it.
    ///
    /// A controller doesn't come through here in play: `pad_action` goes straight to the mappings
    /// (`mapped_key`, `mouse_mapping`). So a key handled below before `mapped_key_event` (a window
    /// shortcut, a special case) never reaches a controller. If a player could want it on one, give
    /// it a handler both can call and a `gamepad::Route` (screenshot and fullscreen do this). A
    /// screen's own raw keys, text and mods' key hooks stay raw on purpose.
    /// `mouse_press_millis` is the monotonic timestamp of a physical press, absent for synthetic
    /// inputs and all other events.
    /// Returns true when button routing already settled the title before its Post.
    pub(in crate::game::client_side) fn handle_window_event_inner_as(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        event: WindowEvent,
        cursor_sample: Option<sunrise_client::window::PointerSample>,
        mouse_press_millis: Option<u64>,
        origin: input_trace::InputOrigin,
    ) -> bool {
        let before_screen =
            |game: &mut Self| game.prepare_window_event(event_loop, &event, cursor_sample);
        let handled = match &event {
            WindowEvent::KeyboardInput { event: key, .. } => self.dispatch_window_key(
                event_loop,
                WindowKeyInput {
                    physical: key.physical_key,
                    state: key.state,
                    repeat: key.repeat,
                    text: key.text.as_deref(),
                    shortcut: self.edit_shortcut(key),
                },
                before_screen,
            ),
            _ => before_screen(self),
        };
        if handled {
            return false;
        }
        let initial_instance = self.client.screen_state.0.screen_instance;
        use crate::game::client_side::native_hooks_client::{
            ScreenMousePost, ScreenMousePostKind, ScreenMousePre,
        };
        let mut mouse_post: Option<ScreenMousePost> = None;
        let mut title_settled = false;
        let result: Result<()> = (|| {
            match event {
                WindowEvent::CloseRequested => {
                    if !self.client.settings.quit_shortcuts
                        && sunrise_client::input::is_quit_shortcut_down(
                            self.client.modifiers,
                            &self.client.physical_keys,
                        )
                    {
                        return Ok(());
                    }
                    self.agent_cancel("the game window was closed; the game saves and exits");
                    self.begin_closing_save(true)?;
                }
                // Input method composition: the committed text types like a key's text.
                WindowEvent::Ime(winit::event::Ime::Commit(text)) => self.ime_commit(&text),
                WindowEvent::Ime(winit::event::Ime::Preedit(text, cursor)) => {
                    self.ime_preedit(&text, cursor)
                }
                WindowEvent::Ime(winit::event::Ime::Disabled) => {
                    sunrise_client::preedit::set_preedit(None)
                }
                WindowEvent::Resized(size) => {
                    if let Some(r) = &mut self.client.renderer.0 {
                        r.resize(size.width, size.height);
                    }
                    self.refresh_window_cursor();
                    self.observe_fullscreen();
                }
                WindowEvent::Focused(focused) => {
                    self.client.focused = focused;
                    self.client.focus_lost = (!focused).then(Instant::now);
                    if focused {
                        self.regrab_on_refocus();
                    } else {
                        // A held pointer goes with the focus, wherever the
                        // system keeps the grab: the window asks for it back
                        // when the focus returns (`regrab_on_refocus`).
                        let held = self.client.captured;
                        if self.common.control.is_none() {
                            self.client.keys.clear();
                            let options = self.toggle_options();
                            self.client
                                .vanilla_mappings
                                .set_keys_up_on_focus_loss(options);
                            // GLFW releases every held key and button when
                            // the window loses focus, so a mod's mappings go
                            // up as vanilla's do; their pending clicks stay.
                            self.client.mod_keys.set_all_up();
                            self.capture(false);
                        } else if self.client.screen_grab {
                            self.capture(false);
                        }
                        self.client.regrab_on_refocus = held;
                    }
                }
                WindowEvent::CursorMoved { .. } => {
                    if self.mod_screen_grabs_pointer() {
                        return Ok(());
                    }
                    if let (Some(r), Some(sample)) = (&self.client.renderer.0, cursor_sample) {
                        let size = [r.config.width, r.config.height];
                        let projection = self.pointer_projection(size);
                        let delta = self
                            .client
                            .window_cursor
                            .map_or([0.0; 2], |previous| projection.drag_delta(previous, sample));
                        self.set_window_cursor(sample, projection);
                        let [w, h] = projection.gui.size.map(|v| v as f32);
                        if self.context_menu_hover([w, h]) {
                            return Ok(());
                        }
                        let overlay = self.observed_overlay_now();
                        if self.observed_screen_view().is_none() && self.studio_mouse_dragged(w) {
                            return Ok(());
                        }
                        let button = if self.pad_synthetic_input() {
                            if self.client.pad.0.menu.held[0] {
                                Some(MouseButton::Left)
                            } else {
                                self.client.pad.0.menu.held[1].then_some(MouseButton::Right)
                            }
                        } else {
                            self.client
                                .drag_button
                                .filter(|b| self.client.physical_buttons.contains(b))
                        };
                        if self.screen_mouse_moved(delta) {
                            return Ok(());
                        }
                        if self.client.screen_state.0.screen_instance != initial_instance {
                            return Ok(());
                        }
                        match self.screen_mouse_dragged(delta) {
                            ScreenMousePre::Canceled => return Ok(()),
                            ScreenMousePre::Passed(post) => mouse_post = Some(post),
                            ScreenMousePre::NotFired => {}
                        }
                        if self.client.screen_state.0.screen_instance != initial_instance {
                            return Ok(());
                        }
                        if delta != [0.0; 2]
                            && let Some(button) = button
                        {
                            // A press on the item panel's list is the panel's until its
                            // release: no widget, menu or slot hears the drag.
                            let panel = self.mod_browser_drag_move();
                            let handled = panel
                                || (self.observed_screen_view().is_some()
                                    && self.mounted_widget_mouse_dragged(
                                        self.cursor(),
                                        glfw_button(button),
                                        delta,
                                        glfw_modifiers(self.client.modifiers),
                                    ));
                            let handled = handled
                                || (self.client.screen_state.0.screen_instance == initial_instance
                                    && self.screen_drag_response(overlay, button, size, (w, h))?);
                            if let Some(post) = mouse_post.as_mut() {
                                post.handled = handled;
                            }
                        }
                    }
                }
                WindowEvent::MouseWheel { .. }
                    if self.client.screen == Screen::Playing
                        && self.client.replay.driver.is_some() => {}
                WindowEvent::MouseWheel { delta, .. } => {
                    let (raw_x, raw_y) = match delta {
                        MouseScrollDelta::LineDelta(x, y) => (f64::from(x), f64::from(y)),
                        MouseScrollDelta::PixelDelta(p) => (p.x / 30.0, p.y / 30.0),
                    };
                    // MouseHandler.onScroll: discrete scrolling and the wheel sensitivity.
                    let discrete = self.client.settings.discrete_scroll;
                    let wheel_sensitivity = self.client.settings.mouse_wheel_sensitivity;
                    let scaled = |v: f64| {
                        (if discrete {
                            sunrise_client::mouse::java_signum(v)
                        } else {
                            v
                        }) * wheel_sensitivity
                    };
                    let d = scaled(raw_y) as f32;
                    if self.context_menu_wheel(d) {
                        return Ok(());
                    }
                    // ScreenEvent.MouseScrolled.Pre: the open screen hears of
                    // the wheel before anything scrolls, and a cancelled turn
                    // scrolls nothing — no bundle, no list, slider or
                    // scrollbar. The hotbar below reads the wheel only while
                    // playing, which the gate excludes. A wheel the pad or the
                    // keyboard's focus made up is not the mouse: it fires
                    // nothing.
                    let scroll = [scaled(raw_x), scaled(raw_y)];
                    if !self.pad_synthetic_input() {
                        let modifiers = glfw_modifiers(self.client.modifiers);
                        if let Some(receiver) = self.observed_input_screen() {
                            let mut canceled = false;
                            self.native_screen_mouse_scrolled_pre(
                                &receiver.view(),
                                scroll,
                                modifiers,
                                &mut canceled,
                            );
                            if canceled {
                                return Ok(());
                            }
                            mouse_post = Some(ScreenMousePost::new(
                                receiver,
                                modifiers,
                                ScreenMousePostKind::Scroll(scroll),
                            ));
                            if self.client.screen_state.0.screen_instance != initial_instance {
                                return Ok(());
                            }
                        }
                        if self.observed_screen_view().is_some() {
                            if self.mounted_widget_mouse_scrolled(self.cursor(), scroll, modifiers)
                            {
                                if let Some(post) = mouse_post.as_mut() {
                                    post.handled = true;
                                }
                                return Ok(());
                            }
                            if self.client.screen_state.0.screen_instance != initial_instance {
                                return Ok(());
                            }
                        }
                        if mouse_post.is_some() && self.container_wheel(d) {
                            if let Some(post) = mouse_post.as_mut() {
                                post.handled = true;
                            }
                            return Ok(());
                        }
                        if self.client.screen_state.0.screen_instance != initial_instance {
                            return Ok(());
                        }
                    }
                    // Only the topmost overlay is the receiving screen.
                    if let Some(overlay) = self.observed_overlay_now() {
                        let handled = match overlay {
                            "code_of_conduct" => self.code_of_conduct_scroll(d),
                            "dialog" => self.dialog_scroll(d),
                            _ => false,
                        };
                        if let Some(post) = mouse_post.as_mut() {
                            post.handled = handled;
                        }
                        return Ok(());
                    }
                    // The item panel beside a container screen pages on the
                    // wheel over it, as JEI's cancelled MouseScrolled.Pre: the
                    // screen underneath (bundle, creative tabs' list, station)
                    // and the Post event hear nothing of that notch.
                    if d != 0.0 && self.mod_browser_wheel(d) {
                        if let Some(post) = mouse_post.as_mut() {
                            post.handled = true;
                        }
                        return Ok(());
                    }
                    let mut screen_scrolled = false;
                    let stick_scroll = self.pad_stick_scroll();
                    let step_scroll = self.pad_step_scroll();
                    if self.client.screen == Screen::Playing
                        && let Some(title) = &mut self.client.title.0
                    {
                        let size = self
                            .client
                            .window
                            .0
                            .as_ref()
                            .map(|w| w.inner_size())
                            .unwrap_or_default();
                        let batch = HudBatch::new(
                            [size.width, size.height],
                            self.client.settings.gui_scale_inputs(),
                        );
                        let gui = (batch.width, batch.height);
                        if step_scroll {
                            title.step_scroll(d, self.client.cursor, gui);
                        } else if stick_scroll {
                            title.stick_scroll(d, self.client.cursor, gui);
                        } else {
                            screen_scrolled = title.scroll_worlds(d, self.client.cursor, gui);
                        }
                    }
                    // MouseHandler.onScroll: the hotbar (or the spectator
                    // menu) takes the wheel only when no screen is open; a
                    // book, lectern, sign or prompt drawn over gameplay is one.
                    if self.hotbar_takes_wheel() {
                        let scroll =
                            self.client
                                .mouse
                                .on_scroll(raw_x, raw_y, discrete, wheel_sensitivity);
                        let (horizontal_steps, vertical_steps) =
                            scroll.map_or((0, 0), |(_, steps)| steps);
                        if self.hotbar_scroll_canceled(
                            [scaled(raw_x), scaled(raw_y)],
                            [horizontal_steps, vertical_steps],
                        ) {
                            return Ok(());
                        }
                        let Some((wheel, _)) = scroll else {
                            return Ok(());
                        };
                        self.hotbar_wheel_step(wheel, vertical_steps);
                    }
                    match self.client.screen {
                        // ChatScreen.mouseScrolled always takes the wheel.
                        Screen::Chat => {
                            self.chat_scroll(d);
                            screen_scrolled = true;
                        }
                        Screen::Social => screen_scrolled |= self.social_scroll(d),
                        Screen::CommandBlock => screen_scrolled |= self.command_block_scroll(d),
                        Screen::Options => screen_scrolled |= self.options_scroll(d)?,
                        _ => {}
                    }
                    // AbstractContainerScreen.mouseScrolled: a bundle under the
                    // cursor takes the wheel before the screen's own scrolling.
                    let bundle = bundles::takes_bundle_wheel(self.client.screen)
                        && d != 0.0
                        && self.bundle_scroll(scaled(raw_x), scaled(raw_y));
                    if bundle {
                        screen_scrolled = true;
                    }
                    if self.client.screen == Screen::Creative
                        && d != 0.0
                        && !bundle
                        && self
                            .client
                            .creative_screen
                            .0
                            .can_scroll(&self.common.pack.names)
                    {
                        self.client
                            .creative_screen
                            .0
                            .wheel(f64::from(d), &self.common.pack.names);
                        screen_scrolled = true;
                    }
                    if self.client.screen == Screen::Inventory
                        && d != 0.0
                        && !bundle
                        && (self.tweak_scroll(d, self.inventory_target().0)
                            || self.station_scroll(d))
                    {
                        screen_scrolled = true;
                    }
                    screen_scrolled |= match self.client.screen {
                        Screen::Stats if d != 0.0 => self.stats_scroll(d),
                        Screen::WorldOptions if d != 0.0 => self.world_options_scroll(d),
                        Screen::ServerLinks if d != 0.0 => self.server_links_scroll(d),
                        Screen::DebugOptions if d != 0.0 => self.debug_options_scroll(d),
                        Screen::HudEditor if d != 0.0 => self.hud_editor_scroll(f64::from(d)),
                        Screen::Advancements => self.advancements_scroll(scroll[0] as f32, d),
                        _ => false,
                    };
                    if let Some(post) = mouse_post.as_mut() {
                        post.handled = screen_scrolled;
                    }
                }
                WindowEvent::MouseInput { state, button, .. } => {
                    let pressed = state == ElementState::Pressed;
                    let code = glfw_button(button);
                    let modifiers = glfw_modifiers(self.client.modifiers);
                    let synthetic = self.pad_synthetic_input();
                    let mouse_source = if synthetic {
                        context_menu::MouseSource::Game
                    } else {
                        origin.into()
                    };
                    if self.context_menu_mouse(mouse_source, button, pressed) {
                        return Ok(());
                    }
                    let owned_release = std::mem::take(&mut self.client.container_owned_up);
                    let had_screen = self.mod_mappings_screen_open() || self.remote_loading();
                    let receiver = (!synthetic).then(|| self.observed_input_screen()).flatten();
                    let title_mode = (self.client.screen == Screen::Playing
                        && !self.screen_over_playing())
                    .then(|| {
                        self.client
                            .title
                            .0
                            .as_ref()
                            .map(sunrise_client::title::Title::mode_activation)
                    })
                    .flatten();
                    let press_millis = mouse_press_millis.filter(|_| receiver.is_some());
                    let double_click = press_millis.is_some_and(|millis| {
                        self.client.screen_state.0.is_double_click(
                            millis,
                            initial_instance,
                            title_mode,
                            code,
                        )
                    });
                    if !synthetic {
                        self.track_drag_button(button, pressed);
                    }
                    if pressed {
                        self.mod_browser_other_press(button == MouseButton::Left);
                    }
                    if let Some(receiver) = &receiver {
                        let mut canceled = false;
                        if pressed {
                            self.native_screen_mouse_button_pressed_pre(
                                &receiver.view(),
                                code,
                                modifiers,
                                double_click,
                                &mut canceled,
                            );
                        } else {
                            self.native_screen_mouse_button_released_pre(
                                &receiver.view(),
                                code,
                                modifiers,
                                &mut canceled,
                            );
                        }
                        if canceled {
                            // A vetoed release of a press on the item panel
                            // drops its item, sending nothing.
                            if !pressed && button == MouseButton::Left {
                                self.mod_browser_release_vetoed();
                            }
                            if self.client.screen_state.0.screen_instance == initial_instance
                                && let Some(millis) = press_millis
                            {
                                self.client.screen_state.0.record_click(
                                    millis,
                                    initial_instance,
                                    title_mode,
                                    code,
                                );
                            }
                            return Ok(());
                        }
                    }
                    // The left release of a press on the item panel's list is
                    // the panel's, even once its menu closed or was replaced:
                    // no screen, mounted widget or container hook hears it.
                    let panel_release =
                        !pressed && button == MouseButton::Left && self.mod_browser_release();
                    let mut handled = owned_release || panel_release;
                    if !panel_release
                        && self.client.screen_state.0.screen_instance == initial_instance
                    {
                        let inventory_release = !pressed
                            && matches!(self.client.screen, Screen::Inventory | Screen::Creative)
                            && self.observed_overlay_now().is_none()
                            && !self.remote_loading();
                        let native_child = pressed.then(|| self.native_pointer_child()).flatten();
                        if let Some(child) = native_child {
                            // A hit belongs to the first ordered child even if it declines.
                            // Only the child's own answer can change focus.
                            let accepted =
                                self.route_screen_button(button, code, true, double_click)?;
                            if accepted
                                && button == MouseButton::Left
                                && self.client.screen_state.0.screen_instance == initial_instance
                            {
                                self.focus_native_pointer_child(child);
                            }
                            handled = true;
                        } else {
                            if !owned_release
                                && !inventory_release
                                && !self.key_focus_driving()
                                && self.observed_screen_view().is_some()
                            {
                                let mouse = self.cursor();
                                handled = if pressed {
                                    self.mounted_widget_mouse_clicked(
                                        mouse,
                                        code,
                                        modifiers,
                                        double_click,
                                    )
                                } else {
                                    self.mounted_widget_mouse_released(mouse, code, modifiers)
                                };
                            }
                            if !handled
                                && self.client.screen_state.0.screen_instance == initial_instance
                            {
                                handled = receiver.is_some()
                                    && self.container_mouse(button, pressed, false);
                            }
                            if !handled
                                && self.client.screen_state.0.screen_instance == initial_instance
                            {
                                handled =
                                    self.route_screen_button(button, code, pressed, double_click)?;
                            }
                        }
                        if inventory_release
                            && !handled
                            && self.client.screen_state.0.screen_instance == initial_instance
                            && !self.key_focus_driving()
                            && self.observed_screen_view().is_some()
                        {
                            handled =
                                self.mounted_widget_mouse_released(self.cursor(), code, modifiers);
                        }
                    }
                    if had_screen {
                        self.settle_title_mode();
                        title_settled = true;
                    }
                    if let Some(receiver) = receiver {
                        handled = self.screen_button_post(
                            &receiver,
                            code,
                            pressed,
                            modifiers,
                            double_click,
                            handled,
                        );
                    }
                    if handled
                        && self.client.screen_state.0.screen_instance == initial_instance
                        && let Some(millis) = press_millis
                    {
                        self.client.screen_state.0.record_click(
                            millis,
                            initial_instance,
                            title_mode,
                            code,
                        );
                    }
                    // Only the unhandled edge, with no screen now open, reaches mappings.
                    if handled
                        || self.mod_mappings_screen_open()
                        || self.remote_loading()
                        || self.client.reload_overlay.is_some()
                        || self.client.pending_entry.0.is_some()
                        || self.client.integrated_starting.is_some()
                        || self.client.pending_save.0.is_some()
                    {
                        return Ok(());
                    }
                    if pressed && !synthetic {
                        self.mod_mappings_button(button, true);
                    }
                    if pressed && !self.client.captured && !had_screen {
                        self.capture(true);
                        return Ok(());
                    }
                    let input = sunrise_client::keybinds::mouse_name(button);
                    if self.key_mapping_canceled(Some(input.as_str()), pressed) {
                        self.client.physical_buttons.remove(&button);
                        return Ok(());
                    }
                    if pressed && self.client.settings.keys.get("key.smoothCamera") == input {
                        self.smooth_camera_action();
                    }
                    self.mapped_mouse_event(button, &input, pressed)?;
                }
                WindowEvent::RedrawRequested => {
                    let flight_spans = self
                        .client
                        .flight
                        .0
                        .as_ref()
                        .filter(|flight| flight.raw)
                        .map(|_| sunrise_level::spans::record_frame());
                    sunrise_level::spans::time("recover_device", || self.recover_device())?;
                    self.observe_fullscreen();
                    self.sync_ime();
                    let now = Instant::now();
                    sunrise_threads::work::tick_start();
                    let elapsed = now.duration_since(self.common.previous);
                    self.client.frame_ticks = elapsed.as_secs_f32() * 20.0;
                    self.common.previous = now;
                    let booked = self.client.profile.unbooked.take();
                    if let Some(frame) = &booked {
                        self.record_perf_frame(
                            elapsed.as_secs_f32() * 1000.0,
                            frame.cpu_ms,
                            frame.gpu_ms,
                            &frame.spans,
                        );
                    }
                    self.client.upload_pace.frame(elapsed);
                    self.studio_fly(elapsed);
                    // Arrow-key camera controls belong to gameplay; chat and
                    // other screens use the same keys for their own navigation.
                    if self.client.screen == Screen::Playing
                        && self.client.title.0.is_none()
                        && self.client.sign_edit.is_none()
                        && self.client.book.is_none()
                        && !self.dialog_open()
                    {
                        let turn = |k| f32::from(u8::from(self.client.keys.raw(&k)));
                        let secs = elapsed.as_secs_f32().min(0.1);
                        let yaw = (turn(KeyCode::ArrowRight) - turn(KeyCode::ArrowLeft)) * 90.0;
                        let pitch = (turn(KeyCode::ArrowDown) - turn(KeyCode::ArrowUp)) * 90.0;
                        if yaw != 0.0 || pitch != 0.0 {
                            // 90 degrees a second, as `Entity.turn` degrees.
                            self.turn_player(
                                f64::from(yaw * secs / 0.15),
                                f64::from(pitch * secs / 0.15),
                            );
                        }
                    }
                    if self.client.title.0.is_none() {
                        if !self.common.is_logical_server() && self.client.remote.is_none() {
                            return Err(
                                "client frame has neither title screen nor remote connection"
                                    .into(),
                            );
                        }
                        sunrise_level::spans::time("update_world", || self.client_update_world());
                    } else if let Err(e) =
                        sunrise_level::spans::time("poll_remote", || self.poll_remote())
                    {
                        log::warn!("Sunrise: {e}");
                    }
                    let updated = Instant::now();
                    self.track_toggle_screens();
                    self.heal_hold_latches();
                    sunrise_hooks::engine::published(self.is_published());
                    self.update_window_title();
                    self.report_activity();
                    let target = self.tick_target_millis();
                    let steps = self.common.clock.advance(elapsed, target);
                    if steps > 0
                        && self.client.title.0.is_none()
                        && let Err(e) = self.poll_remote_before_tick()
                    {
                        log::warn!("Sunrise: {e}");
                    }
                    self.client_tick_steps(steps, Some(event_loop))?;
                    self.extras_frame(elapsed);
                    self.follow_retry()?;
                    self.apply_mouse_look();
                    self.input_look(elapsed.as_secs_f32());
                    self.advance_flight();
                    self.tick_world_icon();
                    let ticked = Instant::now();
                    let limit = self.framerate_limit();
                    let rendered = limit != 0 && self.render_and_flip(elapsed)?;
                    self.native_frame_end()?;
                    // Drain once for both the raw flight and the slow-frame/capture route.
                    let frame_spans = if sunrise_level::spans::recording_here() {
                        sunrise_level::spans::take_raw()
                    } else {
                        Vec::new()
                    };
                    let block_entity_work = flight_spans.as_ref().map_or(
                        [0; 2],
                        sunrise_level::spans::FrameRecording::block_entity_work,
                    );
                    drop(flight_spans);
                    let mut own_work = None;
                    if let Some(r) = self.client.renderer.0.as_mut() {
                        let cpu = now.elapsed().saturating_sub(r.last_wait);
                        sunrise_threads::work::tick_end(cpu.as_nanos() as u64);
                        let ms = |d: Duration| d.as_secs_f32() * 1000.0;
                        let gpu = if rendered {
                            r.gpu_times.last().copied()
                        } else {
                            None
                        };
                        own_work = Some((ms(cpu), gpu));
                        if rendered {
                            let g = &self.client.settings.sunrise;
                            // The scale while Dynamic Resolution moves it on its own.
                            let dynamic_scale = g
                                .dynamic_resolution
                                .then(|| self.client.dynamic_resolution.scale(g.resolution_scale));
                            let times = sunrise_hooks::perf::FrameTimes::new(
                                ms(elapsed),
                                ms(cpu),
                                r.gpu_times.last().copied(),
                                dynamic_scale,
                            );
                            sunrise_hooks::engine::frame_timed(&times);
                            if let Some(&gpu) = r.gpu_times.last() {
                                self.client.gpu_utilization =
                                    f64::from(gpu) * 100.0 / f64::from(ms(cpu).max(0.001));
                            }
                            let settled = self.client.screen == Screen::Playing
                                && !self.client.streaming.terrain_loading();
                            frame_autotune::observe(
                                &self.client.settings.sunrise,
                                settled,
                                (
                                    &mut self.client.gpu_culling_auto,
                                    &mut self.client.dynamic_resolution,
                                ),
                                r,
                                (ms(cpu), ms(elapsed)),
                            );
                            self.client.frame_stats.record_cpu(ms(cpu));
                            let render = r.last_encode;
                            self.client.frame_stats.record_phases([
                                ms(updated - now),
                                ms(ticked - updated),
                                ms(ticked.elapsed().saturating_sub(r.last_wait + render)),
                                ms(render),
                            ]);
                            let stats = r.static_block_entity_frame_stats();
                            self.client.frame_stats.record_block_entities(
                                sunrise_sim::perf_types::RenderCacheStats {
                                    snapshot_rebuilds: stats.snapshot_rebuilds,
                                    snapshot_bytes_built: stats.snapshot_bytes_built,
                                    static_bytes_uploaded: stats.static_bytes_uploaded,
                                    static_entities_drawn: stats.static_entities_drawn,
                                    dynamic_entities_drawn: stats.dynamic_entities_drawn,
                                },
                            );
                        }
                        self.client.frame_stats.record_gpu(&r.gpu_times);
                        r.gpu_times.clear();
                    }
                    self.limit_frame(limit);
                    let paused = self.pauses_game();
                    if paused != self.common.pause {
                        let mut cancel = false;
                        self.native_client_pause_change_pre(paused, &mut cancel);
                        if !cancel {
                            self.common.pause = paused;
                            self.native_client_pause_change_post(paused);
                        }
                    }
                    if let Some(server) = &self.client.integrated {
                        server.set_paused(self.common.pause);
                        server.set_distances(
                            self.client.settings.render_distance,
                            self.client.settings.simulation_distance,
                            self.client.settings.entity_distance,
                        );
                    }
                    self.common.clock.end_frame(self.common.pause);
                    self.poll_screenshot_writes();
                    let mut consumed = !self.client.chat_ui.screenshot_messages.is_empty();
                    let finished = self
                        .client
                        .renderer
                        .0
                        .as_mut()
                        .and_then(|r| r.screenshot_result.take())
                        .and_then(|result| match result {
                            Ok(pending) => self.save_screenshot(pending),
                            Err(error) => {
                                let result = Err(error);
                                self.finish_screenshot(&result, None);
                                Some(result)
                            }
                        });
                    if let Some(result) = &finished
                        && let Some(reply) = self.client.pending_screenshot.0.take()
                    {
                        let _ = reply.send(
                            match result {
                                Ok([w, h]) => {
                                    serde_json::json!({ "ok": true, "width": w, "height": h })
                                }
                                Err(e) => serde_json::json!({ "error": e }),
                            }
                            .to_string(),
                        );
                        consumed = true;
                    }
                    if let Some(result) = self
                        .client
                        .renderer
                        .0
                        .as_mut()
                        .and_then(|r| r.capture_result.take())
                        && let Some(reply) = self.client.pending_render_capture.0.take()
                    {
                        let value = result.unwrap_or_else(|e| serde_json::json!({ "error": e }));
                        let _ = reply.send(value.to_string());
                    }
                    // A result nobody asked for was the world icon's
                    // (`tick_world_icon`): a failure tries again a second
                    // later, as `tryTakeScreenshotIfNeeded` does.
                    if !consumed && let Some(Err(error)) = finished {
                        log::warn!("Sunrise: couldn't save the world icon: {error}");
                    }
                    self.record_streaming_work(rendered, block_entity_work, &frame_spans);
                    if let Some((cpu_ms, gpu_ms)) = own_work {
                        self.client.profile.unbooked = Some(profile_capture::UnbookedFrame {
                            cpu_ms,
                            gpu_ms,
                            spans: frame_spans,
                        });
                    }
                    if rendered {
                        self.client
                            .frame_stats
                            .record(elapsed.as_secs_f32() * 1000.0);
                        if sunrise_threads::alloc_count::ON {
                            self.record_frame_allocs();
                        }
                        self.client
                            .debug_charts
                            .frame_times
                            .log(elapsed.as_nanos() as i64);
                        if self.client.frames == 0 {
                            // The launcher times Play to this line (https://github.com/sunrise-org/sunrise-launcher/blob/main/src/perf.rs).
                            log::info!(
                                "Sunrise: first frame after {} ms",
                                self.client.started.elapsed().as_millis()
                            );
                        }
                        if self.client.title.0.is_some() {
                            self.client.perf.title_shown();
                        }
                        if self.client.title.0.is_none()
                            && self.client.screen == Screen::Playing
                            && self.common.player.is_some()
                        {
                            self.client.perf.frame(
                                elapsed.as_secs_f32() * 1000.0,
                                booked.as_ref().map_or(&[], |frame| frame.spans.as_slice()),
                            );
                            if startup::filling() {
                                let mut fill: Vec<(&'static str, usize)> =
                                    self.client.streaming.stages().to_vec();
                                fill.push(("resident", self.common.world.chunks.len()));
                                fill.push(("uploads", self.client.pending_uploads.0.len()));
                                fill.push(("backlog", self.remote_backlog()));
                                startup::counters("world.fill", &fill);
                            }
                            if startup::on() {
                                startup::view_filled(
                                    self.common.world.chunks.len(),
                                    self.client.streaming.pending()
                                        + self.client.pending_uploads.0.len()
                                        + self.remote_backlog(),
                                    self.client.streaming.radius.max(0) as u32,
                                );
                            }
                            self.client.world_frames += 1;
                        }
                        self.client.frames += 1;
                        self.client.fps_frames += 1;
                        sunrise_level::crash::debug_crash_tick();
                        sunrise_level::crash::relay_worker_panic();
                        if self.client.fps_at.elapsed() >= Duration::from_secs(1) {
                            self.client.fps = self.client.fps_frames as f32
                                / self.client.fps_at.elapsed().as_secs_f32();
                            self.client.fps_frames = 0;
                            self.client.fps_at = Instant::now();
                        }
                        if self
                            .common
                            .options
                            .frames
                            .is_some_and(|f| self.client.frames >= f)
                            || self
                                .common
                                .options
                                .world_frames
                                .is_some_and(|f| self.client.world_frames >= f)
                        {
                            event_loop.exit();
                        }
                    }
                }
                _ => {}
            }

            Ok(())
        })();
        if let Some(post) = mouse_post
            && result.is_ok()
        {
            self.screen_mouse_post(post);
        }
        if let Err(e) = result {
            self.common.error = Some(e);
            event_loop.exit();
        }
        title_settled
    }
    pub fn hud_layers(&mut self, size: [u32; 2]) -> HudBatch {
        use sunrise_client::gui_layers::VanillaGuiLayer;

        self.client.hud_status_heights = Default::default();
        self.client.hud_status_heights_set.set(None);
        self.input_hud_begin();
        self.client.subtitles.deferred.set(false);
        let mut batch = HudBatch::new(size, self.client.settings.gui_scale_inputs());
        batch.glint_colour = self.pvp_glint_texel();
        // GlobalSettingsUniform's GameTime, for a resource pack's text shader.
        batch.game_time = ((self.common.world.game_time.rem_euclid(24000)) as f32
            + self.common.clock.fraction() as f32)
            / 24000.0;
        let (w, h) = (batch.width, batch.height);
        // HDR calibration ends when its page does, whatever moved the screen
        // off Options (a death screen opened by a server, say).
        if self.client.screen != Screen::Options
            && self.client.chat_ui.options.calibration_window.is_some()
        {
            self.show_calibration((w, h));
        }
        if self.client.screen == Screen::Win {
            let drawn = self.draw_screen_with_pre_hook(&mut batch);
            if drawn {
                self.draw_win_screen(&mut batch);
            }
            // The early return skips the match below: the hooks still see the
            // screen, after its own drawing.
            self.publish_screen_regions(&batch, None);
            self.draw_screen_with_post_hook(&mut batch);
            return batch;
        }
        if self.client.screen == Screen::Options && self.client.title.0.is_some() {
            let drawn = self.draw_screen_with_pre_hook(&mut batch);
            self.draw_options(&mut batch, drawn);
            self.publish_screen_regions(&batch, None);
            self.draw_screen_with_post_hook(&mut batch);
            return batch;
        }
        if let Some(title) = &mut self.client.title.0
            && std::mem::take(&mut title.onboarding_finished)
        {
            // Options.onboardingAccessibilityFinished.
            self.client.settings.file.set(
                "onboardAccessibility",
                sunrise_client::options_file::V::Bool(false),
            );
            self.client
                .settings
                .save(&sunrise_gameplay_common::game::options_path());
        }
        if let Some(title) = &self.client.title.0 {
            for found in title.lan_found_narrations((w, h)) {
                self.say_system_queued(&found);
            }
        }
        if matches!(self.client.screen, Screen::DebugOptions | Screen::Mod)
            && self.client.title.0.is_some()
        {
            // DebugOptionsScreen or a mod's screen replaces the title; no level
            // HUD is behind it.
            let drawn = self.draw_screen_with_pre_hook(&mut batch);
            if drawn && self.client.screen == Screen::DebugOptions {
                self.draw_debug_options(&mut batch);
            } else if drawn {
                self.draw_mod_screen(&mut batch);
            }
            self.publish_screen_regions(&batch, None);
            self.draw_screen_with_post_hook(&mut batch);
            return batch;
        }
        if self.client.title.0.is_some() {
            // TitleScreen.extractRenderState, then ToastManager over it: vanilla
            // draws the toast manager on every screen, so a pack download or a
            // resource-reload failure still shows on the title (#4428).
            // The buttons crates add show by what is installed now: the Mods
            // page's list changes while the title is open.
            let mods = self.client.settings.mods.installed.len();
            if let Some(title) = &mut self.client.title.0 {
                title.mods = mods;
            }
            let drawn = self.draw_title_screen_with_pre_hook(&mut batch);
            if drawn && let Some(title) = &self.client.title.0 {
                title.draw_with_items(
                    &self.client.gui.atlas,
                    Some(&self.client.client_pack.item_renderer),
                    &mut batch,
                    self.client.cursor,
                    self.client.settings.file.bool("hideSplashTexts"),
                    self.client.settings.file.bool("hideServerAddress"),
                    &mut |phase, batch| self.draw_screen_phase(batch, phase),
                );
                self.draw_preset_editor(&mut batch);
            }
            // ScreenEvent.Render.Post over the title's own screens: a mod draws
            // over them here (a pack browser over Create New World, say), and the
            // toasts still come after, as over any screen.
            self.publish_screen_regions(&batch, None);
            self.draw_title_screen_with_post_hook(&mut batch);
            return batch;
        }
        // The ToastManager draws over every screen: a server's packs
        // download while the connection screen shows.
        if self.remote_overlay(&mut batch) {
            self.publish_screen_regions(&batch, None);
            return batch;
        }
        // Hud.extractRenderState: F1 (keyToggleGui) hides camera overlays, the
        // crosshair, hotbar, effects, bars, sidebar, action bar, titles and chat;
        // the sleep overlay, debug screen and open screens still draw.
        let spectator = self.common.spectator();
        let camera = self.camera_player();
        let (wi, hi) = (w as i32, h as i32);
        let mut gui_canceled = false;
        let replaying = self.client.replay.playing();
        if !replaying {
            self.native_render_gui_pre(&mut gui_canceled);
        }
        // F1 hides vanilla drawing; RenderGuiEvent.Pre instead cancels the layer walk.
        let hide_hud = self.client.chat_ui.hide_gui || replaying;
        let show_hud = !hide_hud && !gui_canceled;
        // A screen that keeps the in-game UI defers subtitles to its background,
        // even under F1. With no screen, Gui draws them after the debug overlay.
        let book_or_sign = self.client.book.is_some() || self.client.sign_edit.is_some();
        let subtitles = subtitles(
            show_hud,
            if self.client.server_packs.0.prompt.is_some() {
                Some(false)
            } else if self.client.screen != Screen::Playing || book_or_sign {
                Some(in_game_ui(self.client.screen, book_or_sign))
            } else {
                None
            },
        );
        let dispatch_hud = !gui_canceled && !replaying;
        self.client.hud_layer_next = dispatch_hud.then_some(0);
        self.client.chat_ui.room = self.hud_room(w, h, show_hud);
        let mut guide_from = 0;
        if dispatch_hud {
            if show_hud {
                self.draw_spyglass(&mut batch);
                guide_from = batch.vertices.len();
            }
            // Combat feel Legacy hides the attack indicator unless the player keeps it.
            let indicator = if show_hud && !self.pvp_hides_attack_indicator() {
                self.client.settings.attack_indicator
            } else {
                0
            };
            let crosshair_attack = (indicator == 1).then(|| {
                let speed = self.main_hand_attack_speed();
                (
                    self.client.hand_strength.scale(0.0, speed),
                    sunrise_sim::attack_strength::delay(speed),
                )
            });
            self.gui_layer(VanillaGuiLayer::Crosshair, &mut batch, &mut |g, batch| {
                // F3's 3D crosshair takes the 2D one's place. A spectator only sees it over
                // something to open (`Hud.extractCrosshair`). These guards only gate vanilla paint.
                if show_hud
                    && g.client.camera_type.is_first_person()
                    && (!spectator || g.spectator_crosshair())
                    && !g.debug_crosshair()
                    && !g.overlay_holds_view()
                {
                    let full = crosshair_attack.is_some_and(|(strength, delay)| {
                        hud_layout::full_attack_indicator(strength, delay, g.crosshair_living())
                    });
                    let draws = hud_layout::crosshair(
                        wi,
                        hi,
                        crosshair_attack.map(|(strength, _)| strength),
                        full,
                    );
                    // The custom crosshair takes the sprite's place (the first draw); the
                    // attack indicator stays.
                    if g.custom_crosshair() {
                        g.draw_custom_crosshair(batch, wi, hi);
                        hud_layout::paint(batch, &g.client.gui.atlas, &draws[1..]);
                    } else {
                        hud_layout::paint(batch, &g.client.gui.atlas, &draws);
                    }
                }
            });
            self.gui_layer(VanillaGuiLayer::Hotbar, &mut batch, &mut |g, batch| {
                if !show_hud {
                    return;
                }
                if spectator {
                    g.draw_spectator_hotbar(batch, g.own_player_is_spectator());
                } else if let Some((inputs, source)) = g.camera_hotbar(camera) {
                    g.draw_item_hotbar(batch, [wi, hi], &inputs, source);
                    if indicator == 2 {
                        // Items and arm follow the camera; the charge belongs to the local player.
                        let mut progress = g
                            .client
                            .hand_strength
                            .scale(0.0, g.main_hand_attack_speed());
                        g.native_hotbar_attack_indicator_progress(
                            crate::game::native_hooks::HotbarAttackIndicatorProgressCall {
                                progress: &mut progress,
                            },
                        );
                        let mut draws = Vec::new();
                        hud_layout::hotbar_attack_indicator(
                            wi,
                            hi,
                            inputs.right_handed,
                            progress,
                            &mut draws,
                        );
                        hud_layout::paint(batch, &g.client.gui.atlas, &draws);
                    }
                }
            });
            self.gui_layer(VanillaGuiLayer::Effects, &mut batch, &mut |g, batch| {
                // The PvP & HUD effects widget takes the icons' place.
                if show_hud && !g.pvp_effects_shown() {
                    g.draw_effect_icons(batch);
                }
            });
            self.gui_layer(VanillaGuiLayer::BossOverlay, &mut batch, &mut |g, batch| {
                if show_hud {
                    g.draw_boss_bars(batch);
                }
            });
            if show_hud {
                self.draw_extras(&mut batch);
            }

            let mut status_rows = Vec::new();
            let mount = if !show_hud {
                None
            } else {
                self.camera_vehicle_health(camera)
            };
            // NeoForge applies hudVisible and survivalVisible to the vanilla bodies only.
            let show_survival_status = show_hud && self.can_hurt_player();
            self.gui_layer(
                VanillaGuiLayer::PlayerHealth,
                &mut batch,
                &mut |g, batch| {
                    if !show_survival_status {
                        return;
                    }
                    let Some((inputs, kind)) = g.camera_health_inputs(camera) else {
                        return;
                    };
                    status_rows.clear();
                    g.client.health_hud.health_level(
                        &inputs,
                        kind,
                        wi,
                        hi,
                        &mut g.client.hud_status_heights,
                        &mut status_rows,
                    );
                    hud_layout::paint(batch, &g.client.gui.atlas, &status_rows);
                },
            );
            self.gui_layer(VanillaGuiLayer::ArmorLevel, &mut batch, &mut |g, batch| {
                if !show_survival_status {
                    return;
                }
                let Some(armor) = g.camera_armor(camera) else {
                    return;
                };
                status_rows.clear();
                hud_layout::armor_level(
                    armor,
                    wi,
                    hi,
                    &mut g.client.hud_status_heights,
                    &mut status_rows,
                );
                hud_layout::paint(batch, &g.client.gui.atlas, &status_rows);
            });
            self.gui_layer(VanillaGuiLayer::FoodLevel, &mut batch, &mut |g, batch| {
                if !show_survival_status || mount.is_some_and(|(hearts, _)| hearts != 0) {
                    return;
                }
                let Some(inputs) = g.camera_food_inputs(camera) else {
                    return;
                };
                status_rows.clear();
                g.client.health_hud.food_level(
                    &inputs,
                    wi,
                    hi,
                    &mut g.client.hud_status_heights,
                    &mut status_rows,
                );
                hud_layout::paint(batch, &g.client.gui.atlas, &status_rows);
            });
            self.gui_layer(
                VanillaGuiLayer::VehicleHealth,
                &mut batch,
                &mut |g, batch| {
                    let Some((hearts, health)) = mount else {
                        return;
                    };
                    status_rows.clear();
                    hud_layout::vehicle_health(
                        wi,
                        hi,
                        hearts,
                        health,
                        &mut g.client.hud_status_heights,
                        &mut status_rows,
                    );
                    hud_layout::paint(batch, &g.client.gui.atlas, &status_rows);
                },
            );
            self.gui_layer(VanillaGuiLayer::AirLevel, &mut batch, &mut |g, batch| {
                if !show_survival_status {
                    return;
                }
                let Some((inputs, sound_at)) = g.camera_air_inputs(camera) else {
                    return;
                };
                status_rows.clear();
                let pop_sound = g.client.health_hud.air_level(
                    &inputs,
                    wi,
                    hi,
                    &mut g.client.hud_status_heights,
                    &mut status_rows,
                );
                hud_layout::paint(batch, &g.client.gui.atlas, &status_rows);
                // Hud.playAirBubblePoppedSound: Player.playSound (PLAYERS), heard by this
                // client only, so another player's bubbles pop silently.
                if let Some((volume, pitch)) = pop_sound
                    && let Some(at) = sound_at
                {
                    let players = sunrise_level::sim_settings::SOUND_SOURCES
                        .iter()
                        .position(|s| *s == "player")
                        .unwrap_or(0);
                    let capture = g.client.audio.0.capture.take();
                    g.play_audio_on(players, "ui.hud.bubble_pop", Some(at), volume, pitch);
                    g.client.audio.0.capture = capture;
                }
            });
            use sunrise_client::locator_bar::ContextualBar;
            // After the health rows (Hud.extractHotbarAndDecorations): jump, locator or XP bar.
            self.gui_layer(
                VanillaGuiLayer::ContextualInfoBar,
                &mut batch,
                &mut |g, batch| {
                    if !show_hud {
                        return;
                    }
                    let has_experience = g.can_hurt_player();
                    let contextual = g.contextual_bar();
                    if contextual == ContextualBar::JumpableVehicle {
                        g.draw_jump_bar(batch, wi, hi);
                    }
                    if contextual == ContextualBar::Locator {
                        let (sprite, [x, y, w, h]) =
                            sunrise_client::locator_bar::background(wi, hi);
                        sunrise_client::ui::nine_slice(
                            batch,
                            &g.client.gui.atlas,
                            sprite,
                            x as f32,
                            y as f32,
                            w as f32,
                            h as f32,
                            u32::MAX,
                        );
                    }
                    if has_experience && g.player.is_some() {
                        let atlas = &g.client.gui.atlas;
                        let width = |s: &str| HudBatch::text_width(atlas, s) as i32;
                        let draws = hud_layout::experience_colored(
                            g.client.client_pack.polytone_colors.xp_bar,
                            wi,
                            hi,
                            g.experience.progress,
                            g.experience.level,
                            contextual == ContextualBar::Experience,
                            &width,
                        );
                        hud_layout::paint(batch, atlas, &draws);
                    }
                    if contextual == ContextualBar::Locator {
                        let draws = g.locator_bar_draws(wi, hi);
                        hud_layout::paint(batch, &g.client.gui.atlas, &draws);
                    }
                },
            );
            self.gui_layer(
                VanillaGuiLayer::SelectedItemName,
                &mut batch,
                &mut |g, batch| {
                    if show_hud && !spectator {
                        g.draw_selected_name(batch, g.can_hurt_player());
                    }
                },
            );
            // Hud.extractSleepOverlay draws even under F1.
            self.gui_layer(
                VanillaGuiLayer::SleepOverlay,
                &mut batch,
                &mut |g, batch| {
                    let draws = hud_overlays::sleep_overlay(wi, hi, g.sleep_timer);
                    hud_layout::paint(batch, &g.client.gui.atlas, &draws);
                },
            );
            self.gui_layer(
                VanillaGuiLayer::OverlayMessage,
                &mut batch,
                &mut |g, batch| {
                    if show_hud {
                        g.draw_overlay_message(batch);
                    }
                },
            );
            self.gui_layer(VanillaGuiLayer::Title, &mut batch, &mut |g, batch| {
                if show_hud {
                    g.draw_title(batch);
                }
            });
            self.gui_layer(
                VanillaGuiLayer::ScoreboardSidebar,
                &mut batch,
                &mut |g, batch| {
                    if show_hud {
                        g.draw_scoreboard(batch);
                    }
                },
            );
            self.gui_layer(
                VanillaGuiLayer::SubtitleOverlay,
                &mut batch,
                &mut |g, batch| match subtitles {
                    Subtitles::Hidden => {}
                    Subtitles::Now => g.draw_subtitles(batch),
                    Subtitles::Deferred => g.client.subtitles.deferred.set(true),
                },
            );
            let atlas = &self.client.gui.atlas;
            if show_hud
                && let Some((text, at)) = &self.client.message
                && at.elapsed() < Duration::from_secs(3)
            {
                let tw = HudBatch::text_width(atlas, text);
                batch.text(atlas, text, (w - tw) / 2.0, h - 59.0, u32::MAX, true);
            }
            self.gui_layer(VanillaGuiLayer::Chat, &mut batch, &mut |g, batch| {
                // The chat screen has its own paint site for focused chat.
                if show_hud && g.client.screen != Screen::Chat && !g.overlay_holds_view() {
                    g.draw_chat(batch);
                }
            });
            self.gui_layer(VanillaGuiLayer::TabList, &mut batch, &mut |g, batch| {
                // Options polls world skins separately, even if the HUD is hidden.
                if show_hud && !matches!(g.client.screen, Screen::Chat | Screen::Options) {
                    g.draw_tab_list(batch);
                }
            });
            self.draw_mod_gui_layers(None, &mut batch);
            self.client.hud_layer_next = None;
            self.client.hud_status_heights_set.set(None);
            self.native_render_gui_post(&mut batch);
            self.client.hud_layer_bounds.clear();
        }
        // Screen.extractBlurredBackground blurs the world and HUD, while the
        // screen's background, widgets, toasts and debug overlay stay sharp.
        batch.blur_at = batch.vertices.len() as u32;
        // PackConfirmScreen takes the open screen's place (it comes back once
        // the prompt is answered): the screen under the prompt isn't drawn.
        let screen = if self.client.server_packs.0.prompt.is_some() || self.client.book.is_some() {
            Screen::Playing
        } else {
            self.client.screen
        };
        if screen == Screen::Options {
            self.flush_skin_slots();
            self.update_remote_skins();
        }
        let drawn = self.draw_screen_with_pre_hook(&mut batch);
        let mut region_screen = None;
        match screen {
            Screen::Options => self.draw_options(&mut batch, drawn),
            _ if !drawn => {}
            Screen::Paused if self.client.pause_menu => self.draw_pause_menu(&mut batch),
            // PauseScreen(false): only its title, at the top.
            Screen::Paused => {
                self.draw_screen_phase(
                    &mut batch,
                    sunrise_client::screen_draw::RenderPhase::Background,
                );
                let title = self.translate_with_fallback("menu.paused", "Game Paused");
                sunrise_client::widgets::begin_screen(|| {
                    sunrise_client::widgets::ScreenInfo::new("paused", title.clone())
                });
                let tw = HudBatch::text_width(&self.client.gui.atlas, &title);
                batch.text(
                    &self.client.gui.atlas,
                    &title,
                    ((batch.width / 2.0).floor() - (tw / 2.0).floor()).floor(),
                    10.0,
                    u32::MAX,
                    true,
                );
                self.draw_screen_phase(
                    &mut batch,
                    sunrise_client::screen_draw::RenderPhase::Foreground,
                );
            }
            Screen::GameModeSwitcher => self.draw_gamemode_switcher(&mut batch),
            Screen::Dead => self.draw_death_screen(&mut batch),
            Screen::Stats => self.draw_stats(&mut batch),
            Screen::Advancements => self.draw_advancements(&mut batch),
            Screen::Mod => self.draw_mod_screen(&mut batch),
            Screen::WorldOptions => self.draw_world_options(&mut batch),
            Screen::Friends => self.draw_friends_screen(&mut batch),
            Screen::Social => self.draw_social_screen(&mut batch),
            Screen::ServerLinks => self.draw_server_links(&mut batch),
            Screen::ConfirmLink => self.draw_confirm_link(&mut batch),
            Screen::DemoIntro => self.draw_demo_intro(&mut batch),
            Screen::DialogWarning => self.draw_dialog_warning(&mut batch),
            Screen::CommandBlock => self.draw_command_block(&mut batch),
            Screen::BlockEditor => self.draw_block_editor(&mut batch),
            Screen::DebugOptions => self.draw_debug_options(&mut batch),
            Screen::HudEditor => self.draw_hud_editor(&mut batch),
            Screen::Inventory => {
                let from = batch.vertices.len();
                if self.draw_inventory(&mut batch) {
                    region_screen = Some(Screen::Inventory);
                }
                self.draw_pad_menu_guide(&mut batch, from);
            }
            Screen::Creative => {
                let from = batch.vertices.len();
                if self.draw_creative(&mut batch) {
                    region_screen = Some(Screen::Creative);
                }
                self.draw_pad_menu_guide(&mut batch, from);
            }
            Screen::Chat => self.draw_chat_screen(&mut batch),
            Screen::Playing | Screen::Win => {}
        }
        // ScreenEvent.Render.Post: the open screen has drawn itself and its
        // tooltips, so a native mod draws over it here. Never `Playing`: the
        // prompts, books and sign editors drawn over gameplay keep it, draw
        // after this point and so over what a mod drew.
        self.publish_screen_regions(&batch, region_screen);
        self.draw_screen_with_post_hook(&mut batch);
        self.draw_saving_indicator(&mut batch);
        if self.client.server_packs.0.prompt.is_none() {
            self.remote_hud(&mut batch);
            self.draw_sign_editor(&mut batch);
            self.draw_book(&mut batch);
        }
        self.draw_studio_bars(&mut batch);
        if show_hud {
            self.draw_pad_guide(&mut batch, guide_from);
        }
        batch
    }
    pub(in crate::game) fn hud_with_reload(
        &mut self,
        size: [u32; 2],
        reload_overlay: Option<(f32, sunrise_client::level_loading::OverlayFade)>,
    ) -> HudBatch {
        // Gui.extractRenderState passes integer coordinates to the HUD and screen. Keep
        // the projected fractions for input, but draw and hit-test hover at the frame's pixel.
        let pointer = self.project_window_cursor(size);
        if let Some(pointer) = pointer {
            self.client.cursor = pointer.pixel();
        }
        // Off a world (title, options, Win, a prompt before joining) the HUD shows nothing to
        // keep clear of; `hud_layers` fills it in once the world's HUD draws.
        let gui = HudBatch::new(size, self.client.settings.gui_scale_inputs());
        self.client.chat_ui.room = sunrise_client::hud_kit::room::Room::new(gui.width, gui.height);
        // The pack prompt replaces the connecting screen (setScreen), but
        // in game it opens over the world.
        self.settle_title_toasts();
        let mut batch = if (self.client.server_packs.0.prompt.is_some()
            || self.client.code_of_conduct.is_some()
            || self.dialog_open())
            && self.common.player.is_none()
        {
            gui
        } else {
            self.hud_layers(size)
        };
        // Vanilla's confirmation replaces the dialog as the screen.
        if self.client.command_prompt.0.is_none() {
            self.draw_dialog(&mut batch);
        }
        self.draw_pack_prompt(&mut batch);
        self.draw_code_of_conduct(&mut batch);
        self.draw_link_prompt(&mut batch);
        self.draw_command_prompt(&mut batch);
        self.draw_context_menu(&mut batch);
        if let Some((bar, fade)) = reload_overlay {
            self.draw_reload_overlay(&mut batch, bar, fade);
        }
        // Gui.extractRenderState draws toasts over the active screen or overlay,
        // after the saving indicator and before the debug overlay.
        self.draw_every_toast(&mut batch);
        // Gui.extractRenderState: the debug overlay comes after the screen
        // and the toasts, so it is over everything, menus included.
        self.draw_debug_overlay(&mut batch, size);
        self.draw_deferred_subtitles(&mut batch);
        // The layers crates registered to draw over everything (`sunrise_hooks::hud`).
        sunrise_client::extras_hud::draw_top_layers(
            sunrise_hooks::hud::engine::all(),
            &mut batch,
            &self.client.gui.atlas,
        );
        self.draw_pad_over_hud(&mut batch);
        if let Some(pointer) = pointer {
            debug_assert_eq!(
                self.client.cursor,
                pointer.pixel(),
                "drawing changed the input cursor"
            );
            self.client.cursor = pointer.subpixel();
        }
        self.client.client_pack.item_renderer.end_gui_frame();
        batch
    }
    /// `LeverBlock.makeParticle` at full scale: a powered lever puffs one red
    /// dust on the clicking client (`LeverBlock.useWithoutItem`).
    pub(in crate::game::client_side) fn lever_puff(&mut self, pos: [i32; 3]) {
        assert!(
            !self.is_logical_server(),
            "lever_puff called on Server role"
        );
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        let Some(registry_state) = self.common.pack.assets.registry.state(state) else {
            return;
        };
        let prop = |key: &str| registry_state.properties.get(key).map(String::as_str);
        if prop("powered") != Some("true") {
            return;
        }
        // FACING is horizontal; getConnectedDirection is the face's direction.
        let facing = match prop("facing") {
            Some("north") => [0, 0, -1],
            Some("south") => [0, 0, 1],
            Some("west") => [-1, 0, 0],
            Some("east") => [1, 0, 0],
            _ => return,
        };
        let connected = match prop("face") {
            Some("ceiling") => [0, -1, 0],
            Some("floor") => [0, 1, 0],
            _ => facing,
        };
        let at = |base: i32, i: usize| {
            f64::from(base) + 0.5 + 0.1 * f64::from(-facing[i]) + 0.2 * f64::from(-connected[i])
        };
        let p = [at(pos[0], 0), at(pos[1], 1), at(pos[2], 2)];
        self.client.particles.0.add(
            &sunrise_level::particle_options::ParticleOptions::dust([1.0, 0.0, 0.0], 1.0),
            p,
            [0.0; 3],
        );
    }
    /// The entity whose geometry starts at vertex `start` (0: none), for frame
    /// generation to follow its own motion. Each run lasts until the next mark.
    pub(crate) fn mark_motion(&self, start: usize, id: u64) {
        self.client
            .entity_motion
            .borrow_mut()
            .push((start as u32, id));
        // Where its GPU-posed vertices start (`entity_skin`): a shader pack's `at_velocity`
        // follows those too.
        let posed = self.client.skin_frame.borrow().vertices;
        self.client.entity_motion_posed.borrow_mut().push(posed);
    }
    pub(in crate::game) fn pointer_space(
        &self,
        framebuffer: [u32; 2],
    ) -> sunrise_client::window::PointerSpace {
        use sunrise_client::window::PointerSpace;
        match self.client.window.0.as_deref() {
            Some(window) => *self
                .client
                .window_pointer_space
                .get_or_init(|| PointerSpace::from_window(window)),
            _ => PointerSpace::new(framebuffer, 1.0),
        }
    }
    /// Window bookkeeping, shortcuts and overlays before dispatching to the screen. Returns
    /// `true` when an early handler consumed the event.
    fn screen_mouse_moved(&self, delta: [f32; 2]) -> bool {
        if self.pad_synthetic_input()
            || delta == [0.0; 2]
            || self.client.screen_grab
            || !mod_api::__private::listened(mod_api::HookId::ScreenMouseMovedPre)
        {
            return false;
        }
        let Some(receiver) = self.observed_input_screen() else {
            return false;
        };
        let mut canceled = false;
        self.native_screen_mouse_moved_pre(
            &receiver.view(),
            delta,
            glfw_modifiers(self.client.modifiers),
            &mut canceled,
        );
        canceled
    }
    /// The files of the last drop, fired as one list for the open screen (`Screen.onFilesDrop`):
    /// before the next window event, or when the event-loop pass ends. Nothing is built with no
    /// listener or no screen.
    pub(in crate::game) fn fire_dropped_files(&mut self) {
        if self.client.dropped_files.is_empty() {
            return;
        }
        let paths = std::mem::take(&mut self.client.dropped_files);
        if !mod_api::__private::listened(mod_api::HookId::ScreenFilesDropped) {
            return;
        }
        if let Some(receiver) = self.observed_input_screen() {
            self.native_screen_files_dropped(&receiver.view(), &paths);
        }
    }
    pub(in crate::game::client_side) fn prepare_window_event(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        event: &WindowEvent,
        cursor_sample: Option<sunrise_client::window::PointerSample>,
    ) -> bool {
        if matches!(
            event,
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }
        ) {
            self.client.window_pointer_space.take();
        }
        if matches!(
            event,
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            }
        ) && !self.client.pad.0.menu.synthetic
        {
            self.pad_radial_cancel();
        }
        if let WindowEvent::Focused(false) = event {
            self.client.modifiers = winit::keyboard::ModifiersState::empty();
            self.client.physical_keys.clear();
            self.client.physical_buttons.clear();
            self.client.drag_button = None;
            self.client.container_buttons.clear();
            self.client.screen_state.0.context_menu_input.clear();
            self.mod_browser_focus_lost();
        }
        if let WindowEvent::ModifiersChanged(m) = event {
            self.update_modifiers(m.state());
        }
        if let WindowEvent::Focused(false) = event {
            self.update_modifiers(winit::keyboard::ModifiersState::empty());
            self.client.debug_keys.reset();
        }
        // Pack drops on the resource-pack and Create New World screens.
        if let WindowEvent::DroppedFile(path) = event {
            self.drop_pack_file(path);
            if mod_api::__private::listened(mod_api::HookId::ScreenFilesDropped) {
                self.client.dropped_files.push(path.clone());
            }
        }
        // Fullscreen is a window shortcut, before screens consume keyboard input.
        // Not Alt or Super on their own: leaving the window isn't playing with the keyboard.
        if let WindowEvent::KeyboardInput { event, .. } = event
            && event.state == ElementState::Pressed
            && !event.repeat
            && !matches!(
                event.physical_key,
                PhysicalKey::Code(
                    KeyCode::AltLeft | KeyCode::AltRight | KeyCode::SuperLeft | KeyCode::SuperRight
                )
            )
        {
            self.client.pad.0.keyboard_took_over = true;
        }
        if let WindowEvent::KeyboardInput { event, .. } = event
            && event.state == ElementState::Pressed
            && let PhysicalKey::Code(key) = event.physical_key
            && sunrise_client::keybinds::key_name(key)
                == Some(self.client.settings.keys.get("key.fullscreen"))
            && !(self.client.screen == Screen::Options && self.options_waiting_for_key())
        {
            self.client.last_input = Instant::now();
            if !event.repeat {
                self.toggle_fullscreen();
            }
            return true;
        }
        // A screenshot is a window shortcut too: vanilla takes it before a
        // screen or a text box consumes the key (`handleGlobalKeyPress`), so
        // it works with chat, a sign or a book open.
        if let WindowEvent::KeyboardInput { event, .. } = event
            && event.state == ElementState::Pressed
            && let PhysicalKey::Code(key) = event.physical_key
        {
            self.client.last_input = Instant::now();
            if self.screenshot_shortcut(key, event.repeat) {
                return true;
            }
        }
        // KeyboardHandler.keyPress: the debug keys come before the game's.
        if let WindowEvent::KeyboardInput {
            event: key_event, ..
        } = event
            && let PhysicalKey::Code(key) = key_event.physical_key
        {
            use sunrise_client::debug_keys::KeyAction;
            let action = match (key_event.state, key_event.repeat) {
                (ElementState::Released, _) => KeyAction::Release,
                (_, false) => KeyAction::Press,
                (_, true) => KeyAction::Repeat,
            };
            if self.context_menu_key_active()
                && (self.context_menu_open()
                    || (action != KeyAction::Press
                        && self.context_menu_owns_key(Source::Keyboard, key)))
            {
                if action == KeyAction::Release {
                    self.debug_key_release_cleanup(key);
                }
            } else if self.debug_key(key, action) {
                return true;
            }
        }
        // KeyboardHandler.keyPress: the narrator hotkey, on every screen and
        // in game, before the screen takes the key.
        if let WindowEvent::KeyboardInput { event, .. } = event {
            self.narrator_hotkey(event);
        }
        if matches!(
            event,
            WindowEvent::KeyboardInput { .. }
                | WindowEvent::MouseInput { .. }
                | WindowEvent::MouseWheel { .. }
                | WindowEvent::CursorMoved { .. }
        ) {
            self.client.last_input = Instant::now();
        }
        if matches!(
            event,
            WindowEvent::MouseInput { .. } | WindowEvent::MouseWheel { .. }
        ) {
            if !self.pad_synthetic_input() {
                self.reassert_pointer_capture();
            }
            // A click can arrive before the first redraw at a changed GUI size.
            self.refresh_window_cursor();
        }
        // The real mouse takes the pointer back from the keyboard's focus.
        self.key_focus_mouse(event_loop, event, cursor_sample);
        // The screen's narration hears of the input before the screen takes
        // it; the pointer events the keyboard's focus sends aren't the mouse.
        if !self.key_focus_quiet() {
            narrator::narrate_input(&mut self.client.narration, event, self.client.cursor);
        }
        // FileFixerProgressScreen: Cancel is the only way out (`shouldCloseOnEsc`
        // is false), so it takes the pointer and Escape does nothing.
        if let Some(starting) = &self.client.integrated_starting
            && starting.handle.upgrade().is_file_fixing()
        {
            let upgrade = starting.handle.upgrade();
            match event {
                WindowEvent::CursorMoved { .. } => {
                    if let (Some(projection), Some(sample)) =
                        (self.renderer_pointer_projection(), cursor_sample)
                    {
                        self.set_window_cursor(sample, projection);
                    }
                }
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                    ..
                } if !upgrade.is_canceled() => {
                    let (w, h) = self.gui_size();
                    let (x, y) = sunrise_client::level_loading::file_fixer_cancel(w, h);
                    if sunrise_client::ui::hovered(self.client.cursor, x, y, 200.0, 20.0) {
                        upgrade.set_canceled();
                        self.play_audio_button_click();
                    }
                }
                _ => {}
            }
            if matches!(event, WindowEvent::KeyboardInput { .. }) {
                return true;
            }
        }
        if self.client.pending_entry.0.is_some()
            && matches!(event, WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && event.physical_key == PhysicalKey::Code(KeyCode::Escape))
        {
            self.client.pending_entry.0 = None;
            self.client.perf.cancel_world_opening();
            return true;
        }
        if (self.client.pending_entry.0.is_some() || self.client.integrated_starting.is_some())
            && !matches!(
                event,
                WindowEvent::CloseRequested
                    | WindowEvent::Resized(_)
                    | WindowEvent::RedrawRequested
            )
        {
            return true;
        }
        // The pack reload's overlay takes no input until it has faded out
        // (`LoadingOverlay`): `spawn_pack_reload` released what was held.
        if self.client.reload_overlay.is_some()
            && !matches!(
                event,
                WindowEvent::CloseRequested
                    | WindowEvent::Focused(_)
                    | WindowEvent::Resized(_)
                    | WindowEvent::RedrawRequested
            )
        {
            return true;
        }
        if self.client.pending_save.0.is_some() {
            match event {
                WindowEvent::CloseRequested => {
                    if (self.client.settings.quit_shortcuts
                        || !sunrise_client::input::is_quit_shortcut_down(
                            self.client.modifiers,
                            &self.client.physical_keys,
                        ))
                        && let Err(error) = self.begin_closing_save(true)
                    {
                        self.common.error = Some(error);
                        event_loop.exit();
                    }
                }
                WindowEvent::Focused(focused) => {
                    self.client.focused = *focused;
                    if !*focused {
                        self.capture(false);
                    }
                }
                WindowEvent::Resized(size) => {
                    if let Some(r) = &mut self.client.renderer.0 {
                        r.resize(size.width, size.height);
                    }
                }
                WindowEvent::RedrawRequested => {
                    if let Err(error) = self.render_closing_save() {
                        self.common.error = Some(error);
                        event_loop.exit();
                    }
                }
                _ => {}
            }
            return true;
        }
        false
    }
    pub fn client_process_level_events(&mut self) -> Result<()> {
        let events = std::mem::take(&mut self.common.level_events);
        for event in events {
            match event {
                LevelEvent::LevelEvent {
                    id: 2001,
                    pos,
                    data,
                    ..
                } => {
                    // LevelEventHandler plays the block's break sound before asking for debris.
                    let state = BlockStateId(data as u32);
                    let p = [pos.x, pos.y, pos.z];
                    self.play_break_sound(p, state);
                    self.destroy_particles(p, state);
                }
                LevelEvent::LevelEvent {
                    id: 3006,
                    pos,
                    data,
                    ..
                } => self.sculk_charge_event(pos, data),
                LevelEvent::LevelEvent { id: 3007, pos, .. } => self.shriek_event(pos),
                // LevelEventHandler 1010 and 1011: a jukebox's song starts
                // (`playJukeboxSong`) or stops, and the parrots nearby dance or stop.
                LevelEvent::LevelEvent {
                    id: 1010,
                    pos,
                    data,
                    ..
                } => self.jukebox_song_event([pos.x, pos.y, pos.z], data),
                LevelEvent::LevelEvent { id: 1011, pos, .. } => {
                    self.jukebox_stop_event([pos.x, pos.y, pos.z])
                }
                // LevelEventHandler 2019 and 2020: the crack on a face of the
                // block being mined, with its hit sound for 2020.
                LevelEvent::LevelEvent {
                    id: id @ (2019 | 2020),
                    pos,
                    data,
                    ..
                } => {
                    let face = sunrise_core::geometry::Direction::ALL[data.clamp(0, 5) as usize];
                    self.breaking_block_effects([pos.x, pos.y, pos.z], face.normal(), id == 2020);
                }
                LevelEvent::LevelEvent {
                    id: 3008,
                    pos,
                    data,
                    ..
                } => self.brush_complete_event([pos.x, pos.y, pos.z], data),
                LevelEvent::LevelEvent { id: 2004, pos, .. } => {
                    self.spawner_burst([pos.x, pos.y, pos.z])
                }
                // LevelEventHandler 1505: the growth particles, then the sound.
                LevelEvent::LevelEvent {
                    id: 1505,
                    pos,
                    data,
                    ..
                } => {
                    self.growth_particles(pos, data);
                    let at = [pos.x, pos.y, pos.z].map(|v| f64::from(v) + 0.5);
                    self.play_audio("minecraft:item.bone_meal.use", Some(at), 1.0, 1.0);
                }
                LevelEvent::LevelEvent {
                    id: 2015,
                    pos,
                    data,
                    ..
                } => self.dragon_egg_particles([pos.x, pos.y, pos.z], data),
                LevelEvent::LevelEvent {
                    id: 2016,
                    pos,
                    data,
                    ..
                } => self.shulker_teleport_particles([pos.x, pos.y, pos.z], data),
                LevelEvent::LevelEvent {
                    id: 2017,
                    pos,
                    data,
                    ..
                } => self.consume_teleport_particles([pos.x, pos.y, pos.z], data),
                LevelEvent::LevelEvent {
                    id: 2018,
                    pos,
                    data,
                    ..
                } => self.enderman_teleport_particles([pos.x, pos.y, pos.z], data),
                LevelEvent::LevelEvent { id: 2003, pos, .. } => {
                    self.eye_break_particles([pos.x, pos.y, pos.z]);
                }
                LevelEvent::LevelEvent {
                    id: 2013,
                    pos,
                    data,
                    ..
                } => self.smash_dust(pos, data),
                // LevelEventHandler 2002 and 2007: a potion's splash.
                LevelEvent::LevelEvent {
                    id: 2002,
                    pos,
                    data,
                    ..
                } => self.potion_splash_particles(pos, data, false),
                LevelEvent::LevelEvent {
                    id: 2007,
                    pos,
                    data,
                    ..
                } => self.potion_splash_particles(pos, data, true),
                LevelEvent::LevelEvent {
                    id: id @ 3002..=3005,
                    pos,
                    data,
                    ..
                } => self.block_particle_event(id, pos, data),
                LevelEvent::LevelEvent {
                    id: 1500,
                    pos,
                    data,
                    ..
                } => self.composter_fill_event(pos, data == 1),
                // LevelEventHandler 1504: the stalactite's drip particle.
                LevelEvent::LevelEvent { id: 1504, pos, .. } => self.dripstone_drip_particle(pos),
                // LevelEventHandler 2006: the dragon fireball's burst.
                LevelEvent::LevelEvent {
                    id: 2006,
                    pos,
                    data,
                    ..
                } => self.dragon_fireball_burst(pos, data),
                // LevelEventHandler 2009: a wet sponge drying.
                LevelEvent::LevelEvent { id: 2009, pos, .. } => self
                    .puffs_above_block(pos, sunrise_level::particle_options::ParticleType::Cloud),
                // LevelEventHandler 2011 and 2012: a bee growing a crop, a
                // turtle egg laid.
                LevelEvent::LevelEvent {
                    id: 2011 | 2012,
                    pos,
                    data,
                    ..
                } => self.turtle_egg_particles(pos, data),
                // LevelEventHandler 2014: a straw bed destroyed, with no sound.
                LevelEvent::LevelEvent {
                    id: 2014,
                    pos,
                    data,
                    ..
                } => self.destroy_particles([pos.x, pos.y, pos.z], BlockStateId(data as u32)),
                // EnderDragonFight.spawnNewGateway: an always-visible burst
                // and a sound at the gateway for every nearby player.
                LevelEvent::LevelEvent { id: 3000, pos, .. } => {
                    self.process_level_events_end_gateway_client(pos);
                }
                // LevelEventHandler 3009: a sniffer egg cracking.
                LevelEvent::LevelEvent { id: 3009, pos, .. } => {
                    self.sniffer_egg_crack_particles(pos)
                }
                LevelEvent::LevelEvent {
                    id: id @ 3011..=3021,
                    pos,
                    data,
                    ..
                } => self.trial_level_event(id, [pos.x, pos.y, pos.z], data),
                // LevelEvent.SOUND_PORTAL_TRAVEL: LevelEventHandler.levelEvent
                // plays SimpleSoundInstance.forLocalAmbience (ambient, no
                // position). EndPortalBlock.entityInside travels through
                // TeleportTransition.PLAY_PORTAL_SOUND, which sends 1032 to
                // the traveling player. The packet's position is BlockPos.ZERO,
                // so a positional play would not be heard; at None is the
                // relative sound, and it is not captured for other players.
                LevelEvent::LevelEvent { id: 1032, .. } => {
                    self.process_level_events_portal_travel_client();
                }
                // LevelEventHandler dispenser/crafter sounds and directional smoke.
                LevelEvent::LevelEvent {
                    id: id @ (1000 | 1001 | 1002 | 1004 | 1018 | 1049 | 1050 | 1051 | 2000 | 2010),
                    pos,
                    data,
                    ..
                } => {
                    let c = [
                        f64::from(pos.x) + 0.5,
                        f64::from(pos.y) + 0.5,
                        f64::from(pos.z) + 0.5,
                    ];
                    match id {
                        1000 => {
                            self.play_audio("minecraft:block.dispenser.dispense", Some(c), 1.0, 1.0)
                        }
                        1001 => {
                            self.play_audio("minecraft:block.dispenser.fail", Some(c), 1.0, 1.2)
                        }
                        1002 => {
                            self.play_audio("minecraft:block.dispenser.launch", Some(c), 1.0, 1.2)
                        }
                        1004 => self.play_audio(
                            "minecraft:entity.firework_rocket.shoot",
                            Some(c),
                            1.0,
                            1.2,
                        ),
                        1018 => {
                            let r = self.client.particles.0.random();
                            let pitch = (r.next_float() - r.next_float()) * 0.2 + 1.0;
                            self.play_audio("minecraft:entity.blaze.shoot", Some(c), 2.0, pitch);
                        }
                        1051 => {
                            let pitch =
                                0.4 / (self.client.particles.0.random().next_float() * 0.4 + 0.8);
                            self.play_audio(
                                "minecraft:entity.wind_charge.throw",
                                Some(c),
                                0.5,
                                pitch,
                            );
                        }
                        1049 => self.play_audio("minecraft:block.crafter.craft", Some(c), 1.0, 1.0),
                        1050 => self.play_audio("minecraft:block.crafter.fail", Some(c), 1.0, 1.0),
                        2000 | 2010 => {
                            use sunrise_level::particle_options::{ParticleOptions, ParticleType};
                            let particle = if id == 2010 {
                                ParticleType::WhiteSmoke
                            } else {
                                ParticleType::Smoke
                            };
                            let s = match data {
                                0 => [0, -1, 0],
                                1 => [0, 1, 0],
                                2 => [0, 0, -1],
                                3 => [0, 0, 1],
                                4 => [-1, 0, 0],
                                _ => [1, 0, 0],
                            };
                            for _ in 0..10 {
                                let r = self.client.particles.0.random();
                                let pow = r.next_double() * 0.2 + 0.01;
                                let (sx, sy, sz) =
                                    (f64::from(s[0]), f64::from(s[1]), f64::from(s[2]));
                                let p = [
                                    f64::from(pos.x)
                                        + sx * 0.6
                                        + 0.5
                                        + sx * 0.01
                                        + (r.next_double() - 0.5) * sz * 0.5,
                                    f64::from(pos.y)
                                        + sy * 0.6
                                        + 0.5
                                        + sy * 0.01
                                        + (r.next_double() - 0.5) * sy * 0.5,
                                    f64::from(pos.z)
                                        + sz * 0.6
                                        + 0.5
                                        + sz * 0.01
                                        + (r.next_double() - 0.5) * sx * 0.5,
                                ];
                                let v = [
                                    sx * pow + r.next_gaussian() * 0.01,
                                    sy * pow + r.next_gaussian() * 0.01,
                                    sz * pow + r.next_gaussian() * 0.01,
                                ];
                                self.client.particles.0.add(
                                    &ParticleOptions::simple(particle),
                                    p,
                                    v,
                                );
                            }
                        }
                        _ => {}
                    }
                }
                LevelEvent::LevelEvent { id, pos, .. } => {
                    let c = Some([
                        f64::from(pos.x) + 0.5,
                        f64::from(pos.y) + 0.5,
                        f64::from(pos.z) + 0.5,
                    ]);
                    let sound = match id {
                        1503 => Some(("minecraft:block.end_portal_frame.fill", 1.0)),
                        1501 => Some(("minecraft:block.lava.extinguish", 0.5)),
                        1009 => Some(("minecraft:block.fire.extinguish", 0.5)),
                        1502 => Some(("minecraft:block.redstone_torch.burnout", 0.5)),
                        1033 => Some(("minecraft:block.chorus_flower.grow", 1.0)),
                        1034 => Some(("minecraft:block.chorus_flower.death", 1.0)),
                        1045 => Some(("minecraft:block.pointed_dripstone.land", 2.0)),
                        1052 => Some(("minecraft:block.sulfur_spike.land", 2.0)),
                        1046 => Some((
                            "minecraft:block.pointed_dripstone.drip_lava_into_cauldron",
                            2.0,
                        )),
                        1047 => Some((
                            "minecraft:block.pointed_dripstone.drip_water_into_cauldron",
                            2.0,
                        )),
                        // LevelEvent.SOUND_ZOMBIE_WOODEN_DOOR (attack, break).
                        1019 => Some(("minecraft:entity.zombie.attack_wooden_door", 2.0)),
                        1021 => Some(("minecraft:entity.zombie.break_wooden_door", 2.0)),
                        1026 => Some(("minecraft:entity.zombie.infect", 2.0)),
                        1027 => Some(("minecraft:entity.zombie_villager.converted", 2.0)),
                        1030 => Some(("minecraft:block.anvil.use", 1.0)),
                        1029 => Some(("minecraft:block.anvil.destroy", 1.0)),
                        1031 => Some(("minecraft:block.anvil.land", 0.3)),
                        1042 => Some(("minecraft:block.grindstone.use", 1.0)),
                        1044 => Some(("minecraft:block.smithing_table.use", 1.0)),
                        // LevelEvent.SOUND_PAGE_TURN (a lectern's page).
                        1043 => Some(("minecraft:item.book.page_turn", 1.0)),
                        // LevelEvent.SOUND_PHANTOM_BITE: a phantom's bite.
                        1039 => Some(("minecraft:entity.phantom.bite", 0.3)),
                        // LevelEvent.SOUND_SPLASH_POTION and its instant twin.
                        1053 | 1054 => Some(("minecraft:entity.splash_potion.break", 1.0)),
                        _ => None,
                    };
                    if let Some((name, volume)) = sound {
                        self.process_level_events_sound_client(id, c, name, volume);
                    }
                    // LevelEventHandler draws smoke after the sound.
                    match id {
                        1501 => self.puffs_above_block(
                            pos,
                            sunrise_level::particle_options::ParticleType::LargeSmoke,
                        ),
                        1502 => self.torch_burnout_smoke(pos),
                        1503 => self.end_portal_frame_smoke(pos),
                        _ => {}
                    }
                }
                LevelEvent::BlockEntityEvent { pos, a, b, .. } => {
                    self.block_entity_event([pos.x, pos.y, pos.z], a, b)
                }
                // NoteBlock.triggerEvent: a NOTE particle above, coloured by
                // the note (`note / 24.0` in the x speed).
                LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "note_particle",
                } => {
                    use sunrise_level::particle_options::{ParticleOptions, ParticleType};
                    let note = self
                        .common
                        .pack
                        .assets
                        .registry
                        .state(state)
                        .and_then(|s| s.properties.get("note")?.parse::<f64>().ok())
                        .unwrap_or(0.0);
                    self.client.particles.0.add(
                        &ParticleOptions::simple(ParticleType::Note),
                        [
                            f64::from(pos.x) + 0.5,
                            f64::from(pos.y) + 1.2,
                            f64::from(pos.z) + 0.5,
                        ],
                        [note / 24.0, 0.0, 0.0],
                    );
                }
                LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "candle_smoke",
                } => self.candle_smoke([pos.x, pos.y, pos.z], state),
                // The block at `pos` has just replaced `state`: Block.pushEntitiesUp.
                LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "push_entities_up",
                } => {
                    self.client_push_entities_up(
                        pos,
                        state,
                        self.common.world.block(pos.x, pos.y, pos.z),
                    );
                }
                // Client piston replay emits notification-only GameEvents (block_destroy).
                LevelEvent::GameEvent { .. }
                | LevelEvent::Sound { .. }
                | LevelEvent::DropResources { .. }
                | LevelEvent::BlockEvent {
                    kind: "note_custom_sound",
                    ..
                } => {}
                LevelEvent::ContainerLootTable { .. }
                | LevelEvent::FallingBlock { .. }
                | LevelEvent::Player { .. }
                | LevelEvent::PrimeTnt { .. }
                | LevelEvent::BlockEvent {
                    kind:
                        "dispense" | "golem" | "command_block" | "structure_block" | "test_block"
                        | "test_block_tick" | "brushable" | "hatch" | "hatch_sniffer" | "hatch_turtles"
                        | "portal_spawn" | "spawn_ghastling" | "shrieker_respond",
                    ..
                }
                | LevelEvent::GrowTree { .. }
                | LevelEvent::PlaceFeature { .. }
                | LevelEvent::BlockEvent { .. } => {}
            }
        }
        Ok(())
    }
    /// Client-only `Block.pushEntitiesUp` for piston/block replay without server guest tracking.
    pub(in crate::game::client_side) fn client_push_entities_up(
        &mut self,
        pos: BlockPos,
        old: BlockStateId,
        new: BlockStateId,
    ) {
        use sunrise_core::geometry::{Axis, ShapeOp};
        use sunrise_level::collisions::SceneCollisions;

        let at = [pos.x, pos.y, pos.z];
        let shape = |state| {
            let mut scene = SceneCollisions {
                world: &self.common.world,
                binding: self.common.collision,
                subject: None,
            };
            self.common
                .collision
                .collision_shape(&mut scene, state, at)
                .ok()
        };
        let (Some(old_shape), Some(new_shape)) = (shape(old), shape(new)) else {
            return;
        };
        let Ok(added) = old_shape.join_unoptimized(&new_shape, ShapeOp::ONLY_SECOND) else {
            return;
        };
        let added = added.moved(Vec3::new(
            f64::from(pos.x),
            f64::from(pos.y),
            f64::from(pos.z),
        ));
        let Some(bounds) = added.bounds() else {
            return;
        };
        let lift_dy =
            |body: Aabb| 1.0 + added.collide(Axis::Y, body.move_by(Vec3::new(0.0, 1.0, 0.0)), -1.0);
        if let Some(p) = self.common.player.as_mut().filter(|p| !p.spectator) {
            let body = p.bounds();
            if body.intersects(bounds) {
                p.position.y += lift_dy(body);
            }
        }
        for m in &mut self.common.mobs {
            if !m.removed {
                let body = m.bounds();
                if body.intersects(bounds) {
                    m.position.y += lift_dy(body);
                }
            }
        }
        for i in 0..self.common.dropped.len() {
            let body = self.common.dropped[i].bounds();
            if body.intersects(bounds) {
                let dy = lift_dy(body);
                self.common.dropped.update(i, |item| item.position.y += dy);
            }
        }
        for i in 0..self.common.boats.len() {
            let body = self.common.boats[i].bounds();
            if body.intersects(bounds) {
                let dy = lift_dy(body);
                self.common.boats.update(i, |boat| boat.position.y += dy);
            }
        }
        for i in 0..self.common.carts.len() {
            let body = self.common.carts[i].bounds();
            if body.intersects(bounds) {
                let dy = lift_dy(body);
                self.common.carts.update(i, |cart| cart.position.y += dy);
            }
        }
        for i in 0..self.common.armor_stands.len() {
            let body = self.common.armor_stands[i].bounds();
            if body.intersects(bounds) {
                let dy = lift_dy(body);
                self.common
                    .armor_stands
                    .update(i, |stand| stand.position.y += dy);
            }
        }
    }
    #[inline(always)]
    pub(in crate::game) fn process_level_events_end_gateway_client(&mut self, pos: BlockPos) {
        let at = [
            f64::from(pos.x) + 0.5,
            f64::from(pos.y) + 0.5,
            f64::from(pos.z) + 0.5,
        ];
        self.client.particles.0.add_with(
            None,
            &sunrise_level::particle_options::ParticleOptions::simple(
                sunrise_level::particle_options::ParticleType::ExplosionEmitter,
            ),
            at,
            [0.0; 3],
            true,
            true,
        );
        let random = self.client.particles.0.level_random();
        let pitch = (1.0 + (random.next_float() - random.next_float()) * 0.2) * 0.7;
        self.play_audio_unshared("minecraft:block.end_gateway.spawn", Some(at), 10.0, pitch);
    }
    #[inline(always)]
    pub(in crate::game) fn process_level_events_portal_travel_client(&mut self) {
        let pitch = self.client.particles.0.level_random().next_float() * 0.4 + 0.8;
        self.play_audio_on(
            sunrise_game_data::sound_source::AMBIENT,
            "minecraft:block.portal.travel",
            None,
            0.25,
            pitch,
        );
    }
    #[inline(always)]
    pub(in crate::game) fn process_level_events_sound_client(
        &mut self,
        id: i32,
        c: Option<[f64; 3]>,
        name: &str,
        volume: f32,
    ) {
        let pitch = match id {
            1009 => {
                2.6 + (self.common.world.random.next_float()
                    - self.common.world.random.next_float())
                    * 0.8
            }
            // LevelEventHandler draws these from the client
            // level's random, before 1501's and 1502's smoke.
            1501 | 1502 => {
                let r = self.client.particles.0.level_random();
                2.6 + (r.next_float() - r.next_float()) * 0.8
            }
            1039 | 1045 | 1046 | 1047 | 1052 | 1053 | 1054 => {
                // LevelEventHandler draws these from the client
                // level's random, not the server's.
                self.client.particles.0.level_random().next_float() * 0.1 + 0.9
            }
            1029 | 1030 | 1042 | 1043 | 1044 => self.common.world.random.next_float() * 0.1 + 0.9,
            1026 | 1027 => {
                let r = self.client.particles.0.level_random();
                (r.next_float() - r.next_float()) * 0.2 + 1.0
            }
            1019 | 1021 => {
                (self.common.world.random.next_float() - self.common.world.random.next_float())
                    * 0.2
                    + 1.0
            }
            _ => 1.0,
        };
        if matches!(
            id,
            1019 | 1021
                | 1026
                | 1027
                | 1029
                | 1030
                | 1039
                | 1045
                | 1052
                | 1053
                | 1054
                | 1501
                | 1503
        ) {
            // Clients play these sounds from their level events;
            // capturing it also sends a duplicate sound packet.
            self.play_audio_unshared(name, c, volume, pitch);
        } else {
            self.play_audio(name, c, volume, pitch);
        }
    }
    pub(in crate::game::client_side) fn project_window_cursor(
        &self,
        framebuffer: [u32; 2],
    ) -> Option<sunrise_client::window::GuiPointer> {
        self.client.window_cursor.map(|position| {
            self.pointer_projection(framebuffer)
                .project_pointer(position)
        })
    }
    /// Rebuild GPU-owned state after the backend reports device loss.
    pub(in crate::game::client_side) fn recover_device(&mut self) -> Result<()> {
        if !self
            .client
            .renderer
            .0
            .as_deref()
            .is_some_and(Renderer::device_lost)
        {
            return Ok(());
        }
        let now = Instant::now();
        if self
            .client
            .last_device_recovery
            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(60))
        {
            return Err("the graphics device was lost again after recovery".into());
        }
        self.client.last_device_recovery = Some(now);
        let window = self
            .client
            .window
            .0
            .clone()
            .ok_or("no window for graphics recovery")?;
        log::warn!("Sunrise: rebuilding the renderer after graphics device loss");
        // Drop the old surface and device before opening another on this window.
        let mut old = self
            .client
            .renderer
            .0
            .take()
            .ok_or("no renderer for graphics recovery")?;
        // A GPU-culled frame may be what lost it: the renderers after this one don't
        // offer GPU culling, until Sunrise restarts (`graphics::lose_gpu_culling`).
        if old.gpu_frame() {
            log::warn!(
                "Sunrise: GPU culling off until the next start: the graphics device was lost in one of its frames"
            );
            sunrise_client::graphics::lose_gpu_culling();
        }
        let (screenshot, screenshot_result) = (old.screenshot.take(), old.screenshot_result.take());
        let display = old.display_reading();
        self.client.mod_gpu = None;
        drop(old);
        let device = pollster::block_on(sunrise_client::gfx::Opened::for_window(
            &window,
            self.common.options.offscreen,
            self.client.settings.sunrise.api,
            self.client.client_pack.render.texture_layers,
        ))?;
        if self.client.gui.stitch_for_device(
            &self.common.pack.source,
            &self.common.vanilla_root(),
            device.max_atlas_size(),
        )? {
            self.client.particles.0.reload(
                &self.common.pack.source,
                &self.client.gui.atlas.sprites,
                &self.client.gui.atlas.translucent,
            );
            self.client.standalone_geometry = native_block_entity_draw::StandaloneGeometry::new(
                &self.common.pack,
                &self.client.client_pack,
                &self.client.gui,
                Some(self.client_res().block_entity_render()),
            );
            sunrise_client::ui_theme::reset_atlas();
            self.native_texture_atlas_stitched(mod_api::TextureAtlas::Gui);
        }
        let (mut renderer, clouds) = build_renderer(
            window,
            self.common.options.offscreen,
            &self.common.pack,
            &self.client.client_pack,
            &self.client.gui,
            sunrise_client::gfx::RendererDevice::Opened(Box::new(device)),
            None,
            self.client.settings.sunrise.api,
            self.client.settings.sunrise.translucent_batching,
            self.effective_vsync(),
            &self.client.settings.sunrise,
            self.picked_pack_shape(),
            display,
            self.client.settings.texture_atlas_settings(),
        )?;
        clouds?;
        (renderer.screenshot, renderer.screenshot_result) = (screenshot, screenshot_result);
        self.adopt_renderer(Box::new(renderer));
        log::info!("Sunrise: graphics device recovered; rebuilding world meshes");
        Ok(())
    }
    pub(in crate::game::client_side) fn refresh_window_cursor(&mut self) {
        if let Some(position) = self.client.window_cursor
            && let Some(projection) = self.renderer_pointer_projection()
        {
            self.client.cursor = projection.project_pointer(position).subpixel();
        }
    }
    pub(in crate::game::client_side) fn render(&mut self) -> Result<bool> {
        if self.client.screenshot_sequence.0.is_some() {
            return self.render_sequence_frame();
        }
        self.render_frame()
    }
    /// Render the frame, then its two moments after that: `render_frame_post` whatever
    /// became of it, `flip_frame` once it was presented.
    pub fn render_and_flip(&mut self, elapsed: Duration) -> Result<bool> {
        let rendered = match self.render() {
            // The callback can fire during this frame; rebuild on
            // the next redraw rather than exiting on its error.
            Err(_)
                if self
                    .client
                    .renderer
                    .0
                    .as_deref()
                    .is_some_and(Renderer::device_lost) =>
            {
                false
            }
            result => result?,
        };
        self.finish_render_frame(elapsed);
        if rendered {
            // The submit thread may still hold the frame: a listener hears it after the
            // present, and with none the next frame keeps overlapping it.
            if mod_api::__private::listened(mod_api::HookId::FlipFrame)
                && let Some(renderer) = &mut self.client.renderer.0
            {
                renderer.wait_submissions();
                if renderer.device_lost() {
                    return Ok(false);
                }
            }
            self.native_flip_frame();
        }
        Ok(rendered)
    }
    /// Build the complete frame before handing its borrowed geometry and lighting to
    /// the consumer. Tests use the same boundary as GPU submission.
    pub fn render_with(
        &mut self,
        consume: impl FnOnce(&mut Renderer, &Frame<'_>) -> Result<bool>,
    ) -> Result<bool> {
        self.render_with_hooks(|renderer, frame, _| consume(renderer, frame))
    }

    fn render_with_hooks(
        &mut self,
        consume: impl FnOnce(
            &mut Renderer,
            &Frame<'_>,
            Option<&mut dyn sunrise_client::gfx::world_primitives::FrameHooks>,
        ) -> Result<bool>,
    ) -> Result<bool> {
        self.begin_frame_draws();
        self.extract_arm_poses();
        let prepare = spans::guard("prepare");
        let mut pr = spans::laps();
        // The window's GPU holds the atlas: its copy in memory is kept
        // compressed (a no-op once it is).
        if self.client.renderer.0.is_some() {
            self.client.gui.atlas.pack_pixels();
            self.client.client_pack.render.pixels.pack();
        }
        // Before any text of this frame: a full glyph region starts over.
        self.client.gui.atlas.font.begin_frame();
        if self.common.clock.still() {
            // Obfuscated text picks the same glyphs in a frame of the same world tick.
            self.client
                .gui
                .atlas
                .font
                .reseed(self.visual_ticks() as i64);
        }
        self.client.glint_frame = self.capture_glint_frame();
        if self
            .client
            .renderer
            .0
            .as_mut()
            .is_some_and(|r| r.display_changed())
        {
            self.apply_ui_settings();
        }
        self.follow_frame();
        self.observe_screen();
        // `Gui.setScreen` clears the focus of the screen it leaves: a menu
        // that comes back starts with no widget highlighted.
        self.drop_left_screen_focus();
        // Minecraft.doWorldLoad: the LevelLoadingScreen while single player's
        // server starts, with the server's chunk map and progress.
        if let Some(starting) = &self.client.integrated_starting {
            // WorldOpenFlows.upgradeAndOpenWorld: the server's file fixers run on
            // FileFixerProgressScreen, before the LevelLoadingScreen has progress.
            let upgrade = starting.handle.upgrade();
            if upgrade.is_file_fixing() {
                sunrise_client::widgets::begin_screen(|| {
                    sunrise_client::widgets::ScreenInfo::new(
                        "file_fixer",
                        screen_info::tr("upgradeWorld.title", &[]),
                    )
                });
                let cursor = self
                    .client
                    .renderer
                    .0
                    .as_ref()
                    .and_then(|r| self.project_window_cursor([r.config.width, r.config.height]))
                    .map_or(self.client.cursor, |p| p.pixel());
                if let Some(renderer) = &mut self.client.renderer.0 {
                    let size = [renderer.config.width, renderer.config.height];
                    let mut hud = HudBatch::new(size, self.client.settings.gui_scale_inputs());
                    sunrise_client::level_loading::draw_file_fixer(
                        &mut hud,
                        &self.client.gui.atlas,
                        upgrade,
                        cursor,
                    );
                    pr.lap("pr_loading");
                    drop(prepare);
                    return renderer.render(&loading_frame(&hud), None);
                }
                return Ok(false);
            }
            // The server's progress once it has some (`hasProgress`).
            let progress = Some(starting.handle.load_view())
                .filter(|v| v.has_progress())
                .map(|v| v.server_progress());
            sunrise_client::widgets::begin_screen(|| {
                screen_info::level_loading(progress, &screen_info::tr)
            });
            if let Some(renderer) = &mut self.client.renderer.0 {
                let mut hud = HudBatch::new(
                    [renderer.config.width, renderer.config.height],
                    self.client.settings.gui_scale_inputs(),
                );
                let (w, h) = (hud.width, hud.height);
                sunrise_client::ui::menu_background(
                    &mut hud,
                    &self.client.gui.atlas,
                    0.0,
                    0.0,
                    w,
                    h,
                    false,
                );
                // The chunk map and progress are the server thread's.
                let view = self
                    .client
                    .integrated_starting
                    .as_ref()
                    .map(|s| s.handle.load_view().clone());
                let status = |x: i32, z: i32| view.as_ref().and_then(|v| v.status(x, z));
                let progress = view
                    .as_ref()
                    .is_some_and(|v| v.has_progress())
                    .then_some(self.client.loading_progress);
                sunrise_client::level_loading::draw(
                    &mut hud,
                    &self.client.gui.atlas,
                    Some(&status),
                    progress,
                );
                pr.lap("pr_loading");
                drop(prepare);
                return renderer.render(&loading_frame(&hud), None);
            }
            return Ok(false);
        }
        if self.client.pending_entry.0.is_some() {
            if let (Some(renderer), Some(loader)) =
                (&mut self.client.renderer.0, &self.common.rest_loader)
            {
                sunrise_client::widgets::begin_screen(|| {
                    screen_info::level_loading(Some(loader.progress.fraction()), &screen_info::tr)
                });
                let mut hud = HudBatch::new(
                    [renderer.config.width, renderer.config.height],
                    self.client.settings.gui_scale_inputs(),
                );
                sunrise_client::level_loading::draw_progress(
                    &mut hud,
                    &self.client.gui.atlas,
                    loader.progress.line(),
                    loader.progress.fraction(),
                );
                pr.lap("pr_loading");
                drop(prepare);
                return renderer.render(&loading_frame(&hud), None);
            }
            return Ok(false);
        }
        let Some(size) = self
            .client
            .renderer
            .0
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return Ok(false);
        };
        // An opaque LoadingOverlay needs no world or screen behind it; the
        // ToastManager still draws in front of it.
        let reload_overlay = self.reload_overlay_frame();
        if let Some((bar, fade)) = reload_overlay.filter(|(_, fade)| fade.opaque()) {
            let mut hud = HudBatch::new(size, self.client.settings.gui_scale_inputs());
            self.draw_reload_overlay(&mut hud, bar, fade);
            self.draw_every_toast(&mut hud);
            self.client.client_pack.item_renderer.end_gui_frame();
            pr.lap("pr_loading");
            drop(prepare);
            return self
                .client
                .renderer
                .0
                .as_mut()
                .expect("renderer sized above")
                .render(&loading_frame(&hud), None);
        }
        pr.lap("pr_setup");
        let alpha = self.frame_world_partial_tick();
        self.refresh_client_pick(alpha as f32);
        // Camera.getCameraEntityPartialTicks: only players and their mounts stay unfrozen.
        let camera_alpha = self.camera_partial_tick();
        let view = self.frame_camera(camera_alpha as f32);
        self.check_screenshot_sequence_camera(view);
        self.client.posed_rotation = view.map(|(_, yaw, pitch, roll)| (yaw, pitch, roll));
        let eye = view.map(|_| {
            (
                glam::DVec3::from_array(self.client.view.eye),
                self.client.view.eye_rotation,
            )
        });
        let (camera, underwater, view_yaw, view_pitch, view_roll) = match view {
            Some((c, yaw, pitch, roll)) => {
                let s = self.common.world.block(
                    c.x.floor() as i32,
                    c.y.floor() as i32,
                    c.z.floor() as i32,
                );
                (
                    c,
                    self.common.res.facts().get(s).fluid == Fluid::Water,
                    yaw,
                    pitch,
                    roll,
                )
            }
            None => (
                glam::DVec3::new(0.5, 100.0, 0.5),
                false,
                self.common.yaw,
                self.common.pitch,
                0.0,
            ),
        };
        pr.lap("pr_camera");
        let (forward, up) = if view.is_some() {
            (self.client.view.forwards, self.client.view.up)
        } else {
            let q = sunrise_client::camera::rotation_with_roll(view_yaw, view_pitch, view_roll);
            (q.transform([0.0, 0.0, -1.0]), q.transform([0.0, 1.0, 0.0]))
        };
        self.client
            .audio
            .0
            .set_listener(camera.to_array(), forward, up);
        self.follow_audio_stream_entities();
        pr.lap("pr_listener");
        let block = [
            sunrise_core::mth::floor(camera.x),
            sunrise_core::mth::floor(camera.y),
            sunrise_core::mth::floor(camera.z),
        ];
        // Camera.extractRenderState: only spectator inside a solidRender block disables smartCull.
        let cave_culling = self.client.settings.sunrise.cave_culling
            && !(self.common.spectator()
                && self
                    .common
                    .res
                    .facts()
                    .get(self.common.world.block(block[0], block[1], block[2]))
                    .solid_render);
        // The bounded section graph runs on its worker while the frame is prepared, even when
        // smart culling is off (`SectionOcclusionGraph.runUpdates`).
        let effective_render_distance = self.effective_render_distance();
        let effective_detail_distance = (effective_render_distance * 16) as f32;
        if self.client.title.0.is_none()
            && let Some(renderer) = self.client.renderer.0.as_mut()
        {
            renderer.sync_loaded_columns(&self.common.world.chunks);
            sunrise_level::spans::time("occlusion", || {
                renderer.start_occlusion(camera, effective_detail_distance, cave_culling)
            });
        }
        pr.lap("pr_occlusion");
        let biome = self
            .common
            .world
            .chunks
            .get(&(block[0] >> 4, block[2] >> 4))
            .map(|c| c.biome(block[0], block[1], block[2]))
            .unwrap_or(0);
        // Asked before the environment's borrow: the renderer's pack shaders answer.
        let pack_expressions = self.reads_pack_expressions();
        // What the packs' expressions read, taken for the frame and put back after the post
        // chains: its strings are rewritten in place, so it allocates nothing once they have
        // grown to the dimension's and biome's names.
        let mut frame = std::mem::take(&mut self.client.expression_frame);
        let environment = self.view_environment();
        // EnvironmentAttributeProbe: the spatially interpolated biome colours,
        // Gaussian-weighted over the noise biomes around the camera (plains
        // where no chunk is loaded, as ClientLevel.getUncachedNoiseBiome).
        let biomes = world_biomes!(self);
        let weights = {
            let chunks = &self.common.world.chunks;
            let plains = biomes.id("minecraft:plains");
            sunrise_level::biome::gaussian_weights(camera.to_array(), |qx, qy, qz| {
                chunks
                    .get(&(qx >> 2, qz >> 2))
                    .map_or(plains, |c| c.biome(qx * 4, qy * 4, qz * 4))
            })
        };
        // visual/water_fog_color: no dimension sets it, its default is #050533.
        let water_fog = sunrise_level::biome::blend_rgb(&weights, |b| {
            sunrise_level::biome::rgb(biomes.get(b).water_fog)
        });
        // The attribute stack, with the camera's biome mix in the probe's order; the biome
        // maps as the packs left them.
        let attributes_of = |b: u16| {
            let info = biomes.get(b);
            environment.biome(&info.name).unwrap_or(&info.attributes)
        };
        let mut blend = sunrise_world::environment_system::BiomeBlend::default();
        for &(b, weight) in &weights {
            blend.accumulate(weight, attributes_of(b));
        }
        // The packs' biome post-process group, mixed the same way where they set one.
        let empty = sunrise_world::environment_attributes::AttributeMap::default();
        let post_of = |b: u16| {
            environment
                .biome_post(&biomes.get(b).name)
                .unwrap_or(&empty)
        };
        let mut post_blend = sunrise_world::environment_system::BiomeBlend::default();
        if environment.has_biome_post() {
            for &(b, weight) in &weights {
                post_blend.accumulate(weight, post_of(b));
            }
        }
        let (rain, thunder) = self.view_weather(alpha as f32);
        // What the packs' attribute expressions read (only a pack's stack has any).
        let mut polytone_frame = None;
        if self.client.client_pack.environments.is_some()
            || self.client.client_pack.polytone_particles.is_some()
            || self.client.client_pack.polytone_post.is_some()
            || pack_expressions
        {
            let light = self.common.world.light(block[0], block[1], block[2]);
            let info = biomes.get(biome);
            // Every field named, so a new one has to be written here too.
            let sunrise_level::environment::Frame {
                rain: frame_rain,
                thunder: frame_thunder,
                game_time,
                day_time,
                render_distance,
                sky_type,
                dimension,
                player,
                player_velocity,
                player_on_ground,
                in_water,
                camera: frame_camera,
                sky_light,
                block_light,
                biome: biome_name,
                temperature,
                downfall,
            } = &mut frame;
            dimension.clear();
            dimension.push_str(self.common.dim.id());
            biome_name.clone_from(&info.name);
            (*temperature, *downfall) = (info.temperature, info.downfall);
            (*frame_rain, *frame_thunder) = (rain, thunder);
            (*game_time, *day_time) = (self.common.world.game_time, self.view_day_time());
            *render_distance = effective_render_distance;
            *sky_type = match self.dimension_type().skybox {
                sunrise_world::dimension_type::Skybox::Overworld => 1,
                sunrise_world::dimension_type::Skybox::End => 2,
                sunrise_world::dimension_type::Skybox::None => 0,
            };
            *player = self.common.player.as_ref().map_or(camera.to_array(), |p| {
                [p.position.x, p.position.y, p.position.z]
            });
            *player_velocity = self
                .common
                .player
                .as_ref()
                .map_or([0.0; 3], |p| [p.velocity.x, p.velocity.y, p.velocity.z]);
            *player_on_ground = self
                .common
                .player
                .as_ref()
                .is_some_and(|p| p.ground.on_ground);
            *in_water = underwater;
            *frame_camera = camera.to_array();
            (*sky_light, *block_light) = (i32::from(light >> 4), i32::from(light & 15));
            if let Some(particles) = &self.client.client_pack.polytone_particles
                && let Ok(mut f) = particles.frame.write()
            {
                f.clone_from(&frame);
            }
            environment.set_frame(&frame);
            polytone_frame = Some(&frame);
        }
        let fixed_time = self
            .client
            .settings
            .extras
            .pvp
            .tweaks
            .time
            .filter(|_| self.own_view())
            .map(|_| self.view_day_time());
        let sample_clock = |clock: &str| {
            fixed_time.unwrap_or_else(|| {
                self.client.remote.as_ref().map_or_else(
                    || self.common.command_clock_time(clock),
                    |remote| remote.named_clock_time(clock),
                )
            })
        };
        let mut sky = environment.sample_with_clock(
            &sunrise_level::environment::Sample {
                day_time: self.view_day_time(),
                rain,
                thunder,
                blend: Some(&blend),
                biome: Some(attributes_of(biome)),
                post_blend: Some(&post_blend),
                post_biome: Some(post_of(biome)),
            },
            &sample_clock,
        );
        self.end_flash_sky(&mut sky, alpha);
        if self.common.flash > 0 && !self.client.settings.file.bool("hideLightningFlashes") {
            // ClientLevel's flash layer while skyFlashTime > 0: the sky colour
            // moves 22% of the way to (0.8, 0.8, 1.0).
            let flash = [0.8, 0.8, 1.0];
            sky.sky_color =
                std::array::from_fn(|i| sky.sky_color[i] + (flash[i] - sky.sky_color[i]) * 0.22);
            sky.sky_light_factor = 1.0;
        }
        pr.lap("pr_environment");
        // FishingHookRenderer's line, then DebugRenderer: chunk borders and
        // hitboxes (F3+G, F3+B).
        let mut lines = self.fishing_line(alpha, size);
        let mut g = self.debug_gizmos(alpha as f32, camera.to_array());
        self.studio_camera_path(&mut g);
        lines.extend(g.lines);
        let on_top = g.on_top;
        pr.lap("pr_debug");
        let visuals = if self.client.title.0.is_none() {
            self.native_frame_visuals(&mut lines)
        } else {
            native_visuals::FrameVisuals::default()
        };
        pr.lap("pr_native_visuals");
        let view = if view_roll == 0.0 {
            sunrise_client::camera::view_matrix(view_yaw, view_pitch)
        } else {
            glam::Mat4::from_quat(self.camera_orientation().conjugate())
        };
        let at = camera.to_array();
        // GameRenderer.render3dHud: the debug crosshair, first person with the HUD shown.
        let mut crosshair = sunrise_client::gizmos::Gizmos::default();
        if debug_crosshair_drawn(
            self.debug_crosshair(),
            self.client.camera_type.is_first_person(),
            self.client.chat_ui.hide_gui,
            self.shows_panorama(),
            self.overlay_holds_view(),
        ) {
            let scale = HudBatch::new(size, self.client.settings.gui_scale_inputs()).scale;
            crosshair.debug_crosshair(at, view_pitch, view_yaw, scale);
        }
        let debug_lines = sunrise_client::gizmos::camera_relative(&lines, at, view);
        let debug_lines_on_top = sunrise_client::gizmos::camera_relative(&on_top, at, view);
        let debug_crosshair = sunrise_client::gizmos::camera_relative(&crosshair.on_top, at, view);
        // LevelExtractor.extractBlockOutline: model translucency controls draw order;
        // no outline when an entity is targeted (Minecraft.hitResult an EntityHitResult).
        let mut outline_after_terrain = false;
        // GameRenderer.shouldRenderBlockOutline: not with the HUD hidden (F1), and only for a
        // player who may build. Adventure players only get one with a CAN_BREAK or
        // CAN_PLACE_ON item matching the block, and spectators with a menu block; Sunrise
        // does not open menus for spectators, so those never show it. An open
        // screen does not hide it: vanilla checks the HUD and the game mode, not the screen.
        let outline: Vec<_> = self
            .block_outline_hit()
            .filter(|h| {
                let state = self.common.world.block(h.pos[0], h.pos[1], h.pos[2]);
                let mut canceled = false;
                self.native_block_highlight(h.pos, state, alpha as f32, &mut canceled);
                !canceled
            })
            .map(|h| {
                let state = self.common.world.block(h.pos[0], h.pos[1], h.pos[2]);
                outline_after_terrain = self
                    .client
                    .client_pack
                    .render
                    .model_catalog
                    .collect(state, BlockPos::new(h.pos[0], h.pos[1], h.pos[2]))
                    .is_ok_and(|models| {
                        models
                            .iter()
                            .any(|(_, model)| model.faces.iter().any(|f| f.translucent))
                    });
                self.outline_edges_iter(state, h.pos).collect()
            })
            .unwrap_or_default();
        pr.lap("pr_outline");
        let title = self.shows_panorama();
        let (yaw, pitch, roll) = if title {
            // PanoramaRenderer: the title cube spins at 0.1 degrees per tick, pitched 10 degrees.
            (
                self.client.panorama_rotation.update(
                    self.client.started.elapsed().as_secs_f32(),
                    self.client.settings.file.double("panoramaScrollSpeed") as f32,
                ),
                10.0,
                0.0,
            )
        } else {
            (view_yaw, view_pitch, view_roll)
        };
        let frame_view_matrix = if title {
            sunrise_client::camera::view_matrix(yaw, pitch)
        } else {
            view
        };
        let fov_degrees = if title {
            85.0
        } else {
            self.fov_degrees(camera_alpha as f32, camera.to_array())
        };
        let cull_fov_degrees = if title {
            85.0
        } else {
            sunrise_client::gfx::cull_fov(fov_degrees, (self.client.settings.fov as i32) as f32)
        };
        let camera_effects = fov::camera_effects_enabled(title, self.client.studio.flying());
        let bob = if !camera_effects {
            glam::Mat4::IDENTITY
        } else {
            self.bob_matrix(camera_alpha as f32)
        };
        let spin = if !camera_effects {
            glam::Mat4::IDENTITY
        } else {
            self.spinning_matrix(alpha as f32)
        };
        let terrain_distance = self.terrain_distance();
        let distance = (terrain_distance * 16) as f32;
        let depth_far =
            sunrise_client::camera::depth_far(terrain_distance, self.client.settings.cloud_range);
        let mods_read_camera = !mod_api::__private::mods().is_empty();
        let view_transform = if camera_effects
            && self.common.player.is_some()
            && mod_api::__private::listened(mod_api::HookId::RenderViewTransform)
            && let Some(renderer) = &self.client.renderer.0
        {
            let usable = renderer.view_transform_validator(
                [yaw, pitch, roll],
                fov_degrees,
                depth_far,
                bob,
                spin,
            );
            mod_api::ViewTransform::with_frame(usable, |transform| {
                self.native_render_view_transform(camera_alpha as f32, transform);
                glam::Mat4::from_cols_array(&transform.matrix())
            })
        } else {
            glam::Mat4::IDENTITY
        };
        let posed_bob = bob * view_transform;
        self.client.screen_blur_requested.set(false);
        self.client.native_camera_entity_eye =
            (!title && self.camera_subject().is_some()).then_some(self.client.view.eye);
        let previous_camera = self.client.native_camera_frame.take();
        self.client.native_camera_frame = (mods_read_camera && !title).then(|| {
            let mut dimension =
                previous_camera.map_or_else(String::new, |snapshot| snapshot.dimension);
            dimension.clear();
            dimension.push_str(self.common.dim.id());
            let partial = camera_alpha as f32;
            mod_api::Camera {
                position: camera.to_array(),
                eye_height: self.client.view.eye_height_old
                    + (self.client.view.eye_height - self.client.view.eye_height_old) * partial,
                partial_tick: partial,
                yaw,
                pitch,
                roll,
                fov_degrees,
                near: 0.05,
                far: depth_far,
                view: (posed_bob * spin * frame_view_matrix).to_cols_array(),
                projection: glam::Mat4::perspective_rh(
                    fov_degrees.to_radians(),
                    size[0] as f32 / size[1].max(1) as f32,
                    depth_far,
                    0.05,
                )
                .to_cols_array(),
                dimension,
            }
        });
        let _projection_frame = (!title).then(|| self.activate_projection_frame());
        pr.lap("pr_projection");
        let vanilla_skin = self.apply_player_skin_override(title);
        // GUI hooks project against this frame's camera, including its bob/spin and FOV.
        let mut hud = sunrise_level::spans::time("hud", || {
            self.extract_frame_hud(
                size,
                reload_overlay,
                title,
                camera.to_array(),
                view_yaw,
                view_pitch,
                fov_degrees,
            )
        });
        if reload_overlay.is_none() {
            self.flush_ime(&mut hud);
        }
        self.resolve_inline_profiles(&hud.inline_profiles);
        pr.lap("pr_hud");
        // The packs' Polytone post chains, after vanilla's (#11486).
        let game_ticks = self.common.world.game_time as f64 + alpha;
        let delta_ticks = (game_ticks - self.client.polytone_post_ticks).clamp(0.0, 20.0) as f32;
        self.client.polytone_post_ticks = game_ticks;
        self.evaluate_pack_expressions(polytone_frame);
        let (polytone_effects, polytone_extras) =
            match (&self.client.client_pack.polytone_post, polytone_frame) {
                (Some(post), Some(frame)) => {
                    let (effects, extras) = post.frame(frame);
                    (effects, Some(extras))
                }
                _ => (Vec::new(), None),
            };
        let mut requested_post_effects =
            std::mem::take(&mut self.client.requested_post_effects_buf);
        self.sync_requested_post_effects(
            &mut requested_post_effects,
            polytone_effects.iter().map(String::as_str),
        );
        // Polytone's globals (wiki "Extra Shader Uniforms"), built only when something reads
        // them: here when an installed pack core shader does, and by the post effects when a
        // pass of theirs does (`Request::poly_globals`, called at most once a frame). The
        // renderer keeps the bytes in each reading core pipeline's blocks, which write them to
        // the GPU when that pipeline uploads its blocks for its draw; a post pass writes them
        // when it draws.
        let (sun_angle, day_time) = (sky.sun_angle, self.view_day_time());
        let player = (self.common.player.as_ref()).map_or(camera.to_array(), |p| {
            [p.position.x, p.position.y, p.position.z]
        });
        let poly_globals = move || sunrise_client::polytone_post::PolyGlobals {
            // 26.3's reverse-Z projection, as the world's (and its depth) is.
            projection: glam::Mat4::perspective_rh(
                fov_degrees.to_radians(),
                size[0] as f32 / size[1].max(1) as f32,
                depth_far,
                0.05,
            ),
            model_view: posed_bob * spin * frame_view_matrix,
            sun_angle: sun_angle.to_radians() - std::f32::consts::FRAC_PI_2,
            day_time: day_time.rem_euclid(24000) as f32,
            delta_time: delta_ticks,
            player,
        };
        let post_globals = move || poly_globals().bytes();
        let core_globals = self.reads_poly_globals().then(poly_globals);
        self.give_poly_globals(core_globals.as_ref());
        self.client.expression_frame = frame;
        // Camera.createProjectionMatrixForCulling: max(fov, configuredFov), without bob or spin.
        self.client.view_frustum = self.world_view_frustum(
            camera,
            [cull_fov_degrees, yaw, pitch, roll],
            view_transform,
            depth_far,
        );
        let billboard_frustum = if self.client.settings.sunrise.taa {
            self.client.renderer.0.as_ref().map(|r| {
                r.camera_frustum_with_jitter(
                    cull_fov_degrees,
                    yaw,
                    pitch,
                    roll,
                    view_transform,
                    depth_far,
                )
            })
        } else {
            self.client.view_frustum
        };
        // Keep the buffer local while builders need mutable access to the game.
        let mut entities = std::mem::take(&mut self.client.entity_vertices);
        entities.clear();
        let mut static_block_entities = std::sync::Arc::from([]);
        let mut visible_static_block_entity_sections = Vec::new();
        let mut static_block_entity_stats = StaticCacheFrameStats::default();
        self.client.see_through_vertices.clear();
        self.client.glowing_vertices.get_mut().clear();
        self.client.shadow_only_vertices.get_mut().clear();
        pr.lap("pr_entity_setup");
        // The pack is chosen before entity geometry, so a pack that starts this frame
        // keeps block-entity models in the CPU stream instead of dropping them.
        let mut pack_inputs = if title {
            Default::default()
        } else {
            self.shader_pack_inputs(camera, eye.map_or(camera, |e| e.0), alpha as f32, &sky)
        };
        // Retain the existing additive particle stream; the ordered dragon emitter appends
        // the eyes and rays to it without changing their material routing.
        let mut additive = self.client.particles.0.take_additive();
        if !title {
            self.client
                .frame_batch_reserved_quads
                .set(additive.len() / 4);
            let (built, static_sections, visible_static_sections, static_stats) =
                spans::time("entity_geometry", || {
                    self.entity_geometry(camera, alpha, billboard_frustum, entities, &mut additive)
                });
            entities = built;
            static_block_entities = static_sections;
            visible_static_block_entity_sections = visible_static_sections;
            static_block_entity_stats = static_stats;
        } else {
            // No level behind the panorama, so no entity either: the GPU path's
            // draws are started by `begin_skinning`, which only `entity_geometry`
            // runs, and would otherwise stay those of the last world frame and
            // be drawn over the title (#8192). Its models stay stored.
            self.client.frame_batch_reserved_quads.set(0);
            self.client.skin_frame.get_mut().clear_draws();
        }
        pr.lap("pr_entities");
        let bolts = if title {
            Vec::new()
        } else {
            self.bolt_instances(camera)
        };
        if !title {
            self.dragon_fireball_geometry(camera, alpha, &mut entities);
        }
        pr.lap("pr_dragon_fireballs_bolts");
        let sky_effect_blocked = self.camera_subject().is_some_and(|c| c.blocks_sky);
        let (celestial, celestial_bodies, stars) = if title {
            (Vec::new(), Vec::new(), None)
        } else if self.overworld_skybox() && !sky_effect_blocked {
            // Camera.doesMobEffectBlockSky: no sun, moon or stars.
            self.celestial_geometry(&sky, rain)
        } else if self.end_skybox() && self.client.settings.sunrise.sky {
            let (flash, bodies) = self.end_flash_geometry();
            (flash, bodies, None)
        } else {
            (Vec::new(), Vec::new(), None)
        };
        self.custom_sky_frame([camera.x, camera.y, camera.z], &sky, (rain, thunder), title);
        pr.lap("pr_celestial");
        let (mut hand, hand_items) = if title || self.scoping() {
            Default::default()
        } else {
            self.client
                .frame_batch_reserved_quads
                .set((additive.len() / 4).saturating_add(entities.len() / 4));
            self.hand_geometry(camera_alpha)
        };
        // GameRenderer.render3dHud: the screen effects after the hand, in
        // its pass (the hand pass draws with `bob * view`).
        if !title {
            let view = posed_bob * frame_view_matrix;
            let aspect = size[0] as f32 / size[1].max(1) as f32;
            self.item_activation_geometry(camera_alpha as f32, view, aspect, &mut hand);
        }
        pr.lap("pr_hand");
        let hud_fov = self.hud_fov_degrees(camera_alpha as f32, camera.to_array());
        let screen = if title {
            Vec::new()
        } else {
            let scale = self
                .common
                .player
                .as_mut()
                .and_then(|p| p.attribute("minecraft:scale").ok())
                .unwrap_or(1.0) as f32;
            let hand_projection = sunrise_client::camera::hand_projection(
                hud_fov,
                size[0] as f32,
                size[1].max(1) as f32,
                depth_far,
            );
            self.screen_quads(alpha as f32, size, scale, hand_projection)
        };
        if !title {
            self.eye_geometry(camera, &mut entities);
        }
        self.outline_fill(&outline, camera, &mut entities);
        pr.lap("pr_screen_end");
        let weather = if !title && self.can_have_weather() && self.client.settings.sunrise.weather {
            sunrise_level::spans::time("weather", || self.weather_geometry(camera, alpha))
        } else {
            Vec::new()
        };
        let border = (!title)
            .then(|| self.border_draw(camera.to_array(), alpha as f32))
            .flatten();
        pr.lap("pr_weather_border");
        if title {
            self.client.dynamic_lights.clear();
        } else {
            self.gather_dynamic_lights(camera, alpha);
        }
        pr.lap("pr_lights");
        pack_inputs.frame_timing = self
            .client
            .screenshot_sequence
            .0
            .as_ref()
            .map(screenshot_sequence::Sequence::timing);
        pr.lap("pr_shader_inputs");
        // Fog hooks may clear particles too. They must run before the final clear
        // boundary, not while constructing Frame after geometry has been extracted.
        let fog = self.frame_fog(camera, &sky, biome, water_fog);
        let mut explicit_particle_quads = std::mem::take(&mut self.client.explicit_particle_quads);
        explicit_particle_quads.clear();
        if !title {
            // The renderer decides on the packs' GPU particle shaders before the quads are
            // built, and takes the records of the types it draws after.
            self.polytone_gpu_decide(camera);
            let cull = self.client.particles.0.has_provider_particles().then(|| {
                sunrise_client::particles::ParticleCullView::new(
                    camera,
                    [yaw, pitch],
                    size,
                    self.client.settings.fov,
                    fov_degrees,
                    depth_far,
                )
            });
            self.client_particle_geometry(
                camera,
                alpha,
                fov_degrees,
                cull.as_ref()
                    .map(|cull| (cull, &mut explicit_particle_quads)),
                &mut entities,
            );
            self.polytone_gpu_particles(camera, alpha);
        }
        if !title {
            self.follow_debug_selection();
        }
        let mut world_geometry = self.client.world_geometry.take().expect("view storage");
        let white = self.client.gui.atlas.sprites["sunrise/white"];
        world_geometry.reset(
            camera.to_array(),
            view,
            [(white.u0 + white.u1) * 0.5, (white.v0 + white.v1) * 0.5],
        );
        world_geometry.set_model_cut(entities.len() as u32);
        world_geometry.set_time(
            ((self.common.world.game_time.rem_euclid(72_000) as f64 + alpha) / 20.0) as f32,
        );
        if let Some((skin, slim)) = vanilla_skin
            && let Some(avatar) = self.client.player_model.as_mut()
        {
            (avatar.skin, avatar.slim) = (skin, slim);
        }
        // After the mods' world geometry, whose labels add see-through copies.
        let see_through = std::mem::take(&mut self.client.see_through_vertices);
        let colored_lights = if self.client.title.0.is_none() {
            visuals.finish(camera)
        } else {
            Vec::new()
        };
        // Map textures and font glyphs changed while building this frame's
        // geometry (game/blocks/maps/mod.rs, world text) upload with this batch; draining
        // map_patches any earlier leaves them a frame behind.
        hud.patches.append(&mut self.client.map_patches);
        hud.patches.append(self.client.text_patches.get_mut());
        self.dynamic_texture_commands(&mut hud.dynamic_commands);
        let entity_spans = std::mem::take(self.client.entity_spans.get_mut());
        let entity_motion = std::mem::take(self.client.entity_motion.get_mut());
        let entity_motion_posed = std::mem::take(self.client.entity_motion_posed.get_mut());
        let glowing = std::mem::take(self.client.glowing_vertices.get_mut());
        let shadow_only = std::mem::take(self.client.shadow_only_vertices.get_mut());
        let skin = std::mem::take(self.client.skin_frame.get_mut());
        use sunrise_world::dimension_type::Skybox;
        let skybox = self.dimension_type().skybox;
        let camera_fog = self.camera_fog_type(camera.to_array());
        let boss_world_fog = self.client.boss_overlay.should_create_world_fog();
        let fast_clouds = self.effective_clouds() == "fast";
        let frame = Frame {
            weather: &weather,
            weather_time: sunrise_client::weather::Time::new(
                self.common.world.game_time,
                alpha as f32,
            ),
            nether_lighting: self.nether_lighting(),
            camera,
            yaw,
            pitch,
            roll,
            fov_degrees,
            cull_fov_degrees,
            eye: eye.filter(|_| !title).map_or(camera, |e| e.0),
            eye_rotation: eye.filter(|_| !title).map_or([yaw, pitch], |e| e.1),
            first_person: eye.is_none() || title || !self.client.view.detached,
            bob,
            view_transform,
            spin,
            sky,
            shadow_scene_revision: (
                self.common.world.revision,
                self.common.world.chunks.membership_revision(),
            ),
            shadow_cache: self.client.settings.sunrise.shadow_cache,
            distance,
            detail_distance: effective_detail_distance,
            shadow_distance: Some(effective_detail_distance),
            depth_far,
            underwater,
            title,
            sky_pass: !title
                && sunrise_client::fog::sky_pass_present(camera_fog, sky_effect_blocked, skybox),
            sky_blocked: sunrise_client::fog::sky_pass_skipped(
                camera_fog,
                sky_effect_blocked,
                boss_world_fog,
            ),
            skybox: skybox == Skybox::Overworld && self.client.settings.sunrise.sky,
            end_sky: skybox == Skybox::End && self.client.settings.sunrise.sky,
            end_sky_sprite: self
                .client
                .gui
                .atlas
                .sprites
                .get("environment/end_sky")
                .map(|r| [r.u0, r.v0, r.u1, r.v1]),
            boss_world_fog,
            environment_fog: (self.common.dim == Dim::Nether)
                .then_some((sky.fog_start, sky.fog_end)),
            cave_culling,
            water_fog,
            fog: Some(fog),
            dark_disc: self.dark_disc(camera),
            outline,
            outline_after_terrain,
            outline_high_contrast: self.client.settings.file.bool("highContrastBlockOutline"),
            pvp: self.pvp_render_frame(),
            debug_lines: &debug_lines,
            debug_lines_on_top: &debug_lines_on_top,
            debug_crosshair: &debug_crosshair,
            hud: &hud,
            glint_alpha: self.client.glint_frame.alpha,
            glint_ticks: self.client.glint_frame.scroll_ticks(),
            clouds: self
                .frame_cloud_uv(title, &sky)
                .map(|uv| sunrise_client::clouds::Input {
                    texture: &self.client.client_pack.clouds,
                    uv,
                    camera,
                    time: self.common.world.game_time,
                    partial_tick: alpha,
                    range: self.client.settings.cloud_range,
                    fast: fast_clouds,
                    color: sky.cloud_color,
                    height: sky.cloud_height,
                }),
            entities: &entities,
            explicit_particles: Some(&explicit_particle_quads),
            static_block_entities: &static_block_entities,
            visible_static_block_entity_sections: &visible_static_block_entity_sections,
            static_block_entity_stats,
            entity_motion: &entity_motion,
            entity_motion_posed: &entity_motion_posed,
            entity_spans: &entity_spans,
            skin: skin.pending().then_some(&skin),
            glowing: &glowing,
            dynamic_lights: &self.client.dynamic_lights,
            colored_lights: &colored_lights,
            emissive_layers: if self.client.title.0.is_none() {
                &self.client.client_pack.render.emissive_layers
            } else {
                &[]
            },
            lightning: sunrise_client::gfx::lightning::Input {
                bolts: &bolts,
                uv: sunrise_client::gfx::lightning::white_uv(&self.client.gui.atlas.font.white()),
            },
            additive: &additive,
            see_through: &see_through,
            world_primitives: Default::default(),
            shadow_only: &shadow_only,
            celestial: if self.client.settings.sunrise.sky || title {
                &celestial
            } else {
                &[]
            },
            // With `celestial` left empty, these have no quad to draw.
            celestial_bodies: &celestial_bodies,
            stars: stars.filter(|_| self.client.settings.sunrise.sky || title),
            hand: &hand,
            hand_items: &hand_items,
            hand_fov_degrees: hud_fov,
            screen: &screen,
            pack: pack_inputs,
            border,
            game_time: self.common.world.game_time as f32,
            shader_game_time: ((self.common.world.game_time % 24000) as f32 + alpha as f32)
                / 24000.0,
            crumbling_sprite: frame_geometry::crumbling_sprite(
                &self.client.gui.atlas.sprites,
                self.client.settings.improved_transparency,
            ),
            end_portal_sprites: ["environment/end_sky", "entity/end_portal/end_portal"].map(|n| {
                self.client
                    .gui
                    .atlas
                    .sprites
                    .get(n)
                    .map_or([0.0; 4], |r| [r.u0, r.v0, r.u1 - r.u0, r.v1 - r.v0])
            }),
            ticks: self.texture_clock(),
            darkness: self.darkness_lighting(alpha as f32),
            night_vision: self.night_vision_intensity(alpha as f32),
            block_flicker: self.client.fog.block_light_flicker,
            boss_darken: self
                .client
                .boss_overlay
                .darken_amount(self.common.clock.fraction() as f32),
            menu_blur: self.menu_blur(),
            post_effects: sunrise_client::post_chain::Request {
                effects: &requested_post_effects,
                root: &self.common.pack.source,
                extras: polytone_extras.as_ref(),
                poly_globals: Some(&post_globals),
                menu_blur_radius: self.client.settings.menu_blur as f32,
                // GlobalSettingsUniform.update.
                globals: sunrise_client::post_chain::Globals {
                    camera_block_pos: [
                        camera.x.floor() as i32,
                        camera.y.floor() as i32,
                        camera.z.floor() as i32,
                    ],
                    camera_offset: [
                        (camera.x.floor() - camera.x) as f32,
                        (camera.y.floor() - camera.y) as f32,
                        (camera.z.floor() - camera.z) as f32,
                    ],
                    game_time: ((self.common.world.game_time % 24000) as f32 + alpha as f32)
                        / 24000.0,
                    ..Default::default()
                },
            },
            chunk_fade: self.client.settings.chunk_fade as f32,
        };
        pr.lap("pr_frame");
        // Renderer encoding and GPU waiting belong to render, not prepare.
        drop(prepare);
        let debug_passes = self.debug_lists_gpu_passes();
        let lightmap_chart = self.debug_shows_lightmap();
        let mut prepared_draws =
            std::mem::take(&mut *self.client.prepared_model_draws.borrow_mut());
        let mut renderer_slot = self.client.renderer.0.take();
        let result = match renderer_slot.as_mut() {
            Some(renderer) => {
                use sunrise_client::client_perf::request::GpuSwitch;
                let capture_gpu = self.client.profile.gpu();
                let recording = capture_gpu != GpuSwitch::Off;
                renderer.set_pass_timing(debug_passes || self.client.gpu_passes || recording);
                renderer.set_lightmap_readback(lightmap_chart);
                renderer.set_keep_timed_frames(recording);
                renderer.set_pass_detail(
                    self.client.gpu_pass_detail || capture_gpu == GpuSwitch::Detail,
                );
                // `gpu_passes` and a capture's `--gpu` are measurements: they take every sample they can.
                renderer.set_pass_timing_gap(if self.client.gpu_passes || recording {
                    Duration::ZERO
                } else {
                    sunrise_client::debug_overlay::GPU_TIMING_GAP
                });
                renderer.stage_prepared_draws(&prepared_draws);
                self.client.render_snapshot.set(Some(RenderSnapshot {
                    framebuffer: [renderer.config.width, renderer.config.height],
                    prepared_generation: renderer.prepared_generation(),
                    staged_prepared_draws: prepared_draws.len() as u64,
                }));
                self.client
                    .pack_frame_state
                    .set(renderer.shader_pack_state(&frame));
                let mut slots = self.client.pack_frame_slots.borrow_mut();
                slots.clear();
                slots.extend_from_slice(renderer.pack_bound_mod_slots());
                drop(slots);
                let world_camera = self
                    .client
                    .native_camera_frame
                    .as_ref()
                    .filter(|_| self.has_world_render_hooks());
                let mut driver = world_camera.map(|camera| {
                    native_custom_geometry::GameWorldStageDriver::new(
                        self,
                        camera,
                        alpha,
                        &mut world_geometry,
                    )
                });
                let rendered = consume(
                    renderer,
                    &frame,
                    driver
                        .as_mut()
                        .map(|d| d as &mut dyn sunrise_client::gfx::world_primitives::FrameHooks),
                );
                drop(driver);
                self.client.render_snapshot.set(None);
                rendered
            }
            None => {
                self.client.pack_frame_state.set(None);
                self.client.pack_frame_slots.borrow_mut().clear();
                Err("renderer".into())
            }
        };
        drop(frame);
        self.client.renderer.0 = renderer_slot;
        self.apply_pending_present_mode();
        self.client.requested_post_effects_buf = requested_post_effects;
        prepared_draws.clear();
        *self.client.prepared_model_draws.borrow_mut() = prepared_draws;
        let _cleanup = spans::guard("pr_cleanup");
        // A frame the surface refused (occluded, lost, timed out) is `Ok(false)`.
        self.client.frame_drawn = matches!(result, Ok(true));
        // Restore storage even when rendering fails (for example, a lost surface).
        self.client.entity_vertices = entities;
        self.client.explicit_particle_quads = explicit_particle_quads;
        self.client.world_geometry = Some(world_geometry);
        *self.client.entity_spans.get_mut() = entity_spans;
        *self.client.entity_motion.get_mut() = entity_motion;
        *self.client.entity_motion_posed.get_mut() = entity_motion_posed;
        *self.client.glowing_vertices.get_mut() = glowing;
        *self.client.shadow_only_vertices.get_mut() = shadow_only;
        let mut skin = skin;
        // The renderer took the new models when its store grew by them; otherwise they stay
        // pending, or the stores restart (`SkinModels::begin`).
        if self
            .client
            .renderer
            .0
            .as_ref()
            .and_then(|r| r.skinned_models())
            .is_some_and(|stored| stored == skin.base + skin.new_model.len() as u32)
        {
            skin.handed();
        }
        *self.client.skin_frame.get_mut() = skin;
        self.client.see_through_vertices = see_through;
        result
    }
    pub(in crate::game::client_side) fn renderer_pointer_projection(
        &self,
    ) -> Option<sunrise_client::window::PointerProjection> {
        let r = self.client.renderer.0.as_ref()?;
        Some(self.pointer_projection([r.config.width, r.config.height]))
    }
    /// Dispatch the button to the top screen, not to the gameplay mappings underneath it.
    pub(in crate::game::client_side) fn route_screen_button(
        &mut self,
        button: MouseButton,
        code: i32,
        pressed: bool,
        double_click: bool,
    ) -> Result<bool> {
        let left = button == MouseButton::Left;
        let left_press = pressed && left;
        if self.client.command_prompt.0.is_some() {
            return Ok(left_press && self.command_prompt_click());
        }
        if self.client.link_prompt.is_some() {
            return Ok(left_press && self.link_prompt_click());
        }
        if self.client.code_of_conduct.is_some() {
            return Ok(left_press && self.code_of_conduct_click()?);
        }
        if self.client.server_packs.0.prompt.is_some() {
            return Ok(left_press && self.pack_prompt_click()?);
        }
        if self.dialog_open() {
            return if !left {
                Ok(false)
            } else if pressed {
                self.dialog_click()
            } else {
                Ok(self.dialog_release())
            };
        }
        // ConnectScreen's Cancel, under the prompts and dialogs above.
        if self.remote_loading() {
            return Ok(left_press && self.connect_screen_click()?);
        }
        if self.client.screen == Screen::Win {
            return Ok(false);
        }
        if self.client.screen == Screen::Options {
            return if self.options_waiting_for_key() && pressed {
                self.options_bind_mouse(button)?;
                Ok(true)
            } else if matches!(button, MouseButton::Left | MouseButton::Right) {
                self.options_mouse(pressed, !left)
            } else {
                Ok(false)
            };
        }
        if self.title_input_active() {
            if !pressed {
                return Ok(left
                    && self.client.title.0.as_mut().is_some_and(|title| {
                        let released = title.release();
                        title.release_field() || released
                    }));
            }
            if !matches!(button, MouseButton::Left | MouseButton::Middle) {
                return Ok(self.preset_editor_mouse_clicked(code, double_click));
            }
            let size = self
                .client
                .renderer
                .0
                .as_ref()
                .map_or([1, 1], |r| [r.config.width, r.config.height]);
            let b = HudBatch::new(size, self.client.settings.gui_scale_inputs());
            if button == MouseButton::Middle {
                // The world list takes any button; middle selects without joining.
                return Ok(self.preset_editor_mouse_clicked(code, double_click)
                    || self.client.title.0.as_mut().is_some_and(|title| {
                        title.select_world_at(
                            self.client.cursor.0,
                            self.client.cursor.1,
                            (b.width, b.height),
                        )
                    }));
            }
            let atlas = &self.client.gui.atlas;
            let shift = self.has_shift_down();
            let (action, sound, widget) =
                self.client
                    .title
                    .0
                    .as_mut()
                    .map_or((Action::None, false, false), |title| {
                        let (action, sound, widget) = title.click_with_outcome(
                            self.client.cursor.0,
                            self.client.cursor.1,
                            (b.width, b.height),
                        );
                        let field = title.press_field(
                            atlas,
                            self.client.cursor,
                            (b.width, b.height),
                            shift,
                        );
                        let (rule_sound, rules) = title.press_rules_with_outcome(
                            atlas,
                            self.client.cursor,
                            (b.width, b.height),
                        );
                        (action, sound || rule_sound, widget || field || rules)
                    });
            // Done/Cancel are the bound editor's normal widgets. They take the click even
            // when the actual mode change is deferred to `settle_title_mode`.
            let handled = widget || self.preset_editor_mouse_clicked(code, double_click);
            self.save_title_options();
            if sound {
                self.play_audio_button_click();
            }
            self.title_action(action)?;
            self.reopen_data_packs()?;
            return Ok(handled);
        }
        if self.client.book.is_some() || self.client.sign_edit.is_some() {
            if !left_press {
                return Ok(false);
            }
            let size = self
                .client
                .renderer
                .0
                .as_ref()
                .map_or([1, 1], |r| [r.config.width, r.config.height]);
            return Ok(if self.client.book.is_some() {
                self.book_click(size)
            } else {
                self.sign_edit_click(size)
            });
        }
        if self.studio_mouse_input(pressed, button) {
            return Ok(true);
        }
        if matches!(self.client.screen, Screen::Inventory | Screen::Creative)
            && let Some(key) = self
                .client
                .settings
                .keys
                .container_mouse_key(button, self.common.creative)
        {
            if pressed && self.common.carried.is_none() {
                self.inventory_hotkey(key);
            }
            // Neither edge of a bound button is an ordinary slot click.
            return Ok(true);
        }
        if pressed
            && (self.sort_mouse_press(button)
                || self.hud_editor_key_press(&sunrise_client::keybinds::mouse_name(button)))
        {
            return Ok(true);
        }
        // AbstractContainerScreen.mouseClicked (also Creative's fallback) handles
        // every press, even when no widget or slot accepts it. Releases answer separately.
        Ok(match self.client.screen {
            Screen::Inventory | Screen::Creative
                if !matches!(button, MouseButton::Left | MouseButton::Right) =>
            {
                if self.client.settings.keys.get("key.pickItem")
                    != sunrise_client::keybinds::mouse_name(button)
                {
                    return Ok(pressed);
                }
                if pressed {
                    self.inventory_pick_press();
                    true
                } else {
                    self.inventory_pick_release()
                }
            }
            Screen::Inventory if pressed => {
                self.inventory_press(!left);
                true
            }
            Screen::Inventory => {
                self.inventory_mouse_release(!left)
                    || (self.station_release() | self.recipe_book_release())
            }
            Screen::Chat if left_press => self.chat_click(),
            Screen::Creative if pressed => {
                self.click_creative(!left);
                true
            }
            Screen::Creative => {
                self.inventory_mouse_release(!left)
                    || (left && self.client.creative_screen.0.release())
            }
            Screen::Paused if left_press && self.client.pause_menu => self.pause_click()?,
            Screen::WorldOptions if left_press => self.world_options_click(),
            Screen::ServerLinks if left_press => self.server_links_click()?,
            Screen::DebugOptions if left_press => self.debug_options_click(),
            Screen::DebugOptions if left => self.debug_options_release(),
            Screen::HudEditor
                if pressed && matches!(button, MouseButton::Left | MouseButton::Right) =>
            {
                self.hud_editor_press(!left)
            }
            Screen::HudEditor if left => self.hud_editor_release(),
            Screen::ConfirmLink if left_press => self.confirm_link_click(),
            Screen::DemoIntro if left_press => self.demo_intro_click(),
            Screen::DialogWarning if left_press => self.dialog_warning_click()?,
            Screen::WorldOptions if left => self.world_options_release(),
            Screen::Friends if left_press => self.friends_screen_click(),
            Screen::Social if left_press => self.social_click(),
            Screen::CommandBlock if left_press => self.command_block_click(),
            Screen::BlockEditor if left_press => self.block_editor_click(),
            Screen::BlockEditor if left => self.block_editor_release(),
            Screen::Dead if left_press => self.death_click()?,
            Screen::Advancements if left => {
                let size = self
                    .client
                    .renderer
                    .0
                    .as_ref()
                    .map_or([1, 1], |r| [r.config.width, r.config.height]);
                self.advancements_mouse(pressed, size)
            }
            Screen::Stats if left_press => {
                let size = self
                    .client
                    .renderer
                    .0
                    .as_ref()
                    .map_or([1, 1], |r| [r.config.width, r.config.height]);
                self.stats_click(size)
            }
            Screen::Stats if left => self.stats_release(),
            Screen::GameModeSwitcher if !pressed => self.switcher_release(button),
            _ => false,
        })
    }
    /// Dispatch to the topmost screen only, taking the receiver's own result.
    pub(in crate::game::client_side) fn screen_drag_response(
        &mut self,
        overlay: Option<&str>,
        button: MouseButton,
        framebuffer: [u32; 2],
        gui: (f32, f32),
    ) -> Result<bool> {
        let left = button == MouseButton::Left;
        if let Some(overlay) = overlay {
            return Ok(left && overlay == "dialog" && self.dialog_drag());
        }
        if !self.pad_synthetic_input() && self.container_drag() {
            return Ok(true);
        }
        if self.client.screen == Screen::Playing
            && let Some(title) = self.client.title.0.as_mut()
        {
            return Ok(left
                && (title.drag(self.client.cursor, gui)
                    || title.drag_field(&self.client.gui.atlas, self.client.cursor.0)));
        }
        Ok(match self.client.screen {
            Screen::BlockEditor if left => self.block_editor_drag(),
            Screen::Advancements if left => self.advancements_drag(),
            Screen::Stats if left => self.stats_drag(),
            Screen::DebugOptions if left => self.debug_options_drag(),
            Screen::HudEditor if left => self.hud_editor_drag(),
            Screen::WorldOptions if left => self.world_options_drag(),
            Screen::Options if left => self.options_drag()?,
            Screen::Creative => {
                let batch = HudBatch::new(framebuffer, self.client.settings.gui_scale_inputs());
                let (_, oy) = Self::creative_origin(&batch);
                (left
                    && self
                        .client
                        .creative_screen
                        .0
                        .drag_scrollbar(self.client.cursor.1 - oy))
                    || self.inventory_drag_move()
            }
            Screen::Inventory => {
                (left && (self.station_drag() || self.recipe_book_drag()))
                    || self.inventory_drag_move()
            }
            _ => false,
        })
    }
    pub(in crate::game::client_side) fn screen_key_response(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        key: KeyCode,
        pressed: bool,
        repeat: bool,
        shortcut: Option<sunrise_client::clipboard::EditShortcut>,
    ) -> Result<bool> {
        let instance = self.client.screen_state.0.screen_instance;
        let modifiers = glfw_modifiers(self.client.modifiers);
        if !pressed {
            if self.mounted_widget_key_released(key, modifiers) {
                return Ok(true);
            }
            if self.client.screen_state.0.screen_instance != instance
                || self.observed_overlay_now().is_some()
            {
                return Ok(false);
            }
            return match self.client.screen {
                Screen::Options => self.options_key_released(key),
                Screen::Win => self.win_key(key, false),
                _ => Ok(false),
            };
        }
        if !repeat && self.container_key(key) {
            return Ok(true);
        }
        if self.client.screen_state.0.screen_instance != instance {
            return Ok(false);
        }
        // AdvancementsScreen.keyPressed consumes its own binding before its children.
        if self.client.screen == Screen::Advancements
            && self.observed_overlay_now().is_none()
            && sunrise_client::keybinds::key_name(key)
                == Some(self.client.settings.keys.get("key.advancements"))
        {
            self.close_advancements();
            return Ok(true);
        }
        if key == KeyCode::Escape
            && self.client.screen == Screen::Mod
            && self.observed_overlay_now().is_none()
        {
            self.close_mod_screen();
            return Ok(true);
        }
        if self.mounted_widget_key_pressed(key, repeat, modifiers) {
            return Ok(true);
        }
        if self.client.screen_state.0.screen_instance != instance {
            return Ok(false);
        }
        let overlay = self.observed_overlay_now();
        let container =
            matches!(self.client.screen, Screen::Inventory | Screen::Creative) && overlay.is_none();
        if overlay == Some("dialog")
            && (!self.client.screen_state.0.widgets.has_focus() || key == KeyCode::Escape)
            && self.dialog_key(key, shortcut)?
        {
            return Ok(true);
        }
        if self.preset_editor_key(key) {
            return Ok(true);
        }
        let focus_handled = self.key_focus_key(event_loop, key);
        if let Some(handled) = focus_handled
            && (handled || !container)
        {
            return Ok(handled);
        }
        if self.client.screen_state.0.screen_instance != instance {
            return Ok(false);
        }
        // The real focused child has already had the key. A custom child's declined key
        // is not translated into a mouse click or passed to a vanilla field behind it.
        if !container && self.client.screen_state.0.widgets.has_focus() && key != KeyCode::Escape {
            return Ok(false);
        }
        match self.observed_overlay_now() {
            Some("command_prompt") => return Ok(self.command_prompt_key(key)),
            Some("link_prompt") => {
                if key == KeyCode::Escape {
                    self.answer_link_prompt(false);
                    return Ok(true);
                }
                return Ok(false);
            }
            Some("server_pack_prompt") => {
                if key == KeyCode::Escape {
                    self.answer_pack_prompt(false)?;
                    return Ok(true);
                }
                return Ok(false);
            }
            Some("code_of_conduct" | "dialog") => return Ok(false),
            _ => {}
        }
        if self.client.sign_edit.is_some() {
            return Ok(self.sign_edit_key(key, shortcut));
        }
        if self.client.book.is_some() {
            return Ok(self.book_key(key, shortcut));
        }
        if self.client.screen == Screen::Playing && self.client.title.0.is_some() {
            return self.title_screen_key(key, shortcut);
        }
        match self.client.screen {
            Screen::Chat => return Ok(self.chat_key(key, shortcut)),
            Screen::Social => return Ok(self.social_key(key)),
            Screen::CommandBlock => return Ok(self.command_block_key_code(key, shortcut, None)),
            Screen::BlockEditor => return Ok(self.block_editor_key_code(key, shortcut, None)),
            Screen::DebugOptions => return Ok(self.debug_options_key(key, shortcut)),
            Screen::Options => {
                if shortcut.is_some_and(|shortcut| self.options_shortcut(shortcut)) {
                    return Ok(true);
                }
                return self.options_key(key, None);
            }
            Screen::WorldOptions if key != KeyCode::Escape => {
                return Ok(self.world_options_key(key, shortcut));
            }
            Screen::Win => return self.win_key(key, true),
            Screen::Paused if self.client.pause_menu && key != KeyCode::Escape => {
                return self.pause_key(key);
            }
            Screen::Dead => return self.death_key(key),
            _ => {}
        }
        if container {
            return self.container_screen_key(key, shortcut);
        }
        if key == KeyCode::Escape && self.client.screen != Screen::Playing {
            self.key_pressed(key, key)?;
            return Ok(true);
        }
        Ok(false)
    }
    #[allow(dead_code)]
    #[inline(always)]
    pub(in crate::game) fn set_client_message(&mut self, text: String) {
        self.client.message = Some((text, Instant::now()));
    }
    #[inline(always)]
    pub(in crate::game) fn client_award_stat(&mut self, _: &str, _: &str, _: i32) {}
    #[inline(always)]
    pub(in crate::game) fn client_award_custom_stat(&mut self, _: &str, _: i32) {}
    #[allow(dead_code)]
    #[inline(always)]
    pub(in crate::game) fn client_container_changed(&mut self, _: (i32, i32, i32)) {}
    pub fn set_window_cursor(
        &mut self,
        sample: sunrise_client::window::PointerSample,
        projection: sunrise_client::window::PointerProjection,
    ) {
        self.client.window_cursor = Some(sample);
        self.client.cursor = projection.project_pointer(sample).subpixel();
    }
    /// The title remains the no-level sentinel while debug options or a mod's
    /// screen covers it. Only UI dispatch ignores it; world loading and
    /// simulation still see it.
    pub fn title_input_active(&self) -> bool {
        self.client.title.0.is_some()
            && !matches!(self.client.screen, Screen::DebugOptions | Screen::Mod)
    }
    pub(in crate::game::client_side) fn title_screen_key(
        &mut self,
        key: KeyCode,
        shortcut: Option<sunrise_client::clipboard::EditShortcut>,
    ) -> Result<bool> {
        let field_focused = self.vanilla_text_field_focused();
        if let Some(shortcut) = shortcut
            && field_focused
        {
            self.title_shortcut(shortcut);
            return Ok(true);
        }
        let ctrl = self.has_control_down_with_quirk();
        let shift = self.has_shift_down();
        if let Some(sound) = self
            .client
            .title
            .0
            .as_mut()
            .and_then(|t| t.tab_bar_key(key, ctrl, shift))
        {
            if sound {
                self.play_audio_button_click();
            }
            return Ok(true);
        }
        let size = self.gui_size();
        let Some(title) = self.client.title.0.as_mut() else {
            return Ok(false);
        };
        match key {
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space
                if key != KeyCode::Space
                    || (!title.has_caret_field()
                        && (title.list_focused() || title.focused_button.is_some())) =>
            {
                let (action, sound) = title.confirm_key_size(size);
                let handled = sound || !matches!(action, Action::None);
                self.save_title_options();
                if sound {
                    self.play_audio_button_click();
                }
                self.title_action(action)?;
                Ok(handled)
            }
            KeyCode::Backspace
            | KeyCode::Delete
            | KeyCode::ArrowLeft
            | KeyCode::ArrowRight
            | KeyCode::Home
            | KeyCode::End
                if title.has_caret_field() =>
            {
                Ok(title.field_key(key, ctrl, shift))
            }
            KeyCode::ArrowLeft | KeyCode::ArrowRight | KeyCode::ArrowUp | KeyCode::ArrowDown => {
                title.arrow_key(key, size, shift);
                Ok(false)
            }
            key if title.page_missing_mods(key, size) => Ok(true),
            KeyCode::Backspace if field_focused => {
                self.title_edit(None, true);
                Ok(true)
            }
            KeyCode::Tab => {
                title.tab_dir(shift, size);
                Ok(false)
            }
            KeyCode::Escape => {
                title.escape();
                self.reopen_data_packs()?;
                self.save_title_options();
                Ok(true)
            }
            KeyCode::F5 if matches!(title.mode, sunrise_client::title::Mode::Multiplayer(_)) => {
                title.refresh_servers();
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    /// Sets up the same block-entity scene for both world draw paths.
    pub(in crate::game::client_side) fn with_block_entity_scene(
        &self,
        camera: glam::DVec3,
        partial: f32,
        dynamic_visible: &dyn Fn([i32; 3], u32, glam::DVec3, glam::DVec3) -> bool,
        draw: impl FnOnce(&BlockEntityScene<'_>),
    ) {
        let lid = |p: [i32; 3]| self.chest_lid(p, partial);
        let anim = |p: [i32; 3]| self.block_anim(p, partial);
        let substitute = |text: &sunrise_game_data::text::Text| self.substitute_inline_images(text);
        let inline_sprite =
            |atlas: &str, sprite: &str| self.client.gui.atlas.inline_sprite(atlas, sprite);
        use block_entity_render::SignOutline;
        let sign_outline = if self.common.player.is_some()
            && self.client.camera_type.is_first_person()
            && self.scoping()
        {
            SignOutline::FirstPersonScope
        } else {
            self.camera_subject().map_or(SignOutline::None, |subject| {
                SignOutline::CameraEntity(glam::DVec3::from_array(subject.camera.position))
            })
        };
        let scene = BlockEntityScene {
            camera,
            sign_outline,
            orientation: self.camera_orientation(),
            anim: &anim,
            texts: None,
            partial,
            game_time: self.common.world.game_time,
            max_y: self.common.world.shape.max_y() - 1,
            world: &self.common.world,
            sprites: &self.client.gui.atlas.sprites,
            emissive: &self.client.gui.atlas.emissive,
            translucent_sprites: &self.client.gui.atlas.translucent,
            atlas_revision: self.client.gui.atlas.revision,
            nether: self.nether_lighting(),
            lid: &lid,
            sign: &|p| match self.common.world.block_entities.get(&(p[0], p[1], p[2])) {
                Some(BlockEntity::Sign(sign)) => Some(sign),
                _ => None,
            },
            inline_images: self.inline_images_revision().map(|revision| {
                block_entity_render::SignInlineImages {
                    revision,
                    substitute: &substitute,
                    sprite: &inline_sprite,
                }
            }),
            banner: &|p| self.banner_layers_at(p),
            skull: &|p| self.skull_skin_at(p),
            glyph: &|c| self.client.gui.atlas.glyph_cell(c),
            glyphs: self.client.gui.atlas.font.epoch(),
            font: Some(&self.client.gui.atlas.font),
            dynamic_visible: Some(dynamic_visible),
        };
        draw(&scene);
    }
}

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::integrated_common::IntegratedCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_components::NativeComponentsGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::build_creative_mode_tab_contents::GameHooks as _;
