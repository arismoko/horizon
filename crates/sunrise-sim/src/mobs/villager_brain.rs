//! The villager's `Brain` (`Villager.BRAIN_PROVIDER` and
//! `VillagerGoalPackages`), ported behaviour for behaviour: memories that
//! expire, the nine sensors and their scan rates, activities from the
//! `villager_activity` timeline, and the behaviours of each activity, started
//! and ticked in vanilla's order (priority, then the activity's slot in
//! `Brain.availableBehaviorsByPriority`'s hash map, then insertion order).
//!
//! The level random (`ServerLevel.getRandom`) times behaviours and POI scans;
//! the mob's own random drives positions, sounds and jumps. `RunOne` and the
//! other `GateBehavior`s shuffle with a random of their own, seeded with
//! `Brain::shuffle_seed` when the brain is built.
mod reputation;
pub use reputation::murder_witnesses;
use reputation::targetable;

use super::{
    Mob, MobEvent, MobWorld, Species, Who,
    brain::{Timed, living_near},
    path::{Path, PathNode},
};
use crate::health::DamageLevel;
use sunrise_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource},
};

/// `MemorySlot.NEVER_EXPIRE`.
pub(super) const NEVER: i64 = i64::MAX;

/// `MemorySlot`: a value and its time to live.
#[derive(Clone, Debug)]
pub struct Slot<T> {
    v: Option<T>,
    pub(super) ttl: i64,
}
impl<T> Default for Slot<T> {
    fn default() -> Self {
        Self {
            v: None,
            ttl: NEVER,
        }
    }
}
impl<T> Slot<T> {
    /// `MemorySlot.tick` (`Brain.forgetOutdatedMemories`).
    pub(super) fn tick(&mut self) {
        if self.v.is_some() && self.ttl != NEVER {
            if self.ttl <= 0 {
                self.clear();
            } else {
                self.ttl -= 1;
            }
        }
    }
    pub fn set(&mut self, v: T) {
        self.v = Some(v);
        self.ttl = NEVER;
    }
    pub fn set_ttl(&mut self, v: T, ttl: i64) {
        self.v = Some(v);
        self.ttl = ttl;
    }
    pub fn set_opt(&mut self, v: Option<T>) {
        match v {
            Some(v) => self.set(v),
            None => self.clear(),
        }
    }
    pub fn clear(&mut self) {
        self.v = None;
        self.ttl = NEVER;
    }
    pub fn get(&self) -> Option<&T> {
        self.v.as_ref()
    }
    pub fn get_mut(&mut self) -> Option<&mut T> {
        self.v.as_mut()
    }
    pub fn present(&self) -> bool {
        self.v.is_some()
    }
    /// The time to live (`i64::MAX`: never expires).
    pub fn ttl(&self) -> i64 {
        self.ttl
    }
}
impl<T: Clone> Slot<T> {
    pub fn value(&self) -> Option<T> {
        self.v.clone()
    }
}
impl<T> Slot<Vec<T>> {
    /// `Brain.setMemory` of a collection: an empty one erases the memory.
    pub(super) fn set_list(&mut self, v: Vec<T>) {
        if v.is_empty() {
            self.clear();
        } else {
            self.set(v);
        }
    }
}

include!("villager_brain/memories.rs");

include!("villager_brain/schedules.rs");

const FOLLOW_RANGE: f64 = 16.0;
/// `NearestLivingEntitySensor`: everything alive within the follow range,
/// nearest first.
fn sense_living(m: &mut Mob, w: &MobWorld) {
    let list = living_near(m, w);
    let mm = mem(m);
    mm.mobs.set_list(list.clone());
    mm.visible_mobs.set(Visible {
        list,
        cache: Vec::new(),
    });
}
/// The visible-entities memory's line-of-sight test, cached per memory.
fn visible_test(m: &mut Mob, w: &MobWorld, who: Who) -> bool {
    if let Some(v) = mem_ref(m).and_then(|mm| mm.visible_mobs.get()) {
        if let Some(&(_, r)) = v.cache.iter().find(|(k, _)| *k == who) {
            return r;
        }
    } else {
        return false;
    }
    let r = targetable(m, w, who);
    if let Some(v) = mem(m).visible_mobs.v.as_mut() {
        v.cache.push((who, r));
    }
    r
}
/// `NearestVisibleLivingEntities.findClosest`.
fn find_closest(m: &mut Mob, w: &MobWorld, filter: impl Fn(&Mob, Who) -> bool) -> Option<Who> {
    let list = mem_ref(m)?.visible_mobs.get()?.list.clone();
    list.into_iter()
        .find(|&who| filter(m, who) && visible_test(m, w, who))
}
/// `NearestVisibleLivingEntities.contains(entity)`.
fn visible_contains(m: &mut Mob, w: &MobWorld, who: Who) -> bool {
    let Some(list) = mem_ref(m)
        .and_then(|mm| mm.visible_mobs.get())
        .map(|v| v.list.contains(&who))
    else {
        return false;
    };
    list && visible_test(m, w, who)
}
/// `PlayerSensor`.
fn sense_players(m: &mut Mob, w: &MobWorld) {
    let range = m.attrs.follow_range;
    let near = w
        .selected_player()
        .is_some_and(|p| !p.spectator && m.distance_sqr(p.position) < range * range);
    let players: Vec<Who> = if near { vec![Who::Player] } else { vec![] };
    let visible: Vec<Who> = players
        .iter()
        .copied()
        .filter(|&p| targetable(m, w, p))
        .collect();
    let attackable: Vec<Who> = visible
        .iter()
        .copied()
        .filter(|_| m.can_target_player(w, FOLLOW_RANGE, true))
        .collect();
    let mm = mem(m);
    mm.nearest_players.set_list(players);
    mm.nearest_visible_player.set_opt(visible.first().copied());
    mm.nearest_visible_targetable_players
        .set_list(attackable.clone());
    mm.nearest_visible_targetable_player
        .set_opt(attackable.first().copied());
}
/// `NearestBedSensor`: babies remember the nearest reachable bed, trying
/// at most four beds a scan and skipping the ones tried lately.
fn sense_bed(m: &mut Mob, w: &MobWorld, lr: &mut LegacyRandom) {
    if !m.is_baby() {
        return;
    }
    let last_update = w.game_time + i64::from(lr.next_int_bound(20).unwrap_or(0));
    let homes = homes_in_range(m, w, 48);
    let mut cache = std::mem::take(&mut brain_mut(m).bed_cache);
    let mut tried = 0;
    let mut targets = Vec::new();
    for &p in &homes {
        if cache.iter().any(|(q, _)| *q == p) {
            continue;
        }
        tried += 1;
        if tried >= 5 {
            continue;
        }
        cache.push((p, last_update + 40));
        targets.push(p);
    }
    let path = if targets.is_empty() {
        None
    } else {
        m.create_path_to_any(w, &targets, 1)
    };
    // createPath hands back the current path when it still heads for a
    // target position it was asked: only a bed (a POI) counts.
    if let Some(path) = path.filter(|p| p.reached) {
        if homes.contains(&path.target) {
            mem(m).nearest_bed.set(path.target);
        }
    } else if tried < 5 {
        cache.retain(|(_, until)| *until >= last_update);
    }
    brain_mut(m).bed_cache = cache;
}
/// `HurtBySensor`: the last damage (forgotten 40 ticks after) and who did it.
fn sense_hurt(m: &mut Mob, w: &MobWorld) {
    // LivingEntity.getLastDamageSource: forgotten 40 ticks after the hit.
    let age = i64::from(m.age);
    let last = m
        .last_damage
        .as_ref()
        .filter(|(_, t)| age - t <= 40)
        .map(|(d, t)| (d.attacker, *t));
    let mm = mem(m);
    if let Some((by, t)) = last {
        mm.hurt_by.set(t);
        if let Some(by) = by {
            mm.hurt_by_entity.set(by);
        }
    } else {
        mm.hurt_by.clear();
    }
    if let Some(who) = mm.hurt_by_entity.value()
        && !w.target_info(who).is_some_and(|t| t.alive)
    {
        mm.hurt_by_entity.clear();
    }
}
include!("villager_brain/hostile_sensor.rs");
fn sense_babies(m: &mut Mob, w: &MobWorld) {
    let list = mem_ref(m)
        .and_then(|mm| mm.visible_mobs.get())
        .map(|v| v.list.clone())
        .unwrap_or_default();
    let mut babies = Vec::new();
    for who in list {
        let Who::Mob(id) = who else { continue };
        if w.seen(id)
            .is_some_and(|s| s.species == Species::Villager && s.baby)
            && visible_test(m, w, who)
        {
            babies.push(who);
        }
    }
    mem(m).visible_villager_babies.set_list(babies);
}
#[path = "villager_secondary.rs"]
mod secondary;
use secondary::sense_secondary;
/// `GolemSensor`.
fn sense_golem(m: &mut Mob, w: &MobWorld) {
    let golem = mem_ref(m)
        .and_then(|mm| mm.mobs.get())
        .is_some_and(|list| {
            list.iter().any(|who| {
                matches!(who, Who::Mob(id) if w.seen(*id).is_some_and(|s| s.species == Species::IronGolem))
            })
        });
    if golem {
        mem(m).golem_detected_recently.set_ttl(true, 599);
    }
}

