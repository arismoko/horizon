// Included in carrier.rs to retain its private scope.
/// What a recovery pass found and did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Recovered {
    /// Chunks whose cells still held their before-images and were taken
    /// to the after-images.
    pub applied: usize,
    /// Chunks that already held their after-images untouched.
    pub verified: usize,
}

/// Why a recovery refused. Every variant but `NothingToDo` stops the
/// world's load: recovery never guesses, and never generates content.
#[derive(Debug)]
pub enum RecoveryRefused {
    /// The store itself refused to open.
    Store(Refusal),
    /// An in-progress operation's file is gone. By the approved contract
    /// that is a world-load refusal — the file was durable before
    /// `prepared`, so its absence is damage — never a quarantine.
    /// Quarantine belongs to a committed carrier whose content is
    /// unknown, which recovery here does not touch.
    MissingOperation,
    /// An in-progress operation's file does not decode. As with
    /// [`RecoveryRefused::MissingOperation`], the world's load refuses.
    OperationUnreadable(String),
    /// An affected chunk did not strictly read — damage, never absence.
    Region {
        /// The chunk's position.
        chunk: (i32, i32),
        /// Why it refused.
        why: sunrise_world::carrier::region_strict::Strict,
    },
    /// A cell holds neither its before-image nor its after-image.
    CellMismatch {
        /// The cell's world position.
        pos: [i32; 3],
        /// What the cell actually holds, serialized.
        found: String,
    },
    /// The chunk NBT did not parse after a strict read.
    Chunk(String),
    /// The operation belongs to another dimension than the level being
    /// recovered; nothing was touched.
    NotThisDimension(String),
    /// The journal and the operation file name different dimensions:
    /// damage, so the world's load refuses.
    DimensionDisagreement {
        /// What the journal says.
        journal: String,
        /// What the operation file says.
        operation: String,
    },
    /// The operation file's identity (id, kind, UUID or generation)
    /// disagrees with the journal's prepared record: damage, so the
    /// world's load refuses.
    IdentityDisagreement {
        /// Which field disagreed.
        field: &'static str,
    },
    /// The prepared carrier's cargo disagrees with the operation's
    /// before-image: cell count, positions, names or entity content.
    CargoDisagreement {
        /// What disagreed.
        what: &'static str,
    },
    /// Plain I/O.
    Io(String),
}
impl fmt::Display for RecoveryRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(refusal) => write!(f, "{refusal}"),
            Self::MissingOperation => write!(
                f,
                "world load refused: the in-progress operation's file is gone"
            ),
            Self::OperationUnreadable(why) => {
                write!(
                    f,
                    "world load refused: the operation file does not decode: {why}"
                )
            }
            Self::Region { chunk, why } => {
                write!(f, "chunk ({},{}) refuses: {why}", chunk.0, chunk.1)
            }
            Self::CellMismatch { pos, found } => write!(
                f,
                "cell ({},{},{}) holds {found}, neither its before nor its after image",
                pos[0], pos[1], pos[2]
            ),
            Self::Chunk(why) => write!(f, "chunk data: {why}"),
            Self::NotThisDimension(dimension) => write!(
                f,
                "world load refused: the operation belongs to {dimension}, not this level"
            ),
            Self::DimensionDisagreement { journal, operation } => write!(
                f,
                "world load refused: the journal names {journal} but the operation file names {operation}"
            ),
            Self::IdentityDisagreement { field } => write!(
                f,
                "world load refused: the operation file's {field} disagrees with the journal"
            ),
            Self::CargoDisagreement { what } => write!(
                f,
                "world load refused: the prepared carrier's cargo disagrees: {what}"
            ),
            Self::Io(why) => write!(f, "recovery I/O: {why}"),
        }
    }
}

