//! Vehicles with nothing of their own to steer or seat with: armor stands, display entities,
//! interactions and the other entities Sunrise keeps as their compound (a marker, which takes no rider at
//! all, is not one of them). `RideCommand.mount` forces the ride
//! (`startRiding(vehicle, true, true)`), so any of them takes a rider; the rider then sits at the vehicle's
//! default passenger attachment and gets off on sneak, as vanilla's `Entity` does for a vehicle that
//! overrides neither.
use super::Keys;
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;
use sunrise_gameplay_common::game::seats_common::{Seat, could_accept_passenger};

/// The box height of an entity Sunrise keeps as its compound, from its type's `EntityDimensions` (the
/// passenger attachment sits on top of it): displays and markers are `sized(0, 0)`, an interaction is as
/// tall as its `height` (1 by default), an area effect cloud half a block.
fn other_height(kind: &str, tag: Option<&Tag>) -> (f64, f32) {
    match kind {
        "minecraft:interaction" => {
            let field = |key: &str| match tag.and_then(|t| t.get(key)) {
                Some(Tag::Float(v)) => Some(f64::from(f32::from_bits(*v))),
                _ => None,
            };
            (
                field("height").unwrap_or(1.0),
                field("width").unwrap_or(1.0) as f32,
            )
        }
        "minecraft:area_effect_cloud" => (0.5, 6.0),
        _ => (0.0, 0.0),
    }
}

/// `EntityType.canSerialize`, which `Entity.startRiding` requires of a vehicle on the server and which a
/// save requires of a root: false for the types built with `noSave()`.
pub(in crate::game::server_side) fn saves_with_chunk(kind: &str) -> bool {
    !matches!(kind, "minecraft:leash_knot" | "minecraft:lightning_bolt")
}

