//! The game's side of the WebAssembly interface (feature `wasm-host`): the generated host
//! forwarding answers a mod's imports from the hosts of the call in progress, and the
//! generated proxies make a wasm mod a [`Hooks`] and its handlers parts. Loading, linking and
//! checking packages is `horizon-mod-wasm`'s; this module runs what it instantiated.
use crate::HookId;
use glue::Frame;
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};
use wasmtime::component::{Component, Instance, Linker};
use wasmtime::{AsContextMut, Store, StoreContextMut};

use crate::wasm_refused::Refused;

#[cfg(test)]
#[path = "wasm_callback_tests.rs"]
mod callback_tests;

#[cfg(test)]
#[path = "wasm_transport_tests.rs"]
mod transport_tests;

/// Why a linked group can't run, named for the Mods page.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoadError {
    /// The module doesn't export an interface or function this game calls.
    MissingExport(String),
    /// An export this game calls has another type than the game's.
    ChangedExport(String, String),
    /// The game doesn't provide the first import the module needs, or provides it with another
    /// type.
    Import {
        /// The import (`horizon:mod-api/host@0.1.0#entity`).
        import: String,
        /// What is wrong with it.
        why: String,
    },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingExport(name) => write!(f, "mod.wasm doesn't export `{name}`"),
            Self::ChangedExport(name, why) => {
                write!(f, "mod.wasm's `{name}` has another type: {why}")
            }
            Self::Import { import, why } => {
                write!(
                    f,
                    "mod.wasm imports `{import}`, which this game doesn't have: {why}"
                )
            }
        }
    }
}

impl std::error::Error for LoadError {}

/// How long a call into a group may run, and what it may hold. A call past its limit traps,
/// which stops its group.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Budgets {
    /// One hook or part call, by wall clock (epoch interruption). The limit is time, so a slow
    /// machine reaches it sooner: it is far above what a call should take.
    pub call: Duration,
    /// A group's start-up (its constructors and `register`) and a call carrying a load-phase
    /// context (registration, a data load, a resource reload), which read and parse data.
    pub load: Duration,
    /// Count instructions instead of time, for deterministic and debug runs: a mod then stops
    /// at the same place on every machine. Costs about 15–25% per call; off by default.
    pub fuel: Option<Fuel>,
    /// How long a hook call waits for its group while another thread runs it, before it is
    /// refused as busy. A worldgen call waits as long as it takes.
    pub wait: Duration,
    /// The most memory one group may grow to, in bytes.
    pub memory: usize,
    /// The most bytes of new names (a `&'static str` the game keeps for good) one group may
    /// give the game, beside its memory.
    pub names: usize,
}

/// Instruction budgets, in wasmtime fuel units (about one per instruction), for
/// [`Budgets::fuel`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Fuel {
    /// One hook or part call.
    pub call: u64,
    /// A start-up or a load-phase call.
    pub load: u64,
}

impl Default for Fuel {
    fn default() -> Self {
        Self {
            call: 50_000_000,
            load: 4_000_000_000,
        }
    }
}

/// How often the engine's epoch advances: the resolution of [`Budgets`]' time limits.
pub const EPOCH: Duration = Duration::from_millis(1);

/// What a call into a mod is, for its limits and its wait for the group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Call {
    /// A synchronous predicate callback. May borrow a scoped import store and shares the
    /// enclosing call's remaining budget; a standalone call uses the hook budget.
    Callback,
    /// A hook or part call, on the server or client thread: within `Budgets::call`, and
    /// refused past `Budgets::wait` for a group another thread runs.
    Hook,
    /// Start-up or a load-phase call: within `Budgets::load`.
    Load,
    /// A call on a game worker: a structure type's or piece's on a worldgen worker, a block tint
    /// source's on a meshing worker. Within `Budgets::call`, and queued for its group for as long
    /// as that takes. Refusing it would leave the chunk without its structure, or the block
    /// untinted, depending on timing; the holder's own deadline bounds the wait.
    Worker,
}

impl Budgets {
    /// Arm `store` for one `call`: its fuel, or its deadline in epochs from now.
    fn arm(&self, store: &mut Store<State>, call: Call) {
        let load = call == Call::Load;
        match self.fuel {
            Some(fuel) => {
                let _ = store.set_fuel(if load { fuel.load } else { fuel.call });
            }
            None => {
                let limit = if load { self.load } else { self.call };
                store.set_epoch_deadline((limit.as_nanos() / EPOCH.as_nanos()).max(1) as u64);
            }
        }
    }
}

impl Default for Budgets {
    fn default() -> Self {
        Self {
            call: Duration::from_millis(100),
            load: Duration::from_secs(30),
            fuel: None,
            wait: Duration::from_secs(5),
            memory: 256 << 20,
            names: 1 << 20,
        }
    }
}

/// A group's store data: its limits; the hosts of the call in progress are on the thread.
// exhaustive: its fields are private
pub struct State {
    limits: wasmtime::StoreLimits,
    buffers: HashMap<std::any::TypeId, Box<dyn Any + Send>>,
}

impl State {
    /// Reuses transport scratch owned by this store, including after a refused or trapped call.
    /// Wasmtime's owned list bindings otherwise allocate a fresh host vector per crossing.
    pub fn with_buffer<T: Default + Send + 'static, R>(
        mut store: wasmtime::StoreContextMut<'_, Self>,
        f: impl FnOnce(&mut T, wasmtime::StoreContextMut<'_, Self>) -> R,
    ) -> R {
        struct Scope<'a, T: Send + 'static> {
            store: wasmtime::StoreContextMut<'a, State>,
            buffer: Option<T>,
        }
        impl<T: Send + 'static> Drop for Scope<'_, T> {
            fn drop(&mut self) {
                *self
                    .store
                    .data_mut()
                    .buffers
                    .get_mut(&std::any::TypeId::of::<T>())
                    .unwrap()
                    .downcast_mut::<Option<T>>()
                    .expect("scratch type matches its key") = self.buffer.take();
            }
        }
        let buffer = store
            .data_mut()
            .buffers
            .entry(std::any::TypeId::of::<T>())
            .or_insert_with(|| Box::new(None::<T>))
            .downcast_mut::<Option<T>>()
            .expect("scratch type matches its key")
            .take()
            .unwrap_or_default();
        let mut scope = Scope {
            store,
            buffer: Some(buffer),
        };
        f(
            scope.buffer.as_mut().unwrap(),
            wasmtime::AsContextMut::as_context_mut(&mut scope.store),
        )
    }
}

