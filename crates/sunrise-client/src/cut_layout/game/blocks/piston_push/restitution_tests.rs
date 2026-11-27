#![cfg(feature = "client")]
use mod_api::{
    BlockPart, BounceOnEvent, BounceRestitutionEvent, Flow, Hooks, LevelRandomHost, LevelSide,
    Registrar, RegistrationContext,
};
use std::sync::{Arc, Mutex};
use sunrise_gameplay_common::game::boats_common::Boat;
use sunrise_gameplay_common::game::piston_push::{Aabb, Direction, Lid, Vec3};
use sunrise_gameplay_common::game::remote::passengers::PassengerSeat;
use sunrise_level::chunk::Chunk;
use sunrise_protocol::packets::play::{self, Clientbound};
use sunrise_sim::mobs::Mob;
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};
use {
    sunrise_gameplay_common::game::minecarts::CartKind,
    sunrise_gameplay_common::game::minecarts::Minecart,
};

#[derive(Clone, Default)]
struct PushBounce {
    pub restitution: Arc<Mutex<Vec<(bool, i32)>>>,
    landings: Arc<Mutex<Vec<(i32, f64)>>>,
}

#[mod_api::sunrise_part]
impl BlockPart for PushBounce {
    fn bounce_restitution(&self, side: LevelSide<'_>, event: &mut BounceRestitutionEvent<'_>) {
        self.restitution
            .lock()
            .unwrap()
            .push((matches!(side, LevelSide::Client(_)), event.entity));
        event.factor = 1.0;
    }

    fn bounce_on(&self, _: &mut dyn LevelRandomHost, event: &mut BounceOnEvent<'_>) {
        self.landings
            .lock()
            .unwrap()
            .push((event.entity, event.fall_distance));
    }
}

#[mod_api::__subscriptions]
impl Hooks for PushBounce {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let glass = r.block_key("minecraft:glass");
        r.block_part(glass, self.clone());
    }
}

struct RemoveLaterBoat;

#[mod_api::__subscriptions]
impl Hooks for RemoveLaterBoat {
    fn vanilla_game_event(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::VanillaGameEventEvent<'_>,
    ) -> Flow {
        if event.event == "minecraft:bounce" && event.cause == Some(71) {
            assert!(host.entity(72).unwrap().discard(host));
            assert!(host.entity(74).unwrap().discard(host));
        }
        Flow::Continue
    }
}

fn boats_over_glass<'g, G: sunrise_gameplay_common::game::SideGame<'g>>(
    game: &mut G,
    boats: &[(u32, i32)],
) {
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let glass = game
        .pack
        .assets
        .registry
        .default_state("minecraft:glass")
        .unwrap();
    for &(id, x) in boats {
        game.world.set(x, 63, 4, glass).unwrap();
        let mut boat = Boat::new(
            id,
            "oak".into(),
            Vec3::new(f64::from(x) + 0.5, 64.25, 4.5),
            0.0,
        );
        boat.velocity.y = -1.0;
        game.boats.push(boat);
    }
}

