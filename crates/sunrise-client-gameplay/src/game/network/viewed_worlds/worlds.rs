//! The worlds of the levels a client views: one `World` per level the server opened, with the
//! chunk format, shape and facts of the dimension type the server synced for it. A wrapped
//! packet changes only its own level's world; nothing here reaches the player's level, its
//! renderer, its sounds or a mod's hooks.
use super::{
    entities::ViewedEntities,
    meshing::{block_changed, column_changed},
    routing::RouteError,
};
use crate::game::{
    native_connection_groups::{GroupUpdates, column_groups},
    remote_chunks::{ChunkFormat, section_blocks},
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use sunrise_client::streaming::ViewedMeshing;
use sunrise_level::chunk_streamer::take_resent_column;
use sunrise_level::{
    chunk::known_block_state,
    chunk_map::ChunkMap,
    fast_hash::{FxHashMap, FxHashSet},
    world::World,
};
use sunrise_protocol::{
    chunk::BlockEntityInfo,
    connection_groups::Section,
    packets::play::{self, Clientbound as C, ClockState},
};
use sunrise_world::registry::BlockStateId;

/// Another level's world, kept while the server sends it.
pub struct ViewedWorld {
    /// This world's own id, which its meshed sections go by (`meshing`): a level opened again
    /// has another.
    pub(super) id: u64,
    /// The level's key (`minecraft:the_nether`).
    pub level: String,
    /// Its key as a mod's `LevelRef` names it.
    pub(super) key: mod_api::DimensionKey,
    /// Its dimension has the Nether's cardinal light: its sections are shaded so.
    pub(in crate::game) nether_shading: bool,
    /// What its chunk packets decode with.
    format: Arc<ChunkFormat>,
    /// The world clock its day time follows: its dimension type's `default_clock`.
    default_clock: Option<i32>,
    /// That clock as the server last told it, moved on with the game time since
    /// (`ClientClockManager`).
    clock: Option<ClockState>,
    pub world: World,
    pub entities: ViewedEntities,
    /// The block entities of each column as its chunk packet carried them, with the data sent
    /// since: what a travel into the level loads (`switch`). A world that was the player's keeps
    /// its own in `world.block_entities` besides.
    pub block_entities: FxHashMap<(i32, i32), Vec<BlockEntityInfo>>,
    /// The most columns it holds: the cap its `open` said, at most
    /// `viewed_levels::MAX_COLUMNS`, so that a server cannot fill the client's memory through
    /// one opened level.
    pub(in crate::game) columns: u32,
}

impl ViewedWorld {
    pub(in crate::game::client_side) fn new(
        level: String,
        format: Arc<ChunkFormat>,
        default_clock: Option<i32>,
        world: World,
        columns: u32,
    ) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        Self {
            columns,
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            key: mod_api::__private::dimension_key(&level),
            level,
            nether_shading: false,
            format,
            default_clock,
            clock: None,
            world,
            entities: ViewedEntities::default(),
            block_entities: FxHashMap::default(),
        }
    }

    /// A wrapped packet of the level, as `ClientPacketListener` handles it with that level
    /// current: what the world keeps of its chunks, light, biomes, blocks, time, weather and
    /// entities. Block events, destruction progress and level events change nothing yet: what
    /// they show is drawn, and drawing a viewed level is gap 13. A chunk of a new column past
    /// its cap ([`Self::columns`]) is refused before it is decoded.
    pub(super) fn apply(
        &mut self,
        packet: C,
        mut groups: GroupUpdates,
        snapshot: &[Section],
        group_limit: usize,
    ) -> Result<(), RouteError> {
        match packet {
            // `handleLevelChunkWithLight`: the column replaces any it held, with the light the
            // server sent.
            C::LevelChunkWithLight(p)
                if self.world.chunks.len() >= self.columns as usize
                    && !self.world.chunks.contains_key(&(p.x, p.z)) =>
            {
                return Err(RouteError::TooManyColumns(self.columns));
            }
            C::LevelChunkWithLight(p) => match self.format.column(&p) {
                Ok(mut chunk) => {
                    let key = (p.x, p.z);
                    column_groups(&mut chunk, &self.world, snapshot, group_limit)
                        .map_err(RouteError::Groups)?;
                    if let Some(old) = self.world.chunks.get(&key).cloned() {
                        take_resent_column(&mut self.world, &old, chunk, false);
                    } else {
                        self.world.install_light_column(chunk);
                        column_changed(&mut self.world, key);
                    }
                    self.unload_block_entities(key);
                    self.block_entities.insert(key, p.chunk.block_entities);
                }
                Err(e) if !snapshot.is_empty() => {
                    return Err(RouteError::Groups(sunrise_protocol::Error::new(e)));
                }
                Err(e) => {
                    log::warn!("Sunrise: {} chunk {} {}: {e}", self.level, p.x, p.z);
                }
            },
            C::ForgetLevelChunk(p) => {
                let key = (p.pos[0], p.pos[1]);
                self.world.unload_chunk(key);
                self.unload_block_entities(key);
                column_changed(&mut self.world, key);
            }
            C::LightUpdate(p) => {
                let layers = self.format.light(&p.light);
                if let Some(chunk) = self.world.chunks.get_mut(&(p.x, p.z)) {
                    sunrise_level::light::update_light_layers(
                        Arc::make_mut(chunk),
                        &layers,
                        self.format.has_skylight,
                    );
                    column_changed(&mut self.world, (p.x, p.z));
                }
            }
            C::ChunksBiomes(p) => {
                for data in &p.chunks {
                    let [x, z] = data.pos;
                    if let Some(chunk) = self.world.chunks.get_mut(&(x, z))
                        && let Err(e) = self.format.replace_biomes(Arc::make_mut(chunk), data)
                    {
                        log::warn!("Sunrise: {} biomes of chunk {x} {z}: {e}", self.level);
                    }
                    column_changed(&mut self.world, (x, z));
                }
            }
            C::BlockUpdate(p) => {
                self.set_block(p.pos, p.state, groups.remove(&p.pos));
            }
            C::SectionBlocksUpdate(p) => {
                for (pos, state) in section_blocks(&p) {
                    self.set_block(pos, state, groups.remove(&pos));
                }
            }
            // `handleSetTime` / `ClientClockManager.handleUpdates`: the clock it follows moves on
            // by the game time since the last packet at its rate, then takes what the packet
            // carries of it; its day follows that clock.
            // A time whose distance from the last, or whose clock's total, passes the range of a
            // long is refused before anything changes.
            C::SetTime(p) => {
                let delta = (p.game_time.checked_sub(self.world.game_time))
                    .ok_or(RouteError::ClockOverflow)?;
                if let Some(clock) = &mut self.clock {
                    let mut moved = clock.clone();
                    sunrise_gameplay_common::game::remote::client_level_tick::advance_clock(
                        &mut moved, delta,
                    )
                    .ok_or(RouteError::ClockOverflow)?;
                    *clock = moved;
                }
                self.world.game_time = p.game_time;
                if let Some(update) = p
                    .clocks
                    .into_iter()
                    .find(|c| Some(c.clock) == self.default_clock)
                {
                    self.clock = Some(update);
                }
                if let Some(clock) = &self.clock {
                    self.world.day_time = clock.total_ticks;
                }
            }
            C::GameEvent(p) => self.world.weather.game_event(p.event, p.param),
            other => self.entities.apply(other)?,
        }
        Ok(())
    }

    /// A server's block: a state the client lacks (a server with mods it lacks) reads as air,
    /// as in its chunks.
    fn set_block(
        &mut self,
        [x, y, z]: [i32; 3],
        state: i32,
        group: Option<crate::game::native_connection_groups::ReceivedGroup>,
    ) {
        if self.world.set_stored(x, y, z, stored_state(state)) {
            if let Some(group) = group {
                group.apply(&mut self.world, [x, y, z]);
            }
            block_changed(&mut self.world, [x, y, z]);
        }
    }

    /// `LevelChunk.setBlockState` with another block at `pos`: the block entity there goes, with
    /// the data kept for it (a block's own state change keeps them).
    pub(super) fn forget_block_entity(&mut self, [x, y, z]: [i32; 3]) {
        let packed_xz = ((x & 15) << 4 | (z & 15)) as i8;
        if let Some(infos) = self.block_entities.get_mut(&(x >> 4, z >> 4)) {
            infos.retain(|info| (info.packed_xz, i32::from(info.y)) != (packed_xz, y));
        }
        self.world.block_entities.remove(&(x, y, z));
    }

    /// The column `key` goes, or is sent again: so do its block entities.
    fn unload_block_entities(&mut self, key: (i32, i32)) {
        self.block_entities.remove(&key);
        self.world.block_entities.unload_column(key);
    }

    /// A block with a block entity placed at `pos` (`LevelChunk.setBlockState`): the travel makes
    /// its block entity, with the data sent for it since, if any.
    pub(super) fn placed_block_entity(&mut self, pos: [i32; 3]) {
        let placed = play::BlockEntityData {
            pos,
            kind: -1,
            tag: sunrise_core::nbt::Tag::End,
        };
        self.block_entity_data(placed, true);
    }

    /// `handleBlockEntityData`: a block entity's data, kept for the column that holds it, when
    /// the column listed a block entity there or `entity_block`, the block there now has one (a
    /// block entity placed since). Other data is dropped, as by the vanilla client, so what a
    /// world keeps is bounded by its columns' block-entity blocks.
    pub(super) fn block_entity_data(&mut self, p: play::BlockEntityData, entity_block: bool) {
        let [x, y, z] = p.pos;
        let Ok(y16) = i16::try_from(y) else {
            return;
        };
        let packed_xz = ((x & 15) << 4 | (z & 15)) as i8;
        let at = |info: &BlockEntityInfo| info.packed_xz == packed_xz && info.y == y16;
        let column = (x >> 4, z >> 4);
        if !self.world.chunks.contains_key(&column) {
            return;
        }
        let infos = self.block_entities.entry(column).or_default();
        let info = BlockEntityInfo {
            packed_xz,
            y: y16,
            kind: p.kind,
            tag: p.tag,
        };
        match infos.iter_mut().find(|i| at(i)) {
            Some(kept) => *kept = info,
            None if entity_block => infos.push(info),
            None => {}
        }
    }
}

