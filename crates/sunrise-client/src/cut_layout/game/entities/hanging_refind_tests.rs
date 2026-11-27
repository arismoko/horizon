//! After `hanging_break` returns without cancelling, the game finds the item frame or painting
//! again by id: one a listener removed neither breaks nor is removed again, and an empty frame it
//! filled stays.
use crate::game::server_side::hook_meddler_tests::with_meddler;

#[test]
#[ignore = "daily"]
fn a_frame_the_listener_removed_does_not_break() {
    with_meddler(
        |host| {
            assert!(
                host.commands()
                    .run_command("kill @e[type=minecraft:item_frame]")
            );
        },
        |game, gate| {
            assert!(game.run_command("summon minecraft:item_frame 8 100 8 {Facing:3b}"));
            assert!(game.run_command("summon minecraft:item_frame 12 100 8 {Facing:3b}"));
            let dropped = game.dropped.len();
            gate.arm();
            assert!(!game.break_frame_by(1, None), "the frame was gone");
            assert!(gate.fired());
            assert!(game.item_frames.is_empty());
            assert_eq!(game.dropped.len(), dropped, "nothing dropped");
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_painting_the_listener_removed_is_not_removed_again() {
    with_meddler(
        |host| {
            assert!(
                host.commands()
                    .run_command("kill @e[type=minecraft:painting]")
            );
        },
        |game, gate| {
            assert!(game.run_command("summon minecraft:painting 8 100 8 {facing:0b}"));
            assert!(game.run_command("summon minecraft:painting 20 100 8 {facing:0b}"));
            gate.arm();
            assert!(
                !game.break_painting_by(1, false, None),
                "the painting was gone"
            );
            assert!(gate.fired());
            assert!(game.paintings.is_empty());
        },
    );
}

const FILL: &str =
    "item replace entity @e[type=minecraft:item_frame,limit=1] container.0 with minecraft:stick";

#[test]
#[ignore = "daily"]
fn a_frame_the_listener_filled_does_not_break_by_a_kill() {
    with_meddler(
        |host| {
            assert!(host.commands().run_command(FILL));
        },
        |game, gate| {
            assert!(game.run_command("summon minecraft:item_frame 8 100 8 {Facing:3b}"));
            let dropped = game.dropped.len();
            gate.arm();
            assert!(!game.break_frame_by(0, None), "the frame was filled");
            assert!(gate.fired());
            assert_eq!(game.item_frames.len(), 1, "the frame stays");
            assert!(game.item_frames[0].item.is_some());
            assert_eq!(game.dropped.len(), dropped, "nothing dropped");
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_frame_the_listener_filled_survives_the_loss_of_its_support() {
    with_meddler(
        |host| {
            assert!(host.commands().run_command(FILL));
        },
        |game, gate| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            // No wall behind it: the survival check fails when its interval is up.
            assert!(game.run_command("summon minecraft:item_frame 8 100 8 {Facing:3b}"));
            game.item_frames[0].check_interval = 99;
            let dropped = game.dropped.len();
            gate.arm();
            game.tick_item_frames();
            assert!(gate.fired());
            assert_eq!(game.item_frames.len(), 1, "the frame stays");
            assert!(game.item_frames[0].item.is_some());
            assert_eq!(game.dropped.len(), dropped, "nothing dropped");
        },
    );
}
