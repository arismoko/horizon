use crate::game::remote::tests::camera_parity_red_tests::add_entity;
use crate::game::remote::tests::camera_parity_red_tests::add_entity_with_data;
use crate::game::remote::tests::camera_parity_red_tests::hex_f32;
use crate::game::remote::tests::camera_parity_red_tests::hex_f64;
use crate::game::remote::tests::camera_parity_red_tests::make_chunk;
use crate::game::remote::tests::camera_parity_red_tests::parity_witness;
use crate::game::remote::tests::camera_parity_red_tests::sync_interaction_size;
use crate::game::remote::tests::camera_parity_red_tests::vec3_bits_expected;
use std::sync::Arc;
use sunrise_client::{
    debug_entries::Status, debug_overlay::Displayer, level_loading::Reason,
    options_file::V as OptV, screen_effects::Blend,
};
use sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::world_biomes;
use sunrise_items::items::ItemStack;
use sunrise_level::camera_math::CameraType;
use sunrise_level::{effects::EffectInstance, environment::Sky};
use sunrise_protocol::{
    buf::pack_degrees,
    connection::Pipe,
    entity_data::{Value as EntityDataValue, serializer},
    item::Stack as WireStack,
    packets::play::{self, Clientbound as C, SpawnInfo},
};
use sunrise_sim::motion::PlayerMotion;
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycle,
};

pub(in crate::game) fn run_any_tracked_entity_resolves_as_camera_26_3() {
    let witness = parity_witness();
    let records = witness["entity_cameras"]["records"].as_array().unwrap();
    with_client_game(|game| {
        let _server = setup_remote_overworld(game, 1, [0.5, 64.0, 0.5], true);
        let mut mismatches = Vec::new();
        for (idx, row) in records.iter().enumerate() {
            let label = row["label"].as_str().unwrap();
            let kind = row["type"].as_str().unwrap();
            let synced = row["synced_genpvp"].as_bool().unwrap();
            let id = 100 + idx as i32;
            add_entity(game, id, kind, [12.25, 70.5, -4.75], 45.0, 15.0);
            if synced {
                sync_interaction_size(game, id, 0.1, 1.9059);
            }
            game.handle_packet(C::SetCamera(play::SetCamera { camera_id: id }))
                .unwrap();

            let Some(snap) = game.spectated_camera_snapshot() else {
                mismatches.push(format!("{label}: spectated_camera_snapshot returned None"));
                continue;
            };
            if snap.eye_height.to_bits() != hex_f32(&row["eye_height_bits"]) {
                mismatches.push(format!(
                    "{label}: eye_height {:08x} != vanilla {}",
                    snap.eye_height.to_bits(),
                    row["eye_height_bits"]
                ));
            }
            if snap.living.is_some() != row["is_living"].as_bool().unwrap() {
                mismatches.push(format!(
                    "{label}: is_living {} != vanilla {}",
                    snap.living.is_some(),
                    row["is_living"]
                ));
            }

            game.client.view = Default::default();
            game.tick_fov();
            let (pos1, y1, p1) = game.camera_view(1.0).expect("camera view");
            if pos1.to_array().map(f64::to_bits)
                != vec3_bits_expected(&row["camera_pos_after_1_tick_bits"])
            {
                mismatches.push(format!(
                    "{label}: pos1 {:?} != vanilla {}",
                    pos1.to_array().map(|v| format!("{:016x}", v.to_bits())),
                    row["camera_pos_after_1_tick_bits"]
                ));
            }
            if y1.to_bits() != hex_f32(&row["camera_yRot_bits"])
                || p1.to_bits() != hex_f32(&row["camera_xRot_bits"])
            {
                mismatches.push(format!(
                    "{label}: rot ({:08x}, {:08x}) != vanilla ({}, {})",
                    y1.to_bits(),
                    p1.to_bits(),
                    row["camera_yRot_bits"],
                    row["camera_xRot_bits"]
                ));
            }
            let fw = game.client.view.forwards.map(f32::to_bits);
            let want_fw = [
                hex_f32(&row["camera_forwards_bits"][0]),
                hex_f32(&row["camera_forwards_bits"][1]),
                hex_f32(&row["camera_forwards_bits"][2]),
            ];
            if fw != want_fw {
                mismatches.push(format!("{label}: forwards {fw:08x?} != {want_fw:08x?}"));
            }

            for _ in 0..40 {
                game.tick_fov();
            }
            let (pos_settled, _, _) = game.camera_view(1.0).expect("settled camera view");
            if pos_settled.to_array().map(f64::to_bits)
                != vec3_bits_expected(&row["camera_pos_settled_bits"])
            {
                mismatches.push(format!(
                    "{label}: settled pos {:?} != vanilla {}",
                    pos_settled
                        .to_array()
                        .map(|v| format!("{:016x}", v.to_bits())),
                    row["camera_pos_settled_bits"]
                ));
            }
        }
        assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    });
}

pub(in crate::game) fn run_camera_consumers_bob_fog_dark_disc_and_vignette_26_3() {
    let witness = parity_witness();
    let consumers = &witness["camera_consumers"];
    with_client_game(|game| {
        let _server = setup_remote_overworld(game, 1, [0.5, 64.0, 0.5], false);

        // Give the local player hurt + walk bob, Spectator mode, and fog-altering potions.
        {
            let p = game.player.as_mut().unwrap();
            p.health.hurt_animation_time = 6;
            p.health.hurt.hurt_duration = 10;
            p.health.hurt_dir = 45.0;
        }
        game.client.avatar.walk_dist = 1.75;
        game.client.avatar.walk_dist_o = 1.75;
        game.client.avatar.bob = 0.08;
        game.client.avatar.bob_o = 0.04;
        game.client.settings.view_bobbing = true;
        game.set_local_mode(sunrise_command::args::GameMode::Spectator);
        let profile = game.client.remote.as_ref().unwrap().profile;
        game.handle_packet(C::PlayerInfoUpdate(play::PlayerInfoUpdate {
            actions: 0x01 | 0x04 | 0x08,
            entries: vec![play::PlayerInfo {
                uuid: profile,
                name: Some("CameraParityTest".into()),
                game_mode: Some(3),
                listed: Some(true),
                ..play::PlayerInfo::default()
            }],
        }))
        .unwrap();
        game.potions
            .effects
            .add(EffectInstance::new("blindness", 100, 0));
        game.potions
            .effects
            .add(EffectInstance::new("fire_resistance", 100, 0));
        game.potions
            .effects
            .add(EffectInstance::new("night_vision", 300, 0));
        game.client.fog.water_vision_time = 0;

        add_entity(
            game,
            2,
            "minecraft:interaction",
            [0.5, 300.0, 0.5],
            0.0,
            0.0,
        );
        sync_interaction_size(game, 2, 0.1, 1.9059);
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 2 }))
            .unwrap();
        game.client.view = Default::default();
        for _ in 0..40 {
            game.tick_fov();
        }
        let (cam, _, _) = game.camera_view(1.0).unwrap();

        // 1. Bob: non-living Interaction camera produces identity matrix even when player is hurt/walking.
        let bob_cols: Vec<u32> = game
            .bob_matrix(0.5)
            .to_cols_array()
            .into_iter()
            .map(f32::to_bits)
            .collect();
        let want_bob: Vec<u32> = consumers["bob"]["interaction"]["matrix_bits"]
            .as_array()
            .unwrap()
            .iter()
            .map(hex_f32)
            .collect();
        assert_eq!(bob_cols, want_bob, "interaction camera must have no bob");

        // 2. Fog: Interaction is neither Spectator nor LivingEntity, and is not LocalPlayer.
        game.world
            .chunks
            .insert((0, 0), Arc::new(make_chunk(0, 0, 0, 0xF0)));
        let sky = Sky {
            fog_start: 16.0,
            fog_end: 128.0,
            cloud_fog_end: 2048.0,
            ..Sky::default()
        };
        let want_fog = &consumers["fog"]["interaction"];

        let lava = game
            .pack
            .assets
            .registry
            .default_state("minecraft:lava")
            .unwrap();
        game.world.set(0, 301, 0, lava).unwrap();
        let lava_fog = game.frame_fog(cam, &sky, 0, [0.0; 3]);
        assert_eq!(
            lava_fog.environmental_start.to_bits(),
            hex_f32(&want_fog["lava_start_bits"]),
            "interaction camera lava fog start"
        );
        assert_eq!(
            lava_fog.environmental_end.to_bits(),
            hex_f32(&want_fog["lava_end_bits"]),
            "interaction camera lava fog end"
        );

        let snow = game
            .pack
            .assets
            .registry
            .default_state("minecraft:powder_snow")
            .unwrap();
        game.world.set(0, 301, 0, snow).unwrap();
        let snow_fog = game.frame_fog(cam, &sky, 0, [0.0; 3]);
        assert_eq!(
            snow_fog.environmental_start.to_bits(),
            hex_f32(&want_fog["powder_snow_start_bits"]),
            "interaction camera powder snow fog start"
        );
        assert_eq!(
            snow_fog.environmental_end.to_bits(),
            hex_f32(&want_fog["powder_snow_end_bits"]),
            "interaction camera powder snow fog end"
        );

        let water = game
            .pack
            .assets
            .registry
            .default_state("minecraft:water")
            .unwrap();
        game.world.set(0, 301, 0, water).unwrap();
        let water_fog = game.frame_fog(cam, &sky, 0, [0.1, 0.2, 0.6]);
        assert_eq!(
            water_fog.environmental_end.to_bits(),
            hex_f32(&want_fog["water_end_bits"]),
            "non-local camera must not apply LocalPlayer waterVision scaling"
        );

        let air = game
            .pack
            .assets
            .registry
            .default_state("minecraft:air")
            .unwrap();
        game.world.set(0, 301, 0, air).unwrap();
        let air_fog = game.frame_fog(cam, &sky, 0, [0.0; 3]);
        assert!(
            air_fog.environmental_end > 10.0,
            "local player blindness must not clamp Interaction atmospheric fog: {}",
            air_fog.environmental_end
        );

        // 3. Dark disc reads local player eye Y, NOT camera Y.
        game.player.as_mut().unwrap().position = Vec3::new(0.5, 50.0, 0.5);
        assert_eq!(
            game.dark_disc(cam),
            consumers["dark_disc"]["player_below_horizon_dry"]
                .as_bool()
                .unwrap(),
            "player below horizon with camera at y=301.62 must still render dark disc"
        );
        game.player.as_mut().unwrap().position = Vec3::new(0.5, 100.0, 0.5);
        assert_eq!(
            game.dark_disc(glam::DVec3::new(0.5, 50.0, 0.5)),
            consumers["dark_disc"]["player_above_horizon_dry"]
                .as_bool()
                .unwrap(),
            "player above horizon with camera at y=50 must NOT render dark disc"
        );

        // 4. Vignette brightness & border warning follow camera entity, and F1 hides GUI overlays.
        game.player.as_mut().unwrap().position = Vec3::new(0.5, 64.0, 0.5);
        game.world.day_time = 6000;
        game.world
            .chunks
            .insert((5, 0), Arc::new(make_chunk(5, 0, 0, 0x00)));
        add_entity(
            game,
            3,
            "minecraft:interaction",
            [95.5, 300.0, 0.5],
            0.0,
            0.0,
        );
        sync_interaction_size(game, 3, 0.1, 1.9059);
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 3 }))
            .unwrap();
        game.client.screen_fx.vignette_brightness = 0.0;
        game.tick_vignette();
        assert_eq!(
            game.client.screen_fx.vignette_brightness.to_bits(),
            hex_f32(&consumers["vignette"]["interaction_vignette_after_1_tick_bits"]),
            "vignette must sample light at camera entity eye"
        );

        game.border.set_center([0.0, 0.0]).unwrap();
        game.border.set_size(200.0).unwrap();
        game.border.warning_distance = 10;
        game.client.settings.vignette = true;
        game.client.chat_ui.hide_gui = false;
        let hand_proj = sunrise_client::camera::hand_projection(70.0, 1280.0, 800.0, 512.0);
        let quads_shown = game.screen_quads(1.0, [1280, 800], 1.0, hand_proj);
        let vignette_quad = quads_shown
            .iter()
            .find(|q| q.blend == Blend::Vignette)
            .expect("vignette quad expected when HUD visible");
        let color = vignette_quad.vertices[0].color;
        assert!(
            color[1] > color[0] + 0.1,
            "border warning at camera x=95.5 must tint vignette green/blue channels (subtracting into red): {color:?}"
        );

        game.client.chat_ui.hide_gui = true;
        let quads_hidden = game.screen_quads(1.0, [1280, 800], 1.0, hand_proj);
        assert!(
            quads_hidden.iter().all(|q| q.blend != Blend::Vignette),
            "F1 hide_gui must suppress Hud vignette overlay"
        );
    });
}

