#![cfg(test)]
use sunrise_items::items::ItemStack;
#[cfg(test)]
#[path = "tests/player_armor_weapon_tests.rs"]
pub(crate) mod player_armor_weapon_tests;

#[cfg(test)]
pub(crate) mod tests {

    use super::*;

    /// A hit takes the mace one point (`Weapon.itemDamagePerAttack`): a part of the mace that
    /// multiplies the wear by 5 makes it 5, one that halves it leaves 0 (the item stays whole), one
    /// that cancels it none; an iron sword without a part wears as vanilla.
    #[test]
    #[ignore = "daily"]
    fn a_damage_item_part_changes_the_wear_of_a_weapon_hit() {
        use crate::game::server_side::damage_item_tests::{Rule, with_wear_part};
        with_wear_part("minecraft:mace", Rule::Times(5), |game, wear| {
            let hit = |game: &mut sunrise_gameplay::game::ServerGame<'_>, item: &str| {
                game.creative = false;
                game.selected = 0;
                let weapon = ItemStack::new(item, 1);
                game.inventory[0] = Some(weapon.clone());
                game.weapon_wear(&weapon);
                game.inventory[0].as_ref().unwrap().damage
            };
            assert_eq!(hit(game, "minecraft:mace"), 5);
            assert_eq!(wear.take_asked(), [1]);
            wear.set(Rule::Halve);
            assert_eq!(hit(game, "minecraft:mace"), 0);
            assert_eq!(wear.take_asked(), [1]);
            wear.set(Rule::Cancel);
            assert_eq!(hit(game, "minecraft:mace"), 0);
            assert_eq!(wear.take_asked(), [1]);
            assert_eq!(hit(game, "minecraft:iron_sword"), 1);
            assert!(wear.take_asked().is_empty());
        });
    }
}
