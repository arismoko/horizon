//! `PistonMovingBlockEntity.moveCollidedEntities` and `moveStuckEntities` for
//! the entities the game moves itself: shared geometry, types, dispatchers,
//! and role-neutral helpers; server and client passes live in
//! `piston_push_server.rs` and `piston_push_client.rs`.
#[path = "piston_push/carrying.rs"]
pub mod carrying;
pub use super::move_gate::MoveWho;
use crate::game::collision_source::Mover;
use crate::game::entity::{Entity, EntityHandle, with_entity, with_entity_mut};
pub use sunrise_core::geometry::{Aabb, Direction, Vec3, border::BorderGeometry};
use sunrise_core::position::BlockPos;
use sunrise_level::collisions::{EntityCollisions, SceneCollisions, no_collision_empty};
use sunrise_sim::join_seq::JoinSeq;
use sunrise_sim::mobs::Species;
use sunrise_world::{
    block_collision_scan::BlockCollisionWorld, collision_context::CollisionContext,
    entity_move::MoverType, level::data::PushReaction, level::pistons::MovingPiston,
    move_gate::MoveGate, world_movement::EntityMovementWorld,
};

/// What one piston asks for this tick: where it is, its state, and the progress
/// `moveCollidedEntities` runs at (before the tick stores it).
pub type Push = (BlockPos, MovingPiston, f32);

/// What one shulker box asks for this tick (`moveCollidedEntities`): where it
/// is, its facing as a 3D data value, and the lid box it swept
/// (`Shulker.getProgressDeltaAabb`).
pub type ShulkerPush = ([i32; 3], u8, [f64; 6]);

/// What an opening lid pushes with: `Entity.move`'s mover, and for a
/// shulker's, whom it leaves out.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lid {
    /// `ShulkerBoxBlockEntity.moveCollidedEntities` (`MoverType.SHULKER_BOX`).
    Box,
    /// `Shulker.onPeekAmountChange` of the shulker with this id
    /// (`MoverType.SHULKER`): it leaves out the shulkers, the bodies with
    /// `noPhysics` and those sharing its root vehicle (`isPassengerOfSameVehicle`).
    Shulker(u32),
}

impl Lid {
    pub fn mover(self) -> MoverType {
        match self {
            Lid::Box => MoverType::ShulkerBox,
            Lid::Shulker(_) => MoverType::Shulker,
        }
    }
    pub fn shulker(self) -> Option<u32> {
        match self {
            Lid::Box => None,
            Lid::Shulker(id) => Some(id),
        }
    }
}

/// The first position of each moving piston in `block_ticks.moving_pistons`
/// and in `World::moving_pistons`, taken before a tick's pushes.
#[derive(Default)]
pub struct PistonIndex {
    pub(in crate::game) ticked: sunrise_level::fast_hash::FxHashMap<BlockPos, usize>,
    pub(in crate::game) drawn: sunrise_level::fast_hash::FxHashMap<[i32; 3], usize>,
}

/// `Entity.getStepHeight` of each body, read before the push.
pub struct StepHeights {
    pub player: f32,
    pub guests: std::collections::HashMap<u32, f32>,
}

/// One selected body; the handle is only a hint once a move can reenter the level.
#[derive(Clone, Copy)]
pub struct PushTarget {
    pub hint: MoveWho,
    pub id: u32,
    pub join: Option<JoinSeq>,
}

/// What the push needs from one entity.
#[derive(Clone, Copy)]
pub struct Body {
    pub bounds: Aabb,
    pub position: Vec3,
    pub on_ground: bool,
    pub step_height: f32,
    pub piston_reaction: PushReaction,
    /// Standing on the piston's own block (`Entity.isSupportedBy`).
    pub supported: bool,
}

