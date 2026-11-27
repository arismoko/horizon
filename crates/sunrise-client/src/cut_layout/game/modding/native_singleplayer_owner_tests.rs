//! `Players::singleplayer_profile` and `PlayerRef::is_singleplayer_owner` (#4636): the profile
//! the integrated server is created with, whole and fixed before its startup hooks and its hub,
//! matched to players by name ignoring case (`IntegratedServer.isSingleplayerOwner`), and
//! nothing on a dedicated server (`DedicatedServer.isSingleplayerOwner`).
//!
//! Each test runs again in a child process of its own: the game fixture loads the resources
//! in-process and installs process-wide tables (the cauldron contents among them).

use mod_api::{Hooks, ModEntry, PlayerRef, ServerHost, prelude::*};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicI32, AtomicUsize},
};
use sunrise_core::profile::AccountProperty;
use sunrise_gameplay::game::server_side::lan::LanSettings;
use sunrise_gameplay::game::server_side::native_singleplayer_owner::SingleplayerProfile;
use sunrise_gameplay_common::game::execution_role::ExecutionRole;
use sunrise_protocol::packets::configuration::ClientInformation;
use sunrise_sim::lan::{
    Joined, NetConfig, OwnerProfile, PlayerSender, guest::Guest, hub::HubConfig,
};

const OWNER: u128 = 1;
const CHILD: &str = "SUNRISE_SINGLEPLAYER_OWNER_CHILD";

/// Runs `test` (this module's test of that name) in a fresh process, and false there.
fn in_child(test: &str) -> bool {
    // The shared helper checks that exactly this test ran and gives the child a scratch home.
    let name = format!("cut_layout::game::server_side::native_singleplayer_owner_tests::{test}");
    super::native_entity_refs::tests::run_fresh_cases(&name, CHILD, &["owner"])
}

/// A profile as a mod reads it: name, UUID and each property.
type Read = (String, u128, Vec<(String, String, Option<String>)>);

fn read(host: &dyn ServerHost) -> Option<Read> {
    let profile = host.players().singleplayer_profile()?;
    let properties = profile
        .properties()
        .map(|p| {
            let signature = p.signature.map(str::to_owned);
            (p.name.to_owned(), p.value.to_owned(), signature)
        })
        .collect();
    Some((profile.name.to_owned(), profile.uuid, properties))
}

fn textures() -> AccountProperty {
    AccountProperty {
        name: "textures".into(),
        value: "e30=".into(),
        signature: Some("c2lnbmVk".into()),
    }
}

fn launched() -> Read {
    let textures = vec![("textures".into(), "e30=".into(), Some("c2lnbmVk".into()))];
    ("Owner".into(), OWNER, textures)
}

/// The account fetch the window starts as the world opens, answering only after `delay`.
fn slow_account(delay: std::time::Duration) -> OwnerProfile {
    OwnerProfile::new(move || {
        std::thread::sleep(delay);
        vec![textures()]
    })
}

/// Opens the game's hub as `role`'s server: the startup that follows the profile.
fn open_hub(game: &mut sunrise_gameplay::game::ServerGame<'_>, role: ExecutionRole) {
    let settings = LanSettings {
        port: 0,
        guest_commands: false,
        force_game_mode: false,
    };
    let singleplayer_profile = game
        .server
        .singleplayer_profile
        .as_ref()
        .map(|profile| (profile.name.clone(), profile.uuid));
    let net = NetConfig {
        listen: false,
        announce: false,
        lod_distance: 0,
        singleplayer_profile,
        ..NetConfig::lan(0, String::new(), 4)
    };
    let config = HubConfig {
        default_game_mode: 0,
        guest_commands: false,
        force_game_mode: false,
        hardcore: false,
        view_distance: 2,
        simulation_distance: 2,
        dimension: game.dim.id().to_owned(),
        allow_flight: true,
        seed: 0,
        max_players: 4,
        integrated: role == ExecutionRole::IntegratedServer,
        levels: game.server.level_shapes.clone(),
        level_seeds: game.server.levels.0.runtime.seeds(),
    };
    game.lan_host(settings, net, config).unwrap();
}

/// A player joins as `name` with `uuid` and returns their handle.
fn join(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    conn: u32,
    uuid: u128,
    name: &str,
) -> PlayerRef {
    let (tx, _rx) = std::sync::mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn,
            uuid,
            name: name.to_owned(),
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
    guest.placed = true;
    game.server.lan.as_mut().unwrap().hub.guests.push(guest);
    let players = game.players().server_players();
    *players
        .iter()
        .find(|player| player.entity().id() == conn)
        .unwrap()
}

fn leave(game: &mut sunrise_gameplay::game::ServerGame<'_>, conn: u32) {
    game.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .retain(|g| g.conn != conn);
}

