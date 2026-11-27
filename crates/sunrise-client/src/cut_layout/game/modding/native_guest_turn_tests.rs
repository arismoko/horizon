//! Real-game regressions for native handles used during a guest's turn.

#![cfg(test)]

use mod_api::{
    __private::{HookPanicReport, take_hook_panics},
    EntityMoveEvent, Flow, Hooks, LoopContext, MoveCause,
    prelude::*,
};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::execution_role::ExecutionRole;
use sunrise_sim::{lan::PlayerSender, mobs::Mob};

pub use sunrise_gameplay::game::native_guest_turn_tests::{
    expire_guest_damage_cooldown, level_uuids, place_host, register_test_menu, summon_stand,
    told_since, with_command_guests, with_guests, with_resident_guests,
};

#[test]
#[ignore = "daily"]
fn player_dimensions_validate_identity_on_both_server_roles() {
    let previous_extra = sunrise_gameplay_common::game::EXTRA_DIMS
        .read()
        .unwrap()
        .clone();
    with_guests(|game| {
        sunrise_gameplay_common::game::Dim::add_extra("test:pocket");
        for role in [
            ExecutionRole::DedicatedServer,
            ExecutionRole::IntegratedServer,
        ] {
            game.options.execution_role = role;
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| {
                    guest.dimension = "minecraft:overworld".to_owned();
                })
                .unwrap();
            let player = NativeServerGameExt1::player(game, 12).unwrap();
            let other = NativeServerGameExt1::player(game, 13).unwrap();
            let epoch = player.entity().epoch();
            let unknown = mod_api::__private::game::player_ref(player.entity(), 999);
            let stale = mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(
                    mod_api::WorldEpoch::new(epoch.get() + 1),
                    player.entity().id(),
                ),
                player.uuid(),
            );
            let replaced = mod_api::__private::game::player_ref(other.entity(), player.uuid());
            for invalid in [unknown, stale, replaced] {
                assert_eq!(
                    NativeServerGameExt1::player_level(game, invalid),
                    None,
                    "{role:?}"
                );
                assert_eq!(
                    NativeServerGameExt1::player_dimension(game, invalid),
                    None,
                    "{role:?}"
                );
            }
            assert_eq!(
                NativeServerGameExt1::player_level(game, player)
                    .unwrap()
                    .dimension_id(),
                "minecraft:overworld"
            );
            game.as_guest(12, |game| {
                assert_eq!(
                    NativeServerGameExt1::player_dimension(game, player)
                        .unwrap()
                        .to_string(),
                    "minecraft:overworld"
                );
                game.as_guest(13, |game| {
                    assert_eq!(
                        NativeServerGameExt1::player_level(game, player)
                            .unwrap()
                            .dimension_id(),
                        "minecraft:overworld"
                    );
                })
                .unwrap();
            })
            .unwrap();
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| {
                    guest.dimension = "minecraft:the_nether".to_owned();
                })
                .unwrap();
            assert_eq!(
                NativeServerGameExt1::player_dimension(game, player)
                    .unwrap()
                    .to_string(),
                "minecraft:the_nether"
            );
            assert_eq!(
                NativeServerGameExt1::player_level(game, player)
                    .unwrap()
                    .dimension_id(),
                "minecraft:the_nether"
            );
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| {
                    guest.dimension = "test:pocket".to_owned();
                })
                .unwrap();
            assert_eq!(
                NativeServerGameExt1::player_dimension(game, player)
                    .unwrap()
                    .to_string(),
                "test:pocket"
            );
            assert_eq!(
                NativeServerGameExt1::player_level(game, player)
                    .unwrap()
                    .dimension_id(),
                "test:pocket"
            );
        }
    });
    *sunrise_gameplay_common::game::EXTRA_DIMS.write().unwrap() = previous_extra;
}

