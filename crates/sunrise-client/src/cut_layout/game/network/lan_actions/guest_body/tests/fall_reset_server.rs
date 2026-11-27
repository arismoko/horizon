use super::guest;
use crate::game::lan_actions::guest_body::tests::fall_reset::CRITERIA;
use std::{path::Path, sync::Arc};
use sunrise_core::position::ChunkPos;
use sunrise_level::chunk::Chunk;
use sunrise_sim::{
    advancements::Tree,
    lan::{NetConfig, hub::HubConfig},
};

/// A server with one survival guest in the air at 12.5 110 12.5, over a stone floor at y 99.
pub(crate) fn scene<'a>(
    resources: &'a mut Option<Arc<sunrise_gameplay_common::game::Resources>>,
    scratch: &Path,
) -> sunrise_gameplay::game::ServerGame<'a> {
    let mut g = sunrise_gameplay::game::game_tests::tests::server_game(resources);
    g.build_registries();
    g.set_allow_commands(true);
    let folder = scratch.join("data/fall/advancement");
    std::fs::create_dir_all(&folder).unwrap();
    g.server.world_root = scratch.to_path_buf();
    for (name, distance) in CRITERIA.into_iter().zip([
        serde_json::Value::Null,
        serde_json::json!({"max": 1.0}),
        serde_json::json!({"min": 4.9, "max": 5.1}),
    ]) {
        let mut criterion = serde_json::json!({"trigger": "minecraft:fall_from_height"});
        if !distance.is_null() {
            criterion["conditions"] = serde_json::json!({"distance": {"y": distance}});
        }
        let advancement = serde_json::json!({"criteria": {name: criterion}});
        std::fs::write(folder.join(format!("{name}.json")), advancement.to_string()).unwrap();
    }
    g.adv.tree = Arc::new(Tree::load(&scratch.join("data")));
    assert_eq!(g.adv.tree.list.len(), CRITERIA.len());
    let stone = g.pack.assets.registry.default_state("minecraft:stone");
    let mut chunk = Chunk::empty(0, 0);
    for x in 0..16 {
        for z in 0..16 {
            chunk.set(x, 99, z, stone.unwrap().0);
        }
    }
    g.world.chunks.insert((0, 0), Arc::new(chunk));
    g.block_ticks.add_empty_chunk(ChunkPos::new(0, 0)).unwrap();
    assert!(g.run_command("forceload add 0 0"));
    assert!(g.run_command("tick freeze"));
    g.lan_host(
        sunrise_gameplay::game::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        },
        NetConfig {
            listen: false,
            announce: false,
            ..NetConfig::loopback(0, "guest-fall-reset".into(), 2)
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
    let mut guest = guest(1);
    guest.position = [12.5, 110.0, 12.5];
    guest.placed = true;
    guest.loaded = true;
    guest.dimension = "minecraft:overworld".into();
    guest.game_mode = 0;
    guest.on_ground = false;
    guest.load_timeout = 0;
    g.server.lan.as_mut().unwrap().hub.guests.push(guest);
    g.sync_tick_chunks();
    g.server.ticked_entities_through = g.server.next_entity;
    g
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
