use crate::game::remote::tests::client_level_environment_tests::drawn;
use crate::game::remote::tests::client_level_environment_tests::login;
use crate::game::remote::tests::client_level_environment_tests::respawn;
use crate::game::remote::tests::client_level_environment_tests::send_types;
use crate::game::remote::tests::client_level_environment_tests::spawn;
use crate::game::remote::tests::client_level_environment_tests::with_height;
use crate::game::server_side::native_entity_refs::tests::with_uncached_client_game as with_client_game;
use std::sync::Arc;
use sunrise_gameplay_common::world_biomes;
use sunrise_protocol::connection::Pipe;
use sunrise_protocol::packets::play::{self, Clientbound as C};

/// Cloud preparation and atmospheric fog read the synced attributes, not the skybox, while
/// precipitation in a loaded column uses the joined level's sea level from the spawn packet.
#[test]
#[ignore = "daily"]
fn client_clouds_fog_and_precipitation_follow_the_joined_level() {
    use glam::DVec3;
    use serde_json::json;
    use sunrise_core::position::BlockPos;
    use sunrise_level::chunk::Chunk;

    let precipitation_at = |game: &sunrise_client_gameplay::game::ClientGame<'_>, x, y, z| {
        sunrise_gameplay_common::game::weather::precipitation_at(
            &game.world,
            world_biomes!(game),
            game.client.streaming.biome_zoom,
            BlockPos::new(x, y, z),
            game.weather_sea_level(),
        )
    };
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::client_side::remote_client::tests::client_level_environment_tests::client_clouds_fog_and_precipitation_follow_the_joined_level",
        || {
            with_client_game(|game| {
                let mut ty = with_height("overworld", 192.33);
                ty["timelines"] = json!([]);
                ty["attributes"] = json!({
                    "minecraft:visual/cloud_color": "#804db5ed",
                    "minecraft:visual/fog_color": "#335580",
                    "minecraft:visual/sky_color": "#2a4099",
                    "minecraft:visual/sun_angle": 90.0,
                    "minecraft:visual/sunrise_sunset_color": "#99df6619",
                    "minecraft:visual/sky_fog_end_distance": 512.0,
                });
                let types = [
                    ("test:overworld", "overworld"),
                    ("test:none", "none"),
                    ("test:end", "end"),
                    ("test:near", "none"),
                ]
                .map(|(name, skybox)| {
                    let mut ty = ty.clone();
                    ty["skybox"] = json!(skybox);
                    if name == "test:near" {
                        ty["attributes"]["minecraft:visual/sky_fog_end_distance"] = json!(64.0);
                    }
                    (name, ty)
                });
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "ClientWeatherTest").unwrap();
                send_types(game, &types);
                game.client.settings.sunrise.fog = true;
                let camera = DVec3::new(0.0, 64.0, 0.0);
                let mut clouds = Vec::new();
                let mut fog = Vec::new();

                for (index, (dimension, sea_level)) in [
                    ("minecraft:overworld", 0),
                    ("minecraft:the_nether", 63),
                    ("minecraft:the_end", 120),
                    ("test:near", 120),
                ]
                .into_iter()
                .enumerate()
                {
                    let mut entered = spawn(index as i32, dimension);
                    entered.sea_level = sea_level;
                    entered.seed = 42;
                    if index == 0 {
                        login(game, entered);
                        game.handle_packet(C::SetChunkCacheRadius(play::Distance { distance: 8 }))
                            .unwrap();
                    } else {
                        respawn(game, entered);
                    }
                    assert_eq!(game.overworld_skybox(), index == 0);
                    assert_eq!(game.end_skybox(), index == 2);
                    let sky = drawn(game);
                    let mut transparent = sky;
                    transparent.cloud_color[3] = 0.0;
                    game.client.settings.clouds = "true".into();
                    let enabled = game.frame_cloud_uv(false, &sky).is_some();
                    let zero_alpha = game.frame_cloud_uv(false, &transparent).is_some();
                    game.client.settings.clouds = "false".into();
                    let off = game.frame_cloud_uv(false, &sky).is_some();
                    clouds.push([enabled, zero_alpha, off]);

                    let mut no_tint = sky;
                    no_tint.sunrise_color[3] = 0.0;
                    let mut sample = |forward, distance, sky, draws_sky| {
                        game.client.view.forwards = [forward, 0.0, 0.0];
                        game.client.settings.render_distance = distance;
                        game.client.settings.sunrise.sky = draws_sky;
                        let f = game.frame_fog(camera, sky, 0, [0.0; 3]);
                        let [r, g, b, _] = f.color;
                        ([r, g, b].map(f32::to_bits), f.sky_end)
                    };
                    fog.push([
                        sample(-1.0, 8, &sky, true),
                        sample(-1.0, 8, &sky, false),
                        sample(1.0, 8, &sky, true),
                        sample(-1.0, 8, &no_tint, true),
                        sample(-1.0, 3, &sky, true),
                        sample(-1.0, 3, &no_tint, true),
                    ]);

                    if index < 3 {
                        assert_eq!(game.weather_sea_level(), sea_level);
                        assert_eq!(precipitation_at(game, 16, 100, 8), None);
                        let biomes = world_biomes!(game);
                        let taiga = biomes.id("minecraft:taiga");
                        let desert = biomes.id("minecraft:desert");
                        let layout = game.world.layout();
                        let mut chunk = Chunk::with_layout(0, 0, layout);
                        for qy in 0..(layout.max_y() - layout.min_y()) / 4 {
                            for qz in 0..4 {
                                for qx in 0..4 {
                                    let biome = if index == 0 && qx == 1 { desert } else { taiga };
                                    chunk.set_biome_cell(qx, qy, qz, biome);
                                }
                            }
                        }
                        if index == 0 {
                            // At this boundary 26.3 chooses desert (quart x=1 with seed 42),
                            // not the raw quart's taiga (quart x=0).
                            assert_eq!(chunk.biome(3, 100, 5), taiga);
                        }
                        game.world.chunks.insert((0, 0), Arc::new(chunk));
                        if index == 0 {
                            assert_eq!(precipitation_at(game, 3, 100, 5), None);
                        }

                        // The same biome and two already sampled positions keep their first
                        // temperature; new positions use the newly joined level's sea level.
                        assert_eq!(
                            [100, 170].map(|y| precipitation_at(game, 10, y, 10)),
                            [Some(true), Some(true)],
                            "cached at sea level {sea_level}"
                        );
                        let fresh = [[true, true], [false, true], [false, false]][index];
                        assert_eq!(
                            [100, 170].map(|y| precipitation_at(game, 10 + index as i32, y, 10)),
                            fresh.map(Some),
                            "fresh at sea level {sea_level}"
                        );
                        if index == 2 {
                            // Only this test's child process sees the competing server table.
                            sunrise_world::level::register_sea_level(
                                sunrise_core::LevelKey::parse(dimension).unwrap(),
                                0,
                            );
                            assert_eq!(game.dim.sea_level(), 0);
                            assert_eq!(game.weather_sea_level(), sea_level);
                            assert_eq!(precipitation_at(game, 13, 170, 10), Some(false));

                            let mut same_level = spawn(index as i32, dimension);
                            same_level.sea_level = 0;
                            respawn(game, same_level);
                            assert_eq!(game.weather_sea_level(), sea_level);
                            assert_eq!(precipitation_at(game, 9, 170, 10), Some(false));
                        }
                    }
                }

                assert_eq!(clouds, [[true, false, false]; 4]);
                assert_eq!(fog[0], fog[1], "no skybox");
                assert_eq!(fog[0], fog[2], "End skybox");
                for [toward, sky_off, away, transparent, close, close_transparent] in &fog {
                    assert_eq!(toward, sky_off);
                    assert_ne!(toward.0, away.0);
                    assert_eq!(away, transparent);
                    assert_eq!(close, close_transparent);
                }
                // AtmosphericFogEnvironment.getBaseColor: first blend the synthetic fog
                // toward sunrise at 153/255, then toward the sky by
                // 1 - pow(lerp(min(distance / 16, 8) / 32, 0.25, 1), 0.25).
                assert_eq!(fog[0][0].0, [0x3f05c604, 0x3eb3733e, 0x3ea575a1]);
                assert_eq!(fog[3][0], ([0x3f0069df, 0x3eb07820, 0x3eadc0aa], 64.0));
            })
        },
    );
}

use sunrise_gameplay_common::game::weather::WeatherGameExt as _;
