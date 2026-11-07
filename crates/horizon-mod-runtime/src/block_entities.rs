//! Shared revision identities for engine block entities and their API snapshots.
use std::sync::atomic::{AtomicU64, Ordering};

static MADE: AtomicU64 = AtomicU64::new(0);

/// Allocate the identity portion of a newly constructed mod block entity's revision.
/// Snapshots copy their source identity and must not call this allocator.
pub fn new_identity() -> u64 {
    MADE.fetch_add(1, Ordering::Relaxed)
}
