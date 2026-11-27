//! Non-blocking server chunk load/generation requests (`LevelRef::request_chunk`, #5234).
//!
//! Each active request holds a transient loading-only ticket at `ticket_levels::FULL` (level 33 —
//! accessible, not ticking) via `ChunkStreamer::add_async_request` and `Sources::viewed` (`ticking:
//! false, radius: 0`). Completions are captured on the tick thread during spare tick time
//! (`server_has_time()`) into immutable `ChunkSnapshot`s and queued per `(mod_id, level)` for
//! `LevelRef::drain_chunk_requests`. Pending and undrained requests owned by a disabled mod or a
//! closing level are dropped immediately and their tickets released.
use crate::game::ServerGame;
use mod_api::{ChunkLoadError, ChunkRequestRef, CompletedChunkRequest, DimensionKey, LevelRef};
use std::collections::VecDeque;
use sunrise_core::fast_hash::FxHashMap;
use sunrise_gameplay_common::game::Dim;
use sunrise_gameplay_common::game::native_chunk_snapshot::EngineChunkSnapshot;
use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
use sunrise_gameplay_common::game::native_server::NativeServerGameExt4 as _;
use sunrise_gameplay_common::world_biomes;
use sunrise_level::chunk_streamer::{ChunkStreamer, FullColumn, full_column_ready};
use sunrise_level::world::World;

fn calling_mod() -> Option<&'static str> {
    mod_api::content::crash::current_native_mod()
}

fn mod_is_disabled(mod_id: &str) -> bool {
    mod_api::__private::mod_index(mod_id)
        .is_some_and(|index| !mod_api::__private::mod_index_on(index))
}

fn dim_of(key: DimensionKey) -> Dim {
    match key {
        DimensionKey::OVERWORLD => Dim::Overworld,
        DimensionKey::NETHER => Dim::Nether,
        DimensionKey::END => Dim::End,
        other => Dim::Extra(other),
    }
}

struct PendingChunkRequest {
    id: u64,
    mod_id: &'static str,
    level: LevelRef,
    pos: (i32, i32),
}

