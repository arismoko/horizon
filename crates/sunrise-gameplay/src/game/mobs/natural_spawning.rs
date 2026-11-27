//! The game's side of `NaturalSpawner` (`mobs::spawn_selection`): the level
//! it reads, the spawning chunks and caps of `ServerChunkCache.tickChunks`,
//! and the mobs it adds (finalized with the game's `finalizeSpawn`).
use super::Dim;
use mod_api::{SpawnCheck, SpawnFlags, SpawnLimits};
use std::collections::HashMap;
use sunrise_core::{
    geometry::{Aabb, COLLISION_EPSILON, Vec3, VoxelShape},
    position::BlockPos,
    random::{LegacyRandom, LevelRandom, RandomSource, WorldgenRandom, XoroshiroRandom},
};
use sunrise_gameplay_common::game::{GameGameExt1 as _, commands_common::Target};
use sunrise_items::blocks::BlockFacts;
use sunrise_level::{
    biome::{BiomeInfo, SpawnEntry},
    collisions::{SceneCollisions, no_collision_empty},
};
use sunrise_sim::join_seq::JoinSeq;
use sunrise_sim::mobs::{
    Mob, MobType, MobWorld, Species,
    data::{self, MobData},
    seen_grid::SeenGrid,
    spawn_rules::{Reason, SpawnLevel},
    spawn_selection::{
        self, Category, Counted, Heightmap, NaturalLevel, PlayerSquares, SpawnBuffers,
        SpawnCluster, SpawnCost, SpawnPlayer, SpawnRequest, SpawnState, StructureAt,
        StructureSpawns,
    },
};
use sunrise_sim::vanillagen::{GeneratedMob, VanillaTerrain};
use sunrise_world::dimension_type::DimensionFacts;
use sunrise_world::registry::BlockStateId;

/// The primary candidate survives the insertion attempt as an owning row (possibly
/// restored with a new ID in another level) or as a detached body after a veto or transfer.
pub(in crate::game) enum NaturalAdmission {
    Placed { level: Dim, id: u32, join: JoinSeq },
    Detached(Box<Mob>),
}

/// A level's natural spawning policy, as mods set it through its `LevelRef`
/// (`Game::spawn_policies`): the categories' caps and which may spawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::game) struct SpawnPolicy {
    pub(in crate::game) limits: SpawnLimits,
    pub(in crate::game) flags: SpawnFlags,
}

/// `ServerLevel.canSpawnEntitiesInChunk` of `world`: the chunk is loaded.
pub(in crate::game::server_side) fn can_spawn_entities_in_chunk(
    world: &sunrise_level::world::World,
    cx: i32,
    cz: i32,
) -> bool {
    world.chunks.contains_key(&(cx, cz))
}

/// Natural spawning's candidate chunks (`ChunkMap.collectSpawningChunks`):
/// the union of every player's 17×17 chunk square, in player then dx/dz
/// order, each chunk once. `candidates` is one pass's working list (spawning
/// shuffles and drains it); the rest is the union for the players' last chunk
/// positions, kept between ticks. A chunk crossing rebuilds only the squares
/// it can change, and proximity to players is checked live on every pass.
#[derive(Default)]
pub struct SpawnScratch {
    pub candidates: Vec<(i32, i32)>,
    /// Each player's chunk at the last pass.
    chunks: Vec<(i32, i32)>,
    /// For each player, the others whose squares overlap its own, in order.
    neighbours: Vec<Vec<usize>>,
    /// The chunks each player adds to the union: those of its square, in
    /// dx/dz order, that no earlier player's square holds.
    segments: Vec<Vec<(i32, i32)>>,
    stale: Vec<bool>,
    /// The spawn state's allocations, reused across passes.
    spawn_buffers: SpawnBuffers,
    pass: NaturalPassScratch,
}

#[derive(Default)]
struct NaturalPassScratch {
    players: SearchPlayers,
    costed: Vec<String>,
    ids: HashMap<u32, u32>,
}

impl SpawnScratch {
    fn chunk(p: &LevelPlayer) -> (i32, i32) {
        (
            p.position.x.floor() as i32 >> 4,
            p.position.z.floor() as i32 >> 4,
        )
    }
    /// Whether the square around chunk `centre` holds chunk `c`.
    fn holds(centre: (i32, i32), c: (i32, i32)) -> bool {
        centre.0.abs_diff(c.0) <= 8 && centre.1.abs_diff(c.1) <= 8
    }
    fn overlap(a: (i32, i32), b: (i32, i32)) -> bool {
        a.0.abs_diff(b.0) <= 16 && a.1.abs_diff(b.1) <= 16
    }
    /// Bring the segments up to date with the players' chunks. A segment
    /// depends only on its player's chunk and those of the earlier players
    /// whose squares overlap it, so a move rebuilds the mover's segment and
    /// the later ones its old or new square overlaps.
    fn prepare(&mut self, players: &[LevelPlayer]) {
        let n = players.len();
        let all = n != self.chunks.len();
        self.stale.clear();
        self.stale.resize(n, all);
        if !all {
            let mut moved = false;
            for m in 0..n {
                let (old, new) = (self.chunks[m], Self::chunk(&players[m]));
                if old == new {
                    continue;
                }
                moved = true;
                self.stale[m] = true;
                for (i, p) in players.iter().enumerate().skip(m + 1) {
                    let c = Self::chunk(p);
                    self.stale[i] |= Self::overlap(old, c) || Self::overlap(new, c);
                }
            }
            if !moved {
                return;
            }
        }
        self.chunks.clear();
        self.chunks.extend(players.iter().map(Self::chunk));
        self.neighbours.resize_with(n, Vec::new);
        self.neighbours.truncate(n);
        self.segments.resize_with(n, Vec::new);
        self.segments.truncate(n);
        let chunks = &self.chunks;
        for (i, near) in self.neighbours.iter_mut().enumerate() {
            near.clear();
            near.extend((0..n).filter(|&j| j != i && Self::overlap(chunks[i], chunks[j])));
        }
        for (i, segment) in self.segments.iter_mut().enumerate() {
            if !self.stale[i] {
                continue;
            }
            let near = &self.neighbours[i];
            let earlier = &near[..near.partition_point(|&j| j < i)];
            let (pcx, pcz) = chunks[i];
            segment.clear();
            for dx in -8..=8 {
                for dz in -8..=8 {
                    let c = (pcx + dx, pcz + dz);
                    if !earlier.iter().any(|&j| Self::holds(chunks[j], c)) {
                        segment.push(c);
                    }
                }
            }
        }
    }
    /// Fill `candidates` with the union's chunks, in order, that are within
    /// 128 blocks of a non-spectator holding them in its square and that
    /// `keep` accepts. Returns the union's size.
    fn collect(
        &mut self,
        players: &[LevelPlayer],
        mut forced: impl FnMut((i32, i32)) -> bool,
        mut keep: impl FnMut((i32, i32)) -> bool,
    ) -> i32 {
        self.prepare(players);
        self.candidates.clear();
        let mut count = 0;
        for (i, segment) in self.segments.iter().enumerate() {
            count += segment.len();
            let near = &self.neighbours[i];
            for &c in segment {
                let close = close_enough_for_spawning(&players[i], c)
                    || near.iter().any(|&j| {
                        Self::holds(self.chunks[j], c) && close_enough_for_spawning(&players[j], c)
                    })
                    || forced(c);
                if close && keep(c) {
                    self.candidates.push(c);
                }
            }
        }
        count as i32
    }
}

/// A player in the level in place, as spawning and despawning see it.
pub struct LevelPlayer {
    pub(in crate::game) uuid: u128,
    pub position: Vec3,
    pub(in crate::game) bounds: Aabb,
    pub(in crate::game) alive: bool,
    pub(in crate::game) creative: bool,
    pub spectator: bool,
}
impl LevelPlayer {
    fn distance_sqr(&self, x: f64, y: f64, z: f64) -> f64 {
        (self.position.x - x).powi(2)
            + (self.position.y - y).powi(2)
            + (self.position.z - z).powi(2)
    }
    /// `Level.hasNearbyAlivePlayer`'s test of one player (any distance for a
    /// negative range).
    pub fn alive_near(&self, x: f64, y: f64, z: f64, range: f64) -> bool {
        self.alive && !self.spectator && (range < 0.0 || self.distance_sqr(x, y, z) < range * range)
    }
}
/// `ChunkMap.euclideanDistanceSquared` below 128 blocks
/// (`playerIsCloseEnoughForSpawning`).
fn close_enough_for_spawning(p: &LevelPlayer, (cx, cz): (i32, i32)) -> bool {
    let (x, z) = (
        f64::from(cx * 16 + 8) - p.position.x,
        f64::from(cz * 16 + 8) - p.position.z,
    );
    !p.spectator && x * x + z * z < 16384.0
}
/// The chunks within 8 of a player, in `scratch.candidates`, and for each in
/// `scratch.close` whether a non-spectator is within 128 blocks or `forced`
/// says a ticket holds it for natural spawning. How many there are.
/// `EntityGetter.getNearestPlayer(x, y, z, -1, false)`: the nearest
/// non-spectator, alive or not.
pub(in crate::game::server_side) fn nearest_player(
    players: &[LevelPlayer],
    x: f64,
    y: f64,
    z: f64,
) -> Option<&LevelPlayer> {
    let mut best: Option<(&LevelPlayer, f64)> = None;
    for p in players.iter().filter(|p| !p.spectator) {
        let d = p.distance_sqr(x, y, z);
        if best.is_none_or(|(_, b)| d < b) {
            best = Some((p, d));
        }
    }
    best.map(|(p, _)| p)
}

/// [`nearest_player`] for many positions: the non-spectators by the 64-block
/// square they stand in, searched ring by ring outwards from the position.
/// Every mob asks each tick, and with players spread over the map that was a
/// look at every player for every mob.
pub(in crate::game) struct NearestPlayers<'p> {
    players: &'p [LevelPlayer],
    cells: sunrise_level::fast_hash::FxHashMap<(i64, i64), Vec<usize>>,
    /// The cells' bounding box: (x0, z0, x1, z1).
    bounds: (i64, i64, i64, i64),
    /// Few players, or one too far out for cells: every question scans.
    scan: bool,
}
impl<'p> NearestPlayers<'p> {
    const CELL: f64 = 64.0;
    /// Further out than any world border: such positions are answered by a scan.
    const FAR: f64 = 1.0e9;
    pub(in crate::game::server_side) fn new(players: &'p [LevelPlayer]) -> Self {
        let mut cells = sunrise_level::fast_hash::FxHashMap::<(i64, i64), Vec<usize>>::default();
        let mut bounds = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
        let mut scan = players.len() <= 8;
        for (i, p) in players.iter().enumerate().filter(|(_, p)| !p.spectator) {
            if !(p.position.x.abs() < Self::FAR && p.position.z.abs() < Self::FAR) {
                // Past any world border (or not a number): scans answer.
                scan = true;
                break;
            }
            let key = Self::cell(p.position.x, p.position.z);
            cells.entry(key).or_default().push(i);
            bounds = (
                bounds.0.min(key.0),
                bounds.1.min(key.1),
                bounds.2.max(key.0),
                bounds.3.max(key.1),
            );
        }
        Self {
            players,
            cells,
            bounds,
            scan,
        }
    }
    fn cell(x: f64, z: f64) -> (i64, i64) {
        (
            (x / Self::CELL).floor() as i64,
            (z / Self::CELL).floor() as i64,
        )
    }
    /// The same player [`nearest_player`] finds: the nearest, the first in
    /// the list among equals.
    pub(in crate::game::server_side) fn nearest(
        &self,
        x: f64,
        y: f64,
        z: f64,
    ) -> Option<&'p LevelPlayer> {
        if self.scan || !(x.abs() < Self::FAR && z.abs() < Self::FAR) {
            return nearest_player(self.players, x, y, z);
        }
        if self.cells.is_empty() {
            return None;
        }
        let (qx, qz) = Self::cell(x, z);
        let (x0, z0, x1, z1) = self.bounds;
        let last = (qx - x0)
            .abs()
            .max((qx - x1).abs())
            .max((qz - z0).abs())
            .max((qz - z1).abs());
        let mut best: Option<(usize, f64)> = None;
        let mut looked = 0usize;
        for ring in 0..=last {
            let mut look = |kx: i64, kz: i64| {
                looked += 1;
                for &i in self.cells.get(&(kx, kz)).into_iter().flatten() {
                    let d = self.players[i].distance_sqr(x, y, z);
                    if best.is_none_or(|(b, bd)| d < bd || d == bd && i < b) {
                        best = Some((i, d));
                    }
                }
            };
            if ring == 0 {
                look(qx, qz);
            } else {
                for k in -ring..=ring {
                    look(qx + k, qz - ring);
                    look(qx + k, qz + ring);
                }
                for k in 1 - ring..ring {
                    look(qx - ring, qz + k);
                    look(qx + ring, qz + k);
                }
            }
            // Everyone in a further ring is more than `ring` cells away on one
            // axis, so further than the one found.
            let bound = ring as f64 * Self::CELL;
            if best.is_some_and(|(_, d)| d <= bound * bound) {
                break;
            }
            if looked > self.players.len() * 4 {
                return nearest_player(self.players, x, y, z);
            }
        }
        best.map(|(i, _)| &self.players[i])
    }
}

/// Up to this many players, a spawn start measures to every one.
const PLAYERS_SCANNED: usize = 8;
/// Owned search data that survives the world's immutable borrow being released
/// for each admission. A mod may move players during admission.
#[derive(Default)]
struct SearchPlayers {
    players: Vec<LevelPlayer>,
    nearest: Vec<[f64; 3]>,
    squares: std::cell::OnceCell<PlayerSquares>,
}

impl SearchPlayers {
    fn refresh(&mut self, game: &crate::game::ServerGame<'_>) {
        self.players.clear();
        self.players.extend(game.level_players_iter());
        let positions = self
            .players
            .iter()
            .filter(|p| !p.spectator)
            .map(|p| [p.position.x, p.position.y, p.position.z]);
        if !self.nearest.iter().copied().eq(positions.clone()) {
            self.nearest.clear();
            self.nearest.extend(positions);
            self.squares.take();
        }
    }
}

