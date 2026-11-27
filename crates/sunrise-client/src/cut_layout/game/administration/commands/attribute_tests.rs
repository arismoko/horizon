//! `/attribute … modifier value get` must not create the instance it looks up.
//! `AttributeCommand.getAttributeModifier` checks `hasAttribute` (in
//! `getEntityWithAttribute`), then `hasModifier`, and then reads
//! `getModifierValue`; all three use the cached instance or the type's supplier,
//! so a failed lookup leaves the entity's attribute map — and the `attributes`
//! list `LivingEntity.addAdditionalSaveData` writes — untouched.
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;
use sunrise_sim::motion::PlayerMotion;

pub(crate) fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        game.build_registries();
        game.set_allow_commands(true);
        test(game);
    });
}

#[allow(unused_imports)]
pub(crate) use sunrise_gameplay::game::server_side::commands_server::world_cmds::attribute_tests::say;

/// The `attributes` list the next save writes for the player.
pub(crate) fn saved_attributes(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> Tag {
    game.sync_level();
    game.level
        .player_extra
        .as_ref()
        .and_then(|extra| extra.get("attributes"))
        .cloned()
        .expect("the player's save carries its attributes")
}

#[test]
#[ignore = "daily"]
fn failed_modifier_value_get_leaves_the_attribute_store_and_its_save_alone() {
    with_game(|game| {
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 100.0, 4.5)).unwrap());
        game.reset_entity_residency();
        sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 1, 1);
        let key = game
            .player
            .as_ref()
            .unwrap()
            .attributes
            .key("minecraft:luck");
        assert!(key.is_some(), "a player has luck");
        assert!(
            game.player
                .as_ref()
                .unwrap()
                .attributes
                .lookup(key.as_ref())
                .is_none(),
            "an attribute the player never touched has no instance"
        );
        let before = saved_attributes(game);
        let (ok, said) = say(
            game,
            "attribute @s minecraft:luck modifier value get test:none",
        );
        assert!(!ok, "the lookup fails: {said}");
        assert!(
            said.contains("has no modifier test:none"),
            "the missing modifier is what fails, not the attribute: {said}"
        );
        assert_eq!(
            saved_attributes(game),
            before,
            "a failed lookup must not write a new attribute into the save"
        );
        assert!(
            game.player
                .as_ref()
                .unwrap()
                .attributes
                .lookup(key.as_ref())
                .is_none(),
            "nor create the instance"
        );
        // getEntityWithAttribute checks hasAttribute first: an attribute the
        // player's supplier does not have is the attribute error, not the
        // modifier one, and that check creates nothing either.
        let (ok, said) = say(
            game,
            "attribute @s minecraft:flying_speed modifier value get test:none",
        );
        assert!(!ok, "the lookup fails: {said}");
        assert!(
            said.contains("has no attribute"),
            "an attribute the player lacks is the attribute error: {said}"
        );
        assert_eq!(saved_attributes(game), before, "still no new attribute");
        // The same read on a modifier that exists still finds its amount.
        assert!(
            game.run_command("attribute @s minecraft:luck modifier add test:luck 0.25 add_value")
        );
        let (ok, said) = say(
            game,
            "attribute @s minecraft:luck modifier value get test:luck",
        );
        assert!(ok, "an existing modifier reads: {said}");
        assert!(said.contains("is 0.25"), "its amount: {said}");
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
