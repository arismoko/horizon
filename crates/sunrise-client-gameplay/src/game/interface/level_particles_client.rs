use crate::game::ClientGame;
use sunrise_client::particles;
use sunrise_core::{geometry::trig, position::BlockPos, random::RandomSource};
use sunrise_items::items::ItemStack;
use sunrise_level::particle_options::{ParticleOptions, ParticleType};
use sunrise_world::level::LevelEvent;
use sunrise_world::registry::BlockStateId;

impl ClientGame<'_> {
    /// Queue and replay level event `id` at `pos` on the client.
    pub fn client_level_event_at(&mut self, id: i32, pos: [i32; 3], data: i32) {
        self.common.level_events.push(LevelEvent::LevelEvent {
            id,
            pos: BlockPos::new(pos[0], pos[1], pos[2]),
            data,
            source: None,
        });
        let _ = self.client_process_level_events();
    }
    /// `BoneMealItem.addGrowthParticles` (event 1505) on a block or its neighbours.
    pub(in crate::game::client_side) fn growth_particles(&mut self, pos: BlockPos, count: i32) {
        let p = [pos.x, pos.y, pos.z];
        let state = self.common.world.block(p[0], p[1], p[2]);
        let facts = self.common.res.facts();
        let (at, count, xz, height, floating) =
            match self.common.res.behaviours().bone_meal_spreads(state) {
                Some(spreads) => {
                    if spreads {
                        // Neighbor spreaders spark over the block above (#11310).
                        ([p[0], p[1] + 1, p[2]], count * 3, 3.0, 1.0, false)
                    } else {
                        // Growers spark in place, or below for rooted dirt and mangrove leaves.
                        let at = match &*facts.get(state).name {
                            "minecraft:rooted_dirt" | "minecraft:mangrove_leaves" => {
                                [p[0], p[1] - 1, p[2]]
                            }
                            _ => p,
                        };
                        (at, count, 0.5, self.block_shape_top(at), true)
                    }
                }
                None if &*facts.get(state).name == "minecraft:water" => {
                    (p, count * 3, 3.0, 1.0, false)
                }
                // Mod blocks default to in-block grower sparks.
                None if !facts.get(state).name.starts_with("minecraft:") => {
                    (p, count, 0.5, self.block_shape_top(p), true)
                }
                None => return,
            };
        self.spawn_particles(at, count, xz, height, floating, ParticleType::HappyVillager);
    }
    /// `BlockState.getShape(..).max(Y)` of the block at `at`, 1.0 for air.
    pub(in crate::game::client_side) fn block_shape_top(&self, at: [i32; 3]) -> f64 {
        let facts = self.common.res.facts();
        let inside = self.common.world.block(at[0], at[1], at[2]);
        if facts.is_air(inside) {
            1.0
        } else {
            facts
                .get(inside)
                .outline
                .iter()
                .map(|b| b[4])
                .fold(0.0, f64::max)
        }
    }
    /// `LevelEventHandler` 2011 and 2012: happy villager sparks inside the block.
    pub(in crate::game::client_side) fn turtle_egg_particles(&mut self, pos: BlockPos, count: i32) {
        let at = [pos.x, pos.y, pos.z];
        let height = self.block_shape_top(at);
        self.spawn_particles(at, count, 0.5, height, true, ParticleType::HappyVillager);
    }
    /// `SpeleothemBlock.isStalactite`: a speleothem whose tip points down.
    pub(in crate::game::client_side) fn is_stalactite(&self, state: BlockStateId) -> bool {
        self.common
            .res
            .facts()
            .get(state)
            .has_class("SpeleothemBlock")
            && self
                .common
                .pack
                .assets
                .registry
                .state(state)
                .and_then(|s| s.properties.get("vertical_direction"))
                .is_some_and(|d| d == "down")
    }
    /// `ClientLevel.addBreakingBlockEffects`: hit sound and mining crumb on the hit face.
    pub fn breaking_block_effects(&mut self, pos: [i32; 3], face: [i32; 3], play_sound: bool) {
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        if self.common.res.facts().is_air(state) {
            return;
        }
        if play_sound {
            self.play_mining_hit_sound(state, pos, face, true);
        }
        self.crack_particle(pos, face, state);
    }
    /// `LevelEventHandler.levelEvent` 3002 to 3005, drawing from the client
    /// level's random.
    pub(in crate::game) fn block_particle_event(&mut self, id: i32, pos: BlockPos, data: i32) {
        let ty = match id {
            3002 => ParticleType::ElectricSpark,
            3003 => ParticleType::WaxOn,
            3004 => ParticleType::WaxOff,
            3005 => ParticleType::Scrape,
            _ => return,
        };
        let p = [pos.x, pos.y, pos.z];
        let mut spawned = Vec::new();
        let random = self.client.particles.0.level_random();
        // 3002 with an axis: a lightning rod; otherwise copper cleaned by lightning.
        match usize::try_from(data) {
            Ok(axis) if id == 3002 && axis < 3 => {
                particles::along_axis(random, axis, p, (10, 19), |p, v| spawned.push((p, v)));
            }
            _ => particles::on_block_faces(random, p, |p, v| spawned.push((p, v))),
        }
        let options = ParticleOptions::simple(ty);
        for (p, v) in spawned {
            self.client.particles.0.add(&options, p, v);
        }
    }
    /// Event 2017: `TeleportRandomlyConsumeEffect`'s directional particles.
    pub(in crate::game) fn consume_teleport_particles(&mut self, pos: [i32; 3], data: i32) {
        self.client
            .particles
            .0
            .block_teleportation(pos, data, [127, 127, 127]);
    }
    pub(in crate::game) fn dragon_egg_particles(&mut self, pos: [i32; 3], data: i32) {
        self.client
            .particles
            .0
            .block_teleportation(pos, data, [16, 8, 16]);
    }
    /// `LevelEventHandler` 2006 (`DragonFireball.onHit`): 200 dragon breath
    /// particles thrown outwards, then the explosion sound when `data` is 1.
    pub(in crate::game) fn dragon_fireball_burst(&mut self, pos: BlockPos, data: i32) {
        for _ in 0..200 {
            let r = self.client.particles.0.level_random();
            let dist = r.next_float() * 4.0;
            let angle = r.next_float() * std::f32::consts::TAU;
            let vx = f64::from(trig::cos(f64::from(angle)) * dist);
            let vy = 0.01 + r.next_double() * 0.5;
            let vz = f64::from(trig::sin(f64::from(angle)) * dist);
            let options = ParticleOptions {
                scale: dist,
                ..ParticleOptions::simple(ParticleType::DragonBreath)
            };
            self.client.particles.0.add(
                &options,
                [
                    f64::from(pos.x) + vx * 0.1,
                    f64::from(pos.y) + 0.3,
                    f64::from(pos.z) + vz * 0.1,
                ],
                [vx, vy, vz],
            );
        }
        if data == 1 {
            let at = [pos.x, pos.y, pos.z].map(|v| f64::from(v) + 0.5);
            let pitch = self.common.world.random.next_float() * 0.1 + 0.9;
            self.play_audio(
                "minecraft:entity.dragon_fireball.explode",
                Some(at),
                1.0,
                pitch,
            );
        }
    }
    /// `LevelEventHandler` 1504: `PointedDripstoneBlock.spawnDripParticle` at
    /// the tip `pos`. The fluid above the stalactite's root picks the drip;
    /// with none, the dimension's default (lava in the Nether).
    pub(in crate::game) fn dripstone_drip_particle(&mut self, pos: BlockPos) {
        let tip_state = self.common.world.block(pos.x, pos.y, pos.z);
        if !self.is_stalactite(tip_state) {
            return;
        }
        // findRootBlock: up through the same stalactite, at most 11 steps.
        let tip_name = self.common.res.facts().get(tip_state).name.clone();
        let mut root = None;
        for step in 1..11 {
            let y = pos.y + step;
            if !(self.common.world.bottom_y()..self.common.world.top_y()).contains(&y) {
                break;
            }
            let state = self.common.world.block(pos.x, y, pos.z);
            if self.common.res.facts().get(state).name != tip_name {
                root = Some(y);
                break;
            }
            if !self.is_stalactite(state) {
                break;
            }
        }
        let Some(root_y) = root else {
            return;
        };
        let above = self.common.world.block(pos.x, root_y + 1, pos.z);
        let above_facts = self.common.res.facts().get(above);
        let evaporates = self.dimension_type().water_evaporates;
        let fluid = if &*above_facts.name == "minecraft:mud" && !evaporates {
            sunrise_items::blocks::Fluid::Water
        } else {
            above_facts.fluid
        };
        let ty = match fluid {
            sunrise_items::blocks::Fluid::Water => ParticleType::DrippingDripstoneWater,
            sunrise_items::blocks::Fluid::Lava => ParticleType::DrippingDripstoneLava,
            // A mod's fluid drips as no fluid above the dripstone would.
            sunrise_items::blocks::Fluid::None | sunrise_items::blocks::Fluid::Modded
                if evaporates =>
            {
                ParticleType::DrippingDripstoneLava
            }
            sunrise_items::blocks::Fluid::None | sunrise_items::blocks::Fluid::Modded => {
                ParticleType::DrippingDripstoneWater
            }
        };
        let offset = self
            .client
            .client_pack
            .context
            .mesh
            .tables
            .offset(tip_state, pos.x, pos.z)
            .unwrap_or([0.0; 3]);
        // The tip shape starts 5/16 up; the particle sits a pixel below that.
        self.client.particles.0.add(
            &ParticleOptions::simple(ty),
            [
                f64::from(pos.x) + 0.5 + offset[0],
                f64::from(pos.y) + 5.0 / 16.0 - 0.0625,
                f64::from(pos.z) + 0.5 + offset[2],
            ],
            [0.0; 3],
        );
    }
    /// `LevelEventHandler` 1503: smoke on the inserted eye.
    pub(in crate::game) fn end_portal_frame_smoke(&mut self, pos: BlockPos) {
        let options = ParticleOptions::simple(ParticleType::Smoke);
        for _ in 0..16 {
            let r = self.client.particles.0.level_random();
            let x = f64::from(pos.x) + (5.0 + r.next_double() * 6.0) / 16.0;
            let z = f64::from(pos.z) + (5.0 + r.next_double() * 6.0) / 16.0;
            self.client
                .particles
                .0
                .add(&options, [x, f64::from(pos.y) + 0.8125, z], [0.0; 3]);
        }
    }
    /// Event 2018: an enderman's portal trail.
    pub(in crate::game) fn enderman_teleport_particles(&mut self, pos: [i32; 3], data: i32) {
        self.client
            .particles
            .0
            .enderman_teleportation(pos, data, self.client.settings.particles);
    }
    /// `LevelEventHandler.globalLevelEvent`: the sound two blocks from the
    /// camera towards the event, so it is heard from anywhere.
    pub(in crate::game) fn global_level_event_sound(&mut self, id: i32, pos: [i32; 3]) {
        let (sound, volume) = match id {
            1023 => ("minecraft:entity.wither.spawn", 1.0),
            1028 => ("minecraft:entity.ender_dragon.death", 5.0),
            1038 => ("minecraft:block.end_portal.spawn", 1.0),
            _ => return,
        };
        let at = self.common.eye_position().map(|eye| {
            let d = [
                f64::from(pos[0]) + 0.5 - eye[0],
                f64::from(pos[1]) + 0.5 - eye[1],
                f64::from(pos[2]) + 0.5 - eye[2],
            ];
            let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if length < 1.0e-4 {
                return eye;
            }
            [0, 1, 2].map(|k| eye[k] + d[k] / length * 2.0)
        });
        // Each guest plays it from the global packet.
        self.play_audio_unshared(sound, at, volume, 1.0);
    }
    /// `LevelEventHandler.potionSplashParticles` (2002 with `Effect`, 2007
    /// with `InstantEffect`): eight splash potion crumbs, and a hundred
    /// spell particles in the potion's colour `data`, flung out in a disc.
    pub(in crate::game) fn potion_splash_particles(
        &mut self,
        pos: BlockPos,
        data: i32,
        instant: bool,
    ) {
        let ty = if instant {
            ParticleType::InstantEffect
        } else {
            ParticleType::Effect
        };
        // Vec3.atBottomCenterOf.
        let at = [
            f64::from(pos.x) + 0.5,
            f64::from(pos.y),
            f64::from(pos.z) + 0.5,
        ];
        let crumb = self.item_particle(&ItemStack::new("minecraft:splash_potion", 1));
        let r = self.client.particles.0.level_random();
        let crumbs: Vec<[f64; 3]> = (0..8)
            .map(|_| {
                [
                    r.next_gaussian() * 0.15,
                    r.next_double() * 0.2,
                    r.next_gaussian() * 0.15,
                ]
            })
            .collect();
        for v in crumbs {
            self.client.particles.0.add(&crumb, at, v);
        }
        let channel = |shift: i32| ((data >> shift) & 0xFF) as f32 / 255.0;
        let color = [channel(16), channel(8), channel(0)];
        for _ in 0..100 {
            let r = self.client.particles.0.level_random();
            let dist = r.next_double() * 4.0;
            let angle = r.next_double() * std::f64::consts::PI * 2.0;
            let velocity = [
                angle.cos() * dist,
                0.01 + r.next_double() * 0.5,
                angle.sin() * dist,
            ];
            let brightness = 0.75 + r.next_float() * 0.25;
            let options = ParticleOptions {
                color: [
                    color[0] * brightness,
                    color[1] * brightness,
                    color[2] * brightness,
                    1.0,
                ],
                scale: dist as f32,
                ..ParticleOptions::simple(ty)
            };
            self.client.particles.0.add(
                &options,
                [
                    at[0] + velocity[0] * 0.1,
                    at[1] + 0.3,
                    at[2] + velocity[2] * 0.1,
                ],
                velocity,
            );
        }
    }
    /// `LevelEventHandler` 1501 (lava put out) and 2009 (a wet sponge dried):
    /// eight puffs just above the block, drawn after the sound.
    pub(in crate::game) fn puffs_above_block(&mut self, pos: BlockPos, ty: ParticleType) {
        let options = ParticleOptions::simple(ty);
        for _ in 0..8 {
            let r = self.client.particles.0.level_random();
            let x = f64::from(pos.x) + r.next_double();
            let z = f64::from(pos.z) + r.next_double();
            self.client
                .particles
                .0
                .add(&options, [x, f64::from(pos.y) + 1.2, z], [0.0; 3]);
        }
    }
    /// Event 2016: `Shulker.teleportSomewhere`'s particles between the
    /// shell's old and new cells.
    pub(in crate::game) fn shulker_teleport_particles(&mut self, pos: [i32; 3], data: i32) {
        self.client
            .particles
            .0
            .block_teleportation(pos, data, [8, 8, 8]);
    }
    /// `LevelEventHandler` 3009 (a sniffer egg cracking on moss):
    /// `spawnParticlesOnBlockFaces(.., EGG_CRACK, UniformInt.of(3, 6))`.
    pub(in crate::game) fn sniffer_egg_crack_particles(&mut self, pos: BlockPos) {
        let mut spawned = Vec::new();
        particles::on_block_faces_between(
            self.client.particles.0.level_random(),
            [pos.x, pos.y, pos.z],
            3,
            6,
            |p, v| spawned.push((p, v)),
        );
        let options = ParticleOptions::simple(ParticleType::EggCrack);
        for (p, v) in spawned {
            self.client.particles.0.add(&options, p, v);
        }
    }
    /// `ParticleUtils.spawnParticles`: `count` slow particles spread over
    /// `xz` blocks around `at` and `height` up, only above solid ground
    /// unless floating ones are allowed.
    pub(in crate::game::client_side) fn spawn_particles(
        &mut self,
        at: [i32; 3],
        count: i32,
        xz: f64,
        height: f64,
        floating: bool,
        ty: ParticleType,
    ) {
        let options = ParticleOptions::simple(ty);
        for _ in 0..count.max(0) {
            let r = self.client.particles.0.level_random();
            let v = [0.0; 3].map(|_: f64| r.next_gaussian() * 0.02);
            let start = 0.5 - xz;
            let x = f64::from(at[0]) + start + r.next_double() * xz * 2.0;
            let y = f64::from(at[1]) + r.next_double() * height;
            let z = f64::from(at[2]) + start + r.next_double() * xz * 2.0;
            let below = [x.floor() as i32, y.floor() as i32 - 1, z.floor() as i32];
            if floating
                || !self
                    .common
                    .res
                    .facts()
                    .is_air(self.common.world.block(below[0], below[1], below[2]))
            {
                self.client.particles.0.add(&options, [x, y, z], v);
            }
        }
    }
    /// `LevelEventHandler` 1502 (a redstone torch burning out): five smoke
    /// puffs inside the block, drawn after the sound.
    pub(in crate::game) fn torch_burnout_smoke(&mut self, pos: BlockPos) {
        let options = ParticleOptions::simple(ParticleType::Smoke);
        for _ in 0..5 {
            let r = self.client.particles.0.level_random();
            let x = f64::from(pos.x) + r.next_double() * 0.6 + 0.2;
            let y = f64::from(pos.y) + r.next_double() * 0.6 + 0.2;
            let z = f64::from(pos.z) + r.next_double() * 0.6 + 0.2;
            self.client.particles.0.add(&options, [x, y, z], [0.0; 3]);
        }
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;
