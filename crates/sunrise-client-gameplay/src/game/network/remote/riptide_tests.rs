use std::sync::mpsc::Receiver;
use sunrise_gameplay_common::game::remote::{C, Chunk, Vec3, play};
use sunrise_protocol::buf::Reader;
use sunrise_sim::lan::Out;

pub const OWNER: u32 = 12;
pub const START: Vec3 = Vec3::new(4.5, 64.0, 4.5);

/// The arena's one chunk carries plains in every quart `BiomeManager.getBiome` can pick for
/// the blocks `Entity.isInRain` samples at either start height: the feet blocks (4, 64, 4) and
/// (4, 65, 4) and the tops (4, 65, 4) and (4, 66, 4), whose corners are the quarts with x and
/// z in {0, 1} and y in 15..=17.
pub fn arena<'g, G: sunrise_gameplay_common::game::SideGame<'g>>(
    game: &mut G,
    roof: bool,
    insert: impl FnOnce(&mut G, Chunk),
) {
    let plains = game.res.biomes().id("minecraft:plains");
    let mut chunk = Chunk::empty(0, 0);
    let bottom = game.world.bottom_y() >> 2;
    for x in 0..=1 {
        for y in 15..=17 {
            for z in 0..=1 {
                chunk.set_biome_cell(x, y - bottom, z, plains);
            }
        }
    }
    insert(game, chunk);
    let registry = game.pack.assets.registry;
    let stone = registry.default_state("minecraft:stone").unwrap();
    for z in 4..=12 {
        game.world.set(4, 63, z, stone).unwrap();
    }
    if roof {
        // A low roof blocks rain; use water at the feet for this collision case.
        let water = registry.default_state("minecraft:water").unwrap();
        game.world.set(4, 64, 4, water).unwrap();
        game.world.set(4, 66, 4, stone).unwrap();
    }
    game.world.weather.raining = true;
    game.world.weather.settle_levels();
}

pub fn server_to_client(
    rx: &Receiver<Out>,
    client: &mut crate::game::ClientGame<'_>,
) -> Vec<play::PlayerPosition> {
    let mut teleports = Vec::new();
    for body in rx.try_iter().flat_map(|out| match out {
        Out::Packet(body) => vec![body],
        Out::Packets(bodies) => bodies,
        Out::Disconnect(_) => panic!("owner disconnected"),
    }) {
        let mut wire = Reader::new(&body);
        let id = wire.varint().unwrap();
        let packet = C::decode(id, wire.rest()).unwrap();
        if let C::PlayerPosition(ref position) = packet {
            teleports.push(position.clone());
        }
        client.handle_packet(packet).unwrap();
    }
    teleports
}
