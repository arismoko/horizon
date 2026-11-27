//! The frame's world update (`update_world`): what arrived from the server, the chunks that
//! streamed in and the meshes handed to the renderer, the columns that unload (their entities
//! and block entities saved out first); and far terrain (`prepare_lod`, `clear_lod`).
use super::*;

impl crate::game::ClientGame<'_> {
    pub fn client_update_world(&mut self) {
        debug_assert!(
            !self.is_logical_server(),
            "client_update_world called on server"
        );
        if let Err(e) = sunrise_level::spans::time("poll_remote", || self.poll_remote()) {
            log::warn!("Sunrise: {e}");
        }
        if let Err(e) = self.advance_studio_replay() {
            log::warn!("Sunrise Studio replay: {e}");
        }
        // A disconnect during poll_remote ends the client session; stop before world work runs.
        if self.client.title.0.is_some() || self.client.remote.is_none() {
            return;
        }
        self.poll_remote_light();
        // The renderer laid translucent quads out again (a shader pack's flat terrain came
        // or went) and dropped every section and far tile: meshed again, as for a new one.
        if self
            .client
            .renderer
            .0
            .as_deref_mut()
            .is_some_and(Renderer::take_dropped_sections)
        {
            self.clear_lod();
            self.all_sections_changed();
        }
        sunrise_level::spans::time("prepare_lod", || self.prepare_lod());
        // The columns the single-player server changed in bulk without sending them, which far terrain still drew
        // as this window last saw them: summarized again (taken every tick, far terrain on or off, so none waits).
        let changed = sunrise_sim::lan::handoff::take_changed();
        if let Some(lod) = &mut self.client.lod.0 {
            for chunk in changed {
                let column = (chunk.x, chunk.z);
                if self.common.world.chunks.contains_key(&column) {
                    continue;
                }
                lod.observe(chunk, self.common.res.facts());
            }
        }
        self.observe_light_edits();
        self.ask_edited_light_emission();
        let camera_pos = self.visual_camera_position();
        if let (Some(map), Some(pos), Some(lod)) = (
            &self.client.lod_blue_map.0,
            camera_pos,
            &mut self.client.lod.0,
        ) {
            map.set_center(pos[0].floor() as i32, pos[2].floor() as i32);
            for change in map.take_changed() {
                lod.refresh(change.area, change.widths);
            }
        }
        if let Some(pos) = camera_pos {
            self.client.streaming.center =
                ((pos[0].floor() as i32) >> 4, (pos[2].floor() as i32) >> 4);
            self.client.streaming.center_y = pos[1].floor() as i32;
        }
        // A headless game has nothing to draw sections with: like a server's, they aren't
        // meshed while it has no renderer.
        if self.common.options.headless {
            self.client.streaming.mesh = self.client.renderer.0.is_some();
        }
        let areas = self.client.streaming.extra.len();
        // `Level.getChunk`'s tickets keep their columns until they expire.
        self.client.streaming.hold_unknown_tickets();
        self.client.streaming.upload_backlog = self.client.pending_uploads.0.len();
        let mut plan = sunrise_level::spans::time("streaming_plan", || {
            let mut step = self
                .client
                .streaming
                .begin_plan_update(&mut self.common.world);
            loop {
                match step {
                    sunrise_client_render::streaming::PlanStep::Ready(plan) => break plan,
                    sunrise_client_render::streaming::PlanStep::Save { chunk, resume } => {
                        self.client.streaming.save_finished(chunk);
                        step = self
                            .client
                            .streaming
                            .resume_plan_update(&mut self.common.world, resume);
                    }
                }
            }
        });
        let (commit, protos) = self
            .client
            .streaming
            .prepare_commit(&mut self.common.world, &mut plan);
        self.client.streaming.submit_protos(protos);
        let (mut uploads, unloaded) = sunrise_level::spans::time("streaming", || {
            self.client
                .streaming
                .complete_commit(&mut self.common.world, commit)
        });
        // Sections chosen to mesh wait for the mods' `section_visibility` (none without a mod).
        self.prepare_section_visibility();
        self.client.streaming.extra.truncate(areas);
        // The viewed levels' meshing, queued after the player's level's (nothing while no view
        // names one).
        if self.client.streaming.mesh
            && let Some(remote) = &mut self.client.remote
        {
            let streaming = &self.client.streaming;
            remote
                .viewed
                .mesh_frame(|| streaming.viewed_mesh_source(), std::time::Instant::now());
        }
        self.ask_loaded_light_emission();
        // Vanilla saves the chunk before its unload hook and drops its block entities afterwards.
        sunrise_level::spans::time("mods_chunks_changed", || {
            self.native_chunks_loaded();
            self.native_chunks_unloaded(&unloaded);
        });
        for chunk in &unloaded {
            self.common.world.modified.unloaded(&(chunk.x, chunk.z));
        }
        if let Some(lod) = &mut self.client.lod.0 {
            self.client
                .lod_observe
                .extend(uploads.iter().map(|upload| (upload.key.0, upload.key.2)));
            for chunk in &unloaded {
                self.client.lod_observe.remove(&(chunk.x, chunk.z));
            }
            // Unloaded columns first (their summaries are all that remains),
            // then a few resident ones (an edit copies a chunk the worker
            // still holds). The worker summarizes them.
            sunrise_level::spans::time("lod_observe", || {
                for chunk in &unloaded {
                    lod.observe(chunk.clone(), self.common.res.facts());
                }
                while lod.observing() < sunrise_client::lod::OBSERVE_AHEAD
                    && let Some(&key) = self.client.lod_observe.iter().next()
                {
                    self.client.lod_observe.remove(&key);
                    if let Some(chunk) = self.common.world.chunks.get(&key) {
                        lod.observe(chunk.clone(), self.common.res.facts());
                    }
                }
            });
        }
        for chunk in &unloaded {
            self.common.world.modified.unloaded(&(chunk.x, chunk.z));
        }
        if !unloaded.is_empty() {
            let gone: sunrise_level::fast_hash::FxHashSet<(i32, i32)> =
                unloaded.iter().map(|c| (c.x, c.z)).collect();
            sunrise_level::spans::time("unload_block_entities", || {
                for &column in &gone {
                    self.common.world.block_entities.unload_column(column);
                }
            });
            self.common
                .world
                .block_names
                .retain(|p, _| !gone.contains(&(p.0 >> 4, p.2 >> 4)));
        }
        let (far_distance, far_reduced) =
            sunrise_level::spans::time("far_terrain_choice", || self.far_terrain_choice());
        // Terrain still on its way from the server (single player's integrated one
        // too): the view's outermost columns not all in, or its events not all handled.
        let radius = match &self.client.remote {
            // As far as the server fills it: its view distance and one.
            Some(r) => self.client.streaming.radius.min(r.view_distance + 1),
            None => self.client.streaming.radius,
        };
        let view = (self.client.streaming.center, radius);
        let has = |column: (i32, i32)| self.common.world.chunks.contains_key(&column);
        let now = Instant::now();
        let arriving = {
            let _span = sunrise_level::spans::guard("view_arrival");
            self.client.view_arrival.arriving(view, now, has) || self.remote_backlog() > 0
        };
        // Which far tiles a zoom magnifies (`lod::ZoomAim`), read before
        // `self.renderer.as_mut()` below borrows self mutably for the rest of
        // this function. Cheap: trigonometry on one angle, or None when the
        // view is not zoomed.
        let lod_aim = camera_pos.and_then(|camera| self.lod_aim(camera));
        if let Some(renderer) = self.client.renderer.0.as_mut() {
            let g = &self.client.settings.sunrise;
            let budgeted = g.chunk_updates > 0 || g.upload_budget_kib > 0;
            let mut raw_flight = self.client.flight.0.as_mut().filter(|flight| flight.raw);
            // The meshes, once copied, are freed by a worker (`MeshStreamer::free`).
            if !budgeted
                && self.client.pending_uploads.0.is_empty()
                && uploads.len() <= AUTO_UPLOADS_AT_ONCE
            {
                sunrise_level::spans::time("upload", || {
                    for upload in &uploads {
                        if self
                            .client
                            .streaming
                            .can_publish_mesh(&self.common.world, upload)
                        {
                            let bytes = renderer.mesh_upload_bytes(&upload.mesh);
                            renderer.upload(upload.key, &upload.mesh);
                            if let Some(flight) = raw_flight.as_deref_mut() {
                                flight.note_mesh_uploads(1, bytes);
                            }
                        }
                    }
                });
                self.client.streaming.free(uploads);
            } else {
                // Sunrise's chunk-update and upload budgets: the nearest
                // sections go first, the rest wait for later frames; a newer
                // mesh of a waiting section replaces it, and sections of
                // chunks no longer loaded are dropped.
                let pending = &mut self.client.pending_uploads.0;
                let order_span = sunrise_level::spans::guard("order_uploads");
                let fresh: sunrise_level::fast_hash::FxHashSet<(i32, i32, i32)> =
                    uploads.iter().map(|upload| upload.key).collect();
                let current = |upload: &sunrise_client_render::streaming::MeshUpload| {
                    self.common
                        .world
                        .chunks
                        .contains_key(&(upload.key.0, upload.key.2))
                        && self
                            .client
                            .streaming
                            .can_publish_mesh(&self.common.world, upload)
                };
                // Filtering leaves the surviving waiting run in the same order.
                let mut spent: Vec<_> = pending
                    .extract_if(.., |upload| fresh.contains(&upload.key) || !current(upload))
                    .collect();
                spent.extend(uploads.extract_if(.., |upload| !current(upload)));
                let centre = self.client.streaming.center;
                // Section keys count from the world's bottom (`origin`'s min_y + y * 16).
                let bottom = f64::from(self.common.world.bottom_y());
                let cam_y = camera_pos.map_or(4, |pos| ((pos[1] - bottom) / 16.0).floor() as i32);
                let far = |k: &(i32, i32, i32)| section_distance_sq(*k, centre, cam_y);
                let order = Some((centre, cam_y));
                let priority = |upload: &sunrise_client_render::streaming::MeshUpload| {
                    std::cmp::Reverse(far(&upload.key))
                };
                let work = order_uploads(
                    pending,
                    &mut uploads,
                    &mut self.client.pending_uploads_merge.0,
                    self.client.pending_uploads_order == order,
                    priority,
                );
                self.client.pending_uploads_order = order;
                drop(order_span);
                let (limit, budget) = if budgeted {
                    (
                        g.chunk_updates as usize,
                        u64::from(g.upload_budget_kib) * 1024,
                    )
                } else {
                    (0, 0)
                };
                let (mut count, mut bytes) = (0, 0u64);
                // Without the player's budgets, uploads stop once they took this
                // frame's share (`UploadPace::budget`: a quarter of the recent frame
                // time, more with a backlog, at most 2 ms; at least one goes): a burst
                // of finished meshes spreads over frames, nearest first.
                let auto =
                    (!budgeted).then(|| self.client.upload_pace.budget(pending.len(), arriving));
                let started = Instant::now();
                let upload_span = sunrise_level::spans::guard("upload_queue");
                while let Some(upload) = pending.last() {
                    let mesh = &upload.mesh;
                    if auto.is_some_and(|time| count > 0 && started.elapsed() >= time) {
                        break;
                    }
                    let size = renderer.mesh_upload_bytes(mesh);
                    if (limit > 0 && count >= limit)
                        || (budget > 0 && bytes > 0 && bytes + size > budget)
                    {
                        break;
                    }
                    let upload = pending.pop().expect("checked above");
                    renderer.upload(upload.key, &upload.mesh);
                    spent.push(upload);
                    count += 1;
                    bytes += size;
                }
                drop(upload_span);
                if let Some(flight) = raw_flight {
                    flight.note_upload_queue_work(work);
                    flight.note_mesh_uploads(count, bytes);
                }
                self.client.streaming.free(spent);
            }
            renderer.remove_columns(unloaded.iter().map(|c| (c.x, c.z)));
            // Meshes in the works are the view's terrain while it is new, then edits'.
            let meshing = self.client.streaming.busy() || !self.client.pending_uploads.0.is_empty();
            let sampling = self.client.lod.0.as_ref().is_some_and(|lod| lod.sampling());
            renderer.set_terrain_loading(
                arriving || (meshing && self.client.view_arrival.recent(now)) || sampling,
            );
            if let (Some(lod), Some(player)) = (&mut self.client.lod.0, &self.common.player) {
                let lod_distance =
                    sunrise_client::graphics::lod_distance_capped(far_distance, &renderer.info);
                lod.set_near_loading(self.client.streaming.terrain_loading());
                lod.set_reduced_detail(far_reduced);
                // The covered columns change only with the complete ones and
                // the render area: rebuilt then, not every frame.
                let at = (
                    renderer.lod_detail_revision(),
                    self.client.streaming.center,
                    self.client.streaming.radius,
                );
                let covered = (self.client.lod_covered_at != Some(at)).then(|| {
                    self.client.lod_covered_at = Some(at);
                    sunrise_level::spans::time("lod_covered_set", || {
                        renderer
                            .lod_detail_columns()
                            .filter(|&(x, z)| {
                                sunrise_level::chunk_streamer::in_view_distance(at.1, at.2, x, z)
                            })
                            .collect()
                    })
                });
                // Far terrain's finest tiles stand where full-detail terrain
                // ends: on a server, where it stops sending chunks, often well
                // inside the render distance. Thousands of chunk-sized tiles
                // between the two filled in long after the coarser ones.
                let detail = self
                    .client
                    .remote
                    .as_ref()
                    .map_or(self.client.streaming.radius, |r| {
                        self.client.streaming.radius.min(r.sent_radius())
                    });
                let camera =
                    camera_pos.unwrap_or([player.position.x, player.position.y, player.position.z]);
                let meshes = sunrise_level::spans::time("lod_update", || {
                    lod.update(camera, detail, lod_distance, covered, lod_aim)
                });
                renderer.queue_lods(meshes, lod.removed(), lod.take_cutout());
                // A finished job can hold hundreds of tiles: spread them
                // over frames rather than stall one.
                sunrise_level::spans::time("lod_upload", || {
                    renderer.upload_lods(std::time::Duration::from_millis(2))
                });
            }
        }
    }
}

