#[test]
fn client_packet_view_exposes_the_type_and_custom_payload_only() {
    use sunrise_protocol::packets::{
        configuration::{CustomPayload, Ping},
        play::Clientbound,
    };
    let packet = Clientbound::Ping(Ping { id: 7 });
    let ping = super::client_packet_view(&packet);
    assert_eq!(ping.id.to_string(), "minecraft:ping");
    assert_eq!(ping.custom_payload, None);

    let packet = Clientbound::CustomPayload(CustomPayload {
        channel: "example:message".into(),
        data: vec![0, 127, 255],
    });
    let Clientbound::CustomPayload(decoded) = &packet else {
        unreachable!();
    };
    let payload = super::client_packet_view(&packet);
    assert_eq!(payload.id.to_string(), "minecraft:custom_payload");
    let payload = payload.custom_payload.unwrap();
    assert_eq!(payload.channel, "example:message");
    assert_eq!(payload.data, &[0, 127, 255]);
    assert!(std::ptr::eq(payload.channel, decoded.channel.as_str()));
    assert!(std::ptr::eq(payload.data, decoded.data.as_slice()));

    for id in [-1, i32::MAX] {
        let packet = Clientbound::Unknown(id, vec![9]);
        let unknown = super::client_packet_view(&packet);
        assert_eq!(unknown.id.to_string(), "sunrise:unknown");
        assert_eq!(unknown.custom_payload, None);
    }
}

#[test]
fn serverbound_packet_view_names_the_packets_the_game_sends() {
    use sunrise_protocol::packets::{
        configuration::CustomPayload,
        play::{Serverbound, SetCarriedItem},
    };
    let packet = Serverbound::SetCarriedItem(SetCarriedItem { slot: 3 });
    let slot = super::serverbound_packet_view(&packet);
    assert_eq!(slot.id.to_string(), "minecraft:set_carried_item");
    assert_eq!(slot.custom_payload, None);

    let packet = Serverbound::CustomPayload(CustomPayload {
        channel: "example:message".into(),
        data: vec![1, 2],
    });
    let Serverbound::CustomPayload(decoded) = &packet else {
        unreachable!();
    };
    let payload = super::serverbound_packet_view(&packet);
    assert_eq!(payload.id.to_string(), "minecraft:custom_payload");
    let payload = payload.custom_payload.unwrap();
    assert_eq!(
        (payload.channel, payload.data),
        ("example:message", &[1, 2][..])
    );
    assert!(std::ptr::eq(payload.channel, decoded.channel.as_str()));
    assert!(std::ptr::eq(payload.data, decoded.data.as_slice()));

    let packet = Serverbound::Unknown(-1, vec![9]);
    let unknown = super::serverbound_packet_view(&packet);
    assert_eq!(unknown.id.to_string(), "sunrise:unknown");
    assert_eq!(unknown.custom_payload, None);
}
