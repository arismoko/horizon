//! Private custom-book connection state and client network services.
use crate::game::ClientGame;
#[path = "native_net/packet_hold.rs"]
pub mod packet_hold;
use mod_api::{SendPacketError, ServerboundPlayPacket};
pub(in crate::game) use packet_hold::PlaySend;

#[cfg(feature = "client")]
impl ClientGame<'_> {
    /// Opens the chat screen with `text` typed in and the cursor at its end,
    /// as T and / open it (`ChatScreen` with its initial text): a command
    /// macro can fill in `/tp ` for the player to finish. Nothing is sent; the
    /// player edits and sends it, or presses Escape. The box keeps its limit
    /// (256 UTF-16 units) and its character filter. The ordinary screen
    /// opening and closing hooks govern the transition. API @API:open_chat@.
    pub fn open_chat(&mut self, text: &str) -> Result<(), String> {
        if self.begin_chat_screen(text, false) {
            Ok(())
        } else {
            Err("no player in a world".into())
        }
    }

    /// Validate and reserve a vanilla packet for root-commit publication on this
    /// exact physical protocol incarnation. Registered and engine-reserved
    /// channels require their payload/service API; unrelated plugin payloads
    /// retain vanilla's discard-codec limit. No local effects are applied. API 409.
    pub fn send_packet(&mut self, packet: ServerboundPlayPacket) -> Result<(), SendPacketError> {
        let remote = self
            .client
            .remote
            .as_ref()
            .ok_or(SendPacketError::NotConnected)?;
        if !remote.joined || remote.reconfiguring.is_some() {
            return Err(SendPacketError::NotInPlay);
        }
        let prepared = remote.session.prepare_mod_packet(packet)?;
        let owner = mod_api::content::crash::current_native_mod();
        self.after_commit(super::transactions::After::Packet(prepared, owner));
        Ok(())
    }
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

