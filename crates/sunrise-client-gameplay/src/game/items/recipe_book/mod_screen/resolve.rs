//! Resource-backed resolution of mod recipe displays.
use mod_api::{RecipeBookDisplay as Display, RecipeSlotDisplay as Slot};
use sunrise_items::items::ItemStack;
use sunrise_sim::{crafting::Ingredient, menu::place::Stacked};

pub(super) struct Resolver<'a, 'w> {
    game: &'a crate::game::ClientGame<'w>,
    stocks: &'a Stacked,
    placement_stocks: &'a [(ItemStack, i32)],
}

impl<'a, 'w> Resolver<'a, 'w> {
    #[cfg(feature = "client")]
    pub(super) fn new(
        game: &'a crate::game::ClientGame<'w>,
        stocks: &'a Stacked,
        placement_stocks: &'a [(ItemStack, i32)],
    ) -> Self {
        Self {
            game,
            stocks,
            placement_stocks,
        }
    }

    pub(super) fn resolve(&self, slot: &Slot) -> Vec<ItemStack> {
        self.game.resolve_mod_book_declaration_slot(slot)
    }

    pub(super) fn craftable(&self, ingredients: &[Ingredient]) -> bool {
        self.stocks
            .can_craft(&ingredients.iter().collect::<Vec<_>>())
    }

    pub(super) fn search_text(&self, stacks: &[ItemStack]) -> String {
        stacks
            .iter()
            .map(|stack| self.game.stack_hover_name_text(stack).plain())
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    }
}

#[cfg(test)]
fn dye_stack(target: &ItemStack, dye: usize) -> ItemStack {
    let mut stack = target.clone();
    stack.count = 1;
    let current = stack.data().and_then(|data| data.dyed_color);
    stack.data_mut().dyed_color = Some(sunrise_sim::crafting::apply_dyes(current, &[dye]));
    stack
}

impl sunrise_sim::recipe_book::mod_presentation::DisplayResolver for Resolver<'_, '_> {
    fn resolve(&self, slot: &Slot) -> Vec<ItemStack> {
        Resolver::resolve(self, slot)
    }
    fn craftable(&self, recipe: &str, _: &Display, ingredients: &[Ingredient]) -> bool {
        if self
            .game
            .native_screen_menu()
            .is_some_and(|menu| mod_api::RecipeBookLayout::declaring_mod(menu).is_some())
        {
            let Some(plan) = self
                .game
                .native_net
                .placement
                .view
                .as_ref()
                .and_then(|view| view.index.placement(recipe).ok())
            else {
                return false;
            };
            let ingredients = &plan.requirements().ingredients;
            let mut counts: Vec<_> = self
                .placement_stocks
                .iter()
                .map(|(_, amount)| *amount)
                .collect();
            return sunrise_sim::menu::place::can_craft_matching(
                &mut counts,
                ingredients.len(),
                |candidate, ingredient| {
                    if ingredients[ingredient]
                        .test_in(&self.game.loop_context, &self.placement_stocks[candidate].0)
                    {
                        i32::MAX
                    } else {
                        0
                    }
                },
            );
        }
        Resolver::craftable(self, ingredients)
    }
    fn search_text(&self, results: &[ItemStack]) -> String {
        Resolver::search_text(self, results)
    }
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

#[cfg(feature = "client")]
#[cfg(test)]
mod tests {
    use super::*;
    fn can_craft(stocks: &[ItemStack], ingredients: &[Ingredient]) -> bool {
        let mut stacked = Stacked::default();
        for stack in stocks {
            stacked.account_stack(stack, stack.count);
        }
        stacked.can_craft(&ingredients.iter().collect::<Vec<_>>())
    }

    #[test]
    fn dye_blends_existing_color_and_preserves_other_components() {
        let mut target = ItemStack::new("test:target", 7).with_potion("minecraft:water");
        target.data_mut().dyed_color = Some(0xB02E26);
        let dyed = dye_stack(&target, 11);
        assert_eq!(dyed.count, 1);
        assert_eq!(dyed.data().unwrap().dyed_color, Some(0xAD5398));
        let mut expected = target.clone();
        expected.count = 1;
        expected.data_mut().dyed_color = dyed.data().unwrap().dyed_color;
        assert_eq!(dyed, expected);
        assert_eq!(target.count, 7);
    }
    #[test]
    fn duplicate_inputs_consume_capacity_and_reassign_alternatives() {
        let ingredients = [
            Ingredient::of(vec!["test:a".into(), "test:b".into()]),
            Ingredient::of(vec!["test:a".into()]),
        ];
        assert!(!can_craft(&[ItemStack::new("test:a", 1)], &ingredients));
        assert!(can_craft(&[ItemStack::new("test:a", 2)], &ingredients));
        assert!(can_craft(
            &[ItemStack::new("test:a", 1), ItemStack::new("test:b", 1)],
            &ingredients
        ));
    }
}
