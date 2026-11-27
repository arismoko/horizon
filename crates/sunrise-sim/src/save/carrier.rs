//! Transactional carrier ownership store: slice 2, part A (issue #2413).
//!
//! The store owns the world's carrier files flat in the existing `data/`
//! directory — no new folders are ever created:
//! - `sunrise_carriers.journal.{0,1}`: the two-slot journal;
//! - `sunrise_carrier_op_<id>.nbt`: one operation file per in-flight
//!   transfer;
//! - `sunrise_carrier_<uuid>-<gen>.nbt`: one file per live carrier.
//!
//! Every file is published whole through
//! [`sunrise_mod_package::durable`]: a synced temporary, then a
//! write-through move on Windows or `rename` plus a directory `fsync` on
//! Unix. The journal holds `next_op_id`, the index of live carriers, at
//! most one in-progress operation and tombstones, each record numbered
//! and CRC-protected; a new record always goes to the slot that does not
//! hold the newest valid one, and the initialized journal is never
//! deleted, so at least one valid slot survives every single-step crash.
//!
//! Nothing exists before the first explicit transfer: `first_use` writes
//! the `sunrise:carriers_in_use` flag into `level.dat` first (its
//! replacement gap rolls back to an untouched world) and slot 0 second,
//! and a world that never assembles a carrier gains no file and changes
//! no byte. Opening follows the agreed table: no slots and no flag means
//! never used; a flag, or leftover op/carrier files, without slots
//! refuses the load and names the missing store; slots whose newest
//! valid record is from a newer schema refuse too.
//!
//! This is the storage layer only: nothing here is wired into gameplay,
//! `Game` or any public API (that is part B and slice 9).

use crate::save::{read_dat, write_dat};
use std::{
    fmt, fs,
    path::{Path, PathBuf},
};
use sunrise_core::{
    crc32::Crc32,
    nbt::{self, JavaString, Limits, NamedTag, Tag},
};
use sunrise_mod_package::durable;
use sunrise_world::compression::{Compression, decode_single_gzip_member};

/// The journal slot file names, in slot order.
pub const SLOTS: [&str; 2] = ["sunrise_carriers.journal.0", "sunrise_carriers.journal.1"];
/// The `level.dat` key marking a world whose carriers exist.
pub const IN_USE_FLAG: &str = "sunrise:carriers_in_use";
/// A whole slot file, header included, is at most this big.
pub const MAX_SLOT_FILE: usize = 1024 * 1024;
/// At most this many live carriers are indexed.
pub const MAX_CARRIERS: usize = 4096;
/// At most this many tombstones are pending.
pub const MAX_TOMBSTONES: usize = 4096;
/// An operation file is at most this big compressed, on disk.
pub const MAX_OP_COMPRESSED: usize = 32 * 1024 * 1024;
/// An operation file is at most this big decoded.
pub const MAX_OP_DECODED: usize = 40 * 1024 * 1024;
/// An operation file's NBT nests at most this deep (vanilla's own bound).
pub const MAX_OP_DEPTH: usize = 512;
/// An operation touches at most this many cells (two images each).
pub const MAX_OP_CELLS: usize = 4096;
/// One cell's block-entity image is at most this big serialized.
pub const MAX_CELL_ENTITY: usize = 64 * 1024;
/// All block-entity images of one operation side total at most this much.
pub const MAX_OP_ENTITIES: usize = 16 * 1024 * 1024;
/// The slot header: magic, version, record number, body length, CRC.
const SLOT_HEADER: usize = 4 + 2 + 8 + 4 + 4;
/// The slot magic.
const SLOT_MAGIC: [u8; 4] = *b"HCJS";
/// The slot schema this build writes.
const SLOT_VERSION: u16 = 1;
/// The operation-file schema this build writes.
const OP_VERSION: i32 = 1;
/// The node budget for one operation file: the exact schema shape never
/// needs more than a handful of nodes per cell.
const MAX_OP_NODES: usize = 10 * MAX_OP_CELLS;

