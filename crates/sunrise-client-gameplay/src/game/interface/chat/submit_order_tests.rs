use super::*;
use mod_api::{ClientChatEvent, ClientHost, Flow, Hooks, ModEntry, ScreenClosingEvent};
use std::sync::{Arc, Mutex};
use sunrise_protocol::connection::Pipe;

struct ChatOrder(Arc<Mutex<Vec<String>>>);
#[mod_api::__subscriptions]
impl Hooks for ChatOrder {
    fn client_chat(&self, _: &mut dyn ClientHost, event: &mut ClientChatEvent<'_>) -> Flow {
        self.0
            .lock()
            .unwrap()
            .push(format!("chat {}", event.message));
        Flow::Continue
    }
    fn screen_closing(&self, _: &dyn ClientHost, event: &mut ScreenClosingEvent<'_>) {
        self.0
            .lock()
            .unwrap()
            .push(format!("closing {}", event.screen.id));
    }
}

#[test]
#[ignore = "daily"]
fn enter_handles_the_line_before_the_chat_screen_closes() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new(
        "chat-order",
        Box::new(ChatOrder(log.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        crate::game::client_side::native_entity_refs::tests::with_client_game(|game| {
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "ChatOrderTest").unwrap();
            let remote = game.client.remote.as_mut().unwrap();
            remote.joined = true;
            remote.has_logical_player = true;
            game.player = Some(
                sunrise_sim::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::ZERO,
                )
                .unwrap(),
            );
            game.client.screen = Screen::Playing;
            game.open_chat_method("");
            assert!(game.client.screen == Screen::Chat);
            game.chat_key_code(KeyCode::KeyH, Some("hello"));
            log.lock().unwrap().clear();
            assert!(game.chat_key_code(KeyCode::Enter, None));
            assert!(game.client.screen == Screen::Playing);
            assert!(game.client.chat_ui.chat_screen.is_none());
        });
    });
    assert_eq!(*log.lock().unwrap(), ["chat hello", "closing chat"]);
}
