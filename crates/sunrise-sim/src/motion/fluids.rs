//! The player's fluid immersion over the resident scene: `Entity.updateFluidInteraction`
//! (water, waterlogged blocks, bubble columns and lava, with their currents), the
//! water-entry splash and the fall resets that hinge on touching water.
use super::*;
use std::sync::Mutex;
use sunrise_world::{
    entity_fluid::{
        CurrentTarget, EntityFluidInteraction, FluidInteractionWorld, FluidMembership,
        FluidSections, FluidTagId,
    },
    entity_fluid_tick::{self, EntityFluidTickWorld},
    fluid::{FluidHeightWorld, FluidKind, FluidState, FluidStateTable},
    fluid_flow::FluidFlowWorld,
    fluid_shape::FluidShapeCache,
    water_splash::{self, SplashParticle, WaterSplashWorld},
    world_ray::{RayBlockMode, RayError, RayFluidMode, RayResult, RayWorld, clip_world},
};

/// `FluidTags.WATER` and `FluidTags.LAVA`, the fluids with a current.
pub const WATER: FluidTagId = FluidTagId(0);
pub const LAVA: FluidTagId = FluidTagId(1);

/// Per-state fluid facts the player's immersion and fall-reset rays read.
pub struct FluidResources {
    states: FluidStateTable,
    /// `isFaceSturdy(FULL)` per `Direction` (bit = ordinal).
    sturdy: Vec<u8>,
    /// `#minecraft:blocks_fluid_flow` membership.
    blocks_flow: Vec<bool>,
    /// `IceBlock` and its subclasses.
    ice: Vec<bool>,
    /// `FlowingFluid.shapes`: the shape of the first query of each fluid
    /// state, kept for the whole process like the registered fluid's map.
    pub shapes: Mutex<FluidShapeCache>,
}
impl FluidResources {
    pub fn new(assets: &Assets, facts: &crate::blocks::BlockFacts) -> Result<Self> {
        Self::from_registry(assets.registry, facts, &assets.blocks_fluid_flow)
    }
    /// Build fluid queries from the same registry, block facts and flow tag as the scene.
    pub fn from_registry(
        registry: &sunrise_world::registry::BlockRegistry,
        facts: &crate::blocks::BlockFacts,
        blocks_fluid_flow: &BTreeSet<String>,
    ) -> Result<Self> {
        Ok(Self {
            states: FluidStateTable::new(registry)?,
            sturdy: facts.states.iter().map(|s| s.sturdy_full).collect(),
            blocks_flow: facts
                .states
                .iter()
                .map(|s| blocks_fluid_flow.contains(&*s.name))
                .collect(),
            ice: facts
                .states
                .iter()
                .map(|s| s.has_class("IceBlock"))
                .collect(),
            shapes: Mutex::new(FluidShapeCache::default()),
        })
    }
    pub fn fluid(&self, state: BlockStateId) -> FluidState {
        self.states.get(state).unwrap_or(FluidState::EMPTY)
    }
    pub fn states(&self) -> &FluidStateTable {
        &self.states
    }
}

/// Fluid state, tag and shared `FlowingFluid.shapes` provider for `world_ray::clip_world`.
pub struct FluidClip<'a> {
    pub states: &'a FluidStateTable,
    pub items: &'a crate::items::Items,
    pub shapes: &'a Mutex<FluidShapeCache>,
}

struct FluidAbove<'a> {
    world: &'a World,
    states: &'a FluidStateTable,
}

impl FluidHeightWorld for FluidAbove<'_> {
    type Error = std::io::Error;
    fn fluid(&mut self, pos: [i32; 3]) -> std::io::Result<FluidState> {
        if !(self.world.bottom_y()..self.world.top_y()).contains(&pos[1]) {
            return Ok(FluidState::EMPTY);
        }
        if !self.world.chunks.contains_key(&(pos[0] >> 4, pos[2] >> 4)) {
            return Err(std::io::Error::other("ray entered an unloaded block"));
        }
        Ok(self
            .states
            .get(self.world.block(pos[0], pos[1], pos[2]))
            .unwrap_or(FluidState::EMPTY))
    }
    fn sturdy_down(&mut self, _: [i32; 3]) -> std::io::Result<bool> {
        Err(std::io::Error::other(
            "camera support is outside a fluid shape query",
        ))
    }
}

impl FluidClip<'_> {
    pub fn state_of(&self, state: BlockStateId) -> FluidState {
        self.states.get(state).unwrap_or(FluidState::EMPTY)
    }
    pub fn is_source(&self, state: FluidState) -> bool {
        state.is_source()
    }
    pub fn is_empty(&self, state: FluidState) -> bool {
        state.kind() == FluidKind::Empty
    }
    pub fn is_water(&self, state: FluidState) -> bool {
        state.kind() != FluidKind::Empty
            && self
                .items
                .fluid_in_tag("minecraft:water", state.kind().name())
    }
    pub fn shape(
        &self,
        world: &World,
        state: FluidState,
        pos: BlockPos,
    ) -> std::io::Result<VoxelShape> {
        let mut cache = self
            .shapes
            .lock()
            .map_err(|_| std::io::Error::other("fluid shape cache poisoned"))?;
        cache.shape(
            &mut FluidAbove {
                world,
                states: self.states,
            },
            state,
            pos,
        )
    }
}

pub trait FluidRayProvider {
    fn state_of(&mut self, state: BlockStateId) -> FluidState;
    fn is_source(&mut self, state: FluidState) -> bool;
    fn is_empty(&mut self, state: FluidState) -> bool;
    fn is_water(&mut self, state: FluidState) -> bool;
    fn shape(
        &mut self,
        world: &World,
        state: FluidState,
        pos: BlockPos,
    ) -> std::io::Result<VoxelShape>;
}

