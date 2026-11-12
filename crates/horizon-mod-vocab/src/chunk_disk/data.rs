//! The callback's typed store: only encoded values cross the worker boundary.
use super::*;

/// A job-local attachment transaction. No decoded value is shared with live state.
/// Use [`Self::get`] and [`Self::set`] with the ordinary typed attachment key.
/// Any refused operation poisons the job even if the listener ignores its error.
#[derive(Clone, Debug, PartialEq, horizon_mod_api_macros::Wire)]
#[wire(via = ChunkDiskDataWire)]
pub struct ChunkDiskData {
    entries: Compound,
    failure: Option<ChunkDiskFailure>,
}

/// Checked transport for a disk snapshot. Decoding validates all budgets before
/// accepting the store; a forged response cannot bypass the native checks.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, horizon_mod_api_macros::Wire)]
pub struct ChunkDiskDataWire {
    entries: Compound,
    failure: Option<ChunkDiskFailure>,
}

impl From<&ChunkDiskData> for ChunkDiskDataWire {
    fn from(data: &ChunkDiskData) -> Self {
        Self {
            entries: data.entries.clone(),
            failure: data.failure.clone(),
        }
    }
}
impl TryFrom<ChunkDiskDataWire> for ChunkDiskData {
    type Error = ChunkDiskFailure;
    fn try_from(data: ChunkDiskDataWire) -> Result<Self, Self::Error> {
        let mut result = Self::from_nbt(data.entries)?;
        result.failure = data.failure;
        Ok(result)
    }
}

impl ChunkDiskData {
    /// Detach the captured encoded envelopes, checking unknown keys as well.
    /// No attachment decoder or encoder runs during capture.
    pub fn from_nbt(entries: Compound) -> Result<Self, ChunkDiskFailure> {
        validate(&entries)?;
        Ok(Self {
            entries,
            failure: None,
        })
    }

    /// Read the job's typed value, inserting its default if absent. `T` must
    /// support the shared Wire contract on native and wasm alike.
    pub fn get<T: Clone + Send + Sync + wire::Wire + 'static>(
        &mut self,
        key: &AttachmentType<T>,
    ) -> Result<T, ChunkDiskFailure> {
        self.edit(key, |store| key.get(store, Holder::Level))
    }

    /// Replace only the snapshot's value. Failed encoding leaves the original
    /// envelopes intact and refuses publication of the entire job.
    pub fn set<T: Clone + Send + Sync + wire::Wire + 'static>(
        &mut self,
        key: &AttachmentType<T>,
        value: T,
    ) -> Result<(), ChunkDiskFailure> {
        self.edit(key, |store| key.set(store, Holder::Level, value))
    }

    /// Remove a snapshot binding through its typed codec.
    pub fn remove<T: Clone + Send + Sync + wire::Wire + 'static>(
        &mut self,
        key: &AttachmentType<T>,
    ) -> Result<Option<T>, ChunkDiskFailure> {
        self.edit(key, |store| key.remove(store, Holder::Level))
    }

    fn edit<T, R>(
        &mut self,
        key: &AttachmentType<T>,
        apply: impl FnOnce(&mut Attachments) -> Result<R, AttachmentError>,
    ) -> Result<R, ChunkDiskFailure> {
        if let Some(failure) = &self.failure {
            return Err(failure.clone());
        }
        let key_name = format!("{}:{}", key.namespace, key.name);
        let result = (|| {
            if key.codec.is_none() {
                return Err(failure(&key_name, "missing persistence codec"));
            }
            let mut store = Attachments::from_nbt(self.entries.clone());
            let value =
                apply(&mut store).map_err(|error| failure(&key_name, &format!("{error:?}")))?;
            let encoded = store.to_nbt();
            validate(&encoded)?;
            self.entries = encoded;
            Ok(value)
        })();
        if let Err(error) = &result {
            self.failure = Some(error.clone());
        }
        result
    }

    /// Run a nested local savepoint. An explicit abort restores the prior
    /// envelopes; a codec or budget refusal still poisons the outer job.
    pub fn transaction<R, E>(
        &mut self,
        apply: impl FnOnce(&mut Self) -> Result<R, E>,
    ) -> Result<R, E> {
        let previous = self.entries.clone();
        let result = apply(self);
        if result.is_err() {
            self.entries = previous;
        }
        result
    }

    /// The first refusal. A listener cannot clear it or publish tentative edits.
    pub fn failure(&self) -> Option<&ChunkDiskFailure> {
        self.failure.as_ref()
    }

    pub(crate) fn refuse(&mut self, error: ChunkDiskFailure) {
        if self.failure.is_none() {
            self.failure = Some(error);
        }
    }

    pub(crate) fn attribute_failure(&mut self, error: ChunkDiskFailure) {
        self.failure = Some(error);
    }

    /// Publish a fully successful snapshot, or return its structured refusal.
    pub fn finish(self) -> Result<Compound, ChunkDiskFailure> {
        match self.failure {
            Some(error) => Err(error),
            None => Ok(self.entries),
        }
    }
}

fn failure(key: &str, reason: &str) -> ChunkDiskFailure {
    ChunkDiskFailure {
        mod_id: String::new(),
        key: key.into(),
        reason: reason.into(),
    }
}

fn validate(entries: &Compound) -> Result<(), ChunkDiskFailure> {
    let mut total = wire::Writer::new();
    total.limit(1_048_576);
    // Treat the attachment root as an NBT tree so its internal nesting does not
    // consume Wire's smaller structural-record recursion budget.
    for (namespace, names) in entries {
        let Tag::Compound(names) = names else {
            return Err(failure(
                &namespace.to_string_lossy(),
                "invalid attachment namespace",
            ));
        };
        total.put(namespace);
        for (name, envelope) in names {
            let key = format!("{}:{}", namespace.to_string_lossy(), name.to_string_lossy());
            let Tag::Compound(envelope) = envelope else {
                return Err(failure(&key, "invalid persistence envelope"));
            };
            let Some(value) = envelope.get(&"value".into()) else {
                return Err(failure(&key, "missing encoded value"));
            };
            if !horizon_core::nbt::within_depth(value, 64) {
                return Err(failure(&key, "attachment depth exceeds 64"));
            }
            let mut encoded = wire::Writer::new();
            encoded.limit(65_536);
            encoded.put(value);
            if encoded.error().is_some() {
                return Err(failure(&key, "encoded attachment exceeds 65536 bytes"));
            }
            total.put(name);
            total.put(envelope);
            if total.error().is_some() {
                return Err(failure(&key, "chunk attachments exceed 1048576 bytes"));
            }
        }
    }
    Ok(())
}
