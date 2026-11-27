use crate::game::server_side::native_entity_refs::tests::with_client_game;
use serde_json::Value;
use std::sync::Arc;
use sunrise_client::screen_effects::Blend;
use sunrise_core::geometry::Vec3;
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::{Chunk, Light};
use sunrise_protocol::{
    buf::pack_degrees,
    entity_data::Value as EntityDataValue,
    packets::play::{self, Clientbound as C},
};

pub(in crate::game) fn parity_witness() -> Value {
    let path = std::env::var("SUNRISE_CAMERA_PARITY_WITNESS")
        .expect("run through tools/test_camera_parity.py, which records the witness");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "failed to read SUNRISE_CAMERA_PARITY_WITNESS={path} (run through tools/test_camera_parity.py): {err}"
        )
    });
    serde_json::from_str(&raw).expect("valid 26.3 camera parity witness")
}

pub(in crate::game) fn hex_f32(v: &Value) -> u32 {
    u32::from_str_radix(v.as_str().unwrap(), 16).unwrap()
}

pub(in crate::game) fn hex_f64(v: &Value) -> u64 {
    u64::from_str_radix(v.as_str().unwrap(), 16).unwrap()
}

pub(in crate::game) fn vec3_from_bits(v: &Value) -> [f64; 3] {
    [
        f64::from_bits(hex_f64(&v[0])),
        f64::from_bits(hex_f64(&v[1])),
        f64::from_bits(hex_f64(&v[2])),
    ]
}

pub(in crate::game) fn vec3_bits_expected(v: &Value) -> [u64; 3] {
    [hex_f64(&v[0]), hex_f64(&v[1]), hex_f64(&v[2])]
}

pub(in crate::game) fn entity_type_id(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    kind: &str,
) -> i32 {
    game.client
        .remote
        .as_ref()
        .unwrap()
        .builtin
        .get("minecraft:entity_type")
        .unwrap()
        .id(kind)
        .unwrap_or_else(|| panic!("missing entity type {kind}"))
}

#[allow(unused_imports)]
pub(in crate::game) use crate::game::client_side::remote_client::tests::camera_parity_red_tests::setup_remote_overworld;

pub(in crate::game) fn add_player_info(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    id: i32,
    uuid: u128,
) {
    game.handle_packet(C::PlayerInfoUpdate(play::PlayerInfoUpdate {
        actions: 1, // ADD_PLAYER
        entries: vec![play::PlayerInfo {
            uuid,
            name: Some(format!("Remote{id}")),
            ..Default::default()
        }],
    }))
    .unwrap();
}

pub(in crate::game) fn add_entity(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    id: i32,
    kind: &str,
    position: [f64; 3],
    yaw: f32,
    pitch: f32,
) {
    let uuid = id as u128;
    if kind == "minecraft:player" {
        add_player_info(game, id, uuid);
    }
    let kind_id = entity_type_id(game, kind);
    let y = pack_degrees(yaw);
    let p = pack_degrees(pitch);
    game.handle_packet(C::AddEntity(play::AddEntity {
        id,
        uuid,
        kind: kind_id,
        position,
        velocity: [0.0; 3],
        pitch: p,
        yaw: y,
        head_yaw: y,
        data: 0,
    }))
    .unwrap();
}

pub(in crate::game) fn sync_interaction_size(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    id: i32,
    width: f32,
    height: f32,
) {
    game.handle_packet(C::SetEntityData(play::SetEntityData {
        id,
        values: vec![
            (8, 3, EntityDataValue::Float(width)),
            (9, 3, EntityDataValue::Float(height)),
        ],
    }))
    .unwrap();
}

pub(in crate::game) fn make_chunk(cx: i32, cz: i32, biome_id: u16, uniform_light: u8) -> Chunk {
    let mut chunk = Chunk::empty(cx, cz);
    chunk.lit = true;
    for qy in 0..96 {
        for qz in 0..4 {
            for qx in 0..4 {
                chunk.set_biome_cell(qx, qy, qz, biome_id);
            }
        }
    }
    for section in &mut chunk.sections {
        Arc::make_mut(section).light = Light::Uniform(uniform_light);
    }
    chunk
}