#[cfg(feature = "client")]
#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::{run_fresh_cases, with_client_game};
    use super::{SendPacketError, ServerboundPlayPacket};
    use mod_api::Identifier;
    use sunrise_core::vanilla_bytes::Writer;
    use sunrise_gameplay_common::game::Screen;
    use sunrise_protocol::connection::Pipe;

    #[test]
    #[ignore = "daily"]
    fn open_chat_replaces_screens_but_a_veto_keeps_the_old_chat() {
        use mod_api::{
            ClientHost, Hooks, ScreenClosingEvent, ScreenInitPostEvent, ScreenOpening,
            ScreenOpeningEvent,
        };
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        };
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::motion::PlayerMotion;

        if run_fresh_cases(
            "game::client_side::native_net::tests::open_chat_replaces_screens_but_a_veto_keeps_the_old_chat",
            "SUNRISE_OPEN_CHAT_CASE",
            &["transition"],
        ) {
            return;
        }
        #[derive(Default)]
        struct Probe {
            cancel: AtomicBool,
            events: Mutex<Vec<String>>,
        }
        struct Listener(Arc<Probe>);
        #[mod_api::__subscriptions]
        impl Hooks for Listener {
            fn screen_opening(
                &self,
                _: &dyn ClientHost,
                event: &mut ScreenOpeningEvent<'_>,
            ) -> Option<ScreenOpening> {
                self.0
                    .events
                    .lock()
                    .unwrap()
                    .push(format!("{}->{}", event.current.id, event.new.id));
                self.0
                    .cancel
                    .load(Ordering::Relaxed)
                    .then_some(ScreenOpening::Cancel)
            }

            fn screen_closing(&self, _: &dyn ClientHost, event: &mut ScreenClosingEvent<'_>) {
                self.0
                    .events
                    .lock()
                    .unwrap()
                    .push(format!("close {}", event.screen.id));
            }

            fn screen_init_post(&self, _: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
                self.0
                    .events
                    .lock()
                    .unwrap()
                    .push(format!("init {}", event.screen.id));
            }
        }
        let probe = Arc::new(Probe::default());
        let mods = vec![mod_api::ModEntry::new(
            "chat_screens",
            Box::new(Listener(probe.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_client_game(|game| {
                game.player = None;
                assert!(game.open_chat("/tp ").is_err());
                assert!(game.client.chat_ui.chat_screen.is_none());
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "ChatTest").unwrap();
                let (session, mut peer, _keepalive) =
                    sunrise_client::net::Session::packet_hold_probe();
                let remote = game.client.remote.as_mut().unwrap();
                remote.joined = true;
                remote.level_load.finish();
                remote.session = session;
                game.client
                    .render_snapshot
                    .set(Some(crate::game::RenderSnapshot {
                        framebuffer: [800, 600],
                        prepared_generation: None,
                        staged_prepared_draws: 0,
                    }));
                game.player =
                    Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
                game.client.screen = Screen::Paused;
                game.send_unattended_command("unknown_command");
                assert!(game.client.command_prompt.0.is_some());
                probe.events.lock().unwrap().clear();

                probe.cancel.store(true, Ordering::Relaxed);
                assert_eq!(game.open_chat("vetoed"), Ok(()));
                assert!(game.client.command_prompt.0.is_some());
                assert!(game.client.chat_ui.chat_screen.is_none());
                game.observe_screen();
                assert_eq!(
                    std::mem::take(&mut *probe.events.lock().unwrap()),
                    ["command_prompt->chat"]
                );

                probe.cancel.store(false, Ordering::Relaxed);
                assert_eq!(game.open_chat("/tp "), Ok(()));
                assert!(game.client.command_prompt.0.is_none());
                assert!(game.client.screen == Screen::Chat);
                assert_eq!(
                    game.client.chat_ui.chat_screen.as_ref().unwrap().value(),
                    "/tp "
                );
                assert!(!game.screen_over_playing());
                assert_eq!(game.open_chat("second draft"), Ok(()));
                assert_eq!(
                    game.client.chat_ui.chat_screen.as_ref().unwrap().value(),
                    "second draft"
                );
                assert_eq!(
                    game.client.chat_ui.chat_draft.as_ref().unwrap().text,
                    "/tp "
                );
                game.observe_screen();
                assert_eq!(
                    *probe.events.lock().unwrap(),
                    [
                        "command_prompt->chat",
                        "close command_prompt",
                        "init chat",
                        "chat->chat",
                        "close chat",
                        "init chat",
                    ]
                );
                peer.reader
                    .set_read_timeout(Some(std::time::Duration::from_millis(50)))
                    .unwrap();
                assert!(
                    peer.recv().unwrap_err().is_timed_out(),
                    "opening neither answers the prompt nor sends the prefill"
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn waking_with_typed_chat_replaces_it_once_without_saving_a_draft() {
        use mod_api::{
            ClientHost, Hooks, ScreenClosingEvent, ScreenInitPostEvent, ScreenOpening,
            ScreenOpeningEvent,
        };
        use std::sync::{Arc, Mutex};
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::motion::PlayerMotion;

        if run_fresh_cases(
            "game::client_side::native_net::tests::waking_with_typed_chat_replaces_it_once_without_saving_a_draft",
            "SUNRISE_WAKE_CHAT_CASE",
            &["wake"],
        ) {
            return;
        }
        struct Listener(Arc<Mutex<Vec<String>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Listener {
            fn screen_opening(
                &self,
                _: &dyn ClientHost,
                event: &mut ScreenOpeningEvent<'_>,
            ) -> Option<ScreenOpening> {
                self.0
                    .lock()
                    .unwrap()
                    .push(format!("{}->{}", event.current.id, event.new.id));
                None
            }
            fn screen_closing(&self, _: &dyn ClientHost, event: &mut ScreenClosingEvent<'_>) {
                self.0
                    .lock()
                    .unwrap()
                    .push(format!("close {}", event.screen.id));
            }
            fn screen_init_post(&self, _: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
                self.0
                    .lock()
                    .unwrap()
                    .push(format!("init {}", event.screen.id));
            }
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new(
                "wake_chat",
                Box::new(Listener(events.clone())),
            )]
            .into(),
            || {
                with_client_game(|game| {
                    game.player =
                        Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
                    // The tick following wake-up: the existing bed chat still has typed input.
                    assert!(game.begin_chat_screen("typed in bed", false));
                    game.client.chat_ui.in_bed_chat = true;
                    events.lock().unwrap().clear();
                    game.tick_in_bed_chat();
                    game.observe_screen();
                    assert!(game.client.screen == Screen::Chat);
                    assert!(!game.client.chat_ui.in_bed_chat);
                    assert_eq!(
                        game.client.chat_ui.chat_screen.as_ref().unwrap().value(),
                        "typed in bed"
                    );
                    assert!(game.client.chat_ui.chat_draft.is_none());
                    assert_eq!(
                        *events.lock().unwrap(),
                        ["chat->chat", "close chat", "init chat"]
                    );
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn overlay_redirected_to_chat_emits_init_post_once() {
        use mod_api::{
            ClientHost, Hooks, ScreenClosingEvent, ScreenInitPostEvent, ScreenOpening,
            ScreenOpeningEvent, ScreenTarget,
        };
        use std::sync::{Arc, Mutex};
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::motion::PlayerMotion;

        if run_fresh_cases(
            "game::client_side::native_net::tests::overlay_redirected_to_chat_emits_init_post_once",
            "SUNRISE_OVERLAY_CHAT_REDIRECT_CASE",
            &["redirect"],
        ) {
            return;
        }
        struct RedirectToChat(Arc<Mutex<Vec<String>>>);
        #[mod_api::__subscriptions]
        impl Hooks for RedirectToChat {
            fn screen_opening(
                &self,
                _: &dyn ClientHost,
                event: &mut ScreenOpeningEvent<'_>,
            ) -> Option<ScreenOpening> {
                self.0
                    .lock()
                    .unwrap()
                    .push(format!("{}->{}", event.current.id, event.new.id));
                (event.new.id == "link_prompt")
                    .then_some(ScreenOpening::Vanilla(ScreenTarget::Chat))
            }
            fn screen_closing(&self, _: &dyn ClientHost, event: &mut ScreenClosingEvent<'_>) {
                self.0
                    .lock()
                    .unwrap()
                    .push(format!("close {}", event.screen.id));
            }
            fn screen_init_post(&self, _: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
                self.0
                    .lock()
                    .unwrap()
                    .push(format!("init {}", event.screen.id));
            }
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new(
                "overlay_to_chat",
                Box::new(RedirectToChat(events.clone())),
            )]
            .into(),
            || {
                with_client_game(|game| {
                    game.client.title.0 = None;
                    game.player =
                        Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
                    game.client.screen = Screen::Playing;
                    events.lock().unwrap().clear();

                    let opened = game.open_screen_overlay("link_prompt", |game| {
                        game.client.link_prompt = Some("https://example.com".into());
                    });
                    assert!(!opened);
                    assert!(game.client.link_prompt.is_none());
                    assert_eq!(game.client.screen, Screen::Chat);
                    assert!(game.client.chat_ui.chat_screen.is_some());
                    game.observe_screen();
                    assert_eq!(
                        *events.lock().unwrap(),
                        ["playing->link_prompt", "init chat"]
                    );
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn waking_bed_chat_dismisses_empty_or_draft_despite_veto_and_keeps_edited_draft() {
        use crate::game::chat::{ChatDraft, ChatExit};
        use mod_api::{
            ClientHost, Hooks, ScreenClosingEvent, ScreenInitPostEvent, ScreenOpening,
            ScreenOpeningEvent, ScreenTarget,
        };
        use std::sync::{Arc, Mutex};
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::motion::PlayerMotion;
        use winit::keyboard::{KeyCode, ModifiersState};

        if run_fresh_cases(
            "game::client_side::native_net::tests::waking_bed_chat_dismisses_empty_or_draft_despite_veto_and_keeps_edited_draft",
            "SUNRISE_WAKE_BED_CHAT_CASE",
            &["wake_cases"],
        ) {
            return;
        }
        #[derive(Default)]
        struct Probe {
            answer: Mutex<Option<ScreenOpening>>,
            events: Mutex<Vec<String>>,
        }
        struct Listener(Arc<Probe>);
        #[mod_api::__subscriptions]
        impl Hooks for Listener {
            fn screen_opening(
                &self,
                _: &dyn ClientHost,
                event: &mut ScreenOpeningEvent<'_>,
            ) -> Option<ScreenOpening> {
                self.0
                    .events
                    .lock()
                    .unwrap()
                    .push(format!("{}->{}", event.current.id, event.new.id));
                self.0.answer.lock().unwrap().clone()
            }
            fn screen_closing(&self, _: &dyn ClientHost, event: &mut ScreenClosingEvent<'_>) {
                self.0
                    .events
                    .lock()
                    .unwrap()
                    .push(format!("close {}", event.screen.id));
            }
            fn screen_init_post(&self, _: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
                self.0
                    .events
                    .lock()
                    .unwrap()
                    .push(format!("init {}", event.screen.id));
            }
        }
        let probe = Arc::new(Probe::default());
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new(
                "wake_bed_chat",
                Box::new(Listener(probe.clone())),
            )]
            .into(),
            || {
                with_client_game(|game| {
                    game.client.title.0 = None;
                    game.player =
                        Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
                    game.sleeping = None;

                    // Empty bed chat dismisses via Gui.setScreen(null): Closing, no vetoable Opening.
                    assert!(game.begin_chat_screen("", false));
                    game.client.chat_ui.in_bed_chat = true;
                    probe.events.lock().unwrap().clear();
                    *probe.answer.lock().unwrap() = Some(ScreenOpening::Cancel);
                    game.tick_in_bed_chat();
                    game.tick_in_bed_chat();
                    game.observe_screen();
                    assert_eq!(game.client.screen, Screen::Playing);
                    assert!(!game.client.chat_ui.in_bed_chat);
                    assert!(game.client.chat_ui.chat_screen.is_none());
                    assert_eq!(
                        std::mem::take(&mut *probe.events.lock().unwrap()),
                        ["close chat"]
                    );

                    // Ordinary chat (Escape or a submit) dismisses the same way: one Closing, no
                    // Opening to veto, so a vetoed Opening cannot leave the box open.
                    *probe.answer.lock().unwrap() = None;
                    assert!(game.begin_chat_screen("", false));
                    assert_eq!(game.client.screen, Screen::Chat);
                    probe.events.lock().unwrap().clear();
                    *probe.answer.lock().unwrap() = Some(ScreenOpening::Cancel);
                    game.close_chat(ChatExit::Done);
                    game.observe_screen();
                    assert_eq!(game.client.screen, Screen::Playing);
                    assert!(game.client.chat_ui.chat_screen.is_none());
                    assert_eq!(
                        std::mem::take(&mut *probe.events.lock().unwrap()),
                        ["close chat"]
                    );

                    // Untouched restored draft in bed also dismisses despite veto and preserves draft.
                    *probe.answer.lock().unwrap() = None;
                    game.client.chat_ui.chat_draft = Some(ChatDraft {
                        text: "saved draft".into(),
                        command: false,
                    });
                    assert!(game.begin_chat_screen("saved draft", true));
                    assert!(game.client.chat_ui.chat_screen.as_ref().unwrap().is_draft());
                    game.client.chat_ui.in_bed_chat = true;
                    probe.events.lock().unwrap().clear();
                    *probe.answer.lock().unwrap() = Some(ScreenOpening::Cancel);
                    game.tick_in_bed_chat();
                    game.tick_in_bed_chat();
                    game.observe_screen();
                    assert_eq!(game.client.screen, Screen::Playing);
                    assert!(!game.client.chat_ui.in_bed_chat);
                    assert!(game.client.chat_ui.chat_screen.is_none());
                    assert_eq!(
                        game.client
                            .chat_ui
                            .chat_draft
                            .as_ref()
                            .map(|d| d.text.as_str()),
                        Some("saved draft")
                    );
                    assert_eq!(
                        std::mem::take(&mut *probe.events.lock().unwrap()),
                        ["close chat"]
                    );

                    // Editing a restored draft clears the live is_draft flag, so waking carries it
                    // into ordinary chat instead of dismissing the box.
                    *probe.answer.lock().unwrap() = None;
                    assert!(game.begin_chat_screen("saved draft", true));
                    assert!(game.client.chat_ui.chat_screen.as_ref().unwrap().is_draft());
                    game.client.chat_ui.in_bed_chat = true;
                    game.chat_type("!");
                    assert!(!game.client.chat_ui.chat_screen.as_ref().unwrap().is_draft());
                    probe.events.lock().unwrap().clear();
                    *probe.answer.lock().unwrap() = Some(ScreenOpening::Cancel);
                    game.tick_in_bed_chat();
                    assert!(
                        !game.client.chat_ui.in_bed_chat,
                        "the transition is offered once"
                    );
                    assert_eq!(
                        game.client.chat_ui.chat_screen.as_ref().unwrap().value(),
                        "saved draft!"
                    );
                    assert_eq!(
                        game.client.chat_ui.chat_draft.as_ref().unwrap().text,
                        "saved draft"
                    );
                    assert_eq!(*probe.events.lock().unwrap(), ["chat->chat"]);
                    *probe.answer.lock().unwrap() = None;
                    game.tick_in_bed_chat();
                    game.tick_in_bed_chat();
                    game.observe_screen();
                    assert_eq!(game.client.screen, Screen::Chat);
                    assert!(!game.client.chat_ui.in_bed_chat);
                    assert_eq!(
                        game.client.chat_ui.chat_screen.as_ref().unwrap().value(),
                        "saved draft!"
                    );
                    assert_eq!(
                        game.client.chat_ui.chat_draft.as_ref().unwrap().text,
                        "saved draft"
                    );
                    assert_eq!(
                        *probe.events.lock().unwrap(),
                        ["chat->chat"],
                        "later awake ticks do not ask again"
                    );

                    // A redirect doesn't install the requested prefilled instance, even if
                    // another Chat screen is what it opens.
                    for (answer, target) in [
                        (ScreenOpening::Vanilla(ScreenTarget::Pause), Screen::Paused),
                        (ScreenOpening::ModScreen, Screen::Mod),
                        (ScreenOpening::Vanilla(ScreenTarget::Chat), Screen::Chat),
                    ] {
                        game.set_screen(Screen::Playing);
                        game.client.chat_ui.chat_draft = Some(ChatDraft {
                            text: "recover me".into(),
                            command: false,
                        });
                        game.open_chat_method("");
                        game.client.chat_ui.in_bed_chat = true;
                        game.chat_type("!");
                        *probe.answer.lock().unwrap() = Some(answer);
                        game.tick_in_bed_chat();
                        *probe.answer.lock().unwrap() = None;
                        assert_eq!(game.client.screen, target);
                        assert!(!game.client.chat_ui.in_bed_chat);
                        assert_eq!(
                            game.client.chat_ui.chat_draft.as_ref().unwrap().text,
                            "recover me!"
                        );
                        if target == Screen::Chat {
                            assert_eq!(
                                game.client.chat_ui.chat_screen.as_ref().unwrap().value(),
                                ""
                            );
                        }
                        game.open_chat_method("");
                        let restored = game.client.chat_ui.chat_screen.as_ref().unwrap();
                        assert_eq!(restored.value(), "recover me!");
                        assert!(restored.is_draft());
                    }

                    // Ctrl+F borrows the input. Wake must classify the original input, not
                    // the nonempty query that cleared the widget's draft flag.
                    game.client.settings.extras.pvp.chat.search = true;
                    for edited in [false, true] {
                        game.set_screen(Screen::Playing);
                        game.client.chat_ui.chat_draft = Some(ChatDraft {
                            text: "message before search".into(),
                            command: false,
                        });
                        game.open_chat_method("");
                        game.client.chat_ui.in_bed_chat = true;
                        if edited {
                            game.chat_type("!");
                        }
                        game.client.modifiers = if cfg!(target_os = "macos") {
                            ModifiersState::SUPER
                        } else {
                            ModifiersState::CONTROL
                        };
                        game.chat_key_code(KeyCode::KeyF, None);
                        game.client.modifiers = ModifiersState::empty();
                        game.chat_type("query");
                        assert_eq!(
                            game.client.chat_ui.chat_screen.as_ref().unwrap().value(),
                            "query"
                        );
                        game.tick_in_bed_chat();
                        assert!(!game.client.chat_ui.in_bed_chat);
                        if edited {
                            assert_eq!(game.client.screen, Screen::Chat);
                            assert_eq!(
                                game.client.chat_ui.chat_screen.as_ref().unwrap().value(),
                                "message before search!"
                            );
                            assert!(game.client.chat_ui.chat_draft.is_none());
                        } else {
                            assert_eq!(game.client.screen, Screen::Playing);
                            game.open_chat_method("");
                            let restored = game.client.chat_ui.chat_screen.as_ref().unwrap();
                            assert_eq!(restored.value(), "message before search");
                            assert!(restored.is_draft());
                        }
                    }
                });
            },
        );
    }

    /// A joined flag cannot manufacture a physical transport. Packet codec,
    /// reservation, and framing cases live in net::mod_packet_tests.
    #[test]
    #[ignore = "daily"]
    fn mod_packets_require_joined_player_and_physical_transport() {
        if run_fresh_cases(
            "game::client_side::native_net::tests::mod_packets_require_joined_player_and_physical_transport",
            "SUNRISE_PACKET_GUARD_CHILD",
            &["guards"],
        ) {
            return;
        }
        with_client_game(|game| {
            let packet = || {
                let mut body = Writer::new();
                body.string("voicechat:request_secret");
                body.bytes(&[1, 2, 3, 4]);
                ServerboundPlayPacket::new(
                    Identifier::parse("minecraft:custom_payload").unwrap(),
                    body.data,
                )
            };
            assert_eq!(
                game.send_packet(packet()),
                Err(SendPacketError::NotConnected)
            );
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "PacketTest").unwrap();
            assert_eq!(game.send_packet(packet()), Err(SendPacketError::NotInPlay));
            // Deliberately inconsistent actor state: no handshake has installed
            // a writer. Acceptance must still require the physical provider.
            game.client.remote.as_mut().unwrap().joined = true;
            assert_eq!(
                game.send_packet(packet()),
                Err(SendPacketError::NotConnected)
            );
        });
    }
}
