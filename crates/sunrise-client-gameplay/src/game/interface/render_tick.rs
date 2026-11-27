//! Per-tick client visuals: the particle engine, `ClientLevel.animateTick`,
//! campfire smoke and rain splashes (`ClientLevel.tickWeatherEffects`).
use super::{Dim, weather::precipitation_at};
#[cfg(feature = "client")]
use crate::game::host::draw::GameCameraDrawContext;
use mod_api::{
    __private::{SelectedBlockScreenEffectRenderer, block_screen_effect_render},
    BlockScreenEffect,
};
use sunrise_client::camera::NearPlane;
use sunrise_client::fog::{FogData, FogInputs, FogType, settle_rain_fog, update_rain_fog};
use sunrise_client::{
    animate_tick::{AnimateContext, AnimateSampling, AnimateStep, marker_particle_target},
    particles::{ParticleLevel, TrackingBody, TrackingTarget},
};
use sunrise_client::{
    hud_overlays,
    screen_effects::{
        CameraQuads, ScreenInputs, ScreenQuad, lightmap_brightness, nausea_has_effect,
    },
};
use sunrise_core::{
    position::BlockPos,
    random::{LegacyRandom, RandomSource},
};
use sunrise_gameplay_common::game::api_values::TryIntoApi;
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
use sunrise_gameplay_common::game::native_hooks::is_valid_book_shelf::GameHooks as _;
use sunrise_gameplay_common::game::weather_height::motion_blocking_height;
use sunrise_items::blocks::{BlockFacts, Fluid};
use sunrise_level::assets::Properties;
use sunrise_level::particle_options::{ParticleOptions, ParticleType};
use sunrise_level::world::World;
use sunrise_sim::{
    mobs::Mob,
    motion::{PlayerMotion, WATER},
};
use sunrise_world::dimension_type::{CardinalLight, Skybox};
use sunrise_world::{
    environment_attributes::{AMBIENT_PARTICLES, Value as AttributeValue, attribute},
    particle_options::{ParticleOptions as PersistentOptions, block_state_id},
    registry::BlockStateId,
};

/// `ScreenEffectRenderer.submit`'s block overlays in the order it draws them, which
/// `BlockScreenEffects::custom` and `ScreenInputs::custom` follow.
const BLOCK_OVERLAYS: [BlockScreenEffect; 3] = [
    BlockScreenEffect::Block,
    BlockScreenEffect::Water,
    BlockScreenEffect::Fire,
];
/// The block overlays of `ScreenEffectRenderer` a frame shows.
struct BlockScreenEffects {
    /// The view-blocking block the camera is inside, drawn with its particle texture.
    block: Option<BlockStateId>,
    /// Draw the vanilla underwater overlay.
    water: bool,
    /// The player burns, seen in first person. Extraction cancellation does not change this.
    on_fire: bool,
    /// Whether a no-op or a custom renderer replaces the fire overlay.
    fire_suppressed: bool,
    /// The renderer for each overlay in [`BLOCK_OVERLAYS`], shown or canceled.
    custom: [Option<ExtractedBlockScreenEffect>; 3],
}

struct ExtractedBlockScreenEffect {
    state: mod_api::BlockState,
    pos: BlockPos,
    renderer: SelectedBlockScreenEffectRenderer,
}
pub(in crate::game) fn player_tracking_body(player: &PlayerMotion) -> TrackingBody {
    TrackingBody {
        feet: [player.position.x, player.position.y, player.position.z],
        width: f64::from(player.dimensions.width),
        height: f64::from(player.dimensions.height),
    }
}

pub(in crate::game) fn mob_tracking_body(mob: &Mob) -> TrackingBody {
    let (width, height, _) = mob.dims();
    TrackingBody {
        feet: [mob.position.x, mob.position.y, mob.position.z],
        width,
        height,
    }
}

/// `GameRenderer.nightVisionScale` for an optional night-vision duration (`-1` is infinite).
fn night_vision_scale(duration: Option<i32>, partial: f32) -> f32 {
    match duration {
        Some(d) if !(0..=200).contains(&d) => 1.0,
        Some(d) => 0.7 + ((d as f32 - partial) * std::f32::consts::PI * 0.2).sin() * 0.3,
        None => 0.0,
    }
}

