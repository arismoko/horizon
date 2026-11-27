use crate::game::remote::tests::{connect_play_test, spawn_menu_test};
use crate::game::server_side::native_entity_refs::tests::with_client_game;
use mod_api::{
    ClientHost, ClientLevelUnloadEvent, ClientPlayerLoggingOutEvent, ClientStoppingEvent, GameMode,
    Hooks, ModEntry, PlayerRef,
};
use mod_platform::testing::with_mods;
use std::cell::Cell;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use sunrise_client::level_loading::Reason;
use sunrise_client::title::Action;
use sunrise_client_gameplay::game::{
    ClientGame, EventLoopCtl, client_side::remote_client::session_state::RemoteDimensionEntry,
};
use sunrise_core::nbt::Tag;
use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;
use sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycle;
use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
use sunrise_protocol::{
    connection::{Connection, Pipe},
    packets::{
        Empty, configuration as cfg, login,
        play::{self, Clientbound as C, SpawnInfo},
    },
};
use winit::event_loop::ControlFlow;

#[derive(Debug, PartialEq, Eq)]
enum Observed {
    Logout {
        player: Option<PlayerRef>,
        game_mode: Option<GameMode>,
        local_player: Option<PlayerRef>,
        host_game_mode: Option<GameMode>,
        level_live: bool,
    },
    LevelUnload(String),
    Stopping,
}

fn empty_logout() -> Observed {
    Observed::Logout {
        player: None,
        game_mode: None,
        local_player: None,
        host_game_mode: None,
        level_live: false,
    }
}

struct TestLoop {
    exited: Cell<bool>,
}

impl EventLoopCtl for TestLoop {
    fn exit(&self) {
        self.exited.set(true);
    }

    fn set_control_flow(&self, _flow: ControlFlow) {}
}

struct Probe {
    events: Arc<Mutex<Vec<Observed>>>,
    expected_dimension: Arc<Mutex<String>>,
}

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn client_player_logging_out(
        &self,
        host: &mut dyn ClientHost,
        event: &mut ClientPlayerLoggingOutEvent,
    ) {
        let dim = self.expected_dimension.lock().unwrap().clone();
        let local_player = host.local_player();
        let host_game_mode = event.player.and_then(|player| player.game_mode(&*host));
        let level_live = host.level(&dim).is_some();
        self.events.lock().unwrap().push(Observed::Logout {
            player: event.player,
            game_mode: event.game_mode,
            local_player,
            host_game_mode,
            level_live,
        });
    }

    fn client_level_unload(
        &self,
        _host: &mut dyn ClientHost,
        event: &mut ClientLevelUnloadEvent<'_>,
    ) {
        self.events
            .lock()
            .unwrap()
            .push(Observed::LevelUnload(event.dimension.to_owned()));
    }

    fn client_stopping(&self, _host: &mut dyn ClientHost, _event: &mut ClientStoppingEvent) {
        self.events.lock().unwrap().push(Observed::Stopping);
    }
}

fn spawn_info(dimension: &str, game_type: i32) -> SpawnInfo {
    SpawnInfo {
        dimension_type: 0,
        dimension: dimension.into(),
        seed: 0,
        game_type,
        previous_game_type: None,
        debug: false,
        flat: true,
        last_death: None,
        portal_cooldown: 0,
        sea_level: 63,
    }
}

