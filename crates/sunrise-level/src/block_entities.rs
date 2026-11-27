//! The world's block entities and the order they tick in: vanilla's
//! `Level.blockEntityTickers`, kept beside the block entity map so a tick
//! visits only the block entities that tick, in vanilla's order, instead of
//! scanning the whole map once per kind.
//!
//! The list follows `LevelChunk.updateBlockEntityTicker` and
//! `Level.tickBlockEntities`:
//! - a block entity whose block state has a ticker gets a slot at the end of
//!   the list (through `pendingBlockEntityTickers`, so a slot made during a
//!   tick first ticks on the next one);
//! - replacing the block entity at a position (`setBlockEntity`), or changing
//!   its block state to another state with a ticker, keeps its slot
//!   (`RebindableTickingBlockEntityWrapper.rebind`);
//! - removing it (`removeBlockEntity`), unloading its chunk
//!   (`clearAllBlockEntities`) or changing its state to one without a ticker
//!   (a jukebox losing its record) frees the slot; a later ticker there is
//!   appended again;
//! - a loaded chunk's block entities are added in the iteration order of its
//!   `Object2ObjectOpenHashMap<BlockPos, BlockEntity>`
//!   (`registerAllBlockEntitiesAfterLevelLoad`), filled in the order the chunk
//!   listed them.
//!
//! Which states tick is a table by state id, built by the game from the block
//! registry; whether a slot is made is decided when the slot is promoted at
//! the start of the next tick, from the block state then.
use crate::chunk::block_entity::{
    BlockEntity, ModBlockEntity, ModContainer, ModEnergyBuffer, ModFluidTanks,
};
use crate::fast_hash::{FxHashMap, FxHashSet};
use std::collections::hash_map::Entry;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use sunrise_mod_runtime::attachments::{Attachments, observer::AttachmentObserver};

mod carry;
mod contents_watches;
pub use carry::Carried;
pub use contents_watches::{ContentsObservers, ContentsWatches};

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

fn next_token() -> u64 {
    let mut token = NEXT_TOKEN.load(Ordering::Relaxed);
    loop {
        let next = token
            .checked_add(1)
            .expect("block entity token space exhausted");
        match NEXT_TOKEN.compare_exchange_weak(token, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return token,
            Err(current) => token = current,
        }
    }
}

/// Exclusive authority to return one instance taken from a block entity store.
/// Invalidated by removal, replacement, unloading, or the next ticker sweep.
/// Dropping it does not restore the instance; it cannot be cloned or forged.
pub struct BlockEntityLease {
    pos: Pos,
    token: u64,
    generation: u64,
}

/// Opaque snapshot of an in-flight `take` lease at one position, captured by a
/// transaction journal before a tentative removal and restored only on abort.
#[derive(Clone, Copy)]
pub struct TakenLeaseSnapshot {
    taken: Taken,
    generation: u64,
    is_mod: bool,
    shows_box: bool,
}

pub type Pos = (i32, i32, i32);

/// The type of a block entity taken out (`take`), kept until its lease ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TakenType {
    /// A chest-backed container by its own ID, which its block can name over (a barrel or a
    /// trapped chest).
    Container(&'static str),
    /// Any other vanilla block entity, by its type ID.
    Vanilla(&'static str),
    /// A mod's, which its block's factory may not name (an additional valid type): its own key,
    /// `None` for an unregistered type.
    Mod(Option<mod_api::BlockEntityTypeKey>),
}

/// The lease of a block entity taken out, and its type.
#[derive(Clone, Copy)]
struct Taken {
    token: u64,
    kind: TakenType,
}

/// What a block state's block entity does each tick (`BlockState.getTicker`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ticking {
    /// No ticker, and the block has no block entity that could have one.
    #[default]
    No,
    /// A ticker.
    Yes,
    /// The block's block entity, but this state has no ticker (a jukebox
    /// without a record): a state change to it frees the slot.
    Idle,
}

/// A level side's baked native tick states, indexed by state and attached type's numeric id.
pub type ModTickerStates = FxHashMap<(u32, u32), Ticking>;

struct LiveTicker {
    slot: u32,
    /// `BoundTickingBlockEntity.loggedInvalidBlockState`, scoped to the current
    /// occupant because a replacement can reuse the slot. Not an eligibility flag.
    warned_generation: Option<u64>,
}

#[derive(Default)]
pub struct BlockEntities {
    map: FxHashMap<Pos, BlockEntity>,
    /// Runtime contents subscriptions, retired with their observer identity.
    pub contents_watches: ContentsWatches,
    generations: FxHashMap<Pos, u64>,
    on_load: std::collections::VecDeque<(Pos, u64)>,
    data_load_enabled: bool,
    data_loads: std::collections::VecDeque<(Pos, u64)>,
    on_load_enabled: bool,
    attachments: FxHashMap<Pos, Attachments>,
    attachment_observer: Option<Arc<dyn AttachmentObserver<BlockEntityIdentity>>>,
    /// While a transaction is open (`defer_retirement`): the maps of block entities it
    /// removed or replaced, still bound to their index entries, by position and generation.
    /// The root commit retires them (`publish_retirement`); an abort puts back the ones it
    /// restores (`put_back_attachments`) first.
    retiring: Option<Vec<(Pos, u64, Attachments)>>,
    /// `Level.blockEntityTickers`: positions and the slot each was given.
    tickers: Arc<Vec<(Pos, u32)>>,
    /// `LevelChunk.tickersInLevel`: the live slot of each ticking position.
    live: FxHashMap<Pos, LiveTicker>,
    /// Positions that may need a slot, in the order vanilla would have made
    /// it (`pendingBlockEntityTickers`), each with a token; decided at the
    /// next promotion from the block state then.
    added: Vec<(Pos, u32)>,
    /// The token of each position's pending entry: an entry whose token is
    /// not here any more was cancelled.
    pending: FxHashMap<Pos, u32>,
    next_slot: u32,
    /// Whether a slot was freed since the list was last swept: otherwise
    /// every slot in it is still live, unless its block entity is `out`.
    freed: bool,
    /// Positions whose block entity `take` took and `put` has not brought
    /// back.
    out: FxHashMap<Pos, Taken>,
    /// Positions a loaded chunk gave a new block entity over an old one: at
    /// the next tick their slot stays only if their block state ticks
    /// (`updateBlockEntityTicker`: rebind, or remove without a ticker).
    replaced: FxHashSet<Pos>,
    /// `Ticking` by block state id; empty until the game builds it.
    states: Vec<Ticking>,
    mod_ticker_states: Option<Arc<ModTickerStates>>,
    /// The containers of mod blocks' block entities, by block state id;
    /// built with `states`.
    mod_containers: FxHashMap<u32, Arc<ModContainer>>,
    /// The fluid tanks of mod blocks' block entities, by block state id;
    /// built with `states`, like the containers.
    mod_fluids: FxHashMap<u32, Arc<ModFluidTanks>>,
    /// State-aware native factory ownership. Shared across level snapshots;
    /// an empty slice means the declarations have not yet been compiled.
    stateful_entities: Arc<[bool]>,
    /// The energy buffers of mod blocks' block entities, by block state id;
    /// built with `states`, like the containers.
    mod_energy: FxHashMap<u32, Arc<ModEnergyBuffer>>,
    /// The positions of the mod block entities (`BlockEntity::Mod`) by chunk,
    /// for the renderer: the mesher lists block entities by their template,
    /// which leaves out a mod block whose template has none. One taken out
    /// (`take`) stays listed until `put` or `remove`.
    mods: FxHashMap<(i32, i32), FxHashSet<Pos>>,
    /// Every block entity's position by chunk, taken-out ones (`take`) included until `put` or
    /// `remove`, so a column unloads without scanning the whole map.
    columns: FxHashMap<(i32, i32), FxHashSet<Pos>>,
    /// The positions of the block entities that draw a bounding box (structure and test
    /// instance blocks, `shows_box`), so a frame looks at them without scanning the map.
    /// Like `mods`, one taken out stays listed until `put` or `remove`.
    boxes: FxHashSet<Pos>,
    /// The signs a player's editor is on (`SignBlockEntity.playerWhoMayEdit`
    /// set), so the level's tick checks those alone instead of every sign.
    sign_editors: FxHashSet<Pos>,
    asleep: FxHashSet<Pos>,
    /// The sleeping positions that are hoppers, a subset of `asleep`: the item and
    /// vehicle scans look at these alone, never at another kind's sleep.
    sleeping_hoppers: FxHashSet<Pos>,
    /// The block entities moving pistons carry, by destination column and position:
    /// out of the live map until they land.
    carried: FxHashMap<(i32, i32), FxHashMap<Pos, carry::Carried>>,
    column_unload_entries: u64,
    /// The API snapshots a client's mod block entities are ticked with
    /// (`take_client_snapshots`), by position.
    client_snapshots: FxHashMap<Pos, ClientSnapshot>,
}

/// Split borrows used by a resident editor and its lazy before-image.
pub struct BlockEntityLoan<'a> {
    pub entity: &'a mut ModBlockEntity,
    pub identity: BlockEntityIdentity,
    pub attachments: Option<&'a Attachments>,
    pub contents_watches: &'a ContentsWatches,
}