/// Why a world's carrier store refused to open. Every refusal names the
/// store and never lets load continue on a guess.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The flag, or leftover carrier files, say the world has carriers,
    /// but no valid journal slot survives. The world must be restored
    /// from a coherent whole-world backup.
    MissingStore { flag: bool },
    /// Slots exist and none of them holds a valid record.
    NoValidSlot,
    /// The newest valid slot was written by a newer schema.
    NewerSchema { found: u16 },
    /// Reading the store failed on I/O.
    Io(String),
}
impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingStore { flag } => write!(
                f,
                "the carrier store is missing (flag: {flag}): journal slots absent while the \
                 world holds carrier data; restore the world from a whole-world backup"
            ),
            Self::NoValidSlot => {
                write!(f, "no valid carrier journal slot survives")
            }
            Self::NewerSchema { found } => write!(
                f,
                "carrier journal schema {found} is newer than this build's {SLOT_VERSION}"
            ),
            Self::Io(error) => write!(f, "carrier store I/O: {error}"),
        }
    }
}

/// The transfer kind an operation file and journal record name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpKind {
    /// World cells become carrier cells.
    Assemble,
    /// Carrier cells return to the world.
    Disassemble,
}
impl OpKind {
    /// The wire byte.
    const fn as_byte(self) -> u8 {
        match self {
            Self::Assemble => 0,
            Self::Disassemble => 1,
        }
    }
    /// The wire byte, decoded.
    const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Assemble),
            1 => Some(Self::Disassemble),
            _ => None,
        }
    }
    /// The operation file's kind string.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Assemble => "assemble",
            Self::Disassemble => "disassemble",
        }
    }
}

/// The one operation a journal may hold in progress.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InProgress {
    /// The operation's id, from `next_op_id`.
    pub op_id: u64,
    /// Whether it assembles or disassembles.
    pub kind: OpKind,
    /// The carrier it moves.
    pub uuid: u128,
    /// The carrier's generation after this operation.
    pub generation: u32,
    /// The dimension the operation runs in ('"minecraft:overworld"'), so
    /// recovery enters that level and no other.
    pub dimension: String,
}

/// The journal body: what the newest valid slot holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Journal {
    /// The next operation id to hand out; only grows.
    pub next_op_id: u64,
    /// The one operation that may be in flight, if any.
    pub in_progress: Option<InProgress>,
    /// Carriers whose file must not come back to life after a deletion
    /// that may not be durable yet.
    pub tombstones: Vec<(u128, u32)>,
    /// The live carriers, `(uuid, generation)` per entry.
    pub carriers: Vec<(u128, u32)>,
}
impl Journal {
    /// A fresh, empty journal handing out operation ids from one.
    pub const fn empty() -> Self {
        Self {
            next_op_id: 1,
            in_progress: None,
            tombstones: Vec::new(),
            carriers: Vec::new(),
        }
    }
    /// Whether the index lists this carrier.
    pub fn lists(&self, uuid: u128, generation: u32) -> bool {
        self.carriers.contains(&(uuid, generation))
    }
    /// Whether this carrier is tombstoned.
    pub fn tombstoned(&self, uuid: u128, generation: u32) -> bool {
        self.tombstones.contains(&(uuid, generation))
    }
}

/// An opened store: `NeverUsed` when the world has no carriers, or the
/// live journal.
#[derive(Debug)]
pub enum Opened {
    /// No slots, no flag, no files: the world never used carriers.
    NeverUsed,
    /// A valid journal rules the store.
    Live(Store),
}

/// The live store. Every mutation writes the next slot durably before it
/// returns, so a crash always leaves the previous record intact.
#[derive(Debug)]
pub struct Store {
    data: PathBuf,
    /// The newest valid record's number, and the slot it lives in.
    entry_no: u64,
    last_slot: usize,
    journal: Journal,
    /// The index-capacity bound this store enforces; the default
    /// [`MAX_CARRIERS`], smaller in tests to exercise the overflow
    /// paths in seconds.
    capacity: usize,
    /// The tombstones whose carrier files were already absent when the
    /// store opened. Only those may clear: a file deleted during this
    /// session may not be durably deleted yet on Windows, where the
    /// directory entry cannot be synced, so its tombstone holds until a
    /// later open finds the file gone.
    tombstones_absent_at_open: Vec<(u128, u32)>,
}
impl Store {
    /// Eligibility captured at the actual world open, shared by all transfers.
    pub fn retention_snapshot(&self) -> Vec<(u128, u32)> {
        self.tombstones_absent_at_open.clone()
    }

    /// Replace the constructor's disk sample with the world-session snapshot.
    pub fn use_retention_snapshot(&mut self, snapshot: &[(u128, u32)]) {
        self.tombstones_absent_at_open = snapshot.to_vec();
    }

