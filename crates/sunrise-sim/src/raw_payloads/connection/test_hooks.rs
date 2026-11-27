//! The park a payload-wait test waits on: one test at a time arms it for its thread.
use std::cell::RefCell;
use std::sync::mpsc;

thread_local! {
    static ENTERED: RefCell<Option<mpsc::Sender<()>>> = const { RefCell::new(None) };
}

/// Arm the next payload wait on this thread to report that it entered.
pub(crate) fn arm(entered: mpsc::Sender<()>) {
    ENTERED.with_borrow_mut(|slot| *slot = Some(entered));
}

pub(crate) fn notify() {
    ENTERED.with_borrow_mut(|slot| {
        if let Some(entered) = slot.take() {
            let _ = entered.send(());
        }
    });
}