/// A client copy of a mod block entity as its client tick shows it to mods: made once for each
/// revision of the copy (an update merged into it is a new one), not once a tick.
pub struct ClientSnapshot {
    /// The copy's revision the snapshot was made from.
    pub revision: crate::chunk::block_entity::ModRevision,
    /// The snapshot, or `None` when that revision could not be made into one.
    pub entity: Option<mod_api::ModBlockEntity>,
}

/// Storage-owned identity passed to attachment residency observers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct BlockEntityIdentity {
    pub position: [i32; 3],
    pub generation: u64,
}

fn identity(pos: Pos, generations: &FxHashMap<Pos, u64>) -> BlockEntityIdentity {
    BlockEntityIdentity {
        position: [pos.0, pos.1, pos.2],
        generation: generations[&pos],
    }
}

fn retire_attachments(
    maps: &mut FxHashMap<Pos, Attachments>,
    observer: Option<&Arc<dyn AttachmentObserver<BlockEntityIdentity>>>,
    generations: &FxHashMap<Pos, u64>,
    pos: &Pos,
) {
    if let Some(previous) = maps.remove(pos) {
        if let Some(observer) = observer {
            observer.unload(identity(*pos, generations));
        }
        drop(previous);
    }
}

impl Drop for BlockEntities {
    fn drop(&mut self) {
        self.publish_retirement();
        self.unload_attachments();
    }
}

/// A structure or test instance block: the block entities whose box the frame may draw.
fn shows_box(entity: &BlockEntity) -> bool {
    matches!(
        entity,
        BlockEntity::StructureBlock(_) | BlockEntity::TestInstanceBlock(_)
    )
}

fn chunk_of((x, _, z): Pos) -> (i32, i32) {
    (x >> 4, z >> 4)
}

/// Lists `pos` in its chunk's set of `by_chunk`.
fn list(by_chunk: &mut FxHashMap<(i32, i32), FxHashSet<Pos>>, pos: Pos) {
    by_chunk.entry(chunk_of(pos)).or_default().insert(pos);
}

/// Takes `pos` out of its chunk's set of `by_chunk` (`mods`, `columns`).
fn unlist(by_chunk: &mut FxHashMap<(i32, i32), FxHashSet<Pos>>, pos: Pos) {
    let chunk = chunk_of(pos);
    if let Some(set) = by_chunk.get_mut(&chunk) {
        set.remove(&pos);
        if set.is_empty() {
            by_chunk.remove(&chunk);
        }
    }
}

impl std::ops::Deref for BlockEntities {
    type Target = FxHashMap<Pos, BlockEntity>;
    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

impl<'a> IntoIterator for &'a BlockEntities {
    type Item = (&'a Pos, &'a BlockEntity);
    type IntoIter = std::collections::hash_map::Iter<'a, Pos, BlockEntity>;
    fn into_iter(self) -> Self::IntoIter {
        self.map.iter()
    }
}

/// `Vec3i.hashCode`.
fn java_hash((x, y, z): Pos) -> i32 {
    y.wrapping_add(z.wrapping_mul(31))
        .wrapping_mul(31)
        .wrapping_add(x)
}
/// `HashCommon.mix`.
fn mix(x: i32) -> usize {
    let h = (x as u32).wrapping_mul(0x9E37_79B9);
    (h ^ (h >> 16)) as usize
}

/// The order `values().forEach` visits a `new Object2ObjectOpenHashMap<>()`
/// keyed by these positions, put in this order (repeats keep their first
/// slot): open addressing with linear probing, a rehash to twice the size
/// past three quarters full, and iteration from the last slot down.
pub fn chunk_map_order(positions: impl IntoIterator<Item = Pos>) -> Vec<Pos> {
    // HashCommon.arraySize(16, .75f), and maxFill of each size.
    let mut slots: Vec<Option<Pos>> = vec![None; 32];
    let max_fill = |n: usize| (n * 3).div_ceil(4).min(n - 1);
    let mut size = 0;
    for p in positions {
        let mask = slots.len() - 1;
        let mut at = mix(java_hash(p)) & mask;
        let mut known = false;
        while let Some(k) = slots[at] {
            if k == p {
                known = true;
                break;
            }
            at = (at + 1) & mask;
        }
        if known {
            continue;
        }
        slots[at] = Some(p);
        // `if (size++ >= maxFill) rehash(arraySize(size + 1, f))`, which
        // puts the keys back from the last slot down.
        size += 1;
        if size > max_fill(slots.len()) {
            let n = ((size + 1) * 4).div_ceil(3).next_power_of_two();
            let old = std::mem::replace(&mut slots, vec![None; n]);
            for k in old.into_iter().rev().flatten() {
                let mut at = mix(java_hash(k)) & (n - 1);
                while slots[at].is_some() {
                    at = (at + 1) & (n - 1);
                }
                slots[at] = Some(k);
            }
        }
    }
    slots.into_iter().rev().flatten().collect()
}

impl BlockEntities {
    /// Gameplay supplies its subscription decision; storage knows no hook identifiers.
    pub fn set_on_load_enabled(&mut self, enabled: bool) {
        self.on_load_enabled = enabled;
        if !enabled {
            self.on_load.clear();
        }
    }
    /// Queue chunk-payload notices only when gameplay has a subscriber.
    pub fn set_data_load_enabled(&mut self, enabled: bool) {
        self.data_load_enabled = enabled;
        if !enabled {
            self.data_loads.clear();
        }
    }
    /// Whether a promoted chunk has a pending saved-payload notice.
    pub fn has_pending_data_load(&self) -> bool {
        !self.data_loads.is_empty()
    }
    /// Take the next queued identity. Gameplay verifies it still belongs to the loaded instance.
    pub fn next_data_load(&mut self) -> Option<(Pos, u64)> {
        self.data_loads.pop_front()
    }
    /// Whether any attachment notices remain, including possibly retired identities.
    pub fn has_pending_on_load(&self) -> bool {
        !self.on_load.is_empty()
    }
    /// Number of queued notices, including retired entries, at a drain boundary.
    pub fn pending_on_load_len(&self) -> usize {
        self.on_load.len()
    }
    /// Take a live notice from a bounded prefix, counting stale entries too.
    pub fn next_on_load_in_batch(&mut self, remaining: &mut usize) -> Option<(Pos, u64)> {
        while *remaining > 0 {
            *remaining -= 1;
            let (pos, generation) = self.on_load.pop_front()?;
            if self.generation(&pos) == Some(generation)
                && matches!(self.map.get(&pos), Some(BlockEntity::Mod(_)))
            {
                return Some((pos, generation));
            }
        }
        None
    }
    /// Take the next newly attached mod identity. Removed or replaced entries
    /// are stale and skipped; payload rewrites retain the queued generation.
    pub fn next_on_load(&mut self) -> Option<(Pos, u64)> {
        let mut remaining = self.on_load.len();
        self.next_on_load_in_batch(&mut remaining)
    }

    /// Runtime identity, retained while an entity is temporarily taken for its tick.
    pub fn generation(&self, pos: &Pos) -> Option<u64> {
        self.generations.get(pos).copied()
    }
    /// The live identity, including an instance temporarily taken for its tick.
    pub fn identity(&self, pos: Pos) -> Option<BlockEntityIdentity> {
        self.generation(&pos).map(|generation| BlockEntityIdentity {
            position: [pos.0, pos.1, pos.2],
            generation,
        })
    }
    /// Restore the identity captured with an existing entity by a transaction.
    pub fn restore_generation(&mut self, pos: Pos, generation: u64) {
        assert!(self.map.contains_key(&pos));
        if self.generation(&pos) != Some(generation) {
            self.contents_watches.remove(pos);
            retire_attachments(
                &mut self.attachments,
                self.attachment_observer.as_ref(),
                &self.generations,
                &pos,
            );
            self.generations.insert(pos, generation);
        }
    }
    fn new_generation(&mut self, pos: Pos) {
        self.generations.insert(pos, next_token());
    }
    /// Bind this dimension's authoritative maps, including saved entries not yet read.
    pub fn observe_attachments(
        &mut self,
        observer: Arc<dyn AttachmentObserver<BlockEntityIdentity>>,
    ) {
        self.unload_attachments();
        for (pos, map) in &mut self.attachments {
            if map.has_index_entries() || map.has_index_binding() {
                let _ = observer.bind(identity(*pos, &self.generations), map);
            }
        }
        self.attachment_observer = Some(observer);
    }

    fn unload_attachments(&self) {
        if let Some(observer) = &self.attachment_observer {
            for pos in self.attachments.keys() {
                observer.unload(identity(*pos, &self.generations));
            }
        }
    }

