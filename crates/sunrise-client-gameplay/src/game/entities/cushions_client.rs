use super::entity_culling::default_range;
use sunrise_core::geometry::Aabb;
use sunrise_gameplay_common::game::cushions_common::{aabb, snap_yaw};
use sunrise_sim::mobs::anim::Poses;

impl crate::game::ClientGame<'_> {
    /// `CushionRenderer`: the coloured model, rotated to the nearest
    /// direction, with its bottom on the cushion's position.
    pub(in crate::game::client_side) fn cushion_geometry(
        &self,
        camera: glam::DVec3,
        out: &mut Vec<sunrise_client_render::render_geometry::Vertex>,
    ) {
        let Some(model) = self.client.cushion_model.as_ref() else {
            return;
        };
        let sprites = &self.client.gui.atlas.sprites;
        for c in &self.common.cushions {
            if self.hides_as_camera(c.id) {
                continue;
            }
            let a = aabb(c.bounds());
            let (lo, hi) = (a.min_position(), a.max_position());
            let (lo, hi) = (
                glam::DVec3::new(lo.x, lo.y, lo.z),
                glam::DVec3::new(hi.x, hi.y, hi.z),
            );
            let range = default_range(hi - lo, self.view_scale());
            if !self.entity_in_view(camera, (lo + hi) / 2.0, range, lo, hi) {
                continue;
            }
            let Some(uv) = sprites
                .get(&format!("entity/cushion/{}_cushion", c.color_name()))
                .map(|r| [r.u0, r.v0, r.u1, r.v1])
            else {
                continue;
            };
            let culling = self.entity_cull_decision(c, 1.0);
            if culling.hides_entity() {
                continue;
            }
            let entity_start = out.len();
            let p = glam::DVec3::new(c.position.x, c.position.y, c.position.z) - camera;
            let base = glam::Mat4::from_translation(p.as_vec3())
                * glam::Mat4::from_rotation_y((180.0 - snap_yaw(c.yaw)).to_radians())
                * glam::Mat4::from_rotation_x(std::f32::consts::PI)
                * glam::Mat4::from_translation(glam::Vec3::new(0.0, -0.25, 0.0));
            // `EntityRenderer.getPackedLightCoords` probes at the eye height.
            let l = self.common.world.light(
                c.position.x.floor() as i32,
                (c.position.y + 0.2125).floor() as i32,
                c.position.z.floor() as i32,
            );
            let packed = (u32::from(l >> 4) * 16) | ((u32::from(l & 15) * 16) << 8);
            sunrise_client::mobs::render::draw(
                model,
                base,
                &Poses::new(model),
                uv,
                0xFFFF_FFFF,
                packed,
                0,
                out,
            );
            self.finish_entity_occlusion(culling, c, out, entity_start);
        }
    }
    /// `BlockState.isViewBlocking(level, pos, nearPlaneBox)`: stairs and
    /// slabs (`NEAR_PLANE_INTERSECTS_OUTLINE`) block when the near plane
    /// touches a box of their shape, snow only at eight layers, anything else
    /// when it is suffocating.
    pub(in crate::game::client_side) fn state_view_blocking(
        &self,
        state: sunrise_world::registry::BlockStateId,
        pos: [i32; 3],
        near_plane: Aabb,
    ) -> bool {
        let name = self.common.res.facts().get(state).name.as_ref();
        if name == "minecraft:snow" {
            return self
                .common
                .pack
                .assets
                .registry
                .state(state)
                .and_then(|s| s.properties.get("layers"))
                .is_some_and(|l| l.parse::<i32>().is_ok_and(|l| l >= 8));
        }
        if (name.ends_with("_stairs") || name.ends_with("_slab"))
            && name != "minecraft:petrified_oak_slab"
        {
            return self
                .outline_boxes(pos)
                .iter()
                .any(|b| aabb(([b[0], b[1], b[2]], [b[3], b[4], b[5]])).intersects(near_plane));
        }
        self.common.state_suffocating(state)
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;