    /// Reads the world's store and applies the opening table: slot
    /// presence, the `level.dat` flag, leftover carrier files and slot
    /// validity decide between [`Opened::NeverUsed`], [`Opened::Live`]
    /// and a typed [`Refusal`]. Nothing is written or cleaned here.
    pub fn open(world_root: &Path) -> Result<Opened, Refusal> {
        Self::open_with_capacity(world_root, MAX_CARRIERS)
    }

    /// [`Self::open`] with an explicit index capacity (tests).
    pub fn open_with_capacity(world_root: &Path, capacity: usize) -> Result<Opened, Refusal> {
        let data = world_root.join("data");
        let flag = in_use_flag(world_root);
        let Some(records) = read_slots(&data)? else {
            if residual_files(&data)? {
                // Leftover carrier files without a journal are a missing
                // store whatever the flag says.
                return Err(Refusal::MissingStore { flag });
            }
            if flag {
                // The flag without slots: the store was used and is gone.
                return Err(Refusal::MissingStore { flag: true });
            }
            return Ok(Opened::NeverUsed);
        };
        // A newer schema is reported even when the other slot is valid:
        // the newer record is what the world actually was.
        let mut newest: Option<(u64, &SlotRecord)> = None;
        let mut newer = None;
        for record in &records {
            if let SlotError::NewerSchema { found } = record.error() {
                newer = Some(found);
            }
            if record.valid() && newest.is_none_or(|(no, _)| record.entry_no > no) {
                newest = Some((record.entry_no, record));
            }
        }
        if let Some(found) = newer {
            return Err(Refusal::NewerSchema { found });
        }
        let Some((entry_no, record)) = newest else {
            return Err(Refusal::NoValidSlot);
        };
        let slot = record.slot;
        let journal = record.body.clone();
        let tombstones_absent_at_open = tombstones_without_files(&data, &journal)?;
        Ok(Opened::Live(Self {
            data,
            entry_no,
            last_slot: slot,
            journal,
            capacity,
            tombstones_absent_at_open,
        }))
    }

    /// The store's first-ever use, on the explicit call of the first
    /// transfer's preflight (part B). Allowed only when `data/` existed
    /// when the world was opened, so its directory entry was published by
    /// an earlier session. The flag goes into `level.dat` first — a crash
    /// in its replacement gap leaves an untouched world that opens as
    /// never used — then slot 0 holds an empty initialized journal,
    /// published durably and read back before anything else may happen.
    pub fn first_use(world_root: &Path, data_existed_at_open: bool) -> Result<Self, FirstUseError> {
        Self::first_use_with_capacity(world_root, data_existed_at_open, MAX_CARRIERS)
    }

    /// [`Self::first_use`] with an explicit index capacity (tests).
    pub fn first_use_with_capacity(
        world_root: &Path,
        data_existed_at_open: bool,
        capacity: usize,
    ) -> Result<Self, FirstUseError> {
        if !data_existed_at_open {
            return Err(FirstUseError::DataCreatedThisSession);
        }
        match Self::open(world_root)? {
            Opened::Live(_) => return Err(FirstUseError::AlreadyInUse),
            Opened::NeverUsed => {}
        }
        let level_dat = world_root.join("level.dat");
        set_in_use_flag(&level_dat).map_err(FirstUseError::Flag)?;
        if !in_use_flag(world_root) {
            return Err(FirstUseError::Flag("flag did not read back".into()));
        }
        let data = world_root.join("data");
        let body = Journal::empty();
        let bytes = encode_slot(1, &body, capacity).map_err(FirstUseError::Slot)?;
        durable::write(&data.join(SLOTS[0]), &bytes)
            .map_err(|e| FirstUseError::Io(e.to_string()))?;
        // Read the slot back: only a store that opens is a store.
        match Self::open_with_capacity(world_root, capacity)? {
            Opened::Live(store) => Ok(store),
            Opened::NeverUsed => Err(FirstUseError::Slot(
                "slot 0 did not read back as valid".into(),
            )),
        }
    }

    /// The journal as the newest valid record holds it.
    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    /// Hands out the next operation id. Callers record it with
    /// [`Self::begin`] before writing any operation file.
    pub fn next_op_id(&self) -> u64 {
        self.journal.next_op_id
    }