/// `ClientLevel.tickWeatherEffects`: the MOTION_BLOCKING height of the column
/// `x`, `z` when a rain splash may land on it for a camera in block row `y`,
/// that is above the world's bottom and within 10 blocks of the camera.
fn splash_height(world: &World, properties: &Properties, x: i32, y: i32, z: i32) -> Option<i32> {
    let top = motion_blocking_height(world, properties, x, z);
    let hi = y.wrapping_add(10);
    let lo = y.wrapping_sub(10);
    (top > world.bottom_y() && top <= hi && top >= lo).then_some(top)
}
/// The first block at or below `from` that blocks motion (leaves only when
/// `leaves`, as MOTION_BLOCKING_NO_LEAVES skips them) or holds a fluid, plus one.
fn motion_blocking_leaves(
    world: &World,
    facts: &BlockFacts,
    x: i32,
    z: i32,
    from: i32,
    leaves: bool,
) -> i32 {
    let mut y = from.min(world.top_y() - 1);
    while y > world.bottom_y() {
        let f = facts.get(world.block(x, y, z));
        // #minecraft:leaves block motion, but MOTION_BLOCKING_NO_LEAVES skips them
        // (their water still counts).
        let blocks = if f.name.ends_with("_leaves") {
            leaves
        } else {
            f.full_collision
        };
        if blocks || f.fluid != Fluid::None {
            return y + 1;
        }
        y -= 1;
    }
    world.bottom_y()
}
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Polytone GPU particles, before the frame's quads are built: any pending clear, the
    /// renderer's verdict on each shader and on the quads drawn here, then the heightmap for the readers that draw (a custom shader's `Sampler1`,
    /// and the surface rain drawn as quads dies under), every half second.
    pub fn polytone_gpu_decide(&mut self, camera: glam::DVec3) {
        // A mod's clear asked since the last frame goes first: cleared records read nothing.
        self.apply_mod_particle_clear();
        let (world, properties) = (&self.common.world, &self.common.pack.assets.properties);
        let top = |x, z| motion_blocking_height(world, properties, x, z);
        let source = sunrise_client::particles::polytone_gpu::HeightmapSource {
            size: sunrise_client::gfx::POLYTONE_HEIGHTMAP_SIZE,
            min_y: world.bottom_y(),
            top: &top,
        };
        let renderer = self.client.renderer.0.as_deref_mut().map(|r| r as _);
        self.client
            .particles
            .0
            .polytone_gpu_decide(renderer, camera, &source);
    }

    /// After the frame's quads (and any clear in their extraction): the records of the types
    /// the renderer draws with their own shader go to it.
    pub(in crate::game::client_side) fn polytone_gpu_particles(
        &mut self,
        camera: glam::DVec3,
        alpha: f64,
    ) {
        let world = &self.common.world;
        let renderer = self.client.renderer.0.as_deref_mut().map(|r| r as _);
        self.client
            .particles
            .0
            .polytone_gpu_hand_over(renderer, camera, alpha as f32, &|b| {
                world.light(b[0], b[1], b[2])
            });
    }

    /// `GameRenderer`'s cardinal light: the level's type uses the Nether's diffuse lights.
    pub(in crate::game) fn nether_lighting(&self) -> bool {
        self.dimension_type().cardinal_light == CardinalLight::Nether
    }
    /// `SkyRenderer`'s overworld skybox: sun, moon and stars.
    pub fn overworld_skybox(&self) -> bool {
        self.dimension_type().skybox == Skybox::Overworld
    }
    /// `SkyRenderer`'s End skybox and its flashes.
    pub fn end_skybox(&self) -> bool {
        self.dimension_type().skybox == Skybox::End
    }
    /// `LevelRenderer.prepareTranslucents`: prepare the clouds the frame draws.
    pub fn frame_cloud_uv(
        &self,
        title: bool,
        sky: &sunrise_level::environment::Sky,
    ) -> Option<[f32; 2]> {
        if title || !sunrise_client::clouds::visible(self.effective_clouds(), sky.cloud_color[3]) {
            return None;
        }
        let white = self.client.gui.atlas.sprites.get("sunrise/white")?;
        Some([(white.u0 + white.u1) / 2.0, (white.v0 + white.v1) / 2.0])
    }
    pub(in crate::game::client_side) fn forward(&self) -> [f64; 3] {
        let (yaw, pitch) = (
            f64::from(self.common.yaw).to_radians(),
            f64::from(self.common.pitch).to_radians(),
        );
        [
            -yaw.sin() * pitch.cos(),
            -pitch.sin(),
            yaw.cos() * pitch.cos(),
        ]
    }
    /// `Camera.getNearPlane` at the FOV option, as `getFluidInCamera` asks for it.
    pub(in crate::game::client_side) fn near_plane(&self) -> NearPlane {
        let [width, height] = self
            .client
            .renderer
            .0
            .as_ref()
            .map_or([1, 1], |r| [r.config.width, r.config.height]);
        self.client.view.near_plane(
            self.client.settings.fov as i32 as f32,
            width as f32,
            height.max(1) as f32,
        )
    }
    /// `FogRenderer.setupFog` for the frame's camera, and Sunrise's Fog option.
    pub fn frame_fog(
        &self,
        camera: glam::DVec3,
        sky: &sunrise_level::environment::Sky,
        biome: u16,
        water_fog_color: [f32; 3],
    ) -> FogData {
        let cam = self.camera_subject();
        let is_local = cam.as_ref().is_none_or(|c| c.is_local_player);
        let forward = if self.client.view.forwards == [0.0; 3] {
            self.forward().map(|c| c as f32)
        } else {
            self.client.view.forwards
        };
        let at = [camera.x, camera.y, camera.z];
        let fog_type = self.camera_fog_type(at);
        let (rain, thunder) = self.view_weather(self.common.clock.fraction() as f32);
        let title_fog = sunrise_client::fog::FogTables::default();
        let tables = if self.client.title.0.is_some() {
            &title_fog
        } else {
            self.client_res().fog()
        };
        let partial = self.common.clock.fraction() as f32;
        let fog_effects = cam.as_ref().map(|c| c.fog_effects);
        let blindness = fog_effects.map(|e| e.blindness).unwrap_or_else(|| {
            self.common
                .potions
                .effects
                .get("blindness")
                .map(|effect| (effect.duration, effect.duration == -1))
        });
        let darkness = fog_effects
            .map(|e| {
                e.darkness_blend
                    .map(|(previous, current)| previous + (current - previous) * partial)
            })
            .unwrap_or_else(|| self.darkness_fog());
        let fire_resistance = fog_effects.map_or_else(
            || self.common.potions.effects.get("fire_resistance").is_some(),
            |e| e.fire_resistance,
        ) || (is_local && self.pvp_clear_fluids().0);
        let night_vision = fog_effects
            .map(|e| {
                if e.darkness_blend.is_some() {
                    0.0
                } else {
                    night_vision_scale(e.night_vision, partial)
                }
            })
            .unwrap_or_else(|| self.fog_night_vision());
        let input = FogInputs {
            fog_type,
            sky,
            water_fog_color,
            biome,
            render_distance_chunks: self.terrain_distance(),
            distant_terrain: self.client.lod.0.is_some() && self.overworld_skybox(),
            cloud_range_chunks: self.client.settings.cloud_range,
            camera_y: camera.y,
            min_y: self.level_min_y(),
            flat: self.level_flat(),
            forward,
            rain,
            thunder,
            blindness,
            darkness,
            darken_world: self.client.boss_overlay.darken_amount(partial),
            boss_world_fog: self.client.boss_overlay.should_create_world_fog(),
            spectator: cam
                .as_ref()
                .map_or_else(|| self.own_player_is_spectator(), |c| c.is_spectator),
            fire_resistance,
            full_water_vision: !is_local || self.pvp_clear_fluids().1,
            night_vision,
            environment: self.client.client_pack.polytone_colors.env,
            fluid_fog: self.client.client_pack.polytone_colors.fluid_fog,
        };
        let mut fog = sunrise_client::fog::setup_fog(tables, &self.client.fog, &input);
        if !self.client.settings.sunrise.fog {
            sunrise_client::fog::clear_distance_fog(&mut fog, &input);
        }
        if self.client.title.0.is_none() {
            fog.color = self.native_fog_color_now(
                self.common.clock.fraction() as f32,
                at,
                fog_type,
                fog.color,
            );
            fog = self.native_fog_now(self.common.clock.fraction() as f32, at, fog_type, fog);
        }
        fog
    }
    fn native_fog_color_now(
        &self,
        partial: f32,
        camera: [f64; 3],
        fog_type: FogType,
        color: [f32; 4],
    ) -> [f32; 4] {
        let mut rgb = [color[0], color[1], color[2]];
        self.native_compute_fog_color(partial, camera, fog_type, &mut rgb);
        if rgb.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)) {
            [rgb[0], rgb[1], rgb[2], color[3]]
        } else {
            color
        }
    }
    fn native_fog_now(
        &self,
        partial: f32,
        camera: [f64; 3],
        fog_type: FogType,
        original: FogData,
    ) -> FogData {
        let mut fog = original;
        self.native_render_fog(partial, camera, fog_type, &mut fog);
        let distances = [
            fog.environmental_start,
            fog.environmental_end,
            fog.render_distance_start,
            fog.render_distance_end,
            fog.sky_end,
            fog.cloud_end,
        ];
        if fog
            .color
            .iter()
            .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
            && distances.iter().all(|v| v.is_finite())
            && fog.environmental_end > fog.environmental_start
            && fog.render_distance_end > fog.render_distance_start
            && fog.sky_end >= 0.0
            && fog.cloud_end >= 0.0
        {
            fog
        } else {
            original
        }
    }
    /// `FogRenderer.computeFogColor`'s night vision brightenFactor:
    /// `GameRenderer.nightVisionScale`, but not while Darkness is on.
    fn fog_night_vision(&self) -> f32 {
        let partial = self.common.clock.fraction() as f32;
        if self.common.potions.effects.get("darkness").is_some() {
            return 0.0;
        }
        night_vision_scale(
            self.common
                .potions
                .effects
                .get("night_vision")
                .map(|e| e.duration),
            partial,
        )
    }
    /// `ClientLevelData.isFlat` on a connected client.
    fn level_flat(&self) -> bool {
        self.client.remote.as_ref().is_some_and(|r| r.flat)
    }
    /// Client arm of `level_min_y` (`side_level_min_y`).
    pub(in crate::game) fn client_level_min_y(&self) -> i32 {
        self.client
            .remote
            .as_ref()
            .map_or(self.common.world.shape.min_y, |r| r.dimension.min_y)
    }
    /// `SkyRenderer.shouldRenderDarkDisc`: uses `minecraft.player` eye and water state, not camera.
    pub fn dark_disc(&self, _camera: glam::DVec3) -> bool {
        let Some(p) = self.common.player.as_ref() else {
            return false;
        };
        let sunrise = if self.level_flat() {
            f64::from(self.level_min_y())
        } else {
            f64::from(self.client.client_pack.polytone_colors.env.horizon_height)
        };
        let eye_y = p.position.y + f64::from(p.dimensions.eye_height);
        self.overworld_skybox() && eye_y < sunrise && !p.is_under_water()
    }
    /// `ScreenEffectRenderer` + `Hud` camera overlays for this frame.
    /// `scale` is the player's `LivingEntity.getScale`.
    /// `hand_projection` is the frame's `camera::hand_projection`, which mods draw under.
    pub fn screen_quads(
        &self,
        partial: f32,
        size: [u32; 2],
        scale: f32,
        hand_projection: [f32; 16],
    ) -> Vec<ScreenQuad> {
        let Some(p) = self.common.player.as_ref() else {
            return Vec::new();
        };
        let eye = [
            p.position.x,
            p.position.y + f64::from(p.dimensions.eye_height),
            p.position.z,
        ];
        let mut effects = self.block_screen_effects(p, eye, partial, size, scale);
        let custom = self.custom_screen_effects(&mut effects, partial, hand_projection);
        // ScreenEffectRenderer: the block's particle material.
        let block_overlay = effects
            .block
            .and_then(|state| Self::block_particle_sprite(&self.client.client_pack, state));
        let water = effects
            .water
            .then(|| (self.eye_brightness(eye), self.common.yaw, self.common.pitch));
        // Hud.extractCameraOverlays; the spyglass scope (and its easing) is
        // drawn on the HUD by draw_spyglass.
        let fx = &self.client.screen_fx;
        let border_pos =
            self.camera_entity_position()
                .unwrap_or([p.position.x, p.position.y, p.position.z]);
        let overlays = hud_overlays::CameraOverlays {
            vignette: self.vignette_shown().then_some((
                fx.vignette_brightness,
                self.common
                    .border
                    .warning_strength(border_pos[0], border_pos[2]),
            )),
            first_person: self.client.camera_type.is_first_person(),
            scoping: self.scoping(),
            game_time_delta: 0.0,
            // `EquipmentSlot.values()` order across all 8 player equipment slots (including body and saddle).
            equipment: acting_equipment!(self)
                .into_iter()
                .zip(sunrise_items::equipment::SLOT_NAMES)
                .filter_map(|(stack, slot)| {
                    let e = sunrise_sim::item_sync::equippable(stack?, self.common.res.items())?;
                    if e.slot != slot {
                        return None;
                    }
                    let overlay = e.camera_overlay?;
                    Some(
                        overlay
                            .strip_prefix("minecraft:")
                            .map_or_else(|| overlay.clone(), str::to_owned),
                    )
                })
                .collect(),
            ticks_frozen: p.ticks_frozen,
            percent_frozen: p.percent_frozen(),
            partial,
            portal: [fx.o_portal, fx.portal],
            nausea: self.nausea_blend(partial),
            screen_effect_scale: self.screen_effect_scale(),
        };
        let mut scope = self.client.scope_scale;
        let gui_overlays = if self.client.chat_ui.hide_gui {
            Vec::new()
        } else {
            hud_overlays::camera_overlays(size[0] as i32, size[1] as i32, &overlays, &mut scope)
        };
        self.client.screen_fx.build(&ScreenInputs {
            sprites: &self.client.gui.atlas.sprites,
            flags: &self.client.gui.atlas.flags,
            aspect: size[0] as f32 / size[1].max(1) as f32,
            partial,
            block_overlay,
            water,
            on_fire: effects.on_fire && !effects.fire_suppressed,
            fire_lower: self.pvp_fire_lower(),
            gui_overlays: &gui_overlays,
            custom: [&custom[0], &custom[1], &custom[2]],
        })
    }
    /// Submit the selected renderers in vanilla order. Keep their quads separate so each
    /// goes immediately before its own vanilla overlay, if that overlay is still shown.
    fn custom_screen_effects(
        &self,
        effects: &mut BlockScreenEffects,
        partial: f32,
        hand_projection: [f32; 16],
    ) -> [Vec<ScreenQuad>; 3] {
        let mut drawn = <[Vec<ScreenQuad>; 3]>::default();
        for ((effect, custom), drawn) in BLOCK_OVERLAYS
            .into_iter()
            .zip(&effects.custom)
            .zip(&mut drawn)
        {
            let Some(custom) = custom else {
                continue;
            };
            let mut quads = CameraQuads::new(hand_projection);
            let Some(replace) = custom.renderer.submit(
                self,
                block_screen_effect_render(
                    effect,
                    custom.state,
                    custom.pos,
                    partial,
                    self.client.chat_ui.hide_gui,
                    &mut GameCameraDrawContext::new(self, &mut quads),
                ),
            ) else {
                // Only a finished submission can affect the frame.
                continue;
            };
            if replace {
                match effect {
                    BlockScreenEffect::Block => effects.block = None,
                    BlockScreenEffect::Water => effects.water = false,
                    BlockScreenEffect::Fire => effects.fire_suppressed = true,
                }
            }
            *drawn = quads.take();
        }
        drawn
    }
    /// `LevelExtractor.extractPlayerState`: extract the player's block overlays and any
    /// renderers selected for them. Fire eligibility is separate from renderer selection.
    fn block_screen_effects(
        &self,
        p: &PlayerMotion,
        eye: [f64; 3],
        partial: f32,
        size: [u32; 2],
        scale: f32,
    ) -> BlockScreenEffects {
        // LevelExtractor.getViewBlockingState checks player.noPhysics (p.spectator); submit checks isSpectator.
        let modded = mod_api::__private::listened(mod_api::HookId::ExtractBlockScreenEffect);
        let player_partial_tick = self.common.clock.camera_fraction() as f32;
        let mut block = None;
        let mut custom = [None, None, None];
        if !p.spectator {
            // Frustum.getNearPlaneBounds from culling projection (max(fov, option_fov), camera rot).
            let cull_fov = self
                .fov_degrees(partial, eye)
                .max((self.client.settings.fov as i32) as f32);
            let (near_yaw, near_pitch, _) =
                self.client
                    .posed_rotation
                    .unwrap_or((self.common.yaw, self.common.pitch, 0.0));
            let (near_min, near_max) = sunrise_client::camera::near_plane_bounds(
                cull_fov,
                size[0] as f32,
                size[1] as f32,
                near_yaw,
                near_pitch,
                sunrise_client::camera::depth_far(
                    self.terrain_distance(),
                    self.client.settings.cloud_range,
                ),
            );
            let near_plane = sunrise_core::geometry::Aabb::new(
                sunrise_core::geometry::Vec3::new(
                    near_min[0] + eye[0],
                    near_min[1] + eye[1],
                    near_min[2] + eye[2],
                ),
                sunrise_core::geometry::Vec3::new(
                    near_max[0] + eye[0],
                    near_max[1] + eye[1],
                    near_max[2] + eye[2],
                ),
            );
            // The test points use float maths: `(i % 2 - 0.5F) * width * 0.8F`.
            // The height spread is `0.1F * getScale()`: a tiny player's eye is
            // closer to the ground than 0.05.
            let width = p.dimensions.width;
            for i in 0..8 {
                let t = [
                    p.position.x + f64::from(((i & 1) as f32 - 0.5) * width * 0.8),
                    p.position.y
                        + f64::from(p.dimensions.eye_height)
                        + f64::from((((i >> 1) & 1) as f32 - 0.5) * 0.1 * scale),
                    p.position.z + f64::from((((i >> 2) & 1) as f32 - 0.5) * width * 0.8),
                ];
                let pos = [
                    t[0].floor() as i32,
                    t[1].floor() as i32,
                    t[2].floor() as i32,
                ];
                let state = self.common.world.block(pos[0], pos[1], pos[2]);
                if self.common.res.facts().get(state).render_shape
                    != sunrise_items::blocks::RenderShape::Invisible
                    && self.state_view_blocking(state, pos, near_plane)
                {
                    block = Some(state);
                    if modded
                        && let Ok(state) = state.try_into_api()
                        && !self.screen_effect_kept(
                            BlockScreenEffect::Block,
                            state,
                            pos,
                            partial,
                            player_partial_tick,
                            &mut custom[0],
                        )
                    {
                        block = None;
                    }
                    break;
                }
            }
        }
        let first_person =
            self.client.camera_type.is_first_person() && !self.own_player_is_spectator();
        let water = first_person && {
            // LevelExtractor uses the player's tick-sampled eye state, not camera fog geometry.
            // Keep the existing mod override explicit, applied to that answer at the eye.
            let mut submersion = if p.is_eye_in_fluid(WATER) {
                FogType::Water
            } else {
                FogType::Atmospheric
            };
            self.native_camera_submersion(eye, &mut submersion);
            submersion == FogType::Water
                && (!modded
                    || self.screen_effect_kept(
                        BlockScreenEffect::Water,
                        self.client.client_pack.water_overlay_state,
                        eye.map(|c| c.floor() as i32),
                        partial,
                        player_partial_tick,
                        &mut custom[1],
                    ))
        };
        // Canceling fire acts as a no-op renderer only if no listener selected one.
        // A selected renderer can still prepend to the eligible vanilla overlay.
        let on_fire = first_person && p.is_on_fire();
        let fire_suppressed = on_fire
            && modded
            && self
                .common
                .pack
                .assets
                .registry
                .default_state("minecraft:fire")
                .and_then(|fire| fire.try_into_api().ok())
                .is_some_and(|fire| {
                    let feet = [p.position.x, p.position.y, p.position.z].map(|c| c.floor() as i32);
                    !self.screen_effect_kept(
                        BlockScreenEffect::Fire,
                        fire,
                        feet,
                        partial,
                        player_partial_tick,
                        &mut custom[2],
                    ) && custom[2].is_none()
                });
        BlockScreenEffects {
            block,
            water,
            on_fire,
            fire_suppressed,
            custom,
        }
    }
    /// Extract one overlay's renderer independently of whether vanilla's quads are kept.
    fn screen_effect_kept(
        &self,
        effect: BlockScreenEffect,
        state: mod_api::BlockState,
        pos: [i32; 3],
        partial_tick: f32,
        player_partial_tick: f32,
        custom: &mut Option<ExtractedBlockScreenEffect>,
    ) -> bool {
        let pos = BlockPos::new(pos[0], pos[1], pos[2]);
        let (mut canceled, mut renderer) = (false, None);
        self.native_extract_block_screen_effect(
            effect,
            state,
            pos,
            partial_tick,
            player_partial_tick,
            &mut canceled,
            &mut renderer,
        );
        *custom = renderer.map(|renderer| ExtractedBlockScreenEffect {
            state,
            pos,
            renderer,
        });
        !canceled
    }
    /// `Lightmap.getBrightness(getMaxLocalRawBrightness(eye))`.
    fn eye_brightness(&self, eye: [f64; 3]) -> f32 {
        let b = [
            eye[0].floor() as i32,
            eye[1].floor() as i32,
            eye[2].floor() as i32,
        ];
        let packed = self.common.world.light(b[0], b[1], b[2]);
        let sky = i32::from(packed >> 4) - self.sky_darken();
        let raw = sky.max(i32::from(packed & 15)).clamp(0, 15) as u8;
        lightmap_brightness(
            if self.common.dim == Dim::Nether {
                0.1
            } else {
                0.0
            },
            raw,
        )
    }
    /// What `Minecraft` ticks outside the level's own tick: `GameRenderer.tick` runs the light
    /// flicker whenever the game is not paused, frozen or not, while `TextureManager.tick`
    /// needs the level running normally. `SUNRISE_STILL` keeps both with the world.
    pub(in crate::game::client_side) fn tick_frame_visuals(&mut self, running: bool) {
        self.hold_fullbright();
        if running || !self.common.clock.still() {
            self.client.fog.tick_flicker();
        }
        if running {
            self.client.texture_ticks += 1;
        }
    }
    /// What `AtmosphericFogEnvironment.updateRainFogState` reads at the eye: the rain, the sky
    /// light and whether the biome has precipitation.
    fn rain_fog_inputs(&self, eye: [f64; 3]) -> (f32, u8, bool) {
        let b = [
            eye[0].floor() as i32,
            eye[1].floor() as i32,
            eye[2].floor() as i32,
        ];
        let rain = self.view_weather(1.0).0;
        let biome = self
            .common
            .world
            .chunks
            .get(&(b[0] >> 4, b[2] >> 4))
            .map_or(0, |c| c.biome(b[0], b[1], b[2]));
        let rains = world_biomes!(self).get(biome).precipitation;
        (rain, self.common.world.light(b[0], b[1], b[2]) >> 4, rains)
    }
    /// Under `SUNRISE_STILL` a frozen client's rain fog sits at its level, as the rain does: it
    /// eases only in `tick_particles`, which waits for the world.
    pub(in crate::game::client_side) fn settle_rain_fog(&mut self) {
        if let Some(e) = self.camera_eye_position() {
            let (rain, sky_light, rains) = self.rain_fog_inputs(e);
            settle_rain_fog(&mut self.client.fog, rain, sky_light, rains);
        }
    }
    pub(in crate::game::client_side) fn remote_tracking_body(
        &self,
        id: i32,
    ) -> Option<(TrackingTarget, TrackingBody)> {
        if self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| r.entity_id == id)
        {
            let player = self.common.player.as_ref()?;
            return Some((TrackingTarget::LocalPlayer, player_tracking_body(player)));
        }
        Some((
            TrackingTarget::Entity(id),
            self.remote_entity_tracking_body(id)?,
        ))
    }

    /// `Hud.updateVignetteBrightness` from the camera entity's eye, running even while frozen.
    pub fn tick_vignette(&mut self) {
        if let Some(e) = self.camera_eye_position() {
            let brightness = self.eye_brightness(e);
            self.client.screen_fx.tick_vignette(brightness);
        }
    }

    /// Advance particle effects without advancing the client level.
    #[allow(dead_code)]
    pub fn tick_particles(&mut self) {
        self.prepare_particle_sources();
        tick_client_weather_effects(self);
        self.tick_particle_sources();
        self.tick_particle_engine();
    }

    #[inline(always)]
    pub(in crate::game::client_side) fn prepare_particle_sources(&mut self) {
        self.apply_mod_particle_clear();
        let mut laps = sunrise_level::spans::laps();
        let cam_pos = self.client.view.position;
        if self.common.eye_position().is_some() {
            let inside_portal = self.client_portal_effect();
            self.client.screen_fx.tick_portal(inside_portal);
            // LocalPlayer.aiStep: processPortalCooldown after the transition effect.
            if self.client.remote.is_some() && self.common.portal_player.cooldown > 0 {
                self.common.portal_player.cooldown -= 1;
            }
            let nausea = self
                .common
                .potions
                .effects
                .get("nausea")
                .map(nausea_has_effect);
            self.client.screen_fx.tick_spinning(nausea);
            let (rain, sky_light, rains) = self.rain_fog_inputs(cam_pos);
            update_rain_fog(&mut self.client.fog, rain, sky_light, rains, 1.0);
        }
        laps.lap("tp_fx");
        self.client.particles.0.particle_level = self.client.settings.particles;
        self.client.particles.0.camera = cam_pos;
    }

    /// ClientLevel.animateTick, after the level's own tick and before ParticleEngine.tick.
    pub(in crate::game::client_side) fn tick_particle_sources(&mut self) {
        self.update_animation_inputs();
        let mut laps = sunrise_level::spans::laps();
        let (rain, thunder) = self.weather_levels(1.0);
        let ambient_attribute = attribute(AMBIENT_PARTICLES).expect("registered ambient particles");
        let marker =
            marker_particle_target(self.common.mode, self.common.stack_in(self.common.selected));
        // The day timeline: firefly bush sounds from 12600 to 23400.
        let firefly_bush_sounds = self.dimension_type().has_timeline("minecraft:day")
            && (12600..23401).contains(&self.common.world.day_time.rem_euclid(24000));
        // EnvironmentAttributes.CREAKING_ACTIVE: the same overworld night.
        let creaking_active = self.dimension_type().has_timeline("minecraft:day")
            && (12600..23401).contains(&self.common.world.day_time.rem_euclid(24000));
        // Level.isThundering: a thunder level above 0.9 where the sky is open.
        let thundering = self.can_have_weather() && self.common.world.weather.levels(1.0).1 > 0.9;
        let sky_darken = self.sky_darken();
        let ultrawarm = self.dimension_type().water_evaporates;
        // Whether the picks ask for the mod block parts' `animate_tick`.
        let parts = self.mod_animate_parts();
        let block = self
            .common
            .eye_position()
            .map(|e| e.map(|v| v.floor() as i32));
        let mut sampling = block.map(AnimateSampling::new);
        let sea_level = self.weather_sea_level();
        let biome_zoom = self.client.streaming.biome_zoom;
        loop {
            let client_pack = &self.client.client_pack;
            let marker_options = |state| {
                // BlockMarker: the particle sprite, untinted.
                let sprite = Self::block_particle_sprite(client_pack, state)?;
                Some(ParticleOptions::block(
                    ParticleType::BlockMarker,
                    sprite,
                    0xFFFFFF,
                ))
            };
            let world = &self.common.world;
            let facts = self.common.res.facts();
            let biomes = world_biomes!(self.common);
            let environment = Self::level_environment(
                self.client
                    .remote
                    .as_ref()
                    .and_then(|r| r.level_environment.as_deref()),
                &self.common.res,
                self.common.dim.id(),
                &self.dimension_type().id,
            );
            let ui = &self.common.ui;
            let remote = self.client.remote.as_ref();
            let clock = |name: &str| {
                if let Some(remote) = remote {
                    return remote.named_clock_time(name);
                }
                let active = if ui.active_clock.is_empty() {
                    "minecraft:overworld"
                } else {
                    &ui.active_clock
                };
                if name == active {
                    world.day_time
                } else if name == "minecraft:the_end" {
                    world.end_clock
                } else {
                    ui.command_clocks.get(name).map_or(0, |c| c.0)
                }
            };
            let ambient_particles = |[x, y, z]: [i32; 3]| {
                // AMBIENT_PARTICLES isn't spatially interpolated and reads the noise biome.
                let biome = world
                    .noise_biome(x, y, z)
                    .map(|id| &biomes.get(id).attributes);
                let sample = sunrise_level::environment::Sample {
                    day_time: world.day_time,
                    rain,
                    thunder,
                    biome,
                    ..Default::default()
                };
                let Some(AttributeValue::AmbientParticles(entries)) =
                    environment.value_at(ambient_attribute, &sample, &clock)
                else {
                    unreachable!("ambient particles have an empty list as their default")
                };
                entries
            };
            let properties = &self.common.pack.assets.properties;
            let raining = rain > 0.2;
            let raining_at = |p: [i32; 3]| {
                raining
                    && motion_blocking_height(world, properties, p[0], p[2]) <= p[1]
                    && precipitation_at(
                        world,
                        biomes,
                        biome_zoom,
                        BlockPos::new(p[0], p[1], p[2]),
                        sea_level,
                    ) == Some(false)
            };
            let height_no_leaves =
                |p: [i32; 3]| motion_blocking_leaves(world, facts, p[0], p[2], i32::MAX, false);
            let tables = &self.client.client_pack.context.mesh.tables;
            let model_offset = |state, x, z| tables.offset(state, x, z);
            let client_res =
                crate::game::client_side::ClientState::expect_client_res(&self.client.client_res);
            let gateway_particles = |state, p| {
                client_res
                    .block_entity_render()
                    .gateway_faces(world, state, p)
                    .len()
            };
            let ctx = AnimateContext {
                enchant_power: &self.client.ambience.enchant_power,
                world,
                facts,
                collision: self.common.collision,
                biomes,
                registry: self.common.pack.assets.registry,
                raining_at: &raining_at,
                height_no_leaves: &height_no_leaves,
                firefly_bush_sounds,
                creaking_active,
                sky_darken,
                marker: marker.map(|name| (name, &marker_options as &dyn Fn(_) -> _)),
                ambient_particles: Some(&ambient_particles),
                model_offset: &model_offset,
                gateway_particles: &gateway_particles,
                thundering,
                ultrawarm,
                block_parts: parts
                    .then_some(&Self::animate_part_listens as &dyn Fn(BlockStateId) -> bool),
            };
            if let Some(sampling) = sampling.as_mut() {
                if !sampling.started() {
                    laps.lap("tp_ctx");
                    self.client
                        .particles
                        .0
                        .set_polytone(self.client.client_pack.polytone_particles.clone());
                    if let Some(registry) = &self.client.client_pack.polytone_particles {
                        registry.set_biomes(biomes);
                    }
                }
                if let Some(step) =
                    sampling.run(client_res.animate(), &mut self.client.particles.0, &ctx)
                {
                    match step {
                        AnimateStep::BlockPart(state, pos) => self.mod_animate_tick(state, pos),
                        AnimateStep::Ambient(options, at) => self.emit_model_ambient(&options, at),
                        AnimateStep::BookShelf {
                            table,
                            offset,
                            mut valid,
                        } => {
                            if let Some(level) = self.native_client_level() {
                                let [x, y, z] = table;
                                let [ox, oy, oz] = offset;
                                self.native_is_valid_book_shelf(
                                    level,
                                    BlockPos::new(x, y, z),
                                    BlockPos::new(ox, oy, oz),
                                    &mut valid,
                                );
                            }
                            sampling.answer_book_shelf(valid);
                        }
                    }
                    continue;
                }
                laps.lap("tp_animate");
            }
            break;
        }
    }

    /// The ambient options whose providers need the game's model or overlay state.
    pub fn emit_model_ambient(&mut self, particle: &PersistentOptions, at: [f64; 3]) {
        let override_limiter = match particle {
            PersistentOptions::Block(ty, _) => ty.override_limiter(),
            PersistentOptions::Item(_) => ParticleType::Item.override_limiter(),
            PersistentOptions::ElderGuardian => true,
            _ => unreachable!("directly drawable ambient options are handled by sampling"),
        };
        if !self
            .client
            .particles
            .0
            .admit_particle_at(at, override_limiter, false)
        {
            return;
        }
        let options = match particle {
            PersistentOptions::Block(ty, state) => {
                block_state_id(self.common.pack.assets.registry, state).and_then(|state| {
                    self.block_provider_options(*ty, state, at.map(|x| x.floor() as i32))
                })
            }
            PersistentOptions::Item(tag) => sunrise_items::item_nbt::item_from_nbt(tag)
                .filter(|stack| self.items().get(&stack.item).is_some())
                .map(|stack| self.item_particle(&stack)),
            PersistentOptions::ElderGuardian => {
                self.client.particles.0.elder_guardian();
                return;
            }
            _ => unreachable!(),
        };
        if let Some(options) = options {
            let level = sunrise_client::particles::ParticleLevel::new(
                &self.common.world,
                self.common.res.facts(),
                self.common.collision,
            );
            self.client.particles.0.add_admitted_with_level(
                Some(&level),
                &options,
                at,
                [0.0; 3],
                override_limiter,
            );
        }
    }

    /// `ParticleEngine.tick` (and the pickups and sounds it raises): in `Minecraft.tick` after
    /// `Level.tickBlockEntities`, so a particle a block entity just added ages with the rest.
    pub fn tick_particle_engine(&mut self) {
        let mut laps = sunrise_level::spans::laps();
        let world = &self.common.world;
        let facts = self.common.res.facts();
        // SpellParticle.isCloseToScopingPlayer: the eye of a local player looking
        // through a spyglass in first person.
        let scope_eye = self
            .common
            .player
            .as_ref()
            .filter(|_| self.client.camera_type.is_first_person() && self.scoping())
            .map(|p| {
                [
                    p.position.x,
                    p.position.y + f64::from(p.dimensions.eye_height),
                    p.position.z,
                ]
            });
        self.client.particles.0.set_scope_eye(scope_eye);
        let level = ParticleLevel {
            world,
            facts,
            collision: self.common.collision,
            player: self
                .common
                .player
                .as_ref()
                .map(|p| ([p.position.x, p.position.y, p.position.z], p.velocity.y)),
            cached_chunk: std::cell::Cell::new(None),
        };
        laps.lap("tp_level");
        // Resource packs' Polytone entity emitters on the player (#11486): fog, dust and
        // weather around them; nothing without such a pack.
        if let Some(p) = &self.common.player {
            let feet = [p.position.x, p.position.y, p.position.z];
            self.client
                .particles
                .0
                .polytone_entity(&level, "minecraft:player", feet);
        }
        let bodies = self
            .client
            .particles
            .0
            .sample_tracking_bodies(|target| match target {
                TrackingTarget::LocalPlayer => {
                    self.common.player.as_ref().map(player_tracking_body)
                }
                TrackingTarget::Entity(id) => {
                    if self.client.remote.is_some() {
                        return self.remote_entity_tracking_body(id);
                    }
                    self.common
                        .mob_index
                        .find_ref(&self.common.mobs, id as u32)
                        .map(|i| mob_tracking_body(&self.common.mobs[i]))
                }
            });
        self.client.particles.0.tick_tracking(&level, bodies);
        laps.lap("tp_engine");
        self.tick_pickups();
        laps.lap("tp_pickups");
        let camera = self.client.particles.0.camera;
        for (name, at, volume, pitch, delayed) in
            std::mem::take(&mut self.client.particles.0.sounds)
        {
            if delayed {
                self.play_audio_distance_delayed(name, at, volume, pitch, camera);
            } else {
                self.play_audio(name, Some(at), volume, pitch);
            }
        }
    }

    /// The ticks the frame shows (animated textures, the HUD, a shader pack's clock): the
    /// client's, or under `SUNRISE_STILL` (`FixedStep::still`) the world's game time, which
    /// moves only while the world runs.
    pub(in crate::game) fn visual_ticks(&self) -> u64 {
        if self.common.clock.still() {
            self.common.world.game_time.max(0) as u64
        } else {
            self.common.ticks
        }
    }

    /// The clock of the animated textures: `Minecraft.runTick` ticks the `TextureManager`
    /// only while the level runs normally. Under `SUNRISE_STILL` it is the world's ticks.
    pub(in crate::game::client_side) fn texture_clock(&self) -> u64 {
        if self.common.clock.still() {
            self.visual_ticks()
        } else {
            self.client.texture_ticks
        }
    }

    /// `Util.getMillis` for what the frame shows (the world border's scroll, the hearts' blink,
    /// the glint's under the switch): under `SUNRISE_STILL`, the world's ticks in milliseconds.
    pub(in crate::game::client_side) fn visual_millis(&self) -> i64 {
        if self.common.clock.still() {
            self.visual_ticks() as i64 * 50
        } else {
            self.client.started.elapsed().as_millis() as i64
        }
    }
}
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// What the packs' Polytone animation modes read this tick (wiki "Variant Textures"): the
    /// time of day, the weather, and for expressions the packs' Polytone frame, or the time and
    /// weather alone when no Polytone pack makes one.
    fn update_animation_inputs(&mut self) {
        let (rain, thunder) = self.view_weather(0.0);
        let day_time = self.view_day_time();
        let frame = self
            .client
            .client_pack
            .polytone_particles
            .as_ref()
            .and_then(|p| p.frame.read().ok().map(|f| f.clone()))
            .unwrap_or_else(|| sunrise_level::environment::Frame {
                rain,
                thunder,
                game_time: self.common.world.game_time,
                day_time,
                ..Default::default()
            });
        // `screen_time` counts while the same screen stays open.
        let screen = (self.client.screen != super::Screen::Playing).then(|| {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            std::mem::discriminant(&self.client.screen).hash(&mut hasher);
            hasher.finish()
        });
        if let Some(renderer) = &mut self.client.renderer.0 {
            renderer.set_animation_inputs(day_time, [rain, thunder], Some(&frame), screen);
        }
    }
}

