use crate::game::entity::Entity;
use sunrise_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
};
use sunrise_level::collisions::SceneCollisions;
use sunrise_sim::{entity_rules::EntityRules, mobs::Who};

#[derive(Clone, Debug)]
pub struct PrimedTnt {
    /// Entity-owned registration metadata; content clones are detached.
    pub attachment_lifetime: sunrise_mod_runtime::attachments::AttachmentLifetime,
    /// `Entity.yRot` and `xRot` (`/tp`, `/data get ... Rotation`).
    pub rotation: [f32; 2],
    /// When it joined its list (`sunrise_client::join_seq`); set by the list.
    pub join: sunrise_sim::join_seq::JoinSeq,
    /// The entity id a server tracks it by (0 until it is tracked).
    pub id: u32,
    pub position: Vec3,
    pub previous: Vec3,
    pub velocity: Vec3,
    pub fuse: i32,
    /// `Entity.tickCount`: starts at 0, +1 each entity tick, not saved.
    pub ticks: u32,
    pub ground: sunrise_world::ground_support::GroundState,
    /// `Entity.fallDistance` (`fall_distance` in its save).
    pub fall_distance: f64,
    /// `Entity.stuckSpeedMultiplier`: set by powder snow or a cobweb it is
    /// in, it scales the next move, which then starts from rest.
    pub stuck: Vec3,
    /// `Entity.wasTouchingWater`, from its last immersion update.
    pub in_water: bool,
    pub owner: Option<Who>,
    pub guest: Option<u32>,
    /// `Entity.noGravity` (`NoGravity` in its save).
    pub no_gravity: bool,
    /// `PrimedTnt.explosionPower`: 4 by default, saved as `explosion_power`
    /// and clamped to 0..128 when read back.
    pub power: f32,
    /// `PrimedTnt.teleport`: set on the recreated entity, never saved to NBT.
    pub used_portal: bool,
    /// PrimedTnt.DATA_BLOCK_STATE_ID; None is the default TNT state.
    pub block_state: Option<sunrise_world::registry::BlockStateId>,
    /// What `entity_size` answered at its last size refresh (`EntityEvent.Size`): its box takes
    /// the answer while it holds for its vanilla size.
    pub size_answer: Option<sunrise_sim::pose::SizeAnswer>,
}
impl PrimedTnt {
    /// The live state encoded for each existing save view.
    pub fn block_state_nbt(
        &self,
        registry: &sunrise_world::registry::BlockRegistry,
    ) -> sunrise_core::nbt::Tag {
        self.block_state.map_or_else(
            || sunrise_core::nbt::Tag::String("minecraft:tnt".into()),
            |state| sunrise_sim::entity_nbt::state_to_nbt(registry, state),
        )
    }

