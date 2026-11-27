//! The client door of the move gate (#2414): the one function that arms a
//! physical entity move, and the only code allowed to reach the collide
//! entries. A family that resolves its own movement goes through a [`MoveGate`]
//! here; `clippy.toml` bans calling the `resolve*` entries anywhere else, so a
//! forgotten family cannot compile around the gate.
//!
//! [`arm`] is also where the `entity_move` hook fires (#2414, design #2308):
//! decide, fire, apply — in that one function, on the authoritative side, only
//! when a listener subscribes (one branch otherwise: no snapshot, no
//! allocation, no mod scan).
use crate::collisions::{EntityCollisions, SceneCollisions};
use mod_api::MoveCause;
use std::borrow::Cow;
use std::io;
use sunrise_core::geometry::{Aabb, Vec3, border::BorderGeometry};
use sunrise_world::block_collision_scan::BlockCollisionWorld;
use sunrise_world::collision_context::CollisionContext;
use sunrise_world::entity_move::MoverType;
use sunrise_world::move_gate::MoveGate;
use sunrise_world::native_load_context::LoopContext;
use sunrise_world::world_movement::EntityMovementWorld;

/// The `entity_move` hook's target (#2414): the generated dispatch the
/// catalog's generator requires of a target, with the loop's own reader as
/// the context. The move gate uses the same generated dispatch.
pub fn native_entity_move(
    context: &LoopContext,
    entity: u32,
    kind: &str,
    cause: MoveCause,
    delta: &mut [f64; 3],
    canceled: &mut bool,
) {
    let reader = sunrise_level::api_context::LoopReader(context);
    mod_api::dispatch_entity_move(&reader.context(), entity, kind, cause, delta, canceled);
}

/// The cause the mod API names for a `MoverType` (`entity_move` of #2414):
/// vanilla's five mover names, one for one.
fn cause_of(cause: MoverType) -> MoveCause {
    match cause {
        MoverType::SelfMovement => MoveCause::SelfMovement,
        MoverType::Player => MoveCause::Player,
        MoverType::Piston => MoveCause::Piston,
        MoverType::ShulkerBox => MoveCause::ShulkerBox,
        MoverType::Shulker => MoveCause::Shulker,
        MoverType::Carried => MoveCause::Carried,
    }
}

/// The `MoverType` a mod asks for by its cause (`MoveCause`): `cause_of`'s inverse. None for a
/// cause this build does not know (`MoveCause` may grow).
pub fn mover_of(cause: MoveCause) -> Option<MoverType> {
    match cause {
        MoveCause::SelfMovement => Some(MoverType::SelfMovement),
        MoveCause::Player => Some(MoverType::Player),
        MoveCause::Piston => Some(MoverType::Piston),
        MoveCause::ShulkerBox => Some(MoverType::ShulkerBox),
        MoveCause::Shulker => Some(MoverType::Shulker),
        _ => None,
    }
}

/// Arm one physical `Entity.move` invocation (#2414): the entity `entity` is
/// about to move by `delta` for `cause`'s sake. The `entity_move` hook fires
/// here — in this one function — on the authoritative side, and only when a
/// listener subscribes; with none, or on a client's own prediction, the
/// movement is recorded unchanged and the tick cost is the subscriber branch.
/// A listener that cancels skips the invocation; a non-finite replacement is
/// refused whole — the movement keeps the preceding accepted delta. If a
/// listener unwinds, dispatch
/// drops only its edits and disables that callback for this hook. Earlier
/// results stand and later eligible listeners still get the move.
///
/// `kind` names the entity's type (`minecraft:zombie`) only when called: a
/// listener needs it, a move nobody listens to must not pay for it. It runs
/// after the subscriber check, so with no listener a move costs one branch
/// and builds no string.
#[inline(never)]
pub fn arm_with<'k>(
    gate: &mut MoveGate,
    context: &LoopContext,
    entity: u32,
    kind: impl FnOnce() -> Cow<'k, str>,
    cause: MoverType,
    delta: Vec3,
) {
    arm_with_controlled(gate, context, entity, kind, cause, delta, false);
}

