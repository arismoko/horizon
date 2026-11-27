// Types that lived in the API root before the domain split.
use crate::*;

/// A saved fluid name no registered fluid has, kept verbatim until its declaring mod returns.
/// `Wire` sends the name; decoding refuses a name the receiving game has registered.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: its field is private, so only a validated unregistered name makes one
pub struct UnresolvedFluid(pub(crate) Identifier);

/// A dimension (`minecraft:the_nether`), as the game's table numbers it: a `LevelRef`'s identity.
/// A key stays the same dimension until the game exits; `key.id(host)` borrows the name.
// no wire: decoding validates the u32 slot with DimensionKey::from_slot on the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its field is private, so only the game's table creates a key
pub struct DimensionKey(pub(crate) u32);

/// Why a game rule or category declaration was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum GameRuleError {
    /// The identifier is invalid.
    InvalidId,
    /// The category or rule is already declared.
    Duplicate,
    /// An integer rule's `min` is above its `max`, or its default is outside `min..=max`.
    InvalidRange,
}

/// A boolean game rule declared by a mod.
// no wire: decoding validates the u32 slot with BoolGameRuleKey::from_slot on the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: only the registration table creates keys
pub struct BoolGameRuleKey(pub(crate) u32);

/// An integer game rule declared by a mod with
/// [`Registrar::game_rule_int`](GameRulesRegistrar::game_rule_int).
// no wire: decoding validates the u32 slot with IntGameRuleKey::from_slot on the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: only the registration table creates keys
pub struct IntGameRuleKey(pub(crate) u32);

/// A load-owned fluid identity requested through [`Registrar::fluid_key`](FluidsRegistrar::fluid_key) or resolved through
/// [`DataLoadContext::fluid_key`].
// no wire: decoding validates the u32 slot with FluidKey::from_slot on the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: only the registration table creates keys
pub struct FluidKey(pub(crate) u32);

impl FluidKey {
    /// Whether this fluid is in `tag` (`FluidState.is(TagKey)`), over `tags`'s current
    /// authoritative snapshot. An unknown tag is `Some(false)`; unavailable tags or mappings
    /// return `None`.
    pub fn is_in_tag(self, tags: &dyn Tags, tag: TagKey<FluidKey>) -> Option<bool> {
        tags.fluid_in_tag(self, tag)
    }
    /// Whether this fluid and `other` are the same fluid (`Fluid.isSame`): a source and its
    /// flowing form are, so `minecraft:water` and `minecraft:flowing_water` are the same, water
    /// and lava not. False when either key has no world fluid or tables are unavailable.
    pub fn is_same(self, states: &BlockStates<'_>, other: FluidKey) -> bool {
        states.backend().fluid_is_same(self, other).unwrap_or(false)
    }
    /// The bucket item that picks this fluid up (`Fluid.getBucket`), `minecraft:water_bucket`
    /// for water and flowing water. `None` for `minecraft:empty` (whose bucket is air), for a
    /// tank-only fluid, or when tables are unavailable.
    pub fn bucket(self, states: &BlockStates<'_>) -> Option<ItemKey> {
        states.backend().fluid_bucket(self).ok().flatten()
    }
    /// The sound event a bucket plays as it picks this fluid up (`Fluid.getPickupSound`),
    /// `minecraft:item.bucket.fill_lava` for lava and flowing lava. `None` for
    /// `minecraft:empty`, for a tank-only fluid, or when tables are unavailable.
    pub fn pickup_sound(self, states: &BlockStates<'_>) -> Option<&'static str> {
        states.backend().fluid_pickup_sound(self).ok().flatten()
    }
    /// The sound event a bucket plays as it empties this fluid (`BucketItem.playEmptySound`,
    /// NeoForge's `FluidType.getSound(SoundActions.BUCKET_EMPTY)`),
    /// `minecraft:item.bucket.empty_lava` for lava and flowing lava and
    /// `minecraft:item.bucket.empty` for water and a mod's world fluid. `None` for
    /// `minecraft:empty`, for a tank-only fluid, or when tables are unavailable.
    pub fn empty_sound(self, states: &BlockStates<'_>) -> Option<&'static str> {
        states.backend().fluid_empty_sound(self).ok().flatten()
    }
}

/// Sampling parameters for `LevelRef::find_closest_biome`, in blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct BiomeSearch {
    /// Centre of the sampled spiral, in block coordinates.
    pub origin: [i32; 3],
    /// Maximum horizontal search radius in blocks.
    pub radius: i32,
    /// Distance between sampled columns in blocks.
    pub horizontal_step: i32,
    /// Distance between sampled heights in blocks.
    pub vertical_step: i32,
}

impl BiomeSearch {
    /// The radius must be nonnegative and both sampling steps positive.
    pub fn new(origin: [i32; 3], radius: i32, horizontal_step: i32, vertical_step: i32) -> Self {
        Self {
            origin,
            radius,
            horizontal_step,
            vertical_step,
        }
    }
}

/// The first matching sample in vanilla's closest-biome search order.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct BiomeSearchResult {
    /// Registry id of the matching biome.
    pub biome: String,
    /// Block coordinates of the matching sample.
    pub position: [i32; 3],
}

impl BiomeSearchResult {
    /// Pair a matching biome with its sampled position.
    pub fn new(biome: String, position: [i32; 3]) -> Self {
        Self { biome, position }
    }
}

/// Build range and sky properties of a loaded level's dimension type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct LevelDimensionShape {
    /// Lowest buildable block Y.
    pub min_y: i32,
    /// Build height in blocks.
    pub height: i32,
    /// Height used by portals and chorus fruit.
    pub logical_height: i32,
    /// Whether the dimension has skylight.
    pub sky_light: bool,
    /// Whether the dimension has a ceiling.
    pub has_ceiling: bool,
}

impl LevelDimensionShape {
    /// Construct a dimension shape from its build and sky properties.
    pub fn new(
        min_y: i32,
        height: i32,
        logical_height: i32,
        sky_light: bool,
        has_ceiling: bool,
    ) -> Self {
        Self {
            min_y,
            height,
            logical_height,
            sky_light,
            has_ceiling,
        }
    }
}

/// A fluid state copied from the engine. For engine-built values, amount is zero exactly for
/// the empty fluid. A mod may change a placement event's copy; that mod-provided data is
/// forwarded to later Parts and is never consumed by the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct FluidState {
    /// Registered fluid identity.
    pub fluid: FluidKey,
    /// Fluid amount; zero only for empty, otherwise 1 through 8.
    pub amount: u8,
    /// Whether the fluid is falling.
    pub falling: bool,
    /// Whether the engine identifies this as a source.
    pub source: bool,
}

impl FluidState {
    /// Whether this is the empty fluid.
    pub fn is_empty(self) -> bool {
        self.amount == 0
    }
    /// Whether the engine identifies this state as a source.
    pub fn is_source(self) -> bool {
        self.source
    }
    /// Whether this state's fluid is in `tag` (`FluidState.is(TagKey)`), delegating through
    /// `self.fluid`. An unknown tag is `Some(false)`; unavailable tags or mappings return
    /// `None`.
    pub fn is_in_tag(self, tags: &dyn Tags, tag: TagKey<FluidKey>) -> Option<bool> {
        self.fluid.is_in_tag(tags, tag)
    }
    /// Match carried vanilla properties locally. Unknown names or noncanonical values
    /// do not match. Validate against `host.fluid_states()` separately for indexed errors.
    pub fn matches(self, constraints: &[StateConstraint]) -> bool {
        constraints.iter().all(|c| match c.name.as_str() {
            "falling" if !self.is_empty() => c.value == if self.falling { "true" } else { "false" },
            "level" if !self.is_empty() && !self.source => {
                (1..=8).contains(&self.amount) && c.value.as_bytes() == [b'0' + self.amount]
            }
            _ => false,
        })
    }
}

/// The empty frame a fire is about to fill with a nether portal, for
/// `portal_spawn` (`BlockEvent.PortalSpawnEvent.getPortalSize`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct PortalShape {
    /// The portal runs along X (else along Z).
    pub x_axis: bool,
    /// Its bottom-left inside block.
    pub bottom_left: [i32; 3],
    /// The portal frame's inner width in blocks.
    pub width: i32,
    /// The portal frame's inner height in blocks.
    pub height: i32,
}

// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// A server explosion about to happen, for `explosion_start`
/// (`ServerLevel.explode`'s `ServerExplosion`).
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct ExplosionStart<'a> {
    /// The level it happens in (`minecraft:overworld`).
    pub dimension: DimensionKey,
    /// The explosion's center in blocks.
    pub center: [f64; 3],
    /// The explosion's radius in blocks.
    pub radius: f32,
    /// It sets fires (`createFire`).
    pub fire: bool,
    /// What the explosion does to blocks.
    pub interaction: BlockInteraction,
    /// The damage type of its damage source (`minecraft:explosion`,
    /// `minecraft:player_explosion`, `minecraft:bad_respawn_point`).
    pub damage_type: &'a str,
    /// The entity that explodes (`getDirectSourceEntity`): a creeper, a wind
    /// charge, the bearer of Wind Charged, when Sunrise tracks it.
    pub source: Option<EntityRef>,
    /// `getIndirectSourceEntity`: a living source itself, a primed TNT's
    /// owner, or a projectile's living owner (a ghast for its fireball, a
    /// wither for its skull, the thrower of a wind charge).
    pub owner: Option<EntityRef>,
}

/// One server world open or client level; it rejects a previous world's handles.
#[derive(sunrise_mod_api_macros::Wire, Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct WorldEpoch(pub(crate) u64);

impl WorldEpoch {
    /// Create an epoch for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(n: u64) -> Self {
        Self(n)
    }
    /// Read the epoch number.
    pub fn get(self) -> u64 {
        self.0
    }
}

/// A block in a dimension, vanilla's `GlobalPos`: the dimension's key
/// (`minecraft:the_nether`) and the block. The dimension is data, so it need
/// not be open; `host.level(&pos.dimension().to_string())` gives its level when it is.
#[derive(sunrise_mod_api_macros::Wire, Clone, Debug, PartialEq, Eq)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct GlobalPos {
    pub(crate) dimension: Identifier,
    pub(crate) pos: BlockPos,
}

impl GlobalPos {
    /// A block in the dimension with key `dimension`.
    pub fn new(dimension: Identifier, pos: BlockPos) -> Self {
        Self { dimension, pos }
    }
    /// The dimension's key (`minecraft:overworld`).
    pub fn dimension(&self) -> &Identifier {
        &self.dimension
    }
    /// The block.
    pub fn pos(&self) -> BlockPos {
        self.pos
    }
}

impl std::hash::Hash for GlobalPos {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(
            &(self.dimension.namespace(), self.dimension.path(), self.pos),
            state,
        );
    }
}

