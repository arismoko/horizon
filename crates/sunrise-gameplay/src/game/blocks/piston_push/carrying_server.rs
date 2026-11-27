//! Server-side moving-collider carrying pass (`MoveWho::Entity(_)` only, preserving
//! stacked collider forest depth ordering and `position_riders` synchronization).
use super::{
    entity::{Entity, with_entity, with_entity_mut},
    move_gate::MoveWho,
};
use sunrise_core::geometry::{Axis, Vec3};
use sunrise_gameplay_common::game::{
    collision_source::{CollisionSourceGameExt1 as _, Mover},
    piston_push::{
        Body, PistonPushGameExt1 as _, StepHeights,
        carrying::{Rider, SUPPORT_PROBE, support_of, upright_tilt_clearance},
    },
};
use sunrise_level::collisions::{EntityCollisions, SceneCollisions};
use sunrise_world::{
    collision_context::CollisionContext,
    entity_move::MoverType,
    entity_obstruction::EntityIdentity,
    moving_colliders::{self, Prepared, Shove},
    world_movement::{CollisionCheckRequest, no_world_collision},
};

/// What a server carry pass works on, kept from tick to tick so a pass allocates nothing once
/// these have grown to the level's size.
#[derive(Default)]
struct Scratch {
    riders: Vec<Rider>,
    stacked: Vec<(u32, u32)>,
    order: Vec<(u32, u32)>,
    ids: Vec<u32>,
    /// Candidate face heights for clearing a tilted support under an upright body.
    heights: Vec<f64>,
}

thread_local! {
    static SCRATCH: std::cell::RefCell<Scratch> = std::cell::RefCell::new(Scratch::default());
}