#[test]
#[ignore = "daily"]
fn refused_wire_reload_preserves_hub_and_engine_generation() {
    with_guests(|game| {
        let before = game.server.lan.as_ref().unwrap().hub.lan.shared.sync();
        let old_tables = game
            .server
            .world_tables
            .0
            .as_ref()
            .map(|tables| tables as *const _);
        let old_context = game.loop_context.clone();
        let root = std::env::temp_dir().join(format!("reload-wire-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        // Table staging now reads the resource root before the wire reload.
        // Supply its ordinary inputs, but deliberately omit the registry report
        // so it is still the wire preparation that refuses this candidate.
        let mut folders = vec![(game.options.resources.clone(), root.clone())];
        while let Some((source, destination)) = folders.pop() {
            std::fs::create_dir_all(&destination).unwrap();
            for entry in std::fs::read_dir(&source).unwrap() {
                let entry = entry.unwrap();
                let name = entry.file_name();
                let path = entry.path();
                if entry.file_type().unwrap().is_dir() {
                    if source != game.options.resources || name == "data" {
                        folders.push((path, destination.join(name)));
                    }
                } else if name != "registries.json" {
                    std::fs::copy(path, destination.join(name)).unwrap();
                }
            }
        }
        let stack = sunrise_gameplay::game::server_side::functions::pack_stack(
            &game.server.world_root,
            &root,
            &game.pack.data_source,
            &game.pack.data_changes,
            false,
            &[],
            &[],
        );
        let error = game
            .prepare_world_reload(&stack, &root)
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("wire reload") && error.contains("registries.json"),
            "{error}"
        );
        assert!(Arc::ptr_eq(
            &before,
            &game.server.lan.as_ref().unwrap().hub.lan.shared.sync()
        ));
        assert!(Arc::ptr_eq(&old_context, &game.loop_context));
        assert_eq!(
            old_tables,
            game.server
                .world_tables
                .0
                .as_ref()
                .map(|tables| tables as *const _)
        );
        let good = game.options.resources.clone();
        let stack = sunrise_gameplay::game::server_side::functions::pack_stack(
            &game.server.world_root,
            &good,
            &game.pack.data_source,
            &game.pack.data_changes,
            false,
            &[],
            &[],
        );
        let prepared = game.prepare_world_reload(&stack, &good).unwrap();
        game.install_world_tables(prepared);
        game.run_native_advancement_grants();
        assert!(!Arc::ptr_eq(
            &before,
            &game.server.lan.as_ref().unwrap().hub.lan.shared.sync()
        ));
        std::fs::remove_dir_all(root).unwrap();
    });
}

/// Before #228, the host was absent during a guest turn and B and the
/// online list were invisible to native handles.
#[test]
#[ignore = "daily"]
fn another_guest_and_online_players_are_visible_during_a_turn() {
    with_guests(|game| {
        let rider = NativeEntityRefsCommonGameExt1::entity(game, 13).unwrap();
        game.as_guest(12, |game| {
            assert_eq!(rider.position(game), Some([3.0, 64.0, 5.0]));
            let online = game.native_server_players();
            assert_eq!(
                online.iter().map(|p| p.uuid()).collect::<Vec<_>>(),
                [12, 13]
            );
            assert_eq!(online[1].entity().position(game).unwrap(), [3.0, 64.0, 5.0]);
        })
        .unwrap();
    });
}

/// `entity_move_by` moves a guest the server holds, acting or not, through its own body and the
/// hub's copy together, tells its client, and refuses what it cannot move without changing
/// anything.
#[test]
#[ignore = "daily"]
fn move_by_moves_a_player_and_refuses_what_it_cannot() {
    with_guests(|game| {
        // A guest has a body once it has had a turn.
        for conn in [12, 13] {
            game.as_guest(conn, |_| {}).unwrap();
        }
        let a = NativeServerGameExt1::player(game, 12).unwrap();
        let b = NativeServerGameExt1::player(game, 13).unwrap();
        let before = b.entity().position(game).unwrap();
        let by = game
            .entity_move_by(b.entity(), [0.5, 0.0, 0.25], MoveCause::SelfMovement)
            .unwrap();
        assert_eq!(by, [0.5, 0.0, 0.25]);
        let now = [before[0] + 0.5, before[1], before[2] + 0.25];
        assert_eq!(b.entity().position(game), Some(now));
        let hub = &game.server.lan.as_ref().unwrap().hub;
        let guest = hub.guest(13).unwrap();
        assert_eq!(guest.position, now, "the hub's copy follows the body");
        assert!(guest.awaiting_teleport.is_some(), "the client is told");
        // The acting player is moved in its turn, and another during it.
        game.as_guest(12, |game| {
            let own = a.entity().position(game).unwrap();
            assert_eq!(
                game.entity_move_by(a.entity(), [0.0, 0.0, -2.0], MoveCause::SelfMovement),
                Some([0.0, 0.0, -2.0])
            );
            assert_eq!(
                a.entity().position(game),
                Some([own[0], own[1], own[2] - 2.0])
            );
            assert!(
                game.entity_move_by(b.entity(), [1.0, 0.0, 0.0], MoveCause::SelfMovement)
                    .is_some()
            );
        })
        .unwrap();
        // Nothing moves for a value that is not finite or a target outside the world.
        let held = b.entity().position(game);
        for delta in [
            [f64::NAN, 0.0, 0.0],
            [0.0, f64::INFINITY, 0.0],
            [4.0e7, 0.0, 0.0],
        ] {
            assert_eq!(
                game.entity_move_by(b.entity(), delta, MoveCause::SelfMovement),
                None,
                "{delta:?}"
            );
        }
        assert_eq!(b.entity().position(game), held);
    });
}

/// A player's `entity_move_by` fires `entity_move` once, for the player and with the caller's
/// cause; a listener that cancels it leaves the player where it was and the client untold.
#[test]
#[ignore = "daily"]
fn a_players_move_by_is_heard_once_and_a_cancel_moves_nothing() {
    struct Cancel(Arc<Mutex<Vec<(u32, MoveCause)>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Cancel {
        fn entity_move(&self, _: &dyn LoopContext, event: &mut EntityMoveEvent<'_>) -> Flow {
            self.0.lock().unwrap().push((event.entity, event.cause));
            Flow::Cancel
        }
    }
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "refuser",
        Box::new(Cancel(heard.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_guests(|game| {
            game.as_guest(13, |_| {}).unwrap();
            let b = NativeServerGameExt1::player(game, 13).unwrap();
            let before = b.entity().position(game);
            let moved = game.entity_move_by(b.entity(), [1.0, 0.0, 0.0], MoveCause::Piston);
            assert_eq!(moved, Some([0.0; 3]));
            assert_eq!(b.entity().position(game), before);
            assert!(
                game.server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(13)
                    .unwrap()
                    .awaiting_teleport
                    .is_none()
            );
            assert_eq!(
                *heard.lock().unwrap(),
                [(b.entity().id(), MoveCause::Piston)]
            );
        });
    });
}

/// A boat in the way stops a player's `entity_move_by`, as it stops a player walking: the move
/// ends against the boat's side, not at the target.
#[test]
#[ignore = "daily"]
fn a_players_move_by_stops_at_a_boat() {
    with_guests(|game| {
        game.as_guest(13, |_| {}).unwrap();
        let b = NativeServerGameExt1::player(game, 13).unwrap();
        let [x, y, z] = b.entity().position(game).unwrap();
        let free = game
            .entity_move_by(b.entity(), [4.0, 0.0, 0.0], MoveCause::SelfMovement)
            .unwrap();
        assert_eq!(free, [4.0, 0.0, 0.0], "nothing in the way");
        // Back to where it was, then a boat 2 blocks along, its box across the line of travel.
        game.entity_move_by(b.entity(), [-4.0, 0.0, 0.0], MoveCause::SelfMovement)
            .unwrap();
        game.server.next_entity += 1;
        let id = game.server.next_entity;
        game.boats
            .push(sunrise_gameplay_common::game::boats_common::Boat::new(
                id,
                "oak".into(),
                Vec3::new(x + 2.0, y, z),
                0.0,
            ));
        let by = game
            .entity_move_by(b.entity(), [4.0, 0.0, 0.0], MoveCause::SelfMovement)
            .unwrap();
        assert!(
            by[0] > 0.5 && by[0] < 1.6,
            "stopped against the boat's side, moved {by:?}"
        );
    });
}

/// `entity_move_by` names the player as the mover of its move, so a moving collider whose
/// vehicle stack holds the player does not stop it, and one that does not hold it does.
#[test]
#[ignore = "daily"]
fn a_players_move_by_is_the_mover_a_colliders_stack_exempts() {
    use sunrise_core::geometry::{
        Aabb,
        oriented::{ColliderBoxes, Frame, OrientedCollider},
    };
    use sunrise_world::moving_colliders::{Placed, Prepared, Scope};
    with_guests(|game| {
        game.as_guest(13, |_| {}).unwrap();
        let b = NativeServerGameExt1::player(game, 13).unwrap();
        let [x, y, z] = b.entity().position(game).unwrap();
        let player = game
            .server
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guest(13)
            .unwrap()
            .entity_id as u32;
        let block = |id: u32, at: f64, exempt: Vec<u32>| Placed {
            id,
            collider: OrientedCollider::new(
                Frame::translation(Vec3::new(at, y, z - 0.5)),
                std::sync::Arc::new(ColliderBoxes::new(vec![Aabb::new(
                    Vec3::new(0.0, 0.0, 0.0),
                    Vec3::new(1.0, 1.0, 1.0),
                )])),
            ),
            exempt,
        };
        // One collider 1 block along whose stack holds the player, another 3 blocks along
        // whose stack does not.
        let placed = Prepared::new(vec![
            block(900, x + 1.0, vec![900, player]),
            block(901, x + 3.0, vec![901]),
        ]);
        let _scope = Scope::enter_level(game.collider_level(), placed);
        let by = game
            .entity_move_by(b.entity(), [4.0, 0.0, 0.0], MoveCause::SelfMovement)
            .unwrap();
        assert!(
            by[0] > 1.5 && by[0] < 2.8,
            "passed the collider it rides with and stopped at the other, moved {by:?}"
        );
    });
}

/// The server has no predictive landing on a happy ghast: a player over a ghast that is not held
/// still falls through it (`HappyGhast.canBeCollidedWith`, `isClientSide` branch), so a mod's
/// move down is not stopped at its top the way the client's own walk is.
#[test]
#[ignore = "daily"]
fn a_players_move_by_falls_through_a_happy_ghast_that_is_not_held_still() {
    with_guests(|game| {
        game.as_guest(13, |_| {}).unwrap();
        let b = NativeServerGameExt1::player(game, 13).unwrap();
        let [x, y, z] = b.entity().position(game).unwrap();
        game.server.next_entity += 1;
        let id = game.server.next_entity;
        let kind = game
            .res
            .mob_types()
            .get("minecraft:happy_ghast")
            .unwrap()
            .clone();
        game.mobs.push(Mob::new(id, kind, Vec3::new(x, y, z), 0.0));
        let up = game
            .entity_move_by(b.entity(), [0.0, 6.0, 0.0], MoveCause::SelfMovement)
            .unwrap();
        assert_eq!(up, [0.0, 6.0, 0.0], "room to rise past the ghast's top");
        let down = game
            .entity_move_by(b.entity(), [0.0, -6.0, 0.0], MoveCause::SelfMovement)
            .unwrap();
        assert_eq!(down, [0.0, -6.0, 0.0], "not stopped on a ghast's top");
    });
}

/// B's live menu and motion stay available while A acts, and A's own menu reads the swapped-in
/// state, but displaced menus and an outer turn's motion must not use the swapped-out records.
#[test]
#[ignore = "daily"]
fn another_guests_menu_and_push_work_during_a_turn() {
    use sunrise_items::items::ItemStack;

    with_guests(|game| {
        for conn in [12, 13] {
            game.as_guest(conn, |game| {
                game.player.as_mut().unwrap().velocity = Vec3::ZERO;
            })
            .unwrap();
        }
        let a = NativeServerGameExt1::player(game, 12).unwrap();
        let b = NativeServerGameExt1::player(game, 13).unwrap();
        let a_menu = NativeMenuRefsCommonGameExt1::player_container_menu(game, a).unwrap();
        let b_menu = NativeMenuRefsCommonGameExt1::player_container_menu(game, b).unwrap();
        let stone = ItemStack::new("minecraft:stone", 4);
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(13, |guest| guest.inventory[0] = Some(stone.clone()))
            .unwrap();
        game.as_guest(12, |game| {
            assert_eq!(
                NativeMenuRefsCommonGameExt1::player_container_menu(game, a),
                Some(a_menu)
            );
            assert_eq!(a_menu.slot_count(game), Some(46));
            assert_eq!(
                NativeMenuRefsCommonGameExt1::player_container_menu(game, b),
                Some(b_menu)
            );
            assert_eq!(b_menu.slot_count(game), Some(46));
            assert_eq!(
                b_menu.slot(game, 36),
                Some(Some(
                    sunrise_gameplay_common::game::api_values::test_snapshot(&stone)
                ))
            );
            assert!(game.entity_set_rotation(b.entity(), 90.0, 10.0));
            assert!(game.entity_set_head_yaw(b.entity(), 45.0));
            assert_eq!(game.entity_head_yaw(b.entity()), Some(45.0));
            assert!(game.entity_push(b.entity(), [0.25, 0.5, -0.25]));
            assert!(game.entity_push(a.entity(), [0.5, 0.0, 0.0]));
            game.as_guest(13, |game| {
                assert_eq!(
                    game.player.as_ref().unwrap().velocity,
                    Vec3::new(0.25, 0.5, -0.25)
                );
                assert!(NativeMenuRefsCommonGameExt1::player_container_menu(game, a).is_none());
                assert_eq!(
                    NativeMenuRefsCommonGameExt1::player_container_menu(game, b),
                    Some(b_menu)
                );
                assert_eq!(b_menu.slot_count(game), Some(46));
                assert!(!game.entity_push(a.entity(), [1.0, 0.0, 0.0]));
            })
            .unwrap();
            assert_eq!(
                game.player.as_ref().unwrap().velocity,
                Vec3::new(0.5, 0.0, 0.0)
            );
        })
        .unwrap();
        assert_eq!(
            b_menu.slot(game, 36),
            Some(Some(
                sunrise_gameplay_common::game::api_values::test_snapshot(&stone)
            ))
        );
        assert_eq!(
            game.server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(13)
                .unwrap()
                .motion
                .as_ref()
                .unwrap()
                .velocity,
            Vec3::new(0.25, 0.5, -0.25)
        );
    });
}

#[test]
#[ignore = "daily"]
fn velocity_writes_send_only_the_owning_player_the_replacement_motion() {
    use std::sync::mpsc;
    use sunrise_protocol::{buf::Reader, packets::play::Clientbound};
    use sunrise_sim::lan::Out;

    if super::native_entity_refs::tests::run_fresh_cases(
        "cut_layout::game::server_side::native_guest_turn_tests::velocity_writes_send_only_the_owning_player_the_replacement_motion",
        "SUNRISE_PLAYER_VELOCITY_CHILD",
        &["players"],
    ) {
        return;
    }
    with_guests(|game| {
        let [a_packets, b_packets] = [12, 13].map(|conn| {
            let (tx, rx) = mpsc::channel();
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(conn, |guest| {
                    guest.sender = PlayerSender::new(tx, Arc::default());
                })
                .unwrap();
            game.as_guest(conn, |game| {
                let body = game.player.as_mut().unwrap();
                body.velocity = Vec3::new(4.0, 8.0, 16.0);
                body.ground.on_ground = false;
            })
            .unwrap();
            rx
        });
        let take_motion = |rx: &mpsc::Receiver<Out>| -> Vec<_> {
            rx.try_iter()
                .flat_map(|out| match out {
                    Out::Packet(body) => vec![body],
                    Out::Packets(bodies) => bodies,
                    Out::Disconnect(_) => Vec::new(),
                })
                .filter_map(|body| {
                    let mut reader = Reader::new(&body);
                    let id = reader.varint().unwrap();
                    match Clientbound::decode(id, reader.rest()).unwrap() {
                        Clientbound::SetEntityMotion(packet) => Some((packet.id, packet.velocity)),
                        _ => None,
                    }
                })
                .collect()
        };
        let a = NativeServerGameExt1::player(game, 12).unwrap().entity();
        let b = NativeServerGameExt1::player(game, 13).unwrap().entity();
        let stale = mod_api::EntityRef::new(mod_api::WorldEpoch::new(a.epoch().get() + 1), a.id());
        game.as_guest(12, |game| {
            assert!(!a.set_velocity(game, [0.0, f64::INFINITY, 0.0]));
            assert!(!stale.set_velocity(game, [0.0; 3]));
            assert_eq!(
                game.player.as_ref().unwrap().velocity,
                Vec3::new(4.0, 8.0, 16.0)
            );
            assert!(a.set_velocity(game, [1.0, 0.0, -1.0]));
            assert!(a.push(game, [0.0, 1.0, 0.0]));
            assert!(b.set_velocity(game, [-2.0, 0.0, 2.0]));
            assert_eq!(
                game.player.as_ref().unwrap().velocity,
                Vec3::new(1.0, 1.0, -1.0)
            );
            game.as_guest(13, |game| {
                assert_eq!(
                    game.player.as_ref().unwrap().velocity,
                    Vec3::new(-2.0, 0.0, 2.0)
                );
                assert!(!a.set_velocity(game, [0.0; 3]));
            })
            .unwrap();
            assert_eq!(
                game.player.as_ref().unwrap().velocity,
                Vec3::new(1.0, 1.0, -1.0)
            );
        })
        .unwrap();
        // The player's own level can be parked while the mod runs in another one.
        game.enter_level(sunrise_gameplay_common::game::Dim::Nether)
            .unwrap();
        let living = NativeEntityRefsCommonGameExt1::living(game, b).unwrap();
        assert!(living.knockback(game, 1.0, [0.0, 1.0]));
        let b_body = game.command_guest(13).unwrap().motion.as_ref().unwrap();
        assert_eq!(b_body.velocity, Vec3::new(-1.0, 0.0, 0.0));
        assert_eq!(game.dim, sunrise_gameplay_common::game::Dim::Nether);
        // A direction too short to normalise is retried from the player's own random
        // (`Entity.random`), never the level's.
        use sunrise_core::random::RandomSource;
        let mut own = b_body.random.clone();
        let mut level = game
            .server
            .levels
            .0
            .parked_mut(sunrise_gameplay_common::game::Dim::Overworld)
            .unwrap()
            .world
            .random
            .clone();
        let (x, z) = loop {
            let x = (own.next_double() - own.next_double()) * 0.01;
            let z = (own.next_double() - own.next_double()) * 0.01;
            if x * x + z * z >= f64::from(1.0e-5_f32) {
                break (x, z);
            }
        };
        assert!(living.knockback(game, 1.0, [0.0, 0.0]));
        let b_body = game.command_guest(13).unwrap().motion.as_ref().unwrap();
        let length = (x * x + z * z).sqrt();
        assert_eq!(
            b_body.velocity,
            Vec3::new(-1.0 / 2.0 - x / length, 0.0, 0.0 / 2.0 - z / length)
        );
        assert_eq!(b_body.random.clone().next_long(), own.next_long());
        assert_eq!(
            game.server
                .levels
                .0
                .parked_mut(sunrise_gameplay_common::game::Dim::Overworld)
                .unwrap()
                .world
                .random
                .clone()
                .next_long(),
            level.next_long()
        );

        assert_eq!(
            take_motion(&a_packets),
            [(12, [1.0, 0.0, -1.0]), (12, [1.0, 1.0, -1.0])]
        );
        // The zero-direction push sends B a third motion packet; its encoding rounds the
        // random direction, so only the owner is compared.
        let b_motion = take_motion(&b_packets);
        assert_eq!(b_motion.len(), 3);
        assert_eq!(
            b_motion[..2],
            [(13, [-2.0, 0.0, 2.0]), (13, [-1.0, 0.0, 0.0])]
        );
        assert_eq!(b_motion[2].0, 13);
    });
}

/// Native services can reach B during A's turn without exposing displaced
/// inventories or changing ordinary command resolution during nested turns.
#[test]
#[ignore = "daily"]
fn another_guests_services_work_during_a_turn() {
    use mod_api::{CriterionContext, CriterionTest, CriterionTestError, TriggerCriterionError};
    use sunrise_command::args::Selector;
    use sunrise_gameplay_common::game::commands_common::Target;
    use sunrise_items::items::ItemStack;
    use sunrise_sim::advancements as adv;

    struct Accept;
    impl CriterionTest for Accept {
        fn test(&mut self, criterion: &dyn CriterionContext) -> Result<bool, CriterionTestError> {
            assert!(criterion.conditions().is_null());
            Ok(true)
        }
    }
    let trigger = mod_api::Identifier::parse("turns:probe").unwrap();
    let vanilla_trigger = mod_api::Identifier::parse("minecraft:tick").unwrap();

    with_guests(|game| {
        game.build_registries();
        game.level.player_uuid = Some([0, 0, 0, 12]);
        let mut tree = adv::Tree::default();
        tree.list.push(adv::Advancement {
            id: "turns:probe".into(),
            parent: None,
            display: None,
            criteria: vec![adv::Criterion {
                name: "seen".into(),
                trigger: "turns:probe".into(),
                conditions: serde_json::Value::Null.into(),
                player: None,
            }],
            requirements: vec![vec!["seen".into()]],
            rewards: Default::default(),
        });
        tree.by_id.insert("turns:probe".into(), 0);
        tree.by_trigger.insert("turns:probe".into(), vec![(0, 0)]);
        game.adv.tree = Arc::new(tree);
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_all(|guest| {
                guest.advancements_loaded = true;
            });
        let stone = ItemStack::new("minecraft:stone", 4);
        let stone_stack = sunrise_gameplay_common::game::api_values::test_snapshot(&stone);
        let a = NativeServerGameExt1::player(game, 12).unwrap();
        let b = NativeServerGameExt1::player(game, 13).unwrap();
        b.set_inventory_slot(game, 0, Some(stone_stack.clone()))
            .unwrap();
        game.as_guest(12, |game| {
            assert_eq!(game.native_entity_target(a.entity()), Some(Target::Player));
            assert_eq!(
                game.select(&Selector {
                    player_name: Some("guest12".into()),
                    ..Default::default()
                }),
                [Target::Player]
            );
            let read_slot =
                |view: mod_api::InventoryView<'_>| view.get(0).and_then(|s| s.project());
            assert_eq!(
                b.with_inventory(game, read_slot),
                Some(Some(stone_stack.clone()))
            );
            b.set_ender_chest_slot(game, 0, Some(stone_stack)).unwrap();
            b.set_inventory_slot(game, 0, None).unwrap();
            assert_eq!(b.with_inventory(game, read_slot), Some(None));
            assert!(b.with_ender_chest(game, read_slot).flatten().is_some());
            assert_eq!(
                a.trigger_criterion(game, &vanilla_trigger, &mut Accept),
                Err(TriggerCriterionError::BuiltinTrigger)
            );
            assert_eq!(b.trigger_criterion(game, &trigger, &mut Accept), Ok(()));
            assert_eq!(a.trigger_criterion(game, &trigger, &mut Accept), Ok(()));
            assert!(game.native_run_command("give guest13 minecraft:diamond 2"));
            assert!(game.native_run_command("give @a[name=guest13] minecraft:diamond 1"));
            assert!(game.native_run_command(
                "execute as 00000000-0000-0000-0000-00000000000d run give @s minecraft:diamond 1"
            ));
            assert!(game.native_run_command("give guest12 minecraft:apple 1"));
            assert_eq!(
                game.inventory[0],
                Some(ItemStack::new("minecraft:apple", 1))
            );
            game.as_guest(13, |game| {
                assert_eq!(a.with_inventory(game, read_slot), None);
                assert_eq!(
                    a.trigger_criterion(game, &trigger, &mut Accept),
                    Err(TriggerCriterionError::PlayerUnavailable)
                );
                assert!(!game.native_run_command("give guest12 minecraft:dirt 1"));
                assert!(!game.native_run_command("give @a[name=guest12] minecraft:dirt 1"));
                let inventory = game.inventory.clone();
                // An empty execute fork succeeds, but must not touch acting B.
                assert!(game.native_run_command(
                    "execute as 00000000-0000-0000-0000-00000000000c run give @s minecraft:dirt 1"
                ));
                assert_eq!(game.inventory, inventory);
                // The native-only filter is restored, including after failure:
                // ordinary names, UUIDs and selectors still see the outer guest.
                for selector in [
                    Selector {
                        player_name: Some("guest12".into()),
                        ..Default::default()
                    },
                    Selector {
                        uuid: Some(12),
                        ..Default::default()
                    },
                    Selector {
                        kind: 'a',
                        names: vec![("guest12".into(), false)],
                        ..Default::default()
                    },
                ] {
                    assert_eq!(game.select(&selector), [Target::Guest(12)]);
                }
                assert_eq!(
                    game.select(&Selector {
                        player_name: Some("guest13".into()),
                        ..Default::default()
                    }),
                    [Target::Player]
                );
            })
            .unwrap();
        })
        .unwrap();
        let host = game.server.lan.as_ref().unwrap();
        assert!(
            host.hub
                .guest(12)
                .unwrap()
                .advancements
                .has("turns:probe", "seen")
        );
        let b = host.hub.guest(13).unwrap();
        assert!(b.advancements.has("turns:probe", "seen"));
        assert_eq!(b.ender_items[0], Some(stone));
        assert_eq!(b.inventory[0], Some(ItemStack::new("minecraft:diamond", 4)));
    });
}

/// The body/saddle store and existing armor/offhand move with nested turns.
#[test]
#[ignore = "daily"]
fn nested_guest_turns_restore_each_equipment_store() {
    use sunrise_items::equipment::{BODY, SADDLE};
    use sunrise_items::items::ItemStack;

    with_guests(|game| {
        let stack = |id| Some(ItemStack::new(id, 1));
        game.armor[0] = stack("minecraft:iron_helmet");
        game.offhand = stack("minecraft:torch");
        game.equipment[BODY] = stack("minecraft:iron_horse_armor");
        for (conn, body, saddle) in [
            (12, "minecraft:golden_horse_armor", "minecraft:saddle"),
            (
                13,
                "minecraft:diamond_horse_armor",
                "minecraft:leather_horse_armor",
            ),
        ] {
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(conn, |guest| {
                    guest.equipment[BODY] = stack(body);
                    guest.equipment[SADDLE] = stack(saddle);
                })
                .unwrap();
        }
        game.as_guest(12, |game| {
            assert_eq!(game.equipment[BODY], stack("minecraft:golden_horse_armor"));
            game.as_guest(13, |game| {
                assert_eq!(game.equipment[BODY], stack("minecraft:diamond_horse_armor"));
                game.equipment[SADDLE] = stack("minecraft:saddle");
            })
            .unwrap();
            assert_eq!(game.equipment[BODY], stack("minecraft:golden_horse_armor"));
            assert_eq!(game.equipment[SADDLE], stack("minecraft:saddle"));
        })
        .unwrap();
        assert_eq!(game.armor[0], stack("minecraft:iron_helmet"));
        assert_eq!(game.offhand, stack("minecraft:torch"));
        assert_eq!(game.equipment[BODY], stack("minecraft:iron_horse_armor"));
        assert_eq!(
            game.server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(13)
                .unwrap()
                .equipment[SADDLE],
            stack("minecraft:saddle")
        );
    });
}

/// `with_own_record` exposes a guest's own ender chest during both their active
/// turn and a nested turn where their inventory is loaned to the inner record.
#[test]
#[ignore = "daily"]
fn with_own_record_swaps_ender_items_across_nested_turns() {
    use sunrise_items::items::ItemStack;

    with_guests(|game| {
        let host_item = Some(ItemStack::new("minecraft:dirt", 1));
        let outer_item = Some(ItemStack::new("minecraft:diamond", 2));
        let inner_item = Some(ItemStack::new("minecraft:emerald", 3));
        let added_item = Some(ItemStack::new("minecraft:gold_ingot", 4));

        game.server.ender_items[0] = host_item.clone();
        for (conn, item) in [(12, outer_item.clone()), (13, inner_item.clone())] {
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(conn, |guest| guest.ender_items[0] = item)
                .unwrap();
        }

        game.as_guest(12, |game| {
            let gi12 = game.guest_index(12).unwrap();
            let active = game.with_own_record(gi12, |host, _, _| {
                Some(host.hub.guests[gi12].ender_items[0].clone())
            });
            assert_eq!(active, Some(outer_item.clone()));

            game.as_guest(13, |game| {
                let gi12 = game.guest_index(12).unwrap();
                let gi13 = game.guest_index(13).unwrap();
                let suspended = game.with_own_record(gi12, |host, _, _| {
                    let seen = host.hub.guests[gi12].ender_items[0].clone();
                    host.hub
                        .guests
                        .update(gi12, |g| g.ender_items[1] = added_item.clone());
                    Some(seen)
                });
                assert_eq!(suspended, Some(outer_item.clone()));

                let inner = game.with_own_record(gi13, |host, _, _| {
                    Some(host.hub.guests[gi13].ender_items[0].clone())
                });
                assert_eq!(inner, Some(inner_item.clone()));
            })
            .unwrap();

            assert_eq!(game.server.ender_items[0], outer_item);
            assert_eq!(game.server.ender_items[1], added_item);
        })
        .unwrap();

        assert_eq!(game.server.ender_items[0], host_item);
        assert_eq!(game.server.ender_items[1], None);
        let host = game.server.lan.as_ref().unwrap();
        assert_eq!(host.hub.guest(12).unwrap().ender_items[0], outer_item);
        assert_eq!(host.hub.guest(12).unwrap().ender_items[1], added_item);
        assert_eq!(host.hub.guest(13).unwrap().ender_items[0], inner_item);
        assert_eq!(host.hub.guest(13).unwrap().ender_items[1], None);
    });
}

/// A merchant's live session belongs to B even while A is acting.
#[test]
#[ignore = "daily"]
fn another_guests_live_merchant_session_is_visible_during_a_turn() {
    use sunrise_sim::merchant::{Session, Trader};
    use {
        sunrise_gameplay::game::server_side::native_merchant_refs::tests::spawn,
        sunrise_gameplay_common::game::server_side::native_merchant_refs::tests::offer,
        sunrise_gameplay_common::game::trading_common::MerchantState,
    };

    with_guests(|game| {
        let villager = spawn(game, 40, "minecraft:villager");
        let old = offer("minecraft:wheat", "minecraft:emerald");
        let mut live = offer("minecraft:coal", "minecraft:diamond");
        // Opening the screen recomputes the special price for this guest.
        live.special_price_diff = 0;
        game.mobs[0].extra.trade = Some(Box::new(Trader {
            offers: vec![old],
            ..Trader::default()
        }));
        game.mobs[0].extra.flag2 = true;
        let mut session = Session::new(vec![live.clone()], true);
        session.xp = 17;
        session.level = 2;
        game.lan_open_merchant(
            13,
            MerchantState {
                session: Box::new(session),
                scroll: 0,
                dragging: false,
                focused: None,
                mob: Some(40),
                name: None,
            },
        );
        game.as_guest(12, |game| {
            assert_eq!(
                villager.offers(game),
                Some(vec![
                    sunrise_gameplay_common::game::api_values::test_snapshot(&live)
                ])
            );
            assert_eq!(villager.xp(game), Some(17));
            assert_eq!(villager.level(game), Some(2));
            assert_eq!(villager.trading_player(game).map(|p| p.uuid()), Some(13));
        })
        .unwrap();
    });
}

/// The passenger walk (horizon-rust/horizon#8383) keeps #228's host visibility.
#[test]
#[ignore = "daily"]
fn another_guest_follows_a_vehicle_teleported_during_a_turn() {
    with_resident_guests(|game| {
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(13, |guest| guest.vehicle = Some(42))
            .unwrap();
        let horse = game.res.mob_types().get("minecraft:horse").unwrap().clone();
        game.mobs
            .push(Mob::new(42, horse, Vec3::new(3.0, 64.0, 5.0), 0.0));
        // Boarding publishes the seat back to the hub after positioning the body.
        game.guest_boarded(13, None);
        let vehicle = NativeEntityRefsCommonGameExt1::entity(game, 42).unwrap();
        let rider = NativeEntityRefsCommonGameExt1::entity(game, 13).unwrap();
        // EntityTypes stores the horse's passenger attachment as a float.
        let before = [3.0, 64.0 + (f64::from(1.44375_f32) - 0.6), 5.0];
        let after = [10.0, 70.0 + (f64::from(1.44375_f32) - 0.6), -4.0];
        game.as_guest(12, |game| {
            assert_eq!(rider.position(game), Some(before));
            assert!(vehicle.teleport_to(game, [10.0, 70.0, -4.0], None));
            assert_eq!(vehicle.position(game), Some([10.0, 70.0, -4.0]));
            assert_eq!(rider.position(game), Some(after));
            assert_eq!(
                game.server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(13)
                    .unwrap()
                    .vehicle,
                Some(42)
            );
        })
        .unwrap();
    });
}

#[test]
#[ignore = "daily"]
fn a_mounted_guests_turn_publishes_its_teleport_without_dismounting() {
    use sunrise_gameplay_common::game::server_side::native_sound_refs::tests::packets;
    use sunrise_protocol::packets::play::{Clientbound as C, relative};

    with_guests(|game| {
        let horse = game.res.mob_types().get("minecraft:horse").unwrap().clone();
        game.mobs
            .push(Mob::new(42, horse, Vec3::new(3.0, 64.0, 5.0), 0.0));
        // Ordinary seating sends no teleport. A passenger transition does,
        // even to the same position; both carry the transition's relative axes.
        for (vehicle, teleported, destination) in [
            (Some(42), false, [10.0, 70.0, -4.0]),
            (Some(42), true, [10.0, 70.0, -4.0]),
            (Some(42), true, [3.0, 64.84375, 5.0]),
            (None, true, [10.0, 70.0, -4.0]),
        ] {
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |guest| guest.vehicle = vehicle)
                .unwrap();
            if vehicle.is_some() {
                game.guest_boarded(13, None);
            }
            let (tx, rx) = std::sync::mpsc::channel();
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |guest| {
                    guest.sender = PlayerSender::new(tx, Arc::default())
                })
                .unwrap();
            game.as_guest(13, |game| {
                // The body left by a passenger transition, before the guest
                // turn publishes it back to the hub and the connection.
                let body = game.player.as_mut().unwrap();
                body.position = Vec3::new(destination[0], destination[1], destination[2]);
                body.teleported = teleported;
                body.teleported_delta = [true, false, true];
                game.yaw = 120.0;
                game.pitch = -15.0;
            })
            .unwrap();
            let guest = game.server.lan.as_ref().unwrap().hub.guest(13).unwrap();
            assert_eq!(guest.vehicle, vehicle);
            let mut positions = Vec::new();
            for packet in packets(&rx) {
                match packet {
                    C::PlayerPosition(packet) => positions.push(packet),
                    C::SetPassengers(_) => panic!("a passenger transition must not dismount"),
                    _ => {}
                }
            }
            assert_eq!(positions.len(), usize::from(teleported), "{vehicle:?}");
            if teleported {
                let packet = &positions[0];
                assert_eq!(packet.position, destination);
                assert_eq!(packet.delta, [0.0; 3]);
                assert_eq!([packet.yaw, packet.pitch], [120.0, -15.0]);
                assert_eq!(packet.relatives, relative::DELTA_X | relative::DELTA_Z);
                assert_eq!(guest.position, destination);
                assert_eq!(
                    guest.awaiting_teleport,
                    Some((packet.teleport, destination))
                );
            }
        }
    });
}

