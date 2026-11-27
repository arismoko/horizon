//! `Entity`: what every kind in the game's entity lists answers to, so a system
//! can treat them alike and only write down where vanilla's kinds differ.
//!
//! Default methods follow vanilla's `Entity`; a kind overrides one only where
//! its vanilla class does. [`EntityHandle`] names one entity by list and index,
//! so a system can walk handles and reach each entity after other borrows of
//! `Game` end. The walk allocates nothing and dispatch is static: hot paths pay
//! no vtable. A new kind is one line in `entity_lists!` and one `impl Entity`.

use crate::game::{
    FallingBlock, boats_common::Boat, commands_common::Target, fireworks::Rocket,
    minecarts::Minecart, tnt_common::PrimedTnt,
};
use crate::game::{boats_common, dragon_common, tnt_common};
use std::borrow::Cow;
use sunrise_core::geometry::{Aabb, Direction, Vec3};
use sunrise_core::position::BlockPos;
use sunrise_level::collisions::{PowderSnowSubject, SceneCollisions};
use sunrise_sim::entities::{ArmorStand, ExperienceOrb, ItemEntity};
use sunrise_sim::entity_rules::EntityRules;
use sunrise_sim::join_seq::{JoinSeq, Joined as _};
use sunrise_sim::mobs::{Mob, Species};
use sunrise_sim::projectiles::{Projectile, ProjectileKind};
use sunrise_sim::thrown_trident::ThrownTrident;
use sunrise_world::{
    block_collision_scan::CollisionScanError, collision_context::CollisionContext,
    ground_support::GroundState, level::data::PushReaction, move_gate::MoveGate, piston_push,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ThunderHit {
    Untouched,
    /// Burns, takes 5 damage, and the kinds with their own `thunderHit` change.
    Mob,
    /// `VehicleEntity.hurtServer`: 5 x 10 is over the 40 that destroys it.
    Vehicle,
    /// `ExperienceOrb.hurtServer`: 5 damage off 5 health.
    Orb,
    /// `ItemEntity.hurtServer`: burns up unless fire resistant.
    Item,
}

/// What a blast knows about itself, for the entities in its reach to answer.
#[derive(Clone, Copy)]
pub struct Blast {
    /// `VehicleEntity.ignoreExplosion`: a mob's blast, with mob griefing off.
    pub vehicles_ignore: bool,
    /// `ItemEntity.ignoreExplosion` (`shouldAffectBlocklikeEntities`).
    pub items_ignore: bool,
    /// The exploding mob, which `getEntities(source, box)` leaves out.
    pub spared: Option<u32>,
}

/// `EntityType.TRIDENT.getDimensions()`: width, height and eye height.
pub(in crate::game) const THROWN_TRIDENT_DIMENSIONS: [f32; 3] = [0.5, 0.5, 0.13];

pub trait Entity: EntityRules {
    /// `Entity.getId`.
    fn id(&self) -> u32;
    fn position(&self) -> Vec3;
    fn position_mut(&mut self) -> &mut Vec3;
    /// Where it stood when the last tick began (`Entity.lastKnownPosition`'s
    /// reading): `position - previous_position` is `computeSpeed`.
    fn previous_position(&self) -> Vec3;
    /// `Entity.getDeltaMovement`.
    fn velocity(&self) -> Vec3;
    fn velocity_mut(&mut self) -> &mut Vec3;
    /// The server's accumulated `Entity.fallDistance`. A kind that does not track it has zero.
    fn fall_distance(&self) -> f64 {
        self.body_fall_distance()
    }
    /// `Entity.resetFallDistance`: how far it has fallen is forgotten, so its next landing
    /// hurts it as a step does. Required, so a new kind states whether it keeps a fall distance.
    fn reset_fall_distance(&mut self);
    /// `Entity.getYRot` and `getXRot`: yaw, pitch.
    fn rotation(&self) -> [f32; 2];
    /// `Entity.yRotO` and `xRotO`: the previous tick's yaw and pitch.
    fn previous_rotation(&self) -> [f32; 2];
    /// The rotation part of `Entity.teleportSetPosition`: `setYRot`, `setXRot`
    /// (clamped to -90..90), then `setOldRot`.
    fn set_rotation(&mut self, rotation: [f32; 2]);
    fn bounds(&self) -> Aabb;
    /// `Entity.getDimensions(getPose())` before `entity_size`, for a kind the server asks for
    /// at its size passes (`modding/entity_sizes.rs`), its pose standing; None for one asked
    /// another way (a mob, by `native_mob_sizes`) or not at all.
    fn vanilla_dimensions(&self) -> Option<[f32; 3]> {
        None
    }
    /// What `entity_size` last answered for it ([`Self::vanilla_dimensions`]).
    fn size_answer(&self) -> Option<sunrise_sim::pose::SizeAnswer> {
        None
    }
    /// Keep `answer`: its box takes it while it holds.
    fn set_size_answer(&mut self, _answer: sunrise_sim::pose::SizeAnswer) {}
    /// What a selector finds this entity as (`@e`), or `None` for one it cannot
    /// name yet. Required, so a new kind states which.
    fn selector_target(&self) -> Option<Target>;
    /// The entity type id a native mod's join hook names (`minecraft:oak_boat`).
    fn native_type(&self) -> Cow<'_, str>;
    fn on_ground(&self) -> bool {
        self.ground_state().is_some_and(|g| g.on_ground)
    }
    fn body_fall_distance(&self) -> f64 {
        0.0
    }
    fn ground_state(&self) -> Option<&GroundState> {
        None
    }
    fn ground_state_mut(&mut self) -> Option<&mut GroundState> {
        None
    }
    fn support_collision(
        &self,
        _tags: &sunrise_items::items::EntityTypeTags,
    ) -> (CollisionContext<'_>, Option<PowderSnowSubject<'_>>) {
        (
            sunrise_sim::motion::BodyMotion::entity_context(self.id(), self.position()),
            Some(PowderSnowSubject {
                fall_distance: self.body_fall_distance(),
                ..Default::default()
            }),
        )
    }
    /// Update the live owner's retained support. None is `Entity.setOnGround`'s null movement.
    fn update_retained_ground(
        &mut self,
        world: &sunrise_level::world::World,
        binding: &sunrise_world::block_collision::BlockCollisionRegistry<'_>,
        tags: &sunrise_items::items::EntityTypeTags,
        on_ground: bool,
        movement: Option<Vec3>,
    ) -> Result<bool, CollisionScanError<std::io::Error>> {
        let Some(mut ground) = self.ground_state().copied() else {
            return Ok(false);
        };
        ground.on_ground = on_ground;
        let (context, subject) = self.support_collision(tags);
        let mut scene = SceneCollisions {
            world,
            binding,
            subject,
        };
        let scan = scene.update_ground_support(
            &mut ground,
            self.position(),
            self.bounds(),
            movement,
            context,
        );
        if let Some(owner) = self.ground_state_mut() {
            // Entity.setOnGround writes the flag before checking support.
            *owner = ground;
            return scan.map(|()| true);
        }
        Ok(false)
    }
    /// `Entity.maxUpStep`.
    fn get_step_height(&self) -> f32 {
        0.0
    }
    fn is_alive(&self) -> bool {
        true
    }
    /// `Entity.isRemoved`.
    fn is_removed(&self) -> bool {
        false
    }
    /// `Entity.isPickable`: true for kinds a projectile or ray pick can strike.
    fn is_pickable(&self) -> bool {
        false
    }
    /// `Entity.canBeHitByProjectile`: alive and pickable.
    fn can_be_hit_by_projectile(&self) -> bool {
        self.is_alive() && self.is_pickable()
    }
    /// `instanceof LivingEntity`.
    fn is_living(&self) -> bool {
        false
    }
    /// `Entity.isIgnoringBlockTriggers`.
    fn is_ignoring_block_triggers(&self) -> bool {
        false
    }
    /// Whether it presses plates and tripwires: `getEntitiesOfClass` skips
    /// removed entities, and the block's predicate skips the ones above.
    fn triggers_blocks(&self) -> bool {
        !self.is_removed() && !self.is_ignoring_block_triggers()
    }
    /// Whether `tick_blocks` sends the blocks in its box to `entityInside`
    /// (`applyEffectsFromBlocks`). A kind that function sweeps another way, or
    /// whose tick skips that step, says no.
    fn sweeps_blocks_inside(&self) -> bool {
        true
    }
    /// `Entity.thunderHit` for this kind.
    fn thunder_hit(&self) -> ThunderHit {
        ThunderHit::Untouched
    }
    /// The box a lightning strike tests against.
    fn thunder_bounds(&self) -> Aabb {
        self.bounds()
    }
    /// `Entity.ignoreExplosion`: the blast neither hurts nor pushes it. Also
    /// answers for what `getEntities(source, box)` never lists: the exploding
    /// mob, and the dead.
    fn ignores_explosion(&self, _blast: &Blast) -> bool {
        false
    }
    /// Whether a burst of wind moves it: what `Entity.push` moves, unless the
    /// kind says otherwise.
    fn pushed_by_wind(&self) -> bool {
        self.pushed_by_explosion()
    }
    /// Whether `Entity.push` moves it. An arrow in the ground, a wind charge and
    /// an armour stand (`explosion_breaks_decorations` takes those) stay put.
    fn pushed_by_explosion(&self) -> bool {
        true
    }
    /// The box the blast casts its rays at: `getSeenPercent`.
    fn explosion_box(&self) -> Aabb {
        self.bounds()
    }
    /// How far above its feet `pushFromExplosion` aims: `getEyeHeight`, which
    /// defaults to 0.85 of the box.
    fn explosion_eye_height(&self) -> f64 {
        let b = self.explosion_box();
        f64::from((b.max_position().y - b.min_position().y) as f32 * 0.85f32)
    }
    /// Whether this kind can still be waiting for its tick when a projectile
    /// ticks, though it joined before it (orbs, falling blocks, primed TNT: they
    /// tick between the batches, `game/world/ordered_tick.rs`).
    fn ticks_after_projectiles(&self) -> bool {
        false
    }
    /// Whether a new member of this kind waits for its spawn packet before its first tick.
    fn waits_for_spawn_packet(&self) -> bool {
        false
    }
    /// `Entity.push` with the explosion's knockback.
    fn push_from_explosion(&mut self, push: Vec3) {
        let v = self.velocity_mut();
        *v = Vec3::new(v.x + push.x, v.y + push.y, v.z + push.z);
    }
    /// `Entity.isSupportedBy`.
    fn is_supported_by(&self, pos: BlockPos) -> bool {
        self.ground_state()
            .is_some_and(|g| g.supporting_block == Some(pos))
    }
    /// `Entity.getEyeY`, where `updateFluidHeightAndDoFluidPushing` reads the
    /// eye's fluid; the top of the box unless a kind sets it.
    fn fluid_eye_y(&self) -> f64 {
        self.bounds().max_position().y
    }
    /// `Entity.updateFluidHeightAndDoFluidPushing`: its water and lava heights
    /// (`(0, 0)` for a kind that is not pushed by fluid), and the current of
    /// each fluid added to its velocity. One body for every kind.
    fn push_by_fluid(
        &mut self,
        world: &sunrise_level::world::World,
        fluids: &sunrise_sim::motion::FluidResources,
        context: &sunrise_world::native_load_context::LoopContext,
    ) -> (f64, f64) {
        if !self.is_pushed_by_fluid() {
            return (0.0, 0.0);
        }
        let (bounds, at, eye) = (self.bounds(), self.position(), self.fluid_eye_y());
        let (water, lava, _) = sunrise_sim::entities::update_entity_fluids(
            bounds,
            at,
            eye,
            self.velocity_mut(),
            world,
            fluids,
            context,
        );
        (water, lava)
    }
    /// `BubbleColumnBlock.entityInside` through `applyEffectsFromBlocks`: a
    /// column lifts or drags every kind alike.
    fn push_by_bubble_column(
        &mut self,
        world: &sunrise_level::world::World,
        registry: &sunrise_world::registry::BlockRegistry,
        facts: &sunrise_items::blocks::BlockFacts,
        collision: &sunrise_world::block_collision::BlockCollisionRegistry<'_>,
    ) -> bool {
        self.push_by_bubble_column_over(world, registry, facts, collision, |_| {})
    }
    /// [`Entity::push_by_bubble_column`], telling `surface` the position of
    /// each column block it lies in (for the bubbles at the top).
    fn push_by_bubble_column_over(
        &mut self,
        world: &sunrise_level::world::World,
        registry: &sunrise_world::registry::BlockRegistry,
        facts: &sunrise_items::blocks::BlockFacts,
        collision: &sunrise_world::block_collision::BlockCollisionRegistry<'_>,
        surface: impl FnMut(BlockPos),
    ) -> bool {
        let bounds = self.bounds();
        sunrise_sim::motion::bubble_column_push_over(
            world,
            registry,
            facts,
            collision,
            bounds,
            self.velocity_mut(),
            surface,
        )
    }
    /// `Entity.getPistonPushReaction`.
    fn get_piston_push_reaction(&self) -> PushReaction {
        PushReaction::PushPull
    }
    /// A boat or minecart: the kinds whose portal pass opens the tick, before
    /// they move (`AbstractMinecart.tick` handles its portal first).
    fn is_vehicle(&self) -> bool {
        false
    }
    /// `Entity.noPhysics` between ticks: its moves pass through the blocks,
    /// and a shulker's lid leaves it be. `stored` is the value a mod wrote for
    /// a kind that keeps no field of its own (`EntityMemory.no_physics`); None
    /// when none was written, so the kind answers its own default.
    fn no_physics(&self, stored: Option<bool>) -> bool {
        stored.unwrap_or(false)
    }
    /// Apply this kind's resolved movement after the invocation's gate is armed.
    /// Every entity kind states how it moves; family ticks resolve through
    /// the same gate in their own movement path.
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3);
    /// Bounds for `AbstractArrow.shouldFall` after a non-SELF piston move.
    fn piston_release_bounds(&self) -> Option<Aabb> {
        None
    }
    fn release_after_piston(&mut self, _no_block_collision: bool) {}
    /// Set the ground flag for a push, or for the carry pass's collider contact.
    /// Terrain support still comes from `Entity.checkSupportingBlock`; a collider
    /// is not `mainSupportingBlockPos`.
    fn set_piston_on_ground(&mut self, on_ground: bool) {
        if let Some(ground) = self.ground_state_mut() {
            ground.on_ground = on_ground;
        }
    }
    /// `Entity.setOnGround` from outside the entity's own move (`/tp`'s
    /// `TeleportCommand.performTeleport`); kinds that keep no ground flag ignore it.
    fn set_on_ground(&mut self, _on_ground: bool) {}
    /// `PistonMovingBlockEntity.causeBounce`: a slime block sets the velocity
    /// along the movement axis.
    fn piston_slime_bounce(&mut self, movement: Direction) {
        let velocity = self.velocity_mut();
        *velocity = piston_push::bounce_velocity(movement, *velocity);
    }
}

