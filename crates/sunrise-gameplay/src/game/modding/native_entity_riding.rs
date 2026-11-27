//! Vehicles and passengers through an `EntityRef`: what an entity rides, who rides it,
//! getting off, and who holds its lead. Basic topology reads also answer from the client's
//! packet state; riding writes and the remaining reads are server-only.
use super::{commands::Target, mounts_common::steering_item};
use core::ops::ControlFlow;
use mod_api::{EntityReadError, EntityRef, LeashHolder, LivingRef, StartRidingRefusal};
use sunrise_sim::{
    lan::playerdata::ints_uuid,
    mobs::{Mob, Species, equine, happy_ghast, leash::Holder, nautilus},
};

pub(in crate::game) fn visit_passenger_tree(
    root: u32,
    mut next_rider: impl FnMut(u32, Option<u32>) -> Option<u32>,
    parent: impl Fn(u32) -> Option<u32>,
    mut visit: impl FnMut(u32) -> ControlFlow<()>,
) -> ControlFlow<()> {
    let mut vehicle = root;
    let mut after = None;
    loop {
        if let Some(rider) = next_rider(vehicle, after) {
            if rider == root {
                after = Some(rider);
                continue;
            }
            visit(rider)?;
            vehicle = rider;
            after = None;
        } else if vehicle != root
            && let Some(parent) = parent(vehicle)
        {
            after = Some(vehicle);
            vehicle = parent;
        } else {
            return ControlFlow::Continue(());
        }
    }
}