/// The split services retain connected identities even while another player's
/// body is swapped in, and still queue acting-player messages after the hook.
#[test]
#[ignore = "daily"]
fn server_services_preserve_identity_and_message_delivery() {
    use mod_api::{ServerHost, Text};
    use sunrise_protocol::packets::play::Clientbound as C;
    use sunrise_sim::lan::announced_channels::REGISTER;
    with_guests(|game| {
        let guests = &mut game.server.lan.as_mut().unwrap().hub.guests;
        guests.update(0, |g| {
            g.permission = 3;
            g.game_mode = 1;
            g.set_address("192.0.2.12:25565".parse().unwrap());
            g.brand = "test client".into();
            assert!(
                g.sender
                    .announced
                    .apply(REGISTER, b"test:first\0test:second")
            );
        });
        guests.update(1, |g| {
            g.permission = 1;
            g.set_address("[2001:db8::13]:25566".parse().unwrap());
        });
        let players = game.players().server_players();
        assert_eq!(
            players.iter().map(|p| p.uuid()).collect::<Vec<_>>(),
            [12, 13]
        );
        let a = players[0];
        let b = players[1];
        let host: &dyn ServerHost = game;
        assert_eq!(host.players().player_by_name("GUEST12"), Some(a));
        assert_eq!(host.players().player_by_name("missing"), None);
        assert!(game.players().acting_player().is_none());
        assert_eq!(a.name(game), Some("guest12"));
        assert_eq!(a.permission_level(game), Some(3));
        assert_eq!(b.permission_level(game), Some(1));
        assert_eq!(a.can_use_game_master_blocks(game), Some(true));
        assert_eq!(a.address(game), Some("192.0.2.12"));
        assert_eq!(b.address(game), Some("2001:db8::13"));
        assert_eq!(a.client_brand(game), Some("test client"));
        assert_eq!(b.client_brand(game), Some(""));
        let channels = a.announced_channels(game).unwrap();
        assert_eq!(channels.as_ref(), ["test:first", "test:second"]);
        let stale = mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(
                mod_api::WorldEpoch::new(a.entity().epoch().get() + 1),
                a.entity().id(),
            ),
            a.uuid(),
        );
        let wrong_entity = mod_api::__private::game::player_ref(b.entity(), a.uuid());
        let wrong_uuid = mod_api::__private::game::player_ref(a.entity(), 99);
        for invalid in [stale, wrong_entity, wrong_uuid] {
            assert_eq!(invalid.address(game), None);
            assert_eq!(invalid.permission_level(game), None);
        }
        assert!(game.commands().run_command("list"));
        game.server.chat_capture = Some(Vec::new());
        game.commands()
            .feedback(Text::literal("command answer"), false)
            .unwrap();
        assert_eq!(
            game.server.chat_capture.take().unwrap()[0].plain(),
            "command answer"
        );
        game.as_guest(12, |game| {
            assert_eq!(game.players().acting_player(), Some(a));
            assert_eq!(game.players().server_players(), players);
            assert_eq!(a.address(game), Some("192.0.2.12"));
            assert_eq!(b.address(game), Some("2001:db8::13"));
            assert_eq!(a.permission_level(game), Some(3));
            assert_eq!(b.permission_level(game), Some(1));
            assert_eq!(a.can_use_game_master_blocks(game), Some(true));
            // The command source keeps its snapshot while the live permission changes.
            for (permission, can_use) in [(1, false), (3, true)] {
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| guest.permission = permission)
                    .unwrap();
                assert_eq!(a.permission_level(game), Some(permission));
                assert_eq!(a.can_use_game_master_blocks(game), Some(can_use));
            }
            assert!(a.tell(game, Text::literal("chat")).unwrap());
            assert!(a.action_bar(game, Text::literal("overlay")).unwrap());
            assert!(
                a.tab_list(game, &Text::literal("header"), &Text::literal("footer"))
                    .unwrap()
            );
            let packets = &game.server.guest_packets;
            assert!(matches!(&packets[packets.len() - 3], C::SystemChat(p) if !p.overlay));
            assert!(matches!(&packets[packets.len() - 2], C::SystemChat(p) if p.overlay));
            assert!(matches!(&packets[packets.len() - 1], C::TabList(_)));
            assert!(b.tell(game, Text::literal("other player")).unwrap());
            assert!(b.set_nickname(game, Some("B")));
            assert!(b.set_sleep_ignored(game, true));
            game.as_guest(13, |game| {
                assert_eq!(game.players().acting_player(), Some(b));
                assert_eq!(game.players().server_players(), players);
                assert_eq!(a.name(game), Some("guest12"));
                assert_eq!(a.permission_level(game), Some(3));
                assert_eq!(b.permission_level(game), Some(1));
                assert_eq!(a.address(game), Some("192.0.2.12"));
                assert_eq!(b.address(game), Some("2001:db8::13"));
                assert_eq!(a.client_brand(game), Some("test client"));
                assert!(Arc::ptr_eq(&channels, &a.announced_channels(game).unwrap()));
                assert_eq!(a.game_mode(game), None, "body remains displaced");
                assert_eq!(a.can_use_game_master_blocks(game), None);
                for invalid in [stale, wrong_entity, wrong_uuid] {
                    assert_eq!(invalid.address(game), None);
                    assert_eq!(invalid.permission_level(game), None);
                }
                assert!(
                    a.tell(game, Text::literal("displaced but connected"))
                        .unwrap()
                );
            })
            .unwrap();
        })
        .unwrap();
        // Vanish is UUID-persistent, not a live-body operation.
        game.server.lan.as_mut().unwrap().hub.guests.clear();
        assert!(a.hide(game, Some(2)));
        assert_eq!(
            game.server.lan.as_ref().unwrap().hub.hidden.get(&12),
            Some(&2)
        );
        assert!(!a.tell(game, Text::literal("offline")).unwrap());
        assert!(!a.set_nickname(game, Some("offline")));
        assert!(!a.set_sleep_ignored(game, true));
        assert_eq!(a.address(game), None);
        assert_eq!(a.client_brand(game), None);
        assert!(a.announced_channels(game).is_none());
        assert_eq!(channels.as_ref(), ["test:first", "test:second"]);
        assert_eq!(a.permission_level(game), None);
        assert!(!stale.hide(game, None));
        assert_eq!(stale.name(game), None);
    });
}

