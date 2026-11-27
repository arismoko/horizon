//! `LevelEventHandler` 1039 (`LevelEvent.SOUND_PHANTOM_BITE`): the server
//! queues level event 1039 for guests.
use sunrise_gameplay_common::game::{QueuedLevelEvent, World};

fn with_server_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    let mut resources = None;
    let mut g = sunrise_gameplay::game::game_tests::tests::server_game(&mut resources);
    g.options.dedicated = Some(Vec::new());
    g.world = World::new(g.res.facts().air, 1);
    // The guests' copy of a level event is recorded only while capturing.
    g.server.audio_capture_active = true;
    test(&mut g);
}

/// Level event 1039 reaches guests as the level event itself, which they play.
#[test]
#[ignore = "daily"]
fn a_phantom_bite_level_event_reaches_guests_as_level_event_1039() {
    with_server_game(|g| {
        g.server_level_event_at(1039, [0, 64, 0], 0);
        assert_eq!(
            g.server.lan_events,
            vec![QueuedLevelEvent {
                kind: 1039,
                pos: [0, 64, 0],
                data: 0,
                source: None,
            }],
            "the bite reaches the guests as level event 1039"
        );
    });
}
