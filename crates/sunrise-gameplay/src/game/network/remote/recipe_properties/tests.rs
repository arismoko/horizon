use std::sync::Arc;

use sunrise_gameplay_common::game::stations_common::{Kind, Station};

pub fn cut_pack(root: &std::path::Path) -> impl Fn(&str, &str) {
    let pack = root.join("datapacks/cuts");
    let write = move |rel: &str, json: &str| {
        let path = pack.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, json).unwrap();
    };
    write(
        "pack.mcmeta",
        r#"{"pack":{"description":"fixture","min_format":94,"max_format":94}}"#,
    );
    std::fs::write(
        root.join("datapacks.json"),
        r#"{"enabled":["vanilla","file/cuts"],"disabled":[]}"#,
    )
    .unwrap();
    write
}

pub fn place_stonecutter(server: &mut crate::game::ServerGame<'_>) -> Station {
    server
        .common
        .world
        .chunks
        .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
    let state = server
        .common
        .pack
        .assets
        .registry
        .default_state("minecraft:stonecutter")
        .unwrap();
    server.common.world.set(3, 64, 4, state).unwrap();
    Station::new(Kind::Stonecutter, [3, 64, 4])
}
