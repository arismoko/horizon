use super::super::native_entity_refs::tests::with_game;
use std::sync::Arc;
use sunrise_gameplay_common::game::creatures::mob_griefing_tests::Deny;
use {
    sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan,
    sunrise_gameplay_common::game::server_side::native_item_places::tests::guest,
};

/// `EntityMobGriefingEvent`: the mod's answer for one entity reaches the
/// game's checks and the mob AI's; another entity keeps the rule, and a
/// check with no entity reads the rule without asking.
#[test]
#[ignore = "daily"]
fn a_mod_denies_one_entity_griefing() {
    let heard = Arc::default();
    let (denied, other) = (900, 901);
    // Guest 5's entity id (`guest` gives `conn + 19`): a player the mod denies.
    let denied_player = 24;
    let mods = vec![mod_api::ModEntry::new(
        "deny",
        Box::new(Deny(vec![denied, denied_player], Arc::clone(&heard))),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            open_to_lan(game);
            assert!(
                game.ui
                    .rules
                    .bool_at(sunrise_game_data::gamerules::vanilla::MOB_GRIEFING)
            );
            assert!(!game.can_entity_grief(Some(denied)));
            assert!(game.can_entity_grief(Some(other)));
            assert!(game.can_entity_grief(None));
            let world = game.mob_world_base(&[]);
            assert!(!world.can_grief(denied));
            assert!(world.can_grief(other));
            let asked = heard.lock().unwrap().clone();
            assert_eq!(
                asked,
                [(denied, true), (other, true), (denied, true), (other, true)]
            );
            // A large fireball asks for its owner: the ghast the mod denies
            // lights no fire and another ghast's does; a fireball sent back
            // by the guest the mod denies lights none and another guest's
            // does. A gone owner, or none, reads the rule without asking.
            use sunrise_sim::mobs::Who;
            let hub = &mut game.server.lan.as_mut().unwrap().hub;
            hub.guests.push(guest(5));
            hub.guests.push(guest(6));
            heard.lock().unwrap().clear();
            assert!(!game.large_fireball_fire(Some(Who::Mob(denied)), None));
            assert!(game.large_fireball_fire(Some(Who::Mob(other)), None));
            assert!(!game.large_fireball_fire(Some(Who::Player), Some(5)));
            assert!(game.large_fireball_fire(Some(Who::Player), Some(6)));
            assert!(game.large_fireball_fire(Some(Who::Player), Some(7)));
            assert!(game.large_fireball_fire(None, None));
            assert_eq!(
                *heard.lock().unwrap(),
                [
                    (denied, true),
                    (other, true),
                    (denied_player, true),
                    (25, true)
                ]
            );
            // `shouldAffectBlocklikeEntities` asks for the exploding entity: a
            // blast that keeps blocks spares decorations when the mod denies
            // it, and reaches them for another exploding entity.
            use {
                sunrise_gameplay::game::server_side::explosion::BlockInteraction::Destroy,
                sunrise_gameplay::game::server_side::explosion::BlockInteraction::Keep,
            };
            game.server.combat.0.explosion_source = Some(denied);
            assert!(game.blast_spares_decorations(Keep));
            assert!(!game.blast_spares_decorations(Destroy));
            game.server.combat.0.explosion_source = Some(other);
            assert!(!game.blast_spares_decorations(Keep));
            game.server.combat.0.explosion_source = None;
        });
    });
}