/// The state a server's block update leaves: one the client lacks reads as air.
pub(super) fn stored_state(state: i32) -> BlockStateId {
    let state = state.max(0) as u32;
    BlockStateId(if known_block_state(state) { state } else { 0 })
}

/// The worlds a connection views (`Remote::viewed`), by the slot an `open` bound each level to
/// (`sunrise_protocol::viewed_levels`, revision 4).
#[derive(Default)]
pub struct ViewedWorlds {
    /// Each bound slot's world, with the epoch of the `open` or travel swap that bound it.
    pub(super) worlds: FxHashMap<i32, (i32, ViewedWorld)>,
    pub(in crate::game) wanted: FxHashSet<mod_api::DimensionKey>,
    pub(super) meshing: Option<ViewedMeshing>,
    /// The newest epoch an `open` or a travel swap gave on this connection. Dropping every world
    /// leaves it: the server's counter goes on.
    latest: Option<i32>,
    /// The world of the level a travel just left, with the slot of the level entered: it waits
    /// for the server's `swap` right after the Respawn, which binds it there.
    left: Option<(i32, ViewedWorld)>,
}

/// Where a `run` or `close` of a slot at an epoch goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) enum Binding {
    /// The slot is bound with that epoch: to its world.
    Live,
    /// An epoch no newer than the latest that the slot is not bound with: a packet of a level the
    /// slot no longer holds, dropped.
    Stale,
    /// An epoch newer than any `open` gave: a server's error.
    Unknown,
}

