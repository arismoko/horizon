use super::*;
use std::sync::Mutex;

/// The game's screen: each call the guest made.
#[derive(Default)]
struct Record(Mutex<Vec<String>>);
impl Record {
    fn push(&self, call: String) {
        self.0.lock().unwrap().push(call);
    }
}
impl mod_api::StartupScreen for Record {
    fn open(&self, label: String, steps: u32) -> u32 {
        self.push(format!("open {label} {steps}"));
        9
    }
    fn step(&self, bar: u32) {
        self.push(format!("step {bar}"));
    }
    fn set(&self, bar: u32, done: u32) {
        self.push(format!("set {bar} {done}"));
    }
    fn label(&self, bar: u32, label: String) {
        self.push(format!("label {bar} {label}"));
    }
    fn close(&self, bar: u32) {
        self.push(format!("close {bar}"));
    }
    fn message(&self, text: String) {
        self.push(format!("message {text}"));
    }
}

pub(super) fn register(cx: &mod_api::RegistrationContext) {
    let bar = cx.progress("loopback", 10);
    for _ in 0..4 {
        bar.step();
    }
    cx.message("loopback line");
}

#[test]
fn wire_loopback_a_guest_bar_reaches_the_game_screen_by_id() {
    let record = Arc::new(Record::default());
    mod_api::__private::game::install_startup_screen(record.clone()).unwrap();
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    install_counter();
    // Tests running beside this one may register too: count calls, not their order.
    let calls = record.0.lock().unwrap().clone();
    let count = |call: &str| calls.iter().filter(|made| *made == call).count();
    assert!(count("open loopback 10") >= 1, "{calls:?}");
    assert!(count("step 9") >= 4, "{calls:?}");
    assert!(count("message loopback line") >= 1, "{calls:?}");
    assert!(count("close 9") >= 1, "{calls:?}");
}
