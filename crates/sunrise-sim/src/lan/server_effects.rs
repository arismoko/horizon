//! Portable server-side particle and explosion effect payloads, independent of
//! `sunrise-client-render`.
use sunrise_items::items::ItemStack;
use sunrise_world::particle_type::ParticleType;
use sunrise_world::registry::BlockStateId;

#[derive(Clone, Debug, PartialEq)]
pub enum ServerParticleData {
    None,
    Block(BlockStateId),
    ColorScale {
        rgb: [f32; 3],
        scale: f32,
    },
    ColorScale2 {
        from: [f32; 3],
        to: [f32; 3],
        scale: f32,
    },
    ColorPower {
        rgb: [f32; 3],
        power: f32,
    },
    ColorRgba([f32; 4]),
    Scalar(f32),
    Delay(i32),
    Geyser(i32),
    GeyserBase {
        water_blocks: i32,
        burst_impulse_base: f32,
    },
    Vibration {
        source: mod_api::client::particles::PositionSource,
        ticks: i32,
    },
    Item(ItemStack),
    Trail {
        target: [f64; 3],
        color: u32,
        duration: i32,
    },
    Mod {
        name: String,
        options: Vec<u8>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ServerParticle {
    pub ty: ParticleType,
    pub data: ServerParticleData,
}

impl ServerParticle {
    pub fn simple(ty: ParticleType) -> Self {
        Self {
            ty,
            data: ServerParticleData::None,
        }
    }

    pub fn block(ty: ParticleType, state: BlockStateId) -> Self {
        Self {
            ty,
            data: ServerParticleData::Block(state),
        }
    }

    pub fn colored(ty: ParticleType, rgba: [f32; 4]) -> Self {
        Self {
            ty,
            data: ServerParticleData::ColorRgba(rgba),
        }
    }

    pub fn dust(rgb: [f32; 3], scale: f32) -> Self {
        Self {
            ty: ParticleType::Dust,
            data: ServerParticleData::ColorScale {
                rgb,
                scale: scale.clamp(0.01, 4.0),
            },
        }
    }

    pub fn dust_transition(from: [f32; 3], to: [f32; 3], scale: f32) -> Self {
        Self {
            ty: ParticleType::DustColorTransition,
            data: ServerParticleData::ColorScale2 {
                from,
                to,
                scale: scale.clamp(0.01, 4.0),
            },
        }
    }

    pub fn spell(ty: ParticleType, rgb: [f32; 3], power: f32) -> Self {
        Self {
            ty,
            data: ServerParticleData::ColorPower { rgb, power },
        }
    }

    pub fn scalar(ty: ParticleType, value: f32) -> Self {
        Self {
            ty,
            data: ServerParticleData::Scalar(value),
        }
    }

    pub fn shriek(delay: i32) -> Self {
        Self {
            ty: ParticleType::Shriek,
            data: ServerParticleData::Delay(delay),
        }
    }

    pub fn item(stack: ItemStack) -> Self {
        Self {
            ty: ParticleType::Item,
            data: ServerParticleData::Item(stack),
        }
    }

    pub fn trail(target: [f64; 3], color: u32, duration: i32) -> Self {
        Self {
            ty: ParticleType::Trail,
            data: ServerParticleData::Trail {
                target,
                color,
                duration,
            },
        }
    }

    pub fn vibration(source: mod_api::client::particles::PositionSource, ticks: i32) -> Self {
        Self {
            ty: ParticleType::Vibration,
            data: ServerParticleData::Vibration { source, ticks },
        }
    }

    pub fn geyser(ty: ParticleType, water_blocks: i32, burst_impulse_base: f32) -> Self {
        let data = match ty {
            ParticleType::Geyser | ParticleType::GeyserPlume => {
                ServerParticleData::Geyser(water_blocks)
            }
            _ => ServerParticleData::GeyserBase {
                water_blocks,
                burst_impulse_base,
            },
        };
        Self { ty, data }
    }

    pub fn mod_particle(name: impl Into<String>, options: Vec<u8>) -> Self {
        Self {
            ty: ParticleType::Smoke,
            data: ServerParticleData::Mod {
                name: name.into(),
                options,
            },
        }
    }

    pub fn is_mod(&self) -> bool {
        matches!(self.data, ServerParticleData::Mod { .. })
    }
}

/// Whether `particle` has valid portable option data for network delivery.
pub fn server_particle_ok(particle: &ServerParticle) -> bool {
    use ParticleType as T;
    if let ServerParticleData::Mod { name, .. } = &particle.data {
        return name
            .split_once(':')
            .is_some_and(|(ns, path)| !ns.is_empty() && !path.is_empty());
    }
    match particle.ty {
        T::Block | T::BlockMarker | T::FallingDust | T::DustPillar | T::BlockCrumble => {
            matches!(particle.data, ServerParticleData::Block(_))
        }
        T::Item => matches!(
            &particle.data,
            ServerParticleData::Item(stack) if stack.count > 0
        ),
        T::Trail => matches!(
            &particle.data,
            ServerParticleData::Trail { target, .. } if target.iter().all(|v| v.is_finite())
        ),
        T::Vibration => matches!(
            &particle.data,
            ServerParticleData::Vibration { source, .. }
                if match source {
                    mod_api::client::particles::PositionSource::Block(_) => true,
                    mod_api::client::particles::PositionSource::Entity { y_offset, .. } => {
                        y_offset.is_finite()
                    }
                    _ => false,
                }
        ),
        T::Geyser | T::GeyserPlume => matches!(
            particle.data,
            ServerParticleData::Geyser(_) | ServerParticleData::GeyserBase { .. }
        ),
        T::GeyserBase | T::GeyserPoof => matches!(
            particle.data,
            ServerParticleData::GeyserBase {
                burst_impulse_base,
                ..
            } if burst_impulse_base.is_finite()
        ),
        _ => match &particle.data {
            ServerParticleData::None => true,
            ServerParticleData::ColorScale { rgb, scale } => {
                rgb.iter().all(|v| v.is_finite()) && scale.is_finite()
            }
            ServerParticleData::ColorScale2 { from, to, scale } => {
                from.iter().all(|v| v.is_finite())
                    && to.iter().all(|v| v.is_finite())
                    && scale.is_finite()
            }
            ServerParticleData::ColorPower { rgb, power } => {
                rgb.iter().all(|v| v.is_finite()) && power.is_finite()
            }
            ServerParticleData::ColorRgba(rgba) => rgba.iter().all(|v| v.is_finite()),
            ServerParticleData::Scalar(v) => v.is_finite(),
            ServerParticleData::Delay(_) => true,
            _ => false,
        },
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ServerExplosionParticle {
    pub particle: ServerParticle,
    pub scaling: f32,
    pub speed: f32,
    pub weight: i32,
}

impl ServerExplosionParticle {
    /// `Level.DEFAULT_EXPLOSION_BLOCK_PARTICLES`: poof (scaling 0.5) and smoke, one each.
    pub fn defaults() -> Vec<Self> {
        let entry = |ty, scaling| Self {
            particle: ServerParticle::simple(ty),
            scaling,
            speed: 1.0,
            weight: 1,
        };
        vec![
            entry(ParticleType::Poof, 0.5),
            entry(ParticleType::Smoke, 1.0),
        ]
    }
}

/// The particles and sound of `ServerLevel.explode`'s full overload.
#[derive(Clone, Debug, PartialEq)]
pub struct BlastEffects {
    /// The centre particle of a small blast (`ServerExplosion.isSmall`).
    pub small: ServerParticle,
    pub large: ServerParticle,
    pub block_particles: Vec<ServerExplosionParticle>,
    pub sound: String,
}

impl Default for BlastEffects {
    /// What `Level.explode`'s shorter forms pass.
    fn default() -> Self {
        Self {
            small: ServerParticle::simple(ParticleType::Explosion),
            large: ServerParticle::simple(ParticleType::ExplosionEmitter),
            block_particles: ServerExplosionParticle::defaults(),
            sound: "minecraft:entity.generic.explode".into(),
        }
    }
}

impl BlastEffects {
    /// The centre particle: the small one for a radius under 2 or a blast
    /// that keeps its blocks.
    pub fn emitter(&self, small: bool) -> &ServerParticle {
        if small { &self.small } else { &self.large }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ServerQueuedParticle {
    pub dimension: &'static str,
    pub particle: ServerParticle,
    pub at: [f64; 3],
    pub spread: [f64; 3],
    /// Per-axis maximum speed used by the packet's ALTERNATIVE randomization.
    pub speed: [f64; 3],
    pub count: i32,
    /// The packet's `RandomizationType` (0 DEFAULT, 1 ALTERNATIVE).
    pub randomization: i32,
    pub except: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ServerExplosion {
    pub center: [f64; 3],
    pub small: bool,
    pub radius: f32,
    pub block_count: usize,
    pub pushed: Vec<(u32, [f64; 3])>,
    pub effects: BlastEffects,
}
