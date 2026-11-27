use crate::game::ClientGame;
use sunrise_client::particles::{ExplosionParticle, ParticleLevel};
use sunrise_core::{geometry::Vec3, random::RandomSource};
use sunrise_gameplay_common::game::lan_particle_data::decode_mod_particle;
use sunrise_gameplay_common::game::lan_particle_data::particle_position_source;
use sunrise_level::particle_options::{ParticleOptions, ParticleType};
use sunrise_protocol::{
    entity_data::{Particle, ParticleData, PositionSource as NetworkPositionSource},
    packets::play::{self},
};
use sunrise_world::registry::BlockStateId;

fn rgba(argb: i32) -> [f32; 4] {
    let c = |shift: i32| ((argb >> shift) & 255) as f32 / 255.0;
    [c(16), c(8), c(0), c(24)]
}

fn rgb(c: [f32; 4]) -> [f32; 3] {
    [c[0], c[1], c[2]]
}

impl ClientGame<'_> {
    /// Client-only landing block crumbs (`particle_sprite` + local particle engine,
    /// e.g. sniffer digging).
    pub(in crate::game) fn client_landing_dust(
        &mut self,
        state: BlockStateId,
        at: Vec3,
        count: i32,
        speed: f64,
    ) {
        assert!(
            !self.is_logical_server(),
            "client_landing_dust called on server role"
        );
        let at = [at.x, at.y, at.z];
        let block = [
            at[0].floor() as i32,
            (at[1] - 0.2).floor() as i32,
            at[2].floor() as i32,
        ];
        if let Some((sprite, tint)) = self.particle_sprite(state, block) {
            self.client.particles.0.send_particles(
                &ParticleOptions::block(ParticleType::Block, sprite, tint)
                    .with_source(&self.common.res.facts().get(state).name),
                at,
                [0.0; 3],
                speed,
                count as u32,
                None,
            );
        }
    }
    /// `ClientPacketListener.handleExplosion`: the blast's particles (its
    /// block particles as the packet lists them) and its sound, when the
    /// packet carries it, then the player's push.
    pub(in crate::game::client_side) fn remote_explode(&mut self, p: &play::Explode) {
        if p.play_sound {
            let pitch = (1.0
                + (self.client.particles.0.random().next_float()
                    - self.client.particles.0.random().next_float())
                    * 0.2)
                * 0.7;
            // The packet's explosionSound: a wind charge's burst, a bed's
            // or anchor's blast, a modded sound.
            let sound = self
                .remote_sound_name(&p.sound)
                .unwrap_or_else(|| "minecraft:entity.generic.explode".to_owned());
            self.play_audio(&sound, Some(p.center), 4.0, pitch);
        }
        if let Some(options) = self.received_particle(&p.particle, p.center) {
            let level = ParticleLevel::new(
                &self.common.world,
                self.common.res.facts(),
                self.common.collision,
            );
            self.client.particles.0.add_with(
                Some(&level),
                &options,
                p.center,
                [1.0, 0.0, 0.0],
                false,
                false,
            );
        }
        // A wind charge's burst lists none, and one this client can't draw is left out.
        let block_particles: Vec<_> = p
            .block_particles
            .iter()
            .filter_map(|b| {
                Some(ExplosionParticle {
                    options: self.received_particle(&b.particle, p.center)?,
                    scaling: b.scaling,
                    speed: b.speed,
                    weight: u32::try_from(b.weight).ok()?,
                })
            })
            .collect();
        self.client.particles.0.track_explosion(
            p.center,
            p.radius,
            p.block_count.max(0) as usize,
            &block_particles,
        );
        // `Entity.pushFromExplosion` calls `Entity.push`, which rejects a
        // non-finite impulse before even marking the entity for sync.
        if let (Some([x, y, z]), Some(player)) = (p.knockback, self.common.player.as_mut()) {
            let impulse = Vec3::new(x, y, z);
            if impulse.is_finite() {
                let velocity = player.velocity.add_vector(impulse);
                // `Entity.setDeltaMovement` can reject the sum, but the push
                // still marks the entity for sync.
                if velocity.is_finite() {
                    player.velocity = velocity;
                }
                player.needs_sync = true;
            }
        }
    }
    /// `ClientPacketListener.handleParticleEvent`: no count is one particle
    /// moving by speed times spread; otherwise each is spread and sped up by
    /// gaussians on each axis.
    pub(in crate::game::client_side) fn remote_level_particles(
        &mut self,
        p: &play::LevelParticles,
    ) {
        let Some(options) = self.received_particle(&p.particle, p.pos) else {
            return;
        };
        // Combat feel Legacy draws a hit as 1.8.9 did: no sweep, no damage hearts.
        if self.pvp_hides_particle(options.ty) {
            return;
        }
        let Some(remote) = &mut self.client.remote else {
            return;
        };
        let level = ParticleLevel::new(
            &self.common.world,
            self.common.res.facts(),
            self.common.collision,
        );
        self.client.particles.0.handle_level_particles(
            &mut remote.particle_packet_random,
            Some(&level),
            p,
            &options,
        );
    }
    /// The options a server's particle stands for (None for a type or data
    /// this client doesn't draw).
    pub fn received_particle(&mut self, p: &Particle, at: [f64; 3]) -> Option<ParticleOptions> {
        if let Some(ParticleData::Mod { name, options }) = &p.data {
            return decode_mod_particle(name, options);
        }
        let name = self
            .client
            .remote
            .as_ref()?
            .builtin
            .get("minecraft:particle_type")?
            .name(p.id)?
            .to_owned();
        let ty = ParticleType::from_name(&name)?;
        Some(match &p.data {
            None => ParticleOptions::simple(ty),
            Some(ParticleData::Item(stack)) => {
                let remote = self.client.remote.as_ref()?;
                let stack = sunrise_sim::item_sync::from_network(
                    stack,
                    &remote.builtin,
                    &remote.registries,
                );
                self.item_particle_options(&stack)?
            }
            Some(ParticleData::State(s)) => self.block_provider_options(
                ty,
                BlockStateId(*s as u32),
                at.map(|v| v.floor() as i32),
            )?,
            Some(ParticleData::ColorScale(c, scale)) => {
                ParticleOptions::dust(rgb(rgba(*c)), *scale)
            }
            Some(ParticleData::ColorScale2(from, to, scale)) => {
                ParticleOptions::dust_transition(rgb(rgba(*from)), rgb(rgba(*to)), *scale)
            }
            Some(ParticleData::ColorPower(c, power)) => ParticleOptions {
                scale: *power,
                ..ParticleOptions::colored(ty, [rgba(*c)[0], rgba(*c)[1], rgba(*c)[2], 1.0])
            },
            Some(ParticleData::Color(c)) => ParticleOptions::colored(ty, rgba(*c)),
            Some(ParticleData::Scalar(v)) if ty == ParticleType::SculkCharge => {
                ParticleOptions::sculk_charge(*v)
            }
            Some(ParticleData::Scalar(v)) => ParticleOptions {
                scale: *v,
                ..ParticleOptions::simple(ty)
            },
            Some(ParticleData::Delay(d)) => ParticleOptions::shriek(*d),
            Some(ParticleData::Geyser(water_blocks)) => ParticleOptions::geyser(ty, *water_blocks),
            Some(ParticleData::GeyserBase(water_blocks, burst_impulse_base)) => {
                ParticleOptions::geyser_base(ty, *water_blocks, *burst_impulse_base)
            }
            Some(ParticleData::Trail(target, color, duration)) => {
                ParticleOptions::trail(*target, *color as u32 & 0xFF_FFFF, *duration)
            }
            Some(ParticleData::Vibration { source, ticks }) => ParticleOptions::vibration(
                particle_position_source(source),
                self.vibration_destination(source)?,
                *ticks,
            ),
            Some(_) => return None,
        })
    }
    /// `PositionSource.getPosition`: a block's centre, or an entity's feet
    /// raised by the option's offset. An entity this client does not have is
    /// no destination at all: vanilla makes the particle and removes it on its
    /// first tick, so nothing of it is ever drawn.
    pub(in crate::game) fn vibration_destination(
        &self,
        source: &NetworkPositionSource,
    ) -> Option<[f64; 3]> {
        match source {
            NetworkPositionSource::Block([x, y, z]) => Some([
                f64::from(*x) + 0.5,
                f64::from(*y) + 0.5,
                f64::from(*z) + 0.5,
            ]),
            NetworkPositionSource::Entity { id, y_offset } => {
                let id = u32::try_from(*id).ok()?;
                self.common
                    .mobs
                    .iter()
                    .find(|m| m.id == id && !m.removed)
                    .map(|m| {
                        [
                            m.position.x,
                            m.position.y + f64::from(*y_offset),
                            m.position.z,
                        ]
                    })
            }
        }
    }
}
