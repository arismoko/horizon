//! What each kind of mob costs the server tick, for `sunrise-report` (dedicated/report.rs): while
//! a report records, one tick in [`SAMPLE_EVERY`] times every mob's own tick and adds it up by
//! level and mob type.
//! Off, a mob costs one thread-local read; sampled, two clock reads and a map update.
//! A mob's riders tick inside its own tick, so they count for it. The server tick's
//! spans stay enabled while the report or slow-tick ring needs them ([`want_spans`]).
use std::{
    cell::{Cell, RefCell},
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::Instant,
};
use sunrise_level::fast_hash::FxHashMap;
use sunrise_sim::mobs::MobType;

/// One server tick in this many is sampled while a report records.
pub const SAMPLE_EVERY: usize = 4;

/// The report sampling bit.
const REPORT: u8 = 1;

/// The time and the ticks of one mob type in one level.
#[derive(Default)]
struct Cost {
    nanos: u64,
    mob_ticks: u64,
}

/// Sums by level, then by the type's registry id: one entry per type, found without allocating.
type Totals = FxHashMap<&'static str, FxHashMap<String, Cost>>;

thread_local! {
    /// The report bit, set while it samples.
    static SAMPLING: Cell<u8> = const { Cell::new(0) };
    /// The report's sums.
    static COSTS: RefCell<[Totals; 1]> = RefCell::default();
}

/// Who wants the tick's spans recorded besides nobody: `SUNRISE_SPIKES` (the slow ring's steps),
/// a report's recording.
pub const SPANS_SPIKES: u8 = 1;
pub const SPANS_REPORT: u8 = 2;
#[cfg(test)]
const SPANS_MOD: u8 = 4;
static SPAN_USERS: AtomicU8 = AtomicU8::new(0);

/// `user` wants the spans recorded, or no longer does: they stay on while anyone does.
pub fn want_spans(user: u8, on: bool) {
    let users = if on {
        SPAN_USERS.fetch_or(user, Ordering::Relaxed) | user
    } else {
        SPAN_USERS.fetch_and(!user, Ordering::Relaxed) & !user
    };
    sunrise_level::spans::enable(users != 0);
}

#[cfg(test)]
pub fn wanted(user: u8) -> bool {
    SPAN_USERS.load(Ordering::Relaxed) & user != 0
}

/// Held by every test that switches spans: `want_spans` updates the users and the switch in two steps.
#[cfg(test)]
pub fn spans_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Sample (or stop sampling) the mob ticks this thread runs next, for the report.
pub fn set_sampling(on: bool) {
    set(REPORT, on);
}

fn set(who: u8, on: bool) {
    SAMPLING.with(|s| s.set(if on { s.get() | who } else { s.get() & !who }));
}

/// When this tick is sampled, the start of a mob's tick.
#[inline]
pub fn start() -> Option<Instant> {
    (SAMPLING.with(Cell::get) != 0).then(Instant::now)
}

/// A sampled mob of type `kind` in level `dim` ticked since `started`.
#[inline]
pub fn add(dim: &'static str, kind: &MobType, started: Instant) {
    add_kind(dim, &kind.id, started);
}

/// A mob of type `kind` starts its tick: the CPU sampler's work, and `sunrise-report`'s cost by mob type when sampled.
pub fn start_mob(kind: &Arc<MobType>) -> Option<(Instant, Arc<MobType>)> {
    sunrise_threads::work::set_entity(sunrise_threads::work::entity_id(&kind.id));
    start().map(|t| (t, Arc::clone(kind)))
}

/// The mob [`start_mob`] started ended its tick in level `dim`.
pub fn end_mob(dim: &'static str, sampled: Option<(Instant, Arc<MobType>)>) {
    sunrise_threads::work::clear_work();
    if let Some((started, kind)) = sampled {
        add(dim, &kind, started);
    }
}

/// [`add`] for the type with registry id `id`.
fn add_kind(dim: &'static str, id: &str, started: Instant) {
    let nanos = started.elapsed().as_nanos() as u64;
    let who = SAMPLING.with(Cell::get);
    COSTS.with(|c| {
        let mut costs = c.borrow_mut();
        for (sums, bit) in costs.iter_mut().zip([REPORT]) {
            if who & bit == 0 {
                continue;
            }
            let kinds = sums.entry(dim).or_default();
            // The id is copied once, for a type's first sample in a level.
            if !kinds.contains_key(id) {
                kinds.insert(id.to_owned(), Cost::default());
            }
            let cost = kinds.get_mut(id).expect("just inserted");
            cost.nanos += nanos;
            cost.mob_ticks += 1;
        }
    });
}

