// Included in carrier.rs to retain its private scope.
/// One side of a cell's image: the block state as the registry names it,
/// plus the cell's full block-entity NBT when it has one.
#[derive(Clone, Debug, PartialEq)]
pub struct CellImage {
    /// The state's namespaced block name.
    pub name: String,
    /// The state's properties, any order; part B validates them against
    /// the live registry through [`StateValidator`].
    pub properties: Vec<(String, String)>,
    /// The block entity's full NBT, if the cell owns one.
    pub entity: Option<Tag>,
}

/// Decides whether a state image is a state the live registry declares.
/// Part A ships the shape only; part B supplies the registry-backed one,
/// so a state can never be silently accepted or rewritten.
pub trait StateValidator {
    /// Whether the registry declares this name with these properties.
    fn known(&self, name: &str, properties: &[(String, String)]) -> bool;
}

/// Every state is known: part A's stand-in while the registry wiring is
/// part B's.
pub struct AllStatesKnown;
impl StateValidator for AllStatesKnown {
    fn known(&self, _name: &str, _properties: &[(String, String)]) -> bool {
        true
    }
}

/// One cell of an operation: its world and local positions, and its
/// before and after images.
#[derive(Clone, Debug, PartialEq)]
pub struct OperationCell {
    /// The world cell the transfer touches.
    pub world: [i32; 3],
    /// The carrier-local cell it maps to.
    pub local: [i8; 3],
    /// The cell before the transfer.
    pub before: CellImage,
    /// The cell after the transfer.
    pub after: CellImage,
}

/// One transfer's file: what assembly or disassembly does to every cell
/// it touches, so replay can finish or strip exactly one side.
#[derive(Clone, Debug, PartialEq)]
pub struct Operation {
    /// The operation's id, from the journal's `next_op_id`.
    pub op_id: u64,
    /// Whether it assembles or disassembles.
    pub kind: OpKind,
    /// The carrier it moves.
    pub uuid: u128,
    /// The carrier's generation after this operation.
    pub generation: u32,
    /// The dimension the operation runs in; the journal's record must
    /// name the same one or the world's load refuses.
    pub dimension: String,
    /// The touched cells, at most [`MAX_OP_CELLS`].
    pub cells: Vec<OperationCell>,
}

/// Why an operation file could not be encoded or decoded.
#[derive(Clone, Debug, PartialEq)]
pub enum OpError {
    /// The decoded NBT would pass [`MAX_OP_DECODED`].
    DecodedTooBig {
        /// The offending size.
        size: usize,
    },
    /// The compressed file would pass [`MAX_OP_COMPRESSED`].
    CompressedTooBig {
        /// The offending size.
        size: usize,
    },
    /// The file or its NBT does not match the schema.
    Malformed(&'static str),
    /// More cells than [`MAX_OP_CELLS`].
    CellCount {
        /// The offending count.
        count: usize,
    },
    /// A block-entity image past [`MAX_CELL_ENTITY`].
    EntityTooBig {
        /// The offending size.
        size: usize,
    },
    /// The block-entity images past [`MAX_OP_ENTITIES`] in total.
    EntitiesTooBig {
        /// The offending total.
        total: usize,
    },
    /// A state image the validator does not know.
    UnknownState,
    /// A state string (block id, property name or value) past the NBT
    /// string limit, which the disk writer would truncate to empty.
    StringTooLong {
        /// The offending length.
        len: usize,
    },
}
impl fmt::Display for OpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DecodedTooBig { size } => {
                write!(f, "operation decodes to {size} bytes over {MAX_OP_DECODED}")
            }
            Self::CompressedTooBig { size } => {
                write!(
                    f,
                    "operation compresses to {size} bytes over {MAX_OP_COMPRESSED}"
                )
            }
            Self::Malformed(what) => write!(f, "operation file malformed: {what}"),
            Self::CellCount { count } => write!(f, "{count} cells exceeds {MAX_OP_CELLS}"),
            Self::EntityTooBig { size } => {
                write!(
                    f,
                    "block entity image of {size} bytes exceeds {MAX_CELL_ENTITY}"
                )
            }
            Self::EntitiesTooBig { total } => {
                write!(
                    f,
                    "block entity images total {total} bytes over {MAX_OP_ENTITIES}"
                )
            }
            Self::UnknownState => write!(f, "unknown block state image"),
            Self::StringTooLong { len } => write!(
                f,
                "state string of {len} bytes exceeds the NBT string limit"
            ),
        }
    }
}