/// Farthest first (the uploader pops from the back). Returns the number of
/// elements passed to a sort and to the merge, respectively.
pub(in crate::game::client_side) fn order_uploads<T, K: Ord>(
    waiting: &mut Vec<T>,
    fresh: &mut Vec<T>,
    scratch: &mut Vec<T>,
    same_section: bool,
    priority: impl Fn(&T) -> K,
) -> [usize; 2] {
    if !same_section {
        waiting.append(fresh);
        if waiting.len() < 2 {
            return [0, 0];
        }
        waiting.sort_unstable_by_key(priority);
        return [waiting.len(), 0];
    }
    let sorted = if fresh.len() > 1 {
        fresh.sort_unstable_by_key(&priority);
        fresh.len()
    } else {
        0
    };
    if waiting
        .last()
        .zip(fresh.first())
        .is_none_or(|(old, new)| priority(old) <= priority(new))
    {
        waiting.append(fresh);
        return [sorted, 0];
    }

    // At equal distance, write the waiting mesh first so the fresh mesh is
    // popped first.
    let merged = waiting.len() + fresh.len();
    scratch.reserve(merged);
    let mut old = waiting.drain(..).peekable();
    let mut new = fresh.drain(..).peekable();
    while let (Some(a), Some(b)) = (old.peek(), new.peek()) {
        scratch.extend(if priority(a) <= priority(b) {
            old.next()
        } else {
            new.next()
        });
    }
    scratch.extend(old);
    scratch.extend(new);
    std::mem::swap(waiting, scratch);
    [sorted, merged]
}

