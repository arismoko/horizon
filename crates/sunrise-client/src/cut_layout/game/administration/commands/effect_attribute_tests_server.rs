use crate::game::server_side::commands::world_cmds::effect_attribute_tests::with_game;
use sunrise_gameplay_common::game::commands_common::Target;

/// `isTransmittingWaypoint` reads the range with Invisibility's modifier:
/// the given zombie's 100 is hidden to 0, so its next waypoint check
/// untracks it, while the zombie that loaded Invisibility keeps transmitting.
#[test]
#[ignore = "daily"]
fn a_given_invisibility_hides_a_mobs_waypoint_and_a_loaded_one_does_not() {
    with_game(|game| {
        assert!(game.run_command(
            r#"summon minecraft:zombie 2.5 64 2.5 {NoAI:1b,Tags:["given"],UUID:[I;1988,0,0,3]}"#
        ));
        assert!(game.run_command(
            r#"summon minecraft:zombie 4.5 64 2.5 {NoAI:1b,Tags:["loaded"],UUID:[I;1988,0,0,4],active_effects:[{id:"minecraft:invisibility",amplifier:0b,duration:-1}]}"#
        ));
        let mob = |game: &sunrise_gameplay::game::ServerGame<'_>, n: i32| {
            let m = game
                .mobs
                .iter()
                .find(|m| game.persist.uuid_of(m.id) == Some([1988, 0, 0, n]))
                .expect("the summoned zombie");
            Target::Mob(m.id)
        };
        let (given, loaded) = (mob(game, 3), mob(game, 4));
        // A range write marks the entity for its next waypoint check.
        let set_range = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
            for tag in ["given", "loaded"] {
                assert!(game.run_command(&format!(
                    "attribute @e[tag={tag},limit=1] minecraft:waypoint_transmit_range base set 100"
                )));
            }
            game.tick_waypoints();
        };
        set_range(game);
        let tracked = |game: &sunrise_gameplay::game::ServerGame<'_>| {
            game.server.cmds.0.waypoints.clone().unwrap_or_default()
        };
        assert!(tracked(game).contains(&given));
        assert!(tracked(game).contains(&loaded));
        assert!(
            game.run_command("effect give @e[tag=given,limit=1] minecraft:invisibility infinite 0")
        );
        set_range(game);
        assert!(!tracked(game).contains(&given), "{:?}", tracked(game));
        assert!(tracked(game).contains(&loaded), "{:?}", tracked(game));
    });
}

/// The number a successful `/attribute` read says, or None when it failed.
pub(crate) fn read(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    command: &str,
) -> Option<String> {
    game.server.chat_capture = Some(Vec::new());
    let ok = game.run_command(command);
    let said: String = game
        .server
        .chat_capture
        .take()
        .unwrap_or_default()
        .iter()
        .map(sunrise_game_data::text::Text::plain)
        .collect();
    ok.then(|| {
        said.split_whitespace()
            .last()
            .unwrap_or_default()
            .to_owned()
    })
}