/// One level's mob type: its sampled time and mob ticks.
#[derive(Clone, Debug, PartialEq)]
pub struct MobCost {
    pub dimension: &'static str,
    pub kind: String,
    pub nanos: u64,
    pub mob_ticks: u64,
}

/// The report's costs sampled since the last call, costliest first; clears them.
pub fn take() -> Vec<MobCost> {
    take_of(0)
}

fn take_of(sums: usize) -> Vec<MobCost> {
    let costs = COSTS.with(|c| std::mem::take(&mut c.borrow_mut()[sums]));
    let mut out: Vec<MobCost> = costs
        .into_iter()
        .flat_map(|(dimension, kinds)| {
            kinds.into_iter().map(move |(kind, c)| MobCost {
                dimension,
                kind,
                nanos: c.nanos,
                mob_ticks: c.mob_ticks,
            })
        })
        .collect();
    out.sort_by(|a, b| b.nanos.cmp(&a.nanos).then_with(|| a.kind.cmp(&b.kind)));
    out
}

/// What sampling one mob costs on this machine, in nanoseconds: the report's own overhead.
pub fn sample_cost_nanos() -> f64 {
    const N: u32 = 20_000;
    let was = SAMPLING.with(Cell::get);
    let kept = COSTS.with(|c| std::mem::take(&mut c.borrow_mut()[0]));
    SAMPLING.with(|s| s.set(REPORT));
    let timer = Instant::now();
    for _ in 0..N {
        if let Some(started) = start() {
            add_kind("calibration", "calibration", started);
        }
    }
    let per = timer.elapsed().as_nanos() as f64 / f64::from(N);
    SAMPLING.with(|s| s.set(was));
    COSTS.with(|c| c.borrow_mut()[0] = kept);
    per
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(dim: &'static str, id: &str, started: Instant) {
        add_kind(dim, id, started);
    }

    /// Mobs add up by level and type, only on sampled ticks; taking them clears them.
    #[test]
    fn costs_add_up_by_level_and_type_while_sampling() {
        let (zombie, cow) = ("minecraft:zombie", "minecraft:cow");
        set_sampling(false);
        assert!(start().is_none(), "nothing is timed while off");
        set_sampling(true);
        for _ in 0..3 {
            let t = start().expect("sampled");
            std::thread::sleep(std::time::Duration::from_millis(1));
            add("minecraft:overworld", zombie, t);
        }
        add("minecraft:overworld", cow, Instant::now());
        add("minecraft:the_nether", zombie, Instant::now());
        set_sampling(false);
        let costs = take();
        assert_eq!(costs.len(), 3, "{costs:?}");
        assert_eq!(costs[0].kind, "minecraft:zombie");
        assert_eq!(costs[0].dimension, "minecraft:overworld");
        assert_eq!(costs[0].mob_ticks, 3);
        assert!(costs[0].nanos >= 3_000_000, "{}", costs[0].nanos);
        let nether = costs
            .iter()
            .find(|c| c.dimension == "minecraft:the_nether")
            .unwrap();
        assert_eq!(
            (nether.kind.as_str(), nether.mob_ticks),
            ("minecraft:zombie", 1)
        );
        assert!(take().is_empty());
    }

    /// Spans stay on while anyone wants them.
    #[test]
    fn spans_stay_on_while_anyone_wants_them() {
        let _spans = spans_test_lock();
        want_spans(SPANS_MOD, true);
        assert!(sunrise_level::spans::enabled());
        assert_ne!(SPAN_USERS.load(Ordering::Relaxed) & SPANS_MOD, 0);
        want_spans(SPANS_MOD, false);
        assert_eq!(SPAN_USERS.load(Ordering::Relaxed) & SPANS_MOD, 0);
    }

    /// Calibrating leaves what was sampled, and the switch, as they were.
    #[test]
    fn calibration_keeps_the_samples() {
        set_sampling(true);
        add("minecraft:overworld", "minecraft:pig", Instant::now());
        let per = sample_cost_nanos();
        assert!(per > 0.0 && per < 100_000.0, "{per}");
        assert!(start().is_some(), "still sampling");
        set_sampling(false);
        let costs = take();
        assert_eq!(costs.len(), 1);
        assert_eq!(costs[0].kind, "minecraft:pig");
    }
}
