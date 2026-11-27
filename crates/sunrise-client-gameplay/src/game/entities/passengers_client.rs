use crate::game::ClientGame;
use std::collections::HashMap;
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_gameplay_common::game::passengers_common::{Riders, player_box_top};
use sunrise_sim::mobs::riders;

impl ClientGame<'_> {
    /// Client-side seat for a remote general entity row (`remote.general.rows`).
    pub(in crate::game) fn client_general_seat(
        &self,
        id: u32,
        previous: bool,
    ) -> Option<sunrise_gameplay_common::game::seats_common::Seat> {
        let row = self
            .client
            .remote
            .as_ref()?
            .general
            .rows
            .get(&(id as i32))?;
        let ([x, y, z], [yaw, _]) = if previous {
            (row.previous, row.previous_rotation)
        } else {
            (row.position, row.rotation)
        };
        Some(sunrise_gameplay_common::game::seats_common::Seat::general(
            row.type_slot,
            Vec3::new(x, y, z),
            yaw,
        ))
    }

    /// Client-side non-steerable seat lookup (remote general row, armor stand, or kept entity).
    pub(in crate::game::client_side) fn client_seat(
        &self,
        id: u32,
    ) -> Option<sunrise_gameplay_common::game::seats_common::Seat> {
        if let Some(seat) = self.client_general_seat(id, false) {
            return Some(seat);
        }
        if let Some(a) = self.common.armor_stands.iter().find(|a| a.id == id) {
            let (height, width) = if a.marker {
                (0.0, 0.0)
            } else if a.small {
                (f64::from(0.9875_f32), 0.25)
            } else {
                (f64::from(1.975_f32), 0.5)
            };
            return Some(sunrise_gameplay_common::game::seats_common::Seat {
                position: a.position,
                height,
                width,
                attachment: [0.0, height, 0.0],
                yaw: 0.0,
            });
        }
        None
    }

    /// Client-side passenger order from applied `SetPassengers` seats.
    pub(in crate::game::client_side) fn client_passenger_order(&self, vehicle: u32) -> Vec<u32> {
        let Some(remote) = self.client.remote.as_ref() else {
            return Vec::new();
        };
        let mut seats: Vec<_> = remote.passenger_seats(vehicle).collect();
        seats.sort_unstable_by_key(|&(_, index)| index);
        seats.into_iter().map(|(id, _)| id).collect()
    }

    /// Client-side seat index and passenger count on `vehicle`.
    pub(in crate::game::client_side) fn client_passenger_place(
        &self,
        vehicle: u32,
        rider: u32,
    ) -> (usize, usize) {
        let order = self.client_passenger_order(vehicle);
        let index = order.iter().position(|&id| id == rider).unwrap_or(0);
        (index, order.len().max(1))
    }

    pub(in crate::game::client_side) fn client_seat_boat_mobs(&mut self, riding: &Riders) {
        self.seat_boat_mobs_with_order(riding, |game, id| game.client_passenger_order(id));
    }

    pub(in crate::game::client_side) fn client_seat_static_mobs(&mut self, riding: &Riders) {
        for (&vehicle, js) in riding {
            let Some(seat) = self.client_seat(vehicle) else {
                continue;
            };
            for &j in js {
                seat.position_mob(&mut self.common.mobs[j]);
            }
        }
    }

    /// Client-side `Entity.positionRider` for a general passenger, whose own vehicle attachment is zero.
    pub(in crate::game) fn client_general_passenger_position(
        &self,
        vehicle: u32,
        rider: u32,
    ) -> Option<Vec3> {
        if let Some(seat) = self.client_seat(vehicle) {
            return Some(seat.passenger_position(0.0));
        }
        if let Some(b) = self.common.boats.iter().find(|b| b.id == vehicle) {
            let (index, count) = self.client_passenger_place(vehicle, rider);
            return Some(b.seat(index, count, false, 0.0, false));
        }
        if let Some(c) = self.common.carts.iter().find(|c| c.id == vehicle) {
            return Some(super::minecarts::cart_general_seat(c.position));
        }
        if let Some(t) = self
            .common
            .tnts
            .iter()
            .find(|t| t.id == vehicle && t.id != 0)
        {
            return Some(sunrise_gameplay_common::game::tnt_common::tnt_general_seat(
                t.position,
            ));
        }
        if let Some(v) = self
            .common
            .mobs
            .iter()
            .find(|m| m.id == vehicle && !m.removed)
        {
            let (index, count) = self.client_passenger_place(vehicle, rider);
            let attachment = sunrise_gameplay_common::game::CommonGame::mob_passenger_attachment(
                v, false, index, count,
            );
            return Some(riders::seat(attachment, 0.0, v.position, v.yaw));
        }
        let at = self.client_passenger_riding_position(vehicle, None, None, &HashMap::new())?;
        Some(Vec3::new(at.x, at.y, at.z))
    }

    /// Client-side `Entity.getPassengerRidingPosition`.
    pub(in crate::game) fn client_passenger_riding_position(
        &self,
        vehicle: u32,
        me: Option<u32>,
        own_seat: Option<Vec3>,
        index: &HashMap<u32, usize>,
    ) -> Option<glam::DVec3> {
        self.client_passenger_riding_position_inner(vehicle, me, own_seat, index, &mut Vec::new())
    }

    pub(in crate::game::client_side) fn client_passenger_riding_position_inner(
        &self,
        vehicle: u32,
        me: Option<u32>,
        own_seat: Option<Vec3>,
        index: &HashMap<u32, usize>,
        path: &mut Vec<u32>,
    ) -> Option<glam::DVec3> {
        if path.len() >= 64 || path.contains(&vehicle) {
            return None;
        }
        path.push(vehicle);
        if Some(vehicle) == me {
            let at = own_seat?;
            return Some(glam::DVec3::new(at.x, at.y, at.z));
        }
        if let Some(v) = index
            .get(&vehicle)
            .map(|&j| &self.common.mobs[j])
            .or_else(|| self.common.mobs.iter().find(|m| m.id == vehicle))
        {
            // Another player: the top of its box for the pose its server synced.
            let attachment = if &*v.kind.id == "minecraft:player" {
                [
                    0.0,
                    player_box_top(v.extra.synced_pose, riders::scale(v)),
                    0.0,
                ]
            } else {
                riders::passenger_attachment(v)
            };
            let at = riders::seat(attachment, 0.0, v.position, v.yaw);
            return Some(glam::DVec3::new(at.x, at.y, at.z));
        }
        if let Some(d) = self.client.text_displays.0.get(&(vehicle as i32)) {
            return if let Some(parent) = d.vehicle {
                self.client_passenger_riding_position_inner(parent, me, own_seat, index, path)
            } else {
                Some(d.position())
            };
        }
        if let Some(i) = self.client.interactions.get(&(vehicle as i32)) {
            if let Some(parent) = i.vehicle {
                return self
                    .client_passenger_riding_position_inner(parent, me, own_seat, index, path);
            }
            let at = i.position;
            return Some(glam::DVec3::new(at.x, at.y, at.z));
        }
        if let Some(b) = self.common.boats.iter().find(|b| b.id == vehicle) {
            let count = self.client_passenger_order(vehicle).len().max(1);
            let at = b.seat(0, count, false, 0.0, false);
            return Some(glam::DVec3::new(at.x, at.y, at.z));
        }
        if let Some(c) = self.common.carts.iter().find(|c| c.id == vehicle) {
            let at = c
                .position
                .add_vector(Vec3::new(0.0, super::minecarts::SEAT, 0.0));
            return Some(glam::DVec3::new(at.x, at.y, at.z));
        }
        if let Some(a) = self.common.armor_stands.iter().find(|a| a.id == vehicle) {
            let height = if a.marker {
                0.0
            } else if a.small {
                f64::from(0.9875_f32)
            } else {
                f64::from(1.975_f32)
            };
            let at = if let Some(parent) = a.vehicle {
                self.client_passenger_riding_position_inner(parent, me, own_seat, index, path)?
            } else {
                glam::DVec3::new(a.position.x, a.position.y, a.position.z)
            };
            return Some(at + glam::DVec3::Y * height);
        }
        if let Some(seat) = self.client_general_seat(vehicle, false) {
            let at = seat.passenger_position(0.0);
            return Some(glam::DVec3::new(at.x, at.y, at.z));
        }
        // Any other tracked entity sits on its feet; an area effect cloud is
        // 0.5 tall, and ModelEngine parks a model's pivot half a block below
        // its feet for exactly that.
        let remote = self.client.remote.as_ref()?;
        let (kind, position) = remote.tracked(vehicle as i32)?;
        let position = if let Some(parent) = remote.passenger_vehicle(vehicle) {
            let at =
                self.client_passenger_riding_position_inner(parent, me, own_seat, index, path)?;
            [at.x, at.y, at.z]
        } else {
            position
        };
        let lift = if kind == "minecraft:area_effect_cloud" {
            0.5
        } else if kind == "minecraft:player" {
            // A player this client has no copy of yet, so no synced pose: standing.
            player_box_top(0, 1.0)
        } else {
            0.0
        };
        Some(glam::DVec3::new(
            position[0],
            position[1] + lift,
            position[2],
        ))
    }

    /// Client-side `Entity.positionRider` for mobs, displays, interactions, and stands.
    pub(in crate::game) fn client_position_riders(&mut self) {
        let mut riders = Riders::default();
        for (j, m) in self.common.mobs.iter().enumerate() {
            if let Some(v) = m.extra.mount {
                riders.entry(v).or_default().push(j);
            }
        }
        self.client_seat_boat_mobs(&riders);
        self.common.seat_cart_mobs(&riders);
        self.common.seat_tnt_mobs(&riders);
        self.common.seat_cushion_mobs(&riders);
        self.position_remote_general_passengers();
        self.client_seat_static_mobs(&riders);
        let index: HashMap<u32, usize> = self
            .common
            .mobs
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        // Riders of riders settle over a few passes.
        for _ in 0..3 {
            for i in 0..self.common.mobs.len() {
                let Some(v) = self.common.mobs[i].extra.mount else {
                    continue;
                };
                let Some(&j) = index.get(&v) else {
                    if self.common.seats_own_riders(v)
                        || self.client_general_seat(v, false).is_some()
                    {
                        continue;
                    }
                    let me = self.client.remote.as_ref().map(|r| r.entity_id as u32);
                    let own_seat = self.common.player.as_ref().map(|p| {
                        p.position
                            .add_vector(Vec3::new(0.0, f64::from(p.dimensions.height), 0.0))
                    });
                    let Some(at) = self.client_passenger_riding_position(v, me, own_seat, &index)
                    else {
                        continue;
                    };
                    let before = self
                        .client
                        .text_displays
                        .0
                        .get(&(v as i32))
                        .map(|d| d.seat().1)
                        .unwrap_or_else(|| {
                            let old = self.common.mobs[i].previous;
                            glam::DVec3::new(
                                old.x,
                                old.y + riders::vehicle_attachment(&self.common.mobs[i], 0.0),
                                old.z,
                            )
                        });
                    let m = &mut self.common.mobs[i];
                    let offset = riders::vehicle_attachment(m, 0.0);
                    let seat = Vec3::new(at.x, at.y, at.z);
                    m.seated(v, offset, Aabb::new(seat, seat));
                    sunrise_sim::mobs::living::teleports::set_pos(
                        m,
                        Vec3::new(at.x, at.y - offset, at.z),
                    );
                    m.previous = Vec3::new(before.x, before.y - offset, before.z);
                    m.velocity = Vec3::ZERO;
                    m.fall_distance = 0.0;
                    m.ground.on_ground = false;
                    m.extra.passenger = true;
                    m.ai.nav.stop();
                    continue;
                };
                let vehicle = &self.common.mobs[j];
                let place = if sunrise_sim::mobs::camel::is_camel(vehicle.species())
                    || vehicle.species() == sunrise_sim::mobs::Species::HappyGhast
                    || vehicle.kind.passenger_attachments.len() > 1
                {
                    self.client_passenger_place(v, self.common.mobs[i].id)
                } else {
                    (0, 1)
                };
                let attach = sunrise_gameplay_common::game::CommonGame::mob_passenger_attachment(
                    vehicle,
                    sunrise_sim::mobs::drops::is_animal(self.common.mobs[i].species()),
                    place.0,
                    place.1,
                );
                let (pos, prev) = (vehicle.position, vehicle.previous);
                let (yaw, prev_yaw) = (vehicle.yaw, vehicle.previous_yaw);
                let (body, prev_body) = (vehicle.body_yaw, vehicle.previous_body_yaw);
                let rider = &self.common.mobs[i];
                let controlling = riders::controls_vehicle(
                    vehicle,
                    rider,
                    vehicle.rider.is_some() || self.common.riding == Some(v),
                    self.items().entity_type_tags.has_bit(
                        rider.kind.flags.entity_tag_slot,
                        sunrise_items::items::EntityTypeTags::NON_CONTROLLING_RIDER,
                    ),
                );
                let takes_body = riders::takes_vehicle_body(vehicle, rider, controlling);
                let vehicle_width = vehicle.width();
                let vehicle_box = vehicle.bounds();
                let m = &mut self.common.mobs[i];
                let offset = riders::vehicle_attachment(m, vehicle_width);
                m.seated(v, offset, vehicle_box);
                sunrise_sim::mobs::living::teleports::set_pos(
                    m,
                    riders::seat(attach, offset, pos, yaw),
                );
                m.previous = riders::seat(attach, offset, prev, prev_yaw);
                m.velocity = Vec3::ZERO;
                m.fall_distance = 0.0;
                m.ground.on_ground = false;
                if takes_body {
                    m.body_yaw = body;
                    m.previous_body_yaw = prev_body;
                }
                m.extra.vehicle_body = Some((v, prev_body, body));
                m.extra.passenger = true;
                m.ai.nav.stop();
            }
        }
        // General rows riding a mob that was just seated.
        self.position_remote_general_passengers();
        let me = self.client.remote.as_ref().map(|r| r.entity_id as u32);
        let own_seat = self.common.player.as_ref().map(|p| {
            p.position
                .add_vector(Vec3::new(0.0, f64::from(p.dimensions.height), 0.0))
        });
        for _ in 0..3 {
            let mut vehicles: Vec<u32> = self
                .client
                .text_displays
                .0
                .values()
                .filter_map(|d| d.vehicle)
                .chain(self.client.interactions.values().filter_map(|i| i.vehicle))
                .collect();
            vehicles.sort_unstable();
            vehicles.dedup();
            let seats: HashMap<u32, glam::DVec3> = vehicles
                .into_iter()
                .filter_map(|v| {
                    self.client_passenger_riding_position(v, me, own_seat, &index)
                        .map(|at| (v, at))
                })
                .collect();
            let sections = self.client.client_pick.sections.get_mut();
            for (&id, display) in &mut self.client.text_displays.0 {
                if let Some(at) = display.vehicle.and_then(|v| seats.get(&v)) {
                    display.ride(*at);
                    sections.move_id(id as u32, display.position().to_array());
                }
            }
            for interaction in self.client.interactions.values_mut() {
                if let Some(at) = interaction.vehicle.and_then(|v| seats.get(&v)) {
                    crate::game::client_side::interactions::ride_to(interaction, *at);
                }
            }
        }
        for i in 0..self.common.armor_stands.len() {
            let Some(vehicle) = self.common.armor_stands[i].vehicle else {
                continue;
            };
            let Some(at) = self.client_passenger_riding_position(vehicle, me, own_seat, &index)
            else {
                continue;
            };
            let before = index
                .get(&vehicle)
                .map(|&j| {
                    let v = &self.common.mobs[j];
                    riders::seat(
                        riders::passenger_attachment(v),
                        0.0,
                        v.previous,
                        v.previous_yaw,
                    )
                })
                .unwrap_or(self.common.armor_stands[i].previous);
            let at = Vec3::new(at.x, at.y, at.z);
            let stand = &mut self.common.armor_stands[i];
            stand.position = at;
            stand.previous = before;
            stand.velocity = Vec3::ZERO;
        }
        self.sync_seated_pick_sections();
    }

    /// Armor stands riding the local player, on the top of its box.
    pub(in crate::game::client_side) fn seat_own_stands(&mut self) {
        let (Some(me), Some(p)) = (
            self.client.remote.as_ref().map(|r| r.entity_id as u32),
            self.common.player.as_ref(),
        ) else {
            return;
        };
        let up = Vec3::new(0.0, f64::from(p.dimensions.height), 0.0);
        let (at, before) = (
            p.position.add_vector(up),
            self.common.previous_position.add_vector(up),
        );
        for stand in self
            .common
            .armor_stands
            .iter_mut()
            .filter(|a| a.vehicle == Some(me))
        {
            stand.position = at;
            stand.previous = before;
            stand.velocity = Vec3::ZERO;
        }
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::passengers_common::PassengersCommonGameExt1 as _;
