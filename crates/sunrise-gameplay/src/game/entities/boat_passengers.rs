//! Boats and the entities around them: what a boat runs into
//! (`AbstractBoat.canVehicleCollide`), the pushes between a boat and what
//! touches it (`AbstractBoat.push`, `LivingEntity.pushEntities`), the mobs
//! it picks up (the end of `AbstractBoat.tick`), where they sit
//! (`positionRider`, `clampRotation`) and how they get out.
//! Saving them under the boat's `Passengers` lives in `persist.rs`.
#[allow(unused_imports)]
use super::lan_mobs::guest_bounds;
use std::cell::{Cell, Ref, RefCell};
use sunrise_core::geometry::{Aabb, Vec3};
#[allow(unused_imports)]
use sunrise_game_data::scoreboard::pushable_by;
use sunrise_gameplay_common::game::{
    boats_common::{PLAYER_RIDING_OFFSET, boat_pushes, push_impulse},
    commands_common::Target,
};
#[allow(unused_imports)]
use sunrise_level::fast_hash::FxHashMap;
#[allow(unused_imports)]
use sunrise_sim::{
    join_seq::{JoinList, ListStamp},
    mobs::{BoxReach, Mob, Species, Who, riders, seen_grid::SeenGrid},
};

/// `Entity.boardingCooldown` once a mob leaves its boat.
pub const BOARDING_COOLDOWN: i32 = 60;
/// How many box queries the walk over every mob answers, once a write to the mob list has
/// made the grid stale, before a new grid is built: about what building one costs (with
/// 25,000 mobs, six to eight walks). It doubles with each grid of a stretch, so vehicles
/// that each write a mob (a boat pushing the pig beside it) never pay more for grids than
/// for their walks.
const WALKS_BEFORE_GRID: u32 = 8;

/// What a stretch of vehicle ticks knows about the mobs, so a boat or a minecart reads the
/// mobs near it and not every mob of the level ([`Game::mobs_meeting`]).
#[derive(Default)]
pub struct VehicleMobs {
    /// The mobs' [`BoxReach`] with the mob count it was worked out for; None outside a stretch.
    reach: Cell<Option<(usize, BoxReach)>>,
    pub grid: RefCell<MobGrid>,
    /// The last query's candidates, kept for their allocation.
    near: Cell<Vec<u32>>,
    /// Each ridden vehicle's mobs by slot, for the mob list as it stood ([`Game::resident_mob_riders`]).
    riders: RefCell<Option<(ListStamp, FxHashMap<u32, Vec<usize>>)>>,
}
#[derive(Default)]
pub struct MobGrid {
    /// The mobs' boxes by column and the list they were read from: they hold until that
    /// list's next write ([`ListStamp`]), which every push, pick-up, death and join is.
    pub built: Option<(ListStamp, SeenGrid)>,
    /// The queries walked since the grid went stale, and how many are before the next grid.
    walked: u32,
    patience: u32,
}

/// `Entity.canRide` for a mob species: `Warden` and `WitherBoss` override it
/// to return false, so no vehicle ever takes them aboard.
pub(in crate::game::server_side) fn species_can_ride(species: Species) -> bool {
    !matches!(species, Species::Warden | Species::Wither)
}

