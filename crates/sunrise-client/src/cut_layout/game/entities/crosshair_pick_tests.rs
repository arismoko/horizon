//! Crosshair targeting parity tests (#32 TNT/falling blocks, #34 root-vehicle exclusion) against vanilla 26.3.
use mod_api::CrosshairTarget;
use serde_json::Value;
use std::sync::Arc;
use sunrise_client::level_loading::Reason;
use sunrise_core::{geometry::Vec3, position::BlockPos};
use sunrise_gameplay_common::game::side_ops::player::PlayerSideGameExt;
use sunrise_items::items::ItemStack;
use sunrise_level::camera_math::CameraType;
use sunrise_level::chunk::Chunk;
use sunrise_protocol::{
    buf::pack_degrees,
    connection::{Connection, Pipe},
    entity_data::Value as EntityDataValue,
    packets::{
        configuration::{RegistryTags, TagEntry, UpdateTags},
        play::{self, Clientbound as C, SpawnInfo},
    },
};
use sunrise_sim::first_person_sim::Hand;
use sunrise_sim::{entities::ArmorStand, mobs::Mob, motion::PlayerMotion};
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_client_gameplay::game::client_side::native_client_session::tests::finish_login,
    sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
    sunrise_gameplay_common::game::FallingBlock, sunrise_gameplay_common::game::InteractTarget,
    sunrise_gameplay_common::game::entity_memory::OffLevel,
    sunrise_gameplay_common::game::in_own_process,
    sunrise_gameplay_common::game::interactions_common::RemoteInteraction,
    sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycle,
    sunrise_gameplay_common::game::tnt_common::PrimedTnt,
};

fn crosshair_witness() -> Value {
    let path = std::env::var("SUNRISE_CROSSHAIR_PICK_WITNESS")
        .expect("run through tools/test_crosshair_pick_parity.py, which records the witness");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "failed to read SUNRISE_CROSSHAIR_PICK_WITNESS={path} (run through tools/test_crosshair_pick_parity.py): {err}"
        )
    });
    serde_json::from_str(&raw).expect("valid 26.3 crosshair pick witness")
}

fn hex_f32(v: &Value) -> u32 {
    u32::from_str_radix(v.as_str().unwrap(), 16).unwrap()
}

fn hex_f64(v: &Value) -> u64 {
    u64::from_str_radix(v.as_str().unwrap(), 16).unwrap()
}

fn vec3_from_bits(v: &Value) -> [f64; 3] {
    [
        f64::from_bits(hex_f64(&v[0])),
        f64::from_bits(hex_f64(&v[1])),
        f64::from_bits(hex_f64(&v[2])),
    ]
}

fn vec3_bits_expected(v: &Value) -> [u64; 3] {
    [hex_f64(&v[0]), hex_f64(&v[1]), hex_f64(&v[2])]
}

fn face_from_name(name: &str) -> [i32; 3] {
    match name {
        "DOWN" => [0, -1, 0],
        "UP" => [0, 1, 0],
        "NORTH" => [0, 0, -1],
        "SOUTH" => [0, 0, 1],
        "WEST" => [-1, 0, 0],
        "EAST" => [1, 0, 0],
        other => panic!("unexpected block face {other}"),
    }
}

