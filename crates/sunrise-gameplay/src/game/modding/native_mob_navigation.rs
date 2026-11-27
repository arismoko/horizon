//! Navigation helpers for native mod mob handles.
use super::commands::Target;
use mod_api::{MobRef, MoveToPathError, PathType};

/// The numbers a mod hands a mob's navigation (`moveTo(x, y, z, speed)`): a
/// finite speed, and finite coordinates small enough for the pathfinder's
/// block arithmetic. The mob handles' boundary, shared with a goal tick's own
/// `move_to` (#1077) — one check, not a copy per caller.
pub(in crate::game) fn navigation_move_accepts(at: [f64; 3], speed: f64) -> bool {
    speed.is_finite()
        && at
            .iter()
            .all(|v| v.is_finite() && v.abs() <= 2_000_000_000.0)
}

/// The coordinates a mod hands a mob's look control (`setLookAt(x, y, z)`):
/// finite ones — the same boundary the navigation takes, for the control
/// whose input it shares.
pub(in crate::game) fn navigation_look_accepts(at: [f64; 3]) -> bool {
    at.iter().all(|v| v.is_finite())
}

fn navigation_flags_supported(mob: &sunrise_sim::mobs::Mob) -> bool {
    use sunrise_sim::mobs::nav::NavKind;
    matches!(
        mob.ai.nav.kind,
        NavKind::Ground | NavKind::WallClimber | NavKind::Flying
    )
}

/// The nodes a mod hands `PathNavigation.moveTo(Path, speed)`: finite
/// coordinates and a finite speed. Nothing is refused for being unloaded or
/// outside the world, and the node count is unbounded (vanilla's `moveTo`
/// carries no cap); a node outside the block-coordinate range is floored to
/// the saturating integer and reads as void air, as vanilla's `BlockPos` cast
/// does.
pub fn supplied_path_accepts(nodes: &[mod_api::PathNode], speed: f64) -> bool {
    speed.is_finite()
        && nodes
            .iter()
            .all(|node| node.x.is_finite() && node.y.is_finite() && node.z.is_finite())
}

/// The refusal a `false` from `nav_move_to_supplied` stands for once the
/// numbers and the mob are known to be fine: `PathNavigation.moveTo(Path)`
/// refuses a path with no node left, an empty one or one the trim emptied.
/// One conversion, shared by the handle and by a goal tick's view (#4410).
pub(in crate::game) fn supplied_path_refusal(followed: bool) -> Result<(), MoveToPathError> {
    if followed {
        Ok(())
    } else {
        Err(MoveToPathError::Empty)
    }
}