/// ClientLevel.tickWeatherEffects uses the game time already advanced by tickTime.
#[cfg(feature = "client")]
pub(in crate::game::client_side) fn tick_client_weather_effects(
    game: &mut crate::game::ClientGame<'_>,
) {
    let mut laps = sunrise_level::spans::laps();
    let cam_pos = game.client.view.position;
    let rain = game.weather_levels(1.0).0;
    // A mod's `tick_weather_effects` may take this tick's weather effects over.
    let weather_canceled = mod_api::__private::listened(mod_api::HookId::TickWeatherEffects) && {
        let mut canceled = false;
        game.native_tick_weather_effects(
            game.common.dim.id(),
            game.common.world.game_time,
            cam_pos,
            rain,
            &mut canceled,
        );
        canceled
    };
    if weather_canceled
        || rain <= 0.0
        || !game.client.settings.sunrise.weather
        || game.common.eye_position().is_none()
    {
        return;
    }
    let world = &game.common.world;
    let properties = &game.common.pack.assets.properties;
    let facts = game.common.res.facts();
    let biomes = world_biomes!(game.common);
    let sea_level = game.weather_sea_level();
    let biome_zoom = game.client.streaming.biome_zoom;
    let weather_block = cam_pos.map(|v| v.floor() as i32);
    // ClientLevel.tickWeatherEffects: rain splashes around the camera, and the rain sound.
    let mut random = LegacyRandom::new(game.common.world.game_time.wrapping_mul(312_987_231));
    let radius = game.client.settings.weather_radius;
    let diameter = 2 * radius + 1;
    let count = (0.225 * (diameter * diameter) as f32 * rain * rain) as i32
        / if game.client.settings.particles == 1 {
            2
        } else {
            1
        };
    let mut rain_position = None;
    for _ in 0..count {
        let x = weather_block[0]
            .wrapping_add(random.next_int_bound(diameter).unwrap_or(0))
            .wrapping_sub(radius);
        let z = weather_block[2]
            .wrapping_add(random.next_int_bound(diameter).unwrap_or(0))
            .wrapping_sub(radius);
        let Some(top) = splash_height(world, properties, x, weather_block[1], z) else {
            continue;
        };
        if precipitation_at(
            world,
            biomes,
            biome_zoom,
            BlockPos::new(x, top, z),
            sea_level,
        ) != Some(false)
        {
            continue;
        }
        let below = top.wrapping_sub(1);
        rain_position = Some([x, below, z]);
        // The Minimal option stops at the first spot that rains (the
        // rain sound still plays there).
        if game.client.settings.particles >= 2 {
            break;
        }
        let (bx, bz) = (random.next_double(), random.next_double());
        let state = world.block(x, below, z);
        let f = facts.get(state);
        let block_top = game
            .common
            .collision
            .cached_shapes()
            .cached_collision_top(state, [x, below, z], bx, bz)
            .ok()
            .flatten()
            .unwrap_or(0.0);
        let fluid_top = if f.fluid == Fluid::None {
            0.0
        } else {
            f64::from(f.fluid_level) / 9.0
        };
        let smoke = f.fluid == Fluid::Lava
            || &*f.name == "minecraft:magma_block"
            || (f.name.ends_with("campfire")
                && game.client_res().animate().is_lit_campfire(state.0));
        let ty = if smoke {
            ParticleType::Smoke
        } else {
            ParticleType::Rain
        };
        let p = [
            f64::from(x) + bx,
            f64::from(below) + block_top.max(fluid_top),
            f64::from(z) + bz,
        ];
        game.client
            .particles
            .0
            .add(&ParticleOptions::simple(ty), p, [0.0; 3]);
    }
    // The last rain column of the tick makes the sound, more often the
    // longer it has been quiet; indoors it is the quieter one.
    if let Some(at) = rain_position {
        let waited = game.client.rain_sound_time;
        game.client.rain_sound_time += 1;
        if random.next_int_bound(3).unwrap_or(0) < waited {
            game.client.rain_sound_time = 0;
            let roof =
                motion_blocking_height(world, properties, weather_block[0], weather_block[2]);
            let (name, volume, pitch) =
                if at[1] > weather_block[1].wrapping_add(1) && roof > weather_block[1] {
                    ("minecraft:weather.rain.above", 0.1, 0.5)
                } else {
                    ("minecraft:weather.rain", 0.2, 1.0)
                };
            let p = at.map(|c| f64::from(c) + 0.5);
            // playLocalSound: guests make their own rain, so a host
            // must not capture it for them.
            game.play_audio_unshared(name, Some(p), volume, pitch);
        }
    }
    laps.lap("tp_rain");
}

