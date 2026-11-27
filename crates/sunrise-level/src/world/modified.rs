//! `LevelChunk.markUnsaved` and `ChunkMap`'s insertion-ordered eager queue.
use crate::fast_hash::{FxHashMap, FxHashSet};
use std::{
    collections::BTreeMap,
    ops::{Bound, Deref},
    time::{Duration, Instant},
};

type Key = (i32, i32);
const COOLDOWN: Duration = Duration::from_millis(10_000);

/// A chunk's place in `ModifiedChunks` before a tentative write marked it: what a transaction
/// keeps to put the mark back if it aborts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkMark {
    dirty: bool,
    queued: Option<u64>,
}

#[derive(Default)]
pub struct ModifiedChunks {
    dirty: FxHashSet<Key>,
    // Ordered independently of the dirty set: an autosave leaves queue entries
    // in place until the eager loop visits them, including cooled-down ones.
    order: BTreeMap<u64, Key>,
    queued: FxHashMap<Key, u64>,
    sequence: u64,
    next_save: FxHashMap<Key, Instant>,
}
impl Deref for ModifiedChunks {
    type Target = FxHashSet<Key>;
    fn deref(&self) -> &Self::Target {
        &self.dirty
    }
}
impl IntoIterator for ModifiedChunks {
    type Item = Key;
    type IntoIter = std::collections::hash_set::IntoIter<Key>;
    fn into_iter(self) -> Self::IntoIter {
        self.dirty.into_iter()
    }
}
impl ModifiedChunks {
    pub fn insert(&mut self, key: Key) -> bool {
        let changed = self.dirty.insert(key);
        if changed && !self.queued.contains_key(&key) {
            self.sequence += 1;
            self.order.insert(self.sequence, key);
            self.queued.insert(key, self.sequence);
        }
        changed
    }
    pub fn extend(&mut self, keys: impl IntoIterator<Item = Key>) {
        for key in keys {
            self.insert(key);
        }
    }
    /// Where `key` stands now, dirty and queued or not, for `restore`.
    pub fn mark(&self, key: Key) -> ChunkMark {
        ChunkMark {
            dirty: self.dirty.contains(&key),
            queued: self.queued.get(&key).copied(),
        }
    }
    /// Puts back what `mark` kept of `key`: a chunk that was clean is clean again, and a queue
    /// entry made since goes. A chunk that was dirty stays as it is now, since only a save
    /// clears a mark, and a save then saw what it saved.
    pub fn restore(&mut self, key: Key, mark: ChunkMark) {
        if !mark.dirty {
            self.dirty.remove(&key);
        }
        if mark.queued.is_none()
            && let Some(sequence) = self.queued.remove(&key)
        {
            self.order.remove(&sequence);
        }
    }
    pub fn remove(&mut self, key: &Key) -> bool {
        self.dirty.remove(key)
    }
    pub fn drain(&mut self) -> impl Iterator<Item = Key> + '_ {
        self.dirty.drain()
    }
    /// Like `drain`: queue entries of chunks no longer dirty go at the next eager pass.
    pub fn clear(&mut self) {
        self.dirty.clear();
    }
    /// `ChunkMap.scheduleUnload`: forget the cooldown, including clean chunks.
    pub fn unloaded(&mut self, key: &Key) {
        self.next_save.remove(key);
    }
    /// `ChunkMap.saveAllChunks(false)`: clear cooldowns, then save all dirty
    /// full chunks without a limit, starting a fresh cooldown for each write.
    pub fn autosaved(&mut self, keys: &[Key], now: Instant) {
        self.next_save.clear();
        self.next_save
            .extend(keys.iter().map(|&key| (key, now + COOLDOWN)));
    }
    /// `ChunkMap.saveChunksEagerly` / `saveChunkIfNeeded`. `available` checks
    /// both haveTime and activeChunkWrites before each candidate. `loaded` returns
    /// `None` for an absent chunk, `Some(true)` for a lit chunk eligible to save.
    /// Consume the mark before `save` captures the chunk; any new edit queues it again.
    pub fn save_eagerly(
        &mut self,
        now: Instant,
        mut available: impl FnMut() -> bool,
        mut loaded: impl FnMut(Key) -> Option<bool>,
        mut save: impl FnMut(Key, &mut Self) -> bool,
    ) {
        let mut saved = 0;
        let mut after = Bound::Unbounded;
        let last = self.sequence;
        let mut stale = Vec::new();
        while saved < 20 {
            let mut ready = None;
            for (&sequence, &key) in self.order.range((after, Bound::Included(last))) {
                if !available() {
                    break;
                }
                match loaded(key) {
                    None => stale.push((sequence, key)),
                    _ if !self.dirty.contains(&key) => stale.push((sequence, key)),
                    Some(true) if self.next_save.get(&key).is_none_or(|&at| now >= at) => {
                        ready = Some((sequence, key));
                        break;
                    }
                    Some(_) => (),
                }
            }
            for (sequence, key) in stale.drain(..) {
                self.order.remove(&sequence);
                self.queued.remove(&key);
            }
            let Some((sequence, key)) = ready else { break };
            after = Bound::Excluded(sequence);
            self.order.remove(&sequence);
            self.queued.remove(&key);
            self.dirty.remove(&key);
            if save(key, self) {
                self.next_save.insert(key, now + COOLDOWN);
                saved += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restored_mark_takes_back_the_dirty_flag_and_queue_entry_a_write_made() {
        let mut modified = ModifiedChunks::default();
        modified.insert((1, 1));
        let clean = modified.mark((0, 0));
        let dirty = modified.mark((1, 1));
        modified.insert((0, 0));
        modified.insert((1, 1));
        modified.restore((0, 0), clean);
        modified.restore((1, 1), dirty);
        assert!(!modified.contains(&(0, 0)));
        assert!(modified.contains(&(1, 1)));
        assert_eq!(modified.mark((0, 0)), clean);
        assert_eq!(modified.mark((1, 1)), dirty);
        let mut saved = Vec::new();
        modified.save_eagerly(
            Instant::now(),
            || true,
            |_| Some(true),
            |key, _| {
                saved.push(key);
                true
            },
        );
        assert_eq!(saved, [(1, 1)]);
    }

    #[test]
    fn an_edit_during_an_eager_save_stays_dirty() {
        let mut modified = ModifiedChunks::default();
        let key = (3, 4);
        let now = Instant::now();
        modified.insert(key);
        modified.save_eagerly(
            now,
            || true,
            |_| Some(true),
            |saving, marks| {
                assert_eq!(saving, key);
                assert!(
                    marks.insert(key),
                    "the old mark was consumed before capture"
                );
                true
            },
        );
        assert!(modified.contains(&key));
        let mut resaved = Vec::new();
        modified.save_eagerly(
            now + COOLDOWN,
            || true,
            |_| Some(true),
            |saving, _| {
                resaved.push(saving);
                true
            },
        );
        assert_eq!(resaved, [key]);
        assert!(!modified.contains(&key));
    }
}
