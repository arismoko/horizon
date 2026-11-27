//! The cart's `Entity.move` — `AbstractMinecart.move`'s own tick move through
//! the move gate (#2414) — in its own module so `minecarts.rs` keeps its size.
use crate::game::ServerGame;
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_gameplay_common::game::minecarts::{CartKind, Minecart};
use sunrise_sim::motion::BodyContact;

pub(in crate::game) struct CartLanding {
    pub(in crate::game) id: u32,
    pub(in crate::game) kind: CartKind,
    pub(in crate::game) at: Vec3,
    pub(in crate::game) bounds: Aabb,
    pub(in crate::game) distance: f64,
}
impl ServerGame<'_> {
    /// `Entity.move` resets a ray-clipped fall, then `checkFallDamage` uses the
    /// current position, supporting block and the body's last water contact.
    pub(in crate::game) fn cart_move_fall(
        &self,
        c: &Minecart,
        from: Vec3,
        moved: Vec3,
    ) -> (f64, Option<CartLanding>) {
        let mut distance = self.common.vehicle_fall_reset(c.fall_distance, from, moved);
        if moved.y < 0.0 && !c.in_water {
            distance -= f64::from(moved.y as f32);
        }
        if !c.ground.on_ground {
            return (distance, None);
        }
        (
            0.0,
            (distance > 0.0).then(|| CartLanding {
                id: c.id,
                kind: c.kind,
                at: c.position,
                bounds: c.bounds(),
                distance,
            }),
        )
    }

    /// Run `Block.fallOn`, including `MinecartTNT.causeFallDamage`, before the
    /// landing game event. A removed cart cannot resolve block restitution.
    pub(in crate::game) fn finish_cart_landing(
        &mut self,
        landing: Option<CartLanding>,
        effect: BodyContact,
        explode: impl FnOnce(&mut Self, f64) -> bool,
    ) -> bool {
        let Some(CartLanding {
            id,
            kind,
            at,
            bounds,
            distance,
        }) = landing
        else {
            return true;
        };
        self.nonliving_fall_on(id, at, effect);
        let removed = if kind == CartKind::Tnt && distance >= 3.0 {
            let speed = distance / 10.0;
            explode(self, speed * speed)
        } else {
            false
        };
        self.post_landing_event(at, bounds, false);
        !removed
    }
}