/// Item 1: every tracked entity kind resolves through the single camera resolver (26.3 oracle).
#[test]
#[ignore = "run by tools/test_camera_parity.py"]
fn any_tracked_entity_resolves_as_camera_26_3() {
    run_any_tracked_entity_resolves_as_camera_26_3();
}

#[allow(unused_imports)]
pub(in crate::game) use crate::game::client_side::remote_client::tests::camera_parity_red_tests::run_any_tracked_entity_resolves_as_camera_26_3;

/// Item 12: Interaction eye height, rotation, move/teleport interpolation, and far snap (26.3).
#[test]
#[ignore = "run by tools/test_camera_parity.py"]
fn interaction_camera_eye_rotation_move_and_teleport_26_3() {
    run_interaction_camera_eye_rotation_move_and_teleport_26_3();
}

#[allow(unused_imports)]
pub(in crate::game) use crate::game::client_side::remote_client::tests::camera_parity_red_tests::run_interaction_camera_eye_rotation_move_and_teleport_26_3;

/// Item 12: RemoveEntities retains the detached camera entity and post effect until SetCamera (26.3).
#[test]
#[ignore = "run by tools/test_camera_parity.py"]
fn removing_camera_entity_keeps_detached_camera_26_3() {
    run_removing_camera_entity_keeps_detached_camera_26_3();
}

#[allow(unused_imports)]
pub(in crate::game) use crate::game::client_side::remote_client::tests::camera_parity_red_tests::run_removing_camera_entity_keeps_detached_camera_26_3;

/// Item 6: bob, fog, dark disc, vignette, border warning, and F1 hide_gui follow 26.3 camera rules.
#[test]
#[ignore = "run by tools/test_camera_parity.py"]
fn camera_consumers_bob_fog_dark_disc_and_vignette_26_3() {
    run_camera_consumers_bob_fog_dark_disc_and_vignette_26_3();
}

#[allow(unused_imports)]
pub(in crate::game) use crate::game::client_side::remote_client::tests::camera_parity_red_tests::run_camera_consumers_bob_fog_dark_disc_and_vignette_26_3;

/// Item 6: streaming center, particles, F3 debug entries/gizmos, and spectator crosshair (26.3).
#[test]
#[ignore = "run by tools/test_camera_parity.py"]
fn camera_consumers_streaming_particles_debug_and_spectator_crosshair_26_3() {
    run_camera_consumers_streaming_particles_debug_and_spectator_crosshair_26_3();
}

#[allow(unused_imports)]
pub(in crate::game) use crate::game::client_side::remote_client::tests::camera_parity_red_tests::run_camera_consumers_streaming_particles_debug_and_spectator_crosshair_26_3;

/// Item 6: frozen-level camera partial tick matches 26.3 `Camera.getCameraEntityPartialTicks`.
#[test]
#[ignore = "run by tools/test_camera_parity.py"]
fn frozen_camera_partial_tick_matches_26_3_witness() {
    run_frozen_camera_partial_tick_matches_26_3_witness();
}

