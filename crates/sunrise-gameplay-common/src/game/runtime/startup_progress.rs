//! Mods' bars and lines on the startup loading screen (`RegistrationContext::progress` and
//! `message`, NeoForge's `StartupNotificationManager`): kept here, drawn under the stage bar by
//! the startup menu, or logged on a dedicated server, which has no screen.
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

/// How long a mod's line stays on the screen.
const LINE_TIME: Duration = Duration::from_secs(5);
/// The most bars drawn, newest first; older ones wait for a newer one to close.
const SHOWN_BARS: usize = 4;
/// The most lines drawn, newest last.
const SHOWN_LINES: usize = 3;
/// Bars open at once per mod: past this, opening another bar for that mod returns `TooManyBars`.
const MAX_BARS_PER_MOD: usize = 64;
/// Lines kept for drawing.
const MAX_LINES: usize = 32;
/// A startup message line is cut to this many bytes when it is kept.
const MAX_TEXT: usize = 256;

struct Bar {
    label: String,
    done: u32,
    steps: u32,
    /// The tenths of the steps a dedicated server has logged.
    logged: u64,
}

struct Meter {
    screen: Arc<ModProgress>,
    bar: Arc<Mutex<Bar>>,
}

struct TrackedBar {
    owner: &'static str,
    bar: Arc<Mutex<Bar>>,
}

#[derive(Default)]
struct State {
    bars: Vec<TrackedBar>,
    lines: VecDeque<(String, Instant)>,
}

pub(crate) struct ModProgress {
    dedicated: bool,
    state: Mutex<State>,
}

static PROGRESS: OnceLock<Arc<ModProgress>> = OnceLock::new();

/// Hand mods the startup screen, before they register.
pub fn install(dedicated: bool) {
    let progress = PROGRESS.get_or_init(|| Arc::new(ModProgress::new(dedicated)));
    if let Err(error) = mod_api::__private::game::install_startup_screen(progress.clone()) {
        log::warn!("Sunrise: {error}");
    }
}

/// The bars to draw, newest first, as their label and done fraction, and the lines still shown.
pub fn shown() -> (Vec<(String, f32)>, Vec<String>) {
    PROGRESS
        .get()
        .map(|progress| progress.shown(Instant::now()))
        .unwrap_or_default()
}

/// `text` cut to [`MAX_TEXT`] bytes, on one line, without allocating.
fn one_line(text: &str) -> impl Iterator<Item = char> + Clone + '_ {
    text[..text.floor_char_boundary(MAX_TEXT)]
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
}

fn log_line(line: impl Iterator<Item = char> + Clone) {
    let line = std::fmt::from_fn(|f| {
        line.clone()
            .try_for_each(|c| std::fmt::Write::write_char(f, c))
    });
    log::info!("{line}");
}

