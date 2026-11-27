//! Sending a viewed level, in its level pass: `open` with its clock and weather before its first
//! chunk, its chunks in runs inside a plain batch that shares the player's acknowledgement budget,
//! its game time every 20 ticks, its clocks when they jump or change rate and its weather as it
//! changes, the chunks no area covered for [`super::view::LINGER`] ticks forgotten, and `close`
//! once nothing of it is left. Its entities go in the tracking pass (`entities`).
use super::super::{
    hub::{Hub, TickCtx},
    weather,
};
use super::view::View;
use crate::{chunk::Chunk, world::World};
use std::sync::atomic::{AtomicBool, Ordering};
use sunrise_level::chunk_streamer::ChunkState;
use sunrise_protocol::{
    frame,
    packets::{
        Empty,
        play::{self, Clientbound as C},
    },
    viewed_levels::{self, Level, Run},
};

/// The most a `run` payload may carry: a frame's limit less room for its length and compression
/// headers.
const MAX_RUN: usize = frame::MAX_FRAME - 64;
/// The bytes of a run before its first entry: the custom payload's id, the channel, the kind and
/// the level, at most.
const RUN_HEAD: usize = 1 + 1 + viewed_levels::CHANNEL.len() + 1 + 5;

/// The clocks a view's client was last told, as `sync_world` keeps them for a level's players:
/// the Overworld clock's total and rate, and the total and rate of the clock the level follows
/// when it is not the Overworld's.
#[derive(Clone, Copy, Default)]
pub(crate) struct Clock {
    day: i64,
    rate: f32,
    level: Option<(i64, f32)>,
}

impl Clock {
    /// The clocks of the level in place now, at the Overworld clock's `rate`.
    fn now(hub: &Hub, world: &World, rate: f32) -> Self {
        Self {
            day: hub.overworld_day.unwrap_or(world.day_time),
            rate,
            level: (hub.level_clock.as_ref()).map(|&(_, rate)| (world.day_time, rate)),
        }
    }

    /// `ServerClockManager.modifyClock`: whether a clock of `now` jumped from what `self` told
    /// (`time set`, `time add`) past the step one tick at its rate makes, or runs at a new rate.
    fn changed(&self, now: &Self) -> bool {
        let jumped = |last: i64, total: i64, rate: f32| {
            (total - last).abs() > i64::from(rate.ceil() as i32).max(1)
        };
        let level = match (self.level, now.level) {
            (Some((last, last_rate)), Some((total, rate))) => {
                jumped(last, total, rate) || last_rate != rate
            }
            (None, None) => false,
            _ => true,
        };
        jumped(self.day, now.day, now.rate) || self.rate != now.rate || level
    }
}

/// One player's runs of one level: packets in order, a new payload whenever the next would pass
/// a frame's limit.
pub(crate) struct Runs {
    level: Level,
    run: Option<(Run, usize)>,
    payloads: Vec<Vec<u8>>,
}

impl Runs {
    pub(crate) fn new(level: Level) -> Self {
        Self {
            level,
            run: None,
            payloads: Vec::new(),
        }
    }
    /// Adds a play clientbound packet as `Clientbound::encode` writes it, and says whether it
    /// will be sent. One too large to wrap at all, or that the codec refuses, is not; the first
    /// is logged once.
    pub(crate) fn push(&mut self, packet: &[u8]) -> bool {
        let entry = packet.len() + 5;
        if RUN_HEAD + entry > MAX_RUN {
            static LOGGED: AtomicBool = AtomicBool::new(false);
            if !LOGGED.swap(true, Ordering::Relaxed) {
                log::warn!(
                    "[lan] a packet of {} bytes is too large to send in a viewed level",
                    packet.len()
                );
            }
            return false;
        }
        if let Some((run, len)) = self.run.as_mut()
            && *len + entry <= MAX_RUN
        {
            let pushed = run.push(packet).is_ok();
            if pushed {
                *len += entry;
            }
            return pushed;
        }
        self.flush();
        match Run::new(self.level, packet) {
            Ok(run) => {
                self.run = Some((run, RUN_HEAD + entry));
                true
            }
            Err(e) => {
                log::warn!("[lan] not sent in a viewed level: {e}");
                false
            }
        }
    }
    /// The scoped vanilla packet stays byte-identical. Its normal run comes
    /// first in the outer bundle, followed by runs of only section group data.
    pub(crate) fn push_grouped(&mut self, body: &[u8], groups: &[Vec<u8>]) -> bool {
        if groups.is_empty() {
            return self.push(body);
        }
        let mut primary = Self::new(self.level);
        let mut metadata = Self::new(self.level);
        if !primary.push(body) || !groups.iter().all(|packet| metadata.push(packet)) {
            return false;
        }
        let body = primary.finish().pop().expect("one primary packet");
        self.flush();
        self.payloads
            .extend(super::super::hub::block_changes::group_bundle(
                body,
                metadata.finish(),
            ));
        true
    }

