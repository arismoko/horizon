//! Server-thread task queues and progress shared with each connection worker.
use super::config_payloads::ConfigSender;
use crate::{
    raw_payloads::{Endpoint, Transport, connection::Epoch},
    world_spawn::PlayerSpawnFinder,
};
use mod_api::{
    __private::RegisteredConfigurationTask, ConfigurationConnection, ConfigurationTaskError,
    PayloadId, PayloadSendError,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use sunrise_protocol::payload_negotiation::Phase;

pub struct ConfigurationRequest {
    pub connection: ConfigurationConnection,
    pub player: u128,
    pub name: String,
    pub session: Arc<Session>,
}
struct Entry {
    session: Arc<Session>,
    queue: Vec<Option<RegisteredConfigurationTask>>,
}

pub struct Tasks {
    incarnation: u64,
    sessions: BTreeMap<u64, Entry>,
}
impl Tasks {
    pub fn new(incarnation: u64) -> Self {
        Self {
            incarnation,
            sessions: BTreeMap::new(),
        }
    }
    fn local_id(&self, connection: ConfigurationConnection) -> Option<u64> {
        let (incarnation, id) = mod_api::__private::configuration_connection_parts(connection);
        (incarnation == self.incarnation).then_some(id)
    }
    pub fn insert(&mut self, request: &ConfigurationRequest) {
        let Some(id) = self.local_id(request.connection) else {
            return;
        };
        self.sessions.insert(
            id,
            Entry {
                session: request.session.clone(),
                queue: Vec::new(),
            },
        );
    }
    /// Publish a committed registration to the worker.
    pub fn register(
        &mut self,
        connection: ConfigurationConnection,
        tasks: Vec<RegisteredConfigurationTask>,
    ) {
        let Some(id) = self.local_id(connection) else {
            return;
        };
        let Some(entry) = self.sessions.get_mut(&id) else {
            return;
        };
        let task_types = tasks.iter().map(|task| task.task_type().clone()).collect();
        entry.queue = tasks.into_iter().map(Some).collect();
        entry.session.register(task_types);
    }
    pub fn get(&self, connection: ConfigurationConnection) -> Option<&Arc<Session>> {
        let id = self.local_id(connection)?;
        self.sessions.get(&id).map(|entry| &entry.session)
    }
    /// Take the object out of its slot before running or retiring it.
    pub fn take_task(
        &mut self,
        connection: ConfigurationConnection,
        ordinal: usize,
    ) -> Option<RegisteredConfigurationTask> {
        let id = self.local_id(connection)?;
        self.sessions.get_mut(&id)?.queue.get_mut(ordinal)?.take()
    }
    /// Put back a pending object only while its original session and turn are live.
    /// On refusal the caller still owns the object and retires it unlocked.
    pub fn restore_task(
        &mut self,
        connection: ConfigurationConnection,
        session: &Arc<Session>,
        ordinal: usize,
        task: &mut Option<RegisteredConfigurationTask>,
    ) {
        let Some(id) = self.local_id(connection) else {
            return;
        };
        let Some(entry) = self
            .sessions
            .get_mut(&id)
            .filter(|entry| Arc::ptr_eq(&entry.session, session))
        else {
            return;
        };
        if session.transport().closed() {
            return;
        }
        let state = session.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.running(ordinal)
            && let Some(slot) = entry.queue.get_mut(ordinal).filter(|slot| slot.is_none())
        {
            *slot = task.take();
        }
    }
    /// Take out a closed session's remaining tasks so the caller can retire them.
    pub fn remove(
        &mut self,
        connection: ConfigurationConnection,
    ) -> Vec<Option<RegisteredConfigurationTask>> {
        let Some(id) = self.local_id(connection) else {
            return Vec::new();
        };
        self.sessions
            .remove(&id)
            .map_or_else(Vec::new, |entry| entry.queue)
    }
    /// `getChunkRangeFuture` keeps the requested generation work alive apart from the ticket's age.
    pub fn loading_spawns(&self, dimension: &'static str) -> impl Iterator<Item = (i32, i32)> + '_ {
        self.sessions.values().filter_map(move |entry| {
            let state = entry
                .session
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            match state.spawn.as_ref() {
                Some(SpawnTask::Load(spawn))
                    if spawn.level.id == dimension
                        && !state.closed
                        && !entry.session.transport().closed() =>
                {
                    Some(spawn.chunk())
                }
                _ => None,
            }
        })
    }

    /// Visit the next session without retaining a borrow across a task callback.
    pub fn next_session(
        &self,
        after: Option<ConfigurationConnection>,
    ) -> Option<(ConfigurationConnection, Arc<Session>)> {
        use std::ops::Bound::{Excluded, Unbounded};
        let lower = match after {
            None => Unbounded,
            Some(connection) => Excluded(self.local_id(connection)?),
        };
        self.sessions
            .range((lower, Unbounded))
            .next()
            .map(|(&id, entry)| {
                (
                    mod_api::__private::configuration_connection(self.incarnation, id),
                    entry.session.clone(),
                )
            })
    }
}
/// The particular open of the destination level, not just its dimension name.
#[derive(Clone, Copy, Debug)]
pub struct SpawnLevel {
    pub id: &'static str,
    pub generation: u32,
}

/// The prepared place and angle survive the configuration acknowledgement.
#[derive(Clone, Copy, Debug)]
pub struct PreparedSpawn {
    pub level: SpawnLevel,
    pub position: [f64; 3],
    pub rotation: [f32; 2],
}
impl PreparedSpawn {
    pub fn chunk(self) -> (i32, i32) {
        (
            (self.position[0].floor() as i32) >> 4,
            (self.position[2].floor() as i32) >> 4,
        )
    }
}

/// Keeps PrepareSpawnTask.Ready alive until game-thread placement, or until
/// the connection or join is dropped.
pub struct Configured(pub(crate) Arc<Session>);
impl Configured {
    pub fn session(&self) -> &Session {
        &self.0
    }
}
impl Drop for Configured {
    fn drop(&mut self) {
        self.0.close();
    }
}

pub enum SpawnTask {
    Start(u128),
    Find {
        level: SpawnLevel,
        suggestion: [i32; 3],
        rotation: [f32; 2],
        finder: Option<PlayerSpawnFinder>,
    },
    Load(PreparedSpawn),
    Ready(PreparedSpawn),
}

#[derive(Default)]
struct State {
    spawn: Option<SpawnTask>,
    prepared_spawn: Option<PreparedSpawn>,
    task_types: Option<Vec<mod_api::Identifier>>,
    at: usize,
    started: bool,
    finished: bool,
    failure: Option<String>,
    closed: bool,
}
impl State {
    fn tasks_done(&self) -> bool {
        self.task_types.as_ref().map(Vec::len) == Some(self.at)
    }
    fn running(&self, ordinal: usize) -> bool {
        self.task_types
            .as_ref()
            .is_some_and(|types| ordinal < types.len())
            && self.at == ordinal
            && self.started
            && !self.finished
            && !self.closed
            && self.failure.is_none()
    }
}
pub struct Turn {
    pub ordinal: usize,
    pub start: bool,
}

pub struct Session {
    needs_spawn: bool,
    state: Mutex<State>,
    sender: ConfigSender,
}
impl Session {
    #[cfg(test)]
    pub fn new(sender: ConfigSender) -> Arc<Self> {
        Self::new_for_player(sender, None)
    }
    pub(crate) fn new_for_player(sender: ConfigSender, player: Option<u128>) -> Arc<Self> {
        Arc::new(Self {
            needs_spawn: player.is_some(),
            state: Mutex::new(State {
                spawn: player.map(SpawnTask::Start),
                ..State::default()
            }),
            sender,
        })
    }

    /// `ServerConfigurationPacketListenerImpl.returnToWorld`: prepare after the preceding tasks.
    pub fn take_spawn(&self) -> Option<SpawnTask> {
        if !self.needs_spawn {
            return None;
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed || state.failure.is_some() || !state.tasks_done() {
            return None;
        }
        state.spawn.take()
    }

    pub fn advance_spawn(&self, task: SpawnTask) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.prepared_spawn = match &task {
            SpawnTask::Ready(spawn) => Some(*spawn),
            _ => None,
        };
        state.spawn = Some(task);
    }

    pub fn prepared_spawn(&self) -> Option<PreparedSpawn> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .prepared_spawn
    }
    pub(crate) fn register(&self, task_types: Vec<mod_api::Identifier>) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .task_types = Some(task_types);
    }
    /// Advance past a published completion, returning its ordinal for retirement
    /// even when it was the final task. Then take the next eligible turn, if any.
    pub fn turn(&self) -> (Option<usize>, Option<Turn>) {
        if self.transport().closed() {
            return (None, None);
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(task_count) = state.task_types.as_ref().map(Vec::len) else {
            return (None, None);
        };
        if state.closed || state.failure.is_some() {
            return (None, None);
        }
        let completed = state.finished.then_some(state.at);
        if completed.is_some() {
            state.at += 1;
            state.started = false;
            state.finished = false;
        }
        let turn = (state.at < task_count).then(|| {
            let start = !state.started;
            state.started = true;
            Turn {
                ordinal: state.at,
                start,
            }
        });
        (completed, turn)
    }
    pub fn finished(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .finished
    }
    /// Verify that the currently running task on this session matches `task_type`
    /// and return its internal turn ordinal so deferred network publication can
    /// only finish that exact turn.
    pub fn running_ordinal_for_type(
        &self,
        task_type: &mod_api::Identifier,
    ) -> Result<usize, ConfigurationTaskError> {
        if self.transport().closed() {
            return Err(ConfigurationTaskError::Disconnected);
        }
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed || state.failure.is_some() {
            return Err(ConfigurationTaskError::Disconnected);
        }
        let Some(types) = &state.task_types else {
            return Err(ConfigurationTaskError::NotRunning);
        };
        if !(state.started && !state.finished && state.at < types.len()) {
            return Err(ConfigurationTaskError::NotRunning);
        }
        if &types[state.at] != task_type {
            return Err(ConfigurationTaskError::WrongType);
        }
        Ok(state.at)
    }
    /// Mark the currently running task finished immediately if its type matches `task_type`.
    pub fn finish_type(
        &self,
        task_type: &mod_api::Identifier,
    ) -> Result<(), ConfigurationTaskError> {
        if self.transport().closed() {
            return Err(ConfigurationTaskError::Disconnected);
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed || state.failure.is_some() {
            return Err(ConfigurationTaskError::Disconnected);
        }
        let Some(types) = &state.task_types else {
            return Err(ConfigurationTaskError::NotRunning);
        };
        if !(state.started && !state.finished && state.at < types.len()) {
            return Err(ConfigurationTaskError::NotRunning);
        }
        if &types[state.at] != task_type {
            return Err(ConfigurationTaskError::WrongType);
        }
        state.finished = true;
        Ok(())
    }
    /// Mark `ordinal` finished while the caller holds an active `Epoch::with_active`
    /// publication gate. Never acquires `transport()` (whose connection-state lock is
    /// ordered outside epoch invalidation).
    pub fn finish_ordinal_under_epoch(&self, ordinal: usize) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.running(ordinal) {
            state.finished = true;
        }
    }
    /// Mark the currently running task finished if this ordinal is still active.
    pub fn finish_current(&self, ordinal: usize) {
        if self.transport().closed() {
            return;
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.running(ordinal) {
            state.finished = true;
        }
    }
    pub fn fail(&self, reason: impl Into<String>) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.failure.get_or_insert_with(|| reason.into());
    }
    pub fn has_channel(&self, channel: &PayloadId) -> bool {
        !self.closed()
            && self
                .transport()
                .epoch()
                .is_some_and(|epoch| epoch.peer.can_send(Phase::Configuration, channel).is_ok())
    }
    pub fn payload_target(&self) -> Result<(Endpoint, Arc<Epoch>), ConfigurationTaskError> {
        // The retained session may still be in the game thread's task map
        // after I/O publishes Play. Bind the send to this exact Configuration epoch.
        let epoch = self
            .transport()
            .epoch()
            .ok_or(ConfigurationTaskError::Payload(
                PayloadSendError::NotNegotiated,
            ))?;
        if epoch.phase != Phase::Configuration {
            return Err(ConfigurationTaskError::Disconnected);
        }
        Ok((Endpoint::configuration(self.sender.clone(), None), epoch))
    }
    pub fn transport(&self) -> &Arc<Transport> {
        self.sender.payload_transport()
    }
    pub fn result(&self) -> Option<Result<(), String>> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(error) = &state.failure {
            return Some(Err(error.clone()));
        }
        (state.tasks_done() && (!self.needs_spawn || state.prepared_spawn.is_some()))
            .then_some(Ok(()))
    }
    pub fn close(&self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
    }
    pub fn closed(&self) -> bool {
        self.transport().closed() || self.state.lock().unwrap_or_else(|e| e.into_inner()).closed
    }
}