fn entity_type_id(game: &sunrise_client_gameplay::game::ClientGame<'_>, kind: &str) -> i32 {
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

#[must_use]
fn setup_remote_overworld(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    player_id: i32,
    player_pos: [f64; 3],
) -> Pipe {
    game.native_lifecycle = NativeLifecycle::default();
    let (client, server) = Pipe::pair();
    game.connect_local(client, "CrosshairPickTest").unwrap();
    let spawn = SpawnInfo {
        dimension_type: 0,
        dimension: "minecraft:overworld".into(),
        seed: 0,
        game_type: 0,
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
    remote.entity_id = player_id;
    remote.view_distance = 8;
    remote.level_load.finish();
    game.client.title.0 = None;
    let pos = Vec3::new(player_pos[0], player_pos[1], player_pos[2]);
    game.player = Some(PlayerMotion::new(game.motion, pos).unwrap());
    game.previous_position = pos;
    game.client.camera_type = CameraType::FirstPerson;
    game.client.settings.fov = 70.0;
    game.client.settings.render_distance = 8;
    server
}

fn fixture_entity_uuid(id: i32) -> u128 {
    // The wire ID can be negative; the independent fixture UUID only needs to be unique.
    u128::from(id.cast_unsigned()) + 1000
}

fn add_entity_with_data(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    id: i32,
    kind: &str,
    position: [f64; 3],
    yaw: f32,
    pitch: f32,
    data: i32,
) {
    let uuid = fixture_entity_uuid(id);
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

fn sync_interaction_size(
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

fn ensure_chunk(game: &mut sunrise_client_gameplay::game::ClientGame<'_>, cx: i32, cz: i32) {
    if !game.world.chunks.contains_key(&(cx, cz)) {
        game.world
            .chunks
            .insert((cx, cz), Arc::new(Chunk::empty(cx, cz)));
    }
}

fn pin_entity_pose(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    player_id: i32,
    id: i32,
    pos_arr: [f64; 3],
    yaw: f32,
    pitch: f32,
) {
    let pos = Vec3::new(pos_arr[0], pos_arr[1], pos_arr[2]);
    if id == player_id {
        if let Some(p) = game.player.as_mut() {
            p.position = pos;
        }
        game.previous_position = pos;
        return;
    }
    let uid = id as u32;
    if let Some(t) = game
        .client
        .remote
        .as_mut()
        .and_then(|r| r.entities.get_mut(&id))
    {
        (t.position, t.previous) = (pos_arr, pos_arr);
        (t.yaw, t.previous_yaw, t.pitch, t.previous_pitch) = (yaw, yaw, pitch, pitch);
    }
    game.mobs.for_each_mut(|m| {
        if m.id == uid {
            (m.position, m.previous) = (pos, pos);
            (m.yaw, m.previous_yaw, m.head_yaw, m.previous_head_yaw) = (yaw, yaw, yaw, yaw);
            (m.head_pitch, m.previous_head_pitch) = (pitch, pitch);
        }
    });
    game.armor_stands.for_each_mut(|a| {
        if a.id == uid {
            (a.position, a.previous) = (pos, pos);
            (a.yaw, a.previous_yaw, a.pitch) = (yaw, yaw, pitch);
        }
    });
    game.boats.for_each_mut(|b| {
        if b.id == uid {
            (b.position, b.previous, b.yaw, b.previous_yaw, b.pitch) = (pos, pos, yaw, yaw, pitch);
        }
    });
    game.carts.for_each_mut(|c| {
        if c.id == uid {
            (c.position, c.previous) = (pos, pos);
            (c.yaw, c.previous_yaw, c.pitch, c.previous_pitch) = (yaw, yaw, pitch, pitch);
        }
    });
    if let Some(inter) = game.client.interactions.get_mut(&id) {
        (inter.position, inter.previous) = (pos, pos);
        (
            inter.yaw,
            inter.previous_yaw,
            inter.pitch,
            inter.previous_pitch,
        ) = (yaw, yaw, pitch, pitch);
    }
    game.tnts.for_each_mut(|t| {
        if t.id == uid {
            (t.position, t.previous, t.rotation) = (pos, pos, [yaw, pitch]);
        }
    });
    game.falling.for_each_mut(|f| {
        if f.id == uid {
            (f.position, f.previous, f.rotation) = (pos, pos, [yaw, pitch]);
        }
    });
    game.creatures.projectiles.for_each_mut(|p| {
        if p.id == uid {
            (p.position, p.previous, p.yaw, p.pitch) = (pos, pos, yaw, pitch);
        }
    });
    if let Some(d) = game.dragon.as_mut()
        && d.id == uid
    {
        (d.position, d.previous, d.yaw, d.previous_yaw) = (pos, pos, yaw, yaw);
    }
}

fn replay_witness_case(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    tc: &Value,
) -> Pipe {
    let player_id = tc["player_id"].as_i64().unwrap() as i32;
    let camera_id = tc["camera_id"].as_i64().unwrap() as i32;
    let entities = tc["entities"].as_array().unwrap();
    let player_entry = entities
        .iter()
        .find(|e| e["id"].as_i64().unwrap() as i32 == player_id)
        .unwrap();
    let player_pos = vec3_from_bits(&player_entry["pos_bits"]);

    let server = setup_remote_overworld(game, player_id, player_pos);
    for (cx, cz) in [(0, 0), (-1, 0), (0, -1), (-1, -1)] {
        ensure_chunk(game, cx, cz);
    }
    for b in tc["blocks"].as_array().unwrap() {
        let pos = b["pos"].as_array().unwrap();
        let x = pos[0].as_i64().unwrap() as i32;
        let y = pos[1].as_i64().unwrap() as i32;
        let z = pos[2].as_i64().unwrap() as i32;
        ensure_chunk(game, x >> 4, z >> 4);
        let state = game
            .pack
            .assets
            .registry
            .default_state(b["block"].as_str().unwrap())
            .unwrap();
        game.world.set(x, y, z, state).unwrap();
    }

    for e in entities {
        let id = e["id"].as_i64().unwrap() as i32;
        if id == player_id {
            continue;
        }
        let kind = e["kind"].as_str().unwrap();
        let pos = vec3_from_bits(&e["pos_bits"]);
        let yaw = f32::from_bits(hex_f32(&e["yRot_bits"]));
        let pitch = f32::from_bits(hex_f32(&e["xRot_bits"]));
        let data = if kind == "minecraft:falling_block" {
            let block_name = e["block_state"].as_str().unwrap();
            game.pack
                .assets
                .registry
                .default_state(block_name)
                .unwrap()
                .0 as i32
        } else {
            e["data"].as_i64().unwrap() as i32
        };
        add_entity_with_data(game, id, kind, pos, yaw, pitch, data);
        if kind == "minecraft:interaction" {
            sync_interaction_size(game, id, 1.0, 1.0);
        }
    }

    for ride in tc["rides"].as_array().unwrap() {
        let vehicle = ride["vehicle"].as_i64().unwrap() as i32;
        let passengers: Vec<i32> = ride["passengers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_i64().unwrap() as i32)
            .collect();
        game.handle_packet(C::SetPassengers(play::SetPassengers {
            vehicle,
            passengers,
        }))
        .unwrap();
    }

    if camera_id != player_id {
        game.handle_packet(C::GameEvent(play::GameEvent {
            event: 3,
            param: 3.0,
        }))
        .unwrap();
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id }))
            .unwrap();
    }

    // Pin exact witness positions and unquantized float rotations after SetPassengers.
    for e in entities {
        let id = e["id"].as_i64().unwrap() as i32;
        let pos = vec3_from_bits(&e["pos_bits"]);
        let yaw = f32::from_bits(hex_f32(&e["yRot_bits"]));
        let pitch = f32::from_bits(hex_f32(&e["xRot_bits"]));
        pin_entity_pose(game, player_id, id, pos, yaw, pitch);
        if id != player_id {
            game.entity_section_moved_id(id as u32);
        }
    }

    if let Some(moves) = tc.get("moves").and_then(Value::as_array) {
        for mv in moves {
            let id = mv["id"].as_i64().unwrap() as i32;
            let pos = vec3_from_bits(&mv["pos_bits"]);
            pin_entity_pose(game, player_id, id, pos, 0.0, 0.0);
            game.entity_section_moved_id(id as u32);
        }
    }

    if let Some(modes) = tc.get("player_modes").and_then(Value::as_object) {
        for (id_str, mode_val) in modes {
            let id: i32 = id_str.parse().unwrap();
            let mode = mode_val.as_i64().unwrap() as i32;
            let uuid = fixture_entity_uuid(id);
            game.handle_packet(C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                actions: 1 | 4,
                entries: vec![play::PlayerInfo {
                    uuid,
                    name: Some(format!("Remote{id}")),
                    game_mode: Some(mode),
                    ..Default::default()
                }],
            }))
            .unwrap();
        }
    }

    if let Some(healths) = tc.get("dragon_health").and_then(Value::as_object) {
        for (id_str, bits) in healths {
            let id: i32 = id_str.parse().unwrap();
            let health = f32::from_bits(hex_f32(bits));
            game.handle_packet(C::SetEntityData(play::SetEntityData {
                id,
                values: vec![(9, 3, EntityDataValue::Float(health))],
            }))
            .unwrap();
        }
    }

    if let Some(parts) = tc.get("dragon_parts").and_then(Value::as_array)
        && let Some(d) = game.dragon.as_mut()
    {
        for part in parts {
            let index = part["index"].as_u64().unwrap() as usize;
            let p = vec3_from_bits(&part["pos_bits"]);
            let v = Vec3::new(p[0], p[1], p[2]);
            d.parts[index].0 = v;
            d.parts[index].1 = v;
        }
    }

    if let Some(item_id) = tc.get("main_hand_item").and_then(Value::as_str) {
        game.inventory[0] = Some(ItemStack::new(item_id, 1));
        game.selected = 0;
    }

    let cam_pos = vec3_from_bits(&tc["camera_pos_bits"]);
    let cam_yaw = f32::from_bits(hex_f32(&tc["camera_yRot_bits"]));
    let cam_pitch = f32::from_bits(hex_f32(&tc["camera_xRot_bits"]));
    pin_entity_pose(game, player_id, camera_id, cam_pos, cam_yaw, cam_pitch);
    if let Some(prev) = tc.get("camera_prev_pos_bits") {
        let prev = vec3_from_bits(prev);
        let prev_v = Vec3::new(prev[0], prev[1], prev[2]);
        if camera_id == player_id {
            game.previous_position = prev_v;
        } else {
            let uid = camera_id as u32;
            game.mobs.for_each_mut(|m| {
                if m.id == uid {
                    m.previous = prev_v;
                }
            });
            if let Some(d) = game.dragon.as_mut()
                && d.id == uid
            {
                d.previous = prev_v;
            }
        }
    }
    if camera_id == player_id {
        game.yaw = cam_yaw;
        game.pitch = cam_pitch;
    }

    let partial_tick = tc
        .get("partial_tick_bits")
        .map_or(1.0_f32, |v| f32::from_bits(hex_f32(v)));
    game.block_reach = f64::from_bits(hex_f64(&tc["block_reach_bits"]));
    game.entity_reach = f64::from_bits(hex_f64(&tc["entity_reach_bits"]));
    game.refresh_client_pick(partial_tick);
    server
}

