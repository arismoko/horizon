//! Viewed levels, the client's worlds (#3716, 9c): a world for each level a server opens on
//! `horizon:levels`, as Immersive Portals' `ClientWorldLoader` keeps a `ClientLevel` per
//! dimension. The server wraps each packet of such a level with it (`PacketRedirection`); here
//! a `run`'s packets change that level's world and nothing of the player's own: its `World`,
//! the level mods reach through `host.level`, its renderer and its sounds stay as they are.
//!
//! The client views at most `viewed_levels_net::MAX_LEVELS` levels besides its own, the number
//! its capability answer gives. A Login, a Respawn or a new configuration phase drops every
//! viewed world, and the server opens again what it still views. Nothing of a viewed level is
//! drawn yet: drawing is gap 13's world view, and its entities are 9d.
mod routing;
mod worlds;
pub(in crate::game) use worlds::ViewedWorlds;

#[cfg(test)]
mod tests {
    use super::routing::RouteError;
    use crate::game::{Game, game_tests::tests::server_game};
    use horizon_protocol::{
        buf::Reader,
        chunk::{ChunkData, LightData, Paletted, Section, Strategy},
        packets::play::{self, Clientbound as C},
        viewed_levels,
    };
    use horizon_world::registry::BlockStateId;

    const STONE: BlockStateId = BlockStateId(1);
    const AIR: BlockStateId = BlockStateId(0);

    fn spawn(dimension: &str) -> play::SpawnInfo {
        play::SpawnInfo {
            dimension_type: 0,
            dimension: dimension.into(),
            seed: 1,
            game_type: 0,
            previous_game_type: None,
            debug: false,
            flat: false,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        }
    }

