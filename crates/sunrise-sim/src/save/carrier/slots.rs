// Included in carrier.rs to retain its private scope.
/// A slot's record number plus body, with the slot it was read from.
#[derive(Clone, Debug)]
struct SlotRecord {
    slot: usize,
    entry_no: u64,
    body: Journal,
    error: Option<SlotError>,
}
impl SlotRecord {
    const fn valid(&self) -> bool {
        self.error.is_none()
    }
    fn error(&self) -> SlotError {
        self.error.unwrap_or(SlotError::Invalid)
    }
}

/// Why a slot does not hold a valid record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlotError {
    /// The bytes are not a complete, CRC-correct record of this schema
    /// family (a torn or corrupt write, or junk).
    Invalid,
    /// The bytes decode as a record from a newer schema.
    NewerSchema { found: u16 },
}

/// Reads both slots. `Ok(None)` when neither file exists.
fn read_slots(data: &Path) -> Result<Option<[SlotRecord; 2]>, Refusal> {
    let bytes = [read_slot_file(data, 0)?, read_slot_file(data, 1)?];
    match (&bytes[0], &bytes[1]) {
        (None, None) => Ok(None),
        (a, b) => Ok(Some([
            read_slot(0, a.as_deref().unwrap_or(&[])),
            read_slot(1, b.as_deref().unwrap_or(&[])),
        ])),
    }
}

/// One slot file's bytes, `None` when the file does not exist. A file
/// past the slot cap is read as empty — an invalid record — without
/// reading its bytes.
fn read_slot_file(data: &Path, slot: usize) -> Result<Option<Vec<u8>>, Refusal> {
    let path = data.join(SLOTS[slot]);
    if fs::metadata(&path)
        .map(|meta| meta.len() as usize > MAX_SLOT_FILE)
        .unwrap_or(false)
    {
        return Ok(Some(Vec::new()));
    }
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Refusal::Io(error.to_string())),
    }
}