/// `UseBonemeal.tick` after `BoneMealItem.growCrop` grew the crop the
/// villager worked on: the next crop is picked on the next tick, and the next
/// work cycle is 40 ticks away.
pub fn bonemeal_grown(m: &mut Mob, now: i64) {
    for entry in &mut brain_mut(m).list {
        if let B::UseBonemeal {
            next_work, repick, ..
        } = &mut entry.b
        {
            *repick = true;
            *next_work = now + 40;
        }
    }
}

/// `GolemSensor.golemDetected`: a golem was summoned near this villager;
/// for 599 ticks it wants no other.
pub fn golem_detected(m: &mut Mob, now: i64) {
    mem(m).golem_detected_recently.set_ttl(true, 599);
    // What `sync` derives from the memory.
    super::villager::ext_mut(m).golem_until = now + 599 + 1;
}

// ---- Behaviours ---------------------------------------------------------
use sunrise_world::poi::{PoiIdentity as Poi, PoiType};

#[path = "villager_jobs.rs"]
mod profession_jobs;

pub use profession_jobs::poi_at;
use profession_jobs::*;

/// Tickets taken on a POI: by this villager's memories and by others'
/// (counted once per search by [`others_tickets`]).
fn tickets(m: &Mob, others: &crate::fast_hash::FxHashMap<[i32; 3], usize>, p: [i32; 3]) -> usize {
    let mut n = 0;
    if let Some(mm) = mem_ref(m) {
        for s in [
            &mm.job_site,
            &mm.potential_job_site,
            &mm.home,
            &mm.meeting_point,
        ] {
            if s.get() == Some(&p) {
                n += 1;
            }
        }
    }
    n + others.get(&p).copied().unwrap_or(0)
}
/// The tickets the other living villagers' POIs take, by POI.
pub(super) fn others_tickets(
    m: &Mob,
    w: &MobWorld,
) -> crate::fast_hash::FxHashMap<[i32; 3], usize> {
    let mut taken = crate::fast_hash::FxHashMap::default();
    for o in w.others {
        if o.id != m.id && o.alive && o.species == Species::Villager {
            for q in [o.job, o.bed, o.potential_job, o.meeting]
                .into_iter()
                .flatten()
            {
                *taken.entry(q).or_insert(0) += 1;
            }
        }
    }
    taken
}
/// The POIs of chunk `(qx, qz)` from `y0` to `y1` (within the world), in
/// scan order: y, then z, then x. Only the indexed POI blocks of the sections
/// are visited (`PoiManager.getInChunk`).
fn chunk_pois(
    w: &MobWorld,
    (qx, qz): (i32, i32),
    y0: i32,
    y1: i32,
    mut visit: impl FnMut(Poi, [i32; 3]),
) {
    let Some(chunk) = w.world.chunks.get(&(qx, qz)) else {
        return;
    };
    let kind = |state: u32| poi_of_state(w, sunrise_world::registry::BlockStateId(state));
    let (min, mut lists) = (w.world.bottom_y(), Vec::new());
    w.world.pois.column(chunk, min, (y0, y1), kind, &mut lists);
    for (base, pois) in &lists {
        for &(i, poi) in pois.iter() {
            let i = i32::from(i);
            let y = base + (i >> 8);
            if (y0..=y1).contains(&y) {
                visit(poi, [qx * 16 + (i & 15), y, qz * 16 + (i >> 4 & 15)]);
            }
        }
    }
}
/// `PoiManager.getInRange` with `HAS_SPACE`, nearest first (`distSqr` to the
/// block, ties in scan order: chunk by chunk, z then x, and y, z, x within),
/// filtered by `accept`.
fn pois_in_range(
    m: &Mob,
    w: &MobWorld,
    f: PoiFilter,
    radius: i32,
    mut accept: impl FnMut([i32; 3]) -> bool,
) -> Vec<(Poi, [i32; 3])> {
    let c = block_of(m.position);
    let mut found = Vec::new();
    // Every candidate asks how taken it is: count the others' once.
    let mut taken = None;
    let r2 = i64::from(radius) * i64::from(radius);
    let (cx, cz) = (c[0] >> 4, c[2] >> 4);
    let chunks = radius.div_euclid(16) + 1;
    let y0 = (c[1] - radius).max(w.world.bottom_y());
    let y1 = (c[1] + radius).min(w.world.top_y() - 1);
    for qz in cz - chunks..=cz + chunks {
        for qx in cx - chunks..=cx + chunks {
            chunk_pois(w, (qx, qz), y0, y1, |poi, p| {
                if (p[0] - c[0]).abs() > radius
                    || (p[2] - c[2]).abs() > radius
                    || dist_sqr(p, c) > r2
                    || !poi_matches(m, f, poi, w.professions)
                    || tickets(m, taken.get_or_insert_with(|| others_tickets(m, w)), p)
                        >= poi.max_tickets()
                {
                    return;
                }
                if accept(p) {
                    found.push((poi, p));
                }
            });
        }
    }
    found.sort_by_key(|&(_, p)| dist_sqr(p, c));
    found
}
/// `PoiManager.getInRange(poiType -> true, center, radius, IS_OCCUPIED)`: the
/// POIs within `radius` of `center` that some villager holds a ticket on
/// (`taken`, from [`others_tickets`]), in scan order.
pub(super) fn occupied_pois_in_range(
    w: &MobWorld,
    taken: &crate::fast_hash::FxHashMap<[i32; 3], usize>,
    center: [i32; 3],
    radius: i32,
) -> Vec<[i32; 3]> {
    let mut found = Vec::new();
    let r2 = i64::from(radius) * i64::from(radius);
    let (cx, cz) = (center[0] >> 4, center[2] >> 4);
    let chunks = radius.div_euclid(16) + 1;
    let y0 = (center[1] - radius).max(w.world.bottom_y());
    let y1 = (center[1] + radius).min(w.world.top_y() - 1);
    for qz in cz - chunks..=cz + chunks {
        for qx in cx - chunks..=cx + chunks {
            chunk_pois(w, (qx, qz), y0, y1, |_, p| {
                if (p[0] - center[0]).abs() <= radius
                    && (p[2] - center[2]).abs() <= radius
                    && dist_sqr(p, center) <= r2
                    && taken.get(&p).is_some_and(|&n| n > 0)
                {
                    found.push(p);
                }
            });
        }
    }
    found
}
/// `PoiManager.exists(pos, predicate)`.
fn poi_exists(m: &Mob, w: &MobWorld, p: [i32; 3], f: PoiFilter) -> bool {
    poi_at(w, p[0], p[1], p[2]).is_some_and(|poi| poi_matches(m, f, poi, w.professions))
}
/// `BlockPos.closerToCenterThan(position, d)`.
pub(super) fn closer_to_center(p: [i32; 3], at: Vec3, d: f64) -> bool {
    let c = Vec3::new(
        f64::from(p[0]) + 0.5,
        f64::from(p[1]) + 0.5,
        f64::from(p[2]) + 0.5,
    );
    c.distance_squared(at) < d * d
}
fn manhattan(a: [i32; 3], b: [i32; 3]) -> i32 {
    (a[0] - b[0]).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs()
}
/// The village sections: sections holding a taken village POI.
pub(super) fn village_sections(m: &Mob, w: &MobWorld) -> Vec<[i32; 3]> {
    let mut out = Vec::new();
    let mut add = |p: [i32; 3]| {
        let s = [p[0] >> 4, p[1] >> 4, p[2] >> 4];
        if !out.contains(&s) {
            out.push(s);
        }
    };
    if let Some(mm) = mem_ref(m) {
        for s in [
            &mm.job_site,
            &mm.potential_job_site,
            &mm.home,
            &mm.meeting_point,
        ] {
            if let Some(&p) = s.get()
                && poi_at(w, p[0], p[1], p[2]).is_some()
            {
                add(p);
            }
        }
    }
    let mut villager = |o: &super::Seen| {
        if o.alive && o.species == Species::Villager {
            for p in [o.job, o.bed, o.potential_job, o.meeting]
                .into_iter()
                .flatten()
            {
                if poi_at(w, p[0], p[1], p[2]).is_some() {
                    add(p);
                }
            }
        }
    };
    match w.others_villagers {
        Some(slots) => slots.iter().for_each(|&i| villager(&w.others[i as usize])),
        None => w.others.iter().for_each(villager),
    }
    out
}
/// `PoiManager.sectionsToVillage`: the Chebyshev distance in sections to a
/// village section, at most 7.
pub(super) fn sections_to_village(villages: &[[i32; 3]], s: [i32; 3]) -> i32 {
    villages
        .iter()
        .map(|v| {
            (v[0] - s[0])
                .abs()
                .max((v[1] - s[1]).abs())
                .max((v[2] - s[2]).abs())
        })
        .min()
        .unwrap_or(7)
        .min(7)
}

