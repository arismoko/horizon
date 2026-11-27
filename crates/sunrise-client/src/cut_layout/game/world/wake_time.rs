#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
    use sunrise_gameplay_common::game::server_side::wake_time::tests::NOW;

    use sunrise_gameplay_common::game::server_side::wake_time::tests::marker;

    use mod_api::ClockAdjustment;

    use sunrise_gameplay::world_clock::TimeMarker;

    /// What a second listener reads: the first moves the clock 1000 ticks on, the second sees
    /// that and sends it to noon instead, and the game resolves the last word once.
    #[test]
    #[ignore = "daily"]
    fn a_later_listener_reads_and_replaces_the_earlier_adjustment() {
        use std::sync::{Arc, Mutex};
        use sunrise_gameplay::world_clock::OVERWORLD_CLOCK;
        struct First;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for First {
            fn sleep_finished_time(
                &self,
                _: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::SleepFinishedTimeEvent<'_>,
            ) -> mod_api::Flow {
                *event.adjustment = ClockAdjustment::Relative(1_000);
                mod_api::Flow::Continue
            }
        }
        struct Second {
            seen: Arc<Mutex<Option<ClockAdjustment>>>,
            manual: Arc<Mutex<Vec<String>>>,
        }
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Second {
            fn sleep_finished_time(
                &self,
                _: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::SleepFinishedTimeEvent<'_>,
            ) -> mod_api::Flow {
                *self.seen.lock().unwrap() = Some(event.adjustment.clone());
                *event.adjustment = marker("minecraft:noon");
                mod_api::Flow::Continue
            }

            fn time_skip(
                &self,
                _: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::TimeSkipEvent<'_>,
            ) -> mod_api::Flow {
                self.manual.lock().unwrap().push(event.cause.to_owned());
                mod_api::Flow::Cancel
            }
        }
        crate::game::native_guest_turn_tests::with_guests(|game| {
            game.player = None;
            let guests = &mut game.server.lan.as_mut().unwrap().hub.guests;
            for i in 0..guests.len() {
                guests.update(i, |guest| {
                    guest.bed = Some([3, 64, 5]);
                    guest.sleeping = true;
                    guest.sleep_timer = 100;
                });
            }
            let noon = TimeMarker {
                ticks: 6_000,
                period: Some(24_000),
                show: false,
            };
            game.ui
                .markers
                .insert((OVERWORLD_CLOCK.into(), "minecraft:noon".into()), noon);
            let clock = game.level_clock_name(game.dim);
            assert_eq!(clock, OVERWORLD_CLOCK);
            game.set_command_clock_time(&clock, NOW);
            let seen = Arc::new(Mutex::new(None));
            let manual = Arc::new(Mutex::new(Vec::new()));
            let mods = vec![
                mod_api::ModEntry::new("first", Box::new(First) as Box<dyn mod_api::Hooks>),
                mod_api::ModEntry::new(
                    "second",
                    Box::new(Second {
                        seen: seen.clone(),
                        manual: manual.clone(),
                    }) as Box<dyn mod_api::Hooks>,
                ),
            ]
            .into();
            mod_platform::testing::with_mods(mods, || {
                assert!(game.enough_slept());
                game.tick_sleep();
                assert_eq!(
                    *seen.lock().unwrap(),
                    Some(ClockAdjustment::Relative(1_000))
                );
                assert_eq!(game.command_clock_time(&clock), 30_000);
                assert!(!game.enough_slept());
                assert!(manual.lock().unwrap().is_empty());

                let level = game.level(OVERWORLD_CLOCK).unwrap();
                assert!(!game.level_set_clock_time(level, 40_000));
                assert_eq!(game.command_clock_time(&clock), 30_000);
                assert_eq!(*manual.lock().unwrap(), ["mod"]);
            });
        });
    }
}
