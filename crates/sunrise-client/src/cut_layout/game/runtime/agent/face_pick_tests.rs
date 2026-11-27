//! The agent's turns toward a point read the pick the game takes after the turn: `attack ID`
//! and `look-at entity ID` wait for the next tick or frame pick (client_pick.rs) instead of
//! picking themselves, so a `client_pick` listener runs only in those calls. `attack ID`
//! reports what the pick its click was taken with held.
use mod_api::{ClientHost, ClientPickEvent, CrosshairTarget, Hooks};
use serde_json::Value;
use serde_json::json;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;
use sunrise_client_gameplay::game::EventLoopCtl;
use sunrise_client_gameplay::game::client_side::agent::Handled;
use sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::Screen;
use sunrise_level::chunk::Chunk;
use sunrise_protocol::connection::Connection;
use sunrise_protocol::packets::play::{self, Clientbound as C};
use sunrise_sim::motion::PlayerMotion;
use winit::event::MouseButton;
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
};

/// A subscribed `client_pick` listener: counts its calls, and can replace the result.
#[derive(Default)]
struct Listener {
    calls: usize,
    result: Option<Option<CrosshairTarget>>,
}
struct Picks(Arc<Mutex<Listener>>);
#[mod_api::__subscriptions]
impl Hooks for Picks {
    fn client_pick(&self, _: &mut dyn ClientHost, event: &mut ClientPickEvent<'_>) {
        let mut listener = self.0.lock().unwrap();
        listener.calls += 1;
        if let Some(result) = listener.result {
            *event.result = result;
        }
    }
}

struct NoLoop;
impl EventLoopCtl for NoLoop {
    fn exit(&self) {}
    fn set_control_flow(&self, _: winit::event_loop::ControlFlow) {}
}

/// The server's side of the handshake up to play, as `client_pick`'s tests drive it.
fn finish_login(game: &mut sunrise_client_gameplay::game::ClientGame<'_>, server: &mut Connection) {
    use sunrise_protocol::packets::{Empty, configuration as cfg, login};
    server.recv().unwrap(); // Intention.
    server.recv().unwrap(); // Hello.
    server
        .send(
            &login::Clientbound::LoginFinished(login::LoginFinished {
                profile: login::GameProfile {
                    uuid: 42,
                    name: "AgentPickTest".into(),
                    properties: vec![],
                },
                session: 1,
            })
            .encode(),
        )
        .unwrap();
    server.recv().unwrap(); // Login acknowledged.
    server.recv().unwrap(); // Brand.
    server.recv().unwrap(); // Client information.
    server
        .send(&cfg::Clientbound::FinishConfiguration(Empty).encode())
        .unwrap();
    game.acknowledge_test_configuration();
    server.recv().unwrap(); // Finish configuration.
}

/// A guest at (0.5, 1, 0.5) looking west (yaw 90), its stored pick taken that way, with a pig
/// (entity 31) 1.5 blocks east of it, behind its back. The listener's count starts at zero.
fn with_pig_behind(
    run: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>, &Mutex<Listener>),
) {
    with_pig_and_server(|game, listener, _| run(game, listener));
}

/// [`with_pig_behind`], with the server's end of the connection.
fn with_pig_and_server(
    run: impl FnOnce(
        &mut sunrise_client_gameplay::game::ClientGame<'_>,
        &Mutex<Listener>,
        &mut Connection,
    ),
) {
    let listener = Arc::new(Mutex::new(Listener::default()));
    let mods = vec![mod_api::ModEntry::new(
        "agent_picks",
        Box::new(Picks(listener.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            let mut server = Connection::local(logged_client(game));
            server
                .reader
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            finish_login(game, &mut server);
            game.native_lifecycle.begin_open(None);
            game.client.title.0 = None;
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 1.0, 0.5)).unwrap());
            game.previous_position = Vec3::new(0.5, 1.0, 0.5);
            game.yaw = 90.0;
            game.pitch = 0.0;
            game.entity_reach = 3.0;
            game.block_reach = 4.5;
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let pig = game
                .client
                .remote
                .as_ref()
                .unwrap()
                .builtin
                .get("minecraft:entity_type")
                .unwrap()
                .id("minecraft:pig")
                .unwrap();
            game.handle_packet(C::AddEntity(play::AddEntity {
                id: 31,
                uuid: 31,
                kind: pig,
                position: [2.0, 1.0, 0.5],
                velocity: [0.0; 3],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: 0,
            }))
            .unwrap();
            game.refresh_client_pick(1.0);
            assert!(game.mob_hit().is_none(), "the pig starts behind the view");
            // The player is loaded: the tick's keybinds act.
            game.client.remote.as_mut().unwrap().loaded_sent = true;
            listener.lock().unwrap().calls = 0;
            run(game, &listener, &mut server);
        })
    });
}

