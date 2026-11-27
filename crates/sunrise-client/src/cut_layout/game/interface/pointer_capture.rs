#![cfg(test)]
use crate::audio::Audio;
use crate::game::load_resources;
use crate::settings::Settings;
use std::path::PathBuf;
use std::time::Instant;
use sunrise_gameplay_common::game::execution_role;
use sunrise_gameplay_common::game::{Options, Progress, Screen};
use winit::keyboard::KeyCode;
#[cfg(test)]
pub(crate) mod tests {

    #[test]
    #[ignore = "daily"]
    fn movement_key_held_behind_a_screen_counts_after_pointer_grab() {
        use super::*;
        use sunrise_assets::resource_source::{ResourceRoot, ResourceSource};

        let root = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            ResourceRoot::directory(root.clone()),
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
        let mut game = sunrise_client_gameplay::game::ClientGame::new(
            &loaded.resources,
            loaded.client_res.unwrap(),
            loaded.pack,
            loaded.client_pack.unwrap(),
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&ResourceSource::directory(root)),
            None,
            sunrise_gameplay::game::server_side::integrated::serve,
            sunrise_gameplay::game::server_side::integrated::run_component_harness_on_server,
            crate::game::session_verifier(),
            crate::game::profile_resolver,
        )
        .unwrap();

        game.client.focused = true;
        game.client.screen = Screen::Paused;
        game.client.physical_keys.insert(KeyCode::KeyW);
        assert!(!game.movement_key_held(KeyCode::KeyW));

        game.client.screen = Screen::Playing;
        game.capture(true);
        assert_eq!(
            game.movement_key_held(KeyCode::KeyW),
            !cfg!(target_os = "macos")
        );
    }

    /// `MouseHandler.releaseMouse` and `grabMouse` through the mod host's pair: releasing drops
    /// the grab, grabbing an inactive window does nothing, and grabbing with a screen open
    /// closes it and takes the pointer.
    #[test]
    #[ignore = "daily"]
    fn mod_host_releases_and_grabs_the_pointer_as_vanilla_does() {
        use super::*;
        use mod_api::prelude::ClientInputClientHost;
        use sunrise_assets::resource_source::{ResourceRoot, ResourceSource};

        let root = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            ResourceRoot::directory(root.clone()),
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
        options.offscreen = true;
        let mut game = sunrise_client_gameplay::game::ClientGame::new(
            &loaded.resources,
            loaded.client_res.unwrap(),
            loaded.pack,
            loaded.client_pack.unwrap(),
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&ResourceSource::directory(root)),
            None,
            sunrise_gameplay::game::server_side::integrated::serve,
            sunrise_gameplay::game::server_side::integrated::run_component_harness_on_server,
            crate::game::session_verifier(),
            crate::game::profile_resolver,
        )
        .unwrap();
        let (_event_loop, window) =
            sunrise_client_gameplay::game::client_side::native_entity_refs::tests::test_window([
                640, 480,
            ]);
        *game.test_window_mut() = Some(window);
        game.client.title.0 = None;
        game.client.screen = Screen::Playing;
        game.common.player = Some(
            sunrise_sim::motion::PlayerMotion::new(
                game.common.motion,
                sunrise_core::geometry::Vec3::ZERO,
            )
            .unwrap(),
        );

        // Release while playing clears the grab.
        game.client.focused = true;
        game.client.captured = true;
        assert!(ClientInputClientHost::is_mouse_grabbed(&game));
        ClientInputClientHost::set_mouse_grabbed(&mut game, false);
        assert!(!ClientInputClientHost::is_mouse_grabbed(&game));

        // Grab while the window is inactive stays released.
        game.client.focused = false;
        ClientInputClientHost::set_mouse_grabbed(&mut game, true);
        assert!(!ClientInputClientHost::is_mouse_grabbed(&game));

        // Grab with no player in a world does nothing: `setScreen(null)` opens the title.
        game.client.focused = true;
        game.client.screen = Screen::Paused;
        let player = game.common.player.take();
        ClientInputClientHost::set_mouse_grabbed(&mut game, true);
        assert!(!ClientInputClientHost::is_mouse_grabbed(&game));
        assert!(game.client.screen == Screen::Paused);
        game.common.player = player;

        // Grab on the death screen does nothing: `setScreen(null)` keeps the death screen.
        game.client.screen = Screen::Dead;
        ClientInputClientHost::set_mouse_grabbed(&mut game, true);
        assert!(!ClientInputClientHost::is_mouse_grabbed(&game));
        assert!(game.client.screen == Screen::Dead);

        // Grab with a container screen open closes the container and the screen, takes the
        // pointer, and sets `missTime`.
        game.client.screen = Screen::Inventory;
        game.common.craft_size = 3;
        game.common.clicks.miss_time = 0;
        ClientInputClientHost::set_mouse_grabbed(&mut game, true);
        assert!(game.client.screen == Screen::Playing);
        assert_eq!(game.common.craft_size, 2);
        assert!(ClientInputClientHost::is_mouse_grabbed(&game));
        assert_eq!(game.common.clicks.miss_time, 10000);
    }
}
