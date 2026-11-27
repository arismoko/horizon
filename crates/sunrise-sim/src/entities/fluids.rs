//! Dropped-item, experience-orb and armor-stand immersion and currents through the shared vanilla fluid helpers.
use super::{ArmorStand, ExperienceOrb, ItemEntity};
use crate::mobs::living::Fluids;
use crate::{
    motion::{FluidResources, FluidTags, LAVA, WATER},
    world::World,
};
use sunrise_core::{
    geometry::{Aabb, Direction, Vec3},
    position::BlockPos,
};
use sunrise_world::native_load_context::LoopContext;
use sunrise_world::{
    entity_fluid::{
        CurrentTarget, EntityFluidInteraction, FluidInteractionWorld, FluidMembership,
        FluidSections, FluidTagId,
    },
    fluid::{FluidHeightWorld, FluidKind, FluidState},
    fluid_flow::FluidFlowWorld,
    registry::BlockStateId,
};
type Error = std::convert::Infallible;
struct EntityFluidWorld<'a> {
    bounds: Aabb,
    position: Vec3,
    eye_y: f64,
    velocity: &'a mut Vec3,
    world: &'a World,
    fluids: &'a FluidResources,
    context: &'a LoopContext,
}
/// `Entity.move`'s fall-reset ray along a move from `from`, up to 8 blocks:
/// `ClipContext.Fluid.WATER` (the water's shape) and
/// `ClipContext.Block.FALLDAMAGE_RESETTING` (a block `tagged` says is a reset
/// block clips as a full block). Both shapes are queried for every visited
/// block, the water's first (its first query fixes the shared cached shape);
/// either one hitting ends the ray.
pub fn fall_reset_hit(
    world: &World,
    fluids: &FluidResources,
    tagged: impl Fn(BlockPos) -> bool,
    from: Vec3,
    moved: Vec3,
) -> bool {
    let to = from.add_vector(moved.normalize().scale(moved.length().min(8.0)));
    let hit = sunrise_core::geometry::traverse_blocks(from, to, 1_000_000, |p| {
        let water = water_clip(world, fluids, from, to, p);
        let reset = tagged(p)
            && sunrise_core::geometry::VoxelShape::block()
                .clip(from, to, p)
                .is_some();
        (water || reset).then_some(())
    });
    matches!(hit, Ok(Some(())))
}
/// `ClipContext.Fluid.WATER` in a fall-reset ray through `p`: the water's
/// shape from the shared `FlowingFluid.shapes` cache (the first query of a
/// fluid state fixes its height), as the player's own ray reads it.
pub fn water_clip(
    world: &World,
    fluids: &FluidResources,
    from: Vec3,
    to: Vec3,
    p: BlockPos,
) -> bool {
    let state = fluids.fluid(world.block(p.x, p.y, p.z));
    if !matches!(state.kind(), FluidKind::Water | FluidKind::FlowingWater) {
        return false;
    }
    let mut scene = FlowLevel { world, fluids };
    let Ok(mut shapes) = fluids.shapes.lock() else {
        return false;
    };
    shapes
        .shape(&mut scene, state, p)
        .is_ok_and(|shape| shape.clip(from, to, p).is_some())
}
/// Query budget covering every chunk, section, and block cell inside `bounds`
/// (after `EntityFluidInteraction`'s 0.001 deflation), so legal large boxes
/// such as radius-32 area effect clouds are never truncated.
fn fluid_query_budget(bounds: Aabb) -> usize {
    let deflated = bounds.deflate(Vec3::new(0.001, 0.001, 0.001));
    let a = deflated.min_position();
    let b = deflated.max_position();
    if [a.x, a.y, a.z, b.x, b.y, b.z]
        .iter()
        .any(|v| !v.is_finite())
    {
        return 4096;
    }
    let low = [a.x.floor() as i32, a.y.floor() as i32, a.z.floor() as i32];
    let high = [
        (b.x.ceil() as i32).wrapping_sub(1),
        (b.y.ceil() as i32).wrapping_sub(1),
        (b.z.ceil() as i32).wrapping_sub(1),
    ];
    let span = |lo: i32, hi: i32| -> usize {
        if hi < lo {
            0
        } else {
            usize::try_from(i64::from(hi) - i64::from(lo) + 1).unwrap_or(usize::MAX)
        }
    };
    let cx = span(low[0].wrapping_sub(1) >> 4, high[0].wrapping_add(1) >> 4);
    let cz = span(low[2].wrapping_sub(1) >> 4, high[2].wrapping_add(1) >> 4);
    let sy = span(low[1] >> 4, high[1] >> 4);
    let chunks = cx
        .saturating_mul(cz)
        .saturating_mul(1_usize.saturating_add(sy));
    let cells = span(low[0], high[0])
        .saturating_mul(span(low[1], high[1]))
        .saturating_mul(span(low[2], high[2]));
    chunks.saturating_add(cells).max(4096)
}
/// `Entity.baseTick`'s fluid step for an entity that is pushed by fluid: its
/// immersion heights, then the current of each fluid it sits in, added to
/// `velocity` with vanilla's scales (water 0.014, lava 0.007 fast or
/// 0.0023333 slow). Items, experience orbs and armour stands call it for themselves; a boat
/// calls it from its own tick, before `floatBoat` scales the velocity.
pub fn update_entity_fluids(
    bounds: Aabb,
    position: Vec3,
    eye_y: f64,
    velocity: &mut Vec3,
    world: &World,
    fluids: &FluidResources,
    context: &LoopContext,
) -> (f64, f64, bool) {
    let scan = scan_entity_fluids(bounds, position, eye_y, velocity, world, fluids, context);
    (scan.water, scan.lava, scan.eye_in_water)
}

