//! Boats, minecarts, item frames, armor stands and paintings for a server's
//! client: single player on its server thread, a LAN guest, a player on a
//! dedicated server.
//!
//! The client aims at them like at mobs (`Minecraft.hitResult`, then
//! `ServerboundInteractPacket`), and rides the vehicle the server seats it in
//! (`ClientPacketListener.handleSetEntityPassengersPacket`). A boat is
//! controlled by its rider's client: it ticks here and its moves go to the
//! server (`LocalPlayer.tick`: `ServerboundMoveVehiclePacket`,
//! `AbstractBoat.tick`: `ServerboundPaddleBoatPacket`). So is a saddled horse,
//! donkey, mule or camel: its rider's client moves it and reports the moves,
//! and tells the server of a riding jump (`START_RIDING_JUMP`). Pigs and striders
//! also move on their rider's client when saddled and steered with the right stick.
//! A minecart
//! moves on the server with the rider's input, and the rider sits in it.
use super::Keys;
use crate::game::client_side::remote_client::Remote;
use sunrise_client::keybinds;
use sunrise_core::geometry::Vec3;
use sunrise_game_data::text::Text;
use sunrise_gameplay_common::game::remote::passengers::{PassengerSeat, passenger_chain};
use sunrise_protocol::packets::play::{self, Serverbound as S};
use sunrise_sim::mobs::{Mob, equine, happy_ghast};
use sunrise_world::stepped_interpolation::PosRot;

impl Remote {
    pub(in crate::game::client_side) fn riding_entity_exists(
        &self,
        id: i32,
        local_exists: bool,
    ) -> bool {
        if id == self.entity_id {
            local_exists
        } else {
            self.entities.contains_key(&id)
        }
    }

    fn forget_passengers(&mut self, vehicle: u32) {
        self.passenger_vehicles
            .retain(|_, seat| seat.vehicle != vehicle);
    }

    pub fn passenger_vehicle(&self, passenger: u32) -> Option<u32> {
        self.passenger_vehicles
            .get(&passenger)
            .map(|seat| seat.vehicle)
    }

    /// Passenger IDs and their packet indices, not sorted.
    pub(in crate::game) fn passenger_seats(
        &self,
        vehicle: u32,
    ) -> impl Iterator<Item = (u32, usize)> + '_ {
        self.passenger_vehicles
            .iter()
            .filter_map(move |(&id, seat)| (seat.vehicle == vehicle).then_some((id, seat.index)))
    }

    /// The lowest packet seat index strictly after `after` (or the first seat).
    pub(in crate::game) fn next_passenger_seat(
        &self,
        vehicle: u32,
        after: Option<usize>,
    ) -> Option<(u32, usize)> {
        self.passenger_seats(vehicle)
            .filter(|&(_, index)| after.is_none_or(|after| index > after))
            .min_by_key(|&(_, index)| index)
    }

    pub(in crate::game) fn has_passengers(&self, vehicle: u32) -> bool {
        self.passenger_seats(vehicle).next().is_some()
    }
}

/// An object under the crosshair: its entity id, how far along the view it
/// was hit, and where (`EntityHitResult.getLocation` minus the entity's
/// position, the interact packet's location).
pub(in crate::game) struct ObjectHit {
    pub id: i32,
    pub distance: f64,
    pub location: [f64; 3],
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Whether a use starts on an entity (`Minecraft.hitResult` is an `EntityHitResult`).
    pub fn remote_use_targets_entity(&self) -> bool {
        self.entity_targeted()
    }

    fn pick_object_origin(&self, id: u32) -> Option<[f64; 3]> {
        if let Some((_, _, origin)) = self.client_object_target(id as i32) {
            return Some(origin);
        }
        if let Some(row) = self
            .client
            .remote
            .as_ref()
            .and_then(|r| r.general.rows.get(&(id as i32)))
        {
            return Some(row.position);
        }
        if let Some(&block) = self.client.remote_knots.get(&id) {
            let p = sunrise_sim::mobs::leash::knot_state(block).position;
            return Some([p.x, p.y, p.z]);
        }
        if let Some(k) = self
            .common
            .resident_entities
            .knots
            .iter()
            .find(|k| k.id == id)
        {
            let p = sunrise_sim::mobs::leash::knot_state(k.fence).position;
            return Some([p.x, p.y, p.z]);
        }
        if let Some(crystal) = self.common.crystals.iter().find(|c| c.id == id) {
            let p = crystal.position;
            return Some([p.x, p.y, p.z]);
        }
        if let Some(d) = self
            .common
            .dragon
            .as_ref()
            .filter(|d| d.id != 0 || self.client.remote.is_some())
            && let Some(part) =
                sunrise_gameplay_common::game::dragon_common::dragon_part_index(d.id, id)
        {
            let p = d.parts[part].0;
            return Some([p.x, p.y, p.z]);
        }
        None
    }