    /// `EntityTypes.TNT`'s `sized(0.98F, 0.98F)`, widened from the float.
    pub const SIZE: f64 = 0.98_f32 as f64;
    /// `EntityTypes.TNT.getDimensions()`: width, height and eye height.
    pub(in crate::game) const DIMENSIONS: [f32; 3] = [0.98, 0.98, 0.15];
    pub fn bounds(&self) -> Aabb {
        let h = Self::SIZE / 2.0;
        let vanilla = Aabb::new(
            Vec3::new(self.position.x - h, self.position.y, self.position.z - h),
            Vec3::new(
                self.position.x + h,
                self.position.y + Self::SIZE,
                self.position.z + h,
            ),
        );
        sunrise_sim::pose::SizeAnswer::sized_box(
            self.size_answer,
            Self::DIMENSIONS,
            self.position,
            vanilla,
        )
    }
    /// `Entity.move`: a pending stuck multiplier is used up by the next move,
    /// which starts from rest. Returns it, for a move that scales its step by it.
    pub fn take_stuck(&mut self) -> Option<Vec3> {
        if self.stuck.length_squared() <= 1.0e-7 {
            return None;
        }
        self.velocity = Vec3::ZERO;
        Some(std::mem::replace(&mut self.stuck, Vec3::ZERO))
    }
    /// PowderSnowBlock.getCollisionShape: a top that holds it once it has
    /// fallen more than 2.5 blocks, read with [`Self::collision_context`].
    pub fn powder_snow_subject(&self) -> sunrise_level::collisions::PowderSnowSubject<'static> {
        sunrise_level::collisions::PowderSnowSubject {
            fall_distance: self.fall_distance,
            ..Default::default()
        }
    }
    pub fn collision_context(&self) -> sunrise_world::collision_context::CollisionContext<'static> {
        sunrise_sim::motion::BodyMotion::entity_context(self.id, self.position)
    }
    /// Entity.checkFallDamage after a move that stepped `moved_y`: out of
    /// water, each step down is narrowed to a float before it adds to the
    /// fall; landing forgets it. Returns whether it landed after a fall, when
    /// the caller posts HIT_GROUND.
    pub fn check_fall_damage(&mut self, moved_y: f64) -> bool {
        if !self.in_water && moved_y < 0.0 {
            self.fall_distance -= f64::from(moved_y as f32);
        }
        let landed = self.ground.on_ground && self.fall_distance > 0.0;
        if self.ground.on_ground {
            self.fall_distance = 0.0;
        }
        landed
    }
    /// Entity.applyEffectsFromBlocks for a move from `from` to here
    /// (`original` the step it asked for, when the move was recorded): powder
    /// snow and cobwebs it went through make it stuck, a bubble column lifts or
    /// drags it, and slime it stands on slows it, first. Returns the columns it
    /// is at the top of, which send their surface particles.
    pub fn apply_effects_from_blocks(
        &mut self,
        motion: sunrise_sim::motion::BodyMotion<'_>,
        registry: &sunrise_world::registry::BlockRegistry,
        from: Vec3,
        original: Option<Vec3>,
    ) -> Vec<BlockPos> {
        let (world, facts, collision) = (motion.world, motion.facts, motion.collision);
        let mut tops = Vec::new();
        let stuck = sunrise_sim::falling_block::apply_effects_from_blocks(
            from,
            self.position,
            original,
            &mut self.velocity,
            &mut self.fall_distance,
            &mut Vec::new(),
            |p| match &*facts.get(world.block(p.x, p.y, p.z)).name {
                "minecraft:powder_snow" => sunrise_sim::falling_block::Inside::PowderSnow,
                "minecraft:cobweb" => sunrise_sim::falling_block::Inside::Cobweb,
                _ => sunrise_sim::falling_block::Inside::Nothing,
            },
        );
        if let Some(multiplier) = stuck {
            self.stuck = multiplier;
        }
        if self.push_by_bubble_column_over(world, registry, facts, collision, |pos| tops.push(pos))
        {
            self.fall_distance = 0.0;
        }
        tops
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn tick(
        &mut self,
        world: &sunrise_level::world::World,
        collision: &sunrise_world::block_collision::BlockCollisionRegistry<'_>,
        assets: &sunrise_level::assets::Assets,
        facts: &sunrise_items::blocks::BlockFacts,
        fluids: &sunrise_sim::motion::FluidResources,
        context: &sunrise_world::native_load_context::LoopContext,
        border: Option<&sunrise_core::geometry::border::BorderGeometry>,
        blocks: Option<&sunrise_world::movement_properties::BoundMovementProperties>,
    ) -> sunrise_game_data::Result<Vec<BlockPos>> {
        Ok(self.tick_among(
            world,
            collision,
            assets,
            facts,
            fluids,
            context,
            border,
            blocks,
            None,
            &[],
        ))
    }
    /// Client prediction and isolated physics tests have no server landing effects.
    #[allow(clippy::too_many_arguments)]
    pub fn tick_among(
        &mut self,
        world: &sunrise_level::world::World,
        collision: &sunrise_world::block_collision::BlockCollisionRegistry<'_>,
        assets: &sunrise_level::assets::Assets,
        facts: &sunrise_items::blocks::BlockFacts,
        fluids: &sunrise_sim::motion::FluidResources,
        context: &sunrise_world::native_load_context::LoopContext,
        border: Option<&sunrise_core::geometry::border::BorderGeometry>,
        blocks: Option<&sunrise_world::movement_properties::BoundMovementProperties>,
        side: Option<&dyn sunrise_sim::motion::SideLevel>,
        entities: &[Aabb],
    ) -> Vec<BlockPos> {
        let motion = sunrise_sim::motion::BodyMotion {
            world,
            collision,
            facts,
            blocks,
            context,
            side,
        };
        let (movement, recorded) = self.move_before_fall(motion, border, entities);
        if context.side() == sunrise_world::native_load_context::LoopSide::Server {
            self.check_fall_damage(movement.moved.y);
        }
        self.velocity = self.move_restitution(motion, movement).velocity;
        motion.apply_block_speed(self.position, &mut self.velocity);
        motion.step_on(
            (self.id, "minecraft:tnt"),
            (self.position, self.bounds()),
            &self.ground,
            &mut self.velocity,
        );
        self.finish_tick(motion, assets, fluids, recorded.then_some(movement.wanted))
    }

    pub fn move_before_fall(
        &mut self,
        motion: sunrise_sim::motion::BodyMotion<'_>,
        border: Option<&sunrise_core::geometry::border::BorderGeometry>,
        entities: &[Aabb],
    ) -> (sunrise_sim::motion::BodyMove, bool) {
        let sunrise_sim::motion::BodyMotion {
            world,
            collision,
            context,
            ..
        } = motion;
        // `Entity.tickCount` is raised before the rest of the tick, and is not saved.
        self.ticks = self.ticks.saturating_add(1);
        self.previous = self.position;
        self.velocity.y -= self.get_gravity();
        let wanted = self.velocity;
        // The TNT's Entity.move goes through the move gate (#2414), armed
        // before the stuck multiplier of the move itself; PR 1a records the
        // movement unchanged.
        let mut gate = sunrise_sim::move_gate::armed(
            context,
            self.id,
            "minecraft:tnt",
            sunrise_world::entity_move::MoverType::SelfMovement,
            wanted,
        );
        let mut wanted = gate.delta();
        if let Some(multiplier) = self.take_stuck() {
            wanted = wanted.multiply(multiplier);
        }
        let scene = SceneCollisions {
            world,
            binding: collision,
            subject: Some(self.powder_snow_subject()),
        };
        // Entity.collide: the world border's walls and the entities that
        // `canBeCollidedWith` it (a boat, a shulker) stop it too.
        let border = border.map(|b| (b, [self.position.x, self.position.z]));
        let mut collisions = sunrise_level::collisions::EntityCollisions {
            blocks: scene,
            entities,
            border,
        };
        let moved = sunrise_sim::move_gate::entities_resolve_with_context(
            &mut gate,
            &mut collisions,
            self.bounds(),
            wanted,
            0.0,
            self.ground.on_ground,
            self.collision_context(),
        )
        .unwrap_or(Vec3::ZERO);
        let recorded = sunrise_sim::entities::move_applies(wanted, moved);
        if recorded {
            self.position = Vec3::new(
                self.position.x + moved.x,
                self.position.y + moved.y,
                self.position.z + moved.z,
            );
        }
        let authoritative = context.side() == sunrise_world::native_load_context::LoopSide::Server;
        if !gate.skipped() {
            motion.record_move(
                sunrise_sim::motion::BodyCollision {
                    footprint: (self.position, self.bounds()),
                    movement: (wanted, moved),
                    authoritative,
                    context: self.collision_context(),
                    subject: Some(self.powder_snow_subject()),
                },
                &mut self.ground,
            );
        }
        let effect = motion.legacy_contact(self.position, &self.ground);
        (
            sunrise_sim::motion::BodyMove {
                wanted,
                moved,
                effect,
            },
            recorded,
        )
    }

    pub fn move_restitution(
        &self,
        motion: sunrise_sim::motion::BodyMotion<'_>,
        movement: sunrise_sim::motion::BodyMove,
    ) -> sunrise_sim::motion::BodyRestitution {
        motion.restitute(
            (self.id, "minecraft:tnt"),
            movement,
            self.get_gravity(),
            0.98_f32,
            self.velocity,
            self.fall_distance,
        )
    }

    pub fn finish_tick(
        &mut self,
        motion: sunrise_sim::motion::BodyMotion<'_>,
        assets: &sunrise_level::assets::Assets,
        fluids: &sunrise_sim::motion::FluidResources,
        recorded: Option<Vec3>,
    ) -> Vec<BlockPos> {
        let tops = self.apply_effects_from_blocks(motion, assets.registry, self.previous, recorded);
        // Entity.scale(getAirDrag()): 0.98F, widened before it scales motion.
        self.velocity = sunrise_sim::entity_rules::scale_air_drag(self.velocity);
        if self.ground.on_ground {
            self.velocity = Vec3::new(
                self.velocity.x * 0.7,
                self.velocity.y * -0.5,
                self.velocity.z * 0.7,
            );
        }
        self.fuse -= 1;
        if self.fuse > 0 {
            // PrimedTnt updates immersion after moving and decrementing its
            // fuse; water forgets the fall.
            let (water, _) = self.push_by_fluid(motion.world, fluids, motion.context);
            self.in_water = water > 0.0;
            if self.in_water {
                self.fall_distance = 0.0;
            }
        }
        tops
    }
}

pub fn tnt_seat(tnt: Vec3, rider_offset: f64) -> Vec3 {
    Vec3::new(tnt.x, tnt.y + PrimedTnt::SIZE - rider_offset, tnt.z)
}

/// A general rider's seat on primed TNT: no vehicle attachment of its own.
pub fn tnt_general_seat(tnt: Vec3) -> Vec3 {
    tnt_seat(tnt, 0.0)
}

sunrise_sim::impl_joined!(PrimedTnt);
sunrise_mod_runtime::impl_attachment_lifetime!(PrimedTnt);

impl EntityRules for PrimedTnt {
    fn is_no_gravity(&self) -> bool {
        self.no_gravity
    }
    /// `PrimedTnt.getDefaultGravity`.
    fn get_default_gravity(&self) -> f64 {
        0.04
    }
}
