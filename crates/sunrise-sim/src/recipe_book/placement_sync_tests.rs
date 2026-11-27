use super::*;
fn id(value: &str) -> ResourceId {
    value.to_owned().try_into().unwrap()
}
fn window(nonce: u64) -> Window {
    Window {
        window: 7,
        nonce,
        menu: id("demo:pot"),
        book: id("demo:book"),
    }
}
fn open(client: &mut Client) -> CatalogIdentity {
    let Frame::Catalog {
        epoch,
        number,
        window,
        nonce,
        menu,
        book,
    } = client.open(window(8)).unwrap()
    else {
        panic!("catalog request")
    };
    CatalogIdentity {
        epoch,
        number,
        window,
        nonce,
        instance: 3,
        generation: 5,
        menu,
        book,
    }
}
fn client() -> Client {
    let mut client = Client::default();
    client.hello(11);
    assert!(client.ready(11, schema()));
    client
}
fn layout() -> mod_api::RecipeBookLayout {
    mod_api::RecipeBookLayout::Furnace {
        input: 0,
        fuel: 1,
        result: 2,
    }
}
fn complete(client: &mut Client, identity: &CatalogIdentity) {
    client.start(identity.clone(), layout(), 4).unwrap();
    assert!(
        client
            .part(identity.epoch, identity.number, 0, vec![0; 4])
            .unwrap()
            .is_some()
    );
    assert!(client.catalog().is_none());
    assert!(client.accept_catalog(identity.clone()));
}
fn display() -> DisplayId {
    DisplayId {
        recipe: id("demo:soup"),
        source: 2,
    }
}
fn ghost_display() -> mod_api::RecipeBookDisplay {
    mod_api::RecipeBookDisplay::Furnace {
        ingredient: mod_api::RecipeSlotDisplay::Item("minecraft:iron_ore".into()),
        fuel: mod_api::RecipeSlotDisplay::AnyFuel,
        result: mod_api::RecipeSlotDisplay::Item("minecraft:iron_ingot".into()),
        station: mod_api::RecipeSlotDisplay::Item("minecraft:furnace".into()),
        duration: 200,
        experience: 0.7,
    }
}
fn place(client: &Client) -> (CatalogIdentity, DisplayId) {
    let Frame::Place {
        catalog, display, ..
    } = client.place(display(), false).unwrap()
    else {
        panic!("placement request")
    };
    (catalog, display)
}
#[test]
fn negotiation_and_repeated_hello_preserve_monotonic_requests() {
    let mut client = Client::default();
    client.hello(11);
    assert!(!client.ready(12, schema()));
    assert!(!client.ready(11, schema() ^ 1));
    assert!(client.open(window(8)).is_none());
    assert!(client.ready(11, schema()));
    let first = open(&mut client);
    client.hello(11);
    let second = open(&mut client);
    assert!(second.number > first.number);
    let mut authority = Authority::new(11);
    assert_eq!(authority.hello(12, schema()), Err(Refusal::Stale));
    assert_eq!(authority.hello(11, schema() ^ 1), Err(Refusal::Unsupported));
    authority.hello(11, schema()).unwrap();
    authority.publish(first.clone()).unwrap();
    authority.hello(11, schema()).unwrap();
    assert_eq!(authority.publish(first), Err(Refusal::Stale));
}
#[test]
fn catalogue_authority_requires_a_completed_current_response() {
    let mut client = client();
    let identity = open(&mut client);
    assert!(!client.accept_catalog(identity.clone()));
    let mut wrong = identity.clone();
    wrong.instance = 0;
    assert_eq!(client.start(wrong, layout(), 4), Err(Refusal::Stale));
    let mut wrong = identity.clone();
    wrong.nonce += 1;
    assert_eq!(client.start(wrong, layout(), 4), Err(Refusal::Stale));
    complete(&mut client, &identity);
    let current = open(&mut client);
    assert!(!client.accept_catalog(identity));
    complete(&mut client, &current);
    assert_eq!(client.catalog(), Some(&current));
}
#[test]
fn catalogue_larger_than_a_frame_needs_complete_ordered_fragments() {
    use super::super::{
        settings_codec,
        settings_sync::{CHANNEL, Message},
    };
    use sunrise_protocol::{
        frame,
        packets::{configuration::CustomPayload, play},
    };

    let mut client = client();
    let identity = open(&mut client);
    let data: Vec<_> = (0..=frame::MAX_FRAME)
        .scan(1_u32, |state, _| {
            *state ^= *state << 13;
            *state ^= *state >> 17;
            *state ^= *state << 5;
            Some(*state as u8)
        })
        .collect();
    let first = Message::Placement(Frame::Part {
        epoch: identity.epoch,
        number: identity.number,
        offset: 0,
        bytes: data[..CHUNK_BYTES].to_vec(),
    });
    let packet = play::Clientbound::CustomPayload(CustomPayload {
        channel: CHANNEL.into(),
        data: settings_codec::encode(&first).unwrap(),
    })
    .try_encode()
    .unwrap();
    assert_eq!(packet.len(), frame::MAX_GUARANTEED_PACKET_BODY);
    assert!(frame::encode(&packet, None).is_ok());
    assert!(frame::encode(&packet, Some(256)).is_ok());
    client
        .start(identity.clone(), layout(), data.len() as u32)
        .unwrap();
    assert_eq!(
        client.start(identity.clone(), layout(), data.len() as u32),
        Err(Refusal::Invalid)
    );
    assert!(matches!(
        client.part(identity.epoch, identity.number, 1, vec![0]),
        Err(Refusal::Invalid)
    ));
    assert!(matches!(
        client.part(identity.epoch, identity.number, 0, vec![]),
        Err(Refusal::Invalid)
    ));
    assert!(
        client
            .part(
                identity.epoch,
                identity.number,
                0,
                data[..CHUNK_BYTES].to_vec()
            )
            .unwrap()
            .is_none()
    );
    assert!(client.catalog().is_none());
    assert!(matches!(
        client.part(
            identity.epoch,
            identity.number - 1,
            CHUNK_BYTES as u32,
            vec![0; 5]
        ),
        Err(Refusal::Stale)
    ));
    assert!(matches!(
        client.part(
            identity.epoch,
            identity.number,
            CHUNK_BYTES as u32,
            vec![0; data.len() - CHUNK_BYTES + 1],
        ),
        Err(Refusal::Invalid)
    ));
    let result = client
        .part(
            identity.epoch,
            identity.number,
            CHUNK_BYTES as u32,
            data[CHUNK_BYTES..].to_vec(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(result.identity, identity);
    assert_eq!(result.bytes, data);
    assert!(client.catalog().is_none());
    assert!(client.accept_catalog(result.identity));
}
#[test]
fn only_the_current_catalogue_accepts_a_server_ghost() {
    let mut client = client();
    let identity = open(&mut client);
    assert!(!client.accepts_ghost(&identity));
    complete(&mut client, &identity);
    assert!(client.accepts_ghost(&identity));
    let (catalog, clicked) = place(&client);
    assert_eq!(catalog, identity);
    assert_eq!(clicked, display());
    // Multiple placements on the same catalogue do not require a server reply between them.
    assert!(client.place(display(), true).is_some());
    let mut wrong = identity.clone();
    wrong.generation += 1;
    assert!(!client.accepts_ghost(&wrong));
    let mut wrong = identity.clone();
    wrong.instance += 1;
    assert!(!client.accepts_ghost(&wrong));
    let mut wrong = identity.clone();
    wrong.nonce += 1;
    assert!(!client.accepts_ghost(&wrong));
    assert!(client.accepts_ghost(&identity));
}
#[test]
fn close_reload_and_reconnect_invalidate_late_ghosts() {
    let mut client = client();
    let identity = open(&mut client);
    complete(&mut client, &identity);
    assert!(client.accepts_ghost(&identity));
    client.close();
    assert!(!client.accepts_ghost(&identity));
    assert!(client.place(display(), false).is_none());
    let current = open(&mut client);
    complete(&mut client, &current);
    assert!(client.accepts_ghost(&current));
    assert!(client.invalidate(11));
    assert!(!client.accepts_ghost(&current));
    let current = open(&mut client);
    complete(&mut client, &current);
    assert!(client.accepts_ghost(&current));
    client.hello(12);
    assert!(!client.accepts_ghost(&current));
    assert!(client.catalog().is_none());
    assert!(!client.invalidate(11));
}
#[test]
fn server_authority_binds_epoch_instance_generation_and_catalogue() {
    let mut client = client();
    let identity = open(&mut client);
    complete(&mut client, &identity);
    let mut authority = Authority::new(11);
    authority.hello(11, schema()).unwrap();
    authority.publish(identity.clone()).unwrap();
    let (current, _) = place(&client);
    let mut stale = current.clone();
    stale.instance += 1;
    assert_eq!(authority.place(&stale), Err(Refusal::Stale));
    let mut stale = current.clone();
    stale.generation += 1;
    assert_eq!(authority.place(&stale), Err(Refusal::Stale));
    // Repeated placements against the same active catalogue succeed without per-click sequence state.
    authority.place(&current).unwrap();
    authority.place(&current).unwrap();
    authority.clear();
    assert_eq!(authority.place(&current), Err(Refusal::Stale));
    authority.hello(11, schema()).unwrap();
    assert_eq!(authority.publish(identity), Err(Refusal::Stale));
}
#[test]
fn placement_frames_share_the_registered_settings_channel_without_vanilla_ids() {
    let mut client = client();
    let identity = open(&mut client);
    complete(&mut client, &identity);
    let (catalog, mut display) = place(&client);
    let frame = Frame::Ghost {
        catalog: catalog.clone(),
        recipe: display.recipe.clone(),
        display: ghost_display(),
    };
    let message = super::super::settings_sync::Message::Placement(frame);
    let mut bytes = super::super::settings_codec::encode(&message).unwrap();
    assert_eq!(bytes[0], 8);
    assert_eq!(super::super::settings_codec::decode(&bytes), Some(message));
    bytes.push(0);
    assert!(super::super::settings_codec::decode(&bytes).is_none());

    display.recipe = id(&format!("demo:{}", "a".repeat(32_762)));
    let message = super::super::settings_sync::Message::Placement(Frame::Place {
        catalog: catalog.clone(),
        display: display.clone(),
        use_max: true,
    });
    let encoded = super::super::settings_codec::encode(&message).unwrap();
    assert_eq!(
        super::super::settings_codec::decode(&encoded),
        Some(message)
    );

    display.recipe = id(&format!("demo:{}", "a".repeat(32_763)));
    let frame = Frame::Place {
        catalog: catalog.clone(),
        display: display.clone(),
        use_max: true,
    };
    assert!(super::super::placement_codec::encode(&frame).is_err());
    let mut raw = mod_api::wire::Writer::new();
    raw.put(&frame);
    assert!(super::super::placement_codec::decode(raw.bytes()).is_none());

    let oversize_ghost = Frame::Ghost {
        catalog,
        recipe: display.recipe,
        display: ghost_display(),
    };
    assert!(super::super::placement_codec::encode(&oversize_ghost).is_err());
    let mut raw = mod_api::wire::Writer::new();
    raw.put(&oversize_ghost);
    assert!(super::super::placement_codec::decode(raw.bytes()).is_none());

    let slash_path = ResourceId::try_from("demo:kitchen/pot".to_owned()).unwrap();
    assert_eq!(slash_path.as_str(), "demo:kitchen/pot");
    for id in ["..:pot", "pot", ":pot", "demo:", "demo:bad path"] {
        assert!(ResourceId::try_from(id.to_owned()).is_err(), "{id:?}");
    }
}

#[test]
fn invalidation_also_revokes_a_completed_but_unpublished_catalogue() {
    let mut client = client();
    let identity = open(&mut client);
    client.start(identity.clone(), layout(), 4).unwrap();
    assert!(
        client
            .part(identity.epoch, identity.number, 0, vec![0; 4])
            .unwrap()
            .is_some()
    );
    assert!(client.invalidate(identity.epoch));
    assert!(!client.accept_catalog(identity));
}
#[test]
fn catalogue_refusals_cannot_cancel_a_newer_request_or_completed_view() {
    let mut client = client();
    let old = open(&mut client);
    let current = open(&mut client);
    assert!(!client.refused(old.epoch, old.number, old.window, old.nonce));
    assert!(!client.refused(
        current.epoch,
        current.number,
        current.window,
        current.nonce + 1
    ));
    complete(&mut client, &current);
    assert!(!client.refused(current.epoch, current.number, current.window, current.nonce));
    assert!(!client.refused(current.epoch, 0, 0, 0));
    assert_eq!(client.catalog(), Some(&current));
    let current = open(&mut client);
    assert!(client.refused(current.epoch, current.number, current.window, current.nonce));
    assert_eq!(client.start(current, layout(), 4), Err(Refusal::Stale));
}
