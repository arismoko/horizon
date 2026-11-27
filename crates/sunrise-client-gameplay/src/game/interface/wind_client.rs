use crate::game::ClientGame;
use sunrise_sim::projectiles::Projectile;

impl ClientGame<'_> {
    /// `WindChargeRenderer`: the spinning charge and its wind.
    pub(in crate::game::client_side) fn wind_charge_geometry(
        &self,
        p: &Projectile,
        at: glam::Vec3,
        light: u32,
        partial: f32,
        out: &mut Vec<sunrise_client_render::render_geometry::Vertex>,
    ) {
        let Some(model) = self.client.entity_models.wind_charge.as_ref() else {
            return;
        };
        let Some(r) = self
            .client
            .gui
            .atlas
            .sprites
            .get("entity/projectiles/wind_charge")
            .copied()
        else {
            return;
        };
        // `EntityRenderState.ageInTicks`: the tick count plus the partial tick.
        let age = p.age as f32 + partial;
        let mut poses = sunrise_sim::mobs::anim::Poses::new(model);
        poses.yrot("wind_charge", -age * 16.0_f32.to_radians());
        poses.yrot("wind", age * 16.0_f32.to_radians());
        let base = glam::Mat4::from_translation(at)
            * glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0));
        let start = out.len();
        sunrise_client::mobs::render::draw_scrolled(
            model,
            base,
            &poses,
            [r.u0, r.v0, r.u1, r.v1],
            sunrise_client::mobs::render::scroll_offset(
                age,
                sunrise_client::mobs::render::WIND_CHARGE_SCROLL,
            ),
            0xFFFF_FFFF,
            light,
            0,
            out,
        );
        // `RenderTypes.breezeWind` has an `OitPipelineSet`.
        for vertex in &mut out[start..] {
            vertex.layer |= sunrise_client::gfx::OIT_ENTITY;
        }
    }
}
