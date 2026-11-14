//! Host bodies over the subsystem traits. Each is what the game's inherent method of the same
//! name did, with the game's private state behind `engine_access` instead.
use engine_access::{EntityIdentity, EntityRead, EntityView, LevelRead, LevelWrite, UuidState};
use horizon_world::registry::BlockStateId;
use mod_api::{BlockUpdateFlags, EntityRef, LevelRef, LevelWriteError};

/// The entity's position (`Entity.position`).
pub fn entity_position<E: EntityRead>(e: &E, entity: EntityRef) -> Option<[f64; 3]> {
    e.resolve(entity)?.position()
}

/// The block the entity's position is in (`Entity.blockPosition`).
pub fn entity_block_position<E: EntityRead>(e: &E, entity: EntityRef) -> Option<[i32; 3]> {
    entity_position(e, entity).map(|p| p.map(|v| v.floor() as i32))
}

/// The entity's yaw and pitch in degrees; None for a tracked-only client entity.
pub fn entity_rotation<E: EntityRead>(e: &E, entity: EntityRef) -> Option<[f32; 2]> {
    e.resolve(entity)?.rotation()
}

/// The entity's type (`Entity.getType`).
pub fn entity_kind<E: EntityRead>(e: &E, entity: EntityRef) -> Option<String> {
    e.resolve(entity)?.kind()
}

/// The level the entity is in (`Entity.level`).
pub fn entity_level<E: EntityRead>(e: &E, entity: EntityRef) -> Option<LevelRef> {
    e.resolve(entity)?.level()
}

/// The entity's UUID; a server entity that has none yet gets it now, as a new vanilla entity does.
pub fn entity_uuid<E: EntityIdentity>(e: &mut E, entity: EntityRef) -> Option<u128> {
    match e.uuid_state(entity)? {
        UuidState::Known(uuid) => Some(uuid),
        UuidState::Unassigned => Some(e.assign_uuid(entity)),
    }
}

/// An entity read and a block written in one body: sets `state` in the level the entity is in, at
/// the block its position is in. The view is dropped before the write, which needs the subsystem
/// exclusively. (A body of this shape in the slice; no host method pairs the two today.)
pub fn entity_set_block_at<G>(
    g: &mut G,
    entity: EntityRef,
    state: BlockStateId,
    flags: BlockUpdateFlags,
) -> Result<bool, LevelWriteError>
where
    G: EntityRead + LevelWrite,
{
    let Some((pos, level)) = g.resolve(entity).and_then(|view| {
        let [x, y, z] = view.position()?;
        Some((
            [x.floor() as i32, y.floor() as i32, z.floor() as i32],
            view.level()?,
        ))
    }) else {
        return Err(LevelWriteError::Unavailable);
    };
    g.set_block(level, pos, state, flags)
}

/// `Entity.leashTo` a fence: an entity read, a block read and an entity write. False for a stale
/// handle or a block that is no fence.
pub fn entity_leash_to_knot<G>(g: &mut G, entity: EntityRef, pos: [i32; 3]) -> bool
where
    G: EntityRead + LevelRead + engine_access::EntityLeash,
{
    if g.resolve(entity).is_none() || !g.block_has_tag(pos, "minecraft:fences") {
        return false;
    }
    g.leash_to_knot(entity, pos)
}
