//! The delayed tasks mods scheduled in the running world (`schedule_task`): a queue counted in
//! server ticks, owned by the world's lifecycle and dropped with it.

use mod_api::{
    __private::game::{CallbackOwner, allocate_task_id},
    MAX_PENDING_TASKS, ScheduleError, ServerHost, TaskId,
};
use std::{
    collections::{BTreeMap, HashMap},
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
};

pub type ServerTask = Box<dyn FnOnce(&mut dyn ServerHost)>;
/// The owning mod's `[mod] id` (when scheduled inside a mod call) and its callable, detached from
/// the scheduler.
pub(in crate::game) type PendingTask = (Option<&'static str>, ServerTask);

pub fn dispose_pending_task((owner, task): PendingTask) {
    match owner {
        Some(id) => CallbackOwner::dispose_scheduled_task(id, task),
        None => CallbackOwner::call_engine(|| drop(task)),
    }
}

/// Runs every item `next` yields through `run`, even when one run panics: the other items still
/// run, and the first panic comes back for the caller to put back in flight once it has restored
/// its own state. `state` is passed to both so each may borrow the whole of it in turn.
pub fn run_all<S, T>(
    state: &mut S,
    mut next: impl FnMut(&mut S) -> Option<T>,
    mut run: impl FnMut(&mut S, T),
) -> Option<Box<dyn std::any::Any + Send>> {
    let mut panic = None;
    while let Some(item) = next(state) {
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| run(state, item))) {
            if panic.is_none() {
                panic = Some(payload);
            } else if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(payload))) {
                // A later panic's payload may itself panic on drop. Keep draining, and never
                // run another destructor on behalf of a payload we aren't going to report.
                std::mem::forget(payload);
            }
        }
    }
    panic
}

// Keep formatting macros out of the function-replacement lifecycle source.
pub fn report_failure(mod_id: &str, panic: &str) {
    log::error!(
        "mod `{mod_id}` panicked in a scheduled task; its tasks are disabled for this session: {panic}"
    );
}

#[derive(Default)]
pub struct ScheduledTasks {
    /// Ticks advanced since the world opened.
    pub(in crate::game) now: u64,
    /// Waiting tasks by due tick, then id (the order they were scheduled).
    pub(in crate::game) queue: BTreeMap<(u64, TaskId), PendingTask>,
    /// Each waiting task's due tick, to find it again on cancel.
    pub(in crate::game) due: HashMap<TaskId, u64>,
    /// How many tasks each named mod has waiting.
    pub(in crate::game) pending: HashMap<&'static str, usize>,
}

impl ScheduledTasks {
    /// A server tick starts: "now" moves to it before any hook of the tick runs, so tasks
    /// scheduled during this tick are measured from it (`delay_ticks == 0` is due in this tick).
    pub(in crate::game) fn begin_tick(&mut self) {
        self.now += 1;
    }

    /// A task for `owner` due `delay_ticks` ticks from now (`0` is due in the current tick).
    pub fn schedule(
        &mut self,
        owner: Option<&'static str>,
        delay_ticks: u32,
        task: ServerTask,
    ) -> Result<TaskId, ScheduleError> {
        // Until admission succeeds, captures belong to the caller. In particular, a
        // refusal must drop them under the retained mod callback, not the engine scope.
        let mut task = Some(task);
        CallbackOwner::call_host(|| {
            let waiting = if let Some(mod_id) = owner {
                let count = self.pending.entry(mod_id).or_insert(0);
                if *count >= MAX_PENDING_TASKS {
                    log::warn!(
                        "mod `{mod_id}` reached MAX_PENDING_TASKS ({MAX_PENDING_TASKS}) pending scheduled tasks"
                    );
                    return Err(ScheduleError::TooMany);
                }
                Some(count)
            } else {
                None
            };
            let id = allocate_task_id().ok_or(ScheduleError::Unavailable)?;
            if let Some(count) = waiting {
                *count += 1;
            }
            let due = self.now.saturating_add(u64::from(delay_ticks));
            self.queue
                .insert((due, id), (owner, task.take().expect("new task")));
            self.due.insert(id, due);
            Ok(id)
        })
    }

