use core::ops::ControlFlow;
use mod_api::{ChunkRef, LevelReadError, LevelRef, Vec3};
use sunrise_core::position::BlockPos;
use sunrise_level::{chunk_streamer::ChunkState, world::World};
use sunrise_world::registry::BlockStateId;

pub fn block_pos([x, y, z]: [i32; 3]) -> BlockPos {
    BlockPos::new(x, y, z)
}

/// `Level.isInWorldBoundsHorizontal`: x and z inside the 30,000,000-block limit.
pub fn horizontal_in_bounds(pos: [i32; 3]) -> bool {
    (-30_000_000..30_000_000).contains(&pos[0]) && (-30_000_000..30_000_000).contains(&pos[2])
}

pub trait NativeLevelRefsCommonGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// `LevelReader.getBiome`: the noise cell selected by this level's BiomeManager zoom.
    /// Unlike a generating server query, an absent selected column returns None.
    fn native_read_biome(&self, level: LevelRef, pos: [i32; 3]) -> Option<String> {
        let world = self.native_level_read_world(level)?;
        let streaming = if level.dimension() == self.native_level_key() {
            crate::game::Side::streaming_state(self.side_state())
        } else {
            <Self::State as crate::game::side_ops::world::SideOps>::side_parked_chunk_state(
                self.side_state(),
                level.dimension_id(),
            )?
        };
        let [qx, qy, qz] = streaming.biome_zoom.quart_at_block(block_pos(pos));
        let chunk = world.chunks.get(&(qx >> 2, qz >> 2))?;
        // ChunkAccess clamps to the dimension's height, not our padded storage.
        // Clamp before conversion to preserve signed block-coordinate extremes.
        let min_qy = world.shape.min_y >> 2;
        let max_qy = (world.shape.max_y() - 1) >> 2;
        let y = qy.clamp(min_qy, max_qy) << 2;
        let biome = chunk.biome(qx << 2, y, qz << 2);
        Some(world_biomes!(self).get(biome).name.clone())
    }

    /// The client's current copy of `LevelReader.getBiome`, never an authoritative server read.
    fn level_client_biome(&self, level: LevelRef, pos: [i32; 3]) -> Option<String> {
        if self.is_logical_server() {
            return None;
        }
        self.native_read_biome(level, pos)
    }

    /// A biome's natural spawners by category from the world's biome table.
    fn level_biome_spawners(
        &self,
        level: LevelRef,
        biome: &str,
    ) -> Option<Vec<mod_api::world::biome::BiomeSpawners>> {
        self.native_level_read_world(level)?;
        world_biomes!(self).spawners(biome)
    }

    /// A biome's own sky, fog and water colours from the world's biome table.
    fn level_biome_effects(
        &self,
        level: LevelRef,
        biome: &str,
    ) -> Option<mod_api::world::biome::BiomeEffects> {
        self.native_level_read_world(level)?;
        world_biomes!(self).effects(biome)
    }

    /// Whether a biome is in a biome tag, by the world's live tag tables.
    fn level_biome_in_tag(&self, level: LevelRef, biome: &str, tag: &str) -> Option<bool> {
        self.native_level_read_world(level)?;
        if !world_biomes!(self).contains(biome) {
            return None;
        }
        let features = self.res.behaviours().live_features()?;
        Some(features.biome_in_tag(biome, tag.strip_prefix('#').unwrap_or(tag)))
    }

    /// A biome's `Biome.ClimateSettings` from the world's biome table, data packs included.
    fn level_biome_properties(
        &self,
        level: LevelRef,
        biome: &str,
    ) -> Option<mod_api::BiomeProperties> {
        self.native_level_read_world(level)?;
        world_biomes!(self).properties(biome)
    }

    /// `ServerLevel.getLogicalHeight`: the dimension's logical height, not its build height.
    fn level_logical_height(&self, level: LevelRef) -> Option<i32> {
        Some(self.native_level_read_world(level)?.shape.logical_height)
    }

    /// `LevelData.isHardcore`: the world's flag on a server, the login packet's on a client.
    fn level_is_hardcore(&self, level: LevelRef) -> Option<bool> {
        self.native_level_read_world(level)?;
        Some(self.level.hardcore)
    }

    /// `BlockGetter.getBlockFloorHeight` with an empty collision context.
    /// Uses the normal dynamic shapes, querying below only when this shape is empty.
    fn level_block_floor_height(&self, level: LevelRef, [x, y, z]: [i32; 3]) -> Option<f64> {
        let world = self.native_level_read_world(level)?;
        if !world.loaded(x, z) || !world.shape.contains_y(y) {
            return None;
        }
        let here = self.block_collision_shape_in(world, [x, y, z]);
        let below = if here.is_empty() {
            self.block_collision_shape_in(world, [x, y.checked_sub(1)?, z])
        } else {
            sunrise_core::geometry::VoxelShape::empty()
        };
        Some(sunrise_core::geometry::block_floor_height(&here, || &below))
    }

    /// The world a handle names now, or None for a stale or foreign one.
    fn native_level_world<'a>(&'a self, level: LevelRef) -> Option<&'a World>
    where
        '__sg: 'a,
    {
        self.native_level_world_result(level).ok()
    }

    /// The single residency boundary for typed server-level reads.
    fn native_level_world_result<'__loan>(
        &'__loan self,
        level: LevelRef,
    ) -> Result<&'__loan World, mod_api::LevelReadError>
    where
        '__sg: '__loan,
    {
        if !self.is_logical_server() {
            return Err(mod_api::LevelReadError::Unavailable);
        }
        if !self.level_handle_current(level) {
            return Err(mod_api::LevelReadError::Stale);
        }
        if level.dimension() == self.dim.key() {
            return Ok(&self.world);
        }
        <Self::State as crate::game::side_ops::world::SideOps>::side_parked_world(
            self.side_state(),
            level.dimension_id(),
        )
        .ok_or(mod_api::LevelReadError::Stale)
    }

    /// The world a handle names for a read that works on both sides: the
    /// server's level (active or parked), or on a client the one it shows
    /// (`Minecraft.level`, its copy of the server's).
    fn native_level_read_world_result<'__loan>(
        &'__loan self,
        level: LevelRef,
    ) -> Result<&'__loan World, mod_api::LevelReadError>
    where
        '__sg: '__loan,
    {
        if self.is_logical_server() {
            return self.native_level_world_result(level);
        }
        if !self.side_remote_logged_in() {
            return Err(mod_api::LevelReadError::Unavailable);
        }
        if !self.level_handle_current(level) {
            return Err(mod_api::LevelReadError::Stale);
        }
        if level.dimension() != self.native_level_key() {
            return Err(mod_api::LevelReadError::Unavailable);
        }
        Ok(&self.world)
    }

    /// The world a handle names for a read that works on both sides: the
    /// server's level, or on a client the one it shows (`Minecraft.level`, its
    /// copy of the server's). None for a stale handle or another dimension.
    fn native_level_read_world<'__loan>(&'__loan self, level: LevelRef) -> Option<&'__loan World>
    where
        '__sg: '__loan,
    {
        self.native_level_read_world_result(level).ok()
    }

    /// Find a loaded chunk in an open server level: `ServerChunkCache.getChunkNow`, which also
    /// makes a FULL column one of the level's recent FULL lookups (`Streaming::note_full_read`).
    fn chunk(&self, level: LevelRef, pos: [i32; 2]) -> Option<ChunkRef> {
        let world = self.native_level_view_world(level)?;
        let key = (pos[0], pos[1]);
        if !world.chunks.contains_key(&key) {
            return None;
        }
        if let Ok(streaming) = self.native_level_streaming(level) {
            streaming.note_full_read(world, key);
        }
        Some(mod_api::__private::game::chunk_ref(level, pos))
    }

    fn level_visit_loaded_chunks(
        &self,
        level: LevelRef,
        visit: &mut dyn FnMut(ChunkRef) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, LevelReadError> {
        Ok(self
            .native_level_world_result(level)?
            .chunks
            .ordered_keys()
            .map(|(x, z)| mod_api::__private::game::chunk_ref(level, [x, z]))
            .try_for_each(visit))
    }

    fn level_visit_block_collision(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        visit: &mut dyn FnMut([f64; 6]) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, LevelReadError> {
        let world = if self.is_logical_server() {
            self.native_level_world_result(level)?
        } else {
            self.side_client_view_world_result(level)?
        };
        let shape = self.block_collision_shape_in(world, pos);
        for bounds in shape.boxes() {
            let min = bounds.min_position();
            let max = bounds.max_position();
            if visit([min.x, min.y, min.z, max.x, max.y, max.z]).is_break() {
                return Ok(ControlFlow::Break(()));
            }
        }
        Ok(ControlFlow::Continue(()))
    }

    /// The streaming of the level a handle names now, or None for a stale or
    /// foreign one.
    fn native_level_streaming<'__loan>(
        &'__loan self,
        level: LevelRef,
    ) -> Result<&'__loan ChunkState, mod_api::LevelReadError>
    where
        '__sg: '__loan,
    {
        if !self.is_logical_server() {
            return Err(mod_api::LevelReadError::Unavailable);
        }
        if !self.level_handle_current(level) {
            return Err(mod_api::LevelReadError::Stale);
        }
        if level.dimension() == self.dim.key() {
            return Ok(crate::game::Side::streaming_state(self.side_state()));
        }
        <Self::State as crate::game::side_ops::world::SideOps>::side_parked_chunk_state(
            self.side_state(),
            level.dimension_id(),
        )
        .ok_or(mod_api::LevelReadError::Stale)
    }

    /// Borrow the resident site without lending any writable host to discovery.
    fn level_with_capability_site(
        &self,
        at: mod_api::BlockSide,
        visit: &mut dyn FnMut(&dyn mod_api::LevelHost, mod_api::BlockCapabilitySite),
    ) -> Result<(), mod_api::LevelReadError> {
        if !self.level_in_place(at.level) {
            return Err(mod_api::LevelReadError::Unavailable);
        }
        let view = super::native_level_signals::ReadLevel::new(self, at.level, &self.world);
        let Some(site) = view.capability_site(at) else {
            return Err(mod_api::LevelReadError::Unavailable);
        };
        visit(&view, site);
        Ok(())
    }

    /// `FluidState.getFlow` at `pos`: the current that pushes an entity there, after the
    /// `fluid_flow` hook, as entities feel it. Zero where no fluid flows. None as for
    /// `block_state`.
    fn level_fluid_flow(&self, level: LevelRef, pos: [i32; 3]) -> Option<Vec3> {
        let world = self.native_level_read_world(level)?;
        let [x, y, z] = pos;
        world.is_loaded_at(x, y, z).then(|| {
            let flow = sunrise_sim::entities::fluid_flow(
                world,
                &self.res.motion().fluids,
                &self.loop_context,
                block_pos(pos),
            );
            Vec3::new(flow.x, flow.y, flow.z)
        })
    }

    /// The block at `pos` without its properties: `minecraft:oak_stairs`.
    /// None as for `block_state`. API 381.
    fn level_block(&self, level: LevelRef, pos: [i32; 3]) -> Option<mod_api::BlockKey> {
        self.level_block_state(level, pos)
            .and_then(|state| mod_api::__private::block_key(&self.res.facts().get(state).name))
    }

    /// Property `key` of the block state at `pos` (`BlockState.getValue`):
    /// `east` for `facing`. None for a block without it, or as for
    /// `block_state`. API 381.
    fn level_block_property(&self, level: LevelRef, pos: [i32; 3], key: &str) -> Option<&str> {
        let state = self.level_block_state(level, pos)?;
        self.pack
            .assets
            .registry
            .state(state)?
            .properties
            .get(key)
            .map(String::as_str)
    }

    /// Whether this is the level in place: the one the server ticks now, whose block entities
    /// the game's own capabilities read and write and whose capabilities
    /// `LevelRef::capability` looks up. False for a level parked while the server ticks
    /// another (its blocks can still be read), for a handle of an earlier open, and on a
    /// client.
    fn level_in_place(&self, level: LevelRef) -> bool {
        self.is_logical_server()
            && self.level_handle_current(level)
            && self.dim.key() == level.dimension()
    }

    /// Whether `pos` is inside the world's height and its chunk is loaded
    /// (`Level.isLoaded`, which is false outside `isInValidBounds`): a block
    /// read or write there finds the world. False on a client or for a stale
    /// handle: the position is not loaded here. On a client, a chunk it holds is loaded
    /// (`ClientLevel.hasChunkAt`). API 381.
    fn level_is_loaded(&self, level: LevelRef, pos: [i32; 3]) -> bool {
        self.native_level_read_world(level)
            .is_some_and(|w| w.is_loaded_at(pos[0], pos[1], pos[2]))
    }

    fn level_get_light_emission(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
    ) -> Result<u8, mod_api::LevelReadError> {
        use mod_api::LevelReadError;
        if self.native_level_read_world(level).is_none() {
            return Err(if self.level_handle_current(level) {
                LevelReadError::Unavailable
            } else {
                LevelReadError::Stale
            });
        }
        let light = &self.pack.chunk_context.light;
        if light.is_dynamic(state.0)
            && let Some(answer) = self
                .query_dynamic_light_emission(block_pos(pos), state, || {
                    crate::game::light_emission::light_emission_dimension(level)
                })
                .ok_or(LevelReadError::Unavailable)?
        {
            return Ok(answer);
        }
        Ok(light.emission(state.0))
    }

    /// The stored light of one layer at `pos` (`BlockAndLightGetter.getBrightness`), 0 to 15.
    /// As with `block_state`, a client reads the named level, shown or open for viewing.
    /// Returns None if the level is unavailable or the position isn't loaded.
    /// `BlockAndLightGetter.canSeeSky`: whether the local sky light at `pos` is full.
    fn level_can_see_sky(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool> {
        Some(crate::game::native_world_read_common::can_see_sky(
            self.native_level_read_world(level)?,
            pos,
        ))
    }

    /// The lowest block `y` and the height of the level
    /// (`LevelHeightAccessor.getMinY`, `getHeight`). Reads any loaded server level
    /// or the client's current level. None for a stale handle, before client login,
    /// or for a dimension unavailable on this side. Does not load chunks. API 381.
    fn level_height(&self, level: LevelRef) -> Option<(i32, i32)> {
        self.native_level_read_world(level)
            .map(|w| (w.shape.min_y, w.shape.height))
    }

    /// `Level.isInWorldBounds`: `pos` within 30,000,000 blocks on x and z and inside the level's
    /// build height. None as for `level_height`. Does not load chunks.
    fn level_in_world_bounds(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool> {
        let (min_y, height) = self.level_height(level)?;
        Some(horizontal_in_bounds(pos) && pos[1] >= min_y && pos[1] < min_y.saturating_add(height))
    }

    /// `Level.isInSpawnableBounds`: 30,000,000 blocks on x and z, 20,000,000 on y. None as for
    /// `level_height`. Does not load chunks.
    fn level_in_spawnable_bounds(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool> {
        self.native_level_read_world(level)?;
        Some(horizontal_in_bounds(pos) && (-20_000_000..20_000_000).contains(&pos[1]))
    }

    /// The ticks the server has run (`Level.getGameTime`), shared by every
    /// level as in vanilla. API 381.
    fn level_game_time(&self, level: LevelRef) -> Option<i64> {
        self.native_level_read_world(level)?;
        Some(self.world.game_time)
    }

    /// The time of day in ticks (`Level.getOverworldClockTime`), shared by
    /// every level; it keeps counting past 24000. On a client, the synchronized
    /// Overworld clock, advanced at its server-supplied rate, even when the
    /// displayed dimension has a different default clock. API 381.
    fn level_day_time(&self, level: LevelRef) -> Option<i64> {
        self.native_level_read_world(level)?;
        if self.is_logical_server() {
            Some(self.command_clock_time("minecraft:overworld"))
        } else {
            self.side_remote_overworld_clock()
        }
    }

    /// Whether `level` names the level now open under its id: the world open it was made in, and
    /// for a runtime level the open it was given. A handle of an earlier open reads as stale.
    fn level_handle_current(&self, level: LevelRef) -> bool {
        self.side_level_handle_current(level)
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeLevelRefsCommonGameExt1<'__sg> for G {}

use crate::game::side_ops::modapi_modding::ModapiModdingSideGameExt as _;

use crate::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use crate::game::native_hooks::light_emission::GameHooks as _;