    /// The generation a re-assembly of `uuid` takes: one past the
    /// highest generation the journal remembers for it — index entry or
    /// tombstone — computed with checked arithmetic; a carrier whose
    /// remembered generation is `u32::MAX` refuses rather than
    /// wrapping. The durable operation counter is also a lower bound,
    /// so deleting tombstones cannot revive an old generation.
    pub fn next_generation(&self, uuid: u128) -> Result<u32, StoreIoError> {
        let remembered = self
            .journal
            .carriers
            .iter()
            .chain(self.journal.tombstones.iter())
            .filter(|(u, _)| *u == uuid)
            .map(|(_, generation)| *generation)
            .max();
        let generation = self
            .journal
            .next_op_id
            .max(remembered.map_or(1, |last| u64::from(last) + 1));
        if generation > i32::MAX as u64 {
            return Err(StoreIoError("carrier generation limit reached".into()));
        }
        Ok(generation as u32)
    }

    /// Records the operation as in progress and publishes the next slot.
    /// Called after the operation file is durable, before the world
    /// changes.
    /// Refuse before publishing files or saving chunks. A full journal with a
    /// pending operation is reachable only from the unfixed, unshipped version;
    /// recovery of that state is outside this admission contract.
    pub fn check_admission(
        &self,
        kind: OpKind,
        uuid: u128,
        generation: u32,
    ) -> Result<(), StoreIoError> {
        let full = match kind {
            OpKind::Assemble => self.journal.carriers.len() >= self.capacity,
            OpKind::Disassemble => {
                self.journal.tombstones.len() >= MAX_TOMBSTONES
                    && !self.journal.tombstones.contains(&(uuid, generation))
            }
        };
        if full {
            return Err(StoreIoError(
                "CarrierCapacityReopenNeeded: carrier store capacity reached; reopen needed".into(),
            ));
        }
        Ok(())
    }

    pub fn begin(
        &mut self,
        kind: OpKind,
        uuid: u128,
        generation: u32,
        dimension: &str,
    ) -> Result<u64, StoreIoError> {
        self.check_admission(kind, uuid, generation)?;
        // One operation at a time: a nested begin leaves the open one
        // untouched and refuses.
        if self.journal.in_progress.is_some() {
            return Err(StoreIoError("an operation is already in progress".into()));
        }
        // Identity ownership, by kind:
        // - an assembly creates a carrier, so the UUID must be free —
        //   a second one would overwrite the first carrier's payload;
        // - a disassembly consumes the carrier its handle names, so the
        //   UUID and generation must be exactly the index's — a stale
        //   handle refuses.
        let listed = match kind {
            OpKind::Assemble => self.journal.carriers.iter().any(|(held, _)| *held == uuid),
            OpKind::Disassemble => self.journal.lists(uuid, generation),
        };
        match (kind, listed) {
            (OpKind::Assemble, true) => {
                return Err(StoreIoError(format!(
                    "carrier {uuid:032x} already exists at generation {generation}"
                )));
            }
            (OpKind::Disassemble, false) => {
                return Err(StoreIoError(format!(
                    "carrier {uuid:032x} at generation {generation} is not held: a stale handle refuses"
                )));
            }
            _ => {}
        }
        let op_id = self.journal.next_op_id;
        if generation > i32::MAX as u32 {
            return Err(StoreIoError("carrier generation limit reached".into()));
        }
        if op_id > i64::MAX as u64 {
            return Err(StoreIoError("operation id exhausted".into()));
        }
        let next_op_id = op_id
            .checked_add(1)
            .ok_or_else(|| StoreIoError("operation id exhausted".into()))?;
        self.entry_no
            .checked_add(1)
            .ok_or_else(|| StoreIoError("journal entry counter exhausted".into()))?;
        let previous = self.journal.clone();
        self.journal.in_progress = Some(InProgress {
            op_id,
            kind,
            uuid,
            generation,
            dimension: dimension.to_string(),
        });
        self.journal.next_op_id = next_op_id;
        if let Err(error) = self.write_slot() {
            self.journal = previous;
            return Err(error);
        }
        Ok(op_id)
    }