/// `Entity.isVehicle`: a player or a mob rides it.
fn carries(m: &Mob) -> bool {
    m.rider.is_some() || m.extra.rider.is_some()
}
/// `isPushable`, as `pushableBy` and `canVehicleCollide` ask it:
/// `Mob::pushable` plus the shulkers it leaves out (they take no push, but
/// count).
fn pushable(m: &Mob) -> bool {
    !m.removed && (m.pushable() || m.species() == Species::Shulker && m.alive())
}
/// Whether `Entity.push` moves the mob: it is pushable and carries no one.
fn moved_by_push(m: &Mob) -> bool {
    !m.removed && m.pushable() && !carries(m)
}
impl crate::game::ServerGame<'_> {
    /// Starts or ends a stretch of vehicle ticks (`tick_boats`, `tick_carts`): while one runs,
    /// a vehicle's box query ([`Self::mobs_meeting`]) reads the mobs near it from a grid of
    /// their boxes, or tests the mobs' positions against their [`BoxReach`] first. A mob
    /// joining or leaving works the reach out again, and so does [`Self::vehicle_picked_up`].
    /// The first query that needs one works it out (the reach's count is one no list has), so
    /// a level without boats or carts never walks its mobs for them.
    pub fn vehicles_ticking(&self, on: bool) {
        let reach = (on && mod_api::__private::mods().is_empty())
            .then(|| (usize::MAX, BoxReach::default()));
        self.server.vehicle_mobs.reach.set(reach);
        // The boats' grid serves the carts ticking right after them; an older one goes.
        let mut grid = self.server.vehicle_mobs.grid.borrow_mut();
        let stamp = self.common.mobs.stamp();
        let built = grid.built.take().filter(|(s, _)| *s == stamp);
        *grid = MobGrid {
            built,
            ..Default::default()
        };
    }
    /// A vehicle picked a mob up: a shulker turns its lid up (`Shulker.startRiding`), which
    /// changes its box, so the next query works the [`BoxReach`] out again.
    pub(in crate::game::server_side) fn vehicle_picked_up(&self) {
        if let Some((_, reach)) = self.server.vehicle_mobs.reach.get() {
            self.server
                .vehicle_mobs
                .reach
                .set(Some((usize::MAX, reach)));
        }
    }
    /// The mobs' [`BoxReach`] while vehicles tick, none otherwise.
    pub(in crate::game) fn vehicle_mob_reach(&self) -> Option<BoxReach> {
        let (count, reach) = self.server.vehicle_mobs.reach.get()?;
        if count == self.common.mobs.len() {
            return Some(reach);
        }
        let reach = BoxReach::of(&self.common.mobs);
        self.server
            .vehicle_mobs
            .reach
            .set(Some((self.common.mobs.len(), reach)));
        Some(reach)
    }
    /// The slots, ascending, of the mobs whose box may meet `area` into `out` (a superset: the
    /// caller still tests each box), from a grid of the mobs' boxes that the first query of a
    /// stretch of vehicle ticks builds. False: walk them all. That is outside a stretch or with
    /// native mods loaded ([`VehicleMobs::vehicle_mobs_indexed`]), when the grid cannot say, and for
    /// a while after a write to the mob list ([`MobGrid`]).
    fn vehicle_mobs_near(&self, area: Aabb, out: &mut Vec<u32>) -> bool {
        if !self.server.vehicle_mobs.vehicle_mobs_indexed() {
            return false;
        }
        let stamp = self.common.mobs.stamp();
        let mut grid = self.server.vehicle_mobs.grid.borrow_mut();
        if grid.built.as_ref().is_none_or(|(s, _)| *s != stamp) {
            if grid.walked < grid.patience {
                grid.walked += 1;
                return false;
            }
            grid.built = Some((
                stamp,
                SeenGrid::new(self.common.mobs.iter().map(Mob::bounds)),
            ));
            grid.walked = 0;
            grid.patience = (grid.patience * 2).max(WALKS_BEFORE_GRID);
        }
        let built = grid.built.as_ref();
        built.is_some_and(|(_, cells)| cells.candidates(area, out))
    }
    /// A vehicle's box query: `each` gets every mob whose box meets `area`, in slot order,
    /// with its slot and its box. While vehicles tick it reads the mobs near the area
    /// ([`Self::vehicle_mobs_near`]) or, walking them all, tests each position against the
    /// [`BoxReach`] before working the box out. The mobs and their order are those of a walk
    /// that tests every box.
    pub(in crate::game) fn mobs_meeting(
        &self,
        area: Aabb,
        mut each: impl FnMut(usize, &Mob, Aabb),
    ) {
        let mut near = self.server.vehicle_mobs.near.take();
        if self.vehicle_mobs_near(area, &mut near) {
            for &j in &near {
                let m = &self.common.mobs[j as usize];
                let b = m.bounds();
                if b.intersects(area) {
                    each(j as usize, m, b);
                }
            }
        } else {
            let reach = self.vehicle_mob_reach();
            for (j, m) in self.common.mobs.iter().enumerate() {
                if reach.is_some_and(|r| !r.may_meet(m, area)) {
                    continue;
                }
                let b = m.bounds();
                if b.intersects(area) {
                    each(j, m, b);
                }
            }
        }
        self.server.vehicle_mobs.near.set(near);
    }
    /// Whether mob `m`, whose box `reach` bounds, may touch a cart: the carts' grid (`cells`)
    /// names the carts near it, or every cart when the grid cannot (a superset either way).
    pub(in crate::game::server_side) fn mob_may_meet_cart(
        &self,
        reach: BoxReach,
        m: &Mob,
        cells: &SeenGrid,
        near: &mut Vec<u32>,
    ) -> bool {
        if !cells.candidates(reach.around(m.position), near) {
            near.clear();
            near.extend(0..self.common.carts.len() as u32);
        }
        near.iter()
            .any(|&k| reach.may_meet(m, self.common.carts[k as usize].bounds()))
    }
    /// The players in boat `id`: the window's player (or the server player
    /// acting as it) and the server's other players.
    pub(in crate::game) fn boat_players(&self, id: u32) -> usize {
        let guests = self.server.lan.as_ref().map_or(0, |h| {
            h.hub
                .guests
                .iter()
                .filter(|g| Some(g.conn) != self.server.lan_acting && g.vehicle == Some(id))
                .count()
        });
        usize::from(self.common.riding == Some(id)) + guests
    }
    /// The mobs in boat `id`, in their seat order after the players
    /// (`Entity.addPassenger` puts a player first).
    pub fn boat_mobs(&self, id: u32) -> Vec<usize> {
        self.resident_mob_riders(id)
            .filter_map(|(slot, mob)| (!mob.removed && mob.alive()).then_some(slot))
            .collect()
    }
    /// `AbstractBoat.canAddPassenger`: a seat is free and the boat is not
    /// under water.
    pub(in crate::game) fn server_boat_has_room(&self, i: usize) -> bool {
        let b = &self.common.boats[i];
        self.seated_passenger_count(b.id) < b.max_passengers() && !b.under_water()
    }
    /// Where the player in boat `i` sits: in the middle alone, in front of a
    /// mob.
    pub(in crate::game) fn server_player_boat_seat(&self, i: usize) -> Vec3 {
        let b = &self.common.boats[i];
        let players = self.boat_players(b.id);
        // The seated list, with the rider counted before its own link exists.
        let count = self.passenger_order(b.id).len() + usize::from(players == 0);
        b.seat(0, count, false, PLAYER_RIDING_OFFSET, false)
    }
    /// `getRootVehicle` of mob `j`: the mob or boat at the bottom of what it
    /// rides, None when it rides nothing.
    pub(in crate::game) fn mob_root_vehicle(&self, mut j: usize) -> Option<u32> {
        let mut root = None;
        for _ in 0..8 {
            let Some(v) = self.common.mobs[j].extra.mount else {
                break;
            };
            root = Some(v);
            match self.common.mobs.iter().position(|m| m.id == v) {
                Some(k) => j = k,
                None => break,
            }
        }
        root
    }
    /// The players outside boat `id` that push and block it (living, not
    /// spectators), with the target and entity ID used by the riding graph.
    pub(in crate::game) fn players_outside(&self, id: u32) -> Vec<(Aabb, Target, u32)> {
        let mut out = Vec::new();
        if let Some(p) = self.common.player.as_ref()
            && self.common.riding != Some(id)
            && !self.common.spectator()
            && !p.health.dead_or_dying()
        {
            out.push((p.bounds(), Target::Player, self.own_entity_id() as u32));
        }
        if let Some(host) = self.server.lan.as_ref() {
            for g in host.hub.guests.iter().filter(|g| {
                Some(g.conn) != self.server.lan_acting
                    && host.hub.here(g)
                    && g.loaded
                    && !g.dead
                    && !g.spectator()
                    && g.vehicle != Some(id)
            }) {
                out.push((guest_bounds(g), Target::Guest(g.conn), g.entity_id as u32));
            }
        }
        out
    }

    /// The end of `AbstractBoat.tick` on the server for boat `i`: each
    /// entity it touches gets in (a mob, while no player steers) or is
    /// pushed apart from it, and `LivingEntity.pushEntities` of each mob and
    /// player touching it pushes the two apart once more. Only the window's
    /// own player is moved here: a server's players are pushed by their
    /// clients, which tick the boats near them (`remote_pushes`).
    pub fn boat_contacts(&mut self, mut i: usize) {
        let (id, bb, max) = {
            let b = &self.common.boats[i];
            (b.id, b.bounds(), b.max_passengers())
        };
        let area = bb.inflate(Vec3::new(
            f64::from(0.2_f32),
            f64::from(-0.01_f32),
            f64::from(0.2_f32),
        ));
        let mut count = self.seated_passenger_count(id);
        // getControllingPassenger is a player: one rides it (players sit first).
        let pick_up = self.boat_players(id) == 0;
        // canAddPassenger, which startRiding asks after the checks below.
        let room = !self.common.boats[i].under_water();
        // The mobs near the two boxes: the teams are only worked out when something
        // touches the boat.
        let mut near = Vec::new();
        self.mobs_meeting(area.union_bounds(bb), |j, _, mb| {
            if mb.intersects(area) || mb.intersects(bb) {
                near.push(j);
            }
        });
        let outside = self.players_outside(id);
        let touching = !near.is_empty()
            || outside
                .iter()
                .any(|(pb, _, _)| pb.intersects(area) || pb.intersects(bb));
        // (mob, touched by the boat's sweep, its own pushEntities reaches the boat)
        let teams = if touching {
            self.mob_teams()
        } else {
            Default::default()
        };
        let mobs: Vec<(u32, bool, bool)> = near
            .into_iter()
            .filter_map(|j| {
                let m = &self.common.mobs[j];
                if !pushable(m) || m.extra.mount.is_some() && self.mob_root_vehicle(j) == Some(id) {
                    return None;
                }
                let mb = m.bounds();
                let team = teams.of(Who::Mob(m.id));
                let touched = mb.intersects(area) && pushable_by(None, team, true);
                // Bats push nothing (Bat.pushEntities).
                let pushes = m.species() != Species::Bat
                    && mb.intersects(bb)
                    && pushable_by(team, None, true);
                (touched || pushes).then_some((m.id, touched, pushes))
            })
            .collect();
        let player_team = teams.of(Who::Player);
        let players: Vec<(Aabb, bool, bool, bool)> = outside
            .into_iter()
            .map(|(pb, target, _)| {
                let own = target == Target::Player;
                let team = if own { player_team } else { None };
                (
                    pb,
                    own,
                    pb.intersects(area) && pushable_by(None, team, true),
                    pb.intersects(bb) && pushable_by(team, None, true),
                )
            })
            .collect();
        drop(teams);
        for (mob, touched, pushes) in mobs {
            // By id: a mod's mount hook may have removed mobs since the list
            // was made, which moves the others' indices.
            let Some(j) = self.common.mobs.iter().position(|m| m.id == mob) else {
                continue;
            };
            if touched {
                let m = &self.common.mobs[j];
                let boards = pick_up
                    && count < max
                    && m.extra.mount.is_none()
                    && m.width() < sunrise_gameplay_common::game::boats_common::WIDTH as f32
                    && !self.items().entity_type_tags.has_bit(
                        m.kind.flags.entity_tag_slot,
                        sunrise_items::items::EntityTypeTags::CANNOT_BE_PUSHED_ONTO_BOATS,
                    );
                if boards {
                    // startRiding fails quietly (no push) while the mob's
                    // boarding cooldown runs (Entity.canRide) or the boat
                    // is under water, or when a mod cancels it
                    // (EntityMountEvent).
                    let ready = m.extra.boarding_cooldown <= 0 && species_can_ride(m.species());
                    if room && ready && self.native_mob_may_ride(mob, id, true) {
                        // The listener had the game in its hands: find the
                        // boat again by id (it is gone: this boat's pass
                        // ends), count its passengers again, and board the
                        // mob only if it is still there, not removed and not
                        // riding.
                        let Some(now) = self.common.boats.iter().position(|b| b.id == id) else {
                            return;
                        };
                        i = now;
                        count = self.seated_passenger_count(id);
                        if count < self.common.boats[i].max_passengers()
                            && let Some(m) = self
                                .common
                                .mobs
                                .iter_mut()
                                .find(|m| m.id == mob && !m.removed && m.extra.mount.is_none())
                        {
                            sunrise_sim::mobs::shulker::on_start_riding(m);
                            m.extra.mount = Some(id);
                            m.extra.passenger = true;
                            m.ai.nav.stop();
                            count += 1;
                            self.seat_mob(mob);
                            // Mob.startRiding drops its lead.
                            self.break_leash(mob, true, false);
                            self.note_started_riding(id);
                            self.vehicle_picked_up();
                        }
                    }
                    continue;
                }
                self.boat_push_mob(i, j, count);
            }
            if pushes {
                self.boat_push_mob(i, j, count);
            }
        }
        for (pb, own, touched, pushes) in players {
            let Some(v) = push_impulse(self.common.boats[i].position, pb.center())
                .filter(|_| boat_pushes(bb, pb, false))
            else {
                continue;
            };
            for _ in 0..usize::from(touched) + usize::from(pushes) {
                if count == 0 {
                    self.common
                        .boats
                        .update(i, |b| b.velocity = b.velocity.subtract(v));
                }
                if own && let Some(p) = self.common.player.as_mut() {
                    p.velocity = p.velocity.add_vector(v);
                }
            }
        }
        for k in 0..self.common.boats.len() {
            let ob = self.common.boats[k].bounds();
            if k == i || !ob.intersects(area) || !boat_pushes(bb, ob, true) {
                continue;
            }
            let Some(v) =
                push_impulse(self.common.boats[i].position, self.common.boats[k].position)
            else {
                continue;
            };
            if count == 0 {
                self.common
                    .boats
                    .update(i, |b| b.velocity = b.velocity.subtract(v));
            }
            if !self.has_seated_passengers(self.common.boats[k].id) {
                self.common
                    .boats
                    .update(k, |b| b.velocity = b.velocity.add_vector(v));
            }
        }
        for c in 0..self.common.carts.len() {
            let cb = self.common.carts[c].bounds();
            if !cb.intersects(area) || !boat_pushes(bb, cb, false) {
                continue;
            }
            let Some(v) =
                push_impulse(self.common.boats[i].position, self.common.carts[c].position)
            else {
                continue;
            };
            if count == 0 {
                self.common
                    .boats
                    .update(i, |b| b.velocity = b.velocity.subtract(v));
            }
            if !self.has_seated_passengers(self.common.carts[c].id) {
                self.common
                    .carts
                    .update(c, |c| c.velocity = c.velocity.add_vector(v));
            }
        }
    }
    /// `AbstractBoat.push(mob)` from boat `i`, which has `count` passengers.
    fn boat_push_mob(&mut self, i: usize, j: usize, count: usize) {
        let (b, m) = (&self.common.boats[i], &self.common.mobs[j]);
        if !boat_pushes(b.bounds(), m.bounds(), false) {
            return;
        }
        let Some(v) = push_impulse(b.position, m.position) else {
            return;
        };
        if count == 0 {
            self.common
                .boats
                .update(i, |b| b.velocity = b.velocity.subtract(v));
        }
        let m = &mut self.common.mobs[j];
        if moved_by_push(m) {
            m.velocity = m.velocity.add_vector(v);
            m.needs_sync = true;
        }
    }

    /// `positionRider` and `clampRotation` for the mobs in boats: each sits
    /// in its seat facing the way the boat does, an animal in a full boat
    /// sideways, its head kept within its reach of its body.
    pub(in crate::game) fn seat_boat_mobs(
        &mut self,
        riding: &sunrise_gameplay_common::game::passengers_common::Riders,
    ) {
        self.seat_boat_mobs_with_order(riding, |game, id| game.passenger_order(id));
    }
    /// `Entity.setRemoved` for a killed boat or cart; false on a reentrant
    /// removal of the same object while its dismount callbacks are running.
    pub fn kill_vehicle_object(&mut self, target: Target) -> bool {
        let id = match target {
            Target::Boat(id) | Target::Cart(id) => id,
            _ => unreachable!("only boat and cart removals use this helper"),
        };
        self.with_vehicle_removal_effects(
            target,
            |game| game.prepare_vehicle_removal(target, id),
            |game| game.take_destroyed_vehicle_row(target, id),
        )
        .is_some()
    }

    /// The physical sweep of an already logically removed boat or cart.
    pub(in crate::game::server_side) fn take_destroyed_vehicle_row(
        &mut self,
        target: Target,
        id: u32,
    ) {
        use sunrise_gameplay_common::game::entity_memory::RemovedVehicle;
        let vehicle = match target {
            Target::Boat(_) => self
                .common
                .boats
                .iter()
                .position(|b| b.id == id)
                .map(|i| RemovedVehicle::Boat(self.common.boats.remove(i))),
            Target::Cart(_) => self
                .common
                .carts
                .iter()
                .position(|c| c.id == id)
                .map(|i| RemovedVehicle::Cart(self.common.carts.remove(i))),
            _ => unreachable!(),
        };
        if let Some(vehicle) = vehicle {
            self.retain_removed_vehicle(id, vehicle);
        }
    }

    /// Container `remove` spills before `Entity.setRemoved`; `AbstractBoat.remove`
    /// then releases the lead. Keep the emptied owner in place for the dismount.
    pub(in crate::game::server_side) fn prepare_vehicle_removal(
        &mut self,
        target: Target,
        id: u32,
    ) {
        use sunrise_gameplay_common::game::container_loot_level::LootTarget;
        self.common.release_vehicle_container(id);
        let loot = match target {
            Target::Boat(_) => LootTarget::Boat(id),
            Target::Cart(_) => LootTarget::Cart(id),
            _ => unreachable!("only boats and carts have vehicle containers"),
        };
        self.unpack_loot(loot, None);
        let take = |at: Vec3, items: &mut [Option<sunrise_items::items::ItemStack>]| {
            (
                at.components(),
                items.iter_mut().map(Option::take).collect::<Vec<_>>(),
            )
        };
        let slots = match target {
            Target::Boat(_) => self.common.boats.iter().position(|b| b.id == id).map(|i| {
                self.common
                    .boats
                    .update(i, |b| take(b.position, &mut b.items))
            }),
            _ => self.common.carts.iter().position(|c| c.id == id).map(|i| {
                self.common
                    .carts
                    .update(i, |c| take(c.position, &mut c.items))
            }),
        };
        if let Some((at, items)) = slots {
            self.drop_vehicle_contents(at, &items);
        }
        if matches!(target, Target::Boat(_))
            && let Some(i) = self.common.boats.iter().position(|b| b.id == id)
        {
            self.drop_removed_boat_lead(i);
        }
    }

    /// `ejectPassengers` when a submerged boat is no longer rideable.
    pub(in crate::game::server_side) fn eject_boat_mobs(&mut self, i: usize) {
        let b = &self.common.boats[i];
        let (id, y) = (b.id, b.position.y);
        for rider in self.mob_passenger_ids(id) {
            if self
                .common
                .mob_object(rider)
                .is_none_or(|m| m.removed || m.extra.mount != Some(id))
            {
                continue;
            }
            if !self.native_mob_may_ride(rider, id, false) {
                continue;
            }
            let Some(m) = self
                .common
                .mob_object(rider)
                .filter(|m| !m.removed && m.extra.mount == Some(id))
            else {
                continue;
            };
            let at = if let Some(boat) = self.common.boats.iter().find(|b| b.id == id) {
                self.boat_dismount_target(boat, f64::from(m.width()), m.yaw)
            } else {
                self.common.removed_vehicle_dismount_point(
                    m.position,
                    f64::from(m.width()),
                    f64::from(m.height()),
                    y,
                )
            };
            let m = self.mob_object_mut(rider).unwrap();
            m.extra.mount = None;
            m.extra.passenger = false;
            m.extra.boarding_cooldown = BOARDING_COOLDOWN;
            sunrise_sim::mobs::living::teleports::snap_to(m, at);
            self.snap_mob_passengers(rider);
        }
        self.dismount_general_passengers(id);
    }
}

