//! Host backends: what the game answers the API with.
use crate::world::random::with_random_draw;
use crate::*;

/// Why a fake player's `perform_action` did nothing at all: it was refused before any of the
/// action ran.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ActionRefused {
    /// The handle is a real player's.
    NotFake,
    /// The handle is stale, or its fake is gone.
    Stale,
    /// Another player's turn, an open transaction, or another `perform_action` is running.
    /// A `level_tick_pre` or `level_tick_post` listener's own failure boundary is not an open
    /// transaction here: the action runs outside it, so the listener's panic doesn't undo the
    /// action. That panic still rolls back the listener's own writes, and a write it made
    /// before the action, at a place the action then changed, is put back over the action's
    /// result there. A `host.transaction` the listener opens is an open transaction.
    Busy,
    /// The block or entity is out of the player's reach, or the hit isn't on the block
    /// (`ServerGamePacketListenerImpl`'s `handleUseItemOn`, `handleInteract` and
    /// `handleBlockBreakAction` checks).
    OutOfReach,
    /// The block or entity is above or below the world, under spawn protection or outside the
    /// world border (`ServerLevel.mayInteract`): nothing is done there.
    Protected,
    /// The scoped setup's eye position, rotation or entity hit is not finite.
    InvalidSetup,
    /// The action's target isn't an entity in the fake's level, or is the fake itself
    /// (`handleInteract`'s "Attempting to attack an invalid entity").
    InvalidTarget,
    /// The setup's held stack was refused before swapping player state or running the action.
    InvalidStack(StackWriteError),
}

/// An error a handle method answers when its host has no game behind it (a mod's test double):
/// what a `Result`-returning helper refuses with, as `None` and `false` are for the others.
pub trait NoBackend {
    /// The error for a host without a game.
    fn no_backend() -> Self;

    /// The error for a call made on the side that does not have it.
    fn wrong_side() -> Self
    where
        Self: Sized,
    {
        Self::no_backend()
    }
}

impl NoBackend for ActionRefused {
    /// No game: as for a handle whose fake is gone.
    fn no_backend() -> Self {
        Self::Stale
    }
}

/// What a fake player's action did once it ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ActionOutcome {
    /// Vanilla's `InteractionResult` consumed (a success or a consume), and no effect was
    /// blocked.
    Consumed,
    /// Nothing consumed the click: vanilla passed or failed, or a mod stopped the action.
    Passed,
    /// The click ran (the swing, the hooks before the use, vanilla's `Success`), but the effect
    /// it would start was stopped, as a fake can't take part in it.
    EffectBlocked(FakeEffect),
}

// no wire: borrowed host or registrar backend; access stays scoped to the owning call.
/// The game's read-only native backend, as [`Host::backend`] hands it out.
///
/// The backend is reached only through [`Host::backend`] and
/// `Host::backend_mut`, which return opaque `BackendRef`/`BackendMutRef`
/// values that only `mod_api` can build or open. A foreign [`Host`] gets
/// `None` by default, and forwarding a real host's value adds nothing beyond
/// calling with the real host. A handle asked on a host without a backend
/// answers its typed refusal (`None` or `false`).
///
/// Opening one is reserved for this API:
///
/// ```compile_fail
/// use mod_api::{Backend, BackendRef};
/// fn reach(backend: BackendRef<'_>) { let _: &dyn Backend = backend.open(); }
/// ```
///
/// So is reading the backend it holds:
///
/// ```compile_fail
/// use mod_api::BackendRef;
/// fn reach(backend: BackendRef<'_>) { let _ = backend.inner; }
/// ```
// hidden: a game-side type, not part of the mod surface
#[doc(hidden)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct BackendRef<'a> {
    pub(crate) inner: &'a dyn Backend,
}