/// The NBT read/write budgets for one operation file.
fn op_limits() -> Limits {
    Limits {
        max_bytes: MAX_OP_DECODED,
        max_nodes: MAX_OP_NODES,
        max_depth: MAX_OP_DEPTH,
    }
}

/// Encodes the whole operation into memory and refuses it there, before
/// any block is lifted: the decoded NBT must fit [`MAX_OP_DECODED`] and
/// the compressed file [`MAX_OP_COMPRESSED`], so a transfer that cannot
/// be stored never starts.
pub fn encode_operation(operation: &Operation) -> Result<Vec<u8>, OpError> {
    if operation.generation > i32::MAX as u32 {
        return Err(OpError::Malformed("generation exhausted"));
    }
    if operation.op_id > i64::MAX as u64 {
        return Err(OpError::Malformed("operation id exhausted"));
    }
    if operation.cells.len() > MAX_OP_CELLS {
        return Err(OpError::CellCount {
            count: operation.cells.len(),
        });
    }
    // One NBT string each: a string past the wire limit would be written
    // empty, so the refusal happens here, before any block is lifted.
    // The limit is the model's own constant, the same one Cell::new
    // enforces for the carrier codec.
    use sunrise_world::carrier::model::MAX_NBT_STRING_BYTES as NBT_STRING_LIMIT;
    let string = |text: &str| -> Result<(), OpError> {
        if text.len() > NBT_STRING_LIMIT {
            return Err(OpError::StringTooLong { len: text.len() });
        }
        Ok(())
    };
    for cell in &operation.cells {
        for image in [&cell.before, &cell.after] {
            string(&image.name)?;
            for (key, value) in &image.properties {
                string(key)?;
                string(value)?;
            }
        }
    }
    let mut entity_bytes = 0_usize;
    for cell in &operation.cells {
        for image in [&cell.before, &cell.after] {
            if let Some(entity) = &image.entity {
                let size = nbt::write_named(
                    &NamedTag {
                        name: "".into(),
                        tag: entity.clone(),
                    },
                    op_limits(),
                )
                .map_err(|_| OpError::Malformed("block entity does not encode"))?
                .len();
                if size > MAX_CELL_ENTITY {
                    return Err(OpError::EntityTooBig { size });
                }
                entity_bytes += size;
            }
        }
    }
    if entity_bytes > MAX_OP_ENTITIES {
        return Err(OpError::EntitiesTooBig {
            total: entity_bytes,
        });
    }
    let raw = nbt::write_named(
        &NamedTag {
            name: "".into(),
            tag: operation_tag(operation),
        },
        op_limits(),
    )
    .map_err(|e| OpError::Malformed(leak(e.reason)))?;
    if raw.len() > MAX_OP_DECODED {
        return Err(OpError::DecodedTooBig { size: raw.len() });
    }
    let compressed = Compression::Gzip
        .encode(&raw)
        .map_err(|_| OpError::Malformed("operation does not compress"))?;
    if compressed.len() > MAX_OP_COMPRESSED {
        return Err(OpError::CompressedTooBig {
            size: compressed.len(),
        });
    }
    Ok(compressed)
}