/// A player's respawn point (`ServerPlayer.RespawnConfig`): the block they come back at, the
/// way they face there and whether it is forced (`/spawnpoint`) rather than a bed or anchor that
/// must still be there.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: a mod builds it with a literal; the game reads and writes the same three values
#[derive(sunrise_mod_api_macros::Wire)]
pub struct PlayerRespawnConfig {
    /// The dimension and block.
    pub position: GlobalPos,
    /// The yaw in degrees (`Mth.wrapDegrees`, -180 to 180 when read).
    pub yaw: f32,
    /// Whether the point holds without its block (`/spawnpoint`).
    pub forced: bool,
}

/// A level of the open world, by its dimension (`minecraft:the_nether`).
/// `Host::level` makes one, and `entity.level(host)` gives an entity's.
#[derive(sunrise_mod_api_macros::Wire, Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct LevelRef {
    pub(crate) epoch: WorldEpoch,
    pub(crate) dimension: DimensionKey,
    /// Which open of a runtime level this names: 0 for a level every world has, the number the
    /// host gave one open of a level a mod created at run time.
    pub(crate) generation: u32,
}

impl LevelRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(epoch: WorldEpoch, dimension: DimensionKey) -> Self {
        Self {
            epoch,
            dimension,
            generation: 0,
        }
    }
    /// Create a handle for one open of a runtime level.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn with_generation(epoch: WorldEpoch, dimension: DimensionKey, generation: u32) -> Self {
        Self {
            epoch,
            dimension,
            generation,
        }
    }
    /// The open of a runtime level the handle names; 0 for a level every world has.
    // hidden: a game-side helper; a handle of an earlier open reads as stale
    #[doc(hidden)]
    pub fn generation(self) -> u32 {
        self.generation
    }
    /// The level's dimension. `level.dimension().id(host)` borrows its name.
    pub fn dimension(self) -> DimensionKey {
        self.dimension
    }
    /// The name of the level's dimension, for the game's own tables.
    // hidden: a game-side helper; mods ask the host
    #[doc(hidden)]
    pub fn dimension_id(self) -> &'static str {
        crate::content::registry_keys::dimension_id(self.dimension)
            .expect("the game's table made the key")
    }
    /// Side: both.
    /// Property `key` of the block state at `pos` (`StateHolder.getValue`):
    /// `east` for `facing`. None for a block without it, or as for
    /// `block_state`. API 381.
    /// The value borrows `host`, not `key`.
    pub fn block_property<'h>(
        self,
        host: &'h dyn Host,
        pos: [i32; 3],
        key: &str,
    ) -> Option<&'h str> {
        crate::backend::backend_of(host)?.level_block_property(self, pos, key)
    }
    /// Borrow the named server level's live random stream (`Level.getRandom`) for `draw`
    /// only: `level.with_random(host, |rng| rng.next_int_bound(bound))`. The closure cannot
    /// keep or reseed the stream. Draws advance that level's live stream directly in
    /// vanilla's order, even when the level is parked; this never switches or loads levels.
    ///
    /// Returns `None` without calling `draw` if the host lacks server authority, the
    /// handle is stale, the level is not resident, or world access is unavailable.
    ///
    /// Matching vanilla `Level.getRandom()`, draws are not rolled back if `draw` panics
    /// and unwinds, or if an enclosing transaction aborts.
    pub fn with_random<T>(
        self,
        host: &mut dyn ServerHost,
        draw: impl FnOnce(&mut dyn RandomSource) -> T,
    ) -> Option<T> {
        let backend = crate::backend::backend_mut_of(host)?;
        crate::world::random::with_random_draw(draw, |draw| backend.with_level_random(self, draw))
    }
    /// The loaded entity of this server level with `uuid`, as vanilla's
    /// `ServerLevel.getEntity(UUID)` reads the level's entity lookup; NeoForge mods call the
    /// same vanilla method, which NeoForge does not extend. It reads the named open level
    /// whether it is the one in place for this hook or parked while the server ticks
    /// another, through the level's UUID index: it never switches, loads or scans a level.
    /// To search every level, as vanilla's `ServerLevel.getEntityInAnyDimension(UUID)`
    /// does, ask the hook's own level first, then each other level of
    /// [`Levels::visit_levels`]:
    ///
    /// ```
    /// use core::ops::ControlFlow;
    /// use mod_api::prelude::*;
    /// use mod_api::{EntityRef, EntityUuid, LevelRef, ServerHost};
    /// fn find_anywhere(host: &dyn ServerHost, own: LevelRef, uuid: EntityUuid) -> Option<EntityRef> {
    ///     own.entity_by_uuid(host, uuid).or_else(|| {
    ///         let mut found = None;
    ///         let _ = host.levels().visit_levels(&mut |level| {
    ///             found = (level != own).then(|| level.entity_by_uuid(host, uuid)).flatten();
    ///             match found {
    ///                 Some(_) => ControlFlow::Break(()),
    ///                 None => ControlFlow::Continue(()),
    ///             }
    ///         });
    ///         found
    ///     })
    /// }
    /// ```
    ///
    /// Returns `None` for an entity in a hidden section of the level in place or whose chunk is
    /// not loaded, one resident in another
    /// level, a dragon part, an entity held out of lookup, taken from its resident storage,
    /// unavailable under a scoped item loan or queued for removal, a vehicle being
    /// destroyed, a stale handle, a level that is not open, and outside server world access.
    /// The lookup leaves entity and level state alone.
    ///
    /// The handle it returns is an ordinary [`EntityRef`]: the calls that read an entity of a
    /// parked level (`kind`, `uuid`, `level`) answer for it, and the others refuse it while
    /// its level is parked, as they refuse any entity outside the level in place.
    pub fn entity_by_uuid(self, host: &dyn ServerHost, uuid: EntityUuid) -> Option<EntityRef> {
        crate::backend::backend_of(host)?.level_entity_by_uuid(self, uuid)
    }
    /// The open of the world the handle belongs to.
    pub fn epoch(self) -> WorldEpoch {
        self.epoch
    }
}

/// A live watered area in one server level. Created by [`LevelRef::add_water_ticket`].
/// Any holder with this handle can remove it via [`WaterTicket::remove`]; level unload and,
/// when attributed to an enabled mod at creation, that mod's disable also invalidate it.
/// Stays in-process (never serialized or saved): re-add in `level_load` or a block entity's
/// load listener.
// no wire: a live ticket identity can only be issued by its level
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct WaterTicket {
    pub(crate) level: LevelRef,
    pub(crate) id: sunrise_mod_runtime::water_tickets::WaterTicketId,
}

impl WaterTicket {
    /// The level this transient ticket belongs to.
    pub fn level(self) -> LevelRef {
        self.level
    }
}

/// A raid of a level (`Raid`), by the id `Raids` keeps it under, which the
/// raids file saves. `level.raid_at(host, pos)`, `level.visit_raids(host, visit)`,
/// `level.raid(host, id)` and `mob.raid(host)` supply handles. Each call resolves the handle
/// again. Collection visits distinguish a gone raid or a stale level ([`RaidReadError::Stale`])
/// from a phase or side that cannot read it ([`RaidReadError::Unavailable`]). Other calls
/// return `None` or `false` on refusal. Reads and writes reach parked server levels without
/// moving them into place.
#[derive(sunrise_mod_api_macros::Wire, Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct RaidRef {
    pub(crate) level: LevelRef,
    pub(crate) id: i32,
}

impl RaidRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(level: LevelRef, id: i32) -> Self {
        Self { level, id }
    }
    /// The level the raid belongs to.
    pub fn level(self) -> LevelRef {
        self.level
    }
    /// The raid's id in its level (the `raid` argument of the raid hooks).
    pub fn id(self) -> i32 {
        self.id
    }
}

/// Where a raid stands (`Raid.RaidStatus`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum RaidStatus {
    /// Waiting or fighting (`ONGOING`).
    Ongoing,
    /// Every wave was beaten (`VICTORY`).
    Victory,
    /// No villager is left or the time ran out (`LOSS`).
    Loss,
    /// Ended by `stop`, the `/raid stop` command or the `raids` rule (`STOPPED`).
    Stopped,
}

/// The world clock a level follows (`WorldClock`, a `minecraft:world_clock` registry entry): the
/// one its dimension type names as `default_clock`, shared by every level of that dimension type
/// and saved with the world (`ServerClockManager`). `level.clock(host)` makes one; its writes are
/// `ServerClockManager`'s `setTotalTicks`, `setRate` and `setPaused`. A paused clock is a fixed
/// time. A handle resolves again on each call, through its level: `false` once the level is gone,
/// for a handle of an earlier open, and on a client.
#[derive(sunrise_mod_api_macros::Wire, Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct WorldClockRef {
    pub(crate) level: LevelRef,
}

impl WorldClockRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(level: LevelRef) -> Self {
        Self { level }
    }
    /// The level whose default clock this is.
    pub fn level(self) -> LevelRef {
        self.level
    }
}

/// A chunk position in an open level. A handle may be kept after unloading;
/// reads resolve it again, returning `false` or `None` when it is no longer loaded.
#[derive(sunrise_mod_api_macros::Wire, Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ChunkRef {
    pub(crate) level: LevelRef,
    pub(crate) pos: [i32; 2],
}

impl ChunkRef {
    /// Create a handle for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(level: LevelRef, pos: [i32; 2]) -> Self {
        Self { level, pos }
    }
    /// The chunk coordinates, `[x, z]`.
    pub fn pos(self) -> [i32; 2] {
        self.pos
    }
    /// The level containing the chunk.
    pub fn level(self) -> LevelRef {
        self.level
    }
}

/// The status of a loaded chunk that Sunrise can expose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ChunkStatus {
    /// A full chunk in a level's live world.
    Full,
}

/// Why `level.load_chunk` did not bring its chunk to FULL: the error of the `ChunkResult`
/// vanilla's chunk futures carry (`ServerChunkCache.getChunkFuture`, `ChunkResult.isSuccess`),
/// and the failure Sunrise adds where vanilla would crash. `Ok(())` is the chunk loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ChunkLoadError {
    /// Vanilla's `ChunkResult` error (`ChunkHolder.UNLOADED_CHUNK`): the chunk could not be
    /// held, or the level went away during the wait. A stale handle, a client, a level the
    /// server has not opened, a chunk past `ChunkPos.isValid` and a level whose generation is
    /// paused or stopped answer it too.
    Unloaded,
    /// Generation panicked for this chunk or for one its generation needs around it: a mod's
    /// chunk generator, contained by the engine where vanilla would crash. The level stays open
    /// and is not generated there again until it opens again; asking again answers `Failed` at
    /// once.
    Failed,
}

impl crate::NoBackend for ChunkLoadError {
    /// No game: as for a level the server has not opened.
    fn no_backend() -> Self {
        Self::Unloaded
    }
}

/// A chunk's ticket level (`ChunkLevel`), as the server holds it: how much of
/// the level the chunk is kept alive for. Lower is more active. Sunrise keeps
/// no level for a chunk partway through generating, so past `FULL` a chunk is
/// `Unloaded`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ChunkTicketLevel {
    /// A level `ChunkLevel` knows, `FULL_CHUNK_LEVEL` (33) or lower: 31 and
    /// lower is entity ticking (`ENTITY_TICKING_LEVEL`), 32 is block ticking
    /// (`BLOCK_TICKING_LEVEL`) and 33 is loaded without ticking.
    Loaded(u8),
    /// No ticket keeps the chunk loaded.
    Unloaded,
}

