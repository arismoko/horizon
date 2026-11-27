//! PURPOSE: the general entity rows' authoritative tick (#792 stage 4): one
//! pass over the rows in join order, each root's rider subtree inside its
//! tick.
//! WHY: vanilla `ServerLevel.tick` walks `entityTickList.forEach`, and
//! `tickNonPassenger` runs the entity's common tick, its tick, then each
//! rider recursively (`tickPassenger`: a rider is never a root). The tick
//! list is insertion-ordered, and its `ensureActiveIsNotIterated` keeps a
//! join made during the pass out of the pass, so Sunrise's join order
//! (`join_seq.rs`) is that order and a row that joins mid-pass first ticks
//! the next pass.
//! REFS: vanilla `ServerLevel.tick`, `tickNonPassenger`, `tickPassenger`,
//! `EntityTickList`; `join_seq.rs`; `ordered_tick.rs`; `tools/rea_vanilla.py`
//! (`add` appends to the active map, `remove` takes out, and the passive
//! swap holds a mid-pass join for the next tick).
//! LOGS: none. READS-BY: `Game::tick_world`. ENDS-WHEN: every kind ticks
//! from the join-order walk.

use super::super::native_ticks::TickRound;
use crate::game::ServerGame;
use mod_api::EntityRef;
use sunrise_gameplay_common::game::commands_common::Target;
use sunrise_gameplay_common::game::general::{GeneralTypeCatalog, general_type_catalog};
use sunrise_sim::join_seq::JoinSeq;

struct GeneralTickPass<'a> {
    born: JoinSeq,
    catalog: &'a GeneralTypeCatalog,
    subscribed: &'a [bool],
    epoch: Option<mod_api::WorldEpoch>,
    round: TickRound,
}

