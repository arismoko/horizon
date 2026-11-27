//! `sunrise-ctl memory`: what holds the game's memory, by category. The
//! process's resident set and the allocator's committed bytes say how much;
//! the census of the columns, the server's entities, players and chunk send
//! caches, the renderer's bookkeeping and the GPU allocator's resources by
//! label say where it goes. `unattributed_mb` is what the named parts leave.
use crate::game::ServerGame;
use std::collections::BTreeMap;
use std::mem::size_of;
use std::sync::Arc;
use sunrise_level::chunk::Chunk;
use sunrise_sim::lan::hub::Hub;
#[cfg(test)]
use sunrise_sim::server_perf::MemoryCost;

pub(in crate::game) use sunrise_gameplay_common::game::memory_common::*;

pub(in crate::game::server_side) fn vec_bytes<T>(v: &Vec<T>) -> usize {
    v.capacity() * size_of::<T>()
}

pub(in crate::game::server_side) fn string_bytes(s: &Option<String>) -> usize {
    s.as_ref().map_or(0, String::capacity)
}

/// The `limit` rows with the biggest key first, ties by name.
pub(in crate::game::server_side) fn biggest<K: Copy + Ord, V>(
    mut rows: Vec<(String, K, V)>,
    limit: usize,
) -> Vec<(String, K, V)> {
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    rows.truncate(limit);
    rows
}

/// The hub's players: what waits in each one's send queue and what the hub tracks for it.
pub(in crate::game::server_side) fn player_census(hub: &Hub) -> Players {
    let set = |len: usize| len * (size_of::<(i32, i32)>() + 1);
    let mut rows = Vec::new();
    let mut bytes = 0usize;
    for g in hub.guests.iter().filter(|g| g.participates_online()) {
        let (packets, queued) = g.sender.queued();
        bytes += queued
            + set(g.sent_chunks.capacity())
            + vec_bytes(&g.chunk_queue)
            + g.tracker.heap_bytes();
        rows.push((
            g.name.clone(),
            [
                queued,
                packets,
                g.chunk_queue.len(),
                g.sent_chunks.len(),
                g.tracker.len(),
            ],
        ));
    }
    let top = |by: usize| {
        let ranked: Vec<_> = rows
            .iter()
            .map(|(name, v)| (name.clone(), v[by], v[0]))
            .collect();
        biggest(ranked, TOP_PLAYERS)
    };
    Players {
        count: hub
            .guests
            .iter()
            .filter(|g| g.participates_online())
            .count(),
        total_mb: mb(bytes),
        send_queue: top(0)
            .into_iter()
            .map(|(name, queued, _)| {
                let packets = rows.iter().find(|r| r.0 == name).map_or(0, |r| r.1[1]);
                (name, packets, mb(queued))
            })
            .collect(),
        chunks_waiting: top(2).into_iter().map(|(n, v, _)| (n, v)).collect(),
        chunks_sent: top(3).into_iter().map(|(n, v, _)| (n, v)).collect(),
        tracked_entities: top(4).into_iter().map(|(n, v, _)| (n, v)).collect(),
        bytes,
    }
}

pub(in crate::game::server_side) fn chunk_send_census(hub: &Hub) -> ChunkSend {
    let [sections, heights] = hub.encoder.cache_census();
    let frames = sunrise_sim::lan::conn::shared_frame_census();
    let cached = |(entries, bytes): (usize, usize)| Cached {
        entries,
        size_mb: mb(bytes),
    };
    let bytes = sections.1 + heights.1 + frames.1;
    ChunkSend {
        total_mb: mb(bytes),
        sections: cached(sections),
        heightmaps: cached(heights),
        frames: cached(frames),
        bytes,
    }
}