pub(in crate::game) fn run_camera_consumers_streaming_particles_debug_and_spectator_crosshair_26_3()
{
    let witness = parity_witness();
    let consumers = &witness["camera_consumers"];
    let stream = &consumers["streaming_and_particles"];
    let debug = &consumers["debug"];
    let cross = &consumers["spectator_crosshair"];
    let want_chunk = (
        stream["camera_chunk"][0].as_i64().unwrap() as i32,
        stream["camera_chunk"][1].as_i64().unwrap() as i32,
    );
    let want_block_y = stream["camera_block_y"].as_i64().unwrap() as i32;
    let want_cam_pos_bits = vec3_bits_expected(&stream["camera_pos_bits"]);
    let want_xyz_line = debug["position_lines"][0].as_str().unwrap();
    let want_biome_line = debug["biome_line"].as_str().unwrap();
    let want_light_line = debug["light_line"].as_str().unwrap();
    let want_min_x = hex_f64(&debug["chunk_borders"]["min_x_bits"]);
    let want_max_x = hex_f64(&debug["chunk_borders"]["max_x_bits"]);
    let want_min_z = hex_f64(&debug["chunk_borders"]["min_z_bits"]);
    let want_max_z = hex_f64(&debug["chunk_borders"]["max_z_bits"]);
    let want_player_box = debug["entity_hitboxes"]["player_box_emitted_in_first_person"]
        .as_bool()
        .unwrap();
    let want_cam_box = debug["entity_hitboxes"]["camera_interaction_box_emitted_in_first_person"]
        .as_bool()
        .unwrap();
    let want_cart_crosshair = cross["cart_only_crosshair"].as_bool().unwrap();
    let want_occluded_crosshair = cross["with_interaction_crosshair"].as_bool().unwrap();

    with_client_game(|game| {
        let _server = setup_remote_overworld(game, 1, [0.5, 64.0, 0.5], true);
        let plains = *world_biomes!(game).index.get("minecraft:plains").unwrap();
        let desert = *world_biomes!(game).index.get("minecraft:desert").unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(make_chunk(0, 0, plains, 0xF0)));
        game.world
            .chunks
            .insert((6, 6), Arc::new(make_chunk(6, 6, desert, 0x00)));

        add_entity(
            game,
            31,
            "minecraft:interaction",
            [96.5, 300.0, 96.5],
            0.0,
            0.0,
        );
        sync_interaction_size(game, 31, 0.1, 1.9059);
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 31 }))
            .unwrap();
        game.client.view = Default::default();
        for _ in 0..40 {
            game.tick_fov();
        }
        game.camera_view(1.0);

        // 1. Streaming center follows camera entity.
        game.client_update_world();
        assert_eq!(
            game.client.streaming.center, want_chunk,
            "streaming.center must follow camera"
        );
        assert_eq!(
            game.client.streaming.center_y, want_block_y,
            "streaming.center_y must follow camera"
        );

        // 2. Particle camera follows camera entity eye (exact 26.3 camera.position() bits).
        game.tick_particles();
        assert_eq!(
            game.client.particles.0.camera.map(f64::to_bits),
            want_cam_pos_bits,
            "particles.camera must match 26.3 camera.position() bits"
        );

        // 3. F3 debug entries report camera entity position, biome, and light.
        game.client.debug = true;
        game.client
            .debug_entries
            .0
            .set_status("player_position", Status::InOverlay);
        game.client
            .debug_entries
            .0
            .set_status("biome", Status::InOverlay);
        game.client
            .debug_entries
            .0
            .set_status("light_levels", Status::InOverlay);
        let mut displayer = Displayer::default();
        game.debug_entry_lines(&mut displayer, [1280, 800]);
        let (left, right) = displayer.columns(&[]);
        let all_lines: Vec<String> = left.into_iter().chain(right).collect();
        let joined = all_lines.join("\n");
        assert!(
            joined.contains(want_xyz_line),
            "F3 position must match 26.3 witness '{want_xyz_line}', got:\n{joined}"
        );
        assert!(
            joined.contains(want_biome_line),
            "F3 biome must match 26.3 witness '{want_biome_line}', got:\n{joined}"
        );
        assert!(
            joined.contains(want_light_line),
            "F3 light must match 26.3 witness '{want_light_line}', got:\n{joined}"
        );

        // 4. F3 gizmos: chunk_borders bounds match 26.3 ChunkBorderRenderer; hitboxes follow EntityHitboxDebugRenderer.
        game.client
            .debug_entries
            .0
            .set_status("chunk_borders", Status::AlwaysOn);
        game.client
            .debug_entries
            .0
            .set_status("entity_hitboxes", Status::Never);
        let border_gizmos = game.debug_gizmos(1.0, [96.5, 301.62, 96.5]);
        let (mut min_x, mut max_x) = (f64::INFINITY, f64::NEG_INFINITY);
        let (mut min_z, mut max_z) = (f64::INFINITY, f64::NEG_INFINITY);
        for l in &border_gizmos.lines {
            min_x = min_x.min(l.start[0]).min(l.end[0]);
            max_x = max_x.max(l.start[0]).max(l.end[0]);
            min_z = min_z.min(l.start[2]).min(l.end[2]);
            max_z = max_z.max(l.start[2]).max(l.end[2]);
        }
        assert_eq!(min_x.to_bits(), want_min_x, "chunk_borders min_x");
        assert_eq!(max_x.to_bits(), want_max_x, "chunk_borders max_x");
        assert_eq!(min_z.to_bits(), want_min_z, "chunk_borders min_z");
        assert_eq!(max_z.to_bits(), want_max_z, "chunk_borders max_z");

        game.client
            .debug_entries
            .0
            .set_status("chunk_borders", Status::Never);
        game.client
            .debug_entries
            .0
            .set_status("entity_hitboxes", Status::AlwaysOn);
        let hitbox_gizmos = game.debug_gizmos(1.0, [96.5, 301.62, 96.5]);
        let has_player_box = hitbox_gizmos.lines.iter().any(|l| {
            l.start[0] >= 0.0 && l.start[0] <= 1.0 && l.start[1] >= 64.0 && l.start[1] <= 66.0
        });
        assert_eq!(
            has_player_box, want_player_box,
            "local player hitbox in FirstPerson while spectating another entity"
        );
        let has_camera_interaction_box = hitbox_gizmos.lines.iter().any(|l| {
            l.start[0] >= 96.4 && l.start[0] <= 96.6 && l.start[1] >= 300.0 && l.start[1] <= 302.0
        });
        assert_eq!(
            has_camera_interaction_box, want_cam_box,
            "camera entity hitbox in FirstPerson"
        );

        // 5. Spectator crosshair: a nearer Interaction blocks a chest minecart behind it.
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 1 }))
            .unwrap();
        game.yaw = 0.0;
        game.pitch = 0.0;
        add_entity(
            game,
            40,
            "minecraft:chest_minecart",
            [0.5, 65.2, 3.5],
            0.0,
            0.0,
        );
        game.refresh_client_pick(1.0);
        assert_eq!(
            game.spectator_crosshair(),
            want_cart_crosshair,
            "unobstructed chest minecart spectator crosshair"
        );
        add_entity(
            game,
            41,
            "minecraft:interaction",
            [0.5, 65.0, 2.0],
            0.0,
            0.0,
        );
        game.refresh_client_pick(1.0);
        assert_eq!(
            game.spectator_crosshair(),
            want_occluded_crosshair,
            "nearer Interaction is not a MenuProvider and must occlude chest minecart"
        );
    });
}