/// Decodes an operation file: exactly one gzip member, with the compressed
/// size checked before decompression, each length bounded before allocation,
/// and state images checked by the validator.
pub fn decode_operation(bytes: &[u8], states: &dyn StateValidator) -> Result<Operation, OpError> {
    if bytes.len() > MAX_OP_COMPRESSED {
        return Err(OpError::CompressedTooBig { size: bytes.len() });
    }
    let raw = decode_single_gzip_member(bytes, MAX_OP_DECODED)
        .map_err(|_| OpError::Malformed("operation does not decompress"))?;
    let (root, used) =
        nbt::read_named(&raw, op_limits()).map_err(|e| OpError::Malformed(leak(e.reason)))?;
    if used != raw.len() {
        return Err(OpError::Malformed("bytes follow the operation root"));
    }
    let root = root.tag;
    if !matches!(root, Tag::Compound(_)) {
        return Err(OpError::Malformed("root is not a compound"));
    }
    let int = |key: &'static str| -> Result<i32, OpError> {
        match root.get(key) {
            Some(Tag::Int(value)) => Ok(*value),
            _ => Err(OpError::Malformed(leak_key(key))),
        }
    };
    let long = |key: &'static str| -> Result<i64, OpError> {
        match root.get(key) {
            Some(Tag::Long(value)) => Ok(*value),
            _ => Err(OpError::Malformed(leak_key(key))),
        }
    };
    if int("version")? != OP_VERSION {
        return Err(OpError::Malformed("unsupported operation version"));
    }
    let kind = match root.get("kind") {
        Some(Tag::String(value)) => match value.to_string().as_deref() {
            Ok("assemble") => OpKind::Assemble,
            Ok("disassemble") => OpKind::Disassemble,
            _ => return Err(OpError::Malformed("unknown operation kind")),
        },
        _ => return Err(OpError::Malformed(leak_key("kind"))),
    };
    let op_id = u64::try_from(long("op_id")?).map_err(|_| OpError::Malformed(leak_key("op_id")))?;
    let uuid = (((long("carrier_uuid_msb")? as u64) as u128) << 64)
        | (long("carrier_uuid_lsb")? as u64) as u128;
    let generation = u32::try_from(int("carrier_generation")?)
        .map_err(|_| OpError::Malformed(leak_key("carrier_generation")))?;
    let dimension = match root.get("dimension") {
        Some(Tag::String(value)) => value
            .to_string()
            .map_err(|_| OpError::Malformed("dimension is not valid UTF-16"))?,
        _ => return Err(OpError::Malformed(leak_key("dimension"))),
    };
    let cells_tag = match root.get("cells") {
        Some(Tag::List(cells)) => cells,
        _ => return Err(OpError::Malformed(leak_key("cells"))),
    };
    // The cell count is checked before the vector is allocated.
    if cells_tag.len() > MAX_OP_CELLS {
        return Err(OpError::CellCount {
            count: cells_tag.len(),
        });
    }
    let mut cells = Vec::with_capacity(cells_tag.len());
    let mut entity_bytes = 0_usize;
    for cell_tag in cells_tag {
        let Tag::Compound(cell_map) = cell_tag else {
            return Err(OpError::Malformed("cell is not a compound"));
        };
        let world = match cell_map.get(&"world".into()) {
            Some(Tag::IntArray(values)) if values.len() == 3 => [values[0], values[1], values[2]],
            _ => return Err(OpError::Malformed(leak_key("world"))),
        };
        let local = match cell_map.get(&"local".into()) {
            Some(Tag::ByteArray(values)) if values.len() == 3 => {
                [values[0] as i8, values[1] as i8, values[2] as i8]
            }
            _ => return Err(OpError::Malformed(leak_key("local"))),
        };
        let before = decode_image(cell_map.get(&"before".into()), states, &mut entity_bytes)?;
        let after = decode_image(cell_map.get(&"after".into()), states, &mut entity_bytes)?;
        cells.push(OperationCell {
            world,
            local,
            before,
            after,
        });
    }
    if entity_bytes > MAX_OP_ENTITIES {
        return Err(OpError::EntitiesTooBig {
            total: entity_bytes,
        });
    }
    Ok(Operation {
        op_id,
        kind,
        uuid,
        generation,
        dimension,
        cells,
    })
}