/// The packs' expression uniforms on core shaders (`shader_modifiers`, wiki "Shaders").
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Whether an installed pack core shader reads expression uniforms this frame. Fullbright
    /// excludes the lightmap only, and a frame where no world runs the text
    /// ([`Self::world_runs`]). The frame's expression context is made only when needed.
    pub(in crate::game::client_side) fn reads_pack_expressions(&mut self) -> bool {
        let world = self.world_runs();
        (self.client.renderer.0.as_mut()).is_some_and(|r| r.reads_pack_expressions(world))
    }

    /// Whether an installed pack core shader reads Polytone's `PolyGlobals` this frame (the
    /// text's only where a world runs).
    pub(in crate::game::client_side) fn reads_poly_globals(&mut self) -> bool {
        let world = self.world_runs();
        (self.client.renderer.0.as_mut()).is_some_and(|r| r.reads_poly_globals(world))
    }

    /// The frame's `PolyGlobals` for the pack core shaders that read them; none leaves them 0.
    pub(in crate::game::client_side) fn give_poly_globals(
        &mut self,
        globals: Option<&sunrise_client::polytone_post::PolyGlobals>,
    ) {
        let world = self.world_runs();
        if let Some(renderer) = self.client.renderer.0.as_mut() {
            renderer.set_poly_globals(globals.map(|g| g.bytes()).as_ref(), world);
        }
    }

    /// Evaluate the installed pack pipelines' expression uniforms from `frame`, skipping the
    /// lightmap under fullbright, and the text's where no world runs
    /// ([`Self::world_runs`]). Values are uploaded when each pipeline prepares its draw.
    pub(in crate::game::client_side) fn evaluate_pack_expressions(
        &mut self,
        frame: Option<&sunrise_level::environment::Frame>,
    ) {
        let world = self.world_runs();
        if let (Some(frame), Some(renderer)) = (frame, self.client.renderer.0.as_mut()) {
            renderer.evaluate_pack_expressions(frame, world);
        }
    }

    /// Whether the text's Polytone blocks are given this frame: only where a world runs. On a
    /// title frame (the title screen and its menus) the renderer writes them as 0, as it does on
    /// the loading, file fixer and saving screens' frames.
    fn world_runs(&self) -> bool {
        !self.shows_panorama()
    }
}

