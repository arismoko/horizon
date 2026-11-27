//! Engine particle provider options, independent of API registry keys.
use mod_api::client::particles::PositionSource;
use std::{collections::BTreeMap, sync::Arc};
use sunrise_core::{
    geometry::{Vec3, trig},
    random::RandomSource,
};
pub use sunrise_game_data::particle_type::ParticleType;
use sunrise_world::particle_options::ParticleOptions as PersistentOptions;

/// What an emitter seeds on the custom particle it spawns: its own `red`... `custom` and customs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Seed {
    pub fields: BTreeMap<String, f64>,
    pub customs: BTreeMap<String, f64>,
}

/// `ParticleOptions` with the parameters of the option classes used here.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods use the constructors
pub struct ParticleOptions {
    pub ty: ParticleType,
    /// `BlockParticleOption`: the block's particle sprite and terrain tint.
    pub block: Option<(String, u32)>,
    /// RGB(A) for dust, spells, entity effects and tinted leaves.
    pub color: [f32; 4],
    /// `DustParticleOptions.scale`, `PowerParticleOption.power`, `SpellParticleOption.power`,
    /// `SculkChargeParticleOptions.roll`, `ShriekParticleOption.delay`.
    pub scale: f32,
    /// `DustColorTransitionOptions.toColor` (`color` is the from colour).
    pub to_color: [f32; 3],
    /// `TrailParticleOption`: the point it flies to and how many ticks it takes.
    pub trail: Option<([f64; 3], i32)>,
    /// `ItemParticleOption`: the particle sprite of each layer of the item's
    /// model as resolved for the ground.
    pub item: Option<Vec<String>>,
    /// Unresolved stack for server particle delivery.
    pub item_stack: Option<sunrise_items::items::ItemStack>,
    /// `GeyserParticleOptions.waterBlocks` and `GeyserBaseParticleOptions`'
    /// `waterBlocks` and `burstImpulseBase` (0 where the type has none).
    pub geyser: Option<(i32, f32)>,
    /// `VibrationParticleOption`: the listener's position source, the resolved
    /// point the signal flies to and its arrival time in ticks. The source is
    /// sent to clients; the resolved point is used for local rendering
    /// (`ParticleEngine::vibration` is the same particle).
    pub vibration: Option<(PositionSource, [f64; 3], i32)>,
    /// A native mod's own particle type, by its namespaced name (`mymod:spark`):
    /// `SimpleParticleType` plus the provider `particles/<name>.json` names.
    /// API requests borrow the immutable load table; native file input can own its name.
    pub mod_particle: Option<std::borrow::Cow<'static, str>>,
    /// A registered type's owned options and its `overrideLimiter` flag.
    pub typed_particle: Option<(mod_api::__private::TypedParticleOptions, bool)>,
    pub source: Option<Arc<str>>,
    pub polytone_seed: Option<Arc<Seed>>,
}
impl ParticleOptions {
    /// Persistent options that don't need a model lookup or a special renderer.
    pub fn from_persistent(options: &PersistentOptions) -> Option<Self> {
        use PersistentOptions as P;
        let rgb = |color: i32| [16, 8, 0].map(|s| ((color >> s) & 255) as f32 / 255.0);
        Some(match *options {
            P::Simple(ty) => Self::simple(ty),
            P::Dust(color, scale) => Self::dust(rgb(color), scale),
            P::DustTransition(from, to, scale) => Self::dust_transition(rgb(from), rgb(to), scale),
            P::Color(ty, color) => {
                let [r, g, b] = rgb(color);
                Self::colored(ty, [r, g, b, ((color >> 24) & 255) as f32 / 255.0])
            }
            P::Spell(ty, color, scale) => {
                let [r, g, b] = rgb(color);
                Self {
                    scale,
                    ..Self::colored(ty, [r, g, b, 1.0])
                }
            }
            P::Power(scale) => Self {
                scale,
                ..Self::simple(ParticleType::DragonBreath)
            },
            P::Charge(roll) => Self::sculk_charge(roll),
            P::Shriek(delay) => Self::shriek(delay),
            P::Geyser(ty, water, base) => Self::geyser_base(ty, water, base.unwrap_or(0.0)),
            P::Trail(pos, color, ticks) => Self::trail(pos, color as u32, ticks),
            P::Vibration(pos, ticks) => Self::vibration(
                PositionSource::Block(pos),
                pos.map(|v| f64::from(v) + 0.5),
                ticks,
            ),
            P::Item(_) | P::Block(..) | P::ElderGuardian => return None,
        })
    }

