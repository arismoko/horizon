//! The move check of a player a moving collider carries
//! (docs/modding/src/how-to/entities/collision-exclusions.md, "Moving colliders").
//!
//! A client moves its player with the collider as it last heard of it, so the position it reports
//! is in a frame that is some ticks old. The server keeps each collider's recent transforms
//! (`moving_colliders::History`) and judges a report in the world as the client saw it: the
//! collider placed at the age the report fits (`sunrise_core::geometry::carried`), the move from
//! the last accepted position, held in the collider's frame, run through the same speed test and
//! collision check a walking player gets. What the server then keeps is that local position
//! under the collider's transform now, after the world's collision: the frame the client held
//! never enters the server's state. A report that fits no age in the window is corrected to
//! where the server last had the player on the collider, placed under the collider's transform
//! now. A correction is counted, and never counts toward the floating kick.
use super::{Hub, TickCtx};
use std::sync::atomic::Ordering;
use sunrise_core::geometry::{
    Aabb, Vec3,
    carried::{Ages, Body, Carried, Fit, Footing, footing, judge},
};
use sunrise_world::moving_colliders::{as_mover, at_age, with_history};

/// How a report stands to the colliders of the level.
pub(super) enum Plan {
    /// It fits a collider at an age.
    Fits {
        collider: u32,
        attached: bool,
        fit: Fit,
    },
    /// The player was on a collider and the report is on none: it has left, or is cheating.
    Left { age: f64 },
    /// A collider has been near, but the player is not on one: judged in the world the
    /// client saw.
    Near { age: f64 },
}

impl Hub {
    /// Where guest `i`'s report at `to` stands to the level's colliders; `None` when none matters.
    pub(super) fn carry_plan(&self, i: usize, to: [f64; 3]) -> Option<Plan> {
        let g = &self.guests[i];
        if g.vehicle.is_some() || g.spectator() || g.sleeping {
            return None;
        }
        let dimensions = g.dimensions();
        let body = Body {
            half_width: f64::from(dimensions.width) / 2.0,
            height: f64::from(dimensions.height),
        };
        let feet = Vec3::new(to[0], to[1], to[2]);
        let round_trip = g.latency.load(Ordering::Relaxed);
        let ages = |collider: u32| {
            let hint = g
                .carry_hint
                .filter(|(held, _)| *held == collider)
                .map(|(_, ticks)| ticks);
            Ages::new(round_trip, hint)
        };
        with_history(|history| {
            if let Some(held) = g.on_collider {
                let (log, boxes) = history.get(held.collider)?;
                return Some(
                    match judge(
                        log,
                        history.now(),
                        boxes,
                        feet,
                        body,
                        ages(held.collider),
                        false,
                    ) {
                        Some(fit) => Plan::Fits {
                            collider: held.collider,
                            attached: true,
                            fit,
                        },
                        None => Plan::Left { age: held.age },
                    },
                );
            }
            let around = Aabb::new(
                Vec3::new(feet.x - 1.0, feet.y - 0.5, feet.z - 1.0),
                Vec3::new(feet.x + 1.0, feet.y + body.height + 0.5, feet.z + 1.0),
            );
            let mut found = None;
            let mut any = false;
            history.near(around, |collider| {
                any = true;
                let Some((log, boxes)) = history.get(collider) else {
                    return false;
                };
                found = judge(log, history.now(), boxes, feet, body, ages(collider), true)
                    .map(|fit| (collider, fit));
                found.is_some()
            });
            if !any {
                return None;
            }
            if let Some((collider, fit)) = found {
                return Some(Plan::Fits {
                    collider,
                    attached: false,
                    fit,
                });
            }
            Some(Plan::Near {
                age: Ages::new(round_trip, None).anchor,
            })
        })
        .flatten()
    }

    /// The move the speed test (`moved too quickly`) measures for a report that a plan judges, in
    /// the frame the move is made in; `None` for the ordinary one, from where the tick began. A
    /// carried player's is the local move, so a changing age cannot hide a large move in the
    /// collider's frame behind a small one in the world.
    pub(super) fn carry_speed_delta(
        &self,
        i: usize,
        to: [f64; 3],
        plan: Option<&Plan>,
    ) -> Option<[f64; 3]> {
        let held = self.guests[i].on_collider?;
        match plan? {
            Plan::Fits {
                attached: true,
                fit,
                ..
            } => Some([
                fit.local.x - held.local.x,
                fit.local.y - held.local.y,
                fit.local.z - held.local.z,
            ]),
            Plan::Left { .. } => Some([
                to[0] - held.reported.x,
                to[1] - held.reported.y,
                to[2] - held.reported.z,
            ]),
            _ => None,
        }
    }

