//! The client door of the move gate (#2414): the one function that arms a
//! physical entity move, and the only code allowed to reach the collide
//! entries. A family that resolves its own movement goes through a [`MoveGate`]
//! here; `clippy.toml` bans calling the `resolve*` entries anywhere else, so a
//! forgotten family cannot compile around the gate.
//!
//! PR 1a carries no hook: `arm` only records the cause and the movement, every
//! decision is `Run`, and a move with no listener costs one branch. PR 1b adds
//! the `entity_move` hook inside [`arm`] — decide, fire, apply with a revision
//! recheck — leaving these signatures as they are.
use crate::collisions::{EntityCollisions, SceneCollisions};
use horizon_core::geometry::{Aabb, Vec3, border::BorderGeometry};
use horizon_world::block_collision_scan::BlockCollisionWorld;
use horizon_world::collision_context::CollisionContext;
use horizon_world::entity_move::MoverType;
use horizon_world::move_gate::MoveGate;
use horizon_world::native_load_context::LoopContext;
use horizon_world::world_movement::EntityMovementWorld;
use std::borrow::Cow;
use std::io;

/// Arm one physical `Entity.move` invocation (#2414): the entity `entity` is
/// about to move by `delta` for `cause`'s sake. PR 1b fires the `entity_move`
/// hook here — in this one function — and writes what the listeners decided
/// into `gate`; PR 1a records the movement unchanged.
///
/// `kind` names the entity's type (`minecraft:zombie`) only when called: a
/// listener needs it, a move nobody listens to must not pay for it. PR 1b
/// calls it after the subscriber check, so with no listener a move costs one
/// branch and builds no string.
#[inline(never)]
pub fn arm_with<'k>(
    gate: &mut MoveGate,
    _context: &LoopContext,
    _entity: u32,
    _kind: impl FnOnce() -> Cow<'k, str>,
    cause: MoverType,
    delta: Vec3,
) {
    gate.arm(cause, delta);
}

/// [`arm_with`] for a mover whose type name is already a borrowed `&str`.
#[inline]
pub fn arm(
    gate: &mut MoveGate,
    context: &LoopContext,
    entity: u32,
    kind: &str,
    cause: MoverType,
    delta: Vec3,
) {
    arm_with(gate, context, entity, || Cow::Borrowed(kind), cause, delta);
}

/// [`arm_with`] for a fresh gate, in one line at a move site whose type name
/// would cost a string to build (a boat's, a ridden vehicle's).
#[inline]
pub fn armed_with<'k>(
    context: &LoopContext,
    entity: u32,
    kind: impl FnOnce() -> Cow<'k, str>,
    cause: MoverType,
    delta: Vec3,
) -> MoveGate {
    let mut gate = MoveGate::new();
    arm_with(&mut gate, context, entity, kind, cause, delta);
    gate
}

/// [`arm`] for a fresh gate, in one line at a move site.
#[inline]
pub fn armed(
    context: &LoopContext,
    entity: u32,
    kind: &str,
    cause: MoverType,
    delta: Vec3,
) -> MoveGate {
    armed_with(context, entity, || Cow::Borrowed(kind), cause, delta)
}

/// The collide entries: the move gate's own (#2414). The `clippy.toml` ban
/// targets their callers; these wrappers are the one caller-side exception,
/// and everything else resolves through them.
#[allow(clippy::disallowed_methods)]
mod collide {
    use super::*;
    /// `SceneCollisions::resolve` through the gate: the armed move's
    /// collision, stopped by the blocks (and powder snow by the scene's
    /// `subject`).
    pub fn scene(
        gate: &mut MoveGate,
        scene: &mut SceneCollisions<'_, '_>,
        bounds: Aabb,
        movement: Vec3,
        max_step: f32,
        on_ground: bool,
    ) -> crate::Result<Vec3> {
        let _ = gate;
        scene.resolve(bounds, movement, max_step, on_ground)
    }
    /// `SceneCollisions::resolve_with_context` through the gate.
    pub fn scene_with_context(
        gate: &mut MoveGate,
        scene: &mut SceneCollisions<'_, '_>,
        bounds: Aabb,
        movement: Vec3,
        max_step: f32,
        on_ground: bool,
        context: CollisionContext<'_>,
    ) -> crate::Result<Vec3> {
        let _ = gate;
        scene.resolve_with_context(bounds, movement, max_step, on_ground, context)
    }
    /// `SceneCollisions::resolve_in_border_with` through the gate: also
    /// stopped by the world border's walls and the selected `entities` boxes.
    /// The scene is consumed, as the entry it wraps consumes it.
    pub fn scene_in_border_with(
        gate: &mut MoveGate,
        scene: SceneCollisions<'_, '_>,
        bounds: Aabb,
        movement: Vec3,
        max_step: f32,
        on_ground: bool,
        border: Option<(&BorderGeometry, [f64; 2])>,
        entities: &[Aabb],
    ) -> crate::Result<Vec3> {
        let _ = gate;
        scene.resolve_in_border_with(bounds, movement, max_step, on_ground, border, entities)
    }
    /// `EntityCollisions::resolve` through the gate.
    pub fn entities<W>(
        gate: &mut MoveGate,
        collisions: &mut EntityCollisions<'_, W>,
        bounds: Aabb,
        movement: Vec3,
        max_step: f32,
        on_ground: bool,
    ) -> crate::Result<Vec3>
    where
        W: BlockCollisionWorld<Error = io::Error> + EntityMovementWorld<Error = io::Error>,
    {
        let _ = gate;
        collisions.resolve(bounds, movement, max_step, on_ground)
    }
    /// `EntityCollisions::resolve_with_context` through the gate.
    pub fn entities_with_context<W>(
        gate: &mut MoveGate,
        collisions: &mut EntityCollisions<'_, W>,
        bounds: Aabb,
        movement: Vec3,
        max_step: f32,
        on_ground: bool,
        context: CollisionContext<'_>,
    ) -> crate::Result<Vec3>
    where
        W: BlockCollisionWorld<Error = io::Error> + EntityMovementWorld<Error = io::Error>,
    {
        let _ = gate;
        collisions.resolve_with_context(bounds, movement, max_step, on_ground, context)
    }
}
pub use collide::{
    entities as entities_resolve, entities_with_context as entities_resolve_with_context,
    scene as scene_resolve, scene_in_border_with as scene_resolve_in_border_with,
    scene_with_context as scene_resolve_with_context,
};