impl FluidRayProvider for FluidClip<'_> {
    fn state_of(&mut self, state: BlockStateId) -> FluidState {
        FluidClip::state_of(self, state)
    }
    fn is_source(&mut self, state: FluidState) -> bool {
        FluidClip::is_source(self, state)
    }
    fn is_empty(&mut self, state: FluidState) -> bool {
        FluidClip::is_empty(self, state)
    }
    fn is_water(&mut self, state: FluidState) -> bool {
        FluidClip::is_water(self, state)
    }
    fn shape(
        &mut self,
        world: &World,
        state: FluidState,
        pos: BlockPos,
    ) -> std::io::Result<VoxelShape> {
        FluidClip::shape(self, world, state, pos)
    }
}

struct FluidRays<'a, W, F> {
    inner: W,
    world: &'a World,
    fluids: F,
    last_block: Option<(BlockPos, BlockStateId)>,
}

impl<W, F> RayWorld for FluidRays<'_, W, F>
where
    W: RayWorld<Error = std::io::Error, Block = BlockStateId>,
    F: FluidRayProvider,
{
    type Error = std::io::Error;
    type Block = BlockStateId;
    type Fluid = FluidState;
    fn block_state(&mut self, p: BlockPos) -> std::io::Result<BlockStateId> {
        let state = self.inner.block_state(p)?;
        self.last_block = Some((p, state));
        Ok(state)
    }
    fn fluid_state(&mut self, p: BlockPos) -> std::io::Result<FluidState> {
        let state = match self.last_block {
            Some((at, state)) if at == p => state,
            _ => self.block_state(p)?,
        };
        Ok(self.fluids.state_of(state))
    }
    fn block_shape(
        &mut self,
        state: BlockStateId,
        p: BlockPos,
        mode: RayBlockMode,
    ) -> std::io::Result<VoxelShape> {
        self.inner.block_shape(state, p, mode)
    }
    fn interaction_shape(
        &mut self,
        state: BlockStateId,
        p: BlockPos,
    ) -> std::io::Result<VoxelShape> {
        self.inner.interaction_shape(state, p)
    }
    fn fluid_is_source(&mut self, state: FluidState) -> std::io::Result<bool> {
        Ok(self.fluids.is_source(state))
    }
    fn fluid_is_empty(&mut self, state: FluidState) -> std::io::Result<bool> {
        Ok(self.fluids.is_empty(state))
    }
    fn fluid_is_water(&mut self, state: FluidState) -> std::io::Result<bool> {
        Ok(self.fluids.is_water(state))
    }
    fn fluid_shape(&mut self, state: FluidState, p: BlockPos) -> std::io::Result<VoxelShape> {
        self.fluids.shape(self.world, state, p)
    }
}

// Skip FluidRays on RayFluidMode::None because clip_world still polls fluid_state per cell for vanilla parity.
pub fn clip_world_with_fluids<W, F>(
    mut inner: W,
    world: &World,
    start: Vec3,
    end: Vec3,
    blocks: RayBlockMode,
    fluids: RayFluidMode,
    max_visits: usize,
    make_fluids: impl FnOnce() -> F,
) -> std::result::Result<RayResult, RayError<std::io::Error>>
where
    W: RayWorld<Error = std::io::Error, Block = BlockStateId>,
    F: FluidRayProvider,
{
    if fluids == RayFluidMode::None {
        return clip_world(
            &mut inner,
            start,
            end,
            blocks,
            RayFluidMode::None,
            max_visits,
        );
    }
    let mut rays = FluidRays {
        inner,
        world,
        fluids: make_fluids(),
        last_block: None,
    };
    clip_world(&mut rays, start, end, blocks, fluids, max_visits)
}

