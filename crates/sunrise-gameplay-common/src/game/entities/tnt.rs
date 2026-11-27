#[cfg(any(test, feature = "test-support"))]
pub mod tests {

    pub fn write_silence_output(name: &str, rows: &[String]) {
        if let Some(root) = std::env::var_os("SUNRISE_SOURCE_SILENCE_OUTPUT") {
            std::fs::write(
                std::path::Path::new(&root).join(format!("{name}.txt")),
                rows.join("\n") + "\n",
            )
            .unwrap();
        }
    }
    /// Runs `test` on a thread with an 8 MiB stack. A Game's explosion and potion-impact paths
    /// need more than the 2 MiB test thread in a Windows debug build, where those tests
    /// overflowed it (4 MiB is enough).
    pub fn on_big_stack(test: impl FnOnce() + Send + 'static) {
        let thread = std::thread::Builder::new()
            .stack_size(8 << 20)
            .spawn(test)
            .unwrap();
        if let Err(panic) = thread.join() {
            std::panic::resume_unwind(panic);
        }
    }
}
