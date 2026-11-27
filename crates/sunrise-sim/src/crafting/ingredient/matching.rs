//! Concrete stack matching, shared by ordinary crafting and placement.
use super::*;
impl Ingredient {
    /// `test` with the loop context the game already holds.
    pub fn test_in(
        &self,
        context: &sunrise_world::native_load_context::LoopContext,
        stack: &ItemStack,
    ) -> bool {
        self.test_in_guarded(context, stack, &|| true)
            .unwrap_or(false)
    }
    /// Stop before another custom leaf runs if a policy changed the caller's
    /// context. `None` is interruption, never a failed ingredient predicate.
    pub fn test_in_guarded(
        &self,
        context: &sunrise_world::native_load_context::LoopContext,
        stack: &ItemStack,
        guard: &impl Fn() -> bool,
    ) -> Option<bool> {
        if self.children.is_none() {
            return self.test_leaf(context, stack, guard);
        }
        // Data-file trees fit in the inline stack (serde_json bounds nesting).
        // Native placement declarations aren't limited by that parser; only
        // their deeper branches need heap storage, without any depth limit.
        let mut inline = [(self, 0); 128];
        let mut overflow = Vec::new();
        let mut depth = 0;
        let mut current = self;
        let mut completed = None;
        loop {
            let result = if let Some((shape, children)) = &current.children {
                let next = match (shape, completed) {
                    (_, None) => Ok(0),
                    (Shape::Compound, Some((_, true))) => Err(true),
                    (Shape::Intersection, Some((_, false))) => Err(false),
                    (Shape::Compound | Shape::Intersection, Some((index, _))) => Ok(index + 1),
                    (Shape::CustomDisplay, Some((_, accepted))) => Err(accepted),
                    (Shape::Difference, Some((0, false))) => Err(false),
                    (Shape::Difference, Some((0, true))) => Ok(1),
                    (Shape::Difference, Some((_, accepted))) => Err(!accepted),
                };
                match next {
                    Ok(index) => {
                        if let Some(child) = children.get(index) {
                            let frame = (current, index);
                            if let Some(slot) = inline.get_mut(depth) {
                                *slot = frame;
                            } else {
                                overflow.push(frame);
                            }
                            depth += 1;
                            current = child;
                            completed = None;
                            continue;
                        }
                        matches!(shape, Shape::Intersection)
                            || (matches!(shape, Shape::Difference) && index == 1)
                    }
                    Err(result) => result,
                }
            } else {
                current.test_leaf(context, stack, guard)?
            };
            if depth == 0 {
                return Some(result);
            }
            depth -= 1;
            let (parent, index) = if let Some(&frame) = inline.get(depth) {
                frame
            } else {
                overflow.pop().expect("one saved parent per level")
            };
            current = parent;
            completed = Some((index, result));
        }
    }

    fn test_leaf(
        &self,
        context: &sunrise_world::native_load_context::LoopContext,
        stack: &ItemStack,
        guard: &impl Fn() -> bool,
    ) -> Option<bool> {
        if (self.custom.is_none() || !self.items.is_empty()) && !self.items.contains(&*stack.item) {
            return Some(false);
        }
        if self.rule.is_some() {
            return Some(
                self.compiled
                    .as_ref()
                    .is_some_and(|rule| rule.matches(stack)),
            );
        }
        if let Some(predicate) = &self.custom {
            if !guard() {
                return None;
            }
            let reader = sunrise_level::api_context::LoopReader(context);
            let defaults = self.component_defaults.get(stack.item.as_ref());
            let accept = super::stack::with_ingredient_stack(stack, defaults, |view| {
                predicate.test(&reader.context(), view)
            });
            return guard().then_some(accept);
        }
        Some(true)
    }
}
