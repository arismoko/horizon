use crate::game::lan_actions::pickups::pickup_post_tests::CASE;
use crate::game::lan_actions::pickups::pickup_post_tests::Case;
use crate::game::lan_actions::pickups::pickup_post_tests::Probe;
use crate::game::lan_actions::pickups::pickup_post_tests::arm;
use crate::game::lan_actions::pickups::pickup_post_tests::clear_ground;
use crate::game::lan_actions::pickups::pickup_post_tests::ground;
use crate::game::lan_actions::pickups::pickup_post_tests::host_case;
use crate::game::lan_actions::pickups::pickup_post_tests::host_stat;
use crate::game::lan_actions::pickups::pickup_post_tests::with_probe;
use mod_api::TriState;
use sunrise_core::geometry::Vec3;
use sunrise_items::items::ItemStack;
use sunrise_sim::motion::PlayerMotion;

/// Both pickup hooks name the ground item entity, and `item_picked_up`
/// reads its whole stack through the handle, components included.
#[test]
#[ignore = "daily"]
fn guest_pickup_hooks_name_the_ground_item_entity() {
    let probe = Probe::default();
    with_probe(&probe, || {
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            clear_ground(game);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| guest.inventory.fill(None))
                .unwrap();
            arm(&probe, &CASE);
            let mut pick = ItemStack::new("minecraft:diamond_pickaxe", 1);
            pick.damage = 17;
            pick.data_mut().custom_name = Some(sunrise_core::nbt::Tag::String("Named pick".into()));
            game.spawn_item(pick.clone(), [3.0, 64.0, 5.0], [0.0; 3], 0);
            let id = game.server.next_entity;
            let gi = game.guest_index(12).unwrap();
            game.guest_pickups(gi);
            let state = probe.0.lock().unwrap();
            assert_eq!(
                state.touched,
                vec![(
                    id,
                    Some(sunrise_gameplay_common::game::api_values::test_snapshot(
                        &pick
                    ))
                )]
            );
            assert_eq!(state.posted, vec![id]);
            assert_eq!(ground(game, id), None, "the pickaxe was taken");
        });
    });
}

#[test]
#[ignore = "daily"]
fn host_pickup_tells_whole_and_partial_pickups_and_keeps_listener_changes() {
    let probe = Probe::default();
    with_probe(&probe, || {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let scratch = std::path::PathBuf::from(
                std::env::var_os("SUNRISE_HOME").expect("needs an isolated SUNRISE_HOME"),
            )
            .join(format!("pickup-post-{}", std::process::id()));
            game.server.world_root = scratch.clone();
            game.enter_level(sunrise_gameplay_common::game::Dim::Overworld)
                .unwrap();
            sunrise_gameplay::game::native_entity_refs::tests::fixture_resident_column(
                &mut game.world,
                (0, 0),
            );
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(1.5, 64.0, 1.5)).unwrap());
            game.level.player_uuid = Some(sunrise_sim::lan::playerdata::uuid_ints(7));
            game.server.lan_acting_uuid = Some(7);
            game.server.lan_acting_name = Some("pickup_post".into());

            // The whole stack goes in: one post notice, original 8, remainder empty.
            let stat = host_stat(game);
            assert_eq!(
                host_case(game, &probe, &CASE),
                (vec![(7, 8, None)], None, 64)
            );
            assert_eq!(host_stat(game), stat + 8);

            // 8 offered, 5 fit: one post notice, original 8 (untouched by the insertion),
            // remainder 3.
            let stat = host_stat(game);
            let partial = Case { room: 5, ..CASE };
            assert_eq!(
                host_case(game, &probe, &partial),
                (vec![(7, 8, Some(3))], Some(3), 64)
            );
            assert_eq!(host_stat(game), stat + 5);

            // The listener on a partial pickup raises the remainder to 6: the entity keeps 6,
            // the pickup counts 2.
            let stat = host_stat(game);
            let raised = Case {
                raise: Some(6),
                ..partial
            };
            assert_eq!(
                host_case(game, &probe, &raised),
                (vec![(7, 8, Some(3))], Some(6), 64)
            );
            assert_eq!(host_stat(game), stat + 2);

            // The listener on a partial pickup empties the remainder: the entity goes, the
            // pickup counts 8.
            let stat = host_stat(game);
            let emptied = Case {
                empty: true,
                ..partial
            };
            assert_eq!(
                host_case(game, &probe, &emptied),
                (vec![(7, 8, Some(3))], None, 64)
            );
            assert_eq!(host_stat(game), stat + 8);

            // The listener on a partial pickup discards the item after five were inserted.
            let stat = host_stat(game);
            let killed = Case {
                kill: true,
                ..partial
            };
            assert_eq!(
                host_case(game, &probe, &killed),
                (vec![(7, 8, Some(3))], None, 64)
            );
            assert_eq!(host_stat(game), stat + 5);

            // Refused by `item_picked_up`: nothing is told.
            let refused = Case {
                answer: TriState::False,
                ..CASE
            };
            assert_eq!(host_case(game, &probe, &refused), (vec![], Some(8), 56));

            // A full inventory takes nothing, even when `True` forces the pickup.
            let full = Case { room: 0, ..CASE };
            assert_eq!(host_case(game, &probe, &full), (vec![], Some(8), 0));
            let forced = Case {
                answer: TriState::True,
                room: 0,
                delay: 40,
                ..CASE
            };
            assert_eq!(host_case(game, &probe, &forced), (vec![], Some(8), 0));

            // A forced pickup that fits is told as the default one is.
            let forced = Case {
                answer: TriState::True,
                delay: 40,
                ..CASE
            };
            assert_eq!(
                host_case(game, &probe, &forced),
                (vec![(7, 8, None)], None, 64)
            );

            // The listener puts 3 back: the entity stays with 3, the pickup counts 5.
            let stat = host_stat(game);
            let raised = Case {
                raise: Some(3),
                ..CASE
            };
            assert_eq!(
                host_case(game, &probe, &raised),
                (vec![(7, 8, None)], Some(3), 64)
            );
            assert_eq!(host_stat(game), stat + 5);

            // Creative takes the whole ground stack even with no inventory room.
            game.creative = true;
            let full = Case { room: 0, ..CASE };
            let stat = host_stat(game);
            assert_eq!(
                host_case(game, &probe, &full),
                (vec![(7, 8, None)], None, 0)
            );
            assert_eq!(host_stat(game), stat + 8);
            game.creative = false;

            // The listener puts 3 back, then kills the item: it stays gone, with no panic.
            let killed = Case {
                raise: Some(3),
                kill: true,
                ..CASE
            };
            assert_eq!(
                host_case(game, &probe, &killed),
                (vec![(7, 8, None)], None, 64)
            );
            let _ = std::fs::remove_dir_all(scratch);
        });
    });
}