    /// Finishes the in-progress operation: the carrier index and
    /// tombstones take the operation's effect, the in-progress record
    /// clears, the next slot publishes, and the operation file is durably
    /// deleted before the next transfer may start.
    pub fn commit(&mut self) -> Result<(), StoreIoError> {
        let Some(op) = self.journal.in_progress.clone() else {
            return Err(StoreIoError("no operation in progress".into()));
        };
        let previous = self.journal.clone();
        match op.kind {
            OpKind::Assemble => {
                if !self.journal.lists(op.uuid, op.generation) {
                    self.journal.carriers.push((op.uuid, op.generation));
                }
            }
            OpKind::Disassemble => {
                self.journal
                    .carriers
                    .retain(|&c| c != (op.uuid, op.generation));
                if !self.journal.tombstoned(op.uuid, op.generation) {
                    self.journal.tombstones.push((op.uuid, op.generation));
                }
            }
        }
        self.journal.in_progress = None;
        if let Err(error) = self.write_slot() {
            self.journal = previous;
            return Err(error);
        }
        durable::remove(&self.operation_path(op.op_id)).map_err(|e| StoreIoError(e.to_string()))?;
        Ok(())
    }

    /// The operation file's path for `op_id`.
    pub fn operation_path(&self, op_id: u64) -> PathBuf {
        self.data.join(format!("sunrise_carrier_op_{op_id}.nbt"))
    }

    /// Publishes an encoded operation file durably.
    pub fn write_operation(&self, op_id: u64, encoded: &[u8]) -> Result<(), StoreIoError> {
        durable::write(&self.operation_path(op_id), encoded)
            .map_err(|e| StoreIoError(e.to_string()))
    }

    /// The carrier file's path.
    pub fn carrier_path(&self, uuid: u128, generation: u32) -> PathBuf {
        self.data
            .join(format!("sunrise_carrier_{uuid:032x}-{generation}.nbt"))
    }

    /// Publishes a carrier file durably.
    pub fn write_carrier(
        &self,
        uuid: u128,
        generation: u32,
        encoded: &[u8],
    ) -> Result<(), StoreIoError> {
        durable::write(&self.carrier_path(uuid, generation), encoded)
            .map_err(|e| StoreIoError(e.to_string()))
    }

    /// Retention for a just-finished operation, called from the live
    /// transfer path right after `commit`: the disassembly's tombstone
    /// clears only if its carrier file was already absent at open.
    /// File absence in this session is not proof of durable deletion.
    /// This is the per-operation counterpart of [`Self::cleanup`], not a
    /// replacement: world open still runs the full cleanup for files
    /// left by crashed sessions.
    pub fn finish_retention(&mut self) -> Result<(), StoreIoError> {
        let tombstones = self.journal.tombstones.clone();
        let mut changed = false;
        for (uuid, generation) in tombstones {
            if self.tombstones_absent_at_open.contains(&(uuid, generation))
                && !self
                    .data
                    .join(format!("sunrise_carrier_{uuid:032x}-{generation}.nbt"))
                    .exists()
            {
                self.journal.tombstones.retain(|t| *t != (uuid, generation));
                changed = true;
            }
        }
        if changed {
            self.write_slot()?;
        }
        Ok(())
    }

    /// Cleans the store by the valid journal: carrier files outside the
    /// index are orphans and go; an operation file that is not the
    /// in-progress one goes; a tombstone holds until its carrier file's
    /// deletion is durable, then clears with a new slot. Nothing is
    /// silently adopted: what the journal does not know is deleted or
    /// refused, never inferred into ownership.
    pub fn cleanup(&mut self) -> Result<Cleanup, StoreIoError> {
        let mut report = Cleanup::default();
        let names = fs::read_dir(&self.data)
            .map_err(|e| StoreIoError(e.to_string()))?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect::<Vec<_>>();
        let mut journal_changed = false;
        for name in &names {
            if let Some(op_id) = parse_operation_name(name) {
                let current = self.journal.in_progress.as_ref().map(|op| op.op_id);
                if current != Some(op_id) {
                    durable::remove(&self.data.join(name))
                        .map_err(|e| StoreIoError(e.to_string()))?;
                    report.stale_operations += 1;
                }
            } else if let Some((uuid, generation)) = parse_carrier_name(name) {
                if self.journal.lists(uuid, generation) {
                    continue;
                }
                durable::remove(&self.data.join(name)).map_err(|e| StoreIoError(e.to_string()))?;
                if self.journal.tombstoned(uuid, generation) {
                    // The deletion happened this session: on Windows it is
                    // not provably durable yet, so the tombstone holds
                    // until a later open finds the file gone.
                } else {
                    report.orphan_carriers += 1;
                }
            }
        }
        for &(uuid, generation) in &self.tombstones_absent_at_open {
            if self.journal.tombstoned(uuid, generation) {
                self.journal.tombstones.retain(|t| *t != (uuid, generation));
                journal_changed = true;
                report.resolved_tombstones += 1;
            }
        }
        if journal_changed {
            self.write_slot()?;
        }
        Ok(report)
    }

