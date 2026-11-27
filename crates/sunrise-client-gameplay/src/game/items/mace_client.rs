use crate::game::ClientGame;
use sunrise_core::{position::BlockPos, random::RandomSource};
use sunrise_gameplay_common::game::mace_common::KNOCKBACK_RADIUS;
use sunrise_level::particle_options::{ParticleOptions, ParticleType};

impl ClientGame<'_> {
    /// Level event 2013 (`ParticleUtils.spawnSmashAttackParticles`): dust
    /// pillars of the block at `pos`, a cluster over it and a ring 3.5 out.
    pub(in crate::game) fn smash_dust(&mut self, pos: BlockPos, count: i32) {
        let state = self.common.world.block(pos.x, pos.y, pos.z);
        let Some((sprite, tint)) = self.particle_sprite(state, [pos.x, pos.y, pos.z]) else {
            return;
        };
        let options = ParticleOptions::block(ParticleType::DustPillar, sprite, tint);
        let center = [
            f64::from(pos.x) + 0.5,
            f64::from(pos.y) + 1.0,
            f64::from(pos.z) + 0.5,
        ];
        let near = f64::from(0.2f32);
        let mut i = 0;
        while (i as f32) < count as f32 / 3.0 {
            let r = self.client.particles.0.random();
            let x = center[0] + r.next_gaussian() / 2.0;
            let z = center[2] + r.next_gaussian() / 2.0;
            let v = [
                r.next_gaussian() * near,
                r.next_gaussian() * near,
                r.next_gaussian() * near,
            ];
            self.client.particles.0.add(&options, [x, center[1], z], v);
            i += 1;
        }
        let far = f64::from(0.05f32);
        let mut i = 0;
        while (i as f32) < count as f32 / 1.5 {
            let r = self.client.particles.0.random();
            let angle = f64::from(i);
            let x = center[0] + KNOCKBACK_RADIUS * angle.cos() + r.next_gaussian() / 2.0;
            let z = center[2] + KNOCKBACK_RADIUS * angle.sin() + r.next_gaussian() / 2.0;
            let v = [
                r.next_gaussian() * far,
                r.next_gaussian() * far,
                r.next_gaussian() * far,
            ];
            self.client.particles.0.add(&options, [x, center[1], z], v);
            i += 1;
        }
    }
}