pub(in crate::game) fn run_camera_entity_hidden_in_first_person_for_all_kinds_and_local_player_hidden_26_3()
 {
    use sunrise_core::nbt::Tag;
    use sunrise_level::camera_math::CameraType;

    let witness = parity_witness();
    let cases = witness["camera_visibility_all_kinds"]["cases"]
        .as_array()
        .unwrap();
    let camera_types = [
        ("third_person_back", CameraType::ThirdPersonBack),
        ("third_person_front", CameraType::ThirdPersonFront),
        ("first_person", CameraType::FirstPerson),
    ];
    let cam = glam::DVec3::new(5.5, 64.5, 8.5);
    let mut failures = Vec::new();
    let mut next_id = 800i32;

    for tc in cases {
        let name = tc["name"].as_str().unwrap();
        let kind = tc["kind"].as_str().unwrap();
        let data = tc["data"].as_i64().unwrap() as i32;
        let local_sleeping = tc["local_sleeping"].as_bool().unwrap();
        let camera_on_self = tc["camera_on_self"].as_bool().unwrap();

        with_client_game(|game| {
            let _server = setup_remote_overworld(game, 1, [0.5, 64.0, 0.5], false);
            // Displays build their render state on their first tick, which needs a loaded chunk.
            game.world
                .chunks
                .insert((0, 0), Arc::new(make_chunk(0, 0, 0, 0xF0)));
            game.client.view_frustum = None;
            game.client.settings.entity_shadows = false;
            game.client.settings.extras.own_name_tag = false;
            game.tick_player_model();
            game.sleeping = local_sleeping.then_some([0, 64, 0]);

            let cam_id = if camera_on_self {
                1
            } else {
                let id = next_id;
                next_id += 1;
                let stone_state = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap()
                    .0 as i32;
                let stone_item = game
                    .client
                    .remote
                    .as_ref()
                    .unwrap()
                    .builtin
                    .get("minecraft:item")
                    .unwrap()
                    .id("minecraft:stone")
                    .unwrap();
                let spawn_data = if kind == "minecraft:falling_block" {
                    stone_state
                } else {
                    data
                };
                add_entity_with_data(game, id, kind, [5.5, 64.0, 5.5], 0.0, 0.0, spawn_data);
                match kind {
                    "minecraft:item" => {
                        game.handle_packet(C::SetEntityData(play::SetEntityData {
                            id,
                            values: vec![(
                                8,
                                serializer::ITEM_STACK,
                                EntityDataValue::Item(Some(WireStack::new(stone_item, 1))),
                            )],
                        }))
                        .unwrap();
                    }
                    "minecraft:block_display" => {
                        game.handle_packet(C::SetEntityData(play::SetEntityData {
                            id,
                            values: vec![(
                                23,
                                serializer::BLOCK_STATE,
                                EntityDataValue::Id(stone_state),
                            )],
                        }))
                        .unwrap();
                        game.tick_text_displays(true);
                    }
                    "minecraft:item_display" => {
                        game.handle_packet(C::SetEntityData(play::SetEntityData {
                            id,
                            values: vec![(
                                23,
                                serializer::ITEM_STACK,
                                EntityDataValue::Item(Some(WireStack::new(stone_item, 1))),
                            )],
                        }))
                        .unwrap();
                        game.tick_text_displays(true);
                    }
                    "minecraft:text_display" => {
                        game.handle_packet(C::SetEntityData(play::SetEntityData {
                            id,
                            values: vec![(
                                23,
                                serializer::COMPONENT,
                                EntityDataValue::Text(Tag::String("HI".into())),
                            )],
                        }))
                        .unwrap();
                        game.tick_text_displays(true);
                    }
                    _ => {}
                }
                game.handle_packet(C::SetCamera(play::SetCamera { camera_id: id }))
                    .unwrap();
                id
            };

            for (ct_name, ct) in camera_types {
                let want_local = tc["views"][ct_name]["local_player_extracted"]
                    .as_bool()
                    .unwrap();
                let want_cam = tc["views"][ct_name]["camera_entity_extracted"]
                    .as_bool()
                    .unwrap();
                game.client.camera_type = ct;
                if game.player_model_shown() != want_local {
                    failures.push(format!(
                        "{name} {ct_name}: player_model_shown={} want {want_local}",
                        game.player_model_shown()
                    ));
                }
                let mut additive = Vec::new();
                let (verts, _, _, _) =
                    game.entity_geometry(cam, 1.0, None, Vec::new(), &mut additive);
                let has_local_run = game.client.entity_runs.iter().any(|r| {
                    r.family == sunrise_client_gameplay::game::client_side::entity_client::DrawFamily::LocalPlayer
                });
                if has_local_run != want_local {
                    failures.push(format!(
                        "{name} {ct_name}: LocalPlayer run present={has_local_run} want {want_local}"
                    ));
                }
                if !camera_on_self {
                    let drew = !verts.is_empty()
                        || !additive.is_empty()
                        || !game.client.see_through_vertices.is_empty();
                    if drew != want_cam {
                        failures.push(format!(
                            "{name} ({kind}, id={cam_id}) {ct_name}: drew={drew} (verts={}, additive={}, see_through={}) want {want_cam}",
                            verts.len(),
                            additive.len(),
                            game.client.see_through_vertices.len(),
                        ));
                    }
                }
            }

            // Local player own_name_tag must never show while camera is on another entity.
            if !camera_on_self && name == "creeper" {
                game.client.camera_type = CameraType::ThirdPersonBack;
                game.client.settings.extras.own_name_tag = true;
                let own_name = game.own_profile_name();
                let tags = game.shown_name_tags(glam::DVec3::new(0.5, 65.5, 4.5));
                if tags.iter().any(|t| t == &own_name) {
                    failures.push(format!(
                        "{name}: local player own_name_tag shown while spectating another entity: {tags:?}"
                    ));
                }
            }
        });
    }

    // Flames, shadows, and armor-stand name tags must also be suppressed when camera is hidden in first person.
    with_client_game(|game| {
        let _server = setup_remote_overworld(game, 1, [0.5, 64.0, 0.5], false);
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(make_chunk(0, 0, 0, 0xF0)));
        game.world.set(5, 63, 5, stone).unwrap();
        game.client.view_frustum = None;
        game.client.settings.entity_shadows = true;
        game.client.settings.extras.own_name_tag = false;
        game.tick_player_model();

        let stone_item = game
            .client
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:item")
            .unwrap()
            .id("minecraft:stone")
            .unwrap();

        for (label, kind, id) in [
            ("burning_shadow_creeper", "minecraft:creeper", 920),
            ("burning_shadow_item", "minecraft:item", 921),
            ("burning_named_stand", "minecraft:armor_stand", 922),
        ] {
            add_entity_with_data(game, id, kind, [5.5, 64.0, 5.5], 0.0, 0.0, 0);
            match kind {
                "minecraft:creeper" => {
                    game.mobs
                        .iter_mut()
                        .find(|m| m.id == id as u32)
                        .unwrap()
                        .fire_ticks = 40;
                }
                "minecraft:item" => {
                    game.handle_packet(C::SetEntityData(play::SetEntityData {
                        id,
                        values: vec![
                            (0, serializer::BYTE, EntityDataValue::Byte(1)),
                            (
                                8,
                                serializer::ITEM_STACK,
                                EntityDataValue::Item(Some(WireStack::new(stone_item, 1))),
                            ),
                        ],
                    }))
                    .unwrap();
                    if let Some(i) = game.dropped.iter().position(|d| d.id == id as u32) {
                        game.dropped.update(i, |d| d.fire_ticks = 40);
                    }
                }
                "minecraft:armor_stand" => {
                    game.handle_packet(C::SetEntityData(play::SetEntityData {
                        id,
                        values: vec![
                            (0, serializer::BYTE, EntityDataValue::Byte(1)),
                            (
                                2,
                                serializer::OPTIONAL_COMPONENT,
                                EntityDataValue::OptText(Some(Tag::String("StandTag".into()))),
                            ),
                            (3, serializer::BOOLEAN, EntityDataValue::Bool(true)),
                        ],
                    }))
                    .unwrap();
                }
                _ => {}
            }
            game.handle_packet(C::SetCamera(play::SetCamera { camera_id: id }))
                .unwrap();

            game.client.camera_type = CameraType::ThirdPersonBack;
            let mut add_tp = Vec::new();
            let (tp_verts, _, _, _) = game.entity_geometry(cam, 1.0, None, Vec::new(), &mut add_tp);
            if tp_verts.is_empty() && game.client.see_through_vertices.is_empty() {
                failures.push(format!("{label}: third person drew nothing"));
            }

            game.client.camera_type = CameraType::FirstPerson;
            let mut add_fp = Vec::new();
            let (fp_verts, _, _, _) = game.entity_geometry(cam, 1.0, None, Vec::new(), &mut add_fp);
            if !fp_verts.is_empty()
                || !add_fp.is_empty()
                || !game.client.see_through_vertices.is_empty()
            {
                failures.push(format!(
                    "{label}: first person leaked geometry (verts={}, additive={}, see_through={})",
                    fp_verts.len(),
                    add_fp.len(),
                    game.client.see_through_vertices.len(),
                ));
            }

            game.handle_packet(C::RemoveEntities(play::RemoveEntities { ids: vec![id] }))
                .unwrap();
        }
    });

    assert!(
        failures.is_empty(),
        "{} visibility mismatches against real 26.3 LevelExtractor:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

pub(in crate::game) fn run_camera_player_fov_modifier_and_death_isolation_26_3() {
    let witness = parity_witness();
    let cases = witness["camera_player_fov_modifier"]["cases"]
        .as_array()
        .unwrap();
    let find_case = |name: &str| {
        cases
            .iter()
            .find(|c| c["name"] == name)
            .unwrap_or_else(|| panic!("missing fov case {name}"))
    };
    let item_id = |game: &sunrise_client_gameplay::game::ClientGame<'_>, name: &str| {
        game.client
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:item")
            .unwrap()
            .id(name)
            .unwrap()
    };
    let speed_attr_id = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
        game.motion
            .attributes
            .id("minecraft:movement_speed")
            .unwrap()
            .registry_id()
    };

    // Case 1: sprinting speed (base 0.1 + 0.3 ADD_MULTIPLIED_TOTAL = 0.13), fovEffectScale = 1.0
    {
        let tc = find_case("sprinting_speed_scale_1_0");
        with_client_game(|game| {
            let _server = setup_remote_overworld(game, 1, [0.5, 64.0, 0.5], true);
            game.client
                .settings
                .file
                .set("fovEffectScale", OptV::Double(1.0));
            add_entity_with_data(game, 20, "minecraft:player", [5.5, 64.0, 5.5], 0.0, 0.0, 0);
            let attr = speed_attr_id(game);
            game.handle_packet(C::UpdateAttributes(play::UpdateAttributes {
                id: 20,
                values: vec![play::AttributeSnapshot {
                    attribute: attr,
                    base: 0.1,
                    modifiers: vec![play::AttributeModifier {
                        id: "minecraft:sprinting".into(),
                        amount: 0.3,
                        operation: 2,
                    }],
                }],
            }))
            .unwrap();
            game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 20 }))
                .unwrap();
            game.client.view = Default::default();
            game.tick_fov();
            let cam_pos = [5.5, 65.62, 5.5];
            assert_eq!(
                game.fov_degrees(1.0, cam_pos).to_bits(),
                hex_f32(&tc["fov_after_1_tick_p1_bits"]),
                "sprinting_speed_scale_1_0: 1-tick p=1.0 fov"
            );
            assert_eq!(
                game.fov_degrees(0.5, cam_pos).to_bits(),
                hex_f32(&tc["fov_after_1_tick_p05_bits"]),
                "sprinting_speed_scale_1_0: 1-tick p=0.5 fov"
            );
            assert_eq!(
                game.hud_fov_degrees(1.0, cam_pos).to_bits(),
                hex_f32(&tc["hud_after_1_tick_p1_bits"]),
                "sprinting_speed_scale_1_0: 1-tick hud fov"
            );
            for _ in 0..39 {
                game.tick_fov();
            }
            assert_eq!(
                game.fov_degrees(1.0, cam_pos).to_bits(),
                hex_f32(&tc["fov_settled_bits"]),
                "sprinting_speed_scale_1_0: settled fov"
            );
            assert_eq!(
                game.hud_fov_degrees(1.0, cam_pos).to_bits(),
                hex_f32(&tc["hud_settled_bits"]),
                "sprinting_speed_scale_1_0: settled hud fov"
            );
        });
    }

    // Case 2: speed = 0.2, fovEffectScale = 0.5
    {
        let tc = find_case("speed_0_2_scale_0_5");
        with_client_game(|game| {
            let _server = setup_remote_overworld(game, 1, [0.5, 64.0, 0.5], true);
            game.client
                .settings
                .file
                .set("fovEffectScale", OptV::Double(0.5));
            add_entity_with_data(game, 21, "minecraft:player", [5.5, 64.0, 5.5], 0.0, 0.0, 0);
            let attr = speed_attr_id(game);
            game.handle_packet(C::UpdateAttributes(play::UpdateAttributes {
                id: 21,
                values: vec![play::AttributeSnapshot {
                    attribute: attr,
                    base: 0.2,
                    modifiers: vec![],
                }],
            }))
            .unwrap();
            game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 21 }))
                .unwrap();
            game.client.view = Default::default();
            for _ in 0..40 {
                game.tick_fov();
            }
            let cam_pos = [5.5, 65.62, 5.5];
            assert_eq!(
                game.fov_degrees(1.0, cam_pos).to_bits(),
                hex_f32(&tc["fov_settled_bits"]),
                "speed_0_2_scale_0_5: settled fov"
            );
            assert_eq!(
                game.hud_fov_degrees(1.0, cam_pos).to_bits(),
                hex_f32(&tc["hud_settled_bits"]),
                "speed_0_2_scale_0_5: settled hud fov"
            );
        });
    }

    // Case 3: bow draw in main hand at 0, 5, 10, 20, 25 ticks
    {
        let tc = find_case("bow_draw_mainhand");
        with_client_game(|game| {
            let _server = setup_remote_overworld(game, 1, [0.5, 64.0, 0.5], true);
            game.client
                .settings
                .file
                .set("fovEffectScale", OptV::Double(1.0));
            add_entity_with_data(game, 22, "minecraft:player", [5.5, 64.0, 5.5], 0.0, 0.0, 0);
            let bow = WireStack::new(item_id(game, "minecraft:bow"), 1);
            game.handle_packet(C::SetEquipment(play::SetEquipment {
                id: 22,
                slots: vec![(0, Some(bow))],
            }))
            .unwrap();
            game.handle_packet(C::SetEntityData(play::SetEntityData {
                id: 22,
                values: vec![(8, serializer::BYTE, EntityDataValue::Byte(1))],
            }))
            .unwrap();
            game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 22 }))
                .unwrap();
            let mut elapsed = 0i64;
            for sample in tc["samples"].as_array().unwrap() {
                let target_tick = sample["elapsed_ticks"].as_i64().unwrap();
                while elapsed < target_tick {
                    game.tick_remote_entities(true);
                    elapsed += 1;
                }
                game.client.view = Default::default();
                for _ in 0..40 {
                    game.tick_fov();
                }
                assert_eq!(
                    game.fov_degrees(1.0, [5.5, 65.62, 5.5]).to_bits(),
                    hex_f32(&sample["fov_settled_bits"]),
                    "bow_draw_mainhand at elapsed_ticks={target_tick}"
                );
            }
        });
    }

    // Cases 4 & 5: spyglass in offhand (first person vs third person), fovEffectScale = 0.5
    for (name, ct) in [
        ("spyglass_offhand_first_person", CameraType::FirstPerson),
        ("spyglass_offhand_third_person", CameraType::ThirdPersonBack),
    ] {
        let tc = find_case(name);
        with_client_game(|game| {
            let _server = setup_remote_overworld(game, 1, [0.5, 64.0, 0.5], true);
            game.client
                .settings
                .file
                .set("fovEffectScale", OptV::Double(0.5));
            game.client.camera_type = ct;
            add_entity_with_data(game, 23, "minecraft:player", [5.5, 64.0, 5.5], 0.0, 0.0, 0);
            let spyglass = WireStack::new(item_id(game, "minecraft:spyglass"), 1);
            game.handle_packet(C::SetEquipment(play::SetEquipment {
                id: 23,
                slots: vec![(1, Some(spyglass))],
            }))
            .unwrap();
            game.handle_packet(C::SetEntityData(play::SetEntityData {
                id: 23,
                values: vec![(8, serializer::BYTE, EntityDataValue::Byte(3))],
            }))
            .unwrap();
            game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 23 }))
                .unwrap();
            game.client.view = Default::default();
            for _ in 0..40 {
                game.tick_fov();
            }
            let cam_pos = [5.5, 65.62, 5.5];
            assert_eq!(
                game.fov_degrees(1.0, cam_pos).to_bits(),
                hex_f32(&tc["fov_settled_bits"]),
                "{name}: settled fov"
            );
            assert_eq!(
                game.hud_fov_degrees(1.0, cam_pos).to_bits(),
                hex_f32(&tc["hud_settled_bits"]),
                "{name}: settled hud fov"
            );
        });
    }

    // Cases 6 & 7: local player dying + fast + scoping must not affect healthy spectated camera
    for (name, kind, id) in [
        (
            "local_dying_spectating_healthy_remote_player",
            "minecraft:player",
            24,
        ),
        (
            "local_dying_spectating_healthy_creeper",
            "minecraft:creeper",
            25,
        ),
    ] {
        let tc = find_case(name);
        with_client_game(|game| {
            let _server = setup_remote_overworld(game, 1, [0.5, 64.0, 0.5], true);
            game.client
                .settings
                .file
                .set("fovEffectScale", OptV::Double(1.0));
            let local = game.player.as_mut().unwrap();
            local.set_health(0.0, false).unwrap();
            local.speed = 0.3;
            game.client.chat_ui.death_time = 10;
            game.inventory[0] = Some(ItemStack::new("minecraft:spyglass", 1));
            game.selected = 0;
            assert!(game.start_using_item());

            add_entity_with_data(game, id, kind, [5.5, 64.0, 5.5], 0.0, 0.0, 0);
            game.handle_packet(C::SetCamera(play::SetCamera { camera_id: id }))
                .unwrap();
            game.client.view = Default::default();
            for _ in 0..40 {
                game.tick_fov();
            }
            let cam_pos = [5.5, 65.62, 5.5];
            assert_eq!(
                game.fov_degrees(0.5, cam_pos).to_bits(),
                hex_f32(&tc["fov_settled_bits"]),
                "{name}: settled fov"
            );
            assert_eq!(
                game.hud_fov_degrees(0.5, cam_pos).to_bits(),
                hex_f32(&tc["hud_settled_bits"]),
                "{name}: settled hud fov"
            );
        });
    }

    // Case 8: local player healthy, spectating dying + sprinting RemotePlayer
    {
        let tc = find_case("local_healthy_spectating_dying_sprinting_remote_player");
        with_client_game(|game| {
            let _server = setup_remote_overworld(game, 1, [0.5, 64.0, 0.5], true);
            game.client
                .settings
                .file
                .set("fovEffectScale", OptV::Double(1.0));
            add_entity_with_data(game, 26, "minecraft:player", [5.5, 64.0, 5.5], 0.0, 0.0, 0);
            let attr = speed_attr_id(game);
            game.handle_packet(C::UpdateAttributes(play::UpdateAttributes {
                id: 26,
                values: vec![play::AttributeSnapshot {
                    attribute: attr,
                    base: 0.1,
                    modifiers: vec![play::AttributeModifier {
                        id: "minecraft:sprinting".into(),
                        amount: 0.3,
                        operation: 2,
                    }],
                }],
            }))
            .unwrap();
            game.handle_packet(C::SetEntityData(play::SetEntityData {
                id: 26,
                values: vec![(9, serializer::FLOAT, EntityDataValue::Float(0.0))],
            }))
            .unwrap();
            game.mobs
                .iter_mut()
                .find(|m| m.id == 26)
                .unwrap()
                .death_time = 10;
            game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 26 }))
                .unwrap();
            game.client.view = Default::default();
            for _ in 0..40 {
                game.tick_fov();
            }
            let cam_pos = [5.5, 65.62, 5.5];
            assert_eq!(
                game.fov_degrees(0.5, cam_pos).to_bits(),
                hex_f32(&tc["fov_p05_bits"]),
                "dying sprinting remote player p=0.5 fov"
            );
            assert_eq!(
                game.hud_fov_degrees(0.5, cam_pos).to_bits(),
                hex_f32(&tc["hud_p05_bits"]),
                "dying sprinting remote player p=0.5 hud fov"
            );
            assert_eq!(
                game.fov_degrees(1.0, cam_pos).to_bits(),
                hex_f32(&tc["fov_p1_bits"]),
                "dying sprinting remote player p=1.0 fov"
            );
            assert_eq!(
                game.hud_fov_degrees(1.0, cam_pos).to_bits(),
                hex_f32(&tc["hud_p1_bits"]),
                "dying sprinting remote player p=1.0 hud fov"
            );
        });
    }
}