/// A memory a gate or behaviour conditions on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mem {
    VisibleBabies,
    InteractionTarget,
    BreedTarget,
    Home,
}
fn mem_present(m: &Mob, which: Mem) -> bool {
    let Some(mm) = mem_ref(m) else { return false };
    match which {
        Mem::VisibleBabies => mm.visible_villager_babies.present(),
        Mem::InteractionTarget => mm.interaction_target.present(),
        Mem::BreedTarget => mm.breed_target.present(),
        Mem::Home => mm.home.present(),
    }
}
fn mem_erase(m: &mut Mob, which: Mem) {
    let mm = mem(m);
    match which {
        Mem::VisibleBabies => mm.visible_villager_babies.clear(),
        Mem::InteractionTarget => mm.interaction_target.clear(),
        Mem::BreedTarget => mm.breed_target.clear(),
        Mem::Home => mm.home.clear(),
    }
}

/// `GateBehavior`: `RunOne` (shuffled, run one) and the ordered gates.
#[derive(Clone, Debug)]
struct Gate {
    running: bool,
    /// Entry condition: this memory must be absent.
    absent: Option<Mem>,
    /// Erased when the gate stops.
    erase: Option<Mem>,
    shuffled: bool,
    random: LegacyRandom,
    kids: Vec<(B, i32, f64)>,
}
impl Gate {
    fn run_one(seed: i64, kids: Vec<(B, i32)>) -> Self {
        Self {
            running: false,
            absent: None,
            erase: None,
            shuffled: true,
            random: LegacyRandom::new(seed),
            kids: kids.into_iter().map(|(b, w)| (b, w, 0.0)).collect(),
        }
    }
    fn ordered(seed: i64, erase: Mem, kid: B) -> Self {
        Self {
            running: false,
            absent: None,
            erase: Some(erase),
            shuffled: false,
            random: LegacyRandom::new(seed),
            kids: vec![(kid, 1, 0.0)],
        }
    }
    fn try_start(&mut self, m: &mut Mob, cx: &mut Bx) -> bool {
        if self.absent.is_some_and(|a| mem_present(m, a)) {
            return false;
        }
        self.running = true;
        if self.shuffled {
            // ShufflingList.shuffle: a fresh weight for each entry in the
            // current order, then a stable sort.
            for k in &mut self.kids {
                let r = self.random.next_float();
                k.2 = -sunrise_core::jmath::pow(f64::from(r), f64::from(1.0_f32 / k.1 as f32));
            }
            self.kids.sort_by(|a, b| a.2.total_cmp(&b.2));
        }
        for k in &mut self.kids {
            if !k.0.running() && k.0.try_start(m, cx) {
                break;
            }
        }
        true
    }
    fn tick_or_stop(&mut self, m: &mut Mob, cx: &mut Bx) {
        for k in &mut self.kids {
            if k.0.running() {
                k.0.tick_or_stop(m, cx);
            }
        }
        if !self.kids.iter().any(|k| k.0.running()) {
            self.do_stop(m, cx);
        }
    }
    fn do_stop(&mut self, m: &mut Mob, cx: &mut Bx) {
        self.running = false;
        for k in &mut self.kids {
            if k.0.running() {
                k.0.do_stop(m, cx);
            }
        }
        if let Some(e) = self.erase {
            mem_erase(m, e);
        }
    }
}

/// `SetEntityLookTarget`'s predicates.
#[derive(Clone, Copy, Debug, PartialEq)]
enum LookAt {
    Species(Species),
    Player,
    Category(&'static str),
}
/// `InteractWith`'s targets.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Interact {
    Villager,
    BreedVillager,
    Cat,
}
include!("villager_brain/tasks.rs");

#[cfg(test)]
#[path = "villager_farming_tests.rs"]
mod farming_tests;