/// What each startup hook read of the profile.
struct StartupReads(Arc<Mutex<Vec<(&'static str, Option<Read>)>>>);
#[mod_api::__subscriptions]
impl Hooks for StartupReads {
    fn server_about_to_start(
        &self,
        host: &mut dyn ServerHost,
        _: &mut mod_api::ServerAboutToStartEvent,
    ) {
        self.0.lock().unwrap().push(("about", read(host)));
    }
    fn server_starting(&self, host: &mut dyn ServerHost, _: &mut mod_api::ServerStartingEvent) {
        self.0.lock().unwrap().push(("starting", read(host)));
    }
}

#[test]
#[ignore = "daily"]
fn the_startup_hooks_read_the_whole_launch_profile_before_the_hub() {
    if in_child("the_startup_hooks_read_the_whole_launch_profile_before_the_hub") {
        return;
    }
    let reads = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new(
        "owner_read",
        Box::new(StartupReads(reads.clone())),
    )];
    mod_platform::testing::with_mods(mods.into(), || {
        super::native_entity_refs::tests::with_game(|game| {
            // The account answers after the server is created: its creation waits for it,
            // as `getGameProfile` joins `profileFuture`, so no read sees it half made.
            let account = slow_account(std::time::Duration::from_millis(200));
            game.server.singleplayer_profile = Some(SingleplayerProfile::launched(
                "Owner".into(),
                OWNER,
                Some(&account),
            ));
            assert!(game.server.lan.is_none(), "no hub yet");
            game.native_server_about_to_start(false);
            game.native_server_starting(false);
            open_hub(game, ExecutionRole::IntegratedServer);
            assert_eq!(
                read(game),
                Some(launched()),
                "the same snapshot once the hub is up"
            );
        });
    });
    let whole = Some(launched());
    assert_eq!(
        *reads.lock().unwrap(),
        [("about", whole.clone()), ("starting", whole)]
    );
}

#[test]
#[ignore = "daily"]
fn the_integrated_owner_is_the_launch_profile_matched_by_name() {
    if in_child("the_integrated_owner_is_the_launch_profile_matched_by_name") {
        return;
    }
    super::native_entity_refs::tests::with_game(|game| {
        let account = slow_account(std::time::Duration::ZERO);
        game.server.singleplayer_profile = Some(SingleplayerProfile::launched(
            "Owner".into(),
            OWNER,
            Some(&account),
        ));
        open_hub(game, ExecutionRole::IntegratedServer);
        // Commands off: the owner is no operator (`IntegratedPlayerList`).
        let hub = &mut game.server.lan.as_mut().unwrap().hub;
        hub.ops.insert(OWNER, 0);
        hub.singleplayer_owner = Some(OWNER);
        assert_eq!(read(game), Some(launched()), "before the owner joins");

        let guest = join(game, 12, 12, "guest12");
        game.server.lan.as_mut().unwrap().hub.ops.insert(12, 4);
        assert!(
            !guest.is_singleplayer_owner(game),
            "a guest who joined first, op 4"
        );

        let owner = join(game, 13, OWNER, "Owner");
        assert!(owner.is_singleplayer_owner(game), "the owner at op 0");
        assert!(!guest.is_singleplayer_owner(game));

        leave(game, 13);
        assert_eq!(read(game), Some(launched()), "after the owner leaves");
        assert!(!owner.is_singleplayer_owner(game), "a stale handle");

        let returning = join(game, 14, OWNER, "Owner");
        assert!(returning.is_singleplayer_owner(game), "the returning owner");
        assert!(
            !owner.is_singleplayer_owner(game),
            "the old handle stays stale"
        );
        leave(game, 14);

        // The name decides, ignoring case; the UUID plays no part.
        let namesake = join(game, 15, 99, "oWNER");
        assert!(namesake.is_singleplayer_owner(game));
        leave(game, 15);
        let renamed = join(game, 16, OWNER, "NotOwner");
        assert!(
            !renamed.is_singleplayer_owner(game),
            "the owner's UUID, another name"
        );
        assert_eq!(
            read(game),
            Some(launched()),
            "the profile keeps the owner's UUID"
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_dedicated_server_has_no_singleplayer_owner() {
    if in_child("a_dedicated_server_has_no_singleplayer_owner") {
        return;
    }
    let root = super::native_entity_refs::tests::fixture_root();
    let role = ExecutionRole::DedicatedServer;
    super::native_entity_refs::tests::with_game_role(root, role, |game| {
        open_hub(game, role);
        let op = join(game, 12, OWNER, "Owner");
        game.server.lan.as_mut().unwrap().hub.ops.insert(OWNER, 4);
        assert_eq!(read(game), None);
        assert!(!op.is_singleplayer_owner(game));
    });
}
