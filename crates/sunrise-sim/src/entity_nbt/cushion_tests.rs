use super::*;

#[test]
fn flags_take_any_number_and_fall_back_when_absent() {
    let tag = Tag::compound([
        ("byte", Tag::Byte(2)),
        ("int", Tag::Int(1)),
        ("wrapped", Tag::Int(256)),
        ("short", Tag::Short(0)),
        ("text", string("1")),
    ]);
    assert!(flag(&tag, "byte"));
    assert!(flag(&tag, "int"));
    assert!(!flag(&tag, "wrapped"));
    assert!(!flag_or(&tag, "short", true));
    assert!(flag_or(&tag, "text", true));
    assert!(flag_or(&tag, "absent", true));
    assert!(!flag(&tag, "absent"));
}

#[test]
fn cushion_save_preserves_color_anchor_rotation_and_unknown_data() {
    let cushion = Cushion {
        attachment_lifetime: Default::default(),
        join: Default::default(),
        id: 7,
        position: Vec3::new(-1.5, 64.5, 3.5),
        pos: [-2, 64, 3],
        yaw: 270.0,
        color: 14,
        check_interval: 55,
        custom_name: None,
    };
    let raw = Tag::compound([("CustomName", string("Seat"))]);
    let tag = cushion_to_nbt(&cushion, [1, 2, 3, 4], Some(&raw));
    assert_eq!(text(tag.get("CustomName")).as_deref(), Some("Seat"));
    let loaded = cushion_from_nbt(&tag, 8).unwrap();
    assert_eq!(loaded.id, 8);
    assert_eq!(loaded.position, cushion.position);
    assert_eq!(loaded.pos, cushion.pos);
    assert_eq!(loaded.yaw, cushion.yaw);
    assert_eq!(loaded.color_name(), "red");
    assert_eq!(loaded.check_interval, 0);
    assert_eq!(loaded.bounds(), cushion.bounds());
}

#[test]
fn cushion_saves_the_motion_and_pitch_it_was_loaded_with() {
    let raw = Tag::compound([
        (
            "Motion",
            Tag::List(vec![
                Tag::Double(0.5_f64.to_bits()),
                Tag::Double(20.0_f64.to_bits()),
                Tag::Double((-0.25_f64).to_bits()),
            ]),
        ),
        (
            "Rotation",
            Tag::List(vec![
                Tag::Float(45.0_f32.to_bits()),
                Tag::Float(10.0_f32.to_bits()),
            ]),
        ),
    ]);
    let cushion = Cushion {
        attachment_lifetime: Default::default(),
        join: Default::default(),
        id: 7,
        position: Vec3::new(0.5, 64.0, 0.5),
        pos: [0, 64, 0],
        yaw: 45.0,
        color: 0,
        check_interval: 0,
        custom_name: None,
    };
    let saved = cushion_to_nbt(&cushion, [1, 2, 3, 4], Some(&raw));
    assert_eq!(motion(&saved), Vec3::new(0.5, 0.0, -0.25));
    assert_eq!(rotation(&saved), [45.0, 10.0]);
    // A cushion placed in play has no loaded data: zeros.
    let placed = cushion_to_nbt(&cushion, [1, 2, 3, 4], None);
    assert_eq!(motion(&placed), Vec3::ZERO);
    assert_eq!(rotation(&placed), [45.0, 0.0]);
}