/// The game level as natural spawning reads it.
pub struct GameLevel<'g, 'a> {
    pub game: &'g crate::game::ServerGame<'a>,
    pub(in crate::game) w: MobWorld<'g>,
    search: SearchPlayers,
    /// Mobs accepted during this pass, not yet inserted in the game's lists.
    pending: std::cell::RefCell<Vec<PendingMob>>,
    /// UUIDs reserved by configured spawner trees in this pass (both families).
    pending_uuids: std::cell::RefCell<sunrise_core::fast_hash::FxHashSet<[i32; 4]>>,
    /// The mobs this pass's finalizeSpawn calls made (jockeys and mounts),
    /// and the chicken-jockey links on mobs already in the game's list, both
    /// applied by [`Game::add_spawned`].
    pub spawned: std::cell::RefCell<Vec<Mob>>,
    pub rides: std::cell::RefCell<Vec<(u32, u32)>>,
    /// How many provisional ids the pass has handed out (`u32::MAX - n`),
    /// mapped to real ones by [`Game::add_spawned`].
    pub(in crate::game) provisional: std::cell::Cell<u32>,
    /// The game's mobs by box, built once a pass has asked more than
    /// [`SCANNED_QUERIES`] box queries (the mobs do not change while it
    /// holds the game); the first ones scan, as the build costs a few scans.
    pub(in crate::game) boxes: std::cell::OnceCell<MobBoxes>,
    pub(in crate::game) box_queries: std::cell::Cell<u32>,
    /// The entity types some biome of the level gives a spawn cost (the
    /// nether's few): for any other the cost is none wherever it stands, and
    /// spawning asks for one for every loaded mob each tick.
    costed: Option<&'g [String]>,
    /// The spawner block whose tick this pass is, told to the spawn checks;
    /// None for natural spawning and the other passes.
    pub(in crate::game) spawner: Option<mod_api::Spawner>,
    /// What the spawner tick decided `isNearPlayer` is, a mod's `spawner_active` included: the
    /// spawner's own `serverTick` reads it rather than asking again. None outside a spawner tick.
    pub(in crate::game) spawner_active: Option<bool>,
    /// The mob a listened position check created and lent, which the spawn
    /// that follows takes rather than creating it again ([`Checked`]).
    checked: std::cell::RefCell<Option<Checked>>,
    /// Single-entry cache of the last looked-up chunk: natural spawning makes
    /// dozens of block, heightmap, light, biome and structure queries in the
    /// same chunk per pass.
    last_chunk: std::cell::Cell<Option<((i32, i32), Option<&'g sunrise_level::chunk::Chunk>)>>,
}
/// The mob a listened `mob_spawn_position_check` created for its lend, held
/// until the spawn takes it, and the provisional count before it was
/// created: a refused check drops the mob and gives the id back, so a refused
/// attempt leaves the pass as an unlistened one would.
struct Checked {
    body: CheckedBody,
    provisional: u32,
}
enum CheckedBody {
    Fresh(Mob),
    Identified(Mob, [i32; 4]),
    Configured(super::persist::SpawnTree),
}
struct NaturalSpawn {
    primary: (Mob, [i32; 4]),
    passengers: Vec<(Mob, [i32; 4])>,
    veto_admission: bool,
}
pub const SCANNED_QUERIES: u32 = 8;
struct PendingMob {
    kind: std::sync::Arc<MobType>,
    bounds: Aabb,
    collidable: bool,
}
/// A mob not removed: its index in `Game::mobs`, box, and whether
/// `EntitySelector.CAN_BE_COLLIDED_WITH` counts it.
type MobBox = (usize, Aabb, bool);
fn mob_box(i: usize, m: &Mob) -> MobBox {
    (i, m.bounds(), mob_collidable_for_spawn(m))
}
/// `canBeCollidedWith(null)` is distinct from pushing and from blocksBuilding.
fn mob_collidable_for_spawn(mob: &Mob) -> bool {
    mob.alive()
        && match mob.species() {
            Species::Shulker => true,
            Species::HappyGhast => sunrise_sim::mobs::happy_ghast::on_still_timeout(mob),
            _ => false,
        }
}
/// The boxes of the game's mobs not removed, and a grid over them.
pub(in crate::game) struct MobBoxes {
    entries: Vec<MobBox>,
    grid: SeenGrid,
}
impl MobBoxes {
    fn new(mobs: &[Mob]) -> Self {
        let entries: Vec<_> = mobs
            .iter()
            .enumerate()
            .filter(|(_, m)| !m.removed)
            .map(|(i, m)| mob_box(i, m))
            .collect();
        let grid = SeenGrid::new(entries.iter().map(|e| e.1));
        Self { entries, grid }
    }
    /// The entries whose boxes meet `area` (and possibly a few more).
    fn near(&self, area: Aabb) -> impl Iterator<Item = MobBox> + '_ {
        let mut near = Vec::new();
        let indexed = self.grid.candidates(area, &mut near);
        let all = (!indexed).then(|| self.entries.iter());
        near.into_iter()
            .map(|i| self.entries[i as usize])
            .chain(all.into_iter().flatten().copied())
    }
}
impl<'g> GameLevel<'g, '_> {
    /// Every player in the level ([`Game::level_players`]).
    pub(in crate::game) fn players(&self) -> &[LevelPlayer] {
        &self.search.players
    }
    #[inline]
    fn chunk_at(&self, cx: i32, cz: i32) -> Option<&'g sunrise_level::chunk::Chunk> {
        let key = (cx, cz);
        if let Some((cached, chunk)) = self.last_chunk.get()
            && cached == key
        {
            return chunk;
        }
        let chunk = self.w.world.chunk_at(cx, cz);
        self.last_chunk.set(Some((key, chunk)));
        chunk
    }
    /// The mobs not removed whose boxes may meet `area` (callers test them).
    fn mobs_near(&self, area: Aabb) -> Box<dyn Iterator<Item = MobBox> + '_> {
        let asked = self.box_queries.get() + 1;
        self.box_queries.set(asked);
        if asked <= SCANNED_QUERIES && self.boxes.get().is_none() {
            return Box::new(
                self.game
                    .mobs
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| !m.removed)
                    .map(|(i, m)| mob_box(i, m)),
            );
        }
        Box::new(
            self.boxes
                .get_or_init(|| MobBoxes::new(&self.game.mobs))
                .near(area),
        )
    }
    /// Vanilla adds the mob before checking another position, even when our
    /// pass defers insertion. Keep its current box and collision predicate.
    pub(in crate::game::server_side) fn record_pending(&self, mob: &Mob) {
        if !mob.removed {
            self.pending.borrow_mut().push(PendingMob {
                kind: mob.kind.clone(),
                bounds: mob.bounds(),
                collidable: mob_collidable_for_spawn(mob),
            });
        }
    }
    pub(in crate::game::server_side) fn admit_pending_tree(
        &self,
        tree: &mut super::persist::SpawnTree,
    ) -> bool {
        if !tree.admit(&mut self.pending_uuids.borrow_mut(), |uuid| {
            use sunrise_sim::join_seq::residency::table::EntityLookup;
            let game = self.game;
            let mut after = None;
            loop {
                // Release the table before resolving the candidate's owner.
                let next = game
                    .resident_entities
                    .table
                    .borrow_mut()
                    .level_uuid_candidate(game.dim.key(), uuid, after);
                let Some(id) = next else { return false };
                if !matches!(game.server_resident_entity(id.get()), EntityLookup::Missing) {
                    return true;
                }
                after = Some(id);
            }
        }) {
            return false;
        }
        for mob in tree.resident_mobs() {
            self.record_pending(mob);
        }
        true
    }
    fn biome(&self, x: i32, y: i32, z: i32) -> Option<&BiomeInfo> {
        Some(self.w.biomes.get(self.noise_biome(x, y, z)?))
    }
    #[inline]
    fn noise_biome(&self, x: i32, y: i32, z: i32) -> Option<u16> {
        self.w
            .world
            .noise_biome_with_chunk(x, y, z, self.chunk_at(x >> 4, z >> 4))
    }
    #[inline]
    fn light(&self, x: i32, y: i32, z: i32) -> u8 {
        self.w
            .world
            .light_with_chunk(x, y, z, self.chunk_at(x >> 4, z >> 4))
    }
    fn zoomed_biome(&self, x: i32, y: i32, z: i32) -> Option<&BiomeInfo> {
        let [x, y, z] = self.game.zoomed_spawn_cell([x, y, z]);
        self.biome(x, y, z)
    }
    /// A provisional id, distinct within the pass and mapped to a real one
    /// by [`Game::add_spawned`].
    pub(in crate::game::server_side) fn provisional_id(&self) -> u32 {
        let n = self.provisional.get();
        self.provisional.set(n + 1);
        u32::MAX - n
    }
    /// The handle of this pass's level, which the spawn checks are told.
    fn level_ref(&self) -> mod_api::LevelRef {
        self.game
            .level_ref(self.game.lend_epoch(), self.game.dim.key())
    }
    /// The mob the position check of this spawn created, when one did and it
    /// is the one asked for (`entity` at `position`, facing `yaw`): the spawn
    /// goes on with it rather than creating it again.
    pub(in crate::game::server_side) fn take_checked(
        &self,
        entity: &str,
        position: [f64; 3],
        yaw: f32,
    ) -> Option<Mob> {
        let CheckedBody::Fresh(mob) = self.checked.take()?.body else {
            return None;
        };
        let at = [mob.position.x, mob.position.y, mob.position.z];
        (mob.kind.id == entity && at == position && mob.yaw == yaw).then_some(mob)
    }

    pub(in crate::game) fn take_checked_tree(&self) -> Option<super::persist::SpawnTree> {
        match self.checked.take()?.body {
            CheckedBody::Configured(tree) => Some(tree),
            _ => None,
        }
    }

    fn identified_candidate(
        &self,
        entity: &str,
        [x, y, z]: [f64; 3],
        yaw: f32,
    ) -> Option<(Mob, [i32; 4])> {
        let kind = self.game.res.mob_types().get(entity)?.clone();
        let (random, uuid) = sunrise_sim::mobs::living_random();
        let mob = Mob::with_random(self.provisional_id(), kind, Vec3::new(x, y, z), yaw, random);
        Some((mob, uuid))
    }
    /// Finalize the actual natural candidate and keep every admitted body's
    /// constructor identity with it until the level accepts that body.
    fn finalize_spawned<R: RandomSource>(
        &self,
        r: &SpawnRequest,
        random: &mut R,
    ) -> Option<NaturalSpawn> {
        let (mut mob, uuid) = match self.checked.take().map(|checked| checked.body) {
            Some(CheckedBody::Identified(mob, uuid)) => (mob, uuid),
            _ => self.identified_candidate(r.entity, r.position, r.yaw)?,
        };
        // Live packs do not yet thread group data through their members.
        let mut group = None;
        let (outcome, difficulty) = self
            .game
            .finalize_spawn_event(&mut mob, r.reason, None, &mut group);
        let mut passengers = Vec::new();
        if !outcome.skip_initialization {
            let mut level = PassLevel::new(self);
            level.constructor_uuids = Some(Vec::new());
            let mut cx = sunrise_sim::mobs::finalize::Ctx {
                level: &mut level,
                random,
                difficulty,
                created: Vec::new(),
                jockey_rider: None,
            };
            sunrise_sim::mobs::finalize::finalize(&mut cx, &mut mob, r.reason, group);
            let created = cx.created;
            let constructed = level.constructor_uuids.unwrap();
            passengers.extend(created.into_iter().map(|body| {
                // Finalization can discard a proposal or admit it as a vehicle.
                let uuid = constructed.iter().find(|(id, _)| *id == body.id).unwrap().1;
                (body, uuid)
            }));
        }
        Some(NaturalSpawn {
            primary: (mob, uuid),
            passengers,
            veto_admission: outcome.veto_admission,
        })
    }
}

/// A spawning pass's [`finalize::FinalizeLevel`]: the game's mobs and
/// conditions read-only, what finalizeSpawn makes (jockeys, mounts) recorded
/// for [`Game::add_spawned`], as the pass holds the game immutably.
pub struct PassLevel<'a, 'g, 'h> {
    l: &'a GameLevel<'g, 'h>,
    /// Only an admission path consuming constructor identities needs this list.
    constructor_uuids: Option<Vec<(u32, [i32; 4])>>,
}
impl<'a, 'g, 'h> PassLevel<'a, 'g, 'h> {
    pub fn new(l: &'a GameLevel<'g, 'h>) -> Self {
        Self {
            l,
            constructor_uuids: None,
        }
    }
}
impl sunrise_sim::mobs::finalize::FinalizeLevel for PassLevel<'_, '_, '_> {
    fn mob_maps(&self) -> &sunrise_sim::neoforge_mob_maps::NeoForgeMobMaps {
        self.l.game.mob_maps()
    }
    fn spawn_event(
        &mut self,
        mob: &mut sunrise_sim::mobs::Mob,
        reason: sunrise_sim::mobs::spawn_rules::Reason,
    ) -> sunrise_sim::mobs::finalize::SpawnEvent {
        self.l.game.finalize_spawn_event_result(mob, reason)
    }
    fn biome(&self, pos: [i32; 3]) -> String {
        // A spawn site without its chunk (a loaded marker mob) is neutral.
        super::finalize_spawn::biome_or_plains(
            self.l.biome(pos[0], pos[1], pos[2]).map(|b| b.name.clone()),
        )
    }
    fn biome_tag(&self, tag: &str, pos: [i32; 3]) -> bool {
        let Some(b) = self.l.biome(pos[0], pos[1], pos[2]) else {
            return false;
        };
        if let Some(tags) = self
            .l
            .game
            .server
            .world_tables
            .0
            .as_ref()
            .and_then(|t| t.pack_tags.get("worldgen/biome"))
        {
            return tags
                .get(tag)
                .is_some_and(|members| members.contains(&b.name));
        }
        self.l.game.res.mob_data().biome_has(tag, &b.name)
    }
    fn structures_with_piece_at(&self, pos: [i32; 3]) -> Vec<String> {
        self.l.game.structures_at(pos)
    }
    fn structure_tag(&self, tag: &str, structure: &str) -> bool {
        if let Some(tags) = self
            .l
            .game
            .server
            .world_tables
            .0
            .as_ref()
            .and_then(|t| t.pack_tags.get("worldgen/structure"))
        {
            return tags
                .get(tag)
                .is_some_and(|members| members.contains(structure));
        }
        self.l
            .game
            .structure_tag(tag)
            .iter()
            .any(|m| m == structure)
    }
    fn moon_phase(&self, _: [i32; 3]) -> usize {
        self.l.game.finalize_moon_phase()
    }
    fn spawning_monsters(&self) -> bool {
        self.l.game.spawning_monsters()
    }
    fn no_collision(&self, bounds: Aabb) -> bool {
        self.l.no_collision(bounds)
    }
    fn free_chicken(&self, bounds: Aabb) -> Option<u32> {
        self.l.game.finalize_free_chicken(bounds)
    }
    fn ride_chicken(&mut self, chicken: u32, rider: u32) {
        self.l.rides.borrow_mut().push((chicken, rider));
    }
    fn game_time(&self) -> i64 {
        self.l.game.finalize_game_time()
    }
    fn halloween(&self) -> bool {
        self.l.game.finalize_halloween()
    }
    fn create(&mut self, kind: &str, _: sunrise_sim::mobs::spawn_rules::Reason) -> Option<Mob> {
        use sunrise_world::food::Difficulty;
        let t = self.l.game.res.mob_types().get(kind)?;
        if self.l.game.world.health_rules.difficulty == Difficulty::Peaceful
            && !t.allowed_in_peaceful()
        {
            return None;
        }
        let t = t.clone();
        let id = self.l.provisional_id();
        Some(if let Some(uuids) = &mut self.constructor_uuids {
            let (random, uuid) = sunrise_sim::mobs::living_random();
            uuids.push((id, uuid));
            Mob::with_random(id, t, Vec3::ZERO, 0.0, random)
        } else {
            Mob::new(id, t, Vec3::ZERO, 0.0)
        })
    }
    fn spawn_data(&self) -> sunrise_sim::mobs::finalize::SpawnData<'_> {
        self.l.game.finalize_spawn_data()
    }
}
impl SpawnLevel for GameLevel<'_, '_> {
    #[inline]
    fn placement_check(
        &self,
        entity: &str,
        reason: Reason,
        at: [i32; 3],
        default_result: bool,
        random: &mut dyn mod_api::RandomSource,
    ) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::MobSpawnPlacementCheck) {
            return default_result;
        }
        let mut result = SpawnCheck::Default;
        self.game.native_mob_spawn_placement_check(
            mod_api::__private::MobSpawnPlacementCheckCall {
                dimension: self.game.dim.id(),
                kind: entity,
                reason,
                pos: BlockPos::new(at[0], at[1], at[2]),
                default_result,
                level: self.level_ref(),
                random,
                result: &mut result,
            },
        );
        match result {
            SpawnCheck::Succeed => true,
            SpawnCheck::Fail => false,
            _ => default_result,
        }
    }
    #[inline]
    fn within_world_border(&self, x: i32, z: i32) -> bool {
        self.game
            .border
            .geometry()
            .is_within_bounds(f64::from(x), f64::from(z), 0.0)
    }
    #[inline]
    fn state(&self, x: i32, y: i32, z: i32) -> BlockStateId {
        self.w
            .world
            .block_with_chunk(x, y, z, self.chunk_at(x >> 4, z >> 4))
    }
    #[inline]
    fn facts(&self) -> &BlockFacts {
        self.w.facts
    }
    #[inline]
    fn mob_data(&self) -> &MobData {
        self.w.data
    }
    #[inline]
    fn block_tag(&self, tag: &str, block: &str) -> bool {
        self.w.block_tag(tag, block)
    }
    // Level.getBiome: the zoomed biome.
    #[inline]
    fn biome_tag(&self, tag: &str, x: i32, y: i32, z: i32) -> bool {
        let [x, y, z] = self.game.zoomed_spawn_cell([x, y, z]);
        self.w.spawn_biome_tag(tag, self.noise_biome(x, y, z))
    }
    #[inline]
    fn sky_light(&self, x: i32, y: i32, z: i32) -> i32 {
        self.w.spawn_sky_light(|| self.light(x, y, z))
    }
    #[inline]
    fn block_light(&self, x: i32, y: i32, z: i32) -> i32 {
        sunrise_sim::mobs::spawn_rules::spawn_block_light(self.light(x, y, z))
    }
    #[inline]
    fn sky_darken(&self) -> i32 {
        SpawnLevel::sky_darken(&self.w)
    }
    #[inline]
    fn thundering(&self) -> bool {
        self.w.thundering
    }
    #[inline]
    fn sea_level(&self) -> i32 {
        self.w.sea_level()
    }
    #[inline]
    fn world_surface(&self, x: i32, z: i32) -> i32 {
        self.w
            .spawn_world_surface(x, z, self.chunk_at(x >> 4, z >> 4))
    }
    #[inline]
    fn peaceful(&self) -> bool {
        self.w.peaceful()
    }
    #[inline]
    fn dimension_type(&self) -> &DimensionFacts {
        self.game.dimension_type()
    }
    #[inline]
    fn surface_slime_chance(&self, x: i32, y: i32, z: i32) -> f32 {
        self.w.surface_slime_chance(x, y, z)
    }
    #[inline]
    fn world_seed(&self) -> i64 {
        self.w.world_seed
    }
    /// `getNearestPlayer(x, y, z, distance, true)`, over every player.
    #[inline]
    fn player_near(&self, x: f64, y: f64, z: f64, distance: f64) -> bool {
        self.search.players.iter().any(|p| {
            p.alive && !p.creative && !p.spectator && p.distance_sqr(x, y, z) < distance * distance
        })
    }
}
impl NaturalLevel for GameLevel<'_, '_> {
    #[inline]
    fn spawn_hooks(&self) -> bool {
        !mod_api::__private::mods().is_empty()
    }
    /// Vanilla creates the mob before its position checks, so with a listener
    /// the mob a fresh spawn is about to create is created here, the way the
    /// spawn creates it, lent to the listeners, and handed on to the spawn
    /// ([`GameLevel::take_checked`]); a refused check drops it
    /// ([`NaturalLevel::position_refused`]). Creating it draws nothing from
    /// the level's random. A site that does not create the mob fresh (a
    /// spawner's entity data, chunk generation's packs) lends its kind and
    /// position only. With no listener nothing is created.
    #[inline]
    fn position_check(
        &self,
        entity: &str,
        reason: Reason,
        at: [f64; 3],
        pending: spawn_selection::Pending<'_>,
    ) -> SpawnCheck {
        let mut result = SpawnCheck::Default;
        let listened = mod_api::__private::listened(mod_api::HookId::MobSpawnPositionCheck);
        let identified = matches!(reason, Reason::Natural | Reason::ChunkGeneration);
        if !listened && !identified {
            return result;
        }
        // An earlier check's mob that no spawn took is not this one.
        self.checked.take();
        let provisional = self.provisional.get();
        let body = if let Some(tag) = pending.configured {
            self.game
                .decode_spawner_tree(tag, at, pending.yaw)
                .map(CheckedBody::Configured)
        } else if identified {
            self.identified_candidate(entity, at, pending.yaw)
                .map(|(mob, uuid)| CheckedBody::Identified(mob, uuid))
        } else {
            self.game.res.mob_types().get(entity).map(|kind| {
                let [x, y, z] = at;
                CheckedBody::Fresh(Mob::new(
                    self.provisional_id(),
                    kind.clone(),
                    Vec3::new(x, y, z),
                    pending.yaw,
                ))
            })
        };
        let Some(mut body) = body else {
            return SpawnCheck::Fail;
        };
        if !listened {
            *self.checked.borrow_mut() = Some(Checked { body, provisional });
            return result;
        }
        let mob = match &mut body {
            CheckedBody::Fresh(mob) | CheckedBody::Identified(mob, _) => mob,
            CheckedBody::Configured(tree) => match tree.root_mob() {
                Some(mob) => mob,
                None => return SpawnCheck::Fail,
            },
        };
        let level = self.level_ref();
        let at = [mob.position.x, mob.position.y, mob.position.z];
        let borrowed = self.game.borrowed_unspawned_mob(entity, at, Some(mob));
        self.game
            .native_mob_spawn_position_check(mod_api::__private::MobSpawnPositionCheckCall {
                dimension: self.game.dim.id(),
                kind: entity,
                reason,
                pos: at,
                level,
                spawner: self.spawner,
                mob: mod_api::UnspawnedMobView::new(&borrowed),
                result: &mut result,
            });
        *self.checked.borrow_mut() = Some(Checked { body, provisional });
        result
    }
    #[inline]
    fn position_refused(&self) {
        if let Some(refused) = self.checked.take() {
            self.provisional.set(refused.provisional);
        }
    }
    #[inline]
    fn potential_spawns(&self, category: Category, pos: [i32; 3], entries: &mut Vec<SpawnEntry>) {
        let mut canceled = false;
        self.game.native_potential_spawns(
            self.game.dim.id(),
            category.name(),
            pos,
            entries,
            &mut canceled,
        );
        if canceled {
            entries.clear();
        }
    }
    // `Level.getMinY` and `getMaxY`: the level's own range (0..255 in vanilla's
    // Nether and End), not the widened column its chunks hold.
    #[inline]
    fn min_y(&self) -> i32 {
        self.w.world.shape.min_y
    }
    #[inline]
    fn max_y(&self) -> i32 {
        self.w.world.shape.max_y() - 1
    }
    #[inline]
    fn ambient_light(&self) -> f32 {
        self.game.dimension_type().ambient_light
    }
    #[inline]
    fn has_ceiling(&self) -> bool {
        self.w.world.shape.has_ceiling
    }
    #[inline]
    fn mob_type(&self, entity: &str) -> Option<&MobType> {
        self.game.res.mob_types().get(entity).map(|t| t.as_ref())
    }
    // The `natural_mob_spawns` attribute reads the zoomed biome
    // (fullResolutionBiomes), the spawn probability the noise cell.
    #[inline]
    fn biome_spawns(&self, x: i32, y: i32, z: i32) -> Option<&HashMap<String, Vec<SpawnEntry>>> {
        self.zoomed_biome(x, y, z).map(|b| &b.spawns)
    }
    #[inline]
    fn spawn_cost(&self, entity: &str, x: i32, y: i32, z: i32) -> Option<SpawnCost> {
        self.game.spawn_cost_at(self.costed, entity, [x, y, z])
    }
    #[inline]
    fn creature_probability(&self, x: i32, y: i32, z: i32) -> f32 {
        self.biome(x, y, z).map_or(0.1, |b| b.creature_probability)
    }
    #[inline]
    fn structures_at(
        &self,
        x: i32,
        y: i32,
        z: i32,
        wanted: &dyn Fn(&str) -> bool,
    ) -> Vec<StructureAt> {
        if self
            .chunk_at(x >> 4, z >> 4)
            .is_none_or(|c| c.structure_refs.is_empty())
        {
            return Vec::new();
        }
        self.game
            .structures_reaching_where([x, y, z], wanted)
            .into_iter()
            .map(|(id, in_piece)| StructureAt {
                id,
                in_piece,
                in_bounds: true,
            })
            .collect()
    }
    #[inline]
    fn structure_spawns(&self) -> &StructureSpawns {
        &self.game.server.structure_spawns
    }
    #[inline]
    fn nearest_player_positions(&self) -> &[[f64; 3]] {
        &self.search.nearest
    }
    #[inline]
    fn player_positions_near(&self, x: f64, z: f64, extent: f64, out: &mut Vec<[f64; 3]>) {
        let search = &self.search;
        if search.nearest.len() <= PLAYERS_SCANNED {
            out.extend_from_slice(&search.nearest);
            return;
        }
        let squares = search
            .squares
            .get_or_init(|| PlayerSquares::new(&search.nearest));
        squares.near(x, z, extent, out);
    }
    #[inline]
    fn respawn(&self) -> Option<[i32; 3]> {
        // The world spawn point lives in the overworld.
        if self.game.dim != Dim::Overworld {
            return None;
        }
        self.game.level.spawn.map(|s| {
            [
                s[0].floor() as i32,
                s[1].floor() as i32,
                s[2].floor() as i32,
            ]
        })
    }
    /// `ServerLevel.canSpawnEntitiesInChunk`: the whole chunk is inside the border.
    #[inline]
    fn can_spawn_in_chunk(&self, cx: i32, cz: i32) -> bool {
        self.chunk_at(cx, cz).is_some()
            && self.within_world_border(cx << 4, cz << 4)
            && self.within_world_border((cx << 4).wrapping_add(15), (cz << 4).wrapping_add(15))
    }
    #[inline]
    fn no_collision(&self, bounds: Aabb) -> bool {
        self.game.no_spawn_collision(bounds, |query| {
            self.mobs_near(query)
                .any(|(_, b, collidable)| collidable && b.intersects(query))
                || self
                    .pending
                    .borrow()
                    .iter()
                    .any(|mob| mob.collidable && mob.bounds.intersects(query))
        })
    }
    /// `EntityGetter.isUnobstructed`: the fresh mob cannot already have a
    /// shared root vehicle or be a candidate in the level's entity query.
    #[inline]
    fn unobstructed(&self, bounds: Aabb) -> bool {
        let Ok(shape) = VoxelShape::from_box(bounds) else {
            return false;
        };
        let Some(query) = shape.bounds() else {
            return true;
        };
        let obstructs = |bounds| spawn_selection::blocks_spawn_shape(&shape, query, bounds);
        let entities = sunrise_gameplay_common::game::entity::entity_lists_of!(self.game);
        !self
            .search
            .players
            .iter()
            .any(|p| !p.spectator && obstructs(p.bounds))
            && !self
                .pending
                .borrow()
                .iter()
                .any(|mob| obstructs(mob.bounds))
            && !self.mobs_near(query).any(|(_, b, _)| obstructs(b))
            // The parent is a LivingEntity; its separate parts do not block building.
            && !self.game.dragon.as_ref().is_some_and(|dragon| obstructs(dragon.body_bounds()))
            && !entities
                .non_mob_building_blocker_boxes(
                    entities
                        .carts
                        .iter()
                        .map(sunrise_gameplay_common::game::minecarts::Minecart::bounds),
                    &self.game.crystals,
                )
                .any(obstructs)
    }
    #[inline]
    fn spawn_mobs(&self) -> bool {
        self.game
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::SPAWN_MOBS)
    }
    /// WORLD_SURFACE (every spawn attempt's) is the chunk's kept heightmap.
    /// The others are scanned down the column: sections of air alone are
    /// skipped (air counts for no heightmap). An unloaded column reads as air.
    #[inline]
    fn heightmap(&self, kind: Heightmap, x: i32, z: i32) -> i32 {
        let world = self.w.world;
        // A heightmap reads at least the level's minimum Y (an empty column of a level
        // narrower than its chunks' column would read the column's bottom).
        let min_y = world.shape.min_y;
        if kind == Heightmap::WorldSurface {
            return self.world_surface(x, z).max(min_y);
        }
        world
            .chunks
            .get(&(x >> 4, z >> 4))
            .and_then(|chunk| {
                chunk.highest(world.air.0, |y| {
                    spawn_selection::heightmap_blocks(self, kind, x, y, z)
                })
            })
            .map_or(min_y, |y| (y + 1).max(min_y))
    }
    #[inline]
    fn pathfindable_land(&self, x: i32, y: i32, z: i32) -> bool {
        self.w
            .data
            .pathfindable(self.state(x, y, z), data::PathComputationType::Land)
    }
}
impl sunrise_sim::mobs::spawner::SpawnerLevel for GameLevel<'_, '_> {
    /// `Level.hasNearbyAlivePlayer`, over every player.
    #[inline]
    fn alive_player_near(&self, x: f64, y: f64, z: f64, range: f64) -> bool {
        self.search
            .players
            .iter()
            .any(|p| p.alive_near(x, y, z, range))
    }
    #[inline]
    fn spawner_active(&self, near: bool) -> bool {
        self.spawner_active.unwrap_or(near)
    }
    #[inline]
    fn spawner_blocks_work(&self) -> bool {
        self.game
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::SPAWNER_BLOCKS_WORK)
    }
    #[inline]
    fn spawner_type(&self, id: &str) -> Option<sunrise_sim::mobs::spawner::SpawnerTypeFacts> {
        self.game.res.rest().spawner_types.get(id).copied()
    }
    #[inline]
    fn count_entities(&self, id: &str, bounds: Aabb) -> usize {
        let mobs = self
            .mobs_near(bounds)
            .filter(|&(i, b, ..)| self.game.mobs[i].kind.id == id && b.intersects(bounds))
            .count();
        let pending = self
            .pending
            .borrow()
            .iter()
            .filter(|mob| mob.kind.id == id && mob.bounds.intersects(bounds))
            .count();
        mobs + pending
    }
}
fn remap_spawn_links(mob: &mut Mob, links: &HashMap<u32, u32>) {
    if let Some(mount) = mob.extra.mount.as_mut()
        && let Some(&id) = links.get(mount)
    {
        *mount = id;
    }
    if let Some(rider) = mob.extra.rider.as_mut()
        && let Some(&id) = links.get(rider)
    {
        *rider = id;
    }
}
impl<'a> crate::game::ServerGame<'a> {
    /// `CollisionGetter.noCollision(AABB)`: with no source there is no border
    /// shape. `EntityGetter.getEntityCollisions` queries the inflated area.
    pub(in crate::game::server_side) fn no_spawn_collision(
        &self,
        bounds: Aabb,
        mob_collision: impl FnOnce(Aabb) -> bool,
    ) -> bool {
        let mut scene = SceneCollisions {
            world: &self.common.world,
            binding: self.common.res.collision(),
            subject: None,
        };
        if !no_collision_empty(&mut scene, bounds) {
            return false;
        }
        if bounds.average_size() < COLLISION_EPSILON {
            return true;
        }
        let query = bounds.inflate(Vec3::new(
            COLLISION_EPSILON,
            COLLISION_EPSILON,
            COLLISION_EPSILON,
        ));
        !self
            .common
            .boats
            .iter()
            .any(|boat| boat.bounds().intersects(query))
            && !mob_collision(query)
    }

