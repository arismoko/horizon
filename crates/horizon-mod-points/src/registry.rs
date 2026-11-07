//! One slot table for API functions, wasm exports and internals listeners. Sealing orders each
//! phase by descending priority and dependency order; sites retain mod-id tie ordering.
use crate::{POINT_COUNT, PointId, SITE_COUNT, SiteId};
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// Each place's slot: set once, at the seal, where a mod listens. A point that nothing listens at
/// costs one load of it.
pub static POINTS: [AtomicBool; POINT_COUNT] = [const { AtomicBool::new(false) }; POINT_COUNT];
/// Call-expression slots, using the same sealed raw-listener table as points.
pub static SITES: [AtomicBool; SITE_COUNT] = [const { AtomicBool::new(false) }; SITE_COUNT];

struct Owner {
    id: &'static str,
    /// Resolved at the seal, so firing reads the owner's canonical enable flag without a scan.
    mod_index: Option<usize>,
    /// Set when the mod stopped (a trap in its wasm group, or the game giving up on it): its
    /// listeners are skipped from then on.
    stopped: AtomicBool,
}

/// How a slot calls a raw listener.
#[derive(Clone, Copy)]
pub enum RawKind {
    /// A function of the place's listener type, cast back by the slot before calling it.
    Typed(*const ()),
    /// A runtime listener receiving the place index, receiver and raw record or return value.
    ///
    /// # Safety
    ///
    /// The caller must pass the registered place's index and valid pointers of that place's exact
    /// receiver and raw record or return types, respecting their mutability, aliasing, ownership
    /// and lifetime requirements for the call's duration. The receiver is null for a free or
    /// associated function, and at a borrowed-return place. A shared receiver must not be written
    /// to; its mutable pointer only erases the type, not its mutability.
    Erased(unsafe fn(usize, *mut (), *mut ())),
    /// A checked runtime export, with its identity preserved through ordering.
    Indexed(unsafe fn(usize, u32, *mut (), *mut ()), u32),
}

// Function pointers: no data behind them, so they move between threads.
unsafe impl Send for RawKind {}
unsafe impl Sync for RawKind {}

/// A typed API function or a generic wasm forwarding endpoint.
#[derive(Clone, Copy)]
pub enum ApiKind {
    /// A function checked by the generated typed constructor before erasure.
    Typed(*const ()),
    /// One checked export index in a wasm mod.
    #[cfg(all(feature = "wasm-host", not(target_arch = "wasm32")))]
    Forward(&'static dyn crate::wire::Forward, u32),
}

// The pointer is a function; forwarding endpoints are Send + Sync resources.
unsafe impl Send for ApiKind {}
unsafe impl Sync for ApiKind {}

#[derive(Clone, Copy)]
enum Kind {
    Api(ApiKind),
    Raw(RawKind),
}

impl Kind {
    fn is_api(self) -> bool {
        matches!(self, Self::Api(_))
    }
}

/// A raw listener in its place's list.
#[derive(Clone, Copy)]
struct Slot {
    listener: u32,
    kind: Kind,
    priority: i32,
    injection: &'static str,
}

/// The list while mods load: registrations, and stops that came before the seal.
struct Open {
    listeners: Vec<Owner>,
    slots: Vec<(usize, Slot)>,
    stopped: Vec<String>,
    /// Set once [`seal`] has taken the list: registering is refused from then on.
    sealing: bool,
}

struct SlotList {
    entries: Box<[Slot]>,
    api_end: usize,
}

struct Sealed {
    listeners: Box<[Owner]>,
    slots: Box<[SlotList]>,
}

/// Registering, stopping and sealing all hold this lock, so a registration either lands in the
/// sealed list or is refused, and a stop before the seal still holds after it.
static OPEN: Mutex<Open> = Mutex::new(Open {
    listeners: Vec::new(),
    slots: Vec::new(),
    stopped: Vec::new(),
    sealing: false,
});
static SEALED: OnceLock<Sealed> = OnceLock::new();

/// Why a listener wasn't registered.
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RegisterError {
    /// The list was sealed: listeners register while the game loads, before its points are set.
    Sealed,
}

impl std::fmt::Display for RegisterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Sealed => "the game's points were already set",
        })
    }
}

impl std::error::Error for RegisterError {}

fn lock() -> std::sync::MutexGuard<'static, Open> {
    OPEN.lock().unwrap_or_else(|e| e.into_inner())
}

/// The open list, while listeners may still register.
fn open() -> Result<std::sync::MutexGuard<'static, Open>, RegisterError> {
    let open = lock();
    if open.sealing {
        return Err(RegisterError::Sealed);
    }
    Ok(open)
}

/// Mod `id`'s entry in the open list, added on its first registration.
fn listener(open: &mut Open, id: &'static str) -> usize {
    match open.listeners.iter().position(|listener| listener.id == id) {
        Some(index) => index,
        None => {
            open.listeners.push(Owner {
                id,
                mod_index: None,
                stopped: AtomicBool::new(false),
            });
            open.listeners.len() - 1
        }
    }
}

