//! The prebuilt executable of the API route, built once by the SDK (`tools/prebuilt_game.py
//! build`) and shipped in its runtime archive. Its mods come from the `horizon_native_mods`
//! library beside it: the SDK's stub (no mods), replaced by the launcher's build of the
//! instance's API-only mods (`native_mods.rs.template`). The player's machine never relinks it.
//! A mod set with a `[native]` table takes the engine route, the generated `native_main.rs.template`.
#![forbid(unsafe_code)]
static GAME_IDENTITY: [u8; horizon_mod_package::game_identity::MARK.len()] =
    horizon_mod_package::game_identity::MARK;
// The launcher's holder check reads the executable's own bytes (`holder::capable`), so the marker
// is copied into this executable rather than left in the library that defines it.
#[used]
static HOLDER_CAPABILITY: [u8; horizon_mod_package::holder::CAPABILITY.len()] =
    horizon_mod_package::holder::CAPABILITY;
fn main() {
    horizon_mod_host::prepare_main_stack();
    horizon_mod_host::exit_if_compile_child();
    let startup = horizon_client::entry::prepare();
    // No function replacement on this route.
    horizon_client::entry::installed(&[]);
    std::hint::black_box(&HOLDER_CAPABILITY);
    std::hint::black_box(&GAME_IDENTITY);
    let mods = horizon_native_mods::entries();
    let slots = horizon_native_mods::slots();
    horizon_mod_host::main_with(mods, &slots, horizon_native_mods::built(), startup);
}