    /// Withdraw `id` if it is still waiting. `TaskId` is an opaque capability, so holding it
    /// suffices to cancel the task. The caller gets the detached owner and callable back only once
    /// all bookkeeping is updated, so captures can be disposed under the scheduling owner's policy.
    pub fn cancel(&mut self, id: TaskId) -> Option<PendingTask> {
        let &due = self.due.get(&id)?;
        self.remove(due, id)
    }

    /// The next task due by now, taken off the queue before any of its mod code runs.
    pub fn pop_due(&mut self) -> Option<PendingTask> {
        let (&(due, id), _) = self.queue.first_key_value()?;
        if due > self.now {
            return None;
        }
        self.remove(due, id)
    }

    /// The world closed or opened: detach the old queue before running any capture destructors.
    /// Globally unique task ids are never reset, so retained ids cannot alias tasks in a new world.
    pub(in crate::game) fn clear(&mut self) {
        let mut retired = std::mem::take(self).queue.into_values();
        let panic = run_all(&mut retired, Iterator::next, |_, task| {
            dispose_pending_task(task)
        });
        if let Some(payload) = panic {
            resume_unwind(payload);
        }
    }

    pub(in crate::game) fn remove(&mut self, due: u64, id: TaskId) -> Option<PendingTask> {
        let task = self.queue.remove(&(due, id))?;
        self.due.remove(&id);
        if let Some(mod_id) = task.0
            && let Some(waiting) = self.pending.get_mut(mod_id)
        {
            *waiting -= 1;
        }
        Some(task)
    }
}

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;
    use mod_api::testing::FakeHost;
    use std::{cell::RefCell, rc::Rc};

    const A: Option<&str> = Some("a");
    const B: Option<&str> = Some("b");

    struct PanicOnDrop;
    impl Drop for PanicOnDrop {
        fn drop(&mut self) {
            panic!("destructor panicked");
        }
    }

    fn advance(tasks: &mut ScheduledTasks) -> Vec<PendingTask> {
        tasks.begin_tick();
        std::iter::from_fn(|| tasks.pop_due()).collect()
    }

    #[test]
    fn tasks_run_after_their_delay_in_the_order_they_were_scheduled() {
        let mut tasks = ScheduledTasks::default();
        let heard = Rc::new(RefCell::new(Vec::new()));
        for (owner, delay, value) in [(A, 2, 20), (A, 1, 10), (B, 2, 21), (B, 0, 0)] {
            let heard = Rc::clone(&heard);
            tasks
                .schedule(
                    owner,
                    delay,
                    Box::new(move |_| heard.borrow_mut().push(value)),
                )
                .unwrap();
        }
        let mut host = FakeHost::new();
        // delay_ticks == 0 is due at self.now (the current tick) without advancing.
        for (_, task) in std::iter::from_fn(|| tasks.pop_due()) {
            task(&mut host);
        }
        assert_eq!(*heard.borrow(), [0]);
        for (_, task) in advance(&mut tasks) {
            task(&mut host);
        }
        assert_eq!(*heard.borrow(), [0, 10]);
        for (_, task) in advance(&mut tasks) {
            task(&mut host);
        }
        assert_eq!(*heard.borrow(), [0, 10, 20, 21]);
        assert!(advance(&mut tasks).is_empty());
    }

    #[test]
    fn cancelling_a_waiting_task_uses_only_its_opaque_id() {
        let mut tasks = ScheduledTasks::default();
        let capture = Rc::new(());
        let owned = Rc::clone(&capture);
        let id = tasks
            .schedule(A, 3, Box::new(move |_| drop(owned)))
            .unwrap();
        assert_eq!(Rc::strong_count(&capture), 2);
        let unused_id = allocate_task_id().unwrap();
        assert!(tasks.cancel(unused_id).is_none());
        let (owner, detached) = tasks.cancel(id).unwrap();
        assert_eq!(owner, A);
        drop(detached);
        assert_eq!(Rc::strong_count(&capture), 1);
        assert!(tasks.cancel(id).is_none());
        for _ in 0..3 {
            assert!(advance(&mut tasks).is_empty());
        }
        let fired = tasks.schedule(A, 1, Box::new(|_| {})).unwrap();
        assert_eq!(advance(&mut tasks).len(), 1);
        assert!(tasks.cancel(fired).is_none());
    }

    #[test]
    fn unattributed_tasks_are_uncapped_and_each_mod_is_capped_independently() {
        let mut tasks = ScheduledTasks::default();
        let unattributed_first = tasks.schedule(None, 5, Box::new(|_| {})).unwrap();
        for _ in 0..MAX_PENDING_TASKS {
            tasks.schedule(None, 5, Box::new(|_| {})).unwrap();
        }
        assert!(tasks.cancel(unattributed_first).is_some());

        let first = tasks.schedule(A, 5, Box::new(|_| {})).unwrap();
        for _ in 1..MAX_PENDING_TASKS {
            tasks.schedule(A, 5, Box::new(|_| {})).unwrap();
        }
        assert_eq!(
            tasks.schedule(A, 5, Box::new(|_| {})),
            Err(ScheduleError::TooMany)
        );
        assert!(tasks.schedule(B, 5, Box::new(|_| {})).is_ok());
        assert!(tasks.cancel(first).is_some());
        assert!(tasks.schedule(A, 5, Box::new(|_| {})).is_ok());
        for _ in 0..5 {
            advance(&mut tasks);
        }
        assert!(tasks.schedule(A, 1, Box::new(|_| {})).is_ok());
        assert!(tasks.schedule(None, 0, Box::new(|_| {})).is_ok());
    }

    #[test]
    fn pop_due_drains_same_tick_delay_zero_chains_to_exhaustion() {
        let mut tasks = ScheduledTasks::default();
        tasks.begin_tick();
        tasks.schedule(A, 0, Box::new(|_| {})).unwrap();
        assert!(tasks.pop_due().is_some());
        // A delay-0 task scheduled during the drain runs in the same drain without advancing now.
        tasks.schedule(A, 0, Box::new(|_| {})).unwrap();
        assert!(tasks.pop_due().is_some());
        assert!(tasks.pop_due().is_none());
    }

    #[test]
    fn the_largest_delay_does_not_wrap() {
        let mut tasks = ScheduledTasks::default();
        tasks.schedule(A, u32::MAX, Box::new(|_| {})).unwrap();
        assert!(advance(&mut tasks).is_empty());
    }

    #[test]
    fn a_panicking_task_does_not_drop_the_others() {
        let mut state = (ScheduledTasks::default(), FakeHost::new());
        state
            .0
            .schedule(A, 1, Box::new(|_| panic!("first")))
            .unwrap();
        state
            .0
            .schedule(A, 1, Box::new(|_| std::panic::panic_any(PanicOnDrop)))
            .unwrap();
        let heard = Rc::new(RefCell::new(Vec::new()));
        let log = Rc::clone(&heard);
        state
            .0
            .schedule(B, 1, Box::new(move |_| log.borrow_mut().push("third")))
            .unwrap();
        state.0.begin_tick();
        let panic = run_all(&mut state, |s| s.0.pop_due(), |s, (_, task)| task(&mut s.1)).unwrap();
        assert_eq!(panic.downcast_ref::<&str>(), Some(&"first"));
        assert_eq!(*heard.borrow(), ["third"]);
        assert!(state.0.pop_due().is_none());
    }
}
