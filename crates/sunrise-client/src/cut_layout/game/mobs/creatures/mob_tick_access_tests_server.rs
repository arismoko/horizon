use crate::game::creatures::borrowed_tick::mob_tick_access_tests::DespawnProbe;
use crate::game::creatures::borrowed_tick::mob_tick_access_tests::DespawnRecords;
use crate::game::creatures::borrowed_tick::mob_tick_access_tests::ExtendLife;
use crate::game::creatures::borrowed_tick::mob_tick_access_tests::MoveOnArrowPost;
use crate::game::creatures::borrowed_tick::mob_tick_access_tests::TickTiming;
use mod_api::ModEntry;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU32, Ordering},
};
use sunrise_core::geometry::Vec3;
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_gameplay_common::game::Dim;
use sunrise_items::items::ItemStack;
use sunrise_level::chunk::Chunk;
use sunrise_sim::mobs::Mob;

/// As on NeoForge, where each projectile ticks in turn: a mod that moves a mob in one arrow's
/// Post moves it for the next arrow of the same tick, which hits it there.
#[test]
#[ignore = "daily"]
fn a_mob_moved_in_one_arrows_post_is_hit_by_the_next_arrow() {
    let first = Arc::new(AtomicU32::new(0));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "projectile-post-move",
        Box::new(MoveOnArrowPost {
            first: first.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_two_pigs(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            // The first arrow far from both pigs; the second flies east along z = 5.5, where
            // no pig stands until the first arrow's Post moves one there.
            for command in [
                "summon minecraft:arrow 1.5 80.5 12.5 {NoGravity:1b,Motion:[0.1d,0.0d,0.0d]}",
                "summon minecraft:arrow 1.5 80.5 5.5 {NoGravity:1b,Motion:[3.0d,0.0d,0.0d]}",
            ] {
                assert!(game.run_command(command), "{command}");
            }
            first.store(game.creatures.projectiles[0].id, Ordering::Relaxed);
            let health = game.mobs.iter().find(|m| m.id == 72).unwrap().health;
            game.server.ticked_entities_through = u32::MAX;
            game.tick_projectiles().unwrap();
            let pig = game.mobs.iter().find(|m| m.id == 72).unwrap();
            assert_eq!(pig.position.z, 5.5, "the first arrow's Post moved the pig");
            assert!(
                pig.health < health,
                "the second arrow hit the pig where the Post moved it: {} of {health}",
                pig.health
            );
        });
    });
}

/// A mob in a loaded chunk whose own status is below entity-ticking is not on the entity tick
/// list: it fires no `mob_despawn` and stays, while a mob in the forced chunk beside it checks
/// despawning as usual.
#[test]
#[ignore = "daily"]
fn a_mob_outside_the_entity_tick_list_checks_no_despawn() {
    let records = Arc::new(Mutex::new(DespawnRecords::default()));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "mob-tick-listed",
        Box::new(DespawnProbe {
            records: records.clone(),
            allow: Arc::new(AtomicU32::new(72)),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_two_pigs(|game| {
            game.world
                .chunks
                .insert((1, 0), Arc::new(Chunk::empty(1, 0)));
            game.mobs[1].position = Vec3::new(24.5, 80.0, 0.5);
            // Only chunk (0, 0) is forced: (1, 0) is loaded at the next level out.
            game.server
                .tickets
                .0
                .update(Some(sunrise_sim::ticket_levels::Sources {
                    forced: vec![(0, 0)],
                    ..Default::default()
                }));
            assert!(game.entity_tick_listed_area()(1, 0));
            assert!(game.world.loaded(24, 0) && !game.entity_tick_listed_area()(24, 0));
            game.tick_creature_entities().unwrap();
            let heard: Vec<u32> = records.lock().unwrap().0.iter().map(|r| r.0).collect();
            assert_eq!(heard, [71], "only the listed mob checks despawning");
            assert!(
                game.mobs.iter().any(|mob| mob.id == 72),
                "the unlisted mob stays, though its listener would allow the despawn"
            );
        });
    });
}

