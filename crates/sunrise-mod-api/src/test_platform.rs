use super::{
    install_packages, install_parts, install_registration_data, profiler, with_registration_open,
};
use mod_vocab::__private::platform::{Platform, PlatformRefusal, install_platform};
use mod_vocab::testing::engine_platform::{self, EngineTestPlatform};

/// Keep the test process initializer linked when this crate is an rlib.
pub fn keep() {
    std::hint::black_box(&INIT);
}

#[used]
#[cfg_attr(
    any(target_os = "linux", target_os = "android", target_os = "freebsd"),
    unsafe(link_section = ".init_array")
)]
#[cfg_attr(
    target_vendor = "apple",
    unsafe(link_section = "__DATA,__mod_init_func")
)]
#[cfg_attr(windows, unsafe(link_section = ".CRT$XCU"))]
static INIT: extern "C" fn() = enter;

extern "C" fn enter() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    if exe
        .parent()
        .and_then(|dir| dir.file_name())
        .is_some_and(|name| name == "deps")
    {
        install();
    }
}

pub fn install() {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        engine_platform::install(EngineTestPlatform::new(
            super::part_tests::register_game_keys,
            super::part_tests::install_parts_fixture,
            |mods, run| crate::testing::with_mods(mods, run),
        ));
        let result = install_platform(Platform {
            install_packages,
            install_registration_data,
            install_parts,
            with_registration_open,
            profiler_is_enabled: profiler::is_enabled,
            profiler_start: profiler::start,
            profiler_stop: profiler::stop,
            profiler_summary: profiler::summary,
            profiler_firefox_profile: profiler::firefox_profile,
        });
        assert!(matches!(
            result,
            Ok(()) | Err(PlatformRefusal::AlreadyInstalled)
        ));
    });
}
