//! Witness for `Pig.mobInteract` and `Strider.mobInteract`: the real guest
//! Interact route, with only the world and connection constructed by the test.
use serde_json::json;
use std::sync::{Arc, atomic::AtomicI32, mpsc};
use sunrise_core::geometry::Vec3;
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::Chunk;
use sunrise_protocol::{
    buf::Reader,
    packets::{
        configuration::ClientInformation,
        play::{Interact, PlayerInput, Serverbound},
    },
};
use sunrise_sim::{
    lan::{
        Joined, NetConfig, PlayerSender,
        guest::{Guest, InboxItem},
        hub::HubConfig,
    },
    mobs::Mob,
};
use {
    sunrise_gameplay::game::lan::LanSettings, sunrise_gameplay::game::native_entity_refs::tests,
    sunrise_gameplay_common::game::Dim,
    sunrise_gameplay_common::game::execution_role::ExecutionRole,
};

const TEST: &str = "cut_layout::game::server_side::mounts::steerable_boarding_tests::interact_with_a_pig_or_strider";
const CHILD: &str = "SUNRISE_STEERABLE_BOARDING_CHILD";
const VEHICLE: u32 = 42;
const MOB_RIDER: u32 = 43;
const PLAYER: u32 = 44;
const OTHER_PLAYER: u32 = 45;
const CONN: u32 = 1;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Passenger {
    Empty,
    Mob,
    Guest,
}