impl ChunkTicketLevel {
    /// The `ChunkLevel` number, or `None` for an unloaded chunk.
    pub fn level(self) -> Option<u8> {
        match self {
            Self::Loaded(level) => Some(level),
            Self::Unloaded => None,
        }
    }
}

/// A kind of light (`LightLayer`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(sunrise_mod_api_macros::Wire)]
pub enum LightLayer {
    /// Light from emitting blocks (`BLOCK`).
    Block,
    /// Light from the sky (`SKY`).
    Sky,
}

/// A built-in Minecraft feature flag (`net.minecraft.world.flag.FeatureFlags.REGISTRY`).
///
/// Used by general entity declarations (`EntityType.Builder.requiredFeatures`) and checked
/// against a world's [`FeatureFlags`] (`EntityType.isEnabled`) at spawn and load.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum FeatureFlag {
    /// `minecraft:vanilla` (`FeatureFlags.VANILLA`).
    Vanilla,
    /// `minecraft:trade_rebalance` (`FeatureFlags.TRADE_REBALANCE`).
    TradeRebalance,
    /// `minecraft:redstone_experiments` (`FeatureFlags.REDSTONE_EXPERIMENTS`).
    RedstoneExperiments,
    /// `minecraft:minecart_improvements` (`FeatureFlags.MINECART_IMPROVEMENTS`).
    MinecartImprovements,
}

impl FeatureFlag {
    /// Every built-in flag in vanilla `FeatureFlags.REGISTRY` order.
    pub const ALL: [Self; 4] = [
        Self::Vanilla,
        Self::TradeRebalance,
        Self::RedstoneExperiments,
        Self::MinecartImprovements,
    ];

    /// The flag's canonical namespaced id (`minecraft:vanilla`, `minecraft:trade_rebalance`, ...).
    pub const fn id(self) -> &'static str {
        match self {
            Self::Vanilla => "minecraft:vanilla",
            Self::TradeRebalance => "minecraft:trade_rebalance",
            Self::RedstoneExperiments => "minecraft:redstone_experiments",
            Self::MinecartImprovements => "minecraft:minecart_improvements",
        }
    }

    /// Resolve a feature flag id as `FeatureFlags.REGISTRY` does. Like `Identifier.CODEC`,
    /// this uses the `minecraft` namespace if the namespace is omitted or empty.
    pub fn from_id(id: &str) -> Option<Self> {
        let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
        if !matches!(namespace, "" | "minecraft") {
            return None;
        }
        match path {
            "vanilla" => Some(Self::Vanilla),
            "trade_rebalance" => Some(Self::TradeRebalance),
            "redstone_experiments" => Some(Self::RedstoneExperiments),
            "minecart_improvements" => Some(Self::MinecartImprovements),
            _ => None,
        }
    }
}

/// The effective feature flags a world enables (`FeatureFlagSet`, `LevelReader.enabledFeatures`),
/// each as its id (`minecraft:vanilla`, `minecraft:redstone_experiments`), shared with the world.
/// Getting and reading it allocates nothing.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct FeatureFlags {
    flags: Arc<[String]>,
}

impl FeatureFlags {
    /// Wrap the world's effective flag ids as-is. An empty set contains no flags.
    pub fn new(flags: Arc<[String]>) -> Self {
        Self { flags }
    }
    /// `FeatureFlagSet.contains`: whether the flag `flag` (`minecraft:redstone_experiments`, or
    /// `redstone_experiments`) is enabled.
    pub fn contains(&self, flag: &str) -> bool {
        let bare = !flag.contains(':');
        self.iter()
            .any(|id| id == flag || (bare && id.strip_prefix("minecraft:") == Some(flag)))
    }
    /// `FeatureFlagSet.contains(FeatureFlag)`: whether the typed flag `flag` is enabled.
    pub fn contains_flag(&self, flag: FeatureFlag) -> bool {
        self.contains(flag.id())
    }
    /// `FeatureFlagSet.isSubsetOf` (`FeatureElement.isEnabled`): whether every flag in `required`
    /// is enabled in this set.
    pub fn contains_all(&self, required: &[FeatureFlag]) -> bool {
        required.iter().all(|&flag| self.contains_flag(flag))
    }
    /// The enabled flag ids.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.flags.iter().map(String::as_str)
    }
}

/// Copy the world's effective flag ids into an owned list.
impl From<&FeatureFlags> for Vec<String> {
    fn from(flags: &FeatureFlags) -> Self {
        flags.flags.to_vec()
    }
}

impl From<Vec<String>> for FeatureFlags {
    fn from(ids: Vec<String>) -> Self {
        Self::new(ids.into())
    }
}

/// What an explosion a mod sets off does to blocks
/// (`Level.ExplosionInteraction`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ExplosionInteraction {
    /// Blocks stay; entities are hurt and pushed (`NONE`).
    None,
    /// Blocks break and drop by the `block_explosion_drop_decay` rule (`BLOCK`).
    #[default]
    Block,
    /// Blocks break and drop by the `mob_explosion_drop_decay` rule when the
    /// source may grief (its `entity_mob_griefing` answer; with no source, the
    /// `mob_griefing` rule), and stay when it may not (`MOB`).
    Mob,
    /// Blocks break and drop by the `tnt_explosion_drop_decay` rule (`TNT`).
    Tnt,
    /// No block breaks; buttons, levers, doors, trapdoors, fence gates, bells
    /// and candles react as to a wind charge's burst (`TRIGGER`).
    Trigger,
}

/// The resistance a blast meets in one cell (`ExplosionDamageCalculator.getBlockExplosionResistance`),
/// as [`LevelRef::explosion_resistance`](crate::LevelRef::explosion_resistance) reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ExplosionResistance {
    /// No answer: the chunk isn't loaded, the level or the source is stale, or the cell holds a
    /// state the game doesn't know.
    Unavailable,
    /// No resistance at all (air with no fluid, or a cell an immune-blocks calculator does not
    /// name): a ray loses only its fixed step there. Not the same as a resistance of 0.
    NoResistance,
    /// This resistance: a ray's step through the cell takes `(r + 0.3) * 0.3` of its power.
    Resists(f32),
}

/// The cells a batched explosion resistance read covers
/// ([`LevelRef::explosion_resistances`](crate::LevelRef::explosion_resistances)), in the order
/// the answers come.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ResistanceCells {
    /// Every cell from `min` to `max`, both included: x fastest, then z, then y
    /// (index `((y - min.y) * dz + (z - min.z)) * dx + (x - min.x)`).
    Box {
        /// The low corner.
        min: [i32; 3],
        /// The high corner, included.
        max: [i32; 3],
    },
    /// Each column `(x, z)` in the order given, from `min_y` up to `max_y`, both included.
    Columns {
        /// The columns, a repeated one read again.
        columns: Vec<(i32, i32)>,
        /// The lowest cell of each column.
        min_y: i32,
        /// The highest cell of each column, included.
        max_y: i32,
    },
}

/// Why a batched explosion resistance read was refused, with nothing read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ResistanceCellsError {
    /// A box whose `max` is below its `min` on some axis, or columns whose `max_y` is below `min_y`.
    Inverted,
    /// The area's cell count exceeds what this machine can allocate in one read.
    Overflow,
    /// A host without a game.
    NoGame,
}

impl crate::NoBackend for ResistanceCellsError {
    fn no_backend() -> Self {
        Self::NoGame
    }
}

/// Why [`LevelRef::explosion_destroy_blocks`](crate::LevelRef::explosion_destroy_blocks)
/// destroyed nothing, with nothing changed and no hook told.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ExplosionDestroyError {
    /// A stale level, or one not in place; a stale source or one in another level; a client.
    Unavailable,
    /// A centre or radius that isn't finite.
    InvalidBlast,
    /// A host without a game.
    NoGame,
}

impl crate::NoBackend for ExplosionDestroyError {
    fn no_backend() -> Self {
        Self::NoGame
    }
}

/// What made a `tags_updated` notice fire (`TagsUpdatedEvent`'s two subclasses).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum TagsUpdateCause {
    /// `TagsUpdatedEvent.ServerDataLoad`: a server built its data tables.
    ServerDataLoad {
        /// False on a world's first open, true after `/reload`.
        reload: bool,
    },
    /// `TagsUpdatedEvent.ClientPacketReceived`: a client applied the tags its server sent.
    ClientPacketReceived {
        /// The server is the single player window's own.
        integrated_server: bool,
    },
}

impl TagsUpdateCause {
    /// `TagsUpdatedEvent.shouldUpdateStaticData`: whether to update static data, which single
    /// player's two threads share, so that there only the server side updates it.
    pub fn should_update_static_data(self) -> bool {
        !matches!(
            self,
            Self::ClientPacketReceived {
                integrated_server: true
            }
        )
    }
}

/// What made a `data_maps_updated` notice fire (`DataMapsUpdatedEvent.UpdateCause`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum DataMapsUpdateCause {
    /// `SERVER_RELOAD`: the server merged its data maps, on a world's first open or after
    /// `/reload`.
    ServerReload,
    /// `CLIENT_SYNC`: a client applied the data maps its server sent.
    ClientSync,
}

/// The category of an entity's type (`MobCategory`, `EntityType.getCategory`):
/// what natural spawning counts it as. Every kind that isn't a mob is `Misc`,
/// except the ender dragon, a `Monster`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum MobCategory {
    /// `MONSTER`.
    Monster,
    /// `CREATURE`.
    Creature,
    /// `AMBIENT`.
    Ambient,
    /// `AXOLOTLS`.
    Axolotls,
    /// `UNDERGROUND_WATER_CREATURE`.
    UndergroundWaterCreature,
    /// `WATER_CREATURE`.
    WaterCreature,
    /// `WATER_AMBIENT`.
    WaterAmbient,
    /// `MISC`.
    Misc,
}

/// How many mobs of each spawning category a level's natural spawning allows
/// (`MobCategory.getMaxInstancesPerChunk`), read and set through
/// `LevelRef::spawn_limits` and `set_spawn_limits`. Each limit counts twice, as
/// vanilla's own does: the level spawns a category only while its count of
/// that category's mobs stays under `limit * spawnable chunks / 289` (one
/// player's 17 by 17 chunks hold the vanilla value), and only in a chunk with
/// a player near it who has fewer than `limit` of them around. A limit of 0
/// stops the category's natural spawning. `MobCategory::Misc` never spawns
/// naturally, so it has no limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: one field per vanilla spawning category; mods build it from `VANILLA`
#[derive(sunrise_mod_api_macros::Wire)]
pub struct SpawnLimits {
    /// `MONSTER`: 70 in vanilla.
    pub monster: u32,
    /// `CREATURE`: 10 in vanilla.
    pub creature: u32,
    /// `AMBIENT`: 15 in vanilla.
    pub ambient: u32,
    /// `AXOLOTLS`: 5 in vanilla.
    pub axolotls: u32,
    /// `UNDERGROUND_WATER_CREATURE`: 5 in vanilla.
    pub underground_water_creature: u32,
    /// `WATER_CREATURE`: 5 in vanilla.
    pub water_creature: u32,
    /// `WATER_AMBIENT`: 20 in vanilla.
    pub water_ambient: u32,
}