pub(in crate::game) fn run_interaction_camera_eye_rotation_move_and_teleport_26_3() {
    let witness = parity_witness();
    let life = &witness["interaction_lifecycle"];
    with_client_game(|game| {
        let _server = setup_remote_overworld(game, 1, [0.0, 64.0, 0.0], true);

        // 1. Unsynced spawn keeps eye_height == 0.0 and stores spawn rotation.
        add_entity(
            game,
            200,
            "minecraft:interaction",
            [0.5, 300.0, 0.5],
            90.0,
            30.0,
        );
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 200 }))
            .unwrap();
        let snap = game.spectated_camera_snapshot().expect("interaction 200");
        assert_eq!(
            snap.eye_height.to_bits(),
            hex_f32(&life["unsynced_spawn"]["eye_height_bits"])
        );
        assert_eq!(
            snap.y_rot.to_bits(),
            hex_f32(&life["unsynced_spawn"]["yRot_bits"])
        );
        assert_eq!(
            snap.x_rot.to_bits(),
            hex_f32(&life["unsynced_spawn"]["xRot_bits"])
        );

        // 2. Explicit default (1.0, 1.0) refreshes dimensions to eye_height = 0.85.
        sync_interaction_size(game, 200, 1.0, 1.0);
        let snap = game.spectated_camera_snapshot().unwrap();
        assert_eq!(
            snap.eye_height.to_bits(),
            hex_f32(&life["sync_default_1x1"]["eye_height_bits"])
        );

        // 3. DATA_POSE (6) alone refreshes dimensions on a fresh unsynced Interaction.
        add_entity(
            game,
            201,
            "minecraft:interaction",
            [0.5, 300.0, 0.5],
            0.0,
            0.0,
        );
        game.handle_packet(C::SetEntityData(play::SetEntityData {
            id: 201,
            values: vec![(6, serializer::POSE, EntityDataValue::Id(0))],
        }))
        .unwrap();
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 201 }))
            .unwrap();
        let snap = game.spectated_camera_snapshot().unwrap();
        assert_eq!(
            snap.eye_height.to_bits(),
            hex_f32(&life["sync_pose_only"]["eye_height_bits"])
        );

        // 4. MoveEntityPosRot, relative TeleportEntity (Y_ROT|X_ROT), and far TeleportEntity snap.
        add_entity(
            game,
            210,
            "minecraft:interaction",
            [10.0, 64.0, -5.0],
            0.0,
            0.0,
        );
        sync_interaction_size(game, 210, 0.1, 1.9059);
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 210 }))
            .unwrap();
        game.client.view = Default::default();
        for _ in 0..40 {
            game.tick_fov();
        }

        // Tick 1: snapshot old pose, then MoveEntityPosRot (+1.0, +0.5, -0.25), yaw 45, pitch 15.
        game.tick_remote_entities(true);
        game.handle_packet(C::MoveEntityPosRot(play::MoveEntityPosRot(
            play::MoveEntity {
                id: 210,
                delta: Some(play::VecDelta::Linear([4096, 2048, -1024])),
                rotation: Some((pack_degrees(45.0), pack_degrees(15.0))),
                on_ground: false,
            },
        )))
        .unwrap();
        for frame in life["after_move_pos_rot_frames"].as_array().unwrap() {
            let pt = f32::from_bits(hex_f32(&frame["partial_bits"]));
            let (pos, yaw, pitch) = game.camera_view(pt).unwrap();
            assert_eq!(
                pos.to_array().map(f64::to_bits),
                vec3_bits_expected(&frame["camera_pos_bits"]),
                "move pos at partial {pt}"
            );
            assert_eq!(
                yaw.to_bits(),
                hex_f32(&frame["camera_yRot_bits"]),
                "move yaw at partial {pt}"
            );
            assert_eq!(
                pitch.to_bits(),
                hex_f32(&frame["camera_xRot_bits"]),
                "move pitch at partial {pt}"
            );
        }

        // Tick 2: snapshot old pose, then relative TeleportEntity (+30 yaw, -10 pitch).
        game.tick_remote_entities(true);
        game.handle_packet(C::TeleportEntity(play::TeleportEntity {
            id: 210,
            position: [12.0, 65.0, -4.0],
            delta: [0.0; 3],
            yaw: 30.0,
            pitch: -10.0,
            relatives: play::relative::Y_ROT | play::relative::X_ROT,
            on_ground: false,
        }))
        .unwrap();
        for frame in life["after_relative_teleport_frames"].as_array().unwrap() {
            let pt = f32::from_bits(hex_f32(&frame["partial_bits"]));
            let (pos, yaw, pitch) = game.camera_view(pt).unwrap();
            assert_eq!(
                pos.to_array().map(f64::to_bits),
                vec3_bits_expected(&frame["camera_pos_bits"]),
                "rel tp pos at partial {pt}"
            );
            assert_eq!(
                yaw.to_bits(),
                hex_f32(&frame["camera_yRot_bits"]),
                "rel tp yaw at partial {pt}"
            );
            assert_eq!(
                pitch.to_bits(),
                hex_f32(&frame["camera_xRot_bits"]),
                "rel tp pitch at partial {pt}"
            );
        }

        // Tick 3: snapshot old pose, then far TeleportEntity (> 64 blocks) snaps old pose too.
        game.tick_remote_entities(true);
        game.handle_packet(C::TeleportEntity(play::TeleportEntity {
            id: 210,
            position: [100.0, 65.0, -4.0],
            delta: [0.0; 3],
            yaw: 180.0,
            pitch: -20.0,
            relatives: 0,
            on_ground: false,
        }))
        .unwrap();
        for frame in life["after_far_teleport_frames"].as_array().unwrap() {
            let pt = f32::from_bits(hex_f32(&frame["partial_bits"]));
            let (pos, yaw, pitch) = game.camera_view(pt).unwrap();
            assert_eq!(
                pos.to_array().map(f64::to_bits),
                vec3_bits_expected(&frame["camera_pos_bits"]),
                "far tp pos at partial {pt}"
            );
            assert_eq!(
                yaw.to_bits(),
                hex_f32(&frame["camera_yRot_bits"]),
                "far tp yaw at partial {pt}"
            );
            assert_eq!(
                pitch.to_bits(),
                hex_f32(&frame["camera_xRot_bits"]),
                "far tp pitch at partial {pt}"
            );
        }
    });
}

