//! `sleepers_needed`: a mod sets how many players must sleep to skip the night.
use mod_api::{Hooks, Host, ModEntry, SleepersNeededEvent};
use std::sync::{Arc, Mutex, atomic::AtomicUsize, mpsc};
use sunrise_core::nbt::Tag;
use sunrise_game_data::text::Text;
use sunrise_protocol::packets::play::Clientbound;
use sunrise_sim::lan::{Out, PlayerSender, data::component_nbt};

/// Answers `self.0` sleepers, and records what each call saw.
struct Need(i64, Arc<Mutex<Vec<(String, i32, i64, i64)>>>);
#[mod_api::__subscriptions]
impl Hooks for Need {
    fn sleepers_needed(&self, _: &dyn Host, event: &mut SleepersNeededEvent<'_>) {
        self.1.lock().unwrap().push((
            event.dimension.to_owned(),
            event.percentage,
            event.active,
            event.needed,
        ));
        event.needed = self.0;
    }
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
fn a_mods_count_decides_the_skip_and_the_announcement() {
    crate::game::native_guest_turn_tests::with_guests(|game| {
        assert!(game.common.ui.rules.set(
            "players_sleeping_percentage",
            sunrise_game_data::gamerules::Value::Int(100)
        ));
        let (tx, rx) = mpsc::channel();
        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.sender = PlayerSender::new(tx, Arc::new(AtomicUsize::new(0)));
        });
        let clock = game.level_clock_name(game.common.dim);
        super::sleep_skip_night_tests::deep_asleep(game);
        // Guests 12 and 13 count and only 12 is in bed: vanilla needs both.
        assert!(!game.enough_slept());

        let heard = Arc::new(Mutex::new(Vec::new()));
        let one: mod_api::Mods =
            vec![ModEntry::new("one", Box::new(Need(1, heard.clone())))].into();
        mod_platform::testing::with_mods(one, || {
            assert!(game.enough_slept());
            game.announce_sleep_status();
            assert_eq!(
                next_overlay(&rx),
                Some(component_nbt(
                    &Text::translate("sleep.skipping_night", &[]).component()
                ))
            );
            game.tick_sleep();
        });
        let level = game.common.dim.id().to_owned();
        assert_eq!(heard.lock().unwrap().first(), Some(&(level, 100, 2, 2)));
        assert_eq!(game.command_clock_time(&clock), 24_000);
        assert!(
            game.server.lan.as_ref().unwrap().hub.guests[0]
                .bed
                .is_none()
        );

        // An answer below 1 counts as 1: with nobody in bed, the night stays.
        let zero: mod_api::Mods = vec![ModEntry::new("zero", Box::new(Need(0, heard)))].into();
        mod_platform::testing::with_mods(zero, || assert!(!game.enough_slept()));
    });
}