/// `HarvestFarmland.validFarmlandAroundVillager`: ripe crops and bare
/// farmland around the villager.
fn farmland_around(m: &Mob, w: &MobWorld) -> Vec<[i32; 3]> {
    let mut valid = Vec::new();
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                let p = [
                    (m.position.x + f64::from(x)).floor() as i32,
                    (m.position.y + f64::from(y)).floor() as i32,
                    (m.position.z + f64::from(z)).floor() as i32,
                ];
                if farmland_valid(w, p) {
                    valid.push(p);
                }
            }
        }
    }
    valid
}
/// Farmland subclasses include mod templates.
fn farmland(w: &MobWorld, p: [i32; 3]) -> bool {
    w.facts
        .get(w.world.block(p[0], p[1], p[2]))
        .has_class("FarmlandBlock")
}
/// `HarvestFarmland.validPos`: a ripe CropBlock or air above farmland.
fn farmland_valid(w: &MobWorld, p: [i32; 3]) -> bool {
    super::villager::ripe_crop(w, p)
        || w.facts.is_air(w.world.block(p[0], p[1], p[2])) && farmland(w, [p[0], p[1] - 1, p[2]])
}
/// `HarvestFarmland.getValidFarmland`: one picked by the level random.
fn pick_farmland(valid: &[[i32; 3]], cx: &mut Bx) -> Option<[i32; 3]> {
    if valid.is_empty() {
        return None;
    }
    let i = cx
        .level
        .random()
        .next_int_bound(valid.len() as i32)
        .unwrap_or(0) as usize;
    Some(valid[i])
}
/// A tagged BlockItem plants its block's default state.
fn seed_crop(w: &MobWorld, item: &str) -> Option<sunrise_world::registry::BlockStateId> {
    let items = w.items?;
    if !items.item_in_tag("minecraft:villager_plantable_seeds", item) {
        return None;
    }
    items.get(item)?.block
}
/// `HarvestFarmland.tick` at the farmland: harvest a ripe crop, sow bare
/// farmland, and move on from a growing crop.
fn harvest_tick(
    m: &mut Mob,
    cx: &mut Bx,
    next_ok: &mut i64,
    target: &mut Option<[i32; 3]>,
    valid: &mut Vec<[i32; 3]>,
) {
    let Some(p) = *target else { return };
    if cx.now <= *next_ok {
        return;
    }
    let w = cx.w;
    let ripe = super::villager::ripe_crop(w, p);
    let air = w.facts.is_air(w.world.block(p[0], p[1], p[2]));
    let growing = !ripe && super::villager::crop(w, p);
    if ripe {
        cx.events.push(MobEvent::BreakBlock {
            pos: p,
            drop: true,
            by: None,
            breaker: Some(m.id),
        });
    }
    if air && farmland(w, [p[0], p[1] - 1, p[2]]) {
        let e = super::villager::ext_mut(m);
        if let Some((slot, crop)) = e
            .inventory
            .iter()
            .enumerate()
            .filter(|(_, s)| s.count > 0)
            .find_map(|(i, s)| seed_crop(w, &s.item).map(|crop| (i, crop)))
        {
            e.inventory[slot].count -= 1;
            if e.inventory[slot].count == 0 {
                e.inventory.remove(slot);
            }
            cx.events.push(MobEvent::SetBlock {
                pos: p,
                state: crop,
            });
            cx.events.push(MobEvent::GameEvent {
                event: "minecraft:block_place",
                at: p.map(|v| f64::from(v) + 0.5),
                source: m.id,
                state: crop,
            });
            cx.events.push(MobEvent::Sound(
                "item.crop.plant",
                [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])],
                1.0,
                1.0,
            ));
        }
    }
    if growing {
        valid.retain(|q| *q != p);
        *target = pick_farmland(valid, cx);
        if let Some(q) = *target {
            *next_ok = cx.now + 20;
            let mm = mem(m);
            mm.walk_target.set(WalkTarget::block(q, 0.5, 1));
            mm.look_target.set(Tracker::Block(q));
        }
    }
}
/// `UseBonemeal.validPos`: a `CropBlock` below its max age.
fn bonemeal_valid(w: &MobWorld, p: [i32; 3]) -> bool {
    super::villager::crop(w, p) && !super::villager::ripe_crop(w, p)
}
/// `UseBonemeal.pickNextTarget`: one of the valid blocks around the villager,
/// each equally likely (reservoir sampling with the level random).
fn pick_bonemeal_crop(m: &Mob, cx: &mut Bx) -> Option<[i32; 3]> {
    let c = block_of(m.position);
    let mut result = None;
    let mut count = 0;
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                let p = [c[0] + x, c[1] + y, c[2] + z];
                if bonemeal_valid(cx.w, p) {
                    count += 1;
                    if cx.level.random().next_int_bound(count).unwrap_or(1) == 0 {
                        result = Some(p);
                    }
                }
            }
        }
    }
    result
}
/// `UseBonemeal.setCurrentCropAsTarget`.
fn bonemeal_target(m: &mut Mob, crop: Option<[i32; 3]>) {
    if let Some(p) = crop {
        let mm = mem(m);
        mm.look_target.set(Tracker::Block(p));
        mm.walk_target.set(WalkTarget::block(p, 0.5, 1));
    }
}
/// The remembered POI positions a behaviour reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PoiMem {
    Job,
    Potential,
    Home,
    Meeting,
}
impl PoiMem {
    fn get(self, m: &Mob) -> Option<[i32; 3]> {
        let mm = mem_ref(m)?;
        match self {
            PoiMem::Job => mm.job_site.value(),
            PoiMem::Potential => mm.potential_job_site.value(),
            PoiMem::Home => mm.home.value(),
            PoiMem::Meeting => mm.meeting_point.value(),
        }
    }
    fn clear(self, m: &mut Mob) {
        let mm = mem(m);
        match self {
            PoiMem::Job => mm.job_site.clear(),
            PoiMem::Potential => mm.potential_job_site.clear(),
            PoiMem::Home => mm.home.clear(),
            PoiMem::Meeting => mm.meeting_point.clear(),
        }
    }
}
fn dist_sqr(a: [i32; 3], b: [i32; 3]) -> i64 {
    manhattan_sq(a, b)
}
/// Every bed head within `radius` (`PoiManager.getInRange(HOME, ..., ANY)`),
/// in the order of a scan of the cube: y, then z, then x.
fn homes_in_range(m: &Mob, w: &MobWorld, radius: i32) -> Vec<[i32; 3]> {
    let c = block_of(m.position);
    let r2 = i64::from(radius) * i64::from(radius);
    let mut out = Vec::new();
    let y0 = (c[1] - radius).max(w.world.bottom_y());
    let y1 = (c[1] + radius).min(w.world.top_y() - 1);
    for qz in (c[2] - radius) >> 4..=(c[2] + radius) >> 4 {
        for qx in (c[0] - radius) >> 4..=(c[0] + radius) >> 4 {
            chunk_pois(w, (qx, qz), y0, y1, |poi, p| {
                if poi == Poi::Vanilla(PoiType::Home) && dist_sqr(p, c) <= r2 {
                    out.push(p);
                }
            });
        }
    }
    out.sort_unstable_by_key(|p| (p[1], p[2], p[0]));
    out
}
/// `MoveToSkySeeingSpot.hasNoBlocksAbove`.
fn no_blocks_above(m: &Mob, w: &MobWorld, p: [i32; 3]) -> bool {
    w.can_see_sky(p[0], p[1], p[2])
        && f64::from(w.motion_blocking_height(p[0], p[2])) <= m.position.y
}
/// `VillagerMakeLove.isBreedingPossible`.
fn breeding_possible(m: &mut Mob, w: &MobWorld) -> bool {
    let Some(Who::Mob(id)) = mem_ref(m).and_then(|mm| mm.breed_target.value()) else {
        return false;
    };
    villager_target_valid(m, w, Mem::BreedTarget)
        && super::villager::can_breed_in(m, w)
        && w.seen(id).is_some_and(|s| s.in_love)
}
/// `VillagerMakeLove.takeVacantBed`: `PoiManager.take(HOME, ..., 48)` over a
/// sphere of 48 blocks, the nearest bed head no villager has claimed that the
/// villager can walk to (`canReach`: a path that gets there).
fn take_bed_for_child(m: &mut Mob, w: &MobWorld) -> Option<[i32; 3]> {
    let claimed: Vec<[i32; 3]> = w
        .others
        .iter()
        .filter(|s| s.species == Species::Villager && s.id != m.id)
        .filter_map(|s| s.bed)
        .chain(m.extra.bed)
        .collect();
    let from = block_of(m.position);
    let mut beds: Vec<[i32; 3]> = homes_in_range(m, w, 48)
        .into_iter()
        .filter(|p| {
            w.prop(p[0], p[1], p[2], "part") == Some("head")
                && w.prop(p[0], p[1], p[2], "occupied") != Some("true")
                && !claimed.contains(p)
        })
        .collect();
    // Nearest first; equally near beds stay in scan order.
    beds.sort_by_key(|p| dist_sqr(*p, from));
    beds.into_iter().find(|&p| {
        m.create_path_to(w, p, (Poi::Vanilla(PoiType::Home)).valid_range())
            .is_some_and(|path| path.reached)
    })
}
/// `BehaviorUtils.lockGazeAndWalkToEachOther`: this villager's half, and
/// the other villager's through a `LockGaze` event.
fn lock_gaze(m: &mut Mob, target: Who, cx: &mut Bx) {
    lock_gaze_half(m, target);
    if let Who::Mob(other) = target {
        cx.events.push(MobEvent::LockGaze {
            mob: other,
            partner: m.id,
        });
    }
}
/// One villager's half of `lockGazeAndWalkToEachOther`: look at the other
/// and walk to it.
pub fn lock_gaze_half(m: &mut Mob, target: Who) {
    if super::villager::ext(m).is_none_or(|e| e.brain.is_none()) {
        return;
    }
    let mm = mem(m);
    mm.look_target.set(Tracker::Entity(target, true));
    mm.walk_target.set(WalkTarget {
        target: Tracker::Entity(target, true),
        speed: 0.5,
        close: 2,
    });
}
/// `SetWalkTargetFromBlockMemory`.
fn walk_to_block_memory(
    m: &mut Mob,
    cx: &mut Bx,
    which: PoiMem,
    speed: f32,
    close: i32,
    too_far: i32,
    too_long: i64,
) -> bool {
    if mem(m).walk_target.present() {
        return false;
    }
    let Some(target) = which.get(m) else {
        return false;
    };
    let since = mem(m).cant_reach_walk_target_since.value();
    let here = block_of(m.position);
    if since.is_none_or(|s| cx.w.game_time - s <= too_long) {
        if manhattan(target, here) > too_far {
            let to = [
                f64::from(target[0]) + 0.5,
                f64::from(target[1]),
                f64::from(target[2]) + 0.5,
            ];
            let mut tries = 0;
            loop {
                let step = m.default_random_pos_towards(
                    cx.w,
                    15,
                    7,
                    to,
                    f64::from(std::f32::consts::FRAC_PI_2),
                );
                if let Some(s) = step {
                    let b = [
                        s[0].floor() as i32,
                        s[1].floor() as i32,
                        s[2].floor() as i32,
                    ];
                    if manhattan(b, block_of(m.position)) <= too_far {
                        mem(m).walk_target.set(WalkTarget::point(s, speed, close));
                        break;
                    }
                }
                tries += 1;
                if tries == 1000 {
                    which.clear(m);
                    mem(m).cant_reach_walk_target_since.set(cx.now);
                    return true;
                }
            }
        } else if manhattan(target, here) > close {
            mem(m)
                .walk_target
                .set(WalkTarget::block(target, speed, close));
        }
    } else {
        which.clear(m);
        mem(m).cant_reach_walk_target_since.set(cx.now);
    }
    true
}
/// `InteractWithDoor`: open the doors on the path, close the ones passed.
fn interact_with_doors(m: &mut Mob, cx: &mut Bx, from: PathNode, to: PathNode) {
    let (from, to) = ([from.x, from.y, from.z], [to.x, to.y, to.z]);
    // The door being left is remembered even when it is open already; the
    // one ahead only when this villager opens it.
    for (p, always) in [(from, true), (to, false)] {
        let Some(open) = door_open(cx.w, p) else {
            continue;
        };
        if !open {
            cx.events.push(MobEvent::SetDoorOpen { pos: p, open: true });
        }
        if always || !open {
            remember_door(m, p);
        }
    }
    close_doors(m, cx, Some(from), Some(to));
}
/// Whether the block at `p` is a door mobs open (`MOB_INTERACTABLE_DOORS`), and
/// then whether it is open.
fn door_open(w: &MobWorld, p: [i32; 3]) -> Option<bool> {
    let name = w.block_name(p[0], p[1], p[2]);
    (name.ends_with("_door") && w.has_tag("minecraft:mob_interactable_doors", name))
        .then(|| w.prop(p[0], p[1], p[2], "open") == Some("true"))
}
fn remember_door(m: &mut Mob, p: [i32; 3]) {
    let doors = &mut mem(m).doors_to_close;
    match doors.get_mut() {
        Some(list) if list.contains(&p) => {}
        Some(list) => list.push(p),
        None => doors.set(vec![p]),
    }
}
/// `InteractWithDoor.closeDoorsThatIHaveOpenedOrPassedThrough`: the remembered
/// doors other than the two of the step being taken are forgotten, and closed
/// when still open, near (3 blocks) and no other villager within 2 blocks of
/// one is passing through it.
fn close_doors(m: &mut Mob, cx: &mut Bx, from: Option<[i32; 3]>, to: Option<[i32; 3]>) {
    let Some(doors) = mem(m).doors_to_close.get().cloned() else {
        return;
    };
    let near = mem(m).mobs.value().unwrap_or_default();
    let mut kept = Vec::new();
    for door in doors {
        if from == Some(door) || to == Some(door) {
            kept.push(door);
            continue;
        }
        if !closer_to_center(door, m.position, 3.0) || door_open(cx.w, door) != Some(true) {
            continue;
        }
        let passing = near.iter().any(|&who| {
            let Who::Mob(id) = who else { return false };
            cx.w.seen(id).is_some_and(|s| {
                s.species == Species::Villager
                    && closer_to_center(door, s.position, 2.0)
                    && s.villager
                        .as_ref()
                        .and_then(|v| v.path_nodes)
                        .is_some_and(|(prev, next)| prev.is_some_and(|p| p == door || next == door))
            })
        });
        if !passing {
            cx.events.push(MobEvent::SetDoorOpen {
                pos: door,
                open: false,
            });
        }
    }
    mem(m).doors_to_close.set(kept);
}
/// The scan order of POI candidates (before the nearest-first sort).
fn pois_scan_order(_m: &Mob, found: &[(Poi, [i32; 3])]) -> Vec<usize> {
    (0..found.len()).collect()
}
/// Entity event 14: the client shows happy particles (with its own random,
/// not the mob's).
/// Entity event 14 (`AcquirePoi`, `AssignProfessionFromJobSite`):
/// `AbstractVillager.addParticlesAroundSelf(HAPPY_VILLAGER)` on the client.
fn happy_particles(m: &Mob, cx: &mut Bx) {
    cx.events.push(MobEvent::AroundSelf {
        mob: m.id,
        event: 14,
        kind: "happy_villager",
        feet: [m.position.x, m.position.y, m.position.z],
        width: f64::from(m.width()),
        height: f64::from(m.height()),
        burst: sunrise_level::particle_options::Burst::VILLAGER,
    });
}
/// `BehaviorUtils.findSectionClosestToVillage`.
pub(super) fn closest_section(villages: &[[i32; 3]], center: [i32; 3], radius: i32) -> [i32; 3] {
    let here = sections_to_village(villages, center);
    let mut best: Option<([i32; 3], i32)> = None;
    for z in center[2] - radius..=center[2] + radius {
        for y in center[1] - radius..=center[1] + radius {
            for x in center[0] - radius..=center[0] + radius {
                let s = [x, y, z];
                let d = sections_to_village(villages, s);
                if d < here && best.is_none_or(|(_, b)| d < b) {
                    best = Some((s, d));
                }
            }
        }
    }
    best.map_or(center, |(s, _)| s)
}

