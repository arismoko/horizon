use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};
use sunrise_perf_format::v1::Trigger;
use sunrise_sim::perf_types::{Join, StartOptions};

/// The folder `SUNRISE_PROFILE` names; captures go there instead of the game home's `reports/`.
pub const ENV_DIR: &str = "SUNRISE_PROFILE";

pub fn reports_dir() -> PathBuf {
    sunrise_sim::perf_types::reports_dir(
        std::env::var_os(ENV_DIR).as_deref(),
        &sunrise_level::logging::game_home(),
    )
}

/// The path as the player sees it: under the game home's folder when it is there.
pub fn shown(path: &Path) -> String {
    shown_in(path, &sunrise_level::logging::game_home())
}

/// `path` under `home` as `reports/profile-….hzperf`, with `/` on every platform so replies and chat read the same
/// everywhere; a path outside `home` as the platform writes it, ready to paste into its file manager.
pub fn shown_in(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(inside) => inside
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
        Err(_) => path.display().to_string(),
    }
}

pub fn reply_text<T: serde::Serialize>(reply: &T) -> String {
    serde_json::to_string(reply).unwrap_or_default()
}

/// How far the server thread has got with the side of a capture the window asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Asked for, not yet picked up between two ticks.
    Asked,
    Running,
    Ended,
    /// Refused or failed to begin; nothing of this side is in the file.
    Failed,
}

/// A [`Phase`] the window and the server thread share.
pub struct Live(AtomicU8);

impl Live {
    pub fn new() -> Arc<Self> {
        Arc::new(Self(AtomicU8::new(Phase::Asked as u8)))
    }

    pub(in crate::game) fn get(&self) -> Phase {
        match self.0.load(Ordering::Relaxed) {
            0 => Phase::Asked,
            1 => Phase::Running,
            2 => Phase::Ended,
            _ => Phase::Failed,
        }
    }

    pub fn set(&self, phase: Phase) {
        self.0.store(phase as u8, Ordering::Relaxed);
    }

    /// Whether this side is, or is about to be, writing.
    pub fn active(&self) -> bool {
        matches!(self.get(), Phase::Asked | Phase::Running)
    }

    /// Whether the file holds, or will hold, this side.
    pub fn wrote(&self) -> bool {
        self.get() != Phase::Failed
    }
}

/// Where a server capture is written.
pub enum Target {
    /// The window's file, which the window finishes.
    Join(Join),
    /// A file of its own, as a server-only capture needs.
    Own {
        options: StartOptions,
        trigger: Trigger,
        dir: PathBuf,
        viewer: Option<fn() -> Option<String>>,
    },
}
