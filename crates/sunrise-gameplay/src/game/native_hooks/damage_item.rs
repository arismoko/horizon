#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static DISPATCHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl crate::game::ServerGame<'_> {
    #[cfg(any(test, feature = "test-support"))]
    pub fn damage_item_dispatches(&self) -> usize {
        DISPATCHES.with(std::cell::Cell::get)
    }

    /// `IItemExtension.damageItem`: `stack`, worn or used by `entity`, is
    /// about to lose `amount` points of durability (`ItemStack.hurtAndBreak`),
    /// after Unbreaking. Change `amount` (0 or less leaves the item whole),
    /// set `canceled` for no wear, or set `break_now` to break the item.
    /// Server-side.
    pub(in crate::game) fn native_damage_item(
        &mut self,
        stack: &ItemStack,
        entity: Option<LivingRef>,
        amount: &mut i32,
        canceled: &mut bool,
        break_now: &mut bool,
    ) {
        #[cfg(any(test, feature = "test-support"))]
        DISPATCHES.with(|count| count.set(count.get() + 1));
        self.fire_damage_item(stack, entity, amount, canceled, break_now);
    }
}
