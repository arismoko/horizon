//! Field of view (`Camera.tickFov`, `calculateFov`,
//! `AbstractClientPlayer.getFieldOfViewModifier`), view and hurt bobbing
//! (`GameRenderer.bobHurt`, `bobView`), the portal and Nausea spin
//! (`GameRenderer.renderLevel`) and the spyglass (`SpyglassItem`,
//! `Hud.extractSpyglassOverlay`). The arithmetic is `sunrise_client::camera`.
#[cfg(test)]
#[path = "fov_camera_tests.rs"]
mod camera_tests;

use crate::game::ClientGame;
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::game::item_abilities::ItemAbilitiesGameExt1 as _;
use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;

pub(in crate::game::client_side) fn camera_effects_enabled(
    title: bool,
    studio_freecam: bool,
) -> bool {
    !title && !studio_freecam
}

pub(in crate::game::client_side) fn studio_draws_hand(
    studio_freecam: bool,
    ordinary_hand_visible: bool,
) -> bool {
    !studio_freecam && ordinary_hand_visible
}
use sunrise_client::{
    camera::{BobState, CameraFluid, bob_matrix, field_of_view_modifier, spinning_effect_matrix},
    fog::{FogType, fog_type},
    hud::HudBatch,
    hud_layout, hud_overlays,
};
use sunrise_level::camera_math::{FovPlayer, UseItem, raw_field_of_view_modifier};

fn valid_fov_modifier(modifier: f32, fallback: f32) -> f32 {
    if modifier.is_finite() && (0.01..=10.0).contains(&modifier) {
        modifier
    } else {
        fallback
    }
}

