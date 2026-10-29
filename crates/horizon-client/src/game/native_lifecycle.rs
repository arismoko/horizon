//! Read-only lifecycle observations for native mods. A paused pass observes
//! nothing; a frozen world reports `world_running: false`.

use std::sync::atomic::{AtomicU64, Ordering};

use super::{Game, execution_role::ExecutionRole};

/// Epoch allocation spans `Game` instances (a new server `Game` per
/// launch), so the counter is process-wide.
static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);

/// Which `Game` observes the phase; a mirror `Game` never fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NativeAuthority {
    LocalAuthoritative,
    IntegratedServer,
    DedicatedServer,
    RemoteClient,
}

/// One open attempt of one world; it rejects a previous open's commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct NativeWorldEpoch(u64);

/// Non-paused authoritative server ticks since open (not game or frame time).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct NativeTickId(u64);

/// The session phase every event carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SessionPhase {
    Opening,
    Ready,
    Closing,
    Closed,
}

/// The world phases (`native_world_lifecycle`'s `phase`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The phase names are the hook log's stable vocabulary (the Debug text
/// the mixins write); keep them despite the shared prefix.
#[allow(clippy::enum_variant_names)]
pub(super) enum WorldPhase {
    WorldOpenReady,
    /// The open failed; `error_text` is the engine's own error text.
    WorldOpenFailed,
    WorldCloseFinal,
}

/// The tick phases; a frozen world carries `world_running: false`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ServerTickPhase {
    ServerTickStart { world_running: bool },
    ServerTickEnd { world_running: bool },
}

/// The context every lifecycle event carries (provisional).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct NativeLifecycleContext {
    pub(super) authority: NativeAuthority,
    pub(super) epoch: NativeWorldEpoch,
    pub(super) session: SessionPhase,
    pub(super) tick: NativeTickId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OpenAttempt {
    authority: NativeAuthority,
    epoch: u64,
    session: SessionPhase,
    ticks: u64,
    /// A tick started and its end has not fired (or the tick errored).
    tick_open: bool,
}

impl OpenAttempt {
    fn context(&self) -> NativeLifecycleContext {
        NativeLifecycleContext {
            authority: self.authority,
            epoch: NativeWorldEpoch(self.epoch),
            session: self.session,
            tick: NativeTickId(self.ticks),
        }
    }
}

/// The lifecycle bookkeeping: the current attempt's epoch, session phase
/// and non-paused tick count. It only decides whether a phase fires; every
/// hook body is empty, so with no native mods nothing changes.
#[derive(Default)]
pub(super) struct NativeLifecycle {
    attempt: Option<OpenAttempt>,
}

impl NativeLifecycle {
    /// The open starts: a fresh process-wide epoch, never reused.
    pub(super) fn begin_open(&mut self, authority: NativeAuthority) {
        let epoch = NEXT_EPOCH.fetch_add(1, Ordering::Relaxed);
        self.attempt = Some(OpenAttempt {
            authority,
            epoch,
            session: SessionPhase::Opening,
            ticks: 0,
            tick_open: false,
        });
    }

    pub(super) fn world_open_ready(&mut self) -> Option<NativeLifecycleContext> {
        let attempt = self.attempt.as_mut()?;
        if attempt.session != SessionPhase::Opening {
            return None;
        }
        attempt.session = SessionPhase::Ready;
        Some(attempt.context())
    }

    /// `WorldOpenFailed`: only while opening; the attempt is invalidated.
    pub(super) fn world_open_failed(&mut self) -> Option<NativeLifecycleContext> {
        let attempt = self.attempt.as_mut()?;
        if attempt.session != SessionPhase::Opening {
            return None;
        }
        let context = attempt.context();
        self.attempt = None;
        Some(context)
    }

    pub(super) fn begin_close(&mut self) {
        if let Some(attempt) = self.attempt.as_mut() {
            attempt.session = SessionPhase::Closing;
        }
    }