/// [`update_entity_fluids`] with everything the scan found: the water and lava heights, whether
/// the body touches each, and whether its eye is in each. `in_lava` is the raw contact, before a
/// kind's own `firstTick` rule.
pub fn scan_entity_fluids(
    bounds: Aabb,
    position: Vec3,
    eye_y: f64,
    velocity: &mut Vec3,
    world: &World,
    fluids: &FluidResources,
    context: &LoopContext,
) -> Fluids {
    let mut interaction = EntityFluidInteraction::new([WATER, LAVA]);
    let mut scene = EntityFluidWorld {
        bounds,
        position,
        eye_y,
        velocity,
        world,
        fluids,
        context,
    };
    if interaction
        .update(&mut scene, false, fluid_query_budget(bounds))
        .is_err()
    {
        return Fluids::default();
    }
    let water_height = interaction.fluid_height(&scene, WATER);
    let lava_height = interaction.fluid_height(&scene, LAVA);
    // `updateFluidOnEyes`'s answer, kept by the interaction's trackers:
    // whether the entity's eye is inside the water it scanned.
    let eye_in_water = interaction.is_eye_in_fluid(&FluidTags, WATER);
    let eye_in_lava = interaction.is_eye_in_fluid(&FluidTags, LAVA);
    let _ = interaction.apply_current(&mut scene, WATER, 0.014);
    let _ = interaction.apply_current(
        &mut scene,
        LAVA,
        if world.fast_lava {
            0.007
        } else {
            0.0023333333333333335
        },
    );
    Fluids {
        water: water_height,
        lava: lava_height,
        in_water: water_height > 0.0,
        in_lava: lava_height > 0.0,
        eye_in_water,
        eye_in_lava,
        ..Fluids::default()
    }
}
impl ItemEntity {
    /// `Entity.getEyeY` of an item, sampled by `EntityFluidInteraction.update`.
    pub fn fluid_eye_y(&self) -> f64 {
        self.position.y + Self::EYE_HEIGHT
    }
    /// The fluid push as the game runs it, for the unit tests.
    #[cfg(test)]
    pub(super) fn update_fluids(
        &mut self,
        world: &World,
        fluids: &FluidResources,
        context: &LoopContext,
    ) -> (f64, f64) {
        let (bounds, at, eye) = (self.bounds(), self.position, self.fluid_eye_y());
        let (water, lava, _) =
            update_entity_fluids(bounds, at, eye, &mut self.velocity, world, fluids, context);
        (water, lava)
    }
}
impl ExperienceOrb {
    /// `Entity.updateFluidInteraction`: refreshes fluid immersion over the
    /// orb's box and pushes its velocity by any water or lava current; water
    /// resets its fall, and `Entity.baseTick` halves it in lava (`isInLava`
    /// is false on the orb's first tick, `Entity.firstTick`). Answers
    /// `isEyeInFluid(FluidTags.WATER)` for the eye the scan reads, `0.425`
    /// above the orb's feet, `isInWater`, and `isInLava` (before
    /// `Entity.firstTick` is taken into account).
    pub fn update_fluids(
        &mut self,
        world: &World,
        fluids: &FluidResources,
        context: &LoopContext,
    ) -> (bool, bool, bool) {
        let (water, lava, eye) = update_entity_fluids(
            self.bounds(),
            self.position,
            self.position.y + f64::from(0.425f32),
            &mut self.velocity,
            world,
            fluids,
            context,
        );
        if water > 0.0 {
            self.fall_distance = 0.0;
        }
        // `tick_among` counts this tick first: `tickCount` is 1 on the first.
        if lava > 0.0 && self.ticks > 1 {
            self.fall_distance *= 0.5;
        }
        (eye, water > 0.0, lava > 0.0)
    }
}
impl ArmorStand {
    /// `Entity.getEyeY` of a stand: its eye height by size.
    pub fn fluid_eye_y(&self) -> f64 {
        let eye = if self.small {
            f64::from(0.9875f32 * 0.9)
        } else {
            f64::from(1.7775f32)
        };
        self.position.y + eye
    }
    /// The fluid push as the game runs it, for the unit tests: whether the
    /// stand is in water (`isInWater`).
    #[cfg(test)]
    pub fn update_fluids(
        &mut self,
        world: &World,
        fluids: &FluidResources,
        context: &LoopContext,
    ) -> bool {
        use crate::entity_rules::EntityRules;
        if !self.is_pushed_by_fluid() {
            return false;
        }
        let (bounds, at, eye) = (self.bounds(), self.position, self.fluid_eye_y());
        update_entity_fluids(bounds, at, eye, &mut self.velocity, world, fluids, context).0 > 0.0
    }
}
impl FluidMembership for EntityFluidWorld<'_> {
    fn fluid_in_tag(&self, kind: FluidKind, tag: FluidTagId) -> bool {
        FluidTags.fluid_in_tag(kind, tag)
    }
}
impl CurrentTarget for EntityFluidWorld<'_> {
    type Error = Error;
    fn is_player_class(&self) -> bool {
        false
    }
    fn velocity(&mut self) -> Result<Vec3, Error> {
        Ok(*self.velocity)
    }
    fn add_delta_movement(&mut self, delta: Vec3) -> Result<(), Error> {
        let v = *self.velocity;
        let sum = Vec3::new(v.x + delta.x, v.y + delta.y, v.z + delta.z);
        if delta.is_finite() && sum.is_finite() {
            *self.velocity = sum;
        }
        Ok(())
    }
}
impl FluidInteractionWorld for EntityFluidWorld<'_> {
    type Error = Error;
    type Chunk = ();
    fn interaction_box(&mut self) -> Result<Option<Aabb>, Error> {
        Ok(Some(self.bounds.deflate(Vec3::new(0.001, 0.001, 0.001))))
    }
    fn loaded_chunk(&mut self, x: i32, z: i32) -> Result<Option<()>, Error> {
        Ok(self.world.chunks.contains_key(&(x, z)).then_some(()))
    }
    fn chunk_sections(&mut self, _: ()) -> Result<FluidSections<'_>, Error> {
        let layout = self.world.layout();
        Ok(FluidSections {
            minimum_section_y: layout.min_y() >> 4,
            has_fluid: crate::chunk::every_section(layout),
        })
    }
    fn bounding_box_min_y(&self) -> f64 {
        self.position.y
    }
    fn block_x(&self) -> i32 {
        self.position.x.floor() as i32
    }
    fn eye_y(&mut self) -> Result<f64, Error> {
        Ok(self.eye_y)
    }
    fn block_z(&self) -> i32 {
        self.position.z.floor() as i32
    }
    fn fluid_state(&mut self, p: BlockPos) -> Result<FluidState, Error> {
        Ok(self.fluids.fluid(self.world.block(p.x, p.y, p.z)))
    }
    fn fluid_height(&mut self, f: FluidState, p: BlockPos) -> Result<f32, Error> {
        f.height_at(&mut self.fluid_level(), [p.x, p.y, p.z])
    }
    fn camera_height(&mut self, f: FluidState, p: BlockPos) -> Result<f32, Error> {
        f.camera_height_at(&mut self.fluid_level(), [p.x, p.y, p.z])
    }
    fn flow(&mut self, f: FluidState, p: BlockPos) -> Result<Vec3, Error> {
        crate::motion::pushing_flow(self.context, &mut self.fluid_level(), f, p)
    }
}
impl EntityFluidWorld<'_> {
    fn fluid_level(&self) -> FlowLevel<'_> {
        FlowLevel {
            world: self.world,
            fluids: self.fluids,
        }
    }
}
/// `FluidState.getFlow` at `pos` as it pushes an entity there: vanilla's vector, then the
/// `fluid_flow` hook. Zero where no fluid flows.
pub fn fluid_flow(
    world: &World,
    fluids: &FluidResources,
    context: &LoopContext,
    pos: BlockPos,
) -> Vec3 {
    let state = fluids.fluid(world.block(pos.x, pos.y, pos.z));
    let Ok(flow) =
        crate::motion::pushing_flow(context, &mut FlowLevel { world, fluids }, state, pos);
    flow
}
/// `Entity.checkInsideBlocks` intersects the movement with the fluid's
/// current AABB, independently of the block's inside-collision shape.
pub fn swept_fluid_contact(
    world: &World,
    fluids: &FluidResources,
    pos: BlockPos,
    bounds: Aabb,
    delta: Vec3,
) -> bool {
    let fluid = fluids.fluid(world.block(pos.x, pos.y, pos.z));
    if fluid.kind() == FluidKind::Empty {
        return false;
    }
    let height = fluid
        .height_at(&mut FlowLevel { world, fluids }, [pos.x, pos.y, pos.z])
        .unwrap_or_else(|never| match never {});
    let (x, y, z) = (f64::from(pos.x), f64::from(pos.y), f64::from(pos.z));
    let shape = Aabb::new(
        Vec3::new(x, y, z),
        Vec3::new(x + 1.0, y + f64::from(height), z + 1.0),
    );
    bounds.collided_along_vector(delta, &[shape])
}

