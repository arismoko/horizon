use crate::game::creatures::borrowed_panic_tests::BreathPanic;
use crate::game::creatures::borrowed_panic_tests::Breather;
use crate::game::creatures::borrowed_panic_tests::Consumer;
use crate::game::creatures::borrowed_panic_tests::ConsumerProbe;
use crate::game::creatures::borrowed_panic_tests::FAILING_MOD;
use crate::game::creatures::borrowed_panic_tests::Family;
use crate::game::creatures::borrowed_panic_tests::Hooks;
use crate::game::creatures::borrowed_panic_tests::LEVEL_SEED;
use crate::game::creatures::borrowed_panic_tests::Output;
use crate::game::creatures::borrowed_panic_tests::OutputNotice;
use crate::game::creatures::borrowed_panic_tests::PanicToken;
use crate::game::creatures::borrowed_panic_tests::Probe;
use crate::game::creatures::borrowed_panic_tests::Scratch;
use crate::game::creatures::borrowed_panic_tests::VALUE;
use crate::game::creatures::borrowed_panic_tests::assert_panic_report;
use crate::game::creatures::borrowed_panic_tests::eggs;
use crate::game::creatures::borrowed_panic_tests::fixture_game;
use crate::game::creatures::borrowed_panic_tests::fixture_mob;
use crate::game::creatures::borrowed_panic_tests::guest_health;
use crate::game::creatures::borrowed_panic_tests::prime_hook;
use crate::game::creatures::borrowed_panic_tests::queue_egg;
use crate::game::creatures::borrowed_panic_tests::tick_in_level;
use mod_api::__private::take_hook_panics;
use std::sync::Arc;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
use sunrise_core::geometry::Vec3;
use sunrise_core::random::LegacyRandom;
use sunrise_core::random::RandomSource;
use sunrise_gameplay::game::native_entity_refs::tests::with_game;
use sunrise_gameplay_common::game::Dim;
use sunrise_gameplay_common::game::creatures::MobOutputBatch;
use sunrise_gameplay_common::game::creatures::NormalMobActions;
use sunrise_level::chunk::Chunk;
use sunrise_mod_runtime::attachments::Holder;
use sunrise_sim::join_seq::residency::{
    Change, Journal, Kind, Owner,
    table::{EntityLookup, EntityTable, SourceEntity},
};
use sunrise_sim::mobs::Mob;
use sunrise_sim::mobs::MobEvent;

/// One tick of a zombie with 100 air and an iron sword: the air it listened to, then the air
/// and the sword's damage the tick left.
pub(crate) fn breathing_tick(meddle: bool) -> (Vec<i32>, i32, i32) {
    crate::entry::install_client_hooks();
    let _ = sunrise_sim::block_contact::HOOKS
        .set(sunrise_gameplay_common::game::native_hooks::BLOCK_CONTACT_HOOKS);
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "sunrise-breathing-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    )));
    let seen_air = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut left = (0, 0);
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "breathing",
            Box::new(Breather {
                meddle,
                seen_air: seen_air.clone(),
            }),
        )]
        .into(),
        || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                game.server.world_root = scratch.0.clone();
                game.enter_level(Dim::Overworld).unwrap();
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                assert!(game.entity_ticking_area()(1, 0));
                let kind = game
                    .res
                    .mob_types()
                    .get("minecraft:zombie")
                    .unwrap()
                    .clone();
                let mut mob = Mob::new(72, kind, Vec3::new(2.0, 80.0, 0.5), 0.0);
                mob.no_ai = true;
                mob.air = 100;
                mob.extra
                    .set_main_hand_stack(Some(sunrise_items::items::ItemStack::new(
                        "minecraft:iron_sword",
                        1,
                    )));
                game.mobs.push(mob);
                game.tick_creature_entities().unwrap();
                let mob = game.mobs.iter().find(|m| m.id == 72).unwrap();
                left = (mob.air, mob.extra.main_hand_stack().unwrap().damage);
            });
        },
    );
    let seen = seen_air.lock().unwrap().clone();
    (seen, left.0, i32::try_from(left.1).unwrap())
}