impl ViewedWorlds {
    /// Where a `run` or `close` of `slot` at `epoch` goes.
    pub(in crate::game::client_side) fn binding(&self, slot: i32, epoch: i32) -> Binding {
        if self.latest.is_none_or(|latest| epoch > latest) {
            return Binding::Unknown;
        }
        match self.worlds.get(&slot) {
            Some((bound, _)) if *bound == epoch => Binding::Live,
            _ => Binding::Stale,
        }
    }
    /// Whether an `open` at `epoch` is newer than every epoch given before, as each must be.
    pub(in crate::game::client_side) fn is_new(&self, epoch: i32) -> bool {
        self.latest.is_none_or(|latest| epoch > latest)
    }
    /// How many levels it views.
    #[cfg(any(test, feature = "test-support"))]
    pub fn len(&self) -> usize {
        self.worlds.len()
    }
    pub(in crate::game::client_side) fn get_mut(&mut self, level: i32) -> Option<&mut ViewedWorld> {
        self.worlds.get_mut(&level).map(|(_, world)| world)
    }
    /// The key of the level named `level`, while the server keeps it open: what `host.level`
    /// hands a client mod for it.
    pub(in crate::game::client_side) fn key_of(
        &self,
        level: &str,
    ) -> Option<mod_api::DimensionKey> {
        self.worlds
            .values()
            .find(|(_, w)| w.level == level)
            .map(|(_, w)| w.key)
    }
    /// The world of the level `key` names, while the server keeps it open: what a client mod's
    /// reads through its handle read.
    pub(in crate::game::client_side) fn world_of(
        &self,
        key: mod_api::DimensionKey,
    ) -> Option<&World> {
        self.worlds
            .values()
            .find(|(_, w)| w.key == key)
            .map(|(_, w)| &w.world)
    }
    /// The slot of the level named `level`, if the client views it.
    pub fn slot_of(&self, level: &str) -> Option<i32> {
        self.worlds
            .iter()
            .find(|(_, (_, w))| w.level == level)
            .map(|(&slot, _)| slot)
    }
    /// The world of `slot`, if one is bound to it.
    pub fn get(&self, slot: i32) -> Option<&ViewedWorld> {
        self.worlds.get(&slot).map(|(_, world)| world)
    }
    /// Binds `slot` to `world` at `epoch`, replacing any world it held: an `open` starts afresh.
    pub(in crate::game::client_side) fn insert(
        &mut self,
        slot: i32,
        epoch: i32,
        world: ViewedWorld,
    ) {
        self.latest = Some(self.latest.map_or(epoch, |latest| latest.max(epoch)));
        if let Some((_, old)) = self.worlds.insert(slot, (epoch, world)) {
            self.forget_sections(&old);
            free(old);
        }
    }
    /// Takes `slot`'s world out, whole: a travel into it (`switch`).
    pub(in crate::game::client_side) fn take(&mut self, slot: i32) -> Option<ViewedWorld> {
        let (_, world) = self.worlds.remove(&slot)?;
        self.forget_sections(&world);
        Some(world)
    }
    /// Drops `slot`'s world (`close`), releasing the slot; false when it held none.
    pub(in crate::game::client_side) fn remove(&mut self, slot: i32) -> bool {
        let Some((_, world)) = self.worlds.remove(&slot) else {
            return false;
        };
        self.forget_sections(&world);
        free(world);
        true
    }
    /// Drops every world: a Login, a Respawn or a new configuration phase. The epochs go on.
    pub(in crate::game::client_side) fn clear(&mut self) {
        self.meshing = None;
        for (_, (_, world)) in self.worlds.drain() {
            free(world);
        }
        if let Some((_, world)) = self.left.take() {
            free(world);
        }
    }
    /// Keeps `world`, the level a travel left, until the `swap` that binds it to `slot`, the slot
    /// of the level entered.
    pub(in crate::game::client_side) fn hold_left(&mut self, slot: i32, world: ViewedWorld) {
        if let Some((_, old)) = self.left.replace((slot, world)) {
            self.forget_sections(&old);
            free(old);
        }
    }
    /// The server's `swap`: binds the level a travel left to `slot` at `epoch`. False when the
    /// client made no travel swap to `slot`, which it refuses.
    pub(in crate::game::client_side) fn bind_left(&mut self, slot: i32, epoch: i32) -> bool {
        match self.left.take() {
            Some((held, world)) if held == slot => {
                self.insert(slot, epoch, world);
                true
            }
            other => {
                self.left = other;
                false
            }
        }
    }
    fn forget_sections(&mut self, world: &ViewedWorld) {
        if let Some(meshing) = &mut self.meshing {
            meshing.forget(world.id);
        }
    }
}

