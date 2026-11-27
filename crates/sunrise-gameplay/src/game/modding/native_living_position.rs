//! `LivingRef` line of sight, random teleport and the box of a pose (#944):
//! the engine's own clip and `randomTeleport` paths, for players and mobs
//! alike.
use super::commands::Target;
use mod_api::{EntityRef, LivingRef};
use sunrise_core::geometry::Vec3;

impl crate::game::ServerGame<'_> {
    /// `LivingEntity.hasLineOfSight(Entity)`: whether nothing that collides
    /// (`ClipContext.Block.COLLIDER`, fluids ignored) lies between the
    /// living entity's eyes and `target`'s eye height, within 128 blocks.
    /// False for a target in another level. None for a stale or unavailable
    /// living entity or target, one outside the level in place, or on a
    /// client.
    pub(in crate::game) fn living_has_line_of_sight(
        &self,
        living: LivingRef,
        target: EntityRef,
    ) -> Option<bool> {
        let viewer = self.native_living_target(living)?;
        let seen = self.native_entity_target(target)?;
        if self.target_level(viewer) != self.common.dim {
            return None;
        }
        if self.target_level(seen) != self.common.dim {
            return Some(false);
        }
        let feet = self.entity_position(living.entity())?;
        let from = self.entity_eye_position(living.entity())?;
        let to = self.entity_eye_position(target)?;
        let w = self.mob_world_base(&[]);
        Some(w.line_of_sight_for(
            Some(feet[1]),
            Vec3::new(from[0], from[1], from[2]),
            Vec3::new(to[0], to[1], to[2]),
        ))
    }

    /// `LivingEntity.randomTeleport(x, y, z, true, avoid)`: the point `at`
    /// is clamped into the world border and drops to the first block in
    /// `minecraft:entities_can_teleport_to` below it, losing a block of height
    /// per step down. The entity lands there, keeping its motion, unless that
    /// ground or a block in its box at the landing is in the `avoid` block
    /// tag, the box collides or holds a liquid, or the species refuses the
    /// spot (an enderman never lands on ground holding water). A landing
    /// stops a mob's navigation and broadcasts entity event 46 (the portal
    /// particles). Nothing else is played: a chorus fruit's sound and an
    /// enderman's own teleport effects belong to their callers.
    ///
    /// True when the entity moved. False when no spot was found, for a
    /// non-finite point, a fake player, a player in another level, an armor
    /// stand or the dragon, a stale handle, or on a client.
    pub fn living_random_teleport(
        &mut self,
        living: LivingRef,
        at: [f64; 3],
        avoid: Option<&str>,
    ) -> bool {
        if !at.iter().all(|v| v.is_finite()) {
            return false;
        }
        let at = Vec3::new(at[0], at[1], at[2]);
        match self.native_living_target(living) {
            Some(target @ (Target::Player | Target::Guest(_))) => {
                if self.target_level(target) != self.common.dim {
                    return false;
                }
                self.native_player_sleep_write(target, |game| {
                    game.player.is_some() && game.random_teleport(at, avoid)
                })
            }
            Some(Target::Mob(id)) => {
                if !self.common.mobs.iter().any(|m| m.id == id && !m.removed) {
                    return false;
                }
                let mut mobs = self.common.mobs.lift();
                #[cfg(any(test, feature = "test-support"))]
                crate::game::server_side::entity_server::taken_pass_tests::after_take(self);
                let w = self.mob_world_base(&[]);
                let mut events = Vec::new();
                let mut teleported = None;
                let moved = mobs.iter_mut().find(|m| m.id == id).is_some_and(|m| {
                    let from = m.position;
                    let moved =
                        sunrise_sim::mobs::living::random_teleport(m, &w, at, avoid, &mut events);
                    teleported = moved.then_some((from, m.position));
                    moved
                });
                self.common
                    .mobs
                    .restore(mobs, sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst);
                let _ = self.handle_mob_events(events);
                // `teleportTo` is one position update, told at once.
                if let Some((from, to)) = teleported {
                    let (from, to) = ([from.x, from.y, from.z], [to.x, to.y, to.z]);
                    self.entity_section_moved(id, Some(from), to);
                }
                moved
            }
            _ => false,
        }
    }
}

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;

use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;
