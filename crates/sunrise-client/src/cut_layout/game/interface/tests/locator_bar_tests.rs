use std::time::Duration;
use sunrise_assets::resource_source::{Resource, ResourceSource};
use sunrise_client::{level_loading::Reason, locator_bar::WaypointStyles};
use sunrise_client_render::hud_layout::GuiDraw;
use sunrise_core::geometry::Vec3;
use sunrise_protocol::packets::play::TrackedWaypoint;
use sunrise_protocol::{
    connection::Pipe,
    packets::play::{
        self, Clientbound as C, SpawnInfo, WaypointId, WaypointTarget, waypoint_operation,
    },
};
use sunrise_sim::motion::PlayerMotion;
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry,
    sunrise_gameplay_common::game::in_own_process,
    sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycle,
};

fn connect(game: &mut sunrise_client_gameplay::game::ClientGame<'_>, position: Vec3) -> Pipe {
    game.native_lifecycle = NativeLifecycle::default();
    let (client, server) = Pipe::pair();
    game.connect_local(client, "LocatorTest").unwrap();
    game.enter_remote_dimension(
        &SpawnInfo {
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
        },
        RemoteDimensionEntry::Login,
        Reason::Other,
    );
    let remote = game.client.remote.as_mut().unwrap();
    remote.logged_in = true;
    remote.entity_id = 1;
    remote.level_load.finish();
    game.player = Some(PlayerMotion::new(game.motion, position).unwrap());
    game.previous_position = position;
    server
}

fn add_armor_stand(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    id: i32,
    uuid: u128,
    position: [f64; 3],
) {
    add_armor_stand_facing(game, id, uuid, position, 0);
}

fn add_armor_stand_facing(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    id: i32,
    uuid: u128,
    position: [f64; 3],
    yaw: i8,
) {
    let kind = game
        .client
        .remote
        .as_ref()
        .unwrap()
        .builtin
        .get("minecraft:entity_type")
        .unwrap()
        .id("minecraft:armor_stand")
        .unwrap();
    game.apply_packet(C::AddEntity(play::AddEntity {
        id,
        uuid,
        kind,
        position,
        velocity: [0.0; 3],
        pitch: 0,
        yaw,
        head_yaw: yaw,
        data: 0,
    }))
    .unwrap();
}

fn track_block(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    uuid: u128,
    color: i32,
    position: [i32; 3],
) {
    game.handle_waypoint(&TrackedWaypoint {
        operation: waypoint_operation::TRACK,
        id: WaypointId::Uuid(uuid),
        style: "minecraft:test".into(),
        color: Some(-16_777_216 | color),
        target: WaypointTarget::Block(position),
    });
}

