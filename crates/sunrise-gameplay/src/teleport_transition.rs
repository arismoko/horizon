use sunrise_core::geometry::Vec3;

pub fn is_horse(species: sunrise_sim::mobs::Species) -> bool {
    use sunrise_sim::mobs::Species::*;
    matches!(
        species,
        Horse
            | Donkey
            | Mule
            | SkeletonHorse
            | ZombieHorse
            | Camel
            | CamelHusk
            | Llama
            | TraderLlama
    )
}

/// `Entity.calculatePassengerTransition` with no relative axes: the passenger
/// keeps its offset from the vehicle.
pub fn passenger_destination(old_vehicle: Vec3, new_vehicle: Vec3, passenger: Vec3) -> Vec3 {
    Vec3::new(
        new_vehicle.x + (passenger.x - old_vehicle.x),
        new_vehicle.y + (passenger.y - old_vehicle.y),
        new_vehicle.z + (passenger.z - old_vehicle.z),
    )
}

/// `PositionMoveRotation.calculateAbsolute` then `Entity.setXRot`: the pitch
/// is clamped to -90..90 first (so 360 and +infinity are +90), and NaN is
/// discarded (`None`).
pub fn absolute_pitch(pitch: f32) -> Option<f32> {
    let clamped = pitch.clamp(-90.0, 90.0);
    (!clamped.is_nan()).then(|| (clamped % 360.0).clamp(-90.0, 90.0))
}

/// `Entity.calculatePassengerTransition` for the angles: a rider turns by what
/// the vehicle turned, keeping `passenger.getYRot() - getYRot()`. An absolute
/// angle adds that offset to the vehicle's new one; a relative one (`~`) is a
/// delta the rider adds to its own: the same sum. `after` is the angle before
/// the vehicle's pitch clamp; the rider's pitch is clamped on its own
/// (`Entity.setXRot`), and an axis the command did not set leaves the rider's.
pub fn passenger_rotation(before: [f32; 2], after: [f32; 2], rider: [f32; 2]) -> [f32; 2] {
    let axis = |i: usize| {
        if after[i].is_finite() {
            after[i] + (rider[i] - before[i])
        } else {
            rider[i]
        }
    };
    [axis(0), absolute_pitch(axis(1)).unwrap_or(rider[1])]
}

/// An entity's position and angles.
#[derive(Clone, Copy)]
pub struct Pose {
    pub position: Vec3,
    pub rotation: [f32; 2],
}

/// A `TeleportTransition` with no motion of its own: `position` is absolute or, on a `relative` axis, a shift; a `delta` axis keeps its motion; `rotation` is unclamped.
#[derive(Clone, Copy)]
pub struct Transition {
    pub position: Vec3,
    pub relative: [bool; 3],
    pub delta: [bool; 3],
    pub rotation: Option<[f32; 2]>,
}

impl Transition {
    /// `TeleportCommand.performTeleport`: a relative axis carries `to - from` and keeps its motion.
    pub fn of_command(
        from: Pose,
        to: Vec3,
        relative: [bool; 3],
        rotation: Option<[f32; 2]>,
    ) -> Self {
        let axis = |k: usize, to: f64, from: f64| if relative[k] { to - from } else { to };
        Self {
            position: Vec3::new(
                axis(0, to.x, from.position.x),
                axis(1, to.y, from.position.y),
                axis(2, to.z, from.position.z),
            ),
            relative,
            delta: relative,
            rotation,
        }
    }

    /// A transition to an absolute `to` whose motion is kept (a portal's) or stopped (`/tp`'s).
    pub fn to_level(to: Vec3, motion: TeleportMotion, rotation: Option<[f32; 2]>) -> Self {
        Self {
            position: to,
            relative: [false; 3],
            delta: [matches!(motion, TeleportMotion::Keep); 3],
            rotation,
        }
    }

    /// `Entity.calculatePassengerTransition`: the transition of `rider` on a
    /// vehicle standing at `vehicle` that is given `self`.
    pub fn for_passenger(self, vehicle: Pose, rider: Pose) -> Self {
        let offset = |k: usize, rider: f64, vehicle: f64| {
            if self.relative[k] {
                0.0
            } else {
                rider - vehicle
            }
        };
        Self {
            position: Vec3::new(
                self.position.x + offset(0, rider.position.x, vehicle.position.x),
                self.position.y + offset(1, rider.position.y, vehicle.position.y),
                self.position.z + offset(2, rider.position.z, vehicle.position.z),
            ),
            relative: self.relative,
            delta: self.delta,
            rotation: self.rotation.map(|[yaw, pitch]| {
                [
                    yaw + (rider.rotation[0] - vehicle.rotation[0]),
                    pitch + (rider.rotation[1] - vehicle.rotation[1]),
                ]
            }),
        }
    }

