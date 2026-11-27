//! The game-test runner: `GameTestInfo`, `GameTestRunner`, `GameTestTicker`,
//! `MultipleTestTracker`, `ReportGameListener`, `TestCommand.trackAndStartRunner`
//! and the two test instance types (`BlockBasedTestInstance` and
//! `FunctionGameTestInstance`), over the test instance blocks in a level.
//!
//! Retries (`RetryOptions`, `/test run .. <numberOfTimes> <untilFailed>`) and
//! flaky tests (`max_attempts`, `required_successes`) go through
//! `ReportGameListener`'s counters and `GameTestRunner.rerunTest`.
//!
//! Not ported: test sequences (`GameTestSequence`: the built-in tests, only
//! `always_pass`, do not use them), test environments other than the empty
//! `all_of`, and removing the entities of a finished test.
use super::{
    Dim,
    commands::{Target, execute::Source},
    structure_blocks::template_size,
    test_entities::{aabb_of, inflate},
    test_environments::Activation,
    test_instances::TestDef,
};
use std::time::Instant;
use sunrise_core::bounding_box::BoundingBox;
use sunrise_game_data::text::{self, Text};
pub(in crate::game) use sunrise_gameplay::game_tests::{Retry, TestError, number};
use sunrise_level::{
    containers::BlockEntity,
    test_block::TestBlockMode,
    test_instance_block::{ErrorMarker, Status, TestInstanceBlock},
};
use sunrise_sim::lan::data::component_nbt;
use sunrise_world::{
    level::flags,
    worldgen::structure::{Mirror, Rotation, StructureTemplate},
};

type Pos = (i32, i32, i32);

/// What `runAtTickTime` runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// `BlockBasedTestInstance.run`'s `onEachTick` body.
    BlockBasedTick,
}

/// `GameTestInfo`.
pub struct Info {
    pub id: u32,
    /// `id()`: the test's registry id.
    pub key: String,
    def: TestDef,
    /// The test instance block (`getTestBlockPos`), once there is one.
    pub pos: Pos,
    pub has_pos: bool,
    pub dim: Dim,
    pub extra_rotation: Rotation,
    pub tick_count: i32,
    started: bool,
    placed: bool,
    pub done: bool,
    pub error: Option<TestError>,
    pub run_at: Vec<(i64, Action)>,
    started_at: Option<Instant>,
    run_ms: u128,
    pub retry: Retry,
    /// The test whose `ReportGameListener` counts this run: its own id, or
    /// the original's for a rerun (`copyReset`).
    pub origin: u32,
    pub rerun: bool,
    /// `succeed` removes the entities around the structure; the game does it.
    discard_entities: bool,
}

impl Info {
    pub fn new(
        id: u32,
        key: String,
        def: TestDef,
        pos: Option<Pos>,
        dim: Dim,
        extra_rotation: Rotation,
    ) -> Self {
        Self {
            id,
            key,
            def,
            pos: pos.unwrap_or_default(),
            has_pos: pos.is_some(),
            dim,
            extra_rotation,
            tick_count: 0,
            started: false,
            placed: false,
            done: false,
            error: None,
            run_at: Vec::new(),
            started_at: None,
            run_ms: 0,
            retry: Retry::none(),
            origin: id,
            rerun: false,
            discard_entities: false,
        }
    }

    pub fn with_retry(mut self, retry: Retry) -> Self {
        self.retry = retry;
        self
    }

    /// `copyReset`: the same test, level and block, as a new run.
    pub fn copy_reset(&self, id: u32) -> Self {
        let pos = self.has_pos.then_some(self.pos);
        let mut copy = Self::new(
            id,
            self.key.clone(),
            self.def.clone(),
            pos,
            self.dim,
            self.extra_rotation,
        )
        .with_retry(self.retry);
        copy.origin = self.origin;
        copy.rerun = true;
        copy
    }

    /// `getRotation`.
    fn rotation(&self) -> Rotation {
        self.def.rotation.get_rotated(self.extra_rotation)
    }

    /// `assertionException(description)`.
    fn assertion(&self, message: Text) -> TestError {
        TestError::Assert {
            message,
            tick: self.tick_count,
        }
    }

    /// `fail(Component)`.
    pub fn fail(&mut self, message: Text) {
        self.error = Some(self.assertion(message));
    }

    /// `finish`.
    pub fn finish(&mut self) {
        if !self.done {
            self.done = true;
            if let Some(at) = self.started_at.take() {
                self.run_ms = at.elapsed().as_millis();
            }
        }
    }

    /// `succeed`: finishes unless a failure is set.
    pub fn succeed(&mut self) {
        if self.error.is_none() {
            self.finish();
            self.discard_entities = true;
        }
    }

    /// `startExecution(tickDelay)`.
    pub fn start_execution(&mut self, tick_delay: i32) {
        self.tick_count = -(self.def.setup_ticks + tick_delay + 1);
    }

    /// `onEachTick`: the action on every tick from now to the timeout.
    pub fn on_each_tick(&mut self, action: Action) {
        for i in i64::from(self.tick_count)..i64::from(self.def.max_ticks) {
            self.run_at.push((i, action));
        }
    }
}

/// One `GameTestBatch`.
pub struct Batch {
    pub index: i32,
    pub(in crate::game) environment: String,
    inline_environment: Option<super::test_environments::Environment>,
    dimension: Dim,
    pub infos: Vec<Info>,
}

/// What `GameTestRunner.Builder` is given.
pub struct RunnerOptions {
    halt_on_error: bool,
    batch_size: usize,
    clear_between_batches: bool,
    grid_for_existing: bool,
    grid: Option<Grid>,
    /// `trackAndStartRunner`'s source is the command's own, not the
    /// server's (`TestInstanceBlockEntity.runTest`).
    from_command: bool,
    pub summary_source: Option<SummarySource>,
}

impl RunnerOptions {
    pub(in crate::game::server_side) fn new(
        halt_on_error: bool,
        grid: Option<Grid>,
        from_command: bool,
    ) -> Self {
        Self {
            halt_on_error,
            batch_size: 50,
            clear_between_batches: false,
            grid_for_existing: false,
            grid,
            from_command,
            summary_source: None,
        }
    }

    /// `/test verify`: `batcher(fromGameTestInfo(100))`, the grid for
    /// existing tests too, `haltOnError`, `clearBetweenBatches`.
    pub fn verify(grid: Grid) -> Self {
        Self {
            halt_on_error: true,
            batch_size: 100,
            clear_between_batches: true,
            grid_for_existing: true,
            grid: Some(grid),
            from_command: true,
            summary_source: None,
        }
    }
}

/// What `MultipleTestTracker` knows of a test.
pub struct Tracked {
    pub id: u32,
    pub required: bool,
    pub done: bool,
    pub failed: bool,
}

/// `GameTestBatchFactory.fromGameTestInfo(size)`: partition each environment
/// and definition dimension into batches of up to `size`.
fn batch_infos(infos: Vec<Info>, size: usize) -> Vec<Batch> {
    let mut batches: Vec<Batch> = Vec::new();
    for info in infos {
        let index = match batches.iter_mut().rev().find(|batch| {
            batch.environment == info.def.environment
                && batch.inline_environment == info.def.inline_environment
                && batch.dimension == info.dim
                && batch.infos[0].def.dimension == info.def.dimension
        }) {
            Some(batch) if batch.infos.len() < size => {
                batch.infos.push(info);
                continue;
            }
            Some(full) => full.index + 1,
            None => 0,
        };
        batches.push(Batch {
            index,
            environment: info.def.environment.clone(),
            inline_environment: info.def.inline_environment.clone(),
            dimension: info.dim,
            infos: vec![info],
        });
    }
    batches
}

