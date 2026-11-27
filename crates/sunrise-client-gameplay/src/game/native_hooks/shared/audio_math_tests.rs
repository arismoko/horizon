#[test]
fn hand_pose_rejects_invalid_components() {
    let identity = sunrise_gameplay_common::game::native_hooks::hand_pose_matrix([
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]);
    assert_eq!(identity, glam::Mat4::IDENTITY);
    assert_eq!(
        super::hand_pose_matrix([
            f32::NAN,
            17.0,
            f32::INFINITY,
            3601.0,
            -3601.0,
            f32::NAN,
            0.0
        ]),
        identity
    );
    let point = sunrise_gameplay_common::game::native_hooks::hand_pose_matrix([
        1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 2.0,
    ])
    .transform_point3(glam::Vec3::X);
    assert_eq!(point, glam::Vec3::new(3.0, 2.0, 3.0));
}

/// `native_play_sound` takes vanilla's and mods' sound ids, nothing else.
#[test]
fn sound_ids_are_identifiers() {
    for id in [
        "minecraft:block.note_block.harp",
        "mymod:machine/hum",
        "my_mod.x-1:a",
    ] {
        assert!(super::sound_id(id), "{id}");
    }
    for id in [
        "",
        "hum",
        ":hum",
        "mymod:",
        "MyMod:hum",
        "mymod:Hum",
        "my/mod:hum",
        "a:b c",
    ] {
        assert!(!super::sound_id(id), "{id}");
    }
}