impl crate::raw_payloads::ConfigurationTask for Session {
    fn running_ordinal_for_type(
        &self,
        task_type: &mod_api::Identifier,
    ) -> Result<usize, ConfigurationTaskError> {
        Session::running_ordinal_for_type(self, task_type)
    }
    fn finish_ordinal_under_epoch(&self, ordinal: usize) {
        Session::finish_ordinal_under_epoch(self, ordinal)
    }
    fn finish_type(&self, task_type: &mod_api::Identifier) -> Result<(), ConfigurationTaskError> {
        Session::finish_type(self, task_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_drops_only_the_current_listeners_task_registrations() {
        use mod_api::{
            __private::ConfigurationTaskRegistration, ConfigurationTask, ConfigurationTaskContext,
            Identifier, RegisterConfigurationTasksEvent, ServerHost,
        };

        struct Task(Identifier, bool);
        impl ConfigurationTask for Task {
            fn task_type(&self) -> &Identifier {
                if self.1 {
                    panic!("test task_type fault");
                }
                &self.0
            }

            fn start(&mut self, _: &mut dyn ServerHost, _: &mut ConfigurationTaskContext<'_>) {}
        }

        struct Registers(&'static str, bool);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Registers {
            fn register_configuration_tasks(
                &self,
                _: &mut dyn ServerHost,
                event: &mut RegisterConfigurationTasksEvent<'_>,
            ) {
                let task = |refuse| Task(Identifier::parse(self.0).unwrap(), refuse);
                event.register(task(false));
                if self.1 {
                    // Both the staged task and callback failure belong to this listener.
                    event.register(task(true));
                    panic!("the listener unwinds to its checkpoint");
                }
            }
        }

        mod_platform::testing::with_mods(
            vec![
                mod_api::ModEntry::new("first_task", Box::new(Registers("config:first", false))),
                mod_api::ModEntry::new("refused_task", Box::new(Registers("config:next", true))),
                mod_api::ModEntry::new("next_task", Box::new(Registers("config:next", false))),
            ]
            .into(),
            || {
                let mut host = mod_api::testing::FakeHost::new();
                let mut registration = ConfigurationTaskRegistration::default();
                mod_api::dispatch_register_configuration_tasks(
                    &mut host,
                    mod_api::__private::configuration_connection(1, 1),
                    0,
                    "Test",
                    &mut registration,
                );
                assert_eq!(registration.failure(), None);
                let tasks = registration.into_tasks();
                assert_eq!(
                    tasks.iter().map(|task| task.owner.id()).collect::<Vec<_>>(),
                    ["first_task", "next_task"]
                );
            },
        );
    }

    #[test]
    fn a_panicking_task_type_refuses_one_registration_and_disables_only_its_owner() {
        use mod_api::{
            __private::ConfigurationTaskRegistration, ConfigurationTask, ConfigurationTaskContext,
            Identifier, RegisterConfigurationTasksEvent, ServerHost,
            content::crash::current_native_call,
        };

        use std::sync::atomic::{AtomicUsize, Ordering};

        static BROKEN_LOOKUPS: AtomicUsize = AtomicUsize::new(0);
        static BROKEN_DISPOSALS_IN_SCOPE: AtomicUsize = AtomicUsize::new(0);

        struct Task {
            owner: &'static str,
            kind: Identifier,
            panic: bool,
        }
        impl Drop for Task {
            fn drop(&mut self) {
                if self.panic {
                    if mod_api::__private::game::retained_callback_active()
                        && current_native_call().is_some_and(|call| {
                            (call.id, call.hook) == (self.owner, "ConfigurationTask::drop")
                        })
                    {
                        BROKEN_DISPOSALS_IN_SCOPE.fetch_add(1, Ordering::Relaxed);
                    }
                    panic!("test task destructor panic");
                }
            }
        }
        impl ConfigurationTask for Task {
            fn task_type(&self) -> &Identifier {
                let call = current_native_call().unwrap();
                assert_eq!(
                    (call.id, call.hook),
                    (self.owner, "ConfigurationTask::task_type")
                );
                assert!(mod_api::__private::game::retained_callback_active());
                if self.panic {
                    BROKEN_LOOKUPS.fetch_add(1, Ordering::Relaxed);
                    panic!("test task type panic");
                }
                &self.kind
            }

            fn start(&mut self, _: &mut dyn ServerHost, _: &mut ConfigurationTaskContext<'_>) {}
        }

        struct Register(&'static str, bool);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Register {
            fn register_configuration_tasks(
                &self,
                _: &mut dyn ServerHost,
                event: &mut RegisterConfigurationTasksEvent<'_>,
            ) {
                // A task callback failure still refuses admission.
                event.register(Task {
                    owner: self.0,
                    kind: Identifier::parse("config:task").unwrap(),
                    panic: self.1,
                });
            }
        }

        const BROKEN: &str = "broken_task_type";
        const HEALTHY: &str = "healthy_task_type";
        mod_platform::testing::with_mods(
            vec![
                mod_api::ModEntry::new(BROKEN, Box::new(Register(BROKEN, true))),
                mod_api::ModEntry::new(HEALTHY, Box::new(Register(HEALTHY, false))),
            ]
            .into(),
            || {
                let mut host = mod_api::testing::FakeHost::new();
                for peer in [1, 2] {
                    let mut registration = ConfigurationTaskRegistration::default();
                    mod_api::dispatch_register_configuration_tasks(
                        &mut host,
                        mod_api::__private::configuration_connection(1, peer),
                        0,
                        "Test",
                        &mut registration,
                    );
                    if peer == 1 {
                        assert!(matches!(
                            registration.failure(),
                            Some(reason)
                                if reason.contains(BROKEN) && reason.contains("test task type panic")
                        ));
                    } else {
                        assert_eq!(registration.failure(), None);
                    }
                    let tasks = registration.into_tasks();
                    assert_eq!(tasks.len(), 1);
                    assert_eq!(tasks[0].owner.id(), HEALTHY);
                }
                assert_eq!(BROKEN_LOOKUPS.load(Ordering::Relaxed), 1);
                assert_eq!(BROKEN_DISPOSALS_IN_SCOPE.load(Ordering::Relaxed), 2);
                let notices = mod_api::__private::take_hook_panics()
                    .into_iter()
                    .filter(|notice| notice.mod_id == BROKEN)
                    .map(|notice| notice.hook)
                    .collect::<Vec<_>>();
                assert_eq!(
                    notices,
                    [
                        "ConfigurationTask::task_type",
                        "ConfigurationTask::drop",
                        "ConfigurationTask::drop",
                    ]
                );
            },
        );
    }

    #[test]
    fn configuration_finishes_after_tasks_and_spawn_and_keeps_the_ready_spawn_until_acknowledged() {
        use crate::lan::config_payloads::ConfigIo;
        use sunrise_protocol::connection::{Connection, Pipe};

        let (_peer, server) = Pipe::pair();
        let mut server = Connection::local(server);
        let transport = Transport::empty();
        let (events, _receiver) = crate::lan::event_channel_with_capacity(1 << 20);
        let mut io = ConfigIo::new(&mut server, transport.clone(), events).unwrap();
        let session = Session::new_for_player(io.task_sender(transport).unwrap(), Some(123));
        session.register(vec![mod_api::Identifier::parse("demo:task").unwrap()]);
        assert!(session.take_spawn().is_none());
        let turn = session.turn().1.unwrap();
        session.finish_current(turn.ordinal);
        session.turn();
        assert!(matches!(session.take_spawn(), Some(SpawnTask::Start(123))));
        assert!(session.result().is_none());
        let prepared = PreparedSpawn {
            level: SpawnLevel {
                id: "minecraft:the_nether",
                generation: 0,
            },
            position: [-0.5, 67.0, 48.5],
            rotation: [135.0, -20.0],
        };
        session.advance_spawn(SpawnTask::Load(prepared));
        assert!(session.result().is_none());
        assert!(matches!(session.take_spawn(), Some(SpawnTask::Load(_))));
        session.advance_spawn(SpawnTask::Ready(prepared));
        assert!(matches!(session.take_spawn(), Some(SpawnTask::Ready(_))));
        assert_eq!(session.result(), Some(Ok(())));
        assert_eq!(
            session.prepared_spawn().unwrap().position,
            prepared.position
        );
        session.close();
        assert!(session.take_spawn().is_none());
    }

    #[test]
    fn finish_type_and_finish_current_match_only_the_active_task() {
        use crate::lan::config_payloads::ConfigIo;
        use sunrise_protocol::connection::{Connection, Pipe};

        let (_peer, server) = Pipe::pair();
        let mut server = Connection::local(server);
        let transport = Transport::empty();
        let (events, _receiver) = crate::lan::event_channel_with_capacity(1 << 20);
        let mut io = ConfigIo::new(&mut server, transport.clone(), events).unwrap();
        let session = Session::new(io.task_sender(transport).unwrap());
        let first = mod_api::Identifier::parse("demo:first").unwrap();
        let second = mod_api::Identifier::parse("demo:second").unwrap();
        assert_eq!(
            session.running_ordinal_for_type(&first),
            Err(ConfigurationTaskError::NotRunning)
        );

        session.register(vec![first.clone(), second.clone()]);
        assert_eq!(
            session.running_ordinal_for_type(&first),
            Err(ConfigurationTaskError::NotRunning)
        );
        let (completed, turn) = session.turn();
        assert_eq!(completed, None);
        let turn = turn.unwrap();
        assert!(turn.start);
        assert_eq!(turn.ordinal, 0);
        assert_eq!(session.running_ordinal_for_type(&first), Ok(0));
        assert_eq!(
            session.running_ordinal_for_type(&second),
            Err(ConfigurationTaskError::WrongType)
        );
        assert_eq!(
            session.finish_type(&second),
            Err(ConfigurationTaskError::WrongType)
        );
        assert!(!session.finished());
        let (completed, tick) = session.turn();
        assert_eq!(completed, None);
        let tick = tick.unwrap();
        assert!(!tick.start);
        assert_eq!(tick.ordinal, 0);

        assert!(session.finish_type(&first).is_ok());
        assert!(session.finished());
        assert_eq!(
            session.running_ordinal_for_type(&first),
            Err(ConfigurationTaskError::NotRunning)
        );
        assert_eq!(
            session.finish_type(&first),
            Err(ConfigurationTaskError::NotRunning)
        );

        let (completed, next) = session.turn();
        assert_eq!(completed, Some(0));
        let next = next.unwrap();
        assert!(next.start);
        assert_eq!(next.ordinal, 1);
        session.finish_ordinal_under_epoch(turn.ordinal);
        assert!(!session.finished());
        assert_eq!(
            session.running_ordinal_for_type(&first),
            Err(ConfigurationTaskError::WrongType)
        );
        assert_eq!(session.running_ordinal_for_type(&second), Ok(1));
        session.finish_ordinal_under_epoch(next.ordinal);
        let (completed, turn) = session.turn();
        assert_eq!(completed, Some(1));
        assert!(turn.is_none());
        assert_eq!(session.result(), Some(Ok(())));
    }
}
