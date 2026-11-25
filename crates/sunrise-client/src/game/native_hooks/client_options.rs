use super::*;

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Sunrise does.
impl Game<'_> {
    /// A snapshot of the current graphics preferences.
    pub(in crate::game) fn graphics_options(&self) -> ClientGraphicsOptions {
        let s = &self.settings;
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
        self.session_video.vsync.unwrap_or(self.settings.vsync)
    }
    /// The Clouds option in effect, as `options.txt` names it (`false`, `fast`, `true`).
    pub(in crate::game) fn effective_clouds(&self) -> &str {
        self.session_video
            .clouds
            .unwrap_or(self.settings.clouds.as_str())
    }
    /// `Options.enableVsync` for this session only, and the surface reconfigured at once.
    pub(in crate::game) fn set_session_vsync(&mut self, vsync: Option<bool>) {
        self.session_video.vsync = vsync;
        let vsync = self.effective_vsync();
        if let Some(renderer) = &mut self.renderer {
            renderer.set_present_mode(vsync, self.settings.sunrise.present_mode);
        }
    }
    /// `Options.cloudStatus` for this session only.
    pub(in crate::game) fn set_session_clouds(&mut self, clouds: Option<mod_api::CloudStatus>) {
        self.session_video.clouds = clouds.map(|clouds| match clouds {
            mod_api::CloudStatus::Off => "false",
            mod_api::CloudStatus::Fast => "fast",
            mod_api::CloudStatus::Fancy => "true",
        });
    }
    /// View bobbing in effect: the session's (`set_session_view_bobbing`), else the player's
    /// option (`Options.bobView`).
    pub(in crate::game) fn effective_view_bobbing(&self) -> bool {
        self.session_video
            .view_bobbing
            .unwrap_or(self.settings.view_bobbing)
    }
    /// `Options.bobView` for this session only; the next frame's camera reads it.
    pub(in crate::game) fn set_session_view_bobbing(&mut self, view_bobbing: Option<bool>) {
        self.session_video.view_bobbing = view_bobbing;
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
        gui_options_snapshot(&self.settings)
    }
    /// The GUI scale in effect (`Window.getGuiScale`): the scale the window
    /// applies from the GUI Scale option and its size, at least 1; none on a
    /// server or when no window exists. `gui_options().gui_scale` is only the
    /// option, where 0 means automatic.
    pub(in crate::game) fn gui_scale(&self) -> Option<u32> {
        // The size the renderer applies, as the drawing code reads it: it keeps its previous
        // configuration when the window reports a size it cannot draw at.
        let framebuffer = self
            .window
            .as_ref()
            .and_then(|_| self.renderer_framebuffer());
        resolved_gui_scale(
            framebuffer,
            self.settings.gui_scale,
            self.settings.force_unicode,
        )
    }
    /// `Window.getWidth` and `getHeight`: the window's framebuffer pixels, the size
    /// `window_resize` reports; none on a server or when no window exists.
    pub(in crate::game) fn window_size(&self) -> Option<[u32; 2]> {
        let size = self.window.as_ref()?.inner_size();
        Some([size.width, size.height])
    }
    /// A snapshot of the current chat preferences.
    pub(in crate::game) fn chat_options(&self) -> ClientChatOptions {
        let s = &self.settings;
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
            master_volume: self.settings.master_volume as f32,
        }
    }
    /// Whether a shader pack is currently loaded and active (`IrisApi.isShaderPackInUse()`).
    /// Returns `false` while the panorama shows (the title screen, waiting on a server, or a
    /// screen that asks for it) even if a loaded pack remains cached, matching the renderer,
    /// which suppresses the pack whenever `frame.title` is set.
    pub(in crate::game) fn shader_pack_active(&self) -> bool {
        match self.renderer.as_ref() {
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
        pack_state_in_view(self.shows_panorama(), self.pack_frame_state.get())
    }
    /// Whether the active shader pack bound the mod pipeline in `slot` to one of its programs.
    pub(in crate::game) fn pack_slot_bound(&self, slot: usize) -> bool {
        self.shader_pack_state().is_some() && self.pack_frame_slots.borrow().contains(&slot)
    }
    /// How the active translated shader pack binds mod pipeline `slot`: the renderer's own answer,
    /// or while `Renderer::render` has it out, the frame snapshot's.
    pub(in crate::game) fn pack_override_state(&self, slot: usize) -> mod_api::PackOverrideState {
        if let Some(renderer) = self.renderer.as_ref() {
            return renderer.pack_override_state(slot);
        }
        match self.render_snapshot.get() {
            Some(snapshot) if snapshot.pack_iris => {
                if self.pack_frame_slots.borrow().contains(&slot) {
                    mod_api::PackOverrideState::Bound
                } else {
                    mod_api::PackOverrideState::UnboundPack
                }
            }
            _ => mod_api::PackOverrideState::NoPack,
        }
    }
}

