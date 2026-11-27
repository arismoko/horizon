//! Carrier transfers in the server's game: the fences that hold the
//! world's cells and the carrier still while an operation is between
//! prepared and committed, and the assembly that runs at a tick
//! boundary (issue #2413, slice 2 part C).
//!
//! A transfer is one synchronous call on the server's thread at a tick
//! boundary: no tick runs between `prepared` and `committed`, so routes
//! without their own fence chokepoint (the player's manual block
//! placement, guest container clicks, attachment writes) cannot
//! interleave today. If the commit ever becomes asynchronous, those
//! routes need chokepoints of their own.
//!
//! The fences are one set of locked world cells plus locked carriers on
//! the server's level. Each refusal route is checked at one chokepoint:
//! breaking (`Game::break_block`), every mod write through the level's
//! write gate (`native_level_write_allowed`), pistons
//! (`Game::allow_piston_move`) and block-entity container opening
//! (`refuse_locked_container`). With no operation in flight the check is
//! one load of an empty set.

use crate::game::ServerGame;
use sunrise_core::position::BlockPos;
use sunrise_sim::save::block_entity_encode;
use sunrise_sim::save::carrier::{MAX_CARRIERS, Store};
use sunrise_world::carrier::codec;
use sunrise_world::carrier::contact::RigidTransform;
use sunrise_world::carrier::model::{Carrier, Cell, Invalid, LocalPos, MAX_CELLS};
use sunrise_world::level::LevelAccess;
use sunrise_world::level::data::state_flags::REPLACEABLE;

/// Why a transfer refused. Refusals leave no partial transfer.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug)]
pub enum TransferError {
    /// A cell's chunk is not loaded and ticking.
    CellNotLoaded([i32; 3]),
    /// More cells than the carrier model allows (4,096).
    TooManyCells(usize),
    /// A cell holds a block this slice does not move (not a plain block
    /// or a passive block entity).
    UnsupportedBlock([i32; 3]),
    /// Saved block-entity tags have no transfer payload adapter.
    SavedBlockEntity([i32; 3]),
    /// The registry's own states cannot fit the operation envelope.
    Envelope,
    /// The model refused the assembled carrier.
    Model(Invalid),
    /// The store or its files refused.
    Store(String),
    /// Revalidation after the lift found a cell that did not move.
    Revalidation([i32; 3]),
    /// The rotation is not one of the cube's symmetries.
    NotCubeSymmetry,
    /// The selection's span exceeds the local coordinate range, or its
    /// offsets overflowed the checked arithmetic.
    LocalRange,
    /// An empty selection was refused before any indexing.
    EmptySelection,
    /// A rotation other than identity needs a state-rotation adapter
    /// (slice 4's); without one facing properties would be rewritten
    /// silently.
    RotationNeedsAdapter,
    /// A disassembly destination is neither air nor a replaceable block
    /// without a block entity.
    DestinationOccupied([i32; 3]),
    /// The carrier file is missing or does not decode.
    CarrierUnreadable(String),
    /// Plain I/O.
    Io(String),
    /// Only the logical server transfers; a client's mirror of the
    /// world receives the result through synchronization.
    NotLogicalServer,
}
impl std::fmt::Display for TransferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CellNotLoaded(pos) => write!(
                f,
                "cell ({},{},{}) is not in a loaded, ticking chunk",
                pos[0], pos[1], pos[2]
            ),
            Self::TooManyCells(count) => write!(f, "{count} cells exceeds {MAX_CELLS}"),
            Self::UnsupportedBlock(pos) => write!(
                f,
                "cell ({},{},{}) holds a block this slice does not transfer",
                pos[0], pos[1], pos[2]
            ),
            Self::SavedBlockEntity(pos) => {
                write!(f, "saved block entity at {pos:?} prevents transfer")
            }
            Self::Envelope => write!(f, "the registry's states exceed the operation envelope"),
            Self::Model(invalid) => write!(f, "{invalid}"),
            Self::Store(text) => write!(f, "{text}"),
            Self::Revalidation(pos) => write!(
                f,
                "revalidation: cell ({},{},{}) did not move",
                pos[0], pos[1], pos[2]
            ),
            Self::NotCubeSymmetry => write!(f, "the rotation is not a cube symmetry"),
            Self::EmptySelection => write!(f, "an empty selection refuses"),
            Self::LocalRange => {
                write!(f, "the selection's span exceeds the local coordinate range")
            }
            Self::RotationNeedsAdapter => {
                write!(f, "a rotated disassembly needs a state adapter")
            }
            Self::DestinationOccupied(pos) => write!(
                f,
                "destination ({},{},{}) is neither air nor replaceable",
                pos[0], pos[1], pos[2]
            ),
            Self::CarrierUnreadable(why) => write!(f, "carrier file: {why}"),
            Self::Io(text) => write!(f, "carrier transfer I/O: {text}"),
            Self::NotLogicalServer => write!(
                f,
                "only the logical server transfers; this game is a client's mirror"
            ),
        }
    }
}