/// Replays the in-progress operation at world open, before chunks tick
/// and before players join: every affected chunk is strictly read, each
/// cell must hold either its before-image (apply the after-image) or its
/// after-image (leave it), and only when every write-back is synced does
/// the journal record `committed` and drop the operation file.
/// Roll-forward only — nothing is ever written back into a cell.
pub fn recover(
    store: &mut Store,
    region_dir: &Path,
    states: &dyn StateValidator,
    dimension: &str,
) -> Result<Recovered, RecoveryRefused> {
    let Some(op) = &store.journal().in_progress else {
        return Ok(Recovered::default());
    };
    let op_path = store.operation_path(op.op_id);
    let bytes = read_bounded(&op_path, MAX_OP_COMPRESSED).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => RecoveryRefused::MissingOperation,
        _ => RecoveryRefused::OperationUnreadable(error.to_string()),
    })?;
    let operation = decode_operation(&bytes, states)
        .map_err(|e| RecoveryRefused::OperationUnreadable(e.to_string()))?;
    // The operation file's identity must be exactly the journal's
    // prepared record: id, kind, carrier and generation, not only the
    // dimension.
    if operation.op_id != op.op_id {
        return Err(RecoveryRefused::IdentityDisagreement { field: "op_id" });
    }
    if operation.kind != op.kind {
        return Err(RecoveryRefused::IdentityDisagreement { field: "kind" });
    }
    if operation.uuid != op.uuid {
        return Err(RecoveryRefused::IdentityDisagreement { field: "uuid" });
    }
    if operation.generation != op.generation {
        return Err(RecoveryRefused::IdentityDisagreement {
            field: "generation",
        });
    }
    // The journal and the operation file must name one dimension, and
    // it must be the level recovery is running in: anything else is a
    // refusal that touches nothing.
    if operation.dimension != op.dimension {
        return Err(RecoveryRefused::DimensionDisagreement {
            journal: op.dimension.clone(),
            operation: operation.dimension.clone(),
        });
    }
    if operation.dimension != dimension {
        return Err(RecoveryRefused::NotThisDimension(
            operation.dimension.clone(),
        ));
    }
    // An assembling operation publishes its immutable carrier before
    // `prepared`: recovery verifies that file is in place with the
    // journal's UUID and generation — the cargo has exactly one
    // durable owner before the source is stripped — and refuses on
    // anything else. Nothing else here rewrites the carrier; the live
    // writer did, before prepared.
    if operation.kind == OpKind::Assemble {
        let path = store.carrier_path(operation.uuid, operation.generation);
        let bytes = read_bounded(&path, sunrise_world::carrier::codec::MAX_INPUT_BYTES).map_err(
            |error| {
                RecoveryRefused::OperationUnreadable(format!(
                    "the prepared assembly's carrier file does not read: {error}"
                ))
            },
        )?;
        let carrier = sunrise_world::carrier::codec::decode(&bytes).map_err(|e| {
            RecoveryRefused::OperationUnreadable(format!(
                "the prepared assembly's carrier does not decode: {e}"
            ))
        })?;
        if carrier.uuid() != operation.uuid || carrier.generation() != operation.generation {
            return Err(RecoveryRefused::IdentityDisagreement { field: "carrier" });
        }
        // The cargo itself must match the operation's before-image —
        // cell count, names and entity content — so a carrier that is
        // short of cells, or carries different ones, never strips a
        // source it cannot put back.
        if carrier.cells().len() != operation.cells.len() {
            return Err(RecoveryRefused::CargoDisagreement { what: "cell count" });
        }
        for cell in &operation.cells {
            let local = [cell.local[0], cell.local[1], cell.local[2]];
            let Some(carried) = carrier.cells().iter().find(|c| {
                [c.pos().x(), c.pos().y(), c.pos().z()] == local
                    && c.name() == cell.before.name
                    && c.properties().len() == cell.before.properties.len()
                    && c.properties().iter().all(|(key, value)| {
                        cell.before
                            .properties
                            .iter()
                            .any(|(k, v)| key.as_ref() == k && value.as_ref() == v)
                    })
            }) else {
                return Err(RecoveryRefused::CargoDisagreement { what: "cell state" });
            };
            let carried_entity = carried.data().map(|data| {
                sunrise_core::nbt::read_named(data, sunrise_core::nbt::Limits::saved())
                    .map(|(named, _)| named.tag)
            });
            let same_entity = match (&carried_entity, &cell.before.entity) {
                (None, None) => true,
                (Some(Ok(found)), Some(expected)) => entities_equal(found, expected),
                _ => false,
            };
            if !same_entity {
                return Err(RecoveryRefused::CargoDisagreement {
                    what: "entity content",
                });
            }
        }
    }
    let mut chunks: Vec<[i32; 2]> = Vec::new();
    for cell in &operation.cells {
        let key = [cell.world[0] >> 4, cell.world[2] >> 4];
        if !chunks.contains(&key) {
            chunks.push(key);
        }
    }
    let mut recovered = Recovered::default();
    for chunk in chunks {
        let root = sunrise_world::carrier::region_strict::read_chunk(
            region_dir,
            sunrise_core::position::ChunkPos::new(chunk[0], chunk[1]),
        )
        .map_err(|why| RecoveryRefused::Region {
            chunk: (chunk[0], chunk[1]),
            why,
        })?
        .ok_or(RecoveryRefused::Region {
            chunk: (chunk[0], chunk[1]),
            why: sunrise_world::carrier::region_strict::Strict::Missing,
        })?;
        if replay_chunk(&operation, root, chunk, region_dir)? {
            recovered.applied += 1;
        } else {
            recovered.verified += 1;
        }
    }
    store
        .commit()
        .map_err(|e| RecoveryRefused::Io(e.to_string()))?;
    Ok(recovered)
}

