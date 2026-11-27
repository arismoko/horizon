#![cfg(test)]
#[cfg(test)]
#[cfg(any())]
pub(crate) mod server_tick_rate_changed_tests {
    use crate::game::{
        native_entity_refs::tests::run_fresh_cases, native_guest_turn_tests::with_guests,
    };
    use mod_api::{Hooks, ModEntry, ServerHost, ServerTickRateChangedEvent};
    use mod_platform::testing::with_mods;
    use std::sync::{Arc, Mutex, mpsc};
    use sunrise_client::lan::{Out, PlayerSender};
    use sunrise_protocol::{buf::Reader, packets::play::Clientbound as C};

    /// A scheduling mod: logs the rate each notification reads, and optionally retunes on the first.
    struct Scheduler {
        rates: Arc<Mutex<Vec<f32>>>,
        retune: Option<f32>,
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

    fn with_clock_listeners(
        mods: Vec<ModEntry>,
        check: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
    ) {
        with_mods(mods.into(), || with_guests(check));
    }

    /// The hook precedes autosave adjustment and the broadcast. A nested write skips only
    /// the callback already running, and both writes publish the final ticking state.
    #[test]
    #[ignore = "daily"]
    fn a_listener_runs_before_the_autosave_wait_is_adjusted() {
        if run_fresh_cases(
            "cut_layout::game::server_side::native_hooks_server::server_tick_rate_changed::server_tick_rate_changed_tests::a_listener_runs_before_the_autosave_wait_is_adjusted",
            "SUNRISE_TICK_RATE_CHILD",
            &["nested"],
        ) {
            return;
        }
        let observed = Arc::new(Mutex::new(Vec::new()));
        let retuned = Arc::new(Mutex::new(Vec::new()));
        let observer = Scheduler {
            rates: Arc::clone(&observed),
            retune: None,
        };
        let retuner = Scheduler {
            rates: Arc::clone(&retuned),
            retune: Some(40.0),
        };
        let mods = vec![
            ModEntry::new("observer", Box::new(observer)),
            ModEntry::new("retuner", Box::new(retuner)),
        ];
        with_clock_listeners(mods, |game| {
            let (tx, packets) = mpsc::channel();
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| {
                    guest.sender = PlayerSender::new(tx, Arc::default());
                })
                .unwrap();
            game.clock.set_frozen(true);

            assert!(game.set_tick_rate(10.0));
            assert_eq!(*observed.lock().unwrap(), [10.0, 40.0]);
            assert_eq!(*retuned.lock().unwrap(), [10.0]);
            assert_eq!(game.clock.rate(), 40.0);
            assert_eq!(game.server.ticks_until_autosave, 6000);

            let states: Vec<_> = packets
                .try_iter()
                .map(|out| {
                    let Out::Packet(body) = out else {
                        panic!("expected one ticking state packet");
                    };
                    let mut reader = Reader::new(&body);
                    let id = reader.varint().unwrap();
                    let C::TickingState(state) = C::decode(id, reader.rest()).unwrap() else {
                        panic!("expected a ticking state packet");
                    };
                    (state.rate, state.frozen)
                })
                .collect();
            assert_eq!(states, [(40.0, true); 2]);
            game.server.lan.as_mut().unwrap().hub.sync_ticking(
                game.clock.rate(),
                game.clock.frozen(),
                game.clock.step_state(),
            );
            assert!(matches!(packets.try_recv(), Err(mpsc::TryRecvError::Empty)));
        });
    }

    /// `ServerTickRateManager.setTickRate` and `finishTickSprint`, vanilla's only two callers of
    /// `MinecraftServer.onTickRateChanged`: each route tells a mod once, the new rate readable,
    /// and leaves the autosave wait at 300 seconds of ticks when that is shorter.
    #[test]
    #[ignore = "daily"]
    fn a_mod_hears_every_route_of_on_tick_rate_changed_once() {
        if run_fresh_cases(
            "cut_layout::game::server_side::native_hooks_server::server_tick_rate_changed::server_tick_rate_changed_tests::a_mod_hears_every_route_of_on_tick_rate_changed_once",
            "SUNRISE_TICK_RATE_CHILD",
            &["routes"],
        ) {
            return;
        }
        let rates = Arc::new(Mutex::new(Vec::new()));
        let scheduler = Scheduler {
            rates: Arc::clone(&rates),
            retune: None,
        };
        let mods = vec![ModEntry::new("scheduler", Box::new(scheduler))];
        with_clock_listeners(mods, |game| {
            let heard = || std::mem::take(&mut *rates.lock().unwrap());
            assert_eq!(game.server.ticks_until_autosave, 6000);
            assert!(!game.native_run_command("tick rate 0.5"));
            assert!(!game.native_run_command("tick rate 20000.0"));
            assert!(game.native_run_command("tick rate 10"));
            assert_eq!(heard(), [10.0]);
            assert_eq!(game.server.ticks_until_autosave, 3000);
            assert!(game.native_run_command("tick rate 40"));
            assert_eq!(heard(), [40.0]);
            assert_eq!(game.server.ticks_until_autosave, 3000);
            assert!(game.native_run_command("tick rate 40"));
            assert_eq!(heard(), [40.0]);

            assert!(!game.native_run_command("tick sprint stop"));
            assert!(game.native_run_command("tick sprint 5"));
            assert!(heard().is_empty());
            game.server.ticks_until_autosave = 100_000;
            assert!(game.native_run_command("tick sprint stop"));
            assert_eq!(heard(), [40.0]);
            assert_eq!(game.server.ticks_until_autosave, 12_000);

            assert!(game.native_run_command("tick sprint 5"));
            assert!(game.native_run_command("tick freeze"));
            assert_eq!(heard(), [40.0]);
            assert!(game.native_run_command("tick unfreeze"));
            assert!(heard().is_empty());

            assert!(game.native_run_command("tick rate 20"));
            assert_eq!(heard(), [20.0]);
            assert!(game.native_run_command("tick sprint 1"));
            game.server.ticks_until_autosave = 100_000;
            game.clock.world_runs();
            assert!(heard().is_empty());
            game.tick_server(false).unwrap();
            assert_eq!(heard(), [20.0]);
            // Completion shortens the wait before this tick decrements it.
            assert_eq!(game.server.ticks_until_autosave, 5999);
            game.tick_server(false).unwrap();
            assert!(heard().is_empty());
            assert_eq!(game.server.ticks_until_autosave, 5998);

            // Native writes use TickRateManager, not TickCommand's argument range.
            let host: &mut dyn ServerHost = game;
            assert!(host.set_tick_rate(0.5));
            assert_eq!(heard(), [1.0]);
            assert!(host.set_tick_rate(20_000.0));
            assert_eq!(heard(), [20_000.0]);
            for rate in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                assert!(!host.set_tick_rate(rate));
            }
            assert!(heard().is_empty());
            assert_eq!(game.clock.rate(), 20_000.0);
            assert_eq!(game.server.ticks_until_autosave, 300);
        });
    }
}
