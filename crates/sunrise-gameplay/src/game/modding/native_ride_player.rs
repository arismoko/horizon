//! A mob riding a server player (`Entity.startRiding` with a player as the
//! vehicle), for `EntityRef::ride_player` only (#4456): Tinkers' Construct's
//! piggyback pack seats a mob on its wearer. `EntityRef::start_riding` keeps
//! refusing a player vehicle.
//!
//! Vanilla never seats anything on a player (`/ride` refuses with
//! `cant_ride_players`), so every path here is reached only after a mod's
//! call left a mob's `extra.mount` naming a player: the mount checks, the
//! seat (`seat_player_passenger`), the validity (`player_vehicle`) and the
//! vehicle player's own `SetPassengers` (`tell_vehicle_players`) all start by
//! finding a player behind that id, and find none in a vanilla game.
use super::commands::Target;
use mod_api::{EntityRef, RidePlayerRefusal};
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_protocol::packets::play;
use sunrise_sim::lan::entity::Riding;
use sunrise_sim::mobs::{Species, riders};

impl crate::game::ServerGame<'_> {
    /// The entity id a player target has: the vehicle id its passengers keep.
    pub(in crate::game::server_side) fn player_vehicle_id(&self, player: Target) -> Option<u32> {
        match player {
            Target::Player => self
                .common
                .player
                .as_ref()
                .map(|_| self.own_entity_id() as u32)
                .filter(|&id| id != 0),
            Target::Guest(conn) => self
                .command_guest(conn)
                .filter(|g| g.dimension == self.common.dim.id())
                .map(|g| g.entity_id as u32),
            _ => None,
        }
    }

    /// `Entity.startRiding(player, force)` for the mob `entity`, with each
    /// refusal named. The mount itself is `start_riding_target`'s, the path
    /// every other vehicle takes.
    pub fn entity_ride_player(
        &mut self,
        entity: EntityRef,
        player: EntityRef,
        force: bool,
    ) -> Result<(), RidePlayerRefusal> {
        let rider = self
            .native_entity_target(entity)
            .ok_or(RidePlayerRefusal::NoEntity)?;
        let vehicle = self
            .native_entity_target(player)
            .ok_or(RidePlayerRefusal::NoEntity)?;
        if !matches!(vehicle, Target::Player | Target::Guest(_)) {
            return Err(RidePlayerRefusal::NotAPlayer);
        }
        let vehicle_id = self
            .player_vehicle_id(vehicle)
            .ok_or(RidePlayerRefusal::NoEntity)?;
        if entity.id() == player.id() {
            return Err(RidePlayerRefusal::SameEntity);
        }
        let id = match rider {
            Target::Mob(id) => id,
            Target::Player | Target::Guest(_) => return Err(RidePlayerRefusal::PlayerRider),
            _ => return Err(RidePlayerRefusal::UnsupportedRider),
        };
        let mob = self
            .common
            .mobs
            .iter()
            .find(|m| m.id == id && !m.removed)
            .ok_or(RidePlayerRefusal::NoEntity)?;
        if mob.extra.mount == Some(vehicle_id) {
            return Err(RidePlayerRefusal::AlreadyRiding);
        }
        let cannot_ride = mob.extra.boarding_cooldown > 0
            || matches!(mob.species(), Species::Warden | Species::Wither);
        if self.player_rides(vehicle, id) {
            return Err(RidePlayerRefusal::Cycle);
        }
        if !force {
            if !self.seated_passengers(vehicle_id).is_empty() {
                return Err(RidePlayerRefusal::Full);
            }
            if cannot_ride {
                return Err(RidePlayerRefusal::CannotRide);
            }
        }
        if self
            .try_start_riding_with(rider, vehicle, force, true)
            .is_ok()
        {
            Ok(())
        } else {
            Err(RidePlayerRefusal::Canceled)
        }
    }

    /// Gets every mob off the player whose entity id is `vehicle`, or off every player when
    /// None: no save holds a player vehicle, and a player leaving, dying or changing level
    /// carries nothing with it. Each mob gets off as `stop_riding` takes it off (`entity_mount`
    /// hears it); one a listener kept on is taken off anyway. Nothing to do, and nothing
    /// allocated, while no mob rides a player, which is every vanilla game.
    pub(in crate::game) fn eject_player_riders(&mut self, vehicle: Option<u32>) {
        let own = self
            .common
            .player
            .as_ref()
            .map(|_| self.own_entity_id() as u32);
        let is_player =
            |game: &Self, v: u32| match vehicle {
                Some(only) => v == only,
                None => {
                    own == Some(v)
                        || game.server.lan.as_ref().is_some_and(|host| {
                            host.hub.guests.iter().any(|g| g.entity_id as u32 == v)
                        })
                }
            };
        if !self
            .common
            .mobs
            .iter()
            .any(|m| !m.removed && m.extra.mount.is_some_and(|v| is_player(self, v)))
        {
            return;
        }
        let riders: Vec<(u32, u32)> = self
            .common
            .mobs
            .iter()
            .filter(|m| !m.removed)
            .filter_map(|m| {
                m.extra
                    .mount
                    .filter(|&v| is_player(self, v))
                    .map(|v| (m.id, v))
            })
            .collect();
        for (id, v) in riders {
            self.dismount_mob(id);
            if self
                .common
                .mob_object(id)
                .is_some_and(|m| m.extra.mount == Some(v))
            {
                self.dismount_mob_reentrant(id);
            }
        }
    }

    /// Whether `player` rides the mob `rider`, through any number of vehicles.
    fn player_rides(&self, player: Target, rider: u32) -> bool {
        let mut current = self.native_player_vehicle_id(player);
        for _ in 0..=self.riding_link_limit() {
            let Some(id) = current else {
                return false;
            };
            if id == rider {
                return true;
            }
            current = self.vehicle_of_any(id);
        }
        true
    }

    /// `Entity.positionRider` for a mob on a player: the top of the player's
    /// box for its pose (`Player`'s default passenger attachment, unrotated
    /// as it lies on the axis), less the mob's own vehicle attachment. False,
    /// changing nothing, when `vehicle` is no player of this level.
    pub(in crate::game) fn seat_player_passenger(&mut self, index: usize, vehicle: u32) -> bool {
        if self.player_vehicle(vehicle).is_none() {
            return false;
        }
        let Some(player) = self.entity(vehicle) else {
            return false;
        };
        let (Some([x, _, z]), Some([low, high])) = (
            self.entity_position(player),
            self.entity_bounding_box(player),
        ) else {
            return false;
        };
        let width = (high[0] - low[0]) as f32;
        let player_box = Aabb::new(
            Vec3::new(low[0], low[1], low[2]),
            Vec3::new(high[0], high[1], high[2]),
        );
        let m = &mut self.common.mobs[index];
        let offset = riders::vehicle_attachment(m, width);
        m.seated(vehicle, offset, player_box);
        m.previous = m.position;
        sunrise_sim::mobs::living::teleports::set_pos(m, Vec3::new(x, high[1] - offset, z));
        m.velocity = Vec3::ZERO;
        m.fall_distance = 0.0;
        m.ground.on_ground = false;
        m.extra.passenger = true;
        m.ai.nav.stop();
        true
    }

    /// The vehicle players' own `SetPassengers`: no tracker sends a client its
    /// own entity, so a guest carrying a mob is told here when its passengers
    /// change, as the piggyback pack sends `ClientboundSetPassengersPacket` to
    /// its wearer. Nothing is sent while no guest carries anything and none
    /// was told it did, which is every vanilla tick.
    pub(in crate::game) fn tell_vehicle_players(&self, riding: &mut Riding) {
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let guests = &host.hub.guests;
        if riding.told_self.is_empty()
            && guests
                .iter()
                .all(|g| riding.passengers(g.entity_id).is_empty())
        {
            return;
        }
        for g in guests {
            let now = riding.passengers(g.entity_id);
            let told = riding
                .told_self
                .get(&g.entity_id)
                .map_or(&[][..], Vec::as_slice);
            if now == told {
                continue;
            }
            g.send(&play::Clientbound::SetPassengers(play::SetPassengers {
                vehicle: g.entity_id,
                passengers: now.to_vec(),
            }));
            let now = now.to_vec();
            if now.is_empty() {
                riding.told_self.remove(&g.entity_id);
            } else {
                riding.told_self.insert(g.entity_id, now);
            }
        }
        riding
            .told_self
            .retain(|id, _| guests.iter().any(|g| g.entity_id == *id));
    }
}

