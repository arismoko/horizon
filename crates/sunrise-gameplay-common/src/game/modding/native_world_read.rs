//! Read-only block, tag and fluid queries for native mods.

use mod_api::{FluidKey, FluidState, LevelRef, LightLayer};
use sunrise_core::geometry::Direction;
pub use sunrise_items::items::Items;
pub use sunrise_level::world::World;
pub use sunrise_world::{
    fluid::{FluidHeightWorld, FluidKind, FluidState as EngineFluidState},
    level::{Behaviours, fluid_can_be_replaced_with},
    registry::{BlockRegistry, BlockStateId},
};

/// Shared by the level handle and the restricted block-hook view.
pub fn block_face_sturdy(
    world: &World,
    b: &Behaviours,
    binding: &sunrise_world::block_collision::BlockCollisionRegistry<'_>,
    pos: [i32; 3],
    face: mod_api::BlockFace,
    support: mod_api::SupportType,
) -> Option<bool> {
    if !world.is_loaded_at(pos[0], pos[1], pos[2]) {
        return None;
    }
    let state = world.block(pos[0], pos[1], pos[2]);
    let direction = block_face_direction(face);
    if !b.has_cached_support(state) || binding.publishes_shapes(state) {
        use sunrise_level::collisions::SceneCollisions;
        use sunrise_world::{block_support::SupportType, dynamic_support::DynamicSupport};
        let mut scene = SceneCollisions {
            world,
            binding,
            subject: None,
        };
        let support = match support {
            mod_api::SupportType::Full => SupportType::Full,
            mod_api::SupportType::Center => SupportType::Center,
            mod_api::SupportType::Rigid => SupportType::Rigid,
            _ => unreachable!("mod_api::SupportType: unhandled variant"),
        };
        // A state that may publish takes its block entity's shape, else its declared one.
        if binding.publishes_shapes(state) {
            let shape = binding.collision_shape(&mut scene, state, pos).ok()?;
            return support.is_supporting(&shape, direction).ok();
        }
        let dynamic = DynamicSupport::from_state(binding.registry().state(state)?).ok()??;
        return dynamic
            .is_face_sturdy(&mut scene, pos, direction, support)
            .ok();
    }
    Some(match support {
        mod_api::SupportType::Full => b.is_face_sturdy(state, direction),
        mod_api::SupportType::Center => b.is_face_sturdy_center(state, direction),
        mod_api::SupportType::Rigid => b.is_face_sturdy_rigid(state, direction),
        _ => unreachable!("mod_api::SupportType: unhandled variant"),
    })
}

/// Read the published sky light without requiring a block or a resident column.
pub fn can_see_sky(world: &World, pos: [i32; 3]) -> bool {
    world.shape.sky_light && world_brightness(world, LightLayer::Sky, pos) >= 15
}

/// Convert the engine's state using the load-owned kind-to-key mapping.
pub fn api_fluid_state(state: EngineFluidState) -> Option<FluidState> {
    Some(mod_api::__private::fluid_state(
        match state.kind().vanilla_index() {
            Some(index) => mod_api::__private::world_fluid_key(usize::from(index))?,
            None => mod_api::__private::fluid_key(state.kind().name())?,
        },
        state.amount(),
        state.falling(),
        state.is_source(),
    ))
}

/// `FluidState.getShape`'s one box for `fluid` with `above` over it: up to a full block under
/// the same fluid (`FluidState.getHeight`), else to the fluid's own height. None for no fluid.
pub fn fluid_shape(fluid: EngineFluidState, above: Option<EngineFluidState>) -> Option<[f64; 6]> {
    if fluid.kind() == FluidKind::Empty {
        return None;
    }
    let height = if above.is_some_and(|above| above.kind().same_family(fluid.kind())) {
        1.0
    } else {
        f64::from(fluid.own_height())
    };
    Some([0.0, 0.0, 0.0, 1.0, height, 1.0])
}

/// The world fluid a registered key names. None for a tank-only fluid, which has no world
/// form, and for a key the registry doesn't hold.
pub fn engine_fluid_kind(fluid: FluidKey) -> Option<FluidKind> {
    FluidKind::from_name(mod_api::__private::fluid_id(fluid)?)
}