use sunrise_gameplay_common::game::end_common::EndGameExt as _;

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::weather::WeatherGameExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

#[cfg(feature = "client")]
#[cfg(test)]
mod tests {
    use super::{Properties, World, splash_height};
    use std::sync::Arc;
    use sunrise_level::chunk::Chunk;
    use sunrise_world::registry::BlockStateId;

    /// #3810: under a stone roof more than 64 blocks above the camera, as in a
    /// deep excavation, MOTION_BLOCKING is the roof, so no rain splash lands on
    /// the floor; with the roof gone, it does.
    #[test]
    fn no_rain_splash_under_a_high_roof() {
        // State 0 is air, 1 is stone.
        let properties =
            Properties::for_tests(&[(true, false, false, false), (false, false, true, true)]);
        let stone = BlockStateId(1);
        let mut world = World::new(BlockStateId(0), 0);
        world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        for x in 0..16 {
            for z in 0..16 {
                world.set(x, 0, z, stone).unwrap();
            }
        }
        // The camera stands on the floor, its block at y 1.
        assert_eq!(splash_height(&world, &properties, 3, 1, 3), Some(1));
        for roof in [70, 80, 200] {
            world.set(3, roof, 3, stone).unwrap();
            assert_eq!(
                splash_height(&world, &properties, 3, 1, 3),
                None,
                "roof at {roof}"
            );
            world.set(3, roof, 3, BlockStateId(0)).unwrap();
        }
        assert_eq!(splash_height(&world, &properties, 3, 1, 3), Some(1));
        // A roof within 10 blocks is where the splash lands.
        world.set(3, 8, 3, stone).unwrap();
        assert_eq!(splash_height(&world, &properties, 3, 1, 3), Some(9));
    }