    /// The client tick snapshots, lent for one pass over the tickers: give them back with
    /// `keep_client_snapshots`.
    pub fn take_client_snapshots(&mut self) -> FxHashMap<Pos, ClientSnapshot> {
        std::mem::take(&mut self.client_snapshots)
    }
    /// Takes back what `take_client_snapshots` lent, without the positions that no longer tick.
    pub fn keep_client_snapshots(&mut self, mut snapshots: FxHashMap<Pos, ClientSnapshot>) {
        snapshots.retain(|pos, _| self.live.contains_key(pos));
        self.client_snapshots = snapshots;
    }
    /// Whether the ticking table has been built.
    pub fn has_states(&self) -> bool {
        !self.states.is_empty()
    }
    pub fn set_states(&mut self, states: Vec<Ticking>) {
        self.states = states;
    }
    pub fn ticking(&self, state: u32) -> Ticking {
        self.states.get(state as usize).copied().unwrap_or_default()
    }
    /// Install the typed state table prepared with the level's block registry.
    pub fn set_mod_ticker_states(&mut self, states: Arc<ModTickerStates>) {
        self.mod_ticker_states = Some(states);
    }
    /// The native ticker choice for this actual type (not the block's factory type).
    pub fn mod_ticking(&self, state: u32, kind: u32) -> Ticking {
        self.mod_ticker_states
            .as_ref()
            .and_then(|states| states.get(&(state, kind)))
            .copied()
            .unwrap_or_default()
    }
    fn ticker_at(&self, pos: &Pos, state: u32) -> Ticking {
        if self.mod_ticker_states.is_some()
            && let Some(BlockEntity::Mod(entity)) = self.map.get(pos)
        {
            return entity
                .numeric_id()
                .map_or(Ticking::No, |kind| self.mod_ticking(state, kind));
        }
        self.ticking(state)
    }
    pub fn set_mod_containers(&mut self, containers: FxHashMap<u32, Arc<ModContainer>>) {
        self.mod_containers = containers;
    }
    /// The container of a mod block's block entity in block state `state`.
    pub fn mod_container(&self, state: u32) -> Option<&Arc<ModContainer>> {
        self.mod_containers.get(&state)
    }
    pub fn set_mod_fluids(&mut self, fluids: FxHashMap<u32, Arc<ModFluidTanks>>) {
        self.mod_fluids = fluids;
    }
    /// The fluid tanks of a mod block's block entity in block state `state`.
    pub fn mod_fluids(&self, state: u32) -> Option<&Arc<ModFluidTanks>> {
        self.mod_fluids.get(&state)
    }
    pub fn set_stateful_entities(&mut self, states: Vec<bool>) {
        self.stateful_entities = states.into();
    }
    pub fn has_stateful_entity_table(&self) -> bool {
        !self.stateful_entities.is_empty()
    }
    pub fn stateful_entities(&self) -> Arc<[bool]> {
        self.stateful_entities.clone()
    }
    pub fn stateful_entity(&self, state: u32) -> bool {
        self.stateful_entities
            .get(state as usize)
            .copied()
            .unwrap_or(false)
    }
    pub fn set_mod_energy(&mut self, energy: FxHashMap<u32, Arc<ModEnergyBuffer>>) {
        self.mod_energy = energy;
    }
    /// The energy buffer of a mod block's block entity in block state `state`.
    pub fn mod_energy(&self, state: u32) -> Option<&Arc<ModEnergyBuffer>> {
        self.mod_energy.get(&state)
    }

    fn next(&mut self) -> u32 {
        let n = self.next_slot;
        self.next_slot = n.wrapping_add(1);
        n
    }
    /// A ticker may be made for `pos` now (`updateBlockEntityTicker`).
    fn add(&mut self, pos: Pos) {
        let token = self.next();
        self.pending.insert(pos, token);
        self.added.push((pos, token));
    }
    /// `removeBlockEntityTicker`: no slot, made or pending.
    fn cancel(&mut self, pos: &Pos) {
        self.wake_at(pos);
        self.freed |= self.live.remove(pos).is_some();
        self.pending.remove(pos);
    }

