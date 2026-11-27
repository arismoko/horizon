use std::sync::Arc;
use sunrise_core::{geometry::Vec3, position::ChunkPos, random::RandomSource};
use sunrise_level::chunk::Chunk;
use sunrise_sim::{
    mobs::Who,
    motion::PlayerMotion,
    projectiles::{Projectile, ProjectileKind},
};
use {
    crate::game::load_resources, crate::settings::Settings, sunrise_gameplay_common::game::Options,
    sunrise_gameplay_common::game::Progress,
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
    options.game_dir = Some(
        std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
            .join(format!("h114-{:?}", std::thread::current().id())),
    );
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
    game.build_registries();
    game.set_allow_commands(true);

    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("difficulty normal"));
    assert!(game.run_command("forceload add 0 0"));
    for (x, z) in [(0, 1), (1, 0), (1, 1)] {
        game.world
            .chunks
            .insert((x, z), Arc::new(Chunk::empty(x, z)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(x, z))
            .unwrap();
    }
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
    game.creative = true;
    assert!(game.run_command("fill 0 199 0 31 199 31 stone"));
    assert!(game.run_command("summon pig 13.5 200 2.5 {UUID:[I;114,0,0,99],NoAI:1b,NoGravity:1b}"));
    game.world.random.set_seed(114);
    test(&mut game);
}
pub(crate) fn start_events() {
    sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
        .with(|v| *v.borrow_mut() = Some(Vec::new()));
}
pub(crate) fn take_shots() -> Vec<(&'static str, [f64; 3], Option<Who>)> {
    sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS.with(|v| {
        std::mem::take(v.borrow_mut().as_mut().unwrap())
            .into_iter()
            .filter(|e| e.0 == "minecraft:projectile_shoot")
            .collect()
    })
}
pub(crate) fn stop_events() {
    sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
        .with(|v| *v.borrow_mut() = None);
}
pub(crate) fn owner(game: &sunrise_gameplay::game::ServerGame<'_>) -> u32 {
    game.mobs
        .iter()
        .find(|m| game.persist.uuid_of(m.id) == Some([114, 0, 0, 99]))
        .unwrap()
        .id
}
/// #14440: an orb's move adds its drop to the fall outside water; landing
/// after a fall posts HIT_GROUND and forgets the fall (`Entity.checkFallDamage`);
/// in water the drop adds nothing, so a landing posts nothing.
#[test]
#[ignore = "daily"]
fn a_landing_orb_posts_hit_ground_after_a_fall() {
    use sunrise_sim::entities::OrbMove;
    with_game(|game| {
        assert!(game.run_command("summon experience_orb 6.5 200 10.5 {Value:1,fall_distance:2d}"));
        let i = game.orbs.len() - 1;
        let from = Vec3::new(6.5, 200.5, 10.5);
        let down = Vec3::new(0.0, -0.5, 0.0);
        start_events();
        // In the air: the drop adds to the fall.
        game.orbs[i].ground.on_ground = false;
        game.orb_moved(
            i,
            OrbMove {
                from,
                moved: down,
                in_water: false,
            },
        );
        assert_eq!(game.orbs[i].fall_distance, 2.5);
        // On the ground: HIT_GROUND, then the fall is gone.
        game.orbs[i].ground.on_ground = true;
        game.orb_moved(
            i,
            OrbMove {
                from,
                moved: down,
                in_water: false,
            },
        );
        assert_eq!(game.orbs[i].fall_distance, 0.0);
        // In water with no fall: no drop, so no landing event.
        game.orb_moved(
            i,
            OrbMove {
                from,
                moved: down,
                in_water: true,
            },
        );
        let events: Vec<_> = sunrise_gameplay_common::game::side_ops::events::TEST_POSTED_EVENTS
            .with(|v| {
                std::mem::take(v.borrow_mut().as_mut().unwrap())
                    .into_iter()
                    .filter(|e| e.0 == "minecraft:hit_ground")
                    .collect()
            });
        stop_events();
        assert_eq!(events.len(), 1);
        assert_eq!(game.orbs[i].fall_distance, 0.0);
    });
}

