//! Server-side recipe lookup (`LevelRef::recipe_for`) for native mods.
use crate::game::ServerGame;
use mod_api::{LevelRef, RecipeInput, RecipeLookupError, RecipeMatch};
use sunrise_gameplay_common::game::api_values::TryIntoApi;
use sunrise_items::items::ItemStack;
use sunrise_sim::crafting::Recipes;

impl ServerGame<'_> {
    pub(in crate::game) fn synced_recipes(&self) -> Option<&Recipes> {
        self.common.recipe_tables.synced.as_ref()
    }

    /// The engine's matchers, shared with crafting and station menus; no mod-side rematching.
    pub(in crate::game) fn level_recipe_for(
        &self,
        level: LevelRef,
        recipe_type: &mod_api::Identifier,
        inputs: &RecipeInput,
    ) -> Result<Option<RecipeMatch>, RecipeLookupError> {
        use RecipeLookupError::*;
        self.native_level_world(level).ok_or(UnavailableLevel)?;
        if recipe_type.namespace() != "minecraft" {
            return Err(NotMatchable);
        }
        let kind = recipe_type.path();
        let make = |id: &str, output: &ItemStack, remainders, experience, processing_time| {
            Ok(Some(RecipeMatch::new(
                mod_api::Identifier::parse(id).map_err(|_| InvalidRecipe)?,
                output.try_into_api().map_err(|_| InvalidItem)?,
                remainders,
                experience,
                processing_time,
            )))
        };
        let read_stack = |stack: &mod_api::ItemStack| {
            sunrise_gameplay_common::game::api_values::recipe_inputs::stack(stack, self.items())
                .map_err(|_| InvalidItem)
        };
        match kind {
            "crafting" => {
                let RecipeInput::Crafting {
                    width,
                    height,
                    slots,
                } = inputs
                else {
                    return Err(InvalidInput);
                };
                let (w, h) = (*width as usize, *height as usize);
                if w.checked_mul(h) != Some(slots.len()) {
                    return Err(InvalidInput);
                }
                let mut grid = Vec::with_capacity(slots.len());
                for stack in slots {
                    #[allow(clippy::needless_borrows_for_generic_args)] // TODO(#2273)
                    {
                        grid.push(stack.as_ref().map(&read_stack).transpose()?);
                    }
                }
                let scale = |id: i32| self.common.maps.data.get(&id).map(|m| m.scale);
                let recipes = self.recipes();
                let matched =
                    recipes.find_with_dimensions(&self.common.loop_context, &grid, w, w, h, &scale);
                let Some((id, output)) = matched.filter(|(_, output)| output.count > 0) else {
                    return Ok(None);
                };
                let remainders = recipes
                    .remainders_rect(&self.common.loop_context, id, &grid, w, w, h, &|s| {
                        self.crafting_remainder(s)
                    })
                    .into_iter()
                    .map(|stack| stack.try_into_api().map_err(|_| InvalidItem))
                    .collect::<Result<Vec<_>, _>>()?;
                make(id, &output, remainders, 0.0, 0)
            }
            "smelting" | "blasting" | "smoking" | "campfire_cooking" => {
                let RecipeInput::Single(input) = inputs else {
                    return Err(InvalidInput);
                };
                let input = read_stack(input)?;
                let full_kind = match kind {
                    "smelting" => "minecraft:smelting",
                    "blasting" => "minecraft:blasting",
                    "smoking" => "minecraft:smoking",
                    _ => "minecraft:campfire_cooking",
                };
                let Some(recipe) =
                    self.recipes()
                        .cook(&self.common.loop_context, full_kind, &input)
                else {
                    return Ok(None);
                };
                make(
                    &recipe.id,
                    &recipe.result,
                    vec![None],
                    recipe.experience,
                    recipe.time,
                )
            }
            "stonecutting" => {
                let RecipeInput::Single(input) = inputs else {
                    return Err(InvalidInput);
                };
                let input = read_stack(input)?;
                let Some(recipe) = self
                    .stations()
                    .recipes
                    .stonecutting
                    .iter()
                    .find(|r| r.input.test_in(&self.common.loop_context, &input))
                else {
                    return Ok(None);
                };
                make(&recipe.id, &recipe.result, vec![None], 0.0, 0)
            }
            "smithing" => {
                let RecipeInput::Smithing {
                    template,
                    base,
                    addition,
                } = inputs
                else {
                    return Err(InvalidInput);
                };
                #[allow(clippy::needless_borrows_for_generic_args)] // TODO(#2273)
                let template = template.as_ref().map(&read_stack).transpose()?;
                #[allow(clippy::needless_borrows_for_generic_args)] // TODO(#2273)
                let base = base.as_ref().map(&read_stack).transpose()?;
                #[allow(clippy::needless_borrows_for_generic_args)] // TODO(#2273)
                let addition = addition.as_ref().map(&read_stack).transpose()?;
                let stations = self.stations();
                let Some((id, output)) = stations.recipes.smithing_recipe(
                    &self.common.loop_context,
                    &stations.enchantments,
                    template.as_ref(),
                    base.as_ref(),
                    addition.as_ref(),
                ) else {
                    return Ok(None);
                };
                make(id, &output, vec![None; 3], 0.0, 0)
            }
            _ => Err(NotMatchable),
        }
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::native_block_states::NativeBlockStatesGameExt1 as _;

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
