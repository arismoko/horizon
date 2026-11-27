//! Resident chunk membership, with a token for consumers that cache its keys.
use crate::{
    chunk::Chunk,
    fast_hash::{FxHashMap, FxHashSet},
};
use std::{
    collections::{
        BTreeSet, VecDeque,
        hash_map::{Iter, ValuesMut},
    },
    ops::Deref,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

use sunrise_mod_runtime::attachments::{
    ATTACHMENTS_KEY, AttachmentError, Attachments, Holder, attachment_compound,
    observer::AttachmentObserver,
};

type Key = (i32, i32);
type Columns = FxHashMap<Key, Arc<Chunk>>;

/// `ClientChunkCache.Storage`: a square view over floor-modulo slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientChunkRange {
    center: Key,
    radius: u32,
}

impl ClientChunkRange {
    pub fn new(center: Key, view_distance: i32) -> Self {
        Self {
            center,
            radius: view_distance.max(2) as u32 + 3,
        }
    }

    pub fn contains(self, (x, z): Key) -> bool {
        x.abs_diff(self.center.0) <= self.radius && z.abs_diff(self.center.1) <= self.radius
    }

    fn slot(self, (x, z): Key) -> (i64, i64) {
        // A sparse ring keeps only occupied slots; the server supplies the radius.
        let diameter = 2 * i64::from(self.radius) + 1;
        (
            i64::from(x).rem_euclid(diameter),
            i64::from(z).rem_euclid(diameter),
        )
    }
}

/// The logical group cells in a client's resident columns, shared by its active and viewed
/// worlds. A worker's `Arc<Chunk>` snapshot is not another resident column.
#[derive(Clone, Default)]
pub struct ConnectionGroupCount(Arc<AtomicUsize>);

impl ConnectionGroupCount {
    pub fn allows_replacement(&self, old: usize, new: usize, limit: usize) -> bool {
        new <= old
            || self
                .0
                .load(Ordering::Relaxed)
                .checked_add(new - old)
                .is_some_and(|n| n <= limit)
    }
}

struct GroupResidency {
    count: ConnectionGroupCount,
    occupied: usize,
}

impl GroupResidency {
    fn replace(&mut self, old: usize, new: usize) {
        if new > old {
            self.count.0.fetch_add(new - old, Ordering::Relaxed);
            self.occupied += new - old;
        } else if old > new {
            self.count.0.fetch_sub(old - new, Ordering::Relaxed);
            self.occupied -= old - new;
        }
    }
}

impl Drop for GroupResidency {
    fn drop(&mut self) {
        self.replace(self.occupied, 0);
    }
}

static NEXT_REVISION: AtomicU64 = AtomicU64::new(1);
fn next_revision() -> u64 {
    NEXT_REVISION.fetch_add(1, Ordering::Relaxed)
}

/// The membership changes one map keeps for the consumers that read them
/// every tick (`TickChunkMembership.synchronize`); older ones are dropped.
const MAX_RECENT_DELTAS: usize = 256;

/// One membership change of one `ChunkMap`, stamped with the revision it
/// brought the map to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkMembershipDelta {
    Added(Key),
    Removed(Key),
    Reset,
}

/// Read access behaves like a map. Structural writes are explicit so edits to
/// blocks or light don't invalidate consumers of the loaded chunk key set.
/// Tokens are process-wide: replacing the entire map also invalidates caches.
pub struct ChunkMap {
    columns: Columns,
    // The x-then-z projection of visible membership; gameplay still iterates the hash map.
    ordered: BTreeSet<Key>,
    /// In-range coordinates already uniquely index their ring slots. Only hidden occupants
    /// need a modulo index. They retain their blocks, attachments and light, but none of
    /// the normal map reads (including mesh/tick membership) can see them.
    hidden: FxHashMap<(i64, i64), Arc<Chunk>>,
    cache_range: Option<ClientChunkRange>,
    /// Columns a server keeps on this client past the cache's view (`sunrise:levels` `far`): a
    /// centre move never hides one, and one is admitted wherever it is.
    far: FxHashSet<Key>,
    /// Bumped each time `far` changes, for the consumers that cache what it held.
    far_changes: u64,
    revision: u64,
    /// The oldest revision `recent_deltas` still answers for: the revision of
    /// the change it last dropped, or the revision that cleared them. A
    /// consumer holding an older one rebuilds, it does not replay.
    min_valid_revision: u64,
    /// This map's recent membership changes, in revision order.
    /// Revisions are process-wide: the numbers missing in between belong to
    /// the other maps (a server thread's, another dimension's), which say
    /// nothing about this one's keys.
    recent_deltas: VecDeque<(u64, ChunkMembershipDelta)>,
    /// The resident chunks generation has not placed its animals in yet.
    fresh: FxHashSet<Key>,
    attachment_observer: Option<Arc<dyn AttachmentObserver>>,
    group_residency: Option<GroupResidency>,
    /// For each column, how many of the 3×3 columns around it (itself
    /// included) are resident; absent when none is. `surrounded` asks one
    /// map instead of nine every tick, for every ticking chunk.
    near: FxHashMap<Key, u8>,
}
impl Default for ChunkMap {
    fn default() -> Self {
        let revision = next_revision();
        Self {
            columns: Columns::default(),
            ordered: BTreeSet::new(),
            hidden: FxHashMap::default(),
            cache_range: None,
            far: FxHashSet::default(),
            far_changes: 0,
            revision,
            min_valid_revision: revision,
            recent_deltas: VecDeque::new(),
            fresh: FxHashSet::default(),
            attachment_observer: None,
            group_residency: None,
            near: FxHashMap::default(),
        }
    }
}
// Only residency installation binds saved maps. Lighting COW leaves these shared
// entries indexed; attachment access below rebinds an extras COW before its write.
fn bind_saved_attachments(
    observer: &dyn AttachmentObserver,
    key: Key,
    chunk: &mut Arc<Chunk>,
) -> bool {
    let Some(saved) = &chunk.saved else {
        return false;
    };
    if !saved.attachments.has_index_entries() && !saved.attachments.has_index_binding() {
        return false;
    }
    let saved = Arc::make_mut(Arc::make_mut(chunk).saved.as_mut().unwrap());
    observer
        .bind(Holder::Chunk([key.0, key.1]), &mut saved.attachments)
        .is_some()
}

