use crate::game::ServerGame;
use sunrise_gameplay_common::game::pick_entity_common::{
    CrosshairDecision, CrosshairEntityHit, PickKind,
};

fn hanging_center((lo, hi): ([f64; 3], [f64; 3])) -> [f64; 3] {
    [
        (lo[0] + hi[0]) * 0.5,
        (lo[1] + hi[1]) * 0.5,
        (lo[2] + hi[2]) * 0.5,
    ]
}

fn knot_position(block: [i32; 3]) -> [f64; 3] {
    let p = sunrise_sim::mobs::leash::knot_state(block).position;
    [p.x, p.y, p.z]
}

/// Server-only entity section walk without client stores (`remote`, `client.remote_knots`, `client.interactions`).
pub(in crate::game) fn visit_server_section_members(
    game: &crate::game::ServerGame<'_>,
    visit: &mut impl FnMut(PickKind, usize, u32, [f64; 3]),
) {
    use sunrise_gameplay_common::game::entity::Entity as _;
    for (slot, e) in game.mobs.iter().enumerate() {
        let p = e.position();
        visit(PickKind::Mob, slot, e.id(), [p.x, p.y, p.z]);
    }
    for (slot, e) in game.boats.iter().enumerate() {
        let p = e.position();
        visit(PickKind::Boat, slot, e.id(), [p.x, p.y, p.z]);
    }
    for (slot, e) in game.carts.iter().enumerate() {
        let p = e.position();
        visit(PickKind::Cart, slot, e.id(), [p.x, p.y, p.z]);
    }
    for (slot, e) in game.armor_stands.iter().enumerate() {
        let p = e.position();
        visit(PickKind::ArmorStand, slot, e.id(), [p.x, p.y, p.z]);
    }
    for (slot, e) in game.tnts.iter().enumerate() {
        let p = e.position();
        visit(PickKind::Tnt, slot, e.id(), [p.x, p.y, p.z]);
    }
    for (slot, e) in game.falling.iter().enumerate() {
        let p = e.position();
        visit(PickKind::Falling, slot, e.id(), [p.x, p.y, p.z]);
    }
    for (slot, e) in game.creatures.projectiles.iter().enumerate() {
        let p = e.position();
        visit(PickKind::Projectile, slot, e.id(), [p.x, p.y, p.z]);
    }
    for (slot, e) in game.tridents.iter().enumerate() {
        let p = e.position();
        visit(PickKind::Trident, slot, e.id(), [p.x, p.y, p.z]);
    }
    for (slot, e) in game.rockets.iter().enumerate() {
        let p = e.position();
        visit(PickKind::Rocket, slot, e.id(), [p.x, p.y, p.z]);
    }
    for (slot, e) in game.resident_entities.general.rows.iter().enumerate() {
        let p = e.position();
        visit(PickKind::General, slot, e.id(), [p.x, p.y, p.z]);
    }
    for (slot, p) in game.paintings.iter().enumerate() {
        visit(PickKind::Painting, slot, p.id, hanging_center(p.bounds()));
    }
    for (slot, f) in game.item_frames.iter().enumerate() {
        visit(PickKind::ItemFrame, slot, f.id, hanging_center(f.bounds()));
    }
    for (slot, c) in game.cushions.iter().enumerate() {
        visit(
            PickKind::Cushion,
            slot,
            c.id,
            [c.position.x, c.position.y, c.position.z],
        );
    }
    for (slot, k) in game.resident_entities.knots.iter().enumerate() {
        visit(PickKind::LocalKnot, slot, k.id, knot_position(k.fence));
    }
    for (slot, c) in game.crystals.iter().enumerate() {
        if c.id != 0 {
            visit(
                PickKind::Crystal,
                slot,
                c.id,
                [c.position.x, c.position.y, c.position.z],
            );
        }
    }
    for (slot, e) in game.dropped.iter().enumerate() {
        visit(PickKind::Item, slot, e.id(), e.position().components());
    }
    for (slot, e) in game.orbs.iter().enumerate() {
        visit(PickKind::Orb, slot, e.id(), e.position().components());
    }
    if let Some(d) = game.dragon.as_ref().filter(|d| d.id != 0) {
        visit(PickKind::Dragon, 0, d.id, d.position.components());
    }
}

impl ServerGame<'_> {
    /// Authoritative server entity hit from `server_pick_ray_decision`.
    pub fn server_crosshair_entity_hit(&self) -> Option<CrosshairEntityHit> {
        let CrosshairDecision::Entity(hit) = self.server_pick_ray_decision() else {
            return None;
        };
        let is_synthetic_dragon_part = (sunrise_gameplay_common::game::dragon_common::SPECIAL
            ..sunrise_gameplay_common::game::dragon_common::SPECIAL + 8)
            .contains(&hit.id);
        let id = hit.id as i32;
        let pickable = !is_synthetic_dragon_part
            && sunrise_gameplay_common::game::pick_entity_common::has_pick_result(
                self.level_entity(id),
            );
        let suggest_uuid = !is_synthetic_dragon_part && self.resident_dragon_part(hit.id).is_none();
        Some(CrosshairEntityHit {
            id,
            distance: hit.distance_sq.sqrt(),
            point: hit.point,
            pickable,
            suggest_uuid,
        })
    }

    pub(in crate::game) fn server_entity_is_taken(&self, id: u32) -> bool {
        matches!(
            self.server_resident_entity(id),
            sunrise_sim::join_seq::residency::table::EntityLookup::Taken
        )
    }

    /// The fence of a natural knot or an explicitly summoned/loaded leash knot.
    pub(in crate::game) fn server_pick_knot_fence(&self, id: u32) -> Option<[i32; 3]> {
        self.natural_knot(id).or_else(|| {
            let knot = self
                .server
                .others
                .0
                .iter()
                .find(|o| o.id == id && o.knot_check.is_some())?;
            if self.level_entity(id as i32) != Some(super::commands::Target::Other(id)) {
                return None;
            }
            let block = sunrise_core::position::BlockPos::containing(knot.position);
            Some([block.x, block.y, block.z])
        })
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::dragon_common::DragonCommonGameExt1 as _;