/// One cell image read from a cell compound.
fn decode_image(
    tag: Option<&Tag>,
    states: &dyn StateValidator,
    entity_bytes: &mut usize,
) -> Result<CellImage, OpError> {
    let Tag::Compound(map) = tag.ok_or(OpError::Malformed(leak_key("image")))? else {
        return Err(OpError::Malformed("image is not a compound"));
    };
    let name = match map.get(&"state".into()) {
        Some(Tag::String(value)) => value
            .to_string()
            .map_err(|_| OpError::Malformed("state name is not valid UTF-16"))?,
        _ => return Err(OpError::Malformed(leak_key("state"))),
    };
    let mut properties = Vec::new();
    if let Some(properties_map) = map.get(&"properties".into()) {
        let Tag::Compound(properties_map) = properties_map else {
            return Err(OpError::Malformed(leak_key("properties")));
        };
        // Bounded by the already-parsed compound's own node budget.
        properties = properties_map
            .iter()
            .map(|(key, value)| match value {
                Tag::String(value) => Ok((
                    key.to_string()
                        .map_err(|_| OpError::Malformed("property name is not valid UTF-16"))?,
                    value
                        .to_string()
                        .map_err(|_| OpError::Malformed("property value is not valid UTF-16"))?,
                )),
                _ => Err(OpError::Malformed("property is not a string")),
            })
            .collect::<Result<Vec<_>, _>>()?;
    }
    if !states.known(&name, &properties) {
        return Err(OpError::UnknownState);
    }
    let entity = match map.get(&"entity".into()) {
        Some(entity) => {
            let size = nbt::write_named(
                &NamedTag {
                    name: "".into(),
                    tag: entity.clone(),
                },
                op_limits(),
            )
            .map_err(|e| OpError::Malformed(leak(e.reason)))?
            .len();
            if size > MAX_CELL_ENTITY {
                return Err(OpError::EntityTooBig { size });
            }
            *entity_bytes += size;
            Some(entity.clone())
        }
        None => None,
    };
    Ok(CellImage {
        name,
        properties,
        entity,
    })
}

/// The operation's NBT tree.
fn operation_tag(operation: &Operation) -> Tag {
    let cells = operation
        .cells
        .iter()
        .map(|cell| {
            let image = |image: &CellImage| {
                Tag::compound(
                    [("state", Tag::String(image.name.as_str().into()))]
                        .into_iter()
                        .chain((!image.properties.is_empty()).then(|| {
                            (
                                "properties",
                                Tag::compound(
                                    image
                                        .properties
                                        .iter()
                                        .map(|(key, value)| {
                                            (key.as_str(), Tag::String(value.as_str().into()))
                                        })
                                        .collect::<Vec<_>>(),
                                ),
                            )
                        }))
                        .chain(image.entity.clone().map(|entity| ("entity", entity))),
                )
            };
            Tag::compound([
                (
                    "world",
                    Tag::IntArray(vec![cell.world[0], cell.world[1], cell.world[2]]),
                ),
                (
                    "local",
                    Tag::ByteArray(vec![
                        cell.local[0] as u8,
                        cell.local[1] as u8,
                        cell.local[2] as u8,
                    ]),
                ),
                ("before", image(&cell.before)),
                ("after", image(&cell.after)),
            ])
        })
        .collect::<Vec<_>>();
    Tag::compound([
        ("version", Tag::Int(OP_VERSION)),
        ("op_id", Tag::Long(operation.op_id as i64)),
        ("kind", Tag::String(operation.kind.as_str().into())),
        (
            "carrier_uuid_msb",
            Tag::Long((operation.uuid >> 64) as u64 as i64),
        ),
        ("carrier_uuid_lsb", Tag::Long(operation.uuid as u64 as i64)),
        ("carrier_generation", Tag::Int(operation.generation as i32)),
        (
            "dimension",
            Tag::String(operation.dimension.as_str().into()),
        ),
        ("cells", Tag::List(cells)),
    ])
}
