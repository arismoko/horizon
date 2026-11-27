//! Allocation-free projection of a live engine stack into an ingredient predicate.
use mod_api::{__private, IngredientStack};
use sunrise_core::nbt::Compound;
use sunrise_items::{
    item_data::ingredient_components::removed, items::ItemStack, properties::component_int,
};

pub(super) fn with_ingredient_stack<R>(
    stack: &ItemStack,
    defaults: Option<&Compound>,
    read: impl FnOnce(IngredientStack<'_>) -> R,
) -> R {
    let prototype = |id: &str| {
        defaults?
            .iter()
            .find_map(|(key, tag)| key.0.iter().copied().eq(id.encode_utf16()).then_some(tag))
    };
    let damage_id = "minecraft:damage";
    let damage = if stack.carries_component(None, damage_id) {
        Some(stack.effective_damage(None) as u32)
    } else if removed(stack, damage_id) {
        None
    } else {
        prototype(damage_id)
            .and_then(component_int)
            .map(|damage| damage as u32)
    };
    read(__private::ingredient_stack(
        &stack.item,
        __private::item_key(&stack.item),
        stack.count,
        damage,
        &|id| {
            !id.starts_with('!')
                && (stack.carries_component(None, id)
                    || (prototype(id).is_some() && !removed(stack, id)))
        },
        &|id| removed(stack, id),
        &|id| {
            if id.starts_with('!') {
                return None;
            }
            stack.data().and_then(|data| data.saved_other(id))
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_core::nbt::Tag;
    use sunrise_items::item_data::decode_components;

    #[test]
    fn ingredient_stack_lends_live_component_data() {
        let mut stack = ItemStack::new("fixture:ingredient_stack", 3);
        with_ingredient_stack(&stack, None, |view| assert_eq!(view.damage(), None));

        let patch = Tag::compound([
            ("minecraft:damage", Tag::Int(0)),
            (
                "!minecraft:max_stack_size",
                Tag::Compound(Default::default()),
            ),
            (
                "minecraft:custom_data",
                Tag::compound([("grade", Tag::Int(7))]),
            ),
        ]);
        (stack.damage, stack.extra) = decode_components(Some(&patch));
        let stored = stack
            .data()
            .unwrap()
            .saved_other("minecraft:custom_data")
            .unwrap();

        with_ingredient_stack(&stack, None, |view| {
            assert_eq!(view.item_id(), "fixture:ingredient_stack");
            assert_eq!(view.item_key(), None);
            assert_eq!(view.count(), 3);
            assert_eq!(view.damage(), Some(0));
            assert!(view.has_component("minecraft:damage"));
            assert_eq!(view.stored_component("minecraft:damage"), None);
            assert!(!view.has_component("minecraft:max_stack_size"));
            assert!(view.component_removed("minecraft:max_stack_size"));
            assert!(!view.component_removed("minecraft:unbreakable"));
            assert_eq!(view.stored_component("minecraft:max_stack_size"), None);
            assert!(view.has_component("minecraft:custom_data"));
            let lent = view.stored_component("minecraft:custom_data").unwrap();
            assert_eq!(lent.get("grade"), Some(&Tag::Int(7)));
            assert!(std::ptr::eq(lent, stored));
        });
    }
}
