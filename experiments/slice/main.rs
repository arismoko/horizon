//! The prebuilt executable's main for the slice route: the package's native_main.rs.template with the
//! mod list, slots and inventory taken from the replaceable `horizon_native_mods` library instead of
//! generated rows. Built once by the SDK against the stub; the player's machine never relinks it.
#![forbid(unsafe_code)]
static GAME_IDENTITY: [u8; horizon_mod_package::game_identity::MARK.len()] =
    horizon_mod_package::game_identity::MARK;
fn main() {
    horizon_mod_host::prepare_main_stack();
    horizon_mod_host::exit_if_compile_child();
    let startup = horizon_client::entry::prepare();
    // No function replacement on this route: a mod set with a [native] table takes today's route.
    horizon_client::entry::installed(&[]);
    std::hint::black_box(&horizon_mod_package::holder::CAPABILITY);
    std::hint::black_box(&GAME_IDENTITY);
    let built: &'static [horizon_mod_package::runtime::BuiltMod] = Vec::leak(
        horizon_native_mods::built()
            .into_iter()
            .map(|(id, version, side)| horizon_mod_package::runtime::BuiltMod {
                id,
                version,
                side: match side {
                    "Client" => horizon_mod_package::Side::Client,
                    _ => horizon_mod_package::Side::Both,
                },
            })
            .collect(),
    );
    let mods: Vec<mod_api::ModEntry> = horizon_native_mods::entries();
    let slots = horizon_native_mods::slots();
    // Experiment evidence only: what the replaceable library handed the executable.
    eprintln!(
        "slice: mods library gave {} entries, {} slots, {} inventory rows",
        mods.len(),
        slots.len(),
        built.len()
    );
    horizon_mod_host::main_with(mods, &slots, built, startup);
}
