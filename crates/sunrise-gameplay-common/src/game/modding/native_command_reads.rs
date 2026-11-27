#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    use sunrise_sim::advancements::{Advancement, Criterion, Display, Frame, Rewards};

    pub fn mine_stone() -> Advancement {
        let criterion = |name: &str| Criterion {
            name: name.into(),
            trigger: "minecraft:inventory_changed".into(),
            conditions: sunrise_world::lazy_json::LazyJson::new(&serde_json::json!({})),
            player: None,
        };
        let mut icon = sunrise_sim::crafting::recipe_result(&serde_json::json!({
            "id": "minecraft:wooden_pickaxe",
            "components": {"minecraft:enchantment_glint_override": true}
        }))
        .unwrap();
        icon.count = 1;
        Advancement {
            id: "minecraft:story/mine_stone".into(),
            parent: Some("minecraft:story/root".into()),
            display: Some(Display {
                icon,
                title: serde_json::json!({"translate": "advancements.story.mine_stone.title"}),
                description: serde_json::json!("Mine Stone"),
                frame: Frame::Task,
                background: None,
                show_toast: true,
                announce: true,
                hidden: false,
            }),
            criteria: vec![criterion("get_stone"), criterion("get_cobbled_deepslate")],
            requirements: vec![vec!["get_stone".into(), "get_cobbled_deepslate".into()]],
            rewards: Rewards::default(),
        }
    }
}
