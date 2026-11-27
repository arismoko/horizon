#![cfg(test)]
use sunrise_gameplay_common::game::held_items::OFFHAND_SLOT;
use sunrise_items::items::ItemStack;
use sunrise_sim::health::{BlockOutcome, HurtSource};
#[cfg(test)]
mod durability_tests {
    use super::*;
    use sunrise_core::geometry::Vec3;
    use sunrise_sim::motion::PlayerMotion;

    #[test]
    #[ignore = "daily"]
    fn shield_break_clears_the_active_use() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            let mut shield = ItemStack::new("minecraft:shield", 1);
            shield.damage = game.items().get(&shield.item).unwrap().max_damage - 1;
            game.offhand = Some(shield);
            game.using = Some((OFFHAND_SLOT, 20));
            game.after_block(
                BlockOutcome {
                    blocking_item_used: true,
                    hurt: false,
                    blocked: 4.0,
                    shield_damage: 1,
                    fully_blocked: true,
                    sound: false,
                    knockback: false,
                    canceled: false,
                    dealt: 0.0,
                },
                (None, false),
                HurtSource {
                    damage_type: "minecraft:generic",
                    causing: 0,
                    direct: 0,
                    position: None,
                    direct_type: None,
                },
                &sunrise_sim::health::NO_ATTACK,
            );
            assert!(game.offhand.is_none());
            assert!(game.using.is_none());
        });
    }

    /// The wear a raised shield takes from a blocked hit (8 points here), through the part of the
    /// shield: halved to 4, canceled to none, and as vanilla (8) for the part-less item next to it.
    #[test]
    #[ignore = "daily"]
    fn a_damage_item_part_changes_the_wear_of_a_blocking_shield() {
        use {
            crate::game::server_side::damage_item_tests::Rule,
            crate::game::server_side::damage_item_tests::with_wear_part,
        };
        with_wear_part("minecraft:shield", Rule::Halve, |game, wear| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            let block = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                game.offhand = Some(ItemStack::new("minecraft:shield", 1));
                game.using = Some((OFFHAND_SLOT, 20));
                game.after_block(
                    BlockOutcome {
                        blocking_item_used: true,
                        hurt: false,
                        blocked: 0.0,
                        shield_damage: 8,
                        fully_blocked: true,
                        sound: false,
                        knockback: false,
                        canceled: false,
                        dealt: 0.0,
                    },
                    (None, false),
                    HurtSource {
                        damage_type: "minecraft:generic",
                        causing: 0,
                        direct: 0,
                        position: None,
                        direct_type: None,
                    },
                    &sunrise_sim::health::NO_ATTACK,
                );
                game.offhand.as_ref().unwrap().damage
            };
            assert_eq!(block(game), 4);
            assert_eq!(wear.take_asked(), [8]);
            wear.set(Rule::Cancel);
            assert_eq!(block(game), 0);
            assert_eq!(wear.take_asked(), [8]);
        });
    }
}

#[cfg(test)]
mod blocked_hit_use_tests {
    use super::*;
    use sunrise_gameplay::game::game_tests::tests::server_game;
    use sunrise_items::items::ItemStack;

    /// `BlocksAttacks.hurtBlockingItem`: a hit taken while using a shield
    /// counts it as used, even when the shield blocks none of the damage.
    #[test]
    #[ignore = "daily"]
    fn a_blocked_hit_counts_the_shield_as_used() {
        let mut game_resources = None;
        let mut game = server_game(&mut game_resources);
        game.build_registries();
        game.offhand = Some(ItemStack::new("minecraft:shield", 1));
        game.using = Some((OFFHAND_SLOT, 0));
        let source = HurtSource {
            damage_type: "minecraft:mob_attack",
            causing: 1,
            direct: 1,
            position: None,
            direct_type: None,
        };
        let outcome = BlockOutcome {
            blocking_item_used: true,
            hurt: true,
            blocked: 5.0,
            shield_damage: 0,
            fully_blocked: true,
            sound: false,
            knockback: false,
            canceled: false,
            dealt: 0.0,
        };
        assert_eq!(game.stats.get("used", "minecraft:shield"), 0);
        game.after_block(
            outcome,
            (None, false),
            source,
            &sunrise_sim::health::NO_ATTACK,
        );
        assert_eq!(
            game.stats.get("used", "minecraft:shield"),
            1,
            "a hit the shield takes whole counts it"
        );
        // The gate below the award: a hit the shield blocks nothing of still
        // counts it (hurtBlockingItem runs before the `blocked > 0` branch).
        let outcome = BlockOutcome {
            blocked: 0.0,
            ..outcome
        };
        game.after_block(
            outcome,
            (None, false),
            source,
            &sunrise_sim::health::NO_ATTACK,
        );
        assert_eq!(game.stats.get("used", "minecraft:shield"), 2);
    }
}

#[cfg(test)]
#[path = "tests/shield_ram_tests.rs"]
mod ram_tests;
use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