/// The blocks a fluid reads in a loaded world.
struct FlowLevel<'a> {
    world: &'a World,
    fluids: &'a FluidResources,
}
impl FluidHeightWorld for FlowLevel<'_> {
    type Error = Error;
    fn fluid(&mut self, p: [i32; 3]) -> Result<FluidState, Error> {
        Ok(self.fluids.fluid(self.world.block(p[0], p[1], p[2])))
    }
    fn sturdy_down(&mut self, p: [i32; 3]) -> Result<bool, Error> {
        Ok(self
            .fluids
            .sturdy(self.world.block(p[0], p[1], p[2]), Direction::Down))
    }
}

impl FluidFlowWorld for FlowLevel<'_> {
    type Error = Error;
    type Block = BlockStateId;
    fn fluid(&mut self, p: BlockPos) -> Result<FluidState, Error> {
        Ok(self.fluids.fluid(self.world.block(p.x, p.y, p.z)))
    }
    fn block(&mut self, p: BlockPos) -> Result<BlockStateId, Error> {
        Ok(self.world.block(p.x, p.y, p.z))
    }
    fn blocks_fluid_flow(&self, s: BlockStateId) -> bool {
        self.fluids.blocks_flow(s)
    }
    fn is_ice_class(&self, s: BlockStateId) -> bool {
        self.fluids.ice(s)
    }
    fn is_face_sturdy(
        &mut self,
        s: BlockStateId,
        _: BlockPos,
        d: Direction,
    ) -> Result<bool, Error> {
        Ok(self.fluids.sturdy(s, d))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// `FluidState.getFlow`: water running out of a pool flows away from it,
    /// while the pool itself, its edge on a solid floor and the air beside it
    /// have no current.
    #[test]
    #[ignore = "daily"]
    fn fluid_flow_runs_away_from_a_pool_and_is_zero_in_still_water() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            crate::test_fixtures::report_missing();
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let facts = crate::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let fluids = FluidResources::new(&assets, &facts).unwrap();
        let mut world = World::test_scene_with_placements(&assets, &[], 3).unwrap();
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        let water = assets.registry.default_state("minecraft:water").unwrap();
        let mut state = assets.registry.state(water).unwrap().clone();
        state.properties.insert("level".into(), "1".into());
        let flowing = assets.registry.resolve(&state).unwrap();
        for x in 16..=22 {
            for z in 16..=20 {
                world.set(x, 3, z, stone).unwrap();
            }
        }
        for x in 17..=19 {
            for z in 17..=19 {
                world.set(x, 4, z, water).unwrap();
            }
        }
        world.set(20, 4, 18, flowing).unwrap();
        // A level-1 flow beside sources: the source is 1/9 higher to the west,
        // the air to the east on stone gives nothing, so the current is +x.
        let context = crate::test_resources::server_loop_context();
        assert_eq!(
            fluid_flow(&world, &fluids, context, BlockPos::new(20, 4, 18)),
            Vec3::new(1.0, 0.0, 0.0)
        );
        for [x, y, z] in [[18, 4, 18], [17, 4, 17], [21, 4, 18]] {
            assert_eq!(
                fluid_flow(&world, &fluids, context, BlockPos::new(x, y, z)),
                Vec3::ZERO,
                "{x}, {y}, {z}"
            );
        }
    }
    #[test]
    #[ignore = "daily"]
    fn items_follow_flowing_water_but_still_water_has_no_horizontal_push() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            crate::test_fixtures::report_missing();
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let facts = crate::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let fluids = FluidResources::new(&assets, &facts).unwrap();
        let collision = crate::collisions::load(&root, assets.registry).unwrap();
        let mut world = World::test_scene_with_placements(&assets, &[], 3).unwrap();
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        let water = assets.registry.default_state("minecraft:water").unwrap();
        let mut state = assets.registry.state(water).unwrap().clone();
        state.properties.insert("level".into(), "1".into());
        let flowing = assets.registry.resolve(&state).unwrap();
        for x in 19..=21 {
            for z in 19..=21 {
                world.set(x, 3, z, stone).unwrap();
                world.set(x, 4, z, water).unwrap();
            }
        }
        let at = Vec3::new(20.5, 4.0, 20.5);
        let item = ItemEntity {
            size_answer: None,
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            random: sunrise_core::random::LegacyRandom::new(0),
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            id: 1,
            health: 5,
            fire_ticks: 0,
            stack: crate::items::ItemStack {
                item: "minecraft:sugar_cane".into(),
                count: 1,
                damage: 0,
                extra: None,
            },
            thrower: None,
            target: None,
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            ground: Default::default(),
            vertical_collision: false,
            age: 0,
            stack_revision: 0,
            lifespan: ItemEntity::LIFETIME,
            pickup_delay: 0,
            bob_offset: 0.0,
            ticks: 0,
            needs_sync: false,
            stuck: Vec3::ZERO,
            was_touching_water: false,
        };
        let mut still = item.clone();
        assert!(still.tick(
            &world,
            assets.registry,
            &facts,
            &collision,
            false,
            &fluids,
            &[],
            None,
            None
        ));
        assert_eq!(still.position.x, at.x);
        assert_eq!(still.velocity.x, 0.0);
        world.set(21, 4, 20, flowing).unwrap();
        let mut carried = item;
        assert!(carried.tick(
            &world,
            assets.registry,
            &facts,
            &collision,
            false,
            &fluids,
            &[],
            None,
            None
        ));
        // First push is damped by buoyancy before motion, second follows drag.
        // `setUnderwaterMovement` and the air drag are floats (0.99F, 0.98F).
        let (water, air) = (f64::from(0.99_f32), f64::from(0.98_f32));
        assert!((carried.position.x - at.x - 0.014 * water).abs() < 1e-12);
        assert!((carried.velocity.x - (0.014 * water * air + 0.014)).abs() < 1e-12);
        for _ in 0..20 {
            carried.tick(
                &world,
                assets.registry,
                &facts,
                &collision,
                false,
                &fluids,
                &[],
                None,
                None,
            );
        }
        assert!(carried.position.x > at.x + 0.5);
    }

    /// `Entity.baseTick` pushes an experience orb by the current it sits in: an
    /// orb in flowing water gains velocity along the flow and moves, while an orb
    /// in still source water does not gain horizontal velocity (#7442).
    #[test]
    #[ignore = "daily"]
    fn experience_orbs_follow_flowing_water_but_still_water_has_no_horizontal_push() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            crate::test_fixtures::report_missing();
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let facts = crate::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let fluids = FluidResources::new(&assets, &facts).unwrap();
        let collision = crate::collisions::load(&root, assets.registry).unwrap();
        let mut world = World::test_scene_with_placements(&assets, &[], 3).unwrap();
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        let water = assets.registry.default_state("minecraft:water").unwrap();
        let mut state = assets.registry.state(water).unwrap().clone();
        state.properties.insert("level".into(), "1".into());
        let flowing = assets.registry.resolve(&state).unwrap();
        for x in 19..=21 {
            for z in 19..=21 {
                world.set(x, 3, z, stone).unwrap();
                world.set(x, 4, z, water).unwrap();
            }
        }
        let at = Vec3::new(20.5, 4.0, 20.5);
        let orb = ExperienceOrb {
            size_answer: None,
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            id: 1,
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            value: 1,
            age: 0,
            ground: Default::default(),
            count: 1,
            ticks: 0,
            health: 5,
            air: 300,
            following: None,
            fire_ticks: 0,
        };
        let mut still = orb.clone();
        assert!(still.tick(
            &world,
            &facts,
            &collision,
            None,
            true,
            &mut || 0.0,
            None,
            &fluids,
            crate::test_resources::server_loop_context(),
            None,
            crate::motion::dry_side(),
            &mut |_, _| {},
        ));
        assert_eq!(still.position.x, at.x);
        assert_eq!(still.velocity.x, 0.0);
        world.set(21, 4, 20, flowing).unwrap();
        let mut carried = orb;
        assert!(carried.tick(
            &world,
            &facts,
            &collision,
            None,
            true,
            &mut || 0.0,
            None,
            &fluids,
            crate::test_resources::server_loop_context(),
            None,
            crate::motion::dry_side(),
            &mut |_, _| {},
        ));
        // Current pushes the orb, then underwater and air drag each widen
        // their float constants before multiplying the double motion.
        assert!((carried.position.x - at.x - 0.014 * f64::from(0.99_f32)).abs() < 1e-12);
        assert!(
            (carried.velocity.x - 0.014 * f64::from(0.99_f32) * f64::from(0.98_f32)).abs() < 1e-12
        );
        for _ in 0..20 {
            carried.tick(
                &world,
                &facts,
                &collision,
                None,
                true,
                &mut || 0.0,
                None,
                &fluids,
                crate::test_resources::server_loop_context(),
                None,
                crate::motion::dry_side(),
                &mut |_, _| {},
            );
        }
        assert!(carried.position.x > at.x + 0.5);
    }

    /// `ExperienceOrb.tick`'s water branch reads `isEyeInFluid(FluidTags.WATER)`,
    /// not the feet's block: an orb whose feet hang in the air under the
    /// surface, with its eye in the water, floats (`setUnderwaterMovement`)
    /// instead of falling.
    #[test]
    #[ignore = "daily"]
    fn an_orb_whose_eye_dips_into_water_floats_with_its_feet_in_air() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            crate::test_fixtures::report_missing();
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let facts = crate::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let fluids = FluidResources::new(&assets, &facts).unwrap();
        let collision = crate::collisions::load(&root, assets.registry).unwrap();
        let mut world = World::test_scene_with_placements(&assets, &[], 3).unwrap();
        let water = assets.registry.default_state("minecraft:water").unwrap();
        for x in 19..=21 {
            for z in 19..=21 {
                world.set(x, 5, z, water).unwrap();
            }
        }
        let at = Vec3::new(20.5, 4.6, 20.5);
        let mut orb = ExperienceOrb {
            size_answer: None,
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            id: 1,
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            value: 1,
            age: 0,
            ground: Default::default(),
            count: 1,
            ticks: 0,
            health: 5,
            air: 300,
            following: None,
            fire_ticks: 0,
        };
        assert!(orb.tick(
            &world,
            &facts,
            &collision,
            None,
            true,
            &mut || 0.0,
            None,
            &fluids,
            crate::test_resources::server_loop_context(),
            None,
            crate::motion::dry_side(),
            &mut |_, _| {},
        ));
        assert!(
            orb.velocity.y > 0.0,
            "the eye in the water floats the orb, got {}",
            orb.velocity.y
        );
    }

    /// `Entity.baseTick` pushes a boat by the current it sits in: a boat box
    /// in water[level=1..7] gains velocity along the flow, and the same box in
    /// still source water does not. Vanilla's boat tick runs this before
    /// `floatBoat` scales the velocity, which is what `game::boats` does.
    #[test]
    #[ignore = "daily"]
    fn a_boat_box_is_pushed_by_the_current_it_sits_in() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            crate::test_fixtures::report_missing();
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let facts = crate::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let fluids = FluidResources::new(&assets, &facts).unwrap();
        let mut world = World::test_scene_with_placements(&assets, &[], 3).unwrap();
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        let water = assets.registry.default_state("minecraft:water").unwrap();
        let mut state = assets.registry.state(water).unwrap().clone();
        state.properties.insert("level".into(), "1".into());
        let flowing = assets.registry.resolve(&state).unwrap();
        for x in 19..=21 {
            for z in 19..=21 {
                world.set(x, 3, z, stone).unwrap();
                world.set(x, 4, z, water).unwrap();
            }
        }
        // Boat dimensions: 1.375 wide, 0.5625 tall, eye at its top.
        let at = Vec3::new(20.5, 4.0, 20.5);
        let half = 1.375 / 2.0;
        let bounds = Aabb::new(
            Vec3::new(at.x - half, at.y, at.z - half),
            Vec3::new(at.x + half, at.y + 0.5625, at.z + half),
        );
        let mut still = Vec3::ZERO;
        update_entity_fluids(
            bounds,
            at,
            at.y + 0.5625,
            &mut still,
            &world,
            &fluids,
            crate::test_resources::server_loop_context(),
        );
        assert_eq!(still.x, 0.0, "source water has no current");
        world.set(21, 4, 20, flowing).unwrap();
        let mut carried = Vec3::ZERO;
        update_entity_fluids(
            bounds,
            at,
            at.y + 0.5625,
            &mut carried,
            &world,
            &fluids,
            crate::test_resources::server_loop_context(),
        );
        assert!(
            carried.x > 0.0,
            "the flow pushes the boat along +x, got {}",
            carried.x
        );
    }

    #[test]
    #[ignore = "daily"]
    fn armor_stands_follow_flowing_water_but_markers_and_still_water_stay_in_place() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/sunrise-client")
            .join(sunrise_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            crate::test_fixtures::report_missing();
            return;
        }
        let assets = crate::assets::Assets::load(&root).unwrap();
        let facts = crate::blocks::BlockFacts::load(&root, assets.registry).unwrap();
        let fluids = FluidResources::new(&assets, &facts).unwrap();
        let collision = crate::collisions::load(&root, assets.registry).unwrap();
        let mut world = World::test_scene_with_placements(&assets, &[], 3).unwrap();
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        let water = assets.registry.default_state("minecraft:water").unwrap();
        let mut state = assets.registry.state(water).unwrap().clone();
        state.properties.insert("level".into(), "1".into());
        let flowing = assets.registry.resolve(&state).unwrap();
        for x in 19..=21 {
            for z in 19..=21 {
                world.set(x, 3, z, stone).unwrap();
                world.set(x, 4, z, water).unwrap();
            }
        }
        let at = Vec3::new(20.5, 4.0, 20.5);
        let stand = ArmorStand::new(1, at, 0.0);
        let mut still = stand.clone();
        let in_water = still.update_fluids(
            &world,
            &fluids,
            crate::test_resources::server_loop_context(),
        );
        assert!(in_water);
        let mut scene = crate::collisions::SceneCollisions {
            world: &world,
            binding: &collision,
            subject: None,
        };
        let empty = Default::default();
        let climbing =
            sunrise_world::climbing::ClimbingRegistry::new(assets.registry, &empty, &empty, &empty)
                .unwrap();
        still.tick(&mut scene, &facts, &climbing, in_water, None);
        assert_eq!(still.position.x, at.x);
        assert_eq!(still.velocity.x, 0.0);

        world.set(21, 4, 20, flowing).unwrap();
        let mut marker = stand.clone();
        marker.marker = true;
        let in_water = marker.update_fluids(
            &world,
            &fluids,
            crate::test_resources::server_loop_context(),
        );
        assert!(!in_water);
        let mut scene = crate::collisions::SceneCollisions {
            world: &world,
            binding: &collision,
            subject: None,
        };
        marker.tick(&mut scene, &facts, &climbing, in_water, None);
        assert_eq!(marker.position, at);
        assert_eq!(marker.velocity, Vec3::ZERO);

        let mut carried = stand;
        let in_water = carried.update_fluids(
            &world,
            &fluids,
            crate::test_resources::server_loop_context(),
        );
        assert!(in_water);
        let mut scene = crate::collisions::SceneCollisions {
            world: &world,
            binding: &collision,
            subject: None,
        };
        carried.tick(&mut scene, &facts, &climbing, in_water, None);
        assert!((carried.position.x - (at.x + 0.014)).abs() < 1e-12);
        assert!((carried.velocity.x - 0.014 * f64::from(0.8f32)).abs() < 1e-12);
        for _ in 0..20 {
            let in_water = carried.update_fluids(
                &world,
                &fluids,
                crate::test_resources::server_loop_context(),
            );
            let mut scene = crate::collisions::SceneCollisions {
                world: &world,
                binding: &collision,
                subject: None,
            };
            carried.tick(&mut scene, &facts, &climbing, in_water, None);
        }
        assert!(carried.position.x > at.x + 0.5);
    }
}
