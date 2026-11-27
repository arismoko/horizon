//! Minecraft 26.3 display parity tests against VanillaDisplayParityOracle (.local witness).
use glam::{DVec3, Mat4};
use sunrise_client::{
    gfx::{
        Frustum, JoinedFrusta, OIT_ENTITY, TEXT_POLYGON_OFFSET_OVERLAY, TRANSLUCENT_ITEM_OVERLAY,
    },
    mesher::{SHADED_CULL_W, SectionMesh, Vertex},
};
use sunrise_client_gameplay::game::native_hooks::particle_count_tests::frame_scene;
use sunrise_core::nbt::{Compound, Tag};
use sunrise_items::items::ItemStack;
use sunrise_level::camera_math::CameraType;
use sunrise_protocol::entity_data::Value;
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
};
use {
    sunrise_client_gameplay::game::client_side::text_displays::RemoteTextDisplay,
    sunrise_client_gameplay::game::client_side::text_displays::Shows,
};

fn parity_witness() -> serde_json::Value {
    let path = std::env::var("SUNRISE_DISPLAY_PARITY_WITNESS").expect(
        "SUNRISE_DISPLAY_PARITY_WITNESS must be set (run via tools/test_displays_parity.py)",
    );
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"));
    let w: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(w["meta"]["game_version"], "26.3");
    w
}

fn hex_u32(v: &serde_json::Value) -> u32 {
    u32::from_str_radix(v.as_str().unwrap(), 16).unwrap()
}

fn hex_u64(v: &serde_json::Value) -> u64 {
    u64::from_str_radix(v.as_str().unwrap(), 16).unwrap()
}

fn hex_f32(v: &serde_json::Value) -> f32 {
    f32::from_bits(hex_u32(v))
}

fn hex_u32_vec(v: &serde_json::Value) -> Vec<u32> {
    v.as_array().unwrap().iter().map(hex_u32).collect()
}

fn hex_f32_arr<const N: usize>(v: &serde_json::Value) -> [f32; N] {
    let arr = v.as_array().unwrap();
    std::array::from_fn(|i| hex_f32(&arr[i]))
}

fn hex_f64(v: &serde_json::Value) -> f64 {
    f64::from_bits(hex_u64(v))
}

fn hex_f64_arr<const N: usize>(v: &serde_json::Value) -> [f64; N] {
    let arr = v.as_array().unwrap();
    std::array::from_fn(|i| hex_f64(&arr[i]))
}

fn decode_network_nbt_hex(v: &serde_json::Value) -> Tag {
    let hex = v.as_str().unwrap();
    assert_eq!(hex.len() % 2, 0);
    let bytes: Vec<u8> = hex
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect();
    let (tag, consumed) =
        sunrise_core::nbt::read_network(&bytes, sunrise_core::nbt::Limits::network()).unwrap();
    assert_eq!(consumed, bytes.len());
    tag
}

fn looking_south_frustum(fov_deg: f32, aspect: f32) -> JoinedFrusta {
    let proj = Mat4::perspective_rh(fov_deg.to_radians(), aspect, 256.0, 0.05);
    let view = sunrise_client::camera::view_matrix(0.0, 0.0);
    JoinedFrusta::new(Frustum::from_view_projection(proj * view), [])
}

fn collect_text_display_geometry(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    camera: DVec3,
    partial: f64,
    normal: &mut Vec<Vertex>,
    see_through: &mut Vec<Vertex>,
) {
    let mut collector = std::mem::take(&mut game.client.see_through_collector);
    collector.clear();
    game.text_display_geometry(camera, partial, normal, &mut collector);
    collector.flush(see_through);
    game.client.see_through_collector = collector;
}

#[test]
#[ignore = "run by tools/test_displays_parity.py"]
fn initial_start_min_value_and_duration_edge_cases_match_vanilla_26_3() {
    let w = parity_witness();
    let lc = &w["lifecycle"];

    // Initial interpolationStartClientTick = Integer.MIN_VALUE: duration=10 without id 8 snaps.
    let min_case = &lc["initial_start_min_value"];
    let mut d = RemoteTextDisplay::new(Shows::Text, [0.0; 3], 0.0);
    d.metadata(&[
        (23, 0, Value::Text(Tag::String("A".into()))),
        (11, 0, Value::Floats(vec![0.0, 0.0, 0.0])),
    ]);
    d.tick();
    d.metadata(&[
        (9, 1, Value::Int(10)),
        (11, 0, Value::Floats(vec![10.0, 0.0, 0.0])),
    ]);
    d.tick();
    let p0 = d.progress(0.0);
    let p_half = d.progress(0.5);
    assert_eq!(p0.to_bits(), hex_u32(&min_case["progress_at_0_bits"]));
    assert_eq!(
        p_half.to_bits(),
        hex_u32(&min_case["progress_at_half_bits"])
    );
    let blended = d.shown.blend(&d.from, p0);
    assert_eq!(
        blended.translation.map(f32::to_bits).to_vec(),
        hex_u32_vec(&min_case["translation_at_0_bits"])
    );

    // Negative duration snaps and updates render state cleanly.
    let dur_case = &lc["duration_zero_and_negative"];
    let mut dn = RemoteTextDisplay::new(Shows::Text, [0.0; 3], 0.0);
    dn.metadata(&[
        (23, 0, Value::Text(Tag::String("N".into()))),
        (11, 0, Value::Floats(vec![0.0, 0.0, 0.0])),
    ]);
    dn.tick();
    dn.metadata(&[
        (8, 1, Value::Int(0)),
        (9, 1, Value::Int(-3)),
        (11, 0, Value::Floats(vec![9.0, 0.0, 0.0])),
        (18, 2, Value::Float(2.0)),
    ]);
    dn.tick();
    let p_neg = dn.progress(0.5);
    assert_eq!(
        p_neg.to_bits(),
        hex_u32(&dur_case["neg_duration_progress_bits"])
    );
    let b_neg = dn.shown.blend(&dn.from, p_neg);
    assert_eq!(
        b_neg.translation.map(f32::to_bits).to_vec(),
        hex_u32_vec(&dur_case["neg_duration_translation_bits"])
    );
    assert_eq!(
        b_neg.shadow_radius.to_bits(),
        hex_u32(&dur_case["neg_duration_shadow_radius_bits"])
    );
}

#[test]
#[ignore = "run by tools/test_displays_parity.py"]
fn teleport_duration_unclamped_negative_same_target_and_rotation_match_vanilla_26_3() {
    let w = parity_witness();
    let tp = &w["teleport_duration_and_pos_rot_lerp"];

    // 1. Client synced teleport_duration = 100 is NOT clamped to 59.
    let c100 = &tp["client_unclamped_100"];
    let mut d = RemoteTextDisplay::new(Shows::Text, [0.0; 3], 0.0);
    d.metadata(&[(10, 1, Value::Int(100))]);
    d.move_to([100.0, 0.0, 0.0], Some((0.0, 0.0)));
    for tick in 1..=100 {
        d.tick();
        if matches!(tick, 1 | 2 | 50 | 59 | 100) {
            let key = format!("tick_{tick}_x_bits");
            assert_eq!(
                d.position.x.to_bits(),
                hex_u64(&c100[&key]),
                "unclamped teleport_duration=100 at tick {tick}"
            );
        }
    }

    // 2. NBT readAdditionalSaveData still clamps teleport_duration to 0..59.
    let mut tag = Compound::new();
    tag.insert("teleport_duration".into(), Tag::Int(100));
    let state =
        sunrise_gameplay::game::persist::display::read_text_display_nbt(&Tag::Compound(tag));
    let mut nbt_entries = Vec::new();
    state.append_metadata(None, false, |_| None, &mut nbt_entries);
    let nbt_tp = nbt_entries
        .iter()
        .find_map(|(id, _, v)| match (id, v) {
            (10, Value::Int(n)) => Some(*n),
            _ => None,
        })
        .unwrap();
    assert_eq!(nbt_tp, tp["nbt_clamped_100"].as_i64().unwrap() as i32);

    // 3. Negative teleport_duration does not snap and does not move.
    let neg = &tp["negative_teleport_duration"];
    let mut d_neg = RemoteTextDisplay::new(Shows::Text, [0.0; 3], 0.0);
    d_neg.metadata(&[(10, 1, Value::Int(-5))]);
    d_neg.move_to([10.0, 0.0, 0.0], Some((45.0, 15.0)));
    assert_eq!(
        d_neg.position.x.to_bits(),
        hex_u64(&neg["before_tick_x_bits"])
    );
    d_neg.tick();
    assert_eq!(
        d_neg.position.x.to_bits(),
        hex_u64(&neg["after_tick_x_bits"])
    );

    // 4. Repeating the identical target does not restart remaining steps.
    let same = &tp["same_target_no_restart"];
    let mut d_same = RemoteTextDisplay::new(Shows::Text, [0.0; 3], 0.0);
    d_same.metadata(&[(10, 1, Value::Int(4))]);
    d_same.move_to([4.0, 0.0, 0.0], Some((0.0, 0.0)));
    d_same.tick();
    d_same.tick();
    assert_eq!(
        d_same.position.x.to_bits(),
        hex_u64(&same["x_after_2_ticks_bits"])
    );
    d_same.move_to([4.0, 0.0, 0.0], Some((0.0, 0.0)));
    d_same.tick();
    assert_eq!(
        d_same.position.x.to_bits(),
        hex_u64(&same["x_after_repeat_and_tick3_bits"])
    );

    // 5. Yaw wraps modulo 360 (`setYRot`), pitch clamps to [-90, 90] (`setXRot`).
    let rot = &tp["rotation_lerp_and_clamp"];
    let mut d_rot = RemoteTextDisplay::new(Shows::Text, [0.0; 3], 350.0);
    d_rot.move_to([0.0; 3], Some((350.0, -80.0)));
    d_rot.metadata(&[(10, 1, Value::Int(2))]);
    d_rot.move_to([0.0; 3], Some((10.0, 80.0)));
    d_rot.tick();
    assert_eq!(
        d_rot.rotation[0].to_bits(),
        hex_u32(&rot["step1_yRot_bits"])
    );
    assert_eq!(
        d_rot.rotation[1].to_bits(),
        hex_u32(&rot["step1_xRot_bits"])
    );
    d_rot.metadata(&[(10, 1, Value::Int(0))]);
    d_rot.move_to([0.0; 3], Some((0.0, 120.0)));
    assert_eq!(
        d_rot.rotation[1].to_bits(),
        hex_u32(&rot["setXRot_120_bits"])
    );
    d_rot.move_to([0.0; 3], Some((0.0, -120.0)));
    assert_eq!(
        d_rot.rotation[1].to_bits(),
        hex_u32(&rot["setXRot_neg120_bits"])
    );
}