/// A world given up: its columns are freed off the game thread when there are many.
fn free(mut world: ViewedWorld) {
    let chunks = detach_columns_for_free(&mut world);
    let count = chunks.len();
    sunrise_level::chunk::free_columns(chunks, count);
}

fn detach_columns_for_free(world: &mut ViewedWorld) -> ChunkMap {
    let mut chunks = std::mem::take(&mut world.world.chunks);
    // The level left routing now, regardless of when the free worker runs.
    chunks.untrack_connection_groups();
    chunks
}

#[cfg(feature = "client")]
#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_protocol::{
        chunk::{ChunkData, LightData, Paletted, Section, Strategy},
        packets::play,
        registry::DimensionType,
    };

    /// A column of stone at `x, 0`, unlit.
    fn column(x: i32) -> C {
        let overworld = DimensionType::vanilla("minecraft:overworld");
        let section = Section {
            non_empty: 4096,
            fluids: 0,
            states: Paletted::from_values(&[1; 4096], Strategy::Blocks, 16),
            biomes: Paletted::from_values(&[1; 64], Strategy::Biomes, 2),
        };
        C::LevelChunkWithLight(play::LevelChunkWithLight {
            x,
            z: 0,
            chunk: ChunkData {
                heightmaps: Vec::new(),
                buffer: ChunkData::from_sections(&vec![section; overworld.sections()]),
                block_entities: Vec::new(),
            },
            light: LightData {
                sky_mask: Vec::new(),
                block_mask: Vec::new(),
                empty_sky_mask: Vec::new(),
                empty_block_mask: Vec::new(),
                sky: Vec::new(),
                block: Vec::new(),
            },
        })
    }

    #[test]
    fn detaching_a_large_viewed_level_refunds_group_credit_before_its_columns_drop() {
        use sunrise_level::{chunk::Chunk, chunk_map::ConnectionGroupCount};

        const LIMIT: usize = 1;
        let format = Arc::new(ChunkFormat::new(
            &DimensionType::vanilla("minecraft:overworld"),
            vec![0, 7, 9],
            2,
        ));
        let count = ConnectionGroupCount::default();
        let mut viewed = ViewedWorld::new(
            "minecraft:the_nether".into(),
            format,
            None,
            World::new(BlockStateId(0), 1),
            64,
        );
        viewed.world.chunks.track_connection_groups(count.clone());
        let mut grouped = Chunk::empty(0, 0);
        assert_eq!(grouped.set_connection_group(0, 64, 0, Some(13)), Some(true));
        viewed.world.chunks.insert((0, 0), Arc::new(grouped));
        for x in 1..64 {
            viewed
                .world
                .chunks
                .insert((x, 0), Arc::new(Chunk::empty(x, 0)));
        }

        let mut active = ChunkMap::default();
        active.track_connection_groups(count.clone());
        assert!(!active.allows_connection_groups(0, 1, LIMIT));
        // Keep the exact map handed to the worker alive, instead of racing its drop.
        let detached = detach_columns_for_free(&mut viewed);
        assert_eq!(detached.len(), 64);
        assert_eq!(detached[&(0, 0)].connection_group(0, 64, 0), Some(13));
        assert!(active.allows_connection_groups(0, 1, LIMIT));
        let mut replacement = Chunk::empty(1, 0);
        assert_eq!(
            replacement.set_connection_group(0, 64, 0, Some(21)),
            Some(true)
        );
        active.insert((1, 0), Arc::new(replacement));
        drop(detached);
        assert!(!active.allows_connection_groups(0, 1, LIMIT));
        drop(active);
        assert!(count.allows_replacement(0, 1, LIMIT));
    }

    /// A world full at its cap, here 2048 columns, takes a column it holds again, refuses a new
    /// one, and takes it once the server forgot another.
    #[test]
    fn a_viewed_world_holds_at_most_2048_columns() {
        let overworld = DimensionType::vanilla("minecraft:overworld");
        let format = Arc::new(ChunkFormat::new(&overworld, vec![0, 7, 9], 2));
        let mut viewed = ViewedWorld::new(
            "minecraft:the_nether".into(),
            format,
            None,
            World::new(BlockStateId(0), 1),
            2048,
        );
        for x in 0..2048 {
            viewed.apply(column(x), Default::default(), &[], 0).unwrap();
        }
        assert_eq!(viewed.world.chunks.len(), 2048);
        // A replacement is no new column.
        assert_eq!(viewed.apply(column(17), Default::default(), &[], 0), Ok(()));
        assert_eq!(viewed.world.chunks.len(), 2048);
        assert_eq!(
            viewed.apply(column(2048), Default::default(), &[], 0),
            Err(RouteError::TooManyColumns(2048))
        );
        assert!(!viewed.world.chunks.contains_key(&(2048, 0)));
        let forget = C::ForgetLevelChunk(play::ForgetLevelChunk { pos: [5, 0] });
        viewed.apply(forget, Default::default(), &[], 0).unwrap();
        assert_eq!(
            viewed.apply(column(2048), Default::default(), &[], 0),
            Ok(())
        );
        assert!(viewed.world.chunks.contains_key(&(2048, 0)));
        assert_eq!(viewed.world.chunks.len(), 2048);
    }

    /// A time update with no clocks moves the retained clock on by the game time since the last
    /// update at its rate, as the player's own level's does: 1000 at time 100, then an empty
    /// update at time 120, is 1020. A clock at half rate carries its half tick.
    #[test]
    fn an_empty_time_update_moves_the_retained_clock_on() {
        let overworld = DimensionType::vanilla("minecraft:overworld");
        let format = Arc::new(ChunkFormat::new(&overworld, vec![0, 7, 9], 2));
        let mut viewed = ViewedWorld::new(
            "minecraft:the_end".into(),
            format,
            Some(3),
            World::new(BlockStateId(0), 1),
            2048,
        );
        let time = |game_time, clocks| C::SetTime(play::SetTime { game_time, clocks });
        let clock = |clock, total_ticks, rate| ClockState {
            clock,
            total_ticks,
            partial_tick: 0.0,
            rate,
        };
        viewed
            .apply(
                time(100, vec![clock(0, 7, 1.0), clock(3, 1000, 1.0)]),
                Default::default(),
                &[],
                0,
            )
            .unwrap();
        assert_eq!(viewed.world.day_time, 1000);
        viewed
            .apply(time(120, Vec::new()), Default::default(), &[], 0)
            .unwrap();
        assert_eq!(viewed.world.day_time, 1020);
        assert_eq!(viewed.world.game_time, 120);
        viewed
            .apply(
                time(120, vec![clock(3, 2000, 0.5)]),
                Default::default(),
                &[],
                0,
            )
            .unwrap();
        viewed
            .apply(time(123, Vec::new()), Default::default(), &[], 0)
            .unwrap();
        assert_eq!(viewed.world.day_time, 2001);
        viewed
            .apply(time(124, Vec::new()), Default::default(), &[], 0)
            .unwrap();
        assert_eq!(viewed.world.day_time, 2002);
    }

    /// A server's times that would overflow are refused with a fixed reason and change nothing:
    /// the minimum game time then the maximum, and a retained clock at the maximum moved on a
    /// tick.
    #[test]
    fn a_time_past_the_range_of_a_long_is_refused() {
        let overworld = DimensionType::vanilla("minecraft:overworld");
        let format = Arc::new(ChunkFormat::new(&overworld, vec![0, 7, 9], 2));
        let fresh = || {
            ViewedWorld::new(
                "minecraft:the_end".into(),
                format.clone(),
                Some(3),
                World::new(BlockStateId(0), 1),
                2048,
            )
        };
        let mut viewed = fresh();
        let time = |game_time, clocks| C::SetTime(play::SetTime { game_time, clocks });
        viewed
            .apply(time(i64::MIN, Vec::new()), Default::default(), &[], 0)
            .unwrap();
        assert_eq!(
            viewed.apply(time(i64::MAX, Vec::new()), Default::default(), &[], 0),
            Err(RouteError::ClockOverflow)
        );
        assert_eq!(viewed.world.game_time, i64::MIN);
        assert_eq!(
            RouteError::ClockOverflow.to_string(),
            "sunrise:levels: a viewed level's time past its range"
        );
        let full = ClockState {
            clock: 3,
            total_ticks: i64::MAX,
            partial_tick: 0.0,
            rate: 1.0,
        };
        let mut viewed = fresh();
        viewed
            .apply(time(100, vec![full]), Default::default(), &[], 0)
            .unwrap();
        assert_eq!(viewed.world.day_time, i64::MAX);
        assert_eq!(
            viewed.apply(time(101, Vec::new()), Default::default(), &[], 0),
            Err(RouteError::ClockOverflow)
        );
        assert_eq!(
            (viewed.world.game_time, viewed.world.day_time),
            (100, i64::MAX)
        );
    }
}