// ---- Packages -----------------------------------------------------------

fn timed(min: i32, max: i32) -> Timed {
    Timed::new(min, max)
}
fn run_one(seed: i64, kids: Vec<(B, i32)>) -> B {
    B::Gate(Box::new(Gate::run_one(seed, kids)))
}
fn look_at(what: LookAt) -> B {
    B::LookAtEntity {
        running: false,
        what,
    }
}
/// `getMinimalLookBehavior`.
fn look_minimal(seed: i64) -> B {
    run_one(
        seed,
        vec![
            (look_at(LookAt::Species(Species::Villager)), 2),
            (look_at(LookAt::Player), 2),
            (B::DoNothing(timed(30, 60)), 8),
        ],
    )
}
/// `getFullLookBehavior`.
fn look_full(seed: i64) -> B {
    run_one(
        seed,
        vec![
            (look_at(LookAt::Species(Species::Cat)), 8),
            (look_at(LookAt::Species(Species::Villager)), 2),
            (look_at(LookAt::Player), 2),
            (look_at(LookAt::Category("creature")), 1),
            (look_at(LookAt::Category("water_creature")), 1),
            (look_at(LookAt::Category("axolotls")), 1),
            (look_at(LookAt::Category("underground_water_creature")), 1),
            (look_at(LookAt::Category("water_ambient")), 1),
            (look_at(LookAt::Category("monster")), 1),
            (B::DoNothing(timed(30, 60)), 2),
        ],
    )
}
fn stroll(speed: f32, xz: i32, y: i32) -> B {
    B::VillageBoundStroll {
        running: false,
        speed,
        xz,
        y,
    }
}
fn walk_to(which: PoiMem, speed: f32, close: i32, too_far: i32, too_long: i64) -> B {
    B::WalkToBlockMemory {
        running: false,
        which,
        speed,
        close,
        too_far,
        too_long,
    }
}
fn trigger_gate(seed: i64, kids: Vec<(B, i32)>) -> B {
    B::TriggerGate {
        running: false,
        random: LegacyRandom::new(seed),
        kids: kids.into_iter().map(|(b, w)| (b, w, 0.0)).collect(),
    }
}
fn gift() -> B {
    B::GiveGiftToHero {
        t: timed(100, 100),
        until_next: 600,
        given: false,
        started: 0,
    }
}
fn show_trades() -> B {
    B::ShowTrades {
        t: timed(400, 1600),
        look_time: 0,
        player_item: None,
        items: Vec::new(),
        cycle: 0,
        index: 0,
    }
}
fn interact(kind: Interact) -> B {
    B::InteractWith {
        running: false,
        kind,
    }
}
fn trade_gate(seed: i64) -> B {
    B::Gate(Box::new(Gate::ordered(
        seed,
        Mem::InteractionTarget,
        B::TradeWithVillager(timed(60, 60)),
    )))
}
fn jump_on_bed() -> B {
    B::JumpOnBed {
        t: timed(60, 60),
        bed: None,
        time_left: 0,
        jumps: 0,
        cooldown: 0,
    }
}
fn move_to_target_sink(min: i32, max: i32) -> B {
    B::MoveToTargetSink {
        t: timed(min, max),
        cooldown: 0,
        path: None,
        last_target: None,
        speed: 0.0,
    }
}
fn acquire(filter: PoiFilter, poi: PoiFilter, only_adult: bool, event: bool) -> B {
    B::AcquirePoi {
        running: false,
        filter,
        poi,
        only_adult,
        event,
        next: 0,
        retries: Vec::new(),
    }
}
fn validate(filter: PoiFilter, which: PoiMem) -> B {
    B::ValidatePoi {
        running: false,
        filter,
        which,
    }
}
/// `VillagerGoalPackages`: the behaviours of every activity, in iteration
/// order. `speedModifier` is 0.5 throughout.
fn packages(baby: bool, seed: i64, farmer: bool) -> Vec<Entry> {
    let mut v: Vec<Entry> = Vec::new();
    let mut add = |prio: i32, act: Act, b: B| v.push(Entry { prio, act, b });
    if baby {
        // PLAY
        let p = Act::Play;
        add(0, p, move_to_target_sink(80, 120));
        add(5, p, look_full(seed));
        add(5, p, B::PlayTag(false));
        let mut play = Gate::run_one(
            seed,
            vec![
                (interact(Interact::Villager), 2),
                (interact(Interact::Cat), 1),
                (stroll(0.5, 10, 7), 1),
                (B::WalkToLookTarget(false), 1),
                (jump_on_bed(), 2),
                (B::DoNothing(timed(20, 40)), 2),
            ],
        );
        play.absent = Some(Mem::VisibleBabies);
        add(5, p, B::Gate(Box::new(play)));
        add(99, p, B::UpdateActivity(false));
    } else {
        // WORK
        let w = Act::Work;
        add(5, w, look_minimal(seed));
        add(
            5,
            w,
            run_one(
                seed,
                vec![
                    (
                        B::WorkAtPoi {
                            t: timed(60, 60),
                            last_check: 0,
                            composter: farmer,
                        },
                        7,
                    ),
                    (
                        B::StrollAroundPoi {
                            running: false,
                            next_ok: 0,
                            max_dist: 4,
                            which: PoiMem::Job,
                        },
                        2,
                    ),
                    (
                        B::StrollToPoi {
                            running: false,
                            next_ok: 0,
                        },
                        5,
                    ),
                    (
                        B::StrollToPoiList {
                            running: false,
                            next_ok: 0,
                        },
                        5,
                    ),
                    (
                        B::HarvestFarmland {
                            t: timed(60, 60),
                            next_ok: 0,
                            target: None,
                            valid: Vec::new(),
                            worked: 0,
                        },
                        if farmer { 2 } else { 5 },
                    ),
                    (
                        B::UseBonemeal {
                            t: timed(60, 60),
                            next_work: 0,
                            last_session: 0,
                            worked: 0,
                            crop: None,
                            repick: false,
                        },
                        if farmer { 4 } else { 7 },
                    ),
                ],
            ),
        );
        add(10, w, show_trades());
        add(10, w, B::SetLookAndInteract(false));
        add(2, w, walk_to(PoiMem::Job, 0.5, 9, 100, 1200));
        add(3, w, gift());
        add(99, w, B::UpdateActivity(false));
    }
    // CORE
    let c = Act::Core;
    add(0, c, B::Swim(timed(60, 60)));
    add(
        0,
        c,
        B::InteractWithDoor {
            running: false,
            last_node: None,
            cooldown: 0,
        },
    );
    add(0, c, B::LookAtTargetSink(timed(45, 90)));
    add(0, c, B::PanicTrigger(timed(60, 60)));
    add(0, c, B::WakeUp(false));
    add(0, c, B::ReactToBell(false));
    add(0, c, B::SetRaidStatus(false));
    add(0, c, validate(PoiFilter::Held, PoiMem::Job));
    add(0, c, validate(PoiFilter::Acquirable, PoiMem::Potential));
    add(1, c, move_to_target_sink(150, 250));
    add(2, c, B::PoiCompetitorScan(false));
    add(3, c, B::FollowTradingPlayer(timed(i32::MAX, i32::MAX)));
    add(5, c, B::GoToWantedItem(false));
    add(
        6,
        c,
        acquire(PoiFilter::Acquirable, PoiFilter::Acquirable, true, false),
    );
    add(7, c, B::GoToPotentialJobSite(timed(1200, 1200)));
    add(8, c, B::YieldJobSite(false));
    add(
        10,
        c,
        acquire(PoiFilter::Home, PoiFilter::Home, false, true),
    );
    add(
        10,
        c,
        acquire(PoiFilter::Meeting, PoiFilter::Meeting, true, true),
    );
    add(10, c, B::AssignProfession(false));
    add(10, c, B::ResetProfession(false));
    // MEET
    let me = Act::Meet;
    add(
        2,
        me,
        trigger_gate(
            seed,
            vec![
                (
                    B::StrollAroundPoi {
                        running: false,
                        next_ok: 0,
                        max_dist: 40,
                        which: PoiMem::Meeting,
                    },
                    2,
                ),
                (B::SocializeAtBell(false), 2),
            ],
        ),
    );
    add(10, me, show_trades());
    add(10, me, B::SetLookAndInteract(false));
    add(2, me, walk_to(PoiMem::Meeting, 0.5, 6, 100, 200));
    add(3, me, gift());
    add(3, me, validate(PoiFilter::Meeting, PoiMem::Meeting));
    add(3, me, trade_gate(seed));
    add(5, me, look_full(seed));
    add(99, me, B::UpdateActivity(false));
    // REST
    let r = Act::Rest;
    add(2, r, walk_to(PoiMem::Home, 0.5, 1, 150, 1200));
    add(3, r, validate(PoiFilter::Home, PoiMem::Home));
    add(
        3,
        r,
        B::SleepInBed {
            t: timed(60, 60),
            next_ok: 0,
        },
    );
    let mut homeless = Gate::run_one(
        seed,
        vec![
            (
                B::ClosestHomeAsWalkTarget {
                    running: false,
                    last_update: 0,
                    cache: Vec::new(),
                },
                1,
            ),
            (
                B::InsideBrownianWalk {
                    running: false,
                    random: LegacyRandom::new(seed),
                },
                4,
            ),
            (B::GoToClosestVillage(false), 2),
            (B::DoNothing(timed(20, 40)), 2),
        ],
    );
    homeless.absent = Some(Mem::Home);
    add(5, r, B::Gate(Box::new(homeless)));
    add(5, r, look_minimal(seed));
    add(99, r, B::UpdateActivity(false));
    // IDLE
    let i = Act::Idle;
    add(
        2,
        i,
        run_one(
            seed,
            vec![
                (interact(Interact::Villager), 2),
                (interact(Interact::BreedVillager), 1),
                (interact(Interact::Cat), 1),
                (stroll(0.5, 10, 7), 1),
                (B::WalkToLookTarget(false), 1),
                (jump_on_bed(), 1),
                (B::DoNothing(timed(30, 60)), 1),
            ],
        ),
    );
    add(3, i, gift());
    add(3, i, B::SetLookAndInteract(false));
    add(3, i, show_trades());
    add(3, i, trade_gate(seed));
    add(
        3,
        i,
        B::Gate(Box::new(Gate::ordered(
            seed,
            Mem::BreedTarget,
            B::MakeLove {
                t: timed(350, 350),
                birth: 0,
            },
        ))),
    );
    add(5, i, look_full(seed));
    add(99, i, B::UpdateActivity(false));
    // PANIC
    let pa = Act::Panic;
    add(0, pa, B::CalmDown(false));
    add(
        1,
        pa,
        B::WalkAwayFrom {
            running: false,
            hostile: true,
        },
    );
    add(
        1,
        pa,
        B::WalkAwayFrom {
            running: false,
            hostile: false,
        },
    );
    add(3, pa, stroll(0.75, 2, 2));
    add(5, pa, look_minimal(seed));
    // PRE_RAID
    let pr = Act::PreRaid;
    add(0, pr, B::RingBell(false));
    add(
        0,
        pr,
        trigger_gate(
            seed,
            vec![
                (walk_to(PoiMem::Meeting, 0.75, 2, 150, 200), 6),
                (stroll(0.75, 10, 7), 2),
            ],
        ),
    );
    add(5, pr, look_minimal(seed));
    add(99, pr, B::ResetRaidStatus(false));
    // RAID
    let ra = Act::Raid;
    add(
        0,
        ra,
        B::RaidSequence {
            running: false,
            victory: true,
            then: Box::new(trigger_gate(
                seed,
                vec![
                    (
                        B::MoveToSkySeeingSpot {
                            running: false,
                            speed: 0.5,
                        },
                        5,
                    ),
                    (stroll(0.55, 10, 7), 2),
                ],
            )),
        },
    );
    add(0, ra, B::Celebrate(timed(600, 600)));
    add(
        2,
        ra,
        B::RaidSequence {
            running: false,
            victory: false,
            then: Box::new(B::LocateHidingPlace {
                running: false,
                radius: 24,
                speed: 0.7,
                close: 1,
            }),
        },
    );
    add(5, ra, look_minimal(seed));
    add(99, ra, B::ResetRaidStatus(false));
    // HIDE
    let h = Act::Hide;
    add(
        0,
        h,
        B::SetHiddenState {
            running: false,
            hidden: 0,
        },
    );
    add(
        1,
        h,
        B::LocateHidingPlace {
            running: false,
            radius: 32,
            speed: 0.625,
            close: 2,
        },
    );
    add(5, h, look_minimal(seed));
    v.sort_by_key(|e| (e.prio, e.act.slot()));
    v
}