/// The `entity_size` answer a kind keeps in its `size_answer` field, for its vanilla size `$vanilla`.
macro_rules! stored_size {
    ($vanilla:expr) => {
        fn vanilla_dimensions(&self) -> Option<[f32; 3]> {
            Some($vanilla(self))
        }
        fn size_answer(&self) -> Option<sunrise_sim::pose::SizeAnswer> {
            self.size_answer
        }
        fn set_size_answer(&mut self, answer: sunrise_sim::pose::SizeAnswer) {
            self.size_answer = Some(answer);
        }
    };
}
#[allow(unused_imports)]
pub(in crate::game) use stored_size;

/// The common retained support owned by moving bodies.
macro_rules! stored_ground {
    () => {
        stored_ground!(fall_distance);
    };
    ($fall_distance:ident) => {
        fn body_fall_distance(&self) -> f64 {
            self.$fall_distance
        }
        fn ground_state(&self) -> Option<&sunrise_world::ground_support::GroundState> {
            Some(&self.ground)
        }
        fn ground_state_mut(&mut self) -> Option<&mut sunrise_world::ground_support::GroundState> {
            Some(&mut self.ground)
        }
    };
}
#[allow(unused_imports)]
pub(in crate::game) use stored_ground;

/// The fields every kind stores under the same names.
macro_rules! stored_motion {
    () => {
        fn id(&self) -> u32 {
            self.id
        }
        fn position(&self) -> Vec3 {
            self.position
        }
        fn position_mut(&mut self) -> &mut Vec3 {
            &mut self.position
        }
        fn previous_position(&self) -> Vec3 {
            self.previous
        }
        fn velocity(&self) -> Vec3 {
            self.velocity
        }
        fn velocity_mut(&mut self) -> &mut Vec3 {
            &mut self.velocity
        }
    };
}
#[allow(unused_imports)]
pub(in crate::game) use stored_motion;

