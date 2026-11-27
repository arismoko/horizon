//! The helper threads of the server tick's parallel phases: one pool, made once, the first time a
//! phase shares its work, instead of threads started for every phase of every tick. A phase hands
//! it work that gives the same result in whatever order the threads take it, and each part's
//! results stay in order.
//!
//! Whether a tick's phases share their work is the Parallel Tick setting's
//! (`graphics::ParallelTickMode`: Off, Auto or On), decided between ticks for the next one
//! ([`after_tick`]) and never during one. `SUNRISE_PARALLEL_TICK` wins over the setting, and is
//! all a dedicated server follows (`server.toml`'s `parallel_tick`).
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use sunrise_level::tick_mode::ParallelTickMode;

/// Typed gameplay dispatch owns mutable residency and damage contexts, so Sunrise Fork keeps
/// every tick serial whatever the setting or `SUNRISE_PARALLEL_TICK` says (Diverges from upstream).
const FORK_PARALLEL_TICK_ENABLED: bool = false;

/// `SUNRISE_PARALLEL_TICK`, whatever the setting says: `0` keeps every tick serial, `1` turns the
/// server tick's parallel phases on, and `verify` does too, also checking the phases that can be
/// checked against the serial code. Unset, the setting decides.
fn forced() -> Option<u8> {
    static FORCED: OnceLock<Option<u8>> = OnceLock::new();
    *FORCED.get_or_init(|| match std::env::var("SUNRISE_PARALLEL_TICK").as_deref() {
        Ok("0" | "off" | "false") => Some(0),
        Ok("1" | "on" | "true") => Some(1),
        Ok("verify") => Some(2),
        _ => None,
    })
}

/// The Parallel Tick setting, as the window last applied it; Off until it does (a dedicated
/// server never does).
static SETTING: AtomicU8 = AtomicU8::new(0);
/// Whether the server's next tick shares its phases' work, by the setting ([`after_tick`]).
static SHARED: AtomicU8 = AtomicU8::new(0);
/// What Auto is doing, for the status ([`status`]).
static AUTO_STATE: AtomicU8 = AtomicU8::new(0);
static AUTO: Mutex<Auto> = Mutex::new(Auto::new());

/// Whether this tick's phases share their work: 0 serial, 1 shared, 2 shared and checked against
/// the serial code (`SUNRISE_PARALLEL_TICK=verify`). Each phase gives the serial result either way.
pub fn parallel_tick() -> u8 {
    if !FORK_PARALLEL_TICK_ENABLED {
        static NOTICE: std::sync::Once = std::sync::Once::new();
        NOTICE.call_once(|| {
            if forced().is_some_and(|mode| mode > 0) {
                eprintln!("warning: parallel tick is disabled in Sunrise Fork while typed hook dispatch is integrated; using serial tick");
            }
        });
        return 0;
    }
    forced().unwrap_or_else(|| SHARED.load(Ordering::Relaxed))
}

/// The Parallel Tick setting, from the window's settings: On and Off apply from the next tick,
/// and Auto starts watching the ticks again.
pub fn set_setting(mode: ParallelTickMode) {
    let mode = mode as u8;
    if SETTING.swap(mode, Ordering::Relaxed) != mode {
        *AUTO.lock().unwrap_or_else(PoisonError::into_inner) = Auto::new();
        AUTO_STATE.store(0, Ordering::Relaxed);
        SHARED.store(
            u8::from(mode == ParallelTickMode::On as u8),
            Ordering::Relaxed,
        );
    }
}