impl SpawnLimits {
    /// Vanilla's limits, which every level starts with.
    pub const VANILLA: Self = Self {
        monster: 70,
        creature: 10,
        ambient: 15,
        axolotls: 5,
        underground_water_creature: 5,
        water_creature: 5,
        water_ambient: 20,
    };
}

impl Default for SpawnLimits {
    fn default() -> Self {
        Self::VANILLA
    }
}

/// Which categories a level's natural spawning (`NaturalSpawner.spawnForChunk`)
/// may spawn at all, read and set through `LevelRef::spawn_flags` and
/// `set_spawn_flags`. A flag only takes away, for its level alone: monsters
/// still need the `spawn_monsters` game rule, and nothing spawns naturally with
/// `spawn_mobs` off. Spawners, patrols, sieges, phantoms, a new chunk's
/// creatures and other custom spawners don't read the flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: one switch for monsters, one for the rest; mods build it with a literal
#[derive(sunrise_mod_api_macros::Wire)]
pub struct SpawnFlags {
    /// Whether `MobCategory::Monster` spawns: off acts as the `spawn_monsters`
    /// rule off does (`getFilteredSpawningCategories`' `spawnEnemies`).
    pub monsters: bool,
    /// Whether every other spawning category spawns: creatures, ambient and
    /// water mobs, axolotls. Sunrise's own; vanilla has no such switch.
    pub friendlies: bool,
}

impl Default for SpawnFlags {
    /// Both on, as every level starts.
    fn default() -> Self {
        Self {
            monsters: true,
            friendlies: true,
        }
    }
}

/// Why a mob spawns (`EntitySpawnReason`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum SpawnReason {
    /// `NATURAL`: the level's own spawning each tick.
    Natural,
    /// `CHUNK_GENERATION`: a new chunk's creature packs.
    ChunkGeneration,
    /// `SPAWNER`: a mob spawner block.
    Spawner,
    /// `TRIAL_SPAWNER`: a trial spawner.
    TrialSpawner,
    /// `REINFORCEMENT`: a zombie calling for help.
    Reinforcement,
    /// `PATROL`: a pillager patrol.
    Patrol,
    /// `STRUCTURE`: a structure's own mobs.
    Structure,
    /// `EVENT`: a world event, such as a village siege.
    Event,
    /// `COMMAND`: `/summon`.
    Command,
    /// `SPAWN_ITEM_USE`: a spawn egg.
    SpawnItemUse,
    /// `MOB_SUMMONED`: a mob that summons another (an evoker's vexes).
    MobSummoned,
    /// `CONVERSION`: a mob that became another.
    Conversion,
    /// `BREEDING`: a baby.
    Breeding,
    /// `TRIGGERED`: a block or item set it off (a sculk shrieker's warden).
    Triggered,
    /// `JOCKEY`: a rider spawned on a mount.
    Jockey,
    /// `DISPENSER`: a dispenser fired a spawn egg.
    Dispenser,
    /// `BUCKET`: a bucketed mob released.
    Bucket,
    /// `LOAD`: loaded from saved data.
    Load,
    /// `DIMENSION_TRAVEL`: a mob that crossed a portal.
    DimensionTravel,
}

/// What [`LevelRef::spawn_entity`] spawns besides its kind and position: the
/// velocity, rotation and owner it starts with, and the body its family takes.
/// Built with [`EntitySpawn::new`], then its fields set.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct EntitySpawn {
    /// The velocity it starts with, in blocks per tick (`Entity.setDeltaMovement`).
    pub velocity: [f64; 3],
    /// Its yaw and pitch in degrees. None: a thrown projectile faces along its
    /// velocity (`Projectile.shoot`), a mob takes `EntityType.spawn`'s yaw, and
    /// a general row faces south, level.
    pub rotation: Option<[f32; 2]>,
    /// The entity that fired it (`Projectile.setOwner`, `TraceableEntity`): for
    /// damage credit, and so its shooter is not hit by its own shot. None for
    /// a mob.
    pub owner: Option<EntityRef>,
    /// The family's own part of the spawn, which must match the kind's family.
    pub body: SpawnBody,
}

impl EntitySpawn {
    /// A spawn of `body` at rest, unturned and with no owner.
    pub fn new(body: SpawnBody) -> Self {
        Self {
            velocity: [0.0; 3],
            rotation: None,
            owner: None,
            body,
        }
    }
}

/// The family of entity [`LevelRef::spawn_entity`] spawns, with what that
/// family's spawn takes.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum SpawnBody {
    /// A thrown item projectile (vanilla's snowball, egg, ender pearl,
    /// experience bottle, wind charge, splash or lingering potion, or a content
    /// thrown type), carrying and drawn as `item` (`ThrowableItemProjectile.setItem`).
    Thrown {
        /// The item it carries; one of it is kept.
        item: ItemStack,
    },
    /// A mob (vanilla's or a content living type), finalized with `reason`
    /// (`Mob.finalizeSpawn`).
    Living {
        /// Why it spawns, as `finalize_spawn` and `finalizeSpawn` read it.
        reason: SpawnReason,
    },
    /// A row of a code-declared general type (`declare_general_entity`).
    General,
}

/// The local difficulty at a spawn (`DifficultyInstance`): what `finalize_spawn` reads and
/// may change before the mob is finalized.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct Difficulty {
    /// `Difficulty.getId`: 0 peaceful, 1 easy, 2 normal, 3 hard. A larger value is taken as 3.
    pub base: u8,
    /// `DifficultyInstance.getEffectiveDifficulty`: the base scaled by world age, the chunk's
    /// inhabited time and the moon.
    pub effective: f32,
}

impl Difficulty {
    /// A difficulty of `base` and `effective`.
    pub fn new(base: u8, effective: f32) -> Self {
        Self { base, effective }
    }
    /// `DifficultyInstance.getSpecialMultiplier`: 0 below an effective difficulty of 2, 1
    /// above 4, and rising in a straight line between them. Mobs read it for their chance to
    /// pick up loot, a zombie's reinforcements and a skeleton's enchanted bow.
    pub fn special_multiplier(&self) -> f32 {
        if self.effective < 2.0 {
            0.0
        } else if self.effective > 4.0 {
            1.0
        } else {
            (self.effective - 2.0) / 2.0
        }
    }
}

/// What spawner triggered a spawn (`FinalizeSpawnEvent.getSpawner` and
/// `MobSpawnEvent.PositionCheck.getSpawner`).
///
/// Each hook supplies only the variants its NeoForge counterpart can produce:
/// `finalize_spawn` (`Either<BlockEntity, Entity>`) supplies `Block`, `Trial` and `Minecart`,
/// while `mob_spawn_position_check` (`BaseSpawner`) supplies `Block` and `Minecart`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum Spawner {
    /// A monster spawner block (`SpawnerBlockEntity`).
    Block {
        /// Where the block is.
        pos: BlockPos,
    },
    /// A trial spawner block (`TrialSpawnerBlockEntity`).
    Trial {
        /// Where the block is.
        pos: BlockPos,
    },
    /// A spawner minecart entity (`MinecartSpawner`).
    Minecart {
        /// The spawner minecart entity.
        entity: crate::EntityRef,
    },
}

/// What a mod answers a spawn check (`MobSpawnEvent.PositionCheck.Result` and
/// `MobSpawnEvent.SpawnPlacementCheck.Result`, one type for both).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum SpawnCheck {
    /// `SUCCEED`: the check passes.
    Succeed,
    /// `DEFAULT`: keep the check's own answer, including the effective predicate result
    /// for a placement check.
    #[default]
    Default,
    /// `FAIL`: the check fails.
    Fail,
}

/// Where an entity type may stand (`SpawnPlacementType`), checked by
/// `SpawnPlacements.isSpawnPositionOk` separately from its spawn predicate.
// no wire: only used to register native spawn predicates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpawnPlacementType {
    /// `ON_GROUND`: a block that is valid to spawn on below, free room at the feet and the head.
    OnGround,
    /// `IN_WATER`: a water block, with no redstone conductor above.
    InWater,
    /// `IN_LAVA`: a lava block.
    InLava,
    /// `NO_RESTRICTIONS`: anywhere.
    NoRestrictions,
}

/// A read-only view of the level, lent for one spawn predicate call. None of its
/// reads load chunks or write to the level.
pub trait SpawnPlacementLevel {
    /// The namespaced name of the block at `pos`, such as `minecraft:grass_block`.
    fn block_name(&self, pos: BlockPos) -> &str;
    /// Whether the block at `pos` is in `tag`. Use a namespaced tag name without `#`,
    /// such as `minecraft:animals_spawnable_on`.
    fn block_in_tag(&self, pos: BlockPos, tag: &str) -> bool;
    /// Whether the biome at `pos` is in `tag`. Use a namespaced tag name without `#`.
    fn biome_in_tag(&self, pos: BlockPos, tag: &str) -> bool;
    /// The sky light at `pos` (`LightLayer.SKY`).
    fn sky_light(&self, pos: BlockPos) -> i32;
    /// The block light at `pos` (`LightLayer.BLOCK`).
    fn block_light(&self, pos: BlockPos) -> i32;
    /// `Animal.isBrightEnoughToSpawn` at `pos`, without any spawn-reason bypass or random draws.
    fn is_bright_enough_to_spawn(&self, pos: BlockPos) -> bool;
}

/// The level and live random stream lent for one spawn predicate call.
/// To preserve the vanilla trial-spawner light bypass, test [`SpawnReason::TrialSpawner`]
/// before calling either light helper. The helpers themselves do not check the reason.
pub trait SpawnPlacementContext {
    /// The read-only level for this check. Reads never load chunks or write to the level.
    fn level(&self) -> &dyn SpawnPlacementLevel;
    /// The caller's live random stream, not a fork. Draws advance it in call order and
    /// are kept even if this predicate or a later check rejects the spawn. A panic
    /// restores the stream to the start of that predicate's call.
    fn random(&mut self) -> &mut dyn RandomSource;
    /// `Monster.isDarkEnoughToSpawn` at `pos`, without any spawn-reason bypass.
    /// It draws from the same stream as [`Self::random`].
    fn is_dark_enough_to_spawn(&mut self, pos: BlockPos) -> bool;
}

/// How a native spawn predicate composes with a type's existing predicate.
/// For a type not previously registered, the first predicate is the original
/// whatever operation is specified.
// no wire: only used to register native spawn predicates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the vanilla registration contract has And, Or, and Replace, matched by the resolver.
pub enum SpawnPlacementOperation {
    /// Require this predicate as well as a passing original or `Or` predicate.
    And,
    /// Try this predicate if the original and earlier `Or` predicates failed.
    Or,
    /// Replace an existing entry's predicate, bypassing the original and all `And` and `Or`
    /// predicates. Only the last replacement is evaluated. A replacement can change either
    /// placement or heightmap; a field it omits retains its current value.
    Replace,
}