/// Apply the common non-damage `Entity` writes to the state the mover actually owns.
/// An absent field means this mover does not track that effect.
pub fn apply_contact_writes(
    writes: Vec<mod_api::EntityWrite>,
    velocity: &mut Vec3,
    mut fall_distance: Option<&mut f64>,
    mut stuck: Option<&mut Vec3>,
    mut fire_ticks: Option<&mut i32>,
) {
    use mod_api::EntityWrite;
    for write in writes {
        match write {
            EntityWrite::Push([x, y, z]) => *velocity = velocity.add_vector(Vec3::new(x, y, z)),
            EntityWrite::SetMovement([x, y, z]) => *velocity = Vec3::new(x, y, z),
            EntityWrite::StuckIn([x, y, z]) => {
                if let Some(stuck) = stuck.as_deref_mut() {
                    *stuck = Vec3::new(x, y, z);
                }
                if let Some(fall) = fall_distance.as_deref_mut() {
                    *fall = 0.0;
                }
            }
            EntityWrite::ResetFallDistance => {
                if let Some(fall) = fall_distance.as_deref_mut() {
                    *fall = 0.0;
                }
            }
            EntityWrite::SetFireTicks(ticks) => {
                if let Some(fire) = fire_ticks.as_deref_mut() {
                    *fire = ticks;
                }
            }
            EntityWrite::AddEffect { .. } | EntityWrite::Hurt { .. } => {}
        }
    }
}

/// The game's lists of entities, in the order systems walk them. Hands them to
/// `$then`, so one list drives the handles and every typed dispatch.
#[doc(hidden)]
#[macro_export]
macro_rules! entity_lists {
    ($($then:ident)::+, $($args:tt)*) => {
        $($then)::+! {
            ($($args)*)
            Mob(sunrise_sim::mobs::Mob) mobs: mobs,
            Item(sunrise_sim::entities::ItemEntity) dropped: dropped,
            Boat($crate::game::boats_common::Boat) boats: boats,
            Cart($crate::game::minecarts::Minecart) carts: carts,
            ArmorStand(sunrise_sim::entities::ArmorStand) armor_stands: armor_stands,
            Tnt($crate::game::tnt_common::PrimedTnt) tnts: tnts,
            Falling($crate::game::FallingBlock) falling: falling,
            Orb(sunrise_sim::entities::ExperienceOrb) orbs: orbs,
            Projectile(sunrise_sim::projectiles::Projectile) projectiles: creatures.projectiles,
            Rocket($crate::game::fireworks::Rocket) rockets: rockets,
            Trident(sunrise_sim::thrown_trident::ThrownTrident) tridents: tridents,
            General($crate::game::general::GeneralRow) general: resident_entities.general.rows,
        }
    };
}
#[allow(unused_imports)]
pub use crate::entity_lists;