    pub(in crate::game::server_side) fn live_spawn_mob_collision(&self, query: Aabb) -> bool {
        self.common.mobs.iter().any(|mob| {
            !mob.removed && mob_collidable_for_spawn(mob) && mob.bounds().intersects(query)
        })
    }

    /// The natural spawning policy of the level `dimension`: vanilla's until a
    /// mod sets one.
    pub(in crate::game::server_side) fn spawn_policy(
        &self,
        dimension: mod_api::DimensionKey,
    ) -> SpawnPolicy {
        self.server
            .spawn_policies
            .get(&dimension)
            .copied()
            .unwrap_or_default()
    }

    fn spawn_biome(&self, [x, y, z]: [i32; 3]) -> Option<&BiomeInfo> {
        Some(world_biomes!(self).get(self.common.world.noise_biome(x, y, z)?))
    }

    /// The block whose noise cell `BiomeManager.getBiome` reads.
    fn zoomed_spawn_cell(&self, [x, y, z]: [i32; 3]) -> [i32; 3] {
        let [qx, qy, qz] = self
            .server
            .streaming
            .biome_zoom
            .quart_at_block(BlockPos::new(x, y, z));
        [qx << 2, qy << 2, qz << 2]
    }

    fn zoomed_spawn_biome(&self, block: [i32; 3]) -> Option<&BiomeInfo> {
        self.spawn_biome(self.zoomed_spawn_cell(block))
    }

