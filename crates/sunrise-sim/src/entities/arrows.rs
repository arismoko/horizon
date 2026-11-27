//! `AbstractArrow.tick` for arrows, tipped and spectral arrows: the flight with
//! its water inertia and bubbles, the base tick's fluids (currents, splash),
//! bubble columns and water along the path, sticking in blocks, falling out
//! when the block goes, and despawning. The tick stops at an entity it hits:
//! the game hurts the entity (`projectile_hit`), then `finish_entity_hit`
//! spends the arrow or turns it back and ends the tick.
use crate::entity_rules::EntityRules;
use crate::{
    blocks::BlockFacts,
    collisions::SceneCollisions,
    mobs::Who,
    motion::{FluidResources, LAVA, WATER},
    projectiles::{
        Hit, NearestHit, Projectile, ProjectileKind, TargetCandidate, TargetOrder, compute_margin,
    },
    world::World,
};
use std::fmt;
use sunrise_core::nbt::Tag;
use sunrise_core::{
    geometry::{Aabb, Direction, Vec3},
    position::BlockPos,
    random::RandomSource,
};
use sunrise_world::{
    block_collision::BlockCollisionRegistry,
    entity_fluid::{
        CurrentTarget, EntityFluidInteraction, FluidInteractionWorld, FluidMembership,
        FluidSections, FluidTagId,
    },
    entity_fluid_tick::{self, EntityFluidTickWorld},
    fluid::{FluidHeightWorld, FluidKind, FluidState},
    fluid_flow::FluidFlowWorld,
    registry::BlockStateId,
    water_splash::{self, SplashParticle, WaterSplashWorld},
};

include!("arrows/state.rs");

/// The blocks, fluids and weather an arrow flies through.
pub struct ArrowScene<'a, 'r> {
    pub world: &'a World,
    pub registry: &'a sunrise_world::registry::BlockRegistry,
    pub facts: &'a BlockFacts,
    pub collision: &'a BlockCollisionRegistry<'r>,
    pub fluids: &'a FluidResources,
    /// The loop context of the side that ticks the arrow, for the movement hooks.
    pub loop_context: &'a sunrise_world::native_load_context::LoopContext,
    /// `Level.isRainingAt`.
    pub raining: &'a dyn Fn(BlockPos) -> bool,
    /// The `mob_griefing` and `tnt_explodes` game rules.
    pub mob_griefing: bool,
    pub tnt_explodes: bool,
    /// The mod hooks a mob-owned arrow asks (`MobWorld::mob_hooks`).
    pub mob_hooks: Option<&'a dyn crate::mobs::MobHooks>,
    pub owners: &'a dyn OwnerLookup,
}
/// `EntityReference.getEntity` for an owner the arrow only knows by UUID.
pub trait OwnerLookup {
    /// A loaded entity other than a player has this UUID.
    fn non_player_entity(&self, uuid: [i32; 4]) -> bool;
}
impl OwnerLookup for () {
    fn non_player_entity(&self, _: [i32; 4]) -> bool {
        false
    }
}
impl ArrowScene<'_, '_> {
    pub fn block(&self, p: BlockPos) -> BlockStateId {
        self.world.block(p.x, p.y, p.z)
    }
    fn is_air(&self, state: BlockStateId) -> bool {
        self.facts.is_air(state)
    }
    /// `BlockState.getCollisionShape(level, pos)` (an empty collision context).
    fn collision_boxes(&self, state: BlockStateId, p: BlockPos) -> Vec<Aabb> {
        let mut scene = SceneCollisions {
            world: self.world,
            binding: self.collision,
            subject: None,
        };
        self.collision
            .collision_shape(&mut scene, state, [p.x, p.y, p.z])
            .map(|s| s.boxes().to_vec())
            .unwrap_or_default()
    }
    fn property(&self, state: BlockStateId, name: &str) -> Option<&str> {
        self.registry
            .state(state)?
            .properties
            .get(name)
            .map(String::as_str)
    }
    /// `ClipContext.Block.COLLIDER` with `Fluid.NONE`: the first block hit.
    fn clip(
        &self,
        from: Vec3,
        to: Vec3,
        ignored: Option<BlockPos>,
    ) -> Option<(Vec3, BlockPos, [i32; 3])> {
        let mut scene = SceneCollisions {
            world: self.world,
            binding: self.collision,
            subject: None,
        };
        // CollisionContext.of(arrow): scaffolding is open from below.
        let context = sunrise_world::collision_context::CollisionContext::position(from.y);
        let hit = sunrise_core::geometry::traverse_blocks(from, to, 1024, |p: BlockPos| {
            if Some(p) == ignored {
                return None;
            }
            let state = self.block(p);
            if self.is_air(state) {
                return None;
            }
            let shape = self
                .collision
                .collision_shape_with_context(&mut scene, state, p, &context)
                .ok()?;
            shape.clip(from, to, p)
        });
        let hit = hit.ok().flatten()?;
        Some((hit.location, hit.block, hit.direction.normal()))
    }
    /// `CollisionGetter.noCollision(null, box)` over blocks.
    pub fn no_block_collision(&self, b: Aabb) -> bool {
        let (lo, hi) = (b.min_position(), b.max_position());
        for x in (lo.x.floor() as i32 - 1)..=(hi.x.floor() as i32 + 1) {
            for y in (lo.y.floor() as i32 - 1)..=(hi.y.floor() as i32 + 1) {
                for z in (lo.z.floor() as i32 - 1)..=(hi.z.floor() as i32 + 1) {
                    let p = BlockPos::new(x, y, z);
                    let state = self.block(p);
                    if self.is_air(state) {
                        continue;
                    }
                    let origin = Vec3::new(f64::from(x), f64::from(y), f64::from(z));
                    if self
                        .collision_boxes(state, p)
                        .iter()
                        .any(|c| c.move_by(origin).intersects(b))
                    {
                        return false;
                    }
                }
            }
        }
        true
    }
}
include!("arrows/contact.rs");

include!("arrows/flight.rs");

#[cfg(test)]
#[path = "../arrows_client_tests.rs"]
mod client_tests;