/// A native spawn predicate for an entity type, reason and block position. Register a
/// [`Send`] and [`Sync`] function or closure with the same signature, or implement this trait.
/// Use the lent level and random stream to decide the result; do not depend on call counts.
pub trait SpawnPlacementPredicate: Send + Sync + 'static {
    /// Whether this check passes. Use the lent random stream for any draws. The game invokes
    /// the predicate as the registering mod, skipping it when disabled. A panic restores
    /// the predicate's random draws, disables that predicate for the session, and reports
    /// the mod in the log and client error toast. A disabled or refused replacement fails
    /// the predicate check; it never retries the original.
    fn test(
        &self,
        kind: EntityTypeKey,
        ctx: &mut dyn SpawnPlacementContext,
        reason: SpawnReason,
        pos: BlockPos,
    ) -> bool;
}

impl<F> SpawnPlacementPredicate for F
where
    F: Fn(EntityTypeKey, &mut dyn SpawnPlacementContext, SpawnReason, BlockPos) -> bool
        + Send
        + Sync
        + 'static,
{
    fn test(
        &self,
        kind: EntityTypeKey,
        ctx: &mut dyn SpawnPlacementContext,
        reason: SpawnReason,
        pos: BlockPos,
    ) -> bool {
        self(kind, ctx, reason, pos)
    }
}

/// An immediate refusal from [`SpawnPlacementsRegistrar::register_spawn_placement`].
/// Resource-load refusals are reported separately, after the game resolves the registrations.
// no wire: only returned by the native spawn predicate registrar.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpawnPlacementError {
    /// The entity type key is not available in this registrar. The call also records
    /// a registration refusal, even if the returned error is ignored.
    InvalidEntityType,
}

/// What a mod answers a despawn check (`MobDespawnEvent.Result`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum Despawn {
    /// `ALLOW`: the mob despawns now.
    Allow,
    /// `DEFAULT`: vanilla's `Mob.checkDespawn` decides.
    #[default]
    Default,
    /// `DENY`: the mob does not despawn.
    Deny,
}

/// Block shape used by a level ray (`ClipContext.Block`).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
pub enum ClipBlock {
    /// Collider.
    Collider,
    /// Outline.
    Outline,
    /// `ClipContext.Block.VISUAL`: the visual shape (`BlockState.getVisualShape`), the one
    /// vanilla's third-person camera stops on. It is the collision shape except where vanilla
    /// overrides `getVisualShape`: glass, stained and tinted glass, copper grates
    /// (`TransparentBlock`), iron and copper bars and glass panes (`IronBarsBlock`) and powder
    /// snow have none, so the ray passes; soul sand and mud are a full block; snow layers and
    /// fences are their outline (`getShape`). A mod block on one of these templates follows
    /// its template. NeoForge adds nothing here.
    Visual,
}

/// Fluid mode for a level ray (`ClipContext.Fluid`).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
pub enum ClipFluid {
    /// `NONE`: ignore fluids.
    None,
    /// `SOURCE_ONLY`: stop at source fluids (`FluidState.isSource`), including waterlogged blocks.
    SourceOnly,
    /// `ANY`: stop at every non-empty fluid (`!FluidState.isEmpty`), including waterlogged blocks.
    Any,
    /// `WATER`: stop at fluids in the `minecraft:water` fluid tag (`FluidState.is(FluidTags.WATER)`), including tagged mod fluids and waterlogged blocks.
    Water,
}

/// Result of a block ray (`BlockGetter.clip`). `LevelRef::clip` returns it in
/// [`ClipOutcome::Hit`] with `miss` unset; `EntityRef::pick` also returns the vanilla miss result
/// (`BlockHitResult.miss`) with it set.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct ClipHit {
    /// Whether the ray hit nothing. On a miss, `location` is the ray's end point, `block` is
    /// its floored block coordinates, `face` is the nearest direction toward `from - to`, and
    /// `inside` is `false`.
    pub miss: bool,
    /// The hit block's position, or the floored end point on a miss.
    pub block: [i32; 3],
    /// The face the ray entered (or the nearest direction toward `from - to` on a miss).
    pub face: BlockFace,
    /// The exact hit point in blocks, or the ray's end point on a miss.
    pub location: [f64; 3],
    /// Whether the ray started inside the block (`false` on a miss).
    pub inside: bool,
    /// Whether the hit is the world border (`false` for `LevelRef::clip`, which follows
    /// `BlockGetter.clip`, not `CollisionGetter.clipIncludingBorder`).
    pub world_border: bool,
}

/// Whose collision context a level ray reads block shapes with (`CollisionContext.empty()` or
/// `CollisionContext.of(entity)`). It is the part of vanilla's `ClipContext` that varies for a
/// mod; the ray's endpoints, block shape and fluid mode are `LevelRef::clip`'s other arguments.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
pub enum CollisionContext {
    /// `CollisionContext.empty()`: no entity, so shapes that depend on who asks (scaffolding,
    /// powder snow, a mod `BlockPart::collision_shape`) answer their default.
    Empty,
    /// `CollisionContext.of(entity)`: shapes answer for this entity, as for its own movement.
    Entity(EntityRef),
}

/// What a level ray found (`BlockGetter.clip`), or where it had to stop.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ClipOutcome {
    /// The ray stopped on a block or fluid.
    Hit(ClipHit),
    /// The ray reached its end through loaded columns and out-of-height cells and hit nothing
    /// (`BlockHitResult.miss`).
    Miss,
    /// The ray entered a column that is not loaded before it found a hit or its end. Nothing
    /// is loaded and no answer is guessed past it.
    Unloaded {
        /// The first cell of the ray inside the unloaded column.
        at: [i32; 3],
    },
}

/// Why `LevelRef::clip` could not run the ray at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ClipError {
    /// The level handle (or the entity of the `CollisionContext`) is gone or belongs to an
    /// earlier open of the world, the level is not readable from this host, or the ray reached a
    /// shape the level cannot answer (no outline implementation for a dynamic block).
    StaleLevel,
    /// An endpoint has a NaN or infinite coordinate.
    NonFiniteRay,
}

impl crate::NoBackend for ClipError {
    fn no_backend() -> Self {
        Self::StaleLevel
    }
}

/// The world's respawn data (`LevelData.RespawnData`): where a player without a spawn point of
/// their own appears, and facing which way.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct RespawnData {
    /// The spawn block in the level `/setworldspawn` ran in (`RespawnData.globalPos`). Players
    /// go to the Overworld instead while the world has no such level
    /// (`MinecraftServer.findRespawnDimension`).
    pub position: GlobalPos,
    /// The yaw in degrees, -180 to 180 (`Mth.wrapDegrees`).
    pub yaw: f32,
    /// The pitch in degrees, -90 to 90.
    pub pitch: f32,
}

/// A map's saved data (`MapItemSavedData`) as it stands: where it is, how far it reaches and
/// its pixels. The markers (players, banners, frames) are not part of it. The dimension and
/// colors are borrowed for the duration of [`LevelRef::visit_map_data`]'s visitor.
// no wire: borrows the map for a scoped visit.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct MapState<'a> {
    /// The centre block `[x, z]`.
    pub center: [i32; 2],
    /// A map pixel is `1 << scale` blocks, 0 to 4.
    pub scale: u8,
    /// The dimension it shows, `minecraft:overworld`.
    pub dimension: &'a str,
    /// A locked map (a cartography table's glass pane) no longer reveals the world.
    pub locked: bool,
    /// Whether it shows its holders' markers.
    pub tracking: bool,
    /// Whether it shows them however far away they are.
    pub unlimited: bool,
    /// 128 by 128 map colour ids, row by row (`x + z * 128`).
    pub colors: &'a [u8],
}

/// One entity a level ray passes through, as tested by `ProjectileUtil.getEntityHitResult`.
/// Returned by [`LevelRef::ray_trace_entities`] and [`LevelRef::ray_trace_nearest_entity`].
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct EntityRayHit {
    /// The entity hit, a handle of the level the ray ran in.
    pub entity: EntityRef,
    /// Where the ray enters the entity's box, grown by the ray's margin, in blocks.
    pub location: [f64; 3],
    /// How far `location` is from the ray's start, in blocks.
    pub distance: f64,
}

/// Non-paused authoritative server ticks since open (not game or frame time).
#[derive(sunrise_mod_api_macros::Wire, Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct TickId(pub(crate) u64);

impl TickId {
    /// Create a tick ID for the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(n: u64) -> Self {
        Self(n)
    }
    /// Read the tick number.
    pub fn get(self) -> u64 {
        self.0
    }
}

/// The tick phases carry whether the world runs, including frozen steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ServerTickPhase {
    /// The server tick is about to begin.
    ServerTickStart {
        /// False for a frozen step, when the world does not advance.
        world_running: bool,
    },
    /// The server tick has finished.
    ServerTickEnd {
        /// False for a frozen step, when the world does not advance.
        world_running: bool,
    },
}

// no wire: lifecycle callbacks carry local server state, not saved or network data.
/// Context carried by server tick notifications (`server_tick`) and world-read queries.
/// `authority` is the server role. `LocalAuthoritative` and `RemoteClient` never
/// fire. `epoch` identifies the world open attempt, and `tick` counts non-paused
/// server ticks since the world opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct LifecycleContext {
    /// The server role that owns the world.
    pub authority: Authority,
    /// Identifies the world open attempt.
    pub epoch: WorldEpoch,
    /// Non-paused server ticks since the world opened.
    pub tick: TickId,
}

/// How a criterion's advancement progress changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ProgressType {
    /// Grant.
    Grant,
    /// Revoke.
    Revoke,
}

// no wire: a validated event name for native playback; the sound's own protocol carries its name.
/// A validated sound event identifier, independent of what resource packs are loaded.
// exhaustive: private field; constructed only by SoundEventKey::parse.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SoundEventKey(pub(crate) Arc<str>);

impl SoundEventKey {
    /// Parse a sound event name as vanilla's `Identifier.parse` does. With no namespace, or an
    /// empty namespace before `:`, the namespace is `minecraft`. Returns `None` for invalid
    /// identifier syntax, not for an event missing from the current packs. Parse a part's keys
    /// during loading and keep them on the part.
    pub fn parse(name: &str) -> Option<Self> {
        let id = Identifier::parse(name).ok()?;
        Some(Self(id.to_string().into()))
    }

    /// The fully qualified event name (`namespace:path`).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// no wire: an in-process sound-type identity; it is not a sound event on the wire.
/// The vanilla `SoundType` identities that affect sound selection independently
/// of their event keys (see `AbstractHorse.isWoodSoundType`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum BlockSoundTypeIdentity {
    /// An ordinary or custom sound type without one of the identities below.
    #[default]
    Other,
    /// `SoundType.WOOD`.
    Wood,
    /// `SoundType.NETHER_WOOD`.
    NetherWood,
    /// `SoundType.STEM`.
    Stem,
    /// `SoundType.CHERRY_WOOD`.
    CherryWood,
    /// `SoundType.BAMBOO_WOOD`.
    BambooWood,
}