// no wire: borrowed host or registrar backend; access stays scoped to the owning call.
/// The game's mutable native backend, as `Host::backend_mut` hands it out;
/// the same opaque rule as [`BackendRef`], and a wrapper exposes no backend
/// method of its own:
///
/// ```compile_fail
/// use mod_api::BackendMutRef;
/// fn reach(backend: BackendMutRef<'_>) { backend.set_loaded(false); }
/// ```
// hidden: a game-side type, not part of the mod surface
#[doc(hidden)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct BackendMutRef<'a> {
    pub(crate) inner: &'a mut dyn BackendMut,
}

// no wire: borrowed host or registrar backend; access stays scoped to the owning call.
/// The opaque client-write backend, with no access to server writes.
///
/// Only the API can construct or open this value.
// hidden: a game-side type, not part of the mod surface
#[doc(hidden)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ClientBackendMutRef<'a> {
    pub(crate) inner: &'a mut dyn ClientBackendMut,
}

impl<'a> ClientBackendMutRef<'a> {
    pub(crate) fn of(inner: &'a mut dyn ClientBackendMut) -> Self {
        Self { inner }
    }

    pub(crate) fn open(self) -> &'a mut dyn ClientBackendMut {
        self.inner
    }
}

impl<'a> BackendRef<'a> {
    /// Wraps a read backend; only the generated game impl and this module's
    /// test doubles build a wrapper, always around themselves.
    pub(crate) fn of(inner: &'a dyn Backend) -> Self {
        Self { inner }
    }

    /// The backend inside; visible only inside `mod_api`, which the
    /// generated handle code and the capability tables are part of.
    pub(crate) fn open(self) -> &'a dyn Backend {
        self.inner
    }
}

impl<'a> BackendMutRef<'a> {
    /// Wraps a write backend; only the generated game impl and this module's
    /// test doubles build a wrapper, always around themselves.
    pub(crate) fn of(inner: &'a mut dyn BackendMut) -> Self {
        Self { inner }
    }

    /// The backend inside; visible only inside `mod_api`.
    pub(crate) fn open(self) -> &'a mut dyn BackendMut {
        self.inner
    }
}

/// The read backend behind `host`, or `None` where the host has none; the
/// caller answers its typed refusal instead of running on.
// hidden: a game-side helper, not part of the mod authoring API
#[doc(hidden)]
pub fn backend_of(host: &dyn Host) -> Option<&dyn Backend> {
    host.backend().map(|wrapper| wrapper.open())
}

/// The logical side on which a hook is executing.
///
/// A mutable client hook can use client services:
/// ```
/// use mod_api::{self, ClientHost, Hooks, InputKeyEvent};
/// #[derive(Default)]
/// struct ScreenMod;
/// #[mod_api::sunrise_mod]
/// impl Hooks for ScreenMod {
///     fn input_key(&self, host: &mut dyn ClientHost, _: &mut InputKeyEvent<'_>) {
///         host.mod_screen_open("example");
///     }
/// }
/// ```
///
/// A hook that fires on both sides matches on the two:
/// ```
/// use mod_api::HostSide;
/// fn side(host: HostSide<'_>) -> &'static str {
///     match host { HostSide::Server(_) => "server", HostSide::Client(_) => "client" }
/// }
/// ```
///
/// Wrong-side services do not compile:
/// ```compile_fail
/// use mod_api::Host;
/// fn wrong(host: &dyn Host) { host.messages().broadcast(todo!()); }
/// ```
/// ```compile_fail
/// use mod_api::LevelHost;
/// fn wrong(host: &dyn LevelHost) { host.players().server_players(); }
/// ```
/// ```compile_fail
/// use mod_api::ClientHost;
/// fn wrong(host: &mut dyn ClientHost) { host.commands().run_command("list"); }
/// ```
/// ```compile_fail
/// use mod_api::ServerHost;
/// fn wrong(host: &mut dyn ServerHost) { host.mod_screen_open("example"); }
/// ```
///
/// A read-only hook receives only the base host, which cannot be used for a
/// server or client operation, or opened as a backend by a mod:
/// ```compile_fail
/// use mod_api::{Host, ServerHost};
/// fn server(_: &mut dyn ServerHost) {}
/// fn read(host: &dyn Host) { server(host); }
/// ```
/// ```compile_fail
/// use mod_api::{ClientHost, Host};
/// fn client(_: &dyn ClientHost) {}
/// fn read(host: &dyn Host) { client(host); }
/// ```
/// ```compile_fail
/// use mod_api::{Host, ServerHost};
/// fn read(host: &dyn Host) { let _ = host.downcast_ref::<dyn ServerHost>(); }
/// ```
impl HostSide<'_> {
    /// Borrows this side's host again for a shorter call.
    pub fn reborrow(&mut self) -> HostSide<'_> {
        match self {
            Self::Server(host) => HostSide::Server(&mut **host),
            Self::Client(host) => HostSide::Client(&mut **host),
        }
    }
}