    /// `WorldCloseFinal`: at most once per epoch; the epoch retires.
    pub(super) fn world_close_final(&mut self) -> Option<NativeLifecycleContext> {
        let attempt = self.attempt.as_mut()?;
        if attempt.session != SessionPhase::Closing {
            return None;
        }
        attempt.session = SessionPhase::Closed;
        let context = attempt.context();
        self.attempt = None;
        Some(context)
    }

    /// Non-paused ready ticks only; a paused pass observes nothing, does not count.
    pub(super) fn server_tick_start(
        &mut self,
        paused: bool,
        world_running: bool,
    ) -> Option<(NativeLifecycleContext, ServerTickPhase)> {
        let attempt = self.attempt.as_mut()?;
        if paused || attempt.session != SessionPhase::Ready {
            return None;
        }
        attempt.ticks += 1;
        attempt.tick_open = true;
        Some((
            attempt.context(),
            ServerTickPhase::ServerTickStart { world_running },
        ))
    }

    /// Pairs the last start; an errored tick never reaches it.
    pub(super) fn server_tick_end(
        &mut self,
        paused: bool,
        world_running: bool,
    ) -> Option<(NativeLifecycleContext, ServerTickPhase)> {
        let attempt = self.attempt.as_mut()?;
        if paused || attempt.session != SessionPhase::Ready || !attempt.tick_open {
            return None;
        }
        attempt.tick_open = false;
        Some((
            attempt.context(),
            ServerTickPhase::ServerTickEnd { world_running },
        ))
    }
}