// no wire: native block facts and a native callback edit; playback sends the selected event only.
/// A block state's sounds (`BlockState.getSoundType()`, vanilla's `SoundType`): the sound
/// event keys and the base volume and pitch. Playback uses only the event requested by the
/// current action and applies that action's volume and pitch factors.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game and mods build it with a literal
pub struct BlockSoundType {
    /// Identity independent of event keys and levels. In-place event edits keep it;
    /// use [`BlockSoundTypeIdentity::Other`] when constructing a custom sound type.
    pub identity: BlockSoundTypeIdentity,
    /// `SoundType.getVolume`.
    pub volume: f32,
    /// `SoundType.getPitch`.
    pub pitch: f32,
    /// `getBreakSound`: the block broken.
    pub break_sound: SoundEventKey,
    /// `getStepSound`: walked on.
    pub step: SoundEventKey,
    /// `getPlaceSound`: placed.
    pub place: SoundEventKey,
    /// `getHitSound`: hit while being mined, or used by a sniffer digging.
    pub hit: SoundEventKey,
    /// `getFallSound`: landed on.
    pub fall: SoundEventKey,
}

impl BlockSoundType {
    /// The event read for this action. A sniffer digging reads the hit event.
    pub fn event(&self, cause: BlockSoundCause) -> &SoundEventKey {
        match cause {
            BlockSoundCause::Step => &self.step,
            BlockSoundCause::Fall => &self.fall,
            BlockSoundCause::Place => &self.place,
            BlockSoundCause::Break => &self.break_sound,
            BlockSoundCause::Dig => &self.hit,
        }
    }
}

#[cfg(test)]
mod block_sound_tests {
    use super::SoundEventKey;

    #[test]
    fn event_keys_apply_vanilla_identifier_defaults_without_a_sound_registry() {
        for (input, expected) in [
            ("block.stone.step", "minecraft:block.stone.step"),
            (":block.stone.step", "minecraft:block.stone.step"),
            ("test:missing/path", "test:missing/path"),
            ("", "minecraft:"),
        ] {
            assert_eq!(SoundEventKey::parse(input).unwrap().as_str(), expected);
        }
        for invalid in ["x:bad path", "Upper:step", "..:step", "x:y:z"] {
            assert_eq!(SoundEventKey::parse(invalid), None, "{invalid}");
        }
    }
}

/// Why a block's sound type is read (`BlockPart::sound_type`): which of its sounds the game is
/// about to play.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum BlockSoundCause {
    /// An entity steps on it (`Entity.playStepSound`): the step sound at volume × 0.15.
    Step,
    /// A living entity lands on it hard enough to be hurt (`LivingEntity.playBlockFallSound`):
    /// the fall sound at volume × 0.5, pitch × 0.75.
    Fall,
    /// It is placed (`BlockItem.place`): the place sound at `(volume + 1) / 2`, pitch × 0.8.
    Place,
    /// It is broken (level event 2001): the break sound at `(volume + 1) / 2`, pitch × 0.8.
    Break,
    /// A sniffer digs it (`Sniffer.emitDiggingParticles`): the hit sound at a fixed volume 0.5
    /// and pitch 0.5, whatever the answer's volume and pitch.
    Dig,
}

/// A vector of doubles, as vanilla's `Vec3`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
// exhaustive: a vector is its three components, as vanilla's `Vec3`
#[derive(sunrise_mod_api_macros::Wire)]
pub struct Vec3 {
    /// East (+) and west (-).
    pub x: f64,
    /// Up (+) and down (-).
    pub y: f64,
    /// South (+) and north (-).
    pub z: f64,
}

impl Vec3 {
    /// The zero vector (`Vec3.ZERO`).
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);
    /// New.
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }
}

impl From<glam::DVec3> for Vec3 {
    fn from(v: glam::DVec3) -> Self {
        Self::new(v.x, v.y, v.z)
    }
}

impl From<Vec3> for glam::DVec3 {
    fn from(v: Vec3) -> Self {
        Self::new(v.x, v.y, v.z)
    }
}

impl LevelSide<'_> {
    pub(crate) fn reborrow(&mut self) -> LevelSide<'_> {
        match self {
            Self::Server(host) => LevelSide::Server(*host),
            Self::Client(host) => LevelSide::Client(*host),
        }
    }
}

impl LevelRandomSide<'_> {
    pub(crate) fn reborrow(&mut self) -> LevelRandomSide<'_> {
        match self {
            Self::Server(host) => LevelRandomSide::Server(&mut **host),
            Self::Client(host) => LevelRandomSide::Client(*host),
        }
    }
}

/// NeoForge's `TriState`: force a part of an action off or on, or leave it
/// to vanilla.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum TriState {
    /// Deny.
    False,
    #[default]
    /// Leave the game's own answer.
    Default,
    /// Allow.
    True,
}

impl TriState {
    /// Whether the part runs, `vanilla` deciding for `Default`.
    pub fn or(self, vanilla: bool) -> bool {
        match self {
            Self::False => false,
            Self::Default => vanilla,
            Self::True => true,
        }
    }
}

/// Read-only level services, like vanilla's `LevelReader`.
///
/// A level host cannot be used as a game host:
/// ```compile_fail
/// use mod_api::{Host, LevelHost};
/// fn game(_: &dyn Host) {}
/// fn level(host: &dyn LevelHost) { game(host); }
/// ```
/// ```compile_fail
/// use mod_api::{ClientHost, LevelHost, ServerHost};
/// fn server(_: &mut dyn ServerHost) {}
/// fn client(_: &dyn ClientHost) {}
/// fn level(host: &mut dyn LevelHost) { server(host); client(host); }
/// ```
/// A client-side level cannot write:
/// ```compile_fail
/// use mod_api::{BlockUpdateFlags, LevelSide};
/// fn write(side: LevelSide<'_>) {
///     if let LevelSide::Client(level) = side {
///         level.set_block_property([0, 0, 0], "a", "b", BlockUpdateFlags::NONE);
///     }
/// }
/// ```
pub trait LevelHost {
    /// Whether the exact, namespaced `id` is a living entity type (`LivingEntity`).
    /// Includes living content types, players and armor stands, whether alive or not in a level.
    /// Returns `None` before the registry is complete or when no classification exists for `id`.
    /// This is a load-owned registry read, not an entity lookup. It allocates nothing and gives
    /// the same answer on either side; no entity need be loaded.
    fn entity_type_is_living(&self, id: &str) -> Option<bool> {
        crate::content::registry_keys::resolved_entity_types()
            .get()?
            .is_living(id)
    }

