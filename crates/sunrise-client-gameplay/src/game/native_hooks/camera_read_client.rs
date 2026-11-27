use super::*;
use sunrise_gameplay_common::game::native_hooks::camera_read::PROJECT_VIEW;
use sunrise_gameplay_common::game::native_hooks::camera_read::ProjectionFrame;

/// Helpers for native mods, kept as stable as the hook points.
impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn activate_projection_frame(&self) -> ProjectionFrame {
        ProjectionFrame(PROJECT_VIEW.replace(self as *const Self as usize))
    }
    /// `Minecraft.setCameraEntity`: follows the local player or a tracked mob, else false.
    pub(in crate::game) fn set_camera_entity(&mut self, entity: EntityRef) -> bool {
        use sunrise_gameplay_common::game::commands_common::Target;
        let id = match self.native_client_entity_target(entity) {
            Some(Target::Player | Target::Mob(_)) => entity.id() as i32,
            _ => return false,
        };
        self.remote_set_camera(id);
        true
    }
    /// The most recently extracted world frame's camera, if one exists.
    /// During GUI rendering this is the current frame, including bob/spin and FOV.
    pub(in crate::game) fn camera(&self) -> Option<mod_api::Camera> {
        self.client.native_camera_frame.clone()
    }
    /// The entity the view follows (`Minecraft.getCameraEntity`): the entity a server attached
    /// the camera to, else the local player. None outside a client world.
    pub(in crate::game) fn camera_entity(&self) -> Option<EntityRef> {
        if !self.common.native_window() || self.client.title.0.is_some() {
            return None;
        }
        self.common.player.as_ref()?;
        let id = self
            .remote_camera_id()
            .unwrap_or_else(|| self.own_entity_id() as u32);
        Some(EntityRef::new(self.common.native_lifecycle.epoch()?, id))
    }
    /// `GameRenderer.projectPointToScreen` against this frame's camera; None behind the camera,
    /// for a non-finite point, outside a world and before the first frame.
    pub(in crate::game) fn project(&self, at: [f64; 3]) -> Option<[f32; 3]> {
        if PROJECT_VIEW.get() != self as *const Self as usize {
            return None;
        }
        let camera = self.client.native_camera_frame.as_ref()?;
        sunrise_client_render::camera::project_point_rolled(
            &camera.projection,
            camera.position,
            camera.yaw,
            camera.pitch,
            camera.roll,
            at,
        )
    }
}

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