fn command(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    line: &str,
) -> (sunrise_game_data::Result<Handled>, mpsc::Receiver<String>) {
    let (reply, replies) = mpsc::channel();
    let words: Vec<&str> = line.split_whitespace().collect();
    (game.agent_command(&words, line, &reply, &NoLoop), replies)
}

fn calls(listener: &Mutex<Listener>) -> usize {
    listener.lock().unwrap().calls
}

/// A drawn frame: its pick, then the control poll's agent step.
fn frame(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) {
    game.refresh_client_pick(1.0);
    game.client.frames += 1;
    game.advance_agent();
}

/// A game tick's keybinds as in play (`remote_handle_input`): its pick, then the clicks the
/// remote input queue holds, behind the same readiness checks; then the control poll's agent
/// step.
fn tick(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) {
    game.ticks += 1;
    game.remote_handle_input();
    game.advance_agent();
}

/// What the client's attack clicks sent since the last call: `attack ID` and `punch` (the
/// swing).
fn clicks_sent(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    server: &mut Connection,
) -> Vec<String> {
    use sunrise_protocol::packets::{configuration::Ping, play::Serverbound as S};
    // A fence: everything before it was sent before this call.
    game.send(S::Pong(Ping { id: i32::MIN }));
    let mut sent = Vec::new();
    loop {
        let (id, payload) = server.recv().unwrap();
        match S::decode(id, &payload).unwrap() {
            S::Attack(hit) => sent.push(format!("attack {}", hit.id)),
            S::Punch(_) => sent.push("punch".to_owned()),
            S::Pong(ping) if ping.id == i32::MIN => return sent,
            _ => {}
        }
    }
}

/// A second pig, entity 32, 1.5 blocks east and 1 south of the player.
fn add_second_pig(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) {
    let pig = game
        .client
        .remote
        .as_ref()
        .unwrap()
        .builtin
        .get("minecraft:entity_type")
        .unwrap()
        .id("minecraft:pig")
        .unwrap();
    game.handle_packet(C::AddEntity(play::AddEntity {
        id: 32,
        uuid: 32,
        kind: pig,
        position: [2.0, 1.0, 1.5],
        velocity: [0.0; 3],
        pitch: 0,
        yaw: 0,
        head_yaw: 0,
        data: 0,
    }))
    .unwrap();
}

/// The listener makes every later pick pig 32.
fn pick_second_pig(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    listener: &Mutex<Listener>,
) {
    listener.lock().unwrap().result = Some(Some(CrosshairTarget::Entity {
        entity: game.entity(32).unwrap(),
        point: [1.55, 1.45, 1.5],
    }));
}

fn reply(replies: &mpsc::Receiver<String>) -> Value {
    serde_json::from_str(&replies.try_recv().expect("a reply")).unwrap()
}

#[test]
#[ignore = "daily"]
fn look_at_entity_replies_with_the_entity_it_turned_to() {
    with_pig_behind(|game, listener| {
        let (handled, replies) = command(game, "look-at entity 31");
        assert!(matches!(handled, Ok(Handled::Deferred)));
        // East of the eye: yaw -90, set at once; the pick waits for the frame.
        assert!((game.yaw + 90.0).abs() < 1e-3, "{}", game.yaw);
        assert_eq!(calls(listener), 0, "the command itself picks nothing");
        assert_eq!(game.crosshair_target(), None);
        assert!(replies.try_recv().is_err());
        frame(game);
        assert_eq!(calls(listener), 1, "the frame's own pick");
        let reply = reply(&replies);
        assert!(
            (reply["yaw"].as_f64().unwrap() + 90.0).abs() < 1e-3,
            "{reply}"
        );
        assert_eq!(reply["target"]["kind"], json!("entity"));
        assert_eq!(reply["target"]["entity"]["id"], json!(31));
    });
}

