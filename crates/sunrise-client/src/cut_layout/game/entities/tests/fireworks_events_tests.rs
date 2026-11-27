//! `FireworkRocketEntity.explode`'s game event and sound gate, and the
//! `ITEM_USED` an `useOn` counts. Prepared resources only.
use crate::game::{Settings, load_resources};
use std::sync::Arc;
use sunrise_core::{geometry::Vec3, position::ChunkPos, random::LegacyRandom};
use {sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress};
use {
    sunrise_items::firework::FireworkExplosion, sunrise_items::firework::FireworkShape,
    sunrise_items::items::ItemStack, sunrise_level::chunk::Chunk, sunrise_sim::mobs::Who,
};

pub(crate) fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role =
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
    let mut game = sunrise_gameplay::game::ServerGame::new(
        &loaded.resources,
        loaded.pack,
        options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    game.player = Some(
        sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap(),
    );
    game.world.random = sunrise_core::random::LevelRandom::for_level(LegacyRandom::new(114));
    test(&mut game);
}

/// One `small_ball` that twinkles, as `/summon` reads it: a `Starter` of 16
/// ticks that plays a blast first and a twinkle last.
pub(crate) fn twinkling() -> FireworkExplosion {
    FireworkExplosion {
        shape: FireworkShape::SmallBall,
        colors: vec![0xFF5555],
        fade: Vec::new(),
        trail: true,
        twinkle: true,
    }
}

/// Recording of `Game::post_game_event`, as the sculk tests read it.
pub(crate) fn start_events() {
    sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
        .with(|e| *e.borrow_mut() = Some(Vec::new()));
}

pub(crate) fn take_explosions() -> Vec<(&'static str, [f64; 3], Option<Who>)> {
    sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS.with(|e| {
        std::mem::take(e.borrow_mut().as_mut().unwrap())
            .into_iter()
            .filter(|e| e.0 == "minecraft:explode")
            .collect()
    })
}

pub(crate) fn stop_events() {
    sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
        .with(|e| *e.borrow_mut() = None);
}

/// `FireworkRocketEntity.explode`: `gameEvent(GameEvent.EXPLODE, getOwner())` at
/// the rocket, once, with and without explosions; a rocket nobody owns (a
/// dispenser's) has no source entity.
#[test]
#[ignore = "daily"]
fn a_detonating_rocket_posts_one_explode_from_its_owner() {
    with_game(|game| {
        let stack = ItemStack::new("minecraft:firework_rocket", 1);
        let mut burst = game.new_rocket(
            Vec3::new(8.5, 202.0, 8.5),
            &stack,
            false,
            &mut LegacyRandom::new(0),
        );
        burst.explosions = vec![twinkling()].into();
        burst.owner = Some(Who::Player);
        let mut plain = game.new_rocket(
            Vec3::new(9.5, 202.0, 8.5),
            &stack,
            false,
            &mut LegacyRandom::new(0),
        );
        plain.owner = Some(Who::Player);
        let mut stray = game.new_rocket(
            Vec3::new(10.5, 202.0, 8.5),
            &stack,
            false,
            &mut LegacyRandom::new(0),
        );
        stray.explosions = vec![twinkling()].into();

        start_events();
        game.explode_rocket(&burst);
        assert_eq!(
            take_explosions(),
            vec![("minecraft:explode", [8.5, 202.0, 8.5], Some(Who::Player))],
            "the burst is heard once, at the rocket, from the player who shot it"
        );
        game.explode_rocket(&plain);
        assert_eq!(
            take_explosions(),
            vec![("minecraft:explode", [9.5, 202.0, 8.5], Some(Who::Player))],
            "a rocket with no explosions still posts it"
        );
        game.explode_rocket(&stray);
        assert_eq!(
            take_explosions(),
            vec![("minecraft:explode", [10.5, 202.0, 8.5], None)],
            "`getOwner()` is null: no source entity"
        );
        stop_events();
    });
}

/// `ItemStack.useOn` awards `ITEM_USED` for `FireworkRocketItem.useOn`'s
/// SUCCESS; `FireworkRocketItem.use` while gliding awards its own.
#[test]
#[ignore = "daily"]
fn launching_from_a_block_and_while_gliding_use_the_rocket_up() {
    with_game(|game| {
        game.creative = false;
        let stack = ItemStack::new("minecraft:firework_rocket", 1);
        let hit = sunrise_gameplay_common::game::Hit {
            inside: false,
            pos: [2, 199, 2],
            face: [0, 1, 0],
            point: [2.5, 200.0, 2.5],
        };
        for used in 1..=5 {
            let selected = game.selected;
            game.inventory[selected] = Some(stack.clone());
            assert!(game.launch_firework(&hit, &stack), "a rocket goes up");
            assert_eq!(
                game.stats.get("used", "minecraft:firework_rocket"),
                used,
                "each right-click on the block counts one use"
            );
        }
        game.player.as_mut().unwrap().fall_flying = true;
        let selected = game.selected;
        game.inventory[selected] = Some(stack.clone());
        assert!(game.boost_firework(&stack), "the gliding boost launches");
        assert_eq!(game.stats.get("used", "minecraft:firework_rocket"), 6);
    });
}