    fn flush(&mut self) {
        if let Some((run, _)) = self.run.take() {
            self.payloads.push(run.finish());
        }
    }
    /// The payloads, each the whole body of a custom payload packet.
    pub(crate) fn finish(mut self) -> Vec<Vec<u8>> {
        self.flush();
        self.payloads
    }
}

/// `ChunkStatus.LIGHT` needs its neighbours at `INITIALIZE_LIGHT`: a column whose light layers
/// would leave one out waits for it (as `Hub::next_batch`). So does one still waiting for its
/// postprocessing, which vanilla runs before a chunk is sent (`ChunkMap.prepareTickingChunk`).
fn ready(world: &World, streaming: Option<&ChunkState>, c: &Chunk) -> bool {
    c.lit
        && c.postprocess.is_empty()
        && streaming.is_none_or(|streaming| {
            (-1..=1).all(|dz| {
                (-1..=1).all(|dx| {
                    let (x, z) = (c.x + dx, c.z + dz);
                    world.chunks.contains_key(&(x, z)) || streaming.proto_sections(x, z).is_some()
                })
            })
        })
}

/// A batch's columns of `level` in runs: `taken`'s queue places and `bodies`, their encoded
/// packets, in order. Only the columns the runs took come back, so only they are counted in the
/// batch, taken out of the queue and recorded as sent; one too large stays queued.
pub(crate) fn batch_runs<T>(
    level: Level,
    taken: &[(usize, T)],
    bodies: &[Vec<u8>],
    mut groups: impl FnMut(&T) -> Vec<Vec<u8>>,
) -> (Vec<Vec<u8>>, Vec<usize>) {
    let mut runs = Runs::new(level);
    let sent = taken
        .iter()
        .zip(bodies)
        .filter(|((_, element), body)| runs.push_grouped(body, &groups(element)))
        .map(|((idx, _), _)| *idx)
        .collect();
    (runs.finish(), sent)
}

impl Hub {
    /// The level pass of a level nobody is in but some player's client views (a `Ticking` area
    /// ticked it, or a view pass): its changes go to the viewers, then its viewed-level sending.
    /// False, and nothing done, while nobody views it.
    pub fn viewed_pass(&mut self, ctx: &mut TickCtx) -> bool {
        if !self.viewed_here() {
            return false;
        }
        let world = &mut *ctx.world;
        if world.block_log.is_none() {
            world.block_log = Some(Vec::new());
        }
        if world.connection_group_log.is_none() {
            world.connection_group_log = Some(Default::default());
        }
        if world.light_log.is_none() {
            world.light_log = Some(Default::default());
        }
        self.broadcast_light_changes(world);
        self.broadcast_block_changes(world);
        self.send_viewed(ctx);
        self.track_entities(ctx, None);
        true
    }

    /// Each player's view of the level in place (`Guest::views_level`).
    pub(in crate::lan) fn send_viewed(&mut self, ctx: &TickCtx) {
        if !self.viewed.active() {
            return;
        }
        let dimension = self.config.dimension.clone();
        let active = self.viewed.active.clone();
        for i in 0..self.guests.len() {
            if !self.guests[i].views_level(&dimension) {
                continue;
            }
            if self.open_view(i, ctx) && self.update_viewed(i) {
                self.send_view_batch(i, ctx.world, ctx.streaming);
                self.send_view_state(i, ctx.world, ctx.clock_rate);
            }
            self.guests.update(i, |g| g.viewed.recount(&active));
        }
    }

