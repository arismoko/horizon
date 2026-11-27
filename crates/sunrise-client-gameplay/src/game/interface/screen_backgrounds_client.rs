use super::*;
use crate::game::client_side::screens::ScreenClientExt as _;
use crate::game::host::draw::GameDrawContext;

impl crate::game::ClientGame<'_> {
    /// `LevelExtractor` / `LevelRenderer`: the world border wall this frame.
    pub(in crate::game) fn border_draw(
        &mut self,
        camera: [f64; 3],
        partial: f32,
    ) -> Option<sunrise_client::gfx::BorderDraw> {
        use sunrise_client::border_render::{self, BorderRenderer};
        let distance = f64::from(self.effective_render_distance() * 16);
        border_render::extract(
            &self.common.border,
            partial,
            camera,
            distance,
            &mut self.client.border_state,
        );
        // `LevelRenderer.repositionCamera`: the wall is rebuilt when the camera
        // enters another `SectionPos` (it only holds the stretch in range).
        let section = camera.map(|c| (c.floor() as i32) >> 4);
        if self.client.border_section != Some(section) {
            self.client.border_renderer.invalidate();
            self.client.border_section = Some(section);
        }
        // Camera.depthFar, as the renderer's far plane.
        let depth_far = sunrise_client::camera::depth_far(
            self.terrain_distance(),
            self.client.settings.cloud_range,
        );
        if !self.client.border_renderer.prepare(
            &self.client.border_state,
            camera,
            distance,
            depth_far,
        ) {
            return None;
        }
        let millis = self.visual_millis();
        Some(sunrise_client::gfx::BorderDraw {
            vertices: self.client.border_renderer.vertices,
            rebuilt: self.client.border_renderer.take_rebuilt(),
            transforms: self.client.border_renderer.transforms(
                &self.client.border_state,
                camera,
                millis,
            ),
            sides: BorderRenderer::sides(&self.client.border_state, camera, distance),
        })
    }
    /// Draw the presentation retained by the existing load tracker, if one matched at entry.
    pub fn draw_dimension_transition_screen(
        &self,
        batch: &mut HudBatch,
        tracker: &sunrise_client::level_loading::LevelLoadTracker,
        progress: Option<f32>,
    ) -> bool {
        let Some(screen) = tracker.transition_screen() else {
            return false;
        };
        let paint = batch.paint_checkpoint();
        let sounds = self.client.mod_sounds.pending_checkpoint();
        let blur_requested = self.client.screen_blur_requested.get();
        let clear_particles = self.client.mod_clear_particles.get();
        let camera_type = self.client.requested_camera_type.get();
        if screen.draw(
            self,
            tracker.reason.into(),
            progress,
            &mut GameDrawContext::new(self, batch),
        ) {
            return true;
        }
        paint.discard(batch);
        self.client.mod_sounds.discard_pending_since(sounds);
        self.client.screen_blur_requested.set(blur_requested);
        self.client.mod_clear_particles.set(clear_particles);
        self.client.requested_camera_type.set(camera_type);
        false
    }
    /// `LevelLoadingScreen.extractBackground`.
    pub fn draw_level_loading_background(
        &self,
        batch: &mut HudBatch,
        reason: sunrise_client::level_loading::Reason,
    ) {
        use sunrise_client::level_loading::Reason;
        let atlas = &self.client.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        match reason {
            // The panorama and its blur are drawn by the renderer.
            Reason::Other => {
                sunrise_client::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false)
            }
            Reason::NetherPortal => {
                let overlay = hud_overlays::portal_overlay(w as i32, h as i32, 1.0);
                hud_layout::paint(batch, atlas, &[overlay]);
            }
            Reason::EndPortal => batch.fill(atlas, 0.0, 0.0, w, h, rgba(20, 20, 20, 255)),
        }
    }
    /// Dispatch at the boundary reported by the screen that owns the drawing.
    /// No view or layout is built unless this particular phase has a listener.
    /// A mod's own screen: all of it is what the mods draw in its phases.
    pub(in crate::game) fn draw_mod_screen(&self, batch: &mut HudBatch) {
        use sunrise_client::screen_draw::RenderPhase;
        self.draw_screen_phase(batch, RenderPhase::Background);
        self.draw_screen_phase(batch, RenderPhase::Foreground);
    }
    /// Draw mounted widgets belonging to the topmost active overlay screen
    /// (`overlay_id`) inside that overlay's own draw pass, before tooltips.
    pub(in crate::game) fn draw_overlay_mounted_widgets(
        &self,
        batch: &mut HudBatch,
        overlay_id: &'static str,
    ) {
        if self.observed_overlay_now() == Some(overlay_id) {
            self.draw_mounted_screen_widgets(batch);
        }
    }
    pub fn draw_screen_phase(
        &self,
        batch: &mut HudBatch,
        phase: sunrise_client::screen_draw::RenderPhase,
    ) {
        use sunrise_client::screen_draw::RenderPhase;
        if matches!(phase, RenderPhase::Foreground) {
            if !self.screen_over_playing() {
                self.draw_mounted_screen_widgets(batch);
            }
            self.draw_container_hooks(batch);
        }
        let hook = match phase {
            RenderPhase::Background => mod_api::HookId::ScreenRenderBackground,
            RenderPhase::Foreground => mod_api::HookId::ScreenRenderForeground,
        };
        if !mod_api::__private::listened(hook) {
            return;
        }
        let view = self
            .screen_render_view(batch)
            .or_else(|| self.title_screen_render_view(batch));
        if let Some(view) = view {
            match phase {
                RenderPhase::Background => self.native_screen_render_background(batch, &view),
                RenderPhase::Foreground => self.native_screen_render_foreground(batch, &view),
            }
        }
    }
    /// `ScreenEvent.Render.Post`, after the screen's drawing and tooltips,
    /// including when Pre cancelled. Gameplay overlays keep their exclusions.
    /// Its view's `regions` is what this draw published
    /// (`publish_screen_regions`): empty when Pre cancelled the drawing.
    /// This is a notice; mounted children draw
    /// inside the Pre-guarded contents pass (`draw_screen_phase`).
    pub(in crate::game) fn draw_screen_with_post_hook(&self, batch: &mut HudBatch) {
        if let Some(view) = self.screen_render_view(batch) {
            self.screen_render_end(batch, &view);
        }
    }
    /// `ScreenEvent.Render.Pre` for the open screen: whether it draws.
    /// Background and Foreground are reported by the screen while drawing.
    pub(in crate::game) fn draw_screen_with_pre_hook(&self, batch: &mut HudBatch) -> bool {
        match self.screen_render_view(batch) {
            Some(view) => self.screen_render_begin(batch, &view),
            None => true,
        }
    }
    /// `draw_screen_with_post_hook` for the title's own screen, once a frame,
    /// after it has drawn and before the toasts.
    pub(in crate::game) fn draw_title_screen_with_post_hook(&self, batch: &mut HudBatch) {
        if let Some(view) = self.title_screen_render_view(batch) {
            self.screen_render_end(batch, &view);
        }
    }
    /// `draw_screen_with_pre_hook` for the title's own screens.
    pub(in crate::game) fn draw_title_screen_with_pre_hook(&self, batch: &mut HudBatch) -> bool {
        match self.title_screen_render_view(batch) {
            Some(view) => self.screen_render_begin(batch, &view),
            None => true,
        }
    }
    /// `Screen.extractBlurredBackground` over the panorama or the world and
    /// HUD, when `menuBackgroundBlurriness` is at least 1.
    pub fn menu_blur(&self) -> Option<f32> {
        let blurred = self.remote_loading()
            || self.client.title.0.as_ref().is_some_and(|t| {
                matches!(self.client.screen, Screen::Options | Screen::Win)
                    || !matches!(t.mode, sunrise_client::title::Mode::Main)
            })
            || (self.client.title.0.is_none()
                && self.common.player.is_some()
                && (self.client.screen.blurs_background(self.client.pause_menu)
                    || self.dialog_open()
                    || (self.client.screen == Screen::Win && self.win_screen_pauses())))
            || self.client.screen_blur_requested.get();
        let radius = self.client.settings.menu_blur as f32;
        let mut radius = (blurred && radius >= 1.0).then_some(radius);
        self.native_menu_background(&mut radius);
        radius.filter(|r| r.is_finite() && (1.0..=64.0).contains(r))
    }
    /// `Screen.renderTransparentBackground`: draws the translucent dark background
    /// behind an in-game screen (inventory, containers, creative, books, signs, editors),
    /// giving native mods a chance to suppress it (`Flow::Cancel`) or apply menu blur / tiled background.
    pub(in crate::game) fn render_transparent_background(
        &self,
        screen: &str,
        batch: &mut HudBatch,
    ) {
        let mut canceled = false;
        self.native_screen_transparent_background(screen, batch, &mut canceled);
        if !canceled {
            let atlas = &self.client.gui.atlas;
            batch.fill(
                atlas,
                0.0,
                0.0,
                batch.width,
                batch.height,
                rgba(16, 16, 16, 160),
            );
        }
        // Screen.extractBackground draws deferred subtitles after its dimming and before widgets.
        self.draw_deferred_subtitles(batch);
    }
    pub fn requested_post_effects(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.sync_requested_post_effects(&mut out, std::iter::empty());
        out
    }
    /// The open screen as the render hooks see it, once a frame: not while
    /// playing with no screen, nor while the server pack prompt or a book
    /// hides the open screen (`hud_layers` draws `Playing` then), and None
    /// with no mod listening to a render hook, so nothing is built then.
    pub(in crate::game::client_side) fn screen_render_view(
        &self,
        batch: &HudBatch,
    ) -> Option<mod_api::ScreenView<'_>> {
        let screen = self.client.screen;
        if screen == Screen::Playing
            || self.client.server_packs.0.prompt.is_some()
            || self.client.book.is_some()
            || !self.screen_render_listened()
        {
            return None;
        }
        // slot_count mirrors slots() without allocating; keep them in step.
        debug_assert_eq!(self.slot_count(), self.slots().len());
        Some(self.screen_view(batch, screen))
    }
    /// Whether the screen on top asked for the panorama through
    /// [`sunrise_client::screen_draw::Backdrop`].
    ///
    /// This is the opt-in path for Sunrise's own screens: a screen sets its
    /// `backdrop` and the renderer draws the panorama behind it, instead of
    /// every such screen needing its own branch in `shows_panorama`. Vanilla
    /// screens leave the backdrop at `None` and so never reach this.
    ///
    /// Only in a menu: in a world the panorama would replace the world the
    /// player is standing in, which is not what any of these screens want.
    pub(in crate::game::client_side) fn screen_wants_panorama(&self) -> bool {
        use sunrise_client::screen_draw::Backdrop;
        if self.common.player.is_some() {
            return false;
        }
        self.client
            .chat_ui
            .options
            .pack_screen
            .as_ref()
            .is_some_and(|p| p.backdrop == Backdrop::Panorama)
    }
    /// The frame shows the title's panorama, not a level: the title screen, or a
    /// LevelLoadingScreen (Reason.OTHER), which draws it too.
    pub fn shows_panorama(&self) -> bool {
        // The two cases that always had one: the title screen and waiting on
        // a server. Left exactly as they were.
        self.client.title.0.is_some() || self.remote_loading() || self.screen_wants_panorama()
    }
    /// `GameRenderer.update`'s `requestedPostEffects`: `end_of_frame` (a resource
    /// pack's own chain), then the server's `LocalPlayer.postEffects`, then the
    /// active spectated entity's chain, then `extra`. Reuses unchanged `String`
    /// slots in `out` without deduplicating repeated effect IDs.
    pub fn sync_requested_post_effects<'a>(
        &self,
        out: &mut Vec<String>,
        extra: impl IntoIterator<Item = &'a str>,
    ) {
        fn set_slot(out: &mut Vec<String>, len: &mut usize, id: &str) {
            if let Some(slot) = out.get_mut(*len) {
                if slot != id {
                    slot.clear();
                    slot.push_str(id);
                }
            } else {
                out.push(id.to_owned());
            }
            *len += 1;
        }
        let mut len = 0;
        if self.client.title.0.is_none() && self.common.player.is_some() {
            set_slot(out, &mut len, "minecraft:end_of_frame");
            for effect in &self.client.post_effects {
                set_slot(out, &mut len, effect);
            }
            if self.client.spectated_post_effect_active
                && let Some(effect) = self.client.spectated_post_effect
            {
                set_slot(out, &mut len, effect);
            }
        }
        for effect in extra {
            set_slot(out, &mut len, effect);
        }
        out.truncate(len);
    }
    /// The title's own screen as the render hooks see it, once a frame: not
    /// when a screen of the game's own is over the title (Options draws and
    /// fires its own), nor while a dialog, a prompt or a connect or load
    /// hides it, and None with no mod listening to a render hook.
    pub fn title_screen_render_view(&self, batch: &HudBatch) -> Option<mod_api::ScreenView<'_>> {
        if self.client.screen != Screen::Playing
            || self.screen_over_playing()
            || self.remote_loading()
            || !self.screen_render_listened()
        {
            return None;
        }
        self.title_screen_view(batch)
    }
}