impl FluidResources {
    /// `BlockState.isFaceSturdy(FULL)` on that face.
    pub fn sturdy(&self, state: BlockStateId, d: Direction) -> bool {
        self.sturdy
            .get(state.0 as usize)
            .is_some_and(|s| s & 1 << d as u8 != 0)
    }
    /// `#minecraft:blocks_fluid_flow` membership.
    pub fn blocks_flow(&self, state: BlockStateId) -> bool {
        self.blocks_flow
            .get(state.0 as usize)
            .copied()
            .unwrap_or(false)
    }
    /// `IceBlock` or one of its subclasses.
    pub fn ice(&self, state: BlockStateId) -> bool {
        self.ice.get(state.0 as usize).copied().unwrap_or(false)
    }
}
/// The player's `EntityFluidInteraction` and the water flags around it.
#[derive(Clone)]
pub struct PlayerFluids {
    pub interaction: EntityFluidInteraction,
    /// `Entity.wasTouchingWater` (`isInWater`).
    pub was_touching_water: bool,
    /// `Entity.wasEyeInWater`.
    pub was_eye_in_water: bool,
    /// `Entity.firstTick`: no splash, and no lava, until the first base tick ends.
    pub first_tick: bool,
    /// The dimension's `FAST_LAVA` (the Nether), set by the game.
    pub fast_lava: bool,
    /// The boat the player sits in (`Entity.getVehicle`), set by the game.
    pub boat: Option<RiddenBoat>,
}
/// What `AbstractBoat.modifyPassengerFluidInteractionBox` reads of a boat.
#[derive(Clone, Copy, Debug)]
pub struct RiddenBoat {
    /// `AbstractBoat.isUnderWater`.
    pub under_water: bool,
    /// The top of the boat's box (`getBoundingBox().maxY`).
    pub max_y: f64,
}
impl Default for PlayerFluids {
    fn default() -> Self {
        Self {
            interaction: EntityFluidInteraction::new([WATER, LAVA]),
            was_touching_water: false,
            was_eye_in_water: false,
            first_tick: true,
            fast_lava: false,
            boat: None,
        }
    }
}
fn is_tagged(kind: FluidKind, tag: FluidTagId) -> bool {
    if tag == WATER {
        matches!(kind, FluidKind::Water | FluidKind::FlowingWater)
    } else {
        matches!(kind, FluidKind::Lava | FluidKind::FlowingLava)
    }
}
impl PlayerMotion {
    pub fn fluid_height(&self, tag: FluidTagId) -> f64 {
        self.fluids.interaction.fluid_height(&FluidTags, tag)
    }
    pub fn is_eye_in_fluid(&self, tag: FluidTagId) -> bool {
        self.fluids.interaction.is_eye_in_fluid(&FluidTags, tag)
    }
    /// `LocalPlayer.isUnderWater`: `wasUnderwater`, the eye sample `Player.tick`
    /// takes before the base tick (`Entity.isUnderWater` also needs the body
    /// still in water).
    pub fn is_under_water(&self) -> bool {
        self.fluids.was_eye_in_water
    }
    /// `Entity.isInLava`.
    pub fn is_in_lava(&self) -> bool {
        !self.fluids.first_tick && self.fluids.interaction.is_in_fluid(&FluidTags, LAVA)
    }
}
/// `BubbleColumnBlock.entityInside` for an entity that is not the player (a mob,
/// a dropped item, an ender pearl): the bubble columns its box touches push it
/// up or drag it down, harder at the surface (`Entity.handleOnAboveBubbleColumn`
/// and `handleOnInsideBubbleColumn`). Returns whether it was inside a column
/// below the surface, which resets its fall distance.
pub fn bubble_column_push(
    world: &World,
    registry: &sunrise_world::registry::BlockRegistry,
    facts: &crate::blocks::BlockFacts,
    collision: &sunrise_world::block_collision::BlockCollisionRegistry<'_>,
    bounds: Aabb,
    velocity: &mut Vec3,
) -> bool {
    bubble_column_push_over(world, registry, facts, collision, bounds, velocity, |_| {})
}
/// [`bubble_column_push`], telling `surface` the position of each column block
/// the entity is at the top of: `handleOnAboveBubbleColumn` sends the splash and
/// bubble particles over it (`Entity.sendBubbleColumnParticles`).
pub fn bubble_column_push_over(
    world: &World,
    registry: &sunrise_world::registry::BlockRegistry,
    facts: &crate::blocks::BlockFacts,
    collision: &sunrise_world::block_collision::BlockCollisionRegistry<'_>,
    bounds: Aabb,
    velocity: &mut Vec3,
    mut surface: impl FnMut(BlockPos),
) -> bool {
    let mut reset_fall = false;
    bubble_columns_touching(world, registry, facts, collision, bounds, |pos, column| {
        reset_fall |= column.push(velocity);
        if column.surface {
            surface(pos);
        }
    });
    reset_fall
}
/// The column blocks `bounds` touches that it is at the top of: where the
/// particles of `Entity.sendBubbleColumnParticles` start.
pub fn bubble_column_surfaces(
    world: &World,
    registry: &sunrise_world::registry::BlockRegistry,
    facts: &crate::blocks::BlockFacts,
    collision: &sunrise_world::block_collision::BlockCollisionRegistry<'_>,
    bounds: Aabb,
) -> Vec<BlockPos> {
    let mut surfaces = Vec::new();
    bubble_columns_touching(world, registry, facts, collision, bounds, |pos, column| {
        if column.surface {
            surfaces.push(pos);
        }
    });
    surfaces
}
/// Every bubble column block the box touches (`Entity.getInsideBlockPoints`).
fn bubble_columns_touching(
    world: &World,
    registry: &sunrise_world::registry::BlockRegistry,
    facts: &crate::blocks::BlockFacts,
    collision: &sunrise_world::block_collision::BlockCollisionRegistry<'_>,
    bounds: Aabb,
    mut each: impl FnMut(BlockPos, BubbleColumn),
) {
    let b = bounds.deflate(Vec3::new(
        f64::from(1.0e-5f32),
        f64::from(1.0e-5f32),
        f64::from(1.0e-5f32),
    ));
    let (lo, hi) = (b.min_position(), b.max_position());
    for x in lo.x.floor() as i32..=(hi.x.ceil() as i32 - 1) {
        for y in lo.y.floor() as i32..=(hi.y.ceil() as i32 - 1) {
            for z in lo.z.floor() as i32..=(hi.z.ceil() as i32 - 1) {
                let pos = BlockPos::new(x, y, z);
                if let Some(column) = bubble_column_at(world, registry, facts, collision, pos) {
                    each(pos, column);
                }
            }
        }
    }
}
/// A bubble column block as `BubbleColumnBlock.entityInside` sees it: whether
/// it drags down (`DRAG_DOWN`), and whether it is the surface (nothing above it
/// to collide with and no fluid there).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BubbleColumn {
    pub drag: bool,
    pub surface: bool,
}
impl BubbleColumn {
    /// `Entity.handleOnAboveBubbleColumn` at the surface (it throws harder),
    /// else `handleOnInsideBubbleColumn`. Returns whether it was inside below
    /// the surface, which resets the fall distance.
    pub fn push(self, velocity: &mut Vec3) -> bool {
        velocity.y = match (self.surface, self.drag) {
            (true, true) => (-0.9_f64).max(velocity.y - 0.03),
            (true, false) => 1.8_f64.min(velocity.y + 0.1),
            (false, true) => (-0.3_f64).max(velocity.y - 0.03),
            (false, false) => 0.7_f64.min(velocity.y + 0.06),
        };
        !self.surface
    }
}
/// The bubble column at `pos`, if there is one.
pub fn bubble_column_at(
    world: &World,
    registry: &sunrise_world::registry::BlockRegistry,
    facts: &crate::blocks::BlockFacts,
    collision: &sunrise_world::block_collision::BlockCollisionRegistry<'_>,
    pos: BlockPos,
) -> Option<BubbleColumn> {
    let state = world.block(pos.x, pos.y, pos.z);
    if &*facts.get(state).name != "minecraft:bubble_column" {
        return None;
    }
    let drag = registry
        .state(state)
        .and_then(|s| s.properties.get("drag"))
        .is_some_and(|d| d == "true");
    let above = world.block(pos.x, pos.y + 1, pos.z);
    // stateAbove.getCollisionShape(level, pos): the shape is asked
    // at the column's own position.
    let mut scene = SceneCollisions {
        world,
        binding: collision,
        subject: None,
    };
    let context = sunrise_world::collision_context::CollisionContext::empty(false);
    let surface = collision
        .collision_shape_with_context(&mut scene, above, pos, &context)
        .is_ok_and(|shape| shape.is_empty())
        && facts.get(above).fluid == crate::blocks::Fluid::None;
    Some(BubbleColumn { drag, surface })
}
/// Which fluids `FluidTags.WATER` and `FluidTags.LAVA` hold.
pub struct FluidTags;
impl FluidMembership for FluidTags {
    fn fluid_in_tag(&self, kind: FluidKind, tag: FluidTagId) -> bool {
        is_tagged(kind, tag)
    }
}
/// A pending `Entity.updateSwimming` entry. Its answer belongs only to this actor and sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwimEntry {
    entity: i32,
    position: Vec3,
    passenger: bool,
    under_water: bool,
    feet_fluid: FluidKind,
}