    pub fn simple(ty: ParticleType) -> Self {
        Self {
            ty,
            block: None,
            color: [1.0; 4],
            scale: 1.0,
            to_color: [1.0, 0.0, 0.0],
            trail: None,
            item: None,
            item_stack: None,
            geyser: None,
            vibration: None,
            mod_particle: None,
            typed_particle: None,
            polytone_seed: None,
            source: None,
        }
    }
    /// A native mod's `SimpleParticleType`: the sprite set of
    /// `assets/<namespace>/particles/<path>.json`, drawn as a plain billboard.
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            mod_particle: Some(std::borrow::Cow::Owned(name.into())),
            ..Self::simple(ParticleType::Smoke)
        }
    }
    /// Resolve the registered metadata once, when making a spawn request.
    pub fn typed(value: mod_api::__private::TypedParticleOptions) -> Option<Self> {
        let key = value.key();
        Some(Self {
            mod_particle: Some(mod_api::__private::particle_name(key)?.into()),
            typed_particle: Some((value, mod_api::__private::particle_override_limiter(key)?)),
            ..Self::simple(ParticleType::Smoke)
        })
    }
    /// `GeyserParticleOptions(type, waterBlocks)`.
    pub fn geyser(ty: ParticleType, water_blocks: i32) -> Self {
        Self {
            geyser: Some((water_blocks, 0.0)),
            ..Self::simple(ty)
        }
    }
    /// `GeyserBaseParticleOptions(type, waterBlocks, burstImpulseBase)`.
    pub fn geyser_base(ty: ParticleType, water_blocks: i32, burst_impulse_base: f32) -> Self {
        Self {
            geyser: Some((water_blocks, burst_impulse_base)),
            ..Self::simple(ty)
        }
    }
    /// `VibrationParticleOption(destination, arrivalInTicks)`: the listener's
    /// position source and the resolved point this engine flies the signal to.
    pub fn vibration(source: PositionSource, dest: [f64; 3], ticks: i32) -> Self {
        Self {
            vibration: Some((source, dest, ticks)),
            ..Self::simple(ParticleType::Vibration)
        }
    }
    /// `ItemParticleOption`: the stack's layer particle sprites (`BreakingItemParticle`).
    pub fn item(layers: Vec<String>) -> Self {
        Self {
            item: Some(layers),
            ..Self::simple(ParticleType::Item)
        }
    }
    /// `DustColorTransitionOptions`.
    pub fn dust_transition(from: [f32; 3], to: [f32; 3], scale: f32) -> Self {
        Self {
            color: [from[0], from[1], from[2], 1.0],
            to_color: to,
            scale: scale.clamp(0.01, 4.0),
            ..Self::simple(ParticleType::DustColorTransition)
        }
    }
    /// `DustColorTransitionOptions.SCULK_TO_REDSTONE`.
    pub fn sculk_to_redstone() -> Self {
        let rgb = |c: u32| {
            [
                ((c >> 16) & 255) as f32 / 255.0,
                ((c >> 8) & 255) as f32 / 255.0,
                (c & 255) as f32 / 255.0,
            ]
        };
        Self::dust_transition(rgb(3_790_560), rgb(16_711_680), 1.0)
    }
    /// `SculkChargeParticleOptions(roll)`.
    pub fn sculk_charge(roll: f32) -> Self {
        Self {
            scale: roll,
            ..Self::simple(ParticleType::SculkCharge)
        }
    }
    /// `ShriekParticleOption(delay)`.
    pub fn shriek(delay: i32) -> Self {
        Self {
            scale: delay as f32,
            ..Self::simple(ParticleType::Shriek)
        }
    }
    /// `TrailParticleOption(target, color, duration)`.
    pub fn trail(target: [f64; 3], rgb: u32, duration: i32) -> Self {
        Self {
            color: [
                ((rgb >> 16) & 255) as f32 / 255.0,
                ((rgb >> 8) & 255) as f32 / 255.0,
                (rgb & 255) as f32 / 255.0,
                1.0,
            ],
            trail: Some((target, duration)),
            ..Self::simple(ParticleType::Trail)
        }
    }
    pub fn block(ty: ParticleType, sprite: String, tint: u32) -> Self {
        Self {
            block: Some((sprite, tint)),
            ..Self::simple(ty)
        }
    }
    /// `DustParticleOptions(color, scale)`.
    pub fn dust(rgb: [f32; 3], scale: f32) -> Self {
        Self {
            color: [rgb[0], rgb[1], rgb[2], 1.0],
            scale: scale.clamp(0.01, 4.0),
            ..Self::simple(ParticleType::Dust)
        }
    }
    pub fn colored(ty: ParticleType, rgba: [f32; 4]) -> Self {
        Self {
            color: rgba,
            ..Self::simple(ty)
        }
    }
    /// These options with the block or item they carry (`source`), for particle modifier
    /// filters.
    pub fn with_source(mut self, id: &str) -> Self {
        self.source = Some(Arc::from(id));
        self
    }
}