/// `/test verify`'s batches: each test in each rotation 100 times, with
/// `RetryOptions(1, true)`, at the test's block if it has one; the batch
/// index is the rotation's ordinal.
pub fn verify_batches(infos: &[Info], mut next_id: impl FnMut() -> u32) -> Vec<Batch> {
    let mut batches = Vec::new();
    for info in infos {
        for (ordinal, rotation) in Rotation::ALL.into_iter().enumerate() {
            let copies = (0..100)
                .map(|_| {
                    let pos = info.has_pos.then_some(info.pos);
                    Info::new(
                        next_id(),
                        info.key.clone(),
                        info.def.clone(),
                        pos,
                        info.dim,
                        rotation,
                    )
                    .with_retry(Retry::new(1, true))
                })
                .collect();
            batches.push(Batch {
                index: ordinal as i32,
                environment: info.def.environment.clone(),
                inline_environment: info.def.inline_environment.clone(),
                dimension: info.dim,
                infos: copies,
            });
        }
    }
    batches
}

/// A listener's counters, (origin, attempts, successes): its attempts and successes.
pub fn report_of(reports: &mut Vec<(u32, i32, i32)>, origin: u32) -> (&mut i32, &mut i32) {
    let at = reports
        .iter()
        .position(|r| r.0 == origin)
        .unwrap_or_else(|| {
            reports.push((origin, 0, 0));
            reports.len() - 1
        });
    let r = &mut reports[at];
    (&mut r.1, &mut r.2)
}

/// `GameTestRunner`.
pub struct Runner {
    pub batches: Vec<Batch>,
    pub current: usize,
    /// `currentBatchTracker`: the tests of the batch still to finish.
    pub batch_pending: usize,
    pub tracked: Vec<Tracked>,
    pub halt_on_error: bool,
    /// `GameTestBatchFactory.fromGameTestInfo(maxTestsPerBatch)`, for reruns.
    pub batch_size: usize,
    /// `clearBetweenBatches`.
    pub clear_between_batches: bool,
    /// The grid is also `existingStructureSpawner` (`/test verify`): a test
    /// with a block is moved onto it.
    pub grid_for_existing: bool,
    /// The tests of the batch before: their levels and blocks, for
    /// `clearBetweenBatches`.
    pub last_batch: Vec<(Dim, Pos)>,
    /// `newStructureSpawner`: where the tests without a block go.
    pub grid: Option<Grid>,
    /// `ReportGameListener`'s (attempts, successes), by original test.
    pub reports: Vec<(u32, i32, i32)>,
    /// `scheduledForRerun`.
    pub scheduled: Vec<Info>,
    /// `stopped`: no batch is running.
    pub stopped: bool,
    pub summary_source: Option<SummarySource>,
    pub summary_dimensions: Vec<Dim>,
}

/// The command stack and its output source are different: `execute as`
/// changes the actor, while the player who issued the command receives it.
#[derive(Clone)]
pub struct SummarySource {
    pub source: Source,
    pub recipient: Option<Target>,
}

/// `StructureGridSpawner.DimensionGridState`.
pub struct DimGrid {
    first: Pos,
    pub next: Pos,
    /// The z extent of `rowBounds` (an `AABB(BlockPos)` is one block deep).
    pub row_z: (i32, i32),
    count: i32,
    /// `testsInLastBatch`: their blocks.
    last_batch: Vec<Pos>,
}

impl DimGrid {
    pub fn new(first: Pos) -> Self {
        Self {
            first,
            next: first,
            row_z: (first.2, first.2 + 1),
            count: 0,
            last_batch: Vec::new(),
        }
    }
}

/// `StructureGridSpawner`: a grid per dimension, made when the first test of
/// that dimension is placed (`firstTestNorthWestCorner.apply(dimension)`).
pub struct Grid {
    /// The column `createTestPositionAround` starts from: the source's.
    around: (i32, i32),
    per_row: i32,
    /// `clearOnBatch`.
    pub clear_on_batch: bool,
    pub states: Vec<(Dim, DimGrid)>,
}

impl Grid {
    pub fn new(around: (i32, i32), per_row: i32) -> Self {
        Self {
            around,
            per_row,
            clear_on_batch: false,
            states: Vec::new(),
        }
    }

    /// `clearOnBatch = true`.
    pub fn clearing(mut self) -> Self {
        self.clear_on_batch = true;
        self
    }
}

/// What a runner says to the server, sent on the next tick (a test's
/// messages come from the server's console, not a player's command).
pub enum Out {
    /// `sendSuccess(.., broadcast)`, `sendFailure` or `sendSystemMessage` of
    /// the server's command source.
    Console {
        text: Text,
        broadcast: bool,
    },
    /// `ReportGameListener.say`: every player gets it.
    Players(Text),
    Source {
        context: SummarySource,
        kind: SummaryKind,
        text: Text,
    },
    Coordinates(SummarySource, Vec<Dim>),
}

#[derive(Clone, Copy)]
pub enum SummaryKind {
    Success { broadcast: bool },
    Failure,
    System,
}

pub fn summary_line(context: Option<SummarySource>, kind: SummaryKind, text: Text) -> Out {
    match context {
        Some(context) => Out::Source {
            context,
            kind,
            text,
        },
        None => match kind {
            SummaryKind::Success { broadcast } => Out::Console { text, broadcast },
            SummaryKind::Failure => Out::Console {
                text: Text::empty().append(text).with_color(text::RED),
                broadcast: false,
            },
            SummaryKind::System => Out::Console {
                text,
                broadcast: false,
            },
        },
    }
}

/// `ComponentUtils.wrapInSquareBrackets` for the summary's coordinate links.
/// The locate/pos link helpers have their own existing representation.
fn summary_coordinates(key: &str, at: [i32; 3], dim: Dim, rotation: [i32; 2], color: u32) -> Text {
    use sunrise_game_data::text::{ClickEvent, Style};
    let mut args: Vec<_> = at.into_iter().map(number).collect();
    args.push(Text::literal(dim.id().split_once(':').unwrap().1));
    Text::translate("chat.square_brackets", &[Text::translate(key, &args)]).with_style(&Style {
        click: Some(ClickEvent::SuggestCommand(format!(
            "/execute in {} run tp @s {} {} {} {} {}",
            dim.id(),
            at[0],
            at[1],
            at[2],
            rotation[0],
            rotation[1]
        ))),
        hover: Some(std::sync::Arc::new(Text::translate(
            "chat.coordinates.tooltip",
            &[],
        ))),
        ..Style::color(color)
    })
}

/// `GameTestTicker.SINGLETON` and the runner it holds, `FailedTestTracker`.
#[derive(Default)]
pub struct GameTests {
    pub ticker: Vec<Info>,
    pub runner: Option<Runner>,
    ticking: bool,
    halting: bool,
    next_id: u32,
    /// `FailedTestTracker.LAST_FAILED_TESTS`.
    pub failed: Vec<String>,
    pub outbox: Vec<Out>,
    /// The command that starts the runner is still running: its source takes
    /// `TestBatchSummaryDisplayer`'s lines then and there.
    command_start: bool,
    /// `GameTestRunner.currentEnvironment`.
    environment: Option<(Dim, Activation)>,
}

impl GameTests {
    /// A test of the runner (placed or still to place) is in `dim`.
    pub fn has_tests_in(&self, dim: Dim) -> bool {
        self.ticker.iter().any(|i| i.dim == dim)
            || self.runner.as_ref().is_some_and(|r| {
                r.batches
                    .iter()
                    .flat_map(|b| &b.infos)
                    .chain(&r.scheduled)
                    .any(|i| i.dim == dim)
            })
    }

    pub(in crate::game::server_side) fn next_id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }
}