/// Reusable converted list storage. Large guest inputs are not retained across calls.
struct FlatInput<T>(Vec<T>);
impl<T> Default for FlatInput<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}
struct FlatInputGuard<'a, T>(&'a mut Vec<T>);
impl<T> Drop for FlatInputGuard<'_, T> {
    fn drop(&mut self) {
        self.0.clear();
        let keep = 65_536 / std::mem::size_of::<T>().max(1);
        self.0.shrink_to(keep);
    }
}

thread_local! {
    /// The frame of the innermost call into a mod on this thread.
    static CURRENT: Cell<*mut Frame> = const { Cell::new(std::ptr::null_mut()) };
    /// The mod that call runs in, whose handlers an import hands over.
    static CALLER: Cell<Option<Arc<Running>>> = const { Cell::new(None) };
}

/// The objects of the call in progress on this thread (`glue::Frame`).
fn current() -> wasmtime::Result<&'static mut Frame> {
    let frame = CURRENT.get();
    // SAFETY: `call` sets the pointer to a frame on its stack and restores the previous
    // one before that frame drops; imports run only inside a call.
    unsafe { frame.as_mut() }.ok_or_else(|| wasmtime::Error::msg("a mod import ran outside a call"))
}

/// The trap for an object the call didn't hand over: the mod asked for what its hook doesn't
/// give (a client call from a server hook).
fn missing(what: &str) -> wasmtime::Error {
    wasmtime::Error::msg(format!("this hook gives no {what}"))
}

fn array<T, const N: usize>(items: Vec<T>) -> Result<[T; N], Refused> {
    items.try_into().map_err(|_| Refused::SHAPE)
}

/// The one `&'static str` for each distinct string a mod names. The game keeps each for good,
/// so a new one is charged to the calling mod's group, which is refused past `Budgets::names`.
fn intern(value: String) -> Result<&'static str, Refused> {
    static INTERNED: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
    let mut interned = INTERNED.lock().unwrap_or_else(|e| e.into_inner());
    let set = interned.get_or_insert_with(HashSet::new);
    if let Some(existing) = set.get(value.as_str()) {
        return Ok(existing);
    }
    let module = CALLER.take().ok_or(Refused::NO_CALLER)?;
    let group = &module.group;
    let charged = group.names.fetch_add(value.len(), Ordering::Relaxed) + value.len();
    CALLER.set(Some(module.clone()));
    if charged > group.budgets.names {
        return Err(Refused::because(format_args!(
            "its group gave the game more than {} bytes of names",
            group.budgets.names
        )));
    }
    let leaked: &'static str = Box::leak(value.into_boxed_str());
    set.insert(leaked);
    Ok(leaked)
}

impl Names {
    /// `name`'s index, given it first if it has none.
    fn id(&mut self, name: &'static str) -> Result<u32, Refused> {
        if let Some(id) = self.index.get(name) {
            return Ok(*id);
        }
        let id = u32::try_from(self.names.len()).map_err(|_| Refused::SHAPE)?;
        self.names.push(name);
        self.index.insert(name, id);
        Ok(id)
    }
    /// The name index `id` stands for; refused for an index the table never gave.
    fn name(&self, id: u32) -> Result<&'static str, Refused> {
        self.names
            .get(id as usize)
            .copied()
            .ok_or_else(|| Refused::because(format_args!("no name has index {id} in its group")))
    }
}

/// `f` on the calling group's name table.
fn with_names<T>(f: impl FnOnce(&mut Names) -> Result<T, Refused>) -> Result<T, Refused> {
    let module = CALLER.take().ok_or(Refused::NO_CALLER)?;
    let value = f(&mut module.group.named.lock().unwrap_or_else(|e| e.into_inner()));
    CALLER.set(Some(module));
    value
}

/// The calling group's index for `name`.
fn name_id(name: &'static str) -> Result<u32, Refused> {
    with_names(|names| names.id(name))
}

/// The name the calling group's index `id` stands for.
fn name_of(id: u32) -> Result<&'static str, Refused> {
    with_names(|names| names.name(id))
}

impl glue::horizon::mod_api::names::Host for State {
    fn name(&mut self, id: u32) -> wasmtime::Result<String> {
        Ok(name_of(id)?.to_string())
    }
    fn intern(&mut self, name: String) -> wasmtime::Result<u32> {
        Ok(name_id(intern(name)?)?)
    }
}

/// Runs `lower` as `module`'s caller: arguments lowered before the call names its group.
fn lowering<T>(module: &Arc<Running>, lower: impl FnOnce() -> T) -> T {
    let previous = CALLER.replace(Some(module.clone()));
    let value = lower();
    CALLER.set(previous);
    value
}

/// One call's storage for the values its borrowed arguments point into, as on the guest.
#[derive(Default)]
pub(crate) struct Keep {
    held: RefCell<Vec<(*mut u8, unsafe fn(*mut u8))>>,
}

impl Keep {
    fn hold<T>(&self, value: T) -> *mut T {
        unsafe fn drop_box<T>(pointer: *mut u8) {
            // SAFETY: `pointer` came from `Box::into_raw` of a `Box<T>` in `hold`, once.
            drop(unsafe { Box::from_raw(pointer.cast::<T>()) });
        }
        let pointer = Box::into_raw(Box::new(value));
        self.held.borrow_mut().push((pointer.cast(), drop_box::<T>));
        pointer
    }
    pub(crate) fn str(&self, value: String) -> &str {
        // SAFETY: the box stays allocated, unmoved and handed out once, until `self` drops.
        unsafe { &*self.hold(value) }
    }
    pub(crate) fn slice<T>(&self, value: Vec<T>) -> &[T] {
        // SAFETY: as in `str`.
        unsafe { &*self.hold(value) }
    }
    pub(crate) fn val<T>(&self, value: T) -> &T {
        // SAFETY: as in `str`.
        unsafe { &*self.hold(value) }
    }
}