#[test]
#[ignore = "daily"]
fn a_bounce_listener_can_remove_a_later_lid_target() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::piston_push::restitution_tests::a_bounce_listener_can_remove_a_later_lid_target",
        || {
            let part = PushBounce::default();
            let mods: mod_api::Mods = vec![
                mod_api::ModEntry::new("bounce", Box::new(part.clone())),
                mod_api::ModEntry::new("discard", Box::new(RemoveLaterBoat)),
            ]
            .into();
            mod_platform::testing::with_mods(mods.clone(), || {
                sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
                sunrise_gameplay_common::game::native_hooks::install_block_contact_hooks();
                with_game(|game| {
                    boats_over_glass(game, &[(71, 4), (72, 7), (73, 10)]);
                    game.boats.update(0, |boat| boat.fall_distance = 5.0);
                    game.boats.update(1, |boat| boat.position.x = 8.875);
                    game.carts.push(Minecart::new(
                        74,
                        CartKind::Rideable,
                        Vec3::new(9.125, 64.25, 3.95),
                    ));
                    let area = Aabb::new(Vec3::new(3.0, 64.5, 4.5), Vec3::new(12.0, 65.0, 6.0));
                    sunrise_sim::block_contact::in_level(game.tick, Some(game.dim.id()), || {
                        game.lid_push(Lid::Box, area, Vec3::new(-0.5, -0.5, 0.0));
                    });
                    // Discard retires the handle synchronously; the tick sweep owns
                    // physical removal. The lid must not push that departed row.
                    assert!(game.entity(72).is_none());
                    assert!(game.entity(74).is_none());
                    assert_eq!(
                        game.boats[2].position.x, 10.0,
                        "neither discarded hull blocks the next boat"
                    );
                    for boat in game.boats.iter() {
                        if boat.id == 72 {
                            assert_eq!(boat.position.y, 64.25);
                            assert_eq!(boat.velocity.y, -1.0);
                        } else {
                            assert_eq!(boat.position.y, 64.0);
                            assert!(boat.velocity.y > 0.0);
                        }
                    }
                    assert_eq!(
                        *part.restitution.lock().unwrap(),
                        [(false, 71), (false, 73)]
                    );
                    assert_eq!(*part.landings.lock().unwrap(), [(71, 0.0), (73, 0.0)]);
                });
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_downward_shulker_box_lid_bounces_only_the_locally_driven_boat() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::piston_push::restitution_tests::a_downward_shulker_box_lid_bounces_only_the_locally_driven_boat",
        || {
            let part = PushBounce::default();
            let mods: mod_api::Mods =
                vec![mod_api::ModEntry::new("bounce", Box::new(part.clone()))].into();
            mod_platform::testing::with_mods(mods.clone(), || {
                sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
                sunrise_gameplay_common::game::native_hooks::install_block_contact_hooks();
                with_client_game(|game| {
                    game.init_remote_for_replay().unwrap();
                    game.client.remote.as_mut().unwrap().first_passenger = true;
                    game.riding = Some(71);
                    boats_over_glass(game, &[(71, 4), (72, 7)]);
                    game.boats.update(0, |boat| boat.fall_distance = 5.0);
                    game.boats.update(1, |boat| boat.fall_distance = 5.0);
                    let facing = Direction::ALL
                        .iter()
                        .position(|d| *d == Direction::Down)
                        .unwrap() as u8;
                    sunrise_sim::block_contact::in_level(game.tick, Some(game.dim.id()), || {
                        game.push_shulker_local(vec![(
                            [4, 65, 4],
                            facing,
                            [3.0, 64.5, 3.0, 9.0, 65.0, 6.0],
                        )]);
                    });
                    assert_eq!(game.boats[0].position.y, 64.0);
                    assert_eq!(game.boats[0].fall_distance, 0.0);
                    assert!(game.boats[0].velocity.y > 0.0);
                    assert_eq!(game.boats[1].position.y, 64.0);
                    assert!(game.boats[1].ground.on_ground);
                    assert_eq!(game.boats[1].fall_distance, 5.0);
                    assert_eq!(game.boats[1].velocity.y, -1.0);
                    assert_eq!(*part.restitution.lock().unwrap(), [(true, 71)]);
                    assert!(!game.persist.sync_body_position.contains(&71));
                });
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_shulker_box_lid_stops_a_driven_boat_at_another_boat() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::piston_push::restitution_tests::a_shulker_box_lid_stops_a_driven_boat_at_another_boat",
        || {
            with_client_game(|game| {
                game.init_remote_for_replay().unwrap();
                game.client.remote.as_mut().unwrap().first_passenger = true;
                game.riding = Some(71);
                boats_over_glass(game, &[(71, 4), (72, 6)]);
                game.boats.update(1, |boat| boat.position.x = 6.125);
                let facing = Direction::ALL
                    .iter()
                    .position(|d| *d == Direction::East)
                    .unwrap() as u8;
                let push = || vec![([4, 64, 4], facing, [4.125, 64.25, 3.5, 4.625, 64.75, 5.5])];
                game.push_shulker_local(push());
                // The 1.375-wide hulls are 1.625 blocks apart. The first boat
                // moves 0.25 blocks before contact, not the lid's requested 0.51.
                assert_eq!(game.boats[0].position.x, 4.75);
                assert_eq!(game.boats[1].position.x, 6.125);

                // The riding graph can nest vehicles. The same hull on a
                // shared root is not a collider, even when it isn't a rider of 71.
                game.boats.update(0, |boat| boat.position.x = 4.5);
                game.carts.push(Minecart::new(
                    75,
                    CartKind::Rideable,
                    Vec3::new(5.0, 67.0, 4.5),
                ));
                let remote = game.client.remote.as_mut().unwrap();
                remote.passenger_vehicles.insert(
                    71,
                    PassengerSeat {
                        vehicle: 75,
                        index: 0,
                    },
                );
                remote.passenger_vehicles.insert(
                    72,
                    PassengerSeat {
                        vehicle: 75,
                        index: 1,
                    },
                );
                game.push_shulker_local(push());
                assert_eq!(game.boats[0].position.x, 4.5 + 0.51);
                assert_eq!(game.boats[1].position.x, 6.125);
            })
        },
    );
}

#[test]
#[ignore = "daily"]
fn an_observing_boat_keeps_the_lid_push_through_its_last_lerp_step() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::piston_push::restitution_tests::an_observing_boat_keeps_the_lid_push_through_its_last_lerp_step",
        || {
            with_client_game(|game| {
                game.init_remote_for_replay().unwrap();
                boats_over_glass(game, &[(71, 4)]);
                game.boats.update(0, |boat| boat.position.y = 64.0);
                game.handle_packet(Clientbound::EntityPositionSync(play::EntityPositionSync {
                    id: 71,
                    path: play::PositionPath::Linear([4.5, 64.375, 4.5]),
                    yaw: 0.0,
                    pitch: 0.0,
                    on_ground: false,
                }))
                .unwrap();
                game.tick_remote_entities(true);
                game.tick_remote_entities(true);
                assert_eq!(game.boats[0].position.y, 64.25);

                let kind = game
                    .res
                    .mob_types()
                    .get("minecraft:shulker")
                    .unwrap()
                    .clone();
                let mut shell = Mob::new(72, kind, Vec3::new(4.5, 63.25, 4.5), 0.0);
                shell.extra.counter = 100; // Open the lid on the boat's final interpolation tick.
                game.mobs.push(shell);
                game.tick_remote_entities(true);
                let boat = &game.boats[0];
                let rise = boat.previous.y - 64.25;
                assert!(
                    rise > 0.0,
                    "the shulker's lid moves the observer before interpolation"
                );
                assert_eq!(boat.position.y, 64.375 + rise);
            })
        },
    );
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