    /// A client of a server whose Login names four levels, the player in the Overworld.
    fn client(resources: &mut Option<std::sync::Arc<crate::game::Resources>>) -> Game<'_> {
        let mut game = server_game(resources);
        game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
        let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
        game.connect_local(pipe, "Steve").unwrap();
        game.handle_packet(C::Login(play::Login {
            player_id: 7,
            hardcore: false,
            levels: [
                "minecraft:overworld",
                "minecraft:the_nether",
                "minecraft:the_end",
                "test:void",
            ]
            .map(String::from)
            .to_vec(),
            max_players: 20,
            chunk_radius: 8,
            simulation_distance: 8,
            reduced_debug_info: false,
            show_death_screen: true,
            limited_crafting: false,
            spawn: spawn("minecraft:overworld"),
            online_mode: false,
            enforces_secure_chat: false,
        }))
        .unwrap();
        game
    }

    /// A column of stone at `x, z`, unlit.
    fn column(x: i32, z: i32) -> C {
        let section = Section {
            non_empty: 4096,
            fluids: 0,
            states: Paletted::from_values(&[STONE.0 as i32; 4096], Strategy::Blocks, 16),
            biomes: Paletted::from_values(&[1; 64], Strategy::Biomes, 2),
        };
        C::LevelChunkWithLight(play::LevelChunkWithLight {
            x,
            z,
            chunk: ChunkData {
                heightmaps: Vec::new(),
                buffer: ChunkData::from_sections(&vec![section; 24]),
                block_entities: Vec::new(),
            },
            light: LightData {
                sky_mask: Vec::new(),
                block_mask: Vec::new(),
                empty_sky_mask: Vec::new(),
                empty_block_mask: Vec::new(),
                sky: Vec::new(),
                block: Vec::new(),
            },
        })
    }

    /// The custom payload the client's net thread hands the game for a whole packet body.
    fn payload(body: &[u8]) -> C {
        let mut r = Reader::new(body);
        let id = r.varint().unwrap();
        C::decode(id, r.rest()).unwrap()
    }

    fn open(level: i32, dimension: &str) -> C {
        payload(&viewed_levels::open(level, &spawn(dimension)))
    }

    fn run(level: i32, packets: &[C]) -> C {
        let mut run = viewed_levels::Run::new(level, &packets[0].encode()).unwrap();
        for packet in &packets[1..] {
            run.push(&packet.encode()).unwrap();
        }
        payload(&run.finish())
    }

    fn data(packet: &C) -> &[u8] {
        let C::CustomPayload(p) = packet else {
            panic!("not a payload")
        };
        &p.data
    }

    fn viewed<'a>(game: &'a Game, level: i32) -> Option<&'a horizon_client::world::World> {
        Some(&game.remote.as_ref()?.viewed.get(level)?.world)
    }

    /// A wrapped `BlockUpdate` changes the viewed level's block, and the player's level keeps
    /// its own at the same position.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_wrapped_block_update_changes_the_viewed_world_not_the_players() {
        let mut resources = None;
        let mut game = client(&mut resources);
        game.handle_packet(column(0, 0)).unwrap();
        game.handle_packet(open(1, "minecraft:the_nether")).unwrap();
        game.handle_packet(run(1, &[column(0, 0)])).unwrap();
        let block = C::BlockUpdate(play::BlockUpdate {
            pos: [3, 70, 5],
            state: AIR.0 as i32,
        });
        game.handle_packet(run(1, &[block])).unwrap();
        assert_eq!(viewed(&game, 1).unwrap().block(3, 70, 5), AIR);
        assert_eq!(viewed(&game, 1).unwrap().block(3, 71, 5), STONE);
        assert_eq!(game.world.block(3, 70, 5), STONE);
    }

    /// An `open` makes a world for the level, a `run` fills it, and a `close` drops it: a `run`
    /// after it is of a level not open.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn open_makes_a_world_and_close_drops_it() {
        let mut resources = None;
        let mut game = client(&mut resources);
        game.handle_packet(open(2, "minecraft:the_end")).unwrap();
        let world = game.remote.as_ref().unwrap().viewed.get(2).unwrap();
        assert_eq!(world.level, "minecraft:the_end");
        assert_eq!(world.world.chunks.len(), 0);
        game.handle_packet(run(2, &[column(4, -2), column(5, -2)]))
            .unwrap();
        assert_eq!(viewed(&game, 2).unwrap().chunks.len(), 2);
        let forget = C::ForgetLevelChunk(play::ForgetLevelChunk { pos: [4, -2] });
        game.handle_packet(run(2, &[forget])).unwrap();
        assert_eq!(viewed(&game, 2).unwrap().chunks.len(), 1);
        game.handle_packet(payload(&viewed_levels::close(2)))
            .unwrap();
        assert!(viewed(&game, 2).is_none());
        assert_eq!(
            game.route_viewed_levels(data(&run(2, &[column(4, -2)]))),
            Err(RouteError::NotOpen(2))
        );
    }

    /// The client views two levels besides its own: a third `open` closes the connection.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn an_open_past_the_clients_cap_closes_the_connection() {
        let mut resources = None;
        let mut game = client(&mut resources);
        game.handle_packet(open(1, "minecraft:the_nether")).unwrap();
        game.handle_packet(open(2, "minecraft:the_end")).unwrap();
        assert_eq!(
            game.route_viewed_levels(data(&open(3, "test:void"))),
            Err(RouteError::OverCap { level: 3, cap: 2 })
        );
        // An open again of a level open already is no third level: it starts afresh.
        game.handle_packet(open(2, "minecraft:the_end")).unwrap();
        assert!(game.remote.is_some());
        game.handle_packet(open(3, "test:void")).unwrap();
        assert!(game.remote.is_none());
    }

    /// An `open` naming another level than the Login put at its index closes the connection
    /// with a fixed reason: the server's text is not shown.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn an_open_not_at_its_login_index_shows_no_server_text() {
        let mut resources = None;
        let mut game = client(&mut resources);
        let refused = game
            .route_viewed_levels(data(&open(1, "evil:click_here")))
            .unwrap_err();
        assert_eq!(refused, RouteError::UnknownLevel);
        assert_eq!(
            refused.to_string(),
            "horizon:levels: open does not match the Login level index"
        );
        assert!(!refused.to_string().contains("evil"));
        assert!(viewed(&game, 1).is_none());
    }

    /// A viewed level's chunks, time and weather leave the player's level as it was.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn the_players_world_stays_as_it_was() {
        let mut resources = None;
        let mut game = client(&mut resources);
        game.handle_packet(column(0, 0)).unwrap();
        let (game_time, day_time) = (game.world.game_time, game.world.day_time);
        game.handle_packet(open(1, "minecraft:the_nether")).unwrap();
        let time = C::SetTime(play::SetTime {
            game_time: 24_000,
            clocks: Vec::new(),
        });
        let rain = C::GameEvent(play::GameEvent {
            event: 1,
            param: 0.0,
        });
        game.handle_packet(run(1, &[column(1, 0), time, rain]))
            .unwrap();
        let nether = viewed(&game, 1).unwrap();
        assert_eq!(nether.game_time, 24_000);
        assert!(nether.weather.raining);
        assert!(nether.chunks.contains_key(&(1, 0)));
        assert_eq!(game.world.chunks.len(), 1);
        assert!(!game.world.chunks.contains_key(&(1, 0)));
        assert_eq!(game.world.game_time, game_time);
        assert_eq!(game.world.day_time, day_time);
        assert!(!game.world.weather.raining);
        assert_eq!(game.remote.as_ref().unwrap().level, "minecraft:overworld");
    }

    /// A death's Respawn drops the viewed worlds: the server opens again what it still views.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_death_respawn_drops_the_viewed_worlds() {
        let mut resources = None;
        let mut game = client(&mut resources);
        game.handle_packet(open(1, "minecraft:the_nether")).unwrap();
        game.handle_packet(run(1, &[column(0, 0)])).unwrap();
        game.handle_packet(C::Respawn(play::Respawn {
            spawn: spawn("minecraft:overworld"),
            keep: 0,
        }))
        .unwrap();
        assert_eq!(game.remote.as_ref().unwrap().viewed.len(), 0);
    }
}