/// Permission reads keep the existing owner and fake-player contracts, not
/// just the connected-player list that the services enumerate.
#[test]
#[ignore = "daily"]
fn canonical_permissions_preserve_owner_and_fake_player_reads() {
    use mod_api::GameMode;
    use sunrise_sim::motion::PlayerMotion;

    with_guests(|game| {
        let _serving =
            sunrise_gameplay::game::server_side::integrated::tests::install_serving(game, false);
        game.level.player_uuid = Some([0, 0, 0, 99]);
        let mut body = PlayerMotion::new(game.motion, Vec3::new(0.0, 64.0, 0.0)).unwrap();
        body.entity_id = 99;
        game.player = Some(body);
        game.set_mode(sunrise_command::args::GameMode::Creative);
        let owner = NativeServerGameExt1::player(game, 99).unwrap();
        for (allow, permission) in [(false, 0), (true, 4)] {
            game.set_allow_commands(allow);
            assert_eq!(owner.permission_level(game), Some(permission));
            assert_eq!(owner.can_use_game_master_blocks(game), Some(allow));
        }
        game.set_mode(sunrise_command::args::GameMode::Survival);
        assert_eq!(owner.permission_level(game), Some(4));
        assert_eq!(owner.can_use_game_master_blocks(game), Some(false));

        game.server.next_entity = 100;
        let level = NativeEntityRefsCommonGameExt1::level(game, game.dim.id()).unwrap();
        let fake = game.fake_player(level, 1234, "machine").unwrap();
        assert!(!game.players().server_players().contains(&fake));
        assert_eq!(game.players().player_by_name("machine"), None);
        assert_eq!(fake.permission_level(game), Some(0));
        assert_eq!(
            fake.address(game),
            None,
            "no connection outside an acting turn"
        );
        assert_eq!(fake.can_use_game_master_blocks(game), Some(false));
        assert!(fake.set_game_mode(game, GameMode::Creative));
        assert_eq!(fake.permission_level(game), Some(0));
        assert_eq!(fake.can_use_game_master_blocks(game), Some(false));
        let conn = game
            .server
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guests
            .iter()
            .find(|g| g.uuid == fake.uuid())
            .unwrap()
            .conn;
        game.as_guest(conn, |game| {
            assert_eq!(fake.permission_level(game), Some(0));
            assert_eq!(fake.address(game), Some(""), "unknown acting address");
            assert_eq!(fake.client_brand(game), Some(""));
            let empty = fake.announced_channels(game).unwrap();
            assert!(empty.is_empty());
            assert!(Arc::ptr_eq(&empty, &fake.announced_channels(game).unwrap()));
            assert_eq!(fake.can_use_game_master_blocks(game), Some(false));
        })
        .unwrap();
        assert!(game.player_remove_fake(fake));
        assert_eq!(fake.permission_level(game), None);
        assert_eq!(fake.address(game), None);
    });
}

