//! Server-side profiler controls. Export and durable filesystem work run only on stop.
use super::{R, err};
#[cfg(test)]
use mod_platform::__private::profiler;
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use sunrise_game_data::text::Text;
use sunrise_sim::save;

#[derive(Clone, Copy)]
enum Action {
    Start,
    Stop,
    Report,
}
impl crate::game::ServerGame<'_> {
    /// Run the permission-3 profiler command and reply only to its originating sender.
    pub(in crate::game::server_side) fn cmd_modprofile(&mut self, id: &str) -> R<i32> {
        let action = match id {
            "modprofile.start" => Action::Start,
            "modprofile.stop" => Action::Stop,
            "modprofile.report" => Action::Report,
            _ => return Ok(0),
        };
        let lines = run(
            action,
            self.common
                .options
                .game_dir
                .as_deref()
                .unwrap_or(Path::new(".")),
        )?;
        for line in lines {
            self.feedback_unbroadcast(line);
        }
        Ok(1)
    }
}
fn run(action: Action, game_dir: &Path) -> R<Vec<Text>> {
    #[cfg(test)]
    mod_platform::__private::test_platform::install();
    let profiler = mod_api::__private::platform::platform()
        .map_err(|e| err("sunrise.modprofile.writeFailed", &[&e.to_string()]))?;
    match action {
        Action::Start => {
            if !(profiler.profiler_start)() {
                return Err(err("sunrise.modprofile.alreadyRunning", &[]));
            }
            Ok(vec![Text::translate("sunrise.modprofile.started", &[])])
        }
        Action::Stop => {
            if !(profiler.profiler_is_enabled)() {
                return Err(err("sunrise.modprofile.notRunning", &[]));
            }
            // A host command may be invoked synchronously by a mod. Draining here would
            // wait forever for that caller's own native guard to drop.
            if mod_api::content::crash::current_native_call().is_some() {
                return Err(err("sunrise.modprofile.inHook", &[]));
            }
            (profiler.profiler_stop)();
            let bytes = (profiler.profiler_firefox_profile)()
                .map_err(|e| err("sunrise.modprofile.writeFailed", &[&e]))?;
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| err("sunrise.modprofile.writeFailed", &[&e]))?
                .as_nanos();
            let path = write_profile(game_dir, stamp, &bytes)
                .map_err(|e| err("sunrise.modprofile.writeFailed", &[&e]))?;
            Ok(vec![Text::translate(
                "sunrise.modprofile.stopped",
                &[Text::literal(path.display().to_string())],
            )])
        }
        Action::Report => {
            let summary = (profiler.profiler_summary)();
            if summary.is_empty() {
                return Err(err("sunrise.modprofile.noReport", &[]));
            }
            Ok(summary.lines().map(Text::literal).collect())
        }
    }
}
static NEXT: AtomicU64 = AtomicU64::new(0);
fn write_profile(game_dir: &Path, stamp: u128, bytes: &[u8]) -> Result<PathBuf, String> {
    let relative = PathBuf::from("profiles").join(format!(
        "modprofile-{stamp}-{}.json",
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    save::write_durable(&game_dir.join(&relative), bytes).map_err(|e| e.to_string())?;
    Ok(relative)
}

#[cfg(test)]
#[path = "modprofile_tests.rs"]
mod tests;