impl Drop for Keep {
    fn drop(&mut self) {
        for (pointer, drop) in self.held.get_mut().drain(..).rev() {
            // SAFETY: each entry is a live box `hold` made, dropped once here.
            unsafe { drop(pointer) };
        }
    }
}

/// One linked dependency group: one store, one memory, one copy of each member's code.
// exhaustive: its fields are private
pub struct Group {
    store: Mutex<Store<State>>,
    /// The thread running the group now. Re-entry needs a scoped import store; other
    /// calls from this thread refuse instead of deadlocking.
    running: AtomicU64,
    poisoned: AtomicBool,
    budgets: Budgets,
    /// The bytes of new names it gave the game, against `budgets.names`.
    names: AtomicUsize,
    /// The names crossing to and from it, by index: one per name, never reused while the group
    /// lives, and dropped with it.
    named: Mutex<Names>,
}

/// A group's name table: a `&'static str` crosses as its index here, the guest caching each.
#[derive(Default)]
struct Names {
    index: HashMap<&'static str, u32>,
    names: Vec<&'static str>,
}

thread_local! {
    /// This thread's number, for `Group::running` (0 is no thread).
    static THREAD: u64 = {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        NEXT.fetch_add(1, Ordering::Relaxed)
    };
    /// How many groups this thread is inside now.
    static INSIDE: Cell<u32> = const { Cell::new(0) };
}

/// Why a call into a group didn't run.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CallRefused {
    /// A trap stopped the group earlier; it runs no more calls.
    Stopped,
    /// The call came back into the group without a scoped import store to borrow.
    Reentered,
    /// Another thread is running the group: past the wait, or at once when this thread is
    /// inside another group, since two threads each holding a group and waiting for the
    /// other's would deadlock.
    Busy,
}

/// A group entered by this thread, left when dropped.
struct Entered<'a> {
    group: &'a Group,
    store: MutexGuard<'a, Store<State>>,
}

impl Drop for Entered<'_> {
    fn drop(&mut self) {
        self.group.running.store(0, Ordering::Release);
        INSIDE.with(|n| n.set(n.get() - 1));
    }
}

/// How many linked groups are alive.
static LIVE_GROUPS: AtomicUsize = AtomicUsize::new(0);

/// How many linked groups are alive: a group refused at load is gone once its load returns, its
/// store and memory with it.
pub fn live_groups() -> usize {
    LIVE_GROUPS.load(Ordering::Relaxed)
}

impl Drop for Group {
    fn drop(&mut self) {
        LIVE_GROUPS.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Group {
    fn new(store: Store<State>, budgets: Budgets) -> Self {
        LIVE_GROUPS.fetch_add(1, Ordering::Relaxed);
        Self {
            store: Mutex::new(store),
            running: AtomicU64::new(0),
            poisoned: AtomicBool::new(false),
            budgets,
            names: AtomicUsize::new(0),
            named: Mutex::default(),
        }
    }

    /// Enters the group for a `call`: a worker call waits for it as long as another thread
    /// runs it, any other up to `budgets.wait`.
    fn enter(&self, call: Call) -> Result<Entered<'_>, CallRefused> {
        if self.poisoned.load(Ordering::Relaxed) {
            return Err(CallRefused::Stopped);
        }
        let me = THREAD.with(|n| *n);
        if self.running.load(Ordering::Acquire) == me {
            return Err(CallRefused::Reentered);
        }
        // A thread already inside a group never waits for another one.
        let inside = INSIDE.with(Cell::get) > 0;
        let deadline = Instant::now()
            + if inside {
                Duration::ZERO
            } else {
                self.budgets.wait
            };
        let store = loop {
            if call == Call::Worker && !inside {
                break self.store.lock().unwrap_or_else(|e| e.into_inner());
            }
            match self.store.try_lock() {
                Ok(guard) => break guard,
                Err(TryLockError::Poisoned(e)) => break e.into_inner(),
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_micros(50))
                }
                Err(TryLockError::WouldBlock) => return Err(CallRefused::Busy),
            }
        };
        // A call that waited may find the group stopped by the trap of the one it waited for.
        if self.poisoned.load(Ordering::Relaxed) {
            return Err(CallRefused::Stopped);
        }
        self.running.store(me, Ordering::Release);
        INSIDE.with(|n| n.set(n.get() + 1));
        Ok(Entered { group: self, store })
    }
}

/// One wasm mod of a group, as its calls run: its group, its exports and its id.
struct Running {
    group: Arc<Group>,
    instance: Instance,
    exports: glue::ModExports,
    id: &'static str,
    /// The indexes of its handlers the game no longer holds, for the mod to drop at its next
    /// call.
    released: Mutex<Vec<u32>>,
}

/// One wasm mod of a group: the [`Hooks`](crate::Hooks) the game dispatches to.
// exhaustive: its fields are private
pub struct WasmMod {
    module: Arc<Running>,
    subscribed: &'static [Option<HookId>],
}

/// One of a wasm mod's registered parts or handlers, by the index the mod keeps it under.
// exhaustive: its fields are private
pub struct WasmHandler {
    module: Arc<Running>,
    index: u32,
    subscribed: &'static [Option<HookId>],
    /// An erased provider (`dyn Any`): `index` is its address in the group's memory, which
    /// the mod never frees.
    erased: bool,
}

/// What a call answers when the mod gives no answer (its call refused, or a trap): what a
/// mod not listening would.
pub(crate) trait NoAnswer {
    fn no_answer() -> Self;
}

impl<T: Default> NoAnswer for T {
    fn no_answer() -> Self {
        T::default()
    }
}