/// The server thread, after a tick that took `ms`, `cpu_ms` of them on the processor (where the
/// system tells): decides whether the next tick shares its phases' work. Only Auto has something
/// to decide.
pub fn after_tick(ms: f32, cpu_ms: Option<f32>) {
    if !FORK_PARALLEL_TICK_ENABLED
        || forced().is_some()
        || SETTING.load(Ordering::Relaxed) != ParallelTickMode::Auto as u8
    {
        return;
    }
    let mut auto = AUTO.lock().unwrap_or_else(PoisonError::into_inner);
    let was = auto.shared;
    let shared = auto.after_tick(ms, cpu_ms.unwrap_or(ms), sunrise_threads::worker_count());
    AUTO_STATE.store(auto.state as u8, Ordering::Relaxed);
    if shared != was {
        let ms = auto.last_median;
        if shared {
            log::info!("Sunrise: the tick takes {ms:.1} ms, sharing it among threads");
        } else {
            log::info!("Sunrise: the tick takes {ms:.1} ms, back on one thread");
        }
        SHARED.store(u8::from(shared), Ordering::Relaxed);
    }
}

/// The setting, whether the next tick shares its work, and what Auto is doing (`watching`,
/// `trial` or `kept`), for the control status.
pub fn status() -> (&'static str, bool, &'static str) {
    let setting = match (forced(), SETTING.load(Ordering::Relaxed)) {
        (Some(_), _) => "forced",
        (None, 1) => "auto",
        (None, 2) => "on",
        _ => "off",
    };
    let auto = match AUTO_STATE.load(Ordering::Relaxed) {
        1 => "trial",
        2 => "kept",
        _ => "watching",
    };
    (setting, parallel_tick() > 0, auto)
}

/// Ticks in one of Auto's verdicts: five seconds of them.
const WINDOW: usize = 100;
/// The serial tick's median time on the processor, in milliseconds, from which Auto tries
/// sharing it: half of a tick's 50 ms. Below it the server keeps up easily, and helper threads
/// would only take cores from the frame and the chunk workers. Time on the processor, not on the
/// clock: ticks that are slow because the machine is busy or swapping have no work to share.
const HEAVY_MS: f32 = 25.0;
/// The shared tick's median under which Auto goes back to one thread: far enough under
/// `HEAVY_MS` that the serial tick it returns to doesn't start a trial again.
const LIGHT_MS: f32 = 10.0;
/// A trial is kept when the tick's median dropped under this share of what it was.
const KEEP_BELOW: f32 = 0.95;
/// Heavy windows in a row before a trial, and light ones before going back.
const STREAK: u8 = 2;
/// Windows left out after a switch, while the helper threads start and the caches settle.
const SETTLE: u8 = 1;
/// How many ticks after a trial that didn't help before the next one: 5 minutes after the
/// first, 20 after every later one.
const RETRY_AFTER: [u64; 2] = [5 * 60 * 20, 20 * 60 * 20];
/// Fewer hardware threads than this and Auto never shares the tick: the helpers would run on
/// the cores the frame and the chunk workers need.
const MIN_THREADS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// On one thread, looking for heavy ticks.
    Watching = 0,
    /// Shared for a trial: left out `SETTLE` windows, then measured against the ticks before.
    Trial = 1,
    /// The trial made the ticks faster: shared until they are light again.
    Kept = 2,
}

/// What Auto has decided and why, a window of ticks at a time. Nothing here starts a thread:
/// the caller applies what it answers.
struct Auto {
    window: Vec<f32>,
    /// The same ticks' time on the processor.
    cpu: Vec<f32>,
    shared: bool,
    state: State,
    /// Heavy windows in a row while watching, light ones while kept.
    streak: u8,
    /// The serial tick's median before the trial, in milliseconds.
    before: f32,
    /// Windows still left out after a switch.
    settling: u8,
    /// Ticks watched so far, and the tick from which the next trial may start.
    ticks: u64,
    retry_at: u64,
    /// Trials that didn't help.
    rejected: usize,
    last_median: f32,
}

impl Auto {
    const fn new() -> Self {
        Self {
            window: Vec::new(),
            cpu: Vec::new(),
            shared: false,
            state: State::Watching,
            streak: 0,
            before: 0.0,
            settling: 0,
            ticks: 0,
            retry_at: 0,
            rejected: 0,
            last_median: 0.0,
        }
    }

