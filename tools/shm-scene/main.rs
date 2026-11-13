include!("/box/work/shm-machine-scene/src/lib.rs");

struct EarlyLog;
impl log::Log for EarlyLog {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        eprintln!("EARLY {}: {}", record.level(), record.args());
    }
    fn flush(&self) {}
}
static EARLY_LOG: EarlyLog = EarlyLog;
fn main() {
    horizon_mod_host::exit_if_compile_child();
    let startup = horizon_client::entry::prepare();
    log::set_logger(&EARLY_LOG).unwrap();
    log::set_max_level(log::LevelFilter::Warn);
    if std::env::var_os("BENCH_NATIVE").is_some() {
        static BUILT: &[mod_api::__private::native::BuiltMod] =
            &[mod_api::__private::native::BuiltMod {
                id: "machine_sample",
                version: "0.1.0",
                side: mod_api::Side::Both,
            }];
        horizon_mod_host::main_with(
            vec![mod_api::ModEntry::new("machine_sample", hooks())],
            &[],
            BUILT,
            startup,
        );
    } else {
        horizon_mod_host::main_with(Vec::new(), &[], &[], startup);
    }
}