    /// The block-state facts service, as `host.block_states()` gives it on a game host: the
    /// same tables and the same answers, so a part can ask a neighbour's `weathering_age`
    /// inside `random_tick`. Read-only and allocation-free per read. A level view with no
    /// tables answers `None`, `BlockStateError::Unavailable` or `LevelReadError::Unavailable`,
    /// according to the read's result type.
    ///
    /// Computed from this level view's tables.
    fn block_states(&self) -> crate::BlockStates<'_> {
        crate::BlockStates::unavailable()
    }
    /// `BlockState.isFaceSturdy`, through the same engine read as `LevelRef::block_face_sturdy`.
    /// `canSupportRigidBlock` is `(pos, Up, Rigid)`. `canSupportCenter` is `(pos, face, Center)`
    /// except that vanilla also refuses `Down` on blocks tagged `minecraft:unstable_bottom_center`.
    /// Uses the local shapes in an open server level or the current client level. Returns None
    /// for unloaded or out-of-height positions or without a support view. Never loads chunks or
    /// opens loot containers. Available only on the owning game thread, never in worker-thread
    /// hook bodies.
    fn block_face_sturdy(
        &self,
        _pos: [i32; 3],
        _face: crate::BlockFace,
        _support: crate::SupportType,
    ) -> Option<bool> {
        None
    }
    /// `BlockAndLightGetter.canSeeSky`: true when the level's published sky light is 15, as
    /// `LevelRef::can_see_sky` reads it. This is not a block or heightmap read. A dimension with
    /// no skylight always returns false. Returns None without a light view. Never loads chunks;
    /// available only on the owning game thread, never in worker-thread hook bodies.
    ///
    /// Stored light can include padding above and below build height, and layers an unloaded
    /// column retains beside a loaded one. Missing sky light uses the bottom row of the nearest
    /// stored sky layer above it, or 15 if there is none. The engine does not yet compute new light
    /// in padding sections (#3494) or refresh unloaded borders when a block edit changes whether
    /// a section is empty (#3495).
    fn can_see_sky(&self, _pos: [i32; 3]) -> Option<bool> {
        None
    }
    /// `BlockGetter.getBlockEntity` in this level. None if absent, unloaded, outside the
    /// build height, or temporarily detached for a menu operation. Tick callbacks keep the
    /// entity resident until removal. Never loads chunks or opens loot containers.
    /// Clients return their local block entity handle, not unsynced server contents. Owning
    /// game thread only, never reachable from a worker-thread hook body.
    fn block_entity(&self, _pos: [i32; 3]) -> Option<crate::BlockEntityRef> {
        None
    }
    /// Whether the first provider for [`crate::ITEMS`] at `pos` and `side` answers, as in
    /// NeoForge's `Level.getCapability(Capabilities.Item.BLOCK, pos, side)`. `None` as the
    /// side means unsided, not a search over faces. An empty or slotless handler can answer.
    /// Returns `Some(false)` for a resident block with no answer, and `None` for an unavailable
    /// view, unloaded chunk or invalid position. Clients use only their resident world and
    /// installed providers. Does not allocate a handler, open loot, load chunks or query the server.
    fn has_item_capability(&self, _pos: [i32; 3], _side: Option<Direction>) -> Option<bool> {
        None
    }
    /// The number of slots on this side, using local resident storage on client views.
    /// Never generates loot or loads chunks. None for unloaded positions or
    /// unsupported storage. Owning game thread only, never in worker hook bodies.
    fn container_size(&self, _at: BlockSide) -> Option<usize> {
        None
    }
    /// Lend the resident stack in this side-local slot once, using the local copy on clients.
    /// Never loads chunks or generates loot. Unopened slots remain empty.
    /// Invokes `read` only for a readable, nonempty slot in supported storage. Owning game thread only.
    /// Use `with_container_stack` on `&dyn LevelHost` to return a value from the loan.
    fn lend_container_stack(
        &self,
        _at: BlockSide,
        _slot: usize,
        _read: &mut dyn for<'a> FnMut(ItemStackView<'a>),
    ) {
    }
    /// Borrow the host for a snapshot read in the transaction that captured it.
    /// A level view without transaction state supplies only the unscoped default.
    fn attachment_read_scope(&self) -> crate::AttachmentReadScope<'_> {
        crate::AttachmentReadScope::default()
    }
    /// Capture Part state using the same attachment snapshot as `Host::query`.
    /// In a transaction it can be used only while the capturing frame is current.
    /// No chunk loading or loot generation. Refuses unloaded or stale holders; clients
    /// have no server Part state. Owning game thread only, never in worker hook bodies.
    // hidden: typed BlockEntityPartState read implementation.
    #[doc(hidden)]
    fn block_entity_part_snapshot(
        &self,
        _entity: crate::BlockEntityRef,
        _namespace: &str,
        _name: &str,
    ) -> Result<crate::AttachmentBatch, crate::PartStateError> {
        Err(crate::PartStateError::Stale)
    }
    /// Read this level's copy of a block entity's saved data without identity or position
    /// metadata. This never loads chunks or opens loot containers. A mod holder
    /// without declared inventory slots exposes saved `Items` as mod-owned NBT
    /// and preserves the saved list order.
    /// Returns `None` when no saved data is available, the handle is stale, or it names
    /// another level or world. A view with no block entities returns `None` explicitly.
    fn block_entity_data(&self, entity: crate::BlockEntityRef) -> Option<crate::Compound>;

    /// Read an owned block state through the level's engine read.
    /// Out-of-height or unloaded engine reads retain vanilla's air as `Some(air)`.
    /// Returns `None` when the owning state index is unavailable or refuses the state;
    /// this never substitutes an air state for a failed conversion.
    fn block_state(&self, _pos: [i32; 3]) -> Option<BlockState> {
        None
    }
    /// Read a state's contextual friction in this view, as `LevelRef::friction` does.
    /// The queried state need not occupy `pos`. None if the view cannot answer, the
    /// state is unknown or recursion was refused.
    fn friction(
        &self,
        _pos: [i32; 3],
        _state: BlockState,
        _entity: Option<EntityRef>,
    ) -> Option<f32> {
        None
    }
    /// The type of an entity this view lends, if it knows the entity. A friction callback
    /// can read the type of its query entity without a full game host.
    fn entity_kind(&self, _entity: EntityRef) -> Option<EntityTypeKey> {
        None
    }
    /// The block `state` is a state of (`BlockState.getBlock()`): `mymod:shaft`. None for an
    /// unknown state.
    fn state_block(&self, _state: BlockState) -> Option<&str> {
        unimplemented!("LevelHost::state_block")
    }
    /// Parse a block name and properties as `BlockStateParser.parseForBlock`; None for invalid input or unavailable context.
    fn block_state_id(&self, _name: &str) -> Option<BlockState> {
        unimplemented!("LevelHost::block_state_id")
    }
    /// The block's default state (`Block.defaultBlockState`), or None for an unknown block.
    fn block_state_default(&self, _block: BlockKey) -> Option<BlockState> {
        unimplemented!("LevelHost::block_state_default")
    }
    /// The full block state name and properties, as `BlockStateParser.serialize`; None for an unknown id.
    fn state_string(&self, _state: BlockState) -> Option<String> {
        unimplemented!("LevelHost::state_string")
    }
    /// Lowest build Y (`LevelHeightAccessor.getMinY`).
    fn min_y(&self) -> i32 {
        unimplemented!("LevelHost::min_y")
    }
    /// Build height (`LevelHeightAccessor.getHeight`).
    fn height(&self) -> i32 {
        unimplemented!("LevelHost::height")
    }
    /// The merged-tag service for this level's owning-thread context: a frame resource, whose
    /// methods are the imports.
    fn tags(&self) -> &dyn Tags {
        unimplemented!("LevelHost::tags")
    }
    /// `BlockGetter.getFluidState`: the fluid at `pos`, the empty fluid where there is none.
    /// None where the position is not loaded or outside the build height; it never loads a chunk.
    fn fluid_state(&self, _pos: [i32; 3]) -> Option<FluidState> {
        unimplemented!("LevelHost::fluid_state")
    }
    /// `BlockState.setValue`, through `native_state_with_property`; None if invalid.
    fn state_with_property(
        &self,
        _state: BlockState,
        _key: &str,
        _value: &str,
    ) -> Option<BlockState> {
        unimplemented!("LevelHost::state_with_property")
    }
    /// The stored light of one layer at `pos` (`BlockAndLightGetter.getBrightness`), 0 to 15.
    /// It reads published light, including stored padding and retained border layers (see
    /// [`Self::can_see_sky`]). Missing block light is zero. It never loads a chunk.
    fn brightness(&self, _layer: LightLayer, _pos: [i32; 3]) -> i32 {
        unimplemented!("LevelHost::brightness")
    }
    /// `BlockAndLightGetter.getRawBrightness(pos, darkening)`: the brighter of the block light
    /// and the sky light less `sky_darken`. `CropBlock.randomTick` asks it with 0.
    fn raw_brightness(&self, _pos: [i32; 3], _sky_darken: i32) -> i32 {
        unimplemented!("LevelHost::raw_brightness")
    }
    /// `Level.isLoaded`: `pos` is inside the build height and its chunk is loaded. Asking never
    /// loads the chunk.
    fn is_loaded(&self, _pos: [i32; 3]) -> bool {
        unimplemented!("LevelHost::is_loaded")
    }
    /// `Level.isRainingAt`: it rains in this level and `pos` is open to the sky (full sky light,
    /// nothing motion-blocking or fluid above it) in a biome whose precipitation there is rain,
    /// not snow or none. False where `pos` is not loaded, the chunk is never loaded for it.
    fn is_raining_at(&self, _pos: [i32; 3]) -> bool {
        unimplemented!("LevelHost::is_raining_at")
    }
    /// `Level.isRaining`: whether it rains in this level (its rain level is above 0.2),
    /// wherever a position is. Both sides answer for the level the view belongs to. None in a
    /// callback whose level view has no weather: the friction and hit hooks have none.
    fn is_raining(&self) -> Option<bool> {
        None
    }
    /// `Level.getSkyDarken`: how much the sky light is dimmed this tick, 0 to 11, from the
    /// time of day, rain and thunder. With [`Self::raw_brightness`] it gives
    /// `LevelReader.getMaxLocalRawBrightness(pos)`, which `SaplingBlock.randomTick` tests
    /// (`>= 9`). Both sides answer for the level the view belongs to. None where
    /// [`Self::is_raining`] is None.
    fn sky_darken(&self) -> Option<i32> {
        None
    }
    /// `LevelReader.enabledFeatures`: the feature flags the world enables.
    fn enabled_features(&self) -> FeatureFlags {
        unimplemented!("LevelHost::enabled_features")
    }
    /// How strongly the block at `pos` powers its neighbour on the `direction` side, power it
    /// conducts included (`SignalGetter.getSignal(pos, direction)`), 0 to 15: the game's own
    /// read, mods' `block_signal` and `should_check_weak_power` parts included, as
    /// `LevelRef::signal` reads it. It writes nothing and draws nothing, so it leaves the
    /// call it is read in as it was, and it asks no placement or survival hook, so a read made
    /// inside `state_for_placement` or `can_survive` cannot re-enter them. None in a callback
    /// whose level has no signal view: `state_for_placement`, `can_survive`, `may_place_on`,
    /// `can_sustain_plant` and the level tick and shape-update hooks have one. None on a client
    /// for a read that needs a comparator's output the client doesn't hold.
    fn signal(&self, _pos: [i32; 3], _direction: Direction) -> Option<i32> {
        None
    }
    /// The strong power alone that the block at `pos` gives that way
    /// (`SignalGetter.getDirectSignal(pos, direction)`), 0 to 15. None as for `signal`.
    fn direct_signal(&self, _pos: [i32; 3], _direction: Direction) -> Option<i32> {
        None
    }
    /// The strongest strong power reaching the block at `pos` (`SignalGetter.getDirectSignalTo`).
    /// None as for `signal`.
    fn direct_signal_to(&self, _pos: [i32; 3]) -> Option<i32> {
        None
    }
    /// The strongest power reaching the block at `pos` from its six neighbours
    /// (`SignalGetter.getBestNeighborSignal`). None as for `signal`.
    fn best_neighbor_signal(&self, _pos: [i32; 3]) -> Option<i32> {
        None
    }
    /// Whether any neighbour powers the block at `pos` (`SignalGetter.hasNeighborSignal`),
    /// asking no side past the first that does: what a lamp's or a door's placement reads.
    /// None as for `signal`.
    fn has_neighbor_signal(&self, _pos: [i32; 3]) -> Option<bool> {
        None
    }
    /// The block-local shape (`BlockState.getCollisionShape` with an empty context), using a
    /// block entity's published shape when present. `VoxelShape::boxes` visits the geometry
    /// without copying it. Returns `None` for unloaded or out-of-height positions and a
    /// callback that has no shape view. For a handle, use `LevelRef::visit_block_collision`.
    fn collision_shape(&self, _pos: [i32; 3]) -> Option<VoxelShape> {
        None
    }
}

impl dyn LevelHost + '_ {
    /// Read a side-local resident stack without taking ownership. Returns `None` if
    /// [`LevelHost::lend_container_stack`] cannot lend the slot. Item, count and damage need
    /// no projection; call [`ItemStackView::project`] inside `read` to keep an owned stack
    /// or inspect its component patch. The view itself cannot leave the callback.
    pub fn with_container_stack<R>(
        &self,
        at: BlockSide,
        slot: usize,
        read: impl for<'a> FnOnce(ItemStackView<'a>) -> R,
    ) -> Option<R> {
        crate::item::item_stack::read_stack_view(read, |lend| {
            self.lend_container_stack(at, slot, lend);
            Some(())
        })
    }
}

