use super::piston_push::Lid;
use super::{Dim, entity_culling::EntityCullDecision, native_mob_draw::PlayerRender};
use sunrise_assets::item_model::DisplayContext;
use sunrise_client::{graphics::EntityCullingMode, mesher::Vertex};
use sunrise_client_render::render_geometry::EntitySpan;
use sunrise_core::{
    geometry::{Aabb, Vec3, trig::Mth},
    position::BlockPos,
    random::RandomSource,
};
use sunrise_game_data::Result;
use sunrise_gameplay_common::game::creatures::MobDrawFrame;
use sunrise_gameplay_common::game::creatures::Skinned;
use sunrise_gameplay_common::game::creatures::is_item_billboard;
use sunrise_gameplay_common::game::creatures::living_draw::LivingFrame;
use sunrise_gameplay_common::game::creatures::thrown_item_axes;
use sunrise_gameplay_common::game::creatures::thrown_item_quad;
use sunrise_gameplay_common::game::creatures::*;
use sunrise_gameplay_common::game::wither_common::skull_kind;
use sunrise_sim::{
    mobs::{Mob, MobEvent, Species, Who},
    projectiles::{Projectile, ProjectileKind},
};

#[path = "living_draw_client.rs"]
pub(in crate::game) mod living_draw;

/// `FlameFeatureRenderer`: fire quads stacked up a burning entity's box,
/// turned to the camera around Y, at full block light. `at` is the entity's
/// feet relative to the camera.
pub(in crate::game) fn flame_geometry(
    atlas: &sunrise_client::hud::HudAtlas,
    at: glam::Vec3,
    width: f32,
    height: f32,
    camera_yaw: f32,
    light: u8,
    out: &mut Vec<Vertex>,
) {
    let (Some(fire0), Some(fire1)) = (
        atlas.sprites.get("block/fire_0").copied(),
        atlas.sprites.get("block/fire_1").copied(),
    ) else {
        return;
    };
    let s = width * 1.4;
    let mut h = height / s;
    // Mth.rotationAroundAxis(Y, camera.orientation): the camera's turn about Y.
    let turn = glam::Mat3::from_rotation_y(std::f32::consts::PI - camera_yaw.to_radians());
    let lift = 0.3 - (h as i32) as f32 * 0.02;
    let packed = (u32::from(light >> 4) * 16) | ((15 * 16) << 8);
    let (mut r, mut yo, mut zo, mut ss) = (0.5f32, 0.0f32, 0.0f32, 0);
    while h > 0.0 {
        let tex = if ss % 2 == 0 { fire0 } else { fire1 };
        let (mut u0, v0, mut u1, v1) = (tex.u0, tex.v0, tex.u1, tex.v1);
        if ss / 2 % 2 == 0 {
            std::mem::swap(&mut u0, &mut u1);
        }
        for (x, y, uv) in [
            (-r, -yo, [u1, v1]),
            (r, -yo, [u0, v1]),
            (r, 1.4 - yo, [u0, v0]),
            (-r, 1.4 - yo, [u1, v0]),
        ] {
            let p = at + turn * glam::Vec3::new(x, y, zo + lift) * s;
            out.push(Vertex {
                normal: [0; 4],
                pos: p.to_array(),
                uv,
                // HDR output: the flames shine like fire (entity.wgsl fs_hdr_mark).
                layer: u32::from(sunrise_client::mesher::INCANDESCENT) << 16,
                color: 0xFFFF_FFFF,
                light: packed,

                glint_uv: [0.0, 0.0],
            });
        }
        h -= 0.45;
        yo -= 0.45;
        r *= 0.9;
        zo -= 0.03;
        ss += 1;
    }
}