impl Drop for ChunkMap {
    fn drop(&mut self) {
        self.unload_attachments();
    }
}

impl ChunkMap {
    /// Add this map to a connection's resident count. Changes run on the client thread.
    pub fn track_connection_groups(&mut self, count: ConnectionGroupCount) {
        self.untrack_connection_groups();
        let mut residency = GroupResidency { count, occupied: 0 };
        residency.replace(
            0,
            self.columns
                .values()
                .chain(self.hidden.values())
                .map(|c| c.connection_groups.len())
                .sum(),
        );
        self.group_residency = Some(residency);
    }

    /// Stop counting this map's group cells as resident without visiting its columns.
    pub fn untrack_connection_groups(&mut self) {
        drop(self.group_residency.take());
    }

    /// Check the net change before publishing a replacement or an ordered group patch.
    pub fn allows_connection_groups(&self, old: usize, new: usize, limit: usize) -> bool {
        self.group_residency
            .as_ref()
            .is_none_or(|residency| residency.count.allows_replacement(old, new, limit))
    }

    pub(crate) fn connection_group_changed(&mut self, old: bool, new: bool) {
        if let Some(residency) = &mut self.group_residency {
            residency.replace(usize::from(old), usize::from(new));
        }
    }

    /// Install the authoritative dimension's observer, including already loaded
    /// saved entries before their first read. Client/worker maps have no observer.
    pub fn observe_attachments(&mut self, observer: Arc<dyn AttachmentObserver>) {
        self.unload_attachments();
        for (key, chunk) in &mut self.columns {
            bind_saved_attachments(&*observer, *key, chunk);
        }
        self.attachment_observer = Some(observer);
    }

    fn unload_attachments(&self) {
        if let Some(observer) = &self.attachment_observer {
            for (x, z) in self.resident_keys() {
                observer.unload(Holder::Chunk([x, z]));
            }
        }
    }

    /// Read an existing loaded column without creating saved extras or an index binding.
    pub fn attachments(&self, key: &Key) -> Result<Option<&Attachments>, AttachmentError> {
        use sunrise_core::nbt::JavaString;
        static KEY: std::sync::LazyLock<JavaString> =
            std::sync::LazyLock::new(|| ATTACHMENTS_KEY.into());
        let column = self
            .columns
            .get(key)
            .ok_or(AttachmentError::MissingHolder)?;
        let Some(saved) = &column.saved else {
            return Ok(None);
        };
        attachment_compound(saved.root.get(&KEY))?;
        Ok(Some(&saved.attachments))
    }

    /// The live map, rebound after extras COW but never merely for lighting COW.
    pub fn attachments_mut(&mut self, key: &Key) -> Result<&mut Attachments, AttachmentError> {
        let chunk = self
            .columns
            .get_mut(key)
            .ok_or(AttachmentError::MissingHolder)?;
        let saved = Arc::make_mut(Arc::make_mut(chunk).saved.get_or_insert_default());
        attachment_compound(saved.root.get(&ATTACHMENTS_KEY.into()))?;
        if let Some(observer) = &self.attachment_observer
            && !saved.attachments.has_index_binding()
        {
            let _ = observer.bind(Holder::Chunk([key.0, key.1]), &mut saved.attachments);
        }
        Ok(&mut saved.attachments)
    }

    /// Move the view without retiring any column. On a diameter change the caller has
    /// already discarded out-of-range occupants, so all surviving slots are in range.
    #[must_use]
    pub fn set_client_chunk_range(&mut self, range: ClientChunkRange) -> (Vec<Key>, Vec<Key>) {
        if self.cache_range == Some(range) {
            return (Vec::new(), Vec::new());
        }
        let shown: Vec<_> = self
            .hidden
            .values()
            .filter_map(|chunk| {
                let key = (chunk.x, chunk.z);
                range.contains(key).then_some(key)
            })
            .collect();
        for &key in &shown {
            let slot = self
                .cache_range
                .expect("hidden slots have a range")
                .slot(key);
            let chunk = self.hidden.remove(&slot).expect("visible again");
            self.add_visible(key);
            self.columns.insert(key, chunk);
        }
        let hidden: Vec<_> = self
            .columns
            .keys()
            .copied()
            .filter(|&key| !range.contains(key) && !self.far.contains(&key))
            .collect();
        for &key in &hidden {
            let chunk = self
                .columns
                .remove(&key)
                .expect("visible before centre moved");
            self.remove_visible(key);
            let previous = self.hidden.insert(range.slot(key), chunk);
            debug_assert!(previous.is_none(), "one column per client cache slot");
        }
        self.cache_range = Some(range);
        (hidden, shown)
    }