    /// Writes the next record into the slot that does not hold the newest
    /// valid one, durably, and only then adopts it.
    fn write_slot(&mut self) -> Result<(), StoreIoError> {
        let next_entry = self
            .entry_no
            .checked_add(1)
            .ok_or_else(|| StoreIoError("journal entry counter exhausted".into()))?;
        let bytes = encode_slot(next_entry, &self.journal, self.capacity)
            .map_err(|e| StoreIoError(e.to_string()))?;
        let slot = 1 - self.last_slot;
        durable::write(&self.data.join(SLOTS[slot]), &bytes)
            .map_err(|e| StoreIoError(e.to_string()))?;
        self.entry_no = next_entry;
        self.last_slot = slot;
        Ok(())
    }
}

/// What one cleanup pass removed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cleanup {
    /// Carrier files deleted that no index entry owned.
    pub orphan_carriers: usize,
    /// Operation files deleted that were not in progress.
    pub stale_operations: usize,
    /// Tombstones cleared after their carrier file's durable deletion.
    pub resolved_tombstones: usize,
}

/// Why a first use did not happen.
#[derive(Debug)]
pub enum FirstUseError {
    /// `data/` was created during this session; its directory entry is
    /// not durable yet. Save and reopen the world first.
    DataCreatedThisSession,
    /// The store already exists (or refuses to open).
    AlreadyInUse,
    /// Writing or reading back the `level.dat` flag failed.
    Flag(String),
    /// The first slot could not be written or read back.
    Slot(String),
    /// Plain I/O.
    Io(String),
}
impl From<Refusal> for FirstUseError {
    fn from(refusal: Refusal) -> Self {
        Self::Io(refusal.to_string())
    }
}
impl fmt::Display for FirstUseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DataCreatedThisSession => write!(
                f,
                "the world's data folder was created this session; save and reopen before \
                 the first carrier transfer"
            ),
            Self::AlreadyInUse => write!(f, "the carrier store is already in use"),
            Self::Flag(error) => write!(f, "carriers_in_use flag: {error}"),
            Self::Slot(error) => write!(f, "first journal slot: {error}"),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

/// The registry-backed state validator: a state image is known exactly
/// when the block registry resolves it — the name exists and every
/// property with this value is declared on that block. This is part B's
/// replacement of part A's all-accepted seam.
pub struct RegistryStates<'a> {
    registry: &'a sunrise_world::registry::BlockRegistry,
}
impl<'a> RegistryStates<'a> {
    /// Wraps the live block registry.
    pub fn new(registry: &'a sunrise_world::registry::BlockRegistry) -> Self {
        Self { registry }
    }
}
impl StateValidator for RegistryStates<'_> {
    fn known(&self, name: &str, properties: &[(String, String)]) -> bool {
        let descriptor = sunrise_world::chunk::BlockState {
            name: name.to_string(),
            properties: properties.iter().cloned().collect(),
        };
        self.registry.resolve(&descriptor).is_ok()
    }
}

/// The longest state the registry declares, serialized the way an
/// operation file spells it (name plus every `key=value`). Computed once
/// at load; a registry whose states cannot fit the operation envelope
/// refuses preflight, never a half-stored transfer.
pub fn longest_state_bytes(registry: &sunrise_world::registry::BlockRegistry) -> usize {
    let mut longest = 0_usize;
    for id in 0..registry.len() {
        let Some(state) = registry.state(sunrise_world::registry::BlockStateId(id as u32)) else {
            continue;
        };
        let mut size = state.name.len();
        for (key, value) in &state.properties {
            size += key.len() + value.len() + 4;
        }
        longest = longest.max(size);
    }
    longest
}