pub(in crate::game) fn run_particle_camera_before_first_frame_and_after_reset_26_3() {
    use sunrise_client::particles::ParticleLevel;
    use sunrise_level::particle_options::{ParticleOptions, ParticleType};

    let witness = parity_witness();
    let w = &witness["particle_camera_before_first_frame"];
    let before = &w["before_first_frame"];
    let after = &w["after_first_frame"];
    let after_reset = &w["after_camera_reset_before_next_frame"];

    let accepted = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
                    pos: [f64; 3]|
     -> bool {
        game.client.particles.0.clear();
        game.client
            .particles
            .0
            .add(&ParticleOptions::simple(ParticleType::Flame), pos, [0.0; 3]);
        // ParticleEngine::add queues into `pending` (vanilla `particlesToAdd`) until the next tick.
        let level = ParticleLevel {
            world: &game.common.world,
            facts: game.common.res.facts(),
            collision: game.common.collision,
            player: None,
            cached_chunk: std::cell::Cell::new(None),
        };
        game.client.particles.0.tick(&level);
        game.client.particles.0.len() == 1
    };

    with_client_game(|game| {
        let _server = setup_remote_overworld(game, 1, [0.5, 64.0, 0.5], true);
        let plains = *world_biomes!(game).index.get("minecraft:plains").unwrap();
        let desert = *world_biomes!(game).index.get("minecraft:desert").unwrap();
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        for (cx, cz) in [(0, 0), (-1, 0), (0, -1), (-1, -1)] {
            game.world
                .chunks
                .insert((cx, cz), Arc::new(make_chunk(cx, cz, plains, 0xF0)));
        }
        game.world
            .chunks
            .insert((6, 6), Arc::new(make_chunk(6, 6, desert, 0x00)));
        for x in -10..=10 {
            for z in -10..=10 {
                game.world.set(x, -1, z, stone).unwrap();
            }
        }
        game.world.weather.raining = true;
        game.world.weather.rain_level = (1.0, 1.0);
        game.world.game_time = 1;
        game.client.settings.particles = 0;
        game.client.settings.weather_radius = 10;
        game.client.settings.sunrise.weather = true;

        add_entity(
            game,
            31,
            "minecraft:interaction",
            [96.5, 300.0, 96.5],
            0.0,
            0.0,
        );
        sync_interaction_size(game, 31, 0.1, 1.9059);
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 31 }))
            .unwrap();

        // Phase 1: tick before the first frame (Camera.update / alignWithEntity has never run).
        game.client.view = Default::default();
        game.client.fog.rain_fog_multiplier = 0.0;
        game.client.particles.0.clear();
        game.tick_fov();
        game.tick_particles();

        assert_eq!(
            game.client.view.initialized,
            before["initialized"].as_bool().unwrap(),
            "before_first_frame: initialized"
        );
        assert_eq!(
            game.client.particles.0.camera.map(f64::to_bits),
            vec3_bits_expected(&before["camera_pos_bits"]),
            "before_first_frame: particles.camera must be Vec3.ZERO (no camera_eye_position fallback)"
        );
        assert_eq!(
            game.client.fog.rain_fog_multiplier.to_bits(),
            hex_f32(&before["rain_fog_multiplier_bits"]),
            "before_first_frame: rain_fog_multiplier must sample Camera.blockPosition() == (0,0,0)"
        );
        assert_eq!(
            game.client.particles.0.len() as u64,
            before["rain_splash_count"].as_u64().unwrap(),
            "before_first_frame: rain splashes must center on (0,0,0)"
        );
        assert_eq!(
            accepted(game, [0.5, 0.5, 0.5]),
            before["accepted_at_origin"].as_bool().unwrap(),
            "before_first_frame: accepted_at_origin"
        );
        assert_eq!(
            accepted(game, [0.5, 65.62, 0.5]),
            before["accepted_at_local_player_eye"].as_bool().unwrap(),
            "before_first_frame: accepted_at_local_player_eye"
        );
        assert_eq!(
            accepted(game, [96.5, 301.62, 96.5]),
            before["accepted_at_camera_entity_eye"].as_bool().unwrap(),
            "before_first_frame: accepted_at_camera_entity_eye"
        );

        // Phase 2: first frame aligns the camera; ticks now use the aligned position.
        for _ in 0..40 {
            game.tick_fov();
        }
        game.camera_view(1.0);
        game.client.fog.rain_fog_multiplier = 0.0;
        game.client.particles.0.clear();
        game.world.game_time = 1;
        game.tick_particles();

        assert_eq!(
            game.client.view.initialized,
            after["initialized"].as_bool().unwrap(),
            "after_first_frame: initialized"
        );
        assert_eq!(
            game.client.particles.0.camera.map(f64::to_bits),
            vec3_bits_expected(&after["camera_pos_bits"]),
            "after_first_frame: particles.camera"
        );
        assert_eq!(
            game.client.fog.rain_fog_multiplier.to_bits(),
            hex_f32(&after["rain_fog_multiplier_bits"]),
            "after_first_frame: rain_fog_multiplier"
        );
        assert_eq!(
            game.client.particles.0.len() as u64,
            after["rain_splash_count"].as_u64().unwrap(),
            "after_first_frame: rain_splash_count"
        );
        assert_eq!(
            accepted(game, [0.5, 0.5, 0.5]),
            after["accepted_at_origin"].as_bool().unwrap(),
            "after_first_frame: accepted_at_origin"
        );
        assert_eq!(
            accepted(game, [0.5, 65.62, 0.5]),
            after["accepted_at_local_player_eye"].as_bool().unwrap(),
            "after_first_frame: accepted_at_local_player_eye"
        );
        assert_eq!(
            accepted(game, [96.5, 301.62, 96.5]),
            after["accepted_at_camera_entity_eye"].as_bool().unwrap(),
            "after_first_frame: accepted_at_camera_entity_eye"
        );

        // Phase 3: after Camera.reset() (disconnect) + re-entering a level before the next frame,
        // `initialized` is false and `Camera.position()` keeps the last aligned position.
        game.client_logging_out();
        assert_eq!(
            game.client.view.initialized,
            after_reset["initialized"].as_bool().unwrap(),
            "after_camera_reset_before_next_frame: initialized after disconnect"
        );
        let _server2 = setup_remote_overworld(game, 2, [200.5, 64.0, 200.5], true);
        game.tick_fov();
        game.tick_particles();

        assert_eq!(
            game.client.view.initialized,
            after_reset["initialized"].as_bool().unwrap(),
            "after_camera_reset_before_next_frame: initialized before next frame"
        );
        assert_eq!(
            game.client.particles.0.camera.map(f64::to_bits),
            vec3_bits_expected(&after_reset["camera_pos_bits"]),
            "after_camera_reset_before_next_frame: particles.camera must keep last aligned position"
        );
        assert_eq!(
            accepted(game, [0.5, 0.5, 0.5]),
            after_reset["accepted_at_origin"].as_bool().unwrap(),
            "after_camera_reset_before_next_frame: accepted_at_origin"
        );
        assert_eq!(
            accepted(game, [96.5, 301.62, 96.5]),
            after_reset["accepted_at_last_camera_pos"]
                .as_bool()
                .unwrap(),
            "after_camera_reset_before_next_frame: accepted_at_last_camera_pos"
        );
        assert_eq!(
            accepted(game, [200.5, 65.62, 200.5]),
            after_reset["accepted_at_new_entity_eye"].as_bool().unwrap(),
            "after_camera_reset_before_next_frame: accepted_at_new_entity_eye"
        );
    });
}

