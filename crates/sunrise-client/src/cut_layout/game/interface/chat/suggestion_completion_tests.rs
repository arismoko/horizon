use crate::game::{Audio, Settings, load_resources};
use mod_api::{self, ChatSuggestionAppliedEvent, ClientHost, Hooks};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use sunrise_client::chat::ChatScreen;
use sunrise_client::chat::Commands;
use sunrise_client::chat::{ChatAbilities, ChatVisibility};
use sunrise_client::screen_draw::Measure;
#[cfg(test)]
use sunrise_client_gameplay::game::client_side::chat::chat_tree;
use sunrise_gameplay_common::game::Screen;
use winit::keyboard::KeyCode;
use {sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress};

/// Exercise the actual Game keyboard/mouse handlers and generated mod dispatch.
#[test]
#[ignore = "daily"]
fn chat_completion_reaches_mods_from_keyboard_and_mouse() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::client_side::chat::suggestion_completion_tests::chat_completion_reaches_mods_from_keyboard_and_mouse",
        chat_completion_reaches_mods_from_keyboard_and_mouse_isolated,
    );
}

fn chat_completion_reaches_mods_from_keyboard_and_mouse_isolated() {
    let root = PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let scratch = PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap()).join("suggestion-game");
    std::fs::create_dir_all(&scratch).unwrap();
    struct RestoreDir(PathBuf);
    impl Drop for RestoreDir {
        fn drop(&mut self) {
            std::env::set_current_dir(&self.0).unwrap();
        }
    }
    let _restore = RestoreDir(std::env::current_dir().unwrap());
    std::env::set_current_dir(&scratch).unwrap();
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root;
    let mut game = sunrise_client_gameplay::game::ClientGame::new(
        &loaded.resources,
        loaded.client_res.unwrap(),
        loaded.pack,
        loaded.client_pack.unwrap(),
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            scratch.clone(),
        )),
        None,
        sunrise_gameplay::game::server_side::integrated::serve,
        sunrise_gameplay::game::server_side::integrated::run_component_harness_on_server,
        crate::game::session_verifier(),
        crate::game::profile_resolver,
    )
    .unwrap();

    struct Listener(Arc<Mutex<Vec<String>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn chat_suggestion_applied(
            &self,
            _host: &dyn ClientHost,
            e: &mut ChatSuggestionAppliedEvent<'_>,
        ) {
            assert_eq!(e.original, "😀 hi st");
            assert_eq!((e.start, e.end), (6, 8));
            assert_eq!(e.input, format!("😀 hi {}", e.suggestion));
            self.0.lock().unwrap().push(e.input.to_owned());
        }
    }
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "listener",
        Box::new(Listener(calls.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        game.client.screen = Screen::Chat;
        game.ui.registries.players = vec!["Steve".into(), "Steven".into()];
        game.client.chat_ui.chat_screen = Some(ChatScreen::new(
            &Measure(&game.client.gui.atlas.font),
            &Commands {
                dispatcher: chat_tree(&game.client.remote),
                registries: &game.ui.registries,
            },
            "",
            false,
            0,
            640,
            360,
            ChatAbilities::new(ChatVisibility::Full),
        ));
        game.chat_type("😀 hi st");
        game.chat_key_code(KeyCode::Tab, None);
        assert!(calls.lock().unwrap().is_empty(), "only opened the list");
        game.chat_key_code(KeyCode::Tab, None);
        game.chat_key_code(KeyCode::Tab, None);
        assert_eq!(*calls.lock().unwrap(), ["😀 hi Steve", "😀 hi Steven"]);

        // Click the suggestion's rendered text rather than assuming its coordinates.
        let draws = game.client.chat_ui.chat_screen.as_mut().unwrap().draw(
            &Measure(&game.client.gui.atlas.font),
            (-1, -1),
            0,
            0,
        );
        let at = draws
            .into_iter()
            .find_map(|d| match d {
                sunrise_client::screen_draw::Draw::Text { text, x, y, .. }
                    if text.plain() == "Steve" =>
                {
                    Some(((x + 1) as f32, (y + 1) as f32))
                }
                _ => None,
            })
            .expect("Steve is a visible suggestion");
        game.client.cursor = at;
        game.chat_click();
        assert_eq!(
            *calls.lock().unwrap(),
            ["😀 hi Steve", "😀 hi Steven", "😀 hi Steve"]
        );
        assert!(
            game.client
                .chat_ui
                .chat_screen
                .as_mut()
                .unwrap()
                .take_suggestion_completion()
                .is_none()
        );
        game.chat_key_code(KeyCode::ArrowDown, None);
        assert_eq!(calls.lock().unwrap().len(), 3, "navigation does not apply");
    });
}
