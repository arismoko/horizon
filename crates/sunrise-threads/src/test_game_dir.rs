//! The game folder of a test process, set before the first test runs.
//!
//! The game's folder is its working directory (saves, `config/`, `options.txt`, the `level.dat` of an
//! unnamed world, the caches), and `cargo test` starts a test binary in its package's folder, so every
//! test that builds a game would write into the crate's folder, shared by every test and every run.
//! A test binary that links [`keep`] starts in a folder of its own instead: a pre-main hook (a switch
//! while tests already run would split one test's reads and writes across two folders) moves the
//! working directory into `sunrise-client-test-game-dirs/<pid>` beside the binary, so `cargo clean`
//! removes it, and a later run drops the ones a day old.
//!
//! The hook acts only in a cargo test binary (its executable sits in a `deps` folder) that was started
//! in its package's folder, as cargo does: an example or a binary linked with the same feature, a child
//! a test starts in a folder of its own, or a binary run by hand from elsewhere, keeps its directory.
//! A relative `SUNRISE_TEST_RESOURCES` is made absolute first. Production never links this module.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// Links the hook into a test binary: a crate that wants it keeps this function alive
/// (`#[used] static KEEP: fn() = sunrise_threads::test_game_dir::keep;`), because a linker drops the
/// object file of a library that nothing references, and with it the hook.
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
    configure_test_allocator();
    if let Err(error) = move_into_own_folder() {
        eprintln!("test game folder: staying in the working directory: {error}");
    }
}

/// Match production's mimalloc purge and THP settings in test binaries unless overridden.
fn configure_test_allocator() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    if !is_test_binary(&exe) {
        return;
    }
    let is_threads_test = exe
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("sunrise_threads-"));
    if is_threads_test || std::env::var_os("MIMALLOC_PURGE_DELAY").is_some() {
        return;
    }
    crate::configure_allocator();
}

/// Whether `exe` is a cargo test harness binary: those, and only those, are built into `deps`
/// (examples go to `examples`, the package's own binaries beside `deps`).
fn is_test_binary(exe: &std::path::Path) -> bool {
    exe.parent()
        .and_then(|folder| folder.file_name())
        .is_some_and(|name| name == "deps")
}

fn move_into_own_folder() -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    if !is_test_binary(&exe) {
        return Ok(());
    }
    let cwd = std::env::current_dir()?;
    let Some(manifest) = std::env::var_os("CARGO_MANIFEST_DIR") else {
        return Ok(());
    };
    if std::fs::canonicalize(manifest)? != std::fs::canonicalize(&cwd)? {
        return Ok(());
    }
    if let Some(resources) = std::env::var_os("SUNRISE_TEST_RESOURCES") {
        let resources = PathBuf::from(resources);
        if resources.is_relative() {
            // SAFETY: this runs before `main`, while the process has a single thread.
            unsafe { std::env::set_var("SUNRISE_TEST_RESOURCES", cwd.join(resources)) };
        }
    }
    let parent = exe.with_file_name("sunrise-client-test-game-dirs");
    let day_ago = SystemTime::now() - Duration::from_secs(24 * 60 * 60);
    for old in std::fs::read_dir(&parent).into_iter().flatten().flatten() {
        if old
            .metadata()
            .and_then(|meta| meta.modified())
            .is_ok_and(|at| at < day_ago)
        {
            let _ = std::fs::remove_dir_all(old.path());
        }
    }
    let folder = parent.join(std::process::id().to_string());
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder)?;
    std::env::set_current_dir(folder)
}

#[cfg(test)]
mod tests {
    use super::is_test_binary;
    use std::path::Path;

    #[test]
    fn only_a_binary_in_a_deps_folder_is_a_test_binary() {
        assert!(is_test_binary(Path::new(
            "/work/target/debug/deps/sunrise_client-79655a9fd47b6a15"
        )));
        assert!(!is_test_binary(Path::new(
            "/work/target/debug/examples/old_world_fixture"
        )));
        assert!(!is_test_binary(Path::new(
            "/work/target/debug/sunrise-client"
        )));
        assert!(!is_test_binary(Path::new("sunrise-client")));
    }
}
