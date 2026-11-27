pub(crate) use crate::game::server_side::commands::world_cmds::particle_command_tests::game;
use mod_api::client::particles::PositionSource;
use std::sync::Arc;
use sunrise_client::particles::ParticleLevel;
use sunrise_level::chunk::Chunk;
use sunrise_level::particle_options::ParticleOptions;

/// The crumbs the command names are queued on the server and drawn on the client:
/// `TerrainParticle.CrumblingProvider` keeps no particle for options that carry no block.
#[test]
#[ignore = "daily"]
fn block_crumble_draws_the_block_it_names() {
    let mut resources = None;
    let mut server = game(&mut resources);
    let options = drawn(
        &mut server,
        "particle minecraft:block_crumble{block_state:\"minecraft:stone\"} 0.5 100.5 0.5 0 0 0 0 1",
    );
    crate::game::native_entity_refs::tests::with_client_game(|client| {
        client
            .world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        client.client.particles.0.created_requests = None;
        client.client.particles.0.clear();
        client
            .client
            .particles
            .0
            .create(&options, [0.5, 100.5, 0.5], [0.0; 3]);
        let level = ParticleLevel {
            world: &client.common.world,
            facts: client.common.res.facts(),
            collision: client.common.collision,
            player: None,
            cached_chunk: std::cell::Cell::new(None),
        };
        client.client.particles.0.tick(&level);
        assert_eq!(client.client.particles.0.len(), 1, "the crumble is drawn");
    });
}

/// `TerrainParticle.Provider` keeps no particle for air: the command still reaches the
/// player, but the window draws nothing, not some other block's crumbs.
#[test]
#[ignore = "daily"]
fn block_particles_of_air_reach_the_player_and_draw_nothing() {
    use {
        sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client,
        sunrise_protocol::entity_data::{Particle, ParticleData},
        sunrise_sim::lan::server_effects::ServerParticleData,
    };
    let mut resources = None;
    let mut server = game(&mut resources);
    server.server.pending_particles.clear();
    assert!(server.run_command(
        "particle minecraft:block{block_state:\"minecraft:air\"} 0.5 100.5 0.5 0 0 0 0 1"
    ));
    assert_eq!(server.server.pending_particles.len(), 1);
    let queued = server.server.pending_particles.remove(0);
    let ServerParticleData::Block(state) = queued.particle.data else {
        panic!(
            "expected block particle data, got {:?}",
            queued.particle.data
        );
    };
    crate::game::native_entity_refs::tests::with_client_game(|client| {
        let _server = logged_client(client);
        let id = client
            .client
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:particle_type")
            .unwrap()
            .id(&format!("minecraft:{}", queued.particle.ty.name()))
            .unwrap();
        assert!(
            client
                .received_particle(
                    &Particle {
                        id,
                        data: Some(ParticleData::State(state.0 as i32)),
                    },
                    queued.at,
                )
                .is_none()
        );
    });
}

/// `BlockMarker.Provider` draws air's particle material, the missing model's: the missing
/// texture the particle atlas packs.
#[test]
#[ignore = "daily"]
fn a_block_marker_of_air_draws_the_missing_texture() {
    use {
        sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client,
        sunrise_protocol::entity_data::{Particle, ParticleData},
        sunrise_sim::lan::server_effects::ServerParticleData,
    };
    let mut resources = None;
    let mut server = game(&mut resources);
    server.server.pending_particles.clear();
    assert!(server.run_command(
        "particle minecraft:block_marker{block_state:\"minecraft:air\"} 0.5 100.5 0.5 0 0 0 0 1"
    ));
    assert_eq!(server.server.pending_particles.len(), 1);
    let queued = server.server.pending_particles.remove(0);
    let ServerParticleData::Block(state) = queued.particle.data else {
        panic!(
            "expected block particle data, got {:?}",
            queued.particle.data
        );
    };
    crate::game::native_entity_refs::tests::with_client_game(|client| {
        let _server = logged_client(client);
        let id = client
            .client
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:particle_type")
            .unwrap()
            .id(&format!("minecraft:{}", queued.particle.ty.name()))
            .unwrap();
        let marker = client
            .received_particle(
                &Particle {
                    id,
                    data: Some(ParticleData::State(state.0 as i32)),
                },
                queued.at,
            )
            .expect("the marker is drawn");
        let (sprite, tint) = marker.block.expect("the marker carries its sprite");
        assert_eq!(sprite, sunrise_client::hud::MISSING_SPRITE);
        assert_eq!(tint, 0xFFFFFF);
    });
}

/// The one particle the server queues for `command`, decoded into client `ParticleOptions`.
pub(crate) fn drawn(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    command: &str,
) -> ParticleOptions {
    use sunrise_sim::lan::server_effects::ServerParticleData;
    game.server.pending_particles.clear();
    assert!(game.run_command(command), "{command}");
    let mut made = std::mem::take(&mut game.server.pending_particles);
    assert_eq!(made.len(), 1, "{command}");
    let queued = made.remove(0);
    if let ServerParticleData::Mod { name, options } = &queued.particle.data {
        return sunrise_gameplay_common::game::lan_particle_data::decode_mod_particle(
            name, options,
        )
        .expect("mod particle decodes");
    }
    let ty = queued.particle.ty;
    match queued.particle.data {
        ServerParticleData::None => ParticleOptions::simple(ty),
        ServerParticleData::Block(state) => {
            let source = game.res.facts().get(state).name.clone();
            let sprite = format!(
                "block/{}",
                source.strip_prefix("minecraft:").unwrap_or(&source)
            );
            let tint = 0xFF_FFFF;
            ParticleOptions::block(ty, sprite, tint).with_source(&source)
        }
        ServerParticleData::ColorScale { rgb, scale } => ParticleOptions {
            color: [rgb[0], rgb[1], rgb[2], 1.0],
            scale,
            ..ParticleOptions::simple(ty)
        },
        ServerParticleData::ColorScale2 { from, to, scale } => ParticleOptions {
            color: [from[0], from[1], from[2], 1.0],
            to_color: to,
            scale,
            ..ParticleOptions::simple(ty)
        },
        ServerParticleData::ColorPower { rgb, power } => ParticleOptions {
            color: [rgb[0], rgb[1], rgb[2], 1.0],
            scale: power,
            ..ParticleOptions::simple(ty)
        },
        ServerParticleData::ColorRgba(rgba) => ParticleOptions {
            color: rgba,
            ..ParticleOptions::simple(ty)
        },
        ServerParticleData::Scalar(v) => ParticleOptions {
            scale: v,
            ..ParticleOptions::simple(ty)
        },
        ServerParticleData::Delay(ticks) => ParticleOptions {
            scale: ticks as f32,
            ..ParticleOptions::simple(ty)
        },
        ServerParticleData::Vibration { source, ticks } => {
            let to = match &source {
                PositionSource::Block(p) => [
                    f64::from(p[0]) + 0.5,
                    f64::from(p[1]) + 0.5,
                    f64::from(p[2]) + 0.5,
                ],
                _ => queued.at,
            };
            ParticleOptions {
                vibration: Some((source, to, ticks)),
                ..ParticleOptions::simple(ty)
            }
        }
        other => panic!("unexpected particle data for {command}: {other:?}"),
    }
}