impl Game<'_> {
    /// The role is fixed at construction; a mirror `Game` is remote whatever it is.
    fn native_lifecycle_authority(&self) -> NativeAuthority {
        if self.remote.is_some() {
            return NativeAuthority::RemoteClient;
        }
        match self.options.execution_role {
            ExecutionRole::Client => NativeAuthority::LocalAuthoritative,
            ExecutionRole::IntegratedServer => NativeAuthority::IntegratedServer,
            ExecutionRole::DedicatedServer => NativeAuthority::DedicatedServer,
        }
    }

    pub(super) fn native_lifecycle_open_started(&mut self) {
        self.native_lifecycle
            .begin_open(self.native_lifecycle_authority());
    }

    pub(super) fn native_lifecycle_world_ready(&mut self) {
        if let Some(context) = self.native_lifecycle.world_open_ready() {
            self.native_world_lifecycle(&context, WorldPhase::WorldOpenReady, "");
        }
    }

    /// `error_text` is the engine's own error text, unchanged.
    pub(super) fn native_lifecycle_world_failed(&mut self, error_text: &str) {
        if let Some(context) = self.native_lifecycle.world_open_failed() {
            self.native_world_lifecycle(&context, WorldPhase::WorldOpenFailed, error_text);
        }
    }

    /// After the final save, before the lock release; a failed save fired
    /// nothing. Mirrors never fire.
    pub(super) fn native_lifecycle_world_closed(&mut self) {
        if self.remote.is_some() {
            return;
        }
        if let Some(context) = self.native_lifecycle.world_close_final() {
            self.native_world_lifecycle(&context, WorldPhase::WorldCloseFinal, "");
        }
    }

    pub(super) fn native_lifecycle_server_tick_start(&mut self, paused: bool) {
        let world_running = self.clock.world_runs();
        if let Some((context, phase)) = self
            .native_lifecycle
            .server_tick_start(paused, world_running)
        {
            self.native_server_tick(&context, phase);
        }
    }

    /// An errored tick returned at `result?` before this site.
    pub(super) fn native_lifecycle_server_tick_end(&mut self, paused: bool) {
        let world_running = self.clock.world_runs();
        if let Some((context, phase)) = self.native_lifecycle.server_tick_end(paused, world_running)
        {
            self.native_server_tick(&context, phase);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lifecycle mid-open on the server thread.
    fn opening() -> NativeLifecycle {
        let mut lifecycle = NativeLifecycle::default();
        lifecycle.begin_open(NativeAuthority::IntegratedServer);
        lifecycle
    }

    #[test]
    fn world_ready_fires_once_per_epoch() {
        let mut lifecycle = opening();
        let context = lifecycle.world_open_ready().expect("the first ready fires");
        assert_eq!(context.authority, NativeAuthority::IntegratedServer);
        assert!(context.epoch.0 >= 1, "the epoch is globally allocated");
        assert_eq!(context.session, SessionPhase::Ready);
        assert_eq!(context.tick, NativeTickId(0));
        assert!(lifecycle.world_open_ready().is_none(), "at most once");
    }

    #[test]
    fn a_failed_open_invalidates_and_a_retry_is_fresh() {
        let mut lifecycle = opening();
        let context = lifecycle.world_open_failed().expect("the failure fires");
        let failed_epoch = context.epoch;
        assert_eq!(context.session, SessionPhase::Opening);
        assert!(lifecycle.world_open_ready().is_none(), "the epoch is gone");
        assert!(lifecycle.server_tick_start(false, true).is_none());
        lifecycle.begin_open(NativeAuthority::IntegratedServer);
        let context = lifecycle.world_open_ready().expect("the retry opens fresh");
        assert!(context.epoch.0 > failed_epoch.0, "a fresh epoch");
        assert_eq!(context.tick, NativeTickId(0), "a fresh tick count");
    }

    #[test]
    fn close_final_fires_once_and_retires_the_epoch() {
        let mut lifecycle = opening();
        lifecycle.world_open_ready();
        lifecycle.begin_close();
        let context = lifecycle.world_close_final().expect("the close fires");
        assert!(context.epoch.0 >= 1);
        assert_eq!(context.session, SessionPhase::Closed);
        assert!(
            lifecycle.world_close_final().is_none(),
            "a repeated close request is a no-op"
        );
        assert!(lifecycle.server_tick_start(false, true).is_none());
        assert!(lifecycle.world_open_ready().is_none());
    }

    #[test]
    fn a_close_during_opening_still_closes_once() {
        let mut lifecycle = opening();
        lifecycle.begin_close();
        let context = lifecycle.world_close_final().expect("the close fires");
        assert_eq!(context.session, SessionPhase::Closed);
    }
    #[test]
    fn ticks_count_non_paused_ready_ticks_only() {
        let mut lifecycle = opening();
        assert!(
            lifecycle.server_tick_start(true, true).is_none(),
            "a paused pass observes nothing"
        );
        assert!(
            lifecycle.server_tick_start(false, true).is_none(),
            "not ready yet"
        );
        lifecycle.world_open_ready();
        let (context, phase) = lifecycle
            .server_tick_start(false, true)
            .expect("tick 1 starts");
        assert_eq!(
            phase,
            ServerTickPhase::ServerTickStart {
                world_running: true
            }
        );
        assert_eq!(context.tick, NativeTickId(1));
        let (context, _) = lifecycle.server_tick_end(false, true).expect("tick 1 ends");
        assert_eq!(context.tick, NativeTickId(1), "the end of the same tick");
        lifecycle.server_tick_start(false, true);
        let (context, _) = lifecycle.server_tick_end(false, true).expect("tick 2 ends");
        assert_eq!(context.tick, NativeTickId(2));
        assert!(
            lifecycle.server_tick_start(true, true).is_none(),
            "paused again"
        );
        assert!(lifecycle.server_tick_end(true, true).is_none());
        lifecycle.server_tick_start(false, true);
        let (context, _) = lifecycle
            .server_tick_start(false, true)
            .expect("the next non-paused tick");
        assert_eq!(
            context.tick,
            NativeTickId(4),
            "the paused pass did not count"
        );
    }

    #[test]
    fn a_frozen_world_ticks_with_world_running_false() {
        let mut lifecycle = opening();
        lifecycle.world_open_ready();
        let (_, phase) = lifecycle
            .server_tick_start(false, false)
            .expect("a frozen tick fires");
        assert_eq!(
            phase,
            ServerTickPhase::ServerTickStart {
                world_running: false
            }
        );
        let (_, phase) = lifecycle
            .server_tick_end(false, false)
            .expect("its end fires");
        assert_eq!(
            phase,
            ServerTickPhase::ServerTickEnd {
                world_running: false
            }
        );
    }

    #[test]
    fn an_end_needs_its_start() {
        let mut lifecycle = opening();
        lifecycle.world_open_ready();
        assert!(
            lifecycle.server_tick_end(false, true).is_none(),
            "no tick is open"
        );
        lifecycle.server_tick_start(false, true);
        assert!(lifecycle.server_tick_end(false, true).is_some());
        assert!(
            lifecycle.server_tick_end(false, true).is_none(),
            "the pair closed"
        );
    }

    #[test]
    fn an_errored_tick_leaves_its_start_unpaired() {
        // `tick_server` returns at `result?` before its end site when a
        // level fails; the next tick still starts (the documented limit).
        let mut lifecycle = opening();
        lifecycle.world_open_ready();
        assert!(lifecycle.server_tick_start(false, true).is_some());
        assert!(
            lifecycle.server_tick_start(false, true).is_some(),
            "the next tick starts"
        );
    }

    #[test]
    fn ticks_stop_when_closing_starts() {
        let mut lifecycle = opening();
        lifecycle.world_open_ready();
        lifecycle.server_tick_start(false, true);
        lifecycle.begin_close();
        assert!(lifecycle.server_tick_start(false, true).is_none());
        assert!(
            lifecycle.server_tick_end(false, true).is_none(),
            "a closing tick observes no end"
        );
    }

    #[test]
    fn the_epoch_and_ticks_reset_on_reopen() {
        let mut lifecycle = opening();
        let first_epoch = lifecycle.world_open_ready().expect("ready").epoch;
        lifecycle.server_tick_start(false, true);
        lifecycle.server_tick_end(false, true);
        lifecycle.begin_close();
        lifecycle.world_close_final();
        lifecycle.begin_open(NativeAuthority::LocalAuthoritative);
        let context = lifecycle.world_open_ready().expect("the reopen is ready");
        assert!(context.epoch.0 > first_epoch.0, "the reopen is a new epoch");
        assert_eq!(context.tick, NativeTickId(0));
        assert_eq!(context.authority, NativeAuthority::LocalAuthoritative);
    }

    /// The integrated server builds a fresh `Game` per launch; epochs must
    /// not repeat across those instances (a stale epoch from the previous
    /// session would be indistinguishable from the new one).
    #[test]
    fn epochs_do_not_repeat_across_game_instances() {
        let mut first = NativeLifecycle::default();
        first.begin_open(NativeAuthority::IntegratedServer);
        let first_epoch = first.world_open_ready().expect("the first session").epoch;
        let mut second = NativeLifecycle::default();
        second.begin_open(NativeAuthority::IntegratedServer);
        let second_epoch = second.world_open_ready().expect("the reopen").epoch;
        assert!(
            second_epoch.0 > first_epoch.0,
            "a new server Game never reuses the last session's epoch"
        );
    }

    #[test]
    fn a_failed_open_consumes_its_epoch_for_every_later_instance() {
        let mut first = NativeLifecycle::default();
        first.begin_open(NativeAuthority::IntegratedServer);
        let failed = first.world_open_failed().expect("the failure").epoch;
        let mut second = NativeLifecycle::default();
        second.begin_open(NativeAuthority::IntegratedServer);
        let epoch = second.world_open_ready().expect("the next session").epoch;
        assert!(epoch.0 > failed.0, "the failed epoch is never reused");
    }
}