#[doc(hidden)]
#[macro_export]
macro_rules! direct_entity_edit {
    (Projectile) => {
        false
    };
    (Cart) => {
        false
    };
    (Falling) => {
        false
    };
    (Boat) => {
        false
    };
    ($kind:ident) => {
        true
    };
}
#[allow(unused_imports)]
pub use crate::direct_entity_edit;

macro_rules! define_entity_handles {
    (() $($kind:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {
        /// Every list, borrowed apart from the rest of `Game`, for
        /// [`for_each_entity`] while the world is borrowed mutably.
        #[derive(Clone, Copy)]
        pub struct EntityLists<'w> {
            $(pub $field: &'w [$ty],)*
        }

        /// One entity, by list and index.
        #[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
        pub enum EntityHandle {
            $($kind(usize),)*
        }

        impl<'w> EntityLists<'w> {
            /// Merge lists already ordered by join number, taking the lowest head at each
            /// step. The cursors live on the stack.
            pub fn merge_in_join_order(self, include: impl Fn(EntityHandle) -> bool + 'w)
                -> impl Iterator<Item = (EntityHandle, JoinSeq)> + 'w
            {
                let mut cursors = [$(self.$field.len()),*].map(|_| 0usize);
                std::iter::from_fn(move || {
                    let mut best: Option<(usize, EntityHandle, JoinSeq)> = None;
                    let mut list = 0;
                    $(
                        let index = cursors[list];
                        let handle = EntityHandle::$kind(index);
                        if include(handle)
                            && let Some(row) = self.$field.get(index)
                        {
                            let seq = row.join_seq();
                            if best.is_none_or(|(_, _, lowest)| seq < lowest) {
                                best = Some((list, handle, seq));
                            }
                        }
                        list += 1;
                    )*
                    let _ = list;
                    let (list, handle, seq) = best?;
                    cursors[list] += 1;
                    Some((handle, seq))
                })
            }

            /// Visits every entity once in join order. On authoritative lists, tick passes
            /// can reorder storage; report any such list rather than taking it as join order.
            pub fn walk_in_join_order(
                self,
                mut visit: impl FnMut(EntityHandle, JoinSeq),
            ) -> Result<(), JoinOrderError> {
                let mut last = JoinSeq::UNJOINED;
                for (handle, seq) in self.merge_in_join_order(|_| true) {
                    if !seq.is_joined() {
                        return Err(JoinOrderError::Unjoined(handle));
                    }
                    if seq <= last {
                        return Err(JoinOrderError::OutOfOrder(handle));
                    }
                    last = seq;
                    visit(handle, seq);
                }
                Ok(())
            }
        }

impl $crate::game::CommonGame<'_> {
            /// Every entity's handle: lengths read now, nothing allocated, and
            /// no borrow of `Game` held, so the caller can change it mid-walk.
            pub fn entity_handles(&self) -> impl Iterator<Item = EntityHandle> + use<> {
                std::iter::empty()
                    $(.chain((0..self$(.$list)+.len()).map(EntityHandle::$kind)))*
            }
}
    };
}
#[allow(unused_imports)]
pub(crate) use define_entity_handles;

entity_lists!(crate::game::entity::define_entity_handles,);

/// Why [`EntityLists::walk_in_join_order`] stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "the tick order work that reads this has not landed yet"
)]
pub enum JoinOrderError {
    /// Never put in its list through the list's own `push`, so it has no
    /// join number.
    Unjoined(EntityHandle),
    /// Joined before an entity the walk had already visited: its list is out
    /// of join order.
    OutOfOrder(EntityHandle),
}

#[doc(hidden)]
#[macro_export]
macro_rules! entity_access {
    (shared, $list:expr, $index:expr, $entity:ident, $body:expr) => {{
        let $entity = &$list[$index];
        $body
    }};
    (exclusive, $list:expr, $index:expr, $entity:ident, $body:expr) => {
        $list.update($index, |$entity| $body)
    };
}
#[allow(unused_imports)]
pub use crate::entity_access;