#[test]
#[ignore = "daily"]
fn retained_breathing_tail() {
    crate::entry::install_client_hooks();
    let _ = sunrise_sim::block_contact::HOOKS
        .set(sunrise_gameplay_common::game::native_hooks::BLOCK_CONTACT_HOOKS);
    let calls = Arc::new(AtomicUsize::new(0));
    let scratch =
        Scratch(std::env::temp_dir().join(format!("sunrise-breath-panic-{}", std::process::id())));
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "breath-panic",
            Box::new(BreathPanic(calls.clone())),
        )]
        .into(),
        || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                game.server.world_root = scratch.0.clone();
                game.enter_level(Dim::Overworld).unwrap();
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let kind = game
                    .res
                    .mob_types()
                    .get("minecraft:zombie")
                    .unwrap()
                    .clone();
                let mut mob = Mob::new(72, kind, Vec3::new(2.0, 80.0, 0.5), 0.0);
                mob.no_ai = true;
                mob.air = 100;
                mob.extra
                    .set_main_hand_stack(Some(sunrise_items::items::ItemStack::new(
                        "minecraft:iron_sword",
                        1,
                    )));
                game.mobs.push(mob);
                game.tick_creature_entities().unwrap();
                assert_eq!(calls.load(Ordering::SeqCst), 1);
                let mob = game.mobs.iter().find(|m| m.id == 72).unwrap();
                assert_eq!(mob.air, 100);
                assert_eq!(mob.extra.main_hand_stack().unwrap().damage, 0);
                assert_eq!(
                    mod_api::__private::take_hook_panics(),
                    [mod_api::__private::HookPanicReport {
                        mod_id: "breath-panic",
                        hook: "living_breathe",
                    }]
                );
                // Next tick skips the poisoned listener and applies vanilla breathing (+4 air).
                game.tick_creature_entities().unwrap();
                assert_eq!(calls.load(Ordering::SeqCst), 1);
                let mob = game.mobs.iter().find(|m| m.id == 72).unwrap();
                assert_eq!(mob.air, 104);
                assert_eq!(mob.extra.main_hand_stack().unwrap().damage, 0);
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn retained_stale_push_target() {
    let scratch =
        Scratch(std::env::temp_dir().join(format!("sunrise-stale-push-{}", std::process::id())));
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        game.server.world_root = scratch.0.clone();
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.common
            .creatures
            .pending
            .entry(game.common.dim.id())
            .or_default()
            .pushes
            .push(999, Vec3::new(0.25, 0.0, 0.0));
        game.tick_creature_entities().unwrap();
        let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        let mut mob = Mob::new(999, kind, Vec3::new(1.0, 80.0, 0.5), 0.0);
        mob.no_ai = true;
        game.mobs.push(mob);
        game.tick_creature_entities().unwrap();
        assert_eq!(
            game.mobs[0].velocity.x, 0.0,
            "obsolete target push reached a later occupant"
        );
    });
}