impl crate::game::ServerGame<'_> {
    /// Resident mob riders in slot order, including those whose removal is pending.
    /// During an indexed vehicle pass, all lookups share the same slots until
    /// the mob list is next written.
    pub(in crate::game) fn resident_mob_riders(
        &self,
        id: u32,
    ) -> impl Iterator<Item = (usize, &Mob)> {
        let slots = <crate::game::server_side::ServerState as sunrise_gameplay_common::game::side_ops::mobs_rest::SideOps>::side_indexed_mob_rider_slots(&self.server, &self.common.mobs, id);
        let mut at = 0;
        let mut scan = self.common.mobs.iter().enumerate();
        std::iter::from_fn(move || {
            if let Some(slots) = &slots {
                let &slot = slots.get(at)?;
                at += 1;
                Some((slot, &self.common.mobs[slot]))
            } else {
                scan.find(|(_, mob)| mob.extra.mount == Some(id))
            }
        })
    }
}

impl VehicleMobs {
    /// Whether what a stretch of vehicle ticks reads from the mob list may stand in for a walk
    /// over it: a stretch runs, and no native mod is loaded (a hook can move a mob unseen, as
    /// for the carts' `CartIndex`).
    pub(in crate::game) fn vehicle_mobs_indexed(&self) -> bool {
        self.reach.get().is_some() && mod_api::__private::mods().is_empty()
    }

