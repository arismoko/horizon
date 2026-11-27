use super::api_values::api_game_rule_value;
use core::ops::ControlFlow;
use mod_api::{GameRuleReadError, GameRuleValue, ServerReadError};
use sunrise_game_data::gamerules::Value;

impl crate::game::CommonGame<'_> {
    pub fn game_rule(&self, id: &str) -> Result<Value, GameRuleReadError> {
        self.ui.rules.get(id).ok_or(GameRuleReadError::UnknownRule)
    }

    pub fn visit_game_rules(
        &self,
        visit: &mut dyn FnMut(&str, GameRuleValue) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, ServerReadError> {
        Ok(self.visit_available_game_rules(visit))
    }

    /// Both server and level views use the server's feature-filtered rule store.
    pub fn visit_available_game_rules(
        &self,
        visit: &mut dyn FnMut(&str, GameRuleValue) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        self.ui
            .rules
            .available_rule_values()
            .try_for_each(|(rule, value)| visit(&rule.name, api_game_rule_value(value)))
    }
}