/// Squared section-grid distance in `i128` so extreme camera coordinates cannot overflow.
pub(in crate::game::client_side) fn section_distance_sq(
    key: (i32, i32, i32),
    centre: (i32, i32),
    cam_y: i32,
) -> i128 {
    let dx = i128::from(key.0) - i128::from(centre.0);
    let dy = i128::from(key.1) - i128::from(cam_y);
    let dz = i128::from(key.2) - i128::from(centre.1);
    dx * dx + dy * dy + dz * dz
}

impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn clear_lod(&mut self) {
        // The samplers waiting on the server stop first: dropping the far
        // terrain joins them.
        if let Some(remote) = &self.client.remote {
            remote.session.cancel_far_terrain();
        }
        self.client.lod.0 = None;
        self.client.lod_source_attempted = false;
        self.client.lod_observe.clear();
        self.client.lod_blue_map.0 = None;
        self.client.lod_covered_at = None;
        if let Some(renderer) = &mut self.client.renderer.0 {
            renderer.clear_lods();
        }
    }
    pub(in crate::game) fn client_clear_lod(&mut self) {
        self.clear_lod();
    }
    pub(in crate::game::client_side) fn prepare_lod(&mut self) {
        let enabled = self.far_terrain_choice().0 > 0
            && self.client.renderer.0.is_some()
            && self.common.player.is_some()
            && self.client.title.0.is_none();
        if !enabled {
            if self.client.lod.0.is_some() || self.client.lod_source_attempted {
                self.clear_lod();
            }
            return;
        }
        // Another level: its own kept chunks. (Another world at the same
        // address, a proxy's next server, comes through a new login; a seed
        // changed at a respawn of the same level is a plugin hiding it.)
        if self.client.lod_level.as_ref().is_some_and(|level| {
            self.client
                .remote
                .as_ref()
                .is_some_and(|r| r.level != *level)
        }) {
            self.clear_lod();
        }
        if self.client.lod_source_attempted {
            return;
        }
        self.client.lod_source_attempted = true;
        self.client.lod_level = None;
        let dim = self.common.dim;
        let source = match (
            &self.client.integrated,
            self.server_far_terrain(),
            &self.client.remote,
        ) {
            (Some(server), ..) => server
                .ask(|reply| {
                    sunrise_gameplay_common::game::owner_link::OwnerRequest::LodSource(dim, reply)
                })
                .flatten(),
            (None, Some(offer), Some(remote)) => {
                let states = self.common.res.facts().states.len() as u32;
                let biomes = self.common.res.biomes().index.clone();
                let source: Arc<dyn sunrise_level::chunk_streamer::LodSource> =
                    remote.session.far_terrain(offer, biomes, states);
                Some(source)
            }
            // A friend's world has a new code at every share: nothing to keep.
            (None, None, Some(remote))
                if remote
                    .session
                    .address
                    .starts_with(sunrise_client::net::FRIEND) =>
            {
                None
            }
            // No offer from a server: draw what it sent, on this visit and before.
            (None, None, Some(remote)) => std::env::current_dir().ok().map(|home| {
                use sunrise_client::server_terrain as terrain;
                let address = &remote.session.address;
                let root = home.join("server-terrain");
                let dir = terrain::level_dir(&root, address, remote.hashed_seed, &remote.level);
                self.client.lod_level = Some(remote.level.clone());
                // Past them, the server's BlueMap, if the player named it.
                let maps = root.join(terrain::server_folder(address)).join("BlueMap");
                let settings = &self.client.settings.sunrise.blue_map;
                let blue_map = settings.server(address).map(|server| {
                    use sunrise_client::bluemap;
                    // The map's address only at debug: logs land in crash reports.
                    log::info!("Sunrise: far terrain also draws {}'s BlueMap", remote.level);
                    log::debug!("Sunrise: BlueMap at {}", server.url);
                    let map = Arc::new(bluemap::BlueMap::new(
                        &server.url,
                        &remote.level,
                        server.maps.clone(),
                        maps,
                        settings.radius_blocks(),
                        bluemap::Palette::from_game(
                            self.common.pack.assets.registry,
                            self.common.res.biomes(),
                        ),
                    ));
                    map.start();
                    map
                });
                self.client.lod_blue_map.0 = blue_map.clone();
                terrain::ServerTerrain::new(dir, blue_map) as _
            }),
            // A client without a server connection has no world for far terrain.
            (None, _, None) => None,
        };
        if let Some(source) = source {
            self.client.lod.0 = Some(sunrise_client::lod::Lod::new(source, self.worker_context()));
            self.client
                .lod_observe
                .extend(self.common.world.chunks.keys().copied());
        }
    }
}