    /// The nearest pickable non-mob object the view hits within reach.
    pub(in crate::game::client_side) fn remote_object_hit(&self) -> Option<ObjectHit> {
        if let Some(hit) = self.cached_entity_pick() {
            let (entity, distance, point) = hit?;
            if self.common.mobs.iter().any(|m| m.id == entity.id())
                || self
                    .common
                    .creatures
                    .projectiles
                    .iter()
                    .any(|p| p.id == entity.id())
            {
                return None;
            }
            let origin = self
                .pick_object_origin(entity.id())
                .or_else(|| self.entity_position(entity))?;
            return Some(ObjectHit {
                id: entity.id() as i32,
                distance,
                location: std::array::from_fn(|i| point[i] - origin[i]),
            });
        }
        let hit = self.pick_entity_hit_unified()?;
        if self.common.mobs.iter().any(|m| m.id == hit.id)
            || self
                .common
                .creatures
                .projectiles
                .iter()
                .any(|p| p.id == hit.id)
        {
            return None;
        }
        let origin = self.pick_object_origin(hit.id)?;
        Some(ObjectHit {
            id: hit.id as i32,
            distance: hit.distance_sq.sqrt(),
            location: std::array::from_fn(|i| hit.point[i] - origin[i]),
        })
    }

    /// `ClientPacketListener.handleSetEntityPassengersPacket`: who rides
    /// `vehicle` now. The local player gets in or out; mobs and other
    /// players sit where the server keeps them.
    pub fn remote_set_passengers(&mut self, vehicle: i32, mut passengers: Vec<i32>) {
        let Some(remote) = self.client.remote.as_mut() else {
            return;
        };
        // The local entity is known from Login, even before its motion body
        // arrives on PlayerPosition. Other entities must already be tracked.
        let local_exists = remote.logged_in || self.common.player.is_some();
        if !remote.riding_entity_exists(vehicle, local_exists) {
            log::warn!("Sunrise: ignoring passenger list for unknown vehicle {vehicle}");
            return;
        }
        // Forced client startRiding still respects Entity.couldAcceptPassenger.
        if remote.tracked(vehicle).is_some_and(|(kind, _)| {
            !sunrise_gameplay_common::game::seats_common::could_accept_passenger(kind)
        }) {
            return;
        }
        let vehicle_id = vehicle as u32;
        remote.forget_passengers(vehicle_id);
        let mut index = 0;
        passengers.retain(|&passenger| {
            if !remote.riding_entity_exists(passenger, local_exists) {
                return false;
            }
            let rider = passenger as u32;
            if remote.passenger_vehicle(rider) == Some(vehicle_id)
                || passenger_chain(&remote.passenger_vehicles, vehicle_id)
                    .any(|ancestor| ancestor == rider)
            {
                return false;
            }
            remote.passenger_vehicles.insert(
                rider,
                PassengerSeat {
                    vehicle: vehicle_id,
                    index,
                },
            );
            index += 1;
            true
        });
        self.apply_remote_passengers(vehicle, &passengers);
    }

    /// Removing a vehicle ejects its passengers even without a final packet.
    pub(in crate::game::client_side) fn remote_clear_passengers(&mut self, vehicle: i32) {
        if let Some(remote) = self.client.remote.as_mut() {
            remote.forget_passengers(vehicle as u32);
            self.apply_remote_passengers(vehicle, &[]);
        }
    }

    /// A death respawn replaces the local entity even if its level and ID stay.
    pub(in crate::game::client_side) fn reset_remote_player_riding(&mut self) {
        // The old body is gone, not dismounting. Clear local control before
        // updating the vehicle's remaining list so it won't play a dismount.
        if let Some(vehicle) = self.common.riding.take() {
            if let Some(mob) = self
                .common
                .mob_index
                .get_mut(&mut self.common.mobs, vehicle)
            {
                mob.rider = None;
            }
            if let Some(i) = self.common.boats.iter().position(|b| b.id == vehicle) {
                self.common.boats.update(i, |b| b.lerp = None);
            }
        }
        if let Some(remote) = self.client.remote.as_mut() {
            let player = remote.entity_id;
            remote.first_passenger = false;
            remote.passenger_index = 0;
            remote.ghast_dismount = None;
            let parent = remote.passenger_vehicles.remove(&(player as u32));
            self.remote_clear_passengers(player);
            if let Some(parent) = parent
                && let Some(remote) = self.client.remote.as_mut()
            {
                let mut siblings: Vec<_> = remote.passenger_seats(parent.vehicle).collect();
                siblings.sort_unstable_by_key(|&(_, index)| index);
                let ids: Vec<_> = siblings
                    .into_iter()
                    .enumerate()
                    .map(|(index, (id, _))| {
                        remote.passenger_vehicles.insert(
                            id,
                            PassengerSeat {
                                vehicle: parent.vehicle,
                                index,
                            },
                        );
                        id as i32
                    })
                    .collect();
                self.apply_remote_passengers(parent.vehicle as i32, &ids);
            }
        }
    }

