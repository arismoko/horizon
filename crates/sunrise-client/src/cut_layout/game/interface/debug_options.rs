#![cfg(test)]
use std::time::Instant;
use sunrise_client::hud::HudBatch;
use sunrise_gameplay_common::game::execution_role;
use winit::keyboard::KeyCode;
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_client_gameplay::game::client_side::debug_options::footer;

    use super::*;
    use crate::game::load_resources;
    use crate::settings::Settings;
    use sunrise_client::audio::Audio;
    use sunrise_client::debug_entries::Profile;
    use sunrise_client::debug_entries::Status;
    use sunrise_gameplay_common::game::Screen;
    use sunrise_gameplay_common::game::launch_options::Options;
    use sunrise_gameplay_common::game::resource_load::Progress;

    fn title_game<'a>(
        resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
    ) -> sunrise_client_gameplay::game::ClientGame<'a> {
        let root = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES").expect("prepared resources required"),
        );
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            execution_role::ExecutionRole::Client,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = execution_role::ExecutionRole::Client;
        let res = resources.insert(loaded.resources);
        let mut game = sunrise_client_gameplay::game::ClientGame::new(
            res,
            loaded.client_res.unwrap(),
            loaded.pack,
            loaded.client_pack.unwrap(),
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
            sunrise_gameplay::game::server_side::integrated::serve,
            sunrise_gameplay::game::server_side::integrated::run_component_harness_on_server,
            crate::game::session_verifier(),
            crate::game::profile_resolver,
        )
        .unwrap();
        game.client.title.0 = Some(game.new_title(std::path::PathBuf::from("saves")));
        game
    }
    #[test]
    #[ignore = "daily"]
    fn title_debug_overlay_draws_without_a_level() {
        run_title_debug_overlay_draws_without_a_level();
    }
    pub(crate) fn run_title_debug_overlay_draws_without_a_level() {
        use mod_api::prelude::ScreensClientHost;
        use sunrise_client::debug_keys::KeyAction;
        let mut resources = None;
        let mut game = title_game(&mut resources);
        assert!(
            game.player.is_none()
                && game.client.renderer.0.is_none()
                && game.client.integrated.is_none()
        );
        game.debug_key(KeyCode::F3, KeyAction::Press);
        game.debug_key(KeyCode::F3, KeyAction::Release);
        assert!(
            game.client.debug,
            "the ordinary F3 toggle already reaches the title"
        );
        assert!(ScreensClientHost::debug_overlay_visible(&game));
        let mut overlay = HudBatch::new([1280, 720], game.client.settings.gui_scale_inputs());
        game.draw_debug_overlay_under(&mut overlay, [1280, 720]);
        assert!(
            !overlay.vertices.is_empty(),
            "F3 must paint even without a level"
        );
        assert!(game.client.title.0.is_some());
        // Gui treats the title as an open screen even when F1 hides the HUD.
        game.client.chat_ui.hide_gui = true;
        let mut hidden_hud = HudBatch::new([1280, 720], game.client.settings.gui_scale_inputs());
        game.draw_debug_overlay_under(&mut hidden_hud, [1280, 720]);
        assert!(!hidden_hud.vertices.is_empty());
        game.debug_key(KeyCode::F3, KeyAction::Press);
        game.debug_key(KeyCode::F3, KeyAction::Release);
        assert!(!game.client.debug);
        game.client
            .debug_entries
            .0
            .set_status("fps", Status::AlwaysOn);
        assert!(ScreensClientHost::debug_overlay_visible(&game));

        // With no screen open, the hidden HUD suppresses even an always-on entry.
        game.client.title.0 = None;
        game.client.screen = Screen::Playing;
        assert!(!ScreensClientHost::debug_overlay_visible(&game));
        game.client.chat_ui.hide_gui = false;
        assert!(ScreensClientHost::debug_overlay_visible(&game));
    }
    #[test]
    #[ignore = "daily"]
    fn title_debug_options_open_draw_and_return_without_a_level() {
        run_title_debug_options_open_draw_and_return_without_a_level();
    }
    pub(crate) fn run_title_debug_options_open_draw_and_return_without_a_level() {
        use sunrise_client::debug_keys::KeyAction;
        let mut resources = None;
        let mut game = title_game(&mut resources);
        let original_title = game.client.title.0.as_ref().unwrap().screen_key();
        assert!(!game.client.captured);
        game.debug_key(KeyCode::F3, KeyAction::Press);
        game.debug_key(KeyCode::F6, KeyAction::Press);
        assert!(
            game.client.screen == Screen::DebugOptions,
            "F3+F6 must open over the title"
        );
        assert!(game.client.debug_options.is_some());
        // RedrawRequested only updates the world, spawns a player and ticks
        // gameplay when title.is_none(). Keep that no-level sentinel intact.
        assert!(
            game.client.title.0.is_some(),
            "F6 must not enable the world-entry gates"
        );
        assert!(
            !game.title_input_active(),
            "the covered title must not take input"
        );
        assert!(
            game.shows_panorama(),
            "the level-less options retain the panorama"
        );
        assert!(
            game.player.is_none()
                && game.client.renderer.0.is_none()
                && game.client.integrated.is_none()
        );
        let frame = game.hud([1280, 720]);
        assert!(
            !frame.vertices.is_empty(),
            "the actual debug options screen must paint"
        );
        assert!(
            game.client.title.0.is_some(),
            "drawing options must keep world entry disabled"
        );
        sunrise_client::widgets::end_frame(1);
        assert!(
            sunrise_client::widgets::last().iter().any(|w| {
                w.label == sunrise_game_data::lang::get(Profile::Performance.translation_key())
            }),
            "debug options buttons, rather than title buttons, must be visible"
        );
        game.client.debug_options.as_mut().unwrap().search.focused = true;
        assert!(
            game.text_field_focused(),
            "the covered title must not hide search focus"
        );
        game.ime_commit("fps");
        assert_eq!(
            game.client.debug_options.as_ref().unwrap().search.value,
            "fps"
        );
        game.client.debug_options.as_mut().unwrap().search.focused = false;
        game.debug_key(KeyCode::F6, KeyAction::Release);
        game.debug_key(KeyCode::F6, KeyAction::Press);
        assert!(game.client.screen == Screen::Playing);
        assert!(game.client.title.0.is_some() && game.client.debug_options.is_none());
        assert_eq!(
            game.client.title.0.as_ref().unwrap().screen_key(),
            original_title
        );
        assert!(
            !game.client.captured,
            "closing a title screen must not capture the pointer"
        );
        game.debug_key(KeyCode::F6, KeyAction::Release);
        game.debug_key(KeyCode::F3, KeyAction::Release);
        assert!(
            !game.client.debug,
            "a used debug combination does not toggle the overlay"
        );
        game.debug_key(KeyCode::F3, KeyAction::Press);
        game.debug_key(KeyCode::F6, KeyAction::Press);
        let (w, h) = game.gui_size();
        let (xs, y) = footer(w, h);
        game.client.cursor = (xs[3] + 1.0, y + 1.0);
        game.debug_options_click();
        assert!(game.client.title.0.is_some() && game.client.debug_options.is_none());
        assert!(
            !game.client.captured,
            "Done also returns without grabbing the pointer"
        );
        assert!(game.client.screen == Screen::Playing);
        game.client.title.0.as_mut().unwrap().mode = sunrise_client::title::Mode::Select {
            worlds: Vec::new(),
            selected: None,
        };
        let selection = game.client.title.0.as_ref().unwrap().screen_key();
        game.debug_key(KeyCode::F6, KeyAction::Release);
        game.debug_key(KeyCode::F6, KeyAction::Press);
        assert!(game.client.debug_options.is_none());
        assert_eq!(
            game.client.title.0.as_ref().unwrap().screen_key(),
            selection
        );
    }
}
