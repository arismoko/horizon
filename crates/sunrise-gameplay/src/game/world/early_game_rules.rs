//! Install the opening's authoritative rules before either early server hook.
use super::world_loading::PreparedWorld;
use sunrise_game_data::gamerules::{GameRules, Value};

impl crate::game::ServerGame<'_> {
    pub fn prepare_world_and_rules(
        &mut self,
        request: super::world_loading::WorldRequest,
    ) -> sunrise_game_data::Result<PreparedWorld> {
        self.common.join_world_rest()?;
        sunrise_level::spans::startup::arm("world.opening");
        let prepared = request.prepare(
            &self.common.res,
            &self.common.pack,
            &self.common.options,
            &sunrise_world::datafix::progress::UpgradeProgress::new(),
        )?;
        self.load_opening_game_rules(&prepared);
        Ok(prepared)
    }

    pub fn prepare_early_game_rules(
        &mut self,
        loader: super::world_loading::WorldLoader,
    ) -> sunrise_game_data::Result<PreparedWorld> {
        let prepared = loader.finish()?;
        self.load_opening_game_rules(&prepared);
        Ok(prepared)
    }

    pub(in crate::game::server_side) fn load_opening_game_rules(
        &mut self,
        prepared: &PreparedWorld,
    ) {
        let level = &prepared.level;
        let mut rules = GameRules::load_with_registry(
            &prepared.save_root,
            &level.enabled_features,
            level.rule_registry.clone(),
        );
        for (id, raw) in &level.game_rules {
            let value = match rules.get(id) {
                Some(Value::Bool(_)) => Value::Bool(*raw != 0),
                Some(Value::Int(_)) => Value::Int(*raw),
                None => continue,
            };
            rules.set(id, value);
        }
        self.common.ui.rules = rules;
    }
}
