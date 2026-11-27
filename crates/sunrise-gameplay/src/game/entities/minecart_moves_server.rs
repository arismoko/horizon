use super::minecarts::Minecart;
use crate::game::ServerGame;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::collision_source::Mover;
use sunrise_level::collisions::SceneCollisions;

impl ServerGame<'_> {
    /// Entity.move for a cart: collisions zero the blocked components, the world border's walls
    /// among them (`CollisionGetter.borderCollision`, asked for every entity).
    pub fn move_cart(&mut self, c: &mut Minecart, delta: Vec3) -> bool {
        // The cart's own move is a self move (#2414: vanilla's
        // `AbstractMinecart.move` passes SELF for its own tick); PR 1a
        // records the movement unchanged.
        let mut gate = sunrise_sim::move_gate::armed(
            &self.common.loop_context,
            c.id,
            c.kind.id(),
            sunrise_world::entity_move::MoverType::SelfMovement,
            delta,
        );
        let mut delta = gate.delta();
        // Entity.move: a stuck multiplier scales this move and zeroes the velocity.
        if c.stuck.length_squared() > 1.0e-7 {
            delta = delta.multiply(c.stuck);
            c.stuck = Vec3::ZERO;
            c.velocity = Vec3::ZERO;
        }
        let scene = SceneCollisions {
            world: &self.common.world,
            binding: self.common.collision,
            subject: None,
        };
        // A zero move stays zero whatever surrounds it (`Entity.collide`, and the
        // step-up needs a blocked horizontal move): a cart standing on its rails,
        // as most do, gathers no entity boxes.
        let entities = if delta.length_squared() == 0.0 {
            Vec::new()
        } else {
            self.entity_collisions(Mover::Cart(c))
        };
        let mut scene = sunrise_level::collisions::EntityCollisions {
            blocks: scene,
            entities: &entities,
            border: Some((self.common.border.geometry(), [c.position.x, c.position.z])),
        };
        if let Ok(moved) = sunrise_sim::move_gate::entities_resolve(
            &mut gate,
            &mut scene,
            c.bounds(),
            delta,
            0.0,
            c.ground.on_ground,
        ) {
            let from = c.position;
            c.position = Vec3::new(
                c.position.x + moved.x,
                c.position.y + moved.y,
                c.position.z + moved.z,
            );
            let side = game_side!(self);
            let motion = sunrise_sim::motion::BodyMotion {
                world: &self.common.world,
                collision: self.common.collision,
                facts: self.common.res.facts(),
                blocks: Some(&self.common.motion.blocks),
                context: &self.common.loop_context,
                side: Some(&side),
            };
            if !gate.skipped() {
                motion.record_move(
                    sunrise_sim::motion::BodyCollision {
                        footprint: (c.position, c.bounds()),
                        movement: (delta, moved),
                        authoritative: true,
                        context: sunrise_sim::motion::BodyMotion::entity_context(c.id, c.position),
                        subject: None,
                    },
                    &mut c.ground,
                );
            }
            let effect = motion.legacy_contact(c.position, &c.ground);
            let (fall_distance, landing) = self.cart_move_fall(c, from, moved);
            c.fall_distance = fall_distance;
            if landing.is_some() {
                let id = c.id;
                let join = c.join;
                if !self
                    .finish_cart_landing(landing, effect, |game, power| game.explode_cart(c, power))
                    || !self
                        .common
                        .carts
                        .iter()
                        .any(|row| row.id == id && row.join == join)
                    || self
                        .common
                        .persist
                        .removals
                        .contains(&super::commands::Target::Cart(id))
                {
                    return false;
                }
            }
            let entity = (c.id, c.kind.id());
            let response = self.common.body_motion(&game_side!(self)).restitute(
                entity,
                sunrise_sim::motion::BodyMove {
                    wanted: delta,
                    moved,
                    effect,
                },
                sunrise_sim::entity_rules::EntityRules::get_gravity(c),
                0.95_f32,
                c.velocity,
                c.fall_distance,
            );
            c.velocity = self.finish_body_restitution(entity, c.position, response);
            self.common
                .body_motion(&game_side!(self))
                .apply_block_speed(c.position, &mut c.velocity);
        }
        // OldMinecartBehavior applies block effects here and at tick end.
        // NewMinecartBehavior applies them after recording each track step.
        if !self.minecart_improvements() {
            self.cart_block_effects(c);
        }
        true
    }
}

use sunrise_gameplay_common::game::collision_source::CollisionSourceGameExt1 as _;

use sunrise_gameplay_common::game::minecarts::MinecartRailGameExt as _;