    fn spawn_cost_at(
        &self,
        costed: Option<&[String]>,
        entity: &str,
        block: [i32; 3],
    ) -> Option<SpawnCost> {
        if costed.is_some_and(|types| !types.iter().any(|kind| kind == entity)) {
            return None;
        }
        self.zoomed_spawn_biome(block)?
            .spawn_costs
            .get(entity)
            .copied()
    }
    pub fn natural_level(&self) -> GameLevel<'_, 'a> {
        let mut search = SearchPlayers::default();
        search.refresh(self);
        self.natural_search_level(search, None)
    }
    fn natural_search_level<'g>(
        &'g self,
        search: SearchPlayers,
        costed: Option<&'g [String]>,
    ) -> GameLevel<'g, 'a> {
        GameLevel {
            game: self,
            w: self.mob_world_base(&[]),
            search,
            pending: Default::default(),
            pending_uuids: Default::default(),
            spawned: Default::default(),
            rides: Default::default(),
            provisional: Default::default(),
            boxes: Default::default(),
            box_queries: Default::default(),
            costed,
            spawner: None,
            spawner_active: None,
            checked: Default::default(),
            last_chunk: Default::default(),
        }
    }
    /// Gives a spawn pass's provisional mobs (the `u32::MAX - n` ids
    /// [`GameLevel`] hands out) their real ids in order, rewrites the mount
    /// and rider links that reference them, and seats chicken jockeys on the
    /// chickens already in the level. The old→new map comes back for the
    /// pass's events.
    pub(in crate::game::server_side) fn resolve_spawn_ids(
        &mut self,
        mut groups: [&mut [Mob]; 2],
        rides: &[(u32, u32)],
    ) -> HashMap<u32, u32> {
        let mut links: HashMap<u32, u32> = HashMap::new();
        self.assign_spawn_ids(
            groups.iter_mut().flat_map(|group| group.iter_mut()),
            &mut links,
        );
        for mob in groups.iter_mut().flat_map(|group| group.iter_mut()) {
            remap_spawn_links(mob, &links);
        }
        for (chicken, rider) in rides {
            let rider = links.get(rider).copied().unwrap_or(*rider);
            if let Some(c) = self.common.mobs.iter_mut().find(|m| m.id == *chicken) {
                c.extra.jockey = true;
                c.extra.rider = Some(rider);
            }
        }
        links
    }

    fn assign_spawn_ids<'m>(
        &mut self,
        bodies: impl Iterator<Item = &'m mut Mob>,
        links: &mut HashMap<u32, u32>,
    ) {
        for body in bodies {
            self.server.next_entity += 1;
            links.insert(body.id, self.server.next_entity);
            body.id = self.server.next_entity;
        }
    }
    /// Adds the mobs a spawn pass created: ids and the links that need them
    /// (finalizeSpawn's mounts ride by id, and its chickens carry their
    /// zombie). Every mob arrives with a provisional id; real ones are
    /// assigned in spawn order and the links rewritten.
    #[cfg(any(test, feature = "test-support"))]
    pub fn add_spawned(
        &mut self,
        pending: Vec<Mob>,
        created: Vec<Mob>,
        rides: Vec<(u32, u32)>,
        persistent: bool,
    ) -> Vec<u32> {
        let _save_lease = sunrise_sim::join_seq::visibility::PassLease::enter();
        let mut pending = pending;
        let mut created = created;
        for m in pending.iter_mut() {
            m.persistent = persistent;
        }
        self.resolve_spawn_ids([&mut pending, &mut created], &rides);
        let ids = pending.iter().map(|m| m.id).collect();
        for mob in pending {
            self.common.mobs.push(mob);
            self.admit_new_mobs(self.common.mobs.len() - 1);
        }
        for mob in created {
            self.common.mobs.push(mob);
            self.admit_new_mobs(self.common.mobs.len() - 1);
        }
        ids
    }
    /// Every player in the level in place: the local player, and the LAN
    /// guests or server players in it.
    pub fn level_players(&self) -> Vec<LevelPlayer> {
        self.level_players_iter().collect()
    }
    /// `level_players` one at a time, without collecting them.
    pub(in crate::game::server_side) fn level_players_iter(
        &self,
    ) -> impl Iterator<Item = LevelPlayer> + '_ {
        self.local_level_player()
            .into_iter()
            .chain(self.level_guests().map(|g| LevelPlayer {
                uuid: g.uuid,
                position: Vec3::new(g.position[0], g.position[1], g.position[2]),
                bounds: super::lan_mobs::guest_bounds(g),
                alive: !g.dead,
                creative: g.creative(),
                spectator: g.spectator(),
            }))
    }
    fn local_level_player(&self) -> Option<LevelPlayer> {
        if !self.local_player_in_level() {
            return None;
        }
        self.player_view().map(|p| LevelPlayer {
            uuid: sunrise_sim::lan::playerdata::ints_uuid(p.uuid),
            position: p.position,
            bounds: p.bounds,
            alive: p.alive,
            creative: p.creative,
            spectator: p.spectator,
        })
    }
    /// The UUIDs of the players in level `dimension`, for a query that only
    /// names them: no `PlayerView`, so nothing is allocated per player. For the
    /// level in place they are `level_players`', in its order; a parked level
    /// has only its guests.
    pub(in crate::game::server_side) fn level_player_uuids<'s>(
        &'s self,
        dimension: &'s str,
    ) -> impl Iterator<Item = u128> + 's {
        let local = (dimension == self.common.dim.id() && self.local_player_in_level())
            .then(|| sunrise_sim::lan::playerdata::ints_uuid(self.acting_uuid()));
        local
            .into_iter()
            .chain(self.guests_placed_in(dimension).map(|g| g.uuid))
    }
    fn local_player_in_level(&self) -> bool {
        self.common.player.is_some()
            && !self.server.lan_acting_fake
            && self
                .server
                .lan_acting
                .is_none_or(|conn| self.command_guest(conn).is_some_and(|g| g.placed))
    }
    /// The guests in the level in place, past the acting one: `player_view`
    /// already reads that guest's live body, and its record lags behind it.
    fn level_guests(&self) -> impl Iterator<Item = &sunrise_sim::lan::guest::Guest> + '_ {
        self.server
            .lan
            .iter()
            .flat_map(|h| self.guests_placed_in(&h.hub.config.dimension))
    }
    /// The guests placed in level `dimension`, past the acting one.
    fn guests_placed_in<'s>(
        &'s self,
        dimension: &'s str,
    ) -> impl Iterator<Item = &'s sunrise_sim::lan::guest::Guest> + 's {
        self.server.lan.iter().flat_map(move |h| {
            h.hub
                .guests
                .iter()
                .filter(move |g| g.placed_in(dimension) && self.server.lan_acting != Some(g.conn))
        })
    }
    /// `Level.hasNearbyAlivePlayer` before a level is built for the spawn
    /// code: what a spawner asks first, every tick.
    pub fn alive_player_near(&self, x: f64, y: f64, z: f64, range: f64) -> bool {
        if self
            .local_level_player()
            .is_some_and(|p| p.alive_near(x, y, z, range))
        {
            return true;
        }
        self.level_guests().any(|g| {
            let [px, py, pz] = g.position;
            !g.dead
                && !g.spectator()
                && (range < 0.0
                    || (px - x).powi(2) + (py - y).powi(2) + (pz - z).powi(2) < range * range)
        })
    }
    /// `ChunkMap.collectSpawningChunks`: the natural spawn chunk tracker's
    /// chunks (within 8 chunks of a player, `getSpawnCandidateChunks`) that
    /// are ticking and within 128 blocks of a non-spectator, or held by a mod
    /// ticket that forces natural spawning (NeoForge's
    /// `shouldForceNaturalSpawning`). Also returns how many chunks the tracker
    /// holds (`getNaturalSpawnChunkCount`).
    pub fn spawning_chunks(&self, players: &[LevelPlayer], scratch: &mut SpawnScratch) -> i32 {
        let tickets = self.server.streaming.mod_tickets();
        let levels = self.server.tickets.0.levels();
        scratch.collect(
            players,
            |chunk| tickets.forces_natural_spawning(chunk),
            |(cx, cz)| {
                levels.loaded_ticking(cx, cz) && super::ticking_chunk(&self.common.world, cx, cz)
            },
        )
    }
    /// `NaturalSpawner.createState` for a pass over the level in place, under
    /// its spawn limits (`SpawnPolicy`).
    pub fn natural_spawn_state(
        &self,
        level: &GameLevel<'_, '_>,
        spawnable: i32,
        players: Vec<SpawnPlayer>,
        buffers: SpawnBuffers,
        counted: &[Counted],
    ) -> SpawnState {
        SpawnState::new_in(level, spawnable, players, buffers, counted, |cx, cz| {
            self.common.world.chunks.contains_key(&(cx, cz))
        })
        .with_limits(self.spawn_policy(self.common.dim.key()).limits)
    }
    /// The categories a pass over the level in place tries: none with the
    /// `spawn_mobs` rule off, else `getFilteredSpawningCategories` with the
    /// `spawn_monsters` rule, the persistent ones every 400 ticks, and the
    /// level's spawn flags (`SpawnPolicy`) taking away.
    pub fn natural_categories(&self, state: &SpawnState) -> Vec<Category> {
        let rules = &self.common.ui.rules;
        if !rules.bool_at(sunrise_game_data::gamerules::vanilla::SPAWN_MOBS) {
            return Vec::new();
        }
        state.policy_categories(
            self.spawn_policy(self.common.dim.key()).flags,
            rules.bool_at(sunrise_game_data::gamerules::vanilla::SPAWN_MONSTERS),
            self.common.world.game_time % 400 == 0,
        )
    }
    /// The tick's spawning phases in `ServerLevel.tick`'s order: the raids
    /// tick first (`this.raids.tick`), then the chunk source's
    /// (`getChunkSource().tick`): natural spawning, then the custom spawners
    /// of `ServerChunkCache.tickChunks`. A wave that lands this tick therefore
    /// joins before that tick's spawns, with the lower entity ids. Nothing spawns once the
    /// entity ids are exhausted (`entity_ids_exhausted`).
    pub fn tick_spawners(&mut self) {
        if self.entity_ids_exhausted() {
            return;
        }
        let from = self.lan_synced_from();
        sunrise_level::spans::time("spawn_raids", || self.tick_raids());
        // `ServerChunkCache.tickChunks` skips the entire chunk-spawning phase
        // (natural spawning and custom spawners) in debug worlds.
        if !self.server.world_debug {
            sunrise_level::spans::time("spawn_natural", || self.spawn_natural());
            // ServerChunkCache.tickChunks: the custom spawners tick only with spawn_mobs.
            if self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::SPAWN_MOBS)
            {
                sunrise_level::spans::time("spawn_custom", || self.tick_custom_spawners());
            }
        }
        self.lan_synced_since(from);
    }
    /// `ServerChunkCache.tickChunks`: the spawn state over the loaded mobs,
    /// the categories under their global caps (creatures every 400 ticks),
    /// then `tickSpawningChunk` in the shuffled spawning chunks of every
    /// player: inhabited time, then `spawnForChunk`.
    pub fn spawn_natural(&mut self) {
        // `tickChunks` does nothing in a Debug world.
        if self.server.world_debug {
            return;
        }
        let mut laps = sunrise_level::spans::laps();
        let mut players = self.level_players();
        // Fake players (mods' bots) do not anchor natural spawning.
        if let Some(h) = self.server.lan.as_ref() {
            let fake: Vec<u128> = h
                .hub
                .guests
                .iter()
                .filter(|g| g.is_fake())
                .map(|g| g.uuid)
                .collect();
            if !fake.is_empty() {
                players.retain(|p| !fake.contains(&p.uuid));
            }
        }
        if players.is_empty() {
            return;
        }
        let mut scratch = std::mem::take(&mut self.server.spawn_scratch);
        let spawnable = self.spawning_chunks(&players, &mut scratch);
        laps.lap("sn_candidates");
        for &key in &scratch.candidates {
            *self.common.world.inhabited.entry(key).or_insert(0) += 1;
        }
        laps.lap("sn_inhabited");
        let mut random = std::mem::replace(
            &mut self.common.world.random,
            sunrise_core::random::LevelRandom::for_level(LegacyRandom::new(0)),
        );
        scratch.pass.players.refresh(self);
        scratch.pass.costed.clear();
        for kind in world_biomes!(self)
            .list
            .iter()
            .flat_map(|b| b.spawn_costs.keys())
        {
            if !scratch.pass.costed.contains(kind) {
                scratch.pass.costed.push(kind.clone());
            }
        }
        let (mut state, categories) = {
            let level = self.natural_search_level(
                std::mem::take(&mut scratch.pass.players),
                Some(&scratch.pass.costed),
            );
            laps.lap("sn_level");
            let counted: Vec<Counted> = self
                .common
                .mobs
                .iter()
                .map(|m| Counted {
                    entity: &m.kind.id,
                    category: Category::from_name(&m.kind.category),
                    block: [
                        m.position.x.floor() as i32,
                        m.position.y.floor() as i32,
                        m.position.z.floor() as i32,
                    ],
                    is_mob: true,
                    persistent: m.persistent
                        || sunrise_sim::mobs::ai::requires_custom_persistence(m),
                })
                .collect();
            let spawn_players: Vec<SpawnPlayer> = players
                .iter()
                .map(|p| SpawnPlayer {
                    position: [p.position.x, p.position.y, p.position.z],
                    spectator: p.spectator,
                })
                .collect();
            laps.lap("sn_materialize");
            let buffers = std::mem::take(&mut scratch.spawn_buffers);
            let state =
                self.natural_spawn_state(&level, spawnable, spawn_players, buffers, &counted);
            scratch.pass.players = level.search;
            laps.lap("sn_state");
            let categories = self.natural_categories(&state);
            laps.lap("sn_categories");
            (state, categories)
        };
        // ServerChunkCache.lastSpawnState, for the debug overlay.
        let last_state = state.counts();
        // Util.shuffle.
        for i in (1..scratch.candidates.len()).rev() {
            let j = random.next_int_bound(i as i32 + 1).unwrap_or(0) as usize;
            scratch.candidates.swap(i, j);
        }
        laps.lap("sn_shuffle");
        spawn_selection::attempt_counters(false);
        let pass = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if !categories.is_empty() {
                self.run_natural_spawn_pass(
                    &categories,
                    scratch.candidates.drain(..),
                    &mut random,
                    &mut state,
                    &mut scratch.pass,
                );
            }
        }));
        sunrise_threads::work::clear_work();
        spawn_selection::attempt_counters(true);
        laps.lap("sn_attempts");
        self.common.world.random = random;
        self.server.spawn_counts.insert(self.common.dim, last_state);
        scratch.spawn_buffers = state.into_parts().1;
        self.server.spawn_scratch = scratch;
        if let Err(payload) = pass {
            std::panic::resume_unwind(payload);
        }
        laps.lap("sn_finalize");
    }

    /// Resume the same chunk/category search after each admission, borrowing a single view for
    /// all attempts that fail before insertion. A join can change the world's rows and blocks.
    fn run_natural_spawn_pass(
        &mut self,
        categories: &[Category],
        mut chunks: impl Iterator<Item = (i32, i32)>,
        random: &mut LevelRandom,
        state: &mut SpawnState,
        scratch: &mut NaturalPassScratch,
    ) {
        let mut chunk = (0, 0);
        let mut category_index = categories.len();
        let mut cluster: Option<SpawnCluster> = None;
        let mut provisional = 0;
        let may_edit_players = mod_api::__private::listened(mod_api::HookId::EntityJoinLevel)
            || mod_api::__private::listened(mod_api::HookId::EntityConstructing)
            || mod_api::__private::listened(mod_api::HookId::FinalizeSpawn)
            || mod_api::__private::listened(mod_api::HookId::MobSpawnPlacementCheck)
            || mod_api::__private::listened(mod_api::HookId::MobSpawnPositionCheck);
        let mut trace = |_: spawn_selection::Event<'_>| {};
        let _save_lease = sunrise_sim::join_seq::visibility::PassLease::enter();
        loop {
            let next = {
                let level = self.natural_search_level(
                    std::mem::take(&mut scratch.players),
                    Some(&scratch.costed),
                );
                level.provisional.set(provisional);
                let next = loop {
                    if let Some(search) = cluster.as_mut() {
                        if let Some(candidate) =
                            search.next_candidate(&level, random, state, &mut trace)
                        {
                            if let Some(spawn) =
                                level.finalize_spawned(&search.request(candidate), random)
                            {
                                provisional = level.provisional.get();
                                break Some((candidate, spawn));
                            }
                            continue;
                        }
                        cluster.take().unwrap().finish(state);
                    }
                    if let Some(&category) = categories.get(category_index) {
                        category_index += 1;
                        if state.can_spawn_local(category, chunk.0, chunk.1) {
                            let start = spawn_selection::spawn_start(&level, random, chunk);
                            if start[1] > level.min_y() {
                                cluster = SpawnCluster::new(&level, state, category, chunk, start);
                            }
                        }
                    } else {
                        let Some(next) = chunks.find(|&(x, z)| level.can_spawn_in_chunk(x, z))
                        else {
                            break None;
                        };
                        chunk = next;
                        category_index = 0;
                        sunrise_threads::work::set_chunk(chunk.0, chunk.1);
                    }
                };
                scratch.players = level.search;
                let rides = level.rides.into_inner();
                next.map(|(candidate, spawn)| (candidate, spawn, rides))
            };
            let Some((candidate, spawn, rides)) = next else {
                break;
            };
            self.complete_natural_spawn(
                cluster.as_mut().unwrap(),
                candidate,
                spawn,
                &rides,
                &mut scratch.ids,
                random,
                state,
                &scratch.costed,
            );
            if may_edit_players {
                scratch.players.refresh(self);
            }
        }
    }

    /// Natural spawns still need the primary body after a veto or a transfer out of this level.
    /// Take ownership of just that row before clearing its live state.
    pub(in crate::game) fn detach_natural_candidate(&mut self, id: u32, index: usize) -> bool {
        if !matches!(
            self.server.natural_admission.as_ref(),
            Some(NaturalAdmission::Placed { id: candidate, .. }) if *candidate == id
        ) {
            return false;
        }
        let mut mob = self.common.mobs.remove(index);
        mob.extra.spawn_tags = self.tags_of(Target::Mob(id));
        mob.join = JoinSeq::UNJOINED;
        self.server.natural_admission = Some(NaturalAdmission::Detached(Box::new(mob)));
        true
    }

    /// NaturalSpawner.spawnCategoryForPosition: perform admission, then the spawn state's
    /// callback, then ask the actual candidate for the cluster limit before resuming the search.
    #[allow(clippy::too_many_arguments)]
    fn complete_natural_spawn(
        &mut self,
        cluster: &mut SpawnCluster,
        candidate: spawn_selection::SpawnCandidate,
        spawn: NaturalSpawn,
        rides: &[(u32, u32)],
        ids: &mut HashMap<u32, u32>,
        random: &mut LevelRandom,
        state: &mut SpawnState,
        costed: &[String],
    ) {
        let NaturalSpawn {
            primary: (mut mob, uuid),
            mut passengers,
            veto_admission,
        } = spawn;
        mob.persistent = false;
        let provisional = mob.id;
        ids.clear();
        if passengers.is_empty() {
            self.server.next_entity += 1;
            mob.id = self.server.next_entity;
        } else {
            self.assign_spawn_ids(
                std::iter::once(&mut mob).chain(passengers.iter_mut().map(|(body, _)| body)),
                ids,
            );
            for body in std::iter::once(&mut mob).chain(passengers.iter_mut().map(|(body, _)| body))
            {
                remap_spawn_links(body, ids);
            }
        }
        let id = mob.id;
        // A real Host callback reads and writes the level stream between selection draws.
        std::mem::swap(&mut self.common.world.random, random);
        let saved = self.server.natural_admission.take();
        let admitted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // ServerLevelAccessor.addFreshEntityWithPassengers inserts one body at a time.
            // Until its turn, a passenger is only owned by this iterator.
            for (body, uuid) in std::iter::once((mob, uuid)).chain(passengers) {
                let body_id = body.id;
                if body_id == id && veto_admission {
                    self.server.natural_admission =
                        Some(NaturalAdmission::Detached(Box::new(body)));
                    continue;
                }
                let index = self.common.mobs.len();
                self.common.persist.uuids.insert(body_id, uuid);
                self.common.mobs.push(body);
                let join = self.common.mobs[index].join;
                if body_id == id {
                    self.server.natural_admission = Some(NaturalAdmission::Placed {
                        level: self.common.dim,
                        id,
                        join,
                    });
                }
                for &(chicken, rider) in rides {
                    let resolved = if rider == provisional {
                        Some(id)
                    } else {
                        ids.get(&rider).copied()
                    };
                    if resolved == Some(body_id)
                        && let Some(chicken) = self.common.mobs.iter_mut().find(|m| m.id == chicken)
                    {
                        chicken.extra.jockey = true;
                        chicken.extra.rider = Some(body_id);
                    }
                }
                self.join_spawn_tags(index);
                self.native_joins_flush(false, 0);
            }
            let outcome = self
                .server
                .natural_admission
                .take()
                .expect("primary was offered");
            let view = match &outcome {
                NaturalAdmission::Placed { level, id, join } if *level == self.common.dim => {
                    let mob = if join.is_joined() {
                        self.common.mobs.locate(*join).map(|i| &self.common.mobs[i])
                    } else {
                        self.common.persist.off_level.get(id).and_then(
                            sunrise_gameplay_common::game::entity_memory::OffLevel::as_mob,
                        )
                    };
                    mob.map(|mob| (mob, Some(*id)))
                }
                NaturalAdmission::Placed { level, id, join } => self
                    .server
                    .levels
                    .0
                    .parked
                    .get(level)
                    .and_then(|owner| owner.natural_spawn_mob(*id, *join))
                    .map(|(mob, _)| (mob, Some(*id))),
                NaturalAdmission::Detached(mob) => Some((mob.as_ref(), Some(mob.id))),
            };
            if let Some((mob, mob_id)) = view {
                let at = [mob.position.x, mob.position.y, mob.position.z];
                let kind = mob.kind.id.clone();
                let vanilla = spawn_selection::max_cluster_size(mob.species());
                cluster.after_spawn(
                    |entity, block| self.spawn_cost_at(Some(costed), entity, block),
                    state,
                    candidate,
                    at.map(|x| x.floor() as i32),
                    &mut |_| {},
                );
                let mob_ref = mod_api::MobRef::new(mod_api::EntityRef::new(
                    self.lend_epoch(),
                    mob_id.unwrap_or(id),
                ));
                let limit = self.spawn_cluster_size(mob_ref, &kind, vanilla);
                cluster.check_limit(candidate, limit, true);
            } else {
                log::error!("natural spawn admission lost candidate {id}");
            }
        }));
        self.server.natural_admission = saved;
        std::mem::swap(&mut self.common.world.random, random);
        if let Err(payload) = admitted {
            std::panic::resume_unwind(payload);
        }
    }
    /// `NoiseBasedChunkGenerator.spawnOriginalMobs`: creature packs for a
    /// freshly generated chunk, from the biome at its corner column's top,
    /// with the chunk's decoration-seeded random, finalized with the
    /// region's random as generation adds them. The settings' own
    /// `disable_mob_generation` skips the pass, which is how the End's preset
    /// keeps its chunks empty (the dimension is also checked, as before, so a
    /// preset that fails to load cannot start filling the End).
    pub(in crate::game) fn spawn_generation_creatures(&mut self, cx: i32, cz: i32) {
        if !self.common.world.chunks.contains_key(&(cx, cz)) || self.common.dim == Dim::End {
            return;
        }
        let Some(terrain) = self.common.end.locator() else {
            return;
        };
        // `FlatLevelSource.spawnOriginalMobs` is empty: a superflat world places no packs, and
        // neither does a mod's chunk generator, whose own `spawnOriginalMobs` the API has none of.
        if !terrain.noise_ground() {
            return;
        }
        // `NoiseGeneratorSettings.disableMobGeneration`: the shipped End preset
        // asks for no packs at generation, and so would any dimension using it.
        if terrain.disable_mob_generation() {
            return;
        }
        let mut region = terrain.region_random(cx, cz);
        let any = region.as_any();
        let added = if let Some(r) = any.downcast_mut::<LegacyRandom>() {
            self.generation_creatures(cx, cz, &terrain, r)
        } else if let Some(r) = any.downcast_mut::<XoroshiroRandom>() {
            self.generation_creatures(cx, cz, &terrain, r)
        } else {
            return;
        };
        for pack in added {
            self.install_generated_mobs(pack);
        }
    }
    fn generation_creatures<Q: RandomSource + Clone + 'static>(
        &self,
        cx: i32,
        cz: i32,
        terrain: &VanillaTerrain,
        region: &mut Q,
    ) -> Vec<Vec<GeneratedMob>> {
        let mut random = WorldgenRandom::new(LegacyRandom::new(0));
        random.set_decoration_seed(self.common.level.seed, cx * 16, cz * 16);
        let mut added = Vec::new();
        let mut group = None;
        let level = self.natural_level();
        let source = [cx * 16, level.max_y(), cz * 16];
        let mut spawn = |l: &GameLevel, r: &SpawnRequest, region: &mut Q| {
            // Each pack starts without group data.
            if r.group_index == 0 {
                group = None;
            }
            let [x, y, z] = r.position;
            let (bx, by, bz) = (x.floor() as i32, y.floor() as i32, z.floor() as i32);
            let biome = l.zoomed_biome(bx, by, bz).map_or("", |b| b.name.as_str());
            let Some((mob, uuid)) = (match l.checked.take().map(|c| c.body) {
                Some(CheckedBody::Identified(mob, uuid)) => Some((mob, uuid)),
                _ => l.identified_candidate(r.entity, r.position, r.yaw),
            }) else {
                return false;
            };
            match terrain.spawn_generated(
                mob,
                uuid,
                biome,
                region,
                &mut group,
                |mob, difficulty, group| {
                    l.game.finalize_spawn_with_difficulty(
                        mob,
                        Reason::ChunkGeneration,
                        None,
                        difficulty,
                        group,
                    )
                },
            ) {
                Some(pack) => {
                    added.push(pack);
                    true
                }
                None => false,
            }
        };
        spawn_selection::spawn_mobs_for_chunk_generation(
            &level,
            source,
            (cx, cz),
            &mut random,
            region,
            &mut spawn,
            &mut |_| {},
        );
        added
    }
}

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

