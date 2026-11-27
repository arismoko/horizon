use {
    sunrise_gameplay_common::game::remote::C, sunrise_gameplay_common::game::remote::Tag,
    sunrise_gameplay_common::game::remote::play,
};

pub(in crate::game) fn isolated(name: &str, body: impl FnOnce()) {
    sunrise_gameplay_common::game::in_own_process(name, body);
}

pub(in crate::game) fn deliver(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    packet: C,
) {
    let bytes = packet.encode();
    let mut reader = sunrise_protocol::buf::Reader::new(&bytes);
    game.handle_packet(C::decode(reader.varint().unwrap(), reader.rest()).unwrap())
        .unwrap();
}

pub(in crate::game) fn chest(game: &sunrise_client_gameplay::game::ClientGame<'_>) -> C {
    C::OpenScreen(play::OpenScreen {
        container: 12,
        menu: game
            .client
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:menu")
            .unwrap()
            .id("minecraft:generic_9x3")
            .unwrap(),
        title: Tag::String("Test menu".into()),
    })
}

pub(in crate::game) fn close(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) {
    deliver(
        game,
        C::ContainerClose(play::ContainerClose { container: 37 }),
    );
}