    /// Attachment map for a live block entity.
    pub fn attachments_mut(
        &mut self,
        pos: Pos,
    ) -> Option<&mut sunrise_mod_runtime::attachments::Attachments> {
        // Identity survives a temporary take and retires on removal or an
        // abandoned-lease sweep, exactly like attachment ownership.
        if !self.generations.contains_key(&pos) {
            return None;
        }
        let map = self.attachments.entry(pos).or_default();
        if let Some(observer) = &self.attachment_observer
            && !map.has_index_binding()
        {
            let _ = observer.bind(identity(pos, &self.generations), map);
        }
        Some(map)
    }
    /// The attachment map of the block entity at `pos`, if it has one.
    pub fn attachments(&self, pos: Pos) -> Option<&sunrise_mod_runtime::attachments::Attachments> {
        self.attachments.get(&pos)
    }
    /// Puts the attachment map of the live block entity at `pos` back as `data`, or removes it
    /// where it had none: a transaction's abort.
    pub fn set_attachments(
        &mut self,
        pos: Pos,
        data: Option<sunrise_mod_runtime::attachments::Attachments>,
    ) {
        match data {
            Some(data) => self.restore_attachments(pos, data),
            None => {
                retire_attachments(
                    &mut self.attachments,
                    self.attachment_observer.as_ref(),
                    &self.generations,
                    &pos,
                );
            }
        }
    }
    /// Retires the map of the block entity leaving `pos`: at once, or, while a transaction
    /// defers retirement, kept bound until it closes.
    fn retire(&mut self, pos: Pos) {
        self.contents_watches.remove(pos);
        if let Some(retiring) = &mut self.retiring {
            if let (Some(map), Some(&generation)) =
                (self.attachments.remove(&pos), self.generations.get(&pos))
            {
                retiring.push((pos, generation, map));
            }
            return;
        }
        retire_attachments(
            &mut self.attachments,
            self.attachment_observer.as_ref(),
            &self.generations,
            &pos,
        );
    }
    /// From now until `publish_retirement`, a block entity that goes keeps its map bound to
    /// its index entries: the root transaction opened, and only its commit publishes what it
    /// removed.
    pub fn defer_retirement(&mut self) {
        self.retiring.get_or_insert_with(Vec::new);
    }
    /// Whether a transaction is open: between `defer_retirement` and `publish_retirement`.
    pub fn defers_retirement(&self) -> bool {
        self.retiring.is_some()
    }
    /// Retires every map kept since `defer_retirement`, and stops deferring: the root
    /// transaction closed. What an abort did not put back goes: those of block entities a
    /// committed transaction removed, and those of block entities an aborted one made.
    pub fn publish_retirement(&mut self) {
        for (pos, generation, map) in self.retiring.take().into_iter().flatten() {
            if let Some(observer) = &self.attachment_observer {
                observer.unload(BlockEntityIdentity {
                    position: [pos.0, pos.1, pos.2],
                    generation,
                });
            }
            drop(map);
        }
    }
    /// Gives the block entity of `generation` that stands at `pos` again the map it went with
    /// in this transaction, as it was then, still bound: a removed block entity a later write
    /// in the same transaction put back.
    pub fn reattach_retired(&mut self, pos: Pos, generation: u64) {
        if self.generation(&pos) != Some(generation) || self.attachments.contains_key(&pos) {
            return;
        }
        let Some(retiring) = &mut self.retiring else {
            return;
        };
        if let Some(at) = retiring
            .iter()
            .position(|(p, g, _)| *p == pos && *g == generation)
        {
            let (_, _, map) = retiring.swap_remove(at);
            self.attachments.insert(pos, map);
        }
    }
    /// Puts the attachment map of the block entity at `pos` back as a transaction kept it,
    /// `data`, for the block entity of `generation` that stands there again: the very map it
    /// had, still bound, where it went in this transaction, so its index entries never left.
    pub fn put_back_attachments(
        &mut self,
        pos: Pos,
        generation: Option<u64>,
        data: Option<sunrise_mod_runtime::attachments::Attachments>,
    ) {
        let kept = self.retiring.as_mut().and_then(|retiring| {
            let at = retiring
                .iter()
                .position(|(p, g, _)| *p == pos && Some(*g) == generation)?;
            Some(retiring.swap_remove(at).2)
        });
        match (kept, data) {
            (Some(mut map), Some(data)) if self.generation(&pos) == generation => {
                map.replace_contents(data);
                if let Some(current) = self.attachments.insert(pos, map) {
                    // A map the transaction made for the same block entity after it went:
                    // never bound apart from this one, which holds the index entries.
                    drop(current);
                }
            }
            (Some(map), _) => {
                if let (Some(observer), Some(generation)) = (&self.attachment_observer, generation)
                {
                    observer.unload(BlockEntityIdentity {
                        position: [pos.0, pos.1, pos.2],
                        generation,
                    });
                }
                drop(map);
                self.set_attachments(pos, None);
            }
            (None, data) => self.set_attachments(pos, data),
        }
    }
    /// Attachment maps in a loaded chunk, for a save snapshot.
    pub fn chunk_attachments(
        &self,
        x: i32,
        z: i32,
    ) -> Vec<(Pos, sunrise_mod_runtime::attachments::Attachments)> {
        self.attachments
            .iter()
            .filter(|(p, _)| p.0 >> 4 == x && p.2 >> 4 == z)
            .map(|(p, data)| (*p, data.clone()))
            .collect()
    }
    /// Restore a loaded block entity's attachment map.
    pub fn restore_attachments(
        &mut self,
        pos: Pos,
        data: sunrise_mod_runtime::attachments::Attachments,
    ) {
        if self.map.contains_key(&pos) || self.out.contains_key(&pos) {
            let map = self.attachments.entry(pos).or_default();
            map.replace_contents(data);
            if let Some(observer) = &self.attachment_observer
                && map.has_index_entries()
                && !map.has_index_binding()
            {
                let _ = observer.bind(identity(pos, &self.generations), map);
            }
        }
    }
    /// Whether any mod block entity is in the map: until one is, a block change
    /// never looks one up (`LevelAccess::has_mod_block_entity`).
    pub fn has_mods(&self) -> bool {
        !self.mods.is_empty()
    }
    /// The structure and test instance blocks (a frame draws their boxes), in no order.
    pub fn boxes(&self) -> impl Iterator<Item = (&Pos, &BlockEntity)> {
        self.boxes
            .iter()
            .filter_map(|pos| Some((pos, self.map.get(pos)?)))
    }
    /// Keeps `pos` listed in `boxes` exactly while `entity` shows a box.
    fn note_box(&mut self, pos: Pos, entity: Option<&BlockEntity>) {
        if entity.is_some_and(shows_box) {
            self.boxes.insert(pos);
        } else {
            self.boxes.remove(&pos);
        }
    }
    /// The mod block entities within `radius` blocks of `x`, `z` (by chunk, so
    /// far ones are skipped whole; the caller checks each distance).
    pub fn mods_near(&self, x: f64, z: f64, radius: f64) -> impl Iterator<Item = &Pos> {
        let far = move |c: i32, at: f64| {
            let (lo, hi) = (f64::from(c * 16), f64::from(c * 16 + 16));
            at < lo - radius || at > hi + radius
        };
        self.mods
            .iter()
            .filter(move |((cx, cz), _)| !far(*cx, x) && !far(*cz, z))
            .flat_map(|(_, set)| set)
    }
    /// The mod block entities of chunk `chunk`, in no order; one taken out is
    /// listed too.
    pub fn mods_in(&self, chunk: (i32, i32)) -> impl Iterator<Item = &Pos> {
        self.mods.get(&chunk).into_iter().flatten()
    }
    /// The type of the block entity taken out at `pos`; `None` when none is out.
    pub fn taken_type(&self, pos: &Pos) -> Option<TakenType> {
        self.out.get(pos).map(|taken| taken.kind)
    }
    /// Every block entity of chunk `chunk`, in no order; one taken out is listed too.
    pub fn in_chunk(&self, chunk: (i32, i32)) -> impl Iterator<Item = &Pos> {
        self.columns.get(&chunk).into_iter().flatten()
    }
    /// `SignBlockEntity.playerWhoMayEdit` of the sign at `pos`.
    pub fn sign_editor(&self, pos: &Pos) -> Option<u128> {
        match self.map.get(pos) {
            Some(BlockEntity::Sign(sign)) => sign.allowed_editor,
            _ => None,
        }
    }
    /// `SignBlockEntity.setAllowedPlayerEditor`: the one player whose editor
    /// is on the sign at `pos`, or `None` for nobody's.
    pub fn set_sign_editor(&mut self, pos: Pos, who: Option<u128>) {
        let Some(BlockEntity::Sign(sign)) = self.map.get_mut(&pos) else {
            return;
        };
        sign.allowed_editor = who;
        if who.is_some() {
            self.sign_editors.insert(pos);
        } else {
            self.sign_editors.remove(&pos);
        }
    }
    /// The positions of the signs a player's editor is on, in no order.
    pub fn sign_editors(&self) -> impl Iterator<Item = &Pos> {
        self.sign_editors.iter()
    }
    /// Drops `pos` from the signs an editor is on, for a sign that is gone or
    /// no longer holds one. `set_sign_editor` keeps the list itself.
    pub fn forget_sign_editor(&mut self, pos: Pos) {
        self.sign_editors.remove(&pos);
    }
    /// Keeps `pos` listed in `sign_editors` exactly while `entity` holds an
    /// editor, as `note_box` does for the boxes.
    fn note_sign_editor(&mut self, pos: Pos, entity: &BlockEntity) {
        if matches!(entity, BlockEntity::Sign(sign) if sign.allowed_editor.is_some()) {
            self.sign_editors.insert(pos);
        } else {
            self.sign_editors.remove(&pos);
        }
    }
    /// `LevelChunk.getBlockEntity`: the block entity at `pos`, if any.
    pub fn get(&self, pos: &Pos) -> Option<&BlockEntity> {
        self.map.get(pos)
    }
    pub fn get_mut(&mut self, pos: &Pos) -> Option<&mut BlockEntity> {
        self.get_mut_if(pos, |_| true)
    }
    /// Wake and borrow a resident row only when the read-only predicate accepts it.
    pub fn get_mut_if(
        &mut self,
        pos: &Pos,
        accepts: impl FnOnce(&BlockEntity) -> bool,
    ) -> Option<&mut BlockEntity> {
        if !accepts(self.map.get(pos)?) {
            return None;
        }
        self.wake_at(pos);
        self.wake_below(pos);
        self.map.get_mut(pos)
    }
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&Pos, &mut BlockEntity)> {
        self.clear_asleep();
        self.map.iter_mut()
    }
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut BlockEntity> {
        self.clear_asleep();
        self.map.values_mut()
    }
    /// Lends the resident instance and the indexes needed for a journal before-image.
    /// Unlike a mutable simulation lookup, a read-only loan does not wake the row.
    pub fn loan_mod_entity(&mut self, pos: Pos, generation: u64) -> Option<BlockEntityLoan<'_>> {
        let identity = self.identity(pos)?;
        if identity.generation != generation {
            return None;
        }
        let BlockEntity::Mod(entity) = self.map.get_mut(&pos)? else {
            return None;
        };
        Some(BlockEntityLoan {
            entity,
            identity,
            attachments: self.attachments.get(&pos),
            contents_watches: &self.contents_watches,
        })
    }

    pub fn sleep(&mut self, pos: Pos) {
        if self.map.contains_key(&pos) {
            self.asleep.insert(pos);
        }
    }
    /// Sleeps the hopper at `pos`: the scans that wake it only look at hoppers,
    /// so a furnace's or campfire's sleep costs them nothing.
    pub fn sleep_hopper(&mut self, pos: Pos) {
        if self.map.contains_key(&pos) {
            self.asleep.insert(pos);
            self.sleeping_hoppers.insert(pos);
        }
    }
    pub fn is_asleep(&self, pos: &Pos) -> bool {
        self.asleep.contains(pos)
    }
    pub fn is_hopper_asleep(&self, pos: &Pos) -> bool {
        self.sleeping_hoppers.contains(pos)
    }
    /// Whether any block entity sleeps.
    pub fn any_asleep(&self) -> bool {
        !self.asleep.is_empty()
    }
    /// Whether any hopper sleeps (the gate for the item and vehicle scans).
    pub fn any_hopper_asleep(&self) -> bool {
        !self.sleeping_hoppers.is_empty()
    }
    /// Wakes the block entity at `pos`, if it sleeps.
    pub fn wake(&mut self, pos: &Pos) {
        self.wake_at(pos);
    }
    /// Wakes the hopper at `pos` only if it sleeps: a scan walking item or
    /// vehicle boxes leaves another kind's sleeping block entity alone.
    pub fn wake_hopper(&mut self, pos: &Pos) {
        if self.sleeping_hoppers.remove(pos) {
            self.asleep.remove(pos);
        }
    }
    /// Wakes the block entity directly below `pos`: a hopper sleeping there
    /// reads the container above it, so a change up here is a change for it.
    pub fn wake_below(&mut self, pos: &Pos) {
        self.wake_at(&(pos.0, pos.1 - 1, pos.2));
    }
    fn wake_at(&mut self, pos: &Pos) {
        self.asleep.remove(pos);
        self.sleeping_hoppers.remove(pos);
    }
    fn clear_asleep(&mut self) {
        self.asleep.clear();
        self.sleeping_hoppers.clear();
    }
    fn replace_taken(&mut self, pos: Pos) {
        if self.out.remove(&pos).is_some() {
            self.replaced.insert(pos);
        }
    }
    /// `LevelChunk.setBlockEntity` (+ `addAndRegisterBlockEntity`): a new
    /// block entity may get a slot; one replacing another keeps its slot.
    /// One of the same type rewrites it in place (a jukebox taking a disc,
    /// `/data merge block`), where vanilla keeps the object, so its
    /// attachments stay; any other replacement starts them over.
    pub fn insert(&mut self, pos: Pos, entity: BlockEntity) -> Option<BlockEntity> {
        self.insert_impl(pos, entity, false)
    }
    /// Attach a different instance, even when its type matches the old instance.
    /// Payload-only writes use `insert` and retain their identity instead.
    pub fn replace(&mut self, pos: Pos, entity: BlockEntity) -> Option<BlockEntity> {
        self.insert_impl(pos, entity, true)
    }
    fn insert_impl(
        &mut self,
        pos: Pos,
        entity: BlockEntity,
        replace_identity: bool,
    ) -> Option<BlockEntity> {
        let new_is_mod = matches!(entity, BlockEntity::Mod(_));
        let same_type = |old: &BlockEntity| match (old, &entity) {
            (BlockEntity::Mod(old), BlockEntity::Mod(new)) => old.id() == new.id(),
            (old, new) => std::mem::discriminant(old) == std::mem::discriminant(new),
        };
        let new_identity = replace_identity || !self.map.get(&pos).is_some_and(same_type);
        if new_identity {
            self.retire(pos);
            self.new_generation(pos);
        }
        self.replace_taken(pos);
        let new_box = shows_box(&entity);
        self.note_sign_editor(pos, &entity);
        self.wake_at(&pos);
        self.wake_below(&pos);
        let old = self.map.insert(pos, entity);
        list(&mut self.columns, pos);
        if self.on_load_enabled && new_identity && new_is_mod {
            self.on_load.push_back((pos, self.generations[&pos]));
        }
        if old.is_none()
            || new_identity && !self.live.contains_key(&pos) && !self.pending.contains_key(&pos)
        {
            self.add(pos);
        }
        if new_identity && old.is_some() {
            self.replaced.insert(pos);
        }
        if new_box {
            self.boxes.insert(pos);
        } else {
            self.boxes.remove(&pos);
        }
        if new_is_mod {
            list(&mut self.mods, pos);
        } else {
            unlist(&mut self.mods, pos);
        }
        old
    }
    /// The block entity at `pos`, made by `make` when there is none.
    pub fn get_or_insert_with(
        &mut self,
        pos: Pos,
        make: impl FnOnce() -> BlockEntity,
    ) -> &mut BlockEntity {
        let absent = !self.map.contains_key(&pos);
        if absent {
            self.replace_taken(pos);
            self.retire(pos);
            self.new_generation(pos);
            unlist(&mut self.mods, pos);
            list(&mut self.columns, pos);
            self.boxes.remove(&pos);
            self.add(pos);
        }
        self.wake_at(&pos);
        let entity = self.map.entry(pos).or_insert_with(make);
        if absent && matches!(entity, BlockEntity::Mod(_)) {
            if self.on_load_enabled {
                self.on_load.push_back((pos, self.generations[&pos]));
            }
            list(&mut self.mods, pos);
        }
        if absent && shows_box(entity) {
            self.boxes.insert(pos);
        }
        entity
    }
    /// `LevelChunk.removeBlockEntity`: the block entity and its slot.
    pub fn remove(&mut self, pos: &Pos) -> Option<BlockEntity> {
        self.out.remove(pos);
        self.retire(*pos);
        self.generations.remove(pos);
        self.wake_at(pos);
        self.cancel(pos);
        let removed = self.map.remove(pos);
        unlist(&mut self.mods, *pos);
        unlist(&mut self.columns, *pos);
        self.boxes.remove(pos);
        self.sign_editors.remove(pos);
        removed
    }
    /// The block entity at `pos`, out of the map for a while: its slot stays
    /// for `put` to bring it back (a tick working on it).
    pub fn take(&mut self, pos: &Pos) -> Option<(BlockEntityLease, BlockEntity)> {
        self.wake_at(pos);
        let entity = self.map.remove(pos)?;
        let token = next_token();
        let kind = match &entity {
            BlockEntity::Mod(entity) => TakenType::Mod(entity.kind()),
            BlockEntity::Chest(_) => TakenType::Container(entity.vanilla_id().unwrap_or_default()),
            other => TakenType::Vanilla(other.vanilla_id().unwrap_or_default()),
        };
        self.out.insert(*pos, Taken { token, kind });
        let generation = self.generations[pos];
        Some((
            BlockEntityLease {
                pos: *pos,
                token,
                generation,
            },
            entity,
        ))
    }
    /// Re-key a live lease when an intentional state change keeps the instance.
    /// Ordinary removal still revokes it; a retired or foreign lease cannot re-key.
    pub fn rekey(&mut self, pos: &Pos, lease: &mut BlockEntityLease) -> bool {
        if lease.pos != *pos || self.generation(pos) != Some(lease.generation) {
            return false;
        }
        match self.out.get_mut(pos) {
            Some(taken) if taken.token == lease.token => {
                lease.token = next_token();
                taken.token = lease.token;
                true
            }
            _ => false,
        }
    }
    /// Snapshot the active in-flight lease at `pos`, if an owner holds it via `take`.
    pub fn taken_lease_snapshot(&self, pos: &Pos) -> Option<TakenLeaseSnapshot> {
        let taken = *self.out.get(pos)?;
        let generation = *self.generations.get(pos)?;
        let is_mod = self
            .mods
            .get(&chunk_of(*pos))
            .is_some_and(|set| set.contains(pos));
        let shows_box = self.boxes.contains(pos);
        Some(TakenLeaseSnapshot {
            taken,
            generation,
            is_mod,
            shows_box,
        })
    }
    /// Restore an in-flight lease whose tentative removal was aborted, so the
    /// original owner's `put` succeeds with its original identity and attachments.
    pub fn restore_taken_lease(&mut self, pos: Pos, snapshot: TakenLeaseSnapshot) {
        assert!(!self.map.contains_key(&pos));
        self.replaced.remove(&pos);
        self.generations.insert(pos, snapshot.generation);
        self.out.insert(pos, snapshot.taken);
        if snapshot.is_mod {
            self.mods.entry(chunk_of(pos)).or_default().insert(pos);
        }
        if snapshot.shows_box {
            self.boxes.insert(pos);
        }
    }
    /// Return the taken instance in its slot; false if its lease was retired.
    pub fn put(&mut self, lease: BlockEntityLease, entity: BlockEntity) -> bool {
        let pos = lease.pos;
        if self.out.get(&pos).map(|taken| taken.token) != Some(lease.token)
            || self.generation(&pos) != Some(lease.generation)
        {
            return false;
        }
        self.out.remove(&pos);
        self.wake_at(&pos);
        if matches!(entity, BlockEntity::Mod(_)) {
            list(&mut self.mods, pos);
        }
        self.note_box(pos, Some(&entity));
        self.note_sign_editor(pos, &entity);
        if self.map.insert(pos, entity).is_none()
            && !self.live.contains_key(&pos)
            && !self.pending.contains_key(&pos)
        {
            self.add(pos);
        }
        true
    }
    fn retire_taken(&mut self, mut gone: impl FnMut(&Pos) -> bool) {
        let mut out = std::mem::take(&mut self.out);
        out.retain(|pos, _| {
            if gone(pos) {
                self.remove(pos);
                false
            } else {
                true
            }
        });
        self.out = out;
    }
    /// [`Self::retain`] for unloading chunks: removes the block entities at
    /// the positions `gone` names and changes none it keeps, so those that
    /// sleep stay asleep (a moving player unloads chunks most ticks).
    pub fn unload(&mut self, mut gone: impl FnMut(&Pos) -> bool) {
        self.carried.retain(|_, payloads| {
            payloads.retain(|p, _| !gone(p));
            !payloads.is_empty()
        });
        let asleep = std::mem::take(&mut self.asleep);
        let hoppers = std::mem::take(&mut self.sleeping_hoppers);
        self.retain(|p, _| !gone(p));
        self.retire_taken(gone);
        self.asleep = asleep;
        self.sleeping_hoppers = hoppers;
        let map = &self.map;
        self.asleep.retain(|p| map.contains_key(p));
        self.sleeping_hoppers.retain(|p| map.contains_key(p));
    }
    /// [`Self::unload`] of chunk `chunk`'s block entities: a column that goes, or comes again.
    pub fn unload_column(&mut self, chunk: (i32, i32)) {
        self.unload_column_where(chunk, |_| true);
    }
    /// [`Self::unload`] of the block entities of chunk `chunk` that `gone` names. It visits that
    /// chunk's alone, so a frame that loads or unloads many columns costs what they hold, not
    /// what the whole map holds.
    pub fn unload_column_where(&mut self, chunk: (i32, i32), mut gone: impl FnMut(&Pos) -> bool) {
        if let Entry::Occupied(mut column) = self.carried.entry(chunk) {
            let payloads = column.get_mut();
            self.column_unload_entries += payloads.len() as u64;
            payloads.retain(|p, _| !gone(p));
            if column.get().is_empty() {
                column.remove();
            }
        }
        let Some(listed) = self.columns.get(&chunk) else {
            return;
        };
        self.column_unload_entries += listed.len() as u64;
        let doomed: Vec<Pos> = listed.iter().filter(|p| gone(p)).copied().collect();
        for pos in doomed {
            if self.map.remove(&pos).is_some() {
                self.unloaded(pos);
            } else {
                // Taken out: its lease goes, as `retire_taken` drops it.
                self.remove(&pos);
            }
        }
    }
    /// Number of live/taken positions and carried payloads visited by indexed
    /// unloads and replacements since the last read.
    pub fn take_column_unload_entries(&mut self) -> u64 {
        std::mem::take(&mut self.column_unload_entries)
    }

    /// Removes the block entities `keep` refuses, and their slots
    /// (`clearAllBlockEntities` of unloaded chunks).
    pub fn retain(&mut self, mut keep: impl FnMut(&Pos, &mut BlockEntity) -> bool) {
        self.clear_asleep();
        let mut gone = Vec::new();
        self.map.retain(|p, e| {
            let kept = keep(p, e);
            if !kept {
                gone.push(*p);
            }
            kept
        });
        for pos in gone {
            self.unloaded(pos);
        }
    }
    /// Forgets everything kept for `pos`, whose block entity just left the map unloaded: its
    /// attachments retire at once, slots and lists drop it.
    fn unloaded(&mut self, pos: Pos) {
        self.contents_watches.remove(pos);
        retire_attachments(
            &mut self.attachments,
            self.attachment_observer.as_ref(),
            &self.generations,
            &pos,
        );
        self.generations.remove(&pos);
        self.freed |= self.live.remove(&pos).is_some();
        self.pending.remove(&pos);
        self.wake_at(&pos);
        unlist(&mut self.mods, pos);
        unlist(&mut self.columns, pos);
        self.boxes.remove(&pos);
        self.sign_editors.remove(&pos);
    }
    pub fn clear(&mut self) {
        self.contents_watches = ContentsWatches::default();
        self.unload_attachments();
        self.generations.clear();
        self.on_load.clear();
        self.data_loads.clear();
        self.attachments.clear();
        self.map.clear();
        self.mods.clear();
        self.columns.clear();
        self.boxes.clear();
        self.sign_editors.clear();
        self.clear_asleep();
        self.tickers = Arc::default();
        self.live.clear();
        self.added.clear();
        self.pending.clear();
        self.out.clear();
        self.replaced.clear();
        self.carried.clear();
        self.client_snapshots.clear();
        self.freed = false;
    }
    /// A loaded or generated chunk's block entities, in the order the chunk
    /// lists them; they tick in its block entity map's order
    /// (`registerAllBlockEntitiesAfterLevelLoad`). One replacing an entity
    /// already at its position (`LevelChunk.setBlockEntity`) drops that one;
    /// its slot is decided by the block state at the next tick.
    pub fn load_chunk(&mut self, entities: Vec<(Pos, BlockEntity)>) {
        let _load = if entities.is_empty() {
            None
        } else {
            crate::spans::note_block_entity_loads(entities.len());
            crate::spans::raw_flight_guard("load_block_entities")
        };
        let order = chunk_map_order(entities.iter().map(|(p, _)| *p));
        for (p, e) in entities {
            self.contents_watches.remove(p);
            self.replace_taken(p);
            retire_attachments(
                &mut self.attachments,
                self.attachment_observer.as_ref(),
                &self.generations,
                &p,
            );
            self.new_generation(p);
            self.wake_at(&p);
            let new_is_mod = matches!(e, BlockEntity::Mod(_));
            self.note_box(p, Some(&e));
            self.note_sign_editor(p, &e);
            let old = self.map.insert(p, e);
            list(&mut self.columns, p);
            if old.is_some() {
                self.replaced.insert(p);
            }
            if new_is_mod {
                list(&mut self.mods, p);
                if self.data_load_enabled {
                    self.data_loads.push_back((p, self.generations[&p]));
                }
            } else {
                unlist(&mut self.mods, p);
            }
        }
        for p in order {
            if self.on_load_enabled && matches!(self.map.get(&p), Some(BlockEntity::Mod(_))) {
                self.on_load.push_back((p, self.generations[&p]));
            }
            self.add(p);
        }
    }
    /// `LevelChunk.setBlockState` on a block entity's position, from state
    /// `old` to `new`: a ticker made, kept or dropped.
    pub fn state_changed(&mut self, pos: Pos, old: u32, new: u32) {
        self.wake_at(&pos);
        self.wake_below(&pos);
        if self.states.is_empty() || !self.map.contains_key(&pos) {
            return;
        }
        match self.ticker_at(&pos, new) {
            // A new ticker, unless one was made or is pending (rebound).
            Ticking::Yes => {
                if self.ticker_at(&pos, old) != Ticking::Yes && !self.live.contains_key(&pos) {
                    self.add(pos);
                }
            }
            Ticking::Idle => self.cancel(&pos),
            // Another block: the game removes or replaces the block entity.
            Ticking::No => {}
        }
    }
    /// The start of `Level.tickBlockEntities`: pending slots join the list
    /// and freed ones leave it. `state` reads the block state at a position.
    /// Returns the slots to tick, in order; check each with `is_live` before
    /// ticking it, as the ticks before it can free it.
    pub fn tickers(&mut self, state: impl Fn(Pos) -> u32) -> Arc<Vec<(Pos, u32)>> {
        for (pos, token) in std::mem::take(&mut self.added) {
            if self.pending.get(&pos) != Some(&token) {
                continue;
            }
            self.pending.remove(&pos);
            if self.live.contains_key(&pos)
                || !self.map.contains_key(&pos)
                || self.ticker_at(&pos, state(pos)) != Ticking::Yes
            {
                continue;
            }
            let slot = self.next();
            self.live.insert(
                pos,
                LiveTicker {
                    slot,
                    warned_generation: None,
                },
            );
            Arc::make_mut(&mut self.tickers).push((pos, slot));
        }
        for pos in std::mem::take(&mut self.replaced) {
            if self.live.contains_key(&pos) && self.ticker_at(&pos, state(pos)) != Ticking::Yes {
                self.cancel(&pos);
            }
        }
        if !std::mem::take(&mut self.freed) && self.out.is_empty() {
            return Arc::clone(&self.tickers);
        }
        self.retire_taken(|_| true);
        self.freed = false;
        let (live, map) = (&mut self.live, &self.map);
        Arc::make_mut(&mut self.tickers).retain(|(p, slot)| {
            let mine = live.get(p).is_some_and(|ticker| ticker.slot == *slot);
            if mine && !map.contains_key(p) {
                live.remove(p);
            }
            mine && map.contains_key(p)
        });
        Arc::clone(&self.tickers)
    }
    /// The ticker list as it stands, without promoting pending slots.
    pub fn ticker_slots(&self) -> Arc<Vec<(Pos, u32)>> {
        Arc::clone(&self.tickers)
    }
    /// Whether a slot from `tickers` still ticks.
    pub fn is_live(&self, (pos, slot): (Pos, u32)) -> bool {
        self.live
            .get(&pos)
            .is_some_and(|ticker| ticker.slot == slot)
            && self.map.contains_key(&pos)
    }
    /// Record an invalid visit of the bound mod ticker. Returns the type name on
    /// the first such visit so the caller can log the incompatible block state.
    pub fn record_invalid_mod_tick(&mut self, pos: Pos, generation: u64) -> Option<&str> {
        let ticker = self.live.get_mut(&pos)?;
        if ticker.warned_generation.replace(generation) == Some(generation) {
            return None;
        }
        match self.map.get(&pos)? {
            BlockEntity::Mod(entity) => Some(entity.id()),
            _ => None,
        }
    }
    /// A valid tick completed; don't clear a warning made by a replacement instance.
    pub fn clear_ticker_warning(&mut self, pos: Pos, generation: u64) {
        if let Some(ticker) = self.live.get_mut(&pos)
            && ticker.warned_generation == Some(generation)
        {
            ticker.warned_generation = None;
        }
    }
    /// Frees the slot at `pos` (the block entity stays).
    pub fn forget(&mut self, pos: &Pos) {
        self.cancel(pos);
    }
}

