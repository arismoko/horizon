use super::*;

impl Game<'_> {
    pub(super) fn held(&self) -> Option<&ItemStack> {
        self.stack_in(self.held_slot())
    }
}

pub(super) fn held_attack_item(
    inventory: &[Option<ItemStack>],
    selected: usize,
) -> Option<&Arc<str>> {
    inventory.get(selected)?.as_ref().map(|s| &s.item)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(item: &str) -> ItemStack {
        ItemStack {
            item: item.into(),
            count: 1,
            damage: 0,
            extra: None,
        }
    }

    #[test]
    fn held_attack_item_matches_the_cloned_stack() {
        let mut inventory: Vec<Option<ItemStack>> = vec![None; 36];
        inventory[0] = Some(stack("minecraft:stone"));
        inventory[3] = Some(stack("minecraft:diamond_sword"));
        for selected in [0, 1, 3, 35, 36, 1000] {
            // The clone the tick used to make, as the reference.
            let cloned = inventory.get(selected).cloned().flatten().map(|s| s.item);
            assert_eq!(held_attack_item(&inventory, selected).cloned(), cloned);
        }
        assert_eq!(held_attack_item(&[], 0), None);
    }
}