    /// A tick took `ms`, `cpu_ms` of them on the processor, on a machine with `threads`
    /// hardware threads: whether the next one shares its phases' work.
    fn after_tick(&mut self, ms: f32, cpu_ms: f32, threads: usize) -> bool {
        self.ticks += 1;
        self.window.push(ms);
        self.cpu.push(cpu_ms);
        if self.window.len() < WINDOW {
            return self.shared;
        }
        self.window.sort_by(f32::total_cmp);
        self.cpu.sort_by(f32::total_cmp);
        let (median, working) = (self.window[WINDOW / 2], self.cpu[WINDOW / 2]);
        self.window.clear();
        self.cpu.clear();
        self.last_median = median;
        if self.settling > 0 {
            self.settling -= 1;
            return self.shared;
        }
        match self.state {
            State::Watching => {
                let heavy =
                    working >= HEAVY_MS && threads >= MIN_THREADS && self.ticks >= self.retry_at;
                self.streak = if heavy { self.streak + 1 } else { 0 };
                if self.streak >= STREAK {
                    self.before = median;
                    self.switch(State::Trial, true);
                }
            }
            State::Trial => {
                if median < self.before * KEEP_BELOW {
                    self.state = State::Kept;
                    self.streak = 0;
                } else {
                    self.retry_at = self.ticks + RETRY_AFTER[self.rejected.min(1)];
                    self.rejected += 1;
                    self.switch(State::Watching, false);
                }
            }
            State::Kept => {
                self.streak = if median < LIGHT_MS {
                    self.streak + 1
                } else {
                    0
                };
                if self.streak >= STREAK {
                    self.switch(State::Watching, false);
                }
            }
        }
        self.shared
    }

    fn switch(&mut self, state: State, shared: bool) {
        self.state = state;
        self.shared = shared;
        self.streak = 0;
        self.settling = SETTLE;
    }
}

/// How many threads a phase may share its work among: `sunrise-threads`' count (at most 8) while
/// the parallel tick is on, else 1.
pub fn lanes() -> usize {
    if parallel_tick() > 0 {
        sunrise_threads::worker_count().clamp(1, 8)
    } else {
        1
    }
}

/// The pool: one thread fewer than a phase's lanes (the server thread takes parts too).
fn pool() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        log::info!("Sunrise: starting the tick's helper threads");
        rayon::ThreadPoolBuilder::new()
            .num_threads(
                sunrise_threads::worker_count()
                    .clamp(1, 8)
                    .saturating_sub(1)
                    .max(1),
            )
            .thread_name(|i| format!("tick-helper-{i}"))
            .build()
            .expect("the tick's helper threads start")
    })
}

/// Proof that the caller is the thread that owns a `Game` and its tick: the server thread, or the
/// window's frame. Zero-sized and `!Send + !Sync`, so it never reaches another thread, and a
/// closure that captures it can't be handed to [`par_map`] or [`phase`].
///
/// The state only the tick may change takes it, today entering a level for block contacts
/// (`block_contact::in_level`), so such a change happens between a tick's phases, never under
/// one. A `Game` mints its own when it is built and lends it out by copy.
#[derive(Clone, Copy, Debug)]
pub struct TickThread(std::marker::PhantomData<*const ()>);

impl TickThread {
    /// The token for the `Game` this thread runs. Only a loop owner calls this, once, when it
    /// builds its `Game`.
    ///
    /// # Panics
    ///
    /// Inside a tick phase's work: a worker never holds the tick's authority.
    pub fn mint() -> Self {
        assert!(
            !sunrise_core::tick_phase::in_tick_phase(),
            "the tick thread's token was asked for inside a tick phase's work"
        );
        Self(std::marker::PhantomData)
    }
}

