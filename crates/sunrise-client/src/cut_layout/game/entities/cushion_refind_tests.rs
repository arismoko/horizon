//! After `entity_mount` lets a player board a cushion, the game finds the cushion again by id: one
//! a listener removed does not seat the player, and the click is spent.
use crate::game::server_side::hook_meddler_tests::with_meddler;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::InteractTarget;
use sunrise_sim::entities::Cushion;

#[test]
#[ignore = "daily"]
fn a_cushion_the_listener_removed_does_not_seat_the_player() {
    with_meddler(
        |host| {
            assert!(
                host.commands()
                    .run_command("kill @e[type=minecraft:cushion]")
            );
        },
        |game, gate| {
            game.cushions.push(Cushion {
                attachment_lifetime: Default::default(),
                join: Default::default(),
                id: 953,
                position: Vec3::new(2.5, 100.0, 2.5),
                pos: [2, 100, 2],
                yaw: 0.0,
                color: 0,
                check_interval: 0,
                custom_name: None,
            });
            gate.arm();
            let click = InteractTarget {
                id: 953,
                location: [0.0; 3],
            };
            assert!(game.interact_cushion(Some(click)), "the click is spent");
            assert!(gate.fired());
            assert!(game.cushions.is_empty());
            assert_eq!(game.riding, None);
        },
    );
}