pub(in crate::game) fn run_frozen_camera_partial_tick_matches_26_3_witness() {
    let witness = parity_witness();
    let frozen = &witness["camera_consumers"]["frozen_partial"];
    with_client_game(|game| {
        let _server = setup_remote_overworld(game, 1, [0.5, 64.0, 0.5], true);
        add_entity(
            game,
            2,
            "minecraft:interaction",
            [0.5, 300.0, 0.5],
            0.0,
            0.0,
        );
        add_entity(game, 3, "minecraft:creeper", [5.0, 64.0, 5.0], 0.0, 0.0);
        add_entity(game, 4, "minecraft:minecart", [5.0, 64.0, 5.0], 0.0, 0.0);
        game.handle_packet(C::SetPassengers(play::SetPassengers {
            vehicle: 4,
            passengers: vec![1],
        }))
        .unwrap();

        game.clock.reset();
        game.clock.set_frozen(true);
        game.clock.tick_world_runs();
        game.clock.end_frame(false);
        // 3ms / 8ms = 0.375 exactly (0x3ec00000), matching real_partial_bits in witness.
        let ran = game.clock.advance(std::time::Duration::from_millis(3), 8.0);
        assert_eq!(ran, 0);
        assert_eq!(
            (game.clock.camera_fraction() as f32).to_bits(),
            hex_f32(&frozen["real_partial_bits"]),
            "setup residual must match real_partial_bits"
        );

        for (label, cam_id, key) in [
            ("local_player", 1, "local_player_partial_bits"),
            ("interaction", 2, "interaction_partial_bits"),
            ("creeper", 3, "creeper_partial_bits"),
            (
                "minecart_with_player",
                4,
                "minecart_with_player_partial_bits",
            ),
        ] {
            game.handle_packet(C::SetCamera(play::SetCamera { camera_id: cam_id }))
                .unwrap();
            assert_eq!(
                (game.camera_partial_tick() as f32).to_bits(),
                hex_f32(&frozen[key]),
                "frozen camera partial tick for {label}"
            );
        }
    });
}

/// Item 6: block outline requires a player camera entity (`GameRenderer.shouldRenderBlockOutline`).
#[test]
#[ignore = "run by tools/test_camera_parity.py"]
fn block_outline_requires_player_camera_26_3() {
    run_block_outline_requires_player_camera_26_3();
}

pub(in crate::game) fn run_block_outline_requires_player_camera_26_3() {
    let witness = parity_witness();
    let outline = &witness["camera_consumers"]["block_outline"];
    with_client_game(|game| {
        let _server = setup_remote_overworld(game, 1, [0.5, 64.0, 0.5], true);
        game.handle_packet(C::GameEvent(play::GameEvent {
            event: 3,
            param: 1.0,
        }))
        .unwrap();
        game.chat_ui_mut_for_test().hide_gui = false;
        add_entity(
            game,
            2,
            "minecraft:interaction",
            [0.5, 300.0, 0.5],
            0.0,
            0.0,
        );
        add_entity(game, 3, "minecraft:creeper", [5.0, 64.0, 5.0], 0.0, 0.0);

        for (label, cam_id) in [("local_player", 1), ("interaction", 2), ("creeper", 3)] {
            game.handle_packet(C::SetCamera(play::SetCamera { camera_id: cam_id }))
                .unwrap();
            assert_eq!(
                game.should_render_block_outline(),
                outline[label].as_bool().unwrap(),
                "should_render_block_outline for {label}"
            );
        }
    });
}

/// Item 14: culling near plane uses max(fov, option_fov) while scoping with a spyglass (26.3).
#[test]
#[ignore = "run by tools/test_camera_parity.py"]
fn culling_near_plane_uses_max_of_fov_and_option_fov_26_3() {
    run_culling_near_plane_uses_max_of_fov_and_option_fov_26_3();
}

