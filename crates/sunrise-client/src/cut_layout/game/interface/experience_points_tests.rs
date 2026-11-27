//! `Player.giveExperiencePoints` with a negative amount, through the game's own walk:
//! the loss goes down a level at a time through `Player.giveExperienceLevels(-1)`.
#[cfg(feature = "client")]
use crate::game::client_side::viewed_worlds::tests::client;
#[cfg(feature = "client")]
use sunrise_core::geometry::Vec3;
use sunrise_gameplay::game::game_tests::tests::server_game;
use sunrise_gameplay_common::game::GameGameExt1;
#[cfg(feature = "client")]
use sunrise_gameplay_common::game::{
    Screen,
    server_side::native_sound_refs::tests::{guest, packets},
};
#[cfg(feature = "client")]
use sunrise_protocol::packets::play::{self, Clientbound as C};
use sunrise_sim::entities::Experience;
#[cfg(feature = "client")]
use sunrise_sim::motion::PlayerMotion;

/// The death screen's score arrives through player metadata, not the XP total packet.
#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn the_death_screen_receives_the_signed_player_score() {
    let mut resources = None;
    let mut game = client(&mut resources);
    game.common.player = Some(PlayerMotion::new(game.common.motion, Vec3::ZERO).unwrap());
    let (mut guest, received) = guest(7, 42, [0.0; 3]);
    guest.score = -1;
    guest.sync_score();
    let mut sent = packets(&received);
    assert_eq!(sent.len(), 1);
    game.handle_packet(sent.pop().unwrap()).unwrap();
    game.handle_packet(C::SetExperience(play::SetExperience {
        level: 0,
        progress: 0.0,
        total: 100,
    }))
    .unwrap();
    game.handle_packet(C::SetHealth(play::SetHealth {
        health: 0.0,
        food: 20,
        saturation: 5.0,
    }))
    .unwrap();
    assert_eq!(
        (
            game.client.screen,
            game.common.player_score,
            game.common.experience.total
        ),
        (Screen::Dead, -1, 100)
    );
}

/// A point lost at level 0 leaves nothing of the 100: `giveExperienceLevels(-1)` zeroes the
/// total when the level would drop below 0.
#[test]
#[ignore = "daily"]
fn a_point_lost_at_level_zero_zeroes_the_total() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.experience = Experience {
        level: 0,
        progress: 0.0,
        total: 100,
    };
    game.give_experience_points(-1);
    let xp = &game.experience;
    assert_eq!((xp.level, xp.progress, xp.total), (0, 0.0, 0));
}

/// The mirror: above level 0 the same loss only takes the point given and walks the progress
/// back a level (level 5 needs 17, so it lands on level 4).
#[test]
#[ignore = "daily"]
fn a_point_lost_above_level_zero_only_takes_that_point() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.experience = Experience {
        level: 5,
        progress: 0.0,
        total: 100,
    };
    game.give_experience_points(-1);
    let xp = &game.experience;
    assert_eq!(xp.total, 99);
    assert_eq!(xp.level, 4);
    assert!(
        (xp.progress - (1.0 - 1.0 / 15.0)).abs() < 1e-6,
        "{}",
        xp.progress
    );
}