#[cfg(test)]
mod attachment_tests;

#[cfg(test)]
mod lease_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn hopper() -> BlockEntity {
        BlockEntity::Chest(vec![None; 5])
    }
    /// States: 0 air, 1 ticks, 2 idle.
    fn level() -> BlockEntities {
        let mut b = BlockEntities::default();
        b.set_states(vec![Ticking::No, Ticking::Yes, Ticking::Idle]);
        b
    }
    fn order(b: &mut BlockEntities, states: &HashMap<Pos, u32>) -> Vec<Pos> {
        b.tickers(|p| states.get(&p).copied().unwrap_or(0))
            .iter()
            .map(|(p, _)| *p)
            .collect()
    }

    #[test]
    fn generations_follow_identity_not_position_or_tick_storage() {
        let mut b = level();
        let p = (1, 64, 1);
        b.get_or_insert_with(p, hopper);
        let first = b.generation(&p).unwrap();
        b.insert(p, hopper()); // Same-type payload load, as /data does.
        assert_eq!(b.generation(&p), Some(first));
        let (lease, taken) = b.take(&p).unwrap();
        assert_eq!(b.generation(&p), Some(first));
        assert!(b.put(lease, taken));
        assert_eq!(b.generation(&p), Some(first));
        b.insert(
            p,
            BlockEntity::Mod(Box::new(crate::chunk::block_entity::ModBlockEntity::new(
                "test:other".into(),
                Default::default(),
            ))),
        );
        let replaced = b.generation(&p).unwrap();
        assert_ne!(replaced, first);
        b.remove(&p);
        assert_eq!(b.generation(&p), None);
        b.insert(p, hopper());
        let recreated = b.generation(&p).unwrap();
        assert_ne!(recreated, replaced);
        b.unload(|pos| *pos == p);
        assert_eq!(b.generation(&p), None);
        b.load_chunk(vec![(p, hopper())]);
        let loaded = b.generation(&p).unwrap();
        assert_ne!(loaded, recreated);
        b.load_chunk(vec![(p, hopper())]);
        assert_ne!(b.generation(&p), Some(loaded));
        let before_clear = b.generation(&p).unwrap();
        b.clear();
        assert_eq!(b.generation(&p), None);
        b.insert(p, hopper());
        assert_ne!(b.generation(&p), Some(before_clear));
        let mut another = level();
        another.insert(p, hopper());
        assert_ne!(b.generation(&p), another.generation(&p));
    }

    /// Unloading a chunk removes its block entities and leaves the others'
    /// sleep alone; `retain`, which may change what it keeps, wakes them all.
    #[test]
    fn unloading_a_chunk_keeps_the_others_asleep() {
        let (kept, gone) = ((1, 64, 1), (17, 64, 1));
        for indexed in [false, true] {
            let mut b = level();
            b.insert(kept, hopper());
            b.insert(gone, hopper());
            b.sleep_hopper(kept);
            b.sleep_hopper(gone);
            if indexed {
                b.unload_column((1, 0));
            } else {
                b.unload(|p| p.0 >> 4 == 1);
            }
            assert!(b.get(&gone).is_none());
            assert!(!b.is_asleep(&gone));
            assert!(!b.is_hopper_asleep(&gone));
            assert!(b.is_asleep(&kept));
            assert!(b.is_hopper_asleep(&kept));
            b.retain(|_, _| true);
            assert!(!b.any_asleep());
            assert!(!b.any_hopper_asleep());
        }
    }

    /// Column unloads (`unload_column`, `unload_column_where`) leave exactly what the full-scan
    /// `unload` leaves, through any mix of calls that add, take back, return and drop block entities.
    #[test]
    fn column_lists_match_a_full_scan() {
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut roll = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let state = |p: Pos| java_hash(p).unsigned_abs() % 3;
        let make = |kind: u64| match kind {
            0 => hopper(),
            1 => BlockEntity::Campfire(Box::default()),
            _ => BlockEntity::Mod(Box::new(crate::chunk::block_entity::ModBlockEntity::new(
                "test:thing".into(),
                Default::default(),
            ))),
        };
        // A (the column path) and B (the full scan) get the same calls.
        let (mut a, mut b) = (level(), level());
        let mut held = Vec::new();
        for step in 0..6000 {
            let pos = (
                roll(80) as i32 - 40,
                60 + roll(4) as i32,
                roll(80) as i32 - 40,
            );
            let column = chunk_of(pos);
            match roll(11) {
                0 => {
                    let kind = roll(3);
                    a.insert(pos, make(kind));
                    b.insert(pos, make(kind));
                }
                1 => {
                    let kind = roll(3);
                    a.get_or_insert_with(pos, || make(kind));
                    b.get_or_insert_with(pos, || make(kind));
                }
                2 => assert_eq!(a.remove(&pos).is_some(), b.remove(&pos).is_some()),
                3 => match (a.take(&pos), b.take(&pos)) {
                    (Some(from_a), Some(from_b)) => held.push((from_a, from_b)),
                    (None, None) => {}
                    _ => panic!("step {step}: take differs at {pos:?}"),
                },
                4 if !held.is_empty() => {
                    let ((lease_a, entity_a), (lease_b, entity_b)) =
                        held.swap_remove(roll(held.len() as u64) as usize);
                    assert_eq!(a.put(lease_a, entity_a), b.put(lease_b, entity_b));
                }
                5 => {
                    let keep = |p: &Pos, _: &mut BlockEntity| java_hash(*p) % 7 != 0;
                    a.retain(keep);
                    b.retain(keep);
                }
                6 => {
                    let entities = |kind: u64| {
                        (0..4)
                            .map(|i| ((column.0 * 16 + i * 3, 61, column.1 * 16 + i), make(kind)))
                            .collect::<Vec<_>>()
                    };
                    let kind = roll(3);
                    a.load_chunk(entities(kind));
                    b.load_chunk(entities(kind));
                }
                7 => {
                    a.unload_column(column);
                    b.unload(|p| chunk_of(*p) == column);
                }
                8 => {
                    let gone = |p: &Pos| p.1 % 2 == 0;
                    a.unload_column_where(column, gone);
                    b.unload(|p| chunk_of(*p) == column && gone(p));
                }
                9 => assert_eq!(*a.tickers(state), *b.tickers(state), "step {step}"),
                _ => {
                    a.sleep(pos);
                    b.sleep(pos);
                }
            }
            let keys = |s: &BlockEntities| {
                let mut keys: Vec<(Pos, Option<u64>, bool)> = s
                    .keys()
                    .map(|p| (*p, s.generation(p).map(|_| 0), s.is_asleep(p)))
                    .collect();
                keys.sort();
                keys
            };
            assert_eq!(keys(&a), keys(&b), "step {step}");
            let (x, z, radius) = (roll(100) as f64 - 50.0, roll(100) as f64 - 50.0, 20.0);
            assert_eq!(
                near(&a, x, z, radius),
                near(&b, x, z, radius),
                "step {step}"
            );
        }
        // Every list empties with the map: nothing is left behind for a gone block entity.
        for (lease_a, entity_a) in held.into_iter().map(|(a, _)| a) {
            a.put(lease_a, entity_a);
        }
        let all: Vec<(i32, i32)> = a.keys().map(|p| chunk_of(*p)).collect();
        for column in all {
            a.unload_column(column);
        }
        a.tickers(state);
        assert!(a.is_empty() && a.columns.is_empty() && a.mods.is_empty());
    }

    fn near(b: &BlockEntities, x: f64, z: f64, radius: f64) -> Vec<Pos> {
        let mut found: Vec<Pos> = b.mods_near(x, z, radius).copied().collect();
        found.sort();
        found
    }
    #[test]
    fn a_sleeping_block_entity_wakes_on_any_change() {
        let p = (1, 64, 1);
        let mut b = level();
        b.sleep(p);
        assert!(!b.is_asleep(&p));
        b.insert(p, hopper());
        let asleep = |b: &mut BlockEntities, wake: &dyn Fn(&mut BlockEntities)| {
            b.sleep(p);
            assert!(b.is_asleep(&p));
            wake(b);
            b.is_asleep(&p)
        };
        assert!(asleep(&mut b, &|_| {}));
        assert!(!asleep(&mut b, &|b| {
            b.get_mut(&p);
        }));
        assert!(!asleep(&mut b, &|b| {
            b.insert(p, hopper());
        }));
        assert!(!asleep(&mut b, &|b| {
            b.get_or_insert_with(p, hopper);
        }));
        assert!(!asleep(&mut b, &|b| {
            let (lease, held) = b.take(&p).unwrap();
            b.put(lease, held);
        }));
        assert!(!asleep(&mut b, &|b| b.state_changed(p, 1, 1)));
        assert!(!asleep(&mut b, &|b| b.forget(&p)));
        assert!(!asleep(&mut b, &|b| {
            b.iter_mut().count();
        }));
        assert!(!asleep(&mut b, &|b| {
            b.values_mut().count();
        }));
        assert!(!asleep(&mut b, &|b| b.retain(|_, _| true)));
        assert!(!asleep(&mut b, &|b| b.load_chunk(vec![(p, hopper())])));
        assert!(!asleep(&mut b, &|b| {
            b.remove(&p);
            b.insert(p, hopper());
        }));
        b.sleep(p);
        b.clear();
        assert!(!b.is_asleep(&p));
    }
    /// A hopper reads the container above it, so a write or state change up
    /// there wakes the hopper sleeping below before its next tick.
    #[test]
    fn a_sleeping_block_entity_wakes_when_the_block_above_changes() {
        let p = (1, 64, 1);
        let above = (p.0, p.1 + 1, p.2);
        let mut b = level();
        b.insert(p, hopper());
        assert!(!b.any_asleep());
        b.sleep_hopper(p);
        assert!(b.any_asleep() && b.any_hopper_asleep());
        b.insert(above, BlockEntity::Chest(vec![None; 27]));
        assert!(!b.is_asleep(&p), "a container placed above wakes it");
        b.sleep_hopper(p);
        b.get_mut(&above);
        assert!(!b.is_asleep(&p), "a slot write above wakes it");
        b.sleep_hopper(p);
        b.wake(&above);
        assert!(b.is_asleep(&p), "waking another position leaves it asleep");
        b.state_changed(above, 1, 1);
        assert!(!b.is_asleep(&p), "a state change above wakes it");
        b.sleep_hopper(p);
        b.wake_below(&above);
        assert!(!b.is_asleep(&p));
        assert!(!b.any_asleep() && !b.any_hopper_asleep());
        // A furnace's sleep is not a hopper's: the hopper scans skip it.
        b.sleep(above);
        assert!(b.any_asleep());
        assert!(!b.any_hopper_asleep());
        b.wake_hopper(&above);
        assert!(
            b.is_asleep(&above),
            "a hopper scan leaves another kind asleep"
        );
    }
    #[test]
    fn mod_block_entities_are_listed_by_chunk_until_they_go() {
        let mut b = level();
        let modded = || BlockEntity::Mod(Box::default());
        b.insert((1, 64, 1), modded());
        b.insert((200, 64, 1), modded());
        b.insert((2, 64, 2), hopper());
        b.get_or_insert_with((3, 64, 3), modded);
        b.load_chunk(vec![((4, 64, 4), modded()), ((5, 64, 5), hopper())]);
        // Only mods, and only chunks within reach.
        assert_eq!(
            near(&b, 0.0, 0.0, 64.0),
            [(1, 64, 1), (3, 64, 3), (4, 64, 4)]
        );
        assert_eq!(near(&b, 200.0, 0.0, 8.0), [(200, 64, 1)]);
        // Taken out for a tick, still listed; removed or replaced, gone.
        let (lease, taken) = b.take(&(1, 64, 1)).unwrap();
        assert_eq!(near(&b, 0.0, 0.0, 64.0).len(), 3);
        b.put(lease, taken);
        b.remove(&(3, 64, 3));
        b.insert((4, 64, 4), hopper());
        assert_eq!(near(&b, 0.0, 0.0, 64.0), [(1, 64, 1)]);
        b.retain(|p, _| p.0 != 1);
        assert!(near(&b, 0.0, 0.0, 64.0).is_empty());
        b.clear();
        assert!(near(&b, 200.0, 0.0, 8.0).is_empty());
    }
    #[test]
    fn attachments_follow_block_entity_lifetime() {
        use sunrise_mod_runtime::attachments::{AttachmentType, Holder};
        let ty = AttachmentType {
            namespace: "test",
            name: "value",
            default: || 0,
            codec: None,
            copy_on_death: false,
            sync_codec: None,
        };
        let p = (1, 64, 2);
        let mut entities = level();
        assert!(entities.attachments_mut(p).is_none());
        entities.insert(p, hopper());
        ty.set(entities.attachments_mut(p).unwrap(), Holder::Level, 4)
            .unwrap();
        let (lease, held) = entities.take(&p).unwrap();
        entities.put(lease, held);
        assert_eq!(
            ty.get(entities.attachments_mut(p).unwrap(), Holder::Level),
            Ok(4)
        );
        entities.insert(p, hopper());
        assert_eq!(
            ty.get(entities.attachments_mut(p).unwrap(), Holder::Level),
            Ok(4)
        );
        let machine = |id: &str| {
            BlockEntity::Mod(Box::new(crate::chunk::block_entity::ModBlockEntity::new(
                id.into(),
                Default::default(),
            )))
        };
        entities.insert(p, machine("test:a"));
        assert_eq!(
            ty.get(entities.attachments_mut(p).unwrap(), Holder::Level),
            Ok(0)
        );
        ty.set(entities.attachments_mut(p).unwrap(), Holder::Level, 4)
            .unwrap();
        entities.insert(p, machine("test:a"));
        assert_eq!(
            ty.get(entities.attachments_mut(p).unwrap(), Holder::Level),
            Ok(4)
        );
        entities.insert(p, machine("test:b"));
        assert_eq!(
            ty.get(entities.attachments_mut(p).unwrap(), Holder::Level),
            Ok(0)
        );
        ty.set(entities.attachments_mut(p).unwrap(), Holder::Level, 4)
            .unwrap();
        entities.remove(&p);
        entities.insert(p, hopper());
        assert_eq!(
            ty.get(entities.attachments_mut(p).unwrap(), Holder::Level),
            Ok(0)
        );
        ty.set(entities.attachments_mut(p).unwrap(), Holder::Level, 5)
            .unwrap();
        entities.retain(|_, _| false);
        entities.insert(p, hopper());
        assert_eq!(
            ty.get(entities.attachments_mut(p).unwrap(), Holder::Level),
            Ok(0)
        );
    }

    #[test]
    fn slots_follow_vanilla_lifecycle() {
        let mut b = level();
        let mut s = HashMap::new();
        for p in [(0, 0, 0), (1, 0, 0), (2, 0, 0)] {
            s.insert(p, 1);
            b.insert(p, hopper());
        }
        assert_eq!(order(&mut b, &s), [(0, 0, 0), (1, 0, 0), (2, 0, 0)]);
        // Replacing keeps the slot; removing and placing again appends.
        b.insert((0, 0, 0), hopper());
        b.remove(&(1, 0, 0));
        b.insert((1, 0, 0), hopper());
        assert_eq!(order(&mut b, &s), [(0, 0, 0), (2, 0, 0), (1, 0, 0)]);
        // Take and put keep it; an idle state frees it, ticking again appends.
        let (lease, e) = b.take(&(0, 0, 0)).unwrap();
        b.put(lease, e);
        assert_eq!(order(&mut b, &s), [(0, 0, 0), (2, 0, 0), (1, 0, 0)]);
        s.insert((2, 0, 0), 2);
        b.state_changed((2, 0, 0), 1, 2);
        assert_eq!(order(&mut b, &s), [(0, 0, 0), (1, 0, 0)]);
        s.insert((2, 0, 0), 1);
        b.state_changed((2, 0, 0), 2, 1);
        assert_eq!(order(&mut b, &s), [(0, 0, 0), (1, 0, 0), (2, 0, 0)]);
        // A slot freed during a tick is not live any more.
        let slots = b.tickers(|p| s.get(&p).copied().unwrap_or(0));
        b.retain(|p, _| *p != (1, 0, 0));
        assert!(b.is_live(slots[0]) && !b.is_live(slots[1]));
        // A block entity taken and never brought back loses its slot.
        b.take(&(0, 0, 0));
        assert_eq!(order(&mut b, &s), [(2, 0, 0)]);
        assert_eq!(order(&mut b, &s), [(2, 0, 0)]);
    }

    #[test]
    fn chunk_loads_in_open_hash_map_order() {
        // A few keys: their mixed hashes over 32 slots, read from the top.
        let positions = [
            (0, 64, 0),
            (1, 64, 0),
            (0, 64, 1),
            (15, -64, 15),
            (3, 70, 9),
        ];
        let slot = |p: &Pos| mix(java_hash(*p)) & 31;
        let mut expected = positions.to_vec();
        expected.sort_by_key(|p| std::cmp::Reverse(slot(p)));
        let distinct: std::collections::HashSet<usize> = positions.iter().map(slot).collect();
        assert_eq!(distinct.len(), positions.len());
        assert_eq!(chunk_map_order(positions), expected);
        // Past 24 keys the table doubles; every key stays, once.
        let many: Vec<Pos> = (0..100).map(|i| (i % 16, i / 16, 3)).collect();
        let order = chunk_map_order(many.iter().copied().chain(many.iter().copied()));
        assert_eq!(order.len(), 100);
    }

    /// A chunk loaded over a block entity already there: the new one replaces
    /// it, and keeps the old slot only while its block state ticks.
    #[test]
    fn loaded_chunk_replaces_block_entity_by_state() {
        let mut b = level();
        let mut s = HashMap::new();
        let (p, q) = ((0, 0, 0), (1, 0, 0));
        for pos in [p, q] {
            s.insert(pos, 1);
        }
        // Two chunks, so the order is the order they load in.
        b.load_chunk(vec![(p, hopper())]);
        b.load_chunk(vec![(q, hopper())]);
        assert_eq!(order(&mut b, &s), [p, q]);
        // Replaced where the state ticks: the slot stays in its place.
        b.load_chunk(vec![(p, hopper())]);
        assert_eq!(order(&mut b, &s), [p, q]);
        // Replaced where the state has no ticker: the slot is freed, and the
        // new block entity is the one in the map.
        s.insert(p, 2);
        b.load_chunk(vec![(p, BlockEntity::Chest(vec![None; 27]))]);
        assert_eq!(order(&mut b, &s), [q]);
        assert!(matches!(b.get(&p), Some(BlockEntity::Chest(items)) if items.len() == 27));
        // Ticking again later appends it, as any new ticker does.
        s.insert(p, 1);
        b.state_changed(p, 2, 1);
        assert_eq!(order(&mut b, &s), [q, p]);
    }

    #[test]
    fn boxes_list_exactly_the_structure_and_test_blocks() {
        let test_block = || BlockEntity::TestInstanceBlock(Box::default());
        let listed = |b: &BlockEntities| {
            let mut v: Vec<Pos> = b.boxes().map(|(p, _)| *p).collect();
            v.sort();
            v
        };
        let mut b = level();
        let (p, q, r) = ((0, 0, 0), (1, 0, 0), (2, 0, 0));
        b.insert(p, test_block());
        b.insert(q, hopper());
        assert_eq!(listed(&b), [p]);
        // Replaced by another kind, and back.
        b.insert(p, hopper());
        assert_eq!(listed(&b), []);
        b.insert(q, test_block());
        b.get_or_insert_with(r, test_block);
        assert_eq!(listed(&b), [q, r]);
        // Taken out: not shown until put back; replacement during take wins.
        let (lease, taken) = b.take(&q).unwrap();
        assert_eq!(listed(&b), [r]);
        b.put(lease, taken);
        assert_eq!(listed(&b), [q, r]);
        let (lease, taken) = b.take(&r).unwrap();
        b.insert(r, hopper());
        assert!(!b.put(lease, taken));
        assert_eq!(listed(&b), [q]);
        b.remove(&q);
        assert_eq!(listed(&b), []);
        b.load_chunk(vec![(p, test_block()), (q, hopper())]);
        assert_eq!(listed(&b), [p]);
        b.retain(|pos, _| *pos != p);
        assert_eq!(listed(&b), []);
        b.insert(r, test_block());
        b.clear();
        assert!(b.boxes.is_empty());
    }
}

#[cfg(test)]
#[path = "block_entities/on_load_tests.rs"]
mod on_load_tests;