/// Whether a whole operation's worst shape fits the envelope: two images
/// per cell at the registry's longest state, the full 16 MiB of
/// block-entity data, and framing. Refused at preflight, before any
/// block is lifted, when the registry's own declarations outgrow the
/// envelope.
pub fn envelope_preflight(
    registry: &sunrise_world::registry::BlockRegistry,
) -> Result<(), OpError> {
    let worst = 2 * 4096 * longest_state_bytes(registry) + MAX_OP_ENTITIES + 1024 * 1024;
    if worst > MAX_OP_DECODED {
        return Err(OpError::Malformed(
            "the block registry's longest state exceeds the operation envelope",
        ));
    }
    Ok(())
}

/// Whether a rotation is one of the cube's symmetries — a multiple of
/// ninety degrees about a single coordinate axis. Disassembly without a
/// state-rotation adapter (slice 4's `StructureCellPart`) refuses
/// anything else rather than silently rewriting facing properties: at
/// most two components are nonzero, the angle cosine is 0 or ±√½ or ±1,
/// and the sine rides exactly one axis.
pub fn cube_symmetry_rotation(components: [f64; 4]) -> bool {
    let [w, x, y, z] = components;
    if w == 0.0 && x == 0.0 && y == 0.0 && z == 0.0 {
        return false;
    }
    for value in [w, x, y, z] {
        if !value.is_finite() {
            return false;
        }
        let magnitude = value.abs();
        let on_axis = (magnitude - 1.0).abs() < 1.0e-12 || magnitude < 1.0e-12;
        let on_half = (magnitude - std::f64::consts::FRAC_1_SQRT_2).abs() < 1.0e-12;
        if !on_axis && !on_half {
            return false;
        }
    }
    // At most one of the vector components may be nonzero alongside the
    // angle cosine: a quarter turn about exactly one axis.
    [x, y, z].iter().filter(|v| v.abs() >= 1.0e-12).count() <= 1
}

include!("carrier/recovery.rs");

/// A store-level I/O failure, with the operation's text.
#[derive(Debug)]
pub struct StoreIoError(pub String);
impl fmt::Display for StoreIoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "carrier store: {}", self.0)
    }
}

/// Whether the `level.dat` flag is set.
pub fn in_use_flag(world_root: &Path) -> bool {
    read_dat(&world_root.join("level.dat"))
        .as_ref()
        .and_then(|root| root.get("Data"))
        .is_some_and(|data| data.get(IN_USE_FLAG).is_some())
}

/// Writes the flag into `level.dat` through the ordinary dat replacement
/// (temporary, rename, `.dat_old`): a crash in the replacement gap rolls
/// back to the flagless file, so no store state may exist yet.
pub fn set_in_use_flag(level_dat: &Path) -> Result<(), String> {
    let mut root = read_dat(level_dat).ok_or("level.dat does not read")?;
    let Tag::Compound(map) = &mut root else {
        return Err("level.dat root is not a compound".into());
    };
    let data = map
        .get_mut(&JavaString::from("Data"))
        .and_then(|tag| match tag {
            Tag::Compound(data) => Some(data),
            _ => None,
        });
    let Some(data) = data else {
        return Err("level.dat has no Data compound".into());
    };
    data.insert(IN_USE_FLAG.into(), Tag::Byte(1));
    write_dat(level_dat, root).map_err(|e| e.to_string())
}

include!("carrier/slots.rs");

include!("carrier/operation.rs");

/// A static reason from a shared NBT error string.
fn leak(reason: &'static str) -> &'static str {
    reason
}

/// A static reason naming `key`.
fn leak_key(key: &str) -> &'static str {
    match key {
        "version" => "version is not an int",
        "op_id" => "op_id is not a long",
        "kind" => "kind is not a string",
        "carrier_uuid_msb" => "carrier_uuid_msb is not a long",
        "carrier_uuid_lsb" => "carrier_uuid_lsb is not a long",
        "carrier_generation" => "carrier_generation is not an int",
        "dimension" => "dimension is not a string",
        "cells" => "cells is not a list",
        "world" => "world is not three ints",
        "local" => "local is not three bytes",
        "state" => "state is not a string",
        "properties" => "properties is not a compound",
        "entity" => "entity is not an NBT tag",
        _ => "unexpected field",
    }
}

/// Read a disk boundary with one sentinel byte, never allocating the whole file.
pub fn read_bounded(path: &Path, limit: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "carrier input exceeds its byte limit",
        ));
    }
    Ok(bytes)
}