    /// An in-range admission can collide only with a hidden column. The caller must
    /// unload it before inserting, while that column's state is still resident.
    pub fn client_chunk_alias(&self, key: Key) -> Option<Key> {
        let range = self.cache_range?;
        // A far column outside the view has no slot.
        if self.hidden.is_empty() || !range.contains(key) {
            return None;
        }
        let slot = range.slot(key);
        let chunk = self.hidden.get(&slot)?;
        Some((chunk.x, chunk.z))
    }

    /// Whether the column is one a server keeps here past the cache's view.
    pub fn is_far(&self, key: Key) -> bool {
        self.far.contains(&key)
    }

    /// How many far columns there are, and a number that changes whenever they do.
    pub fn far_state(&self) -> (usize, u64) {
        (self.far.len(), self.far_changes)
    }

    /// The server keeps `keys` here past the cache's view from now on. One hidden in its slot
    /// is visible again: those come back, for the caller to draw again.
    #[must_use]
    pub fn keep_far(&mut self, keys: &[Key]) -> Vec<Key> {
        let mut shown = Vec::new();
        for &key in keys {
            if !self.far.insert(key) {
                continue;
            }
            let Some(range) = self.cache_range else {
                continue;
            };
            let slot = range.slot(key);
            if self.hidden.get(&slot).is_some_and(|c| (c.x, c.z) == key) {
                let chunk = self.hidden.remove(&slot).expect("hidden in its slot");
                self.add_visible(key);
                self.columns.insert(key, chunk);
                shown.push(key);
            }
        }
        self.far_changes += 1;
        shown
    }

    /// The server keeps `keys` here no more: the resident ones outside the cache's view, which
    /// the caller forgets.
    #[must_use]
    pub fn release_far(&mut self, keys: &[Key]) -> Vec<Key> {
        let gone = keys
            .iter()
            .copied()
            .filter(|key| self.far.remove(key))
            .filter(|&key| {
                self.columns.contains_key(&key)
                    && self.cache_range.is_some_and(|range| !range.contains(key))
            })
            .collect();
        self.far_changes += 1;
        gone
    }