pub(crate) mod sealed {
    /// Keeps [`super::PhaseView`] to the views this crate vets.
    pub trait Sealed {}
}

/// A tick phase's read-only input: plain data a worker may read on any thread, and nothing that
/// dispatches a hook, reads or writes mod state, or holds a host, a random or a swappable table.
/// Sealed: only the views this crate vets implement it (`RandomTickView`), never `LoopContext`,
/// `World`, `Resources` or the `Game`. A view that needs a table the tick can replace holds its
/// own `Arc` of it, taken when the phase starts (`ListenSnapshot` for who listens, which `phase` puts in force on each worker).
pub trait PhaseView: sealed::Sealed + Sync {
    /// Who listens, for a phase that must tell where a mod would take part: the snapshot the view
    /// captured when the phase started. `None` for a view that reaches no listener.
    fn listens(&self) -> Option<&ListenSnapshot> {
        None
    }
}

/// Who listens, as plain data a worker may read on any thread: the listen index of the game's
/// parts table and each block state's block key, held by `Arc` so a phase keeps the tables it
/// started with while the tick thread replaces them between phases. The tick thread captures it
/// ([`ListenSnapshot::capture`]) into the phase's view; [`phase`] puts it in force on every worker
/// for the work's length, so `block_listens` answers from it there instead of "no listener".
#[derive(Clone, Debug)]
pub struct ListenSnapshot(mod_api::__private::ListenScope);

impl ListenSnapshot {
    /// The snapshot of the tables the tick thread holds now.
    pub fn capture(
        _tick: &TickThread,
        listens: std::sync::Arc<mod_api::__private::ListenIndex>,
        blocks: std::sync::Arc<[mod_api::BlockKey]>,
    ) -> Self {
        Self(mod_api::__private::ListenScope::new(listens, blocks))
    }

    /// Whether a part answers `hook` for the block state, as `block_listens` does on the tick
    /// thread: false for a state the snapshot has no block for.
    pub fn listens(
        &self,
        hook: mod_api::HookId,
        state: sunrise_world::registry::BlockStateId,
    ) -> bool {
        self.0.listens(hook, state.0)
    }

    /// Run `f` with this snapshot answering the engine's routing questions on this thread.
    pub fn enter<R>(&self, f: impl FnOnce() -> R) -> R {
        self.0.enter(f)
    }
}

/// A tick phase: `work` over `items` on the pool (all on this thread while the parallel tick is
/// off, or for fewer than `min` items), with every result back in the order of `items`, whatever
/// order the threads took them in. The tick thread applies them in that order.
///
/// `work` is a function pointer, not a closure, so it captures nothing: what a worker sees is
/// `view` and its item, and nothing that only the tick thread may touch. Inside `work`,
/// `sunrise_core::tick_phase::in_tick_phase` holds on every thread, so tick-owned state refuses it by name
/// ([`TickThread::mint`], `block_contact`'s level):
///
/// ```no_run
/// # use sunrise_sim::parallel::{phase, TickThread};
/// # fn view() -> &'static sunrise_sim::random_tick_sections::RandomTickView<'static> { unimplemented!() }
/// let _tick = TickThread::mint();
/// phase(view(), &[1u32], 1, |_, item| item + 1);
/// ```
/// ```compile_fail
/// # use sunrise_sim::parallel::{phase, TickThread};
/// # fn view() -> &'static sunrise_sim::random_tick_sections::RandomTickView<'static> { unimplemented!() }
/// let tick = TickThread::mint();
/// // A worker can't capture the tick's token (or anything else).
/// phase(view(), &[1u32], 1, |_, item| {
///     std::hint::black_box(tick);
///     item + 1
/// });
/// ```
///
/// Tick phases use this; [`par_map`] and [`par_chunks_mut`] stay for work outside the tick's mod
/// reach (rendering, packs, the LAN tracker's plain data).
pub fn phase<V: PhaseView, T: Sync, R: Send>(
    view: &V,
    items: &[T],
    min: usize,
    work: fn(&V, &T) -> R,
) -> Vec<R> {
    par_map(items, min, |part, out| {
        let mut run = || out.extend(part.iter().map(|item| work(view, item)));
        sunrise_core::tick_phase::as_tick_phase(|| match view.listens() {
            Some(listens) => listens.enter(run),
            None => run(),
        });
    })
}

