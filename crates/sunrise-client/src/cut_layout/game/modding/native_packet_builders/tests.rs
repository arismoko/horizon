use mod_api::prelude::NetworkClientHost;
use mod_api::{ContainerClickRequest, PacketBuildError, PlayPacketRequest};
use sunrise_core::nbt::{Compound, Tag};
use sunrise_protocol::{
    buf::Reader,
    connection::Pipe,
    ids::play::serverbound as sb,
    item::{Component, Hashed, NoNames, Stack},
    packets::play::{Attack, ContainerClick, Serverbound},
    registry::Builtin,
};
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
};

#[test]
#[ignore = "daily"]
fn client_host_hashes_plain_stacks_with_added_and_removed_components() {
    if run_fresh_cases(
        "cut_layout::game::client_side::native_packet_builders::tests::client_host_hashes_plain_stacks_with_added_and_removed_components",
        "SUNRISE_CONTAINER_BUILDER_CHILD",
        &["build"],
    ) {
        return;
    }
    with_client_game(|game| {
        let mut stack =
            mod_api::ItemStack::new(mod_api::__private::item_key("minecraft:stick").unwrap(), 3)
                .unwrap();
        stack
            .set_components(Compound::from([
                ("minecraft:damage".into(), Tag::Int(7)),
                ("!minecraft:lore".into(), Tag::Compound(Compound::new())),
            ]))
            .unwrap();
        let mut request = ContainerClickRequest {
            container_id: 2,
            state_id: 5,
            slot: 36,
            changed_slots: vec![(36, Some(stack.clone())), (37, None)],
            carried: Some(stack),
            ..Default::default()
        };
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "PacketBuilder").unwrap();
        game.client.remote.as_mut().unwrap().builtin = std::sync::Arc::new(Builtin::parse(r#"{"minecraft:item":{"entries":{"minecraft:stick":{"protocol_id":4}}},"minecraft:data_component_type":{"entries":{"minecraft:damage":{"protocol_id":3},"minecraft:lore":{"protocol_id":11}}}}"#).unwrap());
        let attack =
            NetworkClientHost::build_play_packet(game, &PlayPacketRequest::Attack { entity: 7 })
                .unwrap();
        assert_eq!(attack.id.to_string(), "minecraft:attack");
        assert_eq!(
            Serverbound::decode(sb::ATTACK, &attack.body).unwrap(),
            Serverbound::Attack(Attack { id: 7 })
        );
        let got = NetworkClientHost::container_click(game, &request).unwrap();
        let network = Stack {
            added: vec![(3, Component::Int(7))],
            removed: vec![11],
            ..Stack::new(4, 3)
        };
        let hashed = Hashed::create(&network, &NoNames).unwrap();
        let expected = Serverbound::ContainerClick(ContainerClick {
            container: 2,
            state: 5,
            slot: 36,
            button: 0,
            input: 0,
            changed: vec![(36, Some(hashed.clone())), (37, None)],
            carried: Some(hashed),
        })
        .encode();
        let mut reader = Reader::new(&expected);
        reader.varint().unwrap();
        assert_eq!(got.body, reader.rest());
        request.changed_slots = vec![(36, None); 129];
        assert_eq!(
            NetworkClientHost::container_click(game, &request),
            Err(PacketBuildError::TooManyChangedSlots {
                packet: "container_click".into(),
                field: "changed_slots".into()
            })
        );
    });
}
