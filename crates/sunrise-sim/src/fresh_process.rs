//! Fixtures that need a process of their own (process-wide registries, a fresh worldgen table)
//! run one child per case through this helper, which also checks that the child ran its test.
use std::process::Command;

/// Run the ignored-or-not test `name` alone in a new process with `flag` set to `case`, and fail
/// unless that child exited cleanly and ran exactly one test (`--exact` with a name no test has
/// runs nothing and still exits 0).
pub(crate) fn run_in_fresh_process(name: &str, flag: &str, case: &str) {
    let out = Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--include-ignored", "--test-threads=1"])
        .env(flag, case)
        .output()
        .unwrap();
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success()
            && report
                .lines()
                .any(|line| line.starts_with("test result: ok. 1 passed;")),
        "{name} ({case}) failed in its own process:\n{report}"
    );
}
