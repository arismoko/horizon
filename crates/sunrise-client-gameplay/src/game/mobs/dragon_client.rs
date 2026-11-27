use crate::game::ClientGame;
use crate::game::client_side::api_values_client::mod_bar_value;
use sunrise_client::{hud::HudBatch, hud_layout, hud_overlays, mesher::Vertex};
use sunrise_client_gameplay::dragon_render::{dissolve_color, dissolve_mask, dragon_rays};
use sunrise_core::{
    geometry::{Aabb, Vec3, trig},
    random::{LegacyRandom, RandomSource},
};
use sunrise_gameplay_common::game::dragon_common::{
    Dragon, PART_SIZES, Phase, dragon_part_id, dragon_part_index, saved_dragon_phase, wrap,
};
use sunrise_level::particle_options::{ParticleOptions, ParticleType};
use sunrise_protocol::entity_data::Value;

/// `DragonLandingPhase.doClientTick` and `DragonSittingFlamingPhase.doClientTick`:
/// the dragon-breath particles one tick of `doClientTick` spawns at the head, as
/// (position, velocity) pairs. `look` is `EnderDragon.getHeadLookVector(1)`
/// normalised, `head` the head part's centre (`getY(0.5)`), `motion` the
/// dragon's `getDeltaMovement`, and `random` the stream the jitter comes from.
/// Both phases rotate `look` by a quarter turn and throw the result away, so
/// every puff of a fan leaves along the same line; a flaming dragon walks down
/// that line in six steps.
pub(in crate::game) fn breath_puffs(
    phase: &Phase,
    look: [f64; 3],
    head: [f64; 3],
    motion: [f64; 3],
    random: &mut LegacyRandom,
) -> Vec<([f64; 3], [f64; 3])> {
    // A puff's start: the head's centre jittered by `nextGaussian() / 2`.
    let jitter = |random: &mut LegacyRandom| {
        [
            head[0] + random.next_gaussian() / 2.0,
            head[1] + random.next_gaussian() / 2.0,
            head[2] + random.next_gaussian() / 2.0,
        ]
    };
    match phase {
        // Eight a tick, along `-look * 0.08F` (0.3 for the vertical part) plus
        // the dragon's own motion.
        Phase::Landing { .. } => (0..8)
            .map(|_| {
                (
                    jitter(random),
                    [
                        -look[0] * f64::from(0.08_f32) + motion[0],
                        -look[1] * f64::from(0.3_f32) + motion[1],
                        -look[2] * f64::from(0.08_f32) + motion[2],
                    ],
                )
            })
            .collect(),
        // Every second tick of the first ten: eight starts, each a fan of six
        // puffs down the look line, the vertical part of the speed once.
        Phase::Flaming { ticks } if ticks % 2 == 0 && *ticks < 10 => {
            let mut puffs = Vec::with_capacity(48);
            for _ in 0..8 {
                let at = jitter(random);
                for j in 0..6 {
                    let j = f64::from(j);
                    puffs.push((
                        at,
                        [
                            -look[0] * f64::from(0.08_f32) * j,
                            -look[1] * f64::from(0.6_f32),
                            -look[2] * f64::from(0.08_f32) * j,
                        ],
                    ));
                }
            }
            puffs
        }
        _ => Vec::new(),
    }
}

/// Detached GUI dragon state, without dragon parts or entity seeds.
#[derive(Clone, Copy, Debug)]
pub(in crate::game) struct DragonPreview {
    flap: f32,
    sitting: bool,
    hurt_time: i32,
}

impl DragonPreview {
    pub(in crate::game) fn from_dragon(d: &Dragon) -> Self {
        Self {
            flap: d.flap,
            sitting: d.phase.sitting(),
            hurt_time: d.hurt_time,
        }
    }

    /// Build a detached preview from saved dragon NBT, discarding position and orientation.
    pub(in crate::game) fn from_nbt(data: &sunrise_core::nbt::Compound) -> Self {
        let phase =
            saved_dragon_phase(data.get(&sunrise_core::nbt::JavaString::from("DragonPhase")))
                .unwrap_or(Phase::Hover { target: None });
        Self {
            flap: 0.0,
            sitting: phase.sitting(),
            hurt_time: 0,
        }
    }
}

/// GUI dragon body yaw, relative head yaw, and head pitch in degrees.
#[derive(Clone, Copy, Debug, Default)]
pub(in crate::game) struct GuiDragonLook {
    pub body_yaw: f32,
    pub head_yaw: f32,
    pub head_pitch: f32,
}

