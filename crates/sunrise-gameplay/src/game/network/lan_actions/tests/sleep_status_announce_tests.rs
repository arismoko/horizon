//! `sleep_status_announce`: a mod cancels or replaces the action-bar sleeping line.
use crate::game::ServerGame;
use mod_api::{Hooks, ModEntry, ServerHost, SleepStatusAnnounceEvent};
use std::sync::{Arc, atomic::AtomicUsize, mpsc};
use sunrise_core::nbt::Tag;
use sunrise_game_data::text::Text;
use sunrise_protocol::packets::play::Clientbound;
use sunrise_sim::lan::{Out, PlayerSender, data::component_nbt};

/// Cancels the line.
struct Mute;
#[mod_api::__subscriptions]
impl Hooks for Mute {
    fn sleep_status_announce(
        &self,
        _: &mut dyn ServerHost,
        _: &mut SleepStatusAnnounceEvent<'_>,
    ) -> mod_api::Flow {
        mod_api::Flow::Cancel
    }
}

/// Replaces the line with its own.
struct Reword;
#[mod_api::__subscriptions]
impl Hooks for Reword {
    fn sleep_status_announce(
        &self,
        _: &mut dyn ServerHost,
        event: &mut SleepStatusAnnounceEvent<'_>,
    ) -> mod_api::Flow {
        *event.line = mod_api::Text::literal("Sweet dreams");
        mod_api::Flow::Continue
    }
}

/// Guest 12 in bed with a one percent rule: one of the two players sleeps and one is needed.
fn one_sleeper(game: &mut ServerGame<'_>) -> mpsc::Receiver<Out> {
    assert!(game.common.ui.rules.set(
        "players_sleeping_percentage",
        sunrise_game_data::gamerules::Value::Int(1)
    ));
    let (tx, rx) = mpsc::channel();
    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
        g.sender = PlayerSender::new(tx, Arc::new(AtomicUsize::new(0)));
        g.bed = Some([5, 64, 5]);
    });
    rx
}

fn next_overlay(rx: &mpsc::Receiver<Out>) -> Option<Tag> {
    while let Ok(out) = rx.try_recv() {
        let Out::Packet(body) = out else { continue };
        let mut r = sunrise_protocol::buf::Reader::new(&body);
        let id = r.varint().unwrap();
        if let Clientbound::SystemChat(chat) = Clientbound::decode(id, r.rest()).unwrap()
            && chat.overlay
        {
            return Some(chat.content);
        }
    }
    None
}

#[test]
#[ignore = "daily"]
fn a_canceling_listener_sends_no_overlay_line() {
    crate::game::native_guest_turn_tests::with_guests(|game| {
        let rx = one_sleeper(game);
        let mods: mod_api::Mods = vec![ModEntry::new("mute", Box::new(Mute))].into();
        mod_platform::testing::with_mods(mods, || game.announce_sleep_status());
        assert!(next_overlay(&rx).is_none());

        // The same change announced with no mod canceling shows vanilla's line.
        game.server.sleep_seen = (0, 0);
        game.announce_sleep_status();
        assert_eq!(
            next_overlay(&rx),
            Some(component_nbt(
                &Text::translate("sleep.skipping_night", &[]).component()
            ))
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_replaced_line_is_what_players_receive() {
    crate::game::native_guest_turn_tests::with_guests(|game| {
        let rx = one_sleeper(game);
        let mods: mod_api::Mods = vec![ModEntry::new("reword", Box::new(Reword))].into();
        mod_platform::testing::with_mods(mods, || game.announce_sleep_status());
        assert_eq!(
            next_overlay(&rx),
            Some(component_nbt(&Text::literal("Sweet dreams").component()))
        );
    });
}
