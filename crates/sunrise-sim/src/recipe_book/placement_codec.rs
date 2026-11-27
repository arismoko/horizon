//! Placement framing delegates values to the shared Wire codec. The registered
//! play packet's enclosing Connection owns its frame limit.
use super::placement_sync::{Frame, RESOURCE_ID_BYTES};
use mod_api::wire::{Reader, Writer};
use sunrise_protocol::{Error, Result};

fn identifiers_fit(frame: &Frame) -> bool {
    match frame {
        Frame::Catalog { menu, book, .. } => menu.fits_wire() && book.fits_wire(),
        Frame::Start { identity, .. } => identity.menu.fits_wire() && identity.book.fits_wire(),
        Frame::Place {
            catalog, display, ..
        } => catalog.menu.fits_wire() && catalog.book.fits_wire() && display.recipe.fits_wire(),
        Frame::Ghost {
            catalog, recipe, ..
        } => catalog.menu.fits_wire() && catalog.book.fits_wire() && recipe.fits_wire(),
        Frame::Hello { .. }
        | Frame::Ready { .. }
        | Frame::Part { .. }
        | Frame::Refused { .. }
        | Frame::Invalidate { .. } => true,
    }
}

pub(super) fn encode(frame: &Frame) -> Result<Vec<u8>> {
    if !identifiers_fit(frame) {
        return Err(Error::new(format!(
            "placement identifier exceeds the Identifier.STREAM_CODEC limit of {RESOURCE_ID_BYTES} bytes"
        )));
    }
    let mut out = Writer::new();
    out.put(frame);
    if let Some(error) = out.error() {
        return Err(Error::new(format!("book placement frame: {error}")));
    }
    Ok(out.into_bytes())
}

pub(super) fn decode(bytes: &[u8]) -> Option<Frame> {
    let frame = Reader::decode(bytes).ok()?;
    identifiers_fit(&frame).then_some(frame)
}