    pub(in crate::game) fn indexed_mob_rider_slots<'a>(
        &'a self,
        mobs: &JoinList<Mob>,
        id: u32,
    ) -> Option<Ref<'a, [usize]>> {
        if !self.vehicle_mobs_indexed() {
            return None;
        }
        let stamp = mobs.stamp();
        let cache = &self.riders;
        if cache.borrow().as_ref().is_none_or(|(s, _)| *s != stamp) {
            let mut by_vehicle = FxHashMap::<u32, Vec<usize>>::default();
            for (slot, mob) in mobs.iter().enumerate() {
                if let Some(vehicle) = mob.extra.mount {
                    by_vehicle.entry(vehicle).or_default().push(slot);
                }
            }
            cache.replace(Some((stamp, by_vehicle)));
        }
        Some(Ref::map(cache.borrow(), |index| {
            index
                .as_ref()
                .and_then(|(_, by_vehicle)| by_vehicle.get(&id))
                .map_or(&[][..], Vec::as_slice)
        }))
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

#[cfg(test)]
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(test)]
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt6 as _;

use sunrise_gameplay_common::game::passengers_common::PassengersCommonGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_is_capped_and_scaled() {
        // One block apart on x: the full 0.05 along x.
        let v = push_impulse(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0)).unwrap();
        assert!((v.x - f64::from(0.05_f32)).abs() < 1e-12 && v.z == 0.0);
        // Almost on the same spot: no push.
        assert!(push_impulse(Vec3::ZERO, Vec3::new(0.005, 0.0, 0.0)).is_none());
    }

    #[test]
    fn warden_and_wither_never_board() {
        assert!(!species_can_ride(Species::Warden));
        assert!(!species_can_ride(Species::Wither));
        assert!(species_can_ride(Species::Cow));
    }

    #[test]
    #[ignore = "daily"]
    fn a_mod_growing_a_mob_during_vehicle_ticks_keeps_it_in_collision_queries() {
        use super::super::native_entity_refs::tests::with_game;

        struct Resize;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Resize {}

        with_game(|game| {
            super::super::native_entity_refs::tests::fixture_resident_column(
                &mut game.world,
                (0, 0),
            );
            let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            let mut cow = Mob::new(77, kind, Vec3::ZERO, 0.0);
            sunrise_sim::mobs::finalize::set_baby(&mut cow, true);
            game.mobs.push(cow);
            let mob = game.mob(game.entity(77).unwrap()).unwrap();
            let area = Aabb::new(Vec3::new(0.3, 0.0, 0.0), Vec3::new(0.4, 1.0, 0.1));
            assert!(!game.mobs[0].bounds().intersects(area));
            let mods = vec![mod_api::ModEntry::new("resize-test", Box::new(Resize))].into();
            mod_platform::testing::with_mods(mods, || {
                game.vehicles_ticking(true);
                assert!(mob.set_baby(game, false));
                let cow = &game.mobs[0];
                assert!(cow.bounds().intersects(area));
                assert!(
                    game.vehicle_mob_reach()
                        .is_none_or(|reach| reach.may_meet(cow, area))
                );
                game.vehicles_ticking(false);
            });
        });
    }

    #[test]
    fn boats_push_by_height() {
        let boat = Aabb::new(Vec3::new(0.0, 64.0, 0.0), Vec3::new(1.0, 64.5625, 1.0));
        let at = |y: f64| Aabb::new(Vec3::new(0.0, y, 0.0), Vec3::new(1.0, y + 1.0, 1.0));
        assert!(boat_pushes(boat, at(64.0), false));
        assert!(!boat_pushes(boat, at(64.1), false));
        assert!(boat_pushes(boat, at(64.5), true));
        assert!(!boat_pushes(boat, at(64.5625), true));
    }
}
