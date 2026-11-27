#[cfg(feature = "client")]
use mod_api::{Collider, ColliderBox, ColliderPose};
#[cfg(feature = "client")]
use std::sync::Arc;
use sunrise_core::geometry::Vec3;
#[cfg(feature = "client")]
use sunrise_core::nbt::Tag;
#[cfg(feature = "client")]
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_gameplay_common::game::piston_push::carrying::shoving::cancel_shove_velocity;
#[cfg(feature = "client")]
use sunrise_items::items::ItemStack;
#[cfg(feature = "client")]
use sunrise_level::chunk::Chunk;
#[cfg(feature = "client")]
use sunrise_protocol::connection::Connection;
#[cfg(feature = "client")]
use sunrise_protocol::packets::play::{self, Clientbound, Serverbound};
#[cfg(feature = "client")]
use sunrise_sim::entities::ItemEntity;
#[cfg(feature = "client")]
use sunrise_sim::motion::PlayerMotion;
#[cfg(feature = "client")]
#[cfg(test)]
use sunrise_world::moving_colliders;
#[cfg(feature = "client")]
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay::game::native_entity_refs::tests::bind_acting_guest,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};
#[cfg(feature = "client")]
use {
    sunrise_client_gameplay::game::client_side::native_client_session::tests::finish_login,
    sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client,
};

#[cfg(feature = "client")]
const PLAYER: u32 = 42;
#[cfg(feature = "client")]
const CONN: u32 = 1;
#[cfg(feature = "client")]
const PIG_Y: f64 = 73.0;

#[cfg(feature = "client")]
fn empty_chunk<'g, G: sunrise_gameplay_common::game::SideGame<'g>>(game: &mut G) {
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
}

#[cfg(feature = "client")]
fn shape() -> Collider {
    Collider::new(vec![ColliderBox {
        min: [-0.25, -3.0, -0.75],
        max: [0.25, -1.0, 0.75],
    }])
}

#[cfg(feature = "client")]
fn holder(game: &mut sunrise_gameplay::game::ServerGame<'_>, x: f64) -> u32 {
    game.options.dedicated = Some(Vec::new());
    game.native_level_opened(game.dim);
    game.native_joins_skip();
    let id = pig(game, [x, PIG_Y, 4.5]);
    game.entity(id)
        .unwrap()
        .set_collider(game, Some(shape()))
        .unwrap();
    id
}

#[cfg(feature = "client")]
fn pig(game: &mut sunrise_gameplay::game::ServerGame<'_>, at: [f64; 3]) -> u32 {
    let level = game.level(game.dim.id()).unwrap();
    level
        .summon(
            game,
            "minecraft:pig",
            at,
            [
                ("NoGravity".into(), Tag::Byte(1)),
                ("NoAI".into(), Tag::Byte(1)),
            ]
            .into_iter()
            .collect(),
        )
        .unwrap()
        .unwrap()
        .id()
}

#[cfg(feature = "client")]
fn turn(game: &mut sunrise_gameplay::game::ServerGame<'_>, holder: u32, degrees: f32) {
    let start = game.placed_moving_colliders();
    let half = degrees.to_radians() / 2.0;
    game.entity(holder)
        .unwrap()
        .set_collider_pose(
            game,
            ColliderPose::default().with_rotation([0.0, half.sin(), 0.0, half.cos()]),
        )
        .unwrap();
    game.carry_pass(&start);
}

#[cfg(feature = "client")]
fn shove(game: &mut sunrise_gameplay::game::ServerGame<'_>, holder: u32, x: f64) {
    let start = game.placed_moving_colliders();
    let i = game.mobs.iter().position(|m| m.id == holder).unwrap();
    game.mobs.update(i, |m| m.position.x = x);
    game.carry_pass(&start);
}