// ---- Mod behaviors ------------------------------------------------------

/// Validate that this brain keeps a slot for `activity`.
pub(super) fn validate_activity(
    activity: mod_api::Activity,
) -> Result<(), mod_api::BehaviorRefusal> {
    act_of(activity).map(|_| ())
}

/// Apply staged mod behaviors to the villager brain if it is still the same
/// `generation` instance; otherwise discard them safely and return `false`.
pub(super) fn apply_staged(
    m: &mut Mob,
    generation: u64,
    staged: Vec<mod_api::RegisteredBehavior>,
) -> bool {
    let Some(b) = super::villager::ext_mut(m).brain.as_deref_mut() else {
        super::mod_behaviors::discard_registrations(staged);
        return false;
    };
    if b.generation != generation {
        super::mod_behaviors::discard_registrations(staged);
        return false;
    }
    let before = b.list.len();
    for r in staged {
        let act = act_of(r.activity).expect("staged activity was validated on add");
        b.list.push(Entry {
            prio: r.priority,
            act,
            b: B::Mod(super::mod_behaviors::ModBehavior::new(r)),
        });
    }
    if b.list.len() > before {
        b.list.sort_by_key(|e| (e.prio, e.act.slot()));
    }
    true
}

/// The villager brain's slot for a mod-api activity.
fn act_of(activity: mod_api::Activity) -> Result<Act, mod_api::BehaviorRefusal> {
    use mod_api::Activity as A;
    Ok(match activity {
        A::Core => Act::Core,
        A::Idle => Act::Idle,
        A::Work => Act::Work,
        A::Play => Act::Play,
        A::Rest => Act::Rest,
        A::Meet => Act::Meet,
        A::Panic => Act::Panic,
        A::Raid => Act::Raid,
        A::PreRaid => Act::PreRaid,
        A::Hide => Act::Hide,
        other => return Err(mod_api::BehaviorRefusal::UnknownActivity(other)),
    })
}

