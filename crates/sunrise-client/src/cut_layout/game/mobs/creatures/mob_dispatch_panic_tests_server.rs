use mod_api::__private::{HookPanicReport, take_hook_panics};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use sunrise_core::geometry::Vec3;
use sunrise_core::random::{LegacyRandom, RandomSource};
use sunrise_gameplay::game::creatures::mob_dispatch_panic_tests::Case;
use sunrise_gameplay::game::creatures::mob_dispatch_panic_tests::LEVEL_VALUE;
use sunrise_gameplay::game::creatures::mob_dispatch_panic_tests::PanicOnDespawn;
use sunrise_gameplay::game::creatures::mob_dispatch_panic_tests::assert_resident_mobs;
use sunrise_gameplay_common::game::Dim;
use sunrise_level::chunk::Chunk;
use sunrise_mod_runtime::attachments::Holder;
use sunrise_sim::join_seq::residency::{
    Change, Journal, Kind, Owner,
    table::{EntityLookup, EntityTable, SourceEntity},
};
use sunrise_sim::mobs::Mob;
use {
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

#[test]
#[ignore = "daily"]
fn mob_despawn_listener_panic_preserves_owned_rows_and_lent_random() {
    const CHILD: &str = "SUNRISE_MOB_DESPAWN_PANIC_CHILD";
    if run_fresh_cases(
        "cut_layout::game::server_side::creatures_server::mob_dispatch_panic_tests::mob_despawn_listener_panic_preserves_owned_rows_and_lent_random",
        CHILD,
        &[
            "first-overworld",
            "first-nether",
            "later-overworld",
            "later-nether",
            "pending_death-overworld",
            "pending_death-nether",
            "unloaded_row-overworld",
            "unloaded_row-nether",
            "normal-overworld",
            "normal-nether",
            "normal_removal-overworld",
            "normal_removal-nether",
        ],
    ) {
        return;
    }
    let scenario = std::env::var(CHILD).unwrap();
    let (case, dimension) = scenario.split_once('-').unwrap();
    let case = match case {
        "first" => Case::First,
        "later" => Case::Later,
        "pending_death" => Case::PendingDeath,
        "unloaded_row" => Case::UnloadedRow,
        "normal" => Case::Normal,
        "normal_removal" => Case::NormalRemoval,
        _ => panic!("unknown mob despawn case {scenario}"),
    };
    let dimension = match dimension {
        "overworld" => Dim::Overworld,
        "nether" => Dim::Nether,
        _ => panic!("unknown mob despawn dimension {scenario}"),
    };
    let at = match case {
        Case::First | Case::UnloadedRow => Some(1),
        Case::Later | Case::PendingDeath => Some(2),
        Case::Normal | Case::NormalRemoval => None,
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let mods = vec![mod_api::ModEntry::new(
        "panic-on-mob-despawn",
        Box::new(PanicOnDespawn {
            calls: calls.clone(),
            at,
            force_allow: case == Case::NormalRemoval,
        }),
    )]
    .into();
    let root = tempfile::tempdir().unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.server.world_root = root.path().to_owned();
            game.enter_level(dimension).unwrap();
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            assert!(
                game.entity_ticking_area()(1, 0),
                "fixture chunk is not entity-ticking"
            );
            if case == Case::UnloadedRow {
                assert!(!game.world.loaded(33, 0), "fixture zombie is not unloaded");
            }
            let journal = Journal::default();
            let owner = Owner {
                dimension: game.dim.key(),
                kind: Kind::Mob,
            };
            game.mobs
                .bind_residency(&journal, owner, |mob| (mob.id, mob.join));
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            for id in [71, 72, 73, 74] {
                let unloaded = id == 71 && case == Case::UnloadedRow;
                let kind = if unloaded {
                    game.res
                        .mob_types()
                        .get("minecraft:zombie")
                        .unwrap()
                        .clone()
                } else {
                    pig.clone()
                };
                let x = if unloaded { 33.0 } else { f64::from(id - 70) };
                let mut mob = Mob::new(id, kind, Vec3::new(x, 80.0, 0.5), 0.0);
                mob.no_ai = true;
                if id == 71 && case == Case::PendingDeath {
                    // Alive at the initial death sweep; the actual base
                    // tick's burning damage kills it before listener 2.
                    mob.health = 0.5;
                    mob.fire_ticks = 20;
                }
                if id == 74 {
                    // A previously notified corpse is a valid retained
                    // death marker, unlike an orphan ID in this set.
                    mob.health = 0.0;
                    game.server.died.insert(id);
                }
                game.mobs.push(mob);
            }
            let joined: Vec<_> = game.mobs.iter().map(|mob| (mob.id, mob.join)).collect();
            let (removed, expected) = joined.split_at(match case {
                Case::UnloadedRow => 1,
                Case::NormalRemoval => joined.len(),
                _ => 0,
            });
            LEVEL_VALUE
                .set(&mut game.world.attachments, Holder::Level, 42)
                .unwrap();
            game.world.attachment_syncs.insert(Holder::Level);
            let expected_syncs = game.world.attachment_syncs.clone();
            game.world.path_writes = Some(Vec::new());
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(4, 80, 4, stone).unwrap();
            assert_eq!(game.world.path_writes.as_ref().unwrap().len(), 1);
            game.world.random =
                sunrise_core::random::LevelRandom::for_level(LegacyRandom::new(0x5eed));
            let expected_draw = game.world.random.clone().next_long();
            game.tick_creature_entities().unwrap();
            let first_pass_calls = calls.load(Ordering::SeqCst);
            assert_eq!(first_pass_calls, at.unwrap_or(joined.len()));
            let reports = take_hook_panics();
            if at.is_some() {
                assert_eq!(
                    reports,
                    [HookPanicReport {
                        mod_id: "panic-on-mob-despawn",
                        hook: "mob_despawn",
                    }]
                );
            } else {
                assert!(reports.is_empty());
            }
            assert_eq!(
                game.world.path_writes,
                Some(Vec::new()),
                "the first pass did not consume collision writes"
            );
            assert_eq!(game.server.died.contains(&71), case == Case::PendingDeath);
            assert_eq!(
                game.server.died.contains(&74),
                case != Case::NormalRemoval,
                "prior death notification was forgotten or a removed corpse retained"
            );
            assert_eq!(
                LEVEL_VALUE
                    .get_existing(&mut game.world.attachments, Holder::Level)
                    .unwrap(),
                Some(42)
            );
            assert_eq!(
                game.world.attachment_syncs, expected_syncs,
                "pending attachment syncs were lost"
            );
            if case != Case::PendingDeath {
                assert_eq!(
                    game.world.random.clone().next_long(),
                    expected_draw,
                    "lent world random was lost"
                );
            }
            if case == Case::PendingDeath {
                assert!(!game.mobs[0].alive(), "real burning death was not reached");
                assert!(
                    game.creatures.ticked.contains(&71),
                    "prior real tick was lost"
                );
            }
            let mut events = Vec::new();
            journal.drain_into(&mut events);
            let (mut depth, mut lifts) = (0usize, 0usize);
            for event in events {
                assert_eq!(event.owner, owner);
                match event.change {
                    Change::Lifted => {
                        depth += 1;
                        lifts += 1;
                    }
                    Change::Restored => {
                        depth = depth.checked_sub(1).expect("unmatched mob restore")
                    }
                    _ => {}
                }
                journal.push(event);
            }
            assert!(lifts > 0, "actual lifting boundary was not observed");
            assert_eq!(depth, 0, "mob listener abandoned the real lifted pass");
            assert_eq!(
                game.mobs
                    .iter()
                    .map(|mob| (mob.id, mob.join))
                    .collect::<Vec<_>>(),
                expected,
                "mob listener lost or reordered original rows"
            );
            let mut table = EntityTable::new(journal);
            assert_resident_mobs(game, &mut table, &owner, expected);
            for &(id, _) in removed {
                let found = table.entity_lookup(&owner.dimension, id, |at| {
                    let mob = &game.mobs[game.mobs.locate(at.join)?];
                    Some(SourceEntity {
                        id: mob.id,
                        value: (mob.id, mob.join),
                    })
                });
                assert!(matches!(found, EntityLookup::Missing));
            }
            game.tick_creature_entities().unwrap();
            if at.is_some() {
                assert_eq!(calls.load(Ordering::SeqCst), first_pass_calls);
            }
            assert!(take_hook_panics().is_empty());
            assert_eq!(
                game.mobs
                    .iter()
                    .map(|mob| (mob.id, mob.join))
                    .collect::<Vec<_>>(),
                expected,
                "failed listener answer leaked into the next despawn check"
            );
            assert_resident_mobs(game, &mut table, &owner, expected);
        });
    });
}