impl ClientGame<'_> {
    /// `Player.isScoping`: the active use began with a spyglass.
    pub(in crate::game) fn scoping(&self) -> bool {
        self.common.using.is_some()
            && self
                .common
                .use_start
                .item
                .as_ref()
                .is_some_and(|s| self.stack_scopes(s))
    }
    /// Whether `stack` looks through as a spyglass does (`spyglass_scope`).
    pub(in crate::game) fn stack_scopes(&self, stack: &sunrise_items::items::ItemStack) -> bool {
        self.stack_can_perform(
            stack,
            sunrise_gameplay_common::game::item_abilities::ability::SPYGLASS_SCOPE,
            &*stack.item == "minecraft:spyglass",
        )
    }
    /// `options.fovEffectScale`.
    pub(in crate::game::client_side) fn fov_effect_scale(&self) -> f32 {
        self.client.settings.file.double("fovEffectScale") as f32
    }
    /// `options.screenEffectScale` (Distortion Effects).
    pub(in crate::game) fn screen_effect_scale(&self) -> f32 {
        self.client.settings.file.double("screenEffectScale") as f32
    }
    /// `LocalPlayer.getEffectBlendFactor(NAUSEA, partial)`: 0 without the effect.
    pub(in crate::game::client_side) fn nausea_blend(&self, partial: f32) -> f32 {
        if self.common.potions.effects.get("nausea").is_none() {
            return 0.0;
        }
        let fx = &self.client.screen_fx;
        fx.o_nausea + partial * (fx.nausea - fx.o_nausea)
    }
    /// The spin `GameRenderer.renderLevel` multiplies into the level's
    /// projection: the larger of the portal and Nausea intensities, scaled by
    /// Distortion Effects squared, turning by `LocalPlayer.getSpinningEffectAngle`.
    pub(in crate::game::client_side) fn spinning_matrix(&self, partial: f32) -> glam::Mat4 {
        let fx = &self.client.screen_fx;
        let portal = fx.o_portal + partial * (fx.portal - fx.o_portal);
        let scale = self.screen_effect_scale();
        let intensity = portal.max(self.nausea_blend(partial)) * (scale * scale);
        spinning_effect_matrix(intensity, fx.spinning_time + partial * fx.spinning_speed)
    }
    /// What `AbstractClientPlayer.getFieldOfViewModifier` reads.
    pub(in crate::game::client_side) fn fov_player(&self) -> Option<FovPlayer> {
        let p = self.common.player.as_ref()?;
        let using = self.common.using.and_then(|(_, ticks)| {
            let stack = self.common.use_start.item.as_ref()?;
            let item = match &*stack.item {
                "minecraft:bow" => UseItem::Bow,
                _ if self.stack_scopes(stack) => UseItem::Spyglass,
                _ => UseItem::Other,
            };
            Some((item, ticks as i32))
        });
        Some(FovPlayer {
            flying: p.flying,
            walking_speed: self.remote_walking_speed(),
            // Player.aiStep keeps `speed` as the float movement speed attribute.
            movement_speed: f64::from(p.speed),
            using,
        })
    }

    /// The handle of the player entity `id` the camera follows, when it is a player this
    /// client knows by uuid (a detached spectated snapshot has none).
    fn camera_player_ref(&self, id: u32) -> Option<mod_api::PlayerRef> {
        let epoch = self.native_lifecycle.epoch()?;
        Some(mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(epoch, id),
            self.remote_player_uuid(id)?,
        ))
    }
    /// `Camera.tick`: the eye height and the field-of-view modifier ease
    /// halfway toward the camera player's each tick.
    pub fn tick_fov(&mut self) {
        let first_person = self.client.camera_type.is_first_person();
        let effect = self.fov_effect_scale();
        let (fov_player, eye, who) = match self.camera_subject() {
            Some(cam) if !cam.is_local_player => (
                cam.fov_player,
                Some(cam.camera.eye_height),
                self.camera_player_ref(cam.entity_id),
            ),
            _ => (
                self.fov_player(),
                self.common.player.as_ref().map(|p| {
                    if self.common.sleeping.is_some() {
                        0.2
                    } else {
                        p.dimensions.eye_height
                    }
                }),
                self.local_player(),
            ),
        };
        let target = fov_player.map_or(1.0, |player| {
            // AbstractClientPlayer.getFieldOfViewModifier returns 0.1 before
            // Mth.lerp / ComputeFovModifierEvent when scoping in first person.
            let Some(raw) = raw_field_of_view_modifier(&player, first_person) else {
                return field_of_view_modifier(&player, first_person, effect);
            };
            let interpolated = field_of_view_modifier(&player, first_person, effect);
            let Some(who) = who else {
                return interpolated;
            };
            let mut modifier = interpolated;
            self.native_compute_fov_modifier(who, raw, effect, &mut modifier);
            valid_fov_modifier(modifier, interpolated)
        });
        self.client.view.tick(eye, target);
    }
    /// The player's `deathTime` while dead or dying: whole ticks counted by
    /// `LivingEntity.tickDeath`, so `deathTime + partialTicks` in
    /// `Camera.extractRenderState` rises smoothly across frames.
    pub(in crate::game::client_side) fn death_time(&self) -> Option<i32> {
        let p = self.common.player.as_ref()?;
        p.health
            .dead_or_dying()
            .then(|| self.client.chat_ui.death_time.min(i32::MAX as u32) as i32)
    }
    /// `modifyFovBasedOnDeathOrFluid`'s death time: the camera entity's, so a spectated entity's.
    fn camera_death_time(&self) -> Option<i32> {
        self.camera_subject()
            .and_then(|c| c.camera.living)
            .and_then(|l| l.dead)
    }
    /// `Camera.getFluidInCamera` for the camera at `camera`, where the parts of each block it
    /// reads may answer the state the camera sees there (`state_at_viewpoint`), and the mods may
    /// then replace the answer (`camera_submersion`). Every client reader of the camera's fluid
    /// reads it.
    pub fn camera_fog_type(&self, camera: [f64; 3]) -> FogType {
        let mut fog_type = self.camera_fog_type_seen(camera);
        self.native_camera_submersion(camera, &mut fog_type);
        fog_type
    }
    /// The fog type at `camera` as the blocks the camera sees there give it. Both paths go
    /// through the replaceable `fog_type`.
    fn camera_fog_type_seen(&self, camera: [f64; 3]) -> FogType {
        let (world, facts, near) = (
            &self.common.world,
            self.common.res.facts(),
            self.near_plane(),
        );
        if !mod_api::__private::has_scoped_hook(mod_api::HookId::StateAtViewpoint) {
            return fog_type(world, facts, camera, &near, &|state, _| state);
        }
        let seen = |state, [x, y, z]: [i32; 3]| {
            if !sunrise_gameplay_common::game::api_values::block_state::block_listens(
                mod_api::HookId::StateAtViewpoint,
                state,
            ) {
                return state;
            }
            let mut seen = state;
            self.native_state_at_viewpoint(state, BlockPos::new(x, y, z), camera, &mut seen);
            seen
        };
        fog_type(world, facts, camera, &near, &seen)
    }
    /// The fluid at the camera, as `Camera.getFluidInCamera` reports it.
    pub fn camera_fluid(&self, camera: [f64; 3]) -> CameraFluid {
        match self.camera_fog_type(camera) {
            FogType::Water => CameraFluid::Water,
            FogType::Lava => CameraFluid::Lava,
            FogType::PowderSnow => CameraFluid::PowderSnow,
            FogType::Atmospheric => CameraFluid::None,
        }
    }
    /// `Camera.calculateFov` with `modifyFovBasedOnDeathOrFluid`.
    pub fn fov_degrees(&self, partial: f32, camera: [f64; 3]) -> f32 {
        if self.client.studio.flying() {
            return self.client.studio.freecam.fov.clamp(1.0, 179.0);
        }
        let vanilla = self.client.view.calculate_fov(
            partial,
            self.client.settings.fov as i32,
            self.fov_effect_scale(),
            self.camera_death_time(),
            self.camera_fluid(camera),
            false,
        );
        let mut degrees = vanilla;
        self.native_compute_fov(partial, camera, &mut degrees);
        if degrees.is_finite() && (1.0..=179.0).contains(&degrees) {
            degrees
        } else {
            vanilla
        }
    }
    /// How many times the view is magnified right now: the player's field of
    /// view over the one actually in use, and 1 when nothing is zooming.
    ///
    /// Reads the result rather than any one zoom, so it covers both the
    /// spyglass (`AbstractClientPlayer.getFieldOfViewModifier`'s tenth while
    /// `isScoping`) and any mod that narrows the field of view through
    /// `native_compute_fov_modifier` or `native_compute_fov`.
    pub(in crate::game::client_side) fn zoom_magnification(&self, camera: [f64; 3]) -> f32 {
        let now = self.fov_degrees(1.0, camera);
        let base = self.client.settings.fov.clamp(1.0, 179.0);
        if now.is_finite() && now > 0.5 {
            (base / now).max(1.0)
        } else {
            1.0
        }
    }
    /// What far terrain should plan for a zoomed view, or `None` when the
    /// `lodZoomDetail` extra is off or nothing is zooming (`lod::ZoomAim`).
    pub(in crate::game::client_side) fn lod_aim(
        &self,
        camera: [f64; 3],
    ) -> Option<sunrise_client::lod::ZoomAim> {
        if !self.client.settings.extras.lod_zoom_detail {
            return None;
        }
        let yaw = self
            .camera_subject()
            .map_or(self.common.yaw, |c| c.camera.y_rot);
        sunrise_client::lod::ZoomAim::new(
            yaw,
            self.fov_degrees(1.0, camera),
            self.zoom_magnification(camera),
        )
    }
    /// `Camera.calculateHudFov`: the first-person hand's field of view.
    pub fn hud_fov_degrees(&self, partial: f32, camera: [f64; 3]) -> f32 {
        self.client.view.calculate_hud_fov(
            partial,
            self.fov_effect_scale(),
            self.camera_death_time(),
            self.camera_fluid(camera),
        )
    }
    /// The pose `GameRenderer.renderLevel` multiplies into the projection:
    /// `bobHurt`, then `bobView` with the View Bobbing option.
    pub fn bob_matrix(&self, partial: f32) -> glam::Mat4 {
        let Some(cam) = self.camera_subject() else {
            return glam::Mat4::IDENTITY;
        };
        let state = BobState {
            living: cam.hurt.map(|h| h.bob(partial)),
            player: cam.avatar_bob.map(|avatar| {
                (
                    avatar.backwards_interpolated_walk_distance(partial),
                    avatar.interpolated_bob(partial),
                )
            }),
        };
        let tilt = self.client.settings.file.double("damageTiltStrength");
        glam::Mat4::from_cols_array(&bob_matrix(&state, tilt, self.effective_view_bobbing()).m)
    }
    /// `AbstractClientPlayer.tick` and `aiStep` (`ClientAvatarState.tick`,
    /// `updateBob`), `rideTick`'s `resetBob` and `LocalPlayer.move`'s walked
    /// distance, after the player's movement this tick. `before` is the
    /// ground state, velocity and position at the start of the tick.
    pub(in crate::game::client_side) fn tick_avatar(
        &mut self,
        before: (bool, [f64; 3], [f64; 3]),
        riding: bool,
    ) {
        let Some(p) = self.common.player.as_ref() else {
            return;
        };
        let (ground, velocity, start) = before;
        let swimming = p.pose == sunrise_sim::pose::Pose::Swimming;
        let alive = !p.health.dead_or_dying();
        let (dx, dz) = (
            (p.position.x - start[0]) as f32,
            (p.position.z - start[2]) as f32,
        );
        self.client.avatar.tick(start);
        self.client
            .avatar
            .update_bob(ground && alive && !swimming, velocity);
        if riding {
            self.client.avatar.reset_bob();
        } else {
            self.client.avatar.add_walked(dx, dz);
        }
    }
    /// `Hud.extractCameraOverlays`: the spyglass scope, its size easing
    /// towards 1.125 by half the frame's game ticks and reset to 0.5 while
    /// not scoping in first person.
    pub(in crate::game::client_side) fn draw_spyglass(&mut self, batch: &mut HudBatch) {
        self.client.scope_scale +=
            0.5 * self.client.frame_ticks * (1.125 - self.client.scope_scale);
        if !self.client.camera_type.is_first_person() {
            return;
        }
        if !self.scoping() {
            self.client.scope_scale = 0.5;
            return;
        }
        let draws = hud_overlays::spyglass(
            batch.width as i32,
            batch.height as i32,
            self.client.scope_scale,
        );
        hud_layout::paint(batch, &self.client.gui.atlas, &draws);
    }
}

#[cfg(test)]
mod tests {
    use super::{camera_effects_enabled, studio_draws_hand, valid_fov_modifier};
    #[test]
    fn modifier_validation() {
        assert_eq!(valid_fov_modifier(0.5, 1.0), 0.5);
        for value in [f32::NAN, f32::INFINITY, 0.0, 10.1] {
            assert_eq!(valid_fov_modifier(value, 1.0), 1.0);
        }
    }

    #[test]
    fn freecam_uses_unshaken_camera_and_player_view_keeps_hands() {
        assert!(!camera_effects_enabled(false, true));
        assert!(camera_effects_enabled(false, false));
        assert!(!camera_effects_enabled(true, false));
        assert!(studio_draws_hand(false, true));
        assert!(!studio_draws_hand(true, true));
        assert!(!studio_draws_hand(false, false));
    }
}