impl NoAnswer for crate::Flow {
    fn no_answer() -> Self {
        crate::Flow::Continue
    }
}

struct CallScope {
    frame: *mut Frame,
    caller: Option<Arc<Running>>,
}
impl Drop for CallScope {
    fn drop(&mut self) {
        CURRENT.set(self.frame);
        CALLER.set(self.caller.take());
    }
}

thread_local! {
    /// Stores lent by imports on this thread, innermost first. Nodes live on the stack.
    static CALLBACK_STORE: Cell<*const CallbackStore> = const { Cell::new(std::ptr::null()) };
}

struct CallbackStore {
    group: *const Group,
    store: Cell<*mut StoreContextMut<'static, State>>,
    previous: *const CallbackStore,
}

impl Drop for CallbackStore {
    fn drop(&mut self) {
        CALLBACK_STORE.set(self.previous);
    }
}

/// Lends only the import's store, while native code may synchronously call its handlers.
/// No borrow of store data or guest memory may remain live across `f`.
fn with_callback_store<T>(mut store: StoreContextMut<'_, State>, f: impl FnOnce() -> T) -> T {
    let caller = CALLER.take();
    let group = caller.as_ref().map(|m| Arc::as_ptr(&m.group));
    CALLER.set(caller);
    let Some(group) = group else { return f() };
    let scope = CallbackStore {
        group,
        // Erase the context's lifetime only for the stack-scoped TLS node below.
        store: Cell::new((&mut store as *mut StoreContextMut<'_, State>).cast()),
        previous: CALLBACK_STORE.get(),
    };
    CALLBACK_STORE.set(&scope);
    let result = f();
    drop(scope);
    result
}

/// Exclusive loan of a suspended import's context. Removing its pointer prevents another
/// callback from borrowing it while guest code is running; a nested import lends a new one.
struct ResumedStore {
    scope: *const CallbackStore,
    store: *mut StoreContextMut<'static, State>,
}

impl ResumedStore {
    fn take(group: &Group) -> Option<Self> {
        let mut pointer = CALLBACK_STORE.get();
        while !pointer.is_null() {
            // SAFETY: nodes are installed only for `with_callback_store`'s synchronous call,
            // on this thread, and removed before their stack storage is dropped.
            let scope = unsafe { &*pointer };
            if std::ptr::eq(scope.group, group) {
                let store = scope.store.replace(std::ptr::null_mut());
                return (!store.is_null()).then_some(Self {
                    scope: pointer,
                    store,
                });
            }
            pointer = scope.previous;
        }
        None
    }

    fn context(&mut self) -> StoreContextMut<'_, State> {
        // SAFETY: `take` exclusively removed this context from its live import scope. The
        // outer call holds the group lock, and cannot use the context until this loan drops.
        unsafe { &mut *self.store }.as_context_mut()
    }
}

impl Drop for ResumedStore {
    fn drop(&mut self) {
        // SAFETY: this loan is used synchronously inside the import scope that owns the node.
        unsafe { &*self.scope }.store.set(self.store);
    }
}

impl Running {
    /// Runs one `call` with `frame`'s hosts, within its group's limits for it. A refused call
    /// or a trap answers `no_answer()`; a trap stops the whole group.
    fn call_or<T>(
        self: &Arc<Self>,
        mut frame: Frame,
        call: Call,
        no_answer: impl FnOnce() -> T,
        f: impl FnOnce(&glue::ModExports, &mut StoreContextMut<'_, State>) -> wasmtime::Result<T>,
    ) -> T {
        if self.group.poisoned.load(Ordering::Relaxed) {
            return no_answer();
        }
        let mut resumed = if call == Call::Callback {
            ResumedStore::take(&self.group)
        } else {
            None
        };
        let mut entered = if resumed.is_none() {
            match self.group.enter(call) {
                Ok(entered) => Some(entered),
                Err(refused) => {
                    log::warn!("wasm mod {}: call refused: {refused:?}", self.id);
                    return no_answer();
                }
            }
        } else {
            None
        };
        let mut store = match &mut resumed {
            Some(resumed) => resumed.context(),
            None => {
                let store = &mut *entered.as_mut().unwrap().store;
                self.group.budgets.arm(store, call);
                store.as_context_mut()
            }
        };
        let previous = CURRENT.replace(&mut frame);
        let caller = CALLER.replace(Some(self.clone()));
        let scope = CallScope {
            frame: previous,
            caller,
        };
        let released =
            std::mem::take(&mut *self.released.lock().unwrap_or_else(|e| e.into_inner()));
        let result = match released.is_empty() {
            true => Ok(()),
            false => self
                .exports
                .horizon_mod_api_hooks()
                .call_release_handlers(&mut store, released),
        }
        .and_then(|()| f(&self.exports, &mut store));
        drop(scope);
        // Stopped before the group is let go, so no call waiting for it runs on.
        if result.is_err() {
            self.group.poisoned.store(true, Ordering::Relaxed);
        }
        drop(resumed);
        drop(entered);
        match result {
            Ok(value) => value,
            Err(error) => {
                log::error!("wasm mod {} stopped its group: {error:#}", self.id);
                no_answer()
            }
        }
    }
}

/// The answer of a call not sent to `module` because an argument can't cross, with the reason
/// logged.
fn unsent<T>(module: &Running, why: Refused, no_answer: impl FnOnce() -> T) -> T {
    log::error!("wasm mod {}: a call wasn't sent: {why}", module.id);
    no_answer()
}

/// The error a fallible export answers when the mod's call didn't run (refused, or a trap).
fn refused<E: From<(crate::Identifier, String)>>() -> E {
    let mod_api = crate::Identifier::parse("horizon:wasm_mod").expect("a valid id");
    E::from((mod_api, "the mod's call didn't run".into()))
}

impl WasmMod {
    fn call_or<T>(
        &self,
        frame: Frame,
        call: Call,
        no_answer: impl FnOnce() -> T,
        f: impl FnOnce(&glue::ModExports, &mut StoreContextMut<'_, State>) -> wasmtime::Result<T>,
    ) -> T {
        self.module.call_or(frame, call, no_answer, f)
    }
}

impl WasmHandler {
    fn call_or<T>(
        &self,
        frame: Frame,
        call: Call,
        no_answer: impl FnOnce() -> T,
        f: impl FnOnce(&glue::ModExports, &mut StoreContextMut<'_, State>) -> wasmtime::Result<T>,
    ) -> T {
        self.module.call_or(frame, call, no_answer, f)
    }
}

/// A mod's handler shows as its index, the game knowing nothing more of it.
impl std::fmt::Debug for WasmHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmHandler")
            .field("mod", &self.module.id)
            .field("index", &self.index)
            .finish()
    }
}

impl Drop for WasmHandler {
    fn drop(&mut self) {
        OWNED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&(self as *const Self as usize));
        if self.erased {
            return;
        }
        self.module
            .released
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(self.index);
    }
}

/// Where each proxy the game holds of a wasm mod's handler lives, with its mod and that mod's
/// index, so one handed back to its own mod goes by that index (`hand_over`).
static OWNED: LazyLock<Mutex<HashMap<usize, Owned>>> = LazyLock::new(Default::default);

#[derive(Clone, Copy)]
struct Owned {
    module: usize,
    group: usize,
    index: u32,
    erased: bool,
}

thread_local! {
    /// The game's objects each group holds (a capability's handler, another group's or a
    /// native mod's provider), by group, at the index the group calls them by. A group's
    /// calls run on the thread holding it, so they stay on this one.
    static HELD: RefCell<HashMap<usize, Held>> = RefCell::default();
}

/// One group's held objects: each an `Arc` of its trait object, and the free indexes.
#[derive(Default)]
struct Held {
    objects: Vec<Option<Box<dyn Any>>>,
    free: Vec<u32>,
}

fn group_of(module: &Running) -> usize {
    Arc::as_ptr(&module.group) as usize
}

/// The calling mod, and its group, if a call runs.
fn caller() -> (usize, usize) {
    let module = CALLER.take();
    let ids = module
        .as_ref()
        .map_or((0, 0), |m| (Arc::as_ptr(m) as usize, group_of(m)));
    CALLER.set(module);
    ids
}

fn caller_group() -> usize {
    caller().1
}

/// `proxy`, registered by where it lives.
fn owned<P: Deref<Target = WasmHandler>>(proxy: P) -> P {
    let at = &*proxy as *const WasmHandler as usize;
    OWNED.lock().unwrap_or_else(|e| e.into_inner()).insert(
        at,
        Owned {
            module: Arc::as_ptr(&proxy.module) as usize,
            group: group_of(&proxy.module),
            index: proxy.index,
            erased: proxy.erased,
        },
    );
    proxy
}

/// The handler record for `object` as the calling mod gets it: its own handler by its own
/// index (an erased provider, by its address, to any mod of its group, which shares its
/// memory), else the game's object, held for it.
fn hand_over<T: ?Sized + 'static>(object: Arc<T>) -> glue::horizon::mod_api::types::Handler {
    let (module, group) = caller();
    let at = Arc::as_ptr(&object) as *const () as usize;
    let own = OWNED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&at)
        .copied();
    if let Some(Owned { index, .. }) = own.filter(|own| match own.erased {
        true => own.group == group,
        false => own.module == module,
    }) {
        return glue::horizon::mod_api::types::Handler {
            index,
            subscribed: Vec::new(),
            held: false,
        };
    }
    let index = HELD.with(|held| {
        let mut held = held.borrow_mut();
        let held = held.entry(group).or_default();
        let object: Box<dyn Any> = Box::new(object);
        match held.free.pop() {
            Some(index) => {
                held.objects[index as usize] = Some(object);
                index
            }
            None => {
                held.objects.push(Some(object));
                (held.objects.len() - 1) as u32
            }
        }
    });
    glue::horizon::mod_api::types::Handler {
        index,
        subscribed: Vec::new(),
        held: true,
    }
}