    /// Guest `i`'s view of the level in place: its queue made again when the areas changed, the
    /// chunks no area covered for [`super::view::LINGER`] ticks forgotten, and `close` once
    /// nothing of it is left. False once closed.
    fn update_viewed(&mut self, i: usize) -> bool {
        let ticks = self.ticks;
        let dimension = &self.config.dimension;
        self.guests.update(i, |g| {
            let v = &mut g.viewed;
            let areas: Vec<_> = v.areas_in(dimension).cloned().collect();
            let Some(view) = v.views.get_mut(dimension) else {
                return false;
            };
            if view.dirty {
                view.requeue(&areas.iter().collect::<Vec<_>>(), ticks);
            }
            let mut runs = Runs::new(view.level);
            for (x, z) in view.due(ticks) {
                runs.push(&C::ForgetLevelChunk(play::ForgetLevelChunk { pos: [x, z] }).encode());
            }
            g.sender.send_many(runs.finish());
            if areas.is_empty() && view.is_empty() {
                let level = view.level;
                v.views.remove(dimension);
                g.sender.send(viewed_levels::close(level));
                return false;
            }
            true
        })
    }

    /// Opens the level in place on guest `i`'s client if it is not open yet: `open`, then its
    /// clock and weather. False when its client can view no more levels, or cannot name this one
    /// (`Viewed::name_new`).
    fn open_view(&mut self, i: usize, ctx: &TickCtx) -> bool {
        let dimension = &self.config.dimension;
        let g = &self.guests[i];
        if g.viewed.views.contains_key(dimension) {
            return true;
        }
        if g.viewed.views.len() >= usize::from(g.sender.viewed_levels.max_levels) {
            return false;
        }
        let name = |g: &mut super::super::guest::Guest| {
            let answer = g.sender.viewed_levels;
            g.viewed.name_new(dimension, answer)
        };
        let Some(level) = self.guests.update(i, name) else {
            return false;
        };
        let spawn = self.spawn_info(i, 0, ctx.sea_level);
        let mut runs = Runs::new(level);
        runs.push(&self.time_packet(ctx.world, ctx.clock_rate).encode());
        let state = weather::state(ctx.world, dimension);
        weather::send_updates(None, state, |packet| {
            runs.push(&packet.encode());
        });
        let clock = Clock::now(self, ctx.world, ctx.clock_rate);
        let dimension = dimension.clone();
        let caps = self.viewed.caps;
        self.guests.update(i, |g| {
            let mut view = View::new(level);
            view.weather = state;
            view.clock = clock;
            g.viewed.views.insert(dimension, view);
            let columns = caps.for_client(g.sender.viewed_levels).loaded_per_player;
            g.sender.send(viewed_levels::open(level, columns, &spawn));
            g.sender.send_many(runs.finish());
        });
        true
    }

