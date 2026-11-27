use super::*;
use mod_api::{
    ClientAudioOptions, ClientChatOptions, ClientGraphicsOptions, ClientGuiOptions,
    ClientInputOptions, ShaderPackState,
};

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Sunrise does.
impl crate::game::ClientGame<'_> {
    /// The GUI scale in effect (`Window.getGuiScale`): the scale the window
    /// applies from the GUI Scale option and its size, at least 1; none on a
    /// server or when no window exists. `gui_options().gui_scale` is only the
    /// option, where 0 means automatic.
    pub(in crate::game) fn gui_scale(&self) -> Option<u32> {
        // The size the renderer applies, as the drawing code reads it: it keeps its previous
        // configuration when the window reports a size it cannot draw at.
        let framebuffer = self
            .client
            .window
            .0
            .as_ref()
            .and_then(|_| self.renderer_framebuffer());
        resolved_gui_scale(
            framebuffer,
            self.client.settings.gui_scale,
            self.client.settings.force_unicode,
        )
    }
    /// `Window.getWidth` and `getHeight`: the window's framebuffer pixels, the size
    /// `window_resize` reports; none on a server or when no window exists.
    pub(in crate::game) fn window_size(&self) -> Option<[u32; 2]> {
        let size = self.client.window.0.as_ref()?.inner_size();
        Some([size.width, size.height])
    }
    /// A snapshot of the current graphics preferences.
    pub(in crate::game) fn graphics_options(&self) -> ClientGraphicsOptions {
        let s = &self.client.settings;
        ClientGraphicsOptions {
            fov: s.fov,
            render_distance: s.render_distance,
            simulation_distance: s.simulation_distance,
            brightness: s.gamma as f32,
            max_fps: s.max_fps,
            vsync: self.effective_vsync(),
            fullscreen: s.fullscreen,
            view_bobbing: self.effective_view_bobbing(),
            smooth_lighting: s.smooth_lighting,
            particles: s.particles,
            entity_distance: s.entity_distance as f32,
            biome_blend: s.biome_blend,
        }
    }

    /// VSync in effect: the session's (`set_session_vsync`), else the player's option.
    pub(in crate::game) fn effective_vsync(&self) -> bool {
        self.client
            .session_video
            .vsync
            .unwrap_or(self.client.settings.vsync)
    }

    /// The Clouds option in effect, as `options.txt` names it (`false`, `fast`, `true`).
    pub(in crate::game) fn effective_clouds(&self) -> &str {
        self.client
            .session_video
            .clouds
            .unwrap_or(self.client.settings.clouds.as_str())
    }

    /// `Options.enableVsync` for this session only, and the surface reconfigured at once, or,
    /// from a frame callback while `Renderer::render` has the renderer out, as soon as it is
    /// back ([`Self::apply_pending_present_mode`]).
    pub(in crate::game) fn set_session_vsync(&mut self, vsync: Option<bool>) {
        self.client.session_video.vsync = vsync;
        let vsync = self.effective_vsync();
        if let Some(renderer) = &mut self.client.renderer.0 {
            renderer.set_present_mode(vsync, self.client.settings.sunrise.present_mode);
        } else if self.client.render_snapshot.get().is_some() {
            self.client.present_mode_pending = true;
        }
    }

    /// Reconfigures the surface for a VSync change made while `Renderer::render` had the
    /// renderer out; `Game::render` calls it once the renderer is back.
    pub(in crate::game) fn apply_pending_present_mode(&mut self) {
        if !std::mem::take(&mut self.client.present_mode_pending) {
            return;
        }
        let vsync = self.effective_vsync();
        if let Some(renderer) = &mut self.client.renderer.0 {
            renderer.set_present_mode(vsync, self.client.settings.sunrise.present_mode);
        }
    }

    /// `Options.cloudStatus` for this session only.
    pub(in crate::game) fn set_session_clouds(&mut self, clouds: Option<mod_api::CloudStatus>) {
        self.client.session_video.clouds = clouds.map(|clouds| match clouds {
            mod_api::CloudStatus::Off => "false",
            mod_api::CloudStatus::Fast => "fast",
            mod_api::CloudStatus::Fancy => "true",
        });
    }

    /// View bobbing in effect: the session's (`set_session_view_bobbing`), else the player's
    /// option (`Options.bobView`).
    pub(in crate::game) fn effective_view_bobbing(&self) -> bool {
        self.client
            .session_video
            .view_bobbing
            .unwrap_or(self.client.settings.view_bobbing)
    }

    /// `Options.bobView` for this session only; the next frame's camera reads it.
    pub(in crate::game) fn set_session_view_bobbing(&mut self, view_bobbing: Option<bool>) {
        self.client.session_video.view_bobbing = view_bobbing;
    }

    /// The clouds in effect (`Options.getCloudsType`).
    pub(in crate::game) fn cloud_status(&self) -> mod_api::CloudStatus {
        match self.effective_clouds() {
            "false" => mod_api::CloudStatus::Off,
            "fast" => mod_api::CloudStatus::Fast,
            _ => mod_api::CloudStatus::Fancy,
        }
    }
    /// A snapshot of the current GUI and accessibility preferences.
    pub(in crate::game) fn gui_options(&self) -> ClientGuiOptions {
        gui_options_snapshot(&self.client.settings)
    }
    /// A snapshot of the current chat preferences.
    pub(in crate::game) fn chat_options(&self) -> ClientChatOptions {
        let s = &self.client.settings;
        ClientChatOptions {
            visibility: s.chat_visibility,
            colors: s.chat_colors,
            links: s.chat_links,
            opacity: s.chat_opacity as f32,
            scale: s.chat_scale as f32,
            width: s.chat_width as f32,
            height_focused: s.chat_height_focused as f32,
            height_unfocused: s.chat_height_unfocused as f32,
            line_spacing: s.chat_line_spacing as f32,
            delay: s.chat_delay,
        }
    }
    /// A snapshot of the current audio preferences.
    pub(in crate::game) fn audio_options(&self) -> ClientAudioOptions {
        ClientAudioOptions {
            master_volume: self.client.settings.master_volume as f32,
        }
    }
    /// A snapshot of the current input preferences.
    pub(in crate::game) fn input_options(&self) -> ClientInputOptions {
        let settings = &self.client.settings;
        ClientInputOptions {
            sensitivity: settings.sensitivity as f32,
            invert_x: settings.invert_x,
            invert_y: settings.invert_y,
            mouse_wheel_sensitivity: settings.mouse_wheel_sensitivity as f32,
            discrete_scroll: settings.discrete_scroll,
            toggle_crouch: settings.toggle_crouch,
            toggle_sprint: settings.toggle_sprint,
            auto_jump: settings.auto_jump,
        }
    }
    /// Whether a shader pack is currently loaded and active (`IrisApi.isShaderPackInUse()`).
    /// Returns `false` while the panorama shows (the title screen, waiting on a server, or a
    /// screen that asks for it) even if a loaded pack remains cached, matching the renderer,
    /// which suppresses the pack whenever `frame.title` is set.
    pub(in crate::game) fn shader_pack_active(&self) -> bool {
        match self.client.renderer.0.as_ref() {
            Some(renderer) => {
                shader_pack_in_use(self.shows_panorama(), renderer.shader_pack_kind())
            }
            // While `Renderer::render` has the renderer out, the frame's own state says it.
            None => self.shader_pack_state().is_some(),
        }
    }
    /// The active shader pack's cloud and shadow-pass state, as the latest frame's render
    /// callbacks read it; `None` while the panorama shows even when a pack remains loaded.
    pub(in crate::game) fn shader_pack_state(&self) -> Option<ShaderPackState> {
        pack_state_in_view(self.shows_panorama(), self.client.pack_frame_state.get())
    }
    /// Whether the active shader pack bound the mod pipeline in `slot` to one of its programs.
    pub(in crate::game) fn pack_slot_bound(&self, slot: usize) -> bool {
        self.shader_pack_state().is_some() && self.client.pack_frame_slots.borrow().contains(&slot)
    }
    /// How the active translated shader pack binds mod pipeline `slot`: the renderer's own answer,
    /// or while `Renderer::render` has it out, the frame snapshot's.
    pub(in crate::game) fn pack_override_state(&self, slot: usize) -> mod_api::PackOverrideState {
        if let Some(renderer) = self.client.renderer.0.as_ref() {
            return renderer.pack_override_state(slot);
        }
        if !self
            .mod_gpu()
            .is_some_and(sunrise_client_render::gfx::ModGpuResources::pack_iris)
        {
            mod_api::PackOverrideState::NoPack
        } else if self.client.pack_frame_slots.borrow().contains(&slot) {
            mod_api::PackOverrideState::Bound
        } else {
            mod_api::PackOverrideState::UnboundPack
        }
    }
}

