use super::*;

#[derive(Default)]
pub(super) struct LodDecoded {
    pub(super) chunks: crate::fast_hash::FxHashMap<(i32, i32), (std::time::Instant, LodSurfaces)>,
    pub(super) order: std::collections::VecDeque<(i32, i32)>,
}
/// Saved chunks kept decoded for far terrain (each tens of kilobytes), and
/// for how long: edits saved later show after it.
pub(super) const LOD_DECODED: usize = 1024;
pub(super) const LOD_DECODED_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// Loads saved chunks before falling back to generation.
pub struct Persistent<G> {
    pub inner: Arc<G>,
    pub save: Arc<SaveDir>,
    pub registry: Arc<crate::assets::Assets>,
    pub biomes: Arc<Biomes>,
}
pub(super) type LodSurfaces = Arc<[sunrise_level::lod_data::Column; 256]>;
/// The coarsest far tile spacing whose columns read saved chunks: 16 blocks
/// a column, and 4 with Reduced far detail (`LodSampler::set_approximate`).
pub(super) const SAVED_LOD_STRIDE: i32 = 16;
const SAVED_LOD_STRIDE_REDUCED: i32 = 4;
pub(super) struct SavedLodSampler<G: ChunkGenerator> {
    pub(super) generator: Arc<Persistent<G>>,
    pub(super) inner: Box<dyn sunrise_level::chunk_streamer::LodSampler>,
    pub(super) cache: std::collections::VecDeque<((i32, i32), Option<LodSurfaces>)>,
    pub(super) details: Vec<sunrise_level::chunk_streamer::LodDetail>,
    /// Reduced far detail: saved chunks only in tiles up to
    /// `SAVED_LOD_STRIDE_REDUCED` blocks a column.
    pub(super) approximate: bool,
    /// Chunks the generator decorated for far terrain alone, kept from
    /// earlier sessions; saved chunks take precedence over them.
    pub(super) generated: Arc<sunrise_level::lod_data::cache::DiskCache>,
    /// The generator's far samples from earlier sessions (`lod::bake`).
    pub(super) bake: Option<Arc<sunrise_level::lod_data::bake::LodBake>>,
}
impl<G: ChunkGenerator> SavedLodSampler<G> {
    fn saved_columns(&mut self, key: (i32, i32)) -> Option<LodSurfaces> {
        let cached = match self.cache.iter().position(|(at, _)| *at == key) {
            Some(index) => self.cache.remove(index).unwrap(),
            None => (
                key,
                self.generator.save.lod_surfaces_shared(
                    key.0,
                    key.1,
                    &self.generator.registry,
                    &self.generator.biomes,
                ),
            ),
        };
        let columns = cached.1.clone();
        self.cache.push_front(cached);
        self.cache.truncate(128);
        columns
    }
}
impl<G: ChunkGenerator> sunrise_level::chunk_streamer::LodSampler for SavedLodSampler<G> {
    fn fork(&self) -> Option<Box<dyn sunrise_level::chunk_streamer::LodSampler>> {
        Some(Box::new(Self {
            generator: self.generator.clone(),
            inner: self.inner.fork()?,
            cache: Default::default(),
            details: Vec::new(),
            approximate: self.approximate,
            generated: self.generated.clone(),
            bake: self.bake.clone(),
        }))
    }
    fn set_approximate(&mut self, approximate: bool) {
        self.approximate = approximate;
        self.inner.set_approximate(approximate);
    }
    fn request_detail(&mut self, chunks: &[(i32, i32)], center: [i32; 2]) {
        let mut missing = Vec::new();
        for &key in chunks {
            // Generated chunks from earlier sessions answer only these
            // requests: reading them for every sampled cell of a coarse tile
            // slowed sampling more than it saved.
            if let Some(columns) = self
                .saved_columns(key)
                .or_else(|| self.generated.get(key.0, key.1))
            {
                self.details.push((key, columns));
            } else {
                missing.push(key);
            }
        }
        self.inner.request_detail(&missing, center);
    }
    fn drain_details(&mut self) -> Vec<sunrise_level::chunk_streamer::LodDetail> {
        let mut details = std::mem::take(&mut self.details);
        for (key, generated) in self.inner.drain_details() {
            // A generation batch also returns neighbouring, unrequested chunks.
            // Their saved builds must take precedence too.
            let columns = self.saved_columns(key).unwrap_or_else(|| {
                self.generated.put(key.0, key.1, &generated);
                generated
            });
            details.push((key, columns));
        }
        details
    }
    fn grid(
        &mut self,
        x: i32,
        z: i32,
        stride: i32,
        side: usize,
    ) -> Vec<sunrise_level::lod_data::Column> {
        let approximate = self.approximate;
        let baked = (self.bake.as_ref()).and_then(|b| b.grid(approximate, [x, z], stride, side));
        let mut columns = baked.unwrap_or_else(|| {
            let columns = self.inner.grid(x, z, stride, side);
            if let Some(bake) = &self.bake {
                bake.put_grid(approximate, [x, z], stride, side, &columns);
            }
            columns
        });
        // Saved chunks show in tiles up to 16 blocks a column. With Reduced far
        // detail only up to 4 (within about a kilometre): coarser tiles keep
        // the generator's terrain, which saved chunks were made from, since
        // decoding up to 256 saved chunks for each of them held the whole far
        // view back for minutes in large saves. Edits that far away show once
        // their tiles refine on approach.
        let limit = if self.approximate {
            SAVED_LOD_STRIDE_REDUCED
        } else {
            SAVED_LOD_STRIDE
        };
        if stride <= limit {
            for (index, column) in columns.iter_mut().enumerate() {
                let (wx, wz) = (
                    x + (index % side) as i32 * stride,
                    z + (index / side) as i32 * stride,
                );
                if let Some(saved) = self.saved_columns((wx >> 4, wz >> 4)) {
                    *column = sunrise_level::lod_data::Column::reduced(
                        &saved,
                        (wx & 15) as usize,
                        (wz & 15) as usize,
                        stride as usize,
                    );
                }
            }
        }
        columns
    }
    fn sample(&mut self, x: i32, z: i32) -> Option<crate::columns::Surface> {
        self.column(x, z).surface
    }
    fn column(&mut self, x: i32, z: i32) -> sunrise_level::lod_data::Column {
        self.reduced_column(x, z, 1)
    }
    fn reduced_column(&mut self, x: i32, z: i32, width: i32) -> sunrise_level::lod_data::Column {
        if width <= 16
            && let Some(columns) = self.saved_columns((x >> 4, z >> 4))
        {
            return sunrise_level::lod_data::Column::reduced(
                &columns,
                (x & 15) as usize,
                (z & 15) as usize,
                width as usize,
            );
        }
        self.inner.reduced_column(x, z, width)
    }
}