/// A function or checked endpoint at an API, internals or site slot.
pub struct SlotListener {
    id: &'static str,
    place: usize,
    kind: Kind,
    priority: i32,
    injection: &'static str,
}

impl SlotListener {
    /// Mod `id`'s listener `function` at `place`.
    ///
    /// # Safety
    ///
    /// `function` is a function of `place`'s raw listener type: `__<Point>Listener` at a head,
    /// `__<Point>ReturnListener` at a return. The place's slot casts it back to that type and calls
    /// it. The launcher's generated `main` checks the type with a typed `let` before erasing it.
    pub const unsafe fn new(id: &'static str, place: PointId, function: *const ()) -> Self {
        Self {
            id,
            place: place as usize,
            kind: Kind::Raw(RawKind::Typed(function)),
            priority: 0,
            injection: "",
        }
    }

    /// Mod `id`'s erased runtime listener `function` at `place`.
    ///
    /// The callback gets the place's index, the receiver and the raw record at a head or the
    /// return value's place at a return (including `()` for a function with no explicit return
    /// type). The receiver is null for a free or associated function. At a borrowed-return place
    /// it is also null: only the value's place is passed, since the value may borrow the receiver.
    /// A shared receiver is passed as a mutable pointer only to erase its type, not its mutability.
    ///
    /// # Safety
    ///
    /// Given the pointers guaranteed by [`RawKind::Erased`], `function` must interpret them using
    /// `place`'s exact receiver and raw record or return types, respecting their mutability,
    /// aliasing, ownership and lifetime requirements. The pointers and any references derived
    /// from them must not escape the callback. In particular, a shared receiver must not be
    /// written to.
    pub const unsafe fn erased(
        id: &'static str,
        place: PointId,
        function: unsafe fn(usize, *mut (), *mut ()),
    ) -> Self {
        Self {
            id,
            place: place as usize,
            kind: Kind::Raw(RawKind::Erased(function)),
            priority: 0,
            injection: "",
        }
    }

    /// A runtime listener whose export index is retained by the registry.
    ///
    /// # Safety
    /// The callback must obey the same pointer contract as [`Self::erased`].
    pub const unsafe fn indexed(
        id: &'static str,
        place: PointId,
        function: unsafe fn(usize, u32, *mut (), *mut ()),
        index: u32,
    ) -> Self {
        Self {
            id,
            place: place as usize,
            kind: Kind::Raw(RawKind::Indexed(function, index)),
            priority: 0,
            injection: "",
        }
    }

    /// A typed call-site wrap listener, sorted by priority, mod id and injection id.
    ///
    /// # Safety
    ///
    /// `function` must have this site's exact generated listener type. Erased point callbacks
    /// cannot register as site listeners: their signature has no original continuation.
    pub const unsafe fn site(
        id: &'static str,
        place: SiteId,
        priority: i32,
        injection: &'static str,
        function: *const (),
    ) -> Self {
        Self {
            id,
            place: POINT_COUNT + place as usize,
            kind: Kind::Raw(RawKind::Typed(function)),
            priority,
            injection,
        }
    }

    /// An API listener whose function has the place's exact generated function type.
    ///
    /// # Safety
    /// The generated constructor must check the function's host and record types before erasure.
    pub const unsafe fn api(id: &'static str, place: PointId, function: *const ()) -> Self {
        Self {
            id,
            place: place as usize,
            kind: Kind::Api(ApiKind::Typed(function)),
            priority: 0,
            injection: "",
        }
    }

    /// A checked wasm API export, forwarded without a per-point proxy.
    #[cfg(all(feature = "wasm-host", not(target_arch = "wasm32")))]
    pub const fn forwarded(
        id: &'static str,
        place: PointId,
        forward: &'static dyn crate::wire::Forward,
        index: u32,
    ) -> Self {
        Self {
            id,
            place: place as usize,
            kind: Kind::Api(ApiKind::Forward(forward, index)),
            priority: 0,
            injection: "",
        }
    }

    /// Orders a point by priority, then dependency order, then inject id within its mod.
    pub const fn ordered(mut self, priority: i32, injection: &'static str) -> Self {
        self.priority = priority;
        self.injection = injection;
        self
    }

    /// The mod's id.
    pub fn id(&self) -> &'static str {
        self.id
    }
}

/// Registers a function or checked endpoint, in the same order for native and wasm owners.
pub fn register(slot: &SlotListener) -> Result<(), RegisterError> {
    let mut open = open()?;
    let listener = listener(&mut open, slot.id) as u32;
    open.slots.push((
        slot.place,
        Slot {
            listener,
            kind: slot.kind,
            priority: slot.priority,
            injection: slot.injection,
        },
    ));
    Ok(())
}

/// Stops mod `id`'s listeners: none of them runs again, whether the list is sealed yet or not. A
/// trap that stops a wasm group and the game giving up on a native mod both end here.
pub fn stop(id: &str) {
    let mut open = lock();
    match SEALED.get() {
        Some(sealed) => {
            for listener in sealed.listeners.iter().filter(|listener| listener.id == id) {
                listener.stopped.store(true, Ordering::Release);
            }
        }
        None => open.stopped.push(id.to_owned()),
    }
}