fn check_case_hit(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    tc: &Value,
    failures: &mut Vec<String>,
) {
    let name = tc["name"].as_str().unwrap();
    let hit = &tc["hit"];
    let want_type = hit["type"].as_str().unwrap();
    let want_loc_bits = vec3_bits_expected(&hit["location_bits"]);
    let target = game.crosshair_target();

    match want_type {
        "ENTITY" => {
            let want_id = hit["entity_id"].as_i64().unwrap() as i32;
            match target {
                Some(CrosshairTarget::Entity { entity, point }) => {
                    if entity.id() as i32 != want_id {
                        failures.push(format!(
                            "{name}: crosshair entity id {} != vanilla {want_id}",
                            entity.id()
                        ));
                    }
                    let got_bits = point.map(f64::to_bits);
                    if got_bits != want_loc_bits {
                        failures.push(format!(
                            "{name}: entity hit location {:016x?} ({point:?}) != vanilla {:016x?} ({:?})",
                            got_bits,
                            want_loc_bits,
                            want_loc_bits.map(f64::from_bits)
                        ));
                    }
                }
                other => failures.push(format!(
                    "{name}: crosshair_target {other:?} != vanilla ENTITY {want_id}"
                )),
            }
            if game.crosshair_entity_id() != Some(want_id) {
                failures.push(format!(
                    "{name}: crosshair_entity_id {:?} != Some({want_id})",
                    game.crosshair_entity_id()
                ));
            }
            if !game.entity_targeted() {
                failures.push(format!(
                    "{name}: entity_targeted() was false for ENTITY {want_id}"
                ));
            }
            if game.spectator_click_target() != Some(want_id) {
                failures.push(format!(
                    "{name}: spectator_click_target {:?} != Some({want_id})",
                    game.spectator_click_target()
                ));
            }
        }
        "BLOCK" => {
            let bp = hit["block_pos"].as_array().unwrap();
            let want_pos = [
                bp[0].as_i64().unwrap() as i32,
                bp[1].as_i64().unwrap() as i32,
                bp[2].as_i64().unwrap() as i32,
            ];
            let want_face = face_from_name(hit["block_face"].as_str().unwrap());
            let want_inside = hit["inside"].as_bool().unwrap();
            match target {
                Some(CrosshairTarget::Block { pos, hit: bh, .. }) => {
                    let got_pos = [pos.x, pos.y, pos.z];
                    if got_pos != want_pos || bh.face != want_face || bh.inside != want_inside {
                        failures.push(format!(
                            "{name}: block hit ({got_pos:?}, {:?}, {}) != vanilla ({want_pos:?}, {want_face:?}, {want_inside})",
                            bh.face, bh.inside
                        ));
                    }
                    let got_bits = bh.point.map(f64::to_bits);
                    if got_bits != want_loc_bits {
                        failures.push(format!(
                            "{name}: block hit location {:016x?} ({:?}) != vanilla {:016x?} ({:?})",
                            got_bits,
                            bh.point,
                            want_loc_bits,
                            want_loc_bits.map(f64::from_bits)
                        ));
                    }
                }
                other => failures.push(format!(
                    "{name}: crosshair_target {other:?} != vanilla BLOCK {want_pos:?}"
                )),
            }
            if game.crosshair_entity_id().is_some() {
                failures.push(format!(
                    "{name}: crosshair_entity_id {:?} != None for BLOCK",
                    game.crosshair_entity_id()
                ));
            }
            if game.entity_targeted() {
                failures.push(format!("{name}: entity_targeted() was true for BLOCK"));
            }
            if game.spectator_click_target().is_some() {
                failures.push(format!(
                    "{name}: spectator_click_target {:?} != None for BLOCK",
                    game.spectator_click_target()
                ));
            }
        }
        "MISS" => {
            if target.is_some() {
                failures.push(format!(
                    "{name}: crosshair_target {target:?} != vanilla MISS"
                ));
            }
            if game.crosshair_entity_id().is_some() {
                failures.push(format!(
                    "{name}: crosshair_entity_id {:?} != None for MISS",
                    game.crosshair_entity_id()
                ));
            }
            if game.entity_targeted() {
                failures.push(format!("{name}: entity_targeted() was true for MISS"));
            }
            if game.spectator_click_target().is_some() {
                failures.push(format!(
                    "{name}: spectator_click_target {:?} != None for MISS",
                    game.spectator_click_target()
                ));
            }
        }
        other => panic!("unexpected hit type {other}"),
    }
}

/// Parity #32: primed TNT and falling blocks are pickable by the crosshair with exact 26.3 geometry.
#[test]
#[ignore = "daily"]
fn primed_tnt_and_falling_blocks_match_26_3_witness() {
    run_primed_tnt_and_falling_blocks_match_26_3_witness();
}

pub(crate) fn run_primed_tnt_and_falling_blocks_match_26_3_witness() {
    in_own_process(
        "cut_layout::game::client_side::crosshair_pick_tests::primed_tnt_and_falling_blocks_match_26_3_witness",
        || {
            let witness = crosshair_witness();
            let traits = &witness["entity_traits"];
            for key in ["tnt", "falling_block"] {
                assert_eq!(traits[key]["is_pickable"].as_bool(), Some(true), "{key}");
                assert_eq!(
                    traits[key]["can_be_picked_from_inside"].as_bool(),
                    Some(true),
                    "{key}"
                );
                assert_eq!(
                    traits[key]["has_pick_result"].as_bool(),
                    Some(false),
                    "{key}"
                );
                assert_eq!(
                    traits[key]["interact_result"].as_str(),
                    Some("Pass"),
                    "{key}"
                );
                assert_eq!(hex_f32(&traits[key]["pick_radius_bits"]), 0, "{key}");
            }

            let cases = witness["tnt_and_falling"].as_array().unwrap();
            assert_eq!(cases.len(), 18);
            let mut failures = Vec::new();

            for tc in cases {
                let name = tc["name"].as_str().unwrap();
                with_client_game(|game| {
                    let _server = replay_witness_case(game, tc);
                    check_case_hit(game, tc, &mut failures);

                    if tc["hit"]["type"].as_str() == Some("ENTITY") {
                        let hit_id = tc["hit"]["entity_id"].as_i64().unwrap() as i32;
                        let hit_kind = tc["entities"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|e| e["id"].as_i64().unwrap() as i32 == hit_id)
                            .unwrap()["kind"]
                            .as_str()
                            .unwrap();
                        if matches!(hit_kind, "minecraft:tnt" | "minecraft:falling_block") {
                            if game.pick_entity_hit() != Some(None) {
                                failures.push(format!(
                                    "{name}: pick_entity_hit {:?} != Some(None) for {hit_kind}",
                                    game.pick_entity_hit()
                                ));
                            }
                            if !game.within_entity_range(hit_id) {
                                failures.push(format!(
                                    "{name}: within_entity_range({hit_id}) was false for {hit_kind}"
                                ));
                            }
                            let swings_before = game.swings_started;
                            let consumed = game.remote_use_entity(Hand::Main);
                            if consumed || game.swings_started != swings_before {
                                failures.push(format!(
                                    "{name}: remote_use_entity must PASS without swinging for {hit_kind}"
                                ));
                            }
                        }
                    }
                });
            }

            assert!(
                failures.is_empty(),
                "{} #32 TNT/falling-block crosshair mismatches against real 26.3:\n{}",
                failures.len(),
                failures.join("\n")
            );
        },
    );
}

