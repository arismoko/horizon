#[cfg(any())]
#[cfg(test)]
pub(in crate::game) mod server_tick_rate_changed_tests {
    use crate::game::native_guest_turn_tests::with_guests;
    use mod_api::{Hooks, ModEntry, ServerHost, ServerTickRateChangedEvent};
    use mod_platform::testing::with_mods;
    use std::sync::{Arc, Mutex};

    /// A scheduling mod: logs the rate each notification reads, and runs `retune` on the first.
    pub(in crate::game) struct Scheduler {
        pub(in crate::game) rates: Arc<Mutex<Vec<f32>>>,
        pub(in crate::game) retune: Option<f32>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Scheduler {
        fn server_tick_rate_changed(
            &self,
            host: &mut dyn ServerHost,
            _: &mut ServerTickRateChangedEvent,
        ) {
            let first = {
                let mut rates = self.rates.lock().unwrap();
                rates.push(host.tick_stats().0);
                rates.len() == 1
            };
            if let (true, Some(rate)) = (first, self.retune) {
                assert!(host.set_tick_rate(rate));
            }
        }
    }
}
