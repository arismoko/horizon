//! The host a listener is handed: the game's subsystems behind the `Host`, `ServerHost` and
//! `ClientHost` traits, built by the code above the game for one call.
use engine_access::Subsystems;
use mod_api::__private::game::{BackendMutRefOf, BackendRefOf, ClientBackendMutRefOf};
use mod_api::{
    Backend, BackendMut, BackendMutRef, BackendRef, ClientBackendMut, ClientBackendMutRef,
    ClientHost, Host, ServerHost,
};

mod host_delegate;

/// An engine that still answers the host methods this crate has not taken over: its own impls of
/// the three hosts and the three hidden backends. A full version has no such bound.
pub trait Legacy: ServerHost + ClientHost + Backend + BackendMut + ClientBackendMut {}
impl<T: ServerHost + ClientHost + Backend + BackendMut + ClientBackendMut> Legacy for T {}

/// The host around an engine `E` for one call. `E` is the game, named only by the crate that
/// builds this (above the game); the host methods that moved here run against `E`'s subsystem
/// traits, the rest delegate to `E`'s own impl.
pub struct HostOf<'a, E>(&'a mut E);

impl<'a, E> HostOf<'a, E> {
    pub fn new(engine: &'a mut E) -> Self {
        Self(engine)
    }
}

impl<E: Legacy + Subsystems> Backend for HostOf<'_, E> {}
impl<E: Legacy + Subsystems> BackendMut for HostOf<'_, E> {}
impl<E: Legacy + Subsystems> ClientBackendMut for HostOf<'_, E> {}

impl<E: Legacy + Subsystems> Host for HostOf<'_, E> {
    fn backend(&self) -> Option<BackendRef<'_>> {
        Some(<BackendRef as BackendRefOf>::of(self))
    }
}

impl<E: Legacy + Subsystems> ServerHost for HostOf<'_, E> {
    fn backend_mut(&mut self) -> Option<BackendMutRef<'_>> {
        Some(<BackendMutRef as BackendMutRefOf>::of(self))
    }
}

impl<E: Legacy + Subsystems> ClientHost for HostOf<'_, E> {
    fn client_backend_mut(&mut self) -> Option<ClientBackendMutRef<'_>> {
        Some(<ClientBackendMutRef as ClientBackendMutRefOf>::of(self))
    }
}
