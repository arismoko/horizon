use mod_api::{Flow, Hooks, HostTransaction, ProjectileImpactEvent, ServerHost};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::transactions::TransactionsGameExt;
use sunrise_level::chunk::Chunk;
use sunrise_sim::join_seq::residency::{
    Change, Journal, Kind, Owner,
    table::{EntityLookup, EntityTable, SourceEntity},
};
use sunrise_sim::projectiles::Projectile;

struct PanicAfterAdmission {
    admitted: Arc<AtomicBool>,
    panicked: Arc<AtomicBool>,
}
#[mod_api::__subscriptions]
impl Hooks for PanicAfterAdmission {
    fn projectile_impact(
        &self,
        host: &mut dyn ServerHost,
        event: &mut ProjectileImpactEvent<'_>,
    ) -> Flow {
        assert!(host.entity(event.projectile).is_some());
        self.admitted.store(true, Ordering::SeqCst);
        Flow::Continue
    }
    fn vanilla_game_event(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::VanillaGameEventEvent<'_>,
    ) -> Flow {
        if event.event == "minecraft:projectile_shoot" {
            assert!(self.admitted.load(Ordering::SeqCst));
            // Hook dispatch now contains the panic. The callback's own transaction
            // still has to abort its tentative spawn before dispatch resumes.
            let _ = host.transaction::<(), ()>(|host, _| {
                let level = host.level(event.dimension).unwrap();
                let spawned = level.summon(
                    host,
                    "minecraft:snowball",
                    [9.5, 70.0, 0.5],
                    Default::default(),
                );
                assert!(
                    spawned.unwrap().is_some(),
                    "the tentative spawn must exist to roll back"
                );
                self.panicked.store(true, Ordering::SeqCst);
                panic!("listener panicked after impact admission");
            });
        }
        Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn shoot_listener_panic_after_admission_preserves_owned_rows() {
    const CASE: &str = "SUNRISE_PROJECTILE_IMPACT_PANIC_CASE";
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        "cut_layout::game::server_side::native_projectiles::impact_panic_tests::shoot_listener_panic_after_admission_preserves_owned_rows",
        CASE,
        &["overworld", "nether"],
    ) {
        return;
    }
    {
        let dimension = match std::env::var(CASE).as_deref() {
            Ok("nether") => sunrise_gameplay_common::game::Dim::Nether,
            _ => sunrise_gameplay_common::game::Dim::Overworld,
        };
        let admitted = Arc::new(AtomicBool::new(false));
        let panicked = Arc::new(AtomicBool::new(false));
        let mods = vec![mod_api::ModEntry::new(
            "panic-after-admission",
            Box::new(PanicAfterAdmission {
                admitted: admitted.clone(),
                panicked: panicked.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                game.enter_level(dimension).unwrap();
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(2, 64, 0, stone).unwrap();
                // Hook handles resolve through the game's canonical residency table.
                let journal = game.resident_entities.table.borrow().producer();
                let observation = journal.observe_structural_events();
                let owner = Owner {
                    dimension: game.dim.key(),
                    kind: Kind::Projectile,
                };
                game.creatures
                    .projectiles
                    .bind_residency(&journal, owner, |p| (p.id, p.join));
                for i in 0..3 {
                    let x = if i == 2 { 5.5 } else { 0.5 };
                    let kind = if i == 2 {
                        // The missing snowball survives and queues its first-shot event.
                        sunrise_sim::projectiles::ProjectileKind::Snowball
                    } else {
                        // Arrows stay alive after impact, so no row is terminal here.
                        sunrise_sim::projectiles::ProjectileKind::Arrow {
                            base_damage: 2.0,
                            critical: false,
                            effect: None,
                        }
                    };
                    let mut p = Projectile::new(kind, Vec3::new(x, 64.5, 0.5), None);
                    p.id = 71 + i;
                    p.velocity = Vec3::new(3.0, 0.0, 0.0);
                    game.creatures.projectiles.push(p);
                }
                let before: Vec<_> = game
                    .creatures
                    .projectiles
                    .iter()
                    .map(|p| (p.id, p.join))
                    .collect();
                game.tick_projectiles().unwrap();
                assert_eq!(
                    mod_api::__private::poisoned_listeners(mod_api::HookId::VanillaGameEvent),
                    1
                );
                assert!(game.creatures.projectiles[..2].iter().all(|p| p.stuck));
                assert!(admitted.load(Ordering::SeqCst));
                assert!(
                    panicked.load(Ordering::SeqCst),
                    "did not reach the post-admission listener"
                );
                // Observe without stealing events already consumed by hook queries.
                let mut events = Vec::new();
                observation.drain_into(&mut events);
                let mut depth = 0usize;
                for event in events {
                    assert_eq!(event.owner, owner);
                    match event.change {
                        Change::Lifted => depth += 1,
                        Change::Restored => {
                            depth = depth.checked_sub(1).expect("restore without lift")
                        }
                        _ => {}
                    }
                }
                assert_eq!(depth, 0, "listener panic abandoned the lifted pass");
                game.resident_entities
                    .table
                    .borrow_mut()
                    .end_tick()
                    .expect("listener panic abandoned the lifted pass");
                let restored: Vec<_> = game
                    .creatures
                    .projectiles
                    .iter()
                    .map(|p| (p.id, p.join))
                    .collect();
                assert!(!game.has_open_transaction());
                // The contained panic cannot retire a live owner or publish the aborted spawn.
                assert_eq!(restored, before, "panic changed survivor identity or order");
                let mut table = game.resident_entities.table.borrow_mut();
                for expected in restored {
                    let found = table.entity_lookup(&owner.dimension, expected.0, |at| {
                        let list = &game.creatures.projectiles;
                        let entity = &list[list.locate(at.join)?];
                        Some(SourceEntity {
                            id: entity.id,
                            value: (entity.id, entity.join),
                        })
                    });
                    let EntityLookup::Found { location, value } = found else {
                        panic!("restored row did not resolve: {expected:?}");
                    };
                    assert_eq!(location.owner, owner);
                    assert_eq!(value, expected);
                }
                table.end_tick().expect("panic left an abandoned operation");
            });
        });
    }
}

#[test]
#[ignore = "daily"]
fn normal_dispatch_returns_the_owned_pass_for_explicit_restore() {
    for dimension in [
        sunrise_gameplay_common::game::Dim::Overworld,
        sunrise_gameplay_common::game::Dim::Nether,
    ] {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.enter_level(dimension).unwrap();
            let journal = Journal::default();
            let owner = Owner {
                dimension: game.dim.key(),
                kind: Kind::Projectile,
            };
            game.creatures
                .projectiles
                .bind_residency(&journal, owner, |p| (p.id, p.join));
            let mut original = Projectile::new(
                sunrise_sim::projectiles::ProjectileKind::Egg,
                Vec3::new(3.5, 80.0, 0.5),
                None,
            );
            original.id = 7;
            game.creatures.projectiles.push(original);
            let identity = (7, game.creatures.projectiles[0].join);
            let mut pass = game.creatures.projectiles.lift();
            let mut child = pass[0].clone();
            child.id = 8;
            child.join = sunrise_sim::join_seq::JoinSeq::UNJOINED;
            game.creatures.projectiles.push(child);
            let child_identity = (8, game.creatures.projectiles[0].join);
            game.with_projectile_dispatch(&mut pass, |game| {
                assert_eq!(
                    game.creatures
                        .projectiles
                        .iter()
                        .map(|p| (p.id, p.join))
                        .collect::<Vec<_>>(),
                    [child_identity]
                );
            });
            assert_eq!(
                pass.iter().map(|p| (p.id, p.join)).collect::<Vec<_>>(),
                [identity]
            );
            assert_eq!(
                game.creatures.projectiles.len(),
                1,
                "normal dispatch must not auto-restore"
            );
            game.creatures
                .projectiles
                .restore(pass, sunrise_sim::join_seq::RestoreOrder::SpawnsFirst);
            assert_eq!(
                game.creatures
                    .projectiles
                    .iter()
                    .map(|p| (p.id, p.join))
                    .collect::<Vec<_>>(),
                [child_identity, identity]
            );
            // An unwind leaving the dispatch itself (rather than a contained listener
            // panic) still has to restore its owning pass before it propagates.
            let mut pass = game.creatures.projectiles.lift();
            let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                game.with_projectile_dispatch(&mut pass, |_| panic!("dispatch unwind"));
            }))
            .expect_err("the dispatch guard must not swallow panics");
            assert_eq!(unwind.downcast_ref::<&str>(), Some(&"dispatch unwind"));
            assert!(pass.is_empty());
            assert_eq!(
                game.creatures
                    .projectiles
                    .iter()
                    .map(|p| (p.id, p.join))
                    .collect::<Vec<_>>(),
                [child_identity, identity]
            );
            EntityTable::new(journal)
                .end_tick()
                .expect("dispatch changed the lift depth");
        });
    }
}