pub(in crate::game) fn run_removing_camera_entity_keeps_detached_camera_26_3() {
    let witness = parity_witness();
    for kind in ["minecraft:interaction", "minecraft:creeper"] {
        let case = &witness["camera_removal"][kind];
        with_client_game(|game| {
            let _server = setup_remote_overworld(game, 7, [0.5, 64.0, 0.5], true);
            add_entity(game, 31, kind, [10.5, 300.0, 12.5], 45.0, 10.0);
            if kind == "minecraft:interaction" {
                sync_interaction_size(game, 31, 0.1, 1.9059);
            }
            game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 31 }))
                .unwrap();
            game.client.view = Default::default();
            for _ in 0..40 {
                game.tick_fov();
            }
            let (pos_before, _, _) = game.camera_view(1.0).unwrap();
            assert_eq!(
                game.controls_camera(),
                case["after_set_camera"]["is_controlled_camera"]
                    .as_bool()
                    .unwrap(),
                "{kind} before remove"
            );
            assert_eq!(
                game.client.spectated_post_effect,
                case["after_set_camera"]["post_effect"].as_str(),
                "{kind} post effect before remove"
            );
            assert_eq!(
                pos_before.to_array().map(f64::to_bits),
                vec3_bits_expected(&case["after_set_camera"]["camera_pos_bits"]),
                "{kind} pos before remove"
            );

            game.handle_packet(C::RemoveEntities(play::RemoveEntities { ids: vec![31] }))
                .unwrap();
            game.tick_fov();
            let (pos_after, y_after, p_after) = game.camera_view(1.0).unwrap();
            assert_eq!(
                game.controls_camera(),
                case["after_remove_entities"]["is_controlled_camera"]
                    .as_bool()
                    .unwrap(),
                "{kind} must stay detached after removal"
            );
            assert_eq!(
                game.client.spectated_post_effect,
                case["after_remove_entities"]["post_effect"].as_str(),
                "{kind} post effect must persist after removal"
            );
            assert_eq!(
                pos_after.to_array().map(f64::to_bits),
                vec3_bits_expected(&case["after_remove_entities"]["camera_pos_bits"]),
                "{kind} camera pos must remain at removed entity"
            );
            assert_eq!(
                y_after.to_bits(),
                hex_f32(&case["after_remove_entities"]["camera_yRot_bits"]),
                "{kind} camera yaw after removal"
            );
            assert_eq!(
                p_after.to_bits(),
                hex_f32(&case["after_remove_entities"]["camera_xRot_bits"]),
                "{kind} camera pitch after removal"
            );

            game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 7 }))
                .unwrap();
            assert_eq!(
                game.controls_camera(),
                case["after_set_camera_player"]["is_controlled_camera"]
                    .as_bool()
                    .unwrap(),
                "{kind} controls_camera after restore"
            );
            assert_eq!(
                game.client.spectated_post_effect,
                case["after_set_camera_player"]["post_effect"].as_str(),
                "{kind} post effect after restore"
            );
        });
    }
}

