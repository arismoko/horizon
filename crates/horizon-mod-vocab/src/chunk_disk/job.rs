//! Captured worker job and its once-only, recoverable completion.
use super::*;
use std::sync::OnceLock;

// no wire: engine-owned recovery snapshot and once-only worker completion.
/// One captured disk record. The persistence owner retains this until a durable
/// acknowledgement; callback completion alone does not permit dirty clearing.
// hidden: engine-owned save job, built and retained by the persistence owner
#[doc(hidden)]
#[non_exhaustive]
pub struct ChunkDiskJob {
    context: ChunkDiskContextWire,
    captured: Compound,
    completed: OnceLock<Result<Compound, ChunkDiskFailure>>,
    #[cfg(any(test, feature = "testing"))]
    installed: Option<&'static crate::runtime::Installed>,
    #[cfg(any(test, feature = "testing"))]
    parts: Option<&'static crate::parts::Installed>,
}

impl ChunkDiskJob {
    /// Capture encoded state only when the phase has a subscriber. The empty
    /// route does not clone the identity or store, allocate, or invoke a codec.
    pub fn capture(context: &ChunkDiskContextWire, store: &Attachments) -> Option<Self> {
        let hook = match context.phase {
            ChunkDiskPhase::Load => HookId::ChunkDataLoad,
            ChunkDiskPhase::Save => HookId::ChunkDataSave,
        };
        if !crate::listened(hook) {
            return None;
        }
        Some(Self {
            context: context.clone(),
            captured: store.to_nbt(),
            completed: OnceLock::new(),
            #[cfg(any(test, feature = "testing"))]
            installed: crate::runtime::installed(),
            #[cfg(any(test, feature = "testing"))]
            parts: crate::parts::installed(),
        })
    }

    /// Run once on the owning chunk worker. Repeated encoding or recovery
    /// reads borrow the cached outcome and cannot invoke listeners again.
    pub fn run(&self, resources: &dyn WorldgenHost) -> &Result<Compound, ChunkDiskFailure> {
        self.completed.get_or_init(|| {
            #[cfg(any(test, feature = "testing"))]
            let _scope = TestScope(
                crate::TEST_MODS.replace(self.installed),
                crate::parts::TEST_PARTS.replace(self.parts),
            );
            let mut data = ChunkDiskData::from_nbt(self.captured.clone())?;
            let context = OwningChunkDisk::new(&self.context, resources);
            match self.context.phase {
                ChunkDiskPhase::Load => native_chunk_data_load(&context, &mut data),
                ChunkDiskPhase::Save => native_chunk_data_save(&context, &mut data),
            }
            data.finish()
        })
    }

    /// Original encoded envelopes for explicit recovery after callback or IO
    /// failure. Reading these bytes never runs mod code or repeats callbacks.
    pub fn captured(&self) -> &Compound {
        &self.captured
    }

    /// Inspect completion without starting work on a reader or server thread.
    pub fn completed(&self) -> Option<&Result<Compound, ChunkDiskFailure>> {
        self.completed.get()
    }
}

#[cfg(any(test, feature = "testing"))]
struct TestScope(
    Option<&'static crate::runtime::Installed>,
    Option<&'static crate::parts::Installed>,
);
#[cfg(any(test, feature = "testing"))]
impl Drop for TestScope {
    fn drop(&mut self) {
        crate::TEST_MODS.set(self.0);
        crate::parts::TEST_PARTS.set(self.1);
    }
}
