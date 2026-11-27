//! tools/test_block_entity_reach_parity.py's game side: each case through the draws the game
//! makes, not `Reach` alone. A case places one block entity at its position, sets the client's
//! render distance (and the server's view distance, when the case has one) and asks the frame's
//! own geometry whether anything is drawn from that camera. The End gateway, which draws through
//! the frame's renderer, is asked the frame's off-screen selection instead.
#![cfg(feature = "client")]
use std::sync::Arc;
use sunrise_items::items::ItemStack;
use sunrise_level::{
    chunk::Chunk,
    containers::{Beacon, BlockEntity},
};
use sunrise_world::piston_shape::MovingPistonState;

/// Places the case's block entity and reports whether the game draws it from the case's camera.
fn drawn(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    block: &str,
    pos: [i32; 3],
    camera: glam::DVec3,
) -> Option<bool> {
    let registry = game.pack.assets.registry;
    let state = |spec: &str| {
        let (name, props) = spec.split_once('[').unwrap_or((spec, ""));
        let mut state = registry
            .state(registry.default_state(name).expect("a block"))
            .unwrap()
            .clone();
        for kv in props
            .trim_end_matches(']')
            .split(',')
            .filter(|kv| !kv.is_empty())
        {
            let (k, v) = kv.split_once('=').unwrap();
            state.properties.insert(k.into(), v.into());
        }
        registry.id(&state).unwrap()
    };
    let item = || Some(ItemStack::new("minecraft:beef", 1));
    let (spec, entity) = match block {
        "minecraft:campfire" | "minecraft:soul_campfire" => (
            block.to_owned(),
            Some(BlockEntity::Campfire(Box::new(
                sunrise_level::chunk::block_entity::Campfire {
                    items: [item(), None, None, None],
                    ..Default::default()
                },
            ))),
        ),
        b if b.ends_with("_shelf") => (
            block.to_owned(),
            Some(BlockEntity::Shelf(Box::new(
                sunrise_level::chunk::block_entity::Shelf {
                    items: [item(), None, None],
                    ..Default::default()
                },
            ))),
        ),
        "minecraft:suspicious_sand" => (
            "minecraft:suspicious_sand[dusted=1]".to_owned(),
            Some(BlockEntity::Brushable(Box::new(
                sunrise_level::chunk::block_entity::Brushable {
                    item: item(),
                    hit_direction: Some(1),
                    ..Default::default()
                },
            ))),
        ),
        "minecraft:beacon" => (
            block.to_owned(),
            Some(BlockEntity::Beacon(Box::new(Beacon {
                levels: 1,
                sections: vec![(0xff_ff_ff, 4)],
                ..Default::default()
            }))),
        ),
        "minecraft:moving_piston" | "minecraft:end_gateway" => (block.to_owned(), None),
        _ => return None,
    };
    let state = state(&spec);
    let column = (pos[0] >> 4, pos[2] >> 4);
    game.world.block_entities.clear();
    game.world.moving_pistons.clear();
    game.world.moving_piston_fallback.clear();
    game.world.chunks.clear();
    game.world
        .chunks
        .insert(column, Arc::new(Chunk::empty(column.0, column.1)));
    game.world.set(pos[0], pos[1], pos[2], state).unwrap();
    let key = (pos[0], pos[1], pos[2]);
    let mut out = Vec::new();
    match block {
        "minecraft:end_gateway" => return Some(game.block_entity_reach(state.0).off_screen()),
        "minecraft:moving_piston" => {
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.moving_pistons.push((
                pos,
                MovingPistonState {
                    moved_state: stone,
                    direction: sunrise_core::geometry::Direction::East,
                    extending: true,
                    source: false,
                    progress: 0.5,
                    previous_progress: 0.5,
                },
            ));
            game.moving_piston_geometry(camera, 0.0, &mut out);
        }
        "minecraft:beacon" => {
            game.world.block_entities.insert(key, entity.unwrap());
            game.beacon_geometry(camera, 0.0, &mut out);
        }
        "minecraft:suspicious_sand" => {
            game.world.block_entities.insert(key, entity.unwrap());
            game.brushable_geometry(camera, &mut out);
        }
        b if b.ends_with("_shelf") => {
            game.world.block_entities.insert(key, entity.unwrap());
            game.shelf_geometry(camera, &mut out);
        }
        _ => {
            game.world.block_entities.insert(key, entity.unwrap());
            game.campfire_geometry(camera, &mut out);
        }
    }
    Some(!out.is_empty())
}

#[test]
#[ignore = "run through tools/test_block_entity_reach_parity.py"]
fn block_entity_draws_match_the_vanilla_cases() {
    run_block_entity_draws_match_the_vanilla_cases();
}

pub(crate) fn run_block_entity_draws_match_the_vanilla_cases() {
    let (Ok(cases), Ok(out)) = (
        std::env::var("SUNRISE_REACH_DRAW_CASES"),
        std::env::var("SUNRISE_REACH_DRAW_OUT"),
    ) else {
        panic!("run by tools/test_block_entity_reach_parity.py");
    };
    let mut resources = None;
    // A client: it loads what the draws need (the item atlas, baked block models).
    let mut game = crate::game::client_side::beacon_cull_tests::game_as_client(
        &mut resources,
        sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
    );
    let cases: Vec<serde_json::Value> = std::fs::read_to_string(cases)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let mut lines = vec![String::new(); cases.len()];
    // The cases without a server view distance first, then connected to one: a server that sent
    // its view distance caps the render distance (`Options.getEffectiveRenderDistance`).
    let mut order: Vec<usize> = (0..cases.len()).collect();
    order.sort_by_key(|&i| cases[i]["serverDistance"].as_i64().unwrap_or(0) > 0);
    for i in order {
        let case = &cases[i];
        let server = case["serverDistance"].as_i64().unwrap_or(0) as i32;
        if server > 0 {
            if game.client.remote.is_none() {
                let (client, _server) = sunrise_protocol::connection::Pipe::pair();
                game.connect_local(client, "Reach").unwrap();
            }
            game.client.remote.as_mut().unwrap().view_distance = server;
        }
        game.client.settings.render_distance = case["renderDistance"].as_i64().unwrap() as i32;
        let int = |v: &serde_json::Value| v.as_i64().unwrap() as i32;
        let pos = [
            int(&case["pos"][0]),
            int(&case["pos"][1]),
            int(&case["pos"][2]),
        ];
        let float = |v: &serde_json::Value| v.as_f64().unwrap();
        let camera = glam::DVec3::new(
            float(&case["camera"][0]),
            float(&case["camera"][1]),
            float(&case["camera"][2]),
        );
        let block = case["block"].as_str().unwrap();
        let answer = drawn(&mut game, block, pos, camera);
        let field = if block == "minecraft:end_gateway" {
            "offScreen"
        } else {
            "shouldRender"
        };
        lines[i] = serde_json::json!({ field: answer }).to_string();
    }
    std::fs::write(out, lines.join("\n") + "\n").unwrap();
}
