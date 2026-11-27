//! Mobs riding mobs: boarding and dismounting, keeping riders on their
//! vehicles, the jockeys that spawn mounted (chicken jockeys, spider
//! jockeys, strider jockeys, zombie horsemen) and the skeleton horse trap.
//! Saving riders under `Passengers` lives in `persist.rs`.
use crate::game::ServerGame;
use std::collections::HashMap;
use sunrise_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource},
};
use sunrise_gameplay_common::game::passengers_common::{Riders, player_box_top};
use sunrise_sim::mobs::{Species, riders};

/// The seconds a skeleton horse trap waits for a player: 15 minutes.
pub(in crate::game) const TRAP_LIFETIME: i32 = 18000;
/// `SkeletonTrapGoal`: a player this close springs the trap.
pub(in crate::game) const TRAP_RANGE: f64 = 10.0;
/// Ticks of invulnerability the trap's riders and horses start with.
pub(in crate::game) const TRAP_INVULNERABLE: i32 = 60;

pub fn trap_horse_scatter(random: &mut LegacyRandom) -> (f64, f64) {
    (random.triangle(0.0, 1.1485), random.triangle(0.0, 1.1485))
}

impl ServerGame<'_> {
    /// Keep the actual removed object only while a passenger still refers to it.
    pub(in crate::game::server_side) fn retain_removed_vehicle(
        &mut self,
        id: u32,
        vehicle: sunrise_gameplay_common::game::entity_memory::RemovedVehicle,
    ) {
        if self.common.riding == Some(id)
            || self.ridden_by_guest(id)
            || self
                .mob_objects()
                .any(|mob| !mob.removed && mob.extra.mount == Some(id))
            || self.common.general_passengers(id).next().is_some()
        {
            vehicle.retire_attachment_registration();
            self.common.persist.removed_vehicles.insert(id, vehicle);
            self.score_destroyed_entities();
        }
    }

    pub fn release_removed_vehicle(&mut self, id: u32) {
        if self.common.persist.removed_vehicles.contains_key(&id)
            && self.common.riding != Some(id)
            && !self.ridden_by_guest(id)
            && !self
                .mob_objects()
                .any(|mob| !mob.removed && mob.extra.mount == Some(id))
            && self.common.general_passengers(id).next().is_none()
        {
            self.common.persist.removed_vehicles.remove(&id);
            self.forget_entity(id);
        }
    }

    /// Resident passengers below an object that never joined cannot be reached
    /// by ServerLevel's recursive passenger tick. Empty on the normal path.
    pub fn mobs_below_off_level(&self) -> std::collections::HashSet<u32> {
        if self.common.persist.off_level.is_empty() {
            return Default::default();
        }
        let mounts: HashMap<_, _> = self
            .mob_objects()
            .filter(|m| !m.removed)
            .map(|m| (m.id, m.extra.mount))
            .collect();
        self.common
            .mobs
            .iter()
            .filter_map(|m| {
                let mut mount = m.extra.mount;
                for _ in 0..mounts.len() {
                    let id = mount?;
                    if self.common.persist.off_level.contains_key(&id) {
                        return Some(m.id);
                    }
                    mount = mounts.get(&id).copied().flatten();
                }
                None
            })
            .collect()
    }

    /// `Entity.startRiding` for two mobs.
    pub(in crate::game::server_side) fn mount_mob(&mut self, rider: u32, vehicle: u32) {
        self.try_mount_mob(rider, vehicle);
    }
    /// `Entity.startRiding` for two mobs: false when a mod canceled it.
    pub fn try_mount_mob(&mut self, rider: u32, vehicle: u32) -> bool {
        let vehicle = if self.common.boats.iter().any(|b| b.id == vehicle) {
            super::commands::Target::Boat(vehicle)
        } else if self.common.carts.iter().any(|c| c.id == vehicle) {
            super::commands::Target::Cart(vehicle)
        } else {
            super::commands::Target::Mob(vehicle)
        };
        self.start_riding_target(super::commands::Target::Mob(rider), vehicle, true)
    }
    /// Riders sit on their vehicles (`Entity.positionRider`), mobs or boats,
    /// and let go when the vehicle dies or is gone.
    pub fn sync_riders(&mut self) {
        if !self.common.persist.off_level.is_empty()
            || !self.common.persist.removed_vehicles.is_empty()
        {
            self.sync_retained_riders();
            self.position_riders();
            if !self.common.persist.removed_vehicles.is_empty()
                || self
                    .common
                    .persist
                    .off_level
                    .values()
                    .any(sunrise_gameplay_common::game::entity_memory::OffLevel::is_removed)
            {
                // Keep a removed vehicle only while cancellation leaves actual
                // references to it; completed dismounts release its ownership.
                self.prune_entity_memory();
            }
            return;
        }
        // Entity.baseTick.
        for m in &mut self.common.mobs {
            if m.extra.boarding_cooldown > 0 {
                m.extra.boarding_cooldown -= 1;
            }
        }
        let boats: std::collections::HashSet<u32> =
            self.common.boats.iter().map(|b| b.id).collect();
        let carts: std::collections::HashSet<u32> =
            self.common.carts.iter().map(|c| c.id).collect();
        let cushions: std::collections::HashSet<u32> =
            self.common.cushions.iter().map(|c| c.id).collect();
        let tnts: std::collections::HashSet<u32> = self
            .common
            .tnts
            .iter()
            .map(|t| t.id)
            .filter(|&id| id != 0)
            .collect();
        let index: HashMap<u32, usize> = self
            .common
            .mobs
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        let mut riding: HashMap<u32, (i64, u32)> = HashMap::new();
        for i in 0..self.common.mobs.len() {
            let Some(v) = self.common.mobs[i].extra.mount else {
                continue;
            };
            // Not a text display: a server's display mounts come through
            // `remote_set_passengers`, which seats them in `position_riders`.
            let valid = !self.common.mobs[i].removed
                && (boats.contains(&v)
                    || carts.contains(&v)
                    || tnts.contains(&v)
                    || cushions.contains(&v)
                    || self.seat(v).is_some()
                    || index.get(&v).is_some_and(|&j| !self.common.mobs[j].removed)
                    || self.player_vehicle(v).is_some());
            if valid {
                let place = (
                    self.common.mobs[i].extra.vehicle_order,
                    self.common.mobs[i].id,
                );
                riding
                    .entry(v)
                    .and_modify(|first| *first = (*first).min(place))
                    .or_insert(place);
            } else {
                let m = &mut self.common.mobs[i];
                m.extra.mount = None;
                m.extra.passenger = false;
            }
        }
        for m in &mut self.common.mobs {
            m.extra.rider = riding.get(&m.id).map(|&(_, id)| id);
        }
        self.position_riders();
    }
    pub(in crate::game::server_side) fn sync_retained_riders(&mut self) {
        let invalid: Vec<_> = self
            .mob_objects()
            .filter_map(|m| {
                if m.removed {
                    return None;
                }
                let v = m.extra.mount?;
                // Removed vehicles are retried only in the passenger's own
                // eligible entity tick, after its cancellable Pre event.
                if self.common.persist.removed_vehicles.contains_key(&v) {
                    return None;
                }
                let valid = self.common.mob_object(v).is_some_and(|m| !m.removed)
                    || self.boat_object(v).is_some()
                    || self.cart_object(v).is_some()
                    || self.common.cushions.iter().any(|c| c.id == v)
                    || self.common.tnts.iter().any(|t| t.id == v && v != 0)
                    || self.seat(v).is_some()
                    || self.player_vehicle(v).is_some();
                (!valid).then_some((m.id, v))
            })
            .collect();
        for (id, vehicle) in invalid {
            if self.common.mob_object(id).and_then(|m| m.extra.mount) != Some(vehicle) {
                continue;
            }
            // Dismount supplies positioning, descendant snapping and the
            // cancellable mount hook. Never overwrite its post-callback edge.
            self.dismount_mob(id);
        }
        let mut first: HashMap<u32, (i64, u32)> = HashMap::new();
        for m in self.mob_objects() {
            if let Some(v) = m.extra.mount {
                let place = (m.extra.vehicle_order, m.id);
                first
                    .entry(v)
                    .and_modify(|p| *p = (*p).min(place))
                    .or_insert(place);
            }
        }
        let ids: Vec<_> = self.mob_objects().map(|m| m.id).collect();
        for id in ids {
            self.mob_object_mut(id).unwrap().extra.rider = first.get(&id).map(|&(_, id)| id);
        }
        let blocked = self.mobs_below_off_level();
        for m in &mut self.common.mobs {
            if !blocked.contains(&m.id) && m.extra.boarding_cooldown > 0 {
                m.extra.boarding_cooldown -= 1;
            }
        }
    }

    /// The passengers of `vehicle` in `getPassengers` order on the server.
    pub(in crate::game::server_side) fn passenger_order(&self, vehicle: u32) -> Vec<u32> {
        self.seated_passengers(vehicle)
            .into_iter()
            .map(|(_, id, _)| id)
            .collect()
    }

    /// `rider`'s seat index and the passenger count on `vehicle`.
    pub(in crate::game) fn passenger_place(&self, vehicle: u32, rider: u32) -> (usize, usize) {
        let order = self.passenger_order(vehicle);
        let index = order.iter().position(|&id| id == rider).unwrap_or(0);
        (index, order.len().max(1))
    }

    /// Server-side `Entity.positionRider` for a general passenger, whose own vehicle attachment is zero.
    pub(in crate::game) fn server_general_passenger_position(
        &self,
        vehicle: u32,
        rider: u32,
    ) -> Option<Vec3> {
        if let Some(seat) = self.seat(vehicle) {
            return Some(seat.passenger_position(0.0));
        }
        if let Some(b) = self.common.boats.iter().find(|b| b.id == vehicle) {
            let (index, count) = self.passenger_place(vehicle, rider);
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
            let (index, count) = self.passenger_place(vehicle, rider);
            let attachment = sunrise_gameplay_common::game::CommonGame::mob_passenger_attachment(
                v, false, index, count,
            );
            return Some(riders::seat(attachment, 0.0, v.position, v.yaw));
        }
        let at = self.server_passenger_riding_position(vehicle, None, None, &HashMap::new())?;
        Some(Vec3::new(at.x, at.y, at.z))
    }

    /// Server-side `Entity.getPassengerRidingPosition`.
    pub(in crate::game) fn server_passenger_riding_position(
        &self,
        vehicle: u32,
        me: Option<u32>,
        own_seat: Option<Vec3>,
        index: &HashMap<u32, usize>,
    ) -> Option<glam::DVec3> {
        self.server_passenger_riding_position_inner(vehicle, me, own_seat, index, &mut Vec::new())
    }

    pub(in crate::game::server_side) fn server_passenger_riding_position_inner(
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
        if let Some(b) = self.common.boats.iter().find(|b| b.id == vehicle) {
            let count = self.boat_players(vehicle).max(1) + self.boat_mobs(vehicle).len();
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
                self.server_passenger_riding_position_inner(parent, me, own_seat, index, path)?
            } else {
                glam::DVec3::new(a.position.x, a.position.y, a.position.z)
            };
            return Some(at + glam::DVec3::Y * height);
        }
        if let Some(seat) = self.general_seat(vehicle, false) {
            let at = seat.passenger_position(0.0);
            return Some(glam::DVec3::new(at.x, at.y, at.z));
        }
        None
    }

    /// Server-side `Entity.positionRider` for every mob and stand passenger.
    pub(in crate::game) fn server_position_riders(&mut self) {
        let blocked = self.mobs_below_off_level();
        let mut riders = Riders::default();
        for (j, m) in self.common.mobs.iter().enumerate() {
            if blocked.contains(&m.id) {
                continue;
            }
            if let Some(v) = m.extra.mount {
                riders.entry(v).or_default().push(j);
            }
        }
        let mut general_vehicles =
            std::mem::take(&mut self.common.resident_entities.general.riders.mob_vehicles);
        general_vehicles.clear();
        if !self.common.resident_entities.general.rows.is_empty() {
            general_vehicles.extend(
                riders
                    .keys()
                    .copied()
                    .filter(|&vehicle| self.common.general_object(vehicle).is_some()),
            );
        }
        self.common.resident_entities.general.riders.mob_vehicles = general_vehicles;
        self.seat_boat_mobs(&riders);
        self.common.seat_cart_mobs(&riders);
        self.common.seat_tnt_mobs(&riders);
        self.common.seat_cushion_mobs(&riders);
        self.seat_static_mobs(&riders);
        let index: HashMap<u32, usize> = self
            .common
            .mobs
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        for _ in 0..3 {
            for i in 0..self.common.mobs.len() {
                if blocked.contains(&self.common.mobs[i].id) {
                    continue;
                }
                let Some(v) = self.common.mobs[i].extra.mount else {
                    continue;
                };
                let Some(&j) = index.get(&v) else {
                    if self.common.seats_own_riders(v) || self.general_seat(v, false).is_some() {
                        continue;
                    }
                    let _ = self.seat_player_passenger(i, v);
                    continue;
                };
                let vehicle = &self.common.mobs[j];
                let place = if sunrise_sim::mobs::camel::is_camel(vehicle.species())
                    || vehicle.species() == sunrise_sim::mobs::Species::HappyGhast
                    || vehicle.kind.passenger_attachments.len() > 1
                {
                    self.passenger_place(v, self.common.mobs[i].id)
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
        for i in 0..self.common.armor_stands.len() {
            let Some(vehicle) = self.common.armor_stands[i].vehicle else {
                continue;
            };
            let Some(at) = self.server_passenger_riding_position(vehicle, None, None, &index)
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
    }

    /// `SkeletonHorse.customServerAiStep` and `SkeletonTrapGoal`: a trap
    /// horse is a trap for fifteen minutes; a player within ten blocks springs
    /// it.
    pub(in crate::game::server_side) fn tick_skeleton_traps(&mut self) {
        // `Level.hasNearbyAlivePlayer`: alive, not a spectator, in the level.
        let players: Vec<_> = self
            .level_players()
            .into_iter()
            .filter(|p| p.alive && !p.spectator)
            .map(|p| p.position)
            .collect();
        let mut sprung = Vec::new();
        let blocked = self.mobs_below_off_level();
        for m in &mut self.common.mobs {
            if blocked.contains(&m.id) {
                continue;
            }
            if m.species() != Species::SkeletonHorse || !m.extra.trap || !m.alive() {
                continue;
            }
            if !m.persistent {
                m.extra.trap_time += 1;
                if m.extra.trap_time > TRAP_LIFETIME {
                    m.removed = true;
                    continue;
                }
            }
            if players
                .iter()
                .any(|&p| m.distance_sqr(p) < TRAP_RANGE * TRAP_RANGE)
            {
                sprung.push(m.id);
            }
        }
        for id in sprung {
            self.spring_skeleton_trap(id);
        }
    }
    pub fn spring_skeleton_trap(&mut self, id: u32) {
        let Some(horse) = self.common.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        horse.extra.trap = false;
        horse.extra.tame = true;
        horse.extra.age = 0;
        let at = [horse.position.x, horse.position.y, horse.position.z];
        self.spawn_bolt(at, true);
        let Some(skeleton) = self.trap_skeleton(at) else {
            return;
        };
        self.mount_mob(skeleton, id);
        for _ in 0..3 {
            let Some(other) = self.spawn_mob(
                "minecraft:skeleton_horse",
                at,
                true,
                sunrise_gameplay_common::game::creatures::SpawnSite::Finalizing(
                    sunrise_sim::mobs::spawn_rules::Reason::Triggered,
                ),
            ) else {
                continue;
            };
            if let Some(h) = self.common.mobs.iter_mut().find(|m| m.id == other) {
                h.invulnerable_time = TRAP_INVULNERABLE;
                h.extra.tame = true;
                h.extra.age = 0;
            }
            if let Some(rider) = self.trap_skeleton(at) {
                self.mount_mob(rider, other);
                let Some(horse) = self.common.mobs.iter_mut().find(|m| m.id == id) else {
                    continue;
                };
                let (dx, dz) = trap_horse_scatter(&mut horse.random);
                if let Some(h) = self.common.mobs.iter_mut().find(|m| m.id == other) {
                    h.velocity = Vec3::new(h.velocity.x + dx, h.velocity.y, h.velocity.z + dz);
                }
            }
        }
    }
    /// `SkeletonTrapGoal.createSkeleton`: an iron helmet unless it already
    /// has a helm.
    pub(in crate::game::server_side) fn trap_skeleton(&mut self, at: [f64; 3]) -> Option<u32> {
        let id = self.spawn_mob(
            "minecraft:skeleton",
            at,
            true,
            sunrise_gameplay_common::game::creatures::SpawnSite::Finalizing(
                sunrise_sim::mobs::spawn_rules::Reason::Triggered,
            ),
        )?;
        let m = self.common.mobs.iter_mut().find(|m| m.id == id)?;
        m.invulnerable = TRAP_INVULNERABLE;
        if m.extra.head_id().is_none() {
            m.extra.set_slot_id(
                sunrise_items::equipment::HEAD,
                Some("minecraft:iron_helmet".into()),
            );
        }
        let special = self.common.local_difficulty(at).special();
        // `SkeletonTrapGoal.enchant`: clear existing enchantments on the weapon
        // and the helmet, then apply `mob_spawn_equipment`, always.
        for slot in [
            sunrise_sim::mobs::finalize::MAINHAND,
            sunrise_sim::mobs::finalize::HEAD,
        ] {
            if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) {
                sunrise_sim::mobs::finalize::set_slot_enchantments(m, slot, Vec::new());
            }
            self.enchant_item(id, slot, special);
        }
        Some(id)
    }
}

#[cfg(test)]
pub(in crate::game) mod player_seat_tests {
    #[test]
    fn another_players_passengers_sit_on_its_box_for_its_pose() {
        // Player.POSES heights as floats widened to doubles: standing 1.8, crouching 1.5,
        // swimming 0.6.
        assert_eq!(super::player_box_top(0, 1.0), 1.7999999523162842);
        assert_eq!(super::player_box_top(5, 1.0), 1.5);
        assert_eq!(super::player_box_top(3, 1.0), 0.6000000238418579);
        assert_eq!(super::player_box_top(1, 1.0), 0.6000000238418579, "gliding");
        assert_eq!(super::player_box_top(4, 1.0), 0.6000000238418579, "riptide");
        assert_eq!(
            super::player_box_top(2, 1.0),
            0.20000000298023224,
            "sleeping"
        );
        assert_eq!(
            super::player_box_top(12, 1.0),
            1.7999999523162842,
            "a pose with no box stands"
        );
        assert_eq!(super::player_box_top(5, 2.0), 3.0, "scaled with the player");
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod nested_vehicle_discard_tests {
    pub fn set_vehicle_test_attachment(
        game: &mut crate::game::ServerGame<'_>,
        key: &mod_api::AttachmentKey<i32>,
    ) {
        // The real store path binds the resident owner's registration. Rebinding
        // it manually would replace that lifetime before removal is exercised.
        key.set(game, mod_api::Holder::Entity(71), 37).unwrap();
    }
}

#[cfg(test)]
pub(in crate::game) mod trap_scatter_tests {
    use super::*;
    #[test]
    fn trap_horse_scatter_draws_two_triangles_from_original_horse_random() {
        let mut horse_random = LegacyRandom::new(0x5DEECE66D);
        let mut expected = horse_random.clone();
        for _ in 0..3 {
            let (dx, dz) = trap_horse_scatter(&mut horse_random);
            assert_eq!(dx, expected.triangle(0.0, 1.1485));
            assert_eq!(dz, expected.triangle(0.0, 1.1485));
        }
        assert_eq!(horse_random.next_long(), expected.next_long());
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::passengers_common::PassengersCommonGameExt1 as _;
