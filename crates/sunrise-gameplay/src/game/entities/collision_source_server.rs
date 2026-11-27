//! Server-side entity solid-collision helpers split from common `collision_source.rs`.
use super::{
    collision_source::{Mover, boat, cart, cart_reach, player},
    commands_common::Target,
};
use sunrise_gameplay_common::game::transactions::entities::EntityRemovalGameExt as _;
use sunrise_sim::entity_solids::{Candidate, Mover as Rule, Side, SolidKind, Solids};

impl crate::game::ServerGame<'_> {
    /// Fills `out` with the entities that can be solid to a player as `Entity.canBeCollidedWith`
    /// allows (a boat, a living shulker, a happy ghast), as candidates the hub asks its rule of
    /// for each reported move. Built once per tick into the hub's own buffer, and left empty when
    /// no player is connected (`guests`): one test per mob and boat, no allocation once the buffer has grown.
    pub(in crate::game) fn fill_move_solids(&self, out: &mut Vec<Candidate>, guests: bool) {
        out.clear();
        if !guests {
            return;
        }
        let rule = Solids::new(
            Rule::Player {
                feet_y: 0.0,
                riding: None,
                side: Side::Server,
            },
            None,
        );
        out.extend(
            self.common
                .mobs
                .iter()
                .filter(|m| rule.takes(m.solid_kind()))
                .map(|m| m.solid_candidate()),
        );
        out.extend(
            self.common
                .boats
                .iter()
                .map(|b| boat(|id, p, bounds| self.client_copy_box(id, p, bounds), b)),
        );
    }

    /// [`Self::entity_collisions`] for an armor stand, which mobs can ride (`/ride`): each mob's
    /// box comes with the vehicle at the bottom of what it rides, so the stand leaves its own
    /// riders out (`isPassengerOfSameVehicle`, through the grid's `near_except`).
    pub(in crate::game) fn stand_entity_collisions(&self) -> sunrise_sim::entities::ObstacleGrid {
        let mut boxes = Vec::new();
        let mut roots = Vec::new();
        self.each_solid(Mover::Passive, |c, mob| {
            boxes.push(c.bounds);
            roots.push(mob.and_then(|j| c.mount.and_then(|_| self.mob_root_vehicle(j))));
        });
        sunrise_sim::entities::ObstacleGrid::with_roots(boxes, roots)
    }

    pub(in crate::game) fn server_boat_collision_root(
        &self,
        id: u32,
        target: Target,
    ) -> Option<u32> {
        self.native_root_vehicle_id(id, target)
    }

    pub(in crate::game) fn server_vehicle_solids(
        &self,
        mover: Mover<'_>,
        solids: &Solids,
        each: &mut dyn FnMut(&Candidate, Option<usize>),
    ) {
        match mover {
            Mover::Cart(c) => {
                for o in
                    self.common
                        .carts
                        .iter()
                        .filter(|o| o.id != c.id)
                        .map(|c| cart(|id, p, bounds| self.client_copy_box(id, p, bounds), c))
                        .chain(
                            self.common.boats.iter().map(|b| {
                                boat(|id, p, bounds| self.client_copy_box(id, p, bounds), b)
                            }),
                        )
                {
                    if solids.accepts(&o, || None) {
                        each(&o, None);
                    }
                }
                self.mobs_meeting(cart_reach(c), |j, m, _| {
                    let c = m.solid_candidate();
                    if solids.accepts(&c, || None) {
                        each(&c, Some(j));
                    }
                });
                if self.common.riding != Some(c.id)
                    && let Some(p) = self.common.player.as_ref()
                    && !self.common.spectator()
                {
                    let p = player(p.bounds());
                    if solids.accepts(&p, || None) {
                        each(&p, None);
                    }
                }
                if let Some(host) = self.server.lan.as_ref() {
                    for g in host.hub.guests.iter().filter(|g| {
                        host.hub.here(g)
                            && g.loaded
                            && !g.dead
                            && !g.spectator()
                            && g.vehicle != Some(c.id)
                    }) {
                        let p = player(g.bounds());
                        if solids.accepts(&p, || None) {
                            each(&p, None);
                        }
                    }
                }
            }
            Mover::Boat { id, area } => {
                self.mobs_meeting(area, |j, m, _| {
                    let c = m.solid_candidate();
                    let target = Target::Mob(m.id);
                    if solids.accepts(&c, || self.server_boat_collision_root(m.id, target))
                        && !self.target_removed(&target)
                    {
                        each(&c, Some(j));
                    }
                });
                for (c, target) in self
                    .common
                    .boats
                    .iter()
                    .map(|b| {
                        (
                            boat(|id, p, bounds| self.client_copy_box(id, p, bounds), b),
                            Target::Boat(b.id),
                        )
                    })
                    .chain(self.common.carts.iter().map(|c| {
                        (
                            cart(|id, p, bounds| self.client_copy_box(id, p, bounds), c),
                            Target::Cart(c.id),
                        )
                    }))
                    .chain(
                        self.players_outside(id)
                            .into_iter()
                            .map(|(bb, target, id)| {
                                (Candidate::fixed(SolidKind::Player, id, bb), target)
                            }),
                    )
                {
                    if solids.accepts(&c, || self.server_boat_collision_root(c.id, target))
                        && !self.target_removed(&target)
                    {
                        each(&c, None);
                    }
                }
            }
            Mover::Player | Mover::PlayerAt { .. } | Mover::Passive => {}
        }
    }
}

use sunrise_gameplay_common::game::collision_source::CollisionSourceGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
