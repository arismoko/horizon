#![cfg(test)]
#[cfg(test)]
mod tests {
    #[cfg(feature = "client")]
    use super::super::native_entity_refs::tests::{run_fresh_cases, with_client_game, with_game};
    #[cfg(feature = "client")]
    use core::ops::ControlFlow;
    #[cfg(feature = "client")]
    use mod_api::{ChunkRef, LightLayer, WorldEpoch};
    #[cfg(feature = "client")]
    use mod_api::{LevelReadError, LevelRef};
    #[cfg(feature = "client")]
    use sunrise_core::position::BlockPos;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::Dim;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::GameGameExt1;
    use sunrise_gameplay_common::game::api_values::TryIntoApi;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_server::NativeServerGameExt1;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::transactions::TransactionsGameExt;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::weather::WeatherGameExt;

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn review_climate_literals() {
        with_game(|game| {
            let level = game.level("minecraft:overworld").unwrap();
            for (name, temperature, downfall, precipitation, frozen) in [
                ("plains", 0.8, 0.4, true, false),
                ("desert", 2.0, 0.0, false, false),
                ("snowy_plains", 0.0, 0.5, true, false),
                ("ocean", 0.5, 0.5, true, false),
                ("frozen_ocean", 0.0, 0.5, true, true),
            ] {
                let actual = game
                    .level_biome_properties(level, &format!("minecraft:{name}"))
                    .unwrap();
                assert_eq!(
                    actual,
                    mod_api::BiomeProperties {
                        temperature,
                        downfall,
                        has_precipitation: precipitation,
                        frozen
                    },
                    "{name}"
                );
            }
            for id in ["minecraft:no_such_biome", "", "!malformed::id"] {
                assert_eq!(game.level_biome_properties(level, id), None);
            }
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            assert_eq!(game.level_biome_properties(stale, "minecraft:plains"), None);
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_level_with_its_own_clock_counts_apart_from_the_overworlds() {
        with_game(|game| {
            game.world.day_time = 100;
            game.enter_level(Dim::End).unwrap();
            assert_eq!(game.active_clock(), "minecraft:the_end");
            assert_eq!(game.world.day_time, 0, "the End's clock has its own total");
            game.world.day_time = 50;
            assert_eq!(game.command_clock_time("minecraft:overworld"), 100);
            game.enter_level(Dim::Nether).unwrap();
            assert_eq!(
                game.world.day_time, 100,
                "a level without a clock follows the Overworld's"
            );
            assert_eq!(game.command_clock_time("minecraft:the_end"), 50);
            game.enter_level(Dim::Overworld).unwrap();
            assert_eq!(game.world.day_time, 100);
        });
    }

    /// A logical server with its worker queue running rather than the
    /// title-screen streamer's paused placeholder.
    #[cfg(feature = "client")]
    fn with_chunk_streaming(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
        with_game(|game| {
            game.options.headless = true;
            game.server.streaming.paused = false;
            game.server.streaming.radius = 0;
            test(game);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn block_transform_preview_preserves_state_and_marks_simulation() {
        use super::super::native_guest_turn_tests::with_guests;
        use mod_api::{BlockHit, BlockToolModificationEvent, Flow, Hand, Hooks, Host};
        use std::sync::{Arc, Mutex};
        use sunrise_items::items::ItemStack;
        use sunrise_level::chunk::Chunk;

        struct Observer(Arc<Mutex<Vec<bool>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Observer {
            fn block_tool_modification(
                &self,
                _: &dyn Host,
                event: &mut BlockToolModificationEvent<'_>,
            ) -> Flow {
                self.0.lock().unwrap().push(event.tool.simulated);
                Flow::Continue
            }
        }
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "preview_test",
            Box::new(Observer(heard.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_guests(|game| {
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let pos = [0, 64, 0];
                let log = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:oak_log")
                    .unwrap();
                game.world.set(pos[0], pos[1], pos[2], log).unwrap();
                let tool = ItemStack::new("minecraft:iron_axe", 1);
                {
                    let value = Some(tool.clone());
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |g| g.inventory[0] = value);
                }
                let player = game.player(12).unwrap();
                let level = game.level(game.dim.id()).unwrap();
                let hit = BlockHit::new([0.5, 65.0, 0.5], [0, 1, 0], false);
                let preview = level
                    .block_transform(
                        game,
                        player,
                        Hand::Main,
                        &tool.try_into_api().unwrap(),
                        pos,
                        hit,
                    )
                    .unwrap();
                let stripped = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stripped_oak_log")
                    .unwrap();
                assert_eq!(preview.state, stripped.try_into_api().unwrap());
                assert_eq!(preview.durability, 1);
                assert_eq!(game.world.block(pos[0], pos[1], pos[2]), log);
                assert_eq!(
                    game.server.lan.as_ref().unwrap().hub.guests[0].inventory[0],
                    Some(tool.clone())
                );
                assert!(
                    level
                        .block_transform(
                            game,
                            player,
                            Hand::Main,
                            &tool.try_into_api().unwrap(),
                            [16, 64, 0],
                            hit
                        )
                        .is_none()
                );
                game.as_guest(12, |game| {
                    let hit = sunrise_gameplay_common::game::Hit {
                        pos,
                        face: hit.face,
                        point: hit.point,
                        inside: hit.inside,
                    };
                    assert!(game.use_block_transformer(&tool, &hit).unwrap());
                })
                .unwrap();
                assert_eq!(game.world.block(pos[0], pos[1], pos[2]), stripped);
                assert_eq!(
                    game.server.lan.as_ref().unwrap().hub.guests[0].inventory[0]
                        .as_ref()
                        .unwrap()
                        .damage,
                    1
                );
            })
        });
        assert_eq!(*heard.lock().unwrap(), vec![true, false]);
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn shared_level_signal_dispatches_and_clamps_part_output() {
        use mod_api::Host;
        use std::sync::Arc;
        use sunrise_core::geometry::Direction;
        use sunrise_level::chunk::Chunk;

        mod_platform::__private::part_tests::signal_game_thread();
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let y = game.world.bottom_y() + 1;
            let glass = game
                .pack
                .assets
                .registry
                .default_state("minecraft:glass")
                .unwrap();
            game.world.set(1, y, 1, glass).unwrap();
            let level = game.level(game.dim.id()).unwrap();
            let host: &dyn Host = game;
            assert_eq!(level.signal(host, [1, y, 1], Direction::East), Some(15));
            assert_eq!(
                level.direct_signal(host, [1, y, 1], Direction::East),
                Some(0)
            );
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn shared_level_signal_reads_live_comparator_and_refuses_other_levels() {
        use mod_api::Host;
        use std::sync::Arc;
        use sunrise_core::geometry::Direction;
        use sunrise_level::chunk::Chunk;

        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let y = game.world.bottom_y() + 1;
            let registry = &game.common.pack.assets.registry;
            let redstone = registry.default_state("minecraft:redstone_block").unwrap();
            game.common.world.set(1, y, 1, redstone).unwrap();
            let comparator = registry.default_state("minecraft:comparator").unwrap();
            let mut comparator_state = registry.state(comparator).unwrap().clone();
            comparator_state
                .properties
                .insert("powered".into(), "true".into());
            comparator_state
                .properties
                .insert("facing".into(), "east".into());
            let comparator = registry.resolve(&comparator_state).unwrap();
            game.world.set(4, y, 4, comparator).unwrap();
            game.block_ticks
                .comparator_outputs
                .insert(BlockPos::new(4, y, 4), 9);
            let level = game.level(game.dim.id()).unwrap();
            let host: &dyn Host = game;
            assert_eq!(level.signal(host, [1, y, 1], Direction::East), Some(15));
            assert_eq!(level.best_neighbor_signal(host, [2, y, 1]), Some(15));
            assert!(level.has_neighbor_signal(host, [2, y, 1]));
            assert_eq!(level.signal(host, [4, y, 4], Direction::East), Some(9));
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            let other = LevelRef::new(level.epoch(), mod_api::DimensionKey::NETHER);
            for unavailable in [stale, other] {
                assert_eq!(unavailable.signal(host, [1, y, 1], Direction::East), None);
                assert!(!unavailable.has_neighbor_signal(host, [2, y, 1]));
            }
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn respawn_data_maps_and_ticks_write_through_the_handle_and_refuse_others() {
        use std::sync::Arc;
        use sunrise_level::chunk::Chunk;

        with_game(|game| {
            let level = game.level("minecraft:overworld").unwrap();
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            let nether = LevelRef::new(level.epoch(), mod_api::DimensionKey::NETHER);

            // RespawnData.of: the yaw wraps to -180..180 and the pitch clamps to -90..90.
            assert!(level.set_respawn_data(game, [10, 70, -4], 270.0, 120.0));
            let data = level.respawn_data(game).unwrap();
            assert_eq!(data.position.dimension().to_string(), "minecraft:overworld");
            assert_eq!(data.position.pos(), BlockPos::new(10, 70, -4));
            assert_eq!((data.yaw, data.pitch), (-90.0, 90.0));
            assert_eq!(
                game.level.world_spawn,
                Some([10, 70, -4]),
                "level.dat's spawn"
            );
            assert!(!level.set_respawn_data(game, [0, 64, 0], f32::NAN, 0.0));
            assert!(!level.set_respawn_data(game, [30_000_001, 64, 0], 0.0, 0.0));
            assert!(!stale.set_respawn_data(game, [0, 64, 0], 0.0, 0.0));
            assert_eq!(stale.respawn_data(game), None);
            assert_eq!(
                level.respawn_data(game).unwrap().position.pos(),
                BlockPos::new(10, 70, -4),
                "refusals change nothing"
            );
            assert!(
                !nether.set_respawn_data(game, [3, 40, 9], 0.0, 0.0),
                "a level not open yet"
            );
            // SetWorldSpawnCommand in another level: the world spawn moves there, and every
            // level's handle reads it with that level.
            game.enter_level(sunrise_gameplay_common::game::Dim::Nether)
                .unwrap();
            game.enter_level(sunrise_gameplay_common::game::Dim::Overworld)
                .unwrap();
            assert!(nether.set_respawn_data(game, [3, 40, 9], 0.0, 0.0));
            for handle in [level, nether] {
                let data = handle.respawn_data(game).unwrap();
                assert_eq!(
                    data.position.dimension().to_string(),
                    "minecraft:the_nether"
                );
                assert_eq!(data.position.pos(), BlockPos::new(3, 40, 9));
            }
            assert_eq!(
                game.level.world_spawn_dimension.as_deref(),
                Some("minecraft:the_nether"),
                "level.dat's spawn.dimension"
            );

            // A map repainted through the handle reads back and is marked to save and redraw.
            let fresh = sunrise_gameplay_common::game::maps::MapData::fresh(
                0.0,
                0.0,
                0,
                "minecraft:overworld",
            );
            let id = game.new_map_id(fresh);
            let mut colors = Vec::new();
            assert_eq!(
                level.visit_map_data(game, id, &mut |map| colors = map.colors.to_vec()),
                Ok(Some(()))
            );
            assert_eq!(colors.len(), 128 * 128);
            colors[5 + 2 * 128] = 34;
            assert!(level.set_map_data(game, id, &colors));
            assert!(
                !level.set_map_data(game, id, &colors),
                "nothing left to change"
            );
            assert!(!level.set_map_data(game, id, &colors[..10]));
            assert_eq!(
                nether.visit_map_data(game, id, &mut |map| {
                    assert_eq!(map.colors[5 + 2 * 128], 34);
                    assert_eq!((map.center, map.scale), ([0, 0], 0));
                    assert_eq!(map.dimension, "minecraft:overworld");
                }),
                Ok(Some(())),
                "a parked level reads the same world map"
            );
            let retired = LevelRef::with_generation(level.epoch(), level.dimension(), 1);
            let mut unexpected_map: fn(mod_api::MapState<'_>) = |_| panic!("visited a refused map");
            for invalid in [stale, retired] {
                assert_eq!(
                    invalid.visit_map_data(game, id, &mut unexpected_map),
                    Err(mod_api::LevelReadError::Stale)
                );
            }
            let server_role = game.options.execution_role;
            game.options.execution_role =
                sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
            assert_eq!(
                level.visit_map_data(game, id, &mut unexpected_map),
                Err(mod_api::LevelReadError::Unavailable)
            );
            game.options.execution_role = server_role;
            assert!(!stale.set_map_data(game, id, &colors));
            assert_eq!(
                level.visit_map_data(game, i32::MAX, &mut |_| panic!("visited a missing map")),
                Ok(None),
                "a map nothing made"
            );

            // Ticks run only in the level in place, at a loaded position inside its height.
            let height = level.height(game).unwrap();
            assert!(!level.block_tick(game, [0, height.0, 0]));
            assert!(!level.block_random_tick(game, [0, height.0, 0]));
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            assert!(level.block_tick(game, [0, height.0, 0]));
            assert!(level.block_random_tick(game, [0, height.0, 0]));
            assert!(
                !level.block_tick(game, [0, height.0 - 1, 0]),
                "below the world"
            );
            assert!(
                !level.block_random_tick(game, [1_000_000, 64, 1_000_000]),
                "unloaded"
            );
            assert!(!stale.block_tick(game, [0, height.0, 0]));
            assert!(
                !nether.block_random_tick(game, [0, height.0, 0]),
                "a level not in place"
            );
            assert_eq!(stale.can_see_sky(game, [0, height.0, 0]), None);
            assert_eq!(stale.line_of_sight(game, [0.5; 3], [4.5, 0.5, 0.5]), None);
            assert_eq!(stale.lightning_target(game, [0, 64, 0]), None);
            assert_eq!(nether.lightning_target(game, [0, 64, 0]), None);
        });
    }

    #[cfg(feature = "client")]
    fn collect_chunks<T>(
        enumerate: impl FnOnce(
            &mut dyn FnMut(T) -> ControlFlow<()>,
        ) -> Result<ControlFlow<()>, LevelReadError>,
    ) -> Result<Vec<T>, LevelReadError> {
        let mut entries = Vec::new();
        assert_eq!(
            enumerate(&mut |entry| {
                entries.push(entry);
                ControlFlow::Continue(())
            })?,
            ControlFlow::Continue(())
        );
        Ok(entries)
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn loaded_chunks_lists_the_level_in_order_without_loading() {
        use std::sync::Arc;
        use sunrise_level::chunk::Chunk;

        with_game(|game| {
            let level = game.level("minecraft:overworld").unwrap();
            let before = collect_chunks(|visit| level.visit_loaded_chunks(game, visit))
                .unwrap()
                .len();
            for pos in [(2, -1), (0, 5), (0, -3)] {
                game.world
                    .chunks
                    .insert(pos, Arc::new(Chunk::empty(pos.0, pos.1)));
            }
            let chunks = collect_chunks(|visit| level.visit_loaded_chunks(game, visit)).unwrap();
            assert_eq!(chunks.len(), before + 3);
            let positions: Vec<[i32; 2]> = chunks.iter().map(|chunk| chunk.pos()).collect();
            assert!(
                positions.windows(2).all(|pair| pair[0] <= pair[1]),
                "ordered"
            );
            assert!(positions.contains(&[0, -3]) && positions.contains(&[2, -1]));
            let mut first = Vec::new();
            assert_eq!(
                level.visit_loaded_chunks(game, &mut |chunk| {
                    first.push(chunk.pos());
                    ControlFlow::Break(())
                }),
                Ok(ControlFlow::Break(()))
            );
            assert_eq!(first, positions[..1]);
            assert_eq!(game.world.chunks.len(), before + 3);
            let nether = LevelRef::new(level.epoch(), mod_api::DimensionKey::NETHER);
            assert_eq!(
                nether.visit_loaded_chunks(game, &mut |_| panic!("unopened level was visited")),
                Err(LevelReadError::Stale)
            );
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn bounds_reads_follow_the_level_without_loading_chunks() {
        with_game(|game| {
            let level = game.level("minecraft:overworld").unwrap();
            let nether = LevelRef::new(level.epoch(), mod_api::DimensionKey::NETHER);
            assert_eq!(level.in_world_bounds(game, [0, 64, 0]), Some(true));
            assert_eq!(level.in_world_bounds(game, [0, -65, 0]), Some(false));
            assert_eq!(level.in_world_bounds(game, [0, 320, 0]), Some(false));
            assert_eq!(
                level.in_world_bounds(game, [30_000_000, 64, 0]),
                Some(false)
            );
            assert_eq!(level.in_world_bounds(game, [0, i32::MAX, 0]), Some(false));
            assert_eq!(level.in_spawnable_bounds(game, [0, 320, 0]), Some(true));
            assert_eq!(
                level.in_spawnable_bounds(game, [29_999_999, 19_999_999, -30_000_000]),
                Some(true)
            );
            assert_eq!(
                level.in_spawnable_bounds(game, [0, 20_000_000, 0]),
                Some(false)
            );
            assert_eq!(
                nether.in_world_bounds(game, [0, 64, 0]),
                None,
                "not opened yet"
            );
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn world_border_reads_refuse_stale_and_unopened_levels() {
        with_game(|game| {
            let level = game.level("minecraft:overworld").unwrap();
            game.border.set_center([12.0, -8.0]).unwrap();
            game.border.set_size(40.0).unwrap();
            assert_eq!(level.world_border(game), Some([-8.0, -28.0, 32.0, 12.0]));
            let stale_epoch = WorldEpoch::new(level.epoch().get() + 1);
            let stale = LevelRef::new(stale_epoch, level.dimension());
            assert_eq!(stale.world_border(game), None);
            let unopened = LevelRef::new(level.epoch(), mod_api::DimensionKey::NETHER);
            assert_eq!(unopened.world_border(game), None);
            game.border.set_center([0.0, 0.0]).unwrap();
            assert_eq!(level.world_border(game), Some([-20.0, -20.0, 20.0, 20.0]));
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn height_reads_server_levels_and_the_joined_client_level() {
        use mod_api::Host;
        use sunrise_level::chunk::LevelShape;
        use sunrise_protocol::connection::Pipe;

        with_game(|game| {
            let overworld = game.level("minecraft:overworld").unwrap();
            let nether = LevelRef::new(overworld.epoch(), mod_api::DimensionKey::NETHER);
            assert_eq!(overworld.height(game), Some((-64, 384)));
            assert_eq!(nether.height(game), None, "not opened yet");
            game.enter_level(Dim::Nether).unwrap();
            assert_eq!(nether.height(game), Some((0, 256)));
            assert_eq!(overworld.height(game), Some((-64, 384)), "parked level");
            game.enter_level(Dim::Overworld).unwrap();
        });
        with_client_game(|game| {
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "HeightReadTest").unwrap();
            let epoch = game.native_lifecycle.epoch().unwrap();
            let overworld = LevelRef::new(epoch, mod_api::DimensionKey::OVERWORLD);
            let nether = LevelRef::new(epoch, mod_api::DimensionKey::NETHER);
            assert!(game.level("minecraft:overworld").is_none());
            assert_eq!(overworld.height(game), None, "not logged in");
            game.client.remote.as_mut().unwrap().logged_in = true;
            game.world.shape = LevelShape::new(-128, 512, 512, true, false).unwrap();
            let level = game.level("minecraft:overworld").unwrap();
            let host: &dyn Host = game;
            assert_eq!(level.height(host), Some((-128, 512)), "client's own shape");
            assert_eq!(
                nether.height(host),
                None,
                "parked server level is not visible"
            );
            assert!(host.level("minecraft:the_nether").is_none());
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            assert_eq!(stale.height(host), None);
            game.client.remote.as_mut().unwrap().logged_in = false;
            assert_eq!(level.height(game), None, "logged out");
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn block_state_reads_stored_ids_and_refuses_unloaded_positions() {
        use std::sync::Arc;
        use sunrise_level::chunk::Chunk;

        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level(game.dim.id()).unwrap();
            let bottom = game.world.bottom_y();
            let top = game.world.top_y();
            let registry = &game.common.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let stairs = registry.default_state("minecraft:oak_stairs").unwrap();
            let air = registry.default_state("minecraft:air").unwrap();
            for (pos, expected) in [([0, bottom, 0], stone), ([15, top - 1, 15], stairs)] {
                game.world.set(pos[0], pos[1], pos[2], expected).unwrap();
                assert_eq!(
                    level.block_state(game, pos),
                    Some(expected.try_into_api().unwrap())
                );
            }
            assert_eq!(
                level.block_state(game, [1, bottom, 1]),
                Some(air.try_into_api().unwrap())
            );
            assert_eq!(level.block_state(game, [16, bottom, 0]), None);
            assert_eq!(level.block_state(game, [0, bottom - 1, 0]), None);
            assert_eq!(level.block_state(game, [0, top, 0]), None);
        });
    }

    /// `FlowingFluid.getFlow`: water one level below a source beside it flows away from the
    /// source, normalized; so does the source, toward it; a source with only air on stone
    /// around it, and air, have none.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn fluid_flow_reads_the_current_that_pushes_entities() {
        use mod_api::Vec3;
        use std::sync::Arc;
        use sunrise_level::chunk::Chunk;

        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level(game.dim.id()).unwrap();
            let registry = &game.common.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let water = registry.default_state("minecraft:water").unwrap();
            let mut state = registry.state(water).unwrap().clone();
            state.properties.insert("level".into(), "1".into());
            let flowing = registry.resolve(&state).unwrap();
            let y = game.world.bottom_y() + 1;
            for x in 0..16 {
                for z in 0..16 {
                    game.world.set(x, y - 1, z, stone).unwrap();
                }
            }
            game.world.set(1, y, 1, water).unwrap();
            game.world.set(2, y, 1, flowing).unwrap();
            game.world.set(8, y, 8, water).unwrap();
            let east = Some(Vec3::new(1.0, 0.0, 0.0));
            assert_eq!(level.fluid_flow(game, [2, y, 1]), east);
            assert_eq!(level.fluid_flow(game, [1, y, 1]), east);
            assert_eq!(level.fluid_flow(game, [8, y, 8]), Some(Vec3::ZERO));
            assert_eq!(level.fluid_flow(game, [5, y, 5]), Some(Vec3::ZERO));
            assert_eq!(level.fluid_flow(game, [16, y, 0]), None);
        });
    }

    /// A stone floor under chunk (0, 0) at `y - 1` and the standing level `y` returned, with
    /// `fluids` placed `dy` blocks above it: each a block id and its `level` property (0 is a
    /// source).
    #[cfg(feature = "client")]
    fn pool(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        fluids: &[([i32; 3], &str, &str)],
    ) -> i32 {
        use std::sync::Arc;
        use sunrise_level::chunk::Chunk;

        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let y = game.world.bottom_y() + 1;
        for x in 0..16 {
            for z in 0..16 {
                game.world.set(x, y - 1, z, stone).unwrap();
            }
        }
        for &([x, dy, z], name, level) in fluids {
            let mut state = registry
                .state(registry.default_state(name).unwrap())
                .unwrap()
                .clone();
            state.properties.insert("level".into(), level.into());
            game.world
                .set(x, y + dy, z, registry.resolve(&state).unwrap())
                .unwrap();
        }
        y
    }

    /// Vanilla's fluid facts at a position, each against a value worked out by hand: water's 5
    /// and lava's 30 ticks (10 where lava flows fast), an empty fluid's 0, a source 8/9 high
    /// under air and a full block under water, flowing water of level 5 a third high, the
    /// camera's full block under a sturdy ceiling, and what water and lava replace.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn fluid_queries_answer_vanilla_values_at_a_position() {
        use mod_api::{Direction, Host};

        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let y = pool(
                game,
                &[
                    ([1, 0, 1], "minecraft:water", "0"),
                    ([1, 0, 4], "minecraft:water", "0"),
                    ([1, 1, 4], "minecraft:water", "0"),
                    ([1, 0, 7], "minecraft:water", "5"),
                    ([1, 0, 10], "minecraft:lava", "0"),
                    ([1, 0, 12], "minecraft:lava", "5"),
                ],
            );
            let stone = game.pack.assets.registry.default_state("minecraft:stone");
            game.world.set(1, y + 1, 1, stone.unwrap()).unwrap();
            let (water, flowing_water, lava, flowing_lava, empty) = {
                let host: &dyn Host = game;
                let key = |id| host.fluid_key(id).unwrap();
                (
                    key("minecraft:water"),
                    key("minecraft:flowing_water"),
                    key("minecraft:lava"),
                    key("minecraft:flowing_lava"),
                    key("minecraft:empty"),
                )
            };
            let tank_only = game.fluid_key("sunrise:definition_test").unwrap();
            let host: &dyn Host = game;
            let at = |x, y, z| [x, y, z];

            // `Fluid.getTickDelay`: water 5 and lava 30 in the Overworld, a flowing form as its source.
            for (fluid, delay) in [
                (water, 5),
                (flowing_water, 5),
                (lava, 30),
                (flowing_lava, 30),
                (empty, 0),
            ] {
                assert_eq!(
                    level.fluid_tick_delay(host, fluid, at(1, y, 1)),
                    Some(delay)
                );
            }
            assert_eq!(level.fluid_tick_delay(host, tank_only, at(1, y, 1)), None);
            assert_eq!(level.fluid_tick_delay(host, water, at(16, y, 0)), None);
            let height = level.height(host).unwrap();
            assert_eq!(
                level.fluid_tick_delay(host, water, at(1, height.0, 1)),
                Some(5)
            );
            assert_eq!(
                level.fluid_tick_delay(host, water, at(1, height.0 - 1, 1)),
                None
            );

            // The surface: 8/9 under air, a full block under the same fluid, `amount / 9` flowing.
            let ninth = |n: f32| n / 9.0;
            assert_eq!(level.fluid_height(host, at(1, y, 4)), Some(1.0));
            assert_eq!(level.fluid_height(host, at(1, y + 1, 4)), Some(ninth(8.0)));
            assert_eq!(level.fluid_height(host, at(1, y, 7)), Some(ninth(3.0)));
            assert_eq!(level.fluid_height(host, at(1, y, 9)), Some(0.0));
            assert_eq!(level.fluid_height(host, at(16, y, 0)), None);
            // The camera sees a source under a sturdy block as full, and nothing else differently.
            assert_eq!(level.fluid_height_for_camera(host, at(1, y, 1)), Some(1.0));
            assert_eq!(level.fluid_height(host, at(1, y, 1)), Some(ninth(8.0)));
            assert_eq!(
                level.fluid_height_for_camera(host, at(1, y + 1, 4)),
                Some(ninth(8.0))
            );
            assert_eq!(
                level.fluid_height_for_camera(host, at(1, y, 7)),
                Some(ninth(3.0))
            );

            // Water gives way to lava flowing down only, lava to water from 4/9 of a block high.
            let replaced = |fluid, x, z, direction| {
                level.fluid_can_be_replaced_with(host, at(x, y, z), fluid, direction)
            };
            assert_eq!(replaced(lava, 1, 1, Direction::Down), Some(true));
            assert_eq!(replaced(lava, 1, 1, Direction::East), Some(false));
            assert_eq!(replaced(water, 1, 1, Direction::Down), Some(false));
            assert_eq!(replaced(flowing_water, 1, 10, Direction::East), Some(true));
            assert_eq!(replaced(flowing_water, 1, 12, Direction::East), Some(false));
            assert_eq!(replaced(lava, 1, 10, Direction::East), Some(false));
            assert_eq!(replaced(water, 1, 9, Direction::Up), Some(true));
            assert_eq!(replaced(tank_only, 1, 9, Direction::Up), Some(false));
            assert_eq!(
                level.fluid_can_be_replaced_with(host, at(16, y, 0), water, Direction::Down),
                None
            );
        });
    }

    /// Lava flows fast in the Nether: the delay follows the level's own dimension.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn lava_ticks_every_ten_in_a_fast_lava_level() {
        use mod_api::Host;

        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let y = pool(game, &[]);
            game.world.fast_lava = true;
            let host: &dyn Host = game;
            let delay = |id| level.fluid_tick_delay(host, host.fluid_key(id).unwrap(), [1, y, 1]);
            assert_eq!(delay("minecraft:lava"), Some(10));
            assert_eq!(delay("minecraft:flowing_lava"), Some(10));
            assert_eq!(delay("minecraft:water"), Some(5));
        });
    }

    /// An explicit fluid tick runs the flow step: a water source on a floor fills its four
    /// neighbours with flowing water of level 1 (amount 7), once; an empty cell, an unloaded
    /// column, a position below the world, a stale handle and a level not in place do nothing.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn an_explicit_fluid_tick_spreads_a_source_and_refuses_other_positions() {
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            let nether = LevelRef::new(level.epoch(), mod_api::DimensionKey::NETHER);
            let y = pool(game, &[([8, 0, 8], "minecraft:water", "0")]);
            let amounts = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                [[9, 8], [7, 8], [8, 9], [8, 7], [8, 8]]
                    .map(|[x, z]| level.fluid_state(game, [x, y, z]).map(|f| f.amount))
            };
            assert_eq!(amounts(game), [Some(0), Some(0), Some(0), Some(0), Some(8)]);
            assert!(level.fluid_tick(game, [8, y, 8]));
            assert_eq!(amounts(game), [Some(7), Some(7), Some(7), Some(7), Some(8)]);
            let flowing = level.fluid_state(game, [9, y, 8]).unwrap();
            assert!(!flowing.is_source() && !flowing.falling);

            // Ticking the empty fluid is allowed and changes nothing.
            assert!(level.fluid_tick(game, [3, y, 3]));
            assert_eq!(
                level.fluid_state(game, [3, y, 3]).map(|f| f.amount),
                Some(0)
            );
            assert!(
                !level.fluid_tick(game, [1_000_000, y, 1_000_000]),
                "unloaded"
            );
            let below = game.world.bottom_y() - 1;
            assert!(!level.fluid_tick(game, [8, below, 8]), "below the world");
            assert!(!stale.fluid_tick(game, [8, y, 8]));
            assert!(!nether.fluid_tick(game, [8, y, 8]), "a level not in place");
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn forced_chunks_set_clear_and_query_like_forceload() {
        with_chunk_streaming(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let far = game.level_chunk_ref(level, [40, -7]).unwrap();
            let near = game.level_chunk_ref(level, [-3, 2]).unwrap();
            assert_eq!(game.chunk_forced(far), Some(false));
            assert_eq!(
                collect_chunks(|visit| level.visit_forced_chunks(game, visit)),
                Ok(vec![])
            );
            // A synchronous load leaves existing columns for the normal owner
            // update to save and unload, including their entity data.
            let old = std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(-40, 40));
            game.world.chunks.insert((-40, 40), old.clone());
            game.world.modified.insert((-40, 40));
            assert!(!game.chunk_loaded(far));
            assert!(game.chunk_set_forced(far, true));
            for x in 39..=41 {
                for z in -8..=-6 {
                    assert!(game.world.chunks.contains_key(&(x, z)), "missing {x},{z}");
                }
            }
            assert!(game.world.chunks[&(40, -7)].lit);
            assert!(std::sync::Arc::ptr_eq(&old, &game.world.chunks[&(-40, 40)]));
            assert!(game.world.modified.contains(&(-40, 40)));
            assert!(!game.chunk_set_forced(far, true), "already forced");
            assert!(game.chunk_set_forced(near, true));
            assert_eq!(game.chunk_forced(far), Some(true));
            // The command's own set and order.
            assert!(game.server.streaming.forced.contains(&(40, -7)));
            let listed = collect_chunks(|visit| level.visit_forced_chunks(game, visit)).unwrap();
            // Signed packed positions: signed z first, then unsigned low bits (x).
            assert_eq!(listed, vec![[40, -7], [-3, 2]]);
            let mut first = Vec::new();
            assert_eq!(
                level.visit_forced_chunks(game, &mut |pos| {
                    first.push(pos);
                    ControlFlow::Break(())
                }),
                Ok(ControlFlow::Break(()))
            );
            assert_eq!(first, [[40, -7]]);
            assert!(game.chunk_set_forced(far, false));
            assert!(!game.chunk_set_forced(far, false), "already clear");
            assert_eq!(
                collect_chunks(|visit| level.visit_forced_chunks(game, visit)),
                Ok(vec![[-3, 2]])
            );
            assert!(!game.server.streaming.forced.contains(&(40, -7)));
            // Unchanged adds and removals do not load a missing column.
            let pending = game.level_chunk_ref(level, [200, 200]).unwrap();
            assert!(game.server.streaming.set_forced(200, 200, true));
            assert!(!game.chunk_loaded(pending));
            assert!(!game.chunk_set_forced(pending, true));
            assert!(!game.chunk_loaded(pending));
            assert!(game.chunk_set_forced(pending, false));
            assert!(!game.chunk_loaded(pending));
            assert!(game.level_chunk_ref(level, [1_875_000, 0]).is_none());
            assert!(game.level_chunk_ref(level, [0, -1_875_001]).is_none());
            assert!(!game.chunk_set_forced(ChunkRef::new(level, [i32::MAX, 0]), true));
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn paused_forced_loads_are_queued_and_stopped_generation_refuses_adds() {
        if run_fresh_cases(
            concat!(
                "cut_layout::game::server_side::native_level_refs::tests::",
                "paused_forced_loads_are_queued_and_stopped_generation_refuses_adds"
            ),
            "SUNRISE_FORCED_STREAMING_CHILD",
            &["forced_streaming"],
        ) {
            return;
        }

        with_game(|game| {
            use std::sync::Arc;
            use sunrise_level::{chunk::Chunk, world::World};

            game.options.headless = true;
            game.server.streaming.radius = 0;

            // A remote streamer can use a column from a packet; it cannot
            // generate a missing column or wait for its lighting and neighbours.
            let mut remote_world = World::new(game.world.air, 0);
            let streamer = &mut game.server.streaming;
            streamer.remote = true;
            assert!(!streamer.set_forced_loaded(&mut remote_world, 4, 4, true));
            assert!(!streamer.commanded().contains(&(4, 4)));
            assert!(!streamer.forced.contains(&(4, 4)));
            remote_world
                .chunks
                .insert((4, 4), Arc::new(Chunk::empty(4, 4)));
            streamer.paused = false;
            assert!(streamer.set_forced_loaded(&mut remote_world, 4, 4, true));
            assert!(streamer.commanded().contains(&(4, 4)));
            assert!(streamer.forced.contains(&(4, 4)));
            assert!(streamer.set_forced(4, 4, false));
            streamer.remote = false;
            streamer.paused = true;

            let level = game.level(game.dim.id()).unwrap();
            let queued = game.level_chunk_ref(level, [40, -7]).unwrap();
            assert!(!game.chunk_loaded(queued));
            assert!(game.chunk_set_forced(queued, true));
            assert!(!game.chunk_set_forced(queued, true));
            assert_eq!(
                collect_chunks(|visit| level.visit_forced_chunks(game, visit)),
                Ok(vec![[40, -7]])
            );
            assert!(!game.chunk_loaded(queued), "paused streaming does no work");

            game.server.streaming.paused = false;
            game.prepare_levels().unwrap();
            assert!(game.world.chunks[&(40, -7)].lit);
            for x in 39..=41 {
                for z in -8..=-6 {
                    assert!(game.world.chunks.contains_key(&(x, z)), "missing {x},{z}");
                }
            }

            game.server.streaming.abort_generation();
            let missing = game.level_chunk_ref(level, [60, -7]).unwrap();
            assert!(!game.chunk_set_forced(missing, true));
            assert_eq!(
                collect_chunks(|visit| level.visit_forced_chunks(game, visit)),
                Ok(vec![[40, -7]])
            );
            assert!(game.chunk_set_forced(queued, false));
            assert!(!game.chunk_set_forced(queued, true));
            assert_eq!(
                collect_chunks(|visit| level.visit_forced_chunks(game, visit)),
                Ok(vec![])
            );
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn forced_chunk_changes_the_ticket_sources_and_the_saved_set() {
        with_chunk_streaming(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let chunk = game.level_chunk_ref(level, [5, 6]).unwrap();
            assert!(game.chunk_set_forced(chunk, true));
            assert!(game.levels_to_tick().contains(&game.dim));
            for (x, z) in [(-1, -3), (4, -3)] {
                assert!(game.server.streaming.set_forced(x, z, true));
            }
            game.save_chunk_tickets().unwrap();
            let saved = game.server.save.0.load_forced_chunks();
            assert_eq!(
                saved,
                std::collections::HashSet::from([(5, 6), (-1, -3), (4, -3)])
            );
            game.server.streaming.clear_forced();
            assert_eq!(
                level.visit_forced_chunks(game, &mut |_| panic!("cleared ticket was visited")),
                Ok(ControlFlow::Continue(()))
            );
            game.server.streaming.load_forced(saved, Vec::new());
            game.server
                .streaming
                .load_forced([(4, -3)].into(), Vec::new());
            assert_eq!(
                collect_chunks(|visit| level.visit_forced_chunks(game, visit)),
                Ok(vec![[4, -3], [-1, -3], [5, 6]])
            );
            assert!(game.chunk_set_forced(chunk, false));
            game.server.streaming.clear_forced();
            game.save_chunk_tickets().unwrap();
            assert!(game.server.save.0.load_forced_chunks().is_empty());
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn forced_chunks_of_a_parked_level_and_stale_handles() {
        with_chunk_streaming(|game| {
            let stale_epoch = WorldEpoch::new(game.native_lifecycle.epoch().unwrap().get() + 1);
            let level = game.level(game.dim.id()).unwrap();
            let stale_level = mod_api::LevelRef::new(stale_epoch, level.dimension());
            let stale = ChunkRef::new(stale_level, [0, 0]);
            assert_eq!(game.chunk_forced(stale), None);
            assert!(!game.chunk_set_forced(stale, true));
            assert_eq!(
                stale_level.visit_forced_chunks(game, &mut |_| panic!("stale ticket visit")),
                Err(LevelReadError::Stale)
            );
            assert!(game.level_chunk_ref(stale_level, [0, 0]).is_none());
            // A level the server has not opened.
            let other = if game.dim == Dim::Nether {
                Dim::End
            } else {
                Dim::Nether
            };
            let parked = mod_api::LevelRef::new(level.epoch(), other.key());
            let chunk = ChunkRef::new(parked, [1, 1]);
            assert_eq!(game.chunk_forced(chunk), None);
            assert!(!game.chunk_set_forced(chunk, true));
            // Opened, then parked.
            let back = game.dim;
            game.enter_level(other).unwrap();
            game.enter_level(back).unwrap();
            assert_eq!(game.chunk_forced(chunk), Some(false));
            assert!(!game.chunk_loaded(chunk));
            assert!(game.chunk_set_forced(chunk, true));
            assert!(
                game.chunk_loaded(chunk),
                "parked level loaded before returning"
            );
            assert_eq!(game.dim, back, "forcing must not switch the acting level");
            assert_eq!(
                collect_chunks(|visit| parked.visit_forced_chunks(game, visit)),
                Ok(vec![[1, 1]])
            );
            assert_eq!(
                collect_chunks(|visit| level.visit_forced_chunks(game, visit)),
                Ok(vec![])
            );
            assert!(game.levels_to_tick().contains(&other));
            assert!(game.chunk_set_forced(chunk, false));
            assert!(!game.levels_to_tick().contains(&other));
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_level_ref_draws_from_its_levels_own_stream() {
        use mod_api::{LevelRef, ServerHost};
        use sunrise_core::random::RandomSource;

        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            // The level in place: the draws advance `World.random` itself.
            let mut expected = game.world.random.clone();
            let host: &mut dyn ServerHost = game;
            let drawn: Vec<i32> = (0..3)
                .map(|_| level.with_random(&mut *host, |r| r.next_int()).unwrap())
                .collect();
            let want: Vec<i32> = (0..3).map(|_| expected.next_int()).collect();
            assert_eq!(drawn, want);
            assert_eq!(game.world.random.next_int(), expected.next_int());

            // A parked level answers with its own stream, not the one in place.
            let other = if game.dim == Dim::Nether {
                Dim::End
            } else {
                Dim::Nether
            };
            let parked = LevelRef::new(level.epoch(), other.key());
            let host: &mut dyn ServerHost = game;
            assert!(
                parked
                    .with_random::<()>(host, |_| panic!("unopened level lent its random stream"))
                    .is_none(),
                "not open yet"
            );
            let back = game.dim;
            game.enter_level(other).unwrap();
            game.enter_level(back).unwrap();
            let mut expected = game
                .server
                .levels
                .0
                .parked_world_of(other.id())
                .unwrap()
                .random
                .clone();
            let in_place = game.world.random.clone();
            let host: &mut dyn ServerHost = game;
            let drawn = parked.with_random(&mut *host, |r| r.next_long()).unwrap();
            assert_eq!(drawn, expected.next_long());
            let mut after = game
                .server
                .levels
                .0
                .parked_world_of(other.id())
                .unwrap()
                .random
                .clone();
            assert_eq!(after.next_long(), expected.next_long());
            let mut untouched = in_place;
            assert_eq!(game.world.random.next_long(), untouched.next_long());

            // A handle of an earlier open names nothing.
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), game.dim.key());
            let host: &mut dyn ServerHost = game;
            assert!(stale.with_random(&mut *host, |r| r.next_int()).is_none());
        });
    }

    #[cfg(feature = "client")]
    fn level_random_state(random: &sunrise_core::random::LegacyRandom) -> (u64, Option<u64>) {
        let (seed, gaussian) = <(u64, Option<f64>)>::from(random);
        (seed, gaussian.map(f64::to_bits))
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_level_rng_draws_survive_transaction_abort_on_active_and_parked_levels() {
        use mod_api::ObjectiveRenderType;
        use sunrise_core::random::{LegacyRandom, LevelRandom, RandomSource};
        use sunrise_game_data::text::Text;
        use sunrise_gameplay::game::journal_level_write::JournalLevelWrite;

        with_game(|game| {
            let here = game.dim;
            let other = if here == Dim::Nether {
                Dim::End
            } else {
                Dim::Nether
            };
            game.enter_level(other).unwrap();
            let parked = game.level(other.id()).unwrap();
            game.enter_level(here).unwrap();
            let active = game.level(here.id()).unwrap();

            let mut expected_active = LegacyRandom::new(31);
            let mut expected_parked = LegacyRandom::new(47);
            expected_active.next_gaussian();
            expected_parked.next_gaussian();
            game.world.random = LevelRandom::for_level(expected_active.clone());
            game.server
                .levels
                .0
                .parked_world_mut_of(other.id())
                .unwrap()
                .random = LevelRandom::for_level(expected_parked.clone());
            let states = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                [
                    level_random_state(&game.world.random),
                    level_random_state(
                        &game
                            .server
                            .levels
                            .0
                            .parked_world_of(other.id())
                            .unwrap()
                            .random,
                    ),
                ]
            };
            let draw = |rng: &mut dyn mod_api::RandomSource| {
                (rng.next_gaussian().to_bits(), rng.next_long())
            };

            let advanced = game.storage_scope(|game, _| {
                game.add_objective(
                    "outer_discarded",
                    "dummy",
                    Text::literal("Outer"),
                    ObjectiveRenderType::Integer,
                )
                .unwrap();
                {
                    let mut writer = JournalLevelWrite::new(game, active).unwrap();
                    let host: &mut dyn mod_api::LevelWriteHost = &mut writer;
                    assert_eq!(
                        host.with_random(draw),
                        Some((
                            expected_active.next_gaussian().to_bits(),
                            expected_active.next_long()
                        )),
                    );
                }

                let advanced = game.storage_scope(|game, _| {
                    game.add_objective(
                        "child_discarded",
                        "dummy",
                        Text::literal("Child"),
                        ObjectiveRenderType::Integer,
                    )
                    .unwrap();
                    for (level, expected) in [
                        (active, &mut expected_active),
                        (parked, &mut expected_parked),
                    ] {
                        assert_eq!(
                            level.with_random(game, draw),
                            Some((expected.next_gaussian().to_bits(), expected.next_long()))
                        );
                    }
                    let advanced = [
                        level_random_state(&expected_active),
                        level_random_state(&expected_parked),
                    ];
                    assert_eq!(states(game), advanced);

                    (advanced, false)
                });
                assert!(game.objective("child_discarded").is_none());
                assert!(game.objective("outer_discarded").is_some());
                assert_eq!(
                    states(game),
                    advanced,
                    "child abort rolls back journaled state without rewinding active or parked RNG"
                );

                (advanced, false)
            });
            assert!(game.objective("outer_discarded").is_none());
            assert_eq!(
                states(game),
                advanced,
                "outer abort rolls back journaled state without rewinding active or parked RNG"
            );
            for (level, expected) in [
                (active, &mut expected_active),
                (parked, &mut expected_parked),
            ] {
                assert_eq!(
                    level.with_random(game, draw),
                    Some((expected.next_gaussian().to_bits(), expected.next_long())),
                );
            }
        });
    }

    #[cfg(feature = "client")]
    fn collect_mod_tickets(
        game: &sunrise_gameplay::game::ServerGame<'_>,
        level: LevelRef,
        controller: TicketController,
    ) -> Result<Vec<(TicketOwner, [i32; 2], bool)>, LevelReadError> {
        collect_chunks(|visit| {
            level.visit_mod_forced_chunks(game, controller, &mut |owner, pos, natural| {
                visit((owner, pos, natural))
            })
        })
    }

    #[cfg(feature = "client")]
    fn in_mod(id: &'static str) -> mod_api::content::crash::ModGuard {
        mod_platform::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
            id,
            hook: "test",
            part: None,
        })
    }

    #[cfg(feature = "client")]
    fn record_validation(controller: &'static str, validation: &dyn TicketValidation) {
        let blocks = validation.block_tickets().keys().copied().collect();
        SEEN.with_borrow_mut(|seen| seen.push((controller, blocks)));
    }

    #[cfg(feature = "client")]
    use mod_api::ChunkLoadError;
    use mod_api::{self, TicketController};
    #[cfg(feature = "client")]
    use mod_api::{Hooks, ModEntry, Registrar, ServerHost, TicketValidation};
    #[cfg(feature = "client")]
    use std::sync::Arc;
    #[cfg(feature = "client")]
    use sunrise_level::mod_chunk_tickets::{ModTicket, TicketOwner};

    /// Registers three controllers: `quarry` and `drill` validate, `plain` doesn't. `quarry`'s
    /// validation prunes the block at [1, 64, 1]; each validation tells what it was shown.
    #[cfg(feature = "client")]
    struct Machines;
    #[cfg(feature = "client")]
    #[mod_api::__subscriptions]
    impl Hooks for Machines {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            let quarry = r.register_ticket_controller(
                "quarry",
                Some(Arc::new(
                    |_: &mut dyn ServerHost, _, validation: &mut dyn TicketValidation| {
                        record_validation("machines:quarry", validation);
                        validation.remove_block_tickets([1, 64, 1]);
                    },
                )),
            );
            let drill = r.register_ticket_controller(
                "machines:drill",
                Some(Arc::new(
                    |_: &mut dyn ServerHost, _, validation: &mut dyn TicketValidation| {
                        record_validation("machines:drill", validation);
                    },
                )),
            );
            let plain = r.register_ticket_controller("plain", None);
            ISSUED.with_borrow_mut(|issued| {
                issued.insert("machines:quarry", quarry);
                issued.insert("machines:drill", drill);
                issued.insert("machines:plain", plain);
            });
        }
    }
    /// Registers one validating controller, and prunes everything it is shown.
    #[cfg(feature = "client")]
    struct Other;
    #[cfg(feature = "client")]
    #[mod_api::__subscriptions]
    impl Hooks for Other {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            let vault = r.register_ticket_controller(
                "vault",
                Some(Arc::new(
                    |_: &mut dyn ServerHost, _, validation: &mut dyn TicketValidation| {
                        record_validation("other:vault", validation);
                        validation.remove_block_tickets([1, 64, 1]);
                    },
                )),
            );
            ISSUED.with_borrow_mut(|issued| {
                issued.insert("other:vault", vault);
            });
        }
    }
    thread_local! {
        /// What each validation was shown: its controller, then the block positions.
        static SEEN: std::cell::RefCell<Vec<(&'static str, Vec<[i32; 3]>)>> =
            const { std::cell::RefCell::new(Vec::new()) };
        /// Real registrar-issued handles captured during the most recent fixture registration.
        static ISSUED: std::cell::RefCell<std::collections::HashMap<&'static str, TicketController>> =
            std::cell::RefCell::new(std::collections::HashMap::new());
    }

    #[cfg(feature = "client")]
    fn mods() -> mod_api::Mods {
        vec![
            ModEntry::new("machines", Box::new(Machines)),
            ModEntry::new("other", Box::new(Other)),
        ]
        .into()
    }
    #[cfg(feature = "client")]
    fn controller(id: &str) -> TicketController {
        ISSUED.with_borrow(|issued| *issued.get(id).unwrap())
    }
    /// The save-deserialization slot of a registered controller id.
    #[cfg(feature = "client")]
    fn known_slot(id: &str) -> Option<u32> {
        mod_api::__private::game::ticket_controller_slot_by_id(id)
    }
    #[cfg(feature = "client")]
    fn quarry() -> TicketController {
        controller("machines:quarry")
    }
    /// A game whose mods are `Machines` and `Other`, with their controllers registered.
    #[cfg(feature = "client")]
    fn with_machines(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
        mod_platform::testing::with_mods(mods(), || {
            with_chunk_streaming(|game| {
                mod_api::testing::with_installed_ticket_controllers(|| test(game));
            });
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn water_tickets_hydrate_real_farmland_and_preserve_vanilla_water_and_rain() {
        use std::sync::Arc;
        use sunrise_level::chunk::Chunk;

        with_machines(|game| {
            game.install_live_features();
            let level = game.level(game.dim.id()).unwrap();
            let registry = game.pack.assets.registry;
            let dry = registry.default_state("minecraft:farmland").unwrap();
            let wheat = registry.default_state("minecraft:wheat").unwrap();
            let air = registry.default_state("minecraft:air").unwrap();
            let water = registry.default_state("minecraft:water").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            let plains = game.res.biomes().id("minecraft:plains");
            let mut chunk = Chunk::empty(0, 0);
            for qy in 0..game.world.shape.height / 4 {
                for qx in 0..4 {
                    for qz in 0..4 {
                        chunk.set_biome_cell(qx, qy, qz, plains);
                    }
                }
            }
            game.world.chunks.insert((0, 0), Arc::new(chunk));
            game.block_ticks
                .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
                .unwrap();
            assert!(level.set_weather_levels(game, 0.0, 0.0));
            let moisture = |game: &sunrise_gameplay::game::ServerGame<'_>, pos| {
                level
                    .block_property(game, pos, "moisture")
                    .unwrap()
                    .parse::<i32>()
                    .unwrap()
            };
            for x in 4..9 {
                for z in 4..9 {
                    game.world.set(x, 64, z, dry).unwrap();
                    game.world.set(x, 65, z, wheat).unwrap();
                }
            }
            game.world.set(9, 64, 4, dry).unwrap();
            game.world.set(9, 65, 4, wheat).unwrap();
            let ticket = level
                .add_water_ticket(game, [4, 64, 4], [9, 65, 9])
                .unwrap();
            for x in 4..9 {
                for z in 4..9 {
                    assert!(level.has_water_ticket(game, [x, 64, z]));
                    assert!(level.block_random_tick(game, [x, 64, z]));
                    assert_eq!(moisture(game, [x, 64, z]), 7);
                }
            }
            assert!(!level.has_water_ticket(game, [9, 64, 4]));
            assert!(level.block_random_tick(game, [9, 64, 4]));
            assert_eq!(moisture(game, [9, 64, 4]), 0);
            assert!(ticket.remove(game));
            assert!(!ticket.remove(game));
            for expected in (0..7).rev() {
                for x in 4..9 {
                    for z in 4..9 {
                        assert!(level.block_random_tick(game, [x, 64, z]));
                        assert_eq!(moisture(game, [x, 64, z]), expected);
                    }
                }
            }
            // A real water block at vanilla's four-block edge still hydrates without a ticket.
            game.world.set(0, 64, 4, water).unwrap();
            assert!(level.block_random_tick(game, [4, 64, 4]));
            assert_eq!(moisture(game, [4, 64, 4]), 7);
            game.world.set(0, 64, 4, air).unwrap();
            game.world.set(4, 64, 4, dry).unwrap();
            game.world.set(4, 65, 4, air).unwrap();
            assert!(level.set_weather_levels(game, 1.0, 0.0));
            assert_eq!(level.is_raining_at(game, [4, 65, 4]), Some(true));
            assert!(level.block_random_tick(game, [4, 64, 4]));
            assert_eq!(moisture(game, [4, 64, 4]), 7);
            // A roof still excludes rain: tickets have not replaced the rain predicate.
            game.world.set(4, 66, 4, stone).unwrap();
            assert_eq!(level.is_raining_at(game, [4, 65, 4]), Some(false));
            assert!(level.block_random_tick(game, [4, 64, 4]));
            assert_eq!(moisture(game, [4, 64, 4]), 6);
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn water_ticket_handles_isolate_dimensions_owners_and_unload_lifetimes() {
        with_machines(|game| {
            let overworld = game.level(game.dim.id()).unwrap();
            // Unattributed addition is admitted without TLS caller context.
            let anon = overworld
                .add_water_ticket(game, [0, 64, 0], [5, 65, 5])
                .unwrap();
            assert!(overworld.has_water_ticket(game, [0, 64, 0]));
            let stale = LevelRef::new(
                WorldEpoch::new(overworld.epoch().get() + 1),
                overworld.dimension(),
            );
            assert!(!stale.has_water_ticket(game, [0, 64, 0]));
            assert!(
                stale
                    .add_water_ticket(game, [0, 64, 0], [5, 65, 5])
                    .is_none()
            );
            let guard = in_mod("machines");
            let a = overworld
                .add_water_ticket(game, [-17, 64, -1], [18, 65, 2])
                .unwrap();
            drop(guard);
            for pos in [[-17, 64, -1], [17, 64, 1]] {
                assert!(overworld.has_water_ticket(game, pos));
            }
            for pos in [
                [-18, 64, 0],
                [18, 64, 0],
                [0, 63, 0],
                [0, 65, 0],
                [-1, 64, -2],
                [-1, 64, 2],
            ] {
                assert!(!overworld.has_water_ticket(game, pos), "{pos:?}");
            }
            // A handed-over handle can be removed from another mod without disturbing overlaps.
            let guard = in_mod("other");
            assert!(a.remove(game), "any holder with the handle can remove it");
            drop(guard);
            assert!(!a.remove(game));
            assert!(!overworld.has_water_ticket(game, [-17, 64, -1]));
            assert!(overworld.has_water_ticket(game, [0, 64, 0]));
            game.enter_level(Dim::Nether).unwrap();
            let nether = game.level(game.dim.id()).unwrap();
            assert!(!nether.has_water_ticket(game, [0, 64, 0]));
            assert!(overworld.has_water_ticket(game, [0, 64, 0]));
            let guard = in_mod("machines");
            let b = nether
                .add_water_ticket(game, [0, 64, 0], [5, 65, 5])
                .unwrap();
            let parked_owned = overworld
                .add_water_ticket(game, [10, 64, 0], [15, 65, 5])
                .unwrap();
            drop(guard);
            let guard = in_mod("other");
            let overlap = nether
                .add_water_ticket(game, [3, 64, 3], [4, 65, 4])
                .unwrap();
            drop(guard);
            assert!(mod_api::__private::set_mod_enabled("machines", false));
            assert!(!overworld.has_water_ticket(game, [10, 64, 0]));
            assert!(overworld.has_water_ticket(game, [0, 64, 0]));
            assert!(!nether.has_water_ticket(game, [0, 64, 0]));
            assert!(nether.has_water_ticket(game, [3, 64, 3]));
            // A disabled installed mod is refused instead of leaking an unattributed ticket.
            let guard = in_mod("machines");
            assert!(
                nether
                    .add_water_ticket(game, [6, 64, 6], [7, 65, 7])
                    .is_none()
            );
            drop(guard);
            assert!(!nether.has_water_ticket(game, [6, 64, 6]));
            // Unattributed callers still succeed without reviving old tickets.
            let during_disable = nether
                .add_water_ticket(game, [6, 64, 6], [7, 65, 7])
                .unwrap();
            assert!(nether.has_water_ticket(game, [6, 64, 6]));
            assert!(!nether.has_water_ticket(game, [0, 64, 0]));
            assert!(during_disable.remove(game));
            assert!(!nether.has_water_ticket(game, [6, 64, 6]));
            assert!(mod_api::__private::set_mod_enabled("machines", true));
            assert!(
                !overworld.has_water_ticket(game, [10, 64, 0]),
                "re-enable never restores old tickets"
            );
            assert!(!b.remove(game));
            assert!(!parked_owned.remove(game));
            // Server chunk unload invalidates only tickets mastered in that chunk
            // (`FarmlandWaterManager.removeTickets`), leaving secondary chunks and overlaps intact.
            let master_five = nether
                .add_water_ticket(game, [76, 64, 2], [101, 65, 10])
                .unwrap();
            let master_six = nether
                .add_water_ticket(game, [92, 64, 2], [114, 65, 10])
                .unwrap();
            // Secondary covered chunk (4, 0) leaves master_five active.
            game.native_chunk_unload(nether.dimension_id(), [4, 0]);
            assert!(nether.has_water_ticket(game, [76, 64, 2]));
            assert!(nether.has_water_ticket(game, [100, 64, 2]));
            // Master chunk (5, 0) invalidates master_five while preserving master_six's overlap.
            game.native_chunk_unload(nether.dimension_id(), [5, 0]);
            assert!(!master_five.remove(game));
            assert!(!nether.has_water_ticket(game, [76, 64, 2]));
            assert!(nether.has_water_ticket(game, [92, 64, 2]));
            assert!(nether.has_water_ticket(game, [113, 64, 2]));
            // Reissue after unload works independently of the invalidated handle.
            let reissued = nether
                .add_water_ticket(game, [76, 64, 2], [101, 65, 10])
                .unwrap();
            assert!(!master_five.remove(game));
            assert!(nether.has_water_ticket(game, [76, 64, 2]));
            assert!(reissued.remove(game));
            assert!(master_six.remove(game));
            // Chunk unload also reaches parked levels without disturbing the active level.
            let parked_chunk_ticket = overworld
                .add_water_ticket(game, [32, 64, 32], [40, 65, 40])
                .unwrap();
            assert!(overworld.has_water_ticket(game, [32, 64, 32]));
            game.native_chunk_unload(overworld.dimension_id(), [2, 2]);
            assert!(!overworld.has_water_ticket(game, [32, 64, 32]));
            assert!(!parked_chunk_ticket.remove(game));
            assert!(nether.has_water_ticket(game, [3, 64, 3]));
            // Unattributed removal works on parked levels.
            assert!(anon.remove(game));
            assert!(!overworld.has_water_ticket(game, [0, 64, 0]));
            let fresh = overworld
                .add_water_ticket(game, [0, 64, 0], [5, 65, 5])
                .unwrap();
            game.close_loaded_levels_for_stop();
            assert!(!overworld.has_water_ticket(game, [0, 64, 0]));
            assert!(!nether.has_water_ticket(game, [3, 64, 3]));
            assert!(!fresh.remove(game));
            assert!(!overlap.remove(game));
            // The client cannot query or mutate the server's in-memory marks.
            game.options.execution_role =
                sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
            assert!(!nether.has_water_ticket(game, [0, 64, 0]));
            assert!(
                nether
                    .add_water_ticket(game, [0, 64, 0], [5, 65, 5])
                    .is_none()
            );
            assert!(!b.remove(game));
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn water_tickets_are_not_restored_with_saved_farmland() {
        use std::sync::Arc;
        use sunrise_level::{chunk::Chunk, world::World};
        use sunrise_sim::save::SaveDir;

        with_machines(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let registry = game.pack.assets.registry;
            let dry = registry.default_state("minecraft:farmland").unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
                .unwrap();
            game.world.set(4, 64, 4, dry).unwrap();
            let old = level
                .add_water_ticket(game, [4, 64, 4], [9, 65, 9])
                .unwrap();
            assert!(level.block_random_tick(game, [4, 64, 4]));
            assert_eq!(
                level.block_property(game, [4, 64, 4], "moisture"),
                Some("7")
            );
            game.server
                .save
                .0
                .save(
                    &[game.world.chunks.get(&(0, 0)).unwrap().clone()],
                    registry,
                    game.res.biomes(),
                )
                .unwrap();
            let root = game.server.save.0.root.clone();
            let shape = game.world.shape;
            game.close_loaded_levels_for_stop();
            let save = SaveDir::new(root, shape, Some(Default::default()));
            let chunk = save.load(0, 0, registry, game.res.biomes()).unwrap();
            game.world = World::new(registry.default_state("minecraft:air").unwrap(), 0);
            game.world.chunks.insert((0, 0), Arc::new(chunk));
            game.server.save.0 = Arc::new(save);
            game.server.levels.0.reset_active_closed_for_test();
            game.native_lifecycle
                .begin_open(Some(mod_api::Authority::IntegratedServer));
            let reopened = game.level(game.dim.id()).unwrap();
            assert_ne!(level.epoch(), reopened.epoch());
            assert!(!reopened.has_water_ticket(game, [4, 64, 4]));
            assert!(!old.remove(game));
            assert_eq!(
                reopened.block_property(game, [4, 64, 4], "moisture"),
                Some("7")
            );
            assert!(reopened.block_random_tick(game, [4, 64, 4]));
            assert_eq!(
                reopened.block_property(game, [4, 64, 4], "moisture"),
                Some("6")
            );
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn mod_tickets_are_scoped_by_owner_and_share_the_forceload_store() {
        use mod_api::TicketOwner::{Block, Entity};
        // Capture a real handle from an earlier registration table with the same controller name
        // and slot so we can prove it confers no authority in the active registration table.
        let foreign_quarry = mod_platform::testing::with_mods(mods(), quarry);
        with_machines(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let chunk = game.level_chunk_ref(level, [5, 6]).unwrap();
            let (a, b) = (Block([80, 64, 96]), Entity(7));
            assert_ne!(foreign_quarry, quarry());
            assert_eq!(foreign_quarry.id(), "");
            assert_eq!(
                level.visit_mod_forced_chunks(game, foreign_quarry, &mut |_, _, _| panic!(
                    "refused ticket visit"
                )),
                Err(LevelReadError::Unavailable)
            );
            assert!(!game.chunk_set_mod_forced(chunk, foreign_quarry, a, true, false));
            assert!(!game.server.streaming.forced.contains(&(5, 6)));
            // A registered controller works outside a mod callback without TLS attribution.
            assert!(game.chunk_set_mod_forced(chunk, quarry(), a, true, false));
            assert!(
                !game.chunk_set_mod_forced(chunk, quarry(), a, true, false),
                "held"
            );
            assert!(game.chunk_set_mod_forced(chunk, quarry(), b, true, false));
            assert!(game.server.streaming.forced.contains(&(5, 6)));
            assert_eq!(game.chunk_forced(chunk), Some(false), "not /forceload's");
            assert_eq!(
                collect_chunks(|visit| level.visit_forced_chunks(game, visit)),
                Ok(vec![])
            );
            // Two owners in one chunk release separately.
            assert!(game.chunk_set_mod_forced(chunk, quarry(), a, false, false));
            assert!(game.server.streaming.forced.contains(&(5, 6)));
            assert_eq!(
                collect_mod_tickets(game, level, quarry()),
                Ok(vec![(b, [5, 6], false)])
            );
            // Calling through a different mod's callback with the same controller works;
            // another controller with the same owner/chunk cannot read or remove that ticket.
            let guard = in_mod("other");
            assert_eq!(
                collect_mod_tickets(game, level, quarry()),
                Ok(vec![(b, [5, 6], false)])
            );
            assert!(game.chunk_set_mod_forced(chunk, quarry(), a, true, false));
            assert_eq!(
                collect_mod_tickets(game, level, quarry()),
                Ok(vec![(a, [5, 6], false), (b, [5, 6], false)])
            );
            assert!(game.chunk_set_mod_forced(chunk, quarry(), a, false, false));
            assert_eq!(
                collect_mod_tickets(game, level, quarry()),
                Ok(vec![(b, [5, 6], false)])
            );
            let vault = controller("other:vault");
            assert_eq!(collect_mod_tickets(game, level, vault), Ok(vec![]));
            assert!(!game.chunk_set_mod_forced(chunk, vault, b, false, false));
            assert!(game.chunk_set_mod_forced(chunk, vault, b, true, false));
            assert!(game.chunk_set_mod_forced(chunk, vault, b, false, false));
            assert_eq!(
                collect_mod_tickets(game, level, quarry()),
                Ok(vec![(b, [5, 6], false)])
            );
            drop(guard);
            // Another controller of the same mod is apart from the first: it neither lists nor
            // releases its tickets.
            let drill = controller("machines:drill");
            assert_eq!(collect_mod_tickets(game, level, drill), Ok(vec![]));
            assert!(!game.chunk_set_mod_forced(chunk, drill, b, false, false));
            assert!(game.chunk_set_mod_forced(chunk, drill, b, true, false));
            assert!(game.chunk_set_mod_forced(chunk, drill, b, false, false));
            assert_eq!(
                collect_mod_tickets(game, level, quarry()),
                Ok(vec![(b, [5, 6], false)])
            );
            // An invalid registered-slot sentinel (returned on refused registration) refuses
            // without mutating existing or new tickets.
            let invalid = mod_api::__private::parts_installation::refused_ticket_controller();
            let untouched = game.level_chunk_ref(level, [6, 6]).unwrap();
            assert_eq!(
                level.visit_mod_forced_chunks(game, invalid, &mut |_, _, _| panic!(
                    "refused ticket visit"
                )),
                Err(LevelReadError::Unavailable)
            );
            assert!(!game.chunk_set_mod_forced(chunk, invalid, b, false, false));
            assert!(!game.chunk_set_mod_forced(untouched, invalid, a, true, false));
            // A handle from an earlier registration table with the same name and slot cannot
            // read, add, or release the active controller's tickets.
            assert_eq!(
                level.visit_mod_forced_chunks(game, foreign_quarry, &mut |_, _, _| panic!(
                    "refused ticket visit"
                )),
                Err(LevelReadError::Unavailable)
            );
            assert!(!game.chunk_set_mod_forced(chunk, foreign_quarry, b, false, false));
            assert!(!game.chunk_set_mod_forced(untouched, foreign_quarry, a, true, false));
            assert!(!game.server.streaming.forced.contains(&(6, 6)));
            assert_eq!(
                game.server
                    .streaming
                    .mod_tickets()
                    .of_controller(u32::MAX)
                    .count(),
                0
            );
            assert_eq!(
                collect_mod_tickets(game, level, quarry()),
                Ok(vec![(b, [5, 6], false)])
            );
            // /forceload and a ticket on one chunk: the chunk stays until both go.
            assert!(game.chunk_set_forced(chunk, true));
            assert!(game.chunk_set_forced(chunk, false));
            assert!(
                game.server.streaming.forced.contains(&(5, 6)),
                "the ticket holds it"
            );
            assert!(game.chunk_set_forced(chunk, true));
            assert!(game.chunk_set_mod_forced(chunk, quarry(), b, false, false));
            assert!(
                game.server.streaming.forced.contains(&(5, 6)),
                "/forceload holds it"
            );
            game.server.streaming.clear_forced();
            assert!(game.server.streaming.forced.is_empty());
            // The world's bounds, as /forceload checks them.
            let outside = ChunkRef::new(level, [1_875_000, 0]);
            assert!(!game.chunk_set_mod_forced(outside, quarry(), a, true, false));
            // Stale handles refuse without mutating.
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            let stale_chunk = ChunkRef::new(stale, [5, 6]);
            assert_eq!(
                stale.visit_mod_forced_chunks(game, quarry(), &mut |_, _, _| panic!(
                    "stale level visited"
                )),
                Err(LevelReadError::Stale)
            );
            assert!(!game.chunk_set_mod_forced(stale_chunk, quarry(), a, true, false));
            assert!(!game.server.streaming.forced.contains(&(5, 6)));
            // Client side refuses without mutating.
            game.options.execution_role =
                sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
            assert_eq!(
                level.visit_mod_forced_chunks(game, quarry(), &mut |_, _, _| panic!(
                    "refused ticket visit"
                )),
                Err(LevelReadError::Unavailable)
            );
            assert!(!game.chunk_set_mod_forced(chunk, quarry(), a, true, false));
            assert!(!game.server.streaming.forced.contains(&(5, 6)));
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn mod_tickets_save_apart_from_vanillas_tickets_and_release_with_the_world() {
        use mod_api::TicketOwner::{Block, Entity};
        use std::sync::Arc;
        use sunrise_level::chunk_streamer::ChunkStreamer;
        use sunrise_level::testgen::TestTerrain;
        with_machines(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let chunk = game.level_chunk_ref(level, [-9, 4]).unwrap();
            let drill = controller("machines:drill");
            let (block, entity) = (Block([-140, 70, 70]), Entity(u128::MAX - 3));
            assert!(game.chunk_set_mod_forced(chunk, quarry(), block, true, false));
            assert!(game.chunk_set_mod_forced(chunk, quarry(), entity, true, false));
            assert!(game.chunk_set_mod_forced(chunk, drill, block, true, false));
            game.save_chunk_tickets().unwrap();
            let vanilla = game.server.save.0.load_forced_chunks();
            assert!(vanilla.is_empty(), "vanilla's file has no mod ticket");
            // A new level starts with no live tickets; only the saved tickets carry over.
            let reopen = |game: &mut sunrise_gameplay::game::ServerGame<'_>,
                          slot_of: fn(&str) -> Option<u32>| {
                let terrain =
                    TestTerrain::new(game.pack.assets.registry, game.res.biomes()).unwrap();
                game.server.streaming = ChunkStreamer::new(
                    Arc::new(terrain),
                    game.pack.chunk_context_with(game.res.biomes().clone()),
                    1,
                );
                game.server.streaming.radius = 0;
                game.server.streaming.load_forced(
                    game.server.save.0.load_forced_chunks(),
                    game.server.save.0.load_mod_tickets(slot_of),
                );
                game.validate_mod_chunk_tickets(game.dim);
            };
            reopen(game, known_slot);
            // Each ticket comes back under the controller that held it.
            let listed = |game: &sunrise_gameplay::game::ServerGame<'_>, c| {
                collect_mod_tickets(game, level, c).unwrap()
            };
            assert_eq!(listed(game, quarry()).len(), 2);
            assert_eq!(listed(game, drill), vec![(block, [-9, 4], false)]);
            assert!(game.server.streaming.forced.contains(&(-9, 4)));
            // No loaded mod registers the controller: its tickets are discarded, nothing
            // is forced, and the next save drops them from the file.
            reopen(game, |_| None);
            assert!(game.server.streaming.forced.is_empty());
            game.save_chunk_tickets().unwrap();
            reopen(game, known_slot);
            assert!(game.server.streaming.forced.is_empty());
            assert!(game.server.save.0.load_mod_tickets(known_slot).is_empty());
            // Released tickets leave the file.
            assert!(game.chunk_set_mod_forced(chunk, quarry(), Block([-140, 70, 70]), true, false));
            assert!(game.chunk_set_mod_forced(
                chunk,
                quarry(),
                Block([-140, 70, 70]),
                false,
                false
            ));
            game.save_chunk_tickets().unwrap();
            assert!(game.server.save.0.load_mod_tickets(known_slot).is_empty());
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_mod_ticket_keeps_a_parked_level_active() {
        with_machines(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let other = if game.dim == Dim::Nether {
                Dim::End
            } else {
                Dim::Nether
            };
            let back = game.dim;
            game.enter_level(other).unwrap();
            game.enter_level(back).unwrap();
            let parked = mod_api::LevelRef::new(level.epoch(), other.key());
            let chunk = ChunkRef::new(parked, [2, 2]);
            let owner = mod_api::TicketOwner::Block([32, 64, 32]);
            assert!(!game.levels_to_tick().contains(&other));
            assert!(game.chunk_set_mod_forced(chunk, quarry(), owner, true, false));
            assert!(game.levels_to_tick().contains(&other));
            assert!(game.chunk_set_mod_forced(chunk, quarry(), owner, false, false));
            assert!(!game.levels_to_tick().contains(&other));
        });
    }

    /// The result `/forceload query` gives: the chunks listed, or 0 when it reports none.
    #[cfg(feature = "client")]
    fn query_result(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        command: &str,
    ) -> Option<i32> {
        let d = game.ui.dispatcher.0.clone();
        let parse = d.parse(command, &game.ui.registries);
        let (ctx, _) = parse.executable().ok()?;
        let (ctx, root) = (ctx.clone(), parse.context.clone());
        game.run_chain(&root, &ctx).ok()
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn forceload_add_returns_with_each_new_column_loaded() {
        with_chunk_streaming(|game| {
            game.ui.registries.permission = 4;
            assert!(!game.world.chunks.contains_key(&(8, -4)));
            assert!(!game.world.chunks.contains_key(&(9, -4)));
            assert_eq!(query_result(game, "forceload add 128 -64 144 -64"), Some(2));
            for key in [(8, -4), (9, -4)] {
                assert!(
                    game.world.chunks.contains_key(&key),
                    "loaded {key:?} before command return"
                );
                assert!(game.server.streaming.commanded().contains(&key));
            }
            assert_eq!(query_result(game, "forceload add 128 -64 144 -64"), None);
            assert_eq!(
                query_result(game, "forceload remove 128 -64 144 -64"),
                Some(2)
            );
        });
    }

    /// Hears `chunk_load`, noting whether the chunk was resident when it heard it.
    #[cfg(feature = "client")]
    struct Loads;
    #[cfg(feature = "client")]
    #[mod_api::__subscriptions]
    impl Hooks for Loads {
        fn chunk_load(&self, host: &mut dyn ServerHost, event: &mut mod_api::ChunkLoadEvent<'_>) {
            let resident = host
                .level(event.dimension)
                .and_then(|level| host.chunk(level, event.chunk))
                .is_some();
            HEARD.with_borrow_mut(|heard| {
                heard.push((event.dimension.to_owned(), event.chunk, event.new, resident))
            });
        }
    }
    thread_local! {
        /// Each `chunk_load` `Loads` heard: dimension, chunk, new, resident.
        static HEARD: std::cell::RefCell<Vec<(String, [i32; 2], bool, bool)>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    /// `Level.getChunk(x, z)` through `LevelRef::load_chunk` (#4550): a chunk never generated is
    /// FULL when the call returns, lit (its open sky reads 15), with nothing forced, and the
    /// mods heard `chunk_load` for it while it was resident, before the call returned. Its
    /// `UNKNOWN` ticket keeps it through the next tick's unload sweep and expires at the one
    /// after (a timeout of 1, removed below zero), whose sweep unloads it as any chunk with no
    /// ticket. A parked level loads the same way, without switching levels.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn load_chunk_brings_a_lit_full_chunk_under_a_ticket_that_expires_after_a_tick() {
        // The game fixture publishes process-wide worldgen tables, and `Loads` is installed
        // as the process's mods: a child process of its own.
        if run_fresh_cases(
            concat!(
                "cut_layout::game::server_side::native_level_refs::tests::",
                "load_chunk_brings_a_lit_full_chunk_under_a_ticket_that_expires_after_a_tick"
            ),
            "SUNRISE_LOAD_CHUNK_CHILD",
            &["load_chunk"],
        ) {
            return;
        }
        let mods: mod_api::Mods = vec![ModEntry::new("loads", Box::new(Loads))].into();
        mod_platform::testing::with_mods(mods, || {
            with_chunk_streaming(|game| {
                let level = game.level(game.dim.id()).unwrap();
                let far = ChunkRef::new(level, [40, -7]);
                assert!(!game.chunk_loaded(far));
                let host: &mut dyn ServerHost = game;
                assert_eq!(level.load_chunk(host, [40, -7]), Ok(()));
                // Lit when the call returns: open sky at the top of the world, none in the
                // deepslate under it.
                let sky = |host: &dyn ServerHost, y| {
                    level.brightness(host, LightLayer::Sky, [645, y, -105])
                };
                assert_eq!(sky(&*host, 318), Some(15), "open sky");
                assert_eq!(sky(&*host, -60), Some(0), "no sky light underground");
                let heard = HEARD.with_borrow_mut(std::mem::take);
                assert!(
                    heard.contains(&(level.dimension_id().to_owned(), [40, -7], true, true)),
                    "chunk_load heard while resident, before the call returned: {heard:?}"
                );
                assert_eq!(game.chunk_forced(far), Some(false));
                assert!(!game.server.streaming.forced.contains(&(40, -7)));
                let host: &mut dyn ServerHost = game;
                assert_eq!(level.load_chunk(host, [40, -7]), Ok(()), "already full");
                // A changed loaded area sweeps at once: the next tick's sweep keeps the chunk.
                game.server.streaming.radius = 1;
                game.tick_server(false).unwrap();
                assert!(
                    game.chunk_loaded(far),
                    "the ticket holds through the next tick"
                );
                game.server.streaming.radius = 0;
                game.tick_server(false).unwrap();
                assert!(!game.chunk_loaded(far), "unloaded once the ticket expired");
                // The first chunks past `ChunkPos.isValid`, on x and on z.
                let host: &mut dyn ServerHost = game;
                assert_eq!(
                    level.load_chunk(&mut *host, [2_097_062, 0]),
                    Err(ChunkLoadError::Unloaded)
                );
                assert_eq!(
                    level.load_chunk(&mut *host, [0, -2_097_062]),
                    Err(ChunkLoadError::Unloaded)
                );
                // A level the server has not opened, then the same level opened and parked.
                let other = if game.dim == Dim::Nether {
                    Dim::End
                } else {
                    Dim::Nether
                };
                let parked = mod_api::LevelRef::new(level.epoch(), other.key());
                let host: &mut dyn ServerHost = game;
                assert_eq!(
                    parked.load_chunk(host, [3, 3]),
                    Err(ChunkLoadError::Unloaded)
                );
                let back = game.dim;
                game.enter_level(other).unwrap();
                game.enter_level(back).unwrap();
                HEARD.with_borrow_mut(Vec::clear);
                let host: &mut dyn ServerHost = game;
                assert_eq!(parked.load_chunk(host, [3, 3]), Ok(()));
                assert_eq!(game.dim, back, "loading must not switch the acting level");
                let parked_chunk = &game
                    .server
                    .levels
                    .0
                    .parked_world_of(other.id())
                    .unwrap()
                    .chunks[&(3, 3)];
                assert!(parked_chunk.lit, "the parked level's chunk is lit too");
                let heard = HEARD.with_borrow_mut(std::mem::take);
                assert!(
                    heard.iter().any(|(d, chunk, _, resident)| d == other.id()
                        && *chunk == [3, 3]
                        && *resident),
                    "the parked level's chunk_load came before the call returned: {heard:?}"
                );
            });
        });
    }

    /// `ServerChunkCache.getChunkNow` (#4670): after the first purge, a resident read of the FULL
    /// chunk makes it one of the level's recent FULL lookups, so a `load_chunk` that follows in
    /// the same tick answers from them and leaves the `UNKNOWN` ticket as the first load placed
    /// it: it expires at the second purge, whose sweep unloads the chunk. Without the read's entry
    /// the second load would set the ticket again and the chunk would stay.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_resident_read_lets_load_chunk_answer_without_a_new_ticket() {
        // The game fixture publishes process-wide worldgen tables: a child process of its own.
        if run_fresh_cases(
            concat!(
                "cut_layout::game::server_side::native_level_refs::tests::",
                "a_resident_read_lets_load_chunk_answer_without_a_new_ticket"
            ),
            "SUNRISE_LOAD_CHUNK_READ_CHILD",
            &["resident_read"],
        ) {
            return;
        }
        mod_platform::testing::with_mods(Vec::new().into(), || {
            with_chunk_streaming(|game| {
                let level = game.level(game.dim.id()).unwrap();
                let host: &mut dyn ServerHost = game;
                assert_eq!(level.load_chunk(host, [40, -7]), Ok(()));
                // A changed loaded area sweeps at once: the ticket, at 0 ticks left after this
                // tick's purge, keeps the chunk, and the tick's end forgets the lookups.
                game.server.streaming.radius = 1;
                game.tick_server(false).unwrap();
                assert!(
                    game.world.chunks.contains_key(&(40, -7)),
                    "kept by the ticket"
                );
                assert!(game.chunk(level, [40, -7]).is_some(), "the resident read");
                let host: &mut dyn ServerHost = game;
                assert_eq!(level.load_chunk(host, [40, -7]), Ok(()));
                game.server.streaming.radius = 0;
                game.tick_server(false).unwrap();
                assert!(
                    !game.world.chunks.contains_key(&(40, -7)),
                    "the first load's ticket expired at the second purge"
                );
            });
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn forceload_query_lists_only_command_tickets() {
        with_machines(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let chunk = game.level_chunk_ref(level, [3, 3]).unwrap();
            game.ui.registries.permission = 4;
            let owner = mod_api::TicketOwner::Block([48, 64, 48]);
            assert!(game.chunk_set_mod_forced(chunk, quarry(), owner, true, false));
            assert!(game.server.streaming.forced.contains(&(3, 3)));
            assert_eq!(query_result(game, "forceload query"), Some(0), "list form");
            assert_eq!(
                query_result(game, "forceload query 48 48"),
                None,
                "position form"
            );
            assert!(game.chunk_set_forced(chunk, true));
            assert_eq!(query_result(game, "forceload query"), Some(1));
            assert_eq!(query_result(game, "forceload query 48 48"), Some(1));
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn natural_spawning_tickets_and_plain_ones_save_and_load_apart() {
        use mod_api::TicketOwner::Block;
        with_machines(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let plain = game.level_chunk_ref(level, [11, 12]).unwrap();
            let spawning = game.level_chunk_ref(level, [13, 12]).unwrap();
            let owner = Block([176, 64, 192]);
            assert!(game.chunk_set_mod_forced(plain, quarry(), owner, true, false));
            assert!(game.chunk_set_mod_forced(spawning, quarry(), owner, true, true));
            let tickets = game.server.streaming.mod_tickets();
            assert!(!tickets.forces_natural_spawning((11, 12)));
            assert!(tickets.forces_natural_spawning((13, 12)));
            // Both kinds load and tick: they are forced chunks.
            assert!(game.server.streaming.forced.contains(&(11, 12)));
            assert!(game.server.streaming.forced.contains(&(13, 12)));
            assert!(game.levels_to_tick().contains(&game.dim));
            assert_eq!(
                collect_mod_tickets(game, level, quarry()),
                Ok(vec![(owner, [11, 12], false), (owner, [13, 12], true)])
            );
            let mut first = Vec::new();
            assert_eq!(
                level.visit_mod_forced_chunks(game, quarry(), &mut |owner, pos, natural| {
                    first.push((owner, pos, natural));
                    ControlFlow::Break(())
                }),
                Ok(ControlFlow::Break(()))
            );
            assert_eq!(first, [(owner, [11, 12], false)]);
            // One owner holds both kinds on one chunk apart.
            assert!(game.chunk_set_mod_forced(plain, quarry(), owner, true, true));
            assert!(game.chunk_set_mod_forced(plain, quarry(), owner, false, true));
            assert!(game.server.streaming.forced.contains(&(11, 12)));
            assert!(
                !game
                    .server
                    .streaming
                    .mod_tickets()
                    .forces_natural_spawning((11, 12))
            );
            game.save_chunk_tickets().unwrap();
            let saved = game.server.save.0.load_mod_tickets(known_slot);
            game.server.streaming.load_forced(Default::default(), saved);
            game.validate_mod_chunk_tickets(game.dim);
            let tickets = game.server.streaming.mod_tickets();
            assert!(!tickets.forces_natural_spawning((11, 12)));
            assert!(tickets.forces_natural_spawning((13, 12)));
            assert!(tickets.holds((11, 12)));
        });
    }

    /// The flag adds a chunk to the spawning chunks (inhabited time, a spawn
    /// attempt); the local mob cap still wants a non-spectator within 128
    /// blocks of its centre.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_natural_spawning_ticket_far_from_every_player_runs_the_attempt_but_the_local_cap_refuses()
    {
        use mod_api::TicketOwner::Block;
        use std::sync::Arc;
        use sunrise_core::geometry::Vec3;
        use sunrise_level::chunk::Chunk;
        use sunrise_sim::mobs::spawn_selection::{Category, SpawnPlayer, SpawnState};
        use sunrise_sim::motion::PlayerMotion;
        with_machines(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            for cx in 6..=10 {
                for cz in 6..=10 {
                    let mut chunk = Chunk::empty(cx, cz);
                    chunk.lit = true;
                    game.world.chunks.insert((cx, cz), Arc::new(chunk));
                }
            }
            let level = game.level(game.dim.id()).unwrap();
            // Both chunks lie in the player's 8-chunk area and past 128 blocks of it.
            let flagged = game.level_chunk_ref(level, [8, 8]).unwrap();
            let plain = game.level_chunk_ref(level, [8, 7]).unwrap();
            let owner = Block([128, 64, 128]);
            assert!(game.chunk_set_mod_forced(flagged, quarry(), owner, true, true));
            assert!(game.chunk_set_mod_forced(plain, quarry(), owner, true, false));
            game.spawn_natural();
            assert_eq!(game.world.inhabited.get(&(8, 8)), Some(&1));
            assert!(
                !game.world.inhabited.contains_key(&(8, 7)),
                "without the flag it is no spawning chunk"
            );
            assert!(game.mobs.is_empty(), "the local cap refuses");
            // The cap itself: no player within 128 blocks of the chunk.
            let players: Vec<SpawnPlayer> = game
                .level_players()
                .iter()
                .map(|p| SpawnPlayer {
                    position: [p.position.x, p.position.y, p.position.z],
                    spectator: p.spectator,
                })
                .collect();
            let natural = game.natural_level();
            let mut state = SpawnState::new(&natural, 1, players, &[], |_, _| true);
            assert!(!state.can_spawn_local(Category::Monster, 8, 8));
            assert!(state.can_spawn_local(Category::Monster, 0, 0));
        });
    }

    #[cfg(feature = "client")]
    fn saved(controller: &str, pos: [i32; 3], chunk: (i32, i32)) -> ModTicket {
        ModTicket {
            controller: controller_slot(controller),
            owner: TicketOwner::Block(pos),
            chunk,
            natural_spawning: false,
        }
    }
    #[cfg(feature = "client")]
    fn controller_slot(id: &str) -> u32 {
        known_slot(id).unwrap()
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn validation_drops_a_removed_ticket_and_leaves_the_rest() {
        with_machines(|game| {
            // All hold the same block; `quarry` and `vault` prune it, `drill` keeps it.
            let tickets = vec![
                saved("machines:quarry", [1, 64, 1], (7, 7)),
                saved("machines:quarry", [9, 64, 9], (8, 8)),
                saved("machines:drill", [1, 64, 1], (9, 9)),
                saved("machines:plain", [1, 64, 1], (10, 10)),
                saved("other:vault", [1, 64, 1], (11, 11)),
            ];
            game.server
                .streaming
                .load_forced(Default::default(), tickets);
            assert!(
                game.server.streaming.forced.is_empty(),
                "loaded, not yet validated"
            );
            game.validate_mod_chunk_tickets(game.dim);
            let forced = &game.server.streaming.forced;
            assert!(!forced.contains(&(7, 7)), "quarry's prune");
            assert!(forced.contains(&(8, 8)));
            assert!(
                forced.contains(&(9, 9)),
                "drill's validation prunes nothing"
            );
            assert!(forced.contains(&(10, 10)), "no validator, no validation");
            assert!(!forced.contains(&(11, 11)), "other's own controller");
            assert_eq!(
                SEEN.take(),
                [
                    ("machines:quarry", vec![[1, 64, 1], [9, 64, 9]]),
                    ("machines:drill", vec![[1, 64, 1]]),
                    ("other:vault", vec![[1, 64, 1]]),
                ],
                "only the registered callbacks for these groups ran, once each in slot order"
            );
            game.save_chunk_tickets().unwrap();
            let file = game.server.save.0.load_mod_tickets(known_slot);
            assert_eq!(file.len(), 3);
            assert!(
                !file
                    .iter()
                    .any(|t| t.chunk == (7, 7) || t.chunk == (11, 11))
            );
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn validation_panic_disables_only_its_controller_for_the_session() {
        use std::collections::HashSet;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use sunrise_level::mod_chunk_tickets::ModTickets;
        use sunrise_sim::save::{SaveDir, dimension_dir};

        const STALE: [i32; 3] = [1, 64, 1];
        const LIVE: [i32; 3] = [9, 64, 9];
        #[derive(Default)]
        struct Calls {
            panicking: AtomicUsize,
            healthy: AtomicUsize,
        }
        struct Paired(Arc<Calls>);
        #[mod_api::__subscriptions]
        impl Hooks for Paired {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                let calls = Arc::clone(&self.0);
                r.register_ticket_controller(
                    "panicking",
                    Some(Arc::new(
                        move |_: &mut dyn ServerHost, _, validation: &mut dyn TicketValidation| {
                            calls.panicking.fetch_add(1, Ordering::Relaxed);
                            validation.remove_block_tickets(STALE);
                            panic!("chunk ticket validator fixture panic");
                        },
                    )),
                );
                let calls = Arc::clone(&self.0);
                r.register_ticket_controller(
                    "healthy",
                    Some(Arc::new(
                        move |_: &mut dyn ServerHost, _, validation: &mut dyn TicketValidation| {
                            calls.healthy.fetch_add(1, Ordering::Relaxed);
                            validation.remove_block_tickets(STALE);
                        },
                    )),
                );
            }
        }

        let calls = Arc::new(Calls::default());
        let world = tempfile::tempdir().unwrap();
        let mods = vec![ModEntry::new(
            "paired",
            Box::new(Paired(Arc::clone(&calls))),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_chunk_streaming(|game| {
                let tickets = [
                    saved("paired:panicking", STALE, (7, 7)),
                    saved("paired:panicking", LIVE, (8, 8)),
                    saved("paired:healthy", STALE, (9, 9)),
                    saved("paired:healthy", LIVE, (10, 10)),
                ];
                game.server
                    .streaming
                    .load_forced(Default::default(), tickets.to_vec());
                let first = game.dim;
                game.validate_mod_chunk_tickets(first);
                assert_eq!(calls.panicking.load(Ordering::Relaxed), 1);
                assert_eq!(calls.healthy.load(Ordering::Relaxed), 1);
                assert_eq!(
                    game.server.streaming.forced,
                    HashSet::from([(7, 7), (8, 8), (10, 10)])
                );

                let second = if first == Dim::Nether {
                    Dim::End
                } else {
                    Dim::Nether
                };
                game.server.world_root = world.path().to_path_buf();
                let save = SaveDir::new(
                    dimension_dir(world.path(), second.id()),
                    second.shape(),
                    Some(Default::default()),
                );
                let mut saved_rows = ModTickets::default();
                for ticket in tickets {
                    saved_rows.insert(ticket);
                }
                save.save_mod_tickets(&saved_rows, mod_api::__private::game::ticket_controller_id)
                    .unwrap();
                game.enter_level(second).unwrap();
                assert_eq!(calls.panicking.load(Ordering::Relaxed), 1);
                assert_eq!(calls.healthy.load(Ordering::Relaxed), 2);
                assert_eq!(
                    game.server.streaming.forced,
                    HashSet::from([(7, 7), (8, 8), (10, 10)])
                );
            });
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn validation_runs_when_a_parked_level_opens_and_not_without_a_listener() {
        mod_platform::testing::with_mods(mods(), || {
            with_game(|game| {
                let other = if game.dim == Dim::Nether {
                    Dim::End
                } else {
                    Dim::Nether
                };
                let world =
                    std::env::temp_dir().join(format!("sunrise-validate-{}", std::process::id()));
                game.server.world_root = world.clone();
                let root = sunrise_sim::save::dimension_dir(&world, other.id());
                let save =
                    sunrise_sim::save::SaveDir::new(root, other.shape(), Some(Default::default()));
                let mut tickets = sunrise_level::mod_chunk_tickets::ModTickets::default();
                tickets.insert(saved("machines:quarry", [1, 64, 1], (7, 7)));
                tickets.insert(saved("machines:quarry", [9, 64, 9], (8, 8)));
                save.save_mod_tickets(&tickets, mod_api::__private::game::ticket_controller_id)
                    .unwrap();
                let back = game.dim;
                game.enter_level(other).unwrap();
                game.enter_level(back).unwrap();
                let parked = game
                    .server
                    .levels
                    .0
                    .parked_streaming_of(other.id())
                    .unwrap();
                assert!(!parked.forced.contains(&(7, 7)));
                assert!(parked.forced.contains(&(8, 8)));
                assert!(game.levels_to_tick().contains(&other));
                let _ = std::fs::remove_dir_all(&world);
            });
        });
        // No validator is registered: the tickets stay as loaded.
        struct Idle;
        #[mod_api::__subscriptions]
        impl Hooks for Idle {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.register_ticket_controller("quarry", None);
            }
        }
        let mods: mod_api::Mods = vec![ModEntry::new("idle", Box::new(Idle))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let tickets = vec![saved("idle:quarry", [1, 64, 1], (7, 7))];
                game.server
                    .streaming
                    .load_forced(Default::default(), tickets);
                game.validate_mod_chunk_tickets(game.dim);
                assert!(game.server.streaming.forced.contains(&(7, 7)));
            });
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn validation_sees_the_saved_tickets_not_ones_added_after_the_load() {
        with_machines(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let owner = mod_api::TicketOwner::Block([1, 64, 1]);
            let live = game.level_chunk_ref(level, [5, 5]).unwrap();
            assert!(game.chunk_set_mod_forced(live, quarry(), owner, true, false));
            let tickets = vec![saved("machines:quarry", [1, 64, 1], (7, 7))];
            game.server
                .streaming
                .load_forced(Default::default(), tickets);
            // `level_load` runs after the load: its hold is the same owner's.
            let fresh = game.level_chunk_ref(level, [6, 6]).unwrap();
            assert!(game.chunk_set_mod_forced(fresh, quarry(), owner, true, false));
            game.validate_mod_chunk_tickets(game.dim);
            assert!(
                !game.server.streaming.forced.contains(&(7, 7)),
                "saved one pruned"
            );
            assert!(
                game.server.streaming.forced.contains(&(5, 5)),
                "live one kept"
            );
            assert!(
                game.server.streaming.forced.contains(&(6, 6)),
                "fresh one kept"
            );
        });
    }

    #[cfg(feature = "client")]
    fn with_night_client_game(
        test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>),
    ) {
        use std::sync::Arc;
        use sunrise_protocol::{
            connection::Pipe,
            packets::play::{self, Clientbound},
        };

        with_client_game(|game| {
            let (pipe, _server) = Pipe::pair();
            game.connect_local(pipe, "SkyDarkenReadTest").unwrap();
            game.client.remote.as_mut().unwrap().logged_in = true;
            let mut registries = (*game.client.remote.as_ref().unwrap().registries).clone();
            registries.registries.insert(
                "minecraft:world_clock".into(),
                vec![("minecraft:overworld".into(), None)],
            );
            game.set_registries(Arc::new(registries)).unwrap();
            game.handle_packet(Clientbound::SetTime(play::SetTime {
                game_time: 0,
                clocks: vec![play::ClockState {
                    clock: 0,
                    total_ticks: 18000,
                    partial_tick: 0.0,
                    rate: 1.0,
                }],
            }))
            .unwrap();
            test(game);
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn light_and_loaded_reads_match_the_games_query_and_load_nothing() {
        use mod_api::LevelHost;
        use std::sync::Arc;
        use sunrise_level::chunk::{Chunk, Light};

        with_game(|game| {
            let mut chunk = Chunk::empty(0, 0);
            // Block light 5 and sky light 12 in the lowest section.
            Arc::make_mut(&mut chunk.sections[0]).light = Light::Uniform(0xC5);
            game.world.chunks.insert((0, 0), Arc::new(chunk));
            let level = game.level(game.dim.id()).unwrap();
            let lit = [3, -60, 5];
            let sky = game.world.light(3, -60, 5) >> 4;
            let block = game.world.light(3, -60, 5) & 15;
            assert_eq!((sky, block), (12, 5));
            let chunks = game.world.chunks.len();
            // Night: the sky is darkened, so the read is not just the daytime 0.
            game.world.day_time = 18000;
            let darken = game.sky_darken();
            assert!(darken > 0, "night darkens the sky");

            game.with_level(|_, host, _| {
                assert_eq!(host.brightness(LightLayer::Block, lit), 5);
                assert_eq!(host.brightness(LightLayer::Sky, lit), 12);
                assert_eq!(host.raw_brightness(lit, 0), 12);
                assert_eq!(host.raw_brightness(lit, 9), 5);
                // `Level.getSkyDarken` as the game's own light reads take it.
                assert_eq!(LevelHost::sky_darken(&*host), Some(darken));
                let pos = BlockPos::new(lit[0], lit[1], lit[2]);
                assert_eq!(
                    host.raw_brightness(lit, darken),
                    sunrise_world::level::raw_brightness(&*host, pos, darken)
                );
                assert!(host.is_loaded([0, 319, 0]));
                assert!(!host.is_loaded([0, 320, 0]));
                assert!(!host.is_loaded([0, -65, 0]));
                assert!(!host.is_loaded([16, -60, 0]));
            });
            assert_eq!(
                game.level_brightness(level, LightLayer::Block, lit),
                Some(5)
            );
            assert_eq!(game.level_brightness(level, LightLayer::Sky, lit), Some(12));
            assert_eq!(
                game.level_brightness(level, LightLayer::Sky, [16, -60, 0]),
                None
            );
            assert_eq!(
                game.level_brightness(level, LightLayer::Sky, [0, 320, 0]),
                None
            );
            // The level view a block part's callback gets (`ReadLevel`), on the server.
            let view = sunrise_gameplay_common::game::native_level_signals::ReadLevel::new(
                game,
                level,
                &game.world,
            );
            assert_eq!(LevelHost::sky_darken(&view), Some(darken));
            let stale = LevelRef::new(WorldEpoch::new(u64::MAX), game.dim.key());
            assert_eq!(game.level_brightness(stale, LightLayer::Sky, lit), None);
            assert_eq!(game.world.chunks.len(), chunks, "nothing was loaded");
        });
        // A logged-in client reads the same darkening through the same view.
        with_night_client_game(|game| {
            let client = game.level("minecraft:overworld").unwrap();
            // The client's own world, at night as the server's time packet would set it.
            game.world.day_time = 18000;
            let darken = game.sky_darken();
            assert!(darken > 0, "night darkens the client's sky");
            let view = sunrise_gameplay_common::game::native_level_signals::ReadLevel::new(
                game,
                client,
                &game.world,
            );
            assert_eq!(LevelHost::sky_darken(&view), Some(darken));
        });
    }

    /// `check_light` queues the cell with the level's light edits, its state set to itself,
    /// where the next update asks a `dynamic_light` block again and relights it; a stale
    /// handle and a position that isn't loaded queue nothing.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn check_light_queues_the_cell_with_the_light_edits() {
        use std::sync::Arc;
        use sunrise_level::chunk::Chunk;

        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.world.light_edits.clear();
            let level = game.level(game.dim.id()).unwrap();
            let state = game.world.block(3, 64, 5).0;
            assert!(game.level_check_light(level, [3, 64, 5]));
            assert_eq!(game.world.light_edits, [((3, 64, 5), state, state)]);
            let stale = LevelRef::new(WorldEpoch::new(u64::MAX), game.dim.key());
            assert!(!game.level_check_light(stale, [3, 64, 5]));
            assert!(!game.level_check_light(level, [16, 64, 0]), "not loaded");
            assert!(
                !game.level_check_light(level, [3, 320, 5]),
                "above the world"
            );
            assert_eq!(game.world.light_edits.len(), 1);
        });
    }
}

#[cfg(test)]
pub(crate) mod environment_read_tests {
    #[cfg(feature = "client")]
    use mod_api::{BiomeSearch, LevelRef, WorldEpoch};
    #[cfg(feature = "client")]
    use std::sync::Arc;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::weather::WeatherGameExt;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::world_biomes;
    #[cfg(feature = "client")]
    use sunrise_level::chunk::Chunk;
    #[cfg(feature = "client")]
    use {
        crate::game::server_side::native_entity_refs::tests::with_client_game,
        sunrise_gameplay::game::native_entity_refs::tests::with_game,
        sunrise_gameplay_common::game::Dim,
    };

    #[cfg(feature = "client")]
    fn paint_biome(game: &mut sunrise_gameplay_common::game::CommonGame<'_>, name: &str) {
        let biome = world_biomes!(game).id(name);
        let mut chunk = Chunk::empty(0, 0);
        for y in 0..chunk.sections.len() as i32 * 4 {
            for x in 0..4 {
                for z in 0..4 {
                    chunk.set_biome_cell(x, y, z, biome);
                }
            }
        }
        game.world.chunks.insert((0, 0), Arc::new(chunk));
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn biome_reads_follow_live_cells_parked_levels_and_client_login_without_loading() {
        run_biome_reads_follow_live_cells_parked_levels_and_client_login_without_loading();
    }

    #[cfg(feature = "client")]
    pub(crate) fn run_biome_reads_follow_live_cells_parked_levels_and_client_login_without_loading()
    {
        use sunrise_protocol::connection::Pipe;

        with_game(|game| {
            let overworld = game.level("minecraft:overworld").unwrap();
            paint_biome(game, "minecraft:plains");
            let before = (game.world.chunks.len(), game.server.streaming.pending());
            assert_eq!(
                overworld.biome_at(game, [8, 64, 8]).as_deref(),
                Some("minecraft:plains")
            );
            assert_eq!(overworld.biome_at(game, [31, 64, 31]), None);
            assert_eq!(
                before,
                (game.world.chunks.len(), game.server.streaming.pending())
            );
            paint_biome(game, "minecraft:desert");
            assert_eq!(
                overworld.biome_at(game, [8, 64, 8]).as_deref(),
                Some("minecraft:desert")
            );
            game.enter_level(Dim::Nether).unwrap();
            assert_eq!(
                overworld.biome_at(game, [8, 64, 8]).as_deref(),
                Some("minecraft:desert"),
                "parked level's cells"
            );
            game.enter_level(Dim::Overworld).unwrap();
            let stale = LevelRef::new(
                WorldEpoch::new(overworld.epoch().get() + 1),
                overworld.dimension(),
            );
            assert_eq!(stale.biome_at(game, [8, 64, 8]), None);
        });
        with_client_game(|game| {
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "BiomeReadTest").unwrap();
            let overworld = LevelRef::new(
                game.native_lifecycle.epoch().unwrap(),
                mod_api::DimensionKey::OVERWORLD,
            );
            assert_eq!(
                overworld.client_biome(game, [8, 64, 8]),
                None,
                "not logged in"
            );
            game.client.remote.as_mut().unwrap().logged_in = true;
            paint_biome(game, "minecraft:snowy_plains");
            let client_level = game.level("minecraft:overworld").unwrap();
            let before = (game.world.chunks.len(), game.client.streaming.pending());
            assert_eq!(
                client_level.client_biome(game, [8, 64, 8]).as_deref(),
                Some("minecraft:snowy_plains")
            );
            assert_eq!(client_level.client_biome(game, [31, 64, 31]), None);
            let foreign = LevelRef::new(client_level.epoch(), mod_api::DimensionKey::NETHER);
            assert_eq!(foreign.client_biome(game, [8, 64, 8]), None);
            assert_eq!(
                before,
                (game.world.chunks.len(), game.client.streaming.pending())
            );
            game.client.remote.as_mut().unwrap().logged_in = false;
            assert_eq!(
                client_level.client_biome(game, [8, 64, 8]),
                None,
                "logged out"
            );
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn floor_reads_real_collision_shapes_and_refuses_unloaded_positions() {
        run_floor_reads_real_collision_shapes_and_refuses_unloaded_positions();
    }

    #[cfg(feature = "client")]
    pub(crate) fn run_floor_reads_real_collision_shapes_and_refuses_unloaded_positions() {
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let slab = registry.default_state("minecraft:oak_slab").unwrap();
            let fence = registry.default_state("minecraft:oak_fence").unwrap();
            for (x, block, height, above) in [
                (2, stone, 1.0, 0.0),
                (5, slab, 0.5, f64::NEG_INFINITY),
                (8, fence, 1.5, 0.5),
            ] {
                game.world.set(x, 100, 8, block).unwrap();
                assert_eq!(level.block_floor_height(game, [x, 100, 8]), Some(height));
                assert_eq!(level.block_floor_height(game, [x, 101, 8]), Some(above));
            }
            assert_eq!(
                level.block_floor_height(game, [12, 100, 8]),
                Some(f64::NEG_INFINITY)
            );
            let before = (game.world.chunks.len(), game.server.streaming.pending());
            assert_eq!(level.block_floor_height(game, [32, 100, 8]), None);
            assert_eq!(
                level.block_floor_height(game, [8, game.world.top_y(), 8]),
                None
            );
            assert_eq!(
                before,
                (game.world.chunks.len(), game.server.streaming.pending())
            );
            game.enter_level(Dim::Nether).unwrap();
            assert_eq!(
                level.block_floor_height(game, [5, 100, 8]),
                Some(0.5),
                "parked collision shapes"
            );
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            assert_eq!(stale.block_floor_height(game, [5, 100, 8]), None);
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn sky_and_logical_height_read_the_named_dimension_and_client_state() {
        run_sky_and_logical_height_read_the_named_dimension_and_client_state();
    }

    #[cfg(feature = "client")]
    pub(crate) fn run_sky_and_logical_height_read_the_named_dimension_and_client_state() {
        use sunrise_level::chunk::LevelShape;
        use sunrise_protocol::connection::Pipe;

        with_game(|game| {
            let overworld = game.level("minecraft:overworld").unwrap();
            assert_eq!(overworld.logical_height(game), Some(384));
            for time in [0, 6000, 12000, 18000, 24000] {
                game.world.day_time = time;
                assert_eq!(overworld.get_sky_darken(game), Some(game.sky_darken()));
            }
            game.world.day_time = 6000;
            game.world.weather.rain_level = (0.0, 0.0);
            game.world.weather.thunder_level = (0.0, 0.0);
            game.enter_level(Dim::Nether).unwrap();
            let nether = game.level("minecraft:the_nether").unwrap();
            assert_eq!(nether.logical_height(game), Some(128));
            assert_eq!(
                nether.height(game),
                Some((0, 256)),
                "logical and build heights differ"
            );
            assert_eq!(nether.get_sky_darken(game), Some(11));
            // The vanilla fixture records clear midnight as 11 and half-rain noon as 1.
            // Change the shared state while the Overworld snapshot stays parked at noon.
            game.world.day_time = 18000;
            assert_eq!(overworld.day_time(game), Some(18000));
            assert_eq!(overworld.get_sky_darken(game), Some(11));
            game.world.day_time = 6000;
            game.world.weather.rain_level = (0.5, 0.5);
            assert_eq!(overworld.get_sky_darken(game), Some(1));
            assert_eq!(nether.get_sky_darken(game), Some(11));
            assert_eq!(game.dim, Dim::Nether, "a read does not swap levels");
            let parked = game
                .server
                .levels
                .0
                .parked_world_of("minecraft:overworld")
                .unwrap();
            assert_eq!(
                parked.day_time, 6000,
                "the parked snapshot is not rewritten"
            );
            assert_eq!(parked.weather.levels(1.0), (0.0, 0.0));
            game.enter_level(Dim::Overworld).unwrap();
            game.world.day_time = 6000;
            assert_eq!(nether.logical_height(game), Some(128));
            assert_eq!(
                nether.get_sky_darken(game),
                Some(11),
                "parked level keeps its dimension's sky"
            );
            let stale = LevelRef::new(
                WorldEpoch::new(overworld.epoch().get() + 1),
                overworld.dimension(),
            );
            assert_eq!(stale.logical_height(game), None);
            assert_eq!(stale.get_sky_darken(game), None);
        });
        with_client_game(|game| {
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "SkyReadTest").unwrap();
            let epoch = game.native_lifecycle.epoch().unwrap();
            let overworld = LevelRef::new(epoch, mod_api::DimensionKey::OVERWORLD);
            let nether = LevelRef::new(epoch, mod_api::DimensionKey::NETHER);
            assert_eq!(overworld.logical_height(game), None, "not logged in");
            game.client.remote.as_mut().unwrap().logged_in = true;
            game.world.shape = LevelShape::new(-128, 512, 256, true, false).unwrap();
            let level = game.level("minecraft:overworld").unwrap();
            assert_eq!(
                level.logical_height(game),
                Some(256),
                "client's own dimension shape"
            );
            assert_eq!(level.get_sky_darken(game), Some(game.sky_darken()));
            assert_eq!(
                nether.logical_height(game),
                None,
                "another dimension isn't visible to a client"
            );
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn closest_biome_uses_the_named_generator_without_loading_resident_chunks() {
        run_closest_biome_uses_the_named_generator_without_loading_resident_chunks();
    }

    #[cfg(feature = "client")]
    pub(crate) fn run_closest_biome_uses_the_named_generator_without_loading_resident_chunks() {
        use sunrise_sim::vanillagen::{TerrainResources, VanillaTerrain};

        with_game(|game| {
            game.enter_level(Dim::Nether).unwrap();
            let directory = game.options.resources.join("worldgen");
            let terrain = VanillaTerrain::for_dimension(
                TerrainResources {
                    data: Some(&game.pack.data_changes),
                    dir: &directory,
                    registry: game.pack.assets.registry,
                    biomes: world_biomes!(game),
                    item_data: &game.res.core.item_data,
                },
                12345,
                "minecraft:the_nether",
            )
            .unwrap();
            game.end.set_locator(Some(Arc::new(terrain)));
            let nether = game.level("minecraft:the_nether").unwrap();
            let parameters = BiomeSearch::new([8, 64, 8], 0, 32, 64);
            let before = (game.world.chunks.len(), game.server.streaming.pending());
            let found = nether
                .find_closest_biome(game, "#minecraft:is_nether", parameters)
                .expect("all Nether source biomes match");
            assert_eq!(
                found.position,
                [8, 64, 8],
                "vanilla samples the origin first"
            );
            assert!(
                [
                    "minecraft:nether_wastes",
                    "minecraft:soul_sand_valley",
                    "minecraft:crimson_forest",
                    "minecraft:warped_forest",
                    "minecraft:basalt_deltas"
                ]
                .contains(&found.biome.as_str())
            );
            assert_eq!(
                nether.find_closest_biome(game, "minecraft:plains", parameters),
                None
            );
            assert_eq!(
                nether.find_closest_biome(game, "minecraft:no_such_biome", parameters),
                None
            );
            assert_eq!(
                nether.find_closest_biome(
                    game,
                    "#minecraft:is_nether",
                    BiomeSearch::new([0, 64, 0], -1, 32, 64)
                ),
                None
            );
            assert_eq!(
                nether.find_closest_biome(
                    game,
                    "#minecraft:is_nether",
                    BiomeSearch::new([0, 64, 0], 32, 0, 64)
                ),
                None
            );
            // Vanilla `/locate biome` extents and large radii run without an artificial sample cap.
            for parameters in [
                BiomeSearch::new([0, 64, 0], 0, 32, i32::MAX),
                BiomeSearch::new([0, 64, 0], 6400, 32, 64),
                BiomeSearch::new([0, 64, 0], 128, 1, i32::MAX),
                BiomeSearch::new([0, 64, 0], i32::MAX, 1, 1),
            ] {
                let hit = nether
                    .find_closest_biome(game, "#minecraft:is_nether", parameters)
                    .expect("valid search should not be refused by a sample budget");
                assert_eq!(hit.position, [0, 64, 0]);
            }
            for parameters in [
                BiomeSearch::new([0, 64, 0], 0, 32, 0),
                BiomeSearch::new([0, 64, 0], 0, 32, -1),
                BiomeSearch::new([0, 64, 0], 0, -1, 64),
            ] {
                assert_eq!(
                    nether.find_closest_biome(game, "#minecraft:is_nether", parameters),
                    None,
                    "nonpositive step must be refused"
                );
            }
            assert_eq!(
                before,
                (game.world.chunks.len(), game.server.streaming.pending())
            );
            game.enter_level(Dim::Overworld).unwrap();
            let parked = nether
                .find_closest_biome(game, "#minecraft:is_nether", parameters)
                .unwrap();
            assert_eq!(parked, found, "parked level retains its generator and seed");
            let stale = LevelRef::new(
                WorldEpoch::new(nether.epoch().get() + 1),
                nether.dimension(),
            );
            assert_eq!(
                stale.find_closest_biome(game, "#minecraft:is_nether", parameters),
                None
            );
        });
    }

    /// The nearest stronghold lies on vanilla's first ring: 3 strongholds at
    /// `4 * 32 + (nextDouble - 0.5) * 32 * 2.5` chunks from the origin, 88 to 168
    /// (`ConcentricRingsStructurePlacement` with distance 32), each then moved
    /// to a biome it may start in within 112 blocks.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn nearest_structure_uses_the_named_generator_without_loading_resident_chunks() {
        run_nearest_structure_uses_the_named_generator_without_loading_resident_chunks();
    }

    #[cfg(feature = "client")]
    pub(crate) fn run_nearest_structure_uses_the_named_generator_without_loading_resident_chunks() {
        use mod_api::StructureSearch;
        use sunrise_sim::vanillagen::{TerrainResources, VanillaTerrain};

        with_game(|game| {
            let directory = game.options.resources.join("worldgen");
            let terrain = VanillaTerrain::for_dimension(
                TerrainResources {
                    data: Some(&game.pack.data_changes),
                    dir: &directory,
                    registry: game.pack.assets.registry,
                    biomes: world_biomes!(game),
                    item_data: &game.res.core.item_data,
                },
                12345,
                "minecraft:overworld",
            )
            .unwrap();
            game.end.set_locator(Some(Arc::new(terrain)));
            let overworld = game.level("minecraft:overworld").unwrap();
            let before = (game.world.chunks.len(), game.server.streaming.pending());
            let search = StructureSearch::new([0, 64, 0], 100);
            let found = overworld
                .find_nearest_structure(game, "minecraft:stronghold", search)
                .expect("every Overworld has strongholds");
            assert_eq!(found.structure, "minecraft:stronghold");
            let [x, y, z] = found.position;
            assert_eq!(y, 0, "the start chunk's corner at y 0");
            assert_eq!((x.rem_euclid(16), z.rem_euclid(16)), (0, 0));
            // `findBiomeHorizontal` then moves each ring position up to 112 blocks.
            let chunks = f64::from(x.div_euclid(16)).hypot(f64::from(z.div_euclid(16)));
            assert!((81.0..=175.0).contains(&chunks), "{chunks} chunks out");
            assert_eq!(
                overworld.find_nearest_structure(game, "stronghold", search),
                Some(found.clone()),
                "an id without a namespace is minecraft's"
            );
            assert_eq!(
                overworld.find_nearest_structure(game, "#minecraft:eye_of_ender_located", search),
                Some(found.clone()),
                "a tag searches its members"
            );
            assert_eq!(
                overworld.find_nearest_structure(
                    game,
                    "minecraft:stronghold",
                    StructureSearch::new([x, 64, z], 0)
                ),
                Some(found.clone()),
                "from the start itself"
            );
            for structure in ["minecraft:fortress", "minecraft:no_such_structure"] {
                assert_eq!(
                    overworld.find_nearest_structure(game, structure, search),
                    None,
                    "{structure} does not generate here"
                );
            }
            // Radii past 100 are accepted for both concentric-ring and random-spread searches.
            for radius in [101, i32::MAX] {
                assert_eq!(
                    overworld.find_nearest_structure(
                        game,
                        "minecraft:stronghold",
                        StructureSearch::new([0, 64, 0], radius)
                    ),
                    Some(found.clone()),
                    "radius {radius} is accepted"
                );
            }
            let mineshaft = overworld
                .find_nearest_structure(game, "minecraft:mineshaft", search)
                .expect("mineshaft in Overworld");
            assert_eq!(
                overworld.find_nearest_structure(
                    game,
                    "minecraft:mineshaft",
                    StructureSearch::new([0, 64, 0], 101)
                ),
                Some(mineshaft)
            );
            for parameters in [
                StructureSearch::new([0, 64, 0], -1),
                StructureSearch::new([i32::MAX, 64, 0], 100),
                StructureSearch::new([0, 64, i32::MIN], 100),
                StructureSearch::new([30_000_000, 64, 0], 0),
            ] {
                assert_eq!(
                    overworld.find_nearest_structure(game, "minecraft:stronghold", parameters),
                    None,
                    "{parameters:?} is refused"
                );
            }
            assert!(
                overworld
                    .find_nearest_structure(
                        game,
                        "minecraft:stronghold",
                        StructureSearch::new([29_999_999, i32::MIN, -30_000_000], 0)
                    )
                    .is_some(),
                "the world's edge and any height are searched"
            );
            assert_eq!(
                before,
                (game.world.chunks.len(), game.server.streaming.pending())
            );
            let stale = LevelRef::new(
                WorldEpoch::new(overworld.epoch().get() + 1),
                overworld.dimension(),
            );
            assert_eq!(
                stale.find_nearest_structure(game, "minecraft:stronghold", search),
                None
            );
        });
    }

    /// `getStructureWithPieceAt` and `getStructureAt` over a stronghold start the generator
    /// builds: inside a piece both name it, between its pieces only the bounding box does, a tag
    /// and a bare id name it too, another structure is not there, and an unloaded chunk or a
    /// stale level gives no answer.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn structures_at_reads_the_loaded_chunks_references() {
        use mod_api::StructureSearch;
        use sunrise_sim::vanillagen::{TerrainResources, VanillaTerrain};

        with_game(|game| {
            let directory = game.options.resources.join("worldgen");
            let terrain = VanillaTerrain::for_dimension(
                TerrainResources {
                    data: Some(&game.pack.data_changes),
                    dir: &directory,
                    registry: game.pack.assets.registry,
                    biomes: world_biomes!(game),
                    item_data: &game.res.core.item_data,
                },
                12345,
                "minecraft:overworld",
            )
            .unwrap();
            let terrain = Arc::new(terrain);
            game.end.set_locator(Some(terrain.clone()));
            let overworld = game.level("minecraft:overworld").unwrap();
            let found = overworld
                .find_nearest_structure(
                    game,
                    "minecraft:stronghold",
                    StructureSearch::new([0, 64, 0], 100),
                )
                .unwrap();
            let start_chunk = (found.position[0] >> 4, found.position[2] >> 4);
            let start = terrain
                .starts_at(start_chunk.0, start_chunk.1)
                .into_iter()
                .find(|start| start.name == "minecraft:stronghold")
                .unwrap();
            let in_a_piece = |pos: [i32; 3]| {
                start
                    .pieces
                    .iter()
                    .any(|piece| piece.bounding_box().is_inside(pos))
            };
            let in_piece = start.pieces[0].bounding_box().min;
            let [min, max] = [start.bounding_box.min, start.bounding_box.max];
            let between = (min[1]..=max[1])
                .flat_map(|y| {
                    (min[0]..=max[0]).flat_map(move |x| (min[2]..=max[2]).map(move |z| [x, y, z]))
                })
                .find(|&pos| !in_a_piece(pos))
                .expect("a stronghold's box has room between its pieces");

            let stronghold = || Some(vec!["minecraft:stronghold".to_owned()]);
            for pos in [in_piece, between] {
                let chunk = (pos[0] >> 4, pos[2] >> 4);
                if game.world.chunks.contains_key(&chunk) {
                    continue;
                }
                assert_eq!(
                    overworld.structures_at(game, pos, "minecraft:stronghold"),
                    None
                );
                let mut referring = sunrise_level::chunk::Chunk::empty(chunk.0, chunk.1);
                referring
                    .structure_refs
                    .push(("minecraft:stronghold".into(), start_chunk));
                game.world.chunks.insert(chunk, Arc::new(referring));
            }
            for selector in [
                "minecraft:stronghold",
                "stronghold",
                "#minecraft:eye_of_ender_located",
            ] {
                assert_eq!(
                    overworld.structures_with_piece_at(game, in_piece, selector),
                    stronghold()
                );
                assert_eq!(
                    overworld.structures_at(game, in_piece, selector),
                    stronghold()
                );
            }
            assert_eq!(
                overworld.structures_with_piece_at(game, between, "minecraft:stronghold"),
                Some(Vec::new())
            );
            assert_eq!(
                overworld.structures_at(game, between, "minecraft:stronghold"),
                stronghold()
            );
            assert_eq!(
                overworld.structures_at(game, in_piece, "minecraft:fortress"),
                Some(Vec::new())
            );
            let stale = LevelRef::new(
                WorldEpoch::new(overworld.epoch().get() + 1),
                overworld.dimension(),
            );
            assert_eq!(
                stale.structures_at(game, in_piece, "minecraft:stronghold"),
                None
            );
        });
    }

    /// API results over original fixture worlds, compared with independently recorded vanilla calls.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn level_environment_reads_match_vanilla_vectors() {
        run_level_environment_reads_match_vanilla_vectors();
    }

    #[cfg(feature = "client")]
    pub(crate) fn run_level_environment_reads_match_vanilla_vectors() {
        use serde_json::{Value, json};
        use std::collections::HashMap;
        use sunrise_sim::vanillagen::{TerrainResources, VanillaTerrain};

        let fixture = |name: &str, variable: &str| {
            let path = std::env::var_os(variable)
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| {
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../../tests/fixtures")
                        .join(name)
                });
            std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .collect::<Vec<_>>()
        };
        let inputs = fixture("level_environment_reads.jsonl", "SUNRISE_LEVEL_READ_CASES");
        let expected = fixture(
            "level_environment_reads_expected.jsonl",
            "SUNRISE_LEVEL_READ_EXPECTED",
        );
        assert!(inputs.len() >= 476);
        assert_eq!(inputs.len(), expected.len());
        let position =
            |value: &Value| std::array::from_fn::<_, 3, _>(|i| value[i].as_i64().unwrap() as i32);
        with_game(|game| {
            let mut generators = HashMap::new();
            for (input, expected) in inputs.iter().zip(&expected) {
                assert_eq!(input["name"], expected["name"]);
                let dimension = input["dimension"].as_str().unwrap_or("minecraft:overworld");
                let dim = Dim::from_id(dimension);
                if game.dim != dim {
                    game.enter_level(dim).unwrap();
                }
                let kind = input["kind"].as_str().unwrap();
                match kind {
                    "biome" => {
                        game.world.chunks.clear();
                        let ids = ["minecraft:plains", "minecraft:desert", "minecraft:forest"]
                            .map(|name| world_biomes!(game).id(name));
                        for column in input["loaded"].as_array().unwrap() {
                            let (cx, cz) = (
                                column[0].as_i64().unwrap() as i32,
                                column[1].as_i64().unwrap() as i32,
                            );
                            let mut chunk = Chunk::empty(cx, cz);
                            for y in 0..chunk.sections.len() as i32 * 4 {
                                for x in 0..4 {
                                    for z in 0..4 {
                                        let qy = (chunk.min_y() >> 2) + y;
                                        let index =
                                            ((cx * 4 + x) * 31 + qy * 7 + (cz * 4 + z) * 17)
                                                .rem_euclid(3);
                                        chunk.set_biome_cell(x, y, z, ids[index as usize]);
                                    }
                                }
                            }
                            game.world.chunks.insert((cx, cz), Arc::new(chunk));
                        }
                        game.server.streaming.biome_zoom = sunrise_core::biome::BiomeZoom::new(
                            input["zoom_seed"].as_i64().unwrap(),
                        );
                    }
                    "floor" => {
                        game.world.chunks.clear();
                        game.world
                            .chunks
                            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                        for (field, y) in [("here", 100), ("below", 99)] {
                            let state = game
                                .pack
                                .assets
                                .registry
                                .default_state(input[field].as_str().unwrap())
                                .unwrap();
                            game.world.set(8, y, 8, state).unwrap();
                        }
                    }
                    "sky" => {
                        game.world.day_time = input["time"].as_i64().unwrap();
                        let rain = input["rain"].as_f64().unwrap() as f32;
                        let thunder = input["thunder"].as_f64().unwrap() as f32;
                        game.world.weather.rain_level = (rain, rain);
                        game.world.weather.thunder_level = (thunder, thunder);
                    }
                    "search" => {
                        let seed = input["seed"].as_i64().unwrap();
                        let key = (dimension.to_owned(), seed);
                        let generator = generators.entry(key).or_insert_with(|| {
                            let directory = game.options.resources.join("worldgen");
                            let terrain = VanillaTerrain::for_dimension(
                                TerrainResources {
                                    data: Some(&game.pack.data_changes),
                                    dir: &directory,
                                    registry: game.pack.assets.registry,
                                    biomes: world_biomes!(game),
                                    item_data: &game.res.core.item_data,
                                },
                                seed,
                                dimension,
                            )
                            .unwrap();
                            Arc::new(terrain)
                        });
                        game.end.set_locator(Some(generator.clone()));
                    }
                    "logical_height" => {}
                    other => panic!("unknown fixture {other}"),
                }
                let level = game.level(dimension).unwrap();
                let before = (game.world.chunks.len(), game.server.streaming.pending());
                let value = match kind {
                    "biome" => json!(level.biome_at(game, position(&input["position"]))),
                    "floor" => json!(level.block_floor_height(game, [8, 100, 8]).map(|height| {
                        if height == f64::NEG_INFINITY {
                            json!("-Infinity")
                        } else {
                            json!(height)
                        }
                    })),
                    "sky" => json!(level.get_sky_darken(game)),
                    "logical_height" => json!(level.logical_height(game)),
                    "search" => {
                        let parameters = BiomeSearch::new(
                            position(&input["position"]),
                            input["radius"].as_i64().unwrap() as i32,
                            input["horizontal"].as_i64().unwrap() as i32,
                            input["vertical"].as_i64().unwrap() as i32,
                        );
                        json!(
                            level
                                .find_closest_biome(
                                    game,
                                    input["selector"].as_str().unwrap(),
                                    parameters
                                )
                                .map(
                                    |found| json!({"biome":found.biome,"position":found.position})
                                )
                        )
                    }
                    _ => unreachable!(),
                };
                assert_eq!(
                    json!({"name":input["name"],"value":value}),
                    *expected,
                    "{}",
                    input["name"]
                );
                assert_eq!(
                    before,
                    (game.world.chunks.len(), game.server.streaming.pending()),
                    "read queued or loaded a chunk: {}",
                    input["name"]
                );
            }
        });
    }
}
