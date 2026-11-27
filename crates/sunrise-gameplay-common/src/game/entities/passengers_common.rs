//! Common passenger attachment and seating helpers shared by server and client.
use crate::game::tnt_common::tnt_seat;
use std::collections::HashMap;
use sunrise_core::geometry::Vec3;
use {
    sunrise_level::camera_math::wrap_degrees, sunrise_sim::mobs::drops::is_animal,
    sunrise_sim::mobs::riders,
};

impl crate::game::CommonGame<'_> {
    /// The vehicle's attachment for this passenger's actual place in `getPassengers`.
    pub fn mob_passenger_attachment(
        vehicle: &sunrise_sim::mobs::Mob,
        rider_is_animal: bool,
        seat: usize,
        count: usize,
    ) -> [f64; 3] {
        if sunrise_sim::mobs::camel::is_camel(vehicle.species()) {
            sunrise_sim::mobs::camel::passenger_attachment_at(vehicle, seat, count, rider_is_animal)
        } else {
            riders::passenger_attachment_at(vehicle, seat)
        }
    }

    /// A vehicle whose `seat_*_mobs` pass places its mob riders itself.
    pub fn seats_own_riders(&self, vehicle: u32) -> bool {
        self.boats.iter().any(|b| b.id == vehicle)
            || self.carts.iter().any(|c| c.id == vehicle)
            || self.cushions.iter().any(|c| c.id == vehicle)
            || self.tnts.iter().any(|t| t.id == vehicle && t.id != 0)
    }

    /// `Entity.positionRider` for mobs riding primed TNT.
    pub fn seat_tnt_mobs(&mut self, riders: &Riders) {
        for i in 0..self.tnts.len() {
            let (id, pos, prev) = (
                self.tnts[i].id,
                self.tnts[i].position,
                self.tnts[i].previous,
            );
            let Some(js) = riders.get(&id).filter(|_| id != 0) else {
                continue;
            };
            let tnt_box = self.tnts[i].bounds();
            for &j in js {
                let m = &mut self.mobs[j];
                // Primed TNT's box is 0.98 wide.
                let offset = riders::vehicle_attachment(m, 0.98);
                m.seated(id, offset, tnt_box);
                sunrise_sim::mobs::living::teleports::set_pos(m, tnt_seat(pos, offset));
                m.previous = tnt_seat(prev, offset);
                m.velocity = Vec3::ZERO;
                m.fall_distance = 0.0;
                m.ground.on_ground = false;
                m.extra.passenger = true;
                m.ai.nav.stop();
            }
        }
    }

    /// `Entity.positionRider` for mobs sitting on a cushion.
    pub fn seat_cushion_mobs(&mut self, riding: &Riders) {
        for c in &self.cushions {
            let Some(js) = riding.get(&c.id) else {
                continue;
            };
            for &j in js {
                let m = &mut self.mobs[j];
                // Cushion's box is 1 wide.
                let offset = riders::vehicle_attachment(m, 1.0);
                let at = Vec3::new(c.position.x, c.position.y + 0.25 - offset, c.position.z);
                sunrise_sim::mobs::living::teleports::snap_to(m, at);
                m.velocity = Vec3::ZERO;
                m.fall_distance = 0.0;
                m.ground.on_ground = false;
                m.extra.passenger = true;
                m.ai.nav.stop();
            }
        }
    }
}

