#![cfg(test)]

#[cfg(test)]
pub(crate) mod player_leave_tests {
    use mod_api::{
        AttachmentKey, Hooks, PlayerBedLeaveEvent, PlayerLoggedOutEvent, Registrar, ServerHost,
    };
    use std::sync::{Arc, Mutex};
    use sunrise_core::nbt::{Compound, Tag};
    use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;
    use sunrise_mod_runtime::attachments::{AttachmentCodec, AttachmentError, Holder};

    /// Each case runs in a fresh child process: the fixture loads the prepared resources into
    /// the process-wide tables (languages, POIs, cauldron interactions).
    pub(crate) const FLAG: &str = "SUNRISE_PLAYER_LEAVE_CHILD";
    pub(crate) const SAVED: &str = "cut_layout::game::server_side::native_hooks_server::player_leave::player_leave_tests::what_a_listener_changes_on_a_leaving_player_is_saved";
    pub(crate) const ASLEEP: &str = "cut_layout::game::native_hooks::player_leave::player_leave_tests::a_sleeping_player_wakes_before_logging_out";

    /// What a listener heard, in order.
    #[derive(Debug, PartialEq)]
    pub(crate) enum Heard {
        /// `player_bed_leave`'s two flags.
        BedLeave(bool, bool),
        /// `player_logged_out`: the online players in their order, whether the player was found
        /// by UUID and still slept, the attachment write's answer and whether the health was set.
        LoggedOut {
            listed: Vec<u128>,
            found: bool,
            asleep: bool,
            attached: Result<(), AttachmentError>,
            healed: bool,
        },
    }

    pub(crate) struct Leave {
        pub(crate) heard: Arc<Mutex<Vec<Heard>>>,
        pub(crate) value: Mutex<Option<AttachmentKey<i32>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Leave {
        fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            let value = registrar
                .register_attachment_type(
                    mod_api::AttachmentDescriptor::new("leave_test:value", || 0i32).serialize(
                        AttachmentCodec {
                            encode: |n| Ok(Compound::from([("n".into(), Tag::Int(*n))])),
                            decode: |data| match data.get(&"n".into()) {
                                Some(Tag::Int(n)) => Ok(*n),
                                _ => Err(AttachmentError::Decode),
                            },
                        },
                    ),
                )
                .unwrap();
            *self.value.lock().unwrap() = Some(value);
        }
        fn player_bed_leave(&self, _: &mut dyn ServerHost, event: &mut PlayerBedLeaveEvent<'_>) {
            self.heard
                .lock()
                .unwrap()
                .push(Heard::BedLeave(event.wake_immediately, event.update_level));
        }
        fn player_logged_out(
            &self,
            host: &mut dyn ServerHost,
            event: &mut PlayerLoggedOutEvent<'_>,
        ) {
            let listed = host
                .players()
                .server_players()
                .iter()
                .map(|player| player.uuid())
                .collect();
            let player = host.player(event.player);
            let living = player.and_then(|player| host.living(player.entity()));
            let asleep = living.is_some_and(|living| living.sleeping_pos(host).is_some());
            let value = self.value.lock().unwrap().clone().unwrap();
            let attached = value.set(host, Holder::Player(event.player), 77);
            let healed = living.is_some_and(|living| living.set_health(host, 7.0));
            self.heard.lock().unwrap().push(Heard::LoggedOut {
                listed,
                found: player.is_some(),
                asleep,
                attached,
                healed,
            });
        }
    }

    #[allow(unused_imports)]
pub(crate) use crate::game::server_side::native_hooks_server::player_leave::player_leave_tests::first_of_two_leaves;

    /// `ServerPlayer.disconnect` wakes a sleeping player (`stopSleepInBed(true, false)`) before
    /// `PlayerList.remove`: the wake is heard first, and the logout finds them awake.
    #[test]
    #[ignore = "daily"]
    fn a_sleeping_player_wakes_before_logging_out() {
        if run_fresh_cases(ASLEEP, FLAG, &["asleep"]) {
            return;
        }
        first_of_two_leaves(
            |game| {
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |g| {
                        g.bed = Some([5, 64, 5]);
                        g.sleeping = true;
                        g.sleep_timer = 10;
                    })
                    .unwrap();
            },
            |_, told| {
                assert_eq!(told.len(), 2, "{told:?}");
                assert_eq!(told[0], Heard::BedLeave(true, false));
                assert!(
                    matches!(
                        told[1],
                        Heard::LoggedOut {
                            asleep: false,
                            found: true,
                            ..
                        }
                    ),
                    "{told:?}"
                );
            },
        );
    }
}
