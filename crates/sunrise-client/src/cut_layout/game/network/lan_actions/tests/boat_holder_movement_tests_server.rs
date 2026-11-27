use std::sync::{Arc, atomic::AtomicI32, mpsc};
use sunrise_core::position::ChunkPos;
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::Chunk;
use sunrise_protocol::packets::configuration::ClientInformation;
use sunrise_sim::lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig};

pub(crate) fn scene<'a>(
    resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
) -> (sunrise_gameplay::game::ServerGame<'a>, mpsc::Receiver<Out>) {
    let mut g = sunrise_gameplay::game::game_tests::tests::server_game(resources);
    g.build_registries();
    g.set_allow_commands(true);
    let stone = g
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap()
        .0;
    let mut chunk = Chunk::empty(0, 0);
    for x in 0..16 {
        for z in 0..16 {
            chunk.set(x, 99, z, stone);
        }
    }
    g.world.chunks.insert((0, 0), Arc::new(chunk));
    g.block_ticks.add_empty_chunk(ChunkPos::new(0, 0)).unwrap();
    assert!(g.run_command("forceload add 0 0"));
    assert!(g.run_command("tick freeze"));
    assert!(g.run_command("summon oak_boat .5 100 2.5 {UUID:[I;7,7,7,7]}"));
    g.lan_host(
        sunrise_gameplay::game::server_side::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            listen: false,
            announce: false,
            ..NetConfig::loopback(0, "boat-holder-unit".into(), 2)
        },
        HubConfig {
            default_game_mode: 0,
            guest_commands: true,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: "minecraft:overworld".into(),
            allow_flight: false,
            seed: 0,
            max_players: 2,
            integrated: false,
            levels: Default::default(),
            level_seeds: Default::default(),
        },
    )
    .unwrap();
    let (tx, rx) = mpsc::channel();
    let guest_entity_id = g.take_entity_id() as i32;
    let mut guest = Guest::new(
        Joined {
            conn: 1,
            uuid: 1,
            name: "BoatHolder".into(),
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: "unit".into(),
            sender: PlayerSender::new(tx, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:1".parse().unwrap(),
            expected: false,
        },
        guest_entity_id,
        [0.5, 100.0, 0.5],
        2,
    );
    guest.placed = true;
    guest.loaded = true;
    guest.load_timeout = 0;
    guest.dimension = "minecraft:overworld".into();
    guest.inventory[0] = Some(ItemStack::new("minecraft:lead", 1));
    g.server.lan.as_mut().unwrap().hub.guests.push(guest);
    g.sync_tick_chunks();
    g.server.ticked_entities_through = g.server.next_entity;
    (g, rx)
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