pub trait PassengersCommonGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// Shared per-boat mob seating loop parameterized by the side's passenger order reader.
    fn seat_boat_mobs_with_order(
        &mut self,
        riding: &Riders,
        order_of: impl Fn(&Self, u32) -> Vec<u32>,
    ) {
        for i in 0..self.boats.len() {
            let id = self.boats[i].id;
            let mobs: Vec<usize> = riding
                .get(&id)
                .into_iter()
                .flatten()
                .copied()
                .filter(|&j| !self.mobs[j].removed && self.mobs[j].alive())
                .collect();
            if mobs.is_empty() {
                continue;
            }
            let order = order_of(self, id);
            let count = order.len().max(mobs.len());
            let b = &self.boats[i];
            let full = count == b.max_passengers();
            let delta = b.delta_rotation;
            let boat_box = b.bounds();
            let ridden = sunrise_sim::motion::RiddenBoat {
                under_water: b.under_water(),
                max_y: boat_box.max_position().y,
            };
            let seats: Vec<_> = mobs
                .iter()
                .enumerate()
                .map(|(n, &j)| {
                    let m = &self.mobs[j];
                    let animal = is_animal(m.species());
                    // AbstractBoat's box is 1.375 wide.
                    let offset = riders::vehicle_attachment(m, 1.375);
                    let index = order.iter().position(|&rider| rider == m.id).unwrap_or(n);
                    let turn = match (animal && full, m.id.is_multiple_of(2)) {
                        (false, _) => 0.0,
                        (true, true) => 90.0,
                        (true, false) => 270.0,
                    };
                    let turns = !self.items().entity_type_tags.has_bit(
                        m.kind.flags.entity_tag_slot,
                        sunrise_items::items::EntityTypeTags::CAN_TURN_IN_BOATS,
                    );
                    (
                        j,
                        offset,
                        b.seat(index, count, animal, offset, false),
                        b.seat(index, count, animal, offset, true),
                        turns.then_some((b.yaw + turn, b.previous_yaw + turn)),
                    )
                })
                .collect();
            for (j, offset, at, previous, facing) in seats {
                let m = &mut self.mobs[j];
                m.seated(id, offset, boat_box);
                m.fluids.boat = Some((id, ridden));
                sunrise_sim::mobs::living::teleports::set_pos(m, at);
                m.previous = previous;
                m.velocity = Vec3::ZERO;
                m.fall_distance = 0.0;
                m.ground.on_ground = false;
                m.extra.passenger = true;
                m.ai.nav.stop();
                let Some((body, previous_body)) = facing else {
                    continue;
                };
                m.head_yaw += delta;
                m.body_yaw = body;
                m.previous_body_yaw = previous_body;
                m.yaw = body;
                m.previous_yaw = previous_body;
                let limit = m.kind.max_head_y_rot;
                let d = wrap_degrees(body - m.head_yaw);
                m.head_yaw += d - d.clamp(-limit, limit);
            }
        }
    }

    /// `Entity.getPassengerRidingPosition` of a vehicle.
    fn passenger_riding_position(
        &self,
        vehicle: u32,
        me: Option<u32>,
        own_seat: Option<Vec3>,
        index: &HashMap<u32, usize>,
    ) -> Option<glam::DVec3> {
        self.side_passenger_riding_position(vehicle, me, own_seat, index)
    }

    /// `Entity.positionRider` for every passenger on the active side.
    fn position_riders(&mut self) {
        self.side_position_riders();
    }

    /// `Entity.positionRider` for a general passenger, whose own vehicle attachment is zero.
    fn general_passenger_position(&self, vehicle: u32, rider: u32) -> Option<Vec3> {
        self.side_general_passenger_position(vehicle, rider)
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> PassengersCommonGameExt1<'__sg> for G {}

/// The mobs riding each vehicle, by the vehicle's id: their indices, in the mobs' order.
pub type Riders = HashMap<u32, Vec<usize>>;

/// `Player`'s passenger attachment for synced pose `pose` (the `Pose` id): the top of its box
/// in `Player.POSES` (`EntityDimensions.scale`, in floats), at the player's scale. Poses it has
/// no box for stand (`getOrDefault(pose, STANDING_DIMENSIONS)`).
pub fn player_box_top(pose: u8, scale: f64) -> f64 {
    let height: f32 = match pose {
        // FALL_FLYING, SWIMMING, SPIN_ATTACK: 0.6 x 0.6.
        1 | 3 | 4 => 0.6,
        // SLEEPING and DYING: 0.2 x 0.2.
        2 | 7 => 0.2,
        // CROUCHING: 0.6 x 1.5.
        5 => 1.5,
        _ => 1.8,
    };
    f64::from(height * scale as f32)
}

pub trait PassengersCommonGameExt2<'__sg>: crate::game::SideGame<'__sg> {
    /// The player riding through `riding` and its place: the acting guest (its kept place, if
    /// any, comes from the hub's list) or, outside a guest turn, this game's own player.
    fn riding_player(&self) -> Option<(u32, i64)> {
        match (
            crate::game::Side::lan_acting_entity(self.side_state()),
            crate::game::Side::lan_acting(self.side_state()),
        ) {
            (Some(id), _) if crate::game::Side::lan_acting_uuid(self.side_state()).is_some() => {
                Some((id, i64::MIN))
            }
            (_, None) => self.player.as_ref().map(|p| {
                (
                    p.entity_id as u32,
                    crate::game::Side::riding_order(self.side_state()),
                )
            }),
            _ => None,
        }
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> PassengersCommonGameExt2<'__sg> for G {}

use crate::game::side_ops::mobs_rest::MobsRestSideGameExt as _;

use crate::game::tables::GameItems as _;