#[test]
#[ignore = "run by tools/test_displays_parity.py"]
fn display_shadow_respects_pre_tick_and_invisibility_flag_26_3() {
    let w = parity_witness();
    let sh = &w["display_entity_features"]["invisibility_shadow"];
    let mut d = RemoteTextDisplay::new(Shows::Text, [0.5, 0.0, 0.5], 0.0);
    d.metadata(&[
        (18, 2, Value::Float(2.0)),
        (19, 2, Value::Float(1.0)),
        (23, 0, Value::Text(Tag::String("S".into()))),
    ]);
    assert_eq!(
        d.shadow(1.0)
            .map_or(0.0f32.to_bits(), |(_, r, _)| r.to_bits()),
        hex_u32(&sh["pre_tick_shadow_radius_bits"])
    );
    d.tick();
    assert_eq!(
        d.shadow(1.0)
            .map_or(0.0f32.to_bits(), |(_, r, _)| r.to_bits()),
        hex_u32(&sh["visible_shadow_radius_bits"])
    );
    d.metadata(&[(0, 0, Value::Byte(0x20))]);
    assert_eq!(
        d.shadow(1.0)
            .map_or(0.0f32.to_bits(), |(_, r, _)| r.to_bits()),
        hex_u32(&sh["invisible_shadow_radius_bits"])
    );
}

#[test]
#[ignore = "run by tools/test_displays_parity.py"]
fn display_camera_angles_and_billboard_use_active_camera_not_player_26_3() {
    if run_fresh_cases(
        "cut_layout::game::client_side::text_displays::display_parity_tests::display_camera_angles_and_billboard_use_active_camera_not_player_26_3",
        "SUNRISE_DISPLAY_CAMERA_ANGLES_CASE",
        &["camera_angles"],
    ) {
        return;
    }
    let w = parity_witness();
    with_client_game(|game| {
        game.client.view_frustum = None;
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        // Player looks away while active render camera (spectated entity) uses witnessed angles.
        game.yaw = 123.0;
        game.pitch = 45.0;
        for case in w["billboard_camera_angles"]["cases"].as_array().unwrap() {
            game.client.camera_type = match case["camera_type"].as_str().unwrap() {
                "THIRD_PERSON_BACK" => CameraType::ThirdPersonBack,
                "THIRD_PERSON_FRONT" => CameraType::ThirdPersonFront,
                _ => CameraType::FirstPerson,
            };
            let cam_yaw = hex_f32(&case["camera_yRot_bits"]);
            let cam_pitch = hex_f32(&case["camera_xRot_bits"]);
            let ent_yaw = hex_f32(&case["entity_yRot_bits"]);
            let ent_pitch = hex_f32(&case["entity_xRot_bits"]);
            let bb = case["billboard"].as_i64().unwrap() as i8;
            game.client.posed_rotation = Some((cam_yaw, cam_pitch, 0.0));
            game.client.view.y_rot = cam_yaw;
            game.client.view.x_rot = cam_pitch;
            let got = game.display_camera_angles();
            assert_eq!(
                [got[0].to_bits(), got[1].to_bits()],
                [
                    hex_u32(&case["camera_yRot_bits"]),
                    hex_u32(&case["camera_xRot_bits"]),
                ],
                "camera angles for {} ({})",
                case["name"],
                case["camera_type"]
            );

            // Verify a block display renders with the witnessed active-camera pose (not player yaw/pitch).
            game.client.text_displays.0.clear();
            let pos = [0.5, 301.62, 0.5];
            assert!(game.spawn_remote_object(
                83083,
                "minecraft:block_display",
                pos,
                [ent_yaw, ent_pitch, 0.0],
                0
            ));
            let d = game.client.text_displays.0.get_mut(&83083).unwrap();
            d.show(&[(23, 0, Value::Id(stone.0 as i32))], None);
            d.metadata(&[
                (11, 0, Value::Floats(vec![-1.395, 0.3885, -4.2])),
                (12, 0, Value::Floats(vec![0.525, 0.525, 0.525])),
                (13, 0, Value::Floats(vec![0.0, 1.0, 0.0, 0.0])),
                (15, 0, Value::Byte(bb)),
            ]);
            game.tick_text_displays(game.clock.runs_normally());
            let mut out = Vec::new();
            game.block_item_display_geometry(DVec3::from_array(pos), 1.0, &mut out);
            assert!(!out.is_empty());

            // VertexConsumer.putBakedQuad transforms corners via JOML Matrix4f.transformPosition
            // (right-to-left unfused fma), not glam's left-to-right Mat4::transform_point3.
            let expected_pose = sunrise_client::camera::joml::Mat4::affine(Mat4::from_cols_array(
                &hex_f32_arr::<16>(&case["pose_bits"]),
            ));
            let expected_corners: Vec<[u32; 3]> = (0..8)
                .map(|bits| {
                    expected_pose
                        .transform_position([
                            (bits & 1) as f32,
                            ((bits >> 1) & 1) as f32,
                            ((bits >> 2) & 1) as f32,
                        ])
                        .map(f32::to_bits)
                })
                .collect();
            for v in &out {
                let bits = v.pos.map(f32::to_bits);
                assert!(
                    expected_corners.contains(&bits),
                    "{} bb={bb}: vertex {:?} ({bits:08x?}) does not match vanilla pose_bits corners",
                    case["name"],
                    v.pos
                );
            }
        }
    });
}

#[test]
#[ignore = "daily"]
fn no_culling_text_displays_are_not_frustum_culled_26_3() {
    if run_fresh_cases(
        "cut_layout::game::client_side::text_displays::display_parity_tests::no_culling_text_displays_are_not_frustum_culled_26_3",
        "SUNRISE_DISPLAY_NO_CULLING_CASE",
        &["no_culling"],
    ) {
        return;
    }
    with_client_game(|game| {
        let cam = DVec3::new(0.5, 301.620015, 0.5);
        game.client.camera_type = CameraType::FirstPerson;
        game.yaw = 0.0;
        game.pitch = 0.0;
        game.client.posed_rotation = Some((0.0, 0.0, 0.0));
        game.client.view.y_rot = 0.0;
        game.client.view.x_rot = 0.0;
        game.client.view_frustum = Some(looking_south_frustum(70.0, 1280.0 / 800.0));

        // width == 0 && height == 0 (noCulling): translation=(50,0,-50) behind camera still renders.
        assert!(game.spawn_remote_object(
            1001,
            "minecraft:text_display",
            [0.5, 301.62, 0.5],
            [0.0; 3],
            0,
        ));
        game.client
            .text_displays
            .0
            .get_mut(&1001)
            .unwrap()
            .metadata(&[
                (11, 0, Value::Floats(vec![50.0, 0.0, -50.0])),
                (17, 2, Value::Float(0.03)),
                (23, 0, Value::Text(Tag::String("Menu".into()))),
            ]);
        game.tick_text_displays(game.clock.runs_normally());
        let (mut normal, mut see_through) = (Vec::new(), Vec::new());
        collect_text_display_geometry(game, cam, 1.0, &mut normal, &mut see_through);
        assert!(!normal.is_empty());

        // Finite culling box behind the camera must be frustum-culled.
        game.client.text_displays.0.clear();
        assert!(game.spawn_remote_object(
            1002,
            "minecraft:text_display",
            [0.5, 301.62, -1.0],
            [0.0; 3],
            0,
        ));
        game.client
            .text_displays
            .0
            .get_mut(&1002)
            .unwrap()
            .metadata(&[
                (17, 2, Value::Float(0.03)),
                (20, 2, Value::Float(0.2)),
                (21, 2, Value::Float(0.2)),
                (23, 0, Value::Text(Tag::String("Behind".into()))),
            ]);
        game.tick_text_displays(game.clock.runs_normally());
        normal.clear();
        see_through.clear();
        collect_text_display_geometry(game, cam, 1.0, &mut normal, &mut see_through);
        assert!(normal.is_empty());

        // Infinite width alone must be frustum-culled (no +/-2 fallback) via JOML NaN rejection.
        game.client.text_displays.0.clear();
        assert!(game.spawn_remote_object(
            1003,
            "minecraft:text_display",
            [0.5, 301.62, 2.0],
            [0.0; 3],
            0,
        ));
        game.client
            .text_displays
            .0
            .get_mut(&1003)
            .unwrap()
            .metadata(&[
                (20, 2, Value::Float(f32::INFINITY)),
                (21, 2, Value::Float(1.0)),
                (23, 0, Value::Text(Tag::String("Inf".into()))),
            ]);
        game.tick_text_displays(game.clock.runs_normally());
        normal.clear();
        see_through.clear();
        collect_text_display_geometry(game, cam, 1.0, &mut normal, &mut see_through);
        assert!(normal.is_empty());
    });
}