/// The write backend behind `host`, or `None` where the host has none; the
/// caller answers its typed refusal instead of running on.
// hidden: a game-side helper, not part of the mod authoring API
#[doc(hidden)]
pub fn backend_mut_of(host: &mut dyn ServerHost) -> Option<&mut dyn BackendMut> {
    host.backend_mut().map(|wrapper| wrapper.open())
}

// hidden: a game-side helper, not part of the mod authoring API
#[doc(hidden)]
pub fn client_backend_mut_of(host: &mut dyn ClientHost) -> Option<&mut dyn ClientBackendMut> {
    host.client_backend_mut().map(|wrapper| wrapper.open())
}

/// A host is the store of its holders' attachments, so `key.get(host, holder, read)` reads one.
macro_rules! host_attachment_store {
    ($($host:ident),+) => { $(
        impl AttachmentStore for dyn $host + '_ {
            fn read_only(&mut self, holder: Holder) -> AttachmentResult<bool> {
                self.attachments().read_only(holder)
            }
            fn holds(&mut self, holder: Holder, key: &AttachmentStoreKey<'_>) -> AttachmentResult<bool> {
                self.attachments().holds(holder, key)
            }
            fn lend_decoded(&mut self, holder: Holder, key: &AttachmentStoreKey<'_>, read: AttachmentLoan<'_>) -> AttachmentResult<()> {
                self.attachments().lend_decoded(holder, key, read)
            }
            fn binding(&mut self, holder: Holder, key: &AttachmentStoreKey<'_>) -> AttachmentResult<Option<AttachmentBinding>> {
                self.attachments().binding(holder, key)
            }
            fn store(&mut self, holder: Holder, write: AttachmentStoreWrite<'_>) -> AttachmentResult<()> {
                self.attachments().store(holder, write)
            }
            fn discard(&mut self, holder: Holder, key: &AttachmentStoreKey<'_>) -> AttachmentResult<()> {
                self.attachments().discard(holder, key)
            }
            fn queue_sync(&mut self, holder: Holder, key: &AttachmentStoreKey<'_>) -> AttachmentResult<()> {
                self.attachments().queue_sync(holder, key)
            }
        }
    )+ };
}
host_attachment_store!(
    Host,
    ServerHost,
    ClientHost,
    LevelRandomHost,
    HitHost,
    LevelWriteHost
);

macro_rules! level_random_draws {
    ($($host:ident),+) => { $(
        impl dyn $host + '_ {
            /// Borrow the hook's level stream (`Level.getRandom`) for the closure only:
            /// `host.with_random(|rng| rng.next_int_bound(bound))`. Returns `None` without
            /// invoking `draw` if the host does not lend an authoritative level stream.
            ///
            /// Draws advance the live stream directly in vanilla's order and are not rolled
            /// back on panic/unwind or transaction abort, matching vanilla `Level.getRandom()`.
            pub fn with_random<T>(&mut self, draw: impl FnOnce(&mut dyn RandomSource) -> T) -> Option<T> {
                with_random_draw(draw, |draw| self.lend_random(draw))
            }
        }
    )+ };
}
level_random_draws!(LevelRandomHost, HitHost, LevelWriteHost);