impl crate::game::ServerGame<'_> {
    /// The server player whose entity id is `id`, in the level in place: the
    /// player in place or a guest here. None on a client, or for any other id.
    pub(in crate::game) fn player_vehicle(&self, id: u32) -> Option<Target> {
        let entity = EntityRef::new(self.common.native_lifecycle.epoch()?, id);
        match self.native_entity_target(entity)? {
            Target::Player => self.common.player.as_ref().map(|_| Target::Player),
            Target::Guest(conn) => self
                .command_guest(conn)
                .filter(|g| g.dimension == self.common.dim.id())
                .map(|_| Target::Guest(conn)),
            _ => None,
        }
    }

    /// What `id` rides, when `id` may be a player: the next link of a
    /// vehicle walk that can pass through a player carrying a mob.
    pub(in crate::game) fn vehicle_of_any(&self, id: u32) -> Option<u32> {
        match self.player_vehicle(id) {
            Some(player) => self.native_player_vehicle_id(player),
            None => self
                .native_vehicle_id(Target::Mob(id))
                .or_else(|| self.native_vehicle_id(Target::General(id))),
        }
    }

    fn native_player_vehicle_id(&self, player: Target) -> Option<u32> {
        match player {
            Target::Player => self.common.riding,
            Target::Guest(conn) => self.command_guest(conn)?.vehicle,
            _ => None,
        }
    }
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
