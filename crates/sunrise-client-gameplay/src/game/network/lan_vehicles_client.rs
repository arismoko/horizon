use crate::game::ClientGame;
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_gameplay_common::game::lan_common::corners;

impl ClientGame<'_> {
    /// The box of a boat, minecart, armor stand, item frame, painting or general row, and
    /// the point an interact packet's location is measured from (the
    /// client's `remote_object_hit` uses the same points).
    pub(in crate::game) fn client_object_target(
        &self,
        id: i32,
    ) -> Option<([f64; 3], [f64; 3], [f64; 3])> {
        let at = |v: Vec3| [v.x, v.y, v.z];
        let centre = |p: [i32; 3]| p.map(|v| f64::from(v) + 0.5);
        if let Some(i) = self.client.interactions.get(&id) {
            let (lo, hi) = corners(i.bounds());
            return Some((lo, hi, at(i.position)));
        }
        let id = u32::try_from(id).ok()?;
        if let Some(b) = self.common.boats.iter().find(|b| b.id == id) {
            let (lo, hi) = corners(b.bounds());
            return Some((lo, hi, at(b.position)));
        }
        if let Some(c) = self.common.carts.iter().find(|c| c.id == id) {
            let (lo, hi) = corners(c.bounds());
            return Some((lo, hi, at(c.position)));
        }
        if let Some(a) = self.common.armor_stands.iter().find(|a| a.id == id) {
            let (lo, hi) = corners(a.bounds());
            return Some((lo, hi, at(a.position)));
        }
        if let Some(c) = self.common.cushions.iter().find(|c| c.id == id) {
            let (lo, hi) = corners(Aabb::new(
                Vec3::new(c.position.x - 0.5, c.position.y, c.position.z - 0.5),
                Vec3::new(c.position.x + 0.5, c.position.y + 0.25, c.position.z + 0.5),
            ));
            return Some((lo, hi, at(c.position)));
        }
        if let Some(f) = self.common.item_frames.iter().find(|f| f.id == id) {
            let (lo, hi) = f.bounds();
            return Some((lo, hi, centre(f.pos)));
        }
        if let Some(p) = self.common.paintings.iter().find(|p| p.id == id) {
            let (lo, hi) = p.bounds();
            return Some((lo, hi, centre(p.pos)));
        }
        if let Some(t) = self.common.tnts.iter().find(|t| t.id == id) {
            let (lo, hi) = corners(t.bounds());
            return Some((lo, hi, at(t.position)));
        }
        if let Some(f) = self.common.falling.iter().find(|f| f.id == id) {
            let (lo, hi) = corners(f.bounds());
            return Some((lo, hi, at(f.position)));
        }
        if !self.common.persist.removals.is_empty()
            && self
                .common
                .persist
                .removals
                .contains(&sunrise_gameplay_common::game::commands_common::Target::General(id))
        {
            return None;
        }
        let row = self.common.resident_entities.general.row(id)?;
        let (lo, hi) = corners(sunrise_gameplay_common::game::entity::Entity::bounds(row));
        Some((lo, hi, at(row.position)))
    }
}