/// [`hand_over`] for a boxed object: a proxy moving into an `Arc` is registered where it lands.
fn hand_over_box<T: ?Sized + 'static>(object: Box<T>) -> glue::horizon::mod_api::types::Handler {
    let at = &*object as *const T as *const () as usize;
    let own = OWNED.lock().unwrap_or_else(|e| e.into_inner()).remove(&at);
    let object: Arc<T> = Arc::from(object);
    if let Some(own) = own {
        OWNED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(Arc::as_ptr(&object) as *const () as usize, own);
    }
    hand_over(object)
}

/// The calling mod's held object at `index`, as the trait it is called through.
fn held<T: ?Sized + 'static>(index: u32) -> wasmtime::Result<Arc<T>> {
    let group = caller_group();
    HELD.with(|held| {
        held.borrow()
            .get(&group)
            .and_then(|held| held.objects.get(index as usize)?.as_ref())
            .and_then(|object| object.downcast_ref::<Arc<T>>())
            .cloned()
    })
    .ok_or_else(|| wasmtime::Error::msg("a mod called an object the game doesn't hold for it"))
}

impl glue::horizon::mod_api::held_handles::Host for State {
    fn release(&mut self, handle: u32) -> wasmtime::Result<()> {
        let group = caller_group();
        HELD.with(|held| {
            if let Some(held) = held.borrow_mut().get_mut(&group)
                && let Some(slot) = held.objects.get_mut(handle as usize)
                && slot.take().is_some()
            {
                held.free.push(handle);
            }
        });
        Ok(())
    }
}

/// The hooks a subscribed-hooks export named.
fn hook_ids(names: Vec<String>) -> &'static [Option<HookId>] {
    Box::leak(
        names
            .iter()
            .map(|n| glue::hook_id(n))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    )
}

/// The game's proxy of a handler the calling mod handed over (a part it registers), which calls
/// back into it by the index it keeps the handler under.
fn handler_of(handler: glue::horizon::mod_api::types::Handler) -> Result<WasmHandler, Refused> {
    let module = CALLER.take().ok_or(Refused::NO_CALLER)?;
    CALLER.set(Some(module.clone()));
    Ok(WasmHandler {
        module,
        index: handler.index,
        subscribed: hook_ids(handler.subscribed),
        erased: false,
    })
}