/// The list's `(prio, activity, is a mod's)` keys, in `Brain.tick`'s start
/// order — the inspection the goal selector's `entries` gives goals' tests.
pub fn entry_keys(m: &Mob) -> Option<Vec<(i32, Act, bool)>> {
    brain(m).map(|b| {
        b.list
            .iter()
            .map(|e| (e.prio, e.act, matches!(e.b, B::Mod(_))))
            .collect()
    })
}

// ---- The probe's view ---------------------------------------------------

fn who(w: Who) -> String {
    match w {
        Who::Player => "player".into(),
        Who::Mob(id) => format!("e{id}"),
    }
}
fn pos(p: [i32; 3]) -> String {
    format!("{},{},{}", p[0], p[1], p[2])
}
fn tracker(t: Tracker) -> String {
    match t {
        Tracker::Entity(e, _) => who(e),
        Tracker::Block(p) => pos(p),
        Tracker::Point(v) => pos([
            v[0].floor() as i32,
            v[1].floor() as i32,
            v[2].floor() as i32,
        ]),
    }
}
/// The active activities and the memories with values, as the oracle
/// prints them (`act core|idle`, `mem name=value...` sorted).
pub fn describe(m: &Mob) -> Option<(String, String)> {
    let b = brain(m)?;
    let mut acts = ["core", b.activity.name()];
    acts.sort_unstable();
    let mm = &b.mem;
    let mut out: Vec<String> = Vec::new();
    let mut put = |name: &str, value: String, ttl: i64| {
        if ttl == NEVER {
            out.push(format!("{name}={value}"));
        } else {
            out.push(format!("{name}={value}@{ttl}"));
        }
    };
    macro_rules! slot {
        ($name:expr, $s:expr, $f:expr) => {
            if let Some(v) = $s.get() {
                put($name, $f(v), $s.ttl);
            }
        };
    }
    slot!("home", mm.home, |p: &[i32; 3]| pos(*p));
    slot!("job_site", mm.job_site, |p: &[i32; 3]| pos(*p));
    slot!(
        "potential_job_site",
        mm.potential_job_site,
        |p: &[i32; 3]| pos(*p)
    );
    slot!("meeting_point", mm.meeting_point, |p: &[i32; 3]| pos(*p));
    slot!("secondary_job_site", mm.secondary_job_site, |l: &Vec<
        [i32; 3],
    >| format!(
        "n{}",
        l.len()
    ));
    slot!("mobs", mm.mobs, |l: &Vec<Who>| format!("n{}", l.len()));
    slot!("visible_mobs", mm.visible_mobs, |_: &Visible| {
        "NearestVisibleLivingEntities".to_string()
    });
    slot!(
        "visible_villager_babies",
        mm.visible_villager_babies,
        |l: &Vec<Who>| format!("n{}", l.len())
    );
    slot!(
        "nearest_players",
        mm.nearest_players,
        |l: &Vec<Who>| format!("n{}", l.len())
    );
    slot!(
        "nearest_visible_player",
        mm.nearest_visible_player,
        |e: &Who| who(*e)
    );
    slot!(
        "nearest_visible_targetable_player",
        mm.nearest_visible_targetable_player,
        |e: &Who| who(*e)
    );
    slot!(
        "nearest_visible_targetable_players",
        mm.nearest_visible_targetable_players,
        |l: &Vec<Who>| format!("n{}", l.len())
    );
    slot!("walk_target", mm.walk_target, |t: &WalkTarget| format!(
        "{}/{:x}/{}",
        tracker(t.target),
        t.speed.to_bits(),
        t.close
    ));
    slot!("look_target", mm.look_target, |t: &Tracker| tracker(*t));
    slot!("interaction_target", mm.interaction_target, |e: &Who| who(
        *e
    ));
    slot!("breed_target", mm.breed_target, |e: &Who| who(*e));
    slot!("path", mm.path, |p: &Path| format!("path{}", pos(p.target)));
    slot!(
        "doors_to_close",
        mm.doors_to_close,
        |l: &Vec<[i32; 3]>| format!("n{}", l.len())
    );
    slot!("nearest_bed", mm.nearest_bed, |p: &[i32; 3]| pos(*p));
    slot!("hurt_by", mm.hurt_by, |_: &i64| "DamageSource".to_string());
    slot!("hurt_by_entity", mm.hurt_by_entity, |e: &Who| who(*e));
    slot!("nearest_hostile", mm.nearest_hostile, |e: &Who| who(*e));
    slot!("hiding_place", mm.hiding_place, |p: &[i32; 3]| pos(*p));
    slot!("heard_bell_time", mm.heard_bell_time, |t: &i64| t
        .to_string());
    slot!(
        "cant_reach_walk_target_since",
        mm.cant_reach_walk_target_since,
        |t: &i64| t.to_string()
    );
    slot!(
        "golem_detected_recently",
        mm.golem_detected_recently,
        |t: &bool| t.to_string()
    );
    slot!("last_slept", mm.last_slept, |t: &i64| t.to_string());
    slot!("last_woken", mm.last_woken, |t: &i64| t.to_string());
    slot!("last_worked_at_poi", mm.last_worked_at_poi, |t: &i64| t
        .to_string());
    out.sort();
    Some((acts.join("|"), out.join(" ")))
}
/// `Villager.stopSleeping`: remember waking.
pub(super) fn woken(m: &mut Mob, now: i64) {
    if m.species() != Species::Villager {
        return;
    }
    let extra = super::villager::ext_mut(m);
    if let Some(brain) = extra.brain.as_deref_mut() {
        brain.mem.last_woken.set(now);
    } else {
        // A villager can sleep and wake before it has ever ticked. Seed the same
        // memories initialization would have used, without initializing its sensors.
        let saved = extra.saved_memories.take();
        let mut memories = saved.unwrap_or_else(|| Box::new(initial_memories(m)));
        memories.last_woken.set(now);
        super::villager::ext_mut(m).saved_memories = Some(memories);
    }
}
/// `BellBlockEntity.updateEntities`: a bell rung within 32 blocks sets
/// `HEARD_BELL_TIME` to the game time.
pub fn heard_bell(m: &mut Mob, now: i64) {
    if brain(m).is_some() {
        mem(m).heard_bell_time.set(now);
    }
}
/// Whether `NearestItemSensor` is due for this villager (it is then reset).
pub fn take_item_scan(m: &mut Mob) -> bool {
    brain(m).is_some() && std::mem::take(&mut brain_mut(m).scan_items)
}
/// `NearestItemSensor.doTick`: the nearest wanted item in sight, or none.
pub fn set_wanted_item(m: &mut Mob, at: Option<[f64; 3]>) {
    mem(m).nearest_wanted_item.set_opt(at);
}
/// What other villagers see of this one's brain.
pub fn potential_job_site(m: &Mob) -> Option<[i32; 3]> {
    mem_ref(m).and_then(|mm| mm.potential_job_site.value())
}
/// The memories other villagers read: the interaction target, the path's
/// nodes (while it is not done), whether a golem was seen lately and the
/// last sleep.
#[allow(clippy::type_complexity)]
pub fn seen_memories(
    m: &Mob,
) -> (
    Option<Who>,
    Option<(Option<[i32; 3]>, [i32; 3])>,
    bool,
    Option<i64>,
) {
    let Some(mm) = mem_ref(m) else {
        return (None, None, false, None);
    };
    let path = mm
        .path
        .present()
        .then_some(m.ai.nav.path.as_ref())
        .flatten();
    let nodes = path.filter(|p| !p.is_done()).map(|p| {
        let n = |i: usize| [p.nodes[i].x, p.nodes[i].y, p.nodes[i].z];
        ((p.next > 0).then(|| n(p.next - 1)), n(p.next))
    });
    (
        mm.interaction_target.value(),
        nodes,
        mm.golem_detected_recently.present(),
        mm.last_slept.value(),
    )
}

