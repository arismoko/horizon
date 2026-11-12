//! The console for native mods: logged lines reach `console_line`, and a command's feedback
//! comes back from `server_run_command_captured`.
use super::native_guest_turn_tests::with_guests;
use horizon_client::logging;
use mod_api::{ConsoleLineEvent, Hooks, ServerHost};
use std::sync::{Arc, Mutex};

/// Records the probe lines it hears and logs a line of its own for each.
struct Mirror(Arc<Mutex<Vec<(String, String, String)>>>);

#[mod_api::__subscriptions]
impl Hooks for Mirror {
    fn console_line(&self, _: &mut dyn ServerHost, event: &mut ConsoleLineEvent<'_>) {
        if event.message.starts_with("console probe") {
            self.0.lock().unwrap().push((
                event.level.to_owned(),
                event.thread.to_owned(),
                event.message.to_owned(),
            ));
            log::info!("console probe echo");
        }
    }
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn logged_lines_reach_a_listener_but_not_its_own() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new("mirror", Box::new(Mirror(heard.clone())))].into();
    mod_platform::testing::with_mods(mods, || {
        with_guests(|game| {
            game.native_console_lines();
            logging::write_as("Server thread", log::Level::Warn, "console probe 1");
            logging::write_as("Server thread", log::Level::Info, "console probe 2");
            game.native_console_lines();
            game.native_console_lines();
            let heard = heard.lock().unwrap();
            assert_eq!(
                *heard,
                [
                    ("WARN".into(), "Server thread".into(), "console probe 1".into()),
                    ("INFO".into(), "Server thread".into(), "console probe 2".into()),
                ]
            );
        });
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_captured_command_returns_its_feedback() {
    with_guests(|game| {
        let (ok, lines) = game.server_run_command_captured("/time query gametime");
        assert!(ok);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].starts_with("The game time is"), "{lines:?}");
        let (ok, lines) = game.server_run_command_captured("gamerule nosuchrule");
        assert!(!ok);
        assert!(!lines.is_empty());
        assert!(game.chat_capture.is_none(), "the outer capture is put back");
    });
}