impl crate::game::CommonGame<'_> {
    /// Index the first position of each moving piston in `block_ticks` and `world`.
    pub fn build_piston_index(&self) -> PistonIndex {
        let mut index = PistonIndex::default();
        for (i, (p, _)) in self.block_ticks.moving_pistons.iter().enumerate() {
            index.ticked.entry(*p).or_insert(i);
        }
        for (i, (p, _)) in self.world.moving_pistons.iter().enumerate() {
            index.drawn.entry(*p).or_insert(i);
        }
        index
    }

    /// The collision view of the moving piston at `pos` takes the progress
    /// its tick gave it. One no longer moving keeps its entry until
    /// `sync_moving_pistons`.
    pub fn advance_piston_collision(&mut self, pos: BlockPos, index: &PistonIndex) {
        let ticked = &self.block_ticks.moving_pistons;
        // The index holds while the pushes leave both lists as they were;
        // otherwise the first match is searched for, as before.
        let found = index
            .ticked
            .get(&pos)
            .filter(|&&i| ticked.get(i).is_some_and(|(p, _)| *p == pos))
            .copied()
            .or_else(|| ticked.iter().position(|(p, _)| *p == pos));
        let Some(state) = found.map(|i| ticked[i].1.shape_state()) else {
            return;
        };
        let at = [pos.x, pos.y, pos.z];
        let drawn = &mut self.world.moving_pistons;
        let slot = index
            .drawn
            .get(&at)
            .filter(|&&i| drawn.get(i).is_some_and(|(p, _)| *p == at))
            .copied()
            .or_else(|| drawn.iter().position(|(p, _)| *p == at));
        if let Some(i) = slot {
            drawn[i].1 = state;
        }
    }

    /// Whether a shulker's lid leaves a body it meets where it is, given the
    /// already-resolved root vehicle ids of the shulker and the target.
    pub fn lid_spares_with_root(
        &self,
        lid: Lid,
        root: Option<u32>,
        target: MoveWho,
        target_root: Option<u32>,
        no_physics: bool,
    ) -> bool {
        lid.shulker().is_some()
            && (no_physics || self.pushed_shulker(target) || root.is_some() && target_root == root)
    }

    /// Whether a pushed body is a shulker, which a shulker's lid leaves out.
    pub(in crate::game) fn pushed_shulker(&self, target: MoveWho) -> bool {
        matches!(target, MoveWho::Entity(EntityHandle::Mob(i)) if self.mobs[i].species() == Species::Shulker)
    }

    /// Whether a body moves with `noPhysics`, which `Entity.move` follows with `setPos` the
    /// whole way. The four kinds whose struct keeps no such field (a shulker bullet among
    /// them) hold a mod's write in `persist`.
    pub fn pushed_no_physics(&self, target: MoveWho) -> bool {
        match target {
            MoveWho::Entity(handle) => with_entity!(self, handle, entity => {
                let stored = self.persist.no_physics.get(&entity.id()).copied();
                entity.no_physics(stored)
            }),
            _ => false,
        }
    }

    /// What a move needs from a listed entity.
    pub fn listed_body(&self, handle: EntityHandle, pos: Option<BlockPos>) -> Body {
        with_entity!(self, handle, entity => Body {
            bounds: Entity::bounds(entity),
            position: entity.position(),
            on_ground: entity.on_ground(),
            step_height: entity.get_step_height(),
            piston_reaction: entity.get_piston_push_reaction(),
            supported: pos.is_some_and(|pos| entity.is_supported_by(pos)),
        })
    }

    /// `AbstractArrow.move` with a mover other than the arrow itself (a
    /// piston, a shulker lid): after `Entity.move`, whether or not it went
    /// anywhere, a stuck arrow or trident with nothing to collide with within
    /// 0.06 starts falling (`shouldFall`, `startFalling`).
    pub fn release_after_block_move(&mut self, target: MoveWho) {
        let MoveWho::Entity(handle) = target else {
            return;
        };
        let Some(bounds) = with_entity!(self, handle, entity => entity.piston_release_bounds())
        else {
            return;
        };
        let no_block_collision = {
            let mut scene = SceneCollisions {
                world: &self.world,
                binding: self.collision,
                subject: None,
            };
            no_collision_empty(&mut scene, bounds)
        };
        with_entity_mut!(self, handle, entity => {
            entity.release_after_piston(no_block_collision)
        });
    }
}