#[test]
#[ignore = "run by tools/test_displays_parity.py"]
fn display_lifecycle_pre_tick_deferred_swap_live_culling_and_culled_progress_26_3() {
    if run_fresh_cases(
        "cut_layout::game::client_side::text_displays::display_parity_tests::display_lifecycle_pre_tick_deferred_swap_live_culling_and_culled_progress_26_3",
        "SUNRISE_DISPLAY_LIFECYCLE_CASE",
        &["lifecycle"],
    ) {
        return;
    }
    let w = parity_witness();
    let lc = &w["lifecycle"];
    let live = &w["live_culling_and_no_culling"];
    with_client_game(|game| {
        game.client.view_frustum = None;
        game.client.posed_rotation = Some((0.0, 0.0, 0.0));
        game.client.view.y_rot = 0.0;
        game.client.view.x_rot = 0.0;
        // Match VanillaDisplayParityOracle's `Entity.setViewScale(1.0)`.
        game.client.settings.render_distance = 8;
        game.client.settings.entity_distance = 1.0;
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let blue_terra = game
            .pack
            .assets
            .registry
            .default_state("minecraft:blue_terracotta")
            .unwrap();

        // 1. BlockDisplay: no rendering pre-tick or after empty tick; first metadata tick snaps; swap defers to tick.
        let bd_case = &lc["block_display_pre_tick_and_swap"];
        assert!(game.spawn_remote_object(2001, "minecraft:block_display", [0.0; 3], [0.0; 3], 0));
        let mut out = Vec::new();
        game.block_item_display_geometry(DVec3::ZERO, 1.0, &mut out);
        assert_eq!(
            !out.is_empty(),
            bd_case["on_spawn_render_state_present"].as_bool().unwrap()
        );
        game.tick_text_displays(game.clock.runs_normally()); // empty tick before metadata
        game.block_item_display_geometry(DVec3::ZERO, 1.0, &mut out);
        assert_eq!(
            !out.is_empty(),
            bd_case["after_empty_tick_render_state_present"]
                .as_bool()
                .unwrap()
        );
        {
            let d = game.client.text_displays.0.get_mut(&2001).unwrap();
            d.show(&[(23, 0, Value::Id(stone.0 as i32))], None);
            d.metadata(&[
                (8, 1, Value::Int(0)),
                (9, 1, Value::Int(4)),
                (11, 0, Value::Floats(vec![2.0, 3.0, 4.0])),
                (12, 0, Value::Floats(vec![2.0, 2.0, 2.0])),
            ]);
        }
        game.block_item_display_geometry(DVec3::ZERO, 1.0, &mut out);
        assert_eq!(
            !out.is_empty(),
            bd_case["after_metadata_before_tick_render_state_present"]
                .as_bool()
                .unwrap()
        );
        game.tick_text_displays(game.clock.runs_normally());
        let mut after_stone = Vec::new();
        game.block_item_display_geometry(DVec3::ZERO, 0.0, &mut after_stone);
        assert_eq!(
            !after_stone.is_empty(),
            bd_case["after_first_meta_tick_render_state_present"]
                .as_bool()
                .unwrap()
        );
        // Snapped to translation (2, 3, 4), NOT interpolated from (0, 0, 0).
        let min_x = after_stone
            .iter()
            .map(|v| v.pos[0])
            .fold(f32::INFINITY, f32::min);
        assert_eq!(
            min_x.to_bits(),
            hex_u32(&bd_case["tick2_translation_at_0_bits"][0])
        );
        // Swap block without ticking: must still render stone until next tick.
        {
            let d = game.client.text_displays.0.get_mut(&2001).unwrap();
            d.show(&[(23, 0, Value::Id(blue_terra.0 as i32))], None);
            d.metadata(&[(11, 0, Value::Floats(vec![6.0, 3.0, 4.0]))]);
        }
        let mut before_swap_tick = Vec::new();
        game.block_item_display_geometry(DVec3::ZERO, 0.0, &mut before_swap_tick);
        assert_eq!(
            bytemuck::cast_slice::<Vertex, u8>(&before_swap_tick),
            bytemuck::cast_slice::<Vertex, u8>(&after_stone),
            "block swap must wait for the next display tick"
        );
        game.tick_text_displays(game.clock.runs_normally());
        let mut after_swap_tick = Vec::new();
        game.block_item_display_geometry(DVec3::ZERO, 0.5, &mut after_swap_tick);
        assert_ne!(
            bytemuck::cast_slice::<Vertex, u8>(&after_swap_tick),
            bytemuck::cast_slice::<Vertex, u8>(&after_stone),
            "block swap must take effect on the tick"
        );

        // 2. ItemDisplay: pre-tick renders nothing; item & context swap defers to tick.
        game.client.text_displays.0.clear();
        let id_case = &lc["item_display_pre_tick_and_swap"];
        assert!(game.spawn_remote_object(2002, "minecraft:item_display", [0.0; 3], [0.0; 3], 0));
        game.client.text_displays.0.get_mut(&2002).unwrap().show(
            &[(24, 0, Value::Byte(6))],
            Some(Some(ItemStack::new("minecraft:end_crystal", 1))),
        );
        out.clear();
        game.block_item_display_geometry(DVec3::ZERO, 1.0, &mut out);
        assert_eq!(
            !out.is_empty(),
            id_case["before_tick_has_item_state"].as_bool().unwrap()
        );
        game.tick_text_displays(game.clock.runs_normally());
        let mut crystal_verts = Vec::new();
        game.block_item_display_geometry(DVec3::ZERO, 1.0, &mut crystal_verts);
        assert!(!crystal_verts.is_empty());
        game.client.text_displays.0.get_mut(&2002).unwrap().show(
            &[(24, 0, Value::Byte(8))],
            Some(Some(ItemStack::new("minecraft:diamond_sword", 1))),
        );
        let mut pre_swap = Vec::new();
        game.block_item_display_geometry(DVec3::ZERO, 1.0, &mut pre_swap);
        assert_eq!(
            bytemuck::cast_slice::<Vertex, u8>(&pre_swap),
            bytemuck::cast_slice::<Vertex, u8>(&crystal_verts),
            "item swap must wait for the next display tick"
        );

        // 3. Live view_range without tick + uninterpolated distance check.
        game.client.text_displays.0.clear();
        assert!(game.spawn_remote_object(
            2003,
            "minecraft:text_display",
            [0.5, 301.62, 0.5],
            [0.0; 3],
            0,
        ));
        game.client
            .text_displays
            .0
            .get_mut(&2003)
            .unwrap()
            .metadata(&[(23, 0, Value::Text(Tag::String("VR".into())))]);
        game.tick_text_displays(game.clock.runs_normally());
        // Standalone view_range update (no render state id, no tick): range = 1.92.
        game.client
            .text_displays
            .0
            .get_mut(&2003)
            .unwrap()
            .metadata(&[(17, 2, Value::Float(0.03))]);
        let (mut normal, mut see_through) = (Vec::new(), Vec::new());
        collect_text_display_geometry(
            game,
            DVec3::new(0.5 + 1.91, 301.62, 0.5),
            1.0,
            &mut normal,
            &mut see_through,
        );
        assert_eq!(
            !normal.is_empty(),
            live["live_vr_at_1_91"].as_bool().unwrap()
        );
        normal.clear();
        collect_text_display_geometry(
            game,
            DVec3::new(0.5 + 1.93, 301.62, 0.5),
            1.0,
            &mut normal,
            &mut see_through,
        );
        assert_eq!(
            !normal.is_empty(),
            live["live_vr_at_1_93"].as_bool().unwrap()
        );

        // Moving display: distance uses uninterpolated `position`, not `previous.lerp(position, 0.0)`.
        {
            let d = game.client.text_displays.0.get_mut(&2003).unwrap();
            d.previous = DVec3::new(0.5, 301.62, 0.5);
            d.position = DVec3::new(2.5, 301.62, 0.5);
        }
        normal.clear();
        collect_text_display_geometry(
            game,
            DVec3::new(0.5, 301.62, 0.5),
            0.0,
            &mut normal,
            &mut see_through,
        );
        assert_eq!(
            !normal.is_empty(),
            live["moving_uninterpolated_outside_range_partial_0"]
                .as_bool()
                .unwrap()
        );
        {
            let d = game.client.text_displays.0.get_mut(&2003).unwrap();
            d.previous = DVec3::new(2.5, 301.62, 0.5);
            d.position = DVec3::new(0.5, 301.62, 0.5);
        }
        normal.clear();
        collect_text_display_geometry(
            game,
            DVec3::new(0.5, 301.62, 0.5),
            0.0,
            &mut normal,
            &mut see_through,
        );
        assert_eq!(
            !normal.is_empty(),
            live["moving_uninterpolated_inside_range_partial_0"]
                .as_bool()
                .unwrap()
        );

        // 4. last_progress must NOT advance during frames where the display is culled.
        game.client.text_displays.0.clear();
        let culled_case = &lc["last_progress_while_culled"];
        assert!(game.spawn_remote_object(2004, "minecraft:text_display", [0.0; 3], [0.0; 3], 0));
        game.client
            .text_displays
            .0
            .get_mut(&2004)
            .unwrap()
            .metadata(&[
                (23, 0, Value::Text(Tag::String("C".into()))),
                (11, 0, Value::Floats(vec![0.0, 0.0, 0.0])),
                (17, 2, Value::Float(0.05)), // range = 3.2
            ]);
        game.tick_text_displays(game.clock.runs_normally()); // tick 1
        game.client
            .text_displays
            .0
            .get_mut(&2004)
            .unwrap()
            .metadata(&[
                (8, 1, Value::Int(0)),
                (9, 1, Value::Int(4)),
                (11, 0, Value::Floats(vec![10.0, 0.0, 0.0])),
            ]);
        game.tick_text_displays(game.clock.runs_normally()); // tick 2
        game.tick_text_displays(game.clock.runs_normally()); // tick 3: progress(0.0) = 0.25
        normal.clear();
        collect_text_display_geometry(game, DVec3::ZERO, 0.0, &mut normal, &mut see_through);
        assert!(!normal.is_empty());
        assert_eq!(
            game.client.text_displays.0[&2004].last_progress.to_bits(),
            hex_u32(&culled_case["visible_progress_bits"])
        );
        game.tick_text_displays(game.clock.runs_normally()); // tick 4: culled (camera 100 blocks away)
        normal.clear();
        collect_text_display_geometry(
            game,
            DVec3::new(100.0, 0.0, 0.0),
            0.0,
            &mut normal,
            &mut see_through,
        );
        assert!(normal.is_empty());
        // Tick 5: retarget to 20; must start from 0.25 * 10 = 2.5, not 7.5.
        game.client
            .text_displays
            .0
            .get_mut(&2004)
            .unwrap()
            .metadata(&[
                (8, 1, Value::Int(0)),
                (9, 1, Value::Int(4)),
                (11, 0, Value::Floats(vec![20.0, 0.0, 0.0])),
            ]);
        game.tick_text_displays(game.clock.runs_normally());
        let d = &game.client.text_displays.0[&2004];
        let p = d.progress(0.0);
        assert_eq!(
            p.to_bits(),
            hex_u32(&culled_case["retarget_progress_0_bits"])
        );
        assert_eq!(
            d.shown
                .blend(&d.from, p)
                .translation
                .map(f32::to_bits)
                .to_vec(),
            hex_u32_vec(&culled_case["retarget_start_translation_bits"])
        );

        // 5. Rendering during duration == 0 must return 1.0 without overwriting last_progress.
        game.client.text_displays.0.clear();
        let dur_case = &lc["duration_zero_and_negative"];
        assert!(game.spawn_remote_object(2005, "minecraft:text_display", [0.0; 3], [0.0; 3], 0));
        game.client
            .text_displays
            .0
            .get_mut(&2005)
            .unwrap()
            .metadata(&[
                (23, 0, Value::Text(Tag::String("Z".into()))),
                (11, 0, Value::Floats(vec![0.0, 0.0, 0.0])),
            ]);
        game.tick_text_displays(game.clock.runs_normally()); // tick 1
        game.client
            .text_displays
            .0
            .get_mut(&2005)
            .unwrap()
            .metadata(&[
                (8, 1, Value::Int(0)),
                (9, 1, Value::Int(4)),
                (11, 0, Value::Floats(vec![8.0, 0.0, 0.0])),
            ]);
        game.tick_text_displays(game.clock.runs_normally()); // tick 2
        game.tick_text_displays(game.clock.runs_normally()); // tick 3: progress(0.0) = 0.25
        normal.clear();
        collect_text_display_geometry(game, DVec3::ZERO, 0.0, &mut normal, &mut see_through);
        assert_eq!(
            game.client.text_displays.0[&2005].last_progress.to_bits(),
            hex_u32(&dur_case["progress_before_zero_bits"])
        );
        game.client
            .text_displays
            .0
            .get_mut(&2005)
            .unwrap()
            .metadata(&[(9, 1, Value::Int(0))]);
        game.tick_text_displays(game.clock.runs_normally()); // tick 4: duration = 0
        normal.clear();
        collect_text_display_geometry(game, DVec3::ZERO, 0.5, &mut normal, &mut see_through);
        game.client
            .text_displays
            .0
            .get_mut(&2005)
            .unwrap()
            .metadata(&[
                (8, 1, Value::Int(0)),
                (9, 1, Value::Int(4)),
                (11, 0, Value::Floats(vec![20.0, 0.0, 0.0])),
            ]);
        game.tick_text_displays(game.clock.runs_normally()); // tick 5: must start from 0.25 * 8 = 2.0, not 8.0
        let dz = &game.client.text_displays.0[&2005];
        let p_after = dz.progress(0.0);
        assert_eq!(
            p_after.to_bits(),
            hex_u32(&dur_case["progress_after_zero_bits"])
        );
        assert_eq!(
            dz.shown
                .blend(&dz.from, p_after)
                .translation
                .map(f32::to_bits)
                .to_vec(),
            hex_u32_vec(&dur_case["translation_after_zero_bits"])
        );
    });
}

