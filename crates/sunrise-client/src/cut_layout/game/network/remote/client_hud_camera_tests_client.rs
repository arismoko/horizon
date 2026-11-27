use crate::game::remote::tests::client_hud_camera_tests::LOCAL;
use crate::game::remote::tests::client_hud_camera_tests::MOB;
use crate::game::remote::tests::client_hud_camera_tests::WATCHED;
use crate::game::remote::tests::client_hud_camera_tests::attributes;
use crate::game::remote::tests::client_hud_camera_tests::data;
use crate::game::remote::tests::client_hud_camera_tests::drawn_groups;
use crate::game::remote::tests::client_hud_camera_tests::id;
use crate::game::remote::tests::client_hud_camera_tests::mount;
use crate::game::remote::tests::client_hud_camera_tests::num;
use crate::game::remote::tests::client_hud_camera_tests::spawn;
use sunrise_client::level_loading::Reason;
use sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry;
use sunrise_core::geometry::Vec3;
use sunrise_items::items::ItemStack;
use sunrise_protocol::{
    connection::Pipe,
    entity_data::{Value, serializer},
    item::Stack,
    packets::play::{self, Clientbound as C, SpawnInfo},
};
use sunrise_sim::motion::PlayerMotion;
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycle,
};

/// `Hud.getPlayerVehicleWithHealth` and `Hud.getAirBubbleYLine`: the watched rider can
/// ride the dragon or the local player, but a boat leaves the food and air rows unchanged.
#[test]
#[ignore = "daily"]
fn a_watched_rider_gets_hearts_for_the_dragon_and_local_player_but_not_a_boat() {
    use {
        sunrise_client_gameplay::game::remote_camera::CameraPlayer,
        sunrise_gameplay_common::game::Screen, sunrise_gameplay_common::game::in_own_process,
    };

    in_own_process(
        "cut_layout::game::client_side::remote_client::tests::client_hud_camera_tests::a_watched_rider_gets_hearts_for_the_dragon_and_local_player_but_not_a_boat",
        || {
            with_client_game(|game| {
                let (_reader, _writer) = super::connect_menu_test(game);
                super::spawn_menu_test(game);
                game.client.remote.as_mut().unwrap().level_load.finish();
                game.client.screen = Screen::Playing;
                game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
                // LivingEntity.getMaxHealth converts to f32 before Hud adds the half point.
                attributes(game, LOCAL, &[("minecraft:max_health", 31.499_999_9)]);
                game.handle_packet(C::SetHealth(play::SetHealth {
                    health: 17.25,
                    food: 20,
                    saturation: 5.0,
                }))
                .unwrap();

                const DRAGON: i32 = 60;
                const BOAT: i32 = 80;
                game.handle_packet(C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                    actions: 1,
                    entries: vec![play::PlayerInfo {
                        uuid: WATCHED as u128 + 1000,
                        name: Some("Watched".into()),
                        ..play::PlayerInfo::default()
                    }],
                }))
                .unwrap();
                spawn(game, WATCHED, "minecraft:player");
                data(game, WATCHED, vec![(1, serializer::INT, Value::Int(150))]);
                spawn(game, DRAGON, "minecraft:ender_dragon");
                // Seed the live dragon, not its generic mob metadata.
                game.dragon.as_mut().unwrap().health = 37.25;
                spawn(game, BOAT, "minecraft:oak_boat");
                game.handle_packet(C::SetCamera(play::SetCamera { camera_id: WATCHED }))
                    .unwrap();
                let camera = game.camera_player();
                let CameraPlayer::Other(watched) = camera else {
                    panic!("expected the watched player to be the camera");
                };

                for (vehicle, expected, air_y, dragon_max_health) in [
                    (DRAGON, Some((30, 37.25_f32.to_bits())), 171, None),
                    (DRAGON, Some((9, 37.25_f32.to_bits())), 191, Some(18.0)),
                    (LOCAL, Some((16, 17.25_f32.to_bits())), 181, None),
                    (BOAT, None, 191, None),
                ] {
                    if let Some(max_health) = dragon_max_health {
                        attributes(game, DRAGON, &[("minecraft:max_health", max_health)]);
                    }
                    game.handle_packet(C::SetPassengers(play::SetPassengers {
                        vehicle,
                        passengers: vec![WATCHED],
                    }))
                    .unwrap();
                    assert_eq!(game.mobs[watched].extra.mount, Some(vehicle as u32));
                    assert_eq!(game.camera_vehicle(camera).is_some(), expected.is_some());
                    assert_eq!(
                        game.camera_vehicle_health(camera)
                            .map(|(hearts, health)| (hearts, health.to_bits())),
                        expected,
                        "vehicle {vehicle}"
                    );
                    let drawn = drawn_groups(game);
                    let containers = drawn
                        .get("vehicle")
                        .into_iter()
                        .flatten()
                        .filter(|sprite| sprite.0 == "hud/heart/vehicle_container")
                        .count();
                    assert_eq!(
                        containers,
                        expected.map_or(0, |(hearts, _)| hearts as usize)
                    );
                    assert_eq!(drawn.contains_key("food"), expected.is_none());
                    let air = &drawn["air"];
                    assert!(!air.is_empty());
                    assert!(air.iter().all(|sprite| sprite.2 == air_y));
                    game.handle_packet(C::SetPassengers(play::SetPassengers {
                        vehicle,
                        passengers: vec![],
                    }))
                    .unwrap();
                }
            });
        },
    );
}

