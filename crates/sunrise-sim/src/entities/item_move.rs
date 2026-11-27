//! `ItemEntity`'s `Entity.move`: the tick's move step, its collision through
//! the move gate (#2414) and the block effects around it, in its own module so
//! `entities.rs` keeps its size.
use super::*;
use crate::motion::BodyMotion;
impl ItemEntity {
    /// `ItemEntity.tick` up to the blocks' push. The game has just run the
    /// fluid push (`Entity::push_by_fluid`) and passes the heights it found;
    /// it pushes by a bubble column next (`Entity::push_by_bubble_column`) and
    /// then calls [`ItemEntity::tick_settle`]. `no_physics`: the item moves
    /// through blocks this tick ([`ItemEntity::inside_block`]).
    pub fn tick_move(
        &mut self,
        world: &World,
        facts: &BlockFacts,
        collision: &BlockCollisionRegistry<'_>,
        no_physics: bool,
        heights: (f64, f64),
        entity_boxes: &[Aabb],
        border: Option<&BorderGeometry>,
        blocks: Option<&sunrise_world::movement_properties::BoundMovementProperties>,
        context: &sunrise_world::native_load_context::LoopContext,
    ) -> ItemStep {
        let step = self.tick_move_before_block_effects(
            world,
            facts,
            collision,
            no_physics,
            None,
            heights,
            entity_boxes,
            border,
            blocks,
            context,
        );
        let motion = BodyMotion {
            world,
            collision,
            facts,
            blocks,
            context,
            side: None,
        };
        if let Some(response) = self.move_restitution(motion, step) {
            self.velocity = response.velocity;
        }
        if !no_physics {
            self.apply_move_and_vanilla_step(motion, step);
        }
        self.finish_move_block_effects(world, facts, step);
        step
    }

    /// `Entity.move`: resolve the item's move before the owner handles block
    /// contact and resumes block effects. `escape_speed`: with `no_physics`
    /// on a server, the speed of `Entity.moveTowardsClosestSpace`
    /// (`nextFloat() * 0.2 + 0.1`).
    pub fn tick_move_before_block_effects(
        &mut self,
        world: &World,
        facts: &BlockFacts,
        collision: &BlockCollisionRegistry<'_>,
        no_physics: bool,
        escape_speed: Option<f32>,
        (water_height, lava_height): (f64, f64),
        entity_boxes: &[Aabb],
        border: Option<&BorderGeometry>,
        blocks: Option<&BoundMovementProperties>,
        context: &sunrise_world::native_load_context::LoopContext,
    ) -> ItemStep {
        self.ticks += 1;
        self.previous = self.position;
        if self.pickup_delay > 0 && !self.has_infinite_pickup_delay() {
            self.pickup_delay -= 1;
        }
        let prior_fall = self.fall_distance;
        let old_velocity = self.velocity;
        // Entity.updateInWaterStateAndDoFluidPushing: touching water resets the fall.
        if water_height > 0.0 {
            self.fall_distance = 0.0;
        }
        let in_water = water_height > f64::from(0.1f32);
        if in_water {
            // ItemEntity.setUnderwaterMovement: 0.99F, 5.0E-4F, 0.06F.
            self.set_fluid_movement(0.99_f32);
        } else if lava_height > f64::from(0.1f32) {
            // ItemEntity.setUnderLavaMovement: 0.95F, 5.0E-4F, 0.06F.
            self.set_fluid_movement(0.95_f32);
        } else {
            self.velocity.y -= self.get_gravity();
        }
        // ItemEntity.tick: out of a block placed on it after gravity, so the
        // escape's 0.75 scales the fallen vertical speed (#12018).
        if let Some(speed) = escape_speed {
            self.move_towards_closest_space(world, facts, speed);
        }
        let horizontal = self.velocity.x * self.velocity.x + self.velocity.z * self.velocity.z;
        // ItemEntity.tick: skip move 3 of 4 ticks while grounded with no
        // horizontal speed. The idle branch still runs bubble-column push
        // (`applyEffectsFromBlocksForLastMovements`), so lift builds up and
        // the item moves on the next wake-up (#4558).
        if !self.ground.on_ground
            || horizontal > 1.0e-5
            || self.ticks.wrapping_add(self.id).is_multiple_of(4)
        {
            let scene = SceneCollisions {
                world,
                binding: collision,
                subject: None,
            };
            let from = self.position;
            let wanted = self.velocity;
            // The item's Entity.move goes through the move gate (#2414): armed
            // before the move's own handling, no-physics and the stuck
            // multiplier included. PR 1a records the movement unchanged.
            let mut gate = crate::move_gate::armed(
                context,
                self.id,
                "minecraft:item",
                sunrise_world::entity_move::MoverType::SelfMovement,
                wanted,
            );
            let mut wanted = gate.delta();
            // Entity.move with noPhysics: straight through, the ground as it
            // was — and vanilla returns before the stuck multiplier, which
            // stays pending for the next move with physics.
            let moved = if no_physics {
                wanted
            } else {
                // Entity.move: a block the item sits in (cobweb, powder snow)
                // scales this move once and stops it.
                if self.stuck.length_squared() > 1.0e-7 {
                    wanted = wanted.multiply(self.stuck);
                    self.stuck = Vec3::ZERO;
                    self.velocity = Vec3::ZERO;
                }
                // `Entity.collide`: the entities that `canBeCollidedWith` the item stop it
                // too (a boat, a shulker, a happy ghast held still; not a minecart), and
                // so does the world border (`Entity.collectColliders`).
                let mut collisions = crate::collisions::EntityCollisions {
                    blocks: scene,
                    entities: entity_boxes,
                    border: border.map(|b| (b, [self.position.x, self.position.z])),
                };
                crate::move_gate::entities_resolve_with_context(
                    &mut gate,
                    &mut collisions,
                    self.bounds(),
                    wanted,
                    0.0,
                    self.ground.on_ground,
                    self.collision_context(),
                )
                .unwrap_or(Vec3::ZERO)
            };
            let applied = move_applies(wanted, moved);
            if applied {
                self.position = Vec3::new(
                    self.position.x + moved.x,
                    self.position.y + moved.y,
                    self.position.z + moved.z,
                );
            }
            let physics = !no_physics && !gate.skipped();
            if physics {
                let authoritative =
                    context.side() == sunrise_world::native_load_context::LoopSide::Server;
                if authoritative || wanted.y.abs() > 0.0 {
                    self.vertical_collision = moved.y != wanted.y;
                }
                BodyMotion {
                    world,
                    collision,
                    facts,
                    blocks,
                    context,
                    side: None,
                }
                .record_move(
                    crate::motion::BodyCollision {
                        footprint: (self.position, self.bounds()),
                        movement: (wanted, moved),
                        authoritative,
                        context: self.collision_context(),
                        subject: None,
                    },
                    &mut self.ground,
                );
            }
            let restitution = (physics
                && ((wanted.y.abs() > 0.0 && self.vertical_collision)
                    || self.ground.horizontal_collision))
                .then(|| {
                    let effect = BodyMotion {
                        world,
                        collision,
                        facts,
                        blocks,
                        context,
                        side: None,
                    }
                    .legacy_contact(self.position, &self.ground);
                    (wanted, moved, effect)
                });
            let mut landed = None;
            if physics {
                // Entity.checkFallDamage: the move's drop counts out of water, and a landing
                // hands the whole fall to the block below (`getOnPosLegacy`).
                if water_height <= 0.0 && moved.y < 0.0 {
                    self.fall_distance -= f64::from(moved.y as f32);
                }
                if self.ground.on_ground {
                    if self.fall_distance > 0.0 {
                        landed = restitution
                            .map(|(_, _, contact)| (contact.position, self.fall_distance));
                    }
                    self.fall_distance = 0.0;
                }
            }
            let vertical_contact = restitution
                .filter(|(wanted, moved, _)| wanted.y != moved.y)
                .map(|(_, _, contact)| contact.position);
            return ItemStep {
                old_velocity,
                vertical_contact,
                restitution,
                movement: Some(moved),
                landed,
                prior_fall,
                fall_move: physics.then_some((from, moved, applied)),
            };
        }
        ItemStep {
            old_velocity,
            landed: None,
            vertical_contact: None,
            movement: None,
            restitution: None,
            prior_fall,
            fall_move: None,
        }
    }

