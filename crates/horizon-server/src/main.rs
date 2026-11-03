//! Thin executable for the shared server CLI and supervisor library.
fn main() -> std::process::ExitCode {
    horizon_server::main()
}

// Keep the executable thin: the shared library owns the CLI and the supervisor.
