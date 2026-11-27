use crate::game::{
    ServerGame,
    server_side::{dedicated::Lists, native_guest_turn_tests::with_guests},
};
use mod_api::{
    Command, CommandArgumentValue, Flow, Hooks, ModEntry, PermissionLevel, PlayerLoginEvent,
    RegisterCommandsEvent, ServerHost, prelude::ServicesServerHost as _,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicI32, AtomicUsize},
    mpsc,
};
use sunrise_command::{Dispatcher, Registries, network::from_packet};
use sunrise_core::profile::AccountProperty;
use sunrise_game_data::text::Text;
use sunrise_gameplay_common::game::{
    Dim, chat_common::ChatCommonGameExt1 as _, execution_role::ExecutionRole, in_own_process,
};
use sunrise_protocol::{
    buf::{Codec, Reader},
    ids,
    packets::{configuration::ClientInformation, play::Commands},
};
use sunrise_server_files::{
    user_cache::UserCache,
    user_lists::{BanList, IpBanList, OpList, WhiteList},
};
use sunrise_sim::lan::{Joined, Out, PlayerSender, guest::Guest, hub::HostEvent};

const OFFLINE: u128 = 31;
const EXPIRED: u128 = 32;

#[derive(Default)]
struct Seen {
    login: Option<PermissionLevel>,
    arguments: Vec<Vec<(u128, String)>>,
}
struct ProfileProbe(Arc<Mutex<Seen>>);

#[mod_api::__subscriptions]
impl Hooks for ProfileProbe {
    fn player_login(&self, host: &mut dyn ServerHost, event: &mut PlayerLoginEvent<'_>) -> Flow {
        assert!(host.players().player_by_name(event.name).is_none());
        self.0.lock().unwrap().login = Some(
            host.players()
                .profile_permission_level(event.player, event.name),
        );
        Flow::Continue
    }

    fn register_commands(&self, _: &mut dyn ServerHost, event: &mut RegisterCommandsEvent<'_>) {
        let seen = self.0.clone();
        event.commands.push(
            Command::new("inspect_profile <target:game_profile>").executes(
                move |_, _, invocation| {
                    let Some(CommandArgumentValue::GameProfiles(profiles)) =
                        invocation.argument("target")
                    else {
                        panic!("executor must receive resolved profiles");
                    };
                    seen.lock().unwrap().arguments.push(profiles.clone());
                    Ok(1)
                },
            ),
        );
    }
}

fn with_profile_game(test: impl FnOnce(&mut ServerGame<'_>, &std::path::Path, &Mutex<Seen>)) {
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("usercache.json");
    std::fs::write(
        &path,
        r#"[{"uuid":"00000000-0000-0000-0000-00000000000c","name":"OldName","expiresOn":"2099-01-01 00:00:00 +0000"},
             {"uuid":"00000000-0000-0000-0000-00000000001f","name":"OfflineOp","expiresOn":"2099-01-01 00:00:00 +0000"},
             {"uuid":"00000000-0000-0000-0000-000000000020","name":"Expired","expiresOn":"2000-01-01 00:00:00 +0000"}]"#,
    ).unwrap();
    let (cache, warnings) = UserCache::load(path);
    assert!(warnings.is_empty(), "{warnings:?}");
    let seen = Arc::new(Mutex::new(Seen::default()));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "profile_probe",
            Box::new(ProfileProbe(seen.clone())),
        )]
        .into(),
        || {
            with_guests(|game| {
                game.server.user_cache = Some(cache);
                test(game, scratch.path(), &seen);
            })
        },
    );
}

#[test]
#[ignore = "daily"]
fn profiles_are_available_online_offline_and_before_login() {
    in_own_process(
        "game::server_side::native_server_server::profile_tests::profiles_are_available_online_offline_and_before_login",
        || with_profile_game(check_profile_reads),
    );
}

