//! `ChunkHolder.broadcastChanges`: the light and blocks that changed in the level in place, to
//! the players sent those chunks, and, wrapped with the level, to the players whose client views
//! it (`viewed_levels`).
use super::{Guest, Hub};
use crate::{
    chunk::Chunk,
    fast_hash::FxHashSet,
    lan::viewed_levels::send::Runs,
    world::{ChunkResend, World},
};
use std::collections::BTreeMap;
use sunrise_protocol::{
    connection_groups::{self, Cell, Entry, Section},
    packets::{
        Empty,
        custom_payload::CustomPayload,
        play::{self, Clientbound as C},
    },
    viewed_levels::Level,
};

fn encode_groups(section: Section) -> Vec<u8> {
    C::CustomPayload(CustomPayload {
        channel: connection_groups::CHANNEL.into(),
        data: section
            .encode()
            .expect("at most one group per section cell"),
    })
    .encode()
}

/// `LevelChunkAuxiliaryLightManager.sendLightDataTo`: the vanilla packet is
/// first, followed by private data, only when this peer negotiated the channel.
pub(in crate::lan) fn group_bundle(
    body: Vec<u8>,
    groups: Vec<Vec<u8>>,
) -> impl Iterator<Item = Vec<u8>> {
    let delimiter = (!groups.is_empty()).then(|| C::BundleDelimiter(Empty).encode());
    delimiter
        .clone()
        .into_iter()
        .chain([body])
        .chain(groups)
        .chain(delimiter)
}

pub(in crate::lan) fn encode_chunk_connection_groups(chunk: &Chunk) -> Vec<Vec<u8>> {
    let mut sections: BTreeMap<i32, Vec<Entry>> = BTreeMap::new();
    for (&cell, &group) in &chunk.connection_groups {
        let (x, y, z) = chunk.dynamic_cell_coords(cell);
        sections.entry(y >> 4).or_default().push(Entry {
            cell: Cell::of([x.into(), y, z.into()]),
            group: Some(group),
        });
    }
    sections
        .into_iter()
        .map(|(y, entries)| {
            encode_groups(Section {
                pos: [chunk.x, y, chunk.z],
                entries,
            })
        })
        .collect()
}

type Block = (i32, i32, i32);

fn block_packet(world: &World, section: Block, positions: &[Block]) -> C {
    if let [(x, y, z)] = *positions {
        C::BlockUpdate(play::BlockUpdate {
            pos: [x, y, z],
            state: world.block(x, y, z).0 as i32,
        })
    } else {
        C::SectionBlocksUpdate(play::SectionBlocksUpdate {
            section: [section.0, section.1, section.2],
            changes: positions
                .iter()
                .map(|&(x, y, z)| {
                    i64::from(world.block(x, y, z).0) << 12
                        | i64::from(x & 15) << 8
                        | i64::from(z & 15) << 4
                        | i64::from(y & 15)
                })
                .collect(),
        })
    }
}

impl Hub {
    /// Whether any player currently holds `key` in `dimension` for an explicit resend:
    /// a tracked chunk in their own level (`Guest::tracks_chunk`) or a sent chunk in a viewed one.
    pub fn has_chunk_resend_target(&self, dimension: &str, key: (i32, i32)) -> bool {
        self.resend_holders(dimension, key).next().is_some()
    }