    /// Apply the admitted list to the presentation and simulation copies too.
    pub fn apply_remote_passengers(&mut self, vehicle: i32, passengers: &[i32]) {
        let Some(me) = self.client.remote.as_ref().map(|r| r.entity_id) else {
            return;
        };
        let vehicle_id = vehicle as u32;
        self.remote_seat_passengers(vehicle_id, passengers);
        let player_controls = passengers.first().is_some_and(|&id| {
            self.client
                .remote
                .as_ref()
                .is_some_and(|r| r.level_player(id).is_some())
        });
        if let Some(i) = self.common.boats.iter().position(|b| b.id == vehicle_id) {
            self.common.boats.update(i, |boat| {
                boat.paddle_controller = player_controls;
            });
        }

        for m in &mut self.common.mobs {
            // `Entity.isVehicle` (the saddle reins of a ridden horse): the
            // server sends no rider on the mob itself, only this list.
            if m.id == vehicle_id {
                m.extra.rider = passengers.first().map(|&p| p as u32);
            }
            if m.extra.mount == Some(vehicle_id) && !passengers.contains(&(m.id as i32)) {
                m.extra.mount = None;
                m.extra.passenger = false;
            } else if passengers.contains(&(m.id as i32)) {
                m.extra.mount = Some(vehicle_id);
                m.extra.passenger = true;
            }
        }
        for (id, display) in &mut self.client.text_displays.0 {
            if passengers.contains(id) {
                display.vehicle = Some(vehicle_id);
            } else if display.vehicle == Some(vehicle_id) {
                display.vehicle = None;
            }
        }
        for a in &mut self.common.armor_stands {
            if passengers.contains(&(a.id as i32)) {
                a.vehicle = Some(vehicle_id);
            } else if a.vehicle == Some(vehicle_id) {
                a.vehicle = None;
            }
        }
        for (&id, i) in &mut self.client.interactions {
            if passengers.contains(&id) {
                i.vehicle = Some(vehicle_id);
            } else if i.vehicle == Some(vehicle_id) {
                i.vehicle = None;
            }
        }
        if passengers.contains(&me)
            && let Some(r) = self.client.remote.as_mut()
        {
            r.first_passenger = passengers.first() == Some(&me);
            r.passenger_index = passengers.iter().position(|&p| p == me).unwrap_or(0);
        }
        if passengers.contains(&me) {
            if self.common.riding != Some(vehicle_id) {
                self.common.riding = Some(vehicle_id);
                // ClientPacketListener.handleSetEntityPassengersPacket: boarding says how to get off,
                // on the overlay and to the narrator.
                let key = if self.common.cushions.iter().any(|c| c.id == vehicle_id) {
                    "mount.sit"
                } else {
                    "mount.onboard"
                };
                let message = Text::translate(
                    key,
                    &[keybinds::display_name(
                        self.client.settings.keys.get("key.sneak"),
                    )],
                );
                self.say_system_now(&message);
                self.client.titles.set_overlay(message, false);
                if let Some(i) = self.common.carts.iter().position(|c| c.id == vehicle_id) {
                    self.common.carts.update(i, |c| c.passenger_yaw_delta = 0.0);
                }
                if let Some(p) = self.common.player.as_mut() {
                    p.velocity = Vec3::ZERO;
                    p.fall_distance = 0.0;
                }
            }
        } else if self.common.riding == Some(vehicle_id) {
            // The server takes the player out, then teleports it. A boat's easing toward
            // a teleport this client was given ends with its control.
            self.common.riding = None;
            if let Some(i) = self.common.boats.iter().position(|b| b.id == vehicle_id) {
                self.common.boats.update(i, |b| b.lerp = None);
            }
            if let Some(m) = self
                .common
                .mob_index
                .get_mut(&mut self.common.mobs, vehicle_id)
            {
                m.rider = None;
            }
            // HappyGhast.removePassenger: a still timeout of 10 (the server's
            // STAYS_STILL confirms it), and the rider lands on top of the ghast
            // (`place_ghast_under`).
            if let Some(m) = self.common.mobs.iter_mut().find(|m| {
                m.id == vehicle_id && m.species() == sunrise_sim::mobs::Species::HappyGhast
            }) {
                m.extra.counter = 10;
                if let Some(r) = self.client.remote.as_mut() {
                    r.ghast_dismount = Some(vehicle_id);
                }
            }
        }
    }

    /// `Camera.alignWithEntity`'s `NewMinecartBehavior` branch: sample the
    /// vehicle's curved path at `partial`, then apply the camera entity's seat
    /// and vehicle attachment offsets. Camera's eye-height lerp runs afterwards.
    pub(in crate::game::client_side) fn remote_minecart_camera_position(
        &self,
        subject: &super::remote_camera::ResolvedCameraEntity,
        partial: f32,
    ) -> Option<[f64; 3]> {
        if !self.minecart_improvements() {
            return None;
        }
        let id = if subject.is_local_player {
            self.common.riding?
        } else {
            self.client
                .remote
                .as_ref()
                .filter(|r| r.detached_camera.is_none())?
                .passenger_vehicle(subject.entity_id)?
        };
        self.common.carts.iter().find(|c| c.id == id)?;
        let (mut position, _, _) = self.remote_minecart_lerp(id, partial)?;
        let seat_offset = if subject.is_local_player {
            let scale = f64::from(subject.camera.living.map_or(1.0, |l| l.scale));
            sunrise_gameplay_common::minecart_seat::PASSENGER_Y - 0.6 * scale
        } else if let Some(m) = self
            .common
            .mob_index
            .get_ref(&self.common.mobs, subject.entity_id)
        {
            sunrise_gameplay_common::minecart_seat::cart_seat_offset(
                m.species(),
                sunrise_sim::mobs::riders::vehicle_attachment(m, 0.98),
            )
        } else {
            sunrise_gameplay_common::minecart_seat::PASSENGER_Y
        };
        position[1] += seat_offset;
        Some(position)
    }