#[test]
#[ignore = "daily"]
fn look_at_entity_replies_after_a_tick_pick_with_no_frame() {
    with_pig_behind(|game, listener| {
        let (_, replies) = command(game, "look-at entity 31");
        tick(game);
        assert_eq!(game.client.frames, 0, "no frame was drawn");
        assert_eq!(calls(listener), 1, "the tick's own pick");
        let reply = reply(&replies);
        assert_eq!(reply["target"]["entity"]["id"], json!(31), "{reply}");
    });
}

#[test]
#[ignore = "daily"]
fn a_turn_with_motion_replies_after_the_frame_that_turns_it() {
    with_pig_behind(|game, _| {
        let (_, replies) = command(game, "mouse-move 10 0");
        tick(game);
        assert!(replies.try_recv().is_err(), "a tick's pick turns nothing");
        frame(game);
        assert_eq!(reply(&replies)["motion"], json!([10.0, 0.0]));
    });
}

#[test]
#[ignore = "daily"]
fn look_at_over_previews_the_angle_without_picking() {
    with_pig_behind(|game, listener| {
        let (handled, _replies) = command(game, "look-at entity 31 --over 1");
        assert!(matches!(handled, Ok(Handled::Deferred)));
        assert_eq!(calls(listener), 0, "the preview fires no listener");
        assert_eq!(game.crosshair_target(), None, "nor publishes a target");
        assert_eq!(
            (game.yaw, game.pitch),
            (90.0, 0.0),
            "the view turns by mouse"
        );
    });
}

#[test]
#[ignore = "daily"]
fn attack_by_id_hits_the_entity_it_turned_to() {
    with_pig_and_server(|game, listener, server| {
        let (attacked, replies) = command(game, "attack 31");
        assert!(
            matches!(attacked, Ok(Handled::Deferred)),
            "attack 31 was refused: {:?}",
            attacked.err()
        );
        assert_eq!(
            game.client.agent.task.as_ref().map(|t| t.name),
            Some("attack")
        );
        assert_eq!(calls(listener), 0, "the command itself picks nothing");
        let health = game.mobs.iter().find(|m| m.id == 31).unwrap().health;
        frame(game);
        tick(game);
        assert_eq!(calls(listener), 2, "the frame's and the tick's own picks");
        assert_eq!(game.mob_hit().map(|(i, _, _)| game.mobs[i].id), Some(31));
        assert_eq!(
            clicks_sent(game, server),
            Vec::<String>::new(),
            "the click is queued after this tick's keybinds"
        );
        tick(game);
        assert_eq!(
            clicks_sent(game, server),
            ["attack 31", "punch"],
            "the next tick took the click"
        );
        for _ in 0..3 {
            assert!(replies.try_recv().is_err());
            tick(game);
        }
        let reply = reply(&replies);
        assert_eq!(reply["target"], json!(31), "{reply}");
        assert_eq!(reply["hit"], json!(true), "{reply}");
        assert_eq!(reply["health_before"], json!(health), "{reply}");
    });
}

#[test]
#[ignore = "daily"]
fn attack_by_id_reports_the_pick_the_click_was_taken_with() {
    with_pig_and_server(|game, listener, server| {
        add_second_pig(game);
        let (_, replies) = command(game, "attack 31");
        // The first tick's pick holds pig 31: the check passes and the click is queued.
        tick(game);
        assert!(replies.try_recv().is_err());
        // The next tick's pick, the one its keybinds take the click with, holds pig 32.
        pick_second_pig(game, listener);
        for _ in 0..5 {
            tick(game);
        }
        assert_eq!(
            clicks_sent(game, server),
            Vec::<String>::new(),
            "no attack and no swing reached the server"
        );
        let reply = reply(&replies);
        assert_eq!(
            reply["error"],
            json!(
                "another target was under the crosshair when the click was taken; nothing was clicked"
            ),
            "{reply}"
        );
        assert_eq!(reply["picked"], json!(32), "{reply}");
        assert!(game.client.agent.task.is_none());
    });
}

