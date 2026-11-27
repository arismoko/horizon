use mod_api::{FluidKey, FluidState, LevelRef};
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
use sunrise_gameplay_common::game::native_world_read_common::engine_fluid_kind;
use sunrise_gameplay_common::game::{GameLevel, native_level_signals::ReadLevel};
use sunrise_world::level::fluid_tick_delay;

impl crate::game::ServerGame<'_> {
    /// The fluid state at `pos` of the level `level` names (`BlockGetter.getFluidState`). None
    /// for a stale handle, a chunk that isn't loaded or a `y` outside the world: a read never
    /// loads a chunk.
    pub fn level_fluid_state(&self, level: LevelRef, pos: [i32; 3]) -> Option<FluidState> {
        let state = self.level_block_state(level, pos)?;
        super::native_block_tables::BlockTables::of(self)
            .block_state_fluid_state(state)
            .ok()
            .flatten()
    }

    /// `Fluid.getTickDelay(level)` of `fluid` at `pos` in the level in place: vanilla's by its
    /// family and the dimension (`World::fast_lava`), a mod fluid's as registered and as its
    /// `fluid_rules` parts change it there (the one rule path the flow itself reads). None for
    /// another level, a stale handle, a client, an unloaded or out-of-height position and a
    /// fluid with no world form.
    pub(in crate::game) fn level_fluid_tick_delay(
        &self,
        level: LevelRef,
        fluid: FluidKey,
        pos: [i32; 3],
    ) -> Option<i32> {
        let [x, y, z] = pos;
        if !self.level_in_place(level) || !self.common.world.is_loaded_at(x, y, z) {
            return None;
        }
        let kind = engine_fluid_kind(fluid)?;
        let view = ReadLevel::new(self, level, &self.common.world);
        Some(fluid_tick_delay(kind, self.common.world.fast_lava, || {
            let fluid = kind.mod_fluid()?;
            Some(GameLevel::answered_fluid_rules(
                self.common.dim.level(),
                BlockPos::new(x, y, z),
                fluid,
                fluid.get().rules,
                |call| mod_api::dispatch_fluid_rules(&view, call),
            ))
        }))
    }
}

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
