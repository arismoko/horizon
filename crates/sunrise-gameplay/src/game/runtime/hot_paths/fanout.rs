//! Fan-out over native dispatch, with one counted call per subscribed mod.
use super::Recorder;
use std::{
    cell::{Cell, OnceCell},
    num::NonZeroU16,
    sync::LazyLock,
};
use sunrise_game_data::Result;

const CALLS: u64 = 100_000;

thread_local! {
    // Allocate the slots before measuring on the server thread. Outside the measured fixture
    // an incidental call on another thread has nothing to count.
    static LISTENER_CALLS: OnceCell<Box<[Cell<u64>]>> = const { OnceCell::new() };
}

/// Record a call to one independently registered listener without allocating or locking.
#[doc(hidden)]
pub fn note_hot_paths_listener_call(slot: usize) {
    LISTENER_CALLS.with(|calls| {
        if let Some(calls) = calls.get() {
            let count = &calls[slot];
            count.set(count.get() + 1);
        }
    });
}

/// Which native benchmark listeners the executable is asked to install for this process.
#[doc(hidden)]
#[derive(Clone, Copy)]
pub enum HotPathsMods {
    None,
    Native,
    Fanout(NonZeroU16),
}

impl HotPathsMods {
    pub fn from_env() -> Result<Self> {
        static MODE: LazyLock<std::result::Result<HotPathsMods, String>> = LazyLock::new(|| {
            let mods = std::env::var("SUNRISE_HOT_PATHS_MODS").unwrap_or_default();
            if mods == "native" {
                Ok(HotPathsMods::Native)
            } else if let Some(count) = mods.strip_prefix("fanout-") {
                count
                    .parse()
                    .map(HotPathsMods::Fanout)
                    .map_err(|error| format!("invalid SUNRISE_HOT_PATHS_MODS={mods}: {error}"))
            } else {
                Ok(HotPathsMods::None)
            }
        });
        MODE.as_ref()
            .copied()
            .map_err(|error| error.as_str().into())
    }
}

impl crate::game::ServerGame<'_> {
    pub(super) fn hot_hook_fanout(
        &mut self,
        recorder: &mut Recorder,
        listeners: NonZeroU16,
    ) -> Result<()> {
        LISTENER_CALLS.with(|calls| {
            calls.get_or_init(|| (0..listeners.get()).map(|_| Cell::new(0)).collect());
        });
        let entity_tick = format!("mod.hook.fanout.entity_tick.{listeners}");
        let server_started = format!("mod.hook.fanout.server_started.{listeners}");
        if recorder.wants(&entity_tick) {
            self.hot_level(5, 2)?;
            self.hot_mobs(128, 2)?;
            let dimension = self.common.dim.id().to_owned();
            let entities: Vec<_> = self
                .common
                .mobs
                .iter()
                .map(|mob| (mob.id, mob.kind.id.clone()))
                .collect();
            measure(recorder, &entity_tick, || {
                for call in 0..CALLS as usize {
                    let (entity, kind) = &entities[call % entities.len()];
                    let mut canceled = false;
                    self.native_entity_tick_pre(&dimension, *entity, kind.as_str(), &mut canceled);
                }
            })?;
        }
        if recorder.wants(&server_started) {
            self.hot_level(5, 2)?;
            // One notification per successful server start in play. Repeat just the
            // notification to measure dispatch, not the cost of opening a world.
            measure(recorder, &server_started, || {
                for _ in 0..CALLS {
                    self.native_server_started(true);
                }
            })?;
        }
        Ok(())
    }
}

fn measure(recorder: &mut Recorder, name: &str, dispatch: impl FnOnce()) -> Result<()> {
    LISTENER_CALLS.with(|calls| {
        let calls = calls
            .get()
            .expect("fan-out slots are initialized by the fixture");
        for count in calls {
            count.set(0);
        }
        recorder.measure_checked(name, CALLS, dispatch, |()| {
            verified_calls(name, calls, CALLS)
        })
    })
}

fn verified_calls(name: &str, calls: &[Cell<u64>], expected: u64) -> Result<u64> {
    let mut total = 0;
    for (slot, count) in calls.iter().enumerate() {
        let observed = count.get();
        if observed != expected {
            return Err(format!(
                "{name}: listener {slot} expected {expected} calls, got {observed}"
            )
            .into());
        }
        total += observed;
    }
    Ok(total)
}

use sunrise_gameplay_common::game::native_hooks::base::GameHooks as _;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_doubled_listener_cannot_make_up_for_a_missing_listener() {
        let counts = [Cell::new(0), Cell::new(0)];
        LISTENER_CALLS.with(|calls| {
            calls.set(Box::new(counts)).unwrap();
            note_hot_paths_listener_call(1);
            note_hot_paths_listener_call(1);
            let slots = calls.get().unwrap();
            assert!(verified_calls("fanout", slots, 1).is_err());
            slots[1].set(1);
            note_hot_paths_listener_call(0);
            assert_eq!(verified_calls("fanout", slots, 1).unwrap(), 2);
        });
    }
}
