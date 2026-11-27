#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    use sunrise_items::items::ItemStack;

    use sunrise_sim::merchant::{ItemCost, MerchantOffer};

    pub fn offer(cost: &str, result: &str) -> MerchantOffer {
        MerchantOffer {
            base_cost_a: ItemCost {
                stack: ItemStack::new(cost, 20),
            },
            cost_b: None,
            result: ItemStack::new(result, 1),
            uses: 1,
            max_uses: 12,
            reward_exp: true,
            special_price_diff: -2,
            demand: 3,
            price_multiplier: 0.05,
            xp: 7,
        }
    }
}
