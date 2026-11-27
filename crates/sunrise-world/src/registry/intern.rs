//! Retain numeric snapshots without retaining another copy of equal imported data.
use super::{BlockRegistry, data::Data};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
};

/// Worker resources built for one payload, keyed by what the owner derived them from.
pub(super) type SharedWorkers = Arc<Mutex<Vec<(usize, WorkerResource)>>>;
type WorkerResource = Arc<dyn std::any::Any + Send + Sync>;

struct Snapshot {
    data: Arc<Data>,
    workers: SharedWorkers,
    installations: HashMap<usize, &'static BlockRegistry>,
}

// Mod-list identities are never recycled: the owning installation is process-lived.
// Separate mod lists never share a payload, even when their numeric rows happen to match.
static SNAPSHOTS: LazyLock<Mutex<HashMap<usize, Vec<Snapshot>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

impl BlockRegistry {
    /// The worker resource already shared under `owner`, else retain `resource` as it.
    ///
    /// Registries interned from equal payloads share one pool, so a new installation whose
    /// resource derives from the same `owner` key reuses the first instead of keeping another.
    /// The caller guarantees equal keys build equal resources from equal payloads.
    #[doc(hidden)]
    pub fn share_worker_resource(&self, owner: usize, resource: WorkerResource) -> WorkerResource {
        let mut pool = self
            .shared_workers
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some((_, shared)) = pool.iter().find(|(key, _)| *key == owner) {
            return shared.clone();
        }
        pool.push((owner, resource.clone()));
        resource
    }

    /// Names this registry's imported payload, which every registry interned from equal
    /// data shares. A key for caches derived from the data alone: unlike the registry's
    /// address it is equal across installations. The payload outlives the key's use because
    /// the intern table retains it for the process lifetime.
    #[doc(hidden)]
    pub fn payload_identity(&self) -> usize {
        Arc::as_ptr(&self.data) as usize
    }

    /// Retain a registry for an immutable host installation.
    ///
    /// This is host plumbing, not a mod API. `mods` identifies the retained mod list;
    /// `parts` identifies its immutable Parts installation. Without a Parts identity,
    /// a future metadata owner is unknown, so only imported data may be shared.
    /// Saved-state caches remain attached to each registry wrapper; equal payloads may share worker resources under equal owner keys.
    #[doc(hidden)]
    pub fn intern(self, (mods, parts): (usize, Option<usize>)) -> &'static Self {
        let mut snapshots = SNAPSHOTS.lock().unwrap_or_else(|error| error.into_inner());
        let snapshots = snapshots.entry(mods).or_default();
        let snapshot = if let Some(index) = snapshots
            .iter()
            .position(|snapshot| *snapshot.data == *self.data)
        {
            &mut snapshots[index]
        } else {
            snapshots.push(Snapshot {
                data: self.data.clone(),
                workers: SharedWorkers::default(),
                installations: HashMap::new(),
            });
            snapshots
                .last_mut()
                .expect("just inserted registry snapshot")
        };
        if let Some(parts) = parts
            && let Some(&registry) = snapshot.installations.get(&parts)
        {
            return registry;
        }
        let mut registry = self;
        registry.data = snapshot.data.clone();
        registry.shared_workers = snapshot.workers.clone();
        let registry = Box::leak(Box::new(registry));
        if let Some(parts) = parts {
            snapshot.installations.insert(parts, registry);
        }
        registry
    }

    /// Tiny leaked wrapper sharing `self`'s immutable payload with fresh worker metadata slots.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn fork_wrapper(&self) -> &'static Self {
        Box::leak(Box::new(Self {
            mod_worker_resource: std::sync::OnceLock::new(),
            structure_block_entity_transform: std::sync::OnceLock::new(),
            data: self.data.clone(),
            shared_workers: self.shared_workers.clone(),
            saved_states: std::sync::RwLock::new(sunrise_core::fast_hash::FxHashMap::default()),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::BlockStateId;

    fn registry(default: u32, reversed: bool) -> BlockRegistry {
        let states = if reversed {
            r#"[["",false,false,"",{"id":"test:lamp","properties":{"lit":"true"}}],["",false,false,"",{"id":"test:lamp","properties":{"lit":"false"}}]]"#
        } else {
            r#"[["",false,false,"",{"id":"test:lamp","properties":{"lit":"false"}}],["",false,false,"",{"id":"test:lamp","properties":{"lit":"true"}}]]"#
        };
        BlockRegistry::from_facts_json(
            format!(r#"{{"defaults":{{"test:lamp":{default}}},"states":{states}}}"#).as_bytes(),
        )
        .unwrap()
    }

    #[test]
    fn same_installation_reuses_wrapper_but_new_epoch_only_shares_data() {
        let first = registry(0, false).intern((101, Some(1)));
        assert!(first.mod_worker_resource.set(Arc::new(42_u32)).is_ok());
        let same = registry(0, false).intern((101, Some(1)));
        assert!(std::ptr::eq(first, same));
        assert_eq!(
            same.mod_worker_resource
                .get()
                .unwrap()
                .downcast_ref::<u32>(),
            Some(&42)
        );
        let next = registry(0, false).intern((101, Some(2)));
        assert!(!std::ptr::eq(first, next));
        assert!(Arc::ptr_eq(&first.data, &next.data));
        assert!(next.mod_worker_resource.get().is_none());
    }

    #[test]
    fn equal_payloads_share_worker_resources_by_owner_and_other_payloads_do_not() {
        let first = registry(0, false).intern((105, Some(1)));
        let next = registry(0, false).intern((105, Some(2)));
        let other = registry(1, true).intern((105, Some(1)));
        let make = |value: u32| -> WorkerResource { Arc::new(value) };
        let kept = first.share_worker_resource(7, make(1));
        let reused = next.share_worker_resource(7, make(2));
        assert!(Arc::ptr_eq(&kept, &reused));
        assert_eq!(reused.downcast_ref::<u32>(), Some(&1));
        let other_owner = next.share_worker_resource(8, make(3));
        assert!(!Arc::ptr_eq(&kept, &other_owner));
        let separate = other.share_worker_resource(7, make(4));
        assert_eq!(separate.downcast_ref::<u32>(), Some(&4));
    }

    #[test]
    fn different_mods_order_and_defaults_keep_distinct_payloads() {
        let first = registry(0, false).intern((102, Some(1)));
        let other_mods = registry(0, false).intern((103, Some(1)));
        let reordered = registry(1, true).intern((102, Some(1)));
        let defaults = registry(1, false).intern((102, Some(1)));
        for other in [other_mods, reordered, defaults] {
            assert!(!std::ptr::eq(first, other));
            assert!(!Arc::ptr_eq(&first.data, &other.data));
        }
        assert_eq!(first.default_state("test:lamp"), Some(BlockStateId(0)));
        assert_eq!(defaults.default_state("test:lamp"), Some(BlockStateId(1)));
        assert_eq!(
            first
                .state(BlockStateId(0))
                .unwrap()
                .properties
                .get("lit")
                .unwrap(),
            "false"
        );
        assert_eq!(
            reordered
                .state(BlockStateId(0))
                .unwrap()
                .properties
                .get("lit")
                .unwrap(),
            "true"
        );
    }

    #[test]
    fn unknown_installations_never_share_worker_owners() {
        let first = registry(0, false).intern((104, None));
        let next = registry(0, false).intern((104, None));
        assert!(!std::ptr::eq(first, next));
        assert!(Arc::ptr_eq(&first.data, &next.data));
    }
}
