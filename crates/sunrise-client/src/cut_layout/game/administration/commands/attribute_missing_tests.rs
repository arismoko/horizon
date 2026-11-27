//! `/attribute <target> <attribute> …` on an attribute the entity's supplier
//! does not create: `AttributeCommand.getAttributeInstance` asks the map's
//! supplier for an instance and throws `commands.attribute.failed.no_attribute`
//! when it has none (a player has no `minecraft:follow_range`), before any
//! modifier is added or removed. The same id on an attribute the player does
//! have still works.
use std::sync::Arc;
use sunrise_gameplay_common::game::Vec3;
use sunrise_sim::motion::PlayerMotion;

pub(crate) fn game<'a>(
    resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
) -> sunrise_gameplay::game::ServerGame<'a> {
    let mut game = sunrise_gameplay::game::game_tests::tests::server_game(resources);
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 100.0, 0.5)).unwrap());
    game.reset_entity_residency();
    sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(&mut game, 1, 1);
    game
}

#[allow(unused_imports)]
pub(crate) use sunrise_gameplay::game::server_side::commands_server::world_cmds::attribute_missing_tests::run;

/// The one line a command told its issuer (`command_failure` adds no others).
pub(crate) fn only(said: &[String]) -> &str {
    assert_eq!(said.len(), 1, "{said:?}");
    &said[0]
}

#[test]
#[ignore = "daily"]
fn modifier_add_on_an_attribute_the_player_lacks_is_the_no_attribute_error() {
    let mut resources = None;
    let mut game = game(&mut resources);
    let (ok, said) = run(
        &mut game,
        "attribute @s minecraft:follow_range modifier add test:a 1 add_value",
    );
    assert!(!ok, "vanilla throws ERROR_NO_SUCH_ATTRIBUTE: {said:?}");
    // `commands.attribute.failed.no_attribute`, naming `getAttributeDescription`.
    let said = only(&said);
    assert!(said.contains("has no attribute"), "{said:?}");
    assert!(said.contains("Mob Follow Range"), "{said:?}");
    // The failed add stored nothing: the supplier still has no instance.
    let key = game
        .player
        .as_mut()
        .unwrap()
        .attributes
        .key("minecraft:follow_range")
        .expect("registered attribute");
    assert!(
        game.player
            .as_mut()
            .unwrap()
            .attributes
            .get_instance(Some(key))
            .unwrap()
            .is_none()
    );
    // The same modifier on an attribute the player does have is added and read.
    let (ok, said) = run(
        &mut game,
        "attribute @s minecraft:movement_speed modifier add test:a 1 add_value",
    );
    assert!(ok, "{said:?}");
    let (ok, said) = run(
        &mut game,
        "attribute @s minecraft:movement_speed modifier value get test:a",
    );
    assert!(ok, "{said:?}");
    assert!(only(&said).contains("1.0"), "{said:?}");
}

#[test]
#[ignore = "daily"]
fn modifier_remove_on_an_attribute_the_player_lacks_is_the_no_attribute_error() {
    let mut resources = None;
    let mut game = game(&mut resources);
    let (ok, said) = run(
        &mut game,
        "attribute @s minecraft:follow_range modifier remove test:a",
    );
    assert!(!ok, "vanilla throws ERROR_NO_SUCH_ATTRIBUTE: {said:?}");
    let said = only(&said);
    assert!(said.contains("has no attribute"), "{said:?}");
    assert!(said.contains("Mob Follow Range"), "{said:?}");
    // On an attribute the player has, an absent modifier is the no-modifier error.
    let (ok, said) = run(
        &mut game,
        "attribute @s minecraft:movement_speed modifier remove test:a",
    );
    assert!(!ok, "{said:?}");
    assert!(only(&said).contains("has no modifier"), "{said:?}");
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
