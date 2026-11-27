//! Per-level, generation-bound contents observers. Lookup visits only watches of the
//! changed source; the reverse index retires only watches owned by a removed observer.
use super::{BlockEntityIdentity, Pos};
use crate::fast_hash::FxHashMap;
use std::sync::Arc;

#[derive(Clone, Copy)]
struct Watch {
    observer: BlockEntityIdentity,
    order: u64,
}

/// Runtime subscriptions, never saved or copied into a reloaded block entity.
#[derive(Default)]
pub struct ContentsWatches {
    sources: FxHashMap<Pos, Arc<Vec<Watch>>>,
    observers: FxHashMap<Pos, Vec<(Pos, Watch)>>,
    next_order: u64,
}

/// A notification's frozen membership; an absent source needs no allocation.
pub struct ContentsObservers(Option<Arc<Vec<Watch>>>);

impl ContentsObservers {
    /// Visit only this source's registered observers, in registration order.
    pub fn iter(&self) -> impl Iterator<Item = BlockEntityIdentity> + '_ {
        self.0
            .iter()
            .flat_map(|watches| watches.iter().map(|watch| watch.observer))
    }
    /// Whether this source has no watches.
    pub fn is_empty(&self) -> bool {
        self.0.as_ref().is_none_or(|watches| watches.is_empty())
    }
}

impl ContentsWatches {
    /// A stable, allocation-free snapshot for one notification. Registration
    /// changes use copy-on-write, so callbacks affect only later notices.
    pub fn observers(&self, source: Pos) -> ContentsObservers {
        ContentsObservers(self.sources.get(&source).cloned())
    }

    /// The before-image of one observer, including its original dispatch order.
    pub fn snapshot(&self, observer: BlockEntityIdentity) -> Vec<(Pos, u64)> {
        let pos = tuple(observer.position);
        self.observers.get(&pos).map_or_else(Vec::new, |watches| {
            watches
                .iter()
                .filter(|(_, watch)| watch.observer == observer)
                .map(|(source, watch)| (*source, watch.order))
                .collect()
        })
    }

    /// Register idempotently. A self-watch is already covered by source dispatch.
    pub fn set(&mut self, observer: BlockEntityIdentity, source: Pos, enabled: bool) {
        if tuple(observer.position) == source {
            return;
        }
        let pos = tuple(observer.position);
        if enabled {
            if self.observers.get(&pos).is_some_and(|watches| {
                watches
                    .iter()
                    .any(|(at, watch)| *at == source && watch.observer == observer)
            }) {
                return;
            }
            let order = self.next_order;
            self.next_order = self.next_order.wrapping_add(1);
            self.insert(source, Watch { observer, order });
        } else {
            if let Some(watches) = self.observers.get_mut(&pos) {
                watches.retain(|(at, watch)| *at != source || watch.observer != observer);
                if watches.is_empty() {
                    self.observers.remove(&pos);
                }
            }
            if let Some(watches) = self.sources.get_mut(&source) {
                Arc::make_mut(watches).retain(|watch| watch.observer != observer);
                if watches.is_empty() {
                    self.sources.remove(&source);
                }
            }
        }
    }

    fn insert(&mut self, source: Pos, watch: Watch) {
        let watches = self.sources.entry(source).or_default();
        let at = watches.partition_point(|previous| previous.order < watch.order);
        Arc::make_mut(watches).insert(at, watch);
        self.observers
            .entry(tuple(watch.observer.position))
            .or_default()
            .push((source, watch));
    }

    /// Retire an observer on removal, replacement or chunk unload. The source's
    /// subscriptions remain positional: unloading the source does not remove observers.
    pub(super) fn remove(&mut self, pos: Pos) {
        for (source, removed) in self.observers.remove(&pos).into_iter().flatten() {
            if let Some(watches) = self.sources.get_mut(&source) {
                Arc::make_mut(watches).retain(|watch| watch.observer != removed.observer);
                if watches.is_empty() {
                    self.sources.remove(&source);
                }
            }
        }
    }

    /// Restore a journal before-image only after its observer's identity is restored.
    pub fn restore(&mut self, observer: BlockEntityIdentity, kept: Vec<(Pos, u64)>) {
        self.remove(tuple(observer.position));
        for (source, order) in kept {
            self.insert(source, Watch { observer, order });
        }
    }
}

fn tuple([x, y, z]: [i32; 3]) -> Pos {
    (x, y, z)
}
