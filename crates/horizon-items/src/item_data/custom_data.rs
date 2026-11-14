//! The persistent `CustomData.CODEC`, shared by recipes and live stack boundaries.
use horizon_core::nbt::{JavaString, Tag, parse_snbt_utf16, snbt_whitespace};

/// `CustomData.COMPOUND_TAG_CODEC`: compounds or fully parsed flattened SNBT.
/// Compound payloads are opaque NBT, including unknown keys and Java UTF-16.
pub fn decode(value: Tag) -> Result<Tag, String> {
    match value {
        value @ Tag::Compound(_) => Ok(value),
        Tag::String(text) => flattened(&text),
        _ => Err("Not a compound tag or SNBT string".into()),
    }
}

/// Validate a borrowed payload without copying opaque compound trees.
pub fn validate(value: &Tag) -> Result<(), String> {
    match value {
        Tag::Compound(_) => Ok(()),
        Tag::String(text) => flattened(text).map(|_| ()),
        _ => Err("Not a compound tag or SNBT string".into()),
    }
}

fn flattened(text: &JavaString) -> Result<Tag, String> {
    let (value, used) = parse_snbt_utf16(&text.0).map_err(|error| error.to_string())?;
    if !text.0[used..]
        .iter()
        .copied()
        .all(|unit| snbt_whitespace(u32::from(unit)))
    {
        return Err("argument.nbt.trailing".into());
    }
    match value {
        value @ Tag::Compound(_) => Ok(value),
        _ => Err("Expected compound tag".into()),
    }
}