    /// [`Hub::collide`] for a guest that may be on a moving collider: the same answer, with the
    /// collision in the colliders' frames as the client saw them. The commit of an accepted
    /// carried report is left in `carry_commit`.
    pub(super) fn collide_carried(
        &mut self,
        i: usize,
        to: [f64; 3],
        ctx: &TickCtx,
        plan: Option<Plan>,
    ) -> (bool, bool, bool) {
        self.guests.update(i, |g| {
            g.carry_hint = None;
            g.carry_correction = None;
            g.carry_commit = None;
        });
        let Some(plan) = plan else {
            // No collider matters: the player is not carried, if it was.
            self.guests.update(i, |g| {
                g.on_collider = None;
            });
            return self.collide(i, to, ctx);
        };
        match plan {
            Plan::Near { age } => at_age(age, || self.collide(i, to, ctx)),
            Plan::Left { age } => {
                let from = self.guests[i]
                    .on_collider
                    .map_or(self.guests[i].position, |held| {
                        [held.reported.x, held.reported.y, held.reported.z]
                    });
                let result = at_age(age, || self.collide_from(i, from, to, ctx));
                if result.1 {
                    self.carry_rejected(i);
                } else {
                    self.guests.update(i, |g| {
                        g.on_collider = None;
                    });
                }
                result
            }
            Plan::Fits {
                collider,
                attached,
                fit,
            } => {
                let held = self.guests[i].on_collider.filter(|_| attached);
                let from = match held {
                    Some(held) => {
                        let at = fit.frame.to_world(held.local);
                        [at.x, at.y, at.z]
                    }
                    None => self.guests[i].position,
                };
                let (wrongly, refused, stands) =
                    at_age(fit.age, || self.collide_from(i, from, to, ctx));
                if refused {
                    if attached {
                        self.carry_rejected(i);
                    }
                    return (wrongly, refused, stands);
                }
                // Where the server puts the player: the local position under the collider as
                // it is now, moved there through the world's collision.
                let now = with_history(|history| {
                    let (log, _) = history.get(collider)?;
                    log.frame_at(history.now(), 0.0)
                })
                .flatten();
                let target = now.map_or(fit.local, |frame| frame.to_world(fit.local));
                let target = [target.x, target.y, target.z];
                // An attached player is moved from where the server last committed it. One that
                // attaches now is moved from where its report put it, as the client saw the
                // collider, to the same place under the collider now. The collider that carries
                // it is no obstacle to that, as its own stack is none; every other collider and
                // the blocks are.
                let committed = match held {
                    Some(held) => self.settle(
                        i,
                        [held.committed.x, held.committed.y, held.committed.z],
                        target,
                        ctx,
                    ),
                    None => as_mover(collider, || self.settle(i, to, target, ctx)),
                };
                // What is saved is the place on the collider that was actually reached, so a
                // later correction never goes past a wall the commit stopped at.
                let local = match now {
                    Some(frame) if committed != target => {
                        frame.to_local(Vec3::new(committed[0], committed[1], committed[2]))
                    }
                    _ => fit.local,
                };
                let standing = fit.footing == Footing::Standing;
                self.guests.update(i, |g| {
                    g.on_collider = Some(Carried {
                        collider,
                        age: fit.age,
                        local,
                        reported: Vec3::new(to[0], to[1], to[2]),
                        committed: Vec3::new(committed[0], committed[1], committed[2]),
                    });
                    g.carry_commit = Some(committed);
                });
                (wrongly, refused, stands || standing)
            }
        }
    }

    /// A rejected report of a carried player: counted, never a floating tick (the flag of the
    /// last accepted report is cleared, so no tick counts while the correction is on its way),
    /// and the correction is where the server last had the player on the collider, under the
    /// collider's transform now.
    pub(super) fn carry_rejected(&mut self, i: usize) {
        let corrected = self.guests[i].on_collider.and_then(|held| {
            with_history(|history| {
                let (log, _) = history.get(held.collider)?;
                let now = log.frame_at(history.now(), 0.0)?.to_world(held.local);
                Some([now.x, now.y, now.z])
            })
            .flatten()
        });
        self.guests.update(i, |g| {
            g.carry_rejects += 1;
            g.carry_correction = corrected;
            g.client_is_floating = false;
        });
    }

    /// Puts a carried player's attachment back after the teleport of its correction, which
    /// lets go of one: the same collider and local position, committed where it was sent.
    pub(super) fn keep_carried(
        &mut self,
        i: usize,
        carried: Option<Carried>,
        back: Option<[f64; 3]>,
    ) {
        let Some(held) = carried else { return };
        self.guests.update(i, |g| {
            g.on_collider = Some(Carried {
                committed: back.map_or(held.committed, |b| Vec3::new(b[0], b[1], b[2])),
                ..held
            });
        });
    }

    /// After a displacement ([`Hub::displace`]) put the player at `target`: keeps `carried`
    /// attached to the same collider when `target` is still over it as the server places the
    /// collider now (the place on it is re-derived), and lets go otherwise.
    pub(super) fn keep_carried_at(&mut self, i: usize, carried: Option<Carried>, target: [f64; 3]) {
        let Some(held) = carried else { return };
        let g = &self.guests[i];
        let dimensions = g.dimensions();
        let body = Body {
            half_width: f64::from(dimensions.width) / 2.0,
            height: f64::from(dimensions.height),
        };
        let at = Vec3::new(target[0], target[1], target[2]);
        let local = with_history(|history| {
            let (log, boxes) = history.get(held.collider)?;
            let local = log.frame_at(history.now(), 0.0)?.to_local(at);
            footing(boxes, local, body).map(|_| local)
        })
        .flatten();
        let Some(local) = local else { return };
        self.guests.update(i, |g| {
            g.on_collider = Some(Carried {
                local,
                reported: at,
                committed: at,
                ..held
            });
        });
    }
}