#[doc(hidden)]
#[macro_export]
macro_rules! match_entity {
    (($borrow:ident, $entity:ident, $game:expr, $handle:expr, $body:expr)
     $($kind:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {
        match $handle {
            $($crate::game::entity::EntityHandle::$kind(i) => {
                $crate::entity_access!($borrow, $game$(.$list)+, i, $entity, $body)
            })*
        }
    };
}
#[allow(unused_imports)]
pub use crate::match_entity;

/// Runs `$body` with `$entity` bound to the handle's entity, as its own type:
/// one match, each arm compiled for its kind, so nothing goes through a vtable.
#[doc(hidden)]
#[macro_export]
macro_rules! with_entity {
    ($game:expr, $handle:expr, $entity:ident => $body:expr) => {
        $crate::entity_lists!($crate::match_entity, shared, $entity, $game, $handle, $body)
    };
}
#[allow(unused_imports)]
pub use crate::with_entity;

#[path = "entity/list_borrows.rs"]
pub mod list_borrows;
#[allow(unused_imports)]
pub use list_borrows::entity_lists_of;
#[allow(unused_imports)]
pub use list_borrows::{borrow_level_list, borrow_lists};
#[doc(hidden)]
#[macro_export]
macro_rules! walk_lists {
    (($lists:expr, $entity:ident, $body:block)
     $($kind:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {
        $(for $entity in $lists.$field $body)*
    };
}
#[allow(unused_imports)]
pub use crate::walk_lists;

/// Runs `$body` for every entity, with `$entity` as a reference to its own
/// type: one loop per list, each compiled for its kind, so nothing allocates
/// or goes through a vtable and a check a kind answers with a constant folds
/// away. A kind added to `entity_lists!` joins every walk.
#[doc(hidden)]
#[macro_export]
macro_rules! for_each_entity {
    ($lists:expr, $entity:ident => $body:block) => {
        $crate::entity_lists!($crate::walk_lists, $lists, $entity, $body)
    };
}
#[allow(unused_imports)]
pub use crate::for_each_entity;

/// [`with_entity`], with the entity mutable.
#[doc(hidden)]
#[macro_export]
macro_rules! with_entity_mut {
    ($game:expr, $handle:expr, $entity:ident => $body:expr) => {
        $crate::entity_lists!(
            $crate::match_entity,
            exclusive,
            $entity,
            $game,
            $handle,
            $body
        )
    };
}
#[allow(unused_imports)]
pub use crate::with_entity_mut;

impl Entity for Mob {
    stored_ground!();

    fn fall_distance(&self) -> f64 {
        self.fall_distance
    }
    fn reset_fall_distance(&mut self) {
        self.fall_distance = 0.0;
    }
    fn support_collision(
        &self,
        tags: &sunrise_items::items::EntityTypeTags,
    ) -> (CollisionContext<'_>, Option<PowderSnowSubject<'_>>) {
        let (context, subject) = Mob::support_collision(self, Some(tags));
        (context, Some(subject))
    }
    /// `Entity.noPhysics`: written through the mod API; no mob tick rewrites it
    /// (a vex sets it only for the length of its own, custom travel). Its own
    /// field is the write's store, so `stored` is None for a mob.
    fn no_physics(&self, _stored: Option<bool>) -> bool {
        self.no_physics
    }
    /// `Entity.move(MoverType.PISTON, ...)` ends in `setPos`, which `Shulker`
    /// overrides to snap to the block centre.
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let moved = self.position.add_vector(movement);
        if self.species() == Species::Shulker {
            sunrise_sim::mobs::shulker::client_set_pos(self, moved.x, moved.y, moved.z);
        } else {
            self.position = moved;
        }
    }
    fn rotation(&self) -> [f32; 2] {
        [self.yaw, self.head_pitch]
    }
    fn previous_rotation(&self) -> [f32; 2] {
        [self.previous_yaw, self.previous_head_pitch]
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.yaw = yaw;
        self.body_yaw = yaw;
        self.head_yaw = yaw;
        self.head_pitch = pitch.clamp(-90.0, 90.0);
        self.previous_yaw = self.yaw;
        self.previous_head_pitch = self.head_pitch;
    }
    fn native_type(&self) -> Cow<'_, str> {
        super::native_hooks::mob_kind(self).into()
    }
    stored_motion!();
    /// `PersistentEntitySectionManager.Callback.onRemove`: discard stops
    /// section enumeration immediately, before the entity list is swept.
    fn selector_target(&self) -> Option<Target> {
        (!self.removed && self.death_time == 0).then_some(Target::Mob(self.id))
    }
    fn bounds(&self) -> Aabb {
        Mob::bounds(self)
    }
    fn get_step_height(&self) -> f32 {
        self.kind.step_height as f32
    }
    fn is_alive(&self) -> bool {
        !self.removed && self.alive()
    }
    fn is_removed(&self) -> bool {
        self.removed
    }
    fn is_pickable(&self) -> bool {
        !self.removed && !(&*self.kind.id == "minecraft:player" && self.extra.spectator)
    }
    fn is_living(&self) -> bool {
        true
    }
    /// `Bat.isIgnoringBlockTriggers`.
    fn is_ignoring_block_triggers(&self) -> bool {
        self.species() == Species::Bat
    }
    /// `Warden.ignoreExplosion`: while it digs or emerges.
    fn ignores_explosion(&self, blast: &Blast) -> bool {
        !self.alive()
            || blast.spared == Some(self.id)
            || (self.species() == Species::Warden
                && sunrise_sim::mobs::warden::digging_or_emerging(self))
    }
    /// `getEyeHeight` of the mob as it stands: a baby's, a slime's or a
    /// phantom's own scaled eye height, not the adult kind's. The blast's box
    /// is its real `bounds`, so it needs no override.
    fn explosion_eye_height(&self) -> f64 {
        self.eye_height()
    }
    /// `Entity.push` sets `needsSync`.
    fn push_from_explosion(&mut self, push: Vec3) {
        self.velocity = Vec3::new(
            self.velocity.x + push.x,
            self.velocity.y + push.y,
            self.velocity.z + push.z,
        );
        self.needs_sync = true;
    }
    fn thunder_hit(&self) -> ThunderHit {
        if self.alive() {
            ThunderHit::Mob
        } else {
            ThunderHit::Untouched
        }
    }
    /// The kind's own size, as the strike has always read it.
    fn thunder_bounds(&self) -> Aabb {
        let (p, w) = (self.position, self.kind.width / 2.0);
        Aabb::new(
            Vec3::new(p.x - w, p.y, p.z - w),
            Vec3::new(p.x + w, p.y + self.kind.height, p.z + w),
        )
    }
}

impl Entity for ItemEntity {
    stored_size!(|_: &Self| ItemEntity::DIMENSIONS);
    stored_ground!();
    fn fall_distance(&self) -> f64 {
        self.fall_distance
    }
    fn support_collision(
        &self,
        _tags: &sunrise_items::items::EntityTypeTags,
    ) -> (CollisionContext<'_>, Option<PowderSnowSubject<'_>>) {
        (
            self.collision_context(),
            Some(PowderSnowSubject {
                fall_distance: self.fall_distance,
                ..Default::default()
            }),
        )
    }
    fn reset_fall_distance(&mut self) {
        self.fall_distance = 0.0;
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let position = self.position_mut();
        *position = position.add_vector(movement);
    }

    fn rotation(&self) -> [f32; 2] {
        self.rotation
    }
    fn previous_rotation(&self) -> [f32; 2] {
        self.rotation
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.rotation = [yaw, pitch.clamp(-90.0, 90.0)];
    }
    fn fluid_eye_y(&self) -> f64 {
        ItemEntity::fluid_eye_y(self)
    }
    fn native_type(&self) -> Cow<'_, str> {
        "minecraft:item".into()
    }
    stored_motion!();
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Item(self.id))
    }
    fn thunder_hit(&self) -> ThunderHit {
        ThunderHit::Item
    }
    fn ignores_explosion(&self, blast: &Blast) -> bool {
        blast.items_ignore
    }
    fn explosion_eye_height(&self) -> f64 {
        Self::EYE_HEIGHT
    }
    fn bounds(&self) -> Aabb {
        ItemEntity::bounds(self)
    }
    fn piston_slime_bounce(&mut self, movement: Direction) {
        ItemEntity::piston_slime_bounce(self, movement);
    }
}

impl Entity for ExperienceOrb {
    stored_size!(|_: &Self| ExperienceOrb::DIMENSIONS);
    stored_ground!();
    fn reset_fall_distance(&mut self) {
        // An orb keeps no fall distance here.
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let position = self.position_mut();
        *position = position.add_vector(movement);
    }

    fn rotation(&self) -> [f32; 2] {
        self.rotation
    }
    fn previous_rotation(&self) -> [f32; 2] {
        self.rotation
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.rotation = [yaw, pitch.clamp(-90.0, 90.0)];
    }
    fn ticks_after_projectiles(&self) -> bool {
        true
    }
    fn native_type(&self) -> Cow<'_, str> {
        "minecraft:experience_orb".into()
    }
    stored_motion!();
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Orb(self.id))
    }
    fn thunder_hit(&self) -> ThunderHit {
        ThunderHit::Orb
    }
    fn explosion_eye_height(&self) -> f64 {
        f64::from(0.5f32 * 0.85f32)
    }
    fn bounds(&self) -> Aabb {
        ExperienceOrb::bounds(self)
    }
    fn piston_slime_bounce(&mut self, movement: Direction) {
        self.velocity = piston_push::bounce_velocity(movement, self.velocity);
        self.ground.on_ground = false;
        self.ground.clear_support();
    }
}

