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
///
/// Transparent over `E`: a listener's `&mut dyn ServerHost` is the game's own address, with no
/// wrapper value on the stack (`HostOf::new` is a pointer cast). The one `unsafe impl` is what
/// `bytemuck`'s derive expands to, which is why this crate is `deny(unsafe_code)` and not
/// `forbid`, with the allow on this struct alone.
#[repr(transparent)]
#[derive(bytemuck::TransparentWrapper)]
#[allow(unsafe_code)]
pub struct HostOf<E: ?Sized>(E);

impl<E> HostOf<E> {
    pub fn new(engine: &mut E) -> &mut Self {
        bytemuck::TransparentWrapper::wrap_mut(engine)
    }
}

impl<E: Legacy + Subsystems> Backend for HostOf<E> {}
impl<E: Legacy + Subsystems> BackendMut for HostOf<E> {}
impl<E: Legacy + Subsystems> ClientBackendMut for HostOf<E> {}

impl<E: Legacy + Subsystems> Host for HostOf<E> {
    fn backend(&self) -> Option<BackendRef<'_>> {
        Some(<BackendRef as BackendRefOf>::of(self))
    }
}

impl<E: Legacy + Subsystems> ServerHost for HostOf<E> {
    fn backend_mut(&mut self) -> Option<BackendMutRef<'_>> {
        Some(<BackendMutRef as BackendMutRefOf>::of(self))
    }
}

impl<E: Legacy + Subsystems> ClientHost for HostOf<E> {
    fn client_backend_mut(&mut self) -> Option<ClientBackendMutRef<'_>> {
        Some(<ClientBackendMutRef as ClientBackendMutRefOf>::of(self))
    }
}