/// The game's proxy of an erased provider the calling mod handed over, at `handler.index` in
/// its group's memory. No call goes through it: only mods of that group take it back.
fn erased_of(handler: glue::horizon::mod_api::types::Handler) -> Result<WasmHandler, Refused> {
    let mut proxy = handler_of(handler)?;
    proxy.erased = true;
    Ok(proxy)
}

/// The import an instantiation refused: wasmtime names the interface in its outer context and the
/// function in the next (`component imports instance `…``, `instance export `…``), and says why
/// last.
fn refused_import(error: wasmtime::Error) -> LoadError {
    let named: Vec<String> = error
        .chain()
        .take(2)
        .filter_map(|layer| {
            let text = layer.to_string();
            let (_, rest) = text.split_once('`')?;
            Some(rest.split_once('`')?.0.to_owned())
        })
        .collect();
    LoadError::Import {
        import: named.join("#"),
        why: error.root_cause().to_string(),
    }
}

/// A member of a group: its mod id, and the prefix of its exports, `<prefix>-<interface>`.
// exhaustive: the runtime names each member it links, and nothing else builds one
pub struct Member {
    /// The mod's id.
    pub id: &'static str,
    /// The prefix of its exports (`horizon_mod_package::manifest::export_prefix`).
    pub exports: String,
}

/// Instantiates a linked group and makes each member, in dependency order, a mod entry.
pub fn instantiate(
    engine: &wasmtime::Engine,
    component: &Component,
    members: &[Member],
    budgets: Budgets,
    imports: impl FnOnce(&mut Linker<State>),
) -> Result<Vec<Instantiated>, LoadError> {
    let mut linker = Linker::<State>::new(engine);
    glue::Mod::add_to_linker::<State, wasmtime::component::HasSelf<State>>(&mut linker, |s| s)
        .expect("the game's own imports are defined once");
    glue::add_store_imports(&mut linker).expect("the store-aware imports are defined once");
    imports(&mut linker);
    let limits = wasmtime::StoreLimitsBuilder::new()
        .memory_size(budgets.memory)
        .build();
    let mut store = Store::new(
        engine,
        State {
            limits,
            buffers: HashMap::new(),
        },
    );
    store.limiter(|state| &mut state.limits);
    budgets.arm(&mut store, Call::Load);
    let instance: Instance = linker
        .instantiate(&mut store, component)
        .map_err(refused_import)?;
    let mut found = Vec::new();
    for member in members {
        let exports = glue::ModExports::find(&mut store, &instance, &member.exports)?;
        found.push((member.id, exports));
    }
    let group = Arc::new(Group::new(store, budgets));
    let mut entries = Vec::new();
    for (id, exports) in found {
        let module = Arc::new(Running {
            group: group.clone(),
            instance,
            exports,
            id,
            released: Mutex::new(Vec::new()),
        });
        let names = module.call_or(Frame::default(), Call::Load, Vec::new, |m, store| {
            m.horizon_mod_api_hooks().call_subscribed(store)
        });
        let hooks = WasmMod {
            module: module.clone(),
            subscribed: hook_ids(names),
        };
        entries.push(Instantiated {
            entry: crate::ModEntry::new(id, Box::new(hooks)),
            exports: Exports(module),
        });
    }
    Ok(entries)
}

/// A member of an instantiated group: the mod entry to install, and its handle for the exports
/// other crates add to its world.
// exhaustive: `instantiate` makes each
pub struct Instantiated {
    /// The mod's hooks, to install.
    pub entry: crate::ModEntry,
    /// Its other exports.
    pub exports: Exports,
}

/// A member's handle for the exports another crate adds to the mod's world (the points crate's
/// listeners): found by name and type, and called as hook calls with the call's host in the frame,
/// within the group's limits. A trap stops the group as for any call.
// exhaustive: its field is private
#[derive(Clone)]
pub struct Exports(Arc<Running>);