impl MotionWorld<'_, '_, '_> {
    pub(super) fn fluid_at(&self, p: BlockPos) -> FluidState {
        self.resources.fluids.fluid(self.block(p))
    }
    /// `Entity.baseTick`'s fluid phase: `wasEyeInWater`, `updateFluidInteraction`,
    /// swimming and remaining fire, lava's fall halving, then the first-tick clear.
    pub fn fluid_base_tick(&mut self) -> Result<()> {
        self.begin_fluid_tick()?;
        self.finish_fluid_tick(None)
    }
    pub fn begin_fluid_tick(&mut self) -> Result<()> {
        self.player.fluids.was_eye_in_water = self.player.is_eye_in_fluid(WATER);
        self.update_fluids()?;
        Ok(())
    }
    /// `Player.updateSwimming` bypasses the entire check in flight. The entry
    /// predicate is asked before excluding a passenger; continuation never asks it.
    pub fn swim_entry(&self) -> Option<SwimEntry> {
        let p = &self.player;
        (!p.flying && !p.swimming && p.sprinting).then(|| SwimEntry {
            entity: p.entity_id,
            position: p.position,
            passenger: p.is_passenger,
            under_water: self.swim_entry_underwater(),
            feet_fluid: self.fluid_at(BlockPos::containing(p.position)).kind(),
        })
    }
    fn swim_entry_underwater(&self) -> bool {
        self.player.is_under_water()
            && (self.player.client_side || self.player.fluids.was_touching_water)
    }
    pub fn apply_swim_entry(&mut self, answer: Option<(SwimEntry, bool)>) {
        if let Some((entry, answer)) = answer {
            if self.swim_entry() == Some(entry) {
                self.player.swimming = answer && !entry.passenger;
            }
        } else {
            let p = &self.player;
            self.player.swimming = !p.flying
                && !p.is_passenger
                && p.sprinting
                && if p.swimming {
                    p.fluids.was_touching_water
                } else {
                    self.swim_entry_underwater()
                        && is_tagged(
                            self.fluid_at(BlockPos::containing(p.position)).kind(),
                            WATER,
                        )
                };
        }
    }
    /// Finish the fluid phase where no owning server hit needs to settle in between.
    pub fn finish_fluid_tick(&mut self, answer: Option<(SwimEntry, bool)>) -> Result<()> {
        self.swim_and_burn_tick(answer)?;
        self.finish_fluid_base_tick();
        Ok(())
    }

    /// Apply the swim-entry answer, then run the burning hit before decrementing fire.
    pub fn swim_and_burn_tick(&mut self, answer: Option<(SwimEntry, bool)>) -> Result<()> {
        self.apply_swim_entry(answer);
        self.hurt_from_remaining_fire()
    }

    /// Entity.baseTick after the burning hit returned to its owner.
    pub fn finish_fluid_base_tick(&mut self) {
        if self.player.fire_ticks > 0 {
            self.player.fire_ticks -= 1;
        }
        if self.player.fluids.was_touching_water {
            self.player.fire_ticks = self.player.fire_ticks.min(0);
        }
        if self.player.is_in_lava() {
            self.player.fall_distance *= 0.5;
        }
        self.player.fluids.first_tick = false;
    }
    /// `Entity.updateFluidInteraction`: immersion heights, then touching water
    /// resets the fall and splashes on entry, then the currents push.
    pub fn update_fluids(&mut self) -> Result<bool> {
        let mut interaction = std::mem::take(&mut self.player.fluids.interaction);
        let result = entity_fluid_tick::update(&mut interaction, self, WATER, LAVA, 1_000_000);
        self.player.fluids.interaction = interaction;
        result.map_err(|e| e.to_string().into())
    }
    /// `ClipContext.Fluid.WATER` in a fall-reset ray: the water's shape.
    pub(super) fn water_clip(&mut self, from: Vec3, to: Vec3, p: BlockPos) -> Result<bool> {
        let fluid = self.fluid_at(p);
        if !is_tagged(fluid.kind(), WATER) {
            return Ok(false);
        }
        let resources = self.resources;
        let shape = resources
            .fluids
            .shapes
            .lock()
            .map_err(|_| "fluid shape cache poisoned")?
            .shape(self, fluid, p)?;
        Ok(shape.clip(from, to, p).is_some())
    }
}
impl FluidMembership for MotionWorld<'_, '_, '_> {
    fn fluid_in_tag(&self, kind: FluidKind, tag: FluidTagId) -> bool {
        is_tagged(kind, tag)
    }
}
impl CurrentTarget for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    fn is_player_class(&self) -> bool {
        true
    }
    fn velocity(&mut self) -> Result<Vec3> {
        Ok(self.player.velocity)
    }
    fn add_delta_movement(&mut self, delta: Vec3) -> Result<()> {
        // Entity.addDeltaMovement and setDeltaMovement keep only finite values.
        let v = self.player.velocity;
        let sum = Vec3::new(v.x + delta.x, v.y + delta.y, v.z + delta.z);
        if delta.is_finite() && sum.is_finite() {
            self.player.velocity = sum;
        }
        Ok(())
    }
}
impl FluidInteractionWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    type Chunk = ();
    fn interaction_box(&mut self) -> Result<Option<Aabb>> {
        // Entity.getFluidInteractionBox: the box less 0.001, then the vehicle's
        // say (`AbstractBoat.modifyPassengerFluidInteractionBox`).
        let b = self.player.bounds().deflate(Vec3::new(0.001, 0.001, 0.001));
        let Some(boat) = self.player.fluids.boat.filter(|boat| !boat.under_water) else {
            return Ok(Some(b));
        };
        let (lo, hi) = (b.min_position(), b.max_position());
        // A boat that reaches the passenger's head shelters all of it; else
        // only what lies above the boat's top counts.
        if boat.max_y >= hi.y {
            return Ok(None);
        }
        Ok(Some(Aabb::new(
            Vec3::new(lo.x, lo.y.max(boat.max_y), lo.z),
            hi,
        )))
    }
    fn loaded_chunk(&mut self, x: i32, z: i32) -> Result<Option<()>> {
        Ok(self.scene.chunks.contains_key(&(x, z)).then_some(()))
    }
    fn chunk_sections(&mut self, _: ()) -> Result<FluidSections<'_>> {
        // Resident sections keep no fluid count: scanning the box finds no
        // fluid where a section without fluid would have skipped it.
        let layout = self.scene.layout();
        Ok(FluidSections {
            minimum_section_y: layout.min_y() >> 4,
            has_fluid: crate::chunk::every_section(layout),
        })
    }
    fn bounding_box_min_y(&self) -> f64 {
        self.player.bounds().min_position().y
    }
    fn block_x(&self) -> i32 {
        self.player.position.x.floor() as i32
    }
    fn eye_y(&mut self) -> Result<f64> {
        Ok(self.player.position.y + f64::from(self.player.dimensions.eye_height))
    }
    fn block_z(&self) -> i32 {
        self.player.position.z.floor() as i32
    }
    fn fluid_state(&mut self, p: BlockPos) -> Result<FluidState> {
        Ok(self.fluid_at(p))
    }
    fn fluid_height(&mut self, f: FluidState, p: BlockPos) -> Result<f32> {
        f.height_at(self, [p.x, p.y, p.z])
    }
    fn camera_height(&mut self, f: FluidState, p: BlockPos) -> Result<f32> {
        f.camera_height_at(self, [p.x, p.y, p.z])
    }
    fn flow(&mut self, f: FluidState, p: BlockPos) -> Result<Vec3> {
        super::pushing_flow(self.loop_context, self, f, p)
    }
}
impl EntityFluidTickWorld for MotionWorld<'_, '_, '_> {
    /// `Player.isPushedByFluid`: not while flying.
    fn is_pushed_by_fluid(&mut self) -> Result<bool> {
        Ok(!self.player.flying)
    }
    fn reset_fall_distance(&mut self) -> Result<()> {
        self.player.reset_fall_distance();
        Ok(())
    }
    fn was_touching_water(&self) -> bool {
        self.player.fluids.was_touching_water
    }
    fn first_tick(&self) -> bool {
        self.player.fluids.first_tick
    }
    fn water_splash(&mut self) -> Result<()> {
        // `Player.doWaterSplashEffect`: spectators skip the effect (and its random draws).
        if self.player.spectator {
            return Ok(());
        }
        water_splash::water_splash(self, 1_000_000).map_err(|e| e.to_string())?;
        Ok(())
    }
    fn set_was_touching_water(&mut self, value: bool) {
        self.player.fluids.was_touching_water = value;
    }
    fn fast_lava(&mut self) -> Result<bool> {
        Ok(self.player.fluids.fast_lava)
    }
}
impl FluidHeightWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    fn fluid(&mut self, p: [i32; 3]) -> Result<FluidState> {
        Ok(self.fluid_at(BlockPos::new(p[0], p[1], p[2])))
    }
    fn sturdy_down(&mut self, p: [i32; 3]) -> Result<bool> {
        let state = MotionWorld::block(self, BlockPos::new(p[0], p[1], p[2]));
        Ok(self.resources.fluids.sturdy(state, Direction::Down))
    }
}
impl FluidFlowWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    type Block = BlockStateId;
    fn fluid(&mut self, p: BlockPos) -> Result<FluidState> {
        Ok(self.fluid_at(p))
    }
    fn block(&mut self, p: BlockPos) -> Result<BlockStateId> {
        Ok(MotionWorld::block(self, p))
    }
    fn blocks_fluid_flow(&self, s: BlockStateId) -> bool {
        self.resources.fluids.blocks_flow(s)
    }
    fn is_ice_class(&self, s: BlockStateId) -> bool {
        self.resources.fluids.ice(s)
    }
    fn is_face_sturdy(&mut self, s: BlockStateId, _: BlockPos, d: Direction) -> Result<bool> {
        Ok(self.resources.fluids.sturdy(s, d))
    }
}
/// `Entity.doWaterSplashEffect` for the player: the world random draws the
/// pitch and particle offsets.
impl WaterSplashWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    type Entity = ();
    /// `Player.getSwimHighSpeedSplashSound` when true, else `getSwimSplashSound`.
    type Sound = bool;
    fn this_entity(&self) {}
    fn controlling_passenger(&mut self) -> Result<Option<()>> {
        Ok(None)
    }
    fn velocity(&mut self, _: ()) -> Result<Vec3> {
        Ok(self.player.velocity)
    }
    fn splash_sound(&mut self, high_speed: bool) -> Result<bool> {
        Ok(high_speed)
    }
    fn random_float(&mut self) -> Result<f32> {
        Ok(self.scene.random.next_float())
    }
    fn random_double(&mut self) -> Result<f64> {
        Ok(self.scene.random.next_double())
    }
    fn play_sound(&mut self, high_speed: bool, volume: f32, pitch: f32) -> Result<()> {
        self.player.events.push(MotionEvent::Splash {
            high_speed,
            volume,
            pitch,
        });
        Ok(())
    }
    fn position(&self) -> Vec3 {
        self.player.position
    }
    fn width(&self) -> f32 {
        self.player.dimensions.width
    }
    fn particle(&mut self, kind: SplashParticle, position: Vec3, velocity: Vec3) -> Result<()> {
        self.player.events.push(MotionEvent::SplashParticle {
            bubble: matches!(kind, SplashParticle::Bubble),
            position,
            velocity,
        });
        Ok(())
    }
    fn splash_event(&mut self) -> Result<()> {
        // The SPLASH game event: sculk listens to the player's moves itself.
        self.player.events.push(MotionEvent::SplashEvent);
        Ok(())
    }
}

