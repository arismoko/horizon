//! `name=`, `gamemode=` and `team=` read what vanilla reads: the plain name
//! (`EntitySelectorOptions.java:103`, `getPlainTextName`), the player's real
//! `GameType` (`:229`) and `Entity.getTeam` (`:247`), which a tamed pet answers
//! with its owner's team (`TamableAnimal.java:190-203`).
use std::collections::HashSet;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::commands_common::Target;

fn select(game: &mut sunrise_gameplay::game::ServerGame<'_>, text: &str) -> HashSet<Target> {
    let selector = sunrise_command::selector::parse(
        &mut sunrise_command::Reader::new(text),
        &game.ui.registries,
        true,
    )
    .unwrap();
    game.select(&selector).into_iter().collect()
}

fn game<'a>(
    resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
) -> sunrise_gameplay::game::ServerGame<'a> {
    use sunrise_sim::motion::PlayerMotion;
    let mut game = sunrise_gameplay::game::game_tests::tests::server_game(resources);
    game.build_registries();
    game.set_allow_commands(true);
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    assert!(game.run_command("team add red"));
    assert!(game.run_command("team modify red prefix \"[R] \""));
    assert!(game.run_command("team join red @s"));
    game
}

#[test]
#[ignore = "daily"]
fn name_compares_the_plain_name() {
    let mut resources = None;
    let mut game = game(&mut resources);
    // The team prefix shows in the display name, never in the plain one.
    assert_eq!(game.target_name(Target::Player).plain(), "[R] Player");
    assert_eq!(
        select(&mut game, "@a[name=Player]"),
        HashSet::from([Target::Player])
    );
}

#[test]
#[ignore = "daily"]
fn gamemode_compares_the_real_mode() {
    let mut resources = None;
    let mut game = game(&mut resources);
    assert!(game.run_command("gamemode adventure"));
    assert_eq!(
        select(&mut game, "@a[gamemode=adventure]"),
        HashSet::from([Target::Player])
    );
    assert!(select(&mut game, "@a[gamemode=survival]").is_empty());
    assert!(game.run_command("gamemode spectator"));
    assert_eq!(
        select(&mut game, "@a[gamemode=spectator]"),
        HashSet::from([Target::Player])
    );
    assert!(select(&mut game, "@a[gamemode=creative]").is_empty());
}

#[test]
#[ignore = "daily"]
fn team_falls_back_to_the_owners_team() {
    let mut resources = None;
    let mut game = game(&mut resources);
    // `Owner` tames a wolf (TamableAnimal.readAdditionalSaveData).
    let owner = game.acting_player_uuid();
    assert!(game.run_command(&format!(
        "summon wolf 4.5 64 4.5 {{UUID:[I;271,0,0,1],NoAI:1b,NoGravity:1b,\
         Owner:[I;{},{},{},{}]}}",
        owner[0], owner[1], owner[2], owner[3]
    )));
    assert!(game.run_command("summon wolf 8.5 64 4.5 {UUID:[I;271,0,0,2],NoAI:1b,NoGravity:1b}"));
    let tamed = game.target_with_uuid([271, 0, 0, 1]).unwrap();
    let untamed = game.target_with_uuid([271, 0, 0, 2]).unwrap();
    assert!(matches!(tamed, Target::Mob(_)) && matches!(untamed, Target::Mob(_)));
    assert_eq!(
        select(&mut game, "@e[type=wolf,team=red]"),
        HashSet::from([tamed])
    );
    assert_eq!(
        select(&mut game, "@e[type=wolf,team=]"),
        HashSet::from([untamed])
    );
    // A team of its own wins over the owner's (`super.getTeam()` first).
    assert!(game.run_command("team add blue"));
    assert!(game.run_command("team join blue @e[type=wolf,team=red]"));
    assert_eq!(
        select(&mut game, "@e[type=wolf,team=blue]"),
        HashSet::from([tamed])
    );
    assert!(select(&mut game, "@e[type=wolf,team=red]").is_empty());
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;
