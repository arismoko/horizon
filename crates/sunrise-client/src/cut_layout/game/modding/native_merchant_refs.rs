#![cfg(test)]
use mod_api::EntityRef;
use mod_api::MerchantRef;
use sunrise_sim::merchant::Trader;
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::server_side::native_merchant_refs::tests::offer;
    use sunrise_gameplay_common::game::tables::GameItems;

    use sunrise_gameplay::game::server_side::native_merchant_refs::tests::spawn;

    use sunrise_gameplay_common::game::api_values::{StackReadError, TryIntoApi, TryIntoEngine};

    use super::super::native_entity_refs::tests::with_game;

    use super::*;

    use mod_api::WorldEpoch;

    use sunrise_core::geometry::Vec3;

    use sunrise_items::items::ItemStack;

    use sunrise_sim::{merchant::ItemCost, mobs::Mob};

    #[test]
    #[ignore = "daily"]
    fn unresolved_profession_preserves_offers_but_cannot_trade_them() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let villager = spawn(game, 40, "minecraft:villager");
            game.mobs[0].extra.profession =
                sunrise_sim::mobs::villager::ProfessionState::from_id("example:engineer");
            game.mobs[0].extra.trade = Some(Box::new(Trader {
                offers: vec![offer("minecraft:wheat", "minecraft:emerald")],
                ..Trader::default()
            }));
            assert_eq!(villager.offers(game), Some(Vec::new()));
            assert!(!villager.set_profession(game, "minecraft:farmer"));
            assert!(game.interact_trader(0, "minecraft:air").unwrap());
            assert!(game.station.is_none());
            assert_eq!(game.mobs[0].extra.trade.as_ref().unwrap().offers.len(), 1);
            assert_eq!(
                villager.profession(game).as_deref(),
                Some("example:engineer")
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn merchant_handles_edit_offers_and_villager_data() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let villager = spawn(game, 40, "minecraft:villager");
            let trader = spawn(game, 41, "minecraft:wandering_trader");
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(42, zombie, Vec3::ZERO, 0.0));
            assert!(game.merchant(game.entity(42).unwrap()).is_none());

            // Nothing is rolled yet; reading and a refused write draw nothing.
            assert_eq!(villager.offers(game), Some(Vec::new()));
            assert!(!villager.remove_offer(game, 0));
            assert_eq!(
                villager.set_offer(
                    game,
                    0,
                    offer("minecraft:wheat", "minecraft:emerald")
                        .try_into_api()
                        .unwrap()
                ),
                Ok(false)
            );
            assert!(game.mobs[0].extra.trade.as_ref().is_none_or(|t| t.unset));

            // A wandering trader has offers but no villager data.
            assert_eq!(
                trader.add_offer(
                    game,
                    offer("minecraft:emerald", "minecraft:diamond")
                        .try_into_api()
                        .unwrap()
                ),
                Ok(true)
            );
            assert!(trader.offers(game).unwrap().len() > 1);
            assert_eq!(trader.profession(game), None);
            assert!(!trader.set_level(game, 2));
            assert!(!trader.set_xp(game, 5));
            assert_eq!(trader.villager_type(game), None);

            // Put in known offers, as a rolled villager has them.
            game.mobs[0].extra.trade = Some(Box::new(Trader {
                offers: vec![
                    offer("minecraft:wheat", "minecraft:emerald"),
                    offer("minecraft:coal", "minecraft:emerald"),
                ],
                ..Trader::default()
            }));
            let mut changed = offer("minecraft:carrot", "minecraft:diamond");
            changed.uses = 5;
            assert_eq!(
                villager.set_offer(game, 1, changed.try_into_api().unwrap()),
                Ok(true)
            );
            assert_eq!(
                villager.offers(game).unwrap()[1],
                changed.try_into_api().unwrap()
            );
            assert_eq!(
                villager.set_offer(game, 2, changed.try_into_api().unwrap()),
                Ok(false)
            );
            // Empty and unknown engine stacks are refused before an API offer exists.
            let mut bad = changed.clone();
            bad.result.count = 0;
            assert_eq!(bad.try_into_api(), Err(StackReadError::Empty));
            for (a, b, r) in [
                ("minecraft:not_an_item", None, "minecraft:emerald"),
                (
                    "minecraft:wheat",
                    Some("minecraft:not_an_item"),
                    "minecraft:emerald",
                ),
                ("minecraft:wheat", None, "minecraft:not_an_item"),
            ] {
                let mut bad = changed.clone();
                bad.base_cost_a.stack = ItemStack::new(a, 1);
                bad.cost_b = b.map(|b| ItemCost {
                    stack: ItemStack::new(b, 1),
                });
                bad.result = ItemStack::new(r, 1);
                assert_eq!(bad.try_into_api(), Err(StackReadError::UnknownItem));
            }
            assert_eq!(
                villager.add_offer(game, changed.try_into_api().unwrap()),
                Ok(true)
            );
            assert_eq!(villager.offers(game).unwrap().len(), 3);
            assert!(villager.remove_offer(game, 0));
            assert!(!villager.remove_offer(game, 5));
            assert_eq!(
                villager.offers(game).unwrap(),
                vec![changed.clone(), changed].try_into_api().unwrap()
            );
            assert_eq!(villager.trading_player(game), None);

            assert_eq!(villager.profession(game).as_deref(), Some("minecraft:none"));
            assert!(!villager.set_profession(game, "minecraft:wizard"));
            assert!(!villager.set_profession(game, "farmer_x"));
            assert!(villager.set_profession(game, "minecraft:farmer"));
            assert_eq!(
                villager.profession(game).as_deref(),
                Some("minecraft:farmer")
            );
            // A new profession forgets the offers; the same one keeps them.
            assert_eq!(villager.offers(game), Some(Vec::new()));
            game.mobs[0].extra.trade.as_mut().unwrap().unset = false;
            game.mobs[0].extra.trade.as_mut().unwrap().offers =
                vec![offer("minecraft:wheat", "minecraft:emerald")];
            assert!(villager.set_profession(game, "minecraft:farmer"));
            assert_eq!(villager.offers(game).unwrap().len(), 1);

            assert_eq!(villager.level(game), Some(1));
            assert!(villager.set_level(game, 4));
            assert_eq!(villager.level(game), Some(4));
            assert_eq!(game.mobs[0].extra.villager_level, 4);
            assert!(villager.set_level(game, 123456));
            assert_eq!(villager.level(game), Some(123456));
            assert_eq!(game.mobs[0].extra.villager_level, 123456);
            // Vanilla: `Math.max(1, level)`, no upper limit.
            assert!(villager.set_level(game, 0));
            assert_eq!(villager.level(game), Some(1));
            assert!(villager.set_level(game, 6));
            assert_eq!(villager.level(game), Some(6));
            assert!(villager.set_level(game, 1000));
            assert_eq!(villager.level(game), Some(1000));
            assert!(villager.set_level(game, 4));
            assert_eq!(
                villager.villager_type(game).as_deref(),
                Some("minecraft:plains")
            );
            assert!(villager.set_villager_type(game, "minecraft:desert"));
            assert_eq!(
                villager.villager_type(game).as_deref(),
                Some("minecraft:desert")
            );
            assert!(!villager.set_villager_type(game, "minecraft:mars"));
            assert_eq!(villager.xp(game), Some(0));
            assert!(villager.set_xp(game, 80));
            assert_eq!(villager.xp(game), Some(80));
            assert!(villager.set_xp(game, -1));
            assert_eq!(villager.xp(game), Some(-1));
            assert_eq!(villager.offers(game).unwrap().len(), 1);

            // Saved with the entity (`Offers`) and read back unchanged.
            let saved = villager
                .offers(game)
                .unwrap()
                .try_into_engine(game.items())
                .unwrap();
            let tag = sunrise_sim::entity_nbt::offers_to_nbt(&saved, &Default::default());
            assert_eq!(sunrise_sim::entity_nbt::offers_from_nbt(&tag), saved);

            let stale = MerchantRef::new(EntityRef::new(
                WorldEpoch::new(villager.entity().epoch().get() + 1),
                villager.entity().id(),
            ));
            assert_eq!(stale.offers(game), None);
            assert!(!stale.set_xp(game, 1));
            assert_eq!(
                stale.add_offer(
                    game,
                    offer("minecraft:wheat", "minecraft:emerald")
                        .try_into_api()
                        .unwrap()
                ),
                Ok(false)
            );
            let taken = game.mobs.lift();
            assert_eq!(villager.level(game), None);
            game.mobs = taken;
        });
    }
}
