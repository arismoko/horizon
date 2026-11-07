//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `levels` area, implemented by the game and reached through [`Host`].
#[horizon_mod_api_macros::host_area]
pub trait LevelsHost {
    /// Side: both.
    /// Whether a mod with the `horizon-mod.toml` id `id` is installed: a
    /// native, wasm or content package (Fabric's `FabricLoader.isModLoaded`,
    /// NeoForge's `ModList.isLoaded`). The list is fixed at startup.
    #[access(Metadata)]
    #[static_call]
    fn is_loaded(&self, id: &str) -> bool;
    /// Side: both.
    /// Find a loaded chunk in an open server level.
    #[access(WorldOnly)]
    fn chunk(&self, level: LevelRef, pos: [i32; 2]) -> Option<ChunkRef>;
}
/// The handles' reads of the `levels` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait LevelsBackend {
    /// Side: server.
    /// `ServerLevel.findClosestBiome3d`: search this level's generator for an id or `#tag`.
    /// The result follows vanilla's sampled spiral and vertical order; it is not a search
    /// through edited biome cells. Does not load resident chunks or switch parked levels.
    /// None for no match, a stale/unavailable level, an unavailable generator, a negative
    /// radius, nonpositive sampling steps, or a potential sample count above
    /// [`BiomeSearch::MAX_SAMPLES`] (see [`BiomeSearch::new`] for the work formula).
    /// Positive vertical steps larger than the build height sample only the clamped origin.
    #[access(WorldOnly)]
    fn level_find_closest_biome(
        &self,
        level: LevelRef,
        biome: &str,
        parameters: BiomeSearch,
    ) -> Option<BiomeSearchResult>;
    /// Side: both.
    /// `Level.getSkyDarken`: the sky-light reduction from dimension, time and weather.
    /// Reads any loaded server level, including parked levels, or the current client level.
    /// None for a stale or unavailable level. Does not load chunks.
    #[access(WorldOnly)]
    fn level_get_sky_darken(&self, level: LevelRef) -> Option<i32>;
    /// Side: both.
    /// `BlockGetter.getBlockFloorHeight` with the normal collision shapes and an empty
    /// entity context. The result is relative to the block: negative infinity means no floor.
    /// None for a stale level, an unloaded column or a position outside build height.
    /// Reads parked server levels and the current client level without loading chunks.
    #[access(WorldOnly)]
    fn level_block_floor_height(&self, level: LevelRef, pos: [i32; 3]) -> Option<f64>;
    /// Side: server.
    /// The biome id selected by `LevelReader.getBiome` and this level's biome zoom.
    /// Reads an active or parked server level. None for a
    /// stale handle or an unloaded selected noise column. Never generates or loads chunks.
    #[access(WorldOnly)]
    fn level_biome_at(&self, level: LevelRef, pos: [i32; 3]) -> Option<String>;
    /// Side: client.
    /// The biome in the client's current level (`LevelReader.getBiome`). None before
    /// login, for a stale or foreign dimension, or an unloaded selected column. Never loads chunks.
    #[access(WorldOnly)]
    fn level_client_biome(&self, level: LevelRef, pos: [i32; 3]) -> Option<String>;
    /// Side: both.
    /// The dimension's logical height (`ServerLevel.getLogicalHeight`), which may differ
    /// from build height. Reads active and parked server levels or the client's current level.
    /// None for a stale or unavailable level. Does not load chunks.
    #[access(WorldOnly)]
    fn level_logical_height(&self, level: LevelRef) -> Option<i32>;
    /// Side: server.
    /// The level's world border as [min_x, min_z, max_x, max_z], using the
    /// previous size while it moves. Horizon currently shares one border
    /// across its dimensions. None for a stale handle or a closed level.
    #[access(WorldOnly)]
    fn level_world_border(&self, level: LevelRef) -> Option<[f64; 4]>;
    /// Side: both.
    /// The name of a dimension's key.
    #[access(Metadata)]
    fn dimension_key_id(&self, key: DimensionKey) -> Option<String>;
    /// Side: server.
    /// A handle for chunk `pos` of an open level, loaded or not, so a mod can
    /// force an unloaded chunk. None for a stale handle, on a client, and for
    /// a chunk outside the world (`ChunkPos` past 30 million blocks).
    #[access(WorldOnly)]
    fn level_chunk_ref(&self, level: LevelRef, pos: [i32; 2]) -> Option<ChunkRef>;
    /// Side: server.
    /// Whether `/forceload` holds the chunk (`ServerLevel.getForcedChunks`),
    /// loaded or not; a mod's own ticket on it does not count. None for a stale handle or on a client.
    #[access(WorldOnly)]
    fn chunk_forced(&self, chunk: ChunkRef) -> Option<bool>;
    /// Side: server.
    /// The level's force-loaded chunks (`/forceload query`), sorted as
    /// the command lists them. None for a stale handle or on a client.
    #[access(WorldOnly)]
    fn level_forced_chunks(&self, level: LevelRef) -> Option<Vec<[i32; 2]>>;
    /// Side: server.
    /// The chunk tickets `controller` holds in the level, as (owner, chunk
    /// position, natural_spawning), in order. None for a stale handle, on a
    /// client, outside a mod's hook and for another mod's controller.
    #[access(WorldOnly)]
    fn level_mod_forced_chunks(
        &self,
        level: LevelRef,
        controller: TicketController,
    ) -> Option<Vec<(NativeTicketOwner, [i32; 2], bool)>>;
    /// Side: server.
    /// Whether this chunk is loaded; false on a client or for a stale handle.
    #[access(WorldOnly)]
    fn chunk_loaded(&self, chunk: ChunkRef) -> bool;
    /// Side: server.
    /// The status of a loaded chunk. Proto chunks are not exposed here.
    #[access(WorldOnly)]
    fn chunk_status(&self, chunk: ChunkRef) -> Option<ChunkStatus>;
    /// Side: server.
    /// `RecipeManager.getRecipeFor(CRAFTING, CraftingInput.of(width, height, items), level)`,
    /// then the recipe's `assemble` and `getRemainingItems`: what a `width` by `height` grid
    /// (each 1 to 3, `items` row by row) makes, through the crafting table's matcher, with the
    /// world's data packs' recipes and the special recipes. None when no recipe matches or it
    /// makes nothing, for a grid of another size, and off the server.
    #[access(WorldOnly)]
    fn level_craft(
        &self,
        level: LevelRef,
        width: u32,
        height: u32,
        items: &[Option<ItemStack>],
    ) -> Option<CraftResult>;
    /// Side: server.
    /// Entities whose bounding boxes intersect this box (`Level.getEntities`).
    /// Face-only contact does not intersect; spectator players are excluded.
    /// Returns None on a client, for a stale handle, or for a parked level:
    /// its entity handles cannot resolve until the level is in place. The walk
    /// is in entity-list order followed by players; each overlap requires a
    /// handle lookup, so cost grows with the box's contents. Tridents cannot
    /// yet resolve; frames, paintings, other special entities and the dragon
    /// are not on the entity-list walk.
    /// During `hears_projectile_impacts`, the projectile list is taken out of
    /// the game and is not visible; it is restored for `projectile_impact`.
    #[access(WorldOnly)]
    fn level_entities_in(
        &self,
        level: LevelRef,
        min: [f64; 3],
        max: [f64; 3],
    ) -> Option<Vec<EntityRef>>;
    /// Side: both.
    /// The block state ID at `pos` (`Level.getBlockState`), without formatting a string.
    /// None for a chunk that isn't loaded or a `y` outside the world. On a client, the
    /// state is the one the server sent for a chunk the client holds.
    #[access(WorldOnly)]
    fn level_block_state(&self, level: LevelRef, pos: [i32; 3]) -> Option<BlockStateId>;
    /// Side: server.
    /// Preview a held tool's transformation without changing the block or stack. The existing
    /// block-tool hook fires with `simulated` true; listeners must not mutate the level in a
    /// simulation. The player must be live in the level in place, and the block must be loaded;
    /// parked levels cannot provide the hook's level context. A stale handle, invalid face,
    /// unavailable player or level, or no transformation returns None.
    #[access(WorldOnly)]
    fn level_block_transform(
        &self,
        level: LevelRef,
        player: PlayerRef,
        hand: Hand,
        stack: &ItemStack,
        pos: [i32; 3],
        hit: BlockHit,
    ) -> Option<BlockTransformPreview>;
    /// Side: both.
    /// The block at `pos` without its properties: `minecraft:oak_stairs`.
    /// None as for `block_state`. API 381.
    #[access(WorldOnly)]
    fn level_block(&self, level: LevelRef, pos: [i32; 3]) -> Option<String>;
    /// Side: both.
    /// Property `key` of the block state at `pos` (`BlockState.getValue`):
    /// `east` for `facing`. None for a block without it, or as for
    /// `block_state`. API 381.
    #[access(WorldOnly)]
    fn level_block_property(&self, level: LevelRef, pos: [i32; 3], key: &str) -> Option<String>;
    /// Side: server.
    /// Whether this is the level in place: the one the server ticks now, whose block entities
    /// the game's own capabilities read and write and whose capabilities
    /// `LevelRef::capability` looks up. False for a level parked while the server ticks
    /// another (its blocks can still be read), for a handle of an earlier open, and on a
    /// client.
    #[access(WorldOnly)]
    fn level_in_place(&self, level: LevelRef) -> bool;
    /// Side: both.
    /// Whether `pos` is inside the world's height and its chunk is loaded
    /// (`Level.isLoaded`, which is false outside `isInValidBounds`): a block
    /// read or write there finds the world. False on a client or for a stale
    /// handle: the position is not loaded here. On a client, a chunk it holds is loaded
    /// (`ClientLevel.hasChunkAt`). API 381.
    #[access(WorldOnly)]
    fn level_is_loaded(&self, level: LevelRef, pos: [i32; 3]) -> bool;
    /// Side: server.
    /// The stored light of one layer at `pos` (`BlockAndLightGetter.getBrightness`), 0 to 15.
    /// None as for `block_state`: a position that isn't loaded has no light to read.
    #[access(WorldOnly)]
    fn level_brightness(&self, level: LevelRef, layer: LightLayer, pos: [i32; 3]) -> Option<i32>;
    /// Side: both.
    /// The lowest block `y` and the height of the level
    /// (`LevelHeightAccessor.getMinY`, `getHeight`). Reads any loaded server level
    /// or the client's current level. None for a stale handle, before client login,
    /// or for a dimension unavailable on this side. Does not load chunks. API 381.
    #[access(WorldOnly)]
    fn level_height(&self, level: LevelRef) -> Option<(i32, i32)>;
    /// Side: both.
    /// `Level.isInWorldBounds`: `pos` within 30,000,000 blocks on x and z and inside the level's
    /// build height. None as for `native_level_height`. Does not load chunks.
    #[access(WorldOnly)]
    fn level_in_world_bounds(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool>;
    /// Side: both.
    /// `Level.isInSpawnableBounds`: 30,000,000 blocks on x and z, 20,000,000 on y. None as for
    /// `native_level_height`. Does not load chunks.
    #[access(WorldOnly)]
    fn level_in_spawnable_bounds(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool>;
    /// Side: server.
    /// The ticks the server has run (`Level.getGameTime`), shared by every
    /// level as in vanilla. API 381.
    #[access(WorldOnly)]
    fn level_game_time(&self, level: LevelRef) -> Option<i64>;
    /// Side: both.
    /// The time of day in ticks (`Level.getOverworldClockTime`), shared by
    /// every level; it keeps counting past 24000. On a client, the synchronized
    /// Overworld clock, advanced at its server-supplied rate, even when the
    /// displayed dimension has a different default clock. API 381.
    #[access(WorldOnly)]
    fn level_day_time(&self, level: LevelRef) -> Option<i64>;
    /// Side: both.
    /// `FluidState.getFlow` at `pos`: the current that pushes an entity there, after the
    /// `fluid_flow` hook, as entities feel it. Zero where no fluid flows. None as for
    /// `block_state`.
    #[access(WorldOnly)]
    fn level_fluid_flow(&self, level: LevelRef, pos: [i32; 3]) -> Option<Vec3>;
    /// Side: server.
    /// Block-local outline or collision boxes with an empty context. Interaction,
    /// visual, occlusion and support shapes are not exposed. Returns None on a
    /// client, for a stale handle, or for a closed level. Unloaded blocks read
    /// as air; collision errors return no boxes, like `native_block_collision`.
    #[access(WorldOnly)]
    fn level_block_shape(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        kind: NativeBlockShapeKind,
    ) -> Option<Vec<[f64; 6]>>;
    /// Side: server.
    /// Clip an empty-context ray against collider or outline shapes, without fluids.
    /// None on failed reads, dynamic shapes, invalid rays or traversal limits.
    #[access(WorldOnly)]
    fn level_clip(
        &self,
        level: LevelRef,
        from: [f64; 3],
        to: [f64; 3],
        blocks: NativeClipBlock,
        fluids: NativeClipFluid,
    ) -> Option<NativeClipHit>;
    /// Side: server.
    /// Gameplay's face support in the current or a parked server level, including
    /// live block entities for dynamic shapes. None for stale or closed levels,
    /// unloaded blocks, unsupported states or a shape query error.
    #[access(WorldOnly)]
    fn level_block_face_sturdy(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        face: NativeBlockFace,
        support: NativeSupportType,
    ) -> Option<bool>;
    /// Side: server.
    /// How strongly the block at `pos` powers the block on its `direction` side, power it
    /// conducts included (`SignalGetter.getSignal(pos, direction)`, as a block reads its
    /// neighbour `pos` that lies `direction` from it), 0 to 15. None for another level, a stale
    /// handle or a client. Takes a shared host and opens no transaction.
    #[access(WorldOnly)]
    fn level_signal(&self, level: LevelRef, pos: [i32; 3], direction: Direction) -> Option<i32>;
    /// Side: server.
    /// The strong power alone that the block at `pos` gives that way
    /// (`SignalGetter.getDirectSignal(pos, direction)`), 0 to 15. None as for `signal`.
    #[access(WorldOnly)]
    fn level_direct_signal(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        direction: Direction,
    ) -> Option<i32>;
    /// Side: server.
    /// The strongest strong power reaching the block at `pos` from its six neighbours
    /// (`SignalGetter.getDirectSignalTo`), 0 to 15. None as for `signal`.
    #[access(WorldOnly)]
    fn level_direct_signal_to(&self, level: LevelRef, pos: [i32; 3]) -> Option<i32>;
    /// Side: server.
    /// The strongest power reaching the block at `pos` from its six neighbours
    /// (`SignalGetter.getBestNeighborSignal`, what a dispenser or a piston reads), 0 to 15.
    /// None as for `signal`.
    #[access(WorldOnly)]
    fn level_best_neighbor_signal(&self, level: LevelRef, pos: [i32; 3]) -> Option<i32>;
    /// Side: server.
    /// Whether any neighbour powers the block at `pos` (`SignalGetter.hasNeighborSignal`),
    /// asking no side past the first that does. False where `signal` is None.
    #[access(WorldOnly)]
    fn level_has_neighbor_signal(&self, level: LevelRef, pos: [i32; 3]) -> bool;
    /// Side: both.
    /// Dimension shape of a loaded server level, including parked levels.
    #[access(WorldOnly)]
    fn level_dimension_shape(&self, level: LevelRef) -> Option<LevelDimensionShape>;
    /// Side: server.
    /// The world's seed (`WorldOptions.seed`), as `/seed` shows it.
    #[access(WorldOnly)]
    fn level_seed(&self, level: LevelRef) -> Option<i64>;
    /// Side: server.
    /// The difficulty (`Level.getDifficulty`): `peaceful`, `easy`, `normal`
    /// or `hard`, as `/difficulty` names it.
    #[access(WorldOnly)]
    fn level_difficulty(&self, level: LevelRef) -> Option<&'static str>;
    /// Side: server.
    /// Read a registered boolean game rule.
    #[access(WorldOnly)]
    fn level_game_rule_bool(&self, level: LevelRef, key: BoolGameRuleKey) -> Option<bool>;
    /// Side: server.
    /// Game rule `rule` (`minecraft:keep_inventory` or `keep_inventory`), as
    /// `/gamerule` queries it: None for an unknown rule.
    #[access(WorldOnly)]
    fn level_game_rule(&self, level: LevelRef, rule: &str) -> Option<GameRuleValue>;
    /// Side: server.
    /// Every game rule the world has (`GameRules`) with its current value, by `minecraft:` id
    /// (`max_minecart_speed` only with minecart improvements): None for a stale handle and on
    /// a client.
    #[access(WorldOnly)]
    fn level_game_rules(&self, level: LevelRef) -> Option<Vec<(String, GameRuleValue)>>;
    /// Side: both.
    /// The rain and thunder levels, each 0.0 to 1.0 (`Level.getRainLevel`,
    /// `getThunderLevel` at partial tick 1.0): both 0 in a level with no weather.
    /// Reads any open server level or the client's current synchronized level;
    /// None for a stale handle, an unavailable level or a client not logged in.
    #[access(WorldOnly)]
    fn level_weather(&self, level: LevelRef) -> Option<(f32, f32)>;
    /// Side: server.
    /// Whether it rains (`Level.isRaining`: a rain level above 0.2).
    #[access(WorldOnly)]
    fn level_is_raining(&self, level: LevelRef) -> Option<bool>;
    /// Side: server.
    /// `Level.isRainingAt(pos)` in the level a handle names, the one in place or a parked one:
    /// it rains there and `pos` is under open sky in a biome that rains. None for a stale
    /// handle, on a client, and where `pos` is not loaded or outside the world's height; a
    /// read never loads a chunk.
    #[access(WorldOnly)]
    fn level_is_raining_at(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool>;
    /// Side: server.
    /// `LevelReader.enabledFeatures` of the level a handle names. Every level of a world shares
    /// the world's flags. None for a stale handle and on a client.
    #[access(WorldOnly)]
    fn level_enabled_features(&self, level: LevelRef) -> Option<FeatureFlags>;
    /// Side: server.
    /// Whether it thunders (`Level.isThundering`: a thunder level above 0.9).
    #[access(WorldOnly)]
    fn level_is_thundering(&self, level: LevelRef) -> Option<bool>;
    /// Side: server.
    /// The random of the server level `level` names: vanilla's `Level.random`
    /// (`level.getRandom()`), the one its explosions, block drops and other
    /// level draws use, so a mod's draws move vanilla's later ones as a Java
    /// mod's do. A parked level (the Nether while the server ticks the
    /// Overworld) answers with its own stream. This is a copy of it as it
    /// stands: draw with `LevelRef::with_random`, which stores the stream
    /// back, so the draws move the level's own. `None` on a client of a
    /// server, for a stale handle and for a level not open: the Nether and
    /// the End open when someone first goes there and close after 300 ticks
    /// with nobody in them (`close_idle_levels`).
    #[access(WorldOnly)]
    fn level_random(&self, level: LevelRef) -> Option<LegacyRandom>;
    /// Side: server.
    /// `Raids.get(id)`: the raid saved under `id` in the requested level, if
    /// `Raids` still holds it (a stopped raid goes at its next tick).
    #[access(WorldOnly)]
    fn level_raid(&self, level: LevelRef, id: i32) -> Option<RaidRef>;
    /// Side: server.
    /// `ServerLevel.getRaidAt(pos)`: the closest active raid whose centre is
    /// less than 96 blocks away (`Raids.getNearbyRaid`, 9216 squared).
    #[access(WorldOnly)]
    fn level_raid_at(&self, level: LevelRef, pos: [i32; 3]) -> Option<RaidRef>;
    /// Side: server.
    /// Every raid `Raids` holds, in its iteration order, stopped ones
    /// included until their next tick. None for a closed or stale level.
    #[access(WorldOnly)]
    fn level_raids(&self, level: LevelRef) -> Option<Vec<RaidRef>>;
    /// Side: server.
    /// The fluid state at `pos` of the level `level` names (`BlockGetter.getFluidState`). None
    /// for a stale handle, a chunk that isn't loaded or a `y` outside the world: a read never
    /// loads a chunk.
    #[access(WorldOnly)]
    fn level_fluid_state(&self, level: LevelRef, pos: [i32; 3]) -> Option<FluidState>;
    /// Side: both.
    /// Every loaded chunk of an open server level as handles, ordered by chunk x then z. None for
    /// a stale handle or on a client. Loads nothing.
    #[access(WorldOnly)]
    fn level_loaded_chunks(&self, level: LevelRef) -> Option<Vec<ChunkRef>>;
    /// Side: both.
    /// World-space block collision boxes under an empty collision context, without entities or border. Refuses a stale or nonresident level and invalid or overlarge boxes.
    #[access(WorldOnly)]
    fn level_collision_boxes(&self, level: LevelRef, aabb: [f64; 6]) -> Option<Vec<[f64; 6]>>;
}
/// The handles' writes of the `levels` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait LevelsBackendMut {
    /// Side: server.
    /// `/forceload add` or `remove` of one chunk (`ServerLevel.setChunkForced`):
    /// true when the forced set changed. A forced chunk is kept loaded and
    /// entity ticking, saved with the level, and the level keeps ticking
    /// without a player. This is the world's own `/forceload` state, not a
    /// ticket the mod owns: the chunk stays forced across restarts, and after
    /// the mod is unloaded or removed, until `set_forced(false)` or
    /// `/forceload remove` clears it. Unlike vanilla, the chunk is not loaded before
    /// this returns: it loads over the next ticks, as after `/forceload`.
    /// False for a stale handle, on a client, and for a level the server
    /// has not opened.
    #[access(WorldOnly)]
    fn chunk_set_forced(&mut self, chunk: ChunkRef, forced: bool) -> bool;
    /// Side: server.
    /// The chunk ticket of `controller` for `owner` on `chunk`
    /// (`TicketController.forceChunk`): true when the controller's tickets
    /// changed. The controller must be the calling mod's own.
    /// A chunk stays forced, entity ticking and saved, and its level keeps
    /// ticking, while any ticket or `/forceload` holds it. With
    /// `natural_spawning` the chunk joins the spawning chunks (inhabited
    /// time, thunder, a spawn attempt) while it lies in the spawning area of a
    /// player (`ChunkMap.collectSpawningChunks`); the local mob cap still lets
    /// mobs spawn only with a non-spectator within 128 blocks. One
    /// owner holds a plain and a natural-spawning ticket on a chunk apart. Tickets are
    /// scoped by (controller, owner, chunk), so two owners in one chunk, or
    /// two controllers, release separately. They are saved in
    /// `data/horizon/mod_chunk_tickets.dat` under the controller's id; when
    /// the world loads, those of a controller no loaded mod registers are
    /// discarded. False outside a mod's hook, for another mod's controller, on
    /// a client, for a chunk outside the world and for a level the server has
    /// not opened.
    #[access(WorldOnly)]
    fn chunk_set_mod_forced(
        &mut self,
        chunk: ChunkRef,
        controller: TicketController,
        owner: NativeTicketOwner,
        forced: bool,
        natural_spawning: bool,
    ) -> bool;
    /// Side: server.
    /// Set a block in the level in place (`Level.setBlock`). Parked levels,
    /// stale handles, clients and unloaded or out-of-height positions return
    /// false. Client notification follows the supplied update flags.
    #[access(WorldOnly)]
    fn level_set_block(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
        flags: NativeBlockUpdateFlags,
    ) -> bool;
    /// Side: server.
    /// Remove a block in place (`Level.removeBlock`), preserving its fluid.
    #[access(WorldOnly)]
    fn level_remove_block(&mut self, level: LevelRef, pos: [i32; 3], moved: bool) -> bool;
    /// Side: server.
    /// Destroy a block in the level in place (`Level.destroyBlock`), optionally dropping
    /// resources with `tool` (None is an empty hand). With `collect`, return the dropped
    /// stacks without spawning their item entities; otherwise return an empty list after
    /// spawning them. The one level drop path runs loot, block-drop hooks and experience
    /// once, in vanilla's random order. `drop` false suppresses resources entirely.
    /// None means nothing broke: air, a refused write, a stale handle, a client, an
    /// unloaded/out-of-height position, or a parked level.
    #[access(WorldOnly)]
    fn level_destroy_block(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        drop: bool,
        tool: Option<&ItemStack>,
        collect: bool,
    ) -> Result<Option<Vec<ItemStack>>, StackWriteError>;
    /// Side: server.
    /// Send `ServerLevel.destroyBlockProgress` in this level: stages 0..9 show a crack,
    /// any other stage removes it. A machine uses a stable negative breaker id.
    #[access(WorldOnly)]
    fn level_set_break_progress(
        &mut self,
        level: LevelRef,
        breaker: i32,
        pos: [i32; 3],
        progress: i32,
    ) -> bool;
    /// Side: server.
    /// Plays sound event `sound` at `at` for the players in this level
    /// within range (16 blocks, times the volume above 1), as
    /// `ServerLevel.playSound(null, ...)` does, under `source`'s volume
    /// slider. A `minecraft:` event must be registered; a mod's goes by name
    /// and is heard where its resources define it. True means queued, not
    /// heard.
    #[access(WorldOnly)]
    fn level_play_sound(
        &mut self,
        level: LevelRef,
        sound: &str,
        at: [f64; 3],
        source: SoundSource,
        volume: f32,
        pitch: f32,
    ) -> bool;
    /// Side: server.
    /// Sends `count` particles around `at` to the players in this level
    /// within 32 blocks, as `ServerLevel.sendParticles` does: each offset is
    /// a Gaussian times `spread`, and `speed` scales their motion (a zero
    /// count sends one particle moving along `spread`). Block particle types
    /// need `state`; item, trail and vibration particles aren't supported.
    /// True means queued, not drawn.
    #[access(WorldOnly)]
    fn level_send_particles(
        &mut self,
        level: LevelRef,
        options: &ParticleOptions,
        state: Option<BlockStateId>,
        at: [f64; 3],
        spread: [f64; 3],
        speed: f64,
        count: i32,
    ) -> bool;
    /// Side: server.
    /// Plays level event `id` at `pos` with `data` for the players near it
    /// (`Level.levelEvent(null, id, pos, data)`): `1000`s are sounds, `2000`s
    /// particles, as vanilla's `LevelEvent` numbers them (`2001` breaks a
    /// block with state id `data`). No player is left out, the acting one
    /// included. False for an event that carries a block state (2001, 2014,
    /// 3008) when `data` isn't one.
    #[access(WorldOnly)]
    fn level_level_event(&mut self, level: LevelRef, id: i32, pos: [i32; 3], data: i32) -> bool;
    /// Side: server.
    /// Sends game event `event` (`minecraft:block_change`...) from `at`, as
    /// `Level.gameEvent(null, event, at)` does: sculk sensors, shriekers,
    /// wardens and allays in range hear it. False for an event vanilla
    /// doesn't have.
    #[access(WorldOnly)]
    fn level_game_event(&mut self, level: LevelRef, event: &str, at: [f64; 3]) -> bool;
    /// Side: server.
    /// Explodes at `center` with no source entity, as `Level.explode(null,
    /// x, y, z, radius, fire, interaction)` does (`native_explode`): `fire`
    /// lights some of the air around, `interaction` says what happens to
    /// blocks: `None`, `Block` or `Tnt` (vanilla's `Mob` and `Trigger` aren't
    /// offered yet). False for a chunk that isn't loaded or a radius outside
    /// `0 < radius <= 32`.
    #[access(WorldOnly)]
    fn level_explode(
        &mut self,
        level: LevelRef,
        center: [f64; 3],
        radius: f32,
        fire: bool,
        interaction: NativeExplosionInteraction,
    ) -> bool;
    /// Side: server.
    /// Strikes lightning at `at`, as summoning a `minecraft:lightning_bolt`
    /// does: it sets fire, hurts and converts what it hits (a pig to a
    /// zombified piglin...) and every player hears the thunder. With
    /// `visual_only` it only flashes and sounds (`setVisualOnly`). The
    /// `lightning_strike` hook hears it first with the cause `mod` and may
    /// cancel it. False for a chunk that isn't loaded or a canceled strike.
    #[access(WorldOnly)]
    fn level_strike_lightning(&mut self, level: LevelRef, at: [f64; 3], visual_only: bool) -> bool;
    /// Side: server.
    /// Drops `stack` as a broken block drops it (`Block.popResource`): an
    /// item entity at a random point within a quarter block of the middle of
    /// `pos` (three draws from the level random, made for an empty stack and
    /// with `block_drops` off too), with its own random motion and the
    /// default pickup delay of 10 ticks. None, with nothing spawned, for an
    /// empty stack or an unknown item, with the `block_drops` rule off, on a
    /// client, for a stale handle, a level not in place, or an item a mod
    /// kept out (`native_entity_join_level`).
    #[access(WorldOnly)]
    fn level_pop_resource(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        stack: ItemStack,
    ) -> Result<Option<EntityRef>, StackWriteError>;
    /// Side: server.
    /// Spawns `stack` as an item entity at `x y z` (`new ItemEntity(level, x,
    /// y, z, stack)` and `addFreshEntity`): its own random motion (up 0.2, up
    /// to 0.1 either way across), with `pickup_delay` ticks before it can be
    /// picked up, as `ItemEntity.setPickupDelay` at construction (vanilla
    /// passes 10 for a block's drop, 40 for a player's throw and 32767 for
    /// never). None, with nothing
    /// spawned, for an empty stack or an unknown item, a value that isn't
    /// finite, on a client, for a stale handle, a level not in place, or an
    /// item a mod kept out (`native_entity_join_level`).
    #[access(WorldOnly)]
    fn level_spawn_item(
        &mut self,
        level: LevelRef,
        x: f64,
        y: f64,
        z: f64,
        stack: ItemStack,
        pickup_delay: u32,
    ) -> Result<Option<EntityRef>, StackWriteError>;
    /// Side: server.
    /// `spawn_item` with the item entity's motion set to
    /// `motion` (blocks a tick), as `new ItemEntity(level, x, y, z, stack,
    /// deltaX, deltaY, deltaZ)`, and the same `pickup_delay` ticks before it
    /// can be picked up (`ItemEntity.setPickupDelay`). None in the same
    /// cases, and for a motion that isn't finite.
    #[access(WorldOnly)]
    fn level_spawn_item_with_motion(
        &mut self,
        level: LevelRef,
        x: f64,
        y: f64,
        z: f64,
        stack: ItemStack,
        motion: [f64; 3],
        pickup_delay: u32,
    ) -> Result<Option<EntityRef>, StackWriteError>;
    /// Side: server.
    /// Awards `amount` experience at `x y z` as vanilla's `ExperienceOrb.award`
    /// does: orbs of the largest sizes first (2477, 1237, ... 3, 1), each
    /// drawing the id `tryMergeToExisting` compares from the level random and
    /// merging into an orb of the same value whose box meets the 1 block box
    /// around `x y z`, or else a new orb with its own random motion. The
    /// `block_drops` rule doesn't apply (vanilla's blocks check it before
    /// they award). True once the orbs are awarded, even if a mod keeps a new
    /// one out (`native_entity_join_level`). False, with no orbs, for 0, a
    /// value that isn't finite, on a client, for a stale handle or a level
    /// not in place.
    #[access(WorldOnly)]
    fn level_award_experience(
        &mut self,
        level: LevelRef,
        x: f64,
        y: f64,
        z: f64,
        amount: u32,
    ) -> bool;
    /// Side: server.
    /// Tell the neighbours of `pos` that a block changed there, as vanilla's own writes do:
    /// `Neighbours` (`Level.updateNeighborsAt`: a lever's or a button's own block, and the one
    /// it is attached to, when the signal it gives changes), `NeighboursExcept(direction)`
    /// (`updateNeighborsAtExceptFromFacing`: a diode's front) or `OutputSignal`
    /// (`updateNeighbourForOutputSignal`: the comparators beside it, when its comparator output
    /// changes without its state changing). `source` is the state whose block vanilla passes as
    /// the one that changed. False, with nothing done, off the level in place, for a position
    /// that isn't loaded, or for a `source` that is no block state.
    #[access(WorldOnly)]
    fn level_update_neighbors(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        source: BlockStateId,
        update: NeighbourUpdate,
    ) -> bool;
    /// Side: server.
    /// Tell the block at `pos` that `source`'s block beside it changed (`Level.neighborChanged`),
    /// as a diode tells the block in front of it before updating that block's neighbours. False,
    /// with nothing done, as for `update_neighbors`: off the level in place, for a position that
    /// isn't loaded, or for a `source` that is no block state.
    #[access(WorldOnly)]
    fn level_neighbor_changed(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        source: BlockStateId,
    ) -> bool;
    /// Side: server.
    /// Replace the random of the server level `level` names with `random`,
    /// a stream `native_level_random` gave and the mod drew from. False where
    /// that gives `None`.
    #[access(WorldOnly)]
    fn level_set_random(&mut self, level: LevelRef, random: LegacyRandom) -> bool;
    /// Side: both.
    /// Apply bone meal to the block at `pos` as a dispenser does (`BoneMealItem.growCrop`, then
    /// `growWaterPlant`), with no player and no item: the mods' `bonemeal` hook first, level event
    /// 1505 when something grew. True when something grew; false for a refused write, a stale or
    /// parked handle, a client, and a block that did not grow.
    #[access(WorldOnly)]
    fn level_bone_meal(&mut self, level: LevelRef, pos: [i32; 3]) -> bool;
    /// Side: both.
    /// Set the biome of the 4x4x4 cell holding `pos` to `biome` (`minecraft:plains`), as
    /// `/fillbiome` does for one cell. False for a refused write, a stale or parked handle, a
    /// client, an unknown biome and an unchanged cell.
    #[access(WorldOnly)]
    fn level_set_biome(&mut self, level: LevelRef, pos: [i32; 3], biome: &str) -> bool;
}