    /// `PositionMoveRotation.calculateAbsolute` of the position.
    pub fn position_for(self, current: Vec3) -> Vec3 {
        let axis = |k: usize, shift: f64, current: f64| {
            if self.relative[k] {
                current + shift
            } else {
                shift
            }
        };
        Vec3::new(
            axis(0, self.position.x, current.x),
            axis(1, self.position.y, current.y),
            axis(2, self.position.z, current.z),
        )
    }

    /// The transition carries no motion: a relative axis keeps its own, the others stop.
    pub fn velocity_for(self, current: Vec3) -> Vec3 {
        Vec3::new(
            if self.delta[0] { current.x } else { 0.0 },
            if self.delta[1] { current.y } else { 0.0 },
            if self.delta[2] { current.z } else { 0.0 },
        )
    }

    /// The angles after the transition, through `Entity.setXRot`'s clamp; NaN leaves the entity's own.
    pub fn rotation_for(self, current: [f32; 2]) -> Option<[f32; 2]> {
        let [yaw, pitch] = self.rotation?;
        Some([
            if yaw.is_finite() { yaw } else { current[0] },
            absolute_pitch(pitch).unwrap_or(current[1]),
        ])
    }
}

/// What a teleport's `TeleportTransition` does with the entity's motion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TeleportMotion {
    /// `Entity.teleportTo(ServerLevel, ..)`, as `/tp` calls it: no delta and no
    /// `Relative.DELTA`, so the entity arrives still.
    Stop,
    /// A portal's `Relative.DELTA` with no delta of its own: the entity keeps
    /// the motion it had (`PositionMoveRotation.of(this)`), unturned.
    Keep,
}

/// `AbstractMinecart.getKnownMovement` with `OldMinecartBehavior`: the cart's
/// motion clamped to 0.4 either way along x and z, what a teleport's
/// `PositionMoveRotation.of(this)` carries over.
pub fn cart_known_movement(velocity: Vec3) -> Vec3 {
    Vec3::new(
        velocity.x.clamp(-0.4, 0.4),
        velocity.y,
        velocity.z.clamp(-0.4, 0.4),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn passenger_keeps_its_offset() {
        let at = passenger_destination(
            Vec3::new(0.0, 64.0, 0.0),
            Vec3::new(100.0, 70.0, -20.0),
            Vec3::new(0.0, 64.5, 0.25),
        );
        assert_eq!((at.x, at.y, at.z), (100.0, 70.5, -19.75));
    }
    #[test]
    fn pitch_clamps_before_the_setter() {
        assert_eq!(absolute_pitch(360.0), Some(90.0));
        assert_eq!(absolute_pitch(-360.0), Some(-90.0));
        assert_eq!(absolute_pitch(f32::INFINITY), Some(90.0));
        assert_eq!(absolute_pitch(f32::NEG_INFINITY), Some(-90.0));
        assert_eq!(absolute_pitch(45.0), Some(45.0));
        assert_eq!(absolute_pitch(f32::NAN), None);
    }
    #[test]
    fn an_absolute_turn_keeps_the_riders_angle_to_the_vehicle() {
        // A rider 30 degrees off a vehicle that turns from 0 to 90 ends at
        // 120: it keeps the 30 degrees it had on it.
        assert_eq!(
            passenger_rotation([0.0, 0.0], [90.0, 0.0], [30.0, 0.0]),
            [120.0, 0.0]
        );
        // The pitch keeps its offset the same way, through Entity.setXRot's
        // clamp: 40 - 10 = 30 degrees off a vehicle placed at 70 would be 100.
        assert_eq!(
            passenger_rotation([10.0, 10.0], [0.0, 70.0], [0.0, 40.0]),
            [-10.0, 90.0]
        );
    }
    #[test]
    fn a_rider_turns_from_the_vehicles_unclamped_pitch() {
        // `~ ~100` on a level vehicle: the vehicle stops at 90, a rider 50
        // below it adds the whole 100 (calculatePassengerTransition) and ends at 50.
        assert_eq!(
            passenger_rotation([0.0, 0.0], [0.0, 100.0], [0.0, -50.0]),
            [0.0, 50.0]
        );
    }
    #[test]
    fn an_axis_the_command_did_not_set_leaves_the_riders_own() {
        assert!(passenger_rotation([0.0, 0.0], [f32::NAN, 0.0], [30.0, 0.0])[0] == 30.0);
        assert!(passenger_rotation([0.0, 0.0], [0.0, f32::NAN], [0.0, 40.0])[1] == 40.0);
    }
}