impl crate::game::ServerGame<'_> {
    /// A non-steerable general row, armor stand or kept entity as a vehicle.
    pub(in crate::game) fn seat(&self, id: u32) -> Option<Seat> {
        if let Some(seat) = self.general_seat(id, false) {
            return Some(seat);
        }
        if let Some(a) = self.common.armor_stands.iter().find(|a| a.id == id) {
            // ArmorStand: sized(0.5F, 1.975F), half that when small, nothing as a
            // marker; the passenger point is the float height widened to double.
            let (height, width) = if a.marker {
                (0.0, 0.0)
            } else if a.small {
                (f64::from(0.9875_f32), 0.25)
            } else {
                (f64::from(1.975_f32), 0.5)
            };
            return Some(Seat {
                position: a.position,
                height,
                width,
                attachment: [0.0, height, 0.0],
                yaw: 0.0,
            });
        }
        let o = self.server.others.0.iter().find(|o| o.id == id)?;
        if !could_accept_passenger(&o.kind) {
            return None;
        }
        let (height, width) = other_height(&o.kind, self.common.persist_raw(id));
        Some(Seat {
            position: o.position,
            height,
            width,
            attachment: [0.0, height, 0.0],
            yaw: 0.0,
        })
    }

    pub(in crate::game) fn general_seat(&self, id: u32, _previous: bool) -> Option<Seat> {
        let row = self.common.general_row(id)?;
        Some(Seat::general(row.type_slot, row.position, row.rotation[0]))
    }

    /// Seats the player in place on such a vehicle (`ServerPlayer.startRiding`: at once, as a teleport).
    pub(in crate::game::server_side) fn board_seat(&mut self, id: u32, seat: Seat) {
        let at = seat.player_seat();
        if let Some(player) = self.common.player.as_mut() {
            player.position = at;
            player.velocity = Vec3::ZERO;
            player.fall_distance = 0.0;
            player.teleported = true;
            player.teleported_delta = [true; 3];
        }
        self.common.previous_position = at;
        self.start_riding(id);
    }

    /// `LivingEntity.stopRiding` off such a vehicle: `dismountVehicle` on top of it. A vehicle that is
    /// gone lets the rider go where it is.
    pub(in crate::game::server_side) fn get_off_seat(&mut self) {
        let Some(id) = self.common.riding else {
            return;
        };
        if !self.native_may_leave(id) {
            return;
        }
        self.common.riding = None;
        let Some(seat) = self.seat(id) else {
            return;
        };
        let at = seat.dismount();
        if let Some(p) = self.common.player.as_mut() {
            self.common.previous_position = p.position;
            p.position = at;
            p.velocity = Vec3::ZERO;
            p.fall_distance = 0.0;
        }
    }

    /// `Player.rideTick` on such a vehicle: keep the seat (the vehicle may have been moved), get off on
    /// sneak. The seat, or None once off.
    pub(in crate::game::server_side) fn tick_seat_riding(&mut self, keys: Keys) -> Option<Vec3> {
        let vehicle = self.common.riding?;
        let seat = self.seat(vehicle)?;
        if keys.shift {
            self.get_off_seat();
            return None;
        }
        let at = seat.player_seat();
        if let Some(p) = self.common.player.as_mut() {
            self.common.previous_position = p.position;
            p.position = at;
            p.velocity = Vec3::ZERO;
            p.fall_distance = 0.0;
        }
        Some(at)
    }

    /// `Entity.positionRider` for the mobs riding such a vehicle: its passenger attachment less the
    /// rider's own vehicle attachment. A server sends no position for a passenger, so every client seats
    /// it here.
    pub(in crate::game) fn seat_static_mobs(
        &mut self,
        riding: &sunrise_gameplay_common::game::passengers_common::Riders,
    ) {
        for (&vehicle, js) in riding {
            let Some(seat) = self.seat(vehicle) else {
                continue;
            };
            for &j in js {
                seat.position_mob(&mut self.common.mobs[j]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_player_sits_on_the_passenger_point_and_gets_off_on_top() {
        // A display: sized(0, 0), its passenger point at its position.
        let display = Seat {
            position: Vec3::new(1.5, 64.0, -2.5),
            height: 0.0,
            width: 0.0,
            attachment: [0.0; 3],
            yaw: 0.0,
        };
        assert_eq!(display.player_seat(), Vec3::new(1.5, 63.4, -2.5));
        assert_eq!(display.dismount(), Vec3::new(1.5, 64.0, -2.5));
        // An armor stand: its passenger point on top of its 1.975 box.
        let stand = Seat {
            position: Vec3::new(0.5, 64.0, 0.5),
            height: 1.975,
            width: 0.5,
            attachment: [0.0, 1.975, 0.0],
            yaw: 0.0,
        };
        assert!((stand.player_seat().y - 65.375).abs() < 1e-9);
        assert!((stand.dismount().y - 65.975).abs() < 1e-9);
    }

    /// `Entity.positionRider`: the vehicle's position plus its passenger attachment, less the
    /// rider's own vehicle attachment, with the attachment turned by the vehicle's yaw. Expected
    /// values are written out by hand, not derived from the code.
    #[test]
    fn a_rider_sits_on_the_passenger_attachment_less_its_own() {
        let seat = Seat {
            position: Vec3::new(10.0, 64.0, -3.0),
            height: 2.0,
            width: 0.5,
            attachment: [1.0, 2.0, 0.5],
            yaw: 0.0,
        };
        // Facing yaw 0 the attachment is not turned: x 10 + 1, y 64 + 2 - 0.5, z -3 + 0.5.
        assert_eq!(seat.passenger_position(0.5), Vec3::new(11.0, 65.5, -2.5));
        // A rider with no vehicle attachment of its own sits on the point itself.
        assert_eq!(seat.passenger_position(0.0), Vec3::new(11.0, 66.0, -2.5));
        // The default attachment is the top centre of the box, whatever the position.
        let top = Seat {
            attachment: [0.0, 2.0, 0.0],
            ..seat
        };
        assert_eq!(top.passenger_position(0.0), Vec3::new(10.0, 66.0, -3.0));
    }

    #[test]
    fn kept_entities_are_as_tall_as_their_type() {
        assert_eq!(other_height("minecraft:item_display", None), (0.0, 0.0));
        assert_eq!(other_height("minecraft:marker", None), (0.0, 0.0));
        assert_eq!(other_height("minecraft:interaction", None), (1.0, 1.0));
        assert_eq!(
            other_height("minecraft:area_effect_cloud", None),
            (0.5, 6.0)
        );
    }
}