#[cfg(feature = "client")]
fn item(id: u32, x: f64, z: f64, velocity: Vec3) -> ItemEntity {
    ItemEntity::new(
        id,
        ItemStack::new("minecraft:stone", 1),
        Vec3::new(x, 70.25, z),
        velocity,
        0.0,
    )
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_collider_shoves_a_body_crossed_only_mid_sweep() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::piston_push::carrying::shoving::tests::a_collider_shoves_a_body_crossed_only_mid_sweep",
        || {
            with_game(|game| {
                empty_chunk(game);
                let holder = holder(game, 2.5);
                let velocity = Vec3::new(-0.125, 0.0, 0.0625);
                game.dropped.push(item(80, 4.5, 4.5, velocity));
                // The other item only grazes the sweep in Z.
                game.dropped.push(item(81, 4.5, 5.375, velocity));
                shove(game, holder, 6.5);
                // The box ends at x=6.75; the item's half width is 0.125.
                assert_eq!(
                    game.dropped[0].position,
                    Vec3::new(6.75 + 0.125 + 0.01, 70.25, 4.5)
                );
                assert_eq!(game.dropped[0].velocity, Vec3::new(0.0, 0.0, 0.0625));
                assert_eq!(game.dropped[1].position, Vec3::new(4.5, 70.25, 5.375));
            });
        },
    );
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_collider_shove_stops_at_a_wall_without_impact_damage() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::piston_push::carrying::shoving::tests::a_collider_shove_stops_at_a_wall_without_impact_damage",
        || {
            with_game(|game| {
                empty_chunk(game);
                let holder = holder(game, 2.5);
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(6, 70, 4, stone).unwrap();
                let item = item(80, 4.5, 4.5, Vec3::ZERO);
                let health = item.health;
                game.dropped.push(item);
                shove(game, holder, 6.5);
                assert_eq!(game.dropped[0].position, Vec3::new(6.0 - 0.125, 70.25, 4.5));
                assert_eq!(game.dropped[0].health, health);
                assert_eq!(game.collider_origin(holder).unwrap().x, 6.5);
            });
        },
    );
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_later_shove_collides_with_a_holder_already_displaced() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::piston_push::carrying::shoving::tests::a_later_shove_collides_with_a_holder_already_displaced",
        || {
            with_game(|game| {
                empty_chunk(game);
                let source = holder(game, 2.5);
                let first = game.take_entity_id();
                let second = game.take_entity_id();
                game.dropped.push(item(first, 4.0, 4.125, Vec3::ZERO));
                game.dropped.push(item(second, 4.5, 4.875, Vec3::ZERO));
                game.entity(first)
                    .unwrap()
                    .set_collider(
                        game,
                        Some(Collider::new(vec![ColliderBox {
                            min: [-1.0, -0.25, 0.5],
                            max: [-0.75, 0.75, 1.0],
                        }])),
                    )
                    .unwrap();
                shove(game, source, 6.5);
                let first_x = 6.75 + 0.125 + 0.01;
                assert_eq!(game.dropped[0].position.x, first_x);
                // The first item's box began behind the second item. After the first shove,
                // its west face stops the second item before the source's end position.
                assert_eq!(game.dropped[1].position.x, first_x - 1.0 - 0.125);
            });
        },
    );
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_cancelled_turning_shove_leaves_velocity_alone() {
    use mod_api::{EntityMoveEvent, Flow, Hooks, LoopContext, ModEntry};
    struct CancelShove;
    #[mod_api::__subscriptions]
    impl Hooks for CancelShove {
        fn entity_move(&self, _: &dyn LoopContext, _: &mut EntityMoveEvent<'_>) -> Flow {
            Flow::Cancel
        }
    }
    let mods: mod_api::Mods = vec![ModEntry::new("cancel_shove", Box::new(CancelShove))].into();
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::piston_push::carrying::shoving::tests::a_cancelled_turning_shove_leaves_velocity_alone",
        || {
            mod_platform::testing::with_mods(mods, || {
                with_game(|game| {
                    empty_chunk(game);
                    let holder = holder(game, 4.5);
                    let from = Vec3::new(5.2, 70.25, 4.5);
                    let velocity = Vec3::new(-0.3, 0.0, 0.2);
                    game.dropped.push(item(80, 5.2, 4.5, velocity));
                    turn(game, holder, 90.0);
                    assert_eq!(game.dropped[0].position, from);
                    assert_eq!(game.dropped[0].velocity, velocity);
                });
            });
        },
    );
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn an_upward_shove_leaves_the_colliders_vertical_speed() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::piston_push::carrying::shoving::tests::an_upward_shove_leaves_the_colliders_vertical_speed",
        || {
            with_game(|game| {
                empty_chunk(game);
                let holder = holder(game, 4.5);
                game.entity(holder)
                    .unwrap()
                    .set_collider(
                        game,
                        Some(Collider::new(vec![ColliderBox {
                            min: [-1.0, -3.0, -1.0],
                            max: [1.0, -2.9, 1.0],
                        }])),
                    )
                    .unwrap();
                game.dropped
                    .push(item(80, 4.0, 4.5, Vec3::new(0.0, -0.4, 0.0)));
                game.dropped
                    .push(item(81, 5.0, 4.5, Vec3::new(0.0, 0.5, 0.0)));
                let start = game.placed_moving_colliders();
                let rise = (PIG_Y + 0.3) - PIG_Y;
                let i = game.mobs.iter().position(|m| m.id == holder).unwrap();
                game.mobs.update(i, |m| m.position.y = PIG_Y + 0.3);
                game.carry_pass(&start);
                assert!(
                    (game.dropped[0].position.y - 70.41).abs() < 1.0e-6,
                    "falling body: {}",
                    game.dropped[0].position.y
                );
                assert_eq!(game.dropped[0].velocity, Vec3::new(0.0, rise, 0.0));
                assert!(
                    (game.dropped[1].position.y - 70.41).abs() < 1.0e-6,
                    "rising body: {}",
                    game.dropped[1].position.y
                );
                assert_eq!(game.dropped[1].velocity, Vec3::new(0.0, 0.5, 0.0));
            });
        },
    );
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_server_side_turning_shove_never_moves_the_local_player() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::piston_push::carrying::shoving::tests::a_server_side_turning_shove_never_moves_the_local_player",
        || {
            with_game(|game| {
                empty_chunk(game);
                let holder = holder(game, 4.5);
                let from = Vec3::new(5.2, 70.0, 4.5);
                let velocity = Vec3::new(-0.4, 0.0, 0.25);
                game.player = Some(PlayerMotion::new(game.motion, from).unwrap());
                game.player.as_mut().unwrap().velocity = velocity;
                turn(game, holder, 90.0);
                assert_eq!(game.player.as_ref().unwrap().position, from);
                assert_eq!(game.player.as_ref().unwrap().velocity, velocity);
            });
        },
    );
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_client_side_turning_shove_moves_the_local_player() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::piston_push::carrying::shoving::tests::a_client_side_turning_shove_moves_the_local_player",
        || {
            with_client_game(|game| {
                empty_chunk(game);
                let _wire = Connection::local(logged_client(game));
                let mut player = PlayerMotion::new(game.motion, Vec3::new(5.2, 70.0, 4.5)).unwrap();
                player.entity_id = PLAYER as i32;
                player.client_side = true;
                player.velocity = Vec3::new(-0.4, 0.0, 0.25);
                game.player = Some(player);
                let source = 80;
                game.spawn_entity(source, "minecraft:pig", [4.5, PIG_Y, 4.5], [0.0; 3]);
                let remote = game.client.remote.as_mut().unwrap();
                remote.entity_id = PLAYER as i32;
                remote.colliders.insert(
                    source as u32,
                    sunrise_sim::colliders::ColliderState::from_api(&shape()).unwrap(),
                );
                let start = game.client_placed_moving_colliders();
                let half = std::f32::consts::FRAC_1_SQRT_2;
                let mut rotated =
                    sunrise_sim::colliders::ColliderState::from_api(&shape()).unwrap();
                rotated.set_pose(rotated.pose().with_rotation([0.0, half, 0.0, half]));
                game.client
                    .remote
                    .as_mut()
                    .unwrap()
                    .colliders
                    .insert(source as u32, rotated);
                game.carry_pass(&start);
                let player = game.player.as_ref().unwrap();
                assert_ne!(player.position, Vec3::new(5.2, 70.0, 4.5));
            });
        },
    );
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_rider_on_a_turning_collider_is_carried_and_not_shoved_again() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::piston_push::carrying::shoving::tests::a_rider_on_a_turning_collider_is_carried_and_not_shoved_again",
        || {
            with_game(|game| {
                empty_chunk(game);
                let holder = holder(game, 4.5);
                game.entity(holder)
                    .unwrap()
                    .set_collider(
                        game,
                        Some(Collider::new(vec![ColliderBox {
                            min: [-1.0, -3.0, -1.0],
                            max: [1.0, -2.75, 1.0],
                        }])),
                    )
                    .unwrap();
                let mut rider = item(80, 5.5, 4.5, Vec3::new(-0.3, 0.0, 0.2));
                rider.ground.on_ground = true;
                game.dropped.push(rider);
                turn(game, holder, 90.0);
                assert_eq!(game.dropped[0].position, Vec3::new(4.5, 70.25, 3.5));
                assert_eq!(game.dropped[0].velocity, Vec3::new(-0.3, 0.0, 0.2));
            });
        },
    );
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_vanilla_piston_shove_leaves_the_players_velocity_alone() {
    use sunrise_core::{geometry::Direction, position::BlockPos};
    use sunrise_gameplay_common::game::piston_push::PistonPushGameExt1 as _;
    use sunrise_world::level::MovingPiston;

    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::piston_push::carrying::shoving::tests::a_vanilla_piston_shove_leaves_the_players_velocity_alone",
        || {
            with_game(|game| {
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                let moving = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:moving_piston")
                    .unwrap();
                let mut chunk = Chunk::empty(0, 0);
                chunk.set(5, 200, 5, moving.0);
                game.world.chunks.insert((0, 0), Arc::new(chunk));
                let mut player =
                    PlayerMotion::new(game.motion, Vec3::new(5.5, 200.0, 5.5)).unwrap();
                player.velocity = Vec3::new(-0.4, -0.2, 0.1);
                game.player = Some(player);
                let pos = BlockPos::new(5, 200, 5);
                let piston = MovingPiston {
                    moved_state: stone,
                    direction: Direction::East,
                    extending: true,
                    source: false,
                    progress: 0.0,
                    progress_o: 0.0,
                    last_ticked: 0,
                    death_ticks: 0,
                    id: 1,
                };
                game.block_ticks.moving_pistons = vec![(
                    pos,
                    MovingPiston {
                        progress: 0.5,
                        ..piston
                    },
                )];
                game.sync_moving_pistons();
                game.push_piston_entities(vec![(pos, piston, 0.5)]);
                assert!(
                    (game.player.as_ref().unwrap().position.x - 5.81).abs() < 1.0e-6,
                    "piston shove: {}",
                    game.player.as_ref().unwrap().position.x
                );
                assert_eq!(
                    game.player.as_ref().unwrap().velocity,
                    Vec3::new(-0.4, -0.2, 0.1)
                );
            });
        },
    );
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn a_predicted_collider_shove_passes_the_server_move_check() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::piston_push::carrying::shoving::tests::a_predicted_collider_shove_passes_the_server_move_check",
        || {
            let from = Vec3::new(5.0, 70.25, 4.5);
            let (packet, to) = {
                let mut report = None;
                with_client_game(|game| {
                    let mut wire = Connection::local(logged_client(game));
                    wire.reader
                        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                        .unwrap();
                    finish_login(game, &mut wire);
                    game.client.screen = sunrise_gameplay_common::game::Screen::Playing;
                    empty_chunk(game);
                    let mut player = PlayerMotion::new(game.motion, from).unwrap();
                    player.entity_id = PLAYER as i32;
                    player.client_side = true;
                    game.player = Some(player);
                    let source = 80;
                    game.spawn_entity(source, "minecraft:pig", [4.0, PIG_Y, 4.5], [0.0; 3]);
                    let remote = game.client.remote.as_mut().unwrap();
                    remote.entity_id = PLAYER as i32;
                    remote.loaded_sent = true;
                    remote.level_load.finish();
                    remote.colliders.insert(
                        source as u32,
                        sunrise_sim::colliders::ColliderState::from_api(&shape()).unwrap(),
                    );
                    // The last client tick saw the old pose; packet intake precedes the next.
                    game.tick().unwrap();
                    loop {
                        let (id, bytes) = wire.recv().unwrap();
                        if matches!(
                            Serverbound::decode(id, &bytes).unwrap(),
                            Serverbound::ClientTickEnd(_)
                        ) {
                            break;
                        }
                    }
                    game.handle_packet(Clientbound::EntityPositionSync(play::EntityPositionSync {
                        id: source,
                        path: play::PositionPath::Linear([4.75, PIG_Y, 4.5]),
                        yaw: 0.0,
                        pitch: 0.0,
                        on_ground: false,
                    }))
                    .unwrap();
                    game.tick().unwrap();
                    let player = game.player.as_ref().unwrap();
                    let to = player.position.components();
                    assert_eq!(player.bounds().min_position().x, 5.0 + 0.01);
                    let mut movement = None;
                    loop {
                        let (id, bytes) = wire.recv().unwrap();
                        let packet = Serverbound::decode(id, &bytes).unwrap();
                        let sent = match &packet {
                            Serverbound::MovePlayerPos(p) => [p.x, p.y, p.z],
                            Serverbound::MovePlayerPosRot(p) => [p.x, p.y, p.z],
                            Serverbound::ClientTickEnd(_) => break,
                            _ => continue,
                        };
                        assert_eq!(sent, to, "the tick's own report includes the shove");
                        assert!(movement.replace(packet).is_none());
                    }
                    report = Some((movement.expect("position before ClientTickEnd"), to));
                });
                report.unwrap()
            };
            with_game(|game| {
                empty_chunk(game);
                holder(game, 4.75);
                game.player = Some(PlayerMotion::new(game.motion, from).unwrap());
                game.server.lan_acting = Some(CONN);
                game.server.lan_acting_entity = Some(PLAYER);
                game.server.lan_acting_uuid = Some(11);
                game.server.lan_acting_name = Some("pushed-player".into());
                bind_acting_guest(game);
                // The packet belongs to the hub guest, not a second local body.
                game.side_remove_local_player_body();
                game.server.lan_acting = None;
                let hub = &mut game.server.lan.as_mut().unwrap().hub;
                hub.update_guest(CONN, |g| {
                    g.loaded = true;
                    g.load_timeout = 0;
                    g.inbox
                        .push_back(sunrise_sim::lan::guest::InboxItem::Packet(packet, None));
                });
                let placed = game.placed_moving_colliders();
                moving_colliders::in_level(placed, || game.tick_lan_packets());
                let guest = game.server.lan.as_ref().unwrap().hub.guest(CONN).unwrap();
                assert_eq!(
                    guest.received_moves, 1,
                    "the normal movement handler read the report"
                );
                assert_eq!(
                    guest.awaiting_teleport, None,
                    "the normal movement check did not correct it"
                );
                assert_eq!(guest.position, to);
            });
        },
    );
}