    /// The remainder of `Entity.move` after the owning side's fall check.
    pub fn move_restitution(
        &self,
        motion: BodyMotion<'_>,
        step: ItemStep,
    ) -> Option<crate::motion::BodyRestitution> {
        let (wanted, moved, effect) = step.restitution?;
        Some(motion.restitute(
            (self.id, "minecraft:item"),
            crate::motion::BodyMove {
                wanted,
                moved,
                effect,
            },
            self.get_gravity(),
            0.98_f32,
            self.velocity,
            self.fall_distance,
        ))
    }

    /// The speed factor from `Entity.move`, which an idle item did not run.
    pub fn apply_move_speed(&mut self, motion: BodyMotion<'_>, step: ItemStep) {
        if step.movement.is_some() {
            motion.apply_block_speed(self.position, &mut self.velocity);
        }
    }

    /// `Entity.move`'s speed and the vanilla step in a stand-alone or worker tick.
    pub fn apply_move_and_vanilla_step(&mut self, motion: BodyMotion<'_>, step: ItemStep) {
        self.apply_move_speed(motion, step);
        let _ = motion.step_contact(
            (self.id, "minecraft:item"),
            (self.position, self.bounds()),
            &self.ground,
            &mut self.velocity,
        );
    }

    /// The item-specific inside effects of a move, after the grounded step.
    pub fn finish_move_block_effects(&mut self, world: &World, facts: &BlockFacts, step: ItemStep) {
        if step.movement.is_some() {
            self.apply_honey_slide(world, facts);
            if let Some(stuck) = self.stuck_multiplier(world, facts) {
                // Entity.makeStuckInBlock resets the fall. No contact leaves
                // a multiplier already set by step-on or an inside Part alone.
                self.fall_distance = 0.0;
                self.stuck = stuck;
            }
        }
    }
}
