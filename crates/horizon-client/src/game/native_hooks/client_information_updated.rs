use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// NeoForge's `ClientInformationUpdatedEvent`: a server player's client sent new
    /// client information (language, view distance, chat, skin parts, main hand,
    /// filtering, listing, particles) in play. `old` and `new` are the values before and
    /// after this packet, one call per packet; they can be equal. The join's own
    /// information is reported too, once, just before `player_logged_in`, with
    /// `old` the defaults (`ClientInformation.createDefault`) and `new` what the
    /// client joined with, as NeoForge fires it when `ServerPlayer` first sets
    /// it. The player is not swapped in. Server-side.
    pub(in crate::game) fn native_client_information_updated(
        &mut self,
        player: u128,
        player_entity_id: i32,
        name: &str,
        old: &mod_api::ClientInfo,
        new: &mod_api::ClientInfo,
    ) {
        self.fire_client_information_updated(player, player_entity_id, name, old, new);
    }

    /// A placed player's login as the mods hear it: the client information
    /// they joined with (`client_information_updated`, from the defaults),
    /// then `player_logged_in`.
    pub(in crate::game) fn announce_login(&mut self, player: u128, name: &str) {
        if mod_api::__private::listened(mod_api::HookId::ClientInformationUpdated)
            && let Some(g) = self
                .lan
                .as_ref()
                .and_then(|host| host.hub.guests.iter().find(|g| g.uuid == player))
        {
            let entity_id = g.entity_id;
            let new = crate::game::lan::client_info(&g.info);
            // `createDefault`: the packet's constructor gives the join's view distance, and
            // shows every skin layer and lists the player.
            let mut defaults = horizon_protocol::packets::configuration::ClientInformation::new(2);
            defaults.model_customisation = 0;
            defaults.allows_listing = false;
            let old = crate::game::lan::client_info(&defaults);
            self.native_client_information_updated(player, entity_id, name, &old, &new);
        }
        self.native_player_logged_in(player, name);
    }
}

#[cfg(test)]
mod client_information_updated_tests {
    use mod_api::{
        ChatVisibility, ClientInfo, ClientInformationUpdatedEvent, Hooks, MainHand, ModEntry,
        ParticleStatus, PlayerLoggedInEvent, ServerHost,
    };
    use std::sync::{Arc, Mutex};

    /// What was heard, in order: the information's old and new view
    /// distance and main hand, or the login.
    type Heard = Arc<Mutex<Vec<Option<(i8, i8, MainHand)>>>>;
    struct Listen(Heard);
    #[mod_api::__subscriptions]
    impl Hooks for Listen {
        fn client_information_updated(
            &self,
            _: &mut dyn ServerHost,
            event: &mut ClientInformationUpdatedEvent<'_>,
        ) {
            let seen = (
                event.old.view_distance,
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
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn the_joined_client_information_is_heard_before_the_login() {
        let heard = Heard::default();
        let mods: mod_api::Mods =
            vec![ModEntry::new("info", Box::new(Listen(heard.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_guest_turn_tests::with_guests(|game| {
                let host = game.lan.as_mut().unwrap();
                let (uuid, name) = host
                    .hub
                    .update_guest(12, |g| {
                        g.info.view_distance = 12;
                        g.info.main_hand = 0;
                        (g.uuid, g.name.clone())
                    })
                    .unwrap();
                game.announce_login(uuid, &name);
                assert_eq!(
                    *heard.lock().unwrap(),
                    [Some((2, 12, MainHand::Left)), None]
                );
            });
        });
    }

    /// Records the whole `old` information of each update.
    struct Olds(Arc<Mutex<Vec<ClientInfo>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Olds {
        fn client_information_updated(
            &self,
            _: &mut dyn ServerHost,
            event: &mut ClientInformationUpdatedEvent<'_>,
        ) {
            self.0.lock().unwrap().push(event.old.clone());
        }
    }

    /// The `old` side of the join's own report is `ClientInformation.createDefault`, every field.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn the_joined_information_is_reported_against_all_the_vanilla_defaults() {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![ModEntry::new("olds", Box::new(Olds(heard.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_guest_turn_tests::with_guests(|game| {
                let host = game.lan.as_mut().unwrap();
                let (uuid, name) = host
                    .hub
                    .update_guest(12, |g| (g.uuid, g.name.clone()))
                    .unwrap();
                game.announce_login(uuid, &name);
                assert_eq!(
                    *heard.lock().unwrap(),
                    [ClientInfo {
                        language: "en_us".to_owned(),
                        view_distance: 2,
                        chat_visibility: ChatVisibility::Full,
                        chat_colors: true,
                        model_customisation: 0,
                        main_hand: MainHand::Right,
                        text_filtering: false,
                        allows_listing: false,
                        particle_status: ParticleStatus::All,
                    }]
                );
            });
        });
    }
}