#[test]
#[ignore = "daily"]
fn spectated_locator_uses_camera_entity_for_identity_and_distance() {
    in_own_process(
        "cut_layout::game::client_side::locator_bar::tests::spectated_locator_uses_camera_entity_for_identity_and_distance",
        || {
            with_client_game(|game| {
                let _server = connect(game, Vec3::new(0.0, 64.0, 0.0));
                let local_uuid = game.own_uuid();
                add_armor_stand(game, 20, 2, [100.0, 64.0, 0.0]);
                game.apply_packet(C::SetCamera(play::SetCamera { camera_id: 20 }))
                    .unwrap();

                // Keep all three witness waypoints visible by placing the render camera
                // behind them; the entity itself still faces south at x = 100.
                game.client.locator_camera = Some(([-10.0, 66.0, 0.0], -90.0, 45.0, 70.0));
                game.client.waypoint_styles = WaypointStyles::load(&ResourceSource::indexed(
                    [(
                        "assets/minecraft/waypoint_style/test.json".into(),
                        Resource::bytes(
                            br#"{"near_distance":30,"far_distance":80,"sprites":["near","middle","far"]}"#
                                .to_vec(),
                        ),
                    )]
                    .into(),
                ));
                track_block(game, 3, 3, [122, 64, 6]);
                track_block(game, 2, 2, [100, 64, 0]);
                track_block(game, local_uuid, 1, [0, 64, 0]);
                track_block(game, 4, 4, [150, 64, 0]);

                let assert_draws = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
                    let draws = game.locator_bar_draws(427, 240);
                    let sprites: Vec<_> = draws
                        .iter()
                        .filter_map(|d| match d {
                            GuiDraw::Sprite {
                                color, sprite, x, ..
                            } => Some((color & 0xff, sprite.as_ref(), *x)),
                            _ => None,
                        })
                        .collect();
                    assert_eq!(
                        sprites,
                        [
                            (1, "hud/locator_bar_dot/far", 209),
                            (255, "hud/locator_bar_arrow_up", 210),
                            (4, "hud/locator_bar_dot/middle", 209),
                            (255, "hud/locator_bar_arrow_up", 210),
                            (3, "hud/locator_bar_dot/near", 213),
                            (255, "hud/locator_bar_arrow_up", 214),
                        ]
                    );
                };
                assert_draws(game);
                // Removing the entity from the level does not replace Minecraft's camera.
                game.apply_packet(C::RemoveEntities(play::RemoveEntities { ids: vec![20] }))
                    .unwrap();
                assert_draws(game);

                // First frame after SetCamera switches to a stand facing east (-90 deg, packed -64):
                // extract_frame_hud updates locator_camera before HUD extraction so placement and
                // distance both use the new camera rather than the previous frame's north-facing camera.
                use sunrise_client::hud::Rect;
                let near_rect = Rect {
                    u0: 0.25,
                    v0: 0.0,
                    u1: 0.5,
                    v1: 0.25,
                    w: 9,
                    h: 9,
                };
                game.client
                    .gui
                    .atlas
                    .sprites
                    .insert("hud/locator_bar_dot/near".into(), near_rect);
                game.client.gui.atlas.sprites.insert(
                    "hud/locator_bar_dot/far".into(),
                    Rect {
                        u0: 0.75,
                        v0: 0.0,
                        u1: 1.0,
                        v1: 0.25,
                        w: 9,
                        h: 9,
                    },
                );
                game.clear_waypoints();
                track_block(game, 5, 5, [122, 64, 0]);
                game.client.chat_ui.hide_gui = false;
                game.client.locator_camera = Some(([0.0, 65.62, 0.0], 180.0, 0.0, 70.0));
                add_armor_stand_facing(game, 25, 25, [100.5, 64.0, 0.5], -64);
                game.apply_packet(C::SetCamera(play::SetCamera { camera_id: 25 }))
                    .unwrap();
                let (cam_pos, cam_yaw, cam_pitch) = game.camera_view(0.0).unwrap();
                let cam_fov = game.fov_degrees(0.0, cam_pos.to_array());
                let batch = game.extract_frame_hud(
                    [427, 240],
                    None,
                    false,
                    cam_pos.to_array(),
                    cam_yaw,
                    cam_pitch,
                    cam_fov,
                );
                let near_dots: Vec<i32> = batch
                    .vertices
                    .as_chunks::<6>()
                    .0
                    .iter()
                    .filter(|q| {
                        q[0].uv == [near_rect.u0, near_rect.v0]
                            && q[2].uv == [near_rect.u1, near_rect.v1]
                    })
                    .map(|q| q[0].pos[0] as i32)
                    .collect();
                assert_eq!(
                    near_dots,
                    vec![209],
                    "first frame after SetCamera must align placement and distance with the new stand"
                );
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn block_waypoints_use_live_non_player_eyes_within_manhattan_three() {
    in_own_process(
        "cut_layout::game::client_side::locator_bar::tests::block_waypoints_use_live_non_player_eyes_within_manhattan_three",
        || {
            with_client_game(|game| {
                let _server = connect(game, Vec3::new(100.0, 64.0, 0.0));
                add_armor_stand(game, 30, 3, [120.2, 64.0, 5.8]);
                game.armor_stands
                    .iter_mut()
                    .find(|a| a.id == 30)
                    .unwrap()
                    .previous = Vec3::new(119.2, 64.0, 5.8);
                // Offset the witness yaw slightly so the two relevant angles land in
                // different pixels, without changing the waypoint or entity positions.
                game.client.locator_camera = Some(([96.0, 66.0, 0.0], -90.25, 0.0, 70.0));
                game.clock.reset();
                game.clock.set_frozen(false);
                assert!(game.clock.tick_world_runs());
                game.clock.end_frame(false);
                assert_eq!(game.clock.advance(Duration::from_millis(25), 50.0), 0);

                let dot_x = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
                    let draws = game.locator_bar_draws(427, 240);
                    let [GuiDraw::Sprite { x, w: 9, .. }] = draws.as_slice() else {
                        panic!("expected one dot, got {draws:?}");
                    };
                    *x
                };
                let (block, eyes) = game.waypoint_entity(3).unwrap();
                assert_eq!(block, [120, 64, 5]);
                let interpolated_eye = [
                    0x405d_eccc_cccc_cccd,
                    0x4050_71c2_8f80_0000,
                    0x4017_3333_3333_3333,
                ];
                assert_eq!(eyes.map(f64::to_bits), interpolated_eye);
                track_block(game, 3, 3, [122, 64, 6]);
                assert_eq!(dot_x(game), 229, "distance 3: interpolated eye");
                track_block(game, 3, 3, [123, 64, 6]);
                assert_eq!(dot_x(game), 228, "distance 4: block centre");

                // The real DeltaTracker.Timer returns 1.0 for a frozen non-player.
                game.clock.set_frozen(true);
                assert!(!game.clock.tick_world_runs());
                // Minecraft.runTick updates the timer's frozen flag after rendering.
                assert_eq!(
                    game.waypoint_entity(3).unwrap().1.map(f64::to_bits),
                    interpolated_eye
                );
                game.clock.end_frame(false);
                let (_, eyes) = game.waypoint_entity(3).unwrap();
                assert_eq!(
                    eyes.map(f64::to_bits),
                    [
                        0x405e_0ccc_cccc_cccd,
                        0x4050_71c2_8f80_0000,
                        0x4017_3333_3333_3333
                    ],
                );
                assert_eq!(dot_x(game), 228, "distance 4: centre ignores partial tick");
                track_block(game, 3, 3, [122, 64, 6]);
                assert_eq!(dot_x(game), 228, "distance 3: frozen eye");
                game.apply_packet(C::RemoveEntities(play::RemoveEntities { ids: vec![30] }))
                    .unwrap();
                assert_eq!(dot_x(game), 229, "entity absent: block centre");

                // Dimension change clears the current-level UUID index even when persist.uuids
                // retains an old runtime id; reintroducing the UUID under a new id resolves live eyes.
                add_armor_stand(game, 30, 3, [120.2, 64.0, 5.8]);
                game.enter_remote_dimension(
                    &SpawnInfo {
                        dimension_type: 1,
                        dimension: "minecraft:the_nether".into(),
                        seed: 0,
                        game_type: 3,
                        previous_game_type: None,
                        debug: false,
                        flat: true,
                        last_death: None,
                        portal_cooldown: 0,
                        sea_level: 63,
                    },
                    RemoteDimensionEntry::Respawn,
                    Reason::Other,
                );
                assert!(
                    game.persist.uuids.contains_key(&30),
                    "stale persist.uuids entry remains after level replacement"
                );
                assert!(
                    game.waypoint_entity(3).is_none(),
                    "old level entity must not resolve after dimension change"
                );
                add_armor_stand(game, 45, 3, [120.2, 64.0, 5.8]);
                game.armor_stands
                    .iter_mut()
                    .find(|a| a.id == 45)
                    .unwrap()
                    .previous = Vec3::new(119.2, 64.0, 5.8);
                let (rebound_block, rebound_eyes) = game.waypoint_entity(3).unwrap();
                assert_eq!(rebound_block, [120, 64, 5]);
                assert_eq!(rebound_eyes.map(f64::to_bits), interpolated_eye);
                // Re-place the local player in the replacement level (enter_remote_dimension clears it
                // until the server's PlayerPosition packet arrives) so the locator bar has a camera entity.
                let new_pos = Vec3::new(100.0, 64.0, 0.0);
                game.player = Some(PlayerMotion::new(game.motion, new_pos).unwrap());
                game.previous_position = new_pos;
                track_block(game, 3, 3, [122, 64, 6]);
                assert_eq!(
                    dot_x(game),
                    229,
                    "reintroduced UUID uses new runtime id eyes"
                );
            });
        },
    );
}

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