impl crate::game::ClientGame<'_> {
    /// Client-side arrow particles and local sounds.
    pub(in crate::game) fn client_arrow_event(
        &mut self,
        at: Vec3,
        event: sunrise_sim::entities::arrows::ArrowEvent,
    ) {
        use sunrise_level::particle_options::{ParticleOptions, ParticleType};
        use sunrise_sim::entities::arrows::{ArrowEvent, ArrowParticle};
        let kind = |k| match k {
            ArrowParticle::Bubble => ParticleType::Bubble,
            ArrowParticle::Crit => ParticleType::Crit,
            ArrowParticle::Splash => ParticleType::Splash,
        };
        match event {
            ArrowEvent::Particle {
                kind: k,
                position,
                velocity,
            } => self.client.particles.0.add(
                &ParticleOptions::simple(kind(k)),
                [position.x, position.y, position.z],
                [velocity.x, velocity.y, velocity.z],
            ),
            ArrowEvent::Sound {
                sound,
                volume,
                pitch,
            } => self.play_audio_on(
                6,
                arrow_sound_name(sound),
                Some([at.x, at.y, at.z]),
                volume,
                pitch,
            ),
            _ => {}
        }
    }
    /// The level as remote client arrows see it (no server hooks or owner table).
    pub(in crate::game) fn client_arrow_scene<'s>(
        &'s self,
        rain: &'s dyn Fn(BlockPos) -> bool,
    ) -> sunrise_sim::entities::arrows::ArrowScene<'s, 's> {
        self.common.arrow_scene_with(rain, None, &())
    }
    pub(in crate::game) fn client_shulker_lid_push(
        &mut self,
        shulker: u32,
        area: Aabb,
        delta: [f64; 3],
    ) {
        let lid = Lid::Shulker(shulker);
        let movement = Vec3::new(delta[0], delta[1], delta[2]);
        self.lid_push_local(lid, area, movement);
    }
    pub(in crate::game) fn client_handle_mob_events_reusable(
        &mut self,
        events: &mut Vec<MobEvent>,
    ) -> Result<()> {
        self.handle_client_mob_events(events);
        Ok(())
    }
    /// A beam target's centre (half its height), interpolated.
    pub(in crate::game) fn beam_target(&self, who: Option<Who>, alpha: f64) -> Option<glam::DVec3> {
        match who? {
            Who::Player => {
                let p = self.common.player.as_ref()?;
                let (a, b) = (self.common.previous_position, p.position);
                Some(glam::DVec3::new(
                    a.x + (b.x - a.x) * alpha,
                    a.y + (b.y - a.y) * alpha + f64::from(p.dimensions.height) * 0.5,
                    a.z + (b.z - a.z) * alpha,
                ))
            }
            Who::Mob(id) => {
                let m = self.common.mobs.iter().find(|m| m.id == id)?;
                Some(glam::DVec3::new(
                    m.previous.x + (m.position.x - m.previous.x) * alpha,
                    m.previous.y
                        + (m.position.y - m.previous.y) * alpha
                        + f64::from(m.height()) * 0.5,
                    m.previous.z + (m.position.z - m.previous.z) * alpha,
                ))
            }
        }
    }
    pub fn mob_run_geometry(
        &self,
        frame: &MobDrawFrame<'_>,
        range: std::ops::Range<usize>,
        out: &mut Vec<Vertex>,
    ) {
        if frame.precheck {
            self.mob_run::<true>(&frame.living, range, out);
        } else {
            self.mob_run::<false>(&frame.living, range, out);
        }
    }
    pub(in crate::game) fn mob_run<const PRECHECK: bool>(
        &self,
        frame: &LivingFrame<'_>,
        range: std::ops::Range<usize>,
        out: &mut Vec<Vertex>,
    ) {
        for mob in &self.common.mobs[range] {
            self.living_geometry::<PRECHECK>(mob, frame, out);
        }
    }
    pub fn begin_mob_geometry(&self, camera: glam::DVec3, alpha: f64) -> MobDrawFrame<'_> {
        // The coarse mob bounds pay off only in dense async scenes; otherwise,
        // keep the existing exact mesh-culling path.
        let precheck = self.client.settings.sunrise.entity_culling
            == EntityCullingMode::Asynchronous
            && self
                .client
                .occlusion
                .0
                .async_culler
                .as_ref()
                .is_some_and(|culler| culler.camera_is_stationary())
            && self.client.settings.sunrise.entity_culling_precheck != 0
            && self.common.mobs.len()
                >= self.client.settings.sunrise.entity_culling_precheck as usize
            && !self.client.occlusion.0.shadow_casters.0
            && mod_api::__private::mods().is_empty();
        self.client.rendered_mobs.set(0);
        // What custom entity model animations read of the frame.
        sunrise_client::mobs::cem_anim::set_frame(sunrise_client::mobs::cem_anim::Frame {
            time: self.common.world.game_time as f64 + alpha,
            day_time: self.common.world.day_time as f64 + alpha,
            dimension: match self.common.dim {
                Dim::Overworld | Dim::Extra(_) => 0.0,
                Dim::Nether => -1.0,
                Dim::End => 1.0,
            },
            player: self.common.player.as_ref().map_or(camera.to_array(), |p| {
                [p.position.x, p.position.y, p.position.z]
            }),
            player_rot: [self.common.pitch, self.common.yaw],
            paused: self.common.pause,
            first_person: self.client.camera_type.is_first_person(),
            third_person_reversed: self.client.camera_type
                == sunrise_level::camera_math::CameraType::ThirdPersonFront,
            ..Default::default()
        });
        MobDrawFrame {
            precheck,
            living: LivingFrame {
                camera,
                alpha,
                scale: self.view_scale(),
                viewer: self.client_player_team(),
                skinning: self.skinning(),
                listened: self.mob_render_listened(),
                now: std::time::Instant::now(),
            },
        }
    }
    /// Mob models and projectiles for the entity pass.
    /// The renderer's models for the GPU path (`mobs::skin`) this frame, if it
    /// poses entities: a new frame of draws, the store restarted when the
    /// renderer's no longer matches. Once per frame, before any entity draws.
    pub fn begin_skinning(&self) {
        let renderer = self
            .client
            .renderer
            .0
            .as_ref()
            .filter(|r| r.skins_entities());
        match renderer.and_then(|r| r.skinned_models()) {
            Some(stored) => {
                let mut models = self.client.skin_models.0.borrow_mut();
                // Reuse helps CPU-bound crowds, but regressed TAA frame tails in
                // windowed measurements. Keep the original arithmetic with TAA on.
                models.set_rotation_cache_enabled(!self.client.settings.sunrise.taa);
                models.begin(&mut self.client.skin_frame.borrow_mut(), stored);
                models.tagged = renderer.is_some_and(|r| r.skins_tagged());
            }
            None => self.client.skin_frame.borrow_mut().clear_draws(),
        }
    }
    /// Culls one entity's vertices from `entity_start`, counting what it drew
    /// on the GPU path by the corners of `skinned`'s bounds; a culled entity's
    /// posed draws since `skinned`'s mark are taken back.
    pub(in crate::game::client_side) fn cull_skinned(
        &self,
        out: &mut Vec<Vertex>,
        entity_start: usize,
        skinned: Skinned,
    ) {
        let corners = skinned.and_then(|(_, bounds)| bounds);
        if out.len() == entity_start {
            if let Some((lo, hi)) = corners
                && self.cull_entity_camera_bounds(lo, hi)
                && let Some((mark, _)) = skinned
            {
                if self.client.occlusion.0.shadow_casters.0 {
                    let mut frame = self.client.skin_frame.borrow_mut();
                    let end = frame.vertices as usize;
                    sunrise_client::gfx::pack_draw::EntitySpan::hide(
                        &mut frame.spans,
                        mark.2 as usize,
                        end,
                    );
                } else {
                    self.client.skin_frame.borrow_mut().truncate(mark);
                }
            }
            return;
        }
        if let Some((lo, hi)) = corners {
            out.extend([lo, hi].map(|pos| Vertex {
                pos: pos.to_array(),
                ..bytemuck::Zeroable::zeroed()
            }));
        }
        let len = out.len();
        let hidden = self.cull_entity_vertices(out, entity_start);
        if out.len() < len {
            if let Some((mark, _)) = skinned {
                self.client.skin_frame.borrow_mut().truncate(mark);
            }
        } else if corners.is_some() {
            if let Some((mark, _)) = skinned {
                // Previous GPU-only entities can leave zero-length CPU spans at
                // this same offset. Use this query's result, not their metadata.
                if hidden {
                    let mut skin = self.client.skin_frame.borrow_mut();
                    let end = skin.vertices as usize;
                    EntitySpan::hide(&mut skin.spans, mark.2 as usize, end);
                }
            }
            out.truncate(len - 2);
            // The temporary bounds are not real geometry. Keep the culling end marker
            // at the true CPU end before the next entity appends a span there.
            for span in self.client.entity_spans.borrow_mut().iter_mut().rev() {
                if span.start as usize <= out.len() {
                    break;
                }
                span.start = out.len() as u32;
            }
        }
    }
    /// `LivingEntity.makePoofParticles` (entity event 60) about a mob's box.
    pub(in crate::game::client_side) fn death_poof(
        &mut self,
        at: [f64; 3],
        width: f64,
        height: f64,
    ) {
        let options = sunrise_level::particle_options::ParticleOptions::simple(
            sunrise_level::particle_options::ParticleType::Poof,
        );
        for _ in 0..20 {
            let r = self.client.particles.0.level_random();
            let xa = r.next_gaussian() * 0.02;
            let ya = r.next_gaussian() * 0.02;
            let za = r.next_gaussian() * 0.02;
            let x = at[0] + width * (2.0 * r.next_double() - 1.0) - xa * 10.0;
            let y = at[1] + height * r.next_double() - ya * 10.0;
            let z = at[2] + width * (2.0 * r.next_double() - 1.0) - za * 10.0;
            self.client
                .particles
                .0
                .add(&options, [x, y, z], [xa, ya, za]);
        }
    }
    /// `Guardian.aiStep` on a client: two bubbles a tick trailing a guardian
    /// that swims along, and bubbles up the beam to its target. The server
    /// never draws them, so the mob's own random stays untouched.
    pub(in crate::game::client_side) fn guardian_bubbles(&mut self) {
        use sunrise_level::particle_options::{ParticleOptions, ParticleType};
        use sunrise_sim::mobs::guardian;
        let mut swimming = Vec::new();
        let mut beams = Vec::new();
        for m in &self.common.mobs {
            if !m.alive() || !matches!(m.species(), Species::Guardian | Species::ElderGuardian) {
                continue;
            }
            let at = [m.position.x, m.position.y, m.position.z];
            if m.extra.flag2 && m.fluids.in_water {
                let (width, height, _) = m.dims();
                swimming.push((at, width, height, m.view_vector()));
            }
            if let Some((_, scale)) = guardian::beam(m, 0.0)
                && let Some(t) = self.beam_target(m.ai.target, 1.0)
            {
                beams.push(([at[0], m.eye_y(), at[2]], [t.x, t.y, t.z], f64::from(scale)));
            }
        }
        let bubble = ParticleOptions::simple(ParticleType::Bubble);
        for (at, width, height, view) in swimming {
            for _ in 0..2 {
                let (x, y, z) = {
                    let r = self.client.particles.0.random();
                    (r.next_double(), r.next_double(), r.next_double())
                };
                let p = [
                    at[0] + width * (2.0 * x - 1.0) * 0.5 - view[0] * 1.5,
                    at[1] + height * y - view[1] * 1.5,
                    at[2] + width * (2.0 * z - 1.0) * 0.5 - view[2] * 1.5,
                ];
                self.client.particles.0.add(&bubble, p, [0.0; 3]);
            }
        }
        for (eye, target, scale) in beams {
            let d = [target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]];
            let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if length <= 0.0 {
                continue;
            }
            let mut dist = self.client.particles.0.random().next_double();
            while dist < length {
                let r = self.client.particles.0.random();
                dist += 1.8 - scale + r.next_double() * (1.7 - scale);
                let f = dist / length;
                let p = [eye[0] + d[0] * f, eye[1] + d[1] * f, eye[2] + d[2] * f];
                self.client.particles.0.add(&bubble, p, [0.0; 3]);
            }
        }
    }
    /// Handle client-produced `MobEvent` visuals and local movement effects.
    pub fn handle_client_mob_events(&mut self, events: &mut Vec<sunrise_sim::mobs::MobEvent>) {
        use sunrise_level::particle_options::{ParticleOptions, ParticleType};
        use sunrise_sim::mobs::MobEvent;
        for event in events.drain(..) {
            match event {
                MobEvent::Sound(name, at, volume, pitch) => {
                    let id = if name.contains(':') {
                        name.to_owned()
                    } else {
                        format!("minecraft:{name}")
                    };
                    self.play_audio(&id, Some(at), volume, pitch);
                }
                MobEvent::SoundId(name, at, volume, pitch) => {
                    self.play_audio(&name, Some(at), volume, pitch);
                }
                MobEvent::ProfessionWorkSound(name, at, volume, pitch) => {
                    self.play_audio(&name, Some(at), volume, pitch);
                }
                MobEvent::LocalSound(name, at, volume, pitch) => {
                    let id = if name.contains(':') {
                        name.to_owned()
                    } else {
                        format!("minecraft:{name}")
                    };
                    self.play_audio_unshared(&id, Some(at), volume, pitch);
                }
                MobEvent::LandingDust {
                    state,
                    at,
                    count,
                    speed,
                } => self.client_landing_dust(state, Vec3::new(at[0], at[1], at[2]), count, speed),
                MobEvent::PowderSnowParticle { at, velocity } => {
                    self.client.particles.0.add(
                        &ParticleOptions::simple(ParticleType::Snowflake),
                        at,
                        velocity,
                    );
                }
                MobEvent::LevelParticles {
                    kind,
                    at,
                    spread,
                    speed,
                    count,
                } => {
                    if let Some(ty) = ParticleType::from_name(kind) {
                        let options = ParticleOptions::simple(ty);
                        self.client
                            .particles
                            .0
                            .send_particles(&options, at, spread, speed, count, None);
                    }
                }
                MobEvent::Particles { kind, at, count } => {
                    if let Some(ty) = ParticleType::from_name(kind) {
                        let options = ParticleOptions::simple(ty);
                        if count == 0 {
                            self.client.particles.0.add(&options, at, [0.0; 3]);
                        }
                        for _ in 0..count {
                            let r = &mut self.common.world.random;
                            let p = [
                                at[0] + (f64::from(r.next_float()) - 0.5),
                                at[1] + (f64::from(r.next_float()) - 0.5) * 0.5,
                                at[2] + (f64::from(r.next_float()) - 0.5),
                            ];
                            let v = [
                                r.next_gaussian() * 0.02,
                                r.next_gaussian() * 0.02,
                                r.next_gaussian() * 0.02,
                            ];
                            self.client.particles.0.add(&options, p, v);
                        }
                    }
                }
                MobEvent::ParticleAt { kind, at, velocity } => {
                    if let Some(ty) = ParticleType::from_name(kind) {
                        let options = ParticleOptions::simple(ty);
                        self.client.particles.0.add(&options, at, velocity);
                    }
                }
                MobEvent::ParticleColored {
                    at,
                    color: [r, g, b],
                } => {
                    let options =
                        ParticleOptions::colored(ParticleType::EntityEffect, [r, g, b, 1.0]);
                    self.client.particles.0.add(&options, at, [0.0; 3]);
                }
                MobEvent::DigParticles { at } => self.warden_dig_particles(at),
                MobEvent::LidPush {
                    shulker,
                    area,
                    delta,
                } => self.client_shulker_lid_push(shulker, area, delta),
                _ => {}
            }
        }
    }
    /// `Snowball`/`ThrownEgg.handleEntityEvent(3)`: eight crumbs of the
    /// thrown item where it broke, at rest for a snowball, scattered by the
    /// client entity's random (the particle engine's here) for an egg.
    pub(in crate::game::client_side) fn item_burst(&mut self, at: [f64; 3], item: &str) {
        let options = self.item_particle(&sunrise_items::items::ItemStack::new(item, 1));
        let egg = item == "minecraft:egg";
        for _ in 0..8 {
            let v = if egg {
                let r = self.client.particles.0.random();
                [
                    (f64::from(r.next_float()) - 0.5) * 0.08,
                    (f64::from(r.next_float()) - 0.5) * 0.08,
                    (f64::from(r.next_float()) - 0.5) * 0.08,
                ]
            } else {
                [0.0; 3]
            };
            self.client.particles.0.add(&options, at, v);
        }
    }
    pub(in crate::game::client_side) fn pickup_arrow_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        // ItemPickupParticle: a picked-up arrow flying to its collector.
        let taken = self
            .client
            .pickups
            .0
            .live
            .iter()
            .filter_map(|p| match &p.taken {
                super::pickups::Taken::Arrow(a) => Some((&**a, super::pickups::position(p, alpha))),
                _ => None,
            });
        self.projectile_geometry(camera, alpha, taken, out);
    }
    pub(in crate::game) fn projectile_geometry<'a>(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        projectiles: impl Iterator<Item = (&'a Projectile, [f64; 3])>,
        out: &mut Vec<Vertex>,
    ) {
        let atlas = &self.client.gui.atlas;
        let (right, up) =
            thrown_item_axes(self.client.camera_type, self.common.yaw, self.common.pitch);
        // A glowing projectile's outline: what it drew up to the next one.
        let mut glowing = None;
        for (a, at) in projectiles {
            if let Some((start, rgb)) = glowing.take() {
                self.mark_outline(&out[start..], rgb);
            }
            self.mark_entity(out.len(), a.entity_type());
            if a.id != 0
                && let Some(rgb) = self.thing_outline(a.id, a.kind.id())
            {
                glowing = Some((out.len(), rgb));
            }
            let p = glam::DVec3::from_array(at) - camera;
            let c = p.as_vec3();
            // EntityRenderer.getPackedLightCoords: the block its light probe
            // (the eye) is in, at the position this frame draws rather than
            // this tick's, and not the one under its feet.
            let light = self.common.world.light(
                at[0].floor() as i32,
                a.light_probe_y(alpha).floor() as i32,
                at[2].floor() as i32,
            );
            let mut packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            // EntityRenderer.getBlockLightLevel: a shulker bullet and anything
            // on fire are drawn at 15 block light (the fireball renderers'
            // own override); the sky light stays the level's.
            if let Some(block) = a.block_light_override() {
                packed = (packed & 0xFF) | (u32::from(block) * 16) << 8;
            }
            if a.is_arrow() {
                let Some(model) = self.client.arrow_model.as_ref() else {
                    continue;
                };
                // SpectralArrowRenderer, and TippableArrowRenderer's `getColor() > 0`.
                let texture = if a.is_spectral_arrow() {
                    "entity/projectiles/arrow_spectral"
                } else if a.arrow.as_ref().is_some_and(|s| s.effect_color > 0) {
                    "entity/projectiles/arrow_tipped"
                } else {
                    "entity/projectiles/arrow"
                };
                let Some(r) = atlas.sprites.get(texture).copied() else {
                    continue;
                };
                // ArrowRenderer: `Mth.lerp(partialTick, state.yRotO, state.yRot)`
                // and the same for the pitch, so a turning arrow is drawn
                // between the two ticks it turned over.
                let alpha32 = alpha as f32;
                let yaw = a.previous_yaw + alpha32 * (a.yaw - a.previous_yaw);
                let pitch = a.previous_pitch + alpha32 * (a.pitch - a.previous_pitch);
                let m = glam::Mat4::from_translation(c)
                    * glam::Mat4::from_rotation_y((yaw - 90.0).to_radians())
                    * glam::Mat4::from_rotation_z(pitch.to_radians());
                // ArrowRenderer: `state.shake = shakeTime - partialTicks`; ArrowModel.setupAnim
                // rolls the root while the shake timer runs.
                let shake = a
                    .arrow
                    .as_ref()
                    .map_or(0.0, |s| s.shake as f32 - alpha as f32);
                let roll = if shake > 0.0 {
                    -(shake * 3.0).msin() * shake * (std::f32::consts::PI / 180.0)
                } else {
                    0.0
                };
                let pose_for = |part: &sunrise_client::mobs::Part| {
                    let mut pose = part.pose;
                    if std::ptr::eq(part, model) {
                        pose[5] += roll;
                    }
                    pose
                };
                sunrise_client::mobs::render_model(
                    model,
                    m,
                    &pose_for,
                    [r.u0, r.v0, r.u1, r.v1],
                    packed,
                    0,
                    out,
                );
                if a.arrow.as_ref().is_some_and(|s| s.on_fire()) {
                    flame_geometry(atlas, c, 0.5, 0.5, self.common.yaw, light, out);
                }
                continue;
            }
            // ThrownTridentRenderer.
            if matches!(a.kind, ProjectileKind::Trident) {
                let Some(model) = self.client.trident_model.as_ref() else {
                    continue;
                };
                let Some(r) = atlas.sprites.get("entity/trident/trident").copied() else {
                    continue;
                };
                let m = glam::Mat4::from_translation(c)
                    * glam::Mat4::from_rotation_y((a.yaw - 90.0).to_radians())
                    * glam::Mat4::from_rotation_z((a.pitch + 90.0).to_radians());
                let poses = sunrise_sim::mobs::anim::Poses::new(model);
                sunrise_client::mobs::render::draw(
                    model,
                    m,
                    &poses,
                    [r.u0, r.v0, r.u1, r.v1],
                    0xFFFF_FFFF,
                    packed,
                    0,
                    out,
                );
                continue;
            }
            // WindChargeRenderer.
            if matches!(
                a.kind,
                ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge
            ) {
                self.wind_charge_geometry(a, c, packed, alpha as f32, out);
                continue;
            }
            // ShulkerBulletRenderer: the tumbling bullet model, then a larger
            // translucent copy of it (colour 0x26FFFFFF); always 15 block light.
            if matches!(a.kind, ProjectileKind::ShulkerBullet { .. }) {
                let model = &self.client.entity_models.shulker_bullet;
                if let (Some(model), Some(r)) =
                    (model, atlas.sprites.get("entity/shulker/spark").copied())
                {
                    use sunrise_core::geometry::trig;
                    let tc = a.age as f32 + alpha as f32;
                    let mut poses = sunrise_sim::mobs::anim::Poses::new(model);
                    poses.yrot("main", a.yaw.to_radians());
                    poses.xrot("main", a.pitch.to_radians());
                    let light = (packed & 0xFF) | ((15 * 16) << 8);
                    let base = glam::Mat4::from_translation(c)
                        * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.15, 0.0))
                        * glam::Mat4::from_rotation_y(
                            (trig::sin(f64::from(tc * 0.1)) * 180.0).to_radians(),
                        )
                        * glam::Mat4::from_rotation_x(
                            (trig::cos(f64::from(tc * 0.1)) * 180.0).to_radians(),
                        )
                        * glam::Mat4::from_rotation_z(
                            (trig::sin(f64::from(tc * 0.15)) * 360.0).to_radians(),
                        )
                        * glam::Mat4::from_scale(glam::Vec3::new(-0.5, -0.5, 0.5));
                    let uv = [r.u0, r.v0, r.u1, r.v1];
                    sunrise_client::mobs::render::draw(
                        model,
                        base,
                        &poses,
                        uv,
                        0xFFFF_FFFF,
                        light,
                        0,
                        out,
                    );
                    // The translucent shell blends over the opaque core
                    // (`RenderTypes.entityTranslucent`).
                    let shell = out.len();
                    sunrise_client::mobs::render::draw(
                        model,
                        base * glam::Mat4::from_scale(glam::Vec3::splat(1.5)),
                        &poses,
                        uv,
                        0x26FF_FFFF,
                        light,
                        0,
                        out,
                    );
                    sunrise_client::gfx::mark_oit(&mut out[shell..]);
                }
                continue;
            }
            // WitherSkullRenderer: the skull model facing its flight.
            if let Some(dangerous) = skull_kind(a) {
                let model = &self.client.entity_models.wither_skull;
                let tex = if dangerous {
                    "entity/wither/wither_invulnerable"
                } else {
                    "entity/wither/wither"
                };
                if let (Some(model), Some(r)) = (model, atlas.sprites.get(tex).copied()) {
                    let mut poses = sunrise_sim::mobs::anim::Poses::new(model);
                    poses.yrot("head", a.yaw.to_radians());
                    poses.xrot("head", a.pitch.to_radians());
                    let base = glam::Mat4::from_translation(c)
                        * glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0));
                    sunrise_client::mobs::render::draw(
                        model,
                        base,
                        &poses,
                        [r.u0, r.v0, r.u1, r.v1],
                        0xFFFF_FFFF,
                        packed,
                        0,
                        out,
                    );
                }
                continue;
            }
            if self.thrown_item_geometry(a, alpha, at, camera, c, packed, out) {
                continue;
            }
            // `Entity.displayFireAnimation`: a thrown item that flew through a
            // fire block burns (`ThrowableProjectile.tick`'s
            // `applyEffectsFromBlocks`), and ThrownItemRenderer draws the flame
            // over its box as any other entity renderer does.
            if a.on_fire() {
                let size = a.kind.size() as f32;
                flame_geometry(atlas, c, size, size, self.common.yaw, light, out);
            }
            // `ThrownItemRenderer` for a fireball: the item `Fireball.getItem`
            // returns (a fire charge until `setItem` gave it another), its model
            // at the renderer's scale for the kind. `packed` is already 15, the
            // renderer's full bright.
            if let Some(stack) = a.item.as_ref() {
                let owner = super::item_draw::owner_at(at, 0.0, a.kind.id());
                let state = self.resolve_item(stack, DisplayContext::Ground, Some(&owner), 0);
                let pose = glam::Mat4::from_translation(c)
                    * glam::Mat4::from_quat(self.camera_orientation())
                    * glam::Mat4::from_scale(glam::Vec3::splat(a.sprite_scale()));
                self.submit_item_world(&state, pose, packed, out);
                continue;
            }
            let Some(name) = a.item_sprite() else {
                continue;
            };
            let Some(r) = atlas.sprites.get(name).copied() else {
                continue;
            };
            let lift = is_item_billboard(&a.kind);
            let quad = thrown_item_quad(c, right, up, a.sprite_scale(), lift);
            for (v, uv) in quad
                .iter()
                .zip([[r.u0, r.v1], [r.u1, r.v1], [r.u1, r.v0], [r.u0, r.v0]])
            {
                out.push(Vertex {
                    normal: [0; 4],
                    pos: v.to_array(),
                    uv,
                    layer: 0,
                    color: 0xFFFF_FFFF,
                    light: packed,

                    glint_uv: [0.0, 0.0],
                });
            }
        }
        if let Some((start, rgb)) = glowing {
            self.mark_outline(&out[start..], rgb);
        }
    }
    /// Whether this frame's models can be posed on the GPU (`begin_skinning`).
    pub(in crate::game::client_side) fn skinning(&self) -> bool {
        self.client
            .renderer
            .0
            .as_ref()
            .is_some_and(|r| r.skins_entities() && r.skinned_models().is_some())
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn mob_geometry(&self, camera: glam::DVec3, alpha: f64, out: &mut Vec<Vertex>) {
        let frame = self.begin_mob_geometry(camera, alpha);
        self.mob_run_geometry(&frame, 0..self.common.mobs.len(), out);
        self.mark_motion(out.len(), 0);
        self.other_creature_geometry(camera, alpha, out);
    }
    pub(in crate::game) fn other_creature_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        self.elder_apparition_geometry(alpha, out);
        self.spawner_geometry(camera, alpha, out);
        let live = self
            .common
            .creatures
            .projectiles
            .iter()
            .filter(|a| !self.hides_as_camera(a.id))
            .map(|a| {
                let at = [
                    a.previous.x + (a.position.x - a.previous.x) * alpha,
                    a.previous.y + (a.position.y - a.previous.y) * alpha,
                    a.previous.z + (a.position.z - a.previous.z) * alpha,
                ];
                (a, at)
            });
        self.projectile_geometry(camera, alpha, live, out);
    }
}
