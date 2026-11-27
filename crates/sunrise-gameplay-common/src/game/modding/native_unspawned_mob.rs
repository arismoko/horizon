#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    use mod_api::Identifier;
    #[cfg(any(test, feature = "test-support"))]
    use mod_api::ItemStack;

    pub const AT: [f64; 3] = [8.5, 64.0, 8.5];
    pub fn id(s: &str) -> Identifier {
        Identifier::parse(s).unwrap()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn pumpkin() -> ItemStack {
        crate::game::api_values::test_snapshot(&sunrise_items::items::ItemStack::new(
            "minecraft:carved_pumpkin",
            1,
        ))
    }
    #[cfg(any(test, feature = "test-support"))]
    /// A mod goal that only names itself: a mob carries it under this name
    /// while it has its goal, and as `ModGoal` once a copy left it out.
    struct Kept;
    #[cfg(any(test, feature = "test-support"))]
    impl mod_api::Goal for Kept {
        fn flags(&self) -> mod_api::GoalFlags {
            mod_api::GoalFlags::MOVE
        }
        fn name(&self) -> &'static str {
            "Kept"
        }
        fn can_use(&mut self, _: &mut mod_api::GoalContext<'_>) -> bool {
            false
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    struct KeptGoal;
    #[cfg(any(test, feature = "test-support"))]
    #[mod_api::sunrise_part]
    impl mod_api::EntityPart for KeptGoal {
        fn register_goals(
            &self,
            _host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::RegisterGoalsEvent<'_, '_>,
        ) {
            event.goals.add_goal(7, Kept);
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub struct KeptGoalMod;
    #[cfg(any(test, feature = "test-support"))]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for KeptGoalMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.entity_part("minecraft:zombie", KeptGoal);
        }
    }
}