/// A turn lists the acting player once, under their own UUID and with their
/// live body. The host's body is displaced for the turn, so it is not listed.
#[test]
#[ignore = "daily"]
fn level_players_name_each_body_once_during_turns() {
    with_guests(|game| {
        place_host(game);
        assert_eq!(level_uuids(game), [99, 12, 13]);
        game.as_guest(12, |game| {
            assert_eq!(level_uuids(game), [12, 13]);
            assert_eq!(game.player_view().unwrap().uuid, [0, 0, 0, 12]);
            game.player.as_mut().unwrap().position.x = 40.0;
            assert_eq!(game.level_players()[0].position.x, 40.0);
            game.as_guest(13, |game| {
                assert_eq!(level_uuids(game), [13, 12]);
                assert!(game.as_guest(12, |_| ()).is_none());
            })
            .unwrap();
            assert_eq!(level_uuids(game), [12, 13]);
        })
        .unwrap();
        assert_eq!(level_uuids(game), [99, 12, 13]);
    });
}

#[test]
#[ignore = "daily"]
fn level_players_exclude_guests_in_other_levels() {
    with_guests(|game| {
        place_host(game);
        let host = game.server.lan.as_mut().unwrap();
        let elsewhere = host.hub.guests.connection_index(13).unwrap();
        host.hub.guests.update(elsewhere, |guest| {
            guest.dimension = "minecraft:the_nether".to_owned();
        });
        assert_eq!(level_uuids(game), [99, 12]);
        game.as_guest(12, |game| assert_eq!(level_uuids(game), [12]))
            .unwrap();
    });
}

