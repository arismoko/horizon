//! `VehicleEntity.hurtServer` on a minecart: the `ENTITY_DAMAGE` game event it
//! posts, the `CustomName` `destroy` puts on the drop, and
//! `chestVehicleDestroyed`'s angry piglins. Prepared resources only.
use sunrise_core::geometry::Vec3;
use sunrise_gameplay::game::game_tests::tests::server_game;
use sunrise_gameplay::game::server_side::minecarts_server::CartHit;
use sunrise_sim::mobs::{Species, Who};
use {
    sunrise_gameplay_common::game::minecarts::CartKind,
    sunrise_gameplay_common::game::minecarts::Minecart,
};

/// A server game with a player at `(8.5, 64, 8.5)`, as a window's player is
/// there for the piglins that look at it.
fn game_with_player(
    resources: &mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
) -> sunrise_gameplay::game::ServerGame<'_> {
    let mut game = server_game(resources);
    game.build_registries();
    game.set_allow_commands(true);
    game.player = Some(
        sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(8.5, 64.0, 8.5)).unwrap(),
    );
    game
}

fn start_events() {
    sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
        .with(|e| *e.borrow_mut() = Some(Vec::new()));
}

fn take_events() -> Vec<(&'static str, [f64; 3], Option<Who>)> {
    sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
        .with(|e| std::mem::take(e.borrow_mut().as_mut().unwrap()))
}

fn stop_events() {
    sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
        .with(|e| *e.borrow_mut() = None);
}

/// `VehicleEntity.hurtServer`: the hit shakes the cart, adds `damage * 10`,
/// and posts one `ENTITY_DAMAGE` at the cart, attributed to
/// `source.getEntity()`; a creative hit posts it too, and discards the cart.
#[test]
#[ignore = "daily"]
fn a_hit_posts_one_entity_damage_from_its_causing_entity() {
    let mut resources = None;
    let mut game = game_with_player(&mut resources);
    let at = Vec3::new(8.5, 64.0, 8.5);
    game.carts.push(Minecart::new(1, CartKind::Rideable, at));
    start_events();
    assert!(game.hurt_cart_result(0, 1.0, false, false, CartHit::player()));
    assert_eq!(
        take_events(),
        vec![(
            "minecraft:entity_damage",
            [8.5, 64.0, 8.5],
            Some(Who::Player)
        )],
        "one event, at the cart, from the player whose hit it was"
    );
    assert_eq!((game.carts[0].hurt_time, game.carts[0].damage), (10, 10.0));
    // An arrow's hit is the shooter's (`AbstractArrow.getOwner`), here a mob.
    game.carts.push(Minecart::new(2, CartKind::Rideable, at));
    assert!(game.hurt_cart_result(
        1,
        1.0,
        false,
        false,
        CartHit::projectile(Some(Who::Mob(7)), None)
    ));
    assert_eq!(
        take_events(),
        vec![(
            "minecraft:entity_damage",
            [8.5, 64.0, 8.5],
            Some(Who::Mob(7))
        )],
        "an arrow attributes the hit to its owner"
    );
    // A creative hit breaks the cart at once, and is heard the same way.
    assert!(game.hurt_cart_result(0, 0.0, true, false, CartHit::player()));
    assert!(game.carts.iter().all(|c| c.id != 1));
    assert_eq!(take_events().len(), 1, "a creative hit posts it too");
    stop_events();
}

/// `VehicleEntity.destroy`: the drop is the cart's item, carrying the
/// vehicle's `CustomName` (`DataComponents.CUSTOM_NAME`).
#[test]
#[ignore = "daily"]
fn a_broken_cart_drops_its_item_under_its_name() {
    let mut resources = None;
    let mut game = game_with_player(&mut resources);
    assert!(game.run_command(r#"summon minecart 8.5 64 8.5 {CustomName:"Boat"}"#));
    assert_eq!(game.carts.len(), 1);
    assert_eq!(game.carts[0].kind, CartKind::Rideable);
    assert!(game.hurt_cart_result(0, 100.0, false, false, CartHit::default()));
    assert!(game.carts.is_empty(), "the cart is broken");
    assert_eq!(game.dropped.len(), 1, "the cart item alone");
    let stack = &game.dropped[0].stack;
    assert_eq!(&*stack.item, "minecraft:minecart");
    assert_eq!(
        stack.custom_name().as_deref(),
        Some("Boat"),
        "the drop is named as the cart was"
    );
}

/// The piglin calm again, before the next case.
fn calm(game: &mut sunrise_gameplay::game::ServerGame<'_>, piglin: usize) {
    let m = &mut game.mobs[piglin];
    m.extra.anger = 0;
    m.ai.target = None;
    m.extra.piglin.guarded_angry_at = None;
}

/// `chestVehicleDestroyed`: a player's own hit that breaks a chest cart, with
/// `entity_drops` on, angers the piglins that see the player; a creative hit
/// only discards the cart, so they stay calm.
#[test]
#[ignore = "daily"]
fn a_chest_cart_broken_by_a_player_angers_the_piglins_that_see_them() {
    let mut resources = None;
    let mut game = game_with_player(&mut resources);
    assert!(game.run_command("summon piglin 8.5 64 12.5 {NoAI:1b,PersistenceRequired:1b}"));
    let piglin = game
        .mobs
        .iter()
        .position(|m| m.species() == Species::Piglin)
        .expect("the summoned piglin");
    assert!(
        game.ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::ENTITY_DROPS)
    );
    let at = Vec3::new(8.5, 64.0, 8.5);
    game.carts.push(Minecart::new(1, CartKind::Chest, at));
    game.carts.push(Minecart::new(2, CartKind::Chest, at));
    // A creative hit discards the cart: nothing drops, no one is angered.
    assert!(game.hurt_cart_result(0, 0.0, true, false, CartHit::player()));
    assert!(game.dropped.is_empty());
    assert!(
        game.mobs[piglin].extra.piglin.guarded_angry_at.is_none(),
        "a creative hit angers no piglin"
    );
    // `/damage ... by @p`: the player is the hit's direct entity, too.
    assert!(
        game.run_command(
            "damage @e[type=chest_minecart,limit=1] 100 minecraft:player_attack by @p"
        )
    );
    assert!(game.carts.is_empty(), "the damage command broke the cart");
    assert!(
        game.mobs[piglin].extra.piglin.guarded_angry_at.is_some(),
        "by @p angers the piglins that see the player"
    );
    calm(&mut game, piglin);
    // A player's own hit that breaks it does.
    game.carts.push(Minecart::new(3, CartKind::Chest, at));
    assert!(game.hurt_cart_result(0, 100.0, false, false, CartHit::player()));
    assert!(game.carts.is_empty());
    let m = &game.mobs[piglin];
    assert!(
        m.extra.piglin.guarded_angry_at.is_some(),
        "the piglin saw it"
    );
    assert_eq!(
        sunrise_sim::mobs::piglin::angry_at(m),
        Some(600),
        "PiglinAi.setAngerTarget's 600 ticks"
    );
    assert_eq!(m.ai.target, Some(Who::Player));
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