    fn controls_vehicle(&self, id: u32) -> bool {
        self.common.riding == Some(id)
            && self
                .client
                .remote
                .as_ref()
                .is_none_or(|r| r.first_passenger)
    }

    /// AbstractBoat.getControllingPassenger: `id` is a boat this client's player rides as its
    /// first passenger, the only one who steers it. Another vehicle is never a driven boat. A
    /// non-remote world retains its existing local movement ownership.
    pub fn controls_boat(&self, id: u32) -> bool {
        self.controls_vehicle(id) && self.common.boats.iter().any(|b| b.id == id)
    }

    /// Tick the local player's vehicle before the player tick:
    /// false when it rides nothing.
    pub fn remote_ride(&mut self, keys: Keys) -> bool {
        let Some(id) = self.common.riding else {
            return false;
        };
        // Settle an invisible seat riding a moving avatar before reading it.
        // The normal end-of-tick pass alone leaves the local rider a tick behind.
        self.position_riders();
        if let Some(i) = self.common.mobs.iter().position(|m| m.id == id) {
            // LocalPlayer.aiStep: the jump bar charges while jump is held.
            let jump = self.common.charge_mount_jump(i, keys);
            if self.drives(&self.common.mobs[i]) {
                self.drive_mount(i, keys, jump);
            }
            // Another animal moves on the server with the rider's input
            // (PlayerInput); the rider sits where `positionRider` puts it.
            self.position_rider();
            return true;
        }
        // Generic entities are valid vehicles too (plugins use displays as seats).
        // Keep riding set so sendChanges sends rotation, never a walking position.
        if !self.common.cushions.iter().any(|c| c.id == id)
            && !self.common.boats.iter().any(|b| b.id == id)
            && !self.common.carts.iter().any(|c| c.id == id)
        {
            let index = std::collections::HashMap::new();
            if let Some(at) = self.passenger_riding_position(id, None, None, &index) {
                let before = self
                    .client
                    .text_displays
                    .0
                    .get(&(id as i32))
                    .map(|d| d.seat().1)
                    .or_else(|| {
                        self.common
                            .armor_stands
                            .iter()
                            .find(|a| a.id == id)
                            .map(|a| {
                                let delta = a.previous.subtract(a.position);
                                at + glam::DVec3::new(delta.x, delta.y, delta.z)
                            })
                    });
                if let Some(p) = self.common.player.as_mut() {
                    let scale = p.dimensions.width
                        / sunrise_sim::pose::Pose::Standing.dimensions(1.0).width;
                    let (seat, previous) = generic_player_seat(at, before.unwrap_or(at), scale);
                    let previous = if before.is_some() {
                        previous
                    } else {
                        p.position
                    };
                    self.common.previous_position = if self.common.posed_rider != self.common.riding
                    {
                        seat
                    } else {
                        previous
                    };
                    self.common.posed_rider = self.common.riding;
                    p.position = seat;
                    p.velocity = Vec3::ZERO;
                    p.fall_distance = 0.0;
                }
                return true;
            }
        }
        let seat = if let Some(c) = self.common.cushions.iter().find(|c| c.id == id) {
            // The seat does not move; the rider sits on the cushion.
            Some(c.seat())
        } else if let Some(i) = self.common.boats.iter().position(|b| b.id == id) {
            // AbstractBoat.tick on the controlling client: steer and move.
            // It steers with the input `LocalPlayer.rideTick` latched on the
            // previous tick, so the paddle packet describes the motion the
            // server is sent; this tick's keys are latched after it ticked.
            let mut b = self.common.boats[i].clone();
            if self.controls_boat(id) {
                // A boat taken over while it eased toward the server's position keeps
                // easing (the handler stays the entity's), and reports where it goes.
                if let Some((to, rotation, steps)) = self
                    .client
                    .remote
                    .as_mut()
                    .and_then(|r| r.take_object_lerp(id))
                {
                    let rotation = rotation.unwrap_or((b.yaw, b.pitch));
                    b.go_on_easing(Vec3::new(to[0], to[1], to[2]), rotation, steps);
                }
                b = self.tick_boat(b, true);
                b.input = [keys.left, keys.right, keys.forward, keys.backward];
            }
            // positionRider turns and clamps the rider's yaw without changing yRotO.
            self.common.yaw += b.delta_rotation;
            self.common.yaw =
                sunrise_level::camera_math::clamp_boat_rotation(self.common.yaw, b.yaw).0;
            self.common.store_boat_back(i, b);
            // In front of a mob that shares the boat.
            Some(self.player_boat_seat(i))
        } else if self.common.carts.iter().any(|c| c.id == id) {
            // Minecart.positionRider runs on the client after the cart tick.
            // The offset belongs to the cart, so switching vehicles cannot
            // transfer the previous vehicle's pending turn.
            let rotate = self.minecart_improvements()
                && self.client.settings.file.bool("rotateWithMinecart");
            if let Some(i) = self.common.carts.iter().position(|c| c.id == id) {
                self.common.carts.update(i, |c| {
                    c.passenger_yaw_delta = 0.0;
                    if rotate && c.kind == super::minecarts::CartKind::Rideable {
                        crate::game::client_side::minecarts_client::rotate_passenger(
                            c,
                            &mut self.common.yaw,
                        );
                    }
                });
            }
            // A minecart: the server moves it, the rider sits in it.
            self.common.carts.iter().find(|c| c.id == id).map(|c| {
                c.position
                    .add_vector(Vec3::new(0.0, super::minecarts::SEAT, 0.0))
            })
        } else if let Some(at) =
            self.passenger_riding_position(id, None, None, &std::collections::HashMap::new())
        {
            let scale = self.common.player.as_ref().map_or(1.0, |p| {
                p.dimensions.width / sunrise_sim::pose::Pose::Standing.dimensions(1.0).width
            });
            Some(Vec3::new(at.x, at.y - f64::from(0.6_f32 * scale), at.z))
        } else {
            // Entity.baseTick: a passenger whose vehicle is gone stops
            // riding it. Nothing with the id is in the level any more (it
            // died, or the world it rode in was left), so the seat goes.
            self.common.riding = None;
            return false;
        };
        if let Some(p) = self.common.player.as_mut() {
            // The tick a rider takes its seat is a teleport (`Entity.moveTo`,
            // which sets the old position too): the camera must not sweep from
            // where the player stood to the cushion, boat or cart. The ticks
            // after it move with the vehicle and interpolate as usual.
            let first = self.common.posed_rider != self.common.riding;
            self.common.posed_rider = self.common.riding;
            if let Some(v) = seat {
                self.common.previous_position = if first { v } else { p.position };
                p.position = v;
            } else {
                self.common.previous_position = p.position;
            }
            p.velocity = Vec3::ZERO;
            p.fall_distance = 0.0;
        }
        true
    }