#[test]
#[ignore = "daily"]
fn level_players_exclude_fake_players() {
    with_guests(|game| {
        let _serving =
            sunrise_gameplay::game::server_side::integrated::tests::install_serving(game, false);
        place_host(game);
        game.server.next_entity = 100;
        let level = NativeEntityRefsCommonGameExt1::level(game, game.dim.id()).unwrap();
        let fake = game.fake_player(level, 1234, "machine").unwrap();
        let conn = game
            .server
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guests
            .iter()
            .find(|g| g.uuid == fake.uuid())
            .unwrap()
            .conn;
        assert_eq!(level_uuids(game), [99, 12, 13]);
        game.as_guest(conn, |game| {
            assert_eq!(level_uuids(game), [12, 13]);
            game.as_guest(12, |game| assert_eq!(level_uuids(game), [12, 13]))
                .unwrap();
        })
        .unwrap();
        assert_eq!(level_uuids(game), [99, 12, 13]);
    });
}

/// A listener sees the newly granted criterion before its recipe reward on
/// both a player's first tick and a resource reload.
#[test]
#[ignore = "daily"]
fn tick_advancement_notice_precedes_recipe_reward() {
    use mod_api::{Hooks, ModEntry, Mods, PlayerAdvancementProgressEvent, ServerHost};
    use std::sync::Mutex;
    use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;

    if run_fresh_cases(
        "cut_layout::game::server_side::native_guest_turn_tests::tick_advancement_notice_precedes_recipe_reward",
        "SUNRISE_ADVANCEMENT_ORDER_CHILD",
        &["first", "reload"],
    ) {
        return;
    }
    #[derive(Clone)]
    struct Observer(Arc<Mutex<Vec<Option<bool>>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Observer {
        fn player_advancement_progress(
            &self,
            host: &mut dyn ServerHost,
            event: &mut PlayerAdvancementProgressEvent<'_>,
        ) {
            if event.advancement == "testmod:tick" {
                self.0.lock().unwrap().push(
                    host.player(event.player)
                        .and_then(|p| p.has_discovered_recipe(host, "testmod:reward")),
                );
            }
        }
    }
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mods: Mods = vec![ModEntry::new(
        "observer",
        Box::new(Observer(observed.clone())),
    )]
    .into();
    mod_platform::__private::install(mods);
    let scratch =
        std::env::temp_dir().join(format!("sunrise-tick-advancement-{}", std::process::id()));
    let dir = scratch.join("data/testmod/advancement");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tick.json"), r#"{"criteria":{"seen":{"trigger":"minecraft:tick"}},"rewards":{"recipes":["testmod:reward"]}}"#).unwrap();
    with_guests(|game| {
        let mut sync = sunrise_sim::lan::data::SyncData::empty();
        sync.advancements = sunrise_sim::lan::ServerAdvancements::load(
            &scratch,
            &sunrise_world::loot::PackOverlay::NONE,
            // Rewards must name a recipe registered in the server's index.
            sunrise_sim::lan::recipes::BookProjections::single_shapeless("testmod:reward"),
        );
        let reload = std::env::var("SUNRISE_ADVANCEMENT_ORDER_CHILD").as_deref() == Ok("reload");
        let host = game.server.lan.as_mut().unwrap();
        host.hub.guests.update_all(|guest| {
            guest.advancements_loaded = reload;
        });
        host.hub.mods_loaded = reload;
        game.with_advancement_hub(|hub, owners| hub.reload_resources(Arc::new(sync), owners));
        if reload {
            game.run_native_advancement_grants();
        } else {
            game.tick_lan(false);
        }
        let host = game.server.lan.as_ref().unwrap();
        assert!(
            host.hub
                .guest(12)
                .unwrap()
                .recipe_book
                .knows("testmod:reward")
        );
        assert_eq!(*observed.lock().unwrap(), vec![Some(false), Some(false)]);
    });
    std::fs::remove_dir_all(scratch).unwrap();
}

/// `PlayerRef::set_selected_slot` from inside the guest's own turn: their state is in place, so
/// the slot is written there, and their client is still told (the put-back at the end of `guest_turn` sends nothing).
#[test]
#[ignore = "daily"]
fn set_selected_slot_tells_an_acting_guests_client() {
    with_guests(|game| {
        let (tx, rx) = std::sync::mpsc::channel();
        let host = game.server.lan.as_mut().unwrap();
        let index = host.hub.guests.iter().position(|g| g.conn == 13).unwrap();
        host.hub.guests.update(index, |guest| {
            guest.sender = PlayerSender::new(tx, Arc::default())
        });
        game.as_guest(13, |game| {
            let player = NativeServerGameExt1::player(game, 13).unwrap();
            while rx.try_recv().is_ok() {}
            assert!(player.set_selected_slot(game, 8));
            assert!(
                rx.try_recv().is_ok(),
                "SetHeldSlot reaches the acting guest's client"
            );
            assert!(!player.set_selected_slot(game, 9));
        })
        .unwrap();
        assert_eq!(
            game.server.lan.as_ref().unwrap().hub.guests[index].selected,
            8
        );
        // Parked again: the write goes to the guest record and is sent the same way.
        let player = NativeServerGameExt1::player(game, 13).unwrap();
        while rx.try_recv().is_ok() {}
        assert!(player.set_selected_slot(game, 0));
        assert!(
            rx.try_recv().is_ok(),
            "SetHeldSlot reaches a parked guest's client"
        );
        assert_eq!(
            game.server.lan.as_ref().unwrap().hub.guests[index].selected,
            0
        );
    });
}

/// `PlayerRef::drop_stack` hands a refused stack back: an empty one, and one a mod's `item_toss`
/// vetoes, which the game itself would discard.
#[test]
#[ignore = "daily"]
fn a_vetoed_drop_gives_the_stack_back() {
    use mod_api::{Flow, Hooks, ItemTossEvent, ModEntry, ServerHost};
    use mod_platform::testing::with_mods;
    use sunrise_gameplay_common::game::api_values::TryIntoApi;
    use sunrise_items::items::ItemStack;

    struct Veto;
    #[mod_api::__subscriptions]
    impl Hooks for Veto {
        fn item_toss(&self, _: &mut dyn ServerHost, _: &mut ItemTossEvent) -> Flow {
            Flow::Cancel
        }
    }
    let mods = vec![ModEntry::new("veto", Box::new(Veto))];
    with_mods(mods.into(), || {
        with_guests(|game| {
            let player = NativeServerGameExt1::player(game, 13).unwrap();
            let stone = ItemStack::new("minecraft:stone", 3);
            let before = game.dropped.len();
            assert_eq!(
                player.drop_stack(game, stone.try_into_api().unwrap(), true),
                Ok(Some(stone.try_into_api().unwrap())),
                "a vetoed toss comes back whole"
            );
            assert_eq!(game.dropped.len(), before, "and makes no item entity");
            let empty = ItemStack::new("minecraft:stone", 0);
            assert_eq!(
                game.player_drop_stack(player, empty.clone(), false),
                Some(empty)
            );
        });
    });
}

pub(crate) struct PanicOnDamage(pub(crate) Arc<AtomicUsize>);

#[mod_api::__subscriptions]
impl mod_api::Hooks for PanicOnDamage {
    fn living_incoming_damage(
        &self,
        _: &mut dyn mod_api::HitHost,
        _: &mut mod_api::LivingIncomingDamageEvent<'_>,
    ) -> mod_api::Flow {
        self.0.fetch_add(1, Ordering::SeqCst);
        panic!("damage listener panic");
    }
}

#[test]
#[ignore = "daily"]
fn guest_damage_unwind_restores_and_next_same_connection_hit_lands() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "panic_once",
        Box::new(PanicOnDamage(calls.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_guests(|game| {
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |guest| {
                    guest.loaded = true;
                    guest.load_timeout = 0;
                })
                .unwrap();
            game.as_guest(13, |g| {
                use sunrise_command::args::GameMode;
                g.set_game_mode(GameMode::Creative);
                g.set_game_mode(GameMode::Survival);
            })
            .unwrap();
            let outer = game.server.lan_acting;
            let position = game.player.as_ref().map(|p| p.position);
            let health = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                game.server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(13)
                    .unwrap()
                    .motion
                    .as_ref()
                    .unwrap()
                    .health
                    .health()
            };
            let before = health(game);
            let hit = |g: &mut sunrise_gameplay::game::ServerGame<'_>| {
                g.pvp_land(
                    sunrise_gameplay::game::server_side::lan_pvp::Victim::Guest(13),
                    &sunrise_gameplay::game::server_side::lan_pvp::Strike::test_blow(12),
                )
                .is_some()
            };
            assert!(hit(game));
            assert_eq!(health(game), before - 1.0);
            assert_eq!(
                take_hook_panics(),
                [HookPanicReport {
                    mod_id: "panic_once",
                    hook: "living_incoming_damage",
                }],
            );
            assert!(game.server.lan_guest_swaps.is_empty());
            assert!(game.server.interrupted_turns.is_empty());
            assert_eq!(game.server.lan_acting, outer);
            assert_eq!(game.player.as_ref().map(|p| p.position), position);

            expire_guest_damage_cooldown(game, 13);
            assert!(hit(game));
            assert_eq!(health(game), before - 2.0);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert!(take_hook_panics().is_empty());
            assert!(game.server.lan_guest_swaps.is_empty());
            assert_eq!(game.server.lan_acting, outer);
            assert_eq!(game.player.as_ref().map(|p| p.position), position);
            assert_eq!(
                game.as_guest(13, |g| {
                    assert!(g.as_guest(13, |_| ()).is_none());
                    42
                }),
                Some(42)
            );
            assert_eq!(game.as_guest(u32::MAX, |_| 0), None);
        })
    });
}

