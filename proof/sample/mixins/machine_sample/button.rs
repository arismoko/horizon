fn sample_button(
    game: &mut Game<'_>,
    entity: &mut ModBlockEntity,
    id: i32,
    carried: &mut Option<ItemStack>,
) {
    if entity.id != super::machine_sample::MACHINE || id != super::machine_sample::DEPOSIT {
        return;
    }
    let (Some(input), Some(stack)) = (entity.items.first_mut(), carried.as_mut()) else {
        return;
    };
    let limit = game
        .items()
        .get(&stack.item)
        .map_or(0, |item| item.max_stack);
    let compatible = input.as_ref().is_none_or(|item| {
        item.item == stack.item && item.damage == stack.damage && item.extra == stack.extra
    });
    let count = super::machine_sample::deposit_count(
        stack.count,
        input.as_ref().map_or(0, |item| item.count),
        limit,
        compatible,
    );
    if count == 0 {
        return;
    }
    if let Some(input) = input {
        input.count += count;
    } else {
        *input = Some(ItemStack {
            count,
            ..stack.clone()
        });
    }
    stack.count -= count;
    if stack.count == 0 {
        *carried = None;
    }
}
