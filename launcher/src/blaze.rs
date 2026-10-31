//! Pinned Blaze binaries for opt-in native mod builds.
use crate::{backend, rust_toolchain};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

pub const VERSION: &str = "v0.20261030.1";
const RELEASE: &str = "https://github.com/indexable-inc/blaze/releases/download";
const COMPLETE: &str = ".installed";

pub struct Host {
    pub target: &'static str,
    sha256: &'static str,
}

// Both hashes match the release's SHA256SUMS and individual .sha256 assets.
const HOSTS: &[Host] = &[
    Host {
        target: "x86_64-unknown-linux-gnu",
        sha256: "4121feea06809eb5d4d6f4fb5bfe42fdd49281ed73adcfdfaa3a5d6c661c6ce4",
    },
    Host {
        target: "aarch64-apple-darwin",
        sha256: "63daccdad39bef02c3e0ef2d6ce680014aa03d8e1aba0bd34bb88776d42d1ce0",
    },
];

pub fn host() -> Option<&'static Host> {
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        _ => return None,
    };
    HOSTS.iter().find(|h| h.target == target)
}

fn root() -> PathBuf {
    backend::build_dir().join("blaze")
}

fn dir_for(host: &Host) -> PathBuf {
    root().join(format!("{VERSION}-{}", host.target))
}

pub fn installed() -> Option<PathBuf> {
    let dir = dir_for(host()?);
    (dir.join(COMPLETE).is_file() && dir.join("blaze").is_file()).then(|| dir.join("blaze"))
}

pub fn install(progress: impl Fn(f32)) -> Result<PathBuf, String> {
    if let Some(binary) = installed() {
        return Ok(binary);
    }
    let host = host().ok_or_else(|| {
        format!(
            "Blaze is not available for {} {}.",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    let dir = dir_for(host);
    let partial = root().join(format!(".{VERSION}-{}.partial", host.target));
    let _ = fs::remove_dir_all(&partial);
    fs::create_dir_all(&partial).map_err(|e| e.to_string())?;
    let file = format!("blaze-{VERSION}-{}.tar.gz", host.target);
    let archive = root().join(format!(".{file}"));
    backend::download(
        &format!("{RELEASE}/{VERSION}/{file}"),
        0,
        host.sha256,
        &archive,
        &progress,
    )
    .map_err(|e| format!("Blaze {VERSION}: {e}"))?;
    let unpacked = rust_toolchain::unpack_tar(&archive, &partial, archive_path);
    let _ = fs::remove_file(&archive);
    unpacked.map_err(|e| format!("Blaze {VERSION}: {e}"))?;
    if !partial.join("blaze").is_file() {
        return Err("Blaze archive has no executable".into());
    }
    fs::write(partial.join(COMPLETE), VERSION).map_err(|e| e.to_string())?;
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    }
    fs::rename(&partial, &dir).map_err(|e| e.to_string())?;
    progress(1.0);
    Ok(dir.join("blaze"))
}

fn archive_path(path: &Path) -> Option<PathBuf> {
    let mut parts = path.components();
    match (parts.next(), parts.next(), parts.next()) {
        (Some(Component::Normal(_)), Some(Component::Normal(name)), None) => Some(name.into()),
        _ => None,
    }
}

/// Keep Blaze's daemon, toolchain and caches apart from the player's files.
pub fn command(binary: &Path) -> Command {
    command_in(binary, &root().join("state"))
}

fn command_in(binary: &Path, base: &Path) -> Command {
    let mut cmd = Command::new(binary);
    for (name, folder) in [
        ("HOME", "home"),
        ("XDG_CACHE_HOME", "cache"),
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_STATE_HOME", "state"),
        ("XDG_RUNTIME_DIR", "runtime"),
        ("TMPDIR", "tmp"),
        ("TMP", "tmp"),
        ("TEMP", "tmp"),
        ("BLAZE_CACHE", "blaze-cache"),
        ("RUSTUP_HOME", "rustup"),
    ] {
        let dir = base.join(folder);
        let _ = fs::create_dir_all(&dir);
        cmd.env(name, dir);
    }
    cmd
}

pub fn remove() -> Result<(), String> {
    if let Some(binary) = installed() {
        let _ = command(&binary).arg("stop").status();
    }
    let dir = root();
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_are_distinct_and_complete() {
        assert_eq!(HOSTS.len(), 2);
        for host in HOSTS {
            assert_eq!(host.sha256.len(), 64);
            assert!(host.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
        }
        assert_ne!(HOSTS[0].sha256, HOSTS[1].sha256);
    }

    #[test]
    fn archive_paths_stay_under_the_prefix() {
        assert_eq!(
            archive_path(Path::new("blaze-release/blaze")),
            Some(PathBuf::from("blaze"))
        );
        for path in [
            "blaze-release",
            "blaze-release/../evil",
            "/blaze-release/blaze",
            "blaze-release/a/b",
        ] {
            assert_eq!(archive_path(Path::new(path)), None, "{path}");
        }
    }

    #[test]
    fn command_isolates_blazes_state() {
        let temp = std::env::temp_dir().join(format!("horizon-blaze-test-{}", std::process::id()));
        let cmd = command_in(Path::new("/blaze"), &temp);
        for name in [
            "HOME",
            "XDG_CACHE_HOME",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_STATE_HOME",
            "XDG_RUNTIME_DIR",
            "TMPDIR",
            "TMP",
            "TEMP",
            "BLAZE_CACHE",
            "RUSTUP_HOME",
        ] {
            let value = cmd.get_envs().find(|(k, _)| *k == name).unwrap().1.unwrap();
            assert!(Path::new(value).starts_with(&temp), "{name}");
        }
        fs::remove_dir_all(temp).unwrap();
    }
}
