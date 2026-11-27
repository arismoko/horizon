//! `Wolf.mobInteract` for the owner of a tame wolf (#12939): an item of
//! `#wolf_collar_dyes` only ever dyes the collar, so one of the colour the collar
//! already has does nothing, while another item still makes it sit or stand.
#![cfg(feature = "client")]
use super::*;

pub(crate) const WOLF: u32 = 50;

#[allow(unused_imports)]
pub(crate) use crate::game::client_side::core_tests_client::pet_order_tests::game;

pub(crate) fn use_on_wolf(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    item: &str,
) -> (bool, u8, u32) {
    game.inventory[0] = Some(ItemStack::new(item, 1));
    let target = Some(InteractTarget {
        id: WOLF as i32,
        location: [0.0, 0.5, 0.0],
    });
    game.interact_mob(target).unwrap();
    let wolf = game.mobs.iter().find(|m| m.id == WOLF).unwrap();
    let left = game.inventory[0].as_ref().map_or(0, |s| s.count);
    (wolf.extra.sitting, wolf.extra.color, left)
}

#[test]
#[ignore = "daily"]
fn the_collars_own_dye_leaves_a_wolf_as_it_was() {
    let mut resources = None;
    let mut game = game(&mut resources);
    let red = game.mobs[0].extra.color;
    assert_eq!(
        sunrise_level::dyes::DYE_NAMES[usize::from(red)],
        "red",
        "a tamed wolf's collar starts red"
    );
    // The colour it already has: nothing happens, and the dye stays.
    assert_eq!(use_on_wolf(&mut game, "minecraft:red_dye"), (false, red, 1));
    // Another colour dyes the collar and is used up; the wolf keeps standing.
    let (sitting, color, left) = use_on_wolf(&mut game, "minecraft:blue_dye");
    assert!(!sitting);
    assert_eq!(sunrise_level::dyes::DYE_NAMES[usize::from(color)], "blue");
    assert_eq!(left, 0);
    // Any other item still makes the owner's wolf sit.
    assert!(use_on_wolf(&mut game, "minecraft:stick").0);
    // And the collar's own dye leaves a sitting wolf sitting.
    assert!(use_on_wolf(&mut game, "minecraft:blue_dye").0);
}