impl ServerGame<'_> {
    /// Copy one Arc per column, not its sections or block data.
    pub(in crate::game) fn census_chunks(&self) -> Vec<Arc<Chunk>> {
        std::iter::once(&self.common.world)
            .chain(self.server.levels.0.parked_worlds())
            .flat_map(|w| w.chunks.values().cloned())
            .collect()
    }

    /// The entities of every level by type; a record's bytes are its struct and the buffers its fields own.
    pub(in crate::game::server_side) fn entity_census(&self) -> Entities {
        let record = [
            size_of::<sunrise_sim::entities::ItemEntity>(),
            size_of::<sunrise_sim::entities::ExperienceOrb>(),
            size_of::<sunrise_sim::projectiles::Projectile>(),
            size_of::<super::minecarts::Minecart>(),
            size_of::<sunrise_gameplay_common::game::boats_common::Boat>(),
        ];
        let mut by_type: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        let mut level = |mobs: &[sunrise_sim::mobs::Mob], loose: [usize; 5]| {
            for m in mobs.iter().filter(|m| !m.removed) {
                let bytes = size_of::<sunrise_sim::mobs::Mob>()
                    + size_of_val(&*m.ai)
                    + vec_bytes(&m.outbox)
                    + string_bytes(&m.custom_name)
                    + string_bytes(&m.standing_on)
                    + string_bytes(&m.skin)
                    + string_bytes(&m.cape);
                let row = by_type.entry(m.kind.id.clone()).or_default();
                row.0 += 1;
                row.1 += bytes;
            }
            let loose = super::dedicated::report::LOOSE_ENTITIES
                .into_iter()
                .zip(loose)
                .zip(record);
            for ((id, n), size) in loose.filter(|((_, n), _)| *n > 0) {
                let row = by_type.entry(id.to_owned()).or_default();
                row.0 += n;
                row.1 += n * size;
            }
        };
        level(
            &self.common.mobs,
            [
                self.common.dropped.len(),
                self.common.orbs.len(),
                self.common.creatures.projectiles.len(),
                self.common.carts.len(),
                self.common.boats.len(),
            ],
        );
        for dim in self.server.levels.0.parked_dims() {
            if let Some(mobs) = self.server.levels.0.parked_mobs(dim) {
                let loose = self
                    .server
                    .levels
                    .0
                    .parked_entity_counts(dim)
                    .unwrap_or_default();
                level(mobs, loose);
            }
        }
        let count = by_type.values().map(|r| r.0).sum();
        let bytes = by_type.values().map(|r| r.1).sum();
        let rows = by_type.into_iter().map(|(id, (n, b))| (id, b, n)).collect();
        Entities {
            count,
            total_mb: mb(bytes),
            by_type: biggest(rows, TOP)
                .into_iter()
                .map(|(id, b, n)| (id, n, mb(b)))
                .collect(),
            bytes,
        }
    }

    /// This game's own server parts: it is a dedicated server, or the single-player server thread.
    pub(in crate::game::server_side) fn server_side(&self) -> ServerSide {
        let (players, chunk_send) = match &self.server.lan {
            Some(host) => (player_census(&host.hub), chunk_send_census(&host.hub)),
            None => (Players::default(), ChunkSend::default()),
        };
        let (mut generation, mut light) = {
            let q = self.server.streaming.queue_census();
            (q.generation, q.light)
        };
        for streaming in self.server.levels.0.parked_streamings() {
            let q = streaming.queue_census();
            generation = (generation.0 + q.generation.0, generation.1 + q.generation.1);
            light = (light.0 + q.light.0, light.1 + q.light.1);
        }
        ServerSide {
            chunks: self.census_chunks(),
            generation: Generation::of(&self.server.streaming),
            entities: self.entity_census(),
            players,
            chunk_send,
            light_queue: Queue {
                len: light.0,
                size_mb: mb(light.1),
            },
            generation_queue: Queue {
                len: generation.0,
                size_mb: mb(generation.1),
            },
            queue_bytes: generation.1 + light.1,
        }
    }

    pub(in crate::game) fn server_memory_inputs(&self) -> MemoryInputs {
        MemoryInputs {
            render_distance: self.server.streaming.radius,
            client: None,
            far: None,
        }
    }

    pub(in crate::game) fn server_memory_report(&self) -> MemoryReport {
        self.server_memory_inputs()
            .report(Some(self.server_side()), || true, || None)
            .expect("a manual census cannot be canceled")
    }
}

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::{MemoryCost, PendingMemory, biggest, flatten_report, shared, unattributed};
    use serde_json::json;
    use std::sync::{Arc, mpsc::channel};
    use std::task::Poll;
    use sunrise_level::chunk::{Light, Section};

    #[test]
    fn sharing_counts_a_section_only_once_across_both_censuses() {
        let section = Arc::new(Section {
            light: Light::Sky(Box::new([0; 2048]), 0),
            ..Section::default()
        });
        // An equal, independently held section isn't shared.
        let independent = Arc::new((*section).clone());
        let counted = shared(
            [&section, &section, &independent],
            [Arc::as_ptr(&section)].into_iter().collect(),
        );
        assert_eq!(counted.sections, 1);
        assert_eq!(counted.bytes, 2048);
    }

    #[test]
    fn stopping_before_the_server_replies_releases_the_worker_without_collecting() {
        let (mut sample, request) = PendingMemory::spawn(MemoryCost::default(), true, |_, _| {
            panic!("a canceled request reached the worker's census");
        })
        .unwrap();
        sample.cancel();
        // It must finish even while the owner request is still queued.
        sample.worker.join().unwrap();
        request
            .unwrap()
            .collect(|| panic!("the stopped capture collected server memory"));
        assert!(sample.rows.try_recv().is_err());
    }

    #[test]
    fn a_restart_waits_for_the_old_worker_to_release_its_slot_and_never_gets_its_rows() {
        let (entered, entered_rx) = channel();
        let (release, release_rx) = channel();
        let (mut old, _) = PendingMemory::spawn(MemoryCost::default(), false, move |_, _| {
            entered.send(()).unwrap();
            release_rx.recv().unwrap();
            Some(vec![("old".into(), 1.0)])
        })
        .unwrap();
        entered_rx.recv().unwrap();
        old.cancel();
        // Polling a new capture before the old work exits cannot start a new census.
        assert_eq!(old.poll(), Poll::Pending);
        release.send(()).unwrap();
        old.worker.join().unwrap();
        assert!(old.rows.try_recv().is_err());
        let (new, _) = PendingMemory::spawn(MemoryCost::default(), false, |_, _| {
            Some(vec![("new".into(), 2.0)])
        })
        .unwrap();
        new.worker.join().unwrap();
        assert_eq!(new.rows.try_recv().unwrap(), [("new".into(), 2.0)]);
    }

    #[test]
    fn unattributed_is_resident_minus_the_named_bytes_and_may_be_negative() {
        let mib = 1 << 20;
        assert_eq!(
            unattributed(Some(900 * mib), 600 * mib as usize),
            Some(300.0)
        );
        assert_eq!(unattributed(Some(10 * mib), 25 * mib as usize), Some(-15.0));
        assert_eq!(unattributed(None, 5), None);
    }

    #[test]
    fn the_biggest_rows_come_first_with_ties_by_name() {
        let rows = vec![
            ("b".to_string(), 2, ()),
            ("a".to_string(), 2, ()),
            ("c".to_string(), 9, ()),
        ];
        let names: Vec<_> = biggest(rows, 2).into_iter().map(|r| r.0).collect();
        assert_eq!(names, ["c", "a"]);
    }

    #[cfg(any())]
    #[test]
    #[ignore = "daily"]
    fn a_servers_report_names_its_entities_and_players_and_leaves_the_rest_unattributed() {
        use crate::game::test_pos_guest_tests::{GUEST, game};
        use sunrise_sim::mobs::Mob;
        let mut resources = None;
        let mut game = game(&mut resources);
        game.options.dedicated = Some(Vec::new());
        for (id, kind) in [(1, "cow"), (2, "cow"), (3, "pig")] {
            let kind = game
                .res
                .mob_types()
                .get(&format!("minecraft:{kind}"))
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(
                id,
                kind,
                sunrise_core::geometry::Vec3::new(0.5, 64.0, 0.5),
                0.0,
            ));
        }
        // The fixture's guest has no reader, and a packet nobody will read is dropped rather than
        // kept, so its bytes are not counted (`PlayerSender::publish_reserved`). A queue is counted
        // while its reader is still to come: this one outlives the report.
        let (tx, queue) = channel();
        game.side
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |guest| {
                guest.sender = sunrise_sim::lan::PlayerSender::new(tx, Arc::default());
                guest.sender.send(vec![0; 3 << 20]);
            });
        let report = game.memory_report();
        drop(queue);
        let by_type: std::collections::BTreeMap<_, _> = report
            .entities
            .as_ref()
            .unwrap()
            .by_type
            .iter()
            .map(|(id, n, _)| (id.as_str(), *n))
            .collect();
        assert_eq!((by_type["minecraft:cow"], by_type["minecraft:pig"]), (2, 1));
        let players = report.players.as_ref().unwrap();
        assert_eq!(players.count, 1);
        assert_eq!(players.send_queue[0].0, GUEST);
        assert_eq!(players.send_queue[0].2, 3.0);
        assert!(report.client_columns.is_none() && report.columns.is_some());
        let resident = report.process.resident_mb.unwrap();
        let rest = report.unattributed_mb.unwrap();
        assert!(
            (rest - (resident - report.named_mb)).abs() <= 0.15,
            "{rest} {resident} {}",
            report.named_mb
        );
    }

    #[test]
    fn a_report_flattens_to_paths_and_named_list_entries() {
        let report = json!({
            "process": { "resident_mb": 12.5, "note": "skipped" },
            "render_distance": 16,
            "missing": null,
            "by_label_mb": [["chunks", 3.0], ["bare"]],
            "caches": [["blocks", 7, 1.5]],
        });
        let mut rows = flatten_report(&report);
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        let expected = [
            ("by_label_mb[chunks]", 3.0),
            ("caches[blocks]", 7.0),
            ("caches_mb[blocks]", 1.5),
            ("process.resident_mb", 12.5),
            ("render_distance", 16.0),
        ];
        assert_eq!(
            rows.iter()
                .map(|(p, v)| (p.as_str(), *v))
                .collect::<Vec<_>>(),
            expected
        );
    }
}