fn poll_until(game: &mut ClientGame<'_>, predicate: impl Fn(&ClientGame<'_>) -> bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !predicate(game) {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        game.poll_remote_events(false).unwrap();
        if !predicate(game) {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

fn local_server(pipe: Pipe) -> Connection {
    let server = Connection::local(pipe);
    server
        .reader
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    server
}

fn accept_into_configuration(server: &mut Connection, uuid: u128, name: &str) {
    server.recv().unwrap(); // Intention
    server.recv().unwrap(); // Hello
    server
        .send(
            &login::Clientbound::LoginFinished(login::LoginFinished {
                profile: login::GameProfile {
                    uuid,
                    name: name.into(),
                    properties: vec![],
                },
                session: 1,
            })
            .encode(),
        )
        .unwrap();
    server.recv().unwrap(); // LoginAcknowledged
    server.recv().unwrap(); // Brand
    server.recv().unwrap(); // ClientInformation
}

#[test]
#[ignore = "daily"]
fn client_player_logging_out_fires_once_before_teardown_and_handles_no_player_transitions() {
    if run_fresh_cases(
        "cut_layout::game::client_side::remote_client::client_logging_out_tests::client_player_logging_out_fires_once_before_teardown_and_handles_no_player_transitions",
        "SUNRISE_CLIENT_LOGOUT_TEST_CHILD",
        &["logout"],
    ) {
        return;
    }
    let events = Arc::new(Mutex::new(Vec::new()));
    let expected_dimension = Arc::new(Mutex::new("minecraft:overworld".to_owned()));
    let probe = Probe {
        events: Arc::clone(&events),
        expected_dimension: Arc::clone(&expected_dimension),
    };
    let mods: mod_api::Mods = vec![ModEntry::new("logout_probe", Box::new(probe))].into();
    with_mods(mods, || {
        with_client_game(|game| {
            let take = || std::mem::take(&mut *events.lock().unwrap());
            let saves = tempfile::tempdir().unwrap();

            // 1. ConnectScreen.startConnecting emits one no-player logout; a failed
            // address/socket attempt does not emit a second logout when torn down.
            let title = game.new_title(saves.path().to_path_buf());
            game.show_title(title);
            take();
            game.connect_server_with_options("[invalid]", "BadAddr", None, false, false, None)
                .unwrap();
            assert_eq!(
                take(),
                vec![empty_logout()],
                "ConnectScreen.startConnecting notifies once before starting the attempt"
            );
            poll_until(
                game,
                |g| g.client.remote.is_none(),
                "failed connect attempt",
            );
            assert_eq!(
                take(),
                vec![],
                "failed address/socket attempt does not fire client_player_logging_out again"
            );

            // 2. ConnectScreen Cancel aborts the pending connection without emitting logout.
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "CancelAttempt").unwrap();
            take();
            let (x, y) = game
                .connect_cancel(game.gui_size())
                .expect("ConnectScreen Cancel button");
            game.client.cursor = (x + 1.0, y + 1.0);
            game.connect_screen_click().unwrap();
            assert!(game.client.remote.is_none());
            assert_eq!(
                take(),
                vec![],
                "ConnectScreen Cancel does not fire client_player_logging_out"
            );

            // 3. Login-handshake failure (ClientHandshakePacketListenerImpl.onDisconnect)
            // only changes the screen without calling Minecraft.disconnect.
            let (client, server_pipe) = Pipe::pair();
            game.connect_local(client, "LoginReject").unwrap();
            take();
            let mut server = local_server(server_pipe);
            server.recv().unwrap(); // Intention
            server.recv().unwrap(); // Hello
            server
                .send(
                    &login::Clientbound::Disconnect(login::Disconnect {
                        reason_json: r#"{"text":"Refused during login"}"#.into(),
                    })
                    .encode(),
                )
                .unwrap();
            poll_until(
                game,
                |g| g.client.remote.is_none(),
                "login-handshake disconnect",
            );
            assert_eq!(
                take(),
                vec![],
                "login-handshake failure does not fire client_player_logging_out"
            );

            // 4. Configuration disconnect (ClientConfigurationPacketListenerImpl.onDisconnect
            // -> ClientCommonPacketListenerImpl.onDisconnect -> Minecraft.disconnect) emits
            // one no-player logout.
            let (client, server_pipe) = Pipe::pair();
            game.connect_local(client, "ConfigDisconnect").unwrap();
            take();
            let mut server = local_server(server_pipe);
            accept_into_configuration(&mut server, 0x1111_2222, "ConfigDisconnect");
            server
                .send(
                    &cfg::Clientbound::Disconnect(cfg::Disconnect {
                        reason: Tag::String("Disconnected in configuration".into()),
                    })
                    .encode(),
                )
                .unwrap();
            poll_until(
                game,
                |g| g.client.remote.is_none(),
                "configuration disconnect",
            );
            assert_eq!(
                take(),
                vec![empty_logout()],
                "configuration disconnect fires one no-player logout"
            );

            // 5. Logged-in disconnect via leave_server: fires once before teardown
            // with live PlayerRef and GameMode; dimension change (Respawn) does not fire it.
            game.native_lifecycle = NativeLifecycle::default();
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "LoggedIn").unwrap();
            game.enter_remote_dimension(
                &spawn_info("minecraft:overworld", GameMode::Survival.id()),
                RemoteDimensionEntry::Login,
                Reason::Other,
            );
            {
                let remote = game.client.remote.as_mut().unwrap();
                remote.logged_in = true;
                remote.entity_id = 42;
                remote.profile = 0xDEAD_BEEF;
            }
            take();

            let nether = spawn_info("minecraft:the_nether", GameMode::Creative.id());
            *expected_dimension.lock().unwrap() = "minecraft:the_nether".to_owned();
            game.handle_packet(C::Respawn(play::Respawn {
                spawn: nether,
                keep: 0,
            }))
            .unwrap();
            game.handle_packet(C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                actions: 0x01 | 0x04 | 0x08,
                entries: vec![play::PlayerInfo {
                    uuid: 0xDEAD_BEEF,
                    name: Some("LoggedIn".into()),
                    game_mode: Some(GameMode::Creative.id()),
                    listed: Some(true),
                    ..play::PlayerInfo::default()
                }],
            }))
            .unwrap();
            assert_eq!(
                take(),
                vec![Observed::LevelUnload("minecraft:overworld".to_owned())],
                "dimension change unloads the old level without firing client_player_logging_out"
            );

            let live_player = game.local_player().expect("logged-in local player");
            game.leave_server(None).unwrap();
            assert_eq!(
                take(),
                vec![
                    Observed::Logout {
                        player: Some(live_player),
                        game_mode: Some(GameMode::Creative),
                        local_player: Some(live_player),
                        host_game_mode: Some(GameMode::Creative),
                        level_live: true,
                    },
                    Observed::LevelUnload("minecraft:the_nether".to_owned()),
                ],
                "logout runs once with live player and mode before level unload and teardown"
            );
            assert!(game.local_player().is_none());
            assert_eq!(live_player.game_mode(game), None);

            // 6. Duplicate logout calls on the same connection fire only once.
            game.native_lifecycle = NativeLifecycle::default();
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "Deduplicated").unwrap();
            *expected_dimension.lock().unwrap() = "minecraft:overworld".to_owned();
            game.enter_remote_dimension(
                &spawn_info("minecraft:overworld", GameMode::Adventure.id()),
                RemoteDimensionEntry::Login,
                Reason::Other,
            );
            {
                let remote = game.client.remote.as_mut().unwrap();
                remote.logged_in = true;
                remote.entity_id = 77;
                remote.profile = 0xCAFE_BABE;
            }
            game.handle_packet(C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                actions: 0x01 | 0x04 | 0x08,
                entries: vec![play::PlayerInfo {
                    uuid: 0xCAFE_BABE,
                    name: Some("Deduplicated".into()),
                    game_mode: Some(GameMode::Adventure.id()),
                    listed: Some(true),
                    ..play::PlayerInfo::default()
                }],
            }))
            .unwrap();
            take();

            let second_player = game.local_player().expect("second local player");
            game.client_logging_out();
            game.client_logging_out();
            game.leave_server(None).unwrap();
            assert_eq!(
                take(),
                vec![
                    Observed::Logout {
                        player: Some(second_player),
                        game_mode: Some(GameMode::Adventure),
                        local_player: Some(second_player),
                        host_game_mode: Some(GameMode::Adventure),
                        level_live: true,
                    },
                    Observed::LevelUnload("minecraft:overworld".to_owned()),
                ]
            );

            // 7a. StartConfiguration unloads the level before ServerReconfigScreen's
            // Disconnect button can emit a no-player logout.
            game.native_lifecycle = NativeLifecycle::default();
            let (_reader, mut writer) = connect_play_test(game, |_, _| {});
            spawn_menu_test(game);
            take();
            writer.send(&C::StartConfiguration(Empty).encode()).unwrap();
            poll_until(
                game,
                |g| {
                    g.client
                        .remote
                        .as_ref()
                        .is_some_and(|r| r.reconfiguring.is_some())
                },
                "start reconfiguration before clicking Disconnect",
            );
            assert_eq!(
                take(),
                vec![Observed::LevelUnload("minecraft:overworld".to_owned())],
                "reconfiguration starts by unloading the level without a logout"
            );

            game.ticks += 600;
            let (_, (rx, ry)) = game.reconfigure_layout(game.gui_size());
            game.client.cursor = (rx + 1.0, ry + 1.0);
            assert!(game.connect_screen_click().unwrap());
            assert!(game.client.remote.is_none());
            assert_eq!(
                take(),
                vec![empty_logout()],
                "ServerReconfigScreen Disconnect fires one no-player logout"
            );

            // 7b. Server-driven reconfiguration followed by a configuration disconnect
            // unloads the level on StartConfiguration and emits one no-player logout on
            // the disconnect.
            game.native_lifecycle = NativeLifecycle::default();
            let (client, server_pipe) = Pipe::pair();
            game.connect_local(client, "ReconfigDrop").unwrap();
            let mut server = local_server(server_pipe);
            accept_into_configuration(&mut server, 0x3333_4444, "ReconfigDrop");
            server
                .send(&cfg::Clientbound::FinishConfiguration(Empty).encode())
                .unwrap();
            poll_until(
                game,
                |g| g.client.remote.as_ref().is_some_and(|r| r.joined),
                "play join",
            );
            server.recv().unwrap(); // FinishConfiguration
            game.enter_remote_dimension(
                &spawn_info("minecraft:overworld", GameMode::Survival.id()),
                RemoteDimensionEntry::Login,
                Reason::Other,
            );
            {
                let remote = game.client.remote.as_mut().unwrap();
                remote.logged_in = true;
                remote.entity_id = 89;
                remote.profile = 0x3333_4444;
            }
            take();

            server
                .send(&play::Clientbound::StartConfiguration(Empty).encode())
                .unwrap();
            poll_until(
                game,
                |g| {
                    g.client
                        .remote
                        .as_ref()
                        .is_some_and(|r| r.reconfiguring.is_some())
                },
                "start reconfiguration",
            );
            server.recv().unwrap(); // ConfigurationAcknowledged
            server
                .send(
                    &cfg::Clientbound::Disconnect(cfg::Disconnect {
                        reason: Tag::String("Disconnected while reconfiguring".into()),
                    })
                    .encode(),
                )
                .unwrap();
            poll_until(
                game,
                |g| g.client.remote.is_none(),
                "reconfiguration disconnect",
            );
            drop(server);
            assert_eq!(
                take(),
                vec![
                    Observed::LevelUnload("minecraft:overworld".to_owned()),
                    empty_logout(),
                ],
                "reconfiguration unloads the level without logout, then its disconnect notifies once"
            );

            // 8. Native shutdown from the title without a closing-save request emits
            // one no-player logout in save_for_exit before client_stopping.
            let title = game.new_title(saves.path().to_path_buf());
            game.show_title(title);
            game.client.saved_for_exit = false;
            take();

            game.shut_down();
            assert_eq!(
                take(),
                vec![empty_logout(), Observed::Stopping],
                "native shutdown at the title emits one no-player logout before client_stopping"
            );
            game.save_for_exit().unwrap();
            assert_eq!(
                take(),
                vec![],
                "repeat save_for_exit after title Quit does not emit again"
            );

            // 9. Title Quit uses the closing-save path, then exits the loop without
            // emitting again in shut_down.
            let title = game.new_title(saves.path().to_path_buf());
            game.show_title(title);
            game.client.saved_for_exit = false;
            take();

            let ctl = TestLoop {
                exited: Cell::new(false),
            };
            game.title_action(Action::Quit).unwrap();
            game.idle(&ctl);
            assert!(ctl.exited.get(), "title Quit finishes the closing save");
            game.shut_down();
            assert_eq!(
                take(),
                vec![empty_logout(), Observed::Stopping],
                "async title quit emits one no-player logout and does not duplicate on shut_down"
            );
        });
    });
}
