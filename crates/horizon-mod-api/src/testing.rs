//! Test support with API-owned registration and vocabulary-owned fakes.
pub use mod_vocab::testing::*;

/// Run with these mods and their controller and pose registrations on this thread.
/// Nested scopes and unwinding restore the preceding immutable lookup tables.
pub fn with_mods<R>(mods: crate::Mods, f: impl FnOnce() -> R) -> R {
    crate::__private::initialize().expect("test registration requires the mod API runtime");
    let (controllers, poses) = crate::parts::registered_controllers(&mods);
    mod_vocab::testing::with_controller_tables(controllers, poses, || {
        mod_vocab::testing::with_dispatch(mods, f)
    })
}
