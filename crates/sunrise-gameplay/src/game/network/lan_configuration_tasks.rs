//! Dispatch each configuring connection on the server's ordinary tick.
use super::Dim;
use mod_api::{
    __private::{
        ConfigurationTaskOutcome, ConfigurationTaskRegistration, configuration_task_context,
    },
    ConfigurationConnection, ConfigurationTaskError, PayloadBytes, PayloadId,
};
use std::{io, sync::Arc};
use sunrise_sim::{
    lan::{
        Lan, NetConfig,
        configuration_tasks::{
            ConfigurationRequest, Configured, PreparedSpawn, Session, SpawnLevel, SpawnTask,
        },
        data::SyncData,
        playerdata,
    },
    ticket_levels::{PLAYER_SPAWN_RADIUS, SpawnTicketKind},
};

pub(in crate::game::server_side) fn start(sync: Arc<SyncData>, net: NetConfig) -> io::Result<Lan> {
    Lan::start_with_spawn_preparation(sync, net)
}
impl crate::game::ServerGame<'_> {
    fn configuration_callback(
        &mut self,
        session: &Arc<Session>,
        callback: impl FnOnce(&mut Self) -> Result<Option<usize>, String>,
    ) -> bool {
        let (result, commit) = self.storage_scope(|game, _| {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback(game)))
                .unwrap_or_else(|_| Err("Configuration task callback panicked".into()));
            let commit =
                result.is_ok() && !session.closed() && !matches!(session.result(), Some(Err(_)));
            ((result, commit), commit)
        });
        let ordinal = match result {
            Ok(ordinal) => ordinal,
            Err(reason) => {
                session.fail(reason);
                return false;
            }
        };
        if commit && let Some(ordinal) = ordinal {
            session.finish_current(ordinal);
        }
        commit
    }
    pub(in crate::game::server_side) fn tick_lan_configuration_tasks(
        &mut self,
        requests: Vec<ConfigurationRequest>,
    ) -> Option<()> {
        for request in requests {
            if request.session.closed() {
                continue;
            }
            self.server
                .lan
                .as_mut()?
                .hub
                .lan
                .configuration_tasks
                .insert(&request);
            let mut registration = ConfigurationTaskRegistration::default();
            if !mod_api::__private::listened(mod_api::HookId::RegisterConfigurationTasks)
                || self.configuration_callback(&request.session, |game| {
                    game.native_register_configuration_tasks(
                        request.connection,
                        request.player,
                        &request.name,
                        &mut registration,
                    );
                    registration
                        .failure()
                        .map_or(Ok(None), |error| Err(error.to_owned()))
                })
            {
                self.server
                    .lan
                    .as_mut()?
                    .hub
                    .lan
                    .configuration_tasks
                    .register(request.connection, registration.into_tasks());
            }
        }
        let mut after = None;
        while let Some((connection, session)) = self
            .server
            .lan
            .as_ref()?
            .hub
            .lan
            .configuration_tasks
            .next_session(after)
        {
            after = Some(connection);
            if session.closed() {
                let retired = self
                    .server
                    .lan
                    .as_mut()?
                    .hub
                    .lan
                    .configuration_tasks
                    .remove(connection);
                drop(retired);
                continue;
            }
            loop {
                let (completed, turn) = session.turn();
                if let Some(ordinal) = completed {
                    let retired = self
                        .server
                        .lan
                        .as_mut()?
                        .hub
                        .lan
                        .configuration_tasks
                        .take_task(connection, ordinal);
                    drop(retired);
                }
                let Some(turn) = turn else { break };
                let Some(mut task) = self
                    .server
                    .lan
                    .as_mut()?
                    .hub
                    .lan
                    .configuration_tasks
                    .take_task(connection, turn.ordinal)
                else {
                    break;
                };
                // No map borrow or session lock survives extraction. The callback
                // can use the Host (including this session) and its transaction.
                let commit = self.configuration_callback(&session, |game| {
                    let call = if turn.start {
                        "ConfigurationTask::start"
                    } else {
                        "ConfigurationTask::tick"
                    };
                    if !task.owner.is_enabled() {
                        return Ok(Some(turn.ordinal));
                    }
                    let mut outcome = ConfigurationTaskOutcome::Pending;
                    let mut context = configuration_task_context(connection, &mut outcome);
                    task.call(call, |task| {
                        if turn.start {
                            task.start(game, &mut context);
                        } else {
                            task.tick(game, &mut context);
                        }
                    })?;
                    match outcome {
                        ConfigurationTaskOutcome::Pending => Ok(None),
                        ConfigurationTaskOutcome::Finish => Ok(Some(turn.ordinal)),
                        ConfigurationTaskOutcome::Fail(reason) => Err(reason),
                    }
                });
                let mut task = Some(task);
                if commit {
                    self.server
                        .lan
                        .as_mut()?
                        .hub
                        .lan
                        .configuration_tasks
                        .restore_task(connection, &session, turn.ordinal, &mut task);
                }
                drop(task);
                // Only a committed start can immediately open the next turn.
                if !turn.start || !commit || !session.finished() {
                    break;
                }
            }
            if let Some(task) = session.take_spawn() {
                match self.tick_prepare_spawn(task) {
                    Ok(task) => session.advance_spawn(task),
                    Err(error) => session.fail(error),
                }
            }
        }
        Some(())
    }

    fn spawn_level_current(&self, level: SpawnLevel) -> bool {
        Dim::exists(level.id)
            && self.server.levels.0.runtime.generation_of(level.id) == level.generation
    }

    /// PlayerList's unknown-dimension path uses the Overworld. A runtime level's
    /// ticket disappeared with that open, so prepare a fresh Overworld spawn.
    fn prepare_overworld_spawn(&self, lost: SpawnLevel, rotation: [f32; 2]) -> SpawnTask {
        log::warn!(
            "Unknown respawn dimension {}, preparing spawn in the Overworld",
            lost.id
        );
        SpawnTask::Find {
            level: SpawnLevel {
                id: Dim::Overworld.id(),
                generation: 0,
            },
            suggestion: self.world_spawn_position().map(|v| v.floor() as i32),
            rotation,
            finder: None,
        }
    }

    /// Revalidate on the game thread, even when the level was lost between
    /// sending FinishConfiguration and seeing its acknowledgement.
    pub(in crate::game::server_side) fn acknowledged_spawn(
        &self,
        configured: &Configured,
    ) -> Option<Result<Option<PreparedSpawn>, String>> {
        let session = configured.session();
        if let Some(spawn) = session.prepared_spawn()
            && !self.spawn_level_current(spawn.level)
        {
            session.advance_spawn(self.prepare_overworld_spawn(spawn.level, spawn.rotation));
        }
        session
            .result()
            .map(|r| r.map(|()| session.prepared_spawn()))
    }

    /// `PrepareSpawnTask`: resolve the place before play, load its FULL square,
    /// then refresh the ticket while the client acknowledges configuration.
    fn tick_prepare_spawn(&mut self, task: SpawnTask) -> Result<SpawnTask, String> {
        let task = match task {
            SpawnTask::Start(player) => {
                let host = self.server.lan.as_ref().ok_or("Server closed")?;
                let shapes = host.hub.config.levels.clone();
                let owner = host
                    .owner
                    .as_ref()
                    .filter(|o| o.uuid == player)
                    .and_then(|o| o.saved.as_deref());
                let saved = playerdata::saved_position(&self.server.world_root, player, owner);
                let dimension = saved
                    .dimension
                    .as_deref()
                    .into_iter()
                    .chain(self.common.level.world_spawn_dimension.as_deref())
                    .find(|id| shapes.has(id))
                    .map_or(Dim::Overworld, Dim::from_id)
                    .id();
                let level = SpawnLevel {
                    id: dimension,
                    generation: self.server.levels.0.runtime.generation_of(dimension),
                };
                let rotation = saved
                    .rotation
                    .unwrap_or(self.common.level.world_spawn_angle);
                if let Some(position) = saved.position {
                    let prepared = PreparedSpawn {
                        level,
                        position,
                        rotation,
                    };
                    let back = self.common.dim;
                    self.enter_level(Dim::from_id(dimension))
                        .map_err(|e| e.to_string())?;
                    self.server
                        .tickets
                        .0
                        .add_spawn(prepared.chunk(), SpawnTicketKind::Player);
                    self.enter_level(back).map_err(|e| e.to_string())?;
                    return Ok(SpawnTask::Load(prepared));
                }
                SpawnTask::Find {
                    level,
                    suggestion: self.world_spawn_position().map(|v| v.floor() as i32),
                    rotation,
                    finder: None,
                }
            }
            task => task,
        };
        let (level, rotation) = match &task {
            SpawnTask::Find {
                level, rotation, ..
            } => (*level, *rotation),
            SpawnTask::Load(p) | SpawnTask::Ready(p) => (p.level, p.rotation),
            SpawnTask::Start(_) => unreachable!(),
        };
        if !self.spawn_level_current(level) {
            return Ok(self.prepare_overworld_spawn(level, rotation));
        }
        let back = self.common.dim;
        self.enter_level(Dim::from_id(level.id))
            .map_err(|e| e.to_string())?;
        let task = match task {
            SpawnTask::Find {
                level,
                suggestion,
                rotation,
                mut finder,
            } => match self.search_spawn(suggestion, &mut finder) {
                Ok(feet) => {
                    let prepared = PreparedSpawn {
                        level,
                        position: [feet.x, feet.y, feet.z],
                        rotation,
                    };
                    self.server
                        .tickets
                        .0
                        .add_spawn(prepared.chunk(), SpawnTicketKind::Player);
                    SpawnTask::Load(prepared)
                }
                Err(chunk) => {
                    self.server
                        .tickets
                        .0
                        .add_spawn(chunk, SpawnTicketKind::Search);
                    SpawnTask::Find {
                        level,
                        suggestion,
                        rotation,
                        finder,
                    }
                }
            },
            SpawnTask::Load(prepared) => {
                if spawn_area_full(&self.common.world, prepared.chunk(), PLAYER_SPAWN_RADIUS) {
                    // Hand off the pending load range to a fresh Ready ticket.
                    self.server
                        .tickets
                        .0
                        .add_spawn(prepared.chunk(), SpawnTicketKind::Player);
                    SpawnTask::Ready(prepared)
                } else {
                    SpawnTask::Load(prepared)
                }
            }
            SpawnTask::Ready(prepared) => {
                self.server
                    .tickets
                    .0
                    .add_spawn(prepared.chunk(), SpawnTicketKind::Player);
                SpawnTask::Ready(prepared)
            }
            SpawnTask::Start(_) => unreachable!(),
        };
        self.enter_level(back).map_err(|e| e.to_string())?;
        Ok(task)
    }

    pub(in crate::game::server_side) fn tick_spawn_tickets(&mut self, running: bool, notify: bool) {
        if self.server.tickets.0.spawn_areas().next().is_some() {
            // ServerChunkCache.tick doesn't purge in a frozen level, nor
            // does pause-when-empty tick the cache (it only polls chunk work).
            if running {
                let streaming = &self.server.streaming;
                self.server
                    .tickets
                    .0
                    .tick_spawn(|center| streaming.chunk_ready_for_saving(center));
            }
            self.rebuild_chunk_tickets(notify);
        }
    }

    pub(in crate::game) fn configuration_session(
        &self,
        connection: ConfigurationConnection,
    ) -> Option<&Arc<Session>> {
        self.server
            .lan
            .as_ref()?
            .hub
            .lan
            .configuration_tasks
            .get(connection)
    }
    pub(in crate::game) fn configuration_has_channel(
        &self,
        connection: ConfigurationConnection,
        channel: &PayloadId,
    ) -> bool {
        self.configuration_session(connection)
            .is_some_and(|session| session.has_channel(channel))
    }
    pub(in crate::game) fn configuration_send_payload(
        &mut self,
        connection: ConfigurationConnection,
        channel: &PayloadId,
        bytes: PayloadBytes,
    ) -> Result<(), ConfigurationTaskError> {
        let (endpoint, epoch) = self
            .configuration_session(connection)
            .ok_or(ConfigurationTaskError::Disconnected)?
            .payload_target()?;
        self.queue_payload(endpoint, epoch, channel, bytes)
            .map(|_| ())
            .map_err(ConfigurationTaskError::Payload)
    }
}