pub(crate) fn run_fixture(family: Family, output: Output, dim: Dim) {
    crate::entry::install_client_hooks();
    let _ = sunrise_sim::block_contact::HOOKS
        .set(sunrise_gameplay_common::game::native_hooks::BLOCK_CONTACT_HOOKS);
    if family == Family::Knockback {
        assert!(sunrise_level::brewing::register_mod_effects([
            sunrise_level::brewing::ModEffect::new(
                "borrowed-panic:hit",
                0,
                sunrise_level::brewing::Category::Neutral,
                FAILING_MOD,
            ),
        ]));
    }
    let scratch = tempfile::tempdir().unwrap();
    let goal_payload = family.is_goal().then(|| Box::new(PanicToken(0xdecaf)));
    let payload_address = goal_payload
        .as_deref()
        .map(|token| token as *const PanicToken as usize);
    let probe = Arc::new(Probe {
        family,
        retained: Mutex::default(),
        calls: AtomicUsize::new(0),
        posts: Mutex::default(),
        first_proposal: Mutex::default(),
        healthy_proposals: Mutex::default(),
        draw: Mutex::default(),
        outputs: Mutex::default(),
        goal_payload: Mutex::new(goal_payload),
    });
    let entries = || {
        let mut entries = vec![mod_api::ModEntry::new(
            FAILING_MOD,
            Box::new(Hooks {
                probe: probe.clone(),
                panics: true,
            }),
        )];
        if !family.is_goal() {
            entries.push(mod_api::ModEntry::new(
                "healthy",
                Box::new(Hooks {
                    probe: probe.clone(),
                    panics: false,
                }),
            ));
        }
        entries
    };
    mod_platform::testing::with_mods(entries().into(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&entries()).unwrap();
        fixture_game(output, |game| {
            game.server.world_root = scratch.path().to_path_buf();
            game.enter_level(dim).unwrap();
            game.options.dedicated = Some(Vec::new());
            if output != Output::Quiet {
                game.load_server_loot();
            }
            let guests = matches!(output, Output::Guest | Output::Mixed);
            if guests {
                game.world.health_rules.difficulty = sunrise_world::food::Difficulty::Normal;
                let host = game.server.lan.as_mut().unwrap();
                for i in 0..host.hub.guests.len() {
                    host.hub.guests.update(i, |g| {
                        g.dimension = dim.id().to_owned();
                        g.position = if g.conn == 12 {
                            [1.5, 80.0, 0.5]
                        } else {
                            [12.0, 80.0, 12.0]
                        };
                        g.loaded = true;
                        g.load_timeout = 0;
                    });
                }
                assert_eq!(game.lan_guest_views().len(), 2);
            }
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            assert!(game.entity_ticking_area()(1, 0));
            if output == Output::Reset {
                let old_chicken = fixture_mob(game, 71, family, output);
                game.mobs.push(old_chicken);
                queue_egg(game);
                game.clear_entities();
            }
            let journal = Journal::default();
            let owner = Owner {
                dimension: game.dim.key(),
                kind: Kind::Mob,
            };
            game.mobs
                .bind_residency(&journal, owner, |m| (m.id, m.join));
            let ids: &[u32] = match output {
                Output::Push => &[70, 71, 72],
                Output::Mixed => &[71, 72, 73],
                _ => &[71, 72],
            };
            for &id in ids {
                let mob = fixture_mob(game, id, family, output);
                game.mobs.push(mob);
            }
            if output == Output::Mixed {
                // A guest action already queued in an older batch must beat the egg produced by 73.
                game.common
                    .creatures
                    .pending
                    .entry(game.common.dim.id())
                    .or_default()
                    .batches
                    .push_back(MobOutputBatch {
                        normal: NormalMobActions::default(),
                        guests: vec![(12, 71, MobEvent::AttackPlayer(2.0, [1.0, 80.0, 0.5], None))],
                    });
            }
            game.server.last_hurt_by_player.insert(
                71,
                sunrise_gameplay_common::game::combat_common::PlayerCredit(99),
            );
            let rows: Vec<_> = game.mobs.iter().map(|m| (m.id, m.join)).collect();
            VALUE
                .set(&mut game.world.attachments, Holder::Level, 42)
                .unwrap();
            game.world.attachment_syncs.insert(Holder::Level);
            let syncs = game.world.attachment_syncs.clone();
            game.world.path_writes = Some(Vec::new());
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(4, 80, 4, stone).unwrap();
            let floor_depth = if output != Output::Quiet {
                5
            } else if family == Family::MountedGoal {
                3
            } else {
                0
            };
            for x in 0..5 {
                for z in 0..floor_depth {
                    game.world.set(x, 79, z, stone).unwrap();
                }
            }
            if matches!(family, Family::Movement | Family::Landing) {
                game.world.set(2, 79, 0, stone).unwrap();
            }
            let writes = game.world.path_writes.clone();
            assert!(writes.as_ref().is_some_and(|writes| !writes.is_empty()));
            game.world.random =
                sunrise_core::random::LevelRandom::for_level(LegacyRandom::new(LEVEL_SEED));
            let guest_before = guests.then(|| guest_health(game));
            let push_before = game.mobs[0].velocity.x;
            let target = ids.iter().position(|&id| id == 72).unwrap();
            let health_before = game.mobs[target].health;

            let unwind = if family.is_goal() {
                // Registered goals keep their own unwind contract; this is not hook dispatch.
                Some(
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tick_in_level(game)))
                        .expect_err("native goal panic was swallowed"),
                )
            } else {
                tick_in_level(game).unwrap();
                None
            };
            assert_eq!(
                probe.calls.load(Ordering::SeqCst),
                1,
                "first callback entry"
            );
            if let Some(payload) = &unwind {
                let posts = std::mem::take(&mut *probe.posts.lock().unwrap());
                assert!(
                    !posts.contains(&72),
                    "aborted tick fired Post for the panicking mob"
                );
                assert_eq!(
                    posts,
                    [71],
                    "earlier completed mob fired its per-entity Post before the panic"
                );
                let token = payload
                    .downcast_ref::<PanicToken>()
                    .expect("original typed payload");
                assert_eq!(Some(token as *const PanicToken as usize), payload_address);
                assert_eq!(token.0, 0xdecaf);
                assert_eq!(game.world.path_writes, writes);
            } else {
                assert_panic_report(FAILING_MOD, family.hook());
                assert_eq!(std::mem::take(&mut *probe.posts.lock().unwrap()), ids);
                let before = probe
                    .first_proposal
                    .lock()
                    .unwrap()
                    .expect("failing listener's input");
                let healthy = std::mem::take(&mut *probe.healthy_proposals.lock().unwrap());
                assert_eq!(
                    healthy.first(),
                    Some(&before),
                    "later callback did not see the last accepted proposal"
                );
                assert_eq!(
                    game.world.path_writes,
                    Some(Vec::new()),
                    "tick did not consume its path writes"
                );
                match family {
                    Family::Damage | Family::Shield | Family::Landing => {
                        assert!(
                            game.mobs[target].health < health_before,
                            "failed proposal stopped vanilla damage"
                        );
                    }
                    Family::Totem => {
                        assert!(
                            game.mobs[target].health > 0.0,
                            "failed proposal vetoed the totem"
                        );
                        assert!(
                            game.mobs[target].extra.main_hand_stack().is_none(),
                            "totem was not consumed"
                        );
                    }
                    _ => {}
                }
            }

            let mut pending = Vec::new();
            journal.drain_into(&mut pending);
            let (mut depth, mut lifts) = (0usize, 0usize);
            for e in pending {
                assert_eq!(e.owner, owner);
                match e.change {
                    Change::Lifted => {
                        depth += 1;
                        lifts += 1;
                    }
                    Change::Restored => depth = depth.checked_sub(1).unwrap(),
                    _ => {}
                }
                journal.push(e);
            }
            assert!(lifts > 0);
            assert_eq!(depth, 0, "borrowed region was not restored");
            assert_eq!(
                game.mobs.iter().map(|m| (m.id, m.join)).collect::<Vec<_>>(),
                rows
            );
            assert!(
                game.creatures.ticked.contains(&71),
                "earlier mob progress was lost"
            );
            let expected_value = if family.is_goal() || family == Family::Movement {
                42
            } else {
                314
            };
            assert_eq!(
                VALUE
                    .get_existing(&mut game.world.attachments, Holder::Level)
                    .unwrap(),
                Some(expected_value)
            );
            assert_eq!(
                game.server.last_hurt_by_player.get(&71).map(|c| c.0),
                Some(99)
            );
            assert_eq!(game.world.attachment_syncs, syncs);
            let mut random = LegacyRandom::new(LEVEL_SEED);
            let draw = (expected_value == 314).then(|| random.next_long());
            assert_eq!(*probe.draw.lock().unwrap(), draw, "seeded callback draw");
            // The completed outputs don't advance this stream: egg loot uses its named
            // sequence, entity motion and sound pitches use their own streams, and the
            // directed guest hits and pig pushes need no random direction.
            assert_eq!(
                game.world.random.clone().next_long(),
                random.next_long(),
                "level RNG position after the first tick"
            );
            let mut table = EntityTable::new(journal);
            for &(id, join) in &rows {
                let found = table.entity_lookup(&owner.dimension, id, |at| {
                    let m = &game.mobs[game.mobs.locate(at.join)?];
                    Some(SourceEntity {
                        id: m.id,
                        value: m.join,
                    })
                });
                assert!(
                    matches!(found, EntityLookup::Found { location, value } if location.owner == owner && value == join)
                );
            }
            table.end_tick().unwrap();

            if unwind.is_some() {
                assert!(
                    game.mobs[1].ai.targets.is_running("BorrowedTarget"),
                    "actual native target selector was lost"
                );
                assert!(
                    game.mobs[1].ai.goals.is_running("BorrowedProbe"),
                    "actual native goal selector was lost"
                );
                if family == Family::MountedGoal {
                    assert_eq!(
                        game.mobs[1].ai.nav.speed_modifier, 3.75,
                        "private rider navigation not restored"
                    );
                    assert_eq!(
                        game.mobs[0].ai.nav.speed_modifier, 1.25,
                        "vehicle navigation lost requested progress (or routing never loaned)"
                    );
                    assert!(
                        game.mobs[0].ai.nav.path.is_some(),
                        "vehicle lost successful native path"
                    );
                    eprintln!(
                        "BORROWED_MOUNTED_RESTORED:{dim:?}:rider=3.75:vehicle=1.25:path=Some"
                    );
                }
                game.tick_creature_entities().unwrap();
                assert_eq!(
                    probe.posts.lock().unwrap().as_slice(),
                    ids,
                    "real post listener never ran on retry"
                );
                table.end_tick().unwrap();
                if family == Family::MountedGoal {
                    assert_eq!(probe.calls.load(Ordering::SeqCst), 2);
                    eprintln!("BORROWED_MOUNTED_RETRY_OK:{dim:?}");
                }
                assert_eq!(
                    VALUE
                        .get_existing(&mut game.world.attachments, Holder::Level)
                        .unwrap(),
                    Some(expected_value)
                );
                return;
            }

            let expected_outputs: &[OutputNotice] = match output {
                Output::Egg => &[OutputNotice::Egg],
                Output::Guest => &[OutputNotice::Guest(12)],
                Output::Mixed => &[OutputNotice::Guest(12), OutputNotice::Egg],
                _ => &[],
            };
            assert_eq!(probe.outputs.lock().unwrap().as_slice(), expected_outputs);
            let expected_eggs = u32::from(matches!(output, Output::Egg | Output::Mixed));
            assert_eq!(
                eggs(game),
                expected_eggs,
                "egg output after the returning tick"
            );
            if output == Output::Egg {
                assert!(
                    game.mobs[0].extra.egg_time > 1,
                    "laying did not advance the cooldown"
                );
            }
            let guest_after = guest_before.map(|[before12, before13]| {
                let [after12, after13] = guest_health(game);
                assert!(
                    after12 < before12,
                    "guest 12 did not take the hit on the returning tick"
                );
                assert_eq!(after13, before13, "the hit reached the wrong guest");
                game.mobs[0].no_ai = true;
                game.as_guest(12, |g| {
                    g.player.as_mut().unwrap().health.invulnerable_time = 0
                })
                .unwrap();
                [after12, after13]
            });
            if output == Output::Push {
                assert!(
                    game.mobs[0].velocity.x > push_before,
                    "completed push was lost"
                );
                game.mobs[1].no_ai = true;
                game.mobs[1].position.x = 8.0;
                // Isolate the old impulse from carry-over velocity and other contacts.
                game.mobs[0].velocity.x = 0.0;
                game.mobs[target].position.x = 4.0;
            }

            prime_hook(&mut game.mobs[target], family);
            tick_in_level(game).unwrap();
            assert_eq!(
                probe.calls.load(Ordering::SeqCst),
                1,
                "disabled callback ran again"
            );
            assert!(
                !probe.healthy_proposals.lock().unwrap().is_empty(),
                "same hook never reached the healthy callback again"
            );
            assert!(take_hook_panics().is_empty());
            assert_eq!(probe.posts.lock().unwrap().as_slice(), ids);
            table.end_tick().unwrap();
            assert_eq!(
                VALUE
                    .get_existing(&mut game.world.attachments, Holder::Level)
                    .unwrap(),
                Some(expected_value)
            );
            assert_eq!(
                eggs(game),
                expected_eggs,
                "egg replayed or survived clear_entities"
            );
            assert_eq!(
                probe.outputs.lock().unwrap().as_slice(),
                expected_outputs,
                "completed output replayed"
            );
            if let Some(health) = guest_after {
                assert_eq!(guest_health(game), health, "guest action replayed");
            }
            if output == Output::Push {
                assert_eq!(game.mobs[0].velocity.x, 0.0, "push replayed");
            }
        });
    });
}

pub(crate) fn with_consumer_game(
    dim: Dim,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, &ConsumerProbe),
) {
    crate::entry::install_client_hooks();
    let probe = Arc::new(ConsumerProbe::default());
    let scratch = tempfile::tempdir().unwrap();
    let mods = vec![
        mod_api::ModEntry::new(
            "consumer",
            Box::new(Consumer {
                probe: probe.clone(),
                panics: true,
            }),
        ),
        mod_api::ModEntry::new(
            "healthy",
            Box::new(Consumer {
                probe: probe.clone(),
                panics: false,
            }),
        ),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.server.world_root = scratch.path().to_path_buf();
            game.enter_level(dim).unwrap();
            game.options.dedicated = Some(Vec::new());
            game.load_server_loot();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            test(game, &probe);
        });
    });
}
