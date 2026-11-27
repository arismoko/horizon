//! Host lookup keeps the same rectangular input for matching and remainders.
use super::native_entity_refs::tests::with_game;
use mod_api::{CraftingGrid, CraftingRecipeKind, ItemKey, ItemStack};

struct Vertical {
    milk: ItemKey,
    stick: ItemKey,
    diamond: ItemKey,
    bottle: ItemKey,
    paper: ItemKey,
}
impl CraftingRecipeKind for Vertical {
    type Value = ();
    fn decode(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn write(&self, _: &()) -> Result<String, String> {
        Ok("{}".into())
    }
    fn matches(&self, _context: &dyn mod_api::LoopContext, _: &(), grid: CraftingGrid) -> bool {
        grid.width == 1
            && grid.height == 2
            && grid.cells == [ItemStack::new(self.milk, 1), ItemStack::new(self.stick, 1)]
    }
    fn assemble(
        &self,
        _context: &dyn mod_api::LoopContext,
        _: &(),
        _: CraftingGrid,
    ) -> Option<ItemStack> {
        ItemStack::new(self.diamond, 1)
    }
    fn remaining_items(
        &self,
        _context: &dyn mod_api::CraftingRemainders,
        _: &(),
        grid: CraftingGrid,
    ) -> Option<Vec<Option<ItemStack>>> {
        assert_eq!((grid.width, grid.height), (1, 2));
        assert_eq!(
            grid.cells,
            [ItemStack::new(self.milk, 1), ItemStack::new(self.stick, 1)]
        );
        Some(vec![
            ItemStack::new(self.bottle, 2),
            ItemStack::new(self.paper, 3),
        ])
    }
}
struct Recipes;
#[mod_api::__subscriptions]
impl mod_api::Hooks for Recipes {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let kind = Vertical {
            milk: r.item_key("minecraft:milk_bucket"),
            stick: r.item_key("minecraft:stick"),
            diamond: r.item_key("minecraft:diamond"),
            bottle: r.item_key("minecraft:glass_bottle"),
            paper: r.item_key("minecraft:paper"),
        };
        r.register_crafting_recipe_serializer("vertical", kind);
    }
}
#[test]
#[ignore = "daily"]
fn rectangular_recipe_for_preserves_two_custom_remainder_cells() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("rectangular", Box::new(Recipes))].into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let recipes = sunrise_sim::crafting::Recipes::from_sources(
                &game.options.resources,
                game.items(),
                &sunrise_world::loot::PackOverlay::NONE,
                vec![(
                    "rectangular:vertical".into(),
                    serde_json::json!({"type":"rectangular:vertical"}),
                )],
                game.res.core.item_data.context(),
            )
            .unwrap();
            let scratch = tempfile::tempdir().unwrap();
            game.server.world_root = scratch.path().to_path_buf();
            game.reload_datapacks(false, &[]).unwrap();
            game.server.world_tables.0.as_mut().unwrap().recipes = recipes;
            let stack =
                |name, count| ItemStack::new(mod_api::__private::item_key(name).unwrap(), count);
            let input = mod_api::RecipeInput::Crafting {
                width: 1,
                height: 2,
                slots: vec![
                    stack("minecraft:milk_bucket", 1),
                    stack("minecraft:stick", 1),
                ],
            };
            let level = game.level(game.dim.id()).unwrap();
            let recipe = level
                .recipe_for(
                    game,
                    &mod_api::Identifier::parse("minecraft:crafting").unwrap(),
                    &input,
                )
                .unwrap()
                .unwrap();
            assert_eq!(recipe.recipe.to_string(), "rectangular:vertical");
            assert_eq!(Some(recipe.output), stack("minecraft:diamond", 1));
            assert_eq!(
                recipe.remainders,
                vec![
                    stack("minecraft:glass_bottle", 2),
                    stack("minecraft:paper", 3)
                ]
            );
        });
    });
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