/// `f` over `items` in parts on the pool, or all on this thread while the parallel tick is off or
/// for fewer than `min` items: every part's results, in the order of `items`, as one thread would
/// give them.
pub fn par_map<T: Sync, R: Send>(
    items: &[T],
    min: usize,
    f: impl Fn(&[T], &mut Vec<R>) + Sync,
) -> Vec<R> {
    par_map_on(items, min, lanes(), f)
}

/// `f` over `items` in chunks of `chunk`, in place, on the pool: the same sharing out as
/// `sunrise_threads::par_chunks_mut`, without starting threads for every call. A chunk runs on
/// whatever thread takes it, so `f` must not depend on the order. Unlike [`par_map`] it does not
/// wait for the parallel tick's switch (the phases that use it ran shared out before the pool
/// existed, #8158): it shares out unless `SUNRISE_SERIAL_TICK=all` or one thread is all there is.
/// The thread-locals the tick relies on do not follow a helper.
pub fn par_chunks_mut<T: Send>(items: &mut [T], chunk: usize, f: impl Fn(&mut [T]) + Send + Sync) {
    let lanes = if sunrise_threads::serial_tick_all() {
        1
    } else {
        sunrise_threads::worker_count().clamp(1, 8)
    };
    par_chunks_mut_on(items, chunk, lanes, f);
}

/// [`par_chunks_mut`] on `lanes` threads.
fn par_chunks_mut_on<T: Send>(
    items: &mut [T],
    chunk: usize,
    lanes: usize,
    f: impl Fn(&mut [T]) + Send + Sync,
) {
    let chunk = chunk.max(1);
    let count = items.len().div_ceil(chunk);
    if lanes <= 1 || count < 2 {
        for slice in items.chunks_mut(chunk) {
            f(slice);
        }
        return;
    }
    // A thread takes a chunk whole; the lock is only held while a chunk is taken, never while
    // it runs: the chunk is popped in a statement of its own, so the guard is gone before `f`
    // (a guard in a `while let` scrutinee would live through the loop body).
    let pending = Mutex::new(items.chunks_mut(chunk).collect::<Vec<_>>());
    let work = || {
        loop {
            let next = pending.lock().unwrap_or_else(PoisonError::into_inner).pop();
            let Some(slice) = next else { break };
            f(slice);
        }
    };
    pool().in_place_scope(|s| {
        for _ in 1..lanes.min(count) {
            s.spawn(|_| work());
        }
        work();
    });
}