/// Whether a block entity is passive cargo this slice carries: a
/// container or a sign, banner, skull or bell — pure data, no ticker.
/// Ticking machines (furnaces, brewing stands, spawners, sculk,
/// jukeboxes, beacons, campfires, daylight detectors, conduits,
/// gateways) and world-dependent ones refuse the assembly until
/// slice 4's adapters exist.
fn block_entity_is_passive(entity: &sunrise_level::chunk::block_entity::BlockEntity) -> bool {
    use sunrise_level::chunk::block_entity::BlockEntity;
    matches!(
        entity,
        BlockEntity::Chest(_)
            | BlockEntity::Sign(_)
            | BlockEntity::Banner(_)
            | BlockEntity::Skull(_)
            | BlockEntity::Bell
            | BlockEntity::Lectern(_)
    )
}

/// Whether a live block entity's serialized form carries exactly the
/// image's content: the positional `x`/`y`/`z` are placement, not
/// content, so the comparison strips them first.
fn entities_content_equal(live: &sunrise_core::nbt::Tag, image: &sunrise_core::nbt::Tag) -> bool {
    let strip = |tag: &sunrise_core::nbt::Tag| -> sunrise_core::nbt::Tag {
        let mut clone = tag.clone();
        if let sunrise_core::nbt::Tag::Compound(map) = &mut clone {
            for key in ["x", "y", "z"] {
                map.remove(&sunrise_core::nbt::JavaString::from(key));
            }
        }
        clone
    };
    sunrise_core::nbt::compare_nbt(&strip(live), Some(&strip(image)))
}