impl ModProgress {
    fn new(dedicated: bool) -> Self {
        Self {
            dedicated,
            state: Mutex::default(),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn shown(&self, now: Instant) -> (Vec<(String, f32)>, Vec<String>) {
        let mut state = self.state();
        state
            .lines
            .retain(|(_, at)| now.saturating_duration_since(*at) < LINE_TIME);
        let bars = state
            .bars
            .iter()
            .rev()
            .take(SHOWN_BARS)
            .map(|entry| {
                let bar = entry.bar.lock().unwrap_or_else(|p| p.into_inner());
                let done = if bar.steps == 0 {
                    0.0
                } else {
                    bar.done as f32 / bar.steps as f32
                };
                (bar.label.clone(), done)
            })
            .collect();
        let old = state.lines.len().saturating_sub(SHOWN_LINES);
        let lines = state
            .lines
            .iter()
            .skip(old)
            .map(|(line, _)| line.clone())
            .collect();
        (bars, lines)
    }
}

impl mod_api::StartupScreen for ModProgress {
    fn open(
        self: Arc<Self>,
        owner: &'static str,
        label: &str,
        steps: u32,
    ) -> Result<Box<dyn mod_api::ProgressMeter>, mod_api::StartupProgressError> {
        let mut state = self.state();
        let open_for_owner = state
            .bars
            .iter()
            .filter(|entry| entry.owner == owner)
            .count();
        if open_for_owner >= MAX_BARS_PER_MOD {
            drop(state);
            log::warn!(
                "Sunrise: mod {owner} exceeded the {MAX_BARS_PER_MOD}-bar startup progress limit"
            );
            return Err(mod_api::StartupProgressError::TooManyBars);
        }
        let bar = Arc::new(Mutex::new(Bar {
            label: label.to_owned(),
            done: 0,
            steps,
            logged: 0,
        }));
        state.bars.push(TrackedBar {
            owner,
            bar: bar.clone(),
        });
        drop(state);
        if self.dedicated {
            log::info!("{label}");
        }
        Ok(Box::new(Meter { screen: self, bar }))
    }

    fn message(&self, text: &str) {
        let text = one_line(text);
        if self.dedicated {
            log_line(text);
            return;
        }
        let mut state = self.state();
        state.lines.push_back((text.collect(), Instant::now()));
        if state.lines.len() > MAX_LINES {
            state.lines.pop_front();
        }
    }
}

impl Meter {
    /// Update just this bar, without scanning or locking the screen's list.
    fn update(&self, change: impl FnOnce(u32) -> u32) {
        let mut bar = self.bar.lock().unwrap_or_else(|p| p.into_inner());
        bar.done = change(bar.done).min(bar.steps);
        if self.screen.dedicated && bar.steps > 0 {
            let tenths = u64::from(bar.done) * 10 / u64::from(bar.steps);
            if tenths > bar.logged {
                bar.logged = tenths;
                let label = bar.label.clone();
                drop(bar);
                log::info!("{label}: {}%", tenths * 10);
            }
        }
    }
}

impl mod_api::ProgressMeter for Meter {
    fn step(&self) {
        self.update(|done| done.saturating_add(1));
    }

    fn set(&self, done: u32) {
        self.update(|_| done);
    }

    fn label(&self, text: &str) {
        let mut bar = self.bar.lock().unwrap_or_else(|p| p.into_inner());
        if bar.label == text {
            return;
        }
        bar.label.clear();
        bar.label.push_str(text);
        drop(bar);
        if self.screen.dedicated {
            log::info!("{text}");
        }
    }

    fn message(&self, text: &str) {
        mod_api::StartupScreen::message(self.screen.as_ref(), text);
    }
}

impl Drop for Meter {
    fn drop(&mut self) {
        self.screen
            .state()
            .bars
            .retain(|kept| !Arc::ptr_eq(&kept.bar, &self.bar));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::StartupScreen;

    #[test]
    fn multibyte_labels_are_stored_verbatim_and_messages_stop_at_char_boundary() {
        let screen = Arc::new(ModProgress::new(false));
        let initial = format!("{}界", "é".repeat(127));
        let bar = screen.clone().open("test", &initial, 1).unwrap();
        assert_eq!(screen.shown(Instant::now()).0[0].0, initial);
        let updated = "é".repeat(129);
        bar.label(&updated);
        assert_eq!(screen.shown(Instant::now()).0[0].0, updated);
        bar.message(&initial);
        assert_eq!(screen.shown(Instant::now()).1, ["é".repeat(127)]);
    }

    #[test]
    fn sixty_fifth_bar_is_refused_per_mod_even_from_a_scoped_worker() {
        use mod_api::__private::game::RegistrationContextNew;
        use mod_api::{
            BlocksEvent, ComponentTypesEvent, Hooks, ModEntry, Registrar, RegistrationContext,
            StartupProgress, StartupProgressError,
        };

        struct OpensBars(&'static str, Arc<Mutex<Vec<StartupProgress>>>);
        impl OpensBars {
            fn open_batch(&self, context: &RegistrationContext) {
                self.1
                    .lock()
                    .unwrap()
                    .extend((0..16).map(|_| context.progress(self.0, 1).unwrap()));
            }
        }
        #[mod_api::__subscriptions]
        impl Hooks for OpensBars {
            fn register(&self, _: &mut dyn Registrar, context: &RegistrationContext) {
                self.open_batch(context);
            }

            fn blocks(&self, context: &RegistrationContext, _: &mut BlocksEvent<'_>) {
                self.open_batch(context);
            }

            fn component_types(
                &self,
                context: &RegistrationContext,
                _: &mut ComponentTypesEvent<'_>,
            ) {
                self.open_batch(context);
                std::thread::scope(|scope| {
                    scope
                        .spawn(|| {
                            self.open_batch(context);
                            assert!(matches!(
                                context.progress("worker refused", 1),
                                Err(StartupProgressError::TooManyBars)
                            ));
                        })
                        .join()
                        .unwrap();
                });
                assert!(matches!(
                    context.progress("listener refused", 1),
                    Err(StartupProgressError::TooManyBars)
                ));
                let mut bars = self.1.lock().unwrap();
                drop(bars.pop());
                bars.push(context.progress(self.0, 1).unwrap());
            }
        }

        let screen = Arc::new(ModProgress::new(false));
        let opened = Arc::new(Mutex::new(Vec::new()));
        let mods =
            ["alpha", "beta"].map(|id| ModEntry::new(id, Box::new(OpensBars(id, opened.clone()))));
        mod_api::testing::with_startup_screen(screen.clone(), || {
            // This fixture runs the real Registrar path before starting generated dispatch.
            mod_platform::testing::with_mods(Vec::from(mods).into(), || {
                let _registration = mod_platform::__private::RegistrationGuard::open();
                let context = <RegistrationContext as RegistrationContextNew>::new();
                mod_api::dispatch_blocks(&context, &mut |listen| {
                    assert!(listen(&mut |_| panic!("no blocks declared")));
                });
                mod_api::dispatch_component_types(&context, &mut Vec::new());
            });
        });
        assert_eq!(screen.state().bars.len(), 128);
        opened.lock().unwrap().clear();
        assert!(screen.shown(Instant::now()).0.is_empty());
    }

    #[test]
    fn zero_step_bar_remains_finite_and_clamped() {
        let screen = Arc::new(ModProgress::new(false));
        let bar = screen.clone().open("test", "empty", 0).unwrap();
        bar.step();
        bar.set(u32::MAX);
        assert_eq!(screen.shown(Instant::now()).0, [("empty".into(), 0.0)]);
    }

    #[test]
    fn thirty_third_message_keeps_the_newest_lines_and_expires() {
        let screen = ModProgress::new(false);
        for n in 0..33 {
            screen.message(&format!("message {n}"));
        }
        assert_eq!(screen.state().lines.len(), 32);
        let now = Instant::now();
        assert_eq!(
            screen.shown(now).1,
            ["message 30", "message 31", "message 32"]
        );
        assert!(screen.shown(now + Duration::from_secs(5)).1.is_empty());
    }

    #[test]
    fn bars_read_their_steps_newest_first_and_leave_when_dropped() {
        let screen = Arc::new(ModProgress::new(false));
        let first = screen.clone().open("test", "Baking\nmodels", 10).unwrap();
        std::thread::scope(|scope| {
            for _ in 0..2 {
                scope.spawn(|| {
                    first.step();
                    first.step();
                });
            }
        });
        let now = Instant::now;
        assert_eq!(
            screen.shown(now()).0,
            [("Baking\nmodels".to_owned(), 0.4_f32)]
        );
        first.set(99);
        assert_eq!(screen.shown(now()).0[0].1, 1.0);
        // Other bars with the same label still refer to distinct meters.
        let mut more: Vec<_> = (0..4)
            .map(|_| screen.clone().open("test", "Models", 2).unwrap())
            .collect();
        more[2].step();
        assert_eq!(
            screen.shown(now()).0,
            [
                ("Models".to_owned(), 0.0),
                ("Models".to_owned(), 0.5),
                ("Models".to_owned(), 0.0),
                ("Models".to_owned(), 0.0),
            ]
        );
        // Dropping the updated bar leaves all its namesakes and reveals the older bar.
        drop(more.remove(2));
        assert_eq!(
            screen.shown(now()).0,
            [
                ("Models".to_owned(), 0.0),
                ("Models".to_owned(), 0.0),
                ("Models".to_owned(), 0.0),
                ("Baking\nmodels".to_owned(), 1.0),
            ]
        );
        drop(more);
        drop(first);
        assert!(screen.shown(now()).0.is_empty());
    }
}