#[test]
#[ignore = "run by tools/test_displays_parity.py"]
fn text_display_render_order_solid_split_and_see_through_sort_26_3() {
    if run_fresh_cases(
        "cut_layout::game::client_side::text_displays::display_parity_tests::text_display_render_order_solid_split_and_see_through_sort_26_3",
        "SUNRISE_DISPLAY_RENDER_ORDER_CASE",
        &["render_order"],
    ) {
        return;
    }
    let w = parity_witness();
    let phases = &w["text_submit_phases_and_sorting"];
    with_client_game(|game| {
        game.client.view_frustum = None;
        game.client.posed_rotation = Some((0.0, 0.0, 0.0));
        game.client.view.y_rot = 0.0;
        game.client.view.x_rot = 0.0;
        // Match VanillaDisplayParityOracle's `Entity.setViewScale(1.0)`.
        game.client.settings.render_distance = 8;
        game.client.settings.entity_distance = 1.0;

        // 1. Deterministic insertion order (including ID inversions and shift_remove), matching EntityLookup.byId.
        game.client.text_displays.0.clear();
        let initial_ids = [83226, 83191, 83242, 83102, 83249, 83232, 83100, 83250];
        let mut expected_bgs = Vec::new();
        for (idx, &eid) in initial_ids.iter().enumerate() {
            assert!(game.spawn_remote_object(eid, "minecraft:text_display", [0.0; 3], [0.0; 3], 0));
            let argb = 0xFF00_0001u32 + idx as u32;
            let abgr =
                0xFF00_0000u32 | ((argb & 0xFF) << 16) | (argb & 0xFF00) | ((argb >> 16) & 0xFF);
            expected_bgs.push((eid, abgr));
            game.client
                .text_displays
                .0
                .get_mut(&eid)
                .unwrap()
                .metadata(&[
                    (23, 0, Value::Text(Tag::String(" ".into()))),
                    (25, 1, Value::Int(argb as i32)),
                    (26, 0, Value::Byte(-1)),
                ]);
        }
        // Removing a middle display must preserve the remaining displays' insertion order.
        game.client.text_displays.0.shift_remove(&83242);
        expected_bgs.retain(|&(eid, _)| eid != 83242);
        game.tick_text_displays(game.clock.runs_normally());
        let (mut normal, mut see_through) = (Vec::new(), Vec::new());
        collect_text_display_geometry(game, DVec3::ZERO, 1.0, &mut normal, &mut see_through);
        let got_bgs: Vec<u32> = normal
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|q| q[0].layer == TEXT_POLYGON_OFFSET_OVERLAY)
            .map(|q| q[0].color)
            .collect();
        let want_bgs: Vec<u32> = expected_bgs.iter().map(|&(_, c)| c).collect();
        assert_eq!(
            got_bgs, want_bgs,
            "text displays must emit in insertion order matching vanilla EntityLookup.byId"
        );

        // 2. Solid vs translucent phase routing (`canRenderAsSolid` in `phase_routing`).
        let routing = phases["phase_routing"].as_array().unwrap();
        let find_route = |name: &str, improved: bool| {
            routing
                .iter()
                .find(|r| r["name"] == name && r["improved_transparency"] == improved)
                .unwrap()
        };
        let cases: [(&str, &str, u32, i8); 4] = [
            ("opaque_panel", "X", 0xFF12_0A24, -1),
            (
                "translucent_panel_with_opaque_text",
                "1000 ELO",
                0xA010_1722,
                -1,
            ),
            ("transparent_bg_opaque_label", "Kuyruk", 0x0000_0000, -1),
            ("translucent_text_and_bg", "Fade", 0x8000_0000, 128u8 as i8),
        ];
        for (name, text, bg, opacity) in cases {
            game.client.text_displays.0.clear();
            assert!(game.spawn_remote_object(
                3001,
                "minecraft:text_display",
                [0.0; 3],
                [0.0; 3],
                0
            ));
            game.client
                .text_displays
                .0
                .get_mut(&3001)
                .unwrap()
                .metadata(&[
                    (23, 0, Value::Text(Tag::String(text.into()))),
                    (25, 1, Value::Int(bg as i32)),
                    (26, 0, Value::Byte(opacity)),
                ]);
            game.tick_text_displays(game.clock.runs_normally());
            normal.clear();
            see_through.clear();
            collect_text_display_geometry(game, DVec3::ZERO, 1.0, &mut normal, &mut see_through);
            let quads = normal.as_chunks::<4>().0;
            assert!(!quads.is_empty(), "{name}: emitted no quads");
            let route = find_route(name, false);
            let text_is_solid = route["solid"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "text:POLYGON_OFFSET");
            let bg_is_translucent = route["texts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "background:POLYGON_OFFSET");
            let bg_abgr = (bg & 0xFF00_FF00) | ((bg & 0xFF) << 16) | ((bg >> 16) & 0xFF);
            let mut bg_quads = Vec::new();
            let mut glyph_quads = Vec::new();
            for q in quads {
                if bg != 0 && q[0].color == bg_abgr {
                    bg_quads.push(q[0].layer);
                } else {
                    glyph_quads.push(q[0].layer);
                }
            }
            if bg_is_translucent {
                assert!(!bg_quads.is_empty(), "{name}: missing background quad");
                assert!(
                    bg_quads.iter().all(|&l| l == TEXT_POLYGON_OFFSET_OVERLAY),
                    "{name}: background must stay in translucent/OIT polygon-offset overlay, got {bg_quads:?}"
                );
            } else {
                assert!(bg_quads.is_empty(), "{name}: unexpected background quad");
            }
            assert!(!glyph_quads.is_empty(), "{name}: missing glyph quads");
            if text_is_solid {
                assert!(
                    glyph_quads
                        .iter()
                        .all(|&l| l != TEXT_POLYGON_OFFSET_OVERLAY && (l & OIT_ENTITY) == 0),
                    "{name}: opaque text must route to a solid overlay distinct from TEXT_POLYGON_OFFSET_OVERLAY, got {glyph_quads:?}"
                );
            } else {
                assert!(
                    glyph_quads
                        .iter()
                        .all(|&l| l == TEXT_POLYGON_OFFSET_OVERLAY),
                    "{name}: translucent text must route to TEXT_POLYGON_OFFSET_OVERLAY, got {glyph_quads:?}"
                );
            }
        }

        // 3. See-through per-submit far-to-near sort (stable on ties).
        game.client.text_displays.0.clear();
        let st_specs: [(&str, f64, u32); 4] = [
            ("near_2", -2.0, 0xFF00_0001),
            ("tie_first_4", -4.0, 0xFF00_0002),
            ("far_6", -6.0, 0xFF00_0003),
            ("tie_second_4", -4.0, 0xFF00_0004),
        ];
        for (i, &(_, z, argb)) in st_specs.iter().enumerate() {
            let id = 4000 + i as i32;
            assert!(game.spawn_remote_object(
                id,
                "minecraft:text_display",
                [0.0, 0.0, z],
                [0.0; 3],
                0
            ));
            game.client
                .text_displays
                .0
                .get_mut(&id)
                .unwrap()
                .metadata(&[
                    (23, 0, Value::Text(Tag::String("T".into()))),
                    (25, 1, Value::Int(argb as i32)),
                    (26, 0, Value::Byte(-1)),
                    (27, 0, Value::Byte(2)), // see_through
                ]);
        }
        game.tick_text_displays(game.clock.runs_normally());
        normal.clear();
        see_through.clear();
        collect_text_display_geometry(game, DVec3::ZERO, 1.0, &mut normal, &mut see_through);
        assert!(normal.is_empty());
        let expected_names: Vec<&str> = phases["see_through_sorted_order"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let got_names: Vec<&str> = see_through
            .as_chunks::<4>()
            .0
            .iter()
            .filter_map(|q| {
                st_specs.iter().find_map(|&(name, _, argb)| {
                    let abgr = (argb & 0xFF00_FF00) | ((argb & 0xFF) << 16) | ((argb >> 16) & 0xFF);
                    (q[0].color == abgr).then_some(name)
                })
            })
            .collect();
        assert_eq!(got_names, expected_names);
        // Per-submit sorting keeps each display's background quad immediately before its glyph quad.
        let st_quads = see_through.as_chunks::<4>().0;
        assert_eq!(st_quads.len(), 8);
        for pair in st_quads.as_chunks::<2>().0 {
            assert_ne!(pair[0][0].color, 0xFFFF_FFFF, "background precedes glyph");
            assert_eq!(
                pair[1][0].color, 0xFFFF_FFFF,
                "glyph follows its background"
            );
        }
    });
}

#[test]
#[ignore = "run by tools/test_displays_parity.py"]
fn text_display_resolves_inline_player_head_profiles_26_3() {
    if run_fresh_cases(
        "cut_layout::game::client_side::text_displays::display_parity_tests::text_display_resolves_inline_player_head_profiles_26_3",
        "SUNRISE_DISPLAY_INLINE_HEAD_CASE",
        &["inline_head"],
    ) {
        return;
    }
    let w = parity_witness();
    let head = &w["inline_player_head"];
    let tag = decode_network_nbt_hex(&head["component_nbt_hex"]);
    let resolved = &head["resolved_profile"];
    let id_key = format!("id:{}", resolved["id_hex"].as_str().unwrap());
    let name_key = format!(
        "name:{}",
        resolved["name"].as_str().unwrap().to_ascii_lowercase()
    );
    let want_skin = resolved["default_skin"].as_str().unwrap();

    with_client_game(|game| {
        game.client.view_frustum = None;
        game.client.posed_rotation = Some((0.0, 0.0, 0.0));
        game.client.view.y_rot = 0.0;
        game.client.view.x_rot = 0.0;
        // Match VanillaDisplayParityOracle's `Entity.setViewScale(1.0)`.
        game.client.settings.render_distance = 8;
        game.client.settings.entity_distance = 1.0;
        assert!(!game.client.gui.atlas.object_skins.contains_key(&id_key));
        assert!(!game.client.gui.atlas.object_skins.contains_key(&name_key));

        assert!(game.spawn_remote_object(
            83225,
            "minecraft:text_display",
            [0.5, 301.62, 0.5],
            [0.0; 3],
            0,
        ));
        game.client
            .text_displays
            .0
            .get_mut(&83225)
            .unwrap()
            .metadata(&[(23, 0, Value::Text(tag))]);
        game.tick_text_displays(game.clock.runs_normally());
        let cam = DVec3::new(0.5, 301.62, 0.5);
        let (mut normal, mut see_through) = (Vec::new(), Vec::new());
        collect_text_display_geometry(game, cam, 1.0, &mut normal, &mut see_through);
        assert_eq!(
            game.client
                .gui
                .atlas
                .object_skins
                .get(&id_key)
                .map(String::as_str),
            Some(want_skin),
            "inline object:player profile must resolve to vanilla default skin (key: {id_key})"
        );
        assert_eq!(
            game.client
                .gui
                .atlas
                .object_skins
                .get(&name_key)
                .map(String::as_str),
            Some(want_skin),
            "inline object:player profile must resolve to vanilla default skin (key: {name_key})"
        );
    });
}

#[test]
#[ignore = "daily"]
fn block_display_backface_culling_translucent_pass_and_light_emission_26_3() {
    if run_fresh_cases(
        "cut_layout::game::client_side::text_displays::display_parity_tests::block_display_backface_culling_translucent_pass_and_light_emission_26_3",
        "SUNRISE_DISPLAY_BLOCK_LIGHT_CASE",
        &["block_light"],
    ) {
        return;
    }
    with_client_game(|game| {
        game.client.view_frustum = None;
        game.client.posed_rotation = Some((0.0, 0.0, 0.0));
        game.client.view.y_rot = 0.0;
        game.client.view.x_rot = 0.0;
        game.client.settings.render_distance = 8;
        game.client.settings.entity_distance = 1.0;
        let chunk = sunrise_level::chunk::Chunk::empty(0, 0);
        game.world.chunks.insert((0, 0), std::sync::Arc::new(chunk));

        let emit_block = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
                          id: i32,
                          block: &str|
         -> Vec<Vertex> {
            game.client.text_displays.0.clear();
            game.client.glowing_vertices.borrow_mut().clear();
            let state = game.pack.assets.registry.default_state(block).unwrap();
            assert!(game.spawn_remote_object(
                id,
                "minecraft:block_display",
                [0.5, 64.0, 0.5],
                [0.0; 3],
                0,
            ));
            game.client
                .text_displays
                .0
                .get_mut(&id)
                .unwrap()
                .show(&[(23, 0, Value::Id(state.0 as i32))], None);
            game.tick_text_displays(game.clock.runs_normally());
            let mut out = Vec::new();
            game.block_item_display_geometry(DVec3::new(0.5, 64.0, 3.5), 1.0, &mut out);
            assert!(!out.is_empty(), "{block} emitted no vertices");
            out
        };

        let concrete = emit_block(game, 5001, "minecraft:black_concrete");
        assert!(
            concrete.iter().all(|v| v.normal[3] == SHADED_CULL_W),
            "opaque block display must use back-face culling (SHADED_CULL_W)"
        );
        assert!(concrete.iter().all(|v| (v.layer & 0xffff) == 0));

        let glass = emit_block(game, 5002, "minecraft:blue_stained_glass");
        assert!(
            glass.iter().all(|v| v.normal[3] == SHADED_CULL_W),
            "translucent block display must use back-face culling (SHADED_CULL_W)"
        );
        assert!(
            glass
                .iter()
                .all(|v| (v.layer & 0xffff) == TRANSLUCENT_ITEM_OVERLAY),
            "translucent block display must route to TRANSLUCENT_ITEM_OVERLAY"
        );

        let glowstone = emit_block(game, 5003, "minecraft:glowstone");
        assert!(
            glowstone.iter().all(|v| ((v.light >> 8) & 0xff) == 15 * 16),
            "glowstone block display must emit block light 240, got {}",
            (glowstone[0].light >> 8) & 0xff
        );

        // Magma block emissiveRendering resolves to full light (15, 15).
        let magma = emit_block(game, 5004, "minecraft:magma_block");
        assert!(
            magma.iter().all(|v| v.light == 0xF0F0),
            "magma_block display must emit full (15, 15) light 0xF0F0, got {:#x}",
            magma[0].light
        );

        // Ender chest special geometry preserves block light 7 (112).
        let echest = emit_block(game, 5005, "minecraft:ender_chest");
        assert!(
            echest.iter().all(|v| ((v.light >> 8) & 0xff) == 7 * 16),
            "ender_chest display must emit block light 112, got {}",
            (echest[0].light >> 8) & 0xff
        );

        // Glowing block display submits live glow_color_override (red -> blue without ticking).
        game.client
            .text_displays
            .0
            .get_mut(&5005)
            .unwrap()
            .metadata(&[(0, 0, Value::Byte(0x40)), (22, 1, Value::Int(0xFF0000))]);
        game.client.glowing_vertices.borrow_mut().clear();
        let mut out = Vec::new();
        game.block_item_display_geometry(DVec3::new(0.5, 64.0, 3.5), 1.0, &mut out);
        assert!(!game.client.glowing_vertices.borrow().is_empty());
        assert_eq!(game.client.glowing_vertices.borrow()[0].color, 0xFF00_00FF);
        game.client
            .text_displays
            .0
            .get_mut(&5005)
            .unwrap()
            .metadata(&[(22, 1, Value::Int(0x0000FF))]);
        game.client.glowing_vertices.borrow_mut().clear();
        out.clear();
        game.block_item_display_geometry(DVec3::new(0.5, 64.0, 3.5), 1.0, &mut out);
        assert_eq!(game.client.glowing_vertices.borrow()[0].color, 0xFFFF_0000);
    });
}

#[test]
#[ignore = "daily"]
fn display_name_tags_and_fire_block_light_match_vanilla_26_3() {
    if run_fresh_cases(
        "cut_layout::game::client_side::text_displays::display_parity_tests::display_name_tags_and_fire_block_light_match_vanilla_26_3",
        "SUNRISE_DISPLAY_NAME_TAG_CASE",
        &["name_tag"],
    ) {
        return;
    }
    with_client_game(|game| {
        game.client.view_frustum = None;
        game.client.posed_rotation = Some((0.0, 0.0, 0.0));
        game.client.view.y_rot = 0.0;
        game.client.view.x_rot = 0.0;
        game.client.settings.render_distance = 8;
        game.client.settings.entity_distance = 1.0;
        let chunk = sunrise_level::chunk::Chunk::empty(0, 0);
        game.world.chunks.insert((0, 0), std::sync::Arc::new(chunk));

        // 1. Display name tags.
        assert!(game.spawn_remote_object(6001, "minecraft:text_display", [0.0; 3], [0.0; 3], 0));
        game.client
            .text_displays
            .0
            .get_mut(&6001)
            .unwrap()
            .metadata(&[
                (2, 0, Value::OptText(Some(Tag::String("DisplayTag".into())))),
                (3, 0, Value::Bool(true)),
            ]);
        // Pre-first-tick: DisplayRenderer.submit does not submit name tag.
        assert!(game.shown_name_tags(DVec3::new(0.0, 0.0, 5.0)).is_empty());
        // Post-first-tick with empty text: hasSubState() is true -> submits name tag.
        game.client
            .text_displays
            .0
            .get_mut(&6001)
            .unwrap()
            .metadata(&[(23, 0, Value::Text(Tag::String("".into())))]);
        game.tick_text_displays(game.clock.runs_normally());
        let tags = game.shown_name_tags(DVec3::new(0.0, 0.0, 5.0));
        assert!(tags.iter().any(|t| t == "DisplayTag"));
        // Zero view_range suppresses tag through the unified visibility decision.
        game.client
            .text_displays
            .0
            .get_mut(&6001)
            .unwrap()
            .metadata(&[(17, 2, Value::Float(0.0))]);
        assert!(game.shown_name_tags(DVec3::new(0.0, 0.0, 5.0)).is_empty());
        game.client
            .text_displays
            .0
            .get_mut(&6001)
            .unwrap()
            .metadata(&[(17, 2, Value::Float(1.0))]);
        // Beyond 64 blocks: suppressed.
        assert!(
            !game
                .shown_name_tags(DVec3::new(0.0, 0.0, 65.0))
                .iter()
                .any(|t| t == "DisplayTag")
        );
        // Display brightness override lights the solid name tag; shift key (discrete) suppresses see-through copy.
        game.client
            .text_displays
            .0
            .get_mut(&6001)
            .unwrap()
            .metadata(&[
                (16, 1, Value::Int((15 << 20) | (15 << 4))),
                (0, 0, Value::Byte(0x02)),
            ]);
        game.tick_text_displays(game.clock.runs_normally());
        let (mut solid_tag, mut faded_tag) = (Vec::new(), Vec::new());
        let mut tag_collector = std::mem::take(&mut game.client.see_through_collector);
        tag_collector.clear();
        game.name_tag_geometry(
            DVec3::new(0.0, 0.0, 5.0),
            1.0,
            &mut solid_tag,
            &mut tag_collector,
        );
        tag_collector.flush(&mut faded_tag);
        game.client.see_through_collector = tag_collector;
        assert!(!solid_tag.is_empty());
        assert!(
            faded_tag.is_empty(),
            "discrete display must omit see-through tag copy"
        );
        assert!(solid_tag.iter().all(|v| v.light == 0xF0F0));

        // CustomNameVisible = false (not crosshair pick): suppressed.
        game.client
            .text_displays
            .0
            .get_mut(&6001)
            .unwrap()
            .metadata(&[(3, 0, Value::Bool(false))]);
        assert!(
            !game
                .shown_name_tags(DVec3::new(0.0, 0.0, 5.0))
                .iter()
                .any(|t| t == "DisplayTag")
        );

        // 2. Burning display block light = 15 without brightness override; override wins when set.
        game.client.text_displays.0.clear();
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        assert!(game.spawn_remote_object(
            6002,
            "minecraft:block_display",
            [0.5, 64.0, 0.5],
            [0.0; 3],
            0,
        ));
        {
            let d = game.client.text_displays.0.get_mut(&6002).unwrap();
            d.show(&[(23, 0, Value::Id(stone.0 as i32))], None);
            d.metadata(&[(0, 0, Value::Byte(0x01))]); // FLAG_ONFIRE
        }
        game.tick_text_displays(game.clock.runs_normally());
        let mut out = Vec::new();
        game.block_item_display_geometry(DVec3::new(0.5, 64.0, 3.5), 1.0, &mut out);
        assert!(!out.is_empty());
        assert_eq!((out[0].light >> 8) & 0xff, 15 * 16);

        // Brightness override (block=3, sky=5) beats fire light 15.
        game.client
            .text_displays
            .0
            .get_mut(&6002)
            .unwrap()
            .metadata(&[(16, 1, Value::Int((5 << 20) | (3 << 4)))]);
        game.tick_text_displays(game.clock.runs_normally());
        out.clear();
        game.block_item_display_geometry(DVec3::new(0.5, 64.0, 3.5), 1.0, &mut out);
        assert!(!out.is_empty());
        assert_eq!((out[0].light >> 8) & 0xff, 3 * 16);
    });
}

const SECTION_GATE_NAME: &str = "cut_layout::game::client_side::text_displays::display_parity_tests::section_compiled_and_visible_gates_displays_and_mobs_26_3";
const SECTION_GATE_CASE: &str = "SUNRISE_DISPLAY_SECTION_GATE_CASE";

#[test]
#[ignore = "daily"]
fn section_compiled_and_visible_gates_displays_and_mobs_26_3() {
    if run_fresh_cases(SECTION_GATE_NAME, SECTION_GATE_CASE, &["section_gate"]) {
        return;
    }
    with_client_game(|game| {
        let _event_loop = frame_scene(game);
        game.client.view_frustum = None;
        game.client.posed_rotation = Some((0.0, 0.0, 0.0));
        game.client.view.y_rot = 0.0;
        game.client.view.x_rot = 0.0;
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let cam = DVec3::new(0.5, 65.5, 0.5);

        // Inside build height (y = 65.5), section not yet compiled -> hidden.
        assert!(game.spawn_remote_object(
            7001,
            "minecraft:text_display",
            [0.5, 65.5, 3.5],
            [0.0; 3],
            0,
        ));
        game.client
            .text_displays
            .0
            .get_mut(&7001)
            .unwrap()
            .metadata(&[(23, 0, Value::Text(Tag::String("In".into())))]);
        assert!(game.spawn_remote_object(
            7002,
            "minecraft:block_display",
            [0.5, 65.5, 3.5],
            [0.0; 3],
            0,
        ));
        game.client
            .text_displays
            .0
            .get_mut(&7002)
            .unwrap()
            .show(&[(23, 0, Value::Id(stone.0 as i32))], None);
        game.tick_text_displays(game.clock.runs_normally());
        let mob_start = game.mobs.len();
        game.spawn_entity(7003, "minecraft:zombie", [0.5, 65.5, 3.5], [0.0; 3]);
        assert_eq!(game.mobs.len(), mob_start + 1);
        let mob_range = mob_start..game.mobs.len();

        let (mut normal, mut see_through, mut blocks, mut mob_verts) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        collect_text_display_geometry(game, cam, 1.0, &mut normal, &mut see_through);
        game.block_item_display_geometry(cam, 1.0, &mut blocks);
        game.begin_skinning();
        let mob_frame = game.begin_mob_geometry(cam, 1.0);
        game.mob_run_geometry(&mob_frame, mob_range.clone(), &mut mob_verts);
        let mob_drawn = !mob_verts.is_empty() || game.client.skin_frame.borrow().vertices > 0;
        assert!(normal.is_empty(), "text display in uncompiled section");
        assert!(blocks.is_empty(), "block display in uncompiled section");
        assert!(!mob_drawn, "mob in uncompiled section");

        // Compiled empty section while fading (< 0.3 visibility) stays hidden; fade = 0 renders.
        let sy = game.world.shape.column().section_index(65).unwrap() as i32;
        game.client.settings.chunk_fade = 10.0;
        game.client
            .renderer
            .0
            .as_mut()
            .unwrap()
            .upload((0, sy, 0), &SectionMesh::default());
        normal.clear();
        blocks.clear();
        mob_verts.clear();
        collect_text_display_geometry(game, cam, 1.0, &mut normal, &mut see_through);
        game.block_item_display_geometry(cam, 1.0, &mut blocks);
        game.begin_skinning();
        let mob_frame = game.begin_mob_geometry(cam, 1.0);
        game.mob_run_geometry(&mob_frame, mob_range.clone(), &mut mob_verts);
        let mob_drawn = !mob_verts.is_empty() || game.client.skin_frame.borrow().vertices > 0;
        assert!(normal.is_empty(), "text display in fading empty section");
        assert!(blocks.is_empty(), "block display in fading empty section");
        assert!(!mob_drawn, "mob in fading empty section");

        game.client.settings.chunk_fade = 0.0;
        normal.clear();
        blocks.clear();
        mob_verts.clear();
        collect_text_display_geometry(game, cam, 1.0, &mut normal, &mut see_through);
        game.block_item_display_geometry(cam, 1.0, &mut blocks);
        game.begin_skinning();
        let mob_frame = game.begin_mob_geometry(cam, 1.0);
        game.mob_run_geometry(&mob_frame, mob_range, &mut mob_verts);
        let mob_drawn = !mob_verts.is_empty() || game.client.skin_frame.borrow().vertices > 0;
        assert!(!normal.is_empty(), "text display in visible empty section");
        assert!(!blocks.is_empty(), "block display in visible empty section");
        assert!(mob_drawn, "mob in visible empty section");

        // Above and below build height: always visible even when uncompiled.
        for (id, y, label) in [
            (7003, 340.0, "above_build_height_uncompiled"),
            (7004, -80.0, "below_build_height_uncompiled"),
        ] {
            game.client.text_displays.0.clear();
            assert!(game.spawn_remote_object(
                id,
                "minecraft:text_display",
                [0.5, y, 3.5],
                [0.0; 3],
                0,
            ));
            game.client
                .text_displays
                .0
                .get_mut(&id)
                .unwrap()
                .metadata(&[(23, 0, Value::Text(Tag::String("Out".into())))]);
            game.tick_text_displays(game.clock.runs_normally());
            normal.clear();
            collect_text_display_geometry(
                game,
                DVec3::new(0.5, y, 0.5),
                1.0,
                &mut normal,
                &mut see_through,
            );
            assert!(!normal.is_empty(), "{label}");
        }
    });
}

#[test]
#[ignore = "run by tools/test_displays_parity.py"]
fn bold_missing_glyph_displays_and_block_display_83224_match_vanilla_26_3() {
    if run_fresh_cases(
        "cut_layout::game::client_side::text_displays::display_parity_tests::bold_missing_glyph_displays_and_block_display_83224_match_vanilla_26_3",
        "SUNRISE_DISPLAY_83224_CASE",
        &["bold_83224"],
    ) {
        return;
    }
    let w = parity_witness();
    let argb_to_abgr = |v: &serde_json::Value| {
        let argb = u32::from_str_radix(v.as_str().unwrap(), 16).unwrap();
        sunrise_client::hud::rgba(
            (argb >> 16) as u8,
            (argb >> 8) as u8,
            argb as u8,
            (argb >> 24) as u8,
        )
    };

    with_client_game(|game| {
        game.client.view_frustum = None;
        game.client.camera_type = CameraType::FirstPerson;
        game.client.posed_rotation = Some((0.0, 0.0, 0.0));
        game.client.view.y_rot = 0.0;
        game.client.view.x_rot = 0.0;
        game.client.settings.render_distance = 8;
        game.client.settings.entity_distance = 1.0;

        // 1. Bold missing cursor in world TextDisplays matches vanilla TextDisplayRenderer bit-for-bit.
        for case_name in ["origin_fixed", "angled_billboard_transformed"] {
            let case = &w["missing_glyph_quads"]["text_display_world"][case_name];
            let ent_pos = hex_f64_arr::<3>(&case["ent_pos_bits"]);
            let ent_yaw = hex_f32(&case["ent_yaw_bits"]);
            let ent_pitch = hex_f32(&case["ent_pitch_bits"]);
            let cam_pos = DVec3::from_array(hex_f64_arr::<3>(&case["cam_pos_bits"]));
            let cam_yaw = hex_f32(&case["cam_yaw_bits"]);
            let cam_pitch = hex_f32(&case["cam_pitch_bits"]);
            let billboard = case["billboard"].as_i64().unwrap() as i8;
            let cursor_tag = decode_network_nbt_hex(&case["component_nbt_hex"]);

            game.client.posed_rotation = Some((cam_yaw, cam_pitch, 0.0));
            game.client.view.y_rot = cam_yaw;
            game.client.view.x_rot = cam_pitch;
            game.client.text_displays.0.clear();
            assert!(game.spawn_remote_object(
                83100,
                "minecraft:text_display",
                ent_pos,
                [ent_yaw, ent_pitch, 0.0],
                0,
            ));
            game.client
                .text_displays
                .0
                .get_mut(&83100)
                .unwrap()
                .metadata(&[
                    (
                        11,
                        0,
                        Value::Floats(hex_f32_arr::<3>(&case["translation_bits"]).to_vec()),
                    ),
                    (
                        12,
                        0,
                        Value::Floats(hex_f32_arr::<3>(&case["scale_bits"]).to_vec()),
                    ),
                    (
                        13,
                        0,
                        Value::Floats(hex_f32_arr::<4>(&case["left_rotation_bits"]).to_vec()),
                    ),
                    (
                        14,
                        0,
                        Value::Floats(hex_f32_arr::<4>(&case["right_rotation_bits"]).to_vec()),
                    ),
                    (15, 0, Value::Byte(billboard)),
                    (23, 0, Value::Text(cursor_tag)),
                    (25, 1, Value::Int(0)),
                    (26, 0, Value::Byte(-1)),
                    (27, 0, Value::Byte(1)),
                ]);
            game.tick_text_displays(game.clock.runs_normally());
            let (mut normal, mut see_through) = (Vec::new(), Vec::new());
            collect_text_display_geometry(game, cam_pos, 1.0, &mut normal, &mut see_through);
            let want_quads = case["quads"].as_array().unwrap();
            assert_eq!(
                normal.len(),
                want_quads.len() * 4,
                "{case_name}: bold+shadow missing glyph TextDisplay must emit {} quads, got {}",
                want_quads.len(),
                normal.len() / 4
            );
            // Sunrise vertex [(k + 3) % 4] matches BakedSheetGlyph corner k.
            for (qi, (got_q, want_q)) in
                normal.as_chunks::<4>().0.iter().zip(want_quads).enumerate()
            {
                for (k, want_v) in want_q.as_array().unwrap().iter().enumerate() {
                    let gv = got_q[(k + 3) % 4];
                    let wp = hex_u32_vec(&want_v["pos_bits"]);
                    assert_eq!(
                        gv.pos.map(f32::to_bits).to_vec(),
                        wp,
                        "{case_name} quad {qi} corner {k} pos bits: got {:08x?} vs vanilla {:08x?}",
                        gv.pos.map(f32::to_bits),
                        wp
                    );
                    assert_eq!(gv.color, argb_to_abgr(&want_v["color_argb"]));
                }
            }
        }

        // 2. EID 83224 (minecraft:lime_concrete): exact 6-face vertex parity + opaque atlas padding.
        game.client.posed_rotation = Some((0.0, 0.0, 0.0));
        game.client.view.y_rot = 0.0;
        game.client.view.x_rot = 0.0;
        let ent_pos = [
            f64::from_bits(0x3fe0_0000_0000_0000),
            f64::from_bits(0x4072_d9eb_851e_b852),
            f64::from_bits(0x3fe0_0000_0000_0000),
        ];
        let cam_pos = DVec3::new(
            f64::from_bits(0x3fe0_0000_0000_0000),
            f64::from_bits(0x4072_d9eb_94c0_0000),
            f64::from_bits(0x3fe0_0000_0000_0000),
        );
        let lime = game
            .pack
            .assets
            .registry
            .default_state("minecraft:lime_concrete")
            .unwrap();
        game.client.text_displays.0.clear();
        assert!(game.spawn_remote_object(83224, "minecraft:block_display", ent_pos, [0.0; 3], 0,));
        {
            let d = game.client.text_displays.0.get_mut(&83224).unwrap();
            d.show(&[(23, 0, Value::Id(lime.0 as i32))], None);
            d.metadata(&[
                (15, 0, Value::Byte(3)),
                (
                    11,
                    0,
                    Value::Floats(vec![
                        f32::from_bits(0x4039_c28f),
                        f32::from_bits(0x4001_0000),
                        f32::from_bits(0xc089_eb85),
                    ]),
                ),
                (
                    12,
                    0,
                    Value::Floats(vec![
                        f32::from_bits(0x3fce_6666),
                        f32::from_bits(0x3d5c_28f6),
                        f32::from_bits(0x3ca3_d70a),
                    ]),
                ),
                (13, 0, Value::Floats(vec![0.0, 0.0, 0.0, 1.0])),
                (14, 0, Value::Floats(vec![0.0, 0.0, 0.0, 1.0])),
                (16, 1, Value::Int(15_728_880)),
            ]);
        }
        game.tick_text_displays(game.clock.runs_normally());
        let mut block_verts = Vec::new();
        game.block_item_display_geometry(cam_pos, 1.0, &mut block_verts);
        assert_eq!(block_verts.len(), 24, "EID 83224 must emit 6 cube faces");
        let want_faces = w["block_display_83224"]["faces"].as_object().unwrap();
        for dir in ["down", "up", "north", "south", "west", "east"] {
            let want_pos: Vec<[u32; 3]> = want_faces[dir]["emitted_vertices"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| hex_f32_arr::<3>(&v["pos_bits"]).map(f32::to_bits))
                .collect();
            let found = block_verts.as_chunks::<4>().0.iter().any(|q| {
                q.iter()
                    .map(|v| v.pos.map(f32::to_bits))
                    .collect::<Vec<_>>()
                    == want_pos
            });
            assert!(
                found,
                "EID 83224 missing exact vanilla face {dir}: expected {want_pos:08x?}"
            );
        }
        let sprite = game.client.gui.atlas.sprites["block/lime_concrete"];
        let px = (sprite.u0 * game.client.gui.atlas.width as f32).round() as u32;
        let py = (sprite.v0 * game.client.gui.atlas.height as f32).round() as u32;
        let pixels = game.client.gui.atlas.pixels();
        assert!(px >= 1 && py >= 1);
        for (label, bx, by) in [
            ("left", px - 1, py),
            ("right", px + 16, py),
            ("top", px, py - 1),
            ("bottom", px, py + 16),
        ] {
            let a = pixels[((by * game.client.gui.atlas.width + bx) * 4 + 3) as usize];
            assert_eq!(
                a, 255,
                "block/lime_concrete {label} border pixel ({bx},{by}) must be padded opaque, not transparent gutter"
            );
        }
    });
}

#[test]
#[ignore = "daily"]
fn set_camera_on_display_and_unbobbed_culling_match_vanilla_26_3() {
    if run_fresh_cases(
        "cut_layout::game::client_side::text_displays::display_parity_tests::set_camera_on_display_and_unbobbed_culling_match_vanilla_26_3",
        "SUNRISE_DISPLAY_SET_CAMERA_CASE",
        &["set_camera"],
    ) {
        return;
    }
    use sunrise_client::level_loading::Reason;
    use sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry;
    use sunrise_protocol::{
        connection::Pipe,
        packets::play::{self, Clientbound as C, SpawnInfo},
    };
    with_client_game(|game| {
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "DisplayCamTest").unwrap();
        let spawn = SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: 0,
            game_type: 3,
            previous_game_type: None,
            debug: false,
            flat: true,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        };
        game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Login, Reason::Other);
        let remote = game.client.remote.as_mut().unwrap();
        remote.logged_in = true;
        remote.entity_id = 7;

        for cz in -1..=0 {
            let chunk = sunrise_level::chunk::Chunk::empty(0, cz);
            game.world
                .chunks
                .insert((0, cz), std::sync::Arc::new(chunk));
        }

        let td_kind = game
            .client
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:entity_type")
            .unwrap()
            .id("minecraft:text_display")
            .unwrap();
        game.handle_packet(C::AddEntity(play::AddEntity {
            id: 9001,
            uuid: 9001,
            kind: td_kind,
            position: [0.5, 64.0, 0.5],
            velocity: [0.0; 3],
            pitch: 0,
            yaw: 0,
            head_yaw: 0,
            data: 0,
        }))
        .unwrap();
        game.client
            .text_displays
            .0
            .get_mut(&9001)
            .unwrap()
            .metadata(&[(23, 0, Value::Text(Tag::String("CamDisplay".into())))]);
        game.tick_text_displays(true);

        // SetCamera packet targeting a tracked display attaches camera, hides in first person, shows in third person.
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 9001 }))
            .unwrap();
        assert_eq!(game.remote_camera_id(), Some(9001));
        let snap = game
            .spectated_camera_snapshot()
            .expect("camera follows display");
        assert_eq!(snap.position, [0.5, 64.0, 0.5]);
        assert_eq!(snap.eye_height, 0.0);

        game.client.view_frustum = None;
        game.client.posed_rotation = Some((0.0, 0.0, 0.0));
        game.client.view.y_rot = 0.0;
        game.client.view.x_rot = 0.0;
        game.client.camera_type = CameraType::FirstPerson;
        let (mut normal, mut see_through) = (Vec::new(), Vec::new());
        collect_text_display_geometry(
            game,
            DVec3::new(0.5, 64.0, 0.5),
            1.0,
            &mut normal,
            &mut see_through,
        );
        assert!(
            normal.is_empty(),
            "camera display must be hidden in first person"
        );

        game.client.camera_type = CameraType::ThirdPersonBack;
        collect_text_display_geometry(
            game,
            DVec3::new(0.5, 64.0, 0.5),
            1.0,
            &mut normal,
            &mut see_through,
        );
        assert!(
            !normal.is_empty(),
            "camera display must render in third person"
        );

        // Return camera to player and verify shared see-through sort interleaves name tag & text display.
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 7 }))
            .unwrap();
        assert_eq!(game.remote_camera_id(), None);
        game.client.camera_type = CameraType::FirstPerson;
        game.client.text_displays.0.clear();
        assert!(game.spawn_remote_object(
            9002,
            "minecraft:text_display",
            [0.0, 64.0, -2.0],
            [0.0; 3],
            0,
        ));
        game.client
            .text_displays
            .0
            .get_mut(&9002)
            .unwrap()
            .metadata(&[
                (2, 0, Value::OptText(Some(Tag::String("NearTag".into())))),
                (3, 0, Value::Bool(true)),
                (23, 0, Value::Text(Tag::String("".into()))),
            ]);
        assert!(game.spawn_remote_object(
            9003,
            "minecraft:text_display",
            [0.0, 64.0, -6.0],
            [0.0; 3],
            0,
        ));
        game.client
            .text_displays
            .0
            .get_mut(&9003)
            .unwrap()
            .metadata(&[
                (23, 0, Value::Text(Tag::String("FarHolo".into()))),
                (25, 1, Value::Int(0xFF11_2233u32 as i32)),
                (26, 0, Value::Byte(-1)),
                (27, 0, Value::Byte(2)), // see_through
            ]);
        game.tick_text_displays(true);
        let mut collector =
            sunrise_client_gameplay::game::client_side::text_displays::SeeThroughCollector::default(
            );
        let (mut solid, mut st_out) = (Vec::new(), Vec::new());
        let cam = DVec3::new(0.0, 64.0, 0.0);
        game.name_tag_geometry(cam, 1.0, &mut solid, &mut collector);
        game.text_display_geometry(cam, 1.0, &mut solid, &mut collector);
        collector.flush(&mut st_out);
        let far_bg = 0xFF33_2211u32;
        let first_far = st_out
            .as_chunks::<4>()
            .0
            .iter()
            .position(|q| q[0].color == far_bg)
            .expect("far see-through hologram background");
        assert_eq!(
            first_far, 0,
            "farther see-through hologram must sort before nearer faded name tag"
        );
    });
}