impl Entity for Boat {
    stored_size!(|_: &Self| Boat::DIMENSIONS);
    stored_ground!();
    fn reset_fall_distance(&mut self) {
        // A boat keeps no fall distance here.
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let position = self.position_mut();
        *position = position.add_vector(movement);
    }

    fn rotation(&self) -> [f32; 2] {
        [self.yaw, self.pitch]
    }
    fn previous_rotation(&self) -> [f32; 2] {
        [self.previous_yaw, self.pitch]
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.yaw = yaw;
        self.pitch = pitch.clamp(-90.0, 90.0);
        self.previous_yaw = self.yaw;
    }
    fn native_type(&self) -> Cow<'_, str> {
        self.entity_type_name().into()
    }
    stored_motion!();
    fn is_vehicle(&self) -> bool {
        true
    }
    /// `Boat.getEyeY`: the top of the 0.5625 box.
    fn fluid_eye_y(&self) -> f64 {
        self.position.y + super::boats_common::HEIGHT
    }
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Boat(self.id))
    }
    fn thunder_hit(&self) -> ThunderHit {
        ThunderHit::Vehicle
    }
    fn ignores_explosion(&self, blast: &Blast) -> bool {
        blast.vehicles_ignore
    }
    /// `EntityType.OAK_BOAT & co`: the full 0.5625.
    fn explosion_eye_height(&self) -> f64 {
        0.5625
    }
    fn bounds(&self) -> Aabb {
        Boat::bounds(self)
    }
    fn is_pickable(&self) -> bool {
        true
    }
}

impl Entity for Minecart {
    stored_size!(|_: &Self| Minecart::DIMENSIONS);
    stored_ground!();
    fn fall_distance(&self) -> f64 {
        self.fall_distance
    }
    fn reset_fall_distance(&mut self) {
        self.fall_distance = 0.0;
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let position = self.position_mut();
        *position = position.add_vector(movement);
    }

    fn rotation(&self) -> [f32; 2] {
        [self.yaw, self.pitch]
    }
    fn previous_rotation(&self) -> [f32; 2] {
        [self.previous_yaw, self.previous_pitch]
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.yaw = yaw;
        self.pitch = pitch.clamp(-90.0, 90.0);
        self.previous_yaw = self.yaw;
        self.previous_pitch = self.pitch;
    }
    fn native_type(&self) -> Cow<'_, str> {
        self.kind.id().into()
    }
    stored_motion!();
    fn is_vehicle(&self) -> bool {
        true
    }
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Cart(self.id))
    }
    fn thunder_hit(&self) -> ThunderHit {
        ThunderHit::Vehicle
    }
    fn ignores_explosion(&self, blast: &Blast) -> bool {
        blast.vehicles_ignore
    }
    fn bounds(&self) -> Aabb {
        Minecart::bounds(self)
    }
    fn is_pickable(&self) -> bool {
        true
    }
    /// `tick_blocks` sweeps minecarts along with the players.
    fn sweeps_blocks_inside(&self) -> bool {
        false
    }
}

/// Width and height `EntityType.ARMOR_STAND` gives a stand.
pub fn armor_stand_size(stand: &ArmorStand) -> (f64, f64) {
    if stand.marker {
        (0.0, 0.0)
    } else if stand.small {
        (0.25, f64::from(0.9875f32))
    } else {
        (0.5, f64::from(1.975f32))
    }
}

impl Entity for ArmorStand {
    stored_size!(ArmorStand::default_dimensions);
    stored_ground!(enchantment_fall_distance);
    fn support_collision(
        &self,
        _tags: &sunrise_items::items::EntityTypeTags,
    ) -> (CollisionContext<'_>, Option<PowderSnowSubject<'_>>) {
        let (context, subject) = ArmorStand::support_collision(self);
        (context, Some(subject))
    }
    fn reset_fall_distance(&mut self) {
        self.enchantment_fall_distance = 0.0;
    }
    /// `LivingEntity.isAlive`: health above 0 (removal is the level's).
    fn is_alive(&self) -> bool {
        self.health > 0.0
    }
    fn is_pickable(&self) -> bool {
        !self.marker
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let position = self.position_mut();
        *position = position.add_vector(movement);
    }

    fn rotation(&self) -> [f32; 2] {
        [self.yaw, self.pitch]
    }
    fn previous_rotation(&self) -> [f32; 2] {
        [self.previous_yaw, self.previous_pitch]
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.yaw = yaw;
        // Entity.forceSetRotation and teleportSetPosition call ArmorStand.setYHeadRot.
        self.head_yaw = yaw;
        self.previous_head_yaw = yaw;
        self.previous_body_yaw = yaw;
        self.previous_yaw = yaw;
        self.pitch = pitch.clamp(-90.0, 90.0);
        self.previous_pitch = self.pitch;
    }
    fn fluid_eye_y(&self) -> f64 {
        ArmorStand::fluid_eye_y(self)
    }
    fn native_type(&self) -> Cow<'_, str> {
        super::armor_stands_common::TYPE_ID.into()
    }
    stored_motion!();
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Stand(self.id))
    }
    /// `ArmorStand.ignoreExplosion`: a visible stand feels only a burst that
    /// affects block-like entities (`items_ignore` is that flag, negated).
    fn ignores_explosion(&self, blast: &Blast) -> bool {
        self.invisible || blast.items_ignore
    }
    /// `EntityType.ARMOR_STAND`'s size, halved when small; a marker has none.
    fn explosion_box(&self) -> Aabb {
        let (w, h) = armor_stand_size(self);
        let half = w / 2.0;
        let p = self.position;
        Aabb::new(
            Vec3::new(p.x - half, p.y, p.z - half),
            Vec3::new(p.x + half, p.y + h, p.z + half),
        )
    }
    fn explosion_eye_height(&self) -> f64 {
        if self.marker {
            0.0
        } else if self.small {
            f64::from(0.9875f32)
        } else {
            f64::from(1.7775f32)
        }
    }
    /// A burst of wind pushes a stand that a blast only breaks.
    fn pushed_by_wind(&self) -> bool {
        true
    }
    fn bounds(&self) -> Aabb {
        ArmorStand::bounds(self)
    }
    fn is_living(&self) -> bool {
        true
    }
    /// `ArmorStand.isIgnoringBlockTriggers`.
    fn is_ignoring_block_triggers(&self) -> bool {
        self.marker
    }
    /// A NoGravity stand is not swept for the blocks it stands in (plates,
    /// fire): measured beside the real server, it never presses a plate nor
    /// catches fire, and does again once `NoGravity` is cleared.
    fn sweeps_blocks_inside(&self) -> bool {
        !self.no_gravity
    }
    fn pushed_by_explosion(&self) -> bool {
        false
    }
    /// `Entity.noPhysics`: `Marker` or `NoGravity` unless a mod wrote it,
    /// recomputed from them on load because `readAdditionalSaveData` does.
    /// Its own field is the write's store, so `stored` is None for a stand.
    fn no_physics(&self, _stored: Option<bool>) -> bool {
        self.no_physics.unwrap_or(self.marker || self.no_gravity)
    }
    /// `ArmorStand.getPistonPushReaction`: a marker is ignored.
    fn get_piston_push_reaction(&self) -> PushReaction {
        if self.marker {
            PushReaction::IgnoreEntity
        } else {
            PushReaction::PushPull
        }
    }
}

