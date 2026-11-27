#![cfg(test)]
use sunrise_gameplay_common::game::transactions::scratch::ScratchGameExt1 as _;

#[cfg(test)]
mod tests {
    use sunrise_core::nbt::Tag;
    use {
        crate::game::transactions::scratch::*,
        sunrise_gameplay_common::game::transactions::scratch::{ItemPlace, ItemStack},
    };
    use {
        sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
        sunrise_gameplay::game::native_entity_refs::tests::with_game,
    };

    #[test]
    #[ignore = "daily"]
    fn scratch_merges_explicit_default_components_without_overflow() {
        if run_fresh_cases(
            "cut_layout::game::transactions::scratch::tests::scratch_merges_explicit_default_components_without_overflow",
            "SUNRISE_SCRATCH_COMPONENT_CHILD",
            &["run"],
        ) {
            return;
        }
        with_game(|game| {
            let held = ItemStack::new("minecraft:stick", 1);
            let mut offer = held.clone();
            offer
                .data_mut()
                .set_other("minecraft:max_stack_size", Some(Tag::Int(64)));
            assert!(!held.same_item_same_components(&offer));
            let taken = game
                .scratch_scope(None, held, |game, _, place| {
                    let ItemPlace::Scratch(slot) = *place else {
                        unreachable!()
                    };
                    assert_eq!(game.scratch_insert(slot, &offer), 1);
                    true
                })
                .unwrap()
                .unwrap();
            assert_eq!(taken.stack, Some(ItemStack::new("minecraft:stick", 2)));
            assert_eq!(taken.overflow, None);
        });
    }
}
