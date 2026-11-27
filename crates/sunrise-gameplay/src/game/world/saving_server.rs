use crate::game::ServerGame;
use std::{
    collections::{HashSet, VecDeque},
    sync::mpsc::{Receiver, TryRecvError},
    time::{Duration, Instant},
};
use sunrise_game_data::Result;
use sunrise_gameplay_common::game::saving::CLOSING_SNAPSHOT_BUDGET;
use sunrise_gameplay_common::game::saving::CLOSING_SNAPSHOTS_PER_FRAME;
use sunrise_gameplay_common::game::saving::ClosingSave;
use sunrise_gameplay_common::game::saving::SNAPSHOT_BUDGET;
use sunrise_gameplay_common::game::saving::SNAPSHOTS_PER_FRAME;
use sunrise_gameplay_common::game::saving::Stage;

impl ServerGame<'_> {
    #[inline(always)]
    #[allow(dead_code)]
    pub(in crate::game) fn poll_closing_save_chunks_server(
        &mut self,
        keys: &mut VecDeque<(i32, i32)>,
        per_frame: usize,
        budget: Duration,
    ) -> Result<Option<VecDeque<(i32, i32)>>> {
        let started = Instant::now();
        for _ in 0..per_frame {
            let Some(&key) = keys.front() else { break };
            // ChunkMap.save consumes the dirty mark before capturing the snapshot.
            self.common.world.modified.remove(&key);
            let chunks = self.with_entities(self.common.world.chunks.get(&key));
            self.server.save.0.save_later(
                &chunks,
                self.common.pack.assets.registry,
                world_biomes!(self),
            )?;
            keys.pop_front();
            if started.elapsed() >= budget {
                break;
            }
        }
        if keys.is_empty() {
            Ok(Some(self.prepare_entity_save().into_iter().collect()))
        } else {
            Ok(None)
        }
    }
    #[inline(always)]
    #[allow(dead_code)]
    pub(in crate::game) fn poll_closing_save_entities_server(
        &mut self,
        keys: &mut VecDeque<(i32, i32)>,
        per_frame: usize,
        budget: Duration,
    ) -> Result<Option<Receiver<std::io::Result<()>>>> {
        let started = Instant::now();
        for _ in 0..per_frame {
            let Some(&key) = keys.front() else { break };
            let chunks = self.entity_chunks(&HashSet::from([key]), false);
            self.server.save.0.save_entities_later(chunks)?;
            keys.pop_front();
            if started.elapsed() >= budget {
                break;
            }
        }
        if keys.is_empty() {
            self.prune_entity_memory();
            self.save_raids();
            if let Err(error) = self
                .server
                .save
                .0
                .save_attachments(&self.common.world.attachments)
                && !error.is::<sunrise_mod_runtime::attachments::AttachmentEncodeError>()
            {
                return Err(error);
            }
            self.save_chunk_tickets()?;
            self.save_entity_metadata()?;
            self.save_game_metadata()?;
            Ok(Some(self.server.save.0.flush_later()?))
        } else {
            Ok(None)
        }
    }
    #[inline(always)]
    #[allow(dead_code)]
    pub(in crate::game) fn poll_closing_save_flush_server(
        &self,
    ) -> Result<Receiver<std::io::Result<()>>> {
        self.server.save.0.flush_later()
    }
    /// Save the level's chunk tickets: `/forceload`'s in vanilla's `chunk_tickets.dat`, the
    /// mods' in Sunrise's own `mod_chunk_tickets.dat`.
    pub fn save_chunk_tickets(&self) -> Result<()> {
        if self.server.startup_ticket_restore_pending {
            return Ok(());
        }
        self.server.save.0.save_chunk_tickets(
            self.server.streaming.commanded(),
            self.server.tickets.0.portal_tickets(),
        )?;
        self.server.save.0.save_mod_tickets(
            self.server.streaming.mod_tickets(),
            mod_api::__private::game::ticket_controller_id,
        )
    }
    /// Save the level in place: its changed chunks, its entities and its raids.
    pub fn save_level(&mut self, wait: bool) -> Result<()> {
        self.admit_level_save()?;
        // No save holds a player vehicle: a mob a mod seated on a player gets off first.
        self.eject_player_riders(None);
        self.save_border();
        self.save_chunk_tickets()?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match self
                .server
                .streaming
                .prepare_save(&mut self.common.world, wait, deadline)
            {
                sunrise_level::chunk_streamer::SaveStep::Full(chunk) => {
                    self.save_late_full_chunk(chunk);
                }
                sunrise_level::chunk_streamer::SaveStep::Protos(protos) => {
                    self.server.streaming.finish_save(protos, wait);
                    break;
                }
                sunrise_level::chunk_streamer::SaveStep::Pending => unreachable!(),
            }
        }
        let keys: Vec<_> = self.common.world.modified.iter().copied().collect();
        if let Err(error) = self
            .server
            .save
            .0
            .save_attachments(&self.common.world.attachments)
            && !error.is::<sunrise_mod_runtime::attachments::AttachmentEncodeError>()
        {
            return Err(error);
        }
        // ChunkMap.save calls tryMarkSaved before creating the snapshot. A record failure
        // finishes this attempt and does not put the mark back.
        for key in &keys {
            self.common.world.modified.remove(key);
        }
        let chunks =
            self.with_entities(keys.iter().filter_map(|k| self.common.world.chunks.get(k)));
        self.server.save.0.save_later(
            &chunks,
            self.common.pack.assets.registry,
            world_biomes!(self),
        )?;
        // Prepare entities while the chunk writer runs, then use one barrier.
        if !wait {
            self.common.world.modified.autosaved(&keys, Instant::now());
        }
        self.save_entities(false)?;
        self.save_raids();
        if wait {
            self.server.save.0.flush()?;
        }
        if mod_api::__private::listened(mod_api::HookId::LevelSave) {
            self.native_level_save(self.common.dim.id());
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub(in crate::game) fn begin_server_control_save(
        &mut self,
        all: bool,
        reply: std::sync::mpsc::Sender<String>,
    ) -> Result<ClosingSave> {
        if all {
            self.common
                .world
                .modified
                .extend(self.common.world.chunks.keys().copied());
        }
        Ok(ClosingSave {
            quit: false,
            close: false,
            reply: Some(reply),
            started: Instant::now(),
            stage: Stage::Generation,
        })
    }

    /// Advance a server-owned staged save by one step. Returns `Ok(true)` once finished.
    #[allow(dead_code)]
    pub fn poll_server_closing_save(&mut self, save: &mut ClosingSave) -> Result<bool> {
        let defers = !matches!(save.stage, Stage::Server | Stage::Quit)
            && save.started.elapsed() < Duration::from_secs(30);
        if defers && self.defer_chunk_disk_save() {
            return Ok(false);
        }
        let (per_frame, budget) = if save.close {
            (CLOSING_SNAPSHOTS_PER_FRAME, CLOSING_SNAPSHOT_BUDGET)
        } else {
            (SNAPSHOTS_PER_FRAME, SNAPSHOT_BUDGET)
        };
        let result = (|| -> Result<bool> {
            match &mut save.stage {
                Stage::Quit | Stage::Server => Ok(true),
                Stage::Generation => {
                    if save.started.elapsed() > Duration::from_secs(30) {
                        return Err("chunk generation did not finish before saving".into());
                    }
                    let deadline = Instant::now() + budget;
                    loop {
                        match self
                            .server
                            .streaming
                            .prepare_poll_save(&mut self.common.world, deadline)
                        {
                            sunrise_level::chunk_streamer::SaveStep::Pending => break,
                            sunrise_level::chunk_streamer::SaveStep::Full(chunk) => {
                                self.save_late_full_chunk(chunk);
                            }
                            sunrise_level::chunk_streamer::SaveStep::Protos(protos) => {
                                if self.server.streaming.submit_protos(protos) {
                                    save.stage = Stage::Chunks(
                                        self.common.world.modified.iter().copied().collect(),
                                    );
                                }
                                break;
                            }
                        }
                    }
                    Ok(false)
                }
                Stage::Chunks(keys) => {
                    if let Some(entities) =
                        self.poll_closing_save_chunks_server(keys, per_frame, budget)?
                    {
                        save.stage = Stage::Entities(entities);
                    }
                    Ok(false)
                }
                Stage::Entities(keys) => {
                    if let Some(flush) =
                        self.poll_closing_save_entities_server(keys, per_frame, budget)?
                    {
                        save.stage = Stage::Writes(Some(flush));
                    }
                    Ok(false)
                }
                Stage::Writes(flush) => {
                    let Some(receive) = flush else {
                        *flush = Some(self.poll_closing_save_flush_server()?);
                        return Ok(false);
                    };
                    if !self.finish_player_autosaves(false)? {
                        return Ok(false);
                    }
                    let result = match receive.try_recv() {
                        Ok(result) => result,
                        Err(TryRecvError::Empty) => return Ok(false),
                        Err(TryRecvError::Disconnected) => {
                            Err(std::io::Error::other("the region writer stopped"))
                        }
                    };
                    *flush = None;
                    result?;
                    self.entity_save_complete()?;
                    Ok(true)
                }
            }
        })();
        match result {
            Ok(true) => {
                if let Some(reply) = save.reply.take() {
                    let _ = reply.send(serde_json::json!({ "ok": true }).to_string());
                }
                Ok(true)
            }
            Err(error) => {
                if let Some(reply) = save.reply.take() {
                    let _ =
                        reply.send(serde_json::json!({ "error": error.to_string() }).to_string());
                }
                Err(error)
            }
            Ok(false) => Ok(false),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_begin_server_control_save(
        &mut self,
        all: bool,
        reply: std::sync::mpsc::Sender<String>,
    ) -> Result<ClosingSave> {
        self.begin_server_control_save(all, reply)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn test_poll_server_closing_save(&mut self, save: &mut ClosingSave) -> Result<bool> {
        self.poll_server_closing_save(save)
    }
}