/// Per-server state for non-blocking mod chunk requests across all open dimensions.
#[derive(Default)]
pub(in crate::game) struct AsyncChunkRequests {
    next_id: u64,
    last_mods_switched: u64,
    pending: FxHashMap<Dim, VecDeque<PendingChunkRequest>>,
    /// Index of active pending requests by `(dim, request)` -> `(mod_id, pos)` so `cancel` is
    /// O(1) (lazy tombstoning on `pending` queues avoids O(N) linear scan and shift).
    pending_by_handle: FxHashMap<(Dim, ChunkRequestRef), (&'static str, (i32, i32))>,
    /// Active pending request count per `(dim, chunk)` so overlapping requests from multiple mods
    /// (or repeated requests from one mod) release the level-33 ticket only when the last holder
    /// completes or cancels.
    holders: FxHashMap<(Dim, (i32, i32)), usize>,
    /// Count of active (non-tombstoned) pending requests per dimension.
    active_counts: FxHashMap<Dim, usize>,
    completed: FxHashMap<(&'static str, LevelRef), Vec<CompletedChunkRequest>>,
}

impl AsyncChunkRequests {
    pub(in crate::game) fn has_pending_in(&self, dim: Dim) -> bool {
        self.active_counts.get(&dim).copied().unwrap_or(0) > 0
    }

    fn dec_active(&mut self, dim: Dim) {
        if let Some(count) = self.active_counts.get_mut(&dim) {
            if *count <= 1 {
                self.active_counts.remove(&dim);
            } else {
                *count -= 1;
            }
        }
    }

    fn add_holder(&mut self, dim: Dim, pos: (i32, i32)) -> bool {
        let count = self.holders.entry((dim, pos)).or_insert(0);
        *count += 1;
        *count == 1
    }

    fn release_holder(&mut self, dim: Dim, pos: (i32, i32)) -> bool {
        let Some(count) = self.holders.get_mut(&(dim, pos)) else {
            return false;
        };
        if *count <= 1 {
            self.holders.remove(&(dim, pos));
            true
        } else {
            *count -= 1;
            false
        }
    }
}

impl ServerGame<'_> {
    fn native_server_level_dim(&self, level: LevelRef) -> Option<Dim> {
        self.native_level_world(level)?;
        Some(dim_of(level.dimension()))
    }

    fn level_streaming_and_world(&self, dim: Dim) -> Option<(&ChunkStreamer, &World)> {
        if dim == self.common.dim {
            Some((&self.server.streaming, &self.common.world))
        } else {
            self.server
                .levels
                .0
                .parked
                .get(&dim)
                .map(|p| (&p.streaming, &p.world))
        }
    }

    fn level_streaming_mut(&mut self, dim: Dim) -> Option<&mut ChunkStreamer> {
        if dim == self.common.dim {
            Some(&mut self.server.streaming)
        } else {
            self.server
                .levels
                .0
                .parked
                .get_mut(&dim)
                .map(|p| &mut p.streaming)
        }
    }

    /// Whether `dim` has pending non-blocking mod chunk requests that need a streaming pass.
    pub(in crate::game) fn has_async_chunk_requests(&self, dim: Dim) -> bool {
        self.server.async_chunk_requests.has_pending_in(dim)
    }

    /// Enqueue a non-blocking server chunk request (`LevelRef::request_chunk`).
    pub(in crate::game) fn level_request_chunk(
        &mut self,
        level: LevelRef,
        pos: [i32; 2],
    ) -> Result<ChunkRequestRef, ChunkLoadError> {
        let Some(mod_id) = calling_mod() else {
            return Err(ChunkLoadError::Unloaded);
        };
        if mod_is_disabled(mod_id) {
            return Err(ChunkLoadError::Unloaded);
        }
        let Some(dim) = self.native_server_level_dim(level) else {
            return Err(ChunkLoadError::Unloaded);
        };
        let chunk = (pos[0], pos[1]);
        let status = {
            let Some((streaming, world)) = self.level_streaming_and_world(dim) else {
                return Err(ChunkLoadError::Unloaded);
            };
            streaming.check_async_request(world, chunk)
        };
        match status {
            FullColumn::Refused => return Err(ChunkLoadError::Unloaded),
            FullColumn::Failed => return Err(ChunkLoadError::Failed),
            FullColumn::Ready | FullColumn::Pending => {}
        }

        self.server.async_chunk_requests.next_id = self
            .server
            .async_chunk_requests
            .next_id
            .wrapping_add(1)
            .max(1);
        let id = self.server.async_chunk_requests.next_id;
        let first_holder = self.server.async_chunk_requests.add_holder(dim, chunk);
        if first_holder && let Some(streaming) = self.level_streaming_mut(dim) {
            streaming.add_async_request(chunk);
        }
        let handle = mod_api::__private::game::chunk_request_ref(level, id);
        self.server
            .async_chunk_requests
            .pending_by_handle
            .insert((dim, handle), (mod_id, chunk));
        *self
            .server
            .async_chunk_requests
            .active_counts
            .entry(dim)
            .or_insert(0) += 1;
        self.server
            .async_chunk_requests
            .pending
            .entry(dim)
            .or_default()
            .push_back(PendingChunkRequest {
                id,
                mod_id,
                level,
                pos: chunk,
            });
        Ok(handle)
    }

    /// Drain all completed or failed chunk requests owned by the calling mod on `level`.
    pub(in crate::game) fn level_drain_chunk_requests(
        &mut self,
        level: LevelRef,
    ) -> Option<Vec<CompletedChunkRequest>> {
        let mod_id = calling_mod()?;
        self.native_server_level_dim(level)?;
        self.prune_disabled_async_chunk_requests();
        if mod_is_disabled(mod_id) {
            return None;
        }
        Some(
            self.server
                .async_chunk_requests
                .completed
                .remove(&(mod_id, level))
                .unwrap_or_default(),
        )
    }

    /// Cancel a pending or undrained chunk request owned by the calling mod and release its
    /// level-33 ticket immediately if no other request still holds that chunk.
    pub(in crate::game) fn chunk_request_cancel(&mut self, request: ChunkRequestRef) -> bool {
        let Some(mod_id) = calling_mod() else {
            return false;
        };
        let level = request.level();
        let Some(dim) = self.native_server_level_dim(level) else {
            return false;
        };
        self.prune_disabled_async_chunk_requests();
        if let Some(&(owner, pos)) = self
            .server
            .async_chunk_requests
            .pending_by_handle
            .get(&(dim, request))
            && owner == mod_id
        {
            self.server
                .async_chunk_requests
                .pending_by_handle
                .remove(&(dim, request));
            self.server.async_chunk_requests.dec_active(dim);
            if self.server.async_chunk_requests.release_holder(dim, pos) {
                if let Some(streaming) = self.level_streaming_mut(dim) {
                    streaming.remove_async_request(pos);
                }
                if dim == self.common.dim {
                    self.rebuild_chunk_tickets(false);
                }
            }
            if !self.server.async_chunk_requests.has_pending_in(dim) {
                self.server.async_chunk_requests.pending.remove(&dim);
            }
            return true;
        }
        if let Some(done) = self
            .server
            .async_chunk_requests
            .completed
            .get_mut(&(mod_id, level))
            && let Some(idx) = done.iter().position(|entry| entry.request == request)
        {
            done.swap_remove(idx);
            if done.is_empty() {
                self.server
                    .async_chunk_requests
                    .completed
                    .remove(&(mod_id, level));
            }
            return true;
        }
        false
    }

    /// Drop pending and undrained requests owned by any mod that has been disabled, releasing
    /// their level-33 tickets immediately. O(1) when no mod was toggled since the last check.
    pub(in crate::game) fn prune_disabled_async_chunk_requests(&mut self) {
        let switched = mod_api::__private::mods_switched();
        if self.server.async_chunk_requests.last_mods_switched == switched {
            return;
        }
        self.server.async_chunk_requests.last_mods_switched = switched;
        self.prune_async_chunk_requests_where(mod_is_disabled);
    }

    fn prune_async_chunk_requests_where(&mut self, is_disabled: impl Fn(&str) -> bool) {
        if self.server.async_chunk_requests.pending.is_empty()
            && self.server.async_chunk_requests.completed.is_empty()
        {
            return;
        }
        let mut released: Vec<(Dim, (i32, i32))> = Vec::new();
        let mut empty_dims: Vec<Dim> = Vec::new();
        let pending_by_handle = &mut self.server.async_chunk_requests.pending_by_handle;
        let active_counts = &mut self.server.async_chunk_requests.active_counts;
        for (&dim, queue) in &mut self.server.async_chunk_requests.pending {
            queue.retain(|req| {
                let handle = mod_api::__private::game::chunk_request_ref(req.level, req.id);
                if !pending_by_handle.contains_key(&(dim, handle)) {
                    return false;
                }
                if is_disabled(req.mod_id) {
                    pending_by_handle.remove(&(dim, handle));
                    if let Some(count) = active_counts.get_mut(&dim) {
                        if *count <= 1 {
                            active_counts.remove(&dim);
                        } else {
                            *count -= 1;
                        }
                    }
                    released.push((dim, req.pos));
                    false
                } else {
                    true
                }
            });
            if queue.is_empty() {
                empty_dims.push(dim);
            }
        }
        for dim in empty_dims {
            self.server.async_chunk_requests.pending.remove(&dim);
        }
        for (dim, pos) in released {
            if self.server.async_chunk_requests.release_holder(dim, pos)
                && let Some(streaming) = self.level_streaming_mut(dim)
            {
                streaming.remove_async_request(pos);
            }
        }
        self.server
            .async_chunk_requests
            .completed
            .retain(|&(mod_id, _), _| !is_disabled(mod_id));
    }

    /// Drop all pending and undrained requests for a closing level and clear its streaming tickets.
    pub(in crate::game) fn clear_async_chunk_requests_for_dim(&mut self, dim: Dim) {
        self.server.async_chunk_requests.pending.remove(&dim);
        self.server
            .async_chunk_requests
            .pending_by_handle
            .retain(|&(d, _), _| d != dim);
        self.server.async_chunk_requests.active_counts.remove(&dim);
        self.server
            .async_chunk_requests
            .holders
            .retain(|&(holder_dim, _), _| holder_dim != dim);
        let key = dim.key();
        self.server
            .async_chunk_requests
            .completed
            .retain(|&(_, level), _| level.dimension() != key);
        if let Some(streaming) = self.level_streaming_mut(dim) {
            streaming.clear_async_requests();
        }
    }

    /// Drop all pending and undrained async chunk requests across every dimension.
    pub(in crate::game) fn clear_all_async_chunk_requests(&mut self) {
        self.server.async_chunk_requests.pending.clear();
        self.server.async_chunk_requests.pending_by_handle.clear();
        self.server.async_chunk_requests.active_counts.clear();
        self.server.async_chunk_requests.holders.clear();
        self.server.async_chunk_requests.completed.clear();
        self.server.streaming.clear_async_requests();
        for parked in self.server.levels.0.parked.values_mut() {
            parked.streaming.clear_async_requests();
        }
    }

    /// Capture ready (or failed) pending async chunk requests for the level in place (`self.common.dim`)
    /// during spare tick time (`server_has_time()`), releasing their level-33 tickets once
    /// captured so the chunks unload and save through vanilla's ordinary path.
    pub(in crate::game) fn process_async_chunk_requests(&mut self) {
        self.prune_disabled_async_chunk_requests();
        let dim = self.common.dim;
        if !self.server.async_chunk_requests.has_pending_in(dim) {
            return;
        }
        let Some(states) = sunrise_gameplay_common::game::api_values::block_state::current().ok()
        else {
            return;
        };
        let biomes = world_biomes!(self).clone();
        let properties = self.common.pack.assets.properties.clone();

        let Some(mut queue) = self.server.async_chunk_requests.pending.remove(&dim) else {
            return;
        };
        if self.server.streaming.is_aborted() {
            let mut released_any = false;
            while let Some(req) = queue.pop_front() {
                let handle = mod_api::__private::game::chunk_request_ref(req.level, req.id);
                if self
                    .server
                    .async_chunk_requests
                    .pending_by_handle
                    .remove(&(dim, handle))
                    .is_none()
                {
                    continue;
                }
                self.server.async_chunk_requests.dec_active(dim);
                if self
                    .server
                    .async_chunk_requests
                    .release_holder(dim, req.pos)
                {
                    self.server.streaming.remove_async_request(req.pos);
                    released_any = true;
                }
                self.server
                    .async_chunk_requests
                    .completed
                    .entry((req.mod_id, req.level))
                    .or_default()
                    .push(mod_api::__private::game::completed_chunk_request(
                        handle,
                        [req.pos.0, req.pos.1],
                        Err(ChunkLoadError::Unloaded),
                    ));
            }
            if released_any {
                self.rebuild_chunk_tickets(false);
            }
            return;
        }
        let total = queue.len();
        let mut processed = 0usize;
        let mut scanned = 0usize;
        let mut released_chunks: Vec<(i32, i32)> = Vec::new();

        while scanned < total {
            let Some(req) = queue.pop_front() else {
                break;
            };
            scanned += 1;
            let handle = mod_api::__private::game::chunk_request_ref(req.level, req.id);
            if !self
                .server
                .async_chunk_requests
                .pending_by_handle
                .contains_key(&(dim, handle))
            {
                // Cancelled lazily via `chunk_request_cancel`.
                continue;
            }
            if self.server.streaming.full_column_failed(req.pos) {
                self.server
                    .async_chunk_requests
                    .pending_by_handle
                    .remove(&(dim, handle));
                self.server.async_chunk_requests.dec_active(dim);
                released_chunks.push(req.pos);
                self.server
                    .async_chunk_requests
                    .completed
                    .entry((req.mod_id, req.level))
                    .or_default()
                    .push(mod_api::__private::game::completed_chunk_request(
                        handle,
                        [req.pos.0, req.pos.1],
                        Err(ChunkLoadError::Failed),
                    ));
                processed += 1;
                if !self.server_has_time() {
                    break;
                }
            } else if full_column_ready(&self.common.world, req.pos)
                && let Some(column) = self.common.world.chunks.get(&req.pos)
            {
                self.server
                    .async_chunk_requests
                    .pending_by_handle
                    .remove(&(dim, handle));
                self.server.async_chunk_requests.dec_active(dim);
                let has_light = column.lit || column.light_loaded;
                let snapshot = EngineChunkSnapshot::from_chunk(
                    column,
                    self.common.world.shape,
                    states.clone(),
                    biomes.clone(),
                    properties.clone(),
                    has_light,
                );
                released_chunks.push(req.pos);
                self.server
                    .async_chunk_requests
                    .completed
                    .entry((req.mod_id, req.level))
                    .or_default()
                    .push(mod_api::__private::game::completed_chunk_request(
                        handle,
                        [req.pos.0, req.pos.1],
                        Ok(snapshot),
                    ));
                processed += 1;
                if !self.server_has_time() {
                    break;
                }
            } else {
                queue.push_back(req);
                if processed > 0 && scanned.is_multiple_of(64) && !self.server_has_time() {
                    break;
                }
            }
        }

        if !queue.is_empty() {
            self.server.async_chunk_requests.pending.insert(dim, queue);
        }
        let mut released_any = false;
        for pos in released_chunks {
            if self.server.async_chunk_requests.release_holder(dim, pos) {
                self.server.streaming.remove_async_request(pos);
                released_any = true;
            }
        }
        if released_any {
            self.rebuild_chunk_tickets(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::game::native_entity_refs::tests::with_game;
    use mod_api::ChunkLoadError;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use sunrise_gameplay_common::game::Dim;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
    use sunrise_gameplay_common::game::native_hooks::base::GameHooks as _;
    use sunrise_gameplay_common::game::test_fixtures::run_fresh_cases;
    use sunrise_level::chunk::Chunk;

    fn insert_lit_stone_chunk(game: &mut crate::game::ServerGame<'_>, pos: (i32, i32)) {
        let stone_key = mod_api::__private::block_key("minecraft:stone").unwrap();
        let stone_id = sunrise_gameplay_common::game::api_values::default_state(stone_key)
            .unwrap()
            .0;
        let mut column = Chunk::empty(pos.0, pos.1);
        column.lit = true;
        Arc::make_mut(&mut column.sections[4]).set(0, stone_id);
        game.common.world.chunks.insert(pos, Arc::new(column));
    }

    #[test]
    #[ignore = "daily"]
    fn request_chunk_is_non_blocking_level_33_and_respects_spare_time_budget() {
        with_game(|game| {
            game.server.streaming.paused = false;
            let level = game.level("minecraft:overworld").unwrap();
            let stone_key = mod_api::__private::block_key("minecraft:stone").unwrap();

            // 1. Non-blocking request queues level-33 ticket (accessible, NOT ticking).
            let req = {
                let _g = mod_api::__private::enter("mod_a", "test");
                level
                    .request_chunk(game, [12, -8])
                    .expect("request admitted")
            };
            assert!(game.server.streaming.has_async_requests());
            game.rebuild_chunk_tickets(false);
            let levels = game.server.tickets.0.levels();
            assert!(levels.loaded_full(12, -8), "level-33 FULL ticket active");
            assert!(!levels.entity_ticking(12, -8));
            assert!(!levels.block_ticking(12, -8));
            assert!(!levels.random_ticking(12, -8));
            assert!(!levels.loaded_ticking(12, -8));

            // 2. While not ready, drain returns empty Some(vec![]).
            {
                let _g = mod_api::__private::enter("mod_a", "test");
                assert!(level.drain_chunk_requests(game).unwrap().is_empty());
            }

            // 3. Once resident & lit, process captures ChunkSnapshot and releases the ticket.
            insert_lit_stone_chunk(game, (12, -8));
            game.process_async_chunk_requests();
            assert!(!game.server.streaming.has_async_requests());
            {
                let _g = mod_api::__private::enter("mod_a", "test");
                let done = level.drain_chunk_requests(game).unwrap();
                assert_eq!(done.len(), 1);
                assert_eq!(done[0].request, req);
                assert_eq!(done[0].pos, [12, -8]);
                let snap = done[0].result.as_ref().expect("snapshot ok");
                assert_eq!(snap.block_state(0, 0, 0).unwrap().block(), stone_key);
                assert!(snap.has_light());
                // Draining again is empty.
                assert!(level.drain_chunk_requests(game).unwrap().is_empty());
            }

            // 4. Spare-time budget: when deadline already passed, a burst of 5 ready chunks
            // completes exactly 1 per tick (guaranteed forward progress without stalling).
            for x in 20..25 {
                insert_lit_stone_chunk(game, (x, 0));
                let _g = mod_api::__private::enter("mod_a", "test");
                level.request_chunk(game, [x, 0]).unwrap();
            }
            game.server.server_tick_deadline = Some(Instant::now() - Duration::from_millis(1));
            game.process_async_chunk_requests();
            {
                let _g = mod_api::__private::enter("mod_a", "test");
                assert_eq!(level.drain_chunk_requests(game).unwrap().len(), 1);
            }
            game.process_async_chunk_requests();
            {
                let _g = mod_api::__private::enter("mod_a", "test");
                assert_eq!(level.drain_chunk_requests(game).unwrap().len(), 1);
            }
            // Restoring spare time completes the remaining 3 in one tick.
            game.server.server_tick_deadline = None;
            game.process_async_chunk_requests();
            {
                let _g = mod_api::__private::enter("mod_a", "test");
                assert_eq!(level.drain_chunk_requests(game).unwrap().len(), 3);
            }
            assert!(!game.server.streaming.has_async_requests());

            // 5. Out-of-bounds chunk position refused cleanly.
            {
                let _g = mod_api::__private::enter("mod_a", "test");
                assert_eq!(
                    level.request_chunk(game, [i32::MAX, 0]),
                    Err(ChunkLoadError::Unloaded)
                );
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn cancel_isolation_mod_disable_and_level_close_release_tickets() {
        with_game(|game| {
            game.server.streaming.paused = false;
            let level = game.level("minecraft:overworld").unwrap();

            // Per-mod isolation & overlapping chunk refcount: both mod_a and mod_b request [5, 5].
            let req_a = {
                let _g = mod_api::__private::enter("mod_a", "test");
                level.request_chunk(game, [5, 5]).unwrap()
            };
            let req_b = {
                let _g = mod_api::__private::enter("mod_b", "test");
                level.request_chunk(game, [5, 5]).unwrap()
            };
            // mod_b cannot cancel mod_a's request.
            {
                let _g = mod_api::__private::enter("mod_b", "test");
                assert!(!req_a.cancel(game));
            }
            // mod_a cancels its request: ticket stays alive for mod_b!
            {
                let _g = mod_api::__private::enter("mod_a", "test");
                assert!(req_a.cancel(game));
                assert!(!req_a.cancel(game));
            }
            assert!(game.server.streaming.has_async_requests());

            // Complete [5, 5]: only mod_b receives it, not mod_a.
            insert_lit_stone_chunk(game, (5, 5));
            game.process_async_chunk_requests();
            assert!(!game.server.streaming.has_async_requests());
            {
                let _g = mod_api::__private::enter("mod_a", "test");
                assert!(level.drain_chunk_requests(game).unwrap().is_empty());
            }
            // Cancel an undrained completed request before draining: removes it from queue.
            {
                let _g = mod_api::__private::enter("mod_b", "test");
                assert!(req_b.cancel(game));
                assert!(level.drain_chunk_requests(game).unwrap().is_empty());
            }

            // Mod disable drops both pending and undrained completed requests for that mod only
            // and releases its streaming tickets immediately without touching mod_b.
            {
                let _g = mod_api::__private::enter("mod_a", "test");
                level.request_chunk(game, [7, 7]).unwrap();
                level.request_chunk(game, [8, 8]).unwrap();
            }
            {
                let _g = mod_api::__private::enter("mod_b", "test");
                level.request_chunk(game, [10, 10]).unwrap();
            }
            insert_lit_stone_chunk(game, (7, 7));
            game.process_async_chunk_requests();
            // mod_a now has 1 completed ([7, 7]) and 1 pending ([8, 8]); mod_b has 1 pending ([10, 10]).
            game.prune_async_chunk_requests_where(|id| id == "mod_a");
            {
                let _g = mod_api::__private::enter("mod_a", "test");
                assert!(level.drain_chunk_requests(game).unwrap().is_empty());
            }
            let remaining: Vec<_> = game.server.streaming.async_requests().collect();
            assert_eq!(remaining, vec![(10, 10)]);

            // Level close drops pending requests and releases tickets.
            let dim = game.common.dim;
            game.clear_async_chunk_requests_for_dim(dim);
            assert!(!game.server.streaming.has_async_requests());
            assert!(!game.has_async_chunk_requests(Dim::Overworld));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn request_chunk_streams_ungenerated_chunk_to_full_on_active_and_parked_levels_then_unloads() {
        if run_fresh_cases(
            concat!(
                "game::server_side::native_chunk_requests::tests::",
                "request_chunk_streams_ungenerated_chunk_to_full_on_active_and_parked_levels_then_unloads"
            ),
            "SUNRISE_ASYNC_CHUNK_STREAM_CHILD",
            &["stream_e2e"],
        ) {
            return;
        }
        with_game(|game| {
            game.common.options.headless = true;
            game.server.streaming.paused = false;
            game.server.streaming.radius = 0;

            let overworld = game.level("minecraft:overworld").unwrap();
            let target = [28, -19];
            assert!(
                !game
                    .common
                    .world
                    .chunks
                    .contains_key(&(target[0], target[1]))
            );

            // 1. Queue an ungenerated chunk on the active level and drive real `ChunkStreamer` workers.
            let req = {
                let _g = mod_api::__private::enter("mod_a", "test");
                overworld
                    .request_chunk(game, target)
                    .expect("request admitted")
            };
            let mut completed = Vec::new();
            for _ in 0..400 {
                game.tick_server(false).unwrap();
                {
                    let _g = mod_api::__private::enter("mod_a", "test");
                    completed.extend(overworld.drain_chunk_requests(game).unwrap());
                }
                if !completed.is_empty() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(completed.len(), 1, "active-level async request completed");
            assert_eq!(completed[0].request, req);
            assert_eq!(completed[0].pos, target);
            let snap = completed[0].result.as_ref().expect("snapshot ok");
            assert!(snap.has_light(), "streamed chunk is lit");
            assert_eq!(
                snap.light(8, 318, 8).map(|(sky, _)| sky),
                Some(15),
                "open sky light"
            );
            assert!(!game.server.streaming.has_async_requests());

            // Once the transient level-33 ticket is released, an area change triggers the unload
            // sweep and the chunk unloads cleanly.
            game.server.streaming.radius = 1;
            game.tick_server(false).unwrap();
            game.server.streaming.radius = 0;
            game.tick_server(false).unwrap();
            assert!(
                !game
                    .common
                    .world
                    .chunks
                    .contains_key(&(target[0], target[1])),
                "streamed chunk unloads once its level-33 ticket is released"
            );

            // 2. Open and park the Nether, then request a chunk on the parked dimension without
            // switching active levels.
            let back = game.common.dim;
            game.enter_level(Dim::Nether).unwrap();
            game.server.streaming.paused = false;
            game.server.streaming.radius = 0;
            game.enter_level(back).unwrap();

            let nether = game.level("minecraft:the_nether").unwrap();
            let nether_target = [4, 4];
            let nether_req = {
                let _g = mod_api::__private::enter("mod_a", "test");
                nether
                    .request_chunk(game, nether_target)
                    .expect("parked request admitted")
            };
            let mut nether_done = Vec::new();
            for _ in 0..400 {
                game.tick_server(false).unwrap();
                {
                    let _g = mod_api::__private::enter("mod_a", "test");
                    nether_done.extend(nether.drain_chunk_requests(game).unwrap());
                }
                if !nether_done.is_empty() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(nether_done.len(), 1, "parked-level async request completed");
            assert_eq!(nether_done[0].request, nether_req);
            assert_eq!(nether_done[0].pos, nether_target);
            let nether_snap = nether_done[0].result.as_ref().expect("nether snapshot ok");
            assert_eq!(nether_snap.min_y(), 0);
            assert_eq!(nether_snap.section_count(), 16);
            assert!(nether_snap.has_light());
            // Parked level must have released its level-33 ticket immediately so it is no longer
            // stuck in `tickets` when parked.
            assert!(!game.has_async_chunk_requests(Dim::Nether));
            if let Some(parked) = game.server.levels.0.parked.get(&Dim::Nether) {
                assert!(!parked.streaming.has_async_requests());
            }
            game.enter_level(Dim::Nether).unwrap();
            assert!(
                !game
                    .server
                    .tickets
                    .0
                    .levels()
                    .loaded_full(nether_target[0], nether_target[1]),
                "parked dimension cleared released level-33 ticket"
            );
            game.enter_level(back).unwrap();

            // 3. Paused level refuses even an already-resident lit chunk; aborted workers fail any
            // in-flight pending request with `ChunkLoadError::Unloaded` and release its ticket.
            insert_lit_stone_chunk(game, (9, 9));
            game.server.streaming.paused = true;
            {
                let _g = mod_api::__private::enter("mod_a", "test");
                assert_eq!(
                    overworld.request_chunk(game, [9, 9]),
                    Err(ChunkLoadError::Unloaded),
                    "paused level refuses admission even if resident"
                );
            }
            game.server.streaming.paused = false;

            // Client-side (`streaming.remote = true`) refuses admission.
            game.server.streaming.remote = true;
            {
                let _g = mod_api::__private::enter("mod_a", "test");
                assert_eq!(
                    overworld.request_chunk(game, [9, 9]),
                    Err(ChunkLoadError::Unloaded),
                    "remote/client streamer refuses admission"
                );
            }
            game.server.streaming.remote = false;

            // Aborting generation while a request is pending completes it with `Unloaded` and
            // releases its ticket so nothing stays stuck.
            let aborted_req = {
                let _g = mod_api::__private::enter("mod_a", "test");
                overworld
                    .request_chunk(game, [60, 60])
                    .expect("pending request admitted")
            };
            game.server.streaming.abort_generation();
            game.process_async_chunk_requests();
            assert!(!game.server.streaming.has_async_requests());
            {
                let _g = mod_api::__private::enter("mod_a", "test");
                let done = overworld.drain_chunk_requests(game).unwrap();
                assert_eq!(done.len(), 1);
                assert_eq!(done[0].request, aborted_req);
                assert_eq!(
                    done[0].result.as_ref().err(),
                    Some(&ChunkLoadError::Unloaded)
                );
                assert_eq!(
                    overworld.request_chunk(game, [9, 9]),
                    Err(ChunkLoadError::Unloaded),
                    "aborted level refuses new admissions"
                );
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn request_chunk_reports_failed_on_generator_panic_mid_flight_and_at_admission() {
        use mod_api::{
            Hooks, HostSide, LevelTickPostEvent, ModEntry, RuntimeGenerator, RuntimeLevel,
        };
        use sunrise_world::worldgen::mod_generators::{
            ChunkAccess, ModChunkGenerator, ModChunkGenerators, install_mod_chunk_generators,
        };

        if run_fresh_cases(
            concat!(
                "game::server_side::native_chunk_requests::tests::",
                "request_chunk_reports_failed_on_generator_panic_mid_flight_and_at_admission"
            ),
            "SUNRISE_ASYNC_CHUNK_FAILED_CHILD",
            &["failed_chunk"],
        ) {
            return;
        }
        struct PanicsAtOrigin;
        impl ModChunkGenerator for PanicsAtOrigin {
            fn build_terrain(&self, chunk: &mut dyn ChunkAccess) {
                let pos = chunk.pos();
                assert!((pos.x, pos.z) != (0, 0), "generator panics at [0, 0]");
            }
        }
        install_mod_chunk_generators(ModChunkGenerators::from([(
            mod_api::Identifier::parse("asyncfail:terrain").unwrap(),
            Arc::new(PanicsAtOrigin) as Arc<dyn ModChunkGenerator>,
        )]))
        .map_err(|_| "installed twice")
        .unwrap();

        struct Opener;
        #[mod_api::__subscriptions]
        impl Hooks for Opener {
            fn level_tick_post(&self, host: HostSide, _event: &mut LevelTickPostEvent) {
                let HostSide::Server(host) = host else {
                    return;
                };
                let _ = host.runtime_levels().open(
                    "asyncfail:dim",
                    RuntimeLevel::new(
                        "minecraft:overworld",
                        RuntimeGenerator::Mod("asyncfail:terrain".into()),
                        1,
                    ),
                );
            }
        }

        let mods: mod_api::Mods = vec![ModEntry::new("asyncfail", Box::new(Opener))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.common.options.headless = true;
                game.server.streaming.paused = false;
                game.server.streaming.radius = 0;
                let home = game.level("minecraft:overworld").unwrap();
                game.native_level_tick_post(home);
                game.apply_runtime_levels();

                let dim_level = game.level("asyncfail:dim").unwrap();
                let req = {
                    let _g = mod_api::__private::enter("asyncfail", "test");
                    dim_level
                        .request_chunk(game, [0, 0])
                        .expect("queued before failure is known")
                };
                let mut done = Vec::new();
                for _ in 0..400 {
                    game.tick_server(false).unwrap();
                    {
                        let _g = mod_api::__private::enter("asyncfail", "test");
                        done.extend(dim_level.drain_chunk_requests(game).unwrap());
                    }
                    if !done.is_empty() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                assert_eq!(done.len(), 1, "mid-flight generator panic reported");
                assert_eq!(done[0].request, req);
                assert_eq!(done[0].result.as_ref().err(), Some(&ChunkLoadError::Failed));

                // Subsequent admission on the failed chunk fails immediately at admission.
                {
                    let _g = mod_api::__private::enter("asyncfail", "test");
                    assert_eq!(
                        dim_level.request_chunk(game, [0, 0]),
                        Err(ChunkLoadError::Failed)
                    );
                }
            });
        });
    }
}