pub(in crate::game) fn run_culling_near_plane_uses_max_of_fov_and_option_fov_26_3() {
    let witness = parity_witness();
    let case = witness["cull_frustum"]["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "zoomed_fov_30_opt_70")
        .unwrap();
    let near_max_x = f64::from_bits(hex_f64(&case["near_plane_bounds_bits"][3]));
    assert!(
        0.99 + near_max_x > 1.0,
        "26.3 70-deg cull near plane must cross x=1.0 from x=0.99"
    );

    with_client_game(|game| {
        // Bottom stone_slab at [1,65,0] uses NEAR_PLANE_INTERSECTS_OUTLINE: 70-deg hits, 7-deg misses.
        let _server = setup_remote_overworld(game, 1, [0.99, 63.6, 0.5], true);
        game.world
            .chunks
            .insert((0, 0), Arc::new(make_chunk(0, 0, 0, 0xF0)));
        let slab = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone_slab")
            .unwrap();
        game.world.set(1, 65, 0, slab).unwrap();

        game.yaw = 0.0;
        game.pitch = 0.0;
        game.client.settings.fov = 70.0;
        game.chat_ui_mut_for_test().hide_gui = true;
        let hand_proj = sunrise_client::camera::hand_projection(70.0, 1280.0, 800.0, 512.0);
        for _ in 0..20 {
            game.tick_fov();
        }
        let baseline_quads = game.screen_quads(1.0, [1280, 800], 1.0, hand_proj);
        assert!(
            baseline_quads
                .iter()
                .any(|q| q.blend == Blend::Alpha && !q.world_tint),
            "baseline 70-deg near plane must intersect stone_slab at x=1.0"
        );

        game.inventory[0] = Some(ItemStack::new("minecraft:spyglass", 1));
        game.selected = 0;
        assert!(game.start_using_item());
        for _ in 0..20 {
            game.tick_fov();
        }
        let eye = [0.99, 65.22, 0.5];
        assert!(
            game.fov_degrees(1.0, eye) < 15.0,
            "spyglass must narrow render FOV well below option FOV 70"
        );

        let quads = game.screen_quads(1.0, [1280, 800], 1.0, hand_proj);
        assert!(
            quads
                .iter()
                .any(|q| q.blend == Blend::Alpha && !q.world_tint),
            "cull near plane must use max(fov, option_fov)=70 so stone_slab at x=1.0 is view-blocking"
        );
    });
}

pub(in crate::game) fn add_entity_with_data(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    id: i32,
    kind: &str,
    position: [f64; 3],
    yaw: f32,
    pitch: f32,
    data: i32,
) {
    let uuid = id as u128 + 1000;
    if kind == "minecraft:player" {
        add_player_info(game, id, uuid);
    }
    let kind_id = entity_type_id(game, kind);
    let y = pack_degrees(yaw);
    let p = pack_degrees(pitch);
    game.handle_packet(C::AddEntity(play::AddEntity {
        id,
        uuid,
        kind: kind_id,
        position,
        velocity: [0.0; 3],
        pitch: p,
        yaw: y,
        head_yaw: y,
        data,
    }))
    .unwrap();
}

/// Parity #1: any spectated sleeping living entity (RemotePlayer or mob, on a real bed or a
/// non-bed block) adopts the vanilla 26.3 bed view (`bed_yaw - 180` or `0`, pitch `0`, `+0.15`)
/// and stays visible in first person (`LevelExtractor` does not hide a sleeping camera entity).
#[test]
#[ignore = "run by tools/test_camera_parity.py"]
fn sleeping_spectated_living_entity_bed_view_and_visibility_26_3() {
    run_sleeping_spectated_living_entity_bed_view_and_visibility_26_3();
}

#[allow(unused_imports)]
pub(in crate::game) use crate::game::client_side::remote_client::tests::camera_parity_red_tests::run_sleeping_spectated_living_entity_bed_view_and_visibility_26_3;

/// Parity #3: `Camera.tickFov` reads the camera player's own `getFieldOfViewModifier` (movement
/// speed, bow draw, offhand spyglass, `fovEffectScale`, first- vs third-person) and isolates the
/// camera from the local player's death/speed/scoping while still applying the camera entity's own
/// death squeeze.
#[test]
#[ignore = "run by tools/test_camera_parity.py"]
fn camera_player_fov_modifier_and_death_isolation_26_3() {
    run_camera_player_fov_modifier_and_death_isolation_26_3();
}

#[allow(unused_imports)]
pub(in crate::game) use crate::game::client_side::remote_client::tests::camera_parity_red_tests::run_camera_player_fov_modifier_and_death_isolation_26_3;

/// Parity #33: `Minecraft.pick` / `ProjectileUtil.getEntityHitResult` excludes the camera entity
/// itself for every pickable entity family (boats, item frames, minecarts, TNT, interactions,
/// end crystals, paintings, armor stands, mobs) and shoots from the camera entity's exact eye and
/// view direction so a target ahead is picked instead of self.
#[test]
#[ignore = "run by tools/test_camera_parity.py"]
fn camera_entity_excluded_from_pick_for_all_pickable_kinds_26_3() {
    run_camera_entity_excluded_from_pick_for_all_pickable_kinds_26_3();
}

pub(in crate::game) fn run_camera_entity_excluded_from_pick_for_all_pickable_kinds_26_3() {
    let witness = parity_witness();
    let cases = witness["camera_pick_exclusion"]["cases"]
        .as_array()
        .unwrap();
    assert_eq!(cases.len(), 11);

    for (idx, tc) in cases.iter().enumerate() {
        let name = tc["name"].as_str().unwrap();
        let kind = tc["kind"].as_str().unwrap();
        let data = tc["data"].as_i64().unwrap() as i32;
        let spawn_pos = vec3_from_bits(&tc["spawn_pos_bits"]);
        let target_feet = vec3_from_bits(&tc["target_feet_bits"]);
        let cam_id = 700 + idx as i32;

        with_client_game(|game| {
            let _server = setup_remote_overworld(game, 1, [50.5, 64.0, 50.5], true);
            game.handle_packet(C::GameEvent(play::GameEvent {
                event: 3,
                param: 3.0,
            }))
            .unwrap();
            game.entity_reach = 4.5;

            add_entity_with_data(game, cam_id, kind, spawn_pos, 0.0, 0.0, data);
            if kind == "minecraft:interaction" {
                sync_interaction_size(game, cam_id, 1.0, 1.0);
            }
            game.handle_packet(C::SetCamera(play::SetCamera { camera_id: cam_id }))
                .unwrap();

            let snap = game
                .spectated_camera_snapshot()
                .unwrap_or_else(|| panic!("{name}: camera snapshot missing"));
            assert_eq!(
                snap.position.map(f64::to_bits),
                vec3_bits_expected(&tc["camera_pos_bits"]),
                "{name}: camera_pos_bits"
            );
            assert_eq!(
                snap.eye_height.to_bits(),
                hex_f32(&tc["camera_eye_height_bits"]),
                "{name}: camera_eye_height_bits"
            );
            assert_eq!(
                snap.y_rot.to_bits(),
                hex_f32(&tc["camera_yRot_bits"]),
                "{name}: camera_yRot_bits"
            );
            assert_eq!(
                snap.x_rot.to_bits(),
                hex_f32(&tc["camera_xRot_bits"]),
                "{name}: camera_xRot_bits"
            );

            assert!(
                game.native_lifecycle.epoch().is_some(),
                "{name}: world epoch must be live for client pick"
            );
            game.refresh_client_pick(1.0);
            assert_eq!(
                game.crosshair_entity_id(),
                None,
                "{name}: camera entity must be excluded from crosshair_entity_id when alone"
            );
            assert_eq!(
                game.spectator_click_target(),
                None,
                "{name}: camera entity must be excluded from spectator_click_target when alone"
            );

            add_entity_with_data(game, 900, "minecraft:creeper", target_feet, 0.0, 0.0, 0);
            game.refresh_client_pick(1.0);
            let want = tc["hit_with_target_id"].as_i64().unwrap() as i32;
            assert_eq!(
                game.crosshair_entity_id(),
                Some(want),
                "{name}: crosshair_entity_id must hit target past excluded camera entity"
            );
            assert_eq!(
                game.spectator_click_target(),
                Some(want),
                "{name}: spectator_click_target must hit target past excluded camera entity"
            );
        });
    }
}

/// Addendum (a): `LevelExtractor.extractVisibleEntities` never draws `LocalPlayer` unless it is
/// the camera entity (and in third person or sleeping), and hides the camera entity in first
/// person whatever its kind (body, flames, shadows, name tag).
#[test]
#[ignore = "run by tools/test_camera_parity.py"]
fn camera_entity_hidden_in_first_person_for_all_kinds_and_local_player_hidden_26_3() {
    run_camera_entity_hidden_in_first_person_for_all_kinds_and_local_player_hidden_26_3();
}

#[allow(unused_imports)]
pub(in crate::game) use crate::game::client_side::remote_client::tests::camera_parity_red_tests::run_camera_entity_hidden_in_first_person_for_all_kinds_and_local_player_hidden_26_3;

/// Addendum (b): a spectated entity (mob or non-living) riding a living vehicle — including a
/// scaled ArmorStand or scale-sanitized HappyGhast — uses the vehicle's sanitized scale and
/// camera distance in third person, and a scaled ArmorStand camera uses its own scale & distance.
#[test]
#[ignore = "run by tools/test_camera_parity.py"]
fn spectated_rider_mount_attributes_26_3() {
    run_spectated_rider_mount_attributes_26_3();
}

pub(in crate::game) fn run_spectated_rider_mount_attributes_26_3() {
    use {sunrise_client::camera::Camera, sunrise_level::camera_math::CameraType};

    let witness = parity_witness();
    let cases = witness["spectated_rider_mount_attributes"]["cases"]
        .as_array()
        .unwrap();
    let mut failures = Vec::new();

    with_client_game(|game| {
        let _server = setup_remote_overworld(game, 1, [0.5, 64.0, 0.5], true);
        let scale_id = game
            .motion
            .attributes
            .id("minecraft:scale")
            .unwrap()
            .registry_id();
        let dist_id = game
            .motion
            .attributes
            .id("minecraft:camera_distance")
            .unwrap()
            .registry_id();

        let mut next_id = 950i32;
        for tc in cases {
            let name = tc["name"].as_str().unwrap();
            let rider_kind = tc["rider_kind"].as_str().unwrap();
            let rider_scale = f64::from_bits(hex_f64(&tc["rider_scale_bits"]));
            let rider_dist = f64::from_bits(hex_f64(&tc["rider_distance_bits"]));
            let vehicle_kind = tc["vehicle_kind"].as_str();
            let vehicle_scale = f64::from_bits(hex_f64(&tc["vehicle_scale_bits"]));
            let vehicle_dist = f64::from_bits(hex_f64(&tc["vehicle_distance_bits"]));

            let rider_id = next_id;
            next_id += 1;
            let at = [10.25, 64.0, -3.75];
            add_entity_with_data(game, rider_id, rider_kind, at, 45.0, 22.5, 0);
            if rider_kind != "minecraft:oak_boat" {
                game.handle_packet(C::UpdateAttributes(play::UpdateAttributes {
                    id: rider_id,
                    values: vec![
                        play::AttributeSnapshot {
                            attribute: scale_id,
                            base: rider_scale,
                            modifiers: vec![],
                        },
                        play::AttributeSnapshot {
                            attribute: dist_id,
                            base: rider_dist,
                            modifiers: vec![],
                        },
                    ],
                }))
                .unwrap();
            }

            let mut vehicle_id = None;
            if let Some(vk) = vehicle_kind {
                let vid = next_id;
                next_id += 1;
                vehicle_id = Some(vid);
                add_entity_with_data(game, vid, vk, at, 0.0, 0.0, 0);
                game.handle_packet(C::UpdateAttributes(play::UpdateAttributes {
                    id: vid,
                    values: vec![
                        play::AttributeSnapshot {
                            attribute: scale_id,
                            base: vehicle_scale,
                            modifiers: vec![],
                        },
                        play::AttributeSnapshot {
                            attribute: dist_id,
                            base: vehicle_dist,
                            modifiers: vec![],
                        },
                    ],
                }))
                .unwrap();
                game.handle_packet(C::SetPassengers(play::SetPassengers {
                    vehicle: vid,
                    passengers: vec![rider_id],
                }))
                .unwrap();
            }
            game.refresh_remote_attributes();
            game.handle_packet(C::SetCamera(play::SetCamera {
                camera_id: rider_id,
            }))
            .unwrap();

            // Pin exact unquantized yaw/pitch to match the oracle's (45.0, 22.5).
            let pos = Vec3::new(at[0], at[1], at[2]);
            if let Some(m) = game.mobs.iter_mut().find(|m| m.id == rider_id as u32) {
                (m.position, m.previous) = (pos, pos);
                (m.yaw, m.previous_yaw, m.head_yaw, m.previous_head_yaw) = (45.0, 45.0, 45.0, 45.0);
                (m.head_pitch, m.previous_head_pitch) = (22.5, 22.5);
            }
            if let Some(a) = game
                .armor_stands
                .iter_mut()
                .find(|a| a.id == rider_id as u32)
            {
                (a.position, a.previous) = (pos, pos);
                (a.yaw, a.previous_yaw, a.pitch) = (45.0, 45.0, 22.5);
            }
            game.boats.for_each_mut(|b| {
                if b.id == rider_id as u32 {
                    (b.position, b.previous, b.yaw, b.previous_yaw, b.pitch) =
                        (pos, pos, 45.0, 45.0, 22.5);
                }
            });

            let snap = game
                .spectated_camera_snapshot()
                .unwrap_or_else(|| panic!("{name}: missing spectated camera snapshot"));
            let want_eye = hex_f32(&tc["rider_eye_height_bits"]);
            if snap.eye_height.to_bits() != want_eye {
                failures.push(format!(
                    "{name}: eye_height {:08x} ({}) want {:08x} ({})",
                    snap.eye_height.to_bits(),
                    snap.eye_height,
                    want_eye,
                    f32::from_bits(want_eye)
                ));
            }

            for (ct_name, ct) in [
                ("first_person", CameraType::FirstPerson),
                ("third_person_back", CameraType::ThirdPersonBack),
            ] {
                let mut cam = Camera::default();
                for _ in 0..40 {
                    cam.tick(Some(snap.eye_height), 1.0);
                }
                cam.align_with_entity(&snap, 1.0, ct, |_, _| None);
                let want = &tc[ct_name];
                let want_pos = vec3_bits_expected(&want["pos_bits"]);
                let got_pos = cam.position.map(f64::to_bits);
                if got_pos != want_pos {
                    failures.push(format!(
                        "{name} {ct_name}: pos {:?} want {:?}",
                        cam.position,
                        want_pos.map(f64::from_bits)
                    ));
                }
                if cam.x_rot.to_bits() != hex_f32(&want["xRot_bits"])
                    || cam.y_rot.to_bits() != hex_f32(&want["yRot_bits"])
                    || cam.detached != want["detached"].as_bool().unwrap()
                {
                    failures.push(format!(
                        "{name} {ct_name}: rot/detached mismatch ({}, {}, {})",
                        cam.x_rot, cam.y_rot, cam.detached
                    ));
                }
            }

            let mut to_remove = vec![rider_id];
            if let Some(vid) = vehicle_id {
                to_remove.push(vid);
            }
            game.handle_packet(C::RemoveEntities(play::RemoveEntities { ids: to_remove }))
                .unwrap();
        }
    });

    assert!(
        failures.is_empty(),
        "{} mount attribute mismatches against real 26.3:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Vanilla 26.3 `ClientLevel.doAddParticle`, `ClientLevel.tickWeatherEffects`, and
/// `AtmosphericFogEnvironment.updateRainFogState` unconditionally read `mainCamera().position()` /
/// `blockPosition()` with no `isInitialized()` fallback: `Vec3.ZERO` before the first frame, the
/// aligned camera position after the first frame, and the last aligned position after `Camera.reset()`.
#[test]
#[ignore = "run by tools/test_camera_parity.py"]
fn particle_camera_before_first_frame_and_after_reset_26_3() {
    run_particle_camera_before_first_frame_and_after_reset_26_3();
}

#[allow(unused_imports)]
pub(in crate::game) use crate::game::client_side::remote_client::tests::camera_parity_red_tests::run_particle_camera_before_first_frame_and_after_reset_26_3;

/// Review follow-up parity checks against vanilla 26.3 (`review_followup` witness).
#[test]
#[ignore = "run by tools/test_camera_parity.py"]
fn review_followup_camera_parity_26_3() {
    run_review_followup_camera_parity_26_3();
}

#[allow(unused_imports)]
pub(in crate::game) use crate::game::client_side::remote_client::tests::camera_parity_red_tests::run_review_followup_camera_parity_26_3;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;