#[test]
fn an_upward_shove_never_lowers_a_rising_bodys_vertical_speed() {
    let normal = Vec3::new(0.0, 0.6, 0.8);
    // A jumping body hit by a tilted upward shove: the inward part goes, and its vertical speed
    // stays above the collider's rise.
    let jumping = Vec3::new(0.0, 0.9, -0.9);
    let after = cancel_shove_velocity(jumping, normal, 0.6, 0.3);
    let inward = jumping.dot(normal);
    assert!(inward < 0.0);
    let expected = jumping.subtract(normal.scale(inward));
    assert_eq!(after.x, expected.x);
    assert_eq!(after.z, expected.z);
    assert_eq!(after.y, expected.y.max(0.3));
    assert!(after.y > 0.3);

    // A falling body is raised to the collider's rise.
    let falling = Vec3::new(0.0, -0.5, -0.4);
    let after = cancel_shove_velocity(falling, normal, 0.6, 0.3);
    assert_eq!(after.y, 0.3);
}

#[test]
fn a_descending_collider_does_not_set_a_downward_vertical_speed_on_a_rising_body() {
    // A turning collider that sinks overall can still push a body upward at its tilted edge.
    let normal = Vec3::new(0.0, 0.6, 0.8);
    let body = Vec3::new(0.0, 0.2, -0.4);
    let after = cancel_shove_velocity(body, normal, 0.6, -0.25);
    let expected = body.subtract(normal.scale(body.dot(normal)));
    assert_eq!(after.y, expected.y.max(-0.25));
    assert!(after.y > -0.25);
    // A body moving away from the shove keeps its velocity whatever the collider does.
    let away = Vec3::new(0.1, 0.5, 0.4);
    assert_eq!(cancel_shove_velocity(away, normal, 0.6, -0.25), away);
}

#[cfg(feature = "client")]
use sunrise_gameplay_common::game::side_ops::player::PlayerSideGameExt as _;

#[cfg(feature = "client")]
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(feature = "client")]
use sunrise_gameplay_common::game::piston_push::carrying::CarryingGameExt1 as _;
