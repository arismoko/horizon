use crate::game::server_side::native_guest_turn_tests::with_guests;
use std::sync::Arc;

#[test]
#[ignore = "daily"]
fn level_event_excludes_its_source_even_during_another_guests_turn() {
    use sunrise_sim::lan::PlayerSender;
    use sunrise_world::level::{LevelEvent, LevelEventPlayer};

    with_guests(|game| {
        let mut receivers = Vec::new();
        let host = game.server.lan.as_mut().unwrap();
        host.owner = Some(sunrise_gameplay::game::lan::Owner {
            uuid: 12,
            saved: None,
        });
        host.hub.guests.update_all(|guest| {
            let (tx, rx) = std::sync::mpsc::channel();
            guest.sender = PlayerSender::new(tx, Arc::default());
            receivers.push(rx);
        });
        game.as_guest(13, |game| {
            game.lan_record_events(&[LevelEvent::LevelEvent {
                id: 2001,
                pos: sunrise_core::position::BlockPos::new(3, 64, 5),
                data: 1,
                source: Some(LevelEventPlayer(12)),
            }]);
        })
        .unwrap();
        game.lan_flush_effects();
        assert!(
            receivers[0].try_recv().is_err(),
            "integrated host is the source"
        );
        assert!(
            receivers[1].try_recv().is_ok(),
            "the acting guest receives another player's event"
        );

        game.lan_record_events(&[LevelEvent::LevelEvent {
            id: 1500,
            pos: sunrise_core::position::BlockPos::new(3, 64, 5),
            data: 1,
            source: None,
        }]);
        game.lan_flush_effects();
        assert!(
            receivers[0].try_recv().is_ok(),
            "host receives null-source event"
        );
        assert!(
            receivers[1].try_recv().is_ok(),
            "guest receives null-source event"
        );
    });
}