impl crate::game::ServerGame<'_> {
    /// `TestCommand.trackAndStartRunner`: command summaries retain their
    /// original source. Test-block actions and verify keep their existing path.
    pub(in crate::game::server_side) fn track_and_start_command_runner(
        &mut self,
        infos: Vec<Info>,
        grid: Grid,
    ) {
        let mut source = self
            .server
            .command_source
            .clone()
            .unwrap_or_else(|| self.player_source());
        let recipient = if self.server.console_broadcast.is_some() || source.native_output.is_some()
        {
            None
        } else {
            self.server
                .lan_acting
                .map(Target::Guest)
                .or_else(|| self.common.player.as_ref().map(|_| Target::Player))
        };
        if source.entity == Some(Target::Player)
            && let Some(conn) = self.server.lan_acting
        {
            source.entity = Some(Target::Guest(conn));
        }
        let mut options = RunnerOptions::new(false, Some(grid), true);
        // Block/cart output destinations retain the previous path; they are
        // not player/console command-response recipients in this slice.
        options.summary_source = (source.block.is_none() && source.cart.is_none())
            .then_some(SummarySource { source, recipient });
        self.track_and_start_batches(batch_infos(infos, 50), options);
    }
    // ---- GameTestRunner / GameTestTicker ----

    /// `GameTestTicker.clear`: a pass in progress halts at its end.
    pub(in crate::game) fn clear_game_tests(&mut self) {
        if self.server.game_tests.0.ticking {
            self.server.game_tests.0.halting = true;
        } else {
            self.server.game_tests.0.ticker.clear();
            if self.server.game_tests.0.runner.take().is_some() {
                // GameTestRunner.stop: endCurrentEnvironment.
                self.end_test_environment();
            }
        }
    }

    /// `TestCommand.trackAndStartRunner` for tests already built (batched by
    /// environment, 50 to a batch): the tracker and its listeners, then
    /// `GameTestRunner.start`.
    pub fn track_and_start_runner(
        &mut self,
        infos: Vec<Info>,
        halt_on_error: bool,
        grid: Option<Grid>,
        from_command: bool,
    ) {
        let batches = batch_infos(infos, 50);
        self.track_and_start_batches(
            batches,
            RunnerOptions::new(halt_on_error, grid, from_command),
        );
    }

    /// `TestCommand.trackAndStartRunner` for batches already made.
    pub(in crate::game::server_side) fn track_and_start_batches(
        &mut self,
        batches: Vec<Batch>,
        options: RunnerOptions,
    ) {
        let tracked = batches
            .iter()
            .flat_map(|b| &b.infos)
            .map(|i| Tracked {
                id: i.id,
                required: i.def.required,
                done: false,
                failed: false,
            })
            .collect();
        self.server.game_tests.0.runner = Some(Runner {
            batches,
            current: 0,
            batch_pending: 0,
            tracked,
            halt_on_error: options.halt_on_error,
            batch_size: options.batch_size,
            clear_between_batches: options.clear_between_batches,
            grid_for_existing: options.grid_for_existing,
            last_batch: Vec::new(),
            grid: options.grid,
            reports: Vec::new(),
            scheduled: Vec::new(),
            stopped: false,
            summary_source: options.summary_source,
            summary_dimensions: Vec::new(),
        });
        // GameTestRunner.start runs the first batch inside the command.
        self.server.game_tests.0.command_start = options.from_command;
        self.run_batch(0);
        self.server.game_tests.0.command_start = false;
    }

    /// `GameTestRunner.runBatch`.
    fn run_batch(&mut self, index: usize) {
        let Some(runner) = self.server.game_tests.0.runner.as_mut() else {
            return;
        };
        if index >= runner.batches.len() {
            self.end_test_environment();
            self.run_scheduled_reruns();
            return;
        }
        runner.current = index;
        // clearBetweenBatches: the batch before is cleared and its blocks
        // destroyed.
        let cleared = if index > 0 && runner.clear_between_batches {
            std::mem::take(&mut runner.last_batch)
        } else {
            Vec::new()
        };
        // StructureGridSpawner.onBatchStart (clearOnBatch), every dimension.
        let mut grid_cleared: Vec<(Dim, Pos)> = Vec::new();
        if let Some(grid) = runner.grid.as_mut().filter(|g| g.clear_on_batch) {
            for (dim, g) in &mut grid.states {
                grid_cleared.extend(
                    std::mem::take(&mut g.last_batch)
                        .into_iter()
                        .map(|p| (*dim, p)),
                );
                g.next = g.first;
                g.row_z = (g.first.2, g.first.2 + 1);
                g.count = 0;
            }
        }
        for (dim, pos) in cleared {
            self.in_test_level(dim, |g| {
                if let Some(area) = g.test_bounding_box(pos) {
                    g.clear_test_space(&area);
                }
                g.set_test_block_state(pos, "minecraft:air", flags::UPDATE_ALL);
            });
        }
        for (dim, pos) in grid_cleared {
            self.in_test_level(dim, |g| {
                if let Some(area) = g.test_bounding_box(pos) {
                    g.clear_test_space(&area);
                }
            });
        }
        let Some(runner) = self.server.game_tests.0.runner.as_mut() else {
            return;
        };
        let Some(batch) = runner.batches.get_mut(index) else {
            return;
        };
        let mut infos = std::mem::take(&mut batch.infos);
        let (environment, inline_environment, batch_number, dimension) = (
            batch.environment.clone(),
            batch.inline_environment.clone(),
            batch.index,
            batch.dimension,
        );
        // createStructuresForBatch: `StructureSpawner.IN_PLACE`.
        let mut spawned = Vec::new();
        for mut info in infos.drain(..) {
            if self.spawn_test(&mut info) {
                spawned.push(info);
            }
        }
        log::info!(
            "Running test environment '{environment}' batch {batch_number} ({} tests)...",
            spawned.len()
        );
        // endCurrentEnvironment, then TestEnvironmentDefinition.activate.
        self.end_test_environment();
        match inline_environment.or_else(|| self.test_environment(&environment)) {
            Some(definition) => {
                let activation = self.activate_test_environment(definition, dimension);
                self.server.game_tests.0.environment = Some((dimension, activation));
            }
            None => log::error!("Missing test environment {environment}"),
        }
        // TestBatchSummaryDisplayer.testBatchStarting: `sendSuccess(.., true)`
        // to the source that started the runner.
        let starting = Text::translate(
            "commands.test.batch.starting",
            &[Text::literal(environment), number(batch_number)],
        );
        if self.server.game_tests.0.command_start {
            self.feedback(starting);
        } else {
            let context = self
                .server
                .game_tests
                .0
                .runner
                .as_ref()
                .and_then(|r| r.summary_source.clone());
            self.server.game_tests.0.outbox.push(summary_line(
                context,
                SummaryKind::Success { broadcast: true },
                starting,
            ));
        }
        if let Some(runner) = self.server.game_tests.0.runner.as_mut() {
            runner.batch_pending = spawned.len();
            runner.last_batch = spawned.iter().map(|i| (i.dim, i.pos)).collect();
        }
        self.server.game_tests.0.ticker.extend(spawned);
    }

    /// `GameTestRunner.endCurrentEnvironment`.
    fn end_test_environment(&mut self) {
        if let Some((dimension, activation)) = self.server.game_tests.0.environment.take() {
            self.tear_down_test_environment(activation, dimension);
        }
    }

    /// `GameTestRunner.runScheduledRerunTests`: the reruns are batched
    /// again and run, or the runner stops.
    fn run_scheduled_reruns(&mut self) {
        let Some(runner) = self.server.game_tests.0.runner.as_mut() else {
            return;
        };
        if runner.scheduled.is_empty() {
            runner.batches = Vec::new();
            runner.stopped = true;
            runner.summary_source = None;
            return;
        }
        let scheduled = std::mem::take(&mut runner.scheduled);
        log::info!(
            "Starting re-run of tests: {}",
            scheduled
                .iter()
                .map(|i| i.key.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        runner.batches = batch_infos(scheduled, runner.batch_size);
        runner.stopped = false;
        self.run_batch(0);
    }

    /// `GameTestRunner.rerunTest`.
    fn rerun_test(&mut self, info: &Info) {
        let id = self.server.game_tests.0.next_id();
        let copy = info.copy_reset(id);
        let Some(runner) = self.server.game_tests.0.runner.as_mut() else {
            return;
        };
        runner.scheduled.push(copy);
        if runner.stopped {
            self.run_scheduled_reruns();
        }
    }

    /// `GameTestServer`'s per-tick call: `MinecraftServer.tickServer`'s
    /// `GameTestTicker.SINGLETON.tick()`, for the tests of the level in place.
    pub fn tick_game_tests(&mut self) {
        self.flush_game_test_output();
        if self.server.game_tests.0.runner.is_none() {
            return;
        }
        let dim = self.common.dim;
        self.server.game_tests.0.ticking = true;
        let mut infos = std::mem::take(&mut self.server.game_tests.0.ticker);
        for info in infos.iter_mut().filter(|i| i.dim == dim) {
            self.tick_test(info);
        }
        infos.retain(|i| !i.done);
        infos.append(&mut self.server.game_tests.0.ticker);
        self.server.game_tests.0.ticker = infos;
        self.server.game_tests.0.ticking = false;
        if std::mem::take(&mut self.server.game_tests.0.halting) {
            self.clear_game_tests();
        }
    }

    // ---- GameTestInfo ----

    /// `GameTestRunner.spawn`: a test with a block is prepared where it is
    /// (`StructureSpawner.IN_PLACE`), one without goes to the next place in
    /// the grid (`StructureGridSpawner.spawnStructure`), or is dropped
    /// (`NOT_SET`).
    fn spawn_test(&mut self, info: &mut Info) -> bool {
        let dim = info.dim;
        // The test's own level is in place while it is prepared.
        match self.in_test_level(dim, |g| g.spawn_test_here(info)) {
            Some(spawned) => spawned,
            None => {
                log::error!("Could not resolve level for dimension: {}", dim.id());
                false
            }
        }
    }

    /// `spawn_test` with the test's level in place.
    fn spawn_test_here(&mut self, info: &mut Info) -> bool {
        let in_place = !self
            .server
            .game_tests
            .0
            .runner
            .as_ref()
            .is_some_and(|r| r.grid_for_existing);
        if info.has_pos && in_place {
            if !self.prepare_test_structure(info) {
                return false;
            }
            info.start_execution(1);
            self.test_structure_loaded(info);
            return true;
        }
        let Some(mut grid) = self
            .server
            .game_tests
            .0
            .runner
            .as_mut()
            .and_then(|runner| runner.grid.take())
        else {
            return false;
        };
        // `gridFor`: the dimension's grid starts where the source stands.
        let at = match grid.states.iter().position(|(d, _)| *d == info.dim) {
            Some(at) => at,
            None => {
                let first = self.test_position_in_level(grid.around);
                grid.states.push((info.dim, DimGrid::new(first)));
                grid.states.len() - 1
            }
        };
        let per_row = grid.per_row;
        let state = &mut grid.states[at].1;
        info.pos = state.next;
        info.has_pos = true;
        let prepared = self.prepare_test_structure(info);
        if prepared {
            state.last_batch.push(info.pos);
            info.start_execution(1);
            self.test_structure_loaded(info);
            if let Some(b) = self.test_bounding_box(info.pos) {
                // `getTestBounds()`: the structure's box and its padding.
                let width = b.max[0] - b.min[0] + 1;
                state.row_z = (state.row_z.0.min(b.min[2]), state.row_z.1.max(b.max[2] + 1));
                state.next.0 += width + 5;
                state.count += 1;
                if state.count >= per_row {
                    state.count = 0;
                    state.next.2 += state.row_z.1 - state.row_z.0 + 6;
                    state.next.0 = state.first.0;
                    state.row_z = (state.next.2, state.next.2 + 1);
                }
            }
        }
        if let Some(runner) = self.server.game_tests.0.runner.as_mut() {
            runner.grid = Some(grid);
        }
        prepared
    }

    /// Run `f` with `dim`'s level in place (`enter_level`, as a command
    /// whose source is there), then put the level back. None when the level
    /// cannot be entered.
    pub(in crate::game::server_side) fn in_test_level<R>(
        &mut self,
        dim: Dim,
        f: impl FnOnce(&mut Self) -> R,
    ) -> Option<R> {
        if dim == self.common.dim {
            return Some(f(self));
        }
        let back = self.enter_command_level(dim)?;
        let result = f(self);
        if let Err(e) = self.enter_level(back) {
            log::error!("could not return to {}: {e}", back.id());
        }
        Some(result)
    }

    /// `level.getChunk(pos)` for the chunks of `area`: forced (the test needs
    /// them kept) and waited for, as vanilla loads them synchronously.
    pub(in crate::game) fn load_test_chunks(&mut self, area: &BoundingBox) {
        let missing = |g: &Self| -> bool {
            (area.min[0] >> 4..=area.max[0] >> 4).any(|x| {
                (area.min[2] >> 4..=area.max[2] >> 4).any(|z| !g.world.chunks.contains_key(&(x, z)))
            })
        };
        if !missing(self) {
            return;
        }
        self.force_load_test_chunks(area);
        let deadline = Instant::now() + std::time::Duration::from_secs(120);
        while missing(self) && Instant::now() < deadline {
            let _ = self.server.streaming.update(&mut self.common.world);
            self.server
                .streaming
                .wait(std::time::Duration::from_millis(5));
        }
    }

    /// `ReportGameListener.testStructureLoaded`.
    fn test_structure_loaded(&mut self, info: &Info) {
        if let Some(runner) = self.server.game_tests.0.runner.as_mut() {
            *report_of(&mut runner.reports, info.origin).0 += 1;
        }
    }

    /// `GameTestInfo.prepareTestStructure`: the test instance block with the
    /// test's data, then `placeStructure`. False when the block is gone.
    fn prepare_test_structure(&mut self, info: &mut Info) -> bool {
        let pos = info.pos;
        self.load_test_chunks(&BoundingBox::new(
            [pos.0, pos.1, pos.2],
            [pos.0, pos.1, pos.2],
        ));
        self.set_test_block_state(pos, "minecraft:test_instance_block", flags::UPDATE_ALL);
        if self.test_instance_at(pos).is_none() {
            self.common.world.block_entities.insert(
                pos,
                BlockEntity::TestInstanceBlock(Box::<TestInstanceBlock>::default()),
            );
        }
        let size = self
            .template_entry(&info.def.structure)
            .map_or([1, 1, 1], |entry| template_size(&entry.tag));
        if let Some(block) = self.test_instance_mut(pos) {
            block.data.test = Some(info.key.clone());
            block.data.size = size;
            block.data.rotation = info.extra_rotation;
            block.data.ignore_entities = false;
            block.data.status = Status::Cleared;
            block.data.error_message = None;
        }
        self.mark_block_entity_updated(pos);
        // placeStructure
        if let Some(area) = self.test_bounding_box(pos) {
            self.load_test_chunks(&area);
        }
        if !self.place_test_structure(pos) {
            let name = self.test_name(pos).plain();
            info.fail(Text::translate(
                "test.error.structure.failure",
                &[Text::literal(name)],
            ));
        }
        info.placed = true;
        self.encase_test_structure(pos);
        if let Some(bounds) = self.test_bounding_box(pos) {
            self.clear_test_ticks(&bounds);
        }
        true
    }

    /// `getTestBoundingBox`: the structure's box and its padding.
    pub(in crate::game) fn test_bounding_box(&self, pos: Pos) -> Option<BoundingBox> {
        let padding = self.test_holder_padding(pos);
        let inner = self.test_structure_box(pos)?;
        Some(BoundingBox::new(
            inner.min.map(|v| v - padding),
            inner.max.map(|v| v + padding),
        ))
    }

    /// `encaseStructure`: barriers around the box, but for the block itself.
    fn encase_test_structure(&mut self, pos: Pos) {
        let block = self
            .common
            .pack
            .assets
            .registry
            .default_state("minecraft:test_instance_block");
        for at in self.structure_boundary(pos) {
            if Some(self.common.world.block(at.0, at.1, at.2)) != block {
                self.set_test_block_state(at, "minecraft:barrier", flags::UPDATE_ALL);
            }
        }
    }

    /// `GameTestInfo.tick`.
    fn tick_test(&mut self, info: &mut Info) {
        if info.done {
            return;
        }
        if !info.placed {
            info.fail(Text::translate("test.error.ticking_without_structure", &[]));
        }
        let has_block = self.test_instance_at(info.pos).is_some();
        if !has_block {
            info.fail(Text::translate("test.error.missing_block_entity", &[]));
        }
        if info.error.is_some() {
            info.finish();
        } else if !self.test_chunks_loaded(info.pos) {
            return;
        }
        self.tick_internal(info);
        if std::mem::take(&mut info.discard_entities)
            && let Some(b) = self.test_structure_box(info.pos)
        {
            // `GameTestInfo.succeed`: `getStructureBounds().inflate(1.0)`.
            self.discard_entities_in(inflate(aabb_of(&b), 1.0));
        }
        if info.done {
            self.test_finished(info);
        }
    }

    /// The chunks of the structure are loaded (`areEntitiesActuallyLoadedAndTicking`).
    fn test_chunks_loaded(&self, pos: Pos) -> bool {
        let Some(b) = self.test_structure_box(pos) else {
            return true;
        };
        (b.min[0] >> 4..=b.max[0] >> 4).all(|cx| {
            (b.min[2] >> 4..=b.max[2] >> 4)
                .all(|cz| self.common.world.chunks.contains_key(&(cx, cz)))
        })
    }

    /// `GameTestInfo.tickInternal`.
    fn tick_internal(&mut self, info: &mut Info) {
        info.tick_count += 1;
        if info.tick_count < 0 {
            return;
        }
        if !info.started {
            self.start_test(info);
        }
        let now = i64::from(info.tick_count);
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut info.run_at)
            .into_iter()
            .partition(|(time, _)| *time <= now);
        info.run_at = later;
        for (_, action) in due {
            if let Err(error) = self.run_action(info, action) {
                info.error = Some(error);
            }
        }
        if info.tick_count > info.def.max_ticks {
            // No sequences: the test had no result in time.
            info.error = Some(TestError::Timeout(Text::translate(
                "test.error.timeout.no_result",
                &[number(info.def.max_ticks)],
            )));
        }
    }

    /// `GameTestInfo.startTest`.
    pub fn start_test(&mut self, info: &mut Info) {
        info.started = true;
        info.started_at = Some(Instant::now());
        self.set_test_status(info.pos, Status::Running, None);
        if let Err(error) = self.run_instance(info) {
            info.error = Some(error);
        }
    }

    /// `GameTestInstance.run(helper)`.
    fn run_instance(&mut self, info: &mut Info) -> Result<(), TestError> {
        if info.def.block_based {
            return self.run_block_based(info);
        }
        if let Some(callback) = info.def.callback.clone() {
            let context = mod_api::GameTestContext::new(
                self.level(self.common.dim.id()).expect("test server level"),
                self.test_origin(info),
                match info.rotation() {
                    Rotation::None => 0,
                    Rotation::Clockwise90 => 1,
                    Rotation::Clockwise180 => 2,
                    Rotation::Counterclockwise90 => 3,
                },
            );
            // A test exception fails this attempt, as GameTestInfo.startTest does;
            // it must not unwind out of the server tick and stop the remaining tests.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                callback.run(self, &context)
            }))
            .map_err(|payload| {
                TestError::Unknown(super::dedicated::panic_message(payload.as_ref()))
            })?;
            return match result {
                mod_api::GameTestResult::Pass => {
                    info.succeed();
                    Ok(())
                }
                mod_api::GameTestResult::Fail(message) => {
                    Err(info.assertion(Text::literal(message)))
                }
                mod_api::GameTestResult::Pending => Ok(()),
            };
        }
        let function = info.def.function.as_deref().unwrap_or_default();
        match function {
            // ALWAYS_PASS: GameTestHelper::succeed.
            "minecraft:always_pass" => {
                info.succeed();
                Ok(())
            }
            other => Err(TestError::Unknown(format!(
                "Trying to access missing test function: {other}"
            ))),
        }
    }

    fn run_action(&mut self, info: &mut Info, action: Action) -> Result<(), TestError> {
        match action {
            Action::BlockBasedTick => self.block_based_tick(info),
        }
    }

    // ---- GameTestHelper ----

    /// `absolutePos`.
    fn test_absolute_pos(&self, info: &Info, relative: Pos) -> Pos {
        let origin = self.test_origin(info);
        let moved = [
            origin[0] + relative.0,
            origin[1] + relative.1,
            origin[2] + relative.2,
        ];
        let [x, y, z] = StructureTemplate::transform(moved, Mirror::None, info.rotation(), origin);
        (x, y, z)
    }

    /// `getTestOrigin`: the block entity's start corner.
    fn test_origin(&self, info: &Info) -> [i32; 3] {
        let padding = info.def.padding;
        let rotation = self.test_rotation(info.pos);
        self.test_instance_at(info.pos).map_or([0; 3], |b| {
            b.start_corner([info.pos.0, info.pos.1, info.pos.2], padding, rotation)
        })
    }

    /// `assertionException(pos, description)`.
    fn pos_assertion(&self, info: &Info, relative: Pos, message: Text) -> TestError {
        TestError::AssertPos {
            message,
            absolute: self.test_absolute_pos(info, relative),
            relative,
            tick: info.tick_count,
        }
    }

    /// `getBlockEntity(pos, TestBlockEntity.class)`: the absolute position of
    /// the test block.
    fn test_block_entity(&self, info: &Info, relative: Pos) -> Result<Pos, TestError> {
        let absolute = self.test_absolute_pos(info, relative);
        match self.common.world.block_entities.get(&absolute) {
            None => Err(self.pos_assertion(
                info,
                relative,
                Text::translate("test.error.missing_block_entity", &[]),
            )),
            Some(BlockEntity::TestBlock(_)) => Ok(absolute),
            Some(other) => Err(self.pos_assertion(
                info,
                relative,
                Text::translate(
                    "test.error.wrong_block_entity",
                    &[Text::literal(other.id().to_owned())],
                ),
            )),
        }
    }

    /// `forEveryBlockInStructure`: the template-relative positions, x then
    /// y then z.
    fn test_relative_positions(&self, info: &Info) -> Vec<Pos> {
        let Some(b) = self.test_structure_box(info.pos) else {
            return Vec::new();
        };
        let size = [
            b.max[0] - b.min[0] + 1,
            b.max[1] - b.min[1] + 1,
            b.max[2] - b.min[2] + 1,
        ];
        let size = match info.rotation() {
            Rotation::Clockwise90 | Rotation::Counterclockwise90 => [size[2], size[1], size[0]],
            _ => size,
        };
        let mut out = Vec::new();
        for z in 0..size[2] {
            for y in 0..size[1] {
                for x in 0..size[0] {
                    out.push((x, y, z));
                }
            }
        }
        out
    }

    /// `BlockBasedTestInstance.findTestBlocks`.
    fn find_test_blocks(&self, info: &Info, mode: TestBlockMode) -> Vec<Pos> {
        let registry = self.common.pack.assets.registry;
        self.test_relative_positions(info)
            .into_iter()
            .filter(|&relative| {
                let at = self.test_absolute_pos(info, relative);
                registry
                    .state(self.common.world.block(at.0, at.1, at.2))
                    .is_some_and(|s| {
                        &*s.name == "minecraft:test_block"
                            && s.properties.get("mode").map(String::as_str)
                                == Some(mode.serialized_name())
                    })
            })
            .collect()
    }

    // ---- BlockBasedTestInstance ----

    /// `BlockBasedTestInstance.run`.
    fn run_block_based(&mut self, info: &mut Info) -> Result<(), TestError> {
        // findStartBlock
        let starts = self.find_test_blocks(info, TestBlockMode::Start);
        let missing = |game: &Self, info: &Info, key: &str, mode: TestBlockMode| {
            game.info_failure(
                info,
                Text::translate(key, &[Text::translate(&mode.label_key(), &[])]),
            )
        };
        if starts.is_empty() {
            return Err(missing(
                self,
                info,
                "test_block.error.missing",
                TestBlockMode::Start,
            ));
        }
        if starts.len() != 1 {
            return Err(missing(
                self,
                info,
                "test_block.error.too_many",
                TestBlockMode::Start,
            ));
        }
        let start = self.test_block_entity(info, starts[0])?;
        self.trigger_test_block(start);
        info.on_each_tick(Action::BlockBasedTick);
        Ok(())
    }

    /// `helper.fail(message)`: the exception it throws.
    fn info_failure(&self, info: &Info, message: Text) -> TestError {
        info.assertion(message)
    }

    /// The body of `BlockBasedTestInstance.run`'s `onEachTick`.
    fn block_based_tick(&mut self, info: &mut Info) -> Result<(), TestError> {
        let accepts = self.find_test_blocks(info, TestBlockMode::Accept);
        if accepts.is_empty() {
            return Err(self.info_failure(
                info,
                Text::translate(
                    "test_block.error.missing",
                    &[Text::translate(&TestBlockMode::Accept.label_key(), &[])],
                ),
            ));
        }
        let mut accepted = false;
        for relative in accepts {
            let at = self.test_block_entity(info, relative)?;
            accepted |= self.test_block_triggered(at);
        }
        if accepted {
            info.succeed();
            return Ok(());
        }
        // forAllTriggeredTestBlocks(FAIL, fail with the block's message)
        for relative in self.find_test_blocks(info, TestBlockMode::Fail) {
            let at = self.test_block_entity(info, relative)?;
            if self.test_block_triggered(at) {
                let message = self.test_block_message(at);
                return Err(self.info_failure(info, Text::literal(message)));
            }
        }
        // forAllTriggeredTestBlocks(LOG, trigger), each one reset after
        for relative in self.find_test_blocks(info, TestBlockMode::Log) {
            let at = self.test_block_entity(info, relative)?;
            if self.test_block_triggered(at) {
                self.trigger_test_block(at);
                self.reset_test_block(at);
            }
        }
        Ok(())
    }

    // ---- TestInstanceBlockEntity ----

    /// `setRunning`, `setSuccess` and `setErrorMessage`: the status and the
    /// error message it carries (`withStatus` clears it).
    fn set_test_status(&mut self, pos: Pos, status: Status, error: Option<&Text>) {
        let Some(block) = self.test_instance_mut(pos) else {
            return;
        };
        block.data.status = status;
        block.data.error_message = error.map(|t| component_nbt(&t.component()));
        self.mark_block_entity_updated(pos);
    }

    /// `markError`.
    fn mark_test_error(&mut self, pos: Pos, at: Pos, message: &Text) {
        let Some(block) = self.test_instance_mut(pos) else {
            return;
        };
        block.error_markers.push(ErrorMarker {
            pos: [at.0, at.1, at.2],
            text: component_nbt(&message.component()),
        });
        self.mark_block_entity_updated(pos);
    }

    // ---- listeners ----

    /// The listeners of a finished test, in the order they were added:
    /// `ReportGameListener`, the `TestSummaryDisplayer`, the failure tracker
    /// and the runner's batch listener.
    pub fn test_finished(&mut self, info: &Info) {
        let passed = info.error.is_none();
        self.report_game_test(info, passed);
        // TestSummaryDisplayer: only the tests the command started (the
        // listener is not passed on to a rerun's copy).
        let Some(runner) = self.server.game_tests.0.runner.as_mut() else {
            return;
        };
        let tracked = runner.tracked.iter_mut().find(|t| t.id == info.id);
        let counted = tracked.is_some();
        if let Some(t) = tracked {
            t.done = true;
            t.failed = !passed;
            if !runner.summary_dimensions.contains(&info.dim) {
                runner.summary_dimensions.push(info.dim);
            }
        }
        let all_done = counted && runner.tracked.iter().all(|t| t.done);
        let total = runner.tracked.len();
        let failed_required = runner
            .tracked
            .iter()
            .filter(|t| t.failed && t.required)
            .count();
        let failed_optional = runner
            .tracked
            .iter()
            .filter(|t| t.failed && !t.required)
            .count();
        let halt = runner.halt_on_error;
        if all_done {
            let context = runner.summary_source.clone();
            let out = &mut self.server.game_tests.0.outbox;
            out.push(summary_line(
                context.clone(),
                SummaryKind::Success { broadcast: true },
                Text::translate("commands.test.summary", &[number(total as i32)])
                    .with_color(text::WHITE),
            ));
            if failed_required > 0 {
                out.push(summary_line(
                    context.clone(),
                    SummaryKind::Failure,
                    Text::translate(
                        "commands.test.summary.failed",
                        &[number(failed_required as i32)],
                    ),
                ));
            } else {
                out.push(summary_line(
                    context.clone(),
                    SummaryKind::Success { broadcast: true },
                    Text::translate("commands.test.summary.all_required_passed", &[])
                        .with_color(text::GREEN),
                ));
            }
            if failed_optional > 0 {
                out.push(summary_line(
                    context.clone(),
                    SummaryKind::System,
                    Text::translate(
                        "commands.test.summary.optional_failed",
                        &[number(failed_optional as i32)],
                    ),
                ));
            }
            if let Some(context) = context {
                out.push(Out::Coordinates(context, runner.summary_dimensions.clone()));
            }
        }
        // FailedTestTracker.rememberFailedTest
        if !passed && counted {
            self.server.game_tests.0.failed.push(info.key.clone());
        }
        // GameTestRunner's batch listener clears forces in testInfo.getLevel(),
        // the level tick_test runs with in place.
        if passed {
            self.remove_test_barriers(info.pos);
        } else if halt {
            self.server.streaming.clear_forced();
            self.clear_game_tests();
            return;
        }
        let Some(runner) = self.server.game_tests.0.runner.as_mut() else {
            return;
        };
        runner.batch_pending = runner.batch_pending.saturating_sub(1);
        if runner.batch_pending == 0 {
            let next = runner.current + 1;
            self.server.streaming.clear_forced();
            self.run_batch(next);
        }
    }

    /// `ReportGameListener.testPassed` and `testFailed`.
    fn report_game_test(&mut self, info: &Info, passed: bool) {
        let Some(runner) = self.server.game_tests.0.runner.as_mut() else {
            return;
        };
        let (attempts, successes) = report_of(&mut runner.reports, info.origin);
        if passed {
            *successes += 1;
        }
        let (attempts, successes) = (*attempts, *successes);
        let (max_attempts, required) = (info.def.max_attempts, info.def.required_successes);
        let flaky = max_attempts > 1;
        let retry = info.retry;
        let again = if passed {
            if retry.has_retries() {
                let line = retry_line(info, retry, attempts, successes, true);
                self.report_passed(info, &line);
                retry.has_tries_left(attempts, successes)
            } else if !flaky {
                let line = format!(
                    "{} passed! ({}ms / {}gameticks)",
                    info.key, info.run_ms, info.tick_count
                );
                self.report_passed(info, &line);
                false
            } else if successes >= required {
                let line = format!(
                    "{} passed {successes} times of {attempts} attempts.",
                    info.key
                );
                self.report_passed(info, &line);
                false
            } else {
                self.say(
                    text::GREEN,
                    format!(
                        "Flaky test {} succeeded, attempt: {attempts} successes: {successes}",
                        info.key
                    ),
                );
                true
            }
        } else if !flaky {
            self.report_failure(info, None);
            if retry.has_retries() {
                let line = retry_line(info, retry, attempts, successes, false);
                self.say(text::RED, line);
                retry.has_tries_left(attempts, successes)
            } else {
                false
            }
        } else {
            let mut line = format!(
                "Flaky test {} failed, attempt: {attempts}/{max_attempts}",
                info.key
            );
            if required > 1 {
                line.push_str(&format!(", successes: {successes} ({required} required)"));
            }
            self.say(text::YELLOW, line);
            if max_attempts - attempts + successes >= required {
                true
            } else {
                self.report_failure(info, Some((attempts, successes)));
                false
            }
        };
        if again {
            self.rerun_test(info);
        }
    }

    /// `ReportGameListener.reportPassed`.
    fn report_passed(&mut self, info: &Info, line: &str) {
        self.set_test_status(info.pos, Status::Finished, None);
        self.say(text::GREEN, line.to_owned());
    }

    /// `ReportGameListener.reportFailure`: the test's own error, or the
    /// `ExhaustedAttemptsException` of a flaky test that ran out of
    /// attempts (`attempts`, `successes`), whose cause is the test's error.
    fn report_failure(&mut self, info: &Info, exhausted: Option<(i32, i32)>) {
        let Some(error) = &info.error else {
            return;
        };
        let (description, message) = match exhausted {
            None => (error.description(), error.message()),
            Some((attempts, successes)) => {
                let text = format!(
                    "Not enough successes: {successes} out of {attempts} attempts. Required successes: {}. max attempts: {}.",
                    info.def.required_successes, info.def.max_attempts
                );
                // describeError(error) is the cause's message.
                let cause = error.message();
                (
                    Text::literal(cause.clone()),
                    format!("{text} cause: {cause}"),
                )
            }
        };
        self.set_test_status(info.pos, Status::Finished, Some(&description));
        let required = info.def.required;
        let message = format!(
            "{}{} failed! {}",
            if required { "" } else { "(optional) " },
            info.key,
            message
        );
        self.say(if required { text::RED } else { text::YELLOW }, message);
        if let TestError::AssertPos {
            message, absolute, ..
        } = error
        {
            self.mark_test_error(info.pos, *absolute, message);
        }
    }

    /// `ReportGameListener.say`.
    fn say(&mut self, color: u32, text: String) {
        self.server
            .game_tests
            .0
            .outbox
            .push(Out::Players(Text::literal(text).with_color(color)));
    }

    /// Sends what the runner said: the server's console log (and the
    /// operators, when it broadcasts) and the players' chat.
    pub fn flush_game_test_output(&mut self) {
        for out in std::mem::take(&mut self.server.game_tests.0.outbox) {
            match out {
                Out::Console { text, broadcast } => self.test_console_output(text, broadcast),
                Out::Source {
                    context,
                    kind,
                    text,
                } => {
                    self.test_source_output(&context, kind, text);
                }
                Out::Coordinates(context, dimensions) => {
                    if let Some(Target::Guest(conn)) = context.recipient
                        && self.command_guest(conn).is_none()
                    {
                        continue;
                    }
                    self.with_source(context.source.clone(), |g| {
                        g.output_test_summary_coordinates(&context, &dimensions)
                    });
                }
                Out::Players(text) => {
                    if let Some(host) = self.server.lan.as_ref() {
                        host.hub.system_message(component_nbt(&text.component()));
                    }
                }
            }
        }
    }

    fn test_console_output(&mut self, text: Text, broadcast: bool) {
        let plain = text.plain();
        if self.common.options.dedicated.is_some() {
            super::dedicated::log("INFO", &plain);
        } else {
            log::info!("{plain}");
        }
        if broadcast {
            // CommandSourceStack.broadcastToAdmins as the server.
            self.server.console_broadcast = Some(true);
            self.broadcast_to_admins(text);
            self.server.console_broadcast = None;
            self.send_admin_feedback();
            self.server.admin_feedback.clear();
        }
    }

    /// `TestSummaryDisplayer`: feedback uses the original issuer, without
    /// executing another command or invoking the command's result callbacks.
    pub fn test_source_output(&mut self, context: &SummarySource, kind: SummaryKind, text: Text) {
        if let Some(recipient) = context.recipient {
            let delivered = self.for_player(recipient, |g| {
                g.with_source(context.source.clone(), |g| match kind {
                    SummaryKind::Success { broadcast: true } => g.feedback(text),
                    SummaryKind::Success { broadcast: false } => g.feedback_unbroadcast(text),
                    SummaryKind::Failure => g.command_failure(text),
                    SummaryKind::System => g.command_system_message(text),
                });
            });
            if delivered.is_some() {
                self.send_admin_feedback();
            }
        } else if context.source.native_output.is_some() {
            self.with_source(context.source.clone(), |g| match kind {
                SummaryKind::Success { broadcast: true } => {
                    g.feedback(text);
                    g.send_admin_feedback();
                }
                SummaryKind::Success { broadcast: false } => g.feedback_unbroadcast(text),
                SummaryKind::Failure => g.command_failure(text),
                SummaryKind::System => g.command_system_message(text),
            });
        } else if !context.source.silent {
            self.with_source(context.source.clone(), |g| {
                let previous = g.server.console_broadcast;
                let broadcast = matches!(kind, SummaryKind::Success { broadcast: true });
                let text = match kind {
                    SummaryKind::Failure => Text::empty().append(text).with_color(text::RED),
                    _ => text,
                };
                g.test_console_output(text, broadcast);
                g.server.console_broadcast = previous;
            });
        }
    }

    /// `TestCommand.outputPlayerCoordinates` / `outputTestCoordinates` after
    /// the summary. Surface reads reuse the existing test-position helper,
    /// including its missing-chunk policy; this is not a new loading policy.
    fn output_test_summary_coordinates(&mut self, context: &SummarySource, dimensions: &[Dim]) {
        if let Some(Target::Guest(conn)) = context.recipient
            && self.command_guest(conn).is_none()
        {
            return;
        }
        let player = context
            .source
            .entity
            .filter(|target| target.is_player())
            .and_then(|target| {
                self.target_position(target)
                    .map(|pos| (pos, self.target_level(target), self.target_rotation(target)))
            });
        if player.is_some_and(|(_, dim, _)| {
            dimensions.len() <= 1
                && dimensions.contains(&dim)
                && dimensions.contains(&Dim::Overworld)
        }) {
            return;
        }
        if let Some((pos, dim, rotation)) = player {
            self.test_source_output(
                context,
                SummaryKind::Success { broadcast: false },
                summary_coordinates(
                    "test.player.coordinates",
                    pos.map(|v| v.floor() as i32),
                    dim,
                    rotation.map(|v| v.floor() as i32),
                    text::GOLD,
                ),
            );
        }
        let pos = player
            .map_or(context.source.pos, |(pos, _, _)| pos)
            .map(|v| v.floor() as i32);
        for &dim in dimensions {
            let back = self.enter_command_level(dim);
            if self.common.dim != dim {
                continue;
            }
            let y = self.test_position_in_level((pos[0], pos[2])).1;
            if let Some(back) = back {
                self.enter_command_level(back);
            }
            self.test_source_output(
                context,
                SummaryKind::Success { broadcast: false },
                summary_coordinates(
                    "test.run.coordinates",
                    [pos[0], y, pos[2]],
                    dim,
                    [0, 0],
                    text::YELLOW,
                ),
            );
        }
    }
}

