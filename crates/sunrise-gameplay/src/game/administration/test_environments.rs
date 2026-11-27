//! `TestEnvironmentDefinition`: what a game test batch changes in the world
//! while it runs (game rules, weather, the day time, difficulty, functions,
//! all of them), set up when the batch starts and torn down after it. The
//! definitions are the data pack's `test_environment` entries, or inline.
use crate::game::ServerGame;
use sunrise_game_data::gamerules::Value;
use sunrise_gameplay_common::game::{
    Dim,
    native_rain_read::{weather_flags, weather_for_world},
};
pub(in crate::game) use sunrise_gameplay_common::test_environments::{
    Activation, Environment, Saved, WeatherType,
};

impl ServerGame<'_> {
    /// Looks up the batch environment in the world's frozen registry.
    pub fn test_environment(&self, id: &str) -> Option<Environment> {
        let table = match &self.server.world_tables.0 {
            Some(tables) => &tables.test_environments,
            None => self.common.res.test_environments(),
        };
        if id.contains(':') {
            table.get(id).cloned()
        } else {
            table.get(&format!("minecraft:{id}")).cloned()
        }
    }

    /// `TestEnvironmentDefinition.activate`.
    pub(in crate::game::server_side) fn activate_test_environment(
        &mut self,
        environment: Environment,
        dim: Dim,
    ) -> Activation {
        let saved = self.set_up_environment(&environment, dim);
        Activation { environment, saved }
    }

    /// `Activation.teardown`.
    pub(in crate::game::server_side) fn tear_down_test_environment(
        &mut self,
        activation: Activation,
        dim: Dim,
    ) {
        self.tear_down_environment(&activation.environment, activation.saved, dim);
    }

    fn set_up_environment(&mut self, environment: &Environment, dim: Dim) -> Saved {
        match environment {
            Environment::AllOf(all) => Saved::AllOf(
                all.iter()
                    .map(|e| (e.clone(), self.set_up_environment(e, dim)))
                    .collect(),
            ),
            Environment::ClockTime { clock, time } => {
                // Only the overworld clock runs in Sunrise.
                let previous = self.common.world.day_time;
                if clock == "minecraft:overworld" {
                    self.common.world.day_time = *time;
                    self.common.ui.clock.partial = 0.0;
                }
                Saved::Clock(previous)
            }
            Environment::Difficulty(d) => {
                let previous = self.common.ui.difficulty;
                self.set_difficulty_forced(*d);
                Saved::Difficulty(previous)
            }
            Environment::Functions { setup, .. } => {
                if let Some(id) = setup {
                    self.run_environment_function(id);
                }
                Saved::Unit
            }
            Environment::GameRules(rules) => {
                let original = rules
                    .iter()
                    .filter_map(|(name, _)| {
                        let rule = self.common.ui.rules.rule(name)?;
                        Some((
                            rule.name.to_owned(),
                            self.common.ui.rules.get(&rule.name).unwrap_or(rule.default),
                        ))
                    })
                    .collect();
                self.set_environment_rules(rules);
                Saved::GameRules(original)
            }
            Environment::Timelines => Saved::Unit,
            Environment::Weather(kind) => {
                let world = if dim == self.dim {
                    &self.common.world
                } else if let Some(world) = self.server.levels.0.parked_world(dim) {
                    world
                } else {
                    log::warn!(
                        "cannot set up GameTest weather in {}: level is not resident",
                        dim.id()
                    );
                    return Saved::Unit;
                };
                let previous = match weather_flags(weather_for_world(world, dim == Dim::End)) {
                    (_, true) => WeatherType::Thunder,
                    (true, false) => WeatherType::Rain,
                    (false, false) => WeatherType::Clear,
                };
                if self.set_weather_type(dim, *kind) {
                    Saved::Weather(previous)
                } else {
                    Saved::Unit
                }
            }
        }
    }

    fn tear_down_environment(&mut self, environment: &Environment, saved: Saved, dim: Dim) {
        match (environment, saved) {
            (Environment::AllOf(_), Saved::AllOf(all)) => {
                for (e, s) in all.into_iter().rev() {
                    self.tear_down_environment(&e, s, dim);
                }
            }
            (Environment::ClockTime { clock, .. }, Saved::Clock(previous)) => {
                if clock == "minecraft:overworld" {
                    self.common.world.day_time = previous;
                    self.common.ui.clock.partial = 0.0;
                }
            }
            (_, Saved::Difficulty(d)) => self.set_difficulty_forced(d),
            (
                Environment::Functions {
                    teardown: Some(id), ..
                },
                Saved::Unit,
            ) => self.run_environment_function(id),
            (_, Saved::GameRules(original)) => self.set_environment_rules(&original),
            (_, Saved::Weather(previous)) => {
                // resetWeatherCycle, then the saved type.
                let w = &mut self.common.world.weather;
                w.rain_time = 0;
                w.raining = false;
                w.thunder_time = 0;
                w.thundering = false;
                self.set_weather_type(dim, previous);
            }
            _ => {}
        }
    }

    /// `GameRules.setAll`.
    fn set_environment_rules(&mut self, rules: &[(String, Value)]) {
        for (name, value) in rules {
            if let Some(rule) = self.common.ui.rules.rule(name).cloned()
                && self.common.ui.rules.set(&rule.name, *value)
            {
                self.game_rule_changed(&rule.name, *value, None);
            }
        }
        self.common.sync_rules();
    }

    /// `Weather.Type.apply`: `ServerLevel.setWeatherParameters`.
    fn set_weather_type(&mut self, dim: Dim, kind: WeatherType) -> bool {
        let (clear, rain, raining, thundering) = kind.parameters();
        self.set_weather_parameters(dim.id(), clear, rain, raining, thundering)
    }

    /// `Functions.run`: the function with game master permission and
    /// suppressed output; a missing one is logged.
    fn run_environment_function(&mut self, id: &str) {
        let id = if id.contains(':') {
            id.to_owned()
        } else {
            format!("minecraft:{id}")
        };
        let found = self
            .functions_named(&id)
            .ok()
            .and_then(|f| f.into_iter().next());
        match found {
            Some(function) => {
                let source = crate::game::commands::execute::Source {
                    silent: true,
                    ..self.server_source()
                };
                self.server.exec.0.commands = 0;
                let _ = self.run_function(&function, None, source);
            }
            None => log::error!("Test Batch failed for non-existent function {id}"),
        }
    }
}
