use mod_api::client::particles::PositionSource;
use sunrise_level::particle_options::{ParticleOptions, ParticleType};
use sunrise_protocol::entity_data::PositionSource as NetworkPositionSource;
use sunrise_sim::lan::server_effects::{ServerParticle, ServerParticleData};
use sunrise_world::registry::BlockStateId;

/// Convert engine `ParticleOptions` (from `/particle` or native mod APIs) into a portable
/// [`ServerParticle`]. Returns `None` only when a typed mod particle fails stream encoding.
pub fn server_particle_from_options(
    options: &ParticleOptions,
    state: Option<BlockStateId>,
) -> Option<ServerParticle> {
    use ParticleType as T;
    if let Some(name) = &options.mod_particle {
        let bytes = if let Some((value, _)) = &options.typed_particle {
            match mod_api::__private::encode_particle_stream(
                value,
                sunrise_protocol::frame::MAX_UNCOMPRESSED,
            ) {
                Ok(bytes) => bytes,
                Err(error) => {
                    log::warn!("Cannot encode particle {name}: {error}");
                    return None;
                }
            }
        } else {
            Vec::new()
        };
        return Some(ServerParticle::mod_particle(name.as_ref(), bytes));
    }
    let rgb = [options.color[0], options.color[1], options.color[2]];
    let data = match options.ty {
        T::Block | T::BlockMarker | T::FallingDust | T::DustPillar | T::BlockCrumble => match state
        {
            Some(state) => ServerParticleData::Block(state),
            None => ServerParticleData::None,
        },
        T::Dust => ServerParticleData::ColorScale {
            rgb,
            scale: options.scale,
        },
        T::DustColorTransition => ServerParticleData::ColorScale2 {
            from: rgb,
            to: options.to_color,
            scale: options.scale,
        },
        T::Effect | T::InstantEffect => ServerParticleData::ColorPower {
            rgb,
            power: options.scale,
        },
        T::EntityEffect | T::TintedLeaves | T::Flash => {
            ServerParticleData::ColorRgba(options.color)
        }
        T::DragonBreath | T::SculkCharge => ServerParticleData::Scalar(options.scale),
        T::Shriek => ServerParticleData::Delay(options.scale as i32),
        T::Geyser | T::GeyserPlume => match options.geyser {
            Some((water_blocks, _)) => ServerParticleData::Geyser(water_blocks),
            None => ServerParticleData::None,
        },
        T::GeyserBase | T::GeyserPoof => match options.geyser {
            Some((water_blocks, burst_impulse_base)) => ServerParticleData::GeyserBase {
                water_blocks,
                burst_impulse_base,
            },
            None => ServerParticleData::None,
        },
        T::Vibration => match &options.vibration {
            Some((source, _, ticks)) => ServerParticleData::Vibration {
                source: source.clone(),
                ticks: *ticks,
            },
            None => ServerParticleData::None,
        },
        T::Item => match &options.item_stack {
            Some(stack) => ServerParticleData::Item(stack.clone()),
            None => ServerParticleData::None,
        },
        T::Trail => match options.trail {
            Some((target, duration)) => {
                let b = |v: f32| ((v.clamp(0.0, 1.0) * 255.0).round() as u32) & 255;
                let color = (b(rgb[0]) << 16) | (b(rgb[1]) << 8) | b(rgb[2]);
                ServerParticleData::Trail {
                    target,
                    color,
                    duration,
                }
            }
            None => ServerParticleData::None,
        },
        _ => ServerParticleData::None,
    };
    Some(ServerParticle {
        ty: options.ty,
        data,
    })
}

/// Resolve and decode a custom envelope once, before any effect can spawn it.
pub fn decode_mod_particle(name: &str, bytes: &[u8]) -> Option<ParticleOptions> {
    match mod_api::__private::decode_particle_stream(name, bytes) {
        Ok(Some(value)) => ParticleOptions::typed(value),
        Ok(None) => Some(ParticleOptions::named(name)),
        Err(error) => {
            log::warn!("Cannot decode particle {name}: {error}");
            None
        }
    }
}

/// A protocol or mob-event listener as the API's plain particle data.
pub fn particle_position_source(source: &NetworkPositionSource) -> PositionSource {
    match source {
        NetworkPositionSource::Block(pos) => PositionSource::Block(*pos),
        NetworkPositionSource::Entity { id, y_offset } => PositionSource::Entity {
            id: *id,
            y_offset: *y_offset,
        },
    }
}