pub(in crate::game) fn run_review_followup_camera_parity_26_3() {
    let witness = parity_witness();
    let rf = &witness["review_followup"];
    with_client_game(|game| {
        let _server = setup_remote_overworld(game, 1, [0.5, 64.0, 0.5], true);
        let plains = *world_biomes!(game).index.get("minecraft:plains").unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(make_chunk(0, 0, plains, 0xF0)));

        // Finding 1: ExperienceOrb 3-tick correction + EnderDragon living camera state.
        add_entity(
            game,
            401,
            "minecraft:experience_orb",
            [10.0, 64.0, 10.0],
            0.0,
            0.0,
        );
        game.orbs
            .iter_mut()
            .find(|o| o.id == 401)
            .unwrap()
            .no_gravity = true;
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 401 }))
            .unwrap();
        game.client.view = Default::default();
        for _ in 0..40 {
            game.tick_fov();
        }
        game.handle_packet(C::EntityPositionSync(play::EntityPositionSync {
            id: 401,
            path: play::PositionPath::Linear([13.0, 64.0, 10.0]),
            yaw: 0.0,
            pitch: 0.0,
            on_ground: false,
        }))
        .unwrap();
        game.tick_remote_entities(true);
        let (orb_cam_pos, _, _) = game.camera_view(0.5).unwrap();
        assert_eq!(
            orb_cam_pos.to_array().map(f64::to_bits),
            vec3_bits_expected(&rf["orb_correction"]["step1_cam_half_bits"]),
            "orb 3-tick correction half-frame camera pos must match 26.3"
        );

        add_entity(
            game,
            402,
            "minecraft:ender_dragon",
            [10.0, 80.0, -5.0],
            45.0,
            0.0,
        );
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 402 }))
            .unwrap();
        let dragon_cam = game.camera_subject().unwrap();
        assert_eq!(
            dragon_cam.camera.living.is_some(),
            rf["ender_dragon"]["is_living"].as_bool().unwrap()
        );
        assert_eq!(
            dragon_cam.camera.eye_height.to_bits(),
            hex_f32(&rf["ender_dragon"]["eye_height_bits"])
        );

        // Finding 3: SetCamera(creeper) while already in third person installs post effect.
        add_entity(game, 403, "minecraft:creeper", [5.0, 64.0, 5.0], 0.0, 0.0);
        game.client.camera_type = sunrise_level::camera_math::CameraType::ThirdPersonBack;
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 403 }))
            .unwrap();
        assert_eq!(
            game.client.spectated_post_effect,
            rf["third_person_post_effect"]["third_person_creeper_post_effect"].as_str()
        );
        game.client.camera_type = sunrise_level::camera_math::CameraType::FirstPerson;

        // Finding 4: should_render_block_outline_for_hit with spectator/interactable and remote player.
        let craft_state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:crafting_table")
            .unwrap();
        let stone_state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.set(0, 64, 2, craft_state).unwrap();
        game.world.set(0, 64, 3, stone_state).unwrap();
        game.handle_packet(C::GameEvent(play::GameEvent {
            event: 3,
            param: 3.0,
        }))
        .unwrap();
        game.client.chat_ui.hide_gui = false;
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 1 }))
            .unwrap();
        let craft_hit = sunrise_gameplay_common::game::Hit {
            pos: [0, 64, 2],
            face: [0, 0, -1],
            point: [0.5, 64.5, 2.0],
            inside: false,
        };
        let stone_hit = sunrise_gameplay_common::game::Hit {
            pos: [0, 64, 3],
            face: [0, 0, -1],
            point: [0.5, 64.5, 3.0],
            inside: false,
        };
        let bom = &rf["block_outline_matrix"];
        assert_eq!(
            game.should_render_block_outline_for_hit(Some(&craft_hit)),
            bom["spectator_self_crafting_table"].as_bool().unwrap()
        );
        assert_eq!(
            game.should_render_block_outline_for_hit(Some(&stone_hit)),
            bom["spectator_self_stone"].as_bool().unwrap()
        );
        add_entity(game, 2, "minecraft:player", [2.0, 64.0, 2.0], 0.0, 0.0);
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 2 }))
            .unwrap();
        assert_eq!(
            game.should_render_block_outline_for_hit(Some(&stone_hit)),
            bom["spectator_watching_remote_player_stone"]
                .as_bool()
                .unwrap()
        );

        // Finding 6: EnderDragon wire id wrapping matches 26.3 signed 32-bit arithmetic.
        for (key, wire) in [
            ("part_ids_for_max_i32", i32::MAX as u32),
            ("part_ids_for_neg_ten", (-10i32) as u32),
        ] {
            let expected_parts: Vec<i32> = rf["dragon_part_wrapping"][key]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_i64().unwrap() as i32)
                .collect();
            for (idx, want) in expected_parts.into_iter().enumerate() {
                assert_eq!(
                    crate::game::dragon_common::dragon_part_wire_id(wire, idx),
                    want,
                    "dragon part wire id mismatch for {key}[{idx}]"
                );
            }
        }

        // Finding 7: Removing moving remote player keeps mid-tick pose and detached CameraPlayer.
        add_entity(game, 407, "minecraft:player", [0.0, 64.0, 0.0], 0.0, 0.0);
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 407 }))
            .unwrap();
        game.client.view = Default::default();
        for _ in 0..40 {
            game.tick_fov();
        }
        if let Some(m) = game.mobs.iter_mut().find(|m| m.id == 407) {
            m.previous = sunrise_core::geometry::Vec3::new(0.0, 64.0, 0.0);
            m.position = sunrise_core::geometry::Vec3::new(2.0, 64.0, 0.0);
        }
        game.handle_packet(C::RemoveEntities(play::RemoveEntities { ids: vec![407] }))
            .unwrap();
        let (mid_pos, _, _) = game.camera_view(0.5).unwrap();
        assert_eq!(
            mid_pos.to_array().map(f64::to_bits),
            vec3_bits_expected(&rf["detached_mid_tick"]["mid_tick_cam_pos_bits"]),
            "removing moving camera must preserve mid-tick pose"
        );
        assert_eq!(
            matches!(
                game.camera_player(),
                sunrise_client_gameplay::game::client_side::remote_camera::CameraPlayer::Detached
            ),
            rf["detached_mid_tick"]["detached_player_still_camera_player"]
                .as_bool()
                .unwrap()
        );

        // Finding 9: Relative +65 teleport preserves relative old pose and velocity delta.
        add_entity(
            game,
            409,
            "minecraft:interaction",
            [0.0, 64.0, 0.0],
            0.0,
            0.0,
        );
        game.tick_remote_entities(true);
        game.handle_packet(C::EntityPositionSync(play::EntityPositionSync {
            id: 409,
            path: play::PositionPath::Linear([1.0, 64.0, 0.0]),
            yaw: 0.0,
            pitch: 0.0,
            on_ground: false,
        }))
        .unwrap();
        game.handle_packet(C::TeleportEntity(play::TeleportEntity {
            id: 409,
            position: [65.0, 0.0, 0.0],
            delta: [0.5, 0.0, 0.0],
            yaw: 0.0,
            pitch: 0.0,
            relatives: play::relative::X
                | play::relative::Y
                | play::relative::Z
                | play::relative::Y_ROT
                | play::relative::X_ROT,
            on_ground: false,
        }))
        .unwrap();
        let inter = game.client.interactions.get(&409).unwrap();
        assert_eq!(
            [inter.previous.x, inter.previous.y, inter.previous.z].map(f64::to_bits),
            vec3_bits_expected(&rf["relative_teleport_old_pose"]["old_pos_bits"])
        );
        assert_eq!(
            [inter.position.x, inter.position.y, inter.position.z].map(f64::to_bits),
            vec3_bits_expected(&rf["relative_teleport_old_pose"]["cur_pos_bits"])
        );

        // Finding 10: ArmorStand pitch history preserved on intermediate frame.
        add_entity(
            game,
            410,
            "minecraft:armor_stand",
            [5.0, 64.0, 5.0],
            0.0,
            0.0,
        );
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 410 }))
            .unwrap();
        if let Some(a) = game.armor_stands.iter_mut().find(|a| a.id == 410) {
            a.previous_pitch = 0.0;
            a.pitch = 30.0;
        }
        let (_, _, stand_half_pitch) = game.camera_view(0.5).unwrap();
        assert_eq!(
            stand_half_pitch.to_bits(),
            hex_f32(&rf["armor_stand_pitch"]["cam_half_xRot_bits"])
        );

        // Finding 11: 18-deep passenger chain keeps root unfrozen when frozen.
        for k in 0..=17 {
            add_entity(
                game,
                420 + k,
                "minecraft:interaction",
                [0.0, 64.0, 0.0],
                0.0,
                0.0,
            );
        }
        add_entity(game, 440, "minecraft:player", [0.0, 64.0, 0.0], 0.0, 0.0);
        for k in 0..17 {
            game.handle_packet(C::SetPassengers(play::SetPassengers {
                vehicle: 420 + k,
                passengers: vec![420 + k + 1],
            }))
            .unwrap();
        }
        game.handle_packet(C::SetPassengers(play::SetPassengers {
            vehicle: 437,
            passengers: vec![440],
        }))
        .unwrap();
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 420 }))
            .unwrap();
        let deep_sub = game.camera_subject().unwrap();
        assert_eq!(
            !deep_sub.has_player_passenger,
            rf["deep_passenger_chain_18"]["root_frozen"]
                .as_bool()
                .unwrap()
        );

        // Finding 12: Watched living entity blindness blocks sky.
        add_entity(game, 450, "minecraft:creeper", [5.0, 64.0, 5.0], 0.0, 0.0);
        let blindness_id = game
            .client
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:mob_effect")
            .unwrap()
            .id("minecraft:blindness")
            .unwrap();
        game.handle_packet(C::UpdateMobEffect(play::UpdateMobEffect {
            id: 450,
            effect: blindness_id,
            amplifier: 0,
            duration: 200,
            flags: 0,
        }))
        .unwrap();
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 450 }))
            .unwrap();
        assert_eq!(
            game.camera_subject().unwrap().blocks_sky,
            rf["watched_living_effects"]["creeper_blindness_blocks_sky"]
                .as_bool()
                .unwrap()
        );

        // Finding 13: Watched player HurtAnimation stores yaw and living tick decrements cooldown.
        add_entity(game, 451, "minecraft:player", [5.0, 64.0, 5.0], 0.0, 0.0);
        game.handle_packet(C::HurtAnimation(play::HurtAnimation {
            id: 451,
            yaw: 123.5,
        }))
        .unwrap();
        if let Some(m) = game.mobs.iter_mut().find(|m| m.id == 451) {
            m.invulnerable = 20;
        }
        game.tick_remote_entities(true);
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id: 451 }))
            .unwrap();
        assert_eq!(
            game.camera_subject()
                .unwrap()
                .hurt
                .unwrap()
                .hurt_dir
                .to_bits(),
            hex_f32(&rf["watched_player_hurt"]["hurt_dir_bits"])
        );
        assert_eq!(
            game.mobs.iter().find(|m| m.id == 451).unwrap().invulnerable,
            rf["watched_player_hurt"]["cooldown_after_base_tick"]
                .as_i64()
                .unwrap() as i32
        );

        // Finding 14: Max-health updates clamp at living tick, not on receipt.
        let max_health_attr = game
            .motion
            .attributes
            .id("minecraft:max_health")
            .unwrap()
            .registry_id();
        add_entity(game, 452, "minecraft:pig", [0.0, 64.0, 0.0], 0.0, 0.0);
        if let Some(m) = game.mobs.iter_mut().find(|m| m.id == 452) {
            m.attrs.max_health = 20.0;
            m.health = 20.0;
        }
        for val in [10.0, 20.0] {
            game.handle_packet(C::UpdateAttributes(play::UpdateAttributes {
                id: 452,
                values: vec![play::AttributeSnapshot {
                    attribute: max_health_attr,
                    base: val,
                    modifiers: Vec::new(),
                }],
            }))
            .unwrap();
        }
        game.tick_remote_entities(true);
        assert_eq!(
            game.mobs
                .iter()
                .find(|m| m.id == 452)
                .unwrap()
                .health
                .to_bits(),
            hex_f32(&rf["max_health_clamp"]["same_tick_health_bits"])
        );

        add_entity(game, 453, "minecraft:pig", [0.0, 64.0, 0.0], 0.0, 0.0);
        if let Some(m) = game.mobs.iter_mut().find(|m| m.id == 453) {
            m.attrs.max_health = 20.0;
            m.health = 20.0;
        }
        game.handle_packet(C::UpdateAttributes(play::UpdateAttributes {
            id: 453,
            values: vec![play::AttributeSnapshot {
                attribute: max_health_attr,
                base: 10.0,
                modifiers: Vec::new(),
            }],
        }))
        .unwrap();
        game.tick_remote_entities(true);
        game.handle_packet(C::UpdateAttributes(play::UpdateAttributes {
            id: 453,
            values: vec![play::AttributeSnapshot {
                attribute: max_health_attr,
                base: 20.0,
                modifiers: Vec::new(),
            }],
        }))
        .unwrap();
        game.tick_remote_entities(true);
        assert_eq!(
            game.mobs
                .iter()
                .find(|m| m.id == 453)
                .unwrap()
                .health
                .to_bits(),
            hex_f32(&rf["max_health_clamp"]["across_tick_health_bits"])
        );

        // Finding 15: Sleeping metadata moves a living mob onto the bed.
        let bed_state = game
            .pack
            .assets
            .registry
            .resolve(&sunrise_world::chunk::BlockState {
                name: "minecraft:red_bed".into(),
                properties: [("facing", "south"), ("occupied", "false"), ("part", "head")]
                    .into_iter()
                    .map(|(k, v)| (k.into(), v.into()))
                    .collect(),
            })
            .unwrap();
        game.world.set(7, 64, 5, bed_state).unwrap();
        add_entity(game, 454, "minecraft:creeper", [5.5, 64.0, 5.5], 0.0, 0.0);
        game.handle_packet(C::SetEntityData(play::SetEntityData {
            id: 454,
            values: vec![(
                14,
                serializer::OPTIONAL_BLOCK_POS,
                EntityDataValue::OptPos(Some([7, 64, 5])),
            )],
        }))
        .unwrap();
        let v = game.mobs.iter().find(|m| m.id == 454).unwrap();
        assert_eq!(
            [v.position.x, v.position.y, v.position.z].map(f64::to_bits),
            vec3_bits_expected(&rf["sleeping_mob_set_pos_to_bed"]["mob_pos_after_sleep_bits"])
        );
    });
}

