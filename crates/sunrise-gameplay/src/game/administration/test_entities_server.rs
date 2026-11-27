use crate::game::ServerGame;
use sunrise_core::bounding_box::BoundingBox;
use sunrise_gameplay::test_entities::{Aabb, intersects, standing};

impl ServerGame<'_> {
    /// `level.getEntitiesOfClass(Entity.class, bounds, mob -> !(mob instanceof
    /// Player)).forEach(Entity::discard)`: every entity but a player whose box
    /// meets `bounds` is removed without dying or dropping anything.
    pub(in crate::game) fn discard_entities_in(&mut self, bounds: Aabb) {
        let at = |v: sunrise_core::geometry::Vec3| [v.x, v.y, v.z];
        for m in &mut self.common.mobs {
            if !m.removed
                && intersects(
                    standing(at(m.position), f64::from(m.width()), f64::from(m.height())),
                    bounds,
                )
            {
                m.removed = true;
            }
        }
        // EntityType.ITEM and EXPERIENCE_ORB.
        self.common
            .dropped
            .retain(|i| !intersects(standing(at(i.position), 0.25, 0.25), bounds));
        self.common
            .orbs
            .retain(|o| !intersects(standing(at(o.position), 0.5, 0.5), bounds));
        self.common
            .creatures
            .projectiles
            .retain(|p| !intersects(standing(at(p.position), 0.5, 0.5), bounds));
        self.common.armor_stands.retain(|a| {
            // ArmorStand.getDimensions: a marker has none, a small one is
            // half scale.
            let (w, h) = match (a.marker, a.small) {
                (true, _) => (0.0, 0.0),
                (_, true) => (0.25, 0.9875),
                _ => (0.5, 1.975),
            };
            !intersects(standing(at(a.position), w, h), bounds)
        });
        self.common
            .item_frames
            .retain(|f| !intersects(f.bounds(), bounds));
        self.common
            .paintings
            .retain(|p| !intersects(p.bounds(), bounds));
        let (boat_w, boat_h) = (
            sunrise_gameplay_common::game::boats_common::WIDTH,
            sunrise_gameplay_common::game::boats_common::HEIGHT,
        );
        let gone: Vec<u32> = self
            .common
            .boats
            .iter()
            .filter(|b| intersects(standing(at(b.position), boat_w, boat_h), bounds))
            .map(|b| b.id)
            .collect();
        for id in gone {
            if let Some(i) = self.common.boats.iter().position(|b| b.id == id) {
                let b = self.common.boats.remove(i);
                self.boat_removed(&b);
            }
        }
        let (cart_w, cart_h) = (super::minecarts::WIDTH, super::minecarts::HEIGHT);
        self.common
            .carts
            .retain(|c| !intersects(standing(at(c.position), cart_w, cart_h), bounds));
        // EntityType.END_CRYSTAL is 2 by 2.
        self.common
            .crystals
            .retain(|c| !intersects(standing(at(c.position), 2.0, 2.0), bounds));
        // Entities Sunrise does not simulate have no box of their own: the
        // position decides.
        self.server.others.0.retain(|o| {
            let p = at(o.position);
            !(0..3).all(|i| p[i] >= bounds.0[i] && p[i] < bounds.1[i])
        });
    }

    /// `ServerLevel.setChunkForced(x, z, true)` for the chunks of `area`
    /// (`TestInstanceBlockEntity.forceLoadChunks`).
    pub(in crate::game) fn force_load_test_chunks(&mut self, area: &BoundingBox) {
        let (x0, x1) = (area.min[0] >> 4, area.max[0] >> 4);
        let (z0, z1) = (area.min[2] >> 4, area.max[2] >> 4);
        self.server
            .streaming
            .set_forced_area((x0, z0), (x1, z1), true, |_, _, _| {});
    }
}
