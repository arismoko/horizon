#[cfg(test)]
pub(in crate::game) mod tests {
    use crate::game::client_side::native_hooks_client::client_options::{
        gui_options_snapshot, pack_state_in_view, resolved_gui_scale, shader_pack_in_use,
    };
    use mod_api::ShaderPackState;

    /// A frame callback's VSync change, while `Renderer::render` has the renderer out, waits for
    /// the renderer to come back instead of being lost; outside a frame there is nothing to
    /// wait for (#5864).
    #[test]
    #[ignore = "daily"]
    fn a_vsync_change_during_a_frame_waits_for_the_renderer() {
        use mod_api::prelude::ClientOptionsClientHost;
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            assert!(game.client.renderer.0.is_none());
            ClientOptionsClientHost::set_session_vsync(game, Some(false));
            assert!(
                !game.client.present_mode_pending,
                "no frame runs: nothing waits"
            );
            game.client
                .render_snapshot
                .set(Some(crate::game::RenderSnapshot {
                    framebuffer: [800, 600],
                    prepared_generation: None,
                    staged_prepared_draws: 0,
                }));
            ClientOptionsClientHost::set_session_vsync(game, Some(true));
            assert!(game.client.present_mode_pending);
            assert!(ClientOptionsClientHost::graphics_options(game).vsync);
            game.client.render_snapshot.set(None);
            game.apply_pending_present_mode();
            assert!(
                !game.client.present_mode_pending,
                "applied once the frame ends"
            );
        });
    }

    /// The creative inventory's exclusion areas (its tab rows) are read during a frame, while
    /// `Renderer::render` has the renderer out, from the framebuffer it left behind (#5864).
    #[test]
    #[ignore = "daily"]
    fn screen_exclusion_is_read_during_a_frame() {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            game.client.screen = sunrise_gameplay_common::game::Screen::Creative;
            assert!(game.screen_exclusion().is_empty(), "no window, no screen");
            game.client
                .render_snapshot
                .set(Some(crate::game::RenderSnapshot {
                    framebuffer: [800, 600],
                    prepared_generation: None,
                    staged_prepared_draws: 0,
                }));
            let batch = sunrise_client::hud::HudBatch::new(
                [800, 600],
                game.client.settings.gui_scale_inputs(),
            );
            let own = game.screen_own_exclusion(&batch);
            assert!(!own.is_empty());
            assert_eq!(game.screen_exclusion(), own);
        });
    }

    /// Session-only VSync, clouds and view bobbing take effect in the host's reads and never reach
    /// the player's options or `options.txt`; `None` goes back to the option. No renderer runs
    /// here (no GPU): the surface reconfiguration is the same `set_present_mode` call the options
    /// screen makes when VSync changes.
    #[test]
    #[ignore = "daily"]
    fn session_video_options_take_effect_without_being_saved() {
        use mod_api::{CloudStatus, prelude::ClientOptionsClientHost};
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            game.client.settings.vsync = true;
            game.client.settings.clouds = "true".into();
            game.client.settings.view_bobbing = true;
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
            assert!(game.client.settings.vsync);
            assert_eq!(game.client.settings.clouds, "true");
            assert!(game.client.settings.view_bobbing);
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("options.txt");
            game.client.settings.save(&path);
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
