//! One block entity's or entity's saved data upgraded on its own, for callers that hold it outside a chunk (a mod
//! reading a schematic or an old structure): the fixes the chunk loaders run on it, behind a check of the input.
use super::{CURRENT, Ty, key, schema, update};
use horizon_core::nbt::{Compound, JavaString, Tag};

/// The deepest nesting accepted: a list or compound at depth `MAX_DEPTH` (the root compound is at 0) is refused.
/// The depth cap of every NBT read the game makes (`nbt::Limits`, vanilla's `NbtAccounter` default of 512); the
/// fixes walk the tree recursively, so nothing deeper reaches them.
pub const MAX_DEPTH: usize = 512;

/// The largest value accepted, in the bytes it takes as NBT: the chunk loader's own cap on saved data
/// (`nbt::Limits::saved`, 1 GiB, with no node cap), so nothing a saved chunk could carry is refused here.
pub const MAX_BYTES: u64 = 1 << 30;

/// Why a value was not upgraded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpgradeError {
    /// The data version is below zero.
    NegativeVersion(i32),
    /// The data version is newer than the game's ([`CURRENT`]): the fixes only go forward.
    FutureVersion(i32),
    /// A list or compound nests [`MAX_DEPTH`] levels deep or more.
    TooDeep,
    /// The value takes more than [`MAX_BYTES`] as NBT.
    TooLarge,
}

impl std::fmt::Display for UpgradeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NegativeVersion(v) => write!(f, "data version {v} is negative"),
            Self::FutureVersion(v) => {
                write!(f, "data version {v} is newer than the game's ({CURRENT})")
            }
            Self::TooDeep => write!(f, "the data nests {MAX_DEPTH} levels deep or more"),
            Self::TooLarge => write!(f, "the data takes more than {MAX_BYTES} bytes as NBT"),
        }
    }
}

impl std::error::Error for UpgradeError {}

/// One block entity's NBT saved at `data_version`, upgraded as the chunk loader upgrades the block entities of a
/// chunk of that version: each block entity type's fixes, up to [`CURRENT`]. Data of the current version comes back
/// as it is, and so does a block entity whose `id` that version doesn't know, as a chunk keeps it. Fixes of the
/// whole chunk (the blocks around it, the removal of bed block entities) have no chunk to act on and don't run. A
/// tag the fixes can't read comes back as it was saved. Refused before any fix runs: a negative or future version,
/// and a value over [`MAX_DEPTH`] or [`MAX_BYTES`].
pub fn upgrade_block_entity(data_version: i32, tag: Compound) -> Result<Compound, UpgradeError> {
    let tag = checked(data_version, tag)?;
    let id = Tag::compound([(
        "id",
        tag.get(&JavaString::from("id"))
            .cloned()
            .unwrap_or(Tag::End),
    )]);
    // Before 1.13 the known types come from the fixers' tables too: a failure there keeps the data, as a fix's would.
    let known =
        std::panic::catch_unwind(|| schema::block_entity_known(&id, key(data_version as u32, 0)));
    if known.unwrap_or(false) {
        Ok(upgraded(Ty::BlockEntity, tag, data_version))
    } else {
        Ok(tag)
    }
}

/// One entity's NBT saved at `data_version`, upgraded as the entity chunk loader upgrades the entities of a chunk of
/// that version: each entity type's fixes, its passengers' included, up to [`CURRENT`]. Data of the current version
/// comes back as it is; a tag the fixes can't read comes back as it was saved. Refused as
/// [`upgrade_block_entity`] refuses.
pub fn upgrade_entity(data_version: i32, tag: Compound) -> Result<Compound, UpgradeError> {
    let tag = checked(data_version, tag)?;
    Ok(upgraded(Ty::EntityTree, tag, data_version))
}

/// `tag` when its version can be upgraded from and it is within the limits. A refused value is taken apart without
/// recursion, as a value too deep could not be dropped otherwise.
fn checked(data_version: i32, tag: Compound) -> Result<Compound, UpgradeError> {
    let refusal = if data_version < 0 {
        Some(UpgradeError::NegativeVersion(data_version))
    } else if data_version > CURRENT {
        Some(UpgradeError::FutureVersion(data_version))
    } else {
        within_limits(&tag).err()
    };
    match refusal {
        None => Ok(tag),
        Some(error) => {
            dismantle(tag);
            Err(error)
        }
    }
}

/// [`update`] of one value of type `root` up to [`CURRENT`]. A value the fixes fail on (a legacy fixer's panic on
/// data it can't read) comes back as it was saved; the input check has already kept anything too deep away.
fn upgraded(root: Ty, tag: Compound, data_version: i32) -> Compound {
    if data_version == CURRENT {
        return tag;
    }
    let mut value = Tag::Compound(tag.clone());
    let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        update(root, &mut value, data_version, CURRENT);
    }))
    .is_ok();
    match value {
        Tag::Compound(upgraded) if done => upgraded,
        _ => tag,
    }
}

/// Walks `tag` without recursion: the depth of every list and compound and the bytes the whole takes as an NBT file
/// (a root compound with an empty name), stopping at the first limit passed.
fn within_limits(tag: &Compound) -> Result<(), UpgradeError> {
    // The root's type byte and empty name, then its payload.
    let mut bytes: u64 = 3;
    let mut stack: Vec<(&Tag, usize)> = Vec::new();
    let mut add = |n: u64| -> Result<(), UpgradeError> {
        bytes += n;
        if bytes > MAX_BYTES {
            Err(UpgradeError::TooLarge)
        } else {
            Ok(())
        }
    };
    // The root compound, at depth 0: its entries and its end byte.
    add(1)?;
    for (name, value) in tag {
        add(3 + utf8_len(name))?;
        stack.push((value, 1));
    }
    while let Some((value, depth)) = stack.pop() {
        match value {
            Tag::End => {}
            Tag::Byte(_) => add(1)?,
            Tag::Short(_) => add(2)?,
            Tag::Int(_) | Tag::Float(_) => add(4)?,
            Tag::Long(_) | Tag::Double(_) => add(8)?,
            Tag::ByteArray(v) => add(4 + v.len() as u64)?,
            Tag::IntArray(v) => add(4 + 4 * v.len() as u64)?,
            Tag::LongArray(v) => add(4 + 8 * v.len() as u64)?,
            Tag::String(s) => add(2 + utf8_len(s))?,
            Tag::List(items) => {
                if depth >= MAX_DEPTH {
                    return Err(UpgradeError::TooDeep);
                }
                add(5)?;
                stack.extend(items.iter().map(|item| (item, depth + 1)));
            }
            Tag::Compound(entries) => {
                if depth >= MAX_DEPTH {
                    return Err(UpgradeError::TooDeep);
                }
                add(1)?;
                for (name, item) in entries {
                    add(3 + utf8_len(name))?;
                    stack.push((item, depth + 1));
                }
            }
        }
    }
    Ok(())
}

/// The length of `s` in Java's modified UTF-8, as NBT writes a string.
fn utf8_len(s: &JavaString) -> u64 {
    s.0.iter()
        .map(|&unit| match unit {
            0x0001..=0x007F => 1,
            0 | 0x0080..=0x07FF => 2,
            _ => 3,
        })
        .sum()
}

/// Drops `tag` without recursion: every nested list and compound is emptied into a work list first.
fn dismantle(tag: Compound) {
    let mut pending: Vec<Tag> = tag.into_values().collect();
    while let Some(value) = pending.pop() {
        match value {
            Tag::List(items) => pending.extend(items),
            Tag::Compound(entries) => pending.extend(entries.into_values()),
            _ => {}
        }
    }
}