/// Unpack a section's paletted container into `out` (4096 blocks or 64 biome
/// cells): each entry's palette index as stored, unchecked. Returns the
/// palette's length, `None` for a container vanilla does not write. Once a
/// section: far terrain looked both up by name for each block (#6386).
pub(super) fn lod_palette_indices(
    container: &Tag,
    out: &mut [u16],
    minimum_bits: u32,
) -> Option<usize> {
    let palette = match container.get("palette")? {
        Tag::List(palette) if (1..=out.len()).contains(&palette.len()) => palette,
        _ => return None,
    };
    if palette.len() == 1 {
        out.fill(0);
        return Some(1);
    }
    let bits = (usize::BITS - (palette.len() - 1).leading_zeros()).max(minimum_bits) as usize;
    let per_word = 64 / bits;
    let Tag::LongArray(data) = container.get("data")? else {
        return None;
    };
    if data.len() != out.len().div_ceil(per_word) {
        return None;
    }
    for (word, indices) in data.iter().zip(out.chunks_mut(per_word)) {
        let mut word = *word as u64;
        for index in indices {
            *index = (word & ((1 << bits) - 1)) as u16;
            word >>= bits;
        }
    }
    Some(palette.len())
}
fn lod_light_layers(sections: &[(i32, &Tag)], layout: ColumnLayout, sky_light: bool) -> Box<[u8]> {
    let mut layers = vec![(None, None); layout.sections()];
    for &(y, section) in sections {
        let layer = |name: &str| match section.get(name) {
            Some(Tag::ByteArray(data)) if data.len() == 2048 => Some(&data[..]),
            _ => None,
        };
        layers[(y - (layout.min_y() >> 4)) as usize] = (
            if sky_light { layer("SkyLight") } else { None },
            layer("BlockLight"),
        );
    }
    let mut light = vec![0u8; layout.sections() * 4096].into_boxed_slice();
    let mut above = [if sky_light { 15u8 } else { 0 }; 256];
    let nibble = |data: &[u8], index: usize| (data[index / 2] >> ((index & 1) * 4)) & 15;
    for si in (0..layout.sections()).rev() {
        let (sky, block) = layers[si];
        let section = &mut light[si * 4096..(si + 1) * 4096];
        for (index, value) in section.iter_mut().enumerate() {
            *value = sky.map_or(above[index & 255], |data| nibble(data, index)) << 4
                | block.map_or(0, |data| nibble(data, index));
        }
        // Missing sky layers inherit the bottom layer above, as load_light does.
        for (index, value) in above.iter_mut().enumerate() {
            *value = section[index] >> 4;
        }
    }
    light
}
fn lod_face_light(light: &[u8], x: usize, y: usize, z: usize) -> u16 {
    let index = y * 256 + z * 16 + x;
    let mut sky = light[index] >> 4;
    let mut block = light[index] & 15;
    for (valid, index) in [
        (x > 0, index.wrapping_sub(1)),
        (x < 15, index + 1),
        (z > 0, index.wrapping_sub(16)),
        (z < 15, index + 16),
        (y > 0, index.wrapping_sub(256)),
        (index + 256 < light.len(), index + 256),
    ] {
        if valid {
            sky = sky.max(light[index] >> 4);
            block = block.max(light[index] & 15);
        }
    }
    u16::from(sky) << 4 | u16::from(block) << 12
}
pub(super) fn decode_lod_surfaces(
    root: &NamedTag,
    assets: &crate::assets::Assets,
    biomes: &Biomes,
    layout: ColumnLayout,
    sky_light: bool,
) -> Option<LodSurfaces> {
    let Tag::List(sections) = root.tag.get("sections")? else {
        return None;
    };
    let water_state = assets.registry.default_state("minecraft:water");
    let mut sections: Vec<_> = sections
        .iter()
        .filter_map(|section| {
            let y = number(section.get("Y"))? as i32;
            layout
                .section_index(y * 16)
                .is_some()
                .then_some((y, section))
        })
        .collect();
    sections.sort_unstable_by_key(|(y, _)| std::cmp::Reverse(*y));
    let mut columns: Box<[sunrise_level::lod_data::Column; 256]> =
        Box::new(std::array::from_fn(|_| sunrise_level::lod_data::Column {
            surface: None,
            spans: sunrise_level::lod_data::Spans::new(),
        }));
    let mut water = [None; 256];
    let light = lod_light_layers(&sections, layout, sky_light);
    let mut states = BTreeMap::new();
    let mut indices = [0u16; 4096];
    for (section_y, section) in sections {
        if let Some(blocks) = section.get("block_states") {
            let Tag::List(palette) = blocks.get("palette")? else {
                return None;
            };
            // Resolve each state once per palette, rather than parsing NBT for every block.
            let palette: Vec<_> = palette
                .iter()
                .map(|entry| {
                    let descriptor = BlockState::from_nbt(entry).ok()?;
                    let state = *states.entry(descriptor.clone()).or_insert_with(|| {
                        assets
                            .registry
                            .id(&descriptor)
                            .or_else(|| assets.registry.default_state(&descriptor.name))
                    });
                    Some(state.map(|state| {
                        let properties = assets.properties.heightmap_state(state);
                        let fluid = matches!(
                            descriptor.name.as_str(),
                            "minecraft:water" | "minecraft:lava"
                        );
                        // Chorus is thin and non-solid but still far-End detail.
                        let chorus = matches!(
                            descriptor.name.as_str(),
                            "minecraft:chorus_plant" | "minecraft:chorus_flower"
                        );
                        let solid = properties.blocks_motion_in_heightmap();
                        // Kelp and seagrass: the water they hold. Left out, the
                        // water stopped short where they reach the surface.
                        match water_state {
                            Some(water) if !fluid && !solid && properties.has_fluid() => {
                                (water, true, true)
                            }
                            _ => (state, fluid, fluid || solid || chorus),
                        }
                    }))
                })
                .collect::<Option<_>>()?;
            let length = lod_palette_indices(blocks, &mut indices, 4)?;
            if indices.iter().any(|&index| usize::from(index) >= length) {
                return None;
            }
            // The air above the ground, mostly: nothing here is far terrain.
            if !palette.iter().flatten().any(|&(_, _, kept)| kept) {
                continue;
            }
            // Each 4x4x4 cell's biome; 0 where the section's biomes do not say.
            let mut cell_biomes = [0u16; 64];
            if let Some(container) = section.get("biomes")
                && let Some(Tag::List(names)) = container.get("palette")
                && lod_palette_indices(container, &mut cell_biomes, 1).is_some()
            {
                let ids: Vec<u16> = names
                    .iter()
                    .map(|entry| match entry {
                        Tag::String(name) => biomes.id(&name.to_string_lossy()),
                        _ => 0,
                    })
                    .collect();
                for biome in &mut cell_biomes {
                    *biome = ids.get(usize::from(*biome)).copied().unwrap_or(0);
                }
            }
            for z in 0..16 {
                for x in 0..16 {
                    let cell = z * 16 + x;
                    let column = &mut columns[cell];
                    for y in (0..16).rev() {
                        let Some((state, fluid, true)) =
                            palette[usize::from(indices[y * 256 + cell])]
                        else {
                            continue;
                        };
                        let biome = cell_biomes[(y / 4) * 16 + (z / 4) * 4 + x / 4];
                        let height = section_y * 16 + y as i32 + 1;
                        if column.surface.is_none() {
                            if fluid {
                                water[cell].get_or_insert((height, state));
                            } else {
                                column.surface = Some(crate::columns::Surface {
                                    height,
                                    state,
                                    water: water[cell],
                                    biome,
                                });
                            }
                        }
                        let light =
                            lod_face_light(&light, x, (height - 1 - layout.min_y()) as usize, z);
                        let min = (height - 1) as i16;
                        let max = height as i16;
                        if let Some(previous) = column.spans.last_mut()
                            && previous.min == max
                            && (previous.state, previous.biome, previous.light)
                                == (state, biome, light)
                        {
                            previous.min = min;
                        } else {
                            column.spans.push(sunrise_level::lod_data::Span {
                                min,
                                max,
                                state,
                                biome,
                                light,
                            });
                        }
                    }
                }
            }
        }
    }
    // Spans are pushed top down, so the reverse puts the topmost last, as
    // `Column::from_chunk` leaves them. Water keeps the chunk's own light,
    // a level less per block, as the generator's far columns shade it.
    for column in &mut *columns {
        column.spans.reverse();
    }
    Some(Arc::from(columns))
}

