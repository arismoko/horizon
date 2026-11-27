use super::*;
use sunrise_gameplay_common::game::remote::remote_use_swings;

/// `MultiPlayerGameMode.useItem`'s `shouldSwing`: the SUCCESS answers
/// swing, CONSUME's do not (#6425).
#[test]
fn a_thrown_item_swinging_on_a_server_and_a_charge_not() {
    let stack = |id: &str| ItemStack::new(id, 1);
    // SUCCESS: the throwables, a bucket, a boat, a bundle, a book, a map.
    for id in [
        "minecraft:ender_pearl",
        "minecraft:splash_potion",
        "minecraft:wind_charge",
        "minecraft:snowball",
        "minecraft:experience_bottle",
        "minecraft:bucket",
        "minecraft:oak_boat",
        "minecraft:bundle",
        "minecraft:writable_book",
        "minecraft:map",
    ] {
        assert!(remote_use_swings(&stack(id), false), "{id} swings");
    }
    // CONSUME: the charges and startUsingInstantly's uses, a drinkable
    // potion among them (`consumable` set).
    for id in [
        "minecraft:bow",
        "minecraft:crossbow",
        "minecraft:trident",
        "minecraft:spyglass",
        "minecraft:goat_horn",
        "minecraft:ender_eye",
    ] {
        assert!(!remote_use_swings(&stack(id), false), "{id} does not swing");
    }
    // The kinetic weapons charge like a trident.
    assert!(!remote_use_swings(&stack("minecraft:iron_spear"), false));
    // A consumable (food, a drink) starts its use without a swing.
    assert!(!remote_use_swings(&stack("minecraft:apple"), true));
}
