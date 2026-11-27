use super::*;

/// The local player's `TickThrottler(20, 1280)`.
#[derive(Clone, Copy, Default)]
pub struct DropSpamThrottler(i32);

impl DropSpamThrottler {
    pub fn tick(&mut self) {
        if self.0 > 0 {
            self.0 -= 1;
        }
    }

    pub fn is_under_threshold(self) -> bool {
        self.0 < 1280
    }

    pub fn increment(&mut self) {
        self.0 = self.0.wrapping_add(20);
    }
}

impl Menu {
    /// Attach the local player's `dropSpamThrottler` so `Input::Throw` checks
    /// `LocalPlayer.canDropItems` and reflects accepted creative drops.
    pub fn set_drop_spam_throttler(&mut self, throttle: DropSpamThrottler) {
        self.touch();
        self.state.drop_spam = Some(throttle);
    }

    /// Current drop throttler after running clicks on this menu.
    pub fn drop_spam_throttler(&self) -> Option<DropSpamThrottler> {
        self.state.drop_spam
    }

    pub(super) fn can_throw(&self) -> bool {
        self.state
            .drop_spam
            .is_none_or(DropSpamThrottler::is_under_threshold)
    }

    pub(super) fn throw_stack(
        &mut self,
        taken: &ItemStack,
        items: &Items,
        on_creative_drop: &mut dyn FnMut(&ItemStack) -> bool,
    ) {
        if taken.is_empty() {
            return;
        }
        self.drop_stack(taken, items);
        if on_creative_drop(taken)
            && let Some(throttle) = &mut self.state.drop_spam
        {
            throttle.increment();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{crafting::Ingredient, stations::Stonecutting};

    #[test]
    fn drop_spam_burst_and_tick_recovery() {
        let mut throttle = DropSpamThrottler::default();
        throttle.tick(); // Idle ticks do not accumulate credit.
        for _ in 0..64 {
            assert!(throttle.is_under_threshold());
            throttle.increment();
        }
        assert!(!throttle.is_under_threshold());
        assert_eq!(throttle.0, 1280);
        throttle.tick();
        assert!(throttle.is_under_threshold());
        throttle.increment();
        assert_eq!(throttle.0, 1299);
        for _ in 0..19 {
            assert!(!throttle.is_under_threshold());
            throttle.tick();
        }
        assert_eq!(throttle.0, 1280);
        assert!(!throttle.is_under_threshold());
        throttle.tick();
        assert_eq!(throttle.0, 1279);
        assert!(throttle.is_under_threshold());
    }

    #[test]
    fn ctrl_q_rechecks_drop_spam_before_each_replenished_take() {
        super::super::tests::bare_env(|env| {
            // Each take refills the result with the same item, like the repeat
            // slot in SyntheticContainerDrop. Five inputs allow five takes.
            let input = ItemStack::new("minecraft:stone", 5);
            let result = ItemStack::new("minecraft:stone_slab", 1);
            let recipes = env.station_recipes.with_stonecutting(vec![Stonecutting {
                id: "test:cut_stone".into(),
                input: Ingredient::of(vec![input.item.to_string()]),
                result: result.clone(),
            }]);
            let env = Env {
                station_recipes: &recipes,
                ..*env
            };
            let mut menu = Menu::new(Kind::Stonecutter);
            menu.set_slot(&env, 0, Some(input));
            assert!(menu.button(&env, 0));
            assert_eq!(menu.slot(1), Some(&result));

            let mut throttle = DropSpamThrottler::default();
            for _ in 0..62 {
                throttle.increment();
            }
            menu.set_drop_spam_throttler(throttle);
            // First drop is refused/canceled (returns false), so it must not
            // consume budget: two accepted drops still fit before hitting 1280.
            let mut calls = 0usize;
            let mut accepted = Vec::new();
            menu.click_with_creative_drop(&env, 1, 1, Input::Throw, |stack| {
                calls += 1;
                if calls == 1 {
                    return false;
                }
                accepted.push(stack.clone());
                true
            })
            .unwrap();
            assert_eq!(calls, 3);
            assert_eq!(accepted, [result.clone(), result.clone()]);
            assert_eq!(menu.slot(0).map(|s| s.count), Some(2));
            assert_eq!(menu.slot(1), Some(&result));

            let mut after_cap = 0usize;
            menu.click_with_creative_drop(&env, 1, 0, Input::Throw, |_| {
                after_cap += 1;
                true
            })
            .unwrap();
            assert_eq!(menu.slot(0).map(|s| s.count), Some(2));
            assert_eq!(menu.slot(1), Some(&result));
            assert_eq!(after_cap, 0);
        });
    }
}