impl crate::game::ServerGame<'_> {
    /// `PathNavigation.moveTo(x, y, z, speed)`: true if a path was started.
    /// Refuses a stale or unavailable mob, a client, non-finite arguments,
    /// and coordinates too large for safe block arithmetic. As in vanilla,
    /// the mob's own goals may replace the path next tick. False for the
    /// dragon, which flies by its phase and keeps no navigation.
    pub(in crate::game) fn mob_move_to(&mut self, mob: MobRef, at: [f64; 3], speed: f64) -> bool {
        if !navigation_move_accepts(at, speed) {
            return false;
        }
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        let Some(index) = self.common.mobs.iter().position(|m| m.id == id) else {
            return false;
        };
        // The pathfinder reads terrain only. Take the mob list so its selected
        // mob can be mutably borrowed alongside the shared MobWorld, just as
        // the mob tick does. No mod hook fires during pathfinding.
        let mut mobs = self.common.mobs.lift();
        #[cfg(any(test, feature = "test-support"))]
        crate::game::server_side::entity_server::taken_pass_tests::after_take(self);
        let moved = mobs[index].nav_move_to(&self.mob_world_base(&[]), at, speed);
        self.common
            .mobs
            .restore(mobs, sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst);
        moved
    }

    /// `PathNavigation.moveTo(Path, speed)`: follow a path the mod computed.
    /// Each node is floored to its block, classified by the engine on submit
    /// (the classification never filters the path), trimmed as every `moveTo`
    /// trims, and followed as given. As in vanilla, taking the path leaves
    /// the navigation's recompute target as it was (a block change on it
    /// re-searches to the last searched target, and with none the path is
    /// kept); the stuck check drops it. Only non-finite coordinates and
    /// speeds are refused; the node
    /// count is unbounded, unloaded or out-of-world nodes are neither refused
    /// nor clamped, and an extreme finite node is classified and followed with
    /// saturating block arithmetic. `Unavailable` for a stale or unavailable
    /// mob, a client, and the dragon; `Empty` when no node is left to follow.
    pub(in crate::game) fn mob_move_to_path(
        &mut self,
        mob: MobRef,
        nodes: Vec<mod_api::PathNode>,
        speed: f64,
    ) -> Result<(), MoveToPathError> {
        if !supplied_path_accepts(&nodes, speed) {
            return Err(MoveToPathError::NotFinite);
        }
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return Err(MoveToPathError::Unavailable);
        };
        let Some(index) = self.common.mobs.iter().position(|m| m.id == id) else {
            return Err(MoveToPathError::Unavailable);
        };
        let nodes: Vec<[f64; 3]> = nodes.iter().map(|n| [n.x, n.y, n.z]).collect();
        // The pathfinder reads terrain only. Take the mob list so its selected
        // mob can be mutably borrowed alongside the shared MobWorld, as the
        // mob tick does. No mod hook fires during classification.
        let mut mobs = self.common.mobs.lift();
        #[cfg(any(test, feature = "test-support"))]
        crate::game::server_side::entity_server::taken_pass_tests::after_take(self);
        let followed = mobs[index].nav_move_to_supplied(&self.mob_world_base(&[]), &nodes, speed);
        self.common
            .mobs
            .restore(mobs, sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst);
        supplied_path_refusal(followed)
    }

    /// `PathNavigation.stop`. False for a stale or unavailable mob, or on a
    /// client, and for the dragon (no navigation).
    pub(in crate::game) fn mob_stop(&mut self, mob: MobRef) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        let Some(mob) = self.common.mobs.iter_mut().find(|m| m.id == id) else {
            return false;
        };
        mob.nav_stop();
        true
    }

    /// `PathNavigation.isDone`; None for a stale or unavailable mob, or on a
    /// client, and for the dragon (no navigation).
    pub(in crate::game) fn mob_is_done(&self, mob: MobRef) -> Option<bool> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        Some(
            self.common
                .mobs
                .iter()
                .find(|m| m.id == id)?
                .ai
                .nav
                .is_done(),
        )
    }

    /// `PathNavigation.getSpeedModifier`: the multiplier the current path
    /// is followed at (1.0 is the mob's normal speed; 0 until a path sets
    /// one). This is not the mob's movement-speed attribute. None for a
    /// stale or unavailable mob, or on a client, and for the dragon.
    pub(in crate::game) fn mob_navigation_speed(&self, mob: MobRef) -> Option<f64> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        Some(
            self.common
                .mobs
                .iter()
                .find(|m| m.id == id)?
                .ai
                .nav
                .speed_modifier,
        )
    }

    /// The live supported navigation's `can_open_doors` flag; unavailable mobs refuse.
    pub(in crate::game) fn mob_can_open_doors(&self, mob: MobRef) -> Option<bool> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        let mob = self.common.mobs.iter().find(|m| m.id == id && !m.removed)?;
        navigation_flags_supported(mob).then_some(mob.ai.nav.can_open_doors)
    }

    /// `PathNavigation.setCanOpenDoors`; leaves the current path intact.
    pub(in crate::game) fn mob_set_can_open_doors(&mut self, mob: MobRef, value: bool) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        let Some(mob) = self
            .common
            .mobs
            .iter_mut()
            .find(|m| m.id == id && !m.removed)
        else {
            return false;
        };
        if !navigation_flags_supported(mob) {
            return false;
        }
        mob.ai.nav.can_open_doors = value;
        true
    }

    /// The live supported navigation's `can_pass_doors` flag; unavailable mobs refuse.
    pub(in crate::game) fn mob_can_pass_doors(&self, mob: MobRef) -> Option<bool> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        let mob = self.common.mobs.iter().find(|m| m.id == id && !m.removed)?;
        navigation_flags_supported(mob).then_some(mob.ai.nav.can_pass_doors)
    }

    /// Sets the pass-door navigation wish; leaves the current path intact.
    pub(in crate::game) fn mob_set_can_pass_doors(&mut self, mob: MobRef, value: bool) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        let Some(mob) = self
            .common
            .mobs
            .iter_mut()
            .find(|m| m.id == id && !m.removed)
        else {
            return false;
        };
        if !navigation_flags_supported(mob) {
            return false;
        }
        mob.ai.nav.can_pass_doors = value;
        true
    }

    /// The live supported navigation's `can_float` flag; unavailable mobs refuse.
    pub(in crate::game) fn mob_can_float(&self, mob: MobRef) -> Option<bool> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        let mob = self.common.mobs.iter().find(|m| m.id == id && !m.removed)?;
        navigation_flags_supported(mob).then_some(mob.ai.nav.can_float)
    }

    /// `PathNavigation.setCanFloat`; leaves the current path intact.
    pub(in crate::game) fn mob_set_can_float(&mut self, mob: MobRef, value: bool) -> bool {
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        let Some(mob) = self
            .common
            .mobs
            .iter_mut()
            .find(|m| m.id == id && !m.removed)
        else {
            return false;
        };
        if !navigation_flags_supported(mob) {
            return false;
        }
        mob.ai.nav.can_float = value;
        true
    }

    /// `Mob.getPathfindingMalus`: what a node of `kind` costs this mob's
    /// pathfinder — its own override, else the kind's default (a negative
    /// malus closes the kind). None for a stale or unavailable mob, the
    /// dragon, or on a client.
    pub(in crate::game) fn mob_pathfinding_malus(
        &self,
        mob: MobRef,
        kind: PathType,
    ) -> Option<f32> {
        let Target::Mob(id) = self.native_entity_target(mob.entity())? else {
            return None;
        };
        Some(
            self.common
                .mobs
                .iter()
                .find(|m| m.id == id && !m.removed)?
                .malus(kind),
        )
    }

    /// `Mob.setPathfindingMalus`: what the mob's next paths cost through
    /// `kind` (negative closes it, as the species' own overrides do); not
    /// saved, as in vanilla, so a reload or conversion keeps the species'
    /// own. False for a malus that is not finite, a stale or unavailable mob,
    /// the dragon, or on a client.
    pub(in crate::game) fn mob_set_pathfinding_malus(
        &mut self,
        mob: MobRef,
        kind: PathType,
        malus: f32,
    ) -> bool {
        if !malus.is_finite() {
            return false;
        }
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        self.common
            .mobs
            .iter_mut()
            .find(|m| m.id == id && !m.removed)
            .map(|m| m.set_malus(kind, malus))
            .is_some()
    }

    /// `PathNavigation.setSpeedModifier`: sets the multiplier the current
    /// path is followed at, as `move_to`'s speed argument does. The mob's
    /// own AI — a goal's or a brain species' next `moveTo` — sets its own
    /// again, as in vanilla. False for a non-finite speed, a stale or
    /// unavailable mob, or on a client, and for the dragon.
    pub(in crate::game) fn mob_set_navigation_speed(&mut self, mob: MobRef, speed: f64) -> bool {
        if !speed.is_finite() {
            return false;
        }
        let Some(Target::Mob(id)) = self.native_entity_target(mob.entity()) else {
            return false;
        };
        self.common
            .mobs
            .iter_mut()
            .find(|m| m.id == id)
            .map(|m| m.ai.nav.speed_modifier = speed)
            .is_some()
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;