#[cfg(test)]
mod poi_classification_tests {
    use super::*;
    use sunrise_world::chunk::BlockState;

    #[test]
    fn unresolved_profession_does_not_claim_or_reset_a_job() {
        let mut mob = Mob::new(1, crate::mobs::sensor_scan_tests::kind(), Vec3::ZERO, 0.0);
        mob.extra.profession = super::super::villager::ProfessionState::from_id("example:engineer");
        assert!(!poi_matches(
            &mob,
            PoiFilter::Held,
            Poi::Vanilla(PoiType::Farmer),
            crate::mobs::villager::profession_table()
        ));
        assert!(!poi_matches(
            &mob,
            PoiFilter::Acquirable,
            Poi::Vanilla(PoiType::Farmer),
            crate::mobs::villager::profession_table()
        ));
        assert!(!can_reset_profession(
            &mob,
            crate::mobs::villager::profession_table()
        ));
        assert_eq!(mob.extra.profession.id(), "example:engineer");
    }

    #[test]
    fn vanilla_job_filters_and_capacity_remain_distinct() {
        let mut mob = Mob::new(1, crate::mobs::sensor_scan_tests::kind(), Vec3::ZERO, 0.0);
        assert!(!poi_matches(
            &mob,
            PoiFilter::Held,
            Poi::Vanilla(PoiType::Farmer),
            crate::mobs::villager::profession_table()
        ));
        assert!(poi_matches(
            &mob,
            PoiFilter::Acquirable,
            Poi::Vanilla(PoiType::Farmer),
            crate::mobs::villager::profession_table()
        ));
        mob.extra.profession = super::super::villager::ProfessionState::from_index(5);
        assert!(poi_matches(
            &mob,
            PoiFilter::Held,
            Poi::Vanilla(PoiType::Farmer),
            crate::mobs::villager::profession_table()
        ));
        assert!(poi_matches(
            &mob,
            PoiFilter::Acquirable,
            Poi::Vanilla(PoiType::Farmer),
            crate::mobs::villager::profession_table()
        ));
        assert!(!poi_matches(
            &mob,
            PoiFilter::Held,
            Poi::Vanilla(PoiType::Butcher),
            crate::mobs::villager::profession_table()
        ));
        mob.extra.profession = super::super::villager::ProfessionState::from_index(11);
        assert!(!poi_matches(
            &mob,
            PoiFilter::Held,
            Poi::Vanilla(PoiType::Farmer),
            crate::mobs::villager::profession_table()
        ));
        assert!(!poi_matches(
            &mob,
            PoiFilter::Acquirable,
            Poi::Vanilla(PoiType::Farmer),
            crate::mobs::villager::profession_table()
        ));
        mob.extra.profession = super::super::villager::ProfessionState::from_index(20);
        assert!(!poi_matches(
            &mob,
            PoiFilter::Held,
            Poi::Vanilla(PoiType::Farmer),
            crate::mobs::villager::profession_table()
        ));
        assert!(!poi_matches(
            &mob,
            PoiFilter::Acquirable,
            Poi::Vanilla(PoiType::Farmer),
            crate::mobs::villager::profession_table()
        ));
        assert_eq!(Poi::Vanilla(PoiType::Farmer).max_tickets(), 1);
        assert_eq!(Poi::Vanilla(PoiType::Meeting).max_tickets(), 32);
        assert_eq!(Poi::Vanilla(PoiType::Meeting).valid_range(), 6);
    }

    #[test]
    #[ignore = "requires the vanilla bed POI oracle output"]
    fn vanilla_bed_poi_states() {
        let path = std::env::var("SUNRISE_BED_POI_ORACLE").expect("oracle output path");
        let rows = std::fs::read_to_string(path).unwrap();
        assert_eq!(rows.lines().count(), 272);
        for row in rows.lines() {
            let fields: Vec<_> = row.split_whitespace().collect();
            let [name, part, expected] = fields[..] else {
                panic!("invalid oracle row")
            };
            let expected = expected == "true";
            let state = BlockState {
                name: name.into(),
                properties: [("part".into(), part.into())].into_iter().collect(),
            };
            assert_eq!(
                runtime_bed_poi(name, Some(part)).is_some(),
                expected,
                "{row}"
            );
            assert_eq!(
                sunrise_world::poi::saved_poi(&state) == Some(PoiType::Home),
                expected,
                "{row}"
            );
        }
    }

    /// A block a mod added to `minecraft:home` (`extend_poi_types`) is the bed a
    /// villager's search finds, although vanilla gives hay bales no POI.
    #[test]
    fn a_block_a_mod_added_to_home_is_a_bed_to_a_villager() {
        let Some(root) = std::env::var_os("SUNRISE_TEST_RESOURCES") else {
            return;
        };
        assert!(crate::mobs::world::with_resource_test_world(
            std::path::Path::new(&root),
            &[],
            true,
            true,
            |world, registry| {
                let hay = registry.default_state("minecraft:hay_block").unwrap();
                if !world.chunks.contains_key(&(0, 0)) {
                    world.chunks.insert(
                        (0, 0),
                        std::sync::Arc::new(crate::chunk::Chunk::empty(0, 0)),
                    );
                }
                world.set(0, 64, 0, hay).unwrap();
                world
                    .set(1, 64, 0, registry.default_state("minecraft:stone").unwrap())
                    .unwrap();
            },
            |w, _| {
                let hay_states = (0..w.registry.len())
                    .filter_map(|i| {
                        w.registry
                            .state(sunrise_world::registry::BlockStateId(i as u32))
                    })
                    .filter(|state| state.name == "minecraft:hay_block")
                    .map(|state| sunrise_world::poi::PoiExtension {
                        state: state.clone(),
                        poi: "minecraft:home".into(),
                    })
                    .collect::<Vec<_>>();
                assert_eq!(hay_states.len(), 3);
                let meeting_states = hay_states
                    .iter()
                    .cloned()
                    .map(|mut state| {
                        state.poi = "minecraft:meeting".into();
                        state
                    })
                    .collect();
                sunrise_world::poi::with_extensions(hay_states, |refusals| {
                    assert_eq!(refusals, Vec::<String>::new());
                    assert_eq!(poi_at(w, 0, 64, 0), Some(Poi::Vanilla(PoiType::Home)));
                    assert_eq!(poi_at(w, 1, 64, 0), None);
                    let mob = Mob::new(1, crate::mobs::sensor_scan_tests::kind(), Vec3::ZERO, 0.0);
                    assert!(poi_exists(&mob, w, [0, 64, 0], PoiFilter::Home));
                    assert!(!poi_exists(&mob, w, [0, 64, 0], PoiFilter::Meeting));
                    assert!(!poi_exists(&mob, w, [1, 64, 0], PoiFilter::Home));
                });
                sunrise_world::poi::with_extensions(meeting_states, |refusals| {
                    assert!(refusals.is_empty());
                    assert_eq!(poi_at(w, 0, 64, 0), Some(Poi::Vanilla(PoiType::Meeting)));
                });
                sunrise_world::poi::with_extensions(Vec::new(), |_| {
                    assert_eq!(poi_at(w, 0, 64, 0), None);
                });
            },
        ));
    }

    #[test]
    fn bed_heads_agree_between_runtime_and_saved_pois() {
        for (name, part, runtime, saved) in [
            ("minecraft:straw_bed", "head", false, false),
            ("minecraft:straw_bed", "foot", false, false),
            ("minecraft:red_bed", "head", true, true),
            ("minecraft:red_bed", "foot", false, false),
        ] {
            let state = BlockState {
                name: name.into(),
                properties: [("part".into(), part.into())].into_iter().collect(),
            };
            assert_eq!(runtime_bed_poi(name, Some(part)).is_some(), runtime);
            assert_eq!(sunrise_world::poi::poi_type(&state).is_some(), saved);
        }
    }
}

#[cfg(test)]
#[path = "villager_trades_tests.rs"]
mod trades_tests;

#[cfg(test)]
mod reputation_witness_tests;
