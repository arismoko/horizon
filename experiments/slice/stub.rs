//! The SDK's stub `horizon_native_mods`: no mods. Built once with the game and linked into the prebuilt
//! executable; the launcher replaces the library file with one built from the API slice.
// Takes mod_api from the game dylib, so this library imports it rather than linking a copy.
extern crate horizon_mod_host as _;

#[inline(never)]
pub fn entries() -> Vec<mod_api::ModEntry> {
    std::hint::black_box(Vec::new())
}

#[inline(never)]
pub fn slots() -> Vec<mod_api::__private::points::SlotListener> {
    std::hint::black_box(Vec::new())
}

/// (id, version, side) per mod: `BuiltMod` lives in horizon_mod_package, outside the API slice.
#[inline(never)]
pub fn built() -> Vec<(&'static str, &'static str, &'static str)> {
    std::hint::black_box(Vec::new())
}