/// The blocks around one position of a level, as `FluidState.getHeight` reads them: an unloaded
/// block is empty and has no sturdy underside.
struct FluidColumn<'a, 'g, S: crate::game::SideGame<'g>> {
    game: &'a S,
    level: LevelRef,
    pub resource_lifetime: std::marker::PhantomData<fn(&'g ()) -> &'g ()>,
}
impl<'__carrier, S: crate::game::SideGame<'__carrier>> FluidColumn<'_, '__carrier, S> {
    fn fluid_at(&self, pos: [i32; 3]) -> EngineFluidState {
        self.game
            .level_block_state(self.level, pos)
            .map_or(EngineFluidState::EMPTY, |state| {
                self.game.res.behaviours().fluid(state)
            })
    }
}
impl<'__carrier, S: crate::game::SideGame<'__carrier>> FluidHeightWorld
    for FluidColumn<'_, '__carrier, S>
{
    type Error = std::convert::Infallible;
    fn fluid(&mut self, pos: [i32; 3]) -> Result<EngineFluidState, Self::Error> {
        Ok(self.fluid_at(pos))
    }
    fn sturdy_down(&mut self, pos: [i32; 3]) -> Result<bool, Self::Error> {
        Ok(self
            .game
            .level_block_state(self.level, pos)
            .is_some_and(|state| {
                self.game
                    .res
                    .behaviours()
                    .is_face_sturdy(state, Direction::Down)
            }))
    }
}

pub trait NativeWorldReadGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// The fluid's shape at `pos` (`FluidState.getShape`), block-local: none for no fluid, else
    /// the floor to `FluidState.getHeight`, a full block under the same fluid and the fluid's
    /// own height otherwise. None as for `level_block_state`; an unloaded block above is no
    /// fluid.
    fn level_fluid_shape(&self, level: LevelRef, pos: [i32; 3]) -> Option<Vec<[f64; 6]>> {
        let behaviours = self.res.behaviours();
        let fluid = behaviours.fluid(self.level_block_state(level, pos)?);
        let above = self
            .level_block_state(level, [pos[0], pos[1] + 1, pos[2]])
            .map(|state| behaviours.fluid(state));
        Some(fluid_shape(fluid, above).into_iter().collect())
    }

    /// `FluidState.getHeight(level, pos)` of the fluid at `pos`. None for a stale handle or a
    /// position that isn't loaded, as `level_block_state`.
    fn level_fluid_height(&self, level: LevelRef, pos: [i32; 3]) -> Option<f32> {
        let mut column = FluidColumn {
            game: self,
            level,
            resource_lifetime: std::marker::PhantomData,
        };
        self.level_block_state(level, pos)?;
        let Ok(height) = column.fluid_at(pos).height_at(&mut column, pos);
        Some(height)
    }

    /// The surface height the camera's immersion test reads at `pos`
    /// (`FluidState::camera_height_at`). None as for `level_fluid_height`.
    fn level_fluid_height_for_camera(&self, level: LevelRef, pos: [i32; 3]) -> Option<f32> {
        let mut column = FluidColumn {
            game: self,
            level,
            resource_lifetime: std::marker::PhantomData,
        };
        self.level_block_state(level, pos)?;
        let Ok(height) = column.fluid_at(pos).camera_height_at(&mut column, pos);
        Some(height)
    }

    /// `FluidState.canBeReplacedWith(level, pos, fluid, direction)` for the fluid at `pos`. False
    /// for a `fluid` with no world form; None as for `level_fluid_height`.
    fn level_fluid_can_be_replaced_with(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        fluid: FluidKey,
        direction: Direction,
    ) -> Option<bool> {
        let mut column = FluidColumn {
            game: self,
            level,
            resource_lifetime: std::marker::PhantomData,
        };
        self.level_block_state(level, pos)?;
        let here = column.fluid_at(pos);
        let Some(other) = engine_fluid_kind(fluid) else {
            return Some(false);
        };
        Some(fluid_can_be_replaced_with(here, other, direction, || {
            let Ok(height) = here.height_at(&mut column, pos);
            height
        }))
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeWorldReadGameExt1<'__sg> for G {}

/// The stored light of `layer` at `pos` in `world`, as the game's own light query reads it.
pub(in crate::game) fn world_brightness(world: &World, layer: LightLayer, pos: [i32; 3]) -> i32 {
    let light = world.light(pos[0], pos[1], pos[2]);
    i32::from(match layer {
        LightLayer::Block => light & 15,
        LightLayer::Sky => light >> 4,
    })
}

/// The engine's direction of a mod API block face.
pub fn block_face_direction(face: mod_api::BlockFace) -> sunrise_core::geometry::Direction {
    use mod_api::BlockFace;
    use sunrise_core::geometry::Direction;
    match face {
        BlockFace::Down => Direction::Down,
        BlockFace::Up => Direction::Up,
        BlockFace::North => Direction::North,
        BlockFace::South => Direction::South,
        BlockFace::West => Direction::West,
        BlockFace::East => Direction::East,
        _ => unreachable!("BlockFace: unhandled variant"),
    }
}

use crate::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