impl ServerGame<'_> {
    /// The general rows' entity phase: roots in join order, each root's
    /// bookkeeping, subscribed server behavior and the global tick pair,
    /// then its riders — every passenger gets the same Pre, tick and Post
    /// inside its own ride tick, at any depth of a stack, in the flat order
    /// the maintainers ruled: vehicle Pre, tick, Post, then each passenger
    /// (Pre, tick, Post), then that passenger's own passengers, depth
    /// first. A canceled vehicle Pre skips only the vehicle: no
    /// bookkeeping, no tick, no Post — its passengers still ride. A row
    /// that joined this tick ticks from the next (the tick list's passive
    /// swap), rows outside entity-ticking chunks stay (`ServerLevel.tick`'s
    /// range gate), and a parked level never reaches here at all. A type
    /// slot resolved without subscribers costs its bookkeeping and nothing
    /// else: no per-row type lookup, call, or event allocation.
    pub fn tick_general(&mut self) -> sunrise_game_data::Result<()> {
        if self.common.resident_entities.general.rows.is_empty() {
            return Ok(());
        }
        // `begin_ordered_ticks`' cutoff: a row numbered at or above `born`
        // joined during this tick and first ticks the next.
        let born = self.server.ordered.born;
        let catalog = general_type_catalog();
        // One bit per type slot, resolved once per pass. The rows keep plain
        // slots, so the walk itself never names a type until a part hears it.
        let subscribed = general_tick_subscribed(catalog);
        let entity_ticking = self.entity_ticking_area();
        let mut pass = GeneralTickPass {
            born,
            catalog,
            subscribed: &subscribed,
            epoch: self.common.native_lifecycle.epoch(),
            round: self.native_tick_round(),
        };
        // The walk stays on the live list, as the ordered run's does: a
        // lifted pass would take every row's residency slot, and a part's
        // writes reach its row through that slot. What joins during the pass
        // lands at or above `born`, ticks from the next pass, and never
        // reaches this loop's rows.
        let total = self.common.resident_entities.general.rows.len();
        // The rider adjacency, resolved once into the reusable first-child
        // and next-sibling index: a stable world's pass allocates nothing
        // (the index rebuilds in place only when the rows or a mount
        // changed), and every edge is visited exactly once, through its
        // vehicle's chain — no row scans the list.
        self.common.resident_entities.general.riders.refresh(
            &self.common.resident_entities.general.rows,
            self.common.resident_entities.general.mount_writes,
        );
        let index = &mut self.common.resident_entities.general.riders;
        index.ticked.clear();
        index.ticked.resize(total, false);

        // Walk wholly general branches from the roots first. If a branch
        // crosses a vanilla vehicle, its general riders follow after that
        // vehicle has moved and after any general ancestors have been seated.
        let mut foreign = std::mem::take(&mut index.foreign_parents);
        foreign.clear();
        for index in 0..total {
            let row = &self.common.resident_entities.general.rows[index];
            if row.join >= born {
                continue;
            }
            if let Some(vehicle) = row.mount {
                if self.common.general_row(vehicle).is_none() {
                    let mut depth = 1;
                    let mut parent = vehicle;
                    while depth <= self.riding_link_limit() {
                        let Some(next) = self.vehicle_of_any(parent) else {
                            break;
                        };
                        depth += 1;
                        parent = next;
                    }
                    foreign.push((depth, row.vehicle_order, index));
                }
            } else if entity_ticking(row.position.x.floor() as i32, row.position.z.floor() as i32) {
                self.tick_general_branch(index, None, &mut pass);
            }
        }
        foreign.sort_unstable();
        for &(_, _, index) in &foreign {
            let row = &self.common.resident_entities.general.rows[index];
            let Some(vehicle) = row.mount else { continue };
            if self.common.general_row(vehicle).is_some() {
                continue;
            }
            let root = self.native_root_vehicle_id(row.id, Target::General(row.id));
            if self.entity(vehicle).is_none() {
                if !self.common.persist.off_level.contains_key(&vehicle) {
                    self.dismount_general(row.id, vehicle);
                }
                continue;
            }
            if let Some([x, _, z]) =
                root.and_then(|root| self.entity(root).and_then(|e| self.entity_position(e)))
                && entity_ticking(x.floor() as i32, z.floor() as i32)
            {
                self.tick_general_branch(index, Some(vehicle), &mut pass);
            }
        }
        self.common.resident_entities.general.riders.foreign_parents = foreign;
        Ok(())
    }

    /// Visit one general branch once, without recursing on the mod's graph.
    fn tick_general_branch(
        &mut self,
        index: usize,
        mount: Option<u32>,
        pass: &mut GeneralTickPass<'_>,
    ) {
        let mut pending = std::mem::take(&mut self.common.resident_entities.general.riders.pending);
        pending.push((index, mount));
        while let Some((index, mount)) = pending.pop() {
            let Some(id) = self.tick_general_at(index, mount, pass) else {
                continue;
            };
            let from = pending.len();
            let riders = &self.common.resident_entities.general.riders;
            let mut child = riders.first_child[index];
            while let Some(index) = child {
                pending.push((index, Some(id)));
                child = riders.next_sibling[index];
            }
            pending[from..].reverse();
        }
        self.common.resident_entities.general.riders.pending = pending;
    }

    /// A joined row and its whole general branch, in boarding order. A
    /// canceled root skips its tick but not its passengers. On a passenger
    /// tick, cancellation still leaves its seat and bookkeeping intact.
    fn tick_general_at(
        &mut self,
        index: usize,
        mount: Option<u32>,
        pass: &mut GeneralTickPass<'_>,
    ) -> Option<u32> {
        let row = &self.common.resident_entities.general.rows[index];
        if row.join >= pass.born
            || self.common.resident_entities.general.riders.ticked[index]
            || !general_row_live(self, index, mount)
        {
            return None;
        }
        let (id, slot) = (row.id, row.type_slot);
        // A callback can dismount a passenger, but cannot make it tick again
        // if its new root/seat is reached later in this pass.
        self.common.resident_entities.general.riders.ticked[index] = true;
        let kind = pass.catalog.key(slot);
        let ticks = !pass.round.pre() || self.native_tick_pre_one(&mut pass.round, id, kind);
        if !general_row_live(self, index, mount) {
            return None;
        }
        if ticks || mount.is_some() {
            self.begin_general_row_tick(index, mount, pass.epoch);
            // `Entity.checkBelowWorld` discarded it: no tick for a removed row.
            if !general_row_live(self, index, mount) {
                return None;
            }
        }
        if ticks {
            if pass.subscribed.get(slot).copied().unwrap_or(false)
                && let Some(epoch) = pass.epoch
            {
                self.dispatch_general_tick(kind, epoch, id);
            }
            if pass.round.post() {
                self.native_tick_post_one(&pass.round, id, kind);
            }
        }
        if !general_row_live(self, index, mount) {
            return None;
        }
        if mount.is_some() {
            self.position_general_row(index);
        }
        if self
            .common
            .resident_entities
            .general
            .riders
            .mob_vehicles
            .contains(&id)
            && let Some(seat) = self.general_seat(id, false)
        {
            let mut at = 0;
            while let Some(mob) = self.common.mobs.get_mut(at) {
                if !mob.removed && mob.extra.mount == Some(id) {
                    seat.position_mob(mob);
                    let rider = mob.id;
                    self.snap_mob_passengers(rider);
                }
                at += 1;
            }
        }
        general_row_live(self, index, mount).then_some(id)
    }

    /// `Entity.commonTick` and `Entity.baseTick` before the type's tick (`GeneralRow::base_tick`),
    /// with the level's fluids; a row below the world is discarded (`Entity.checkBelowWorld`).
    fn begin_general_row_tick(
        &mut self,
        index: usize,
        mount: Option<u32>,
        epoch: Option<mod_api::WorldEpoch>,
    ) {
        let min_y = self.common.world.layout().min_y();
        let row = &mut self.common.resident_entities.general.rows[index];
        let id = row.id;
        let bounds = sunrise_gameplay_common::game::entity::Entity::bounds(row);
        let (_, height) = general_type_catalog().dimensions(row.type_slot);
        let alive = row.base_tick(
            bounds,
            height,
            mount.is_some(),
            min_y,
            |bounds, at, eye, velocity| {
                sunrise_sim::entities::scan_entity_fluids(
                    bounds,
                    at,
                    eye,
                    velocity,
                    &self.common.world,
                    &self.common.res.motion().fluids,
                    &self.common.loop_context,
                )
            },
        );
        if !alive {
            match epoch {
                Some(epoch) => {
                    self.remove_target_now(EntityRef::new(epoch, id), Target::General(id))
                }
                None => self.common.persist.removals.push(Target::General(id)),
            }
            return;
        }
        // General entities lack a hurt Part, so only the fire timer changes here.
        if row.base.fire_ticks > 0 {
            row.base.fire_ticks = if general_type_catalog().fire_immune(row.type_slot) {
                0
            } else {
                row.base.fire_ticks - 1
            };
        }
    }

    fn position_general_row(&mut self, index: usize) {
        let row = &self.common.resident_entities.general.rows[index];
        let Some(vehicle) = row.mount else { return };
        let at = self.general_passenger_position(vehicle, row.id);
        if let Some(at) = at {
            let row = &mut self.common.resident_entities.general.rows[index];
            let (id, before) = (row.id, row.position);
            row.position = at;
            row.fall_distance = 0.0;
            self.entity_section_step(id, [before.x, before.y, before.z], [at.x, at.y, at.z]);
        }
    }

    /// One row's subscribed behavior, contained: the dispatch runs in a
    /// transaction. A panic aborts the row's writes without unbalancing the pass.
    fn dispatch_general_tick(&mut self, kind: &str, epoch: mod_api::WorldEpoch, id: u32) {
        let failures = mod_api::runtime::isolated_failures();
        let _ = self.storage_transaction(|game, _| {
            game.tick_general_row_scoped(kind, EntityRef::new(epoch, id));
            if mod_api::runtime::isolated_failures() == failures {
                Ok(())
            } else {
                Err(())
            }
        });
    }
}

