//! `ThrownItemRenderer`: a thrown potion, experience bottle or content mod's thrown type drawn as
//! its item, facing the camera; or, for a thrown type a mod registered a renderer for
//! (`register_entity_renderers`), drawn by that renderer instead.
use crate::game::ClientGame;
use sunrise_assets::item_model::DisplayContext;
use sunrise_client::mesher::Vertex;
use sunrise_sim::projectiles::{Projectile, ProjectileKind};

impl ClientGame<'_> {
    /// Draws the thrown item projectile `a` at `at` (the frame's position; `c` the same relative
    /// to the camera), lit by `packed` as the entity pass packs it: true when it was one and is
    /// drawn.
    pub(in crate::game) fn thrown_item_geometry(
        &self,
        a: &Projectile,
        alpha: f64,
        at: [f64; 3],
        camera: glam::DVec3,
        c: glam::Vec3,
        packed: u32,
        out: &mut Vec<Vertex>,
    ) -> bool {
        if let ProjectileKind::Thrown(t) = &a.kind
            && let Some(resolved) = super::native_entity_renderers::thrown_renderer(t)
        {
            let partial = alpha as f32;
            let [width, height, eye_height] = a
                .size_answer
                .filter(|s| s.vanilla == a.kind.dimensions())
                .and_then(|s| s.size)
                .unwrap_or_else(|| a.kind.dimensions());
            let pos = glam::DVec3::from_array(at);
            let entity: mod_api::EntityRenderState = mod_api::__private::built::EntityRenderState {
                id: a.id,
                partial,
                age: a.age as f32 + partial,
                yaw: a.previous_yaw + partial * (a.yaw - a.previous_yaw),
                pitch: a.previous_pitch + partial * (a.pitch - a.previous_pitch),
                position: at,
                size: [width, height],
                eye_height,
                distance_to_camera_sq: (pos - camera).length_squared(),
                living: None,
                tnt_fuse: None,
                orb_value: None,
                painting: None,
                item_frame: None,
                attachments: None,
            }
            .into();
            let block = at.map(|x| x.floor() as i32);
            if self.mod_entity_renderer_geometry(
                &resolved,
                &entity,
                t.id,
                pos - camera,
                Some(pos),
                block,
                packed,
                out,
            ) {
                return true;
            }
        }
        let Some(stack) = a.drawn_item() else {
            return false;
        };
        let owner = super::item_draw::owner_at(at, 0.0, a.kind.id());
        let state = self.resolve_item(&stack, DisplayContext::Ground, Some(&owner), 0);
        let pose =
            glam::Mat4::from_translation(c) * glam::Mat4::from_quat(self.camera_orientation());
        self.submit_item_world(&state, pose, packed, out);
        true
    }
}
