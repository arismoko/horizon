//! tools/test_block_entity_item_geometry_parity.py: the items campfires, suspicious sand
//! and gravel, shelves and vaults hand to the item renderer, case by case.
#![cfg(feature = "client")]
use std::sync::Arc;
use sunrise_assets::item_model::DisplayContext;
use sunrise_items::items::ItemStack;
use sunrise_level::{
    chunk::{Chunk, Light},
    containers::BlockEntity,
};
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_client_gameplay::game::item_draw::BlockItemDraw,
};

fn stack(item: &serde_json::Value) -> ItemStack {
    ItemStack::new(item[0].as_str().unwrap(), item[1].as_u64().unwrap() as u32)
}

/// One case's block, its block entity and its light (sky, block) over every column around it.
fn place(game: &mut sunrise_client_gameplay::game::ClientGame<'_>, case: &serde_json::Value) {
    let int = |v: &serde_json::Value| v.as_i64().unwrap() as i32;
    let key = (
        int(&case["pos"][0]),
        int(&case["pos"][1]),
        int(&case["pos"][2]),
    );
    game.world.block_entities.clear();
    game.world.chunks.clear();
    let packed = (int(&case["light"][0]) << 4 | int(&case["light"][1])) as u8;
    for dx in -1..=1 {
        for dz in -1..=1 {
            let column = ((key.0 >> 4) + dx, (key.2 >> 4) + dz);
            let mut chunk = Chunk::empty(column.0, column.1);
            for section in &mut chunk.sections {
                Arc::make_mut(section).light = Light::Uniform(packed);
            }
            game.world.chunks.insert(column, Arc::new(chunk));
        }
    }
    let registry = game.pack.assets.registry;
    let name = case["blockName"].as_str().unwrap();
    let mut state = registry
        .state(registry.default_state(name).unwrap())
        .unwrap()
        .clone();
    for (k, v) in case["props"].as_object().unwrap() {
        state
            .properties
            .insert(k.clone(), v.as_str().unwrap().to_owned());
    }
    let state = registry.id(&state).unwrap();
    game.world.set(key.0, key.1, key.2, state).unwrap();
    // A lit chunk's light storage already includes the source block's own emission.
    let emission = game.res.facts().get(state).emission;
    if emission > packed & 15 {
        let column = game
            .world
            .chunks
            .get_mut(&(key.0 >> 4, key.2 >> 4))
            .unwrap();
        let chunk = Arc::make_mut(column);
        let section = chunk.section_index(key.1).unwrap();
        let mut light = Box::new([packed; 4096]);
        light[sunrise_level::chunk::index(key.0, key.1, key.2)] = (packed & 0xf0) | emission;
        Arc::make_mut(&mut chunk.sections[section]).light = Light::packed(light);
    }
    let slots = |n: usize| {
        let mut items = vec![None; n];
        for e in case["items"].as_array().into_iter().flatten() {
            items[e[0].as_u64().unwrap() as usize] = Some(stack(&serde_json::json!([e[1], e[2]])));
        }
        items
    };
    let entity = match case["kind"].as_str().unwrap() {
        "campfire" => {
            BlockEntity::Campfire(Box::new(sunrise_level::chunk::block_entity::Campfire {
                items: slots(4).try_into().unwrap(),
                ..Default::default()
            }))
        }
        "shelf" => BlockEntity::Shelf(Box::new(sunrise_level::chunk::block_entity::Shelf {
            items: slots(3).try_into().unwrap(),
            align_to_bottom: case["align"].as_bool().unwrap(),
        })),
        "brushable" => {
            BlockEntity::Brushable(Box::new(sunrise_level::chunk::block_entity::Brushable {
                item: Some(stack(&case["item"])),
                hit_direction: Some(int(&case["hit"])),
                ..Default::default()
            }))
        }
        _ => {
            let spin = &case["spin"];
            BlockEntity::Vault(Box::new(sunrise_level::vault::Vault {
                display_item: case.get("item").map(stack),
                o_spin: spin[0].as_f64().unwrap() as f32,
                spin: spin[1].as_f64().unwrap() as f32,
                ..Default::default()
            }))
        }
    };
    game.world.block_entities.insert(key, entity);
}

/// `ItemDisplayContext.getSerializedName`.
fn context_name(display: DisplayContext) -> &'static str {
    match display {
        DisplayContext::Fixed => "fixed",
        DisplayContext::OnShelf => "on_shelf",
        DisplayContext::Ground => "ground",
        other => panic!("no block entity draws {other:?}"),
    }
}

fn row(d: BlockItemDraw) -> serde_json::Value {
    // Sunrise packs sky * 16 in bits 0..8 and block * 16 in 8..16; vanilla's light coords are
    // block << 4 | sky << 20.
    let (sky, block) = (d.light & 0xff, d.light >> 8 & 0xff);
    serde_json::json!({
        "item": &*d.stack.item,
        "count": d.stack.count,
        "context": context_name(d.display),
        "seed": d.seed,
        "matrix": d.pose.to_cols_array().iter().map(|v| format!("{:x}", v.to_bits())).collect::<Vec<_>>(),
        "light": block | sky << 16,
    })
}

fn case_rows(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    case: &serde_json::Value,
) -> serde_json::Value {
    place(game, case);
    let c = &case["camera"];
    let camera = glam::DVec3::new(
        c[0].as_f64().unwrap(),
        c[1].as_f64().unwrap(),
        c[2].as_f64().unwrap(),
    );
    let mut rows = Vec::new();
    match case["kind"].as_str().unwrap() {
        "campfire" => game.campfire_items(camera, |d| rows.push(row(d))),
        "shelf" => game.shelf_items(camera, |d| rows.push(row(d))),
        "brushable" => game.brushable_items(camera, |d| rows.push(row(d))),
        _ => {
            let partial = case["spin"][2].as_f64().unwrap();
            game.vault_items(camera, partial, |d| rows.push(row(d)));
        }
    }
    serde_json::json!({ "name": case["name"], "submits": rows })
}

#[test]
#[ignore = "run through tools/test_block_entity_item_geometry_parity.py"]
fn block_entity_items_match_the_vanilla_cases() {
    run_block_entity_items_match_the_vanilla_cases();
}

pub(crate) fn run_block_entity_items_match_the_vanilla_cases() {
    let (Ok(cases), Ok(out)) = (
        std::env::var("SUNRISE_BLOCK_ENTITY_ITEM_CASES"),
        std::env::var("SUNRISE_BLOCK_ENTITY_ITEM_OUT"),
    ) else {
        panic!("run by tools/test_block_entity_item_geometry_parity.py");
    };
    with_client_game(|game| {
        let mut lines = String::new();
        for line in std::fs::read_to_string(cases).unwrap().lines() {
            let case: serde_json::Value = serde_json::from_str(line).unwrap();
            lines += &case_rows(game, &case).to_string();
            lines.push('\n');
        }
        std::fs::write(out, lines).unwrap();
    });
}