use sunrise_gameplay_common::game::light_emission::LightEmissionGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::SharedGameExt1 as _;

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::{order_uploads, section_distance_sq};
    use std::cmp::Reverse;

    #[test]
    fn section_distance_sq_handles_extreme_coordinates_without_overflow() {
        let far = section_distance_sq(
            (i32::MAX, i32::MAX, i32::MAX),
            (i32::MIN, i32::MIN),
            i32::MIN,
        );
        let near = section_distance_sq(
            (i32::MIN, i32::MIN, i32::MIN),
            (i32::MIN, i32::MIN),
            i32::MIN,
        );
        assert_eq!(near, 0);
        assert!(far > near);
    }

    #[test]
    fn stationary_upload_budget_gives_fresh_meshes_priority_on_ties() {
        let mut waiting = vec![(20, "far"), (4, "waiting")];
        let mut fresh = vec![(1, "nearest"), (9, "middle"), (4, "fresh")];
        let mut scratch = Vec::new();
        let priority = |&(distance, _): &(i32, &str)| Reverse(distance);
        order_uploads(&mut waiting, &mut fresh, &mut scratch, true, priority);
        // Pin the stationary queue's fresh-first tie policy: two uploads leave
        // the waiting mesh first in line next frame. The old sort_unstable_by_key
        // did not guarantee this tie order.
        let selected: Vec<_> = std::iter::from_fn(|| waiting.pop().map(|(_, name)| name))
            .take(2)
            .collect();
        assert_eq!(selected, ["nearest", "fresh"]);
        assert_eq!(waiting.pop(), Some((4, "waiting")));
        assert_eq!(waiting.pop(), Some((9, "middle")));
        assert_eq!(waiting.pop(), Some((20, "far")));
    }
}