#[test]
#[ignore = "daily"]
fn attack_by_id_reports_a_click_no_tick_took() {
    if run_fresh_cases(
        "cut_layout::game::client_side::agent::face_pick_tests::attack_by_id_reports_a_click_no_tick_took",
        "SUNRISE_AGENT_ATTACK_CLICK_NOT_TAKEN_CHILD",
        &["switcher"],
    ) {
        return;
    }

    with_pig_and_server(|game, _, server| {
        let (_, replies) = command(game, "attack 31");
        tick_with_player_clicks(game, 1);
        // The switcher skips keybinds, but opening it doesn't clear attack's toggle mapping.
        game.set_screen(Screen::GameModeSwitcher);
        tick(game);
        assert_eq!(clicks_sent(game, server), Vec::<String>::new());
        // The ordinary click survives; the agent's pending aimed click does not.
        game.set_screen(Screen::Playing);
        tick(game);
        assert_eq!(clicks_sent(game, server), ["attack 31", "punch"]);
        // A new player click isn't claimed by the command either.
        player_click(game);
        tick(game);
        assert_eq!(clicks_sent(game, server), ["attack 31", "punch"]);
        tick(game);
        assert_eq!(
            reply(&replies)["error"],
            json!("the attack click was not taken")
        );
    });
}

#[test]
#[ignore = "daily"]
fn attack_by_id_stop_retires_only_its_queued_click() {
    if run_fresh_cases(
        "cut_layout::game::client_side::agent::face_pick_tests::attack_by_id_stop_retires_only_its_queued_click",
        "SUNRISE_AGENT_ATTACK_STOP_CHILD",
        &["stop"],
    ) {
        return;
    }

    with_pig_and_server(|game, listener, server| {
        add_second_pig(game);
        let (_, replies) = command(game, "attack 31");
        tick_with_player_clicks(game, 1);

        let (stopped, _) = command(game, "stop");
        assert!(matches!(stopped, Ok(Handled::Reply(_))));
        assert_eq!(reply(&replies)["error"], json!("cancelled by stop"));

        pick_second_pig(game, listener);
        tick(game);
        assert_eq!(clicks_sent(game, server), ["attack 32", "punch"]);
    });
}

#[test]
#[ignore = "daily"]
fn attack_by_id_expiry_retires_only_its_queued_click() {
    if run_fresh_cases(
        "cut_layout::game::client_side::agent::face_pick_tests::attack_by_id_expiry_retires_only_its_queued_click",
        "SUNRISE_AGENT_ATTACK_EXPIRY_CHILD",
        &["expiry"],
    ) {
        return;
    }

    with_pig_and_server(|game, listener, server| {
        add_second_pig(game);
        let (_, replies) = command(game, "attack 31");
        tick_with_player_clicks(game, 1);

        game.client.agent.task.as_mut().unwrap().deadline = Instant::now();
        game.advance_agent();
        assert_eq!(
            reply(&replies)["error"],
            json!("the attack click was not taken")
        );

        pick_second_pig(game, listener);
        tick(game);
        assert_eq!(clicks_sent(game, server), ["attack 32", "punch"]);
    });
}

/// A player's left click through the same mouse mapping, queued for the next tick.
fn player_click(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) {
    game.agent_mouse(MouseButton::Left, true).unwrap();
    game.agent_mouse(MouseButton::Left, false).unwrap();
}

/// A tick whose keybinds run, then `clicks` player clicks, then the control poll's agent step
/// (where `attack ID` queues its click, behind them).
fn tick_with_player_clicks(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    clicks: usize,
) {
    game.ticks += 1;
    game.remote_handle_input();
    for _ in 0..clicks {
        player_click(game);
    }
    game.advance_agent();
}

fn attacks_on(sent: &[String], id: u32) -> usize {
    sent.iter()
        .filter(|s| **s == format!("attack {id}"))
        .count()
}

#[test]
#[ignore = "daily"]
fn attack_by_id_drops_its_own_click_not_a_players_in_the_same_tick() {
    with_pig_and_server(|game, listener, server| {
        add_second_pig(game);
        let (_, replies) = command(game, "attack 31");
        tick_with_player_clicks(game, 1);
        pick_second_pig(game, listener);
        tick(game);
        assert_eq!(
            clicks_sent(game, server),
            ["attack 32", "punch"],
            "the player's click acts, the command's is dropped"
        );
        assert_eq!(reply(&replies)["picked"], json!(32));
    });
}

#[test]
#[ignore = "daily"]
fn attack_by_id_checks_its_own_click_behind_a_tick_of_player_clicks() {
    with_pig_and_server(|game, listener, server| {
        add_second_pig(game);
        let (_, replies) = command(game, "attack 31");
        // All 64 player clicks and the command's own click drain in the same tick;
        // only the command's click checks its target and drops when pig 32 is picked.
        tick_with_player_clicks(game, 64);
        pick_second_pig(game, listener);
        tick(game);
        let sent = clicks_sent(game, server);
        assert_eq!(attacks_on(&sent, 32), 64, "every player click acts");
        assert_eq!(attacks_on(&sent, 31), 0);
        assert_eq!(reply(&replies)["picked"], json!(32));
    });
}