/// `ReportGameListener.handleRetry`'s line: the counters, then the test's
/// result.
pub fn retry_line(
    info: &Info,
    retry: Retry,
    attempts: i32,
    successes: i32,
    passed: bool,
) -> String {
    let mut report = format!(
        "[Run: {attempts:>4}, Ok: {successes:>4}, Fail: {:>4}",
        attempts - successes
    );
    if !retry.unlimited() {
        report.push_str(&format!(", Left: {:>4}", retry.number_of_tries - attempts));
    }
    report.push(']');
    let name = format!(
        "{} {}! {}ms",
        info.key,
        if passed { "passed" } else { "failed" },
        info.run_ms
    );
    format!("{report:<53}{name}")
}

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    #[cfg(test)]
    use super::{Batch, Dim, RunnerOptions};

    #[cfg(test)]
    #[test]
    #[ignore = "daily"]
    fn game_test_batch_uses_the_weather_parameter_veto() {
        use mod_api::{Flow, Hooks, ModEntry, ServerHost, WeatherChangeEvent};
        use std::sync::{Arc, Mutex};

        type Proposal = (String, i32, i32, bool, bool);
        struct RefuseThunder(Arc<Mutex<Vec<Proposal>>>);
        #[mod_api::__subscriptions]
        impl Hooks for RefuseThunder {
            fn weather_change(
                &self,
                _: &mut dyn ServerHost,
                event: &mut WeatherChangeEvent<'_>,
            ) -> Flow {
                self.0.lock().unwrap().push((
                    event.dimension.to_owned(),
                    event.clear_ticks,
                    event.weather_ticks,
                    event.raining,
                    event.thundering,
                ));
                if event.thundering {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
        }

        sunrise_gameplay_common::game::in_own_process(
            "game::server_side::game_tests::tests::game_test_batch_uses_the_weather_parameter_veto",
            || {
                let seen = Arc::new(Mutex::new(Vec::new()));
                let mods = vec![ModEntry::new(
                    "test_weather",
                    Box::new(RefuseThunder(seen.clone())),
                )];
                mod_platform::testing::with_mods(mods.into(), || {
                    crate::game::native_entity_refs::tests::with_game(|game| {
                        game.server.world_root = game.options.game_dir.clone().unwrap();
                        game.enter_level(Dim::Nether).unwrap();
                        game.enter_level(Dim::Overworld).unwrap();
                        let pack = game.server.world_root.join("datapacks/fixture");
                        let environments = pack.join("data/example/test_environment");
                        std::fs::create_dir_all(&environments).unwrap();
                        std::fs::write(
                            pack.join("pack.mcmeta"),
                            r#"{"pack":{"description":"weather veto fixture","pack_format":100}}"#,
                        )
                        .unwrap();
                        for weather in ["rain", "thunder"] {
                            std::fs::write(
                                environments.join(format!("{weather}.json")),
                                serde_json::json!({"type": "minecraft:weather", "weather": weather})
                                    .to_string(),
                            )
                            .unwrap();
                        }
                        game.reload_datapacks(false, &[]).unwrap();
                        // Exercise the real runner's environment lifecycle without spawning a structure.
                        let run_batch =
                            |game: &mut crate::game::ServerGame<'_>, dimension, name: &str| {
                                game.track_and_start_batches(
                                    vec![Batch {
                                        index: 0,
                                        environment: format!("example:{name}"),
                                        inline_environment: None,
                                        dimension,
                                        infos: Vec::new(),
                                    }],
                                    RunnerOptions::new(false, None, false),
                                );
                            };
                        let parameters = |game: &crate::game::ServerGame<'_>| {
                            let w = &game.world.weather;
                            (
                                w.clear_time,
                                w.rain_time,
                                w.thunder_time,
                                w.raining,
                                w.thundering,
                            )
                        };
                        game.world.weather.raining = true;
                        game.world.weather.thundering = true;
                        game.world.weather.set_levels(1.0, 1.0);
                        game.server
                            .levels
                            .0
                            .parked_mut(Dim::Nether)
                            .unwrap()
                            .world
                            .weather
                            .set_levels(1.0, 1.0);
                        run_batch(game, Dim::Nether, "rain");
                        assert_eq!(parameters(game), (0, 100_000, 100_000, true, false));
                        game.clear_game_tests();
                        // Neither the shared flags nor the Nether's raw strengths make it wet.
                        assert_eq!(parameters(game), (100_000, 0, 0, false, false));

                        game.enter_level(Dim::Nether).unwrap();
                        // The Overworld is still locally thundering despite the now-clear flags.
                        run_batch(game, Dim::Overworld, "rain");
                        assert_eq!(parameters(game), (0, 100_000, 100_000, true, false));
                        game.world.weather.clear_time = 73;
                        game.clear_game_tests();
                        // resetWeatherCycle still runs, but the saved thunder write is vetoed.
                        assert_eq!(parameters(game), (73, 0, 0, false, false));

                        game.world.weather.rain_time = 9;
                        run_batch(game, Dim::Nether, "thunder");
                        assert_eq!(parameters(game), (73, 9, 0, false, false));
                        game.clear_game_tests();
                        // The vetoed setup must not arrange a teardown that changes the weather.
                        assert_eq!(parameters(game), (73, 9, 0, false, false));
                        let nether = || String::from("minecraft:the_nether");
                        let overworld = || String::from("minecraft:overworld");
                        assert_eq!(
                            *seen.lock().unwrap(),
                            [
                                (nether(), 0, 100_000, true, false),
                                (nether(), 100_000, 0, false, false),
                                (overworld(), 0, 100_000, true, false),
                                (overworld(), 0, 100_000, true, true),
                                (nether(), 0, 100_000, true, true),
                            ]
                        );
                        assert_eq!(game.dim, Dim::Nether);
                    });
                });
            },
        );
    }

    use crate::game::native_entity_refs::tests::load_fixture_server_resources;
    use std::sync::Arc;
    use sunrise_gameplay_common::game::{
        Options, Resources,
        execution_role::ExecutionRole,
        test_fixtures::{fixture_root, offline_profiles},
    };
    use sunrise_level::sim_settings::SimSettings;

    pub fn server_game(resources: &mut Option<Arc<Resources>>) -> crate::game::ServerGame<'_> {
        let root = fixture_root();
        let settings = SimSettings::default();
        let (loaded, pack) = load_fixture_server_resources(&root, ExecutionRole::IntegratedServer);
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root;
        options.execution_role = ExecutionRole::IntegratedServer;
        let res = resources.insert(loaded);
        let mut game =
            crate::game::ServerGame::new(res, pack, options, settings, None, offline_profiles)
                .unwrap();
        game.reset_entity_residency();
        game.install_root_block_transformers();
        game.native_lifecycle
            .begin_open(Some(mod_api::Authority::IntegratedServer));
        game.world
            .chunks
            .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
        game
    }
}