impl crate::game::ServerGame<'_> {
    /// The dismount search with a vehicle or passenger lent out of `self.mobs`.
    pub(in crate::game::server_side) fn scoped_mob_dismount_location(
        &self,
        vehicle: Option<&Mob>,
        vehicle_id: u32,
        rider: Option<&Mob>,
        passenger: EntityRef,
    ) -> Option<[f64; 3]> {
        let at = if let Some(rider) = rider {
            if let Some(boat) = self.common.boats.iter().find(|boat| boat.id == vehicle_id) {
                Some(self.boat_dismount_target(boat, f64::from(rider.width()), rider.yaw))
            } else if let Some(mount) = vehicle {
                Some(
                    self.horse_dismount_mobs(mount, rider)
                        .unwrap_or_else(|| super::mounts::on_top(mount)),
                )
            } else {
                self.cart_dismount_mob(vehicle_id, rider)
            }
        } else if let Some(mount) = vehicle {
            match self.native_entity_target(passenger)? {
                Target::Player => Some(self.player_dismount_from_lent_mob(
                    mount,
                    self.common.yaw,
                    self.common.player.as_ref()?,
                )),
                Target::Guest(conn) => {
                    let guest = self.command_guest(conn)?;
                    Some(self.player_dismount_from_lent_mob(
                        mount,
                        guest.yaw,
                        guest.motion.as_deref()?,
                    ))
                }
                _ => None,
            }
        } else {
            let vehicle = self.entity(vehicle_id)?;
            return self.entity_dismount_location(vehicle, LivingRef::new(passenger));
        };
        if let Some(at) = at {
            return Some([at.x, at.y, at.z]);
        }
        let vehicle = self.entity(vehicle_id)?;
        let [x, _, z] = self.entity_position(vehicle)?;
        let [_, [_, top, _]] = self.entity_bounding_box(vehicle)?;
        Some([x, top, z])
    }

    /// A live portal root carries a direct or indirect mount through complete objects.
    pub(in crate::game::server_side) fn vehicle_tree_contains_mount(
        &self,
        root: u32,
        mount: u32,
    ) -> bool {
        // A retained mob has no handle: its own object is its target.
        let target = self
            .entity(mount)
            .and_then(|entity| self.native_entity_target(entity))
            .or_else(|| {
                self.common
                    .mob_object(mount)
                    .filter(|mob| !mob.removed)
                    .map(|_| Target::Mob(mount))
            });
        self.entity(root).is_some()
            && target.is_some_and(|target| self.native_root_vehicle_id(mount, target) == Some(root))
    }

    /// Whether the player `t` holds the stick that steers `species` in either hand.
    pub(in crate::game::server_side) fn native_player_steers(
        &self,
        species: Species,
        t: Target,
    ) -> bool {
        let (main, offhand) = match t {
            Target::Player => (self.common.held(), self.common.offhand.as_ref()),
            Target::Guest(conn) => {
                let Some(guest) = self.command_guest(conn) else {
                    return false;
                };
                (
                    guest.inventory.get(guest.selected).and_then(Option::as_ref),
                    guest.offhand.as_ref(),
                )
            }
            _ => return false,
        };
        steering_item(
            species,
            main.map(|s| s.item.as_ref()),
            offhand.map(|s| s.item.as_ref()),
        )
    }
    /// The overrides of `getControllingPassenger` on `mob`: whether the player `rider`, its
    /// first passenger, steers it.
    pub(in crate::game::server_side) fn native_player_controls(
        &self,
        mob: &Mob,
        rider: Target,
    ) -> bool {
        let species = mob.species();
        match species {
            Species::Pig | Species::Strider => {
                mob.extra.saddled() && self.native_player_steers(species, rider)
            }
            Species::HappyGhast => {
                happy_ghast::wearing_harness(mob) && !happy_ghast::on_still_timeout(mob)
            }
            s if equine::is_equine(s) || nautilus::is_nautilus(s) => mob.extra.saddled(),
            _ => false,
        }
    }
    /// The first passenger, when it steers the entity (`Entity.getControllingPassenger`): a
    /// player in a boat, a player on a mount that is ready for one, or a mob on a mob with AI
    /// unless `#non_controlling_rider` names it. `None` when nothing steers it, or for a stale
    /// handle.
    pub(in crate::game) fn entity_controlling_passenger(
        &self,
        entity: EntityRef,
    ) -> Option<LivingRef> {
        let vehicle = self.native_entity_target(entity)?;
        let first_id = self.first_seated_passenger(entity.id())?.1;
        let first = self.entity(first_id)?;
        let rider = self.native_entity_target(first)?;
        let is_player = matches!(rider, Target::Player | Target::Guest(_));
        match vehicle {
            Target::Boat(_) if is_player => Some(LivingRef::new(first)),
            Target::Mob(id) => {
                let mob = self.common.mobs.iter().find(|m| m.id == id)?;
                if is_player {
                    return self
                        .native_player_controls(mob, rider)
                        .then(|| LivingRef::new(first));
                }
                let rider = self.common.mobs.iter().find(|m| m.id == first.id())?;
                let tagged = self.items().entity_type_tags.has_bit(
                    rider.kind.flags.entity_tag_slot,
                    sunrise_items::items::EntityTypeTags::NON_CONTROLLING_RIDER,
                );
                (!mob.no_ai && !tagged).then(|| LivingRef::new(first))
            }
            _ => None,
        }
    }
    /// Whether both entities share a root vehicle (`Entity.isPassengerOfSameVehicle`): true for
    /// an entity and itself, false for two entities riding nothing. `None` for a stale handle.
    pub(in crate::game) fn entity_is_passenger_of_same_vehicle(
        &self,
        entity: EntityRef,
        other: EntityRef,
    ) -> Option<bool> {
        let target = self.native_entity_target(entity)?;
        let other_target = self.native_entity_target(other)?;
        Some(
            self.native_root_vehicle_id(entity.id(), target)?
                == self.native_root_vehicle_id(other.id(), other_target)?,
        )
    }
    /// Where getting off this vehicle puts `passenger` as it stands now, riding it or not. This
    /// is the game's own placement, from the code `stop_riding` runs, so it can differ from
    /// `Entity.getDismountLocationForPassenger`. A player leaves a boat ahead of them or on top
    /// of it, a cart beside or on top of it, a strider by vanilla's search, a happy ghast on
    /// top, and any other mob beside or on top of it. A mob leaves a boat as a player does, a
    /// cart by vanilla's search, a horse, camel or llama at its side or feet, and any other mob
    /// on top: vanilla's searches for `Animal` and the strider don't reach mobs yet, and those
    /// for horses, `Animal` and the cart don't reach players. Nothing rides anything else, so
    /// there it is the top of the box. Reads only: no `mob_ride` or dismount veto is asked and
    /// the passenger's pose stays. `None` for a stale handle, a passenger other than a player
    /// or a mob, a player with no motion state, or a vehicle whose box Sunrise does not keep.
    pub(in crate::game) fn entity_dismount_location(
        &self,
        vehicle: EntityRef,
        passenger: LivingRef,
    ) -> Option<[f64; 3]> {
        self.native_entity_target(vehicle)?;
        let id = vehicle.id();
        let at = match self.native_entity_target(passenger.entity())? {
            Target::Mob(rider) => self.mob_dismount_point(id, rider),
            Target::Player => {
                self.player_dismount_point(id, self.common.yaw, self.common.player.as_ref()?)
            }
            Target::Guest(conn) => {
                let guest = self.command_guest(conn)?;
                self.player_dismount_point(id, guest.yaw, guest.motion.as_deref()?)
            }
            _ => return None,
        };
        if let Some(at) = at {
            return Some([at.x, at.y, at.z]);
        }
        let [x, _, z] = self.entity_position(vehicle)?;
        let [_, [_, top, _]] = self.entity_bounding_box(vehicle)?;
        Some([x, top, z])
    }
    /// A lead's live holder as a handle: `None` when it is out of reach.
    pub(in crate::game::server_side) fn native_leash_holder(
        &self,
        holder: Option<Holder>,
        player: Option<[i32; 4]>,
    ) -> Option<LeashHolder> {
        Some(match holder {
            None => LeashHolder::Unleashed,
            Some(Holder::Knot(pos)) => LeashHolder::Knot(pos),
            Some(Holder::Mob(id)) => {
                let entity = self.entity(id)?;
                if !matches!(self.native_entity_target(entity), Some(Target::Mob(_))) {
                    return None;
                }
                LeashHolder::Entity(entity)
            }
            Some(Holder::Boat(id)) => {
                let entity = self.entity(id)?;
                if !matches!(self.native_entity_target(entity), Some(Target::Boat(_))) {
                    return None;
                }
                LeashHolder::Entity(entity)
            }
            Some(Holder::Player) => LeashHolder::Entity(self.player(ints_uuid(player?))?.entity()),
        })
    }
    /// The live holder of a mob's or boat's lead (`Leashable.getLeashHolder`), or `Unleashed`
    /// when no lead exists. `None` means an entity that takes no lead, or that the entity,
    /// holder or saved holder is unavailable, stale or in a parked level. Does not load a level.
    pub(in crate::game) fn entity_leash_holder(&self, entity: EntityRef) -> Option<LeashHolder> {
        match self.native_entity_target(entity)? {
            Target::Mob(id) => {
                let mob = self.common.mobs.iter().find(|m| m.id == id)?;
                if mob.extra.leash_pending.is_some() {
                    return None;
                }
                self.native_leash_holder(mob.extra.leash, mob.extra.leash_player)
            }
            Target::Boat(id) => {
                let boat = self.common.boats.iter().find(|b| b.id == id)?;
                if boat.leash_pending.is_some() {
                    return None;
                }
                self.native_leash_holder(boat.leash, self.boat_leash_player(boat))
            }
            _ => None,
        }
    }
    /// Gives the holder the mob's or boat's lead (`Leashable.setLeashedTo`): the entity lets go
    /// of any other holder and gets off its vehicle, where a mod's `mob_ride` lets it. As
    /// there, neither `canBeLeashed` nor the distance is asked: a holder too far away snaps
    /// the lead on the next tick. False for a stale handle, the entity itself, a holder other
    /// than a player, mob or boat, or an entity that takes no lead.
    pub(in crate::game) fn entity_leash_to(
        &mut self,
        entity: EntityRef,
        holder: EntityRef,
    ) -> bool {
        let (Some(target), Some(by)) = (
            self.native_entity_target(entity),
            self.native_entity_target(holder),
        ) else {
            return false;
        };
        if entity == holder {
            return false;
        }
        let (holder, player) = match by {
            Target::Mob(id) => (Holder::Mob(id), None),
            Target::Boat(id) => (Holder::Boat(id), None),
            Target::Player | Target::Guest(_) => match self.native_player_facts(by) {
                Some(facts) => (Holder::Player, Some(facts.uuid)),
                None => return false,
            },
            _ => return false,
        };
        self.native_set_leash(entity, target, holder, player)
    }
    /// Ties the mob's or boat's lead to the fence at `pos`, as `entity_leash_to` does to
    /// an entity; the knot is there while a lead hangs on it. False for a stale handle, an
    /// entity that takes no lead, or no fence at `pos`.
    pub(in crate::game) fn entity_leash_to_knot(
        &mut self,
        entity: EntityRef,
        pos: [i32; 3],
    ) -> bool {
        let Some(target) = self.native_entity_target(entity) else {
            return false;
        };
        let block = self
            .common
            .res
            .facts()
            .get(self.common.world.block(pos[0], pos[1], pos[2]));
        if !self.named_block_in_tag("minecraft:fences", &block.name) {
            return false;
        }
        self.native_set_leash(entity, target, Holder::Knot(pos), None)
    }
    /// `Leashable.setLeashedTo` from a handle: the lead first, then off the vehicle.
    pub(in crate::game::server_side) fn native_set_leash(
        &mut self,
        entity: EntityRef,
        target: Target,
        holder: Holder,
        player: Option<[i32; 4]>,
    ) -> bool {
        let leashed = self.set_leash(target, holder, player);
        if leashed {
            self.entity_stop_riding(entity);
        }
        leashed
    }
    /// Takes the lead off a mob or boat: the lead falls as an item with `drop_lead`
    /// (`Leashable.dropLeash`), and is gone without it (`removeLeash`). A mob's removal is heard
    /// by `entity_unleash`. False when no holder has the lead, for a stale handle, or for an
    /// entity that takes no lead. A holder still unresolved from a save is left alone.
    pub(in crate::game) fn entity_detach_leash(
        &mut self,
        entity: EntityRef,
        drop_lead: bool,
    ) -> bool {
        self.native_entity_target(entity)
            .is_some_and(|target| self.detach_leash(target, drop_lead))
    }
    /// Attempts one mount, shared by handle writes and the ride command.
    pub fn start_riding_target(&mut self, rider: Target, vehicle: Target, force: bool) -> bool {
        self.try_start_riding_target(rider, vehicle, force).is_ok()
    }

    /// `start_riding_target` with its refusal named (`StartRidingRefusal`).
    pub fn try_start_riding_target(
        &mut self,
        rider: Target,
        vehicle: Target,
        force: bool,
    ) -> Result<(), StartRidingRefusal> {
        self.try_start_riding_with(rider, vehicle, force, false)
    }

    /// `try_start_riding_target`, where `onto_player` admits a player vehicle for a mob rider:
    /// only `entity_ride_player` passes true, so every other call keeps refusing one, as
    /// vanilla's `startRiding` callers and `/ride` do.
    pub(in crate::game::server_side) fn try_start_riding_with(
        &mut self,
        rider: Target,
        vehicle: Target,
        force: bool,
        onto_player: bool,
    ) -> Result<(), StartRidingRefusal> {
        if let Target::Guest(conn) = rider {
            if self
                .command_guest(conn)
                .is_none_or(|guest| guest.dimension != self.common.dim.id())
            {
                return Err(StartRidingRefusal::NoEntity);
            }
            return self
                .for_player(rider, |game| {
                    game.try_start_riding_with(Target::Player, vehicle, force, onto_player)
                })
                .unwrap_or(Err(StartRidingRefusal::NoEntity));
        }
        let vehicle_id = self.native_riding_preconditions(rider, vehicle, force, onto_player)?;
        let previous_vehicle = self.native_vehicle_id(rider);
        let allowed = match rider {
            Target::Player => self.native_may_board(vehicle_id),
            Target::Mob(id) | Target::General(id) => self.native_mob_may_ride(id, vehicle_id, true),
            _ => false,
        };
        if !allowed || self.native_vehicle_id(rider) != previous_vehicle {
            return Err(StartRidingRefusal::Canceled);
        }
        self.native_riding_preconditions(rider, vehicle, force, onto_player)?;
        if let Some(previous) = previous_vehicle {
            match rider {
                Target::Player => self.stop_riding(),
                Target::Mob(id) => {
                    if self.common.persist.mount_notifications.contains(&(id, previous, false))
                        && self.common.persist.removals.iter().any(|target| {
                            matches!(*target, Target::Boat(n) | Target::Cart(n) if n == previous)
                        })
                    {
                        // This explicit callback mutation completes the in-flight edge,
                        // rather than recursively notifying it a second time.
                        self.dismount_mob_reentrant(id);
                    } else {
                        self.dismount_mob(id);
                    }
                }
                Target::General(id) => self.dismount_general(id, previous),
                _ => return Err(StartRidingRefusal::UnsupportedRider),
            }
            if self.native_vehicle_id(rider).is_some() {
                return Err(StartRidingRefusal::Canceled);
            }
        }
        // Dismount dispatch can also discard, remount or fill the target vehicle.
        self.native_riding_preconditions(rider, vehicle, force, onto_player)?;
        match rider {
            Target::Player => {
                match vehicle {
                    // `ServerPlayer.startRiding` teleports with `Relative.ROTATION`: the
                    // rider keeps their view, whatever the vehicle's yaw.
                    Target::Boat(_) => {}
                    Target::Mob(id) => {
                        if let Some(mob) = self.common.mobs.iter_mut().find(|m| m.id == id) {
                            mob.extra.flag = false;
                            mob.extra.counter = 0;
                        }
                        self.common.mount.jump_ticks = 0;
                        self.common.mount.jump_scale = 0.0;
                    }
                    Target::Stand(id) | Target::Other(id) | Target::General(id) => {
                        let Some(seat) = self.seat(id) else {
                            return Err(StartRidingRefusal::UnsupportedVehicle);
                        };
                        self.board_seat(id, seat);
                        return self.native_seated(rider, vehicle_id);
                    }
                    _ => {}
                }
                self.common.riding = Some(vehicle_id);
                self.seat_player(vehicle_id);
                self.trigger_player("minecraft:started_riding");
            }
            Target::Mob(id) => {
                sunrise_sim::mobs::riders::link(&mut self.common.mobs, id, vehicle_id);
                self.seat_mob(id);
                self.break_leash(id, true, false);
            }
            Target::General(id) => {
                let order = self.boarding_order(vehicle_id, id, false);
                if !self.set_general_mount(id, Some((vehicle_id, order))) {
                    return Err(StartRidingRefusal::NoEntity);
                }
                self.note_started_riding(vehicle_id);
            }
            _ => return Err(StartRidingRefusal::UnsupportedRider),
        }
        self.native_seated(rider, vehicle_id)
    }

    /// The mount's last check: the rider sits on the vehicle, or a hook it
    /// dispatched took it off again.
    pub(in crate::game::server_side) fn native_seated(
        &self,
        rider: Target,
        vehicle_id: u32,
    ) -> Result<(), StartRidingRefusal> {
        if self.native_vehicle_id(rider) == Some(vehicle_id) {
            Ok(())
        } else {
            Err(StartRidingRefusal::Canceled)
        }
    }

    /// Validate the captured relationship at each callback boundary, naming
    /// the first check that fails, in the order the checks have always run.
    pub(in crate::game::server_side) fn native_riding_preconditions(
        &self,
        rider: Target,
        vehicle: Target,
        force: bool,
        onto_player: bool,
    ) -> Result<u32, StartRidingRefusal> {
        use StartRidingRefusal as R;
        let vehicle_id = match vehicle {
            Target::Mob(id)
            | Target::Boat(id)
            | Target::Cart(id)
            | Target::Stand(id)
            | Target::General(id)
            | Target::Other(id) => id,
            // Only `ride_player` seats a mob on a player (`native_ride_player.rs`), at each
            // re-check too: the plain call refuses a player vehicle.
            Target::Player | Target::Guest(_) if onto_player && matches!(rider, Target::Mob(_)) => {
                self.player_vehicle_id(vehicle).ok_or(R::NoEntity)?
            }
            // `Entity.startRiding` refuses an unsaved vehicle type, the player's, before `force`.
            Target::Player | Target::Guest(_) => return Err(R::PlayerVehicle),
            _ => return Err(R::UnsupportedVehicle),
        };
        if self.entity(vehicle_id).is_none()
            || matches!(rider, Target::Mob(id) if self.mobs.iter().all(|m| m.id != id || m.removed))
            || matches!(rider, Target::General(id) if self.general_row(id).is_none())
        {
            return Err(R::NoEntity);
        }
        if !matches!(rider, Target::Player | Target::Mob(_) | Target::General(_)) {
            return Err(R::UnsupportedRider);
        }
        if rider == vehicle {
            return Err(R::SameEntity);
        }
        if self.native_vehicle_id(rider) == Some(vehicle_id) {
            return Err(R::AlreadyRiding);
        }
        if let Target::General(id) = vehicle {
            let slot = self.common.general_row(id).ok_or(R::NoEntity)?.type_slot;
            if super::general::general_type_catalog().save_policy(slot)
                == mod_api::GeneralSavePolicy::NoSave
            {
                return Err(R::UnsupportedVehicle);
            }
        }
        // Intermediate vehicles can be retained objects, not ordinary handles.
        let mut current = Some(vehicle_id);
        for _ in 0..=self.riding_link_limit() {
            let Some(id) = current else { break };
            if rider == Target::Mob(id)
                || rider == Target::General(id)
                || (rider == Target::Player && self.player_vehicle_id(rider) == Some(id))
            {
                return Err(R::Cycle);
            }
            current = self.vehicle_of_any(id);
        }
        if current.is_some() {
            return Err(R::Cycle);
        }
        if !force {
            match rider {
                Target::Player if self.server_acting_sneak() => {
                    return Err(R::CannotRide);
                }
                Target::Mob(id)
                    if self
                        .common
                        .mobs
                        .iter()
                        .find(|m| m.id == id)
                        .is_none_or(|m| {
                            m.extra.boarding_cooldown > 0
                                || matches!(
                                    m.species(),
                                    sunrise_sim::mobs::Species::Warden
                                        | sunrise_sim::mobs::Species::Wither
                                )
                        }) =>
                {
                    return Err(R::CannotRide);
                }
                Target::General(id)
                    if self
                        .common
                        .general_row(id)
                        .is_some_and(|row| row.boarding_cooldown > 0) =>
                {
                    return Err(R::CannotRide);
                }
                _ => {}
            }
        }
        match vehicle {
            Target::Boat(id) => {
                let index = self
                    .common
                    .boats
                    .iter()
                    .position(|b| b.id == id)
                    .ok_or(R::NoEntity)?;
                let full = if force {
                    self.seated_passenger_count(id) >= self.common.boats[index].max_passengers()
                } else {
                    !self.boat_has_room(index)
                };
                if full {
                    return Err(R::Full);
                }
            }
            Target::Cart(id) => {
                if !self.common.carts.iter().any(|cart| cart.id == id) {
                    return Err(R::NoEntity);
                }
                if self.has_seated_passengers(id) {
                    return Err(R::Full);
                }
            }
            Target::Mob(id) => {
                let mob = self
                    .common
                    .mobs
                    .iter()
                    .find(|m| m.id == id && m.alive())
                    .ok_or(R::NoEntity)?;
                let seats = self.seated_passengers(id);
                let players = seats
                    .iter()
                    .filter(|(_, _, player)| *player)
                    .count()
                    .max(usize::from(mob.rider.is_some()));
                let passengers = players + seats.iter().filter(|(_, _, player)| !*player).count();
                if !force && !sunrise_sim::mobs::riders::can_add_passenger(mob, passengers) {
                    return Err(R::Full);
                }
            }
            Target::Stand(id) | Target::Other(id) | Target::General(id) => {
                if self.seat(id).is_none() {
                    return Err(R::UnsupportedVehicle);
                }
                if !force && !self.seated_passengers(id).is_empty() {
                    return Err(R::Full);
                }
            }
            // `Player` keeps `Entity.canAddPassenger`: one passenger unless forced.
            Target::Player | Target::Guest(_) => {
                if !force && !self.seated_passengers(vehicle_id).is_empty() {
                    return Err(R::Full);
                }
            }
            _ => return Err(R::UnsupportedVehicle),
        }
        Ok(vehicle_id)
    }

    /// Attempts to board a vehicle through the shared server mount path.
    ///
    /// General rows use the same admission and cancellable mount notifications
    /// as players and mobs. Only the explicit `entity_ride_player` route lets a
    /// mob ride a player; force does not make unsaved vehicles eligible.
    pub fn entity_start_riding(
        &mut self,
        entity: EntityRef,
        vehicle: EntityRef,
        force: bool,
    ) -> Result<(), StartRidingRefusal> {
        let rider = self
            .native_entity_target(entity)
            .ok_or(StartRidingRefusal::NoEntity)?;
        let vehicle = self
            .native_entity_target(vehicle)
            .ok_or(StartRidingRefusal::NoEntity)?;
        self.try_start_riding_target(rider, vehicle, force)
    }

    /// Gets the entity off its vehicle (`Entity.stopRiding`), where the
    /// vehicle lets it out, as `/ride ... dismount` does. False when it rode
    /// nothing, a mod's `mob_ride` hook kept it on, or for a stale handle.
    pub fn entity_stop_riding(&mut self, entity: EntityRef) -> bool {
        let Some(t) = self.native_entity_target(entity) else {
            return false;
        };
        let Some(vehicle) = self.native_vehicle_id(t) else {
            return false;
        };
        match t {
            Target::Mob(id) => self.dismount_mob(id),
            Target::Player | Target::Guest(_) => {
                self.for_player(t, |g| g.stop_riding());
            }
            Target::General(id) => self.dismount_general(id, vehicle),
            _ => return false,
        }
        let stopped = self.native_vehicle_id(t).is_none();
        if stopped {
            self.release_removed_vehicle(vehicle);
        }
        stopped
    }
    pub(in crate::game::server_side) fn dismount_general(&mut self, id: u32, vehicle: u32) {
        if self.native_mob_may_ride(id, vehicle, false)
            && self.native_vehicle_id(Target::General(id)) == Some(vehicle)
        {
            self.set_general_mount(id, None);
            self.release_removed_vehicle(vehicle);
        }
    }

    /// `Entity.setRemoved`: stop each direct passenger in list order, not
    /// the reverse order of `ejectPassengers`. A callback can move a later
    /// passenger, so check each edge again before asking it to stop riding.
    pub(in crate::game::server_side) fn dismount_removed_passengers(
        &mut self,
        vehicle: u32,
        y: f64,
    ) {
        let passengers = self.seated_passengers(vehicle);
        let own = self
            .common
            .player
            .as_ref()
            .filter(|_| self.server.lan_acting.is_none())
            .map(|player| player.entity_id as u32);
        // An own-player edge that has no seat entry still needs to dismount.
        if let Some(own) = own
            && self.common.riding == Some(vehicle)
            && !passengers
                .iter()
                .any(|&(_, id, player)| player && id == own)
        {
            self.stop_riding_from_removed_vehicle(vehicle, y);
        }
        for (_, id, player) in passengers {
            if player {
                let target = if own == Some(id) {
                    Some(Target::Player)
                } else {
                    self.player_vehicle(id)
                };
                if let Some(target) = target
                    && self.native_vehicle_id(target) == Some(vehicle)
                {
                    self.for_player(target, |g| g.stop_riding_from_removed_vehicle(vehicle, y));
                }
            } else if self.native_vehicle_id(Target::Mob(id)) == Some(vehicle) {
                self.dismount_mob_from_removed_vehicle(id, y);
            } else if self.native_vehicle_id(Target::General(id)) == Some(vehicle) {
                self.dismount_general(id, vehicle);
            }
        }
    }

    pub(in crate::game::server_side) fn dismount_general_passengers(&mut self, vehicle: u32) {
        let mut passengers: Vec<_> = self
            .common
            .general_passengers(vehicle)
            .map(|row| (row.vehicle_order, row.id))
            .collect();
        passengers.sort_unstable();
        for (_, rider) in passengers.into_iter().rev() {
            if self.native_vehicle_id(Target::General(rider)) == Some(vehicle) {
                self.dismount_general(rider, vehicle);
            }
        }
    }

    /// Gets every passenger off the entity (`Entity.ejectPassengers`), last
    /// first as vanilla walks the list, so `mob_ride` hears them in its
    /// order. False when nothing rode it, or for a stale handle.
    pub fn entity_eject_passengers(&mut self, entity: EntityRef) -> bool {
        // Dismount hooks may change the live seat list, so take the initial handles first.
        let mut passengers = Vec::new();
        if self
            .entity_visit_passengers(entity, &mut |rider| {
                passengers.push(rider);
                ControlFlow::Continue(())
            })
            .is_err()
        {
            return false;
        }
        let mut any = false;
        for p in passengers.into_iter().rev() {
            any |= self.entity_stop_riding(p);
        }
        any
    }
}