/// Parity #34: same-root-vehicle exclusion follows the real vehicle chain across nested rides and all kinds.
#[test]
#[ignore = "daily"]
fn nested_ride_root_vehicle_exclusion_matches_26_3_witness() {
    run_nested_ride_root_vehicle_exclusion_matches_26_3_witness();
}

pub(crate) fn run_nested_ride_root_vehicle_exclusion_matches_26_3_witness() {
    in_own_process(
        "cut_layout::game::client_side::crosshair_pick_tests::nested_ride_root_vehicle_exclusion_matches_26_3_witness",
        || {
            let witness = crosshair_witness();
            let cases = witness["nested_rides"].as_array().unwrap();
            assert_eq!(cases.len(), 19);
            let mut failures = Vec::new();

            for tc in cases {
                with_client_game(|game| {
                    let _server = replay_witness_case(game, tc);
                    check_case_hit(game, tc, &mut failures);
                });
            }

            assert!(
                failures.is_empty(),
                "{} #34 nested-ride crosshair mismatches against real 26.3:\n{}",
                failures.len(),
                failures.join("\n")
            );
        },
    );
}

/// Review parity: dragon wrapping IDs & tracking lifetime, spectator players, shulker bullet radius,
/// leash knots, section order, section moves, displaced camera, AttackRange, and strict reaches.
#[test]
#[ignore = "requires SUNRISE_CROSSHAIR_PICK_WITNESS"]
fn review_parity_cases_match_26_3_witness() {
    run_review_parity_cases_match_26_3_witness();
}

pub(crate) fn run_review_parity_cases_match_26_3_witness() {
    in_own_process(
        "cut_layout::game::client_side::crosshair_pick_tests::review_parity_cases_match_26_3_witness",
        || {
            let witness = crosshair_witness();
            let traits = &witness["entity_traits"];
            assert_eq!(
                traits["shulker_bullet"]["is_pickable"].as_bool(),
                Some(true)
            );
            assert_eq!(
                hex_f32(&traits["shulker_bullet"]["pick_radius_bits"]),
                1.0_f32.to_bits()
            );
            assert_eq!(traits["fireball"]["is_pickable"].as_bool(), Some(true));
            assert_eq!(
                hex_f32(&traits["fireball"]["pick_radius_bits"]),
                1.0_f32.to_bits()
            );
            assert_eq!(traits["arrow"]["is_pickable"].as_bool(), Some(false));
            assert_eq!(hex_f32(&traits["arrow"]["pick_radius_bits"]), 0);

            let cases = witness["review_parity"].as_array().unwrap();
            assert_eq!(cases.len(), 15);
            let mut failures = Vec::new();

            for tc in cases {
                with_client_game(|game| {
                    let _server = replay_witness_case(game, tc);
                    check_case_hit(game, tc, &mut failures);
                });
            }

            assert!(
                failures.is_empty(),
                "{} review-parity crosshair mismatches against real 26.3:\n{}",
                failures.len(),
                failures.join("\n")
            );
        },
    );
}

/// Finding 14: middle-clicking TNT or a falling block sends PickItemFromEntity (never PickItemFromBlock),
/// separately from the server's Entity.getPickResult returning None.
#[test]
#[ignore = "daily"]
fn middle_click_sends_pick_item_from_entity_for_tnt_and_falling_block() {
    run_middle_click_sends_pick_item_from_entity_for_tnt_and_falling_block();
}

pub(crate) fn run_middle_click_sends_pick_item_from_entity_for_tnt_and_falling_block() {
    in_own_process(
        "cut_layout::game::client_side::crosshair_pick_tests::middle_click_sends_pick_item_from_entity_for_tnt_and_falling_block",
        || {
            for (id, kind) in [(10, "minecraft:tnt"), (20, "minecraft:falling_block")] {
                with_client_game(|game| {
                    let server = setup_remote_overworld(game, 1, [0.5, 64.0, 0.5]);
                    let mut conn = Connection::local(server);
                    conn.reader
                        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                        .unwrap();
                    finish_login(game, &mut conn);
                    ensure_chunk(game, 0, 0);
                    let stone = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    let sand = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:sand")
                        .unwrap();
                    game.world.set(0, 65, 4, stone).unwrap();
                    let data = if kind == "minecraft:falling_block" {
                        sand.0 as i32
                    } else {
                        0
                    };
                    add_entity_with_data(game, id, kind, [0.5, 65.1, 2.5], 0.0, 0.0, data);
                    game.yaw = 0.0;
                    game.pitch = 0.0;
                    game.block_reach = 4.5;
                    game.entity_reach = 3.0;
                    game.refresh_client_pick(1.0);
                    assert_eq!(game.crosshair_entity_id(), Some(id));
                    assert!(
                        game.pick_result_of(id).is_none(),
                        "{kind} has no server getPickResult"
                    );
                    game.test_remote_pick_click();
                    game.send(play::Serverbound::Punch(sunrise_protocol::packets::Empty));
                    let (pkt_id, body) = conn.recv().expect("PickItemFromEntity packet");
                    let decoded = play::Serverbound::decode(pkt_id, &body).unwrap();
                    assert!(
                        matches!(&decoded, play::Serverbound::PickItemFromEntity(p) if p.id == id),
                        "expected PickItemFromEntity({id}) for {kind}, got {decoded:?}"
                    );
                    let (pkt_id, body) = conn.recv().expect("play packet fence");
                    let decoded = play::Serverbound::decode(pkt_id, &body).unwrap();
                    assert!(
                        matches!(decoded, play::Serverbound::Punch(_)),
                        "expected the fence after the entity pick for {kind}, got {decoded:?}"
                    );
                });
            }
        },
    );
}

/// Finding 4: synced `#minecraft:redirectable_projectile` tags make a flying arrow pickable with
/// 1.0 radius, and `AbstractArrow.isInGround` stops picking it once stuck.
#[test]
#[ignore = "daily"]
fn dynamic_redirectable_tag_controls_arrow_pick_and_in_ground_stops_it() {
    run_dynamic_redirectable_tag_controls_arrow_pick_and_in_ground_stops_it();
}