#[test]
#[ignore = "run by tools/test_displays_parity.py"]
fn review_parity_witness_matches_vanilla_26_3() {
    let w = parity_witness();
    let rp = &w["review_parity"];

    // 1. Movement packets: relative yaw against active target and >4096 oldPosAndRot.
    use sunrise_protocol::packets::play::relative as rel;
    let mv = &rp["movement_packets"];
    let mut dt = RemoteTextDisplay::new(Shows::Text, [0.0; 3], 0.0);
    dt.metadata(&[(10, 1, Value::Int(10))]);
    dt.move_to([10.0, 0.0, 0.0], Some((30.0, 0.0)));
    let flags = rel::X | rel::Y | rel::Z | rel::Y_ROT;
    assert_eq!(
        dt.apply_teleport_packet([0.0; 3], 15.0, 0.0, flags).1,
        mv["rel_yaw_was_interpolated"].as_bool().unwrap()
    );
    for _ in 0..10 {
        dt.tick();
    }
    assert_eq!(
        dt.rotation[0].to_bits(),
        hex_u32(&mv["rel_yaw_final_yRot_bits"])
    );

    let mut df = RemoteTextDisplay::new(Shows::Text, [1.0, 2.0, 3.0], 10.0);
    df.snap_to(DVec3::new(1.0, 2.0, 3.0), 10.0, 5.0);
    df.previous = DVec3::new(-4.0, -5.0, -6.0);
    df.old_rotation = [20.0, -10.0];
    assert_eq!(
        df.apply_teleport_packet([100.0, 0.0, 0.0], 15.0, 0.0, flags)
            .1,
        mv["far_was_interpolated"].as_bool().unwrap()
    );
    assert_eq!(
        df.position.to_array(),
        hex_f64_arr::<3>(&mv["far_pos_bits"])
    );
    assert_eq!(
        df.previous.to_array(),
        hex_f64_arr::<3>(&mv["far_old_pos_bits"])
    );
    assert_eq!(
        df.rotation.map(f32::to_bits).to_vec(),
        hex_u32_vec(&mv["far_rot_bits"])
    );
    assert_eq!(
        df.old_rotation.map(f32::to_bits).to_vec(),
        hex_u32_vec(&mv["far_old_rot_bits"])
    );

    // 2. Rotation arithmetic and endpoints.
    let rot = &rp["rotation_arithmetic"];
    let mut d3 = RemoteTextDisplay::new(Shows::Text, [0.0; 3], 0.0);
    d3.metadata(&[(10, 1, Value::Int(3))]);
    d3.move_to([0.0; 3], Some((0.0, 7.0)));
    d3.tick();
    assert_eq!(
        d3.rotation[1].to_bits(),
        hex_u32(&rot["pitch_0_to_7_tick1_bits"])
    );
    let mut dsnap = RemoteTextDisplay::new(Shows::Text, [0.0; 3], 0.0);
    dsnap.snap_to(DVec3::ZERO, 400.0, 400.0);
    assert_eq!(
        dsnap.rotation[0].to_bits(),
        hex_u32(&rot["snap_400_yRot_bits"])
    );
    assert_eq!(
        dsnap.rotation[1].to_bits(),
        hex_u32(&rot["snap_400_xRot_bits"])
    );
    dsnap.set_y_rot_raw(f32::NAN);
    dsnap.set_x_rot_raw(f32::INFINITY);
    assert_eq!(
        dsnap.rotation[0].to_bits(),
        hex_u32(&rot["nonfinite_retained_yRot_bits"])
    );
    assert_eq!(
        dsnap.rotation[1].to_bits(),
        hex_u32(&rot["nonfinite_retained_xRot_bits"])
    );
    dsnap.snap_to(DVec3::ZERO, 0.0, 0.0);
    dsnap.old_rotation[0] = 350.0;
    assert_eq!(
        dsnap.rotation_at(1.0)[0].to_bits(),
        hex_u32(&rot["getYRot_partial1_bits"])
    );

    // 3. Partial moves at duration 0, negative duration repeat, and seated adjustment.
    let pm = &rp["partial_moves_and_seating"];
    let mut dp = RemoteTextDisplay::new(Shows::Text, [0.0; 3], 0.0);
    dp.metadata(&[(10, 1, Value::Int(4))]);
    dp.move_to([10.0, 0.0, 0.0], Some((0.0, 0.0)));
    dp.tick();
    dp.metadata(&[(10, 1, Value::Int(0))]);
    dp.move_to_opt(None, Some((45.0, 0.0)));
    assert_eq!(
        dp.position.x.to_bits(),
        hex_u64(&pm["partial_rot_at_dur0_x_bits"])
    );
    assert_eq!(
        dp.rotation[0].to_bits(),
        hex_u32(&pm["partial_rot_at_dur0_yRot_bits"])
    );

    let mut dneg = RemoteTextDisplay::new(Shows::Text, [0.0; 3], 0.0);
    dneg.metadata(&[(10, 1, Value::Int(4))]);
    dneg.move_to([4.0, 0.0, 0.0], Some((0.0, 0.0)));
    dneg.tick();
    dneg.metadata(&[(10, 1, Value::Int(-2))]);
    dneg.move_to([4.0, 0.0, 0.0], Some((0.0, 0.0)));
    dneg.tick();
    assert_eq!(
        dneg.position.x.to_bits(),
        hex_u64(&pm["repeat_same_target_neg_dur_x_bits"])
    );

    let mut dseat = RemoteTextDisplay::new(Shows::Text, [0.0; 3], 0.0);
    dseat.metadata(&[(10, 1, Value::Int(4))]);
    dseat.move_to([0.0; 3], Some((40.0, 0.0)));
    dseat.tick();
    dseat.ride(DVec3::new(5.0, 2.0, -1.0));
    dseat.tick();
    assert_eq!(
        dseat.position.to_array(),
        hex_f64_arr::<3>(&pm["seated_adjust_pos_bits"])
    );
    assert_eq!(
        dseat.rotation[0].to_bits(),
        hex_u32(&pm["seated_adjust_yRot_bits"])
    );

    // 5. Transformation interpolation state: int wrap, negative duration shadow radius, t=1.0 large translation.
    let tx = &rp["transformation_state"];
    let mut dw = RemoteTextDisplay::new(Shows::Text, [0.0; 3], 0.0);
    dw.metadata(&[(23, 0, Value::Text(Tag::String("W".into())))]);
    dw.tick();
    dw.metadata(&[
        (8, 1, Value::Int(i32::MAX)),
        (9, 1, Value::Int(4)),
        (11, 0, Value::Floats(vec![4.0, 0.0, 0.0])),
    ]);
    dw.tick();
    assert_eq!(dw.start, tx["wrapped_start_tick"].as_i64().unwrap());
    assert_eq!(
        dw.progress(0.0).to_bits(),
        hex_u32(&tx["wrapped_progress_0_bits"])
    );

    let mut dsr = RemoteTextDisplay::new(Shows::Text, [0.0; 3], 0.0);
    dsr.metadata(&[
        (23, 0, Value::Text(Tag::String("S".into()))),
        (18, 2, Value::Float(1.0)),
    ]);
    dsr.tick();
    dsr.metadata(&[
        (8, 1, Value::Int(0)),
        (9, 1, Value::Int(-2)),
        (18, 2, Value::Float(5.0)),
    ]);
    dsr.tick();
    dsr.metadata(&[(8, 1, Value::Int(0)), (9, 1, Value::Int(4))]);
    dsr.tick();
    let sr_p = dsr.calculate_interpolation_progress(0.0);
    assert_eq!(
        dsr.eval_render_state(sr_p).shadow_radius.to_bits(),
        hex_u32(&tx["neg_dur_then_start_only_shadow_r_at_0_bits"])
    );

    let from_t = sunrise_client::text_display::TextDisplayData {
        translation: [33_554_432.0, 0.0, 0.0],
        ..Default::default()
    };
    let to_t = sunrise_client::text_display::TextDisplayData {
        translation: [1.0, 0.0, 0.0],
        ..Default::default()
    };
    assert_eq!(
        to_t.blend(&from_t, 1.0).translation[0].to_bits(),
        hex_u32(&tx["large_trans_at_1_x_bits"])
    );

    // 6. NaN view range and subnormal / infinite culling box.
    let rg = &rp["range_and_aabb"];
    let mut dr = RemoteTextDisplay::new(Shows::Text, [0.0; 3], 0.0);
    dr.metadata(&[(17, 2, Value::Float(f32::NAN))]);
    assert_eq!(
        dr.in_range(DVec3::ZERO, 1.0),
        rg["nan_view_range_renders_at_0"].as_bool().unwrap()
    );
    dr.metadata(&[
        (20, 2, Value::Float(f32::from_bits(1))),
        (21, 2, Value::Float(1.0)),
    ]);
    let (slo, shi) = dr.interpolated_bounding_box(1.0);
    assert_eq!(
        [slo.x, slo.y, slo.z, shi.x, shi.y, shi.z].map(f64::to_bits),
        hex_f64_arr::<6>(&rg["subnormal_width_box_bits"]).map(f64::to_bits)
    );
    dr.metadata(&[(20, 2, Value::Float(f32::INFINITY))]);
    let (ilo, ihi) = dr.interpolated_bounding_box(1.0);
    assert_eq!(
        [ilo.x, ilo.y, ilo.z, ihi.x, ihi.y, ihi.z].map(f64::to_bits),
        hex_f64_arr::<6>(&rg["inf_width_box_bits"]).map(f64::to_bits)
    );

    // 14. Live glow_color_override red -> blue.
    let gl = &rp["glow_color_override"];
    let mut dg = RemoteTextDisplay::new(Shows::Block, [0.0; 3], 0.0);
    dg.metadata(&[(22, 1, Value::Int(0xFF0000))]);
    assert_eq!(dg.data.glow_color as i64, gl["red"].as_i64().unwrap());
    dg.metadata(&[(22, 1, Value::Int(0x0000FF))]);
    assert_eq!(dg.data.glow_color as i64, gl["blue"].as_i64().unwrap());

    // 15. JOML exact arithmetic: translateX(16777216) * rotateZ(-45deg) on (1, 1, 0),
    // plus TextDisplayData::pose's Transformation.compose quaternion path and JOML slerp.
    let jm = &rp["joml_arithmetic"];
    let ang = (-45.0_f64).to_radians() as f32;
    let mut m = sunrise_client::camera::joml::Mat4::IDENTITY;
    m.translate(16_777_216.0, 0.0, 0.0);
    m.rotate_z(ang);
    let pt = m.transform_position([1.0, 1.0, 0.0]);
    assert_eq!(
        pt.map(f32::to_bits).to_vec(),
        hex_u32_vec(&jm["rot_neg45_trans_16m_bits"])
    );
    let td_jm = sunrise_client::text_display::TextDisplayData {
        translation: [16_777_216.0, 0.0, 0.0],
        left_rotation: [0.0, 0.0, (ang * 0.5).sin(), (ang * 0.5).cos()],
        ..Default::default()
    };
    let td_pt = td_jm
        .pose([0.0; 3], [0.0; 2], [0.0; 2])
        .transform_position([1.0, 1.0, 0.0]);
    assert_eq!(
        td_pt.map(f32::to_bits).to_vec(),
        hex_u32_vec(&jm["compose_rot_neg45_trans_16m_bits"])
    );
    let norm = |q: [f32; 4]| {
        let len_sq = q[0] * q[0] + (q[1] * q[1] + (q[2] * q[2] + q[3] * q[3]));
        let inv = 1.0 / (f64::from(len_sq).sqrt() as f32);
        q.map(|c| c * inv)
    };
    let qa = sunrise_client::text_display::TextDisplayData {
        left_rotation: norm([0.1, 0.2, 0.3, 0.9]),
        ..Default::default()
    };
    let qb = sunrise_client::text_display::TextDisplayData {
        left_rotation: norm([-0.4, 0.5, -0.2, 0.7]),
        ..Default::default()
    };
    assert_eq!(
        qb.blend(&qa, 0.37).left_rotation.map(f32::to_bits).to_vec(),
        hex_u32_vec(&jm["slerp_bits"])
    );
}