/// Pure lifecycle rule for [`Game::shader_pack_state`]: the same as `shader_pack_in_use`'s.
fn pack_state_in_view(
    on_title_screen: bool,
    state: Option<ShaderPackState>,
) -> Option<ShaderPackState> {
    state.filter(|_| !on_title_screen)
}

/// Pure lifecycle rule for [`Game::shader_pack_active`]: a loaded pack is inactive while the
/// panorama shows (the renderer suppresses it via `!frame.title` rather than dropping it when
/// leaving a world).
fn shader_pack_in_use(on_title_screen: bool, loaded_pack: Option<&str>) -> bool {
    !on_title_screen && loaded_pack.is_some()
}

/// Convert live client settings into the API-owned GUI snapshot.
fn gui_options_snapshot(s: &sunrise_client::settings::Settings) -> ClientGuiOptions {
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
fn resolved_gui_scale(framebuffer: Option<[u32; 2]>, gui_scale: u32, unicode: bool) -> Option<u32> {
    framebuffer.map(|size| sunrise_client::window::GuiSize::new(size, gui_scale, unicode).scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Session-only VSync, clouds and view bobbing take effect in the host's reads and never reach
    /// the player's options or `options.txt`; `None` goes back to the option. No renderer runs
    /// here (no GPU): the surface reconfiguration is the same `set_present_mode` call the options
    /// screen makes when VSync changes.
    #[test]
    #[ignore = "daily"]
    fn session_video_options_take_effect_without_being_saved() {
        use mod_api::{CloudStatus, prelude::ClientOptionsClientHost};
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            game.settings.vsync = true;
            game.settings.clouds = "true".into();
            game.settings.view_bobbing = true;
            assert!(ClientOptionsClientHost::graphics_options(game).vsync);
            assert!(ClientOptionsClientHost::graphics_options(game).view_bobbing);
            assert_eq!(
                ClientOptionsClientHost::cloud_status(game),
                CloudStatus::Fancy
            );

            ClientOptionsClientHost::set_session_vsync(game, Some(false));
            ClientOptionsClientHost::set_session_clouds(game, Some(CloudStatus::Fast));
            ClientOptionsClientHost::set_session_view_bobbing(game, Some(false));
            assert!(!ClientOptionsClientHost::graphics_options(game).vsync);
            assert!(!ClientOptionsClientHost::graphics_options(game).view_bobbing);
            assert_eq!(
                ClientOptionsClientHost::cloud_status(game),
                CloudStatus::Fast
            );
            assert!(game.settings.vsync);
            assert_eq!(game.settings.clouds, "true");
            assert!(game.settings.view_bobbing);
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("options.txt");
            game.settings.save(&path);
            let saved = std::fs::read_to_string(&path).unwrap();
            assert!(
                saved.lines().any(|line| line == "enableVsync:true"),
                "{saved}"
            );
            assert!(
                saved.lines().any(|line| {
                    line.strip_prefix("renderClouds:")
                        .is_some_and(|value| value.trim_matches('"') == "true")
                }),
                "{saved}"
            );
            assert!(saved.lines().any(|line| line == "bobView:true"), "{saved}");

            ClientOptionsClientHost::set_session_clouds(game, Some(CloudStatus::Off));
            assert_eq!(
                ClientOptionsClientHost::cloud_status(game),
                CloudStatus::Off
            );
            ClientOptionsClientHost::set_session_vsync(game, None);
            ClientOptionsClientHost::set_session_clouds(game, None);
            ClientOptionsClientHost::set_session_view_bobbing(game, None);
            assert!(ClientOptionsClientHost::graphics_options(game).vsync);
            assert!(ClientOptionsClientHost::graphics_options(game).view_bobbing);
            assert_eq!(
                ClientOptionsClientHost::cloud_status(game),
                CloudStatus::Fancy
            );
        });
    }

    #[test]
    fn gui_snapshot_tracks_advanced_tooltip_toggle() {
        let mut settings = sunrise_client::settings::Settings::default();
        for enabled in [false, true, false] {
            settings.file.set(
                "advancedItemTooltips",
                sunrise_client::options_file::V::Bool(enabled),
            );
            assert_eq!(
                gui_options_snapshot(&settings).advanced_item_tooltips,
                enabled
            );
        }
        settings.file.set(
            "screenEffectScale",
            sunrise_client::options_file::V::Double(0.25),
        );
        assert_eq!(gui_options_snapshot(&settings).screen_effect_scale, 0.25);
    }

    /// The snapshot carries the Damage Tilt option as the player sets it, off and full included.
    #[test]
    fn gui_snapshot_follows_the_damage_tilt_option() {
        let mut settings = sunrise_client::settings::Settings::default();
        assert_eq!(gui_options_snapshot(&settings).damage_tilt_strength, 1.0);
        for strength in [0.0, 0.35, 1.0] {
            settings.file.set(
                "damageTiltStrength",
                sunrise_client::options_file::V::Double(strength),
            );
            assert_eq!(
                gui_options_snapshot(&settings).damage_tilt_strength,
                strength
            );
        }
    }

    /// `Options.getBackgroundColor` (#5653): with "Text Background: Chat only" on (the default)
    /// a backdrop keeps its own default opacity; off, it takes the player's opacity. The
    /// snapshot's resolvers give the colour the engine's own text backdrops use.
    #[test]
    fn gui_snapshot_resolves_the_text_background_as_vanilla_does() {
        let mut settings = sunrise_client::settings::Settings {
            text_background_opacity: 0.6,
            ..Default::default()
        };
        let gui = gui_options_snapshot(&settings);
        assert!(gui.background_for_chat_only);
        assert_eq!(gui.background_opacity(0.4), 0.4);
        assert_eq!(gui.background_color(0.4), 0x6600_0000);
        assert_eq!(gui.background_color(0.4), settings.text_background(0.4));
        settings.file.set(
            "backgroundForChatOnly",
            sunrise_client::options_file::V::Bool(false),
        );
        let gui = gui_options_snapshot(&settings);
        assert!(!gui.background_for_chat_only);
        assert_eq!(gui.background_opacity(0.4), 0.6);
        assert_eq!(gui.background_color(0.0), 0x9900_0000);
        assert_eq!(gui.background_color(0.0), settings.text_background(0.0));
    }

    /// The literal `Window.calculateScale` outcomes the issue asks for: the
    /// option is a maximum, and the 320x240 floor wins over it.
    #[test]
    fn gui_scale_resolves_the_option_and_the_window() {
        assert_eq!(resolved_gui_scale(Some([1280, 720]), 0, false), Some(3));
        assert_eq!(resolved_gui_scale(Some([1920, 1080]), 0, false), Some(4));
        assert_eq!(resolved_gui_scale(Some([1920, 1080]), 2, false), Some(2));
        assert_eq!(resolved_gui_scale(Some([1280, 720]), 5, false), Some(3));
        assert_eq!(resolved_gui_scale(Some([320, 240]), 0, false), Some(1));
        // The Unicode font raises an odd resolved scale by one.
        assert_eq!(resolved_gui_scale(Some([1920, 1080]), 3, true), Some(4));
    }

    /// A window the renderer did not resize (an undrawable size keeps its previous
    /// configuration) resolves from the renderer's 1280x720, not the window's own
    /// 1920x1080, which alone would give 4.
    #[test]
    fn gui_scale_follows_the_size_the_renderer_applies() {
        assert_eq!(resolved_gui_scale(Some([1280, 720]), 0, false), Some(3));
        assert_eq!(resolved_gui_scale(Some([1920, 1080]), 0, false), Some(4));
    }

    #[test]
    fn gui_scale_needs_a_window() {
        assert_eq!(resolved_gui_scale(None, 0, false), None);
        assert_eq!(resolved_gui_scale(None, 4, true), None);
    }

    #[test]
    fn shader_pack_active_is_false_on_title_screen_even_when_pack_remains_loaded() {
        assert!(!shader_pack_in_use(false, None));
        assert!(!shader_pack_in_use(true, None));
        assert!(!shader_pack_in_use(true, Some("iris")));
        assert!(shader_pack_in_use(false, Some("iris")));
    }

    #[test]
    fn shader_pack_state_is_none_on_title_screen_and_keeps_the_frames_values_in_world() {
        let state = ShaderPackState::new(true, None, 7);
        assert_eq!(pack_state_in_view(true, Some(state)), None);
        assert_eq!(pack_state_in_view(false, None), None);
        let read = pack_state_in_view(false, Some(state)).expect("in world");
        assert!(read.hides_clouds);
        assert_eq!(read.generation, 7);
    }
}