    /// `LocalPlayer.tick` for a passenger: the boat it controls goes to the
    /// server every tick with its paddles (the rider's own position does not).
    pub fn remote_vehicle_packets(&self) -> Vec<S> {
        if let Some(m) = self.driven_mount() {
            return vec![mount_move(m, self.driven_mount_heading(m))];
        }
        let Some(b) = self
            .common
            .riding
            .filter(|&id| self.controls_boat(id))
            .and_then(|id| self.common.boats.iter().find(|b| b.id == id))
        else {
            return Vec::new();
        };
        // ServerboundMoveVehiclePacket.fromEntity: while a teleport eases the
        // boat, the spot it is going to.
        let (at, yaw, pitch) = b.client_position();
        vec![
            S::MoveVehicle(play::MoveVehicleRequest {
                position: [at.x, at.y, at.z],
                yaw,
                pitch,
                on_ground: b.ground.on_ground,
            }),
            S::PaddleBoat(play::PaddleBoat {
                left: b.paddling[0],
                right: b.paddling[1],
            }),
        ]
    }

    /// The vehicle this client moves, when it is `id`, as `PositionMoveRotation.of` sees it:
    /// its client position and rotation (`getClientPositionAndRotation`: while an
    /// interpolation runs, where it is going) and its known movement, which is its controller's
    /// (`getKnownMovement`): the local player's, nothing while it rides (`Entity.rideTick`).
    pub(in crate::game::client_side) fn driven_vehicle(
        &self,
        id: u32,
    ) -> Option<(Vec3, f32, f32, Vec3)> {
        let known = self
            .common
            .player
            .as_ref()
            .map_or(Vec3::ZERO, |p| p.velocity);
        if let Some(m) = self.driven_mount().filter(|m| m.id == id) {
            let at = self.driven_mount_heading(m);
            let [x, y, z] = at.position;
            return Some((Vec3::new(x, y, z), at.yaw, at.pitch, known));
        }
        let b = self
            .driven_boat()
            .filter(|&i| self.common.boats[i].id == id)?;
        let (at, yaw, pitch) = self.common.boats[b].client_position();
        Some((at, yaw, pitch, known))
    }

    /// The boat this client steers, as an index into `boats`.
    pub fn driven_boat(&self) -> Option<usize> {
        self.common
            .riding
            .filter(|&id| self.controls_boat(id))
            .and_then(|id| self.common.boats.iter().position(|b| b.id == id))
    }

    /// `Entity.getClientPosition` of the vehicle this client moves: for a boat a
    /// teleport is easing, where it is going.
    pub(in crate::game::client_side) fn driven_client_position(&self) -> Option<Vec3> {
        if let Some(m) = self.driven_mount() {
            let [x, y, z] = self.driven_mount_heading(m).position;
            return Some(Vec3::new(x, y, z));
        }
        self.driven_boat()
            .map(|i| self.common.boats[i].client_position().0)
    }