fn check_profile_reads(game: &mut ServerGame<'_>, root: &std::path::Path, seen: &Mutex<Seen>) {
    let host = game.server.lan.as_mut().unwrap();
    Arc::get_mut(&mut host.hub.lan.shared)
        .unwrap()
        .config
        .singleplayer_profile = Some(("Owner".into(), 33));
    host.hub.ops.insert(33, 0); // The hub's automatic owner level with commands off.
    host.hub.ops.insert(12, 3); // An explicit LAN op entry.
    host.hub
        .update_guest(12, |guest| {
            guest.properties.push(AccountProperty {
                name: "textures".into(),
                value: "skin".into(),
                signature: None,
            })
        })
        .unwrap();
    let players = game.players();
    let online = players.profile_by_name("gUeSt12").unwrap();
    assert_eq!((online.uuid, online.name), (12, "guest12"));
    assert_eq!(online.properties().next().unwrap().value, "skin");
    assert_eq!(
        players.profile_by_uuid(12),
        Some(online),
        "the online name wins over the cache"
    );
    let cached = players.profile_by_name("oFfLiNeOp").unwrap();
    assert_eq!((cached.uuid, cached.name), (OFFLINE, "OfflineOp"));
    assert_eq!(cached.properties().len(), 0);
    assert_eq!(players.profile_by_uuid(OFFLINE), Some(cached));
    assert!(players.profile_by_name("Expired").is_none());
    assert!(players.profile_by_uuid(EXPIRED).is_none());
    assert!(players.profile_by_name("Unknown").is_none());
    assert!(players.profile_by_uuid(99).is_none());

    for (allow, guest_commands, uuid, name, expected) in [
        (false, false, 33, "Owner", PermissionLevel::All),
        (true, false, 99, "oWnEr", PermissionLevel::Owners),
        (true, false, 33, "Other", PermissionLevel::All),
        (true, false, 12, "guest12", PermissionLevel::All),
        (true, true, 12, "guest12", PermissionLevel::Gamemasters),
        (false, true, 12, "guest12", PermissionLevel::Admins),
        (false, true, 13, "guest13", PermissionLevel::All),
    ] {
        game.set_allow_commands(allow);
        game.lan_apply_settings(guest_commands, false);
        assert_eq!(
            game.players().profile_permission_level(uuid, name),
            expected
        );
        if let Some(guest) = game.server.lan.as_ref().unwrap().hub.guest(uuid as u32)
            && guest.uuid == uuid
        {
            assert_eq!(
                PermissionLevel::from_level(i32::from(guest.permission)),
                expected,
                "hub dispatch permission for {name} (allow={allow}, guest_commands={guest_commands}) must agree with profile_permission_level"
            );
        }
    }

    game.server.lan.as_mut().unwrap().hub.ops.insert(33, 1);
    assert_eq!(
        game.players().profile_permission_level(33, "Owner"),
        PermissionLevel::Owners,
        "a listed integrated owner is level 4 even with commands off"
    );

    let mut ops = OpList::new(root.join("ops.json"));
    ops.load_str(
        r#"[{"uuid":"00000000-0000-0000-0000-00000000001f","name":"PreviousName","level":3}]"#,
    )
    .unwrap();
    game.common.options.execution_role = ExecutionRole::DedicatedServer;
    game.server.server_lists.0 = Some(Arc::new(Mutex::new(Lists {
        ops,
        white: WhiteList::new(root.join("whitelist.json")),
        bans: BanList::new(root.join("banned-players.json")),
        ip_bans: IpBanList::new(root.join("banned-ips.json")),
        use_whitelist: false,
        enforce_whitelist: false,
        op_level: 4,
        player_idle_timeout: 0,
        stop: false,
        autosave: true,
    })));
    assert_eq!(
        game.players()
            .profile_permission_level(OFFLINE, "OfflineOp"),
        PermissionLevel::Admins
    );
    assert_eq!(
        game.players().profile_permission_level(99, "PreviousName"),
        PermissionLevel::All
    );
    game.native_player_login(OFFLINE, "OfflineOp", &mut false, &mut None);
    assert_eq!(seen.lock().unwrap().login, Some(PermissionLevel::Admins));
}

#[test]
#[ignore = "daily"]
fn failed_join_placement_does_not_populate_user_cache() {
    in_own_process(
        "game::server_side::native_server_server::profile_tests::failed_join_placement_does_not_populate_user_cache",
        || {
            with_profile_game(|game, root, _| {
                let conn = 14u32;
                let uuid = 44u128;
                game.server.next_entity = game.server.next_entity.max(conn);
                let (tx, _rx) = mpsc::channel();
                let guest = Guest::new(
                    Joined {
                        conn,
                        uuid,
                        name: "Unplaced".into(),
                        properties: Vec::new(),
                        info: ClientInformation::new(8),
                        brand: String::new(),
                        sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                        latency: Arc::new(AtomicI32::new(0)),
                        address: "127.0.0.1:1".parse().unwrap(),
                        expected: false,
                    },
                    conn as i32,
                    [3.0, 64.0, 5.0],
                    10,
                );
                game.server.lan.as_mut().unwrap().hub.guests.push(guest);

                // Force `lan_guest_joined` to fail opening the fallback level so placement aborts.
                let saved_dim = game.common.dim;
                let saved_resources =
                    std::mem::replace(&mut game.common.options.resources, root.to_path_buf());
                let saved_test_terrain =
                    std::mem::replace(&mut game.common.options.test_terrain, false);
                game.common.dim = Dim::Nether;

                let host = game.server.lan.take().unwrap();
                game.server.lan = game.lan_process_events(
                    host,
                    [HostEvent::Joined(conn, None)],
                    &[],
                    [0.5, 64.0, 0.5],
                );
                assert!(
                    !game
                        .server
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guest(conn)
                        .unwrap()
                        .placed,
                    "join must fail to place when the target level cannot open"
                );
                assert!(
                    game.server
                        .user_cache
                        .as_ref()
                        .unwrap()
                        .find_by_id(uuid)
                        .is_none(),
                    "failed join must not write the user cache"
                );
                assert!(game.players().profile_by_uuid(uuid).is_none());
                assert!(game.players().profile_by_name("Unplaced").is_none());

                // Once the target level is active, the join places the guest and populates the cache.
                game.common.dim = saved_dim;
                game.common.options.resources = saved_resources;
                game.common.options.test_terrain = saved_test_terrain;
                let host = game.server.lan.take().unwrap();
                game.server.lan = game.lan_process_events(
                    host,
                    [HostEvent::Joined(conn, None)],
                    &[],
                    [0.5, 64.0, 0.5],
                );
                assert!(
                    game.server
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guest(conn)
                        .unwrap()
                        .placed
                );
                let cached = game
                    .server
                    .user_cache
                    .as_ref()
                    .unwrap()
                    .find_by_id(uuid)
                    .unwrap();
                assert_eq!(cached.name, "Unplaced");
            })
        },
    );
}

