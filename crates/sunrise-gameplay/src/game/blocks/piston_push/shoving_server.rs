//! Server-side directional push of mod moving colliders (`MoveWho::Entity(_)` only).
use super::{
    entity::{Entity, with_entity, with_entity_mut},
    move_gate::MoveWho,
};
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::piston_push::{
    StepHeights,
    carrying::{Rider, cancel_shove_velocity, collect_sweeps, sweep_displacement},
};
use sunrise_world::entity_move::MoverType;
use sunrise_world::moving_colliders::{self, Prepared, Shove};

impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn server_shove_pass(
        &mut self,
        start: &Prepared,
        steps: &StepHeights,
        riders: &[Rider],
    ) {
        let end = self.placed_moving_colliders();
        let sweeps = collect_sweeps(start, &end);
        if sweeps.is_empty() {
            return;
        }

        let targets: Vec<_> = self
            .push_targets()
            .filter(|target| matches!(target, MoveWho::Entity(_)))
            .filter_map(|target| self.stable_push_target(target))
            .collect();
        let mut current = end.clone();
        let mut stand_grid = None;
        let mut shoved = false;
        for (after, kind, swept) in sweeps {
            let source = after.id;
            let first = riders.partition_point(|rider| rider.support < source);
            let supported = &riders[first..];
            for &selected in &targets {
                if after.exempt.contains(&selected.id)
                    || supported
                        .iter()
                        .take_while(|r| r.support == source)
                        .any(|r| r.id == selected.id)
                {
                    continue;
                }
                let Some(target) = self.resolve_push_target(selected) else {
                    continue;
                };
                let Some(body) = self.push_body(target, None, steps) else {
                    continue;
                };
                if !swept.intersects(body.bounds) {
                    continue;
                }
                let (_, no_physics, rides) = self.server_carry_facts(target);
                if no_physics || rides {
                    continue;
                }
                if let Some(movement) = sweep_displacement(&after.collider, kind, body.bounds) {
                    {
                        let _scope = moving_colliders::Scope::enter_level(
                            self.collider_level(),
                            current.clone(),
                        );
                        let _shove = Shove::enter(selected.id, source);
                        self.move_pushed_body(
                            MoverType::Carried,
                            target,
                            body,
                            movement,
                            false,
                            &mut stand_grid,
                        );
                    }
                    shoved = true;
                    if let Some(MoveWho::Entity(handle)) = self.resolve_push_target(selected) {
                        let moved = with_entity!(self, handle, entity => entity.position())
                            .subtract(body.position);
                        if moved != Vec3::ZERO {
                            let rise = after.collider.frame.origin.y - kind.before().frame.origin.y;
                            let normal = movement.normalize();
                            with_entity_mut!(self, handle, entity => {
                                let velocity = cancel_shove_velocity(
                                    *entity.velocity_mut(),
                                    normal,
                                    movement.y,
                                    rise,
                                );
                                *entity.velocity_mut() = velocity;
                            });
                        }
                    }
                    if current.placed().any(|p| p.exempt.contains(&selected.id)) {
                        // This body owns a collider, or is the vehicle of a holder. Its
                        // new pose is an obstacle to later shoves, not another sweep.
                        self.position_riders();
                        current = self.placed_moving_colliders();
                    }
                }
            }
        }
        if shoved {
            self.position_riders();
        }
    }
}

use sunrise_gameplay_common::game::passengers_common::PassengersCommonGameExt1 as _;