    /// `handleTeleportEntity` (`setValuesFromPositionPacket`) for the vehicle this
    /// client moves, with the packet's relative parts already counted: it eases there
    /// (`moveOrInterpolateTo`) unless it is more than 64 blocks away, when it is put
    /// there and reported to the server at once. Its deltaMovement and onGround are the
    /// packet's.
    pub(in crate::game::client_side) fn teleport_driven_vehicle(
        &mut self,
        to: [f64; 3],
        rotation: (f32, f32),
        velocity: Vec3,
        on_ground: bool,
    ) {
        let at = Vec3::new(to[0], to[1], to[2]);
        if let Some(id) = self.driven_mount().map(|m| m.id) {
            // tooBigToInterpolate measures from where the mount is going while it eases.
            let far = self.driven_mount().is_some_and(|m| {
                let [x, y, z] = self.driven_mount_heading(m).position;
                Vec3::new(x, y, z).distance_squared(at) > 4096.0
            });
            if far {
                self.remote_move_vehicle_and_file(to, rotation);
            } else if let Some(i) = self.common.mobs.iter().position(|m| m.id == id) {
                let at = PosRot {
                    position: to,
                    yaw: rotation.0,
                    pitch: rotation.1,
                };
                self.interpolate_driven_mount_to(i, at);
            }
            if let Some(m) = self.common.mob_index.get_mut(&mut self.common.mobs, id) {
                m.velocity = velocity;
                m.ground.on_ground = on_ground;
            }
            if far && let Some(report) = self.remote_vehicle_packets().into_iter().next() {
                self.send(report);
            }
            return;
        }
        let Some(i) = self.driven_boat() else {
            return;
        };
        // tooBigToInterpolate measures from where the boat is going while it eases.
        let far = self.common.boats[i]
            .client_position()
            .0
            .distance_squared(at)
            > 4096.0;
        let id = self.common.boats[i].id;
        self.common.boats.update(i, |b| {
            if far {
                b.lerp = None;
                b.position = at;
                b.previous = at;
                b.yaw = rotation.0;
                b.previous_yaw = rotation.0;
                b.pitch = rotation.1;
            } else {
                b.interpolate_to(at, rotation.0, rotation.1);
            }
            b.velocity = velocity;
            b.ground.on_ground = on_ground;
        });
        if far {
            self.entity_section_moved_id(id);
            if let Some(report) = self.remote_vehicle_packets().into_iter().next() {
                self.send(report);
            }
        }
    }

    /// `ClientPacketListener.handleMoveVehicle`: the server put the boat or
    /// the mount the player drives back (`absSnapTo`: position and rotation, the
    /// old ones too; its deltaMovement stays).
    pub(in crate::game::client_side) fn remote_move_vehicle(
        &mut self,
        position: [f64; 3],
        (yaw, pitch): (f32, f32),
    ) {
        let at = Vec3::new(position[0], position[1], position[2]);
        // absSnapRotationTo.
        let (yaw, pitch) = (yaw % 360.0, pitch.clamp(-90.0, 90.0) % 360.0);
        if let Some(id) = self.driven_mount().map(|m| m.id) {
            self.cancel_driven_mount_interpolation(id);
        }
        if let Some(id) = self.driven_mount().map(|m| m.id)
            && let Some(m) = self.common.mob_index.get_mut(&mut self.common.mobs, id)
        {
            m.previous = at;
            m.position = at;
            m.yaw = yaw;
            m.previous_yaw = yaw;
            m.head_pitch = pitch;
            m.previous_head_pitch = pitch;
            return;
        }
        let Some(i) = self.driven_boat() else {
            return;
        };
        // After the interpolation is cancelled.
        self.common.boats.update(i, |b| {
            b.lerp = None;
            b.previous = at;
            b.position = at;
            b.yaw = yaw;
            b.previous_yaw = yaw;
            b.pitch = pitch;
        });
    }

    /// `AbstractBoat.handleEntityEvent`: 71 and 72, a bubble column dragging
    /// the boat down or throwing it up (`handleBubbleColumnEffect`, which the
    /// client applies to the boat it drives).
    pub(in crate::game::client_side) fn remote_boat_event(&mut self, id: i32, event: i8) {
        let drag_down = event == sunrise_gameplay_common::game::boats_common::BUBBLE_DOWN_EVENT;
        if !drag_down && event != sunrise_gameplay_common::game::boats_common::BUBBLE_UP_EVENT {
            return;
        }
        if let Some(i) = self.common.boats.iter().position(|b| b.id == id as u32) {
            self.handle_bubble_column_effect(i, drag_down);
        }
    }