/// One slot read: header, lengths, CRC and body, each checked before the
/// memory behind it is touched.
fn read_slot(slot: usize, bytes: &[u8]) -> SlotRecord {
    let invalid = |entry_no| SlotRecord {
        slot,
        entry_no,
        body: Journal::empty(),
        error: Some(SlotError::Invalid),
    };
    let newer = |found| SlotRecord {
        slot,
        entry_no: 0,
        body: Journal::empty(),
        error: Some(SlotError::NewerSchema { found }),
    };
    if bytes.len() < SLOT_HEADER || bytes[..4] != SLOT_MAGIC {
        return invalid(0);
    }
    let entry_no = u64::from_le_bytes(bytes[6..14].try_into().unwrap());
    let body_len = u32::from_le_bytes(bytes[14..18].try_into().unwrap()) as usize;
    let crc = u32::from_le_bytes(bytes[18..22].try_into().unwrap());
    // The declared body is checked against the slot cap and the bytes
    // actually present before anything is allocated for it.
    if body_len > MAX_SLOT_FILE - SLOT_HEADER || bytes.len() - SLOT_HEADER != body_len {
        return invalid(entry_no);
    }
    // The CRC covers everything after the magic: the version, the record
    // number, the body length and the body. A single flipped bit anywhere
    // makes the record invalid — an older record can never flip into
    // looking newer, and a corrupted version byte is not a schema.
    if slot_crc(bytes) != crc {
        return invalid(entry_no);
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version > SLOT_VERSION {
        return newer(version);
    }
    let body = &bytes[SLOT_HEADER..];
    match decode_journal(body) {
        Some(journal) => SlotRecord {
            slot,
            entry_no,
            body: journal,
            error: None,
        },
        None => invalid(entry_no),
    }
}

/// Encodes one slot: header, then the CRC-protected body. The body is
/// checked against the slot cap before the file is written.
fn encode_slot(entry_no: u64, journal: &Journal, capacity: usize) -> Result<Vec<u8>, String> {
    let body = encode_journal(journal, capacity)?;
    let len = SLOT_HEADER + body.len();
    if len > MAX_SLOT_FILE {
        return Err(format!(
            "journal record of {len} bytes exceeds {MAX_SLOT_FILE}"
        ));
    }
    let mut bytes = Vec::with_capacity(len);
    bytes.extend_from_slice(&SLOT_MAGIC);
    bytes.extend_from_slice(&SLOT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&entry_no.to_le_bytes());
    bytes.extend_from_slice(&(body.len() as u32).to_le_bytes());
    bytes.extend_from_slice(
        &slot_crc_input(SLOT_VERSION, entry_no, body.len(), &body).to_le_bytes(),
    );
    bytes.extend_from_slice(&body);
    Ok(bytes)
}

/// The slot CRC's input: everything after the magic — version, record
/// number, body length — followed by the body itself.
fn slot_crc_input(version: u16, entry_no: u64, body_len: usize, body: &[u8]) -> u32 {
    let mut input = Vec::with_capacity(14 + body.len());
    input.extend_from_slice(&version.to_le_bytes());
    input.extend_from_slice(&entry_no.to_le_bytes());
    input.extend_from_slice(&(body_len as u32).to_le_bytes());
    input.extend_from_slice(body);
    Crc32::checksum(&input)
}

/// The CRC of an encoded slot record, [`slot_crc_input`] over its own
/// bytes.
fn slot_crc(bytes: &[u8]) -> u32 {
    let body_len = u32::from_le_bytes(bytes[14..18].try_into().unwrap()) as usize;
    slot_crc_input(
        u16::from_le_bytes([bytes[4], bytes[5]]),
        u64::from_le_bytes(bytes[6..14].try_into().unwrap()),
        body_len,
        &bytes[SLOT_HEADER..SLOT_HEADER + body_len],
    )
}

/// The body serializer: `next_op_id`, the in-progress operation, the
/// tombstone and carrier counts, then their fixed-size entries.
impl Journal {
    /// Store-level bound check for tests: whether the journal fits the
    /// capacity at encode.
    pub fn encode_for_tests(&self, capacity: usize) -> Result<Vec<u8>, String> {
        encode_journal(self, capacity)
    }
}

fn encode_journal(journal: &Journal, capacity: usize) -> Result<Vec<u8>, String> {
    if journal.carriers.len() > capacity {
        return Err(format!("carrier index exceeds {capacity}"));
    }
    if journal.tombstones.len() > MAX_TOMBSTONES {
        return Err(format!("tombstones exceed {MAX_TOMBSTONES}"));
    }
    let mut body =
        Vec::with_capacity(8 + 30 + 8 + (journal.tombstones.len() + journal.carriers.len()) * 20);
    body.extend_from_slice(&journal.next_op_id.to_le_bytes());
    match &journal.in_progress {
        None => body.push(0),
        Some(op) => {
            body.push(1);
            body.extend_from_slice(&op.op_id.to_le_bytes());
            body.push(op.kind.as_byte());
            body.extend_from_slice(&op.uuid.to_le_bytes());
            body.extend_from_slice(&op.generation.to_le_bytes());
            body.extend_from_slice(&(op.dimension.len() as u16).to_le_bytes());
            body.extend_from_slice(op.dimension.as_bytes());
        }
    }
    body.extend_from_slice(&(journal.tombstones.len() as u32).to_le_bytes());
    for &(uuid, generation) in &journal.tombstones {
        body.extend_from_slice(&uuid.to_le_bytes());
        body.extend_from_slice(&generation.to_le_bytes());
    }
    body.extend_from_slice(&(journal.carriers.len() as u32).to_le_bytes());
    for &(uuid, generation) in &journal.carriers {
        body.extend_from_slice(&uuid.to_le_bytes());
        body.extend_from_slice(&generation.to_le_bytes());
    }
    Ok(body)
}

/// The body parser: fixed 20-byte entries, counts bounded and checked
/// against the bytes present before the vectors grow, and every byte of
/// the body must be consumed.
fn decode_journal(body: &[u8]) -> Option<Journal> {
    // next_op_id, the in-progress flag, then, per section, a u32 count and
    // 20-byte entries.
    let mut at = 0_usize;
    let take = |at: &mut usize, n: usize| -> Option<&[u8]> {
        let head = body.get(*at..*at + n)?;
        *at += n;
        Some(head)
    };
    let next_op_id = u64::from_le_bytes(take(&mut at, 8)?.try_into().unwrap());
    let in_progress = match take(&mut at, 1)?[0] {
        0 => None,
        1 => {
            let op_id = u64::from_le_bytes(take(&mut at, 8)?.try_into().unwrap());
            let kind = OpKind::from_byte(take(&mut at, 1)?[0])?;
            let uuid = u128::from_le_bytes(take(&mut at, 16)?.try_into().unwrap());
            let generation = u32::from_le_bytes(take(&mut at, 4)?.try_into().unwrap());
            let dimension_len = u16::from_le_bytes(take(&mut at, 2)?.try_into().unwrap()) as usize;
            // Bounded before allocation: the length fits what remains.
            if body.len() < at + dimension_len {
                return None;
            }
            let dimension = String::from_utf8(take(&mut at, dimension_len)?.to_vec()).ok()?;
            Some(InProgress {
                op_id,
                kind,
                uuid,
                generation,
                dimension,
            })
        }
        _ => return None,
    };
    let section = |at: &mut usize, cap: usize| -> Option<Vec<(u128, u32)>> {
        let count = u32::from_le_bytes(take(at, 4)?.try_into().unwrap()) as usize;
        // Bounded before allocation, with every entry's bytes present;
        // the caller's exact-consumption check catches any slack.
        if count > cap || body.len() < *at + count * 20 {
            return None;
        }
        let entries = (0..count)
            .map(|_| {
                let uuid = u128::from_le_bytes(take(at, 16)?.try_into().unwrap());
                let generation = u32::from_le_bytes(take(at, 4)?.try_into().unwrap());
                Some((uuid, generation))
            })
            .collect::<Option<Vec<_>>>()?;
        Some(entries)
    };
    let tombstones = section(&mut at, MAX_TOMBSTONES)?;
    let carriers = section(&mut at, MAX_CARRIERS)?;
    if at != body.len() {
        return None;
    }
    Some(Journal {
        next_op_id,
        in_progress,
        tombstones,
        carriers,
    })
}

/// The tombstones whose carrier files are not in `data/` right now.
/// They are the ones a later cleanup may clear: their deletions predate
/// this open.
fn tombstones_without_files(data: &Path, journal: &Journal) -> Result<Vec<(u128, u32)>, Refusal> {
    if journal.tombstones.is_empty() {
        return Ok(Vec::new());
    }
    let present = fs::read_dir(data)
        .map_err(|e| Refusal::Io(e.to_string()))?
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter_map(|name| parse_carrier_name(&name))
        .collect::<Vec<_>>();
    Ok(journal
        .tombstones
        .iter()
        .filter(|tombstone| !present.contains(tombstone))
        .copied()
        .collect())
}

/// Whether `data/` holds any carrier file (operation or carrier), which
/// without a journal is a missing store whatever the flag says.
fn residual_files(data: &Path) -> Result<bool, Refusal> {
    let entries = match fs::read_dir(data) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(Refusal::Io(error.to_string())),
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if parse_operation_name(name).is_some() || parse_carrier_name(name).is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `sunrise_carrier_op_<id>.nbt` → the id.
fn parse_operation_name(name: &str) -> Option<u64> {
    name.strip_prefix("sunrise_carrier_op_")?
        .strip_suffix(".nbt")?
        .parse()
        .ok()
}

/// `sunrise_carrier_<uuid>-<gen>.nbt` → the carrier.
fn parse_carrier_name(name: &str) -> Option<(u128, u32)> {
    let stem = name
        .strip_prefix("sunrise_carrier_")?
        .strip_suffix(".nbt")?;
    let (uuid, generation) = stem.split_once('-')?;
    if uuid.len() != 32 || !uuid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some((
        u128::from_str_radix(uuid, 16).ok()?,
        generation.parse().ok()?,
    ))
}