impl ClientGame<'_> {
    /// The boss bars' draws on a `w` x `h` GUI.
    pub fn boss_bar_draws(&self, w: i32, h: i32) -> Vec<hud_layout::GuiDraw> {
        let now = sunrise_client::boss_overlay::now_millis();
        let bars = self.client.boss_overlay.bars(now);
        // NeoForge's `CustomizeGuiOverlayEvent.BossEventProgress`, asked per
        // bar only while a mod listens.
        let customized = mod_api::__private::listened(mod_api::HookId::CustomizeBossEventProgress);
        hud_overlays::boss_overlay(w, h, &bars, &self.client.gui.atlas.font, |index, at| {
            let Some(bar) = customized
                .then(|| self.client.boss_overlay.mod_bar(index, now))
                .flatten()
            else {
                return true;
            };
            let Ok(bar) = mod_bar_value(&bar) else {
                return true;
            };
            let mut cancel = false;
            self.native_customize_boss_event_progress(
                &bar,
                &mut at.x,
                &mut at.y,
                &mut at.increment,
                &mut cancel,
            );
            !cancel
        })
    }
    /// `BossHealthOverlay.shouldPlayMusic`.
    pub(in crate::game::client_side) fn boss_music(&self) -> bool {
        self.client.boss_overlay.should_play_music()
    }
    /// `EnderDragonRenderer.submit` and `EnderDragonModel.setupAnim`, from the real dragon's slot.
    pub(in crate::game::client_side) fn dragon_geometry(
        &self,
        d: &Dragon,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
        additive: &mut Vec<Vertex>,
    ) {
        self.dragon_geometry_as::<false>(
            Some(d),
            DragonPreview::from_dragon(d),
            (camera, alpha),
            GuiDragonLook::default(),
            out,
            additive,
        );
    }

    /// Draws the full-bright GUI dragon body and additive eyes at the supplied orientation.
    #[inline(always)]
    pub(in crate::game) fn gui_dragon_geometry(
        &self,
        preview: DragonPreview,
        look: GuiDragonLook,
        out: &mut Vec<Vertex>,
        eyes: &mut Vec<Vertex>,
    ) {
        self.dragon_geometry_as::<true>(None, preview, (glam::DVec3::ZERO, 1.0), look, out, eyes);
    }

    /// `dragon_geometry` specialised by `GUI` so world draws drop GUI branches and add no slot.
    #[inline(always)]
    fn dragon_geometry_as<const GUI: bool>(
        &self,
        world: Option<&Dragon>,
        preview: DragonPreview,
        view: (glam::DVec3, f64),
        gui: GuiDragonLook,
        out: &mut Vec<Vertex>,
        additive: &mut Vec<Vertex>,
    ) {
        let (camera, alpha) = view;
        let Some(model) = self.client.dragon_model.as_ref() else {
            return;
        };
        let a = alpha as f32;
        let sprite = |n: &str| {
            self.client
                .gui
                .atlas
                .sprites
                .get(n)
                .map(|r| [r.u0, r.v0, r.u1, r.v1])
        };
        let (Some(skin), Some(eyes)) = (
            sprite("entity/enderdragon/dragon"),
            sprite("entity/enderdragon/dragon_eyes"),
        ) else {
            return;
        };
        let pos = match world {
            Some(d) if !GUI => {
                let p = d.previous.lerp(d.position, alpha);
                glam::DVec3::new(p.x, p.y, p.z)
            }
            _ => glam::DVec3::ZERO,
        };
        let h = |delay: usize| match world {
            Some(d) if !GUI => d.sample_lerp(delay, a),
            _ => (0.0, gui.body_yaw),
        };
        let yr = h(7).1;
        let rot2 = (h(5).0 - h(10).0) as f32;
        let mut base = glam::Mat4::from_translation((pos - camera).as_vec3())
            * glam::Mat4::from_rotation_y((-yr).to_radians())
            * glam::Mat4::from_rotation_x((rot2 * 10.0).to_radians())
            * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.0, 1.0))
            * glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0))
            * glam::Mat4::from_translation(glam::Vec3::new(0.0, -1.501, 0.0));
        let rays_base = base * glam::Mat4::from_translation(glam::Vec3::new(0.0, -1.0, -2.0));
        // setupAnim.
        let flap = match world {
            Some(d) if !GUI => (d.old_flap + (d.flap - d.old_flap) * a) * std::f32::consts::TAU,
            _ => preview.flap * std::f32::consts::TAU,
        };
        let mut bounce = (flap - 1.0).sin() + 1.0;
        bounce = (bounce * bounce + bounce * 2.0) * 0.05;
        base *=
            glam::Mat4::from_translation(glam::Vec3::new(0.0, (bounce - 2.0) * 16.0, -48.0) / 16.0)
                * glam::Mat4::from_rotation_x((bounce * 2.0).to_radians());
        let mut poses = sunrise_client::mobs::anim::Poses::new(model);
        poses.get("jaw")[3] = (flap.sin() + 1.0) * 0.2;
        let sitting = preview.sitting;
        let landing = match world {
            Some(d) if !GUI => matches!(d.phase, Phase::Landing { .. } | Phase::Takeoff { .. }),
            _ => false,
        };
        let egg_dist = match world {
            Some(d) if !GUI => {
                let p = self.common.podium_top();
                ((p.x - d.position.x).powi(2)
                    + (p.y - d.position.y).powi(2)
                    + (p.z - d.position.z).powi(2))
                .sqrt()
            }
            _ => 0.0,
        };
        let head_offset = |part: usize, start: (f64, f32), point: (f64, f32)| -> f32 {
            if landing {
                part as f32 / (egg_dist / 4.0).max(1.0) as f32
            } else if sitting {
                part as f32
            } else if part == 6 {
                0.0
            } else {
                (point.0 - start.0) as f32
            }
        };
        let start = h(6);
        let rot2y = wrap(h(5).1 - h(10).1);
        let rot = wrap(h(5).1 + rot2y / 2.0);
        let deg = std::f32::consts::PI / 180.0;
        let (mut xx, mut yy, mut zz) = (0.0f32, 20.0f32, -12.0f32);
        for i in 0..5 {
            let point = h(5 - i);
            let pose = poses.get(&format!("neck{i}"));
            let neck_x = (i as f32 * 0.45 + flap).cos() * 0.15;
            pose[4] = wrap(point.1 - start.1) * deg * 1.5;
            pose[3] = neck_x + head_offset(i, start, point) * deg * 1.5 * 5.0;
            pose[5] = -wrap(point.1 - rot) * deg * 1.5;
            pose[0] = xx;
            pose[1] = yy;
            pose[2] = zz;
            let (xr, yrr) = (pose[3], pose[4]);
            xx -= yrr.sin() * xr.cos() * 10.0;
            yy += xr.sin() * 10.0;
            zz -= yrr.cos() * xr.cos() * 10.0;
        }
        let head = poses.get("head");
        head[0] = xx;
        head[1] = yy;
        head[2] = zz;
        if GUI {
            head[4] = wrap(gui.head_yaw) * deg;
            head[3] = gui.head_pitch * deg;
            head[5] = 0.0;
        } else {
            let current = h(0);
            head[4] = wrap(current.1 - start.1) * deg;
            head[3] = wrap(head_offset(6, start, current)) * deg * 1.5 * 5.0;
            head[5] = -wrap(current.1 - rot) * deg;
        }
        poses.get("body")[5] = -rot2y * 1.5 * deg;
        let lw = [0.125 - flap.cos() * 0.2, -0.25, -(flap.sin() + 0.125) * 0.8];
        let lt = ((flap + 2.0).sin() + 0.5) * 0.75;
        {
            let p = poses.get("left_wing");
            (p[3], p[4], p[5]) = (lw[0], lw[1], lw[2]);
        }
        poses.get("left_wing_tip")[5] = lt;
        {
            let p = poses.get("right_wing");
            (p[3], p[4], p[5]) = (lw[0], -lw[1], -lw[2]);
        }
        poses.get("right_wing_tip")[5] = -lt;
        for side in ["left", "right"] {
            poses.get(&format!("{side}_hind_leg"))[3] = 1.0 + bounce * 0.1;
            poses.get(&format!("{side}_hind_leg_tip"))[3] = 0.5 + bounce * 0.1;
            poses.get(&format!("{side}_hind_foot"))[3] = 0.75 + bounce * 0.1;
            poses.get(&format!("{side}_front_leg"))[3] = 1.3 + bounce * 0.1;
            poses.get(&format!("{side}_front_leg_tip"))[3] = -0.5 - bounce * 0.1;
            poses.get(&format!("{side}_front_foot"))[3] = 0.75 + bounce * 0.1;
        }
        let mut tail_x = 0.0f32;
        let (mut xx, mut yy, mut zz) = (0.0f32, 10.0f32, 60.0f32);
        let start = h(11);
        for i in 0..12 {
            let point = h(12 + i);
            tail_x += (i as f32 * 0.45 + flap).sin() * 0.05;
            let p = poses.get(&format!("tail{i}"));
            p[4] = (wrap(point.1 - start.1) * 1.5 + 180.0) * deg;
            p[3] = tail_x + (point.0 - start.0) as f32 * deg * 1.5 * 5.0;
            p[5] = wrap(point.1 - rot) * deg * 1.5;
            p[0] = xx;
            p[1] = yy;
            p[2] = zz;
            let (xr, yrr) = (p[3], p[4]);
            yy += xr.sin() * 10.0;
            zz -= yrr.cos() * xr.cos() * 10.0;
            xx -= yrr.sin() * xr.cos() * 10.0;
        }
        let light = match world {
            Some(d) if !GUI => {
                let l = self.common.world.light(
                    d.position.x.floor() as i32,
                    d.position.y.floor() as i32,
                    d.position.z.floor() as i32,
                );
                (u32::from(l >> 4) * 16) | ((u32::from(l & 15) * 16) << 8)
            }
            // `renderEntityInInventory`: LightTexture.FULL_BRIGHT.
            _ => sunrise_client::mobs::render::FULL_BRIGHT,
        };
        let death = match world {
            Some(d) if !GUI && d.death_time > 0 => d.death_time as f32 + a,
            _ => 0.0,
        };
        // Dying: DYING_RENDER_TYPE, `entityCutoutDissolve` of the skin with dragon_exploding as
        // the mask, white at `ARGB.white(1 - deathTime / 200)` and no overlay.
        let mask = sprite("entity/enderdragon/dragon_exploding").filter(|_| death > 0.0);
        let (color, overlay) = if mask.is_some() {
            (dissolve_color(death), sunrise_client::gfx::DISSOLVE_OVERLAY)
        } else {
            (0xFFFF_FFFF, u32::from(preview.hurt_time > 0))
        };
        let first = out.len();
        if !GUI {
            self.mark_entity(first, "minecraft:ender_dragon");
            self.mark_motion(first, 0);
        }
        sunrise_client::mobs::render::draw(model, base, &poses, skin, color, light, overlay, out);
        if let Some(mask) = mask {
            dissolve_mask(&mut out[first..], skin, mask);
        }
        // The skin's OptiFine emissive overlay, full bright.
        let glow = self
            .client_res()
            .client_mob_data()
            .emissive
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .overlay("entity/enderdragon/dragon")
            .and_then(sprite);
        if let Some(glow) = glow.filter(|_| mask.is_none()) {
            let full = sunrise_client::mobs::render::FULL_BRIGHT;
            sunrise_client::mobs::render::draw(model, base, &poses, glow, color, full, 0, out);
        }
        #[cfg(test)]
        if let Some(d) = world
            && !GUI
        {
            super::native_render_pair_tests::witness_body(d.id, first, out.len());
        }
        // RenderTypes.eyes: the glowing eyes, added on top.
        let first_eye = additive.len();
        sunrise_client::mobs::render::draw(
            model,
            base,
            &poses,
            eyes,
            0xFFFF_FFFF,
            240 | 240 << 8,
            0,
            additive,
        );
        // `RenderTypes.eyes` has an `OitPipelineSet`: improved transparency
        // moves the tagged quads into its stages (the HDR tier is the eyes'
        // shine, `mobs::render::EYES_TIER`).
        if GUI {
            return;
        }
        for v in &mut additive[first_eye..] {
            v.layer = 2
                | sunrise_client::gfx::OIT_ENTITY
                | u32::from(sunrise_client::mesher::BRIGHT) << 16;
        }
        if death > 0.0 {
            let rays = additive.len();
            self.dragon_rays(rays_base, death / 200.0, additive);
            for v in &mut additive[rays..] {
                v.layer |= u32::from(sunrise_client::mesher::VERY_BRIGHT) << 16;
            }
        }
        // The healing beam to the nearest crystal.
        if let Some(d) = world
            && let Some(c) = d.nearest_crystal.and_then(|i| self.common.crystals.get(i))
        {
            let bob = f64::from(sunrise_client::end_crystal::y(c.time as f32 + a));
            let to = glam::DVec3::new(c.position.x, c.position.y + bob, c.position.z);
            self.crystal_beam(pos, to - pos, self.visual_ticks() as f32 + a, camera, out);
        }
    }
    /// `submitCrystalBeams`: an eight-sided scrolling beam tube from `from` (+2 up) along `delta`.
    pub(in crate::game) fn crystal_beam(
        &self,
        from: glam::DVec3,
        delta: glam::DVec3,
        time: f32,
        camera: glam::DVec3,
        out: &mut Vec<Vertex>,
    ) {
        let Some(s) = self
            .client
            .gui
            .atlas
            .sprites
            .get("entity/end_crystal/end_crystal_beam")
            .copied()
        else {
            return;
        };
        // Like the other layer-6 beams, this is a separate pack material from its body.
        self.mark_stage(out.len(), "NONE");
        let q = |v: f32| (v * 65535.0).round().clamp(0.0, 65535.0) as u32;
        let wrap_light = q(s.v0) | q(s.v1 - s.v0) << 16;
        let (dx, dy, dz) = (delta.x as f32, delta.y as f32, delta.z as f32);
        let horizontal = (dx * dx + dz * dz).sqrt();
        let length = (dx * dx + dy * dy + dz * dz).sqrt();
        let m = glam::Mat4::from_translation(
            (from - camera).as_vec3() + glam::Vec3::new(0.0, 2.0, 0.0),
        ) * glam::Mat4::from_rotation_y(-dz.atan2(dx) - std::f32::consts::FRAC_PI_2)
            * glam::Mat4::from_rotation_x(-horizontal.atan2(dy) - std::f32::consts::FRAC_PI_2);
        let (v0, v1) = (-time * 0.01, length / 32.0 - time * 0.01);
        let (mut last_sin, mut last_cos, mut last_u) = (0.0f32, 0.75f32, 0.0f32);
        let u = |x: f32| s.u0 + (s.u1 - s.u0) * x;
        for i in 1..=8 {
            let a = i as f32 * std::f32::consts::TAU / 8.0;
            let (sin, cos) = (a.sin() * 0.75, a.cos() * 0.75);
            let uu = i as f32 / 8.0;
            for (p, uv, c) in [
                (
                    glam::Vec3::new(last_sin * 0.2, last_cos * 0.2, 0.0),
                    [u(last_u), v0],
                    0xFF00_0000,
                ),
                (
                    glam::Vec3::new(last_sin, last_cos, length),
                    [u(last_u), v1],
                    0xFFFF_FFFF,
                ),
                (glam::Vec3::new(sin, cos, length), [u(uu), v1], 0xFFFF_FFFF),
                (
                    glam::Vec3::new(sin * 0.2, cos * 0.2, 0.0),
                    [u(uu), v0],
                    0xFF00_0000,
                ),
            ] {
                out.push(Vertex {
                    normal: [0; 4],
                    pos: m.transform_point3(p).to_array(),
                    uv,
                    // HDR output: the healing beam shines (entity.wgsl fs_hdr_mark).
                    layer: 6 | u32::from(sunrise_client::mesher::BRIGHT) << 16,
                    color: c,
                    light: wrap_light,

                    glint_uv: [0.0, 0.0],
                });
            }
            (last_sin, last_cos, last_u) = (sin, cos, uu);
        }
    }
    /// Dragon fireballs as the dragon_fireball sprite facing the camera.
    pub(in crate::game) fn dragon_fireball_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        let Some(s) = self
            .client
            .gui
            .atlas
            .sprites
            .get("entity/enderdragon/dragon_fireball")
            .copied()
        else {
            return;
        };
        // DragonFireballRenderer.submit: turned by the camera's orientation.
        let [right, up] = super::billboard_axes(self.camera_orientation());
        for b in self.common.creatures.projectiles.iter().filter(|p| {
            p.kind == sunrise_sim::projectiles::ProjectileKind::DragonFireball
                && !self.hides_as_camera(p.id)
        }) {
            let p = glam::DVec3::new(
                b.previous.x + (b.position.x - b.previous.x) * alpha,
                b.previous.y + (b.position.y - b.previous.y) * alpha,
                b.previous.z + (b.position.z - b.previous.z) * alpha,
            ) - camera;
            // DragonFireballRenderer: scale 2, full bright.
            let h = 1.0;
            for (c, uv) in [
                (p - right * h - up * h, [s.u0, s.v1]),
                (p + right * h - up * h, [s.u1, s.v1]),
                (p + right * h + up * h, [s.u1, s.v0]),
                (p - right * h + up * h, [s.u0, s.v0]),
            ] {
                out.push(Vertex {
                    normal: [0; 4],
                    pos: c.as_vec3().to_array(),
                    uv,
                    layer: 0,
                    color: 0xFFFF_FFFF,
                    light: 240 | 240 << 8,

                    glint_uv: [0.0, 0.0],
                });
            }
        }
    }
    /// `DragonLandingPhase.doClientTick`, `DragonSittingFlamingPhase.doClientTick`,
    /// `DragonDeathPhase.doClientTick` and `DragonSittingAttackingPhase.doClientTick`,
    /// which `EnderDragon.aiStep` runs on a client. The first two breathe
    /// dragon breath from the head (their `look.yRot(...)` results are thrown
    /// away in vanilla, so every puff leaves along the same line), the third
    /// puffs an explosion every 10 ticks and the last growls on every tick.
    pub(in crate::game::client_side) fn remote_dragon_phase_tick(&mut self) {
        let look = self.common.dragon_look_normalised();
        let Some(d) = self.common.dragon.as_mut() else {
            return;
        };
        let head = d.parts[0].0;
        let head_y = head.y + PART_SIZES[0].1 * 0.5;
        let motion = d.velocity;
        let at = [d.position.x, d.position.y, d.position.z];
        // `flameTicks`: the client copy's own counter, advanced before it is read.
        if let Phase::Flaming { ticks } = &mut d.phase {
            *ticks += 1;
        }
        let puffs = breath_puffs(
            &d.phase,
            look,
            [head.x, head_y, head.z],
            [motion.x, motion.y, motion.z],
            &mut d.random,
        );
        let mut emitter = None;
        let mut growl = None;
        match &mut d.phase {
            Phase::Dying { time, .. } => {
                if *time % 10 == 0 {
                    let r = &mut d.random;
                    let o = [
                        f64::from((r.next_float() - 0.5) * 8.0),
                        f64::from((r.next_float() - 0.5) * 4.0),
                        f64::from((r.next_float() - 0.5) * 8.0),
                    ];
                    emitter = Some([at[0] + o[0], at[1] + 2.0 + o[1], at[2] + o[2]]);
                }
                *time += 1;
            }
            Phase::Attacking { .. } => {
                growl = Some(0.8 + d.random.next_float() * 0.3);
            }
            _ => {}
        }
        let breath = ParticleOptions::simple(ParticleType::DragonBreath);
        for (p, v) in puffs {
            self.client.particles.0.add(&breath, p, v);
        }
        if let Some(p) = emitter {
            self.client.particles.0.add(
                &ParticleOptions::simple(ParticleType::ExplosionEmitter),
                p,
                [0.0; 3],
            );
        }
        if let Some(pitch) = growl {
            self.play_audio("minecraft:entity.ender_dragon.growl", Some(at), 2.5, pitch);
        }
    }
    /// One client tick of the dragon. `lerp`: the step toward its last
    /// sent position and yaw (LivingEntity's interpolation), and the
    /// steps left including this one.
    pub(in crate::game::client_side) fn tick_remote_dragon(
        &mut self,
        lerp: Option<([f64; 3], Option<f32>, u32)>,
    ) {
        let Some(d) = self.common.dragon.as_mut() else {
            return;
        };
        d.previous = d.position;
        d.previous_yaw = d.yaw;
        d.old_flap = d.flap;
        d.ticks += 1;
        d.hurt_time = (d.hurt_time - 1).max(0);
        for part in &mut d.parts {
            part.1 = part.0;
        }
        if let Some((target, yaw, steps)) = lerp {
            let n = f64::from(steps.max(1));
            d.position = Vec3::new(
                d.position.x + (target[0] - d.position.x) / n,
                d.position.y + (target[1] - d.position.y) / n,
                d.position.z + (target[2] - d.position.z) / n,
            );
            if let Some(yaw) = yaw {
                d.yaw += wrap(yaw - d.yaw) / steps.max(1) as f32;
            }
        }
        if !d.alive() {
            // EnderDragon.tickDeath on a client: it rises, and bursts
            // from tick 180 to 200; the server removes it.
            d.death_time += 1;
            let (t, p) = (d.death_time, d.position);
            // `EnderDragon.aiStep` still growls when the phase is not sitting.
            let sitting = d.phase.sitting();
            if (180..=200).contains(&t) {
                let r = &mut d.random;
                let o = [
                    f64::from((r.next_float() - 0.5) * 8.0),
                    f64::from((r.next_float() - 0.5) * 4.0),
                    f64::from((r.next_float() - 0.5) * 8.0),
                ];
                self.client.particles.0.add(
                    &ParticleOptions::simple(ParticleType::ExplosionEmitter),
                    [p.x + o[0], p.y + 2.0 + o[1], p.z + o[2]],
                    [0.0; 3],
                );
            }
            if let Some(d) = self.common.dragon.as_mut() {
                // deathMove is new Vec3(0.0, 0.1F, 0.0): the float widened.
                let rise = f64::from(0.1_f32);
                d.position.y += rise;
                for part in &mut d.parts {
                    part.0.y += rise;
                }
            }
            // `EnderDragon.aiStep` after `tickDeath`: the client growl is
            // before the `isDeadOrDying` branch, then a dead dragon adds
            // `ParticleTypes.EXPLOSION` every tick. Both use the risen spot.
            let mut growl = None;
            if let Some(d) = self.common.dragon.as_mut()
                && !sitting
            {
                d.growl -= 1;
                if d.growl < 0 {
                    // `this.random`: pitch, then `nextInt(200)`, before the puff.
                    let pitch = 0.8 + d.random.next_float() * 0.3;
                    d.growl = 200 + d.random.next_int_bound(200).unwrap_or(0);
                    growl = Some(([d.position.x, d.position.y, d.position.z], pitch));
                }
            }
            if let Some((at, pitch)) = growl {
                self.play_audio("minecraft:entity.ender_dragon.growl", Some(at), 2.5, pitch);
            }
            let Some(d) = self.common.dragon.as_mut() else {
                return;
            };
            let p = d.position;
            let r = &mut d.random;
            let o = [
                f64::from((r.next_float() - 0.5) * 8.0),
                f64::from((r.next_float() - 0.5) * 4.0),
                f64::from((r.next_float() - 0.5) * 8.0),
            ];
            self.client.particles.0.add(
                &ParticleOptions::simple(ParticleType::Explosion),
                [p.x + o[0], p.y + 2.0 + o[1], p.z + o[2]],
                [0.0; 3],
            );
            return;
        }
        // processFlappingMovement and onFlap: its wing beats, and their sound.
        // Mth.cos(flapTime * 6.2831855F); flapSpeed's `(float) Math.pow(2.0, movement.y)`.
        let flap_was = trig::cos(f64::from(d.old_flap * std::f32::consts::TAU));
        let v = d.velocity;
        let mut flap_speed = 0.2 / (((v.x * v.x + v.z * v.z).sqrt() as f32) * 10.0 + 1.0);
        flap_speed *= 2f64.powf(v.y) as f32;
        d.flap += if d.phase.sitting() { 0.1 } else { flap_speed };
        let flap_now = trig::cos(f64::from(d.flap * std::f32::consts::TAU));
        let at = [d.position.x, d.position.y, d.position.z];
        if flap_was <= -0.3 && flap_now >= -0.3 {
            let pitch = 0.8 + self.common.world.random.next_float() * 0.3;
            self.play_audio("minecraft:entity.ender_dragon.flap", Some(at), 5.0, pitch);
        }
        let d = self.common.dragon.as_mut().expect("dragon");
        if !d.phase.sitting() {
            d.growl -= 1;
            if d.growl < 0 {
                d.growl = 200 + d.random.next_int_bound(200).unwrap_or(0);
                let pitch = 0.8 + self.common.world.random.next_float() * 0.3;
                self.play_audio("minecraft:entity.ender_dragon.growl", Some(at), 2.5, pitch);
            }
        }
        // checkCrystals' search runs on a client too (the healing beam).
        let d = self.common.dragon.as_mut().expect("dragon");
        if d.random.next_int_bound(10).unwrap_or(1) == 0 {
            let p = d.position;
            let nearest = self.common.nearest_crystal_to(p);
            self.common.dragon.as_mut().expect("dragon").nearest_crystal = nearest;
        }
        let d = self.common.dragon.as_mut().expect("dragon");
        d.yaw = wrap(d.yaw);
        // EnderDragon.aiStep on a client with isNoAi, as on the server.
        if d.no_ai {
            d.flap = 0.5;
            return;
        }
        d.record();
        self.remote_dragon_phase_tick();
        self.common.place_dragon_parts();
    }

    /// `BossHealthOverlay.extractRenderState`: whatever `publish_boss_bars`
    /// (or a server's packets) put on this client's overlay.
    pub(in crate::game) fn draw_boss_bars(&self, batch: &mut HudBatch) {
        let draws = self.boss_bar_draws(batch.width as i32, batch.height as i32);
        hud_layout::paint(batch, &self.client.gui.atlas, &draws);
    }

    /// `EnderDragonRenderer.submitRays`: untextured, additive death rays.
    pub(in crate::game) fn dragon_rays(&self, base: glam::Mat4, t: f32, out: &mut Vec<Vertex>) {
        dragon_rays(base, t, self.client.gui.atlas.font.white(), out);
    }

    pub(in crate::game) fn client_resident_dragon_part_index(&self, id: u32) -> Option<usize> {
        let dragon = self.common.dragon.as_ref()?;
        dragon_part_index(dragon.id, id)
    }

    /// Client-side entity check for `end_crystal_fits` (`EndCrystalItem.useOn`).
    pub(in crate::game) fn client_entity_in(&self, room: Aabb) -> bool {
        let touches = |lo: Vec3, hi: Vec3| {
            let (a, b) = (room.min_position(), room.max_position());
            lo.x < b.x && hi.x > a.x && lo.y < b.y && hi.y > a.y && lo.z < b.z && hi.z > a.z
        };
        let player = !self.common.spectator()
            && self
                .common
                .player
                .as_ref()
                .is_some_and(|p| touches(p.bounds().min_position(), p.bounds().max_position()));
        player
            || self.common.crystals.iter().any(|c| {
                touches(
                    Vec3::new(c.position.x - 1.0, c.position.y, c.position.z - 1.0),
                    Vec3::new(c.position.x + 1.0, c.position.y + 2.0, c.position.z + 1.0),
                )
            })
            || self.common.mobs.iter().any(|m| {
                let w = f64::from(m.width()) / 2.0;
                let h = f64::from(m.height());
                touches(
                    Vec3::new(m.position.x - w, m.position.y, m.position.z - w),
                    Vec3::new(m.position.x + w, m.position.y + h, m.position.z + w),
                )
            })
    }

    /// Client-side dragon retirement: forgets parent and all 8 parts' client sidecars before discard.
    pub(in crate::game::client_side) fn client_discard_dragon(&mut self) {
        if let Some(id) = self
            .common
            .dragon
            .as_ref()
            .filter(|dragon| dragon.id != 0 || self.client.remote.is_some())
            .map(|dragon| dragon.id)
        {
            for part in 0..8 {
                self.client_forget_entity(dragon_part_id(id, part));
            }
            self.client_forget_entity(id);
        }
        self.common.dragon.discard();
    }

    /// `handleAddEntity` for `minecraft:ender_dragon`. Its constructor
    /// sets the full health and the hovering phase until the server's
    /// synched data says otherwise.
    pub fn spawn_remote_dragon(&mut self, id: i32, position: [f64; 3], yaw: f32) {
        let at = Vec3::new(position[0], position[1], position[2]);
        let mut d = Dragon::new(at, yaw, i64::from(id));
        d.id = id as u32;
        d.phase = Phase::from_id(10);
        d.join = sunrise_sim::join_seq::JoinSeq::next();
        self.client_discard_dragon();
        self.common.dragon.replace(d);
        self.common.place_dragon_parts();
        if let Some(d) = self.common.dragon.as_mut() {
            for part in &mut d.parts {
                part.1 = part.0;
            }
        }
    }

    /// The remote dragon, if `id` is it or one of its parts.
    pub(in crate::game::client_side) fn remote_dragon_mut(
        &mut self,
        id: i32,
    ) -> Option<&mut Dragon> {
        let uid = id as u32;
        let remote = self.client.remote.is_some();
        self.common.dragon.as_mut().filter(|d| {
            (d.id != 0 || remote) && (d.id == uid || dragon_part_index(d.id, uid).is_some())
        })
    }

    /// `handleSetEntityData`: LivingEntity.DATA_HEALTH_ID and
    /// EnderDragon.DATA_PHASE (`onSyncedDataUpdated` sets the phase).
    pub(in crate::game) fn remote_dragon_data(&mut self, id: i32, values: &[(u8, i32, Value)]) {
        let Some(d) = self.remote_dragon_mut(id) else {
            return;
        };
        for (index, _, value) in values {
            match (index, value) {
                (9, Value::Float(health)) => d.health = *health,
                (16, Value::Int(phase)) => d.set_phase(Phase::from_id(*phase)),
                // Mob.DATA_MOB_FLAGS_ID: isNoAi.
                (15, Value::Byte(flags)) => d.no_ai = flags & 1 != 0,
                _ => {}
            }
        }
    }

    /// `handleSetEntityMotion`: the velocity its wings flap by.
    pub(in crate::game) fn remote_dragon_motion(&mut self, id: i32, velocity: [f64; 3]) {
        if let Some(d) = self.remote_dragon_mut(id) {
            d.velocity = Vec3::new(velocity[0], velocity[1], velocity[2]);
        }
    }

    /// `handleHurtAnimation` and `handleDamageEvent`: the red flash.
    pub(in crate::game) fn remote_dragon_hurt(&mut self, id: i32) {
        if let Some(d) = self.remote_dragon_mut(id) {
            // `LivingEntity.handleDamageEvent`: the red flash. Its sound is the
            // server's (`playHurtSound`); a client plays one only for its own player.
            d.hurt_time = 10;
        }
    }

    /// `handleRemoveEntities`, or a crystal removed: the healing beam
    /// keeps pointing at the same crystal.
    pub fn remove_remote_dragon(&mut self, id: i32) {
        if self
            .common
            .dragon
            .as_ref()
            .is_some_and(|d| d.id as i32 == id)
        {
            self.client_discard_dragon();
            return;
        }
        let Some(index) = self.common.crystals.iter().position(|c| c.id as i32 == id) else {
            return;
        };
        if let Some(d) = self.common.dragon.as_mut() {
            match d.nearest_crystal {
                Some(n) if n == index => d.nearest_crystal = None,
                Some(n) if n > index => d.nearest_crystal = Some(n - 1),
                _ => {}
            }
        }
    }
}