    /// `AbstractBoat.tick`: the part that reaches bubble columns, for every
    /// boat on a client: `status`, the two `applyEffectsFromBlocks` (a boat
    /// above a column may splash; the boat the player drives is lifted or
    /// dragged by a column it is inside) and `tickBubbleColumn`'s wobble from
    /// the synched bubble time, followed by the oars on boats this client does not drive.
    pub(in crate::game) fn tick_remote_boat_bubbles(&mut self) {
        for i in 0..self.common.boats.len() {
            let driven = self.controls_vehicle(self.common.boats[i].id);
            if !driven {
                // `AbstractBoat.tick`: the non-authoritative copy carries no
                // cached motion into the next time this player drives it.
                self.common.boats.update(i, |b| b.velocity = Vec3::ZERO);
                self.common.boats.update(i, |b| b.delta_rotation = 0.0);
                self.refresh_boat_status(i);
            }
            let touches = self.common.boat_bubble_columns(i);
            for pass in 0..2 {
                if pass == 1 && driven {
                    self.boat_second_step(i);
                }
                for &column in &touches {
                    if column.surface {
                        if self.common.boat_splash_roll(i) {
                            self.boat_bubble_splash(i);
                        }
                    } else if driven {
                        self.boat_inside_bubble_column(i, column);
                    }
                }
            }
            self.common.boats.update(
                i,
                sunrise_gameplay_common::game::boats_common::Boat::tick_bubble_wobble,
            );
            if !driven {
                self.common
                    .boats
                    .update(i, |b| Self::tick_boat_paddles(b, &mut self.client.audio.0));
            }
        }
    }

    /// Whether this client moves `m`, the mount the player rides: as its
    /// first passenger (`AbstractHorse.getControllingPassenger`), on its
    /// saddle (`isLocalInstanceAuthoritative`).
    pub(in crate::game) fn drives(&self, m: &Mob) -> bool {
        self.common.riding == Some(m.id)
            && client_moves(
                m,
                self.client
                    .remote
                    .as_ref()
                    .is_some_and(|r| r.first_passenger),
                self.steers_mount(m),
            )
    }

    fn steers_mount(&self, m: &Mob) -> bool {
        sunrise_gameplay_common::game::mounts_common::steering_item(
            m.species(),
            self.common.held().map(|s| s.item.as_ref()),
            self.common.offhand.as_ref().map(|s| s.item.as_ref()),
        )
    }

    /// The mount this client moves, if the player rides one.
    pub fn driven_mount(&self) -> Option<&Mob> {
        let id = self.common.riding?;
        self.common
            .mobs
            .iter()
            .find(|m| m.id == id && self.drives(m))
    }

    /// `LocalPlayer.aiStep` decides sprint after the rider's base tick sampled fluids.
    pub(in crate::game::client_side) fn remote_rider_sprint(
        &mut self,
        keys: Keys,
        window: i32,
    ) -> sunrise_game_data::Result<()> {
        let mount = self.common.mobs.iter().position(|m| self.drives(m));
        let can_sprint = mount.is_some_and(|i| equine::can_sprint(self.common.mobs[i].species()));
        self.player_sprint_tick(keys, window, Some((can_sprint, mount.is_some())))
    }

    /// `LivingEntity.travelRidden`, then `LocalPlayer.aiStep`, on the
    /// controlling client. The vehicle ticks before its passenger
    /// (`ClientLevel.tickNonPassenger`), so the mount moves (`tickRidden`,
    /// then `travel`) on what the rider's previous tick left: `xxa`/`zza` from
    /// `applyInput` (the keys through `modifyInput`: ×0.98, square movement),
    /// the sprint and the jump `onPlayerJump` set. Only then does this tick's
    /// aiStep read the keys, send a released jump (`sendRidingJump`) and run
    /// `onPlayerJump`, for the mount's next tick. Moving on this tick's keys
    /// put every ridden move a tick ahead of the server's view of the input.
    pub fn drive_mount(&mut self, i: usize, keys: Keys, jump: f32) {
        let old_pose = self.interpolate_driven_mount(i);
        let steer = self.steers_mount(&self.common.mobs[i]);
        let (yaw, pitch) = (self.common.yaw, self.common.pitch);
        // The first ridden tick reads what the player's last walking tick applied.
        let walked = self
            .common
            .player
            .as_ref()
            .map_or([0.0; 2], |p| p.applied_input);
        // Camel.getRiddenSpeed reads the live controller flag before the rider ticks.
        let sprint = self.common.player.as_ref().is_some_and(|p| p.sprinting);
        let m = &mut self.common.mobs[i];
        let last = m.rider.unwrap_or(equine::Rider {
            strafe: walked[0],
            forward: walked[1],
            ..Default::default()
        });
        // tickRidden reads the controller's look as it is now.
        m.rider = Some(equine::Rider {
            yaw,
            pitch,
            steer,
            sprint,
            client: false,
            ..last
        });
        let mut mob = self.common.mobs.remove(i);
        mob.extra.client_side = true;
        let mut lent = sunrise_sim::health::LentLevel::take(&mut self.common.world);
        let mut events = Vec::new();
        let hooks = self.common.damage_hooks.client_side();
        {
            let side = self.client_mob_side();
            let world = self.client_mob_world_base(&[], &side);
            let mut level = self.predicting_damage_level(&mut lent, &hooks);
            mob.tick(&world, &mut level, &mut events);
        }
        // LivingEntity.baseTick's tickEffects, inside the mount's own tick (#3948).
        super::potions::client_tick_effects(
            &mut mob.extra.client_effects,
            &mut mob.extra.darkness_blend,
        );
        // Only the server's packets take the mount away.
        mob.removed = false;
        lent.give_back(&mut self.common.world);
        // Entity.commonTick's setOldPosAndRot came before the interpolation step: the
        // frame eases from the last tick's pose, not from the stepped one.
        if let Some((position, yaw, pitch)) = old_pose {
            mob.previous = position;
            mob.previous_yaw = yaw;
            mob.previous_head_pitch = pitch;
        }
        self.common.mobs.insert(i, mob);
        // LocalPlayer.aiStep, after the vehicle's tick.
        if jump > 0.0
            && let Some(id) = self.client.remote.as_ref().map(|r| r.entity_id)
        {
            self.send(riding_jump(id, self.common.mount.jump_scale));
        }
        let [strafe, forward] =
            sunrise_game_data::move_input::modify_input(keys.direction(), None, None);
        let m = &mut self.common.mobs[i];
        if jump > 0.0 && equine::is_equine(m.species()) {
            // `onPlayerJump`: the mount slides on the rider's keys and rears
            // while the server does the same.
            equine::on_player_jump(m);
        }
        // playerJumpPendingScale waits for the ground (the tick spends it).
        let pending = m.rider.map_or(0.0, |r| r.jump);
        m.rider = Some(equine::Rider {
            strafe,
            forward,
            yaw,
            pitch,
            jump: if jump > 0.0 { jump } else { pending },
            steer,
            sprint,
            jumping: keys.jump,
            sneak: keys.shift,
            client: false,
        });
        // #4821: footstep, gallop, breathe, splash and landing sounds from mob.tick
        let _ = self.handle_mob_events(events);
    }