impl<G: ChunkGenerator> ChunkGenerator for Persistent<G> {
    type Worker = G::Worker;
    fn spawn_height(&self) -> i32 {
        self.inner.spawn_height()
    }
    fn worker(&self) -> G::Worker {
        let save = self.save.clone();
        self.inner.set_saved_starts(Arc::new(move |pos| {
            let root = save.raw_chunk(pos.x, pos.z)?;
            match root.tag.get("structures")?.get("starts")? {
                Tag::Compound(starts) => Some(starts.clone()),
                _ => None,
            }
        }));
        self.inner.worker()
    }
    fn lod_sampler(self: Arc<Self>) -> Box<dyn sunrise_level::chunk_streamer::LodSampler> {
        let inner = ChunkGenerator::lod_sampler(self.inner.clone());
        let generated = Arc::new(sunrise_level::lod_data::cache::DiskCache::new(
            self.save.root.join("data/sunrise_far_terrain"),
            sunrise_level::lod_data::cache::build_key(
                self.registry.registry.len(),
                self.biomes.list.len(),
            ),
        ));
        let bake = sunrise_level::lod_data::bake::LodBake::open(
            &self.save.root,
            self.save.shape,
            self.registry.registry.len(),
            self.biomes.list.len(),
        )
        .map(Arc::new);
        Box::new(SavedLodSampler {
            generator: self,
            inner,
            cache: Default::default(),
            details: Vec::new(),
            approximate: false,
            generated,
            bake,
        })
    }
    fn load(&self, x: i32, z: i32) -> Option<Chunk> {
        self.save
            .tables
            .get_or_init(|| Arc::new(SaveTables::new(&self.registry)));
        let mut chunk = self
            .save
            .try_load(x, z, self.registry.registry, &self.biomes)
            .ok()
            .flatten()?;
        for s in &mut chunk.sections {
            Arc::make_mut(s).compact();
        }
        chunk.disk_entities = self.save.load_entities(x, z).unwrap_or_default();
        Some(chunk)
    }
    fn accept_loaded_chunk(&self, chunk: &mut Chunk) {
        self.save
            .load_disk_attachments(chunk, mod_api::ChunkDiskKind::Full);
    }
    fn lod_surface(
        &self,
        worker: &mut G::Worker,
        x: i32,
        z: i32,
    ) -> Option<crate::columns::Surface> {
        // Resident/visited summaries override this procedural fallback in the LOD cache.
        self.inner.lod_surface(worker, x, z)
    }
    fn prepare_loaded(&self, worker: &mut G::Worker, chunk: &Chunk) {
        self.inner.prepare_loaded(worker, chunk);
    }
    fn all_idle(&self) {
        self.inner.all_idle();
    }
    fn idle(&self, worker: &mut G::Worker) {
        self.inner.idle(worker);
    }
    fn proto(
        &self,
        worker: &mut G::Worker,
        x: i32,
        z: i32,
    ) -> std::result::Result<sunrise_level::chunk_streamer::Proto, String> {
        let (mut proto, saved) = self.worker_proto(worker, x, z)?;
        if let Some(saved) = saved {
            self.accept_loaded_proto(&mut proto, *saved);
        }
        Ok(proto)
    }
    fn worker_proto(
        &self,
        worker: &mut G::Worker,
        x: i32,
        z: i32,
    ) -> std::result::Result<
        (
            sunrise_level::chunk_streamer::Proto,
            Option<Box<SavedProto>>,
        ),
        String,
    > {
        let mut proto = self.inner.proto(worker, x, z)?;
        let Some(saved) = self
            .save
            .decode_saved_proto(x, z, self.registry.registry, &self.biomes)
        else {
            return Ok((proto, None));
        };
        if self.save.has_chunk_disk_hook(mod_api::ChunkDiskPhase::Load) {
            Ok((proto, Some(Box::new(saved))))
        } else {
            self.inner.restore_proto(&mut proto, saved);
            Ok((proto, None))
        }
    }
    fn accept_loaded_proto(
        &self,
        proto: &mut sunrise_level::chunk_streamer::Proto,
        mut saved: SavedProto,
    ) -> bool {
        self.save
            .load_disk_attachments(&mut saved.column, mod_api::ChunkDiskKind::Proto);
        self.inner.restore_proto(proto, saved);
        self.inner.is_decorated(proto)
    }
    fn is_decorated(&self, proto: &sunrise_level::chunk_streamer::Proto) -> bool {
        self.inner.is_decorated(proto)
    }
    fn light_view(&self, proto: &sunrise_level::chunk_streamer::Proto) -> Option<Chunk> {
        self.inner.light_view(proto)
    }
    fn non_air_sections(
        &self,
        proto: &sunrise_level::chunk_streamer::Proto,
    ) -> Option<crate::section_mask::SectionMask> {
        self.inner.non_air_sections(proto)
    }
    fn cache_census(&self) -> Vec<(&'static str, usize, Option<usize>)> {
        self.inner.cache_census()
    }
    fn proto_heap_bytes(
        &self,
        proto: &sunrise_level::chunk_streamer::Proto,
    ) -> ([usize; 6], [usize; 16]) {
        self.inner.proto_heap_bytes(proto)
    }
    fn save_protos_on_owner(&self) -> bool {
        self.save.has_chunk_disk_hook(mod_api::ChunkDiskPhase::Save)
    }
    fn save_protos(&self, protos: &[&sunrise_level::chunk_streamer::Proto]) {
        self.save
            .tables
            .get_or_init(|| Arc::new(SaveTables::new(&self.registry)));
        let saved: Vec<SavedProto> = protos
            .iter()
            .filter_map(|p| self.inner.saved_proto(p))
            .collect();
        if let Err(e) = self
            .save
            .save_protos(saved, self.registry.registry, &self.biomes)
        {
            log::warn!("Sunrise: failed to save proto chunks: {e}");
        }
    }
    fn save_finished(&self, mut chunk: Chunk) {
        self.save
            .tables
            .get_or_init(|| Arc::new(SaveTables::new(&self.registry)));
        let key = (chunk.x, chunk.z);
        let entities = std::mem::take(&mut chunk.entities);
        chunk.disk_entities.clear();
        // This chunk did not enter the level. Add only the new generation
        // records; its separate entity region may predate the terrain.
        let saved = self
            .save
            .save_later(&[Arc::new(chunk)], self.registry.registry, &self.biomes)
            .and_then(|_| {
                if !entities.is_empty()
                    && let Ok(mut stored) = self.save.load_entities(key.0, key.1)
                {
                    stored.extend(entities);
                    self.save.save_entities_later(vec![(key, stored)])?;
                }
                Ok(())
            });
        if let Err(e) = saved {
            log::warn!("Sunrise: failed to save chunk {} {}: {e}", key.0, key.1);
        }
    }
    fn decorates(&self) -> bool {
        self.inner.decorates()
    }
    fn decorates_beside_final(&self) -> bool {
        self.inner.decorates_beside_final()
    }
    fn proto_beside_final(
        &self,
        worker: &mut G::Worker,
        snapshot: &Chunk,
    ) -> Option<sunrise_level::chunk_streamer::Proto> {
        self.inner.proto_beside_final(worker, snapshot)
    }
    fn final_writes(
        &self,
        snapshot: &Chunk,
        proto: &sunrise_level::chunk_streamer::Proto,
    ) -> Vec<([i32; 3], u32)> {
        self.inner.final_writes(snapshot, proto)
    }
    fn final_block_entities(
        &self,
        entries: &[(&sunrise_level::chunk_streamer::Proto, &[([i32; 3], u32)])],
    ) -> Vec<((i32, i32, i32), crate::chunk::block_entity::BlockEntity)> {
        self.inner.final_block_entities(entries)
    }
    fn decorate(
        &self,
        worker: &mut G::Worker,
        region: &mut [sunrise_level::chunk_streamer::Proto; 9],
    ) {
        self.inner.decorate(worker, region);
    }
    fn decorate_with_outer(
        &self,
        worker: &mut G::Worker,
        region: &mut [sunrise_level::chunk_streamer::Proto; 9],
        outer: &[Arc<Chunk>],
    ) {
        self.inner.decorate_with_outer(worker, region, outer);
    }
    fn finish(&self, proto: sunrise_level::chunk_streamer::Proto) -> Chunk {
        let mut chunk = self.inner.finish(proto);
        // EntityStorage is independent of terrain: a newly generated column
        // can already have a saved entity section.
        chunk.disk_entities = self
            .save
            .load_entities(chunk.x, chunk.z)
            .unwrap_or_default();
        chunk
    }
}