/// [`par_map`] on `lanes` threads (a test's count, or a caller that caps them).
///
/// The parts are small (four a thread) and go to whichever thread is free, this one too: a helper
/// the machine is slow to run (the chunk workers compete for the same cores) holds the phase up by
/// one part at most instead of an eighth of the work, and the server thread works instead of only
/// waiting.
pub fn par_map_on<T: Sync, R: Send>(
    items: &[T],
    min: usize,
    lanes: usize,
    f: impl Fn(&[T], &mut Vec<R>) + Sync,
) -> Vec<R> {
    if lanes <= 1 || items.len() < min.max(2) {
        let mut out = Vec::with_capacity(items.len());
        f(items, &mut out);
        return out;
    }
    let size = items.len().div_ceil(lanes * 4);
    let count = items.len().div_ceil(size);
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Vec<R>>> = (0..count).map(|_| Mutex::new(Vec::new())).collect();
    let work = || {
        loop {
            let k = next.fetch_add(1, Ordering::Relaxed);
            if k >= count {
                break;
            }
            let part = &items[k * size..((k + 1) * size).min(items.len())];
            let mut done = Vec::with_capacity(part.len());
            f(part, &mut done);
            *slots[k].lock().unwrap_or_else(PoisonError::into_inner) = done;
        }
    };
    pool().in_place_scope(|s| {
        for _ in 1..lanes {
            s.spawn(|_| work());
        }
        work();
    });
    let mut out = Vec::with_capacity(items.len());
    for slot in slots {
        out.extend(slot.into_inner().unwrap_or_else(PoisonError::into_inner));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Auto keeps a quiet world on one thread, shares a heavy tick only after two heavy windows,
    /// keeps the trial when the ticks got faster, goes back once they are light, and after a
    /// trial that didn't help waits before the next.
    #[test]
    fn auto_shares_only_heavy_ticks_and_only_while_it_helps() {
        let run = |auto: &mut Auto, ms: f32, windows: usize| {
            let mut shared = auto.shared;
            for _ in 0..windows * WINDOW {
                shared = auto.after_tick(ms, ms, 8);
            }
            shared
        };
        let mut auto = Auto::new();
        assert!(!run(&mut auto, 4.0, 20), "a quiet world stays serial");
        assert!(!run(&mut auto, 36.0, 1), "one heavy window is not enough");
        assert!(run(&mut auto, 36.0, 1), "the second starts a trial");
        // The window after the switch is left out; the next one is faster: kept.
        assert!(run(&mut auto, 30.0, 2));
        assert_eq!(auto.state, State::Kept);
        // Light ticks for two windows: back on one thread, and it stays there.
        assert!(run(&mut auto, 6.0, 1));
        assert!(!run(&mut auto, 6.0, 1));
        assert!(!run(&mut auto, 9.0, 20));
        // A trial that doesn't help is undone, and not tried again for five minutes.
        assert!(
            run(&mut auto, 36.0, 3),
            "two heavy windows, then the one left out"
        );
        assert!(!run(&mut auto, 36.0, 2), "no faster: undone");
        assert!(!run(&mut auto, 36.0, 40), "waiting");
        assert!(run(&mut auto, 36.0, 20), "tried again after five minutes");
        // Too few hardware threads: never.
        let mut small = Auto::new();
        for _ in 0..20 * WINDOW {
            assert!(!small.after_tick(40.0, 40.0, 2));
        }
        // Ticks that are slow on the clock while the thread hardly works (a busy or swapping
        // machine) have nothing to share.
        let mut starved = Auto::new();
        for _ in 0..20 * WINDOW {
            assert!(!starved.after_tick(150.0, 4.0, 8));
        }
    }

    /// On several threads, every item's result comes back once, in the items' order, for item
    /// counts that don't divide into the parts evenly; a part may give several results or none.
    #[test]
    fn parts_come_back_in_order() {
        for n in [0usize, 1, 7, 33, 1000, 4097] {
            let items: Vec<u64> = (0..n as u64).collect();
            let f = |part: &[u64], out: &mut Vec<u64>| {
                for &x in part {
                    // Spin a little so the threads overlap.
                    let mut h = x;
                    for _ in 0..200 {
                        h = h.wrapping_mul(6364136223846793005).wrapping_add(1);
                    }
                    if x % 3 != 0 {
                        out.push(x * 10);
                    }
                    if x % 5 == 0 {
                        out.push(h);
                    }
                }
            };
            let mut serial = Vec::new();
            f(&items, &mut serial);
            assert_eq!(par_map_on(&items, 1, 4, f), serial, "{n} items");
        }
    }

    /// Every chunk is handled once, whole, and its items changed in place, for item counts that
    /// don't divide into chunks evenly.
    #[test]
    fn chunks_are_each_handled_once() {
        for n in [0usize, 1, 15, 16, 17, 79, 80, 1000] {
            let mut items: Vec<u64> = (0..n as u64).collect();
            par_chunks_mut_on(&mut items, 16, 4, |chunk| {
                assert!(chunk.len() <= 16);
                for x in chunk {
                    *x = *x * 3 + 1;
                }
            });
            assert_eq!(items, (0..n as u64).map(|x| x * 3 + 1).collect::<Vec<_>>());
        }
    }

    /// Chunks really run at the same time: one waits (bounded, so a serial run fails instead of
    /// hanging) until another has started, and each item is still handled exactly once.
    #[test]
    fn chunks_overlap_and_run_once() {
        use std::sync::atomic::AtomicBool;
        use std::time::{Duration, Instant};
        let active = AtomicUsize::new(0);
        let overlapped = AtomicBool::new(false);
        let deadline = Instant::now() + Duration::from_secs(10);
        let runs: Vec<AtomicUsize> = (0..64).map(|_| AtomicUsize::new(0)).collect();
        let mut items: Vec<usize> = (0..64).collect();
        par_chunks_mut_on(&mut items, 4, 4, |chunk| {
            for &i in chunk.iter() {
                runs[i].fetch_add(1, Ordering::SeqCst);
            }
            if active.fetch_add(1, Ordering::SeqCst) >= 1 {
                overlapped.store(true, Ordering::SeqCst);
            }
            while !overlapped.load(Ordering::SeqCst) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
            }
            active.fetch_sub(1, Ordering::SeqCst);
        });
        assert!(
            overlapped.load(Ordering::SeqCst),
            "no two chunks ran at once"
        );
        assert!(runs.iter().all(|r| r.load(Ordering::SeqCst) == 1));
    }

    /// `phase` puts the view's listen snapshot in force on every worker thread, and only inside
    /// the work: outside a phase no scope answers.
    #[test]
    fn a_phase_answers_who_listens_from_its_views_snapshot() {
        struct View(ListenSnapshot);
        impl sealed::Sealed for View {}
        impl PhaseView for View {
            fn listens(&self) -> Option<&ListenSnapshot> {
                Some(&self.0)
            }
        }
        let snapshot = ListenSnapshot::capture(
            &TickThread::mint(),
            Default::default(),
            Vec::<mod_api::BlockKey>::new().into(),
        );
        let items: Vec<u32> = (0..64).collect();
        let hook = mod_api::HookId::AnalogOutput;
        assert_eq!(mod_api::__private::scoped_state_listens(hook, 0), None);
        let answers = phase(&View(snapshot), &items, 1, |_, _| {
            mod_api::__private::scoped_state_listens(mod_api::HookId::AnalogOutput, 0)
        });
        assert!(answers.iter().all(|&a| a == Some(false)));
        assert_eq!(mod_api::__private::scoped_state_listens(hook, 0), None);
    }

    /// A phase's work never gets the tick's authority, on whatever thread it runs: minting the
    /// token there is refused, and the tick thread can mint it again once the phase is over.
    #[test]
    fn a_phase_worker_cannot_take_the_tick_token() {
        struct View;
        impl sealed::Sealed for View {}
        impl PhaseView for View {}
        let items: Vec<u32> = (0..64).collect();
        let refused = phase(&View, &items, 1, |_, _| {
            std::panic::catch_unwind(TickThread::mint).is_err()
        });
        assert!(refused.iter().all(|&refused| refused));
        assert!(!sunrise_core::tick_phase::in_tick_phase());
        TickThread::mint();
    }

    /// A panic in a part reaches the caller instead of being lost on a helper.
    #[test]
    fn a_panic_reaches_the_caller() {
        let items: Vec<u32> = (0..256).collect();
        let caught = std::panic::catch_unwind(|| {
            par_map_on(&items, 1, 4, |part, out: &mut Vec<u32>| {
                assert!(!part.contains(&200), "item 200");
                out.extend(part);
            })
        });
        assert!(caught.is_err());
    }
}