#[cfg(test)]
mod fluid_clip_tests {
    use super::*;
    use crate::{chunk::Chunk, collisions::SceneCollisions, items::Items};
    use std::cell::Cell;
    use std::collections::HashSet;
    use std::io;
    use std::sync::{Arc, Mutex};
    use sunrise_core::{
        geometry::{Axis, Direction, Vec3, VoxelShape},
        nbt::Tag,
        position::BlockPos,
    };
    use sunrise_world::{
        block_collision::BlockCollisionRegistry,
        collision::CachedCollisionRegistry,
        collision_context::CollisionContext,
        fluid::FluidStateTable,
        fluid_shape::FluidShapeCache,
        registry::BlockStateId,
        world_ray::{RayBlockMode, RayFluidMode, RayWorld},
    };

    struct SceneBlockRays<'a, 'r> {
        scene: SceneCollisions<'a, 'r>,
        context: CollisionContext<'static>,
    }

    impl RayWorld for SceneBlockRays<'_, '_> {
        type Error = io::Error;
        type Block = BlockStateId;
        type Fluid = ();
        fn block_state(&mut self, p: BlockPos) -> io::Result<BlockStateId> {
            let world = self.scene.world;
            if !(world.bottom_y()..world.top_y()).contains(&p.y)
                || !world.chunks.contains_key(&(p.x >> 4, p.z >> 4))
            {
                return Err(io::Error::other("ray entered an unloaded block"));
            }
            Ok(world.block(p.x, p.y, p.z))
        }
        fn fluid_state(&mut self, _: BlockPos) -> io::Result<()> {
            Ok(())
        }
        fn block_shape(
            &mut self,
            state: BlockStateId,
            p: BlockPos,
            _: RayBlockMode,
        ) -> io::Result<VoxelShape> {
            let binding = self.scene.binding;
            binding
                .collision_shape_with_context(&mut self.scene, state, p, &self.context)
                .map_err(|e| io::Error::other(format!("{e:?}")))
        }
        fn interaction_shape(&mut self, _: BlockStateId, _: BlockPos) -> io::Result<VoxelShape> {
            Ok(VoxelShape::empty())
        }
        fn fluid_is_source(&mut self, _: ()) -> io::Result<bool> {
            Ok(false)
        }
        fn fluid_is_empty(&mut self, _: ()) -> io::Result<bool> {
            Ok(true)
        }
        fn fluid_is_water(&mut self, _: ()) -> io::Result<bool> {
            Ok(false)
        }
        fn fluid_shape(&mut self, _: (), _: BlockPos) -> io::Result<VoxelShape> {
            Ok(VoxelShape::empty())
        }
    }

