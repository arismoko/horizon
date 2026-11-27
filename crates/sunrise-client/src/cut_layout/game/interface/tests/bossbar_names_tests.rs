//! `/bossbar get <id> players` names its players as vanilla does
//! (`BossBarCommands.getPlayers` → `ComponentUtils.formatList(players,
//! Player::getDisplayName)`): the team's formatting, the `/tell` click, the
//! entity hover and the name a shift-click inserts.
use std::sync::{Arc, atomic::AtomicI32, mpsc};
use sunrise_game_data::text::Text;
use sunrise_gameplay_common::game::Dim;
use sunrise_protocol::packets::configuration::ClientInformation;
use sunrise_sim::{
    lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig},
    motion::PlayerMotion,
};
use {
    sunrise_gameplay::game::lan::LanSettings,
    sunrise_gameplay_common::game::lan_actions::GuestCommand,
};

/// A host game with a window player and two other online players.
fn game_with_two_guests<'a>(
    resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
) -> sunrise_gameplay::game::ServerGame<'a> {
    use crate::game::load_resources;
    use crate::settings::Settings;
    use {sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress};
    let root = std::path::PathBuf::from(
        std::env::var_os("SUNRISE_TEST_RESOURCES").expect("set SUNRISE_TEST_RESOURCES"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role =
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
    let res = &*resources.insert(loaded.resources);
    let mut game = sunrise_gameplay::game::ServerGame::new(
        res,
        loaded.pack,
        options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    game.build_registries();
    game.set_allow_commands(true);
    game.player = Some(
        PlayerMotion::new(
            game.motion,
            sunrise_core::geometry::Vec3::new(10.5, 64.0, -3.25),
        )
        .unwrap(),
    );
    game.lan_host(
        LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            announce: false,
            ..NetConfig::lan(0, "bossbar-names-test".into(), 3)
        },
        HubConfig {
            default_game_mode: 0,
            guest_commands: true,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: Dim::Overworld.id().into(),
            allow_flight: false,
            seed: 0,
            max_players: 3,
            integrated: false,
            levels: Default::default(),
            level_seeds: Default::default(),
        },
    )
    .unwrap();
    for (conn, name) in [(2, "Guest"), (3, "Other")] {
        let (tx, _rx) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn,
                uuid: u128::from(conn),
                name: name.into(),
                properties: Vec::new(),
                info: ClientInformation::new(2),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            },
            conn as i32,
            [8.5, 64.0, 8.5],
            0,
        );
        guest.placed = true;
        guest.loaded = true;
        game.server.lan.as_mut().unwrap().hub.guests.push(guest);
    }
    game
}

/// A command typed by guest 2, as `LanHub` queues it: the feedback it got.
fn guest_2_command(game: &mut sunrise_gameplay::game::ServerGame<'_>, command: &str) -> Vec<Text> {
    let (ok, lines) = game
        .run_guest_command_told(GuestCommand {
            conn: 2,
            command: command.into(),
            permission: 4,
            integrated: false,
            quiet: false,
        })
        .expect("guest 2 is online");
    assert!(ok, "{command}");
    lines
}

#[test]
#[ignore = "daily"]
fn a_boss_bars_player_list_is_decorated_like_other_player_lists() {
    let mut resources = None;
    let mut game = game_with_two_guests(&mut resources);
    assert!(game.run_command("bossbar add demo:bar Demo"));
    assert!(game.run_command("team add demo_team"));
    assert!(game.run_command("team modify demo_team color red"));
    assert!(game.run_command("team join demo_team Guest"));
    assert!(game.run_command("team join demo_team Other"));
    guest_2_command(&mut game, "bossbar set demo:bar players @a");

    let lines = guest_2_command(&mut game, "bossbar get demo:bar players");
    assert_eq!(lines.len(), 1, "{lines:?}");
    let message = lines[0].component();
    // commands.bossbar.get.players.some: the bar's name, the count, the list.
    let names = message["with"][2]["extra"]
        .as_array()
        .expect("the player list");
    for (uuid, name) in [(2, "Guest"), (3, "Other")] {
        let listed = names
            .iter()
            .find(|n| n["insertion"] == name)
            .unwrap_or_else(|| panic!("no decorated name for {name} in {message}"));
        // Player::getDisplayName: the team's colour, the `/tell` click, the
        // entity hover and the name a shift-click inserts.
        assert_eq!(listed["color"], "red", "{listed}");
        assert_eq!(
            listed["click_event"]["command"],
            format!("/tell {name} "),
            "{listed}"
        );
        assert_eq!(listed["hover_event"]["action"], "show_entity", "{listed}");
        assert_eq!(listed["hover_event"]["id"], "minecraft:player", "{listed}");
        assert_eq!(listed["hover_event"]["name"], name, "{listed}");
        assert_eq!(
            listed["hover_event"]["uuid"],
            sunrise_protocol::crypto::uuid_string(uuid),
            "{listed}"
        );
    }
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
