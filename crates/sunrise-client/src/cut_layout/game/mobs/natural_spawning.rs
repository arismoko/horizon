#![cfg(test)]
use sunrise_core::geometry::Vec3;
use sunrise_core::random::LegacyRandom;
use sunrise_gameplay::game::server_side::natural_spawning::SpawnScratch;
use sunrise_gameplay_common::game::Dim;
use sunrise_sim::mobs::spawn_selection;
#[cfg(test)]
#[path = "natural_spawning/spawn_obstruction_fixture.rs"]
mod spawn_obstruction_fixture;

#[cfg(test)]
pub(crate) mod tests {

    use super::*;
    use sunrise_client_lib::mobs::spawn_rules::SpawnLevel;
    use sunrise_client_lib::mobs::spawn_selection::NaturalLevel;
    use sunrise_core::random::RandomSource;
    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_gameplay_common::world_biomes;
    use sunrise_sim::mobs::spawn_selection::{Category, Heightmap};

    /// `Level.hasNearbyAlivePlayer` as spawners ask it, against the same
    /// test on every player's full record (`level_players`).
    #[test]
    #[ignore = "daily"]
    fn alive_player_near_tests_guests_where_they_stand() {
        use std::sync::{Arc, atomic::AtomicI32, mpsc};
        use sunrise_protocol::packets::configuration::ClientInformation;
        use sunrise_sim::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
        let mut resources = None;
        let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
        let net = NetConfig {
            announce: false,
            ..NetConfig::lan(0, "spawner-players-test".into(), 8)
        };
        game.lan_host(
            sunrise_gameplay::game::server_side::lan::LanSettings {
                port: 0,
                guest_commands: true,
                force_game_mode: false,
            },
            net,
            HubConfig {
                default_game_mode: 0,
                guest_commands: true,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: Dim::Overworld.id().into(),
                allow_flight: false,
                seed: 0,
                max_players: 8,
                integrated: false,
                levels: Default::default(),
                level_seeds: Default::default(),
            },
        )
        .unwrap();
        // (x, dead, spectator): alive at 20, dead at 5, a spectator at 2.
        for (conn, (x, dead, spectator)) in
            [(20.5, false, false), (5.5, true, false), (2.5, false, true)]
                .into_iter()
                .enumerate()
        {
            let conn = conn as u32 + 2;
            let (tx, _rx) = mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn,
                    uuid: u128::from(conn),
                    name: format!("Near{conn}"),
                    properties: Vec::new(),
                    info: ClientInformation::new(2),
                    brand: "test".into(),
                    sender: PlayerSender::new(tx, Arc::default()),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:25565".parse().unwrap(),
                    expected: false,
                },
                conn as i32,
                [x, 64.0, 0.5],
                1,
            );
            guest.placed = true;
            guest.loaded = true;
            guest.dead = dead;
            if spectator {
                guest.game_mode = 3;
            }
            game.server.lan.as_mut().unwrap().hub.guests.push(guest);
        }
        let full = |game: &sunrise_gameplay::game::ServerGame<'_>, x: f64, range: f64| {
            game.level_players()
                .iter()
                .any(|p| p.alive_near(x, 64.0, 0.5, range))
        };
        for (x, range, near) in [
            (0.5, 16.0, false),   // only the dead and the spectator in range
            (5.5, 16.0, true),    // the live one 15 away
            (4.4, 16.0, false),   // 16.1 away
            (4.5, 16.0, false),   // 16 away: not below the range
            (-500.0, -1.0, true), // any distance
            (36.4, 16.0, true),
        ] {
            assert_eq!(
                game.alive_player_near(x, 64.0, 0.5, range),
                near,
                "{x} {range}"
            );
            assert_eq!(full(&game, x, range), near, "{x} {range}");
        }
    }
    /// An End chunk read back from a save holds the table's first biome (badlands) in the
    /// sections its widened column adds below Y 0 and above 255. Natural spawning reads the
    /// level's own range (`getMinY` 0, `getMaxY` 255) and biomes clamped to it
    /// (`getNoiseBiome`): next to a floor at Y 0 every monster list is the End's, and an
    /// empty column's surface is the floor (#15880).
    #[test]
    #[ignore = "daily"]
    fn end_spawning_reads_the_level_range_not_the_widened_column() {
        use std::sync::Arc;
        use sunrise_level::chunk::{Chunk, LevelShape};
        let mut resources = None;
        let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
        game.dim = Dim::End;
        game.world.shape = LevelShape::END;
        let end = world_biomes!(game).id("minecraft:the_end");
        // The column runs -64..320: quart rows 16..80 are the level's 0..256. The zoom
        // reaches into the neighbours.
        for (cx, cz) in (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, z))) {
            let mut chunk = Chunk::with_layout(cx, cz, LevelShape::END.column());
            for qy in 16..80 {
                for qz in 0..4 {
                    for qx in 0..4 {
                        chunk.set_biome_cell(qx, qy, qz, end);
                    }
                }
            }
            game.world.chunks.insert((cx, cz), Arc::new(chunk));
        }
        let level = game.natural_level();
        for x in 0..16 {
            for z in 0..16 {
                for y in [1, 2, 3, 254, 255] {
                    let list = spawn_selection::mobs_at(&level, Category::Monster, x, y, z);
                    let types: Vec<&str> = list.iter().map(|e| e.entity.as_str()).collect();
                    assert_eq!(types, ["minecraft:enderman"], "at {x} {y} {z}");
                }
            }
        }
        assert_eq!((level.min_y(), level.max_y()), (0, 255));
        assert_eq!(level.heightmap(Heightmap::WorldSurface, 3, 3), 0);
        assert_eq!(level.heightmap(Heightmap::MotionBlocking, 3, 3), 0);
    }
    #[test]
    #[ignore = "daily"]
    fn authoritative_world_border_gates_positions_and_whole_chunks() {
        use sunrise_core::position::BlockPos;
        use sunrise_world::level::LevelAccess;
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            game.border.set_center([8.0, 8.0]).unwrap();
            game.border.set_size(16.0).unwrap();
            assert!(game.natural_level().can_spawn_in_chunk(0, 0));
            assert!(!game.natural_level().can_spawn_in_chunk(1, 0)); // unloaded
            game.border.set_size(15.5).unwrap();
            assert!(!game.natural_level().can_spawn_in_chunk(0, 0)); // partial
            assert!(game.natural_level().within_world_border(1, 1));
            assert!(!game.natural_level().within_world_border(0, 0));
            game.with_level(|_, level, _| {
                assert!(level.within_world_border(BlockPos::new(1, 64, 1)));
                assert!(!level.within_world_border(BlockPos::new(0, 64, 0)));
            });
            game.border.set_size(16.5).unwrap();
            assert!(game.natural_level().can_spawn_in_chunk(0, 0));
            assert!(game.mob_world_base(&[]).within_world_border(0, 0));
            assert!(!game.mob_world_base(&[]).within_world_border(17, 0));
        });
    }
    #[test]
    #[ignore = "daily"]
    fn world_border_outer_chunk_gate_keeps_attempt_randomness() {
        use std::sync::Arc;
        use sunrise_level::chunk::Chunk;
        use sunrise_sim::motion::PlayerMotion;
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            // Only the center chunk can tick: its eight neighbors are loaded,
            // but none of those has all its own neighbors. The center is partial.
            for x in -1..=1 {
                for z in -1..=1 {
                    let mut chunk = Chunk::empty(x, z);
                    chunk.lit = x == 0 && z == 0;
                    game.world.chunks.insert((x, z), Arc::new(chunk));
                }
            }
            game.server.tickets.0.update(None);
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 64.0, 8.5)).unwrap());
            game.border.set_center([8.0, 8.0]).unwrap();
            game.border.set_size(2.0).unwrap();
            let mut scratch = SpawnScratch::default();
            game.spawning_chunks(&game.level_players(), &mut scratch);
            assert_eq!(scratch.candidates, vec![(0, 0)]);
            assert!(
                game.ui
                    .rules
                    .bool_at(sunrise_game_data::gamerules::vanilla::SPAWN_MOBS)
            );
            // A contained chunk must actually reach the spawning pass, so an
            // empty category list cannot satisfy the rejected-chunk assertion.
            game.border.set_size(64.0).unwrap();
            game.world.random.set_seed(12345);
            game.spawn_natural();
            assert_ne!(
                game.world.random.next_long(),
                LegacyRandom::new(12345).next_long()
            );
            game.world.inhabited.clear();
            game.border.set_size(2.0).unwrap();
            game.world.random.set_seed(12345);
            game.spawn_natural();
            // No shuffle draw for one candidate, and no attempt draws for a
            // rejected chunk. Inhabited time still advances before the gate.
            assert_eq!(game.world.inhabited.get(&(0, 0)), Some(&1));
            assert_eq!(
                game.world.random.next_long(),
                LegacyRandom::new(12345).next_long()
            );
        });
    }
    /// `ServerLevel.tick` ticks the raids before the chunk source spawns
    /// (`raids.tick`, then `getChunkSource().tick`): a wave that lands in a
    /// tick joins before that tick's natural and custom spawns, with the
    /// lower entity ids. The phases name themselves in the spans they run in.
    #[test]
    #[ignore = "daily"]
    fn raids_tick_before_natural_and_custom_spawning() {
        let mut resources = None;
        let mut game = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
        sunrise_level::spans::record_here(true);
        let _ = sunrise_level::spans::take_raw();
        game.tick_spawners();
        let raw = sunrise_level::spans::take_raw();
        sunrise_level::spans::record_here(false);
        let phases: Vec<&str> = raw
            .iter()
            .map(|s| s.name)
            .filter(|name| name.starts_with("spawn_"))
            .collect();
        assert_eq!(
            phases,
            ["spawn_raids", "spawn_natural", "spawn_custom"],
            "ServerLevel.tick ticks the raids before the chunk source's spawns"
        );
    }
}