    /// `extract_block_screen_effect`: a mod that cancels the overlay of stone at one position
    /// leaves the frame's block overlays as they are with no block there, the fire overlay
    /// included; dirt in the same place keeps its overlay.
    #[test]
    #[ignore = "daily"]
    fn a_mod_cancels_the_screen_effect_of_one_block() {
        use mod_api::{
            BlockScreenEffect, ClientHost, ExtractBlockScreenEffectEvent, Flow, Hooks, ModEntry,
            Mods,
        };
        use sunrise_core::{geometry::Vec3, position::BlockPos};
        static SEEN: std::sync::Mutex<Vec<(BlockScreenEffect, BlockStateId, BlockPos)>> =
            std::sync::Mutex::new(Vec::new());
        struct Mod(BlockStateId);
        #[mod_api::__subscriptions]
        impl Hooks for Mod {
            fn extract_block_screen_effect(
                &self,
                _: &dyn ClientHost,
                event: &mut ExtractBlockScreenEffectEvent<'_>,
            ) -> Flow {
                SEEN.lock().unwrap().push((
                    event.effect,
                    sunrise_gameplay_common::game::api_values::TryIntoEngine::try_into_engine(
                        &event.state,
                        &(),
                    )
                    .unwrap(),
                    event.pos,
                ));
                if event.effect == BlockScreenEffect::Block
                    && sunrise_gameplay_common::game::api_values::TryIntoEngine::try_into_engine(
                        &event.state,
                        &(),
                    )
                    .unwrap()
                        == self.0
                    && event.pos == BlockPos::new(0, 65, 0)
                {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
        }
        super::super::native_entity_refs::tests::with_client_game(|game| {
            let registry = &game.pack.assets.registry;
            let state = |name: &str| registry.default_state(name).unwrap();
            let (air, stone, dirt, fire) = (
                state("minecraft:air"),
                state("minecraft:stone"),
                state("minecraft:dirt"),
                state("minecraft:fire"),
            );
            game.common.world.chunks.insert(
                (0, 0),
                Arc::new(Chunk::with_layout(0, 0, game.common.world.layout())),
            );
            // The eye is at y 65.62, so all eight test points are in block (0, 65, 0).
            let mut player =
                sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            player.position = Vec3::new(0.5, 64.0, 0.5);
            player.fire_ticks = 100;
            game.player = Some(player);
            let effects = |game: &mut crate::game::ClientGame<'_>, block| {
                game.world.set(0, 65, 0, block).unwrap();
                let p = game.player.as_ref().unwrap();
                let eye = [0.5, 64.0 + f64::from(p.dimensions.eye_height), 0.5];
                let effects = game.block_screen_effects(p, eye, 0.0, [854, 480], 1.0);
                assert!(effects.custom.iter().all(Option::is_none));
                (effects.block, effects.water, effects.on_fire)
            };
            let open = effects(game, air);
            let in_dirt = effects(game, dirt);
            assert_eq!(effects(game, stone), (Some(stone), false, true));
            let mods: Mods = vec![ModEntry::new("screen-effect", Box::new(Mod(stone)))].into();
            mod_platform::testing::with_mods(mods, || {
                assert_eq!(effects(game, stone), open);
                assert_eq!(effects(game, dirt), in_dirt);
            });
            assert_eq!(in_dirt, (Some(dirt), false, true));
            assert_eq!(open, (None, false, true));
            let seen = SEEN.lock().unwrap();
            let feet = BlockPos::new(0, 64, 0);
            assert_eq!(
                seen[..],
                [
                    (BlockScreenEffect::Block, stone, BlockPos::new(0, 65, 0)),
                    (BlockScreenEffect::Fire, fire, feet),
                    (BlockScreenEffect::Block, dirt, BlockPos::new(0, 65, 0)),
                    (BlockScreenEffect::Fire, fire, feet),
                ]
            );
        });
    }
}