impl ServerGame<'_> {
    /// Assembles `cells` into one carrier. One synchronous call on the
    /// server's thread at a tick boundary — no tick runs between
    /// `prepared` and `committed`, which is why routes without their own
    /// fence chokepoint (the player's manual block placement, guest
    /// container clicks, attachment writes) cannot interleave; if the
    /// commit ever becomes asynchronous, they will need chokepoints.
    ///
    /// Order: preflight; the operation encoded in memory; the affected
    /// chunks forced to disk with the writer's acknowledged barrier
    /// (before the #2488 obligation, under which saves refuse); a
    /// revalidation that every cell still holds exactly its preflight
    /// state and entity — a mismatch refuses with the world and the
    /// store untouched; then, under the obligation, the operation file
    /// and `prepared` durably, the lift (owned removal, no drops), the
    /// fences, and forward only: after `prepared` an I/O failure leaves
    /// the fences up, marks the transfer broken so every later transfer
    /// refuses until the world reopens, and hands the operation to
    /// recovery at open. The carrier file and the chunks go out through
    /// the flush barrier, the journal commits, the fences drop.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn carrier_assemble(
        &mut self,
        uuid: u128,
        cells: &[BlockPos],
    ) -> Result<(), TransferError> {
        use sunrise_sim::join_seq::visibility::PassLease;
        use sunrise_sim::save::carrier::{CellImage, OpKind, encode_operation, envelope_preflight};
        use sunrise_sim::save::carrier::{Operation, OperationCell};

        // A remote guest's mirror never transfers: the server's own
        // transfer reaches it through synchronization, so a guest's
        // call refuses with a reason rather than silently succeeding
        // (the disassembly's gate refuses the same way).
        if !self.common.is_logical_server() {
            return Err(TransferError::NotLogicalServer);
        }
        if self.common.world.carrier_fences.is_broken() {
            return Err(TransferError::Store(
                "a previous carrier transfer did not finish; reopen the world".into(),
            ));
        }
        if cells.is_empty() {
            // An empty assembly refuses before any indexing or lookup.
            return Err(TransferError::EmptySelection);
        }
        if cells.len() > MAX_CELLS {
            return Err(TransferError::TooManyCells(cells.len()));
        }
        let registry = self.common.pack.assets.registry;
        envelope_preflight(registry).map_err(|_| TransferError::Envelope)?;

        // Preflight: loaded chunks; plain blocks or passive block
        // entities (this slice moves containers; ticking adapters are
        // slice 4's).
        let air = registry
            .default_state("minecraft:air")
            .ok_or_else(|| TransferError::Io("no air in the registry".into()))?;
        for pos in cells {
            let (x, z) = (pos.x >> 4, pos.z >> 4);
            if !self.common.world.chunks.contains_key(&(x, z)) {
                return Err(TransferError::CellNotLoaded([pos.x, pos.y, pos.z]));
            }
            if self.common.world.block(pos.x, pos.y, pos.z) == air {
                return Err(TransferError::UnsupportedBlock([pos.x, pos.y, pos.z]));
            }
            // Saved-only entities have no carrier payload adapter yet.
            if sunrise_gameplay_common::game::spill_common::saved_entity(
                &self.common.world,
                [pos.x, pos.y, pos.z],
            )
            .is_some()
            {
                return Err(TransferError::SavedBlockEntity([pos.x, pos.y, pos.z]));
            }
            // The passive-or-supported rule runs before anything is
            // lifted: a block entity this slice does not carry (ticking
            // machines, world-dependent ones) refuses in preflight, with
            // the world untouched.
            if let Some(entity) = self.common.world.block_entities.get(&(pos.x, pos.y, pos.z))
                && !block_entity_is_passive(entity)
            {
                return Err(TransferError::UnsupportedBlock([pos.x, pos.y, pos.z]));
            }
        }

        // The store exists only from the first actual transfer. The
        // capacity override is the tests' retention seam.
        #[cfg(any(test, feature = "test-support"))]
        let capacity = self
            .common
            .world
            .carrier_fences
            .capacity_override
            .unwrap_or(MAX_CARRIERS);
        #[cfg(not(any(test, feature = "test-support")))]
        let capacity = MAX_CARRIERS;
        let mut store = match Store::open_with_capacity(&self.server.world_root, capacity) {
            Ok(sunrise_sim::save::carrier::Opened::Live(store)) => store,
            Ok(sunrise_sim::save::carrier::Opened::NeverUsed) => {
                // The gate reads the flag captured at world open, not
                // the folder's existence now: a data folder created
                // this session does not pass for an old one.
                Store::first_use_with_capacity(
                    &self.server.world_root,
                    self.common.world.carrier_data_existed_at_open,
                    capacity,
                )
                .map_err(|e| TransferError::Store(e.to_string()))?
            }
            Err(e) => return Err(TransferError::Store(e.to_string())),
        };

        store.use_retention_snapshot(&self.server.carrier_retention_snapshot);

        // Before/after images from the live world, encoded in memory —
        // a transfer that cannot be stored never lifts a block.
        let mut operation_cells = Vec::with_capacity(cells.len());
        for pos in cells {
            let key = [pos.x, pos.y, pos.z];
            let state = self.common.world.block(pos.x, pos.y, pos.z);
            let descriptor = registry
                .state(state)
                .ok_or(TransferError::UnsupportedBlock(key))?;
            let entity = self
                .common
                .world
                .block_entities
                .get(&(key[0], key[1], key[2]))
                .map(|entity| {
                    block_entity_encode::encode(
                        (key[0], key[1], key[2]),
                        entity,
                        &descriptor.name,
                        self.common.res.items(),
                    )
                });
            operation_cells.push(OperationCell {
                world: key,
                local: [0, 0, 0],
                before: CellImage {
                    name: descriptor.name.clone(),
                    properties: descriptor
                        .properties
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect(),
                    entity,
                },
                after: CellImage {
                    name: "minecraft:air".to_string(),
                    properties: Vec::new(),
                    entity: None,
                },
            });
        }
        // Local coordinates derive from the selection's minimum corner
        // — the pivot — so the input order of `cells` no longer shapes
        // geometry: a cell's local position is its offset from that
        // corner, computed with checked arithmetic and checked against
        // the i8 range before it narrows. The sparse layout survives
        // (gaps stay gaps), and any selection whose span exceeds the
        // local range refuses before a block is lifted.
        let pivot = [
            cells.iter().map(|p| p.x).min().unwrap_or(0),
            cells.iter().map(|p| p.y).min().unwrap_or(0),
            cells.iter().map(|p| p.z).min().unwrap_or(0),
        ];
        for cell in &mut operation_cells {
            let local = [
                cell.world[0]
                    .checked_sub(pivot[0])
                    .ok_or(TransferError::LocalRange)?,
                cell.world[1]
                    .checked_sub(pivot[1])
                    .ok_or(TransferError::LocalRange)?,
                cell.world[2]
                    .checked_sub(pivot[2])
                    .ok_or(TransferError::LocalRange)?,
            ];
            if local
                .iter()
                .any(|v| !(i32::from(i8::MIN)..=i32::from(i8::MAX)).contains(v))
            {
                return Err(TransferError::LocalRange);
            }
            cell.local = [local[0] as i8, local[1] as i8, local[2] as i8];
        }
        let carrier_cells = operation_cells
            .iter()
            .map(|cell| -> Result<Cell, TransferError> {
                // The entity rides the carrier file whole: its NBT
                // serialized once, so a later disassembly replays it. A
                // serialization failure is a refusal — the entity is
                // never silently dropped from the carrier.
                let data = cell
                    .before
                    .entity
                    .as_ref()
                    .map(|entity| {
                        sunrise_core::nbt::write_named(
                            &sunrise_core::nbt::NamedTag {
                                name: "".into(),
                                tag: entity.clone(),
                            },
                            sunrise_core::nbt::Limits::saved(),
                        )
                        .map_err(|e| TransferError::Io(e.to_string()))
                    })
                    .transpose()?;
                Cell::new(
                    LocalPos::new(cell.local[0], cell.local[1], cell.local[2]),
                    &cell.before.name,
                    cell.before.properties.clone(),
                    data,
                )
                .map_err(TransferError::Model)
            })
            .collect::<Result<Vec<_>, TransferError>>()?;
        // A re-assembly of a UUID the journal remembers takes the next
        // generation, bounded by the signed operation format. The durable
        // operation counter is a lower bound even for a fresh UUID.
        let generation = store
            .next_generation(uuid)
            .map_err(|e| TransferError::Store(e.to_string()))?;
        let carrier = Carrier::new(
            uuid,
            generation,
            0,
            RigidTransform::identity_at(sunrise_core::geometry::Vec3::new(
                pivot[0] as f64,
                pivot[1] as f64,
                pivot[2] as f64,
            ))
            .map_err(|_| TransferError::Io("the identity placement refused".into()))?,
            carrier_cells,
        )
        .map_err(TransferError::Model)?;

        let op_id = store.next_op_id();
        let operation = Operation {
            op_id,
            kind: OpKind::Assemble,
            uuid,
            generation,
            dimension: self.common.dim.id().to_string(),
            cells: operation_cells.clone(),
        };
        let encoded = encode_operation(&operation).map_err(|_| TransferError::Envelope)?;

        // The affected chunks reach the disk, acknowledged, before the
        // obligation: under #2488 saves refuse, so this must precede the
        // lease. Replay will find its pre-image here.
        store
            .check_admission(OpKind::Assemble, uuid, generation)
            .map_err(|e| TransferError::Store(e.to_string()))?;
        self.save_affected(cells)
            .map_err(|e| TransferError::Io(e.to_string()))?;

        // Revalidation, before any intent is published: every cell must
        // still hold exactly the state and the entity the images were
        // built from. A mismatch refuses with the world and the store
        // untouched.
        self.revalidate_cells(&operation_cells, registry)?;

        // From here the path runs forward to `committed`. The lift is
        // under #2488's obligation, fenced until the barrier confirms.
        let _lease = PassLease::enter();
        // The immutable carrier is published and verified by reading it
        // back BEFORE any intent exists: once `prepared` runs, the
        // cargo's only durable owner is this file, and recovery only
        // verifies it — nothing after the lift ever rewrites it.
        let carrier_bytes =
            codec::encode(&carrier).map_err(|e| TransferError::Store(e.to_string()))?;
        store
            .write_carrier(uuid, generation, &carrier_bytes)
            .map_err(|e| TransferError::Store(e.to_string()))?;
        let read_back = sunrise_sim::save::carrier::read_bounded(
            &store.carrier_path(uuid, generation),
            codec::MAX_INPUT_BYTES,
        )
        .map_err(|e| TransferError::Store(e.to_string()))?;
        if read_back != carrier_bytes {
            return Err(TransferError::Store(
                "the published carrier did not read back identically".into(),
            ));
        }
        store
            .write_operation(op_id, &encoded)
            .map_err(|e| TransferError::Store(e.to_string()))?;
        store
            .begin(OpKind::Assemble, uuid, generation, self.common.dim.id())
            .map_err(|e| TransferError::Store(e.to_string()))?;
        for pos in cells {
            let old = self.common.world.block(pos.x, pos.y, pos.z);
            // Owned removal without drops or notifications: the carrier
            // takes the entity whole, and the cell becomes air through
            // the level's own write path.
            self.with_level(|_b, level, ticks| {
                level.remove_block_entity(*pos, old, false);
                _b.set_block(level, ticks, *pos, air, 3);
            });
            self.common.world.modified.insert((pos.x >> 4, pos.z >> 4));
        }
        // Fence the world cells and the carrier until the barrier lifts.
        for pos in cells {
            self.common
                .world
                .carrier_fences
                .lock_cell([pos.x, pos.y, pos.z]);
        }
        self.common.world.carrier_fences.lock_carrier(uuid);
        drop(_lease);

        // The application is this same synchronous call's own writes; a
        // cell that is not air here is a bug, not a refusal path.
        for pos in cells {
            debug_assert_eq!(self.world.block(pos.x, pos.y, pos.z), air);
        }

        // Forward only: an I/O failure past `prepared` keeps the fences,
        // marks the transfer broken and leaves the operation to
        // recovery at world open.
        if let Err(error) = self.finish_assemble(&mut store, uuid, generation, cells) {
            log::error!("carrier transfer did not finish: {error}");
            self.common.world.carrier_fences.mark_broken();
            carrier_stop_session(self, &error);
            return Err(error);
        }
        for pos in cells {
            self.common
                .world
                .carrier_fences
                .unlock_cell([pos.x, pos.y, pos.z]);
        }
        self.common.world.carrier_fences.unlock_carrier(uuid);
        Ok(())
    }

    /// Disassembles the carrier back into the world, the assembly's
    /// mirror with the same order rules. `translation` places the
    /// carrier's local origin in the world on the grid; `rotation` must
    /// be one of the cube's symmetries, and this slice — with no
    /// state-rotation adapter — accepts only the identity, refusing
    /// rather than silently rewriting facing properties. Destinations
    /// must be air or a replaceable block without a block entity; the
    /// entity returns from the carrier's own payload, then the ordinary
    /// neighbour updates run. One synchronous server call: no tick runs
    /// between `prepared` and `committed`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn carrier_disassemble(
        &mut self,
        uuid: u128,
        generation: u32,
        translation: [i32; 3],
        rotation: [f64; 4],
    ) -> Result<(), TransferError> {
        use sunrise_core::nbt::{self, Tag};
        use sunrise_sim::join_seq::visibility::PassLease;
        use sunrise_sim::save::carrier::{
            CellImage, OpKind, Operation, OperationCell, cube_symmetry_rotation, envelope_preflight,
        };

        // A remote guest's mirror never transfers: the server's own
        // transfer reaches it through synchronization, so a guest's
        // call refuses with a reason rather than silently succeeding
        // (the assembly's gate refuses the same way). This is the
        // logical-server guard the review's item 5 asked for.
        if !self.common.is_logical_server() {
            return Err(TransferError::NotLogicalServer);
        }
        if self.common.world.carrier_fences.is_broken() {
            return Err(TransferError::Store(
                "a previous carrier transfer did not finish; reopen the world".into(),
            ));
        }
        if !cube_symmetry_rotation(rotation) {
            return Err(TransferError::NotCubeSymmetry);
        }
        if rotation != [1.0, 0.0, 0.0, 0.0] {
            return Err(TransferError::RotationNeedsAdapter);
        }
        let registry = self.common.pack.assets.registry;
        envelope_preflight(registry).map_err(|_| TransferError::Envelope)?;

        #[cfg(any(test, feature = "test-support"))]
        let capacity = self
            .common
            .world
            .carrier_fences
            .capacity_override
            .unwrap_or(MAX_CARRIERS);
        #[cfg(not(any(test, feature = "test-support")))]
        let capacity = MAX_CARRIERS;
        let mut store = match Store::open_with_capacity(&self.server.world_root, capacity) {
            Ok(sunrise_sim::save::carrier::Opened::Live(store)) => store,
            _ => {
                return Err(TransferError::Store(
                    "the carrier store does not open".into(),
                ));
            }
        };
        store.use_retention_snapshot(&self.server.carrier_retention_snapshot);
        if !store.journal().lists(uuid, generation) {
            return Err(TransferError::CarrierUnreadable(
                "the journal does not list this carrier".into(),
            ));
        }
        let carrier_bytes = sunrise_sim::save::carrier::read_bounded(
            &store.carrier_path(uuid, generation),
            codec::MAX_INPUT_BYTES,
        )
        .map_err(|e| TransferError::CarrierUnreadable(e.to_string()))?;
        let carrier = codec::decode(&carrier_bytes)
            .map_err(|e| TransferError::CarrierUnreadable(e.to_string()))?;

        // Destinations: on loaded chunks, air or replaceable without a
        // block entity. The images are built from the carrier itself.
        let mut operation_cells = Vec::with_capacity(carrier.cells().len());
        for cell in carrier.cells() {
            // Destination arithmetic is checked: an overflowing
            // placement refuses rather than wrapping the world.
            let offset = [
                i32::from(cell.pos().x()),
                i32::from(cell.pos().y()),
                i32::from(cell.pos().z()),
            ];
            let world = [
                translation[0]
                    .checked_add(offset[0])
                    .ok_or(TransferError::LocalRange)?,
                translation[1]
                    .checked_add(offset[1])
                    .ok_or(TransferError::LocalRange)?,
                translation[2]
                    .checked_add(offset[2])
                    .ok_or(TransferError::LocalRange)?,
            ];
            if !self
                .common
                .world
                .chunks
                .contains_key(&(world[0] >> 4, world[2] >> 4))
            {
                return Err(TransferError::CellNotLoaded(world));
            }
            let state = self.common.world.block(world[0], world[1], world[2]);
            let occupied = self
                .common
                .world
                .block_entities
                .contains_key(&(world[0], world[1], world[2]));
            let replaceable =
                self.with_level(|b, _level, _ticks| b.facts(state).flags & REPLACEABLE != 0);
            if sunrise_gameplay_common::game::spill_common::saved_entity(&self.common.world, world)
                .is_some()
            {
                return Err(TransferError::SavedBlockEntity(world));
            }
            if !replaceable || occupied {
                return Err(TransferError::DestinationOccupied(world));
            }
            // An entity payload that does not decode refuses the whole
            // disassembly — it is never silently dropped.
            let entity = cell
                .data()
                .map(|bytes| {
                    nbt::read_named(bytes, sunrise_core::nbt::Limits::saved())
                        .map(|(named, _)| named.tag)
                        .map_err(|e| TransferError::CarrierUnreadable(e.to_string()))
                })
                .transpose()?;
            let properties = cell
                .properties()
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect();
            // The destination's before-image records what is really
            // there — the replaceable block's own state, not a guess
            // that it was air.
            let before_descriptor = registry.state(state);
            let before = if state == registry.default_state("minecraft:air").unwrap_or(state) {
                CellImage {
                    name: "minecraft:air".to_string(),
                    properties: Vec::new(),
                    entity: None,
                }
            } else {
                CellImage {
                    name: before_descriptor
                        .map(|d| d.name.clone())
                        .unwrap_or_default(),
                    properties: before_descriptor
                        .map(|d| {
                            d.properties
                                .iter()
                                .map(|(k, v)| (k.to_string(), v.to_string()))
                                .collect()
                        })
                        .unwrap_or_default(),
                    entity: None,
                }
            };
            operation_cells.push(OperationCell {
                world,
                local: [cell.pos().x(), cell.pos().y(), cell.pos().z()],
                before,
                after: CellImage {
                    name: cell.name().to_string(),
                    properties,
                    entity,
                },
            });
        }

        let cells: Vec<BlockPos> = operation_cells
            .iter()
            .map(|cell| BlockPos::new(cell.world[0], cell.world[1], cell.world[2]))
            .collect();
        let op_id = store.next_op_id();
        let operation = Operation {
            op_id,
            kind: OpKind::Disassemble,
            uuid,
            generation,
            dimension: self.common.dim.id().to_string(),
            cells: operation_cells.clone(),
        };
        let encoded = sunrise_sim::save::carrier::encode_operation(&operation)
            .map_err(|_| TransferError::Envelope)?;

        store
            .check_admission(OpKind::Disassemble, uuid, generation)
            .map_err(|e| TransferError::Store(e.to_string()))?;
        self.save_affected(&cells)
            .map_err(|e| TransferError::Io(e.to_string()))?;
        self.revalidate_destinations(&operation_cells)?;

        let _lease = PassLease::enter();
        store
            .write_operation(op_id, &encoded)
            .map_err(|e| TransferError::Store(e.to_string()))?;
        store
            .begin(OpKind::Disassemble, uuid, generation, self.common.dim.id())
            .map_err(|e| TransferError::Store(e.to_string()))?;
        for cell in &operation_cells {
            let pos = BlockPos::new(cell.world[0], cell.world[1], cell.world[2]);
            // The state resolves from the saved properties, not the
            // block's default: an oak stair kept facing=north and
            // half=bottom through the round trip, never snapping to the
            // default facing.
            let state = {
                let descriptor = sunrise_world::chunk::BlockState {
                    name: cell.after.name.clone(),
                    properties: cell.after.properties.iter().cloned().collect(),
                };
                self.common
                    .pack
                    .assets
                    .registry
                    .resolve(&descriptor)
                    .map_err(|_| TransferError::UnsupportedBlock(cell.world))?
            };
            // The block through the level's own write path — neighbour
            // updates included — then the entity from the carrier's
            // payload.
            self.with_level(|_b, level, ticks| {
                _b.set_block(level, ticks, pos, state, 3);
            });
            self.common.world.modified.insert((pos.x >> 4, pos.z >> 4));
            if let Some(Tag::Compound(map)) = cell.after.entity.clone() {
                self.with_level(|_b, level, _ticks| {
                    level.load_block_entity(pos, map);
                });
            }
        }
        for cell in &operation_cells {
            self.common.world.carrier_fences.lock_cell(cell.world);
        }
        self.common.world.carrier_fences.lock_carrier(uuid);
        drop(_lease);

        if let Err(error) = self.finish_disassemble(&mut store, uuid, generation, &cells) {
            log::error!("carrier transfer did not finish: {error}");
            self.common.world.carrier_fences.mark_broken();
            carrier_stop_session(self, &error);
            return Err(error);
        }
        for cell in &operation_cells {
            self.common.world.carrier_fences.unlock_cell(cell.world);
        }
        self.common.world.carrier_fences.unlock_carrier(uuid);
        Ok(())
    }

    /// The disassembly's commit tail: the chunks and the barrier, then
    /// `committed` — whose tombstone holds until the carrier file's
    /// deletion proves durable at a later open.
    fn finish_disassemble(
        &mut self,
        store: &mut Store,
        uuid: u128,
        generation: u32,
        cells: &[BlockPos],
    ) -> Result<(), TransferError> {
        self.save_affected(cells)
            .map_err(|e| TransferError::Io(e.to_string()))?;
        #[cfg(any(test, feature = "test-support"))]
        if self.common.world.carrier_fences.fail_flush {
            return Err(TransferError::Io(
                "injected: the flush barrier failed after prepared".into(),
            ));
        }
        self.server
            .save
            .0
            .flush()
            .map_err(|e| TransferError::Io(e.to_string()))?;
        store
            .commit()
            .map_err(|e| TransferError::Store(e.to_string()))?;
        // The tombstone this commit leaves holds the carrier from coming
        // back until a later open finds the file durably gone.
        if let Err(error) = std::fs::remove_file(store.carrier_path(uuid, generation)) {
            log::warn!("carrier file removal deferred: {error}");
        }
        // Only tombstones eligible at the actual world open may clear.
        // New deletions retain their witnesses throughout this session;
        // reaching the finite tombstone bound requires reopening.
        store
            .finish_retention()
            .map_err(|e| TransferError::Store(e.to_string()))?;
        Ok(())
    }

    /// Whether every destination still holds what the images recorded:
    /// replaceable, with no block entity.
    pub fn revalidate_destinations(
        &mut self,
        cells: &[sunrise_sim::save::carrier::OperationCell],
    ) -> Result<(), TransferError> {
        for cell in cells {
            let pos = cell.world;
            let state = self.common.world.block(pos[0], pos[1], pos[2]);
            let replaceable =
                self.with_level(|b, _level, _ticks| b.facts(state).flags & REPLACEABLE != 0);
            let entity = self
                .common
                .world
                .block_entities
                .contains_key(&(pos[0], pos[1], pos[2]));
            if sunrise_gameplay_common::game::spill_common::saved_entity(&self.common.world, pos)
                .is_some()
            {
                return Err(TransferError::SavedBlockEntity(pos));
            }
            if !replaceable || entity {
                return Err(TransferError::Revalidation(pos));
            }
        }
        Ok(())
    }

    /// The commit tail: the carrier file, the affected chunks and the
    /// flush barrier, then `committed`.
    fn finish_assemble(
        &mut self,
        store: &mut Store,
        uuid: u128,
        generation: u32,
        cells: &[BlockPos],
    ) -> Result<(), TransferError> {
        let _ = (uuid, generation);
        self.save_affected(cells)
            .map_err(|e| TransferError::Io(e.to_string()))?;
        #[cfg(any(test, feature = "test-support"))]
        if self.common.world.carrier_fences.fail_flush {
            return Err(TransferError::Io(
                "injected: the flush barrier failed after prepared".into(),
            ));
        }
        self.server
            .save
            .0
            .flush()
            .map_err(|e| TransferError::Io(e.to_string()))?;
        store
            .commit()
            .map_err(|e| TransferError::Store(e.to_string()))?;
        Ok(())
    }

    /// Whether every cell still holds exactly its image's state and
    /// entity, read straight from the live world.
    pub fn revalidate_cells(
        &self,
        cells: &[sunrise_sim::save::carrier::OperationCell],
        registry: &sunrise_world::registry::BlockRegistry,
    ) -> Result<(), TransferError> {
        for cell in cells {
            let pos = cell.world;
            if sunrise_gameplay_common::game::spill_common::saved_entity(&self.common.world, pos)
                .is_some()
            {
                return Err(TransferError::SavedBlockEntity(pos));
            }
            let state = self.common.world.block(pos[0], pos[1], pos[2]);
            let descriptor = registry
                .state(state)
                .ok_or(TransferError::Revalidation(pos))?;
            let same_state =
                descriptor.name == cell.before.name
                    && descriptor.properties.len() == cell.before.properties.len()
                    && cell.before.properties.iter().all(|(key, value)| {
                        descriptor.properties.get(key).is_some_and(|v| v == value)
                    });
            if !same_state {
                return Err(TransferError::Revalidation(pos));
            }
            let found_entity = self
                .common
                .world
                .block_entities
                .get(&(pos[0], pos[1], pos[2]));
            match (found_entity, &cell.before.entity) {
                (None, None) => {}
                // Presence alone is not a witness: the live entity must
                // serialize to exactly the image's content.
                (Some(live), Some(image)) => {
                    let name = registry
                        .state(state)
                        .map(|descriptor| descriptor.name.as_str())
                        .unwrap_or("");
                    let encoded = block_entity_encode::encode(
                        (pos[0], pos[1], pos[2]),
                        live,
                        name,
                        self.common.res.items(),
                    );
                    if !entities_content_equal(&encoded, image) {
                        return Err(TransferError::Revalidation(pos));
                    }
                }
                _ => return Err(TransferError::Revalidation(pos)),
            }
        }
        Ok(())
    }
    /// The world's carrier state at open (`Game::load_world_state`):
    /// the store's first-use gate reads the data folder's existence
    /// now — a folder created later this session (by the first save)
    /// does not pass for an old one — and a transfer that never
    /// finished recovers before the first tick and any player. A
    /// refusal fails the open, and no save runs after it.
    pub fn carrier_world_opened(&mut self) -> sunrise_game_data::Result<()> {
        self.common.world.carrier_data_existed_at_open =
            self.server.world_root.join("data").is_dir();
        self.server.carrier_retention_snapshot = match Store::open(&self.server.world_root) {
            Ok(sunrise_sim::save::carrier::Opened::Live(store)) => store.retention_snapshot(),
            Ok(sunrise_sim::save::carrier::Opened::NeverUsed) => Vec::new(),
            Err(error) => {
                self.common.world.carrier_fences.mark_broken();
                return Err(format!("world load refused: {error}").into());
            }
        };
        let result = self.carrier_recover_all_levels();
        if result.is_err() {
            self.common.world.carrier_fences.mark_broken();
        }
        result
    }

    /// `Game::break_block` behind the carrier fence: a cell held
    /// between prepared and committed refuses before the break runs.
    pub(in crate::game::server_side) fn carrier_break_block(
        &mut self,
        pos: [i32; 3],
    ) -> sunrise_game_data::Result<()> {
        if self.common.world.carrier_fences.locks_cell(&pos) {
            return Err("a carrier transfer holds this cell".into());
        }
        self.player_break_block(pos, None).map(|_| ())
    }

    /// Recovers the world's carrier store at open, before any chunk
    /// ticks and before any player joins: the level in place first, then
    /// every parked level through the same enter/return route the
    /// autosave uses. A refusal fails the world's load with a clear
    /// message — and nothing saves after it: the open itself aborts,
    /// while the store stays exactly as recovery found it.
    pub fn carrier_recover_all_levels(&mut self) -> sunrise_game_data::Result<()> {
        // No pending operation: no level is touched at all.
        let dimension = match sunrise_sim::save::carrier::Store::open(&self.server.world_root) {
            Ok(sunrise_sim::save::carrier::Opened::Live(store)) => {
                match store.journal().in_progress.as_ref() {
                    Some(op) => op.dimension.clone(),
                    // No pending operation — but retention still runs: the
                    // production cleanup clears finished disassemblies'
                    // tombstones and orphaned files at every open.
                    None => {
                        self.carrier_cleanup();
                        return Ok(());
                    }
                }
            }
            Ok(sunrise_sim::save::carrier::Opened::NeverUsed) => return Ok(()),
            Err(error) => return Err(format!("world load refused: {error}").into()),
        };
        // Recovery enters the operation's own dimension and no other: a
        // transfer in the Nether recovers in the Nether even when the
        // world opens in the Overworld with the Nether not parked -
        // enter_level opens an unheld level on demand. A dimension id
        // the world cannot hold at all refuses the load.
        let target = match dimension.as_str() {
            "minecraft:overworld" => sunrise_gameplay_common::game::Dim::Overworld,
            "minecraft:the_nether" => sunrise_gameplay_common::game::Dim::Nether,
            "minecraft:the_end" => sunrise_gameplay_common::game::Dim::End,
            _ => match sunrise_gameplay_common::game::Dim::extra(&dimension) {
                Some(dim) => dim,
                None => {
                    return Err(format!(
                        "world load refused: the pending carrier operation names \
                         {dimension}, which this world does not hold"
                    )
                    .into());
                }
            },
        };
        let back = self.common.dim;
        if target != back {
            self.enter_level(target).map_err(|e| {
                format!("world load refused: carrier recovery could not enter {dimension}: {e}")
            })?;
        }
        let recovered = self.carrier_recover_current();
        if target != back {
            self.enter_level(back).map_err(|e| {
                format!("world load refused: recovery could not return from {dimension}: {e}")
            })?;
        }
        recovered.map_err(|e| format!("world load refused: {e}"))?;
        // Production cleanup, as every open runs it: orphans, stale
        // operation files and tombstones of durably deleted carriers
        // clear here. New session deletions remain bounded by the journal's
        // capacity, not by an assumption that unlink is durable.
        self.carrier_cleanup();
        Ok(())
    }

    /// Cleans the store by the valid journal — the validated cleanup of
    /// the storage layer, run from world open. Errors are logged and
    /// swallowed: cleanup is retention work, never a load refusal.
    fn carrier_cleanup(&mut self) {
        let mut store = match sunrise_sim::save::carrier::Store::open(&self.server.world_root) {
            Ok(sunrise_sim::save::carrier::Opened::Live(store)) => store,
            _ => return,
        };
        if let Err(error) = store.cleanup() {
            log::warn!("carrier store cleanup: {error}");
        }
    }

    /// Recovers the level in place: no store means nothing to do; a live
    /// store's in-progress operation replays against this level's region
    /// directory, and only if the operation names this level.
    fn carrier_recover_current(&mut self) -> Result<(), String> {
        let mut store = match sunrise_sim::save::carrier::Store::open(&self.server.world_root) {
            Ok(sunrise_sim::save::carrier::Opened::Live(store)) => store,
            Ok(sunrise_sim::save::carrier::Opened::NeverUsed) => return Ok(()),
            Err(error) => return Err(error.to_string()),
        };
        if store.journal().in_progress.is_none() {
            return Ok(());
        }
        // SaveDir::root is the dimension folder; the regions live in its
        // region/ child.
        let region = self.server.save.0.root.join("region");
        let states =
            sunrise_sim::save::carrier::RegistryStates::new(self.common.pack.assets.registry);
        sunrise_sim::save::carrier::recover(&mut store, &region, &states, self.common.dim.id())
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Saves every affected chunk and waits for the writer's barrier, so
    /// the sectors are on disk with the regions synced.
    #[cfg_attr(not(test), allow(dead_code))]
    fn save_affected(&mut self, cells: &[BlockPos]) -> sunrise_game_data::Result<()> {
        let mut keys = Vec::new();
        for pos in cells {
            let key = (pos.x >> 4, pos.z >> 4);
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        let chunks = keys
            .iter()
            .filter_map(|key| self.common.world.chunks.get(key).cloned())
            .collect::<Vec<_>>();
        if chunks.is_empty() {
            return Ok(());
        }
        self.server.save.0.save_later(
            &chunks,
            self.common.pack.assets.registry,
            world_biomes!(self),
        )?;
        self.server.save.0.flush()
    }
}

/// Stops the session after a failed carrier commit, the same way any
/// fatal save error does: `Game::error` is set (the first error wins),
/// the event handler sees the error and exits the loop (`runtime/window_handler.rs`), and
/// every world save refuses while the fences are broken
/// (`save_game_with`, world/autosave.rs), so the exit path's save and
/// any autosave write nothing over the affected regions. The prepared
/// operation stays on disk for recovery at the next open.
fn carrier_stop_session(game: &mut crate::game::ServerGame<'_>, error: &TransferError) {
    if game.error.is_none() {
        game.error = Some(format!("carrier transfer failed: {error}").into());
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;

#[cfg(test)]
mod tests {
    /// The store's default capacity is the contract's 4,096.
    #[test]
    fn the_default_index_capacity_is_4096() {
        use sunrise_sim::save::carrier::MAX_CARRIERS;
        assert_eq!(MAX_CARRIERS, 4096);
        // The public constructors carry the default; a store built with
        // it enforces the same bound at encode, visible by refusing an
        // index past it.
        let root =
            std::env::temp_dir().join(format!("sunrise-capacity-default-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("data")).unwrap();
        let mut journal = sunrise_sim::save::carrier::Journal::default();
        for i in 0..=MAX_CARRIERS as u128 {
            journal.carriers.push((i, 1));
        }
        assert!(
            journal.encode_for_tests(MAX_CARRIERS).is_err(),
            "4097 entries must refuse at the default bound"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The #2488 obligation is what autosave and `save-all` refuse on:
    /// holding it refuses, releasing it admits again.
    #[test]
    fn the_save_obligation_gates_saves() {
        use sunrise_sim::join_seq::visibility::{PassLease, passes_settled};
        let _lease = PassLease::enter();
        assert!(!passes_settled());
        assert!(sunrise_gameplay_common::game::CommonGame::admit_entity_save().is_err());
        drop(_lease);
        assert!(passes_settled());
        assert!(sunrise_gameplay_common::game::CommonGame::admit_entity_save().is_ok());
    }
}