/// The host of a call through [`Exports`]: the reading one, or a side's writing one.
#[non_exhaustive]
pub enum CallHost<'a> {
    /// A reading call's host.
    Read(&'a dyn crate::Host),
    /// A server call's writing host.
    Server(&'a mut dyn crate::ServerHost),
    /// A client call's writing host.
    Client(&'a mut dyn crate::ClientHost),
    /// A call with no host: the mod's imports that need one trap.
    None,
}

impl Exports {
    /// The mod's id.
    pub fn id(&self) -> &'static str {
        self.0.id
    }

    /// The member's function `function` of its exported interface `interface`
    /// (`<prefix>-<interface>`), with the game's type for it.
    pub fn func<P, R>(
        &self,
        interface: &str,
        function: &str,
    ) -> Result<wasmtime::component::TypedFunc<P, R>, LoadError>
    where
        P: wasmtime::component::ComponentNamedList + wasmtime::component::Lower,
        R: wasmtime::component::ComponentNamedList + wasmtime::component::Lift,
    {
        let named = format!("{interface}#{function}");
        let mut store = self.0.group.store.lock().unwrap_or_else(|e| e.into_inner());
        let instance = &self.0.instance;
        let found = instance
            .get_export_index(&mut *store, None, interface)
            .and_then(|interface| {
                instance.get_export_index(&mut *store, Some(&interface), function)
            })
            .ok_or_else(|| LoadError::MissingExport(named.clone()))?;
        instance
            .get_typed_func(&mut *store, found)
            .map_err(|e| LoadError::ChangedExport(named, e.to_string()))
    }

    /// Runs `f` as one hook call with `host`: `None` when it didn't run (refused, or a trap,
    /// which stops the group).
    pub fn call<T>(
        &self,
        host: CallHost<'_>,
        f: impl FnOnce(&mut StoreContextMut<'_, State>) -> wasmtime::Result<T>,
    ) -> Option<T> {
        let mut frame = Frame::default();
        match host {
            CallHost::Read(h) => frame.host(h),
            CallHost::Server(h) => frame.server_host_mut(h),
            CallHost::Client(h) => frame.client_host_mut(h),
            CallHost::None => {}
        }
        self.0
            .call_or(frame, Call::Hook, || None, |_, store| f(store).map(Some))
    }

    /// Runs `f` as the group's load-phase call, with no host: for the checks the game makes of
    /// a member once, at load.
    pub fn load<T>(
        &self,
        f: impl FnOnce(&mut StoreContextMut<'_, State>) -> wasmtime::Result<T>,
    ) -> Option<T> {
        self.0.call_or(
            Frame::default(),
            Call::Load,
            || None,
            |_, store| f(store).map(Some),
        )
    }

    /// Whether a trap stopped the member's group.
    pub fn stopped(&self) -> bool {
        self.0.group.poisoned.load(Ordering::Relaxed)
    }
}

#[allow(
    clippy::all,
    unreachable_code,
    unused_imports,
    unused_variables,
    unused_parens,
    dead_code,
    non_snake_case,
    unused_mut
)]
mod glue {
    use super::{array, intern};
    include!(concat!(env!("OUT_DIR"), "/wasm_host_generated.rs"));

    #[test]
    fn refused_menu_button_output_keeps_all_host_state() {
        let mut entity = crate::ModBlockEntity::new("test:counter".into(), Default::default());
        let revision = entity.revision();
        let context = crate::menu::MenuButtonContext {
            player: 0,
            player_entity_id: 0,
            pos: [0; 3],
            id: 0,
        };
        let mut carried = None;
        let mut event = crate::MenuButtonEvent {
            context: &context,
            entity: crate::ModBlockEntityView::new(&mut entity),
            carried: &mut carried,
        };
        let mut wire = block_entity_view(&event.entity).unwrap();
        wire.entity.fields.data = vec![17];
        let output = horizon::mod_api::types::MenuButtonEventOut {
            entity: Some(wire),
            carried: Some(Some(vec![])),
        };
        assert!(write_menu_button_event(&mut event, output).is_err());
        assert!(
            entity.data.is_empty(),
            "a refused return must apply no earlier writes"
        );
        assert_eq!(entity.revision(), revision);
        assert!(carried.is_none());
    }

    #[test]
    fn refused_menu_quick_move_output_keeps_all_host_state() {
        let mut entity = crate::ModBlockEntity::new("test:counter".into(), Default::default());
        let revision = entity.revision();
        let mut stack = crate::ItemStack {
            item: "minecraft:stone".into(),
            count: 1,
            damage: 0,
            extra: None,
        };
        let original = stack.clone();
        let mut event = crate::MenuQuickMoveEvent {
            pos: [0; 3],
            entity: crate::ModBlockEntityView::new(&mut entity),
            container_slots: 2,
            slot: 0,
            stack: &mut stack,
        };
        let mut wire = block_entity_view(&event.entity).unwrap();
        wire.entity.fields.data = vec![17];
        let output = horizon::mod_api::types::MenuQuickMoveEventOut {
            pos: Some((1, 2, 3)),
            entity: Some(wire),
            container_slots: Some(4),
            slot: Some(1),
            stack: Some(vec![]),
        };
        assert!(write_menu_quick_move_event(&mut event, output).is_err());
        assert_eq!(event.pos, [0; 3]);
        assert_eq!(event.container_slots, 2);
        assert_eq!(event.slot, 0);
        assert!(entity.data.is_empty());
        assert_eq!(entity.revision(), revision);
        assert_eq!(stack, original);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_scope_restores_tls_on_unwind() {
        let previous = CURRENT.get();
        let mut frame = Frame::default();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _scope = CallScope {
                frame: CURRENT.replace(&mut frame),
                caller: CALLER.replace(None),
            };
            panic!("fixture unwind");
        }));
        assert!(result.is_err());
        assert_eq!(CURRENT.get(), previous);
        assert!(CALLER.take().is_none());
    }

    #[test]
    fn world_line_array_tuple_roundtrip() {
        let line = crate::WorldLine::new(
            [1.25, -2.5, 3.75],
            [4.0, 5.5, -6.25],
            0x12345678,
            2.5,
            crate::WorldDepth::ThroughWalls,
        );
        assert_eq!(glue::world_line_roundtrip(&line).unwrap(), line);
    }

    #[test]
    fn recipe_rule_generated_wire_roundtrip() {
        use crate::{Ingredient, Recipe, RecipeRule, RecipeTemplate, Tag};
        let mut recipe = Recipe::new(
            "test:rule",
            "minecraft:crafting_transmute",
            Arc::new(serde_json::json!({"custom": 42})),
        );
        recipe.rule = Some(RecipeRule::Transmute {
            input: Ingredient::of(vec!["test:input".into()]),
            material: Ingredient::of(vec!["test:material".into()]),
            min: 2,
            max: 4,
            add_count: true,
            result: RecipeTemplate::new(
                None,
                3,
                Some(Tag::compound([("!test:removed", Tag::Int(0))])),
            ),
        });
        let output = glue::recipe_roundtrip(&recipe).unwrap();
        assert_eq!(output.id, recipe.id);
        assert_eq!(*output.json, *recipe.json);
        let Some(RecipeRule::Transmute {
            input,
            material,
            min,
            max,
            add_count,
            result,
        }) = output.rule
        else {
            panic!("wrong generated recipe rule")
        };
        assert_eq!(input.order, ["test:input"]);
        assert_eq!(material.order, ["test:material"]);
        assert_eq!((min, max, add_count), (2, 4, true));
        assert_eq!((result.item, result.count), (None, 3));
        assert!(matches!(
            result.components.unwrap().get("!test:removed"),
            Some(Tag::Int(0))
        ));
    }

    #[test]
    fn smithing_rule_generated_wire_roundtrip() {
        use crate::{Ingredient, Recipe, RecipeRule, RecipeTemplate, RecipeTrimPattern, Tag};
        let mut recipe = Recipe::new(
            "test:smith",
            "minecraft:smithing_transform",
            Arc::new(serde_json::json!({})),
        );
        recipe.rule = Some(RecipeRule::SmithingTransform {
            template: None,
            base: Ingredient::of(vec!["test:base".into()]),
            addition: None,
            result: RecipeTemplate::new(
                Some("test:result".into()),
                2,
                Some(Tag::compound([("test:patch", Tag::Int(7))])),
            ),
        });
        let Some(RecipeRule::SmithingTransform {
            template,
            addition,
            result,
            ..
        }) = glue::recipe_roundtrip(&recipe).unwrap().rule
        else {
            panic!("wrong rule")
        };
        assert!(template.is_none() && addition.is_none());
        assert_eq!(result.count, 2);
        assert_eq!(
            result.components.unwrap().get("test:patch"),
            Some(&Tag::Int(7))
        );
        recipe.rule = Some(RecipeRule::SmithingTrim {
            template: Ingredient::of(vec![]),
            base: Ingredient::of(vec![]),
            addition: Ingredient::of(vec![]),
            pattern: RecipeTrimPattern::Inline {
                asset_id: "test:pattern".into(),
                description: Tag::String("description".into()),
                decal: true,
            },
        });
        let Some(RecipeRule::SmithingTrim {
            pattern:
                RecipeTrimPattern::Inline {
                    asset_id,
                    description,
                    decal,
                },
            ..
        }) = glue::recipe_roundtrip(&recipe).unwrap().rule
        else {
            panic!("wrong rule")
        };
        assert_eq!(asset_id, "test:pattern");
        assert_eq!(description, Tag::String("description".into()));
        assert!(decal);
    }

    fn group(wait: Duration) -> Arc<Group> {
        let engine = wasmtime::Engine::default();
        let limits = wasmtime::StoreLimitsBuilder::new().build();
        let budgets = Budgets {
            wait,
            ..Budgets::default()
        };
        Arc::new(Group::new(
            Store::new(
                &engine,
                State {
                    limits,
                    buffers: HashMap::new(),
                },
            ),
            budgets,
        ))
    }

    #[test]
    fn a_name_keeps_one_index_and_an_index_never_given_is_refused() {
        let mut names = Names::default();
        let overworld = names.id("minecraft:overworld").unwrap();
        let nether = names.id("minecraft:the_nether").unwrap();
        assert_ne!(overworld, nether);
        assert_eq!(names.id("minecraft:overworld").unwrap(), overworld);
        assert_eq!(names.name(nether).unwrap(), "minecraft:the_nether");
        let refused = names.name(nether + 1).unwrap_err();
        assert!(
            refused.to_string().contains("no name has index"),
            "{refused}"
        );
    }

    #[test]
    fn a_call_back_into_its_own_group_is_refused_at_once() {
        let group = group(Duration::from_secs(60));
        let _inside = group.enter(Call::Hook).unwrap();
        let started = Instant::now();
        assert_eq!(group.enter(Call::Hook).err(), Some(CallRefused::Reentered));
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn two_threads_each_inside_a_group_calling_the_other_refuse_at_once() {
        let (a, b) = (
            group(Duration::from_secs(60)),
            group(Duration::from_secs(60)),
        );
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let cross = |mine: Arc<Group>, theirs: Arc<Group>, barrier: Arc<std::sync::Barrier>| {
            std::thread::spawn(move || {
                let _inside = mine.enter(Call::Hook).unwrap();
                barrier.wait();
                let started = Instant::now();
                let refused = theirs.enter(Call::Hook).err();
                // Hold on until the other thread has tried too.
                barrier.wait();
                (refused, started.elapsed())
            })
        };
        let one = cross(a.clone(), b.clone(), barrier.clone());
        let two = cross(b, a, barrier);
        for thread in [one, two] {
            let (refused, took) = thread.join().unwrap();
            assert_eq!(refused, Some(CallRefused::Busy));
            assert!(took < Duration::from_millis(100), "{took:?}");
        }
    }

    #[test]
    fn a_call_waits_for_another_thread_then_refuses_or_runs() {
        let group = group(Duration::from_millis(200));
        let inside = group.enter(Call::Hook).unwrap();
        let other = group.clone();
        let refused = std::thread::spawn(move || other.enter(Call::Hook).err())
            .join()
            .unwrap();
        assert_eq!(refused, Some(CallRefused::Busy));
        let other = group.clone();
        let waiting = std::thread::spawn(move || other.enter(Call::Hook).is_ok());
        std::thread::sleep(Duration::from_millis(20));
        drop(inside);
        assert!(waiting.join().unwrap());
    }

    #[test]
    fn a_worker_call_waits_past_the_wait_for_its_group() {
        let group = group(Duration::from_millis(1));
        let inside = group.enter(Call::Hook).unwrap();
        let other = group.clone();
        let waiting = std::thread::spawn(move || other.enter(Call::Worker).is_ok());
        std::thread::sleep(Duration::from_millis(50));
        drop(inside);
        assert!(waiting.join().unwrap());
    }

    #[test]
    fn a_worker_call_queued_behind_a_trap_finds_the_group_stopped() {
        let group = group(Duration::from_millis(1));
        let inside = group.enter(Call::Hook).unwrap();
        let other = group.clone();
        let waiting = std::thread::spawn(move || other.enter(Call::Worker).err());
        std::thread::sleep(Duration::from_millis(50));
        group.poisoned.store(true, Ordering::Relaxed);
        drop(inside);
        assert_eq!(waiting.join().unwrap(), Some(CallRefused::Stopped));
    }

    #[test]
    fn a_stopped_group_runs_nothing() {
        let group = group(Duration::ZERO);
        group.poisoned.store(true, Ordering::Relaxed);
        assert_eq!(group.enter(Call::Hook).err(), Some(CallRefused::Stopped));
    }
}

#[cfg(test)]
#[path = "wasm_key_tests.rs"]
mod key_tests;
