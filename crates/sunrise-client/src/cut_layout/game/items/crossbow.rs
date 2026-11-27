#![cfg(test)]
#[cfg(feature = "client")]
use sunrise_items::items::ItemStack;
#[cfg(test)]
pub(crate) mod tests {

    #[cfg(feature = "client")]
    use super::*;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::crossbow_common::{ARROW, CROSSBOW};

    #[cfg(feature = "client")]
    pub(crate) use crate::game::client_side::crossbow_client::tests::game_with_player;

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_refused_crossbow_shot_keeps_its_loaded_projectiles() {
        struct Refuse;

        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Refuse {
            fn arrow_loose(
                &self,
                _: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::ArrowLooseEvent<'_>,
            ) -> mod_api::Flow {
                assert_eq!(event.charge, 1);
                assert!(event.has_ammo);
                event.charge = -1;
                mod_api::Flow::Continue
            }
        }

        let mods = vec![mod_api::ModEntry::new("refuse", Box::new(Refuse))].into();
        mod_platform::testing::with_mods(mods, || {
            let root =
                std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
            let mut resources = None;
            let (mut game, _) = game_with_player(&mut resources, &root);
            let mut crossbow = ItemStack::new(CROSSBOW, 1);
            crossbow.data_mut().charged = vec![ItemStack::new(ARROW, 1)];
            *game.held_mut() = Some(crossbow.clone());
            game.shoot_crossbow();
            assert_eq!(game.held(), Some(&crossbow));
            assert!(game.creatures.projectiles.is_empty());
        });
    }

    /// `Projectile.applyOnProjectileSpawned` runs `EnchantmentHelper.onProjectileSpawned`
    /// for the weapon an arrow is fired from, bow or crossbow, so a Flame
    /// crossbow shoots burning arrows: Flame's `Ignite(constant(100))` is 100
    /// seconds, 2000 ticks, on every arrow of a Multishot volley (#10482).
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_flame_crossbow_shoots_burning_arrows() {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let mut resources = None;
        let (mut game, _) = game_with_player(&mut resources, &root);
        // Shoots a crossbow with `enchantments` and one arrow loaded per shot,
        // and returns how long each new arrow burns for.
        let mut shoot = |enchantments: &[&str], shots: usize| -> Vec<i32> {
            let mut crossbow = ItemStack::new(CROSSBOW, 1);
            for id in enchantments {
                crossbow.enchant(id, 1);
            }
            crossbow.data_mut().charged = vec![ItemStack::new(ARROW, 1); shots];
            *game.held_mut() = Some(crossbow);
            let before = game.creatures.projectiles.len();
            game.shoot_crossbow();
            game.creatures.projectiles[before..]
                .iter()
                .map(|p| p.arrow.as_ref().expect("an arrow").fire_ticks)
                .collect()
        };
        assert_eq!(shoot(&[], 1), vec![0]);
        assert_eq!(shoot(&["minecraft:flame"], 1), vec![2000]);
        assert_eq!(
            shoot(&["minecraft:flame", "minecraft:multishot"], 3),
            vec![2000; 3]
        );
    }
}
