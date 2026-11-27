//! Crafting remainders shared by menus and automated crafters.
use super::*;
impl Recipes {
    /// `Recipe.getRemainingItems` placed on the `size`x`size` grid: an item's
    /// crafting remainder (buckets, bottles), the patterned banner a duplicate
    /// keeps and the written book a clone keeps.
    pub fn remainders(
        &self,
        context: &LoopContext,
        recipe: &str,
        grid: &[Option<ItemStack>],
        size: usize,
        remainder: &dyn Fn(&ItemStack) -> Option<ItemStack>,
    ) -> Vec<Option<ItemStack>> {
        let mut out = self.remainders_rect(context, recipe, grid, size, size, size, remainder);
        if out.len() < size * size {
            out.resize(size * size, None);
        }
        out
    }

    /// `Recipe.getRemainingItems` placed on a `width`x`height` grid laid out with row stride `stride`.
    pub fn remainders_rect(
        &self,
        context: &LoopContext,
        recipe: &str,
        grid: &[Option<ItemStack>],
        stride: usize,
        width: usize,
        height: usize,
        remainder: &dyn Fn(&ItemStack) -> Option<ItemStack>,
    ) -> Vec<Option<ItemStack>> {
        let kind = self
            .crafting
            .iter()
            .find(|(n, _)| n == recipe)
            .map(|(_, r)| r);
        if matches!(kind, Some(Recipe::Mod(_))) {
            return self
                .mod_remainders_rect(context, recipe, grid, stride, width, height, remainder);
        }
        let mut out = vec![None; grid.len()];
        let Some(input) = Input::of_rect(grid, stride, width, height) else {
            return out;
        };
        for (i, cell) in input.cells.iter().enumerate() {
            let Some(s) = cell else { continue };
            let at = (input.top + i / input.width) * stride + input.left + i % input.width;
            let rest = remainder(s);
            if rest.is_some() {
                out[at] = rest;
                continue;
            }
            let copy = ItemStack {
                count: 1,
                ..(*s).clone()
            };
            match kind {
                Some(Recipe::Special(Special::BannerDuplicate { .. }))
                    if !s.banner_layers().is_empty() =>
                {
                    out[at] = Some(copy);
                }
                // BookCloningRecipe stops at the first copied book.
                Some(Recipe::Special(Special::BookCloning { .. }))
                    if s.data().is_some_and(|d| d.written.is_some()) =>
                {
                    out[at] = Some(copy);
                    break;
                }
                _ => {}
            }
        }
        out
    }
}