/// Applies one chunk's cells: strict compare against the images, the
/// after-image written through the storage-level chunk (no block
/// behavior, exactly what a replay may do), then a synced write-back.
fn replay_chunk(
    operation: &Operation,
    root: sunrise_core::nbt::NamedTag,
    chunk: [i32; 2],
    region_dir: &Path,
) -> Result<bool, RecoveryRefused> {
    let entities = root_entities(&root);
    let mut data = sunrise_world::chunk::ChunkData::new(root).map_err(|e| {
        RecoveryRefused::Chunk(format!(
            "chunk ({},{}) does not parse: {e}",
            chunk[0], chunk[1]
        ))
    })?;
    let mut touched = false;
    for cell in &operation.cells {
        if [cell.world[0] >> 4, cell.world[2] >> 4] != chunk {
            continue;
        }
        let pos =
            sunrise_core::position::BlockPos::new(cell.world[0], cell.world[1], cell.world[2]);
        let found = data.block(pos).map_err(|e| {
            RecoveryRefused::Chunk(format!(
                "cell ({},{},{}) does not read: {e}",
                pos.x, pos.y, pos.z
            ))
        })?;
        let entity = entities
            .iter()
            .find(|tag| entity_at(tag, [pos.x, pos.y, pos.z]))
            .cloned();
        // Exact witnesses: the state AND the block entity must equal
        // the image's. A matching state over a different entity is a
        // mismatch, never a silent pass.
        let state_is = |image: &CellImage| match &found {
            None => image.name == "minecraft:air" && image.properties.is_empty(),
            Some(state) => {
                state.name == image.name
                    && state.properties.len() == image.properties.len()
                    && image
                        .properties
                        .iter()
                        .all(|(key, value)| state.properties.get(key).is_some_and(|v| v == value))
            }
        };
        let entity_is = |image: &CellImage| match (&entity, &image.entity) {
            (None, None) => true,
            (Some(found), Some(expected)) => entities_equal(found, expected),
            _ => false,
        };
        if state_is(&cell.after) && entity_is(&cell.after) {
            continue;
        }
        if !(state_is(&cell.before) && entity_is(&cell.before)) {
            let found = found
                .map(|state| {
                    let mut text = state.name.clone();
                    for (key, value) in &state.properties {
                        text.push_str(&format!(",{key}={value}"));
                    }
                    if entity.is_some() {
                        text.push_str(",+entity");
                    }
                    text
                })
                .unwrap_or_else(|| "minecraft:air".to_string());
            return Err(RecoveryRefused::CellMismatch {
                pos: [pos.x, pos.y, pos.z],
                found,
            });
        }
        let after = sunrise_world::chunk::BlockState {
            name: cell.after.name.clone(),
            properties: cell.after.properties.iter().cloned().collect(),
        };
        data.set_block_data(pos, after).map_err(|e| {
            RecoveryRefused::Chunk(format!(
                "cell ({},{},{}) does not write: {e}",
                pos.x, pos.y, pos.z
            ))
        })?;
        touched = true;
    }
    if !touched {
        return Ok(false);
    }
    // Block entities ride the root's own list: the after-image's entity
    // is installed, the before position's cleared, by plain NBT edits
    // so nothing else in the chunk moves.
    let root = data.into_nbt();
    let root = edit_block_entities(root, operation, chunk)?;
    write_chunk_back(region_dir, root, chunk)?;
    Ok(true)
}

/// The chunk root's `block_entities` list, by value.
fn root_entities(root: &sunrise_core::nbt::NamedTag) -> Vec<sunrise_core::nbt::Tag> {
    match root.tag.get("block_entities") {
        Some(sunrise_core::nbt::Tag::List(list)) => list.clone(),
        _ => Vec::new(),
    }
}