#[test]
#[ignore = "daily"]
/// A contained listener panic and an unwinding engine panic both leave the outer capture in
/// place. The same connection's next messaged turn still runs (#2244).
fn guest_message_capture_is_restored_after_a_real_native_panic() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "capture_repro",
        Box::new(PanicOnDamage(calls.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_guests(|game| {
            for conn in [12, 13] {
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(conn, |guest| {
                        guest.loaded = true;
                        guest.load_timeout = 0;
                    })
                    .unwrap();
                game.as_guest(conn, |g| {
                    use sunrise_command::args::GameMode;
                    g.set_game_mode(GameMode::Creative);
                    g.set_game_mode(GameMode::Survival);
                })
                .unwrap();
            }
            let outer = game.server.lan_acting;
            let feedback = || vec![sunrise_game_data::text::Text::literal("outer feedback")];
            game.server.chat_capture = Some(feedback());
            let hit = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                assert_eq!(game.server.chat_capture, Some(Vec::new()));
                game.pvp_land(
                    sunrise_gameplay::game::server_side::lan_pvp::Victim::Guest(12),
                    &sunrise_gameplay::game::server_side::lan_pvp::Strike::test_blow(13),
                )
                .is_some()
            };
            assert_eq!(game.as_guest_messaged(13, hit), Some(true));
            assert_eq!(
                take_hook_panics(),
                [HookPanicReport {
                    mod_id: "capture_repro",
                    hook: "living_incoming_damage",
                }],
            );
            assert_eq!(game.server.chat_capture, Some(feedback()));
            assert!(game.server.lan_guest_swaps.is_empty());
            assert!(game.server.interrupted_turns.is_empty());
            assert_eq!(game.server.lan_acting, outer);

            let panic = catch_unwind(AssertUnwindSafe(|| {
                game.as_guest_messaged(13, |game| {
                    game.server
                        .chat_capture
                        .as_mut()
                        .unwrap()
                        .push(sunrise_game_data::text::Text::literal("guest feedback"));
                    panic!("engine panic during a messaged turn");
                })
            }))
            .unwrap_err();
            assert_eq!(
                panic.downcast_ref::<&str>(),
                Some(&"engine panic during a messaged turn"),
            );
            assert_eq!(game.server.chat_capture, Some(feedback()));
            assert!(game.server.lan_guest_swaps.is_empty());
            assert_eq!(game.server.lan_acting, outer);

            expire_guest_damage_cooldown(game, 12);
            assert_eq!(game.as_guest_messaged(13, hit), Some(true));
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert!(take_hook_panics().is_empty());
            assert_eq!(game.server.chat_capture, Some(feedback()));
            assert!(game.server.lan_guest_swaps.is_empty());
            assert_eq!(game.server.lan_acting, outer);
        });
    });
}

pub(crate) struct PanicOnFood(pub(crate) Arc<AtomicUsize>);

#[mod_api::__subscriptions]
impl mod_api::Hooks for PanicOnFood {
    fn food_level_changed(
        &self,
        _: &mut dyn mod_api::ServerHost,
        _: &mut mod_api::FoodLevelChangedEvent<'_>,
    ) -> mod_api::Flow {
        self.0.fetch_add(1, Ordering::SeqCst);
        panic!("food listener panic");
    }
}

/// A contained food-listener panic at turn end does not block write-back (#2240).
/// A later engine panic still unwinds after the guest is restored.
#[test]
#[ignore = "daily"]
fn a_panic_in_a_turn_end_callback_still_restores_the_guest() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "turn_end_panic",
        Box::new(PanicOnFood(calls.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_guests(|game| {
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |guest| {
                    guest.loaded = true;
                    guest.load_timeout = 0;
                })
                .unwrap();
            let food = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                game.server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(13)
                    .unwrap()
                    .food
            };
            let before = food(game);
            let outer = game.server.lan_acting;
            assert_eq!(
                game.as_guest(13, |g| g.player.as_mut().unwrap().health.food.food -= 3),
                Some(()),
            );
            assert_eq!(
                take_hook_panics(),
                [HookPanicReport {
                    mod_id: "turn_end_panic",
                    hook: "food_level_changed",
                }],
            );
            assert!(game.server.lan_guest_swaps.is_empty());
            assert!(game.server.interrupted_turns.is_empty());
            assert_eq!(game.server.lan_acting, outer);
            assert_eq!(food(game), before - 3);

            let panic = catch_unwind(AssertUnwindSafe(|| {
                game.as_guest(13, |g| {
                    g.player.as_mut().unwrap().health.food.food -= 1;
                    panic!("engine panic during a guest turn");
                })
            }))
            .unwrap_err();
            assert_eq!(
                panic.downcast_ref::<&str>(),
                Some(&"engine panic during a guest turn"),
            );
            assert!(game.server.lan_guest_swaps.is_empty());
            assert_eq!(game.server.lan_acting, outer);
            assert_eq!(food(game), before - 4);

            assert_eq!(
                game.as_guest(13, |g| {
                    g.player.as_mut().unwrap().health.food.food -= 1;
                    7
                }),
                Some(7),
                "the same connection's next turn runs",
            );
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert!(take_hook_panics().is_empty());
            assert_eq!(food(game), before - 5);
            assert!(game.server.lan_guest_swaps.is_empty());
            assert_eq!(game.server.lan_acting, outer);
        });
    });
}