/// Whether each type slot has a part subscribed to the tick hook: nothing
/// at all when no part anywhere subscribes (the dispatch's own bit check),
/// so a parts-free world resolves nothing.
fn general_tick_subscribed(catalog: &GeneralTypeCatalog) -> Vec<bool> {
    if !mod_api::__private::has_part_hook(mod_api::HookId::GeneralEntityTick) {
        return Vec::new();
    }
    (0..catalog.len())
        .map(|slot| {
            mod_api::__private::entity_part_hooked(
                mod_api::HookId::GeneralEntityTick,
                "general_entity_tick",
                catalog.key(slot),
            )
        })
        .collect()
}

/// Whether the row at `index` is a live row still riding `mount`: re-read at
/// every step of the walk, because a listener's callback can queue the row's
/// removal — `entity_remove` leaves it in the list until the sweep — or
/// rewrite its mount. The row's identity is its place: nothing in the pass
/// reorders the storage, so an index read now is the row it names.
fn general_row_live(game: &crate::game::ServerGame<'_>, index: usize, mount: Option<u32>) -> bool {
    game.resident_entities
        .general
        .rows
        .get(index)
        .is_some_and(|row| {
            row.mount == mount && !game.persist.removals.contains(&Target::General(row.id))
        })
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_ticks::NativeTicksGameExt1 as _;

use sunrise_gameplay_common::game::passengers_common::PassengersCommonGameExt1 as _;
