//! Client-side spawner display mobs, spinning geometry, and particles.
use crate::game::ClientGame;
use std::collections::HashMap;
use sunrise_client_render::block_entity_reach::Reach;
use sunrise_core::{geometry::Vec3, random::RandomSource};
use sunrise_gameplay_common::game::native_hooks::spawner_active::GameHooks as _;
use sunrise_level::containers::BlockEntity;
use sunrise_level::particle_options::{ParticleOptions, ParticleType};
use sunrise_sim::mobs::{
    Mob,
    spawner::{SpawnDataExt, SpawnerExt},
};

type Pos = sunrise_level::block_entities::Pos;

/// What the spawner code keeps between ticks.
#[derive(Default)]
pub(in crate::game) struct SpawnerState {
    /// The mob shown in each spawner and the id it was made for
    /// (`getOrCreateDisplayEntity`).
    pub(in crate::game) displays: HashMap<Pos, (String, Mob)>,
}

impl ClientGame<'_> {
    /// The client ticks of these spawners, then the shown mobs of spawners
    /// that are gone go.
    #[allow(dead_code)]
    pub(in crate::game) fn client_tick_spawners(&mut self, keys: &[Pos]) {
        for &key in keys {
            self.client_tick_spawner(key);
        }
        self.forget_spawner_displays();
    }
    /// Drops the display mobs of spawners and trial spawners that are gone.
    pub(in crate::game) fn forget_spawner_displays(&mut self) {
        let entities = &self.common.world.block_entities;
        self.client.spawners.0.displays.retain(|k, _| {
            matches!(
                entities.get(k),
                Some(BlockEntity::Spawner(_) | BlockEntity::TrialSpawner(_))
            )
        });
    }
    /// `Mob.handleEntityEvent` 20 (`spawnAnim` on a client): the poof about
    /// mob `id`.
    pub(in crate::game) fn poof_particles(&mut self, id: u32) {
        let Some(m) = self.common.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        let (at, w, h) = (m.position, f64::from(m.width()), f64::from(m.height()));
        let options = ParticleOptions::simple(ParticleType::Poof);
        for _ in 0..20 {
            let r = self.client.particles.0.level_random();
            let xa = r.next_gaussian() * 0.02;
            let ya = r.next_gaussian() * 0.02;
            let za = r.next_gaussian() * 0.02;
            let x = at.x + w * (2.0 * r.next_double() - 1.0) - xa * 10.0;
            let y = at.y + h * r.next_double() - ya * 10.0;
            let z = at.z + w * (2.0 * r.next_double() - 1.0) - za * 10.0;
            self.client
                .particles
                .0
                .add(&options, [x, y, z], [xa, ya, za]);
        }
    }
    /// `LevelEventHandler` 2004: twenty smoke and flame pairs around the spawner.
    pub(in crate::game) fn spawner_burst(&mut self, pos: [i32; 3]) {
        let (smoke, flame) = (
            ParticleOptions::simple(ParticleType::Smoke),
            ParticleOptions::simple(ParticleType::Flame),
        );
        for _ in 0..20 {
            let r = self.client.particles.0.level_random();
            let x = f64::from(pos[0]) + 0.5 + (r.next_double() - 0.5) * 2.0;
            let y = f64::from(pos[1]) + 0.5 + (r.next_double() - 0.5) * 2.0;
            let z = f64::from(pos[2]) + 0.5 + (r.next_double() - 0.5) * 2.0;
            self.client.particles.0.add(&smoke, [x, y, z], [0.0; 3]);
            self.client.particles.0.add(&flame, [x, y, z], [0.0; 3]);
        }
    }
    /// The client copy's tick: particles and the spin while a player is near.
    pub(in crate::game) fn client_tick_spawner(&mut self, key: Pos) {
        let near = {
            let Some(BlockEntity::Spawner(s)) = self.common.world.block_entities.get(&key) else {
                return;
            };
            let range = f64::from(s.required_player_range);
            let center = [
                f64::from(key.0) + 0.5,
                f64::from(key.1) + 0.5,
                f64::from(key.2) + 0.5,
            ];
            self.common.player.as_ref().is_some_and(|p| {
                let d = (p.position.x - center[0]).powi(2)
                    + (p.position.y - center[1]).powi(2)
                    + (p.position.z - center[2]).powi(2);
                !p.health.dead_or_dying()
                    && !self.common.spectator()
                    && (range < 0.0 || d < range * range)
            })
        };
        let near = self.spawner_active(
            || mod_api::Spawner::Block {
                pos: mod_api::BlockPos::new(key.0, key.1, key.2),
            },
            near,
        );
        let display = self.update_spawner_display(key);
        let pos = [key.0, key.1, key.2];
        let random = self.client.particles.0.level_random();
        let Some(BlockEntity::Spawner(s)) = self.common.world.block_entities.get_mut(&key) else {
            return;
        };
        if let Some(p) = s.client_tick(near, display, random, pos) {
            self.client
                .particles
                .0
                .add(&ParticleOptions::simple(ParticleType::Smoke), p, [0.0; 3]);
            self.client
                .particles
                .0
                .add(&ParticleOptions::simple(ParticleType::Flame), p, [0.0; 3]);
        }
    }
    /// `getOrCreateDisplayEntity`: the mob shown in the spawner, made again
    /// when its type changes. False when there is none.
    pub(in crate::game) fn update_spawner_display(&mut self, key: Pos) -> bool {
        let Some(kind) = self.spawner_display_kind(key) else {
            self.client.spawners.0.displays.remove(&key);
            return false;
        };
        if self
            .client
            .spawners
            .0
            .displays
            .get(&key)
            .is_none_or(|(k, _)| *k != kind)
        {
            let Some(t) = self.common.res.mob_types().get(&kind).cloned() else {
                return false;
            };
            let mob = Mob::new(u32::MAX, t, Vec3::ZERO, 0.0);
            self.client.spawners.0.displays.insert(key, (kind, mob));
        }
        true
    }
    pub(in crate::game) fn client_update_spawner_display(&mut self, key: Pos) -> bool {
        self.update_spawner_display(key)
    }
    /// The type of the mob a spawner shows: its next data's id, when the game
    /// knows the mob.
    /// A trial spawner shows one only in the states with a spinning mob.
    pub(in crate::game::client_side) fn spawner_display_kind(&self, key: Pos) -> Option<String> {
        let id = match self.common.world.block_entities.get(&key)? {
            BlockEntity::Spawner(s) => s.next_data()?.id()?,
            BlockEntity::TrialSpawner(s) => {
                let block = self
                    .common
                    .pack
                    .assets
                    .registry
                    .state(self.common.world.block(key.0, key.1, key.2))?;
                if sunrise_game_data::block_entity_types::factory_block(&block.name)
                    != "minecraft:trial_spawner"
                {
                    return None;
                }
                let state = block
                    .properties
                    .get("trial_spawner_state")
                    .and_then(|v| sunrise_sim::mobs::trial_spawner::TrialState::from_name(v))
                    .unwrap_or(sunrise_sim::mobs::trial_spawner::TrialState::Inactive);
                state.spinning_mob_speed()?;
                s.next_spawn_data.as_ref()?.id()?
            }
            _ => return None,
        };
        self.common.res.mob_types().contains_key(&id).then_some(id)
    }
    /// `SpawnerRenderer` and `TrialSpawnerRenderer`: the next mob of every
    /// spawner near the camera, small and turning.
    pub(in crate::game) fn spawner_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<sunrise_client_render::render_geometry::Vertex>,
    ) {
        let atlas = &self.client.gui.atlas;
        let lookup = |name: &str| atlas.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        // A shader pack's shadow pass draws a block entity only in the sections it draws
        // (`Renderer::shadow_pass_block_entities`, #8464): a spawner's mob out of the view and
        // out of those sections isn't drawn, where it would have gone to the shadow pass alone.
        let shadow_sections =
            self.client.renderer.0.as_ref().filter(|_| {
                self.client.pack_ids.is_some() && self.client.occlusion.0.shadow_casters.0
            });
        for (key, (_, mob)) in &self.client.spawners.0.displays {
            let at = glam::DVec3::new(f64::from(key.0), f64::from(key.1), f64::from(key.2));
            if !Reach::Default.should_render([key.0, key.1, key.2], camera, 0) {
                continue;
            }
            if let Some(r) = shadow_sections
                && !r.shadow_pass_drew([key.0, key.1, key.2])
                && self.client.view_frustum.is_some_and(|v| {
                    let o = (at - camera).as_vec3();
                    !v.intersects(o - glam::Vec3::ONE, o + glam::Vec3::splat(2.0))
                })
            {
                continue;
            }
            let (spin, scale) = match self.common.world.block_entities.get(key) {
                Some(BlockEntity::Spawner(s)) => {
                    s.display_pose(alpha as f32, mob.width(), mob.height())
                }
                Some(BlockEntity::TrialSpawner(s)) => sunrise_sim::mobs::spawner::display_pose(
                    s.o_spin,
                    s.spin,
                    alpha as f32,
                    mob.width(),
                    mob.height(),
                ),
                _ => continue,
            };
            let origin = glam::Mat4::from_translation(
                (at - camera).as_vec3() + glam::Vec3::new(0.5, 0.4, 0.5),
            ) * glam::Mat4::from_rotation_y(spin.to_radians())
                * glam::Mat4::from_translation(glam::Vec3::new(0.0, -0.2, 0.0))
                * glam::Mat4::from_rotation_x((-30.0_f32).to_radians())
                * glam::Mat4::from_scale(glam::Vec3::splat(scale));
            let light = self.common.world.light(key.0, key.1, key.2);
            let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            self.mark_entity(out.len(), &mob.kind.id);
            if mob.species() == sunrise_sim::mobs::Species::SulfurCube {
                self.sulfur_cube_body_geometry(
                    mob,
                    alpha as f32,
                    sunrise_client::mobs::render::base_transform_at(mob, alpha as f32, origin),
                    packed,
                    out,
                );
            }
            sunrise_client::mobs::render::render_posed(
                mob,
                alpha as f32,
                origin,
                &lookup,
                packed,
                self,
                out,
            );
        }
    }
}