/// Pure lifecycle rule for [`ClientGame::shader_pack_state`]: the same as `shader_pack_in_use`'s.
pub(in crate::game) fn pack_state_in_view(
    on_title_screen: bool,
    state: Option<ShaderPackState>,
) -> Option<ShaderPackState> {
    state.filter(|_| !on_title_screen)
}

/// Pure lifecycle rule for [`ClientGame::shader_pack_active`]: a loaded pack is inactive while the
/// panorama shows (the renderer suppresses it via `!frame.title` rather than dropping it when
/// leaving a world).
pub(in crate::game) fn shader_pack_in_use(
    on_title_screen: bool,
    loaded_pack: Option<&str>,
) -> bool {
    !on_title_screen && loaded_pack.is_some()
}

/// Convert live client settings into the API-owned GUI snapshot.
pub(in crate::game) fn gui_options_snapshot(
    s: &sunrise_client::settings::Settings,
) -> ClientGuiOptions {
    ClientGuiOptions {
        gui_scale: s.gui_scale,
        attack_indicator: s.attack_indicator,
        main_hand_left: s.main_hand_left,
        reduced_debug_info: s.reduced_debug_info,
        advanced_item_tooltips: s.file.bool("advancedItemTooltips"),
        high_contrast: s.high_contrast,
        show_subtitles: s.show_subtitles,
        notification_display_time: s.notification_display_time as f32,
        text_background_opacity: s.text_background_opacity as f32,
        background_for_chat_only: s.file.bool("backgroundForChatOnly"),
        menu_blur: s.menu_blur,
        screen_effect_scale: s.file.double("screenEffectScale") as f32,
        // The factor the engine's own hurt tilt reads (`fov.rs`, `bob_hurt`).
        damage_tilt_strength: s.file.double("damageTiltStrength"),
    }
}

/// `Window.getGuiScale` for a framebuffer (`None` without a window): the
/// window's own `GuiSize` resolution, the same one it draws with.
pub(in crate::game) fn resolved_gui_scale(
    framebuffer: Option<[u32; 2]>,
    gui_scale: u32,
    unicode: bool,
) -> Option<u32> {
    framebuffer.map(|size| sunrise_client::window::GuiSize::new(size, gui_scale, unicode).scale)
}