pub(crate) fn run_dynamic_redirectable_tag_controls_arrow_pick_and_in_ground_stops_it() {
    in_own_process(
        "cut_layout::game::client_side::crosshair_pick_tests::dynamic_redirectable_tag_controls_arrow_pick_and_in_ground_stops_it",
        || {
            with_client_game(|game| {
                let _server = setup_remote_overworld(game, 1, [0.5, 64.0, 0.5]);
                ensure_chunk(game, 0, 0);
                // Place arrow 0.8 blocks off-axis so only 1.0 pick radius hits it.
                add_entity_with_data(game, 40, "minecraft:arrow", [1.3, 65.5, 2.5], 0.0, 0.0, 0);
                game.yaw = 0.0;
                game.pitch = 0.0;
                game.block_reach = 4.5;
                game.entity_reach = 3.0;
                game.refresh_client_pick(1.0);
                assert_eq!(
                    game.crosshair_entity_id(),
                    None,
                    "default tags do not include arrow in #minecraft:redirectable_projectile"
                );
                let arrow_type = entity_type_id(game, "minecraft:arrow");
                game.handle_packet(C::UpdateTags(UpdateTags {
                    registries: vec![RegistryTags {
                        registry: "minecraft:entity_type".into(),
                        tags: vec![TagEntry {
                            name: "minecraft:redirectable_projectile".into(),
                            ids: vec![arrow_type],
                        }],
                    }],
                }))
                .unwrap();
                game.refresh_client_pick(1.0);
                assert_eq!(
                    game.crosshair_entity_id(),
                    Some(40),
                    "flying arrow in synced #minecraft:redirectable_projectile is picked with 1.0 radius"
                );
                game.handle_packet(C::SetEntityData(play::SetEntityData {
                    id: 40,
                    values: vec![(10, 8, EntityDataValue::Bool(true))],
                }))
                .unwrap();
                game.refresh_client_pick(1.0);
                assert_eq!(
                    game.crosshair_entity_id(),
                    None,
                    "in-ground arrow is no longer pickable"
                );
            });
        },
    );
}

/// Parity #32 and #34: singleplayer/unstored pick paths share the unified pickable query and root exclusion.
#[test]
#[ignore = "daily"]
fn singleplayer_and_unstored_pick_share_unified_query_and_root_exclusion_26_3() {
    run_singleplayer_and_unstored_pick_share_unified_query_and_root_exclusion_26_3();
}

