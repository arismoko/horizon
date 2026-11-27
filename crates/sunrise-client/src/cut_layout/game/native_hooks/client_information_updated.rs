#![cfg(test)]
#[cfg(test)]
pub(crate) mod client_information_updated_tests {
    use mod_api::{
        ChatVisibility, ClientInfo, ClientInformationUpdatedEvent, Hooks, MainHand, ModEntry,
        ParticleStatus, PlayerLoggedInEvent, ServerHost,
    };
    use std::sync::{Arc, Mutex};

    /// What was heard, in order: the information's old value and new view
    /// distance and main hand, or the login.
    type Heard = Arc<Mutex<Vec<Option<(ClientInfo, i8, MainHand)>>>>;
    struct Listen(Heard);
    #[mod_api::__subscriptions]
    impl Hooks for Listen {
        fn client_information_updated(
            &self,
            _: &mut dyn ServerHost,
            event: &mut ClientInformationUpdatedEvent<'_>,
        ) {
            let seen = (
                event.old.clone(),
                event.new.view_distance,
                event.new.main_hand,
            );
            self.0.lock().unwrap().push(Some(seen));
        }
        fn player_logged_in(&self, _: &mut dyn ServerHost, _: &mut PlayerLoggedInEvent<'_>) {
            self.0.lock().unwrap().push(None);
        }
    }

    /// The information a guest joins with is heard once, from the defaults,
    /// before the login.
    #[test]
    #[ignore = "daily"]
    fn the_joined_client_information_is_heard_before_the_login() {
        let heard = Heard::default();
        let mods: mod_api::Mods =
            vec![ModEntry::new("info", Box::new(Listen(heard.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
                let host = game.server.lan.as_mut().unwrap();
                let (uuid, name) = host
                    .hub
                    .update_guest(12, |g| {
                        g.info.view_distance = 12;
                        g.info.main_hand = 0;
                        (g.uuid, g.name.clone())
                    })
                    .unwrap();
                game.announce_login(uuid, &name);
                // Vanilla's `ClientInformation.createDefault()`.
                let defaults = ClientInfo {
                    language: "en_us".into(),
                    view_distance: 2,
                    chat_visibility: ChatVisibility::Full,
                    chat_colors: true,
                    model_customisation: 0,
                    main_hand: MainHand::Right,
                    text_filtering: false,
                    allows_listing: false,
                    particle_status: ParticleStatus::All,
                };
                assert_eq!(
                    *heard.lock().unwrap(),
                    [Some((defaults, 12, MainHand::Left)), None]
                );
            });
        });
    }
}