    /// The level a mount this client predicts is hurt in, with the client's hooks (`hooks`)
    /// and the tags its server sent, so its tag view answers the server's membership.
    pub fn predicting_damage_level<'a>(
        &'a self,
        lent: &'a mut sunrise_sim::health::LentLevel,
        hooks: &'a sunrise_sim::health::DamageHooks,
    ) -> sunrise_sim::health::DamageLevel<'a> {
        sunrise_sim::health::DamageLevel::lent(
            &self.common.world,
            self.common.pack.assets.registry,
            lent,
            self.client.remote.as_ref().and_then(|r| r.tags.as_ref()),
            None,
            hooks,
        )
        .on_client(true)
    }
}

/// Whether a player's client moves `m`, the mount it rides
/// (`isLocalInstanceAuthoritative`): a saddled horse, donkey, mule or camel
/// it is the first passenger of (`AbstractHorse.getControllingPassenger`),
/// or a harnessed adult Happy Ghast it is the first passenger of
/// (`HappyGhast.getControllingPassenger`: wearing body armor, player on
/// top, not on a still timeout — the rider steers it with `travelRidden`
/// on their own client, and the mount goes to the server as `MoveVehicle`).
pub fn client_moves(m: &Mob, first_passenger: bool, steer: bool) -> bool {
    if !first_passenger || !m.alive() {
        return false;
    }
    if equine::moved_by_rider(m.species()) && m.extra.saddled() {
        return !matches!(
            m.species(),
            sunrise_sim::mobs::Species::Pig | sunrise_sim::mobs::Species::Strider
        ) || steer;
    }
    m.species() == sunrise_sim::mobs::Species::HappyGhast
        && happy_ghast::harnessed_adult(m)
        && !happy_ghast::on_still_timeout(m)
}

/// `LocalPlayer.sendRidingJump`: START_RIDING_JUMP with
/// `floor(getJumpRidingScale() * 100)`, as `onPlayerJump` gets it.
pub fn riding_jump(player: i32, scale: f32) -> S {
    S::PlayerCommand(play::PlayerCommand {
        id: player,
        action: 3,
        data: (scale * 100.0).floor() as i32,
    })
}

/// `ServerboundMoveVehiclePacket.fromEntity`: where the mount this client
/// moves is, and how it is turned (`at`: getClientPositionAndRotation).
pub fn mount_move(m: &Mob, at: PosRot) -> S {
    S::MoveVehicle(play::MoveVehicleRequest {
        position: at.position,
        yaw: at.yaw,
        pitch: at.pitch,
        on_ground: m.ground.on_ground,
    })
}

/// Avatar's VEHICLE attachment scales independently of the vehicle's seat.
pub fn generic_player_seat(at: glam::DVec3, before: glam::DVec3, scale: f32) -> (Vec3, Vec3) {
    let offset = 0.6 * f64::from(scale);
    (
        Vec3::new(at.x, at.y - offset, at.z),
        Vec3::new(before.x, before.y - offset, before.z),
    )
}

use sunrise_gameplay_common::game::boats_common::BoatsCommonGameExt1 as _;

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;

use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;

use sunrise_gameplay_common::game::mounts_common::MountsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::passengers_common::PassengersCommonGameExt1 as _;

use sunrise_gameplay_common::game::minecarts::MinecartRailGameExt as _;

use sunrise_gameplay_common::game::boats_common::BoatStatusGameExt as _;

use sunrise_gameplay_common::game::native_hooks::can_start_swimming::SprintGameExt as _;
