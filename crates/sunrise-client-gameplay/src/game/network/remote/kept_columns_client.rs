use super::*;
use sunrise_gameplay_common::game::remote::kept_columns::Column;
use sunrise_level::{chunk_map::ClientChunkRange, fast_hash::FxHashSet};
use sunrise_protocol::viewed_levels::MAX_COLUMNS;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Every column goes with the client level, including hidden ring slots.
    pub fn forget_all_remote_columns(&mut self) -> Vec<Column> {
        self.clear_remote_light();
        let mut gone = self.common.world.chunks.resident_keys().collect();
        self.forget_remote_columns(&mut gone, false)
    }
    /// Forgets the `gone` columns (and empties the set): their chunks (freed on a thread
    /// of their own when there are many), their sections in the renderer and their block
    /// entities, each store once for all of them. When the level stays open,
    /// far terrain keeps their last state (`server_terrain`). Returns the
    /// positions it freed, in `gone`'s order: only those still resident,
    /// so a later forget of the same positions frees nothing.
    pub fn forget_remote_columns(
        &mut self,
        gone: &mut FxHashSet<Column>,
        keep_terrain: bool,
    ) -> Vec<Column> {
        if gone.is_empty() {
            return Vec::new();
        }
        // The client chunk seam: one unload per column still resident, with
        // its data still there, before `take_remote_columns` frees it.
        let freed_at: Vec<(i32, i32)> = gone
            .iter()
            .filter(|key| self.common.world.chunks.resident_get(key).is_some())
            .copied()
            .collect();
        for &(x, z) in &freed_at {
            self.client_chunk_unload(x, z, keep_terrain);
        }
        self.free_remote_columns(gone, keep_terrain);
        freed_at
    }
    /// Explicit Forget always removes the requested column's queued light. Only a
    /// visible, exact-coordinate block column can be dropped from the ring.
    pub(in crate::game::client_side) fn forget_remote_packets(
        &mut self,
        positions: &mut FxHashSet<Column>,
    ) {
        self.forget_remote_light(positions);
        positions.retain(|&(x, z)| {
            self.remote_chunk_in_range(x, z) && self.common.world.chunks.contains_key(&(x, z))
        });
        self.forget_remote_columns(positions, true);
    }
    pub(in crate::game::client_side) fn free_remote_columns(
        &mut self,
        gone: &mut FxHashSet<Column>,
        keep_terrain: bool,
    ) {
        if gone.is_empty() {
            return;
        }
        let freed = sunrise_level::chunk_streamer::ChunkStreamer::take_remote_columns(
            &mut self.common.world,
            gone,
        );
        self.client
            .streaming
            .forget_columns(&mut self.common.world, gone);
        if keep_terrain && let Some(lod) = &mut self.client.lod.0 {
            for chunk in &freed {
                lod.observe(chunk.clone(), self.common.res.facts());
            }
        }
        if let Some(renderer) = self.client.renderer.0.as_mut() {
            renderer.remove_columns(gone.iter().copied());
        }
        sunrise_level::chunk::free_columns(freed, gone.len());
        self.client
            .be_anims
            .pots
            .retain(|[x, _, z], _| !gone.contains(&(*x >> 4, *z >> 4)));
        sunrise_level::spans::time("unload_block_entities", || {
            for &column in gone.iter() {
                self.common.world.block_entities.unload_column(column);
            }
        });
        if let Some(r) = self.client.remote.as_mut() {
            r.attachments.retain(|holder, _| match *holder {
                Holder::Chunk([x, z]) => !gone.contains(&(x, z)),
                Holder::BlockEntity([x, _, z]) => !gone.contains(&(x >> 4, z >> 4)),
                _ => true,
            });
        }
        gone.clear();
    }
    pub(in crate::game::client_side) fn remote_chunk_in_range(&self, x: i32, z: i32) -> bool {
        self.client.replay.playing()
            || self.common.world.chunks.is_far((x, z))
            || self.client.remote.as_ref().is_none_or(|r| {
                ClientChunkRange::new(r.chunk_center, r.view_distance).contains((x, z))
            })
    }

    /// `sunrise:levels` `far` (revision 5): the server keeps `kept` here past the view from now
    /// on, and `released` no more; a released column outside the view goes, as a forget. False,
    /// keeping none of `kept`, when they would pass `MAX_COLUMNS`: the connection closes.
    pub(in crate::game) fn remote_far_columns(
        &mut self,
        kept: &[Column],
        released: &[Column],
    ) -> bool {
        let mut gone: FxHashSet<Column> = self
            .common
            .world
            .chunks
            .release_far(released)
            .into_iter()
            .collect();
        let (far, _) = self.common.world.chunks.far_state();
        let new = kept
            .iter()
            .filter(|&&key| !self.common.world.chunks.is_far(key));
        let within = far + new.count() <= MAX_COLUMNS as usize;
        if within {
            let shown = self.common.world.chunks.keep_far(kept);
            self.client.streaming.remote_chunk_visibility_changed(
                &mut self.common.world,
                &[],
                &shown,
            );
        }
        // Released, they are out of range: not `forget_remote_packets`, which keeps those.
        self.forget_remote_light(&gone);
        self.forget_remote_columns(&mut gone, true);
        within
    }

    pub(in crate::game) fn set_remote_chunk_cache_center(&mut self, center: Column) {
        if let Some(r) = self.client.remote.as_mut() {
            r.chunk_center = center;
            let range = ClientChunkRange::new(center, r.view_distance);
            let adopted_view = std::mem::take(&mut r.awaiting_viewed_chunk_center);
            if !self.client.replay.playing() {
                if adopted_view {
                    // Viewed worlds hold arbitrary areas, not a ring. Only copy entries
                    // in the new active view when establishing its cache for the first time.
                    let chunks = &self.common.world.chunks;
                    let mut gone = chunks
                        .keys()
                        .copied()
                        .filter(|&key| !range.contains(key) && !chunks.is_far(key))
                        .collect();
                    self.forget_remote_columns(&mut gone, true);
                }
                self.update_remote_chunk_view(range);
            }
        }
    }
    pub(in crate::game) fn set_remote_chunk_cache_radius(&mut self, distance: i32) {
        self.client.streaming.radius =
            sunrise_gameplay_common::game::streaming_radius::streaming_radius_for(
                self.client.settings.render_distance,
                Some(distance),
            );
        if let Some(r) = self.client.remote.as_mut() {
            let old = ClientChunkRange::new(r.chunk_center, r.view_distance);
            let range = ClientChunkRange::new(r.chunk_center, distance);
            r.view_distance = distance;
            if old != range && !r.awaiting_viewed_chunk_center && !self.client.replay.playing() {
                // ClientChunkCache.updateViewRadius copies the in-range slots into new
                // storage and abandons the rest. Their data is freed here, so each tells
                // its `client_chunk_unload`, unlike a centre move's hidden slots.
                let chunks = &self.common.world.chunks;
                let mut gone: FxHashSet<Column> = chunks
                    .resident_keys()
                    .filter(|&key| !range.contains(key) && !chunks.is_far(key))
                    .collect();
                for &(x, z) in &gone {
                    self.forget_chunk_passages(x, z);
                }
                self.forget_remote_columns(&mut gone, true);
                self.update_remote_chunk_view(range);
            }
        }
    }
    pub(in crate::game::client_side) fn update_remote_chunk_view(
        &mut self,
        range: ClientChunkRange,
    ) {
        let (hidden, shown) = self.common.world.chunks.set_client_chunk_range(range);
        if let Some(renderer) = &mut self.client.renderer.0 {
            renderer.remove_columns(hidden.iter().copied());
        }
        self.client.streaming.remote_chunk_visibility_changed(
            &mut self.common.world,
            &hidden,
            &shown,
        );
    }
}
