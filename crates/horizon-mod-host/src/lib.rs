//! Installed mod code loading above the engine (docs/layers.md, #2534).
//! The platform prepares the engine, loads mods here, then makes one engine start call.
#![forbid(unsafe_code)]

mod loading;
pub use loading::main_with;

pub use horizon_mod_wasm::compile_child;

/// Handles the private compiler invocation before any game startup work.
pub fn exit_if_compile_child() {
    if let Some(code) = compile_child(std::env::args_os().skip(1)) {
        std::process::exit(code);
    }
}

/// A typed API-point listener supplied by generated native startup.
pub use mod_points::__private::SlotListener;