#[test]
#[ignore = "daily"]
fn game_profile_arguments_use_the_vanilla_parser_and_deliver_profiles() {
    in_own_process(
        "game::server_side::native_server_server::profile_tests::game_profile_arguments_use_the_vanilla_parser_and_deliver_profiles",
        || {
            with_profile_game(|game, _, seen| {
                let (tx, rx) = mpsc::channel();
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |g| {
                        g.sender = PlayerSender::new(tx, Arc::default());
                    })
                    .unwrap();
                game.build_registries();
                game.register_native_commands();
                check_client_completion(rx);
                check_argument_resolution(game, seen);
            })
        },
    );
}

fn check_client_completion(rx: mpsc::Receiver<Out>) {
    let packet = rx
        .try_iter()
        .flat_map(|out| match out {
            Out::Packet(packet) => vec![packet],
            Out::Packets(packets) => packets,
            Out::Disconnect(_) => panic!("unexpected disconnect"),
        })
        .find_map(|body| {
            let mut r = Reader::new(&body);
            (r.varint().unwrap() == ids::play::clientbound::COMMANDS)
                .then(|| Commands::read(&mut r).unwrap())
        })
        .unwrap();
    let node = packet
        .nodes
        .iter()
        .find(|node| node.name == "inspect_profile")
        .unwrap();
    let argument = &packet.nodes[node.children[0] as usize];
    assert_eq!(
        argument.parser,
        Some((7, vec![])),
        "vanilla's game_profile parser has no properties"
    );
    assert_eq!(argument.suggestions, None);
    let client = from_packet(&packet, &Dispatcher::new());
    let reg = Registries {
        players: vec!["guest12".into(), "guest13".into()],
        permission: 2,
        ..Registries::default()
    };
    for (typed, expected) in [
        ("inspect_profile guest1", "guest12"),
        ("inspect_profile @", "@a"),
    ] {
        let parse = client.parse(typed, &reg);
        let (_, suggestions, ask_server) =
            client.client_suggestions_asking(&parse, typed.len(), &reg);
        assert!(!ask_server);
        assert!(
            suggestions
                .iter()
                .any(|candidate| candidate.text == expected),
            "{suggestions:?}"
        );
    }
}

fn check_argument_resolution(game: &mut ServerGame<'_>, seen: &Mutex<Seen>) {
    for (target, key) in [
        ("Missing", "argument.player.unknown"),
        (
            "00000000-0000-0000-0000-00000000001f",
            "argument.player.unknown",
        ),
        ("@a[name=Missing]", "argument.entity.notfound.player"),
    ] {
        game.server.chat_capture = Some(Vec::new());
        assert!(game.perform_native_console_command(&format!("inspect_profile {target}")));
        let output = game.server.chat_capture.take().unwrap();
        assert_eq!(
            output[0].plain(),
            Text::translate(key, &[]).plain(),
            "{target}"
        );
    }
    assert!(seen.lock().unwrap().arguments.is_empty());
    assert!(game.perform_native_console_command("inspect_profile offlineop"));
    assert!(game.perform_native_console_command("inspect_profile @a"));
    assert_eq!(
        seen.lock().unwrap().arguments,
        [
            vec![(OFFLINE, "OfflineOp".into())],
            vec![(12, "guest12".into()), (13, "guest13".into())]
        ],
    );
}