    /// The players eligible for an explicit chunk resend of `key` in `dimension`:
    /// same-level players for whom `Guest::tracks_chunk` holds (`ChunkMap.isChunkTracked`),
    /// plus players viewing `dimension` who have been sent `key`.
    fn resend_holders<'a>(
        &'a self,
        dimension: &'a str,
        key: (i32, i32),
    ) -> impl Iterator<Item = (usize, Option<Level>)> + 'a {
        let viewed = self.viewed.active();
        self.guests
            .simulation_indexed_iter()
            .filter_map(move |(i, g)| {
                if g.watches_level(dimension) {
                    g.tracks_chunk(dimension, key).then_some((i, None))
                } else if viewed && g.views_level(dimension) {
                    g.viewed
                        .view(dimension)
                        .filter(|v| v.sent.contains(&key))
                        .map(|v| (i, Some(v.level)))
                } else {
                    None
                }
            })
    }

    /// The players sent chunk `key` in `dimension` (`ChunkMap.getPlayers`): those in it (None),
    /// and those whose client views it, with the level to wrap with and the chunks each was sent.
    fn chunk_holders<'a>(
        &'a self,
        dimension: &'a str,
        key: (i32, i32),
    ) -> impl Iterator<Item = (usize, Option<Level>, &'a FxHashSet<(i32, i32)>)> + 'a {
        let viewed = self.viewed.active();
        self.guests
            .simulation_indexed_iter()
            .filter_map(move |(i, g)| {
                if g.watches_level(dimension) {
                    g.sent_chunks
                        .contains(&key)
                        .then_some((i, None, &g.sent_chunks))
                } else if viewed && g.views_level(dimension) {
                    g.viewed
                        .view(dimension)
                        .filter(|v| v.sent.contains(&key))
                        .map(|v| (i, Some(v.level), &v.sent))
                } else {
                    None
                }
            })
    }

    /// Send a world-state correction directly to one guest, with its group at the same
    /// receive boundary. This does not publish the replacement to its other holders.
    pub fn send_authoritative_block(&self, conn: u32, world: &World, pos: [i32; 3]) {
        let Some(guest) = self.guest(conn) else {
            return;
        };
        let [x, y, z] = pos;
        let block = block_packet(world, (x >> 4, y >> 4, z >> 4), &[(x, y, z)]);
        if guest.negotiated_connection_groups() && guest.sent_chunks.contains(&(x >> 4, z >> 4)) {
            let group = world.connection_group(x, y, z);
            if group.is_some()
                || world
                    .connection_group_log
                    .as_ref()
                    .is_some_and(|log| log.contains(&(x, y, z)))
            {
                let payload = encode_groups(Section {
                    pos: [x >> 4, y >> 4, z >> 4],
                    entries: vec![Entry {
                        cell: Cell::of(pos),
                        group,
                    }],
                });
                guest
                    .sender
                    .send_batch(group_bundle(block.encode(), vec![payload]));
                return;
            }
        }
        guest.send(&block);
    }

    /// Plain holders take the original body, with the same scoped run if needed.
    /// For a refused scoped resend, forget the stale column and let the view's
    /// usual requeue pick it up. Do not publish just one half of a pair.
    fn send_to_holders_grouped(
        &mut self,
        holders: &[(usize, Option<Level>)],
        body: &[u8],
        groups: &[Vec<u8>],
        resend: Option<(i32, i32)>,
    ) {
        let dimension = &self.config.dimension;
        for &(i, level) in holders {
            self.guests.update(i, |guest| {
                let groups = if guest.negotiated_connection_groups() {
                    groups
                } else {
                    &[]
                };
                if let Some(level) = level {
                    let mut runs = Runs::new(level);
                    if runs.push_grouped(body, groups) {
                        if groups.is_empty() {
                            guest.sender.send_many(runs.finish());
                        } else {
                            guest.sender.send_batch(runs.finish());
                        }
                    } else if let Some(key @ (x, z)) = resend
                        && let Some(view) = guest.viewed.views.get_mut(dimension)
                    {
                        view.sent.remove(&key);
                        view.linger.remove(&key);
                        view.dirty = true;
                        let mut runs = Runs::new(level);
                        runs.push(
                            &C::ForgetLevelChunk(play::ForgetLevelChunk { pos: [x, z] }).encode(),
                        );
                        guest.sender.send_many(runs.finish());
                    }
                } else if groups.is_empty() {
                    guest.sender.send(body.to_vec());
                } else {
                    guest
                        .sender
                        .send_batch(group_bundle(body.to_vec(), groups.to_vec()));
                }
            });
        }
    }

    fn send_to_holders(&mut self, holders: &[(usize, Option<Level>)], body: &[u8]) {
        self.send_to_holders_grouped(holders, body, &[], None);
    }

    /// `ChunkHolder.broadcastChanges`' light part: the light sections that
    /// changed in a column go to the players for whom it is on the border of
    /// what they track (`ChunkMap.isChunkOnTrackedBorder`); the others light
    /// the blocks they are sent themselves.
    pub(in crate::lan) fn broadcast_light_changes(&mut self, world: &mut World) {
        let Some(log) = world.light_log.as_mut() else {
            return;
        };
        if log.is_empty() {
            return;
        }
        let mut changes: Vec<_> = log.drain().collect();
        // One entry per column, so the order is the columns'.
        changes.sort_unstable_by_key(|&(column, _)| column);
        for ((x, z), sections) in changes {
            let viewers: Vec<(usize, Option<Level>)> = self
                .chunk_holders(&self.config.dimension, (x, z))
                .filter(|(_, _, sent)| {
                    (-1..=1).any(|dz| (-1..=1).any(|dx| !sent.contains(&(x + dx, z + dz))))
                })
                .map(|(i, level, _)| (i, level))
                .collect();
            if viewers.is_empty() {
                continue;
            }
            let Some(chunk) = world.chunks.get(&(x, z)) else {
                continue;
            };
            let body = self.encoder.light_update(chunk, sections);
            self.send_to_holders(&viewers, &body);
        }
    }

    pub(in crate::lan) fn broadcast_block_changes(&mut self, world: &mut World) {
        // Chunks changed in bulk (`World::edit_runs`) go whole to those who see them: one packet each, where a
        // change of each block would be thousands (the client lights and meshes them once, on its workers).
        let mut bulk = Vec::new();
        if !world.bulk_log.is_empty() {
            bulk.extend(world.bulk_log.drain());
            bulk.sort_unstable();
            // The single-player window's far terrain draws a column it isn't sent as the window last saw it: it is
            // handed the column to summarize again (`handoff::changed`).
            let unsent = |g: &Guest, key: &(i32, i32)| {
                self.here(g)
                    && g.address() == crate::lan::conn::MEMORY_ADDRESS
                    && !g.sent_chunks.contains(key)
            };
            for key in &bulk {
                if let Some(chunk) = world.chunks.get(key)
                    && self.guests.iter().any(|g| unsent(g, key))
                {
                    crate::lan::handoff::changed(chunk);
                }
            }
        }
        let mut resends = Vec::new();
        if !world.chunk_resends.is_empty() {
            // Release only resends whose preceding light completion marker reached `Ready`
            // (`ChunkMap.waitForLightBeforeSending` / `ThreadedLevelLightEngine.waitForPendingTasks`).
            world.chunk_resends.retain(|&key, state| {
                if !world.chunks.contains_key(&key) {
                    return false;
                }
                match *state {
                    ChunkResend::Ready => {
                        resends.push(key);
                        false
                    }
                    ChunkResend::WaitingEdits { .. } | ChunkResend::WaitingLight { .. } => {
                        self.has_chunk_resend_target(&self.config.dimension, key)
                    }
                }
            });
            resends.sort_unstable();
        }
        let mut sent_whole = Vec::new();
        if !bulk.is_empty() || !resends.is_empty() {
            let mut whole = bulk.clone();
            whole.extend_from_slice(&resends);
            whole.sort_unstable();
            whole.dedup();
            let columns: Vec<&Chunk> = whole
                .iter()
                .filter_map(|k| world.chunks.get(k).map(|c| &**c))
                .collect();
            self.encoder.warm(&columns);
            for chunk in columns {
                let key = (chunk.x, chunk.z);
                let is_bulk = bulk.binary_search(&key).is_ok();
                let holders: Vec<_> = if is_bulk {
                    self.holders(key)
                } else {
                    self.resend_holders(&self.config.dimension, key).collect()
                };
                if holders.is_empty() {
                    continue;
                }
                let body = match self.encoder.body_in(chunk, world) {
                    Ok(body) => body,
                    Err(error) => {
                        log::warn!(
                            "[lan] bulk chunk ({}, {}) could not be encoded: {error}",
                            chunk.x,
                            chunk.z
                        );
                        continue;
                    }
                };
                let groups =
                    if !chunk.connection_groups.is_empty() && self.has_group_holders(&holders) {
                        encode_chunk_connection_groups(chunk)
                    } else {
                        Vec::new()
                    };
                self.send_to_holders_grouped(&holders, &body, &groups, Some(key));
                // An explicit resend can omit a still-held column outside the current view.
                // Do not discard ordinary updates unless all of their holders were covered.
                if is_bulk
                    || self
                        .chunk_holders(&self.config.dimension, key)
                        .all(|(i, level, _)| holders.contains(&(i, level)))
                {
                    sent_whole.push(key);
                }
            }
        }
        let mut group_changes = world
            .connection_group_log
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default();
        let mut log = world
            .block_log
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default();
        if !sent_whole.is_empty() {
            // (what changed in them one at a time went with them, but for one-shot fields: a chunk has none)
            log.retain(|p| {
                sent_whole.binary_search(&(p.0 >> 4, p.2 >> 4)).is_err()
                    || matches!(
                        world.block_entities.get(p),
                        Some(crate::containers::BlockEntity::Mod(m)) if m.has_client_update_fields()
                    )
            });
        }
        if !log.is_empty() {
            log.sort_unstable();
            log.dedup();
            // Group by section.
            let mut sections: std::collections::BTreeMap<(i32, i32, i32), Vec<(i32, i32, i32)>> =
                Default::default();
            for p in log {
                sections
                    .entry((p.0 >> 4, p.1 >> 4, p.2 >> 4))
                    .or_default()
                    .push(p);
            }
            for ((sx, sy, sz), positions) in sections {
                let key = (sx, sz);
                let holders = self.holders(key);
                let mut entries = Vec::new();
                if !group_changes.is_empty()
                    || world
                        .chunks
                        .get(&key)
                        .is_some_and(|c| !c.connection_groups.is_empty())
                {
                    let negotiated = self.has_group_holders(&holders);
                    for &(x, y, z) in &positions {
                        let changed = group_changes.remove(&(x, y, z));
                        if negotiated {
                            let group = world.connection_group(x, y, z);
                            if changed || group.is_some() {
                                entries.push(Entry {
                                    cell: Cell::of([x, y, z]),
                                    group,
                                });
                            }
                        }
                    }
                }
                if holders.is_empty() {
                    continue;
                }
                let body = block_packet(world, (sx, sy, sz), &positions).encode();
                let groups = (!entries.is_empty()).then(|| {
                    encode_groups(Section {
                        pos: [sx, sy, sz],
                        entries,
                    })
                });
                self.send_to_holders_grouped(&holders, &body, groups.as_slice(), None);
                // Block entities with data the client shows.
                for &(x, y, z) in &positions {
                    let state = world.block(x, y, z).0;
                    if !self
                        .encoder
                        .entity_matches(state, world.block_entities.get(&(x, y, z)))
                    {
                        continue;
                    }
                    // A decorated pot is no block entity: its tag is the chunk's saved one.
                    let state = world.block(x, y, z).0;
                    let tag = match world.block_entities.get(&(x, y, z)) {
                        Some(entity) => self.encoder.change_tag((x, y, z), state, entity),
                        None => world
                            .chunks
                            .get(&(x >> 4, z >> 4))
                            .and_then(|c| crate::lan::chunks::pot_tag(c, (x, y, z))),
                    };
                    let Some(tag) = tag else {
                        continue;
                    };
                    let Some(kind) = self.encoder.entity_type(state) else {
                        continue;
                    };
                    match C::BlockEntityData(play::BlockEntityData {
                        pos: [x, y, z],
                        kind,
                        tag,
                    })
                    .try_encode()
                    {
                        Ok(body) => self.send_to_holders(&holders, &body),
                        Err(error) => log::warn!(
                            "[lan] block entity update at ({x}, {y}, {z}) could not be encoded: {error}"
                        ),
                    }
                }
            }
        }
        if !group_changes.is_empty() {
            // A block scheduled for the next broadcast is still held by the
            // existing block log. Leave its group in the same dirty set.
            let held: crate::fast_hash::FxHashSet<_> =
                world.block_log_held.iter().copied().collect();
            let mut sections: BTreeMap<Block, Vec<Block>> = BTreeMap::new();
            group_changes.retain(|&(x, y, z)| {
                let key = (x >> 4, z >> 4);
                if sent_whole.binary_search(&key).is_ok() || !world.is_loaded_at(x, y, z) {
                    return false;
                }
                if held.contains(&(x, y, z)) {
                    return true;
                }
                sections
                    .entry((key.0, y >> 4, key.1))
                    .or_default()
                    .push((x, y, z));
                false
            });
            for (section @ (x, y, z), mut positions) in sections {
                let mut holders = self.holders((x, z));
                holders.retain(|&(i, _)| self.guests[i].negotiated_connection_groups());
                if holders.is_empty() {
                    continue;
                }
                positions.sort_unstable();
                // Without a scheduled vanilla update, send the current state
                // rather than risk decorating an older block on the client.
                let body = block_packet(world, section, &positions).encode();
                let groups = encode_groups(Section {
                    pos: [x, y, z],
                    entries: positions
                        .into_iter()
                        .map(|(x, y, z)| Entry {
                            cell: Cell::of([x, y, z]),
                            group: world.connection_group(x, y, z),
                        })
                        .collect(),
                });
                self.send_to_holders_grouped(&holders, &body, &[groups], None);
            }
        }
        if let Some(log) = &mut world.connection_group_log {
            *log = group_changes;
        }
    }

    fn has_group_holders(&self, holders: &[(usize, Option<Level>)]) -> bool {
        holders
            .iter()
            .any(|&(i, _)| self.guests[i].negotiated_connection_groups())
    }

    /// [`Self::chunk_holders`] without the chunks they were sent.
    fn holders(&self, key: (i32, i32)) -> Vec<(usize, Option<Level>)> {
        self.chunk_holders(&self.config.dimension, key)
            .map(|(i, level, _)| (i, level))
            .collect()
    }
}
