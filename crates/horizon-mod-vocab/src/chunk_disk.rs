//! Isolated attachment stores for chunk-worker disk callbacks.
use crate::*;
use std::sync::Arc;

#[path = "chunk_disk/data.rs"]
mod data;
pub use data::ChunkDiskData;

/// Which serialized chunk representation owns a disk job.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum ChunkDiskKind {
    /// A completed chunk.
    Full,
    /// A chunk still in generation.
    Proto,
}

/// The boundary at which a disk job runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum ChunkDiskPhase {
    /// After decoding, before residency publication.
    Load,
    /// After capture, before encoding.
    Save,
}

/// Identity captured by the existing world and chunk residency owner.
/// This value grants no live lookup or write authority.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct ChunkDiskContextWire {
    /// The existing world's lifetime, never a second chunk generation.
    pub world: WorldEpoch,
    /// The owning dimension's resource identifier.
    pub dimension: Identifier,
    /// Chunk coordinates, in chunks.
    pub pos: [i32; 2],
    /// Full or proto disk representation.
    pub kind: ChunkDiskKind,
    /// Load or save callback boundary.
    pub phase: ChunkDiskPhase,
}

impl ChunkDiskContextWire {
    /// Capture the existing world identity and the disk record's coordinates.
    pub fn new(
        world: WorldEpoch,
        dimension: Identifier,
        pos: [i32; 2],
        kind: ChunkDiskKind,
        phase: ChunkDiskPhase,
    ) -> Self {
        Self {
            world,
            dimension,
            pos,
            kind,
            phase,
        }
    }
}

/// One opaque saved binding. Unknown keys retain this envelope unchanged.
#[derive(Clone, Debug, PartialEq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct AttachmentDiskEntryWire {
    /// Registered attachment namespace.
    pub namespace: String,
    /// Registered name within the namespace.
    pub name: String,
    /// Schema of this envelope. The current attachment codec uses schema zero.
    pub schema: u32,
    /// The persistence envelope, including the encoded value and its policies.
    pub encoded: Compound,
}

/// Why a chunk disk job refused, with the failing mod and attachment when known.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct ChunkDiskFailure {
    /// The listener's owner; empty for input validation before dispatch.
    pub mod_id: String,
    /// The attachment key, or empty for a failure outside a typed operation.
    pub key: String,
    /// The failed operation or missing capability.
    pub reason: String,
}

/// A worker's publication decision. Refusal never contains a partial store.
#[derive(Clone, Debug, PartialEq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum ChunkDiskResultWire {
    /// All listeners and budget checks succeeded.
    Success(Vec<AttachmentDiskEntryWire>),
    /// Keep the original captured store and the old disk record.
    Refused(ChunkDiskFailure),
}

/// A disk callback's shared Host. It has no live backend and is never a
/// `ServerHost`: live handle methods refuse, and world writes do not compile.
/// Registry and settings reads use the owning resource generation's snapshot.
pub trait ChunkDiskHost: Host {
    /// Captured world, dimension, chunk, representation and phase.
    fn job(&self) -> ChunkDiskContextWire;
    /// Immutable registry names in ID order.
    fn registry(&self, name: &str) -> Option<Arc<[String]>>;
}

// no wire: a borrowed worker-only Host, carried by a synchronous Wire frame.
/// Engine-side capability lasting only for one callback dispatch.
#[doc(hidden)]
pub struct OwningChunkDisk<'a> {
    job: &'a ChunkDiskContextWire,
    resources: &'a dyn WorldgenHost,
}

impl<'a> OwningChunkDisk<'a> {
    /// Bind the captured identity to immutable resource-generation reads.
    pub fn new(job: &'a ChunkDiskContextWire, resources: &'a dyn WorldgenHost) -> Self {
        Self { job, resources }
    }
}

impl Host for OwningChunkDisk<'_> {}
crate::impl_host_areas!(OwningChunkDisk<'_>, except(levels, config));
impl prelude::LevelsHost for OwningChunkDisk<'_> {
    fn is_loaded(&self, id: &str) -> bool {
        self.resources.mods().get(id).is_some()
    }
    fn chunk(&self, _: LevelRef, _: [i32; 2]) -> Option<ChunkRef> {
        None
    }
}
impl prelude::ConfigHost for OwningChunkDisk<'_> {
    fn setting(&self, mod_id: &str, key: &str) -> Option<String> {
        self.resources.setting(mod_id, key)
    }
}
impl ChunkDiskHost for OwningChunkDisk<'_> {
    fn job(&self) -> ChunkDiskContextWire {
        self.job.clone()
    }
    fn registry(&self, name: &str) -> Option<Arc<[String]>> {
        self.resources.registry(name)
    }
}

/// The worker's load entry point, discovered by the hook catalog generator.
#[doc(hidden)]
#[inline(never)]
pub fn native_chunk_data_load(context: &OwningChunkDisk<'_>, data: &mut ChunkDiskData) {
    crate::dispatch_chunk_data_load(context, data);
}

/// The worker's save entry point, discovered by the hook catalog generator.
#[doc(hidden)]
#[inline(never)]
pub fn native_chunk_data_save(context: &OwningChunkDisk<'_>, data: &mut ChunkDiskData) {
    crate::dispatch_chunk_data_save(context, data);
}

#[path = "chunk_disk/job.rs"]
mod job;
#[doc(hidden)]
pub use job::ChunkDiskJob;

#[cfg(test)]
#[path = "chunk_disk/tests.rs"]
mod tests;

/// Fence native unwinds and synchronous guest failures before publication.
pub(crate) fn invoke(mod_id: &str, call: impl FnOnce()) -> Result<(), ChunkDiskFailure> {
    let failures = horizon_mod_runtime::dispatch_failures::dispatch_failure_counter();
    let before = failures.get();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(call));
    let reason = if result.is_err() {
        Some("native listener unwound")
    } else if before != failures.get() {
        Some("wasm listener trapped or its Wire response was refused")
    } else {
        None
    };
    match reason {
        None => Ok(()),
        Some(reason) => Err(ChunkDiskFailure {
            mod_id: mod_id.into(),
            key: String::new(),
            reason: reason.into(),
        }),
    }
}
