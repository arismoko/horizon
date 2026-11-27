//! Private settings codec. The registered play packet uses the connection's framing.
use super::{
    BookFlags,
    placement_sync::{RESOURCE_ID_BYTES, ResourceId},
    settings_sync::Message,
};
use std::collections::BTreeSet;
use sunrise_core::vanilla_bytes::Writer;
use sunrise_protocol::{Error, Result, buf::Reader, custom_book::VERSION};

pub const PLACEMENT_TAG: u8 = 8;

pub fn encode(message: &Message) -> Result<Vec<u8>> {
    fn key(out: &mut Writer, id: &ResourceId) -> Result<()> {
        if !id.fits_wire() {
            return Err(Error::new(format!(
                "book identifier has {} bytes; Identifier.STREAM_CODEC permits {RESOURCE_ID_BYTES}",
                id.as_str().len()
            )));
        }
        out.string(id.as_str());
        Ok(())
    }
    fn count(out: &mut Writer, len: usize) -> Result<()> {
        out.varint(i32::try_from(len).map_err(|_| Error::new("book count cannot fit a VarInt"))?);
        Ok(())
    }
    let mut out = Writer::new();
    let pair = |out: &mut Writer, flags: BookFlags| {
        out.bool(flags.open);
        out.bool(flags.filtering);
    };
    match message {
        Message::Hello => out.bytes(&[5, VERSION]),
        Message::Ready => out.bytes(&[6, VERSION]),
        Message::Offer(keys) => {
            // A book that cannot be represented on the wire stays local; it must
            // not prevent the rest of the registry from negotiating.
            let keys: Vec<_> = keys
                .iter()
                .filter(|id| {
                    if id.fits_wire() {
                        return true;
                    }
                    let namespace = id.as_str().split(':').next().unwrap_or_default();
                    log::warn!(
                        "[recipe book] cannot offer a book from {namespace}: {}-byte identifier exceeds Identifier.STREAM_CODEC ({RESOURCE_ID_BYTES} bytes)",
                        id.as_str().len()
                    );
                    false
                })
                .collect();
            out.u8(0);
            count(&mut out, keys.len())?;
            for id in keys {
                out.string(id.as_str());
            }
        }
        Message::Snapshot {
            epoch,
            revision,
            entries,
        } => {
            out.u8(1);
            out.i64(*epoch as i64);
            out.i64(*revision as i64);
            count(&mut out, entries.len())?;
            for (id, flags) in entries {
                key(&mut out, id)?;
                pair(&mut out, *flags);
            }
        }
        Message::Request {
            epoch,
            number,
            key: id,
            flags,
        }
        | Message::Update {
            epoch,
            revision: number,
            key: id,
            flags,
        } => {
            out.u8(if matches!(message, Message::Request { .. }) {
                2
            } else {
                3
            });
            out.i64(*epoch as i64);
            out.i64(*number as i64);
            key(&mut out, id)?;
            pair(&mut out, *flags);
        }
        Message::Placement(frame) => {
            out.u8(PLACEMENT_TAG);
            out.data.extend(super::placement_codec::encode(frame)?);
        }
    }
    Ok(out.data)
}

pub fn decode(data: &[u8]) -> Option<Message> {
    fn key(r: &mut Reader<'_>) -> Option<ResourceId> {
        ResourceId::try_from(r.string(RESOURCE_ID_BYTES).ok()?).ok()
    }
    fn flags(r: &mut Reader<'_>) -> Option<BookFlags> {
        let open = r.u8().ok()?;
        let filtering = r.u8().ok()?;
        if open > 1 || filtering > 1 {
            return None;
        }
        Some(BookFlags {
            open: open == 1,
            filtering: filtering == 1,
        })
    }
    let mut r = Reader::new(data);
    // The private epochs and revisions are fixed-width unsigned bit patterns.
    let number = |r: &mut Reader<'_>| r.i64().ok().map(|bits| bits as u64);
    let tag = r.u8().ok()?;
    let result = match tag {
        0 | 1 => {
            let (epoch, revision) = if tag == 1 {
                (number(&mut r)?, number(&mut r)?)
            } else {
                (0, 0)
            };
            let n = usize::try_from(r.varint().ok()?).ok()?;
            // A length prefix and at least "a:b", plus two flags per snapshot entry.
            let min = if tag == 0 { 4 } else { 6 };
            if n > r.remaining() / min {
                return None;
            }
            let mut seen = BTreeSet::new();
            let mut entries = Vec::new();
            for _ in 0..n {
                let id = key(&mut r)?;
                if !seen.insert(id.clone()) {
                    return None;
                }
                let flags = if tag == 1 {
                    flags(&mut r)?
                } else {
                    BookFlags::default()
                };
                entries.push((id, flags));
            }
            if tag == 0 {
                Message::Offer(entries.into_iter().map(|(id, _)| id).collect())
            } else {
                Message::Snapshot {
                    epoch,
                    revision,
                    entries,
                }
            }
        }
        2 | 3 => {
            let epoch = number(&mut r)?;
            let number = number(&mut r)?;
            let key = key(&mut r)?;
            let flags = flags(&mut r)?;
            if tag == 2 {
                Message::Request {
                    epoch,
                    number,
                    key,
                    flags,
                }
            } else {
                Message::Update {
                    epoch,
                    revision: number,
                    key,
                    flags,
                }
            }
        }
        5 | 6 if r.u8().ok()? == VERSION => {
            if tag == 5 {
                Message::Hello
            } else {
                Message::Ready
            }
        }
        PLACEMENT_TAG => Message::Placement(super::placement_codec::decode(r.rest())?),
        _ => return None,
    };
    (r.remaining() == 0).then_some(result)
}