/// Seals the list, so no listener registers after it, and sets each slot a mod listens at. The game
/// calls it once, after the mods load.
pub fn seal() {
    let (mut listeners, raw_list) = {
        let mut open = lock();
        if open.sealing {
            return;
        }
        open.sealing = true;
        (
            std::mem::take(&mut open.listeners),
            std::mem::take(&mut open.slots),
        )
    };
    for listener in &mut listeners {
        listener.mod_index = mod_api::__private::mod_index(listener.id);
    }
    let mut raw = vec![Vec::new(); POINT_COUNT + SITE_COUNT];
    for (place, function) in raw_list {
        raw[place].push(function);
    }
    for (place, site) in raw.iter_mut().enumerate() {
        if place < POINT_COUNT {
            site.sort_by_key(|entry| {
                (
                    std::cmp::Reverse(entry.kind.is_api()),
                    std::cmp::Reverse(entry.priority),
                )
            });
            for group in site.chunk_by_mut(|a, b| {
                a.kind.is_api() == b.kind.is_api()
                    && a.priority == b.priority
                    && a.listener == b.listener
            }) {
                group.sort_by_key(|entry| entry.injection);
            }
            continue;
        }
        site.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| {
                    listeners[a.listener as usize]
                        .id
                        .cmp(listeners[b.listener as usize].id)
                })
                .then_with(|| a.injection.cmp(b.injection))
        });
    }
    let open = lock();
    // Every stop so far, before the seal or while it ran, is in `stopped`; the next ones see
    // `SEALED`, set while the lock is held.
    for listener in &listeners {
        if open.stopped.iter().any(|id| id == listener.id) {
            listener.stopped.store(true, Ordering::Release);
        }
    }
    let sealed = SEALED.get_or_init(|| Sealed {
        listeners: listeners.into(),
        slots: raw
            .into_iter()
            .map(|entries| SlotList {
                api_end: entries.partition_point(|entry| entry.kind.is_api()),
                entries: entries.into_boxed_slice(),
            })
            .collect(),
    });
    for (index, slot) in POINTS.iter().enumerate() {
        let listened = !sealed.slots[index].entries.is_empty();
        slot.store(listened, Ordering::Release);
    }
    for (index, slot) in SITES.iter().enumerate() {
        slot.store(
            !sealed.slots[POINT_COUNT + index].entries.is_empty(),
            Ordering::Release,
        );
    }
}

fn active(listener: &Owner) -> bool {
    !listener.stopped.load(Ordering::Acquire)
        && listener
            .mod_index
            .is_none_or(mod_api::__private::mod_index_on)
}

/// Invoke the next enabled site listener under its mod guard. The generated typed slot recurses
/// from the returned index, so the first listener is outermost, without allocating a chain.
pub fn wrap<O>(
    place: SiteId,
    from: usize,
    call: impl FnOnce(Option<(*const (), usize)>) -> O,
) -> O {
    if let Some(sealed) = SEALED.get() {
        for (index, raw) in sealed.slots[POINT_COUNT + place as usize]
            .entries
            .iter()
            .enumerate()
            .skip(from)
        {
            let listener = &sealed.listeners[raw.listener as usize];
            if !active(listener) {
                continue;
            }
            let Kind::Raw(RawKind::Typed(function)) = raw.kind else {
                unreachable!("only typed listeners register at sites")
            };
            let _guard = mod_api::__private::enter(listener.id, raw.injection);
            return call(Some((function, index + 1)));
        }
    }
    call(None)
}

/// Runs `call` for the mod `listener` while it runs `name`, so a panic in it names the mod; a
/// disabled or stopped mod is skipped.
fn run(sealed: &Sealed, listener: u32, name: &'static str, call: impl FnOnce()) {
    let listener = &sealed.listeners[listener as usize];
    if active(listener) {
        let _guard = mod_api::__private::enter(listener.id, name);
        call();
    }
}

/// Walk one phase of a slot's ordered entries, under their owners' guards.
fn fire(place: PointId, api: bool, call: &mut dyn FnMut(Kind)) {
    let Some(sealed) = SEALED.get() else { return };
    let slot = &sealed.slots[place as usize];
    let entries = if api {
        &slot.entries[..slot.api_end]
    } else {
        &slot.entries[slot.api_end..]
    };
    for entry in entries {
        run(sealed, entry.listener, entry.injection, || call(entry.kind));
    }
}

/// Run the API functions and forwarding endpoints at a point.
pub(crate) fn fire_api(place: PointId, call: &mut dyn FnMut(ApiKind)) {
    fire(place, true, &mut |kind| {
        if let Kind::Api(api) = kind {
            call(api);
        }
    });
}

/// Run the internals functions at a point.
pub(crate) fn fire_raw(place: PointId, _name: &'static str, call: &mut dyn FnMut(RawKind)) {
    fire(place, false, &mut |kind| {
        if let Kind::Raw(raw) = kind {
            call(raw);
        }
    });
}
