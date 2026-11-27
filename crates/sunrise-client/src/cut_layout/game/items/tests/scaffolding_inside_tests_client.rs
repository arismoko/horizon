use sunrise_command::args::GameMode;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay::game::server_side::use_hand::scaffolding_inside_tests::name;
use sunrise_gameplay::game::server_side::use_hand::scaffolding_inside_tests::set_sneak;
use sunrise_items::items::ItemStack;
use sunrise_sim::motion::PlayerMotion;
use {sunrise_gameplay::game::game_tests::tests::server_game, sunrise_gameplay_common::game::Hit};

/// A player standing inside a scaffolding block, looking straight up: the eye
/// is inside the block's outline (a full block while scaffolding is in hand),
/// so the click is an inside one, its face the side the look comes from.
/// Sneaking then sends the placement up the column; a click that lost the flag
/// goes to the clicked side instead, and one that lost the face beside it.
#[test]
#[ignore = "daily"]
fn a_click_from_inside_a_scaffolding_block_places_on_the_opposite_face() {
    const COLUMN: [i32; 3] = [2, 5, 0];
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.options.dedicated = None;
    game.set_mode(GameMode::Creative);
    let scaffolding = game
        .pack
        .assets
        .registry
        .default_state("minecraft:scaffolding")
        .unwrap();
    // A supported block (stability 0), so a block may go on top of it.
    let supported = game
        .with_props(scaffolding, &[("distance", "0")])
        .expect("scaffolding has a distance property");
    game.world
        .set(COLUMN[0], COLUMN[1], COLUMN[2], supported)
        .unwrap();
    game.player = Some(
        PlayerMotion::new(
            game.motion,
            Vec3::new(
                f64::from(COLUMN[0]) + 0.5,
                f64::from(COLUMN[1]) - 1.0,
                f64::from(COLUMN[2]) + 0.5,
            ),
        )
        .unwrap(),
    );
    *game.held_mut() = Some(ItemStack::new("minecraft:scaffolding", 1));
    game.yaw = 0.0;
    game.pitch = -90.0;
    set_sneak(&mut game, true);
    let hit: Hit = game.pick().expect("the eye is inside the column");
    assert_eq!(hit.pos, COLUMN, "the click is on the block the eye is in");
    assert!(game.use_hand(Some(&hit)).unwrap());
    assert_eq!(
        name(&game, [COLUMN[0], COLUMN[1] + 1, COLUMN[2]]),
        "minecraft:scaffolding",
        "inside + sneak places on the opposite of the clicked (down) face: upwards"
    );
    assert_eq!(
        name(&game, [COLUMN[0], COLUMN[1] - 1, COLUMN[2]]),
        "minecraft:air",
        "a click without the inside flag places on the clicked face: below"
    );
    assert_eq!(
        name(&game, [COLUMN[0] + 1, COLUMN[1], COLUMN[2]]),
        "minecraft:air",
        "a ray that lost the face places beside the column"
    );
}

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