pub(crate) fn run_singleplayer_and_unstored_pick_share_unified_query_and_root_exclusion_26_3() {
    in_own_process(
        "cut_layout::game::client_side::crosshair_pick_tests::singleplayer_and_unstored_pick_share_unified_query_and_root_exclusion_26_3",
        || {
            with_game(|game| {
                assert!(
                    !game.side_uses_stored_pick(),
                    "with_game exercises the unstored server pick path"
                );
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                game.block_reach = 4.5;
                game.entity_reach = 3.0;
                let registry = game.pack.assets.registry;
                let stone = registry.default_state("minecraft:stone").unwrap();
                let sand = registry.default_state("minecraft:sand").unwrap();
                let air = registry.default_state("minecraft:air").unwrap();

                let place_player = |game: &mut sunrise_gameplay::game::ServerGame<'_>,
                                    pos: [f64; 3],
                                    yaw: f32,
                                    pitch: f32| {
                    let v = Vec3::new(pos[0], pos[1], pos[2]);
                    let mut p = PlayerMotion::new(game.motion, v).unwrap();
                    p.entity_id = 1;
                    game.player = Some(p);
                    game.previous_position = v;
                    game.yaw = yaw;
                    game.pitch = pitch;
                };

                // A: server primed TNT occludes stone behind it and reports no getPickResult.
                place_player(game, [0.5, 64.0, 0.5], 0.0, 0.0);
                game.world.set(0, 65, 4, stone).unwrap();
                let tnt_pos = Vec3::new(0.5, 65.1, 2.5);
                game.tnts.push(PrimedTnt {
                    size_answer: None,
                    attachment_lifetime: Default::default(),
                    rotation: [0.0, 0.0],
                    join: Default::default(),
                    no_gravity: false,
                    id: 10,
                    position: tnt_pos,
                    previous: tnt_pos,
                    velocity: Vec3::ZERO,
                    fuse: 80,
                    ticks: 0,
                    ground: Default::default(),
                    fall_distance: 0.0,
                    stuck: Vec3::ZERO,
                    in_water: false,
                    owner: None,
                    guest: None,
                    power: 4.0,
                    used_portal: false,
                    block_state: None,
                });
                let hit = game.server_crosshair_entity_hit().expect("server TNT hit");
                assert_eq!(hit.id, 10, "singleplayer TNT crosshair_entity_id");
                assert!(!hit.pickable, "singleplayer TNT has no pick result");
                assert!(game.pick().is_none(), "singleplayer TNT occludes stone");
                assert!(
                    game.within_entity_range(10),
                    "singleplayer TNT within_entity_range"
                );

                // B: server falling sand occludes stone behind it and reports no getPickResult.
                game.tnts.clear();
                let fb_pos = Vec3::new(0.5, 65.1, 2.5);
                game.falling.push(FallingBlock {
                    size_answer: None,
                    attachment_lifetime: Default::default(),
                    rotation: [0.0, 0.0],
                    join: Default::default(),
                    id: 20,
                    state: sand,
                    blocks_building: true,
                    start_pos: BlockPos::new(0, 65, 2),
                    position: fb_pos,
                    previous: fb_pos,
                    velocity: Vec3::ZERO,
                    age: 0,
                    no_gravity: false,
                    portal: Default::default(),
                    drop_item: false,
                    fall_distance: 0.0,
                    ground: Default::default(),
                    fire_ticks: 0,
                    sync_velocity: false,
                    stuck: Vec3::ZERO,
                    damage: Default::default(),
                    saved: None,
                });
                let hit = game
                    .server_crosshair_entity_hit()
                    .expect("server falling block hit");
                assert_eq!(hit.id, 20, "singleplayer falling block crosshair_entity_id");
                assert!(
                    !hit.pickable,
                    "singleplayer falling block has no pick result"
                );
                assert!(
                    game.pick().is_none(),
                    "singleplayer falling block occludes stone"
                );
                assert!(
                    game.within_entity_range(20),
                    "singleplayer falling block within_entity_range"
                );

                // C: unmounted server player carrying a pig excludes the pig and hits ceiling stone.
                game.falling.clear();
                game.world.set(0, 65, 4, air).unwrap();
                game.world.set(0, 68, 0, stone).unwrap();
                game.riding = None;
                place_player(game, [0.5, 64.0, 0.5], 0.0, -90.0);
                let pig_type = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                let mut pig = Mob::new(150, pig_type, Vec3::new(0.5, 66.2, 0.5), 0.0);
                pig.extra.mount = Some(1);
                pig.extra.passenger = true;
                game.mobs.push(pig);
                assert!(
                    game.server_crosshair_entity_hit().is_none(),
                    "pig riding unmounted player must be excluded"
                );
                assert_eq!(game.pick().map(|h| h.pos), Some([0, 68, 0]));

                // D: server camel rider with eye inside camel box looks through co-passenger husk to external TNT.
                game.mobs.clear();
                game.world.set(0, 68, 0, air).unwrap();
                let camel_type = game.res.mob_types().get("minecraft:camel").unwrap().clone();
                let husk_type = game.res.mob_types().get("minecraft:husk").unwrap().clone();
                game.mobs
                    .push(Mob::new(130, camel_type, Vec3::new(0.5, 64.0, 0.5), 0.0));
                let mut husk = Mob::new(131, husk_type, Vec3::new(0.5, 64.17, 1.0), 0.0);
                husk.extra.mount = Some(130);
                husk.extra.passenger = true;
                game.mobs.push(husk);
                let behind_tnt = Vec3::new(0.5, 65.3, 2.5);
                game.tnts.push(PrimedTnt {
                    size_answer: None,
                    attachment_lifetime: Default::default(),
                    rotation: [0.0, 0.0],
                    join: Default::default(),
                    no_gravity: false,
                    id: 201,
                    position: behind_tnt,
                    previous: behind_tnt,
                    velocity: Vec3::ZERO,
                    fuse: 80,
                    ticks: 0,
                    ground: Default::default(),
                    fall_distance: 0.0,
                    stuck: Vec3::ZERO,
                    in_water: false,
                    owner: None,
                    guest: None,
                    power: 4.0,
                    used_portal: false,
                    block_state: None,
                });
                game.riding = Some(130);
                place_player(game, [0.5, 64.17, 0.0], 0.0, 0.0);
                assert_eq!(
                    game.server_crosshair_entity_hit().map(|h| h.id),
                    Some(201),
                    "unified unstored pick must pick TNT past camel co-passenger"
                );

                // E (server): server horse excludes ArmorStand co-passenger, picking external falling block.
                game.mobs.clear();
                game.tnts.clear();
                let horse_type = game.res.mob_types().get("minecraft:horse").unwrap().clone();
                game.mobs
                    .push(Mob::new(110, horse_type, Vec3::new(0.5, 64.0, 0.5), 0.0));
                let mut stand = ArmorStand::new(112, Vec3::new(0.5, 65.6, 1.4), 0.0);
                stand.vehicle = Some(110);
                game.armor_stands.push(stand);
                let behind_fb = Vec3::new(0.5, 66.1, 2.9);
                game.falling.push(FallingBlock {
                    size_answer: None,
                    attachment_lifetime: Default::default(),
                    rotation: [0.0, 0.0],
                    join: Default::default(),
                    id: 202,
                    state: sand,
                    blocks_building: true,
                    start_pos: BlockPos::new(0, 66, 2),
                    position: behind_fb,
                    previous: behind_fb,
                    velocity: Vec3::ZERO,
                    age: 0,
                    no_gravity: false,
                    portal: Default::default(),
                    drop_item: false,
                    fall_distance: 0.0,
                    ground: Default::default(),
                    fire_ticks: 0,
                    sync_velocity: false,
                    stuck: Vec3::ZERO,
                    damage: Default::default(),
                    saved: None,
                });
                game.riding = Some(110);
                place_player(game, [0.5, 65.0, 0.5], 0.0, 0.0);
                assert_eq!(
                    game.server_crosshair_entity_hit().map(|h| h.id),
                    Some(202),
                    "horse co-passenger stand must be excluded"
                );

                // F (Finding 9): out-of-reach TNT (at 3.2) in front of chest (at 3.5) yields MISS,
                // and a block at exact block_reach (4.5) fails strict `<`.
                game.mobs.clear();
                game.armor_stands.clear();
                game.falling.clear();
                game.tnts.clear();
                game.riding = None;
                let chest = registry.default_state("minecraft:chest").unwrap();
                place_player(game, [0.5, 64.0, 0.5], 0.0, 0.0);
                game.world.set(0, 65, 4, chest).unwrap();
                let far_tnt = Vec3::new(0.5, 65.1, 4.19);
                game.tnts.push(PrimedTnt {
                    size_answer: None,
                    attachment_lifetime: Default::default(),
                    rotation: [0.0, 0.0],
                    join: Default::default(),
                    no_gravity: false,
                    id: 210,
                    position: far_tnt,
                    previous: far_tnt,
                    velocity: Vec3::ZERO,
                    fuse: 80,
                    ticks: 0,
                    ground: Default::default(),
                    fall_distance: 0.0,
                    stuck: Vec3::ZERO,
                    in_water: false,
                    owner: None,
                    guest: None,
                    power: 4.0,
                    used_portal: false,
                    block_state: None,
                });
                assert!(game.server_crosshair_entity_hit().is_none());
                assert!(
                    game.pick().is_none(),
                    "out-of-reach TNT occluding chest must not fall back to chest on unstored pick()"
                );
                game.tnts.clear();
                game.world.set(0, 65, 4, air).unwrap();
                game.world.set(0, 65, 5, stone).unwrap();
                assert!(
                    game.pick().is_none(),
                    "block at exact block_reach (4.5) must fail strict <"
                );
                game.world.set(0, 65, 5, air).unwrap();

                // G (Finding 2): attacking with TNT or falling block in front of a zombie must not hurt it.
                let zombie_type = game
                    .res
                    .mob_types()
                    .get("minecraft:zombie")
                    .unwrap()
                    .clone();
                let zombie = Mob::new(220, zombie_type, Vec3::new(0.5, 64.5, 2.8), 0.0);
                let zombie_hp = zombie.health;
                game.mobs.push(zombie);
                let shield_tnt = Vec3::new(0.5, 65.1, 1.8);
                game.tnts.push(PrimedTnt {
                    size_answer: None,
                    attachment_lifetime: Default::default(),
                    rotation: [0.0, 0.0],
                    join: Default::default(),
                    no_gravity: false,
                    id: 221,
                    position: shield_tnt,
                    previous: shield_tnt,
                    velocity: Vec3::ZERO,
                    fuse: 80,
                    ticks: 0,
                    ground: Default::default(),
                    fall_distance: 0.0,
                    stuck: Vec3::ZERO,
                    in_water: false,
                    owner: None,
                    guest: None,
                    power: 4.0,
                    used_portal: false,
                    block_state: None,
                });
                game.attack();
                assert_eq!(
                    game.mobs[0].health, zombie_hp,
                    "TNT in front must shield zombie from melee attack"
                );
                game.tnts.clear();
                game.falling.push(FallingBlock {
                    size_answer: None,
                    attachment_lifetime: Default::default(),
                    rotation: [0.0, 0.0],
                    join: Default::default(),
                    id: 222,
                    state: sand,
                    blocks_building: true,
                    start_pos: BlockPos::new(0, 65, 1),
                    position: shield_tnt,
                    previous: shield_tnt,
                    velocity: Vec3::ZERO,
                    age: 0,
                    no_gravity: false,
                    portal: Default::default(),
                    drop_item: false,
                    fall_distance: 0.0,
                    ground: Default::default(),
                    fire_ticks: 0,
                    sync_velocity: false,
                    stuck: Vec3::ZERO,
                    damage: Default::default(),
                    saved: None,
                });
                game.attack();
                assert_eq!(
                    game.mobs[0].health, zombie_hp,
                    "falling block in front must shield zombie from melee attack"
                );
                game.falling.clear();
                game.mobs.clear();

                // H (Finding 12): named interaction reaches a same-root co-passenger without ray root rejection.
                let camel_type = game.res.mob_types().get("minecraft:camel").unwrap().clone();
                let villager_type = game
                    .res
                    .mob_types()
                    .get("minecraft:villager")
                    .unwrap()
                    .clone();
                game.mobs
                    .push(Mob::new(230, camel_type, Vec3::new(0.5, 64.0, 0.5), 0.0));
                let mut copassenger = Mob::new(231, villager_type, Vec3::new(0.5, 64.5, 1.0), 0.0);
                copassenger.extra.mount = Some(230);
                copassenger.extra.passenger = true;
                game.mobs.push(copassenger);
                game.riding = Some(230);
                assert_eq!(
                    game.mob_interaction_target(Some(InteractTarget {
                        id: 231,
                        location: [0.0; 3],
                    })),
                    Some(1),
                    "named interaction must reach a same-root co-passenger"
                );

                // I (Finding 13): vehicle-root walk crosses retained off-level links past resident row count.
                game.mobs.clear();
                game.persist.off_level.clear();
                let pig_type = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                // Only 1 resident mob (id 305), linked through 5 retained off-level mobs (304 -> ... -> 300).
                for link in 300..305 {
                    let mut retained =
                        Mob::new(link, pig_type.clone(), Vec3::new(0.5, 64.0, 0.5), 0.0);
                    if link > 300 {
                        retained.extra.mount = Some(link - 1);
                        retained.extra.passenger = true;
                    }
                    game.persist.off_level.insert(link, OffLevel::Mob(retained));
                }
                let mut resident_rider = Mob::new(305, pig_type, Vec3::new(0.5, 65.0, 2.0), 0.0);
                resident_rider.extra.mount = Some(304);
                resident_rider.extra.passenger = true;
                game.mobs.push(resident_rider);
                game.riding = Some(300);
                place_player(game, [0.5, 64.0, 0.5], 0.0, 0.0);
                assert!(
                    game.side_pick_shares_vehicle(305),
                    "vehicle-root walk must traverse retained off-level links past resident count"
                );
                assert!(
                    game.server_crosshair_entity_hit().is_none(),
                    "same-root rider linked through retained off-level mobs must be excluded"
                );
            });

            with_client_game(|game| {
                let _pipe = sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client(game);
                game.client.remote.as_mut().unwrap().entity_id = 1;
                ensure_chunk(game, 0, 0);
                game.client.title.0 = None;
                game.block_reach = 4.5;
                game.entity_reach = 3.0;
                let registry = game.pack.assets.registry;
                let stone = registry.default_state("minecraft:stone").unwrap();
                let sand = registry.default_state("minecraft:sand").unwrap();
                let air = registry.default_state("minecraft:air").unwrap();

                let place_player = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
                                    pos: [f64; 3],
                                    yaw: f32,
                                    pitch: f32| {
                    let v = Vec3::new(pos[0], pos[1], pos[2]);
                    let mut p = PlayerMotion::new(game.motion, v).unwrap();
                    p.entity_id = 1;
                    game.player = Some(p);
                    game.previous_position = v;
                    game.yaw = yaw;
                    game.pitch = pitch;
                };
                let track = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
                             id: i32,
                             kind: &str,
                             position: [f64; 3],
                             vehicle: Option<u32>| {
                    game.client.remote.as_mut().unwrap().entities.insert(
                        id,
                        sunrise_client_gameplay::game::client_side::remote_client::session_state::Tracked {
                            kind: kind.into(),
                            position,
                            previous: position,
                            yaw: 0.0,
                            pitch: 0.0,
                            previous_yaw: 0.0,
                            previous_pitch: 0.0,
                            codec_base: position,
                            on_fire: false,
                            mod_fire_immune: false,
                            silent: false,
                            vehicle,
                            tick_count: 0,
                            size_answer: None,
                        },
                    );
                };

                // A (client): primed TNT occludes stone behind it and reports no getPickResult.
                place_player(game, [0.5, 64.0, 0.5], 0.0, 0.0);
                game.world.set(0, 65, 4, stone).unwrap();
                let tnt_pos = Vec3::new(0.5, 65.1, 2.5);
                game.tnts.push(PrimedTnt {
                    size_answer: None,
                    attachment_lifetime: Default::default(),
                    rotation: [0.0, 0.0],
                    join: Default::default(),
                    no_gravity: false,
                    id: 10,
                    position: tnt_pos,
                    previous: tnt_pos,
                    velocity: Vec3::ZERO,
                    fuse: 80,
                    ticks: 0,
                    ground: Default::default(),
                    fall_distance: 0.0,
                    stuck: Vec3::ZERO,
                    in_water: false,
                    owner: None,
                    guest: None,
                    power: 4.0,
                    used_portal: false,
                    block_state: None,
                });
                track(game, 10, "minecraft:tnt", [0.5, 65.1, 2.5], None);
                game.refresh_client_pick(1.0);
                assert_eq!(
                    game.crosshair_entity_id(),
                    Some(10),
                    "client TNT crosshair_entity_id"
                );
                assert!(game.entity_targeted(), "client TNT entity_targeted");
                assert_eq!(
                    game.spectator_click_target(),
                    Some(10),
                    "client TNT spectator_click_target"
                );
                assert_eq!(
                    game.pick_entity_hit(),
                    Some(None),
                    "client TNT has no pick result"
                );
                assert!(
                    game.within_entity_range(10),
                    "client TNT within_entity_range"
                );

                // B (client): falling sand occludes stone behind it and reports no getPickResult.
                game.tnts.clear();
                let fb_pos = Vec3::new(0.5, 65.1, 2.5);
                game.falling.push(FallingBlock {
                    size_answer: None,
                    attachment_lifetime: Default::default(),
                    rotation: [0.0, 0.0],
                    join: Default::default(),
                    id: 20,
                    state: sand,
                    blocks_building: true,
                    start_pos: BlockPos::new(0, 65, 2),
                    position: fb_pos,
                    previous: fb_pos,
                    velocity: Vec3::ZERO,
                    age: 0,
                    no_gravity: false,
                    portal: Default::default(),
                    drop_item: false,
                    fall_distance: 0.0,
                    ground: Default::default(),
                    fire_ticks: 0,
                    sync_velocity: false,
                    stuck: Vec3::ZERO,
                    damage: Default::default(),
                    saved: None,
                });
                track(game, 20, "minecraft:falling_block", [0.5, 65.1, 2.5], None);
                game.refresh_client_pick(1.0);
                assert_eq!(
                    game.crosshair_entity_id(),
                    Some(20),
                    "client falling block crosshair_entity_id"
                );
                assert!(
                    game.entity_targeted(),
                    "client falling block entity_targeted"
                );
                assert_eq!(
                    game.spectator_click_target(),
                    Some(20),
                    "client falling block spectator_click_target"
                );
                assert_eq!(
                    game.pick_entity_hit(),
                    Some(None),
                    "client falling block has no pick result"
                );
                assert!(
                    game.within_entity_range(20),
                    "client falling block within_entity_range"
                );

                // C (client): unmounted player carrying a pig excludes the pig and hits ceiling stone.
                game.falling.clear();
                game.world.set(0, 65, 4, air).unwrap();
                game.world.set(0, 68, 0, stone).unwrap();
                game.riding = None;
                place_player(game, [0.5, 64.0, 0.5], 0.0, -90.0);
                let pig_type = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                game.mobs
                    .push(Mob::new(150, pig_type, Vec3::new(0.5, 66.2, 0.5), 0.0));
                game.client
                    .remote
                    .as_mut()
                    .unwrap()
                    .passenger_vehicles
                    .insert(
                        150,
                        sunrise_gameplay_common::game::remote::passengers::PassengerSeat {
                            vehicle: 1,
                            index: 0,
                        },
                    );
                game.refresh_client_pick(1.0);
                assert_eq!(
                    game.crosshair_entity_id(),
                    None,
                    "pig riding unmounted client player must be excluded"
                );
                assert!(
                    !game.entity_targeted(),
                    "pig riding unmounted client player must not be targeted"
                );
                assert_eq!(
                    game.spectator_click_target(),
                    None,
                    "pig riding unmounted client player must not be spectator target"
                );
                assert_eq!(game.pick().map(|h| h.pos), Some([0, 68, 0]));

                // D (client): camel rider looks through co-passenger husk to external TNT.
                game.mobs.clear();
                game.world.set(0, 68, 0, air).unwrap();
                let camel_type = game.res.mob_types().get("minecraft:camel").unwrap().clone();
                let husk_type = game.res.mob_types().get("minecraft:husk").unwrap().clone();
                game.mobs
                    .push(Mob::new(130, camel_type, Vec3::new(0.5, 64.0, 0.5), 0.0));
                game.mobs
                    .push(Mob::new(131, husk_type, Vec3::new(0.5, 64.17, 1.0), 0.0));
                game.client
                    .remote
                    .as_mut()
                    .unwrap()
                    .passenger_vehicles
                    .insert(
                        131,
                        sunrise_gameplay_common::game::remote::passengers::PassengerSeat {
                            vehicle: 130,
                            index: 1,
                        },
                    );
                let behind_tnt = Vec3::new(0.5, 65.3, 2.5);
                game.tnts.push(PrimedTnt {
                    size_answer: None,
                    attachment_lifetime: Default::default(),
                    rotation: [0.0, 0.0],
                    join: Default::default(),
                    no_gravity: false,
                    id: 201,
                    position: behind_tnt,
                    previous: behind_tnt,
                    velocity: Vec3::ZERO,
                    fuse: 80,
                    ticks: 0,
                    ground: Default::default(),
                    fall_distance: 0.0,
                    stuck: Vec3::ZERO,
                    in_water: false,
                    owner: None,
                    guest: None,
                    power: 4.0,
                    used_portal: false,
                    block_state: None,
                });
                track(game, 201, "minecraft:tnt", [0.5, 65.3, 2.5], None);
                game.riding = Some(130);
                place_player(game, [0.5, 64.17, 0.0], 0.0, 0.0);
                game.refresh_client_pick(1.0);
                assert_eq!(
                    game.crosshair_entity_id(),
                    Some(201),
                    "client pick must pick TNT past camel co-passenger"
                );
                assert_eq!(
                    game.spectator_click_target(),
                    Some(201),
                    "client spectator_click_target must pick TNT past camel co-passenger"
                );
                assert!(game.entity_targeted());

                // E (client): connected client horse excludes RemoteInteraction co-passenger.
                game.mobs.clear();
                game.tnts.clear();
                let horse_type = game.res.mob_types().get("minecraft:horse").unwrap().clone();
                game.mobs
                    .push(Mob::new(110, horse_type, Vec3::new(0.5, 64.0, 0.5), 0.0));
                let behind_fb = Vec3::new(0.5, 66.1, 2.9);
                game.falling.push(FallingBlock {
                    size_answer: None,
                    attachment_lifetime: Default::default(),
                    rotation: [0.0, 0.0],
                    join: Default::default(),
                    id: 202,
                    state: sand,
                    blocks_building: true,
                    start_pos: BlockPos::new(0, 66, 2),
                    position: behind_fb,
                    previous: behind_fb,
                    velocity: Vec3::ZERO,
                    age: 0,
                    no_gravity: false,
                    portal: Default::default(),
                    drop_item: false,
                    fall_distance: 0.0,
                    ground: Default::default(),
                    fire_ticks: 0,
                    sync_velocity: false,
                    stuck: Vec3::ZERO,
                    damage: Default::default(),
                    saved: None,
                });
                track(game, 202, "minecraft:falling_block", [0.5, 66.1, 2.9], None);
                let mut inter = RemoteInteraction::new([0.5, 66.1, 1.9]);
                inter.vehicle = Some(110);
                inter.dimensions_refreshed = true;
                game.client.interactions.insert(113, inter);
                game.riding = Some(110);
                place_player(game, [0.5, 65.0, 0.5], 0.0, 0.0);
                track(
                    game,
                    113,
                    "minecraft:interaction",
                    [0.5, 66.1, 1.9],
                    Some(110),
                );
                game.client
                    .remote
                    .as_mut()
                    .unwrap()
                    .passenger_vehicles
                    .insert(
                        113,
                        sunrise_gameplay_common::game::remote::passengers::PassengerSeat {
                            vehicle: 110,
                            index: 1,
                        },
                    );
                game.refresh_client_pick(1.0);
                assert_eq!(
                    game.crosshair_entity_id(),
                    Some(202),
                    "client horse co-passenger interaction must be excluded"
                );
                assert_eq!(
                    game.spectator_click_target(),
                    Some(202),
                    "client spectator_click_target must skip co-passenger interaction"
                );

                // F (client): out-of-reach TNT occluding chest yields MISS; exact block_reach fails strict `<`.
                game.mobs.clear();
                game.client.interactions.clear();
                game.falling.clear();
                game.tnts.clear();
                game.riding = None;
                let chest = registry.default_state("minecraft:chest").unwrap();
                place_player(game, [0.5, 64.0, 0.5], 0.0, 0.0);
                game.world.set(0, 65, 4, chest).unwrap();
                let far_tnt = Vec3::new(0.5, 65.1, 4.19);
                game.tnts.push(PrimedTnt {
                    size_answer: None,
                    attachment_lifetime: Default::default(),
                    rotation: [0.0, 0.0],
                    join: Default::default(),
                    no_gravity: false,
                    id: 210,
                    position: far_tnt,
                    previous: far_tnt,
                    velocity: Vec3::ZERO,
                    fuse: 80,
                    ticks: 0,
                    ground: Default::default(),
                    fall_distance: 0.0,
                    stuck: Vec3::ZERO,
                    in_water: false,
                    owner: None,
                    guest: None,
                    power: 4.0,
                    used_portal: false,
                    block_state: None,
                });
                track(game, 210, "minecraft:tnt", [0.5, 65.1, 4.19], None);
                game.refresh_client_pick(1.0);
                assert_eq!(game.crosshair_entity_id(), None);
                assert!(
                    game.pick().is_none(),
                    "out-of-reach TNT occluding chest must not fall back to chest on client pick()"
                );
                game.tnts.clear();
                game.world.set(0, 65, 4, air).unwrap();
                game.world.set(0, 65, 5, stone).unwrap();
                game.refresh_client_pick(1.0);
                assert!(
                    game.pick().is_none(),
                    "block at exact block_reach (4.5) must fail strict <"
                );
            });
        },
    );
}

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;

use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;

use sunrise_gameplay_common::game::pick_entity_common::PickEntityCommonGameExt1 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