pub(in crate::game) fn run_sleeping_spectated_living_entity_bed_view_and_visibility_26_3() {
    let witness = parity_witness();
    let cases = witness["sleeping_spectated_camera"]["cases"]
        .as_array()
        .unwrap();
    assert_eq!(cases.len(), 9);

    with_client_game(|game| {
        let _server = setup_remote_overworld(game, 1, [0.5, 64.0, 0.5], true);
        game.world
            .chunks
            .insert((0, 0), Arc::new(make_chunk(0, 0, 0, 0xF0)));
        let registry = game.pack.assets.registry;
        let bed_block = |facing: &str| {
            registry
                .resolve(&sunrise_world::chunk::BlockState {
                    name: "minecraft:red_bed".into(),
                    properties: [("facing", facing), ("occupied", "false"), ("part", "head")]
                        .into_iter()
                        .map(|(k, v)| (k.into(), v.into()))
                        .collect(),
                })
                .unwrap()
        };
        let furnace_west = registry
            .resolve(&sunrise_world::chunk::BlockState {
                name: "minecraft:furnace".into(),
                properties: [("facing", "west"), ("lit", "false")]
                    .into_iter()
                    .map(|(k, v)| (k.into(), v.into()))
                    .collect(),
            })
            .unwrap();
        let stone = registry.default_state("minecraft:stone").unwrap();

        for (id, tc) in (500..).zip(cases.iter()) {
            let name = tc["name"].as_str().unwrap();
            let kind = tc["kind"].as_str().unwrap();
            let sleeping = tc["sleeping"].as_bool().unwrap();
            let state = match name {
                "remote_player_bed_south" | "remote_player_awake" => bed_block("south"),
                "remote_player_bed_west" | "zombie_bed_west" => bed_block("west"),
                "remote_player_bed_north" => bed_block("north"),
                "remote_player_bed_east" => bed_block("east"),
                "remote_player_furnace_non_bed" => furnace_west,
                "remote_player_stone_non_bed" | "zombie_stone_non_bed" => stone,
                other => panic!("unexpected sleeping case {other}"),
            };
            game.world.set(10, 64, 10, state).unwrap();

            add_entity_with_data(game, id, kind, [10.5, 64.6875, 10.5], 37.5, -15.0, 0);
            game.handle_packet(C::SetEntityData(play::SetEntityData {
                id,
                values: vec![
                    (
                        6,
                        serializer::POSE,
                        EntityDataValue::Id(if sleeping { 2 } else { 0 }),
                    ),
                    (
                        14,
                        serializer::OPTIONAL_BLOCK_POS,
                        EntityDataValue::OptPos(sleeping.then_some([10, 64, 10])),
                    ),
                ],
            }))
            .unwrap();
            game.handle_packet(C::SetCamera(play::SetCamera { camera_id: id }))
                .unwrap();

            game.client.view = Default::default();
            game.tick_fov();
            let (pos1, _, _) = game.camera_view(1.0).expect("1-tick camera view");
            assert_eq!(
                pos1.to_array().map(f64::to_bits),
                vec3_bits_expected(&tc["camera_pos_after_1_tick_bits"]),
                "{name}: 1-tick camera_pos_bits"
            );

            for _ in 0..40 {
                game.tick_fov();
            }
            let (pos, yaw, pitch) = game.camera_view(1.0).expect("settled camera view");
            assert_eq!(
                pos.to_array().map(f64::to_bits),
                vec3_bits_expected(&tc["camera_pos_settled_bits"]),
                "{name}: settled camera_pos_bits"
            );
            assert_eq!(
                yaw.to_bits(),
                hex_f32(&tc["camera_yRot_bits"]),
                "{name}: camera_yRot_bits"
            );
            assert_eq!(
                pitch.to_bits(),
                hex_f32(&tc["camera_xRot_bits"]),
                "{name}: camera_xRot_bits"
            );
            let q = game.client.view.rotation;
            assert_eq!(
                [q.x.to_bits(), q.y.to_bits(), q.z.to_bits(), q.w.to_bits()],
                [
                    hex_f32(&tc["camera_rotation_bits"][0]),
                    hex_f32(&tc["camera_rotation_bits"][1]),
                    hex_f32(&tc["camera_rotation_bits"][2]),
                    hex_f32(&tc["camera_rotation_bits"][3]),
                ],
                "{name}: camera_rotation_bits"
            );
            let subject = game.camera_subject().expect("camera subject");
            assert_eq!(
                subject.is_sleeping,
                tc["state_is_sleeping"].as_bool().unwrap(),
                "{name}: subject.is_sleeping"
            );
            assert_eq!(
                game.hides_as_camera(id as u32),
                tc["hidden_in_first_person"].as_bool().unwrap(),
                "{name}: hides_as_camera in first person"
            );
        }
    });
}

#[must_use]
pub(in crate::game) fn setup_remote_overworld(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    player_id: i32,
    player_pos: [f64; 3],
    flat: bool,
) -> Pipe {
    game.native_lifecycle = NativeLifecycle::default();
    let (client, server) = Pipe::pair();
    game.connect_local(client, "CameraParityTest").unwrap();
    let spawn = SpawnInfo {
        dimension_type: 0,
        dimension: "minecraft:overworld".into(),
        seed: 0,
        game_type: 0,
        previous_game_type: None,
        debug: false,
        flat,
        last_death: None,
        portal_cooldown: 0,
        sea_level: 63,
    };
    game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Login, Reason::Other);
    let remote = game.client.remote.as_mut().unwrap();
    remote.logged_in = true;
    remote.entity_id = player_id;
    remote.view_distance = 8;
    remote.level_load.finish();
    game.player = Some(
        PlayerMotion::new(
            game.motion,
            Vec3::new(player_pos[0], player_pos[1], player_pos[2]),
        )
        .unwrap(),
    );
    game.client.camera_type = CameraType::FirstPerson;
    game.client.settings.fov = 70.0;
    game.client.settings.render_distance = 8;
    server
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
