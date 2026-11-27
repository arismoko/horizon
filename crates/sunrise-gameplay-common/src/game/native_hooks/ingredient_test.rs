use super::*;

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Sunrise does.
impl crate::game::CommonGame<'_> {
    /// Whether `stack` fits `ingredient` (`Ingredient.test`), matched with this
    /// game's loop context.
    pub fn recipe_ingredient_test(&self, ingredient: &Ingredient, stack: &ItemStack) -> bool {
        ingredient.test_in(&self.loop_context, stack)
    }
}

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Sunrise does.
pub trait IngredientTestGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// Enumerate once at publication using the engine predicate, never a second matcher.
    fn recipe_ingredient_item_keys(
        &self,
        ingredient: &Ingredient,
    ) -> Result<std::sync::Arc<[mod_api::ItemKey]>, mod_api::IngredientKeysError> {
        use mod_api::IngredientKeysError::*;
        let mut pending = Vec::from([ingredient]);
        let mut candidates = std::collections::HashSet::new();
        while let Some(node) = pending.pop() {
            if node.custom.is_some() || node.rule.is_some() {
                return Err(NotEnumerable);
            }
            if let Some((shape, children)) = &node.children {
                if let mod_api::item::crafting::Shape::Intersection = shape
                    && children.is_empty()
                {
                    // The existing matcher defines all([]) as true. Such a programmatically
                    // constructed shape needs the full candidate universe, not an empty list.
                    candidates.extend(self.items().items.keys().map(|name| &**name));
                }
                pending.extend(children);
            } else {
                candidates.extend(node.items.iter().map(String::as_str));
            }
        }
        let mut accepted = Vec::new();
        for name in candidates {
            let (name, _) = self.items().items.get_key_value(name).ok_or(UnknownItem)?;
            let key = mod_api::__private::item_key(name).ok_or(UnknownItem)?;
            let stack = ItemStack::new(name, 1);
            if ingredient.test_in(&self.loop_context, &stack) {
                accepted.push(key);
            }
        }
        Ok(accepted.into())
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> IngredientTestGameExt1<'__sg> for G {}

use crate::game::tables::GameItems as _;
