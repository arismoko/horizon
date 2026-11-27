#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    use serde_json::Value;
    use sunrise_sim::advancements::{self as adv, Frame};

    pub fn announced(title: Value, description: Value) -> adv::Advancement {
        adv::Advancement {
            id: "quests:board".into(),
            parent: None,
            display: Some(adv::Display {
                icon: sunrise_items::items::ItemStack::new("minecraft:book", 1),
                title,
                description,
                frame: Frame::Task,
                background: None,
                show_toast: true,
                announce: true,
                hidden: false,
            }),
            criteria: Vec::new(),
            requirements: Vec::new(),
            rewards: adv::Rewards::default(),
        }
    }
}