#[test]
#[ignore = "daily"]
fn loaded_arrow_first_tick_dispatches_once_with_its_real_owner() {
    with_game(|game| {
        let owner = owner(game);
        // Already-shot is an independent silent control before the old failure.
        for (n, flag) in [(1, Some(1)), (2, None), (3, Some(0))] {
            let extra = flag
                .map(|f| format!(",HasBeenShot:{f}b"))
                .unwrap_or_default();
            assert!(game.run_command(&format!("summon arrow 6.5 201.5 10.5 {{UUID:[I;114,0,0,{n}],NoGravity:1b,Motion:[0d,0d,0d],Owner:[I;114,0,0,99]{extra}}}")));
            let id = game
                .creatures
                .projectiles
                .iter()
                .find(|p| game.persist.uuid_of(p.id) == Some([114, 0, 0, n]))
                .unwrap()
                .id;
            let p = game
                .creatures
                .projectiles
                .iter()
                .find(|p| p.id == id)
                .unwrap();
            assert_eq!(p.owner_uuid, Some([114, 0, 0, 99]));
            assert_eq!(p.arrow.as_ref().unwrap().has_been_shot, flag == Some(1));
            assert!(p.no_gravity);
            assert_eq!(p.velocity, Vec3::ZERO);
            start_events();
            game.tick_projectiles().unwrap();
            let p = game
                .creatures
                .projectiles
                .iter()
                .find(|p| p.id == id)
                .unwrap();
            assert!(p.arrow.as_ref().unwrap().has_been_shot);
            assert_eq!(p.position, Vec3::new(6.5, 201.5, 10.5));
            assert_eq!(p.velocity, Vec3::ZERO);
            let first = take_shots();
            game.tick_projectiles().unwrap();
            let second = take_shots();
            stop_events();
            assert!(second.is_empty(), "no repeated shot on tick2");
            let expected = if flag == Some(1) {
                Vec::new()
            } else {
                vec![(
                    "minecraft:projectile_shoot",
                    [6.5, 201.5, 10.5],
                    Some(Who::Mob(owner)),
                )]
            };
            assert_eq!(first, expected, "actual first tick, input={flag:?}");
            assert!(game.run_command(&format!(
                "kill {}",
                sunrise_protocol::crypto::uuid_string(sunrise_sim::lan::playerdata::ints_uuid([
                    114, 0, 0, n
                ]))
            )));
        }
    });
}
pub(crate) fn natural(kind: &str) {
    with_game(|game| {
        let shooter = owner(game);
        match kind {
            "bow"=>{
                assert!(game.run_command("item replace entity @s weapon.mainhand with bow"));
                assert!(game.run_command("item replace entity @s hotbar.1 with arrow 4"));
            },
            "crossbow"=>assert!(game.run_command("item replace entity @s weapon.mainhand with crossbow[charged_projectiles=[{id:\"minecraft:arrow\",count:1}]]")),
            _=>{}
        }
        let clock = game.world.game_time;
        start_events();
        match kind {
            "bow" => {
                let bow = game.held().unwrap().clone();
                game.release_bow(&bow, 20);
            }
            "crossbow" => {
                let bow = game.held().unwrap().clone();
                assert!(game.use_crossbow(&bow));
            }
            "mob" => {
                let mut p = Projectile::new(
                    ProjectileKind::Arrow {
                        base_damage: 2.0,
                        critical: false,
                        effect: None,
                    },
                    Vec3::new(6.5, 201.5, 6.5),
                    Some(Who::Mob(shooter)),
                );
                p.velocity = Vec3::new(0.0, 0.0, 1.0);
                game.launch_mob_projectile(p);
            }
            _ => unreachable!(),
        }
        assert_eq!(game.creatures.projectiles.len(), 1, "real launch {kind}");
        let p = &game.creatures.projectiles[0];
        let id = p.id;
        assert!(id > 0);
        assert!(
            !p.arrow.as_ref().unwrap().has_been_shot,
            "saved pre-tick state"
        );
        let launch = take_shots();
        game.tick_projectiles().unwrap();
        let p = game
            .creatures
            .projectiles
            .iter()
            .find(|p| p.id == id)
            .unwrap();
        assert!(p.arrow.as_ref().unwrap().has_been_shot);
        let at = [p.position.x, p.position.y, p.position.z];
        let first = take_shots();
        game.tick_projectiles().unwrap();
        let second = take_shots();
        stop_events();
        assert_eq!(
            game.world.game_time, clock,
            "fixture directly invokes actual projectile ticks; runtime clocks independently admitted"
        );
        println!("H114_RNG {kind} {}", game.world.random.next_long());
        assert_eq!(
            [launch.len(), first.len(), second.len()],
            [0, 1, 0],
            "actual launch/tick1/tick2 counts {kind}"
        );
        assert_eq!(
            first[0],
            (
                "minecraft:projectile_shoot",
                at,
                Some(if kind == "mob" {
                    Who::Mob(shooter)
                } else {
                    Who::Player
                })
            )
        );
    });
}
#[test]
#[ignore = "daily"]
fn bow_launch_uses_only_the_first_tick_event() {
    natural("bow")
}
#[test]
#[ignore = "daily"]
fn crossbow_launch_uses_only_the_first_tick_event() {
    natural("crossbow")
}
#[test]
#[ignore = "daily"]
fn mob_launch_uses_only_the_first_tick_event() {
    natural("mob")
}
#[test]
#[ignore = "daily"]
fn rocket_and_mob_trident_post_only_on_their_first_tick() {
    with_game(|game| {
        assert!(game.run_command("item replace entity @s weapon.mainhand with crossbow[charged_projectiles=[{id:\"minecraft:firework_rocket\",count:1}]]"));
        start_events();
        let bow = game.held().unwrap().clone();
        assert!(game.use_crossbow(&bow));
        assert_eq!(game.rockets.len(), 1);
        assert!(take_shots().is_empty(), "no rocket shot before its tick");
        game.tick_rockets();
        assert_eq!(take_shots().len(), 1, "rocket first tick");
        game.tick_rockets();
        assert!(take_shots().is_empty(), "no repeated rocket shot");
        stop_events();
        for route in ["flight", "finish_hit", "breeze"] {
            if route == "breeze" {
                assert!(game.run_command("summon breeze 6.5 201.5 7.5 {NoAI:1b,NoGravity:1b}"));
            }
            let mut p = Projectile::new(
                ProjectileKind::Trident,
                Vec3::new(6.5, 201.5, 6.5),
                Some(Who::Mob(owner(game))),
            );
            p.velocity = Vec3::new(0.0, 0.0, 1.0);
            start_events();
            game.launch_mob_projectile(p);
            let id = game.creatures.projectiles.last().unwrap().id;
            assert!(
                take_shots().is_empty(),
                "no trident shot before its tick: {route}"
            );
            if route == "finish_hit" {
                let p = game.creatures.projectiles.pop().unwrap();
                game.finish_arrow_hit(p, sunrise_sim::entities::arrows::EntityHit::Deflected);
            } else {
                game.tick_projectiles().unwrap();
            }
            assert_eq!(take_shots().len(), 1, "trident first tick: {route}");
            if route == "breeze" {
                let p = game
                    .creatures
                    .projectiles
                    .iter()
                    .find(|p| p.id == id)
                    .unwrap();
                assert!(
                    p.velocity.z < 0.0,
                    "the actual breeze deflected the trident"
                );
            }
            game.tick_projectiles().unwrap();
            assert!(take_shots().is_empty(), "no repeated trident shot: {route}");
            stop_events();
        }
    });
}
#[test]
#[ignore = "daily"]
fn finish_hit_dispatches_once_and_the_next_tick_does_not_repeat_it() {
    with_game(|game| {
        assert!(game.run_command("summon arrow 6.5 201.5 10.5 {UUID:[I;114,0,0,20],NoGravity:1b,Motion:[0d,0d,0d],Owner:[I;114,0,0,99]}"));
        let mut arrow = game.creatures.projectiles.pop().unwrap();
        let owner = owner(game);
        assert_eq!(arrow.owner_uuid, Some([114, 0, 0, 99]));
        // Resolve through the actual registered owner, as tick_projectiles does.
        arrow.owner = game.who_by_uuid(arrow.owner_uuid.unwrap());
        assert_eq!(arrow.owner, Some(Who::Mob(owner)));
        assert!(!arrow.arrow.as_ref().unwrap().has_been_shot);
        let id = arrow.id;
        start_events();
        game.finish_arrow_hit(arrow, sunrise_sim::entities::arrows::EntityHit::Deflected);
        let first = take_shots();
        assert!(
            game.creatures
                .projectiles
                .iter()
                .find(|p| p.id == id)
                .unwrap()
                .arrow
                .as_ref()
                .unwrap()
                .has_been_shot
        );
        game.tick_projectiles().unwrap();
        assert!(take_shots().is_empty());
        stop_events();
        assert_eq!(
            first,
            vec![(
                "minecraft:projectile_shoot",
                [6.5, 201.5, 10.5],
                Some(Who::Mob(owner))
            )]
        );
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