fn scene(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    kind: &str,
    passenger: Passenger,
    saddled: bool,
    held: Option<&str>,
    secondary: bool,
) -> serde_json::Value {
    let at = Vec3::new(0.5, 100.0, 2.5);
    let mob_type = |id| game.res.mob_types().get(id).unwrap().clone();
    let mut vehicle = Mob::new(VEHICLE, mob_type(kind), at, 0.0);
    vehicle.set_saddle(saddled.then(|| ItemStack::new("minecraft:saddle", 1)));
    let mob_rider = (passenger == Passenger::Mob).then(|| {
        let mut rider = Mob::new(MOB_RIDER, mob_type("minecraft:chicken"), at, 0.0);
        rider.extra.mount = Some(VEHICLE);
        rider
    });
    game.mobs.clear();
    game.mobs.push(vehicle);
    if let Some(rider) = mob_rider {
        game.mobs.push(rider);
    }

    let (sender, _packets) = mpsc::channel();
    let make_guest = |conn, id: u32, name: &str, position| {
        let mut guest = Guest::new(
            Joined {
                conn,
                uuid: u128::from(conn),
                name: name.into(),
                properties: Vec::new(),
                info: ClientInformation::new(2),
                brand: "fixture".into(),
                sender: PlayerSender::new(sender.clone(), Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            id as i32,
            position,
            2,
        );
        guest.dimension = game.dim.id().to_owned();
        guest.placed = true;
        guest.loaded = true;
        guest.load_timeout = 0;
        guest
    };
    let mut guest = make_guest(CONN, PLAYER, "BoardingPlayer", [0.5, 100.0, 0.5]);
    let guest_rider = (passenger == Passenger::Guest).then(|| {
        let mut rider = make_guest(CONN + 1, OTHER_PLAYER, "SeatedPlayer", [at.x, at.y, at.z]);
        rider.vehicle = Some(VEHICLE);
        rider
    });
    guest.inventory[guest.selected] = held.map(|item| ItemStack::new(item, 1));
    // PlayerInput and Interact agree about secondary use. Interact's handling
    // of stale PlayerInput is a different packet-parity problem.
    for packet in [
        Serverbound::PlayerInput(PlayerInput {
            flags: if secondary { 32 } else { 0 },
        }),
        Serverbound::Interact(Interact {
            id: VEHICLE as i32,
            hand: 0,
            location: [0.0, 0.45, 0.0],
            secondary,
        }),
    ] {
        let encoded = packet.encode();
        let mut reader = Reader::new(&encoded);
        let id = reader.varint().unwrap();
        guest.inbox.push_back(InboxItem::Packet(
            Serverbound::decode(id, reader.rest()).unwrap(),
            None,
        ));
    }
    let guests = &mut game.server.lan.as_mut().unwrap().hub.guests;
    guests.clear();
    guests.push(guest);
    if let Some(rider) = guest_rider {
        guests.push(rider);
    }
    game.reset_entity_residency();
    assert_eq!(
        game.has_seated_passengers(VEHICLE),
        passenger != Passenger::Empty,
        "fixture seat"
    );
    game.server.level_pass = Default::default();
    game.tick_lan_packets();

    let guest = game.server.lan.as_ref().unwrap().hub.guest(CONN).unwrap();
    let mut seats = game
        .seated_passengers(VEHICLE)
        .into_iter()
        .map(|(_, id, player)| match (id, player) {
            (MOB_RIDER, false) => "mob",
            (PLAYER, true) => "player",
            (OTHER_PLAYER, true) => "other_player",
            _ => panic!("unexpected passenger {id}"),
        })
        .collect::<Vec<_>>();
    seats.sort_unstable();
    json!({"seats": seats, "riding": guest.vehicle == Some(VEHICLE),
           "held": guest.inventory[guest.selected].as_ref().map_or(0, |stack| stack.count),
           "saddled": game.mob_object(VEHICLE).unwrap().saddle_item().is_some()})
}

#[test]
#[ignore = "daily"]
fn interact_with_a_pig_or_strider() {
    run_interact_with_a_pig_or_strider();
}

pub(crate) fn run_interact_with_a_pig_or_strider() {
    if tests::run_fresh_cases(TEST, CHILD, &["boarding"]) {
        return;
    }
    tests::with_game_role(
        tests::fixture_root(),
        ExecutionRole::DedicatedServer,
        |game| {
            game.options.dedicated = Some(Vec::new());
            game.build_registries();
            game.lan_host(
                LanSettings {
                    port: 0,
                    guest_commands: false,
                    force_game_mode: false,
                },
                NetConfig {
                    listen: false,
                    announce: false,
                    lod_distance: 0,
                    ..NetConfig::lan(0, String::new(), 16)
                },
                HubConfig {
                    default_game_mode: 0,
                    guest_commands: false,
                    force_game_mode: false,
                    hardcore: false,
                    view_distance: 2,
                    simulation_distance: 2,
                    dimension: Dim::Overworld.id().to_owned(),
                    allow_flight: true,
                    seed: 0,
                    max_players: 16,
                    integrated: false,
                    levels: game.server.level_shapes.clone(),
                    level_seeds: game.server.levels.0.runtime.seeds(),
                },
            )
            .unwrap();
            game.server
                .streaming
                .insert_remote(&mut game.common.world, Chunk::empty(0, 0));
            game.server.next_entity = OTHER_PLAYER + 1;
            let report = std::env::var_os("SUNRISE_STEERABLE_BOARDING_REPORT");
            let mut rows = serde_json::Map::new();
            for (kind, food) in [
                ("pig", "minecraft:carrot"),
                ("strider", "minecraft:warped_fungus"),
            ] {
                use Passenger::{Empty, Guest, Mob};
                for (name, passenger, saddled, held, secondary, seats) in [
                    ("empty", Empty, true, None, false, vec!["player"]),
                    ("occupied", Mob, true, None, false, vec!["mob"]),
                    ("secondary", Empty, true, None, true, vec![]),
                    ("occupied_feed", Mob, true, Some(food), false, vec!["mob"]),
                    (
                        "occupied_saddle",
                        Mob,
                        false,
                        Some("minecraft:saddle"),
                        false,
                        vec!["mob"],
                    ),
                    (
                        "player_occupied",
                        Guest,
                        true,
                        None,
                        false,
                        vec!["other_player"],
                    ),
                    (
                        "player_occupied_saddle",
                        Guest,
                        false,
                        Some("minecraft:saddle"),
                        false,
                        vec!["other_player"],
                    ),
                ] {
                    let label = format!("{kind}/{name}");
                    let observed = scene(
                        game,
                        &format!("minecraft:{kind}"),
                        passenger,
                        saddled,
                        held,
                        secondary,
                    );
                    if report.is_none() {
                        assert_eq!(
                            observed,
                            json!({"seats": seats, "riding": name == "empty",
                                                "held": 0, "saddled": true}),
                            "{label}"
                        );
                    }
                    rows.insert(label, observed);
                }
            }
            if let Some(path) = report {
                std::fs::write(path, serde_json::to_vec_pretty(&rows).unwrap()).unwrap();
            }
        },
    );
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
