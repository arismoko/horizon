//! Registered engine payloads must retain their own structural or byte bounds.
use sunrise_core::vanilla_bytes::Writer;
use sunrise_protocol::{
    self as protocol,
    buf::{Codec, Reader},
};

#[test]
fn every_registered_engine_codec_refuses_a_hostile_oversized_body() {
    type Rejects = fn(&[u8]) -> bool;
    let cases: [(&str, Rejects); 15] = [
        ("minecraft:brand", |body| {
            let mut packet = Writer::new();
            packet.string("minecraft:brand");
            packet.bytes(body);
            protocol::packets::custom_payload::ServerboundPayload::read(&mut Reader::new(
                &packet.data,
            ))
            .is_err()
        }),
        ("sunrise:quiet_commands", |body| {
            protocol::quiet::decode(body).is_err()
        }),
        ("sunrise:mod_list", |body| {
            protocol::mod_list::decode(body).is_err()
        }),
        ("sunrise:attachment_sync", |body| {
            protocol::attachment_sync::decode(body).is_err()
        }),
        ("sunrise:connection_groups", |body| {
            protocol::connection_groups::Section::decode(body).is_err()
        }),
        ("sunrise:menu_extra", |body| {
            protocol::menu_extra::decode(body).is_err()
        }),
        ("sunrise:anvil_cost", |body| {
            protocol::anvil_cost::decode(body).is_err()
        }),
        ("sunrise:passage_sync", |body| {
            protocol::passage_sync::decode(body).is_err()
        }),
        ("sunrise:passage_cross", |body| {
            protocol::passage_sync::decode_cross(body).is_err()
        }),
        ("sunrise:carry_age", |body| {
            protocol::carry_age::decode(body).is_err()
        }),
        ("sunrise:lod", |body| {
            crate::lod_net::Offer::decode(body).is_none()
                && crate::lod_net::Request::decode(body).is_err()
                && crate::lod_net::decode_reply(body, &Default::default(), 1).is_err()
        }),
        ("sunrise:block_registry", |body| {
            crate::content_identity::decode_payload(body).is_err()
        }),
        ("sunrise:recipe_content", |body| {
            super::recipes::ContentPart::decode(body).is_err()
        }),
        ("sunrise:recipe_transfer", |body| {
            super::recipes::decode_transfer_request(body).is_err()
                && super::recipes::decode_transfer_reply(body).is_err()
        }),
        ("sunrise:recipe_book_settings", |body| {
            crate::recipe_book::settings_codec::decode(body).is_none()
        }),
    ];
    // Start with zeros: valid empty/fixed fields followed by excessive trailing
    // bytes, not just an impossible prefix which all parsers trivially reject.
    let mut body = vec![0; 2_097_152];
    for (channel, rejects) in cases {
        assert!(rejects(&body), "{channel} accepted an oversized body");
    }
    // A real LOD offer with trailing bytes must also fail. Its decoder used
    // to accept its prefix and ignore the rest (without allocating it).
    let mut offer = crate::lod_net::Offer::new(16).encode();
    offer.append(&mut body);
    assert!(crate::lod_net::Offer::decode(&offer).is_none());
}
