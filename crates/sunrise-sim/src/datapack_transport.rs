//! Bounded custom-payload fragments; activate only after the complete snapshot validates.
use super::*;
use sunrise_protocol::packets::custom_payload::CLIENTBOUND_LIMIT;

const MAGIC: &[u8; 4] = b"DRS1";
const HEADER: usize = 12;
// Vanilla's ClientboundCustomPayloadPacket.MAX_PAYLOAD_SIZE includes our fragment header.
pub const CHUNK_DATA: usize = CLIENTBOUND_LIMIT - HEADER;

fn snapshot_too_large() -> String {
    format!(
        "Datapack registry snapshot exceeds the {}-byte transport limit.",
        mod_api::MAX_SNAPSHOT_BYTES
    )
}

pub fn chunks(bytes: &[u8]) -> Result<impl Iterator<Item = Vec<u8>> + '_, String> {
    if bytes.is_empty() {
        return Err("Cannot send an empty datapack registry snapshot.".into());
    }
    if bytes.len() > mod_api::MAX_SNAPSHOT_BYTES {
        return Err(snapshot_too_large());
    }
    let total = bytes.len() as u32;
    Ok(bytes.chunks(CHUNK_DATA).enumerate().map(move |(i, part)| {
        let mut packet = Vec::with_capacity(HEADER + part.len());
        packet.extend_from_slice(MAGIC);
        packet.extend_from_slice(&total.to_le_bytes());
        packet.extend_from_slice(&((i * CHUNK_DATA) as u32).to_le_bytes());
        packet.extend_from_slice(part);
        packet
    }))
}

#[derive(Debug, Default)]
struct Assembly {
    pending: Option<(usize, Vec<u8>)>,
}
impl Assembly {
    fn push(&mut self, packet: &[u8]) -> Result<Option<Vec<u8>>, String> {
        let previous = self.pending.take();
        let bad = || "Invalid or out-of-order datapack registry snapshot fragment.".to_string();
        let (header, part) = packet.split_at_checked(HEADER).ok_or_else(bad)?;
        if !header.starts_with(MAGIC) {
            return Err(bad());
        }
        let total = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
        let offset = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
        let mut bytes = if let Some((expected, bytes)) = previous {
            if total != expected || offset != bytes.len() {
                return Err(bad());
            }
            bytes
        } else {
            if offset != 0 {
                return Err(bad());
            }
            if total > mod_api::MAX_SNAPSHOT_BYTES {
                return Err(snapshot_too_large());
            }
            Vec::new()
        };
        if offset >= total || part.len() != CHUNK_DATA.min(total - offset) {
            return Err(bad());
        }
        bytes.extend_from_slice(part);
        if bytes.len() == total {
            Ok(Some(bytes))
        } else {
            self.pending = Some((total, bytes));
            Ok(None)
        }
    }
}

/// A single client configuration's synchronized registries and in-flight snapshot.
#[derive(Debug, Default)]
pub struct Receiver {
    assembly: Assembly,
    received: Option<Arc<mod_api::DatapackRegistrySet>>,
}

impl Receiver {
    pub fn received(&self) -> Option<&Arc<mod_api::DatapackRegistrySet>> {
        self.received.as_ref()
    }

    pub fn owned_heap_bytes(&self) -> usize {
        use sunrise_protocol::retained::OwnedHeap;
        self.assembly.pending.owned_heap_bytes().saturating_add(
            self.received
                .as_deref()
                .map_or(0, mod_api::__private::queued_datapack_set_bytes),
        )
    }

    pub fn finish_transfer(&self) -> Result<(), String> {
        if self.assembly.pending.is_some() {
            Err("Incomplete datapack registry snapshot at the end of configuration.".into())
        } else {
            Ok(())
        }
    }

    pub fn accept_chunk(&mut self, packet: &[u8]) -> Result<(), String> {
        if let Some(received) = self.decode_chunk(packet)? {
            self.received = Some(Arc::new(received));
        }
        Ok(())
    }

    /// Accept one snapshot fragment, returning the validated set when a complete snapshot
    /// finishes decoding without mutating [`Self::received`].
    pub fn decode_chunk(
        &mut self,
        packet: &[u8],
    ) -> Result<Option<mod_api::DatapackRegistrySet>, String> {
        if let Some(bytes) = self.assembly.push(packet)? {
            let declarations = mod_api::__private::datapack_registry_declarations();
            let received = receive(declarations, &bytes)?;
            return Ok(Some(received));
        }
        Ok(None)
    }

    /// Replace the received set after the client binds vanilla value handles on the game thread.
    pub fn replace_received(&mut self, set: Arc<mod_api::DatapackRegistrySet>) {
        self.received = Some(set);
    }
}

#[cfg(test)]
#[path = "datapack_transport_tests.rs"]
mod tests;
