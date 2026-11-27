//! World epochs for native handles, and lifecycle observations on authoritative servers.
//! A paused pass observes nothing; a frozen world reports `world_running: false` except during a step.

use sunrise_gameplay_common::game::native_lifecycle_common::OpenState;

use super::{
    execution_role::ExecutionRole,
    scheduled_tasks::{ServerTask, dispose_pending_task, report_failure, run_all},
};
use mod_api::{ScheduleError, TaskId};
// Their old home, for mixin code that names them here.
#[allow(unused_imports)]
pub(in crate::game) use mod_api::{
    Authority, LifecycleContext, ServerTickPhase, TickId, WorldEpoch,
};

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn server_lifecycle_open_at(&mut self, epoch: WorldEpoch) {
        self.server.levels.0.active_closed = false;
        self.clear_composter_randoms();
        self.reset_entity_residency();
        self.common.entity_initialization = Default::default();
        self.common.attachment_indexes.clear();
        self.common.creatures.custom_spawners.clear();
        self.common.passages.retire_world();
        self.clear_all_async_chunk_requests();
        self.close_all_saved_chunk_readers();
        self.close_all_detached_generators();
        let authority = match self.common.options.execution_role {
            ExecutionRole::Client => None,
            ExecutionRole::IntegratedServer => Some(Authority::IntegratedServer),
            ExecutionRole::DedicatedServer => Some(Authority::DedicatedServer),
        };
        self.common.native_lifecycle.begin_open_at(authority, epoch);
    }

    pub(in crate::game) fn server_lifecycle_world_ready(&mut self) {
        // The saved scoreboard is there for the server-started hooks, before any tick.
        self.load_scoreboard();
        if let Some(context) = self.common.native_lifecycle.world_open_ready() {
            self.native_server_started(context.authority == Authority::DedicatedServer);
        }
    }

    /// Retire the current world epoch and clear per-world mod state without
    /// synthesizing `LevelEvent.Unload` events (which only fire when
    /// `close_loaded_levels_for_stop` closes each loaded level).
    /// Idempotent and safe on every exit path (orderly close, failed open,
    /// tick failure or panic, save failure, and client disconnect).
    pub(in crate::game) fn server_lifecycle_world_closed(&mut self) {
        self.server.levels.0.clear();
        self.finish_world_closed_common(
            |game| {
                game.clear_all_async_chunk_requests();
                game.close_all_saved_chunk_readers();
                game.close_all_detached_generators();
            },
            |game| game.clear_composter_randoms(),
        );
    }

    pub(in crate::game::server_side) fn drain_due_tasks(
        &mut self,
    ) -> Option<Box<dyn std::any::Any + Send>> {
        run_all(
            self,
            |game| game.native_lifecycle.tasks.pop_due(),
            |game, (owner, task)| {
                use mod_api::__private::game::CallbackOwner;

                let Some(mod_id) = owner else {
                    // Engine-owned work still follows the server's own failure policy.
                    let _: Result<(), std::convert::Infallible> =
                        game.storage_transaction(|game, _| {
                            task(game);
                            Ok(())
                        });
                    return;
                };
                if let Err(panic) =
                    CallbackOwner::call_scheduled_task(mod_id, game, |game| task(game))
                {
                    report_failure(mod_id, &panic);
                    if let Some(serving) = &game.server.serving {
                        serving.reports().report_scheduled_task_failure(mod_id);
                    }
                }
            },
        )
    }

    pub fn native_lifecycle_server_tick_start(&mut self, world_running: bool) {
        if let Some((context, phase)) = self
            .common
            .native_lifecycle
            .server_tick_start(world_running)
        {
            self.native_console_lines();
            self.native_server_tick(&context, phase);
            // Tasks come due after the start phase. Only unattributed engine panics unwind.
            if let Some(payload) = self.drain_due_tasks() {
                std::panic::resume_unwind(payload);
            }
        }
    }

    /// `schedule_task`: a delayed callable attributed to the current mod when called inside one.
    pub fn schedule_task(
        &mut self,
        delay_ticks: u32,
        task: ServerTask,
    ) -> Result<TaskId, ScheduleError> {
        let owner = mod_api::content::crash::current_native_mod();
        if self.common.native_lifecycle.attempt.is_none_or(|attempt| {
            attempt.authority.is_none() || attempt.session == OpenState::Closing
        }) {
            return Err(ScheduleError::Unavailable);
        }
        self.common
            .native_lifecycle
            .tasks
            .schedule(owner, delay_ticks, task)
    }

    /// `cancel_task`: withdraws a waiting task by its opaque `TaskId`, disposing of its captures
    /// under the scheduling owner's panic policy when attributed.
    pub fn cancel_task(&mut self, id: TaskId) -> bool {
        let Some(task) = mod_api::__private::game::CallbackOwner::call_host(|| {
            self.common.native_lifecycle.tasks.cancel(id)
        }) else {
            return false;
        };
        dispose_pending_task(task);
        true
    }

    /// An errored tick returned at `result?` before this site.
    pub fn native_lifecycle_server_tick_end(&mut self, paused: bool) {
        if let Some((context, phase)) = self.common.native_lifecycle.server_tick_end(paused) {
            self.native_server_tick(&context, phase);
            // Drain tasks due this tick, including delay 0 scheduled during the tick body or end hook.
            if let Some(payload) = self.drain_due_tasks() {
                std::panic::resume_unwind(payload);
            }
        }
    }
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycleCommonGameExt1 as _;
