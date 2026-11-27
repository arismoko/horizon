//! Speculative decoration is isolated from gameplay and never writes normal saves.
use super::*;
use crate::{
    columns::Surface,
    fast_hash::{FxHashMap, FxHashSet},
};
use std::{
    collections::VecDeque,
    sync::{
        Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};
use sunrise_level::{
    chunk_streamer::LodDetail,
    lod_data::{Column, Span},
};
use sunrise_world::{
    heightmap::HeightmapState,
    registry::BlockStateId,
    worldgen::{
        decoration_order::vanilla_order,
        generator::{GenBlockStates, GeneratorError},
    },
};

const CACHE: usize = 1024;
/// Chunks a side a decoration batch emits. Each needs a two-chunk border of
/// proto chunks, which the batch shares: 4 built four protos per chunk, 8
/// builds 2.25, and the protos (noise, surface) are most of far decoration.
const BATCH: i32 = 8;
fn batch_origin((x, z): (i32, i32)) -> (i32, i32) {
    (x.div_euclid(BATCH) * BATCH, z.div_euclid(BATCH) * BATCH)
}
#[derive(Default)]
struct Requests {
    /// What the far terrain wants decorated now (`Builder::request`), nearest
    /// first, short of what is built or being built: the threads take their
    /// batches from the front.
    wanted: VecDeque<(i32, i32)>,
    /// The batches the decoration threads are working on.
    current: FxHashSet<(i32, i32)>,
    finished: FxHashSet<(i32, i32)>,
}
impl Requests {
    /// `wanted` becomes `chunks` nearest `center` first, short of the
    /// chunks built and the batches being built.
    fn replace_wanted(&mut self, chunks: &[(i32, i32)], center: [i32; 2]) {
        let Self {
            wanted,
            current,
            finished,
        } = self;
        wanted.clear();
        wanted.extend(
            chunks
                .iter()
                .copied()
                .filter(|&key| !finished.contains(&key) && !current.contains(&batch_origin(key))),
        );
        wanted
            .make_contiguous()
            .sort_unstable_by_key(|&key| (distance(key, center), key));
    }
}
struct Shared {
    requests: Mutex<Requests>,
    wake: Condvar,
    stop: AtomicBool,
}
fn distance((x, z): (i32, i32), center: [i32; 2]) -> i64 {
    let dx = i64::from(x * 16 + 8 - center[0]);
    let dz = i64::from(z * 16 + 8 - center[1]);
    dx * dx + dz * dz
}
struct Builder {
    shared: Arc<Shared>,
    results: mpsc::Receiver<LodDetail>,
    threads: Vec<thread::JoinHandle<()>>,
}
impl Builder {
    fn new(terrain: Arc<VanillaTerrain>, background: bool) -> Self {
        let shared = Arc::new(Shared {
            requests: Mutex::new(Requests::default()),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        let (tx, results) = mpsc::channel();
        // Idle priority, and within far terrain's share of the cores
        // (`lod::decoration_threads`).
        let count = sunrise_level::lod_data::decoration_threads(sunrise_threads::worker_count());
        let threads = (0..count)
            .map(|_| Self::spawn(terrain.clone(), shared.clone(), tx.clone(), background))
            .collect();
        Self {
            shared,
            results,
            threads,
        }
    }
    fn spawn(
        terrain: Arc<VanillaTerrain>,
        worker: Arc<Shared>,
        tx: mpsc::Sender<LodDetail>,
        background: bool,
    ) -> thread::JoinHandle<()> {
        std::thread::Builder::new()
            .stack_size(sunrise_threads::GAME_THREAD_STACK_BYTES)
            .name("Far decoration".into())
            .spawn(move || {
                if background {
                    sunrise_threads::idle_current();
                }
                // A superflat world's far ring is its flat ground (`FlatTerrain::lod_surface`);
                // a normal overworld rebuilt here would draw hills, oceans and trees.
                if !terrain.noise_ground() {
                    worker.stop.store(true, Ordering::Relaxed);
                    return;
                }
                // Structure pieces have mutable one-shot placement flags. Sharing
                // gameplay's seeded pipeline would consume its chests/spawners.
                let generator = match OverworldGenerator::for_dimension(
                    terrain.generator.data().clone(),
                    terrain.seed,
                    &terrain.preset,
                    &terrain.dimension,
                ) {
                    Ok(generator) => generator,
                    Err(error) => {
                        log::warn!("Sunrise: far decoration unavailable: {error}");
                        worker.stop.store(true, Ordering::Relaxed);
                        return;
                    }
                };
                let mut state = generator.worker();
                loop {
                    let key = {
                        let mut requests = worker.requests.lock().unwrap();
                        let origin = loop {
                            if worker.stop.load(Ordering::Relaxed) {
                                break None;
                            }
                            if let Some(&key) = requests.wanted.front() {
                                break Some(batch_origin(key));
                            }
                            requests = worker.wake.wait(requests).unwrap();
                        };
                        let Some(origin) = origin else { break };
                        requests.wanted.retain(|&key| batch_origin(key) != origin);
                        requests.current.insert(origin);
                        origin
                    };
                    let result = decorated_batch(&mut state, key, BATCH, &worker.stop, |chunk| {
                        let pos = chunk.position();
                        let key = (pos.x, pos.z);
                        worker.requests.lock().unwrap().finished.insert(key);
                        let columns = columns(chunk, &terrain.biome_map, &terrain.dimension);
                        if tx.send((key, columns)).is_err() {
                            worker.stop.store(true, Ordering::Relaxed);
                        }
                    });
                    worker.requests.lock().unwrap().current.remove(&key);
                    match result {
                        Ok(_) => {}
                        Err(error) => {
                            log::warn!(
                                "Sunrise: far decoration failed at {} {}: {error}",
                                key.0,
                                key.1
                            );
                            worker.stop.store(true, Ordering::Relaxed);
                            break;
                        }
                    }
                }
            })
            .expect("far decoration worker")
    }
    /// The chunks the far terrain draws without their detail now. `Lod`
    /// asks again for all it still misses every few hundred milliseconds, so
    /// a request stands for the whole of what the view wants: chunks earlier
    /// ones asked for and this one leaves out (flown past, out of view, under
    /// the near chunks) are not built.
    fn request(&self, chunks: &[(i32, i32)], center: [i32; 2]) {
        if self.shared.stop.load(Ordering::Relaxed) {
            return;
        }
        let mut requests = self.shared.requests.lock().unwrap();
        requests.replace_wanted(chunks, center);
        drop(requests);
        self.shared.wake.notify_all();
    }
}
impl Drop for Builder {
    fn drop(&mut self) {
        // Under the lock the threads check it with: one between its check and
        // its wait missed the wake, and closing the world waited forever.
        let requests = self.shared.requests.lock();
        self.shared.stop.store(true, Ordering::Relaxed);
        drop(requests);
        self.shared.wake.notify_all();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

/// Share the two-chunk terrain halo and one-chunk feature halo across adjacent
/// targets. Emit each target only after all nine feature centers have run.
/// These proto chunks never enter World, lighting or normal save queues.
fn decorated_batch(
    worker: &mut GeneratorWorker,
    origin: (i32, i32),
    size: i32,
    stop: &AtomicBool,
    mut emit: impl FnMut(&ProtoChunk),
) -> Result<FxHashMap<(i32, i32), ProtoChunk>, GeneratorError> {
    // Each region starts afresh; flags from a previous speculative structure
    // placement must not suppress blocks in this region.
    worker.forget_structure_starts();
    let mut chunks = FxHashMap::default();
    for dx in -2..size + 2 {
        for dz in -2..size + 2 {
            if stop.load(Ordering::Relaxed) {
                return Ok(FxHashMap::default());
            }
            let key = (origin.0 + dx, origin.1 + dz);
            chunks.insert(key, worker.generate_proto(ChunkPos::new(key.0, key.1))?);
        }
    }
    let mut remaining: FxHashMap<_, u8> = (0..size)
        .flat_map(|x| (0..size).map(move |z| ((origin.0 + x, origin.1 + z), 9)))
        .collect();
    let center = (origin.0 + size / 2, origin.1 + size / 2);
    for (x, z) in vanilla_order(center, size / 2)
        .into_iter()
        .filter(|&(x, z)| {
            x >= origin.0 - 1 && x <= origin.0 + size && z >= origin.1 - 1 && z <= origin.1 + size
        })
    {
        if stop.load(Ordering::Relaxed) {
            return Ok(FxHashMap::default());
        }
        let mut region: [ProtoChunk; 9] = std::array::from_fn(|i| {
            chunks
                .remove(&(x + i as i32 % 3 - 1, z + i as i32 / 3 - 1))
                .expect("generated neighborhood")
        });
        worker.decorate(&mut region)?;
        for chunk in region {
            let pos = chunk.position();
            chunks.insert((pos.x, pos.z), chunk);
        }
        for dx in -1..=1 {
            for dz in -1..=1 {
                let key = (x + dx, z + dz);
                if let Some(left) = remaining.get_mut(&key) {
                    *left -= 1;
                    if *left == 0 {
                        emit(&chunks[&key]);
                    }
                }
            }
        }
    }
    assert!(
        remaining.values().all(|&left| left == 0),
        "incomplete far decoration halo"
    );
    Ok(chunks)
}

#[cfg(test)]
fn decorated_region(
    worker: &mut GeneratorWorker,
    center: (i32, i32),
    stop: &AtomicBool,
) -> Result<Option<ProtoChunk>, GeneratorError> {
    Ok(decorated_batch(worker, center, 1, stop, |_| {})?.remove(&center))
}
fn columns(chunk: &ProtoChunk, biome_map: &[u16], dimension: &str) -> Arc<[Column; 256]> {
    let states = chunk.data().block_states();
    let registry = chunk.data().registry();
    // The level's range, all of it inside its column; the column's bottom
    // stands for "no surface".
    let (min, height) = (chunk.terrain.min_y(), chunk.terrain.height());
    let max = min + height;
    let bottom = crate::chunk::ColumnLayout::of_level(min, height).min_y();
    let water = registry.default_state("minecraft:water");
    Arc::new(std::array::from_fn(|index| {
        let x = (index & 15) as i32;
        let z = (index >> 4) as i32;
        let mut column = Column::default();
        for y in min..max {
            let mut state = chunk.block(x, y, z);
            let properties = states.get(state);
            let mut name = registry.state(state).map_or("", |s| s.name.as_str());
            if properties.is_air()
                || (!properties.blocks_motion_in_heightmap()
                    && !properties.has_fluid()
                    && !matches!(name, "minecraft:chorus_plant" | "minecraft:chorus_flower"))
            {
                continue;
            }
            // Kelp and seagrass hold water: far off they are the water around
            // them, as saved chunks' far columns have them. As spans of their
            // own they hid the seabed with no top face of theirs, a hole
            // where the sky showed through (white dots across the sea).
            if !properties.blocks_motion_in_heightmap()
                && properties.has_fluid()
                && !matches!(name, "minecraft:water" | "minecraft:lava")
                && let Some(water) = water
            {
                state = water;
                name = "minecraft:water";
            }
            let fluid = matches!(name, "minecraft:water" | "minecraft:lava");
            let biome = biome_map
                .get(chunk.biome_id_at(x, y, z).0 as usize)
                .copied()
                .unwrap_or(0);
            // shortcut: no propagated far block light, use the normal lighting
            // engine if accurate distant block-light propagation is required.
            // Skylight is `shade_sky`'s, once the column is complete.
            let light = if dimension == "minecraft:the_nether" {
                0
            } else {
                240
            } | if name == "minecraft:lava" {
                15 << 12
            } else {
                0
            };
            column.push(Span {
                min: y as i16,
                max: (y + 1) as i16,
                state,
                biome,
                light,
            });
            if fluid {
                let surface = column.surface.get_or_insert(Surface {
                    height: bottom,
                    state: BlockStateId(0),
                    water: None,
                    biome,
                });
                surface.water = Some((y + 1, state));
            } else {
                column.surface = Some(Surface {
                    height: y + 1,
                    state,
                    water: None,
                    biome,
                });
            }
        }
        if dimension != "minecraft:the_nether" {
            shade_sky(&mut column, states, Dampening::PerLevel);
        }
        column
    }))
}
/// How much of its own dampening a run records, for [`shade_sky`]. Either
/// way the light it passes on to the spans under it is the same, so seabeds
/// and the ground under canopies are lit identically.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Dampening {
    /// A span a level, as a real chunk's light has them. A column that
    /// `Column::reduced` merges with its neighbours needs this: a reduction
    /// takes the brightest light of each slice it merges, so one bright span
    /// for a whole sea lit its bed as brightly as its surface.
    PerLevel,
    /// One span for the run, lit as its top is. A previewed column already
    /// stands for a whole far cell and is never reduced against neighbouring
    /// columns, so nothing merges a run with the bed under it, and the levels
    /// between its top and its bottom reach no face that is drawn: the spans
    /// inside a run have neither a top nor a bottom face (`hides` matches on
    /// state, not light), and the side of a run is exposed at its top, where
    /// the light is the run's brightest -- below that the neighbour hides it,
    /// and above it `add_side` raises the light to the neighbour's own sky.
    ///
    /// Over real seed-1234 terrain, nine tiles each at levels 0, 2, 4 and 6,
    /// `PerLevel` leaves 18.7, 12.3, 12.0 and 12.6 spans a column where the
    /// runs themselves are 4.1 to 4.5: 64% to 78% of every stored far span
    /// was a gradient step, and a sea 15 blocks deep or more spent 16 spans
    /// on one.
    PerRun,
}
/// Skylight as it reaches the top of each span straight down from the sky:
/// fluids and leaves take a level per block, as vanilla's dampening does.
/// Full skylight everywhere lit seabeds and the ground under canopies as if
/// they stood in the open.
// ponytail: straight down only. Solid blocks take none, though vanilla's
// stop it: the faces under them that far terrain shows are cliffs and
// overhangs, which vanilla lights sideways from the open air, and they went
// black. The lighting engine for exact light.
fn shade_sky(column: &mut Column, states: &GenBlockStates, dampening: Dampening) {
    shade_runs(
        column,
        |id| {
            let state = states.get(id);
            // A solid block stops no light here, as the note above says; a
            // fluid or leaves take a level a block.
            !(state.motion && state.no_leaves)
        },
        dampening,
    );
}
/// [`shade_sky`] over a predicate rather than the generator's block states,
/// which have no constructor outside `sunrise-world`.
fn shade_runs(column: &mut Column, dampens: impl Fn(BlockStateId) -> bool, dampening: Dampening) {
    let mut sky = 15u32;
    let mut shaded = Vec::with_capacity(column.spans.len());
    for span in column.spans.iter().rev() {
        let lit = |min, max, sky: u32| Span {
            min,
            max,
            light: (span.light & !0xf0) | (sky << 4) as u16,
            ..*span
        };
        if !dampens(span.state) {
            shaded.push(lit(span.min, span.max, sky));
            continue;
        }
        if dampening == Dampening::PerRun {
            shaded.push(lit(span.min, span.max, sky));
            // What the per-level walk below leaves, so the spans under the
            // run are lit the same either way.
            let height = i32::from(span.max) - i32::from(span.min);
            sky = sky.saturating_sub(height.max(0) as u32);
            continue;
        }
        let mut top = span.max;
        while top > span.min {
            let bottom = if sky == 0 { span.min } else { top - 1 };
            shaded.push(lit(bottom, top, sky));
            sky = sky.saturating_sub((top - bottom) as u32);
            top = bottom;
        }
    }
    shaded.reverse();
    column.spans = shaded.into();
}

/// A wide cell's column is one point of the terrain: a cave, an overhang's
/// lip or the water an aquifer-less sample puts in caves under sea level stood
/// for the whole cell, a slab hanging over air or a band of water in a cliff
/// once its neighbours were lower. Under the top solid span the column is
/// ground: gaps take the span below, buried fluids the ground under them.
fn fill_under_surface(column: &mut Column, fluid: impl Fn(BlockStateId) -> bool) {
    let Some(top) = column.spans.iter().rposition(|span| !fluid(span.state)) else {
        return;
    };
    let Some(ground) = column.spans[..=top].iter().find(|span| !fluid(span.state)) else {
        return;
    };
    let mut state = ground.state;
    let mut filled = Column {
        surface: column.surface,
        spans: sunrise_level::lod_data::Spans::with_capacity(column.spans.len()),
    };
    for (index, &span) in column.spans.iter().enumerate() {
        let mut span = span;
        if index <= top {
            if fluid(span.state) {
                span.state = state;
            } else {
                state = span.state;
            }
            if let Some(below) = filled.spans.last_mut()
                && below.max < span.min
            {
                below.max = span.min;
            }
        }
        filled.push(span);
    }
    *column = filled;
}

/// A whole tile inside visited chunks needs no density work at all. This
/// used to evaluate the lattice first and throw all of it away in the per-cell
/// cache check below.
fn cached_grid(
    cache: &FxHashMap<(i32, i32), Arc<[Column; 256]>>,
    x: i32,
    z: i32,
    width: i32,
    side: usize,
) -> Option<Vec<Column>> {
    if width > 16 || cache.is_empty() {
        return None;
    }
    let mut columns = Vec::with_capacity(side * side);
    for dz in 0..side {
        for dx in 0..side {
            let (wx, wz) = (x + dx as i32 * width, z + dz as i32 * width);
            let cached = cache.get(&(wx >> 4, wz >> 4))?;
            columns.push(Column::reduced(
                cached,
                (wx & 15) as usize,
                (wz & 15) as usize,
                width as usize,
            ));
        }
    }
    Some(columns)
}

pub(super) struct VanillaLodSampler {
    generator: Arc<VanillaTerrain>,
    worker: GeneratorWorker,
    builder: Option<Builder>,
    cache: FxHashMap<(i32, i32), Arc<[Column; 256]>>,
    order: VecDeque<(i32, i32)>,
    details: Vec<LodDetail>,
    /// Reduced far detail: `lod_grid` approximates wide cells.
    approximate: bool,
}
impl VanillaLodSampler {
    pub(super) fn new(generator: Arc<VanillaTerrain>) -> Self {
        let worker = generator.generator.worker();
        let builder = Some(Builder::new(generator.clone(), true));
        Self {
            generator,
            worker,
            builder,
            cache: FxHashMap::default(),
            order: VecDeque::new(),
            details: Vec::new(),
            approximate: false,
        }
    }
}
impl LodSampler for VanillaLodSampler {
    fn fork(&self) -> Option<Box<dyn LodSampler>> {
        Some(Box::new(Self {
            generator: self.generator.clone(),
            worker: self.generator.generator.worker(),
            builder: None,
            cache: FxHashMap::default(),
            order: VecDeque::new(),
            details: Vec::new(),
            approximate: self.approximate,
        }))
    }
    fn set_approximate(&mut self, approximate: bool) {
        self.approximate = approximate;
    }
    fn request_detail(&mut self, chunks: &[(i32, i32)], center: [i32; 2]) {
        let mut missing = Vec::new();
        for &key in chunks {
            if let Some(columns) = self.cache.get(&key) {
                self.details.push((key, columns.clone()));
            } else {
                missing.push(key);
            }
        }
        if let Some(builder) = &self.builder {
            builder.request(&missing, center);
        }
    }
    fn drain_details(&mut self) -> Vec<LodDetail> {
        let Some(builder) = &self.builder else {
            return std::mem::take(&mut self.details);
        };
        while let Ok((key, columns)) = builder.results.try_recv() {
            if !self.cache.contains_key(&key) {
                self.order.push_back(key);
            }
            self.cache.insert(key, columns.clone());
            while self.cache.len() > CACHE {
                if let Some(old) = self.order.pop_front() {
                    self.cache.remove(&old);
                    builder
                        .shared
                        .requests
                        .lock()
                        .unwrap()
                        .finished
                        .remove(&old);
                }
            }
            self.details.push((key, columns));
        }
        std::mem::take(&mut self.details)
    }
    fn sample(&mut self, x: i32, z: i32) -> Option<Surface> {
        self.column(x, z).surface
    }
    fn column(&mut self, x: i32, z: i32) -> Column {
        self.reduced_column(x, z, 1)
    }
    fn reduced_column(&mut self, x: i32, z: i32, width: i32) -> Column {
        if width <= 16
            && let Some(columns) = self.cache.get(&(x >> 4, z >> 4))
        {
            return Column::reduced(
                columns,
                (x & 15) as usize,
                (z & 15) as usize,
                width as usize,
            );
        }
        self.grid(x, z, width, 1).pop().unwrap_or_default()
    }
    fn grid(&mut self, x: i32, z: i32, width: i32, side: usize) -> Vec<Column> {
        if let Some(columns) = cached_grid(&self.cache, x, z, width, side) {
            return columns;
        }
        let samples =
            match self
                .worker
                .lod_grid(x + width / 2, z + width / 2, width, side, self.approximate)
            {
                Ok(samples) => samples,
                Err(error) => {
                    log::warn!("Sunrise: direct far sampling failed: {error}");
                    return vec![Column::default(); side * side];
                }
            };
        samples
            .into_iter()
            .enumerate()
            .map(|(index, (surface, spans))| {
                let (wx, wz) = (
                    x + (index % side) as i32 * width,
                    z + (index / side) as i32 * width,
                );
                if width <= 16
                    && let Some(columns) = self.cache.get(&(wx >> 4, wz >> 4))
                {
                    return Column::reduced(
                        columns,
                        (wx & 15) as usize,
                        (wz & 15) as usize,
                        width as usize,
                    );
                }
                self.preview_column(width, surface, spans)
            })
            .collect()
    }
}
impl VanillaLodSampler {
    fn preview_column(
        &self,
        width: i32,
        surface: Option<SurfaceSample>,
        spans: Vec<(i32, i32, BlockStateId)>,
    ) -> Column {
        let surface = surface.map(|sample| self.generator.client_surface(sample));
        let biome = surface.map_or(0, |surface| surface.biome);
        let registry = self.generator.generator.data().registry();
        let mut column = Column {
            surface,
            spans: sunrise_level::lod_data::Spans::new(),
        };
        for (min, max, state) in spans.into_iter().rev() {
            column.push(Span {
                min: min as i16,
                max: max as i16,
                state,
                biome,
                light: if self.generator.dimension == "minecraft:the_nether" {
                    0
                } else {
                    240
                } | if registry
                    .state(state)
                    .is_some_and(|s| s.name == "minecraft:lava")
                {
                    15 << 12
                } else {
                    0
                },
            });
        }
        let nether = self.generator.dimension == "minecraft:the_nether";
        if width > 1 && self.generator.dimension == "minecraft:overworld" {
            let states = self.generator.generator.data().block_states();
            fill_under_surface(&mut column, |state| states.get(state).has_fluid());
        }
        if !nether {
            shade_sky(
                &mut column,
                self.generator.generator.data().block_states(),
                Dampening::PerRun,
            );
        }
        column
    }
}

#[cfg(test)]
mod tests {
    use sunrise_world::native_load_context::DataLoadOrigin;
    /// The all-cached path must return exactly the per-cell reductions the
    /// old override produced, including negative chunk coordinates.
    #[test]
    fn cached_grid_equals_each_cells_reduction() {
        let mut cache: FxHashMap<(i32, i32), Arc<[Column; 256]>> = FxHashMap::default();
        let columns: Arc<[Column; 256]> = Arc::new(std::array::from_fn(|i| {
            Column::from_surface(Some(Surface {
                height: 60 + i as i32 % 8,
                state: BlockStateId(i as u32 % 3 + 1),
                water: None,
                biome: 0,
            }))
        }));
        cache.insert((0, 0), columns.clone());
        cache.insert((-1, -1), columns.clone());
        for (x, z) in [(0, 0), (4, 4), (8, 8), (-8, -8), (-16, -16)] {
            let fast = cached_grid(&cache, x, z, 4, 2)
                .unwrap_or_else(|| panic!("inside cached chunks at {x},{z}"));
            for dz in 0..2 {
                for dx in 0..2 {
                    let (wx, wz) = (x + dx * 4, z + dz * 4);
                    assert_eq!(
                        fast[dz as usize * 2 + dx as usize],
                        Column::reduced(&columns, (wx & 15) as usize, (wz & 15) as usize, 4)
                    );
                }
            }
        }
        // One missing chunk falls back to the generator path.
        assert!(cached_grid(&cache, 12, 0, 4, 2).is_none());
        assert!(cached_grid(&cache, 0, 0, 32, 2).is_none());
    }

    use super::*;
    /// Flying on, the decoration threads build what the view wants now, nearest
    /// first: what an earlier request asked for is not kept. The builder has no
    /// threads here: the test reads the queue they take from.
    #[test]
    fn decoration_takes_the_latest_request_nearest_first() {
        let builder = Builder {
            shared: Arc::new(Shared {
                requests: Mutex::new(Requests::default()),
                wake: Condvar::new(),
                stop: AtomicBool::new(false),
            }),
            results: mpsc::channel().1,
            threads: Vec::new(),
        };
        let wanted = || Vec::from(builder.shared.requests.lock().unwrap().wanted.clone());
        builder.request(&[(-3, 0), (-2, 1), (20, 0)], [0, 0]);
        {
            // A thread builds the batch at 24..32 and has emitted (17, 2) already.
            let mut requests = builder.shared.requests.lock().unwrap();
            requests.current.insert((24, 0));
            requests.finished.insert((17, 2));
        }
        // The player flew east: the request leaves the chunks behind out.
        builder.request(&[(40, 5), (25, 3), (17, 2), (20, 0), (30, 1)], [320, 0]);
        assert_eq!(wanted(), [(20, 0), (40, 5)]);
        // The view wants nothing: nothing is left to build.
        builder.request(&[], [0, 0]);
        assert!(wanted().is_empty());
    }
    #[test]
    #[ignore = "daily"]
    fn decorated_lod_refines_real_seed_canopies_and_all_dimensions() -> crate::Result<()> {
        let root = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES").expect("resources"),
        );
        let assets = crate::assets::Assets::load(&root)?;
        let biomes = crate::biome::Biomes::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
        )?;
        let item_data = crate::test_resources::item_data(&root, DataLoadOrigin::ServerDataLoad)?;
        let terrain_resources = crate::vanillagen::TerrainResources {
            data: None,
            dir: &root.join("worldgen"),
            registry: assets.registry,
            biomes: &biomes,
            item_data: &item_data,
        };
        let terrain = Arc::new(VanillaTerrain::new(terrain_resources, 1234)?);
        let mut sampler = VanillaLodSampler::new(terrain.clone());
        // This bounded test waits for decoration. Idle workers can be starved
        // by other tests, so keep the real request/result path at normal priority.
        sampler.builder = Some(Builder::new(terrain.clone(), false));
        let mut fork = sampler.fork().expect("independent far sampler");
        let direct = sampler.grid(-1024, 0, 32, 16);
        assert_eq!(
            direct,
            fork.grid(-1024, 0, 32, 16),
            "sampling workers agree"
        );
        let key = (-64, 0);
        sampler.request_detail(&[key], [-1024, 0]);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let generated = loop {
            if let Some((_, columns)) = sampler.drain_details().into_iter().find(|(k, _)| *k == key)
            {
                break columns;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "decoration worker stalled"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        let count = |suffix: &str| {
            generated
                .iter()
                .flat_map(|column| &column.spans)
                .filter(|span| {
                    assets
                        .registry
                        .state(span.state)
                        .unwrap()
                        .name
                        .ends_with(suffix)
                })
                .map(|span| i32::from(span.max - span.min))
                .sum::<i32>()
        };
        assert!(count("_leaves") > 0, "unexplored forest has a real canopy");
        assert!(count("_log") > 0, "canopy retains its trunks");
        let sky = |span: &Span| (span.light >> 4) & 15;
        assert!(
            generated
                .iter()
                .filter_map(|column| column.spans.last())
                .all(|top| sky(top) == 15),
            "the top of every column is in the open"
        );
        assert!(
            generated
                .iter()
                .flat_map(|column| &column.spans)
                .any(|span| (1..15).contains(&sky(span))),
            "ground under the canopy is shaded, not black"
        );
        assert!(generated.iter().all(|column| {
            column
                .spans
                .windows(2)
                .all(|pair| pair[0].max <= pair[1].min)
        }));
        for z in 0..16 {
            for x in 0..16 {
                assert_eq!(
                    sampler.column(key.0 * 16 + x, key.1 * 16 + z),
                    generated[(z * 16 + x) as usize]
                );
            }
        }
        sampler.request_detail(&[key], [-1024, 0]);
        let cached = sampler.drain_details();
        assert!(
            Arc::ptr_eq(&cached[0].1, &generated),
            "reuses completed compact data"
        );
        drop(sampler);
        let mut worker = terrain.generator.worker();
        let started = std::time::Instant::now();
        for x in 0..BATCH {
            for z in 0..BATCH {
                decorated_region(&mut worker, (key.0 + x, key.1 + z), &AtomicBool::new(false))?
                    .unwrap();
            }
        }
        let separate = started.elapsed();
        let started = std::time::Instant::now();
        let mut emitted = FxHashMap::default();
        let region = decorated_batch(&mut worker, key, BATCH, &AtomicBool::new(false), |chunk| {
            let pos = chunk.position();
            assert!(
                emitted
                    .insert(
                        (pos.x, pos.z),
                        columns(chunk, &terrain.biome_map, "minecraft:overworld")
                    )
                    .is_none()
            );
        })?;
        eprintln!(
            "far generation: {} separate targets={separate:?}, shared targets={:?}",
            emitted.len(),
            started.elapsed()
        );
        assert_eq!(emitted.len(), (BATCH * BATCH) as usize);
        for (key, data) in emitted {
            assert_eq!(
                data,
                columns(&region[&key], &terrain.biome_map, "minecraft:overworld"),
                "emitted before all neighbouring features completed"
            );
        }
        assert_eq!(batch_origin((-1, -BATCH - 1)), (-BATCH, -2 * BATCH));
        let stop = AtomicBool::new(true);
        assert!(decorated_region(&mut terrain.generator.worker(), key, &stop)?.is_none());
        for (dimension, center) in [
            ("minecraft:the_nether", (0, 0)),
            ("minecraft:the_end", (16, 0)),
        ] {
            let generator = OverworldGenerator::for_dimension(
                terrain.generator.data().clone(),
                1234,
                "minecraft:normal",
                dimension,
            )?;
            let mut emitted = 0;
            let mut region = decorated_batch(
                &mut generator.worker(),
                center,
                BATCH,
                &AtomicBool::new(false),
                |_| emitted += 1,
            )?;
            assert_eq!(emitted, (BATCH * BATCH) as usize);
            let chunk = region.remove(&center).unwrap();
            let data = columns(&chunk, &terrain.biome_map, dimension);
            if dimension == "minecraft:the_nether" {
                assert!(
                    data.iter()
                        .flat_map(|c| &c.spans)
                        .all(|s| s.light & 0xf0 == 0)
                );
                assert!(data.iter().flat_map(|c| &c.spans).any(|s| {
                    assets.registry.state(s.state).unwrap().name == "minecraft:lava"
                        && s.light >> 12 == 15
                }));
            } else {
                assert!(
                    data.iter().all(|c| c.spans.is_empty()),
                    "End void stays empty"
                );
            }
        }
        // Reuse the saved-proto bastion fixture. Repeated speculative regions
        // must reconstruct structures after their one-shot placement flags ran.
        let generator = OverworldGenerator::for_dimension(
            terrain.generator.data().clone(),
            -1860902098141295193,
            "minecraft:normal",
            "minecraft:the_nether",
        )?;
        let mut worker = generator.worker();
        let mut previous = None;
        for _ in 0..2 {
            let chunk = decorated_region(&mut worker, (3, 7), &AtomicBool::new(false))?.unwrap();
            assert!(
                chunk
                    .structures()
                    .iter()
                    .any(|(name, _, _, _)| name.contains("bastion"))
            );
            let data = columns(&chunk, &terrain.biome_map, "minecraft:the_nether");
            let snapshot = (data, chunk.block_entities);
            if let Some(previous) = &previous {
                assert_eq!(
                    &snapshot, previous,
                    "speculative structure state leaked between regions"
                );
            }
            previous = Some(snapshot);
        }
        Ok(())
    }
    /// The refined path must keep the real feature's end stone in its columns.
    #[test]
    #[ignore = "daily"]
    fn end_small_islands_from_decoration_survive_far_columns() -> crate::Result<()> {
        let root = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES").expect("resources"),
        );
        let assets = crate::assets::Assets::load(&root)?;
        let biomes = crate::biome::Biomes::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
        )?;
        let item_data = crate::test_resources::item_data(&root, DataLoadOrigin::ServerDataLoad)?;
        let terrain_resources = crate::vanillagen::TerrainResources {
            data: None,
            dir: &root.join("worldgen"),
            registry: assets.registry,
            biomes: &biomes,
            item_data: &item_data,
        };
        let terrain = Arc::new(VanillaTerrain::for_dimension(
            terrain_resources,
            1234,
            "minecraft:the_end",
        )?);
        let mut worker = terrain.generator.worker();
        let stop = AtomicBool::new(false);
        let mut centers = Vec::new();
        'search: for cx in (64..=512).step_by(4) {
            for cz in (-512..=512).step_by(4) {
                let (x, z) = (cx * 16 + 8, cz * 16 + 8);
                let biome = worker.noise_biomes(&[[x >> 2, 16, z >> 2]]);
                if biome.first().and_then(|name| name.as_deref())
                    != Some("minecraft:small_end_islands")
                {
                    continue;
                }
                if worker.surface_sample(x, z).is_some() {
                    continue;
                }
                centers.push((cx, cz));
                if centers.len() >= 200 {
                    break 'search;
                }
            }
        }
        let end_stone = "minecraft:end_stone";
        for &(cx, cz) in &centers {
            let base = worker.generate_proto(sunrise_core::position::ChunkPos::new(cx, cz))?;
            let Some(chunk) = decorated_region(&mut worker, (cx, cz), &stop)? else {
                continue;
            };
            let min = chunk.terrain.min_y();
            let max = min + chunk.terrain.height();
            // The end stone decoration added: the island, not the terrain under it.
            let mut added = Vec::new();
            let mut raw = 0;
            for y in min..max {
                for z in 0..16 {
                    for x in 0..16 {
                        if chunk.block_name(x, y, z) == end_stone {
                            raw += 1;
                            if base.block_name(x, y, z) != end_stone {
                                added.push((x, y, z));
                            }
                        }
                    }
                }
            }
            if added.is_empty() {
                continue;
            }
            let far = columns(&chunk, &terrain.biome_map, "minecraft:the_end");
            let is_stone = |state| assets.registry.state(state).unwrap().name == end_stone;
            let kept: usize = far
                .iter()
                .flat_map(|column| &column.spans)
                .filter(|span| is_stone(span.state))
                .map(|span| (span.max - span.min) as usize)
                .sum();
            assert_eq!(
                raw, kept,
                "far columns keep the real small island in {cx} {cz}"
            );
            for &(x, y, z) in &added {
                assert!(
                    far[(z * 16 + x) as usize].spans.iter().any(|span| {
                        is_stone(span.state) && i32::from(span.min) <= y && y < i32::from(span.max)
                    }),
                    "decorated end stone at {x} {y} {z} in {cx} {cz} is in its far column"
                );
            }
            return Ok(());
        }
        panic!(
            "no decorated small end island among {} candidates",
            centers.len()
        );
    }
    /// Vanilla decoration places chorus plants in the End; the far columns must
    /// keep them even though they are not motion-blocking blocks.
    #[test]
    #[ignore = "daily"]
    fn end_far_columns_keep_real_chorus_plants() -> crate::Result<()> {
        let root = std::path::PathBuf::from(
            std::env::var_os("SUNRISE_TEST_RESOURCES").expect("resources"),
        );
        let assets = crate::assets::Assets::load(&root)?;
        let biomes = crate::biome::Biomes::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
        )?;
        let item_data = crate::test_resources::item_data(&root, DataLoadOrigin::ServerDataLoad)?;
        let terrain_resources = crate::vanillagen::TerrainResources {
            data: None,
            dir: &root.join("worldgen"),
            registry: assets.registry,
            biomes: &biomes,
            item_data: &item_data,
        };
        let terrain = Arc::new(VanillaTerrain::for_dimension(
            terrain_resources,
            1234,
            "minecraft:the_end",
        )?);
        let mut worker = terrain.generator.worker();
        let chorus = |state| {
            matches!(
                assets.registry.state(state).unwrap().name.as_str(),
                "minecraft:chorus_plant" | "minecraft:chorus_flower"
            )
        };
        let mut checked = 0;
        'search: for cx in (-160..=160).step_by(16) {
            for cz in (-160..=160).step_by(16) {
                let (x, z) = (cx * 16 + 8, cz * 16 + 8);
                let biome = worker.noise_biomes(&[[x >> 2, 16, z >> 2]]);
                if biome.first().and_then(|name| name.as_deref()) != Some("minecraft:end_highlands")
                {
                    continue;
                }
                if worker.surface_sample(x, z).is_none() {
                    continue;
                }
                checked += 1;
                if checked > 24 {
                    break 'search;
                }
                let Some(chunk) = decorated_region(&mut worker, (cx, cz), &AtomicBool::new(false))?
                else {
                    continue;
                };
                let min = chunk.terrain.min_y();
                let max = min + chunk.terrain.height();
                let mut raw = 0;
                for y in min..max {
                    for zz in 0..16 {
                        for xx in 0..16 {
                            raw += usize::from(chorus(chunk.block(xx, y, zz)));
                        }
                    }
                }
                if raw == 0 {
                    continue;
                }
                let kept: usize = columns(&chunk, &terrain.biome_map, "minecraft:the_end")
                    .iter()
                    .flat_map(|column| &column.spans)
                    .filter(|span| chorus(span.state))
                    .map(|span| (span.max - span.min) as usize)
                    .sum();
                assert_eq!(
                    raw, kept,
                    "far columns keep every real chorus block in {cx} {cz}"
                );
                return Ok(());
            }
        }
        panic!("no generated end_highlands chunk with chorus among {checked} candidates");
    }
}