pub trait PistonPushGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// `Entity.move(mover, …)` for the local player in place, dispatched by role.
    fn move_local_player(
        &mut self,
        mover: MoverType,
        movement: Vec3,
        auto_jump: bool,
    ) -> sunrise_game_data::Result<Option<Vec3>> {
        self.side_move_local_player(mover, movement, auto_jump)
    }

    /// Push whatever this tick's moving pistons ran into, dispatched by role.
    fn push_piston_entities(&mut self, pushes: Vec<Push>) {
        self.side_push_piston_entities(pushes);
    }

    /// `Entity.collide`'s `getEntityCollisions` for a body a piston or a
    /// shulker lid pushes.
    fn pushed_entity_colliders(&self, target: MoveWho) -> Vec<Aabb> {
        match target {
            MoveWho::Entity(EntityHandle::Boat(_) | EntityHandle::Cart(_)) => Vec::new(),
            MoveWho::Entity(handle) => {
                let own = with_entity!(self, handle, entity => Entity::bounds(entity));
                let mut boxes = self.entity_collisions(Mover::Passive);
                boxes.retain(|b| *b != own);
                boxes
            }
            _ => Vec::new(),
        }
    }

    fn pushed_living_collision<'__loan>(
        &'__loan self,
        target: MoveWho,
    ) -> Option<(
        CollisionContext<'__loan>,
        sunrise_level::collisions::PowderSnowSubject<'__loan>,
    )>
    where
        '__sg: '__loan,
    {
        match target {
            MoveWho::Entity(EntityHandle::Mob(i)) => {
                Some(self.mobs[i].support_collision(Some(&self.items().entity_type_tags)))
            }
            MoveWho::Entity(EntityHandle::ArmorStand(i)) => {
                Some(self.armor_stands[i].support_collision())
            }
            _ => None,
        }
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> PistonPushGameExt1<'__sg> for G {}

/// The lid's push: the swept box in world space (`Shulker.
/// getProgressDeltaAabb`) and the movement out along the facing, the box's own
/// size plus 0.01 (`ShulkerBoxBlockEntity.moveCollidedEntities`).
pub fn shulker_lid(facing: u8, lid: [f64; 6]) -> (Vec3, Aabb) {
    let direction = Direction::ALL[usize::from(facing) % Direction::ALL.len()];
    let [x, y, z] = direction.normal();
    (
        Vec3::new(
            (lid[3] - lid[0] + 0.01) * f64::from(x),
            (lid[4] - lid[1] + 0.01) * f64::from(y),
            (lid[5] - lid[2] + 0.01) * f64::from(z),
        ),
        Aabb::new(
            Vec3::new(lid[0], lid[1], lid[2]),
            Vec3::new(lid[3], lid[4], lid[5]),
        ),
    )
}

/// The direction of a movement's first non-zero axis, the one `limitPistonMovement` keeps.
pub fn first_axis_direction(movement: Vec3) -> Option<Direction> {
    if movement.x != 0.0 {
        Some(if movement.x > 0.0 {
            Direction::East
        } else {
            Direction::West
        })
    } else if movement.y != 0.0 {
        Some(if movement.y > 0.0 {
            Direction::Up
        } else {
            Direction::Down
        })
    } else if movement.z != 0.0 {
        Some(if movement.z > 0.0 {
            Direction::South
        } else {
            Direction::North
        })
    } else {
        None
    }
}

/// `Entity.move(MoverType.PISTON, movement)` for a pushed body: the movement
/// it makes. The world's blocks stop it, and so does the world border:
/// `Entity.collectCollidersIgnoringWorldBorder` adds the border's walls when
/// the body is inside close to the border, as ordinary movement does. The
/// `gate` is the push's armed move gate (#2414); the resolve goes through it.
pub fn pushed_movement<W>(
    gate: &mut MoveGate,
    blocks: W,
    border: &BorderGeometry,
    at: [f64; 2],
    before: Aabb,
    movement: Vec3,
    step_height: f32,
    on_ground: bool,
    context: Option<CollisionContext<'_>>,
    entities: &[Aabb],
) -> sunrise_game_data::Result<Vec3>
where
    W: BlockCollisionWorld<Error = std::io::Error> + EntityMovementWorld<Error = std::io::Error>,
{
    // Without the entity's own context, the one `resolve` reads at its feet.
    let context = context.unwrap_or_else(|| CollisionContext::position(before.min_position().y));
    let mut collisions = EntityCollisions::with_border(blocks, border, at);
    collisions.entities = entities;
    sunrise_sim::move_gate::entities_resolve_with_context(
        gate,
        &mut collisions,
        before,
        movement,
        step_height,
        on_ground,
        context,
    )
}

use crate::game::side_ops::blocks::BlocksSideGameExt as _;

use crate::game::tables::GameItems as _;

use crate::game::collision_source::CollisionSourceGameExt1 as _;

#[cfg(test)]
pub mod tests {
    use super::*;
    use sunrise_level::collisions::EmptyCollisions;

    /// A border 10 wide around the origin: its wall's inner face is at x = 5.
    fn border() -> BorderGeometry {
        BorderGeometry::new([0.0, 0.0], 10.0, 29_999_984).expect("a border")
    }

    /// `Entity.collectCollidersIgnoringWorldBorder`: the border's walls stop a
    /// pushed body that is inside close to the border.
    #[test]
    fn a_pushed_body_stops_at_the_world_border() {
        // A body 0.6 wide, 0.4 blocks short of the wall, pushed east into it.
        let before = Aabb::new(Vec3::new(4.0, 0.0, 0.0), Vec3::new(4.6, 1.8, 0.6));
        let border = border();
        let moved = pushed_movement(
            &mut MoveGate::new(),
            EmptyCollisions,
            &border,
            [4.3, 0.3],
            before,
            Vec3::new(0.51, 0.0, 0.0),
            0.0,
            false,
            None,
            &[],
        )
        .expect("the push resolves");
        // The wall stops the body 0.4 blocks short of the whole push.
        assert!(
            (moved.x - 0.4).abs() < 1.0e-9,
            "the body was not stopped by the border: {moved:?}"
        );
    }

    /// Away from the border, the same push moves the whole way.
    #[test]
    fn a_pushed_body_moves_whole_away_from_the_border() {
        let before = Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.6, 1.8, 0.6));
        let border = border();
        let moved = pushed_movement(
            &mut MoveGate::new(),
            EmptyCollisions,
            &border,
            [0.3, 0.3],
            before,
            Vec3::new(0.51, 0.0, 0.0),
            0.0,
            false,
            None,
            &[],
        )
        .expect("the push resolves");
        assert!(
            (moved.x - 0.51).abs() < 1.0e-9,
            "the body did not move the whole push: {moved:?}"
        );
    }
}