#[test]
#[ignore = "daily"]
fn attack_by_id_click_drained_by_an_item_in_use_is_not_a_later_click() {
    with_pig_and_server(|game, _, server| {
        let (_, replies) = command(game, "attack 31");
        tick(game);
        // An item in use: the tick drains the click queues without acting on them.
        game.using = Some((game.selected, 0));
        tick(game);
        game.using = None;
        // The player's next click on pig 31 is not the command's.
        player_click(game);
        tick(game);
        assert_eq!(attacks_on(&clicks_sent(game, server), 31), 1);
        for _ in 0..3 {
            tick(game);
        }
        assert_eq!(
            reply(&replies)["error"],
            json!("the attack click was not taken")
        );
    });
}

#[test]
#[ignore = "daily"]
fn attack_by_id_drains_behind_a_large_player_burst() {
    with_pig_and_server(|game, _, server| {
        let (_, replies) = command(game, "attack 31");
        tick_with_player_clicks(game, 1024);
        tick(game);
        assert_eq!(attacks_on(&clicks_sent(game, server), 31), 1025);
        for _ in 0..3 {
            tick(game);
        }
        let reply = reply(&replies);
        assert_eq!(reply["hit"], json!(true), "{reply}");
        assert_eq!(reply["target"], json!(31), "{reply}");
    });
}

#[test]
#[ignore = "daily"]
fn an_ordinary_click_attacks_what_the_pick_holds() {
    with_pig_and_server(|game, listener, server| {
        add_second_pig(game);
        pick_second_pig(game, listener);
        // A click with no `attack ID` waiting.
        player_click(game);
        tick(game);
        assert_eq!(clicks_sent(game, server), ["attack 32", "punch"]);
    });
}

#[test]
#[ignore = "daily"]
fn attack_by_id_ends_without_a_click_on_death_or_without_ticks() {
    with_pig_and_server(|game, _, server| {
        let (_, replies) = command(game, "attack 31");
        game.client.screen = Screen::Dead;
        game.advance_agent();
        assert_eq!(reply(&replies)["error"], json!("the player died"));
        game.client.screen = Screen::Playing;
        let (_, replies) = command(game, "attack 31");
        game.client.agent.task.as_mut().unwrap().deadline = Instant::now();
        game.advance_agent();
        assert_eq!(
            reply(&replies)["error"],
            json!("game ticks are not advancing (paused?)")
        );
        // A screen opened over the world before the tick: the click would land on it.
        let (_, replies) = command(game, "attack 31");
        game.client.screen = Screen::GameModeSwitcher;
        tick(game);
        let error = reply(&replies)["error"].as_str().unwrap().to_owned();
        assert!(error.ends_with("screen is open; close it first (close, or click-button)"));
        game.client.screen = Screen::Playing;
        tick(game);
        assert_eq!(clicks_sent(game, server), Vec::<String>::new());
    });
}

#[test]
#[ignore = "daily"]
fn attack_by_id_follows_the_listener_decision() {
    with_pig_and_server(|game, listener, server| {
        listener.lock().unwrap().result = Some(None);
        let (attacked, replies) = command(game, "attack 31");
        assert!(matches!(attacked, Ok(Handled::Deferred)));
        tick(game);
        assert_eq!(calls(listener), 1);
        let reply = reply(&replies);
        assert_eq!(
            reply["error"],
            json!("that entity is out of reach or hidden; get closer")
        );
        assert!(game.client.agent.task.is_none());
        tick(game);
        assert_eq!(
            clicks_sent(game, server),
            Vec::<String>::new(),
            "nothing was clicked"
        );
    });
}

#[test]
#[ignore = "daily"]
fn look_at_entity_waits_for_a_running_turn() {
    with_pig_behind(|game, _| {
        let (turning, _turn) = command(game, "turn 10 0 --over 1");
        assert!(matches!(turning, Ok(Handled::Deferred)));
        let (handled, _) = command(game, "look-at entity 31");
        assert_eq!(
            handled.err().map(|e| e.to_string()),
            Some("already turning; wait for it or send stop".into())
        );
        assert_eq!(game.yaw, 90.0, "the running turn keeps the view");
    });
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