impl crate::game::ServerGame<'_> {
    /// Server arm of [`Game::side_carry_pass`]: carries and shoves server-simulated
    /// entities (`MoveWho::Entity(_)`, excluding players and guests).
    pub(in crate::game) fn server_carry_pass(&mut self, start: &Prepared) {
        if start.is_empty() || !self.server_any_collider_moved(start) {
            return;
        }
        let mut scratch = SCRATCH.with(|cell| std::mem::take(&mut *cell.borrow_mut()));
        let steps = self.push_step_heights();
        scratch.riders.clear();
        self.server_collider_riders(start, &steps, &mut scratch);
        if !scratch.riders.is_empty() {
            self.server_carry_in_order(start, &steps, &mut scratch);
            // A passenger that owns a collider moves with its vehicle too.
            self.position_riders();
        }
        self.server_shove_pass(start, &steps, &scratch.riders);
        SCRATCH.with(|cell| *cell.borrow_mut() = scratch);
    }

    fn server_any_collider_moved(&self, start: &Prepared) -> bool {
        start.placed().any(|p| {
            self.collider_now_frame(p.id)
                .is_some_and(|now| now != p.collider.frame)
        })
    }

    fn server_carry_in_order(
        &mut self,
        start: &Prepared,
        steps: &StepHeights,
        scratch: &mut Scratch,
    ) {
        scratch.order.clear();
        let mut at = 0;
        while at < scratch.riders.len() {
            let collider = scratch.riders[at].support;
            let mut depth = 0;
            let mut up = collider;
            while let Ok(i) = scratch.stacked.binary_search_by_key(&up, |&(id, _)| id) {
                up = scratch.stacked[i].1;
                depth += 1;
                if depth > start.len() as u32 || up == collider {
                    break;
                }
            }
            scratch.order.push((depth, collider));
            while at < scratch.riders.len() && scratch.riders[at].support == collider {
                at += 1;
            }
        }
        scratch.order.sort_unstable();
        let mut stand_grid = None;
        let mut wave = 0;
        let mut at = 0;
        while at < scratch.order.len() {
            wave = wave.max(scratch.order[at].0);
            let now = self.placed_moving_colliders();
            {
                let _scope =
                    moving_colliders::Scope::enter_level(self.collider_level(), now.clone());
                while at < scratch.order.len() && scratch.order[at].0 <= wave {
                    let collider = scratch.order[at].1;
                    let first = scratch.riders.partition_point(|r| r.support < collider);
                    let count = scratch.riders[first..]
                        .iter()
                        .take_while(|r| r.support == collider)
                        .count();
                    self.server_carry_riders_of(
                        collider,
                        start,
                        &now,
                        steps,
                        first..first + count,
                        scratch,
                        &mut stand_grid,
                    );
                    at += 1;
                }
            }
            wave += 1;
        }
    }

    fn server_collider_riders(
        &mut self,
        start: &Prepared,
        steps: &StepHeights,
        scratch: &mut Scratch,
    ) {
        scratch.riders.clear();
        scratch.stacked.clear();
        scratch.ids.clear();
        scratch.ids.extend(start.placed().map(|p| p.id));
        scratch.ids.sort_unstable();
        for target in self.push_targets() {
            if !matches!(target, MoveWho::Entity(_)) {
                continue;
            }
            let Some(body) = self.push_body(target, None, steps) else {
                continue;
            };
            if !body.on_ground {
                continue;
            }
            let (id, no_physics, rides) = self.server_carry_facts(target);
            if no_physics || rides {
                continue;
            }
            if let Some(support) = support_of(start, body.bounds, id) {
                scratch.riders.push(Rider {
                    target,
                    id,
                    support,
                });
            }
        }
        scratch.riders.sort_unstable_by_key(|r| r.support);
        for rider in &scratch.riders {
            if scratch.ids.binary_search(&rider.id).is_ok() {
                scratch.stacked.push((rider.id, rider.support));
            }
        }
        scratch.stacked.sort_unstable();
    }

    /// The entity id of a server-carried body, whether it has no physics, and whether it rides.
    pub(in crate::game::server_side) fn server_carry_facts(
        &self,
        target: MoveWho,
    ) -> (u32, bool, bool) {
        match target {
            MoveWho::Entity(handle) => {
                let (id, no_physics) = with_entity!(self, handle, entity => {
                    let stored = self.persist.no_physics.get(&entity.id()).copied();
                    (entity.id(), entity.no_physics(stored))
                });
                (id, no_physics, self.push_root(target) != Some(id))
            }
            MoveWho::Player | MoveWho::Guest(_) => (0, true, false),
        }
    }

    fn server_carry_riders_of(
        &mut self,
        collider: u32,
        start: &Prepared,
        now: &Prepared,
        steps: &StepHeights,
        riders: std::ops::Range<usize>,
        scratch: &mut Scratch,
        stand_grid: &mut Option<sunrise_sim::entities::ObstacleGrid>,
    ) {
        let Some(before) = start.placed().find(|p| p.id == collider) else {
            return;
        };
        let Some(after) = now.placed().find(|p| p.id == collider) else {
            return;
        };
        let turned = !before.collider.frame.same_turn(&after.collider.frame);
        for rider in &scratch.riders[riders] {
            let Some(body) = self.push_body(rider.target, None, steps) else {
                continue;
            };
            let centre = body.bounds.center();
            let contact = Vec3::new(centre.x, body.bounds.min(Axis::Y), centre.z);
            let rigid = after
                .collider
                .frame
                .to_world(before.collider.frame.to_local(contact))
                .subtract(contact);
            let carried = if turned {
                upright_tilt_clearance(&after.collider, body.bounds, rigid, &mut scratch.heights)
            } else {
                rigid
            };
            if carried.length_squared() >= 1.0e-18 {
                let _shove = Shove::enter(rider.id, collider);
                self.move_pushed_body(
                    MoverType::Carried,
                    rider.target,
                    body,
                    carried,
                    false,
                    stand_grid,
                );
            }
            let grounded = self
                .push_body(rider.target, None, steps)
                .is_some_and(|after_body| {
                    self.carried_body_grounded(rider.target, rider.id, after_body)
                });
            if let MoveWho::Entity(handle) = rider.target {
                with_entity_mut!(self, handle, entity => entity.set_piston_on_ground(grounded));
            }
        }
    }

    /// Collision inspection of the post-carry body and a downward support probe. A body that is
    /// already intersecting a block/entity/collider or has nothing under it is not grounded.
    fn carried_body_grounded(&mut self, target: MoveWho, id: u32, body: Body) -> bool {
        let entities = if target == MoveWho::Player {
            self.entity_collisions(Mover::Player)
        } else {
            self.pushed_entity_colliders(target)
        };
        let mut collisions = EntityCollisions::with_border(
            SceneCollisions {
                world: &self.common.world,
                binding: self.common.collision,
                subject: None,
            },
            self.common.border.geometry(),
            [body.position.x, body.position.z],
        );
        collisions.entities = &entities;
        let mut clear = |bounds| {
            no_world_collision(
                &mut collisions,
                CollisionCheckRequest {
                    entity: EntityIdentity(u64::from(id)),
                    context: CollisionContext::position(body.bounds.min(Axis::Y)),
                    bounds,
                    candidate_budget: 1_000_000,
                },
            )
        };
        clear(body.bounds).is_ok_and(|clear| clear)
            && clear(body.bounds.move_by(Vec3::new(0.0, -SUPPORT_PROBE, 0.0)))
                .is_ok_and(|clear| !clear)
    }
}

use sunrise_gameplay_common::game::passengers_common::PassengersCommonGameExt1 as _;