impl Entity for PrimedTnt {
    stored_size!(|_: &Self| PrimedTnt::DIMENSIONS);
    stored_ground!();
    fn support_collision(
        &self,
        _tags: &sunrise_items::items::EntityTypeTags,
    ) -> (CollisionContext<'_>, Option<PowderSnowSubject<'_>>) {
        (self.collision_context(), Some(self.powder_snow_subject()))
    }
    fn reset_fall_distance(&mut self) {
        // Primed TNT keeps no fall distance here.
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let position = self.position_mut();
        *position = position.add_vector(movement);
    }

    fn rotation(&self) -> [f32; 2] {
        self.rotation
    }
    fn previous_rotation(&self) -> [f32; 2] {
        self.rotation
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.rotation = [yaw, pitch.clamp(-90.0, 90.0)];
    }
    fn ticks_after_projectiles(&self) -> bool {
        true
    }
    fn waits_for_spawn_packet(&self) -> bool {
        true
    }
    fn native_type(&self) -> Cow<'_, str> {
        "minecraft:tnt".into()
    }
    stored_motion!();
    /// `EntityTypes.TNT`'s explicit `eyeHeight(0.15F)`. Nothing vanilla reads
    /// for TNT depends on it (only `isEyeInFluid` does), but it is the type's.
    fn fluid_eye_y(&self) -> f64 {
        self.position.y + f64::from(0.15_f32)
    }
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Tnt(self.id))
    }
    /// Pushed from its position, not its eyes.
    fn explosion_eye_height(&self) -> f64 {
        0.0
    }
    fn bounds(&self) -> Aabb {
        PrimedTnt::bounds(self)
    }
    fn is_pickable(&self) -> bool {
        true
    }
    fn piston_slime_bounce(&mut self, movement: Direction) {
        self.velocity = piston_push::bounce_velocity(movement, self.velocity);
        // Leave the ground so the next tick does not apply the on-ground
        // Y * -0.5 friction that cancels the launch.
        self.ground.on_ground = false;
        self.ground.clear_support();
    }
}

impl Entity for FallingBlock {
    stored_size!(|_: &Self| FallingBlock::DIMENSIONS);
    stored_ground!();
    fn fall_distance(&self) -> f64 {
        self.fall_distance
    }
    fn support_collision(
        &self,
        _tags: &sunrise_items::items::EntityTypeTags,
    ) -> (CollisionContext<'_>, Option<PowderSnowSubject<'_>>) {
        (
            sunrise_sim::motion::BodyMotion::entity_context(self.id, self.position),
            Some(PowderSnowSubject {
                fall_distance: self.fall_distance,
                falling_block: true,
                ..Default::default()
            }),
        )
    }
    fn reset_fall_distance(&mut self) {
        self.fall_distance = 0.0;
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let position = self.position_mut();
        *position = position.add_vector(movement);
    }
    fn rotation(&self) -> [f32; 2] {
        self.rotation
    }
    fn previous_rotation(&self) -> [f32; 2] {
        self.rotation
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.rotation = [yaw, pitch.clamp(-90.0, 90.0)];
    }
    fn ticks_after_projectiles(&self) -> bool {
        true
    }
    fn waits_for_spawn_packet(&self) -> bool {
        true
    }
    fn native_type(&self) -> Cow<'_, str> {
        "minecraft:falling_block".into()
    }
    stored_motion!();
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Falling(self.id))
    }
    fn bounds(&self) -> Aabb {
        FallingBlock::bounds(self)
    }
    fn is_pickable(&self) -> bool {
        true
    }
}

impl Entity for Projectile {
    stored_size!(|p: &Self| p.kind.dimensions());
    stored_ground!();
    fn fall_distance(&self) -> f64 {
        self.arrow
            .as_ref()
            .map_or(self.fall_distance, |arrow| arrow.fall_distance)
    }
    fn support_collision(
        &self,
        _tags: &sunrise_items::items::EntityTypeTags,
    ) -> (CollisionContext<'_>, Option<PowderSnowSubject<'_>>) {
        (
            sunrise_sim::motion::BodyMotion::entity_context(self.id, self.position),
            Some(PowderSnowSubject {
                fall_distance: self.fall_distance(),
                ..Default::default()
            }),
        )
    }
    fn reset_fall_distance(&mut self) {
        if let Some(arrow) = self.arrow.as_deref_mut() {
            arrow.fall_distance = 0.0;
        }
    }
    fn rotation(&self) -> [f32; 2] {
        [self.yaw, self.pitch]
    }
    fn previous_rotation(&self) -> [f32; 2] {
        [self.previous_yaw, self.previous_pitch]
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.yaw = yaw;
        self.pitch = pitch.clamp(-90.0, 90.0);
        self.previous_yaw = self.yaw;
        self.previous_pitch = self.pitch;
    }
    /// `Entity.move(MoverType.PISTON)` ends in `setOnGroundWithMovement`.
    fn set_piston_on_ground(&mut self, on_ground: bool) {
        self.ground.on_ground = on_ground;
    }
    fn set_on_ground(&mut self, on_ground: bool) {
        self.ground.on_ground = on_ground;
    }
    fn native_type(&self) -> Cow<'_, str> {
        self.entity_type().into()
    }
    stored_motion!();
    fn selector_target(&self) -> Option<Target> {
        use sunrise_sim::projectiles::ProjectileKind as K;
        // The kinds /data can write (entity_nbt).
        let selectable = self.is_arrow()
            || sunrise_sim::entity_nbt::saved_with_chunk(self)
            || matches!(
                self.kind,
                K::LargeFireball { .. }
                    | K::SmallFireball
                    | K::DragonFireball
                    | K::WindCharge
                    | K::BreezeWindCharge
            );
        selectable.then_some(Target::Projectile(self.id))
    }
    /// `AbstractArrow.push` does nothing in the ground; a wind charge's motion
    /// stays 0.
    fn pushed_by_explosion(&self) -> bool {
        !self.stuck && !self.kind.ignores_blast()
    }
    fn explosion_eye_height(&self) -> f64 {
        self.kind.eye_height()
    }
    fn push_from_explosion(&mut self, push: Vec3) {
        self.velocity = Vec3::new(
            self.velocity.x + push.x,
            self.velocity.y + push.y,
            self.velocity.z + push.z,
        );
        self.needs_sync = true;
    }
    fn bounds(&self) -> Aabb {
        Projectile::bounds(self)
    }
    /// A shulker bullet's, set when it is made, unless a mod wrote one: the
    /// write has no field on this struct and lands in `stored`, so a cleared
    /// flag is what the movement code reads; an arrow's `AbstractArrow.noPhysics`.
    fn no_physics(&self, stored: Option<bool>) -> bool {
        stored.unwrap_or_else(|| {
            matches!(self.kind, ProjectileKind::ShulkerBullet { .. })
                || self.arrow.as_ref().is_some_and(|a| a.no_physics)
        })
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        Projectile::move_by_piston(self, movement);
    }
    fn piston_release_bounds(&self) -> Option<Aabb> {
        Projectile::piston_release_bounds(self)
    }
    fn release_after_piston(&mut self, no_block_collision: bool) {
        Projectile::release_after_piston(self, no_block_collision);
    }
    fn piston_slime_bounce(&mut self, movement: Direction) {
        Projectile::piston_slime_bounce(self, movement);
    }
    fn is_pickable(&self) -> bool {
        super::deflect_common::attackable(self)
    }
}

