//! The locator bar on this client (sunrise_client::locator_bar): the
//! waypoints a server tracks for the player (`handleWaypoint`; on a served
//! world the host's own connection in lan/waypoints.rs), the experience
//! bar's priority after a change, and the dots drawn from the last frame's
//! camera.
use crate::game::ClientGame;
use sunrise_client::{
    hud_layout::GuiDraw,
    locator_bar::{ClientWaypoints, View},
};
use sunrise_protocol::packets::play::TrackedWaypoint;

impl ClientGame<'_> {
    /// `ClientPacketListener.handleWaypoint`.
    pub fn handle_waypoint(&mut self, packet: &TrackedWaypoint) {
        self.client.waypoints.apply(packet);
    }
    /// A new connection or world starts with no waypoints.
    pub fn clear_waypoints(&mut self) {
        self.client.waypoints = ClientWaypoints::default();
    }
    /// `LocalPlayer.setExperienceValues`: a change of progress shows the
    /// experience bar for 100 ticks, except the first one
    /// (`experienceDisplayStartTick` starts at `Integer.MIN_VALUE`).
    pub(in crate::game::client_side) fn track_experience_display(&mut self) {
        let progress = self.common.experience.progress;
        let (last, seen, _) = &mut self.client.experience_display;
        if progress != *last {
            *last = progress;
            if *seen {
                self.client.experience_display.2 = self.visual_ticks().max(1);
            } else {
                *seen = true;
            }
        }
        // EnchantmentScreen and AnvilScreen.containerTick keep it up.
        if self.common.station.as_ref().is_some_and(|s| {
            matches!(
                s.kind,
                super::stations::Kind::Enchanting | super::stations::Kind::Anvil
            )
        }) {
            self.client.experience_display.2 = self.visual_ticks().max(1);
        }
    }
    /// `Hud.willPrioritizeExperienceInfo`. A start of 0 is none: the stamps above are at least 1
    /// (a new world under `SUNRISE_STILL` is still at tick 0).
    pub(in crate::game::client_side) fn experience_recent(&self) -> bool {
        let (_, _, start) = self.client.experience_display;
        start != 0 && start + 100 > self.visual_ticks()
    }
    /// `Hud.nextContextualInfoState`: jump bar reads `LocalPlayer.jumpableVehicle`, not `getCameraPlayer`.
    pub fn contextual_bar(&self) -> sunrise_client::locator_bar::ContextualBar {
        sunrise_client::locator_bar::ContextualBar::next(
            self.has_waypoints(),
            self.jumpable_mount().is_some(),
            self.mount_jumping(),
            self.can_hurt_player(),
            self.experience_recent(),
        )
    }
    /// `LocalPlayer.jumpableVehicle` as a handle: the mount whose jump bar `contextual_bar` shows.
    pub(in crate::game) fn jumpable_vehicle(&self) -> Option<mod_api::EntityRef> {
        self.entity(self.jumpable_mount()?)
    }
    /// The local player's jump bar charge.
    pub(in crate::game) fn jump_riding_scale(&self) -> f32 {
        self.common.mount.jump_scale
    }
    /// `ClientWaypointManager.hasWaypoints`: what `contextual_bar` reads for the locator bar.
    pub(in crate::game) fn has_waypoints(&self) -> bool {
        self.client.waypoints.has_waypoints()
    }
    /// `Vec3iWaypoint.position`: a live client's entity, with `Entity.getEyePosition` at its own partial tick.
    pub fn waypoint_entity(&self, uuid: u128) -> Option<([i32; 3], [f64; 3])> {
        let (position, previous, eye_height, frozen) = if let Some(p) = &self.common.player
            && self.own_profile_uuid() == Some(uuid)
        {
            let vec = |v: sunrise_core::geometry::Vec3| [v.x, v.y, v.z];
            (
                vec(p.position),
                vec(self.common.previous_position),
                f64::from(p.dimensions.eye_height),
                false,
            )
        } else if let Some(remote) = &self.client.remote {
            let &id = remote.level_uuids.get(&uuid)?;
            let entity = self.live_entity_camera_subject(remote, id)?;
            // TickRateManager.isEntityFrozen: players and entities carrying them keep moving.
            let frozen = !self.common.clock.runs_normally()
                && !entity.is_player
                && !entity.has_player_passenger;
            (
                entity.camera.position,
                entity.camera.previous,
                f64::from(entity.camera.eye_height),
                frozen,
            )
        } else {
            return None;
        };
        let alpha = if frozen {
            self.common.clock.fraction()
        } else {
            self.common.clock.camera_fraction()
        };
        let mut eyes = std::array::from_fn(|i| previous[i] + alpha * (position[i] - previous[i]));
        eyes[1] += eye_height;
        Some((position.map(|c| c.floor() as i32), eyes))
    }
    /// `LocatorBar.extractRenderState`: distance and self-exclusion from the camera entity,
    /// angles and projection from the last frame's render camera.
    pub fn locator_bar_draws(&self, gui_width: i32, gui_height: i32) -> Vec<GuiDraw> {
        let Some((camera_pos, yaw, pitch, fov)) = self.client.locator_camera else {
            return Vec::new();
        };
        let Some((camera_uuid, camera_entity_pos)) = self.camera_identity_and_position() else {
            return Vec::new();
        };
        // Camera.getViewRotationProjectionMatrix, through view space: the
        // depth along the look direction and the height on screen.
        let (sy, cy) = (
            f64::from(yaw.to_radians()).sin(),
            f64::from(yaw.to_radians()).cos(),
        );
        let (sp, cp) = (
            f64::from(pitch.to_radians()).sin(),
            f64::from(pitch.to_radians()).cos(),
        );
        let forward = [-sy * cp, -sp, cy * cp];
        let up = [-sy * sp, cp, cy * sp];
        let focal = 1.0 / f64::from((fov / 2.0).to_radians()).tan();
        let project = |o: [f64; 3]| {
            let dot = |a: [f64; 3]| a[0] * o[0] + a[1] * o[1] + a[2] * o[2];
            let depth = dot(forward);
            let y = dot(up) * focal / depth;
            // Points behind the camera land past the far plane (z > 1).
            [0.0, y, if depth < 0.0 { 2.0 } else { 0.5 }]
        };
        let entity = |uuid| self.waypoint_entity(uuid);
        let view = View {
            gui_width,
            gui_height,
            camera_uuid,
            camera_entity_pos,
            camera_pos,
            yaw,
            pitch,
            fov,
            project: &project,
            entity: &entity,
            styles: &self.client.waypoint_styles,
            tick: self.visual_ticks(),
        };
        self.client.waypoints.draw(&view)
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
