use super::{tests::empty_item_data, *};
use crate::raw_payloads::{Side, Transport};
use sunrise_core::vanilla_bytes::Writer;
use sunrise_protocol::connection::Pipe;

fn play_reader(
    mut reader: ConnReader,
    writer: Arc<Mutex<Option<QueuedWriter>>>,
    send: ingress::Producer<Event>,
    data: Arc<ItemData>,
    transport: Arc<Transport>,
) -> JoinHandle<Result<Reason, Reason>> {
    std::thread::spawn(move || {
        receive_play(
            &mut reader,
            Arc::default(),
            &writer,
            &Arc::new(AtomicBool::new(false)),
            &Arc::new(Language::new()),
            &send,
            &Cookies::default(),
            &Arc::default(),
            &data,
            &Arc::new(Mutex::new(None)),
            &transport,
            &Arc::default(),
            &Arc::default(),
        )
    })
}

#[test]
fn malformed_metadata_aborts_the_open_bundle_and_refunds_its_prefix() {
    let (root, data) = empty_item_data("malformed-bundle");
    let (client, server) = Pipe::pair();
    let (reader, writer) = Connection::local(client).split();
    let writer = Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap())));
    let (send, events) = event_channel();
    let mut server = Connection::local(server);
    let delimiter = play::Clientbound::BundleDelimiter(Empty).encode();
    server.send(&delimiter).unwrap();
    server
        .send(
            &play::Clientbound::AddEntity(play::AddEntity {
                id: 37,
                uuid: 37,
                kind: 0,
                position: [0.5, 64.0, 0.5],
                velocity: [0.0; 3],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: 0,
            })
            .encode(),
        )
        .unwrap();
    let mut metadata = Writer::new();
    metadata.varint(sunrise_protocol::ids::play::clientbound::SET_ENTITY_DATA);
    metadata.varint(37); // No metadata entries or terminator.
    server.send(&metadata.data).unwrap();
    server.send(&delimiter).unwrap();
    // EOF also lets a receiver that incorrectly skips the decode failure finish the test.
    drop(server);
    let error = play_reader(
        reader,
        writer.clone(),
        send.clone(),
        data,
        Transport::empty(),
    )
    .join()
    .unwrap()
    .unwrap_err();
    assert_eq!(
        error.key.as_deref(),
        Some("disconnect.genericReason"),
        "{error:?}"
    );
    assert!(
        events.try_recv_owned().is_err(),
        "neither the spawn nor a standalone DecodeError may escape"
    );
    assert_eq!(send.stats().reserved_bytes, 0);
    assert_eq!(send.stats().peak_reserved_bytes, 0);
    writer.lock().unwrap().take().unwrap().shutdown();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn malformed_attachment_aborts_packet_decoding_with_or_without_a_bundle() {
    use sunrise_protocol::{attachment_sync, general_entity};

    for bundled in [false, true] {
        let (root, data) = empty_item_data("malformed-pairing-attachment");
        let transport =
            Transport::capture(Side::Client, crate::net::PAYLOAD_RECORD_OVERHEAD).unwrap();
        transport
            .accept(transport.local.manifest.entries())
            .unwrap();
        transport.finish_configuration().unwrap();
        let (client, server) = Pipe::pair();
        let (reader, writer) = Connection::local(client).split();
        let writer = Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap())));
        let (send, events) = event_channel();
        let mut server = Connection::local(server);
        let delimiter = play::Clientbound::BundleDelimiter(Empty);
        let spawn = play::Clientbound::CustomPayload(cfg::CustomPayload {
            channel: general_entity::CHANNEL.into(),
            data: general_entity::encode(&general_entity::Message::Spawn {
                id: 37,
                uuid: 37,
                type_key: "test:general".into(),
                position: [0.5, 64.0, 0.5],
                rotation: [0.0; 2],
                velocity: [0.0; 3],
            })
            .unwrap(),
        });
        let broken = play::Clientbound::CustomPayload(cfg::CustomPayload {
            channel: attachment_sync::CHANNEL.into(),
            data: vec![5, 0, 0], // Invalid holder kind, but a valid outer CustomPayload.
        });
        if bundled {
            server.send(&delimiter.encode()).unwrap();
            server.send(&spawn.encode()).unwrap();
        }
        server.send(&broken.encode()).unwrap();
        if bundled {
            server.send(&delimiter.encode()).unwrap();
        }
        drop(server);
        let reader = play_reader(reader, writer.clone(), send, data, transport);
        assert!(
            matches!(
                events.recv_timeout_owned(Duration::from_secs(5)),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
            ),
            "the reader must close without publishing the spawn or its attachments"
        );
        let error = reader.join().unwrap().unwrap_err();
        assert_eq!(
            error.text,
            "attachment sync in play: attachment sync: holder kind 5"
        );
        assert_eq!(events.stats().reserved_bytes, 0);
        assert_eq!(events.stats().peak_reserved_bytes, 0);
        writer.lock().unwrap().take().unwrap().shutdown();
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn a_pairing_bundle_accepts_a_wire_valid_high_expansion_attachment() {
    use sunrise_core::nbt::Tag;
    use sunrise_mod_runtime::attachments::Holder;
    use sunrise_protocol::attachment_sync;

    let (root, data) = empty_item_data("expanded-pairing-attachment");
    let transport = Transport::capture(Side::Client, crate::net::PAYLOAD_RECORD_OVERHEAD).unwrap();
    transport
        .accept(transport.local.manifest.entries())
        .unwrap();
    transport.finish_configuration().unwrap();
    let (client, server) = Pipe::pair();
    let (reader, writer) = Connection::local(client).split();
    let writer = Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap())));
    let (send, events) = event_channel();
    let mut server = Connection::local(server);
    let delimiter = play::Clientbound::BundleDelimiter(Empty).encode();
    let spawn = play::Clientbound::AddEntity(play::AddEntity {
        id: 37,
        uuid: 37,
        kind: 0,
        position: [0.5, 64.0, 0.5],
        velocity: [0.0; 3],
        pitch: 0,
        yaw: 0,
        head_yaw: 0,
        data: 0,
    });
    // Entity 37, replace, one change a:b. Its nameless compound has a list of
    // 200,000 byte compounds (each {v: -7}), about 1.2 MB on the wire.
    const COUNT: u32 = 200_000;
    let mut body = vec![0, 37, 1, 1, 1, b'a', 1, b'b', 10, 9, 0, 1, b'l', 10];
    body.extend(COUNT.to_be_bytes());
    for _ in 0..COUNT {
        body.extend([1, 0, 1, b'v', 249, 0]);
    }
    body.push(0);
    let attachments = play::Clientbound::CustomPayload(cfg::CustomPayload {
        channel: attachment_sync::CHANNEL.into(),
        data: body,
    });
    let reader = play_reader(reader, writer.clone(), send, data, transport);
    for packet in [
        delimiter.clone(),
        spawn.encode(),
        attachments.encode(),
        delimiter,
    ] {
        server.send(&packet).unwrap();
    }
    drop(server);
    let timeout = Duration::from_secs(5);
    let Event::Bundle(bundle) = events.recv_timeout_owned(timeout).unwrap().into_parts().0 else {
        panic!("expected a completed pairing bundle");
    };
    let mut members = bundle.packets.into_iter();
    assert!(matches!(members.next(), Some(BundleMember::Packet(p)) if p == spawn));
    let Some(BundleMember::AttachmentSync(staged)) = members.next() else {
        panic!("expected a decoded attachment sync");
    };
    assert!(members.next().is_none());
    let (message, payload) = staged.into_message();
    assert_eq!(play::Clientbound::CustomPayload(payload), attachments);
    assert_eq!(message.holder, Holder::Entity(37));
    assert!(message.replace);
    let [change] = &message.changes[..] else {
        panic!("expected one attachment change");
    };
    assert_eq!(change.namespace, JavaString::from("a"));
    assert_eq!(change.name, JavaString::from("b"));
    let Some(Tag::List(items)) = change.value.as_ref().and_then(|map| map.get(&"l".into())) else {
        panic!("expected the staged attachment list");
    };
    let key = "v".into();
    assert_eq!(items.len(), COUNT as usize);
    assert!(
        items.iter().all(
            |item| matches!(item, Tag::Compound(map) if map.get(&key) == Some(&Tag::Byte(-7)))
        )
    );
    assert_eq!(events.stats().reserved_bytes, 0);
    let peak = events.stats().peak_reserved_bytes;
    assert!(peak > 1_200_000 && peak < 96 << 20, "charged {peak} bytes");
    reader.join().unwrap().unwrap_err();
    writer.lock().unwrap().take().unwrap().shutdown();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "daily"]