impl Entity for Rocket {
    stored_size!(|_: &Self| Rocket::DIMENSIONS);
    stored_ground!();
    fn reset_fall_distance(&mut self) {
        // A firework rocket keeps no fall distance here.
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let position = self.position_mut();
        *position = position.add_vector(movement);
    }
    fn rotation(&self) -> [f32; 2] {
        self.rotation
    }
    fn previous_rotation(&self) -> [f32; 2] {
        self.previous_rotation
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.rotation = [yaw, pitch.clamp(-90.0, 90.0)];
        self.previous_rotation = self.rotation;
    }
    fn native_type(&self) -> Cow<'_, str> {
        "minecraft:firework_rocket".into()
    }
    stored_motion!();
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Rocket(self.id))
    }
    /// 0.25 wide, so 0.2125 to the eyes.
    fn explosion_eye_height(&self) -> f64 {
        0.2125
    }
    fn bounds(&self) -> Aabb {
        Rocket::bounds(self)
    }
    /// `FireworkRocketEntity.tick`: boosting a player skips the step.
    fn sweeps_blocks_inside(&self) -> bool {
        !self.attached
    }
}

impl Entity for ThrownTrident {
    stored_size!(|_: &Self| THROWN_TRIDENT_DIMENSIONS);
    stored_ground!();
    fn reset_fall_distance(&mut self) {
        // A thrown trident keeps no fall distance here.
    }
    fn rotation(&self) -> [f32; 2] {
        [self.yaw, self.pitch]
    }
    fn previous_rotation(&self) -> [f32; 2] {
        [self.previous_yaw, self.previous_pitch]
    }
    fn set_rotation(&mut self, [yaw, pitch]: [f32; 2]) {
        let previous = self.rotation();
        let yaw = if yaw.is_finite() { yaw } else { previous[0] };
        let pitch = if pitch.is_nan() { previous[1] } else { pitch };
        self.yaw = yaw;
        self.pitch = pitch.clamp(-90.0, 90.0);
        self.previous_yaw = self.yaw;
        self.previous_pitch = self.pitch;
    }
    fn set_on_ground(&mut self, on_ground: bool) {
        self.ground.on_ground = on_ground;
    }
    fn native_type(&self) -> Cow<'_, str> {
        "minecraft:trident".into()
    }
    stored_motion!();
    fn selector_target(&self) -> Option<Target> {
        Some(Target::Trident(self.id))
    }
    /// Like any arrow (`AbstractArrow.push`) not pushed while in the ground.
    fn pushed_by_explosion(&self) -> bool {
        !self.in_ground
    }
    /// `EntityType.TRIDENT`: eyes at 0.13.
    fn explosion_eye_height(&self) -> f64 {
        f64::from(0.13_f32)
    }
    fn bounds(&self) -> Aabb {
        let p = self.position;
        let vanilla = Aabb::new(
            Vec3::new(p.x - 0.25, p.y, p.z - 0.25),
            Vec3::new(p.x + 0.25, p.y + 0.5, p.z + 0.25),
        );
        sunrise_sim::pose::SizeAnswer::sized_box(
            self.size_answer,
            THROWN_TRIDENT_DIMENSIONS,
            p,
            vanilla,
        )
    }
    /// `AbstractArrow.noPhysics`, set once a loyal trident turns back. Its own
    /// field is the write's store, so `stored` is None for a trident.
    fn no_physics(&self, _stored: Option<bool>) -> bool {
        self.no_physics
    }
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        ThrownTrident::move_by_piston(self, movement);
    }
    fn piston_release_bounds(&self) -> Option<Aabb> {
        ThrownTrident::piston_release_bounds(self)
    }
    fn release_after_piston(&mut self, no_block_collision: bool) {
        ThrownTrident::release_after_piston(self, no_block_collision);
    }
    fn piston_slime_bounce(&mut self, movement: Direction) {
        ThrownTrident::piston_slime_bounce(self, movement);
    }
    /// `GameLevel::tick_blocks` sweeps tridents itself.
    fn sweeps_blocks_inside(&self) -> bool {
        false
    }
}

impl<'w> EntityLists<'w> {
    /// Non-player, non-mob boxes with `Entity.blocksBuilding`. The block level
    /// supplies its minecart snapshot; the mob level reads the live cart boxes.
    pub fn non_mob_building_blocker_boxes(
        self,
        carts: impl Iterator<Item = Aabb> + 'w,
        crystals: &'w [dragon_common::Crystal],
    ) -> impl Iterator<Item = Aabb> + 'w {
        carts
            .chain(
                self.armor_stands
                    .iter()
                    .filter(|s| !s.marker)
                    .map(|s| s.bounds()),
            )
            .chain(self.boats.iter().map(boats_common::Boat::bounds))
            .chain(crystals.iter().map(dragon_common::Crystal::bounds))
            .chain(self.tnts.iter().map(tnt_common::PrimedTnt::bounds))
            .chain(
                self.falling
                    .iter()
                    .filter(|f| f.blocks_building)
                    .map(FallingBlock::bounds),
            )
    }
}