/// [`arm_with`] with an explicit `client_controlled` flag for a body a client
/// controls (a player, or a vehicle a player is driving): `side = "server"`
/// listeners are skipped for those bodies.
#[inline(never)]
pub fn arm_with_controlled<'k>(
    gate: &mut MoveGate,
    context: &LoopContext,
    entity: u32,
    kind: impl FnOnce() -> Cow<'k, str>,
    cause: MoverType,
    delta: Vec3,
    client_controlled: bool,
) {
    // The requested delta is the floor every listener result falls back to:
    // armed first, the decision replaced only by a result that passes
    // validation — the request stays what `overridden` measures against.
    gate.arm(entity, cause, delta);
    if !mod_api::__private::listened(mod_api::HookId::EntityMove) {
        return;
    }
    let kind = kind();
    let client_controlled = client_controlled || &*kind == "minecraft:player";
    let mut replaced = [delta.x, delta.y, delta.z];
    let mut canceled = false;
    let reader = sunrise_level::api_context::LoopReader(context);
    mod_api::dispatch_entity_move_for(
        &reader.context(),
        entity,
        &kind,
        cause_of(cause),
        &mut replaced,
        &mut canceled,
        client_controlled,
    );
    if canceled {
        gate.skip();
    } else {
        gate.decide(Vec3::new(replaced[0], replaced[1], replaced[2]));
    }
}

/// The pre-existing finite, per-axis range on direct `move_by` requests from mods.
/// It is not a limit on `Entity.move` or `entity_move` listener proposals.
pub fn valid_mod_move_delta(delta: [f64; 3]) -> bool {
    delta.iter().all(|v| v.is_finite() && v.abs() <= 64.0)
}

#[test]
fn mod_move_deltas_use_an_inclusive_finite_bound_on_every_axis() {
    assert!(valid_mod_move_delta([-64.0, 0.0, 64.0]));
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -64.5, 64.5] {
        for axis in 0..3 {
            let mut delta = [0.0; 3];
            delta[axis] = invalid;
            assert!(!valid_mod_move_delta(delta));
        }
    }
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

/// [`arm_with_controlled`] for a mover whose type name is already a borrowed `&str`.
#[inline]
pub fn arm_controlled(
    gate: &mut MoveGate,
    context: &LoopContext,
    entity: u32,
    kind: &str,
    cause: MoverType,
    delta: Vec3,
    client_controlled: bool,
) {
    arm_with_controlled(
        gate,
        context,
        entity,
        || Cow::Borrowed(kind),
        cause,
        delta,
        client_controlled,
    );
}

/// [`arm_with_controlled`] for a fresh gate, in one line at a move site whose
/// type name would cost a string to build.
#[inline]
pub fn armed_with_controlled<'k>(
    context: &LoopContext,
    entity: u32,
    kind: impl FnOnce() -> Cow<'k, str>,
    cause: MoverType,
    delta: Vec3,
    client_controlled: bool,
) -> MoveGate {
    let mut gate = MoveGate::new();
    arm_with_controlled(
        &mut gate,
        context,
        entity,
        kind,
        cause,
        delta,
        client_controlled,
    );
    gate
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
    use sunrise_level::moving_colliders;
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
        moving_colliders::as_mover(gate.mover(), || {
            scene.resolve(bounds, movement, max_step, on_ground)
        })
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
        moving_colliders::as_mover(gate.mover(), || {
            scene.resolve_with_context(bounds, movement, max_step, on_ground, context)
        })
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
        moving_colliders::as_mover(gate.mover(), || {
            scene.resolve_in_border_with(bounds, movement, max_step, on_ground, border, entities)
        })
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
        moving_colliders::as_mover(gate.mover(), || {
            collisions.resolve(bounds, movement, max_step, on_ground)
        })
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
        moving_colliders::as_mover(gate.mover(), || {
            collisions.resolve_with_context(bounds, movement, max_step, on_ground, context)
        })
    }
}
pub use collide::{
    entities as entities_resolve, entities_with_context as entities_resolve_with_context,
    scene as scene_resolve, scene_in_border_with as scene_resolve_in_border_with,
    scene_with_context as scene_resolve_with_context,
};
