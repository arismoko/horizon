use sunrise_world::stepped_interpolation::{Path, PosRot, Start, SteppedInterpolation};

/// The packet a living entity's move came in.
#[derive(Clone, Copy, PartialEq)]
pub enum MovePacket {
    /// `handleMoveEntity`: always interpolated.
    Move,
    /// `handleEntityPositionSync` for a ticking entity.
    PositionSync,
    /// `handleEntityPositionSync` for a non-ticking entity (`snapTo` unconditionally).
    PositionSyncSnap,
    /// `handleTeleportEntity` (`setValuesFromPositionPacket`).
    Teleport,
}

/// A move `packet` to `to` for a living entity at `entity`, through its
/// handler (`interpolateTo`): where it is put at once, if anywhere. A move
/// too big to interpolate is a `snapTo`; vanilla leaves the handler running
/// and its next step shifts what is left by the jump
/// (`adjustInterpolationTargetFromDeltas`), which this handler doesn't port,
/// so the steps are dropped instead. That is the same for an entity that
/// wasn't moving, as a teleported one usually isn't.
pub fn stepped_move(
    handler: &mut SteppedInterpolation,
    packet: MovePacket,
    entity: PosRot,
    to: [f64; 3],
    path: Option<Path>,
    rotation: Option<(f32, f32)>,
) -> Option<PosRot> {
    if packet.snaps(handler, entity, to) {
        handler.cancel();
        let (yaw, pitch) = rotation.unwrap_or((entity.yaw, entity.pitch));
        return Some(PosRot {
            position: to,
            yaw,
            pitch,
        });
    }
    match handler.interpolate_to(entity, path, rotation) {
        Start::Snap(at) => Some(at),
        Start::Interpolating | Start::Unchanged => None,
    }
}

impl MovePacket {
    /// `tooBigToInterpolate`: a move to `to` of more than 64 blocks snaps.
    /// A position sync measures it from where the entity is, a teleport from
    /// where it is headed (`PositionMoveRotation.of`'s
    /// `getClientPositionAndRotation`).
    pub fn snaps(self, handler: &SteppedInterpolation, entity: PosRot, to: [f64; 3]) -> bool {
        let from = match self {
            MovePacket::Move => return false,
            MovePacket::PositionSyncSnap => return true,
            MovePacket::PositionSync => entity.position,
            MovePacket::Teleport => handler.heading(entity).position,
        };
        let d = [0, 1, 2].map(|i| to[i] - from[i]);
        d[0] * d[0] + d[1] * d[1] + d[2] * d[2] > 4096.0
    }
}
