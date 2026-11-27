//! Shared cauldron client selection. Prediction never invokes authoritative callbacks.
use sunrise_core::vanilla_bytes::Writer;
use sunrise_protocol::{
    buf::{Codec, Ident, Reader},
    packets::{configuration::CustomPayload, play::Clientbound},
};
use sunrise_sim::cauldron::interactions::Dispatcher;

pub use sunrise_protocol::packets::custom_payload::{
    CAULDRON_ENTRIES_CHANNEL as ENTRIES_CHANNEL, CAULDRON_USE_CHANNEL as USE_CHANNEL,
};

/// Publish the server's selection snapshot only to peers that can suspend
/// and resume these clicks. Vanilla and other peers keep their vanilla packets.
pub fn publish_registrations<'a>(
    table: &crate::game::cauldron_interactions::Table,
    guests: impl IntoIterator<Item = &'a sunrise_sim::lan::guest::Guest>,
) {
    let mut packet = None;
    for guest in guests {
        let transport = guest.sender.payload_transport();
        if sunrise_sim::raw_payloads::negotiated(transport, ENTRIES_CHANNEL)
            && sunrise_sim::raw_payloads::negotiated(transport, USE_CHANNEL)
        {
            guest.send(packet.get_or_insert_with(|| registration_packet(table)));
        }
    }
}

/// Only selection keys are synchronized; the server never exports or predicts
/// a callback. An empty snapshot also clears entries after a reload.
pub fn registration_packet(table: &crate::game::cauldron_interactions::Table) -> Clientbound {
    let mut body = Writer::new();
    table.for_each_mod_entry(|block, item, tag| {
        body.string(block);
        body.string(item);
        body.bool(tag);
    });
    Clientbound::CustomPayload(CustomPayload {
        channel: ENTRIES_CHANNEL.into(),
        data: body.data,
    })
}

pub fn decode_registrations(data: &[u8]) -> sunrise_protocol::Result<Dispatcher<()>> {
    let mut table = Dispatcher::default();
    let mut input = Reader::new(data);
    while input.remaining() != 0 {
        let block = Ident::read(&mut input)?;
        let item = Ident::read(&mut input)?;
        let _ = if input.bool()? {
            table.register_tag(block, item, ())
        } else {
            table.register(block, item, ())
        };
    }
    Ok(table)
}

/// The two independent decisions made by the server's UseItemOn turn.
#[derive(Default)]
pub struct UseOnOutcome {
    /// The client may run its local item-on-block phase when the block passed or was skipped.
    pub block_fell_through: bool,
    /// Only a final Pass lets the client continue to air use or the other hand.
    pub passed: bool,
}

pub fn use_result(receipt: i64, outcome: UseOnOutcome) -> Clientbound {
    let mut body = Writer::new();
    body.i64(receipt);
    body.bool(outcome.block_fell_through);
    body.bool(outcome.passed);
    Clientbound::CustomPayload(CustomPayload {
        channel: USE_CHANNEL.into(),
        data: body.data,
    })
}
