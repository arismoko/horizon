//! The physical bounds already kept for selector targets. This is not a
//! rendered box or a new source of entity dimensions.
use super::Target;
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_gameplay_common::game::entity::Entity;

fn extents((lo, hi): ([f64; 3], [f64; 3])) -> Aabb {
    Aabb::new(
        Vec3::new(lo[0], lo[1], lo[2]),
        Vec3::new(hi[0], hi[1], hi[2]),
    )
}

/// `EntitySelectorParser.createAabb`, moved by the selector origin.
pub(in crate::game) fn selector_box(origin: [f64; 3], delta: [f64; 3]) -> Aabb {
    let lo = std::array::from_fn(|i| origin[i] + delta[i].min(0.0));
    let hi = std::array::from_fn(|i| origin[i] + (delta[i].max(0.0) + 1.0));
    extents((lo, hi))
}

impl crate::game::ServerGame<'_> {
    /// `Entity.getBoundingBox`: use each simulated kind's existing physical
    /// box. None means no authoritative box is modeled here yet; the selector
    /// retains its existing point fallback for those kinds.
    pub(in crate::game) fn selector_target_bounds(&self, target: Target) -> Option<Aabb> {
        match target {
            Target::General(id) => self
                .common
                .resident_entities
                .general
                .row(id)
                .map(Entity::bounds),
            Target::Player => self.common.player.as_ref().map(|p| p.bounds()),
            Target::Guest(conn) => self.command_guest(conn).map(|p| p.bounds()),
            Target::Hook(id) => self.fishing_hook_by_id(id).map(|(h, _)| h.bounds()),
            Target::DragonPart(id) => self
                .resident_dragon_part(id)
                .map(|(d, i)| d.part_geometry(i).1),
            Target::Mob(id) => self
                .common
                .mobs
                .iter()
                .find(|e| e.id == id)
                .map(Entity::bounds),
            Target::Item(id) => self
                .common
                .dropped
                .iter()
                .find(|e| e.id == id)
                .map(Entity::bounds),
            Target::Orb(id) => self
                .common
                .orbs
                .iter()
                .find(|e| e.id == id)
                .map(Entity::bounds),
            Target::Projectile(id) => self
                .common
                .creatures
                .projectiles
                .iter()
                .find(|e| e.id == id)
                .map(Entity::bounds),
            Target::Stand(id) => self
                .common
                .armor_stands
                .iter()
                .find(|e| e.id == id)
                .map(Entity::bounds),
            Target::Boat(id) => self
                .common
                .boats
                .iter()
                .find(|e| e.id == id)
                .map(Entity::bounds),
            Target::Cart(id) => self
                .common
                .carts
                .iter()
                .find(|e| e.id == id)
                .map(Entity::bounds),
            Target::Tnt(id) => self
                .common
                .tnts
                .iter()
                .find(|e| e.id == id)
                .map(Entity::bounds),
            Target::Falling(id) => self
                .common
                .falling
                .iter()
                .find(|e| e.id == id)
                .map(Entity::bounds),
            Target::Rocket(id) => self
                .common
                .rockets
                .iter()
                .find(|e| e.id == id)
                .map(Entity::bounds),
            Target::Trident(id) => self
                .common
                .tridents
                .iter()
                .find(|e| e.id == id)
                .map(Entity::bounds),
            Target::Frame(id) => self
                .common
                .item_frames
                .iter()
                .find(|e| e.id == id)
                .map(|e| extents(e.bounds())),
            Target::Painting(id) => self
                .common
                .paintings
                .iter()
                .find(|e| e.id == id)
                .map(|e| extents(e.bounds())),
            Target::Cushion(id) => self
                .common
                .cushions
                .iter()
                .find(|e| e.id == id)
                .map(|e| extents(e.bounds())),
            Target::Crystal(id) => self
                .common
                .crystals
                .iter()
                .find(|e| e.id == id)
                .map(|e| e.bounds()),
            Target::Bolt(id) => self.common.bolts.iter().find(|b| b.id == id).map(|b| {
                let p = Vec3::new(b.at[0], b.at[1], b.at[2]);
                Aabb::new(p, p)
            }),
            Target::Other(_)
            | Target::Fangs(_)
            | Target::Dragon(_)
            | Target::Spawner(_)
            | Target::Cloud(_) => None,
            // LeashFenceKnotEntity.recalculateBoundingBox, at the knot's centre.
            Target::Knot(id) => self.natural_knot(id).map(|fence| {
                let knot = sunrise_sim::mobs::leash::knot_state(fence);
                let (p, half) = (knot.position, knot.width / 2.0);
                Aabb::new(
                    Vec3::new(p.x - half, p.y, p.z - half),
                    Vec3::new(p.x + half, p.y + knot.height, p.z + half),
                )
            }),
        }
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::dragon_common::DragonCommonGameExt1 as _;