impl crate::game::ServerGame<'_> {
    /// What the entity rides (`Entity.getVehicle`), if its vehicle has a handle.
    pub fn entity_vehicle(&self, entity: EntityRef) -> Option<EntityRef> {
        let target = self.native_entity_target(entity)?;
        self.entity(self.native_vehicle_id(target)?)
    }

    /// Whether the entity rides another object, even one without a handle.
    pub fn entity_is_passenger(&self, entity: EntityRef) -> Option<bool> {
        let target = self.native_entity_target(entity)?;
        let vehicle = self.native_vehicle_id(target);
        Some(vehicle.is_some())
    }

    /// Whether the HUD draws this entity's hearts while the local player rides it
    /// (`Entity.showVehicleHealth`, which is `instanceof LivingEntity`).
    pub fn entity_shows_vehicle_health(&self, entity: EntityRef) -> Option<bool> {
        Some(self.native_entity_target(entity)?.is_living())
    }

    /// The lowest vehicle under the entity (`Entity.getRootVehicle`): the
    /// entity itself when it rides nothing. Off-level intermediaries are traversed;
    /// an off-level root or a cycle returns no handle, never a descendant substitute.
    pub(in crate::game) fn entity_root_vehicle(&self, entity: EntityRef) -> Option<EntityRef> {
        let target = self.native_entity_target(entity)?;
        self.entity(self.native_root_vehicle_id(entity.id(), target)?)
    }

    /// `Entity.getPassengers`: resident handles in boarding order.
    pub fn entity_visit_passengers(
        &self,
        entity: EntityRef,
        visit: &mut dyn FnMut(EntityRef) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, EntityReadError> {
        self.native_server_entity_target(entity)?;
        let mut after = None;
        let result = std::iter::from_fn(|| {
            let id = self.next_passenger_seat(entity.id(), after)?;
            after = Some(id);
            Some(id)
        })
        .filter_map(|id| self.entity(id))
        .try_for_each(visit);
        Ok(result)
    }

    /// Whether any object rides the entity, even one without a handle.
    pub fn entity_is_vehicle(&self, entity: EntityRef) -> Option<bool> {
        self.native_entity_target(entity)?;
        Some(self.has_seated_passengers(entity.id()))
    }

    /// `Entity.getIndirectPassengers`: traverse retained riders, but lend only resident handles.
    pub(in crate::game) fn entity_visit_indirect_passengers(
        &self,
        entity: EntityRef,
        visit: &mut dyn FnMut(EntityRef) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, EntityReadError> {
        self.native_server_entity_target(entity)?;
        Ok(visit_passenger_tree(
            entity.id(),
            |vehicle, after| self.next_passenger_seat(vehicle, after),
            |rider| self.seated_passenger_vehicle(rider),
            |rider| {
                self.entity(rider)
                    .map_or(ControlFlow::Continue(()), &mut *visit)
            },
        ))
    }

    /// Follow full object links, returning no root for a cycle without allocating a visited set.
    pub(in crate::game) fn native_root_vehicle_id(&self, id: u32, target: Target) -> Option<u32> {
        let mut root = id;
        let mut next = self.native_vehicle_id(target);
        for _ in 0..=self.riding_link_limit() {
            let Some(vehicle) = next else {
                return Some(root);
            };
            root = vehicle;
            next = self.vehicle_of_any(vehicle);
        }
        None
    }

    /// The number of objects that could themselves have a mount, bounding a
    /// walk of valid links. Only mobs, general rows and players can be riders.
    pub(in crate::game) fn riding_link_limit(&self) -> usize {
        self.common.mobs.len()
            + self.common.persist.off_level.len()
            + self.common.resident_entities.general.rows.len()
            + self
                .server
                .lan
                .as_ref()
                .map_or(0, |host| host.hub.guests.len())
            + 1
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::boats_common::BoatsCommonGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