/// `ServerChunkCache.addTicketAndLoadWithRadius`: every chunk, including the corners, is FULL.
fn spawn_area_full(world: &sunrise_level::world::World, (cx, cz): (i32, i32), radius: i32) -> bool {
    (-radius..=radius).all(|z| {
        (-radius..=radius).all(|x| {
            world
                .chunks
                .get(&(cx + x, cz + z))
                .is_some_and(|chunk| chunk.lit)
        })
    })
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::payload_transport::PayloadTransportGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_level::{chunk::Chunk, world::World};
    use sunrise_world::registry::BlockStateId;

    #[test]
    fn spawn_is_not_ready_until_the_last_corner_is_full() {
        let mut world = World::new(BlockStateId(0), 0);
        let center = (8, -6);
        let corner = (5, -9);
        for z in -9..=-3 {
            for x in 5..=11 {
                if (x, z) != corner {
                    let mut column = Chunk::empty(x, z);
                    column.lit = true;
                    world.chunks.insert((x, z), Arc::new(column));
                }
            }
        }
        assert!(!spawn_area_full(&world, center, PLAYER_SPAWN_RADIUS));
        let mut column = Chunk::empty(corner.0, corner.1);
        column.lit = false;
        world.chunks.insert(corner, Arc::new(column));
        assert!(!spawn_area_full(&world, center, PLAYER_SPAWN_RADIUS));
        Arc::make_mut(world.chunks.get_mut(&corner).unwrap()).lit = true;
        assert!(spawn_area_full(&world, center, PLAYER_SPAWN_RADIUS));
    }
}