/// `withPermission(level)`: the line is checked at the level the mod gives, whatever the
/// entity's own (a guest's is 0 here), with no ceiling. `TickCommand` needs admins (3), so game
/// masters (2) are refused it and admins run it; `GiveCommand` needs game masters.
#[test]
#[ignore = "daily"]
fn run_command_checks_the_line_at_the_level_given() {
    use mod_api::PermissionLevel::{Admins, All, Gamemasters, Owners};

    with_command_guests("run_command_checks_the_line_at_the_level_given", |game| {
        game.build_registries();
        let a = NativeServerGameExt1::player(game, 12).unwrap().entity();
        assert!(!a.run_command(game, "give @s minecraft:diamond 1", All, false));
        assert!(a.run_command(game, "give @s minecraft:diamond 1", Gamemasters, false));
        assert!(!a.run_command(game, "tick query", Gamemasters, false));
        assert!(a.run_command(game, "tick query", Admins, false));
        assert!(a.run_command(game, "tick query", Owners, false));
        let stand = summon_stand(game);
        assert!(!stand.run_command(game, "tick query", Gamemasters, false));
        assert!(stand.run_command(game, "tick query", Admins, false));
    });
}

/// The entity is the executor, from its own position in its level: `@s` is it and `~` its
/// position, not the server spawn's (from there the stand would end at (2, 64, -1)). A player
/// out of a turn, the acting player and another player during that turn are each `@s`.
#[test]
#[ignore = "daily"]
fn run_command_runs_as_the_entity_from_where_it_is() {
    use mod_api::PermissionLevel::Gamemasters;
    use sunrise_items::items::ItemStack;

    with_command_guests("run_command_runs_as_the_entity_from_where_it_is", |game| {
        game.build_registries();
        let a = NativeServerGameExt1::player(game, 12).unwrap().entity();
        let b = NativeServerGameExt1::player(game, 13).unwrap().entity();
        let holds = |game: &sunrise_gameplay::game::ServerGame<'_>, conn: u32, item: &str| {
            let stack = ItemStack::new(item, 1);
            game.server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(conn)
                .unwrap()
                .inventory
                .iter()
                .flatten()
                .any(|s| *s == stack)
        };
        let stand = summon_stand(game);
        let guest = |game: &sunrise_gameplay::game::ServerGame<'_>, conn: u32| {
            game.server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(conn)
                .unwrap()
                .position
        };

        assert!(stand.run_command(game, "/tp @s ~2 ~ ~-1", Gamemasters, false));
        assert_eq!(stand.position(game), Some([3.5, 70.0, 0.5]));
        assert_eq!(guest(game, 12), [3.0, 64.0, 5.0]);
        assert_eq!(guest(game, 13), [3.0, 64.0, 5.0]);

        assert!(a.run_command(game, "give @s minecraft:diamond 1", Gamemasters, false));
        game.as_guest(12, |game| {
            assert!(a.run_command(game, "give @s minecraft:apple 1", Gamemasters, false));
            assert!(b.run_command(game, "give @s minecraft:stick 1", Gamemasters, false));
        })
        .unwrap();
        assert!(holds(game, 12, "minecraft:diamond"));
        assert!(holds(game, 12, "minecraft:apple"));
        assert!(holds(game, 13, "minecraft:stick"));
        assert!(!holds(game, 13, "minecraft:diamond"));
        assert!(!holds(game, 12, "minecraft:stick"));
    });
}

/// A player's source answers them as their own command does, feedback and failure both
/// (`ServerPlayer.commandSource`); another entity's says nothing to anyone, not even the
/// player whose turn it runs in (`CommandSource.NULL`).
#[test]
#[ignore = "daily"]
fn run_command_answers_a_player_and_no_one_for_another_entity() {
    use mod_api::PermissionLevel::Admins;

    let name = "run_command_answers_a_player_and_no_one_for_another_entity";
    with_command_guests(name, |game| {
        game.build_registries();
        let a = NativeServerGameExt1::player(game, 12).unwrap().entity();
        let stand = summon_stand(game);
        game.as_guest(12, |game| {
            for (line, ran) in [("tick query", true), ("kill @e[type=minecraft:pig]", false)] {
                let before = game.server.guest_packets.len();
                assert_eq!(a.run_command(game, line, Admins, false), ran, "{line}");
                assert!(told_since(game, before), "the player is told: {line}");
                let before = game.server.guest_packets.len();
                assert_eq!(stand.run_command(game, line, Admins, false), ran, "{line}");
                assert!(!told_since(game, before), "no one is told: {line}");
            }
        })
        .unwrap();
    });
}

/// A line that doesn't parse, or parses and fails, answers false without a panic, and so does a
/// handle of an earlier open or of an entity that is gone.
#[test]
#[ignore = "daily"]
fn run_command_answers_false_for_a_bad_line_or_a_stale_handle() {
    use mod_api::PermissionLevel::Owners;

    let name = "run_command_answers_false_for_a_bad_line_or_a_stale_handle";
    with_command_guests(name, |game| {
        game.build_registries();
        let a = NativeServerGameExt1::player(game, 12).unwrap().entity();
        let stand = summon_stand(game);
        for entity in [a, stand] {
            for line in [
                "",
                "/",
                "no_such_command",
                "give @s minecraft:diamond many",
                "tp @s ~ ~ ~ ~ ~ ~ ~",
                "kill @e[type=minecraft:pig]",
            ] {
                assert!(!entity.run_command(game, line, Owners, false), "{line:?}");
            }
        }
        let stale = mod_api::EntityRef::new(mod_api::WorldEpoch::new(a.epoch().get() + 1), a.id());
        assert!(!stale.run_command(game, "tick query", Owners, false));
        assert!(stand.run_command(game, "kill @s", Owners, false));
        assert!(!stand.run_command(game, "tick query", Owners, false));
    });
}

/// A live player in another dimension is outside the level in place: that level is parked
/// while the hook runs, so the call answers false and nothing runs, neither the part about
/// the player (`give @s`) nor the part about the server (`gamerule`). Back in the level in
/// place, the same player and line run.
#[test]
#[ignore = "daily"]
fn run_command_refuses_a_player_whose_level_is_parked() {
    use mod_api::PermissionLevel::Owners;
    use sunrise_game_data::gamerules::vanilla::SEND_COMMAND_FEEDBACK;
    use sunrise_items::items::ItemStack;

    let name = "run_command_refuses_a_player_whose_level_is_parked";
    with_command_guests(name, |game| {
        game.build_registries();
        let b = NativeServerGameExt1::player(game, 13).unwrap().entity();
        let move_to = |game: &mut sunrise_gameplay::game::ServerGame<'_>, dimension: &str| {
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |guest| guest.dimension = dimension.to_owned())
                .unwrap();
        };
        let has_stick = |game: &sunrise_gameplay::game::ServerGame<'_>| {
            let stick = ItemStack::new("minecraft:stick", 1);
            game.server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(13)
                .unwrap()
                .inventory
                .iter()
                .flatten()
                .any(|s| *s == stick)
        };
        assert!(game.ui.rules.bool_at(SEND_COMMAND_FEEDBACK));

        move_to(game, "minecraft:the_nether");
        assert!(!b.run_command(game, "give @s minecraft:stick 1", Owners, false));
        assert!(!b.run_command(game, "gamerule send_command_feedback false", Owners, false));
        assert!(game.ui.rules.bool_at(SEND_COMMAND_FEEDBACK));

        move_to(game, "minecraft:overworld");
        assert!(!has_stick(game));
        assert!(b.run_command(game, "give @s minecraft:stick 1", Owners, false));
        assert!(has_stick(game));
    });
}

/// `ServerPlayer.createCommandSourceStack`: a player's source is their own position and
/// rotation, also when the mod calls from inside an outer command run from elsewhere
/// (`execute positioned 50 80 50 rotated 90 30 run <a mod's command>`), for a player entered
/// for it and for the acting player in their own turn. From the outer source, `~1 ~ ~ ~10 ~`
/// would put them at (51, 80, 50) facing yaw 100, pitch 30.
#[test]
#[ignore = "daily"]
fn run_command_runs_from_the_players_own_place_inside_an_outer_command() {
    use mod_api::PermissionLevel::Gamemasters;
    use sunrise_gameplay::game::server_side::commands::execute::Source;

    let name = "run_command_runs_from_the_players_own_place_inside_an_outer_command";
    with_command_guests(name, |game| {
        game.build_registries();
        let a = NativeServerGameExt1::player(game, 12).unwrap().entity();
        let b = NativeServerGameExt1::player(game, 13).unwrap().entity();
        let place = |game: &sunrise_gameplay::game::ServerGame<'_>, conn: u32| {
            let g = game.server.lan.as_ref().unwrap().hub.guest(conn).unwrap();
            (g.position, [g.yaw, g.pitch])
        };
        let (start, [yaw, pitch]) = place(game, 12);
        assert_eq!(place(game, 13), (start, [yaw, pitch]));
        let outer = Source {
            pos: [50.0, 80.0, 50.0],
            rot: [90.0, 30.0],
            ..game.server_source()
        };
        game.with_source(outer, |game| {
            assert!(b.run_command(game, "tp @s ~1 ~ ~ ~10 ~", Gamemasters, false));
            game.as_guest(12, |game| {
                assert!(a.run_command(game, "tp @s ~1 ~ ~ ~10 ~", Gamemasters, false));
            })
            .unwrap();
        });
        for conn in [12, 13] {
            assert_eq!(
                place(game, conn),
                ([start[0] + 1.0, start[1], start[2]], [yaw + 10.0, pitch]),
                "guest {conn}"
            );
        }
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;

use sunrise_gameplay_common::game::native_menu_refs_common::NativeMenuRefsCommonGameExt1;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1;