    /// The next batch of the level in place guest `i` views: its ready columns nearest to an
    /// area's centre first, as many as the batch quota its own level left.
    fn send_view_batch(&mut self, i: usize, world: &World, streaming: Option<&ChunkState>) {
        let g = &self.guests[i];
        if self.ticks <= g.placed_at + 1
            || g.unacknowledged >= g.max_unacknowledged
            || g.batch_quota < 1.0
        {
            return;
        }
        let Some(view) = g.viewed.views.get(&self.config.dimension) else {
            return;
        };
        // The client keeps at most its cap of columns of a viewed level (the cap its `open` said,
        // or before revision 4 the fixed one): past it, a `Ticking` area's square beside `Loaded`
        // ones is kept but not sent.
        let cap = self.viewed.caps.for_client(g.sender.viewed_levels);
        let room = (cap.loaded_per_player as usize).saturating_sub(view.sent.len());
        let taken: Vec<(usize, &Chunk)> = view
            .queue
            .iter()
            .enumerate()
            .filter_map(|(idx, key)| Some((idx, &**world.chunks.get(key)?)))
            .filter(|(_, c)| ready(world, streaming, c))
            .take((g.batch_quota.floor() as usize).min(room))
            .collect();
        if taken.is_empty() {
            return;
        }
        let chunks: Vec<&Chunk> = taken.iter().map(|&(_, c)| c).collect();
        self.encoder.warm(&chunks);
        let encoded = self.encoder.bodies_with(&chunks, world, &|x, z| {
            streaming.and_then(|s| s.proto_sections(x, z))
        });
        let mut ok_taken = Vec::with_capacity(taken.len());
        let mut bodies = Vec::with_capacity(encoded.len());
        let mut failed = Vec::new();
        for ((idx, chunk), result) in taken.into_iter().zip(encoded) {
            match result {
                Ok(body) => {
                    ok_taken.push((idx, chunk));
                    bodies.push(body);
                }
                Err(error) => {
                    log::warn!(
                        "[lan] viewed chunk ({}, {}) could not be encoded: {error}",
                        chunk.x,
                        chunk.z
                    );
                    failed.push(idx);
                }
            }
        }
        let sync_groups = g.negotiated_connection_groups();
        let mut grouped = false;
        let (payloads, sent) = batch_runs(view.level, &ok_taken, &bodies, |chunk| {
            if sync_groups && !chunk.connection_groups.is_empty() {
                grouped = true;
                super::super::hub::block_changes::encode_chunk_connection_groups(chunk)
            } else {
                Vec::new()
            }
        });
        if sent.is_empty() && failed.is_empty() {
            return;
        }
        let dimension = self.config.dimension.clone();
        let size = sent.len();
        self.guests.update(i, |g| {
            if size > 0 {
                g.batch_quota -= size as f32;
                g.send(&C::ChunkBatchStart(Empty));
                if grouped {
                    g.sender.send_batch(payloads);
                } else {
                    g.sender.send_many(payloads);
                }
                g.send(&C::ChunkBatchFinished(play::ChunkBatchFinished {
                    size: size as i32,
                }));
                g.unacknowledged += 1;
            }
            if let Some(view) = g.viewed.views.get_mut(&dimension) {
                let mut remove: Vec<(usize, bool)> = sent
                    .into_iter()
                    .map(|idx| (idx, true))
                    .chain(failed.into_iter().map(|idx| (idx, false)))
                    .collect();
                remove.sort_unstable_by_key(|&(idx, _)| idx);
                for (idx, was_sent) in remove.into_iter().rev() {
                    let key = view.queue.remove(idx);
                    if was_sent {
                        view.sent.insert(key);
                    }
                }
            }
        });
    }

    /// The level's game time every 20 ticks (`MinecraftServer.forceGameTimeSynchronization`, as
    /// `sync_world`), every clock when one jumped or changed rate (`time set`, `time add`, a
    /// clock paused or sped up: `ServerClockManager.modifyClock`), and its weather as it changes.
    fn send_view_state(&mut self, i: usize, world: &World, clock_rate: f32) {
        let dimension = self.config.dimension.clone();
        let sync_time = self.ticks.is_multiple_of(20);
        let clock = Clock::now(self, world, clock_rate);
        let state = weather::state(world, &dimension);
        let clocks = (self.guests[i].viewed.view(&dimension))
            .filter(|view| view.clock.changed(&clock))
            .map(|_| self.time_packet(world, clock_rate).encode());
        self.guests.update(i, |g| {
            let Some(view) = g.viewed.views.get_mut(&dimension) else {
                return;
            };
            let mut runs = Runs::new(view.level);
            view.clock = clock;
            if let Some(clocks) = &clocks {
                runs.push(clocks);
            } else if sync_time {
                runs.push(
                    &C::SetTime(play::SetTime {
                        game_time: world.game_time,
                        clocks: Vec::new(),
                    })
                    .encode(),
                );
            }
            weather::send_updates(Some(view.weather), state, |packet| {
                runs.push(&packet.encode());
            });
            view.weather = state;
            g.sender.send_many(runs.finish());
        });
    }

    /// Sends `packet` to the players whose client views the level in place and holds chunk
    /// `key` of it, wrapped (block events and level events near a viewed chunk).
    pub(in crate::lan) fn send_viewers_of(&self, key: (i32, i32), packet: &C) {
        self.send_to_views(|view| view.sent.contains(&key), packet);
    }

    /// Sends `packet`, wrapped, to the players whose client views the level in place in a view
    /// `holds` picks.
    pub(super) fn send_to_views(&self, holds: impl Fn(&View) -> bool, packet: &C) {
        if !self.viewed.active() {
            return;
        }
        let mut body = None;
        for g in self.guests.iter() {
            if let Some(view) = self.view_here(g).filter(|v| holds(v)) {
                let body = body.get_or_insert_with(|| packet.encode());
                let mut runs = Runs::new(view.level);
                runs.push(body);
                g.sender.send_many(runs.finish());
            }
        }
    }
}