/// The server-side scene of one recorded case, delivered as the packets a client gets.
pub(in crate::game) fn set_up(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    case: &serde_json::Value,
) {
    let game_type = match case["gameMode"].as_str().unwrap() {
        "survival" => 0,
        "creative" => 1,
        "adventure" => 2,
        "spectator" => 3,
        other => panic!("game mode {other}"),
    };
    game.native_lifecycle = NativeLifecycle::default();
    let (client, _server) = Pipe::pair();
    game.connect_local(client, "HudCameraTest").unwrap();
    let spawn_info = SpawnInfo {
        dimension_type: 0,
        dimension: "minecraft:overworld".into(),
        seed: 0,
        game_type,
        previous_game_type: None,
        debug: false,
        flat: true,
        last_death: None,
        portal_cooldown: 0,
        sea_level: 63,
    };
    game.enter_remote_dimension(&spawn_info, RemoteDimensionEntry::Login, Reason::Other);
    let remote = game.client.remote.as_mut().unwrap();
    remote.logged_in = true;
    remote.entity_id = LOCAL;
    remote.level_load.finish();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());

    // The viewer's own state: what the local hotbar and bars would show.
    game.selected = case["selected"].as_u64().unwrap() as usize;
    game.client.settings.main_hand_left = !case["rightHanded"].as_bool().unwrap();
    for (i, pop) in case["slots"].as_array().unwrap().iter().enumerate() {
        if let Some(pop) = pop.as_u64() {
            game.inventory[i] = Some(ItemStack::new("minecraft:stone", 1));
            game.client.hotbar_pop[i] = pop as u8;
        }
    }
    game.handle_packet(C::SetHealth(play::SetHealth {
        health: num(case, "health") as f32,
        food: num(case, "food") as i32,
        saturation: num(case, "saturation") as f32,
    }))
    .unwrap();
    attributes(
        game,
        LOCAL,
        &[
            ("minecraft:max_health", num(case, "maxHealth")),
            ("minecraft:armor", num(case, "armor")),
        ],
    );
    game.player.as_mut().unwrap().air = num(case, "air") as i32;
    if case.get("vehicleMaxHealth").is_some() {
        mount(
            game,
            40,
            LOCAL,
            num(case, "vehicleMaxHealth"),
            num(case, "vehicleHealth"),
        );
    }

    match case["cameraKind"].as_str().unwrap() {
        "self" => {}
        "mob" => {
            spawn(game, MOB, "minecraft:creeper");
            game.remote_set_camera(MOB);
        }
        "otherPlayer" => {
            let w = &case["watched"];
            game.handle_packet(C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                actions: 1, // ADD_PLAYER
                entries: vec![play::PlayerInfo {
                    uuid: WATCHED as u128 + 1000,
                    name: Some("Watched".into()),
                    ..Default::default()
                }],
            }))
            .unwrap();
            spawn(game, WATCHED, "minecraft:player");
            data(
                game,
                WATCHED,
                vec![
                    (9, serializer::FLOAT, Value::Float(num(w, "health") as f32)),
                    (
                        17,
                        serializer::FLOAT,
                        Value::Float(num(w, "absorption") as f32),
                    ),
                    (1, serializer::INT, Value::Int(num(w, "air") as i32)),
                    (
                        15,
                        serializer::HUMANOID_ARM,
                        Value::Id(i32::from(w["rightHanded"].as_bool().unwrap())),
                    ),
                ],
            );
            attributes(
                game,
                WATCHED,
                &[
                    ("minecraft:max_health", num(w, "maxHealth")),
                    ("minecraft:armor", num(w, "armor")),
                ],
            );
            let sword = Stack::new(id(game, "minecraft:item", "minecraft:diamond_sword"), 1);
            let shield = Stack::new(id(game, "minecraft:item", "minecraft:shield"), 1);
            let mut slots = vec![];
            if w["mainHand"].as_bool().unwrap() {
                slots.push((0u8, Some(sword)));
            }
            if w["offhand"].as_bool().unwrap() {
                slots.push((1u8, Some(shield)));
            }
            game.handle_packet(C::SetEquipment(play::SetEquipment { id: WATCHED, slots }))
                .unwrap();
            if w.get("vehicleMaxHealth").is_some() {
                mount(
                    game,
                    41,
                    WATCHED,
                    num(w, "vehicleMaxHealth"),
                    num(w, "vehicleHealth"),
                );
            }
            game.remote_set_camera(WATCHED);
        }
        other => panic!("camera kind {other}"),
    }
    if case["attackIndicator"] == "HOTBAR" {
        game.client.settings.attack_indicator = 2;
    }
}