/// Whether an entity tag sits at `pos`.
fn entity_at(tag: &sunrise_core::nbt::Tag, pos: [i32; 3]) -> bool {
    let read = |name: &str| match tag.get(name) {
        Some(sunrise_core::nbt::Tag::Int(value)) => Some(*value),
        _ => None,
    };
    read("x") == Some(pos[0]) && read("y") == Some(pos[1]) && read("z") == Some(pos[2])
}

/// Whether two serialized block entities carry the same content: the
/// exact replay witness. The positional `x`/`y`/`z` ints are placement,
/// not content — the stored image keeps its own coordinates — so the
/// comparison strips them and then compares everything else.
fn entities_equal(a: &sunrise_core::nbt::Tag, b: &sunrise_core::nbt::Tag) -> bool {
    let strip = |tag: &sunrise_core::nbt::Tag| -> sunrise_core::nbt::Tag {
        let mut clone = tag.clone();
        if let sunrise_core::nbt::Tag::Compound(map) = &mut clone {
            for key in ["x", "y", "z"] {
                map.remove(&sunrise_core::nbt::JavaString::from(key));
            }
        }
        clone
    };
    sunrise_core::nbt::compare_nbt(&strip(a), Some(&strip(b)))
}

/// Installs the after-images' block entities and clears the before
/// positions' in the chunk root, by NBT edits.
fn edit_block_entities(
    mut root: sunrise_core::nbt::NamedTag,
    operation: &Operation,
    chunk: [i32; 2],
) -> Result<sunrise_core::nbt::NamedTag, RecoveryRefused> {
    use sunrise_core::nbt::Tag;
    let Tag::Compound(map) = &mut root.tag else {
        return Err(RecoveryRefused::Chunk(
            "chunk root is not a compound".into(),
        ));
    };
    let mut entities = match map.get(&"block_entities".into()) {
        Some(Tag::List(list)) => list.clone(),
        _ => Vec::new(),
    };
    for cell in &operation.cells {
        if [cell.world[0] >> 4, cell.world[2] >> 4] != chunk {
            continue;
        }
        let (x, y, z) = (cell.world[0], cell.world[1], cell.world[2]);
        let at = |tag: &Tag| -> bool {
            let read = |name: &str| match tag.get(name) {
                Some(Tag::Int(v)) => Some(*v),
                _ => None,
            };
            read("x") == Some(x) && read("y") == Some(y) && read("z") == Some(z)
        };
        entities.retain(|tag| !at(tag));
        if let Some(entity) = &cell.after.entity {
            let Tag::Compound(mut entity_map) = entity.clone() else {
                return Err(RecoveryRefused::Chunk(
                    "after-image entity is not a compound".into(),
                ));
            };
            for (key, value) in [("x", x), ("y", y), ("z", z)] {
                entity_map.insert(key.into(), Tag::Int(value));
            }
            entities.push(Tag::Compound(entity_map));
        }
    }
    if entities.is_empty() {
        map.remove(&"block_entities".into());
    } else {
        map.insert("block_entities".into(), Tag::List(entities));
    }
    Ok(root)
}

/// Writes the replayed chunk back through the region file, synced: the
/// write must be on disk before the journal says `committed`.
fn write_chunk_back(
    region_dir: &Path,
    root: sunrise_core::nbt::NamedTag,
    chunk: [i32; 2],
) -> Result<(), RecoveryRefused> {
    use sunrise_world::region::ChunkRecord;
    let position = sunrise_core::position::ChunkPos::new(chunk[0], chunk[1]);
    let record = ChunkRecord::from_nbt(
        &root,
        Compression::Zlib,
        0,
        sunrise_core::nbt::Limits::saved(),
    )
    .map_err(|e| RecoveryRefused::Chunk(e.to_string()))?;
    let path = region_dir.join(format!("r.{}.{}.mca", position.x >> 5, position.z >> 5));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| RecoveryRefused::Io(e.to_string()))?;
    }
    let mut region = sunrise_world::region::RegionFile::open_writable(&path, true)
        .map_err(|e| RecoveryRefused::Io(e.to_string()))?;
    region
        .write(position, &record)
        .map_err(|e| RecoveryRefused::Io(e.to_string()))?;
    region
        .sync()
        .map_err(|e| RecoveryRefused::Io(e.to_string()))?;
    Ok(())
}