/// A burst of particles about an entity's box (`around_self`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Burst {
    pub count: u32,
    /// Scale of the Gaussian speeds; `None` spawns them at rest without drawing any.
    pub speed: Option<f64>,
    /// Added to `getRandomY()`.
    pub y: f64,
}
impl Burst {
    /// `AbstractVillager.addParticlesAroundSelf`.
    pub const VILLAGER: Self = Self {
        count: 5,
        speed: Some(0.02),
        y: 1.0,
    };
    /// `TamableAnimal`/`AbstractHorse.spawnTamingParticles`,
    /// `Ocelot.spawnTrustingParticles`, `Animal`'s entity event 18.
    pub const TAMING: Self = Self {
        count: 7,
        speed: Some(0.02),
        y: 0.5,
    };
    /// `Allay`'s entity event 18: three `spawnHeartParticle`.
    pub const ALLAY: Self = Self {
        count: 3,
        speed: Some(0.02),
        y: 0.5,
    };
    /// `Dolphin.addParticlesAroundSelf`.
    pub const DOLPHIN: Self = Self {
        count: 7,
        speed: Some(0.01),
        y: 0.2,
    };
    /// `AbstractHorse.handleEating`'s growth particle.
    pub const HORSE_EATING: Self = Self {
        count: 1,
        speed: None,
        y: 0.5,
    };
}
/// The particle bursts entity events show on the client: the particle and
/// burst of `handleEntityEvent(event)` for the entity type `entity`.
pub fn entity_event_burst(entity: &str, event: u8) -> Option<(&'static str, Burst)> {
    let entity = entity.strip_prefix("minecraft:").unwrap_or(entity);
    let tamable = matches!(
        entity,
        "wolf" | "cat" | "parrot" | "nautilus" | "zombie_nautilus"
    );
    let horse = matches!(
        entity,
        "horse"
            | "donkey"
            | "mule"
            | "llama"
            | "trader_llama"
            | "camel"
            | "camel_husk"
            | "skeleton_horse"
            | "zombie_horse"
    );
    Some(match (entity, event) {
        ("villager", 12) => ("heart", Burst::VILLAGER),
        ("villager", 13) => ("angry_villager", Burst::VILLAGER),
        ("villager", 14) => ("happy_villager", Burst::VILLAGER),
        ("villager", 42) => ("splash", Burst::VILLAGER),
        ("allay", 18) => ("heart", Burst::ALLAY),
        ("dolphin", 38) => ("happy_villager", Burst::DOLPHIN),
        ("ocelot", 41) => ("heart", Burst::TAMING),
        ("ocelot", 40) => ("smoke", Burst::TAMING),
        (_, 7) if tamable || horse => ("heart", Burst::TAMING),
        (_, 6) if tamable || horse => ("smoke", Burst::TAMING),
        (_, 18) if !matches!(entity, "villager" | "allay") => ("heart", Burst::TAMING),
        _ => return None,
    })
}

/// Particles about an entity's box, as `AbstractVillager.addParticlesAroundSelf`,
/// `Animal`'s love hearts (entity event 18), `TamableAnimal` and
/// `AbstractHorse.spawnTamingParticles`, `Ocelot.spawnTrustingParticles`,
/// `Allay`'s hearts and `Dolphin.addParticlesAroundSelf` place them: `count`
/// times three Gaussian speeds scaled by `speed` (none drawn without one), then
/// `getRandomX(1.0)`, `getRandomY() + y` and `getRandomZ(1.0)`.
pub fn around_self(
    random: &mut impl RandomSource,
    feet: [f64; 3],
    width: f64,
    height: f64,
    burst: Burst,
    mut emit: impl FnMut([f64; 3], [f64; 3]),
) {
    for _ in 0..burst.count {
        let v = burst.speed.map_or([0.0; 3], |s| {
            [
                random.next_gaussian() * s,
                random.next_gaussian() * s,
                random.next_gaussian() * s,
            ]
        });
        let p = [
            feet[0] + width * ((2.0 * random.next_double() - 1.0) * 1.0),
            feet[1] + height * random.next_double() + burst.y,
            feet[2] + width * ((2.0 * random.next_double() - 1.0) * 1.0),
        ];
        emit(p, v);
    }
}

/// `LivingEntity.spawnItemParticles(stack, count)` (eating, drinking, an item
/// breaking): `count` crumbs in front of the eyes, thrown up and forwards,
/// both turned by the pitch and yaw in degrees.
pub fn item_use_particles(
    random: &mut impl RandomSource,
    pitch: f32,
    yaw: f32,
    position: [f64; 3],
    eye_y: f64,
    count: u32,
    mut emit: impl FnMut([f64; 3], [f64; 3]),
) {
    let (x_rot, y_rot) = (-pitch * trig::DEG_TO_RAD, -yaw * trig::DEG_TO_RAD);
    for _ in 0..count {
        let d = Vec3::new(
            (f64::from(random.next_float()) - 0.5) * 0.1,
            f64::from(random.next_float()) * 0.1 + 0.1,
            0.0,
        )
        .rotate_x(x_rot)
        .rotate_y(y_rot);
        let y = f64::from(-random.next_float()) * 0.6 - 0.3;
        let p = Vec3::new((f64::from(random.next_float()) - 0.5) * 0.3, y, 0.6)
            .rotate_x(x_rot)
            .rotate_y(y_rot);
        emit(
            [p.x + position[0], p.y + eye_y, p.z + position[2]],
            [d.x, d.y + 0.05, d.z],
        );
    }
}
