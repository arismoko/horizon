use super::{Arg, Context, R, err};
use crate::game::commands::test_runs::Finder;
use crate::game::commands::test_runs::Pos;
use crate::game::commands::test_runs::rotation_for_steps;
use sunrise_game_data::text::{self, Text};
use {
    crate::game::game_tests::Grid, crate::game::game_tests::Info, crate::game::game_tests::Retry,
    crate::game::game_tests::RunnerOptions, crate::game::game_tests::verify_batches,
    crate::game::test_instances::TestDef, sunrise_gameplay_common::game::Dim,
};

impl crate::game::ServerGame<'_> {
    /// The tests a finder gives (`findTests`), by id.
    pub(in crate::game::server_side) fn finder_tests(
        &self,
        finder: &Finder,
    ) -> Vec<(String, TestDef)> {
        let keys: Vec<String> = match finder {
            Finder::Failed(only_required) => self
                .server
                .game_tests
                .0
                .failed
                .iter()
                .filter(|k| !only_required || self.test_def(k).is_some_and(|def| def.required))
                .cloned()
                .collect(),
            Finder::Selection { tests, copies } => {
                (0..*copies).flat_map(|_| tests.iter().cloned()).collect()
            }
            _ => Vec::new(),
        };
        keys.into_iter()
            .filter_map(|k| self.test_def(&k).map(|def| (k, def)))
            .collect()
    }
    /// `TestCommand.run`.
    pub(in crate::game::server_side) fn run_tests(
        &mut self,
        finder: &Finder,
        retry: Retry,
        steps: i32,
        per_row: i32,
    ) -> R<i32> {
        self.clear_game_tests();
        let mut infos = Vec::new();
        for pos in self.finder_positions(finder) {
            infos.extend(self.test_info_at(pos, retry));
        }
        for (key, def) in self.finder_tests(finder) {
            infos.extend(self.test_info_for(key, def, steps, retry)?);
        }
        if infos.is_empty() {
            self.feedback_unbroadcast(Text::translate("commands.test.no_tests", &[]));
            return Ok(0);
        }
        self.server.game_tests.0.failed.clear();
        self.feedback_unbroadcast(Text::translate(
            "commands.test.run.running",
            &[Text::literal(infos.len().to_string())],
        ));
        let grid = Grid::new(self.test_around(), per_row);
        self.track_and_start_command_runner(infos, grid);
        Ok(1)
    }
    /// `CommandSourceStack.sendFailure`.
    pub(in crate::game::server_side) fn test_failure(&mut self, message: Text) {
        if self
            .server
            .command_source
            .as_ref()
            .is_some_and(|s| s.silent)
        {
            return;
        }
        self.send_command_output(
            mod_api::CommandFeedback::Failure,
            message.with_color(text::RED),
        );
    }
    /// `createGameTestInfo`: the test of a test instance block.
    pub(in crate::game::server_side) fn test_info_at(
        &mut self,
        pos: Pos,
        retry: Retry,
    ) -> Option<Info> {
        let key = self.test_instance_at(pos).and_then(|b| b.data.test.clone());
        if self.test_instance_at(pos).is_none() {
            self.test_failure(Text::translate(
                "commands.test.error.test_instance_not_found.position",
                &[pos.0, pos.1, pos.2].map(|v| Text::literal(v.to_string())),
            ));
            return None;
        }
        let Some(def) = key.as_deref().and_then(|k| self.test_def(k)) else {
            let name = self.test_name(pos);
            self.test_failure(Text::translate(
                "commands.test.error.non_existant_test",
                &[name],
            ));
            return None;
        };
        let key = key.unwrap_or_default();
        let key = if key.contains(':') {
            key
        } else {
            format!("minecraft:{key}")
        };
        if !self.verify_test_structure(&def.structure) {
            return None;
        }
        let rotation = self.test_rotation(pos);
        let id = self.server.game_tests.0.next_id();
        Some(Info::new(id, key, def, Some(pos), self.common.dim, rotation).with_retry(retry))
    }
    /// `toGameTestInfo`: a test without a block, in the level in place.
    pub(in crate::game::server_side) fn test_info_for(
        &mut self,
        key: String,
        def: TestDef,
        steps: i32,
        retry: Retry,
    ) -> R<Option<Info>> {
        if !self.verify_test_structure(&def.structure) {
            return Ok(None);
        }
        // `server.getLevel(dimension)`: the server's levels.
        let dim = match def.dimension.as_str() {
            "minecraft:overworld" | "minecraft:the_nether" | "minecraft:the_end" => {
                Some(Dim::from_id(&def.dimension))
            }
            _ => None,
        }
        .filter(|d| *d == self.common.dim || !self.common.options.execution_role.is_client());
        let Some(dim) = dim else {
            self.test_failure(Text::literal(format!(
                "Could not resolve level for dimension: {}",
                def.dimension
            )));
            return Ok(None);
        };
        let rotation = rotation_for_steps(steps).ok_or_else(|| err("command.failed", &[]))?;
        let id = self.server.game_tests.0.next_id();
        Ok(Some(
            Info::new(id, key, def, None, dim, rotation).with_retry(retry),
        ))
    }
    /// `TestCommand.verify`.
    pub(in crate::game) fn test_verify(&mut self, ctx: &Context) -> R<i32> {
        self.clear_game_tests();
        let tests = match ctx.get("tests") {
            Some(Arg::Str(pattern)) => sunrise_command::args::select_resources(
                self.common.ui.registries.ids("minecraft:test_instance"),
                pattern,
            )
            .into_iter()
            .cloned()
            .collect(),
            _ => Vec::new(),
        };
        let finder = Finder::Selection { tests, copies: 1 };
        let mut infos = Vec::new();
        for pos in self.finder_positions(&finder) {
            infos.extend(self.test_info_at(pos, Retry::none()));
        }
        for (key, def) in self.finder_tests(&finder) {
            infos.extend(self.test_info_for(key, def, 0, Retry::none())?);
        }
        self.server.game_tests.0.failed.clear();
        let batches = verify_batches(&infos, || self.server.game_tests.0.next_id());
        let grid = Grid::new(self.test_around(), 10).clearing();
        self.track_and_start_batches(batches, RunnerOptions::verify(grid));
        Ok(1)
    }
}