/// The item family keeps its own contract while the mob slice changes: the
/// item tick still lifts its list, and an `item_expire` listener still edits
/// the item's lifetime, which the engine applies when the tick ends.
#[test]
#[ignore = "daily"]
fn item_lifetime_edit_in_the_item_tick_still_lands() {
    let named = Arc::new(AtomicU32::new(0));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "item-lifetime-access",
        Box::new(ExtendLife { id: named.clone() }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        let root = std::env::temp_dir().join(format!(
            "sunrise-item-lifetime-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        with_game(|game| {
            game.server.world_root = root.clone();
            game.enter_level(Dim::Overworld).unwrap();
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.push_item_entity(
                ItemStack::new("minecraft:apple", 1),
                [1.5, 64.0, 1.5],
                Some([0.0, 0.0, 0.0]),
                0,
                None,
            );
            let id = game.dropped.last().unwrap().id;
            named.store(id, Ordering::Relaxed);
            game.server.ticked_entities_through = id;
            game.dropped.last_mut().unwrap().age = 6000;
            let mut round = game.native_tick_round();
            game.tick_dropped_items(&mut round);
            let item = game
                .dropped
                .iter()
                .find(|item| item.id == id)
                .expect("the extended item stayed");
            assert_eq!(
                item.lifespan, 6100,
                "the listener's 100 ticks extended the item's lifespan"
            );
            assert_eq!(
                item.age, 6001,
                "extending lifespan leaves the item's age intact"
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    });
}

/// Two arrows of a type a mod names, on the server: each told alone, its Post right after its own
/// flight and before the next arrow's Pre, and a Post handle reads where its flight took it.
#[test]
#[ignore = "daily"]
fn named_projectiles_hear_each_their_own_pre_and_post() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "projectile-tick-timing",
        Box::new(TickTiming {
            replace_cancels: false,
            heard: heard.clone(),
        }),
    )]
    .into();
    let mut ids = Vec::new();
    mod_platform::testing::with_mods(mods, || {
        with_two_pigs(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            for z in [0.5, 2.5] {
                assert!(game.run_command(&format!(
                    "summon minecraft:arrow 1.5 80.5 {z} {{NoGravity:1b,Motion:[0.5d,0.0d,0.0d]}}"
                )));
            }
            ids = game.creatures.projectiles.iter().map(|p| p.id).collect();
            game.server.ticked_entities_through = u32::MAX;
            game.tick_projectiles().unwrap();
        });
    });
    let heard = heard.lock().unwrap();
    let phases: Vec<_> = heard
        .iter()
        .map(|(phase, entities, _)| (*phase, entities.clone()))
        .collect();
    assert_eq!(
        phases,
        [
            ("pre", vec![ids[0]]),
            ("post", vec![ids[0]]),
            ("pre", vec![ids[1]]),
            ("post", vec![ids[1]]),
        ]
    );
    let first = heard[1].2.expect("the Post reads the first arrow");
    assert!(
        first[0] > 1.5,
        "the first arrow flew in its own tick: {first:?}"
    );
}

/// Two idle pigs in an entity-ticking chunk, at x = 1 and x = 2, ready for
/// the real creature pass.
pub(crate) fn with_two_pigs(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    let root = std::env::temp_dir().join(format!(
        "sunrise-mob-tick-access-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    with_game(|game| {
        game.server.world_root = root.clone();
        game.enter_level(Dim::Overworld).unwrap();
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        assert!(
            game.entity_ticking_area()(1, 0),
            "fixture chunk is not entity-ticking"
        );
        let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        for (id, x) in [(71u32, 1.0f64), (72u32, 2.0f64)] {
            let mut mob = Mob::new(id, kind.clone(), Vec3::new(x, 80.0, 0.5), 0.0);
            mob.no_ai = true;
            mob.no_gravity = true;
            mob.ground.on_ground = true;
            game.mobs.push(mob);
        }
        test(game);
    });
    let _ = std::fs::remove_dir_all(&root);
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_ticks::NativeTicksGameExt1 as _;