    /// Includes hidden occupants for a storage resize or level teardown.
    pub fn resident_keys(&self) -> impl Iterator<Item = Key> + '_ {
        self.columns
            .keys()
            .copied()
            .chain(self.hidden.values().map(|chunk| (chunk.x, chunk.z)))
    }

    pub fn resident_get(&self, key: &Key) -> Option<&Arc<Chunk>> {
        self.columns.get(key).or_else(|| {
            if self.hidden.is_empty() {
                return None;
            }
            let chunk = self.hidden.get(&self.cache_range?.slot(*key))?;
            ((chunk.x, chunk.z) == *key).then_some(chunk)
        })
    }

    /// Light is owned by the level, not by the block cache's current view.
    pub fn resident_get_mut(&mut self, key: &Key) -> Option<&mut Arc<Chunk>> {
        if let Some(chunk) = self.columns.get_mut(key) {
            return Some(chunk);
        }
        if self.hidden.is_empty() {
            return None;
        }
        let chunk = self.hidden.get_mut(&self.cache_range?.slot(*key))?;
        ((chunk.x, chunk.z) == *key).then_some(chunk)
    }

    pub fn membership_revision(&self) -> u64 {
        self.revision
    }
    fn record_delta(&mut self, delta: ChunkMembershipDelta) {
        if self.recent_deltas.len() >= MAX_RECENT_DELTAS
            && let Some((evicted, _)) = self.recent_deltas.pop_front()
        {
            self.min_valid_revision = evicted;
        }
        self.recent_deltas.push_back((self.revision, delta));
    }
    /// This map's membership changes since `last_revision`, one of its own
    /// earlier `membership_revision`s, or `None` when they can no longer be
    /// answered: the consumer's revision predates what the ring kept, the map
    /// was cleared since (a reset rebuilds everything), or the revision is not
    /// this map's, which its own revisions give away: every one of them in the
    /// answered range is the opening revision or a change the ring still
    /// holds, so a revision another map took in between is refused, not
    /// replayed.
    pub fn deltas_since(
        &self,
        last_revision: u64,
    ) -> Option<impl Iterator<Item = (u64, ChunkMembershipDelta)> + '_> {
        if last_revision == 0
            || last_revision < self.min_valid_revision
            || last_revision > self.revision
            || self.recent_deltas.is_empty()
        {
            return None;
        }
        let pos = self
            .recent_deltas
            .partition_point(|(rev, _)| *rev <= last_revision);
        // A revision this map never took: another map's, drawing on the same
        // process-wide counter. Only the opening revision and the ones the
        // ring still holds are this map's.
        let is_own_revision = last_revision == self.min_valid_revision
            || (pos > 0 && self.recent_deltas[pos - 1].0 == last_revision);
        if !is_own_revision {
            return None;
        }
        let slice = self.recent_deltas.range(pos..);
        if slice
            .clone()
            .any(|(_, d)| *d == ChunkMembershipDelta::Reset)
        {
            return None;
        }
        Some(slice.copied())
    }
    pub fn insert(&mut self, key: Key, mut chunk: Arc<Chunk>) -> Option<Arc<Chunk>> {
        let limits = chunk
            .saved
            .as_ref()
            .map(|saved| {
                saved
                    .block_entity_sources
                    .iter()
                    .filter_map(|(pos, tag)| match tag.get("sunrise:stack_limit") {
                        Some(sunrise_core::nbt::Tag::Int(n)) if (1..=99).contains(n) => {
                            Some((*pos, *n as u32))
                        }
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        if chunk.container_stack_limits != limits {
            Arc::make_mut(&mut chunk).container_stack_limits = limits;
        }
        if let Some(observer) = &self.attachment_observer {
            let bound = bind_saved_attachments(&**observer, key, &mut chunk);
            if !bound && self.columns.contains_key(&key) {
                observer.unload(Holder::Chunk([key.0, key.1]));
            }
        }
        if chunk.fresh {
            self.fresh.insert(key);
        } else {
            self.fresh.remove(&key);
        }
        let groups = chunk.connection_groups.len();
        let previous = self.columns.insert(key, chunk);
        if let Some(residency) = &mut self.group_residency {
            residency.replace(
                previous.as_ref().map_or(0, |c| c.connection_groups.len()),
                groups,
            );
        }
        if previous.is_none() {
            self.add_visible(key);
        }
        previous
    }
    fn add_visible(&mut self, key: Key) {
        self.ordered.insert(key);
        for dz in -1..=1 {
            for dx in -1..=1 {
                let near = (key.0.wrapping_add(dx), key.1.wrapping_add(dz));
                *self.near.entry(near).or_insert(0) += 1;
            }
        }
        self.revision = next_revision();
        self.record_delta(ChunkMembershipDelta::Added(key));
    }
    fn remove_visible(&mut self, key: Key) {
        self.ordered.remove(&key);
        for dz in -1..=1 {
            for dx in -1..=1 {
                let near = (key.0.wrapping_add(dx), key.1.wrapping_add(dz));
                if let Some(n) = self.near.get_mut(&near) {
                    *n -= 1;
                    if *n == 0 {
                        self.near.remove(&near);
                    }
                }
            }
        }
        self.revision = next_revision();
        self.record_delta(ChunkMembershipDelta::Removed(key));
    }
    pub fn remove(&mut self, key: &Key) -> Option<Arc<Chunk>> {
        self.fresh.remove(key);
        let previous = if let Some(chunk) = self.columns.remove(key) {
            self.remove_visible(*key);
            Some(chunk)
        } else if let Some(range) = self.cache_range {
            let slot = range.slot(*key);
            if self.hidden.get(&slot).is_some_and(|c| (c.x, c.z) == *key) {
                self.hidden.remove(&slot)
            } else {
                None
            }
        } else {
            None
        };
        if let Some(chunk) = &previous {
            if let Some(residency) = &mut self.group_residency {
                residency.replace(chunk.connection_groups.len(), 0);
            }
            if let Some(observer) = &self.attachment_observer {
                observer.unload(Holder::Chunk([key.0, key.1]));
            }
        }
        previous
    }
    pub fn clear(&mut self) {
        if !self.far.is_empty() {
            self.far.clear();
            self.far_changes += 1;
        }
        if !self.columns.is_empty() || !self.hidden.is_empty() {
            self.unload_attachments();
            self.columns.clear();
            self.hidden.clear();
            if let Some(residency) = &mut self.group_residency {
                residency.replace(residency.occupied, 0);
            }
            self.ordered.clear();
            self.fresh.clear();
            self.near.clear();
            self.revision = next_revision();
            self.recent_deltas.clear();
            self.recent_deltas
                .push_back((self.revision, ChunkMembershipDelta::Reset));
            self.min_valid_revision = self.revision;
        }
    }
    /// Resident columns in lexical (x, z) order, independently of hash iteration.
    pub fn ordered_keys(&self) -> impl ExactSizeIterator<Item = Key> + '_ {
        self.ordered.iter().copied()
    }

    /// Whether the column at `key` and its eight neighbours are all resident.
    #[inline]
    pub fn surrounded(&self, key: Key) -> bool {
        self.near.get(&key) == Some(&9)
    }

    /// Whether the centre is lit and its 3×3 is resident, the column readiness
    /// required by `ChunkMap.prepareTickingChunk`.
    #[inline]
    pub fn is_ticking_ready(&self, key: Key) -> bool {
        self.columns.get(&key).is_some_and(|chunk| chunk.lit) && self.surrounded(key)
    }

    pub fn has_fresh(&self) -> bool {
        !self.fresh.is_empty()
    }
    /// The fresh keys in the chunk map's own iteration order, so the spawns
    /// they drive run in the order a scan of the columns gave.
    pub fn fresh_keys_ordered(&self) -> Vec<Key> {
        if self.fresh.is_empty() {
            Vec::new()
        } else if self.fresh.len() == 1 {
            self.fresh.iter().copied().collect()
        } else {
            self.columns
                .keys()
                .filter(|k| self.fresh.contains(k))
                .copied()
                .collect()
        }
    }
    pub fn remove_fresh(&mut self, key: &Key) {
        self.fresh.remove(key);
    }
    pub fn get_mut(&mut self, key: &Key) -> Option<&mut Arc<Chunk>> {
        self.columns.get_mut(key)
    }
    pub fn values_mut(&mut self) -> ValuesMut<'_, Key, Arc<Chunk>> {
        self.columns.values_mut()
    }
}
impl Deref for ChunkMap {
    type Target = Columns;
    fn deref(&self) -> &Columns {
        &self.columns
    }
}
impl<'a> IntoIterator for &'a ChunkMap {
    type Item = (&'a Key, &'a Arc<Chunk>);
    type IntoIter = Iter<'a, Key, Arc<Chunk>>;
    fn into_iter(self) -> Self::IntoIter {
        self.columns.iter()
    }
}
impl crate::light::LightWorld for ChunkMap {
    fn section(&self, cx: i32, s: usize, cz: i32) -> Option<&Arc<crate::chunk::Section>> {
        self.columns.section(cx, s, cz)
    }
    fn set_section_light(
        &mut self,
        cx: i32,
        s: usize,
        cz: i32,
        light: crate::chunk::Light,
        changed: &mut crate::light::LightChanges,
    ) {
        self.columns.set_section_light(cx, s, cz, light, changed);
    }
    fn layout(&self) -> crate::chunk::ColumnLayout {
        self.columns.layout()
    }
    fn dynamic_emission(&self, x: i32, y: i32, z: i32) -> Option<u8> {
        self.columns.dynamic_emission(x, y, z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{AttachmentHolder, ChunkRef, DimensionKey, LevelRef, WorldEpoch};
    use std::sync::{Mutex, Weak, atomic::AtomicUsize};
    use sunrise_core::nbt::Tag;
    use sunrise_mod_runtime::attachments::{
        AttachmentCodec, AttachmentType, index::AttachmentIndex,
        observer::IndexedAttachmentObserver,
    };

    #[test]
    fn active_and_viewed_residence_share_credit_until_replaced_or_unloaded() {
        let count = ConnectionGroupCount::default();
        let mut active = crate::world::World::new(sunrise_world::registry::BlockStateId(0), 1);
        let mut viewed = ChunkMap::default();
        active.chunks.track_connection_groups(count.clone());
        viewed.track_connection_groups(count.clone());
        let mut column = Chunk::empty(0, 0);
        column.set_connection_group(0, 0, 0, Some(9));
        viewed.insert((0, 0), Arc::new(column));
        active.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        assert!(active.set_connection_group(0, 0, 0, Some(8), false));
        assert!(!count.allows_replacement(0, 1, 2));
        assert!(count.allows_replacement(1, 1, 2));
        assert!(count.allows_replacement(2, 1, 1));

        // Mesh workers can keep the old Arc; it is no longer resident.
        let old = viewed.insert((0, 0), Arc::new(Chunk::empty(0, 0))).unwrap();
        assert_eq!(old.connection_group(0, 0, 0), Some(9));
        assert!(count.allows_replacement(0, 1, 2));
        viewed.insert((0, 0), old);
        assert!(!count.allows_replacement(0, 1, 2));
        active.unload_chunk((0, 0)).unwrap();
        assert!(count.allows_replacement(0, 1, 2));
        drop(viewed);
        assert!(count.allows_replacement(0, 2, 2));
    }

    type Index = Arc<Mutex<AttachmentIndex<AttachmentHolder>>>;
    static VALUE: AttachmentType<i32> = AttachmentType {
        namespace: "chunk_test",
        name: "value",
        default: || 0,
        codec: Some(AttachmentCodec {
            encode: |n| Ok([("n".into(), Tag::Int(*n))].into()),
            decode: |form| match form.get(&"n".into()) {
                Some(Tag::Int(n)) => Ok(*n),
                _ => Err(AttachmentError::Decode),
            },
        }),
        copy_on_death: false,
        sync_codec: None,
    };
    fn observe(map: &mut ChunkMap, index: &Index, dimension: DimensionKey) -> Arc<AtomicUsize> {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let level = LevelRef::new(WorldEpoch::new(42), dimension);
        map.observe_attachments(Arc::new(IndexedAttachmentObserver::new(
            index,
            move |holder| {
                counter.fetch_add(1, Ordering::Relaxed);
                match holder {
                    Holder::Chunk(pos) => Some(AttachmentHolder::Chunk(ChunkRef::new(level, pos))),
                    _ => None,
                }
            },
        )));
        calls
    }
    fn key() -> sunrise_mod_runtime::attachments::index::AttachmentKey {
        ("chunk_test".into(), "value".into())
    }
    fn saved_chunk() -> Arc<Chunk> {
        let mut source = Attachments::default();
        VALUE.set(&mut source, Holder::Chunk([3, 5]), 7).unwrap();
        let form = source.try_to_nbt().unwrap();
        let mut chunk = Chunk::empty(3, 5);
        chunk.saved = Some(Arc::new(crate::chunk::ChunkExtras {
            attachments: Attachments::from_nbt(form),
            ..Default::default()
        }));
        Arc::new(chunk)
    }

    #[test]
    fn chunk_attachment_residency_load_cow_unload_and_dimension_swap() {
        let index = Index::default();
        let other_index = Index::default();
        let mut map = ChunkMap::default();
        // Observer installation also handles chunks loaded before level-open.
        map.insert((3, 5), saved_chunk());
        let calls = observe(&mut map, &index, DimensionKey::OVERWORLD);
        let old = index.lock().unwrap().snapshot(&key());
        assert_eq!(old.len(), 1);
        assert_eq!(
            old[0].holder,
            AttachmentHolder::Chunk(ChunkRef::new(
                LevelRef::new(WorldEpoch::new(42), DimensionKey::OVERWORLD),
                [3, 5]
            ))
        );
        let save = map[&(3, 5)].clone();
        Arc::make_mut(map.get_mut(&(3, 5)).unwrap()).lit = true;
        crate::light::LightWorld::set_section_light(
            &mut map,
            3,
            0,
            5,
            crate::chunk::Light::Uniform(0),
            &mut crate::light::LightChanges::default(),
        );
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(Arc::ptr_eq(&old, &index.lock().unwrap().snapshot(&key())));
        VALUE
            .set(
                map.attachments_mut(&(3, 5)).unwrap(),
                Holder::Chunk([3, 5]),
                9,
            )
            .unwrap();
        let current = index.lock().unwrap().snapshot(&key());
        assert_eq!(current[0].entry.read(&VALUE), Ok(&9));
        assert_eq!(old[0].entry.read(&VALUE), Ok(&7));
        assert_eq!(
            save.saved
                .as_ref()
                .unwrap()
                .attachments
                .entry("chunk_test", "value")
                .unwrap()
                .unwrap()
                .read(&VALUE),
            Ok(&7)
        );
        assert!(map.attachments_mut(&(3, 5)).is_ok());
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        let current_save = map[&(3, 5)].clone();
        // Unrelated extras COW detaches the map; residency must not rely on it.
        let chunk = Arc::make_mut(map.get_mut(&(3, 5)).unwrap());
        Arc::make_mut(chunk.saved.as_mut().unwrap()).last_update = 20;
        let mut other = ChunkMap::default();
        observe(&mut other, &other_index, DimensionKey::NETHER);
        other.insert((3, 5), saved_chunk());
        std::mem::swap(&mut map, &mut other);
        map.remove(&(3, 5));
        assert!(other_index.lock().unwrap().snapshot(&key()).is_empty());
        assert_eq!(index.lock().unwrap().snapshot(&key()).len(), 1);
        other.remove(&(3, 5));
        assert!(index.lock().unwrap().snapshot(&key()).is_empty());
        assert_eq!(calls.load(Ordering::Relaxed), 3);
        assert_eq!(current[0].entry.read(&VALUE), Ok(&9));
        assert_eq!(
            current_save
                .saved
                .as_ref()
                .unwrap()
                .attachments
                .entry("chunk_test", "value")
                .unwrap()
                .unwrap()
                .read(&VALUE),
            Ok(&9)
        );
    }

    #[test]
    fn untouched_empty_chunks_do_not_register_before_attachment_access() {
        let index = Index::default();
        let mut map = ChunkMap::default();
        let calls = observe(&mut map, &index, DimensionKey::OVERWORLD);
        map.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let mut saved_empty = Chunk::empty(1, 0);
        saved_empty.saved = Some(Arc::default());
        map.insert((1, 0), Arc::new(saved_empty));
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        VALUE
            .set(
                map.attachments_mut(&(0, 0)).unwrap(),
                Holder::Chunk([0, 0]),
                5,
            )
            .unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        let batch = index.lock().unwrap().snapshot(&key());
        assert_eq!(batch[0].entry.read(&VALUE), Ok(&5));
    }

    #[test]
    fn chunk_replacement_clear_and_map_drop_retire_only_resident_rows() {
        let index = Index::default();
        let mut map = ChunkMap::default();
        observe(&mut map, &index, DimensionKey::OVERWORLD);
        map.insert((3, 5), saved_chunk());
        let retained = index.lock().unwrap().snapshot(&key());
        let mut replacement = Chunk::empty(3, 5);
        let mut extras = crate::chunk::ChunkExtras::default();
        VALUE
            .set(&mut extras.attachments, Holder::Chunk([3, 5]), 11)
            .unwrap();
        replacement.saved = Some(Arc::new(extras));
        map.insert((3, 5), Arc::new(replacement));
        let replaced = index.lock().unwrap().snapshot(&key());
        assert_eq!(replaced[0].entry.read(&VALUE), Ok(&11));
        map.insert((3, 5), Arc::new(Chunk::empty(3, 5)));
        assert!(index.lock().unwrap().snapshot(&key()).is_empty());
        map.insert((3, 5), saved_chunk());
        map.clear();
        assert!(index.lock().unwrap().snapshot(&key()).is_empty());
        map.insert((3, 5), saved_chunk());
        drop(map);
        assert!(index.lock().unwrap().snapshot(&key()).is_empty());
        assert_eq!(retained[0].entry.read(&VALUE), Ok(&7));
    }

    #[test]
    fn chunk_removal_clear_and_drop_release_mod_payloads_unlocked() {
        #[derive(Clone)]
        struct Probe {
            index: Weak<Mutex<AttachmentIndex<AttachmentHolder>>>,
            drops: Arc<Mutex<Vec<bool>>>,
        }
        impl Drop for Probe {
            fn drop(&mut self) {
                let unlocked = self.index.upgrade().unwrap().try_lock().is_ok();
                self.drops.lock().unwrap().push(unlocked);
            }
        }
        static PROBE: AttachmentType<Probe> = AttachmentType {
            namespace: "chunk_test",
            name: "value",
            default: || unreachable!(),
            codec: None,
            copy_on_death: false,
            sync_codec: None,
        };
        let index = Index::default();
        let drops = Arc::new(Mutex::new(Vec::new()));
        let payload = || {
            let mut chunk = Chunk::empty(3, 5);
            let mut extras = crate::chunk::ChunkExtras::default();
            PROBE
                .set(
                    &mut extras.attachments,
                    Holder::Chunk([3, 5]),
                    Probe {
                        index: Arc::downgrade(&index),
                        drops: drops.clone(),
                    },
                )
                .unwrap();
            chunk.saved = Some(Arc::new(extras));
            Arc::new(chunk)
        };
        let mut map = ChunkMap::default();
        observe(&mut map, &index, DimensionKey::OVERWORLD);
        map.insert((3, 5), payload());
        drop(map.remove(&(3, 5)));
        map.insert((3, 5), payload());
        map.clear();
        map.insert((3, 5), payload());
        drop(map);
        assert_eq!(*drops.lock().unwrap(), [true; 3]);
    }

    #[test]
    fn membership_tokens_track_keys_and_map_replacement_only() {
        let mut map = ChunkMap::default();
        let empty = map.membership_revision();
        map.clear();
        map.remove(&(0, 0));
        assert_eq!(empty, map.membership_revision());
        let chunk = Arc::new(Chunk::empty(0, 0));
        map.insert((0, 0), chunk.clone());
        let inserted = map.membership_revision();
        assert_ne!(empty, inserted);
        map.insert((0, 0), chunk);
        Arc::make_mut(map.get_mut(&(0, 0)).unwrap()).lit = true;
        for column in map.values_mut() {
            Arc::make_mut(column).light_loaded = true;
        }
        crate::light::LightWorld::set_section_light(
            &mut map,
            0,
            0,
            0,
            crate::chunk::Light::Uniform(0),
            &mut crate::light::LightChanges::default(),
        );
        assert_eq!(inserted, map.membership_revision());
        assert!(map.ordered_keys().eq([(0, 0)]), "replacement keeps one key");
        map.remove(&(0, 0));
        for (x, z) in [(1, -3), (-2, 4), (-2, -1)] {
            map.insert((x, z), Arc::new(Chunk::empty(x, z)));
        }
        assert!(map.ordered_keys().eq([(-2, -1), (-2, 4), (1, -3)]));
        assert_ne!(inserted, map.membership_revision());
        let replaced = map.membership_revision();
        map.clear();
        assert!(map.ordered_keys().next().is_none());
        assert_ne!(replaced, map.membership_revision());
        let cleared = map.membership_revision();
        map = ChunkMap::default();
        assert!(map.ordered_keys().next().is_none());
        assert_ne!(cleared, map.membership_revision());
    }

    /// The deltas' kinds, checking the ring answers in revision order.
    fn kinds(deltas: Vec<(u64, ChunkMembershipDelta)>) -> Vec<ChunkMembershipDelta> {
        let mut last = 0;
        for (rev, _) in &deltas {
            assert!(*rev > last, "the ring answers in revision order");
            last = *rev;
        }
        deltas.into_iter().map(|(_, d)| d).collect()
    }

    /// Revision numbers are process-wide: the ones another map takes in
    /// between, and a payload-only rewrite that takes none at all, say nothing
    /// about this map's keys.
    #[test]
    fn deltas_since_ignores_revisions_other_maps_take() {
        let mut map = ChunkMap::default();
        let mut other = ChunkMap::default();
        let chunk = Arc::new(Chunk::empty(0, 0));
        map.insert((0, 0), chunk.clone());
        let seen = map.membership_revision();
        for key in [(1, 0), (2, 0), (3, 0)] {
            other.insert(key, chunk.clone());
        }
        map.insert((0, 0), Arc::new(Chunk::empty(0, 0))); // Payload only.
        map.insert((4, 0), chunk.clone());
        map.remove(&(0, 0));
        assert_eq!(
            kinds(map.deltas_since(seen).unwrap().collect()),
            [
                ChunkMembershipDelta::Added((4, 0)),
                ChunkMembershipDelta::Removed((0, 0)),
            ]
        );
        // The consumer's own revision answers, with nothing new since.
        assert!(
            map.deltas_since(map.membership_revision())
                .unwrap()
                .next()
                .is_none()
        );
        // A revision another map took in between: refused, not replayed.
        assert!(map.deltas_since(other.membership_revision()).is_none());
    }

    /// A map built ahead of the live one, its revisions interleaved with it,
    /// answering for a revision it never took: refused, so the consumer
    /// rebuilds rather than replay another map's changes.
    #[test]
    fn deltas_since_none_for_interleaved_revision_from_another_map() {
        let mut map = ChunkMap::default();
        let mut other = ChunkMap::default();
        let chunk = Arc::new(Chunk::empty(0, 0));
        map.insert((0, 0), chunk.clone());
        let seen = map.membership_revision();
        other.insert((1, 0), chunk.clone());
        let foreign = other.membership_revision();
        map.insert((2, 0), chunk.clone());
        assert!(map.deltas_since(foreign).is_none());
        assert_eq!(
            kinds(map.deltas_since(seen).unwrap().collect()),
            [ChunkMembershipDelta::Added((2, 0))]
        );
    }

    /// The ring answers for the changes it kept, and only those.
    #[test]
    fn deltas_since_none_for_a_revision_the_ring_dropped() {
        let mut map = ChunkMap::default();
        let chunk = Arc::new(Chunk::empty(0, 0));
        let created = map.membership_revision();
        let mut revisions = Vec::new();
        let changes = MAX_RECENT_DELTAS as i32 + 1;
        for i in 0..changes {
            map.insert((i, 0), chunk.clone());
            revisions.push(map.membership_revision());
        }
        let (first, kept) = revisions.split_first().unwrap();
        // The oldest change fell out with the consumer's revision: rebuild.
        assert!(map.deltas_since(created).is_none());
        assert_eq!(map.deltas_since(*first).unwrap().count(), kept.len());
    }

    /// A reset rebuilds whatever the consumer last saw, and the ring answers
    /// for what follows it.
    #[test]
    fn deltas_since_none_across_a_clear() {
        let mut map = ChunkMap::default();
        let chunk = Arc::new(Chunk::empty(0, 0));
        map.insert((0, 0), chunk.clone());
        let before = map.membership_revision();
        map.clear();
        assert!(map.deltas_since(before).is_none());
        let cleared = map.membership_revision();
        map.insert((1, 0), chunk);
        assert_eq!(
            kinds(map.deltas_since(cleared).unwrap().collect()),
            [ChunkMembershipDelta::Added((1, 0))]
        );
    }

    #[test]
    fn fresh_chunks_tracking() {
        let mut map = ChunkMap::default();
        assert!(!map.has_fresh());
        let mut chunk = Chunk::empty(1, 2);
        chunk.fresh = true;
        map.insert((1, 2), Arc::new(chunk));
        assert!(map.has_fresh());
        assert_eq!(map.fresh_keys_ordered(), vec![(1, 2)]);

        map.remove_fresh(&(1, 2));
        assert!(!map.has_fresh());

        let mut chunk2 = Chunk::empty(3, 4);
        chunk2.fresh = true;
        map.insert((3, 4), Arc::new(chunk2));
        assert!(map.has_fresh());

        map.remove(&(3, 4));
        assert!(!map.has_fresh());
    }

    #[test]
    fn fresh_keys_ordered_matches_chunk_map_iteration_order() {
        let mut map = ChunkMap::default();
        let keys = [(-5, 10), (12, -3), (0, 0), (7, 8), (-1, -1), (4, 4)];
        for (i, &key) in keys.iter().enumerate() {
            let mut chunk = Chunk::empty(key.0, key.1);
            chunk.fresh = i % 2 == 0;
            map.insert(key, Arc::new(chunk));
        }

        let ordered = map.fresh_keys_ordered();
        let expected: Vec<Key> = map
            .keys()
            .copied()
            .filter(|k| map.fresh.contains(k))
            .collect();
        assert_eq!(ordered, expected);
        assert_eq!(ordered.len(), 3);
    }

    #[test]
    fn surrounded_follows_loads_unloads_and_clears() {
        // Brute force: the column and its eight neighbours are all resident.
        fn expected(map: &ChunkMap, (x, z): Key) -> bool {
            (-1..=1).all(|dz| (-1..=1).all(|dx| map.contains_key(&(x + dx, z + dz))))
        }
        let mut map = ChunkMap::default();
        let mut seed = 0x2545_f491_u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        for step in 0..4000 {
            let key = ((next() % 9) as i32 - 4, (next() % 9) as i32 - 4);
            if next() % 3 == 0 {
                map.remove(&key);
            } else {
                // Replacing a resident column changes nothing.
                map.insert(key, Arc::new(Chunk::empty(key.0, key.1)));
            }
            if step == 2500 {
                map.clear();
            }
            for x in -6..=6 {
                for z in -6..=6 {
                    assert_eq!(
                        map.surrounded((x, z)),
                        expected(&map, (x, z)),
                        "{x},{z} at {step}"
                    );
                }
            }
        }
    }
}