    struct CountingFluidClip<'a> {
        inner: FluidClip<'a>,
        lookups: &'a Cell<usize>,
    }

    impl FluidRayProvider for CountingFluidClip<'_> {
        fn state_of(&mut self, state: BlockStateId) -> FluidState {
            self.lookups.set(self.lookups.get() + 1);
            self.inner.state_of(state)
        }
        fn is_source(&mut self, state: FluidState) -> bool {
            self.lookups.set(self.lookups.get() + 1);
            self.inner.is_source(state)
        }
        fn is_empty(&mut self, state: FluidState) -> bool {
            self.lookups.set(self.lookups.get() + 1);
            self.inner.is_empty(state)
        }
        fn is_water(&mut self, state: FluidState) -> bool {
            self.lookups.set(self.lookups.get() + 1);
            self.inner.is_water(state)
        }
        fn shape(
            &mut self,
            world: &World,
            state: FluidState,
            pos: BlockPos,
        ) -> io::Result<VoxelShape> {
            self.lookups.set(self.lookups.get() + 1);
            self.inner.shape(world, state, pos)
        }
    }

    #[test]
    fn fluid_clip_matrix_matches_vanilla_modes_shapes_and_ties() {
        let state_specs: &[&str] = &[
            "minecraft:air",
            "minecraft:stone",
            "minecraft:water[level=0]",
            "minecraft:water[level=5]",
            "minecraft:water[level=4]",
            "minecraft:water[level=8]",
            "minecraft:lava[level=0]",
            "minecraft:lava[level=2]",
            "minecraft:oak_slab[type=bottom,waterlogged=true]",
            "minecraft:oak_slab[type=top,waterlogged=true]",
        ];
        let state_tags: Vec<Tag> = state_specs
            .iter()
            .map(|spec| {
                let Some((name, rest)) = spec.split_once('[') else {
                    return Tag::String((*spec).into());
                };
                let props = rest
                    .trim_end_matches(']')
                    .split(',')
                    .map(|pair| {
                        let (k, v) = pair.split_once('=').unwrap();
                        (k, Tag::String(v.into()))
                    })
                    .collect::<Vec<_>>();
                Tag::compound([
                    ("id", Tag::String(name.into())),
                    ("properties", Tag::compound(props)),
                ])
            })
            .collect();
        let registry = sunrise_world::registry::BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(sunrise_world::DATA_VERSION)),
            ("states", Tag::List(state_tags.clone())),
            (
                "defaults",
                Tag::compound([
                    ("minecraft:air", Tag::Int(0)),
                    ("minecraft:stone", Tag::Int(1)),
                    ("minecraft:water", Tag::Int(2)),
                    ("minecraft:lava", Tag::Int(6)),
                    ("minecraft:oak_slab", Tag::Int(8)),
                ]),
            ),
        ]))
        .unwrap();
        let box_shape = |y0: f64, y1: f64, full: bool| {
            let one = Tag::LongArray(vec![0, 1.0_f64.to_bits() as i64]);
            Tag::compound([
                ("canonical", Tag::Byte(0)),
                ("cube", Tag::Byte(i8::from(full))),
                ("cube_axes", Tag::Byte(if full { 7 } else { 5 })),
                ("x", one.clone()),
                (
                    "y",
                    Tag::LongArray(vec![y0.to_bits() as i64, y1.to_bits() as i64]),
                ),
                ("z", one),
                ("occupied", Tag::ByteArray(vec![1])),
                ("discrete_min", Tag::IntArray(vec![0; 3])),
                ("discrete_max", Tag::IntArray(vec![1; 3])),
            ])
        };
        let empty = VoxelShape::empty();
        let (min, max) = empty.discrete_bounds();
        let axes = empty.cube_coordinate_axes();
        let bits = |axis| {
            Tag::LongArray(
                empty
                    .coordinates(axis)
                    .iter()
                    .map(|n| n.to_bits() as i64)
                    .collect(),
            )
        };
        let empty_tag = Tag::compound([
            ("canonical", Tag::Byte(0)),
            ("cube", Tag::Byte(0)),
            (
                "cube_axes",
                Tag::Byte((axes[0] as i8) | ((axes[1] as i8) << 1) | ((axes[2] as i8) << 2)),
            ),
            ("x", bits(Axis::X)),
            ("y", bits(Axis::Y)),
            ("z", bits(Axis::Z)),
            ("occupied", Tag::ByteArray(vec![])),
            ("discrete_min", Tag::IntArray(min.to_vec())),
            ("discrete_max", Tag::IntArray(max.to_vec())),
        ]);
        let table = Tag::compound([
            ("DataVersion", Tag::Int(sunrise_world::DATA_VERSION)),
            ("states", Tag::List(state_tags)),
            (
                "shapes",
                Tag::List(vec![
                    empty_tag,
                    box_shape(0.0, 1.0, true),
                    box_shape(0.0, 0.5, false),
                    box_shape(0.5, 1.0, false),
                ]),
            ),
            (
                "cached_shapes",
                Tag::IntArray(vec![0, 1, 0, 0, 0, 0, 0, 0, 2, 3]),
            ),
        ]);
        let cached = CachedCollisionRegistry::from_nbt(&table, &registry).unwrap();
        let binding = BlockCollisionRegistry::new(Arc::new(cached)).unwrap();
        let states = FluidStateTable::new(&registry).unwrap();
        let mut items = Items::for_tests(Default::default());
        items.fluid_tags.insert(
            "minecraft:water".into(),
            HashSet::from([
                "minecraft:water".into(),
                "minecraft:flowing_water".into(),
                "minecraft:lava".into(),
                "minecraft:flowing_lava".into(),
            ]),
        );
        let shapes = Mutex::new(FluidShapeCache::default());

        let (stone, water_src, water_flow3, water_flow4, water_fall) = (
            BlockStateId(1),
            BlockStateId(2),
            BlockStateId(3),
            BlockStateId(4),
            BlockStateId(5),
        );
        let (lava_src, lava_flow6, slab_bot, slab_top) = (
            BlockStateId(6),
            BlockStateId(7),
            BlockStateId(8),
            BlockStateId(9),
        );

        let mut world = World::new(BlockStateId(0), 0);
        world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        world.chunks.insert((0, -1), Arc::new(Chunk::empty(0, -1)));
        let cells = [
            (
                0,
                water_src,
                f64::from(8.0_f32 / 9.0),
                [false, true, true, true],
            ),
            (
                1,
                water_flow3,
                f64::from(3.0_f32 / 9.0),
                [false, false, true, true],
            ),
            (
                2,
                lava_src,
                f64::from(8.0_f32 / 9.0),
                [false, true, true, true],
            ),
            (
                3,
                lava_flow6,
                f64::from(6.0_f32 / 9.0),
                [false, false, true, true],
            ),
            (
                4,
                slab_bot,
                f64::from(8.0_f32 / 9.0),
                [false, true, true, true],
            ),
        ];
        for &(x, state, _, _) in &cells {
            world.set(x, 63, 0, stone).unwrap();
            world.set(x, 64, 0, state).unwrap();
        }
        // Neighbour-dependent height: water above lifts flowing and falling shapes to 1.0.
        world.set(5, 64, 0, water_flow4).unwrap();
        world.set(5, 65, 0, water_src).unwrap();
        world.set(6, 64, 0, water_fall).unwrap();
        world.set(6, 65, 0, water_src).unwrap();
        // Solid-on-tie comparison: plain water vs waterlogged top slab.
        world.set(7, 64, 0, water_src).unwrap();
        world.set(8, 64, 0, slab_top).unwrap();
        // Block in front of fluid (z=2) and behind fluid (z=3).
        world.set(9, 64, 2, stone).unwrap();
        world.set(9, 64, 3, water_src).unwrap();

        let clip_counted = |world: &World, from: Vec3, to: Vec3, mode: RayFluidMode| {
            let builds = Cell::new(0usize);
            let lookups = Cell::new(0usize);
            let rays = SceneBlockRays {
                scene: SceneCollisions {
                    world,
                    binding: &binding,
                    subject: None,
                },
                context: CollisionContext::empty(false),
            };
            let result = clip_world_with_fluids(
                rays,
                world,
                from,
                to,
                RayBlockMode::Collider,
                mode,
                usize::MAX,
                || {
                    builds.set(builds.get() + 1);
                    CountingFluidClip {
                        inner: FluidClip {
                            states: &states,
                            items: &items,
                            shapes: &shapes,
                        },
                        lookups: &lookups,
                    }
                },
            );
            (result, builds.get(), lookups.get())
        };
        let clip = |world: &World, from: Vec3, to: Vec3, mode: RayFluidMode| {
            clip_counted(world, from, to, mode).0
        };

        let modes = [
            RayFluidMode::None,
            RayFluidMode::SourceOnly,
            RayFluidMode::Any,
            RayFluidMode::Water,
        ];
        for &(x, state, height, stops) in &cells {
            let cx = f64::from(x) + 0.5;
            for (mode, expect_fluid) in modes.into_iter().zip(stops) {
                let (res, builds, lookups) = clip_counted(
                    &world,
                    Vec3::new(cx, 65.5, 0.5),
                    Vec3::new(cx, 62.5, 0.5),
                    mode,
                );
                let res = res.unwrap();
                if mode == RayFluidMode::None {
                    assert_eq!((builds, lookups), (0, 0), "x={x} None skips fluid lookups");
                } else {
                    assert_eq!(builds, 1, "x={x} mode={mode:?}");
                    assert!(lookups > 0, "x={x} mode={mode:?}");
                }
                assert!(!res.miss, "x={x} mode={mode:?}");
                if expect_fluid {
                    assert_eq!(
                        res.hit.block,
                        BlockPos::new(x, 64, 0),
                        "x={x} mode={mode:?}"
                    );
                    assert_eq!(res.hit.direction, Direction::Up, "x={x} mode={mode:?}");
                    assert_eq!(res.hit.location.y, 64.0 + height, "x={x} mode={mode:?}");
                    assert!(!res.hit.inside);
                } else if state == slab_bot {
                    assert_eq!(res.hit.block, BlockPos::new(x, 64, 0));
                    assert_eq!(res.hit.location.y, 64.5);
                } else {
                    assert_eq!(
                        res.hit.block,
                        BlockPos::new(x, 63, 0),
                        "x={x} mode={mode:?}"
                    );
                    assert_eq!(res.hit.location.y, 64.0, "x={x} mode={mode:?}");
                }
            }
        }

        // Passing horizontally above flowing water's 3/9 surface misses it; below hits North.
        let over = clip(
            &world,
            Vec3::new(1.5, 64.5, -0.5),
            Vec3::new(1.5, 64.5, 1.5),
            RayFluidMode::Any,
        )
        .unwrap();
        assert!(over.miss);
        let side = clip(
            &world,
            Vec3::new(1.5, 64.2, -0.5),
            Vec3::new(1.5, 64.2, 1.5),
            RayFluidMode::Any,
        )
        .unwrap();
        assert!(!side.miss);
        assert_eq!(side.hit.block, BlockPos::new(1, 64, 0));
        assert_eq!(side.hit.direction, Direction::North);
        assert_eq!(side.hit.location, Vec3::new(1.5, 64.2, 0.0));

        // Neighbour-dependent height (cells 5 and 6): water above lifts flowing and falling cell shapes to y=65.0.
        for x in [5, 6] {
            let cx = f64::from(x) + 0.5;
            let hit = clip(
                &world,
                Vec3::new(cx, 64.95, -0.5),
                Vec3::new(cx, 64.95, 0.5),
                RayFluidMode::Any,
            )
            .unwrap();
            assert!(!hit.miss, "x={x}");
            assert_eq!(hit.hit.block, BlockPos::new(x, 64, 0));
            assert_eq!(hit.hit.direction, Direction::North);
            assert_eq!(hit.hit.location, Vec3::new(cx, 64.95, 0.0));
        }

        // Solid-on-tie: at (x+0.5, 64 + 8/9, 0), fluid hits Up while top slab ties on North.
        let h = f64::from(8.0_f32 / 9.0);
        let plain = clip(
            &world,
            Vec3::new(7.5, 64.0 + h + 0.5, -0.5),
            Vec3::new(7.5, 64.0 + h - 0.5, 0.5),
            RayFluidMode::Any,
        )
        .unwrap();
        assert_eq!(plain.hit.block, BlockPos::new(7, 64, 0));
        assert_eq!(plain.hit.direction, Direction::Up);
        let tied = clip(
            &world,
            Vec3::new(8.5, 64.0 + h + 0.5, -0.5),
            Vec3::new(8.5, 64.0 + h - 0.5, 0.5),
            RayFluidMode::Any,
        )
        .unwrap();
        assert_eq!(tied.hit.block, BlockPos::new(8, 64, 0));
        assert_eq!(tied.hit.direction, Direction::North);
        assert_eq!(tied.hit.location, Vec3::new(8.5, 64.0 + h, 0.0));

        // Ray starting inside fluid sets inside=true with opposite direction.
        let inside = clip(
            &world,
            Vec3::new(0.5, 64.25, 0.5),
            Vec3::new(0.5, 64.25, 2.5),
            RayFluidMode::SourceOnly,
        )
        .unwrap();
        assert!(!inside.miss && inside.hit.inside);
        assert_eq!(inside.hit.block, BlockPos::new(0, 64, 0));
        assert_eq!(inside.hit.direction, Direction::North);

        // Block in front of fluid wins; block behind fluid loses to fluid.
        let front = clip(
            &world,
            Vec3::new(9.5, 64.5, 1.5),
            Vec3::new(9.5, 64.5, 4.5),
            RayFluidMode::Any,
        )
        .unwrap();
        assert_eq!(front.hit.block, BlockPos::new(9, 64, 2));
        let behind = clip(
            &world,
            Vec3::new(9.5, 64.5, 4.5),
            Vec3::new(9.5, 64.5, 1.5),
            RayFluidMode::Any,
        )
        .unwrap();
        assert_eq!(behind.hit.block, BlockPos::new(9, 64, 3));

        // No fixed 4096 visit limit: a ray across >4096 loaded cells completes as a clean miss.
        let mut corridor = World::new(BlockStateId(0), 0);
        for cx in 0..265 {
            corridor
                .chunks
                .insert((cx, 0), Arc::new(Chunk::empty(cx, 0)));
        }
        let long = clip(
            &corridor,
            Vec3::new(0.5, 64.5, 0.5),
            Vec3::new(4200.5, 64.5, 0.5),
            RayFluidMode::Any,
        )
        .unwrap();
        assert!(long.miss);
        assert_eq!(long.hit.block, BlockPos::new(4200, 64, 0));

        // Unloaded chunks and non-finite coordinates fail without loading any chunk.
        let loaded_before = world.chunks.len();
        assert!(
            clip(
                &world,
                Vec3::new(15.5, 64.5, 0.5),
                Vec3::new(17.5, 64.5, 0.5),
                RayFluidMode::Any,
            )
            .is_err()
        );
        assert!(
            clip(
                &world,
                Vec3::new(f64::NAN, 64.5, 0.5),
                Vec3::new(1.5, 64.5, 0.5),
                RayFluidMode::Any,
            )
            .is_err()
        );
        assert_eq!(world.chunks.len(), loaded_before);
    }
}
