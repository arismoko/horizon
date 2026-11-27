#![cfg(test)]
use sunrise_core::position::BlockPos;
use sunrise_items::items::ItemStack;
use sunrise_level::containers::BlockEntity;

#[cfg(test)]
pub(crate) mod part_remainder_tests {
    use super::*;
    use sunrise_level::containers::Crafter;
    struct Milk;
    #[mod_api::sunrise_part]
    impl mod_api::ItemPart for Milk {
        fn crafting_remainder(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::CraftingRemainderEvent<'_>,
        ) {
            *event.remainder = None;
        }
    }
    struct Remainders;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Remainders {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let milk = r.item_key("minecraft:milk_bucket");
            r.item_part(milk, Milk);
        }
    }
    #[test]
    #[ignore = "daily"]
    fn crafter_dispense_observes_part_replaced_remainders() {
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("milk", Box::new(Remainders))].into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                sunrise_gameplay::game::native_entity_refs::tests::resolve_parts(game).unwrap();
                let state = game
                    .block_state_id("minecraft:crafter[orientation=north_up]")
                    .unwrap();
                let pos = BlockPos::new(0, 100, 0);
                let grid = [
                    "milk_bucket",
                    "milk_bucket",
                    "milk_bucket",
                    "sugar",
                    "egg",
                    "sugar",
                    "wheat",
                    "wheat",
                    "wheat",
                ];
                let mut crafter = Crafter::default();
                for (slot, item) in crafter.items.iter_mut().zip(grid) {
                    *slot = Some(ItemStack::new(&format!("minecraft:{item}"), 1));
                }
                game.world
                    .block_entities
                    .insert((0, 100, 0), BlockEntity::Crafter(crafter));
                game.dropped.clear();
                game.crafter_dispense(pos, state).unwrap();
                let output: Vec<_> = game
                    .dropped
                    .iter()
                    .map(|e| (e.stack.item.to_string(), e.stack.count))
                    .collect();
                assert_eq!(
                    output,
                    [("minecraft:cake".to_owned(), 1)],
                    "the Part suppresses all three data-only buckets"
                );
            });
        });
    }
}
