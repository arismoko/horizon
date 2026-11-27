//! Common entity-pick types and queries shared by client crosshair picking and server entity picks.
use super::{Hit, pick_ray::PickCandidate};
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_items::items::ItemStack;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PickKind {
    Mob,
    Boat,
    Cart,
    ArmorStand,
    Tnt,
    Falling,
    Projectile,
    Trident,
    Rocket,
    General,
    Painting,
    ItemFrame,
    Cushion,
    Interaction,
    RemoteGeneral,
    RemoteKnot,
    LocalKnot,
    Crystal,
    RemoteDisplay,
    Item,
    Orb,
    LocalPlayer,
    Dragon,
}

#[derive(Clone, Copy)]
pub enum AttackRangePickResult {
    Entity(PickCandidate),
    Block(Hit),
    Miss,
}

#[derive(Clone, Copy)]
pub enum CrosshairDecision {
    Entity(PickCandidate),
    Block(Hit),
    Miss,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrosshairEntityHit {
    pub id: i32,
    pub distance: f64,
    pub point: [f64; 3],
    pub pickable: bool,
    pub suggest_uuid: bool,
}

pub fn aabb_extents((lo, hi): ([f64; 3], [f64; 3])) -> Aabb {
    Aabb::new(
        Vec3::new(lo[0], lo[1], lo[2]),
        Vec3::new(hi[0], hi[1], hi[2]),
    )
}

pub fn knot_bounds(block: [i32; 3]) -> Aabb {
    let knot = sunrise_sim::mobs::leash::knot_state(block);
    let p = knot.position;
    let half = knot.width / 2.0;
    Aabb::new(
        Vec3::new(p.x - half, p.y, p.z - half),
        Vec3::new(p.x + half, p.y + knot.height, p.z + half),
    )
}

/// Whether Pick Block can name an item for the entity (`Entity.getPickResult`).
pub fn has_pick_result(target: Option<crate::game::commands_common::Target>) -> bool {
    use crate::game::commands_common::Target;
    !matches!(
        target,
        Some(
            Target::Projectile(_)
                | Target::DragonPart(_)
                | Target::Other(_)
                | Target::General(_)
                | Target::Tnt(_)
                | Target::Falling(_)
        )
    )
}

pub trait PickEntityCommonGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// `Entity.getPickResult` of the entity `id`.
    fn pick_result_of(&self, id: i32) -> Option<ItemStack> {
        let id = id as u32;
        // EnderDragonPart.getPickResult delegates to its owner's Mob.getPickResult.
        let id = self
            .resident_dragon_part(id)
            .map_or(id, |(dragon, _)| dragon.id);
        let mob_kind = self
            .mobs
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.kind.id.as_str())
            .or_else(|| {
                self.dragon
                    .as_ref()
                    .is_some_and(|d| d.id == id && (d.id != 0 || self.side_has_remote()))
                    .then_some("minecraft:ender_dragon")
            });
        let item = if let Some(kind) = mob_kind {
            // Mob.getPickResult: SpawnEggItem.byId.
            let egg = format!("{kind}_spawn_egg");
            return self.items().get(&egg).map(|_| ItemStack::new(&egg, 1));
        } else if let Some(b) = self.boats.iter().find(|b| b.id == id) {
            b.entity_type()
        } else if let Some(c) = self.carts.iter().find(|c| c.id == id) {
            c.kind.item().to_owned()
        } else if self.armor_stands.iter().any(|a| a.id == id) {
            "minecraft:armor_stand".to_owned()
        } else if let Some(c) = self.cushions.iter().find(|c| c.id == id) {
            c.item()
        } else if let Some(f) = self.item_frames.iter().find(|f| f.id == id) {
            // ItemFrame.getPickResult: what it frames, else the frame itself.
            return Some(
                f.item
                    .clone()
                    .unwrap_or_else(|| ItemStack::new(f.kind(), 1)),
            );
        } else if self.paintings.iter().any(|p| p.id == id) {
            "minecraft:painting".to_owned()
        } else if self.crystals.iter().any(|c| c.id == id) {
            "minecraft:end_crystal".to_owned()
        } else if self.side_pick_knot_fence(id).is_some() {
            // LeashFenceKnotEntity.getPickResult.
            "minecraft:lead".to_owned()
        } else {
            return None;
        };
        Some(ItemStack::new(&item, 1))
    }

    /// `Player.isWithinEntityInteractionRange(entity, 3.0)`.
    fn within_entity_range(&self, id: i32) -> bool {
        let Some(eye) = self.eye_position() else {
            return false;
        };
        let Some(b) = self
            .side_object_target(id)
            .map(|(lo, hi, _)| (lo, hi))
            .or_else(|| {
                let uid = id as u32;
                let bounds = self
                    .mobs
                    .iter()
                    .find(|m| m.id == uid)
                    .map(|m| m.bounds())
                    .or_else(|| self.tracked_special_box(id))
                    .or_else(|| self.side_pick_knot_fence(uid).map(knot_bounds))?;
                let (lo, hi) = (bounds.min_position(), bounds.max_position());
                Some(([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]))
            })
        else {
            return false;
        };
        let d2: f64 = (0..3)
            .map(|a| (eye[a] - eye[a].clamp(b.0[a], b.1[a])).powi(2))
            .sum();
        let range = self.entity_reach + 3.0;
        d2 < range * range
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> PickEntityCommonGameExt1<'__sg> for G {}

use crate::game::side_ops::lan::LanSideGameExt as _;

use crate::game::side_ops::mobs_rest::MobsRestSideGameExt as _;

use crate::game::side_ops::modapi_hooks::ModapiHooksSideGameExt as _;

use crate::game::tables::GameItems as _;

use crate::game::dragon_common::DragonCommonGameExt1 as _;
