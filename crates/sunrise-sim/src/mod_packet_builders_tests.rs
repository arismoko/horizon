use super::*;
use sunrise_core::nbt::{Compound, Tag};
use sunrise_items::items::ItemStack as EngineStack;
use sunrise_protocol::{buf::Reader, item::Component};

fn builtin() -> Builtin {
    Builtin::parse(r#"{"minecraft:item":{"entries":{"minecraft:stick":{"protocol_id":4}}},"minecraft:data_component_type":{"entries":{"minecraft:damage":{"protocol_id":3},"minecraft:lore":{"protocol_id":11}}}}"#).unwrap()
}
fn fixture() -> ContainerClickRequest {
    mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
    mod_api::__private::resolve_items(["minecraft:stick"]).unwrap();
    let mut stack =
        mod_api::ItemStack::new(mod_api::__private::item_key("minecraft:stick").unwrap(), 3)
            .unwrap();
    stack
        .set_components(Compound::from([
            ("minecraft:damage".into(), Tag::Int(7)),
            ("!minecraft:lore".into(), Tag::Compound(Compound::new())),
        ]))
        .unwrap();
    ContainerClickRequest {
        container_id: 2,
        state_id: 5,
        slot: 36,
        changed_slots: vec![(36, Some(stack.clone())), (37, None)],
        carried: Some(stack),
        ..Default::default()
    }
}
fn engine(value: &mod_api::ItemStack) -> Option<EngineStack> {
    let patch = value
        .components()
        .map(|p| Tag::Compound(p.as_nbt().clone()));
    let (_, extra) = sunrise_items::item_data::decode_components(patch.as_ref());
    Some(EngineStack {
        item: "minecraft:stick".into(),
        count: value.count(),
        damage: value.damage(),
        extra,
    })
}

#[test]
fn plain_component_stacks_match_game_container_codec() {
    mod_platform::testing::with_mods(vec![].into(), || {
        let request = fixture();
        let got =
            encode_container_click(&request, &builtin(), &Dynamic::default(), engine).unwrap();
        let network = Stack {
            added: vec![(3, Component::Int(7))],
            removed: vec![11],
            ..Stack::new(4, 3)
        };
        let hashed = Hashed::create(&network, &sunrise_protocol::item::NoNames).unwrap();
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
        assert_eq!(got.id.to_string(), "minecraft:container_click");
        assert_eq!(got.body, reader.rest());
    });
}

#[test]
fn container_count_slots_and_unencodable_components_are_named() {
    mod_platform::testing::with_mods(vec![].into(), || {
        let mut request = fixture();
        request.changed_slots = vec![(36, None); 129];
        assert_eq!(
            encode_container_click(&request, &builtin(), &Dynamic::default(), |_| panic!(
                "limits must precede conversion"
            )),
            Err(PacketBuildError::TooManyChangedSlots {
                packet: "container_click".into(),
                field: "changed_slots".into()
            })
        );
        request.changed_slots = vec![(32768, None)];
        assert_eq!(
            encode_container_click(&request, &builtin(), &Dynamic::default(), engine),
            Err(range("changed_slots[0].slot"))
        );
        request.changed_slots.clear();
        request
            .carried
            .as_mut()
            .unwrap()
            .set_components(Compound::from([("test:unknown".into(), Tag::Int(1))]))
            .unwrap();
        assert_eq!(
            encode_container_click(&request, &builtin(), &Dynamic::default(), engine),
            Err(component_error("carried.components.test:unknown"))
        );
        let mut zero =
            mod_api::ItemStack::of(mod_api::__private::item_key("minecraft:stick").unwrap());
        zero.set_damage(0);
        request.carried = Some(zero);
        assert_eq!(
            encode_container_click(&request, &builtin(), &Dynamic::default(), engine),
            Err(component_error("carried.components.minecraft:damage"))
        );
        let unsupported = Stack {
            added: vec![(999, Component::Raw(vec![]))],
            ..Stack::new(4, 3)
        };
        assert!(checked_hash(&unsupported, &sunrise_protocol::item::NoNames, "carried").is_err());
    });
}

fn decode_built(request: &PlayPacketRequest, expected_name: &str, expected_id: i32) -> Serverbound {
    let packet = encode_play_packet(request).unwrap();
    assert_eq!(packet.id.to_string(), format!("minecraft:{expected_name}"));
    Serverbound::decode(expected_id, &packet.body).unwrap()
}

#[test]
fn every_typed_play_packet_decodes_to_independent_fields() {
    use sunrise_protocol::ids::play::serverbound as sb;

    match decode_built(
        &PlayPacketRequest::UseItem {
            hand: 1,
            sequence: 7,
            yaw: 90.0,
            pitch: -12.5,
        },
        "use_item",
        sb::USE_ITEM,
    ) {
        Serverbound::UseItem(p) => {
            assert_eq!(p.hand, 1);
            assert_eq!(p.sequence, 7);
            assert_eq!(p.yaw.to_bits(), 90.0_f32.to_bits());
            assert_eq!(p.pitch.to_bits(), (-12.5_f32).to_bits());
        }
        other => panic!("unexpected packet: {other:?}"),
    }
    match decode_built(
        &PlayPacketRequest::UseItem {
            hand: 0,
            sequence: 3,
            yaw: f32::INFINITY,
            pitch: f32::NAN,
        },
        "use_item",
        sb::USE_ITEM,
    ) {
        Serverbound::UseItem(p) => {
            assert_eq!(p.hand, 0);
            assert_eq!(p.sequence, 3);
            assert_eq!(p.yaw.to_bits(), f32::INFINITY.to_bits());
            assert!(p.pitch.is_nan());
        }
        other => panic!("unexpected packet: {other:?}"),
    }

    match decode_built(
        &PlayPacketRequest::UseItemOn {
            hand: 0,
            pos: [1, 64, -2],
            face: 1,
            cursor: [0.5, 1.0, 0.25],
            inside: true,
            world_border: false,
            sequence: 8,
        },
        "use_item_on",
        sb::USE_ITEM_ON,
    ) {
        Serverbound::UseItemOn(p) => {
            assert_eq!(p.hand, 0);
            assert_eq!(p.pos, [1, 64, -2]);
            assert_eq!(p.face, 1);
            assert_eq!(p.cursor, [0.5, 1.0, 0.25]);
            assert!(p.inside);
            assert!(!p.world_border);
            assert_eq!(p.sequence, 8);
        }
        other => panic!("unexpected packet: {other:?}"),
    }

    for action in 0..=8 {
        match decode_built(
            &PlayPacketRequest::PlayerAction {
                action,
                pos: [1, 64, -2],
                direction: 3,
                sequence: 7,
            },
            "player_action",
            sb::PLAYER_ACTION,
        ) {
            Serverbound::PlayerAction(p) => {
                assert_eq!(p.action, action);
                assert_eq!(p.pos, [1, 64, -2]);
                assert_eq!(p.direction, 3);
                assert_eq!(p.sequence, 7);
            }
            other => panic!("unexpected packet: {other:?}"),
        }
    }

    // Independent vanilla wire body bytes for Interact (VarInt id, VarInt hand, LpVec3, Bool).
    for (location, sneaking, expected_body, expected_decoded) in [
        (
            [f64::NAN, 0.0, 0.0],
            true,
            &[9_u8, 1, 0, 1][..],
            [0.0, 0.0, 0.0],
        ),
        (
            [0.0, 1.0, -1.0],
            false,
            &[9, 1, 249, 255, 0, 1, 255, 249, 0][..],
            [0.0, 1.0, -1.0],
        ),
        (
            [0.0, 64.0, -64.0],
            true,
            &[9, 1, 252, 255, 0, 1, 255, 249, 16, 1][..],
            [0.0, 64.0, -64.0],
        ),
        (
            [f64::INFINITY, f64::NEG_INFINITY, 0.0],
            false,
            &[9, 1, 247, 255, 127, 254, 0, 3, 255, 255, 255, 255, 15, 0][..],
            [1.717_986_918_3e10, -1.717_986_918_3e10, 0.0],
        ),
    ] {
        let packet = encode_play_packet(&PlayPacketRequest::Interact {
            entity: 9,
            hand: 1,
            location,
            sneaking,
        })
        .unwrap();
        assert_eq!(packet.id.to_string(), "minecraft:interact");
        assert_eq!(packet.body, expected_body);
        match Serverbound::decode(sb::INTERACT, &packet.body).unwrap() {
            Serverbound::Interact(p) => {
                assert_eq!(p.id, 9);
                assert_eq!(p.hand, 1);
                assert_eq!(p.location, expected_decoded);
                assert_eq!(p.secondary, sneaking);
            }
            other => panic!("unexpected packet: {other:?}"),
        }
    }

    match decode_built(
        &PlayPacketRequest::Attack { entity: 9 },
        "attack",
        sb::ATTACK,
    ) {
        Serverbound::Attack(p) => assert_eq!(p.id, 9),
        other => panic!("unexpected packet: {other:?}"),
    }

    for pos in [
        [1, 64, -2],
        [-33_554_432, -2048, -33_554_432],
        [33_554_431, 2047, 33_554_431],
    ] {
        match decode_built(
            &PlayPacketRequest::PickItemFromBlock {
                pos,
                include_data: true,
            },
            "pick_item_from_block",
            sb::PICK_ITEM_FROM_BLOCK,
        ) {
            Serverbound::PickItemFromBlock(p) => {
                assert_eq!(p.pos, pos);
                assert!(p.include_data);
            }
            other => panic!("unexpected packet: {other:?}"),
        }
    }

    match decode_built(
        &PlayPacketRequest::PickItemFromEntity {
            entity: 9,
            include_data: false,
        },
        "pick_item_from_entity",
        sb::PICK_ITEM_FROM_ENTITY,
    ) {
        Serverbound::PickItemFromEntity(p) => {
            assert_eq!(p.id, 9);
            assert!(!p.include_data);
        }
        other => panic!("unexpected packet: {other:?}"),
    }

    let sign_lines = [
        "Hello".to_string(),
        "世界".to_string(),
        "🦀".to_string(),
        String::new(),
    ];
    for front in [false, true] {
        match decode_built(
            &PlayPacketRequest::SignUpdate {
                pos: [1, 64, -2],
                front,
                lines: sign_lines.clone(),
            },
            "sign_update",
            sb::SIGN_UPDATE,
        ) {
            Serverbound::SignUpdate(p) => {
                assert_eq!(p.pos, [1, 64, -2]);
                assert_eq!(p.front, front);
                assert_eq!(p.lines, sign_lines);
            }
            other => panic!("unexpected packet: {other:?}"),
        }
    }
    let exact_utf16 = [
        "🦀".repeat(192),
        "a".repeat(384),
        String::new(),
        String::new(),
    ];
    match decode_built(
        &PlayPacketRequest::SignUpdate {
            pos: [1, 64, -2],
            front: true,
            lines: exact_utf16.clone(),
        },
        "sign_update",
        sb::SIGN_UPDATE,
    ) {
        Serverbound::SignUpdate(p) => {
            assert_eq!(p.pos, [1, 64, -2]);
            assert!(p.front);
            assert_eq!(p.lines, exact_utf16);
        }
        other => panic!("unexpected packet: {other:?}"),
    }

    match decode_built(
        &PlayPacketRequest::MovePlayerPos {
            pos: [1.5, 64.0, -2.25],
            on_ground: true,
            horizontal_collision: false,
        },
        "move_player_pos",
        sb::MOVE_PLAYER_POS,
    ) {
        Serverbound::MovePlayerPos(p) => {
            assert_eq!(p.x, 1.5);
            assert_eq!(p.y, 64.0);
            assert_eq!(p.z, -2.25);
            assert_eq!(p.flags, 1);
        }
        other => panic!("unexpected packet: {other:?}"),
    }
    match decode_built(
        &PlayPacketRequest::MovePlayerPos {
            pos: [f64::INFINITY, 64.0, f64::NEG_INFINITY],
            on_ground: false,
            horizontal_collision: true,
        },
        "move_player_pos",
        sb::MOVE_PLAYER_POS,
    ) {
        Serverbound::MovePlayerPos(p) => {
            assert_eq!(p.x.to_bits(), f64::INFINITY.to_bits());
            assert_eq!(p.y, 64.0);
            assert_eq!(p.z.to_bits(), f64::NEG_INFINITY.to_bits());
            assert_eq!(p.flags, 2);
        }
        other => panic!("unexpected packet: {other:?}"),
    }

    match decode_built(
        &PlayPacketRequest::MovePlayerPosRot {
            pos: [1.5, 64.0, -2.25],
            yaw: 90.0,
            pitch: -12.5,
            on_ground: false,
            horizontal_collision: true,
        },
        "move_player_pos_rot",
        sb::MOVE_PLAYER_POS_ROT,
    ) {
        Serverbound::MovePlayerPosRot(p) => {
            assert_eq!(p.x, 1.5);
            assert_eq!(p.y, 64.0);
            assert_eq!(p.z, -2.25);
            assert_eq!(p.yaw.to_bits(), 90.0_f32.to_bits());
            assert_eq!(p.pitch.to_bits(), (-12.5_f32).to_bits());
            assert_eq!(p.flags, 2);
        }
        other => panic!("unexpected packet: {other:?}"),
    }
    match decode_built(
        &PlayPacketRequest::MovePlayerPosRot {
            pos: [f64::INFINITY, 0.0, 0.0],
            yaw: 0.0,
            pitch: 0.0,
            on_ground: true,
            horizontal_collision: false,
        },
        "move_player_pos_rot",
        sb::MOVE_PLAYER_POS_ROT,
    ) {
        Serverbound::MovePlayerPosRot(p) => {
            assert_eq!(p.x.to_bits(), f64::INFINITY.to_bits());
            assert_eq!(p.flags, 1);
        }
        other => panic!("unexpected packet: {other:?}"),
    }

    match decode_built(
        &PlayPacketRequest::MovePlayerRot {
            yaw: 90.0,
            pitch: -12.5,
            on_ground: true,
            horizontal_collision: true,
        },
        "move_player_rot",
        sb::MOVE_PLAYER_ROT,
    ) {
        Serverbound::MovePlayerRot(p) => {
            assert_eq!(p.yaw.to_bits(), 90.0_f32.to_bits());
            assert_eq!(p.pitch.to_bits(), (-12.5_f32).to_bits());
            assert_eq!(p.flags, 3);
        }
        other => panic!("unexpected packet: {other:?}"),
    }

    for (on_ground, horizontal_collision, expected_flags) in [
        (false, false, 0),
        (true, false, 1),
        (false, true, 2),
        (true, true, 3),
    ] {
        match decode_built(
            &PlayPacketRequest::MovePlayerStatusOnly {
                on_ground,
                horizontal_collision,
            },
            "move_player_status_only",
            sb::MOVE_PLAYER_STATUS_ONLY,
        ) {
            Serverbound::MovePlayerStatus(p) => assert_eq!(p.flags, expected_flags),
            other => panic!("unexpected packet: {other:?}"),
        }
    }
}

#[test]
fn invalid_play_packet_fields_are_refused_by_packet_and_field() {
    assert_eq!(
        encode_play_packet(&PlayPacketRequest::MovePlayerPos {
            pos: [f64::NAN, 64.0, -2.25],
            on_ground: true,
            horizontal_collision: false,
        }),
        Err(PacketBuildError::NonFinite {
            packet: "move_player_pos".into(),
            field: "position".into(),
        })
    );
    assert_eq!(
        encode_play_packet(&PlayPacketRequest::MovePlayerPosRot {
            pos: [0.0, f64::NAN, 0.0],
            yaw: 0.0,
            pitch: 0.0,
            on_ground: false,
            horizontal_collision: false,
        }),
        Err(PacketBuildError::NonFinite {
            packet: "move_player_pos_rot".into(),
            field: "position".into(),
        })
    );
    assert_eq!(
        encode_play_packet(&PlayPacketRequest::MovePlayerPosRot {
            pos: [0.0; 3],
            yaw: f32::INFINITY,
            pitch: 0.0,
            on_ground: false,
            horizontal_collision: false,
        }),
        Err(PacketBuildError::NonFinite {
            packet: "move_player_pos_rot".into(),
            field: "yaw".into(),
        })
    );
    assert_eq!(
        encode_play_packet(&PlayPacketRequest::MovePlayerPosRot {
            pos: [0.0; 3],
            yaw: 0.0,
            pitch: f32::NAN,
            on_ground: false,
            horizontal_collision: false,
        }),
        Err(PacketBuildError::NonFinite {
            packet: "move_player_pos_rot".into(),
            field: "pitch".into(),
        })
    );
    assert_eq!(
        encode_play_packet(&PlayPacketRequest::MovePlayerRot {
            yaw: f32::NAN,
            pitch: 0.0,
            on_ground: false,
            horizontal_collision: false,
        }),
        Err(PacketBuildError::NonFinite {
            packet: "move_player_rot".into(),
            field: "yaw".into(),
        })
    );
    assert_eq!(
        encode_play_packet(&PlayPacketRequest::MovePlayerRot {
            yaw: 0.0,
            pitch: f32::NEG_INFINITY,
            on_ground: false,
            horizontal_collision: false,
        }),
        Err(PacketBuildError::NonFinite {
            packet: "move_player_rot".into(),
            field: "pitch".into(),
        })
    );
    assert_eq!(
        encode_play_packet(&PlayPacketRequest::SignUpdate {
            pos: [1, 64, -2],
            front: true,
            lines: ["a".repeat(385), String::new(), String::new(), String::new()],
        }),
        Err(PacketBuildError::SignLineTooLong {
            packet: "sign_update".into(),
            field: "lines[0]".into(),
        })
    );
    assert_eq!(
        encode_play_packet(&PlayPacketRequest::SignUpdate {
            pos: [1, 64, -2],
            front: true,
            lines: [
                String::new(),
                String::new(),
                "🦀".repeat(193),
                String::new(),
            ],
        }),
        Err(PacketBuildError::SignLineTooLong {
            packet: "sign_update".into(),
            field: "lines[2]".into(),
        })
    );
    assert_eq!(
        encode_play_packet(&PlayPacketRequest::PlayerAction {
            action: 9,
            pos: [1, 64, -2],
            direction: 0,
            sequence: 7,
        }),
        Err(PacketBuildError::UnknownAction {
            packet: "player_action".into(),
            field: "action".into(),
        })
    );
    for direction in [-1, 6] {
        assert_eq!(
            encode_play_packet(&PlayPacketRequest::PlayerAction {
                action: 0,
                pos: [1, 64, -2],
                direction,
                sequence: 7,
            }),
            Err(PacketBuildError::UnknownAction {
                packet: "player_action".into(),
                field: "direction".into(),
            })
        );
    }
    assert_eq!(
        encode_play_packet(&PlayPacketRequest::UseItem {
            hand: 2,
            sequence: 7,
            yaw: 0.0,
            pitch: 0.0,
        }),
        Err(PacketBuildError::UnknownAction {
            packet: "use_item".into(),
            field: "hand".into(),
        })
    );
    assert_eq!(
        encode_play_packet(&PlayPacketRequest::UseItemOn {
            hand: 0,
            pos: [1, 64, -2],
            face: 6,
            cursor: [0.0; 3],
            inside: false,
            world_border: false,
            sequence: 7,
        }),
        Err(PacketBuildError::UnknownAction {
            packet: "use_item_on".into(),
            field: "face".into(),
        })
    );
    for cursor in [[f32::NAN; 3], [f32::INFINITY, 0.0, 0.0]] {
        assert_eq!(
            encode_play_packet(&PlayPacketRequest::UseItemOn {
                hand: 0,
                pos: [1, 64, -2],
                face: 1,
                cursor,
                inside: false,
                world_border: false,
                sequence: 7,
            }),
            Err(PacketBuildError::NonFinite {
                packet: "use_item_on".into(),
                field: "cursor".into(),
            })
        );
    }
    assert_eq!(
        encode_play_packet(&PlayPacketRequest::Interact {
            entity: 9,
            hand: 2,
            location: [0.0; 3],
            sneaking: false,
        }),
        Err(PacketBuildError::UnknownAction {
            packet: "interact".into(),
            field: "hand".into(),
        })
    );
    for pos in [
        [33_554_432, 64, 0],
        [-33_554_433, 64, 0],
        [0, 2048, 0],
        [0, -2049, 0],
        [0, 0, 33_554_432],
    ] {
        assert_eq!(
            encode_play_packet(&PlayPacketRequest::PickItemFromBlock {
                pos,
                include_data: false,
            }),
            Err(PacketBuildError::OutOfRange {
                packet: "pick_item_from_block".into(),
                field: "position".into(),
            })
        );
    }
}
