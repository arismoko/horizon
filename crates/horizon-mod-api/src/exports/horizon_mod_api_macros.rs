/// Internal test-fixture attribute for modules with multiple hook sets; mods use `#[mod_api::horizon_mod]`.
#[doc(hidden)]
pub use horizon_mod_api_macros::__subscriptions;
/// Mark a hook implementation and derive its subscriptions.
///
/// On `impl Hooks`, also exports `pub fn hooks()` using the mod's `Default`
/// implementation. Put that impl at the crate root so the launcher finds it.
///
/// An implementation without the attribute does not compile:
/// ```compile_fail
/// use mod_api::Hooks;
/// struct Mod;
/// impl Hooks for Mod {}
/// ```
/// Rustc rejects a misspelled trait method (E0407):
/// ```compile_fail,E0407
/// use mod_api::{self, Hooks};
/// #[derive(Default)]
/// struct Mod;
/// #[mod_api::horizon_mod]
/// impl Hooks for Mod {
///     fn baby_entity_spwan(&self) {}
/// }
/// ```
/// Two mod markers in one module define the same export:
/// ```compile_fail,E0428
/// use mod_api::{self, Hooks};
/// #[derive(Default)] struct A;
/// #[derive(Default)] struct B;
/// #[mod_api::horizon_mod] impl Hooks for A {}
/// #[mod_api::horizon_mod] impl Hooks for B {}
/// ```
pub use horizon_mod_api_macros::{horizon_mod, horizon_part, if_installed};