fn a_memory_only_snapshot_fits_ingress_despite_its_expanded_empty_compounds() {
    assert_memory_snapshot_fits(10, 0);
}

#[test]
#[ignore = "daily"]
fn a_memory_only_snapshot_fits_ingress_despite_millions_of_byte_nodes() {
    assert_memory_snapshot_fits(1, (-7i8) as u8);
}

fn assert_memory_snapshot_fits(element_opcode: u8, fill: u8) {
    use sunrise_core::nbt::Tag;
    use sunrise_mod_runtime::attachments::Holder;
    use sunrise_protocol::attachment_sync;

    let (root, data) = empty_item_data("memory-only-pairing-attachment");
    let transport = Transport::capture(Side::Client, crate::net::PAYLOAD_RECORD_OVERHEAD).unwrap();
    transport
        .accept(transport.local.manifest.entries())
        .unwrap();
    transport.finish_configuration().unwrap();
    let (client, server) = Pipe::pair();
    let (reader, writer) = Connection::local(client).split();
    let writer = Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap())));
    let (send, events) = event_channel();
    let mut server = Connection::local(server);

    // Each value is under the NBT node limit. The packet exceeds even a
    // compressed TCP frame's decoded limit. Retaining an eight-byte slot per
    // node in addition to the encoded packet would exceed 96 MiB.
    const FIELDS: u8 = 12;
    const COUNT: i32 = 980_000;
    let mut body = vec![0, 37, 1, FIELDS];
    for index in 0..FIELDS {
        body.extend([1, b'a', 1, b'a' + index, 10, 9, 0, 1, b'l', element_opcode]);
        body.extend(COUNT.to_be_bytes());
        body.resize(body.len() + COUNT as usize, fill);
        body.push(0);
    }
    let wire = body.len();
    assert!(wire > sunrise_protocol::frame::MAX_UNCOMPRESSED);
    assert!(wire + 8 * FIELDS as usize * COUNT as usize > 96 << 20);

    let attachments = play::Clientbound::CustomPayload(cfg::CustomPayload {
        channel: attachment_sync::CHANNEL.into(),
        data: body,
    });
    let old_charge = Event::Packet(Box::new(attachments.clone())).ingress_charge();
    let reader = play_reader(reader, writer.clone(), send, data, transport);
    server.send(&attachments.encode()).unwrap();
    drop(server);
    let (event, credit) = events
        .recv_timeout_owned(Duration::from_secs(15))
        .unwrap()
        .into_parts();
    assert!(event.ingress_charge() <= old_charge);
    let Event::AttachmentSync(staged) = event else {
        panic!("expected staged memory attachment sync");
    };
    assert!(events.stats().reserved_bytes >= wire);
    assert!(events.stats().reserved_bytes < wire * 2);
    let (message, payload) = staged.into_message();
    assert_eq!(play::Clientbound::CustomPayload(payload), attachments);
    assert_eq!(message.holder, Holder::Entity(37));
    assert!(message.replace);
    assert_eq!(message.changes.len(), FIELDS as usize);
    let key = JavaString::from("l");
    for (index, change) in (0..FIELDS).zip(message.changes) {
        assert_eq!(change.name, JavaString(vec![u16::from(b'a' + index)]));
        let Some(Tag::List(items)) = change.value.as_ref().and_then(|map| map.get(&key)) else {
            panic!("expected an attachment list");
        };
        assert_eq!(items.len(), COUNT as usize);
        assert!(items.iter().all(|tag| match tag {
            Tag::Compound(map) => element_opcode == 10 && map.is_empty(),
            Tag::Byte(n) => element_opcode == 1 && *n == -7,
            _ => false,
        }));
    }
    drop(credit);
    assert_eq!(events.stats().reserved_bytes, 0);
    reader.join().unwrap().unwrap_err();
    writer.lock().unwrap().take().unwrap().shutdown();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn malformed_play_packets_fail_decode_before_bundling() {
    let lang = Arc::new(
        [("disconnect.genericReason".to_owned(), "%s".to_owned())]
            .into_iter()
            .collect::<Language>(),
    );
    let mut delim_extra = Writer::new();
    delim_extra.varint(sunrise_protocol::ids::play::clientbound::BUNDLE_DELIMITER);
    delim_extra.u8(0x01);
    let mut trunc_post = Writer::new();
    trunc_post.varint(sunrise_protocol::ids::play::clientbound::POST_EFFECTS);
    trunc_post.varint(1);
    let mut unknown_id = Writer::new();
    unknown_id.varint(999);
    let cases = [
        (
            "bundle_delimiter_extra_bytes",
            delim_extra.data,
            "Internal Exception: io.netty.handler.codec.DecoderException: java.io.IOException: Packet play/clientbound/minecraft:bundle_delimiter (ClientboundBundleDelimiterPacket) was larger than I expected, found 1 bytes extra whilst reading packet clientbound/minecraft:bundle_delimiter",
        ),
        (
            "truncated_post_effects",
            trunc_post.data,
            "Internal Exception: io.netty.handler.codec.DecoderException: Failed to decode packet 'clientbound/minecraft:post_effects'",
        ),
        (
            "unregistered_packet_id",
            unknown_id.data,
            "Internal Exception: io.netty.handler.codec.DecoderException: Received unknown packet id 999",
        ),
    ];
    for (name, raw, expected_text) in cases {
        let (root, data) = empty_item_data(name);
        let (client, server) = Pipe::pair();
        let (mut reader, writer) = Connection::local(client).split();
        let writer = Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap())));
        let (send, events) = event_channel();
        let mut server = Connection::local(server);
        server.send(&raw).unwrap();
        drop(server);
        let lang = lang.clone();
        let writer_for_thread = writer.clone();
        let error = std::thread::spawn(move || {
            receive_play(
                &mut reader,
                Arc::default(),
                &writer_for_thread,
                &Arc::new(AtomicBool::new(false)),
                &lang,
                &send,
                &Cookies::default(),
                &Arc::default(),
                &data,
                &Arc::new(Mutex::new(None)),
                &Transport::empty(),
                &Arc::default(),
                &Arc::default(),
            )
        })
        .join()
        .unwrap()
        .unwrap_err();
        assert_eq!(
            error.key.as_deref(),
            Some("disconnect.genericReason"),
            "{name}"
        );
        assert_eq!(error.text, expected_text, "{name}");
        assert!(
            events.try_recv_owned().is_err(),
            "{name}: no event may escape"
        );
        writer.lock().unwrap().take().unwrap().shutdown();
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn bundled_large_simple_particles_stay_compact_in_memory() {
    use sunrise_protocol::entity_data::{Value, serializer};
    use sunrise_protocol::retained::OwnedHeap;

    let (root, data) = empty_item_data("compact-particles-bundle");
    let (client, server) = Pipe::pair();
    let (reader, writer) = Connection::local(client).split();
    let writer = Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap())));
    let (send, events) = event_channel();
    let mut server = Connection::local(server);
    let delimiter = play::Clientbound::BundleDelimiter(Empty).encode();

    const COUNT: usize = 65_537;
    let mut frame = Writer::new();
    frame.varint(sunrise_protocol::ids::play::clientbound::SET_ENTITY_DATA);
    frame.varint(37);
    frame.u8(10);
    frame.varint(serializer::PARTICLES);
    frame.varint(COUNT as i32);
    for _ in 0..COUNT {
        frame.varint(0);
    }
    frame.u8(0xFF);
    let wire_len = frame.data.len();

    let reader = play_reader(reader, writer.clone(), send, data, Transport::empty());
    server.send(&delimiter).unwrap();
    server.send(&frame.data).unwrap();
    server.send(&delimiter).unwrap();
    drop(server);
    let Event::Bundle(bundle) = events
        .recv_timeout_owned(Duration::from_secs(5))
        .unwrap()
        .into_parts()
        .0
    else {
        panic!("expected completed bundle");
    };
    let [BundleMember::Packet(play::Clientbound::SetEntityData(pkt))] = &bundle.packets[..] else {
        panic!("expected one SetEntityData member");
    };
    let Value::Particles(list) = &pkt.values[0].2 else {
        panic!("expected Particles value");
    };
    assert_eq!(list.len(), COUNT);
    assert_eq!(list.owned_heap_bytes(), COUNT);
    assert!(
        bundle.owned_bytes() < wire_len * 2,
        "bundle heap {} should stay near wire size {wire_len}",
        bundle.owned_bytes()
    );
    reader.join().unwrap().unwrap_err();
    writer.lock().unwrap().take().unwrap().shutdown();
    std::fs::remove_dir_all(root).unwrap();
}
