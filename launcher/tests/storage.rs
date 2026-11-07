//! Real CLI regressions use separate launcher/game/config roots and never touch player data.
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

struct Home(PathBuf);
impl Home {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "horizon-storage-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn file(&self, relative: &str, contents: &[u8]) -> PathBuf {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        path
    }
    fn clear(&self, row: &str) -> serde_json::Value {
        let output = Command::new(env!("CARGO_BIN_EXE_horizon-launcher"))
            .args(["storage", "--clear", row])
            .current_dir(&self.0)
            .env("HORIZON_HOME", self.0.join("game"))
            .env("HORIZON_LAUNCHER_HOME", self.0.join("launcher"))
            .env("XDG_DATA_HOME", self.0.join("data"))
            .env("XDG_CONFIG_HOME", self.0.join("config"))
            .env("HORIZON_LAUNCHER_TRASH", self.0.join("trash"))
            .env("HORIZON_LAUNCHER_CLIPBOARD", self.0.join("clipboard"))
            .env("HORIZON_LAUNCHER_DESKTOP", self.0.join("desktop"))
            .env("HORIZON_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("HORIZON_NO_AUDIO", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn build_files_clear_selects_target_not_worlds_or_prebuilt_games() {
    let home = Home::new();
    let world = home.file("launcher/worlds/precious/level.dat", b"world");
    let output = home.file("launcher/native/target/fast/output", &[7; 8192]);
    let source = home.file("launcher/native/source/rev/lib.rs", b"source");
    let prebuilt = home.file("launcher/native/prebuilt/rev/game", b"prebuilt");
    let report = home.clear("build-files");
    assert_eq!(fs::read(world).unwrap(), b"world");
    assert_eq!(fs::read(prebuilt).unwrap(), b"prebuilt");
    assert!(!output.exists());
    assert!(!source.exists());
    let row = report["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["category"] == "build-files")
        .unwrap();
    assert!(row["freed_bytes"].as_u64().unwrap() > 0);
    assert!(row["remaining_bytes"].as_u64().is_some());
}

#[cfg(unix)]
#[test]
fn running_unplayed_version_is_excluded_until_its_process_exits() {
    use std::process::Stdio;
    let home = Home::new();
    home.file("launcher/versions/v0.0.1/.installed", b"");
    home.file("launcher/settings.json", br#"{"selected":"v0.0.1"}"#);
    home.file("launcher/versions/native-live/.installed", b"");
    let game = home.file("launcher/versions/native-live/game", b"running");
    // ps sees the version path while read waits on the retained pipe. No game data is needed.
    let mut process = Command::new("sh")
        .args(["-c", "read -r ignored", "storage-fixture"])
        .arg(&game)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    home.clear("spare-game-builds");
    assert!(game.exists());
    process.kill().unwrap();
    process.wait().unwrap();
    home.clear("spare-game-builds");
    assert!(!game.exists());
}