/// Restricted access to the hook's own level and its live `Level.getRandom` stream.
pub trait LevelRandomHost: LevelHost {
    /// Vanilla comparator container signal, 0 to 15, through the engine comparator path.
    /// Generates unopened container loot once before reading, as vanilla getItem does;
    /// stack maxima are honoured. `Some(0)` at a loaded position without a usable container view.
    /// None on clients, for unloaded or out-of-height positions.
    /// Never loads chunks. Owning game thread only, never
    /// reachable from a worker-thread hook body.
    fn container_signal(&mut self, _pos: [i32; 3]) -> Option<i32> {
        None
    }
    /// The holders' server-owned attachments; `key.get_in(level, holder, read)` reads one through it.
    fn attachments(&mut self) -> &mut dyn AttachmentStore {
        unimplemented!("LevelRandomHost::attachments")
    }
    /// Lend the level's live random stream to `draw` for this call by using
    /// [`RandomDraw::run`]. A host that cannot lend the stream declines by not running it,
    /// as the default implementation does. Draws advance the live stream directly and are
    /// not rolled back on panic/unwind or transaction abort, matching vanilla
    /// `Level.getRandom()`.
    ///
    /// Listeners request a draw with `host.with_random(|rng| rng.next_int_bound(bound))`.
    fn lend_random(&mut self, _draw: RandomDraw<'_>) {}
}

/// What a hit's listener may do beyond reading and drawing the level's random: play level
/// sounds and draw from the random of an entity the hit lends. It is the host of
/// `living_incoming_damage`, `living_damage_pre`, `living_damage_post`, `living_shield_block`
/// and `living_use_totem`. It writes no block and schedules no tick.
///
/// A sound is queued at the call, and its seed is drawn then from the level's sound-seed
/// stream, never from the level's [draw stream](LevelRandomHost::lend_random). An entity's
/// own stream (`Entity.getRandom`) advances in listener order, so the hit's continuation sees
/// the draws. A hit lends the entity its source names as its causing entity, where the game
/// has it at hand: the attacking player or mob of a melee hit (player → mob, mob → mob and
/// mob → player) or of Thorns, the shooter or owner of a projectile, a firework rocket, evoker
/// fangs, a wither skull, a thrown trident or a wind charge, the igniter or owner of an
/// explosion, and a pufferfish stinging a mob inside its own tick. A guest that is not in place
/// is lent from its hub record, for its reads only (no random, no sound); the integrated owner
/// displaced by another player's turn is not lent. Other damage lends none: damage with no
/// causing entity (environment, fall, fire, drowning) and the sites that do not lend theirs (a
/// thrown potion's or an effect cloud's owner, among others). When the victim caused its own
/// hit (a player's own TNT hurting that player), the source names the victim and nothing else is
/// lent: [`causing`](Self::causing) answers `None` and the listener reads the entity through
/// [`victim`](Self::victim). Any other entity, a stale handle and a call on a client answer
/// `None` or `false`.
pub trait HitHost: LevelRandomHost {
    /// `Level.playSound(null, x, y, z, sound, source, volume, pitch)`: plays sound event
    /// `sound` at `at` for every player in this level within range (16 blocks, times the
    /// volume above 1), under `source`'s volume slider. Registered events go by ID and other
    /// valid keys by name, with volume and pitch as supplied. True means queued, not heard;
    /// false for a malformed event key or where no sound can be sent.
    fn play_sound_at(
        &mut self,
        _sound: &str,
        _at: [f64; 3],
        _source: SoundSource,
        _volume: f32,
        _pitch: f32,
    ) -> bool {
        unimplemented!("HitHost::play_sound_at")
    }
    /// `Level.playSound(null, entity, sound, source, volume, pitch)`: the same sound bound to
    /// `entity`, so it follows it, heard by every player within range of where the entity is now,
    /// the entity's own player too (it is not `EntityRef::play_sound`, which leaves that player
    /// out and plays nothing for a silent mob). Uses the same event resolution and volume and
    /// pitch as `play_sound_at`. False for a malformed key or a handle the hit does not lend.
    fn play_sound_from(
        &mut self,
        _entity: EntityRef,
        _sound: &str,
        _source: SoundSource,
        _volume: f32,
        _pitch: f32,
    ) -> bool {
        unimplemented!("HitHost::play_sound_from")
    }
    /// Lend the entity's live `Entity.getRandom` stream for one draw closure.
    /// Decline an entity this hit does not lend without invoking the closure. Draws are
    /// not rewound on panic or abort. Use [`EntityRef::with_random`] to request a loan.
    fn lend_entity_random(&mut self, _entity: EntityRef, _draw: RandomDraw<'_>) {}
    /// The entity the hit lands on, when it is living (`LivingIncomingDamageEvent.getEntity`), at
    /// `living_incoming_damage`, `living_damage_pre` and `living_damage_post`: a mob, a player
    /// or the dragon (an armor stand's own `hurtServer` never reaches these hooks). Its reads
    /// answer from the live entity at the call. None at `living_shield_block` and
    /// `living_use_totem`, which lend no view.
    fn victim(&self) -> Option<LivingView<'_>> {
        unimplemented!("HitHost::victim")
    }
    /// The hit's causing entity (`DamageSource.getEntity`), at the same three hooks, when the
    /// hit lends that entity (see [`HitHost`]: a melee attacker, a projectile's shooter, an
    /// explosion's igniter, a parked guest's record...) and the source names it: None where the
    /// source names another entity or none (a wither skull whose shooter was removed), where it
    /// names the victim itself (read that through [`Self::victim`]), for a removed entity, at a
    /// damage site that lends no entity, for a non-living causer and for a causer in another
    /// level.
    fn causing(&self) -> Option<LivingView<'_>> {
        unimplemented!("HitHost::causing")
    }
}

/// A `LevelReader` with vanilla's `ScheduledTickAccess`. Shape updates cannot write blocks.
pub trait LevelTicksHost: LevelHost {
    /// Schedule a block tick through `native_schedule_block_tick`.
    fn schedule_block_tick(&mut self, _pos: [i32; 3], _block: BlockKey, _delay: i32) -> bool {
        unimplemented!("LevelTicksHost::schedule_block_tick")
    }
    /// Schedule a fluid tick through `native_schedule_fluid_tick`.
    fn schedule_fluid_tick(&mut self, _pos: [i32; 3], _fluid: &str, _delay: i32) -> bool {
        unimplemented!("LevelTicksHost::schedule_fluid_tick")
    }
}

/// Mutable level services, like vanilla's `ServerLevel`.
pub trait LevelWriteHost: LevelTicksHost + LevelRandomHost {
    /// Replace the block at `pos` with `state` through `Level.setBlock(pos, state, flags)`, as
    /// `LevelRef::set_block` does: the old block's block entity goes and the new block's is
    /// created in the write, its neighbour and shape updates run per `flags`, and its level events
    /// queue in order with the caller's. A mod block entity starts with the data its
    /// `block_entity` hook gives, which gets this level. True when the block changed; false off an
    /// authoritative server, for a position that is not loaded or outside the build height, an
    /// invalid state, or the state it already is. It never loads a chunk.
    fn set_block(&mut self, _pos: [i32; 3], _state: BlockState, _flags: BlockUpdateFlags) -> bool {
        unimplemented!("LevelWriteHost::set_block")
    }
    /// Remove a block through `Level.removeBlock`, preserving its fluid. `moved` marks a piston move.
    fn remove_block(&mut self, _pos: [i32; 3], _moved: bool) -> bool {
        unimplemented!("LevelWriteHost::remove_block")
    }
    /// Change a block property through `Level.setBlock` with [`BlockUpdateFlags`], preserving its block entity.
    fn set_block_property(
        &mut self,
        _pos: [i32; 3],
        _key: &str,
        _value: &str,
        _flags: BlockUpdateFlags,
    ) -> bool {
        unimplemented!("LevelWriteHost::set_block_property")
    }
    /// Queue a sound for nearby players under the given [`SoundSource`]. Registered events go by ID and other valid keys by
    /// name, with volume and pitch as supplied.
    /// Inside an open transaction in the active level, the level-sound hook runs synchronously.
    /// Only the resolved sound waits for the owning level's outermost commit and draws its seed
    /// when published. An abort discards it. Outside a transaction a queued sound draws its
    /// seed at the call, even if a later hook cancels it.
    fn play_sound(
        &mut self,
        _sound: &str,
        _at: [f64; 3],
        _source: SoundSource,
        _volume: f32,
        _pitch: f32,
    ) -> bool {
        unimplemented!("LevelWriteHost::play_sound")
    }
    /// `ServerLevel.sendParticles` through `native_send_particles`.
    /// Item particles require `options.item_stack`; invalid stacks return false, and so does an
    /// item crumb whose colour has a component outside 0 to 1. Inside an open transaction the
    /// particles wait for the outermost commit and are discarded on abort.
    fn send_particles(
        &mut self,
        _options: &ParticleOptions,
        _state: Option<BlockState>,
        _at: [f64; 3],
        _spread: [f64; 3],
        _speed: f64,
        _count: i32,
    ) -> bool {
        unimplemented!("LevelWriteHost::send_particles")
    }
    /// `LevelAccessor.levelEvent(null, id, pos, data)` through `native_level_event`: the event
    /// is queued with the write's own events, in order. Inside an open transaction it plays at the
    /// outermost commit, in call order with the other deferred effects, and is discarded on abort;
    /// otherwise it plays when the write is done.
    /// `LevelRef::level_event` takes the same `id` and `data`.
    fn level_event(&mut self, _id: i32, _pos: [i32; 3], _data: i32) -> bool {
        unimplemented!("LevelWriteHost::level_event")
    }
}

/// Search parameters for `LevelRef::find_nearest_structure`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct StructureSearch {
    /// Where the search starts, in block coordinates; the distance is horizontal.
    pub origin: [i32; 3],
    /// How many rings of placement cells around the origin's are searched for
    /// random-spread structure sets (vanilla's `searchRadius`); `/locate` uses 100.
    pub radius: i32,
}

impl StructureSearch {
    /// The radius must be nonnegative.
    pub fn new(origin: [i32; 3], radius: i32) -> Self {
        Self { origin, radius }
    }
}

/// The nearest structure start a search found.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct StructureSearchResult {
    /// Registry id of the structure, the tag's member that matched for a `#tag`.
    pub structure: String,
    /// Where `/locate` reports the start (`StructurePlacement.getLocatePos`):
    /// its chunk's corner at y 0, moved by the set's `locate_offset`.
    pub position: [i32; 3],
}

impl StructureSearchResult {
    /// Pair a found structure with its start position.
    pub fn new(structure: String, position: [i32; 3]) -> Self {
        Self {
            structure,
            position,
        }
    }
}

/// Why a server-wide game rule read was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum GameRuleReadError {
    /// No rule has this identifier.
    UnknownRule,
}

/// One of the scoreboard's objectives of an open server world (vanilla's `Objective`). It holds
/// the world's epoch and an identity the board gave the objective when it was added, so an
/// objective that was removed (even if its name was added again) or a handle from an earlier
/// open reads as stale: reads answer `None`, [`ObjectiveError::Stale`] or
/// [`ObjectiveReadError::Stale`], and writes change nothing.
#[derive(sunrise_mod_api_macros::Wire, Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ObjectiveRef {
    pub(crate) epoch: WorldEpoch,
    pub(crate) id: u64,
}

/// One of the scoreboard's teams of an open server world (vanilla's `PlayerTeam`). It holds the
/// world's epoch and an identity the board gave the team when it was added, so a team that was
/// removed (even if its name was added again) or a handle from an earlier open reads as stale:
/// reads answer `None`, [`TeamError::Stale`] or [`TeamMembersReadError::Stale`], and writes
/// change nothing.
#[derive(sunrise_mod_api_macros::Wire, Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct TeamRef {
    pub(crate) epoch: WorldEpoch,
    pub(crate) id: u64,
}

/// Why a bulk block edit through [`LevelRef::edit_blocks`] was refused before changing any block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum BlockEditError {
    /// The level is gone or its handle belongs to an earlier lifetime.
    Stale,
    /// The operation is unavailable on this side, phase or host.
    Unavailable,
    /// A palette entry is not a valid block state in the owning registry.
    InvalidState(BlockStateError),
    /// A filter predicate failed typed validation or its block or tag is unavailable.
    InvalidPredicate,
    /// A run names a palette entry past the end of `BlockEdit::palette`.
    PaletteOutOfRange,
    /// A run names a filter past the end of `BlockEdit::filters`.
    FilterOutOfRange,
}

impl crate::NoBackend for BlockEditError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

impl From<BlockStateError> for BlockEditError {
    fn from(error: BlockStateError) -> Self {
        match error {
            BlockStateError::Unavailable => Self::Unavailable,
            error => Self::InvalidState(error),
        }
    }
}