#[cfg(test)]
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(test)]
mod tests {
    #[cfg(test)]
    /// Answers both spawn checks with one fixed result.
    struct ChecksMod(SpawnCheck);
    #[cfg(test)]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for ChecksMod {
        fn mob_spawn_placement_check(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::MobSpawnPlacementCheckEvent<'_>,
        ) {
            *event.result = self.0;
        }
        fn mob_spawn_position_check(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::MobSpawnPositionCheckEvent<'_>,
        ) {
            *event.result = self.0;
        }
    }
    type CartReads = (Option<String>, Option<[f64; 3]>);
    fn cart_reads(
        host: &dyn mod_api::Host,
        spawner: Option<mod_api::Spawner>,
    ) -> Option<CartReads> {
        let mod_api::Spawner::Minecart { entity } = spawner? else {
            return None;
        };
        Some((
            entity
                .kind(host)
                .and_then(|key| key.id(host))
                .map(|id| id.as_str().to_owned()),
            entity.position(host),
        ))
    }
    #[cfg(test)]
    type MobReads = (
        Result<mod_api::Identifier, mod_api::UnspawnedMobError>,
        Result<[f64; 3], mod_api::UnspawnedMobError>,
        Result<bool, mod_api::UnspawnedMobError>,
    );
    #[cfg(test)]
    type Log = std::sync::Arc<std::sync::Mutex<Vec<Heard>>>;
    #[cfg(test)]
    /// Gives every natural monster position one entry: the sulfur cube, whose spawn predicate
    /// has no conditions, in packs of four.
    struct SulfurCubesOnly;
    #[cfg(test)]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for SulfurCubesOnly {
        fn potential_spawns(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::PotentialSpawnsEvent<'_>,
        ) -> mod_api::Flow {
            if event.category == "monster" {
                *event.entries = vec![SpawnEntry {
                    entity: SULFUR_CUBE.into(),
                    weight: 1,
                    min: 4,
                    max: 4,
                    constant: true,
                }];
            }
            mod_api::Flow::Continue
        }
    }
    pub const SULFUR_CUBE: &str = "minecraft:sulfur_cube";
    #[cfg(test)]
    /// A spawned mob as the draw comparisons see it: id, kind, position, yaw, persistence and
    /// the next draw of its own random (seeded from its id).
    type Snap = (u32, String, [f64; 3], f32, bool, i32);
    #[cfg(test)]
    fn snap(m: &Mob) -> Snap {
        (
            m.id,
            m.kind.id.clone(),
            [m.position.x, m.position.y, m.position.z],
            m.yaw,
            m.persistent,
            m.random.clone().next_int(),
        )
    }
    /// Runs `go` with no mod (`None`), or with a position-check listener answering `answer`
    /// beside `extra` mods, and what that listener heard.
    fn with_checks<R>(
        answer: Option<SpawnCheck>,
        extra: impl Fn() -> Vec<mod_api::ModEntry>,
        go: impl FnOnce() -> R,
    ) -> (R, Vec<Heard>) {
        let log = Log::default();
        let mut mods = extra();
        if let Some(answer) = answer {
            mods.push(mod_api::ModEntry::new(
                "spawn-context",
                Box::new(ContextMod {
                    answer,
                    log: log.clone(),
                }),
            ));
        }
        let out = if mods.is_empty() {
            go()
        } else {
            mod_platform::testing::with_mods(mods.into(), go)
        };
        let heard = log.lock().unwrap().clone();
        (out, heard)
    }
    #[cfg(test)]
    /// Answers `finalize_spawn` the way a test asks.
    #[derive(Clone, Copy)]
    enum FinalizeAnswer {
        Pass,
        SkipFinalize,
        StopSpawn,
        StopAgeableSpawn,
        SkipAndStop,
    }
    #[cfg(test)]
    struct FinalizeMod(FinalizeAnswer);
    #[cfg(test)]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for FinalizeMod {
        fn finalize_spawn(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::FinalizeSpawnEvent<'_>,
        ) -> mod_api::Flow {
            match self.0 {
                FinalizeAnswer::Pass => {}
                FinalizeAnswer::SkipFinalize => return mod_api::Flow::Cancel,
                FinalizeAnswer::StopSpawn => event.spawn_cancelled = true,
                FinalizeAnswer::StopAgeableSpawn => {
                    let mut group = mod_api::AgeableGroup::new(false, 0.0);
                    group.size = i32::MAX;
                    *event.group = Some(mod_api::SpawnGroup::Ageable(group));
                    event.spawn_cancelled = true;
                }
                FinalizeAnswer::SkipAndStop => {
                    event.spawn_cancelled = true;
                    return mod_api::Flow::Cancel;
                }
            }
            mod_api::Flow::Continue
        }
    }
    #[cfg(test)]
    /// Counts the `finalize_spawn` events it hears.
    struct CountFinalize(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    #[cfg(test)]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for CountFinalize {
        fn finalize_spawn(
            &self,
            _: &dyn mod_api::Host,
            _: &mut mod_api::FinalizeSpawnEvent<'_>,
        ) -> mod_api::Flow {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            mod_api::Flow::Continue
        }
    }
    #[cfg(test)]
    /// Records the kind and reason of every `finalize_spawn` event.
    struct RecordFinalize(std::sync::Arc<std::sync::Mutex<Vec<(String, mod_api::SpawnReason)>>>);
    #[cfg(test)]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for RecordFinalize {
        fn finalize_spawn(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::FinalizeSpawnEvent<'_>,
        ) -> mod_api::Flow {
            self.0
                .lock()
                .unwrap()
                .push((event.kind.to_owned(), event.reason));
            mod_api::Flow::Continue
        }
    }
    #[cfg(test)]
    /// Answers every `mob_despawn` event with the value in its cell and counts them.
    struct Despawns(
        std::sync::Arc<std::sync::Mutex<mod_api::Despawn>>,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
    );
    #[cfg(test)]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Despawns {
        fn mob_despawn(&self, _: &dyn mod_api::Host, event: &mut mod_api::MobDespawnEvent<'_>) {
            self.1.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            *event.result = *self.0.lock().unwrap();
        }
    }
    #[cfg(test)]
    /// Answers the group size limit of a type, reading the mob it is lent.
    struct ZombieClusters;
    #[cfg(test)]
    #[mod_api::sunrise_part]
    impl mod_api::EntityPart for ZombieClusters {
        fn spawn_cluster_size(
            &self,
            host: &dyn mod_api::Host,
            event: &mut mod_api::SpawnClusterSizeEvent,
        ) {
            assert!(
                event.mob.entity().kind(host).is_some(),
                "the mob is resident"
            );
            event.size = event.vanilla * 2;
        }
    }
    #[cfg(test)]
    /// A second part on the same type: asked only when the first does not answer.
    struct LaterClusters;
    #[cfg(test)]
    #[mod_api::sunrise_part]
    impl mod_api::EntityPart for LaterClusters {
        fn spawn_cluster_size(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::SpawnClusterSizeEvent,
        ) {
            event.size += 1;
        }
    }
    #[cfg(test)]
    /// A part that leaves the size alone.
    struct SilentClusters;
    #[cfg(test)]
    #[mod_api::sunrise_part]
    impl mod_api::EntityPart for SilentClusters {}
    const JOINED_POSITION: [f64; 3] = [42.5, 65.0, 42.5];
    const SPAWN_CLUSTER_CASE: &str = "SUNRISE_SPAWN_CLUSTER_CHILD";
    static JOINED_CANDIDATE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    static CLUSTER_QUERIES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    struct JoinedClusters;
    #[mod_api::sunrise_part]
    impl mod_api::EntityPart for JoinedClusters {
        fn spawn_cluster_size(
            &self,
            host: &dyn mod_api::Host,
            event: &mut mod_api::SpawnClusterSizeEvent,
        ) {
            let id = JOINED_CANDIDATE.load(std::sync::atomic::Ordering::Relaxed);
            if std::env::var(SPAWN_CLUSTER_CASE).as_deref() == Ok("valid") {
                assert_eq!(event.mob.entity().id(), id);
                assert_eq!(event.mob.entity().position(host), Some(JOINED_POSITION));
                let tags = event.mob.entity().tags(host).unwrap();
                assert!(tags.contains("after-admission"));
                assert_eq!(tags.iter_utf16().count(), 1);
            }
            CLUSTER_QUERIES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            event.size = 1;
        }
    }
    #[cfg(test)]
    struct ClusterMod;
    #[cfg(test)]
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for ClusterMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.entity_part("minecraft:zombie", ZombieClusters);
            r.entity_part("minecraft:zombie", LaterClusters);
            r.entity_part("minecraft:skeleton", SilentClusters);
            r.entity_part(SULFUR_CUBE, JoinedClusters);
        }
        fn entity_join_level(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityJoinLevelEvent<'_>,
        ) -> mod_api::Flow {
            if event.kind != SULFUR_CUBE || event.dimension != "minecraft:overworld" {
                return mod_api::Flow::Continue;
            }
            let id = event.subject.id();
            let entity = host.entity(id).unwrap();
            assert!(entity.add_tag(host, "after-admission"));
            JOINED_CANDIDATE.store(id, std::sync::atomic::Ordering::Relaxed);
            match std::env::var(SPAWN_CLUSTER_CASE).unwrap().as_str() {
                "unwind" => {} // The engine checkpoint fires after this callback returns.
                "move" => {
                    assert!(entity.teleport_to_level(
                        host,
                        "minecraft:the_nether",
                        JOINED_POSITION,
                        None
                    ));
                }
                case => {
                    assert!(entity.teleport_to(host, JOINED_POSITION, None));
                    if case == "reject" {
                        return mod_api::Flow::Cancel;
                    }
                }
            }
            mod_api::Flow::Continue
        }
    }

    use super::*;
    use sunrise_core::nbt::{Compound, Tag};

    fn player(x: f64, y: f64, z: f64, spectator: bool) -> LevelPlayer {
        LevelPlayer {
            uuid: 0,
            position: Vec3::new(x, y, z),
            bounds: Aabb::new(Vec3::new(x, y, z), Vec3::new(x, y, z)),
            alive: true,
            creative: false,
            spectator,
        }
    }

    /// A chunk in a player's spawning area but past 128 blocks is a spawning
    /// chunk only while a ticket forces it; a chunk outside the area never is.
    #[test]
    fn a_forcing_ticket_makes_a_far_chunk_in_the_area_a_spawning_chunk() {
        let players = [player(8.0, 64.0, 8.0, false)];
        let mut scratch = SpawnScratch::default();
        let corner = (8, 8);
        let count = scratch.collect(&players, |_| false, |_| true);
        assert_eq!(count, 289);
        assert!(
            !scratch.candidates.contains(&corner),
            "no player within 128 blocks"
        );
        assert!(scratch.candidates.contains(&(1, 1)));
        scratch.collect(&players, |c| c == corner || c == (30, 30), |_| true);
        assert!(scratch.candidates.contains(&corner));
        assert!(
            !scratch.candidates.contains(&(-8, -8)),
            "only the held chunk"
        );
        assert!(!scratch.candidates.contains(&(30, 30)), "outside the area");
    }

    /// The candidates as the uncached port collected them every tick: each
    /// player's square in dx/dz order into one ordered set, close to any
    /// player that holds the chunk. Returns the set's size too.
    fn uncached_candidates(players: &[LevelPlayer]) -> (i32, Vec<(i32, i32)>) {
        let mut order = Vec::new();
        let mut close = Vec::new();
        let mut seen = HashMap::new();
        for p in players {
            let (pcx, pcz) = SpawnScratch::chunk(p);
            for dx in -8..=8 {
                for dz in -8..=8 {
                    let key = (pcx + dx, pcz + dz);
                    let i = *seen.entry(key).or_insert_with(|| {
                        order.push(key);
                        close.push(false);
                        order.len() - 1
                    });
                    close[i] |= close_enough_for_spawning(p, key);
                }
            }
        }
        let kept = order.iter().zip(&close).filter(|c| *c.1).map(|c| *c.0);
        (order.len() as i32, kept.collect())
    }

    #[test]
    fn spawn_candidates_keep_order_and_live_proximity_across_cache_hits() {
        let mut players = vec![
            player(0.0, 64.0, 0.0, false),
            player(16.0, 64.0, 0.0, false),
        ];
        let mut scratch = SpawnScratch::default();
        // Two adjacent 17x17 squares: the first square, then the new east edge.
        assert_eq!(scratch.collect(&players, |_| false, |_| true), 306);
        assert!(scratch.candidates.contains(&(-8, 0)));
        // Spawning consumes the working list; the next pass must not care.
        scratch.candidates.reverse();
        scratch.candidates.truncate(3);
        // Moving inside its chunk takes player 0 out of range of the west edge.
        players[0].position.x = 15.5;
        assert_eq!(scratch.collect(&players, |_| false, |_| true), 306);
        assert!(!scratch.candidates.contains(&(-8, 0)));
        assert_eq!(scratch.candidates, uncached_candidates(&players).1);
        // Nothing is close to spectators, though the squares still count.
        for p in &mut players {
            p.spectator = true;
        }
        assert_eq!(scratch.collect(&players, |_| false, |_| true), 306);
        assert!(scratch.candidates.is_empty());
        for p in &mut players {
            p.spectator = false;
        }
        // Player 1 near its chunk's east side reaches the middle of chunk 9.
        players[1].position.x = 31.9;
        assert_eq!(scratch.collect(&players, |_| false, |_| true), 306);
        assert!(scratch.candidates.contains(&(9, 0)));
        assert_eq!(scratch.candidates, uncached_candidates(&players).1);
        // `keep` filters without reordering.
        scratch.collect(&players, |_| false, |(x, z)| x == z);
        let diagonal: Vec<_> = uncached_candidates(&players)
            .1
            .into_iter()
            .filter(|&(x, z)| x == z)
            .collect();
        assert!(diagonal.len() > 1);
        assert_eq!(scratch.candidates, diagonal);
    }

    /// Players walking, crossing borders, meeting, splitting, joining, leaving
    /// and swapping order: every pass matches the uncached collection.
    #[test]
    #[ignore = "daily"]
    fn spawn_candidates_match_the_uncached_union_while_players_move() {
        let mut seed = 0x5DEECE66Du64;
        let mut next = move |bound: u64| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) % bound
        };
        let mut players: Vec<LevelPlayer> = [0.0, 40.0, 300.0, 1200.0, -500.0]
            .iter()
            .map(|&x| player(x, 64.0, x / 2.0, false))
            .collect();
        let mut scratch = SpawnScratch::default();
        for step in 0..3000 {
            match next(40) {
                0 if players.len() < 8 => {
                    let x = next(1200) as f64 - 600.0;
                    let at = next(players.len() as u64 + 1) as usize;
                    players.insert(at, player(x, 64.0, -x, false));
                }
                1 if players.len() > 1 => {
                    players.remove(next(players.len() as u64) as usize);
                }
                2 if players.len() > 1 => {
                    let i = next(players.len() as u64 - 1) as usize;
                    players.swap(i, i + 1);
                }
                3 => {
                    let i = next(players.len() as u64) as usize;
                    players[i].spectator = !players[i].spectator;
                }
                4 => {
                    // Teleport onto another player: identical squares.
                    let (i, j) = (next(players.len() as u64), next(players.len() as u64));
                    players[i as usize].position = players[j as usize].position;
                }
                _ => {
                    for p in &mut players {
                        p.position.x += next(13) as f64 - 6.0 + 0.37;
                        p.position.z += next(13) as f64 - 6.0 - 0.21;
                    }
                }
            }
            let (count, expected) = uncached_candidates(&players);
            assert_eq!(
                scratch.collect(&players, |_| false, |_| true),
                count,
                "step {step}"
            );
            assert_eq!(scratch.candidates, expected, "step {step}");
        }
    }

    /// The grid finds the same player a look at every one finds, the first of
    /// two at the same distance included, and never a spectator.
    #[test]
    fn nearest_players_find_what_a_scan_finds() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = |m: f64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 1_000_000) as f64 / 1_000_000.0 * 2.0 * m - m
        };
        let mut players: Vec<LevelPlayer> = (0..150)
            .map(|i| player(next(3_000.0), 64.0 + next(60.0), next(3_000.0), i % 11 == 0))
            .collect();
        // Two at the same distance from the origin, the second one first in no cell order.
        players.push(player(300.0, 64.0, 0.0, false));
        players.push(player(-300.0, 64.0, 0.0, false));
        players.push(player(0.0, 64.0, 300.0, false));
        let grid = NearestPlayers::new(&players);
        let index = |p: Option<&LevelPlayer>| p.map(|p| std::ptr::from_ref(p) as usize);
        let mut probes: Vec<(f64, f64, f64)> = (0..400)
            .map(|_| (next(3_000.0), next(100.0), next(3_000.0)))
            .collect();
        probes.extend([(0.0, 64.0, 0.0), (1.0e12, 0.0, 0.0), (f64::NAN, 0.0, 0.0)]);
        for (x, y, z) in probes {
            assert_eq!(
                index(grid.nearest(x, y, z)),
                index(nearest_player(&players, x, y, z)),
                "({x}, {y}, {z})"
            );
        }
        players.push(player(NearestPlayers::FAR * 2.0, 64.0, 0.0, false));
        let grid = NearestPlayers::new(&players);
        assert!(grid.scan);
        for x in [0.0, NearestPlayers::FAR * 2.0] {
            assert_eq!(
                index(grid.nearest(x, 64.0, 0.0)),
                index(nearest_player(&players, x, 64.0, 0.0))
            );
        }
        let spectators: Vec<LevelPlayer> = (0..20)
            .map(|i| player(f64::from(i) * 100.0, 0.0, 0.0, true))
            .collect();
        assert!(
            NearestPlayers::new(&spectators)
                .nearest(0.0, 0.0, 0.0)
                .is_none()
        );
    }

    /// The placement check runs after vanilla's predicate (`default_result` is its answer), so
    /// its draws are vanilla's whatever a mod answers. The position check replaces vanilla's two
    /// checks: no listener and `Default` draw exactly as vanilla, `Succeed` and `Fail` draw nothing.
    #[test]
    #[ignore = "daily"]
    fn spawn_checks_pin_the_random_draws_of_both_halves() {
        use crate::game::native_entity_refs::tests::with_game;
        with_game(|game| {
            let level = game.natural_level();
            let placement = |random: &mut LegacyRandom| {
                let ok = sunrise_sim::mobs::spawn_rules::check_spawn_rules(
                    level.mob_type("minecraft:sulfur_cube").unwrap().into(),
                    &level,
                    Reason::Natural,
                    random,
                    [8, 64, 8],
                );
                (ok, random.next_int())
            };
            let position = |random: &mut LegacyRandom| {
                let ok = spawn_selection::position_ok(
                    &level,
                    "minecraft:sulfur_cube",
                    Reason::Natural,
                    [8.5, 64.0, 8.5],
                    spawn_selection::Pending {
                        yaw: 0.0,
                        configured: None,
                    },
                    || {
                        random.next_int();
                        false
                    },
                );
                (ok, random.next_int())
            };
            let run = |answer: Option<SpawnCheck>| {
                let go = || {
                    (
                        placement(&mut LegacyRandom::new(11)),
                        position(&mut LegacyRandom::new(11)),
                    )
                };
                match answer {
                    None => go(),
                    Some(a) => mod_platform::testing::with_mods(
                        vec![mod_api::ModEntry::new(
                            "spawn-checks",
                            Box::new(ChecksMod(a)),
                        )]
                        .into(),
                        go,
                    ),
                }
            };
            let mut seed = LegacyRandom::new(11);
            let first = seed.next_int();
            let second = seed.next_int();
            let none = run(None);
            assert!(none.0.0, "the sulfur cube's predicate has no conditions");
            assert_eq!(none.1, (false, second), "vanilla's closure drew once");
            assert_eq!(run(Some(SpawnCheck::Default)), none);
            let succeed = run(Some(SpawnCheck::Succeed));
            assert_eq!(
                succeed.0,
                (true, none.0.1),
                "placement draws stay vanilla's"
            );
            assert_eq!(
                succeed.1,
                (true, first),
                "Succeed skips the checks and their draws"
            );
            let fail = run(Some(SpawnCheck::Fail));
            assert_eq!(fail.0, (false, none.0.1));
            assert_eq!(fail.1, (false, first));
        });
    }

    /// What a spawn-check listener heard: the hook, the kind, the level and the spawner, and on
    /// the position check what the lent mob read (kind, position, whether its head slot is
    /// empty).
    #[derive(Clone, Debug, PartialEq)]
    struct Heard {
        hook: &'static str,
        kind: String,
        level: mod_api::LevelRef,
        spawner: Option<mod_api::Spawner>,
        cart: Option<CartReads>,
        mob: Option<MobReads>,
    }

    /// Records both spawn checks and answers the position check with `answer`, reading the lent
    /// mob and changing nothing else.
    struct ContextMod {
        answer: SpawnCheck,
        log: Log,
    }
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for ContextMod {
        fn mob_spawn_placement_check(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::MobSpawnPlacementCheckEvent<'_>,
        ) {
            self.log.lock().unwrap().push(Heard {
                hook: "placement",
                kind: event.kind.to_owned(),
                level: event.level,
                spawner: None,
                cart: None,
                mob: None,
            });
        }
        fn mob_spawn_position_check(
            &self,
            host: &dyn mod_api::Host,
            event: &mut mod_api::MobSpawnPositionCheckEvent<'_>,
        ) {
            let mob = event.mob;
            let reads = (
                mob.kind(),
                mob.position(),
                mob.equipment(mod_api::EquipmentSlot::Head)
                    .map(|e| e.is_none()),
            );
            self.log.lock().unwrap().push(Heard {
                hook: "position",
                kind: event.kind.to_owned(),
                level: event.level,
                spawner: event.spawner,
                cart: cart_reads(host, event.spawner),
                mob: Some(reads),
            });
            *event.result = self.answer;
        }
        fn finalize_spawn(
            &self,
            host: &dyn mod_api::Host,
            event: &mut mod_api::FinalizeSpawnEvent<'_>,
        ) -> mod_api::Flow {
            let level = host.level(event.dimension).unwrap();
            let reads = (
                event.mob.kind(),
                event.mob.position(),
                event
                    .mob
                    .equipment(mod_api::EquipmentSlot::Head)
                    .map(|e| e.is_none()),
            );
            self.log.lock().unwrap().push(Heard {
                hook: "finalize",
                kind: event.kind.to_owned(),
                level,
                spawner: event.spawner,
                cart: cart_reads(host, event.spawner),
                mob: Some(reads),
            });
            mod_api::Flow::Continue
        }
    }

    /// A survival player standing at `at`, and empty lit chunks around the origin.
    fn spawn_check_world(game: &mut crate::game::ServerGame<'_>, at: Vec3) {
        use std::sync::Arc;
        use sunrise_level::chunk::Chunk;
        game.player = Some(sunrise_sim::motion::PlayerMotion::new(game.motion, at).unwrap());
        for cx in -2..=4 {
            for cz in -2..=4 {
                let mut chunk = Chunk::empty(cx, cz);
                chunk.lit = true;
                game.world.chunks.insert((cx, cz), Arc::new(chunk));
            }
        }
    }
    fn level_of(game: &crate::game::ServerGame<'_>) -> mod_api::LevelRef {
        game.level(game.dim.id()).unwrap()
    }

    /// Natural spawning (`spawnCategoryForPosition`): a listener hears both checks with the
    /// level and no spawner, and reads the pending mob; answering `Default` it leaves every draw,
    /// id and spawned mob as with no listener, so the mob it read is the one that spawned, not
    /// created twice. A `Fail` drops each mob: none is added and no provisional id is used.
    #[test]
    #[ignore = "daily"]
    fn natural_spawn_checks_lend_the_mob_and_keep_the_draws() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
        use sunrise_level::random_source::set_thread_uniquifier;
        if run_fresh_cases(
            &format!(
                "{}::natural_spawn_checks_lend_the_mob_and_keep_the_draws",
                module_path!().split_once("::").unwrap().1
            ),
            "SUNRISE_TEST_SEEDS",
            &["1"],
        ) {
            return;
        }
        with_game(|game| {
            spawn_check_world(game, Vec3::new(0.5, 64.0, 0.5));
            let stone = game.pack.assets.registry.default_state("minecraft:stone");
            for x in 32..48 {
                for z in 32..48 {
                    game.world.set(x, 63, z, stone.unwrap()).unwrap();
                }
            }
            let level = level_of(game);
            let attempt = |game: &mut crate::game::ServerGame<'_>| {
                set_thread_uniquifier(12345).unwrap();
                let mut random = LegacyRandom::new(42);
                let natural = game.natural_level();
                let players = natural
                    .search
                    .players
                    .iter()
                    .map(|p| SpawnPlayer {
                        position: [p.position.x, p.position.y, p.position.z],
                        spectator: p.spectator,
                    })
                    .collect();
                let mut state = SpawnState::new(&natural, 289, players, &[], |_, _| true);
                let mut pending = Vec::new();
                let mut spawn = |l: &GameLevel, r: &SpawnRequest, rng: &mut LegacyRandom| {
                    let (mob, _) = l.finalize_spawned(r, rng)?.primary;
                    l.record_pending(&mob);
                    pending.push(mob);
                    Some(spawn_selection::Spawned { school: true })
                };
                spawn_selection::spawn_category_for_position(
                    &natural,
                    &mut random,
                    &mut state,
                    Category::Monster,
                    (2, 2),
                    [40, 64, 40],
                    &mut spawn,
                    &mut |_, _, vanilla| vanilla,
                    &mut |_| {},
                );
                assert!(natural.checked.borrow().is_none(), "no created mob is left");
                let used = natural.provisional.get();
                drop(natural);
                (
                    pending.iter().map(snap).collect::<Vec<_>>(),
                    used,
                    random.next_int(),
                )
            };
            let spawns = || {
                vec![mod_api::ModEntry::new(
                    "sulfur-cubes",
                    Box::new(SulfurCubesOnly),
                )]
            };
            let (unlistened, heard) = with_checks(None, spawns, || attempt(game));
            assert!(heard.is_empty());
            assert!(!unlistened.0.is_empty(), "the attempt spawns");
            assert_eq!(
                unlistened.1 as usize,
                unlistened.0.len(),
                "one provisional id per spawned mob"
            );
            let (listened, heard) =
                with_checks(Some(SpawnCheck::Default), spawns, || attempt(game));
            assert_eq!(listened, unlistened, "same draws, ids and mobs");
            let positions: Vec<_> = heard.iter().filter(|h| h.hook == "position").collect();
            assert!(positions.len() >= unlistened.0.len());
            for h in &heard {
                assert_eq!(
                    (h.kind.as_str(), h.level, h.spawner),
                    (SULFUR_CUBE, level, None)
                );
            }
            assert!(heard.iter().any(|h| h.hook == "placement"));
            // Each spawned mob was read at its own position, a full mob (its head slot answers).
            for m in &unlistened.0 {
                let reads = (
                    Ok(mod_api::Identifier::parse(SULFUR_CUBE).unwrap()),
                    Ok(m.2),
                    Ok(true),
                );
                assert!(positions.iter().any(|h| h.mob.as_ref() == Some(&reads)));
            }
            let (refused, heard) = with_checks(Some(SpawnCheck::Fail), spawns, || attempt(game));
            assert!(refused.0.is_empty(), "a failed check adds nothing");
            assert_eq!(refused.1, 0, "and gives each provisional id back");
            assert!(heard.iter().any(|h| h.hook == "position"));
        });
    }

    /// A spawner block and a spawner minecart: placement checks carry no spawner (NeoForge's
    /// `SpawnPlacementCheck`), while position checks (`PositionCheck.getSpawner`) and
    /// `finalize_spawn` (`FinalizeSpawnEvent.getSpawner`) carry `Spawner::Block` for a block
    /// spawner and `Spawner::Minecart` for a spawner minecart; a `Default` listener on the
    /// position check leaves the draws, the real ids and the mobs that join as with none (the lent
    /// mob is the one that joins); a `Fail` adds no mob, uses no entity id and leaves no tracking
    /// entry. Configured spawn data lends a fully loaded mob.
    #[test]
    #[ignore = "daily"]
    fn spawner_checks_tell_the_block_and_keep_the_draws() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
        use sunrise_level::chunk::block_entity::Next;
        use sunrise_sim::mobs::spawner::{SpawnData, SpawnDataExt};
        if run_fresh_cases(
            &format!(
                "{}::spawner_checks_tell_the_block_and_keep_the_draws",
                module_path!().split_once("::").unwrap().1
            ),
            "SUNRISE_BLOCK_SPAWNER_CHECKS",
            &["checks"],
        ) {
            return;
        }
        with_game(|game| {
            spawn_check_world(game, Vec3::new(0.5, 70.0, 0.5));
            let level = level_of(game);
            let key = (2, 70, 2);
            let block = mod_api::Spawner::Block {
                pos: mod_api::BlockPos::new(2, 70, 2),
            };
            let tick = |game: &mut crate::game::ServerGame<'_>, extra: Option<(&str, Tag)>| {
                let (mobs, entities, tracked) = (
                    game.mobs.len(),
                    game.server.next_entity,
                    game.persist.uuids.len(),
                );
                game.world.random.set_seed(7);
                let mut entity = Compound::default();
                entity.insert("id".into(), Tag::String(SULFUR_CUBE.into()));
                if let Some((name, value)) = extra {
                    entity.insert(name.into(), value);
                }
                let mut spawner = sunrise_sim::mobs::spawner::Spawner {
                    spawn_delay: 0,
                    next: Some(Next::Own(SpawnData::new(entity, None, None))),
                    ..Default::default()
                };
                game.server_tick_spawner_of(key, &mut spawner, None)
                    .unwrap();
                let added: Vec<Snap> = game.mobs[mobs..].iter().map(snap).collect();
                let out = (
                    added,
                    game.server.next_entity - entities,
                    game.persist.uuids.len() - tracked,
                    game.world.random.next_int(),
                );
                for m in game.mobs.drain(mobs..) {
                    game.persist.uuids.remove(&m.id);
                }
                game.server.next_entity = entities;
                out
            };
            let (unlistened, _) = with_checks(None, Vec::new, || tick(game, None));
            assert!(!unlistened.0.is_empty(), "the spawner spawns");
            let (listened, heard) =
                with_checks(Some(SpawnCheck::Default), Vec::new, || tick(game, None));
            assert_eq!(listened, unlistened, "same draws, ids and mobs");
            assert!(heard.iter().any(|h| h.hook == "placement"));
            assert!(heard.iter().any(|h| h.hook == "position"));
            assert!(heard.iter().any(|h| h.hook == "finalize"));
            for h in &heard {
                let expected = if h.hook == "placement" {
                    None
                } else {
                    Some(block)
                };
                assert_eq!(
                    (h.kind.as_str(), h.level, h.spawner),
                    (SULFUR_CUBE, level, expected)
                );
                if let Some(reads) = &h.mob {
                    assert_eq!(reads.2, Ok(true), "a fresh spawn lends the whole mob");
                }
            }
            for m in &unlistened.0 {
                assert!(
                    heard
                        .iter()
                        .any(|h| h.mob.as_ref().is_some_and(|r| r.1 == Ok(m.2)))
                );
            }
            let (refused, heard) =
                with_checks(Some(SpawnCheck::Fail), Vec::new, || tick(game, None));
            assert!(heard.iter().any(|h| h.hook == "position"));
            assert_eq!(
                (refused.0.len(), refused.1, refused.2),
                (0, 0, 0),
                "no mob, no entity id, no tracking entry"
            );
            let helmet = || {
                Some((
                    "equipment",
                    Tag::compound([(
                        "head",
                        Tag::compound([
                            ("id", Tag::String("minecraft:iron_helmet".into())),
                            ("count", Tag::Int(1)),
                        ]),
                    )]),
                ))
            };
            let (configured, heard) =
                with_checks(Some(SpawnCheck::Default), Vec::new, || tick(game, helmet()));
            assert!(!configured.0.is_empty());
            // The configured entity is loaded before the position check, so the check reads the
            // equipment its data gave, as the finalization does.
            for (hook, expected) in [("position", Ok(false)), ("finalize", Ok(false))] {
                let event = heard.iter().find(|h| h.hook == hook).unwrap();
                let (kind, position, head) = event.mob.as_ref().unwrap();
                assert_eq!(*kind, Ok(mod_api::Identifier::parse(SULFUR_CUBE).unwrap()));
                assert!(position.is_ok());
                assert_eq!(*head, expected);
                assert_eq!(event.spawner, Some(block));
            }
            let cancel = || {
                vec![mod_api::ModEntry::new(
                    "cancel-configured",
                    Box::new(FinalizeMod(FinalizeAnswer::StopSpawn)),
                )]
            };
            let (cancelled, heard) =
                with_checks(Some(SpawnCheck::Default), cancel, || tick(game, helmet()));
            assert!(heard.iter().any(|h| h.hook == "finalize"));
            assert_eq!((cancelled.0.len(), cancelled.1, cancelled.2), (0, 0, 0));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn spawner_minecart_hooks_read_the_joined_cart_during_its_tick() {
        use crate::game::native_entity_refs::tests::with_game;
        use sunrise_level::chunk::block_entity::Next;
        use sunrise_sim::mobs::spawner::{SpawnData, SpawnDataExt, Spawner};
        use sunrise_sim::ticket_levels::{Sources, ViewedTicket};
        with_game(|game| {
            spawn_check_world(game, Vec3::new(0.5, 70.0, 0.5));
            game.server.tickets.0.update(Some(Sources {
                viewed: vec![ViewedTicket {
                    center: (0, 0),
                    radius: 0,
                    ticking: true,
                }],
                ..Default::default()
            }));
            let start = [2.5, 70.0, 2.5];
            let cart_kind = "minecraft:spawner_minecart";
            let Some(Target::Cart(id)) = game.summon_with_nbt(cart_kind, start, "{}").unwrap()
            else {
                panic!("spawner minecart joined");
            };
            let owner = game.entity(id).unwrap();
            let entity = [
                ("id".into(), Tag::String(SULFUR_CUBE.into())),
                (
                    "Pos".into(),
                    Tag::List(
                        [4.5f64, 70.0, 4.5]
                            .map(|n| Tag::Double(n.to_bits()))
                            .to_vec(),
                    ),
                ),
                (
                    "equipment".into(),
                    Tag::compound([(
                        "head",
                        Tag::compound([
                            ("id", Tag::String("minecraft:iron_helmet".into())),
                            ("count", Tag::Int(1)),
                        ]),
                    )]),
                ),
            ]
            .into();
            game.carts.update(0, |cart| {
                cart.spawner = Some(Box::new(Spawner {
                    spawn_delay: 0,
                    spawn_count: 1,
                    next: Some(Next::Own(SpawnData::new(entity, None, None))),
                    ..Default::default()
                }));
            });
            game.world.random.set_seed(7);
            let (_, heard) = with_checks(Some(SpawnCheck::Default), Vec::new, || game.tick_carts());
            let position = game.carts[0].position.components();
            assert_ne!(position, start, "the cart moved before its spawner ticked");
            for hook in ["position", "finalize"] {
                let event = heard.iter().find(|h| h.hook == hook).unwrap();
                assert_eq!(
                    event.spawner,
                    Some(mod_api::Spawner::Minecart { entity: owner })
                );
                assert_eq!(event.cart, Some((Some(cart_kind.into()), Some(position))));
            }
            // The configured entity's equipment is readable in the position check, on the
            // minecart path too: the tree the spawner loaded is the one lent.
            let event = heard.iter().find(|h| h.hook == "position").unwrap();
            assert_eq!(event.mob.as_ref().unwrap().2, Ok(false), "a helmet is worn");
        });
    }

    /// A trial spawner: `mob_spawn_placement_check` runs with no spawner (NeoForge's
    /// `SpawnPlacementCheck`), `mob_spawn_position_check` does not run (NeoForge's `TrialSpawner`
    /// is not a `BaseSpawner`), and `finalize_spawn` hears `Spawner::Trial` at the block; a
    /// `Default` listener leaves the draws, ids and mob as with none; a `Fail` placement check
    /// adds no mob and no tracking entry.
    #[test]
    #[ignore = "daily"]
    fn trial_spawner_checks_tell_the_block_and_keep_the_draws() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
        use sunrise_sim::mobs::spawner::{SpawnData, SpawnDataExt};
        use sunrise_sim::mobs::trial_spawner::{TrialSpawner, TrialState};
        if run_fresh_cases(
            &format!(
                "{}::trial_spawner_checks_tell_the_block_and_keep_the_draws",
                module_path!().split_once("::").unwrap().1
            ),
            "SUNRISE_TRIAL_SPAWNER_CHECKS",
            &["checks"],
        ) {
            return;
        }
        with_game(|game| {
            spawn_check_world(game, Vec3::new(0.5, 70.0, 0.5));
            let level = level_of(game);
            let key = (2, 70, 2);
            let trial = mod_api::Spawner::Trial {
                pos: mod_api::BlockPos::new(2, 70, 2),
            };
            let tick = |game: &mut crate::game::ServerGame<'_>, configured: bool| {
                let (mobs, entities, tracked) = (
                    game.mobs.len(),
                    game.server.next_entity,
                    game.persist.uuids.len(),
                );
                game.world.random.set_seed(9);
                let mut entity = Compound::default();
                entity.insert("id".into(), Tag::String(SULFUR_CUBE.into()));
                if configured {
                    entity.insert("Health".into(), Tag::Float(5.0f32.to_bits()));
                }
                let mut spawner = TrialSpawner {
                    next_spawn_data: Some(SpawnData::new(entity, None, None)),
                    ..Default::default()
                };
                game.server_tick_trial_spawner(key, &mut spawner, TrialState::Active, false)
                    .unwrap();
                let added: Vec<Snap> = game.mobs[mobs..].iter().map(snap).collect();
                let out = (
                    added,
                    game.server.next_entity - entities,
                    game.persist.uuids.len() - tracked,
                    game.world.random.next_int(),
                );
                for m in game.mobs.drain(mobs..) {
                    game.persist.uuids.remove(&m.id);
                }
                game.server.next_entity = entities;
                out
            };
            let (unlistened, _) = with_checks(None, Vec::new, || tick(game, false));
            assert_eq!(unlistened.0.len(), 1, "the trial spawner spawns one mob");
            assert!(unlistened.0[0].4, "a trial spawner's mob is persistent");
            let (listened, heard) =
                with_checks(Some(SpawnCheck::Default), Vec::new, || tick(game, false));
            assert_eq!(listened, unlistened, "same draws, ids and mob");
            assert!(heard.iter().all(|h| h.hook != "position"));
            assert!(heard.iter().any(|h| h.hook == "placement"));
            assert!(
                !heard.iter().any(|h| h.hook == "position"),
                "trial spawners do not fire PositionCheck"
            );
            assert!(heard.iter().any(|h| h.hook == "finalize"));
            for h in &heard {
                let expected = if h.hook == "finalize" {
                    Some(trial)
                } else {
                    None
                };
                assert_eq!(
                    (h.kind.as_str(), h.level, h.spawner),
                    (SULFUR_CUBE, level, expected)
                );
            }
            // A trial spawner has no position event, so only a placement refusal stops it.
            let refuse = || {
                vec![mod_api::ModEntry::new(
                    "refuse-placement",
                    Box::new(ChecksMod(SpawnCheck::Fail)),
                )]
            };
            let (refused, heard) =
                with_checks(Some(SpawnCheck::Default), refuse, || tick(game, false));
            assert!(!heard.is_empty());
            assert!(heard.iter().all(|h| h.hook == "placement"));
            assert_eq!(
                (refused.0.len(), refused.1, refused.2),
                (0, 0, 0),
                "no mob, no entity id, no tracking entry"
            );
            let (configured, heard) =
                with_checks(Some(SpawnCheck::Default), Vec::new, || tick(game, true));
            assert_eq!(configured.0.len(), 1);
            assert!(
                configured.0[0].4,
                "a configured trial mob is persistent too"
            );
            let event = heard.iter().find(|h| h.hook == "finalize").unwrap();
            assert_eq!(event.spawner, Some(trial));
            assert_eq!(event.mob.as_ref().unwrap().2, Ok(true));
        });
    }

    type NameLog = std::sync::Arc<std::sync::Mutex<Vec<(&'static str, Option<mod_api::Text>)>>>;

    struct ConfiguredMobListener {
        veto: bool,
        heard: NameLog,
    }

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for ConfiguredMobListener {
        fn mob_spawn_position_check(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::MobSpawnPositionCheckEvent<'_>,
        ) {
            self.heard
                .lock()
                .unwrap()
                .push(("position", event.mob.custom_name().unwrap()));
        }

        fn finalize_spawn(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::FinalizeSpawnEvent<'_>,
        ) -> mod_api::Flow {
            self.heard
                .lock()
                .unwrap()
                .push(("finalize", event.mob.custom_name().unwrap()));
            event
                .mob
                .set_custom_name(Some(mod_api::Text::literal("edited")))
                .unwrap();
            event.spawn_cancelled = self.veto;
            mod_api::Flow::Continue
        }
    }

    #[test]
    #[ignore = "daily"]
    fn configured_spawner_lends_and_finalizes_its_loaded_body() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
        use sunrise_level::chunk::block_entity::Next;
        use sunrise_sim::mobs::spawner::{SpawnData, SpawnDataExt};
        const FLAG: &str = "SUNRISE_CONFIGURED_SPAWNER_LEND";
        if run_fresh_cases(
            &format!(
                "{}::configured_spawner_lends_and_finalizes_its_loaded_body",
                module_path!().split_once("::").unwrap().1
            ),
            FLAG,
            &["keep", "veto"],
        ) {
            return;
        }
        let veto = std::env::var(FLAG).unwrap() == "veto";
        with_game(|game| {
            spawn_check_world(game, Vec3::new(0.5, 70.0, 0.5));
            game.world.random.set_seed(7);
            let before = game.mobs.len();
            let uuid_count = game.persist.uuids.len();
            let mut entity = Compound::default();
            entity.insert("id".into(), Tag::String(SULFUR_CUBE.into()));
            entity.insert("CustomName".into(), Tag::String("configured".into()));
            entity.insert("Health".into(), Tag::Float(5.0_f32.to_bits()));
            let mut spawner = sunrise_sim::mobs::spawner::Spawner {
                spawn_delay: 0,
                spawn_count: 1,
                next: Some(Next::Own(SpawnData::new(entity, None, None))),
                ..Default::default()
            };
            let heard = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            mod_platform::testing::with_mods(
                vec![mod_api::ModEntry::new(
                    "configured-spawner",
                    Box::new(ConfiguredMobListener {
                        veto,
                        heard: heard.clone(),
                    }),
                )]
                .into(),
                || {
                    game.server_tick_spawner_of((2, 70, 2), &mut spawner, None)
                        .unwrap()
                },
            );
            let name = || Some(mod_api::Text::literal("configured"));
            assert_eq!(
                *heard.lock().unwrap(),
                [("position", name()), ("finalize", name())]
            );
            if veto {
                assert_eq!(game.mobs.len(), before);
                assert_eq!(game.persist.uuids.len(), uuid_count);
            } else {
                assert_eq!(game.mobs.len(), before + 1);
                let mob = &game.mobs[before];
                assert_eq!(mob.custom_name.as_deref(), Some("edited"));
                // No `Size`: AbstractCubeMob.readAdditionalSaveData sets size 1, whose maximum
                // health of 4 clamps the saved 5 (LivingEntity.setHealth).
                assert_eq!(mob.health.to_bits(), 4.0_f32.to_bits());
            }
        });
    }

    /// On spawners, `spawn_cancelled` vetoes admission after vanilla finalization
    /// (when unconfigured and not cancelled with `Flow::Cancel`), trial persistence,
    /// and any configured equipment table roll (NeoForge's
    /// `EventHooks.finalizeMobSpawnSpawner`, `BaseSpawner.serverTick`,
    /// `TrialSpawner.spawnMob`, and `NeoForgeEventHandler.builtinMobSpawnBlocker`).
    #[test]
    #[ignore = "daily"]
    fn configured_spawner_veto_rolls_equipment_without_joining() {
        use crate::game::native_entity_refs::tests::with_game;
        use sunrise_items::random_sequences::RandomSequences;
        use sunrise_level::chunk::block_entity::{CustomSpawnRules, Next};
        use sunrise_sim::mobs::spawner::{SpawnData, SpawnDataExt, Spawner};
        use sunrise_sim::mobs::trial_spawner::{TrialSpawner, TrialState};

        with_game(|game| {
            spawn_check_world(game, Vec3::new(0.5, 70.0, 0.5));
            game.load_server_loot();
            game.level.seed = 42;
            let key = (2, 70, 2);
            let equipment = Tag::compound([
                (
                    "loot_table",
                    Tag::String("minecraft:equipment/trial_chamber_melee".into()),
                ),
                ("slot_drop_chances", Tag::Float(0.0f32.to_bits())),
            ]);
            let empty_sequences = RandomSequences::default().to_nbt();

            let run_block = |game: &mut crate::game::ServerGame<'_>,
                             kind: &str,
                             configured: bool,
                             with_equipment: bool,
                             answer: Option<FinalizeAnswer>| {
                let (mobs, entities, tracked) = (
                    game.mobs.len(),
                    game.server.next_entity,
                    game.persist.uuids.len(),
                );
                game.server.rand_sequences = RandomSequences::default();
                game.world.random.set_seed(7);
                let mut entity = Compound::default();
                entity.insert("id".into(), Tag::String(kind.into()));
                if configured {
                    entity.insert("Health".into(), Tag::Float(5.0f32.to_bits()));
                }
                // BaseSpawner.serverTick uses custom light rules in place of regular spawn rules.
                // Use the full SpawnData.CustomSpawnRules.LIGHT_RANGE for the cow fixture.
                let rules = (kind == "minecraft:cow").then_some(CustomSpawnRules {
                    block: (0, 15),
                    sky: (0, 15),
                });
                let mut spawner = Spawner {
                    spawn_delay: 0,
                    spawn_count: 1,
                    next: Some(Next::Own(SpawnData::new(
                        entity,
                        rules,
                        with_equipment.then(|| equipment.clone()),
                    ))),
                    ..Default::default()
                };
                let mut go = |game: &mut crate::game::ServerGame<'_>| {
                    game.server_tick_spawner_of(key, &mut spawner, None)
                        .unwrap();
                };
                match answer {
                    None => go(game),
                    Some(a) => mod_platform::testing::with_mods(
                        vec![mod_api::ModEntry::new("finalize", Box::new(FinalizeMod(a)))].into(),
                        || go(game),
                    ),
                }
                let out = (
                    game.mobs.len() - mobs,
                    game.server.next_entity - entities,
                    game.persist.uuids.len() - tracked,
                    game.server.rand_sequences.to_nbt(),
                    game.world.random.next_int(),
                );
                for m in game.mobs.drain(mobs..) {
                    game.persist.uuids.remove(&m.id);
                }
                game.server.next_entity = entities;
                out
            };

            let joined = run_block(game, SULFUR_CUBE, true, true, None);
            assert_eq!(joined.0, 1, "configured block spawner spawns one mob");
            assert_ne!(
                joined.3, empty_sequences,
                "configured equipment table rolled its random sequence"
            );
            let vetoed = run_block(
                game,
                SULFUR_CUBE,
                true,
                true,
                Some(FinalizeAnswer::StopSpawn),
            );
            assert_eq!(
                (vetoed.0, vetoed.1, vetoed.2),
                (0, 0, 0),
                "spawn_cancelled vetoes admission"
            );
            assert_eq!(
                vetoed.3, joined.3,
                "spawn_cancelled preserves the configured equipment table roll"
            );

            // Unconfigured block spawner: `spawn_cancelled` alone still runs vanilla
            // finalization before admission veto; `Flow::Cancel` skips finalization.
            let plain_joined = run_block(game, SULFUR_CUBE, false, false, None);
            assert_eq!(plain_joined.0, 1);
            let plain_vetoed = run_block(
                game,
                SULFUR_CUBE,
                false,
                false,
                Some(FinalizeAnswer::StopSpawn),
            );
            assert_eq!((plain_vetoed.0, plain_vetoed.1, plain_vetoed.2), (0, 0, 0));
            assert_eq!(
                plain_vetoed.4, plain_joined.4,
                "spawn_cancelled without Flow::Cancel preserves vanilla finalization draws"
            );
            let plain_skipped = run_block(
                game,
                SULFUR_CUBE,
                false,
                false,
                Some(FinalizeAnswer::SkipAndStop),
            );
            assert_eq!(
                (plain_skipped.0, plain_skipped.1, plain_skipped.2),
                (0, 0, 0)
            );
            assert_ne!(
                plain_skipped.4, plain_joined.4,
                "Flow::Cancel skips vanilla finalization draws"
            );

            let cow_joined = run_block(game, "minecraft:cow", false, false, None);
            assert_eq!(cow_joined.0, 1);
            let cow_vetoed = run_block(
                game,
                "minecraft:cow",
                false,
                false,
                Some(FinalizeAnswer::StopAgeableSpawn),
            );
            assert_eq!((cow_vetoed.0, cow_vetoed.1, cow_vetoed.2), (0, 0, 0));
            assert_eq!(
                cow_vetoed.4, cow_joined.4,
                "an ageable group at the Java int limit still finalizes before the veto"
            );

            let run_trial = |game: &mut crate::game::ServerGame<'_>,
                             answer: Option<FinalizeAnswer>| {
                let (mobs, entities, tracked) = (
                    game.mobs.len(),
                    game.server.next_entity,
                    game.persist.uuids.len(),
                );
                game.server.rand_sequences = RandomSequences::default();
                game.world.random.set_seed(9);
                let mut entity = Compound::default();
                entity.insert("id".into(), Tag::String(SULFUR_CUBE.into()));
                entity.insert("Health".into(), Tag::Float(5.0f32.to_bits()));
                let mut spawner = TrialSpawner {
                    next_spawn_data: Some(SpawnData::new(entity, None, Some(equipment.clone()))),
                    ..Default::default()
                };
                let mut go = |game: &mut crate::game::ServerGame<'_>| {
                    game.server_tick_trial_spawner(key, &mut spawner, TrialState::Active, false)
                        .unwrap();
                };
                match answer {
                    None => go(game),
                    Some(a) => mod_platform::testing::with_mods(
                        vec![mod_api::ModEntry::new("finalize", Box::new(FinalizeMod(a)))].into(),
                        || go(game),
                    ),
                }
                let out = (
                    game.mobs.len() - mobs,
                    game.server.next_entity - entities,
                    game.persist.uuids.len() - tracked,
                    spawner.total_mobs_spawned,
                    spawner.current_mobs.len(),
                    game.server.rand_sequences.to_nbt(),
                );
                for m in game.mobs.drain(mobs..) {
                    game.persist.uuids.remove(&m.id);
                }
                game.server.next_entity = entities;
                out
            };

            let trial_joined = run_trial(game, None);
            assert_eq!((trial_joined.0, trial_joined.3, trial_joined.4), (1, 1, 1));
            assert_ne!(
                trial_joined.5, empty_sequences,
                "trial spawner equipment table rolled its random sequence"
            );
            let trial_vetoed = run_trial(game, Some(FinalizeAnswer::StopSpawn));
            assert_eq!(
                (
                    trial_vetoed.0,
                    trial_vetoed.1,
                    trial_vetoed.2,
                    trial_vetoed.3,
                    trial_vetoed.4,
                ),
                (0, 0, 0, 0, 0),
                "trial spawner spawn_cancelled vetoes admission"
            );
            assert_eq!(
                trial_vetoed.5, trial_joined.5,
                "trial spawner spawn_cancelled preserves the configured equipment table roll"
            );
        });
    }

    /// No listener and a listener that changes nothing finalize and draw as vanilla; a cancel
    /// skips finalization and its draws, the mob still spawning; `spawn_cancelled` adds no mob.
    /// The veto is an admission decision, independent of the three draws and the
    /// follow-range bonus from `Mob.finalizeSpawn` (the silverfish has no override).
    #[test]
    #[ignore = "daily"]
    fn finalize_spawn_cancels_pin_the_draws_and_the_outcome() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
        use sunrise_gameplay_common::game::creatures::SpawnSite;
        if run_fresh_cases(
            &format!(
                "{}::finalize_spawn_cancels_pin_the_draws_and_the_outcome",
                module_path!().split_once("::").unwrap().1
            ),
            "SUNRISE_FINALIZE_CANCEL_CASE",
            &["independent"],
        ) {
            return;
        }
        with_game(|game| {
            for (skip, veto) in [(false, true), (true, false), (true, true)] {
                game.world.random = LevelRandom::for_level(LegacyRandom::new(5));
                let answer = match (skip, veto) {
                    (false, false) => FinalizeAnswer::Pass,
                    (true, false) => FinalizeAnswer::SkipFinalize,
                    (false, true) => FinalizeAnswer::StopSpawn,
                    (true, true) => FinalizeAnswer::SkipAndStop,
                };
                let mods = vec![mod_api::ModEntry::new(
                    "finalize",
                    Box::new(FinalizeMod(answer)),
                )];
                let spawned = mod_platform::testing::with_mods(mods.into(), || {
                    game.spawn_mob(
                        "minecraft:silverfish",
                        [8.5, 64.0, 8.5],
                        false,
                        SpawnSite::Finalizing(Reason::Command),
                    )
                });
                assert_eq!(spawned.is_none(), veto);
                let mut expected = LegacyRandom::new(5);
                if !skip {
                    let _ = (
                        expected.next_double(),
                        expected.next_double(),
                        expected.next_float(),
                    );
                }
                assert_eq!(
                    game.world.random.next_int(),
                    expected.next_int(),
                    "skip={skip}, veto={veto}"
                );
                if let Some(id) = spawned {
                    let mob = game.mobs.iter().find(|mob| mob.id == id).unwrap();
                    assert!(!mob.extra.attributes.as_ref().is_some_and(|a| {
                        a.modifiers
                            .iter()
                            .any(|m| m.id == "minecraft:random_spawn_bonus")
                    }));
                }
            }
        });
    }

    /// A site vanilla makes without `finalizeSpawn` (an infested block's silverfish) is not
    /// told; one it finalizes is.
    #[test]
    #[ignore = "daily"]
    fn finalize_spawn_is_told_only_where_vanilla_finalizes() {
        use crate::game::native_entity_refs::tests::with_game;
        use sunrise_gameplay_common::game::creatures::SpawnSite;
        let heard = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mods = vec![mod_api::ModEntry::new(
            "count-finalize",
            Box::new(CountFinalize(heard.clone())),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_game(|game| {
                let at = [8.5, 64.0, 8.5];
                let count = || heard.load(std::sync::atomic::Ordering::Relaxed);
                assert!(
                    game.spawn_mob("minecraft:silverfish", at, false, SpawnSite::Plain)
                        .is_some()
                );
                assert_eq!(count(), 0, "a plain site is not told");
                assert!(
                    game.spawn_mob(
                        "minecraft:silverfish",
                        at,
                        false,
                        SpawnSite::Finalizing(Reason::Spawner)
                    )
                    .is_some()
                );
                assert_eq!(count(), 1, "a finalizing site is told once");
            });
        });
    }

    /// `ZombieHorse.finalizeSpawn` finalizes its rider with the horse's own
    /// `spawnReason`, a baby zombie's chicken with `JOCKEY`.
    #[test]
    #[ignore = "daily"]
    fn jockeys_finalize_with_the_parents_reason() {
        use crate::game::native_entity_refs::tests::with_game;
        use sunrise_gameplay_common::game::creatures::SpawnSite;
        let heard = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "record-finalize",
            Box::new(RecordFinalize(heard.clone())),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_game(|game| {
                game.ui.difficulty = 2;
                let at = [8.5, 64.0, 8.5];
                game.spawn_mob(
                    "minecraft:zombie_horse",
                    at,
                    false,
                    SpawnSite::Finalizing(Reason::Natural),
                )
                .unwrap();
                let heard = heard.lock().unwrap();
                let natural = Reason::Natural;
                assert_eq!(
                    *heard,
                    [
                        ("minecraft:zombie_horse".to_owned(), natural),
                        ("minecraft:zombie".to_owned(), natural)
                    ]
                );
            });
        });
    }

    /// `MobDespawnEvent` wraps the whole of `Mob.checkDespawn`: `Default` and no
    /// listener leave vanilla's checks and their `nextInt(800)` draw as they
    /// are, `Deny` keeps a peaceful-mode zombie and draws nothing, `Allow`
    /// discards it in normal and draws nothing. The mob's own random is the
    /// real draw source (a `LegacyRandom`), compared with an untouched copy.
    /// A mob in an unloaded chunk is never asked.
    #[test]
    #[ignore = "daily"]
    fn mob_despawn_answers_override_vanilla_and_default_passes_through() {
        use crate::game::native_entity_refs::tests::with_game;
        use mod_api::Despawn::{Allow, Default as Vanilla, Deny};
        use std::sync::atomic::Ordering::Relaxed;
        let answer = std::sync::Arc::new(std::sync::Mutex::new(Vanilla));
        let asked = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mods = vec![mod_api::ModEntry::new(
            "despawns",
            Box::new(Despawns(answer.clone(), asked.clone())),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_game(|game| {
                let id = game
                    .spawn_mob(
                        "minecraft:zombie",
                        [8.5, 64.0, 8.5],
                        false,
                        sunrise_gameplay_common::game::creatures::SpawnSite::Plain,
                    )
                    .unwrap();
                // Asks the hook, then runs `despawn_check` for an idle zombie 40
                // blocks from the player. Returns whether it stayed and whether
                // its random is where an untouched copy of it is.
                let ask = |game: &mut crate::game::ServerGame<'_>, said, peaceful: bool| {
                    *answer.lock().unwrap() = said;
                    let mut mob = game.mobs.iter().find(|m| m.id == id).unwrap().clone();
                    mob.ai.no_action_time = 601;
                    let mut untouched = mob.random.clone();
                    let handle = mod_api::MobRef::new(mod_api::EntityRef::new(
                        game.native_lifecycle.epoch().unwrap(),
                        id,
                    ));
                    let pos = [mob.position.x, mob.position.y, mob.position.z];
                    sunrise_gameplay_common::game::native_hooks::NativeHookHost { host: game }
                        .native_mob_despawn(
                            "minecraft:overworld",
                            handle,
                            &mob.kind.id,
                            pos,
                            &mut mob.despawn,
                        );
                    let mut world = game.mob_world_base(&[]);
                    world.difficulty.effective = if peaceful { 0.0 } else { 2.0 };
                    let mut player = mob.position;
                    player.x += 40.0;
                    world.nearest_player = Some(player);
                    let kept = sunrise_sim::mobs::ai::despawn_check(&mut mob, &world);
                    let untouched_random =
                        mob.random.next_int_bound(1 << 30) == untouched.next_int_bound(1 << 30);
                    (kept, untouched_random)
                };
                assert!(!ask(game, Vanilla, true).0, "Default: peaceful discards it");
                assert_eq!(
                    ask(game, Deny, true),
                    (true, true),
                    "Deny keeps it, no draw"
                );
                assert_eq!(
                    ask(game, Allow, false),
                    (false, true),
                    "Allow discards it, no draw"
                );
                assert!(
                    !ask(game, Vanilla, false).1,
                    "Default rolls vanilla's nextInt(800)"
                );
                assert_eq!(asked.load(Relaxed), 4);
                // No chunk is loaded here, so the mob loop skips the mob before
                // the hook: an unloaded mob is never asked.
                *answer.lock().unwrap() = Deny;
                game.tick_creatures().unwrap();
                assert_eq!(asked.load(Relaxed), 4, "an unloaded mob is not asked");
            });
        });
    }

    /// With no mod listening, `Mob.despawn` stays `Default` and vanilla's checks
    /// and draws run as before.
    #[test]
    #[ignore = "daily"]
    fn mob_despawn_with_no_listener_draws_as_vanilla() {
        use crate::game::native_entity_refs::tests::with_game;
        with_game(|game| {
            let id = game
                .spawn_mob(
                    "minecraft:zombie",
                    [8.5, 64.0, 8.5],
                    false,
                    sunrise_gameplay_common::game::creatures::SpawnSite::Plain,
                )
                .unwrap();
            let mut mob = game.mobs.iter().find(|m| m.id == id).unwrap().clone();
            mob.ai.no_action_time = 601;
            let mut untouched = mob.random.clone();
            let mut world = game.mob_world_base(&[]);
            world.difficulty.effective = 2.0;
            let mut player = mob.position;
            player.x += 40.0;
            world.nearest_player = Some(player);
            sunrise_sim::mobs::ai::despawn_check(&mut mob, &world);
            assert_eq!(mob.despawn, mod_api::Despawn::Default);
            assert_ne!(
                mob.random.next_int_bound(1 << 30),
                untouched.next_int_bound(1 << 30),
                "vanilla's nextInt(800) is drawn"
            );
        });
    }

    const NATURAL_HORSE: &str = "minecraft:zombie_horse";
    const NATURAL_RIDER: &str = "minecraft:zombie";
    // Independent Java LCG calculation: successive wrapping seed-uniquifier steps
    // from 12345, followed by Entity.<init>'s two nextLongs and UUID masks.
    const HORSE_UUID: u128 = 0x5725645a0d694163_8ea9477e028e0fc3;
    const RIDER_UUID: u128 = 0xd5f0a5e475f54728_8649f20043164fd8;

    struct HorseIdentityMod {
        veto: bool,
        joined: std::sync::Arc<std::sync::Mutex<Vec<(u32, Option<u32>)>>>,
    }

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for HorseIdentityMod {
        fn potential_spawns(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::PotentialSpawnsEvent<'_>,
        ) -> mod_api::Flow {
            if event.category == "monster" {
                *event.entries = vec![SpawnEntry {
                    entity: NATURAL_HORSE.into(),
                    weight: 1,
                    min: 1,
                    max: 1,
                    constant: true,
                }];
            }
            mod_api::Flow::Continue
        }

        fn mob_spawn_placement_check(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::MobSpawnPlacementCheckEvent<'_>,
        ) {
            *event.result = SpawnCheck::Succeed;
        }

        fn mob_spawn_position_check(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::MobSpawnPositionCheckEvent<'_>,
        ) {
            *event.result = SpawnCheck::Succeed;
        }

        fn entity_join_level(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityJoinLevelEvent<'_>,
        ) -> mod_api::Flow {
            let expected = match event.kind {
                NATURAL_HORSE => HORSE_UUID,
                NATURAL_RIDER => RIDER_UUID,
                _ => return mod_api::Flow::Continue,
            };
            let resolved = host
                .level(event.dimension)
                .and_then(|level| level.entity_by_uuid(host, mod_api::EntityUuid(expected)))
                .map(|entity| entity.id());
            self.joined
                .lock()
                .unwrap()
                .push((event.subject.id(), resolved));
            if self.veto && event.kind == NATURAL_RIDER {
                mod_api::Flow::Cancel
            } else {
                mod_api::Flow::Continue
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn natural_admission_publishes_primary_and_passenger_constructor_uuids() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
        if run_fresh_cases(
            &format!(
                "{}::natural_admission_publishes_primary_and_passenger_constructor_uuids",
                module_path!().split_once("::").unwrap().1
            ),
            "SUNRISE_TEST_SEEDS",
            &["1"],
        ) {
            return;
        }
        for veto in [false, true] {
            with_game(|game| {
                spawn_check_world(game, Vec3::new(0.5, 64.0, 0.5));
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                for x in 32..48 {
                    for z in 32..48 {
                        game.world.set(x, 63, z, stone).unwrap();
                    }
                }
                let joined = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
                let mods = vec![mod_api::ModEntry::new(
                    "natural-uuids",
                    Box::new(HorseIdentityMod {
                        veto,
                        joined: joined.clone(),
                    }),
                )];
                mod_platform::testing::with_mods(mods.into(), || {
                    use sunrise_sim::lan::playerdata::uuid_ints;
                    sunrise_level::random_source::set_thread_uniquifier(12345).unwrap();
                    let mut random = LevelRandom::for_level(LegacyRandom::new(42));
                    let (mut cluster, mut state, candidate, spawn) = {
                        let level = game.natural_level();
                        let players = vec![SpawnPlayer {
                            position: [0.5, 64.0, 0.5],
                            spectator: false,
                        }];
                        let mut state = SpawnState::new(&level, 289, players, &[], |_, _| true);
                        let mut cluster = SpawnCluster::new(
                            &level,
                            &mut state,
                            Category::Monster,
                            (2, 2),
                            [40, 64, 40],
                        )
                        .unwrap();
                        let candidate = cluster
                            .next_candidate(&level, &mut random, &mut state, &mut |_| {})
                            .unwrap();
                        let spawn = level
                            .finalize_spawned(&cluster.request(candidate), &mut random)
                            .unwrap();
                        (cluster, state, candidate, spawn)
                    };
                    game.complete_natural_spawn(
                        &mut cluster,
                        candidate,
                        spawn,
                        &[],
                        &mut HashMap::new(),
                        &mut random,
                        &mut state,
                        &[],
                    );
                    let seen = joined.lock().unwrap();
                    let [(horse, visible_horse), (rider, visible_rider)] = seen.as_slice() else {
                        panic!("expected one horse and its rider, saw {seen:?}");
                    };
                    assert_eq!(*visible_horse, Some(*horse));
                    assert_eq!(*visible_rider, Some(*rider));
                    assert_eq!(game.persist.uuids.get(horse), Some(&uuid_ints(HORSE_UUID)));
                    let rider_uuid = uuid_ints(RIDER_UUID);
                    assert_eq!(
                        game.persist.uuids.get(rider),
                        (!veto).then_some(&rider_uuid)
                    );
                    let level = game.level_ref(game.lend_epoch(), game.dim.key());
                    assert_eq!(
                        level
                            .entity_by_uuid(game, mod_api::EntityUuid(RIDER_UUID))
                            .map(|r| r.id()),
                        (!veto).then_some(*rider)
                    );
                    if !veto {
                        let mob = game.mobs.iter().find(|m| m.id == *rider).unwrap();
                        assert_eq!(mob.extra.mount, Some(*horse));
                    }
                });
            });
        }
    }

    /// The parts of the new mob's type edit one signed size in turn: the second sees the first's
    /// answer (4 doubled, then one more), a part that leaves it alone and a type with no part
    /// keep vanilla's. The mob they are told of is resident, so the first part reads it.
    #[test]
    #[ignore = "daily"]
    fn entity_parts_compose_on_the_spawn_cluster_size() {
        use crate::game::native_entity_refs::tests::with_game;
        let entries = vec![mod_api::ModEntry::new("clusters", Box::new(ClusterMod))];
        mod_platform::testing::with_mods(entries.into(), || {
            with_game(|game| {
                game.resolve_parts().unwrap();
                let ids: Vec<(&str, u32)> =
                    ["minecraft:zombie", "minecraft:skeleton", "minecraft:cow"]
                        .into_iter()
                        .map(|kind| {
                            let id = game
                                .spawn_mob(
                                    kind,
                                    [8.5, 64.0, 8.5],
                                    false,
                                    sunrise_gameplay_common::game::creatures::SpawnSite::Plain,
                                )
                                .unwrap();
                            (kind, id)
                        })
                        .collect();
                let game: &crate::game::ServerGame<'_> = game;
                let ask = |kind: &str| {
                    let (_, id) = ids.iter().find(|(k, _)| *k == kind).unwrap();
                    let mob = mod_api::MobRef::new(mod_api::EntityRef::new(game.lend_epoch(), *id));
                    game.spawn_cluster_size(mob, kind, 4)
                };
                assert_eq!(ask("minecraft:zombie"), 9);
                assert_eq!(ask("minecraft:skeleton"), 4);
                assert_eq!(ask("minecraft:cow"), 4);
            })
        });
    }

    #[test]
    #[ignore = "daily"]
    fn entity_parts_answer_the_spawn_cluster_size() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
        if run_fresh_cases(
            "game::server_side::natural_spawning::tests::entity_parts_answer_the_spawn_cluster_size",
            SPAWN_CLUSTER_CASE,
            &["valid", "reject", "move", "unwind"],
        ) {
            return;
        }
        let entries = vec![
            mod_api::ModEntry::new("clusters", Box::new(ClusterMod)),
            mod_api::ModEntry::new("sulfur-cubes", Box::new(SulfurCubesOnly)),
        ];
        mod_platform::__private::install(entries.into());
        sunrise_gameplay_common::game::api_values::install_parts(&[mod_api::ModEntry::new(
            "clusters",
            Box::new(ClusterMod),
        )])
        .unwrap();
        with_game(|game| {
            game.resolve_parts().unwrap();
            spawn_check_world(game, Vec3::new(0.5, 64.0, 0.5));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            for x in 32..48 {
                for z in 32..48 {
                    game.world.set(x, 63, z, stone).unwrap();
                }
            }
            let mut random = LevelRandom::for_level(LegacyRandom::new(42));
            let (mut cluster, mut state, candidate, primary) = {
                let level = game.natural_level();
                let players = vec![SpawnPlayer {
                    position: [0.5, 64.0, 0.5],
                    spectator: false,
                }];
                let mut state = SpawnState::new(&level, 289, players, &[], |_, _| true);
                let mut cluster =
                    SpawnCluster::new(&level, &mut state, Category::Monster, (2, 2), [40, 64, 40])
                        .unwrap();
                let candidate = cluster
                    .next_candidate(&level, &mut random, &mut state, &mut |_| {})
                    .unwrap();
                let mob = level
                    .finalize_spawned(&cluster.request(candidate), &mut random)
                    .unwrap();
                (cluster, state, candidate, mob)
            };
            let case = std::env::var(SPAWN_CLUSTER_CASE).unwrap();
            if case == "unwind" {
                let first = game.server.next_entity + 1;
                let mut primary = primary;
                primary.passengers = ["minecraft:cow", "minecraft:pig"]
                    .into_iter()
                    .enumerate()
                    .map(|(index, name)| {
                        let kind = game.res.mob_types().get(name).unwrap().clone();
                        let (random, uuid) = sunrise_sim::mobs::living_random();
                        (
                            Mob::with_random(
                                u32::MAX - 1 - index as u32,
                                kind,
                                Vec3::ZERO,
                                0.0,
                                random,
                            ),
                            uuid,
                        )
                    })
                    .collect();
                use crate::game::pending_births::escaping_fault::{self, At};
                escaping_fault::expect(At::Admission, first, || {
                    game.complete_natural_spawn(
                        &mut cluster,
                        candidate,
                        primary,
                        &[],
                        &mut HashMap::new(),
                        &mut random,
                        &mut state,
                        &[],
                    );
                });
                assert_eq!(
                    JOINED_CANDIDATE.load(std::sync::atomic::Ordering::Relaxed),
                    first,
                    "primary reached its join listener before the engine fault"
                );
                game.native_joins_flush(false, 0);
                assert!(
                    game.mobs
                        .iter()
                        .all(|m| !(first..=first + 2).contains(&m.id))
                );
                assert!(game.server.natural_admission.is_none());
                return;
            }
            game.complete_natural_spawn(
                &mut cluster,
                candidate,
                primary,
                &[],
                &mut HashMap::new(),
                &mut random,
                &mut state,
                &[],
            );
            let id = JOINED_CANDIDATE.load(std::sync::atomic::Ordering::Relaxed);
            assert_ne!(id, 0);
            assert_eq!(game.mobs.iter().any(|m| m.id == id), case == "valid");
            if case == "move" {
                let moved = game.server.levels.0.parked_mobs(Dim::Nether).unwrap();
                assert!(moved.iter().any(|m| {
                    m.species() == Species::SulfurCube
                        && m.id != id
                        && [m.position.x, m.position.y, m.position.z] == JOINED_POSITION
                }));
            }
            assert_eq!(
                CLUSTER_QUERIES.load(std::sync::atomic::Ordering::Relaxed),
                1
            );
            assert_eq!(state.counts().1.into_iter().sum::<i32>(), 1);
            assert!(
                cluster
                    .next_candidate(&game.natural_level(), &mut random, &mut state, &mut |_| {})
                    .is_none(),
                "the live candidate's part limits the cluster to one"
            );
        });
    }

    const SILVERFISH: &str = "minecraft:silverfish";
    const MOVED_PLAYER: [f64; 3] = [42.5, 64.0, 38.5];
    const RESUMPTION_PLAYER: u32 = 10;
    struct SilverfishPack {
        move_player: bool,
    }

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for SilverfishPack {
        fn potential_spawns(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::PotentialSpawnsEvent<'_>,
        ) -> mod_api::Flow {
            if event.category == "monster" {
                *event.entries = vec![SpawnEntry {
                    entity: SILVERFISH.into(),
                    weight: 1,
                    min: 4,
                    max: 4,
                    constant: true,
                }];
            }
            mod_api::Flow::Continue
        }

        fn entity_join_level(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityJoinLevelEvent<'_>,
        ) -> mod_api::Flow {
            if self.move_player && event.kind == SILVERFISH {
                let player = host.entity(RESUMPTION_PLAYER).unwrap();
                assert!(player.teleport_to(host, MOVED_PLAYER, None));
            }
            mod_api::Flow::Continue
        }
    }

    #[test]
    #[ignore = "daily"]
    fn resuming_the_natural_spawn_pass_keeps_the_constructor_streams() {
        use crate::game::native_entity_refs::tests::{
            bind_local_player, run_fresh_cases, with_game,
        };
        if run_fresh_cases(
            &format!(
                "{}::resuming_the_natural_spawn_pass_keeps_the_constructor_streams",
                module_path!().split_once("::").unwrap().1
            ),
            "SUNRISE_TEST_SEEDS",
            &["1"],
        ) {
            return;
        }
        // Independent Java LCG calculation from NaturalSpawner.getRandomPosWithin and
        // spawnCategoryForPosition: seed 1099 starts at (42, 64, 38). This pack picks
        // one weighted entry and its constant count. Every silverfish passes the
        // stone-floor predicate without drawing, then consumes one level yaw and
        // Mob.finalizeSpawn's two doubles and one float. The per-mob states after
        // construction are the wrapping uniquifier steps from 12345 (nextLong
        // twice for Entity.<init>'s UUID and nextFloat for LivingEntity.<init>'s yaw).
        const EXPECTED: [([f64; 3], u32, u64); 4] = [
            ([44.5, 64.0, 37.5], 0x42bb_ed1c, 0xb901_cbf7_095f),
            ([48.5, 64.0, 39.5], 0x4397_91e0, 0x1965_53db_d76b),
            ([45.5, 64.0, 40.5], 0x430f_0a9a, 0xca79_aeb8_d527),
            ([45.5, 64.0, 38.5], 0x42cd_912f, 0x287f_9289_3a33),
        ];
        for move_player in [false, true] {
            let mods = vec![mod_api::ModEntry::new(
                "silverfish-pack",
                Box::new(SilverfishPack { move_player }),
            )];
            mod_platform::testing::with_mods(mods.into(), || {
                with_game(|game| {
                    let player = [0.5, 64.0, 0.5];
                    spawn_check_world(game, Vec3::new(player[0], player[1], player[2]));
                    bind_local_player(game, RESUMPTION_PLAYER, 10);
                    let stone = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    for x in 24..64 {
                        for z in 24..64 {
                            game.world.set(x, 63, z, stone).unwrap();
                        }
                    }
                    let players = vec![SpawnPlayer {
                        position: player,
                        spectator: false,
                    }];
                    let mut state =
                        SpawnState::new(&game.natural_level(), 289, players, &[], |_, _| true);
                    let mut scratch = NaturalPassScratch::default();
                    scratch.players.refresh(game);
                    let mut random = LevelRandom::for_level(LegacyRandom::new(1099));
                    sunrise_level::random_source::set_thread_uniquifier(12345).unwrap();
                    game.run_natural_spawn_pass(
                        &[Category::Monster],
                        std::iter::once((2, 2)),
                        &mut random,
                        &mut state,
                        &mut scratch,
                    );
                    let actual: Vec<_> = game
                        .mobs
                        .iter()
                        .filter(|mob| mob.kind.id == SILVERFISH)
                        .map(|mob| {
                            (
                                [mob.position.x, mob.position.y, mob.position.z],
                                mob.yaw.to_bits(),
                                mob.random.state(),
                            )
                        })
                        .collect();
                    if move_player {
                        let pos = game.player.as_ref().unwrap().position;
                        assert_eq!([pos.x, pos.y, pos.z], MOVED_PLAYER);
                        // The join moved the player before the next search: every remaining
                        // attempt is within 24 blocks and only consumes the offsets.
                        assert_eq!(actual, EXPECTED[..1]);
                        assert_eq!(random.next_int(), 260_227_472);
                    } else {
                        assert_eq!(actual, EXPECTED);
                        assert_eq!(random.next_int(), 273_362_771);
                    }
                })
            });
        }
    }
}
